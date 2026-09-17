use std::fs;
use std::path::{Path, PathBuf};

use super::{
    ClaudeCodePluginManager, CliAgentPluginManager, MINIMUM_PLATFORM_PLUGIN_VERSION,
    check_installed, check_platform_plugin_installed, claude_code_marketplace_has_local_override,
    installed_platform_plugin_version, installed_version,
};

fn transaction_tempdir() -> tempfile::TempDir {
    tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
}

fn transaction_installation(home: &Path, version: &str) -> PathBuf {
    let trees: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../specs/cli-agent-parity/fixtures/claude-warp-compatible-original-trees.json"
    ))
    .unwrap();
    let cache = home
        .join("plugins/cache/claude-code-warp/warp")
        .join(version);
    for (name, contents) in trees[version].as_object().unwrap() {
        let path = cache.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents.as_str().unwrap()).unwrap();
    }
    let source = serde_json::json!({"source":"github","repo":"warpdotdev/claude-code-warp"});
    let documents = [
        (
            "settings.json",
            serde_json::json!({"enabledPlugins":{"warp@claude-code-warp":true},"extraKnownMarketplaces":{"claude-code-warp":{"source":source}}}),
        ),
        (
            "plugins/known_marketplaces.json",
            serde_json::json!({"claude-code-warp":{"source":source,"installLocation":home.join("plugins/marketplaces/claude-code-warp")}}),
        ),
        (
            "plugins/installed_plugins.json",
            serde_json::json!({"version":2,"plugins":{"warp@claude-code-warp":[{"scope":"user","installPath":cache,"version":version}]}}),
        ),
    ];
    for (file, value) in documents {
        fs::write(home.join(file), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    cache
}

fn transaction_original(home: &Path) -> [serde_json::Value; 3] {
    [
        super::read_claude_document(home, "plugins/known_marketplaces.json")
            .unwrap()
            .1,
        super::read_claude_document(home, "settings.json")
            .unwrap()
            .1,
        super::read_claude_document(home, "plugins/installed_plugins.json")
            .unwrap()
            .1,
    ]
}

fn patched_transaction_stage() -> tempfile::TempDir {
    let stage = transaction_tempdir();
    let cache = transaction_installation(stage.path(), "2.2.0");
    // 事务测试从已修补的缓存开始，不以通知运行时的平台门禁代替文件发布验证。
    // 使用真实随附内容并核对完整缓存；Windows 可测试事务，但仍不能自动安装通知。
    for (relative, contents) in [
        (
            "scripts/build-payload.sh",
            include_str!(
                "../../../../assets/bundled/cli-agent-plugins/claude/scripts/build-payload.sh"
            ),
        ),
        (
            "scripts/on-stop.sh",
            include_str!("../../../../assets/bundled/cli-agent-plugins/claude/scripts/on-stop.sh"),
        ),
        (
            "scripts/should-use-structured.sh",
            include_str!(
                "../../../../assets/bundled/cli-agent-plugins/claude/scripts/should-use-structured.sh"
            ),
        ),
        (
            "hooks/hooks.json",
            include_str!("../../../../assets/bundled/cli-agent-plugins/claude/hooks/hooks.json"),
        ),
        (
            "scripts/warp-notify.sh",
            include_str!(
                "../../../../assets/bundled/cli-agent-plugins/claude/scripts/warp-notify.sh"
            ),
        ),
    ] {
        fs::write(cache.join(relative), contents).unwrap();
    }
    super::notification_patch::verify_staged_claude_cache(&cache).unwrap();
    stage
}

#[cfg(windows)]
#[test]
fn claude_windows_notification_installation_is_rejected_without_changing_files() {
    let home = transaction_tempdir();
    let cache = transaction_installation(home.path(), "2.2.0");
    let original = transaction_original(home.path());
    assert!(super::notification_patch::preflight(home.path(), super::PatchKind::Claude).unwrap());
    assert!(!ClaudeCodePluginManager::new(None, None, None).can_auto_install());

    let error =
        super::notification_patch::apply(home.path(), super::PatchKind::Claude, "").unwrap_err();

    assert_eq!(
        error.message,
        super::notification_patch::unsupported().message
    );
    assert!(error.log.is_empty());
    assert_eq!(transaction_original(home.path()), original);
    let trees: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../specs/cli-agent-parity/fixtures/claude-warp-compatible-original-trees.json"
    ))
    .unwrap();
    for (relative, contents) in trees["2.2.0"].as_object().unwrap() {
        assert_eq!(
            fs::read_to_string(cache.join(relative)).unwrap(),
            contents.as_str().unwrap(),
            "Windows 拒绝安装后必须保留原文件：{relative}"
        );
    }
    assert!(!home.path().join(super::CLAUDE_PUBLICATION_JOURNAL).exists());
}

#[test]
fn claude_upgrade_publishes_patched_cache_and_keeps_old_version_files() {
    let home = transaction_tempdir();
    let old_cache = transaction_installation(home.path(), "2.1.0");
    let previous_hook = fs::read(old_cache.join("hooks/hooks.json")).unwrap();
    let stage = patched_transaction_stage();
    let original = transaction_original(home.path());
    super::publish_staged_claude(home.path(), stage.path(), &original, |_, _| Ok(())).unwrap();
    assert_eq!(installed_version(home.path()).as_deref(), Some("2.2.0"));
    assert!(super::notification_patch::is_applied(
        home.path(),
        super::PatchKind::Claude
    ));
    assert_eq!(
        fs::read(old_cache.join("hooks/hooks.json")).unwrap(),
        previous_hook
    );
    assert!(!old_cache.join(".orphaned_at").exists());
    assert!(!home.path().join(super::CLAUDE_PUBLICATION_JOURNAL).exists());
}

#[test]
fn claude_patch_failure_keeps_original_active_registry_unchanged() {
    let home = transaction_tempdir();
    transaction_installation(home.path(), "2.1.0");
    let stage = patched_transaction_stage();
    fs::write(
        stage
            .path()
            .join("plugins/cache/claude-code-warp/warp/2.2.0/scripts/on-stop.sh"),
        "并发修改",
    )
    .unwrap();
    let before = fs::read(home.path().join("plugins/installed_plugins.json")).unwrap();
    assert!(
        super::publish_staged_claude(
            home.path(),
            stage.path(),
            &transaction_original(home.path()),
            |_, _| Ok(())
        )
        .is_err()
    );
    assert_eq!(
        fs::read(home.path().join("plugins/installed_plugins.json")).unwrap(),
        before
    );
    assert_eq!(installed_version(home.path()).as_deref(), Some("2.1.0"));
    assert!(
        !home
            .path()
            .join("plugins/cache/claude-code-warp/warp/2.2.0")
            .exists()
    );
}

#[test]
fn claude_publication_failure_restores_registry_and_preserves_unrelated_edits() {
    let home = transaction_tempdir();
    transaction_installation(home.path(), "2.1.0");
    let stage = patched_transaction_stage();
    let original = transaction_original(home.path());
    assert!(
        super::publish_staged_claude(home.path(), stage.path(), &original, |step, home| {
            if step == 4 {
                let mut registry =
                    super::read_claude_document(home, "plugins/installed_plugins.json")?.1;
                registry["plugins"]["other@user-marketplace"] =
                    serde_json::json!([{"version":"9.0.0"}]);
                fs::write(
                    home.join("plugins/installed_plugins.json"),
                    serde_json::to_vec(&registry)?,
                )?;
                return Err(std::io::Error::other("注入发布后失败"));
            }
            Ok(())
        })
        .is_err()
    );
    let registry = super::read_claude_document(home.path(), "plugins/installed_plugins.json")
        .unwrap()
        .1;
    assert_eq!(
        registry["plugins"]["warp@claude-code-warp"],
        original[2]["plugins"]["warp@claude-code-warp"]
    );
    assert_eq!(
        registry["plugins"]["other@user-marketplace"][0]["version"],
        "9.0.0"
    );
    assert!(!home.path().join(super::CLAUDE_PUBLICATION_JOURNAL).exists());
}

#[test]
fn claude_recovery_after_interrupted_publication_restores_old_active_version() {
    let home = transaction_tempdir();
    transaction_installation(home.path(), "2.1.0");
    let stage = patched_transaction_stage();
    let original = transaction_original(home.path());
    let result = std::panic::catch_unwind(|| {
        super::publish_staged_claude(home.path(), stage.path(), &original, |step, _| {
            assert_ne!(step, 4, "模拟发布写入后进程中断，不进入函数错误恢复分支");
            Ok(())
        })
    });
    assert!(result.is_err());
    assert_eq!(installed_version(home.path()).as_deref(), Some("2.2.0"));
    assert!(home.path().join(super::CLAUDE_PUBLICATION_JOURNAL).exists());
    super::recover_claude_publication(home.path()).unwrap();
    assert_eq!(installed_version(home.path()).as_deref(), Some("2.1.0"));
    assert!(
        home.path()
            .join("plugins/cache/claude-code-warp/warp/2.2.0")
            .exists()
    );
    assert!(!home.path().join(super::CLAUDE_PUBLICATION_JOURNAL).exists());
}

#[test]
fn claude_concurrent_disable_is_preserved_and_blocks_recovery() {
    let home = transaction_tempdir();
    transaction_installation(home.path(), "2.1.0");
    let stage = patched_transaction_stage();
    let original = transaction_original(home.path());
    assert!(
        super::publish_staged_claude(home.path(), stage.path(), &original, |step, home| {
            if step == 2 {
                let mut settings = super::read_claude_document(home, "settings.json")?.1;
                settings["enabledPlugins"]["warp@claude-code-warp"] = false.into();
                settings["theme"] = "user-theme".into();
                fs::write(home.join("settings.json"), serde_json::to_vec(&settings)?)?;
            }
            Ok(())
        })
        .is_err()
    );
    assert!(super::recover_claude_publication(home.path()).is_err());
    assert_eq!(installed_version(home.path()).as_deref(), Some("2.1.0"));
    let settings = super::read_claude_document(home.path(), "settings.json")
        .unwrap()
        .1;
    assert_eq!(settings["enabledPlugins"]["warp@claude-code-warp"], false);
    assert_eq!(settings["theme"], "user-theme");
    assert!(home.path().join(super::CLAUDE_PUBLICATION_JOURNAL).exists());
}

#[test]
fn claude_publication_lock_does_not_guess_old_operation_liveness() {
    let home = transaction_tempdir();
    let first = super::lock_claude_publication(home.path()).unwrap();
    assert!(super::lock_claude_publication(home.path()).is_err());
    drop(first);
    assert!(super::lock_claude_publication(home.path()).is_ok());
}

#[test]
fn claude_unknown_recovery_record_is_preserved_without_writes() {
    let home = transaction_tempdir();
    transaction_installation(home.path(), "2.1.0");
    let journal = home.path().join(super::CLAUDE_PUBLICATION_JOURNAL);
    fs::write(&journal, br#"{"version":99,"documents":[]}"#).unwrap();
    let before = fs::read(home.path().join("plugins/installed_plugins.json")).unwrap();
    assert!(super::recover_claude_publication(home.path()).is_err());
    assert_eq!(
        fs::read(home.path().join("plugins/installed_plugins.json")).unwrap(),
        before
    );
    assert_eq!(
        fs::read(&journal).unwrap(),
        br#"{"version":99,"documents":[]}"#
    );
}

#[cfg(unix)]
#[test]
fn claude_journal_and_lock_reject_symbolic_and_hard_links() {
    let home = transaction_tempdir();
    fs::create_dir(home.path().join("plugins")).unwrap();
    let external = tempfile::NamedTempFile::new().unwrap();
    let journal = home.path().join(super::CLAUDE_PUBLICATION_JOURNAL);
    std::os::unix::fs::symlink(external.path(), &journal).unwrap();
    assert!(super::recover_claude_publication(home.path()).is_err());
    fs::remove_file(&journal).unwrap();
    fs::hard_link(external.path(), &journal).unwrap();
    assert!(super::recover_claude_publication(home.path()).is_err());
    let lock = home
        .path()
        .join("plugins/infinishell-claude-publication.lock");
    std::os::unix::fs::symlink(external.path(), &lock).unwrap();
    assert!(super::lock_claude_publication(home.path()).is_err());
    assert_eq!(fs::read(external.path()).unwrap(), b"");
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "需要显式私有 HOME 和受测 CLI；由独立无模型探针启动"]
async fn real_claude_plugin_transaction() {
    let root = PathBuf::from(std::env::var("INFINISHELL_CLAUDE_TRANSACTION_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    assert_eq!(
        fs::read_to_string(root.join("private-no-model-fixture")).unwrap(),
        "claude-plugin-transaction\n"
    );
    let home = root.join("home/.claude");
    assert_eq!(
        PathBuf::from(std::env::var("CLAUDE_CONFIG_DIR").unwrap())
            .canonicalize()
            .unwrap(),
        home
    );
    assert_eq!(
        PathBuf::from(std::env::var("HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("home")
    );
    assert!(!home.join(".credentials.json").exists());
    assert_eq!(installed_version(&home).as_deref(), Some("2.1.0"));
    let original_registry = fs::read(home.join("plugins/installed_plugins.json")).unwrap();
    let original_hook =
        fs::read(home.join("plugins/cache/claude-code-warp/warp/2.1.0/hooks/hooks.json")).unwrap();
    let case = std::env::var("INFINISHELL_CLAUDE_TRANSACTION_CASE").unwrap();
    let path = std::env::var("PATH").unwrap();
    match case.as_str() {
        "upgrade" => {
            ClaudeCodePluginManager::new(None, None, Some(path))
                .notification_operation()
                .await
                .unwrap();
            assert_eq!(installed_version(&home).as_deref(), Some("2.2.0"));
            assert!(super::notification_patch::is_applied(
                &home,
                super::PatchKind::Claude
            ));
            // 完整 manager 路径必须保留发布时已验证的整棵缓存树。
            assert!(super::notification_patch::preflight(&home, super::PatchKind::Claude).unwrap());
            assert!(!home.join(super::CLAUDE_PUBLICATION_JOURNAL).exists());
        }
        "patch-failure" => {
            use std::os::unix::fs::PermissionsExt as _;
            let stage = root.join("patch-failure-native");
            fs::create_dir(&stage).unwrap();
            let mut log = String::new();
            let runtime = super::VerifiedRuntime::probe(
                super::PatchKind::Claude,
                &home,
                Some(&path),
                &mut log,
            )
            .await
            .unwrap()
            .with_home(&stage);
            runtime
                .run(
                    &["plugin", "marketplace", "add", super::MARKETPLACE_REPO],
                    &mut log,
                )
                .await
                .unwrap();
            runtime
                .run(&["plugin", "install", super::PLUGIN_KEY], &mut log)
                .await
                .unwrap();
            let cache = stage.join("plugins/cache/claude-code-warp/warp/2.2.0");
            let original_stop = fs::read(cache.join("scripts/on-stop.sh")).unwrap();
            let hooks = cache.join("hooks");
            let permissions = fs::metadata(&hooks).unwrap().permissions();
            fs::set_permissions(&hooks, fs::Permissions::from_mode(0o500)).unwrap();
            let outcome = super::notification_patch::apply(&stage, super::PatchKind::Claude, &log);
            fs::set_permissions(&hooks, permissions).unwrap();
            assert!(
                outcome.is_err(),
                "必须观察真实写入失败，不能把高权限环境的成功当负向验收"
            );
            assert_eq!(
                fs::read(cache.join("scripts/on-stop.sh")).unwrap(),
                original_stop
            );
            assert_eq!(
                fs::read(home.join("plugins/installed_plugins.json")).unwrap(),
                original_registry
            );
            assert_eq!(installed_version(&home).as_deref(), Some("2.1.0"));
        }
        _ => panic!("未知的私有原生事务测试分支"),
    }
    assert_eq!(
        fs::read(home.join("plugins/cache/claude-code-warp/warp/2.1.0/hooks/hooks.json")).unwrap(),
        original_hook
    );
    fs::write(
        root.join(format!("rust-{case}-result.json")),
        serde_json::to_vec_pretty(&serde_json::json!({
            "case":case,"verified":true,"model_requests":0,"old_cache_preserved":true,
            "active_version":installed_version(&home),"rust_entry":"real_claude_plugin_transaction"
        }))
        .unwrap(),
    )
    .unwrap();
}

/// A version strictly below `version`, so below-minimum tests track the
/// constant instead of a hardcoded literal. Assumes `version` > "0.0.0".
fn version_below(version: &str) -> String {
    let mut parts: Vec<u64> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    for part in parts.iter_mut().rev() {
        if *part > 0 {
            *part -= 1;
            break;
        }
    }
    parts
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

#[test]
fn installed_when_plugin_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"version": "1.0.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(check_installed(dir.path()));
}

#[test]
fn explicitly_disabled_plugin_is_not_active() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("plugins")).unwrap();
    fs::write(
        dir.path().join("plugins/installed_plugins.json"),
        r#"{"plugins":{"warp@claude-code-warp":[{"version":"2.1.0"}]}}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("settings.json"),
        r#"{"enabledPlugins":{"warp@claude-code-warp":false},"theme":"dark"}"#,
    )
    .unwrap();
    assert!(!check_installed(dir.path()));
    assert!(super::check_plugin_disabled(
        dir.path(),
        "warp@claude-code-warp"
    ));
    assert_eq!(
        fs::read_to_string(dir.path().join("settings.json")).unwrap(),
        r#"{"enabledPlugins":{"warp@claude-code-warp":false},"theme":"dark"}"#
    );
}

#[test]
fn update_instructions_preserve_marketplace_registration() {
    let instructions = &super::UPDATE_INSTRUCTIONS;
    assert_eq!(
        instructions.steps[0].command,
        "claude plugin marketplace update claude-code-warp"
    );
    assert_eq!(
        instructions.steps[1].command,
        "claude plugin update warp@claude-code-warp"
    );
}

#[test]
fn local_marketplace_override_detects_directory_source() {
    let dir = tempfile::tempdir().unwrap();
    let settings = serde_json::json!({
        "extraKnownMarketplaces": {
            "claude-code-warp": {
                "source": {
                    "path": "/Users/example/Developer/claude-code-warp-internal",
                    "source": "directory"
                }
            }
        }
    });
    fs::write(
        dir.path().join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();

    assert!(claude_code_marketplace_has_local_override(dir.path()));
}

#[test]
fn local_marketplace_override_ignores_repo_source() {
    let dir = tempfile::tempdir().unwrap();
    let settings = serde_json::json!({
        "extraKnownMarketplaces": {
            "claude-code-warp": {
                "source": "warpdotdev/claude-code-warp"
            }
        }
    });
    fs::write(
        dir.path().join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();

    assert!(!claude_code_marketplace_has_local_override(dir.path()));
}

#[test]
#[serial_test::serial]
fn local_marketplace_override_via_trait_uses_claude_config_dir() {
    let dir = tempfile::tempdir().unwrap();
    let settings = serde_json::json!({
        "extraKnownMarketplaces": {
            "claude-code-warp": {
                "source": {
                    "path": "../claude-code-warp-internal",
                    "source": "directory"
                }
            }
        }
    });
    fs::write(
        dir.path().join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.path()) };
    let result = ClaudeCodePluginManager::new(None, None, None).has_local_marketplace_override();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") };

    assert!(result);
}

#[test]
fn installed_platform_plugin_version_returns_version_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"version": MINIMUM_PLATFORM_PLUGIN_VERSION}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert_eq!(
        installed_platform_plugin_version(dir.path()).as_deref(),
        Some(MINIMUM_PLATFORM_PLUGIN_VERSION)
    );
}

#[test]
fn platform_plugin_installed_when_platform_plugin_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"version": MINIMUM_PLATFORM_PLUGIN_VERSION}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(check_platform_plugin_installed(dir.path()));
}

#[test]
#[serial_test::serial]
fn platform_plugin_needs_update_via_trait_when_version_below_minimum() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"version": version_below(MINIMUM_PLATFORM_PLUGIN_VERSION)}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.path()) };
    let result = ClaudeCodePluginManager::new(None, None, None).platform_plugin_needs_update();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") };

    assert!(result);
}

#[test]
#[serial_test::serial]
fn platform_plugin_does_not_need_update_via_trait_when_current() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"version": MINIMUM_PLATFORM_PLUGIN_VERSION}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.path()) };
    let result = ClaudeCodePluginManager::new(None, None, None).platform_plugin_needs_update();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") };

    assert!(!result);
}

#[test]
#[serial_test::serial]
fn platform_plugin_needs_update_via_trait_when_installed_without_version() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "oz-harness-support@claude-code-warp": [{"scope": "user"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.path()) };
    let result = ClaudeCodePluginManager::new(None, None, None).platform_plugin_needs_update();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") };

    assert!(result);
}

#[test]
fn platform_plugin_not_installed_when_only_notification_plugin_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"version": "1.0.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(!check_platform_plugin_installed(dir.path()));
}

#[test]
fn not_installed_when_plugin_key_absent() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "some-other-plugin": [{"version": "1.0.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(!check_installed(dir.path()));
}

#[test]
fn not_installed_when_plugin_array_empty() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": []
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(!check_installed(dir.path()));
}

#[test]
fn not_installed_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!check_installed(dir.path()));
}

#[test]
fn not_installed_when_json_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();
    fs::write(plugins_dir.join("installed_plugins.json"), "not json").unwrap();

    assert!(!check_installed(dir.path()));
}

#[test]
fn not_installed_when_plugins_key_missing() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({"other_key": "value"});
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert!(!check_installed(dir.path()));
}

/// Tests `ClaudeCodePluginManager::is_installed` end-to-end by pointing
/// `CLAUDE_CONFIG_DIR` at a temp directory with a valid installed_plugins.json.
#[test]
#[serial_test::serial]
fn is_installed_via_trait_with_claude_config_dir_env() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"version": "1.0.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.path()) };
    let result = ClaudeCodePluginManager::new(None, None, None).is_installed();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") };

    assert!(result);
}

#[test]
#[serial_test::serial]
fn not_installed_via_trait_when_claude_config_dir_empty() {
    let dir = tempfile::tempdir().unwrap();

    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.path()) };
    let result = ClaudeCodePluginManager::new(None, None, None).is_installed();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") };

    assert!(!result);
}

#[test]
fn installed_version_returns_version_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"version": "1.5.0"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert_eq!(installed_version(dir.path()).as_deref(), Some("1.5.0"));
}

#[test]
fn installed_version_returns_none_when_no_version_field() {
    let dir = tempfile::tempdir().unwrap();
    let plugins_dir = dir.path().join("plugins");
    fs::create_dir_all(&plugins_dir).unwrap();

    let json = serde_json::json!({
        "plugins": {
            "warp@claude-code-warp": [{"scope": "user"}]
        }
    });
    fs::write(
        plugins_dir.join("installed_plugins.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    assert_eq!(installed_version(dir.path()), None);
}

#[test]
fn installed_version_returns_none_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(installed_version(dir.path()), None);
}

#[test]
fn remote_install_instructions_include_installation_instead_of_local_enable_only() {
    let instructions = ClaudeCodePluginManager::new(None, None, None).remote_install_instructions();
    assert!(std::ptr::eq(instructions, &*super::INSTALL_INSTRUCTIONS));
    assert_eq!(
        instructions.steps[0].command,
        "claude plugin marketplace add warpdotdev/claude-code-warp"
    );
    assert!(
        instructions
            .steps
            .iter()
            .any(|step| step.command == "claude plugin install warp@claude-code-warp")
    );
    assert!(
        !instructions
            .steps
            .iter()
            .any(|step| step.command == "claude plugin enable warp@claude-code-warp")
    );
}
