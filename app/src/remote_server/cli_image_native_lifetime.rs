//! 原生消费者的只读生存期身份；退出与映像 exec 分开，不提供终止能力。

use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Token {
    pub pid: i32,
    pub(super) boot: String,
    values: Vec<u64>,
}

impl Token {
    pub(super) fn capture(pid: i32) -> io::Result<Self> {
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err(invalid());
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
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return false;
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
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err(invalid());
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

    #[cfg(target_os = "linux")]
    pub(super) fn linux_identity(&self) -> io::Result<command::managed::LinuxProcessIdentity> {
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
    pub(super) fn macos_identity(&self) -> io::Result<command::managed::MacosProcessIdentity> {
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

/// 普通消费者的退出证明还绑定观察上下文；不改变既有专属票据的序列化格式。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeLifetime {
    version: u32,
    process: Token,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linux_observer: Option<LinuxObserver>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinuxObserver {
    pid_namespace: (u64, u64),
    proc_root: (u64, u64),
}

impl NativeLifetime {
    pub(super) fn capture(pid: i32) -> io::Result<Self> {
        let linux_observer = Self::observer()?;
        let process = Token::capture(pid)?;
        if Self::observer()? != linux_observer {
            return Err(invalid());
        }
        Ok(Self {
            version: 1,
            process,
            linux_observer,
        })
    }

    pub(super) fn pid(&self) -> i32 {
        self.process.pid
    }

    pub(super) fn exited(&self) -> io::Result<bool> {
        if self.version != 1 || self.process.pid <= 1 || self.process.boot.is_empty() {
            return Err(invalid());
        }
        #[cfg(target_os = "linux")]
        {
            let identity = self.process.linux_identity()?;
            if identity.start_time_ticks == 0 || self.linux_observer.is_none() {
                return Err(invalid());
            }
            if command::managed::linux_boot_session()? != self.process.boot {
                return Ok(true);
            }
        }
        #[cfg(target_os = "macos")]
        {
            let identity = self.process.macos_identity()?;
            if identity.unique_id == 0 || self.linux_observer.is_some() {
                return Err(invalid());
            }
        }
        // 同 boot 的观察 namespace 或 proc 挂载改变不能解释为原进程退出。
        if Self::observer()? != self.linux_observer {
            return Err(invalid());
        }
        let exited = self.process.exited()?;
        if Self::observer()? != self.linux_observer {
            return Err(invalid());
        }
        Ok(exited)
    }

    fn observer() -> io::Result<Option<LinuxObserver>> {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let namespace = std::fs::metadata("/proc/self/ns/pid")?;
            let root = std::fs::metadata("/proc")?;
            Ok(Some(LinuxObserver {
                pid_namespace: (namespace.dev(), namespace.ino()),
                proc_root: (root.dev(), root.ino()),
            }))
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(None)
        }
    }
}

fn invalid() -> io::Error {
    io::Error::other("原生进程生存期身份不匹配")
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
#[path = "cli_image_native_lifetime_tests.rs"]
mod tests;
