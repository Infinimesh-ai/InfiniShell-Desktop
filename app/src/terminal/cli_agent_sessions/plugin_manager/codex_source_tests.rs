use super::*;

#[cfg(any(unix, all(windows, target_arch = "x86_64")))]
#[path = "codex_source_live_tests.rs"]
mod live_tests;

#[path = "codex_source_recovery_tests.rs"]
mod recovery_tests;

fn private_home() -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().canonicalize().unwrap();
    (directory, home)
}

fn previous_home(home: &Path) -> PathBuf {
    let source = home
        .join("plugins/infinishell-sources")
        .join(&PREVIOUS_BUNDLE.directory)
        .join("source");
    for (name, contents) in FILES {
        let path = source.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(PREVIOUS_BUNDLE.files[*name].mode),
            )
            .unwrap();
        }
    }
    for (name, contents) in [
        ("hooks/hooks.json", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/revisions/rev3/hooks/hooks.json").as_slice()),
        ("scripts/warp-notify.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/revisions/rev3/scripts/warp-notify.sh").as_slice()),
        ("scripts/on-prompt-submit.sh", include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/revisions/rev3/scripts/on-prompt-submit.sh").as_slice()),
    ] {
        fs::write(source.join("plugins/warp").join(name), contents).unwrap();
    }
    fs::write(
        source.parent().unwrap().join("SOURCE_METADATA.json"),
        PREVIOUS_METADATA,
    )
    .unwrap();
    verify_revision(home, &PREVIOUS_BUNDLE, PREVIOUS_METADATA).unwrap();
    for name in revision_tree(&PREVIOUS_BUNDLE, "plugins/warp/", false).keys() {
        let path = cache_root(home, "warp").join("0.4.0").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(source.join("plugins/warp").join(name), path).unwrap();
    }
    let mut document = DocumentMut::new();
    document["marketplaces"][MARKETPLACE]["source_type"] = toml_edit::value("local");
    document["marketplaces"][MARKETPLACE]["source"] = toml_edit::value(source.to_str().unwrap());
    document["plugins"]["warp@codex-warp"]["enabled"] = toml_edit::value(true);
    document["plugins"]["orchestration@codex-warp"]["enabled"] = toml_edit::value(false);
    document["hooks"]["state"]["user"]["trusted_hash"] = toml_edit::value("保持用户信任");
    save(home, &document);
    source
}

fn rev4_home(home: &Path) -> PathBuf {
    let source = home
        .join("plugins/infinishell-sources")
        .join(&REV4_BUNDLE.directory)
        .join("source");
    for (name, contents) in FILES {
        let path = source.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(REV4_BUNDLE.files[*name].mode),
            )
            .unwrap();
        }
    }
    fs::write(
        source.join("plugins/warp/scripts/warp-notify.sh"),
        include_bytes!(
            "../../../../assets/bundled/cli-agent-plugins/codex/revisions/rev4/scripts/warp-notify.sh"
        ),
    )
    .unwrap();
    fs::write(
        source.parent().unwrap().join("SOURCE_METADATA.json"),
        REV4_METADATA,
    )
    .unwrap();
    verify_revision(home, &REV4_BUNDLE, REV4_METADATA).unwrap();
    for name in revision_tree(&REV4_BUNDLE, "plugins/warp/", false).keys() {
        let path = cache_root(home, "warp").join("0.4.0").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(source.join("plugins/warp").join(name), path).unwrap();
    }
    let mut document = DocumentMut::new();
    document["marketplaces"][MARKETPLACE]["source_type"] = toml_edit::value("local");
    document["marketplaces"][MARKETPLACE]["source"] = toml_edit::value(source.to_str().unwrap());
    document["plugins"]["warp@codex-warp"]["enabled"] = toml_edit::value(true);
    save(home, &document);
    source
}

#[test]
fn exact_rev4_migrates_to_rev5_without_overwriting_previous_source() {
    let (_directory, home) = private_home();
    let previous = rev4_home(&home);
    let old_tree = tree(&previous, false).unwrap();
    assert!(!has_custom_source(&home));
    assert!(!is_current(&home));
    assert!(notification_patch::preflight(&home, PatchKind::Codex).unwrap());
    let (transaction, staged, original, installed) = prepared(&home);
    commit_install(
        &home,
        "warp",
        &original,
        &installed,
        &staged,
        transaction.path(),
        |_, _| Ok(()),
    )
    .unwrap();
    invalidate(&home);
    assert!(is_current(&home));
    assert_eq!(tree(&previous, false).unwrap(), old_tree);
    assert!(is_previous_notification_cache(
        &transaction.path().join("previous-cache/0.4.0")
    ));
}

#[test]
fn exact_rev3_migrates_without_overwriting_previous_source_or_trust() {
    let (_directory, home) = private_home();
    let previous = previous_home(&home);
    let old_tree = tree(&previous, false).unwrap();
    assert!(!has_custom_source(&home));
    assert!(!is_current(&home));
    assert!(notification_patch::preflight(&home, PatchKind::Codex).unwrap());
    assert!(!notification_patch::full_tree_is_applied(
        &home,
        PatchKind::Codex
    ));
    let (transaction, staged, original, installed) = prepared(&home);
    commit_install(
        &home,
        "warp",
        &original,
        &installed,
        &staged,
        transaction.path(),
        |_, _| Ok(()),
    )
    .unwrap();
    invalidate(&home);
    assert!(is_current(&home));
    assert_eq!(tree(&previous, false).unwrap(), old_tree);
    assert!(is_previous_notification_cache(
        &transaction.path().join("previous-cache/0.4.0")
    ));
    assert_eq!(
        config(&home)["hooks"]["state"]["user"]["trusted_hash"].as_str(),
        Some("保持用户信任")
    );
    assert_eq!(
        config(&home)["plugins"]["orchestration@codex-warp"]["enabled"].as_bool(),
        Some(false)
    );
}

#[test]
fn rev3_upgrade_failure_restores_previous_pointer_cache_and_keeps_source() {
    for failing_step in [1, 2] {
        let (_directory, home) = private_home();
        let previous = previous_home(&home);
        let old_tree = tree(&previous, false).unwrap();
        let (transaction, staged, original, installed) = prepared(&home);
        assert!(
            commit_install(
                &home,
                "warp",
                &original,
                &installed,
                &staged,
                transaction.path(),
                |step, _| {
                    if step == failing_step {
                        Err(io::Error::other("注入 rev3 迁移故障"))
                    } else {
                        Ok(())
                    }
                }
            )
            .is_err()
        );
        assert!(original.matches(&Scope::read(&config(&home), "warp@codex-warp")));
        assert!(is_previous_notification_cache(
            &cache_root(&home, "warp").join("0.4.0")
        ));
        assert_eq!(tree(&previous, false).unwrap(), old_tree);
    }
}

#[test]
fn modified_rev3_source_or_mixed_cache_is_not_a_migration_candidate() {
    let (_directory, home) = private_home();
    let previous = previous_home(&home);
    let path = previous.join("plugins/warp/scripts/on-prompt-submit.sh");
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, "用户自定义脚本".as_bytes()).unwrap();
    assert!(has_custom_source(&home));
    assert!(validate_existing(&home, &config(&home)).is_err());
    assert_eq!(fs::read(&path).unwrap(), "用户自定义脚本".as_bytes());
    fs::write(&path, bytes).unwrap();
    fs::write(
        cache_root(&home, "warp").join("0.4.0/scripts/warp-notify.sh"),
        include_bytes!("../../../../assets/bundled/cli-agent-plugins/codex/scripts/warp-notify.sh"),
    )
    .unwrap();
    assert!(notification_patch::preflight(&home, PatchKind::Codex).is_err());
    assert!(!is_previous_notification_cache(
        &cache_root(&home, "warp").join("0.4.0")
    ));
}

fn config(home: &Path) -> DocumentMut {
    read_config(home).unwrap().1
}

fn save(home: &Path, document: &DocumentMut) {
    fs::write(home.join("config.toml"), document.to_string()).unwrap();
}

fn owned_scope(home: &Path, key: &str) -> Scope {
    let mut document = DocumentMut::new();
    document["marketplaces"][MARKETPLACE]["source_type"] = toml_edit::value("local");
    document["marketplaces"][MARKETPLACE]["source"] =
        toml_edit::value(source_path(home).to_str().unwrap());
    document["plugins"][key]["enabled"] = toml_edit::value(true);
    let mut scope = Scope::read(&document, key);
    // 原生 marketplace add 生成普通 TOML 表，与纯索引构造的内联表不同。
    scope.marketplace = Item::Table(scope.marketplace.into_table().unwrap());
    scope
}

fn copy_plugin(home: &Path, name: &str, destination: &Path) {
    let prefix = format!("plugins/{name}/");
    for (path, _) in FILES {
        if let Some(relative) = path.strip_prefix(&prefix) {
            let target = destination.join("0.4.0").join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(source_path(home).join(path), target).unwrap();
        }
    }
}

fn prepared(home: &Path) -> (TempDir, PathBuf, Scope, Scope) {
    materialize(home).unwrap();
    let transactions = home.join("plugins/infinishell-transactions");
    fs::create_dir_all(&transactions).unwrap();
    let transaction = TempDir::new_in(transactions).unwrap();
    let staged = transaction.path().join("staged-warp");
    copy_plugin(home, "warp", &staged);
    let original = Scope::read(&config(home), "warp@codex-warp");
    let installed = owned_scope(home, "warp@codex-warp");
    (transaction, staged, original, installed)
}

#[test]
fn missing_target_is_distinct_from_concurrently_created_empty_table() {
    let absent = Scope::read(&DocumentMut::new(), "warp@codex-warp");
    let placeholder = "[marketplaces.codex-warp]\n"
        .parse::<DocumentMut>()
        .unwrap();
    let empty_marketplace = Scope::read(&placeholder, "warp@codex-warp");
    // toml_edit 对 None 和空表都输出空串，必须额外比较项目类型。
    assert_eq!(
        absent.marketplace.to_string(),
        empty_marketplace.marketplace.to_string()
    );
    assert!(!absent.matches(&empty_marketplace));
    let empty_enabled = "[plugins.'warp@codex-warp'.enabled]\n"
        .parse::<DocumentMut>()
        .unwrap();
    assert!(!absent.matches(&Scope::read(&empty_enabled, "warp@codex-warp")));
    assert!(absent.matches(&Scope::read(&DocumentMut::new(), "warp@codex-warp")));
}

#[test]
fn valid_toml_with_invalid_target_shape_returns_error_without_writes_or_panic() {
    for bytes in [
        "plugins = 'x'\n",
        "marketplaces = 'x'\n",
        "[plugins]\n'warp@codex-warp' = true\n",
        "plugins = { 'orchestration@codex-warp' = 7 }\n",
        "[marketplaces]\ncodex-warp = true\n",
        "[plugins.'warp@codex-warp']\nenabled = 'false'\n",
    ] {
        let (_directory, home) = private_home();
        let (transaction, staged, original, installed) = prepared(&home);
        fs::write(home.join("config.toml"), bytes).unwrap();
        assert!(read_config(&home).is_err(), "{bytes}");
        assert!(
            write_scoped(&home, "warp@codex-warp", &original, &installed).is_err(),
            "{bytes}"
        );
        assert!(
            commit_install(
                &home,
                "warp",
                &original,
                &installed,
                &staged,
                transaction.path(),
                |_, _| Ok(())
            )
            .is_err(),
            "{bytes}"
        );
        assert_eq!(fs::read_to_string(home.join("config.toml")).unwrap(), bytes);
        assert!(!cache_root(&home, "warp").exists());
    }
}

#[test]
fn valid_inline_user_tables_remain_supported_by_scoped_write() {
    let (_directory, home) = private_home();
    materialize(&home).unwrap();
    let initial = "plugins = { 'warp@codex-warp' = { enabled = true, user_note = '保留' } }\nmarketplaces = { other = { source_type = 'local', source = '/user' } }\n";
    fs::write(home.join("config.toml"), initial).unwrap();
    let before = Scope::read(&config(&home), "warp@codex-warp");
    let mut installed = owned_scope(&home, "warp@codex-warp");
    // 原生暂存的 marketplace 使用普通表，真实配置的父表仍可为内联表。
    installed.marketplace = Item::Table(installed.marketplace.into_table().unwrap());
    write_scoped(&home, "warp@codex-warp", &before, &installed).unwrap();
    assert!(installed.matches(&Scope::read(&config(&home), "warp@codex-warp")));
    let updated = config(&home);
    assert_eq!(
        updated["plugins"]["warp@codex-warp"]["user_note"].as_str(),
        Some("保留")
    );
    assert_eq!(
        updated["marketplaces"]["other"]["source"].as_str(),
        Some("/user")
    );
}

#[test]
fn full_source_keeps_fixed_upstream_files_and_only_five_reviewed_changes() {
    let (_directory, home) = private_home();
    materialize(&home).unwrap();
    assert_eq!(FILES.len(), 36);
    assert_eq!(
        tree(&source_path(&home), false).unwrap(),
        expected_tree("", false)
    );
    let changed = BUNDLE
        .files
        .iter()
        .filter(|(_, file)| file.sha256 != file.upstream_sha256)
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        changed,
        [
            "plugins/warp/hooks/hooks.json",
            "plugins/warp/scripts/build-payload.sh",
            "plugins/warp/scripts/on-prompt-submit.sh",
            "plugins/warp/scripts/on-stop.sh",
            "plugins/warp/scripts/warp-notify.sh"
        ]
    );
    assert!(
        BUNDLE
            .files
            .contains_key("plugins/orchestration/.codex-plugin/plugin.json")
    );
    assert!(BUNDLE.files.contains_key("LICENSE"));
    materialize(&home).unwrap();
    assert_eq!(
        fs::read_dir(source_parent(&home).parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn owned_metadata_alone_never_authorizes_modified_or_missing_source() {
    let (_directory, home) = private_home();
    materialize(&home).unwrap();
    let mut document = config(&home);
    owned_scope(&home, "warp@codex-warp").apply(&mut document, "warp@codex-warp");
    save(&home, &document);
    assert!(is_current(&home));
    fs::write(
        source_path(&home).join("plugins/orchestration/scripts/on-stop.sh"),
        "用户内容",
    )
    .unwrap();
    invalidate(&home);
    assert!(!is_current(&home));
    assert!(has_custom_source(&home));
    assert!(materialize(&home).is_err());
    assert_eq!(
        fs::read_to_string(source_path(&home).join("plugins/orchestration/scripts/on-stop.sh"))
            .unwrap(),
        "用户内容"
    );
}

#[test]
fn arbitrary_local_source_and_unknown_git_revision_are_rejected() {
    let (_directory, home) = private_home();
    let mut document = DocumentMut::new();
    document["marketplaces"][MARKETPLACE]["source_type"] = toml_edit::value("local");
    document["marketplaces"][MARKETPLACE]["source"] = toml_edit::value("/user/plugin");
    save(&home, &document);
    assert!(has_custom_source(&home));
    assert!(validate_existing(&home, &document).is_err());
    document["marketplaces"][MARKETPLACE]["source_type"] = toml_edit::value("git");
    document["marketplaces"][MARKETPLACE]["source"] =
        toml_edit::value("https://github.com/warpdotdev/codex-warp.git");
    document["marketplaces"][MARKETPLACE]["ref"] = toml_edit::value("user-branch");
    assert!(validate_existing(&home, &document).is_err());
}

#[test]
fn migration_keeps_disabled_orchestration_other_marketplace_and_hook_trust() {
    let (_directory, home) = private_home();
    let initial = "# 用户配置注释\napproval_policy = 'on-request'\n[plugins.'orchestration@codex-warp']\nenabled = false\n[marketplaces.other]\nsource_type = 'local'\nsource = '/用户来源'\n[hooks.state.reviewed]\ntrusted_hash = 'user-owned-trust'\n";
    fs::write(home.join("config.toml"), initial).unwrap();
    let (transaction, staged, original, installed) = prepared(&home);
    let orchestration = cache_root(&home, "orchestration");
    copy_plugin(&home, "orchestration", &orchestration);
    let old_orchestration = tree(&orchestration, false).unwrap();
    commit_install(
        &home,
        "warp",
        &original,
        &installed,
        &staged,
        transaction.path(),
        |_, _| Ok(()),
    )
    .unwrap();
    let current = config(&home);
    assert_eq!(
        current["plugins"]["orchestration@codex-warp"]["enabled"].as_bool(),
        Some(false)
    );
    assert_eq!(
        current["marketplaces"]["other"]["source"].as_str(),
        Some("/用户来源")
    );
    assert_eq!(
        current["hooks"]["state"]["reviewed"]["trusted_hash"].as_str(),
        Some("user-owned-trust")
    );
    assert!(current.to_string().starts_with("# 用户配置注释"));
    assert_eq!(tree(&orchestration, false).unwrap(), old_orchestration);
    assert!(notification_patch::full_tree_is_applied(
        &home,
        PatchKind::Codex
    ));
    assert!(is_current(&home));
}

#[test]
fn concurrent_disable_before_final_configuration_write_restores_cache_and_stays_disabled() {
    let (_directory, home) = private_home();
    let (transaction, staged, original, installed) = prepared(&home);
    let result = commit_install(
        &home,
        "warp",
        &original,
        &installed,
        &staged,
        transaction.path(),
        |step, home| {
            if step == 1 {
                let mut document = config(home);
                document["plugins"]["warp@codex-warp"]["enabled"] = toml_edit::value(false);
                save(home, &document);
            }
            Ok(())
        },
    );
    assert!(result.is_err());
    assert!(!cache_root(&home, "warp").exists());
    assert_eq!(
        config(&home)["plugins"]["warp@codex-warp"]["enabled"].as_bool(),
        Some(false)
    );
    assert!(marketplace(&config(&home)).is_none());
}

#[test]
fn failure_after_config_commit_restores_only_changed_keys_and_keeps_concurrent_unrelated_write() {
    let (_directory, home) = private_home();
    let (transaction, staged, original, installed) = prepared(&home);
    let result = commit_install(
        &home,
        "warp",
        &original,
        &installed,
        &staged,
        transaction.path(),
        |step, home| {
            if step == 2 {
                let mut document = config(home);
                document["model"] = toml_edit::value("concurrent-user-model");
                save(home, &document);
                return Err(io::Error::other("注入提交后失败"));
            }
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(
        config(&home)["model"].as_str(),
        Some("concurrent-user-model")
    );
    assert!(marketplace(&config(&home)).is_none());
    assert!(!cache_root(&home, "warp").exists());
}

#[test]
fn unknown_concurrent_cache_edit_is_kept_and_old_backup_remains_recoverable() {
    let (_directory, home) = private_home();
    let (transaction, staged, original, installed) = prepared(&home);
    let target = cache_root(&home, "warp");
    copy_plugin(&home, "warp", &target);
    let previous = tree(&target, false).unwrap();
    let result = commit_install(
        &home,
        "warp",
        &original,
        &installed,
        &staged,
        transaction.path(),
        |step, home| {
            if step == 2 {
                fs::write(
                    cache_root(home, "warp").join("0.4.0/scripts/on-stop.sh"),
                    "并发新内容",
                )?;
                return Err(io::Error::other("注入缓存并发修改"));
            }
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read_to_string(target.join("0.4.0/scripts/on-stop.sh")).unwrap(),
        "并发新内容"
    );
    assert_eq!(
        tree(&transaction.path().join("previous-cache"), false).unwrap(),
        previous
    );
}

#[cfg(unix)]
#[test]
fn source_mode_change_is_rejected_without_resetting_user_permissions() {
    use std::os::unix::fs::PermissionsExt as _;
    let (_directory, home) = private_home();
    materialize(&home).unwrap();
    let target = source_path(&home).join("plugins/orchestration/scripts/on-stop.sh");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(verify_owned(&home).is_err());
    assert!(materialize(&home).is_err());
    assert_eq!(
        fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn source_symlink_is_rejected_without_changing_referenced_file() {
    use std::os::unix::fs::symlink;
    let (_directory, home) = private_home();
    materialize(&home).unwrap();
    let external = home.join("external");
    fs::write(&external, "原始外部文件").unwrap();
    let target = source_path(&home).join("plugins/warp/scripts/on-stop.sh");
    fs::remove_file(&target).unwrap();
    symlink(&external, &target).unwrap();
    assert!(verify_owned(&home).is_err());
    assert!(materialize(&home).is_err());
    assert_eq!(fs::read_to_string(external).unwrap(), "原始外部文件");
}
