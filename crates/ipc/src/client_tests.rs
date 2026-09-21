use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use futures::executor::block_on;
use futures::io::{AsyncWrite, AsyncWriteExt as _};
use parking_lot::Mutex;
use warpui_core::r#async::executor::Background;

use super::*;
use crate::platform::server::ConnectionListenerImpl;

struct TestService;

impl Service for TestService {
    type Request = ();
    type Response = ();
}

#[derive(Clone, Default)]
struct RecordingWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl RecordingWriter {
    fn bytes(&self) -> Vec<u8> {
        self.bytes.lock().clone()
    }
}

impl AsyncWrite for RecordingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _ctx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.bytes.lock().extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[derive(Default)]
struct GatedWriterState {
    bytes: Vec<u8>,
    gate_open: bool,
    waker: Option<Waker>,
}

#[derive(Clone, Default)]
struct GatedWriter {
    state: Arc<Mutex<GatedWriterState>>,
}

impl GatedWriter {
    fn bytes(&self) -> Vec<u8> {
        self.state.lock().bytes.clone()
    }

    fn open_gate(&self) {
        let waker = {
            let mut state = self.state.lock();
            state.gate_open = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl AsyncWrite for GatedWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut state = self.state.lock();
        if !state.bytes.is_empty() && !state.gate_open {
            state.waker = Some(ctx.waker().clone());
            return Poll::Pending;
        }

        let write_len = if state.bytes.is_empty() {
            1
        } else {
            bytes.len()
        };
        state.bytes.extend_from_slice(&bytes[..write_len]);
        Poll::Ready(Ok(write_len))
    }

    fn poll_flush(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn assert_complete_frame(wire: &[u8], request_id: RequestId) {
    let (header, payload) = wire.split_at(std::mem::size_of::<usize>());
    let frame_bytes = usize::from_be_bytes(header.try_into().unwrap());
    assert_eq!(frame_bytes, payload.len());
    let request: Request = bincode::deserialize(payload).unwrap();
    assert_eq!(*request.id(), request_id);
}

#[test]
fn dropping_a_queued_request_prevents_wire_write() {
    block_on(async {
        let (outbound_message_tx, outbound_message_rx) = async_channel::unbounded();
        let (pending_request_info_tx, pending_request_info_rx) = async_channel::unbounded();
        let (_disconnect_tx, disconnect_rx) = async_channel::bounded(1);
        let executor = Arc::new(Background::new(1, |_| {
            "ipc-client-queued-cancellation-test".to_owned()
        }));
        let client = Client {
            outbound_message_tx,
            pending_request_info_tx: pending_request_info_tx.clone(),
            disconnect_rx,
            max_frame_bytes: None,
            _background_executor: executor,
        };

        let mut request = Box::pin(client.send_request::<TestService>(Vec::new()));
        assert!(matches!(futures::poll!(request.as_mut()), Poll::Pending));
        drop(request);
        drop(client);

        let mut pending_responses = PendingResponses::default();
        pending_responses.apply_update(pending_request_info_rx.recv().await.unwrap());
        let writer = RecordingWriter::default();
        Client::handle_outgoing_requests(
            writer.clone(),
            None,
            outbound_message_rx,
            pending_request_info_tx,
        )
        .await;

        assert!(writer.bytes().is_empty());
        assert!(pending_responses.response_senders.is_empty());
    });
}

#[test]
fn cancellation_after_registration_but_before_write_prevents_wire_write() {
    block_on(async {
        let (outbound_message_tx, outbound_message_rx) = async_channel::unbounded();
        let (pending_request_info_tx, pending_request_info_rx) = async_channel::unbounded();
        let request = Request::new(service_id::<TestService>(), Vec::new());
        let request_id = *request.id();
        let state = Arc::new(RequestState::new());
        let (response_result_tx, response_result_rx) = oneshot::channel();
        let cancellation_guard = RequestCancellationGuard::new(
            request_id,
            state.clone(),
            pending_request_info_tx.clone(),
        );
        outbound_message_tx
            .send(OutboundRequest {
                request,
                response_result_tx,
                state: state.clone(),
            })
            .await
            .unwrap();
        drop(outbound_message_tx);

        let writer = RecordingWriter::default();
        let mut outgoing = Box::pin(Client::handle_outgoing_requests(
            writer.clone(),
            None,
            outbound_message_rx,
            pending_request_info_tx,
        ));
        assert!(matches!(futures::poll!(outgoing.as_mut()), Poll::Pending));

        let mut pending_responses = PendingResponses::default();
        pending_responses.apply_update(pending_request_info_rx.recv().await.unwrap());
        assert_eq!(pending_responses.response_senders.len(), 1);
        drop(response_result_rx);
        drop(cancellation_guard);
        pending_responses.apply_update(pending_request_info_rx.recv().await.unwrap());
        assert!(pending_responses.response_senders.is_empty());

        outgoing.await;

        assert!(writer.bytes().is_empty());
        assert_eq!(
            state.phase.load(Ordering::Acquire),
            REQUEST_CANCELLED_BEFORE_WRITE
        );
    });
}

#[test]
fn cancellation_after_write_removes_an_unanswered_pending_request() {
    block_on(async {
        let (outbound_message_tx, outbound_message_rx) = async_channel::unbounded();
        let (pending_request_info_tx, pending_request_info_rx) = async_channel::unbounded();
        let request = Request::new(service_id::<TestService>(), Vec::new());
        let request_id = *request.id();
        let state = Arc::new(RequestState::new());
        let (response_result_tx, response_result_rx) = oneshot::channel();
        let cancellation_guard = RequestCancellationGuard::new(
            request_id,
            state.clone(),
            pending_request_info_tx.clone(),
        );
        outbound_message_tx
            .send(OutboundRequest {
                request,
                response_result_tx,
                state: state.clone(),
            })
            .await
            .unwrap();
        drop(outbound_message_tx);

        let writer = RecordingWriter::default();
        let mut outgoing = Box::pin(Client::handle_outgoing_requests(
            writer.clone(),
            None,
            outbound_message_rx,
            pending_request_info_tx,
        ));
        assert!(matches!(futures::poll!(outgoing.as_mut()), Poll::Pending));
        let mut pending_responses = PendingResponses::default();
        pending_responses.apply_update(pending_request_info_rx.recv().await.unwrap());

        outgoing.await;
        assert_eq!(state.phase.load(Ordering::Acquire), REQUEST_WRITTEN);
        assert_eq!(pending_responses.response_senders.len(), 1);
        assert_complete_frame(&writer.bytes(), request_id);

        drop(response_result_rx);
        drop(cancellation_guard);
        pending_responses.apply_update(pending_request_info_rx.recv().await.unwrap());

        assert!(pending_responses.response_senders.is_empty());
        assert_eq!(
            state.phase.load(Ordering::Acquire),
            REQUEST_CANCELLED_AFTER_WRITE
        );
    });
}

#[test]
fn cancellation_during_write_finishes_the_frame_before_reusing_the_connection() {
    block_on(async {
        let (outbound_message_tx, outbound_message_rx) = async_channel::unbounded();
        let (pending_request_info_tx, pending_request_info_rx) = async_channel::unbounded();
        let request = Request::new(service_id::<TestService>(), Vec::new());
        let request_id = *request.id();
        let state = Arc::new(RequestState::new());
        let (response_result_tx, response_result_rx) = oneshot::channel();
        let cancellation_guard = RequestCancellationGuard::new(
            request_id,
            state.clone(),
            pending_request_info_tx.clone(),
        );
        outbound_message_tx
            .send(OutboundRequest {
                request,
                response_result_tx,
                state: state.clone(),
            })
            .await
            .unwrap();
        drop(outbound_message_tx);

        let writer = GatedWriter::default();
        let mut outgoing = Box::pin(Client::handle_outgoing_requests(
            writer.clone(),
            None,
            outbound_message_rx,
            pending_request_info_tx,
        ));
        assert!(matches!(futures::poll!(outgoing.as_mut()), Poll::Pending));
        let mut pending_responses = PendingResponses::default();
        pending_responses.apply_update(pending_request_info_rx.recv().await.unwrap());
        assert!(matches!(futures::poll!(outgoing.as_mut()), Poll::Pending));
        assert_eq!(writer.bytes().len(), 1);
        assert_eq!(state.phase.load(Ordering::Acquire), REQUEST_WRITING);

        drop(response_result_rx);
        drop(cancellation_guard);
        pending_responses.apply_update(pending_request_info_rx.recv().await.unwrap());
        writer.open_gate();
        outgoing.await;

        assert_complete_frame(&writer.bytes(), request_id);
        assert!(pending_responses.response_senders.is_empty());
        assert_eq!(
            state.phase.load(Ordering::Acquire),
            REQUEST_CANCELLED_AFTER_WRITE
        );
    });
}

#[test]
fn cancellations_before_registration_do_not_accumulate_pending_responses() {
    block_on(async {
        let (pending_request_info_tx, pending_request_info_rx) = async_channel::unbounded();
        let mut pending_responses = PendingResponses::default();

        for _ in 0..4_096 {
            let request = Request::new(service_id::<TestService>(), Vec::new());
            let request_id = *request.id();
            let state = Arc::new(RequestState::new());
            let (response_result_tx, response_result_rx) = oneshot::channel();
            let cancellation_guard = RequestCancellationGuard::new(
                request_id,
                state.clone(),
                pending_request_info_tx.clone(),
            );
            drop(response_result_rx);
            drop(cancellation_guard);
            pending_responses.apply_update(pending_request_info_rx.recv().await.unwrap());

            let (registered_tx, registered_rx) = oneshot::channel();
            pending_responses.apply_update(PendingRequestUpdate::Register(PendingRequestInfo {
                request_id,
                response_result_tx,
                state,
                registered_tx,
            }));
            registered_rx.await.unwrap();
        }

        assert!(pending_responses.response_senders.is_empty());
    });
}

#[test]
fn client_disconnects_after_an_oversized_response_header() {
    let address = ConnectionAddress::new();
    #[cfg(unix)]
    let endpoint = address.0.clone();
    let listener = ConnectionListenerImpl::new(address.clone()).unwrap();
    let executor = Arc::new(Background::new(1, |_| {
        "ipc-client-frame-limit-test".to_owned()
    }));

    block_on(async {
        let (client, connection) = futures::join!(
            Client::connect_with_max_frame_bytes(address, executor, 12),
            listener.accept_connection(),
        );
        let client = client.unwrap();
        let (_, mut writer) = connection.unwrap().into_split();
        writer.write_all(&13_usize.to_be_bytes()).await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);

        client.wait_for_disconnect().await;
    });

    drop(listener);
    #[cfg(unix)]
    let _ = std::fs::remove_file(endpoint);
}
