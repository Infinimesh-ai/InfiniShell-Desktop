#[allow(dead_code)]
pub mod entry;
mod query;

use std::collections::{HashMap, HashSet};

use clap::ValueEnum;
pub use entry::{
    AgentConversationEntry, AgentConversationEntryId, AgentConversationNavigationSubject,
    AgentConversationProvenance,
};
use fuzzy_match::FuzzyMatchResult;
use itertools::Itertools;
pub use query::query_conversation_entries;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::WarpTheme;
use warp_core::ui::theme::color::internal_colors;
use warpui::color::ColorU;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity, WindowId};

use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::ambient_agents::{
    AgentSource, AmbientAgentLiveSessionState, AmbientAgentTask, AmbientAgentTaskId,
    AmbientAgentTaskState,
};
use crate::ai::artifacts::Artifact;
use crate::ai::blocklist::orchestration_topology::orchestration_aware_conversation_status;
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, ConversationStatusUpdate,
};
use crate::ai::conversation_navigation::ConversationNavigationData;
use crate::auth::AuthStateProvider;
use crate::ui_components::icons::Icon;
use crate::workspace::{RestoreConversationLayout, WorkspaceAction};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    Available,
    Expired,
    Unavailable,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum StatusFilter {
    #[default]
    All,
    Working,
    Done,
    Failed,
}

impl StatusFilter {
    /// Returns `true` if a status transition from `prev_bucket` to `new_bucket` flips
    /// whether an item is included by this filter. `All` matches every bucket so it
    /// is never crossed; the other variants are crossed when exactly one of the buckets
    /// equals this filter.
    pub(crate) fn is_membership_crossed(
        self,
        prev_bucket: StatusFilter,
        new_bucket: StatusFilter,
    ) -> bool {
        match self {
            StatusFilter::All => false,
            StatusFilter::Working | StatusFilter::Done | StatusFilter::Failed => {
                (prev_bucket == self) != (new_bucket == self)
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SourceFilter {
    #[default]
    All,
    Specific(AgentSource),
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum CreatorFilter {
    #[default]
    All,
    Specific {
        name: String,
        uid: String,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ArtifactFilter {
    #[default]
    All,
    PullRequest,
    Plan,
    Screenshot,
    File,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum CreatedOnFilter {
    #[default]
    All,
    Last24Hours,
    Past3Days,
    LastWeek,
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum EnvironmentFilter {
    #[default]
    All,
    NoEnvironment,
    Specific(String),
}

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnerFilter {
    All,
    #[default]
    PersonalOnly,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum HarnessFilter {
    #[default]
    All,
    Specific(Harness),
}

impl Serialize for HarnessFilter {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            HarnessFilter::All => serializer.serialize_str("all"),
            HarnessFilter::Specific(harness) => serializer.collect_str(harness),
        }
    }
}

impl<'de> Deserialize<'de> for HarnessFilter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Harness::from_str(&raw, false)
            .ok()
            .map(HarnessFilter::Specific)
            .unwrap_or(HarnessFilter::All))
    }
}

#[derive(Default, PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub struct AgentManagementFilters {
    pub owners: OwnerFilter,
    pub status: StatusFilter,
    pub source: SourceFilter,
    pub created_on: CreatedOnFilter,
    pub creator: CreatorFilter,
    pub artifact: ArtifactFilter,
    #[serde(default)]
    pub environment: EnvironmentFilter,
    #[serde(default)]
    pub harness: HarnessFilter,
}

/// Frontend-specific classification of a normalized conversation-list entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentConversationListEntryState {
    Selected,
    OpenElsewhere,
    Available,
    Unavailable,
}

/// Per-frontend policy for classifying normalized conversation-list entries.
pub trait AgentConversationListPolicy: 'static {
    /// Classifies `entry` as selected, open elsewhere, available, or unavailable.
    fn classify_entry(
        &self,
        entry: &AgentConversationEntry,
        app: &AppContext,
    ) -> AgentConversationListEntryState;
}

/// A normalized conversation entry paired with optional title-match metadata.
pub struct AgentConversationQueryResult {
    pub entry: AgentConversationEntry,
    pub title_match: Option<FuzzyMatchResult>,
}

impl AgentManagementFilters {
    pub fn reset_all_but_owner(&mut self) {
        self.status = StatusFilter::default();
        self.source = SourceFilter::default();
        self.created_on = CreatedOnFilter::default();
        self.creator = CreatorFilter::default();
        self.artifact = ArtifactFilter::default();
        self.environment = EnvironmentFilter::default();
        self.harness = HarnessFilter::default();
    }

    pub fn is_filtering(&self) -> bool {
        self.status != StatusFilter::default()
            || self.source != SourceFilter::default()
            || self.created_on != CreatedOnFilter::default()
            || self.creator != CreatorFilter::default() && self.owners != OwnerFilter::PersonalOnly
            || self.artifact != ArtifactFilter::default()
            || self.environment != EnvironmentFilter::default()
            || self.harness != HarnessFilter::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRunDisplayStatus {
    /// Raw task-service lifecycle states. `from_task` only returns `TaskInProgress` while the
    /// task still has an active execution, or when there is no shadowed local conversation to
    /// provide a more granular status.
    TaskQueued,
    TaskPending,
    TaskClaimed,
    TaskInProgress,
    TaskSucceeded,
    TaskFailed,
    TaskError,
    TaskBlocked {
        blocked_action: String,
    },
    TaskCancelled,
    TaskUnknown,
    /// Conversation-derived lifecycle states, used for interactive conversations and for
    /// in-progress ambient tasks after they can be resolved to their shadowed local conversation.
    ConversationInProgress,
    ConversationSucceeded,
    ConversationError,
    ConversationBlocked {
        blocked_action: String,
    },
    ConversationCancelled,
}

impl AgentRunDisplayStatus {
    pub fn from_task(task: &AmbientAgentTask, app: &AppContext) -> Self {
        match &task.state {
            AmbientAgentTaskState::Queued
            | AmbientAgentTaskState::Pending
            | AmbientAgentTaskState::Claimed => Self::from_task_state(task),
            AmbientAgentTaskState::InProgress => {
                if task.has_active_execution() {
                    return Self::from_task_state(task);
                }
                let history_model = BlocklistAIHistoryModel::as_ref(app);
                entry::conversation_id_shadowed_by_task(task, history_model)
                    .and_then(|conversation_id| history_model.conversation(&conversation_id))
                    .map(|conversation| {
                        // Roll the whole orchestration subtree (children,
                        // grandchildren, …) into the root card's status.
                        Self::from_conversation_status(&orchestration_aware_conversation_status(
                            history_model,
                            conversation,
                        ))
                    })
                    .unwrap_or_else(|| Self::from_task_state(task))
            }
            AmbientAgentTaskState::Succeeded
            | AmbientAgentTaskState::Failed
            | AmbientAgentTaskState::Error
            | AmbientAgentTaskState::Blocked
            | AmbientAgentTaskState::Cancelled
            | AmbientAgentTaskState::Unknown => Self::from_task_state(task),
        }
    }

    pub fn from_conversation_status(status: &ConversationStatus) -> Self {
        match status {
            ConversationStatus::InProgress => Self::ConversationInProgress,
            // A recovery is in flight; the run is still working.
            ConversationStatus::TransientError => Self::ConversationInProgress,
            ConversationStatus::Success => Self::ConversationSucceeded,
            ConversationStatus::Error => Self::ConversationError,
            ConversationStatus::Cancelled => Self::ConversationCancelled,
            ConversationStatus::Blocked { blocked_action } => Self::ConversationBlocked {
                blocked_action: blocked_action.clone(),
            },
            // Treat a yielded conversation as still in progress for the
            // agent-run list display so it stays in the working bucket.
            ConversationStatus::WaitingForEvents => Self::ConversationInProgress,
        }
    }

    fn from_task_state(task: &AmbientAgentTask) -> Self {
        match &task.state {
            AmbientAgentTaskState::Queued => Self::TaskQueued,
            AmbientAgentTaskState::Pending => Self::TaskPending,
            AmbientAgentTaskState::Claimed => Self::TaskClaimed,
            AmbientAgentTaskState::InProgress => Self::TaskInProgress,
            AmbientAgentTaskState::Succeeded => Self::TaskSucceeded,
            AmbientAgentTaskState::Failed => Self::TaskFailed,
            AmbientAgentTaskState::Error => Self::TaskError,
            AmbientAgentTaskState::Blocked => Self::TaskBlocked {
                blocked_action: task
                    .status_message
                    .as_ref()
                    .map(|m| m.message.clone())
                    .unwrap_or_else(|| "Task blocked".to_string()),
            },
            AmbientAgentTaskState::Cancelled => Self::TaskCancelled,
            AmbientAgentTaskState::Unknown => Self::TaskUnknown,
        }
    }

    pub fn status_filter(&self) -> StatusFilter {
        match self {
            AgentRunDisplayStatus::TaskQueued
            | AgentRunDisplayStatus::TaskPending
            | AgentRunDisplayStatus::TaskClaimed
            | AgentRunDisplayStatus::TaskInProgress
            | AgentRunDisplayStatus::ConversationInProgress => StatusFilter::Working,
            AgentRunDisplayStatus::TaskSucceeded | AgentRunDisplayStatus::ConversationSucceeded => {
                StatusFilter::Done
            }
            AgentRunDisplayStatus::TaskFailed
            | AgentRunDisplayStatus::TaskError
            | AgentRunDisplayStatus::TaskBlocked { .. }
            | AgentRunDisplayStatus::TaskCancelled
            | AgentRunDisplayStatus::TaskUnknown
            | AgentRunDisplayStatus::ConversationError
            | AgentRunDisplayStatus::ConversationBlocked { .. }
            | AgentRunDisplayStatus::ConversationCancelled => StatusFilter::Failed,
        }
    }

    pub fn to_conversation_status(&self) -> ConversationStatus {
        match self {
            AgentRunDisplayStatus::TaskQueued
            | AgentRunDisplayStatus::TaskPending
            | AgentRunDisplayStatus::TaskClaimed
            | AgentRunDisplayStatus::TaskInProgress
            | AgentRunDisplayStatus::ConversationInProgress => ConversationStatus::InProgress,
            AgentRunDisplayStatus::TaskSucceeded | AgentRunDisplayStatus::ConversationSucceeded => {
                ConversationStatus::Success
            }
            AgentRunDisplayStatus::TaskFailed
            | AgentRunDisplayStatus::TaskError
            | AgentRunDisplayStatus::TaskUnknown
            | AgentRunDisplayStatus::ConversationError => ConversationStatus::Error,
            AgentRunDisplayStatus::TaskBlocked { blocked_action }
            | AgentRunDisplayStatus::ConversationBlocked { blocked_action } => {
                ConversationStatus::Blocked {
                    blocked_action: blocked_action.clone(),
                }
            }
            AgentRunDisplayStatus::TaskCancelled | AgentRunDisplayStatus::ConversationCancelled => {
                ConversationStatus::Cancelled
            }
        }
    }

    pub fn is_cancellable(&self) -> bool {
        self.is_working()
    }

    pub fn is_working(&self) -> bool {
        matches!(
            self,
            AgentRunDisplayStatus::TaskQueued
                | AgentRunDisplayStatus::TaskPending
                | AgentRunDisplayStatus::TaskClaimed
                | AgentRunDisplayStatus::TaskInProgress
                | AgentRunDisplayStatus::ConversationInProgress
        )
    }

    pub fn status_icon_and_color(&self, theme: &WarpTheme) -> (Icon, ColorU) {
        match self {
            AgentRunDisplayStatus::TaskQueued
            | AgentRunDisplayStatus::TaskPending
            | AgentRunDisplayStatus::TaskClaimed
            | AgentRunDisplayStatus::TaskInProgress
            | AgentRunDisplayStatus::ConversationInProgress => {
                (Icon::ClockLoader, theme.ansi_fg_magenta())
            }
            AgentRunDisplayStatus::TaskSucceeded | AgentRunDisplayStatus::ConversationSucceeded => {
                (Icon::Check, theme.ansi_fg_green())
            }
            AgentRunDisplayStatus::TaskFailed
            | AgentRunDisplayStatus::TaskError
            | AgentRunDisplayStatus::TaskUnknown
            | AgentRunDisplayStatus::ConversationError => (Icon::Triangle, theme.ansi_fg_red()),
            AgentRunDisplayStatus::TaskBlocked { .. }
            | AgentRunDisplayStatus::ConversationBlocked { .. } => {
                (Icon::StopFilled, theme.ansi_fg_yellow())
            }
            AgentRunDisplayStatus::TaskCancelled => (
                Icon::Cancelled,
                theme.disabled_text_color(theme.background()).into_solid(),
            ),
            AgentRunDisplayStatus::ConversationCancelled => {
                (Icon::StopFilled, internal_colors::neutral_5(theme))
            }
        }
    }
}

impl std::fmt::Display for AgentRunDisplayStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentRunDisplayStatus::TaskQueued => write!(f, "Queued"),
            AgentRunDisplayStatus::TaskPending => write!(f, "Pending"),
            AgentRunDisplayStatus::TaskClaimed => write!(f, "Claimed"),
            AgentRunDisplayStatus::TaskInProgress
            | AgentRunDisplayStatus::ConversationInProgress => write!(f, "In progress"),
            AgentRunDisplayStatus::TaskSucceeded | AgentRunDisplayStatus::ConversationSucceeded => {
                write!(f, "Done")
            }
            AgentRunDisplayStatus::TaskFailed => write!(f, "Failed"),
            AgentRunDisplayStatus::TaskError | AgentRunDisplayStatus::ConversationError => {
                write!(f, "Error")
            }
            AgentRunDisplayStatus::TaskBlocked { .. }
            | AgentRunDisplayStatus::ConversationBlocked { .. } => write!(f, "Blocked"),
            AgentRunDisplayStatus::TaskCancelled | AgentRunDisplayStatus::ConversationCancelled => {
                write!(f, "Cancelled")
            }
            AgentRunDisplayStatus::TaskUnknown => write!(f, "Failed"),
        }
    }
}

/// Stores conversation metadata needed for display in conversation/task views.
pub struct ConversationMetadata {
    pub nav_data: ConversationNavigationData,
}

pub(crate) fn artifacts_match_filter(
    artifacts: &[Artifact],
    artifact_filter: &ArtifactFilter,
) -> bool {
    match artifact_filter {
        ArtifactFilter::All => true,
        ArtifactFilter::PullRequest => artifacts
            .iter()
            .any(|artifact| matches!(artifact, Artifact::PullRequest { .. })),
        ArtifactFilter::Plan => artifacts
            .iter()
            .any(|artifact| matches!(artifact, Artifact::Plan { .. })),
        ArtifactFilter::Screenshot => artifacts
            .iter()
            .any(|artifact| matches!(artifact, Artifact::Screenshot { .. })),
        ArtifactFilter::File => artifacts
            .iter()
            .any(|artifact| matches!(artifact, Artifact::File { .. })),
    }
}

/// This model serves as a unified interface for reading both local and ambient agent conversations
/// (i.e. conversations & tasks).
///
/// Zap(本地优先):上游在这里维护 30s 轮询 + RTC 失效 + 云端 task/conversation metadata 拉取。
/// Zap 删除了账号体系与云端 AI 网关,因此该模型只做本地聚合:tasks 仅来自本地 BYOP /
/// ambient agent 运行时写入,conversations 来自 `ConversationNavigationData` 与历史库。
///
/// This model backs both the agent management view and the conversation list view.
pub struct AgentConversationsModel {
    /// A map of task IDs to agent tasks.
    tasks: HashMap<AmbientAgentTaskId, AmbientAgentTask>,
    /// A map of conversation IDs to local conversations.
    conversations: HashMap<AIConversationId, ConversationMetadata>,
    /// Set of view IDs actively consuming this model's data per window.
    /// Zap:本地化后无轮询,仅作为 register_view_open/closed 的占位记录使用。
    active_data_consumers_per_window: HashMap<WindowId, HashSet<EntityId>>,
    /// Whether we have finished the initial (local) conversation load.
    has_finished_initial_load: bool,
    /// Task IDs that have been manually opened from the management page.
    /// These will appear in the conversation list even if their source is not user-initiated
    /// (and even after they have been closed).
    manually_opened_task_ids: HashSet<AmbientAgentTaskId>,
}

pub enum AgentConversationsModelEvent {
    /// Conversation data was loaded or refreshed.
    ConversationsLoaded,
    /// New tasks were received (view should diff against its local state).
    /// Zap:本地化后没有轮询来源,保留给未来的本地 task 批量注入路径与既有订阅方。
    NewTasksReceived,
    /// Existing task data may have been updated (e.g., state changes).
    TasksUpdated,
    /// Conversation status data was updated
    ConversationUpdated { kind: ConversationUpdateKind },
    /// Conversation artifacts were updated (plans, PRs, etc.)
    ConversationArtifactsUpdated { conversation_id: AIConversationId },
    /// A task was manually opened from the management page.
    TaskManuallyOpened,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationUpdateKind {
    /// The conversation was re-loaded into a terminal view.
    Restored,
    /// The conversation's status was set.
    StatusSet {
        prev_filter: StatusFilter,
        new_filter: StatusFilter,
    },
    /// Conversation metadata or capabilities changed.
    MetadataChanged,
    /// Conversation title changed.
    TitleChanged,
}

impl Entity for AgentConversationsModel {
    type Event = AgentConversationsModelEvent;
}

impl SingletonEntity for AgentConversationsModel {}

impl AgentConversationsModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Zap(本地化,Phase 3b-1 / Wave 6-6):上游在这里订阅 NetworkStatus / WindowManager /
        // AuthManager / UpdateManager 来驱动云端轮询与 RTC 失效。这些子系统在 Zap 中已物理删除,
        // 只保留本地数据源的订阅。
        //
        // Issue #93 修复:必须订阅 BlocklistAIHistoryModel 的事件,否则用户在历史对话
        // 列表中删除对话后,本模型缓存的 conversations 不会刷新,UI 将持续展示已删除的项。
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, move |me, _, event, ctx| {
            me.handle_history_event(event, ctx);
        });

        let active_views_model = ActiveAgentViewsModel::handle(ctx);
        ctx.subscribe_to_model(&active_views_model, |me, _, _event, ctx| {
            me.sync_conversations(ctx);
        });

        let mut model = Self {
            tasks: HashMap::new(),
            conversations: HashMap::new(),
            active_data_consumers_per_window: HashMap::new(),
            has_finished_initial_load: false,
            manually_opened_task_ids: HashSet::new(),
        };

        model.sync_conversations(ctx);
        // 本地同步是同步完成的,没有后续的云端阶段需要等待。
        model.has_finished_initial_load = true;
        model
    }

    pub fn is_loading(&self) -> bool {
        !self.has_finished_initial_load
    }

    /// Returns whether cloud conversation metadata failed to load.
    ///
    /// Zap 不拉取云端 conversation metadata,因此永远不会进入失败态。
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    pub(crate) fn cloud_conversation_metadata_load_failed(&self) -> bool {
        false
    }

    /// Sync all conversations to the AgentConversationsModel.
    ///
    /// This function will loop through all active panes, recently closed panes, and historical
    /// conversations to construct a complete snapshot of conversations.
    pub fn sync_conversations(&mut self, ctx: &mut ModelContext<Self>) {
        if !FeatureFlag::InteractiveConversationManagementView.is_enabled() {
            return;
        }

        let nav_data_list = ConversationNavigationData::all_conversations(ctx);

        self.conversations.clear();
        for nav_data in nav_data_list {
            let conversation_id = nav_data.id;
            let metadata = ConversationMetadata { nav_data };
            self.conversations.insert(conversation_id, metadata);
        }

        ctx.emit(AgentConversationsModelEvent::ConversationsLoaded);
    }

    /// Called when a view that consumes this model's data becomes visible.
    /// Uses view_id to make registration idempotent.
    pub fn register_view_open(
        &mut self,
        window_id: WindowId,
        view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.active_data_consumers_per_window
            .entry(window_id)
            .or_default()
            .insert(view_id);
        self.sync_conversations(ctx);
    }

    /// Called when a view that consumes this model's data becomes hidden.
    /// Uses view_id to make unregistration idempotent.
    pub fn register_view_closed(
        &mut self,
        window_id: WindowId,
        view_id: EntityId,
        _ctx: &mut ModelContext<Self>,
    ) {
        if let Some(views) = self.active_data_consumers_per_window.get_mut(&window_id) {
            views.remove(&view_id);
            if views.is_empty() {
                self.active_data_consumers_per_window.remove(&window_id);
            }
        }
    }

    /// Returns whether the unfiltered conversation list contains any entries.
    pub fn has_items(&self, app: &AppContext) -> bool {
        !self.unfiltered_entries(app).is_empty()
    }

    /// Returns an iterator over all ambient agent tasks.
    pub fn tasks_iter(&self) -> impl Iterator<Item = &AmbientAgentTask> {
        self.tasks.values()
    }

    #[cfg(test)]
    pub(crate) fn insert_task_for_test(&mut self, task: AmbientAgentTask) {
        self.tasks.insert(task.task_id, task);
    }

    pub(crate) fn mark_task_execution_ended(
        &mut self,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(task) = self.tasks.get_mut(&task_id) else {
            return;
        };
        let was_active = task.has_active_execution();
        task.is_sandbox_running = false;
        if was_active {
            ctx.emit(AgentConversationsModelEvent::TasksUpdated);
        }
    }

    /// Returns normalized, owned entries for agent management/navigation surfaces.
    pub fn get_entries(
        &self,
        filters: &AgentManagementFilters,
        app: &AppContext,
    ) -> Vec<AgentConversationEntry> {
        self.unfiltered_entries(app)
            .into_iter()
            .filter(|entry| entry.matches_filters(filters, app))
            .sorted_by(|a, b| b.display.last_updated.cmp(&a.display.last_updated))
            .collect()
    }

    /// Returns normalized entries before user-selected filters are applied.
    fn unfiltered_entries(&self, app: &AppContext) -> Vec<AgentConversationEntry> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let mut entries = Vec::new();
        // Local conversation IDs represented by a task — either shown as a
        // task entry or hidden along with a child task — and therefore not
        // emitted as standalone conversation entries by the loops below.
        let mut attached_conversation_ids = HashSet::new();
        let mut emitted_conversation_ids = HashSet::new();

        for task in self.tasks.values() {
            // Child agents (runs carrying `parent_run_id`) are represented
            // under their parent's status card and must not appear as standalone
            // entries — this mirrors the local navigation path's exclusion via
            // `AIConversation::should_exclude_from_navigation`. Any local
            // conversation shadowed by a child task is hidden along with it.
            if task.parent_run_id.is_some() {
                if let Some(conversation_id) =
                    entry::conversation_id_shadowed_by_task(task, history_model)
                {
                    attached_conversation_ids.insert(conversation_id);
                }
                continue;
            }
            let entry = entry::entry_for_task(task, history_model, app);
            if let Some(conversation_id) = entry.identity.local_conversation_id {
                attached_conversation_ids.insert(conversation_id);
            }
            entries.push(entry);
        }

        for metadata in self.conversations.values() {
            let conversation_id = metadata.nav_data.id;
            if attached_conversation_ids.contains(&conversation_id) {
                continue;
            }
            let entry = entry::entry_for_conversation(metadata, history_model, app);
            emitted_conversation_ids.insert(conversation_id);
            entries.push(entry);
        }

        for metadata in history_model.get_local_conversations_metadata() {
            if attached_conversation_ids.contains(&metadata.id)
                || emitted_conversation_ids.contains(&metadata.id)
            {
                continue;
            }
            let nav_data =
                ConversationNavigationData::from_historical_conversation_metadata(metadata);
            entries.push(entry::entry_for_historical_metadata(
                metadata,
                nav_data,
                history_model,
                app,
            ));
        }

        entries
    }

    pub fn get_entry_by_id(
        &self,
        id: &AgentConversationEntryId,
        app: &AppContext,
    ) -> Option<AgentConversationEntry> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        match id {
            AgentConversationEntryId::AmbientRun(task_id) => self
                .tasks
                .get(task_id)
                .map(|task| entry::entry_for_task(task, history_model, app)),
            AgentConversationEntryId::Conversation(conversation_id) => self
                .conversations
                .get(conversation_id)
                .map(|metadata| entry::entry_for_conversation(metadata, history_model, app))
                .or_else(|| {
                    history_model
                        .get_conversation_metadata(conversation_id)
                        .map(|metadata| {
                            let nav_data =
                                ConversationNavigationData::from_historical_conversation_metadata(
                                    metadata,
                                );
                            entry::entry_for_historical_metadata(
                                metadata,
                                nav_data,
                                history_model,
                                app,
                            )
                        })
                }),
        }
    }

    pub fn resolve_open_action(
        subject: AgentConversationNavigationSubject,
        restore_layout: Option<RestoreConversationLayout>,
        app: &AppContext,
    ) -> Option<WorkspaceAction> {
        let model = Self::as_ref(app);
        match subject {
            AgentConversationNavigationSubject::Entry(id) => model
                .get_entry_by_id(&id, app)
                .and_then(|entry| model.resolve_entry_open_action(&entry, restore_layout, app)),
            AgentConversationNavigationSubject::ServerToken(server_token) => model
                .entry_for_server_token(&server_token, app)
                .and_then(|entry| model.resolve_entry_open_action(&entry, restore_layout, app))
                .or_else(|| {
                    Some(WorkspaceAction::OpenConversationTranscriptViewer {
                        ambient_agent_task_id: model.task_id_for_server_token(&server_token),
                        conversation_id: server_token,
                    })
                }),
        }
    }

    pub fn resolve_copy_link(
        subject: AgentConversationNavigationSubject,
        app: &AppContext,
    ) -> Option<String> {
        let model = Self::as_ref(app);
        match subject {
            AgentConversationNavigationSubject::Entry(id) => model
                .get_entry_by_id(&id, app)
                .and_then(|entry| model.resolve_entry_copy_link(&entry)),
            AgentConversationNavigationSubject::ServerToken(server_token) => model
                .entry_for_server_token(&server_token, app)
                .and_then(|entry| model.resolve_entry_copy_link(&entry))
                .or_else(|| Some(server_token.conversation_link())),
        }
    }

    fn resolve_entry_open_action(
        &self,
        entry: &AgentConversationEntry,
        restore_layout: Option<RestoreConversationLayout>,
        app: &AppContext,
    ) -> Option<WorkspaceAction> {
        let active_views_model = ActiveAgentViewsModel::as_ref(app);

        if let Some(task_id) = entry.identity.ambient_agent_task_id {
            match self
                .tasks
                .get(&task_id)
                .map(AmbientAgentTask::active_live_session_state)
            {
                Some(AmbientAgentLiveSessionState::Attachable { session_id }) => {
                    return Some(WorkspaceAction::OpenOrAttachAmbientAgentConversation {
                        session_id,
                        task_id,
                    });
                }
                Some(AmbientAgentLiveSessionState::ActiveUnattachable) => {
                    return active_views_model
                        .get_terminal_view_id_for_ambient_task(task_id)
                        .map(
                            |terminal_view_id| WorkspaceAction::FocusTerminalViewInWorkspace {
                                terminal_view_id,
                            },
                        );
                }
                Some(AmbientAgentLiveSessionState::Inactive) | None => {}
            }

            if let Some(terminal_view_id) =
                active_views_model.get_terminal_view_id_for_ambient_task(task_id)
            {
                return Some(WorkspaceAction::FocusTerminalViewInWorkspace { terminal_view_id });
            }
        }

        if let Some(conversation_id) = entry.identity.local_conversation_id
            && active_views_model.is_conversation_open(conversation_id, app)
        {
            if let Some(nav_data) = self
                .conversations
                .get(&conversation_id)
                .map(|metadata| &metadata.nav_data)
            {
                return Some(WorkspaceAction::RestoreOrNavigateToConversation {
                    conversation_id,
                    window_id: nav_data.window_id,
                    pane_view_locator: nav_data.pane_view_locator,
                    terminal_view_id: nav_data.terminal_view_id,
                    restore_layout,
                });
            }

            if let Some(terminal_view_id) =
                active_views_model.get_terminal_view_id_for_conversation(conversation_id, app)
            {
                return Some(WorkspaceAction::FocusTerminalViewInWorkspace { terminal_view_id });
            }
        }

        if let Some(conversation_id) = entry.identity.local_conversation_id {
            let nav_data = self
                .conversations
                .get(&conversation_id)
                .map(|metadata| &metadata.nav_data);
            if !entry.backing.has_cloud_data
                || entry.backing.has_local_persisted_data
                || entry.backing.has_loaded_conversation
                || nav_data.is_some()
            {
                return Some(WorkspaceAction::RestoreOrNavigateToConversation {
                    conversation_id,
                    window_id: nav_data.and_then(|nav_data| nav_data.window_id),
                    pane_view_locator: None,
                    terminal_view_id: nav_data.and_then(|nav_data| nav_data.terminal_view_id),
                    restore_layout,
                });
            }
        }

        entry
            .identity
            .server_conversation_token
            .as_ref()
            .map(|token| WorkspaceAction::OpenConversationTranscriptViewer {
                conversation_id: token.clone(),
                ambient_agent_task_id: entry.identity.ambient_agent_task_id,
            })
    }

    fn resolve_entry_copy_link(&self, entry: &AgentConversationEntry) -> Option<String> {
        if let Some(task_id) = entry.identity.ambient_agent_task_id
            && let Some(session_link) = self.tasks.get(&task_id).and_then(|task| {
                task.has_active_execution()
                    .then(|| {
                        task.active_run_execution()
                            .session_link
                            .map(ToString::to_string)
                    })
                    .flatten()
            })
        {
            return Some(session_link);
        }

        entry
            .identity
            .server_conversation_token
            .as_ref()
            .map(ServerConversationToken::conversation_link)
    }

    fn entry_for_server_token(
        &self,
        server_token: &ServerConversationToken,
        app: &AppContext,
    ) -> Option<AgentConversationEntry> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);
        if let Some(task) = self.tasks.values().find(|task| {
            task.conversation_id()
                .is_some_and(|conversation_id| conversation_id == server_token.as_str())
        }) {
            return Some(entry::entry_for_task(task, history_model, app));
        }

        let conversation_id = history_model.find_conversation_id_by_server_token(server_token)?;
        if let Some(task) = self.tasks.values().find(|task| {
            entry::conversation_id_shadowed_by_task(task, history_model) == Some(conversation_id)
        }) {
            return Some(entry::entry_for_task(task, history_model, app));
        }

        self.get_entry_by_id(
            &AgentConversationEntryId::Conversation(conversation_id),
            app,
        )
    }

    fn task_id_for_server_token(
        &self,
        server_token: &ServerConversationToken,
    ) -> Option<AmbientAgentTaskId> {
        self.tasks.values().find_map(|task| {
            task.conversation_id()
                .is_some_and(|conversation_id| conversation_id == server_token.as_str())
                .then_some(task.task_id)
        })
    }

    fn handle_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if !FeatureFlag::InteractiveConversationManagementView.is_enabled() {
            return;
        }
        match event {
            // Events that affect conversation navigation data - need full sync
            BlocklistAIHistoryEvent::StartedNewConversation { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::SplitConversation { .. }
            | BlocklistAIHistoryEvent::RestoredConversations { .. }
            | BlocklistAIHistoryEvent::RemoveConversation { .. }
            | BlocklistAIHistoryEvent::DeletedConversation { .. }
            | BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface { .. }
            | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
            => {
                self.sync_conversations(ctx);
            }

            // Status changes - just trigger re-render since status is looked up at render time
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                update, new_status, ..
            } => {
                let kind = match update {
                    ConversationStatusUpdate::Restored => ConversationUpdateKind::Restored,
                    ConversationStatusUpdate::Changed { prev_status } => {
                        ConversationUpdateKind::StatusSet {
                            prev_filter: AgentRunDisplayStatus::from_conversation_status(
                                prev_status,
                            )
                            .status_filter(),
                            new_filter: AgentRunDisplayStatus::from_conversation_status(new_status)
                                .status_filter(),
                        }
                    }
                };
                ctx.emit(AgentConversationsModelEvent::ConversationUpdated { kind });
            }

            // Artifact changes - sync live artifacts into the cached task and notify.
            BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                conversation_id, ..
            } => {
                let conversation = BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id);
                let Some(conversation) = conversation else {
                    return;
                };

                let task_id = conversation.task_id().or_else(|| {
                    conversation
                        .server_metadata()
                        .and_then(|metadata| metadata.ambient_agent_task_id)
                });
                if let Some(task_id) = task_id {
                    // If the conversation is associated with a task, update the saved task
                    // with live artifacts.
                    if let Some(task) = self.tasks.get_mut(&task_id) {
                        task.artifacts = conversation.artifacts().to_vec();
                        ctx.emit(AgentConversationsModelEvent::TasksUpdated);
                    }
                }
                ctx.emit(AgentConversationsModelEvent::ConversationArtifactsUpdated {
                    conversation_id: *conversation_id,
                });
            }
            BlocklistAIHistoryEvent::UpdatedConversationTitle {
                conversation_id,
                title,
                ..
            } => {
                let history_model = BlocklistAIHistoryModel::as_ref(ctx);
                for task in self.tasks.values_mut() {
                    if entry::conversation_id_shadowed_by_task(task, history_model)
                        == Some(*conversation_id)
                    {
                        task.title = title.clone();
                    }
                }

                ctx.emit(AgentConversationsModelEvent::ConversationUpdated {
                    kind: ConversationUpdateKind::TitleChanged,
                });
            }

            // Task/exchange-level changes that don't affect conversation navigation.
            BlocklistAIHistoryEvent::CreatedSubtask { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. }
            | BlocklistAIHistoryEvent::ReassignedExchange { .. }
            | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            // UpdatedStreamingExchange covers streaming and other exchange-level updates but
            // doesn't change any ConversationNavigationData fields (title comes from
            // UpdateTaskDescription, last_updated uses exchange.start_time which is set at append time).
            | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
            | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces { .. }
            | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
            | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
            | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. } => {}

            BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. } => {
                ctx.emit(AgentConversationsModelEvent::ConversationUpdated {
                    kind: ConversationUpdateKind::MetadataChanged,
                });
            }
        }
    }

    /// Get raw task data by task ID
    pub fn get_task_data(&self, task_id: &AmbientAgentTaskId) -> Option<AmbientAgentTask> {
        self.tasks.get(task_id).cloned()
    }

    /// 按 task ID 读取本地已缓存的 task 数据。
    ///
    /// Zap(本地优先):上游在这里会向云端 `GET /api/v1/agent/runs/{id}` 补取缺失的 task,
    /// 并维护 in-flight 去重与失败退避。Zap 没有云端 run API,调用方如果恢复了旧布局但
    /// 本地模型没有对应 task,这里返回 `None`,由现有面板降级路径处理。
    ///
    /// 因为不再有异步补取,签名精简为只读的单参形式。
    pub fn get_or_async_fetch_task_data(
        &self,
        task_id: &AmbientAgentTaskId,
    ) -> Option<AmbientAgentTask> {
        self.tasks.get(task_id).cloned()
    }

    /// Returns all (name, uid) pairs for creators of tasks in the model.
    ///
    /// We use this function to populate the available creator filter list
    /// based on the tasks we have.
    pub fn get_all_creators(&self, app: &AppContext) -> Vec<(String, String)> {
        let mut creators: Vec<(String, String)> = self
            .tasks
            .values()
            .filter_map(|task| {
                let name = entry::task_creator_name(task, app)?;
                let uid = entry::task_creator_uid(task)?;
                Some((name, uid))
            })
            .collect();

        // Include the current user since they may have local conversations
        let auth_state = AuthStateProvider::as_ref(app).get();
        if let (Some(name), Some(uid)) = (auth_state.display_name(), auth_state.user_id()) {
            creators.push((name, uid.to_string()));
        }

        creators.sort_by(|a, b| a.0.cmp(&b.0));
        creators.dedup_by(|a, b| a.0 == b.0);

        creators
    }

    pub fn mark_task_as_manually_opened(
        &mut self,
        task_id: AmbientAgentTaskId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.manually_opened_task_ids.insert(task_id) {
            ctx.emit(AgentConversationsModelEvent::TaskManuallyOpened);
        }
    }

    #[allow(dead_code)]
    pub fn is_task_manually_opened(&self, task_id: &AmbientAgentTaskId) -> bool {
        self.manually_opened_task_ids.contains(task_id)
    }
}

#[cfg(test)]
#[path = "agent_conversations_model_tests.rs"]
mod tests;
