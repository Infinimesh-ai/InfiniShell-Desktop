use std::fs;

use serde_json::json;

use super::{
    BUNDLED_FILES, PLUGIN_VERSION, backup_plugin, installed_plugin, plugin_disabled,
    plugin_enabled, runtime_is_compatible, write_bundle,
};

fn install_fixture(root: &std::path::Path) -> std::path::PathBuf {
    let source = write_bundle(&root.join("source")).unwrap();
    let installed = backup_plugin(&source, &root.join("installed-plugins")).unwrap();
    fs::write(
        root.join("installed-plugins/registry.json"),
        serde_json::to_string(&json!({
            "version": 1,
            "repos": {
                "source-one": {
                    "kind": {"type": "Local", "source_path": source},
                    "path": installed,
                    "plugins": {"infinishell-grok": {"version": PLUGIN_VERSION}}
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    installed
}

#[test]
fn native_registry_reads_verified_installed_files() {
    let directory = tempfile::tempdir().unwrap();
    let installed = install_fixture(directory.path());
    let plugin = installed_plugin(directory.path()).unwrap().unwrap();
    assert_eq!(plugin.path, installed);
    assert_eq!(plugin.version, "0.1.0");
}

#[test]
fn native_disabled_state_wins_even_if_enabled_list_contains_plugin() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("config.toml"), "[ui]\nscreen_mode='minimal'\n[plugins]\nenabled=['infinishell-grok']\ndisabled=['user/hash/infinishell-grok']\n").unwrap();
    assert!(plugin_disabled(directory.path()).unwrap());
    assert!(!plugin_enabled(directory.path()).unwrap());
    assert!(
        fs::read_to_string(directory.path().join("config.toml"))
            .unwrap()
            .contains("screen_mode='minimal'")
    );
}

#[test]
fn malformed_native_config_does_not_allow_automatic_install() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("config.toml"), "[plugins").unwrap();
    assert!(plugin_disabled(directory.path()).is_err());
}

#[test]
fn invalid_disabled_list_does_not_silently_enable_plugin() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("config.toml"),
        "[plugins]\ndisabled='infinishell-grok'\n",
    )
    .unwrap();
    assert!(plugin_disabled(directory.path()).is_err());
}

#[test]
fn missing_plugin_file_does_not_count_as_installed() {
    let directory = tempfile::tempdir().unwrap();
    let installed = install_fixture(directory.path());
    fs::remove_file(installed.join("hooks/notify.cjs")).unwrap();
    assert!(installed_plugin(directory.path()).is_err());
}

#[test]
fn stale_registry_version_does_not_hide_ineffective_update() {
    let directory = tempfile::tempdir().unwrap();
    let installed = install_fixture(directory.path());
    fs::write(
        installed.join(".grok-plugin/plugin.json"),
        r#"{"name":"infinishell-grok","version":"0.0.9"}"#,
    )
    .unwrap();
    assert!(installed_plugin(directory.path()).is_err());
}

#[test]
fn duplicate_plugin_registration_requires_explicit_repair() {
    let directory = tempfile::tempdir().unwrap();
    install_fixture(directory.path());
    let registry_path = directory.path().join("installed-plugins/registry.json");
    let mut registry: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&registry_path).unwrap()).unwrap();
    registry["repos"]["source-two"] = registry["repos"]["source-one"].clone();
    fs::write(registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
    assert!(installed_plugin(directory.path()).is_err());
}

#[test]
fn bundled_version_and_hook_runtime_match_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let source = write_bundle(directory.path()).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(source.join(".grok-plugin/plugin.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["version"], PLUGIN_VERSION);
    assert_eq!(BUNDLED_FILES.len(), 4);
    assert!(
        fs::read_to_string(source.join("hooks/hooks.json"))
            .unwrap()
            .contains("process.env.GROK_PLUGIN_ROOT")
    );
}

#[test]
fn existing_modified_bundle_is_not_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let source = write_bundle(directory.path()).unwrap();
    fs::write(source.join("README.md"), "user modification").unwrap();
    assert!(write_bundle(directory.path()).is_err());
    assert_eq!(
        fs::read_to_string(source.join("README.md")).unwrap(),
        "user modification"
    );
}

#[test]
fn recovery_snapshot_survives_removal_of_original_installation() {
    let directory = tempfile::tempdir().unwrap();
    let source = write_bundle(&directory.path().join("source")).unwrap();
    let recovery = backup_plugin(&source, &directory.path().join("backups")).unwrap();
    fs::remove_dir_all(&source).unwrap();
    assert_eq!(
        fs::read_to_string(recovery.join("hooks/notify.cjs")).unwrap(),
        BUNDLED_FILES[2].1
    );
}

#[test]
fn only_tested_grok_and_supported_node_versions_pass_runtime_probe() {
    assert!(runtime_is_compatible(
        "grok 1.0.30 (04b7ffed98c6)\n",
        "v22.18.0\n"
    ));
    assert!(!runtime_is_compatible("grok 1.0.29\n", "v22.18.0\n"));
    assert!(!runtime_is_compatible("grok 1.0.31\n", "v22.18.0\n"));
    assert!(!runtime_is_compatible("grok 1.0.30\n", "v16.20.0\n"));
    assert!(!runtime_is_compatible("grok unknown\n", "v22.18.0\n"));
}

#[test]
fn remote_install_instructions_do_not_substitute_local_enable_only_instructions() {
    use super::{CliAgentPluginManager, GrokPluginManager};
    let instructions = GrokPluginManager::new(None).remote_install_instructions();
    assert!(std::ptr::eq(instructions, &*super::INSTALL_INSTRUCTIONS));
    assert!(!std::ptr::eq(instructions, &*super::ENABLE_INSTRUCTIONS));
    assert_eq!(instructions.steps[0].command, "grok --version");
    assert_eq!(instructions.steps[1].command, "node --version");
}
