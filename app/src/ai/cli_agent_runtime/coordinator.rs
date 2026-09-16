//! 托管 CLI 的本地协调器：先提交状态与输入记录，再驱动协议；重启仅加载记录。

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::SyncSender;

use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Aborted};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt, pin_mut, select};

#[path = "coordinator_tools.rs"]
mod tools;
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelContext, ModelSpawner, SingletonEntity};

use super::{
    InputContent, RuntimeAction, RuntimeCommand, RuntimeConnection, RuntimeController,
    RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome,
};
use crate::ai::local_cli_mailbox::{acknowledge_runtime_message, send_prepared_result};
use crate::persistence::ModelEvent;
use crate::persistence::local_cli_tasks::{
    LocalCliEnqueueOutcome, acknowledge_message, checkpoint_task, checkpoint_task_with_message,
    enqueue_message, enqueue_task_result, load_messages, load_task_generations, load_tasks,
};
use crate::persistence::model::{
    LocalCliMessage, LocalCliMessageState, LocalCliTask, LocalCliTaskState,
};

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
    pub(crate) async fn send(&self, command: RuntimeCommand) -> Result<(), String> {
        self.prepare_send(command)
            .await?
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
    message_id: Uuid,
    action: RuntimeAction,
    reply: oneshot::Sender<Result<(), String>>,
}

pub(crate) struct LocalCLITaskCoordinator {
    sender: Option<SyncSender<ModelEvent>>,
    entries: HashMap<String, ManagedTaskEntry>,
    restored: Vec<LocalCliTask>,
    recovery_error: Option<String>,
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

    /// 启动时只读恢复结果；中断标记由应用启动阶段统一执行，不重复变更活跃记录。
    pub(crate) fn refresh_records(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(sender) = self.sender.clone() else {
            return;
        };
        ctx.spawn(
            async move {
                load_tasks(&sender, false)?
                    .await
                    .map_err(|_| "本地任务读取确认已关闭".to_owned())?
            },
            |model, result, ctx| {
                match result {
                    Ok(records) => {
                        model.restored = records;
                        model.recovery_error = None;
                    }
                    Err(error) => model.recovery_error = Some(error),
                }
                ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
                ctx.notify();
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
        if self
            .entries
            .get(&task.task_id)
            .is_some_and(|entry| entry.snapshot.connected)
        {
            return Err(crate::t!("cli-agent-task-already-running"));
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
                let config: serde_json::Value =
                    serde_json::from_str(&task.config_json).map_err(|error| error.to_string())?;
                options.selected_skills = match config.get("selected_skills") {
                    Some(value) => {
                        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?
                    }
                    None => Vec::new(),
                };
                options.permission_ceiling = match config.get("permission_ceiling") {
                    Some(value) => {
                        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?
                    }
                    None => None,
                };
            }
        }
        let sender = self
            .sender
            .clone()
            .ok_or_else(|| crate::t!("cli-agent-task-save-failed"))?;
        let connection = match Harness::parse_orchestration_harness(&task.harness) {
            Some(Harness::Codex) => super::codex::connect(options.clone()),
            Some(Harness::Claude) => super::claude::connect(options.clone()),
            Some(
                Harness::Grok
                | Harness::Oz
                | Harness::Gemini
                | Harness::OpenCode
                | Harness::Unknown,
            )
            | None => {
                return Err(crate::t!("cli-agent-managed-version-unavailable"));
            }
        }
        .map_err(|error| error.to_string())?;
        let token = options.generation;
        let task_id = task.task_id.clone();
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
                )
                .await
            },
            move |model, result, ctx| {
                if let Some(entry) = model.entries.get_mut(&task_id)
                    && entry.token == token
                {
                    entry.snapshot.connected = false;
                    entry.snapshot.ready = false;
                    entry.snapshot.approvals.clear();
                    entry.pending_tool_calls.clear();
                    if let Err(error) = result {
                        entry.snapshot.error = Some(error);
                        if !entry.snapshot.task.state.is_terminal() {
                            entry.snapshot.task.state = LocalCliTaskState::Disconnected;
                        }
                    }
                    ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
                    ctx.notify();
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
        let Some(sender) = self.sender.clone() else {
            return;
        };
        let Some(endpoint) = self.endpoint(&message.recipient_task_id) else {
            return;
        };
        if endpoint.generation != message.recipient_generation
            || (endpoint.harness == Harness::Claude && endpoint.active_turn_id.is_some())
        {
            return;
        }
        let task_id = endpoint.task_id.clone();
        let generation = endpoint.generation;
        let token = endpoint.runtime_generation;
        let sender_task_id = message.sender_task_id.clone();
        let recipient_task_id = message.recipient_task_id.clone();
        ctx.spawn(
            async move { send_prepared_result(&sender, endpoint, message).await },
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
        ctx.emit(LocalCLITaskCoordinatorEvent::Changed);
        ctx.notify();
    }
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

async fn run_managed_task(
    mut snapshot: ManagedTaskSnapshot,
    expected_generation: Option<i64>,
    prepared: bool,
    initial_input: Option<(Uuid, Vec<InputContent>)>,
    token: Uuid,
    sender: SyncSender<ModelEvent>,
    options: SessionOptions,
    connection: RuntimeConnection,
    mut commands: mpsc::Receiver<ManagedRequest>,
    spawner: ModelSpawner<LocalCLITaskCoordinator>,
) -> Result<(), String> {
    if matches!(options.target, SessionTarget::Resume { .. }) {
        verify_previous_process_exit(&sender, &snapshot.task, expected_generation, &options)
            .await?;
    }
    if prepared {
        let records = load_tasks(&sender, false)?
            .await
            .map_err(|_| "本地任务读取确认已关闭".to_owned())??;
        if !records.iter().any(|record| record == &snapshot.task) {
            return Err("准备启动的本地任务记录已过期或不存在".to_owned());
        }
    } else {
        checkpoint_task(&sender, snapshot.task.clone(), expected_generation)?
            .await
            .map_err(|_| "本地任务启动确认已关闭".to_owned())??;
    }
    // 即使多个应用实例读到了同一 Queued 快照，也只有一个能提交启动占用。
    let unclaimed = snapshot.task.clone();
    let mut config: serde_json::Value =
        serde_json::from_str(&snapshot.task.config_json).map_err(|error| error.to_string())?;
    config["selected_skills"] =
        serde_json::to_value(&options.selected_skills).map_err(|error| error.to_string())?;
    config["local_tools"] =
        serde_json::to_value(options.local_tools).map_err(|error| error.to_string())?;
    config["permission_policy"] =
        serde_json::to_value(options.permission_policy).map_err(|error| error.to_string())?;
    config["permission_ceiling"] =
        serde_json::to_value(&options.permission_ceiling).map_err(|error| error.to_string())?;
    config["runtime_generation"] = json!(options.generation);
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
            .map_err(|_| "本地任务界面已关闭".to_owned())?;
        return Err(message);
    }
    let mut initial_request =
        initial_input.map(|(id, input)| (id, RuntimeAction::Submit { input }));
    if let Some((message_id, action)) = &initial_request {
        let message = input_message(&snapshot.task, *message_id, action)?;
        let outcome = enqueue_message(&sender, message)?
            .await
            .map_err(|_| "初始输入入队确认已关闭".to_owned())??;
        if !matches!(outcome, LocalCliEnqueueOutcome::Created) {
            return Err("初始输入已经存在，拒绝重复启动".to_owned());
        }
    }
    let RuntimeConnection {
        controller,
        mut events,
        task,
    } = connection;
    let mut accepted_turns = HashSet::new();
    let mut finished_turns = HashSet::new();
    let mut tool_calls = FuturesUnordered::new();
    let mut tool_abort_handles = HashMap::new();
    let mut cancelled_tools = HashSet::new();
    let worker = async {
        loop {
            enum Incoming {
                Event(Option<RuntimeEvent>),
                Request(Option<ManagedRequest>),
                Tool(Option<Result<tools::ToolCompletion, Aborted>>),
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
                pin_mut!(event, request, tool);
                select! {
                    event = event => Incoming::Event(event),
                    request = request => Incoming::Request(request),
                    tool = tool => Incoming::Tool(tool),
                }
            };
            match incoming {
                Incoming::Event(Some(event)) => {
                    if event.generation != token {
                        continue;
                    }
                    // 重复开始与旧轮终态不能清空当前输出、工具调用或新一轮的审批。
                    if matches!(&event.kind,RuntimeEventKind::TurnStarted {turn_id} if snapshot.active_turn_id.as_ref()==Some(turn_id))
                        || matches!(&event.kind,RuntimeEventKind::TurnFinished {turn_id, ..} if snapshot.active_turn_id.as_ref()!=Some(turn_id))
                    {
                        continue;
                    }
                    match &event.kind {
                        RuntimeEventKind::LocalToolCancelled { turn_id, call_id } => {
                            let key = (turn_id.clone(), call_id.clone());
                            cancelled_tools.insert(key.clone());
                            if let Some(handle) = tool_abort_handles.remove(&key) {
                                // 包括等待 SQLite 的阶段；取消后不能恢复轮询并派发新副作用。
                                AbortHandle::abort(&handle);
                            }
                        }
                        RuntimeEventKind::TurnFinished { .. }
                        | RuntimeEventKind::Disconnected { .. } => {
                            tool_abort_handles.clear();
                            tool_calls.clear();
                        }
                        _ => {}
                    }
                    if let RuntimeEventKind::MessageAccepted {
                        turn_id: Some(turn_id),
                        ..
                    } = &event.kind
                        && snapshot.active_turn_id.as_ref() != Some(turn_id)
                        && !finished_turns.contains(turn_id)
                    {
                        accepted_turns.insert(turn_id.clone());
                    }
                    if let RuntimeEventKind::TurnStarted { turn_id } = &event.kind {
                        if finished_turns.contains(turn_id) {
                            continue;
                        }
                        if snapshot.task.state.is_terminal() {
                            if !accepted_turns.contains(turn_id) {
                                return Err("未经确认的新回合不能重新打开任务".to_owned());
                            }
                            let previous = snapshot.task.clone();
                            let mut next = next_task_generation(&previous)?;
                            commit_transition(&sender, &mut next, &previous).await?;
                            snapshot.task = next;
                        }
                        accepted_turns.remove(turn_id);
                    }
                    let previous = snapshot.task.clone();
                    let mut updated = snapshot.clone();
                    apply_runtime_event(&mut updated, &event)?;
                    if updated.task != previous {
                        commit_transition(&sender, &mut updated.task, &previous).await?;
                    }
                    snapshot = updated;
                    acknowledge_runtime_message(
                        &sender,
                        &snapshot.task.task_id,
                        snapshot.task.generation,
                        &event.kind,
                    )
                    .await?;
                    if matches!(event.kind, RuntimeEventKind::SessionReady { .. })
                        && let Some((message_id, action)) = initial_request.take()
                    {
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
                    if matches!(event.kind, RuntimeEventKind::TurnFinished { .. })
                        && snapshot.task.state.is_terminal()
                    {
                        if let RuntimeEventKind::TurnFinished { turn_id, .. } = &event.kind {
                            finished_turns.insert(turn_id.clone());
                        }
                        let result = enqueue_task_result(
                            &sender,
                            snapshot.task.task_id.clone(),
                            snapshot.task.generation,
                        )?
                        .await
                        .map_err(|_| "本地任务结果回收确认已关闭".to_owned())??;
                        if let Some(message) = result {
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
                                .map_err(|_| "本地任务界面已关闭".to_owned())?;
                        }
                    }
                    let tool_request = match &event.kind {
                        RuntimeEventKind::LocalToolRequested { request } => Some(request.clone()),
                        _ => None,
                    };
                    let published = snapshot.clone();
                    spawner
                        .spawn(move |model, ctx| model.publish(token, published, Some(event), ctx))
                        .await
                        .map_err(|_| "本地任务界面已关闭".to_owned())?;
                    if let Some(request) = tool_request {
                        if tool_calls.len() >= 16 {
                            let completion = tools::ToolCompletion {
                                generation: snapshot.task.generation,
                                turn_id: request.turn_id,
                                call_id: request.call_id,
                                receipt_id: None,
                                result: Err("本地工具并发请求达到上限".into()),
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
                                .map_err(|error| error.to_string())?;
                        } else {
                            let key = (request.turn_id.clone(), request.call_id.clone());
                            let (handle, registration) = AbortHandle::new_pair();
                            tool_abort_handles.insert(key, handle);
                            tool_calls.push(Abortable::new(
                                tools::execute(
                                    request,
                                    snapshot.clone(),
                                    options.clone(),
                                    sender.clone(),
                                    spawner.clone(),
                                ),
                                registration,
                            ));
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
                            .map_err(|error| error.to_string())?;
                    }
                }
                Incoming::Tool(Some(Err(Aborted))) | Incoming::Tool(None) => {}
                Incoming::Event(None) | Incoming::Request(None) => break,
                Incoming::Request(Some(request)) => {
                    let result = if request.expected_generation != snapshot.task.generation {
                        Err(crate::t!("cli-agent-task-invalid-launch"))
                    } else if request.from_mailbox {
                        if !snapshot.ready || !snapshot.task.state.is_active() {
                            Err(crate::t!("cli-agent-message-recipient-unavailable"))
                        } else if unverified_overlap(&snapshot, &request.action) {
                            Err(crate::t!("cli-agent-claude-queue-unverified"))
                        } else {
                            controller
                                .send(RuntimeCommand {
                                    generation: token,
                                    message_id: request.message_id,
                                    action: request.action,
                                })
                                .await
                                .map_err(|error| error.to_string())
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
                        .map_err(|_| "本地任务界面已关闭".to_owned())?;
                }
            }
        }
        Ok::<(), String>(())
    };
    // 丢弃传输会关闭独立控制管道；监督进程清理后写入回执，恢复须再次核对。
    let transport = task.map(|result| {
        result.map_err(|error| {
            (
                error.to_string(),
                error.permission_ceiling_evidence().cloned(),
            )
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
    if !snapshot.task.state.is_terminal() && snapshot.task.state != LocalCliTaskState::Disconnected
    {
        let previous = snapshot.task.clone();
        snapshot.task.state = LocalCliTaskState::Disconnected;
        commit_transition(&sender, &mut snapshot.task, &previous).await?;
    }
    snapshot.connected = false;
    snapshot.ready = false;
    snapshot.approvals.clear();
    snapshot.error = result.as_ref().err().map(|(message, _)| message.clone());
    spawner
        .spawn(move |model, ctx| model.publish(token, snapshot, None, ctx))
        .await
        .map_err(|_| "本地任务界面已关闭".to_owned())?;
    result.map(|_| ()).map_err(|(message, _)| message)
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
        "codex" | "claude" => super::permissions::verify_parent_binding(
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
        if super::managed_process::confirmed_exit(&state_dir, generation)?.is_none() {
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
    if unverified_overlap(snapshot, &action) {
        return Err(crate::t!("cli-agent-claude-queue-unverified"));
    }
    let is_input = matches!(
        action,
        RuntimeAction::Submit { .. } | RuntimeAction::Steer { .. }
    );
    if is_input {
        let messages = load_messages(
            sender,
            snapshot.task.task_id.clone(),
            snapshot.task.generation,
        )?
        .await
        .map_err(|_| "输入记录读取确认已关闭".to_owned())??;
        if let Some(previous) = messages
            .iter()
            .find(|message| message.message_id == message_id.to_string())
        {
            let body = serde_json::to_string(&action).map_err(|error| error.to_string())?;
            if previous.sender_task_id != snapshot.task.task_id || previous.body != body {
                return Err("重复消息 ID 的输入内容不一致".to_owned());
            }
            if !allow_prepared || previous.state != LocalCliMessageState::Queued {
                return Ok(());
            }
        }
    }
    let mut input_prepared = allow_prepared;
    if matches!(action, RuntimeAction::Submit { .. }) && snapshot.task.state.is_terminal() {
        let previous = snapshot.task.clone();
        let updated = next_task_generation(&previous)?;
        let message = input_message(&updated, message_id, &action)?;
        checkpoint_task_with_message(sender, updated.clone(), Some(previous.generation), message)?
            .await
            .map_err(|_| "后续输入与任务代数提交确认已关闭".to_owned())??;
        snapshot.task = updated;
        snapshot.output.clear();
        input_prepared = true;
    }
    if is_input {
        let message = input_message(&snapshot.task, message_id, &action)?;
        match enqueue_message(sender, message)?
            .await
            .map_err(|_| "输入入队确认已关闭".to_owned())??
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
        .map_err(|_| "输入交付记录确认已关闭".to_owned())??;
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
            .map_err(|_| "输入失败记录确认已关闭".to_owned())??;
        }
        return Err(error.to_string());
    }
    Ok(())
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
        body: serde_json::to_string(action).map_err(|error| error.to_string())?,
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    })
}

fn unverified_overlap(snapshot: &ManagedTaskSnapshot, action: &RuntimeAction) -> bool {
    snapshot.task.harness == "claude"
        && snapshot.active_turn_id.is_some()
        && matches!(action, RuntimeAction::Submit { .. })
}

fn next_task_generation(previous: &LocalCliTask) -> Result<LocalCliTask, String> {
    let mut task = previous.clone();
    task.generation = task.generation.checked_add(1).ok_or("任务代数已耗尽")?;
    task.revision = 0;
    task.state = LocalCliTaskState::Queued;
    task.result = None;
    task.terminal_evidence = None;
    Ok(task)
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
            return Err("原生会话身份不匹配，拒绝更新本地任务".to_owned());
        }
        snapshot.task.native_session_id = Some(native_id.clone());
    }
    match &event.kind {
        RuntimeEventKind::SessionReady {
            effective_permissions,
        } => {
            snapshot.ready = true;
            let version = match snapshot.task.harness.as_str() {
                "codex" => "0.147.0",
                "claude" => "2.1.273",
                _ => return Err("未验证的 CLI 不得进入托管就绪状态".to_owned()),
            };
            let mut config: serde_json::Value = serde_json::from_str(&snapshot.task.config_json)
                .map_err(|error| error.to_string())?;
            let object = config.as_object_mut().ok_or("本地任务配置无效")?;
            object.insert("cli_version".into(), version.into());
            object.insert(
                "effective_permissions".into(),
                effective_permissions.clone(),
            );
            snapshot.task.config_json = config.to_string();
        }
        RuntimeEventKind::MessageAccepted { .. } | RuntimeEventKind::CommandDispatched { .. } => {}
        RuntimeEventKind::TurnStarted { turn_id } => {
            if snapshot.active_turn_id.as_ref() == Some(turn_id) {
                return Ok(());
            }
            if snapshot.task.state.is_terminal() {
                return Err("旧回合不能重新打开已完成的任务".into());
            }
            snapshot.active_turn_id = Some(turn_id.clone());
            snapshot.task.state = LocalCliTaskState::Running;
            snapshot.output.clear();
        }
        RuntimeEventKind::TextDelta { turn_id, text, .. } => {
            if snapshot.active_turn_id.as_ref() == Some(turn_id) {
                if snapshot.output.len() + text.len() > 10 * 1024 * 1024 {
                    return Err("任务输出超过本地记录限制".into());
                }
                snapshot.output.push_str(text);
            }
        }
        RuntimeEventKind::Progress { .. } => {}
        RuntimeEventKind::LocalToolCancelled { .. } => {}
        RuntimeEventKind::LocalToolRequested { request } => {
            if snapshot.active_turn_id.as_ref() != Some(&request.turn_id) {
                return Err("本地工具属于过期回合".into());
            }
        }
        RuntimeEventKind::ApprovalRequested {
            approval_id,
            turn_id,
            method,
            details,
        } => {
            if snapshot.active_turn_id.as_ref() != Some(turn_id) {
                return Err("审批属于过期回合".into());
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
            if snapshot.active_turn_id.as_ref() != Some(turn_id) {
                return Ok(());
            }
            snapshot.task.state = match outcome {
                TurnOutcome::Completed => LocalCliTaskState::Completed,
                TurnOutcome::Cancelled => LocalCliTaskState::Cancelled,
                TurnOutcome::Failed { .. } => LocalCliTaskState::Failed,
            };
            if snapshot.task.state == LocalCliTaskState::Completed
                && snapshot.task.native_session_id.is_none()
            {
                return Err("缺少原生会话 ID，无法确认任务完成".into());
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
        RuntimeEventKind::RequestFailed { message, .. } => snapshot.error = Some(message.clone()),
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
        task.revision = previous.revision.checked_add(1).ok_or("任务修订号已耗尽")?;
    }
    checkpoint_task(sender, task.clone(), Some(previous.generation))?
        .await
        .map_err(|_| "本地任务提交确认已关闭".to_owned())?
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
