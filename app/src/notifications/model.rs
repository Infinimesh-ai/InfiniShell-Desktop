//! 通知中心数据模型(单例 Singleton)。
//!
//! 002ce467 cloud-removal 删除 `agent_management` 时把这个 model 一并清掉了,但
//! - 软件本体的 BYOP agent (Oz) 完成/出错通知
//! - 第三方 CLI agent (Claude / Codex / DeepSeek 等) 状态通知
//!
//! 仍需要走通知中心。本模块是删前 `AgentNotificationsModel` 的精简版:
//! - 去掉了 `ActiveAgentViewsModel` 订阅(该 model 是云端管理 view 的状态来源,已删)。
//!   原本用 `is_conversation_open` 判断"对话视图是否还开着",改成查
//!   `BlocklistAIHistoryModel::conversation()` 判断"对话是否还在内存里"。
//! - 去掉了 `AgentManagementEvent::ConversationNeedsAttention`(legacy toast 路径,
//!   已被 mailbox/toast_stack 取代)。
//! - 去掉了 `should_trigger_notification` legacy 判断(只走 mailbox 路径)。

use std::collections::HashMap;

use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity, ViewHandle};

use crate::BlocklistAIHistoryModel;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, ConversationStatusUpdate, QueuedQueryModel};
use crate::notifications::item::{
    NotificationCategory, NotificationId, NotificationItem, NotificationItems, NotificationOrigin,
    NotificationSourceAgent,
};
use crate::terminal::cli_agent_sessions::{
    CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::terminal::{CLIAgent, TerminalView};
use crate::workspace::util::is_terminal_view_in_same_tab;
use crate::workspace::{Workspace, WorkspaceRegistry};

/// 通知中心的单例 model:
/// - 在 BYOP agent 对话状态(`BlocklistAIHistoryModel`)和 CLI agent 会话状态
///   (`CLIAgentSessionsModel`)发生关键变化时往 mailbox 推通知;
/// - 维护 `pending_artifacts`(每个对话当前 turn 累积的 artifact),并在终态时
///   随通知一起 flush。
pub struct NotificationsModel {
    notifications: NotificationItems,
    /// 当前 turn 累积的 artifact;在终态(Success/Cancelled/Error)时 drain 进通知,
    /// InProgress 时清空。
    pub(crate) pending_artifacts: HashMap<AIConversationId, Vec<Artifact>>,
}

impl Entity for NotificationsModel {
    type Event = NotificationsEvent;
}

impl SingletonEntity for NotificationsModel {}

impl NotificationsModel {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, move |me, _, event, ctx| {
            me.handle_history_event(event, ctx);
        });

        let cli_sessions_model = CLIAgentSessionsModel::handle(ctx);
        ctx.subscribe_to_model(&cli_sessions_model, |me, _, event, ctx| {
            me.handle_cli_agent_session_event(event, ctx);
        });

        Self {
            notifications: NotificationItems::default(),
            pending_artifacts: HashMap::new(),
        }
    }

    pub(crate) fn notifications(&self) -> &NotificationItems {
        &self.notifications
    }

    pub(crate) fn mark_item_read(&mut self, id: NotificationId, ctx: &mut ModelContext<Self>) {
        if self.notifications.mark_item_read(id) {
            ctx.emit(NotificationsEvent::NotificationUpdated);
        }
    }

    pub(crate) fn mark_all_items_read(&mut self, ctx: &mut ModelContext<Self>) {
        if self.notifications.mark_all_items_read() {
            ctx.emit(NotificationsEvent::AllNotificationsMarkedRead);
        }
    }

    /// 把指定 terminal view 上的所有通知标记为已读。
    pub(crate) fn mark_items_from_terminal_view_read(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::HOANotifications.is_enabled() {
            return;
        }
        if self
            .notifications
            .mark_all_terminal_view_items_as_read(terminal_view_id)
        {
            ctx.emit(NotificationsEvent::NotificationUpdated);
        }
    }

    fn handle_cli_agent_session_event(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::HOANotifications.is_enabled() {
            return;
        }

        match event {
            CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            } => {
                self.remove_notification_by_source(
                    NotificationOrigin::CLISession(*terminal_view_id),
                    ctx,
                );
            }
            CLIAgentSessionsModelEvent::Started { .. }
            | CLIAgentSessionsModelEvent::InputSessionChanged { .. }
            | CLIAgentSessionsModelEvent::SessionUpdated { .. } => {}
            CLIAgentSessionsModelEvent::StatusChanged {
                terminal_view_id,
                agent,
                status,
                session_context,
            } => match status {
                // agent 重新开始干活 → 之前的通知作废。
                CLIAgentSessionStatus::InProgress => {
                    self.remove_notification_by_source(
                        NotificationOrigin::CLISession(*terminal_view_id),
                        ctx,
                    );
                }
                CLIAgentSessionStatus::Success => {
                    let title = session_context.display_title().unwrap_or_else(|| {
                        crate::t!(
                            "notifications-agent-completed-title",
                            agent = agent.display_name()
                        )
                    });
                    let message = match agent {
                        CLIAgent::Codex | CLIAgent::DeepSeek | CLIAgent::Antigravity => {
                            crate::t!("notifications-from-agent", agent = agent.display_name())
                        }
                        _ => crate::t!("notifications-task-completed"),
                    };
                    let metadata = TerminalViewMetadata::lookup(*terminal_view_id, ctx);
                    self.add_notification(
                        title,
                        message,
                        NotificationCategory::Complete,
                        NotificationSourceAgent::CLI {
                            agent: *agent,
                            is_ambient: metadata.is_ambient,
                        },
                        NotificationOrigin::CLISession(*terminal_view_id),
                        *terminal_view_id,
                        vec![],
                        metadata.branch,
                        ctx,
                    );
                }
                CLIAgentSessionStatus::Failed {
                    error_type,
                    message,
                } => {
                    let title = session_context.display_title().unwrap_or_else(|| {
                        crate::t!(
                            "notifications-agent-failed-title",
                            agent = agent.display_name()
                        )
                    });
                    let body = match (message.as_deref(), error_type.as_deref()) {
                        (Some(msg), Some(kind)) => format!("{kind}: {msg}"),
                        (Some(msg), None) => msg.to_owned(),
                        (None, Some(kind)) => kind.to_owned(),
                        (None, None) => crate::t!("notifications-agent-error"),
                    };
                    let metadata = TerminalViewMetadata::lookup(*terminal_view_id, ctx);
                    self.add_notification(
                        title,
                        body,
                        NotificationCategory::Error,
                        NotificationSourceAgent::CLI {
                            agent: *agent,
                            is_ambient: metadata.is_ambient,
                        },
                        NotificationOrigin::CLISession(*terminal_view_id),
                        *terminal_view_id,
                        vec![],
                        metadata.branch,
                        ctx,
                    );
                }
                CLIAgentSessionStatus::Blocked { message } => {
                    let title = session_context.display_title().unwrap_or_else(|| {
                        crate::t!(
                            "notifications-agent-needs-attention-title",
                            agent = agent.display_name()
                        )
                    });
                    let metadata = TerminalViewMetadata::lookup(*terminal_view_id, ctx);
                    self.add_notification(
                        title,
                        message
                            .clone()
                            .unwrap_or_else(|| crate::t!("notifications-waiting-for-input")),
                        NotificationCategory::Request,
                        NotificationSourceAgent::CLI {
                            agent: *agent,
                            is_ambient: metadata.is_ambient,
                        },
                        NotificationOrigin::CLISession(*terminal_view_id),
                        *terminal_view_id,
                        vec![],
                        metadata.branch,
                        ctx,
                    );
                }
                CLIAgentSessionStatus::Cancelled => {
                    let title = session_context.display_title().unwrap_or_else(|| {
                        crate::t!(
                            "notifications-agent-cancelled-title",
                            agent = agent.display_name()
                        )
                    });
                    let metadata = TerminalViewMetadata::lookup(*terminal_view_id, ctx);
                    self.add_notification(
                        title,
                        crate::t!("notifications-cancelled-by-user"),
                        NotificationCategory::Complete,
                        NotificationSourceAgent::CLI {
                            agent: *agent,
                            is_ambient: metadata.is_ambient,
                        },
                        NotificationOrigin::CLISession(*terminal_view_id),
                        *terminal_view_id,
                        vec![],
                        metadata.branch,
                        ctx,
                    );
                }
            },
        }
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // 对话被显式删除 / ephemeral 清理时,顺手清掉它的通知和 pending artifact。
        if let BlocklistAIHistoryEvent::DeletedConversation {
            conversation_id, ..
        }
        | BlocklistAIHistoryEvent::RemoveConversation {
            conversation_id, ..
        } = event
        {
            if FeatureFlag::HOANotifications.is_enabled() {
                self.pending_artifacts.remove(conversation_id);
                self.remove_notification_by_source(
                    NotificationOrigin::Conversation(*conversation_id),
                    ctx,
                );
            }
            return;
        }

        // Artifact 在 turn 内增量到达时累积起来。
        if let BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
            conversation_id,
            artifact,
            ..
        } = event
        {
            if FeatureFlag::HOANotifications.is_enabled() {
                self.pending_artifacts
                    .entry(*conversation_id)
                    .or_default()
                    .push(artifact.clone());
            }
            return;
        }

        let BlocklistAIHistoryEvent::UpdatedConversationStatus {
            terminal_surface_id,
            conversation_id,
            // 启动恢复对话不应触发通知。
            update: ConversationStatusUpdate::Changed { .. },
            ..
        } = event
        else {
            return;
        };

        if !FeatureFlag::HOANotifications.is_enabled() {
            return;
        }

        let ai_history_model = BlocklistAIHistoryModel::as_ref(ctx);
        let Some(updated_conversation) = ai_history_model.conversation(conversation_id) else {
            return;
        };

        if updated_conversation.should_exclude_from_navigation() {
            return;
        }

        let status = updated_conversation.status().clone();
        let latest_query = updated_conversation.latest_user_query();
        self.handle_history_event_for_mailbox(
            &status,
            *conversation_id,
            latest_query,
            *terminal_surface_id,
            ctx,
        );
    }

    fn handle_history_event_for_mailbox(
        &mut self,
        status: &ConversationStatus,
        conversation_id: AIConversationId,
        latest_query: Option<String>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let origin = NotificationOrigin::Conversation(conversation_id);

        // 对话在内存里已经不存在(被 evict / 删除) → 没有可导航的目标,直接清掉相关通知。
        // 这里替代了原 `ActiveAgentViewsModel::is_conversation_open` 检查。
        if BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_none()
        {
            self.pending_artifacts.remove(&conversation_id);
            self.remove_notification_by_source(origin, ctx);
            return;
        }

        let title = latest_query.unwrap_or_else(|| crate::t!("terminal-agent-task"));
        let metadata = TerminalViewMetadata::lookup(terminal_view_id, ctx);
        let oz_agent = NotificationSourceAgent::Oz {
            is_ambient: metadata.is_ambient,
        };

        match status {
            // agent 重新开始干活(或正在从瞬时错误中自动恢复)→ 之前的通知作废。
            ConversationStatus::InProgress | ConversationStatus::TransientError => {
                self.remove_notification_by_source(origin, ctx);
            }
            ConversationStatus::Success => {
                // Suppress the completion notification when a queued follow-up prompt will
                // auto-send as soon as this conversation finishes. The conversation isn't
                // really in a stopped state, so the notification would be noisy. Pending
                // artifacts are left intact so they roll into the notification fired when the
                // conversation eventually finishes with an empty queue.
                if QueuedQueryModel::as_ref(ctx).has_autofireable_prompt(conversation_id) {
                    return;
                }
                let artifacts = self.flush_pending_artifacts(conversation_id);
                self.add_notification(
                    title,
                    crate::t!("notifications-task-completed"),
                    NotificationCategory::Complete,
                    oz_agent,
                    origin,
                    terminal_view_id,
                    artifacts,
                    metadata.branch,
                    ctx,
                );
            }
            ConversationStatus::Cancelled => {
                let artifacts = self.flush_pending_artifacts(conversation_id);
                self.add_notification(
                    title,
                    crate::t!("notifications-task-cancelled"),
                    NotificationCategory::Complete,
                    oz_agent,
                    origin,
                    terminal_view_id,
                    artifacts,
                    metadata.branch,
                    ctx,
                );
            }
            ConversationStatus::Blocked { blocked_action } => {
                self.add_notification(
                    title,
                    blocked_action.clone(),
                    NotificationCategory::Request,
                    oz_agent,
                    origin,
                    terminal_view_id,
                    vec![],
                    metadata.branch,
                    ctx,
                );
            }
            ConversationStatus::Error => {
                let artifacts = self.flush_pending_artifacts(conversation_id);
                self.add_notification(
                    title,
                    crate::t!("notifications-something-went-wrong"),
                    NotificationCategory::Error,
                    oz_agent,
                    origin,
                    terminal_view_id,
                    artifacts,
                    metadata.branch,
                    ctx,
                );
            }
            // Yielded conversations are still active; mirror the
            // InProgress arm and clear any stale notification for this
            // origin.
            ConversationStatus::WaitingForEvents => {
                self.remove_notification_by_source(origin, ctx);
            }
        }
    }

    /// 删除指定 source 的现有通知(若有),并 emit 更新事件。
    fn remove_notification_by_source(
        &mut self,
        origin: NotificationOrigin,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.notifications.remove_by_origin(origin) {
            ctx.emit(NotificationsEvent::NotificationUpdated);
        }
    }

    /// drain 出指定对话当前 turn 累积的 artifact。
    pub(crate) fn flush_pending_artifacts(
        &mut self,
        conversation_id: AIConversationId,
    ) -> Vec<Artifact> {
        self.pending_artifacts
            .remove(&conversation_id)
            .unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn add_notification(
        &mut self,
        title: String,
        message: String,
        category: NotificationCategory,
        agent: NotificationSourceAgent,
        origin: NotificationOrigin,
        terminal_view_id: EntityId,
        artifacts: Vec<Artifact>,
        branch: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let is_visible = is_terminal_view_visible(terminal_view_id, ctx);
        let item = NotificationItem::new(
            title,
            message,
            category,
            agent,
            origin,
            is_visible,
            terminal_view_id,
            artifacts,
            branch,
        );

        let id = item.id;
        self.notifications.push(item);
        ctx.emit(NotificationsEvent::NotificationAdded { id });
    }
}

#[derive(Clone, Debug)]
pub enum NotificationsEvent {
    /// 通知中心新增了一条通知。
    NotificationAdded { id: NotificationId },
    /// 通知的已读状态变了。
    NotificationUpdated,
    /// 全部标记为已读。
    AllNotificationsMarkedRead,
}

fn is_terminal_view_visible(terminal_view_id: EntityId, app: &AppContext) -> bool {
    let Some(active_id) = active_focused_terminal_id(app) else {
        return false;
    };
    active_id == terminal_view_id
        || is_terminal_view_in_same_tab(&active_id, &terminal_view_id, app)
}

/// Per-notification metadata derived from a single [`TerminalView`] lookup. Both fields
/// are read on the same emit path, so we resolve the view once and pass the projection
/// down rather than walking the workspace tree for each.
struct TerminalViewMetadata {
    is_ambient: bool,
    branch: Option<String>,
}

impl TerminalViewMetadata {
    fn lookup(terminal_view_id: EntityId, app: &AppContext) -> Self {
        let Some(terminal_view) = find_terminal_view_by_id(terminal_view_id, app) else {
            return Self {
                is_ambient: false,
                branch: None,
            };
        };
        let view = terminal_view.as_ref(app);
        Self {
            is_ambient: view.is_ambient_agent_session(app),
            branch: view.current_git_branch(app),
        }
    }
}

fn find_terminal_view_by_id(
    terminal_view_id: EntityId,
    app: &AppContext,
) -> Option<ViewHandle<TerminalView>> {
    for (_, workspace_handle) in WorkspaceRegistry::as_ref(app).all_workspaces(app) {
        for pane_group in workspace_handle.as_ref(app).tab_views() {
            let pane_group = pane_group.as_ref(app);
            for pane_id in pane_group.terminal_pane_ids() {
                if let Some(terminal_view) = pane_group.terminal_view_from_pane_id(pane_id, app)
                    && terminal_view.id() == terminal_view_id
                {
                    return Some(terminal_view);
                }
            }
        }
    }
    None
}

fn active_focused_terminal_id(app: &AppContext) -> Option<EntityId> {
    let active_window = app.windows().active_window()?;
    let workspace = app
        .views_of_type::<Workspace>(active_window)
        .and_then(|views| views.first().cloned())?;

    let workspace = workspace.as_ref(app);
    workspace.active_terminal_id(app)
}
