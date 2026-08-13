use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};

define_settings_group!(AutoupdateSettings, settings: [
    automatic_updates_enabled: AutomaticUpdatesEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        storage_key: "AutomaticUpdatesEnabled",
        toml_path: "updates.automatic_updates_enabled",
        description: "Whether InfiniShell automatically checks for and downloads updates in the background.",
    },
]);
