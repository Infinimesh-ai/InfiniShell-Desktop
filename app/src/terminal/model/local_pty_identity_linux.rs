//! 从实际 PTY master 取得 slave 描述符，Linux 不通过 shell PID 猜测终端路径。

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};

use command::managed::{LinuxProcessHandle, LinuxProcessIdentity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LocalPtyIdentity {
    shell: LinuxProcessIdentity,
    slave_device: u64,
}

impl LocalPtyIdentity {
    pub(crate) fn capture(shell_pid: u32, master: &File) -> io::Result<Self> {
        // TIOCGPTPEER 由内核返回本 master 的 slave；私有挂载或重命名不改变绑定。
        let descriptor = unsafe {
            libc::ioctl(
                master.as_raw_fd(),
                libc::TIOCGPTPEER,
                libc::O_RDONLY | libc::O_NOCTTY | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        let slave = unsafe { File::from_raw_fd(descriptor) };
        let metadata = slave.metadata()?;
        let shell =
            LinuxProcessHandle::capture(i32::try_from(shell_pid).map_err(io::Error::other)?)?
                .snapshot()?;
        if !metadata.file_type().is_char_device()
            || metadata.rdev() == 0
            || shell.tty_device != metadata.rdev()
            || shell.session <= 0
        {
            return Err(io::Error::other("Linux shell 与本地 PTY 不匹配"));
        }
        Ok(Self {
            shell: shell.identity,
            slave_device: metadata.rdev(),
        })
    }

    pub(crate) fn current_shell(&self) -> io::Result<LinuxProcessIdentity> {
        let current = LinuxProcessHandle::capture(self.shell.pid)?.snapshot()?;
        if !self.shell.same_lifetime(current.identity) || current.tty_device != self.slave_device {
            return Err(io::Error::other("Linux 本地 PTY shell 生存期已改变"));
        }
        Ok(current.identity)
    }

    pub(crate) fn slave_device(&self) -> u64 {
        self.slave_device
    }
}
