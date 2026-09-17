//! 真实生产安装器的无模型迁移；仅由隔离运行器显式启动，Windows 产品门控保持原样。

use std::env;
use std::fs::File;
use std::io::Write;

use serde_json::Value;

use super::*;

const MARKER: &str = "isolated Codex source migration without credentials or model commands\n";

struct Evidence {
    file: File,
    root: PathBuf,
}

impl Evidence {
    fn record(&mut self, value: Value) {
        let encoded = serde_json::to_string(&value).unwrap();
        let encoded = encoded.replace(&*self.root.to_string_lossy(), "<probe-root>");
        writeln!(self.file, "{encoded}").unwrap();
        self.file.flush().unwrap();
    }
}

fn fixture(home: &Path) -> PathBuf {
    fs::create_dir_all(home).unwrap();
    let previous = previous_home(home);
    let mut document = config(home);
    document["cli_auth_credentials_store"] = toml_edit::value("file");
    document["approval_policy"] = toml_edit::value("on-request");
    document["sandbox_mode"] = toml_edit::value("read-only");
    document["model"] = toml_edit::value("user-owned-model-fixture");
    document["marketplaces"]["user-owned"]["source_type"] = toml_edit::value("local");
    document["marketplaces"]["user-owned"]["source"] =
        toml_edit::value(home.join("other-source").to_str().unwrap());
    fs::write(
        home.join("config.toml"),
        format!("# 保留用户配置注释\n{document}"),
    )
    .unwrap();
    let prefix = "plugins/orchestration/";
    for name in PREVIOUS_BUNDLE.files.keys() {
        if let Some(relative) = name.strip_prefix(prefix) {
            let target = cache_root(home, "orchestration")
                .join("0.4.0")
                .join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(previous.join(name), target).unwrap();
        }
    }
    assert!(is_previous_notification_cache(
        &cache_root(home, "warp").join("0.4.0")
    ));
    validate_existing(home, &config(home)).unwrap();
    previous
}

fn preserved_config(document: &DocumentMut) -> Vec<String> {
    [
        document["hooks"].to_string(),
        document["plugins"]["orchestration@codex-warp"].to_string(),
        document["marketplaces"]["user-owned"].to_string(),
        document["cli_auth_credentials_store"].to_string(),
        document["approval_policy"].to_string(),
        document["sandbox_mode"].to_string(),
        document["model"].to_string(),
    ]
    .into()
}

async fn real_install(
    home: &Path,
    runtime: &VerifiedRuntime,
    evidence: &mut Evidence,
    phase: &str,
) {
    let mut log = String::new();
    let outcome = install(home, "warp", runtime, &mut log).await;
    evidence.record(json!({"event":"install_observed", "phase":phase,
        "succeeded":outcome.is_ok(), "native_install_log":log,
        "error":outcome.as_ref().err().map(|error| &error.message)}));
    assert!(
        outcome.is_ok(),
        "生产 Rust 安装失败，原始输出已保存至私有验收证据"
    );
    assert!(log.contains("$ codex plugin marketplace add "));
    assert!(log.contains("$ codex plugin add warp@codex-warp"));
}

async fn exercise(root: &Path, runtime: &VerifiedRuntime, evidence: &mut Evidence) {
    let home = root.join("cases/迁移 空格 ' $() &");
    let old_source = fixture(&home);
    let old_source_tree = cache_snapshot(&old_source).unwrap();
    let old_cache = cache_snapshot(&cache_root(&home, "warp")).unwrap();
    let old_orchestration = cache_snapshot(&cache_root(&home, "orchestration")).unwrap();
    let old_metadata = fs::read(old_source.parent().unwrap().join("SOURCE_METADATA.json")).unwrap();
    let original_settings = preserved_config(&config(&home));
    assert!(!is_current(&home));
    evidence.record(
        json!({"event":"rev3_fixture_verified", "source_files":old_source_tree.len(),
        "cache_files":old_cache.len(), "source_metadata_sha256":digest(&old_metadata),
        "source_tree_sha256":digest(&serde_json::to_vec(&old_source_tree).unwrap()),
        "cache_tree_sha256":digest(&serde_json::to_vec(&old_cache).unwrap()),
        "initial_installation":"controlled_exact_rev3_fixture",
        "trust_fixture_is_not_native_authorization":true}),
    );

    real_install(&home, runtime, evidence, "rev3_to_rev4").await;
    verify_owned(&home).unwrap();
    assert!(is_current(&home));
    assert_eq!(
        tree(&cache_root(&home, "warp").join("0.4.0"), false).unwrap(),
        expected_tree("plugins/warp/", false)
    );
    assert_eq!(cache_snapshot(&old_source).unwrap(), old_source_tree);
    assert_eq!(
        fs::read(old_source.parent().unwrap().join("SOURCE_METADATA.json")).unwrap(),
        old_metadata
    );
    assert_eq!(
        cache_snapshot(&cache_root(&home, "orchestration")).unwrap(),
        old_orchestration
    );
    assert_eq!(preserved_config(&config(&home)), original_settings);
    assert!(
        fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .starts_with("# 保留用户配置注释\n")
    );
    assert!(owned_entry(&config(&home), &home));
    let transactions = fs::read_dir(home.join("plugins/infinishell-transactions"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(transactions.len(), 1);
    let transaction = &transactions[0];
    assert_eq!(
        cache_snapshot(&transaction.join("previous-cache")).unwrap(),
        old_cache
    );
    let state_bytes = fs::read(transaction.join("state.json")).unwrap();
    let state: Value = serde_json::from_slice(&state_bytes).unwrap();
    assert_eq!(state["phase"], "verified");
    assert_eq!(state["plugin"], "warp@codex-warp");
    evidence.record(
        json!({"event":"migration_verified", "rev4_source_and_cache_verified":true,
        "old_source_and_metadata_preserved":true, "old_cache_and_transaction_preserved":true,
        "user_configuration_and_trust_preserved":true, "disabled_orchestration_preserved":true,
        "source_metadata_sha256":digest(METADATA.as_bytes()), "transaction_phase":state["phase"]}),
    );

    real_install(&home, runtime, evidence, "repeat_rev4_install").await;
    assert!(is_current(&home));
    assert_eq!(preserved_config(&config(&home)), original_settings);
    assert_eq!(cache_snapshot(&old_source).unwrap(), old_source_tree);
    assert_eq!(
        cache_snapshot(&transaction.join("previous-cache")).unwrap(),
        old_cache
    );
    assert_eq!(
        fs::read(transaction.join("state.json")).unwrap(),
        state_bytes
    );
    assert_eq!(
        fs::read_dir(home.join("plugins/infinishell-transactions"))
            .unwrap()
            .count(),
        1
    );
    evidence.record(
        json!({"event":"repeat_install_verified", "previous_recovery_material_preserved":true}),
    );

    for case in [
        "modified_rev3_source",
        "mixed_cache",
        "unknown_source",
        "disabled_target",
    ] {
        let home = root.join("cases").join(case);
        let previous = fixture(&home);
        match case {
            "modified_rev3_source" => fs::write(
                previous.join("plugins/warp/scripts/on-stop.sh"),
                "# 用户自定义脚本\n",
            )
            .unwrap(),
            "mixed_cache" => {
                let contents = FILES
                    .iter()
                    .find(|(name, _)| *name == "plugins/warp/scripts/warp-notify.sh")
                    .unwrap()
                    .1;
                fs::write(
                    cache_root(&home, "warp").join("0.4.0/scripts/warp-notify.sh"),
                    contents,
                )
                .unwrap();
            }
            "unknown_source" | "disabled_target" => {
                let mut document = config(&home);
                if case == "unknown_source" {
                    document["marketplaces"][MARKETPLACE]["source"] =
                        toml_edit::value(home.join("user-source").to_str().unwrap());
                } else {
                    document["plugins"]["warp@codex-warp"]["enabled"] = toml_edit::value(false);
                }
                save(&home, &document);
            }
            unexpected => panic!("未知验收场景：{unexpected}"),
        }
        let before = cache_snapshot(&home).unwrap();
        let mut log = String::new();
        let outcome = install(&home, "warp", runtime, &mut log).await;
        let after = cache_snapshot(&home).unwrap();
        evidence.record(json!({"event":"rejection_observed", "case":case,
            "rejected":outcome.is_err(), "bytes_and_modes_unchanged":before == after,
            "before_tree_sha256":digest(&serde_json::to_vec(&before).unwrap()),
            "after_tree_sha256":digest(&serde_json::to_vec(&after).unwrap()),
            "native_command_invoked":!log.is_empty(), "rev4_source_created":source_parent(&home).exists(),
            "error":outcome.as_ref().err().map(|error| &error.message)}));
        assert!(outcome.is_err(), "未知或禁用状态必须拒绝安装：{case}");
        assert_eq!(before, after, "拒绝路径不得修改任何文件字节或模式：{case}");
        assert!(log.is_empty());
        assert!(!source_parent(&home).exists());
        assert!(!home.join("plugins/infinishell-transactions").exists());
    }
    evidence.record(
        json!({"event":"rust_source_migration_passed", "passed":true,
        "model_commands_sent":0, "hook_authorization_performed":false,
        "native_hook_execution_verified":false, "windows_product_gate_opened":false,
        "gui_verified":false, "rejected_cases":4}),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要 run_codex_source_live.py、生产平台门控允许的固定 Codex 0.147.0 与私有 HOME；不使用凭据或模型"]
async fn real_codex_source_migration_without_model() {
    let root = PathBuf::from(
        env::var_os("INFINISHELL_CODEX_SOURCE_ROOT").expect("必须通过专用隔离运行器启动"),
    )
    .canonicalize()
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-codex-source-probe")).unwrap(),
        MARKER
    );
    for (key, suffix) in [
        ("HOME", "home"),
        ("USERPROFILE", "home"),
        ("CODEX_HOME", "codex"),
    ] {
        assert_eq!(
            PathBuf::from(env::var_os(key).unwrap())
                .canonicalize()
                .unwrap(),
            root.join(suffix)
        );
    }
    assert!(!root.join("codex/auth.json").exists());
    let path = env::var("PATH").unwrap();
    let artifact = PathBuf::from(env::var_os("INFINISHELL_CODEX_SOURCE_ARTIFACT").unwrap());
    assert!(artifact.is_absolute() && !artifact.starts_with(&root));
    let mut evidence = Evidence {
        file: fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(artifact)
            .unwrap(),
        root: root.clone(),
    };
    evidence.record(
        json!({"event":"rust_source_migration_started", "scope":"production_rust_installer",
        "platform":env::consts::OS, "credentials_provided":false, "model_commands_sent":0}),
    );
    let mut log = String::new();
    let runtime =
        VerifiedRuntime::probe(PatchKind::Codex, &root.join("codex"), Some(&path), &mut log).await;
    evidence.record(
        json!({"event":"production_runtime_probe", "succeeded":runtime.is_ok(), "native_log":log,
        "error":runtime.as_ref().err().map(|error| &error.message)}),
    );
    let runtime = runtime.unwrap_or_else(|_| panic!("生产运行时版本与平台门控未通过"));
    exercise(&root, &runtime, &mut evidence).await;
}
