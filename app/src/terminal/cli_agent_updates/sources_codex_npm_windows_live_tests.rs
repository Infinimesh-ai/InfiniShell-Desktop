//! Windows x64 私有真实 npm 登记与产品事务验收；固定候选只替代渠道输入，不替代来源和隔离门禁。

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::super::{Channel, Invocation, Source};
use super::{CLIAgent, Error, UpdatePlan, contract, managed_process, npm, tree};
use crate::terminal::cli_agent_updates::sources;

const SCOPE: &str = "codex_windows_npm_transaction_no_model_v1";
const MARKER: &[u8] = b"InfiniShell private Windows npm transaction fixture; no credentials\n";
const CANDIDATE_CHANGE: &[u8] = b"owned fixture changed after official tree verification\n";
const EXTERNAL_CHANGE: &[u8] = b"later external install must survive\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Point {
    Prepared,
    OldMoved,
    Published,
}
#[derive(Clone)]
struct Control {
    point: Point,
    reached: async_channel::Sender<()>,
    resume: async_channel::Receiver<()>,
}
tokio::task_local! { static CONTROL: Control; static RELEASE: String; static EVIDENCE: PathBuf; }

pub(in super::super) fn fixed_release(agent: CLIAgent, channel: Channel) -> Option<String> {
    if agent != CLIAgent::Codex || channel != Channel::Latest {
        return None;
    }
    RELEASE
        .try_with(Clone::clone)
        .ok()
        .filter(|version| matches!(version.as_str(), "0.155.1" | "0.156.1"))
}
pub(in super::super) fn journal_root() -> Option<PathBuf> {
    EVIDENCE.try_with(|root| root.join("journal")).ok()
}
fn step() -> String {
    std::env::var("INFINISHELL_CLI_CODEX_WINDOWS_NPM_STEP").unwrap_or_default()
}
fn save_raw(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "evidence_exists")?;
    file.write_all(bytes).map_err(|_| "evidence_write")?;
    file.sync_all().map_err(|_| "evidence_sync".into())
}
fn save(path: &Path, value: &impl Serialize) -> Result<(), String> {
    save_raw(
        path,
        &serde_json::to_vec_pretty(value).map_err(|_| "evidence_shape")?,
    )
}
fn digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "file_missing")?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "file_read_failed")?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn check(value: bool, message: &str) -> Result<(), String> {
    if value { Ok(()) } else { Err(message.into()) }
}
fn mapped(error: Error) -> String {
    format!("{error:?}")
}
fn package(root: &Path) -> PathBuf {
    root.join("prefix/node_modules/@openai/codex")
}
fn entry(root: &Path) -> PathBuf {
    root.join("prefix/codex.cmd")
}

pub(super) async fn checkpoint(point: Point) {
    let _ = EVIDENCE.try_with(|root| {
        let name = match point {
            Point::Prepared => "prepared-journal.safe.json",
            Point::OldMoved => "old-moved-journal.safe.json",
            Point::Published => "published-journal.safe.json",
        };
        let raw = fs::read(super::journal_path(&root.join("journal"))).expect("断点 journal 缺失");
        save_raw(&root.join(name), &raw).expect("无法封存真实断点 journal");
    });
    if let Ok(control) = CONTROL.try_with(Clone::clone) {
        if control.point == point {
            control.reached.send(()).await.expect("断点接收者必须存在");
            let _ = control.resume.recv().await;
        }
    }
}
pub(super) fn preserve_probe_stdout(generation: uuid::Uuid, bytes: &[u8]) {
    let _ = EVIDENCE.try_with(|root| {
        save_raw(&root.join(format!("probe-{generation}.stdout")), bytes)
            .expect("无法保留探针原始输出");
    });
}
pub(super) fn preserve_verified_inputs(
    wrapper_metadata: &[u8],
    platform_metadata: &[u8],
    wrapper: &Path,
    platform: &Path,
) {
    let _ = EVIDENCE.try_with(|root| {
        for (name, bytes) in [
            ("verified-wrapper.metadata.json", wrapper_metadata),
            ("verified-platform.metadata.json", platform_metadata),
        ] {
            save_raw(&root.join(name), bytes).expect("无法保留官方元数据");
        }
        for (name, source) in [
            ("verified-wrapper.tgz", wrapper),
            ("verified-platform.tgz", platform),
        ] {
            let mut input = File::open(source).expect("已验官方包缺失");
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join(name))
                .expect("官方包收据已存在");
            std::io::copy(&mut input, &mut output).expect("无法保留官方原包");
            output.sync_all().expect("无法持久化官方原包");
        }
    });
}
fn probe_kind(root: &Path, invocation: &Invocation) -> Result<&'static str, Error> {
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(root.join("manifest.private.json")).map_err(|_| Error::SourceChanged)?,
    )
    .map_err(|_| Error::SourceChanged)?;
    let program = invocation
        .program
        .canonicalize()
        .map_err(|_| Error::SourceChanged)?;
    let prefix = program == manifest.node.path
        && invocation.args
            == [
                manifest.npm_cli.path.into_os_string(),
                "prefix".into(),
                "--global".into(),
            ];
    let version = program == entry(root) && invocation.args == [OsString::from("--version")];
    let powershell =
        super::version_invocation(&root.join("prefix/codex.ps1"))?.is_some_and(|expected| {
            expected.program == invocation.program && expected.args == invocation.args
        });
    if !invocation.env.is_empty() || !invocation.env_remove.is_empty() {
        return Err(Error::UnsupportedSource);
    }
    if prefix {
        Ok("npm_prefix_global")
    } else if version {
        Ok("public_cmd_version")
    } else if powershell {
        Ok("public_powershell_version")
    } else {
        Err(Error::UnsupportedSource)
    }
}
pub(in super::super) fn audit_probe(invocation: &Invocation) -> Result<(), Error> {
    EVIDENCE
        .try_with(|root| probe_kind(root, invocation).map(|_| ()))
        .unwrap_or(Ok(()))
}
pub(in super::super) fn preserve_readonly_stdout(invocation: &Invocation, bytes: &[u8]) {
    let _ = EVIDENCE.try_with(|root| {
        let kind = probe_kind(root, invocation).expect("只读命令已通过审计");
        let id = uuid::Uuid::new_v4();
        save_raw(&root.join(format!("readonly-{id}.stdout")), bytes).expect("无法保留公共入口原始输出");
        save(&root.join(format!("readonly-{id}.safe.json")), &json!({"kind":kind,"program":invocation.program,
            "arguments":invocation.args,"stdout_sha256":format!("{:x}",Sha256::digest(bytes)),"step":step()})).unwrap();
    });
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binary {
    path: PathBuf,
    sha256: String,
}
#[derive(Clone, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Member {
    length: u64,
    sha256: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    scope: String,
    case: String,
    root: PathBuf,
    worker: Binary,
    supervisor: Binary,
    node: Binary,
    npm_cli: Binary,
    old_public_sha256: String,
    npm_install_sha256: String,
    npm_tree: BTreeMap<String, Member>,
    shims: BTreeMap<String, String>,
    source_sha256: BTreeMap<String, String>,
}

const SOURCES: &[(&str, &[u8])] = &[
    (
        "app/src/terminal/cli_agent_updates.rs",
        include_bytes!("../cli_agent_updates.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources.rs",
        include_bytes!("sources.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_npm.rs",
        include_bytes!("sources_npm.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_npm_release.rs",
        include_bytes!("sources_npm_release.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_npm_codex_windows.rs",
        include_bytes!("sources_npm_codex_windows.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_npm_codex_windows_tree.rs",
        include_bytes!("sources_npm_codex_windows_tree.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_npm_codex_windows_contract.rs",
        include_bytes!("sources_npm_codex_windows_contract.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_codex_npm_windows_live_tests.rs",
        include_bytes!("sources_codex_npm_windows_live_tests.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process_version_probe.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_version_probe.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process_atomic_windows.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_windows.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process_npm_probe_windows.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_npm_probe_windows.rs"),
    ),
    (
        "crates/command/src/windows_appcontainer.rs",
        include_bytes!("../../../../crates/command/src/windows_appcontainer.rs"),
    ),
    (
        "script/cli-agent-parity/codex_0156_package_manifest.json",
        include_bytes!("../../../../script/cli-agent-parity/codex_0156_package_manifest.json"),
    ),
    (
        "script/cli-agent-parity/run_claude_npm_update_live.py",
        include_bytes!("../../../../script/cli-agent-parity/run_claude_npm_update_live.py"),
    ),
    (
        "script/cli-agent-parity/run_codex_npm_windows_update_live.py",
        include_bytes!("../../../../script/cli-agent-parity/run_codex_npm_windows_update_live.py"),
    ),
];

fn env_paths() -> [(&'static str, &'static str); 17] {
    [
        ("HOME", "home"),
        ("USERPROFILE", "home"),
        ("APPDATA", "home/AppData/Roaming"),
        ("LOCALAPPDATA", "home/AppData/Local"),
        ("CODEX_HOME", "home/.codex"),
        ("CLAUDE_CONFIG_DIR", "home/.claude"),
        ("GROK_HOME", "home/.grok"),
        ("XDG_CONFIG_HOME", "home/.config"),
        ("XDG_DATA_HOME", "home/.local/share"),
        ("XDG_CACHE_HOME", "home/.cache"),
        ("XDG_STATE_HOME", "home/.local/state"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
        ("TMPDIR", "tmp"),
        ("NPM_CONFIG_USERCONFIG", "npm/user.npmrc"),
        ("NPM_CONFIG_GLOBALCONFIG", "npm/global.npmrc"),
        ("NPM_CONFIG_CACHE", "npm/cache"),
    ]
}
fn manager_inventory(manifest: &Manifest) -> Result<BTreeMap<String, Member>, String> {
    let root = manifest
        .npm_cli
        .path
        .parent()
        .and_then(Path::parent)
        .ok_or("npm_parent_missing")?;
    sources::plain_ancestors(root).map_err(mapped)?;
    let mut pending = vec![PathBuf::new()];
    let mut files = BTreeMap::new();
    let mut visited = 0;
    while let Some(relative) = pending.pop() {
        visited += 1;
        check(visited <= 16384, "npm_manager_inventory_limit")?;
        let path = root.join(&relative);
        let identity = tree::path_identity(&path).map_err(mapped)?;
        if identity.directory {
            for child in fs::read_dir(&path).map_err(|_| "npm_manager_read")? {
                let child = relative.join(child.map_err(|_| "npm_manager_read")?.file_name());
                tree::safe_relative(&child).map_err(mapped)?;
                pending.push(child);
            }
        } else {
            files.insert(
                relative.to_string_lossy().replace('\\', "/"),
                Member {
                    length: identity.length,
                    sha256: super::hex(&identity.digest),
                },
            );
        }
    }
    Ok(files)
}
fn verify_shims(manifest: &Manifest) -> Result<(), String> {
    check(manifest.shims.len() == 3, "shim_count")?;
    for (name, bytes) in contract::shims() {
        let path = manifest.root.join("prefix").join(name);
        check(
            fs::read(&path).ok().as_deref() == Some(bytes.as_bytes())
                && manifest.shims.get(name) == Some(&digest(&path)?),
            "real_npm_shim_contract",
        )?;
    }
    Ok(())
}
fn validate(manifest: &Manifest, path: &Path) -> Result<(), String> {
    let root = &manifest.root;
    check(
        cfg!(all(windows, target_arch = "x86_64"))
            && manifest.schema == 1
            && manifest.scope == SCOPE
            && root.canonicalize().ok().as_ref() == Some(root)
            && root
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.starts_with("infinishell-codex-windows-npm-"))
            && path == root.join("manifest.private.json")
            && fs::read(root.join(".infinishell-windows-npm-live"))
                .ok()
                .as_deref()
                == Some(MARKER),
        "fixture_contract",
    )?;
    sources::plain_ancestors(root).map_err(mapped)?;
    tree::path_identity(root).map_err(mapped)?;
    check(
        [
            "updated",
            "old_moved",
            "published_receipt_missing",
            "external_change_preserved",
            "candidate_changed_preserved",
        ]
        .contains(&manifest.case.as_str()),
        "case_unknown",
    )?;
    for (name, relative) in env_paths() {
        let expected = root.join(relative);
        check(
            std::env::var_os(name).map(PathBuf::from).as_ref() == Some(&expected)
                && expected.canonicalize().ok().as_ref() == Some(&expected),
            "private_environment_path",
        )?;
    }
    let allowed = env_paths()
        .into_iter()
        .map(|(name, _)| name)
        .chain([
            "SYSTEMROOT",
            "WINDIR",
            "COMSPEC",
            "PATH",
            "PATHEXT",
            "LANG",
            "LC_ALL",
            "DISABLE_AUTOUPDATER",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            "INFINISHELL_CLI_CODEX_WINDOWS_NPM_ALLOW",
            "INFINISHELL_CLI_CODEX_WINDOWS_NPM_MANIFEST",
            "INFINISHELL_CLI_CODEX_WINDOWS_NPM_STEP",
            "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE",
        ])
        .collect::<std::collections::BTreeSet<_>>();
    for (name, _) in std::env::vars_os() {
        check(
            allowed.contains(name.to_string_lossy().to_ascii_uppercase().as_str()),
            "inherited_environment",
        )?;
    }
    check(
        std::env::var("INFINISHELL_CLI_CODEX_WINDOWS_NPM_ALLOW").as_deref() == Ok(SCOPE)
            && std::env::current_dir()
                .ok()
                .and_then(|path| path.canonicalize().ok())
                == Some(root.join("project"))
            && std::env::var_os("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE").map(PathBuf::from)
                == Some(manifest.supervisor.path.clone())
            && matches!(step().as_str(), "execute" | "recover"),
        "invocation_contract",
    )?;
    for binary in [
        &manifest.worker,
        &manifest.supervisor,
        &manifest.node,
        &manifest.npm_cli,
    ] {
        check(
            binary.path.canonicalize().ok().as_ref() == Some(&binary.path)
                && digest(&binary.path)? == binary.sha256,
            "binary_binding",
        )?;
    }
    check(
        std::env::current_exe()
            .ok()
            .and_then(|path| path.canonicalize().ok())
            .as_ref()
            == Some(&manifest.worker.path),
        "worker_binding",
    )?;
    check(
        manifest
            .node
            .path
            .parent()
            .map(|path| path.join("node_modules/npm/bin/npm-cli.js"))
            == Some(manifest.npm_cli.path.clone())
            && npm::registered_manager(&manifest.npm_cli.path).map_err(mapped)?
            && manager_inventory(manifest)? == manifest.npm_tree,
        "same_registered_node_npm_installation",
    )?;
    check(
        manifest.source_sha256.len() == SOURCES.len(),
        "source_count",
    )?;
    for (name, bytes) in SOURCES {
        check(
            manifest.source_sha256.get(*name) == Some(&format!("{:x}", Sha256::digest(bytes))),
            "compiled_source_binding",
        )?;
    }
    verify_shims(manifest)?;
    let install_path = root.join("npm-install.safe.json");
    check(
        digest(&install_path)? == manifest.npm_install_sha256,
        "npm_install_receipt_changed",
    )?;
    let receipt: Value =
        serde_json::from_slice(&fs::read(install_path).map_err(|_| "npm_install_missing")?)
            .map_err(|_| "npm_install_shape")?;
    check(
        receipt["operation"] == "real_npm_private_install"
            && receipt["exit_code"] == 0
            && receipt["scripts_disabled"] == true
            && receipt["node"]["sha256"] == manifest.node.sha256
            && receipt["npm_cli"]["sha256"] == manifest.npm_cli.sha256
            && receipt["private_prefix"] == root.join("prefix").to_string_lossy().as_ref(),
        "real_npm_registration_required",
    )?;
    check(
        !root.join("home/.codex/auth.json").exists()
            && fs::read(root.join("home/.codex/config.toml")).ok()
                == fs::read(root.join("before-config.raw")).ok(),
        "private_auth_or_config",
    )?;
    if step() == "execute" {
        check(
            digest(&package(root).join("bin/codex.js"))? == manifest.old_public_sha256,
            "old_public_binding",
        )?;
    }
    Ok(())
}
async fn inspect(manifest: &Manifest) -> Result<sources::CheckReport, Error> {
    let client = std::sync::Arc::new(http_client::Client::new());
    RELEASE
        .scope(
            "0.156.1".to_owned(),
            sources::inspect(
                CLIAgent::Codex,
                Some(entry(&manifest.root)),
                Channel::Latest,
                &client,
            ),
        )
        .await
}
async fn prepare(manifest: &Manifest) -> Result<UpdatePlan, String> {
    let report = inspect(manifest).await.map_err(mapped)?;
    check(
        report.source == Source::Npm
            && report.installed_version == "0.155.1"
            && report.latest_version == "0.156.1"
            && report.error.is_none(),
        "product_npm_plan_mismatch",
    )?;
    report.plan.ok_or_else(|| "product_npm_plan_missing".into())
}
fn load_journal(root: &Path) -> Result<super::Journal, String> {
    serde_json::from_slice(
        &fs::read(super::journal_path(&root.join("journal"))).map_err(|_| "journal_missing")?,
    )
    .map_err(|_| "journal_invalid".into())
}
fn probe_evidence(root: &Path, journal: &super::Journal) -> Result<Value, String> {
    check(
        journal.probes.len() == 2,
        "both_candidate_public_modes_required",
    )?;
    let mut records = Vec::new();
    for (mode, probe) in ["cmd", "powershell"].into_iter().zip(&journal.probes) {
        check(
            probe.input.mode() == mode
                && probe.observed.as_deref() == Some("0.156.1")
                && probe.input.stage() == journal.stage
                && probe.input.node() == journal.owner.node.canonical
                && probe.digest == super::probe_digest(&probe.input, journal.id).map_err(mapped)?,
            "public_probe_binding",
        )?;
        let binding = managed_process::PreparedLaunchBinding::codex_windows_npm_version_probe(
            probe.digest.clone(),
        )
        .map_err(|_| "probe_binding_invalid")?;
        let receipt = managed_process::confirmed_exit_with_binding(
            &root.join("journal"),
            probe.generation,
            &binding,
        )
        .map_err(|_| "probe_cleanup_invalid")?
        .ok_or("probe_cleanup_missing")?;
        check(
            receipt.cleanup_confirmed
                && receipt.exit_code == Some(0)
                && receipt.containment != "not_started",
            "probe_not_successfully_cleaned",
        )?;
        let output = fs::read(root.join(format!("probe-{}.stdout", probe.generation)))
            .map_err(|_| "probe_stdout_missing")?;
        check(
            std::str::from_utf8(&output).ok().map(str::trim) == Some("codex-cli 0.156.1"),
            "probe_stdout_version",
        )?;
        let record = root
            .join("journal/cli-agent-processes")
            .join(probe.generation.to_string());
        let raw = fs::read(record.join("exit.json")).map_err(|_| "raw_exit_missing")?;
        save_raw(
            &root.join(format!("{mode}-exit-{}.safe.json", step())),
            &raw,
        )?;
        records.push(json!({"mode":mode,"generation":probe.generation,"binding_digest":probe.digest,
            "input":probe.input,"cleanup_receipt":receipt,"stdout_sha256":format!("{:x}",Sha256::digest(output))}));
    }
    Ok(
        json!({"public_modes":["cmd","powershell"],"independent_generations":true,
        "public_node_launcher_executed":true,"records":records}),
    )
}
fn verify_permissions(before: &tree::Snapshot, after: &tree::Snapshot) -> Result<(), String> {
    for (path, old) in &before.members {
        if let Some(new) = after.members.get(path) {
            check(old.security == new.security, "owner_or_acl_changed")?;
        }
    }
    Ok(())
}
async fn finish_evidence(
    manifest: &Manifest,
    before: &tree::Snapshot,
    probe: Value,
    expected: &str,
    expected_failure: Option<&str>,
) -> Result<Value, String> {
    let root = &manifest.root;
    verify_shims(manifest)?;
    let after = tree::snapshot(&package(root)).map_err(mapped)?;
    verify_permissions(before, &after)?;
    save(&root.join("after-tree.safe.json"), &after)?;
    for name in ["codex.cmd", "codex.ps1"] {
        check(
            sources::version(CLIAgent::Codex, &root.join("prefix").join(name))
                .await
                .map_err(mapped)?
                == expected,
            "public_version_after",
        )?;
    }
    check(
        fs::read(root.join("before-config.raw")).ok()
            == fs::read(root.join("home/.codex/config.toml")).ok()
            && manager_inventory(manifest)? == manifest.npm_tree,
        "configuration_or_manager_changed",
    )?;
    Ok(
        json!({"case":manifest.case,"accepted":true,"public_version":expected,"candidate_probe":probe,
        "expected_failure":expected_failure,"cold_recovery_process":step()=="recover","entry_shims_preserved":true,
        "owner_and_acl_preserved":true,"private_config_bytes_unchanged":true,"private_npm_registration_executed":true,
        "npm_lifecycle_executed":false,"consumer_channel_discovery_covered":false,"model_inputs_sent":0,"g09_closed":false}),
    )
}
async fn exercise(manifest: &Manifest) -> Result<Value, String> {
    let root = &manifest.root;
    let before = tree::snapshot(&package(root)).map_err(mapped)?;
    save(&root.join("before-tree.safe.json"), &before)?;
    let plan = prepare(manifest).await?;
    if manifest.case == "updated" {
        check(
            sources::execute(plan, None).await.map_err(mapped)? == "0.156.1",
            "updated_version",
        )?;
        let saved: super::Journal = serde_json::from_slice(
            &fs::read(root.join("published-journal.safe.json"))
                .map_err(|_| "published_receipt_missing")?,
        )
        .map_err(|_| "published_receipt_shape")?;
        check(
            !super::journal_path(&root.join("journal")).exists()
                && !saved.backup.exists()
                && !saved.stage.exists(),
            "completed_transaction_residue",
        )?;
        let evidence = probe_evidence(root, &saved)?;
        return finish_evidence(manifest, &before, evidence, "0.156.1", None).await;
    }
    let point = match manifest.case.as_str() {
        "candidate_changed_preserved" => Point::Prepared,
        "old_moved" => Point::OldMoved,
        "published_receipt_missing" | "external_change_preserved" => Point::Published,
        _ => return Err("case_unknown".into()),
    };
    let (reached_tx, reached_rx) = async_channel::bounded(1);
    let (resume_tx, resume_rx) = async_channel::bounded(1);
    let task = tokio::spawn(EVIDENCE.scope(
        root.clone(),
        CONTROL.scope(
            Control {
                point,
                reached: reached_tx,
                resume: resume_rx,
            },
            async move { sources::execute(plan, None).await },
        ),
    ));
    if !matches!(
        tokio::time::timeout(std::time::Duration::from_secs(700), reached_rx.recv()).await,
        Ok(Ok(()))
    ) {
        task.abort();
        let _ = task.await;
        return Err("fault_checkpoint_not_reached".into());
    }
    let saved = load_journal(root)?;
    let journal_path = super::journal_path(&root.join("journal"));
    save_raw(
        &root.join("checkpoint-journal.safe.json"),
        &fs::read(&journal_path).map_err(|_| "checkpoint_missing")?,
    )?;
    if point == Point::Prepared {
        let changed = saved.stage.join("bin/codex.js");
        fs::write(&changed, CANDIDATE_CHANGE).map_err(|_| "candidate_change_failed")?;
        resume_tx.send(()).await.map_err(|_| "checkpoint_closed")?;
        check(
            task.await.map_err(|_| "update_task_failed")? == Err(Error::RecoveryRequired),
            "candidate_not_rejected",
        )?;
        check(
            tree::snapshot(&package(root)).map_err(mapped)? == before
                && journal_path.exists()
                && fs::read(changed).ok().as_deref() == Some(CANDIDATE_CHANGE)
                && load_journal(root)?.probes.is_empty(),
            "candidate_failure_overwrote_state",
        )?;
        return finish_evidence(
            manifest,
            &before,
            json!({"public_modes":[],"started":false,"rejected_before_spawn":true}),
            "0.155.1",
            Some("RecoveryRequired"),
        )
        .await;
    }
    // 两个候选进程已经有真实退出证明后才中断更新任务；新 libtest 进程随后调用生产恢复。
    check(
        saved.probes.len() == 2 && saved.phase == super::Phase::OldMoved,
        "publication_checkpoint_phase",
    )?;
    super::verified_exit(&root.join("journal"), &saved, true).map_err(mapped)?;
    task.abort();
    check(task.await.is_err(), "update_not_interrupted")?;
    if manifest.case == "external_change_preserved" {
        fs::write(package(root).join("external-npm-change"), EXTERNAL_CHANGE)
            .map_err(|_| "external_change_failed")?;
    }
    Ok(
        json!({"case":manifest.case,"accepted":false,"needs_cold_recovery":true,
        "checkpoint":format!("{point:?}"),"durable_phase":"OldMoved","candidate_cleanup_confirmed":true,"model_inputs_sent":0}),
    )
}
async fn cold_recover(manifest: &Manifest) -> Result<Value, String> {
    let root = &manifest.root;
    check(
        [
            "old_moved",
            "published_receipt_missing",
            "external_change_preserved",
        ]
        .contains(&manifest.case.as_str()),
        "cold_case_unknown",
    )?;
    let before: tree::Snapshot = serde_json::from_slice(
        &fs::read(root.join("before-tree.safe.json")).map_err(|_| "before_missing")?,
    )
    .map_err(|_| "before_invalid")?;
    let saved: super::Journal = serde_json::from_slice(
        &fs::read(root.join("checkpoint-journal.safe.json")).map_err(|_| "checkpoint_missing")?,
    )
    .map_err(|_| "checkpoint_invalid")?;
    check(
        saved.phase == super::Phase::OldMoved && saved.owner.package == package(root),
        "cold_checkpoint_invalid",
    )?;
    let result = inspect(manifest).await;
    let (expected, failure) = if manifest.case == "external_change_preserved" {
        check(
            matches!(result, Err(Error::RecoveryRequired)),
            "external_change_not_rejected",
        )?;
        check(
            fs::read(package(root).join("external-npm-change"))
                .ok()
                .as_deref()
                == Some(EXTERNAL_CHANGE)
                && tree::snapshot(&saved.backup).map_err(mapped)? == before
                && super::journal_path(&root.join("journal")).exists(),
            "external_state_overwritten",
        )?;
        ("0.156.1", Some("RecoveryRequired"))
    } else {
        let report = result.map_err(mapped)?;
        check(
            report.source == Source::Npm
                && report.installed_version == "0.155.1"
                && tree::snapshot(&package(root)).map_err(mapped)? == before
                && !super::journal_path(&root.join("journal")).exists()
                && !saved.stage.exists()
                && !saved.backup.exists(),
            "cold_rollback_mismatch",
        )?;
        ("0.155.1", None)
    };
    let evidence = probe_evidence(root, &saved)?;
    finish_evidence(manifest, &before, evidence, expected, failure).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要真实 Windows x64 npm 私有安装与同源码 worker/supervisor，不提供认证或模型输入"]
async fn real_codex_windows_npm_update_without_model() {
    let path = PathBuf::from(
        std::env::var_os("INFINISHELL_CLI_CODEX_WINDOWS_NPM_MANIFEST").expect("缺少私有清单"),
    );
    let bytes = fs::read(&path).expect("无法读取私有清单");
    assert!(bytes.len() <= 4 * 1024 * 1024);
    let manifest: Manifest = serde_json::from_slice(&bytes).expect("私有清单格式错误");
    let result = EVIDENCE
        .scope(manifest.root.clone(), async {
            validate(&manifest, &path)?;
            save(
                &manifest.root.join(format!("started-{}.safe.json", step())),
                &json!({"scope":SCOPE,"case":manifest.case,"source_sha256":manifest.source_sha256}),
            )?;
            if step() == "recover" {
                cold_recover(&manifest).await
            } else {
                exercise(&manifest).await
            }
        })
        .await;
    let evidence = match &result {
        Ok(value) => value.clone(),
        Err(error) => {
            json!({"case":manifest.case,"accepted":false,"error":error,"model_inputs_sent":0})
        }
    };
    save(
        &manifest.root.join(format!("result-{}.safe.json", step())),
        &evidence,
    )
    .unwrap();
    assert!(
        result.is_ok(),
        "Windows npm 产品验收失败，原始现场保留：{result:?}"
    );
}
