use tempfile::{Builder, TempDir};
use windows_registry::CURRENT_USER;

use super::{KEY_NOT_FOUND_ERR, RegistryBackedPreferences};
use crate::user_preferences::UserPreferences;

const OWNER_KEY: &str = "RegistryPreferencesTestOwner";

struct TestRegistry {
    directory: TempDir,
    key_paths: Vec<String>,
}

impl TestRegistry {
    fn new() -> Self {
        let mut registry = Self {
            directory: Builder::new()
                .prefix("infinishell-registry-test-")
                .rand_bytes(16)
                .tempdir()
                .expect("创建独占测试名称"),
            key_paths: Vec::new(),
        };
        // 只认领随机应用名下的三个自有键，不读取或创建日常应用键。
        for suffix in ["", "-acceptance", "-g09"] {
            let app_name = registry.app_name();
            let path = format!("Software\\InfiniShell\\{app_name}{suffix}");
            let error = CURRENT_USER.open(&path).expect_err("测试键必须尚不存在");
            assert_eq!(error.code(), KEY_NOT_FOUND_ERR);
            CURRENT_USER
                .create(&path)
                .expect("创建自有测试键")
                .set_string(OWNER_KEY, app_name)
                .expect("记录测试键所有权");
            registry.key_paths.push(path);
        }
        registry
    }

    fn app_name(&self) -> &str {
        self.directory.path().file_name().unwrap().to_str().unwrap()
    }
}

impl Drop for TestRegistry {
    fn drop(&mut self) {
        for path in &self.key_paths {
            // 仅清理仍有本次所有权标记的精确路径，绝不删除公共父键。
            let owner = CURRENT_USER
                .open(path)
                .and_then(|key| key.get_string(OWNER_KEY));
            if owner.ok().as_deref() == Some(self.app_name()) {
                let result = CURRENT_USER.remove_tree(path);
                if !std::thread::panicking() {
                    result.expect("删除自有测试键");
                }
            } else if !std::thread::panicking() {
                panic!("测试键所有权不符，保留该键: {path}");
            }
        }
    }
}

#[test]
fn unprofiled_preferences_keep_legacy_values() {
    let registry = TestRegistry::new();
    let app_name = registry.app_name();
    let legacy_path = format!("Software\\InfiniShell\\{app_name}");
    let legacy_key = CURRENT_USER.create(&legacy_path).unwrap();
    legacy_key
        .set_string("PrivatePreference", "legacy")
        .unwrap();

    let preferences = RegistryBackedPreferences::new(app_name, None);
    assert_eq!(
        preferences
            .read_value("PrivatePreference")
            .unwrap()
            .as_deref(),
        Some("legacy")
    );
    preferences
        .write_value("PrivatePreference", "updated".to_owned())
        .unwrap();
    drop(preferences);

    assert_eq!(
        legacy_key.get_string("PrivatePreference").unwrap(),
        "updated"
    );
}

#[test]
fn profile_preferences_do_not_share_values() {
    let registry = TestRegistry::new();
    let default = RegistryBackedPreferences::new(registry.app_name(), None);
    let g09 = RegistryBackedPreferences::new(registry.app_name(), Some("g09"));
    default
        .write_value("PrivatePreference", "default".to_owned())
        .unwrap();
    g09.write_value("PrivatePreference", "g09".to_owned())
        .unwrap();

    let acceptance = RegistryBackedPreferences::new(registry.app_name(), Some("acceptance"));
    assert_eq!(acceptance.read_value("PrivatePreference").unwrap(), None);
    acceptance
        .write_value("PrivatePreference", "acceptance".to_owned())
        .unwrap();
    drop(acceptance);
    let reopened = RegistryBackedPreferences::new(registry.app_name(), Some("acceptance"));
    assert_eq!(
        reopened.read_value("PrivatePreference").unwrap().as_deref(),
        Some("acceptance")
    );
    reopened.remove_value("PrivatePreference").unwrap();

    assert_eq!(reopened.read_value("PrivatePreference").unwrap(), None);
    assert_eq!(
        default.read_value("PrivatePreference").unwrap().as_deref(),
        Some("default")
    );
    assert_eq!(
        g09.read_value("PrivatePreference").unwrap().as_deref(),
        Some("g09")
    );
}

#[test]
fn migration_markers_are_profile_local() {
    let registry = TestRegistry::new();
    let default = RegistryBackedPreferences::new(registry.app_name(), None);
    let g09 = RegistryBackedPreferences::new(registry.app_name(), Some("g09"));
    default
        .write_value("SettingsFileMigrationComplete", "true".to_owned())
        .unwrap();
    let acceptance = RegistryBackedPreferences::new(registry.app_name(), Some("acceptance"));

    assert_eq!(
        acceptance
            .read_value("SettingsFileMigrationComplete")
            .unwrap(),
        None
    );
    acceptance
        .write_value("SettingsFileMigrationComplete", "true".to_owned())
        .unwrap();
    acceptance
        .remove_value("SettingsFileMigrationComplete")
        .unwrap();

    assert_eq!(
        acceptance
            .read_value("SettingsFileMigrationComplete")
            .unwrap(),
        None
    );
    assert_eq!(
        default
            .read_value("SettingsFileMigrationComplete")
            .unwrap()
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        g09.read_value("SettingsFileMigrationComplete").unwrap(),
        None
    );
}
