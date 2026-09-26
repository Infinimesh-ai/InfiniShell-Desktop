//! 本机 CLI 运行时宿主；稳定重关联必须由宿主持有协议状态，不能重放旧输入。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use command::Stdio;
use command::r#async::{Child, Command};
use ipc::{Client, ConnectionAddress, Service, ServiceImpl};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warpui::r#async::Timer;
use warpui::r#async::executor::Background;

use super::local_tools::NativeLocalToolRequest;
use super::{
    RuntimeCommand, RuntimeController, RuntimeError, RuntimeEvent, RuntimeEventKind,
    SessionOptions, TurnOutcome,
};

const HOST_MANIFEST_VERSION: u32 = 2;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
// 协调器允许最多 10 MiB 原始输出；JSON 控制字符最坏会扩大到约六倍，因此单条记录
// 与总账本使用同一硬上限，由总字节数继续约束整个 generation。
const MAX_JOURNAL_RECORD_BYTES: usize = MAX_JOURNAL_BYTES as usize;
const MAX_JOURNAL_EVENTS: u64 = 16_384;
const MAX_PULL_EVENTS: usize = 256;
const MAX_IPC_JSON_BYTES: usize = MAX_JOURNAL_BYTES as usize;
const RUNTIME_HOST_FRAME_ENVELOPE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RUNTIME_HOST_FRAME_BYTES: usize = MAX_IPC_JSON_BYTES + RUNTIME_HOST_FRAME_ENVELOPE_BYTES;
const MAX_HOST_RECORD_BYTES: u64 = 4 * 1024 * 1024;
const RUNTIME_HOST_CALL_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_HOST_EXIT_TIMEOUT: Duration = Duration::from_secs(10);
// 25 秒覆盖托管监督者的握手、宽限退出和资源域强制清理窗口，并留出收据落盘余量。
const RUNTIME_HOST_NATIVE_CLEANUP_ATTEMPTS: usize = 1_000;
const RUNTIME_HOST_POLL_INTERVAL: Duration = Duration::from_millis(25);
const HOST_WORKER_COMMAND: &str = "cli-agent-supervisor";
const STARTUP_EXIT_RECORD: &str = "startup-exit.json";
const STARTUP_INCOMPLETE_RECORD: &str = "startup-incomplete.json";
pub(crate) const COMMAND_NOT_DELIVERED_CODE: &str = "runtime_host.command_not_delivered";

mod json_wire {
    use std::io::{self, Write as _};

    use serde::{Deserialize as _, Deserializer, Serialize, Serializer, de::Error as _};

    struct LimitedJsonBuffer {
        bytes: Vec<u8>,
    }

    impl LimitedJsonBuffer {
        fn new() -> Self {
            Self { bytes: Vec::new() }
        }
    }

    impl io::Write for LimitedJsonBuffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > super::MAX_IPC_JSON_BYTES.saturating_sub(self.bytes.len()) {
                return Err(io::Error::other("runtime_host.ipc_json_payload_too_large"));
            }
            self.bytes
                .try_reserve_exact(bytes.len())
                .map_err(io::Error::other)?;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub(super) fn serialize<T: Serialize, S: Serializer>(
        value: &T,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&to_vec(value).map_err(serde::ser::Error::custom)?)
    }

    pub(super) fn to_vec<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
        let mut buffer = LimitedJsonBuffer::new();
        serde_json::to_writer(&mut buffer, value).map_err(io::Error::other)?;
        buffer.flush()?;
        Ok(buffer.bytes)
    }

    pub(super) fn deserialize<'de, T: serde::de::DeserializeOwned, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<T, D::Error> {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        if bytes.len() > super::MAX_IPC_JSON_BYTES {
            return Err(D::Error::custom("runtime_host.ipc_json_payload_too_large"));
        }
        serde_json::from_slice(&bytes).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeHostManifest {
    version: u32,
    task_id: String,
    task_generation: i64,
    harness: Harness,
    runtime_generation: Uuid,
    host_instance_id: Uuid,
    token: Uuid,
    endpoint: String,
    options: SessionOptions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeHostReady {
    version: u32,
    runtime_generation: Uuid,
    host_instance_id: Uuid,
    manifest_sha256: String,
    process_id: u32,
    process_start_time: u64,
    executable: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeHostExitReceipt {
    pub version: u32,
    pub runtime_generation: Uuid,
    pub host_instance_id: Uuid,
    pub manifest_sha256: String,
    pub journal_sha256: String,
    pub last_event_sequence: u64,
    pub acknowledged_sequence: u64,
    pub native_process: NativeProcessCompletion,
    pub native_cleanup_sha256: Option<String>,
    pub adapter_task_terminated: bool,
    pub adapter_succeeded: bool,
    pub event_journal_completed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeProcessCompletion {
    NotStarted,
    Exited,
    Unconfirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuntimeHostStartupFailurePhase {
    Manifest,
    Executable,
    Spawn,
    ReadyHandshake,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeHostStartupExitReceipt {
    version: u32,
    task_id: String,
    task_generation: i64,
    runtime_generation: Uuid,
    host_instance_id: Uuid,
    manifest_sha256: Option<String>,
    phase: RuntimeHostStartupFailurePhase,
    host_process_id: Option<u32>,
    host_exit_success: Option<bool>,
    host_exit_code: Option<i32>,
    native_process: NativeProcessCompletion,
    native_cleanup_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeHostStartupIncomplete {
    version: u32,
    task_id: String,
    task_generation: i64,
    runtime_generation: Uuid,
    host_instance_id: Uuid,
    manifest_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObservedHostExit {
    process_id: u32,
    success: bool,
    exit_code: Option<i32>,
}

struct SpawnedHostGuard {
    process: Option<Child>,
    record: RuntimeHostRecord,
}

impl SpawnedHostGuard {
    fn new(process: Child, record: RuntimeHostRecord) -> Self {
        Self {
            process: Some(process),
            record,
        }
    }

    fn process_mut(&mut self) -> &mut Child {
        self.process.as_mut().expect("宿主进程句柄必须存在")
    }

    fn release(mut self) -> io::Result<()> {
        remove_startup_incomplete_marker(&self.record)?;
        drop(self.process.take());
        Ok(())
    }
}

impl Drop for SpawnedHostGuard {
    fn drop(&mut self) {
        let Some(process) = self.process.take() else {
            return;
        };
        // Drop 无法 await；把进程句柄移交给独立监视线程，使 spawn future
        // 取消后仍能等待宿主退出、核对真实 CLI 清理并持久化收据。
        // 线程创建或收敛失败时不清除未确认标记，恢复入口仍会 fail-closed。
        let _ = spawn_cancelled_startup_monitor(self.record.clone(), process);
    }
}

fn spawn_cancelled_startup_monitor(
    record: RuntimeHostRecord,
    mut process: Child,
) -> io::Result<std::thread::JoinHandle<io::Result<()>>> {
    // kill 只是请求；即使返回成功，也必须由监视线程观察实际退出并核对
    // managed-process 回执后才能清除未确认标记。
    let _ = process.kill();
    std::thread::Builder::new()
        .name("cli-runtime-startup-cleanup".to_owned())
        .spawn(move || {
            warpui_core::r#async::block_on(async {
                converge_spawned_startup_failure(&record, &mut process).await?;
                remove_startup_incomplete_marker(&record)
            })
        })
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeHostRecord {
    manifest: RuntimeHostManifest,
    manifest_sha256: String,
    directory: PathBuf,
}

impl RuntimeHostRecord {
    pub(crate) fn runtime_generation(&self) -> Uuid {
        self.manifest.runtime_generation
    }

    pub(crate) fn task_id(&self) -> &str {
        &self.manifest.task_id
    }

    pub(crate) fn task_generation(&self) -> i64 {
        self.manifest.task_generation
    }

    pub(crate) fn host_instance_id(&self) -> Uuid {
        self.manifest.host_instance_id
    }

    pub(crate) fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub(crate) fn harness(&self) -> Harness {
        self.manifest.harness
    }

    pub(crate) fn options(&self) -> &SessionOptions {
        &self.manifest.options
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostCommandDisposition {
    Recorded,
    DeliveredToAdapter,
    NativeAccepted,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HostCommandStatus {
    pub message_id: Uuid,
    pub digest: String,
    pub disposition: HostCommandDisposition,
    pub turn_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct HostedEvent {
    pub sequence: u64,
    pub digest: String,
    pub event: RuntimeEvent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct HostedApproval {
    pub turn_id: String,
    pub method: String,
    pub details: serde_json::Value,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct RuntimeHostSnapshot {
    pub ready: bool,
    pub connected: bool,
    pub native_session_id: Option<String>,
    pub active_turn_id: Option<String>,
    pub approvals: BTreeMap<String, HostedApproval>,
    pub local_tools: BTreeMap<String, NativeLocalToolRequest>,
    /// 已由原生 CLI 接收、但尚未开始或并入现有回合的 turn。
    pub accepted_turn_ids: BTreeSet<String>,
    /// 已结束的 turn；重关联后用于拒绝迟到或重复事件。
    pub finished_turn_ids: BTreeSet<String>,
    pub output: String,
    pub last_error: Option<String>,
}

impl RuntimeHostSnapshot {
    fn apply(&mut self, event: &RuntimeEvent) {
        if event.native_session_id.is_some() {
            self.native_session_id.clone_from(&event.native_session_id);
        }
        match &event.kind {
            RuntimeEventKind::SessionReady { .. } => {
                self.ready = true;
                self.connected = true;
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } => {
                self.approvals.insert(
                    approval_id.clone(),
                    HostedApproval {
                        turn_id: turn_id.clone(),
                        method: method.clone(),
                        details: details.clone(),
                    },
                );
            }
            RuntimeEventKind::ApprovalResolved { approval_id, .. }
            | RuntimeEventKind::ApprovalCancelled { approval_id } => {
                self.approvals.remove(approval_id);
            }
            RuntimeEventKind::LocalToolRequested { request } => {
                self.local_tools
                    .insert(request.call_id.clone(), request.clone());
            }
            RuntimeEventKind::LocalToolCancelled { call_id, .. } => {
                self.local_tools.remove(call_id);
            }
            RuntimeEventKind::MessageAccepted {
                turn_id: Some(turn_id),
                ..
            } => {
                self.accepted_turn_ids.insert(turn_id.clone());
            }
            RuntimeEventKind::TurnStarted { turn_id } => {
                self.accepted_turn_ids.remove(turn_id);
                self.active_turn_id = Some(turn_id.clone());
            }
            RuntimeEventKind::InputJoined { message_id, .. } => {
                self.accepted_turn_ids.remove(&message_id.to_string());
            }
            RuntimeEventKind::TextDelta { text, .. } => self.output.push_str(text),
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } => {
                if self.active_turn_id.as_ref() == Some(turn_id) {
                    self.active_turn_id = None;
                }
                self.output.clone_from(output);
                self.approvals.clear();
                self.local_tools.clear();
                self.accepted_turn_ids.remove(turn_id);
                self.finished_turn_ids.insert(turn_id.clone());
                self.last_error = match outcome {
                    TurnOutcome::Failed { message } => Some(message.clone()),
                    TurnOutcome::Completed | TurnOutcome::Cancelled => None,
                };
            }
            RuntimeEventKind::RequestFailed {
                message_id,
                message,
            } => {
                self.accepted_turn_ids.remove(&message_id.to_string());
                self.last_error = Some(message.clone());
            }
            RuntimeEventKind::Disconnected { reason } => {
                self.connected = false;
                self.ready = false;
                self.approvals.clear();
                self.local_tools.clear();
                self.last_error = Some(reason.clone());
            }
            RuntimeEventKind::MessageAccepted { turn_id: None, .. }
            | RuntimeEventKind::CommandDispatched { .. }
            | RuntimeEventKind::Progress { .. } => {}
        }
    }

    pub(crate) fn apply_recovered_event(&mut self, event: &RuntimeEvent) {
        self.apply(event);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum JournalPayload {
    OwnerClaimed {
        owner_epoch: u64,
        client_id: Uuid,
    },
    CommandRecorded {
        status: HostCommandStatus,
    },
    CommandUpdated {
        status: HostCommandStatus,
    },
    Event {
        event: HostedEvent,
    },
    EventsAcknowledged {
        owner_epoch: u64,
        through_sequence: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct JournalRecord {
    sequence: u64,
    previous_sha256: String,
    payload: JournalPayload,
}

struct RuntimeHostState {
    manifest: RuntimeHostManifest,
    owner_epoch: u64,
    owner: Option<Uuid>,
    acknowledged_sequence: u64,
    next_journal_sequence: u64,
    journal_bytes: u64,
    journal_sha256: String,
    journal: File,
    journal_sealed: bool,
    events: Vec<HostedEvent>,
    commands: HashMap<Uuid, HostCommandStatus>,
    snapshot: RuntimeHostSnapshot,
    #[cfg(test)]
    append_failure_countdown: Option<usize>,
}

impl RuntimeHostState {
    fn new(
        directory: &Path,
        manifest: RuntimeHostManifest,
        manifest_sha256: String,
    ) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let journal = options.open(directory.join("runtime.journal"))?;
        Ok(Self {
            manifest,
            owner_epoch: 0,
            owner: None,
            acknowledged_sequence: 0,
            next_journal_sequence: 1,
            journal_bytes: 0,
            journal_sha256: manifest_sha256,
            journal,
            journal_sealed: false,
            events: Vec::new(),
            commands: HashMap::new(),
            snapshot: RuntimeHostSnapshot::default(),
            #[cfg(test)]
            append_failure_countdown: None,
        })
    }

    fn authenticate(&self, token: Uuid, task_id: &str, generation: Uuid) -> bool {
        token == self.manifest.token
            && task_id == self.manifest.task_id
            && generation == self.manifest.runtime_generation
    }

    fn owns(&self, owner_epoch: u64, client_id: Uuid) -> bool {
        self.owner_epoch == owner_epoch && self.owner == Some(client_id)
    }

    fn append(&mut self, payload: JournalPayload) -> io::Result<()> {
        if self.journal_sealed {
            return Err(io::Error::other("运行时宿主账本已封存"));
        }
        #[cfg(test)]
        if let Some(remaining) = self.append_failure_countdown.as_mut() {
            if *remaining == 0 {
                self.append_failure_countdown = None;
                return Err(io::Error::other("测试注入的运行时宿主账本失败"));
            }
            *remaining -= 1;
        }
        let record = JournalRecord {
            sequence: self.next_journal_sequence,
            previous_sha256: self.journal_sha256.clone(),
            payload,
        };
        let bytes = json_wire::to_vec(&record)?;
        if bytes.len() > MAX_JOURNAL_RECORD_BYTES
            || self.journal_bytes.saturating_add(bytes.len() as u64 + 1) > MAX_JOURNAL_BYTES
        {
            return Err(io::Error::other("运行时宿主账本超过安全上限"));
        }
        self.journal.write_all(&bytes)?;
        self.journal.write_all(b"\n")?;
        self.journal.sync_data()?;
        self.next_journal_sequence = self
            .next_journal_sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("运行时宿主账本序号耗尽"))?;
        self.journal_bytes += bytes.len() as u64 + 1;
        self.journal_sha256 = format!("{:x}", Sha256::digest(&bytes));
        Ok(())
    }

    fn record_event(&mut self, event: RuntimeEvent) -> io::Result<()> {
        let sequence = self
            .events
            .last()
            .map_or(1, |event| event.sequence.saturating_add(1));
        if sequence > MAX_JOURNAL_EVENTS {
            return Err(io::Error::other("运行时宿主事件数量超过安全上限"));
        }
        let digest = format!("{:x}", Sha256::digest(json_wire::to_vec(&event)?));
        let hosted = HostedEvent {
            sequence,
            digest,
            event,
        };
        self.append(JournalPayload::Event {
            event: hosted.clone(),
        })?;
        // Event 已经落盘后必须立即进入可拉取集合；即使随后命令状态账本失败，
        // 也不能遗失可向应用说明失败/已接收事实的唯一原生事件或复用事件序号。
        self.snapshot.apply(&hosted.event);
        self.events.push(hosted.clone());
        match &hosted.event.kind {
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } => self.update_command(
                *message_id,
                HostCommandDisposition::NativeAccepted,
                turn_id.clone(),
            )?,
            RuntimeEventKind::RequestFailed { message_id, .. } => {
                self.update_command(*message_id, HostCommandDisposition::Failed, None)?
            }
            RuntimeEventKind::SessionReady { .. }
            | RuntimeEventKind::CommandDispatched { .. }
            | RuntimeEventKind::TurnStarted { .. }
            | RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::TextDelta { .. }
            | RuntimeEventKind::Progress { .. }
            | RuntimeEventKind::ApprovalRequested { .. }
            | RuntimeEventKind::ApprovalResolved { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::LocalToolRequested { .. }
            | RuntimeEventKind::TurnFinished { .. }
            | RuntimeEventKind::Disconnected { .. } => {}
        }
        Ok(())
    }

    fn acknowledged_snapshot(&self) -> RuntimeHostSnapshot {
        let mut snapshot = RuntimeHostSnapshot::default();
        for event in self
            .events
            .iter()
            .take_while(|event| event.sequence <= self.acknowledged_sequence)
        {
            snapshot.apply(&event.event);
        }
        snapshot
    }

    fn update_command(
        &mut self,
        message_id: Uuid,
        disposition: HostCommandDisposition,
        turn_id: Option<String>,
    ) -> io::Result<()> {
        let Some(mut status) = self.commands.get(&message_id).cloned() else {
            return Ok(());
        };
        status.disposition = disposition;
        status.turn_id = turn_id;
        self.append(JournalPayload::CommandUpdated {
            status: status.clone(),
        })?;
        self.commands.insert(message_id, status);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum RuntimeHostRequest {
    Inspect {
        token: Uuid,
        task_id: String,
        runtime_generation: Uuid,
    },
    ClaimOwner {
        token: Uuid,
        task_id: String,
        runtime_generation: Uuid,
        observed_epoch: u64,
        client_id: Uuid,
    },
    Command {
        token: Uuid,
        task_id: String,
        runtime_generation: Uuid,
        owner_epoch: u64,
        client_id: Uuid,
        #[serde(with = "json_wire")]
        command: RuntimeCommand,
    },
    PullEvents {
        token: Uuid,
        task_id: String,
        runtime_generation: Uuid,
        owner_epoch: u64,
        client_id: Uuid,
        after_sequence: u64,
    },
    AckEvents {
        token: Uuid,
        task_id: String,
        runtime_generation: Uuid,
        owner_epoch: u64,
        client_id: Uuid,
        through_sequence: u64,
    },
    InspectCommands {
        token: Uuid,
        task_id: String,
        runtime_generation: Uuid,
        owner_epoch: u64,
        client_id: Uuid,
        message_ids: Vec<Uuid>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum RuntimeHostResponse {
    Inspection {
        host_instance_id: Uuid,
        owner_epoch: u64,
        last_event_sequence: u64,
        acknowledged_sequence: u64,
        #[serde(with = "json_wire")]
        snapshot: RuntimeHostSnapshot,
    },
    OwnerClaimed {
        owner_epoch: u64,
        last_event_sequence: u64,
        acknowledged_sequence: u64,
        #[serde(with = "json_wire")]
        snapshot: RuntimeHostSnapshot,
    },
    CommandStatus(HostCommandStatus),
    Events(#[serde(with = "json_wire")] Vec<HostedEvent>),
    EventsAcknowledged {
        through_sequence: u64,
    },
    CommandStatuses(Vec<HostCommandStatus>),
    Rejected {
        code: RuntimeHostRejection,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeHostRejection {
    StateUnavailable,
    IdentityMismatch,
    StaleOwner,
    OwnerEpochChanged,
    OwnerEpochExhausted,
    OwnerJournalFailed,
    StaleCommandGeneration,
    CommandEncodingFailed,
    CommandIdentityChanged,
    CommandJournalFailed,
    InvalidEventAck,
    EventAckJournalFailed,
}

pub(crate) struct RuntimeHostService;

impl Service for RuntimeHostService {
    type Request = RuntimeHostRequest;
    type Response = RuntimeHostResponse;
}

#[derive(Clone)]
struct RuntimeHostServiceImpl {
    state: Arc<Mutex<RuntimeHostState>>,
    controller: RuntimeController,
}

impl RuntimeHostServiceImpl {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, RuntimeHostState>, RuntimeHostResponse> {
        self.state
            .lock()
            .map_err(|_| RuntimeHostResponse::Rejected {
                code: RuntimeHostRejection::StateUnavailable,
            })
    }

    fn authenticate(
        state: &RuntimeHostState,
        token: Uuid,
        task_id: &str,
        generation: Uuid,
    ) -> Result<(), RuntimeHostResponse> {
        if state.authenticate(token, task_id, generation) {
            Ok(())
        } else {
            Err(RuntimeHostResponse::Rejected {
                code: RuntimeHostRejection::IdentityMismatch,
            })
        }
    }

    fn authorize_owner(
        state: &RuntimeHostState,
        token: Uuid,
        task_id: &str,
        generation: Uuid,
        owner_epoch: u64,
        client_id: Uuid,
    ) -> Result<(), RuntimeHostResponse> {
        Self::authenticate(state, token, task_id, generation)?;
        if state.owns(owner_epoch, client_id) {
            Ok(())
        } else {
            Err(RuntimeHostResponse::Rejected {
                code: RuntimeHostRejection::StaleOwner,
            })
        }
    }
}

#[async_trait]
impl ServiceImpl for RuntimeHostServiceImpl {
    type Service = RuntimeHostService;

    async fn handle_request(&self, request: RuntimeHostRequest) -> RuntimeHostResponse {
        match request {
            RuntimeHostRequest::Inspect {
                token,
                task_id,
                runtime_generation,
            } => {
                let state = match self.lock() {
                    Ok(state) => state,
                    Err(response) => return response,
                };
                if let Err(response) =
                    Self::authenticate(&state, token, &task_id, runtime_generation)
                {
                    return response;
                }
                RuntimeHostResponse::Inspection {
                    host_instance_id: state.manifest.host_instance_id,
                    owner_epoch: state.owner_epoch,
                    last_event_sequence: state.events.last().map_or(0, |event| event.sequence),
                    acknowledged_sequence: state.acknowledged_sequence,
                    snapshot: state.acknowledged_snapshot(),
                }
            }
            RuntimeHostRequest::ClaimOwner {
                token,
                task_id,
                runtime_generation,
                observed_epoch,
                client_id,
            } => {
                let mut state = match self.lock() {
                    Ok(state) => state,
                    Err(response) => return response,
                };
                if let Err(response) =
                    Self::authenticate(&state, token, &task_id, runtime_generation)
                {
                    return response;
                }
                if observed_epoch != state.owner_epoch {
                    return RuntimeHostResponse::Rejected {
                        code: RuntimeHostRejection::OwnerEpochChanged,
                    };
                }
                let Some(owner_epoch) = state.owner_epoch.checked_add(1) else {
                    return RuntimeHostResponse::Rejected {
                        code: RuntimeHostRejection::OwnerEpochExhausted,
                    };
                };
                if state
                    .append(JournalPayload::OwnerClaimed {
                        owner_epoch,
                        client_id,
                    })
                    .is_err()
                {
                    return RuntimeHostResponse::Rejected {
                        code: RuntimeHostRejection::OwnerJournalFailed,
                    };
                }
                state.owner_epoch = owner_epoch;
                state.owner = Some(client_id);
                RuntimeHostResponse::OwnerClaimed {
                    owner_epoch,
                    last_event_sequence: state.events.last().map_or(0, |event| event.sequence),
                    acknowledged_sequence: state.acknowledged_sequence,
                    snapshot: state.acknowledged_snapshot(),
                }
            }
            RuntimeHostRequest::Command {
                token,
                task_id,
                runtime_generation,
                owner_epoch,
                client_id,
                command,
            } => {
                let bytes = match serde_json::to_vec(&command) {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        return RuntimeHostResponse::Rejected {
                            code: RuntimeHostRejection::CommandEncodingFailed,
                        };
                    }
                };
                let digest = format!("{:x}", Sha256::digest(bytes));
                {
                    let state = match self.lock() {
                        Ok(state) => state,
                        Err(response) => return response,
                    };
                    if let Err(response) = Self::authorize_owner(
                        &state,
                        token,
                        &task_id,
                        runtime_generation,
                        owner_epoch,
                        client_id,
                    ) {
                        return response;
                    }
                    if command.generation != state.manifest.runtime_generation {
                        return RuntimeHostResponse::Rejected {
                            code: RuntimeHostRejection::StaleCommandGeneration,
                        };
                    }
                    if let Some(previous) = state.commands.get(&command.message_id) {
                        return if previous.digest == digest {
                            RuntimeHostResponse::CommandStatus(previous.clone())
                        } else {
                            RuntimeHostResponse::Rejected {
                                code: RuntimeHostRejection::CommandIdentityChanged,
                            }
                        };
                    }
                }

                // 先取得有界队列容量，再做最终 owner 校验和落盘。这样等待期间发生 claim 时，
                // 旧 owner 既不会留下 CommandRecorded，也不会在新 owner 接管后投递命令。
                let permit = match self.controller.reserve_command(command.generation).await {
                    Ok(permit) => Some(permit),
                    Err(RuntimeError::ControllerClosed) => None,
                    Err(RuntimeError::StaleGeneration) => {
                        return RuntimeHostResponse::Rejected {
                            code: RuntimeHostRejection::StaleCommandGeneration,
                        };
                    }
                    Err(
                        RuntimeError::UnsupportedVersion(_)
                        | RuntimeError::InvalidConfiguration(_)
                        | RuntimeError::Protocol(_)
                        | RuntimeError::PermissionCeilingRejected { .. }
                        | RuntimeError::Io(_)
                        | RuntimeError::RequestTimedOut
                        | RuntimeError::EventBackpressure,
                    ) => {
                        unreachable!("reserve_command 只能返回 generation 或 channel 错误")
                    }
                };
                let (message_id, mut status) = {
                    let mut state = match self.lock() {
                        Ok(state) => state,
                        Err(response) => return response,
                    };
                    if let Err(response) = Self::authorize_owner(
                        &state,
                        token,
                        &task_id,
                        runtime_generation,
                        owner_epoch,
                        client_id,
                    ) {
                        return response;
                    }
                    if command.generation != state.manifest.runtime_generation {
                        return RuntimeHostResponse::Rejected {
                            code: RuntimeHostRejection::StaleCommandGeneration,
                        };
                    }
                    if let Some(previous) = state.commands.get(&command.message_id) {
                        return if previous.digest == digest {
                            RuntimeHostResponse::CommandStatus(previous.clone())
                        } else {
                            RuntimeHostResponse::Rejected {
                                code: RuntimeHostRejection::CommandIdentityChanged,
                            }
                        };
                    }
                    let status = HostCommandStatus {
                        message_id: command.message_id,
                        digest,
                        disposition: HostCommandDisposition::Recorded,
                        turn_id: None,
                    };
                    if state
                        .append(JournalPayload::CommandRecorded {
                            status: status.clone(),
                        })
                        .is_err()
                    {
                        return RuntimeHostResponse::Rejected {
                            code: RuntimeHostRejection::CommandJournalFailed,
                        };
                    }
                    state.commands.insert(command.message_id, status.clone());
                    if permit.is_none() {
                        let failure = RuntimeEvent {
                            generation: runtime_generation,
                            native_session_id: state.snapshot.native_session_id.clone(),
                            kind: RuntimeEventKind::RequestFailed {
                                message_id: command.message_id,
                                message: COMMAND_NOT_DELIVERED_CODE.to_owned(),
                            },
                        };
                        if state.record_event(failure).is_ok() {
                            return RuntimeHostResponse::CommandStatus(
                                state
                                    .commands
                                    .get(&command.message_id)
                                    .cloned()
                                    .expect("RequestFailed 成功后命令状态必须存在"),
                            );
                        }
                        return RuntimeHostResponse::CommandStatus(status);
                    }
                    drop(
                        permit
                            .expect("关闭的 adapter 已在持锁分支内持久化失败")
                            .send(command),
                    );
                    (status.message_id, status)
                };
                let mut state = match self.lock() {
                    Ok(state) => state,
                    Err(response) => return response,
                };
                if let Some(current) = state.commands.get(&message_id)
                    && current.disposition != HostCommandDisposition::Recorded
                {
                    // adapter 可能在 send 返回后立即发出原生接收事件；不得把更强状态降级。
                    return RuntimeHostResponse::CommandStatus(current.clone());
                }
                status.disposition = HostCommandDisposition::DeliveredToAdapter;
                if state
                    .append(JournalPayload::CommandUpdated {
                        status: status.clone(),
                    })
                    .is_err()
                {
                    // 命令可能已经进入 adapter；保持 Recorded 即“结果未知”，重连绝不重投。
                    return RuntimeHostResponse::CommandStatus(
                        state
                            .commands
                            .get(&message_id)
                            .cloned()
                            .expect("CommandRecorded 成功后状态必须存在"),
                    );
                }
                state.commands.insert(status.message_id, status.clone());
                RuntimeHostResponse::CommandStatus(status)
            }
            RuntimeHostRequest::PullEvents {
                token,
                task_id,
                runtime_generation,
                owner_epoch,
                client_id,
                after_sequence,
            } => {
                let state = match self.lock() {
                    Ok(state) => state,
                    Err(response) => return response,
                };
                if let Err(response) = Self::authorize_owner(
                    &state,
                    token,
                    &task_id,
                    runtime_generation,
                    owner_epoch,
                    client_id,
                ) {
                    return response;
                }
                RuntimeHostResponse::Events(
                    state
                        .events
                        .iter()
                        .filter(|event| event.sequence > after_sequence)
                        .take(MAX_PULL_EVENTS)
                        .cloned()
                        .collect(),
                )
            }
            RuntimeHostRequest::AckEvents {
                token,
                task_id,
                runtime_generation,
                owner_epoch,
                client_id,
                through_sequence,
            } => {
                let mut state = match self.lock() {
                    Ok(state) => state,
                    Err(response) => return response,
                };
                if let Err(response) = Self::authorize_owner(
                    &state,
                    token,
                    &task_id,
                    runtime_generation,
                    owner_epoch,
                    client_id,
                ) {
                    return response;
                }
                let last = state.events.last().map_or(0, |event| event.sequence);
                if through_sequence < state.acknowledged_sequence || through_sequence > last {
                    return RuntimeHostResponse::Rejected {
                        code: RuntimeHostRejection::InvalidEventAck,
                    };
                }
                if through_sequence > state.acknowledged_sequence {
                    if state
                        .append(JournalPayload::EventsAcknowledged {
                            owner_epoch,
                            through_sequence,
                        })
                        .is_err()
                    {
                        return RuntimeHostResponse::Rejected {
                            code: RuntimeHostRejection::EventAckJournalFailed,
                        };
                    }
                    state.acknowledged_sequence = through_sequence;
                }
                RuntimeHostResponse::EventsAcknowledged { through_sequence }
            }
            RuntimeHostRequest::InspectCommands {
                token,
                task_id,
                runtime_generation,
                owner_epoch,
                client_id,
                message_ids,
            } => {
                let state = match self.lock() {
                    Ok(state) => state,
                    Err(response) => return response,
                };
                if let Err(response) = Self::authorize_owner(
                    &state,
                    token,
                    &task_id,
                    runtime_generation,
                    owner_epoch,
                    client_id,
                ) {
                    return response;
                }
                RuntimeHostResponse::CommandStatuses(
                    message_ids
                        .into_iter()
                        .filter_map(|message_id| state.commands.get(&message_id).cloned())
                        .collect(),
                )
            }
        }
    }
}

fn host_directory(state_dir: &Path, generation: Uuid) -> PathBuf {
    state_dir
        .join("cli-agent-hosts")
        .join(generation.to_string())
}

fn host_manifest_path(state_dir: &Path, generation: Uuid) -> PathBuf {
    host_directory(state_dir, generation).join("manifest.json")
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn rejection_error(code: RuntimeHostRejection) -> io::Error {
    io::Error::other(format!("runtime host rejected request: {code:?}"))
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn write_new_record(path: &Path, contents: &[u8]) -> io::Result<()> {
    if contents.len() as u64 > MAX_HOST_RECORD_BYTES {
        return Err(io::Error::other("运行时宿主记录超过安全上限"));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

fn write_atomic_record(path: &Path, contents: &[u8]) -> io::Result<()> {
    if contents.len() as u64 > MAX_HOST_RECORD_BYTES {
        return Err(io::Error::other("运行时宿主记录超过安全上限"));
    }
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("运行时宿主记录缺少目录"))?;
    let mut temporary = NamedTempFile::new_in(directory)?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn read_private_record(path: &Path) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_HOST_RECORD_BYTES {
        return Err(io::Error::other("运行时宿主记录不是受控普通文件"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::other("运行时宿主记录权限不正确"));
        }
    }
    fs::read(path)
}

fn endpoint_name(host_instance_id: Uuid) -> String {
    #[cfg(target_os = "macos")]
    {
        // macOS 的 Unix socket 路径上限很短；TMPDIR 常已接近上限，固定短前缀并靠随机
        // host_instance_id 与私有 token 完成身份隔离。
        return Path::new("/tmp")
            .join(format!("is-cli-host-{host_instance_id}.sock"))
            .to_string_lossy()
            .into_owned();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return std::env::temp_dir()
            .join(format!("infinishell-cli-host-{host_instance_id}.sock"))
            .to_string_lossy()
            .into_owned();
    }
    #[cfg(windows)]
    {
        format!("InfiniShell_CLI_HOST_{host_instance_id}")
    }
}

fn process_identity(process_id: u32) -> io::Result<(u64, PathBuf)> {
    let pid = Pid::from_u32(process_id);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    let process = system
        .process(pid)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "运行时宿主进程不存在"))?;
    let executable = process
        .exe()
        .ok_or_else(|| io::Error::other("运行时宿主进程缺少可执行文件身份"))?
        .canonicalize()?;
    Ok((process.start_time(), executable))
}

fn runtime_host_executable() -> io::Result<PathBuf> {
    #[cfg(test)]
    {
        let path = std::env::var_os("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("真实宿主测试需指定已构建的宿主 worker 二进制"))?;
        if !path.is_absolute() || !path.is_file() {
            return Err(io::Error::other("宿主 worker 二进制路径无效"));
        }
        Ok(path)
    }
    #[cfg(not(test))]
    super::managed_process::supervisor_executable()
}

fn validate_ready(record: &RuntimeHostRecord, ready: &RuntimeHostReady) -> io::Result<()> {
    let (process_start_time, executable) = process_identity(ready.process_id)?;
    if ready.version != HOST_MANIFEST_VERSION
        || ready.runtime_generation != record.manifest.runtime_generation
        || ready.host_instance_id != record.manifest.host_instance_id
        || ready.manifest_sha256 != record.manifest_sha256
        || ready.process_start_time != process_start_time
        || ready.executable != executable
        || executable != runtime_host_executable()?.canonicalize()?
    {
        return Err(io::Error::other("运行时宿主进程身份不匹配"));
    }
    Ok(())
}

fn validate_manifest(
    path: &Path,
    manifest: &RuntimeHostManifest,
    directory: &Path,
) -> io::Result<()> {
    if manifest.version != HOST_MANIFEST_VERSION
        || manifest.task_id.trim().is_empty()
        || manifest.task_generation < 1
        || manifest.runtime_generation.is_nil()
        || manifest.host_instance_id.is_nil()
        || manifest.token.is_nil()
        || manifest.options.generation != manifest.runtime_generation
        || !manifest.options.executable.is_absolute()
        || !manifest.options.cwd.is_absolute()
        || manifest.endpoint != endpoint_name(manifest.host_instance_id)
        || path.file_name() != Some(std::ffi::OsStr::new("manifest.json"))
        || directory.file_name()
            != Some(std::ffi::OsStr::new(
                &manifest.runtime_generation.to_string(),
            ))
        || !matches!(
            manifest.harness,
            Harness::Codex | Harness::Claude | Harness::Grok
        )
    {
        return Err(io::Error::other("运行时宿主启动契约不匹配"));
    }
    let state_dir = directory
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("运行时宿主缺少状态域"))?;
    if manifest.options.state_dir.canonicalize()? != state_dir.canonicalize()? {
        return Err(io::Error::other("运行时宿主状态域不匹配"));
    }
    Ok(())
}

pub(crate) fn load_record(
    state_dir: &Path,
    generation: Uuid,
) -> io::Result<Option<RuntimeHostRecord>> {
    let directory = host_directory(state_dir, generation);
    let path = directory.join("manifest.json");
    let bytes = match read_private_record(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let manifest: RuntimeHostManifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    validate_manifest(&path, &manifest, &directory)?;
    Ok(Some(RuntimeHostRecord {
        manifest,
        manifest_sha256: digest(&bytes),
        directory,
    }))
}

fn startup_failure_error(original: io::Error, receipt: io::Result<()>) -> io::Error {
    match receipt {
        Ok(()) => original,
        Err(receipt_error) => io::Error::new(
            original.kind(),
            format!("{original}; 运行时宿主启动失败未能可靠收敛：{receipt_error}"),
        ),
    }
}

fn not_started_startup_receipt(
    manifest: &RuntimeHostManifest,
    manifest_sha256: Option<String>,
    phase: RuntimeHostStartupFailurePhase,
) -> RuntimeHostStartupExitReceipt {
    RuntimeHostStartupExitReceipt {
        version: HOST_MANIFEST_VERSION,
        task_id: manifest.task_id.clone(),
        task_generation: manifest.task_generation,
        runtime_generation: manifest.runtime_generation,
        host_instance_id: manifest.host_instance_id,
        manifest_sha256,
        phase,
        host_process_id: None,
        host_exit_success: None,
        host_exit_code: None,
        native_process: NativeProcessCompletion::NotStarted,
        native_cleanup_sha256: None,
    }
}

fn write_startup_exit_receipt(
    directory: &Path,
    receipt: &RuntimeHostStartupExitReceipt,
) -> io::Result<()> {
    write_new_record(
        &directory.join(STARTUP_EXIT_RECORD),
        &serde_json::to_vec(receipt).map_err(io::Error::other)?,
    )?;
    #[cfg(unix)]
    File::open(directory)?.sync_all()?;
    Ok(())
}

fn write_startup_incomplete_marker(record: &RuntimeHostRecord) -> io::Result<()> {
    let marker = RuntimeHostStartupIncomplete {
        version: HOST_MANIFEST_VERSION,
        task_id: record.manifest.task_id.clone(),
        task_generation: record.manifest.task_generation,
        runtime_generation: record.manifest.runtime_generation,
        host_instance_id: record.manifest.host_instance_id,
        manifest_sha256: record.manifest_sha256.clone(),
    };
    write_new_record(
        &record.directory.join(STARTUP_INCOMPLETE_RECORD),
        &serde_json::to_vec(&marker).map_err(io::Error::other)?,
    )?;
    #[cfg(unix)]
    File::open(&record.directory)?.sync_all()?;
    Ok(())
}

fn remove_startup_incomplete_marker(record: &RuntimeHostRecord) -> io::Result<()> {
    fs::remove_file(record.directory.join(STARTUP_INCOMPLETE_RECORD))?;
    #[cfg(unix)]
    File::open(&record.directory)?.sync_all()?;
    Ok(())
}

fn create_record(
    task_id: String,
    task_generation: i64,
    harness: Harness,
    options: SessionOptions,
) -> io::Result<RuntimeHostRecord> {
    if task_id.trim().is_empty()
        || task_generation < 1
        || options.generation.is_nil()
        || !options.state_dir.is_absolute()
    {
        return Err(io::Error::other("运行时宿主任务身份无效"));
    }
    let parent = options.state_dir.join("cli-agent-hosts");
    fs::create_dir_all(&parent)?;
    let parent = parent.canonicalize()?;
    let directory = parent.join(options.generation.to_string());
    create_private_directory(&directory)?;
    let host_instance_id = Uuid::new_v4();
    let manifest = RuntimeHostManifest {
        version: HOST_MANIFEST_VERSION,
        task_id,
        task_generation,
        harness,
        runtime_generation: options.generation,
        host_instance_id,
        token: Uuid::new_v4(),
        endpoint: endpoint_name(host_instance_id),
        options,
    };
    let bytes = match serde_json::to_vec(&manifest).map_err(io::Error::other) {
        Ok(bytes) => bytes,
        Err(error) => {
            let receipt = not_started_startup_receipt(
                &manifest,
                None,
                RuntimeHostStartupFailurePhase::Manifest,
            );
            return Err(startup_failure_error(
                error,
                write_startup_exit_receipt(&directory, &receipt),
            ));
        }
    };
    let manifest_sha256 = digest(&bytes);
    if let Err(error) = write_new_record(&directory.join("manifest.json"), &bytes) {
        let receipt = not_started_startup_receipt(
            &manifest,
            Some(manifest_sha256),
            RuntimeHostStartupFailurePhase::Manifest,
        );
        return Err(startup_failure_error(
            error,
            write_startup_exit_receipt(&directory, &receipt),
        ));
    }
    Ok(RuntimeHostRecord {
        manifest,
        manifest_sha256,
        directory,
    })
}

#[cfg(test)]
pub(crate) fn write_manifest_failure_receipt_for_test(
    task_id: String,
    task_generation: i64,
    harness: Harness,
    options: SessionOptions,
) -> io::Result<()> {
    let parent = options.state_dir.join("cli-agent-hosts");
    fs::create_dir_all(&parent)?;
    let directory = parent.join(options.generation.to_string());
    create_private_directory(&directory)?;
    let host_instance_id = Uuid::new_v4();
    let manifest = RuntimeHostManifest {
        version: HOST_MANIFEST_VERSION,
        task_id,
        task_generation,
        harness,
        runtime_generation: options.generation,
        host_instance_id,
        token: Uuid::new_v4(),
        endpoint: endpoint_name(host_instance_id),
        options,
    };
    let receipt =
        not_started_startup_receipt(&manifest, None, RuntimeHostStartupFailurePhase::Manifest);
    write_startup_exit_receipt(&directory, &receipt)
}

#[cfg(test)]
pub(crate) fn write_cancelled_startup_marker_for_test(
    task_id: String,
    task_generation: i64,
    harness: Harness,
    options: SessionOptions,
) -> io::Result<()> {
    let record = create_record(task_id, task_generation, harness, options)?;
    write_startup_incomplete_marker(&record)
}

pub(crate) fn is_runtime_host_manifest(path: &Path) -> io::Result<bool> {
    let bytes = read_private_record(path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    Ok(value.get("version").and_then(serde_json::Value::as_u64)
        == Some(HOST_MANIFEST_VERSION as u64)
        && value.get("host_instance_id").is_some())
}

pub(crate) struct RuntimeHostClient {
    record: RuntimeHostRecord,
    client_id: Uuid,
    owner_epoch: Option<u64>,
    client: Arc<Client>,
    _executor: Arc<Background>,
}

pub(crate) enum RuntimeHostStartupState {
    LiveReattachable(RuntimeHostClient),
    ExitedResumable {
        receipt: RuntimeHostExitReceipt,
        pending_events: Vec<HostedEvent>,
        snapshot: RuntimeHostSnapshot,
    },
    Unconfirmed(RuntimeHostUnconfirmed),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeHostUnconfirmed {
    MissingManifest,
    HostUnavailable,
    AdapterNotStarted,
    NativeExitUnconfirmed,
}

impl RuntimeHostClient {
    async fn connect(record: RuntimeHostRecord) -> io::Result<Self> {
        let executor = Arc::new(Background::new(1, |_| "cli-runtime-host-client".to_owned()));
        let client = Client::connect_with_max_frame_bytes(
            ConnectionAddress::from(record.manifest.endpoint.clone()),
            executor.clone(),
            MAX_RUNTIME_HOST_FRAME_BYTES,
        )
        .await
        .map_err(io::Error::other)?;
        Ok(Self {
            record,
            client_id: Uuid::new_v4(),
            owner_epoch: None,
            client: Arc::new(client),
            _executor: executor,
        })
    }

    async fn call(&self, request: RuntimeHostRequest) -> io::Result<RuntimeHostResponse> {
        let caller = ipc::service_caller::<RuntimeHostService>(self.client.clone());
        let call = caller.call(request);
        let timeout = Timer::after(RUNTIME_HOST_CALL_TIMEOUT);
        futures::pin_mut!(call, timeout);
        match futures::future::select(call, timeout).await {
            futures::future::Either::Left((result, _)) => result.map_err(io::Error::other),
            futures::future::Either::Right((_, _)) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "runtime_host.ipc_call_timed_out",
            )),
        }
    }

    pub(crate) async fn inspect(&self) -> io::Result<RuntimeHostResponse> {
        self.call(RuntimeHostRequest::Inspect {
            token: self.record.manifest.token,
            task_id: self.record.manifest.task_id.clone(),
            runtime_generation: self.record.manifest.runtime_generation,
        })
        .await
    }

    async fn claim_owner_state(
        &mut self,
        observed_epoch: u64,
    ) -> io::Result<(u64, u64, RuntimeHostSnapshot)> {
        let response = self
            .call(RuntimeHostRequest::ClaimOwner {
                token: self.record.manifest.token,
                task_id: self.record.manifest.task_id.clone(),
                runtime_generation: self.record.manifest.runtime_generation,
                observed_epoch,
                client_id: self.client_id,
            })
            .await?;
        match response {
            RuntimeHostResponse::OwnerClaimed {
                owner_epoch,
                last_event_sequence,
                acknowledged_sequence,
                snapshot,
            } => {
                self.owner_epoch = Some(owner_epoch);
                Ok((last_event_sequence, acknowledged_sequence, snapshot))
            }
            RuntimeHostResponse::Rejected { code } => Err(rejection_error(code)),
            RuntimeHostResponse::Inspection { .. }
            | RuntimeHostResponse::CommandStatus(_)
            | RuntimeHostResponse::Events(_)
            | RuntimeHostResponse::EventsAcknowledged { .. }
            | RuntimeHostResponse::CommandStatuses(_) => {
                Err(io::Error::other("运行时宿主所有者响应类型无效"))
            }
        }
    }

    pub(crate) async fn claim_owner(&mut self, observed_epoch: u64) -> io::Result<u64> {
        self.claim_owner_state(observed_epoch).await?;
        Ok(self.owner_epoch.expect("claim 成功后必须设置 owner epoch"))
    }

    fn owner_epoch(&self) -> io::Result<u64> {
        self.owner_epoch
            .ok_or_else(|| io::Error::other("运行时宿主尚未取得所有权"))
    }

    pub(crate) async fn send_command(
        &self,
        command: RuntimeCommand,
    ) -> io::Result<HostCommandStatus> {
        let response = self
            .call(RuntimeHostRequest::Command {
                token: self.record.manifest.token,
                task_id: self.record.manifest.task_id.clone(),
                runtime_generation: self.record.manifest.runtime_generation,
                owner_epoch: self.owner_epoch()?,
                client_id: self.client_id,
                command,
            })
            .await?;
        match response {
            RuntimeHostResponse::CommandStatus(status) => Ok(status),
            RuntimeHostResponse::Rejected { code } => Err(rejection_error(code)),
            RuntimeHostResponse::Inspection { .. }
            | RuntimeHostResponse::OwnerClaimed { .. }
            | RuntimeHostResponse::Events(_)
            | RuntimeHostResponse::EventsAcknowledged { .. }
            | RuntimeHostResponse::CommandStatuses(_) => {
                Err(io::Error::other("运行时宿主命令响应类型无效"))
            }
        }
    }

    pub(crate) async fn pull_events(&self, after_sequence: u64) -> io::Result<Vec<HostedEvent>> {
        let response = self
            .call(RuntimeHostRequest::PullEvents {
                token: self.record.manifest.token,
                task_id: self.record.manifest.task_id.clone(),
                runtime_generation: self.record.manifest.runtime_generation,
                owner_epoch: self.owner_epoch()?,
                client_id: self.client_id,
                after_sequence,
            })
            .await?;
        match response {
            RuntimeHostResponse::Events(events) => Ok(events),
            RuntimeHostResponse::Rejected { code } => Err(rejection_error(code)),
            RuntimeHostResponse::Inspection { .. }
            | RuntimeHostResponse::OwnerClaimed { .. }
            | RuntimeHostResponse::CommandStatus(_)
            | RuntimeHostResponse::EventsAcknowledged { .. }
            | RuntimeHostResponse::CommandStatuses(_) => {
                Err(io::Error::other("运行时宿主事件响应类型无效"))
            }
        }
    }

    pub(crate) async fn acknowledge_events(&self, through_sequence: u64) -> io::Result<()> {
        let response = self
            .call(RuntimeHostRequest::AckEvents {
                token: self.record.manifest.token,
                task_id: self.record.manifest.task_id.clone(),
                runtime_generation: self.record.manifest.runtime_generation,
                owner_epoch: self.owner_epoch()?,
                client_id: self.client_id,
                through_sequence,
            })
            .await?;
        match response {
            RuntimeHostResponse::EventsAcknowledged {
                through_sequence: actual,
            } if actual == through_sequence => Ok(()),
            RuntimeHostResponse::Rejected { code } => Err(rejection_error(code)),
            RuntimeHostResponse::Inspection { .. }
            | RuntimeHostResponse::OwnerClaimed { .. }
            | RuntimeHostResponse::CommandStatus(_)
            | RuntimeHostResponse::Events(_)
            | RuntimeHostResponse::EventsAcknowledged { .. }
            | RuntimeHostResponse::CommandStatuses(_) => {
                Err(io::Error::other("运行时宿主确认响应类型无效"))
            }
        }
    }

    pub(crate) async fn inspect_commands(
        &self,
        message_ids: Vec<Uuid>,
    ) -> io::Result<Vec<HostCommandStatus>> {
        let response = self
            .call(RuntimeHostRequest::InspectCommands {
                token: self.record.manifest.token,
                task_id: self.record.manifest.task_id.clone(),
                runtime_generation: self.record.manifest.runtime_generation,
                owner_epoch: self.owner_epoch()?,
                client_id: self.client_id,
                message_ids,
            })
            .await?;
        match response {
            RuntimeHostResponse::CommandStatuses(statuses) => Ok(statuses),
            RuntimeHostResponse::Rejected { code } => Err(rejection_error(code)),
            RuntimeHostResponse::Inspection { .. }
            | RuntimeHostResponse::OwnerClaimed { .. }
            | RuntimeHostResponse::CommandStatus(_)
            | RuntimeHostResponse::Events(_)
            | RuntimeHostResponse::EventsAcknowledged { .. } => {
                Err(io::Error::other("运行时宿主命令检查响应类型无效"))
            }
        }
    }

    pub(crate) fn options(&self) -> &SessionOptions {
        &self.record.manifest.options
    }

    pub(crate) fn task_identity(&self) -> (&str, i64) {
        (
            &self.record.manifest.task_id,
            self.record.manifest.task_generation,
        )
    }
}

pub(crate) struct RuntimeHostClaim {
    client: RuntimeHostClient,
    pub(crate) snapshot: RuntimeHostSnapshot,
    pub(crate) acknowledged_sequence: u64,
    pub(crate) catch_up_through: u64,
}

impl RuntimeHostClaim {
    pub(crate) async fn pull_events(&self, after_sequence: u64) -> io::Result<Vec<HostedEvent>> {
        self.client.pull_events(after_sequence).await
    }

    pub(crate) async fn acknowledge_events(&mut self, through_sequence: u64) -> io::Result<()> {
        self.client.acknowledge_events(through_sequence).await?;
        self.acknowledged_sequence = through_sequence;
        Ok(())
    }

    pub(crate) fn into_connection(self) -> io::Result<super::RuntimeConnection> {
        if self.acknowledged_sequence != self.catch_up_through {
            return Err(io::Error::other("运行时宿主 catch-up 尚未完成"));
        }
        Ok(bridge_connection(self.client, self.catch_up_through))
    }
}

async fn claim_for_connection(mut client: RuntimeHostClient) -> io::Result<RuntimeHostClaim> {
    let RuntimeHostResponse::Inspection { owner_epoch, .. } = client.inspect().await? else {
        return Err(io::Error::other("运行时宿主检查响应无效"));
    };
    let (last_event_sequence, acknowledged_sequence, snapshot) =
        client.claim_owner_state(owner_epoch).await?;
    Ok(RuntimeHostClaim {
        client,
        snapshot,
        acknowledged_sequence,
        catch_up_through: last_event_sequence,
    })
}

async fn forward_host_events(
    client: Arc<RuntimeHostClient>,
    sender: tokio::sync::mpsc::Sender<RuntimeEvent>,
    mut committed_events: tokio::sync::mpsc::Receiver<()>,
    acknowledged_sequence: u64,
) -> io::Result<()> {
    let mut after_sequence = acknowledged_sequence;
    loop {
        let hosted = client.pull_events(after_sequence).await?;
        if hosted.is_empty() {
            Timer::after(Duration::from_millis(25)).await;
            continue;
        }
        for event in hosted {
            let sequence = event.sequence;
            sender
                .send(event.event)
                .await
                .map_err(|_| io::Error::other("运行时宿主事件接收方已关闭"))?;
            committed_events
                .recv()
                .await
                .ok_or_else(|| io::Error::other("运行时宿主事件提交方已关闭"))?;
            client.acknowledge_events(sequence).await?;
            after_sequence = sequence;
        }
    }
}

async fn forward_host_command(
    client: &RuntimeHostClient,
    command: RuntimeCommand,
) -> io::Result<()> {
    let status = client.send_command(command).await?;
    match status.disposition {
        HostCommandDisposition::Recorded => Err(io::Error::other(
            "运行时宿主命令交付结果未知；保留原消息且禁止重投",
        )),
        HostCommandDisposition::DeliveredToAdapter
        | HostCommandDisposition::NativeAccepted
        | HostCommandDisposition::Failed => Ok(()),
    }
}

fn bridge_connection(client: RuntimeHostClient, after_sequence: u64) -> super::RuntimeConnection {
    let generation = client.record.manifest.runtime_generation;
    let (mut controller, mut commands, sender, events) = super::channels(generation);
    let (host_event_acks, committed_events) = tokio::sync::mpsc::channel(1);
    controller.host_event_acks = Some(host_event_acks);
    let client = Arc::new(client);
    let command_client = client.clone();
    let event_client = client;
    let task = Box::pin(async move {
        let commands = async move {
            while let Some(command) = commands.recv().await {
                forward_host_command(&command_client, command).await?;
            }
            Ok::<(), io::Error>(())
        };
        let events = forward_host_events(event_client, sender, committed_events, after_sequence);
        futures::future::try_join(commands, events)
            .await
            .map(|_| ())
            .map_err(RuntimeError::Io)
    });
    super::RuntimeConnection {
        controller,
        events,
        task,
    }
}

pub(crate) fn spawn_connection(
    task_id: String,
    task_generation: i64,
    harness: Harness,
    options: SessionOptions,
) -> super::RuntimeConnection {
    let generation = options.generation;
    let (mut controller, mut commands, sender, events) = super::channels(generation);
    let (host_event_acks, committed_events) = tokio::sync::mpsc::channel(1);
    controller.host_event_acks = Some(host_event_acks);
    let task = Box::pin(async move {
        let client = spawn_host(task_id, task_generation, harness, options).await?;
        let claim = claim_for_connection(client).await?;
        let acknowledged_sequence = claim.acknowledged_sequence;
        let client = claim.client;
        let client = Arc::new(client);
        let command_client = client.clone();
        let event_client = client;
        let commands = async move {
            while let Some(command) = commands.recv().await {
                forward_host_command(&command_client, command).await?;
            }
            Ok::<(), io::Error>(())
        };
        let events = forward_host_events(
            event_client,
            sender,
            committed_events,
            acknowledged_sequence,
        );
        futures::future::try_join(commands, events)
            .await
            .map(|_| ())
            .map_err(RuntimeError::Io)
    });
    super::RuntimeConnection {
        controller,
        events,
        task,
    }
}

pub(crate) async fn reattach_connection(client: RuntimeHostClient) -> io::Result<RuntimeHostClaim> {
    claim_for_connection(client).await
}

pub(crate) async fn connect_existing(
    state_dir: &Path,
    generation: Uuid,
) -> io::Result<Option<RuntimeHostClient>> {
    let Some(record) = load_record(state_dir, generation)? else {
        return Ok(None);
    };
    connect_verified(record).await.map(Some)
}

async fn connect_verified(record: RuntimeHostRecord) -> io::Result<RuntimeHostClient> {
    let ready: RuntimeHostReady =
        serde_json::from_slice(&read_private_record(&record.directory.join("ready.json"))?)
            .map_err(io::Error::other)?;
    validate_ready(&record, &ready)?;
    let client = RuntimeHostClient::connect(record).await?;
    match client.inspect().await? {
        RuntimeHostResponse::Inspection {
            host_instance_id, ..
        } if host_instance_id == client.record.manifest.host_instance_id => Ok(client),
        RuntimeHostResponse::Rejected { code } => Err(rejection_error(code)),
        RuntimeHostResponse::Inspection { .. }
        | RuntimeHostResponse::OwnerClaimed { .. }
        | RuntimeHostResponse::CommandStatus(_)
        | RuntimeHostResponse::Events(_)
        | RuntimeHostResponse::EventsAcknowledged { .. }
        | RuntimeHostResponse::CommandStatuses(_) => {
            Err(io::Error::other("运行时宿主握手身份不匹配"))
        }
    }
}

fn read_startup_incomplete_marker(
    state_dir: &Path,
    generation: Uuid,
) -> io::Result<Option<RuntimeHostStartupIncomplete>> {
    let directory = host_directory(state_dir, generation);
    let bytes = match read_private_record(&directory.join(STARTUP_INCOMPLETE_RECORD)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let marker: RuntimeHostStartupIncomplete =
        serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let record = load_record(state_dir, generation)?
        .ok_or_else(|| io::Error::other("运行时宿主取消标记缺少 manifest"))?;
    if marker.version != HOST_MANIFEST_VERSION
        || marker.task_id != record.manifest.task_id
        || marker.task_generation != record.manifest.task_generation
        || marker.runtime_generation != generation
        || marker.host_instance_id != record.manifest.host_instance_id
        || marker.manifest_sha256 != record.manifest_sha256
    {
        return Err(io::Error::other("运行时宿主取消标记身份不匹配"));
    }
    Ok(Some(marker))
}

fn read_startup_exit_receipt(
    state_dir: &Path,
    generation: Uuid,
) -> io::Result<Option<RuntimeHostStartupExitReceipt>> {
    let directory = host_directory(state_dir, generation);
    let bytes = match read_private_record(&directory.join(STARTUP_EXIT_RECORD)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let receipt: RuntimeHostStartupExitReceipt =
        serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if receipt.version != HOST_MANIFEST_VERSION
        || receipt.task_id.trim().is_empty()
        || receipt.task_generation < 1
        || receipt.runtime_generation != generation
        || receipt.host_instance_id.is_nil()
    {
        return Err(io::Error::other("运行时宿主启动失败收据身份不匹配"));
    }
    match receipt.phase {
        RuntimeHostStartupFailurePhase::Manifest => {}
        RuntimeHostStartupFailurePhase::Executable
        | RuntimeHostStartupFailurePhase::Spawn
        | RuntimeHostStartupFailurePhase::ReadyHandshake => {
            let record = load_record(state_dir, generation)?
                .ok_or_else(|| io::Error::other("运行时宿主启动失败收据缺少 manifest"))?;
            if receipt.task_id != record.manifest.task_id
                || receipt.task_generation != record.manifest.task_generation
                || receipt.host_instance_id != record.manifest.host_instance_id
                || receipt.manifest_sha256.as_deref() != Some(record.manifest_sha256.as_str())
            {
                return Err(io::Error::other("运行时宿主启动失败收据与 manifest 不匹配"));
            }
        }
    }
    match receipt.phase {
        RuntimeHostStartupFailurePhase::Manifest
        | RuntimeHostStartupFailurePhase::Executable
        | RuntimeHostStartupFailurePhase::Spawn => {
            if receipt.host_process_id.is_some()
                || receipt.host_exit_success.is_some()
                || receipt.host_exit_code.is_some()
                || receipt.native_process != NativeProcessCompletion::NotStarted
                || receipt.native_cleanup_sha256.is_some()
            {
                return Err(io::Error::other("运行时宿主未启动收据包含不存在的进程证据"));
            }
        }
        RuntimeHostStartupFailurePhase::ReadyHandshake => {
            if receipt.host_process_id == Some(0)
                || receipt.host_process_id.is_none()
                || receipt.host_exit_success.is_none()
                || receipt.native_process == NativeProcessCompletion::Unconfirmed
            {
                return Err(io::Error::other("运行时宿主握手失败收据缺少进程退出证据"));
            }
            let exit_status_consistent = match (receipt.host_exit_success, receipt.host_exit_code) {
                (Some(true), Some(exit_code)) => exit_code == 0,
                (Some(true), None) => false,
                (Some(false), Some(exit_code)) => exit_code != 0,
                (Some(false), None) => true,
                (None, Some(_)) => false,
                (None, None) => false,
            };
            if !exit_status_consistent {
                return Err(io::Error::other("运行时宿主进程退出状态证据不一致"));
            }
        }
    }
    let native = super::managed_process::process_completion(&directory.join("native"), generation)?;
    match receipt.native_process {
        NativeProcessCompletion::NotStarted => match native {
            super::managed_process::ProcessCompletion::NotStarted
                if receipt.native_cleanup_sha256.is_none() => {}
            super::managed_process::ProcessCompletion::NotStarted
            | super::managed_process::ProcessCompletion::Exited(_)
            | super::managed_process::ProcessCompletion::Unconfirmed => {
                return Err(io::Error::other(
                    "运行时宿主启动失败收据缺少真实进程收敛证明",
                ));
            }
        },
        NativeProcessCompletion::Exited => match native {
            super::managed_process::ProcessCompletion::Exited(native) => {
                let native_sha256 = digest(&serde_json::to_vec(&native).map_err(io::Error::other)?);
                if receipt.native_cleanup_sha256.as_deref() != Some(&native_sha256) {
                    return Err(io::Error::other(
                        "运行时宿主启动失败收据的原生清理摘要不匹配",
                    ));
                }
            }
            super::managed_process::ProcessCompletion::NotStarted
            | super::managed_process::ProcessCompletion::Unconfirmed => {
                return Err(io::Error::other(
                    "运行时宿主启动失败收据缺少真实进程收敛证明",
                ));
            }
        },
        NativeProcessCompletion::Unconfirmed => {
            return Err(io::Error::other(
                "运行时宿主启动失败收据缺少真实进程收敛证明",
            ));
        }
    }
    Ok(Some(receipt))
}

/// 恢复入口在读取 manifest 前先核对持久启动证据，避免合法的 manifest 阶段
/// 收据被 `load_record(None)` 静默跳过；返回 false 时必须按身份不匹配 fail-closed。
pub(crate) fn startup_evidence_matches_task(
    state_dir: &Path,
    generation: Uuid,
    task_id: &str,
    task_generation: i64,
) -> io::Result<Option<bool>> {
    if let Some(marker) = read_startup_incomplete_marker(state_dir, generation)? {
        return Ok(Some(
            marker.task_id == task_id && marker.task_generation == task_generation,
        ));
    }
    Ok(read_startup_exit_receipt(state_dir, generation)?
        .map(|receipt| receipt.task_id == task_id && receipt.task_generation == task_generation))
}

/// 更新空闲判断只接受已核验的未启动或完整退出证据，IPC 不可达本身不是退出证明。
pub(crate) fn exit_confirmed_for_update(state_dir: &Path, generation: Uuid) -> io::Result<bool> {
    if read_startup_incomplete_marker(state_dir, generation)?.is_some() {
        return Ok(false);
    }
    if read_startup_exit_receipt(state_dir, generation)?.is_some() {
        return Ok(true);
    }
    if load_record(state_dir, generation)?.is_some() {
        return Ok(confirmed_exit(state_dir, generation)?.is_some());
    }
    // 兼容独立宿主引入前的已确认原生进程收据；不存在任何证据时继续保持忙状态。
    Ok(super::managed_process::confirmed_exit(state_dir, generation)?.is_some())
}

pub(crate) async fn classify_startup(
    state_dir: &Path,
    generation: Uuid,
) -> io::Result<RuntimeHostStartupState> {
    if read_startup_incomplete_marker(state_dir, generation)?.is_some() {
        return Ok(RuntimeHostStartupState::Unconfirmed(
            RuntimeHostUnconfirmed::NativeExitUnconfirmed,
        ));
    }
    if let Some(receipt) = read_startup_exit_receipt(state_dir, generation)? {
        return Ok(RuntimeHostStartupState::Unconfirmed(
            match receipt.native_process {
                NativeProcessCompletion::NotStarted => RuntimeHostUnconfirmed::AdapterNotStarted,
                NativeProcessCompletion::Exited => RuntimeHostUnconfirmed::HostUnavailable,
                NativeProcessCompletion::Unconfirmed => {
                    return Err(io::Error::other(
                        "运行时宿主启动失败收据不允许未确认的原生退出",
                    ));
                }
            },
        ));
    }
    let Some(record) = load_record(state_dir, generation)? else {
        return Ok(RuntimeHostStartupState::Unconfirmed(
            RuntimeHostUnconfirmed::MissingManifest,
        ));
    };
    if let Some(receipt) = read_exit_receipt(&record)? {
        return match receipt.native_process {
            NativeProcessCompletion::Exited => {
                let (receipt, pending_events, snapshot) =
                    recover_exited_events(state_dir, generation)?
                        .ok_or_else(|| io::Error::other("运行时宿主退出回执丢失"))?;
                Ok(RuntimeHostStartupState::ExitedResumable {
                    receipt,
                    pending_events,
                    snapshot,
                })
            }
            NativeProcessCompletion::NotStarted => Ok(RuntimeHostStartupState::Unconfirmed(
                RuntimeHostUnconfirmed::AdapterNotStarted,
            )),
            NativeProcessCompletion::Unconfirmed => Ok(RuntimeHostStartupState::Unconfirmed(
                RuntimeHostUnconfirmed::NativeExitUnconfirmed,
            )),
        };
    }
    match connect_verified(record).await {
        Ok(client) => Ok(RuntimeHostStartupState::LiveReattachable(client)),
        Err(_) => Ok(RuntimeHostStartupState::Unconfirmed(
            RuntimeHostUnconfirmed::HostUnavailable,
        )),
    }
}

async fn observe_spawned_host_exit(
    process: &mut Child,
    process_id: u32,
) -> io::Result<ObservedHostExit> {
    if process.try_status()?.is_none()
        && let Err(error) = process.kill()
        && process.try_status()?.is_none()
    {
        return Err(error);
    }
    let status = match process.try_status()? {
        Some(status) => status,
        None => {
            let status = process.status();
            let timeout = Timer::after(RUNTIME_HOST_EXIT_TIMEOUT);
            futures::pin_mut!(status, timeout);
            match futures::future::select(status, timeout).await {
                futures::future::Either::Left((status, _)) => status?,
                futures::future::Either::Right((_, _)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "运行时宿主终止后未在期限内退出",
                    ));
                }
            }
        }
    };
    Ok(ObservedHostExit {
        process_id,
        success: status.success(),
        exit_code: status.code(),
    })
}

async fn wait_for_native_startup_cleanup(
    record: &RuntimeHostRecord,
) -> io::Result<super::managed_process::ProcessCompletion> {
    for attempt in 0..=RUNTIME_HOST_NATIVE_CLEANUP_ATTEMPTS {
        match super::managed_process::process_completion(
            &record.directory.join("native"),
            record.manifest.runtime_generation,
        )? {
            super::managed_process::ProcessCompletion::Unconfirmed
                if attempt < RUNTIME_HOST_NATIVE_CLEANUP_ATTEMPTS =>
            {
                Timer::after(RUNTIME_HOST_POLL_INTERVAL).await;
            }
            super::managed_process::ProcessCompletion::Unconfirmed => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "运行时宿主退出后真实 CLI 清理仍未确认",
                ));
            }
            completion => return Ok(completion),
        }
    }
    unreachable!("原生清理轮询必须在有界循环内返回")
}

fn existing_host_exit_is_recoverable(
    record: &RuntimeHostRecord,
    receipt: &RuntimeHostExitReceipt,
    native: &super::managed_process::ProcessCompletion,
) -> io::Result<bool> {
    match receipt.native_process {
        NativeProcessCompletion::NotStarted => match native {
            super::managed_process::ProcessCompletion::NotStarted => {
                Ok(receipt.native_cleanup_sha256.is_none())
            }
            super::managed_process::ProcessCompletion::Exited(_)
            | super::managed_process::ProcessCompletion::Unconfirmed => Ok(false),
        },
        NativeProcessCompletion::Exited => match native {
            super::managed_process::ProcessCompletion::Exited(native) => {
                let native_sha256 = digest(&serde_json::to_vec(native).map_err(io::Error::other)?);
                if receipt.native_cleanup_sha256.as_deref() != Some(&native_sha256) {
                    return Ok(false);
                }
                Ok(confirmed_exit(
                    &record.manifest.options.state_dir,
                    record.manifest.runtime_generation,
                )
                .is_ok_and(|receipt| receipt.is_some()))
            }
            super::managed_process::ProcessCompletion::NotStarted
            | super::managed_process::ProcessCompletion::Unconfirmed => Ok(false),
        },
        NativeProcessCompletion::Unconfirmed => Ok(false),
    }
}

fn observed_startup_exit_receipt(
    record: &RuntimeHostRecord,
    observed: ObservedHostExit,
    native: super::managed_process::ProcessCompletion,
) -> io::Result<RuntimeHostStartupExitReceipt> {
    let (native_process, native_cleanup_sha256) = match native {
        super::managed_process::ProcessCompletion::NotStarted => {
            (NativeProcessCompletion::NotStarted, None)
        }
        super::managed_process::ProcessCompletion::Exited(native) => (
            NativeProcessCompletion::Exited,
            Some(digest(
                &serde_json::to_vec(&native).map_err(io::Error::other)?,
            )),
        ),
        super::managed_process::ProcessCompletion::Unconfirmed => {
            return Err(io::Error::other(
                "未确认真实 CLI 清理时禁止生成宿主启动失败收据",
            ));
        }
    };
    Ok(RuntimeHostStartupExitReceipt {
        version: HOST_MANIFEST_VERSION,
        task_id: record.manifest.task_id.clone(),
        task_generation: record.manifest.task_generation,
        runtime_generation: record.manifest.runtime_generation,
        host_instance_id: record.manifest.host_instance_id,
        manifest_sha256: Some(record.manifest_sha256.clone()),
        phase: RuntimeHostStartupFailurePhase::ReadyHandshake,
        host_process_id: Some(observed.process_id),
        host_exit_success: Some(observed.success),
        host_exit_code: observed.exit_code,
        native_process,
        native_cleanup_sha256,
    })
}

async fn converge_spawned_startup_failure(
    record: &RuntimeHostRecord,
    process: &mut Child,
) -> io::Result<()> {
    let process_id = process.id();
    let observed = observe_spawned_host_exit(process, process_id).await?;
    let native = wait_for_native_startup_cleanup(record).await?;
    if let Ok(Some(receipt)) = read_exit_receipt(record)
        && existing_host_exit_is_recoverable(record, &receipt, &native)?
    {
        return Ok(());
    }
    let receipt = observed_startup_exit_receipt(record, observed, native)?;
    write_startup_exit_receipt(&record.directory, &receipt)
}

pub(crate) async fn spawn_host(
    task_id: String,
    task_generation: i64,
    harness: Harness,
    options: SessionOptions,
) -> io::Result<RuntimeHostClient> {
    let record = create_record(task_id, task_generation, harness, options)?;
    let executable = match runtime_host_executable() {
        Ok(executable) => executable,
        Err(error) => {
            let receipt = not_started_startup_receipt(
                &record.manifest,
                Some(record.manifest_sha256.clone()),
                RuntimeHostStartupFailurePhase::Executable,
            );
            return Err(startup_failure_error(
                error,
                write_startup_exit_receipt(&record.directory, &receipt),
            ));
        }
    };
    let mut command = Command::new(executable);
    command
        .arg(HOST_WORKER_COMMAND)
        .arg(record.directory.join("manifest.json"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // 未确认标记必须先于进程创建持久化，确保 spawn future 在任意 await 点取消时
    // 恢复入口都不会把一次 kill 请求误判为可重试的未启动收据。
    write_startup_incomplete_marker(&record)?;
    let spawned = match command.spawn() {
        Ok(process) => process,
        Err(error) => {
            let receipt = not_started_startup_receipt(
                &record.manifest,
                Some(record.manifest_sha256.clone()),
                RuntimeHostStartupFailurePhase::Spawn,
            );
            if let Err(receipt_error) = write_startup_exit_receipt(&record.directory, &receipt) {
                return Err(startup_failure_error(error, Err(receipt_error)));
            }
            // 先持久化未启动收据再清除未收敛标记；任一中断点都必须留下
            // fail-closed 证据，不能出现 manifest 存在但启动阶段不可判定的空窗。
            if let Err(clear_error) = remove_startup_incomplete_marker(&record) {
                return Err(startup_failure_error(error, Err(clear_error)));
            }
            return Err(error);
        }
    };
    let mut process = SpawnedHostGuard::new(spawned, record.clone());
    let startup = async {
        for _ in 0..400 {
            if let Some(receipt) = read_exit_receipt(&record)? {
                let message = match receipt.native_process {
                    NativeProcessCompletion::NotStarted => "运行时宿主在真实 CLI 启动前终止",
                    NativeProcessCompletion::Exited => "运行时宿主在连接前已经退出",
                    NativeProcessCompletion::Unconfirmed => "运行时宿主退出但原生清理未确认",
                };
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, message));
            }
            match read_private_record(&record.directory.join("ready.json")) {
                Ok(bytes) => {
                    let ready: RuntimeHostReady =
                        serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                    validate_ready(&record, &ready)?;
                    return connect_verified(record.clone()).await;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    Timer::after(RUNTIME_HOST_POLL_INTERVAL).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "运行时宿主未在期限内就绪",
        ))
    }
    .await;
    match startup {
        Ok(client) => {
            process.release()?;
            Ok(client)
        }
        Err(error) => {
            let mut convergence =
                converge_spawned_startup_failure(&record, process.process_mut()).await;
            if convergence.is_ok() {
                convergence = process.release();
            }
            Err(startup_failure_error(error, convergence))
        }
    }
}

pub(crate) fn confirmed_exit(
    state_dir: &Path,
    generation: Uuid,
) -> io::Result<Option<RuntimeHostExitReceipt>> {
    let Some(record) = load_record(state_dir, generation)? else {
        return Ok(None);
    };
    let bytes = match read_private_record(&record.directory.join("host-exit.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let receipt: RuntimeHostExitReceipt =
        serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if receipt.version != HOST_MANIFEST_VERSION
        || receipt.runtime_generation != generation
        || receipt.host_instance_id != record.manifest.host_instance_id
        || receipt.manifest_sha256 != record.manifest_sha256
        || receipt.native_process != NativeProcessCompletion::Exited
        || !receipt.adapter_task_terminated
        || !receipt.event_journal_completed
        || receipt.native_cleanup_sha256.is_none()
    {
        return Err(io::Error::other("运行时宿主退出回执不完整或不匹配"));
    }
    let summary = verify_journal(&record)?;
    if receipt.journal_sha256 != summary.final_sha256
        || receipt.last_event_sequence != summary.last_event_sequence
        || receipt.acknowledged_sequence != summary.acknowledged_sequence
    {
        return Err(io::Error::other("运行时宿主退出账本与回执不匹配"));
    }
    let native = super::managed_process::confirmed_exit(
        &record.directory.join("native"),
        record.manifest.runtime_generation,
    )?
    .ok_or_else(|| io::Error::other("运行时宿主缺少真实进程退出回执"))?;
    let native_sha256 = digest(&serde_json::to_vec(&native).map_err(io::Error::other)?);
    if receipt.native_cleanup_sha256.as_deref() != Some(native_sha256.as_str()) {
        return Err(io::Error::other("运行时宿主原生清理回执摘要不匹配"));
    }
    Ok(Some(receipt))
}

fn read_exit_receipt(record: &RuntimeHostRecord) -> io::Result<Option<RuntimeHostExitReceipt>> {
    let bytes = match read_private_record(&record.directory.join("host-exit.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let receipt: RuntimeHostExitReceipt =
        serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if receipt.version != HOST_MANIFEST_VERSION
        || receipt.runtime_generation != record.manifest.runtime_generation
        || receipt.host_instance_id != record.manifest.host_instance_id
        || receipt.manifest_sha256 != record.manifest_sha256
    {
        return Err(io::Error::other("运行时宿主终止回执身份不匹配"));
    }
    Ok(Some(receipt))
}

struct JournalSummary {
    final_sha256: String,
    last_event_sequence: u64,
    acknowledged_sequence: u64,
    events: Vec<HostedEvent>,
    snapshot: RuntimeHostSnapshot,
}

fn verify_journal(record: &RuntimeHostRecord) -> io::Result<JournalSummary> {
    let path = record.directory.join("runtime.journal");
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > MAX_JOURNAL_BYTES {
        return Err(io::Error::other("运行时宿主账本不是受控普通文件"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
            return Err(io::Error::other("运行时宿主账本权限不正确"));
        }
    }
    let bytes = fs::read(path)?;
    let mut expected_record_sequence = 1;
    let mut expected_event_sequence = 1;
    let mut acknowledged_sequence = 0;
    let mut previous_sha256 = record.manifest_sha256.clone();
    let mut events = Vec::new();
    let mut snapshot = RuntimeHostSnapshot::default();
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_JOURNAL_RECORD_BYTES {
            return Err(io::Error::other("运行时宿主账本记录超过安全上限"));
        }
        let journal: JournalRecord = serde_json::from_slice(line).map_err(io::Error::other)?;
        if journal.sequence != expected_record_sequence
            || journal.previous_sha256 != previous_sha256
        {
            return Err(io::Error::other("运行时宿主账本哈希链不连续"));
        }
        match &journal.payload {
            JournalPayload::Event { event } => {
                if event.sequence != expected_event_sequence
                    || event.digest
                        != digest(&serde_json::to_vec(&event.event).map_err(io::Error::other)?)
                {
                    return Err(io::Error::other("运行时宿主事件账本不连续"));
                }
                expected_event_sequence = expected_event_sequence
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("运行时宿主事件序号耗尽"))?;
                events.push(event.clone());
            }
            JournalPayload::EventsAcknowledged {
                through_sequence, ..
            } => {
                let last_event_sequence = expected_event_sequence - 1;
                if *through_sequence < acknowledged_sequence
                    || *through_sequence > last_event_sequence
                {
                    return Err(io::Error::other("运行时宿主事件确认账本无效"));
                }
                acknowledged_sequence = *through_sequence;
            }
            JournalPayload::OwnerClaimed { .. }
            | JournalPayload::CommandRecorded { .. }
            | JournalPayload::CommandUpdated { .. } => {}
        }
        previous_sha256 = digest(line);
        expected_record_sequence = expected_record_sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("运行时宿主账本序号耗尽"))?;
    }
    for event in events
        .iter()
        .take_while(|event| event.sequence <= acknowledged_sequence)
    {
        snapshot.apply(&event.event);
    }
    Ok(JournalSummary {
        final_sha256: previous_sha256,
        last_event_sequence: expected_event_sequence - 1,
        acknowledged_sequence,
        events,
        snapshot,
    })
}

pub(crate) fn recover_exited_events(
    state_dir: &Path,
    generation: Uuid,
) -> io::Result<
    Option<(
        RuntimeHostExitReceipt,
        Vec<HostedEvent>,
        RuntimeHostSnapshot,
    )>,
> {
    let Some(receipt) = confirmed_exit(state_dir, generation)? else {
        return Ok(None);
    };
    let record = load_record(state_dir, generation)?
        .ok_or_else(|| io::Error::other("运行时宿主记录不存在"))?;
    let summary = verify_journal(&record)?;
    let pending_events = summary
        .events
        .into_iter()
        .filter(|event| event.sequence > receipt.acknowledged_sequence)
        .collect();
    Ok(Some((receipt, pending_events, summary.snapshot)))
}

fn record_from_manifest_path(path: &Path) -> io::Result<RuntimeHostRecord> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("运行时宿主 manifest 缺少目录"))?
        .to_owned();
    let bytes = read_private_record(path)?;
    let manifest: RuntimeHostManifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    validate_manifest(path, &manifest, &directory)?;
    Ok(RuntimeHostRecord {
        manifest,
        manifest_sha256: digest(&bytes),
        directory,
    })
}

fn remove_endpoint(endpoint: &str) {
    #[cfg(unix)]
    {
        let _ = fs::remove_file(endpoint);
    }
    #[cfg(windows)]
    let _ = endpoint;
}

fn write_exit_receipt(
    record: &RuntimeHostRecord,
    state: &Arc<Mutex<RuntimeHostState>>,
    adapter_task_terminated: bool,
    adapter_succeeded: bool,
    event_journal_completed: bool,
) -> io::Result<RuntimeHostExitReceipt> {
    let native = super::managed_process::process_completion(
        &record.directory.join("native"),
        record.manifest.runtime_generation,
    );
    let (native_process, native_cleanup_sha256) = match native.as_ref() {
        Ok(super::managed_process::ProcessCompletion::NotStarted) => {
            (NativeProcessCompletion::NotStarted, None)
        }
        Ok(super::managed_process::ProcessCompletion::Exited(receipt)) => (
            NativeProcessCompletion::Exited,
            Some(digest(
                &serde_json::to_vec(receipt).map_err(io::Error::other)?,
            )),
        ),
        Ok(super::managed_process::ProcessCompletion::Unconfirmed) | Err(_) => {
            (NativeProcessCompletion::Unconfirmed, None)
        }
    };
    let mut state = state
        .lock()
        .map_err(|_| io::Error::other("运行时宿主状态不可用"))?;
    // 退出回执绑定当前账本终点；封存后，在途 IPC 请求不能再追加 ACK 或命令记录。
    state.journal_sealed = true;
    let receipt = RuntimeHostExitReceipt {
        version: HOST_MANIFEST_VERSION,
        runtime_generation: record.manifest.runtime_generation,
        host_instance_id: record.manifest.host_instance_id,
        manifest_sha256: record.manifest_sha256.clone(),
        journal_sha256: state.journal_sha256.clone(),
        last_event_sequence: state.events.last().map_or(0, |event| event.sequence),
        acknowledged_sequence: state.acknowledged_sequence,
        native_process,
        native_cleanup_sha256,
        adapter_task_terminated,
        adapter_succeeded,
        event_journal_completed,
    };
    drop(state);
    write_new_record(
        &record.directory.join("host-exit.json"),
        &serde_json::to_vec(&receipt).map_err(io::Error::other)?,
    )?;
    native.map(|_| receipt)
}

struct HostRunResult {
    adapter: Result<(), RuntimeError>,
    journal: io::Result<()>,
}

fn start_runtime_host_ipc(
    record: &RuntimeHostRecord,
    state: Arc<Mutex<RuntimeHostState>>,
    controller: RuntimeController,
    executor: Arc<Background>,
) -> io::Result<ipc::Server> {
    let (server, _) = ipc::ServerBuilder::default()
        .with_fixed_address(record.manifest.endpoint.clone())
        .with_max_frame_bytes(MAX_RUNTIME_HOST_FRAME_BYTES)
        .with_service(RuntimeHostServiceImpl { state, controller })
        .build_and_run(executor)
        .map_err(|error| io::Error::other(format!("运行时宿主 IPC 启动失败：{error:?}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if let Err(error) =
            fs::set_permissions(&record.manifest.endpoint, fs::Permissions::from_mode(0o600))
        {
            drop(server);
            remove_endpoint(&record.manifest.endpoint);
            return Err(error);
        }
    }
    Ok(server)
}

#[cfg(test)]
pub(crate) async fn start_acknowledged_host_for_test(
    task_id: String,
    task_generation: i64,
    options: SessionOptions,
    events: Vec<RuntimeEvent>,
) -> io::Result<(
    ipc::Server,
    tokio::sync::mpsc::Receiver<RuntimeCommand>,
    RuntimeHostRecord,
)> {
    start_acknowledged_host_for_harness_for_test(
        task_id,
        task_generation,
        Harness::Codex,
        options,
        events,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn start_acknowledged_host_for_harness_for_test(
    task_id: String,
    task_generation: i64,
    harness: Harness,
    options: SessionOptions,
    events: Vec<RuntimeEvent>,
) -> io::Result<(
    ipc::Server,
    tokio::sync::mpsc::Receiver<RuntimeCommand>,
    RuntimeHostRecord,
)> {
    let record = create_record(task_id, task_generation, harness, options)?;
    let mut state = RuntimeHostState::new(
        &record.directory,
        record.manifest.clone(),
        record.manifest_sha256.clone(),
    )?;
    let acknowledged_sequence = events.len() as u64;
    for event in events {
        state.record_event(event)?;
    }
    let (controller, commands, _, _) = super::channels(record.runtime_generation());
    let executor = Arc::new(Background::new(2, |_| "cli-recovery-test-host".to_owned()));
    let server =
        start_runtime_host_ipc(&record, Arc::new(Mutex::new(state)), controller, executor)?;
    let process_id = std::process::id();
    let (process_start_time, executable) = process_identity(process_id)?;
    let ready = RuntimeHostReady {
        version: HOST_MANIFEST_VERSION,
        runtime_generation: record.runtime_generation(),
        host_instance_id: record.host_instance_id(),
        manifest_sha256: record.manifest_sha256.clone(),
        process_id,
        process_start_time,
        executable,
    };
    write_new_record(
        &record.directory.join("ready.json"),
        &serde_json::to_vec(&ready).map_err(io::Error::other)?,
    )?;
    let mut client = connect_verified(record.clone()).await?;
    client.claim_owner(0).await?;
    client.acknowledge_events(acknowledged_sequence).await?;
    Ok((server, commands, record))
}

fn run_connection(
    record: &RuntimeHostRecord,
    state: Arc<Mutex<RuntimeHostState>>,
    connection: super::RuntimeConnection,
) -> io::Result<HostRunResult> {
    let super::RuntimeConnection {
        controller,
        mut events,
        task,
    } = connection;
    let executor = Arc::new(Background::new(2, |_| "cli-runtime-host".to_owned()));
    let server = start_runtime_host_ipc(record, state.clone(), controller, executor.clone())?;
    let ready_result = (|| {
        let process_id = std::process::id();
        let (process_start_time, executable) = process_identity(process_id)?;
        let ready = RuntimeHostReady {
            version: HOST_MANIFEST_VERSION,
            runtime_generation: record.manifest.runtime_generation,
            host_instance_id: record.manifest.host_instance_id,
            manifest_sha256: record.manifest_sha256.clone(),
            process_id,
            process_start_time,
            executable,
        };
        write_new_record(
            &record.directory.join("ready.json"),
            &serde_json::to_vec(&ready).map_err(io::Error::other)?,
        )
    })();
    if let Err(error) = ready_result {
        drop(server);
        remove_endpoint(&record.manifest.endpoint);
        return Err(error);
    }

    let (journal_done_tx, journal_done_rx) = async_channel::bounded(1);
    let event_state = state;
    executor
        .spawn(async move {
            let result = loop {
                match events.recv().await {
                    Some(event) => {
                        let result = event_state
                            .lock()
                            .map_err(|_| io::Error::other("运行时宿主状态不可用"))
                            .and_then(|mut state| state.record_event(event));
                        if let Err(error) = result {
                            break Err(error);
                        }
                    }
                    None => break Ok(()),
                }
            };
            let _ = journal_done_tx.send(result).await;
        })
        .detach();

    let outcome = warpui_core::r#async::block_on(async move {
        let adapter = task;
        let journal = journal_done_rx.recv();
        futures::pin_mut!(adapter, journal);
        match futures::future::select(adapter, journal).await {
            futures::future::Either::Left((adapter, journal)) => {
                let journal = journal
                    .await
                    .unwrap_or_else(|_| Err(io::Error::other("运行时宿主事件账本任务提前退出")));
                HostRunResult { adapter, journal }
            }
            futures::future::Either::Right((journal, adapter)) => {
                let journal = journal
                    .unwrap_or_else(|_| Err(io::Error::other("运行时宿主事件账本任务提前退出")));
                match journal {
                    Ok(()) => HostRunResult {
                        adapter: adapter.await,
                        journal: Ok(()),
                    },
                    Err(error) => {
                        // 账本失败后继续运行会产生无法持久恢复的原生事件；取消 adapter
                        // 让监督控制管道清理真实 CLI，并以未确认退出安全收敛。
                        drop(adapter);
                        HostRunResult {
                            adapter: Err(RuntimeError::Protocol(
                                "runtime_host.event_journal_failed".to_owned(),
                            )),
                            journal: Err(error),
                        }
                    }
                }
            }
        }
    });
    drop(server);
    remove_endpoint(&record.manifest.endpoint);
    Ok(outcome)
}

pub(crate) fn run_worker(path: &Path) -> io::Result<()> {
    let record = record_from_manifest_path(path)?;
    let native_state_dir = record.directory.join("native");
    create_private_directory(&native_state_dir)?;
    let state = Arc::new(Mutex::new(RuntimeHostState::new(
        &record.directory,
        record.manifest.clone(),
        record.manifest_sha256.clone(),
    )?));
    let mut options = record.manifest.options.clone();
    options.state_dir.clone_from(&native_state_dir);
    let connection = match record.manifest.harness {
        Harness::Codex => super::codex::connect(options),
        // 原生进程状态独立落盘，附件仍属于已核验 manifest 的应用数据目录。
        Harness::Claude => super::claude::connect_with_attachment_store(
            options,
            record
                .manifest
                .options
                .state_dir
                .join("local-cli-attachments"),
        ),
        Harness::Grok => super::grok::connect_with_profile_state_dir(
            options,
            record.manifest.options.state_dir.clone(),
        ),
        Harness::Oz | Harness::OpenCode | Harness::Gemini | Harness::Unknown => {
            return Err(io::Error::other("运行时宿主不支持此 CLI"));
        }
    }
    .map_err(io::Error::other);
    let connection = match connection {
        Ok(connection) => connection,
        Err(error) => {
            write_exit_receipt(&record, &state, false, false, true)?;
            return Err(error);
        }
    };
    let outcome = match run_connection(&record, state.clone(), connection) {
        Ok(outcome) => outcome,
        Err(error) => {
            write_exit_receipt(&record, &state, false, false, true)?;
            return Err(error);
        }
    };

    let exit_result = write_exit_receipt(
        &record,
        &state,
        true,
        outcome.adapter.is_ok(),
        outcome.journal.is_ok(),
    );
    outcome.journal?;
    exit_result?;
    outcome.adapter.map_err(io::Error::other)
}

#[cfg(test)]
#[path = "runtime_host_tests.rs"]
mod tests;
