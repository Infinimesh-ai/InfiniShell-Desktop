#[cfg(all(windows, target_arch = "x86_64"))]
use super::super::{CliAgentPluginManager, codex::CodexPluginManager};
use super::*;
#[cfg(all(windows, target_arch = "x86_64"))]
use crate::features::FeatureFlag;

fn controlled_home() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("plugins/cache/codex-warp/warp/0.4.0");
    // 使用完整受测插件，确保这些测试确实经过生产完整树校验与原生配置读取。
    for (relative, contents) in [
        (".codex-plugin/plugin.json", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/.codex-plugin/plugin.json").as_slice()),
        ("hooks/hooks.json", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/hooks/hooks.json").as_slice()),
        ("scripts/build-payload.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/build-payload.sh").as_slice()),
        ("scripts/on-permission-request.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-permission-request.sh").as_slice()),
        ("scripts/on-post-tool-use.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-post-tool-use.sh").as_slice()),
        ("scripts/on-prompt-submit.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-prompt-submit.sh").as_slice()),
        ("scripts/on-session-start.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-session-start.sh").as_slice()),
        ("scripts/on-stop.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/on-stop.sh").as_slice()),
        ("scripts/should-use-structured.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/should-use-structured.sh").as_slice()),
        ("scripts/warp-notify.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/source/plugins/warp/scripts/warp-notify.sh").as_slice()),
    ] {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    fs::write(directory.path().join("config.toml"), config().to_string()).unwrap();
    directory
}

fn contents(home: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    file_stamps(home)
        .unwrap()
        .into_iter()
        .filter(|(path, _, _)| path.is_file())
        .map(|(path, _, _)| {
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

fn config() -> DocumentMut {
    let mut value = String::new();
    for hook in &HOOKS.hooks {
        value.push_str(&format!(
            "[hooks.state.{}]\nenabled = true\ntrusted_hash = {}\n",
            serde_json::to_string(&hook.key).unwrap(),
            serde_json::to_string(&hook.current_hash).unwrap()
        ));
    }
    value.parse().unwrap()
}

#[test]
fn missing_native_trust_needs_review() {
    assert_eq!(
        configured_status(&DocumentMut::new()),
        NativeAuthorizationStatus::Required
    );
}

#[test]
fn exact_native_hashes_configure_but_do_not_prove_activation() {
    assert_eq!(
        configured_status(&config()),
        NativeAuthorizationStatus::Configured
    );
}

#[test]
fn disabled_modified_or_missing_hooks_never_become_configured() {
    for hook in &HOOKS.hooks {
        let mut disabled = config();
        disabled["hooks"]["state"][&hook.key]["enabled"] = toml_edit::value(false);
        assert_eq!(
            configured_status(&disabled),
            NativeAuthorizationStatus::Required
        );
        let mut modified = config();
        modified["hooks"]["state"][&hook.key]["trusted_hash"] = toml_edit::value("sha256:old");
        assert_eq!(
            configured_status(&modified),
            NativeAuthorizationStatus::Required
        );
        let mut missing = config();
        missing["hooks"]["state"]
            .as_table_mut()
            .unwrap()
            .remove(&hook.key);
        assert_eq!(
            configured_status(&missing),
            NativeAuthorizationStatus::Required
        );
    }
}

#[test]
fn native_contract_is_exactly_the_recorded_cli_response_and_bundled_manifest() {
    use sha2::{Digest as _, Sha256};
    let source: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-native-hook-trust-rev4-macos.json"
    ))
    .unwrap();
    let bundled: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../assets/bundled/cli-agent-plugins/codex/NATIVE_HOOK_TRUST.json"
    ))
    .unwrap();
    assert_eq!(source["hooks"], bundled["hooks"]);
    assert_eq!(bundled["cli"], "codex-cli 0.147.0");
    assert_eq!(bundled["plugin_version"], "0.4.0");
    assert_eq!(bundled["patch_revision"], 4);
    assert_eq!(bundled["verified_platform"], "macos");
    assert_eq!(bundled["windows_verified"], false);
    assert_eq!(
        bundled["source_metadata_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../../../assets/bundled/cli-agent-plugins/codex/revisions/rev4/SOURCE_METADATA.json"
            ))
        )
    );
    let patch_metadata: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../assets/bundled/cli-agent-plugins/codex/PATCH_METADATA.json"
    ))
    .unwrap();
    assert_eq!(patch_metadata["patch_revision"], 5);
    assert_eq!(
        bundled["cli"],
        format!(
            "codex-cli {}",
            patch_metadata["cli_contract_version"].as_str().unwrap()
        )
    );
    assert_eq!(
        bundled["hooks_file_sha256"],
        patch_metadata["files"]["hooks/hooks.json"]["replacement_sha256"]
    );
    assert_eq!(HOOKS.hooks.len(), 5);
    assert_eq!(
        bundled["hooks_file_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../../../assets/bundled/cli-agent-plugins/codex/hooks/hooks.json"
            ))
        )
    );
}

#[test]
fn trust_query_is_read_only_and_malformed_config_stays_unknown() {
    let directory = controlled_home();
    let content = "not valid [toml\n";
    fs::write(directory.path().join("config.toml"), content).unwrap();
    let before = contents(directory.path());
    assert_eq!(status(directory.path()), NativeAuthorizationStatus::Unknown);
    assert_eq!(contents(directory.path()), before);
}

#[test]
fn invalid_enabled_type_is_unknown_instead_of_treated_as_enabled() {
    let mut value = config();
    value["hooks"]["state"][&HOOKS.hooks[0].key]["enabled"] = toml_edit::value("false");
    assert_eq!(
        configured_status(&value),
        NativeAuthorizationStatus::Unknown
    );
}

#[test]
fn explicit_invalidation_removes_only_the_requested_home() {
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    for key in [home.path().to_owned(), other.path().to_owned()] {
        cache.insert(
            key,
            CachedStatus {
                checked_at: Instant::now(),
                stamps: Vec::new(),
                status: NativeAuthorizationStatus::Configured,
            },
        );
    }
    drop(cache);
    invalidate(home.path());
    let cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    assert!(!cache.contains_key(home.path()));
    assert!(cache.contains_key(other.path()));
}

#[test]
fn cold_cache_reads_configured_trust_without_installing_or_probing_a_runtime() {
    let directory = controlled_home();
    let before_stamps = file_stamps(directory.path()).unwrap();
    let before_contents = contents(directory.path());

    assert_eq!(
        status(directory.path()),
        NativeAuthorizationStatus::Configured
    );
    assert_eq!(
        status(directory.path()),
        NativeAuthorizationStatus::Configured
    );

    // 清空本目录的进程内缓存，模拟重启后的首次读取；不能重新要求安装时的版本缓存。
    invalidate(directory.path());
    assert_eq!(
        status(directory.path()),
        NativeAuthorizationStatus::Configured
    );
    assert_eq!(file_stamps(directory.path()).unwrap(), before_stamps);
    assert_eq!(contents(directory.path()), before_contents);
}

#[test]
fn matching_native_hashes_do_not_configure_an_incomplete_plugin_tree() {
    let directory = controlled_home();
    fs::remove_file(
        directory
            .path()
            .join("plugins/cache/codex-warp/warp/0.4.0/scripts/on-session-start.sh"),
    )
    .unwrap();
    assert_eq!(status(directory.path()), NativeAuthorizationStatus::Unknown);
}

#[test]
fn expired_cache_observes_native_authorization_revocation() {
    let directory = controlled_home();
    assert_eq!(
        status(directory.path()),
        NativeAuthorizationStatus::Configured
    );
    let mut revoked = config();
    revoked["hooks"]["state"][&HOOKS.hooks[0].key]["enabled"] = toml_edit::value(false);
    fs::write(directory.path().join("config.toml"), revoked.to_string()).unwrap();
    CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_mut(directory.path())
        .unwrap()
        .checked_at = Instant::now() - Duration::from_secs(2);

    assert_eq!(
        status(directory.path()),
        NativeAuthorizationStatus::Required
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("config.toml")).unwrap(),
        revoked.to_string()
    );
}

#[test]
fn recreated_cache_rejects_changed_trust_and_damaged_plugin_files() {
    let directory = controlled_home();
    assert_eq!(
        status(directory.path()),
        NativeAuthorizationStatus::Configured
    );
    let mut changed = config();
    changed["hooks"]["state"][&HOOKS.hooks[0].key]["trusted_hash"] = toml_edit::value("sha256:old");
    fs::write(directory.path().join("config.toml"), changed.to_string()).unwrap();
    invalidate(directory.path());
    assert_eq!(
        status(directory.path()),
        NativeAuthorizationStatus::Required
    );

    fs::write(directory.path().join("config.toml"), config().to_string()).unwrap();
    let modified = directory
        .path()
        .join("plugins/cache/codex-warp/warp/0.4.0/scripts/on-stop.sh");
    fs::write(&modified, "用户保留的自定义脚本").unwrap();
    invalidate(directory.path());
    assert_eq!(status(directory.path()), NativeAuthorizationStatus::Unknown);
    assert_eq!(
        fs::read_to_string(modified).unwrap(),
        "用户保留的自定义脚本"
    );
}

#[test]
fn recreated_cache_does_not_reuse_trust_after_config_removal() {
    let directory = controlled_home();
    assert_eq!(
        status(directory.path()),
        NativeAuthorizationStatus::Configured
    );
    fs::remove_file(directory.path().join("config.toml")).unwrap();
    invalidate(directory.path());

    assert_eq!(
        status(directory.path()),
        NativeAuthorizationStatus::Required
    );
    assert!(!directory.path().join("config.toml").exists());
}

#[cfg(unix)]
#[test]
fn literal_backslash_filename_cannot_impersonate_a_controlled_path() {
    let directory = controlled_home();
    let root = directory.path().join("plugins/cache/codex-warp/warp/0.4.0");
    fs::rename(
        root.join("scripts/on-session-start.sh"),
        root.join("scripts\\on-session-start.sh"),
    )
    .unwrap();

    assert_eq!(status(directory.path()), NativeAuthorizationStatus::Unknown);
}

// macOS 文件系统在 rename 阶段以 EILSEQ 拒绝此文件名；本测试只在 Linux 验证产品拒绝路径。
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_filename_cannot_enter_the_controlled_tree() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let directory = controlled_home();
    let root = directory.path().join("plugins/cache/codex-warp/warp/0.4.0");
    fs::rename(
        root.join("scripts/on-session-start.sh"),
        root.join(OsString::from_vec(b"untracked-\xff".to_vec())),
    )
    .unwrap();

    assert_eq!(status(directory.path()), NativeAuthorizationStatus::Unknown);
}

#[test]
fn manual_authorization_is_inside_codex_and_has_no_automatic_shell_execution() {
    use super::super::{CliAgentPluginManager as _, codex::CodexPluginManager};
    use crate::features::FeatureFlag;
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let manager = CodexPluginManager::new(None);
    let instructions = manager.native_authorization_instructions().unwrap();
    assert_eq!(instructions.steps.len(), 1);
    assert_eq!(instructions.steps[0].command, "/hooks");
    assert!(!instructions.steps[0].executable);
}

#[test]
fn windows_contract_uses_only_the_formal_five_hooks_and_exact_resource_bytes() {
    use sha2::{Digest as _, Sha256};
    let source: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-native-hook-trust-rev4-windows.json"
    ))
    .unwrap();
    let bundled: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../assets/bundled/cli-agent-plugins/codex/NATIVE_HOOK_TRUST_WINDOWS.json"
    ))
    .unwrap();
    assert_eq!(source["hooks"], bundled["hooks"]);
    assert_eq!(
        source["native_artifact_sha256"],
        bundled["native_artifact_sha256"]
    );
    assert_eq!(bundled["cli"], "codex-cli 0.147.0");
    assert_eq!(bundled["patch_revision"], 4);
    assert_eq!(bundled["verified_platform"], "windows");
    assert_eq!(bundled["verified_architecture"], "x86_64");
    assert_eq!(bundled["native_hook_lifecycle_verified"], false);
    assert_eq!(bundled["windows_product_enabled"], false);
    assert_eq!(bundled["credentials_provided"], false);
    assert_eq!(bundled["model_request_attempted"], false);
    assert_eq!(
        bundled["source_metadata_sha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../../../assets/bundled/cli-agent-plugins/codex/revisions/rev4/SOURCE_METADATA.json"
            ))
        )
    );
    let hooks_bytes =
        include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/hooks/hooks.json");
    assert_eq!(
        bundled["hooks_file_sha256"],
        format!("{:x}", Sha256::digest(hooks_bytes))
    );
    let hooks: serde_json::Value = serde_json::from_slice(hooks_bytes).unwrap();
    let recorded = bundled["hooks"].as_array().unwrap();
    assert_eq!(recorded.len(), 5);
    assert_eq!(hooks["hooks"].as_object().unwrap().len(), recorded.len());
    for hook in recorded {
        let (event, entries) = hooks["hooks"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(event, _)| event.eq_ignore_ascii_case(hook["eventName"].as_str().unwrap()))
            .unwrap();
        assert!(!event.contains("probe"));
        let command = entries[0]["hooks"][0]["commandWindows"].as_str().unwrap();
        assert_eq!(
            hook["command_sha256"],
            format!("{:x}", Sha256::digest(command.as_bytes()))
        );
        assert_eq!(hook["trustStatus"], "untrusted");
        assert_eq!(hook["enabled"], true);
        assert_eq!(hook["pluginId"], "warp@codex-warp");
    }
    assert_eq!(source["cases"].as_array().unwrap().len(), 2);
    for case in source["cases"].as_array().unwrap() {
        let registration = &case["formal_registration"];
        assert_eq!(registration["hook_count"], 5);
        assert_eq!(registration["mode"], "formal");
        assert_eq!(registration["scripts_instrumented"], false);
        assert_eq!(registration["source_manifest_modified"], false);
        assert_eq!(registration["no_thread_or_turn_started"], true);
        assert_eq!(registration["native_trust_confirmed"], true);
        assert_eq!(registration["config_rollback"]["restored"], true);
        for hook in recorded {
            let key = hook["key"].as_str().unwrap();
            assert_eq!(case["untrusted_hashes"][key], hook["currentHash"]);
            assert_eq!(case["trusted_hashes"][key], hook["currentHash"]);
        }
    }
    let directory = controlled_home();
    let plugin = directory.path().join("plugins/cache/codex-warp/warp/0.4.0");
    let tree = source["resource_tree_sha256"].as_object().unwrap();
    assert_eq!(tree.len(), 10);
    for (relative, expected) in tree {
        assert_eq!(
            *expected,
            format!(
                "{:x}",
                Sha256::digest(fs::read(plugin.join(relative)).unwrap())
            )
        );
    }
}

#[test]
fn selected_contract_matches_the_current_platform() {
    let contract: serde_json::Value = serde_json::from_str(CONTRACT).unwrap();
    assert_eq!(
        contract["verified_platform"],
        if cfg!(windows) { "windows" } else { "macos" }
    );
    assert_eq!(contract["hooks"].as_array().unwrap().len(), 5);
}

#[test]
fn any_other_platform_hook_hash_requires_native_authorization() {
    let other: serde_json::Value = serde_json::from_str(if cfg!(windows) {
        include_str!("../../../../assets/bundled/cli-agent-plugins/codex/NATIVE_HOOK_TRUST.json")
    } else {
        include_str!(
            "../../../../assets/bundled/cli-agent-plugins/codex/NATIVE_HOOK_TRUST_WINDOWS.json"
        )
    })
    .unwrap();
    for hook in other["hooks"].as_array().unwrap() {
        let mut value = config();
        value["hooks"]["state"][hook["key"].as_str().unwrap()]["trusted_hash"] =
            toml_edit::value(hook["currentHash"].as_str().unwrap());
        assert_eq!(
            configured_status(&value),
            NativeAuthorizationStatus::Required
        );
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
#[serial_test::serial]
fn windows_manager_reads_trust_without_mutation_before_install_preflight() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let home = controlled_home();
    fs::write(
        home.path().join("config.toml"),
        format!(
            "{}\n[plugins.\"warp@codex-warp\"]\nenabled = true\n",
            config()
        ),
    )
    .unwrap();
    let before = contents(home.path());
    let previous = std::env::var_os("CODEX_HOME");
    unsafe { std::env::set_var("CODEX_HOME", home.path()) };
    let manager = CodexPluginManager::new(None);
    let native_status = manager.native_authorization_status();
    let can_install = manager.can_auto_install();
    match previous {
        Some(value) => unsafe { std::env::set_var("CODEX_HOME", value) },
        None => unsafe { std::env::remove_var("CODEX_HOME") },
    }
    assert_eq!(native_status, NativeAuthorizationStatus::Configured);
    assert!(can_install);
    assert_eq!(contents(home.path()), before);
}
