//! 显式授权的零模型原生升级验收；只读私有清单，不提供通用命令执行入口。

#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _, symlink};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[cfg(target_os = "macos")]
use command::Stdio;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use warpui::r#async::Timer;

use super::{
    Channel, Error, Journal, Source, execute, inspect, journal_root, parse_version,
    previous_failure_for_intent,
};
use crate::ai::cli_agent_runtime::managed_process::{self, ExitReason};
use crate::terminal::cli_agent::CLIAgent;

const SCOPE: &str = "cli_autoupdate_native_product";
const MARKER: &str = "InfiniShell private updater fixture; no credentials or model inputs\n";
const MAX_MANIFEST: usize = 64 * 1024;
const COMPILED_SOURCE_FILES: [(&str, &[u8]); 8] = [
    (
        "app/src/terminal/cli_agent_updates.rs",
        include_bytes!("../cli_agent_updates.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources.rs",
        include_bytes!("sources.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_live_tests.rs",
        include_bytes!("sources_live_tests.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process_atomic_linux.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_linux.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process_atomic_macos.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_macos.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process_atomic_windows.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_windows.rs"),
    ),
    (
        "script/cli-agent-parity/verify_cli_autoupdate.py",
        include_bytes!("../../../../script/cli-agent-parity/verify_cli_autoupdate.py"),
    ),
];
const SUPERVISOR_COMPILED_SOURCES: [&[u8]; 6] = [
    include_bytes!("../cli_agent_updates.rs"),
    include_bytes!("sources.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_linux.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_macos.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_windows.rs"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binary {
    path: PathBuf,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ConfigTransition {
    Channel {
        before: Option<String>,
        after: String,
    },
    CodexMarker {
        before_sha256: Option<String>,
        after_sha256: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    scope: String,
    case_id: String,
    root: PathBuf,
    agent: String,
    channel: String,
    expected: String,
    old_version: String,
    target_version: String,
    entry: PathBuf,
    old_binary: Binary,
    target_binary: Binary,
    worker: Binary,
    supervisor: Binary,
    source_manifest: Binary,
    gates_report: Binary,
    bundle_report: Binary,
    timeout_seconds: u64,
    #[serde(default)]
    config_transition: Option<ConfigTransition>,
}

#[derive(Clone, Eq, PartialEq)]
struct ConfigSnapshot {
    bytes: Option<Vec<u8>>,
    mode: Option<u32>,
}

#[derive(Serialize)]
struct Evidence {
    schema: u32,
    scope: &'static str,
    case_id: String,
    agent: String,
    channel: String,
    expected: String,
    stage: &'static str,
    passed: bool,
    error: Option<String>,
    failure_code: Option<&'static str>,
    manifest_sha256: String,
    worker_sha256: String,
    supervisor_sha256: String,
    source_manifest_sha256: String,
    old_sha256: String,
    target_sha256: String,
    old_version: String,
    target_version: String,
    product_inspect_calls: u32,
    product_execute_calls: u32,
    model_inputs_sent: u32,
    credentials_provided: bool,
    private_environment_verified: bool,
    credential_files_absent_before: bool,
    credential_files_absent_after: bool,
    config_files_checked: usize,
    config_bytes_unchanged: bool,
    config_semantics_unchanged: bool,
    config_permissions_unchanged: bool,
    config_transition: Option<ConfigTransition>,
    config_transition_verified: bool,
    unrelated_config_bytes_unchanged: bool,
    config_permissions_preserved: bool,
    plan_requires_native_update: Option<bool>,
    supervisor_generations_unchanged: bool,
    entry_unchanged: bool,
    old_binary_unchanged: bool,
    target_reference_unchanged: bool,
    entry_matches_expected: bool,
    post_version_matches: bool,
    journal_absent: bool,
    production_chain_verified: bool,
    same_source_build_verified: bool,
    same_commit_verified_by_runner: bool,
    failure_intent_persisted: bool,
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn binary_sha(path: &Path) -> Result<String, &'static str> {
    super::stamp(path)
        .map(|stamp| hex::encode(stamp.digest))
        .map_err(|_| "binary_unreadable")
}

fn private_file(path: &Path, mode: u32) -> Result<Vec<u8>, &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "private_file_missing")?;
    if !metadata.is_file()
        || metadata.permissions().mode() & 0o777 != mode
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.len() > MAX_MANIFEST as u64
    {
        return Err("private_file_boundary");
    }
    fs::read(path).map_err(|_| "private_file_unreadable")
}

fn under(root: &Path, path: &Path) -> bool {
    path.is_absolute()
        && path != root
        && path.starts_with(root)
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
}

fn plain_path(root: &Path, path: &Path) -> Result<(), &'static str> {
    if !under(root, path) {
        return Err("path_outside_fixture");
    }
    for ancestor in path.ancestors().take_while(|ancestor| *ancestor != root) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err("fixture_symlink"),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("fixture_path_unreadable"),
        }
    }
    Ok(())
}

fn agent_channel(manifest: &Manifest) -> Result<(CLIAgent, Channel), &'static str> {
    let agent = match manifest.agent.as_str() {
        "codex" => CLIAgent::Codex,
        "claude" => CLIAgent::Claude,
        "grok" => CLIAgent::Grok,
        _ => return Err("invalid_agent"),
    };
    let channel = match manifest.channel.as_str() {
        "follow_installation" => Channel::FollowInstallation,
        "latest" => Channel::Latest,
        "stable" => Channel::Stable,
        "alpha" => Channel::Alpha,
        _ => return Err("invalid_channel"),
    };
    if !super::channel_supported(agent, channel) {
        return Err("unsupported_channel");
    }
    Ok((agent, channel))
}

fn auth_absent(root: &Path) -> bool {
    [
        "home/.codex/auth.json",
        "home/.grok/auth.json",
        "home/.claude/.credentials.json",
        "home/.claude/credentials.json",
        "home/.netrc",
        "home/.npmrc",
    ]
    .iter()
    .all(|relative| {
        fs::symlink_metadata(root.join(relative))
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    })
}

fn fixture_environment_entry_allowed(key: &OsStr, value: &OsStr, uid: u32) -> bool {
    let Some(key) = key.to_str() else {
        return false;
    };
    let allowed = [
        "HOME",
        "USERPROFILE",
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
        "GROK_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "TMPDIR",
        "TMP",
        "TEMP",
        "PATH",
        "LANG",
        "LC_ALL",
        "SHELL",
        "DISABLE_AUTOUPDATER",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
        "INFINISHELL_CLI_AUTOUPDATE_ALLOW",
        "INFINISHELL_CLI_AUTOUPDATE_CASE",
        "INFINISHELL_CLI_AUTOUPDATE_MANIFEST",
        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE",
    ];
    if allowed.contains(&key) {
        return true;
    }
    // macOS 框架在 main 前加入此键；只认可本进程 UID 的已核实默认编码。
    cfg!(target_os = "macos")
        && key == "__CF_USER_TEXT_ENCODING"
        && value == OsStr::new(&format!("0x{uid:X}:0x0:0x0"))
}

fn verify_environment(manifest: &Manifest, manifest_path: &Path) -> Result<(), &'static str> {
    let root = &manifest.root;
    let root_metadata = fs::symlink_metadata(root).map_err(|_| "fixture_missing")?;
    if !root_metadata.is_dir()
        || root_metadata.permissions().mode() & 0o777 != 0o700
        || root_metadata.uid() != unsafe { libc::geteuid() }
        || root.canonicalize().ok().as_ref() != Some(root)
        || !root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("infinishell-cli-autoupdate-"))
        || manifest_path != root.join("manifest.private.json")
        || private_file(&root.join(".infinishell-cli-autoupdate"), 0o600)? != MARKER.as_bytes()
    {
        return Err("fixture_identity");
    }
    let expected = [
        ("HOME", "home"),
        ("USERPROFILE", "home"),
        ("CODEX_HOME", "home/.codex"),
        ("CLAUDE_CONFIG_DIR", "home/.claude"),
        ("GROK_HOME", "home/.grok"),
        ("XDG_CONFIG_HOME", "home/.config"),
        ("XDG_DATA_HOME", "home/.local/share"),
        ("XDG_CACHE_HOME", "home/.cache"),
        ("XDG_STATE_HOME", "home/.local/state"),
        ("TMPDIR", "tmp"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
    ];
    for (name, relative) in expected {
        let path = root.join(relative);
        if env::var_os(name).map(PathBuf::from).as_ref() != Some(&path)
            || path.canonicalize().ok().as_ref() != Some(&path)
        {
            return Err("environment_path");
        }
        plain_path(root, &path)?;
    }
    if env::current_dir().ok().as_ref() != Some(&root.join("project"))
        || env::var("INFINISHELL_CLI_AUTOUPDATE_ALLOW").as_deref() != Ok("native-update-no-model")
        || env::var("INFINISHELL_CLI_AUTOUPDATE_CASE").as_deref() != Ok(manifest.case_id.as_str())
        || env::var_os("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE")
            .map(PathBuf::from)
            .as_ref()
            != Some(&manifest.supervisor.path)
        || env::var("PATH").ok()
            != Some(format!(
                "{}:{}:/usr/bin:/bin:/usr/sbin:/sbin",
                root.join("home/.local/bin").display(),
                root.join("home/.grok/bin").display()
            ))
    {
        return Err("environment_contract");
    }
    // 平台只可补入已核实的默认编码标记，凭据、代理和进程注入变量仍拒绝。
    let uid = unsafe { libc::getuid() };
    if env::vars_os().any(|(key, value)| !fixture_environment_entry_allowed(&key, &value, uid))
        || !auth_absent(root)
    {
        return Err("environment_not_isolated");
    }
    let journal = journal_root().map_err(|_| "state_unavailable")?;
    plain_path(root, &journal)?;
    if !journal.starts_with(root.join("home")) {
        return Err("state_outside_home");
    }
    Ok(())
}

fn verify_build_binding(manifest: &Manifest) -> Result<(), &'static str> {
    let read = |binary: &Binary| -> Result<Value, &'static str> {
        if binary_sha(&binary.path)? != binary.sha256
            || fs::metadata(&binary.path)
                .map_err(|_| "build_binding_missing")?
                .len()
                > 1024 * 1024
        {
            return Err("build_binding_digest");
        }
        serde_json::from_slice(&fs::read(&binary.path).map_err(|_| "build_binding_missing")?)
            .map_err(|_| "build_binding_shape")
    };
    let source = read(&manifest.source_manifest)?;
    let gates = read(&manifest.gates_report)?;
    let bundle = read(&manifest.bundle_report)?;
    let matches_binary = |value: &Value, binary: &Binary| {
        value
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .as_ref()
            == Some(&binary.path)
            && value.get("sha256").and_then(Value::as_str) == Some(binary.sha256.as_str())
    };
    if !compiled_source_files_match(&source)
        || gates.get("source_manifest_sha256").and_then(Value::as_str)
            != Some(manifest.source_manifest.sha256.as_str())
        || bundle.get("source_manifest_sha256").and_then(Value::as_str)
            != Some(manifest.source_manifest.sha256.as_str())
        || !gates
            .get("test_binary")
            .is_some_and(|value| matches_binary(value, &manifest.worker))
        || !bundle
            .get("worker")
            .is_some_and(|value| matches_binary(value, &manifest.supervisor))
        || binary_sha(&manifest.supervisor.path)? != manifest.supervisor.sha256
        || !binary_contains_all(&manifest.supervisor.path, &SUPERVISOR_COMPILED_SOURCES)?
        || !strict_signature_verified(&manifest.supervisor.path)
    {
        return Err("build_binding_mismatch");
    }
    Ok(())
}

fn binary_contains_all(path: &Path, needles: &[&[u8]]) -> Result<bool, &'static str> {
    let maximum = needles.iter().map(|needle| needle.len()).max().unwrap_or(0);
    if maximum == 0 || needles.iter().any(|needle| needle.is_empty()) {
        return Err("build_binding_shape");
    }
    let mut found = vec![false; needles.len()];
    let mut file = fs::File::open(path).map_err(|_| "build_binding_missing")?;
    let mut tail = Vec::new();
    let mut chunk = vec![0; 1024 * 1024];
    loop {
        let count = file.read(&mut chunk).map_err(|_| "build_binding_missing")?;
        if count == 0 {
            return Ok(found.into_iter().all(|present| present));
        }
        tail.extend_from_slice(&chunk[..count]);
        for (index, needle) in needles.iter().enumerate() {
            if !found[index]
                && tail
                    .windows(needle.len())
                    .any(|candidate| candidate == *needle)
            {
                found[index] = true;
            }
        }
        if found.iter().all(|present| *present) {
            return Ok(true);
        }
        let retained = tail.len().min(maximum.saturating_sub(1));
        let discarded = tail.len() - retained;
        tail.drain(..discarded);
    }
}

fn compiled_source_files_match(source: &Value) -> bool {
    let Some(files) = source.get("files").and_then(Value::as_array) else {
        return false;
    };
    COMPILED_SOURCE_FILES.iter().all(|(expected_path, bytes)| {
        let mut rows = files
            .iter()
            .filter(|row| row.get("path").and_then(Value::as_str) == Some(*expected_path));
        let Some(row) = rows.next() else {
            return false;
        };
        rows.next().is_none()
            && row.get("sha256").and_then(Value::as_str) == Some(sha(bytes).as_str())
            && row
                .get("bytes")
                .is_none_or(|value| value.as_u64() == Some(bytes.len() as u64))
    })
}

fn strict_signature_verified(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        return command::blocking::Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        true
    }
}

fn config_snapshot(root: &Path) -> Result<BTreeMap<&'static str, ConfigSnapshot>, &'static str> {
    let mut result = BTreeMap::new();
    for relative in [
        "home/.codex/config.toml",
        "home/.codex/packages/standalone/auto-update-version",
        "home/.grok/config.toml",
        "home/.claude/settings.json",
        "home/.claude/.claude.json",
        "home/.claude.json",
        "home/.profile",
        "home/.bashrc",
        "home/.bash_profile",
        "home/.zshrc",
        "home/.zprofile",
        "home/.zshenv",
        "home/.config/fish/config.fish",
    ] {
        let path = root.join(relative);
        plain_path(root, &path)?;
        let snapshot = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && metadata.len() <= 1024 * 1024 => ConfigSnapshot {
                bytes: Some(fs::read(path).map_err(|_| "config_unreadable")?),
                mode: Some(metadata.permissions().mode() & 0o777),
            },
            Ok(_) => return Err("config_not_regular"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ConfigSnapshot {
                bytes: None,
                mode: None,
            },
            Err(_) => return Err("config_unreadable"),
        };
        result.insert(relative, snapshot);
    }
    Ok(result)
}

fn semantic_equal(relative: &str, before: &ConfigSnapshot, after: &ConfigSnapshot) -> bool {
    if before.bytes == after.bytes {
        return true;
    }
    let (Some(before), Some(after)) = (&before.bytes, &after.bytes) else {
        return false;
    };
    if relative.ends_with(".json") {
        let left = serde_json::from_slice::<Value>(before);
        return left.is_ok() && left.ok() == serde_json::from_slice::<Value>(after).ok();
    }
    if relative.ends_with(".toml") {
        let parse = |bytes: &[u8]| std::str::from_utf8(bytes).ok()?.parse::<toml::Value>().ok();
        let left = parse(before);
        return left.is_some() && left == parse(after);
    }
    false
}

fn transition_valid(manifest: &Manifest) -> bool {
    let Some(transition) = &manifest.config_transition else {
        return manifest.expected != "channel_only";
    };
    if !matches!(manifest.expected.as_str(), "updated" | "channel_only") {
        return false;
    }
    match transition {
        ConfigTransition::Channel { before, after } => {
            let allowed = match manifest.agent.as_str() {
                "grok" => ["stable", "alpha"],
                "claude" => ["latest", "stable"],
                _ => return false,
            };
            after == &manifest.channel
                && allowed.contains(&after.as_str())
                && before
                    .as_deref()
                    .is_none_or(|before| allowed.contains(&before))
                && before.as_deref().unwrap_or(allowed[0]) != after
        }
        ConfigTransition::CodexMarker {
            before_sha256,
            after_sha256,
        } => {
            manifest.agent == "codex"
                && manifest.expected == "updated"
                && [before_sha256, after_sha256].into_iter().all(|digest| {
                    digest.as_ref().is_none_or(|digest| {
                        digest.len() == 64
                            && digest
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    })
                })
        }
    }
}

fn channel_config_matches(
    agent: &str,
    before: &ConfigSnapshot,
    after: &ConfigSnapshot,
    expected_before: Option<&str>,
    expected_after: &str,
) -> bool {
    let Some(after) = after.bytes.as_deref() else {
        return false;
    };
    match agent {
        "claude" => {
            let mut expected = match before.bytes.as_deref() {
                Some(bytes) => match serde_json::from_slice::<Value>(bytes) {
                    Ok(value) => value,
                    Err(_) => return false,
                },
                None => serde_json::json!({}),
            };
            let Some(object) = expected.as_object_mut() else {
                return false;
            };
            if object.get("autoUpdatesChannel").map(Value::as_str) != expected_before.map(Some) {
                return false;
            }
            object.insert(
                "autoUpdatesChannel".to_owned(),
                Value::String(expected_after.to_owned()),
            );
            serde_json::from_slice::<Value>(after).ok().as_ref() == Some(&expected)
        }
        "grok" => {
            let parse = |bytes: &[u8]| std::str::from_utf8(bytes).ok()?.parse::<toml::Value>().ok();
            let Some(mut expected) = parse(before.bytes.as_deref().unwrap_or_default()) else {
                return false;
            };
            let Some(table) = expected.as_table_mut() else {
                return false;
            };
            let cli = table
                .entry("cli")
                .or_insert_with(|| toml::Value::Table(Default::default()));
            let Some(cli) = cli.as_table_mut() else {
                return false;
            };
            if cli.get("channel").map(toml::Value::as_str) != expected_before.map(Some) {
                return false;
            }
            cli.insert(
                "channel".to_owned(),
                toml::Value::String(expected_after.to_owned()),
            );
            parse(after).as_ref() == Some(&expected)
        }
        _ => false,
    }
}

fn codex_marker_matches(
    manifest: &Manifest,
    before: &ConfigSnapshot,
    after: &ConfigSnapshot,
) -> bool {
    let releases = manifest
        .root
        .join("home/.codex/packages/standalone/releases");
    let Some(old) = manifest
        .old_binary
        .path
        .strip_prefix(&releases)
        .ok()
        .and_then(|path| path.components().next())
        .and_then(|component| component.as_os_str().to_str())
    else {
        return false;
    };
    let platforms = [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-musl",
        "aarch64-pc-windows-msvc",
        "x86_64-pc-windows-msvc",
    ];
    let Some(platform) = platforms
        .into_iter()
        .find(|platform| old == format!("{}-{platform}", manifest.old_version))
    else {
        return false;
    };
    let target = format!("{}-{platform}", manifest.target_version);
    let (Ok(old_version), Ok(target_version)) = (
        parse_version(&manifest.old_version),
        parse_version(&manifest.target_version),
    ) else {
        return false;
    };
    let expected = if target_version.alpha.is_some() {
        None
    } else if old_version.alpha.is_none() && before.bytes.as_deref() == Some(old.as_bytes()) {
        Some(target.as_bytes())
    } else if before.bytes.as_deref() == Some(target.as_bytes()) {
        None
    } else {
        before.bytes.as_deref()
    };
    after.bytes.as_deref() == expected
}

// 只为清单声明的目标字段放行语义变更；其余路径仍须逐字节保全。
fn verify_config_transition(
    manifest: &Manifest,
    before: &BTreeMap<&str, ConfigSnapshot>,
    after: &BTreeMap<&str, ConfigSnapshot>,
) -> (bool, bool, bool) {
    if !transition_valid(manifest) || before.keys().ne(after.keys()) {
        return (false, false, false);
    }
    let Some(transition) = &manifest.config_transition else {
        let unchanged = before == after;
        return (unchanged, unchanged, unchanged);
    };
    let path = match transition {
        ConfigTransition::Channel { .. } => match manifest.agent.as_str() {
            "grok" => "home/.grok/config.toml",
            "claude" => "home/.claude/settings.json",
            _ => return (false, false, false),
        },
        ConfigTransition::CodexMarker { .. } => {
            "home/.codex/packages/standalone/auto-update-version"
        }
    };
    let (Some(old), Some(new)) = (before.get(path), after.get(path)) else {
        return (false, false, false);
    };
    let unrelated = before
        .iter()
        .all(|(key, old)| *key == path || after.get(key) == Some(old));
    let permissions = unrelated
        && match (old.bytes.is_some(), new.bytes.is_some()) {
            (true, true) => old.mode == new.mode,
            (false, true) => old.mode.is_none() && new.mode == Some(0o600),
            (true, false) => {
                matches!(transition, ConfigTransition::CodexMarker { .. }) && new.mode.is_none()
            }
            (false, false) => old.mode.is_none() && new.mode.is_none(),
        };
    let intended = match transition {
        ConfigTransition::Channel { before, after } => {
            channel_config_matches(&manifest.agent, old, new, before.as_deref(), after)
        }
        ConfigTransition::CodexMarker {
            before_sha256,
            after_sha256,
        } => {
            old.bytes.as_deref().map(sha).as_ref() == before_sha256.as_ref()
                && new.bytes.as_deref().map(sha).as_ref() == after_sha256.as_ref()
                && codex_marker_matches(manifest, old, new)
        }
    };
    (intended && unrelated && permissions, unrelated, permissions)
}

fn supervisor_generations(root: &Path) -> Result<BTreeSet<PathBuf>, &'static str> {
    let path = journal_root()
        .map_err(|_| "state_unavailable")?
        .join("cli-agent-processes");
    plain_path(root, &path)?;
    let rows = match fs::read_dir(path) {
        Ok(rows) => rows,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(_) => return Err("supervisor_snapshot_failed"),
    };
    let mut result = BTreeSet::new();
    for row in rows {
        let row = row.map_err(|_| "supervisor_snapshot_failed")?;
        if !row.file_type().is_ok_and(|kind| kind.is_dir()) || result.len() >= 64 {
            return Err("supervisor_snapshot_failed");
        }
        result.insert(row.path());
    }
    Ok(result)
}

struct WriteBlock {
    path: PathBuf,
    mode: u32,
    active: bool,
}

impl WriteBlock {
    fn new(root: &Path, agent: CLIAgent) -> Result<Self, &'static str> {
        let path = if agent == CLIAgent::Codex {
            root.join("home/.codex/packages/standalone/releases")
        } else if agent == CLIAgent::Claude {
            root.join("home/.local/share/claude/versions")
        } else if agent == CLIAgent::Grok {
            root.join("home/.grok/downloads")
        } else {
            return Err("failure_boundary_invalid");
        };
        plain_path(root, &path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| "failure_boundary_missing")?;
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err("failure_boundary_invalid");
        }
        let mode = metadata.permissions().mode() & 0o777;
        fs::set_permissions(&path, fs::Permissions::from_mode(mode & !0o222))
            .map_err(|_| "failure_boundary_read_only_failed")?;
        Ok(Self {
            path,
            mode,
            active: true,
        })
    }

    fn restore(mut self) -> Result<(), &'static str> {
        fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode))
            .map_err(|_| "failure_boundary_restore_failed")?;
        self.active = false;
        Ok(())
    }
}

impl Drop for WriteBlock {
    fn drop(&mut self) {
        if self.active {
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
        }
    }
}

async fn wait_for_native_start(root: &Path, agent: CLIAgent) -> Result<uuid::Uuid, &'static str> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let path = journal_root()
                .map_err(|_| "state_unavailable")?
                .join(format!("{}.json", agent.command_prefix()));
            if let Ok(bytes) = fs::read(path)
                && let Ok(journal) = serde_json::from_slice::<Journal>(&bytes)
                && let Some(generation) = journal.generation
            {
                let directory = journal_root()
                    .map_err(|_| "state_unavailable")?
                    .join("cli-agent-processes")
                    .join(generation.to_string());
                plain_path(root, &directory)?;
                #[cfg(target_os = "macos")]
                let started = directory.join("macos-native.json").is_file();
                #[cfg(not(target_os = "macos"))]
                let started = directory.join("manifest.json").is_file();
                if started {
                    return Ok(generation);
                }
            }
            Timer::after(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "native_start_timeout")?
}

async fn wait_for_confirmed_exit(root: &Path, generation: uuid::Uuid) -> Result<(), &'static str> {
    let state = journal_root().map_err(|_| "state_unavailable")?;
    plain_path(root, &state)?;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match managed_process::confirmed_exit(&state, generation) {
                Ok(Some(receipt))
                    if receipt.containment != "not_started"
                        && matches!(
                            receipt.exit_reason,
                            ExitReason::HostDisconnected | ExitReason::StdioClosed
                        ) =>
                {
                    return Ok(());
                }
                Ok(Some(_)) | Err(_) => return Err("interrupted_exit_unconfirmed"),
                Ok(None) => {
                    Timer::after(Duration::from_millis(10)).await;
                }
            }
        }
    })
    .await
    .map_err(|_| "interrupted_exit_timeout")?
}

async fn exercise(manifest: &Manifest, evidence: &mut Evidence) -> Result<(), &'static str> {
    let (agent, channel) = agent_channel(manifest)?;
    let root = &manifest.root;
    let before = config_snapshot(root)?;
    let generations_before = supervisor_generations(root)?;
    let original_entry = fs::read_link(&manifest.entry).map_err(|_| "entry_not_symlink")?;
    evidence.config_files_checked = before.len();
    let client = Arc::new(http_client::Client::new());
    evidence.stage = "inspect_before";
    evidence.product_inspect_calls += 1;
    let report = inspect(agent, Some(manifest.entry.clone()), channel, &client)
        .await
        .map_err(|error| {
            evidence.error = Some(format!("{error:?}"));
            "inspect_before_failed"
        })?;
    if report.source != Source::Native
        || report.error.is_some()
        || report.up_to_date
        || report.installed_version != manifest.old_version
        || report.latest_version != manifest.target_version
    {
        return Err("native_plan_mismatch");
    }
    let plan = report.plan.ok_or("native_plan_missing")?;
    let intent = plan.intent.clone();
    evidence.plan_requires_native_update = Some(plan.requires_native_update());
    if plan.requires_native_update() != (manifest.expected != "channel_only") {
        return Err("native_plan_mismatch");
    }
    // 检查阶段也必须保持配置；不能把探测副作用当成升级前基线。
    if config_snapshot(root)? != before {
        return Err("inspect_changed_configuration");
    }
    evidence.stage = "execute";
    let mut original_link = None;
    let mut write_block = None;
    if manifest.expected == "source_changed_rejected" {
        original_link = Some(fs::read_link(&manifest.entry).map_err(|_| "entry_not_symlink")?);
        fs::remove_file(&manifest.entry).map_err(|_| "source_change_failed")?;
        symlink(&manifest.target_binary.path, &manifest.entry)
            .map_err(|_| "source_change_failed")?;
    } else if manifest.expected == "command_failed_rolled_back" {
        write_block = Some(WriteBlock::new(root, agent)?);
    }
    evidence.product_execute_calls += 1;
    let executed = if manifest.expected == "interrupted_recovered" {
        let task = tokio::spawn(async move { execute(plan, None).await });
        let generation = wait_for_native_start(root, agent).await?;
        if task.is_finished() {
            return Err("native_update_completed_before_interrupt");
        }
        task.abort();
        let _ = task.await;
        wait_for_confirmed_exit(root, generation).await?;
        Ok(manifest.old_version.clone())
    } else {
        execute(plan, None).await
    };
    if let Some(write_block) = write_block {
        write_block.restore()?;
    }
    if let Some(original) = original_link {
        // 仅恢复仍指向本夹具目标的入口，不能覆盖意外的并发修改。
        if fs::read_link(&manifest.entry).ok().as_ref() != Some(&manifest.target_binary.path) {
            return Err("source_change_restore_conflict");
        }
        fs::remove_file(&manifest.entry).map_err(|_| "source_change_restore_failed")?;
        symlink(original, &manifest.entry).map_err(|_| "source_change_restore_failed")?;
    }
    let expected_version = match manifest.expected.as_str() {
        "updated" | "channel_only" => {
            if executed.as_ref().ok() != Some(&manifest.target_version) {
                evidence.error = executed.err().map(|error| format!("{error:?}"));
                return Err("execute_failed");
            }
            &manifest.target_version
        }
        "source_changed_rejected" => {
            if executed != Err(Error::SourceChanged) {
                return Err("source_change_not_rejected");
            }
            evidence.error = Some("SourceChanged".to_owned());
            &manifest.old_version
        }
        "command_failed_rolled_back" => {
            if executed != Err(Error::CommandFailed) {
                evidence.error = executed.err().map(|error| format!("{error:?}"));
                return Err("command_failure_not_rolled_back");
            }
            evidence.error = Some("CommandFailed".to_owned());
            &manifest.old_version
        }
        "interrupted_recovered" => &manifest.old_version,
        _ => return Err("invalid_expectation"),
    };
    evidence.stage = "inspect_after";
    evidence.product_inspect_calls += 1;
    let report = inspect(agent, Some(manifest.entry.clone()), channel, &client)
        .await
        .map_err(|error| {
            evidence.error = Some(format!("{error:?}"));
            "inspect_after_failed"
        })?;
    let failure_expected = matches!(
        manifest.expected.as_str(),
        "command_failed_rolled_back" | "interrupted_recovered"
    );
    let persisted = previous_failure_for_intent(
        &journal_root().map_err(|_| "state_unavailable")?,
        agent,
        &manifest.entry,
        &intent,
        false,
    )
    .map_err(|_| "failure_intent_not_persisted")?;
    evidence.failure_intent_persisted = report.failed_target.as_deref()
        == failure_expected.then_some(manifest.target_version.as_str())
        && persisted.as_deref() == failure_expected.then_some(manifest.target_version.as_str());
    if !evidence.failure_intent_persisted {
        return Err("failure_intent_not_persisted");
    }
    evidence.post_version_matches = &report.installed_version == expected_version
        && report.latest_version == manifest.target_version
        && report.source == Source::Native
        && report.error.is_none()
        && report.up_to_date == matches!(manifest.expected.as_str(), "updated" | "channel_only");
    evidence.entry_matches_expected = binary_sha(&manifest.entry)?
        == if matches!(manifest.expected.as_str(), "updated" | "channel_only") {
            manifest.target_binary.sha256.clone()
        } else {
            manifest.old_binary.sha256.clone()
        };
    evidence.entry_unchanged = fs::read_link(&manifest.entry).ok().as_ref()
        == Some(&original_entry)
        && manifest.entry.canonicalize().ok().as_ref() == Some(&manifest.old_binary.path)
        && binary_sha(&manifest.entry)?.as_str() == manifest.old_binary.sha256;
    evidence.supervisor_generations_unchanged = supervisor_generations(root)? == generations_before;
    if matches!(
        manifest.expected.as_str(),
        "command_failed_rolled_back" | "interrupted_recovered"
    ) && evidence.supervisor_generations_unchanged
    {
        return Err("native_update_not_started");
    }
    if matches!(
        manifest.expected.as_str(),
        "channel_only" | "command_failed_rolled_back" | "interrupted_recovered"
    ) && (!evidence.entry_unchanged || !evidence.supervisor_generations_unchanged)
    {
        if manifest.expected == "channel_only" || !evidence.entry_unchanged {
            return Err("entry_changed_unexpectedly");
        }
    }
    evidence.production_chain_verified = true;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "仅由 verify_cli_autoupdate.py 显式授权私有原生升级；不接受凭据或模型输入"]
async fn real_native_update_without_model() {
    // 所有失败只显示稳定阶段名；配置、环境值和原生命令输出均不写入测试日志。
    let path = env::var_os("INFINISHELL_CLI_AUTOUPDATE_MANIFEST")
        .map(PathBuf::from)
        .expect("必须使用专用验收驱动");
    let bytes = private_file(&path, 0o600).expect("私有清单验证失败");
    let manifest: Manifest = serde_json::from_slice(&bytes).expect("私有清单结构无效");
    assert!(
        manifest.schema == 1
            && manifest.scope == SCOPE
            && (30..=600).contains(&manifest.timeout_seconds)
            && !manifest.case_id.is_empty()
            && manifest.case_id.len() <= 64
            && manifest
                .case_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && matches!(
                manifest.expected.as_str(),
                "updated"
                    | "source_changed_rejected"
                    | "channel_only"
                    | "command_failed_rolled_back"
                    | "interrupted_recovered"
            )
            && parse_version(&manifest.old_version).is_ok()
            && parse_version(&manifest.target_version).is_ok()
            && (manifest.old_version == manifest.target_version)
                == (manifest.expected == "channel_only")
            && transition_valid(&manifest),
        "清单取值无效"
    );
    agent_channel(&manifest).expect("渠道无效");
    verify_environment(&manifest, &path).expect("隔离环境无效");
    verify_build_binding(&manifest).expect("测试程序与监督程序必须来自同一已通过门禁的源码清单");
    assert!(under(&manifest.root, &manifest.entry), "入口越界");
    assert!(
        manifest.entry.is_symlink()
            && if manifest.expected == "channel_only" {
                manifest.old_binary.path == manifest.target_binary.path
                    && manifest.old_binary.sha256 == manifest.target_binary.sha256
            } else {
                manifest.old_binary.path != manifest.target_binary.path
                    && manifest.old_binary.sha256 != manifest.target_binary.sha256
            },
        "原生入口与旧新文件必须独立绑定"
    );
    plain_path(
        &manifest.root,
        manifest.entry.parent().expect("入口无父目录"),
    )
    .expect("入口父目录越界");
    assert!(
        manifest.entry.canonicalize().ok().as_ref() == Some(&manifest.old_binary.path),
        "入口未绑定旧文件"
    );
    for binary in [&manifest.old_binary, &manifest.target_binary] {
        plain_path(&manifest.root, &binary.path).expect("原生文件越界");
        assert!(
            binary_sha(&binary.path).ok().as_deref() == Some(binary.sha256.as_str()),
            "原生文件摘要不符"
        );
    }
    assert!(
        env::current_exe()
            .ok()
            .and_then(|path| path.canonicalize().ok())
            == manifest.worker.path.canonicalize().ok()
            && binary_sha(&manifest.worker.path).ok().as_deref()
                == Some(manifest.worker.sha256.as_str())
            && binary_sha(&manifest.source_manifest.path).ok().as_deref()
                == Some(manifest.source_manifest.sha256.as_str()),
        "验证输入绑定不符"
    );
    let before = config_snapshot(&manifest.root).expect("配置边界无效");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(manifest.root.join("events.safe.json"))
        .expect("验收收据已存在或不可写");
    let mut evidence = Evidence {
        schema: 1,
        scope: SCOPE,
        case_id: manifest.case_id.clone(),
        agent: manifest.agent.clone(),
        channel: manifest.channel.clone(),
        expected: manifest.expected.clone(),
        stage: "started",
        passed: false,
        error: None,
        failure_code: None,
        manifest_sha256: sha(&bytes),
        worker_sha256: manifest.worker.sha256.clone(),
        supervisor_sha256: manifest.supervisor.sha256.clone(),
        source_manifest_sha256: manifest.source_manifest.sha256.clone(),
        old_sha256: manifest.old_binary.sha256.clone(),
        target_sha256: manifest.target_binary.sha256.clone(),
        old_version: manifest.old_version.clone(),
        target_version: manifest.target_version.clone(),
        product_inspect_calls: 0,
        product_execute_calls: 0,
        model_inputs_sent: 0,
        credentials_provided: false,
        private_environment_verified: true,
        credential_files_absent_before: true,
        credential_files_absent_after: false,
        config_files_checked: before.len(),
        config_bytes_unchanged: false,
        config_semantics_unchanged: false,
        config_permissions_unchanged: false,
        config_transition: manifest.config_transition.clone(),
        config_transition_verified: false,
        unrelated_config_bytes_unchanged: false,
        config_permissions_preserved: false,
        plan_requires_native_update: None,
        supervisor_generations_unchanged: false,
        entry_unchanged: false,
        old_binary_unchanged: false,
        target_reference_unchanged: false,
        entry_matches_expected: false,
        post_version_matches: false,
        journal_absent: false,
        production_chain_verified: false,
        same_source_build_verified: true,
        same_commit_verified_by_runner: false,
        failure_intent_persisted: false,
    };
    let result = exercise(&manifest, &mut evidence).await;
    evidence.failure_code = result.as_ref().err().copied();
    if let Ok(after) = config_snapshot(&manifest.root) {
        (
            evidence.config_transition_verified,
            evidence.unrelated_config_bytes_unchanged,
            evidence.config_permissions_preserved,
        ) = verify_config_transition(&manifest, &before, &after);
        evidence.config_bytes_unchanged = before.iter().all(|(path, value)| {
            after
                .get(path)
                .is_some_and(|after| value.bytes == after.bytes)
        });
        evidence.config_semantics_unchanged = before.iter().all(|(path, value)| {
            after
                .get(path)
                .is_some_and(|after| semantic_equal(path, value, after))
        });
        evidence.config_permissions_unchanged = before.iter().all(|(path, value)| {
            after
                .get(path)
                .is_some_and(|after| value.mode == after.mode)
        });
    }
    if result.is_ok() && !evidence.config_transition_verified {
        evidence.failure_code = Some("config_transition_mismatch");
    }
    evidence.old_binary_unchanged = binary_sha(&manifest.old_binary.path).ok().as_deref()
        == Some(manifest.old_binary.sha256.as_str());
    evidence.target_reference_unchanged = binary_sha(&manifest.target_binary.path).ok().as_deref()
        == Some(manifest.target_binary.sha256.as_str());
    evidence.credential_files_absent_after = auth_absent(&manifest.root);
    evidence.journal_absent = journal_root().is_ok_and(|path| {
        fs::symlink_metadata(path.join(format!("{}.json", manifest.agent)))
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    });
    evidence.passed = result.is_ok()
        && evidence.config_transition_verified
        && evidence.unrelated_config_bytes_unchanged
        && evidence.config_permissions_preserved
        && evidence.old_binary_unchanged
        && evidence.target_reference_unchanged
        && evidence.credential_files_absent_after
        && evidence.journal_absent
        && evidence.entry_matches_expected
        && evidence.post_version_matches
        && evidence.production_chain_verified
        && evidence.failure_intent_persisted;
    if result.is_ok() {
        evidence.stage = "finished";
    }
    let passed = evidence.passed;
    serde_json::to_writer(&mut file, &evidence).expect("安全收据写入失败");
    writeln!(file).expect("安全收据写入失败");
    file.sync_all().expect("安全收据写入失败");
    assert!(passed, "升级验收未通过，详情仅记录于安全阶段收据");
}

fn transition_fixture(
    agent: &str,
    channel: &str,
    transition: Option<ConfigTransition>,
) -> Manifest {
    let binary = || Binary {
        path: PathBuf::from("/fixture/reference"),
        sha256: "a".repeat(64),
    };
    Manifest {
        schema: 1,
        scope: SCOPE.to_owned(),
        case_id: "offline".to_owned(),
        root: PathBuf::from("/fixture"),
        agent: agent.to_owned(),
        channel: channel.to_owned(),
        expected: "updated".to_owned(),
        old_version: "0.147.0".to_owned(),
        target_version: "0.148.0".to_owned(),
        entry: PathBuf::from("/fixture/home/.local/bin/codex"),
        old_binary: Binary {
            path: PathBuf::from(
                "/fixture/home/.codex/packages/standalone/releases/0.147.0-aarch64-apple-darwin/bin/codex",
            ),
            sha256: "a".repeat(64),
        },
        target_binary: binary(),
        worker: binary(),
        supervisor: binary(),
        source_manifest: binary(),
        gates_report: binary(),
        bundle_report: binary(),
        timeout_seconds: 480,
        config_transition: transition,
    }
}

fn snapshot(bytes: Option<&str>) -> ConfigSnapshot {
    ConfigSnapshot {
        bytes: bytes.map(|bytes| bytes.as_bytes().to_vec()),
        mode: bytes.map(|_| 0o600),
    }
}

#[test]
fn compiled_source_binding_rejects_a_forged_manifest_digest() {
    let files = COMPILED_SOURCE_FILES
        .iter()
        .map(|(path, bytes)| {
            serde_json::json!({
                "path": path,
                "bytes": bytes.len(),
                "sha256": sha(bytes),
            })
        })
        .collect::<Vec<_>>();
    let mut source = serde_json::json!({"files": files});
    assert!(compiled_source_files_match(&source));

    source["files"][0]["sha256"] = Value::String("0".repeat(64));
    assert!(!compiled_source_files_match(&source));
}

#[test]
fn binary_source_binding_matches_across_read_boundaries() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("supervisor");
    let mut bytes = vec![b'x'; 1024 * 1024 - 2];
    bytes.extend_from_slice(b"source-one-middle-source-two");
    fs::write(&path, bytes).unwrap();

    assert_eq!(
        binary_contains_all(&path, &[b"source-one".as_slice(), b"source-two".as_slice()]),
        Ok(true)
    );
    assert_eq!(
        binary_contains_all(&path, &[b"source-one".as_slice(), b"missing".as_slice()]),
        Ok(false)
    );
}

#[test]
fn channel_transition_preserves_other_json_values_and_rejects_permission_changes() {
    let manifest = transition_fixture(
        "claude",
        "latest",
        Some(ConfigTransition::Channel {
            before: Some("stable".to_owned()),
            after: "latest".to_owned(),
        }),
    );
    let before = BTreeMap::from([
        (
            "home/.claude/settings.json",
            snapshot(Some(
                r#"{"autoUpdatesChannel":"stable","permissions":{"defaultMode":"default"},"other":7}"#,
            )),
        ),
        ("home/.profile", snapshot(Some("original shell config"))),
    ]);
    let mut after = before.clone();
    after.insert(
        "home/.claude/settings.json",
        snapshot(Some(
            r#"{"autoUpdatesChannel":"latest","permissions":{"defaultMode":"default"},"other":7}"#,
        )),
    );
    assert_eq!(
        verify_config_transition(&manifest, &before, &after),
        (true, true, true)
    );
    after.get_mut("home/.claude/settings.json").unwrap().mode = Some(0o644);
    assert_eq!(
        verify_config_transition(&manifest, &before, &after),
        (false, true, false)
    );
}

#[test]
fn channel_transition_rejects_other_fields_and_concurrent_shell_saves() {
    let manifest = transition_fixture(
        "claude",
        "latest",
        Some(ConfigTransition::Channel {
            before: Some("stable".to_owned()),
            after: "latest".to_owned(),
        }),
    );
    let before = BTreeMap::from([
        (
            "home/.claude/settings.json",
            snapshot(Some(
                r#"{"autoUpdatesChannel":"stable","permissions":{"defaultMode":"default"}}"#,
            )),
        ),
        ("home/.profile", snapshot(Some("original shell config"))),
    ]);
    let after = BTreeMap::from([
        (
            "home/.claude/settings.json",
            snapshot(Some(
                r#"{"autoUpdatesChannel":"latest","permissions":{"defaultMode":"bypassPermissions"}}"#,
            )),
        ),
        ("home/.profile", snapshot(Some("original shell config"))),
    ]);
    assert_eq!(
        verify_config_transition(&manifest, &before, &after),
        (false, true, true)
    );
    let after = BTreeMap::from([
        (
            "home/.claude/settings.json",
            snapshot(Some(
                r#"{"autoUpdatesChannel":"latest","permissions":{"defaultMode":"default"}}"#,
            )),
        ),
        ("home/.profile", snapshot(Some("concurrent user save"))),
    ]);
    assert_eq!(
        verify_config_transition(&manifest, &before, &after),
        (false, false, false)
    );
}

#[test]
fn grok_channel_transition_rejects_ui_defaults_and_auto_update_changes() {
    let before = snapshot(Some(
        "[cli]\nchannel='stable'\nauto_update=false\n[ui]\nhide_banner=true\n",
    ));
    let after = snapshot(Some(
        "[cli]\nchannel='alpha'\nauto_update=false\n[ui]\nhide_banner=true\n",
    ));
    assert!(channel_config_matches(
        "grok",
        &before,
        &after,
        Some("stable"),
        "alpha"
    ));
    let changed = snapshot(Some(
        "[cli]\nchannel='alpha'\nauto_update=true\n[ui]\nhide_banner=true\n",
    ));
    assert!(!channel_config_matches(
        "grok",
        &before,
        &changed,
        Some("stable"),
        "alpha"
    ));
    let changed = snapshot(Some(
        "[cli]\nchannel='alpha'\nauto_update=false\n[ui]\nhide_banner=true\nshow_diff=true\n",
    ));
    assert!(!channel_config_matches(
        "grok",
        &before,
        &changed,
        Some("stable"),
        "alpha"
    ));
}

#[test]
fn absent_config_allows_only_minimal_selected_channel_with_private_mode() {
    let manifest = transition_fixture(
        "grok",
        "alpha",
        Some(ConfigTransition::Channel {
            before: None,
            after: "alpha".to_owned(),
        }),
    );
    let before = BTreeMap::from([("home/.grok/config.toml", snapshot(None))]);
    let mut after = BTreeMap::from([(
        "home/.grok/config.toml",
        snapshot(Some("[cli]\nchannel='alpha'\n")),
    )]);
    assert_eq!(
        verify_config_transition(&manifest, &before, &after),
        (true, true, true)
    );
    after.get_mut("home/.grok/config.toml").unwrap().mode = Some(0o644);
    assert_eq!(
        verify_config_transition(&manifest, &before, &after),
        (false, true, false)
    );
    assert!(!channel_config_matches(
        "grok",
        &snapshot(None),
        &snapshot(Some("[cli]\nchannel='alpha'\ninstaller='internal'\n")),
        None,
        "alpha"
    ));
    assert!(channel_config_matches(
        "claude",
        &snapshot(None),
        &snapshot(Some(r#"{"autoUpdatesChannel":"stable"}"#)),
        None,
        "stable"
    ));
}

#[test]
fn reverse_channel_switch_preserves_custom_file_modes_and_other_values() {
    let manifest = transition_fixture(
        "claude",
        "stable",
        Some(ConfigTransition::Channel {
            before: Some("latest".to_owned()),
            after: "stable".to_owned(),
        }),
    );
    let before = BTreeMap::from([(
        "home/.claude/settings.json",
        ConfigSnapshot {
            bytes: Some(br#"{"autoUpdatesChannel":"latest","other":1}"#.to_vec()),
            mode: Some(0o640),
        },
    )]);
    let after = BTreeMap::from([(
        "home/.claude/settings.json",
        ConfigSnapshot {
            bytes: Some(br#"{"autoUpdatesChannel":"stable","other":1}"#.to_vec()),
            mode: Some(0o640),
        },
    )]);
    assert_eq!(
        verify_config_transition(&manifest, &before, &after),
        (true, true, true)
    );
    assert!(channel_config_matches(
        "grok",
        &snapshot(Some("[cli]\nchannel='alpha'\nauto_update=false")),
        &snapshot(Some("[cli]\nchannel='stable'\nauto_update=false")),
        Some("alpha"),
        "stable"
    ));
}

#[test]
fn channel_baseline_rejects_unknown_values_and_wrong_table_shapes() {
    assert!(!channel_config_matches(
        "claude",
        &snapshot(Some(r#"{"autoUpdatesChannel":false}"#)),
        &snapshot(Some(r#"{"autoUpdatesChannel":"latest"}"#)),
        None,
        "latest"
    ));
    assert!(!channel_config_matches(
        "grok",
        &snapshot(Some("cli='alpha'")),
        &snapshot(Some("[cli]\nchannel='alpha'")),
        None,
        "alpha"
    ));
    assert!(!channel_config_matches(
        "claude",
        &snapshot(Some(r#"{"autoUpdatesChannel":"latest"}"#)),
        &snapshot(Some(r#"{"autoUpdatesChannel":"stable"}"#)),
        Some("stable"),
        "stable"
    ));
}

#[test]
fn follow_rejected_failed_and_interrupted_updates_cannot_declare_channel_mutations() {
    let mut manifest = transition_fixture(
        "claude",
        "follow_installation",
        Some(ConfigTransition::Channel {
            before: Some("stable".to_owned()),
            after: "latest".to_owned(),
        }),
    );
    assert!(!transition_valid(&manifest));
    manifest.channel = "latest".to_owned();
    manifest.expected = "source_changed_rejected".to_owned();
    assert!(!transition_valid(&manifest));
    manifest.expected = "command_failed_rolled_back".to_owned();
    assert!(!transition_valid(&manifest));
    manifest.expected = "interrupted_recovered".to_owned();
    assert!(!transition_valid(&manifest));
    manifest.expected = "source_changed_rejected".to_owned();
    manifest.config_transition = None;
    let before = BTreeMap::from([("home/.claude/settings.json", snapshot(Some("{}")))]);
    let after = BTreeMap::from([("home/.claude/settings.json", snapshot(Some("{ }")))]);
    assert_eq!(
        verify_config_transition(&manifest, &before, &after),
        (false, false, false)
    );
    assert_eq!(
        verify_config_transition(&manifest, &before, &before),
        (true, true, true)
    );
}

#[test]
fn codex_stable_marker_follows_only_an_exact_enabled_original_release() {
    let manifest = transition_fixture("codex", "latest", None);
    let old = snapshot(Some("0.147.0-aarch64-apple-darwin"));
    let target = snapshot(Some("0.148.0-aarch64-apple-darwin"));
    assert!(codex_marker_matches(&manifest, &old, &target));
    assert!(!codex_marker_matches(
        &manifest,
        &snapshot(Some("0.147.0-aarch64-apple-darwin\n")),
        &target
    ));
    assert!(!codex_marker_matches(&manifest, &snapshot(None), &target));
    assert!(codex_marker_matches(
        &manifest,
        &snapshot(None),
        &snapshot(None)
    ));
    assert!(codex_marker_matches(
        &manifest,
        &snapshot(Some("invalid-old-marker")),
        &snapshot(Some("invalid-old-marker"))
    ));
}

#[test]
fn codex_alpha_removes_marker_and_returning_to_stable_keeps_daemon_disabled() {
    let mut manifest = transition_fixture("codex", "alpha", None);
    manifest.target_version = "0.148.0-alpha.2".to_owned();
    let old = snapshot(Some("0.147.0-aarch64-apple-darwin"));
    assert!(codex_marker_matches(&manifest, &old, &snapshot(None)));
    assert!(!codex_marker_matches(&manifest, &old, &old));
    manifest.old_version = "0.148.0-alpha.2".to_owned();
    manifest.old_binary.path = PathBuf::from(
        "/fixture/home/.codex/packages/standalone/releases/0.148.0-alpha.2-aarch64-apple-darwin/bin/codex",
    );
    manifest.target_version = "0.147.0".to_owned();
    assert!(codex_marker_matches(
        &manifest,
        &snapshot(None),
        &snapshot(None)
    ));
    assert!(!codex_marker_matches(&manifest, &snapshot(None), &old));
}

#[test]
fn codex_invalid_old_marker_matching_new_release_is_removed_without_activation() {
    let manifest = transition_fixture("codex", "latest", None);
    let target = snapshot(Some("0.148.0-aarch64-apple-darwin"));
    assert!(codex_marker_matches(&manifest, &target, &snapshot(None)));
    assert!(!codex_marker_matches(&manifest, &target, &target));
}

#[test]
fn marker_transition_binds_both_hashes_and_preserves_all_other_paths() {
    let old = "0.147.0-aarch64-apple-darwin";
    let target = "0.148.0-aarch64-apple-darwin";
    let manifest = transition_fixture(
        "codex",
        "latest",
        Some(ConfigTransition::CodexMarker {
            before_sha256: Some(sha(old.as_bytes())),
            after_sha256: Some(sha(target.as_bytes())),
        }),
    );
    let before = BTreeMap::from([(
        "home/.codex/packages/standalone/auto-update-version",
        snapshot(Some(old)),
    )]);
    let after = BTreeMap::from([(
        "home/.codex/packages/standalone/auto-update-version",
        snapshot(Some(target)),
    )]);
    assert_eq!(
        verify_config_transition(&manifest, &before, &after),
        (true, true, true)
    );
    let changed = BTreeMap::from([(
        "home/.codex/packages/standalone/auto-update-version",
        snapshot(Some("0.148.0-x86_64-apple-darwin")),
    )]);
    assert_eq!(
        verify_config_transition(&manifest, &before, &changed),
        (false, true, true)
    );
}

#[cfg(test)]
#[path = "sources_live_environment_tests.rs"]
mod environment_tests;
