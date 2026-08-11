mod auth_secret_ftux_dropdown;
mod auth_secret_ftux_view;
pub(crate) mod auth_secret_selector;
mod block;
mod delete_auth_secret_confirmation_dialog;
// Zap Wave 7-2:`first_time_setup` 随 ambient agent UI 物理删。
mod footer;
mod harness_selector;
mod host_selector;
mod loading_screen;
mod model;
mod model_selector;
mod progress;
mod progress_ui_state;
mod tips;
mod view_impl;

pub use auth_secret_ftux_view::{AuthSecretFtuxAction, AuthSecretFtuxView, AuthSecretFtuxViewEvent};
pub use auth_secret_selector::{
    AuthSecretSelector, AuthSecretSelectorAction, AuthSecretSelectorEvent,
};
pub use block::*;
pub use footer::{render_error_footer, render_loading_footer};
pub use harness_selector::{HarnessSelector, HarnessSelectorAction, HarnessSelectorEvent};
pub use host_selector::{
    Host, HostSelector, HostSelectorAction, HostSelectorEvent, NakedHeaderButtonTheme,
};
pub use loading_screen::{render_ambient_agent_error_screen, render_ambient_agent_loading_screen};
pub use model::{AgentProgress, AmbientAgentViewModel, AmbientAgentViewModelEvent, Status};
pub use model_selector::{ModelSelector, ModelSelectorAction, ModelSelectorEvent};
pub use progress::{render_progress, ProgressProps, ProgressStep, ProgressStepState};
pub use progress_ui_state::AmbientAgentProgressUIState;
pub use tips::{get_ambient_agent_tips, AmbientAgentTip};
use warp_core::features::FeatureFlag;
use warpui::{AppContext, ModelHandle};

use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewState};
use crate::terminal::TerminalModel;

/// Returns `true` when an ambient agent session is in any pre-first-exchange phase —
/// either still spawning (loading screen) or running setup commands before the first
/// agent turn. In this state, we hide the interactive input and render a loading footer.
///
/// Zap 说明:上游版本额外由 `FeatureFlag::CloudMode` 把关,并把 local→cloud 交接面板
/// (`AmbientAgentViewModel::is_local_to_cloud_handoff`)当作「这是云端 agent 面板」的
/// 权威信号。Zap 没有云端 agent,这两者都不存在,因此:
/// - 门控只保留 `FeatureFlag::AgentView`;
/// - 来源判定改用本地 ambient agent 的 `origin.is_ambient_agent()`,再叠加共享 ambient
///   会话 `TerminalModel::is_shared_ambient_agent_session()`(裸链接加入 / attach 到运行中
///   会话时,agent view 的 entry origin 是 `SharedSessionSelection`,不是 `AmbientAgent`)。
///
/// 函数名沿用上游的 `is_cloud_agent_pre_first_exchange` 以保持与全部调用点一致。
pub fn is_cloud_agent_pre_first_exchange(
    ambient_agent_view_model: Option<&ModelHandle<AmbientAgentViewModel>>,
    agent_view_controller: &ModelHandle<AgentViewController>,
    terminal_model: &TerminalModel,
    app: &AppContext,
) -> bool {
    if !FeatureFlag::AgentView.is_enabled() {
        return false;
    }

    let Some(ambient_agent_view_model) = ambient_agent_view_model else {
        return false;
    };

    let view_model = ambient_agent_view_model.as_ref(app);

    let is_in_pre_first_exchange_status = matches!(
        view_model.status(),
        Status::WaitingForSession { .. } | Status::AgentRunning
    );
    if !is_in_pre_first_exchange_status {
        return false;
    }

    let agent_view_state = agent_view_controller.as_ref(app).agent_view_state().clone();
    let AgentViewState::Active { origin, .. } = agent_view_state else {
        return false;
    };

    // Shared-session viewers of an ambient run (raw link join / attach-to-running) enter agent
    // view via `SharedSessionSelection`, so `is_shared_ambient_agent_session()` is the
    // authoritative signal for that path — e.g. a post-death follow-up spinning up a new session
    // must still count as pre-first-exchange so the setup progress + prompt-queuing UI render.
    if !origin.is_ambient_agent() && !terminal_model.is_shared_ambient_agent_session() {
        return false;
    }

    // For non-oz harness runs, there is no Oz `AppendedExchange` to key off of, so we also
    // exit the pre-first-exchange phase when the harness CLI (e.g. `claude`, `gemini`) has
    // been detected. See `mark_harness_command_started`.
    if view_model.harness_command_started() {
        return false;
    }

    // Loading phase (`WaitingForSession`): no setup commands have started yet, but we're
    // still pre-first-exchange. Skip the block-list flag check.
    if matches!(view_model.status(), Status::WaitingForSession { .. }) {
        return true;
    }

    terminal_model
        .block_list()
        .is_executing_oz_environment_startup_commands()
}
