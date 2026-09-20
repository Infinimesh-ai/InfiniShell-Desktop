use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn interrupt_commit(home: &Path, step: usize) -> (TempDir, Scope, Scope) {
    let (transaction, staged, original, installed) = prepared(home);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        commit_install(
            home,
            "warp",
            &original,
            &installed,
            &staged,
            transaction.path(),
            |current, _| {
                assert_ne!(current, step, "模拟进程退出，不执行事务错误回滚");
                Ok(())
            },
        )
    }));
    assert!(outcome.is_err());
    (transaction, original, installed)
}

fn phase(transaction: &Path) -> String {
    serde_json::from_slice::<serde_json::Value>(&fs::read(transaction.join("state.json")).unwrap())
        .unwrap()["phase"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn every_interrupted_install_state_recovers_without_reinstalling_or_losing_old_cache() {
    // panic 只用于跳过当前栈的错误回滚；这组纯文件测试不宣称真实 SIGKILL。
    for step in [0, 3, 4, 1, 5, 2, 6] {
        let (_directory, home) = private_home();
        previous_home(&home);
        let original_cache = cache_snapshot(&cache_root(&home, "warp")).unwrap();
        let (transaction, _, installed) = interrupt_commit(&home, step);
        let mut document = config(&home);
        document["model"] = toml_edit::value("用户在中断后修改的模型");
        save(&home, &document);
        recover_transactions(&home).unwrap();
        assert_eq!(phase(transaction.path()), "verified");
        assert!(installed.matches(&Scope::read(&config(&home), "warp@codex-warp")));
        verify_installed_cache(&cache_root(&home, "warp"), "warp").unwrap();
        assert_eq!(
            cache_snapshot(&transaction.path().join("previous-cache")).unwrap(),
            original_cache
        );
        assert_eq!(
            config(&home)["model"].as_str(),
            Some("用户在中断后修改的模型")
        );
        assert_eq!(
            config(&home)["hooks"]["state"]["user"]["trusted_hash"].as_str(),
            Some("保持用户信任")
        );
        let bytes = fs::read(home.join("config.toml")).unwrap();
        let cache = cache_snapshot(&cache_root(&home, "warp")).unwrap();
        recover_transactions(&home).unwrap();
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), bytes);
        assert_eq!(cache_snapshot(&cache_root(&home, "warp")).unwrap(), cache);
    }
}

#[test]
fn interrupted_first_install_recovers_absent_scope_and_cache() {
    for step in [0, 3, 4, 1, 5, 2] {
        let (_directory, home) = private_home();
        fs::write(
            home.join("config.toml"),
            "# 保留注释\nmodel = 'user-model'\n",
        )
        .unwrap();
        let (transaction, _, installed) = interrupt_commit(&home, step);
        recover_transactions(&home).unwrap();
        assert_eq!(phase(transaction.path()), "verified");
        assert!(installed.matches(&Scope::read(&config(&home), "warp@codex-warp")));
        assert!(!transaction.path().join("previous-cache").exists());
        assert!(
            fs::read_to_string(home.join("config.toml"))
                .unwrap()
                .starts_with("# 保留注释\n")
        );
    }
}

#[test]
fn user_disable_after_interruption_is_not_overwritten() {
    let (_directory, home) = private_home();
    previous_home(&home);
    let (transaction, _, _) = interrupt_commit(&home, 1);
    let mut document = config(&home);
    document["plugins"]["warp@codex-warp"]["enabled"] = toml_edit::value(false);
    save(&home, &document);
    let config_before = fs::read(home.join("config.toml")).unwrap();
    let cache_before = cache_snapshot(&cache_root(&home, "warp")).unwrap();
    let backup_before = cache_snapshot(&transaction.path().join("previous-cache")).unwrap();
    assert!(recover_transactions(&home).is_err());
    assert_eq!(fs::read(home.join("config.toml")).unwrap(), config_before);
    assert_eq!(
        cache_snapshot(&cache_root(&home, "warp")).unwrap(),
        cache_before
    );
    assert_eq!(
        cache_snapshot(&transaction.path().join("previous-cache")).unwrap(),
        backup_before
    );
}

#[test]
fn interrupted_error_rollback_restores_only_the_verified_original_cache() {
    for cache_already_restored in [false, true] {
        let (_directory, home) = private_home();
        previous_home(&home);
        let original_cache = cache_snapshot(&cache_root(&home, "warp")).unwrap();
        let (transaction, original, _) = interrupt_commit(&home, 1);
        fs::remove_dir_all(cache_root(&home, "warp")).unwrap();
        if cache_already_restored {
            fs::rename(
                transaction.path().join("previous-cache"),
                cache_root(&home, "warp"),
            )
            .unwrap();
        }
        let before = fs::read(home.join("config.toml")).unwrap();
        recover_transactions(&home).unwrap();
        assert_eq!(phase(transaction.path()), "rolled_back");
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), before);
        assert!(original.matches(&Scope::read(&config(&home), "warp@codex-warp")));
        assert_eq!(
            cache_snapshot(&cache_root(&home, "warp")).unwrap(),
            original_cache
        );
        recover_transactions(&home).unwrap();
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), before);
    }
}

#[test]
fn orchestration_cache_recovers_with_its_own_enabled_scope() {
    let (_directory, home) = private_home();
    materialize(&home).unwrap();
    let transactions = home.join("plugins/infinishell-transactions");
    fs::create_dir_all(&transactions).unwrap();
    let transaction = TempDir::new_in(transactions).unwrap();
    let staged = transaction.path().join("staged-orchestration");
    copy_plugin(&home, "orchestration", &staged);
    let original = Scope::read(&config(&home), "orchestration@codex-warp");
    let installed = owned_scope(&home, "orchestration@codex-warp");
    assert!(
        catch_unwind(AssertUnwindSafe(|| commit_install(
            &home,
            "orchestration",
            &original,
            &installed,
            &staged,
            transaction.path(),
            |step, _| {
                assert_ne!(step, 4, "模拟缓存发布后中断");
                Ok(())
            }
        )))
        .is_err()
    );
    recover_transactions(&home).unwrap();
    assert_eq!(phase(transaction.path()), "verified");
    verify_installed_cache(&cache_root(&home, "orchestration"), "orchestration").unwrap();
    assert!(installed.matches(&Scope::read(&config(&home), "orchestration@codex-warp")));
    assert!(
        Scope::read(&config(&home), "warp@codex-warp")
            .enabled
            .is_none()
    );
}

#[test]
fn edited_new_cache_or_old_backup_is_preserved_for_review() {
    for relative in ["target", "backup"] {
        let (_directory, home) = private_home();
        previous_home(&home);
        let (transaction, _, _) = interrupt_commit(&home, 1);
        let root = if relative == "target" {
            cache_root(&home, "warp")
        } else {
            transaction.path().join("previous-cache")
        };
        let path = root.join("0.4.0/scripts/on-stop.sh");
        fs::write(&path, "用户修改的插件脚本").unwrap();
        let before = fs::read(home.join("config.toml")).unwrap();
        assert!(recover_transactions(&home).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "用户修改的插件脚本");
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), before);
        assert!(transaction.path().join("previous-cache").exists());
    }
}

#[test]
fn scope_journal_distinguishes_absence_from_empty_marketplace_table() {
    let absent = Scope::read(&DocumentMut::new(), "warp@codex-warp");
    let empty = Scope::read(
        &"[marketplaces.codex-warp]\n"
            .parse::<DocumentMut>()
            .unwrap(),
        "warp@codex-warp",
    );
    let restored_absent = journal_scope(
        &scope_document(&absent, "warp@codex-warp"),
        "warp@codex-warp",
    )
    .unwrap();
    let restored_empty = journal_scope(
        &scope_document(&empty, "warp@codex-warp"),
        "warp@codex-warp",
    )
    .unwrap();
    assert!(restored_absent.matches(&absent));
    assert!(restored_empty.matches(&empty));
    assert!(!restored_absent.matches(&restored_empty));
}

#[test]
fn unknown_journal_or_path_traversal_cannot_modify_cache_or_configuration() {
    for mutation in ["version", "staged_cache", "extra_field"] {
        let (_directory, home) = private_home();
        previous_home(&home);
        let (transaction, _, _) = interrupt_commit(&home, 0);
        let path = transaction.path().join("state.json");
        let mut journal: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        journal[mutation] = match mutation {
            "version" => json!(2),
            "staged_cache" => json!("../../outside"),
            "extra_field" => json!(true),
            unexpected => panic!("未知测试变更：{unexpected}"),
        };
        fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
        let before = fs::read(home.join("config.toml")).unwrap();
        let cache = cache_snapshot(&cache_root(&home, "warp")).unwrap();
        assert!(recover_transactions(&home).is_err());
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), before);
        assert_eq!(cache_snapshot(&cache_root(&home, "warp")).unwrap(), cache);
        assert!(path.exists());
    }
}

#[test]
fn legacy_completed_archive_is_retained_but_legacy_unfinished_state_is_not_guessed() {
    let (_directory, home) = private_home();
    let (transaction, _, _) = interrupt_commit(&home, 0);
    let path = transaction.path().join("state.json");
    let mut legacy = json!({"phase":"verified","plugin":"warp@codex-warp","source":source_path(&home),
        "original_cache":null,"installed_cache":{},"original_marketplace":"","original_enabled":""});
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    recover_transactions(&home).unwrap();
    assert!(path.exists());
    legacy["phase"] = json!("cache_written");
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(recover_transactions(&home).is_err());
    assert!(!cache_root(&home, "warp").exists());
}

#[test]
fn publication_lock_blocks_a_second_recovery_until_the_owner_releases_it() {
    let (_directory, home) = private_home();
    let lock = publication_lock(&home).unwrap();
    assert!(publication_lock(&home).is_err());
    drop(lock);
    assert!(publication_lock(&home).is_ok());
}

#[cfg(unix)]
#[test]
fn recovery_refuses_staged_cache_and_journal_symlinks_without_touching_their_targets() {
    let (_directory, home) = private_home();
    let (transaction, _, _) = interrupt_commit(&home, 0);
    let staged = transaction.path().join("staged-warp");
    let other = home.join("outside");
    fs::rename(&staged, &other).unwrap();
    std::os::unix::fs::symlink(&other, &staged).unwrap();
    let snapshot = cache_snapshot(&other).unwrap();
    assert!(recover_transactions(&home).is_err());
    assert_eq!(cache_snapshot(&other).unwrap(), snapshot);
    assert!(!cache_root(&home, "warp").exists());
    fs::remove_file(&staged).unwrap();
    fs::rename(&other, &staged).unwrap();
    let state = transaction.path().join("state.json");
    let saved_state = transaction.path().join("saved-state.json");
    fs::rename(&state, &saved_state).unwrap();
    let before = fs::read(&saved_state).unwrap();
    std::os::unix::fs::symlink(home.join("missing-state.json"), &state).unwrap();
    assert!(recover_transactions(&home).is_err());
    assert_eq!(fs::read(&saved_state).unwrap(), before);
    assert!(!home.join("missing-state.json").exists());
    assert!(!cache_root(&home, "warp").exists());
}

fn process_fixture_root() -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("INFINISHELL_CODEX_JOURNAL_CRASH_ROOT").expect("缺少合成根目录"),
    );
    assert_eq!(root.canonicalize().unwrap(), root);
    assert!(
        root.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("infinishell-codex-journal-crash-")
    );
    plain_path(&root).unwrap();
    assert_eq!(
        fs::read(relative_file(&root, ".fixture-owner").unwrap()).unwrap(),
        b"Codex journal SIGKILL fixture v1\n"
    );
    root
}

#[test]
#[ignore = "仅由外部强杀驱动执行，不运行 Codex CLI"]
fn interrupted_install_process_fixture() {
    let root = process_fixture_root();
    let home = root.join("home");
    assert!(!home.exists());
    fs::create_dir(&home).unwrap();
    let _lock = publication_lock(&home).unwrap();
    previous_home(&home);
    fs::write(
        root.join("original-cache.json"),
        serde_json::to_vec(&cache_snapshot(&cache_root(&home, "warp")).unwrap()).unwrap(),
    )
    .unwrap();
    let stop = std::env::var("INFINISHELL_CODEX_JOURNAL_CRASH_STEP")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    assert!([0, 3, 4, 1, 5, 2, 6].contains(&stop));
    let (transaction, staged, original, installed) = prepared(&home);
    let transaction = transaction.keep();
    fs::write(
        root.join("transaction-name"),
        transaction.file_name().unwrap().to_str().unwrap(),
    )
    .unwrap();
    commit_install(
        &home,
        "warp",
        &original,
        &installed,
        &staged,
        &transaction,
        |step, _| {
            if step == stop {
                fs::write(root.join("checkpoint"), step.to_string())?;
                // 外部驱动必须在这个窗口发送 SIGKILL；超时失败不能冒充强杀成功。
                std::thread::sleep(Duration::from_secs(10));
                panic!("未收到外部强杀");
            }
            Ok(())
        },
    )
    .unwrap();
    panic!("未到达指定强杀窗口");
}

#[test]
#[ignore = "仅在外部驱动已确认 SIGKILL 后恢复同一合成目录"]
fn recover_interrupted_install_process_fixture() {
    let root = process_fixture_root();
    assert!(root.join("checkpoint").is_file());
    let home = root.join("home");
    let _lock = publication_lock(&home).unwrap();
    recover_transactions(&home).unwrap();
    let transaction = home
        .join("plugins/infinishell-transactions")
        .join(fs::read_to_string(root.join("transaction-name")).unwrap());
    assert_eq!(phase(&transaction), "verified");
    verify_installed_cache(&cache_root(&home, "warp"), "warp").unwrap();
    assert!(owned_entry(&config(&home), &home));
    assert_eq!(
        config(&home)["plugins"]["warp@codex-warp"]["enabled"].as_bool(),
        Some(true)
    );
    assert_eq!(
        config(&home)["hooks"]["state"]["user"]["trusted_hash"].as_str(),
        Some("保持用户信任")
    );
    let old: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("original-cache.json")).unwrap()).unwrap();
    assert_eq!(
        cache_snapshot(&transaction.join("previous-cache")).unwrap(),
        old
    );
    let before = fs::read(home.join("config.toml")).unwrap();
    recover_transactions(&home).unwrap();
    assert_eq!(fs::read(home.join("config.toml")).unwrap(), before);
}
