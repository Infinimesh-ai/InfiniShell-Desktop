//! 以 pidfd 固定进程生存期，再用 UNIX_DIAG 的 inode/cookie 双向绑定 socket。

use std::ffi::{CStr, CString, OsString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::mem::{self, MaybeUninit};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use command::managed::{LinuxProcessHandle, LinuxProcessSnapshot, linux_peer_handle};

use super::{Budget, MAX_FDS, MAX_PIDS, ProcessSnapshot, SocketEndpoint, invalid};

const SOCK_DIAG_BY_FAMILY: u16 = 20;
const UNIX_DIAG_SHOW: u32 = 0x01 | 0x02 | 0x04 | 0x40;
const NETLINK_HEADER: usize = 16;
const DIAG_HEADER: usize = 16;
const TCP_ESTABLISHED: u8 = 1;
const TCP_LISTEN: u8 = 10;

pub(super) struct Process {
    handle: LinuxProcessHandle,
}

impl Process {
    pub(super) fn capture(pid: i32) -> io::Result<Self> {
        let process = Self {
            handle: LinuxProcessHandle::capture(pid)?,
        };
        process.snapshot()?;
        Ok(process)
    }

    pub(super) fn same_identity(&self, other: &Self) -> bool {
        self.handle.identity() == other.handle.identity()
    }

    fn directory(&self) -> io::Result<File> {
        let identity = self.handle.identity();
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(format!("/proc/{}", identity.pid))?;
        let metadata = directory.metadata()?;
        let mut filesystem = MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::fstatfs(directory.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { filesystem.assume_init() }.f_type != 0x9fa0
            || metadata.ino() != identity.proc_inode
            || metadata.uid() != identity.uid
            || identity.uid != unsafe { libc::geteuid() }
        {
            return Err(invalid("tmux procfs 身份已改变"));
        }
        Ok(directory)
    }

    pub(super) fn snapshot(&self) -> io::Result<ProcessSnapshot> {
        let before = self.handle.snapshot()?;
        let directory = self.directory()?;
        // cwd 是已验证 procfs 的内核链接；不读取环境中的 PWD。
        let cwd = PathBuf::from(OsString::from_vec(read_link_at(&directory, c"cwd")?));
        if !cwd.is_absolute() || !before.executable.is_absolute() || fs::canonicalize(&cwd)? != cwd
        {
            return Err(invalid("tmux 内核路径无效"));
        }
        let after = self.handle.snapshot()?;
        if !same_snapshot(&before, &after)
            || read_link_at(&directory, c"cwd")? != cwd.as_os_str().as_bytes()
        {
            return Err(invalid("tmux 进程取样期间已改变"));
        }
        Ok(ProcessSnapshot {
            pid: before.identity.pid,
            parent: before.parent_pid,
            uid: before.identity.uid,
            group: before.process_group,
            foreground_group: before.foreground_group,
            session: before.session,
            tty: before.tty_device,
            executable: before.executable,
            executable_file: (
                before.identity.executable_device,
                before.identity.executable_inode,
            ),
            cwd,
        })
    }

    pub(super) fn sockets(&self, budget: &Budget) -> io::Result<Vec<SocketEndpoint>> {
        budget.check()?;
        let before = self.snapshot()?;
        let directory = self.directory()?;
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c"fd".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        let descriptors = unsafe { File::from_raw_fd(descriptor) };
        let entries = fs::read_dir(format!("/proc/self/fd/{}", descriptors.as_raw_fd()))?;
        let mut diag = Diagnostics::open()?;
        let mut endpoints = Vec::new();
        for (count, entry) in entries.enumerate() {
            budget.check()?;
            if count >= MAX_FDS {
                return Err(invalid("tmux 描述符数量超出预算"));
            }
            let name = entry?.file_name();
            let number = decimal(name.as_bytes()).ok_or_else(|| invalid("tmux 描述符编号无效"))?;
            let descriptor = i32::try_from(number).map_err(|_| invalid("tmux 描述符编号越界"))?;
            let name = CString::new(name.as_bytes()).map_err(|_| invalid("tmux 描述符名称无效"))?;
            let link = read_link_at(&descriptors, &name)?;
            let Some(inode) = socket_inode(&link)? else {
                continue;
            };
            let Some(local) = diag.query(inode, None, budget)? else {
                continue;
            };
            if local.kind != libc::SOCK_STREAM as u8 {
                continue;
            }
            if local.uid != before.uid {
                return Err(invalid("tmux socket 用户不匹配"));
            }
            let peer = if local.state == TCP_LISTEN {
                validate_listener(&local)?;
                None
            } else if local.state == TCP_ESTABLISHED {
                // connect 已完成但 server 尚未 accept 时，对端还没有文件 inode。
                // 该端点不参与授权；专属连接校验在同一预算内等待 accept。
                let Some(peer_inode) = local.peer else {
                    continue;
                };
                let peer = diag
                    .query(peer_inode, None, budget)?
                    .ok_or_else(|| invalid("tmux 连接对端已经消失"))?;
                validate_pair(&local, &peer)?;
                if diag.query(peer.inode, Some(peer.cookie), budget)?.as_ref() != Some(&peer) {
                    return Err(invalid("tmux 连接对端 cookie 已改变"));
                }
                Some(peer)
            } else {
                continue;
            };
            if diag
                .query(local.inode, Some(local.cookie), budget)?
                .as_ref()
                != Some(&local)
                || read_link_at(&descriptors, &name)? != link
            {
                return Err(invalid("tmux 描述符或 socket cookie 已改变"));
            }
            endpoints.push(SocketEndpoint {
                descriptor,
                socket: u64::from(local.inode),
                protocol: local.cookie,
                peer_socket: peer.as_ref().map_or(0, |peer| u64::from(peer.inode)),
                peer_protocol: peer.as_ref().map_or(0, |peer| peer.cookie),
                listening: local.state == TCP_LISTEN,
                local_path: local.path,
                peer_path: peer.and_then(|peer| peer.path),
            });
        }
        if before != self.snapshot()? {
            return Err(invalid("tmux socket 取样期间进程已改变"));
        }
        budget.check()?;
        Ok(endpoints)
    }
}

pub(super) fn peer(stream: &UnixStream) -> io::Result<Process> {
    // 不支持 SO_PEERPIDFD 的内核保持拒绝，不能用可复用的裸 PID 代替。
    let process = Process {
        handle: linux_peer_handle(stream)?,
    };
    process.snapshot()?;
    Ok(process)
}

/// 绑定已有 server 的原 pidfd；凭据只作一致性检查，不据其 PID 获取新身份。
pub(super) fn validate_peer(
    stream: &UnixStream,
    expected_server: &Process,
    budget: &Budget,
) -> io::Result<()> {
    budget.check()?;
    let expected = expected_server.snapshot()?;
    let credentials = stream_credentials(stream)?;
    if credentials.0 != expected.pid
        || credentials.1 != expected.uid
        || expected.uid != unsafe { libc::geteuid() }
    {
        return Err(invalid("tmux 新连接凭据与原 server 不匹配"));
    }
    let (inode, cookie) = stream_identity(stream)?;
    let mut diagnostics = Diagnostics::open()?;
    loop {
        budget.check()?;
        if expected_server.snapshot()? != expected {
            return Err(invalid("tmux 新连接核验期间原 server 已改变"));
        }
        let local = diagnostics
            .query(inode, Some(cookie), budget)?
            .ok_or_else(|| invalid("tmux 自有连接已消失"))?;
        if local.kind != libc::SOCK_STREAM as u8
            || local.state != TCP_ESTABLISHED
            || local.uid != expected.uid
        {
            return Err(invalid("tmux 自有连接类型或用户无效"));
        }
        let Some(peer_inode) = local.peer else {
            wait_for_accept(budget)?;
            continue;
        };
        let peer = diagnostics
            .query(peer_inode, None, budget)?
            .ok_or_else(|| invalid("tmux 新连接对端已消失"))?;
        validate_pair(&local, &peer)?;
        let sockets = expected_server.sockets(budget)?;
        let Some(endpoint) = reverse_endpoint(&local, &peer, &sockets)? else {
            // 内核可先为 accept 创建 socket 文件，再把 FD 安装到 server 的表中。
            // 只等待原进程发布该精确端点，绝不依据路径或凭据另找进程。
            wait_for_accept(budget)?;
            continue;
        };
        if diagnostics
            .query(peer.inode, Some(peer.cookie), budget)?
            .as_ref()
            != Some(&peer)
            || diagnostics.query(inode, Some(cookie), budget)?.as_ref() != Some(&local)
            || !expected_server.sockets(budget)?.contains(endpoint)
            || stream_identity(stream)? != (inode, cookie)
            || stream_credentials(stream)? != credentials
            || expected_server.snapshot()? != expected
        {
            return Err(invalid("tmux 新连接的双向端点或原 server 已改变"));
        }
        budget.check()?;
        return Ok(());
    }
}

fn wait_for_accept(budget: &Budget) -> io::Result<()> {
    budget.check()?;
    thread::sleep(
        Duration::from_millis(5).min(budget.deadline.saturating_duration_since(Instant::now())),
    );
    budget.check()
}

fn stream_credentials(stream: &UnixStream) -> io::Result<(i32, u32)> {
    let mut credentials = MaybeUninit::<libc::ucred>::uninit();
    let mut size = mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &mut size,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if size as usize != mem::size_of::<libc::ucred>() {
        return Err(invalid("tmux 新连接凭据长度无效"));
    }
    let credentials = unsafe { credentials.assume_init() };
    if credentials.pid <= 1 {
        return Err(invalid("tmux 新连接凭据 PID 无效"));
    }
    Ok((credentials.pid, credentials.uid))
}

fn stream_identity(stream: &UnixStream) -> io::Result<(u32, u64)> {
    let mut metadata = MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(stream.as_raw_fd(), metadata.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let metadata = unsafe { metadata.assume_init() };
    let inode =
        u32::try_from(metadata.st_ino).map_err(|_| invalid("tmux 自有 socket inode 越界"))?;
    if metadata.st_mode & libc::S_IFMT != libc::S_IFSOCK || inode == 0 {
        return Err(invalid("tmux 自有描述符不是有效 socket"));
    }
    // SO_COOKIE 与 UNIX_DIAG 使用同一 sock_gen_cookie；不从可替换的路径取身份。
    let mut cookie = 0_u64;
    let mut size = mem::size_of_val(&cookie) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_COOKIE,
            (&mut cookie as *mut u64).cast(),
            &mut size,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if size as usize != mem::size_of_val(&cookie) || cookie == 0 || cookie == u64::MAX {
        return Err(invalid("tmux 自有 socket cookie 无效"));
    }
    Ok((inode, cookie))
}

fn reverse_endpoint<'a>(
    local: &DiagSocket,
    peer: &DiagSocket,
    sockets: &'a [SocketEndpoint],
) -> io::Result<Option<&'a SocketEndpoint>> {
    validate_pair(local, peer)?;
    let mut matching = sockets.iter().filter(|endpoint| {
        !endpoint.listening
            && endpoint.descriptor >= 0
            && endpoint.socket == u64::from(peer.inode)
            && endpoint.protocol == peer.cookie
            && endpoint.peer_socket == u64::from(local.inode)
            && endpoint.peer_protocol == local.cookie
    });
    let found = matching.next();
    if matching.next().is_some() {
        return Err(invalid("tmux 新连接的 server 端点不唯一"));
    }
    Ok(found)
}

pub(super) fn pids(group: Option<i32>, budget: &Budget) -> io::Result<Vec<i32>> {
    if group.is_some_and(|value| value <= 0) {
        return Err(invalid("tmux 前台进程组无效"));
    }
    let mut values = Vec::new();
    let mut candidates = 0;
    for entry in fs::read_dir("/proc")? {
        budget.check()?;
        let entry = entry?;
        let Some(pid) =
            decimal(entry.file_name().as_bytes()).and_then(|pid| i32::try_from(pid).ok())
        else {
            continue;
        };
        if pid <= 1 {
            continue;
        }
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            continue;
        }
        candidates += 1;
        if candidates > MAX_PIDS {
            return Err(invalid("tmux 进程候选超出预算"));
        }
        // 枚举只是候选发现；退出或不可读取的候选不会获得任何授权。
        let Ok(handle) = LinuxProcessHandle::capture(pid) else {
            continue;
        };
        let Ok(snapshot) = handle.snapshot() else {
            continue;
        };
        if metadata.ino() == snapshot.identity.proc_inode
            && group.is_none_or(|group| group == snapshot.process_group)
        {
            values.push(pid);
        }
    }
    values.sort_unstable();
    budget.check()?;
    Ok(values)
}

fn same_snapshot(left: &LinuxProcessSnapshot, right: &LinuxProcessSnapshot) -> bool {
    left.identity == right.identity
        && left.parent_pid == right.parent_pid
        && left.process_group == right.process_group
        && left.session == right.session
        && left.tty_device == right.tty_device
        && left.foreground_group == right.foreground_group
        && left.executable == right.executable
}

fn read_link_at(directory: &File, name: &CStr) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; 4096];
    let length = unsafe {
        libc::readlinkat(
            directory.as_raw_fd(),
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if length < 0 {
        return Err(io::Error::last_os_error());
    }
    if length == 0 || length as usize >= bytes.len() {
        return Err(invalid("tmux 内核链接长度无效"));
    }
    bytes.truncate(length as usize);
    if bytes.contains(&0) {
        return Err(invalid("tmux 内核链接含 NUL"));
    }
    Ok(bytes)
}

fn decimal(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty()
        || bytes.len() > 10
        || !bytes.iter().all(u8::is_ascii_digit)
        || (bytes.len() > 1 && bytes[0] == b'0')
    {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn socket_inode(link: &[u8]) -> io::Result<Option<u32>> {
    let Some(value) = link.strip_prefix(b"socket:[") else {
        return Ok(None);
    };
    let inode = value
        .strip_suffix(b"]")
        .and_then(decimal)
        .filter(|inode| *inode != 0)
        .ok_or_else(|| invalid("tmux socket inode 无效"))?;
    Ok(Some(inode))
}

#[derive(Debug, Eq, PartialEq)]
struct DiagSocket {
    inode: u32,
    cookie: u64,
    kind: u8,
    state: u8,
    uid: u32,
    peer: Option<u32>,
    path: Option<PathBuf>,
    vfs: Option<(u32, u32)>,
}

fn validate_pair(local: &DiagSocket, peer: &DiagSocket) -> io::Result<()> {
    if local.state != TCP_ESTABLISHED
        || peer.state != TCP_ESTABLISHED
        || local.kind != libc::SOCK_STREAM as u8
        || peer.kind != libc::SOCK_STREAM as u8
        || local.peer != Some(peer.inode)
        || peer.peer != Some(local.inode)
        || local.inode == peer.inode
        || local.cookie == peer.cookie
        || local.uid != peer.uid
        || local.cookie == 0
        || peer.cookie == 0
        || local.cookie == u64::MAX
        || peer.cookie == u64::MAX
    {
        return Err(invalid("tmux socket 对端不能双向核验"));
    }
    Ok(())
}

fn validate_listener(socket: &DiagSocket) -> io::Result<()> {
    let path = socket
        .path
        .as_ref()
        .ok_or_else(|| invalid("tmux listener 缺少文件路径"))?;
    let (inode, device) = socket
        .vfs
        .ok_or_else(|| invalid("tmux listener 缺少 VFS 身份"))?;
    let metadata = fs::symlink_metadata(path)?;
    // UNIX_DIAG_VFS 的 dev 使用内核 12:20 编码，不能直接与用户态 dev_t 数字比较。
    if socket.state != TCP_LISTEN
        || socket.peer.is_some()
        || metadata.mode() & libc::S_IFMT != libc::S_IFSOCK
        || metadata.uid() != socket.uid
        || metadata.ino() != u64::from(inode)
        || libc::major(metadata.dev()) != device >> 20
        || libc::minor(metadata.dev()) != device & ((1 << 20) - 1)
    {
        return Err(invalid("tmux listener 路径与内核 VFS 不匹配"));
    }
    Ok(())
}

struct Diagnostics {
    descriptor: OwnedFd,
    port: u32,
    sequence: u32,
}

impl Diagnostics {
    fn open() -> io::Result<Self> {
        let descriptor = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                libc::NETLINK_SOCK_DIAG,
            )
        };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
        let mut address: libc::sockaddr_nl = unsafe { mem::zeroed() };
        address.nl_family = libc::AF_NETLINK as _;
        if unsafe {
            libc::bind(
                descriptor.as_raw_fd(),
                (&address as *const libc::sockaddr_nl).cast(),
                mem::size_of_val(&address) as _,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut length = mem::size_of_val(&address) as libc::socklen_t;
        if unsafe {
            libc::getsockname(
                descriptor.as_raw_fd(),
                (&mut address as *mut libc::sockaddr_nl).cast(),
                &mut length,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if length as usize != mem::size_of_val(&address)
            || address.nl_pid == 0
            || address.nl_family != libc::AF_NETLINK as _
            || address.nl_groups != 0
        {
            return Err(invalid("tmux NETLINK 本地地址无效"));
        }
        Ok(Self {
            descriptor,
            port: address.nl_pid,
            sequence: 0,
        })
    }

    fn query(
        &mut self,
        inode: u32,
        cookie: Option<u64>,
        budget: &Budget,
    ) -> io::Result<Option<DiagSocket>> {
        budget.check()?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("tmux NETLINK 序列溢出"))?;
        let request = diag_request(inode, cookie, self.sequence, self.port);
        let mut kernel: libc::sockaddr_nl = unsafe { mem::zeroed() };
        kernel.nl_family = libc::AF_NETLINK as _;
        loop {
            budget.check()?;
            let sent = unsafe {
                libc::sendto(
                    self.descriptor.as_raw_fd(),
                    request.as_ptr().cast(),
                    request.len(),
                    libc::MSG_NOSIGNAL,
                    (&kernel as *const libc::sockaddr_nl).cast(),
                    mem::size_of_val(&kernel) as _,
                )
            };
            if sent == request.len() as isize {
                break;
            }
            if sent >= 0 {
                return Err(invalid("tmux NETLINK 请求短写"));
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(error);
            }
            self.wait(libc::POLLOUT, budget)?;
        }
        let mut bytes = [0; 4096];
        loop {
            self.wait(libc::POLLIN, budget)?;
            let mut source: libc::sockaddr_nl = unsafe { mem::zeroed() };
            let mut length = mem::size_of_val(&source) as libc::socklen_t;
            let received = unsafe {
                libc::recvfrom(
                    self.descriptor.as_raw_fd(),
                    bytes.as_mut_ptr().cast(),
                    bytes.len(),
                    libc::MSG_TRUNC,
                    (&mut source as *mut libc::sockaddr_nl).cast(),
                    &mut length,
                )
            };
            budget.check()?;
            if received < 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) {
                    continue;
                }
                return Err(error);
            }
            if received as usize > bytes.len()
                || length as usize != mem::size_of_val(&source)
                || source.nl_family != libc::AF_NETLINK as _
                || source.nl_pid != 0
                || source.nl_groups != 0
            {
                return Err(invalid("tmux NETLINK 回应来源或长度无效"));
            }
            return parse_response(
                &bytes[..received as usize],
                inode,
                cookie,
                self.sequence,
                self.port,
            );
        }
    }

    fn wait(&self, events: i16, budget: &Budget) -> io::Result<()> {
        loop {
            budget.check()?;
            let timeout = budget
                .deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .max(1);
            let mut descriptor = libc::pollfd {
                fd: self.descriptor.as_raw_fd(),
                events,
                revents: 0,
            };
            let result =
                unsafe { libc::poll(&mut descriptor, 1, timeout.min(i32::MAX as u128) as i32) };
            budget.check()?;
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if result == 0 {
                continue;
            }
            if descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0
                || descriptor.revents & events == 0
            {
                return Err(invalid("tmux NETLINK 描述符已失效"));
            }
            return Ok(());
        }
    }
}

fn diag_request(inode: u32, cookie: Option<u64>, sequence: u32, port: u32) -> [u8; 40] {
    let mut bytes = [0; 40];
    bytes[..4].copy_from_slice(&40_u32.to_ne_bytes());
    bytes[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
    bytes[6..8].copy_from_slice(&1_u16.to_ne_bytes());
    bytes[8..12].copy_from_slice(&sequence.to_ne_bytes());
    bytes[12..16].copy_from_slice(&port.to_ne_bytes());
    bytes[16] = libc::AF_UNIX as u8;
    bytes[20..24].copy_from_slice(&u32::MAX.to_ne_bytes());
    bytes[24..28].copy_from_slice(&inode.to_ne_bytes());
    bytes[28..32].copy_from_slice(&UNIX_DIAG_SHOW.to_ne_bytes());
    let cookie = cookie.unwrap_or(u64::MAX);
    bytes[32..36].copy_from_slice(&(cookie as u32).to_ne_bytes());
    bytes[36..40].copy_from_slice(&((cookie >> 32) as u32).to_ne_bytes());
    bytes
}

fn read_u32(bytes: &[u8]) -> io::Result<u32> {
    Ok(u32::from_ne_bytes(
        bytes
            .try_into()
            .map_err(|_| invalid("tmux NETLINK 数字长度无效"))?,
    ))
}

fn parse_response(
    bytes: &[u8],
    inode: u32,
    cookie: Option<u64>,
    sequence: u32,
    port: u32,
) -> io::Result<Option<DiagSocket>> {
    if bytes.len() < NETLINK_HEADER
        || read_u32(&bytes[..4])? as usize != bytes.len()
        || read_u32(&bytes[8..12])? != sequence
        || read_u32(&bytes[12..16])? != port
        || bytes[6..8] != [0, 0]
    {
        return Err(invalid("tmux NETLINK 回应头无效"));
    }
    let kind = u16::from_ne_bytes([bytes[4], bytes[5]]);
    let body = &bytes[NETLINK_HEADER..];
    if kind == libc::NLMSG_ERROR as u16 {
        if body.len() < 20 {
            return Err(invalid("tmux NETLINK 错误头截断"));
        }
        let error = i32::from_ne_bytes(
            body[..4]
                .try_into()
                .map_err(|_| invalid("tmux NETLINK 错误无效"))?,
        );
        if error == -libc::ENOENT && cookie.is_none() {
            return Ok(None);
        }
        if error >= 0 || error == i32::MIN {
            return Err(invalid("tmux NETLINK 错误码无效"));
        }
        return Err(io::Error::from_raw_os_error(-error));
    }
    if kind != SOCK_DIAG_BY_FAMILY
        || body.len() < DIAG_HEADER
        || body[0] != libc::AF_UNIX as u8
        || body[3] != 0
        || read_u32(&body[4..8])? != inode
        || inode == 0
    {
        return Err(invalid("tmux UNIX_DIAG 回应无效"));
    }
    let actual_cookie =
        u64::from(read_u32(&body[8..12])?) | (u64::from(read_u32(&body[12..16])?) << 32);
    if actual_cookie == 0
        || actual_cookie == u64::MAX
        || cookie.is_some_and(|cookie| cookie != actual_cookie)
    {
        return Err(invalid("tmux UNIX_DIAG cookie 无效"));
    }
    let mut attributes = &body[DIAG_HEADER..];
    let mut seen = 0_u16;
    let mut path = None;
    let mut vfs = None;
    let mut peer = None;
    let mut uid = None;
    while !attributes.is_empty() {
        if attributes.len() < 4 {
            return Err(invalid("tmux UNIX_DIAG 属性头截断"));
        }
        let length = u16::from_ne_bytes([attributes[0], attributes[1]]) as usize;
        let kind = u16::from_ne_bytes([attributes[2], attributes[3]]);
        let padded = (length + 3) & !3;
        if length < 4 || padded > attributes.len() || kind > 7 || seen & (1 << kind) != 0 {
            return Err(invalid("tmux UNIX_DIAG 属性重复或越界"));
        }
        seen |= 1 << kind;
        let value = &attributes[4..length];
        match kind {
            0 => {
                if value.is_empty() || value.len() > 108 {
                    return Err(invalid("tmux UNIX_DIAG 名称长度无效"));
                }
                if value[0] != 0 {
                    let value = value.strip_suffix(&[0]).unwrap_or(value);
                    if value.contains(&0) {
                        return Err(invalid("tmux UNIX_DIAG 名称含 NUL"));
                    }
                    let value = PathBuf::from(OsString::from_vec(value.to_vec()));
                    if !value.is_absolute() {
                        return Err(invalid("tmux UNIX_DIAG 名称不是绝对路径"));
                    }
                    path = Some(value);
                }
            }
            1 => {
                if value.len() != 8 {
                    return Err(invalid("tmux UNIX_DIAG VFS 长度无效"));
                }
                vfs = Some((read_u32(&value[..4])?, read_u32(&value[4..])?));
            }
            2 => {
                let inode = read_u32(value)?;
                // 未 accept 的 stream 对端尚无文件 inode，不能因此授予端点身份。
                peer = (inode != 0).then_some(inode);
            }
            6 => {
                if value.len() != 1 {
                    return Err(invalid("tmux UNIX_DIAG shutdown 长度无效"));
                }
            }
            7 => {
                uid = Some(read_u32(value)?);
            }
            3..=5 => return Err(invalid("tmux UNIX_DIAG 返回未请求的属性")),
            8..=u16::MAX => return Err(invalid("tmux UNIX_DIAG 属性未知")),
        }
        attributes = &attributes[padded..];
    }
    Ok(Some(DiagSocket {
        inode,
        cookie: actual_cookie,
        kind: body[1],
        state: body[2],
        uid: uid.ok_or_else(|| invalid("tmux UNIX_DIAG 缺少用户身份"))?,
        peer,
        path,
        vfs,
    }))
}

#[cfg(test)]
#[path = "tmux_native_linux_tests.rs"]
mod tests;
