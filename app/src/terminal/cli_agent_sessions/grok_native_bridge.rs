//! 普通本地 PTY 的固定 Grok 原生事务客户端；不启动进程、不发按键、不重投。
//!
//! 所有阻塞发现与 I/O 必须在后台执行。调用者须先持久领取消息，才可调用 submit；
//! 写入后的任何错误均属未知结果，只能携带原身份查询，不能据此清除用户草稿。

use std::fs::{self, Metadata, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use command::managed::{
    MacosPeerHandle, MacosProcessIdentity, macos_boot_session, macos_peer_handle,
    macos_process_terminal,
};
use serde::{Deserialize, Serialize};
use socket2::{Domain, SockAddr, Socket, Type};
use uuid::Uuid;

pub(crate) use super::grok_native_bridge_identity::NativeBridgeProcess;
use super::grok_native_bridge_identity::{ARTIFACT_SHA256, Artifact, FileStamp, verify_terminal};
use crate::terminal::model::local_pty_identity::LocalPtyIdentity;

const MAX_FRAME_BYTES: usize = 256 * 1024;
const MAX_TEXT_BYTES: usize = 128 * 1024;
const MAX_MANIFEST_BYTES: u64 = 4096;
const MAX_DISCOVERY_ENTRIES: usize = 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const LEASE_LIFETIME: Duration = Duration::from_secs(5);

/// 可保存的身份快照不含定位凭据，也不代表输入仍可提交。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBridgeBinding {
    pub instance_id: Uuid,
    pub native_process: NativeBridgeProcess,
    pub boot_session: String,
    pub shell: NativeBridgeProcess,
    pub slave_device: u64,
    pub artifact_sha256: String,
}

/// 查询必须携带持久领取时的期望身份，不能采信回包中的另一会话或另一正文。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBridgeMessage {
    pub message_id: Uuid,
    pub session_id: String,
    pub payload_digest: String,
}

impl NativeBridgeMessage {
    pub(crate) fn new(message_id: Uuid, session_id: String, text: &str) -> io::Result<Self> {
        if message_id.is_nil()
            || !valid_session(&session_id)
            || text.trim().is_empty()
            || text.len() > MAX_TEXT_BYTES
        {
            return Err(invalid());
        }
        Ok(Self {
            message_id,
            session_id,
            payload_digest: format!("blake3:{}", blake3::hash(text.as_bytes()).to_hex()),
        })
    }

    fn validate(&self) -> io::Result<()> {
        if self.message_id.is_nil()
            || !valid_session(&self.session_id)
            || !self
                .payload_digest
                .strip_prefix("blake3:")
                .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(lower_hex))
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeBridgeReceiptState {
    Claimed,
    Dispatched,
    NativeAcknowledged,
    NativeRejected,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBridgeReceipt {
    pub message_id: String,
    pub prompt_id: String,
    pub session_id: String,
    pub payload_digest: String,
    pub state: NativeBridgeReceiptState,
}

/// Unknown 和所有 I/O 错误都不能解锁重投；Rejected 也不代表先前消息从未提交。
pub(crate) enum NativeBridgeResponse {
    Receipt(NativeBridgeReceipt),
    Unknown,
    Rejected { reason: String },
}

pub(crate) struct NativeBridgeState {
    pub ready: bool,
    pub input_epoch: u64,
    pub reason: Option<String>,
    pub lease: Option<NativeBridgeLease>,
}

/// 不实现 Clone、Serialize 或 Deserialize，过期租约不能从磁盘重新获得授权。
pub(crate) struct NativeBridgeLease {
    instance_id: Uuid,
    issued_before: Instant,
    wire: WireLease,
}

/// 只确认首页初始化的固定目标，不授予正文写入权，也不是接收回执。
pub(crate) struct NativeBridgePreparation {
    instance_id: Uuid,
    input_epoch: u64,
    agent_id: usize,
    expected_binding_epoch: u32,
    expected_session_id: String,
}

impl NativeBridgePreparation {
    pub(crate) fn lease_from_state(
        &self,
        state: NativeBridgeState,
    ) -> io::Result<Option<NativeBridgeLease>> {
        if state.input_epoch < self.input_epoch {
            return Err(invalid());
        }
        let Some(lease) = state.lease else {
            return Ok(None);
        };
        if !state.ready
            || lease.instance_id != self.instance_id
            || lease.wire.agent_id != self.agent_id
            || lease.wire.binding_epoch != self.expected_binding_epoch
            || lease.wire.session_id != self.expected_session_id
        {
            return Err(invalid());
        }
        Ok(Some(lease))
    }
}

impl NativeBridgeLease {
    pub(crate) fn session_id(&self) -> &str {
        &self.wire.session_id
    }
    pub(crate) fn binding_epoch(&self) -> u32 {
        self.wire.binding_epoch
    }
    pub(crate) fn input_epoch(&self) -> u64 {
        self.wire.input_epoch
    }
}

// 包含私有 token 的对象均不实现 Debug；错误只含稳定分类，不携带原始响应。
pub(crate) struct NativeBridge {
    locator: Locator,
    pty: LocalPtyIdentity,
    peer: MacosProcessIdentity,
    artifact: Artifact,
    binding: NativeBridgeBinding,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    protocol_version: u32,
    native_pid: u32,
    instance_id: String,
    socket: PathBuf,
    token: String,
}

struct Locator {
    root: PathBuf,
    root_stamp: NodeStamp,
    directory: PathBuf,
    directory_stamp: NodeStamp,
    socket_stamp: NodeStamp,
    manifest_stamp: FileStamp,
    manifest: Manifest,
}

#[derive(Clone, Copy)]
struct NodeStamp {
    device: u64,
    inode: u64,
}

impl NodeStamp {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    fn matches(self, metadata: &Metadata) -> bool {
        self.device == metadata.dev() && self.inode == metadata.ino()
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireLease {
    lease_id: String,
    session_id: String,
    agent_id: usize,
    binding_epoch: u32,
    input_epoch: u64,
}

#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Request<'a> {
    State {
        instance_id: &'a str,
    },
    PrepareSessionIfIdle {
        instance_id: &'a str,
        input_epoch: u64,
    },
    SubmitIfIdle {
        instance_id: &'a str,
        lease_id: &'a str,
        session_id: &'a str,
        binding_epoch: u32,
        input_epoch: u64,
        message_id: String,
        prompt_id: String,
        text: &'a str,
    },
    QueryReceipt {
        instance_id: &'a str,
        message_id: String,
    },
}

#[derive(Serialize)]
struct Envelope<'a> {
    token: &'a str,
    request: Request<'a>,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum WireResponse {
    State {
        instance_id: String,
        input_epoch: u64,
        ready: bool,
        reason: Option<String>,
        lease: Option<WireLease>,
    },
    Prepared {
        instance_id: String,
        input_epoch: u64,
        agent_id: usize,
        expected_binding_epoch: u32,
        expected_session_id: String,
    },
    NotReady {
        reason: String,
    },
    Receipt {
        receipt: NativeBridgeReceipt,
    },
    Unknown {
        message_id: String,
    },
    Rejected {
        reason: String,
    },
}

impl NativeBridge {
    pub(crate) fn discover(root: &Path, pty: LocalPtyIdentity) -> io::Result<Option<Self>> {
        let root_metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        validate_ancestors(root)?;
        private_directory(&root_metadata)?;
        let root_stamp = NodeStamp::of(&root_metadata);
        let boot_session = macos_boot_session()?;
        pty.current_shell()?;
        let mut found = None;
        for (index, entry) in fs::read_dir(root)?.enumerate() {
            if index >= MAX_DISCOVERY_ENTRIES {
                return Err(invalid());
            }
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_instance_name(&name) {
                continue;
            }
            // 其他 TUI 可正在写入清单或退出；单个不完整 locator 不能成为可信候选。
            let locator = match Locator::read(root, root_stamp, entry.path()) {
                Ok(Some(locator)) => locator,
                Ok(None) | Err(_) => continue,
            };
            let stream = match connect(&locator.manifest.socket) {
                Ok(stream) => stream,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            };
            let peer = macos_peer_handle(&stream)?;
            let terminal = macos_process_terminal(peer.identity())?;
            // 同一 PTY 中已挂起的旧 Grok 不是当前候选，不能阻挡新的前台实例。
            if terminal.tty_device != pty.slave_device()
                || terminal.process_group <= 0
                || terminal.process_group != terminal.foreground_group
            {
                continue;
            }
            if found.is_some() {
                return Err(invalid());
            }
            if peer.identity().pid as u32 != locator.manifest.native_pid {
                return Err(invalid());
            }
            let shell = verify_terminal(peer.identity(), pty)?;
            let artifact = Artifact::capture(&peer)?;
            let binding = NativeBridgeBinding {
                instance_id: canonical_uuid(&locator.manifest.instance_id)?,
                native_process: peer.identity().into(),
                boot_session: boot_session.clone(),
                shell: shell.into(),
                slave_device: pty.slave_device(),
                artifact_sha256: ARTIFACT_SHA256.to_owned(),
            };
            let bridge = Self {
                locator,
                pty,
                peer: peer.identity(),
                artifact,
                binding,
            };
            bridge.verify(&peer)?;
            found = Some(bridge);
        }
        if macos_boot_session()? != boot_session
            || !root_stamp.matches(&fs::symlink_metadata(root)?)
        {
            return Err(invalid());
        }
        Ok(found)
    }

    pub(crate) fn binding(&self) -> &NativeBridgeBinding {
        &self.binding
    }

    pub(crate) fn state(&self) -> io::Result<NativeBridgeState> {
        let issued_before = Instant::now();
        let response = self.exchange(
            Request::State {
                instance_id: &self.locator.manifest.instance_id,
            },
            None,
            || true,
        )?;
        parse_state(response, self.binding.instance_id, issued_before)
    }

    /// 仅由用户本次提交在首页触发一次；请求不含正文，响应丢失后不得重试初始化。
    pub(crate) fn prepare_session_if_idle(
        &self,
        state: NativeBridgeState,
    ) -> io::Result<Option<NativeBridgePreparation>> {
        if state.ready
            || state.lease.is_some()
            || state.reason.as_deref() != Some("no_active_agent")
        {
            return Err(invalid());
        }
        let response = self.exchange(
            Request::PrepareSessionIfIdle {
                instance_id: &self.locator.manifest.instance_id,
                input_epoch: state.input_epoch,
            },
            None,
            || true,
        )?;
        parse_preparation(response, self.binding.instance_id, state.input_epoch)
    }

    /// 租约按值消费；任何部分写入或响应丢失都留给持久消息身份做只读查询。
    pub(crate) fn submit(
        &self,
        lease: NativeBridgeLease,
        message_id: Uuid,
        text: &str,
        admit: impl FnOnce() -> bool,
    ) -> io::Result<NativeBridgeResponse> {
        if lease.instance_id != self.binding.instance_id
            || lease.issued_before.elapsed() >= LEASE_LIFETIME
        {
            return Err(invalid());
        }
        let expected = NativeBridgeMessage::new(message_id, lease.wire.session_id.clone(), text)?;
        let response = self.exchange(
            Request::SubmitIfIdle {
                instance_id: &self.locator.manifest.instance_id,
                lease_id: &lease.wire.lease_id,
                session_id: &lease.wire.session_id,
                binding_epoch: lease.wire.binding_epoch,
                input_epoch: lease.wire.input_epoch,
                message_id: message_id.to_string(),
                prompt_id: message_id.to_string(),
                text,
            },
            Some(lease.issued_before + LEASE_LIFETIME),
            admit,
        )?;
        parse_receipt(response, &expected)
    }

    pub(crate) fn query(&self, expected: &NativeBridgeMessage) -> io::Result<NativeBridgeResponse> {
        expected.validate()?;
        let response = self.exchange(
            Request::QueryReceipt {
                instance_id: &self.locator.manifest.instance_id,
                message_id: expected.message_id.to_string(),
            },
            None,
            || true,
        )?;
        parse_receipt(response, expected)
    }

    fn verify(&self, peer: &MacosPeerHandle) -> io::Result<()> {
        self.locator.validate()?;
        if macos_boot_session()? != self.binding.boot_session {
            return Err(invalid());
        }
        if peer.identity() != self.peer
            || peer.identity().pid as u32 != self.locator.manifest.native_pid
        {
            return Err(invalid());
        }
        verify_terminal(peer.identity(), self.pty)?;
        self.artifact.verify(peer)?;
        verify_terminal(peer.identity(), self.pty)?;
        self.locator.validate()?;
        Ok(())
    }

    fn exchange(
        &self,
        request: Request<'_>,
        lease_deadline: Option<Instant>,
        admit: impl FnOnce() -> bool,
    ) -> io::Result<WireResponse> {
        let bytes = serde_json::to_vec(&Envelope {
            token: &self.locator.manifest.token,
            request,
        })
        .map_err(|_| invalid())?;
        if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
            return Err(invalid());
        }
        self.locator.validate()?;
        let mut stream = connect(&self.locator.manifest.socket)?;
        // 对端正常回包后会关闭连接；只在本连接首次取得内核凭据，之后仍复核活进程和映像。
        let peer = macos_peer_handle(&stream)?;
        self.verify(&peer)?;
        write_admitted_frame(&mut stream, &bytes, lease_deadline, admit)?;
        self.verify(&peer)?;
        let bytes = read_frame(&mut stream, Instant::now() + IO_TIMEOUT)?;
        self.verify(&peer)?;
        serde_json::from_slice(&bytes).map_err(|_| invalid())
    }
}

impl Locator {
    fn read(root: &Path, root_stamp: NodeStamp, directory: PathBuf) -> io::Result<Option<Self>> {
        let metadata = fs::symlink_metadata(&directory)?;
        private_directory(&metadata)?;
        if metadata.dev() != root_stamp.device {
            return Err(invalid());
        }
        let directory_stamp = NodeStamp::of(&metadata);
        let path = directory.join("manifest.json");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        private_file(&metadata)?;
        if !metadata.is_file()
            || metadata.dev() != root_stamp.device
            || metadata.len() == 0
            || metadata.len() > MAX_MANIFEST_BYTES
        {
            return Err(invalid());
        }
        let manifest_stamp = FileStamp::of(&metadata);
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&path)?;
        if !manifest_stamp.matches(&file.metadata()?) {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        (&file)
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != metadata.len() || !manifest_stamp.matches(&file.metadata()?) {
            return Err(invalid());
        }
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let instance = canonical_uuid(&manifest.instance_id)?;
        if manifest.protocol_version != 1
            || manifest.native_pid == 0
            || manifest.native_pid > i32::MAX as u32
            || directory.file_name() != Some(format!("t-{}", instance.simple()).as_ref())
            || manifest.socket != directory.join("control.sock")
            || manifest.token.len() != 64
            || !manifest.token.bytes().all(lower_hex)
        {
            return Err(invalid());
        }
        let socket = fs::symlink_metadata(&manifest.socket)?;
        private_file(&socket)?;
        if !socket.file_type().is_socket() || socket.dev() != root_stamp.device {
            return Err(invalid());
        }
        let locator = Self {
            root: root.to_owned(),
            root_stamp,
            directory,
            directory_stamp,
            socket_stamp: NodeStamp::of(&socket),
            manifest_stamp,
            manifest,
        };
        locator.validate()?;
        Ok(Some(locator))
    }

    fn validate(&self) -> io::Result<()> {
        validate_ancestors(&self.root)?;
        for (path, stamp) in [
            (&self.root, self.root_stamp),
            (&self.directory, self.directory_stamp),
        ] {
            let metadata = fs::symlink_metadata(path)?;
            private_directory(&metadata)?;
            if !stamp.matches(&metadata) {
                return Err(invalid());
            }
        }
        let manifest = fs::symlink_metadata(self.directory.join("manifest.json"))?;
        private_file(&manifest)?;
        let socket = fs::symlink_metadata(&self.manifest.socket)?;
        private_file(&socket)?;
        if !manifest.is_file()
            || !self.manifest_stamp.matches(&manifest)
            || !socket.file_type().is_socket()
            || !self.socket_stamp.matches(&socket)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn parse_state(
    response: WireResponse,
    expected: Uuid,
    issued_before: Instant,
) -> io::Result<NativeBridgeState> {
    let WireResponse::State {
        instance_id,
        input_epoch,
        ready,
        reason,
        lease,
    } = response
    else {
        return Err(invalid());
    };
    if canonical_uuid(&instance_id)? != expected
        || ready != lease.is_some()
        || ready == reason.is_some()
        || reason.as_ref().is_some_and(|reason| !valid_reason(reason))
        || issued_before.elapsed() >= LEASE_LIFETIME
    {
        return Err(invalid());
    }
    let lease = lease
        .map(|wire| {
            canonical_uuid(&wire.lease_id)?;
            if !valid_session(&wire.session_id) || wire.input_epoch != input_epoch {
                return Err(invalid());
            }
            Ok(NativeBridgeLease {
                instance_id: expected,
                issued_before,
                wire,
            })
        })
        .transpose()?;
    Ok(NativeBridgeState {
        ready,
        input_epoch,
        reason,
        lease,
    })
}

fn parse_receipt(
    response: WireResponse,
    expected: &NativeBridgeMessage,
) -> io::Result<NativeBridgeResponse> {
    expected.validate()?;
    match response {
        WireResponse::Receipt { receipt } => {
            if receipt.message_id != expected.message_id.to_string()
                || receipt.prompt_id != receipt.message_id
                || receipt.session_id != expected.session_id
                || receipt.payload_digest != expected.payload_digest
            {
                return Err(invalid());
            }
            Ok(NativeBridgeResponse::Receipt(receipt))
        }
        WireResponse::Unknown { message_id } if message_id == expected.message_id.to_string() => {
            Ok(NativeBridgeResponse::Unknown)
        }
        WireResponse::Rejected { reason } if valid_reason(&reason) => {
            Ok(NativeBridgeResponse::Rejected { reason })
        }
        WireResponse::State { .. }
        | WireResponse::Prepared { .. }
        | WireResponse::NotReady { .. }
        | WireResponse::Unknown { .. }
        | WireResponse::Rejected { .. } => Err(invalid()),
    }
}

fn parse_preparation(
    response: WireResponse,
    expected_instance: Uuid,
    requested_epoch: u64,
) -> io::Result<Option<NativeBridgePreparation>> {
    match response {
        WireResponse::Prepared {
            instance_id,
            input_epoch,
            agent_id,
            expected_binding_epoch,
            expected_session_id,
        } => {
            if canonical_uuid(&instance_id)? != expected_instance
                || requested_epoch.checked_add(1) != Some(input_epoch)
                || !valid_session(&expected_session_id)
            {
                return Err(invalid());
            }
            Ok(Some(NativeBridgePreparation {
                instance_id: expected_instance,
                input_epoch,
                agent_id,
                expected_binding_epoch,
                expected_session_id,
            }))
        }
        WireResponse::NotReady { reason } | WireResponse::Rejected { reason }
            if valid_reason(&reason) =>
        {
            Ok(None)
        }
        WireResponse::State { .. }
        | WireResponse::Receipt { .. }
        | WireResponse::Unknown { .. }
        | WireResponse::NotReady { .. }
        | WireResponse::Rejected { .. } => Err(invalid()),
    }
}

fn connect(path: &Path) -> io::Result<UnixStream> {
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    socket.connect_timeout(&SockAddr::unix(path)?, IO_TIMEOUT)?;
    Ok(socket.into())
}

fn write_admitted_frame(
    stream: &mut UnixStream,
    bytes: &[u8],
    lease_deadline: Option<Instant>,
    admit: impl FnOnce() -> bool,
) -> io::Result<()> {
    if bytes.is_empty()
        || bytes.len() > MAX_FRAME_BYTES
        || lease_deadline.is_some_and(|deadline| Instant::now() >= deadline)
    {
        return Err(invalid());
    }
    // GUI/持久层在最后核验之后原子领取写入许可；拒绝时不能写出任何请求字节。
    if !admit() || lease_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(invalid());
    }
    write_frame(stream, bytes, Instant::now() + IO_TIMEOUT)
}

fn write_frame(stream: &mut UnixStream, bytes: &[u8], deadline: Instant) -> io::Result<()> {
    let frame = [(bytes.len() as u32).to_be_bytes().as_slice(), bytes].concat();
    let mut remaining = frame.as_slice();
    while !remaining.is_empty() {
        stream.set_write_timeout(Some(time_left(deadline)?))?;
        match stream.write(remaining) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(count) => remaining = &remaining[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn read_frame(stream: &mut UnixStream, deadline: Instant) -> io::Result<Vec<u8>> {
    // macOS 在对端完整关闭后拒绝修改 SO_RCVTIMEO，但缓冲中仍可能有完整响应。
    // 单次请求写完后改用非阻塞读取，并以同一绝对期限等待帧头和正文。
    stream.set_nonblocking(true)?;
    let mut prefix = [0u8; 4];
    read_exact(stream, &mut prefix, deadline)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(invalid());
    }
    let mut bytes = vec![0u8; length];
    read_exact(stream, &mut bytes, deadline)?;
    Ok(bytes)
}

fn read_exact(stream: &mut UnixStream, mut bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        let remaining = time_left(deadline)?;
        let timeout = remaining.as_millis().saturating_add(1).min(i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        time_left(deadline)?;
        if result == 0 {
            continue;
        }
        // POLLHUP 也须读取剩余字节；完整帧可以随关闭到达，截断只由实际 EOF 判定。
        match stream.read(bytes) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Ok(count) => bytes = &mut bytes[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn time_left(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|time| !time.is_zero())
        .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
}

fn validate_ancestors(root: &Path) -> io::Result<()> {
    if !root.is_absolute()
        || root
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        || fs::canonicalize(root)? != root
    {
        return Err(invalid());
    }
    let owner = unsafe { libc::geteuid() };
    for path in root.ancestors() {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || (metadata.uid() != owner && metadata.uid() != 0)
            || metadata.mode() & 0o022 != 0
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn private_directory(metadata: &Metadata) -> io::Result<()> {
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(invalid());
    }
    Ok(())
}

fn private_file(metadata: &Metadata) -> io::Result<()> {
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(invalid());
    }
    Ok(())
}

fn canonical_uuid(value: &str) -> io::Result<Uuid> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| !id.is_nil() && id.to_string() == value)
        .ok_or_else(invalid)
}

fn valid_instance_name(value: &str) -> bool {
    value
        .strip_prefix("t-")
        .is_some_and(|value| value.len() == 32 && value.bytes().all(lower_hex))
}

fn lower_hex(value: u8) -> bool {
    value.is_ascii_digit() || (b'a'..=b'f').contains(&value)
}
fn valid_session(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096
}
fn valid_reason(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|value| value.is_ascii_lowercase() || value == b'_')
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "grok_native_bridge.protocol_or_identity",
    )
}

#[cfg(test)]
#[path = "grok_native_bridge_tests.rs"]
mod tests;
