pub mod event;
mod event_cursor;
pub(crate) mod grok_leader_input;
pub(crate) mod grok_owned_launch;
#[cfg(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64"))]
pub(crate) mod grok_owned_worker;
mod grok_permission_evidence;
pub use grok_permission_evidence::{GrokPermissionEvidence, GrokPermissionObservation};
pub mod listener;
#[cfg(feature = "local_fs")]
mod local_tasks;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod plugin_manager;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use event::{CLIAgentEvent, CLIAgentEventSource, CLIAgentEventType};
use uuid::Uuid;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use self::event_cursor::{EventCursor, EventDisposition};
use self::listener::CLIAgentSessionListener;
use super::CLIAgent;
use crate::ai::blocklist::InputConfig;

/// Ctrl-C 后等待可信事件的时间；超时只能将结果标记为未知。
pub const CTRL_C_CANCEL_WINDOW: Duration = Duration::from_secs(2);

/// Status of a tracked CLI agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CLIAgentSessionStatus {
    InProgress,
    /// 缺少可信终态，需要回到原生终端确认。
    Unknown,
    /// 连接已结束，但尚未收到任务终态。
    Disconnected,
    Success,
    Failed {
        error_type: Option<String>,
        message: Option<String>,
    },
    Blocked {
        message: Option<String>,
    },
    /// 原生协议已确认取消；新的 prompt_submit 可以开启下一轮。
    Cancelled,
}

impl CLIAgentSessionStatus {
    pub fn to_conversation_status(&self) -> crate::ai::agent::conversation::ConversationStatus {
        use crate::ai::agent::conversation::ConversationStatus;
        match self {
            CLIAgentSessionStatus::InProgress => ConversationStatus::InProgress,
            CLIAgentSessionStatus::Unknown => ConversationStatus::Blocked {
                blocked_action: crate::t!("cli-agent-status-unknown"),
            },
            CLIAgentSessionStatus::Disconnected => ConversationStatus::Blocked {
                blocked_action: crate::t!("cli-agent-status-disconnected"),
            },
            CLIAgentSessionStatus::Success => ConversationStatus::Success,
            CLIAgentSessionStatus::Failed { .. } => ConversationStatus::Error,
            CLIAgentSessionStatus::Blocked { message } => ConversationStatus::Blocked {
                blocked_action: message.clone().unwrap_or_default(),
            },
            CLIAgentSessionStatus::Cancelled => ConversationStatus::Cancelled,
        }
    }
}

/// Rich context accumulated from CLI agent session events.
#[derive(Debug, Clone, Default)]
pub struct CLIAgentSessionContext {
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input_preview: Option<String>,
    pub summary: Option<String>,
    pub query: Option<String>,
    pub response: Option<String>,
    /// 仅本次监听期间的原生 SessionStart 证据，不能从数据库恢复为有效证据。
    pub grok_permission_evidence: GrokPermissionEvidence,
}

/// State of the rich input editor for composing a prompt to send to a CLI agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CLIAgentInputState {
    /// The rich input editor is not open.
    Closed,
    /// The rich input editor is open.
    Open {
        /// How this session was opened (for telemetry).
        entrypoint: CLIAgentInputEntrypoint,
        /// The input config that was active before opening rich input.
        previous_input_config: InputConfig,
        /// Whether the previous lock state was established while the input buffer was empty.
        previous_was_lock_set_with_empty_buffer: bool,
    },
}

/// Why the CLI agent rich input was closed (for telemetry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CLIAgentRichInputCloseReason {
    /// User explicitly closed (Escape, Ctrl-G, footer button).
    Manual,
    /// Auto-closed due to agent status change (e.g. Blocked).
    AutoToggle,
    /// Auto-dismissed after submitting a prompt.
    Submit,
    /// Closed for another reason (chip removed, session ended, shared session sync).
    Other,
}

/// How a [`CLIAgentInputState`] was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CLIAgentInputEntrypoint {
    /// User pressed Ctrl-G while a CLI agent was active.
    CtrlG,
    /// User clicked the rich input button in the CLI agent footer.
    FooterButton,
    /// Automatically opened when the CLI agent resumed work (left a blocked state)
    /// and the auto-show setting is enabled.
    AutoShow,
    /// Rich input was opened to mirror a shared-session participant's state.
    SharedSessionSync,
}

impl CLIAgentSessionContext {
    pub(crate) fn display_title(&self) -> Option<String> {
        self.latest_user_prompt().or_else(|| self.title_like_text())
    }

    pub(crate) fn latest_user_prompt(&self) -> Option<String> {
        self.query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(str::to_owned)
    }

    /// Returns summary text suitable as a fallback title when no user prompt is available.
    pub(crate) fn title_like_text(&self) -> Option<String> {
        self.summary
            .as_deref()
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(str::to_owned)
    }
}

/// A tracked CLI agent session.
#[derive(Debug, Clone)]
pub struct CLIAgentSession {
    pub agent: CLIAgent,
    pub status: CLIAgentSessionStatus,
    pub session_context: CLIAgentSessionContext,
    /// Rich input editor state.
    pub input_state: CLIAgentInputState,
    /// Whether status-driven auto-toggle is enabled for this session.
    pub should_auto_toggle_input: bool,
    /// Event listener for plugin-backed sessions or Codex OSC9 fallback.
    /// `None` for non-Codex sessions created by command detection alone.
    /// Dropping this handle cleans up the listener's PTY event subscription.
    pub listener: Option<ModelHandle<CLIAgentSessionListener>>,
    /// The plugin version reported by structured plugin events.
    /// `None` if the plugin predates version reporting or Codex is using OSC9 fallback.
    pub plugin_version: Option<String>,
    /// `None` when the session is local.
    /// `Some("user@hostname")` when running over SSH (warpified or legacy).
    /// Used as a key for per-host plugin install failure tracking.
    pub remote_host: Option<String>,
    /// Draft text saved from the rich input composer when it was closed.
    /// Restored into the editor when the composer is reopened.
    pub draft_text: Option<String>,
    /// When the session was detected via a custom toolbar command pattern,
    /// the first word of the command (the binary/alias the user typed).
    /// Used to customize plugin instructions and force manual install mode.
    pub custom_command_prefix: Option<String>,
    /// Set once the session has received any structured OSC 777 (rich)
    /// notification. Codex's OSC 9 fallback never sets it, so this is the
    /// single source of truth for whether the session is plugin-backed.
    pub received_rich_notification: bool,
}

impl CLIAgentSession {
    pub fn is_remote(&self) -> bool {
        self.remote_host.is_some()
    }

    /// 是否已收到插件的结构化状态和上下文；这不能单独证明回合成功结束。
    /// Codex OSC 9 回退和仅注册监听器都不算收到结构化通知。
    pub fn supports_rich_status(&self) -> bool {
        self.received_rich_notification
    }

    /// Clears state populated by `PermissionRequest`. Called whenever the
    /// session leaves the permission flow (the user replied, a blocking tool
    /// completed, a new prompt is submitted, or the session ends successfully)
    /// so the permission summary doesn't leak into later UI surfaces — most
    /// visibly the tab title, which can fall back to `summary` when `query`
    /// is unset.
    fn clear_permission_scoped_state(&mut self) {
        self.session_context.summary = None;
        self.session_context.tool_name = None;
        self.session_context.tool_input_preview = None;
    }

    /// Applies an event to this session, updating context and status.
    /// Returns the new status if it changed, or `None` if the event was irrelevant.
    fn apply_event(&mut self, event: &CLIAgentEvent) -> Option<CLIAgentSessionStatus> {
        if event.source == CLIAgentEventSource::CodexOsc9Fallback {
            if self.received_rich_notification || self.status == CLIAgentSessionStatus::Unknown {
                return None;
            }
            self.status = CLIAgentSessionStatus::Unknown;
            return Some(self.status.clone());
        }
        // 同一轮的终态不可被晚到回调覆盖；只有新的输入才开启下一轮。
        if matches!(
            self.status,
            CLIAgentSessionStatus::Success
                | CLIAgentSessionStatus::Failed { .. }
                | CLIAgentSessionStatus::Cancelled
        ) && !matches!(
            event.event,
            CLIAgentEventType::PromptSubmit | CLIAgentEventType::SessionStart
        ) {
            return None;
        }
        self.session_context.cwd = event.cwd.clone().or(self.session_context.cwd.take());
        self.session_context.project = event
            .project
            .clone()
            .or(self.session_context.project.take());
        self.session_context.session_id = event
            .session_id
            .clone()
            .or(self.session_context.session_id.take());

        let new_status = match &event.event {
            CLIAgentEventType::PromptSubmit => {
                self.session_context.query = event.payload.query.clone();
                self.session_context.response = None;
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::InProgress
            }
            CLIAgentEventType::ToolComplete => {
                if !matches!(self.status, CLIAgentSessionStatus::Blocked { .. })
                    && !(self.status == CLIAgentSessionStatus::Unknown
                        && matches!(
                            self.agent,
                            CLIAgent::Codex | CLIAgent::Claude | CLIAgent::Grok
                        ))
                {
                    return None;
                }
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::InProgress
            }
            CLIAgentEventType::Stop => {
                if event.payload.query.is_some() {
                    self.session_context.query = event.payload.query.clone();
                }
                if event.payload.response.is_some() {
                    self.session_context.response = event.payload.response.clone();
                }
                self.clear_permission_scoped_state();
                // Stop 在其他 hook 决定继续之前执行；原生回合 ID 只能证明归属。
                // 保留当前响应供查看，但不能锁定成功并吞掉同回合后续审批或失败。
                if matches!(
                    self.agent,
                    CLIAgent::Codex | CLIAgent::Claude | CLIAgent::Grok
                ) {
                    CLIAgentSessionStatus::Unknown
                } else {
                    CLIAgentSessionStatus::Success
                }
            }
            CLIAgentEventType::StopFailure => {
                self.session_context.query = event.payload.query.clone();
                self.session_context.response = event.payload.response.clone();
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::Failed {
                    error_type: event.payload.error_type.clone(),
                    message: event.payload.response.clone(),
                }
            }
            CLIAgentEventType::PermissionRequest => {
                self.session_context.summary = event.payload.summary.clone();
                self.session_context.tool_name = event.payload.tool_name.clone();
                self.session_context.tool_input_preview = event.payload.tool_input_preview.clone();
                CLIAgentSessionStatus::Blocked {
                    message: event.payload.summary.clone(),
                }
            }
            CLIAgentEventType::QuestionAsked => CLIAgentSessionStatus::Blocked {
                message: event
                    .payload
                    .summary
                    .clone()
                    .or_else(|| Some(crate::t!("cli-agent-waiting-for-answer"))),
            },
            CLIAgentEventType::PermissionReplied => {
                if !matches!(self.status, CLIAgentSessionStatus::Blocked { .. }) {
                    return None;
                }
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::InProgress
            }
            // IdlePrompt means the agent is sitting at its prompt waiting for input.
            // This should not affect status — otherwise it would override Success after a Stop event.
            CLIAgentEventType::IdlePrompt => return None,
            CLIAgentEventType::Notification => return None,
            CLIAgentEventType::Cancelled => {
                self.clear_permission_scoped_state();
                CLIAgentSessionStatus::Cancelled
            }
            CLIAgentEventType::SessionStart => {
                self.plugin_version = event.payload.plugin_version.clone();
                return None;
            }
            CLIAgentEventType::Unknown(_) => return None,
        };

        if self.status == new_status {
            return None;
        }
        self.status = new_status.clone();
        Some(new_status)
    }
}

/// Events emitted by `CLIAgentSessionsModel` for subscribers (e.g., `AgentNotificationsModel`).
#[allow(dead_code)] // `agent` fields on Started/InputSessionChanged/Ended are used for logging and future subscribers.
#[derive(Debug, Clone)]
pub enum CLIAgentSessionsModelEvent {
    /// 会话级原生审批提醒；不改变回合、取消等待或持久化任务状态。
    AttentionRequested {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
    Started {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
    StatusChanged {
        terminal_view_id: EntityId,
        agent: CLIAgent,
        status: CLIAgentSessionStatus,
        session_context: Box<CLIAgentSessionContext>,
    },
    InputSessionChanged {
        terminal_view_id: EntityId,
        agent: CLIAgent,
        /// The input state BEFORE this change. When transitioning from
        /// `Open` → `Closed`, contains the saved input config to restore.
        previous_input_state: CLIAgentInputState,
        /// The input state AFTER this change.
        new_input_state: CLIAgentInputState,
    },
    Ended {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
    /// The agent session has been updated. Subscribers may use this as a trigger for best-effort
    /// saving of state derived from the agent's session.
    SessionUpdated {
        terminal_view_id: EntityId,
        agent: CLIAgent,
    },
}

impl CLIAgentSessionsModelEvent {
    pub fn terminal_view_id(&self) -> EntityId {
        match self {
            CLIAgentSessionsModelEvent::Started {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::AttentionRequested {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::InputSessionChanged {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            }
            | CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id, ..
            } => *terminal_view_id,
        }
    }
}

/// Per-session state for a Ctrl-C-initiated pending cancellation. Kept
/// separate from `CLIAgentSession` because it is synthesized entirely
/// client-side (see `observe_ctrl_c_write`) rather than reported by the
/// plugin protocol.
#[derive(Default)]
struct CtrlCCancelState {
    /// Whether a `prompt_submit` has been seen for this session. Guards
    /// against arming on the optimistic `InProgress` status set when a
    /// session is first registered, before any turn has actually started.
    has_seen_prompt_submit: bool,
    /// Abort handle for the in-flight grace-window timer, if armed.
    pending_cancel: Option<SpawnedFutureHandle>,
    /// 标识 `pending_cancel` 所属的等待窗口。`SpawnedFutureHandle::abort` 要等下次轮询
    /// 才生效，已完成计时并排队的回调仍可能执行。回调仅在令牌仍匹配时生效，
    /// 防止过时回调把较新的事件状态覆盖为 `Unknown`。
    armed_token: Option<u64>,
}

/// Singleton model that tracks pane-scoped CLI agent state and plugin-enriched session context.
pub struct CLIAgentSessionsModel {
    sessions: HashMap<EntityId, CLIAgentSession>,
    /// Tracks (agent, remote_host) pairs where an auto plugin operation (install or update) has failed.
    /// Shared across all views so failure in one tab is reflected everywhere.
    plugin_auto_failures: HashSet<(CLIAgent, Option<String>)>,
    /// Ctrl-C pending-cancel state, keyed by terminal view. See `observe_ctrl_c_write`.
    ctrl_c_cancel_state: HashMap<EntityId, CtrlCCancelState>,
    /// Source of `CtrlCCancelState::armed_token` values. Monotonically increasing;
    /// never reused, so a stale callback can never alias a newer window.
    next_ctrl_c_token: u64,
    event_cursors: HashMap<EntityId, EventCursor>,
    input_generations: HashMap<EntityId, Uuid>,
    input_submissions: HashMap<EntityId, Uuid>,
    #[cfg(feature = "local_fs")]
    local_task_bindings: HashMap<EntityId, local_tasks::TaskBinding>,
    #[cfg(feature = "local_fs")]
    restored_local_tasks: Vec<crate::persistence::model::LocalCliTask>,
    #[cfg(feature = "local_fs")]
    local_task_recovery_failed: bool,
}

impl Entity for CLIAgentSessionsModel {
    type Event = CLIAgentSessionsModelEvent;
}

impl SingletonEntity for CLIAgentSessionsModel {}

impl CLIAgentSessionsModel {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            plugin_auto_failures: HashSet::new(),
            ctrl_c_cancel_state: HashMap::new(),
            next_ctrl_c_token: 0,
            event_cursors: HashMap::new(),
            input_generations: HashMap::new(),
            input_submissions: HashMap::new(),
            #[cfg(feature = "local_fs")]
            local_task_bindings: HashMap::new(),
            #[cfg(feature = "local_fs")]
            restored_local_tasks: Vec::new(),
            #[cfg(feature = "local_fs")]
            local_task_recovery_failed: false,
        }
    }

    pub fn session(&self, terminal_view_id: EntityId) -> Option<&CLIAgentSession> {
        self.sessions.get(&terminal_view_id)
    }

    /// 空闲升级要求原生会话已退出；等待输入或已完成回合仍可能持有 CLI 进程。
    pub(crate) fn has_local_session(&self, agent: CLIAgent) -> bool {
        self.sessions
            .values()
            .any(|session| session.agent == agent && !session.is_remote())
    }

    /// 异步粘贴和延迟 Enter 必须核对代次，禁止写入已取消或替换的会话。
    pub(crate) fn input_generation(&self, terminal_view_id: EntityId) -> Option<Uuid> {
        self.input_generations.get(&terminal_view_id).copied()
    }

    /// 同一代际只允许一个文本、附件与 Enter 提交流程，拒绝时调用方保留草稿。
    pub(crate) fn begin_input_submission(
        &mut self,
        terminal_view_id: EntityId,
        generation: Uuid,
    ) -> bool {
        if self.input_generation(terminal_view_id) != Some(generation)
            || self.input_submissions.get(&terminal_view_id) == Some(&generation)
        {
            return false;
        }
        self.input_submissions.insert(terminal_view_id, generation);
        true
    }

    pub(crate) fn is_input_submission_current(
        &self,
        terminal_view_id: EntityId,
        generation: Uuid,
    ) -> bool {
        self.input_generation(terminal_view_id) == Some(generation)
            && self.input_submissions.get(&terminal_view_id) == Some(&generation)
    }

    pub(crate) fn finish_input_submission(&mut self, terminal_view_id: EntityId, generation: Uuid) {
        if self.input_submissions.get(&terminal_view_id) == Some(&generation) {
            self.input_submissions.remove(&terminal_view_id);
        }
    }

    /// Returns `true` if the rich input editor is currently open for this terminal.
    pub fn is_input_open(&self, terminal_view_id: EntityId) -> bool {
        self.sessions
            .get(&terminal_view_id)
            .is_some_and(|s| matches!(s.input_state, CLIAgentInputState::Open { .. }))
    }

    /// Registers a plugin-backed listener on the session for this terminal.
    ///
    /// If a session for the same agent already exists (e.g. created earlier by
    /// command detection), it is upgraded with the listener and plugin context.
    /// Otherwise a new session is created.
    ///
    /// The optional `cwd` / `project` / `session_id` fields supply initial
    /// context when available (e.g. from a `SessionStart` event). Passing
    /// `None` for all three is fine — happens when the plugin is installed
    /// mid-session and there is no start event to extract context from.
    #[allow(clippy::too_many_arguments)]
    pub fn register_listener(
        &mut self,
        terminal_view_id: EntityId,
        agent: CLIAgent,
        cwd: Option<String>,
        project: Option<String>,
        session_id: Option<String>,
        plugin_version: Option<String>,
        remote_host: Option<String>,
        should_auto_toggle_input: bool,
        listener: ModelHandle<CLIAgentSessionListener>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(session) = self
            .sessions
            .get_mut(&terminal_view_id)
            .filter(|s| s.agent == agent)
        {
            // 监听器被替换或会话身份变化后，旧证据不能随上下文一起继承。
            if session.agent == CLIAgent::Grok
                && (session
                    .listener
                    .as_ref()
                    .is_some_and(|previous| previous.id() != listener.id())
                    || session_id
                        .as_ref()
                        .zip(session.session_context.session_id.as_ref())
                        .is_some_and(|(incoming, active)| incoming != active))
            {
                session
                    .session_context
                    .grok_permission_evidence
                    .invalidate();
            }
            // Upgrade existing session with plugin context.
            session.listener = Some(listener);
            session.plugin_version = plugin_version;
            session.remote_host = remote_host;
            session.should_auto_toggle_input = should_auto_toggle_input;
            session.session_context.cwd = cwd.or(session.session_context.cwd.take());
            session.session_context.project = project.or(session.session_context.project.take());
            session.session_context.session_id =
                session_id.or(session.session_context.session_id.take());
            return;
        }

        self.set_session(
            terminal_view_id,
            CLIAgentSession {
                agent,
                status: CLIAgentSessionStatus::InProgress,
                session_context: CLIAgentSessionContext {
                    cwd,
                    project,
                    session_id,
                    ..Default::default()
                },
                input_state: CLIAgentInputState::Closed,
                should_auto_toggle_input,
                listener: Some(listener),
                plugin_version,
                remote_host,
                draft_text: None,
                custom_command_prefix: None,
                received_rich_notification: false,
            },
            ctx,
        );
    }

    pub fn remove_session(&mut self, terminal_view_id: EntityId, ctx: &mut ModelContext<Self>) {
        self.input_generations.remove(&terminal_view_id);
        self.input_submissions.remove(&terminal_view_id);
        self.abort_pending_cancel(terminal_view_id);
        self.ctrl_c_cancel_state.remove(&terminal_view_id);
        self.event_cursors.remove(&terminal_view_id);
        if let Some(session) = self.sessions.remove(&terminal_view_id) {
            #[cfg(feature = "local_fs")]
            {
                self.persist_local_task_status(
                    terminal_view_id,
                    CLIAgentSessionStatus::Disconnected,
                    session.session_context.clone(),
                    None,
                    ctx,
                );
                self.local_task_bindings.remove(&terminal_view_id);
            }
            if matches!(
                session.status,
                CLIAgentSessionStatus::InProgress
                    | CLIAgentSessionStatus::Blocked { .. }
                    | CLIAgentSessionStatus::Unknown
            ) {
                ctx.emit(CLIAgentSessionsModelEvent::StatusChanged {
                    terminal_view_id,
                    agent: session.agent,
                    status: CLIAgentSessionStatus::Disconnected,
                    session_context: Box::new(session.session_context),
                });
            }
            ctx.emit(CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: session.agent,
            });
        }
    }

    /// Updates the session's status and context from a parsed CLI agent event.
    /// Rich plugin events latch `received_rich_notification` so rich-status
    /// surfaces stay consistent even if the first event was not SessionStart.
    pub fn update_from_event(
        &mut self,
        terminal_view_id: EntityId,
        event: &CLIAgentEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get(&terminal_view_id) else {
            return;
        };
        // 拒绝其他 CLI、旧原生会话和重投事件，且不让它们解除正在等待的取消。
        if event.agent != session.agent
            || event
                .session_id
                .as_ref()
                .zip(session.session_context.session_id.as_ref())
                .is_some_and(|(incoming, active)| incoming != active)
        {
            return;
        }

        let disposition = self
            .event_cursors
            .entry(terminal_view_id)
            .or_default()
            .accept(event);
        // 先去重，再观察权限；无回合 ID 的会话提醒也必须撤销失效证据。
        if disposition != EventDisposition::Drop {
            let session = self
                .sessions
                .get_mut(&terminal_view_id)
                .expect("session checked above");
            let context = &mut session.session_context;
            if context.grok_permission_evidence.observe(
                event,
                context.session_id.as_deref(),
                context.cwd.as_deref(),
            ) && !(disposition == EventDisposition::Accept
                && matches!(
                    event.event,
                    CLIAgentEventType::SessionStart
                        | CLIAgentEventType::PromptSubmit
                        | CLIAgentEventType::ToolComplete
                ))
            {
                ctx.emit(CLIAgentSessionsModelEvent::SessionUpdated {
                    terminal_view_id,
                    agent: session.agent,
                });
            }
        }
        let session = self
            .sessions
            .get(&terminal_view_id)
            .expect("session checked above");
        match disposition {
            EventDisposition::Drop => return,
            EventDisposition::SessionAttention => {
                // 只展示已知原生会话的提醒，不能把无关联通知当成当前回合的审批状态。
                if event.session_id.as_deref().is_some_and(|incoming| {
                    !incoming.is_empty()
                        && Some(incoming) == session.session_context.session_id.as_deref()
                }) && matches!(
                    session.status,
                    CLIAgentSessionStatus::InProgress | CLIAgentSessionStatus::Blocked { .. }
                ) {
                    ctx.emit(CLIAgentSessionsModelEvent::AttentionRequested {
                        terminal_view_id,
                        agent: session.agent,
                    });
                }
                return;
            }
            EventDisposition::UnverifiedTerminal => {
                let session = self
                    .sessions
                    .get_mut(&terminal_view_id)
                    .expect("session checked above");
                // 无原生回合关联的旧插件通知不能更新结果，也不能覆盖已经确认的终态。
                if matches!(
                    session.status,
                    CLIAgentSessionStatus::Success
                        | CLIAgentSessionStatus::Failed { .. }
                        | CLIAgentSessionStatus::Cancelled
                        | CLIAgentSessionStatus::Unknown
                ) {
                    return;
                }
                session.status = CLIAgentSessionStatus::Unknown;
                ctx.emit(CLIAgentSessionsModelEvent::StatusChanged {
                    terminal_view_id,
                    agent: session.agent,
                    status: session.status.clone(),
                    session_context: Box::new(session.session_context.clone()),
                });
                #[cfg(feature = "local_fs")]
                {
                    let context = session.session_context.clone();
                    self.persist_local_task_status(
                        terminal_view_id,
                        CLIAgentSessionStatus::Unknown,
                        context,
                        None,
                        ctx,
                    );
                }
                return;
            }
            EventDisposition::Accept => {}
        }

        // 普通广播、未知事件与空闲通知不确认任务仍运行，不能解除取消等待。
        if !matches!(
            event.event,
            CLIAgentEventType::IdlePrompt
                | CLIAgentEventType::Notification
                | CLIAgentEventType::Unknown(_)
        ) {
            self.abort_pending_cancel(terminal_view_id);
        }
        if matches!(event.event, CLIAgentEventType::PromptSubmit) {
            self.ctrl_c_cancel_state
                .entry(terminal_view_id)
                .or_default()
                .has_seen_prompt_submit = true;
        }

        let session = self
            .sessions
            .get_mut(&terminal_view_id)
            .expect("session presence checked above");

        if event.source == CLIAgentEventSource::RichPlugin {
            session.received_rich_notification = true;
        }

        let event_type = &event.event;
        if let Some(new_status) = session.apply_event(event) {
            let agent = session.agent;
            ctx.emit(CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                agent,
                status: new_status,
                session_context: Box::new(session.session_context.clone()),
            });
        }

        if matches!(
            event_type,
            CLIAgentEventType::SessionStart
                | CLIAgentEventType::PromptSubmit
                | CLIAgentEventType::ToolComplete
        ) {
            ctx.emit(CLIAgentSessionsModelEvent::SessionUpdated {
                terminal_view_id,
                agent: session.agent,
            });
        }
        #[cfg(feature = "local_fs")]
        {
            let status = session.status.clone();
            let context = session.session_context.clone();
            let evidence = (event.source == CLIAgentEventSource::RichPlugin
                && matches!(
                    event.event,
                    CLIAgentEventType::Stop
                        | CLIAgentEventType::StopFailure
                        | CLIAgentEventType::Cancelled
                ))
            .then(|| {
                serde_json::json!({
                    "source": "osc777",
                    "event": format!("{:?}", event.event),
                    "event_id": event.payload.event_id,
                    "sequence": event.payload.sequence,
                    "turn_id": event.payload.turn_id,
                    "prompt_id": event.payload.prompt_id,
                    "session_id": event.session_id,
                })
                .to_string()
            });
            self.persist_local_task_status(terminal_view_id, status, context, evidence, ctx);
        }
    }

    /// 观察发往此会话 PTY 的 Ctrl-C 字节（`0x03`），调用方仍应立即原样转发。
    /// 仅在会话处于 `InProgress` 或 `Blocked`、支持富状态（不含 Codex OSC 9 回退），
    /// 且已收到 `prompt_submit` 时启动原生确认等待，避免误用注册时的乐观运行状态。
    /// 超时且没有可信事件时只标记 Unknown；重复 Ctrl-C 不重置等待窗口。
    pub fn observe_ctrl_c_write(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(generation) = self.input_generations.get_mut(&terminal_view_id) {
            *generation = Uuid::new_v4();
        }
        self.observe_ctrl_c_write_with_window(terminal_view_id, CTRL_C_CANCEL_WINDOW, ctx);
    }

    fn observe_ctrl_c_write_with_window(
        &mut self,
        terminal_view_id: EntityId,
        window: Duration,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get(&terminal_view_id) else {
            return;
        };
        let can_arm = matches!(
            session.status,
            CLIAgentSessionStatus::InProgress | CLIAgentSessionStatus::Blocked { .. }
        ) && session.supports_rich_status()
            && self
                .ctrl_c_cancel_state
                .get(&terminal_view_id)
                .is_some_and(|state| state.has_seen_prompt_submit);
        if !can_arm {
            return;
        }
        if self
            .ctrl_c_cancel_state
            .get(&terminal_view_id)
            .is_some_and(|state| state.pending_cancel.is_some())
        {
            return;
        }

        let token = self.next_ctrl_c_token;
        self.next_ctrl_c_token += 1;
        let state = self
            .ctrl_c_cancel_state
            .entry(terminal_view_id)
            .or_default();
        let handle = ctx.spawn_abortable(
            async move { warpui::r#async::Timer::after(window).await },
            move |model, _, ctx| model.resolve_pending_cancel(terminal_view_id, token, ctx),
            |_, _| {},
        );
        state.pending_cancel = Some(handle);
        state.armed_token = Some(token);
    }

    /// 等待窗口结束且未收到解除等待的插件事件时，将会话标为 `Unknown`。
    /// 若 `token` 不再匹配，则窗口已被解除、替换或移除；忽略尚未被 abort 阻止的排队回调。
    fn resolve_pending_cancel(
        &mut self,
        terminal_view_id: EntityId,
        token: u64,
        ctx: &mut ModelContext<Self>,
    ) {
        let owns_current_window = self
            .ctrl_c_cancel_state
            .get_mut(&terminal_view_id)
            .is_some_and(|state| {
                if state.armed_token != Some(token) {
                    return false;
                }
                state.pending_cancel = None;
                state.armed_token = None;
                true
            });
        if !owns_current_window {
            return;
        }
        self.resolve_unconfirmed_interrupt(terminal_view_id, ctx);
    }

    /// Aborts and clears any armed pending-cancel window for this terminal.
    /// Invalidates the token so a callback already queued when the abort
    /// fires too late to matter (see `resolve_pending_cancel`) is a no-op.
    fn abort_pending_cancel(&mut self, terminal_view_id: EntityId) {
        if let Some(state) = self.ctrl_c_cancel_state.get_mut(&terminal_view_id) {
            state.armed_token = None;
            if let Some(handle) = state.pending_cancel.take() {
                handle.abort();
            }
        }
    }

    /// 仅对仍存在且处于 `InProgress` 或 `Blocked` 的会话设置 `Unknown` 并发出状态事件。
    fn resolve_unconfirmed_interrupt(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };
        if !matches!(
            session.status,
            CLIAgentSessionStatus::InProgress | CLIAgentSessionStatus::Blocked { .. }
        ) {
            return;
        }

        // 沉默只代表未确认；不能把慢响应、断线或插件失效当成取消成功。
        session.status = CLIAgentSessionStatus::Unknown;
        let agent = session.agent;
        let session_context = Box::new(session.session_context.clone());
        #[cfg(feature = "local_fs")]
        self.persist_local_task_status(
            terminal_view_id,
            CLIAgentSessionStatus::Unknown,
            *session_context.clone(),
            None,
            ctx,
        );
        ctx.emit(CLIAgentSessionsModelEvent::StatusChanged {
            terminal_view_id,
            agent,
            status: CLIAgentSessionStatus::Unknown,
            session_context,
        });
    }

    /// Whether Ctrl-C cancellation has already resolved (`Cancelled`) or is
    /// still pending (the grace window is armed) for this session. Only
    /// used by tests.
    #[cfg(test)]
    pub(crate) fn has_pending_or_resolved_ctrl_c_cancel(&self, terminal_view_id: EntityId) -> bool {
        if matches!(
            self.sessions.get(&terminal_view_id).map(|s| &s.status),
            Some(CLIAgentSessionStatus::Cancelled | CLIAgentSessionStatus::Unknown)
        ) {
            return true;
        }
        self.ctrl_c_cancel_state
            .get(&terminal_view_id)
            .is_some_and(|state| state.pending_cancel.is_some())
    }

    pub fn open_input(
        &mut self,
        terminal_view_id: EntityId,
        entrypoint: CLIAgentInputEntrypoint,
        previous_input_config: InputConfig,
        previous_was_lock_set_with_empty_buffer: bool,
        should_auto_toggle_input: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };

        let previous_input_state = session.input_state;
        session.input_state = CLIAgentInputState::Open {
            entrypoint,
            previous_input_config,
            previous_was_lock_set_with_empty_buffer,
        };
        session.should_auto_toggle_input = should_auto_toggle_input;

        ctx.emit(CLIAgentSessionsModelEvent::InputSessionChanged {
            terminal_view_id,
            agent: session.agent,
            previous_input_state,
            new_input_state: session.input_state,
        });
    }

    pub fn close_input(
        &mut self,
        terminal_view_id: EntityId,
        should_auto_toggle_input: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(session) = self.sessions.get_mut(&terminal_view_id) else {
            return;
        };
        if session.input_state == CLIAgentInputState::Closed {
            return;
        }

        let previous_input_state = session.input_state;
        session.input_state = CLIAgentInputState::Closed;
        self.input_generations
            .insert(terminal_view_id, Uuid::new_v4());
        session.should_auto_toggle_input = should_auto_toggle_input;
        ctx.emit(CLIAgentSessionsModelEvent::InputSessionChanged {
            terminal_view_id,
            agent: session.agent,
            previous_input_state,
            new_input_state: CLIAgentInputState::Closed,
        });
    }

    pub fn set_session(
        &mut self,
        terminal_view_id: EntityId,
        session: CLIAgentSession,
        ctx: &mut ModelContext<Self>,
    ) {
        let agent = session.agent;
        self.input_generations
            .insert(terminal_view_id, Uuid::new_v4());
        // Close any open rich input before replacing, so subscribers can
        // restore input config before the session ends.
        self.close_input(terminal_view_id, false, ctx);
        // A fresh session must re-observe `prompt_submit` before Ctrl-C can
        // arm, and any pending window belonged to the session being replaced.
        self.abort_pending_cancel(terminal_view_id);
        self.ctrl_c_cancel_state.remove(&terminal_view_id);
        self.event_cursors.remove(&terminal_view_id);
        if let Some(old) = self.sessions.insert(terminal_view_id, session) {
            ctx.emit(CLIAgentSessionsModelEvent::Ended {
                terminal_view_id,
                agent: old.agent,
            });
        }

        ctx.emit(CLIAgentSessionsModelEvent::Started {
            terminal_view_id,
            agent,
        });
    }

    /// Records that an auto plugin operation (install or update) failed for the given agent/host.
    /// `remote_host` is `None` for local sessions, `Some("user@hostname")` for remote.
    #[cfg(not(target_family = "wasm"))]
    pub fn record_plugin_auto_failure(&mut self, agent: CLIAgent, remote_host: Option<String>) {
        self.plugin_auto_failures.insert((agent, remote_host));
    }

    /// Saves draft text from the rich input composer for the given terminal.
    /// Stores `None` for empty or whitespace-only text.
    pub fn set_draft(&mut self, terminal_view_id: EntityId, text: String) {
        if let Some(session) = self.sessions.get_mut(&terminal_view_id) {
            session.draft_text = if text.trim().is_empty() {
                None
            } else {
                Some(text)
            };
        }
    }

    /// Clears any saved draft text for the given terminal.
    pub fn clear_draft(&mut self, terminal_view_id: EntityId) {
        if let Some(session) = self.sessions.get_mut(&terminal_view_id) {
            session.draft_text = None;
        }
    }

    /// Returns and clears the draft text for the given terminal, if any.
    pub fn take_draft(&mut self, terminal_view_id: EntityId) -> Option<String> {
        self.sessions
            .get_mut(&terminal_view_id)
            .and_then(|s| s.draft_text.take())
    }

    /// Whether an auto plugin operation has previously failed for this agent on this host.
    pub fn has_plugin_auto_failed(&self, agent: CLIAgent, remote_host: &Option<String>) -> bool {
        self.plugin_auto_failures
            .contains(&(agent, remote_host.clone()))
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
