//! 仅一次、精确 generation 的 npm CMD 因果取证；不参与普通映像授权。

use std::os::windows::io::BorrowedHandle;

use windows::Win32::System::Threading::{PROCESS_NAME_WIN32, QueryFullProcessImageNameW};
use windows::core::PWSTR;

use super::creation_witness::{CreationWitness, Disposition, Summary, VerifiedImage};
use super::native_snapshot::{VerifiedModule, capture, prepare_modules};
use super::*;

const SNAPSHOT_AFTER: Duration = Duration::from_secs(15);
const MAX_WITNESS_MODULES: usize = 64;
const MAX_WITNESS_THREADS: usize = 16;
const GENERATION_ENV: &str = "INFINISHELL_WINDOWS_NATIVE_WITNESS_GENERATION";

#[derive(Debug)]
struct BoundModule {
    lease: SystemHelperLease,
    base: u64,
    size: u32,
    dll: bool,
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
    modules: HashMap<u32, Vec<BoundModule>>,
    births: HashMap<u32, u64>,
    creation: Option<CreationWitness>,
    completed_creation: Option<Summary>,
    creation_unavailable: Option<serde_json::Value>,
    snapshot: Option<serde_json::Value>,
    failures: Vec<serde_json::Value>,
    cancelled: bool,
    exit_confirmed: bool,
}

fn enabled(generation: Uuid, mode: &str, value: Option<&str>) -> bool {
    mode == "cmd" && value.is_some_and(|value| value == generation.to_string())
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

fn mapped_image_size(file: &File) -> io::Result<u32> {
    let mut file = file.try_clone()?;
    let file_size = inspect_handle(&file)?.size;
    let offset = u64::from(read_u32(&mut file, DOS_HEADER_PE_OFFSET, file_size)?);
    if offset > MAX_PE_HEADER_OFFSET {
        return Err(error("native_witness.pe_offset"));
    }
    let header = read_at::<24>(&mut file, offset, file_size)?;
    if &header[..4] != b"PE\0\0" || u16::from_le_bytes([header[4], header[5]]) != 0x8664 {
        return Err(error("native_witness.pe_machine"));
    }
    let optional = offset
        .checked_add(24)
        .ok_or_else(|| error("native_witness.pe_offset"))?;
    let header = read_at::<60>(&mut file, optional, file_size)?;
    if u16::from_le_bytes([header[0], header[1]]) != 0x20b {
        return Err(error("native_witness.pe_optional"));
    }
    let size = u32::from_le_bytes(header[56..60].try_into().unwrap());
    if size == 0 || u64::from(size) > MAX_NATIVE_EXECUTABLE_BYTES {
        return Err(error("native_witness.pe_image_size"));
    }
    Ok(size)
}

impl WindowsImageDebugSession {
    pub(in super::super) fn bind_native_witness(&mut self, generation: Uuid, mode: &str) {
        let value = std::env::var(GENERATION_ENV).ok();
        if enabled(generation, mode, value.as_deref())
            && self.package_images.is_some()
            && self.npm_diagnostics.is_some()
        {
            self.native_witness = Some(NativeWitness {
                generation,
                modules: HashMap::new(),
                births: HashMap::new(),
                creation: None,
                completed_creation: None,
                creation_unavailable: None,
                snapshot: None,
                failures: Vec::new(),
                cancelled: false,
                exit_confirmed: false,
            });
        }
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
        // 仅在原 CREATE/LOAD 授权通过后取得独立拒写租约；路径只找回原 file identity。
        let prepared = (|| {
            let lease = lease_mapped_image(file, &final_path_from_handle(file)?, dll, false)?;
            let size = mapped_image_size(&lease.program)?;
            Ok::<_, io::Error>((lease, size))
        })();
        let (lease, size) = prepared.map_err(|failure| {
            self.native_failure("module_binding", safe_failure("module_binding", &failure))
        })?;
        let modules = self
            .native_witness
            .as_mut()
            .unwrap()
            .modules
            .entry(pid)
            .or_default();
        if modules.len() >= MAX_WITNESS_MODULES || modules.iter().any(|module| module.base == base)
        {
            return Err(error(
                "managed_process.native_witness_module_limit_or_duplicate",
            ));
        }
        modules.push(BoundModule {
            lease,
            base,
            size,
            dll,
        });
        Ok(())
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
        let witness = self.native_witness.as_mut().unwrap();
        witness.births.insert(event.dwProcessId, birth);
        if role == NpmProcessRole::Root {
            if witness.creation.is_some() {
                return Err(error("native_witness.duplicate_root"));
            }
            let root = witness.modules[&event.dwProcessId][0].creation_image();
            match CreationWitness::new(event, root, birth, at_ms) {
                Ok(creation) => witness.creation = Some(creation),
                Err(failure) if failure.coverage_unavailable() => {
                    // new 从未写线程寄存器；只保留覆盖未知，仍使用原映像/Job 的快照边界。
                    witness.creation_unavailable = Some(
                        serde_json::json!({"stage":"new","failure":failure,"restoration":"NotModified"}),
                    );
                }
                Err(failure) => return Err(self.native_failure("creation_bind", failure)),
            }
        }
        Ok(())
    }

    pub(super) fn native_witness_unload(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        let at_ms = self
            .npm_diagnostics
            .as_ref()
            .map(|value| value.started.elapsed().as_millis())
            .unwrap_or(0);
        let Some(witness) = &mut self.native_witness else {
            return Ok(());
        };
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
        }
    }

    pub(super) fn native_witness_exception(
        &mut self,
        event: &DEBUG_EVENT,
    ) -> io::Result<Option<windows::Win32::Foundation::NTSTATUS>> {
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

    fn capture_native_witness(&mut self, container: &AppContainerProbe) -> io::Result<()> {
        if self.pending_event.is_some() {
            return Err(error("native_witness.pending_debug_event"));
        }
        let started_ms = self.native_at_ms();
        // 先占用唯一采样机会；失败也不能重采并替换原因。
        self.native_witness.as_mut().unwrap().snapshot =
            Some(serde_json::json!({"started_ms":started_ms,"complete":false}));
        let desktop = container.native_witness_desktop();
        let desktop_failure = desktop
            .as_ref()
            .err()
            .map(|failure| safe_failure("expected_desktop", failure));
        let expected_desktop = desktop.ok().flatten();
        let mut observations = Vec::new();
        let mut balanced = true;
        let mut thread_count = 0;
        match container.native_witness_processes() {
            Err(failure) => observations.push(safe_failure("job_members", &failure)),
            Ok(processes) => {
                for (pid, process) in processes {
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
                                let snapshot = capture(
                                    process.process(),
                                    process.created_filetime(),
                                    thread.as_handle(),
                                    expected_desktop,
                                    &prepared,
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
        witness.snapshot = Some(
            serde_json::json!({"started_ms":started_ms,"finished_ms":finished_ms,"complete":true,"suspension_balanced":balanced,"desktop_failure":desktop_failure,"processes":observations}),
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

    pub(super) fn wait_for_native_witness_event(
        &mut self,
        deadline: Instant,
        timeout_message: &'static str,
        container: &AppContainerProbe,
    ) -> io::Result<DEBUG_EVENT> {
        loop {
            if self
                .cancellation
                .as_ref()
                .is_some_and(|value| value.load(Ordering::Acquire))
            {
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
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
            }
            if self
                .native_witness
                .as_ref()
                .is_some_and(|value| value.snapshot.is_none())
                && self
                    .npm_diagnostics
                    .as_ref()
                    .is_some_and(|value| value.started.elapsed() >= SNAPSHOT_AFTER)
                && self.root_process_id != 0
                && self.native_witness.as_ref().is_some_and(|value| {
                    value.births.contains_key(&self.root_process_id)
                        && value.modules.contains_key(&self.root_process_id)
                })
            {
                self.capture_native_witness(container)?;
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
            match wait_for_debug_event(interval.as_millis().max(1) as u32, diagnostics) {
                Ok(event) => return Ok(event),
                Err(failure)
                    if failure
                        .get_ref()
                        .and_then(|value| value.downcast_ref::<WindowsError>())
                        .is_some_and(|value| {
                            value.code() == HRESULT::from_win32(ERROR_SEM_TIMEOUT.0)
                        }) => {}
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
            witness.exit_confirmed = true;
        }
        Ok(())
    }

    pub(in super::super) fn record_native_witness_result(&self) {
        if let Some(witness) = &self.native_witness {
            let summary = serde_json::json!({"generation":witness.generation,"snapshot":witness.snapshot,"creation":witness.creation.as_ref().map(CreationWitness::summary).or(witness.completed_creation.as_ref()),"creation_unavailable":witness.creation_unavailable,"failures":witness.failures,"cancelled":witness.cancelled,"exit_confirmed":witness.exit_confirmed,"requires_original_exit":witness.creation.as_ref().is_some_and(CreationWitness::requires_restore_or_original_exit)});
            // 仅地址、摘要、对象身份和固定错误类别；不含内存字节、路径、命令行或环境。
            warp_core::safe_eprintln!(safe:("managed_process.windows_native_witness={summary}"),full:("managed_process.windows_native_witness={summary}"));
        }
    }
}

#[cfg(test)]
#[path = "managed_process_atomic_windows_witness_tests.rs"]
mod tests;
