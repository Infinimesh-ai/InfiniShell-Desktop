//! 独立监督 worker 的进程所有权；不修改普通 Command 的派生或退出行为。

use std::io;
use std::process::{Child, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
#[path = "managed_windows.rs"]
mod windows_identity;
#[cfg(windows)]
pub use windows_identity::{WindowsProcessIdentity, WindowsProcessLease, windows_process_exited};

#[cfg(target_os = "macos")]
#[path = "managed_macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{
    MacosCoalition, MacosProcessIdentity, macos_boot_session, macos_peer_identity,
    macos_process_identity, macos_signal_owned_process,
};

#[cfg(target_os = "linux")]
#[path = "managed_linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    LinuxProcessHandle, LinuxProcessIdentity, LinuxProcessSnapshot, linux_boot_session,
    linux_peer_handle, linux_peer_identity, linux_process_exited, linux_process_identity,
    linux_signal_owned_process, linux_verify_identity_support,
};

#[cfg(target_os = "linux")]
use std::os::unix::process::ExitStatusExt as _;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Containment {
    LinuxSubtree,
    WindowsJob,
    UnixProcessGroup,
}

/// 仅在独立监督进程中调用，避免接管主应用的其他子进程。
pub fn prepare_supervisor() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    // 内核将本监督者子树中的孤儿后代重新交给本进程，包含自行 setsid 的工具。
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// child 必须是本监督者刚派生、尚在等待执行授权的内部 worker。
pub struct ManagedTree {
    child: Child,
    complete: bool,
    confirmed_status: Option<ExitStatus>,
    #[cfg(unix)]
    group_signalled: bool,
    #[cfg(target_os = "linux")]
    root_status: Option<ExitStatus>,
    #[cfg(windows)]
    job: StrictJob,
}

impl ManagedTree {
    pub fn claim(mut child: Child) -> io::Result<Self> {
        #[cfg(unix)]
        if unsafe { libc::getpgid(child.id() as libc::pid_t) } != child.id() as libc::pid_t {
            let error = io::Error::other("托管 worker 未处于自己的进程组");
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        #[cfg(windows)]
        let job = match StrictJob::claim(&child) {
            Ok(job) => job,
            Err(error) => {
                // 此时未发执行授权，失败不能让真实 CLI 继续启动。
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            child,
            complete: false,
            confirmed_status: None,
            #[cfg(unix)]
            group_signalled: false,
            #[cfg(target_os = "linux")]
            root_status: None,
            #[cfg(windows)]
            job,
        })
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn containment(&self) -> Containment {
        #[cfg(target_os = "linux")]
        return Containment::LinuxSubtree;
        #[cfg(target_os = "macos")]
        return Containment::UnixProcessGroup;
        #[cfg(windows)]
        return Containment::WindowsJob;
    }

    /// Unix 保留僵尸 root，直到组清理结束，防止先回收 PID 再误杀复用的进程组。
    pub fn root_exited(&mut self) -> io::Result<bool> {
        #[cfg(unix)]
        {
            let mut information = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id() as libc::id_t,
                    &mut information,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            let pid = unsafe { information.si_pid() };
            #[cfg(target_os = "macos")]
            let pid = information.si_pid;
            Ok(pid != 0)
        }
        #[cfg(windows)]
        {
            self.child.try_wait().map(|status| status.is_some())
        }
    }

    pub fn terminate_and_confirm(&mut self, timeout: Duration) -> io::Result<ExitStatus> {
        if let Some(status) = self.confirmed_status {
            return Ok(status);
        }
        #[cfg(target_os = "linux")]
        let result = self.terminate_linux(timeout);
        #[cfg(target_os = "macos")]
        let result = self.terminate_macos(timeout);
        #[cfg(windows)]
        let result = self.terminate_windows(timeout);
        if let Ok(status) = result {
            self.complete = true;
            self.confirmed_status = Some(status);
        }
        result
    }

    #[cfg(unix)]
    fn signal_owned_group(&mut self) -> io::Result<()> {
        if self.group_signalled {
            return Ok(());
        }
        let result = unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        self.group_signalled = true;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn terminate_linux(&mut self, timeout: Duration) -> io::Result<ExitStatus> {
        self.signal_owned_group()?;
        let deadline = Instant::now() + timeout;
        loop {
            // 只读取内核列出的直属子进程；其 PID 在本监督者 wait 前不会被复用。
            let parent = std::process::id();
            let mut children = std::collections::BTreeSet::new();
            for task in std::fs::read_dir(format!("/proc/{parent}/task"))? {
                let path = task?.path().join("children");
                let contents = match std::fs::read_to_string(path) {
                    Ok(contents) => contents,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                for child in contents.split_whitespace() {
                    children.insert(child.parse::<libc::pid_t>().map_err(io::Error::other)?);
                }
            }
            for pid in children {
                if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(error);
                    }
                }
            }
            loop {
                let mut status = 0;
                let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
                if pid > 0 {
                    if pid as u32 == self.child.id() {
                        self.root_status = Some(ExitStatus::from_raw(status));
                    }
                    continue;
                }
                if pid < 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::ECHILD) {
                        return self
                            .root_status
                            .ok_or_else(|| io::Error::other("缺少原托管进程的退出状态"));
                    }
                    if error.raw_os_error() != Some(libc::EINTR) {
                        return Err(error);
                    }
                }
                break;
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "托管子树退出未确认",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(target_os = "macos")]
    fn terminate_macos(&mut self, timeout: Duration) -> io::Result<ExitStatus> {
        // macOS 向只剩僵尸的组发信号会返回 EPERM；已经完整退出时不应再请求终止。
        // 真正活跃的组仍要求信号成功，不能把任意 EPERM 当作已退出。
        if !macos_group_has_live_members(self.child.id())? && self.root_exited()? {
            self.complete = true;
            return self.child.wait();
        }
        if let Err(error) = self.signal_owned_group() {
            // 最后一个活跃成员可能在快照之后自然退出；仍须重新取得完整退出证明。
            if error.raw_os_error() == Some(libc::EPERM)
                && !macos_group_has_live_members(self.child.id())?
                && self.root_exited()?
            {
                self.complete = true;
                return self.child.wait();
            }
            return Err(error);
        }
        let deadline = Instant::now() + timeout;
        loop {
            if !macos_group_has_live_members(self.child.id())? && self.root_exited()? {
                // 从此以后不再向该组发信号；wait 可能释放原 PID 给别的进程。
                self.complete = true;
                return self.child.wait();
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "托管进程组退出未确认",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(windows)]
    fn terminate_windows(&mut self, timeout: Duration) -> io::Result<ExitStatus> {
        self.job.terminate()?;
        let deadline = Instant::now() + timeout;
        while self.job.active_processes()? != 0 {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "托管 Job 退出未确认",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.child.wait()
    }
}

impl Drop for ManagedTree {
    fn drop(&mut self) {
        if !self.complete {
            // 监督者的错误路径仍尽力清理；没有成功回执就不能被当作已确认退出。
            let _ = self.terminate_and_confirm(Duration::from_secs(3));
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_group_has_live_members(group: u32) -> io::Result<bool> {
    // 当前 SDK sys/proc_info.h 的 PROC_PGRP_ONLY；查询严格限于本次拥有的组。
    const PROC_PGRP_ONLY: u32 = 2;
    let needed = unsafe { libc::proc_listpids(PROC_PGRP_ONLY, group, std::ptr::null_mut(), 0) };
    if needed < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pids = vec![0i32; needed as usize / size_of::<i32>() + 32];
    let bytes = unsafe {
        libc::proc_listpids(
            PROC_PGRP_ONLY,
            group,
            pids.as_mut_ptr().cast(),
            (pids.len() * size_of::<i32>()) as i32,
        )
    };
    if bytes < 0 || bytes as usize >= pids.len() * size_of::<i32>() {
        return Err(io::Error::other("进程组快照不完整"));
    }
    for pid in pids
        .into_iter()
        .take(bytes as usize / size_of::<i32>())
        .filter(|pid| *pid > 0)
    {
        let mut information = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
        let count = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut information as *mut libc::proc_bsdinfo).cast(),
                size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if count == 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            continue;
        }
        if count as usize != size_of::<libc::proc_bsdinfo>() {
            return Err(io::Error::other("无法确认受管理组成员状态"));
        }
        if information.pbi_pgid == group && information.pbi_status != libc::SZOMB {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(windows)]
struct StrictJob(std::os::windows::io::OwnedHandle);

#[cfg(windows)]
impl StrictJob {
    fn claim(child: &Child) -> io::Result<Self> {
        use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _};
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        let handle = unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::other)?;
        let owned = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle.0) };
        let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&information as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .map_err(io::Error::other)?;
            AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle()))
                .map_err(io::Error::other)?;
        }
        Ok(Self(owned))
    }

    fn terminate(&self) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle as _;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::TerminateJobObject;
        unsafe { TerminateJobObject(HANDLE(self.0.as_raw_handle()), 1) }.map_err(io::Error::other)
    }

    fn active_processes(&self) -> io::Result<u32> {
        use std::os::windows::io::AsRawHandle as _;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };
        let mut information = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(HANDLE(self.0.as_raw_handle())),
                JobObjectBasicAccountingInformation,
                (&mut information as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
        }
        .map_err(io::Error::other)?;
        Ok(information.ActiveProcesses)
    }
}

#[cfg(test)]
#[path = "managed_tests.rs"]
mod tests;
