#[cfg(test)]
use warpui::App;

#[cfg(test)]
pub fn initialize_settings_for_tests(app: &mut App) {
    use warp_core::execution_mode::ExecutionMode;
    initialize_settings_for_tests_with_mode(app, ExecutionMode::App, false);
}

#[cfg(test)]
pub fn initialize_history_persistence_for_tests(app: &mut App) {
    use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

    initialize_settings_for_tests(app);

    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
}

#[cfg(test)]
pub fn initialize_settings_for_tests_with_mode(
    app: &mut App,
    mode: warp_core::execution_mode::ExecutionMode,
    is_sandboxed: bool,
) {
    use warp_core::execution_mode::AppExecutionMode;
    use warp_core::semantic_selection::SemanticSelection;

    use crate::drive::settings::WarpDriveSettings;
    use crate::search::command_search::settings::CommandSearchSettings;
    use crate::settings::app_icon::AppIconSettings;
    use crate::settings::app_installation_detection::UserAppInstallDetectionSettings;
    // Zap i18n:`LanguageSettings` 是我方新增的设置组,生产路径在
    // `settings::init::register_all_settings` 里注册;测试 setup 需同样注册,
    // 否则读取语言设置的代码会 panic。
    use crate::settings::language::LanguageSettings;
    use crate::settings::manager::SettingsManager;
    use crate::settings::network::NetworkSettings;
    use crate::settings::{
        AISettings, AccessibilitySettings, AliasExpansionSettings, AppEditorSettings,
        AutoupdateSettings, BlockVisibilitySettings, CloudSyncSettings, CodeSettings,
        DebugSettings, EmacsBindingsSettings, FontSettings, GPUSettings, InputModeSettings,
        InputSettings, LocalControlSettings, NativePreferenceSettings, PaneSettings,
        PreferencesSettings, SameLinePromptBlockSettings, ScrollSettings, SelectionSettings,
        SharedObjectLimitBannerSettings, SshSettings, ThemeSettings, TuiAutoupdateSettings,
        TuiThemeSettings, TuiVoiceSettings, TuiZeroStateSettings, VimBannerSettings,
        WarpDrivePrivacySettings, init_and_register_user_preferences,
    };
    use crate::terminal::BlockListSettings;
    use crate::terminal::general_settings::GeneralSettings;
    use crate::terminal::keys_settings::KeysSettings;
    use crate::terminal::ligature_settings::LigatureSettings;
    use crate::terminal::safe_mode_settings::SafeModeSettings;
    use crate::terminal::session_settings::SessionSettings;
    use crate::terminal::settings::TerminalSettings;
    use crate::terminal::shared_session::settings::SharedSessionSettings;
    use crate::terminal::warpify::settings::WarpifySettings;
    use crate::undo_close::UndoCloseSettings;
    use crate::user_config::WarpConfig;
    use crate::window_settings::WindowSettings;
    use crate::workflows::aliases::WorkflowAliases;
    use crate::workspace::tab_settings::TabSettings;
    app.add_singleton_model(|ctx| AppExecutionMode::new(mode, is_sandboxed, ctx));

    app.update(init_and_register_user_preferences);
    app.add_singleton_model(|_ctx| SettingsManager::default());
    app.add_singleton_model(WarpConfig::mock);
    app.update(|ctx| {
        // Register a no-op secure storage provider for testing.
        warpui_extras::secure_storage::register_noop("test", ctx);
    });

    AccessibilitySettings::register(app);
    app.update(AISettings::register_and_subscribe_to_events);
    AliasExpansionSettings::register(app);
    // Zap Wave 7-3:`AmbientAgentSettings` 随 ambient-agent UI 子系统物理删。
    AppEditorSettings::register(app);
    BlockVisibilitySettings::register(app);
    BlockListSettings::register(app);
    PreferencesSettings::register(app);
    CommandSearchSettings::register(app);
    DebugSettings::register(app);
    AppIconSettings::register(app);
    EmacsBindingsSettings::register(app);

    #[cfg(feature = "local_fs")]
    {
        crate::util::file::external_editor::EditorSettings::register(app);
    }

    FontSettings::register(app);
    GeneralSettings::register(app);
    GPUSettings::register(app);
    InputModeSettings::register(app);
    InputSettings::register(app);
    KeysSettings::register(app);
    LanguageSettings::register(app);
    LigatureSettings::register(app);
    if warp_core::features::FeatureFlag::WarpControlCli.is_enabled() {
        LocalControlSettings::register(app);
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use crate::settings::LinuxAppConfiguration;
        LinuxAppConfiguration::register(app);
    }

    NativePreferenceSettings::register(app);
    // 以下几组与 `settings::init::register_all_settings` 对齐:合并后 settings_view
    // 的各 page view 在构造期就会读取它们,测试 setup 缺注册会直接 panic。
    NetworkSettings::register(app);
    AutoupdateSettings::register(app);
    TuiAutoupdateSettings::register(app);
    TuiThemeSettings::register(app);
    TuiZeroStateSettings::register(app);
    WarpDrivePrivacySettings::register(app);
    UserAppInstallDetectionSettings::register(app);
    CloudSyncSettings::register(app);
    WorkflowAliases::register(app);
    SafeModeSettings::register(app);
    SameLinePromptBlockSettings::register(app);
    ScrollSettings::register(app);
    SelectionSettings::register(app);
    app.update(|ctx| {
        WarpifySettings::register(ctx);
    });
    SessionSettings::register(app);
    SshSettings::register(app);
    TabSettings::register(app);
    TerminalSettings::register(app);
    PaneSettings::register(app);
    ThemeSettings::register(app);
    TuiVoiceSettings::register(app);
    UndoCloseSettings::register(app);
    VimBannerSettings::register(app);
    SharedObjectLimitBannerSettings::register(app);
    WarpDriveSettings::register(app);
    WindowSettings::register(app);
    SharedSessionSettings::register(app);
    CodeSettings::register(app);
    SemanticSelection::register(app);

    app.update(|ctx| {
        // Add settings models that are backed by secure storage, not user preferences.
        ctx.add_singleton_model(ai::api_keys::ApiKeyManager::new);
        // 代理密码同样走 secure storage(上面已注册 no-op provider)。
        ctx.add_singleton_model(crate::settings::network_secrets::ProxyCredentials::new);
        // 云同步 token 走 OS 密钥库,同样需要在测试里注册。
        ctx.add_singleton_model(crate::settings::CloudSyncTokenStore::new);
    });
}
