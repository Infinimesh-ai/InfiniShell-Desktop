use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use futures::channel::oneshot;
use futures::future::FutureExt;
use futures::io::BufReader;
use futures::{AsyncRead, AsyncWrite};
use warpui_core::r#async::executor::Background;

use super::Service;
use super::protocol::{
    ConnectionAddress, ProtocolError, RequestId, Response, receive_message, send_message,
};
use super::service::service_id;
use crate::platform::client::connect_client;
use crate::protocol::Request;

#[derive(Debug)]
pub enum InitializationError {
    Io(std::io::Error),
    UnsupportedPlatform,
}

#[derive(thiserror::Error, Debug)]
pub enum ClientError {
    #[error("Failed to initialize client: {0:?}")]
    Initialization(InitializationError),

    #[error("Connection was dropped.")]
    Disconnected,

    #[error("Internal error occurred: {0:?}")]
    InternalProtocol(#[from] ProtocolError),

    #[error("The channel for receiving the response from the inbound message task is closed.")]
    ResponseChannelClosed,

    #[error(
        "The channel for transmitting pending request info to the inbound message task is closed."
    )]
    PendingRequestInfoChannelClosed,
}

pub type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug)]
struct PendingRequestInfo {
    /// The ID of the in-flight request.
    request_id: RequestId,

    /// A sender for relaying the response bytes back to the caller of `send_request()`.
    response_result_tx: oneshot::Sender<Result<Vec<u8>>>,

    /// 调用方取消与 socket 写入之间共享的原子状态。
    state: Arc<RequestState>,

    /// 写入请求前等待入站任务完成登记，避免快速响应先到而被永久丢弃。
    registered_tx: oneshot::Sender<()>,
}

#[derive(Debug)]
enum PendingRequestUpdate {
    Register(PendingRequestInfo),
    Cancel {
        request_id: RequestId,
    },
    Fail {
        request_id: RequestId,
        error: ClientError,
    },
}

const REQUEST_QUEUED: u8 = 0;
const REQUEST_REGISTERED: u8 = 1;
const REQUEST_WRITING: u8 = 2;
const REQUEST_WRITTEN: u8 = 3;
const REQUEST_CANCELLED_BEFORE_WRITE: u8 = 4;
const REQUEST_CANCELLED_AFTER_WRITE: u8 = 5;
const REQUEST_COMPLETED: u8 = 6;

#[derive(Debug)]
struct RequestState {
    phase: AtomicU8,
}

impl RequestState {
    fn new() -> Self {
        Self {
            phase: AtomicU8::new(REQUEST_QUEUED),
        }
    }

    /// 登记只在请求尚未取消时成功。取消先到时不会留下待响应项。
    fn register(&self) -> bool {
        self.phase
            .compare_exchange(
                REQUEST_QUEUED,
                REQUEST_REGISTERED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// 与取消竞争 socket 写入所有权；只有成功者可以开始写完整 frame。
    fn begin_write(&self) -> bool {
        self.phase
            .compare_exchange(
                REQUEST_REGISTERED,
                REQUEST_WRITING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn mark_write_complete(&self) {
        let _ = self.phase.compare_exchange(
            REQUEST_WRITING,
            REQUEST_WRITTEN,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// 返回是否首次取消。写入开始后只清理响应等待者，不能声称请求未发送。
    fn cancel(&self) -> bool {
        let mut phase = self.phase.load(Ordering::Acquire);
        loop {
            let cancelled_phase = match phase {
                REQUEST_QUEUED | REQUEST_REGISTERED => REQUEST_CANCELLED_BEFORE_WRITE,
                REQUEST_WRITING | REQUEST_WRITTEN => REQUEST_CANCELLED_AFTER_WRITE,
                REQUEST_CANCELLED_BEFORE_WRITE
                | REQUEST_CANCELLED_AFTER_WRITE
                | REQUEST_COMPLETED => return false,
                invalid => {
                    debug_assert!(false, "未知 IPC 请求状态：{invalid}");
                    return false;
                }
            };
            match self.phase.compare_exchange_weak(
                phase,
                cancelled_phase,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(current) => phase = current,
            }
        }
    }

    fn complete(&self) {
        self.phase.store(REQUEST_COMPLETED, Ordering::Release);
    }
}

struct RequestCancellationGuard {
    request_id: RequestId,
    state: Arc<RequestState>,
    pending_request_info_tx: async_channel::Sender<PendingRequestUpdate>,
    armed: bool,
}

impl RequestCancellationGuard {
    fn new(
        request_id: RequestId,
        state: Arc<RequestState>,
        pending_request_info_tx: async_channel::Sender<PendingRequestUpdate>,
    ) -> Self {
        Self {
            request_id,
            state,
            pending_request_info_tx,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RequestCancellationGuard {
    fn drop(&mut self) {
        if self.armed && self.state.cancel() {
            let _ = self
                .pending_request_info_tx
                .try_send(PendingRequestUpdate::Cancel {
                    request_id: self.request_id,
                });
        }
    }
}

#[derive(Debug)]
struct PendingResponse {
    response_result_tx: oneshot::Sender<Result<Vec<u8>>>,
    state: Arc<RequestState>,
}

#[derive(Default)]
struct PendingResponses {
    response_senders: HashMap<RequestId, PendingResponse>,
}

impl PendingResponses {
    fn apply_update(&mut self, update: PendingRequestUpdate) {
        match update {
            PendingRequestUpdate::Register(PendingRequestInfo {
                request_id,
                response_result_tx,
                state,
                registered_tx,
            }) => {
                if state.register() {
                    self.response_senders.insert(
                        request_id,
                        PendingResponse {
                            response_result_tx,
                            state,
                        },
                    );
                }
                let _ = registered_tx.send(());
            }
            PendingRequestUpdate::Cancel { request_id } => {
                self.response_senders.remove(&request_id);
            }
            PendingRequestUpdate::Fail { request_id, error } => {
                if let Some(pending) = self.response_senders.remove(&request_id) {
                    pending.state.complete();
                    let _ = pending.response_result_tx.send(Err(error));
                }
            }
        }
    }

    fn complete(&mut self, request_id: RequestId, response_result: Result<Vec<u8>>) -> bool {
        let Some(pending) = self.response_senders.remove(&request_id) else {
            return false;
        };
        pending.state.complete();
        let _ = pending.response_result_tx.send(response_result);
        true
    }
}

#[derive(Debug)]
struct OutboundRequest {
    // The request to be sent to the server.
    request: Request,

    // A sender for relaying any error that occurs when sending the request.
    //
    // If the request is sent successfully, this sender is moved to the new `PendingRequestInfo`
    // created for the request, where it is eventually used to relay response bytes back to the
    // caller.
    response_result_tx: oneshot::Sender<Result<Vec<u8>>>,

    /// 调用方、出站任务与入站任务共享的请求生命周期。
    state: Arc<RequestState>,
}

pub struct Client {
    /// A sender for relaying requests from `Self::send_request()` to the background task
    /// responsible for actually writing requests to the socket.
    outbound_message_tx: async_channel::Sender<OutboundRequest>,

    /// `send_request` 的取消 guard 用它同步清理入站任务中的待响应项。
    pending_request_info_tx: async_channel::Sender<PendingRequestUpdate>,

    /// A receiver for a single-message bounded channel that emits an event when the server
    /// connection is dropped.
    disconnect_rx: async_channel::Receiver<()>,

    /// 此连接选择的 frame 上限；typed service 在分配请求 payload 前先用它做预检。
    max_frame_bytes: Option<usize>,

    /// A reference to the background executor so that we don't drop it while waiting on tasks
    /// that use it to run to completion. Otherwise, it can hang when all references are dropped.
    _background_executor: Arc<Background>,
}

impl Client {
    /// Creates a client connected to a server corresponding to the given `connection_address`.
    ///
    /// If successful, spawns background tasks to send requests and receive responses.
    pub async fn connect(
        connection_address: ConnectionAddress,
        background_executor: Arc<Background>,
    ) -> Result<Self> {
        Self::connect_with_optional_frame_limit(connection_address, background_executor, None).await
    }

    /// 创建仅对此客户端启用双向 frame 上限的连接。
    pub async fn connect_with_max_frame_bytes(
        connection_address: ConnectionAddress,
        background_executor: Arc<Background>,
        max_frame_bytes: usize,
    ) -> Result<Self> {
        Self::connect_with_optional_frame_limit(
            connection_address,
            background_executor,
            Some(max_frame_bytes),
        )
        .await
    }

    async fn connect_with_optional_frame_limit(
        connection_address: ConnectionAddress,
        background_executor: Arc<Background>,
        max_frame_bytes: Option<usize>,
    ) -> Result<Self> {
        #[cfg(windows)]
        let (reader, writer) = {
            // Windows 命名管道会绑定创建它的 Tokio reactor；连接与后续 I/O 必须在同一
            // background runtime 中运行，否则 stream 可能已连接却永远无法完成读写。
            let (connection_tx, connection_rx) = oneshot::channel();
            background_executor
                .spawn(async move {
                    let _ = connection_tx.send(connect_client(connection_address).await);
                })
                .detach();
            connection_rx
                .await
                .map_err(|_| ClientError::Disconnected)??
        };
        #[cfg(not(windows))]
        let (reader, writer) = connect_client(connection_address).await?;
        let (disconnect_tx, disconnect_rx) = async_channel::bounded(1);
        let (pending_request_info_tx, pending_request_info_rx) = async_channel::unbounded();
        let disconnect_tx_clone = disconnect_tx.clone();
        background_executor
            .spawn(async move {
                Self::handle_incoming_responses(reader, max_frame_bytes, pending_request_info_rx)
                    .await;
                let _ = disconnect_tx_clone.try_send(());
            })
            .detach();

        let (outbound_message_tx, outbound_message_rx) = async_channel::unbounded();
        let outgoing_pending_request_info_tx = pending_request_info_tx.clone();
        background_executor
            .spawn(async move {
                Self::handle_outgoing_requests(
                    writer,
                    max_frame_bytes,
                    outbound_message_rx,
                    outgoing_pending_request_info_tx,
                )
                .await;
                let _ = disconnect_tx.try_send(());
            })
            .detach();

        Ok(Self {
            outbound_message_tx,
            pending_request_info_tx,
            disconnect_rx,
            max_frame_bytes,
            _background_executor: background_executor,
        })
    }

    pub(super) fn max_frame_bytes(&self) -> Option<usize> {
        self.max_frame_bytes
    }

    pub async fn wait_for_disconnect(&self) {
        let _ = self.disconnect_rx.recv().await;
    }

    /// Schedules the given message to be written to the underlying transport.
    pub(super) async fn send_request<S: Service>(&self, request_bytes: Vec<u8>) -> Result<Vec<u8>> {
        let request = Request::new(service_id::<S>(), request_bytes);
        let request_id = *request.id();
        let state = Arc::new(RequestState::new());
        let mut cancellation_guard = RequestCancellationGuard::new(
            request_id,
            state.clone(),
            self.pending_request_info_tx.clone(),
        );

        // Create a channel for the response result. The sending end is sent to the outbound
        // message task. The outbound message task uses it to relay any error that might occur
        // when sending the message. If the message is sent successfully, the sending end is
        // forwarded to the _inbound_ message task, which will eventually use it to relay the
        // response bytes.
        let (response_result_tx, response_result_rx) = oneshot::channel();

        if self
            .outbound_message_tx
            .send(OutboundRequest {
                request,
                response_result_tx,
                state: state.clone(),
            })
            .await
            .is_err()
        {
            // The background inbound traffic processing task exited, so we must be disconnected.
            state.complete();
            cancellation_guard.disarm();
            return Err(ClientError::Disconnected);
        }

        let result = match response_result_rx.await {
            Ok(response_result) => response_result,
            Err(_) => Err(ClientError::ResponseChannelClosed),
        };
        state.complete();
        cancellation_guard.disarm();
        result
    }

    /// Handles incoming response messages and relays them  back to the caller via a
    /// request-specific async channel.
    async fn handle_incoming_responses(
        reader: impl AsyncRead + Unpin,
        max_frame_bytes: Option<usize>,
        pending_request_info_rx: async_channel::Receiver<PendingRequestUpdate>,
    ) {
        let mut reader = BufReader::new(reader);

        let mut pending_responses = PendingResponses::default();

        loop {
            futures::select! {
                pending_request_info = pending_request_info_rx.recv().fuse() => {
                    match pending_request_info {
                        Ok(update) => pending_responses.apply_update(update),
                        Err(_) => {
                            // This happens when the channel is closed, which implies the client
                            // was `Drop`ped, so break and exit.
                            break;
                        }

                    }
                }
                response = receive_message(&mut reader, max_frame_bytes).fuse() => {
                    match response {
                        Ok(response) => {
                            let (request_id, response_result) = match response {
                                Response::Success {
                                    request_id,
                                    bytes: response_bytes,
                                    ..
                                } =>  {
                                    (request_id, Ok(response_bytes))
                                }
                                Response::Failure {
                                    request_id,
                                    error_message,
                                } => {
                                    (request_id, Err(ClientError::InternalProtocol(ProtocolError::Other(error_message))))
                                }
                            };

                            if !pending_responses.complete(request_id, response_result) {
                                // When there is no corresponding response_senders
                                // entry for the message's request ID, we weren't
                                // expecting it.
                                log::warn!("Received unexpected message with id {request_id}.");
                            }
                        }
                        Err(e) => {
                            match e {
                                ProtocolError::Disconnected(_)
                                | ProtocolError::FrameTooLarge { .. }
                                | ProtocolError::FrameAllocationFailed { .. } => {
                                    // The server was disconnected, so break and exit.
                                    break;
                                }
                                ProtocolError::Serialization(error) => {
                                    log::warn!("Error occurred while receiving message: {error:?}");
                                }
                                ProtocolError::Other(error) => {
                                    log::warn!("Error occurred while receiving message: {error:?}");
                                    break;
                                }
                            }
                        }
                    }
                }
                complete => break,
            }
        }
    }

    /// Polls `outbound_message_rx` for request messages and sends them over the IPC transport.
    ///
    /// 请求在写入前先把 `response_result_tx` 登记到入站任务；登记确认后才允许对端响应。
    async fn handle_outgoing_requests(
        mut writer: impl AsyncWrite + Unpin,
        max_frame_bytes: Option<usize>,
        outbound_request_rx: async_channel::Receiver<OutboundRequest>,
        pending_request_info_tx: async_channel::Sender<PendingRequestUpdate>,
    ) {
        while let Ok(OutboundRequest {
            request,
            response_result_tx,
            state,
        }) = outbound_request_rx.recv().await
        {
            let request_id = *request.id();
            if state.phase.load(Ordering::Acquire) == REQUEST_CANCELLED_BEFORE_WRITE {
                continue;
            }
            let (registered_tx, registered_rx) = oneshot::channel();
            let update = PendingRequestUpdate::Register(PendingRequestInfo {
                request_id,
                response_result_tx,
                state: state.clone(),
                registered_tx,
            });
            if let Err(error) = pending_request_info_tx.send(update).await {
                if let PendingRequestUpdate::Register(pending) = error.0 {
                    let _ = pending
                        .response_result_tx
                        .send(Err(ClientError::PendingRequestInfoChannelClosed));
                }
                break;
            }
            if registered_rx.await.is_err() {
                break;
            }
            if !state.begin_write() {
                continue;
            }
            match send_message(&mut writer, request, max_frame_bytes).await {
                Ok(()) => state.mark_write_complete(),
                Err(ProtocolError::Disconnected(_)) => {
                    state.complete();
                    let _ = pending_request_info_tx
                        .send(PendingRequestUpdate::Fail {
                            request_id,
                            error: ClientError::Disconnected,
                        })
                        .await;
                    break;
                }
                Err(e) => {
                    state.complete();
                    let _ = pending_request_info_tx
                        .send(PendingRequestUpdate::Fail {
                            request_id,
                            error: ClientError::InternalProtocol(e),
                        })
                        .await;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
