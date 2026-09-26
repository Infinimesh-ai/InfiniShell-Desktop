//! 只保存本地 PTY 创建时的内核身份；不从 shell 环境、历史记录或通知恢复。

use std::ffi::CStr;
use std::fs::{self, File};
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
use std::os::unix::io::AsRawFd as _;
use std::path::Path;

use command::managed::{MacosProcessIdentity, macos_process_identity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LocalPtyIdentity {
    shell: MacosProcessIdentity,
    slave_device: u64,
}

impl LocalPtyIdentity {
    pub(crate) fn capture(shell_pid: u32, master: &File) -> io::Result<Self> {
        // 使用 SCM_RIGHTS 已传入的 master；无需改变 PTY spawner 的二进制消息格式。
        let mut name = [0u8; 128];
        if unsafe {
            libc::ioctl(
                master.as_raw_fd(),
                libc::TIOCPTYGNAME as libc::c_ulong,
                name.as_mut_ptr(),
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let name = CStr::from_bytes_until_nul(&name).map_err(io::Error::other)?;
        let path = Path::new(std::ffi::OsStr::from_bytes(name.to_bytes()));
        let metadata = fs::metadata(path)?;
        if !metadata.file_type().is_char_device() {
            return Err(io::Error::other("本地 PTY 设备类型已变化"));
        }
        let slave_device = metadata.rdev();
        let pid = i32::try_from(shell_pid).map_err(io::Error::other)?;
        if pid <= 0 || slave_device == 0 {
            return Err(io::Error::other("本地 PTY 身份不完整"));
        }
        Ok(Self {
            shell: macos_process_identity(pid)?,
            slave_device,
        })
    }

    /// shell 启动脚本可能 exec；允许映像代际变化，但不能接受复用 PID 的新进程。
    pub(crate) fn current_shell(&self) -> io::Result<MacosProcessIdentity> {
        let current = macos_process_identity(self.shell.pid)?;
        if current.unique_id != self.shell.unique_id
            || current.resource_cid != self.shell.resource_cid
        {
            return Err(io::Error::other("本地 PTY shell 生存期已变化"));
        }
        Ok(current)
    }

    pub(crate) fn slave_device(&self) -> u64 {
        self.slave_device
    }
}

#[cfg(test)]
#[path = "local_pty_identity_tests.rs"]
mod tests;
