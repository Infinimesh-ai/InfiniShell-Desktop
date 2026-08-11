use enum_iterator::Sequence;
use instant::Instant;
use uuid::Uuid;
use warpui::EntityId;

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::artifacts::Artifact;
use crate::terminal::CLIAgent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotificationId(Uuid);

impl NotificationId {
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationCategory {
    /// 任务完成(成功 / 取消)
    Complete,
    /// 需要用户介入(权限请求或 idle prompt)
    Request,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Sequence)]
pub enum NotificationFilter {
    All,
    Unread,
    Errors,
}

impl NotificationFilter {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            NotificationFilter::All => "All tabs",
            NotificationFilter::Unread => "Unread",
            NotificationFilter::Errors => "Errors",
        }
    }
}

/// 通知发出方。`Oz` 是 Zap 自家本地 BYOP agent;`CLI(...)` 是第三方 CLI agent
/// (Claude Code / Codex / DeepSeek 等)。
/// `is_ambient` 决定 [`render_agent_avatar`] 是否渲染 cloud lobe。
#[derive(Debug, Clone, Copy)]
#[allow(clippy::upper_case_acronyms)]
pub enum NotificationSourceAgent {
    Oz { is_ambient: bool },
    CLI { agent: CLIAgent, is_ambient: bool },
}

impl NotificationSourceAgent {
    pub fn is_ambient(&self) -> bool {
        match self {
            NotificationSourceAgent::Oz { is_ambient }
            | NotificationSourceAgent::CLI { is_ambient, .. } => *is_ambient,
        }
    }
}

/// 标识这条通知所属的对话或会话。
/// 用于:
/// - 去重(同一 origin 的新通知会替换旧的)
/// - 清理(对话/会话关闭时一并清掉相关通知)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationOrigin {
    Conversation(AIConversationId),
    /// CLI session 按 terminal view id 区分(每个 pane 至多一个 CLI agent session)。
    CLISession(EntityId),
}

#[derive(Debug, Clone)]
pub struct NotificationItem {
    pub id: NotificationId,
    pub origin: NotificationOrigin,
    pub title: String,
    pub message: String,
    pub category: NotificationCategory,
    pub agent: NotificationSourceAgent,
    /// 用户是否已读
    /// (点过这条通知,或者已经导航到对应对话/会话)。
    pub is_read: bool,
    pub created_at: Instant,
    pub terminal_view_id: EntityId,
    pub artifacts: Vec<Artifact>,
    /// 通知关联的 git 分支。
    /// 有值时按"rich"布局渲染(头部多一行 branch);无值时回退到"simple"布局。
    pub branch: Option<String>,
}

impl NotificationItem {
    /// 标记为已读;若先前是未读则返回 true。
    fn mark_as_read(&mut self) -> bool {
        if self.is_read {
            return false;
        }
        self.is_read = true;
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        title: String,
        message: String,
        category: NotificationCategory,
        agent: NotificationSourceAgent,
        origin: NotificationOrigin,
        is_read: bool,
        terminal_view_id: EntityId,
        artifacts: Vec<Artifact>,
        branch: Option<String>,
    ) -> Self {
        Self {
            id: NotificationId::new(),
            origin,
            title,
            message,
            category,
            agent,
            is_read,
            created_at: Instant::now(),
            terminal_view_id,
            artifacts,
            branch,
        }
    }
}

#[derive(Debug, Default)]
pub struct NotificationItems {
    items: Vec<NotificationItem>,
}

impl NotificationItems {
    /// 把新通知插到列表头(同时按 origin 去重,并截断最多 100 条)。
    pub(crate) fn push(&mut self, item: NotificationItem) {
        self.remove_by_origin(item.origin);
        self.items.insert(0, item);
        self.items.truncate(100);
    }

    pub(crate) fn remove_by_origin(&mut self, key: NotificationOrigin) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.origin != key);
        self.items.len() != before
    }

    pub(crate) fn items_filtered(
        &self,
        filter: NotificationFilter,
    ) -> impl Iterator<Item = &NotificationItem> {
        self.items.iter().filter(move |item| match filter {
            NotificationFilter::All => true,
            NotificationFilter::Unread => !item.is_read,
            NotificationFilter::Errors => item.category == NotificationCategory::Error,
        })
    }

    pub(crate) fn filtered_count(&self, filter: NotificationFilter) -> usize {
        self.items_filtered(filter).count()
    }

    /// 返回顶部应当显示的过滤器 tab。"All" 始终显示,
    /// 其它过滤器只在至少有一条匹配项时显示。
    pub(crate) fn visible_filters(&self) -> Vec<NotificationFilter> {
        enum_iterator::all::<NotificationFilter>()
            .filter(|f| *f == NotificationFilter::All || self.filtered_count(*f) > 0)
            .collect()
    }

    pub(crate) fn get_by_id(&self, id: NotificationId) -> Option<&NotificationItem> {
        self.items.iter().find(|item| item.id == id)
    }

    /// 把指定 terminal view 上的所有通知标记为已读;有变更则返回 true。
    pub(crate) fn mark_all_terminal_view_items_as_read(
        &mut self,
        terminal_view_id: EntityId,
    ) -> bool {
        let mut any_changed = false;
        for item in &mut self.items {
            if item.terminal_view_id == terminal_view_id {
                any_changed |= item.mark_as_read();
            }
        }
        any_changed
    }

    pub(crate) fn mark_item_read(&mut self, id: NotificationId) -> bool {
        self.items
            .iter_mut()
            .find(|item| item.id == id)
            .is_some_and(|item| item.mark_as_read())
    }

    pub(crate) fn mark_all_items_read(&mut self) -> bool {
        let mut any_changed = false;
        for item in &mut self.items {
            any_changed |= item.mark_as_read();
        }
        any_changed
    }

    pub(crate) fn has_unread_for_terminal_view(&self, terminal_view_id: EntityId) -> bool {
        self.items
            .iter()
            .any(|item| item.terminal_view_id == terminal_view_id && !item.is_read)
    }
}

#[cfg(test)]
#[path = "item_tests.rs"]
mod tests;
