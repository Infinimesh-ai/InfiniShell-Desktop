//! Windows 固定官方输入的零模型产品事务；仅本测试作用域固定发行输入和私有状态目录。

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{Channel, Journal, Source, VerificationProgress, execute, inspect};
use crate::ai::cli_agent_runtime::managed_process::{self, ExitReason};
use crate::terminal::cli_agent::CLIAgent;

const SCOPE: &str = "cli_autoupdate_windows_native_product";
const MARKER: &[u8] = b"InfiniShell private updater fixture; no credentials or model inputs\n";
const SOURCES: [(&str, &[u8]); 4] = [
    (
        "app/src/terminal/cli_agent_updates.rs",
        include_bytes!("../cli_agent_updates.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources.rs",
        include_bytes!("sources.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process_atomic_windows.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_windows.rs"),
    ),
];
const CONFIGS: [&str; 13] = [
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
];
const AUTH: [&str; 6] = [
    "home/.codex/auth.json",
    "home/.grok/auth.json",
    "home/.claude/.credentials.json",
    "home/.claude/credentials.json",
    "home/.netrc",
    "home/.npmrc",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binary {
    path: PathBuf,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    scope: String,
    root: PathBuf,
    agent: String,
    entry: PathBuf,
    old_version: String,
    target_version: String,
    old_binary: Binary,
    target_binary: Binary,
    worker: Binary,
    supervisor: Binary,
    sources: BTreeMap<String, String>,
    configs: BTreeMap<String, Option<String>>,
}

tokio::task_local! { static FIXTURE: (CLIAgent, String, PathBuf); }

pub(super) fn fixed_release(agent: CLIAgent, channel: Channel) -> Option<String> {
    FIXTURE
        .try_with(|(selected, version, _)| {
            (*selected == agent && matches!(channel, Channel::Latest | Channel::Stable))
                .then(|| version.clone())
        })
        .ok()
        .flatten()
}

pub(super) fn journal_root() -> Option<PathBuf> {
    FIXTURE
        .try_with(|(_, _, root)| root.join("state/cli-agent-updates"))
        .ok()
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn digest(path: &Path) -> Result<String, &'static str> {
    let mut file = File::open(path).map_err(|_| "binary_missing")?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "binary_unreadable")?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn inside(root: &Path, path: &Path) -> Result<(), &'static str> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "fixture_path_outside")?;
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("fixture_path_outside");
    }
    // 可见 Codex 入口的两层官方联接单独由来源发现验证；私有记录不允许重解析。
    super::plain_ancestors(path).map_err(|_| "fixture_path_reparse")
}

fn configs(root: &Path) -> Result<BTreeMap<String, Option<String>>, &'static str> {
    CONFIGS
        .into_iter()
        .map(|name| {
            let path = root.join(name);
            inside(root, &path)?;
            let value = if path.try_exists().map_err(|_| "config_unreadable")? {
                Some(digest(&path)?)
            } else {
                None
            };
            Ok((name.to_owned(), value))
        })
        .collect()
}

fn auth_absent(root: &Path) -> bool {
    AUTH.iter().all(|name| {
        fs::symlink_metadata(root.join(name))
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    })
}

fn supervisor_has_sources(path: &Path) -> Result<bool, &'static str> {
    let mut file = File::open(path).map_err(|_| "supervisor_missing")?;
    let overlap = SOURCES.iter().map(|(_, bytes)| bytes.len()).max().unwrap();
    let mut found = [false; SOURCES.len()];
    let mut pending = Vec::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "supervisor_unreadable")?;
        if count == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..count]);
        for (index, (_, bytes)) in SOURCES.iter().enumerate() {
            if !found[index] && pending.windows(bytes.len()).any(|part| part == *bytes) {
                found[index] = true;
            }
        }
        if found.iter().all(|found| *found) {
            return Ok(true);
        }
        if pending.len() > overlap {
            pending.drain(..pending.len() - overlap);
        }
    }
    Ok(false)
}

fn fixed_contract(manifest: &Manifest) -> Option<CLIAgent> {
    match (
        manifest.agent.as_str(),
        manifest.old_version.as_str(),
        manifest.target_version.as_str(),
        manifest.old_binary.sha256.as_str(),
        manifest.target_binary.sha256.as_str(),
    ) {
        (
            "codex",
            "0.155.1",
            "0.156.1",
            "eba0f32c976667cb9298efafd98513e823eeda7b576a03ec658bb8be8d336316",
            "70bcb05f9bf1a4e7306edd0cd1b57d02af3267ad02a34b26f45c8c4bb20a3301",
        ) => Some(CLIAgent::Codex),
        (
            "claude",
            "2.1.278",
            "2.1.280",
            "006ea5c8638f67f10a5ae66bb232fd267c9f6af294e3f03f4cfcf1fd3f2cced8",
            "0e4195524b73eb77efbdf3e2b36de5322a29f0ca575dfd2d9b4f946b1d425469",
        ) => Some(CLIAgent::Claude),
        (
            "grok",
            "1.0.40",
            "1.0.41",
            "034c883fa3962ab6ca409c2d3c7501c642166535dd39fa936ecffe1ac2cad92e",
            "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25",
        ) => Some(CLIAgent::Grok),
        _ => None,
    }
}

// Python 准备器的原生隔离开关必须保留，且只认可关闭自动行为的固定值。
fn valid_native_isolation_switch(key: &str, value: &OsStr) -> bool {
    matches!(
        (key, value.to_str()),
        ("GROK_DISABLE_AUTOUPDATER", Some("1"))
            | (
                "GROK_AUTO_UPDATE"
                    | "GROK_CLAUDE_HOOKS_ENABLED"
                    | "GROK_CLAUDE_MCPS_ENABLED"
                    | "GROK_CODEX_HOOKS_ENABLED"
                    | "GROK_CODEX_MCPS_ENABLED",
                Some("0")
            )
    )
}

fn validate(manifest: &Manifest, path: &Path) -> Result<CLIAgent, &'static str> {
    let root = &manifest.root;
    if manifest.schema != 1
        || manifest.scope != SCOPE
        || !root.is_absolute()
        || !root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("infinishell-cli-autoupdate-"))
        || path != &root.join("manifest.windows.private.json")
        || fs::read(root.join(".infinishell-cli-autoupdate"))
            .ok()
            .as_deref()
            != Some(MARKER)
        || env::var("INFINISHELL_CLI_AUTOUPDATE_ALLOW").as_deref() != Ok("native-update-no-model")
    {
        return Err("fixture_contract");
    }
    inside(root, path)?;
    for (key, relative) in [
        ("HOME", "home"),
        ("USERPROFILE", "home"),
        ("APPDATA", "home/AppData/Roaming"),
        ("LOCALAPPDATA", "home/AppData/Local"),
        ("CODEX_HOME", "home/.codex"),
        ("CLAUDE_CONFIG_DIR", "home/.claude"),
        ("GROK_HOME", "home/.grok"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
        ("TMPDIR", "tmp"),
    ] {
        if env::var_os(key).map(PathBuf::from) != Some(root.join(relative)) {
            return Err("environment_path");
        }
        inside(root, &root.join(relative))?;
    }
    for (key, value) in env::vars_os() {
        let key = key.to_string_lossy().to_ascii_uppercase();
        if !matches!(
            key.as_str(),
            "HOME"
                | "USERPROFILE"
                | "APPDATA"
                | "LOCALAPPDATA"
                | "SYSTEMROOT"
                | "WINDIR"
                | "COMSPEC"
                | "PATHEXT"
                | "PATH"
                | "LANG"
                | "LC_ALL"
                | "CODEX_HOME"
                | "CLAUDE_CONFIG_DIR"
                | "GROK_HOME"
                | "XDG_CONFIG_HOME"
                | "XDG_DATA_HOME"
                | "XDG_CACHE_HOME"
                | "TMP"
                | "TEMP"
                | "TMPDIR"
                | "CODEX_INSTALL_DIR"
                | "CODEX_RELEASE"
                | "CODEX_NON_INTERACTIVE"
                | "DISABLE_AUTOUPDATER"
                | "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"
                | "INFINISHELL_CLI_AUTOUPDATE_ALLOW"
                | "INFINISHELL_CLI_AUTOUPDATE_MANIFEST"
                | "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE"
        ) && !valid_native_isolation_switch(&key, &value)
        {
            return Err("environment_not_isolated");
        }
    }
    if !auth_absent(root)
        || configs(root)? != manifest.configs
        || env::current_dir().ok().and_then(|p| p.canonicalize().ok())
            != root.join("project").canonicalize().ok()
        || env::var_os("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE").map(PathBuf::from)
            != Some(manifest.supervisor.path.clone())
    {
        return Err("configuration_or_environment_binding");
    }
    for binary in [&manifest.old_binary, &manifest.target_binary] {
        inside(root, &binary.path)?;
    }
    if !manifest.entry.starts_with(root.join("home"))
        || digest(&manifest.entry)? != manifest.old_binary.sha256
    {
        return Err("entry_binding");
    }
    for binary in [
        &manifest.old_binary,
        &manifest.target_binary,
        &manifest.worker,
        &manifest.supervisor,
    ] {
        if digest(&binary.path)? != binary.sha256 {
            return Err("binary_binding");
        }
    }
    if env::current_exe().ok().and_then(|p| p.canonicalize().ok())
        != manifest.worker.path.canonicalize().ok()
        || SOURCES
            .iter()
            .any(|(name, bytes)| manifest.sources.get(*name) != Some(&sha(bytes)))
        || manifest
            .sources
            .get("app/src/terminal/cli_agent_updates/sources_windows_live_tests.rs")
            != Some(&sha(include_bytes!("sources_windows_live_tests.rs")))
        || manifest
            .sources
            .get("script/cli-agent-parity/run_fixed_cli_autoupdate.py")
            != Some(&sha(include_bytes!(
                "../../../../script/cli-agent-parity/run_fixed_cli_autoupdate.py"
            )))
        || !supervisor_has_sources(&manifest.supervisor.path)?
    {
        return Err("build_binding");
    }
    fixed_contract(manifest).ok_or("fixed_release_binding")
}

#[derive(Default, Serialize)]
struct Evidence {
    schema: u32,
    scope: &'static str,
    agent: String,
    old_version: String,
    target_version: String,
    manifest_sha256: String,
    worker_sha256: String,
    supervisor_sha256: String,
    old_sha256: String,
    target_sha256: String,
    fixed_release_input: bool,
    capability_or_source_gate_bypassed: bool,
    isolated_journal_root_for_test: bool,
    credentials_provided: bool,
    model_inputs_sent: u32,
    product_inspect_calls: u32,
    product_execute_calls: u32,
    supervisor_exit_reason: Option<ExitReason>,
    supervisor_exit_code: Option<i32>,
    native_exit_confirmed: bool,
    strict_job_cleanup_confirmed: bool,
    entry_matches_target: bool,
    target_version_matches: bool,
    config_files_checked: usize,
    config_bytes_unchanged: bool,
    journal_absent: bool,
    old_binary_unchanged: bool,
    target_reference_unchanged: bool,
    credentials_absent: bool,
    error: Option<String>,
    failure_code: Option<&'static str>,
    passed: bool,
}

async fn exercise(
    manifest: &Manifest,
    agent: CLIAgent,
    evidence: &mut Evidence,
) -> Result<(), &'static str> {
    let client = Arc::new(http_client::Client::new());
    evidence.product_inspect_calls += 1;
    let before = inspect(
        agent,
        Some(manifest.entry.clone()),
        Channel::FollowInstallation,
        &client,
    )
    .await
    .map_err(|error| {
        evidence.error = Some(format!("{error:?}"));
        "inspect_before_failed"
    })?;
    if before.source != Source::Native
        || before.error.is_some()
        || before.up_to_date
        || before.installed_version != manifest.old_version
        || before.latest_version != manifest.target_version
    {
        evidence.error = before.error.map(|error| format!("{error:?}"));
        return Err("native_plan_mismatch");
    }
    let plan = before.plan.ok_or("native_plan_missing")?;
    if !plan.requires_native_update() || configs(&manifest.root)? != manifest.configs {
        return Err("inspect_changed_configuration");
    }
    let (entered_sender, entered_receiver) = async_channel::bounded(1);
    let (resume_sender, resume_receiver) = async_channel::bounded(1);
    let progress = VerificationProgress::new(entered_sender, resume_receiver);
    evidence.product_execute_calls += 1;
    let observe = async {
        if entered_receiver.recv().await.is_err() {
            return None;
        }
        let state = journal_root()?;
        let receipt = (|| {
            let journal: Journal = serde_json::from_slice(
                &fs::read(state.join(format!("{}.json", manifest.agent))).ok()?,
            )
            .ok()?;
            let binding = super::journal_binding(&journal).ok()?;
            managed_process::confirmed_exit_with_binding(&state, journal.generation?, &binding)
                .ok()
                .flatten()
        })();
        let _ = resume_sender.send(()).await;
        receipt
    };
    let (executed, receipt) = futures::join!(execute(plan, Some(progress)), observe);
    evidence.supervisor_exit_reason = receipt.as_ref().map(|receipt| receipt.exit_reason);
    evidence.supervisor_exit_code = receipt.as_ref().and_then(|receipt| receipt.exit_code);
    // 输出 EOF 可以先于根进程退出被观察到；监督回执保留最初的 StdioClosed 原因。
    // Windows 执行 worker 仅在调试会话确认原生 exit 0 后成功返回，Job 强杀使用 exit 1。
    // 因此仍要求真实 exit 0 和严格 Job 清空，且不接受显式停止或宿主断连。
    evidence.native_exit_confirmed = receipt.as_ref().is_some_and(|receipt| {
        matches!(
            receipt.exit_reason,
            ExitReason::NativeExit | ExitReason::StdioClosed
        ) && receipt.exit_code == Some(0)
            && receipt.cleanup_confirmed
            && receipt.containment == "windows_job"
    });
    evidence.strict_job_cleanup_confirmed = receipt
        .as_ref()
        .is_some_and(|receipt| receipt.cleanup_confirmed && receipt.containment == "windows_job");
    evidence.product_inspect_calls += 1;
    let after = inspect(
        agent,
        Some(manifest.entry.clone()),
        Channel::FollowInstallation,
        &client,
    )
    .await;
    evidence.entry_matches_target =
        digest(&manifest.entry).ok().as_deref() == Some(manifest.target_binary.sha256.as_str());
    evidence.target_version_matches = after.as_ref().is_ok_and(|report| {
        report.source == Source::Native
            && report.error.is_none()
            && report.up_to_date
            && report.installed_version == manifest.target_version
            && report.latest_version == manifest.target_version
    });
    if executed.as_ref().ok() != Some(&manifest.target_version) {
        evidence.error = executed.err().map(|error| format!("{error:?}"));
        return Err("execute_failed");
    }
    if after.is_err() {
        evidence.error = after.err().map(|error| format!("{error:?}"));
        return Err("inspect_after_failed");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "仅在绑定官方固定文件、监督二进制和私有清单后执行真实 Windows 更新；零模型输入"]
async fn real_native_update_without_model() {
    let path = PathBuf::from(
        env::var_os("INFINISHELL_CLI_AUTOUPDATE_MANIFEST").expect("必须使用私有驱动"),
    );
    let metadata = fs::metadata(&path).expect("私有清单不存在");
    assert!(
        metadata.is_file() && metadata.len() <= 65536,
        "私有清单尺寸无效"
    );
    let bytes = fs::read(&path).expect("私有清单不可读");
    let manifest: Manifest = serde_json::from_slice(&bytes).expect("私有清单结构无效");
    let agent = validate(&manifest, &path).expect("私有固定合同核验失败");
    let mut evidence = Evidence {
        schema: 1,
        scope: SCOPE,
        agent: manifest.agent.clone(),
        old_version: manifest.old_version.clone(),
        target_version: manifest.target_version.clone(),
        manifest_sha256: sha(&bytes),
        worker_sha256: manifest.worker.sha256.clone(),
        supervisor_sha256: manifest.supervisor.sha256.clone(),
        old_sha256: manifest.old_binary.sha256.clone(),
        target_sha256: manifest.target_binary.sha256.clone(),
        fixed_release_input: true,
        isolated_journal_root_for_test: true,
        config_files_checked: CONFIGS.len(),
        ..Evidence::default()
    };
    FIXTURE
        .scope(
            (
                agent,
                manifest.target_version.clone(),
                manifest.root.clone(),
            ),
            async {
                let result = exercise(&manifest, agent, &mut evidence).await;
                evidence.failure_code = result.err();
                evidence.config_bytes_unchanged =
                    configs(&manifest.root).ok().as_ref() == Some(&manifest.configs);
                evidence.old_binary_unchanged = digest(&manifest.old_binary.path).ok().as_deref()
                    == Some(manifest.old_binary.sha256.as_str());
                evidence.target_reference_unchanged =
                    digest(&manifest.target_binary.path).ok().as_deref()
                        == Some(manifest.target_binary.sha256.as_str());
                evidence.credentials_absent = auth_absent(&manifest.root);
                evidence.journal_absent = journal_root().is_some_and(|root| {
                    fs::symlink_metadata(root.join(format!("{}.json", manifest.agent)))
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                });
                evidence.passed = evidence.failure_code.is_none()
                    && evidence.product_execute_calls == 1
                    && evidence.product_inspect_calls == 2
                    && evidence.native_exit_confirmed
                    && evidence.strict_job_cleanup_confirmed
                    && evidence.entry_matches_target
                    && evidence.target_version_matches
                    && evidence.config_bytes_unchanged
                    && evidence.journal_absent
                    && evidence.old_binary_unchanged
                    && evidence.target_reference_unchanged
                    && evidence.credentials_absent;
            },
        )
        .await;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(manifest.root.join("receipt.windows.safe.json"))
        .expect("安全收据已存在或不可写");
    serde_json::to_writer(&mut file, &evidence).expect("安全收据不可写");
    writeln!(file).expect("安全收据不可写");
    file.sync_all().expect("安全收据未持久化");
    assert!(
        evidence.passed,
        "Windows 产品事务未通过，详情仅记录于安全收据"
    );
}

#[tokio::test]
async fn fixed_windows_metadata_and_journal_do_not_escape_test_scope() {
    assert!(fixed_release(CLIAgent::Claude, Channel::Latest).is_none());
    assert!(journal_root().is_none());
    FIXTURE
        .scope(
            (
                CLIAgent::Claude,
                "2.1.280".to_owned(),
                PathBuf::from(r"C:\private-fixture"),
            ),
            async {
                assert_eq!(
                    fixed_release(CLIAgent::Claude, Channel::Latest).as_deref(),
                    Some("2.1.280")
                );
                assert!(fixed_release(CLIAgent::Grok, Channel::Stable).is_none());
                assert!(fixed_release(CLIAgent::Claude, Channel::Alpha).is_none());
                assert_eq!(
                    journal_root(),
                    Some(PathBuf::from(r"C:\private-fixture/state/cli-agent-updates"))
                );
            },
        )
        .await;
    assert!(fixed_release(CLIAgent::Claude, Channel::Latest).is_none());
    assert!(journal_root().is_none());
}

#[test]
fn fixed_windows_environment_accepts_only_disabled_native_switch_values() {
    let switches = [
        ("GROK_AUTO_UPDATE", "0"),
        ("GROK_DISABLE_AUTOUPDATER", "1"),
        ("GROK_CLAUDE_HOOKS_ENABLED", "0"),
        ("GROK_CLAUDE_MCPS_ENABLED", "0"),
        ("GROK_CODEX_HOOKS_ENABLED", "0"),
        ("GROK_CODEX_MCPS_ENABLED", "0"),
    ];
    for (key, value) in switches {
        assert!(valid_native_isolation_switch(key, OsStr::new(value)));
        for invalid in [if value == "0" { "1" } else { "0" }, "", "false", " 0"] {
            assert!(!valid_native_isolation_switch(key, OsStr::new(invalid)));
        }
    }
    assert!(!valid_native_isolation_switch(
        "XAI_API_KEY",
        OsStr::new("fixture")
    ));
}
