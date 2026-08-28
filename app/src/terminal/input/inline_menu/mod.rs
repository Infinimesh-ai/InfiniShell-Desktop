//! Generic inline menu view for rendering search results with selection and navigation.
mod message_bar;
mod message_provider;
mod model;
pub(crate) mod positioning;
pub mod styles;
mod view;

pub use message_bar::{InlineMenuMessageArgs, InlineMenuMessageBarArgs};
pub use message_provider::{InlineMenuMessageProvider, default_navigation_message_items};
pub use model::{InlineMenuModel, InlineMenuModelEvent, InlineMenuTabConfig};
pub use positioning::InlineMenuPositioner;
use serde::{Deserialize, Serialize};
pub use view::{
    DetailsRenderConfig, InlineMenuAction, InlineMenuClickBehavior, InlineMenuEvent,
    InlineMenuHeaderConfig, InlineMenuRowAction, InlineMenuView, QueryResultRendererExt,
};

use super::{InputSuggestionsMode, UserQueryMenuAction};

/// Identifies a specific inline menu type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Identifies a specific inline menu.",
    rename_all = "snake_case"
)]
pub enum InlineMenuType {
    SlashCommands,
    ModelSelector,
    ConversationMenu,
    ProfileSelector,
    PromptsMenu,
    SkillMenu,
    UserQueryMenu,
    RewindMenu,
    InlineHistoryMenu,
    IndexedReposMenu,
    PlanMenu,
}

impl InlineMenuType {
    fn display_label(&self) -> String {
        match self {
            InlineMenuType::SlashCommands => crate::t!("terminal-menu-commands"),
            InlineMenuType::ModelSelector => crate::t!("terminal-menu-model"),
            InlineMenuType::ConversationMenu => crate::t!("terminal-menu-conversations"),
            InlineMenuType::ProfileSelector => crate::t!("terminal-menu-profiles"),
            InlineMenuType::PromptsMenu => crate::t!("terminal-menu-prompts"),
            InlineMenuType::SkillMenu => crate::t!("terminal-menu-skills"),
            InlineMenuType::UserQueryMenu => crate::t!("terminal-menu-fork"),
            InlineMenuType::RewindMenu => crate::t!("terminal-menu-rewind"),
            InlineMenuType::InlineHistoryMenu => crate::t!("terminal-menu-history"),
            InlineMenuType::IndexedReposMenu => crate::t!("terminal-menu-repositories"),
            InlineMenuType::PlanMenu => crate::t!("terminal-menu-plans"),
        }
    }

    pub(crate) fn from_suggestions_mode(mode: &InputSuggestionsMode) -> Option<Self> {
        match mode {
            InputSuggestionsMode::SlashCommands => Some(InlineMenuType::SlashCommands),
            InputSuggestionsMode::ModelSelector => Some(InlineMenuType::ModelSelector),
            InputSuggestionsMode::ConversationMenu => Some(InlineMenuType::ConversationMenu),
            InputSuggestionsMode::ProfileSelector => Some(InlineMenuType::ProfileSelector),
            InputSuggestionsMode::PromptsMenu => Some(InlineMenuType::PromptsMenu),
            InputSuggestionsMode::SkillMenu => Some(InlineMenuType::SkillMenu),
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::ForkFrom,
                ..
            } => Some(InlineMenuType::UserQueryMenu),
            InputSuggestionsMode::UserQueryMenu {
                action: UserQueryMenuAction::Rewind,
                ..
            } => Some(InlineMenuType::RewindMenu),
            InputSuggestionsMode::InlineHistoryMenu { .. } => {
                Some(InlineMenuType::InlineHistoryMenu)
            }
            InputSuggestionsMode::IndexedReposMenu => Some(InlineMenuType::IndexedReposMenu),
            InputSuggestionsMode::PlanMenu { .. } => Some(InlineMenuType::PlanMenu),
            InputSuggestionsMode::Closed
            | InputSuggestionsMode::HistoryUp { .. }
            | InputSuggestionsMode::CompletionSuggestions { .. }
            | InputSuggestionsMode::StaticWorkflowEnumSuggestions { .. }
            | InputSuggestionsMode::DynamicWorkflowEnumSuggestions { .. }
            | InputSuggestionsMode::AIContextMenu { .. } => None,
        }
    }
}
