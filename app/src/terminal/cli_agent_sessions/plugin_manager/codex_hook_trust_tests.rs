use super::*;

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
    let bundled: serde_json::Value = serde_json::from_str(CONTRACT).unwrap();
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
                "../../../../assets/bundled/cli-agent-plugins/codex/SOURCE_METADATA.json"
            ))
        )
    );
    let patch_metadata: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../assets/bundled/cli-agent-plugins/codex/PATCH_METADATA.json"
    ))
    .unwrap();
    assert_eq!(bundled["patch_revision"], patch_metadata["patch_revision"]);
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
