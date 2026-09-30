//! Unix 普通 Grok 桥的私有发现、活进程绑定及有界传输。

use std::fs::{self, Metadata, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use command::managed::{
    LinuxProcessHandle as PeerHandle, LinuxProcessIdentity as ProcessIdentity,
    linux_boot_session as boot_session, linux_receive_peer_handle,
};
#[cfg(target_os = "macos")]
use command::managed::{
    MacosPeerHandle as PeerHandle, MacosProcessIdentity as ProcessIdentity,
    macos_boot_session as boot_session, macos_peer_handle as peer_handle, macos_process_terminal,
};
use serde::{Deserialize, Serialize};
use socket2::{Domain, SockAddr, Socket, Type};
use uuid::Uuid;

use super::super::grok_native_bridge_identity::NativeBridgeProcess;
use super::super::grok_native_bridge_identity::{
    ARTIFACT_SHA256, Artifact, FileStamp, verify_terminal,
};
use crate::terminal::model::local_pty_identity::LocalPtyIdentity;

use super::{
    Envelope, IO_TIMEOUT, MAX_FRAME_BYTES, Request, canonical_uuid, invalid, lower_hex,
    valid_instance_name,
};

const MAX_MANIFEST_BYTES: u64 = 4096;
const MAX_DISCOVERY_ENTRIES: usize = 1024;

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

// 包含私有 token 的对象均不实现 Debug；错误只含稳定分类，不携带原始响应。
pub(super) struct Transport {
    locator: Locator,
    pty: LocalPtyIdentity,
    peer: ProcessIdentity,
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

impl Transport {
    pub(crate) fn discover(root: &Path, pty: LocalPtyIdentity) -> io::Result<Option<Self>> {
        let root_metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        validate_ancestors(root)?;
        private_directory(&root_metadata)?;
        let root_stamp = NodeStamp::of(&root_metadata);
        let boot_identity = boot_session()?;
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
            let peer = peer_handle(&stream)?;
            let (tty_device, process_group, foreground_group) = terminal_state(&peer)?;
            // 同一 PTY 中已挂起的旧 Grok 不是当前候选，不能阻挡新的前台实例。
            if tty_device != pty.slave_device()
                || process_group <= 0
                || process_group != foreground_group
            {
                continue;
            }
            if found.is_some() {
                return Err(invalid());
            }
            if peer.identity().pid as u32 != locator.manifest.native_pid {
                return Err(invalid());
            }
            let shell = verify_peer_terminal(&peer, pty)?;
            let artifact = Artifact::capture(&peer)?;
            let binding = NativeBridgeBinding {
                instance_id: canonical_uuid(&locator.manifest.instance_id)?,
                native_process: peer.identity().into(),
                boot_session: boot_identity.clone(),
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
        if boot_session()? != boot_identity || !root_stamp.matches(&fs::symlink_metadata(root)?) {
            return Err(invalid());
        }
        Ok(found)
    }

    pub(crate) fn binding(&self) -> &NativeBridgeBinding {
        &self.binding
    }

    fn verify(&self, peer: &PeerHandle) -> io::Result<()> {
        self.locator.validate()?;
        if boot_session()? != self.binding.boot_session {
            return Err(invalid());
        }
        if peer.identity() != self.peer
            || peer.identity().pid as u32 != self.locator.manifest.native_pid
        {
            return Err(invalid());
        }
        verify_peer_terminal(peer, self.pty)?;
        self.artifact.verify(peer)?;
        verify_peer_terminal(peer, self.pty)?;
        self.locator.validate()?;
        Ok(())
    }

    pub(super) fn exchange(
        &self,
        request: Request<'_>,
        lease_deadline: Option<Instant>,
        admit: impl FnOnce() -> bool,
    ) -> io::Result<Vec<u8>> {
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
        let peer = peer_handle(&stream)?;
        self.verify(&peer)?;
        write_admitted_frame(&mut stream, &bytes, lease_deadline, admit)?;
        self.verify(&peer)?;
        let bytes = read_frame(&mut stream, Instant::now() + IO_TIMEOUT)?;
        self.verify(&peer)?;
        Ok(bytes)
    }
}
#[cfg(target_os = "macos")]
fn terminal_state(peer: &PeerHandle) -> io::Result<(u64, i32, i32)> {
    let terminal = macos_process_terminal(peer.identity())?;
    Ok((
        terminal.tty_device,
        terminal.process_group,
        terminal.foreground_group,
    ))
}

#[cfg(target_os = "linux")]
fn terminal_state(peer: &PeerHandle) -> io::Result<(u64, i32, i32)> {
    let terminal = peer.snapshot()?;
    Ok((
        terminal.tty_device,
        terminal.process_group,
        terminal.foreground_group,
    ))
}

fn verify_peer_terminal(peer: &PeerHandle, pty: LocalPtyIdentity) -> io::Result<ProcessIdentity> {
    #[cfg(target_os = "macos")]
    {
        verify_terminal(peer.identity(), pty)
    }
    #[cfg(target_os = "linux")]
    {
        verify_terminal(peer, pty)
    }
}

#[cfg(target_os = "linux")]
fn peer_handle(stream: &UnixStream) -> io::Result<PeerHandle> {
    linux_receive_peer_handle(stream, Instant::now() + IO_TIMEOUT)
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

fn connect(path: &Path) -> io::Result<UnixStream> {
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    // 连接前启用凭证接收，不能让抢先发来的身份前导丢失内核认证的 ucred。
    #[cfg(target_os = "linux")]
    socket.set_passcred(true)?;
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
        let timeout = remaining
            .as_millis()
            .saturating_add(1)
            .min(i32::MAX as u128) as i32;
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

#[cfg(test)]
#[path = "grok_native_bridge_transport_unix_tests.rs"]
mod tests;
