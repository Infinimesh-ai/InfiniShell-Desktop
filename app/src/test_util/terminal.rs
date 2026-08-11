#[cfg(feature = "local_fs")]
use ai::skills::SKILL_PROVIDER_DEFINITIONS;
#[cfg(feature = "local_fs")]
use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use std::sync::Arc;
use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle, WindowId};
use watcher::HomeDirectoryWatcher;

use super::settings::initialize_history_persistence_for_tests;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::agent_providers::AgentProviderSecrets;
use crate::ai::agent_tips::AITipModel;
use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::ai::blocklist::agent_view::orchestration_pill_bar_model::OrchestrationPillBarModel;
use crate::ai::blocklist::orchestration_events::OrchestrationEventService;
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, BlocklistAIPermissions, QueuedQueryModel, SerializedBlockListItem,
};
use crate::ai::connected_self_hosted_workers::ConnectedSelfHostedWorkersModel;
use crate::ai::document::ai_document_model::AIDocumentModel;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::LLMPreferences;
use crate::ai::mcp::gallery::MCPGalleryManager;
use crate::ai::mcp::templatable_manager::TemplatableMCPServerManager;
use crate::ai::pricing_promotion::PricingPromotionState;
use crate::ai::restored_conversations::RestoredAgentConversations;
use crate::ai::skills::SkillManager;
use crate::ai::{AIRequestUsageModel, AgentTip};
use crate::auth::AuthManager;
use crate::auth::AuthStateProvider;
use crate::changelog_model::ChangelogModel;
use crate::cloud_object::model::persistence::ObjectStoreModel;
use crate::cloud_object::update_manager::UpdateManager;
use crate::code_review::git_repo_model::GitRepoModels;
use crate::context_chips::prompt::Prompt;
use crate::experiments;
use crate::network::NetworkStatus;
use crate::notifications::model::NotificationsModel;
use crate::pricing::PricingInfoModel;
use crate::search::files::model::FileSearchModel;
use crate::settings::PrivacySettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::suggestions::ignored_suggestions_model::IgnoredSuggestionsModel;
use crate::system::{SystemInfo, SystemStats};
use crate::terminal::alt_screen_reporting::AltScreenReporting;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::resizable_data::ResizableData;
use crate::terminal::view::inline_banner::ByoLlmAuthBannerSessionState;
use crate::terminal::{History, TerminalView};
use crate::undo_close::UndoCloseStack;
use crate::warp_managed_paths_watcher::WarpManagedPathsWatcher;
use crate::workflows::local_workflows::LocalWorkflows;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::{ActiveSession, OneTimeModalModel, WorkspaceRegistry};
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Initializes all of the necessary models to use a terminal view.
pub fn initialize_app_for_terminal_view(app: &mut App) {
    initialize_history_persistence_for_tests(app);

    app.add_singleton_model(|_| ChangelogModel::new(Arc::new(http_client::Client::new())));
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| SystemStats::new());
    app.add_singleton_model(|_| Prompt::mock());
    app.add_singleton_model(ObjectStoreModel::mock);
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(MCPGalleryManager::new);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(PrivacySettings::mock);
    app.add_singleton_model(|_ctx| SyncedInputState::mock());
    app.add_singleton_model(|_| ResizableData::default());
    app.add_singleton_model(LocalWorkflows::new);
    app.add_singleton_model(|_| History::default());
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    // QueuedQueryModel subscribes to history events; register after the
    // history model is in place.
    app.add_singleton_model(QueuedQueryModel::new);
    // Pill bar model subscribes to history events; register after the
    // history model is in place.
    app.add_singleton_model(|ctx| OrchestrationPillBarModel::new(Default::default(), ctx));
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());
    app.add_singleton_model(OrchestrationEventService::new);
    // Zap:`LocalAgentTaskSyncModel`(云端任务同步)与 `OrchestrationEventStreamer`
    // (依赖已删的 server_api SSE 网关)两个模块都未在 `ai::blocklist` 挂载,不再注册。
    app.add_singleton_model(|_| ActiveAgentViewsModel::new());
    app.add_singleton_model(BlocklistAIPermissions::new);
    // Zap:通知中心单例(上游 `AgentNotificationsModel` 的本地等价物),
    // 构造时会订阅 BlocklistAIHistoryModel / CLIAgentSessionsModel,必须排在它们之后。
    app.add_singleton_model(NotificationsModel::new);
    app.add_singleton_model(UndoCloseStack::new);

    app.add_singleton_model(AIRequestUsageModel::new_for_test);
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
    app.add_singleton_model(AgentProviderSecrets::new);
    // `CloudSyncTokenStore` 已由 `initialize_settings_for_tests` 统一注册,这里不再重复。
    app.add_singleton_model(LLMPreferences::new);
    app.add_singleton_model(HarnessAvailabilityModel::new);
    app.add_singleton_model(|ctx| AITipModel::<AgentTip>::new_for_agent_tips(ctx));
    app.add_singleton_model(ConnectedSelfHostedWorkersModel::new);
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(|_| DetectedRepositories::default());
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(|ctx| {
        let model = RepoMetadataModel::new(ctx);
        model.register_force_included_paths(
            SKILL_PROVIDER_DEFINITIONS
                .iter()
                .map(|provider| provider.skills_path.clone()),
            ctx,
        );
        model.set_project_skill_provider_paths(
            SKILL_PROVIDER_DEFINITIONS
                .iter()
                .map(|provider| provider.skills_path.clone()),
            ctx,
        );
        model
    });
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(|_| GitRepoModels::new());
    // Zap:RepoOutlines 已删除,不再注册。
    app.add_singleton_model(HomeDirectoryWatcher::new_for_test);
    app.add_singleton_model(WarpManagedPathsWatcher::new_for_testing);
    app.add_singleton_model(SkillManager::new);

    app.add_singleton_model(|_| TemplatableMCPServerManager::default());
    app.add_singleton_model(|ctx| {
        AIExecutionProfilesModel::new(&crate::LaunchMode::new_for_unit_test(), ctx)
    });
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);

    #[cfg(not(target_family = "wasm"))]
    app.add_singleton_model(SystemInfo::new);

    app.add_singleton_model(|_| RestoredAgentConversations::new_seeded(vec![]));
    app.add_singleton_model(OneTimeModalModel::new);
    app.add_singleton_model(|_| WorkspaceRegistry::new());
    app.add_singleton_model(|_| IgnoredSuggestionsModel::new(vec![]));
    app.add_singleton_model(|_| PricingInfoModel::new());
    app.add_singleton_model(PricingPromotionState::new);
    app.add_singleton_model(AIDocumentModel::new);
    app.add_singleton_model(ByoLlmAuthBannerSessionState::new);
    app.add_singleton_model(|_| GitHubAuthNotifier::new());
    app.add_singleton_model(AgentConversationsModel::new);

    app.update(experiments::init);
    AltScreenReporting::register(app);
    // shared-session viewer 会读取远端服务器管理器(见 lib.rs 的同名注册)。
    app.add_singleton_model(crate::remote_server::manager::RemoteServerManager::new);
    // shared-session viewer 相关的 view 在构造期读取 `AvailableShells`
    // (生产路径见 lib.rs 的 `terminal::available_shells::register`)。
    #[cfg(feature = "local_tty")]
    crate::terminal::available_shells::register(app);
}

/// Creates a window in `app` with a [`TerminalView`] as the root view.
/// Returns the handle to that terminal view.
pub fn add_window_with_terminal(
    app: &mut App,
    restored_blocks: Option<&[SerializedBlockListItem]>,
) -> ViewHandle<TerminalView> {
    add_window_with_id_and_terminal(app, restored_blocks).1
}

/// Creates a window in `app` with a [`TerminalView`] as the root view.
/// Returns the WindowID and the handle to that terminal view.
pub fn add_window_with_id_and_terminal(
    app: &mut App,
    restored_blocks: Option<&[SerializedBlockListItem]>,
) -> (WindowId, ViewHandle<TerminalView>) {
    let tips_model = app.add_model(|_| Default::default());
    app.add_window(WindowStyle::NotStealFocus, |ctx| {
        TerminalView::new_for_test(tips_model, restored_blocks, ctx)
    })
}
