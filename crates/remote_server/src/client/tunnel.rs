use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use dashmap::DashMap;
use futures::channel::mpsc;
use futures::io::{AsyncRead, AsyncWrite};
use futures::task::AtomicWaker;
use futures::{Sink, Stream};

use crate::proto::{
    ClientMessage, ServerMessage, TunnelChannel, TunnelData, TunnelExit, TunnelHalfClose,
    TunnelReset, TunnelWindowUpdate, server_message, tunnel_client_message, tunnel_server_message,
};
use crate::protocol::{INITIAL_TUNNEL_WINDOW, MAX_TUNNEL_CHUNK_SIZE};

const MAX_STDERR_TAIL_BYTES: usize = 64 * 1024;

enum TunnelInbound {
    Data(Vec<u8>),
    Eof,
    Reset(String),
}

struct TunnelShared {
    write_credit: AtomicUsize,
    write_waker: AtomicWaker,
    stdout_credit: AtomicUsize,
    stderr_credit: AtomicUsize,
    stdout_offset: AtomicU64,
    stderr_offset: AtomicU64,
    stderr_tail: Mutex<VecDeque<u8>>,
    exit: Mutex<Option<TunnelExit>>,
    exit_tx: async_channel::Sender<()>,
    exit_rx: async_channel::Receiver<()>,
    finished: AtomicBool,
}

impl TunnelShared {
    fn new() -> Self {
        let (exit_tx, exit_rx) = async_channel::bounded(1);
        Self {
            write_credit: AtomicUsize::new(0),
            write_waker: AtomicWaker::new(),
            stdout_credit: AtomicUsize::new(INITIAL_TUNNEL_WINDOW),
            stderr_credit: AtomicUsize::new(INITIAL_TUNNEL_WINDOW),
            stdout_offset: AtomicU64::new(0),
            stderr_offset: AtomicU64::new(0),
            stderr_tail: Mutex::new(VecDeque::with_capacity(MAX_STDERR_TAIL_BYTES)),
            exit: Mutex::new(None),
            exit_tx,
            exit_rx,
            finished: AtomicBool::new(false),
        }
    }
}

struct TunnelEntry {
    inbound_tx: mpsc::Sender<TunnelInbound>,
    shared: Arc<TunnelShared>,
}

#[derive(Clone)]
pub(super) struct TunnelMultiplexer {
    entries: Arc<DashMap<String, TunnelEntry>>,
    outbound_tx: mpsc::Sender<ClientMessage>,
}

impl TunnelMultiplexer {
    pub(super) fn new(outbound_tx: mpsc::Sender<ClientMessage>) -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
            outbound_tx,
        }
    }

    pub(super) fn register(&self, stream_id: String) -> TunnelStream {
        let (inbound_tx, inbound_rx) = mpsc::channel(16);
        let shared = Arc::new(TunnelShared::new());
        self.entries.insert(
            stream_id.clone(),
            TunnelEntry {
                inbound_tx,
                shared: Arc::clone(&shared),
            },
        );
        TunnelStream {
            stream_id,
            inbound_rx,
            outbound_tx: self.outbound_tx.clone(),
            entries: Arc::clone(&self.entries),
            shared,
            read_buffer: VecDeque::new(),
            pending_window_update: 0,
            write_offset: 0,
            write_closed: false,
            read_eof: false,
        }
    }

    pub(super) fn activate(&self, stream_id: &str, stdin_window_bytes: usize) -> io::Result<()> {
        if stdin_window_bytes > INITIAL_TUNNEL_WINDOW {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SSH tunnel granted an invalid stdin window",
            ));
        }
        let entry = self.entries.get(stream_id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "SSH tunnel stream is not registered",
            )
        })?;
        entry
            .shared
            .write_credit
            .store(stdin_window_bytes, Ordering::Release);
        entry.shared.write_waker.wake();
        Ok(())
    }

    pub(super) fn remove(&self, stream_id: &str) {
        if let Some((_, entry)) = self.entries.remove(stream_id) {
            entry.shared.finished.store(true, Ordering::Release);
        }
    }

    pub(super) async fn handle_server_message(&self, msg: ServerMessage) {
        let Some(server_message::Message::Tunnel(tunnel)) = msg.message else {
            return;
        };
        let stream_id = tunnel.stream_id;
        let Some(message) = tunnel.message else {
            self.reset_stream(&stream_id, "SSH tunnel response had no message");
            return;
        };

        match message {
            tunnel_server_message::Message::Data(data) => {
                self.handle_data(&stream_id, data).await;
            }
            tunnel_server_message::Message::WindowUpdate(update) => {
                self.handle_window_update(&stream_id, update);
            }
            tunnel_server_message::Message::Exit(exit) => {
                if let Some(entry) = self.entries.get(&stream_id) {
                    *entry
                        .shared
                        .exit
                        .lock()
                        .expect("tunnel exit mutex poisoned") = Some(exit);
                    entry.shared.finished.store(true, Ordering::Release);
                    let _ = entry.shared.exit_tx.try_send(());
                    let mut tx = entry.inbound_tx.clone();
                    let _ = tx.try_send(TunnelInbound::Eof);
                }
            }
            tunnel_server_message::Message::Reset(reset) => {
                self.reset_stream(&stream_id, &reset.message);
            }
            tunnel_server_message::Message::ControlRegistered(_)
            | tunnel_server_message::Message::Opened(_) => {
                self.reset_stream(
                    &stream_id,
                    "unexpected SSH tunnel response without request id",
                );
            }
        }
    }

    async fn handle_data(&self, stream_id: &str, data: TunnelData) {
        if data.data.len() > MAX_TUNNEL_CHUNK_SIZE {
            self.reset_stream(stream_id, "SSH tunnel data frame exceeded the size limit");
            return;
        }
        let Some(entry) = self.entries.get(stream_id) else {
            return;
        };
        let shared = Arc::clone(&entry.shared);
        match data.channel() {
            TunnelChannel::Stdout => {
                if !reserve_inbound(
                    &shared.stdout_credit,
                    &shared.stdout_offset,
                    data.offset,
                    data.data.len(),
                ) {
                    drop(entry);
                    self.reset_stream(stream_id, "invalid SSH tunnel stdout offset or window");
                    return;
                }
                let mut tx = entry.inbound_tx.clone();
                if tx.try_send(TunnelInbound::Data(data.data)).is_err() {
                    drop(entry);
                    self.reset_stream(stream_id, "SSH tunnel stdout queue overflowed");
                }
            }
            TunnelChannel::Stderr => {
                if !reserve_inbound(
                    &shared.stderr_credit,
                    &shared.stderr_offset,
                    data.offset,
                    data.data.len(),
                ) {
                    drop(entry);
                    self.reset_stream(stream_id, "invalid SSH tunnel stderr offset or window");
                    return;
                }
                let consumed = data.data.len();
                {
                    let mut tail = shared
                        .stderr_tail
                        .lock()
                        .expect("tunnel stderr mutex poisoned");
                    tail.extend(data.data);
                    while tail.len() > MAX_STDERR_TAIL_BYTES {
                        tail.pop_front();
                    }
                }
                shared.stderr_credit.fetch_add(consumed, Ordering::Release);
                drop(entry);
                let mut outbound = self.outbound_tx.clone();
                let update = ClientMessage::tunnel(
                    String::new(),
                    stream_id.to_string(),
                    tunnel_client_message::Message::WindowUpdate(TunnelWindowUpdate {
                        channel: TunnelChannel::Stderr.into(),
                        consumed_bytes: consumed as u32,
                    }),
                );
                let _ = futures::SinkExt::send(&mut outbound, update).await;
            }
            TunnelChannel::Unspecified | TunnelChannel::Stdin => {
                drop(entry);
                self.reset_stream(
                    stream_id,
                    "server sent SSH tunnel data on an invalid channel",
                );
            }
        }
    }

    fn handle_window_update(&self, stream_id: &str, update: TunnelWindowUpdate) {
        if update.channel() != TunnelChannel::Stdin || update.consumed_bytes == 0 {
            self.reset_stream(stream_id, "invalid SSH tunnel stdin window update");
            return;
        }
        let Some(entry) = self.entries.get(stream_id) else {
            return;
        };
        let amount = update.consumed_bytes as usize;
        let result =
            entry
                .shared
                .write_credit
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |credit| {
                    credit
                        .checked_add(amount)
                        .filter(|new| *new <= INITIAL_TUNNEL_WINDOW)
                });
        if result.is_err() {
            drop(entry);
            self.reset_stream(stream_id, "SSH tunnel stdin window overflowed");
            return;
        }
        entry.shared.write_waker.wake();
    }

    fn reset_stream(&self, stream_id: &str, message: &str) {
        if let Some((_, entry)) = self.entries.remove(stream_id) {
            entry.shared.finished.store(true, Ordering::Release);
            entry.shared.write_waker.wake();
            let _ = entry.shared.exit_tx.try_send(());
            let mut tx = entry.inbound_tx;
            let _ = tx.try_send(TunnelInbound::Reset(message.to_string()));
        }
    }

    pub(super) fn disconnect_all(&self) {
        let stream_ids = self
            .entries
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for stream_id in stream_ids {
            self.reset_stream(&stream_id, "parent remote-server connection was lost");
        }
    }
}

fn reserve_inbound(
    credit: &AtomicUsize,
    offset: &AtomicU64,
    received_offset: u64,
    len: usize,
) -> bool {
    if len == 0 || offset.load(Ordering::Acquire) != received_offset {
        return false;
    }
    if credit
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_sub(len)
        })
        .is_err()
    {
        return false;
    }
    offset.fetch_add(len as u64, Ordering::Release);
    true
}

pub struct TunnelStream {
    stream_id: String,
    inbound_rx: mpsc::Receiver<TunnelInbound>,
    outbound_tx: mpsc::Sender<ClientMessage>,
    entries: Arc<DashMap<String, TunnelEntry>>,
    shared: Arc<TunnelShared>,
    read_buffer: VecDeque<u8>,
    pending_window_update: usize,
    write_offset: u64,
    write_closed: bool,
    read_eof: bool,
}

impl TunnelStream {
    pub fn connection_handle(&self) -> TunnelConnectionHandle {
        TunnelConnectionHandle {
            stream_id: self.stream_id.clone(),
            outbound_tx: self.outbound_tx.clone(),
            entries: Arc::clone(&self.entries),
            shared: Arc::clone(&self.shared),
            terminated: false,
        }
    }

    pub fn stderr_tail(&self) -> String {
        let tail = self
            .shared
            .stderr_tail
            .lock()
            .expect("tunnel stderr mutex poisoned")
            .iter()
            .copied()
            .collect::<Vec<_>>();
        String::from_utf8_lossy(&tail).into_owned()
    }

    pub fn exit_status(&self) -> Option<TunnelExit> {
        self.shared
            .exit
            .lock()
            .expect("tunnel exit mutex poisoned")
            .clone()
    }

    fn poll_send_window_update(&mut self, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.pending_window_update == 0 {
            return Poll::Ready(Ok(()));
        }
        match Pin::new(&mut self.outbound_tx).poll_ready(ctx) {
            Poll::Ready(Ok(())) => {
                let consumed = self.pending_window_update;
                let message = ClientMessage::tunnel(
                    String::new(),
                    self.stream_id.clone(),
                    tunnel_client_message::Message::WindowUpdate(TunnelWindowUpdate {
                        channel: TunnelChannel::Stdout.into(),
                        consumed_bytes: consumed as u32,
                    }),
                );
                if let Err(error) = Pin::new(&mut self.outbound_tx).start_send(message) {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        error.to_string(),
                    )));
                }
                self.shared
                    .stdout_credit
                    .fetch_add(consumed, Ordering::Release);
                self.pending_window_update = 0;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                error.to_string(),
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncRead for TunnelStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if buffer.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if self.poll_send_window_update(ctx).is_pending() {
            return Poll::Pending;
        }

        loop {
            if !self.read_buffer.is_empty() {
                let count = buffer.len().min(self.read_buffer.len());
                for slot in &mut buffer[..count] {
                    *slot = self.read_buffer.pop_front().expect("buffer length checked");
                }
                self.pending_window_update += count;
                return Poll::Ready(Ok(count));
            }
            if self.read_eof {
                return Poll::Ready(Ok(0));
            }
            match Pin::new(&mut self.inbound_rx).poll_next(ctx) {
                Poll::Ready(Some(TunnelInbound::Data(data))) => self.read_buffer.extend(data),
                Poll::Ready(Some(TunnelInbound::Eof)) | Poll::Ready(None) => {
                    self.read_eof = true;
                }
                Poll::Ready(Some(TunnelInbound::Reset(message))) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        message,
                    )));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl AsyncWrite for TunnelStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.write_closed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "SSH tunnel stdin is closed",
            )));
        }
        if buffer.is_empty() {
            return Poll::Ready(Ok(0));
        }

        self.shared.write_waker.register(ctx.waker());
        let credit = self.shared.write_credit.load(Ordering::Acquire);
        if credit == 0 {
            if self.shared.finished.load(Ordering::Acquire) {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "SSH tunnel is closed",
                )));
            }
            return Poll::Pending;
        }
        match Pin::new(&mut self.outbound_tx).poll_ready(ctx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(error)) => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    error.to_string(),
                )));
            }
            Poll::Pending => return Poll::Pending,
        }

        let count = buffer.len().min(credit).min(MAX_TUNNEL_CHUNK_SIZE);
        if self
            .shared
            .write_credit
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_sub(count)
            })
            .is_err()
        {
            ctx.waker().wake_by_ref();
            return Poll::Pending;
        }
        let message = ClientMessage::tunnel(
            String::new(),
            self.stream_id.clone(),
            tunnel_client_message::Message::Data(TunnelData {
                channel: TunnelChannel::Stdin.into(),
                offset: self.write_offset,
                data: buffer[..count].to_vec(),
            }),
        );
        if let Err(error) = Pin::new(&mut self.outbound_tx).start_send(message) {
            self.shared.write_credit.fetch_add(count, Ordering::Release);
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                error.to_string(),
            )));
        }
        self.write_offset += count as u64;
        Poll::Ready(Ok(count))
    }

    fn poll_flush(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.outbound_tx)
            .poll_flush(ctx)
            .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()))
    }

    fn poll_close(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.write_closed {
            match Pin::new(&mut self.outbound_tx).poll_ready(ctx) {
                Poll::Ready(Ok(())) => {
                    let message = ClientMessage::tunnel(
                        String::new(),
                        self.stream_id.clone(),
                        tunnel_client_message::Message::HalfClose(TunnelHalfClose {
                            channel: TunnelChannel::Stdin.into(),
                        }),
                    );
                    Pin::new(&mut self.outbound_tx)
                        .start_send(message)
                        .map_err(|error| {
                            io::Error::new(io::ErrorKind::BrokenPipe, error.to_string())
                        })?;
                    self.write_closed = true;
                }
                Poll::Ready(Err(error)) => {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        error.to_string(),
                    )));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        self.poll_flush(ctx)
    }
}

impl Drop for TunnelStream {
    fn drop(&mut self) {
        self.entries.remove(&self.stream_id);
        if self.shared.finished.load(Ordering::Acquire) {
            return;
        }
        let _ = self.outbound_tx.try_send(ClientMessage::tunnel(
            String::new(),
            self.stream_id.clone(),
            tunnel_client_message::Message::Reset(TunnelReset {
                message: "client dropped SSH tunnel stream".to_string(),
            }),
        ));
    }
}

impl std::fmt::Debug for TunnelStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TunnelStream")
            .field("stream_id", &self.stream_id)
            .finish_non_exhaustive()
    }
}

pub struct TunnelConnectionHandle {
    stream_id: String,
    outbound_tx: mpsc::Sender<ClientMessage>,
    entries: Arc<DashMap<String, TunnelEntry>>,
    shared: Arc<TunnelShared>,
    terminated: bool,
}

impl TunnelConnectionHandle {
    fn terminate_internal(&mut self) {
        if self.terminated || self.shared.finished.swap(true, Ordering::AcqRel) {
            self.terminated = true;
            return;
        }
        self.terminated = true;
        if let Some((_, mut entry)) = self.entries.remove(&self.stream_id) {
            let _ = entry.inbound_tx.try_send(TunnelInbound::Reset(
                "SSH tunnel connection was terminated".to_string(),
            ));
        }
        self.shared.write_waker.wake();
        let _ = self.shared.exit_tx.try_send(());
        let _ = self.outbound_tx.try_send(ClientMessage::tunnel(
            String::new(),
            self.stream_id.clone(),
            tunnel_client_message::Message::Reset(TunnelReset {
                message: "SSH tunnel connection was terminated".to_string(),
            }),
        ));
    }
}

impl std::fmt::Debug for TunnelConnectionHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TunnelConnectionHandle")
            .field("stream_id", &self.stream_id)
            .finish_non_exhaustive()
    }
}

#[cfg(not(target_family = "wasm"))]
impl crate::transport::TransportConnection for TunnelConnectionHandle {
    fn terminate(&mut self) {
        self.terminate_internal();
    }

    fn try_exit_status(&mut self) -> Option<crate::manager::RemoteServerExitStatus> {
        self.shared
            .exit
            .lock()
            .expect("tunnel exit mutex poisoned")
            .as_ref()
            .map(|exit| crate::manager::RemoteServerExitStatus {
                code: exit.exit_code,
                signal_killed: false,
            })
    }

    fn wait_for_exit(
        self: Box<Self>,
    ) -> Pin<Box<dyn futures::Future<Output = Option<crate::manager::RemoteServerExitStatus>> + Send>>
    {
        Box::pin(async move {
            if !self.shared.finished.load(Ordering::Acquire) {
                let _ = self.shared.exit_rx.recv().await;
            }
            self.shared
                .exit
                .lock()
                .expect("tunnel exit mutex poisoned")
                .as_ref()
                .map(|exit| crate::manager::RemoteServerExitStatus {
                    code: exit.exit_code,
                    signal_killed: false,
                })
        })
    }

    fn stderr_tail(&self) -> Option<String> {
        let stderr = self
            .shared
            .stderr_tail
            .lock()
            .expect("tunnel stderr mutex poisoned")
            .iter()
            .copied()
            .collect::<Vec<_>>();
        (!stderr.is_empty()).then(|| String::from_utf8_lossy(&stderr).into_owned())
    }
}

impl Drop for TunnelConnectionHandle {
    fn drop(&mut self) {
        self.terminate_internal();
    }
}

#[cfg(test)]
#[path = "tunnel_tests.rs"]
mod tests;
