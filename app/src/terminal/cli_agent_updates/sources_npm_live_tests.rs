//! 零模型、私有官方 npm 树的真实事务验收；只运行已绑定 npm 的只读 prefix 查询。

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::super::{Channel, Source};
use super::{CLIAgent, Error, UpdatePlan, managed_process, npm, tree};
use crate::terminal::cli_agent_updates::sources;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Point {
    Prepared,
    Exchanged,
}
#[derive(Clone)]
struct Control {
    point: Point,
    reached: async_channel::Sender<()>,
    resume: async_channel::Receiver<()>,
}
tokio::task_local! { static CONTROL: Control; static RELEASE: String; static EVIDENCE: PathBuf; }
pub(super) fn preserve_probe_stdout(generation: uuid::Uuid, bytes: &[u8]) {
    let _ = EVIDENCE.try_with(|root| {
        use std::os::unix::fs::OpenOptionsExt as _;
        let path = root.join(format!("probe-{generation}.stdout"));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .expect("无法保留原始版本输出");
        file.write_all(bytes).expect("无法写入原始版本输出");
        file.sync_all().expect("无法持久化原始版本输出");
    });
}
pub(super) fn fixed_release(agent: CLIAgent, channel: Channel) -> Option<String> {
    if agent != CLIAgent::Claude || channel != Channel::Latest {
        return None;
    }
    RELEASE
        .try_with(Clone::clone)
        .ok()
        .filter(|version| matches!(version.as_str(), "2.1.278" | "2.1.280"))
}
fn step() -> String {
    std::env::var("INFINISHELL_CLI_NPM_UPDATE_STEP").unwrap_or_default()
}

pub(super) fn preserve_verified_inputs(
    wrapper_metadata: &[u8],
    platform_metadata: &[u8],
    wrapper: &Path,
    platform: &Path,
) {
    let _ = EVIDENCE.try_with(|root| {
        use std::os::unix::fs::OpenOptionsExt as _;
        for (name, bytes) in [
            ("verified-wrapper.metadata.json", wrapper_metadata),
            ("verified-platform.metadata.json", platform_metadata),
        ] {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(root.join(name))
                .expect("无法建立原始元数据收据");
            file.write_all(bytes).expect("无法写入原始元数据收据");
            file.sync_all().unwrap();
        }
        for (name, source) in [
            ("verified-wrapper.tgz", wrapper),
            ("verified-platform.tgz", platform),
        ] {
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(root.join(name))
                .expect("无法建立原始包收据");
            let mut input = File::open(source).expect("官方已验包缺失");
            std::io::copy(&mut input, &mut output).expect("无法保留原始包收据");
            output.sync_all().unwrap();
        }
    });
}

pub(super) fn audit_probe(invocation: &sources::Invocation) -> Result<(), Error> {
    EVIDENCE
        .try_with(|root| {
            let manifest: Manifest = serde_json::from_slice(
                &fs::read(root.join("manifest.private.json")).map_err(|_| Error::SourceChanged)?,
            )
            .map_err(|_| Error::SourceChanged)?;
            let program = invocation
                .program
                .canonicalize()
                .map_err(|_| Error::SourceChanged)?;
            let npm_args = [
                manifest.npm_cli.path.as_os_str().to_owned(),
                "prefix".into(),
                "--global".into(),
            ];
            let prefix = program == manifest.node.path
                && invocation.args == npm_args
                && digest(&program).ok().as_deref() == Some(manifest.node.sha256.as_str())
                && digest(&manifest.npm_cli.path).ok().as_deref()
                    == Some(manifest.npm_cli.sha256.as_str());
            let version = program == package(root).join("bin/claude.exe")
                && invocation.args == [std::ffi::OsString::from("--version")];
            if (!prefix && !version)
                || !invocation.env.is_empty()
                || !invocation.env_remove.is_empty()
            {
                return Err(Error::UnsupportedSource);
            }
            save(
                &root.join(format!("readonly-probe-{}.safe.json", uuid::Uuid::new_v4())),
                &json!({
                    "kind":if prefix { "npm_prefix_global" } else { "public_cli_version" },
                    "program":program,"arguments":invocation.args,"install_or_script":false,
                }),
            )
            .map_err(|_| Error::PersistenceFailed)
        })
        .unwrap_or(Ok(()))
}

pub(super) async fn checkpoint(point: Point) {
    let _ = EVIDENCE.try_with(|root| {
        let source = super::journal_path(&sources::journal_root().unwrap(), CLIAgent::Claude);
        let raw = fs::read(source).unwrap();
        let name = match point {
            Point::Prepared => "prepared-journal.safe.json",
            Point::Exchanged => "exchanged-journal.safe.json",
        };
        save_raw(&root.join(name), &raw).expect("无法保留精确断点的原始 journal");
    });
    if let Ok(control) = CONTROL.try_with(Clone::clone) {
        if control.point == point {
            control
                .reached
                .send(())
                .await
                .expect("测试断点接收者必须存在");
            let _ = control.resume.recv().await;
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binary {
    path: PathBuf,
    sha256: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    scope: String,
    case: String,
    root: PathBuf,
    node: Binary,
    npm_cli: Binary,
    worker: Binary,
    supervisor: Binary,
    old_public_sha256: String,
    source_sha256: BTreeMap<String, String>,
}

const SCOPE: &str = "claude_npm_transaction_no_model_v1";
const MARKER: &[u8] = b"InfiniShell private npm transaction fixture; no credentials\n";
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
        "app/src/terminal/cli_agent_updates/sources_npm_transaction.rs",
        include_bytes!("sources_npm_transaction.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_npm_tree_unix.rs",
        include_bytes!("sources_npm_tree_unix.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_npm_live_tests.rs",
        include_bytes!("sources_npm_live_tests.rs"),
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
        "app/src/ai/cli_agent_runtime/managed_process_atomic_macos.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_macos.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process_atomic_linux.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_linux.rs"),
    ),
    (
        "app/src/ai/cli_agent_runtime/managed_process_atomic_linux_glibc.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_linux_glibc.rs"),
    ),
    (
        "script/cli-agent-parity/run_claude_npm_update_live.py",
        include_bytes!("../../../../script/cli-agent-parity/run_claude_npm_update_live.py"),
    ),
];

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
fn save(path: &Path, value: &impl Serialize) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| "evidence_exists")?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|_| "evidence_shape")?)
        .map_err(|_| "evidence_write")?;
    file.sync_all().map_err(|_| "evidence_sync".into())
}
fn save_raw(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| "raw_evidence_exists")?;
    file.write_all(bytes).map_err(|_| "raw_evidence_write")?;
    file.sync_all().map_err(|_| "raw_evidence_sync".into())
}

fn mapped(error: Error) -> String {
    format!("{error:?}")
}
fn entry(root: &Path) -> PathBuf {
    root.join("prefix/bin/claude")
}
fn package(root: &Path) -> PathBuf {
    root.join("prefix/lib/node_modules/@anthropic-ai/claude-code")
}
fn tree(root: &Path) -> Result<tree::Snapshot, String> {
    tree::Directory::open(&package(root))
        .and_then(|directory| directory.snapshot())
        .map_err(mapped)
}

fn validate(manifest: &Manifest, path: &Path) -> Result<(), String> {
    let root = &manifest.root;
    let metadata = fs::symlink_metadata(root).map_err(|_| "fixture_missing")?;
    check(
        manifest.schema == 1
            && manifest.scope == SCOPE
            && path == root.join("manifest.private.json")
            && root.canonicalize().ok().as_ref() == Some(root)
            && metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o7777 == 0o700
            && fs::read(root.join(".infinishell-npm-live")).ok().as_deref() == Some(MARKER),
        "fixture_contract",
    )?;
    check(
        [
            "updated",
            "swap_receipt_missing",
            "external_change_preserved",
            "candidate_changed_preserved",
            "unreviewed_downgrade_rejected",
        ]
        .contains(&manifest.case.as_str()),
        "case_unknown",
    )?;
    for (name, relative) in env_paths() {
        let expected = root.join(relative);
        check(
            std::env::var_os(name).map(PathBuf::from).as_ref() == Some(&expected)
                && expected.canonicalize().ok().as_ref() == Some(&expected),
            "environment_path",
        )?;
    }
    check(
        std::env::current_dir().ok().as_ref() == Some(&root.join("project"))
            && std::env::var("INFINISHELL_CLI_NPM_UPDATE_ALLOW").as_deref() == Ok(SCOPE)
            && std::env::var_os("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE")
                .map(PathBuf::from)
                .as_ref()
                == Some(&manifest.supervisor.path),
        "environment_contract",
    )?;
    let allowed = env_paths()
        .into_iter()
        .map(|(name, _)| name)
        .chain([
            "PATH",
            "LANG",
            "LC_ALL",
            "DISABLE_AUTOUPDATER",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            "INFINISHELL_CLI_NPM_UPDATE_ALLOW",
            "INFINISHELL_CLI_NPM_UPDATE_MANIFEST",
            "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE",
            "INFINISHELL_CLI_NPM_UPDATE_STEP",
            "NPM_CONFIG_USERCONFIG",
            "NPM_CONFIG_GLOBALCONFIG",
            "NPM_CONFIG_CACHE",
        ])
        .collect::<std::collections::BTreeSet<_>>();
    for (name, value) in std::env::vars() {
        let apple = cfg!(target_os = "macos")
            && name == "__CF_USER_TEXT_ENCODING"
            && value == format!("0x{:X}:0x0:0x0", unsafe { libc::getuid() });
        check(
            allowed.contains(name.as_str()) || apple,
            "inherited_environment",
        )?;
    }
    for binary in [
        &manifest.node,
        &manifest.npm_cli,
        &manifest.worker,
        &manifest.supervisor,
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
        "test_binary_binding",
    )?;
    for (name, bytes) in SOURCES {
        check(
            manifest.source_sha256.get(*name) == Some(&format!("{:x}", Sha256::digest(bytes))),
            "compiled_source_binding",
        )?;
    }
    check(
        matches!(step().as_str(), "execute" | "recover"),
        "step_unknown",
    )?;
    if step() == "execute" {
        check(
            digest(&entry(root))? == manifest.old_public_sha256,
            "old_public_binding",
        )?;
    }
    for (name, relative) in [
        ("NPM_CONFIG_USERCONFIG", "npm/user.npmrc"),
        ("NPM_CONFIG_GLOBALCONFIG", "npm/global.npmrc"),
        ("NPM_CONFIG_CACHE", "npm/cache"),
    ] {
        check(
            std::env::var_os(name).map(PathBuf::from) == Some(root.join(relative)),
            "npm_config_not_private",
        )?;
    }
    check(
        npm::registered_manager(&manifest.npm_cli.path).map_err(mapped)?,
        "registered_npm_manager",
    )?;
    let journal = sources::journal_root().map_err(mapped)?;
    check(
        journal.starts_with(root.join("home")),
        "journal_outside_fixture",
    )?;
    Ok(())
}

fn env_paths() -> Vec<(&'static str, &'static str)> {
    vec![
        ("HOME", "home"),
        ("USERPROFILE", "home"),
        ("CLAUDE_CONFIG_DIR", "home/.claude"),
        ("CODEX_HOME", "home/.codex"),
        ("GROK_HOME", "home/.grok"),
        ("XDG_CONFIG_HOME", "home/.config"),
        ("XDG_CACHE_HOME", "home/.cache"),
        ("XDG_DATA_HOME", "home/.local/share"),
        ("XDG_STATE_HOME", "home/.local/state"),
        ("TMPDIR", "tmp"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
    ]
}

async fn inspect(manifest: &Manifest, target: &str) -> Result<sources::CheckReport, Error> {
    let client = std::sync::Arc::new(http_client::Client::new());
    RELEASE
        .scope(
            target.to_owned(),
            sources::inspect(
                CLIAgent::Claude,
                Some(entry(&manifest.root)),
                Channel::Latest,
                &client,
            ),
        )
        .await
}
async fn prepare(manifest: &Manifest, target: &str) -> Result<UpdatePlan, String> {
    let report = inspect(manifest, target).await.map_err(mapped)?;
    check(
        report.source == Source::Npm && report.error.is_none() && report.latest_version == target,
        "product_npm_plan_mismatch",
    )?;
    report.plan.ok_or_else(|| "product_npm_plan_missing".into())
}

fn verify_preserved_permissions(
    before: &tree::Snapshot,
    after: &tree::Snapshot,
) -> Result<(), String> {
    let before = serde_json::to_value(before).map_err(|_| "permissions_before_shape")?;
    let after = serde_json::to_value(after).map_err(|_| "permissions_after_shape")?;
    for field in ["uid", "gid", "mode"] {
        check(
            before["root"][field] == after["root"][field],
            "root_permissions_changed",
        )?;
    }
    let nodes = before["nodes"]
        .as_object()
        .ok_or("permissions_nodes_shape")?;
    for (path, node) in nodes {
        if let Some(actual) = after["nodes"].get(path) {
            for field in ["uid", "gid", "mode"] {
                check(
                    node["identity"][field] == actual["identity"][field],
                    "member_permissions_changed",
                )?;
            }
        }
    }
    Ok(())
}

async fn exercise(manifest: &Manifest) -> Result<Value, String> {
    let root = &manifest.root;
    let old = tree(root)?;
    save(&root.join("before-tree.safe.json"), &old)?;
    let link = super::entry_link(&entry(root)).map_err(mapped)?;
    let plan = prepare(manifest, "2.1.280").await?;
    check(plan.installed_version == "2.1.278", "old_version_not_fixed")?;
    let journal_root = sources::journal_root().map_err(mapped)?;
    let mut expected_failure = None;
    let recover_invoked = false;
    if matches!(
        manifest.case.as_str(),
        "updated" | "unreviewed_downgrade_rejected"
    ) {
        check(
            sources::execute(plan, None).await.map_err(mapped)? == "2.1.280",
            "updated_version",
        )?;
        if manifest.case == "unreviewed_downgrade_rejected" {
            let updated = tree(root)?;
            let refused = inspect(manifest, "2.1.278").await.map_err(mapped)?;
            check(
                refused.error == Some(Error::InvalidRelease) && refused.plan.is_none(),
                "unreviewed_downgrade_not_rejected",
            )?;
            check(tree(root)? == updated, "downgrade_changed_installation")?;
            expected_failure = Some("InvalidRelease");
        }
    } else {
        let point = if manifest.case == "candidate_changed_preserved" {
            Point::Prepared
        } else {
            Point::Exchanged
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
        let reached =
            tokio::time::timeout(std::time::Duration::from_secs(300), reached_rx.recv()).await;
        if !matches!(reached, Ok(Ok(()))) {
            task.abort();
            let _ = task.await;
            return Err("fault_checkpoint_not_reached".into());
        }
        let journal_path = super::journal_path(&journal_root, CLIAgent::Claude);
        let raw = fs::read(&journal_path).map_err(|_| "journal_missing")?;
        let journal: super::Journal =
            serde_json::from_slice(&raw).map_err(|_| "journal_invalid")?;
        save_raw(&root.join("checkpoint-journal.safe.json"), &raw)?;
        if point == Point::Prepared {
            let candidate = journal
                .owner
                .package_root
                .parent()
                .ok_or("parent_missing")?
                .join(&journal.stage_name)
                .join("bin/claude.exe");
            fs::write(
                &candidate,
                b"owned fixture changed after official tree verification\n",
            )
            .map_err(|_| "candidate_change_failed")?;
            resume_tx.send(()).await.map_err(|_| "checkpoint_closed")?;
            check(
                task.await.map_err(|_| "update_task_failed")? == Err(Error::RecoveryRequired),
                "changed_candidate_not_preserved",
            )?;
            check(
                tree(root)? == old && journal_path.exists() && candidate.exists(),
                "candidate_failure_overwrote_state",
            )?;
            expected_failure = Some("RecoveryRequired");
        } else {
            task.abort();
            check(task.await.is_err(), "update_not_interrupted")?;
            if manifest.case == "external_change_preserved" {
                fs::write(
                    package(root).join("external-npm-change"),
                    b"later external install must survive\n",
                )
                .map_err(|_| "external_change_failed")?;
            }
            save(&root.join("before-link.safe.json"), &link)?;
            return Ok(
                json!({"case":manifest.case,"accepted":false,"needs_cold_recovery":true,
                "checkpoint":"exchange completed; durable journal remains SwapIntent",
                "original_version_probe_receipt_preserved":true,"model_inputs_sent":0}),
            );
        }
    }
    let after_link = super::entry_link(&entry(root)).map_err(mapped)?;
    check(
        serde_json::to_value(&link).unwrap() == serde_json::to_value(&after_link).unwrap(),
        "public_entry_changed",
    )?;
    let after = tree(root)?;
    verify_preserved_permissions(&old, &after)?;
    save(&root.join("after-tree.safe.json"), &after)?;
    let observed = sources::version(CLIAgent::Claude, &entry(root))
        .await
        .map_err(mapped)?;
    let expected = if matches!(
        manifest.case.as_str(),
        "swap_receipt_missing" | "candidate_changed_preserved"
    ) {
        "2.1.278"
    } else {
        "2.1.280"
    };
    check(observed == expected, "public_version_after")?;
    let mut receipts = Vec::new();
    let processes = journal_root.join("cli-agent-processes");
    for child in fs::read_dir(&processes).map_err(|_| "supervisor_generations_missing")? {
        let directory = child.map_err(|_| "supervisor_generation_invalid")?.path();
        let generation = directory
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| uuid::Uuid::parse_str(name).ok())
            .ok_or("generation_invalid")?;
        let receipt = managed_process::confirmed_exit(&journal_root, generation)
            .map_err(|_| "cleanup_failed")?
            .ok_or("cleanup_missing")?;
        check(receipt.cleanup_confirmed, "cleanup_not_confirmed")?;
        if manifest.case == "candidate_changed_preserved" {
            check(
                receipt.containment == "not_started",
                "changed_candidate_was_executed",
            )?;
        }
        let raw_receipt =
            fs::read(directory.join("exit.json")).map_err(|_| "raw_cleanup_receipt_missing")?;
        save_raw(
            &root.join(format!("exit-{generation}.safe.json")),
            &raw_receipt,
        )?;
        receipts.push(serde_json::to_value(receipt).map_err(|_| "receipt_invalid")?);
    }
    check(!receipts.is_empty(), "no_supervisor_evidence")?;
    Ok(
        json!({"case":manifest.case,"accepted":true,"public_version":observed,"expected_failure":expected_failure,
        "recover_invoked":recover_invoked,"entry_link_preserved":true,"cleanup_receipts":receipts,
        "model_inputs_sent":0,"npm_readonly_prefix_query_covered":true,"npm_install_or_lifecycle_executed":false,
        "busy_scope":"separate existing model state-machine gate, not this backend invocation"}),
    )
}

async fn cold_recover(manifest: &Manifest) -> Result<Value, String> {
    check(
        matches!(
            manifest.case.as_str(),
            "swap_receipt_missing" | "external_change_preserved"
        ),
        "cold_case_unknown",
    )?;
    let root = &manifest.root;
    let before: tree::Snapshot = serde_json::from_slice(
        &fs::read(root.join("before-tree.safe.json")).map_err(|_| "before_missing")?,
    )
    .map_err(|_| "before_invalid")?;
    let saved: super::Journal = serde_json::from_slice(
        &fs::read(root.join("checkpoint-journal.safe.json")).map_err(|_| "checkpoint_missing")?,
    )
    .map_err(|_| "checkpoint_invalid")?;
    check(
        saved.phase == super::Phase::SwapIntent && saved.owner.package_root == package(root),
        "cold_checkpoint_invalid",
    )?;
    let journal_root = sources::journal_root().map_err(mapped)?;
    let result = inspect(manifest, "2.1.280").await;
    let version;
    if manifest.case == "swap_receipt_missing" {
        let report = result.map_err(mapped)?;
        check(
            report.installed_version == "2.1.278" && report.source == Source::Npm,
            "cold_inspect_version",
        )?;
        check(
            tree(root)? == before && !super::journal_path(&journal_root, CLIAgent::Claude).exists(),
            "cold_rollback_mismatch",
        )?;
        version = "2.1.278";
    } else {
        check(
            matches!(result, Err(Error::RecoveryRequired)),
            "cold_external_change_not_rejected",
        )?;
        check(
            fs::read(package(root).join("external-npm-change"))
                .ok()
                .as_deref()
                == Some(b"later external install must survive\n"),
            "cold_external_change_overwritten",
        )?;
        let backup = saved
            .owner
            .package_root
            .parent()
            .ok_or("parent_missing")?
            .join(&saved.stage_name);
        check(
            tree::Directory::open(&backup)
                .and_then(|directory| directory.snapshot())
                .map_err(mapped)?
                == before,
            "cold_backup_changed",
        )?;
        check(
            super::journal_path(&journal_root, CLIAgent::Claude).exists(),
            "cold_journal_not_preserved",
        )?;
        version = "2.1.280";
    }
    let expected_link: Value = serde_json::from_slice(
        &fs::read(root.join("before-link.safe.json")).map_err(|_| "link_missing")?,
    )
    .map_err(|_| "link_invalid")?;
    check(
        serde_json::to_value(super::entry_link(&entry(root)).map_err(mapped)?).unwrap()
            == expected_link,
        "cold_public_link_changed",
    )?;
    let probe = saved.probe.ok_or("cold_probe_missing")?;
    let binding =
        managed_process::PreparedLaunchBinding::claude_npm_version_probe(probe.binding_digest)
            .map_err(|_| "cold_binding_invalid")?;
    let receipt =
        managed_process::confirmed_exit_with_binding(&journal_root, probe.generation, &binding)
            .map_err(|_| "cold_receipt_invalid")?
            .ok_or("cold_receipt_missing")?;
    check(
        receipt.cleanup_confirmed && receipt.exit_code == Some(0),
        "cold_probe_not_cleaned",
    )?;
    let after = tree(root)?;
    verify_preserved_permissions(&before, &after)?;
    save(&root.join("after-tree.safe.json"), &after)?;
    let raw_receipt = fs::read(
        journal_root
            .join("cli-agent-processes")
            .join(probe.generation.to_string())
            .join("exit.json"),
    )
    .map_err(|_| "raw_cold_receipt_missing")?;
    save_raw(&root.join("cold-exit.safe.json"), &raw_receipt)?;
    Ok(
        json!({"case":manifest.case,"accepted":true,"cold_recovery_process":true,"public_version":version,
        "model_inputs_sent":0,"cleanup_confirmed":true,"source_discovery_prefix_query_covered":manifest.case == "swap_receipt_missing"}),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要明确的私有官方 npm fixture 和同源码内置产品二进制"]
async fn real_claude_npm_update_without_model() {
    let path = PathBuf::from(
        std::env::var_os("INFINISHELL_CLI_NPM_UPDATE_MANIFEST").expect("缺少私有清单"),
    );
    let metadata = fs::symlink_metadata(&path).expect("无法读取私有清单元数据");
    assert!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.mode() & 0o7777 == 0o600
            && metadata.uid() == unsafe { libc::geteuid() }
    );
    let bytes = fs::read(&path).expect("无法读取私有清单");
    assert!(bytes.len() <= 65536);
    let manifest: Manifest = serde_json::from_slice(&bytes).expect("清单格式错误");
    validate(&manifest, &path).expect("验收授权、环境或构建绑定不匹配");
    save(
        &manifest.root.join(format!("started-{}.safe.json", step())),
        &json!({"scope":SCOPE,"case":manifest.case,"source_sha256":manifest.source_sha256}),
    )
    .unwrap();
    let result = EVIDENCE
        .scope(manifest.root.clone(), async {
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
        "产品 npm 事务验收失败，原始结果已保留：{result:?}"
    );
}

#[tokio::test]
async fn fixed_npm_release_input_is_scoped_and_never_enables_future_versions() {
    assert!(fixed_release(CLIAgent::Claude, Channel::Latest).is_none());
    RELEASE
        .scope("2.1.280".to_owned(), async {
            assert_eq!(
                fixed_release(CLIAgent::Claude, Channel::Latest).as_deref(),
                Some("2.1.280")
            );
            assert!(fixed_release(CLIAgent::Codex, Channel::Latest).is_none());
            assert!(fixed_release(CLIAgent::Claude, Channel::Stable).is_none());
        })
        .await;
    RELEASE
        .scope("2.1.281".to_owned(), async {
            assert!(fixed_release(CLIAgent::Claude, Channel::Latest).is_none());
        })
        .await;
    assert!(fixed_release(CLIAgent::Claude, Channel::Latest).is_none());
}

#[tokio::test]
async fn fault_checkpoints_do_nothing_outside_the_explicit_live_scope() {
    checkpoint(Point::Prepared).await;
    checkpoint(Point::Exchanged).await;
    assert!(EVIDENCE.try_with(Clone::clone).is_err());
}
