//! Windows 原生 PE 更新程序的 replacement lease 候选实现。
//!
//! 本模块只接受来源层已经冻结身份的单个原生 PE。程序句柄不共享写入或删除，
//! 从卷根到程序父目录的每一层都不共享删除；因此在 `CreateProcessW` 成功创建 image
//! 以前保持对象身份。目录必须申请真实读取访问参与共享删除检查；固定官方更新器
//! 由来源层绑定固定安装布局；未知 helper 或 DLL 在放行调试事件前拒绝。

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read as _, Seek as _};
use std::os::windows::ffi::OsStringExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

use command::blocking::Command;
use windows::Win32::Foundation::{
    DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, EXCEPTION_BREAKPOINT, GENERIC_READ, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, GetFinalPathNameByHandleW,
    VOLUME_NAME_DOS,
};
use windows::Win32::System::Diagnostics::Debug::{
    CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, ContinueDebugEvent, DEBUG_EVENT,
    EXCEPTION_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT, EXIT_THREAD_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT,
    OUTPUT_DEBUG_STRING_EVENT, RIP_EVENT, UNLOAD_DLL_DEBUG_EVENT, WaitForDebugEvent,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Threading::TerminateProcess;

use super::{AtomicDirectoryIdentity, ExpectedFileId, ExpectedFileIdentity, sha256_file};

const MAX_NATIVE_EXECUTABLE_BYTES: u64 = 1024 * 1024 * 1024;
const DOS_HEADER_PE_OFFSET: u64 = 0x3c;
const MAX_PE_HEADER_OFFSET: u64 = 16 * 1024 * 1024;
const PE_SIGNATURE_AND_FILE_HEADER_BYTES: u64 = 24;
const SECTION_HEADER_BYTES: u64 = 40;
const MAX_PE_SECTIONS: u16 = 96;
const MAX_DATA_DIRECTORIES: u32 = 16;
const IMPORT_DIRECTORY_INDEX: u32 = 1;
const BOUND_IMPORT_DIRECTORY_INDEX: u32 = 11;
const DELAY_IMPORT_DIRECTORY_INDEX: u32 = 13;
const IMPORT_DESCRIPTOR_BYTES: u64 = 20;
const DELAY_IMPORT_DESCRIPTOR_BYTES: u64 = 32;
const MAX_IMPORT_DIRECTORY_BYTES: u64 = 1024 * 1024;
const MAX_IMPORT_NAME_BYTES: u64 = 260;
const DEBUG_INITIAL_TIMEOUT: Duration = Duration::from_secs(20);
const DEBUG_SESSION_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const DEBUG_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_WINDOWS_PATH_U16: usize = 32_768;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LeasedIdentity {
    id: ExpectedFileId,
    size: u64,
    attributes: u32,
}

#[derive(Debug)]
struct AncestorLease {
    file: File,
    identity: LeasedIdentity,
}

/// 持有程序和所有祖先的拒绝替换句柄；Drop 是唯一释放路径。
#[derive(Debug)]
pub(super) struct WindowsReplacementLease {
    execution_path: PathBuf,
    program: File,
    program_identity: LeasedIdentity,
    expected_sha256: String,
    ancestors: Vec<AncestorLease>,
    system_directory: AncestorLease,
    powershell: Option<SystemHelperLease>,
}

/// 固定系统 helper 的文件和祖先租约；不接受同目录其他程序或 DLL。
#[derive(Debug)]
struct SystemHelperLease {
    program: File,
    identity: LeasedIdentity,
    sha256: String,
    ancestors: Vec<AncestorLease>,
}

impl SystemHelperLease {
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            program: self.program.try_clone()?,
            identity: self.identity,
            sha256: self.sha256.clone(),
            ancestors: self
                .ancestors
                .iter()
                .map(|ancestor| {
                    Ok(AncestorLease {
                        file: ancestor.file.try_clone()?,
                        identity: ancestor.identity,
                    })
                })
                .collect::<io::Result<Vec<_>>>()?,
        })
    }

    fn verify_image(&self, file: &File) -> io::Result<()> {
        for ancestor in &self.ancestors {
            if inspect_handle(&ancestor.file)? != ancestor.identity {
                return Err(error(
                    "managed_process.atomic_windows_helper_ancestor_changed",
                ));
            }
        }
        if inspect_handle(&self.program)? != self.identity
            || inspect_handle(file)? != self.identity
            || sha256_file(&mut file.try_clone()?)? != self.sha256
        {
            return Err(error("managed_process.atomic_windows_helper_image_changed"));
        }
        Ok(())
    }
}

/// 从卷根到 cwd 本身持有参与共享删除检查的目录读取租约。
#[derive(Debug)]
pub(super) struct WindowsDirectoryLease {
    execution_path: PathBuf,
    identity: ExpectedFileId,
    ancestors: Vec<AncestorLease>,
}

/// 持续消费 Windows loader 调试事件，所有映像都必须在首次执行初始化代码前通过句柄校验。
#[derive(Debug)]
pub(super) struct WindowsImageDebugSession {
    root_process_id: u32,
    expected_program_id: ExpectedFileId,
    expected_program_size: u64,
    expected_program_sha256: String,
    system_directory: AncestorLease,
    powershell: Option<SystemHelperLease>,
    processes: HashMap<u32, OwnedHandle>,
    initial_breakpoints: HashSet<u32>,
}

impl WindowsDirectoryLease {
    pub(super) fn execution_path(&self) -> &Path {
        &self.execution_path
    }

    pub(super) fn verify_for_spawn(&self) -> io::Result<()> {
        for ancestor in &self.ancestors {
            let current = inspect_handle(&ancestor.file)?;
            if current != ancestor.identity || !is_plain_kind(current.attributes, true) {
                return Err(error("managed_process.atomic_windows_cwd_identity_changed"));
            }
        }
        if self
            .ancestors
            .last()
            .is_none_or(|ancestor| ancestor.identity.id != self.identity)
        {
            return Err(error("managed_process.atomic_windows_cwd_identity_changed"));
        }
        let current_path_identity = open_ancestor(&self.execution_path)
            .and_then(|file| inspect_handle(&file))
            .map_err(|_| error("managed_process.atomic_windows_cwd_identity_changed"))?;
        if current_path_identity.id != self.identity
            || !is_plain_kind(current_path_identity.attributes, true)
        {
            return Err(error("managed_process.atomic_windows_cwd_identity_changed"));
        }
        Ok(())
    }
}

impl WindowsReplacementLease {
    pub(super) fn execution_path(&self) -> &Path {
        &self.execution_path
    }

    /// 再次从同一租约句柄复核身份、大小、摘要和 PE 结构。
    pub(super) fn verify_for_spawn(&mut self) -> io::Result<()> {
        for ancestor in &self.ancestors {
            let current = inspect_handle(&ancestor.file)?;
            if current != ancestor.identity || !is_plain_kind(current.attributes, true) {
                return Err(error(
                    "managed_process.atomic_windows_ancestor_identity_changed",
                ));
            }
        }
        let before = inspect_handle(&self.program)?;
        if before != self.program_identity || !is_plain_kind(before.attributes, false) {
            return Err(error(
                "managed_process.atomic_windows_program_identity_changed",
            ));
        }
        let digest = sha256_file(&mut self.program)?;
        require_pe(&mut self.program, before.size)?;
        let after = inspect_handle(&self.program)?;
        if digest != self.expected_sha256 || after != before {
            return Err(error(
                "managed_process.atomic_windows_program_content_changed",
            ));
        }
        Ok(())
    }

    /// 校验未来 suspended spawn 的命令身份，但不使用普通 `spawn` 启动。
    ///
    /// 当前 Command API 不返回主线程句柄，无法在 image 创建后先释放租约再可靠恢复。
    /// 调用方只能把通过校验的命令交给后续专用 suspended API；普通 spawn 必须禁用。
    pub(super) fn verify_command_for_suspended_spawn(
        &mut self,
        command: &Command,
    ) -> io::Result<()> {
        if Path::new(command.get_program()) != self.execution_path {
            return Err(error(
                "managed_process.atomic_windows_command_program_mismatch",
            ));
        }
        self.verify_for_spawn()
    }

    /// 在主线程已恢复后接管首个调试事件，并用事件携带的 image handle 复核根映像。
    ///
    /// 首事件在用户态执行前送达；校验完成前保留程序与祖先租约，也不继续该事件。
    pub(super) fn begin_image_debug_session(
        self,
        root_process_id: u32,
    ) -> io::Result<WindowsImageDebugSession> {
        let mut session = WindowsImageDebugSession {
            root_process_id,
            expected_program_id: self.program_identity.id,
            expected_program_size: self.program_identity.size,
            expected_program_sha256: self.expected_sha256.clone(),
            system_directory: AncestorLease {
                file: self.system_directory.file.try_clone()?,
                identity: self.system_directory.identity,
            },
            powershell: self
                .powershell
                .as_ref()
                .map(SystemHelperLease::try_clone)
                .transpose()?,
            processes: HashMap::new(),
            initial_breakpoints: HashSet::new(),
        };
        let event = wait_for_debug_event_until(
            Instant::now() + DEBUG_INITIAL_TIMEOUT,
            "managed_process.atomic_windows_initial_debug_event_timed_out",
        )?;
        if event.dwDebugEventCode != CREATE_PROCESS_DEBUG_EVENT
            || event.dwProcessId != root_process_id
        {
            // 未识别首事件时不放行任何用户态执行；外层严格 Job 清理整棵树。
            return Err(error(
                "managed_process.atomic_windows_initial_debug_event_invalid",
            ));
        }
        if let Err(failure) = session.handle_create_process(&event, true) {
            session.reject_event_and_drain(&event);
            return Err(failure);
        }
        drop(self);
        continue_debug_event(&event, DBG_CONTINUE)?;
        Ok(session)
    }
}

impl WindowsImageDebugSession {
    /// loader 运行期间持续验证每个根进程、子进程与 DLL 的实际映像句柄。
    ///
    /// Windows 的初始调试断点发生在静态 DLL 已映射、但任何 DLL 初始化例程执行前；
    /// 后续 `LOAD_DLL_DEBUG_EVENT` 同样在新映像初始化前暂停。拒绝事件时先终止全部
    /// debuggee，再继续并排空事件，未验证映像没有执行窗口。
    pub(super) fn wait_for_exit(&mut self, child: &mut Child) -> io::Result<ExitStatus> {
        let mut root_exit_observed = false;
        let deadline = Instant::now() + DEBUG_SESSION_TIMEOUT;
        loop {
            let event = wait_for_debug_event_until(
                deadline,
                "managed_process.atomic_windows_debug_session_timed_out",
            )?;
            let continue_status = match self.validate_event(&event) {
                Ok(status) => status,
                Err(failure) => {
                    self.reject_event_and_drain(&event);
                    return Err(failure);
                }
            };
            if event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT
                && event.dwProcessId == self.root_process_id
            {
                root_exit_observed = true;
            }
            continue_debug_event(&event, continue_status)?;
            if root_exit_observed && self.processes.is_empty() {
                return child.wait();
            }
        }
    }

    fn validate_event(
        &mut self,
        event: &DEBUG_EVENT,
    ) -> io::Result<windows::Win32::Foundation::NTSTATUS> {
        match event.dwDebugEventCode {
            CREATE_PROCESS_DEBUG_EVENT => {
                self.handle_create_process(event, event.dwProcessId == self.root_process_id)?;
                Ok(DBG_CONTINUE)
            }
            CREATE_THREAD_DEBUG_EVENT => {
                let information = unsafe { event.u.CreateThread };
                close_owned_handle(information.hThread)?;
                Ok(DBG_CONTINUE)
            }
            EXCEPTION_DEBUG_EVENT => {
                let information = unsafe { event.u.Exception };
                if information.ExceptionRecord.ExceptionCode == EXCEPTION_BREAKPOINT
                    && self.initial_breakpoints.insert(event.dwProcessId)
                {
                    Ok(DBG_CONTINUE)
                } else {
                    Ok(DBG_EXCEPTION_NOT_HANDLED)
                }
            }
            EXIT_THREAD_DEBUG_EVENT | OUTPUT_DEBUG_STRING_EVENT | UNLOAD_DLL_DEBUG_EVENT => {
                Ok(DBG_CONTINUE)
            }
            EXIT_PROCESS_DEBUG_EVENT => {
                self.processes.remove(&event.dwProcessId);
                self.initial_breakpoints.remove(&event.dwProcessId);
                Ok(DBG_CONTINUE)
            }
            LOAD_DLL_DEBUG_EVENT => {
                let information = unsafe { event.u.LoadDll };
                let file = file_from_debug_handle(information.hFile)?;
                self.verify_system_image(&file)?;
                Ok(DBG_CONTINUE)
            }
            RIP_EVENT => Err(error("managed_process.atomic_windows_loader_rip_event")),
            code => Err(io::Error::other(format!(
                "managed_process.atomic_windows_unknown_debug_event:{code:?}"
            ))),
        }
    }

    fn handle_create_process(&mut self, event: &DEBUG_EVENT, root: bool) -> io::Result<()> {
        let information = unsafe { event.u.CreateProcessInfo };
        let process = owned_handle(information.hProcess)?;
        if self.processes.insert(event.dwProcessId, process).is_some() {
            return Err(error(
                "managed_process.atomic_windows_duplicate_process_event",
            ));
        }
        let file = file_from_debug_handle(information.hFile)?;
        close_owned_handle(information.hThread)?;
        if root {
            self.verify_root_image(&file)
        } else if let Some(helper) = &self.powershell
            && inspect_handle(&file)?.id == helper.identity.id
        {
            helper.verify_image(&file)
        } else {
            self.verify_system_image(&file)
        }
    }

    fn verify_root_image(&self, file: &File) -> io::Result<()> {
        let identity = inspect_handle(file)?;
        if identity.id != self.expected_program_id
            || identity.size != self.expected_program_size
            || !is_plain_kind(identity.attributes, false)
        {
            return Err(error(
                "managed_process.atomic_windows_created_image_identity_changed",
            ));
        }
        let mut duplicate = file.try_clone()?;
        if sha256_file(&mut duplicate)? != self.expected_program_sha256
            || inspect_handle(file)? != identity
        {
            return Err(error(
                "managed_process.atomic_windows_created_image_content_changed",
            ));
        }
        Ok(())
    }

    fn verify_system_image(&self, file: &File) -> io::Result<()> {
        let system_current = inspect_handle(&self.system_directory.file)?;
        if system_current != self.system_directory.identity
            || !is_plain_kind(system_current.attributes, true)
        {
            return Err(error(
                "managed_process.atomic_windows_system_directory_changed",
            ));
        }
        let image_identity = inspect_handle(file)?;
        if image_identity.size == 0 || !is_plain_kind(image_identity.attributes, false) {
            return Err(error(
                "managed_process.atomic_windows_loaded_image_not_plain",
            ));
        }
        let final_path = final_path_from_handle(file)?;
        let parent = final_path
            .parent()
            .ok_or_else(|| error("managed_process.atomic_windows_loaded_image_parent_missing"))?;
        let parent_file = open_ancestor(parent)?;
        let parent_identity = inspect_handle(&parent_file)?;
        if parent_identity.id != self.system_directory.identity.id
            || !is_plain_kind(parent_identity.attributes, true)
        {
            #[cfg(test)]
            tests::record_rejected_image(&final_path, &self.system_directory.file);
            return Err(error(
                "managed_process.atomic_windows_loaded_image_outside_system_directory",
            ));
        }
        Ok(())
    }

    fn reject_event_and_drain(&mut self, event: &DEBUG_EVENT) {
        // 当前事件所属进程必须已有可终止句柄；否则不能继续未验证的事件。
        if !self.processes.contains_key(&event.dwProcessId) {
            return;
        }
        for process in self.processes.values() {
            let handle = HANDLE(process.as_raw_handle());
            if unsafe { TerminateProcess(handle, 1) }.is_err() {
                // 无法确认终止时不继续未验证事件，交由外层严格 Job 清理。
                return;
            }
        }
        let _ = continue_debug_event(event, DBG_CONTINUE);
        let deadline = Instant::now() + DEBUG_DRAIN_TIMEOUT;
        while !self.processes.is_empty() {
            let Ok(next) = wait_for_debug_event_until(
                deadline,
                "managed_process.atomic_windows_debug_drain_timed_out",
            ) else {
                break;
            };
            if next.dwDebugEventCode == CREATE_PROCESS_DEBUG_EVENT {
                let information = unsafe { next.u.CreateProcessInfo };
                let Ok(process) = owned_handle(information.hProcess) else {
                    return;
                };
                let handle = HANDLE(process.as_raw_handle());
                if unsafe { TerminateProcess(handle, 1) }.is_err() {
                    return;
                }
                self.processes.insert(next.dwProcessId, process);
                let _ = close_owned_handle(information.hThread);
                if !information.hFile.is_invalid() {
                    let _ = file_from_debug_handle(information.hFile);
                }
            } else if next.dwDebugEventCode == CREATE_THREAD_DEBUG_EVENT {
                let information = unsafe { next.u.CreateThread };
                let _ = close_owned_handle(information.hThread);
            } else if next.dwDebugEventCode == LOAD_DLL_DEBUG_EVENT {
                let information = unsafe { next.u.LoadDll };
                if !information.hFile.is_invalid() {
                    let _ = file_from_debug_handle(information.hFile);
                }
            } else if next.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT {
                self.processes.remove(&next.dwProcessId);
            }
            let _ = continue_debug_event(&next, DBG_CONTINUE);
        }
    }
}

/// 从冻结身份创建租约。祖先按卷根到直接父目录的顺序锁定，逐层拒绝 reparse。
pub(super) fn prepare(expected: &ExpectedFileIdentity) -> io::Result<WindowsReplacementLease> {
    validate_expected(expected)?;
    // 必须在派生前完成所有可能失败的系统目录准备。进程成为 debuggee 后若在首事件前
    // 返回，SuspendedChild 的同步回收会等待尚未继续的调试事件，形成不可恢复的死锁。
    let system_directory = prepare_system_directory()?;
    let execution_path = expected.canonical_path.clone();
    let parent = execution_path
        .parent()
        .ok_or_else(|| error("managed_process.atomic_windows_program_parent_missing"))?;
    let mut ancestor_paths: Vec<_> = parent.ancestors().map(Path::to_owned).collect();
    ancestor_paths.reverse();
    if ancestor_paths.is_empty() {
        return Err(error(
            "managed_process.atomic_windows_program_parent_missing",
        ));
    }

    // 锁定所有祖先比只识别可写 ACL 更保守；无法打开任一层时保持 fail closed。
    let mut ancestors = Vec::with_capacity(ancestor_paths.len());
    for path in ancestor_paths {
        let file = open_ancestor(&path)?;
        let identity = inspect_handle(&file)?;
        if !is_plain_kind(identity.attributes, true) {
            return Err(error("managed_process.atomic_windows_ancestor_not_plain"));
        }
        ancestors.push(AncestorLease { file, identity });
    }

    let mut program = open_program(&execution_path)?;
    let program_identity = inspect_handle(&program)?;
    if !is_plain_kind(program_identity.attributes, false)
        || program_identity.size != expected.size
        || Some(program_identity.id) != expected.file_id
    {
        return Err(error(
            "managed_process.atomic_windows_program_identity_changed",
        ));
    }
    let digest = sha256_file(&mut program)?;
    require_pe(&mut program, program_identity.size)?;
    if digest != expected.sha256 || inspect_handle(&program)? != program_identity {
        return Err(error(
            "managed_process.atomic_windows_program_content_changed",
        ));
    }

    let powershell = prepare_powershell(&system_directory)?;
    Ok(WindowsReplacementLease {
        execution_path,
        program,
        program_identity,
        expected_sha256: expected.sha256.clone(),
        ancestors,
        system_directory,
        powershell,
    })
}

pub(super) fn capture_directory_id(path: &Path) -> io::Result<ExpectedFileId> {
    let directory = open_ancestor(path)?;
    let identity = inspect_handle(&directory)?;
    if !is_plain_kind(identity.attributes, true) {
        return Err(error("managed_process.atomic_windows_cwd_not_plain"));
    }
    Ok(identity.id)
}

pub(super) fn prepare_directory(
    expected: &AtomicDirectoryIdentity,
) -> io::Result<WindowsDirectoryLease> {
    if !expected.path.is_absolute() || !expected.canonical_path.is_absolute() {
        return Err(error("managed_process.atomic_windows_cwd_identity_invalid"));
    }
    let mut ancestor_paths: Vec<_> = expected
        .canonical_path
        .ancestors()
        .map(Path::to_owned)
        .collect();
    ancestor_paths.reverse();
    let mut ancestors = Vec::with_capacity(ancestor_paths.len());
    for path in ancestor_paths {
        let file = open_ancestor(&path)?;
        let identity = inspect_handle(&file)?;
        if !is_plain_kind(identity.attributes, true) {
            return Err(error("managed_process.atomic_windows_cwd_not_plain"));
        }
        ancestors.push(AncestorLease { file, identity });
    }
    let lease = WindowsDirectoryLease {
        execution_path: expected.canonical_path.clone(),
        identity: expected.file_id,
        ancestors,
    };
    lease.verify_for_spawn()?;
    Ok(lease)
}

fn validate_expected(expected: &ExpectedFileIdentity) -> io::Result<()> {
    if !expected.path.is_absolute()
        || !expected.canonical_path.is_absolute()
        || expected.file_id.is_none()
        || expected.size == 0
        || expected.size > MAX_NATIVE_EXECUTABLE_BYTES
        || expected.sha256.len() != 64
        || !expected.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(error(
            "managed_process.atomic_windows_source_identity_invalid",
        ));
    }
    Ok(())
}

fn open_program(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .access_mode(GENERIC_READ.0)
        // 故意不共享写入和删除；后续替换、截断、rename 和 delete 均必须失败。
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    options.open(path)
}

fn open_ancestor(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .access_mode(GENERIC_READ.0 | FILE_READ_ATTRIBUTES.0)
        // 单独 READ_ATTRIBUTES 不参与共享删除检查；真实目录读取访问才能阻止叶目录替换。
        // 允许目录内容的普通读写，但不共享 DELETE。
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
        .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0);
    options.open(path)
}

fn inspect_handle(file: &File) -> io::Result<LeasedIdentity> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
        .map_err(io::Error::other)?;
    Ok(LeasedIdentity {
        id: ExpectedFileId {
            volume: u64::from(information.dwVolumeSerialNumber),
            index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        },
        size: (u64::from(information.nFileSizeHigh) << 32) | u64::from(information.nFileSizeLow),
        attributes: information.dwFileAttributes,
    })
}

fn prepare_powershell(system_directory: &AncestorLease) -> io::Result<Option<SystemHelperLease>> {
    let system_path = final_path_from_handle(&system_directory.file)?;
    let mut ancestors = Vec::new();
    let mut path = system_path;
    for name in ["WindowsPowerShell", "v1.0"] {
        path.push(name);
        let file = match open_ancestor(&path) {
            Ok(file) => file,
            Err(failure) if failure.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(failure) => return Err(failure),
        };
        let identity = inspect_handle(&file)?;
        if !is_plain_kind(identity.attributes, true) {
            return Err(error(
                "managed_process.atomic_windows_helper_parent_not_plain",
            ));
        }
        ancestors.push(AncestorLease { file, identity });
    }
    path.push("powershell.exe");
    let mut program = match open_program(&path) {
        Ok(file) => file,
        Err(failure) if failure.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(failure) => return Err(failure),
    };
    let identity = inspect_handle(&program)?;
    if !is_plain_kind(identity.attributes, false) || identity.size == 0 {
        return Err(error("managed_process.atomic_windows_helper_not_plain"));
    }
    require_pe(&mut program, identity.size)?;
    let sha256 = sha256_file(&mut program)?;
    if inspect_handle(&program)? != identity {
        return Err(error("managed_process.atomic_windows_helper_image_changed"));
    }
    Ok(Some(SystemHelperLease {
        program,
        identity,
        sha256,
        ancestors,
    }))
}

fn prepare_system_directory() -> io::Result<AncestorLease> {
    let mut buffer = vec![0_u16; MAX_WINDOWS_PATH_U16];
    let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(error(
            "managed_process.atomic_windows_system_directory_unavailable",
        ));
    }
    let path = PathBuf::from(OsString::from_wide(&buffer[..length]));
    let canonical = std::fs::canonicalize(path)?;
    let file = open_ancestor(&canonical)?;
    let identity = inspect_handle(&file)?;
    if !is_plain_kind(identity.attributes, true) {
        return Err(error(
            "managed_process.atomic_windows_system_directory_not_plain",
        ));
    }
    Ok(AncestorLease { file, identity })
}

fn final_path_from_handle(file: &File) -> io::Result<PathBuf> {
    let mut buffer = vec![0_u16; MAX_WINDOWS_PATH_U16];
    let length = unsafe {
        GetFinalPathNameByHandleW(HANDLE(file.as_raw_handle()), &mut buffer, VOLUME_NAME_DOS)
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(error(
            "managed_process.atomic_windows_loaded_image_path_unavailable",
        ));
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..length])))
}

fn file_from_debug_handle(handle: HANDLE) -> io::Result<File> {
    if handle.is_invalid() {
        return Err(error(
            "managed_process.atomic_windows_debug_image_handle_missing",
        ));
    }
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

fn owned_handle(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_invalid() {
        return Err(error(
            "managed_process.atomic_windows_debug_process_handle_missing",
        ));
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}

fn close_owned_handle(handle: HANDLE) -> io::Result<()> {
    drop(owned_handle(handle)?);
    Ok(())
}

fn wait_for_debug_event(timeout_ms: u32) -> io::Result<DEBUG_EVENT> {
    let mut event = DEBUG_EVENT::default();
    unsafe { WaitForDebugEvent(&mut event, timeout_ms) }.map_err(io::Error::other)?;
    Ok(event)
}

fn wait_for_debug_event_until(
    deadline: Instant,
    timeout_message: &'static str,
) -> io::Result<DEBUG_EVENT> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
    }
    let timeout_ms = u32::try_from(remaining.as_millis())
        .unwrap_or(u32::MAX)
        .max(1);
    wait_for_debug_event(timeout_ms).map_err(|failure| {
        if Instant::now() >= deadline {
            io::Error::new(io::ErrorKind::TimedOut, timeout_message)
        } else {
            failure
        }
    })
}

fn continue_debug_event(
    event: &DEBUG_EVENT,
    status: windows::Win32::Foundation::NTSTATUS,
) -> io::Result<()> {
    unsafe { ContinueDebugEvent(event.dwProcessId, event.dwThreadId, status) }
        .map_err(io::Error::other)
}

fn is_plain_kind(attributes: u32, directory: bool) -> bool {
    let is_directory = attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    let is_reparse = attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
    !is_reparse && is_directory == directory
}

#[derive(Clone, Copy, Debug)]
struct PeSection {
    virtual_address: u64,
    virtual_span: u64,
    raw_offset: u64,
    raw_size: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PeDataDirectory {
    rva: u64,
    size: u64,
}

/// 接受结构完整、导入名称有界的本机 PE；实际解析到的每个映像仍由调试事件句柄绑定。
fn require_pe(file: &mut File, size: u64) -> io::Result<()> {
    if size < DOS_HEADER_PE_OFFSET + 4 {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }
    let dos = read_at::<64>(file, 0, size)?;
    if &dos[..2] != b"MZ" {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }
    let pe_offset = u64::from(u32::from_le_bytes(
        dos[DOS_HEADER_PE_OFFSET as usize..DOS_HEADER_PE_OFFSET as usize + 4]
            .try_into()
            .expect("固定 DOS 头范围有效"),
    ));
    if pe_offset < 64
        || pe_offset > MAX_PE_HEADER_OFFSET
        || checked_end(pe_offset, PE_SIGNATURE_AND_FILE_HEADER_BYTES, size).is_err()
    {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }
    let header = read_at::<24>(file, pe_offset, size)?;
    let machine = u16::from_le_bytes([header[4], header[5]]);
    let section_count = u16::from_le_bytes([header[6], header[7]]);
    let optional_header_size = u16::from_le_bytes([header[20], header[21]]);
    let characteristics = u16::from_le_bytes([header[22], header[23]]);
    if &header[..4] != b"PE\0\0"
        || section_count == 0
        || section_count > MAX_PE_SECTIONS
        || characteristics & 0x0002 == 0
        || characteristics & 0x2000 != 0
    {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }

    let optional_offset = checked_add(pe_offset, PE_SIGNATURE_AND_FILE_HEADER_BYTES)?;
    checked_end(optional_offset, u64::from(optional_header_size), size)?;
    let optional_magic = u16::from_le_bytes(read_at::<2>(file, optional_offset, size)?);
    let (minimum_optional_size, directory_count_offset, directory_offset) = match optional_magic {
        0x010b if machine == 0x014c => (96_u64, 92_u64, 96_u64),
        0x020b if matches!(machine, 0x8664 | 0xaa64) => (112_u64, 108_u64, 112_u64),
        0x010b | 0x020b => {
            return Err(error("managed_process.atomic_windows_program_not_pe"));
        }
        _ => return Err(error("managed_process.atomic_windows_program_not_pe")),
    };
    if u64::from(optional_header_size) < minimum_optional_size {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }

    let size_of_headers = u64::from(read_u32(file, checked_add(optional_offset, 60)?, size)?);
    let directory_count = read_u32(
        file,
        checked_add(optional_offset, directory_count_offset)?,
        size,
    )?;
    if directory_count > MAX_DATA_DIRECTORIES {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }
    let directories_bytes = u64::from(directory_count)
        .checked_mul(8)
        .ok_or_else(|| error("managed_process.atomic_windows_program_not_pe"))?;
    if directory_offset
        .checked_add(directories_bytes)
        .is_none_or(|end| end > u64::from(optional_header_size))
    {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }

    let section_table_offset = checked_add(optional_offset, u64::from(optional_header_size))?;
    let section_table_size = u64::from(section_count)
        .checked_mul(SECTION_HEADER_BYTES)
        .ok_or_else(|| error("managed_process.atomic_windows_program_not_pe"))?;
    let section_table_end = checked_end(section_table_offset, section_table_size, size)?;
    if size_of_headers < section_table_end || size_of_headers > size {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }

    let mut sections: Vec<PeSection> = Vec::with_capacity(usize::from(section_count));
    for index in 0..section_count {
        let offset = checked_add(
            section_table_offset,
            u64::from(index) * SECTION_HEADER_BYTES,
        )?;
        let section = read_at::<40>(file, offset, size)?;
        let virtual_size = u64::from(u32::from_le_bytes(section[8..12].try_into().unwrap()));
        let virtual_address = u64::from(u32::from_le_bytes(section[12..16].try_into().unwrap()));
        let raw_size = u64::from(u32::from_le_bytes(section[16..20].try_into().unwrap()));
        let raw_offset = u64::from(u32::from_le_bytes(section[20..24].try_into().unwrap()));
        let virtual_span = virtual_size.max(raw_size);
        if virtual_address < size_of_headers
            || virtual_span == 0
            || virtual_address.checked_add(virtual_span).is_none()
            || (raw_size != 0
                && (raw_offset < size_of_headers
                    || raw_offset
                        .checked_add(raw_size)
                        .is_none_or(|end| end > size)))
        {
            return Err(error("managed_process.atomic_windows_program_not_pe"));
        }
        let candidate = PeSection {
            virtual_address,
            virtual_span,
            raw_offset,
            raw_size,
        };
        if sections.iter().any(|existing| {
            ranges_overlap(
                existing.virtual_address,
                existing.virtual_span,
                candidate.virtual_address,
                candidate.virtual_span,
            ) || (existing.raw_size != 0
                && candidate.raw_size != 0
                && ranges_overlap(
                    existing.raw_offset,
                    existing.raw_size,
                    candidate.raw_offset,
                    candidate.raw_size,
                ))
        }) {
            return Err(error("managed_process.atomic_windows_program_not_pe"));
        }
        sections.push(candidate);
    }

    let import = read_data_directory(
        file,
        size,
        optional_offset,
        directory_offset,
        directory_count,
        IMPORT_DIRECTORY_INDEX,
    )?;
    let delay_import = read_data_directory(
        file,
        size,
        optional_offset,
        directory_offset,
        directory_count,
        DELAY_IMPORT_DIRECTORY_INDEX,
    )?;
    let bound_import = read_data_directory(
        file,
        size,
        optional_offset,
        directory_offset,
        directory_count,
        BOUND_IMPORT_DIRECTORY_INDEX,
    )?;
    if bound_import != PeDataDirectory::default() {
        return Err(error(
            "managed_process.atomic_windows_bound_import_unsupported",
        ));
    }
    require_bounded_descriptor_table(
        file,
        size,
        size_of_headers,
        &sections,
        import,
        IMPORT_DESCRIPTOR_BYTES,
        false,
    )?;
    require_bounded_descriptor_table(
        file,
        size,
        size_of_headers,
        &sections,
        delay_import,
        DELAY_IMPORT_DESCRIPTOR_BYTES,
        true,
    )?;
    file.rewind()?;
    Ok(())
}

fn read_data_directory(
    file: &mut File,
    file_size: u64,
    optional_offset: u64,
    directory_offset: u64,
    directory_count: u32,
    index: u32,
) -> io::Result<PeDataDirectory> {
    if index >= directory_count {
        return Ok(PeDataDirectory::default());
    }
    let entry_offset = checked_add(
        checked_add(optional_offset, directory_offset)?,
        u64::from(index) * 8,
    )?;
    let entry = read_at::<8>(file, entry_offset, file_size)?;
    Ok(PeDataDirectory {
        rva: u64::from(u32::from_le_bytes(entry[..4].try_into().unwrap())),
        size: u64::from(u32::from_le_bytes(entry[4..].try_into().unwrap())),
    })
}

fn require_bounded_descriptor_table(
    file: &mut File,
    file_size: u64,
    size_of_headers: u64,
    sections: &[PeSection],
    directory: PeDataDirectory,
    descriptor_size: u64,
    delay: bool,
) -> io::Result<()> {
    if directory == PeDataDirectory::default() {
        return Ok(());
    }
    if directory.rva == 0
        || directory.size < descriptor_size
        || directory.size > MAX_IMPORT_DIRECTORY_BYTES
    {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }

    if directory.size % descriptor_size != 0 {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }
    let descriptor_offset = rva_to_file_offset(
        directory.rva,
        directory.size,
        file_size,
        size_of_headers,
        sections,
    )?;
    let descriptor_count = directory.size / descriptor_size;
    for index in 0..descriptor_count {
        let offset = checked_add(descriptor_offset, index * descriptor_size)?;
        let mut descriptor = [0_u8; DELAY_IMPORT_DESCRIPTOR_BYTES as usize];
        file.seek(io::SeekFrom::Start(offset))?;
        file.read_exact(&mut descriptor[..descriptor_size as usize])?;
        let bytes = &descriptor[..descriptor_size as usize];
        if bytes.iter().all(|byte| *byte == 0) {
            return Ok(());
        }
        if delay && u32::from_le_bytes(descriptor[..4].try_into().unwrap()) != 1 {
            return Err(error(
                "managed_process.atomic_windows_delay_import_not_rva_based",
            ));
        }
        let name_offset = if delay { 4 } else { 12 };
        let name_rva = u64::from(u32::from_le_bytes(
            descriptor[name_offset..name_offset + 4].try_into().unwrap(),
        ));
        require_import_name(file, file_size, size_of_headers, sections, name_rva)?;
    }
    Err(error(
        "managed_process.atomic_windows_import_table_unterminated",
    ))
}

fn require_import_name(
    file: &mut File,
    file_size: u64,
    size_of_headers: u64,
    sections: &[PeSection],
    name_rva: u64,
) -> io::Result<()> {
    if name_rva == 0 {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }
    let offset = rva_to_file_offset(name_rva, 1, file_size, size_of_headers, sections)?;
    file.seek(io::SeekFrom::Start(offset))?;
    let mut name = Vec::new();
    for _ in 0..MAX_IMPORT_NAME_BYTES {
        let mut byte = [0_u8; 1];
        file.read_exact(&mut byte)?;
        if byte[0] == 0 {
            break;
        }
        name.push(byte[0]);
    }
    if name.is_empty()
        || name.len() as u64 == MAX_IMPORT_NAME_BYTES
        || !name.is_ascii()
        || name.contains(&b'/')
        || name.contains(&b'\\')
        || name.contains(&b':')
        || !name
            .get(name.len().saturating_sub(4)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(b".dll"))
    {
        return Err(error("managed_process.atomic_windows_import_name_invalid"));
    }
    Ok(())
}

fn rva_to_file_offset(
    rva: u64,
    length: u64,
    file_size: u64,
    size_of_headers: u64,
    sections: &[PeSection],
) -> io::Result<u64> {
    if rva < size_of_headers {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }

    let mut resolved = None;
    for section in sections {
        let relative = match rva.checked_sub(section.virtual_address) {
            Some(relative) if relative < section.virtual_span => relative,
            Some(_) | None => continue,
        };
        if relative
            .checked_add(length)
            .is_none_or(|end| end > section.raw_size)
        {
            return Err(error("managed_process.atomic_windows_program_not_pe"));
        }
        let offset = checked_add(section.raw_offset, relative)?;
        checked_end(offset, length, file_size)?;
        if resolved.replace(offset).is_some() {
            return Err(error("managed_process.atomic_windows_program_not_pe"));
        }
    }
    resolved.ok_or_else(|| error("managed_process.atomic_windows_program_not_pe"))
}

fn ranges_overlap(left_start: u64, left_size: u64, right_start: u64, right_size: u64) -> bool {
    left_start < right_start.saturating_add(right_size)
        && right_start < left_start.saturating_add(left_size)
}

fn checked_add(left: u64, right: u64) -> io::Result<u64> {
    left.checked_add(right)
        .ok_or_else(|| error("managed_process.atomic_windows_program_not_pe"))
}

fn checked_end(offset: u64, length: u64, limit: u64) -> io::Result<u64> {
    let end = checked_add(offset, length)?;
    if end > limit {
        return Err(error("managed_process.atomic_windows_program_not_pe"));
    }
    Ok(end)
}

fn read_u32(file: &mut File, offset: u64, file_size: u64) -> io::Result<u32> {
    Ok(u32::from_le_bytes(read_at::<4>(file, offset, file_size)?))
}

fn read_at<const N: usize>(file: &mut File, offset: u64, file_size: u64) -> io::Result<[u8; N]> {
    checked_end(offset, N as u64, file_size)?;
    file.seek(io::SeekFrom::Start(offset))?;
    let mut bytes = [0_u8; N];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn error(message: &'static str) -> io::Error {
    io::Error::other(message)
}

#[cfg(test)]
#[path = "managed_process_atomic_windows_tests.rs"]
mod tests;
