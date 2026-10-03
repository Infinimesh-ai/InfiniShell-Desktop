//! 接收普通 Linux TUI 在本连接上交付的 pidfd，不以裸 PID 重新取得授权。

use std::io;
use std::mem;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::net::UnixStream;
use std::ptr;
use std::time::Instant;

use super::LinuxProcessHandle;

const IDENTITY_MARKER: u8 = 1;
// 留出额外控制空间，以便接管并关闭恶意多传的 FD；协议本身仍只接受一个。
const CONTROL_WORDS: usize = 32;

pub fn linux_receive_peer_handle(
    stream: &UnixStream,
    deadline: Instant,
) -> io::Result<LinuxProcessHandle> {
    let expected = peer_credentials(stream)?;
    loop {
        remaining_millis(deadline)?;
        let mut marker = 0u8;
        let mut vector = libc::iovec {
            iov_base: (&mut marker as *mut u8).cast(),
            iov_len: 1,
        };
        // usize 保证 cmsghdr 的原生对齐；不能用对齐仅为 1 的字节数组。
        let mut control = [0usize; CONTROL_WORDS];
        let mut message = unsafe { mem::zeroed::<libc::msghdr>() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = mem::size_of_val(&control);
        let count = unsafe {
            libc::recvmsg(
                stream.as_raw_fd(),
                &mut message,
                libc::MSG_CMSG_CLOEXEC | libc::MSG_DONTWAIT,
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            match error.kind() {
                io::ErrorKind::Interrupted => continue,
                io::ErrorKind::WouldBlock => {
                    wait_readable(stream, deadline)?;
                    continue;
                }
                _ => return Err(error),
            }
        }
        // 即使 marker、长度或截断标记错误，也先接管所有内核已安装的 FD，保证失败时关闭。
        let (descriptor, credentials) = collect_identity(&message, &control)?;
        remaining_millis(deadline)?;
        if count != 1
            || marker != IDENTITY_MARKER
            || credentials.pid <= 0
            || credentials.uid != unsafe { libc::geteuid() }
            || !same_credentials(&credentials, &expected)
            || !same_credentials(&peer_credentials(stream)?, &expected)
        {
            return Err(invalid());
        }
        // 信号 0 同时证明这是活进程的 pidfd；普通文件/eventfd 不能伪装成身份凭据。
        if unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                descriptor.as_raw_fd(),
                0,
                ptr::null::<libc::siginfo_t>(),
                0,
            )
        } == -1
        {
            return Err(io::Error::last_os_error());
        }
        let handle = LinuxProcessHandle::from_pidfd(descriptor, credentials.pid)?;
        if handle.identity().uid != credentials.uid {
            return Err(invalid());
        }
        remaining_millis(deadline)?;
        return Ok(handle);
    }
}

fn collect_identity(
    message: &libc::msghdr,
    control: &[usize; CONTROL_WORDS],
) -> io::Result<(OwnedFd, libc::ucred)> {
    let mut descriptors: Vec<OwnedFd> = Vec::new();
    let mut credentials = None;
    let mut rights_messages = 0;
    let mut credential_messages = 0;
    let mut valid = message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) == 0;
    let available = message.msg_controllen.min(mem::size_of_val(control));
    valid &= message.msg_controllen <= mem::size_of_val(control);
    let bytes = control.as_ptr().cast::<u8>();
    let header_length = unsafe { libc::CMSG_LEN(0) } as usize;
    let mut offset = 0usize;
    while available.saturating_sub(offset) >= mem::size_of::<libc::cmsghdr>() {
        let header = unsafe { ptr::read_unaligned(bytes.add(offset).cast::<libc::cmsghdr>()) };
        if header.cmsg_len < header_length || header.cmsg_len > available - offset {
            valid = false;
            break;
        }
        let payload_length = header.cmsg_len - header_length;
        let payload = unsafe { bytes.add(offset + header_length) };
        match (header.cmsg_level, header.cmsg_type) {
            (libc::SOL_SOCKET, libc::SCM_RIGHTS) => {
                rights_messages += 1;
                valid &= payload_length > 0 && payload_length % mem::size_of::<i32>() == 0;
                for index in 0..payload_length / mem::size_of::<i32>() {
                    let raw = unsafe {
                        ptr::read_unaligned(
                            payload.add(index * mem::size_of::<i32>()).cast::<i32>(),
                        )
                    };
                    if raw < 0
                        || descriptors
                            .iter()
                            .any(|descriptor| descriptor.as_raw_fd() == raw)
                    {
                        valid = false;
                    } else {
                        descriptors.push(unsafe { OwnedFd::from_raw_fd(raw) });
                    }
                }
            }
            (libc::SOL_SOCKET, libc::SCM_CREDENTIALS) => {
                credential_messages += 1;
                if payload_length == mem::size_of::<libc::ucred>() {
                    credentials =
                        Some(unsafe { ptr::read_unaligned(payload.cast::<libc::ucred>()) });
                } else {
                    valid = false;
                }
            }
            _ => valid = false,
        }
        offset += unsafe { libc::CMSG_SPACE(payload_length as u32) } as usize;
    }
    // 内核会关闭未能装入截断控制缓冲的 FD；已装入的全部由 descriptors 在失败时释放。
    if !valid || rights_messages != 1 || descriptors.len() != 1 || credential_messages != 1 {
        return Err(invalid());
    }
    Ok((
        descriptors.pop().ok_or_else(invalid)?,
        credentials.ok_or_else(invalid)?,
    ))
}

fn peer_credentials(stream: &UnixStream) -> io::Result<libc::ucred> {
    let mut credentials = unsafe { mem::zeroed::<libc::ucred>() };
    let mut length = mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if length as usize != mem::size_of::<libc::ucred>()
        || credentials.pid <= 0
        || credentials.uid != unsafe { libc::geteuid() }
    {
        return Err(invalid());
    }
    Ok(credentials)
}

fn same_credentials(left: &libc::ucred, right: &libc::ucred) -> bool {
    left.pid == right.pid && left.uid == right.uid && left.gid == right.gid
}

fn wait_readable(stream: &UnixStream, deadline: Instant) -> io::Result<()> {
    loop {
        let timeout = remaining_millis(deadline)?;
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
        remaining_millis(deadline)?;
        if result > 0 {
            // HUP 仍须 recvmsg，以接收关闭前已排队的完整前导；EOF 由实际接收结果拒绝。
            return Ok(());
        }
    }
}

fn remaining_millis(deadline: Instant) -> io::Result<i32> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .map(|remaining| {
            remaining
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32
        })
        .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "Linux socket 身份前导无效")
}
