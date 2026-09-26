use std::ffi::OsStr;
use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::os::windows::process::CommandExt as _;
use std::process::Child;

use anyhow::{Context, Result};
use warp_errors::report_error;

#[path = "windows_appcontainer.rs"]
mod appcontainer;
pub use appcontainer::AppContainerProbe;

/// 主线程仍处于 `CREATE_SUSPENDED` 状态的子进程。
///
/// 此类型不提供直接取出 [`Child`] 的通道；只有原生恢复成功后才会移交进程所有权。
#[derive(Debug)]
pub struct SuspendedChild {
    child: Option<Child>,
    primary_thread: Option<OwnedHandle>,
}

impl SuspendedChild {
    pub(crate) fn from_child(child: Child) -> io::Result<Self> {
        Self::from_child_with_child_on_error(child).map_err(|(error, mut child)| {
            terminate_and_wait(&mut child);
            error
        })
    }

    pub(crate) fn from_child_with_child_on_error(
        child: Child,
    ) -> std::result::Result<Self, (io::Error, Child)> {
        let primary_thread = find_only_thread(child.id());
        Self::from_child_with_primary_thread(child, primary_thread)
    }

    fn from_child_with_primary_thread(
        child: Child,
        primary_thread: io::Result<OwnedHandle>,
    ) -> std::result::Result<Self, (io::Error, Child)> {
        match primary_thread {
            Ok(primary_thread) => Ok(Self {
                child: Some(child),
                primary_thread: Some(primary_thread),
            }),
            Err(error) => Err((error, child)),
        }
    }

    /// 返回冻结进程的进程 ID。
    #[must_use]
    pub fn id(&self) -> u32 {
        self.child.as_ref().expect("冻结进程必须存在").id()
    }

    /// 恢复原生主线程，成功后返回普通子进程句柄。
    pub fn resume(self) -> io::Result<Child> {
        match self.resume_with_child_on_error() {
            Ok(child) => Ok(child),
            Err((error, mut child)) => {
                terminate_and_wait(&mut child);
                Err(error)
            }
        }
    }

    /// 严格 Job 中使用：恢复失败时移交子进程，由调用方终止，避免在调试事件未继续时等待。
    pub fn resume_with_child_on_error(mut self) -> std::result::Result<Child, (io::Error, Child)> {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Threading::ResumeThread;

        let primary_thread = self
            .primary_thread
            .as_ref()
            .expect("冻结进程必须保留主线程句柄");
        let previous_count = unsafe { ResumeThread(HANDLE(primary_thread.as_raw_handle())) };
        if previous_count == u32::MAX {
            return Err((
                io::Error::last_os_error(),
                self.child.take().expect("冻结进程必须存在"),
            ));
        }
        if previous_count != 1 {
            return Err((
                io::Error::other(format!(
                    "恢复冻结进程时的悬挂计数应为 1，实际为 {previous_count}"
                )),
                self.child.take().expect("冻结进程必须存在"),
            ));
        }

        self.primary_thread.take();
        Ok(self.child.take().expect("冻结进程必须存在"))
    }
}

impl std::os::windows::io::AsRawHandle for SuspendedChild {
    fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        self.child.as_ref().expect("冻结进程必须存在").as_raw_handle()
    }
}

impl Drop for SuspendedChild {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            terminate_and_wait(child);
        }
    }
}

fn terminate_and_wait(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn find_only_thread(process_id: u32) -> io::Result<OwnedHandle> {
    use windows::Win32::Foundation::ERROR_NO_MORE_FILES;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows::Win32::System::Threading::{OpenThread, THREAD_SUSPEND_RESUME};
    use windows::core::HRESULT;

    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.map_err(io::Error::from)?;
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.0) };
    let snapshot_handle = windows::Win32::Foundation::HANDLE(snapshot.as_raw_handle());
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    unsafe { Thread32First(snapshot_handle, &mut entry) }.map_err(io::Error::from)?;

    let mut thread_id = None;
    loop {
        if entry.th32OwnerProcessID == process_id {
            if thread_id.replace(entry.th32ThreadID).is_some() {
                return Err(io::Error::other(format!(
                    "冻结进程 {process_id} 存在多个线程，拒绝恢复"
                )));
            }
        }

        match unsafe { Thread32Next(snapshot_handle, &mut entry) } {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_FILES.0) => break,
            Err(error) => return Err(io::Error::from(error)),
        }
    }

    let thread_id = thread_id.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("未找到冻结进程 {process_id} 的主线程"),
        )
    })?;
    let thread =
        unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, thread_id) }.map_err(io::Error::from)?;
    Ok(unsafe { OwnedHandle::from_raw_handle(thread.0) })
}

#[derive(Debug, thiserror::Error)]
pub enum JobObjectError {
    #[error("Failed to create job: {0}")]
    CreateFailed(std::io::Error),

    #[error("Failed to assign process to job: {0}")]
    AssignFailed(std::io::Error),

    #[error("Failed to set info for job: {0}")]
    SetInfoFailed(std::io::Error),

    #[error("Failed to get info for job: {0}")]
    GetInfoFailed(std::io::Error),

    #[error(transparent)]
    Other(anyhow::Error),
}

impl From<win32job::JobError> for JobObjectError {
    fn from(error: win32job::JobError) -> Self {
        match error {
            win32job::JobError::CreateFailed(e) => JobObjectError::CreateFailed(e),
            win32job::JobError::AssignFailed(e) => JobObjectError::AssignFailed(e),
            win32job::JobError::SetInfoFailed(e) => JobObjectError::SetInfoFailed(e),
            win32job::JobError::GetInfoFailed(e) => JobObjectError::GetInfoFailed(e),
            _ => JobObjectError::Other(error.into()),
        }
    }
}

/// We use Job Objects to handle killing child processes when the program is
/// closed. This builder struct is used to configure a Job Object and associate it
/// with processes. Processes associated with a job will be killed when the handle
/// to the job is dropped at the end of the program's lifecycle.
///
/// NOTE: We've encountered issues with assigning some processes to jobs that
/// already contain other processes (i.e. `pwsh.exe`), so we only want to
/// assign a single process to a job.
///
/// For more information on Job Objects, see:
/// https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
#[derive(Debug, Default)]
pub struct JobObject {
    assign_current_process: bool,
    assign_process: Option<isize>,
    kill_children_on_close: bool,
}

impl JobObject {
    pub fn new() -> Self {
        Self::default()
    }

    /// Assigns the current process to the Job Object. This can be used to ensure
    /// that children of the current process are associated with the job.
    pub fn assign_current_process(mut self) -> Self {
        self.assign_current_process = true;
        self
    }

    /// Assigns a process to the Job Object. This process will be killed when the
    /// current process is closed.
    pub fn assign_process(mut self, process: isize) -> Self {
        self.assign_process = Some(process);
        self
    }

    /// Configures the Job Object so children of the assigned processes are
    /// automatically associated with the job, thus killing them along with
    /// their parents on close.
    pub fn kill_children_on_close(mut self) -> Self {
        self.kill_children_on_close = true;
        self
    }

    fn create_internal(self) -> Result<(), win32job::JobError> {
        let job = win32job::Job::create()?;

        let mut info = job.query_extended_limit_info()?;
        // Mark the job as "kill on job close", so all processes associated with
        // the job are killed when the handle to the job is closed.
        info.limit_kill_on_job_close();
        info.limit_breakaway_ok();
        if !self.kill_children_on_close {
            info.limit_silent_breakaway_ok();
        }
        job.set_extended_limit_info(&info)?;

        if self.assign_current_process {
            job.assign_current_process()?;
        }
        if let Some(process) = self.assign_process {
            job.assign_process(process)?;
        }

        Box::leak(Box::new(job));
        Ok(())
    }

    /// Creates a new Job Object and assigns any specified processes to it. The
    /// handle to the job is leaked to ensure that the job lives for the lifetime
    /// of the program.
    pub fn create(self) -> Result<(), JobObjectError> {
        self.create_internal().map_err(Into::into)
    }
}

pub fn init() {
    if let Err(e) = JobObject::new()
        .kill_children_on_close()
        .assign_current_process()
        .create()
        .context("Failed to create job object for the program")
    {
        report_error!(e);
    }
}

pub trait CommandExt {
    /// Append literal text to the command line without any quoting or escaping.
    ///
    /// This is useful for passing arguments to `cmd.exe /c`, which doesn't follow
    /// `CommandLineToArgvW` escaping rules.
    fn raw_arg<S: AsRef<OsStr>>(&mut self, text_to_append_as_is: S) -> &mut Self;
}

use async_process::windows::CommandExt as _;

impl CommandExt for crate::blocking::Command {
    fn raw_arg<S: AsRef<OsStr>>(&mut self, text_to_append_as_is: S) -> &mut Self {
        self.inner.raw_arg(text_to_append_as_is);
        self
    }
}

impl CommandExt for crate::r#async::Command {
    fn raw_arg<S: AsRef<OsStr>>(&mut self, text_to_append_as_is: S) -> &mut Self {
        self.inner.raw_arg(text_to_append_as_is);
        self
    }
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
