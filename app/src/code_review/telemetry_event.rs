//! Zap:遥测上报层已删除。这里仅保留 code review 的事件类型壳与辅助枚举,
//! 供既有调用点继续类型检查;`send_telemetry_from_ctx!` 在 Zap 里是无副作用的编译期 shim。
//! 已删除 `impl TelemetryEvent` / `impl TelemetryEventDesc` / `register_telemetry_event!`
//! 以及 strum 的 `EnumDiscriminants` + `EnumIter` 派生。
#![allow(dead_code)]

use std::fmt::Display;
use std::time::Duration;

use serde::Serialize;
use serde_with::SerializeDisplay;

use crate::code_review::diff_state::{BackendOrigin, DiffMode, DiffOperation};
use crate::server::telemetry::CLIAgentType;
use crate::view_components::find::FindDirection;

/// Identifies which git button the user clicked in the code review header.
/// Each variant maps to one of the primary action button / dropdown items.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum GitButtonKind {
    #[serde(rename = "commit")]
    Commit,
    #[serde(rename = "push")]
    Push,
    #[serde(rename = "publish")]
    Publish,
    #[serde(rename = "create_pr")]
    CreatePr,
    #[serde(rename = "view_pr")]
    ViewPr,
}

/// Identifies which git operation actually ran when a `GitDialog` completed.
/// Distinguishes commit-dialog chained intents (e.g. commit-and-push) from
/// standalone push/publish/create-PR dialogs so analytics can tell the user
/// flows apart.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum GitOperationKind {
    /// Commit dialog with the commit-only intent.
    #[serde(rename = "commit_only")]
    CommitOnly,
    /// Commit dialog with the commit-and-push intent.
    #[serde(rename = "commit_and_push")]
    CommitAndPush,
    /// Commit dialog with the commit-and-create-PR intent.
    #[serde(rename = "commit_and_create_pr")]
    CommitAndCreatePr,
    /// Standalone push dialog.
    #[serde(rename = "push")]
    Push,
    /// Standalone publish dialog (push that also sets upstream).
    #[serde(rename = "publish")]
    Publish,
    /// Standalone create-PR dialog.
    #[serde(rename = "create_pr")]
    CreatePr,
}

/// Terminal status of a `GitDialog`. Captures both async-op outcomes and
/// pre-confirmation user cancels in a single enum.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum GitDialogStatus {
    /// User confirmed the dialog and the underlying git operation succeeded.
    #[serde(rename = "succeeded")]
    Succeeded,
    /// User confirmed the dialog and the underlying git operation failed.
    #[serde(rename = "failed")]
    Failed,
    /// User cancelled the dialog (ESC / close button / cancel button) before
    /// the async op ran.
    #[serde(rename = "cancelled")]
    Cancelled,
}

/// Entry points for opening the code review pane.
#[derive(Clone, Copy, Debug, SerializeDisplay, Default)]
pub enum CodeReviewPaneEntrypoint {
    /// Opened via the git diff chip (git changes button in AI control panel).
    GitDiffChip,
    /// Opened via the "View changes" button when Agent mode is done running.
    AgentModeCompleted,
    /// Opened via the "Review changes" button when Agent mode is running.
    AgentModeRunning,
    /// Opened via the "/code-review" slash command.
    SlashCommand,
    /// Opened by the agent tool call.
    InvokedByAgent,
    // Force opened when user accepted first diff of a conversation
    ForceOpened,
    // Opened via the agent mode diff header
    CodeDiffHeader,
    // Opened via the pane header
    PaneHeader,
    // Opened via the code mode v2 right panel button
    RightPanel,
    /// Opened via the CLI agent view footer (e.g., Claude Code).
    CLIAgentView,
    /// Opened via other means (unknown entry point).
    #[default]
    Other,
}

impl Display for CodeReviewPaneEntrypoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitDiffChip => write!(f, "git_diff_chip"),
            Self::AgentModeCompleted => write!(f, "agent_mode_completed"),
            Self::AgentModeRunning => write!(f, "agent_mode_running"),
            Self::SlashCommand => write!(f, "slash_command"),
            Self::InvokedByAgent => write!(f, "invoked_by_agent"),
            Self::ForceOpened => write!(f, "force_opened"),
            Self::CodeDiffHeader => write!(f, "agent_mode_diff_header"),
            Self::PaneHeader => write!(f, "pane_header"),
            Self::RightPanel => write!(f, "right_panel"),
            Self::CLIAgentView => write!(f, "cli_agent_view"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// Origin of an "Add to context" action.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum AddToContextOrigin {
    /// User selected text and added it to context.
    #[serde(rename = "selected_text")]
    SelectedText,
    /// User clicked the gutter to add a line/hunk to context.
    #[serde(rename = "gutter")]
    Gutter,
    /// User clicked the "Add diff set as context" button in code review header.
    #[serde(rename = "code_review_header")]
    #[allow(unused)]
    CodeReviewHeader,
}

/// Where code review content was sent after the user action.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum CodeReviewContextDestination {
    /// Written directly to the terminal PTY for an active CLI agent.
    #[serde(rename = "pty")]
    Pty,
    /// Inserted into the InfiniShell AI input buffer as plain text.
    #[serde(rename = "agent_input")]
    AgentInput,
    /// Registered as an AI attachment and referenced from the input.
    #[serde(rename = "agent_attachment")]
    AgentAttachment,
    /// Inserted into the active command buffer while a command is running.
    #[serde(rename = "active_command_buffer")]
    ActiveCommandBuffer,
    /// Submitted as an inline code review request through the InfiniShell AI path.
    #[serde(rename = "agent_review")]
    AgentReview,
    /// Inserted into CLI agent rich input.
    #[serde(rename = "rich_input")]
    RichInput,
}

/// Scope of a diff set attachment initiated from code review.
#[derive(Clone, Copy, Debug, Serialize)]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum DiffSetContextScope {
    /// Attach the full diff set for the current review.
    #[serde(rename = "all")]
    All,
    /// Attach the diff set for a single file.
    #[serde(rename = "file")]
    File,
}

/// Pane state change for minimize/maximize events.
#[derive(Clone, Copy, Debug, Serialize)]
pub enum PaneStateChange {
    /// Pane was minimized.
    #[serde(rename = "minimized")]
    Minimized,
    /// Pane was maximized.
    #[serde(rename = "maximized")]
    Maximized,
}

/// Telemetry events associated with the code review pane.
#[derive(Serialize, Debug)]
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
pub enum CodeReviewTelemetryEvent {
    /// Emitted when the code review pane is opened.
    PaneOpened {
        is_local: Option<bool>,
        entrypoint: CodeReviewPaneEntrypoint,
        is_code_mode_v2: bool,
        /// The CLI agent type if opened from a CLI agent footer (e.g., Claude Code).
        cli_agent: Option<CLIAgentType>,
    },
    /// Emitted when a user adds content to AI context from code review.
    AddToContext {
        is_local: Option<bool>,
        origin: AddToContextOrigin,
        destination: CodeReviewContextDestination,
        diff_set_scope: Option<DiffSetContextScope>,
    },
    /// Emitted when a user clicks the revert hunk button.
    RevertHunkClicked { is_local: Option<bool> },
    /// Emitted when a file is saved in the code review pane.
    FileSaved { is_local: Option<bool> },
    /// Emitted when the code review pane is minimized or maximized.
    PaneStateChanged {
        is_local: Option<bool>,
        state_change: PaneStateChange,
    },
    /// Emitted when the diff base is changed (e.g., from uncommitted to main branch).
    BaseChanged {
        is_local: Option<bool>,
        /// The new diff mode.
        mode: DiffMode,
    },
    /// Failure when we are calculating the diff metadata.
    LoadMetadataFailed {
        backend_origin: BackendOrigin,
        mode: DiffMode,
        error: String,
    },
    /// Failure when we are loading the actual diff content. Shared across
    /// file-invalidation, full diff load, and remote diff paths; the
    /// `operation` field distinguishes which one produced the failure.
    LoadDiffFailed {
        backend_origin: BackendOrigin,
        operation: DiffOperation,
        mode: DiffMode,
        error: String,
        /// Time elapsed between when the tracked diff load was requested and
        /// when this failure was observed. `None` if no tracked load was in
        /// flight (e.g. a background invalidation error).
        load_duration: Option<Duration>,
    },
    /// Emitted when a full diff load completes successfully.
    DiffLoadCompleted {
        is_local: Option<bool>,
        mode: DiffMode,
        file_count: usize,
        files_changed: usize,
        total_additions: usize,
        total_deletions: usize,
        /// Time elapsed between when the diff load was requested and when the
        /// diffs were ready. `None` if the load was not initiated through a
        /// tracked entry point (e.g. background refresh).
        load_duration: Option<Duration>,
    },
    /// Emitted when the code review find bar is opened or closed.
    FindBarToggled {
        is_local: Option<bool>,
        /// Whether the find bar is now open.
        is_open: bool,
    },
    /// Emitted when search mode settings are changed.
    FindBarModeChanged {
        is_local: Option<bool>,
        /// Whether case-sensitive search is enabled.
        case_sensitive: bool,
        /// Whether regex search is enabled.
        regex: bool,
    },
    /// Emitted when the user navigates to the next or previous match.
    FindNavigated {
        is_local: Option<bool>,
        /// Direction of navigation.
        direction: FindDirection,
    },
    /// Emitted when the inline comment editor is opened in the code review pane.
    CommentEditorOpened { is_local: Option<bool> },
    /// Emitted when a new comment is added to the inline review.
    CommentAdded { is_local: Option<bool> },
    /// Emitted when an existing comment is edited.
    CommentEdited { is_local: Option<bool> },
    /// Emitted when a comment is deleted from the inline review.
    CommentDeleted {
        is_local: Option<bool>,
        is_imported: bool,
    },
    /// Emitted when the bottom comment list panel is expanded.
    CommentListExpanded {
        is_local: Option<bool>,
        /// Number of comments currently in the list.
        comment_count: usize,
    },
    /// Emitted when the user submits an inline review to the agent.
    ReviewSubmitted {
        is_local: Option<bool>,
        /// Number of comments in the submitted review.
        comment_count: usize,
        /// Number of unique files with comments.
        file_count: usize,
        /// Where the review was submitted.
        destination: CodeReviewContextDestination,
    },
    /// Emitted when a comment in the list view is clicked to jump to its location.
    CommentListItemClicked { is_local: Option<bool> },
    /// Emitted when one or more comments fail to be precisely relocated after code changes.
    CommentRelocationFailed {
        is_local: Option<bool>,
        /// Number of comments that could not be matched to an exact line and had to fall back.
        fallback_count: usize,
    },
    /// Emitted when one or more comments are resolved.
    CommentResolved {
        /// Number of comments resolved by this operation.
        resolved_count: usize,
    },
    /// Emitted when the agent's insert_code_review_comments tool call is received and processed.
    CommentsReceived {
        is_local: Option<bool>,
        /// Number of raw InsertReviewComment items from the tool call.
        raw_count: usize,
        /// Number of successfully converted PendingImportedReviewComments.
        converted_count: usize,
        /// Number of AttachedReviewComments after thread flattening.
        thread_count: usize,
    },
    /// Emitted after newly-imported comments are relocated against editor lines.
    CommentsAttached {
        is_local: Option<bool>,
        /// Number of non-outdated imported comments after relocation.
        active_count: usize,
        /// Number of outdated imported comments after relocation.
        outdated_count: usize,
    },
    /// Emitted when a user clicks a git operation button in the code review
    /// header (primary button or dropdown item).
    GitButtonTriggered {
        is_local: Option<bool>,
        button: GitButtonKind,
    },
    /// Emitted when a git dialog reaches a terminal state — either the async
    /// op succeeded / failed, or the user cancelled before confirming.
    GitDialogCompleted {
        is_local: Option<bool>,
        /// The git operation that ran or would have run (e.g. `commit_and_push`
        /// for the commit dialog with that chained intent).
        operation: GitOperationKind,
        /// Whether the dialog succeeded, failed, or was cancelled.
        status: GitDialogStatus,
        /// Raw error string when `status == Failed`, `None` otherwise.
        error: Option<String>,
    },
}
