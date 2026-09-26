//! 已附着控制台内的专属子进程；创建瞬间加入禁止脱离的 Job，再由调用方允许执行。

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle, FromRawHandle as _, OwnedHandle, RawHandle};
use std::path::Path;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Console::GetConsoleProcessList;
use windows::Win32::System::JobObjects::{
    CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
    ResumeThread, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

struct JobAttributes {
    storage: Vec<usize>,
}

impl JobAttributes {
    fn new(job: &HANDLE) -> io::Result<Self> {
        let mut bytes = 0;
        let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut bytes) };
        if bytes == 0 || bytes > 65536 {
            return Err(io::Error::other("专属控制台启动属性长度无效"));
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let pointer = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        unsafe { InitializeProcThreadAttributeList(Some(pointer), 1, None, &mut bytes) }
            .map_err(io::Error::from)?;
        let mut result = Self { storage };
        unsafe {
            UpdateProcThreadAttribute(
                result.pointer(),
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                Some((job as *const HANDLE).cast()),
                size_of::<HANDLE>(),
                None,
                None,
            )
        }
        .map_err(io::Error::from)?;
        Ok(result)
    }

    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.storage.as_mut_ptr().cast())
    }
}

impl Drop for JobAttributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.pointer()) };
    }
}

/// 无进程派生的能力门禁；实际 ConPTY 和原子归属仍在启动时核对。
pub fn platform_available() -> io::Result<()> {
    let job = unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::from)?;
    let job = unsafe { OwnedHandle::from_raw_handle(job.0) };
    JobAttributes::new(&HANDLE(job.as_raw_handle()))?;
    crate::managed::WindowsProcessLease::capture(std::process::id())?.validate()
}

/// 不继承 Job 句柄，创建时也不允许 breakaway；拥有者异常退出后由内核结束整棵子树。
pub struct OwnedConsoleChild {
    process: OwnedHandle,
    thread: Option<OwnedHandle>,
    job: OwnedHandle,
}

impl AsRawHandle for OwnedConsoleChild {
    fn as_raw_handle(&self) -> RawHandle {
        self.process.as_raw_handle()
    }
}

impl OwnedConsoleChild {
    /// 只在已经核对其父 shell 和真实 ConPTY 的专属入口中调用；不为 GUI 创建控制台。
    pub fn spawn_suspended(
        program: &Path,
        arguments: &[OsString],
        cwd: &Path,
        environment_overrides: &[(&OsStr, &OsStr)],
    ) -> io::Result<Self> {
        if !program.is_absolute() || !cwd.is_absolute() {
            return Err(io::Error::other("专属控制台启动需要绝对路径"));
        }
        let mut clients = [0u32; 1];
        if unsafe { GetConsoleProcessList(&mut clients) } == 0 {
            return Err(io::Error::other("专属控制台启动尚未附着真实控制台"));
        }
        let job = unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::from)?;
        let job = unsafe { OwnedHandle::from_raw_handle(job.0) };
        let raw_job = HANDLE(job.as_raw_handle());
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                raw_job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(io::Error::from)?;
        let mut attributes = JobAttributes::new(&raw_job)?;
        let program = wide(program.as_os_str())?;
        let directory = wide(cwd.as_os_str())?;
        let mut command = Vec::new();
        // 命令行只接受独立 argv；程序路径同时作为 lpApplicationName，不能变成 shell 表达式。
        quote_units(&program[..program.len() - 1], &mut command);
        for argument in arguments {
            let argument = wide(argument)?;
            command.push(b' ' as u16);
            quote_units(&argument[..argument.len() - 1], &mut command);
        }
        command.push(0);
        if command.len() > 32767 {
            return Err(io::Error::other("专属控制台命令行过长"));
        }
        let environment = environment(environment_overrides)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = attributes.pointer();
        let mut information = PROCESS_INFORMATION::default();
        unsafe {
            CreateProcessW(
                PCWSTR(program.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                Some(environment.as_ptr().cast()),
                PCWSTR(directory.as_ptr()),
                &startup.StartupInfo,
                &mut information,
            )
        }
        .map_err(io::Error::from)?;
        // CreateProcess 成功时 Job 已拥有子进程；此后任一错误都由 Job 析构终止，不能留下孤儿。
        let child = Self {
            process: unsafe { OwnedHandle::from_raw_handle(information.hProcess.0) },
            thread: Some(unsafe { OwnedHandle::from_raw_handle(information.hThread.0) }),
            job,
        };
        if !child.owns_process(&child)? {
            return Err(io::Error::other("专属控制台进程未处于原子 Job"));
        }
        Ok(child)
    }

    pub fn owns_process(&self, process: &impl AsRawHandle) -> io::Result<bool> {
        let mut member = BOOL::default();
        unsafe {
            IsProcessInJob(
                HANDLE(process.as_raw_handle()),
                Some(HANDLE(self.job.as_raw_handle())),
                &mut member,
            )
        }
        .map_err(io::Error::from)?;
        Ok(member.as_bool())
    }

    /// 调用前必须已将原始创建句柄身份和一次派发收据落盘。
    pub fn resume(&mut self) -> io::Result<()> {
        let thread = self
            .thread
            .as_ref()
            .ok_or_else(|| io::Error::other("专属控制台进程已经恢复"))?;
        let count = unsafe { ResumeThread(HANDLE(thread.as_raw_handle())) };
        if count == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        if count != 1 {
            return Err(io::Error::other("专属控制台主线程悬挂计数改变"));
        }
        self.thread.take();
        Ok(())
    }

    pub fn root_exit_code(&self) -> io::Result<Option<u32>> {
        match unsafe { WaitForSingleObject(HANDLE(self.process.as_raw_handle()), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                unsafe { GetExitCodeProcess(HANDLE(self.process.as_raw_handle()), &mut code) }
                    .map_err(io::Error::from)?;
                Ok(Some(code))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// 只终止本次创建瞬间拥有的 Job；必须确认 ActiveProcesses 为零才算清理完成。
    pub fn terminate_and_confirm(&self, timeout: Duration) -> io::Result<()> {
        unsafe { TerminateJobObject(HANDLE(self.job.as_raw_handle()), 1) }
            .map_err(io::Error::from)?;
        let deadline = Instant::now() + timeout;
        loop {
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            unsafe {
                QueryInformationJobObject(
                    Some(HANDLE(self.job.as_raw_handle())),
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    None,
                )
            }
            .map_err(io::Error::from)?;
            if accounting.ActiveProcesses == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "专属控制台 Job 退出未确认",
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut units = value.encode_wide().collect::<Vec<_>>();
    if units.contains(&0) {
        return Err(io::Error::other("专属控制台参数包含 NUL"));
    }
    units.push(0);
    Ok(units)
}

fn quote_units(argument: &[u16], output: &mut Vec<u16>) {
    output.push(b'"' as u16);
    let mut slashes = 0;
    for unit in argument {
        if *unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        output.extend(std::iter::repeat_n(
            b'\\' as u16,
            slashes * if *unit == b'"' as u16 { 2 } else { 1 },
        ));
        if *unit == b'"' as u16 {
            output.push(b'\\' as u16);
        }
        slashes = 0;
        output.push(*unit);
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
}

fn environment(overrides: &[(&OsStr, &OsStr)]) -> io::Result<Vec<u16>> {
    let mut entries = std::env::vars_os()
        .filter(|(key, _)| {
            !overrides.iter().any(|(name, _)| {
                key.to_string_lossy()
                    .eq_ignore_ascii_case(&name.to_string_lossy())
            })
        })
        .collect::<Vec<_>>();
    entries.extend(
        overrides
            .iter()
            .map(|(name, value)| (name.to_os_string(), value.to_os_string())),
    );
    entries.sort_by_key(|(key, _)| key.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (key, value) in entries {
        let key = wide(&key)?;
        let value = wide(&value)?;
        block.extend_from_slice(&key[..key.len() - 1]);
        block.push(b'=' as u16);
        block.extend(value);
    }
    block.push(0);
    Ok(block)
}
