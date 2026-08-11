use settings::schema::SettingSchemaEntry;
use settings::{Setting, SettingSurfaces, SettingsMode};

use super::{IsCrashReportingEnabled, IsTelemetryEnabled};

// Zap:上游还在这里断言 `IsCloudConversationStorageEnabled`,该设置随云端会话存储
// 一起剥离,`WarpDrivePrivacySettings` 里已无此项,故从待检列表中移除。
#[test]
fn privacy_settings_apply_to_gui_and_tui() {
    for storage_key in [
        IsTelemetryEnabled::toml_key(),
        IsCrashReportingEnabled::toml_key(),
    ] {
        let entry = inventory::iter::<SettingSchemaEntry>
            .into_iter()
            .find(|entry| entry.storage_key == storage_key)
            .unwrap_or_else(|| panic!("missing schema entry for {storage_key}"));
        let surfaces = (entry.surfaces_fn)();

        assert_eq!(surfaces, SettingSurfaces::ALL, "{storage_key}");
        assert!(surfaces.includes(SettingsMode::Gui), "{storage_key}");
        assert!(surfaces.includes(SettingsMode::Tui), "{storage_key}");
    }
}
