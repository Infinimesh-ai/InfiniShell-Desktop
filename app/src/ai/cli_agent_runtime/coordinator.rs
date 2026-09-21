//! 托管 CLI 的本地协调器：先提交状态与输入记录，再驱动协议；重启仅加载记录。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Aborted};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt, pin_mut, select};

#[path = "coordinator_tools.rs"]
mod tools;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, ModelSpawner, SingletonEntity};

use super::local_tools::NativeLocalToolRequest;
use super::{
    InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand, RuntimeConnection,
    RuntimeController, RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome,
};
use crate::ai::local_cli_mailbox::{
    acknowledge_runtime_message, dispatch_prepared_result_if_current, send_prepared_result,
};
use crate::persistence::ModelEvent;
use crate::persistence::local_cli_tasks::{
    LocalCliEnqueueOutcome, acknowledge_message, checkpoint_task, checkpoint_task_with_message,
    enqueue_message, enqueue_task_result, grok_mailbox_digest, grok_mailbox_origin_matches,
    grok_result_history_matches, load_messages, load_task_generations, load_task_messages,
    load_tasks,
};
use crate::persistence::model::{
    LocalCliMessage, LocalCliMessageState, LocalCliReceiptKind, LocalCliTask, LocalCliTaskState,
};
use crate::terminal::cli_agent::{CLIAgent, cli_agent_update_in_progress};

#[derive(Clone)]
pub(crate) struct ManagedTaskEndpoint {
    pub task_id: String,
    pub generation: i64,
    pub harness: Harness,
    pub runtime_generation: Uuid,
    pub active_turn_id: Option<String>,
    commands: mpsc::Sender<ManagedRequest>,
}

impl ManagedTaskEndpoint {
    /// 邮箱落盘期间任务可能已经换代；必须在协议写入前重新核对。
    pub(crate) async fn send_result(
        &self,
        command: RuntimeCommand,
        message: LocalCliMessage,
    ) -> Result<(), String> {
        let mut prepared = self.prepare_send(command).await?;
        prepared.request.prepared_result = Some(message);
        prepared
            .commit()
            .await
            .map_err(|_| crate::t!("cli-agent-status-disconnected"))?
    }

    /// 先保留队列容量，调用方可以在同一应用回调中验证父轮并同步提交。
    pub(crate) async fn prepare_send(
        &self,
        command: RuntimeCommand,
    ) -> Result<PreparedManagedSend, String> {
        if command.generation != self.runtime_generation {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        let (reply, receiver) = oneshot::channel();
        let permit = self
            .commands
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
        Ok(PreparedManagedSend {
            permit,
            receiver,
            request: ManagedRequest {
                expected_generation: self.generation,
                from_mailbox: true,
                prepared_result: None,
                message_id: command.message_id,
                action: command.action,
                reply,
            },
        })
    }
}

pub(crate) struct PreparedManagedSend {
    permit: mpsc::OwnedPermit<ManagedRequest>,
    request: ManagedRequest,
    receiver: oneshot::Receiver<Result<(), String>>,
}

impl PreparedManagedSend {
    pub(crate) fn commit(self) -> oneshot::Receiver<Result<(), String>> {
        self.permit.send(self.request);
        self.receiver
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ManagedApproval {
    pub approval_id: String,
    pub turn_id: String,
    pub method: String,
    pub details: serde_json::Value,
}

#[derive(Clone, Debug)]
pub(crate) struct ManagedTaskSnapshot {
    pub task: LocalCliTask,
    pub ready: bool,
    pub connected: bool,
    pub active_turn_id: Option<String>,
    pub approvals: Vec<ManagedApproval>,
    pub output: String,
    pub error: Option<String>,
}

/// 回执仍属于提交时的代数；只有原生 started 才把下一轮升级为新的运行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ClaudePendingInput {
    message_id: Uuid,
    submission_generation: i64,
}

fn claude_pending_input(task: &LocalCliTask) -> Result<Option<ClaudePendingInput>, String> {
    if task.harness != "claude" {
        return Ok(None);
    }
    let config: serde_json::Value = serde_json::from_str(&task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    config
        .get("claude_pending_input")
        .filter(|value| !value.is_null())
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))
        })
        .transpose()
}

pub(crate) fn claude_input_pending(task: &LocalCliTask) -> bool {
    // 配置无法解释时也不能开放第二个输入，避免失去跨代回执关联。
    !claude_pending_input(task).is_ok_and(|pending| pending.is_none())
}

fn set_claude_pending_input(
    task: &mut LocalCliTask,
    pending: Option<ClaudePendingInput>,
) -> Result<(), String> {
    let mut config: serde_json::Value = serde_json::from_str(&task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
    if let Some(pending) = pending {
        object.insert("claude_pending_input".into(), json!(pending));
    } else {
        object.remove("claude_pending_input");
    }
    task.config_json = config.to_string();
    Ok(())
}

// 原生当前请求加最多 32 条本地等待输入；不删除旧关联后把迟到 ACK 当成新输入。
const MAX_GROK_PENDING_INPUTS: usize = 33;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct GrokInputLink {
    message_id: Uuid,
    submission_generation: i64,
    runtime_generation: Uuid,
    native_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mailbox_sha256: Option<String>,
}

#[derive(Default)]
struct GrokInputLinks {
    pending: Vec<GrokInputLink>,
    current: Option<GrokInputLink>,
}

fn grok_input_links(task: &LocalCliTask) -> Result<GrokInputLinks, String> {
    if task.harness != "grok" {
        return Ok(GrokInputLinks::default());
    }
    let config: serde_json::Value = serde_json::from_str(&task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    let pending: Vec<GrokInputLink> = config
        .get("grok_pending_inputs")
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?
        .unwrap_or_default();
    let current: Option<GrokInputLink> = config
        .get("grok_current_input")
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    let runtime_generation = config
        .get("runtime_generation")
        .and_then(serde_json::Value::as_str)
        .and_then(|generation| Uuid::parse_str(generation).ok())
        .filter(|generation| !generation.is_nil());
    let mut messages = HashSet::new();
    let mut turns = HashSet::new();
    if pending.len() + usize::from(current.is_some()) > MAX_GROK_PENDING_INPUTS
        || current
            .as_ref()
            .is_some_and(|input| input.native_turn_id.is_none())
        || pending.iter().chain(current.iter()).any(|input| {
            input.submission_generation < 1
                || input.submission_generation > task.generation
                || Some(input.runtime_generation) != runtime_generation
                || !messages.insert(input.message_id)
                || input.mailbox_sha256.as_ref().is_some_and(|digest| {
                    digest.len() != 64
                        || !digest
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                })
                || input.native_turn_id.as_ref().is_some_and(|turn| {
                    turn.trim().is_empty()
                        || turn.len() > 4096
                        || turn.chars().any(char::is_control)
                        || !turns.insert(turn.clone())
                })
        })
    {
        return Err(crate::t!("cli-agent-task-invalid-launch"));
    }
    Ok(GrokInputLinks { pending, current })
}

fn set_grok_input_links(task: &mut LocalCliTask, links: &GrokInputLinks) -> Result<(), String> {
    let mut config: serde_json::Value = serde_json::from_str(&task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    let object = config
        .as_object_mut()
        .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
    object.insert("grok_pending_inputs".into(), json!(links.pending));
    if let Some(current) = &links.current {
        object.insert("grok_current_input".into(), json!(current));
    } else {
        object.remove("grok_current_input");
    }
    task.config_json = config.to_string();
    Ok(())
}

/// 关联必须指向原提交代的真实消息；待启动回合另外核对已提交的原生 ACK。
async fn verify_grok_input_message(
    sender: &SyncSender<ModelEvent>,
    task: &LocalCliTask,
    input: &GrokInputLink,
    require_ack: bool,
) -> Result<LocalCliMessageState, String> {
    let messages = load_messages(sender, task.task_id.clone(), input.submission_generation)?
        .await
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
    let message = messages
        .iter()
        .find(|message| message.message_id == input.message_id.to_string())
        .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
    if task.version != 1
        || message.version != 1
        || message.recipient_task_id != task.task_id
        || message.recipient_generation != input.submission_generation
        || !matches!(
            message.state,
            LocalCliMessageState::Sent | LocalCliMessageState::Acknowledged
        )
        || (message.state == LocalCliMessageState::Acknowledged
            && (message.receipt_kind != Some(LocalCliReceiptKind::NativeProtocol)
                || input.native_turn_id.is_none()))
        || (message.state == LocalCliMessageState::Sent && message.receipt_kind.is_some())
        || (require_ack && message.state != LocalCliMessageState::Acknowledged)
    {
        return Err(crate::t!("cli-agent-task-invalid-launch"));
    }
    let original = if input.submission_generation == task.generation {
        task.clone()
    } else {
        load_task_generations(sender, task.task_id.clone())?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
            .into_iter()
            .find(|original| original.generation == input.submission_generation)
            .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?
    };
    let original_config: serde_json::Value = serde_json::from_str(&original.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    if original.version != 1
        || original.harness != "grok"
        || task.native_session_id.as_ref().is_none_or(|session| {
            session.trim().is_empty()
                || session.len() > 4096
                || session.chars().any(char::is_control)
        })
        || original.native_session_id != task.native_session_id
        || original_config["runtime_generation"]
            .as_str()
            .and_then(|runtime| Uuid::parse_str(runtime).ok())
            != Some(input.runtime_generation)
    {
        return Err(crate::t!("cli-agent-task-invalid-launch"));
    }
    if let Some(digest) = &input.mailbox_sha256 {
        let source = load_task_generations(sender, message.sender_task_id.clone())?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
            .into_iter()
            .find(|source| source.generation == message.sender_generation)
            .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
        if !grok_mailbox_origin_matches(message, &source, &original, digest) {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
    } else if message.sender_task_id != task.task_id
        || message.sender_generation != input.submission_generation
        || message.subject != "user_input"
        || !matches!(
            serde_json::from_str::<RuntimeAction>(&message.body),
            Ok(RuntimeAction::Submit { .. })
        )
    {
        return Err(crate::t!("cli-agent-task-invalid-launch"));
    }
    Ok(message.state)
}

async fn verify_grok_pending_inputs(
    sender: &SyncSender<ModelEvent>,
    task: &LocalCliTask,
    links: &GrokInputLinks,
    runtime_generation: Uuid,
) -> Result<(), String> {
    for input in &links.pending {
        if input.runtime_generation != runtime_generation {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        verify_grok_input_message(sender, task, input, false).await?;
    }
    Ok(())
}

pub(crate) enum LocalCLITaskCoordinatorEvent {
    Changed,
    /// 相关消息已尝试持久提交；界面重新读取状态，不从此通知推断原生接收。
    MessagesChanged {
        sender_task_id: String,
        recipient_task_id: String,
    },
    /// 此时相应任务状态、结果或消息确认已经收到 SQLite 提交确认。
    Runtime {
        task: LocalCliTask,
        event: RuntimeEvent,
    },
    ResultReady {
        task: LocalCliTask,
        message: LocalCliMessage,
    },
    /// 已入队的普通子任务消息；Oz 历史桥自行领取，不能冒充原生接收。
    ParentMessageReady {
        task: LocalCliTask,
        message: LocalCliMessage,
    },
}

struct ManagedTaskEntry {
    token: Uuid,
    snapshot: ManagedTaskSnapshot,
    commands: mpsc::Sender<ManagedRequest>,
    pending_tool_calls: HashSet<String>,
}

struct ManagedRequest {
    expected_generation: i64,
    from_mailbox: bool,
    prepared_result: Option<LocalCliMessage>,
    message_id: Uuid,
    action: RuntimeAction,
    reply: oneshot::Sender<Result<(), String>>,
}

struct RecoveredHost {
    snapshot: ManagedTaskSnapshot,
    options: SessionOptions,
    connection: RuntimeConnection,
    initial_input: Option<(Uuid, Vec<InputContent>)>,
    pending_local_tools: Vec<NativeLocalToolRequest>,
    protocol_state: RecoveryProtocolState,
    start_mode: ManagedStartMode,
}

struct RecoveryBatch {
    records: Vec<LocalCliTask>,
    hosts: Vec<RecoveredHost>,
    results: Vec<(LocalCliTask, LocalCliMessage)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedStartMode {
    Fresh,
    Reattached,
    RetryNotStarted,
}

#[derive(Default)]
struct RecoveryProtocolState {
    accepted_turns: HashSet<String>,
    finished_turns: HashSet<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RuntimeHostRecoveryCheckpoint {
    runtime_generation: Uuid,
    host_instance_id: Uuid,
    manifest_sha256: String,
    recovered_through: u64,
}

#[derive(Clone, Debug)]
struct RuntimeHostRecoveryIdentity {
    runtime_generation: Uuid,
    host_instance_id: Uuid,
    manifest_sha256: String,
}

impl From<&super::runtime_host::RuntimeHostRecord> for RuntimeHostRecoveryIdentity {
    fn from(record: &super::runtime_host::RuntimeHostRecord) -> Self {
        Self {
            runtime_generation: record.runtime_generation(),
            host_instance_id: record.host_instance_id(),
            manifest_sha256: record.manifest_sha256().to_owned(),
        }
    }
}

pub(crate) struct LocalCLITaskCoordinator {
    sender: Option<SyncSender<ModelEvent>>,
    entries: HashMap<String, ManagedTaskEntry>,
    restored: Vec<LocalCliTask>,
    recovery_error: Option<String>,
    recovery_owner: Option<Uuid>,
}

impl Entity for LocalCLITaskCoordinator {
    type Event = LocalCLITaskCoordinatorEvent;
}

impl SingletonEntity for LocalCLITaskCoordinator {}

impl LocalCLITaskCoordinator {
    pub(crate) fn new(sender: Option<SyncSender<ModelEvent>>) -> Self {
        Self {
            sender,
            entries: HashMap::new(),
            restored: Vec::new(),
            recovery_error: None,
            recovery_owner: None,
        }
    }

    pub(crate) fn sender(&self) -> Option<SyncSender<ModelEvent>> {
        self.sender.clone()
    }

    #[cfg(test)]
    pub(crate) fn with_restored_for_test(records: Vec<LocalCliTask>) -> Self {
        let mut coordinator = Self::new(None);
        coordinator.restored = records;
        coordinator
    }

    pub(crate) fn endpoint(&self, task_id: &str) -> Option<ManagedTaskEndpoint> {
        let entry = self.entries.get(task_id)?;
        let snapshot = &entry.snapshot;
        if !snapshot.ready || !snapshot.connected || !snapshot.task.state.is_active() {
            return None;
        }
        Some(ManagedTaskEndpoint {
            task_id: task_id.to_owned(),
            generation: snapshot.task.generation,
            harness: Harness::parse_orchestration_harness(&snapshot.task.harness)?,
            runtime_generation: entry.token,
            active_turn_id: snapshot.active_turn_id.clone(),
            commands: entry.commands.clone(),
        })
    }

    fn result_endpoint(&self, task_id: &str) -> Option<ManagedTaskEndpoint> {
        let entry = self.entries.get(task_id)?;
        let snapshot = &entry.snapshot;
        if snapshot.task.harness != "grok" {
            return self.endpoint(task_id);
        }
        if !grok_result_slot_available(snapshot).ok()? {
            return None;
        }
        Some(ManagedTaskEndpoint {
            task_id: task_id.to_owned(),
            generation: snapshot.task.generation,
            harness: Harness::Grok,
            runtime_generation: entry.token,
            active_turn_id: None,
            commands: entry.commands.clone(),
        })
    }

    pub(crate) fn snapshot(&self, task_id: &str) -> Option<&ManagedTaskSnapshot> {
        self.entries.get(task_id).map(|entry| &entry.snapshot)
    }

    pub(crate) fn snapshots(&self) -> impl Iterator<Item = &ManagedTaskSnapshot> {
        self.entries.values().map(|entry| &entry.snapshot)
    }

    pub(crate) fn restored_tasks(&self) -> &[LocalCliTask] {
        &self.restored
    }

    pub(crate) fn recovery_error(&self) -> Option<&str> {
        self.recovery_error.as_deref()
    }

    /// 启动时恢复持久记录，重关联仍存活的宿主，并离线收敛已退出宿主的未确认事件。
    pub(crate) fn refresh_records(&mut self, ctx: &mut ModelContext<Self>) {
        if self.recovery_owner.is_some() {
            return;
        }
        let Some(sender) = self.sender.clone() else {
            return;
        };
        let recovery_owner = Uuid::new_v4();
        self.recovery_owner = Some(recovery_owner);
        let owned = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.snapshot.connected)
            .map(|(task_id, entry)| (task_id.clone(), entry.token))
            .collect();
        ctx.spawn(
            async move {
                let records = load_tasks(&sender, false)?
                    .await
                    .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
                recover_runtime_hosts(&sender, records, &owned).await
            },
            move |model, result, ctx| {
                if model.recovery_owner != Some(recovery_owner) {
                    return;
                }
                model.recovery_owner = None;
                match result {
                    Ok(batch) => {
                        model.restored = batch.records;
                        model.recovery_error = None;
                        for recovered in batch.hosts {
                            model.install_recovered_host(recovered, ctx);
                        }
                        for (task, message) in batch.results {
                            model.deliver_result(message.clone(), ctx);
                            ctx.emit(LocalCLITaskCoordinatorEvent::ResultReady { task, message });
                        }
                    }
                    Err(error) => model.recovery_error = Some(error),
                }
                ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
                ctx.notify();
            },
        );
    }

    fn install_recovered_host(&mut self, recovered: RecoveredHost, ctx: &mut ModelContext<Self>) {
        let Some(sender) = self.sender.clone() else {
            return;
        };
        let token = recovered.options.generation;
        let task_id = recovered.snapshot.task.task_id.clone();
        if self
            .entries
            .get(&task_id)
            .is_some_and(|entry| entry.snapshot.connected)
        {
            return;
        }
        let (commands, receiver) = mpsc::channel(32);
        let pending_tool_calls = recovered
            .pending_local_tools
            .iter()
            .map(|request| request.call_id.clone())
            .collect();
        self.entries.insert(
            task_id.clone(),
            ManagedTaskEntry {
                token,
                snapshot: recovered.snapshot.clone(),
                commands,
                pending_tool_calls,
            },
        );
        let spawner = ctx.spawner();
        ctx.spawn(
            async move {
                run_managed_task(
                    recovered.snapshot,
                    None,
                    true,
                    recovered.initial_input,
                    token,
                    sender,
                    recovered.options,
                    recovered.connection,
                    receiver,
                    spawner,
                    recovered.pending_local_tools,
                    recovered.protocol_state,
                    recovered.start_mode,
                )
                .await
            },
            move |model, result, ctx| {
                let mut recover_exited_events = false;
                if let Some(entry) = model.entries.get_mut(&task_id)
                    && entry.token == token
                {
                    entry.snapshot.connected = false;
                    entry.snapshot.ready = false;
                    entry.snapshot.approvals.clear();
                    entry.pending_tool_calls.clear();
                    if let Err(error) = result {
                        entry.snapshot.error = Some(error);
                    }
                    recover_exited_events =
                        serde_json::from_str::<serde_json::Value>(&entry.snapshot.task.config_json)
                            .ok()
                            .and_then(|config| {
                                config["runtime_host_start_state"]
                                    .as_str()
                                    .map(str::to_owned)
                            })
                            .as_deref()
                            == Some("exited_pending_recovery");
                    ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
                    ctx.notify();
                }
                if recover_exited_events {
                    model.refresh_records(ctx);
                }
            },
        );
    }

    /// 同一应用实例中的连接只能重新打开；只有无活跃连接时才允许启动历史恢复。
    pub(crate) fn start(
        &mut self,
        task: LocalCliTask,
        options: SessionOptions,
        expected_generation: Option<i64>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        self.start_inner(task, options, expected_generation, false, None, ctx)
    }

    /// 编排已提交初始记录时使用；启动前再次核对该记录，避免重复执行。
    pub(crate) fn start_prepared(
        &mut self,
        task: LocalCliTask,
        options: SessionOptions,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        self.start_inner(task, options, None, true, None, ctx)
    }

    pub(crate) fn start_prepared_with_input(
        &mut self,
        task: LocalCliTask,
        options: SessionOptions,
        input: Vec<InputContent>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        self.start_inner(
            task,
            options,
            None,
            true,
            Some((Uuid::new_v4(), input)),
            ctx,
        )
    }

    pub(crate) fn start_with_input(
        &mut self,
        task: LocalCliTask,
        options: SessionOptions,
        expected_generation: Option<i64>,
        message_id: Uuid,
        input: Vec<InputContent>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        self.start_inner(
            task,
            options,
            expected_generation,
            false,
            Some((message_id, input)),
            ctx,
        )
    }

    fn start_inner(
        &mut self,
        task: LocalCliTask,
        mut options: SessionOptions,
        expected_generation: Option<i64>,
        prepared: bool,
        initial_input: Option<(Uuid, Vec<InputContent>)>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        if !FeatureFlag::LocalCLIManagedTasks.is_enabled() {
            return Err(crate::t!("cli-agent-managed-version-unavailable"));
        }
        // 启动恢复持有一次应用内所有权扫描；扫描结束前不允许新连接与其争抢 host epoch。
        if self.recovery_owner.is_some() {
            return Err(crate::t!("cli-agent-task-already-running"));
        }
        if self
            .entries
            .get(&task.task_id)
            .is_some_and(|entry| entry.snapshot.connected)
        {
            return Err(crate::t!("cli-agent-task-already-running"));
        }
        let harness = Harness::parse_orchestration_harness(&task.harness);
        // 新建、历史继续和子任务都经过此处；升级未收敛前不创建原生连接。
        if let Some(agent) = harness.and_then(CLIAgent::from_harness)
            && cli_agent_update_in_progress(agent, ctx)
        {
            return Err(crate::t!(
                "settings-cli-updates-launch-blocked",
                agent = agent.display_name()
            ));
        }
        if task.state != LocalCliTaskState::Queued
            || task.revision != 0
            || options.cwd.to_string_lossy() != task.working_directory
            || !options.cwd.is_dir()
        {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        options.state_dir = super::current_state_dir();
        match &options.target {
            SessionTarget::New
                if expected_generation.is_some() || task.native_session_id.is_some() =>
            {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            SessionTarget::Resume { native_session_id }
                if task.native_session_id.as_ref() != Some(native_session_id)
                    || expected_generation.is_none() =>
            {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            SessionTarget::New | SessionTarget::Resume { .. } => {}
        }
        match &options.target {
            SessionTarget::New => {
                if let Some((_, input)) = &initial_input {
                    options.selected_skills = input
                        .iter()
                        .filter_map(|content| match content {
                            InputContent::Skill { name, path } => {
                                Some(super::local_skills::SelectedLocalSkill {
                                    name: name.clone(),
                                    path: path.clone(),
                                })
                            }
                            InputContent::Text(_) | InputContent::LocalImage(_) => None,
                        })
                        .collect();
                }
            }
            SessionTarget::Resume { .. } => {
                let config: serde_json::Value = serde_json::from_str(&task.config_json)
                    .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
                options.selected_skills = match config.get("selected_skills") {
                    Some(value) => serde_json::from_value(value.clone())
                        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?,
                    None => Vec::new(),
                };
                options.permission_ceiling = match config.get("permission_ceiling") {
                    Some(value) => serde_json::from_value(value.clone())
                        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?,
                    None => None,
                };
                options.claude_profile = serde_json::from_value(
                    config.get("claude_profile").cloned().unwrap_or_default(),
                )
                .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
                options.grok_profile =
                    serde_json::from_value(config.get("grok_profile").cloned().unwrap_or_default())
                        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
                if options.permission_policy == PermissionPolicy::ClaudeRestrictedFilesV1
                    && options.claude_profile.is_none()
                {
                    return Err(crate::t!("cli-agent-task-invalid-launch"));
                }
            }
        }
        let sender = self
            .sender
            .clone()
            .ok_or_else(|| crate::t!("cli-agent-task-save-failed"))?;
        let validation = match harness {
            Some(Harness::Codex) => super::codex::connect(options.clone()),
            Some(Harness::Claude) => super::claude::connect(options.clone()),
            Some(Harness::Grok) => {
                if (task.parent_task_id.is_some() || task.parent_generation.is_some())
                    && (!matches!(
                        options.permission_policy,
                        PermissionPolicy::GrokRestrictedReadV1
                            | PermissionPolicy::GrokRestrictedFilesV1
                    ) || options.permission_ceiling.is_none())
                {
                    return Err(crate::t!("cli-agent-grok-managed-unverified"));
                }
                super::grok::connect(options.clone())
            }
            Some(Harness::Oz | Harness::Gemini | Harness::OpenCode | Harness::Unknown) | None => {
                return Err(crate::t!("cli-agent-managed-version-unavailable"));
            }
        }
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        drop(validation);
        let token = options.generation;
        let task_id = task.task_id.clone();
        let connection = super::runtime_host::spawn_connection(
            task_id.clone(),
            task.generation,
            harness.expect("已验证的托管 CLI 必须存在"),
            options.clone(),
        );
        let (commands, receiver) = mpsc::channel(32);
        let snapshot = ManagedTaskSnapshot {
            output: String::new(),
            task,
            ready: false,
            connected: true,
            active_turn_id: None,
            approvals: Vec::new(),
            error: None,
        };
        self.entries.insert(
            task_id.clone(),
            ManagedTaskEntry {
                token,
                snapshot: snapshot.clone(),
                commands,
                pending_tool_calls: HashSet::new(),
            },
        );
        let spawner = ctx.spawner();
        ctx.spawn(
            async move {
                run_managed_task(
                    snapshot,
                    expected_generation,
                    prepared,
                    initial_input,
                    token,
                    sender,
                    options,
                    connection,
                    receiver,
                    spawner,
                    Vec::new(),
                    RecoveryProtocolState::default(),
                    ManagedStartMode::Fresh,
                )
                .await
            },
            move |model, result, ctx| {
                let mut recover_exited_events = false;
                if let Some(entry) = model.entries.get_mut(&task_id)
                    && entry.token == token
                {
                    entry.snapshot.connected = false;
                    entry.snapshot.ready = false;
                    entry.snapshot.approvals.clear();
                    entry.pending_tool_calls.clear();
                    if let Err(error) = result {
                        entry.snapshot.error = Some(error);
                    }
                    recover_exited_events =
                        serde_json::from_str::<serde_json::Value>(&entry.snapshot.task.config_json)
                            .ok()
                            .and_then(|config| {
                                config["runtime_host_start_state"]
                                    .as_str()
                                    .map(str::to_owned)
                            })
                            .as_deref()
                            == Some("exited_pending_recovery");
                    ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
                    ctx.notify();
                }
                if recover_exited_events {
                    model.refresh_records(ctx);
                }
            },
        );
        ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
        ctx.notify();
        Ok(())
    }

    /// 返回值只确认控制请求已入应用队列；原生接收确认由 Runtime 事件单独报告。
    pub(crate) fn request(
        &mut self,
        task_id: &str,
        message_id: Uuid,
        action: RuntimeAction,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        let entry = self
            .entries
            .get(task_id)
            .filter(|entry| {
                entry.snapshot.connected
                    && (entry.snapshot.ready || matches!(action, RuntimeAction::Shutdown))
            })
            .ok_or_else(|| crate::t!("cli-agent-status-disconnected"))?;
        let (reply, receiver) = oneshot::channel();
        entry
            .commands
            .try_send(ManagedRequest {
                expected_generation: entry.snapshot.task.generation,
                from_mailbox: false,
                prepared_result: None,
                message_id,
                action,
                reply,
            })
            .map_err(|_| crate::t!("cli-agent-input-still-sending"))?;
        Ok(receiver)
    }

    pub(crate) fn request_for_generation(
        &mut self,
        task_id: &str,
        expected_generation: i64,
        message_id: Uuid,
        action: RuntimeAction,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        if self
            .entries
            .get(task_id)
            .is_none_or(|entry| entry.snapshot.task.generation != expected_generation)
        {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        self.request(task_id, message_id, action)
    }

    /// 只向当前活跃连接首次投递已提交的子任务结果；恢复记录不会自动启动父任务。
    fn deliver_result(&mut self, message: LocalCliMessage, ctx: &mut ModelContext<Self>) {
        if self.sender.is_none() {
            return;
        }
        let Some(endpoint) = self.result_endpoint(&message.recipient_task_id) else {
            return;
        };
        if (endpoint.generation != message.recipient_generation
            && endpoint.harness != Harness::Grok)
            || self
                .entries
                .get(&endpoint.task_id)
                .is_some_and(|entry| claude_input_pending(&entry.snapshot.task))
        {
            return;
        }
        let task_id = endpoint.task_id.clone();
        let generation = endpoint.generation;
        let token = endpoint.runtime_generation;
        let sender_task_id = message.sender_task_id.clone();
        let recipient_task_id = message.recipient_task_id.clone();
        ctx.spawn(
            async move { send_prepared_result(endpoint, message).await },
            move |model, result, ctx| {
                if let Err(error) = result
                    && let Some(entry) = model.entries.get_mut(&task_id)
                    && entry.token == token
                    && entry.snapshot.task.generation == generation
                {
                    entry.snapshot.error = Some(error);
                }
                ctx.emit(LocalCLITaskCoordinatorEvent::MessagesChanged {
                    sender_task_id,
                    recipient_task_id,
                });
                ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
                ctx.notify();
            },
        );
    }

    /// 仅重试从未领取的消息；原生已接收或交付不确定的消息不能自动重发。
    fn retry_pending_results(&mut self, task_id: &str, ctx: &mut ModelContext<Self>) {
        let Some(endpoint) = self.result_endpoint(task_id) else {
            return;
        };
        if self
            .entries
            .get(task_id)
            .is_some_and(|entry| claude_input_pending(&entry.snapshot.task))
        {
            return;
        }
        let Some(sender) = self.sender.clone() else {
            return;
        };
        let task_id = endpoint.task_id.clone();
        let error_task_id = task_id.clone();
        let generation = endpoint.generation;
        let token = endpoint.runtime_generation;
        let current = self
            .entries
            .get(&task_id)
            .map(|entry| entry.snapshot.task.clone());
        ctx.spawn(
            async move {
                let current = current.ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
                let messages = if current.harness == "grok" {
                    load_task_messages(&sender, task_id.clone())?
                } else {
                    load_messages(&sender, task_id.clone(), generation)?
                }
                .await
                .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
                let generations = if current.harness == "grok" {
                    load_task_generations(&sender, task_id.clone())?
                        .await
                        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
                } else {
                    Vec::new()
                };
                let mut candidate = None;
                let mut ordinary = Vec::new();
                for message in messages {
                    if message.recipient_task_id != task_id
                        || message.state != LocalCliMessageState::Queued
                    {
                        continue;
                    }
                    if message.subject != "local_task_result" {
                        if message.recipient_generation == generation
                            && !matches!(
                                message.subject.as_str(),
                                "native_tool_call" | "native_tool_result"
                            )
                        {
                            ordinary.push(message);
                        }
                        continue;
                    }
                    if current.harness == "grok" {
                        if !grok_result_history_matches(&message, &generations, &current) {
                            continue;
                        }
                        let original = generations
                            .iter()
                            .find(|task| task.generation == message.recipient_generation)
                            .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
                        let sources =
                            load_task_generations(&sender, message.sender_task_id.clone())?
                                .await
                                .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
                        let digest = grok_mailbox_digest(&message)
                            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
                        if !sources.iter().any(|source| {
                            grok_mailbox_origin_matches(&message, source, original, &digest)
                        }) {
                            continue;
                        }
                    } else if message.recipient_generation != generation {
                        continue;
                    }
                    if candidate.is_none() {
                        candidate = Some(message);
                    }
                }
                let mut changes = Vec::new();
                for message in ordinary {
                    let source = load_task_generations(&sender, message.sender_task_id.clone())?
                        .await
                        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
                        .into_iter()
                        .find(|task| task.generation == message.sender_generation)
                        .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
                    tools::dispatch_managed_message(
                        &sender,
                        endpoint.clone(),
                        source,
                        current.clone(),
                        message.clone(),
                    )
                    .await?;
                    changes.push((message.sender_task_id, message.recipient_task_id));
                }
                Ok::<_, String>((task_id, candidate, changes))
            },
            move |model, result, ctx| {
                if result.is_err()
                    && let Some(entry) = model.entries.get_mut(&error_task_id)
                    && entry.token == token
                    && entry.snapshot.task.generation == generation
                {
                    entry.snapshot.error = Some(crate::t!("cli-agent-message-delivery-failed"));
                    ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
                    ctx.notify();
                }
                if let Ok((task_id, candidate, changes)) = result {
                    for (sender_task_id, recipient_task_id) in changes {
                        ctx.emit(LocalCLITaskCoordinatorEvent::MessagesChanged {
                            sender_task_id,
                            recipient_task_id,
                        });
                    }
                    if let Some(message) = candidate
                        && model.entries.get(&task_id).is_some_and(|entry| {
                            entry.token == token && entry.snapshot.task.generation == generation
                        })
                    {
                        model.deliver_result(message, ctx);
                    }
                }
            },
        );
    }

    fn publish(
        &mut self,
        token: Uuid,
        snapshot: ManagedTaskSnapshot,
        event: Option<RuntimeEvent>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(entry) = self
            .entries
            .get_mut(&snapshot.task.task_id)
            .filter(|entry| entry.token == token)
        else {
            return;
        };
        let task = snapshot.task.clone();
        let previous_turn = entry.snapshot.active_turn_id.clone();
        entry.snapshot = snapshot;
        let retry_results = event.as_ref().is_some_and(|event| {
            matches!(
                event.kind,
                RuntimeEventKind::SessionReady { .. }
                    | RuntimeEventKind::TurnStarted { .. }
                    | RuntimeEventKind::InputJoined { .. }
                    | RuntimeEventKind::RequestFailed { .. }
            ) || (task.harness == "grok"
                && matches!(
                    event.kind,
                    RuntimeEventKind::TurnFinished {
                        outcome: TurnOutcome::Completed,
                        ..
                    }
                ))
        });
        if let Some(event) = &event {
            match &event.kind {
                RuntimeEventKind::LocalToolRequested { request } => {
                    entry.pending_tool_calls.insert(request.call_id.clone());
                }
                RuntimeEventKind::LocalToolCancelled { call_id, turn_id } => {
                    if entry.snapshot.active_turn_id.as_ref() == Some(turn_id) {
                        entry.pending_tool_calls.remove(call_id);
                    }
                }
                RuntimeEventKind::TurnStarted { turn_id }
                    if previous_turn.as_ref() != Some(turn_id) =>
                {
                    entry.pending_tool_calls.clear()
                }
                RuntimeEventKind::TurnFinished { turn_id, .. }
                    if previous_turn.as_ref() == Some(turn_id) =>
                {
                    entry.pending_tool_calls.clear()
                }
                RuntimeEventKind::Disconnected { .. } => entry.pending_tool_calls.clear(),
                _ => {}
            }
        }
        if let Some(event) = event {
            ctx.emit(LocalCLITaskCoordinatorEvent::Runtime { task, event });
        }
        if retry_results {
            let task_id = entry.snapshot.task.task_id.clone();
            self.retry_pending_results(&task_id, ctx);
        }
        ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
        ctx.notify();
    }
}

fn snapshot_from_host(
    task: LocalCliTask,
    host: super::runtime_host::RuntimeHostSnapshot,
) -> ManagedTaskSnapshot {
    let error = host.last_error.map(|message| {
        if message == super::runtime_host::COMMAND_NOT_DELIVERED_CODE {
            crate::t!("cli-agent-message-delivery-failed")
        } else {
            message
        }
    });
    ManagedTaskSnapshot {
        task,
        ready: host.ready,
        connected: host.connected,
        active_turn_id: host.active_turn_id,
        approvals: host
            .approvals
            .into_iter()
            .map(|(approval_id, approval)| ManagedApproval {
                approval_id,
                turn_id: approval.turn_id,
                method: approval.method,
                details: approval.details,
            })
            .collect(),
        output: host.output,
        error,
    }
}

fn protocol_state_from_host(
    host: &super::runtime_host::RuntimeHostSnapshot,
) -> RecoveryProtocolState {
    RecoveryProtocolState {
        accepted_turns: host.accepted_turn_ids.iter().cloned().collect(),
        finished_turns: host.finished_turn_ids.iter().cloned().collect(),
    }
}

fn refresh_verified_profiles(
    options: &mut SessionOptions,
    task: &LocalCliTask,
) -> Result<(), String> {
    let config: serde_json::Value = serde_json::from_str(&task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    options.claude_profile =
        serde_json::from_value(config.get("claude_profile").cloned().unwrap_or_default())
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    options.grok_profile =
        serde_json::from_value(config.get("grok_profile").cloned().unwrap_or_default())
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    Ok(())
}

fn native_session_mismatches(
    task: &LocalCliTask,
    host: &super::runtime_host::RuntimeHostSnapshot,
) -> bool {
    task.native_session_id
        .as_ref()
        .zip(host.native_session_id.as_ref())
        .is_some_and(|(saved, hosted)| saved != hosted)
}

fn advance_recovery_protocol_state(state: &mut RecoveryProtocolState, event: &RuntimeEvent) {
    match &event.kind {
        RuntimeEventKind::MessageAccepted {
            turn_id: Some(turn_id),
            ..
        } => {
            state.accepted_turns.insert(turn_id.clone());
        }
        RuntimeEventKind::TurnStarted { turn_id } => {
            state.accepted_turns.remove(turn_id);
        }
        RuntimeEventKind::InputJoined { message_id, .. } => {
            state.accepted_turns.remove(&message_id.to_string());
        }
        RuntimeEventKind::TurnFinished { turn_id, .. } => {
            state.accepted_turns.remove(turn_id);
            state.finished_turns.insert(turn_id.clone());
        }
        RuntimeEventKind::RequestFailed { message_id, .. } => {
            state.accepted_turns.remove(&message_id.to_string());
        }
        RuntimeEventKind::SessionReady { .. }
        | RuntimeEventKind::MessageAccepted { turn_id: None, .. }
        | RuntimeEventKind::CommandDispatched { .. }
        | RuntimeEventKind::TextDelta { .. }
        | RuntimeEventKind::Progress { .. }
        | RuntimeEventKind::LocalToolRequested { .. }
        | RuntimeEventKind::LocalToolCancelled { .. }
        | RuntimeEventKind::ApprovalRequested { .. }
        | RuntimeEventKind::ApprovalResolved { .. }
        | RuntimeEventKind::ApprovalCancelled { .. }
        | RuntimeEventKind::Disconnected { .. } => {}
    }
}

fn recovery_checkpoint(
    task: &LocalCliTask,
) -> Result<Option<RuntimeHostRecoveryCheckpoint>, String> {
    let config: serde_json::Value = serde_json::from_str(&task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    config
        .get("runtime_host_recovery")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))
}

async fn persist_recovery_checkpoint(
    sender: &SyncSender<ModelEvent>,
    task: &mut LocalCliTask,
    identity: &RuntimeHostRecoveryIdentity,
    recovered_through: u64,
) -> Result<(), String> {
    let previous = task.clone();
    let mut config: serde_json::Value = serde_json::from_str(&task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    config["runtime_host_recovery"] = serde_json::to_value(RuntimeHostRecoveryCheckpoint {
        runtime_generation: identity.runtime_generation,
        host_instance_id: identity.host_instance_id,
        manifest_sha256: identity.manifest_sha256.clone(),
        recovered_through,
    })
    .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    task.config_json = config.to_string();
    commit_transition(sender, task, &previous).await
}

async fn transition_recovery_state(
    sender: &SyncSender<ModelEvent>,
    task: &mut LocalCliTask,
    state: LocalCliTaskState,
    start_state: &str,
) -> Result<(), String> {
    let previous = task.clone();
    let mut config: serde_json::Value = serde_json::from_str(&task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    config["runtime_host_start_state"] = json!(start_state);
    task.config_json = config.to_string();
    task.state = state;
    commit_transition(sender, task, &previous).await
}

async fn recover_runtime_hosts(
    sender: &SyncSender<ModelEvent>,
    records: Vec<LocalCliTask>,
    owned: &HashMap<String, Uuid>,
) -> Result<RecoveryBatch, String> {
    let state_dir = super::current_state_dir();
    recover_runtime_hosts_in_state_dir(sender, records, owned, &state_dir).await
}

async fn recover_runtime_hosts_in_state_dir(
    sender: &SyncSender<ModelEvent>,
    mut records: Vec<LocalCliTask>,
    owned: &HashMap<String, Uuid>,
    state_dir: &Path,
) -> Result<RecoveryBatch, String> {
    let mut hosts = Vec::new();
    let mut results: Vec<(LocalCliTask, LocalCliMessage)> = Vec::new();
    for task in &mut records {
        // 任务终态可能已提交，而同一宿主事件尚未来得及生成父任务结果。
        // 结果 ID 由任务及代数确定；这里只补齐仍为 Queued 的记录，Sent 仍表示
        // 交付不确定，不能因应用重启而再次执行父任务输入。
        if let Some(message) = recover_terminal_task_result(sender, task).await?
            && !results
                .iter()
                .any(|(_, existing)| existing.message_id == message.message_id)
        {
            results.push((task.clone(), message));
        }
        if !task.state.is_active() {
            continue;
        }
        let config: serde_json::Value = match serde_json::from_str(&task.config_json) {
            Ok(config) => config,
            Err(_) => continue,
        };
        let Some(generation) = config
            .get("runtime_generation")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
        else {
            continue;
        };
        // 当前协调器已拥有的连接绝不能因界面刷新再次 claim；否则会 fence 正在工作的 epoch。
        if runtime_host_is_owned(owned, &task.task_id) {
            continue;
        }
        match super::runtime_host::startup_evidence_matches_task(
            state_dir,
            generation,
            &task.task_id,
            task.generation,
        )
        .map_err(|_| crate::t!("cli-agent-status-disconnected"))?
        {
            Some(false) => {
                transition_recovery_state(
                    sender,
                    task,
                    LocalCliTaskState::Unconfirmed,
                    "startup_evidence_identity_mismatch",
                )
                .await?;
                continue;
            }
            Some(true) | None => {}
        }
        let classification = super::runtime_host::classify_startup(state_dir, generation)
            .await
            .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
        let Some(record) = super::runtime_host::load_record(state_dir, generation)
            .map_err(|_| crate::t!("cli-agent-status-disconnected"))?
        else {
            let start_state = match &classification {
                super::runtime_host::RuntimeHostStartupState::Unconfirmed(
                    super::runtime_host::RuntimeHostUnconfirmed::AdapterNotStarted,
                ) => "not_started_launch_intent_missing",
                super::runtime_host::RuntimeHostStartupState::Unconfirmed(
                    super::runtime_host::RuntimeHostUnconfirmed::MissingManifest,
                ) => "missing_manifest",
                super::runtime_host::RuntimeHostStartupState::Unconfirmed(
                    super::runtime_host::RuntimeHostUnconfirmed::HostUnavailable,
                )
                | super::runtime_host::RuntimeHostStartupState::Unconfirmed(
                    super::runtime_host::RuntimeHostUnconfirmed::NativeExitUnconfirmed,
                )
                | super::runtime_host::RuntimeHostStartupState::LiveReattachable(_)
                | super::runtime_host::RuntimeHostStartupState::ExitedResumable { .. } => {
                    "startup_evidence_missing_manifest"
                }
            };
            transition_recovery_state(sender, task, LocalCliTaskState::Unconfirmed, start_state)
                .await?;
            continue;
        };
        if record.task_id() != task.task_id || record.task_generation() != task.generation {
            transition_recovery_state(
                sender,
                task,
                LocalCliTaskState::Unconfirmed,
                "identity_mismatch",
            )
            .await?;
            continue;
        }
        match classification {
            super::runtime_host::RuntimeHostStartupState::LiveReattachable(client) => {
                let mut options = client.options().clone();
                let preclaim_snapshot = match client
                    .inspect()
                    .await
                    .map_err(|_| crate::t!("cli-agent-status-disconnected"))?
                {
                    super::runtime_host::RuntimeHostResponse::Inspection { snapshot, .. } => {
                        snapshot
                    }
                    super::runtime_host::RuntimeHostResponse::OwnerClaimed { .. }
                    | super::runtime_host::RuntimeHostResponse::CommandStatus(_)
                    | super::runtime_host::RuntimeHostResponse::Events(_)
                    | super::runtime_host::RuntimeHostResponse::EventsAcknowledged { .. }
                    | super::runtime_host::RuntimeHostResponse::CommandStatuses(_)
                    | super::runtime_host::RuntimeHostResponse::Rejected { .. } => {
                        return Err(crate::t!("cli-agent-status-disconnected"));
                    }
                };
                if native_session_mismatches(task, &preclaim_snapshot) {
                    transition_recovery_state(
                        sender,
                        task,
                        LocalCliTaskState::Unconfirmed,
                        "native_session_mismatch",
                    )
                    .await?;
                    continue;
                }
                let mut claim = super::runtime_host::reattach_connection(client)
                    .await
                    .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
                let mut host_snapshot = claim.snapshot.clone();
                // Inspect 与 claim 之间仍可能完成首个 SessionReady；claim 快照先校验已 ACK 状态。
                if native_session_mismatches(task, &host_snapshot) {
                    transition_recovery_state(
                        sender,
                        task,
                        LocalCliTaskState::Unconfirmed,
                        "native_session_mismatch",
                    )
                    .await?;
                    continue;
                }
                let mut protocol_state = protocol_state_from_host(&host_snapshot);
                let mut snapshot = snapshot_from_host(task.clone(), host_snapshot.clone());
                // owner claim 已确认宿主连接存活；固定 tail 内若有 Disconnected 会再将其收敛为 false。
                snapshot.connected = true;
                let mut recovered_through = claim.acknowledged_sequence;
                let mut caught_up_session_ready = false;
                while recovered_through < claim.catch_up_through {
                    let hosted = claim
                        .pull_events(recovered_through)
                        .await
                        .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
                    let mut progressed = false;
                    for hosted in hosted {
                        let Some(expected_sequence) = recovered_through.checked_add(1) else {
                            return Err(crate::t!("cli-agent-task-invalid-launch"));
                        };
                        if hosted.sequence != expected_sequence {
                            return Err(crate::t!("cli-agent-task-invalid-launch"));
                        }
                        if hosted.sequence > claim.catch_up_through {
                            break;
                        }
                        verify_runtime_event_generation(&hosted.event, generation)?;
                        let committed = commit_catch_up_runtime_event(
                            sender,
                            &mut snapshot,
                            &mut host_snapshot,
                            &hosted.event,
                            &mut protocol_state,
                        )
                        .await?;
                        caught_up_session_ready |=
                            matches!(&hosted.event.kind, RuntimeEventKind::SessionReady { .. });
                        claim
                            .acknowledge_events(hosted.sequence)
                            .await
                            .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
                        recovered_through = hosted.sequence;
                        progressed = true;
                        if let Some(message) = committed.result {
                            results.push((snapshot.task.clone(), message));
                        }
                    }
                    if !progressed {
                        return Err(crate::t!("cli-agent-status-disconnected"));
                    }
                }
                // claim 固定的 tail 已全部提交并 ACK；此后才允许恢复工具和发布可用连接。
                if native_session_mismatches(&snapshot.task, &host_snapshot) {
                    transition_recovery_state(
                        sender,
                        &mut snapshot.task,
                        LocalCliTaskState::Unconfirmed,
                        "native_session_mismatch",
                    )
                    .await?;
                    *task = snapshot.task;
                    continue;
                }
                if caught_up_session_ready {
                    refresh_verified_profiles(&mut options, &snapshot.task)?;
                }
                let pending_local_tools = host_snapshot.local_tools.values().cloned().collect();
                let connection = claim
                    .into_connection()
                    .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
                *task = snapshot.task.clone();
                hosts.push(RecoveredHost {
                    snapshot,
                    options,
                    connection,
                    initial_input: None,
                    pending_local_tools,
                    protocol_state,
                    start_mode: ManagedStartMode::Reattached,
                });
            }
            super::runtime_host::RuntimeHostStartupState::ExitedResumable {
                receipt,
                pending_events,
                snapshot: mut host_snapshot,
            } => {
                let recovery_identity = RuntimeHostRecoveryIdentity::from(&record);
                let recovered_through = match recovery_checkpoint(task)? {
                    Some(checkpoint)
                        if checkpoint.runtime_generation == record.runtime_generation()
                            && checkpoint.host_instance_id == record.host_instance_id()
                            && checkpoint.manifest_sha256 == record.manifest_sha256()
                            && checkpoint.recovered_through >= receipt.acknowledged_sequence
                            && checkpoint.recovered_through <= receipt.last_event_sequence =>
                    {
                        checkpoint.recovered_through
                    }
                    Some(checkpoint)
                        if checkpoint.runtime_generation == record.runtime_generation() =>
                    {
                        transition_recovery_state(
                            sender,
                            task,
                            LocalCliTaskState::Unconfirmed,
                            "recovery_checkpoint_mismatch",
                        )
                        .await?;
                        continue;
                    }
                    Some(_) | None => receipt.acknowledged_sequence,
                };
                // SQLite 水位之后的快照由宿主账本确定；已收敛事件也必须重建进程内协议状态。
                for hosted in pending_events
                    .iter()
                    .take_while(|hosted| hosted.sequence <= recovered_through)
                {
                    host_snapshot.apply_recovered_event(&hosted.event);
                }
                let mut protocol_state = protocol_state_from_host(&host_snapshot);
                let mut snapshot = snapshot_from_host(task.clone(), host_snapshot);
                snapshot.connected = true;
                for hosted in pending_events
                    .into_iter()
                    .filter(|hosted| hosted.sequence > recovered_through)
                {
                    if hosted.event.generation != generation {
                        return Err(crate::t!("cli-agent-task-invalid-launch"));
                    }
                    let committed = commit_runtime_event(
                        sender,
                        &mut snapshot,
                        &hosted.event,
                        &mut protocol_state.accepted_turns,
                        &mut protocol_state.finished_turns,
                    )
                    .await?;
                    advance_recovery_protocol_state(&mut protocol_state, &hosted.event);
                    if committed
                        && matches!(hosted.event.kind, RuntimeEventKind::TurnFinished { .. })
                        && snapshot.task.state.is_terminal()
                        && let Some(message) = enqueue_task_result(
                            sender,
                            snapshot.task.task_id.clone(),
                            snapshot.task.generation,
                        )?
                        .await
                        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
                    {
                        results.push((snapshot.task.clone(), message));
                    }
                    // 此检查点只在事件相关任务/消息及结果均已提交后推进；崩溃前未推进会安全重放。
                    persist_recovery_checkpoint(
                        sender,
                        &mut snapshot.task,
                        &recovery_identity,
                        hosted.sequence,
                    )
                    .await?;
                }
                if receipt.last_event_sequence == recovered_through
                    && recovery_checkpoint(&snapshot.task)?.is_none()
                {
                    persist_recovery_checkpoint(
                        sender,
                        &mut snapshot.task,
                        &recovery_identity,
                        recovered_through,
                    )
                    .await?;
                }
                if snapshot.task.state.is_terminal()
                    && let Some(message) = enqueue_task_result(
                        sender,
                        snapshot.task.task_id.clone(),
                        snapshot.task.generation,
                    )?
                    .await
                    .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
                    && !results
                        .iter()
                        .any(|(_, existing)| existing.message_id == message.message_id)
                {
                    results.push((snapshot.task.clone(), message));
                }
                if snapshot.task.state.is_active() {
                    transition_recovery_state(
                        sender,
                        &mut snapshot.task,
                        LocalCliTaskState::Disconnected,
                        "exited_recovered",
                    )
                    .await?;
                }
                *task = snapshot.task;
            }
            super::runtime_host::RuntimeHostStartupState::Unconfirmed(
                super::runtime_host::RuntimeHostUnconfirmed::AdapterNotStarted,
            ) => {
                let messages = load_messages(sender, task.task_id.clone(), task.generation)?
                    .await
                    .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
                let queued: Vec<_> = messages
                    .into_iter()
                    .filter(|message| {
                        message.subject == "user_input"
                            && message.state == LocalCliMessageState::Queued
                    })
                    .collect();
                if queued.len() != 1 {
                    transition_recovery_state(
                        sender,
                        task,
                        LocalCliTaskState::Unconfirmed,
                        "not_started_input_uncertain",
                    )
                    .await?;
                    continue;
                }
                let message = &queued[0];
                let action: RuntimeAction = serde_json::from_str(&message.body)
                    .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
                let RuntimeAction::Submit { input } = action else {
                    transition_recovery_state(
                        sender,
                        task,
                        LocalCliTaskState::Unconfirmed,
                        "not_started_input_invalid",
                    )
                    .await?;
                    continue;
                };
                let mut options = record.options().clone();
                options.generation = Uuid::new_v4();
                let connection = super::runtime_host::spawn_connection(
                    task.task_id.clone(),
                    task.generation,
                    record.harness(),
                    options.clone(),
                );
                hosts.push(RecoveredHost {
                    snapshot: ManagedTaskSnapshot {
                        task: task.clone(),
                        ready: false,
                        connected: true,
                        active_turn_id: None,
                        approvals: Vec::new(),
                        output: String::new(),
                        error: None,
                    },
                    options,
                    connection,
                    initial_input: Some((
                        Uuid::parse_str(&message.message_id)
                            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?,
                        input,
                    )),
                    pending_local_tools: Vec::new(),
                    protocol_state: RecoveryProtocolState::default(),
                    start_mode: ManagedStartMode::RetryNotStarted,
                });
            }
            super::runtime_host::RuntimeHostStartupState::Unconfirmed(reason) => {
                let start_state = match reason {
                    super::runtime_host::RuntimeHostUnconfirmed::MissingManifest => {
                        "missing_manifest"
                    }
                    super::runtime_host::RuntimeHostUnconfirmed::HostUnavailable => {
                        "host_unavailable"
                    }
                    super::runtime_host::RuntimeHostUnconfirmed::AdapterNotStarted => {
                        unreachable!("未启动分支已单独处理")
                    }
                    super::runtime_host::RuntimeHostUnconfirmed::NativeExitUnconfirmed => {
                        "native_exit_unconfirmed"
                    }
                };
                transition_recovery_state(
                    sender,
                    task,
                    LocalCliTaskState::Unconfirmed,
                    start_state,
                )
                .await?;
            }
        }
    }
    Ok(RecoveryBatch {
        records,
        hosts,
        results,
    })
}

async fn recover_terminal_task_result(
    sender: &SyncSender<ModelEvent>,
    task: &LocalCliTask,
) -> Result<Option<LocalCliMessage>, String> {
    if !task.state.is_terminal() || task.parent_task_id.is_none() {
        return Ok(None);
    }
    if task.version != 1 || task.parent_generation.is_none() {
        return Err(crate::t!("cli-agent-task-invalid-launch"));
    }
    let Some(message) = enqueue_task_result(sender, task.task_id.clone(), task.generation)?
        .await
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
    else {
        return Err(crate::t!("cli-agent-task-invalid-launch"));
    };
    match (message.state, message.receipt_kind) {
        (LocalCliMessageState::Queued, None) => Ok(Some(message)),
        // Sent 只证明曾开始派发；恢复不能据此重投。终态回执也只保留事实。
        (LocalCliMessageState::Sent, None)
        | (
            LocalCliMessageState::Acknowledged,
            Some(LocalCliReceiptKind::NativeProtocol | LocalCliReceiptKind::ApplicationHistory),
        )
        | (LocalCliMessageState::Failed | LocalCliMessageState::Cancelled, None) => Ok(None),
        (LocalCliMessageState::Unknown, _)
        | (LocalCliMessageState::Queued | LocalCliMessageState::Sent, Some(_))
        | (LocalCliMessageState::Acknowledged, None)
        | (LocalCliMessageState::Acknowledged, Some(LocalCliReceiptKind::Unknown))
        | (LocalCliMessageState::Failed | LocalCliMessageState::Cancelled, Some(_)) => {
            Err(crate::t!("cli-agent-task-invalid-launch"))
        }
    }
}

fn runtime_host_is_owned(owned: &HashMap<String, Uuid>, task_id: &str) -> bool {
    owned.contains_key(task_id)
}

async fn publish_message_change(
    spawner: &ModelSpawner<LocalCLITaskCoordinator>,
    message: &LocalCliMessage,
) {
    let sender_task_id = message.sender_task_id.clone();
    let recipient_task_id = message.recipient_task_id.clone();
    // 已提交的数据不依赖界面仍存活；应用关闭时无需把刷新失败改写成任务失败。
    let _ = spawner
        .spawn(move |_, ctx| {
            ctx.emit(LocalCLITaskCoordinatorEvent::MessagesChanged {
                sender_task_id,
                recipient_task_id,
            });
        })
        .await;
}

async fn classify_runtime_after_disconnect(
    options: &SessionOptions,
) -> Option<super::runtime_host::RuntimeHostStartupState> {
    for attempt in 0..40 {
        let classification =
            super::runtime_host::classify_startup(&options.state_dir, options.generation)
                .await
                .ok()?;
        if !matches!(
            classification,
            super::runtime_host::RuntimeHostStartupState::Unconfirmed(
                super::runtime_host::RuntimeHostUnconfirmed::HostUnavailable
            )
        ) || attempt == 39
        {
            return Some(classification);
        }
        Timer::after(Duration::from_millis(25)).await;
    }
    None
}

const LOCAL_TOOL_LEASE_SUBJECT: &str = "native_tool_lease";

fn local_tool_lease_id(
    task: &LocalCliTask,
    runtime_generation: Uuid,
    request: &NativeLocalToolRequest,
) -> Uuid {
    let mut digest = Sha256::new();
    for value in [
        "infinishell-local-tool-lease-v1",
        task.task_id.as_str(),
        &task.generation.to_string(),
        &runtime_generation.to_string(),
        request.turn_id.as_str(),
        request.call_id.as_str(),
    ] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    let digest = digest.finalize();
    Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 至少包含 16 字节"))
}

fn local_tool_lease_message(
    task: &LocalCliTask,
    runtime_generation: Uuid,
    request: &NativeLocalToolRequest,
) -> Result<LocalCliMessage, String> {
    let request_bytes = serde_json::to_vec(request)
        .map_err(|_| "runtime_host.local_tool_lease_encoding_failed".to_owned())?;
    let body = json!({
        "runtime_generation": runtime_generation,
        "turn_id": &request.turn_id,
        "call_id": &request.call_id,
        "request_sha256": format!("{:x}", Sha256::digest(request_bytes)),
    })
    .to_string();
    Ok(LocalCliMessage {
        version: 1,
        message_id: local_tool_lease_id(task, runtime_generation, request).to_string(),
        sender_task_id: task.task_id.clone(),
        recipient_task_id: task.task_id.clone(),
        sender_generation: task.generation,
        recipient_generation: task.generation,
        subject: LOCAL_TOOL_LEASE_SUBJECT.into(),
        body,
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    })
}

async fn ensure_local_tool_lease(
    sender: &SyncSender<ModelEvent>,
    task: &LocalCliTask,
    runtime_generation: Uuid,
    request: &NativeLocalToolRequest,
) -> Result<(), String> {
    let lease = local_tool_lease_message(task, runtime_generation, request)?;
    enqueue_message(sender, lease.clone())?
        .await
        .map_err(|_| "runtime_host.local_tool_lease_commit_closed".to_owned())??;
    let stored = load_messages(sender, task.task_id.clone(), task.generation)?
        .await
        .map_err(|_| "runtime_host.local_tool_lease_read_closed".to_owned())??;
    if !stored.iter().any(|message| message == &lease) {
        return Err("runtime_host.local_tool_lease_mismatch".into());
    }
    Ok(())
}

fn local_tool_error_receipt_id(task: &LocalCliTask, request: &NativeLocalToolRequest) -> Uuid {
    let identity = serde_json::to_string(&(
        &task.task_id,
        task.generation,
        &request.turn_id,
        &request.call_id,
    ))
    .expect("本地工具错误回执身份必须可序列化");
    let mut digest = Sha256::new();
    digest.update(Uuid::nil().as_bytes());
    digest.update(identity.as_bytes());
    let digest = digest.finalize();
    Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 至少包含 16 字节"))
}

fn local_tool_result_id(call_id: Uuid) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(call_id.as_bytes());
    digest.update(b"result");
    let digest = digest.finalize();
    Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 至少包含 16 字节"))
}

async fn recovered_local_tool_completion(
    sender: &SyncSender<ModelEvent>,
    task: &LocalCliTask,
    runtime_generation: Uuid,
    request: &NativeLocalToolRequest,
) -> Result<Option<tools::ToolCompletion>, String> {
    let lease = local_tool_lease_message(task, runtime_generation, request)?;
    let stored = load_messages(sender, task.task_id.clone(), task.generation)?
        .await
        .map_err(|_| "runtime_host.local_tool_recovery_read_closed".to_owned())??;
    if !stored.iter().any(|message| message == &lease) {
        return Err("runtime_host.local_tool_lease_missing".into());
    }
    let error_receipt_id = local_tool_error_receipt_id(task, request);
    if let Some(receipt) = stored
        .iter()
        .find(|message| message.message_id == error_receipt_id.to_string())
    {
        if receipt.subject != "native_tool_result"
            || receipt.sender_task_id != task.task_id
            || receipt.recipient_task_id != task.task_id
        {
            return Err("runtime_host.local_tool_error_receipt_identity_mismatch".into());
        }
        let result = serde_json::from_str::<Result<serde_json::Value, String>>(&receipt.body)
            .map_err(|_| "runtime_host.local_tool_error_receipt_invalid".to_owned())?;
        return Ok(Some(tools::ToolCompletion {
            generation: task.generation,
            turn_id: request.turn_id.clone(),
            call_id: request.call_id.clone(),
            receipt_id: None,
            result,
        }));
    }
    let request_body = serde_json::to_string(request)
        .map_err(|_| "runtime_host.local_tool_call_encoding_failed".to_owned())?;
    let mut calls = stored.iter().filter(|message| {
        message.subject == "native_tool_call"
            && message.sender_task_id == task.task_id
            && message.recipient_task_id == task.task_id
            && message.sender_generation == task.generation
            && message.recipient_generation == task.generation
            && message.body == request_body
    });
    let Some(call) = calls.next() else {
        return Ok(None);
    };
    if calls.next().is_some() {
        return Err("runtime_host.local_tool_duplicate_execution_record".into());
    }
    let call_id = Uuid::parse_str(&call.message_id)
        .map_err(|_| "runtime_host.local_tool_execution_record_invalid".to_owned())?;
    let result_id = local_tool_result_id(call_id);
    let Some(receipt) = stored
        .iter()
        .find(|message| message.message_id == result_id.to_string())
    else {
        return Ok(Some(tools::ToolCompletion {
            generation: task.generation,
            turn_id: request.turn_id.clone(),
            call_id: request.call_id.clone(),
            receipt_id: None,
            result: Err("runtime_host.local_tool_execution_unconfirmed".into()),
        }));
    };
    if receipt.subject != "native_tool_result"
        || receipt.sender_task_id != task.task_id
        || receipt.recipient_task_id != task.task_id
    {
        return Err("runtime_host.local_tool_result_receipt_identity_mismatch".into());
    }
    let result = serde_json::from_str::<Result<serde_json::Value, String>>(&receipt.body)
        .map_err(|_| "runtime_host.local_tool_result_receipt_invalid".to_owned())?;
    Ok(Some(tools::ToolCompletion {
        generation: task.generation,
        turn_id: request.turn_id.clone(),
        call_id: request.call_id.clone(),
        receipt_id: Some(result_id),
        result,
    }))
}

fn recover_local_tool(
    request: NativeLocalToolRequest,
    snapshot: ManagedTaskSnapshot,
    options: SessionOptions,
    sender: SyncSender<ModelEvent>,
    spawner: ModelSpawner<LocalCLITaskCoordinator>,
) -> futures::future::BoxFuture<'static, tools::ToolCompletion> {
    async move {
        let generation = snapshot.task.generation;
        let turn_id = request.turn_id.clone();
        let call_id = request.call_id.clone();
        let recovered =
            recovered_local_tool_completion(&sender, &snapshot.task, options.generation, &request)
                .await;
        match recovered {
            Ok(Some(completion)) => completion,
            Ok(None) => tools::execute(request, snapshot, options, sender, spawner).await,
            Err(error) => tools::ToolCompletion {
                generation,
                turn_id,
                call_id,
                receipt_id: None,
                result: Err(error),
            },
        }
    }
    .boxed()
}

async fn run_managed_task(
    mut snapshot: ManagedTaskSnapshot,
    expected_generation: Option<i64>,
    prepared: bool,
    initial_input: Option<(Uuid, Vec<InputContent>)>,
    token: Uuid,
    sender: SyncSender<ModelEvent>,
    mut options: SessionOptions,
    connection: RuntimeConnection,
    mut commands: mpsc::Receiver<ManagedRequest>,
    spawner: ModelSpawner<LocalCLITaskCoordinator>,
    recovered_local_tools: Vec<NativeLocalToolRequest>,
    mut protocol_state: RecoveryProtocolState,
    start_mode: ManagedStartMode,
) -> Result<(), String> {
    let mut initial_request = None;
    if start_mode == ManagedStartMode::Fresh
        && matches!(options.target, SessionTarget::Resume { .. })
    {
        verify_previous_process_exit(&sender, &snapshot.task, expected_generation, &options)
            .await?;
    }
    if start_mode == ManagedStartMode::Reattached {
        if expected_generation.is_some() || initial_input.is_some() {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
    } else if prepared {
        let records = load_tasks(&sender, false)?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
        if !records.iter().any(|record| record == &snapshot.task) {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
    } else {
        checkpoint_task(&sender, snapshot.task.clone(), expected_generation)?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
    }
    if start_mode != ManagedStartMode::Reattached {
        // 即使多个应用实例读到了同一 Queued 快照，也只有一个能提交启动占用。
        let unclaimed = snapshot.task.clone();
        let mut config: serde_json::Value = serde_json::from_str(&snapshot.task.config_json)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        config["selected_skills"] = serde_json::to_value(&options.selected_skills)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        config["local_tools"] = serde_json::to_value(options.local_tools)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        config["permission_policy"] = serde_json::to_value(options.permission_policy)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        config["permission_ceiling"] = serde_json::to_value(&options.permission_ceiling)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        config["claude_profile"] = serde_json::to_value(&options.claude_profile)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        config["grok_profile"] = serde_json::to_value(&options.grok_profile)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        config["runtime_generation"] = json!(options.generation);
        if let Some(config) = config.as_object_mut() {
            config.remove("runtime_host_recovery");
            config.remove("runtime_host_start_state");
        }
        // 旧进程已确认退出后才能走到这里；旧输入保留在消息表，恢复不会自动重投。
        if snapshot.task.harness == "claude" {
            let object = config
                .as_object_mut()
                .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
            object.remove("claude_pending_input");
            object.remove("claude_current_input");
        } else if snapshot.task.harness == "grok" {
            let object = config
                .as_object_mut()
                .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
            // 原进程的等待输入只留在历史记录；新进程继续历史不自动投递它们。
            object.remove("grok_pending_inputs");
            object.remove("grok_current_input");
        }
        snapshot.task.config_json = config.to_string();
        commit_transition(&sender, &mut snapshot.task, &unclaimed).await?;
        if let Err(error) = verify_saved_parent_ceiling(&sender, &snapshot.task, &options).await {
            if let Some(evidence) = error.permission_ceiling_evidence() {
                persist_permission_failure(
                    &sender,
                    &mut snapshot.task,
                    error.to_string(),
                    evidence.clone(),
                )
                .await?;
            }
            let message = error.to_string();
            snapshot.connected = false;
            snapshot.ready = false;
            snapshot.error = Some(message.clone());
            spawner
                .spawn(move |model, ctx| model.publish(token, snapshot, None, ctx))
                .await
                .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
            return Err(message);
        }
        initial_request = initial_input.map(|(id, input)| (id, RuntimeAction::Submit { input }));
        if let Some((message_id, action)) = &initial_request {
            if start_mode == ManagedStartMode::Fresh {
                let message = input_message(&snapshot.task, *message_id, action)?;
                let outcome = enqueue_message(&sender, message)?
                    .await
                    .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
                if !matches!(outcome, LocalCliEnqueueOutcome::Created) {
                    return Err(crate::t!("cli-agent-task-invalid-launch"));
                }
            }
        }
    }
    let RuntimeConnection {
        controller,
        mut events,
        task,
    } = connection;
    let accepted_turns = &mut protocol_state.accepted_turns;
    let finished_turns = &mut protocol_state.finished_turns;
    let mut tool_calls = FuturesUnordered::new();
    let mut tool_abort_handles = HashMap::new();
    for request in recovered_local_tools {
        let key = (request.turn_id.clone(), request.call_id.clone());
        let (handle, registration) = AbortHandle::new_pair();
        tool_abort_handles.insert(key, handle);
        let call = if tool_calls.len() >= 16 {
            let completion = tools::ToolCompletion {
                generation: snapshot.task.generation,
                turn_id: request.turn_id,
                call_id: request.call_id,
                receipt_id: None,
                result: Err(crate::t!("cli-agent-local-tool-concurrency-limit")),
            };
            async move { completion }.boxed()
        } else {
            recover_local_tool(
                request,
                snapshot.clone(),
                options.clone(),
                sender.clone(),
                spawner.clone(),
            )
        };
        tool_calls.push(Abortable::new(call, registration));
    }
    let mut cancelled_tools = HashSet::new();
    let mut queue_start_wait: Option<Instant> = None;
    let worker = async {
        loop {
            enum Incoming {
                Event(Option<RuntimeEvent>),
                Request(Option<ManagedRequest>),
                Tool(Option<Result<tools::ToolCompletion, Aborted>>),
                QueueStartTimedOut,
            }
            let incoming = {
                let event = events.recv().fuse();
                let request = commands.recv().fuse();
                let tool = async {
                    if tool_calls.is_empty() {
                        futures::future::pending().await
                    } else {
                        tool_calls.next().await
                    }
                }
                .fuse();
                let queue_timeout = async {
                    if let Some(started) = queue_start_wait {
                        Timer::after(Duration::from_secs(30).saturating_sub(started.elapsed()))
                            .await;
                    } else {
                        futures::future::pending().await
                    }
                }
                .fuse();
                pin_mut!(event, request, tool, queue_timeout);
                select! {
                    event = event => Incoming::Event(event),
                    request = request => Incoming::Request(request),
                    tool = tool => Incoming::Tool(tool),
                    () = queue_timeout => Incoming::QueueStartTimedOut,
                }
            };
            match incoming {
                Incoming::Event(Some(event)) => {
                    verify_runtime_event_generation(&event, token)?;
                    match &event.kind {
                        RuntimeEventKind::LocalToolCancelled { turn_id, call_id } => {
                            let key = (turn_id.clone(), call_id.clone());
                            cancelled_tools.insert(key.clone());
                            if let Some(handle) = tool_abort_handles.remove(&key) {
                                // 包括等待 SQLite 的阶段；取消后不能恢复轮询并派发新副作用。
                                AbortHandle::abort(&handle);
                            }
                        }
                        RuntimeEventKind::TurnFinished { turn_id, .. }
                            if snapshot.active_turn_id.as_ref() == Some(turn_id) =>
                        {
                            tool_abort_handles.clear();
                            tool_calls.clear();
                        }
                        RuntimeEventKind::Disconnected { .. } => {
                            tool_abort_handles.clear();
                            tool_calls.clear();
                        }
                        _ => {}
                    }
                    let committed = commit_and_ack_runtime_event(
                        &sender,
                        &mut snapshot,
                        &controller,
                        &event,
                        accepted_turns,
                        finished_turns,
                    )
                    .await?;
                    let tool_request = match &event.kind {
                        RuntimeEventKind::LocalToolRequested { request } => Some(request.clone()),
                        _ => None,
                    };
                    if let Some(message) = committed.result {
                        let task = snapshot.task.clone();
                        spawner
                            .spawn(move |model, ctx| {
                                if model
                                    .entries
                                    .get(&task.task_id)
                                    .is_some_and(|entry| entry.token == token)
                                {
                                    model.deliver_result(message.clone(), ctx);
                                    ctx.emit(LocalCLITaskCoordinatorEvent::ResultReady {
                                        task,
                                        message,
                                    });
                                }
                            })
                            .await
                            .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
                    }
                    if !committed.committed && tool_request.is_none() {
                        continue;
                    }
                    if matches!(event.kind, RuntimeEventKind::SessionReady { .. }) {
                        // 固定策略已随就绪记录提交，后续子任务只能继承这份已验证策略。
                        refresh_verified_profiles(&mut options, &snapshot.task)?;
                        if let Some((message_id, action)) = initial_request.take() {
                            send_user_request(
                                &sender,
                                &mut snapshot,
                                &controller,
                                token,
                                message_id,
                                action,
                                true,
                            )
                            .await?;
                        }
                    }
                    let published = snapshot.clone();
                    spawner
                        .spawn(move |model, ctx| model.publish(token, published, Some(event), ctx))
                        .await
                        .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
                    if let Some(request) = tool_request {
                        if tool_calls.len() >= 16 {
                            let completion = tools::ToolCompletion {
                                generation: snapshot.task.generation,
                                turn_id: request.turn_id,
                                call_id: request.call_id,
                                receipt_id: None,
                                result: Err(crate::t!("cli-agent-local-tool-concurrency-limit")),
                            };
                            let message_id =
                                tools::prepare_tool_reply(&sender, &snapshot.task, &completion)
                                    .await?;
                            controller
                                .send(RuntimeCommand {
                                    generation: token,
                                    message_id,
                                    action: RuntimeAction::RespondLocalTool {
                                        call_id: completion.call_id,
                                        turn_id: completion.turn_id,
                                        result: completion.result,
                                    },
                                })
                                .await
                                .map_err(runtime_error_for_user)?;
                        } else {
                            let key = (request.turn_id.clone(), request.call_id.clone());
                            let (handle, registration) = AbortHandle::new_pair();
                            tool_abort_handles.insert(key, handle);
                            let call = if committed.committed {
                                tools::execute(
                                    request,
                                    snapshot.clone(),
                                    options.clone(),
                                    sender.clone(),
                                    spawner.clone(),
                                )
                            } else {
                                recover_local_tool(
                                    request,
                                    snapshot.clone(),
                                    options.clone(),
                                    sender.clone(),
                                    spawner.clone(),
                                )
                            };
                            tool_calls.push(Abortable::new(call, registration));
                        }
                    }
                }
                Incoming::Tool(Some(Ok(completion))) => {
                    tool_abort_handles
                        .remove(&(completion.turn_id.clone(), completion.call_id.clone()));
                    if completion.generation == snapshot.task.generation
                        && !cancelled_tools
                            .contains(&(completion.turn_id.clone(), completion.call_id.clone()))
                        && snapshot.active_turn_id.as_ref() == Some(&completion.turn_id)
                    {
                        let message_id =
                            tools::prepare_tool_reply(&sender, &snapshot.task, &completion).await?;
                        controller
                            .send(RuntimeCommand {
                                generation: token,
                                message_id,
                                action: RuntimeAction::RespondLocalTool {
                                    call_id: completion.call_id,
                                    turn_id: completion.turn_id,
                                    result: completion.result,
                                },
                            })
                            .await
                            .map_err(runtime_error_for_user)?;
                    }
                }
                Incoming::Tool(Some(Err(Aborted))) | Incoming::Tool(None) => {}
                Incoming::QueueStartTimedOut => {
                    return Err(crate::t!("cli-agent-claude-queue-uncertain"));
                }
                Incoming::Event(None) | Incoming::Request(None) => break,
                Incoming::Request(Some(request)) => {
                    let result = if request.expected_generation != snapshot.task.generation {
                        Err(crate::t!("cli-agent-task-invalid-launch"))
                    } else if request.from_mailbox {
                        let completed_result = snapshot.task.harness == "grok"
                            && snapshot.task.state == LocalCliTaskState::Completed
                            && snapshot.active_turn_id.is_none()
                            && request.prepared_result.is_some();
                        if !snapshot.connected
                            || !snapshot.ready
                            || (!snapshot.task.state.is_active() && !completed_result)
                        {
                            Err(crate::t!("cli-agent-message-recipient-unavailable"))
                        } else {
                            send_mailbox_request(
                                &sender,
                                &mut snapshot,
                                &controller,
                                token,
                                request.message_id,
                                request.action,
                                request.prepared_result,
                            )
                            .await
                        }
                    } else {
                        send_user_request(
                            &sender,
                            &mut snapshot,
                            &controller,
                            token,
                            request.message_id,
                            request.action,
                            false,
                        )
                        .await
                    };
                    let _ = request.reply.send(result);
                    let published = snapshot.clone();
                    spawner
                        .spawn(move |model, ctx| model.publish(token, published, None, ctx))
                        .await
                        .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
                }
            }
            // 仅在上一轮已结束、下一轮仍未开始时计时；状态噪声不会重新延长期限。
            if snapshot.connected
                && snapshot.ready
                && snapshot.active_turn_id.is_none()
                && claude_input_pending(&snapshot.task)
            {
                queue_start_wait.get_or_insert_with(Instant::now);
            } else {
                queue_start_wait = None;
            }
        }
        Ok::<(), String>(())
    };
    // 丢弃传输会关闭独立控制管道；监督进程清理后写入回执，恢复须再次核对。
    let transport = task.map(|result| {
        result.map_err(|error| {
            let evidence = error.permission_ceiling_evidence().cloned();
            (runtime_error_for_user(error), evidence)
        })
    });
    let worker = worker.map(|result| result.map_err(|error| (error, None)));
    let result = futures::future::try_join(transport, worker).await;
    if let Err((message, Some(evidence))) = &result {
        persist_permission_failure(
            &sender,
            &mut snapshot.task,
            message.clone(),
            evidence.clone(),
        )
        .await?;
    }
    if !snapshot.task.state.is_terminal() {
        match classify_runtime_after_disconnect(&options).await {
            Some(super::runtime_host::RuntimeHostStartupState::Unconfirmed(
                super::runtime_host::RuntimeHostUnconfirmed::AdapterNotStarted,
            )) if snapshot.task.state == LocalCliTaskState::Queued => {
                transition_recovery_state(
                    &sender,
                    &mut snapshot.task,
                    LocalCliTaskState::Queued,
                    "not_started",
                )
                .await?;
            }
            Some(super::runtime_host::RuntimeHostStartupState::LiveReattachable(_)) => {
                let state = snapshot.task.state;
                transition_recovery_state(&sender, &mut snapshot.task, state, "live_detached")
                    .await?;
            }
            Some(super::runtime_host::RuntimeHostStartupState::ExitedResumable {
                pending_events,
                ..
            }) => {
                let (state, start_state) = if pending_events.is_empty() {
                    (LocalCliTaskState::Disconnected, "exited_recovered")
                } else {
                    (LocalCliTaskState::Unconfirmed, "exited_pending_recovery")
                };
                transition_recovery_state(&sender, &mut snapshot.task, state, start_state).await?;
            }
            Some(super::runtime_host::RuntimeHostStartupState::Unconfirmed(reason)) => {
                let start_state = match reason {
                    super::runtime_host::RuntimeHostUnconfirmed::MissingManifest => {
                        "missing_manifest"
                    }
                    super::runtime_host::RuntimeHostUnconfirmed::HostUnavailable => {
                        "host_unavailable"
                    }
                    super::runtime_host::RuntimeHostUnconfirmed::AdapterNotStarted => {
                        "not_started_after_dispatch"
                    }
                    super::runtime_host::RuntimeHostUnconfirmed::NativeExitUnconfirmed => {
                        "native_exit_unconfirmed"
                    }
                };
                transition_recovery_state(
                    &sender,
                    &mut snapshot.task,
                    LocalCliTaskState::Unconfirmed,
                    start_state,
                )
                .await?;
            }
            None => {
                transition_recovery_state(
                    &sender,
                    &mut snapshot.task,
                    LocalCliTaskState::Unconfirmed,
                    "classification_failed",
                )
                .await?;
            }
        }
    }
    snapshot.connected = false;
    snapshot.ready = false;
    snapshot.approvals.clear();
    snapshot.error = result.as_ref().err().map(|(message, _)| message.clone());
    spawner
        .spawn(move |model, ctx| model.publish(token, snapshot, None, ctx))
        .await
        .map_err(|_| crate::t!("cli-agent-status-disconnected"))?;
    result.map(|_| ()).map_err(|(message, _)| message)
}

fn runtime_error_for_user(error: RuntimeError) -> String {
    match error {
        RuntimeError::Io(_)
        | RuntimeError::ControllerClosed
        | RuntimeError::Protocol(_)
        | RuntimeError::RequestTimedOut
        | RuntimeError::EventBackpressure => crate::t!("cli-agent-status-disconnected"),
        RuntimeError::StaleGeneration | RuntimeError::InvalidConfiguration(_) => {
            crate::t!("cli-agent-task-invalid-launch")
        }
        RuntimeError::UnsupportedVersion(_) => {
            crate::t!("cli-agent-managed-version-unavailable")
        }
        RuntimeError::PermissionCeilingRejected { .. } => {
            crate::t!("cli-agent-task-permission-ceiling-unavailable")
        }
    }
}

async fn verify_saved_parent_ceiling(
    sender: &SyncSender<ModelEvent>,
    task: &LocalCliTask,
    options: &SessionOptions,
) -> Result<(), RuntimeError> {
    let rejected = || {
        super::permissions::rejected(
            options.permission_ceiling.as_ref(),
            &json!(null),
            "parent_record_unavailable",
            false,
        )
    };
    let Some(parent_id) = &task.parent_task_id else {
        return if options.permission_ceiling.is_none() {
            Ok(())
        } else {
            Err(rejected())
        };
    };
    let parent = load_task_generations(sender, parent_id.clone())
        .map_err(|_| rejected())?
        .await
        .map_err(|_| rejected())?
        .map_err(|_| rejected())?
        .into_iter()
        .find(|parent| Some(parent.generation) == task.parent_generation)
        .ok_or_else(rejected)?;
    match parent.harness.as_str() {
        "codex" | "claude" | "grok" => super::permissions::verify_parent_binding(
            options.permission_ceiling.as_ref(),
            &parent,
            task,
        ),
        // Oz 父任务由应用权限控制；不把其记录伪装成原生 CLI 权限证据。
        "oz" if options.permission_ceiling.is_none() => Ok(()),
        _ => Err(rejected()),
    }
}

async fn persist_permission_failure(
    sender: &SyncSender<ModelEvent>,
    task: &mut LocalCliTask,
    message: String,
    evidence: serde_json::Value,
) -> Result<(), String> {
    let previous = task.clone();
    task.state = LocalCliTaskState::Failed;
    task.result = Some(message);
    task.terminal_evidence = Some(evidence.to_string());
    commit_transition(sender, task, &previous).await
}

async fn verify_previous_process_exit(
    sender: &SyncSender<ModelEvent>,
    task: &LocalCliTask,
    expected_generation: Option<i64>,
    options: &SessionOptions,
) -> Result<(), String> {
    let previous = load_tasks(sender, false)?
        .await
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
        .into_iter()
        .find(|previous| {
            previous.task_id == task.task_id && Some(previous.generation) == expected_generation
        })
        .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
    let configuration: serde_json::Value = serde_json::from_str(&previous.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    let generation = configuration
        .get("runtime_generation")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| crate::t!("cli-agent-task-exit-unconfirmed"))?;
    let state_dir = options.state_dir.clone();
    let executable = options.executable.clone();
    let cwd = options.cwd.clone();
    blocking::unblock(move || {
        if super::runtime_host::load_record(&state_dir, generation)?.is_some() {
            if super::runtime_host::confirmed_exit(&state_dir, generation)?.is_none() {
                return Err(std::io::Error::other("runtime_host_exit_unconfirmed"));
            }
        } else if super::managed_process::confirmed_exit(&state_dir, generation)?.is_none() {
            // 抢先封存尚未声明的旧代，晚到的 spawn 将失败；已有启动记录不能被覆盖。
            super::managed_process::record_not_started(
                &state_dir,
                generation,
                &executable,
                &[],
                &cwd,
            )?;
        }
        Ok::<(), std::io::Error>(())
    })
    .await
    .map_err(|_| crate::t!("cli-agent-task-exit-unconfirmed"))?;
    Ok(())
}

async fn send_user_request(
    sender: &SyncSender<ModelEvent>,
    snapshot: &mut ManagedTaskSnapshot,
    controller: &RuntimeController,
    token: Uuid,
    message_id: Uuid,
    action: RuntimeAction,
    allow_prepared: bool,
) -> Result<(), String> {
    if !snapshot.connected || (!snapshot.ready && !matches!(action, RuntimeAction::Shutdown)) {
        return Err(crate::t!("cli-agent-status-disconnected"));
    }
    let is_input = matches!(
        action,
        RuntimeAction::Submit { .. } | RuntimeAction::Steer { .. }
    );
    if is_input {
        let receipt_generation = if snapshot.task.harness == "grok" {
            let links = grok_input_links(&snapshot.task)?;
            links
                .pending
                .iter()
                .chain(links.current.iter())
                .find(|input| input.message_id == message_id)
                .map_or(snapshot.task.generation, |input| {
                    input.submission_generation
                })
        } else {
            snapshot.task.generation
        };
        let messages = load_messages(sender, snapshot.task.task_id.clone(), receipt_generation)?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
        if let Some(previous) = messages
            .iter()
            .find(|message| message.message_id == message_id.to_string())
        {
            let body = serde_json::to_string(&action)
                .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
            if previous.sender_task_id != snapshot.task.task_id || previous.body != body {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            if !allow_prepared || previous.state != LocalCliMessageState::Queued {
                return Ok(());
            }
        }
    }
    let mut input_prepared = allow_prepared;
    if snapshot.task.harness == "grok" && matches!(action, RuntimeAction::Submit { .. }) {
        let previous = snapshot.task.clone();
        let mut links = grok_input_links(&previous)?;
        let limit = if snapshot.active_turn_id.is_some() {
            MAX_GROK_PENDING_INPUTS - 1
        } else {
            MAX_GROK_PENDING_INPUTS
        };
        if links.pending.len() >= limit {
            return Err(crate::t!("cli-agent-grok-queue-full"));
        }
        let mut updated = if previous.state.is_terminal() {
            // 新代会取消旧 Queued 消息，只有已派发的等待输入才可以保留。
            verify_grok_pending_inputs(sender, &previous, &links, token).await?;
            links.current = None;
            next_task_generation(&previous)?
        } else {
            previous.clone()
        };
        links.pending.push(GrokInputLink {
            message_id,
            submission_generation: updated.generation,
            runtime_generation: token,
            native_turn_id: None,
            mailbox_sha256: None,
        });
        set_grok_input_links(&mut updated, &links)?;
        if updated.generation == previous.generation {
            updated.revision = previous
                .revision
                .checked_add(1)
                .ok_or_else(|| crate::t!("cli-agent-task-save-failed"))?;
        }
        let message = input_message(&updated, message_id, &action)?;
        // 初始准备输入和队列关联在同一事务核对，不把已有消息改投到新代。
        checkpoint_task_with_message(sender, updated.clone(), Some(previous.generation), message)?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
        snapshot.task = updated;
        if snapshot.task.generation != previous.generation {
            snapshot.output.clear();
        }
        input_prepared = true;
    } else if snapshot.task.harness == "claude" && matches!(action, RuntimeAction::Submit { .. }) {
        if claude_input_pending(&snapshot.task) {
            return Err(crate::t!("cli-agent-claude-queue-full"));
        }
        let previous = snapshot.task.clone();
        let mut updated = if previous.state.is_terminal() {
            next_task_generation(&previous)?
        } else {
            previous.clone()
        };
        let pending = ClaudePendingInput {
            message_id,
            submission_generation: updated.generation,
        };
        set_claude_pending_input(&mut updated, Some(pending))?;
        if updated.generation == previous.generation {
            updated.revision = previous
                .revision
                .checked_add(1)
                .ok_or_else(|| crate::t!("cli-agent-task-save-failed"))?;
        }
        // 队列占用和原始输入一起提交，不能先派发再补记录。
        let message = input_message(&updated, message_id, &action)?;
        checkpoint_task_with_message(sender, updated.clone(), Some(previous.generation), message)?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
        snapshot.task = updated;
        if previous.state.is_terminal() {
            snapshot.output.clear();
        }
        input_prepared = true;
    } else if matches!(action, RuntimeAction::Submit { .. }) && snapshot.task.state.is_terminal() {
        let previous = snapshot.task.clone();
        let updated = next_task_generation(&previous)?;
        let message = input_message(&updated, message_id, &action)?;
        checkpoint_task_with_message(sender, updated.clone(), Some(previous.generation), message)?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
        snapshot.task = updated;
        snapshot.output.clear();
        input_prepared = true;
    }
    if is_input {
        let message = input_message(&snapshot.task, message_id, &action)?;
        match enqueue_message(sender, message)?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
        {
            LocalCliEnqueueOutcome::Existing(LocalCliMessageState::Queued) if input_prepared => {}
            LocalCliEnqueueOutcome::Existing(_) => return Ok(()),
            LocalCliEnqueueOutcome::Created => {}
        }
        acknowledge_message(
            sender,
            message_id.to_string(),
            snapshot.task.task_id.clone(),
            snapshot.task.generation,
            LocalCliMessageState::Sent,
        )?
        .await
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
    }
    if let Err(error) = controller
        .send(RuntimeCommand {
            generation: token,
            message_id,
            action,
        })
        .await
    {
        if is_input {
            acknowledge_message(
                sender,
                message_id.to_string(),
                snapshot.task.task_id.clone(),
                snapshot.task.generation,
                LocalCliMessageState::Failed,
            )?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
            if snapshot.task.harness == "grok" {
                let previous = snapshot.task.clone();
                let mut links = grok_input_links(&previous)?;
                links.pending.retain(|input| input.message_id != message_id);
                set_grok_input_links(&mut snapshot.task, &links)?;
                commit_transition(sender, &mut snapshot.task, &previous).await?;
            }
        }
        return Err(runtime_error_for_user(error));
    }
    Ok(())
}

fn grok_result_slot_available(snapshot: &ManagedTaskSnapshot) -> Result<bool, String> {
    Ok(snapshot.task.harness == "grok"
        && snapshot.task.state == LocalCliTaskState::Completed
        && snapshot.connected
        && snapshot.ready
        && snapshot.active_turn_id.is_none()
        && grok_input_links(&snapshot.task)?.pending.is_empty())
}

async fn send_mailbox_request(
    sender: &SyncSender<ModelEvent>,
    snapshot: &mut ManagedTaskSnapshot,
    controller: &RuntimeController,
    token: Uuid,
    message_id: Uuid,
    action: RuntimeAction,
    prepared_result: Option<LocalCliMessage>,
) -> Result<(), String> {
    if let Some(message) = &prepared_result {
        if message.subject != "local_task_result"
            || message.state != LocalCliMessageState::Queued
            || message.recipient_task_id != snapshot.task.task_id
            || (message.recipient_generation != snapshot.task.generation
                && snapshot.task.harness != "grok")
            || message.recipient_generation > snapshot.task.generation
            || message.message_id != message_id.to_string()
        {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        let mut stored = load_messages(
            sender,
            snapshot.task.task_id.clone(),
            message.recipient_generation,
        )?
        .await
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
        .into_iter()
        .find(|stored| stored.message_id == message.message_id)
        .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
        let state = stored.state;
        stored.state = LocalCliMessageState::Queued;
        stored.receipt_kind = None;
        if stored != *message {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        // 同一结果的重复派发不占新槽，也不能把真实待启动输入误报成失败。
        if state != LocalCliMessageState::Queued {
            return Ok(());
        }
    }
    if snapshot.task.harness == "grok" {
        if prepared_result.is_some() {
            // 自动结果不能预先进入原生队列，否则前一回合失败后仍可能被原生自动执行。
            if !grok_result_slot_available(snapshot)? {
                return Ok(());
            }
        } else if !snapshot.connected
            || !snapshot.ready
            || !matches!(
                snapshot.task.state,
                LocalCliTaskState::Queued
                    | LocalCliTaskState::Running
                    | LocalCliTaskState::WaitingForUser
            )
        {
            return Err(crate::t!("cli-agent-message-recipient-unavailable"));
        }
    }
    // 队列占用与结果领取在同一 worker 中处理；明确未写入的结果仍保留 Queued。
    if snapshot.task.harness == "claude"
        && matches!(action, RuntimeAction::Submit { .. })
        && claude_input_pending(&snapshot.task)
    {
        if prepared_result.is_some() {
            // 自动结果等待原生 started/joined 清槽后重试；延期不代表接收或失败。
            return Ok(());
        }
        return Err(crate::t!("cli-agent-claude-queue-full"));
    }
    if snapshot.task.harness == "grok" && matches!(action, RuntimeAction::Submit { .. }) {
        let links = grok_input_links(&snapshot.task)?;
        let limit = MAX_GROK_PENDING_INPUTS - usize::from(links.current.is_some());
        if links.pending.len() >= limit {
            if prepared_result.is_some() {
                // 未领取的自动结果保留 Queued；空槽出现前不能宣称已派发。
                return Ok(());
            }
            return Err(crate::t!("cli-agent-grok-queue-full"));
        }
    }
    if let Some(message) = prepared_result {
        return dispatch_prepared_result_if_current(
            sender,
            message,
            (snapshot.task.harness == "grok").then(|| snapshot.task.clone()),
            || send_mailbox_action(sender, snapshot, controller, token, message_id, action),
        )
        .await;
    }
    send_mailbox_action(sender, snapshot, controller, token, message_id, action).await
}

async fn send_mailbox_action(
    sender: &SyncSender<ModelEvent>,
    snapshot: &mut ManagedTaskSnapshot,
    controller: &RuntimeController,
    token: Uuid,
    message_id: Uuid,
    action: RuntimeAction,
) -> Result<(), String> {
    let grok_input =
        snapshot.task.harness == "grok" && matches!(action, RuntimeAction::Submit { .. });
    if grok_input {
        let previous = snapshot.task.clone();
        let mut links = grok_input_links(&previous)?;
        let config: serde_json::Value = serde_json::from_str(&previous.config_json)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        if !snapshot.connected
            || !snapshot.ready
            || config["runtime_generation"]
                .as_str()
                .and_then(|value| Uuid::parse_str(value).ok())
                != Some(token)
        {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        let message = load_task_messages(sender, previous.task_id.clone())?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
            .into_iter()
            .find(|message| {
                message.message_id == message_id.to_string()
                    && message.recipient_task_id == previous.task_id
            })
            .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
        if message.subject == "local_task_result" {
            if !grok_result_slot_available(snapshot)? {
                return Err(crate::t!("cli-agent-message-recipient-unavailable"));
            }
        } else if message.recipient_generation != previous.generation {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        if !matches!(
            previous.state,
            LocalCliTaskState::Queued
                | LocalCliTaskState::Running
                | LocalCliTaskState::WaitingForUser
        ) && !(previous.state == LocalCliTaskState::Completed
            && snapshot.active_turn_id.is_none()
            && message.subject == "local_task_result")
        {
            return Err(crate::t!("cli-agent-message-recipient-unavailable"));
        }
        if action
            != (RuntimeAction::Submit {
                input: vec![InputContent::Text(format!(
                    "Subject: {}\n\n{}",
                    message.subject, message.body
                ))],
            })
        {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        let digest = grok_mailbox_digest(&message)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
        if let Some(input) = links
            .pending
            .iter()
            .chain(links.current.iter())
            .find(|input| input.message_id == message_id)
        {
            if input.mailbox_sha256.as_deref() != Some(digest.as_str()) {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            verify_grok_input_message(sender, &previous, input, false).await?;
            return Ok(());
        }
        if message.state != LocalCliMessageState::Sent || message.receipt_kind.is_some() {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        if links.pending.len() >= MAX_GROK_PENDING_INPUTS - usize::from(links.current.is_some()) {
            return Err(crate::t!("cli-agent-grok-queue-full"));
        }
        let source = load_task_generations(sender, message.sender_task_id.clone())?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
            .into_iter()
            .find(|source| source.generation == message.sender_generation)
            .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
        let generations = load_task_generations(sender, previous.task_id.clone())?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
        let original = generations
            .iter()
            .find(|task| task.generation == message.recipient_generation)
            .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
        if !grok_mailbox_origin_matches(&message, &source, original, &digest)
            || (message.subject == "local_task_result"
                && !grok_result_history_matches(&message, &generations, &previous))
        {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        links.pending.push(GrokInputLink {
            message_id,
            submission_generation: message.recipient_generation,
            runtime_generation: token,
            native_turn_id: None,
            mailbox_sha256: Some(digest),
        });
        let mut updated = previous.clone();
        set_grok_input_links(&mut updated, &links)?;
        // 邮箱记录已提交 Sent；队列关联另行提交成功后才允许写入原生协议。
        commit_transition(sender, &mut updated, &previous).await?;
        snapshot.task = updated;
    }
    if snapshot.task.harness == "claude" && matches!(action, RuntimeAction::Submit { .. }) {
        let previous = snapshot.task.clone();
        let pending = ClaudePendingInput {
            message_id,
            submission_generation: previous.generation,
        };
        let mut updated = previous.clone();
        set_claude_pending_input(&mut updated, Some(pending))?;
        commit_transition(sender, &mut updated, &previous).await?;
        snapshot.task = updated;
    }
    let result = controller
        .send(RuntimeCommand {
            generation: token,
            message_id,
            action,
        })
        .await
        .map_err(runtime_error_for_user);
    if result.is_err() && grok_input {
        let submission_generation = grok_input_links(&snapshot.task)?
            .pending
            .iter()
            .find(|input| input.message_id == message_id)
            .map(|input| input.submission_generation)
            .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
        acknowledge_message(
            sender,
            message_id.to_string(),
            snapshot.task.task_id.clone(),
            submission_generation,
            LocalCliMessageState::Failed,
        )?
        .await
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
        let previous = snapshot.task.clone();
        let mut links = grok_input_links(&previous)?;
        links.pending.retain(|input| input.message_id != message_id);
        set_grok_input_links(&mut snapshot.task, &links)?;
        commit_transition(sender, &mut snapshot.task, &previous).await?;
    }
    result
}

fn input_message(
    task: &LocalCliTask,
    message_id: Uuid,
    action: &RuntimeAction,
) -> Result<LocalCliMessage, String> {
    Ok(LocalCliMessage {
        version: 1,
        message_id: message_id.to_string(),
        sender_task_id: task.task_id.clone(),
        recipient_task_id: task.task_id.clone(),
        sender_generation: task.generation,
        recipient_generation: task.generation,
        subject: "user_input".into(),
        body: serde_json::to_string(action)
            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?,
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    })
}

fn next_task_generation(previous: &LocalCliTask) -> Result<LocalCliTask, String> {
    let mut task = previous.clone();
    task.generation = task
        .generation
        .checked_add(1)
        .ok_or_else(|| crate::t!("cli-agent-task-save-failed"))?;
    task.revision = 0;
    task.state = LocalCliTaskState::Queued;
    task.result = None;
    task.terminal_evidence = None;
    Ok(task)
}

fn runtime_event_is_stale(
    snapshot: &ManagedTaskSnapshot,
    event: &RuntimeEvent,
    finished_turns: &HashSet<String>,
) -> Result<bool, String> {
    let pending = claude_pending_input(&snapshot.task)?;
    let joined_result = match &event.kind {
        RuntimeEventKind::TurnFinished { turn_id, .. } => {
            claude_input_is_joined(snapshot, turn_id)?
        }
        _ => false,
    };
    Ok(
        matches!(&event.kind, RuntimeEventKind::TurnStarted { turn_id }
        if snapshot.active_turn_id.as_ref() == Some(turn_id) || finished_turns.contains(turn_id))
            || matches!(&event.kind, RuntimeEventKind::TurnFinished { turn_id, .. }
            if snapshot.active_turn_id.as_ref() != Some(turn_id)
                && !pending.as_ref().is_some_and(|input| input.message_id.to_string() == *turn_id)
                && !joined_result),
    )
}

fn claude_input_is_joined(snapshot: &ManagedTaskSnapshot, input_id: &str) -> Result<bool, String> {
    if snapshot.task.harness != "claude" {
        return Ok(false);
    }
    let config: serde_json::Value = serde_json::from_str(&snapshot.task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    Ok(config
        .get("claude_joined_inputs")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|inputs| {
            inputs.iter().any(|input| {
                input["message_id"].as_str() == Some(input_id)
                    && input["turn_id"].as_str() == snapshot.active_turn_id.as_deref()
                    && input.get("outcome").is_none()
            })
        }))
}

fn claude_joined_event_is_persisted(
    snapshot: &ManagedTaskSnapshot,
    message_id: Uuid,
    turn_id: &str,
) -> Result<bool, String> {
    if snapshot.task.harness != "claude" || snapshot.active_turn_id.as_deref() != Some(turn_id) {
        return Ok(false);
    }
    let config: serde_json::Value = serde_json::from_str(&snapshot.task.config_json)
        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
    let message_id = message_id.to_string();
    Ok(config
        .get("claude_joined_inputs")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|inputs| {
            inputs.iter().any(|input| {
                input["message_id"].as_str() == Some(message_id.as_str())
                    && input["turn_id"].as_str() == Some(turn_id)
                    && input["submission_generation"].as_i64() == Some(snapshot.task.generation)
                    && input.get("outcome").is_none()
            })
        }))
}

async fn commit_runtime_event(
    sender: &SyncSender<ModelEvent>,
    snapshot: &mut ManagedTaskSnapshot,
    event: &RuntimeEvent,
    accepted_turns: &mut HashSet<String>,
    finished_turns: &mut HashSet<String>,
) -> Result<bool, String> {
    let mut grok_links = grok_input_links(&snapshot.task)?;
    let mut grok_failed_before_start = false;
    if snapshot.task.harness == "grok"
        && snapshot.connected
        && snapshot.ready
        && snapshot.active_turn_id.is_none()
        && let RuntimeEventKind::TurnFinished {
            turn_id,
            outcome: TurnOutcome::Failed { .. },
            ..
        } = &event.kind
        && let Some(input) = grok_links
            .pending
            .first()
            .filter(|input| input.native_turn_id.as_ref() == Some(turn_id))
    {
        if input.runtime_generation != event.generation
            || event.native_session_id.is_none()
            || event.native_session_id != snapshot.task.native_session_id
            || finished_turns.contains(turn_id)
            || matches!(
                snapshot.task.state,
                LocalCliTaskState::Disconnected
                    | LocalCliTaskState::Unconfirmed
                    | LocalCliTaskState::Unknown
            )
        {
            return Ok(false);
        }
        // 队首已被原生确认后也可能在 started 前失败；不能丢弃失败或虚构开始。
        verify_grok_input_message(sender, &snapshot.task, input, true).await?;
        grok_links.current = Some(grok_links.pending.remove(0));
        grok_failed_before_start = true;
    }
    if !grok_failed_before_start && runtime_event_is_stale(snapshot, event, finished_turns)? {
        return Ok(false);
    }
    let mut grok_receipt_generation = None;
    let mut grok_started = false;
    let mut grok_failed_input = None;
    if snapshot.task.harness == "grok" {
        if let RuntimeEventKind::MessageAccepted {
            message_id,
            turn_id,
        } = &event.kind
        {
            let turn_id = turn_id
                .as_ref()
                .filter(|turn| {
                    !turn.trim().is_empty()
                        && turn.len() <= 4096
                        && !turn.chars().any(char::is_control)
                })
                .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
            if finished_turns.contains(turn_id) {
                return Ok(false);
            }
            let input = if let Some(index) = grok_links
                .pending
                .iter()
                .position(|input| input.message_id == *message_id)
            {
                if index != 0
                    || grok_links
                        .current
                        .as_ref()
                        .is_some_and(|input| input.native_turn_id.as_ref() == Some(turn_id))
                    || grok_links.pending[index]
                        .native_turn_id
                        .as_ref()
                        .is_some_and(|previous| previous != turn_id)
                {
                    return Err(crate::t!("cli-agent-task-invalid-launch"));
                }
                &mut grok_links.pending[index]
            } else if let Some(input) = grok_links
                .current
                .as_mut()
                .filter(|input| input.message_id == *message_id)
            {
                if input.native_turn_id.as_ref() != Some(turn_id) {
                    return Err(crate::t!("cli-agent-task-invalid-launch"));
                }
                if snapshot.task.state.is_terminal() {
                    return Ok(false);
                }
                input
            } else {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            };
            if input.runtime_generation != event.generation {
                return Ok(false);
            }
            if event.native_session_id.is_none()
                || event.native_session_id != snapshot.task.native_session_id
            {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            let state = verify_grok_input_message(sender, &snapshot.task, input, false).await?;
            if state == LocalCliMessageState::Acknowledged {
                if input.native_turn_id.as_ref() != Some(turn_id) {
                    return Err(crate::t!("cli-agent-task-invalid-launch"));
                }
                // 原生重复确认只核对已提交的原回执，不能跨代再次变更消息状态。
                return Ok(false);
            }
            grok_receipt_generation = Some(input.submission_generation);
            input.native_turn_id = Some(turn_id.clone());
        } else if let RuntimeEventKind::TurnStarted { turn_id } = &event.kind {
            let input = grok_links
                .pending
                .first()
                .filter(|input| input.native_turn_id.as_ref() == Some(turn_id))
                .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
            if input.runtime_generation != event.generation {
                return Ok(false);
            }
            if !snapshot.connected
                || !snapshot.ready
                || snapshot.active_turn_id.is_some()
                || event.native_session_id.is_none()
                || event.native_session_id != snapshot.task.native_session_id
            {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            verify_grok_input_message(sender, &snapshot.task, input, true).await?;
            grok_links.current = Some(grok_links.pending.remove(0));
            grok_started = true;
        } else if let RuntimeEventKind::RequestFailed { message_id, .. } = &event.kind
            && let Some(index) = grok_links
                .pending
                .iter()
                .position(|input| input.message_id == *message_id)
        {
            if grok_links.pending[index].runtime_generation != event.generation {
                return Ok(false);
            }
            grok_receipt_generation = Some(grok_links.pending[index].submission_generation);
            grok_failed_input = Some(*message_id);
        }
    }
    let pending = claude_pending_input(&snapshot.task)?;
    // 已接收而尚未 started 的下一轮不能被当成当前轮的完成或取消。
    if matches!(&event.kind, RuntimeEventKind::TurnFinished { turn_id, .. }
        if snapshot.active_turn_id.as_ref() != Some(turn_id)
            && pending.as_ref().is_some_and(|input| input.message_id.to_string() == *turn_id))
    {
        return Err(crate::t!("cli-agent-claude-queue-uncertain"));
    }
    if let RuntimeEventKind::MessageAccepted {
        message_id,
        turn_id: Some(turn_id),
    } = &event.kind
        && snapshot.active_turn_id.as_ref() != Some(turn_id)
        && !finished_turns.contains(turn_id)
    {
        if snapshot.task.harness == "claude"
            && !pending.as_ref().is_some_and(|input| {
                input.message_id == *message_id
                    && message_id.to_string() == *turn_id
                    && input.submission_generation == snapshot.task.generation
            })
        {
            return Err(crate::t!("cli-agent-claude-queue-uncertain"));
        }
        accepted_turns.insert(turn_id.clone());
    }
    if let RuntimeEventKind::TurnStarted { turn_id } = &event.kind {
        if snapshot.task.harness == "claude"
            && (!snapshot.connected
                || !snapshot.ready
                || snapshot.active_turn_id.is_some()
                || !accepted_turns.contains(turn_id)
                || !pending.as_ref().is_some_and(|input| {
                    input.message_id.to_string() == *turn_id
                        && input.submission_generation == snapshot.task.generation
                }))
        {
            return Err(crate::t!("cli-agent-claude-queue-uncertain"));
        }
        if snapshot.task.state.is_terminal() {
            if !grok_started && !accepted_turns.contains(turn_id) {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            let previous = snapshot.task.clone();
            let mut next = next_task_generation(&previous)?;
            if grok_started {
                verify_grok_pending_inputs(sender, &previous, &grok_links, event.generation)
                    .await?;
                // 自动开新代与真实回合关联一起落盘，崩溃后也保留原提交身份。
                set_grok_input_links(&mut next, &grok_links)?;
            }
            commit_transition(sender, &mut next, &previous).await?;
            snapshot.task = next;
        }
        accepted_turns.remove(turn_id);
    }
    if grok_failed_before_start && snapshot.task.state.is_terminal() {
        let previous = snapshot.task.clone();
        verify_grok_pending_inputs(sender, &previous, &grok_links, event.generation).await?;
        let mut next = next_task_generation(&previous)?;
        set_grok_input_links(&mut next, &grok_links)?;
        commit_transition(sender, &mut next, &previous).await?;
        snapshot.task = next;
    }
    if let RuntimeEventKind::InputJoined {
        message_id,
        turn_id,
    } = &event.kind
    {
        if pending.is_none()
            && accepted_turns.contains(&message_id.to_string())
            && claude_joined_event_is_persisted(snapshot, *message_id, turn_id)?
        {
            let messages = load_messages(
                sender,
                snapshot.task.task_id.clone(),
                snapshot.task.generation,
            )?
            .await
            .map_err(|_| crate::t!("cli-agent-task-save-failed"))??;
            let persisted_receipt = messages.iter().any(|message| {
                message.message_id == message_id.to_string()
                    && message.subject == "user_input"
                    && message.state == LocalCliMessageState::Acknowledged
            });
            if !persisted_receipt {
                return Err(crate::t!("cli-agent-claude-queue-uncertain"));
            }
            // SQLite 已提交、恢复水位尚未推进时只重建协议状态；不得再次改写业务记录。
            accepted_turns.remove(&message_id.to_string());
            return Ok(false);
        }
        if snapshot.task.harness != "claude"
            || !snapshot.connected
            || !snapshot.ready
            || snapshot.active_turn_id.as_ref() != Some(turn_id)
            || !accepted_turns.contains(&message_id.to_string())
            || !pending.as_ref().is_some_and(|input| {
                input.message_id == *message_id
                    && input.submission_generation == snapshot.task.generation
            })
        {
            return Err(crate::t!("cli-agent-claude-queue-uncertain"));
        }
        accepted_turns.remove(&message_id.to_string());
    }
    let previous = snapshot.task.clone();
    let mut updated = snapshot.clone();
    if snapshot.task.harness == "grok" {
        set_grok_input_links(&mut updated.task, &grok_links)?;
    } else if snapshot.task.harness == "claude" {
        if let RuntimeEventKind::TurnStarted { turn_id } = &event.kind {
            set_claude_pending_input(&mut updated.task, None)?;
            let mut config: serde_json::Value = serde_json::from_str(&updated.task.config_json)
                .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
            // 保留执行回合与原始提交的对应关系；不会篡改旧代的消息回执。
            config["claude_current_input"] = json!({
                "turn_id": turn_id,
                "submission_generation": pending.as_ref().map(|input| input.submission_generation),
            });
            updated.task.config_json = config.to_string();
        } else if matches!(&event.kind, RuntimeEventKind::InputJoined { .. }) {
            set_claude_pending_input(&mut updated.task, None)?;
        } else if matches!(&event.kind, RuntimeEventKind::RequestFailed { message_id, .. }
            if pending.as_ref().is_some_and(|input| input.message_id == *message_id))
        {
            set_claude_pending_input(&mut updated.task, None)?;
        }
    }
    if grok_failed_before_start && let RuntimeEventKind::TurnFinished { output, .. } = &event.kind {
        // 此结果只保存真实失败及原提交关联，不生成 TurnStarted，也不改已确认的消息。
        updated.task.state = LocalCliTaskState::Failed;
        updated.task.result = Some(output.clone());
        updated.task.terminal_evidence = Some(
            json!({"native_session_id":event.native_session_id,"event":event.kind}).to_string(),
        );
        updated.output = output.clone();
        updated.approvals.clear();
    } else {
        apply_runtime_event(&mut updated, event)?;
    }
    if updated.task != previous {
        commit_transition(sender, &mut updated.task, &previous).await?;
    }
    *snapshot = updated;
    let receipt = acknowledge_runtime_message(
        sender,
        &snapshot.task.task_id,
        grok_receipt_generation.unwrap_or(snapshot.task.generation),
        &event.kind,
    )
    .await?;
    if grok_receipt_generation.is_some() && receipt.is_none() {
        return Err(crate::t!("cli-agent-task-invalid-launch"));
    }
    if let Some(message_id) = grok_failed_input {
        // 先以仍然持久的等待关联核验失败回执，再清槽；跨代失败不能借无关联写回。
        let previous = snapshot.task.clone();
        grok_links
            .pending
            .retain(|input| input.message_id != message_id);
        set_grok_input_links(&mut snapshot.task, &grok_links)?;
        commit_transition(sender, &mut snapshot.task, &previous).await?;
    }
    if let RuntimeEventKind::TurnFinished { turn_id, .. } = &event.kind {
        if grok_failed_before_start {
            accepted_turns.remove(turn_id);
        }
        finished_turns.insert(turn_id.clone());
    }
    Ok(true)
}

struct CommittedRuntimeEvent {
    committed: bool,
    result: Option<LocalCliMessage>,
}

async fn commit_runtime_event_dependencies(
    sender: &SyncSender<ModelEvent>,
    snapshot: &mut ManagedTaskSnapshot,
    event: &RuntimeEvent,
    accepted_turns: &mut HashSet<String>,
    finished_turns: &mut HashSet<String>,
) -> Result<CommittedRuntimeEvent, String> {
    let committed =
        commit_runtime_event(sender, snapshot, event, accepted_turns, finished_turns).await?;
    if let RuntimeEventKind::LocalToolRequested { request } = &event.kind {
        // ACK 表示重启后已有足够信息判断“未开始、已有结果或执行不确定”。
        // 即使业务事件已提交，租约前崩溃的重放也必须幂等补齐租约。
        ensure_local_tool_lease(sender, &snapshot.task, event.generation, request).await?;
    }
    let result = if matches!(event.kind, RuntimeEventKind::TurnFinished { .. })
        && snapshot.task.state.is_terminal()
        && snapshot.task.parent_task_id.is_some()
    {
        // 事件业务状态可能已提交但宿主水位尚未推进；重复事件仍须补齐确定性结果。
        enqueue_task_result(
            sender,
            snapshot.task.task_id.clone(),
            snapshot.task.generation,
        )?
        .await
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))??
    } else {
        None
    };
    Ok(CommittedRuntimeEvent { committed, result })
}

async fn commit_catch_up_runtime_event(
    sender: &SyncSender<ModelEvent>,
    snapshot: &mut ManagedTaskSnapshot,
    host_snapshot: &mut super::runtime_host::RuntimeHostSnapshot,
    event: &RuntimeEvent,
    protocol_state: &mut RecoveryProtocolState,
) -> Result<CommittedRuntimeEvent, String> {
    let committed = commit_runtime_event_dependencies(
        sender,
        snapshot,
        event,
        &mut protocol_state.accepted_turns,
        &mut protocol_state.finished_turns,
    )
    .await?;
    advance_recovery_protocol_state(protocol_state, event);
    host_snapshot.apply_recovered_event(event);
    Ok(committed)
}

async fn commit_and_ack_runtime_event(
    sender: &SyncSender<ModelEvent>,
    snapshot: &mut ManagedTaskSnapshot,
    controller: &RuntimeController,
    event: &RuntimeEvent,
    accepted_turns: &mut HashSet<String>,
    finished_turns: &mut HashSet<String>,
) -> Result<CommittedRuntimeEvent, String> {
    let committed =
        commit_runtime_event_dependencies(sender, snapshot, event, accepted_turns, finished_turns)
            .await?;
    // 幂等重复也必须在全部派生产物持久化后推进水位，否则崩溃会永久留下半提交状态。
    controller
        .acknowledge_host_event()
        .await
        .map_err(runtime_error_for_user)?;
    Ok(committed)
}

fn verify_runtime_event_generation(event: &RuntimeEvent, token: Uuid) -> Result<(), String> {
    if event.generation == token {
        Ok(())
    } else {
        Err(crate::t!("cli-agent-task-invalid-launch"))
    }
}

fn verified_claude_profile(
    task: &LocalCliTask,
    config: &serde_json::Value,
    effective_permissions: &serde_json::Value,
) -> Result<super::permissions::ClaudeRestrictedFilesV1, String> {
    let reject = || crate::t!("cli-agent-task-invalid-launch");
    if task.harness != "claude"
        || effective_permissions.get("fixedProfileVerified") != Some(&json!(true))
        || effective_permissions.get("permissionMode")
            != Some(&json!(
                super::claude_profile::FIXED_PROTOCOL_PERMISSION_MODE
            ))
    {
        return Err(reject());
    }
    let profile: super::permissions::ClaudeRestrictedFilesV1 = serde_json::from_value(
        effective_permissions
            .get("claudeRestrictedFilesV1")
            .cloned()
            .ok_or_else(reject)?,
    )
    .map_err(|_| reject())?;
    profile
        .validate(&PathBuf::from(&task.working_directory))
        .map_err(|_| crate::t!("cli-agent-task-permission-ceiling-unavailable"))?;
    let previous: Option<super::permissions::ClaudeRestrictedFilesV1> =
        serde_json::from_value(config.get("claude_profile").cloned().unwrap_or_default())
            .map_err(|_| reject())?;
    if previous
        .as_ref()
        .is_some_and(|previous| !profile.same_scope(previous))
    {
        return Err(reject());
    }
    Ok(profile)
}

fn apply_runtime_event(
    snapshot: &mut ManagedTaskSnapshot,
    event: &RuntimeEvent,
) -> Result<(), String> {
    if let Some(native_id) = &event.native_session_id {
        if snapshot
            .task
            .native_session_id
            .as_ref()
            .is_some_and(|previous| previous != native_id)
        {
            return Err(crate::t!("cli-agent-task-invalid-launch"));
        }
        snapshot.task.native_session_id = Some(native_id.clone());
    }
    match &event.kind {
        RuntimeEventKind::SessionReady {
            verified_cli_version,
            effective_permissions,
        } => {
            if !matches!(snapshot.task.harness.as_str(), "codex" | "claude" | "grok") {
                return Err(crate::t!("cli-agent-managed-version-unavailable"));
            }
            let mut config: serde_json::Value = serde_json::from_str(&snapshot.task.config_json)
                .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
            if !config.is_object() {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            let same_runtime = config["cli_version_runtime_generation"] == json!(event.generation);
            match verified_cli_version {
                Some(version) => {
                    if version.is_empty()
                        || (same_runtime && config["cli_version"] != json!(version))
                        || event.native_session_id.is_none()
                    {
                        return Err(crate::t!("cli-agent-task-invalid-launch"));
                    }
                    config["cli_version"] = json!(version);
                    config["cli_version_runtime_generation"] = json!(event.generation);
                }
                None => {
                    // Claude 控制握手先允许首条输入，随后 system/init 才能配对版本。
                    // 此时保留历史版本，但绝不将它重新绑定到本次连接。
                    if snapshot.task.harness != "claude"
                        || event.native_session_id.is_some()
                        || same_runtime
                    {
                        return Err(crate::t!("cli-agent-task-invalid-launch"));
                    }
                }
            }
            if config.get("permission_policy")
                == Some(&json!(PermissionPolicy::ClaudeRestrictedFilesV1))
            {
                let profile =
                    verified_claude_profile(&snapshot.task, &config, effective_permissions)?;
                config["claude_profile"] = serde_json::to_value(profile)
                    .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
            }
            if matches!(
                config["permission_policy"].as_str(),
                Some("GrokRestrictedReadV1" | "GrokRestrictedFilesV1")
            ) {
                if snapshot.task.harness != "grok"
                    || effective_permissions["appCreationPolicyApplied"] != true
                    || effective_permissions["permissionEnforcementVerified"] != false
                {
                    return Err(crate::t!("cli-agent-task-invalid-launch"));
                }
                let profile: super::permissions::GrokCreationPolicyV1 =
                    serde_json::from_value(effective_permissions["grokCreationPolicyV1"].clone())
                        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
                profile
                    .validate()
                    .map_err(|_| crate::t!("cli-agent-task-permission-ceiling-unavailable"))?;
                if config["permission_policy"] != json!(profile.permission_policy())
                    || effective_permissions["requestedPolicy"]
                        != json!(profile.permission_policy())
                    || profile.working_directory()
                        != PathBuf::from(&snapshot.task.working_directory)
                            .canonicalize()
                            .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?
                    || config
                        .get("grok_profile")
                        .filter(|value| !value.is_null())
                        .is_some_and(|saved| *saved != json!(profile))
                {
                    return Err(crate::t!("cli-agent-task-invalid-launch"));
                }
                config["grok_profile"] = json!(profile);
            }
            let object = config
                .as_object_mut()
                .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
            object.insert(
                "effective_permissions".into(),
                effective_permissions.clone(),
            );
            snapshot.task.config_json = config.to_string();
            snapshot.ready = true;
        }
        RuntimeEventKind::MessageAccepted { .. } | RuntimeEventKind::CommandDispatched { .. } => {}
        RuntimeEventKind::InputJoined {
            message_id,
            turn_id,
        } => {
            let mut config: serde_json::Value = serde_json::from_str(&snapshot.task.config_json)
                .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
            let inputs = config
                .as_object_mut()
                .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?
                .entry("claude_joined_inputs")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
            if inputs.len() >= 1024
                || inputs
                    .iter()
                    .any(|input| input["message_id"] == message_id.to_string())
            {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            inputs.push(json!({"message_id":message_id,"turn_id":turn_id,
                "submission_generation":snapshot.task.generation}));
            snapshot.task.config_json = config.to_string();
        }
        RuntimeEventKind::TurnStarted { turn_id } => {
            if snapshot.active_turn_id.as_ref() == Some(turn_id) {
                return Ok(());
            }
            if snapshot.task.state.is_terminal() {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            snapshot.active_turn_id = Some(turn_id.clone());
            snapshot.task.state = LocalCliTaskState::Running;
            snapshot.output.clear();
        }
        RuntimeEventKind::TextDelta { turn_id, text, .. } => {
            if snapshot.active_turn_id.as_ref() == Some(turn_id) {
                if snapshot.output.len() + text.len() > 10 * 1024 * 1024 {
                    return Err(crate::t!("cli-agent-task-save-failed"));
                }
                snapshot.output.push_str(text);
            }
        }
        RuntimeEventKind::Progress { .. } => {}
        RuntimeEventKind::LocalToolCancelled { .. } => {}
        RuntimeEventKind::LocalToolRequested { request } => {
            if snapshot.active_turn_id.as_ref() != Some(&request.turn_id) {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
        }
        RuntimeEventKind::ApprovalRequested {
            approval_id,
            turn_id,
            method,
            details,
        } => {
            if snapshot.active_turn_id.as_ref() != Some(turn_id) {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            if !snapshot
                .approvals
                .iter()
                .any(|approval| &approval.approval_id == approval_id)
            {
                snapshot.approvals.push(ManagedApproval {
                    approval_id: approval_id.clone(),
                    turn_id: turn_id.clone(),
                    method: method.clone(),
                    details: details.clone(),
                });
            }
            snapshot.task.state = LocalCliTaskState::WaitingForUser;
        }
        RuntimeEventKind::ApprovalResolved { approval_id, .. }
        | RuntimeEventKind::ApprovalCancelled { approval_id } => {
            snapshot
                .approvals
                .retain(|approval| &approval.approval_id != approval_id);
            if snapshot.approvals.is_empty() && snapshot.active_turn_id.is_some() {
                snapshot.task.state = LocalCliTaskState::Running;
            }
        }
        RuntimeEventKind::TurnFinished {
            turn_id,
            outcome,
            output,
        } => {
            if claude_input_is_joined(snapshot, turn_id)? {
                let mut config: serde_json::Value =
                    serde_json::from_str(&snapshot.task.config_json)
                        .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
                let input = config["claude_joined_inputs"]
                    .as_array_mut()
                    .and_then(|inputs| {
                        inputs
                            .iter_mut()
                            .find(|input| input["message_id"] == *turn_id)
                    })
                    .ok_or_else(|| crate::t!("cli-agent-task-invalid-launch"))?;
                // 只记录明确关联此输入的原生结果，不将它当作另一个执行回合。
                input["outcome"] = json!(outcome);
                snapshot.task.config_json = config.to_string();
                return Ok(());
            }
            if snapshot.active_turn_id.as_ref() != Some(turn_id) {
                return Ok(());
            }
            let config: serde_json::Value = serde_json::from_str(&snapshot.task.config_json)
                .map_err(|_| crate::t!("cli-agent-task-invalid-launch"))?;
            if config
                .get("claude_joined_inputs")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|inputs| {
                    inputs.iter().any(|input| {
                        input["turn_id"].as_str() == Some(turn_id) && input.get("outcome").is_none()
                    })
                })
            {
                return Err(crate::t!("cli-agent-claude-queue-uncertain"));
            }
            snapshot.task.state = match outcome {
                TurnOutcome::Completed => LocalCliTaskState::Completed,
                TurnOutcome::Cancelled => LocalCliTaskState::Cancelled,
                TurnOutcome::Failed { .. } => LocalCliTaskState::Failed,
            };
            if snapshot.task.state == LocalCliTaskState::Completed
                && snapshot.task.native_session_id.is_none()
            {
                return Err(crate::t!("cli-agent-task-invalid-launch"));
            }
            snapshot.task.result = Some(output.clone());
            snapshot.task.terminal_evidence = Some(
                json!({"native_session_id": event.native_session_id, "event": event.kind})
                    .to_string(),
            );
            snapshot.output = output.clone();
            snapshot.active_turn_id = None;
            snapshot.approvals.clear();
        }
        RuntimeEventKind::RequestFailed { message, .. } => {
            snapshot.error = Some(
                if message == super::runtime_host::COMMAND_NOT_DELIVERED_CODE {
                    crate::t!("cli-agent-message-delivery-failed")
                } else {
                    message.clone()
                },
            );
        }
        RuntimeEventKind::Disconnected { reason } => {
            snapshot.connected = false;
            snapshot.ready = false;
            snapshot.approvals.clear();
            snapshot.error = Some(reason.clone());
            if !snapshot.task.state.is_terminal() {
                snapshot.task.state = LocalCliTaskState::Disconnected;
            }
        }
    }
    Ok(())
}

async fn commit_transition(
    sender: &SyncSender<ModelEvent>,
    task: &mut LocalCliTask,
    previous: &LocalCliTask,
) -> Result<(), String> {
    if task.generation == previous.generation {
        task.revision = previous
            .revision
            .checked_add(1)
            .ok_or_else(|| crate::t!("cli-agent-task-save-failed"))?;
    }
    checkpoint_task(sender, task.clone(), Some(previous.generation))?
        .await
        .map_err(|_| crate::t!("cli-agent-task-save-failed"))?
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;

#[cfg(all(test, not(target_family = "wasm")))]
#[path = "coordinator_cli_update_tests.rs"]
mod cli_update_launch_tests;

#[cfg(test)]
#[path = "claude_coordinator_live_tests.rs"]
mod claude_live_tests;

#[cfg(test)]
#[path = "grok_coordinator_live_tests.rs"]
mod grok_live_tests;

#[cfg(test)]
#[path = "grok_child_coordinator_live_tests.rs"]
mod grok_child_live_tests;
