//! Linux 普通终端进程的内核身份；所有信号只经过已核对的 pidfd。

use std::ffi::{CString, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::ffi::OsStringExt as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinuxProcessIdentity {
    pub pid: i32,
    pub start_time_ticks: u64,
    pub proc_inode: u64,
    pub pid_namespace_device: u64,
    pub pid_namespace_inode: u64,
    pub uid: u32,
    pub executable_device: u64,
    pub executable_inode: u64,
}

impl LinuxProcessIdentity {
    /// exec 可改变映像，却不能把仍活着的同一进程当成已退出。
    pub fn same_lifetime(self, other: Self) -> bool {
        self.pid == other.pid
            && self.start_time_ticks == other.start_time_ticks
            && self.proc_inode == other.proc_inode
            && self.pid_namespace_device == other.pid_namespace_device
            && self.pid_namespace_inode == other.pid_namespace_inode
    }
}

pub struct LinuxProcessSnapshot {
    pub identity: LinuxProcessIdentity,
    pub parent_pid: i32,
    pub process_group: i32,
    pub session: i32,
    pub tty_device: u64,
    pub foreground_group: i32,
    pub executable: PathBuf,
    pub arguments: Vec<OsString>,
}

/// 长期持有 pidfd；PID 被回收后，该描述符也不会指向新进程。
pub struct LinuxProcessHandle {
    descriptor: OwnedFd,
    identity: LinuxProcessIdentity,
}

impl LinuxProcessHandle {
    pub fn capture(pid: i32) -> io::Result<Self> {
        let descriptor = open_pidfd(pid)?;
        Self::from_pidfd(descriptor, pid)
    }

    fn from_pidfd(descriptor: OwnedFd, pid: i32) -> io::Result<Self> {
        if pid <= 0 || pidfd_pid(&descriptor)? != pid || pidfd_exited(&descriptor)? {
            return Err(io::Error::other("Linux 进程生存期无法绑定"));
        }
        let snapshot = snapshot_with_pidfd(&descriptor, pid)?;
        Ok(Self {
            descriptor,
            identity: snapshot.identity,
        })
    }

    pub fn identity(&self) -> LinuxProcessIdentity {
        self.identity
    }

    pub fn snapshot(&self) -> io::Result<LinuxProcessSnapshot> {
        let snapshot = snapshot_with_pidfd(&self.descriptor, self.identity.pid)?;
        if snapshot.identity != self.identity {
            return Err(io::Error::other("Linux 进程内核身份或映像已改变"));
        }
        Ok(snapshot)
    }

    pub fn exited(&self) -> io::Result<bool> {
        pidfd_exited(&self.descriptor)
    }

    pub fn signal(&self, signal: i32) -> io::Result<()> {
        self.snapshot()?;
        // 信号的最终目标是 pidfd，核对后发生退出也不会命中复用的 PID。
        if unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.descriptor.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        } == -1
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

pub fn linux_boot_session() -> io::Result<String> {
    let value = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let value = value.trim();
    if value.len() != 36
        || !value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err(io::Error::other("Linux 启动身份无效"));
    }
    Ok(value.to_owned())
}

pub fn linux_process_identity(pid: i32) -> io::Result<LinuxProcessIdentity> {
    LinuxProcessHandle::capture(pid).map(|handle| handle.identity())
}

/// 在派发 TUI 前核对 pidfd、procfs 和 Linux 6.5 的对端 pidfd 能力。
pub fn linux_verify_identity_support() -> io::Result<()> {
    let process = LinuxProcessHandle::capture(std::process::id() as i32)?;
    // 信号 0 只检查 pidfd_send_signal 的可用性，不向进程投递信号。
    process.signal(0)?;
    let (socket, _peer) = UnixStream::pair()?;
    let identity = linux_peer_handle(&socket)?.identity();
    if identity != process.identity() {
        return Err(io::Error::other("Linux 本机 socket 内核身份不匹配"));
    }
    Ok(())
}

pub fn linux_peer_handle(stream: &UnixStream) -> io::Result<LinuxProcessHandle> {
    let mut descriptor = -1;
    let mut size = std::mem::size_of::<i32>() as libc::socklen_t;
    // 不支持时保留 ENOPROTOOPT，不以 SO_PEERCRED 的裸 PID 降级。
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERPIDFD,
            (&mut descriptor as *mut i32).cast(),
            &mut size,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if descriptor < 0 {
        return Err(io::Error::other("Linux socket 未返回对端 pidfd"));
    }
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    if size as usize != std::mem::size_of::<i32>() {
        return Err(io::Error::other("Linux socket pidfd 长度无效"));
    }
    let mut credentials = unsafe { std::mem::zeroed::<libc::ucred>() };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if size as usize != std::mem::size_of::<libc::ucred>()
        || credentials.uid != unsafe { libc::geteuid() }
    {
        return Err(io::Error::other("Linux socket 对端用户不匹配"));
    }
    let handle = LinuxProcessHandle::from_pidfd(descriptor, credentials.pid)?;
    if handle.identity.uid != credentials.uid {
        return Err(io::Error::other("Linux socket 对端凭据已改变"));
    }
    Ok(handle)
}

pub fn linux_peer_identity(stream: &UnixStream) -> io::Result<LinuxProcessIdentity> {
    linux_peer_handle(stream).map(|handle| handle.identity())
}

pub fn linux_signal_owned_process(
    expected: LinuxProcessIdentity,
    boot: &str,
    signal: i32,
) -> io::Result<()> {
    if linux_boot_session()? != boot {
        return Err(io::Error::other("Linux 启动代际已改变"));
    }
    let handle = LinuxProcessHandle::capture(expected.pid)?;
    if handle.identity() != expected {
        return Err(io::Error::other("Linux 已保存进程身份不匹配"));
    }
    handle.signal(signal)
}

pub fn linux_process_exited(expected: LinuxProcessIdentity) -> io::Result<bool> {
    let descriptor = match open_pidfd(expected.pid) {
        Ok(descriptor) => descriptor,
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Ok(true),
        Err(error) => return Err(error),
    };
    if pidfd_exited(&descriptor)? {
        return Ok(true);
    }
    let current = snapshot_with_pidfd(&descriptor, expected.pid)?;
    Ok(!expected.same_lifetime(current.identity))
}

fn open_pidfd(pid: i32) -> io::Result<OwnedFd> {
    if pid <= 0 {
        return Err(io::Error::other("Linux PID 无效"));
    }
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor as i32) })
}

fn pidfd_exited(descriptor: &OwnedFd) -> io::Result<bool> {
    let mut poll = libc::pollfd {
        fd: descriptor.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    if unsafe { libc::poll(&mut poll, 1, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if poll.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
        return Err(io::Error::other("Linux pidfd 已失效"));
    }
    Ok(poll.revents & (libc::POLLIN | libc::POLLHUP) != 0)
}

fn pidfd_pid(descriptor: &OwnedFd) -> io::Result<i32> {
    let content = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", descriptor.as_raw_fd()))?;
    content
        .lines()
        .find_map(|line| line.strip_prefix("Pid:\t"))
        .and_then(|pid| pid.parse().ok())
        .ok_or_else(|| io::Error::other("Linux pidfd 缺少可核对的 PID"))
}

fn open_at(directory: &File, name: &str) -> io::Result<File> {
    let name = CString::new(name).map_err(io::Error::other)?;
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn read_at(directory: &File, name: &str, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_at(directory, name)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("Linux 进程元数据过大"));
    }
    Ok(bytes)
}

fn snapshot_with_pidfd(descriptor: &OwnedFd, pid: i32) -> io::Result<LinuxProcessSnapshot> {
    if pidfd_exited(descriptor)? || pidfd_pid(descriptor)? != pid {
        return Err(io::Error::other("Linux 进程已退出"));
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(format!("/proc/{pid}"))?;
    let metadata = directory.metadata()?;
    let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(directory.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { filesystem.assume_init() }.f_type != 0x9fa0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(io::Error::other("Linux procfs 类型或进程用户无效"));
    }
    let stat = read_at(&directory, "stat", 64 * 1024)?;
    let end = stat
        .iter()
        .rposition(|byte| *byte == b')')
        .ok_or_else(|| io::Error::other("Linux stat 缺少命令边界"))?;
    let fields = std::str::from_utf8(&stat[end + 1..])
        .map_err(io::Error::other)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let number = |index: usize| -> io::Result<i64> {
        fields
            .get(index)
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| io::Error::other("Linux stat 字段无效"))
    };
    let executable = open_at(&directory, "exe")?;
    let image = executable.metadata()?;
    let namespace = open_at(&directory, "ns/pid")?.metadata()?;
    let arguments = read_at(&directory, "cmdline", 1024 * 1024)?;
    if !image.is_file() || !arguments.ends_with(&[0]) || number(19)? <= 0 {
        return Err(io::Error::other("Linux 进程映像或参数无效"));
    }
    let snapshot = LinuxProcessSnapshot {
        identity: LinuxProcessIdentity {
            pid,
            start_time_ticks: number(19)? as u64,
            proc_inode: metadata.ino(),
            pid_namespace_device: namespace.dev(),
            pid_namespace_inode: namespace.ino(),
            uid: metadata.uid(),
            executable_device: image.dev(),
            executable_inode: image.ino(),
        },
        parent_pid: i32::try_from(number(1)?).map_err(io::Error::other)?,
        process_group: i32::try_from(number(2)?).map_err(io::Error::other)?,
        session: i32::try_from(number(3)?).map_err(io::Error::other)?,
        tty_device: number(4)? as u32 as u64,
        foreground_group: i32::try_from(number(5)?).map_err(io::Error::other)?,
        executable: std::fs::read_link(format!("/proc/self/fd/{}", executable.as_raw_fd()))?,
        arguments: arguments[..arguments.len() - 1]
            .split(|byte| *byte == 0)
            .map(|part| OsString::from_vec(part.to_vec()))
            .collect(),
    };
    // 每次取样前后都核对 pidfd；proc 目录描述符固定同一生存期，不能随路径复用改变对象。
    let image_after = open_at(&directory, "exe")?.metadata()?;
    if pidfd_exited(descriptor)?
        || pidfd_pid(descriptor)? != pid
        || (image_after.dev(), image_after.ino()) != (image.dev(), image.ino())
    {
        return Err(io::Error::other("Linux 进程取样期间身份改变"));
    }
    Ok(snapshot)
}
