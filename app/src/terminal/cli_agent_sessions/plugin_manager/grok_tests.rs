use std::fs;

use base64::Engine as _;
use serde_json::json;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
use sha2::{Digest as _, Sha256};
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
use warpui::r#async::FutureExt as _;

use super::*;
use crate::terminal::cli_agent_sessions::event::{CLI_AGENT_NOTIFICATION_SENTINEL, parse_event};
use crate::terminal::cli_agent_sessions::event_cursor::{EventCursor, EventDisposition};

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
    // 重定位真实注册形状并使用当前随附版本；原始证据保持不变。
    entry["kind"]["source_path"] = json!(source);
    entry["path"] = json!(installed);
    entry["plugins"][PLUGIN_NAME]["version"] = json!(PLUGIN_VERSION);
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
fn private_installer_source_scope_is_explicit_without_changing_native_defaults() {
    let mut manager = GrokPluginManager::new(None);
    assert_eq!(
        manager.source_root().unwrap(),
        bundled_source_root().unwrap()
    );
    let directory = tempfile::tempdir().unwrap();
    let source = directory
        .path()
        .join("隔离 appdata/InfiniShell/cli-agent-plugins/grok");
    manager.test_source_root = Some(source.clone());
    assert_eq!(manager.source_root().unwrap(), source);
    assert!(!source.exists());
    manager.test_source_root = Some(PathBuf::from("relative-source"));
    assert!(manager.source_root().is_err());
}

#[test]
fn native_registry_reads_verified_installed_files() {
    let directory = tempfile::tempdir().unwrap();
    let installed = install_fixture(directory.path());
    let plugin = installed_plugin(directory.path()).unwrap().unwrap();
    assert_eq!(plugin.path, installed);
    assert_eq!(plugin.version, PLUGIN_VERSION);
}

#[test]
fn startup_bridge_preserves_user_hooks_and_config_across_repeat_install() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    install_fixture(root);
    fs::create_dir(root.join("hooks")).unwrap();
    fs::write(root.join("hooks/user.json"), "用户 hook 原件").unwrap();
    let before = fs::read(root.join("config.toml")).unwrap();
    let executable = &root.join("固定 cli/grok");
    let node = &root.join("运行 时/node");
    install_startup_bridge(root, executable, node).unwrap();
    assert!(startup_bridge_current(root, executable, node));
    let first = fs::read(root.join("hooks/infinishell-1.0.41.json")).unwrap();
    install_startup_bridge(root, executable, node).unwrap();
    assert_eq!(
        fs::read(root.join("hooks/infinishell-1.0.41.json")).unwrap(),
        first
    );
    assert_eq!(fs::read(root.join("config.toml")).unwrap(), before);
    assert_eq!(
        fs::read_to_string(root.join("hooks/user.json")).unwrap(),
        "用户 hook 原件"
    );
}

#[test]
fn windows_startup_bridge_keeps_shell_metacharacters_inside_encoded_arguments() {
    let paths = [
        r"C:\固定 Node\node.exe",
        r"C:\space ' & %PATH%\bridge.cjs",
        r"C:\grok.exe",
        r"C:\插件$(`x)\",
    ];
    let command = windows_startup_bridge_command(&paths).unwrap();
    let encoded = command
        .strip_prefix("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
        .unwrap();
    assert!(
        encoded
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'+' | b'/' | b'='))
    );
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    let source = String::from_utf16(
        &bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    for path in paths {
        assert!(source.contains(&format!("'{}'", path.replace('\'', "''"))));
    }
    assert!(source.contains("$info.RedirectStandardInput = $false"));
    assert!(source.contains("$info.CreateNoWindow = $false"));
    let template =
        include_str!("../../../../../script/cli-agent-parity/codex_windows_hook_command.ps1");
    assert!(
        source.starts_with(
            template
                .split_once("# BEGIN_NOTIFICATION_LAUNCH")
                .unwrap()
                .0
        )
    );
    assert!(windows_startup_bridge_command(&["one", "two", "three"]).is_err());
    assert!(windows_startup_bridge_command(&["one", "two", "three", "bad\0path"]).is_err());
    assert!(windows_startup_bridge_command(&[&"x".repeat(4000), "two", "three", "four"]).is_err());
}

fn legacy_startup_bridge_bytes() -> Vec<u8> {
    // 反向还原唯一已发布补桥；完整摘要确保测试没有自行定义更宽的“旧版”。
    let text = String::from_utf8(STARTUP_BRIDGE_SCRIPT.to_vec())
        .unwrap()
        .replace(
            "全局源只补固定桌面版本的加载缺口",
            "全局源只补固定 Mac 版本的加载缺口",
        )
        .replace(
            "![\"darwin\", \"linux\", \"win32\"].includes(process.platform)",
            "process.platform !== \"darwin\"",
        )
        .replace(
            "maxBuffer: 1024 * 1024, windowsHide: true,",
            "maxBuffer: 1024 * 1024,",
        );
    assert_eq!(
        format!("{:x}", Sha256::digest(text.as_bytes())),
        LEGACY_STARTUP_BRIDGE_SHA256
    );
    text.into_bytes()
}

#[test]
fn startup_bridge_upgrades_only_the_known_old_script_with_its_exact_json_pair() {
    for mutation in ["none", "script", "json", "backup", "disabled"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        install_fixture(root);
        let executable = root.join("固定 cli/grok");
        let node = root.join("运行 时/node");
        let files = startup_bridge_files(root, &executable, &node).unwrap();
        fs::create_dir(root.join("hooks")).unwrap();
        let old = legacy_startup_bridge_bytes();
        fs::write(&files[0].0, &old).unwrap();
        fs::write(&files[1].0, &files[1].1).unwrap();
        match mutation {
            "none" => {}
            "script" => fs::write(&files[0].0, "用户修改").unwrap(),
            "json" => fs::write(&files[1].0, "用户修改").unwrap(),
            "backup" => fs::write(files[0].0.with_extension("cjs.previous"), "用户修改").unwrap(),
            "disabled" => fs::write(
                root.join("config.toml"),
                "[plugins]\ndisabled=['infinishell-grok']\n",
            )
            .unwrap(),
            other => panic!("未知夹具 {other}"),
        }
        let before = files
            .iter()
            .map(|(path, _)| fs::read(path).unwrap())
            .collect::<Vec<_>>();
        let result = install_startup_bridge(root, &executable, &node);
        if mutation == "none" {
            result.unwrap();
            assert!(startup_bridge_current(root, &executable, &node));
            assert_eq!(
                fs::read(files[0].0.with_extension("cjs.previous")).unwrap(),
                old
            );
            install_startup_bridge(root, &executable, &node).unwrap();
        } else {
            assert!(result.is_err(), "{mutation}");
            assert_eq!(
                files
                    .iter()
                    .map(|(path, _)| fs::read(path).unwrap())
                    .collect::<Vec<_>>(),
                before
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn startup_bridge_does_not_upgrade_a_symlinked_legacy_script() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    install_fixture(root);
    let files = startup_bridge_files(root, &root.join("grok"), &root.join("node")).unwrap();
    fs::create_dir(root.join("hooks")).unwrap();
    let target = root.join("user-script");
    let old = legacy_startup_bridge_bytes();
    fs::write(&target, &old).unwrap();
    std::os::unix::fs::symlink(&target, &files[0].0).unwrap();
    fs::write(&files[1].0, &files[1].1).unwrap();
    assert!(install_startup_bridge(root, &root.join("grok"), &root.join("node")).is_err());
    assert_eq!(fs::read(target).unwrap(), old);
}

#[test]
fn startup_bridge_refuses_conflicting_json_before_publishing_script() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    install_fixture(root);
    fs::create_dir(root.join("hooks")).unwrap();
    let conflict = root.join("hooks/infinishell-1.0.41.json");
    fs::write(&conflict, "用户同名文件").unwrap();
    assert!(install_startup_bridge(root, &root.join("grok"), &root.join("node")).is_err());
    assert_eq!(fs::read_to_string(conflict).unwrap(), "用户同名文件");
    assert!(!root.join("hooks/infinishell-1.0.41.cjs").exists());
}

#[test]
fn startup_bridge_refuses_disabled_plugin_without_creating_hooks() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    install_fixture(root);
    fs::write(
        root.join("config.toml"),
        "[plugins]\ndisabled=['infinishell-grok']\n",
    )
    .unwrap();
    assert!(install_startup_bridge(root, &root.join("grok"), &root.join("node")).is_err());
    assert!(!root.join("hooks").exists());
    assert!(plugin_disabled(root).unwrap());
}

#[cfg(unix)]
#[test]
fn startup_bridge_does_not_follow_hook_directory_symlink() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    install_fixture(root);
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("hooks")).unwrap();
    assert!(install_startup_bridge(root, &root.join("grok"), &root.join("node")).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn startup_bridge_and_reloaded_plugin_same_event_notifies_only_once() {
    let mut cursor = EventCursor::default();
    let prompt = parse_event(Some(CLI_AGENT_NOTIFICATION_SENTINEL),
        r#"{"v":1,"agent":"grok","event":"prompt_submit","session_id":"native","prompt_id":"turn","event_id":"same-prompt"}"#).unwrap();
    let stop = parse_event(Some(CLI_AGENT_NOTIFICATION_SENTINEL),
        r#"{"v":1,"agent":"grok","event":"stop","session_id":"native","prompt_id":"turn","event_id":"same-stop"}"#).unwrap();
    assert_eq!(cursor.accept(&prompt), EventDisposition::Accept);
    assert_eq!(cursor.accept(&prompt), EventDisposition::Drop);
    assert_eq!(cursor.accept(&stop), EventDisposition::Accept);
    assert_eq!(cursor.accept(&stop), EventDisposition::Drop);
}

const LEGACY_HOOKS: &str =
    include_str!("../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.2-hooks.json");
const LEGACY_012_README: &str =
    include_str!("../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.2-readme.md");
const LEGACY_012_NOTIFY: &str =
    include_str!("../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.2-notify.cjs");

const LEGACY_README: &str =
    include_str!("../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.0-readme.md");
const LEGACY_NOTIFY: &str =
    include_str!("../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.0-notify.cjs");

fn write_legacy_bundle(root: &Path) -> PathBuf {
    let source = root.join("0.1.0");
    fs::create_dir_all(&source).unwrap();
    for (name, _) in BUNDLED_FILES {
        let destination = source.join(name);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let contents = match *name {
            ".grok-plugin/plugin.json" => {
                let mut manifest: Value = serde_json::from_str(LEGACY_012_MANIFEST).unwrap();
                manifest["version"] = json!("0.1.0");
                serde_json::to_string(&manifest).unwrap()
            }
            "README.md" => LEGACY_README.to_owned(),
            "hooks/notify.cjs" => LEGACY_NOTIFY.to_owned(),
            "hooks/hooks.json" => LEGACY_HOOKS.to_owned(),
            name => panic!("旧版配方未定义文件：{name}"),
        };
        fs::write(destination, contents).unwrap();
    }
    source
}

#[test]
fn known_010_source_and_backup_remain_valid_without_rewriting_the_old_recipe() {
    let directory = tempfile::tempdir().unwrap();
    let old = write_legacy_bundle(directory.path());
    let before_tree = plugin_tree(&old, false).unwrap();
    let before = fs::read(old.join("README.md")).unwrap();
    assert_eq!(before, LEGACY_README.as_bytes());
    assert_eq!(
        fs::read(old.join("hooks/notify.cjs")).unwrap(),
        LEGACY_NOTIFY.as_bytes()
    );
    validate_expected_tree(&old, "0.1.0").unwrap();
    let backup = backup_plugin(&old, directory.path()).unwrap();
    validate_expected_tree(&backup, "0.1.0").unwrap();
    let current = write_bundle(directory.path()).unwrap();
    validate_expected_tree(&current, PLUGIN_VERSION).unwrap();
    assert_ne!(current, old);
    assert_eq!(fs::read(old.join("README.md")).unwrap(), before);
    assert_eq!(fs::read(backup.join("README.md")).unwrap(), before);
    assert_eq!(plugin_tree(&old, false).unwrap(), before_tree);
    assert_eq!(plugin_tree(&backup, false).unwrap(), before_tree);
    assert_ne!(fs::read(current.join("README.md")).unwrap(), before);
}

#[test]
fn legacy_notify_is_byte_exact_and_rejected_for_other_recipe_versions() {
    assert_eq!(
        format!("{:x}", Sha256::digest(LEGACY_NOTIFY.as_bytes())),
        LEGACY_NOTIFY_SHA256
    );
    let directory = tempfile::tempdir().unwrap();
    let old = write_legacy_bundle(directory.path());
    validate_expected_tree(&old, "0.1.0").unwrap();
    assert_eq!(
        fs::read(old.join("hooks/notify.cjs")).unwrap(),
        LEGACY_NOTIFY.as_bytes()
    );
    let current = write_bundle(directory.path()).unwrap();
    for version in [PLUGIN_VERSION, "0.0.9"] {
        let mut manifest: Value = serde_json::from_str(BUNDLED_FILES[0].1).unwrap();
        manifest["version"] = json!(version);
        fs::write(
            current.join(".grok-plugin/plugin.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(current.join("hooks/notify.cjs"), LEGACY_NOTIFY).unwrap();
        let before = plugin_tree(&current, false).unwrap();
        assert!(validate_expected_tree(&current, version).is_err());
        assert_eq!(plugin_tree(&current, false).unwrap(), before);
    }
}

#[test]
fn modified_legacy_readme_is_rejected_and_not_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let old = write_legacy_bundle(directory.path());
    fs::write(old.join("README.md"), format!("{LEGACY_README}\n用户说明")).unwrap();
    let before = fs::read(old.join("README.md")).unwrap();
    assert!(validate_expected_tree(&old, "0.1.0").is_err());
    write_bundle(directory.path()).unwrap();
    assert_eq!(fs::read(old.join("README.md")).unwrap(), before);
}

#[test]
fn legacy_readme_cannot_prove_the_current_plugin_or_an_unverified_older_version() {
    let directory = tempfile::tempdir().unwrap();
    let old = write_legacy_bundle(directory.path());
    for version in [PLUGIN_VERSION, "0.0.9"] {
        let mut manifest: Value = BUNDLED_FILES
            .iter()
            .find(|(name, _)| *name == ".grok-plugin/plugin.json")
            .map(|(_, contents)| serde_json::from_str(contents).unwrap())
            .unwrap();
        manifest["version"] = json!(version);
        fs::write(old.join(".grok-plugin/plugin.json"), manifest.to_string()).unwrap();
        assert!(validate_expected_tree(&old, version).is_err());
    }
}

#[test]
fn valid_legacy_readme_does_not_allow_modified_hooks_or_extra_source_files() {
    for changed in ["hooks/notify.cjs", "hooks/hooks.json", "user.txt"] {
        let directory = tempfile::tempdir().unwrap();
        let old = write_legacy_bundle(directory.path());
        fs::write(old.join(changed), "用户内容").unwrap();
        assert!(validate_expected_tree(&old, "0.1.0").is_err());
        assert_eq!(fs::read_to_string(old.join(changed)).unwrap(), "用户内容");
    }
}

const LEGACY_011_NOTIFY: &str =
    include_str!("../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.1-notify.cjs");
const LEGACY_011_README: &str =
    include_str!("../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.1-readme.md");
const LEGACY_011_ORIGINAL_README: &str = include_str!(
    "../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.1-original-readme.md"
);

fn write_011_bundle(root: &Path, readme: &str) -> PathBuf {
    let source = root.join("0.1.1");
    for (name, _) in BUNDLED_FILES {
        let destination = source.join(name);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let contents = match *name {
            ".grok-plugin/plugin.json" => {
                let mut manifest: Value = serde_json::from_str(LEGACY_012_MANIFEST).unwrap();
                manifest["version"] = json!("0.1.1");
                manifest.to_string()
            }
            "README.md" => readme.to_owned(),
            "hooks/notify.cjs" => LEGACY_011_NOTIFY.to_owned(),
            "hooks/hooks.json" => LEGACY_HOOKS.to_owned(),
            name => panic!("旧版配方未定义文件：{name}"),
        };
        fs::write(destination, contents).unwrap();
    }
    source
}

#[test]
fn known_011_sources_and_backups_keep_original_bytes() {
    for readme in [LEGACY_011_ORIGINAL_README, LEGACY_011_README] {
        let directory = tempfile::tempdir().unwrap();
        let source = write_011_bundle(directory.path(), readme);
        let before = validate_expected_tree(&source, "0.1.1").unwrap();
        let backup = backup_plugin(&source, directory.path()).unwrap();
        assert_eq!(validate_expected_tree(&backup, "0.1.1").unwrap(), before);
        write_bundle(directory.path()).unwrap();
        assert_eq!(validate_expected_tree(&source, "0.1.1").unwrap(), before);
        assert!(validate_expected_tree(&source, PLUGIN_VERSION).is_err());
    }
}

#[test]
fn modified_011_notification_or_readme_cannot_be_upgraded_as_owned_source() {
    for name in ["hooks/notify.cjs", "README.md"] {
        let directory = tempfile::tempdir().unwrap();
        let source = write_011_bundle(directory.path(), LEGACY_011_README);
        fs::write(source.join(name), "用户修改").unwrap();
        assert!(validate_expected_tree(&source, "0.1.1").is_err());
        assert_eq!(fs::read_to_string(source.join(name)).unwrap(), "用户修改");
    }
}

fn write_013_bundle(root: &Path) -> PathBuf {
    let source = root.join("0.1.3");
    for (name, contents) in [
        (
            ".grok-plugin/plugin.json",
            include_str!(
                "../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.3-plugin.json"
            ),
        ),
        ("hooks/hooks.json", BUNDLED_FILES[1].1),
        (
            "hooks/notify.cjs",
            include_str!(
                "../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.3-notify.cjs"
            ),
        ),
        (
            "README.md",
            include_str!(
                "../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.3-README.md"
            ),
        ),
    ] {
        let destination = source.join(name);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, contents).unwrap();
    }
    source
}

#[test]
fn known_013_recipe_is_preserved_and_modified_files_cannot_acquire_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let source = write_013_bundle(directory.path());
    let original = validate_expected_tree(&source, "0.1.3").unwrap();
    let backup = backup_plugin(&source, directory.path()).unwrap();
    write_bundle(directory.path()).unwrap();
    assert_eq!(validate_expected_tree(&backup, "0.1.3").unwrap(), original);
    assert!(validate_expected_tree(&source, PLUGIN_VERSION).is_err());
    for (name, _) in LEGACY_013_SHA256 {
        let path = source.join(name);
        let bytes = fs::read(&path).unwrap();
        let mut changed = bytes.clone();
        changed.push(b' ');
        fs::write(&path, &changed).unwrap();
        assert!(validate_expected_tree(&source, "0.1.3").is_err());
        assert_eq!(fs::read(&path).unwrap(), changed);
        fs::write(&path, bytes).unwrap();
    }
    assert_eq!(validate_expected_tree(&source, "0.1.3").unwrap(), original);
}

#[test]
fn bridge_migration_recovers_registry_commit_before_json_and_rejects_modified_bindings() {
    for mutation in [
        "none",
        "json_digest",
        "script_digest",
        "target_version",
        "source",
        "json",
        "script",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let old_cache = install_fixture(root);
        let source_root = root.join("source");
        let old_source = write_013_bundle(&source_root);
        for (name, _) in BUNDLED_FILES {
            fs::copy(old_source.join(name), old_cache.join(name)).unwrap();
        }
        let registry_path = root.join("installed-plugins/registry.json");
        let mut registry: Value =
            serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
        registry["repos"]["source-one"]["kind"]["source_path"] = json!(old_source);
        registry["repos"]["source-one"]["plugins"][PLUGIN_NAME]["version"] = json!("0.1.3");
        fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
        let executable = root.join("grok");
        let node = root.join("node");
        let files = startup_bridge_files(root, &executable, &node).unwrap();
        fs::create_dir(root.join("hooks")).unwrap();
        fs::write(&files[0].0, legacy_startup_bridge_bytes()).unwrap();
        fs::write(&files[1].0, &files[1].1).unwrap();
        let old = installed_plugin(root).unwrap().unwrap();
        persist_startup_bridge_migration(root, &executable, &node, &old, &source_root).unwrap();
        let record_path = startup_bridge_migration_path(root);
        let original_record = fs::read(&record_path).unwrap();
        let new_source = write_bundle(&source_root).unwrap();
        let new_cache = root.join("installed-plugins/source-two");
        fs::create_dir(&new_cache).unwrap();
        for (name, _) in BUNDLED_FILES {
            let destination = new_cache.join(name);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(new_source.join(name), destination).unwrap();
        }
        let mut entry = registry["repos"]
            .as_object_mut()
            .unwrap()
            .remove("source-one")
            .unwrap();
        entry["path"] = json!(new_cache);
        entry["kind"]["source_path"] = json!(new_source);
        entry["plugins"][PLUGIN_NAME]["version"] = json!(PLUGIN_VERSION);
        registry["repos"]["source-two"] = entry;
        fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
        fs::remove_dir_all(old_cache).unwrap();
        // 模拟原生注册提交后立即崩溃；重进程没有前次内存中的旧 JSON。
        let mut record: Value = serde_json::from_slice(&original_record).unwrap();
        match mutation {
            "none" => {}
            "json_digest" => record["old_json_sha256"] = json!("0".repeat(64)),
            "script_digest" => record["old_script_sha256"] = json!("0".repeat(64)),
            "target_version" => record["target_version"] = json!("0.1.5"),
            "source" => fs::write(old_source.join("hooks/notify.cjs"), "用户修改").unwrap(),
            "json" => fs::write(&files[1].0, "用户修改").unwrap(),
            "script" => fs::write(&files[0].0, "用户修改").unwrap(),
            other => panic!("未知夹具 {other}"),
        }
        fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
        let before = [
            fs::read(&files[0].0).unwrap(),
            fs::read(&files[1].0).unwrap(),
        ];
        let resumed = pending_startup_bridge_migration(root, &executable, &node, &source_root)
            .and_then(|pending| {
                let (_, previous_json) = pending.ok_or_else(invalid_tree)?;
                migrate_startup_bridge(root, &executable, &node, &previous_json)
            });
        if mutation == "none" {
            resumed.unwrap();
            assert!(startup_bridge_current(root, &executable, &node));
            assert!(record_path.is_file());
            // JSON 和脚本已完成、标记尚未删除的中断同样可以幂等续修。
            let (_, previous_json) =
                pending_startup_bridge_migration(root, &executable, &node, &source_root)
                    .unwrap()
                    .unwrap();
            migrate_startup_bridge(root, &executable, &node, &previous_json).unwrap();
        } else {
            assert!(resumed.is_err(), "{mutation}");
            assert_eq!(
                [
                    fs::read(&files[0].0).unwrap(),
                    fs::read(&files[1].0).unwrap()
                ],
                before
            );
        }
    }
}

#[test]
fn startup_bridge_migration_binds_the_prior_verified_cache_and_preserves_user_edits() {
    for changed_json in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let installed = install_fixture(root);
        let executable = root.join("grok");
        let node = root.join("node");
        let old_files = startup_bridge_files(root, &executable, &node).unwrap();
        fs::create_dir(root.join("hooks")).unwrap();
        fs::write(&old_files[0].0, legacy_startup_bridge_bytes()).unwrap();
        fs::write(&old_files[1].0, &old_files[1].1).unwrap();
        let previous = capture_startup_bridge(root, &executable, &node)
            .unwrap()
            .unwrap();
        let registry_path = root.join("installed-plugins/registry.json");
        let mut registry: Value =
            serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
        let next = root.join("installed-plugins/source-two");
        fs::rename(installed, &next).unwrap();
        let mut entry = registry["repos"]
            .as_object_mut()
            .unwrap()
            .remove("source-one")
            .unwrap();
        entry["path"] = json!(next);
        registry["repos"]["source-two"] = entry;
        fs::write(registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
        if changed_json {
            fs::write(&old_files[1].0, "用户修改").unwrap();
        }
        let result = migrate_startup_bridge(root, &executable, &node, &previous);
        if changed_json {
            assert!(result.is_err());
            assert_eq!(fs::read(&old_files[1].0).unwrap(), "用户修改".as_bytes());
            assert_eq!(
                fs::read(&old_files[0].0).unwrap(),
                legacy_startup_bridge_bytes()
            );
        } else {
            result.unwrap();
            assert!(startup_bridge_current(root, &executable, &node));
            let digest = format!("{:x}", Sha256::digest(&previous));
            assert_eq!(
                fs::read(
                    old_files[1]
                        .0
                        .with_extension(format!("json.{digest}.previous"))
                )
                .unwrap(),
                previous
            );
            // 模拟 JSON 已写、脚本未写即中断；不依赖旧 registry 搜索也可继续完成。
            fs::write(&old_files[0].0, legacy_startup_bridge_bytes()).unwrap();
            install_startup_bridge(root, &executable, &node).unwrap();
            assert!(startup_bridge_current(root, &executable, &node));
        }
    }
}

fn write_012_bundle(root: &Path) -> PathBuf {
    let source = root.join("0.1.2");
    for (name, contents) in [
        (".grok-plugin/plugin.json", LEGACY_012_MANIFEST),
        ("hooks/hooks.json", LEGACY_HOOKS),
        ("hooks/notify.cjs", LEGACY_012_NOTIFY),
        ("README.md", LEGACY_012_README),
    ] {
        let destination = source.join(name);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, contents).unwrap();
    }
    source
}

#[test]
fn known_012_source_and_backup_preserve_all_four_original_files() {
    let directory = tempfile::tempdir().unwrap();
    let source = write_012_bundle(directory.path());
    let original = validate_expected_tree(&source, "0.1.2").unwrap();
    for (name, digest) in LEGACY_012_SHA256 {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(&original.get(*name).unwrap().contents)
            ),
            *digest
        );
    }
    let backup = backup_plugin(&source, directory.path()).unwrap();
    write_bundle(directory.path()).unwrap();
    assert_eq!(validate_expected_tree(&source, "0.1.2").unwrap(), original);
    assert_eq!(validate_expected_tree(&backup, "0.1.2").unwrap(), original);
    assert!(validate_expected_tree(&source, PLUGIN_VERSION).is_err());
}

#[test]
fn any_modified_012_recipe_file_is_rejected_without_overwriting_user_changes() {
    for (name, _) in LEGACY_012_SHA256 {
        let directory = tempfile::tempdir().unwrap();
        let source = write_012_bundle(directory.path());
        let path = source.join(name);
        let mut changed = fs::read(&path).unwrap();
        changed.push(b' ');
        fs::write(&path, &changed).unwrap();
        assert!(validate_expected_tree(&source, "0.1.2").is_err());
        assert_eq!(fs::read(&path).unwrap(), changed);
    }
}

#[test]
fn new_hooks_and_other_old_versions_cannot_form_a_012_owned_recipe() {
    let directory = tempfile::tempdir().unwrap();
    let source = write_012_bundle(directory.path());
    assert!(validate_expected_tree(&source, "0.1.1").is_err());
    assert!(validate_expected_tree(&source, "0.0.9").is_err());
    fs::write(source.join("hooks/hooks.json"), BUNDLED_FILES[1].1).unwrap();
    assert!(validate_expected_tree(&source, "0.1.2").is_err());
}

#[cfg(not(target_family = "wasm"))]
mod migration_async_tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Fault {
        None,
        DisableAfterUninstall,
        EditSourceAfterUninstall,
        EditCacheAfterInstall,
        EditRegistryAfterInstall,
        FailBeforeInstall,
        FailPartialInstall,
        FailCompleteInstall,
        FailRecovery,
    }

    struct MockMutations {
        home: PathBuf,
        current_source: PathBuf,
        legacy_source: PathBuf,
        template: Value,
        fault: Fault,
        calls: Mutex<Vec<&'static str>>,
    }

    fn legacy_fixture(home: &Path) -> (InstalledPlugin, PathBuf, PathBuf) {
        let cache = install_fixture(home);
        let source_root = home.join("source");
        let legacy = write_legacy_bundle(&source_root);
        for (name, _) in BUNDLED_FILES {
            fs::copy(legacy.join(name), cache.join(name)).unwrap();
        }
        let registry_path = home.join("installed-plugins/registry.json");
        let mut registry: Value =
            serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
        registry["repos"]["source-one"]["kind"]["source_path"] = json!(legacy);
        registry["repos"]["source-one"]["plugins"][PLUGIN_NAME]["version"] = json!("0.1.0");
        fs::write(registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
        let previous = installed_plugin(home).unwrap().unwrap();
        let current = write_bundle(&source_root).unwrap();
        (previous, source_root, current)
    }

    impl MockMutations {
        fn new(home: &Path, previous: &InstalledPlugin, source: &Path, fault: Fault) -> Self {
            let registry: Value = serde_json::from_slice(
                &fs::read(home.join("installed-plugins/registry.json")).unwrap(),
            )
            .unwrap();
            Self {
                home: home.to_owned(),
                current_source: source.to_owned(),
                legacy_source: previous.source.clone(),
                template: registry["repos"]["source-one"].clone(),
                fault,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn write_config(&self, enabled: bool, disabled: bool) {
            let mut config = config_without_plugin(&read_config(&self.home).unwrap()).unwrap();
            if enabled || disabled {
                let plugins = config
                    .as_table_mut()
                    .unwrap()
                    .entry("plugins".to_owned())
                    .or_insert_with(|| toml::Value::Table(Default::default()))
                    .as_table_mut()
                    .unwrap();
                if enabled {
                    plugins
                        .entry("enabled".to_owned())
                        .or_insert_with(|| toml::Value::Array(Vec::new()))
                        .as_array_mut()
                        .unwrap()
                        .push(toml::Value::String(PLUGIN_NAME.to_owned()));
                }
                if disabled {
                    plugins
                        .entry("disabled".to_owned())
                        .or_insert_with(|| toml::Value::Array(Vec::new()))
                        .as_array_mut()
                        .unwrap()
                        .push(toml::Value::String(PLUGIN_NAME.to_owned()));
                }
            }
            fs::write(
                self.home.join("config.toml"),
                toml::to_string(&config).unwrap(),
            )
            .unwrap();
        }
    }

    #[async_trait]
    impl PluginMutationRunner for MockMutations {
        async fn mutate(
            &self,
            mutation: PluginMutation<'_>,
            log: &mut String,
        ) -> Result<(), PluginInstallError> {
            let label = match mutation {
                PluginMutation::Uninstall => "uninstall",
                PluginMutation::Install(source) => {
                    if source == self.current_source.as_path() {
                        "install_current"
                    } else {
                        "install_recovery"
                    }
                }
            };
            self.calls.lock().unwrap().push(label);
            // 只模拟异步命令边界与已知四文件注册，不派生进程、不修改全局环境。
            tokio::task::yield_now().await;
            if label == "install_current"
                && matches!(self.fault, Fault::FailBeforeInstall | Fault::FailRecovery)
            {
                log.push_str("受控安装命令失败\n");
                return Err(operation_error(log));
            }
            let registry_path = self.home.join("installed-plugins/registry.json");
            let mut registry: Value =
                serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
            let repos = registry["repos"].as_object_mut().unwrap();
            match mutation {
                PluginMutation::Uninstall => {
                    if let Some(plugin) = registered_plugin(&self.home).unwrap() {
                        fs::remove_dir_all(plugin.path).unwrap();
                    }
                    repos.retain(|_, repo| {
                        !repo["plugins"]
                            .as_object()
                            .unwrap()
                            .contains_key(PLUGIN_NAME)
                    });
                    self.write_config(false, false);
                }
                PluginMutation::Install(source) => {
                    let key = if source == self.current_source.as_path() {
                        "mock-current"
                    } else {
                        "mock-recovery"
                    };
                    let cache = self.home.join("installed-plugins").join(key);
                    fs::create_dir_all(&cache).unwrap();
                    for (name, _) in BUNDLED_FILES {
                        let target = cache.join(name);
                        fs::create_dir_all(target.parent().unwrap()).unwrap();
                        fs::copy(source.join(name), target).unwrap();
                    }
                    let manifest: Value = serde_json::from_slice(
                        &fs::read(source.join(".grok-plugin/plugin.json")).unwrap(),
                    )
                    .unwrap();
                    let mut entry = self.template.clone();
                    entry["kind"]["source_path"] = json!(source);
                    entry["path"] = json!(cache);
                    entry["plugins"][PLUGIN_NAME]["version"] = manifest["version"].clone();
                    repos.insert(key.to_owned(), entry);
                    self.write_config(true, false);
                    if label == "install_current" && self.fault == Fault::FailPartialInstall {
                        fs::remove_file(cache.join("README.md")).unwrap();
                    }
                    if label == "install_current" && self.fault == Fault::EditCacheAfterInstall {
                        fs::write(cache.join("README.md"), "用户新说明").unwrap();
                    }
                }
            }
            if label == "install_current" && self.fault == Fault::EditRegistryAfterInstall {
                registry["user_revision"] = json!("用户新注册意图");
            }
            fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
            if label == "uninstall" && self.fault == Fault::DisableAfterUninstall {
                self.write_config(false, true);
            }
            if label == "uninstall" && self.fault == Fault::EditSourceAfterUninstall {
                fs::write(self.legacy_source.join("README.md"), "用户新来源说明").unwrap();
            }
            tokio::task::yield_now().await;
            let failed_current = label == "install_current"
                && matches!(
                    self.fault,
                    Fault::FailPartialInstall
                        | Fault::FailCompleteInstall
                        | Fault::EditCacheAfterInstall
                        | Fault::EditRegistryAfterInstall
                );
            if failed_current || (label == "install_recovery" && self.fault == Fault::FailRecovery)
            {
                log.push_str("受控命令返回失败\n");
                return Err(operation_error(log));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn disabled_at_upgrade_boundary_never_mutates_native_state() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let (previous, source_root, source) = legacy_fixture(home);
        let runner = MockMutations::new(home, &previous, &source, Fault::None);
        runner.write_config(false, true);
        let config = fs::read(home.join("config.toml")).unwrap();
        let registry = fs::read(home.join("installed-plugins/registry.json")).unwrap();
        let old_cache = plugin_tree(&previous.path, false).unwrap();
        assert!(
            upgrade_plugin(
                home,
                &previous,
                &source_root,
                &source,
                &runner,
                &mut String::new()
            )
            .await
            .is_err()
        );
        assert!(runner.calls.lock().unwrap().is_empty());
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config);
        assert_eq!(
            fs::read(home.join("installed-plugins/registry.json")).unwrap(),
            registry
        );
        assert_eq!(plugin_tree(&previous.path, false).unwrap(), old_cache);
    }

    #[tokio::test]
    async fn async_disable_stops_upgrade_without_reinstall_or_restore() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let (previous, source_root, source) = legacy_fixture(home);
        let legacy = plugin_tree(&previous.source, false).unwrap();
        let runner = MockMutations::new(home, &previous, &source, Fault::DisableAfterUninstall);
        assert!(
            upgrade_plugin(
                home,
                &previous,
                &source_root,
                &source,
                &runner,
                &mut String::new()
            )
            .await
            .is_err()
        );
        assert_eq!(*runner.calls.lock().unwrap(), ["uninstall"]);
        assert!(plugin_disabled(home).unwrap());
        assert!(registered_plugin(home).unwrap().is_none());
        assert_eq!(plugin_tree(&previous.source, false).unwrap(), legacy);
    }

    #[tokio::test]
    async fn async_source_edit_is_preserved_without_installing_backup() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let (previous, source_root, source) = legacy_fixture(home);
        let runner = MockMutations::new(home, &previous, &source, Fault::EditSourceAfterUninstall);
        assert!(
            upgrade_plugin(
                home,
                &previous,
                &source_root,
                &source,
                &runner,
                &mut String::new()
            )
            .await
            .is_err()
        );
        assert_eq!(*runner.calls.lock().unwrap(), ["uninstall"]);
        assert_eq!(
            fs::read_to_string(previous.source.join("README.md")).unwrap(),
            "用户新来源说明"
        );
    }

    #[tokio::test]
    async fn user_cache_edit_during_failed_install_prevents_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let (previous, source_root, source) = legacy_fixture(home);
        let runner = MockMutations::new(home, &previous, &source, Fault::EditCacheAfterInstall);
        assert!(
            upgrade_plugin(
                home,
                &previous,
                &source_root,
                &source,
                &runner,
                &mut String::new()
            )
            .await
            .is_err()
        );
        assert_eq!(
            *runner.calls.lock().unwrap(),
            ["uninstall", "install_current"]
        );
        let plugin = registered_plugin(home).unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(plugin.path.join("README.md")).unwrap(),
            "用户新说明"
        );
    }

    #[tokio::test]
    async fn concurrent_registry_change_is_preserved_without_rollback() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let (previous, source_root, source) = legacy_fixture(home);
        let runner = MockMutations::new(home, &previous, &source, Fault::EditRegistryAfterInstall);
        assert!(
            upgrade_plugin(
                home,
                &previous,
                &source_root,
                &source,
                &runner,
                &mut String::new()
            )
            .await
            .is_err()
        );
        assert_eq!(
            *runner.calls.lock().unwrap(),
            ["uninstall", "install_current"]
        );
        let registry: Value = serde_json::from_slice(
            &fs::read(home.join("installed-plugins/registry.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(registry["user_revision"], "用户新注册意图");
    }

    #[tokio::test]
    async fn failed_upgrade_install_restores_complete_legacy_recipe() {
        for fault in [
            Fault::FailBeforeInstall,
            Fault::FailPartialInstall,
            Fault::FailCompleteInstall,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let home = directory.path();
            let (previous, source_root, source) = legacy_fixture(home);
            let before = plugin_tree(&previous.path, false).unwrap();
            let legacy = plugin_tree(&previous.source, false).unwrap();
            let unrelated_config = config_without_plugin(&read_config(home).unwrap()).unwrap();
            let runner = MockMutations::new(home, &previous, &source, fault);
            let error = upgrade_plugin(
                home,
                &previous,
                &source_root,
                &source,
                &runner,
                &mut String::new(),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error.message,
                crate::t!("cli-agent-plugin-grok-update-restored")
            );
            let restored = installed_plugin(home).unwrap().unwrap();
            assert_eq!(restored.version, "0.1.0");
            assert!(restored.source.starts_with(source_root.join("recovery")));
            assert_eq!(plugin_tree(&restored.path, false).unwrap(), before);
            assert_eq!(plugin_tree(&previous.source, false).unwrap(), legacy);
            assert_eq!(
                config_without_plugin(&read_config(home).unwrap()).unwrap(),
                unrelated_config
            );
            assert!(plugin_enabled(home).unwrap());
            let expected = if fault == Fault::FailBeforeInstall {
                vec!["uninstall", "install_current", "install_recovery"]
            } else {
                vec![
                    "uninstall",
                    "install_current",
                    "uninstall",
                    "install_recovery",
                ]
            };
            assert_eq!(*runner.calls.lock().unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn failed_restore_command_never_reports_restore_success() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let (previous, source_root, source) = legacy_fixture(home);
        let runner = MockMutations::new(home, &previous, &source, Fault::FailRecovery);
        let error = upgrade_plugin(
            home,
            &previous,
            &source_root,
            &source,
            &runner,
            &mut String::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.message,
            crate::t!("cli-agent-plugin-grok-restore-failed")
        );
        assert_eq!(
            *runner.calls.lock().unwrap(),
            ["uninstall", "install_current", "install_recovery"]
        );
    }

    #[tokio::test]
    async fn upgrade_011_and_failed_install_preserve_the_original_source() {
        for fault in [Fault::None, Fault::FailBeforeInstall] {
            let directory = tempfile::tempdir().unwrap();
            let home = directory.path();
            let cache = install_fixture(home);
            let source_root = home.join("source");
            let old = write_011_bundle(&source_root, LEGACY_011_README);
            for (name, _) in BUNDLED_FILES {
                fs::copy(old.join(name), cache.join(name)).unwrap();
            }
            let registry_path = home.join("installed-plugins/registry.json");
            let mut registry: Value =
                serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
            registry["repos"]["source-one"]["kind"]["source_path"] = json!(old);
            registry["repos"]["source-one"]["plugins"][PLUGIN_NAME]["version"] = json!("0.1.1");
            fs::write(registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
            let previous = installed_plugin(home).unwrap().unwrap();
            let original = plugin_tree(&old, false).unwrap();
            let source = write_bundle(&source_root).unwrap();
            let runner = MockMutations::new(home, &previous, &source, fault);
            let result = upgrade_plugin(
                home,
                &previous,
                &source_root,
                &source,
                &runner,
                &mut String::new(),
            )
            .await;
            assert_eq!(result.is_ok(), fault == Fault::None);
            let installed = installed_plugin(home).unwrap().unwrap();
            assert_eq!(
                installed.version,
                if fault == Fault::None {
                    PLUGIN_VERSION
                } else {
                    "0.1.1"
                }
            );
            assert_eq!(plugin_tree(&old, false).unwrap(), original);
            validate_expected_tree(&installed.path, &installed.version).unwrap();
        }
    }

    #[tokio::test]
    async fn upgrade_012_and_failed_install_preserve_the_original_source() {
        for fault in [Fault::None, Fault::FailBeforeInstall] {
            let directory = tempfile::tempdir().unwrap();
            let home = directory.path();
            let cache = install_fixture(home);
            let source_root = home.join("source");
            let old = write_012_bundle(&source_root);
            for (name, _) in BUNDLED_FILES {
                fs::copy(old.join(name), cache.join(name)).unwrap();
            }
            let registry_path = home.join("installed-plugins/registry.json");
            let mut registry: Value =
                serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
            registry["repos"]["source-one"]["kind"]["source_path"] = json!(old);
            registry["repos"]["source-one"]["plugins"][PLUGIN_NAME]["version"] = json!("0.1.2");
            fs::write(registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
            let previous = installed_plugin(home).unwrap().unwrap();
            let original = plugin_tree(&old, false).unwrap();
            let source = write_bundle(&source_root).unwrap();
            let runner = MockMutations::new(home, &previous, &source, fault);
            let result = upgrade_plugin(
                home,
                &previous,
                &source_root,
                &source,
                &runner,
                &mut String::new(),
            )
            .await;
            assert_eq!(result.is_ok(), fault == Fault::None);
            let installed = installed_plugin(home).unwrap().unwrap();
            assert_eq!(
                installed.version,
                if fault == Fault::None {
                    PLUGIN_VERSION
                } else {
                    "0.1.2"
                }
            );
            assert_eq!(plugin_tree(&old, false).unwrap(), original);
            validate_expected_tree(&installed.path, &installed.version).unwrap();
        }
    }

    #[tokio::test]
    async fn normal_upgrade_preserves_the_full_old_source_tree() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let (previous, source_root, source) = legacy_fixture(home);
        fs::write(home.join("config.toml"), "[ui]\nscreen_mode='minimal'\n[plugins]\nenabled=['infinishell-grok','user-other']\ndisabled=['user-disabled']\n[permissions]\nalways_approve=false\n").unwrap();
        let other_config = config_without_plugin(&read_config(home).unwrap()).unwrap();
        let registry_path = home.join("installed-plugins/registry.json");
        let mut registry: Value =
            serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
        registry["repos"]["unrelated"] =
            json!({"plugins":{"user-other":{"version":"2.0.0"}},"keep":true});
        fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
        let other_registry =
            registry_without_plugin(Some(&fs::read(&registry_path).unwrap())).unwrap();
        let before = plugin_tree(&previous.source, false).unwrap();
        let runner = MockMutations::new(home, &previous, &source, Fault::None);
        upgrade_plugin(
            home,
            &previous,
            &source_root,
            &source,
            &runner,
            &mut String::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            *runner.calls.lock().unwrap(),
            ["uninstall", "install_current"]
        );
        assert_eq!(
            installed_plugin(home).unwrap().unwrap().version,
            PLUGIN_VERSION
        );
        assert_eq!(plugin_tree(&previous.source, false).unwrap(), before);
        assert_eq!(
            config_without_plugin(&read_config(home).unwrap()).unwrap(),
            other_config
        );
        assert_eq!(
            registry_without_plugin(Some(&fs::read(&registry_path).unwrap())).unwrap(),
            other_registry
        );
    }
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
    assert_eq!(
        GrokPluginManager::new(None).minimum_plugin_version(),
        manifest["version"].as_str().unwrap()
    );
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
fn current_grok_notification_contract_is_limited_to_exact_desktop_version() {
    assert_eq!(
        runtime_is_compatible("grok 1.0.41 (4220f3b224a6)\n", "v22.18.0\n"),
        cfg!(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "windows"
        ))
    );
    for version in ["1.0.40", "1.0.42", "1.0.41-beta"] {
        assert!(!runtime_is_compatible(
            &format!("grok {version}\n"),
            "v22.18.0\n"
        ));
    }
    assert!(!runtime_is_compatible("grok 1.0.41\n", "v16.20.0\n"));
    assert!(!runtime_is_compatible("other 1.0.41\n", "v22.18.0\n"));
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

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
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

#[cfg(windows)]
async fn windows_startup_bridge_argv_probe(
    manager: &GrokPluginManager,
    node: &Path,
    root: &Path,
    artifact: &Path,
    evidence: &mut Value,
) -> Value {
    record_live_installer_stage(artifact, evidence, "windows_bridge_prepare");
    // 只验证真实启动层的 argv；此节点脚本不冒充原生 hook 派发或通知 worker。
    let script = root.join("参数 ' 空格 & 目录.cjs");
    fs::write(&script, "process.stdout.write(JSON.stringify(process.argv.slice(2).map(x=>Buffer.from(x,'utf8').toString('base64'))));").unwrap();
    let arguments = [r"C:\固定 grok\grok.exe", r"C:\插件 '$ %PATH% ! & 目录\"];
    let command = windows_startup_bridge_command(&[
        node.to_str().unwrap(),
        script.to_str().unwrap(),
        arguments[0],
        arguments[1],
    ])
    .unwrap();
    let system_root = PathBuf::from(env::var_os("SYSTEMROOT").unwrap());
    let shells = [
        (
            "windows_bridge_cmd",
            system_root.join("System32/cmd.exe"),
            vec!["/d", "/s", "/c"],
        ),
        (
            "windows_bridge_powershell",
            system_root.join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            vec!["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"],
        ),
    ];
    for (stage, executable, flags) in shells {
        record_live_installer_stage(artifact, evidence, stage);
        let mut log = String::new();
        let mut args = flags.into_iter().map(OsStr::new).collect::<Vec<_>>();
        args.push(OsStr::new(&command));
        let result = manager
            .run(&executable, &args, Duration::from_secs(10), &mut log)
            .await;
        evidence["windows_bridge_last_command"] = match &result {
            Ok(output) => json!({
                "stage":stage, "error_kind":null, "native_exit_code":output.status.code(),
                "stdout_bytes":output.stdout.len(), "stderr_bytes":output.stderr.len(),
                "stdout_sha256":format!("{:x}", Sha256::digest(&output.stdout)),
                "stderr_sha256":format!("{:x}", Sha256::digest(&output.stderr)),
            }),
            // 生产 run 将原生失败封装为 PluginInstallError，不能据此虚构退出码或输出日志。
            Err(_) => {
                json!({"stage":stage, "error_kind":"plugin_install_error", "native_exit_code":null})
            }
        };
        record_live_installer_stage(artifact, evidence, &format!("{stage}_returned"));
        let output = result.unwrap();
        let actual: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            actual,
            arguments.map(|value| base64::engine::general_purpose::STANDARD.encode(value))
        );
        record_live_installer_stage(artifact, evidence, &format!("{stage}_verified"));
    }
    fs::remove_file(script).unwrap();
    json!({"kind":"windows_bridge_argv", "cmd_verified":true, "powershell_51_verified":true,
        "space_unicode_and_shell_metacharacters_preserved":true, "native_hook_execution_verified":false})
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn record_live_installer_stage(artifact: &Path, evidence: &mut Value, stage: &str) {
    evidence["stage"] = json!(stage);
    evidence["progress"]
        .as_array_mut()
        .unwrap()
        .push(json!(stage));
    fs::write(artifact, serde_json::to_vec_pretty(evidence).unwrap()).unwrap();
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn installed_hook_export_version_probe(
    manager: &GrokPluginManager,
    node: &Path,
    home: &Path,
    probe_session: &str,
) -> Value {
    let plugin = installed_plugin(home).unwrap().unwrap();
    let before = plugin_tree(&plugin.path, false).unwrap();
    let script = plugin.path.join("hooks/notify.cjs");
    let manifest: Value =
        serde_json::from_slice(&fs::read(plugin.path.join(".grok-plugin/plugin.json")).unwrap())
            .unwrap();
    let registry = fs::read(home.join("installed-plugins/registry.json")).unwrap();
    let config = fs::read(home.join("config.toml")).unwrap();
    // 真实 Node 加载已安装模块的导出函数；此检查明确不等同 main 的 /dev/tty 通道。
    let program = r#"
const hook = require(process.argv[1]);
const session = process.argv[2];
const input = hook.normalize({hookEventName:"session_start",sessionId:session}, {
  WARP_CLI_AGENT_PROTOCOL_VERSION:"1",GROK_HOOK_EVENT:"session_start",GROK_SESSION_ID:session
});
process.stdout.write(JSON.stringify(hook.makeNotification(input, {})));
"#;
    let mut command = Command::new(node);
    command
        .arg("-e")
        .arg(program)
        .arg(&script)
        .arg(probe_session)
        .env_clear()
        .kill_on_drop(true);
    let output = command
        .output()
        .with_timeout(Duration::from_secs(5))
        .await
        .expect("已安装通知模块检查超时")
        .expect("已安装通知模块检查无法启动");
    assert!(output.status.success());
    assert!(output.stderr.is_empty() && output.stdout.len() <= 4096);
    let notification: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(notification["v"], 1);
    assert_eq!(notification["agent"], "grok");
    assert_eq!(notification["event"], "session_start");
    assert_eq!(notification["session_id"], probe_session);
    assert_eq!(notification["plugin_version"], manifest["version"]);
    assert_eq!(
        notification["plugin_version"].as_str().unwrap(),
        manager.minimum_plugin_version()
    );
    assert_eq!(plugin_tree(&plugin.path, false).unwrap(), before);
    assert_eq!(installed_plugin(home).unwrap(), Some(plugin));
    assert_eq!(
        fs::read(home.join("installed-plugins/registry.json")).unwrap(),
        registry
    );
    assert_eq!(fs::read(home.join("config.toml")).unwrap(), config);
    json!({
        "kind":"installed_module_export_probe", "node_exit_code":output.status.code().unwrap(),
        "plugin_version":notification["plugin_version"], "installed_manifest_version":manifest["version"],
        "manager_minimum_plugin_version":manager.minimum_plugin_version(),
        "installed_notify_sha256":format!("{:x}", Sha256::digest(fs::read(script).unwrap())),
        "stdout_bytes":output.stdout.len(), "stdout_sha256":format!("{:x}", Sha256::digest(&output.stdout)),
        "stderr_bytes":output.stderr.len(), "stderr_sha256":format!("{:x}", Sha256::digest(&output.stderr)),
        "installed_hook_export_version_verified":true, "main_entry_verified":false,
        "tty_main_validation_required":true, "config_registry_and_cache_unchanged":true,
        "model_input_submitted":false
    })
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
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
    #[cfg(not(windows))]
    assert_eq!(
        dirs::home_dir().unwrap().canonicalize().unwrap(),
        root.join("home")
    );
    #[cfg(not(windows))]
    let data_root = dirs::data_local_dir().unwrap().canonicalize().unwrap();
    #[cfg(windows)]
    let data_root = PathBuf::from(env::var_os("LOCALAPPDATA").unwrap())
        .canonicalize()
        .unwrap();
    assert!(data_root.starts_with(root.join("home")));
    let manager = GrokPluginManager::new(Some(env::var("PATH").unwrap()));
    #[cfg(windows)]
    let manager = {
        let mut manager = manager;
        manager.test_source_root = Some(data_root.join("InfiniShell/cli-agent-plugins/grok"));
        manager
    };
    let source_root = manager.source_root().unwrap();
    assert!(source_root.starts_with(&data_root) && !source_root.exists());
    let grok_home = grok_home_dir().unwrap().canonicalize().unwrap();
    assert_eq!(grok_home, root.join("home/.grok"));
    assert_eq!(fs::read_dir(&grok_home).unwrap().count(), 0);
    let artifact = root.join("grok-production-installer.json");
    assert!(!artifact.exists());
    let mut evidence = json!({
        "passed": false, "credentials_provided": false, "model_input_submitted": false,
        "test_binary_sha256": format!("{:x}", Sha256::digest(fs::read(env::current_exe().unwrap()).unwrap())),
        "steps": [],
        "progress": [],
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
    record_live_installer_stage(&artifact, &mut evidence, "binary_bindings_verified");
    let mut native_log = String::new();
    record_live_installer_stage(&artifact, &mut evidence, "runtime_verification");
    let (grok, _) = manager.verify_runtime(&mut native_log).await.unwrap();
    record_live_installer_stage(&artifact, &mut evidence, "runtime_verified");
    let node = manager.executable("node").unwrap();
    record_live_installer_stage(&artifact, &mut evidence, "node_version_probe");
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
    record_live_installer_stage(&artifact, &mut evidence, "node_version_verified");
    record_live_installer_stage(&artifact, &mut evidence, "grok_version_probe");
    let output = manager
        .run(
            &grok,
            &[OsStr::new("--version")],
            Duration::from_secs(3),
            &mut native_log,
        )
        .await
        .unwrap();
    let grok_version = String::from_utf8(output.stdout).unwrap();
    assert!(runtime_is_compatible(&grok_version, &node_version));
    evidence["grok_version"] = json!(
        grok_version
            .trim()
            .strip_prefix("grok ")
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
    );
    evidence["node_version"] = json!(node_version.trim());
    record_live_installer_stage(&artifact, &mut evidence, "grok_version_verified");
    #[cfg(windows)]
    {
        let verified =
            windows_startup_bridge_argv_probe(&manager, &node, &root, &artifact, &mut evidence)
                .await;
        evidence["windows_bridge_argv"] = verified;
        record_live_installer_stage(&artifact, &mut evidence, "windows_bridge_verified");
    }
    record_live_installer_stage(&artifact, &mut evidence, "production_install");
    assert!(!manager.is_installed());

    manager.install().await.unwrap();
    assert!(manager.is_installed() && !manager.needs_update());
    record_live_installer_stage(&artifact, &mut evidence, "native_inspect");
    let inspected = manager
        .run(
            &grok,
            &[OsStr::new("inspect"), OsStr::new("--json")],
            Duration::from_secs(5),
            &mut native_log,
        )
        .await
        .unwrap();
    let inspected: Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(inspected["grokVersion"], evidence["grok_version"]);
    let observed = inspected["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|plugin| plugin["name"] == PLUGIN_NAME)
        .collect::<Vec<_>>();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0]["enabled"], true);
    assert_eq!(observed[0]["provides"]["hooks"], true);
    assert_eq!(
        observed[0]["path"],
        json!(installed_plugin(&grok_home).unwrap().unwrap().path)
    );
    evidence["native_inspect_verified"] = json!(true);
    evidence["native_hook_execution_verified"] = json!(false);
    let registry_path = grok_home.join("installed-plugins/registry.json");
    let config_path = grok_home.join("config.toml");
    record_live_installer_stage(&artifact, &mut evidence, "installed_hook_export");
    let installed_hook_export = installed_hook_export_version_probe(
        &manager,
        &node,
        &grok_home,
        "infinishell-installed-hook-install-export-probe",
    )
    .await;
    evidence["steps"]
        .as_array_mut()
        .unwrap()
        .push(json!({"step": "production_install", "passed": true,
            "installed_hook_export":installed_hook_export, "installed_hook_main_verified":false}));
    record_live_installer_stage(&artifact, &mut evidence, "production_install_verified");

    // 用完整旧配方建立原生升级夹具；下面仍由生产 update 完成升级。
    record_live_installer_stage(&artifact, &mut evidence, "legacy_fixture_install");
    let legacy_source = write_013_bundle(&source_root);
    validate_expected_tree(&legacy_source, "0.1.3").unwrap();
    let legacy_source_before = plugin_tree(&legacy_source, false).unwrap();
    manager
        .run(
            &grok,
            &[
                OsStr::new("plugin"),
                OsStr::new("uninstall"),
                OsStr::new("--keep-data"),
                OsStr::new(PLUGIN_NAME),
            ],
            Duration::from_secs(30),
            &mut native_log,
        )
        .await
        .unwrap();
    manager
        .install_source(&grok, &legacy_source, &mut native_log)
        .await
        .unwrap();
    let old = installed_plugin(&grok_home).unwrap().unwrap();
    assert_eq!(old.version, "0.1.3");
    assert_eq!(old.source, legacy_source);
    if grok_version.starts_with("grok 1.0.41 ") {
        // 明确构造旧包对应的旧 bridge 配方，不把夹具发布动作算作生产升级。
        let files = startup_bridge_files(&grok_home, &grok, &node).unwrap();
        fs::write(&files[0].0, legacy_startup_bridge_bytes()).unwrap();
        fs::write(&files[1].0, &files[1].1).unwrap();
    }
    assert!(manager.is_installed() && manager.needs_update() && manager.can_auto_install());
    record_live_installer_stage(
        &artifact,
        &mut evidence,
        "production_upgrade_known_013_to_014",
    );
    manager.update().await.unwrap();
    let plugin = installed_plugin(&grok_home).unwrap().unwrap();
    assert_eq!(plugin.version, PLUGIN_VERSION);
    assert!(manager.is_installed() && !manager.needs_update());
    validate_expected_tree(&legacy_source, "0.1.3").unwrap();
    assert_eq!(
        plugin_tree(&legacy_source, false).unwrap(),
        legacy_source_before
    );
    let upgraded_hook_export = installed_hook_export_version_probe(
        &manager,
        &node,
        &grok_home,
        "infinishell-installed-hook-upgrade-export-probe",
    )
    .await;
    evidence["steps"].as_array_mut().unwrap().push(json!({
        "step":"production_upgrade_known_013_to_014", "passed":true,
        "installed_hook_export":upgraded_hook_export, "installed_hook_main_verified":false,
        "previous_plugin_version":"0.1.3", "current_plugin_version":PLUGIN_VERSION,
        "legacy_source_unchanged":true, "fixture_old_native_install":true,
        "production_update_call":true, "model_input_submitted":false
    }));
    record_live_installer_stage(&artifact, &mut evidence, "production_upgrade_verified");
    let registry = fs::read(&registry_path).unwrap();
    let config = fs::read(&config_path).unwrap();

    record_live_installer_stage(&artifact, &mut evidence, "production_same_version_update");
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
    record_live_installer_stage(
        &artifact,
        &mut evidence,
        "production_same_version_update_verified",
    );

    record_live_installer_stage(
        &artifact,
        &mut evidence,
        "production_disabled_update_rejected",
    );
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
    record_live_installer_stage(
        &artifact,
        &mut evidence,
        "production_disabled_update_rejected_verified",
    );

    // 此段调用生产文件事务的故障注入点，不把它写成 apply 或 SIGKILL 验收。
    record_live_installer_stage(
        &artifact,
        &mut evidence,
        "file_transaction_failure_rollback",
    );
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
    // 已通过的第五段立即落盘，后续恢复失败也不能丢失这段真实检查。
    record_live_installer_stage(
        &artifact,
        &mut evidence,
        "file_transaction_failure_rollback_verified",
    );
    record_live_installer_stage(&artifact, &mut evidence, "production_update_after_rollback");
    manager.update().await.unwrap();
    assert!(manager.is_installed() && !manager.needs_update());
    assert_eq!(fs::read(&config_path).unwrap(), enabled_config);
    assert_eq!(fs::read(&registry_path).unwrap(), enabled_registry);
    evidence["steps"]
        .as_array_mut()
        .unwrap()
        .push(json!({"step": "production_update_after_rollback", "passed": true}));
    evidence["passed"] = json!(true);
    record_live_installer_stage(&artifact, &mut evidence, "finished");
}
