use std::fs;

use serde_json::json;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use sha2::{Digest as _, Sha256};

use super::*;

fn install_fixture(root: &std::path::Path) -> std::path::PathBuf {
    let source = write_bundle(&root.join("source")).unwrap();
    let installed = root.join("installed-plugins/source-one");
    fs::create_dir_all(&installed).unwrap();
    for (relative, _) in BUNDLED_FILES {
        let path = installed.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(source.join(relative), path).unwrap();
    }
    let recording: Value = serde_json::from_str(include_str!(
        "../../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-installed-integrity-macos.json"
    ))
    .unwrap();
    let native_registry = &recording["native_registry_after_disable"];
    let mut entry = native_registry["repos"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    // 仅将真实原生记录的路径和键重定位到测试目录，保留其他原生字段。
    entry["kind"]["source_path"] = json!(source);
    entry["path"] = json!(installed);
    fs::write(
        root.join("installed-plugins/registry.json"),
        serde_json::to_string(&json!({
            "version": native_registry["version"],
            "repos": {"source-one": entry}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("config.toml"),
        "[ui]\nscreen_mode='minimal'\n[plugins]\nenabled=['infinishell-grok']\n",
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

#[test]
fn damaged_installed_hook_is_not_reported_as_ready() {
    let directory = tempfile::tempdir().unwrap();
    let installed = install_fixture(directory.path());
    fs::write(installed.join("hooks/notify.cjs"), "").unwrap();

    assert!(installed_plugin(directory.path()).is_err());
}

#[test]
fn future_plugin_version_is_not_accepted_as_a_verified_tree() {
    let directory = tempfile::tempdir().unwrap();
    let installed = install_fixture(directory.path());
    let path = installed.join(".grok-plugin/plugin.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["version"] = json!("9.0.0");
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let registry_path = directory.path().join("installed-plugins/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    registry["repos"]["source-one"]["plugins"][PLUGIN_NAME]["version"] = json!("9.0.0");
    fs::write(registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();

    assert!(registered_plugin(directory.path()).unwrap().is_some());
    assert!(installed_plugin(directory.path()).is_err());
}

#[test]
fn same_version_repair_restores_missing_and_damaged_files_without_registry_or_config_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let installed = install_fixture(root);
    let plugin = registered_plugin(root).unwrap().unwrap();
    let config = fs::read(root.join("config.toml")).unwrap();
    let registry = fs::read(root.join("installed-plugins/registry.json")).unwrap();
    fs::remove_file(installed.join("hooks/hooks.json")).unwrap();
    fs::write(installed.join("hooks/notify.cjs"), "").unwrap();

    repair_current_plugin(root, &plugin, &root.join("source"), |_, _| Ok(())).unwrap();

    assert!(installed_plugin(root).unwrap().is_some());
    assert_eq!(fs::read(root.join("config.toml")).unwrap(), config);
    assert_eq!(
        fs::read(root.join("installed-plugins/registry.json")).unwrap(),
        registry
    );
    // 已经完整的同版本目录不应再写任何文件。
    repair_current_plugin(root, &plugin, &root.join("source"), |_, _| {
        panic!("完整目录不应触发文件替换")
    })
    .unwrap();
}

#[test]
fn failed_repair_restores_the_original_damage_and_missing_files() {
    for missing in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let installed = install_fixture(root);
        let plugin = registered_plugin(root).unwrap().unwrap();
        let first = installed.join("hooks/hooks.json");
        if missing {
            fs::remove_file(&first).unwrap();
        } else {
            fs::write(&first, "原来的损坏内容").unwrap();
        }
        fs::write(installed.join("hooks/notify.cjs"), "第二个损坏文件").unwrap();
        let original = plugin_tree(&installed, true).unwrap();
        let config = fs::read(root.join("config.toml")).unwrap();
        let registry = fs::read(root.join("installed-plugins/registry.json")).unwrap();

        let result = repair_current_plugin(root, &plugin, &root.join("source"), |index, _| {
            if index == 2 {
                assert_eq!(fs::read_to_string(&first).unwrap(), BUNDLED_FILES[1].1);
                return Err(io::Error::other("模拟第二次写入失败"));
            }
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(plugin_tree(&installed, true).unwrap(), original);
        assert_eq!(fs::read(root.join("config.toml")).unwrap(), config);
        assert_eq!(
            fs::read(root.join("installed-plugins/registry.json")).unwrap(),
            registry
        );
    }
}

#[test]
fn repair_keeps_initial_and_concurrent_native_disable() {
    for disable_before_start in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let installed = install_fixture(root);
        let plugin = registered_plugin(root).unwrap().unwrap();
        fs::write(installed.join("hooks/hooks.json"), "损坏甲").unwrap();
        fs::write(installed.join("hooks/notify.cjs"), "损坏乙").unwrap();
        let original = plugin_tree(&installed, true).unwrap();
        let disabled =
            "[ui]\nscreen_mode='minimal'\n[plugins]\nenabled=[]\ndisabled=['infinishell-grok']\n";
        if disable_before_start {
            fs::write(root.join("config.toml"), disabled).unwrap();
        }

        let result = repair_current_plugin(root, &plugin, &root.join("source"), |index, _| {
            assert!(!disable_before_start);
            if index == 2 {
                fs::write(root.join("config.toml"), disabled)?;
            }
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(plugin_tree(&installed, true).unwrap(), original);
        assert_eq!(
            fs::read_to_string(root.join("config.toml")).unwrap(),
            disabled
        );
        assert!(plugin_disabled(root).unwrap());
    }
}

#[test]
fn rollback_preserves_concurrent_edits_to_a_repaired_file() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let installed = install_fixture(root);
    let plugin = registered_plugin(root).unwrap().unwrap();
    let first = installed.join("hooks/hooks.json");
    fs::write(&first, "损坏甲").unwrap();
    fs::write(installed.join("hooks/notify.cjs"), "损坏乙").unwrap();

    let error = repair_current_plugin(root, &plugin, &root.join("source"), |index, _| {
        if index == 2 {
            fs::write(&first, "用户并发修改")?;
            return Err(io::Error::other("模拟写入失败"));
        }
        Ok(())
    })
    .unwrap_err();

    assert!(error.to_string().contains("未恢复"));
    assert_eq!(fs::read_to_string(first).unwrap(), "用户并发修改");
    assert_eq!(
        fs::read_to_string(installed.join("hooks/notify.cjs")).unwrap(),
        "损坏乙"
    );
}

#[test]
fn unknown_files_directories_and_modified_sources_are_not_overwritten() {
    for mutation in [
        "cache_file",
        "cache_directory",
        "source_file",
        "source_content",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let installed = install_fixture(root);
        let plugin = registered_plugin(root).unwrap().unwrap();
        fs::write(installed.join("hooks/notify.cjs"), "原来的内容").unwrap();
        match mutation {
            "cache_file" => fs::write(installed.join("user.cjs"), "用户内容").unwrap(),
            "cache_directory" => fs::create_dir(installed.join("empty-user-directory")).unwrap(),
            "source_file" => fs::write(plugin.source.join("user.cjs"), "用户内容").unwrap(),
            "source_content" => {
                fs::write(plugin.source.join("hooks/notify.cjs"), "用户内容").unwrap()
            }
            _ => unreachable!(),
        }

        assert!(
            repair_current_plugin(root, &plugin, &root.join("source"), |_, _| {
                panic!("未知内容不能进入写入阶段")
            })
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(installed.join("hooks/notify.cjs")).unwrap(),
            "原来的内容"
        );
        if mutation.starts_with("source") {
            assert!(write_bundle(&root.join("source")).is_err());
        } else {
            assert!(installed_plugin(root).is_err());
        }
    }
}

#[test]
fn changed_registry_and_external_cache_paths_cannot_reuse_old_registration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let installed = install_fixture(root);
    let plugin = registered_plugin(root).unwrap().unwrap();
    fs::write(installed.join("hooks/notify.cjs"), "原来的内容").unwrap();
    let registry_path = root.join("installed-plugins/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    let external = write_bundle(&root.join("external-source")).unwrap();
    registry["repos"]["source-one"]["kind"]["source_path"] = json!(external);
    fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();

    assert!(
        repair_current_plugin(root, &plugin, &root.join("source"), |_, _| {
            panic!("旧注册记录不能进入写入阶段")
        })
        .is_err()
    );
    let current = registered_plugin(root).unwrap().unwrap();
    assert!(validate_owned_source(&current, &root.join("source")).is_err());

    registry["repos"]["source-one"]["path"] = json!(external);
    fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
    assert!(registered_plugin(root).is_err());
    assert_eq!(
        fs::read_to_string(installed.join("hooks/notify.cjs")).unwrap(),
        "原来的内容"
    );
    assert!(validate_expected_tree(&external, PLUGIN_VERSION).is_ok());
}

#[test]
fn source_changed_during_repair_causes_rollback_without_overwriting_the_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let installed = install_fixture(root);
    let plugin = registered_plugin(root).unwrap().unwrap();
    fs::write(installed.join("hooks/notify.cjs"), "原来的损坏内容").unwrap();
    let original = plugin_tree(&installed, true).unwrap();

    let result = repair_current_plugin(root, &plugin, &root.join("source"), |_, _| {
        fs::write(plugin.source.join("hooks/notify.cjs"), "用户修改了来源")
    });

    assert!(result.is_err());
    assert_eq!(plugin_tree(&installed, true).unwrap(), original);
    assert_eq!(
        fs::read_to_string(plugin.source.join("hooks/notify.cjs")).unwrap(),
        "用户修改了来源"
    );
}

#[test]
fn incomplete_directory_after_interruption_remains_invalid_until_explicit_repair() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let installed = install_fixture(root);
    let plugin = registered_plugin(root).unwrap().unwrap();
    fs::write(installed.join("hooks/hooks.json"), "损坏甲").unwrap();
    fs::write(installed.join("hooks/notify.cjs"), "损坏乙").unwrap();
    // 此处仅构造一份中途写完第一个文件的磁盘状态，不冒充真实进程崩溃。
    fs::copy(
        plugin.source.join("hooks/hooks.json"),
        installed.join("hooks/hooks.json"),
    )
    .unwrap();
    assert!(installed_plugin(root).is_err());

    repair_current_plugin(root, &plugin, &root.join("source"), |_, _| Ok(())).unwrap();

    assert!(installed_plugin(root).unwrap().is_some());
}

#[cfg(unix)]
#[test]
fn symlinks_and_hardlinks_cannot_redirect_plugin_repairs() {
    use std::os::unix::fs::symlink;

    for hardlink in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let installed = install_fixture(root);
        let plugin = registered_plugin(root).unwrap().unwrap();
        let target = root.join("user-file");
        fs::write(&target, "用户内容").unwrap();
        let hook = installed.join("hooks/notify.cjs");
        fs::remove_file(&hook).unwrap();
        if hardlink {
            fs::hard_link(&target, &hook).unwrap();
        } else {
            symlink(&target, &hook).unwrap();
        }

        assert!(installed_plugin(root).is_err());
        assert!(
            repair_current_plugin(root, &plugin, &root.join("source"), |_, _| {
                panic!("链接不能进入写入阶段")
            })
            .is_err()
        );
        assert_eq!(fs::read_to_string(target).unwrap(), "用户内容");
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
#[ignore = "仅供独立无凭据进程运行，必须显式提供私有目录及固定 Grok/Node 摘要"]
async fn live_grok_production_installer_repairs_and_preserves_disable() {
    tokio::time::timeout(
        Duration::from_secs(180),
        run_live_grok_production_installer(),
    )
    .await
    .expect("真实 Grok 插件验收超时");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn run_live_grok_production_installer() {
    let test_name = "terminal::cli_agent_sessions::plugin_manager::grok::tests::live_grok_production_installer_repairs_and_preserves_disable";
    let arguments = env::args().collect::<Vec<_>>();
    for argument in ["--ignored", "--exact", "--test-threads=1", test_name] {
        assert!(arguments.iter().any(|actual| actual == argument));
    }
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_PLUGIN_LIVE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-plugin-live")).unwrap(),
        "isolated unauthenticated Grok plugin verification\n"
    );
    // 只核对运行器已隔离的进程环境，不修改并行测试可见的全局环境。
    for name in [
        "HOME",
        "GROK_HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "TMPDIR",
    ] {
        let path = PathBuf::from(env::var_os(name).unwrap())
            .canonicalize()
            .unwrap();
        assert!(path.starts_with(&root), "{name} 必须位于私有目录");
    }
    assert!(env::vars_os().all(|(key, _)| {
        let key = key.to_string_lossy().to_ascii_uppercase();
        !["TOKEN", "API_KEY", "AUTH", "SECRET", "CREDENTIAL"]
            .iter()
            .any(|part| key.contains(*part))
    }));
    assert!(env::var_os("GROK_CONFIG_PATH").is_none());
    assert!(env::var_os("GROK_CONFIG").is_none());
    assert_eq!(
        env::current_dir().unwrap().canonicalize().unwrap(),
        root.join("work")
    );
    assert_eq!(
        dirs::home_dir().unwrap().canonicalize().unwrap(),
        root.join("home")
    );
    let data_root = dirs::data_local_dir().unwrap().canonicalize().unwrap();
    assert!(data_root.starts_with(root.join("home")));
    let source_root = bundled_source_root().unwrap();
    assert!(source_root.starts_with(&data_root) && !source_root.exists());
    let grok_home = grok_home_dir().unwrap().canonicalize().unwrap();
    assert_eq!(grok_home, root.join("home/.grok"));
    assert_eq!(fs::read_dir(&grok_home).unwrap().count(), 0);
    let artifact = root.join("grok-production-installer.json");
    assert!(!artifact.exists());
    let manager = GrokPluginManager::new(Some(env::var("PATH").unwrap()));
    let mut evidence = json!({
        "passed": false, "credentials_provided": false, "model_input_submitted": false,
        "test_binary_sha256": format!("{:x}", Sha256::digest(fs::read(env::current_exe().unwrap()).unwrap())),
        "steps": [],
    });
    for (program, variable) in [
        ("grok", "INFINISHELL_GROK_PLUGIN_LIVE_GROK_SHA256"),
        ("node", "INFINISHELL_GROK_PLUGIN_LIVE_NODE_SHA256"),
    ] {
        let executable = manager.executable(program).unwrap().canonicalize().unwrap();
        let actual = format!("{:x}", Sha256::digest(fs::read(executable).unwrap()));
        assert_eq!(
            actual,
            env::var(variable).expect("必须指定固定可执行文件摘要")
        );
        evidence[format!("{program}_sha256")] = json!(actual);
    }
    fs::write(&artifact, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    let mut native_log = String::new();
    let grok = manager.verify_runtime(&mut native_log).await.unwrap();
    let node = manager.executable("node").unwrap();
    let output = manager
        .run(
            &node,
            &[OsStr::new("--version")],
            Duration::from_secs(3),
            &mut native_log,
        )
        .await
        .unwrap();
    let node_version = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        node_version.trim(),
        env::var("INFINISHELL_GROK_PLUGIN_LIVE_NODE_VERSION").unwrap()
    );
    evidence["grok_version"] = json!(TESTED_GROK_VERSION);
    evidence["node_version"] = json!(node_version.trim());
    assert!(!manager.is_installed());

    manager.install().await.unwrap();
    assert!(manager.is_installed() && !manager.needs_update());
    let plugin = installed_plugin(&grok_home).unwrap().unwrap();
    let registry_path = grok_home.join("installed-plugins/registry.json");
    let config_path = grok_home.join("config.toml");
    let registry = fs::read(&registry_path).unwrap();
    let config = fs::read(&config_path).unwrap();
    evidence["steps"]
        .as_array_mut()
        .unwrap()
        .push(json!({"step": "production_install", "passed": true}));
    fs::write(&artifact, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();

    fs::write(
        plugin.path.join("hooks/notify.cjs"),
        "// 本次私有损坏夹具\n",
    )
    .unwrap();
    assert!(!manager.is_installed() && manager.needs_update());
    manager.update().await.unwrap();
    assert!(manager.is_installed() && !manager.needs_update());
    assert_eq!(fs::read(&registry_path).unwrap(), registry);
    assert_eq!(fs::read(&config_path).unwrap(), config);
    evidence["steps"].as_array_mut().unwrap().push(json!({"step": "production_same_version_update", "passed": true, "config_and_registry_unchanged": true}));
    fs::write(&artifact, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();

    manager
        .run(
            &grok,
            &[
                OsStr::new("plugin"),
                OsStr::new("disable"),
                OsStr::new(PLUGIN_NAME),
            ],
            Duration::from_secs(5),
            &mut native_log,
        )
        .await
        .unwrap();
    fs::write(
        plugin.path.join("hooks/notify.cjs"),
        "// 禁用后的私有损坏夹具\n",
    )
    .unwrap();
    let disabled_config = fs::read(&config_path).unwrap();
    let damaged = plugin_tree(&plugin.path, false).unwrap();
    assert!(manager.is_disabled());
    assert!(manager.update().await.is_err());
    assert_eq!(fs::read(&config_path).unwrap(), disabled_config);
    assert_eq!(fs::read(&registry_path).unwrap(), registry);
    assert_eq!(plugin_tree(&plugin.path, false).unwrap(), damaged);
    evidence["steps"].as_array_mut().unwrap().push(json!({"step": "production_disabled_update_rejected", "passed": true, "config_and_files_unchanged": true}));
    fs::write(&artifact, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();

    // 此段调用生产文件事务的故障注入点，不把它写成 apply 或 SIGKILL 验收。
    manager
        .run(
            &grok,
            &[
                OsStr::new("plugin"),
                OsStr::new("enable"),
                OsStr::new(PLUGIN_NAME),
            ],
            Duration::from_secs(5),
            &mut native_log,
        )
        .await
        .unwrap();
    fs::write(plugin.path.join("hooks/hooks.json"), "损坏甲").unwrap();
    let before_failure = plugin_tree(&plugin.path, false).unwrap();
    let enabled_config = fs::read(&config_path).unwrap();
    let enabled_registry = fs::read(&registry_path).unwrap();
    let result = repair_current_plugin(&grok_home, &plugin, &source_root, |index, _| {
        if index == 2 {
            assert_eq!(
                fs::read_to_string(plugin.path.join("hooks/hooks.json")).unwrap(),
                BUNDLED_FILES[1].1
            );
            return Err(io::Error::other("真实原生缓存的受控文件事务故障"));
        }
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(plugin_tree(&plugin.path, false).unwrap(), before_failure);
    assert_eq!(fs::read(&config_path).unwrap(), enabled_config);
    assert_eq!(fs::read(&registry_path).unwrap(), enabled_registry);
    evidence["steps"].as_array_mut().unwrap().push(json!({"step": "file_transaction_failure_rollback", "passed": true, "production_apply_call": false, "fault_after_first_replacement": true}));
    manager.update().await.unwrap();
    assert!(manager.is_installed() && !manager.needs_update());
    assert_eq!(fs::read(&config_path).unwrap(), enabled_config);
    assert_eq!(fs::read(&registry_path).unwrap(), enabled_registry);
    evidence["steps"]
        .as_array_mut()
        .unwrap()
        .push(json!({"step": "production_update_after_rollback", "passed": true}));
    evidence["passed"] = json!(true);
    fs::write(&artifact, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
}
