//! 仅授权的首个 PowerShell 候选读取分类返回同停点 CLR 栈；不参与候选成功判定。

use std::io::Write as _;

use command::windows::{ClrNativeReturnStop, ClrReader, ClrReaderImage, ClrRuntimeBinding};

use super::native_snapshot::{Identity, identity};
use super::*;

const SINGLE_STEP: u32 = 0x80000004;
const READ_BUDGET_BYTES: u32 = 24 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
enum ReturnStage {
    PreNode,
    PostNode,
}

impl ReturnStage {
    fn file_prefix(self) -> &'static str {
        match self {
            Self::PreNode => "native-clr-pre-node-classification",
            Self::PostNode => "native-clr-classification",
        }
    }

    fn scope(self) -> &'static str {
        match self {
            Self::PreNode => "pre_node_classification_return_only",
            Self::PostNode => "post_start_classification_return_only",
        }
    }
}

#[derive(Debug, Default)]
struct ReturnBudget {
    pre_node: Option<u64>,
    post_node: Option<u64>,
}

impl ReturnBudget {
    fn last_sequence(&self) -> u64 {
        self.post_node.or(self.pre_node).unwrap_or(0)
    }

    fn reserve(&mut self, stage: ReturnStage, sequence: u64, expired: bool) -> io::Result<()> {
        let available = match stage {
            ReturnStage::PreNode => self.pre_node.is_none() && self.post_node.is_none(),
            ReturnStage::PostNode => self.post_node.is_none(),
        };
        if !available || sequence <= self.last_sequence() || expired {
            return Err(error("native_clr.event_sequence_or_deadline"));
        }
        // 读取失败也消耗本阶段唯一预算；不能借重试重复观察原停点。
        match stage {
            ReturnStage::PreNode => self.pre_node = Some(sequence),
            ReturnStage::PostNode => self.post_node = Some(sequence),
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ThreadBinding {
    handle: OwnedHandle,
    identity: Identity,
}

#[derive(Debug)]
pub(super) struct NativeClr {
    generation: Uuid,
    directory: PathBuf,
    image: ClrReaderImage,
    runtime: Option<(u64, ClrRuntimeBinding)>,
    runtime_binding_at_load: Option<serde_json::Value>,
    threads: HashMap<u32, ThreadBinding>,
    active_reader: Option<ClrReader>,
    budget: ReturnBudget,
    completed: u64,
    pre_node_completed: u64,
    phase: &'static str,
    identity_failure: Option<serde_json::Value>,
    reader_cleanup: Option<serde_json::Value>,
}

fn validate_return_binding(
    event: &DEBUG_EVENT,
    root_pid: u32,
    sequence: u64,
    classification: &serde_json::Value,
) -> io::Result<()> {
    if event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT || event.dwProcessId != root_pid {
        return Err(error("native_clr.return_event"));
    }
    let information = unsafe { event.u.Exception };
    let entry = classification["entry_sequence"].as_u64();
    let node = classification["node_create_sequence"].as_u64();
    if information.dwFirstChance != 1
        || information.ExceptionRecord.ExceptionCode.0 as u32 != SINGLE_STEP
        || classification["identity"]["process_id"] != root_pid
        || classification["identity"]["thread_id"] != event.dwThreadId
        || classification["return_sequence"] != sequence
        || !node
            .zip(entry)
            .is_some_and(|(node, entry)| node != 0 && node < entry && entry < sequence)
        || classification["node_process_id"]
            .as_u64()
            .is_none_or(|pid| pid == 0 || pid == u64::from(root_pid) || pid > u32::MAX as u64)
        || classification["node_process_birth"]
            .as_u64()
            .is_none_or(|birth| birth == 0)
        || classification["expected_node_matched"] != true
        || classification["flags"] != 0x2000
        || !classification["raw_return_u64"]
            .as_u64()
            .is_some_and(|value| {
                classification["raw_return_low32"].as_u64() == Some(u64::from(value as u32))
            })
        || classification["registers_restored"] != true
        || classification["execution_context_unchanged"] != true
        || classification["post_start_clr_stack_required"] != true
    {
        return Err(error("native_clr.return_classification_binding"));
    }
    Ok(())
}

fn validate_pre_node_return_binding(
    event: &DEBUG_EVENT,
    root_pid: u32,
    sequence: u64,
    generation: Uuid,
    classification: &serde_json::Value,
) -> io::Result<()> {
    if event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT || event.dwProcessId != root_pid {
        return Err(error("native_clr.pre_node_return_event"));
    }
    let information = unsafe { event.u.Exception };
    if root_pid == 0
        || event.dwThreadId == 0
        || information.dwFirstChance != 1
        || information.ExceptionRecord.ExceptionCode.0 as u32 != SINGLE_STEP
        || classification["generation"] != serde_json::json!(generation.as_bytes())
        || classification["identity"]["process_id"] != root_pid
        || classification["identity"]["thread_id"] != event.dwThreadId
        || classification["identity"]["process_birth"]
            .as_u64()
            .is_none_or(|value| value == 0)
        || classification["identity"]["thread_birth"]
            .as_u64()
            .is_none_or(|value| value == 0)
        || classification["entry_sequence"]
            .as_u64()
            .is_none_or(|entry| entry == 0 || entry >= sequence)
        || classification["return_sequence"] != sequence
        || classification.get("node_create_sequence").is_some()
        || classification.get("node_process_id").is_some()
        || classification.get("node_process_birth").is_some()
        || classification
            .get("post_start_clr_stack_required")
            .is_some()
        || classification["pre_node_clr_stack_required"] != true
        || classification["expected_node_matched"] != true
        || classification["flags"] != 0x2000
        || !classification["raw_return_u64"]
            .as_u64()
            .is_some_and(|value| {
                classification["raw_return_low32"].as_u64() == Some(u64::from(value as u32))
            })
        || classification["registers_restored"] != true
        || classification["execution_context_unchanged"] != true
    {
        return Err(error("native_clr.pre_node_return_classification_binding"));
    }
    Ok(())
}

fn write_receipt(path: &Path, receipt: &serde_json::Value) -> io::Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer(&mut output, receipt)?;
    output.write_all(b"\n")?;
    output.sync_all()
}

impl NativeClr {
    pub(super) fn bind(
        generation: Uuid,
        directory: &Path,
        binding: &super::super::NativeWitnessReaderBinding,
    ) -> io::Result<Self> {
        let hash = hex::decode(&binding.sha256)
            .ok()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or_else(|| error("native_clr.reader_digest"))?;
        Ok(Self {
            generation,
            directory: directory.to_path_buf(),
            image: ClrReaderImage::bind(&binding.path, hash)?,
            runtime: None,
            runtime_binding_at_load: None,
            threads: HashMap::new(),
            active_reader: None,
            budget: ReturnBudget::default(),
            completed: 0,
            pre_node_completed: 0,
            phase: "bound",
            identity_failure: None,
            reader_cleanup: None,
        })
    }

    pub(super) fn retain_thread(
        &mut self,
        process: &OwnedHandle,
        process_created: u64,
        process_id: u32,
        thread_id: u32,
        original: HANDLE,
    ) -> io::Result<()> {
        if self.threads.contains_key(&thread_id) {
            return Err(error("native_clr.duplicate_thread"));
        }
        // 只复制原 CREATE 句柄，不以可重用的 TID 重开线程。
        self.phase = "retain_thread";
        let handle = duplicate_process_handle(original)?;
        let (identity, _) = identity(
            HANDLE(process.as_raw_handle()),
            HANDLE(handle.as_raw_handle()),
            process_created,
        )
        .map_err(|failure| {
            self.identity_failure = Some(serde_json::json!(failure));
            error("native_clr.thread_identity")
        })?;
        if identity.process_id != process_id || identity.thread_id != thread_id {
            return Err(error("native_clr.thread_event_mismatch"));
        }
        self.threads
            .insert(thread_id, ThreadBinding { handle, identity });
        Ok(())
    }

    pub(super) fn bind_runtime(&mut self, file: &File, base: u64) -> io::Result<()> {
        let path = final_path_from_handle(file)?;
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("clr.dll"))
        {
            return Ok(());
        }
        if self.runtime.is_some() {
            return Err(error("native_clr.duplicate_runtime"));
        }
        // A 层核固定 Framework64 来源、原 LOAD 文件身份和配对 DAC；不借 unwind 租约放行。
        self.phase = "bind_runtime";
        let runtime = ClrRuntimeBinding::from_load(file, base)?;
        // 即使没有分类返回，也保留本代原 LOAD 身份；配对 DAC 不代表 reader 已加载它。
        self.runtime_binding_at_load = Some(runtime.receipt());
        self.runtime = Some((base, runtime));
        Ok(())
    }

    pub(super) fn unload(&mut self, base: u64) -> io::Result<()> {
        self.ensure_reaped()?;
        if self.runtime.as_ref().is_some_and(|(held, _)| *held == base) {
            self.runtime = None;
        }
        Ok(())
    }

    pub(super) fn thread_exited(&mut self, tid: u32) {
        // 调用点仅位于原 EXIT_THREAD 成功 Continue 之后。
        self.threads.remove(&tid);
    }

    pub(super) fn process_exited(&mut self) {
        self.threads.clear();
        self.runtime = None;
    }

    pub(super) fn ensure_reaped(&self) -> io::Result<()> {
        if self
            .active_reader
            .as_ref()
            .is_some_and(|reader| !reader.is_reaped())
        {
            return Err(error("native_clr.reader_not_reaped"));
        }
        Ok(())
    }

    pub(super) fn abort_after_target_termination(&mut self, deadline: Instant) -> io::Result<()> {
        if let Some(reader) = &mut self.active_reader {
            reader.abort_and_reap(deadline)?;
            self.reader_cleanup = Some(serde_json::json!({
                "event_sequence":self.budget.last_sequence(),"binding":reader.binding(),
                "reader_reaped":reader.is_reaped(),"target_termination_requested_first":true,
            }));
        }
        self.ensure_reaped()?;
        self.active_reader = None;
        Ok(())
    }

    pub(super) fn observe_native_return(
        &mut self,
        event: &DEBUG_EVENT,
        process: &OwnedHandle,
        process_created: u64,
        root_pid: u32,
        sequence: u64,
        classification: &serde_json::Value,
        deadline: Instant,
        cancellation: Option<&AtomicBool>,
    ) -> io::Result<()> {
        self.observe_return(
            ReturnStage::PostNode,
            event,
            process,
            process_created,
            root_pid,
            sequence,
            classification,
            deadline,
            cancellation,
        )
    }

    pub(super) fn observe_pre_node_return(
        &mut self,
        event: &DEBUG_EVENT,
        process: &OwnedHandle,
        process_created: u64,
        root_pid: u32,
        sequence: u64,
        classification: &serde_json::Value,
        deadline: Instant,
        cancellation: Option<&AtomicBool>,
    ) -> io::Result<()> {
        self.observe_return(
            ReturnStage::PreNode,
            event,
            process,
            process_created,
            root_pid,
            sequence,
            classification,
            deadline,
            cancellation,
        )
    }

    fn observe_return(
        &mut self,
        stage: ReturnStage,
        event: &DEBUG_EVENT,
        process: &OwnedHandle,
        process_created: u64,
        root_pid: u32,
        sequence: u64,
        classification: &serde_json::Value,
        deadline: Instant,
        cancellation: Option<&AtomicBool>,
    ) -> io::Result<()> {
        match stage {
            ReturnStage::PreNode => validate_pre_node_return_binding(
                event,
                root_pid,
                sequence,
                self.generation,
                classification,
            )?,
            ReturnStage::PostNode => {
                validate_return_binding(event, root_pid, sequence, classification)?;
            }
        }
        if classification["generation"] != serde_json::json!(self.generation.as_bytes()) {
            return Err(error("native_clr.return_generation"));
        }
        self.phase = "native_return_identity";
        self.ensure_reaped()?;
        if cancellation.is_some_and(|value| value.load(Ordering::Acquire)) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "native_clr.cancelled",
            ));
        }
        self.budget
            .reserve(stage, sequence, Instant::now() >= deadline)?;
        let prefix = stage.file_prefix();
        // 先保全原停点；后续身份、运行时或 reader 失败不能抹去本次事件。
        write_receipt(
            &self
                .directory
                .join(format!("{prefix}-{sequence}.pending.json")),
            &serde_json::json!({"generation":self.generation,"process_id":root_pid,
                "thread_id":event.dwThreadId,"event_sequence":sequence,"hresult":0,
                "classification":classification,"operation":3,"scope":stage.scope(),
                "phase":"pending_not_read","candidate_success_inferred":false}),
        )?;
        let thread = self
            .threads
            .get(&event.dwThreadId)
            .ok_or_else(|| error("native_clr.original_thread_missing"))?;
        let (current, _) = identity(
            HANDLE(process.as_raw_handle()),
            HANDLE(thread.handle.as_raw_handle()),
            process_created,
        )
        .map_err(|failure| {
            self.identity_failure = Some(serde_json::json!(failure));
            error("native_clr.thread_identity")
        })?;
        let current_identity = serde_json::to_value(current)?;
        if current != thread.identity
            || current.process_id != root_pid
            || classification["identity"]["process_birth"] != current_identity["process_created"]
            || classification["identity"]["thread_birth"] != current_identity["thread_created"]
        {
            return Err(error("native_clr.thread_identity_changed"));
        }
        let (_, runtime) = self
            .runtime
            .as_mut()
            .ok_or_else(|| error("native_clr.runtime_missing"))?;
        self.phase = "reader_start";
        // start 成功即存入所有者；后续绑定、发送、取消和解析失败均不能提前丢失 reader。
        self.active_reader = Some(ClrReader::start_native_return(
            &mut self.image,
            &self.directory,
            sequence,
            deadline,
        )?);
        let reader = self.active_reader.as_mut().unwrap();
        self.phase = "reader_bind_and_send";
        reader.bind_native_return_and_send(
            &mut self.image,
            runtime,
            ClrNativeReturnStop {
                process: process.as_handle(),
                thread: thread.handle.as_handle(),
                process_id: root_pid,
                thread_id: event.dwThreadId,
                event_sequence: sequence,
                exception_code: SINGLE_STEP,
                first_chance: 1,
                hresult: 0,
                read_budget_bytes: READ_BUDGET_BYTES,
            },
        )?;
        self.phase = "reader_poll";
        let result = loop {
            if let Some(result) =
                reader.poll(|| cancellation.is_some_and(|value| value.load(Ordering::Acquire)))?
            {
                break result;
            }
        };
        if !reader.is_reaped() {
            return Err(error("native_clr.reader_not_reaped"));
        }
        let receipt = serde_json::json!({
            "schema":1,"generation":self.generation,"mode":"powershell",
            "event_sequence":sequence,"process_id":root_pid,"thread_id":event.dwThreadId,
            "identity":current,"exception_code":SINGLE_STEP,"hresult":0,"operation":3,"scope":stage.scope(),
            "classification":classification,"live_threads_restored_before_reader":true,
            "binding":reader.binding(),"reader":self.image.receipt(),"runtime":runtime.receipt(),
            "result":result,"reader_reaped":true,"reader_job_empty":true,"candidate_success_inferred":false,
        });
        self.phase = "write_receipt";
        write_receipt(
            &self.directory.join(format!("{prefix}-{sequence}.json")),
            &receipt,
        )?;
        // 合法 observed/partial/unavailable 均已绑定且 reader 已退出；后两者不是读取通过。
        self.active_reader = None;
        match stage {
            ReturnStage::PreNode => self.pre_node_completed += 1,
            ReturnStage::PostNode => self.completed += 1,
        }
        self.phase = "event_recorded";
        Ok(())
    }

    pub(super) fn summary(&self) -> serde_json::Value {
        serde_json::json!({"recorded_events":self.completed,"last_sequence":self.budget.post_node.unwrap_or(0),
            "operation":3,"scope":"post_start_classification_return_only",
            "pre_node":{"recorded_events":self.pre_node_completed,
                "last_sequence":self.budget.pre_node.unwrap_or(0),
                "scope":"pre_node_classification_return_only","candidate_success_inferred":false},
            "total_recorded_events":self.completed + self.pre_node_completed,
            "last_reader_sequence":self.budget.last_sequence(),
            "phase":self.phase,"identity_failure":self.identity_failure,"reader_cleanup":self.reader_cleanup,
            "runtime_binding_at_load":self.runtime_binding_at_load,
            "runtime_still_held":self.runtime.is_some(),
            "active_reader":self.active_reader.as_ref().map(|reader| serde_json::json!({
                "reaped":reader.is_reaped(),"binding":reader.binding()}))})
    }
}

#[cfg(test)]
#[path = "managed_process_atomic_windows_clr_tests.rs"]
mod tests;
