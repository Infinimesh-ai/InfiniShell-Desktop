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
    MacosCoalition, MacosPeerHandle, MacosProcessIdentity, MacosTerminalSnapshot,
    macos_boot_session, macos_peer_handle, macos_peer_identity, macos_process_identity,
    macos_process_terminal, macos_signal_owned_process,
};

#[cfg(target_os = "linux")]
#[path = "managed_linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    LinuxProcessHandle, LinuxProcessIdentity, LinuxProcessSnapshot, linux_boot_session,
    linux_peer_handle, linux_peer_identity, linux_process_exited, linux_process_identity,
    linux_receive_peer_handle, linux_signal_owned_process, linux_verify_identity_support,
};

#[cfg(target_os = "linux")]
use std::os::unix::process::ExitStatusExt as _;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Containment {
    LinuxSubtree,
    WindowsJob,
    UnixProcessGroup,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ProbeJobOperation {
    ClaimBootstrap,
    VerifyCandidate,
}

/// 仅在原认证控制连接传输；句柄值属于仍存活的原执行 worker，不能当作 PID 打开。
#[cfg(windows)]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeJobRequest {
    operation: ProbeJobOperation,
    source_handle: u64,
    identity: probe_job::Identity,
}

#[cfg(windows)]
impl ProbeJobRequest {
    pub fn capture(
        operation: ProbeJobOperation,
        process: std::os::windows::io::BorrowedHandle<'_>,
    ) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle as _;
        let raw = windows::Win32::Foundation::HANDLE(process.as_raw_handle());
        probe_job::alive(raw)?;
        Ok(Self {
            operation,
            source_handle: process.as_raw_handle() as usize as u64,
            identity: probe_job::identity(raw)?,
        })
    }

    pub fn operation(&self) -> ProbeJobOperation {
        self.operation
    }
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
    #[cfg(windows)]
    probe_job: probe_job::Authorization,
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
            #[cfg(windows)]
            probe_job: probe_job::Authorization::Initial,
        })
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// 监督者保留唯一原 Job 句柄；失败后本代不再接受任何进程授权请求。
    #[cfg(windows)]
    pub fn authorize_probe_process(
        &mut self,
        request: &ProbeJobRequest,
        expected_image: &std::path::Path,
    ) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle as _;
        use windows::Win32::Foundation::HANDLE;
        if self.complete {
            return Err(io::Error::other("托管 Job 已经结束"));
        }
        let previous = std::mem::replace(&mut self.probe_job, probe_job::Authorization::Rejected);
        self.probe_job = probe_job::authorize(
            previous,
            request,
            expected_image,
            HANDLE(self.child.as_raw_handle()),
            HANDLE(self.job.0.as_raw_handle()),
        )?;
        Ok(())
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

#[cfg(windows)]
mod probe_job {
    use super::{ProbeJobOperation, ProbeJobRequest};
    use std::ffi::OsString;
    use std::io;
    use std::mem::size_of_val;
    use std::os::windows::ffi::OsStringExt as _;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use std::path::{Path, PathBuf};
    use windows::Win32::Foundation::{
        DUPLICATE_HANDLE_OPTIONS, DuplicateHandle, FILETIME, HANDLE, WAIT_FAILED, WAIT_TIMEOUT,
    };
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER, TokenIsAppContainer,
        TokenSessionId, TokenStatistics, TokenUser,
    };
    use windows::Win32::System::JobObjects::{AssignProcessToJobObject, IsProcessInJob};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessId, GetProcessTimes, OpenProcessToken, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE, QueryFullProcessImageNameW, WaitForSingleObject,
    };
    use windows::core::{BOOL, PWSTR};

    #[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct Identity {
        pid: u32,
        created_at: u64,
        logon_low: u32,
        logon_high: i32,
        session_id: u32,
        owner: Vec<u8>,
        image: PathBuf,
        appcontainer: u32,
    }

    pub(super) enum Authorization {
        Initial,
        Bootstrap {
            worker: Identity,
            bootstrap: Identity,
        },
        Complete,
        Rejected,
    }

    pub(super) fn alive(process: HANDLE) -> io::Result<()> {
        let result = unsafe { WaitForSingleObject(process, 0) };
        if result == WAIT_TIMEOUT {
            Ok(())
        } else if result == WAIT_FAILED {
            Err(io::Error::last_os_error())
        } else {
            Err(io::Error::other("Job 授权进程已退出或无法确认"))
        }
    }

    fn token_value<T: Default>(
        token: HANDLE,
        class: windows::Win32::Security::TOKEN_INFORMATION_CLASS,
    ) -> io::Result<T> {
        let mut value = T::default();
        let mut size = 0;
        unsafe {
            GetTokenInformation(
                token,
                class,
                Some((&mut value as *mut T).cast()),
                size_of::<T>() as u32,
                &mut size,
            )
        }
        .map_err(io::Error::from)?;
        if size as usize != size_of::<T>() {
            return Err(io::Error::other("Job 授权令牌字段长度不匹配"));
        }
        Ok(value)
    }

    pub(super) fn identity(process: HANDLE) -> io::Result<Identity> {
        let pid = unsafe { GetProcessId(process) };
        if pid == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
            .map_err(io::Error::from)?;
        let mut token = HANDLE::default();
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(io::Error::from)?;
        let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
        let raw = HANDLE(token.as_raw_handle());
        let statistics: TOKEN_STATISTICS = token_value(raw, TokenStatistics)?;
        // 固定对齐缓冲仅读取 SID，边界检查先于任何 SID 内容访问。
        let mut storage = [0usize; 512];
        let mut length = 0;
        unsafe {
            GetTokenInformation(
                raw,
                TokenUser,
                Some(storage.as_mut_ptr().cast()),
                size_of_val(&storage) as u32,
                &mut length,
            )
        }
        .map_err(io::Error::from)?;
        if (length as usize) < size_of::<TOKEN_USER>() || length as usize > size_of_val(&storage) {
            return Err(io::Error::other("Job 授权用户字段长度不匹配"));
        }
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        let start = storage.as_ptr() as usize;
        let end = start + length as usize;
        let sid = user.User.Sid.0 as usize;
        if sid < start || sid.checked_add(8).is_none_or(|value| value > end) {
            return Err(io::Error::other("Job 授权用户 SID 边界无效"));
        }
        let header = unsafe { std::slice::from_raw_parts(sid as *const u8, 8) };
        let sid_length = 8 + usize::from(header[1]) * 4;
        if header[0] != 1
            || header[1] > 15
            || sid.checked_add(sid_length).is_none_or(|value| value > end)
        {
            return Err(io::Error::other("Job 授权用户 SID 无效"));
        }
        let owner = unsafe { std::slice::from_raw_parts(sid as *const u8, sid_length) }.to_vec();
        let mut image = [0u16; 32768];
        let mut size = image.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(image.as_mut_ptr()),
                &mut size,
            )
        }
        .map_err(io::Error::from)?;
        if size == 0 || size as usize >= image.len() {
            return Err(io::Error::other("Job 授权映像路径无效"));
        }
        Ok(Identity {
            pid,
            created_at: (u64::from(creation.dwHighDateTime) << 32)
                | u64::from(creation.dwLowDateTime),
            logon_low: statistics.AuthenticationId.LowPart,
            logon_high: statistics.AuthenticationId.HighPart,
            session_id: token_value(raw, TokenSessionId)?,
            owner,
            image: PathBuf::from(OsString::from_wide(&image[..size as usize])).canonicalize()?,
            appcontainer: token_value(raw, TokenIsAppContainer)?,
        })
    }

    pub(super) fn accepts_stage(state: &Authorization, operation: ProbeJobOperation) -> bool {
        matches!(
            (state, operation),
            (Authorization::Initial, ProbeJobOperation::ClaimBootstrap)
                | (
                    Authorization::Bootstrap { .. },
                    ProbeJobOperation::VerifyCandidate
                )
        )
    }

    pub(super) fn validate_identity(
        previous: &Authorization,
        request: &ProbeJobRequest,
        expected_image: &Path,
        worker: &Identity,
        actual: &Identity,
    ) -> io::Result<()> {
        if !accepts_stage(previous, request.operation)
            || *actual != request.identity
            || actual.pid == worker.pid
            || actual.owner != worker.owner
            || actual.session_id != worker.session_id
            || actual.image != expected_image
            || actual.created_at < worker.created_at
        {
            return Err(io::Error::other("Job 授权原进程身份不匹配"));
        }
        match previous {
            Authorization::Initial => {
                if actual.appcontainer != 0
                    || (actual.logon_low, actual.logon_high)
                        == (worker.logon_low, worker.logon_high)
                {
                    return Err(io::Error::other("Job 引导器未处于本次新登录身份"));
                }
            }
            Authorization::Bootstrap {
                worker: original_worker,
                bootstrap,
            } => {
                if original_worker != worker
                    || actual.appcontainer != 1
                    || actual.pid == bootstrap.pid
                    || actual.created_at < bootstrap.created_at
                    || (actual.logon_low, actual.logon_high)
                        != (bootstrap.logon_low, bootstrap.logon_high)
                {
                    return Err(io::Error::other("Job 候选未继承本次引导器身份"));
                }
            }
            Authorization::Complete | Authorization::Rejected => {
                return Err(io::Error::other("Job 授权已结束"));
            }
        }
        Ok(())
    }

    pub(super) fn authorize(
        previous: Authorization,
        request: &ProbeJobRequest,
        expected_image: &Path,
        worker: HANDLE,
        job: HANDLE,
    ) -> io::Result<Authorization> {
        if !accepts_stage(&previous, request.operation)
            || request.source_handle == 0
            || request.source_handle > isize::MAX as u64
        {
            return Err(io::Error::other("Job 授权阶段或原句柄无效"));
        }
        alive(worker)?;
        let worker_identity = identity(worker)?;
        let mut desired = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
        if request.operation == ProbeJobOperation::ClaimBootstrap {
            desired |= PROCESS_SET_QUOTA | PROCESS_TERMINATE;
        }
        let mut copied = HANDLE::default();
        unsafe {
            DuplicateHandle(
                worker,
                HANDLE(request.source_handle as usize as *mut _),
                GetCurrentProcess(),
                &mut copied,
                desired.0,
                false,
                DUPLICATE_HANDLE_OPTIONS(0),
            )
        }
        .map_err(io::Error::from)?;
        // 临时副本在返回 ACK 前释放，不延长新 LUID，也不向 worker 复制 Job。
        let copied = unsafe { OwnedHandle::from_raw_handle(copied.0) };
        let process = HANDLE(copied.as_raw_handle());
        alive(process)?;
        let actual = identity(process)?;
        validate_identity(
            &previous,
            request,
            expected_image,
            &worker_identity,
            &actual,
        )?;
        let next = match previous {
            Authorization::Initial => {
                let mut member = BOOL::default();
                unsafe { IsProcessInJob(process, Some(job), &mut member) }
                    .map_err(io::Error::from)?;
                if !member.as_bool() {
                    unsafe { AssignProcessToJobObject(job, process) }.map_err(io::Error::from)?;
                }
                Authorization::Bootstrap {
                    worker: worker_identity,
                    bootstrap: actual.clone(),
                }
            }
            Authorization::Bootstrap { .. } => {
                // 候选已加入自己的子 Job，此处绝不通过补分配掩盖父 Job 继承失败。
                Authorization::Complete
            }
            Authorization::Complete | Authorization::Rejected => {
                return Err(io::Error::other("Job 授权已结束"));
            }
        };
        let mut member = BOOL::default();
        unsafe { IsProcessInJob(process, Some(job), &mut member) }.map_err(io::Error::from)?;
        if !member.as_bool() || identity(process)? != actual {
            return Err(io::Error::other("候选不属于监督者原 Job 或身份已变化"));
        }
        alive(process)?;
        Ok(next)
    }
}

#[cfg(test)]
#[path = "managed_tests.rs"]
mod tests;
