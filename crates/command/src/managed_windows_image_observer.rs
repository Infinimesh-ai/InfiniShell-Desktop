//! 普通 CLI 的短暂映像观察；调试连接不拥有用户进程的生杀权。

use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _};
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, DUPLICATE_HANDLE_OPTIONS, DuplicateHandle,
    ERROR_SEM_TIMEOUT, EXCEPTION_BREAKPOINT, GENERIC_READ, HANDLE, NTSTATUS, WAIT_OBJECT_0,
};
use windows::Win32::System::Diagnostics::Debug::{
    CREATE_PROCESS_DEBUG_EVENT, CREATE_PROCESS_DEBUG_INFO, CREATE_THREAD_DEBUG_EVENT,
    ContinueDebugEvent, DEBUG_EVENT, DebugActiveProcess, DebugActiveProcessStop,
    DebugSetProcessKillOnExit, EXCEPTION_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT,
    EXIT_THREAD_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT, OUTPUT_DEBUG_STRING_EVENT, RIP_EVENT,
    UNLOAD_DLL_DEBUG_EVENT, WaitForDebugEvent,
};
use windows::Win32::System::Threading::{
    DEBUG_ONLY_THIS_PROCESS, GetCurrentProcess, WaitForSingleObject,
};
use windows::core::HRESULT;

use crate::blocking::Command;
use crate::managed::WindowsProcessLease;

const EVENT_TIMEOUT: Duration = Duration::from_secs(5);
const HELPER_EXIT_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_EVENTS: usize = 4096;

/// 返回内核调试事件提供的实际主映像文件，不按进程路径重新打开文件。
///
/// 调用方须提供可信的私有 bootstrap 命令：只读取 stdin 至 EOF 后退出，不执行用户
/// 工作、不派生子进程。本函数强制独立调试启动、管道 stdin 和空 stdout/stderr；
/// 不把目标放入 Job，也不终止目标。调用方应在后台执行，并继续核验映像摘要及管道/PTY。
pub fn observe_process_image(target: &WindowsProcessLease, bootstrap: Command) -> io::Result<File> {
    target.validate()?;
    if target.identity().pid == std::process::id() {
        return Err(io::Error::other("映像观察器不能附加自身"));
    }
    let held = WindowsProcessLease::from_process_handle(target)?;
    let worker = thread::Builder::new()
        .name("grok-image-observer".to_owned())
        .spawn(move || -> io::Result<File> {
            let mut observer = Observer::start(bootstrap)?;
            observer.attach(held)?;
            let image = observer.observe()?;
            observer.detach_target()?;
            observer.finish_helper()?;
            Ok(image)
        })?;
    // 先等专用线程退出；即使显式 detach 失败，已设为 FALSE 的线程退出策略仍负责脱离。
    let image = worker
        .join()
        .map_err(|_| io::Error::other("映像观察线程异常退出"))??;
    target.validate()?;
    Ok(image)
}

struct Observer {
    helper: Child,
    helper_attached: bool,
    helper_exited: bool,
    kill_on_exit_disabled: bool,
    helper_ready: bool,
    target: Option<WindowsProcessLease>,
    target_ready: bool,
    image: Option<File>,
}

impl Observer {
    fn start(mut bootstrap: Command) -> io::Result<Self> {
        if !std::path::Path::new(bootstrap.get_program()).is_absolute() {
            return Err(io::Error::other("映像观察 bootstrap 必须使用绝对路径"));
        }
        // 不继承 caller 提供的 stdio；进程/文件副本均不可继承，不传额外原生句柄。
        bootstrap
            .creation_flags(DEBUG_ONLY_THIS_PROCESS.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let helper = bootstrap.spawn()?;
        let mut observer = Self {
            helper,
            helper_attached: true,
            helper_exited: false,
            kill_on_exit_disabled: false,
            helper_ready: false,
            target: None,
            target_ready: false,
            image: None,
        };
        // 官方合同要求先有调试连接。此时只有私有 helper，用户进程尚未被附加。
        // FALSE 同时适用于本线程未来的 debuggee，整个观察期绝不改回 TRUE。
        unsafe { DebugSetProcessKillOnExit(false) }.map_err(io::Error::from)?;
        observer.kill_on_exit_disabled = true;
        observer.pump_until(Instant::now() + EVENT_TIMEOUT, |observer| {
            observer.helper_ready
        })?;
        Ok(observer)
    }

    fn attach(&mut self, target: WindowsProcessLease) -> io::Result<()> {
        if !self.kill_on_exit_disabled || !self.helper_ready || self.helper_exited {
            return Err(io::Error::other("映像观察安全启动尚未完成"));
        }
        if target.identity().pid == self.helper.id() || self.target.is_some() {
            return Err(io::Error::other("映像观察目标不合法"));
        }
        target.validate()?;
        if self.helper.try_wait()?.is_some() {
            return Err(io::Error::other("映像观察 bootstrap 已退出"));
        }
        unsafe { DebugActiveProcess(target.identity().pid) }.map_err(io::Error::from)?;
        self.target = Some(target);
        Ok(())
    }

    fn observe(&mut self) -> io::Result<File> {
        self.pump_until(Instant::now() + EVENT_TIMEOUT, |observer| {
            observer.target_ready && observer.image.is_some()
        })?;
        self.target
            .as_ref()
            .ok_or_else(|| io::Error::other("映像观察没有目标"))?
            .validate()?;
        self.image
            .take()
            .ok_or_else(|| io::Error::other("原生调试事件缺少映像句柄"))
    }

    fn detach_target(&mut self) -> io::Result<()> {
        if let Some(target) = &self.target {
            unsafe { DebugActiveProcessStop(target.identity().pid) }.map_err(io::Error::from)?;
            self.target = None;
        }
        Ok(())
    }

    fn finish_helper(&mut self) -> io::Result<()> {
        // 只有目标明确脱离后才关闭 helper 的 stdin；退出事件仍由同一线程继续。
        if self.target.is_some() {
            return Err(io::Error::other("原生目标尚未脱离映像观察"));
        }
        drop(self.helper.stdin.take());
        self.pump_until(Instant::now() + HELPER_EXIT_TIMEOUT, |observer| {
            observer.helper_exited
        })?;
        let status = unsafe {
            WaitForSingleObject(
                HANDLE(self.helper.as_raw_handle()),
                HELPER_EXIT_TIMEOUT.as_millis() as u32,
            )
        };
        if status != WAIT_OBJECT_0
            || !self
                .helper
                .try_wait()?
                .is_some_and(|status| status.success())
        {
            return Err(io::Error::other("映像观察 bootstrap 未正常退出"));
        }
        Ok(())
    }

    fn pump_until(
        &mut self,
        deadline: Instant,
        finished: impl Fn(&Self) -> bool,
    ) -> io::Result<()> {
        for _ in 0..MAX_EVENTS {
            if finished(self) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "映像观察超时"));
            }
            let mut event = DEBUG_EVENT::default();
            let timeout_ms = remaining.as_millis().min(100).max(1) as u32;
            match unsafe { WaitForDebugEvent(&mut event, timeout_ms) } {
                Ok(()) => {}
                Err(error) if error.code() == HRESULT::from_win32(ERROR_SEM_TIMEOUT.0) => {
                    continue;
                }
                Err(error) => return Err(io::Error::from(error)),
            }
            let is_helper = event.dwProcessId == self.helper.id();
            let is_target = self
                .target
                .as_ref()
                .is_some_and(|target| target.identity().pid == event.dwProcessId);
            let initial_breakpoint = event.dwDebugEventCode == EXCEPTION_DEBUG_EVENT
                && unsafe { event.u.Exception.ExceptionRecord.ExceptionCode }
                    == EXCEPTION_BREAKPOINT
                && ((is_helper && !self.helper_ready) || (is_target && !self.target_ready));
            let status = if event.dwDebugEventCode == EXCEPTION_DEBUG_EVENT && !initial_breakpoint {
                DBG_EXCEPTION_NOT_HANDLED
            } else {
                DBG_CONTINUE
            };
            let pending = PendingEvent { event, status };
            let result = self.process_event(&pending.event, is_helper, is_target);
            let continued = pending.resume();
            result?;
            continued?;
            if initial_breakpoint {
                if is_helper {
                    self.helper_ready = true;
                } else {
                    self.target_ready = true;
                }
            }
        }
        Err(io::Error::other("映像观察事件超过上限"))
    }

    fn process_event(
        &mut self,
        event: &DEBUG_EVENT,
        is_helper: bool,
        is_target: bool,
    ) -> io::Result<()> {
        // 即使事件被拒绝，也先接管所有应由调试器关闭的映像句柄。
        let file = match event.dwDebugEventCode {
            CREATE_PROCESS_DEBUG_EVENT => image_file(unsafe { event.u.CreateProcessInfo.hFile }),
            LOAD_DLL_DEBUG_EVENT => image_file(unsafe { event.u.LoadDll.hFile }),
            _code => None,
        };
        if !is_helper && !is_target {
            return Err(io::Error::other("映像观察收到其它进程事件"));
        }
        match event.dwDebugEventCode {
            CREATE_PROCESS_DEBUG_EVENT if is_target => {
                if self.image.is_some() {
                    return Err(io::Error::other("映像观察收到重复主映像事件"));
                }
                let target = self
                    .target
                    .as_ref()
                    .ok_or_else(|| io::Error::other("映像观察没有目标"))?;
                verify_event_process(unsafe { &event.u.CreateProcessInfo }, target)?;
                self.image =
                    Some(readonly_image(file.ok_or_else(|| {
                        io::Error::other("原生调试事件缺少映像句柄")
                    })?)?);
                Ok(())
            }
            EXIT_PROCESS_DEBUG_EVENT if is_helper => {
                self.helper_exited = true;
                self.helper_attached = false;
                if self.target.is_some() {
                    Err(io::Error::other("映像观察 bootstrap 提前退出"))
                } else {
                    Ok(())
                }
            }
            EXIT_PROCESS_DEBUG_EVENT => Err(io::Error::other("映像观察期间原生进程退出")),
            RIP_EVENT => Err(io::Error::other("映像观察收到系统调试错误")),
            CREATE_PROCESS_DEBUG_EVENT
            | CREATE_THREAD_DEBUG_EVENT
            | EXIT_THREAD_DEBUG_EVENT
            | LOAD_DLL_DEBUG_EVENT
            | UNLOAD_DLL_DEBUG_EVENT
            | OUTPUT_DEBUG_STRING_EVENT
            | EXCEPTION_DEBUG_EVENT => Ok(()),
            _code => Err(io::Error::other("映像观察收到未知调试事件")),
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        // 目标脱离失败时仍保留 FALSE 策略，线程退出完成最后脱离；绝不杀目标。
        let _ = self.detach_target();
        if self.helper_attached {
            let _ = unsafe { DebugActiveProcessStop(self.helper.id()) };
        }
        drop(self.helper.stdin.take());
        if !self.helper_exited {
            // 只有本模块刚创建的私有 helper 可以终止，且只使用原 Child 句柄。
            let helper = HANDLE(self.helper.as_raw_handle());
            if unsafe { WaitForSingleObject(helper, 1000) } != WAIT_OBJECT_0 {
                let _ = self.helper.kill();
                let _ = unsafe { WaitForSingleObject(helper, 1000) };
            }
        }
    }
}

struct PendingEvent {
    event: DEBUG_EVENT,
    status: NTSTATUS,
}

impl PendingEvent {
    fn resume(self) -> io::Result<()> {
        let result = unsafe {
            ContinueDebugEvent(self.event.dwProcessId, self.event.dwThreadId, self.status)
        }
        .map_err(io::Error::from);
        // 成败都只显式 continue 一次；失败由 Observer 脱离及线程退出处理。
        std::mem::forget(self);
        result
    }
}

impl Drop for PendingEvent {
    fn drop(&mut self) {
        let _ = unsafe {
            ContinueDebugEvent(self.event.dwProcessId, self.event.dwThreadId, self.status)
        };
    }
}

fn image_file(handle: HANDLE) -> Option<File> {
    if handle.is_invalid() {
        None
    } else {
        Some(unsafe { File::from_raw_handle(handle.0) })
    }
}

fn readonly_image(file: File) -> io::Result<File> {
    // 同一内核文件对象降权复制；关闭原始调试 hFile，不向调用方暴露写权限或可继承句柄。
    let current = unsafe { GetCurrentProcess() };
    let mut duplicate = HANDLE::default();
    unsafe {
        DuplicateHandle(
            current,
            HANDLE(file.as_raw_handle()),
            current,
            &mut duplicate,
            GENERIC_READ.0,
            false,
            DUPLICATE_HANDLE_OPTIONS(0),
        )
    }
    .map_err(io::Error::from)?;
    Ok(unsafe { File::from_raw_handle(duplicate.0) })
}

fn verify_event_process(
    event: &CREATE_PROCESS_DEBUG_INFO,
    target: &WindowsProcessLease,
) -> io::Result<()> {
    if event.hProcess.is_invalid() {
        return Err(io::Error::other("原生调试事件缺少进程句柄"));
    }
    // 原 process/thread 句柄归调试系统；只借用并复制查询权限，不提前关闭原件。
    let borrowed = unsafe { BorrowedHandle::borrow_raw(event.hProcess.0) };
    let observed = WindowsProcessLease::from_process_handle(&borrowed)?;
    target.validate()?;
    if observed.identity() != target.identity() {
        return Err(io::Error::other("原生调试事件进程身份不匹配"));
    }
    observed.validate()?;
    target.validate()
}

#[cfg(test)]
#[path = "managed_windows_image_observer_tests.rs"]
mod tests;
