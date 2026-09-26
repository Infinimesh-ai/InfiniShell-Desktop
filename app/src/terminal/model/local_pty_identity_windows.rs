//! 由实际 ConPTY 派生结果创建身份；保留原始 shell 句柄和本次终端代际。

use std::fmt;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use command::managed::{WindowsProcessIdentity, WindowsProcessLease};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct LocalPtyIdentity {
    generation: Uuid,
    shell: Arc<WindowsProcessLease>,
    active: Arc<AtomicBool>,
}

impl fmt::Debug for LocalPtyIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalPtyIdentity")
            .field("generation", &self.generation)
            .field("shell", &self.shell.identity())
            .finish()
    }
}

impl PartialEq for LocalPtyIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation && self.shell.identity() == other.shell.identity()
    }
}
impl Eq for LocalPtyIdentity {}

impl LocalPtyIdentity {
    /// 只允许 ConPTY 的 CreateProcess 成功分支传入原始进程句柄，不接受枚举的 PID。
    pub(crate) fn from_spawned_conpty(shell: &impl AsRawHandle) -> io::Result<Self> {
        Ok(Self {
            generation: Uuid::new_v4(),
            shell: Arc::new(WindowsProcessLease::from_process_handle(shell)?),
            active: Arc::new(AtomicBool::new(true)),
        })
    }

    pub(crate) fn generation(&self) -> Uuid {
        self.generation
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub(crate) fn current_shell(&self) -> io::Result<WindowsProcessIdentity> {
        if !self.active.load(Ordering::Acquire) {
            return Err(io::Error::other("原 ConPTY 已关闭"));
        }
        self.shell.validate()?;
        Ok(self.shell.identity())
    }

    /// 在关闭 ConPTY 之前撤销所有 GUI 快照；进程退出检查仍由保留句柄完成。
    pub(crate) fn invalidate(&self) {
        self.active.store(false, Ordering::Release);
    }
}
