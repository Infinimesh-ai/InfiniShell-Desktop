//! 由实际 ConPTY 派生结果创建身份；保留原始 shell 句柄和本次终端代际。

use std::fmt;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use command::managed::{WindowsProcessIdentity, WindowsProcessLease};
use command::managed_windows_console_probe::{ConsoleProbe, spawn_probe};
use uuid::Uuid;
use windows::Win32::System::Console::HPCON;

#[derive(Clone)]
pub(crate) struct LocalPtyIdentity {
    generation: Uuid,
    shell: Arc<WindowsProcessLease>,
    active: Arc<AtomicBool>,
    console: Arc<Mutex<Option<isize>>>,
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
    /// 调用方必须在 ClosePseudoConsole 之前 invalidate，不能让存储的 HPCON 先失效。
    pub(crate) unsafe fn from_spawned_conpty(
        shell: &impl AsRawHandle,
        console: HPCON,
    ) -> io::Result<Self> {
        if console.0 == 0 {
            return Err(io::Error::other("原 ConPTY 句柄无效"));
        }
        Ok(Self {
            generation: Uuid::new_v4(),
            shell: Arc::new(WindowsProcessLease::from_process_handle(shell)?),
            active: Arc::new(AtomicBool::new(true)),
            console: Arc::new(Mutex::new(Some(console.0))),
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
        if !self.is_active() {
            return Err(io::Error::other("原 ConPTY 已关闭"));
        }
        Ok(self.shell.identity())
    }

    /// 只把锁持有到 CreateProcess 返回；等待和映像摘要核验都在锁外执行。
    pub(crate) fn spawn_console_probe(
        &self,
        native: &WindowsProcessLease,
    ) -> io::Result<ConsoleProbe> {
        let expected = self.current_shell()?;
        native.validate()?;
        let executable = crate::remote_server::rust_ssh::worker_executable()?;
        let held = self
            .console
            .lock()
            .map_err(|_| io::Error::other("原 ConPTY 身份锁失效"))?;
        let console = (*held)
            .filter(|_| self.is_active())
            .ok_or_else(|| io::Error::other("原 ConPTY 已关闭"))?;
        // invalidate 使用同一锁，保证真实 HPCON 在创建期间不能被关闭或重用。
        let probe = unsafe { spawn_probe(HPCON(console), &executable, &self.shell, native) }?;
        drop(held);
        if self.current_shell()? != expected {
            return Err(io::Error::other("原 ConPTY shell 身份已变化"));
        }
        native.validate()?;
        Ok(probe)
    }

    /// 在关闭 ConPTY 之前撤销所有 GUI 快照；进程退出检查仍由保留句柄完成。
    pub(crate) fn invalidate(&self) {
        self.active.store(false, Ordering::Release);
        // 即使先前查询线程 panic，也必须撤销句柄，再允许原拥有者 ClosePseudoConsole。
        self.console
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
}
