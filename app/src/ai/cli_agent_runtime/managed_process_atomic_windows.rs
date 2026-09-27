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
use std::os::windows::io::{AsHandle as _, AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use command::blocking::Command;
use command::windows::AppContainerProbe;
use windows::Win32::Foundation::{
    DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, DUPLICATE_SAME_ACCESS, DuplicateHandle,
    ERROR_SEM_TIMEOUT, EXCEPTION_BREAKPOINT, FILETIME, GENERIC_READ, HANDLE, HLOCAL, LocalFree,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetLengthSid, IsValidAcl, IsValidSid,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, GetFinalPathNameByHandleW,
    VOLUME_NAME_DOS,
};
use windows::Win32::System::Diagnostics::Debug::{
    CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, ContinueDebugEvent, DEBUG_EVENT,
    DEBUG_EVENT_CODE, EXCEPTION_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT, EXIT_THREAD_DEBUG_EVENT,
    LOAD_DLL_DEBUG_EVENT, OUTPUT_DEBUG_STRING_EVENT, RIP_EVENT, UNLOAD_DLL_DEBUG_EVENT,
    WaitForDebugEvent,
};
use windows::Win32::System::SystemInformation::{
    GetSystemDirectoryW, GetSystemTimePreciseAsFileTime,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, TerminateProcess, WaitForSingleObject,
};
use windows::core::HRESULT;

use super::{
    AtomicDirectoryIdentity, ExpectedFileId, ExpectedFileIdentity, WindowsChildImage, sha256_file,
};

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
    child_image: Option<WindowsChildImage>,
    package_images: Option<Vec<(bool, SystemHelperLease)>>,
    npm_console_host: Option<SystemHelperLease>,
}

/// 固定系统 helper 的文件和祖先租约；不接受同目录其他程序或 DLL。
#[derive(Debug)]
struct SystemHelperLease {
    program: File,
    identity: LeasedIdentity,
    sha256: String,
    ancestors: Vec<AncestorLease>,
    npm_role: NpmProcessRole,
}

impl SystemHelperLease {
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            program: self.program.try_clone()?,
            identity: self.identity,
            sha256: self.sha256.clone(),
            npm_role: self.npm_role,
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
    child_image: Option<WindowsChildImage>,
    package_images: Option<Vec<(bool, SystemHelperLease)>>,
    npm_console_host: Option<SystemHelperLease>,
    child_images: HashMap<u32, SystemHelperLease>,
    component_images: HashMap<u32, Vec<SystemHelperLease>>,
    processes: HashMap<u32, OwnedHandle>,
    held_package_processes: Vec<OwnedHandle>,
    initial_breakpoints: HashSet<u32>,
    pending_event: Option<(u32, u32, DEBUG_EVENT_CODE)>,
    root_exit_observed: bool,
    cancellation: Option<Arc<AtomicBool>>,
    npm_diagnostics: Option<NpmProcessDiagnostics>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NpmProcessRole {
    Root,
    Node,
    Codex,
    Console,
    BoundOther,
    Unknown,
}

impl NpmProcessRole {
    fn bound_image(path: &Path) -> Self {
        match path.file_name().and_then(|name| name.to_str()) {
            Some(name) if name.eq_ignore_ascii_case("node.exe") => Self::Node,
            Some(name) if name.eq_ignore_ascii_case("codex.exe") => Self::Codex,
            Some(_) | None => Self::BoundOther,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Node => "node",
            Self::Codex => "codex",
            Self::Console => "console",
            Self::BoundOther => "bound-other",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug)]
struct NpmProcessDiagnostics {
    started: Instant,
    roles: HashMap<u32, NpmProcessRole>,
    pending_exit_code: Option<u32>,
    cleanup: bool,
    cancel_observed: bool,
    termination_requested_at: Option<u64>,
}

struct NpmProcessEvent {
    phase: &'static str,
    mode: &'static str,
    role: &'static str,
    elapsed_ms: u128,
    native_exit_code: Option<u32>,
    cancel_observed: bool,
    remaining: [usize; 6],
}

impl NpmProcessDiagnostics {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            roles: HashMap::new(),
            pending_exit_code: None,
            cleanup: false,
            cancel_observed: false,
            termination_requested_at: None,
        }
    }

    fn received(&mut self, process_id: u32, code: DEBUG_EVENT_CODE, exit_code: Option<u32>) {
        if code == CREATE_PROCESS_DEBUG_EVENT {
            // 清理阶段也可能首次收到 CREATE；未经过原映像门禁时只能记为 unknown。
            self.roles
                .entry(process_id)
                .or_insert(NpmProcessRole::Unknown);
        }
        self.pending_exit_code = exit_code;
    }

    fn begin_cleanup(&mut self, cancel_observed: bool) {
        self.cleanup = true;
        self.cancel_observed |= cancel_observed;
    }

    fn creation_timing(&self, created_at: Option<u64>) -> (Option<bool>, Option<u64>) {
        let Some((created_at, stopped_at)) = created_at.zip(self.termination_requested_at) else {
            return (None, None);
        };
        (
            Some(created_at <= stopped_at),
            stopped_at
                .checked_sub(created_at)
                .map(|ticks| ticks / 10_000),
        )
    }

    fn continued(&mut self, process_id: u32, code: DEBUG_EVENT_CODE) -> Option<NpmProcessEvent> {
        let phase = if code == CREATE_PROCESS_DEBUG_EVENT {
            "create_continued"
        } else if code == EXIT_PROCESS_DEBUG_EVENT {
            "exit_continued"
        } else {
            return None;
        };
        let role = self
            .roles
            .get(&process_id)
            .copied()
            .unwrap_or(NpmProcessRole::Unknown);
        if code == EXIT_PROCESS_DEBUG_EVENT {
            self.roles.remove(&process_id);
        }
        let mut remaining = [0; 6];
        for role in self.roles.values() {
            remaining[*role as usize] += 1;
        }
        Some(NpmProcessEvent {
            phase,
            mode: if self.cleanup { "cleanup" } else { "normal" },
            role: role.as_str(),
            elapsed_ms: self.started.elapsed().as_millis(),
            native_exit_code: self.pending_exit_code.take(),
            cancel_observed: self.cancel_observed,
            remaining,
        })
    }
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
    pub(super) fn set_package_images(
        &mut self,
        images: Vec<ExpectedFileIdentity>,
    ) -> io::Result<()> {
        if images.is_empty() || images.len() > 64 || self.child_image.is_some() {
            return Err(error("npm 包探针映像集合无效"));
        }
        let mut identities = HashSet::new();
        let mut leases = Vec::new();
        for expected in images {
            let dll = expected
                .path
                .extension()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("dll"));
            let lease = prepare_package_image(&expected, dll)?;
            if !identities.insert((lease.identity.id.volume, lease.identity.id.index)) {
                return Err(error("npm 包探针映像身份重复"));
            }
            leases.push((dll, lease));
        }
        self.package_images = Some(leases);
        Ok(())
    }

    /// npm 公共入口需要的唯一系统控制台依赖，必须在派生前锁定；其他调用方不隐式启用。
    pub(super) fn enable_npm_console_host(&mut self) -> io::Result<()> {
        if self.package_images.is_none() || self.npm_console_host.is_some() {
            return Err(error(
                "managed_process.atomic_windows_console_binding_invalid",
            ));
        }
        self.npm_console_host = Some(prepare_npm_console_host(&self.system_directory)?);
        Ok(())
    }

    pub(super) fn set_child_image(&mut self, image: Option<WindowsChildImage>) -> io::Result<()> {
        if let Some(image) = &image {
            image.validate()?;
        }
        self.child_image = image;
        Ok(())
    }

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
        let mut session = self.prepare_image_debug_session()?;
        session.verify_initial_image(root_process_id, true, None)?;
        drop(self);
        Ok(session)
    }

    /// 包探针在派生前准备状态，首事件失败时仍由调用方持有待继续的事件。
    pub(super) fn prepare_image_debug_session(&self) -> io::Result<WindowsImageDebugSession> {
        Ok(WindowsImageDebugSession {
            root_process_id: 0,
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
            child_image: self.child_image.clone(),
            package_images: self
                .package_images
                .as_ref()
                .map(|images| {
                    images
                        .iter()
                        .map(|(dll, lease)| Ok((*dll, lease.try_clone()?)))
                        .collect::<io::Result<Vec<_>>>()
                })
                .transpose()?,
            npm_console_host: self
                .npm_console_host
                .as_ref()
                .map(SystemHelperLease::try_clone)
                .transpose()?,
            child_images: HashMap::new(),
            component_images: HashMap::new(),
            processes: HashMap::new(),
            held_package_processes: Vec::new(),
            initial_breakpoints: HashSet::new(),
            pending_event: None,
            root_exit_observed: false,
            cancellation: None,
            npm_diagnostics: self
                .npm_console_host
                .is_some()
                .then(NpmProcessDiagnostics::new),
        })
    }
}

impl WindowsImageDebugSession {
    pub(super) fn bind_cancellation(&mut self, cancellation: Arc<AtomicBool>) {
        self.cancellation = Some(cancellation);
    }

    pub(super) fn record_package_termination_request(&mut self) {
        if let Some(diagnostics) = &mut self.npm_diagnostics {
            diagnostics
                .termination_requested_at
                .get_or_insert_with(|| filetime_ticks(unsafe { GetSystemTimePreciseAsFileTime() }));
        }
    }

    fn cleanup_image_role(&self, file: &File) -> NpmProcessRole {
        let Ok(identity) = inspect_handle(file) else {
            return NpmProcessRole::Unknown;
        };
        if !is_plain_kind(identity.attributes, false) {
            return NpmProcessRole::Unknown;
        }
        if identity.id == self.expected_program_id && identity.size == self.expected_program_size {
            return NpmProcessRole::Root;
        }
        if let Some(console) = &self.npm_console_host
            && identity == console.identity
        {
            return NpmProcessRole::Console;
        }
        self.package_images
            .as_ref()
            .and_then(|images| {
                images
                    .iter()
                    .find(|(dll, lease)| !*dll && identity == lease.identity)
            })
            .map_or(NpmProcessRole::Unknown, |(_, lease)| lease.npm_role)
    }

    fn record_cleanup_create(&self, event: &DEBUG_EVENT) {
        let information = unsafe { event.u.CreateProcessInfo };
        // 接管并释放本次 hFile；观察失败只能成为未知，不能阻断原终止和 Continue。
        let file = file_from_debug_handle(information.hFile).ok();
        let Some(diagnostics) = &self.npm_diagnostics else {
            return;
        };
        // 只匹配已持租约的文件身份；这不是正常阶段的映像授权，不回填 roles。
        let observed_image_role = file
            .as_ref()
            .map_or(NpmProcessRole::Unknown, |file| {
                self.cleanup_image_role(file)
            })
            .as_str();
        let (created_before_stop, creation_age_at_stop_ms) =
            diagnostics.creation_timing(process_creation_time(information.hProcess));
        let elapsed_ms = diagnostics.started.elapsed().as_millis();
        // FILETIME 属于系统时钟，仅供诊断；不证明用户代码执行，也不参与清理判定。
        warp_core::safe_eprintln!(
            safe: ("managed_process.windows_npm_cleanup_create observed_image_role={observed_image_role} binding=lease_identity_only created_before_stop={created_before_stop:?} creation_age_at_stop_ms={creation_age_at_stop_ms:?} elapsed_ms={elapsed_ms}"),
            full: ("managed_process.windows_npm_cleanup_create observed_image_role={observed_image_role} binding=lease_identity_only created_before_stop={created_before_stop:?} creation_age_at_stop_ms={creation_age_at_stop_ms:?} elapsed_ms={elapsed_ms}")
        );
    }

    fn next_event(
        &mut self,
        deadline: Instant,
        timeout_message: &'static str,
    ) -> io::Result<DEBUG_EVENT> {
        if self.pending_event.is_some() {
            return Err(error(
                "managed_process.atomic_windows_debug_event_still_pending",
            ));
        }
        let event = match self.cancellation.as_deref() {
            Some(cancellation) => {
                wait_for_cancellable_debug_event(deadline, timeout_message, cancellation)?
            }
            None => wait_for_debug_event_until(deadline, timeout_message)?,
        };
        self.pending_event = Some((event.dwProcessId, event.dwThreadId, event.dwDebugEventCode));
        if let Some(diagnostics) = &mut self.npm_diagnostics {
            let exit_code = (event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT)
                .then(|| unsafe { event.u.ExitProcess.dwExitCode });
            diagnostics.received(event.dwProcessId, event.dwDebugEventCode, exit_code);
        }
        Ok(event)
    }

    fn continue_pending(&mut self, status: windows::Win32::Foundation::NTSTATUS) -> io::Result<()> {
        let (process_id, thread_id, code) = self
            .pending_event
            .ok_or_else(|| error("managed_process.atomic_windows_debug_event_missing"))?;
        unsafe { ContinueDebugEvent(process_id, thread_id, status) }.map_err(io::Error::other)?;
        self.pending_event = None;
        // EXIT 只有继续成功后才释放调试器持有的进程句柄并计入清理完成。
        if code == EXIT_PROCESS_DEBUG_EVENT {
            self.processes.remove(&process_id);
            self.initial_breakpoints.remove(&process_id);
            self.child_images.remove(&process_id);
            self.component_images.remove(&process_id);
            if process_id == self.root_process_id {
                self.root_exit_observed = true;
            }
        }
        // 诊断只在原 Continue 成功后推进，不参与映像授权、事件继续或清理完成判定。
        let cancel_observed = self
            .cancellation
            .as_ref()
            .is_some_and(|value| value.load(Ordering::Acquire));
        if let Some(diagnostics) = &mut self.npm_diagnostics {
            diagnostics.cancel_observed |= cancel_observed;
            if let Some(event) = diagnostics.continued(process_id, code) {
                let NpmProcessEvent {
                    phase,
                    mode,
                    role,
                    elapsed_ms,
                    native_exit_code,
                    cancel_observed,
                    remaining,
                } = event;
                let [
                    remaining_root,
                    remaining_node,
                    remaining_codex,
                    remaining_console,
                    remaining_bound_other,
                    remaining_unknown,
                ] = remaining;
                warp_core::safe_eprintln!(
                    safe: ("managed_process.windows_npm_event phase={phase} mode={mode} role={role} elapsed_ms={elapsed_ms} native_exit_code={native_exit_code:?} cancel_observed={cancel_observed} remaining_root={remaining_root} remaining_node={remaining_node} remaining_codex={remaining_codex} remaining_console={remaining_console} remaining_bound_other={remaining_bound_other} remaining_unknown={remaining_unknown}"),
                    full: ("managed_process.windows_npm_event phase={phase} mode={mode} role={role} elapsed_ms={elapsed_ms} native_exit_code={native_exit_code:?} cancel_observed={cancel_observed} remaining_root={remaining_root} remaining_node={remaining_node} remaining_codex={remaining_codex} remaining_console={remaining_console} remaining_bound_other={remaining_bound_other} remaining_unknown={remaining_unknown}")
                );
                if code == EXIT_PROCESS_DEBUG_EVENT
                    && process_id == self.root_process_id
                    && let Some(native_exit_code) = native_exit_code
                {
                    // 保留旧根 EXIT 字段；新旧事件统一使用会话时钟，清理 EXIT 明示 mode。
                    let remaining = self.processes.len();
                    let held = self.held_package_processes.len();
                    warp_core::safe_eprintln!(
                        safe: ("managed_process.windows_npm_phase phase=root_exit_continued elapsed_ms={elapsed_ms} remaining_debug_processes={remaining} held_process_count={held} native_exit_code={native_exit_code} mode={mode} cancel_observed={cancel_observed}"),
                        full: ("managed_process.windows_npm_phase phase=root_exit_continued elapsed_ms={elapsed_ms} remaining_debug_processes={remaining} held_process_count={held} native_exit_code={native_exit_code} mode={mode} cancel_observed={cancel_observed}")
                    );
                }
            }
        }
        Ok(())
    }

    fn verify_initial_image(
        &mut self,
        root_process_id: u32,
        reject_on_error: bool,
        container: Option<&AppContainerProbe>,
    ) -> io::Result<()> {
        self.root_process_id = root_process_id;
        let event = self.next_event(
            Instant::now() + DEBUG_INITIAL_TIMEOUT,
            "managed_process.atomic_windows_initial_debug_event_timed_out",
        )?;
        if event.dwDebugEventCode != CREATE_PROCESS_DEBUG_EVENT
            || event.dwProcessId != root_process_id
        {
            close_unconsumed_debug_image(&event)?;
            // 未识别首事件时不放行任何用户态执行；外层严格 Job 清理整棵树。
            return Err(error(
                "managed_process.atomic_windows_initial_debug_event_invalid",
            ));
        }
        if let Err(failure) = self.handle_create_process(&event, true, container) {
            if reject_on_error {
                self.reject_event_and_drain(&event);
            }
            return Err(failure);
        }
        self.continue_pending(DBG_CONTINUE)
    }

    pub(super) fn verify_package_initial_image(&mut self, root_process_id: u32) -> io::Result<()> {
        if self.package_images.is_none() {
            return Err(error(
                "managed_process.atomic_windows_package_images_missing",
            ));
        }
        self.verify_initial_image(root_process_id, false, None)
    }

    pub(super) fn verify_package_initial_image_in_container(
        &mut self,
        container: &AppContainerProbe,
    ) -> io::Result<()> {
        if self.package_images.is_none() {
            return Err(error(
                "managed_process.atomic_windows_package_images_missing",
            ));
        }
        self.verify_initial_image(container.id(), false, Some(container))
    }

    /// loader 运行期间持续验证每个根进程、子进程与 DLL 的实际映像句柄。
    ///
    /// Windows 的初始调试断点发生在静态 DLL 已映射、但任何 DLL 初始化例程执行前；
    /// 后续 `LOAD_DLL_DEBUG_EVENT` 同样在新映像初始化前暂停。拒绝事件时先终止全部
    /// debuggee，再继续并排空事件，未验证映像没有执行窗口。
    pub(super) fn wait_for_exit(&mut self, child: &mut Child) -> io::Result<ExitStatus> {
        self.drain_until_exit()?;
        child.wait()
    }

    /// 原生 CreateProcess 探针持有自己的进程句柄，仍共享完全相同的映像事件校验。
    pub(super) fn drain_until_exit(&mut self) -> io::Result<()> {
        self.drain_events(true, None)
    }

    /// 专属包探针由调用方先终止精确 Job，再收敛保留下来的失败事件。
    pub(super) fn drain_package_until_exit(&mut self) -> io::Result<()> {
        if self.package_images.is_none() {
            return Err(error(
                "managed_process.atomic_windows_package_images_missing",
            ));
        }
        self.drain_events(false, None)
    }

    pub(super) fn drain_package_in_container_until_exit(
        &mut self,
        container: &AppContainerProbe,
    ) -> io::Result<()> {
        if self.package_images.is_none() {
            return Err(error(
                "managed_process.atomic_windows_package_images_missing",
            ));
        }
        self.drain_events(false, Some(container))
    }

    fn drain_events(
        &mut self,
        reject_on_error: bool,
        container: Option<&AppContainerProbe>,
    ) -> io::Result<()> {
        let started = Instant::now();
        let deadline = started + DEBUG_SESSION_TIMEOUT;
        if container.is_some() {
            for process in self.processes.values() {
                self.held_package_processes.push(process.try_clone()?);
            }
        }
        loop {
            let event = self.next_event(
                deadline,
                "managed_process.atomic_windows_debug_session_timed_out",
            )?;
            let continue_status = match self.validate_event_in_container(&event, container) {
                Ok(status) => status,
                Err(failure) => {
                    if reject_on_error {
                        self.reject_event_and_drain(&event);
                    }
                    return Err(failure);
                }
            };
            if container.is_some() && event.dwDebugEventCode == CREATE_PROCESS_DEBUG_EVENT {
                self.held_package_processes
                    .push(self.processes[&event.dwProcessId].try_clone()?);
            }
            self.continue_pending(continue_status)?;
            if self.root_exit_observed && self.processes.is_empty() {
                // EXIT Continue 仅释放调试事件；仍须逐个确认已授权映像的真实进程句柄 signaled。
                return self.wait_for_package_processes_exit(deadline);
            }
        }
    }

    fn validate_event_in_container(
        &mut self,
        event: &DEBUG_EVENT,
        container: Option<&AppContainerProbe>,
    ) -> io::Result<windows::Win32::Foundation::NTSTATUS> {
        match event.dwDebugEventCode {
            CREATE_PROCESS_DEBUG_EVENT => {
                self.handle_create_process(
                    event,
                    event.dwProcessId == self.root_process_id,
                    container,
                )?;
                Ok(DBG_CONTINUE)
            }
            // 原始线程句柄由 ContinueDebugEvent 在 EXIT_* 时关闭，不能提前释放。
            CREATE_THREAD_DEBUG_EVENT => Ok(DBG_CONTINUE),
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
            EXIT_PROCESS_DEBUG_EVENT => Ok(DBG_CONTINUE),
            LOAD_DLL_DEBUG_EVENT => {
                let information = unsafe { event.u.LoadDll };
                let file = file_from_debug_handle(information.hFile)?;
                if let Some((_, lease)) = self.package_images.as_ref().and_then(|images| {
                    images.iter().find(|(dll, lease)| {
                        *dll && inspect_handle(&file)
                            .is_ok_and(|identity| identity.id == lease.identity.id)
                    })
                }) {
                    lease.verify_image(&file)?;
                } else if self.verify_system_image(&file).is_err() {
                    let dependencies = self.component_images.entry(event.dwProcessId).or_default();
                    let identity = inspect_handle(&file)?;
                    if let Some(held) = dependencies
                        .iter()
                        .find(|held| held.identity.id == identity.id)
                    {
                        held.verify_image(&file)?;
                    } else {
                        if dependencies.len() >= 256 {
                            return Err(error("managed_process.atomic_windows_component_limit"));
                        }
                        let lease = prepare_component_image(&file, &self.system_directory, true)
                            .inspect_err(|_| {
                                #[cfg(test)]
                                if let Ok(path) = final_path_from_handle(&file) {
                                    tests::record_rejected_image(
                                        &path,
                                        &self.system_directory.file,
                                    );
                                }
                            })?;
                        dependencies.push(lease);
                    }
                }
                Ok(DBG_CONTINUE)
            }
            RIP_EVENT => Err(error("managed_process.atomic_windows_loader_rip_event")),
            code => Err(io::Error::other(format!(
                "managed_process.atomic_windows_unknown_debug_event:{code:?}"
            ))),
        }
    }

    fn handle_create_process(
        &mut self,
        event: &DEBUG_EVENT,
        root: bool,
        container: Option<&AppContainerProbe>,
    ) -> io::Result<()> {
        let information = unsafe { event.u.CreateProcessInfo };
        // hFile 属于调试器；即使复制进程句柄失败，也必须释放它。
        let file = file_from_debug_handle(information.hFile);
        let process = duplicate_process_handle(information.hProcess)?;
        if self.processes.insert(event.dwProcessId, process).is_some() {
            return Err(error(
                "managed_process.atomic_windows_duplicate_process_event",
            ));
        }
        let file = file?;
        if self.npm_console_host.is_some() {
            container
                .ok_or_else(|| error("managed_process.atomic_windows_console_container_missing"))
                .and_then(|container| {
                    container.verify_package_process(self.processes[&event.dwProcessId].as_handle())
                })
                .inspect_err(|_| {
                    let details = self.package_image_rejection_fields(&file);
                    // 仅记录固定阶段与已脱敏映像证据；不输出 token、SID、参数或完整路径。
                    warp_core::safe_eprintln!(
                        safe: ("managed_process.windows_npm_boundary_rejected_image phase=package_process_boundary image={details}"),
                        full: ("managed_process.windows_npm_boundary_rejected_image phase=package_process_boundary image={details}")
                    );
                })?;
        }
        // 原生安装器会再次执行当前 CLI 查询版本；必须仍是根映像的同一文件身份与摘要。
        let role = if root || inspect_handle(&file)?.id == self.expected_program_id {
            self.verify_root_image(&file)?;
            if root {
                NpmProcessRole::Root
            } else {
                NpmProcessRole::BoundOther
            }
        } else if let Some(images) = &self.package_images {
            let identity = inspect_handle(&file)?;
            if let Some(console) = &self.npm_console_host
                && identity.id == console.identity.id
            {
                verify_npm_console_host(console, &file, &self.system_directory)?;
                NpmProcessRole::Console
            } else {
                let (_, lease) = images
                    .iter()
                    .find(|(dll, lease)| !*dll && lease.identity.id == identity.id)
                    .ok_or_else(|| {
                        let details = self.package_image_rejection_fields(&file);
                        // 监督 worker 尚未初始化 GUI 日志；仅向已封存的 stderr 写固定分类和句柄证据。
                        warp_core::safe_eprintln!(
                            safe: ("managed_process.windows_npm_rejected_image={details}"),
                            full: ("managed_process.windows_npm_rejected_image={details}")
                        );
                        error("npm 包探针拒绝未绑定的子进程映像")
                    })?;
                lease.verify_image(&file)?;
                lease.npm_role
            }
        } else if let Some(helper) = &self.powershell
            && inspect_handle(&file)?.id == helper.identity.id
        {
            helper.verify_image(&file)?;
            NpmProcessRole::Unknown
        } else if self.verify_system_image(&file).is_ok() {
            NpmProcessRole::Unknown
        } else if let Ok(lease) = prepare_component_image(&file, &self.system_directory, false) {
            self.child_images.insert(event.dwProcessId, lease);
            NpmProcessRole::Unknown
        } else if let Some(expected) = &self.child_image {
            let lease = prepare_child_image(&file, expected).inspect_err(|_| {
                #[cfg(test)]
                if let Ok(path) = final_path_from_handle(&file) {
                    tests::record_rejected_image(&path, &self.system_directory.file);
                }
            })?;
            self.child_images.insert(event.dwProcessId, lease);
            NpmProcessRole::Unknown
        } else {
            #[cfg(test)]
            if let Ok(path) = final_path_from_handle(&file) {
                tests::record_rejected_image(&path, &self.system_directory.file);
            }
            return Err(error(
                "managed_process.atomic_windows_loaded_image_outside_system_directory",
            ));
        };
        if let Some(diagnostics) = &mut self.npm_diagnostics {
            diagnostics.roles.insert(event.dwProcessId, role);
        }
        Ok(())
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
        if self.continue_pending(DBG_CONTINUE).is_err() {
            return;
        }
        let deadline = Instant::now() + DEBUG_DRAIN_TIMEOUT;
        while !self.processes.is_empty() {
            let Ok(next) = self.next_event(
                deadline,
                "managed_process.atomic_windows_debug_drain_timed_out",
            ) else {
                break;
            };
            if next.dwDebugEventCode == CREATE_PROCESS_DEBUG_EVENT {
                let information = unsafe { next.u.CreateProcessInfo };
                let _image = file_from_debug_handle(information.hFile);
                let Ok(process) = duplicate_process_handle(information.hProcess) else {
                    return;
                };
                let handle = HANDLE(process.as_raw_handle());
                if unsafe { TerminateProcess(handle, 1) }.is_err() {
                    return;
                }
                self.processes.insert(next.dwProcessId, process);
            } else if next.dwDebugEventCode == LOAD_DLL_DEBUG_EVENT {
                let information = unsafe { next.u.LoadDll };
                if !information.hFile.is_invalid() {
                    let _ = file_from_debug_handle(information.hFile);
                }
            }
            if self.continue_pending(DBG_CONTINUE).is_err() {
                return;
            }
        }
    }

    /// 只能在本次 AppContainer Job 已成功请求终止后调用，仍在原调试线程排空事件。
    pub(super) fn drain_terminated_processes(&mut self, root_process_id: u32) -> io::Result<()> {
        self.drain_terminated_processes_in(root_process_id, None)
    }

    pub(super) fn drain_terminated_package_in_container(
        &mut self,
        container: &AppContainerProbe,
    ) -> io::Result<()> {
        self.drain_terminated_processes_in(container.id(), Some(container))
    }

    fn drain_terminated_processes_in(
        &mut self,
        root_process_id: u32,
        container: Option<&AppContainerProbe>,
    ) -> io::Result<()> {
        if self.npm_console_host.is_some() && container.is_none() {
            return Err(error(
                "managed_process.atomic_windows_console_container_missing",
            ));
        }
        let deadline = Instant::now() + DEBUG_DRAIN_TIMEOUT;
        if self.root_process_id != 0 && self.root_process_id != root_process_id {
            return Err(error(
                "managed_process.atomic_windows_cleanup_root_mismatch",
            ));
        }
        self.root_process_id = root_process_id;
        // 调用方已经终止精确 Job；停止请求不能再次打断原线程的事件收敛。
        if let Some(diagnostics) = &mut self.npm_diagnostics {
            diagnostics.begin_cleanup(
                self.cancellation
                    .as_ref()
                    .is_some_and(|value| value.load(Ordering::Acquire)),
            );
        }
        self.cancellation = None;
        if let Some(container) = container {
            for process in self.processes.values() {
                request_debugged_process_termination(container, process)?;
                self.held_package_processes.push(process.try_clone()?);
            }
        }
        if let Some((process_id, _, _)) = self.pending_event {
            if process_id != root_process_id && !self.processes.contains_key(&process_id) {
                return Err(error(
                    "managed_process.atomic_windows_cleanup_unknown_event",
                ));
            }
            // 验证阶段已接管并释放 hFile；这里只继续原事件，不能再次构造 File。
            self.continue_pending(DBG_CONTINUE)?;
        }
        while !self.root_exit_observed || !self.processes.is_empty() {
            let event = self.next_event(
                deadline,
                "managed_process.atomic_windows_debug_drain_timed_out",
            )?;
            match event.dwDebugEventCode {
                CREATE_PROCESS_DEBUG_EVENT => {
                    self.record_cleanup_create(&event);
                    let process =
                        duplicate_process_handle(unsafe { event.u.CreateProcessInfo.hProcess })?;
                    if self.processes.insert(event.dwProcessId, process).is_some() {
                        return Err(error(
                            "managed_process.atomic_windows_duplicate_process_event",
                        ));
                    }
                    if let Some(container) = container {
                        // 先保存真实句柄；核验或终止失败时不能丢失仍待处理事件的进程身份。
                        let process = &self.processes[&event.dwProcessId];
                        request_debugged_process_termination(container, process)?;
                        self.held_package_processes.push(process.try_clone()?);
                    }
                }
                LOAD_DLL_DEBUG_EVENT => close_unconsumed_debug_image(&event)?,
                CREATE_THREAD_DEBUG_EVENT
                | EXIT_THREAD_DEBUG_EVENT
                | EXIT_PROCESS_DEBUG_EVENT
                | EXCEPTION_DEBUG_EVENT
                | OUTPUT_DEBUG_STRING_EVENT
                | UNLOAD_DLL_DEBUG_EVENT
                | RIP_EVENT => {}
                _code => {
                    return Err(error(
                        "managed_process.atomic_windows_cleanup_unknown_event",
                    ));
                }
            }
            self.continue_pending(DBG_CONTINUE)?;
        }
        self.wait_for_package_processes_exit(deadline)
    }

    fn wait_for_package_processes_exit(&mut self, deadline: Instant) -> io::Result<()> {
        // 等待失败时保留所有真实句柄；后续 abort 不能用已移除的 EXIT 记录生成空集合成功。
        wait_for_debugged_processes_exit(&self.held_package_processes, deadline)?;
        self.held_package_processes.clear();
        Ok(())
    }

    fn package_image_rejection_fields(&self, file: &File) -> serde_json::Value {
        let path = final_path_from_handle(file).ok();
        let system_path = final_path_from_handle(&self.system_directory.file).ok();
        let identity = inspect_handle(file).ok();
        let digest = identity
            .filter(|value| value.size <= MAX_NATIVE_EXECUTABLE_BYTES)
            .and_then(|_| file.try_clone().ok())
            .and_then(|mut file| sha256_file(&mut file).ok());
        let identity_unchanged =
            identity.is_some_and(|value| inspect_handle(file).ok() == Some(value));
        let images = self.package_images.as_deref().unwrap_or_default();
        serde_json::json!({
            "basename_class": rejected_image_basename(path.as_deref()),
            "directory_class": rejected_image_directory(path.as_deref(), system_path.as_deref()),
            "volume": identity.map(|value| value.id.volume),
            "file_index": identity.map(|value| value.id.index),
            "size": identity.map(|value| value.size),
            "sha256": digest.as_deref(),
            "handle_identity_unchanged": identity_unchanged,
            "package_identity_match": identity.is_some_and(|value| images.iter().any(|(dll, lease)| !*dll && lease.identity.id == value.id)),
            "package_digest_match": digest.as_ref().is_some_and(|value| images.iter().any(|(dll, lease)| !*dll && lease.sha256 == *value)),
        })
    }
}

fn rejected_image_basename(path: Option<&Path>) -> &'static str {
    let name = path
        .and_then(Path::file_name)
        .and_then(|value| value.to_str());
    [
        "cmd.exe",
        "powershell.exe",
        "node.exe",
        "conhost.exe",
        "openconsole.exe",
        "codex.exe",
        "codex-code-mode-host.exe",
        "codex-command-runner.exe",
        "codex-windows-sandbox-setup.exe",
        "codex-voice-host.exe",
        "rg.exe",
    ]
    .into_iter()
    .find(|known| name.is_some_and(|value| value.eq_ignore_ascii_case(known)))
    .unwrap_or("unknown")
}

fn rejected_image_directory(path: Option<&Path>, system: Option<&Path>) -> &'static str {
    let Some((path, system)) = path.zip(system) else {
        return "unresolved";
    };
    if path.parent() == Some(system) {
        return "windows_system32";
    }
    let relative = system
        .parent()
        .and_then(|root| path.strip_prefix(root).ok());
    match relative
        .and_then(|value| value.components().next())
        .and_then(|part| part.as_os_str().to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("system32") => "windows_system32_subdirectory",
        Some("microsoft.net") => "windows_microsoft_net",
        Some("winsxs") => "windows_winsxs",
        Some(_) => "windows_other",
        None => "outside_windows",
    }
}

fn close_unconsumed_debug_image(event: &DEBUG_EVENT) -> io::Result<()> {
    let handle = match event.dwDebugEventCode {
        CREATE_PROCESS_DEBUG_EVENT => unsafe { event.u.CreateProcessInfo.hFile },
        LOAD_DLL_DEBUG_EVENT => unsafe { event.u.LoadDll.hFile },
        _code => return Ok(()),
    };
    if !handle.is_invalid() {
        drop(file_from_debug_handle(handle)?);
    }
    Ok(())
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
        child_image: None,
        package_images: None,
        npm_console_host: None,
    })
}

fn prepare_package_image(
    expected: &ExpectedFileIdentity,
    dll: bool,
) -> io::Result<SystemHelperLease> {
    validate_expected(expected)?;
    let mut ancestors = Vec::new();
    for path in expected
        .canonical_path
        .parent()
        .ok_or_else(|| error("npm 映像缺少父目录"))?
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let file = open_ancestor(path)?;
        let identity = inspect_handle(&file)?;
        if !is_plain_kind(identity.attributes, true) {
            return Err(error("npm 映像祖先不是普通目录"));
        }
        ancestors.push(AncestorLease { file, identity });
    }
    let mut program = open_program(&expected.canonical_path)?;
    let identity = inspect_handle(&program)?;
    if Some(identity.id) != expected.file_id
        || identity.size != expected.size
        || !is_plain_kind(identity.attributes, false)
    {
        return Err(error("npm 映像身份不匹配"));
    }
    require_pe_kind(&mut program, identity.size, dll)?;
    let sha256 = sha256_file(&mut program)?;
    if sha256 != expected.sha256 || inspect_handle(&program)? != identity {
        return Err(error("npm 映像内容改变"));
    }
    Ok(SystemHelperLease {
        program,
        identity,
        sha256,
        ancestors,
        npm_role: NpmProcessRole::bound_image(&expected.canonical_path),
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

/// 子进程可以在更新器自己的下载目录中出现，但内容只能是来源层独立绑定的目标。
fn prepare_child_image(file: &File, expected: &WindowsChildImage) -> io::Result<SystemHelperLease> {
    expected.validate()?;
    let path = final_path_from_handle(file)?;
    let lease = lease_mapped_image(file, &path, false, false)?;
    if lease.identity.size != expected.size || lease.sha256 != expected.sha256 {
        return Err(error("managed_process.atomic_windows_child_image_mismatch"));
    }
    Ok(lease)
}

fn lease_mapped_image(
    file: &File,
    path: &Path,
    dll: bool,
    protected_component: bool,
) -> io::Result<SystemHelperLease> {
    let parent = path
        .parent()
        .ok_or_else(|| error("managed_process.atomic_windows_loaded_image_parent_missing"))?;
    let mut paths: Vec<_> = parent.ancestors().collect();
    paths.reverse();
    let mut ancestors = Vec::new();
    for path in paths {
        let file = open_ancestor(path)?;
        let identity = inspect_handle(&file)?;
        if !is_plain_kind(identity.attributes, true) {
            return Err(error("managed_process.atomic_windows_ancestor_not_plain"));
        }
        ancestors.push(AncestorLease { file, identity });
    }
    let mut program = open_program(path)?;
    let identity = inspect_handle(&program)?;
    if identity != inspect_handle(file)?
        || !is_plain_kind(identity.attributes, false)
        || identity.size == 0
        || identity.size > MAX_NATIVE_EXECUTABLE_BYTES
    {
        return Err(error(
            "managed_process.atomic_windows_loaded_image_not_plain",
        ));
    }
    require_pe_image(&mut program, identity.size, dll, protected_component)?;
    let sha256 = sha256_file(&mut program)?;
    let lease = SystemHelperLease {
        program,
        identity,
        sha256,
        ancestors,
        npm_role: NpmProcessRole::Unknown,
    };
    lease.verify_image(file)?;
    Ok(lease)
}

/// 只接受系统 API 锚定的组件 DLL 或精确编译器路径；这不是 Authenticode 校验。
fn prepare_component_image(
    file: &File,
    system: &AncestorLease,
    dll: bool,
) -> io::Result<SystemHelperLease> {
    let official_system = prepare_system_directory()?;
    if official_system.identity.id != system.identity.id {
        return Err(error(
            "managed_process.atomic_windows_system_directory_changed",
        ));
    }
    let system_path = final_path_from_handle(&system.file)?;
    let windows_path = system_path
        .parent()
        .ok_or_else(|| error("managed_process.atomic_windows_system_directory_changed"))?;
    let path = final_path_from_handle(file)?;
    if !(if dll {
        is_component_path(&path, windows_path)
    } else {
        is_framework_compiler_path(&path, windows_path)
    }) {
        return Err(error(
            "managed_process.atomic_windows_loaded_image_outside_system_directory",
        ));
    }
    let windows = open_ancestor(windows_path)?;
    let windows_identity = inspect_handle(&windows)?;
    let lease = lease_mapped_image(file, &path, dll, true)?;
    let mut found_windows = false;
    for ancestor in &lease.ancestors {
        if ancestor.identity.id == windows_identity.id {
            found_windows = true;
        }
        if found_windows {
            require_protected_security(&ancestor.file)?;
        }
    }
    if !found_windows {
        return Err(error(
            "managed_process.atomic_windows_system_directory_changed",
        ));
    }
    require_protected_security(&lease.program)?;
    lease.verify_image(file)?;
    Ok(lease)
}

fn is_framework_compiler_path(path: &Path, windows: &Path) -> bool {
    // Windows PowerShell 的默认 CodeDOM 编译器来自系统 CLR 4 目录，不信任 PATH 或环境覆写。
    ["Framework", "Framework64"].iter().any(|framework| {
        let runtime = windows
            .join("Microsoft.NET")
            .join(framework)
            .join("v4.0.30319");
        ["csc.exe", "cvtres.exe"].iter().any(|name| {
            path.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&runtime.join(name).as_os_str().to_string_lossy())
        })
    })
}

fn is_component_path(path: &Path, windows: &Path) -> bool {
    let image: Vec<_> = path.components().collect();
    let root: Vec<_> = windows.components().collect();
    if image.len() <= root.len() + 1
        || !image.iter().zip(&root).all(|(left, right)| {
            left.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
        })
    {
        return false;
    }
    let component = image[root.len()].as_os_str().to_string_lossy();
    let runtime_root = ["Microsoft.NET", "WinSxS", "System32"]
        .iter()
        .any(|name| component.eq_ignore_ascii_case(name));
    // .NET 的原生映像缓存仍位于 Windows/assembly，限于 NativeImages_* 子目录。
    let native_images = component.eq_ignore_ascii_case("assembly")
        && image.get(root.len() + 1).is_some_and(|name| {
            let name = name.as_os_str().to_string_lossy().to_ascii_lowercase();
            name.starts_with("nativeimages_v")
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
        });
    (runtime_root || native_images)
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
}

struct SecurityDescriptor(HLOCAL);
impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(self.0));
        }
    }
}

fn require_protected_security(file: &File) -> io::Result<()> {
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let result = unsafe {
        GetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            Some(&mut descriptor),
        )
    };
    if result.0 != 0 {
        return Err(io::Error::from_raw_os_error(result.0 as i32));
    }
    let _descriptor = SecurityDescriptor(HLOCAL(descriptor.0));
    if owner.0.is_null()
        || dacl.is_null()
        || !unsafe { IsValidSid(owner) }.as_bool()
        || !unsafe { IsValidAcl(dacl) }.as_bool()
    {
        return Err(error(
            "managed_process.atomic_windows_component_security_invalid",
        ));
    }
    let owner_bytes =
        unsafe { std::slice::from_raw_parts(owner.0.cast::<u8>(), GetLengthSid(owner) as usize) };
    let acl_bytes =
        unsafe { std::slice::from_raw_parts(dacl.cast::<u8>(), usize::from((*dacl).AclSize)) };
    if !protected_security(owner_bytes, acl_bytes) {
        return Err(error(
            "managed_process.atomic_windows_component_security_untrusted",
        ));
    }
    Ok(())
}

fn system_sid(subauthorities: &[u32]) -> Vec<u8> {
    let mut sid = vec![1, subauthorities.len() as u8, 0, 0, 0, 0, 0, 5];
    for value in subauthorities {
        sid.extend_from_slice(&value.to_le_bytes());
    }
    sid
}

fn trusted_component_sid(sid: &[u8]) -> bool {
    // SYSTEM、Administrators、TrustedInstaller；后者是 Windows Resource Protection 的服务 SID。
    sid == system_sid(&[18])
        || sid == system_sid(&[32, 544])
        || sid
            == system_sid(&[
                80, 956008885, 3418522649, 1831038044, 1853292631, 2271478464,
            ])
}

fn protected_security(owner: &[u8], acl: &[u8]) -> bool {
    if !trusted_component_sid(owner)
        || acl.len() < 8
        || !matches!(acl[0], 2 | 4)
        || usize::from(u16::from_le_bytes([acl[2], acl[3]])) != acl.len()
    {
        return false;
    }
    let count = u16::from_le_bytes([acl[4], acl[5]]);
    let mut offset = 8;
    // 具体文件写入、子项删除、删除、改 DACL/owner、通用写入及完全控制。
    const WRITE_ACCESS: u32 = 0x0000_0156 | 0x000d_0000 | 0x5000_0000;
    for _ in 0..count {
        let Some(header) = acl.get(offset..offset + 4) else {
            return false;
        };
        let size = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if size < 16 || size % 4 != 0 {
            return false;
        }
        let Some(ace) = acl.get(offset..offset + size) else {
            return false;
        };
        // 对 object/callback/未知 ACE 不推断其授权语义；普通 deny 只收紧权限。
        if !matches!(ace[0], 0 | 1) || ace[1] & !0x1f != 0 {
            return false;
        }
        let sid = &ace[8..];
        if sid[0] != 1 || sid[1] > 15 || sid.len() != 8 + usize::from(sid[1]) * 4 {
            return false;
        }
        let mask = u32::from_le_bytes(ace[4..8].try_into().expect("ACE 已验证长度"));
        if mask & !0xf11f_01ff != 0 {
            return false;
        }
        // INHERIT_ONLY 不授予当前对象权限；后续每个实际子对象仍独立检查。
        if ace[0] == 0
            && ace[1] & 0x08 == 0
            && mask & WRITE_ACCESS != 0
            && !trusted_component_sid(sid)
        {
            return false;
        }
        offset += size;
    }
    // AceCount 之外是未使用空间，不授予权限；不能要求系统分配的保留字节为零。
    true
}

/// 只信任系统 API 锚定的单个宿主；不接受 PATH、用户副本、目录通配或运行时发现的替代品。
fn prepare_npm_console_host(system: &AncestorLease) -> io::Result<SystemHelperLease> {
    let official_system = prepare_system_directory()?;
    if official_system.identity != system.identity {
        return Err(error(
            "managed_process.atomic_windows_system_directory_changed",
        ));
    }
    let path = final_path_from_handle(&system.file)?.join("conhost.exe");
    let file = open_program(&path)?;
    let lease = lease_mapped_image(&file, &path, false, true)?;
    verify_npm_console_host(&lease, &file, system)?;
    Ok(lease)
}

fn verify_npm_console_host(
    lease: &SystemHelperLease,
    file: &File,
    system: &AncestorLease,
) -> io::Result<()> {
    let official_system = prepare_system_directory()?;
    if official_system.identity != system.identity {
        return Err(error(
            "managed_process.atomic_windows_system_directory_changed",
        ));
    }
    let system_path = final_path_from_handle(&system.file)?;
    let expected_path = system_path.join("conhost.exe");
    if !final_path_from_handle(file)?
        .as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected_path.as_os_str().to_string_lossy())
    {
        return Err(error(
            "managed_process.atomic_windows_console_source_changed",
        ));
    }
    let windows_path = system_path
        .parent()
        .ok_or_else(|| error("managed_process.atomic_windows_system_directory_changed"))?;
    let windows = open_ancestor(windows_path)?;
    let windows_identity = inspect_handle(&windows)?;
    let mut found_windows = false;
    for ancestor in &lease.ancestors {
        if ancestor.identity.id == windows_identity.id {
            found_windows = true;
        }
        if found_windows {
            require_protected_security(&ancestor.file)?;
        }
    }
    if !found_windows {
        return Err(error(
            "managed_process.atomic_windows_console_source_changed",
        ));
    }
    require_protected_security(&lease.program)?;
    require_protected_security(file)?;
    lease.verify_image(file)
}

fn wait_for_debugged_processes_exit(
    processes: &[OwnedHandle],
    deadline: Instant,
) -> io::Result<()> {
    for process in processes {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait_ms = u32::try_from(remaining.as_millis()).unwrap_or(u32::MAX - 1);
        match unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), wait_ms) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                return Err(error(
                    "managed_process.atomic_windows_cleanup_process_not_signaled",
                ));
            }
            _status => return Err(io::Error::last_os_error()),
        }
    }
    Ok(())
}

/// 调用方已经成功终止精确 Job；对未归属该 Job 的调试后代另行持柄终止。
fn request_debugged_process_termination(
    container: &AppContainerProbe,
    process: &OwnedHandle,
) -> io::Result<()> {
    if container.contains_package_process(process.as_handle())? {
        return Ok(());
    }
    let handle = HANDLE(process.as_raw_handle());
    if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
        return Ok(());
    }
    unsafe { TerminateProcess(handle, 1) }.map_err(io::Error::other)
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
        npm_role: NpmProcessRole::Unknown,
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

fn filetime_ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn process_creation_time(process: HANDLE) -> Option<u64> {
    if process.is_invalid() {
        return None;
    }
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
        .ok()
        .map(|()| filetime_ticks(creation))
}

/// 调试事件中的原 process/thread 句柄由系统关闭；本模块只拥有独立副本。
fn duplicate_process_handle(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_invalid() {
        return Err(error(
            "managed_process.atomic_windows_debug_process_handle_missing",
        ));
    }
    let process = unsafe { GetCurrentProcess() };
    let mut duplicate = HANDLE::default();
    unsafe {
        DuplicateHandle(
            process,
            handle,
            process,
            &mut duplicate,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .map_err(io::Error::other)?;
    Ok(unsafe { OwnedHandle::from_raw_handle(duplicate.0) })
}

fn wait_for_debug_event(timeout_ms: u32) -> io::Result<DEBUG_EVENT> {
    let mut event = DEBUG_EVENT::default();
    unsafe { WaitForDebugEvent(&mut event, timeout_ms) }.map_err(io::Error::other)?;
    Ok(event)
}

fn wait_for_cancellable_debug_event(
    deadline: Instant,
    timeout_message: &'static str,
    cancellation: &AtomicBool,
) -> io::Result<DEBUG_EVENT> {
    loop {
        if cancellation.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "managed_process.atomic_windows_probe_cancelled",
            ));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
        }
        // Windows 调试接口必须留在创建进程的线程；控制监听只设置停止标记。
        let interval = remaining.min(Duration::from_millis(100));
        let mut event = DEBUG_EVENT::default();
        match unsafe { WaitForDebugEvent(&mut event, interval.as_millis().max(1) as u32) } {
            Ok(()) => return Ok(event),
            Err(failure) if failure.code() == HRESULT::from_win32(ERROR_SEM_TIMEOUT.0) => {}
            Err(failure) => return Err(io::Error::other(failure)),
        }
    }
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
    require_pe_kind(file, size, false)
}

fn require_pe_kind(file: &mut File, size: u64, dll: bool) -> io::Result<()> {
    require_pe_image(file, size, dll, dll)
}

fn require_pe_image(
    file: &mut File,
    size: u64,
    dll: bool,
    protected_component: bool,
) -> io::Result<()> {
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
        || (characteristics & 0x2000 != 0) != dll
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
    if !protected_component && bound_import != PeDataDirectory::default() {
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
        protected_component,
    )?;
    require_bounded_descriptor_table(
        file,
        size,
        size_of_headers,
        &sections,
        delay_import,
        DELAY_IMPORT_DESCRIPTOR_BYTES,
        true,
        false,
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
    allow_trailing_data: bool,
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

    // 受保护系统组件中的 .NET PE 可把名称、thunk 等尾部包含在普通导入目录内。
    // 仍须在完整映射的有界目录中找到全零终止项；主程序和延迟导入保持原严格合同。
    if !allow_trailing_data && directory.size % descriptor_size != 0 {
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
