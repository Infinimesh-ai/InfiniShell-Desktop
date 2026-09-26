//! 专属 Codex 子进程的持久内核身份；不以裸 PID、路径或计时推断所有权。

use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Token {
    pub pid: i32,
    boot: String,
    values: Vec<u64>,
}

impl Token {
    pub(super) fn capture(pid: i32) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let id = command::managed::linux_process_identity(pid)?;
            Ok(Self {
                pid,
                boot: command::managed::linux_boot_session()?,
                values: vec![
                    id.start_time_ticks,
                    id.proc_inode,
                    id.pid_namespace_device,
                    id.pid_namespace_inode,
                    id.uid.into(),
                    id.executable_device,
                    id.executable_inode,
                ],
            })
        }
        #[cfg(target_os = "macos")]
        {
            let id = command::managed::macos_process_identity(pid)?;
            Ok(Self {
                pid,
                boot: command::managed::macos_boot_session()?,
                values: vec![id.pid_version.into(), id.unique_id, id.resource_cid],
            })
        }
    }

    pub(super) fn validate(&self) -> io::Result<()> {
        if Self::capture(self.pid)? != *self {
            return Err(invalid());
        }
        Ok(())
    }

    pub(super) fn same_lifetime(&self, other: &Self) -> bool {
        if self.pid != other.pid || self.boot != other.boot {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            self.values.len() == 7
                && other.values.len() == 7
                && self.values[..5] == other.values[..5]
        }
        #[cfg(target_os = "macos")]
        {
            self.values.len() == 3 && other.values.len() == 3 && self.values[1] == other.values[1]
        }
    }

    pub(super) fn exited(&self) -> io::Result<bool> {
        #[cfg(target_os = "linux")]
        {
            if command::managed::linux_boot_session()? != self.boot {
                return Ok(true);
            }
            command::managed::linux_process_exited(self.linux_identity()?)
        }
        #[cfg(target_os = "macos")]
        {
            if command::managed::macos_boot_session()? != self.boot {
                return Ok(true);
            }
            let id = self.macos_identity()?;
            match command::managed::macos_process_identity(self.pid) {
                Ok(now) => Ok(now.unique_id != id.unique_id),
                Err(error) => {
                    // 查询失败不是退出；只有内核明确报告进程不存在才能回收。
                    let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
                    let count = unsafe {
                        libc::proc_pidinfo(
                            self.pid,
                            libc::PROC_PIDTBSDINFO,
                            0,
                            (&mut info as *mut libc::proc_bsdinfo).cast(),
                            std::mem::size_of_val(&info) as i32,
                        )
                    };
                    if count == 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                    {
                        Ok(true)
                    } else {
                        Err(error)
                    }
                }
            }
        }
    }

    /// 调用者先证明子进程属于当前持久票据；这里仅防止 PID 复用。
    pub(super) fn terminate(&self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            command::managed::linux_signal_owned_process(
                self.linux_identity()?,
                &self.boot,
                libc::SIGTERM,
            )
        }
        #[cfg(target_os = "macos")]
        {
            command::managed::macos_signal_owned_process(
                self.macos_identity()?,
                &self.boot,
                libc::SIGTERM,
            )
        }
    }

    #[cfg(target_os = "linux")]
    fn linux_identity(&self) -> io::Result<command::managed::LinuxProcessIdentity> {
        let [
            start_time_ticks,
            proc_inode,
            pid_namespace_device,
            pid_namespace_inode,
            uid,
            executable_device,
            executable_inode,
        ] = self.values.as_slice()
        else {
            return Err(invalid());
        };
        Ok(command::managed::LinuxProcessIdentity {
            pid: self.pid,
            start_time_ticks: *start_time_ticks,
            proc_inode: *proc_inode,
            pid_namespace_device: *pid_namespace_device,
            pid_namespace_inode: *pid_namespace_inode,
            uid: (*uid).try_into().map_err(|_| invalid())?,
            executable_device: *executable_device,
            executable_inode: *executable_inode,
        })
    }

    #[cfg(target_os = "macos")]
    fn macos_identity(&self) -> io::Result<command::managed::MacosProcessIdentity> {
        let [pid_version, unique_id, resource_cid] = self.values.as_slice() else {
            return Err(invalid());
        };
        Ok(command::managed::MacosProcessIdentity {
            pid: self.pid,
            pid_version: (*pid_version).try_into().map_err(|_| invalid())?,
            unique_id: *unique_id,
            resource_cid: *resource_cid,
        })
    }
}

fn invalid() -> io::Error {
    io::Error::other("专属 Codex 进程身份不匹配")
}
