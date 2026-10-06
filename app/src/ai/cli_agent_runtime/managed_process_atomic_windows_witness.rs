//! 仅一次、精确 generation/mode 的 npm 原生取证；不参与普通映像授权。

use std::fmt;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::BorrowedHandle;

use command::windows::{ShellClassificationObservation, ShellClassificationWitness};
use windows::Win32::System::Threading::{PROCESS_NAME_WIN32, QueryFullProcessImageNameW};
use windows::core::PWSTR;

use super::creation_witness::{
    CreationWitness, Disposition, Failure as CreationFailure, Summary, VerifiedImage,
};
use super::native_clr::NativeClr;
use super::native_snapshot::{
    CpuSample, Identity, PreparedModules, VerifiedModule, capture, identity, prepare_modules,
    prepare_partial_modules,
};
use super::*;

const SNAPSHOT_AFTER: Duration = Duration::from_secs(15);
const LATE_SNAPSHOT_AFTER: Duration = Duration::from_secs(240);
const MAX_WITNESS_MODULES: usize = 64;
const MAX_WITNESS_THREADS: usize = 16;
const GENERATION_ENV: &str = "INFINISHELL_WINDOWS_NATIVE_WITNESS_GENERATION";
const MODE_ENV: &str = "INFINISHELL_WINDOWS_NATIVE_WITNESS_MODE";

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum WitnessMode {
    Cmd,
    PowerShell,
}

impl WitnessMode {
    fn prepare_root_modules(
        self,
        modules: &[VerifiedModule<'_>],
    ) -> Result<(PreparedModules, Option<serde_json::Value>, bool), native_snapshot::Failure> {
        match self {
            Self::Cmd => prepare_modules(modules).map(|prepared| (prepared, None, false)),
            Self::PowerShell => {
                let result = prepare_partial_modules(modules)?;
                let partial = !result.omitted.is_empty();
                let evidence = serde_json::json!({
                    "partial":partial,"omitted":result.omitted,
                    "exception_bytes_attempted":result.exception_bytes_attempted,
                    "per_module_budget":native_snapshot::MAX_EXCEPTION_BYTES,
                    "total_budget":native_snapshot::MAX_TOTAL_EXCEPTION_BYTES,
                });
                Ok((result.prepared, Some(evidence), partial))
            }
        }
    }

    fn bind_creation(
        self,
        event: &DEBUG_EVENT,
        root: VerifiedImage<'_>,
        birth: u64,
        at_ms: u128,
    ) -> Result<Option<CreationWitness>, CreationFailure> {
        match self {
            Self::Cmd => CreationWitness::new(event, root, birth, at_ms).map(Some),
            // PS 使用独立的映像分类见证器，不构造 CMD 专用导入观察。
            Self::PowerShell => Ok(None),
        }
    }

    fn retain_module(self, count: usize, duplicate: bool) -> io::Result<bool> {
        if duplicate || count >= MAX_WITNESS_MODULES && self == Self::Cmd {
            return Err(error(
                "managed_process.native_witness_module_limit_or_duplicate",
            ));
        }
        Ok(count < MAX_WITNESS_MODULES)
    }
}

#[derive(Debug)]
struct BoundModule {
    lease: SystemHelperLease,
    base: u64,
    size: u32,
    dll: bool,
}

#[derive(Debug)]
struct RootThread {
    process: OwnedHandle,
    thread: OwnedHandle,
    identity: Identity,
    process_created: u64,
}

struct OriginalDebugEvent(DEBUG_EVENT);

impl fmt::Debug for OriginalDebugEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OriginalDebugEvent")
            .field("process_id", &self.0.dwProcessId)
            .field("thread_id", &self.0.dwThreadId)
            .field("code", &self.0.dwDebugEventCode)
            .finish()
    }
}

#[derive(Debug, Eq, PartialEq)]
enum SnapshotAction {
    Cancelled,
    Expired,
    Early,
    Late,
    Wait,
}

fn snapshot_action(
    cancelled: bool,
    expired: bool,
    elapsed: Duration,
    early_taken: bool,
    late_taken: bool,
    root_ready: bool,
) -> SnapshotAction {
    if cancelled {
        SnapshotAction::Cancelled
    } else if expired {
        SnapshotAction::Expired
    } else if root_ready && !early_taken && elapsed >= SNAPSHOT_AFTER {
        SnapshotAction::Early
    } else if root_ready && early_taken && !late_taken && elapsed >= LATE_SNAPSHOT_AFTER {
        SnapshotAction::Late
    } else {
        SnapshotAction::Wait
    }
}

impl BoundModule {
    fn creation_image(&self) -> VerifiedImage<'_> {
        VerifiedImage {
            file: &self.lease.program,
            base: self.base,
            size: self.size,
            sha256: &self.lease.sha256,
        }
    }
    fn snapshot_image(&self) -> VerifiedModule<'_> {
        VerifiedModule {
            file: &self.lease.program,
            base: self.base,
            size: self.size,
            sha256: &self.lease.sha256,
        }
    }
}

#[derive(Debug)]
pub(super) struct NativeWitness {
    generation: Uuid,
    mode: WitnessMode,
    modules: HashMap<u32, Vec<BoundModule>>,
    module_coverage_partial: bool,
    skipped_module_events: u64,
    unsupported_module_events: u64,
    births: HashMap<u32, u64>,
    creation: Option<CreationWitness>,
    completed_creation: Option<Summary>,
    creation_unavailable: Option<serde_json::Value>,
    snapshot: Option<serde_json::Value>,
    root_thread: Option<RootThread>,
    clr: Option<NativeClr>,
    classification: Option<ShellClassificationWitness>,
    completed_classification: Option<serde_json::Value>,
    expected_node: Option<Vec<u16>>,
    node_create: Option<serde_json::Value>,
    classification_entry: Option<serde_json::Value>,
    classification_return: Option<serde_json::Value>,
    pre_node_classification_entry: Option<serde_json::Value>,
    pre_node_classification_return: Option<serde_json::Value>,
    post_node_exception: Option<serde_json::Value>,
    pending_event: Option<OriginalDebugEvent>,
    continuation_failed: bool,
    expected_temp_environment: [Vec<u16>; 3],
    early_root_cpu: Option<CpuSample>,
    late_snapshot: Option<serde_json::Value>,
    failures: Vec<serde_json::Value>,
    cancelled: bool,
    exit_confirmed: bool,
}

fn classification_node_path(
    images: &[(bool, SystemHelperLease)],
    package_root: &WindowsDirectoryLease,
    mapped_root: &Path,
) -> io::Result<Vec<u16>> {
    let mapped: Vec<_> = mapped_root.as_os_str().encode_wide().collect();
    if mapped.len() != 3
        || !(b'D' as u16..=b'Z' as u16).contains(&mapped[0])
        || mapped[1..] != [58, 92]
    {
        return Err(error("native_witness.mapped_root_invalid"));
    }
    package_root.verify_for_spawn()?;
    let mut nodes = images
        .iter()
        .filter(|(dll, lease)| !dll && lease.npm_role == NpmProcessRole::Node);
    let (_, node) = nodes
        .next()
        .ok_or_else(|| error("native_witness.node_lease_missing"))?;
    if nodes.next().is_some() {
        return Err(error("native_witness.node_lease_ambiguous"));
    }
    node.verify_image(&node.program)?;
    if !node
        .ancestors
        .iter()
        .any(|ancestor| ancestor.identity.id == package_root.identity)
    {
        return Err(error("native_witness.node_outside_package_root"));
    }
    let original_root = package_root
        .ancestors
        .last()
        .ok_or_else(|| error("native_witness.package_root_lease_missing"))?;
    let root = final_path_from_handle(&original_root.file)?;
    let node_path = final_path_from_handle(&node.program)?;
    let root: Vec<_> = root.as_os_str().encode_wide().collect();
    let node_path: Vec<_> = node_path.as_os_str().encode_wide().collect();
    // 原 Node 拒写租约与目录租约先核共同身份，再逐 UTF-16 单元核对路径边界与固定布局。
    // mapped_root 来自同一 cwd 的 station 验证回调；宿主绝不重开目标登录会话的局部盘。
    let suffix = node_path
        .strip_prefix(root.as_slice())
        .and_then(|suffix| suffix.strip_prefix(&[92]))
        .ok_or_else(|| error("native_witness.node_root_path_mismatch"))?;
    if ![r"runtime\node.exe", r"install\node.exe"]
        .iter()
        .any(|relative| suffix.iter().copied().eq(relative.encode_utf16()))
    {
        return Err(error("native_witness.node_layout_mismatch"));
    }
    let expected = mapped.into_iter().chain(suffix.iter().copied()).collect();
    node.verify_image(&node.program)?;
    package_root.verify_for_spawn()?;
    Ok(expected)
}

fn enabled(
    generation: Uuid,
    mode: &str,
    value: Option<&str>,
    authorized_mode: Option<&str>,
) -> Option<WitnessMode> {
    if authorized_mode != Some(mode) || value != Some(generation.to_string().as_str()) {
        return None;
    }
    match mode {
        "cmd" => Some(WitnessMode::Cmd),
        "powershell" => Some(WitnessMode::PowerShell),
        _ => None,
    }
}

fn expected_temp_environment(environment: &[(OsString, OsString)]) -> Option<[Vec<u16>; 3]> {
    let value = |key: &str| {
        let mut matching = environment.iter().filter(|(name, _)| {
            name.to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(key))
        });
        let (_, value) = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        let value: Vec<_> = value.encode_wide().collect();
        (!value.contains(&0)).then_some(value)
    };
    Some([value("TMP")?, value("TEMP")?, value("USERPROFILE")?])
}

fn safe_failure(stage: &'static str, failure: &io::Error) -> serde_json::Value {
    let hresult = failure
        .get_ref()
        .and_then(|source| source.downcast_ref::<WindowsError>())
        .map(|source| source.code().0);
    let win32_error = failure.raw_os_error().or_else(|| {
        hresult.and_then(|code| {
            let bits = code as u32;
            (bits & 0xffff_0000 == 0x8007_0000).then_some((bits & 0xffff) as i32)
        })
    });
    // 只拆即时错误对象；不能重新读 LastError 或输出错误正文中的路径。
    serde_json::json!({"stage": stage, "kind": format!("{:?}", failure.kind()), "hresult":hresult,"win32_error":win32_error})
}

fn classification_failure(stage: &'static str, failure: &io::Error) -> serde_json::Value {
    let mut evidence = safe_failure(stage, failure);
    let detail = failure.to_string();
    // command 的固定读回诊断只含控制位及相等性；不输出其它错误正文或目标地址。
    if detail.starts_with("调试寄存器写入读回不符：addresses_equal=") {
        evidence["register_readback"] = serde_json::json!(detail);
    }
    evidence
}

fn mapped_image_size(file: &File, mode: WitnessMode, dll: bool) -> io::Result<Option<u32>> {
    let mut file = file.try_clone()?;
    let file_size = inspect_handle(&file)?.size;
    if &read_at::<2>(&mut file, 0, file_size)? != b"MZ" {
        return Err(error("native_witness.pe_signature"));
    }
    let offset = u64::from(read_u32(&mut file, DOS_HEADER_PE_OFFSET, file_size)?);
    if !(64..=MAX_PE_HEADER_OFFSET).contains(&offset) {
        return Err(error("native_witness.pe_offset"));
    }
    let header = read_at::<24>(&mut file, offset, file_size)?;
    if &header[..4] != b"PE\0\0" {
        return Err(error("native_witness.pe_signature"));
    }
    let machine = u16::from_le_bytes([header[4], header[5]]);
    let optional_size = u64::from(u16::from_le_bytes([header[20], header[21]]));
    let optional = offset
        .checked_add(24)
        .ok_or_else(|| error("native_witness.pe_offset"))?;
    checked_end(optional, optional_size, file_size)?;
    if optional_size < 60 {
        return Err(error("native_witness.pe_optional"));
    }
    let header = read_at::<60>(&mut file, optional, file_size)?;
    let magic = u16::from_le_bytes([header[0], header[1]]);
    let native_x64 = match (machine, magic) {
        (0x8664, 0x20b) if optional_size >= 112 => true,
        (0x014c, 0x10b) if optional_size >= 96 => false,
        _ => return Err(error("native_witness.pe_machine_or_optional")),
    };
    let size = u32::from_le_bytes(header[56..60].try_into().unwrap());
    if size == 0 || u64::from(size) > MAX_NATIVE_EXECUTABLE_BYTES {
        return Err(error("native_witness.pe_image_size"));
    }
    if native_x64 {
        Ok(Some(size))
    } else if mode == WitnessMode::PowerShell && dll {
        // 仅表示已授权模块不适用于 x64 原生解栈，不代表完整 PE 或托管栈已经验证。
        Ok(None)
    } else {
        Err(error("native_witness.pe_machine"))
    }
}

impl WindowsImageDebugSession {
    pub(in super::super) fn bind_native_witness(
        &mut self,
        generation: Uuid,
        mode: &str,
        environment: &[(OsString, OsString)],
        package_root: &WindowsDirectoryLease,
        mapped_root: &Path,
        record_directory: &Path,
        reader: Option<&super::super::NativeWitnessReaderBinding>,
    ) -> io::Result<()> {
        let value = std::env::var(GENERATION_ENV).ok();
        let authorized_mode = std::env::var(MODE_ENV).ok();
        if let Some(mode) = enabled(
            generation,
            mode,
            value.as_deref(),
            authorized_mode.as_deref(),
        ) && self.package_images.is_some()
            && self.npm_diagnostics.is_some()
        {
            let expected_temp_environment = expected_temp_environment(environment)
                .ok_or_else(|| error("native_witness.temp_environment_invalid"))?;
            let expected_node = match mode {
                WitnessMode::PowerShell => Some(classification_node_path(
                    self.package_images.as_deref().unwrap_or_default(),
                    package_root,
                    mapped_root,
                )?),
                WitnessMode::Cmd => None,
            };
            let clr = match (mode, reader) {
                (WitnessMode::PowerShell, Some(reader)) => {
                    Some(NativeClr::bind(generation, record_directory, reader)?)
                }
                (WitnessMode::Cmd, None) => None,
                (WitnessMode::PowerShell, None) | (WitnessMode::Cmd, Some(_)) => {
                    return Err(error("native_witness.reader_mode_mismatch"));
                }
            };
            self.native_witness = Some(NativeWitness {
                generation,
                mode,
                modules: HashMap::new(),
                module_coverage_partial: false,
                skipped_module_events: 0,
                unsupported_module_events: 0,
                births: HashMap::new(),
                creation: None,
                completed_creation: None,
                creation_unavailable: None,
                snapshot: None,
                root_thread: None,
                clr,
                classification: None,
                completed_classification: None,
                expected_node,
                node_create: None,
                classification_entry: None,
                classification_return: None,
                pre_node_classification_entry: None,
                pre_node_classification_return: None,
                post_node_exception: None,
                pending_event: None,
                continuation_failed: false,
                expected_temp_environment,
                early_root_cpu: None,
                late_snapshot: None,
                failures: Vec::new(),
                cancelled: false,
                exit_confirmed: false,
            });
        } else if reader.is_some() {
            return Err(error("native_witness.reader_not_authorized"));
        }
        Ok(())
    }

    fn native_at_ms(&self) -> u128 {
        self.npm_diagnostics
            .as_ref()
            .expect("取证仅绑定 npm 诊断时钟")
            .started
            .elapsed()
            .as_millis()
    }

    fn native_failure(&mut self, stage: &'static str, failure: impl Serialize) -> io::Error {
        if let Some(witness) = &mut self.native_witness {
            witness
                .failures
                .push(serde_json::json!({"stage":stage,"failure":failure}));
        }
        error("managed_process.native_witness_failed")
    }

    pub(super) fn native_witness_received(&mut self, event: &DEBUG_EVENT) {
        if let Some(witness) = &mut self.native_witness {
            witness.pending_event = Some(OriginalDebugEvent(*event));
        }
    }

    fn native_sequence(&self) -> io::Result<u64> {
        self.npm_diagnostics
            .as_ref()
            .and_then(|diagnostics| diagnostics.trace.as_ref())
            .map(|trace| trace.received)
            .filter(|sequence| *sequence != 0)
            .ok_or_else(|| error("native_witness.event_sequence_missing"))
    }

    pub(super) fn native_witness_load(
        &mut self,
        event: &DEBUG_EVENT,
        file: &File,
    ) -> io::Result<()> {
        if self.native_witness.is_none() {
            return Ok(());
        }
        self.native_post_node_root_sample(event)?;
        let base = unsafe { event.u.LoadDll.lpBaseOfDll } as u64;
        if event.dwProcessId == self.root_process_id {
            // CLR/DAC 与 Shell32 独立绑定原已授权 LOAD，不受快照的 64 项容量限制。
            if let Some(clr) = self
                .native_witness
                .as_mut()
                .and_then(|witness| witness.clr.as_mut())
            {
                clr.bind_runtime(file, base)?;
            }
            let shell32 = final_path_from_handle(file)?
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("shell32.dll"));
            if shell32
                && self
                    .native_witness
                    .as_ref()
                    .is_some_and(|witness| witness.classification.is_some())
            {
                let lease = self.clone_authorized_native_module(event.dwProcessId, file)?;
                let hash = hex::decode(&lease.sha256)
                    .ok()
                    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                    .ok_or_else(|| error("native_witness.shell32_digest"))?;
                let shell = self
                    .native_witness
                    .as_mut()
                    .and_then(|witness| witness.classification.as_mut())
                    .unwrap();
                let outcome = shell
                    .bind_shell32(event, file, hash)
                    .and_then(|()| shell.arm_all(event));
                if let Err(failure) = outcome {
                    return Err(self.native_failure(
                        "classification_load",
                        classification_failure("classification_load", &failure),
                    ));
                }
            }
        }
        self.native_witness_module(event.dwProcessId, file, base, true)
    }

    pub(super) fn native_witness_module(
        &mut self,
        pid: u32,
        file: &File,
        base: u64,
        dll: bool,
    ) -> io::Result<()> {
        if self.native_witness.is_none()
            || !self.npm_diagnostics.as_ref().is_some_and(|diagnostics| {
                matches!(
                    diagnostics.roles.get(&pid),
                    Some(NpmProcessRole::Root | NpmProcessRole::Node)
                )
            })
        {
            return Ok(());
        }
        // 调用方已完成独立 CREATE/LOAD 映像授权；容量只限制额外诊断租约。
        let witness = self.native_witness.as_mut().unwrap();
        let modules = witness.modules.entry(pid).or_default();
        if !witness.mode.retain_module(
            modules.len(),
            modules.iter().any(|module| module.base == base),
        )? {
            witness.module_coverage_partial = true;
            witness.skipped_module_events = witness.skipped_module_events.saturating_add(1);
            return Ok(());
        }
        let mode = witness.mode;
        // 仅在原 CREATE/LOAD 授权通过后取得独立拒写租约；路径只找回原 file identity。
        let prepared = (|| {
            let lease = if mode == WitnessMode::PowerShell && dll {
                self.clone_authorized_native_module(pid, file)?
            } else {
                lease_mapped_image(file, &final_path_from_handle(file)?, dll, false)?
            };
            let size = mapped_image_size(&lease.program, mode, dll)?;
            Ok::<_, io::Error>((lease, size))
        })();
        let (lease, size) = prepared.map_err(|failure| {
            self.native_failure("module_binding", safe_failure("module_binding", &failure))
        })?;
        let Some(size) = size else {
            let witness = self.native_witness.as_mut().unwrap();
            witness.module_coverage_partial = true;
            witness.unsupported_module_events = witness.unsupported_module_events.saturating_add(1);
            return Ok(());
        };
        let modules = self
            .native_witness
            .as_mut()
            .unwrap()
            .modules
            .entry(pid)
            .or_default();
        modules.push(BoundModule {
            lease,
            base,
            size,
            dll,
        });
        Ok(())
    }

    fn clone_authorized_native_module(
        &self,
        pid: u32,
        file: &File,
    ) -> io::Result<SystemHelperLease> {
        let identity = inspect_handle(file)?;
        let cached = self
            .package_images
            .as_ref()
            .and_then(|images| {
                images.iter().find_map(|(dll, lease)| {
                    (*dll && lease.identity.id == identity.id).then_some(lease)
                })
            })
            .or_else(|| {
                self.component_images
                    .get(&pid)
                    .and_then(|images| images.iter().find(|lease| lease.identity.id == identity.id))
            });
        if let Some(lease) = cached {
            // 普通 LOAD 已按来源完成 PE 和权限审核；不能再套用另一套包映像语法。
            lease.verify_image(file)?;
            let cloned = lease.try_clone()?;
            cloned.verify_image(file)?;
            return Ok(cloned);
        }

        // System32 授权不缓存每个 DLL；仍须从原事件与原目录身份建立拒写租约。
        self.verify_system_image(file)?;
        let path = final_path_from_handle(file)?;
        let parent = path
            .parent()
            .ok_or_else(|| error("native_witness.module_parent"))?;
        let mut paths: Vec<_> = parent.ancestors().collect();
        paths.reverse();
        let ancestors = paths
            .into_iter()
            .map(|path| {
                let file = open_ancestor(path)?;
                let identity = inspect_handle(&file)?;
                if !is_plain_kind(identity.attributes, true) {
                    return Err(error("managed_process.atomic_windows_ancestor_not_plain"));
                }
                Ok(AncestorLease { file, identity })
            })
            .collect::<io::Result<Vec<_>>>()?;
        if ancestors.last().map(|ancestor| ancestor.identity)
            != Some(self.system_directory.identity)
        {
            return Err(error(
                "managed_process.atomic_windows_system_directory_changed",
            ));
        }
        let mut program = open_program(&path)?;
        if inspect_handle(&program)? != identity
            || identity.size == 0
            || identity.size > MAX_NATIVE_EXECUTABLE_BYTES
            || !is_plain_kind(identity.attributes, false)
        {
            return Err(error(
                "managed_process.atomic_windows_loaded_image_not_plain",
            ));
        }
        let lease = SystemHelperLease {
            sha256: sha256_file(&mut program)?,
            program,
            identity,
            ancestors,
            npm_role: NpmProcessRole::Unknown,
        };
        self.verify_system_image(file)?;
        lease.verify_image(file)?;
        Ok(lease)
    }

    pub(super) fn native_witness_create(
        &mut self,
        event: &DEBUG_EVENT,
        file: &File,
        role: NpmProcessRole,
    ) -> io::Result<()> {
        if self.native_witness.is_none()
            || !matches!(role, NpmProcessRole::Root | NpmProcessRole::Node)
        {
            return Ok(());
        }
        let info = unsafe { event.u.CreateProcessInfo };
        let birth = process_creation_time(info.hProcess)
            .ok_or_else(|| error("native_witness.process_birth"))?;
        self.native_witness_module(event.dwProcessId, file, info.lpBaseOfImage as u64, false)?;
        let at_ms = self.native_at_ms();
        let sequence = self.native_sequence()?;
        let witness = self.native_witness.as_mut().unwrap();
        witness.births.insert(event.dwProcessId, birth);
        if role == NpmProcessRole::Root {
            if witness.creation.is_some() || witness.root_thread.is_some() {
                return Err(error("native_witness.duplicate_root"));
            }
            // 只从已验证的原 CREATE 事件复制；后续不能以 PID/TID 重开替代。
            let process = duplicate_process_handle(info.hProcess)?;
            let thread = duplicate_process_handle(info.hThread)?;
            let (bound, _) = match identity(
                HANDLE(process.as_raw_handle()),
                HANDLE(thread.as_raw_handle()),
                birth,
            ) {
                Ok(identity) => identity,
                Err(failure) => return Err(self.native_failure("root_identity", failure)),
            };
            if bound.process_id != event.dwProcessId || bound.thread_id != event.dwThreadId {
                return Err(error("native_witness.root_event_identity_changed"));
            }
            if let Some(clr) = &mut witness.clr {
                clr.retain_thread(
                    &process,
                    birth,
                    event.dwProcessId,
                    event.dwThreadId,
                    info.hThread,
                )?;
            }
            witness.root_thread = Some(RootThread {
                process,
                thread,
                identity: bound,
                process_created: birth,
            });
            if witness.mode == WitnessMode::PowerShell {
                witness.classification = Some(ShellClassificationWitness::new(
                    event,
                    birth,
                    *witness.generation.as_bytes(),
                    witness
                        .expected_node
                        .clone()
                        .ok_or_else(|| error("native_witness.node_lease_missing"))?,
                )?);
            }
            let root = witness.modules[&event.dwProcessId][0].creation_image();
            match witness.mode.bind_creation(event, root, birth, at_ms) {
                Ok(creation) => witness.creation = creation,
                Err(failure) if failure.coverage_unavailable() => {
                    // new 从未写线程寄存器；只保留覆盖未知，仍使用原映像/Job 的快照边界。
                    witness.creation_unavailable = Some(
                        serde_json::json!({"stage":"new","failure":failure,"restoration":"NotModified"}),
                    );
                }
                Err(failure) => return Err(self.native_failure("creation_bind", failure)),
            }
        } else if let Some(shell) = &mut witness.classification {
            // 普通 CREATE 路径已验证原 Job/token、映像身份与摘要；此处才绑定首个 Node。
            shell.set_node_created(event, sequence, birth)?;
            witness.node_create = Some(serde_json::json!({
                "sequence":sequence,"process_id":event.dwProcessId,"process_birth":birth,
                "original_create_bound":true,
            }));
        }
        Ok(())
    }

    pub(super) fn native_witness_unload(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        self.native_post_node_root_sample(event)?;
        let at_ms = self
            .npm_diagnostics
            .as_ref()
            .map(|value| value.started.elapsed().as_millis())
            .unwrap_or(0);
        let Some(witness) = &mut self.native_witness else {
            return Ok(());
        };
        if event.dwProcessId == self.root_process_id
            && let Some(shell) = &mut witness.classification
            && shell.bound_base() == Some(unsafe { event.u.UnloadDll.lpBaseOfDll } as u64)
        {
            shell.unload(event)?;
        }
        if event.dwProcessId == self.root_process_id
            && let Some(clr) = &mut witness.clr
        {
            clr.unload(unsafe { event.u.UnloadDll.lpBaseOfDll } as u64)?;
        }
        // 根 DLL 卸载后不能让旧地址的断点认领重用映像；在原停点先撤回，不再重新布点。
        if event.dwProcessId == self.root_process_id
            && let Some(creation) = &mut witness.creation
            && creation.requires_restore_or_original_exit()
            && let Err(failure) = creation.withdraw_while_stopped(event, at_ms)
        {
            return Err(self.native_failure("creation_unload_withdraw", failure));
        }
        if let Some(modules) = witness.modules.get_mut(&event.dwProcessId) {
            let base = unsafe { event.u.UnloadDll.lpBaseOfDll } as u64;
            modules.retain(|module| module.base != base);
        }
        Ok(())
    }

    pub(super) fn native_witness_process_exit(&mut self, pid: u32) {
        if let Some(witness) = &mut self.native_witness {
            witness.modules.remove(&pid);
            witness.births.remove(&pid);
            // 根线程、分类状态及 CLR 原句柄留到真实退出等待与最终核验成功后释放。
        }
    }

    pub(super) fn native_witness_exception(
        &mut self,
        event: &DEBUG_EVENT,
        deadline: Instant,
    ) -> io::Result<Option<windows::Win32::Foundation::NTSTATUS>> {
        self.native_post_node_root_sample(event)?;
        if let Some(status) = self.native_classification_exception(event, deadline)? {
            return Ok(Some(status));
        }
        self.native_post_node_exception(event, deadline)?;
        let at_ms = self
            .npm_diagnostics
            .as_ref()
            .map(|value| value.started.elapsed().as_millis())
            .unwrap_or(0);
        let Some(witness) = &mut self.native_witness else {
            return Ok(None);
        };
        if witness.cancelled {
            return Ok(None);
        }
        let Some(creation) = &mut witness.creation else {
            return Ok(None);
        };
        let initial = event.dwProcessId == self.root_process_id
            && !self.initial_breakpoints.contains(&event.dwProcessId)
            && unsafe { event.u.Exception.ExceptionRecord.ExceptionCode } == EXCEPTION_BREAKPOINT;
        if initial {
            let modules = witness
                .modules
                .get(&self.root_process_id)
                .ok_or_else(|| error("native_witness.root_modules_missing"))?;
            for module in modules {
                module.lease.verify_image(&module.lease.program)?;
            }
            let modules: Vec<_> = modules
                .iter()
                .filter(|module| module.dll)
                .map(BoundModule::creation_image)
                .collect();
            if let Err(failure) = creation.arm_on_initial_breakpoint(event, &modules, at_ms) {
                if !failure.coverage_unavailable() {
                    return Err(self.native_failure("creation_arm", failure));
                }
                // 只限确定覆盖不足；仍在原停点撤回并复核无待恢复状态。
                // NotModified 仅表示从未写入，不能记成 ReadbackVerified。
                if let Err(restore_failure) = creation.withdraw_while_stopped(event, at_ms) {
                    return Err(self.native_failure("creation_arm_withdraw", restore_failure));
                }
                if creation.requires_restore_or_original_exit() {
                    return Err(error("managed_process.native_witness_restore_unconfirmed"));
                }
                witness.creation_unavailable =
                    Some(serde_json::json!({"stage":"arm","failure":failure}));
            }
            return Ok(None);
        }
        match creation.observe_exception(event, at_ms) {
            Ok(Disposition::NotOwned) => Ok(None),
            Ok(Disposition::OwnedEntry | Disposition::OwnedReturn) => Ok(Some(DBG_CONTINUE)),
            Err(failure) => Err(self.native_failure("creation_exception", failure)),
        }
    }

    pub(super) fn native_clr_create_thread(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        if event.dwProcessId != self.root_process_id {
            return Ok(());
        }
        let Some(witness) = &mut self.native_witness else {
            return Ok(());
        };
        let Some(clr) = &mut witness.clr else {
            return Ok(());
        };
        let root = witness
            .root_thread
            .as_ref()
            .ok_or_else(|| error("native_clr.root_missing"))?;
        clr.retain_thread(
            &root.process,
            root.process_created,
            event.dwProcessId,
            event.dwThreadId,
            unsafe { event.u.CreateThread.hThread },
        )?;
        if let Some(shell) = &mut witness.classification {
            let outcome = shell
                .retain_thread(event)
                .and_then(|()| shell.arm_all(event));
            if let Err(failure) = outcome {
                return Err(self.native_failure(
                    "classification_thread",
                    classification_failure("classification_thread", &failure),
                ));
            }
        }
        self.native_post_node_root_sample(event)?;
        Ok(())
    }

    fn native_post_node_root_sample(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        if event.dwProcessId != self.root_process_id
            || self
                .native_witness
                .as_ref()
                .is_none_or(|witness| witness.classification.is_none())
        {
            return Ok(());
        }
        let sequence = self.native_sequence()?;
        let Some(shell) = self
            .native_witness
            .as_mut()
            .and_then(|witness| witness.classification.as_mut())
        else {
            return Ok(());
        };
        // 仅首个支持的原 root 活停点；不宣称覆盖所有事件类型或整个执行区间。
        if let Err(failure) = shell.sample_post_node_root_stop(event, sequence) {
            return Err(self.native_failure(
                "classification_post_node_sample",
                classification_failure("classification_post_node_sample", &failure),
            ));
        }
        Ok(())
    }

    pub(super) fn native_witness_continued(
        &mut self,
        pid: u32,
        tid: u32,
        code: DEBUG_EVENT_CODE,
    ) -> io::Result<()> {
        let cleanup = self
            .npm_diagnostics
            .as_ref()
            .is_some_and(|diagnostics| diagnostics.cleanup);
        let Some(witness) = &mut self.native_witness else {
            return Ok(());
        };
        let result = (|| {
            let event = witness
                .pending_event
                .take()
                .ok_or_else(|| error("native_witness.original_event_missing"))?
                .0;
            if (event.dwProcessId, event.dwThreadId, event.dwDebugEventCode) != (pid, tid, code) {
                return Err(error("native_witness.original_event_changed"));
            }
            if pid == self.root_process_id && code == EXIT_THREAD_DEBUG_EVENT {
                if let Some(shell) = &mut witness.classification
                    && shell.thread_handle(tid).is_ok()
                    && let Err(failure) = shell.thread_exited(&event)
                {
                    if failure.kind() == io::ErrorKind::WouldBlock {
                        // Continue 已成功；线程可稍后退出，保留原句柄并在根退出后逐一核验。
                        witness.failures.push(serde_json::json!({
                            "stage":"classification_thread_exit_deferred",
                            "process_id":pid,"thread_id":tid,
                            "failure":safe_failure("thread_exit", &failure),
                        }));
                        return Ok(());
                    }
                    return Err(failure);
                }
                if let Some(clr) = &mut witness.clr {
                    clr.thread_exited(tid);
                }
            }
            Ok(())
        })();
        if let Err(failure) = result {
            witness.failures.push(serde_json::json!({
                "stage":"original_event_continued","process_id":pid,"thread_id":tid,
                "failure":safe_failure("original_event_continued", &failure),
            }));
            if cleanup {
                // 不能因诊断钩子提前中止已终止 Job 的排空；最终仍按失败返回。
                witness.continuation_failed = true;
                return Ok(());
            }
            return Err(failure);
        }
        Ok(())
    }

    pub(super) fn native_clr_can_continue(&self) -> io::Result<()> {
        if let Some(clr) = self
            .native_witness
            .as_ref()
            .and_then(|witness| witness.clr.as_ref())
        {
            clr.ensure_reaped()?;
        }
        Ok(())
    }

    pub(super) fn native_clr_abort_after_target_termination(
        &mut self,
        deadline: Instant,
    ) -> io::Result<()> {
        if let Some(clr) = self
            .native_witness
            .as_mut()
            .and_then(|witness| witness.clr.as_mut())
        {
            clr.abort_after_target_termination(deadline)?;
        }
        Ok(())
    }

    fn native_post_node_exception(
        &mut self,
        event: &DEBUG_EVENT,
        deadline: Instant,
    ) -> io::Result<()> {
        if event.dwProcessId != self.root_process_id || self.native_witness.is_none() {
            return Ok(());
        }
        let sequence = self.native_sequence()?;
        let witness = self.native_witness.as_mut().unwrap();
        let Some(shell) = &mut witness.classification else {
            return Ok(());
        };
        let exception = match shell.observe_post_node_exception(event, sequence) {
            Ok(Some(receipt)) => receipt,
            Ok(None) => return Ok(()),
            Err(failure) => {
                return Err(self.native_failure(
                    "post_node_exception_select",
                    classification_failure("post_node_exception_select", &failure),
                ));
            }
        };
        if witness.post_node_exception.is_some() || witness.classification_entry.is_some() {
            return Err(error("native_witness.post_node_exception_phase"));
        }
        let eligible = exception.reader_eligible;
        let exception = serde_json::to_value(exception)?;
        witness.post_node_exception = Some(exception.clone());
        if eligible && shell.requires_restoration() {
            return Err(error(
                "native_witness.post_node_exception_restore_unconfirmed",
            ));
        }
        let pre_return = witness
            .pre_node_classification_return
            .as_ref()
            .ok_or_else(|| error("native_witness.post_node_exception_pre_return_missing"))?;
        let node = witness
            .node_create
            .as_ref()
            .ok_or_else(|| error("native_witness.post_node_exception_node_missing"))?;
        let root = witness
            .root_thread
            .as_ref()
            .ok_or_else(|| error("native_clr.root_missing"))?;
        let clr = witness
            .clr
            .as_mut()
            .ok_or_else(|| error("native_clr.reader_missing"))?;
        if let Err(failure) = clr.observe_post_node_exception(
            event,
            &root.process,
            root.process_created,
            self.root_process_id,
            sequence,
            &exception,
            pre_return,
            node,
            deadline,
            self.cancellation.as_deref(),
        ) {
            return Err(self.native_failure(
                "clr_post_node_exception",
                safe_failure("clr_post_node_exception", &failure),
            ));
        }
        if !eligible {
            return Ok(());
        }
        if let Err(failure) = clr.ensure_reaped() {
            return Err(self.native_failure(
                "clr_post_node_exception_reap",
                safe_failure("clr_post_node_exception_reap", &failure),
            ));
        }
        if Instant::now() >= deadline
            || self
                .cancellation
                .as_ref()
                .is_some_and(|value| value.load(Ordering::Acquire))
        {
            return Err(error(
                "native_witness.post_node_exception_resume_cancelled_or_expired",
            ));
        }
        if let Err(failure) = shell.resume_after_post_node_exception(event, sequence) {
            return Err(self.native_failure(
                "post_node_exception_resume",
                classification_failure("post_node_exception_resume", &failure),
            ));
        }
        // 不认领此异常；外层仍以原 DBG_EXCEPTION_NOT_HANDLED 交给 CLR。
        Ok(())
    }

    fn native_classification_exception(
        &mut self,
        event: &DEBUG_EVENT,
        deadline: Instant,
    ) -> io::Result<Option<windows::Win32::Foundation::NTSTATUS>> {
        if event.dwProcessId != self.root_process_id {
            return Ok(None);
        }
        if self.native_witness.is_none() {
            return Ok(None);
        }
        let sequence = self.native_sequence()?;
        let Some(witness) = &mut self.native_witness else {
            return Ok(None);
        };
        let Some(shell) = &mut witness.classification else {
            return Ok(None);
        };
        let receipt = match shell.observe(event, sequence) {
            Ok(ShellClassificationObservation::NotOwned) => return Ok(None),
            Ok(ShellClassificationObservation::OwnedSkipped) => return Ok(Some(DBG_CONTINUE)),
            Ok(ShellClassificationObservation::OwnedPreNodeEntry) => {
                if witness.node_create.is_some() || witness.pre_node_classification_entry.is_some()
                {
                    return Err(error("native_witness.pre_node_entry_phase"));
                }
                witness.pre_node_classification_entry = Some(serde_json::json!({
                    "sequence":sequence,"process_id":event.dwProcessId,"thread_id":event.dwThreadId,
                }));
                return Ok(Some(DBG_CONTINUE));
            }
            Ok(ShellClassificationObservation::OwnedPreNodeReturn(receipt)) => {
                if witness.node_create.is_some() || witness.pre_node_classification_return.is_some()
                {
                    return Err(error("native_witness.pre_node_return_phase"));
                }
                let receipt = serde_json::to_value(receipt)?;
                witness.pre_node_classification_return = Some(receipt.clone());
                if shell.requires_restoration() {
                    return Err(error("native_witness.pre_node_restore_unconfirmed"));
                }
                let clr = witness
                    .clr
                    .as_mut()
                    .ok_or_else(|| error("native_clr.reader_missing"))?;
                let root = witness
                    .root_thread
                    .as_ref()
                    .ok_or_else(|| error("native_clr.root_missing"))?;
                let observed = clr.observe_pre_node_return(
                    event,
                    &root.process,
                    root.process_created,
                    self.root_process_id,
                    sequence,
                    &receipt,
                    deadline,
                    self.cancellation.as_deref(),
                );
                if let Err(failure) = observed {
                    return Err(self.native_failure(
                        "clr_pre_node_classification",
                        safe_failure("clr_pre_node_classification", &failure),
                    ));
                }
                // 同停点 reader 必须先完成原 Job 回收；任一步失败均不重新布防或继续事件。
                if let Err(failure) = clr.ensure_reaped() {
                    return Err(self.native_failure(
                        "clr_pre_node_reap",
                        safe_failure("clr_pre_node_reap", &failure),
                    ));
                }
                if Instant::now() >= deadline
                    || self
                        .cancellation
                        .as_ref()
                        .is_some_and(|value| value.load(Ordering::Acquire))
                {
                    return Err(error("native_witness.pre_node_resume_cancelled_or_expired"));
                }
                if let Err(failure) = shell.resume_after_pre_node_return(event, sequence) {
                    return Err(self.native_failure(
                        "classification_pre_node_resume",
                        classification_failure("classification_pre_node_resume", &failure),
                    ));
                }
                return Ok(Some(DBG_CONTINUE));
            }
            Ok(ShellClassificationObservation::OwnedEntry) => {
                witness.classification_entry = Some(serde_json::json!({
                    "sequence":sequence,"process_id":event.dwProcessId,"thread_id":event.dwThreadId,
                }));
                return Ok(Some(DBG_CONTINUE));
            }
            Ok(ShellClassificationObservation::OwnedReturn(receipt)) => receipt,
            Err(failure) => {
                let evidence = classification_failure("classification_exception", &failure);
                if let Err(restore) = shell.finish_observation(event) {
                    witness.failures.push(serde_json::json!({
                        "stage":"classification_restore",
                        "failure":classification_failure("classification_restore", &restore),
                    }));
                }
                return Err(self.native_failure("classification_exception", evidence));
            }
        };
        let receipt = serde_json::to_value(receipt)?;
        witness.classification_return = Some(receipt.clone());
        if shell.requires_restoration() {
            return Err(error("native_witness.classification_restore_unconfirmed"));
        }
        let clr = witness
            .clr
            .as_mut()
            .ok_or_else(|| error("native_clr.reader_missing"))?;
        let root = witness
            .root_thread
            .as_ref()
            .ok_or_else(|| error("native_clr.root_missing"))?;
        clr.observe_native_return(
            event,
            &root.process,
            root.process_created,
            self.root_process_id,
            sequence,
            &receipt,
            deadline,
            self.cancellation.as_deref(),
        )
        .map_err(|failure| {
            self.native_failure(
                "clr_classification",
                safe_failure("clr_classification", &failure),
            )
        })?;
        Ok(Some(DBG_CONTINUE))
    }

    // PID 仅是已持句柄查询的字段；授权仍是同 Job/token 与原映像租约。
    fn native_process_role(
        &self,
        process: BorrowedHandle<'_>,
        pid: u32,
        birth: u64,
        container: &AppContainerProbe,
    ) -> io::Result<NpmProcessRole> {
        container.verify_package_process(process)?;
        let mut path = vec![0u16; MAX_WINDOWS_PATH_U16];
        let mut length = path.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                HANDLE(process.as_raw_handle()),
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut length,
            )
        }
        .map_err(io::Error::other)?;
        if length == 0 || length as usize >= path.len() {
            return Err(error("native_witness.image_path_bounds"));
        }
        path.truncate(length as usize);
        let file = open_program(&PathBuf::from(OsString::from_wide(&path)))?;
        let identity = inspect_handle(&file)?;
        if pid == self.root_process_id {
            if self
                .native_witness
                .as_ref()
                .and_then(|w| w.births.get(&pid))
                .copied()
                != Some(birth)
            {
                return Err(error("native_witness.root_birth_changed"));
            }
            self.verify_root_image(&file)?;
            return Ok(NpmProcessRole::Root);
        }
        let lease = self
            .package_images
            .as_ref()
            .and_then(|images| {
                images.iter().find(|(dll, lease)| {
                    !dll && lease.npm_role == NpmProcessRole::Node
                        && lease.identity.id == identity.id
                })
            })
            .ok_or_else(|| error("native_witness.image_not_root_or_node"))?;
        lease.1.verify_image(&file)?;
        Ok(NpmProcessRole::Node)
    }

    fn snapshot_stopped(&self, deadline: Instant) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(|value| value.load(Ordering::Acquire))
            || Instant::now() >= deadline
    }

    fn capture_native_witness(
        &mut self,
        container: &AppContainerProbe,
        deadline: Instant,
    ) -> io::Result<()> {
        if self.pending_event.is_some() {
            return Err(error("native_witness.pending_debug_event"));
        }
        let started_ms = self.native_at_ms();
        // 先占用唯一采样机会；失败也不能重采并替换原因。
        self.native_witness.as_mut().unwrap().snapshot =
            Some(serde_json::json!({"started_ms":started_ms,"complete":false}));
        // 根主线程优先使用原 CREATE 句柄，不依赖枚举、重开成功或其他线程占满额度。
        let (root_main_thread, early_root_cpu, mut balanced, partial_unwind) =
            self.capture_root_snapshot(container, deadline);
        let root_identity = self
            .native_witness
            .as_ref()
            .unwrap()
            .root_thread
            .as_ref()
            .map(|root| root.identity);
        let root_only = self.native_witness.as_ref().unwrap().mode == WitnessMode::PowerShell;
        let (expected_desktop, desktop_failure) = if root_only {
            // PS 的桌面已由原主线程快照记录，不枚举或重开其他线程。
            (None, None)
        } else if balanced && !self.snapshot_stopped(deadline) {
            match container.native_witness_desktop() {
                Ok(desktop) => (desktop, None),
                Err(failure) => (None, Some(safe_failure("expected_desktop", &failure))),
            }
        } else {
            (
                None,
                Some(serde_json::json!({"skipped":"root_unbalanced_or_cancelled_or_expired"})),
            )
        };
        let mut observations = Vec::new();
        let mut thread_count = 1;
        let processes = if !root_only && balanced && !self.snapshot_stopped(deadline) {
            container.native_witness_processes()
        } else {
            Ok(Vec::new())
        };
        match processes {
            Err(failure) => observations.push(safe_failure("job_members", &failure)),
            Ok(processes) => {
                for (pid, process) in processes {
                    if self.snapshot_stopped(deadline) {
                        observations.push(
                            serde_json::json!({"skipped":"cancelled_or_expired_before_capture"}),
                        );
                        break;
                    }
                    let process = match process {
                        Ok(process) => process,
                        Err(failure) => {
                            observations.push(serde_json::json!({"pid":pid,"failure":safe_failure("process_discovery",&failure)}));
                            continue;
                        }
                    };
                    let role = match self.native_process_role(
                        process.process(),
                        pid,
                        process.created_filetime(),
                        container,
                    ) {
                        Ok(role) => role,
                        Err(failure) => {
                            observations.push(serde_json::json!({"pid":pid,"failure":safe_failure("process_authority",&failure)}));
                            continue;
                        }
                    };
                    let witness = self.native_witness.as_ref().unwrap();
                    let modules = if witness.births.get(&pid) == Some(&process.created_filetime()) {
                        witness
                            .modules
                            .get(&pid)
                            .map(Vec::as_slice)
                            .unwrap_or_default()
                    } else {
                        &[]
                    };
                    // 未收到该进程 LOAD 的模块绝不借用另一个进程的映像表。
                    let bindings = modules
                        .iter()
                        .map(|module| {
                            module.lease.verify_image(&module.lease.program)?;
                            Ok(module.snapshot_image())
                        })
                        .collect::<io::Result<Vec<_>>>();
                    let mut module_failure = None;
                    let prepared = match bindings {
                        Ok(bindings) => match prepare_modules(&bindings) {
                            Ok(prepared) => prepared,
                            Err(failure) => {
                                module_failure = Some(serde_json::to_value(failure).unwrap());
                                prepare_modules(&[]).unwrap()
                            }
                        },
                        Err(failure) => {
                            module_failure = Some(safe_failure("module_lease", &failure));
                            prepare_modules(&[]).unwrap()
                        }
                    };
                    let mut threads = Vec::new();
                    match process.threads() {
                        Err(failure) => threads.push(safe_failure("thread_discovery", &failure)),
                        Ok(members) => {
                            for (tid, thread) in members {
                                if root_identity.is_some_and(|root| {
                                    root.process_id == pid && root.thread_id == tid
                                }) {
                                    continue;
                                }
                                if thread_count == MAX_WITNESS_THREADS {
                                    threads.push(serde_json::json!({"tid":tid,"reason":"snapshot_thread_limit"}));
                                    break;
                                }
                                let thread = match thread {
                                    Ok(thread) => thread,
                                    Err(failure) => {
                                        threads.push(serde_json::json!({"tid":tid,"failure":safe_failure("thread_open",&failure)}));
                                        continue;
                                    }
                                };
                                // 暂停前再次复核持有进程的 Job/token；上下文模块还会复核原线程归属与创建时间。
                                if let Err(failure) =
                                    container.verify_package_process(process.process())
                                {
                                    threads.push(serde_json::json!({"tid":tid,"failure":safe_failure("process_revalidate",&failure)}));
                                    break;
                                }
                                if self.snapshot_stopped(deadline) {
                                    threads.push(serde_json::json!({"tid":tid,"skipped":"cancelled_or_expired_before_capture"}));
                                    break;
                                }
                                let snapshot = capture(
                                    process.process(),
                                    process.created_filetime(),
                                    thread.as_handle(),
                                    expected_desktop,
                                    &prepared,
                                    None,
                                    None,
                                    || self.snapshot_stopped(deadline),
                                );
                                thread_count += 1;
                                balanced &= snapshot.suspension_balanced();
                                threads.push(serde_json::json!({"tid":tid,"snapshot":snapshot}));
                                if !balanced {
                                    break;
                                }
                            }
                        }
                    }
                    observations.push(serde_json::json!({"pid":pid,"created_filetime":process.created_filetime(),"role":role.as_str(),"module_failure":module_failure,"threads":threads}));
                    if !balanced {
                        break;
                    }
                }
            }
        }
        let finished_ms = self.native_at_ms();
        let witness = self.native_witness.as_mut().unwrap();
        witness.module_coverage_partial |= partial_unwind;
        witness.early_root_cpu = early_root_cpu;
        witness.snapshot = Some(
            serde_json::json!({"started_ms":started_ms,"finished_ms":finished_ms,"complete":true,"root_main_thread":root_main_thread,"module_coverage_partial":witness.module_coverage_partial,"skipped_module_events":witness.skipped_module_events,"unsupported_module_events":witness.unsupported_module_events,"unsupported_module_reason":(witness.unsupported_module_events > 0).then_some("non_x64_native_unwind"),"suspension_balanced":balanced,"desktop_failure":desktop_failure,"processes":observations}),
        );
        if let Some(creation) = &mut witness.creation
            && let Err(failure) =
                creation.note_pre_cancel_snapshot(started_ms, finished_ms, balanced)
        {
            return Err(self.native_failure("snapshot_timeline", failure));
        }
        if !balanced {
            return Err(error("managed_process.native_witness_suspend_unbalanced"));
        }
        Ok(())
    }

    fn capture_root_snapshot(
        &self,
        container: &AppContainerProbe,
        deadline: Instant,
    ) -> (serde_json::Value, Option<CpuSample>, bool, bool) {
        let observation = (|| {
            if self.snapshot_stopped(deadline) {
                return Ok((
                    serde_json::json!({"skipped":"cancelled_or_expired_before_capture"}),
                    None,
                    true,
                    false,
                ));
            }
            let witness = self.native_witness.as_ref().unwrap();
            let root = witness
                .root_thread
                .as_ref()
                .ok_or_else(|| serde_json::json!({"unknown":"original_root_missing"}))?;
            self.native_process_role(
                root.process.as_handle(),
                root.identity.process_id,
                root.process_created,
                container,
            )
            .map_err(|failure| safe_failure("root_authority", &failure))?;
            let modules = witness
                .modules
                .get(&root.identity.process_id)
                .ok_or_else(|| serde_json::json!({"unknown":"root_modules_missing"}))?;
            let bindings = modules
                .iter()
                .map(|module| {
                    module.lease.verify_image(&module.lease.program)?;
                    Ok(module.snapshot_image())
                })
                .collect::<io::Result<Vec<_>>>()
                .map_err(|failure| safe_failure("root_module_lease", &failure))?;
            let (prepared, module_preparation, partial_unwind) =
                witness.mode.prepare_root_modules(&bindings).map_err(
                    |failure| serde_json::json!({"stage":"root_prepare_modules","failure":failure}),
                )?;
            let desktop = container.native_witness_desktop();
            let desktop_failure = desktop
                .as_ref()
                .err()
                .map(|failure| safe_failure("expected_desktop", failure));
            container
                .verify_package_process(root.process.as_handle())
                .map_err(|failure| safe_failure("root_revalidate", &failure))?;
            // 租约/模块准备后再次处理取消与截止，不能为了采样拖延原终止。
            if self.snapshot_stopped(deadline) {
                return Ok((
                    serde_json::json!({"skipped":"cancelled_or_expired_before_capture","module_preparation":module_preparation}),
                    None,
                    true,
                    partial_unwind,
                ));
            }
            let sampled_at_ms = self.native_at_ms();
            let snapshot = capture(
                root.process.as_handle(),
                root.process_created,
                root.thread.as_handle(),
                desktop.ok().flatten(),
                &prepared,
                Some(&root.identity),
                Some(&witness.expected_temp_environment),
                || self.snapshot_stopped(deadline),
            );
            let balanced = snapshot.suspension_balanced();
            let cpu = snapshot.cpu_sample(sampled_at_ms);
            Ok::<_, serde_json::Value>((
                serde_json::json!({"sampled_at_ms":sampled_at_ms,"snapshot":snapshot,"desktop_failure":desktop_failure,"module_preparation":module_preparation}),
                cpu,
                balanced,
                partial_unwind,
            ))
        })();
        match observation {
            Ok(observation) => observation,
            Err(failure) => (serde_json::json!({"failure":failure}), None, true, false),
        }
    }

    fn capture_late_native_witness(
        &mut self,
        container: &AppContainerProbe,
        deadline: Instant,
    ) -> io::Result<()> {
        if self.pending_event.is_some() {
            return Err(error("native_witness.pending_debug_event"));
        }
        let started_ms = self.native_at_ms();
        self.native_witness.as_mut().unwrap().late_snapshot =
            Some(serde_json::json!({"started_ms":started_ms,"complete":false}));
        let (observation, cpu, balanced, partial_unwind) =
            self.capture_root_snapshot(container, deadline);
        let cpu_difference = match (
            self.native_witness
                .as_ref()
                .unwrap()
                .early_root_cpu
                .as_ref(),
            cpu,
        ) {
            (Some(early), Some(late)) => match late.difference_from(early) {
                Ok(difference) => {
                    serde_json::json!({"difference":difference,"earlier":early,"later":late})
                }
                Err(failure) => {
                    serde_json::json!({"failure":failure,"earlier":early,"later":late})
                }
            },
            (None, _) => serde_json::json!({"unknown":"early_root_cpu_unavailable"}),
            (Some(_), None) => serde_json::json!({"unknown":"late_root_cpu_unavailable"}),
        };
        let finished_ms = self.native_at_ms();
        let witness = self.native_witness.as_mut().unwrap();
        witness.module_coverage_partial |= partial_unwind;
        witness.late_snapshot = Some(
            serde_json::json!({"started_ms":started_ms,"finished_ms":finished_ms,"complete":true,"observation":observation,"cpu_difference":cpu_difference,"module_coverage_partial":witness.module_coverage_partial,"skipped_module_events":witness.skipped_module_events,"unsupported_module_events":witness.unsupported_module_events,"unsupported_module_reason":(witness.unsupported_module_events > 0).then_some("non_x64_native_unwind")}),
        );
        if !balanced {
            return Err(error("managed_process.native_witness_suspend_unbalanced"));
        }
        Ok(())
    }

    pub(super) fn wait_for_native_witness_event(
        &mut self,
        deadline: Instant,
        timeout_message: &'static str,
        container: &AppContainerProbe,
    ) -> io::Result<DEBUG_EVENT> {
        loop {
            let witness = self.native_witness.as_ref().unwrap();
            let action = snapshot_action(
                self.cancellation
                    .as_ref()
                    .is_some_and(|value| value.load(Ordering::Acquire)),
                Instant::now() >= deadline,
                self.npm_diagnostics.as_ref().unwrap().started.elapsed(),
                witness.snapshot.is_some(),
                witness.late_snapshot.is_some(),
                witness.root_thread.is_some()
                    && witness.births.contains_key(&self.root_process_id)
                    && witness.modules.contains_key(&self.root_process_id),
            );
            match action {
                SnapshotAction::Cancelled => {
                    if let Some(trace) = self
                        .npm_diagnostics
                        .as_mut()
                        .and_then(|value| value.trace.as_mut())
                    {
                        trace.last_boundary = "wait_cancelled";
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "managed_process.atomic_windows_probe_cancelled",
                    ));
                }
                SnapshotAction::Expired => {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
                }
                SnapshotAction::Early => {
                    self.capture_native_witness(container, deadline)?;
                }
                SnapshotAction::Late => self.capture_late_native_witness(container, deadline)?,
                SnapshotAction::Wait => {}
            }
            if self
                .cancellation
                .as_ref()
                .is_some_and(|value| value.load(Ordering::Acquire))
            {
                continue;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
            }
            let interval = remaining.min(Duration::from_millis(100));
            let diagnostics = self
                .npm_diagnostics
                .as_mut()
                .and_then(|value| value.trace.as_mut());
            match wait_for_debug_event(
                interval.as_millis().max(1) as u32,
                deadline,
                diagnostics,
                self.station_debugger.as_ref(),
            ) {
                Ok(Some(event)) => return Ok(event),
                Ok(None) => {}
                Err(failure)
                    if debug_error_code(&failure)
                        == Some(HRESULT::from_win32(ERROR_SEM_TIMEOUT.0)) => {}
                Err(failure) => return Err(failure),
            }
        }
    }

    pub(in super::super) fn note_native_witness_cancel(&mut self) {
        let at_ms = self
            .npm_diagnostics
            .as_ref()
            .map(|value| value.started.elapsed().as_millis())
            .unwrap_or(0);
        if let Some(witness) = &mut self.native_witness
            && !witness.cancelled
        {
            witness.cancelled = true;
            if let Some(creation) = &mut witness.creation
                && let Err(failure) = creation.note_cancel(at_ms)
            {
                witness
                    .failures
                    .push(serde_json::json!({"stage":"cancel_timeline","failure":failure}));
            }
        }
    }

    pub(super) fn native_witness_confirm_exit(&mut self) -> io::Result<()> {
        let at_ms = self
            .npm_diagnostics
            .as_ref()
            .map(|value| value.started.elapsed().as_millis())
            .unwrap_or(0);
        if let Some(witness) = &mut self.native_witness
            && !witness.exit_confirmed
            && self.root_exit_observed
        {
            if let Some(shell) = &mut witness.classification {
                shell.confirm_process_exit()?;
            }
            if witness.continuation_failed {
                return Err(error("native_witness.original_continuation_failed"));
            }
            if let Some(creation) = witness.creation.take() {
                match creation.finish_after_exit(at_ms) {
                    Ok(summary) => witness.completed_creation = Some(summary),
                    Err((creation, failure)) => {
                        // 确认失败保留原句柄；不能让暂取对象变成未获证的提前释放。
                        witness.creation = Some(creation);
                        return Err(self.native_failure("original_exit", failure));
                    }
                }
            }
            if let Some(clr) = &mut witness.clr {
                clr.ensure_reaped()?;
                clr.process_exited();
            }
            // 原退出和 reader 回收确认后只保留摘要；关闭进程、线程句柄再释放登录会话。
            witness.completed_classification = witness
                .classification
                .take()
                .map(|shell| serde_json::json!(shell.summary()));
            witness.root_thread.take();
            witness.exit_confirmed = true;
        }
        Ok(())
    }

    pub(in super::super) fn record_native_witness_result(&self) {
        if let Some(witness) = &self.native_witness {
            let summary = serde_json::json!({"generation":witness.generation,"mode":witness.mode,"creation_enabled":witness.mode == WitnessMode::Cmd,"module_coverage_partial":witness.module_coverage_partial,"skipped_module_events":witness.skipped_module_events,"unsupported_module_events":witness.unsupported_module_events,"unsupported_module_reason":(witness.unsupported_module_events > 0).then_some("non_x64_native_unwind"),"snapshot":witness.snapshot,"late_snapshot":witness.late_snapshot,"creation":witness.creation.as_ref().map(CreationWitness::summary).or(witness.completed_creation.as_ref()),"creation_unavailable":witness.creation_unavailable,"failures":witness.failures,"cancelled":witness.cancelled,"exit_confirmed":witness.exit_confirmed,"requires_original_exit":witness.creation.as_ref().is_some_and(CreationWitness::requires_restore_or_original_exit)});
            let mut summary = summary;
            summary["clr"] = witness
                .clr
                .as_ref()
                .map(NativeClr::summary)
                .unwrap_or(serde_json::Value::Null);
            summary["classification"] = witness
                .classification
                .as_ref()
                .map(|shell| serde_json::json!(shell.summary()))
                .or_else(|| witness.completed_classification.clone())
                .unwrap_or(serde_json::Value::Null);
            summary["classification_node_create"] = serde_json::json!(witness.node_create);
            summary["classification_entry"] = serde_json::json!(witness.classification_entry);
            summary["classification_return"] = serde_json::json!(witness.classification_return);
            summary["pre_node_classification_entry"] =
                serde_json::json!(witness.pre_node_classification_entry);
            summary["pre_node_classification_return"] =
                serde_json::json!(witness.pre_node_classification_return);
            summary["post_node_exception"] = serde_json::json!(witness.post_node_exception);
            // 仅身份、数值及三项环境匹配布尔；不含原始内存、路径、命令行或环境值。
            warp_core::safe_eprintln!(safe:("managed_process.windows_native_witness={summary}"),full:("managed_process.windows_native_witness={summary}"));
        }
    }
}

#[cfg(test)]
#[path = "managed_process_atomic_windows_witness_tests.rs"]
mod tests;
