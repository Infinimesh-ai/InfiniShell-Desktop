use std::collections::{HashMap, VecDeque};
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use command::r#async::Command;
use futures::future::{AbortHandle, Abortable};
use futures::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use remote_server::proto::{
    ClientMessage, ErrorCode, ErrorResponse, OpenSshStream, OpenSshStreamResponse,
    RegisterSshControl, RegisterSshControlResponse, ServerMessage, SshControlOwnership,
    SshStreamPurpose, TunnelChannel, TunnelData, TunnelExit, TunnelReset, TunnelServerMessage,
    TunnelWindowUpdate, client_message, server_message, tunnel_client_message,
    tunnel_server_message,
};
use remote_server::protocol::{
    INITIAL_TUNNEL_WINDOW, MAX_TUNNEL_CHUNK_SIZE, MAX_TUNNELS_PER_CONNECTION,
};
use warpui::r#async::FutureExt as _;
use warpui::r#async::executor;

const MAX_CONTROLS_PER_CONNECTION: usize = 64;
const MAX_CONTROL_PATH_BYTES: usize = 1024;
const MAX_IDENTITY_KEY_BYTES: usize = 4 * 1024;
const MAX_STDERR_TAIL_BYTES: usize = 64 * 1024;
const CONTROL_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct RegisteredControl {
    path: PathBuf,
    ownership: SshControlOwnership,
    staged_binary_path: String,
}

struct TunnelProcess {
    stdin_tx: async_channel::Sender<Vec<u8>>,
    stdout_credit_tx: async_channel::Sender<()>,
    stderr_credit_tx: async_channel::Sender<()>,
    expected_stdin_offset: AtomicU64,
    stdin_credit: AtomicUsize,
    stdout_in_flight: AtomicUsize,
    stderr_in_flight: AtomicUsize,
    stdout_returned_credit: AtomicUsize,
    stderr_returned_credit: AtomicUsize,
    stderr_tail: Mutex<VecDeque<u8>>,
    abort_handle: AbortHandle,
    stdin_closed: AtomicBool,
    finished: AtomicBool,
}

struct TunnelBrokerInner {
    controls: Mutex<HashMap<String, RegisteredControl>>,
    streams: Mutex<HashMap<String, Arc<TunnelProcess>>>,
    control_outbound_tx: async_channel::Sender<ServerMessage>,
    outbound_tx: async_channel::Sender<ServerMessage>,
    executor: Arc<executor::Background>,
}

impl Drop for TunnelBrokerInner {
    fn drop(&mut self) {
        let streams = self
            .streams
            .get_mut()
            .expect("tunnel stream mutex poisoned");
        for (_, stream) in streams.drain() {
            stream.abort_handle.abort();
            stream.stdin_tx.close();
        }
    }
}

#[derive(Clone)]
pub(super) struct TunnelBroker {
    inner: Arc<TunnelBrokerInner>,
}

impl TunnelBroker {
    pub(super) fn new(
        control_outbound_tx: async_channel::Sender<ServerMessage>,
        outbound_tx: async_channel::Sender<ServerMessage>,
        executor: Arc<executor::Background>,
    ) -> Self {
        Self {
            inner: Arc::new(TunnelBrokerInner {
                controls: Mutex::new(HashMap::new()),
                streams: Mutex::new(HashMap::new()),
                control_outbound_tx,
                outbound_tx,
                executor,
            }),
        }
    }

    pub(super) fn is_tunnel_message(message: &ClientMessage) -> bool {
        matches!(&message.message, Some(client_message::Message::Tunnel(_)))
    }

    pub(super) async fn handle_message(&self, message: ClientMessage) {
        let request_id = message.request_id;
        let Some(client_message::Message::Tunnel(tunnel)) = message.message else {
            return;
        };
        let stream_id = tunnel.stream_id;
        let Some(message) = tunnel.message else {
            self.send_error(request_id, "SSH tunnel message had no inner payload")
                .await;
            return;
        };

        match message {
            tunnel_client_message::Message::RegisterControl(registration) => {
                self.register_control(request_id, registration).await;
            }
            tunnel_client_message::Message::ReleaseControl(release) => {
                let control = self
                    .inner
                    .controls
                    .lock()
                    .expect("SSH control mutex poisoned")
                    .remove(&release.control_id);
                if let Some(control) = control {
                    match control.ownership {
                        SshControlOwnership::WarpManaged => {
                            if release.stop_control_master {
                                let args = remote_server::ssh::ssh_args(&control.path);
                                let stopped = Command::new("ssh")
                                    .arg("-O")
                                    .arg("exit")
                                    .args(args)
                                    .kill_on_drop(true)
                                    .output()
                                    .with_timeout(CONTROL_CHECK_TIMEOUT)
                                    .await;
                                if !matches!(stopped, Ok(Ok(output)) if output.status.success()) {
                                    log::warn!("停止远端 SSH ControlMaster 失败");
                                }
                            }
                        }
                        SshControlOwnership::UserOwned => {
                            log::debug!("释放用户拥有的 SSH ControlMaster 引用")
                        }
                        SshControlOwnership::Unspecified => {}
                    }
                }
            }
            tunnel_client_message::Message::Open(open) => {
                self.open_stream(request_id, stream_id, open).await;
            }
            tunnel_client_message::Message::Data(data) => {
                self.handle_data(&stream_id, data).await;
            }
            tunnel_client_message::Message::WindowUpdate(update) => {
                self.handle_window_update(&stream_id, update).await;
            }
            tunnel_client_message::Message::HalfClose(half_close) => {
                if half_close.channel() != TunnelChannel::Stdin {
                    self.reset_stream(&stream_id, "invalid SSH tunnel half-close channel")
                        .await;
                    return;
                }
                let Some(stream) = self.stream(&stream_id) else {
                    self.reset_stream(&stream_id, "unknown SSH tunnel stream")
                        .await;
                    return;
                };
                if stream.stdin_closed.swap(true, Ordering::AcqRel) {
                    self.reset_stream(&stream_id, "duplicate SSH tunnel half-close")
                        .await;
                    return;
                }
                stream.stdin_tx.close();
            }
            tunnel_client_message::Message::Reset(reset) => {
                self.remove_stream(&stream_id, true);
                log::debug!("客户端关闭了 SSH 隧道：{}", reset.message);
            }
        }
    }

    async fn register_control(&self, request_id: String, registration: RegisterSshControl) {
        let path = PathBuf::from(&registration.socket_path);
        let ownership = registration.ownership();
        if registration.owner_session_id == 0
            || registration.hop_depth == 0
            || registration.hop_depth > 8
            || registration.socket_path.len() > MAX_CONTROL_PATH_BYTES
            || !matches!(
                ownership,
                SshControlOwnership::WarpManaged | SshControlOwnership::UserOwned
            )
        {
            self.send_error(request_id, "invalid SSH control registration")
                .await;
            return;
        }
        if let Err(error) = validate_control_socket(&path) {
            log::warn!("拒绝注册 SSH ControlMaster：{error}");
            self.send_error(request_id, "SSH ControlMaster socket is not valid")
                .await;
            return;
        }
        let control_count = self
            .inner
            .controls
            .lock()
            .expect("SSH control mutex poisoned")
            .len();
        if control_count >= MAX_CONTROLS_PER_CONNECTION {
            self.send_error(request_id, "too many registered SSH controls")
                .await;
            return;
        }
        let args = remote_server::ssh::ssh_args(&path);
        let checked = Command::new("ssh")
            .arg("-O")
            .arg("check")
            .args(args)
            .kill_on_drop(true)
            .output()
            .with_timeout(CONTROL_CHECK_TIMEOUT)
            .await;
        if !matches!(checked, Ok(Ok(output)) if output.status.success()) {
            self.send_error(request_id, "SSH ControlMaster is not active")
                .await;
            return;
        }

        let control_id = uuid::Uuid::new_v4().to_string();
        let staged_binary_path = format!(
            "{}/infinishell-upload-{control_id}.tar.gz",
            remote_server::setup::remote_server_dir()
        );
        self.inner
            .controls
            .lock()
            .expect("SSH control mutex poisoned")
            .insert(
                control_id.clone(),
                RegisteredControl {
                    path,
                    ownership,
                    staged_binary_path,
                },
            );
        self.send_tunnel(
            request_id,
            String::new(),
            tunnel_server_message::Message::ControlRegistered(RegisterSshControlResponse {
                control_id,
            }),
        )
        .await;
    }

    async fn open_stream(&self, request_id: String, stream_id: String, open: OpenSshStream) {
        if stream_id.is_empty()
            || open.stdout_window_bytes as usize > INITIAL_TUNNEL_WINDOW
            || open.stderr_window_bytes as usize > INITIAL_TUNNEL_WINDOW
            || open.stdout_window_bytes == 0
            || open.stderr_window_bytes == 0
        {
            self.send_error(request_id, "invalid SSH tunnel open request")
                .await;
            return;
        }
        let stream_limit_reached = {
            let streams = self
                .inner
                .streams
                .lock()
                .expect("tunnel stream mutex poisoned");
            streams.len() >= MAX_TUNNELS_PER_CONNECTION || streams.contains_key(&stream_id)
        };
        if stream_limit_reached {
            self.send_error(
                request_id,
                "SSH tunnel limit reached or stream already exists",
            )
            .await;
            return;
        }
        let Some(control) = self
            .inner
            .controls
            .lock()
            .expect("SSH control mutex poisoned")
            .get(&open.control_id)
            .cloned()
        else {
            self.send_error(request_id, "unknown SSH control reference")
                .await;
            return;
        };
        let (remote_command, initial_stdin, accepts_client_stdin) =
            match ssh_stream_command(&open, &control.staged_binary_path) {
                Ok(command) => command,
                Err(message) => {
                    self.send_error(request_id, message).await;
                    return;
                }
            };

        let mut args = remote_server::ssh::ssh_args(&control.path);
        args.push(remote_command);
        let mut child = match Command::new("ssh")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                log::warn!("无法启动 SSH 隧道子进程：{error}");
                self.send_error(request_id, "failed to start SSH tunnel")
                    .await;
                return;
            }
        };
        let Some(stdin) = child.stdin.take() else {
            self.send_error(request_id, "failed to capture SSH tunnel stdin")
                .await;
            return;
        };
        let Some(stdout) = child.stdout.take() else {
            self.send_error(request_id, "failed to capture SSH tunnel stdout")
                .await;
            return;
        };
        let Some(stderr) = child.stderr.take() else {
            self.send_error(request_id, "failed to capture SSH tunnel stderr")
                .await;
            return;
        };

        let (stdin_tx, stdin_rx) = async_channel::bounded::<Vec<u8>>(8);
        // credit 数量在原子计数器中合并，channel 只负责唤醒输出 pump。这样客户端
        // 即使连续消费很多小块，也不会因通知队列瞬时填满而误判为协议溢出。
        let (stdout_credit_tx, stdout_credit_rx) = async_channel::bounded::<()>(1);
        let (stderr_credit_tx, stderr_credit_rx) = async_channel::bounded::<()>(1);
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let process = Arc::new(TunnelProcess {
            stdin_tx,
            stdout_credit_tx,
            stderr_credit_tx,
            expected_stdin_offset: AtomicU64::new(0),
            stdin_credit: AtomicUsize::new(INITIAL_TUNNEL_WINDOW),
            stdout_in_flight: AtomicUsize::new(0),
            stderr_in_flight: AtomicUsize::new(0),
            stdout_returned_credit: AtomicUsize::new(0),
            stderr_returned_credit: AtomicUsize::new(0),
            stderr_tail: Mutex::new(VecDeque::with_capacity(MAX_STDERR_TAIL_BYTES)),
            abort_handle,
            stdin_closed: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        });
        self.inner
            .streams
            .lock()
            .expect("tunnel stream mutex poisoned")
            .insert(stream_id.clone(), Arc::clone(&process));

        self.send_tunnel(
            request_id,
            stream_id.clone(),
            tunnel_server_message::Message::Opened(OpenSshStreamResponse {
                stdin_window_bytes: INITIAL_TUNNEL_WINDOW as u32,
            }),
        )
        .await;

        let weak = Arc::downgrade(&self.inner);
        let stdin_stream_id = stream_id.clone();
        let stdin_process = Arc::clone(&process);
        self.inner
            .executor
            .spawn(pump_stdin(
                weak.clone(),
                stdin_stream_id,
                stdin_process,
                stdin,
                stdin_rx,
                initial_stdin,
                accepts_client_stdin,
            ))
            .detach();

        let stdout_process = Arc::clone(&process);
        let stderr_process = Arc::clone(&process);
        self.inner
            .executor
            .spawn(async move {
                let work = async move {
                    let stdout_task = pump_output(
                        weak.clone(),
                        stream_id.clone(),
                        TunnelChannel::Stdout,
                        stdout,
                        open.stdout_window_bytes as usize,
                        stdout_credit_rx,
                        Arc::clone(&stdout_process),
                    );
                    let stderr_task = pump_output(
                        weak.clone(),
                        stream_id.clone(),
                        TunnelChannel::Stderr,
                        stderr,
                        open.stderr_window_bytes as usize,
                        stderr_credit_rx,
                        Arc::clone(&stderr_process),
                    );
                    let (status, _, _) = futures::join!(child.status(), stdout_task, stderr_task);
                    if let Some(inner) = weak.upgrade() {
                        let stderr_tail = process
                            .stderr_tail
                            .lock()
                            .expect("tunnel stderr mutex poisoned")
                            .iter()
                            .copied()
                            .collect::<Vec<_>>();
                        process.finished.store(true, Ordering::Release);
                        process.stdin_tx.close();
                        inner
                            .streams
                            .lock()
                            .expect("tunnel stream mutex poisoned")
                            .remove(&stream_id);
                        let exit_code = status.ok().and_then(|status| status.code());
                        let _ = inner
                            .outbound_tx
                            .send(server_tunnel_message(
                                String::new(),
                                stream_id,
                                tunnel_server_message::Message::Exit(TunnelExit {
                                    exit_code,
                                    stderr_tail: String::from_utf8_lossy(&stderr_tail).into_owned(),
                                }),
                            ))
                            .await;
                    }
                };
                let _ = Abortable::new(work, abort_registration).await;
            })
            .detach();
    }

    async fn handle_data(&self, stream_id: &str, data: TunnelData) {
        if data.channel() != TunnelChannel::Stdin
            || data.data.is_empty()
            || data.data.len() > MAX_TUNNEL_CHUNK_SIZE
        {
            self.reset_stream(stream_id, "invalid SSH tunnel stdin frame")
                .await;
            return;
        }
        let Some(stream) = self.stream(stream_id) else {
            self.reset_stream(stream_id, "unknown SSH tunnel stream")
                .await;
            return;
        };
        if stream.expected_stdin_offset.load(Ordering::Acquire) != data.offset
            || stream
                .stdin_credit
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |credit| {
                    credit.checked_sub(data.data.len())
                })
                .is_err()
        {
            self.reset_stream(stream_id, "invalid SSH tunnel stdin offset or window")
                .await;
            return;
        }
        stream
            .expected_stdin_offset
            .fetch_add(data.data.len() as u64, Ordering::Release);
        if stream.stdin_tx.try_send(data.data).is_err() {
            self.reset_stream(stream_id, "SSH tunnel stdin queue overflowed")
                .await;
        }
    }

    async fn handle_window_update(&self, stream_id: &str, update: TunnelWindowUpdate) {
        if update.consumed_bytes == 0 {
            self.reset_stream(stream_id, "invalid SSH tunnel window update")
                .await;
            return;
        }
        let Some(stream) = self.stream(stream_id) else {
            self.reset_stream(stream_id, "unknown SSH tunnel stream")
                .await;
            return;
        };
        let (in_flight, returned_credit, credit_tx) = match update.channel() {
            TunnelChannel::Stdout => (
                &stream.stdout_in_flight,
                &stream.stdout_returned_credit,
                &stream.stdout_credit_tx,
            ),
            TunnelChannel::Stderr => (
                &stream.stderr_in_flight,
                &stream.stderr_returned_credit,
                &stream.stderr_credit_tx,
            ),
            TunnelChannel::Unspecified | TunnelChannel::Stdin => {
                self.reset_stream(stream_id, "invalid SSH tunnel output window channel")
                    .await;
                return;
            }
        };
        let consumed = update.consumed_bytes as usize;
        if !return_output_credit(in_flight, returned_credit, consumed) {
            self.reset_stream(stream_id, "SSH tunnel output window overflowed")
                .await;
            return;
        }
        match credit_tx.try_send(()) {
            Ok(()) | Err(async_channel::TrySendError::Full(())) => {}
            Err(async_channel::TrySendError::Closed(())) => {
                self.reset_stream(stream_id, "SSH tunnel output pump is closed")
                    .await;
            }
        }
    }

    fn stream(&self, stream_id: &str) -> Option<Arc<TunnelProcess>> {
        self.inner
            .streams
            .lock()
            .expect("tunnel stream mutex poisoned")
            .get(stream_id)
            .cloned()
    }

    fn remove_stream(&self, stream_id: &str, abort: bool) {
        if let Some(stream) = self
            .inner
            .streams
            .lock()
            .expect("tunnel stream mutex poisoned")
            .remove(stream_id)
        {
            stream.finished.store(true, Ordering::Release);
            stream.stdin_tx.close();
            if abort {
                stream.abort_handle.abort();
            }
        }
    }

    async fn reset_stream(&self, stream_id: &str, message: &str) {
        self.remove_stream(stream_id, true);
        self.send_tunnel(
            String::new(),
            stream_id.to_string(),
            tunnel_server_message::Message::Reset(TunnelReset {
                message: message.to_string(),
            }),
        )
        .await;
    }

    async fn send_tunnel(
        &self,
        request_id: String,
        stream_id: String,
        message: tunnel_server_message::Message,
    ) {
        let message = server_tunnel_message(request_id, stream_id, message);
        let sender = if message.request_id.is_empty() {
            &self.inner.outbound_tx
        } else {
            &self.inner.control_outbound_tx
        };
        let _ = sender.send(message).await;
    }

    async fn send_error(&self, request_id: String, message: &str) {
        if request_id.is_empty() {
            return;
        }
        let _ = self
            .inner
            .control_outbound_tx
            .send(ServerMessage {
                request_id,
                message: Some(server_message::Message::Error(ErrorResponse {
                    code: ErrorCode::InvalidRequest.into(),
                    message: message.to_string(),
                })),
            })
            .await;
    }
}

fn ssh_stream_command(
    open: &OpenSshStream,
    staged_binary_path: &str,
) -> Result<(String, Option<Vec<u8>>, bool), &'static str> {
    let purpose = open.purpose();
    if purpose != SshStreamPurpose::RemoteServerProxy && !open.identity_key.is_empty() {
        return Err("identity key is only valid for the remote-server proxy");
    }
    match purpose {
        SshStreamPurpose::Unspecified => Err("SSH tunnel purpose is unspecified"),
        SshStreamPurpose::DetectPlatform => Ok(("uname -sm".to_string(), None, false)),
        SshStreamPurpose::PreinstallCheck => Ok((
            "bash -s".to_string(),
            Some(
                remote_server::setup::PREINSTALL_CHECK_SCRIPT
                    .as_bytes()
                    .to_vec(),
            ),
            false,
        )),
        SshStreamPurpose::CheckBinary => {
            Ok((remote_server::setup::binary_check_command(), None, false))
        }
        SshStreamPurpose::CheckOldBinary => Ok((
            format!(
                "test -d {}",
                shell_words::quote(&remote_server::setup::remote_server_dir())
            ),
            None,
            false,
        )),
        SshStreamPurpose::InstallBinary => Ok((
            "bash -s".to_string(),
            Some(remote_server::setup::install_script(None).into_bytes()),
            false,
        )),
        SshStreamPurpose::RemoteServerProxy => {
            if open.identity_key.is_empty() || open.identity_key.len() > MAX_IDENTITY_KEY_BYTES {
                return Err("invalid remote-server identity key");
            }
            Ok((
                format!(
                    "{} remote-server-proxy --identity-key {}",
                    remote_server::setup::remote_server_binary(),
                    shell_words::quote(&open.identity_key)
                ),
                None,
                true,
            ))
        }
        SshStreamPurpose::RemoveBinary => Ok((
            remote_server::setup::remote_server_removal_command(),
            None,
            false,
        )),
        SshStreamPurpose::StageBinary => {
            let install_dir = remote_server::setup::remote_server_dir();
            let partial_path = format!("{staged_binary_path}.part");
            Ok((
                format!(
                    "mkdir -p {} && cat > {} && mv {} {}",
                    shell_words::quote(&install_dir),
                    shell_words::quote(&partial_path),
                    shell_words::quote(&partial_path),
                    shell_words::quote(staged_binary_path),
                ),
                None,
                true,
            ))
        }
        SshStreamPurpose::InstallStagedBinary => Ok((
            "bash -s".to_string(),
            Some(remote_server::setup::install_script(Some(staged_binary_path)).into_bytes()),
            false,
        )),
    }
}

fn validate_control_socket(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(path.is_absolute(), "ControlMaster path is not absolute");
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        !metadata.file_type().is_symlink(),
        "ControlMaster path is a symlink"
    );
    anyhow::ensure!(
        metadata.file_type().is_socket(),
        "ControlMaster path is not a socket"
    );
    // SAFETY: `geteuid` 不读取或修改内存，只返回当前进程的有效用户 ID。
    let effective_user_id = unsafe { libc::geteuid() };
    anyhow::ensure!(
        metadata.uid() == effective_user_id,
        "ControlMaster owner differs"
    );
    Ok(())
}

async fn pump_stdin(
    inner: Weak<TunnelBrokerInner>,
    stream_id: String,
    process: Arc<TunnelProcess>,
    mut stdin: impl AsyncWrite + Unpin,
    stdin_rx: async_channel::Receiver<Vec<u8>>,
    initial_stdin: Option<Vec<u8>>,
    accepts_client_stdin: bool,
) {
    if let Some(initial_stdin) = initial_stdin
        && (stdin.write_all(&initial_stdin).await.is_err() || stdin.flush().await.is_err())
    {
        return;
    }
    if !accepts_client_stdin {
        let _ = stdin.close().await;
        return;
    }
    while let Ok(data) = stdin_rx.recv().await {
        if stdin.write_all(&data).await.is_err() || stdin.flush().await.is_err() {
            break;
        }
        process
            .stdin_credit
            .fetch_add(data.len(), Ordering::Release);
        let Some(inner) = inner.upgrade() else {
            return;
        };
        if inner
            .outbound_tx
            .send(server_tunnel_message(
                String::new(),
                stream_id.clone(),
                tunnel_server_message::Message::WindowUpdate(TunnelWindowUpdate {
                    channel: TunnelChannel::Stdin.into(),
                    consumed_bytes: data.len() as u32,
                }),
            ))
            .await
            .is_err()
        {
            return;
        }
    }
    let _ = stdin.close().await;
}

async fn pump_output(
    inner: Weak<TunnelBrokerInner>,
    stream_id: String,
    channel: TunnelChannel,
    mut reader: impl AsyncRead + Unpin,
    initial_credit: usize,
    credit_rx: async_channel::Receiver<()>,
    process: Arc<TunnelProcess>,
) {
    let mut credit = initial_credit;
    let mut offset = 0_u64;
    let returned_credit = match channel {
        TunnelChannel::Stdout => &process.stdout_returned_credit,
        TunnelChannel::Stderr => &process.stderr_returned_credit,
        TunnelChannel::Unspecified | TunnelChannel::Stdin => return,
    };
    loop {
        while credit_rx.try_recv().is_ok() {}
        let additional = returned_credit.swap(0, Ordering::AcqRel);
        let Some(updated) = credit.checked_add(additional) else {
            return;
        };
        if updated > INITIAL_TUNNEL_WINDOW {
            return;
        }
        credit = updated;
        if credit == 0 {
            let Ok(()) = credit_rx.recv().await else {
                return;
            };
            continue;
        }
        let mut buffer = vec![0; credit.min(MAX_TUNNEL_CHUNK_SIZE)];
        let Ok(read) = reader.read(&mut buffer).await else {
            return;
        };
        if read == 0 {
            return;
        }
        buffer.truncate(read);
        if channel == TunnelChannel::Stderr {
            let mut tail = process
                .stderr_tail
                .lock()
                .expect("tunnel stderr mutex poisoned");
            tail.extend(buffer.iter().copied());
            while tail.len() > MAX_STDERR_TAIL_BYTES {
                tail.pop_front();
            }
        }
        let in_flight = match channel {
            TunnelChannel::Stdout => &process.stdout_in_flight,
            TunnelChannel::Stderr => &process.stderr_in_flight,
            TunnelChannel::Unspecified | TunnelChannel::Stdin => return,
        };
        in_flight.fetch_add(read, Ordering::Release);
        let Some(inner) = inner.upgrade() else {
            return;
        };
        if inner
            .outbound_tx
            .send(server_tunnel_message(
                String::new(),
                stream_id.clone(),
                tunnel_server_message::Message::Data(TunnelData {
                    channel: channel.into(),
                    offset,
                    data: buffer,
                }),
            ))
            .await
            .is_err()
        {
            return;
        }
        offset += read as u64;
        credit -= read;
    }
}

fn return_output_credit(
    in_flight: &AtomicUsize,
    returned_credit: &AtomicUsize,
    consumed: usize,
) -> bool {
    if in_flight
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_sub(consumed)
        })
        .is_err()
    {
        return false;
    }
    returned_credit
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(consumed)
                .filter(|updated| *updated <= INITIAL_TUNNEL_WINDOW)
        })
        .is_ok()
}

fn server_tunnel_message(
    request_id: String,
    stream_id: String,
    message: tunnel_server_message::Message,
) -> ServerMessage {
    ServerMessage {
        request_id,
        message: Some(server_message::Message::Tunnel(TunnelServerMessage {
            stream_id,
            message: Some(message),
        })),
    }
}

#[cfg(test)]
#[path = "tunnel_tests.rs"]
mod tests;
