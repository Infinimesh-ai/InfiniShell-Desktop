//! 官方固定映像与人工私有 cask 登记的后端事务验收；不代表消费者 Homebrew 安装来源。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::super::{ArgumentRef, ArtifactRole, BoundInvocation, Channel, Installation, Source};
use super::{
    CLIAgent, Directory, Error, Identity, Journal, Link, Phase, Snapshot, UpdatePlan,
    managed_process,
};
use crate::terminal::cli_agent_updates::sources;

const SCOPE: &str = "claude_brew_backend_no_model_v1";
const MARKER: &[u8] =
    b"InfiniShell private cask backend fixture; synthetic registration; no credentials\n";
const OLD: &str = "2.1.278";
const TARGET: &str = "2.1.280";
const TOKEN: &str = "claude-code@latest";
const CHANGED: &[u8] = b"owned candidate changed after complete prepared snapshot\n";
const EXTERNAL: &[u8] = b"external cask mutation must survive recovery\n";
const CONTRACT: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/claude_21280_brew_manifest.json");

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
    worker: Binary,
    supervisor: Binary,
    inputs: BTreeMap<String, Binary>,
    source_sha256: BTreeMap<String, String>,
}
#[derive(Clone)]
struct Scope {
    manifest: Manifest,
    manifest_stamp: sources::Stamp,
    manifest_identity: (u64, u64),
    marker_identity: (u64, u64),
    root_identity: Identity,
    prefix_identity: Identity,
}
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
tokio::task_local! { static FIXTURE: Arc<Scope>; static CONTROL: Control; }

fn check(value: bool, reason: &str) -> Result<(), String> {
    if value { Ok(()) } else { Err(reason.into()) }
}
fn mapped(error: Error) -> String {
    format!("{error:?}")
}
fn digest(path: &Path) -> Result<String, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| "file_open")?;
    let info = file.metadata().map_err(|_| "file_metadata")?;
    check(info.is_file() && info.nlink() == 1, "file_type")?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let size = file.read(&mut buffer).map_err(|_| "file_read")?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn raw(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| "evidence_exists")?;
    file.write_all(bytes).map_err(|_| "evidence_write")?;
    file.sync_all().map_err(|_| "evidence_sync".into())
}
fn save(path: &Path, value: &impl Serialize) -> Result<(), String> {
    raw(
        path,
        &serde_json::to_vec_pretty(value).map_err(|_| "evidence_json")?,
    )
}
fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&sources::read_limited(path, 12 * sources::MAX_CONFIG).map_err(mapped)?)
        .map_err(|_| "evidence_parse".into())
}
fn entry(root: &Path) -> PathBuf {
    root.join("prefix/bin/claude")
}
fn package(root: &Path) -> PathBuf {
    root.join("prefix/Caskroom").join(TOKEN)
}
fn state(root: &Path) -> PathBuf {
    root.join("state")
}
fn journal_path(root: &Path) -> PathBuf {
    super::journal_path(&state(root), CLIAgent::Claude)
}
fn snapshot(path: &Path) -> Result<Snapshot, String> {
    Directory::open(path)
        .and_then(|directory| directory.snapshot())
        .map_err(mapped)
}
fn env_paths() -> [(&'static str, &'static str); 12] {
    [
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
fn step() -> String {
    std::env::var("INFINISHELL_CLAUDE_BREW_STEP").unwrap_or_default()
}

fn private_file_identity(path: &Path) -> Option<(u64, u64)> {
    let metadata = fs::symlink_metadata(path).ok()?;
    (metadata.is_file()
        && metadata.nlink() == 1
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o7777 == 0o600
        && path.canonicalize().ok().as_deref() == Some(path))
    .then_some((metadata.dev(), metadata.ino()))
}

impl Scope {
    fn valid(&self) -> bool {
        let root = &self.manifest.root;
        root.canonicalize().ok().as_ref() == Some(root)
            && private_file_identity(&root.join("manifest.private.json"))
                == Some(self.manifest_identity)
            && private_file_identity(&root.join(".infinishell-cask-live"))
                == Some(self.marker_identity)
            && sources::stamp(&root.join("manifest.private.json"))
                .ok()
                .as_ref()
                == Some(&self.manifest_stamp)
            && fs::read(root.join(".infinishell-cask-live"))
                .ok()
                .as_deref()
                == Some(MARKER)
            && Directory::open(root)
                .and_then(|directory| directory.identity())
                .ok()
                .as_ref()
                == Some(&self.root_identity)
            && Directory::open(&root.join("prefix"))
                .and_then(|directory| directory.identity())
                .ok()
                .as_ref()
                == Some(&self.prefix_identity)
    }
}

pub(super) fn allows_private_prefix(prefix: &Path, journal_root: &Path) -> bool {
    FIXTURE
        .try_with(|scope| {
            scope.valid()
                && prefix == scope.manifest.root.join("prefix")
                && journal_root == state(&scope.manifest.root)
        })
        .unwrap_or(false)
}

/// 只有完整验证后的 task 才能重放固定官方原件；生产下载分支不变。
pub(super) fn copy_fixed_download(
    url: &str,
    output: &mut File,
    limit: u64,
) -> Option<Result<(), Error>> {
    FIXTURE
        .try_with(|scope| {
            if !scope.valid() {
                return Err(Error::SourceChanged);
            }
            let input = scope
                .manifest
                .inputs
                .get(url)
                .ok_or(Error::InvalidRelease)?;
            if input.path.canonicalize().ok().as_ref() != Some(&input.path) {
                return Err(Error::SourceChanged);
            }
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(&input.path)
                .map_err(|_| Error::SourceChanged)?;
            let metadata = file.metadata().map_err(|_| Error::SourceChanged)?;
            if !metadata.is_file()
                || metadata.nlink() != 1
                || metadata.len() == 0
                || metadata.len() > limit
            {
                return Err(Error::InvalidRelease);
            }
            let mut copied = 0_u64;
            let mut hash = Sha256::new();
            let mut buffer = [0; 65536];
            loop {
                let count = file.read(&mut buffer).map_err(|_| Error::SourceChanged)?;
                if count == 0 {
                    break;
                }
                copied = copied
                    .checked_add(count as u64)
                    .ok_or(Error::InvalidRelease)?;
                if copied > metadata.len() || copied > limit {
                    return Err(Error::InvalidRelease);
                }
                hash.update(&buffer[..count]);
                output
                    .write_all(&buffer[..count])
                    .map_err(|_| Error::PersistenceFailed)?;
            }
            if copied != metadata.len()
                || format!("{:x}", hash.finalize()) != input.sha256
                || digest(&input.path).ok().as_ref() != Some(&input.sha256)
            {
                return Err(Error::SourceChanged);
            }
            Ok(())
        })
        .ok()
}

pub(super) async fn checkpoint(point: Point, path: &Path) {
    if let Ok(scope) = FIXTURE.try_with(Arc::clone) {
        assert!(scope.valid(), "测试作用域身份发生变化");
        assert_eq!(path, journal_path(&scope.manifest.root));
        let name = match point {
            Point::Prepared => "prepared-journal.safe.json",
            Point::Exchanged => "exchanged-journal.safe.json",
        };
        raw(&scope.manifest.root.join(name), &fs::read(path).unwrap()).unwrap();
        if let Ok(control) = CONTROL.try_with(Clone::clone) {
            if control.point == point {
                control.reached.send(()).await.unwrap();
                control.resume.recv().await.unwrap();
            }
        }
    }
}

pub(super) fn preserve_stdout(generation: uuid::Uuid, bytes: &[u8]) {
    let _ = FIXTURE.try_with(|scope| {
        assert!(scope.valid(), "测试作用域身份发生变化");
        raw(
            &scope
                .manifest
                .root
                .join(format!("candidate-{generation}.stdout")),
            bytes,
        )
        .unwrap();
    });
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
        "app/src/terminal/cli_agent_updates/sources_brew.rs",
        include_bytes!("sources_brew.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_brew_transaction.rs",
        include_bytes!("sources_brew_transaction.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_npm_tree_unix.rs",
        include_bytes!("sources_npm_tree_unix.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_brew_transaction_tests.rs",
        include_bytes!("sources_brew_transaction_tests.rs"),
    ),
    (
        "app/src/terminal/cli_agent_updates/sources_claude_brew_live_tests.rs",
        include_bytes!("sources_claude_brew_live_tests.rs"),
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
        "app/src/ai/cli_agent_runtime/managed_process_macos.rs",
        include_bytes!("../../ai/cli_agent_runtime/managed_process_macos.rs"),
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
        "script/cli-agent-parity/claude_21280_brew_manifest.json",
        CONTRACT,
    ),
    (
        "script/cli-agent-parity/run_claude_brew_update_live.py",
        include_bytes!("../../../../script/cli-agent-parity/run_claude_brew_update_live.py"),
    ),
];

fn validate(manifest: Manifest, path: &Path) -> Result<Scope, String> {
    let root = &manifest.root;
    let metadata = fs::symlink_metadata(path).map_err(|_| "manifest_missing")?;
    check(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.mode() & 0o7777 == 0o600
            && metadata.uid() == unsafe { libc::geteuid() },
        "manifest_identity",
    )?;
    let root_info = fs::symlink_metadata(root).map_err(|_| "fixture_missing")?;
    check(
        manifest.schema == 1
            && manifest.scope == SCOPE
            && path == root.join("manifest.private.json")
            && root.canonicalize().ok().as_ref() == Some(root)
            && root_info.is_dir()
            && root_info.uid() == unsafe { libc::geteuid() }
            && root_info.mode() & 0o7777 == 0o700
            && fs::read(root.join(".infinishell-cask-live"))
                .ok()
                .as_deref()
                == Some(MARKER),
        "fixture_contract",
    )?;
    check(
        [
            "updated",
            "swap_receipt_missing",
            "external_change_preserved",
            "candidate_changed_preserved",
        ]
        .contains(&manifest.case.as_str()),
        "case_unknown",
    )?;
    check(
        matches!(step().as_str(), "execute" | "recover"),
        "step_unknown",
    )?;
    for (name, relative) in env_paths() {
        check(
            std::env::var_os(name).map(PathBuf::from) == Some(root.join(relative))
                && root.join(relative).canonicalize().ok() == Some(root.join(relative)),
            "environment_path",
        )?;
    }
    check(
        std::env::current_dir().ok() == Some(root.join("project"))
            && std::env::var("INFINISHELL_CLAUDE_BREW_ALLOW").as_deref() == Ok(SCOPE)
            && std::env::var_os("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE").map(PathBuf::from)
                == Some(manifest.supervisor.path.clone()),
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
            "INFINISHELL_CLAUDE_BREW_ALLOW",
            "INFINISHELL_CLAUDE_BREW_STEP",
            "INFINISHELL_CLAUDE_BREW_MANIFEST",
            "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE",
        ])
        .collect::<std::collections::BTreeSet<_>>();
    for (name, value) in std::env::vars() {
        let apple = name == "__CF_USER_TEXT_ENCODING"
            && value == format!("0x{:X}:0x0:0x0", unsafe { libc::getuid() });
        check(
            allowed.contains(name.as_str()) || apple,
            "inherited_environment",
        )?;
    }
    for binary in [&manifest.worker, &manifest.supervisor] {
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
        "worker_identity",
    )?;
    check(
        manifest.source_sha256.len() == SOURCES.len(),
        "source_member_set",
    )?;
    for (name, bytes) in SOURCES {
        check(
            manifest.source_sha256.get(*name) == Some(&format!("{:x}", Sha256::digest(bytes))),
            "compiled_source_binding",
        )?;
    }
    let contract: Value = serde_json::from_slice(CONTRACT).map_err(|_| "contract_parse")?;
    let originals = contract["official_inputs"]
        .as_object()
        .ok_or("contract_inputs")?;
    check(
        manifest.inputs.len() == originals.len() + 1,
        "input_member_set",
    )?;
    for (url, expected) in originals
        .iter()
        .map(|(url, value)| (url.as_str(), value))
        .chain(std::iter::once((
            contract["metadata"]["url"].as_str().ok_or("metadata_url")?,
            &contract["metadata"],
        )))
    {
        let input = manifest.inputs.get(url).ok_or("input_missing")?;
        check(
            input.path.canonicalize().ok().as_ref() == Some(&input.path)
                && digest(&input.path)? == input.sha256
                && Some(input.sha256.as_str()) == expected["sha256"].as_str()
                && fs::metadata(&input.path)
                    .map_err(|_| "input_missing")?
                    .len()
                    == expected["length"].as_u64().ok_or("input_length")?,
            "fixed_input_binding",
        )?;
    }
    check(
        fs::read(root.join("prefix/bin/brew")).ok().as_deref()
            == Some(b"synthetic manager identity; never execute\n"),
        "synthetic_manager_identity",
    )?;
    let scope = Scope {
        manifest_stamp: sources::stamp(path).map_err(mapped)?,
        manifest_identity: private_file_identity(path).ok_or("manifest_identity")?,
        marker_identity: private_file_identity(&root.join(".infinishell-cask-live"))
            .ok_or("marker_identity")?,
        root_identity: Directory::open(root)
            .and_then(|directory| directory.identity())
            .map_err(mapped)?,
        prefix_identity: Directory::open(&root.join("prefix"))
            .and_then(|directory| directory.identity())
            .map_err(mapped)?,
        manifest,
    };
    check(scope.valid(), "scope_identity")?;
    Ok(scope)
}

fn plan(root: &Path) -> Result<UpdatePlan, String> {
    let public = entry(root);
    let manager = sources::stamp(&root.join("prefix/bin/brew")).map_err(mapped)?;
    let receipt = package(root).join(".metadata/INSTALL_RECEIPT.json");
    let invocation = BoundInvocation::manual_only(
        [
            (ArtifactRole::Program, manager.canonical.clone()),
            (ArtifactRole::Manager, manager.canonical.clone()),
            (ArtifactRole::Entry, public.clone()),
            (ArtifactRole::InstallRoot, root.join("prefix")),
            (ArtifactRole::DependencyRoot, package(root)),
            (ArtifactRole::Registration, receipt.clone()),
        ],
        ArtifactRole::Program,
        ["upgrade", "--cask", "--greedy", TOKEN]
            .into_iter()
            .map(|argument| ArgumentRef::Literal(argument.into()))
            .collect(),
        "私有人工登记仅进入后端，不执行 brew",
    );
    Ok(UpdatePlan {
        agent: CLIAgent::Claude,
        installation: Installation {
            source: Source::Homebrew,
            stamp: sources::stamp(&public).map_err(mapped)?,
            entry: public,
            manager: Some(manager),
            helper: None,
            registration: Some((receipt.clone(), sources::stamp(&receipt).map_err(mapped)?)),
            invocation: Some(invocation),
            channel: Channel::Latest,
            config: None,
            error: None,
            source_target: Some(TARGET.into()),
        },
        installed_version: OLD.into(),
        target_version: TARGET.into(),
        config: None,
        intent: "claude-brew-backend-private-fixed".into(),
        downgrade: None,
        #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
        claude_npm_platform: None,
    })
}

async fn public_version(root: &Path, expected: &str, label: &str) -> Result<String, String> {
    let contract: Value = serde_json::from_slice(CONTRACT).map_err(|_| "contract_parse")?;
    let url =
        format!("https://downloads.claude.ai/claude-code-releases/{expected}/darwin-arm64/claude");
    let canonical = entry(root)
        .canonicalize()
        .map_err(|_| "public_entry_broken")?;
    check(
        canonical == package(root).join(expected).join("claude")
            && Some(digest(&canonical)?.as_str())
                == contract["official_inputs"][&url]["sha256"].as_str(),
        "public_image_binding",
    )?;
    let output = sources::run(
        &sources::Invocation::new(entry(root), ["--version"]),
        sources::PROBE_TIMEOUT,
    )
    .await
    .map_err(mapped)?;
    raw(&root.join(format!("public-{label}.stdout")), &output)?;
    save(
        &root.join(format!("public-{label}.safe.json")),
        &json!({"program":entry(root),"arguments":["--version"],"canonical":canonical,"expected":expected,"image_sha256":digest(&canonical)?}),
    )?;
    let observed = sources::parse_cli_agent_version(
        CLIAgent::Claude,
        std::str::from_utf8(&output).map_err(|_| "public_stdout_utf8")?,
    )
    .ok_or("public_version_invalid")?;
    check(observed == expected, "public_version_mismatch")?;
    Ok(observed)
}

fn generation_set(root: &Path) -> Result<Vec<String>, String> {
    let path = state(root).join("cli-agent-processes");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut names = fs::read_dir(path)
        .map_err(|_| "generations_read")?
        .map(|item| {
            item.map_err(|_| "generation_read").and_then(|entry| {
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "generation_name")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}
fn cleanup(root: &Path, journal: &Journal, label: &str) -> Result<bool, String> {
    let probe = journal.probe.as_ref().ok_or("probe_missing")?;
    let binding = super::probe_binding(CLIAgent::Claude, probe.digest.clone()).map_err(mapped)?;
    let receipt =
        managed_process::confirmed_exit_with_binding(&state(root), probe.generation, &binding)
            .map_err(|_| "exit_invalid")?
            .ok_or("exit_missing")?;
    check(
        receipt.cleanup_confirmed
            && receipt.exit_code == Some(0)
            && probe.completed
            && probe.version.as_deref() == Some(TARGET),
        "candidate_cleanup",
    )?;
    let output = fs::read(root.join(format!("candidate-{}.stdout", probe.generation)))
        .map_err(|_| "candidate_stdout_missing")?;
    check(
        probe.output_sha256.as_deref() == Some(format!("{:x}", Sha256::digest(&output)).as_str())
            && sources::parse_cli_agent_version(
                CLIAgent::Claude,
                std::str::from_utf8(&output).map_err(|_| "candidate_stdout_utf8")?,
            )
            .as_deref()
                == Some(TARGET),
        "candidate_stdout_binding",
    )?;
    raw(
        &root.join(format!("{label}-exit.safe.json")),
        &fs::read(
            state(root)
                .join("cli-agent-processes")
                .join(probe.generation.to_string())
                .join("exit.json"),
        )
        .map_err(|_| "exit_raw_missing")?,
    )?;
    Ok(true)
}
fn verify_config(root: &Path) -> Result<(), String> {
    check(
        fs::read(package(root).join(".metadata/config.json")).map_err(|_| "cask_config_missing")?
            == fs::read(root.join("before-cask-config.raw"))
                .map_err(|_| "before_cask_config_missing")?
            && fs::read(root.join("home/.claude/settings.json"))
                .map_err(|_| "user_config_missing")?
                == fs::read(root.join("before-user-config.raw"))
                    .map_err(|_| "before_user_config_missing")?,
        "config_changed",
    )
}

async fn exercise(scope: Arc<Scope>) -> Result<Value, String> {
    let manifest = &scope.manifest;
    let root = &manifest.root;
    let before = snapshot(&package(root))?;
    let old_link = super::link(&entry(root)).map_err(mapped)?;
    save(&root.join("before-tree.safe.json"), &before)?;
    save(&root.join("before-link.safe.json"), &old_link)?;
    raw(
        &root.join("before-cask-config.raw"),
        &fs::read(package(root).join(".metadata/config.json")).map_err(|_| "config_missing")?,
    )?;
    raw(
        &root.join("before-user-config.raw"),
        &fs::read(root.join("home/.claude/settings.json")).map_err(|_| "config_missing")?,
    )?;
    public_version(root, OLD, "before").await?;
    check(generation_set(root)?.is_empty(), "fixture_has_generations")?;
    let plan = plan(root)?;
    if manifest.case == "updated" {
        check(
            super::execute(&plan, &state(root), None)
                .await
                .map_err(mapped)?
                == TARGET,
            "updated_return",
        )?;
        check(!journal_path(root).exists(), "committed_journal_remains")?;
        let exchanged: Journal = load(&root.join("exchanged-journal.safe.json"))?;
        check(
            snapshot(&package(root))? == exchanged.prepared.clone().ok_or("prepared_missing")?
                && super::link(&entry(root)).map_err(mapped)?
                    == exchanged
                        .prepared_link
                        .clone()
                        .ok_or("prepared_link_missing")?
                && !exchanged.parent().join(exchanged.stage_name()).exists()
                && fs::symlink_metadata(exchanged.link_stage()).is_err(),
            "published_state",
        )?;
        let cleaned = cleanup(root, &exchanged, "candidate")?;
        verify_config(root)?;
        save(
            &root.join("after-tree.safe.json"),
            &snapshot(&package(root))?,
        )?;
        save(
            &root.join("after-link.safe.json"),
            &super::link(&entry(root)).map_err(mapped)?,
        )?;
        let version = public_version(root, TARGET, "after").await?;
        return Ok(
            json!({"case":manifest.case,"accepted":true,"public_version":version,"cleanup_confirmed":cleaned,"model_inputs_sent":0}),
        );
    }
    let point = if manifest.case == "candidate_changed_preserved" {
        Point::Prepared
    } else {
        Point::Exchanged
    };
    let (reached_tx, reached_rx) = async_channel::bounded(1);
    let (resume_tx, resume_rx) = async_channel::bounded(1);
    let journal_root = state(root);
    let task = tokio::spawn(FIXTURE.scope(
        Arc::clone(&scope),
        CONTROL.scope(
            Control {
                point,
                reached: reached_tx,
                resume: resume_rx,
            },
            async move { super::execute(&plan, &journal_root, None).await },
        ),
    ));
    if !matches!(
        tokio::time::timeout(std::time::Duration::from_secs(300), reached_rx.recv()).await,
        Ok(Ok(()))
    ) {
        task.abort();
        let _ = task.await;
        return Err("fault_checkpoint_not_reached".into());
    }
    let bytes = fs::read(journal_path(root)).map_err(|_| "checkpoint_missing")?;
    let journal: Journal = serde_json::from_slice(&bytes).map_err(|_| "checkpoint_shape")?;
    raw(&root.join("checkpoint-journal.safe.json"), &bytes)?;
    if point == Point::Prepared {
        check(
            journal.phase == Phase::Prepared && journal.probe.is_none(),
            "prepared_checkpoint",
        )?;
        let candidate = journal
            .parent()
            .join(journal.stage_name())
            .join(TARGET)
            .join("claude");
        fs::write(&candidate, CHANGED).map_err(|_| "candidate_mutation")?;
        let changed = snapshot(&journal.parent().join(journal.stage_name()))?;
        save(&root.join("mutated-stage.safe.json"), &changed)?;
        resume_tx.send(()).await.map_err(|_| "checkpoint_resume")?;
        check(
            task.await.map_err(|_| "update_task_join")? == Err(Error::RecoveryRequired),
            "candidate_change_not_rejected",
        )?;
        check(
            snapshot(&package(root))? == before
                && super::link(&entry(root)).map_err(mapped)? == old_link
                && snapshot(&journal.parent().join(journal.stage_name()))? == changed
                && fs::read(journal_path(root)).map_err(|_| "retained_journal_missing")? == bytes
                && generation_set(root)?.is_empty(),
            "candidate_rejection_not_preserved",
        )?;
        verify_config(root)?;
        save(
            &root.join("after-tree.safe.json"),
            &snapshot(&package(root))?,
        )?;
        save(
            &root.join("after-link.safe.json"),
            &super::link(&entry(root)).map_err(mapped)?,
        )?;
        let version = public_version(root, OLD, "after").await?;
        return Ok(
            json!({"case":manifest.case,"accepted":true,"public_version":version,"candidate_never_started":true,"probe":null,"expected_failure":"RecoveryRequired","model_inputs_sent":0}),
        );
    }
    check(
        journal.phase == Phase::ExchangeIntent,
        "exchange_checkpoint",
    )?;
    let cleaned = cleanup(root, &journal, "candidate")?;
    task.abort();
    check(
        task.await.is_err_and(|error| error.is_cancelled()),
        "update_not_interrupted",
    )?;
    if manifest.case == "external_change_preserved" {
        raw(&package(root).join("external-cask-change"), EXTERNAL)?;
    }
    save(
        &root.join("checkpoint-tree.safe.json"),
        &snapshot(&package(root))?,
    )?;
    save(
        &root.join("checkpoint-link.safe.json"),
        &super::link(&entry(root)).map_err(mapped)?,
    )?;
    save(
        &root.join("checkpoint-backup.safe.json"),
        &snapshot(&journal.parent().join(journal.stage_name()))?,
    )?;
    save(
        &root.join("checkpoint-generations.safe.json"),
        &generation_set(root)?,
    )?;
    Ok(
        json!({"case":manifest.case,"accepted":false,"needs_cold_recovery":true,"cleanup_confirmed":cleaned,
        "execute_pid":std::process::id(),"checkpoint":"directory and public link exchanged; durable journal remains ExchangeIntent","model_inputs_sent":0}),
    )
}

async fn cold_recover(manifest: &Manifest) -> Result<Value, String> {
    let root = &manifest.root;
    check(
        matches!(
            manifest.case.as_str(),
            "swap_receipt_missing" | "external_change_preserved"
        ),
        "recovery_case",
    )?;
    let execute_result: Value = load(&root.join("result-execute.safe.json"))?;
    check(
        execute_result["needs_cold_recovery"] == true
            && execute_result["execute_pid"].as_u64() != Some(u64::from(std::process::id())),
        "not_cold_process",
    )?;
    let journal: Journal = load(&root.join("checkpoint-journal.safe.json"))?;
    check(journal.phase == Phase::ExchangeIntent, "cold_phase")?;
    let before: Snapshot = load(&root.join("before-tree.safe.json"))?;
    let old_link: Link = load(&root.join("before-link.safe.json"))?;
    let result = super::recover(CLIAgent::Claude, &entry(root), &state(root));
    // 在断言前保留实际状态，拒绝被实现顺序意外破坏的公共链接。
    save(
        &root.join("after-link.safe.json"),
        &super::link(&entry(root)).map_err(mapped)?,
    )?;
    save(
        &root.join("after-tree.safe.json"),
        &snapshot(&package(root))?,
    )?;
    let expected;
    if manifest.case == "swap_receipt_missing" {
        check(result == Ok(None), "cold_rollback_return")?;
        check(
            snapshot(&package(root))? == before
                && super::link(&entry(root)).map_err(mapped)? == old_link
                && !journal_path(root).exists()
                && !journal.parent().join(journal.stage_name()).exists()
                && fs::symlink_metadata(journal.link_stage()).is_err(),
            "cold_rollback_state",
        )?;
        expected = OLD;
    } else {
        check(
            result == Err(Error::RecoveryRequired),
            "external_change_not_rejected",
        )?;
        let checkpoint: Snapshot = load(&root.join("checkpoint-tree.safe.json"))?;
        let checkpoint_link: Link = load(&root.join("checkpoint-link.safe.json"))?;
        check(
            snapshot(&package(root))? == checkpoint
                && super::link(&entry(root)).map_err(mapped)? == checkpoint_link
                && snapshot(&journal.parent().join(journal.stage_name()))? == before
                && fs::read(package(root).join("external-cask-change"))
                    .ok()
                    .as_deref()
                    == Some(EXTERNAL)
                && fs::read(journal_path(root)).map_err(|_| "journal_missing")?
                    == fs::read(root.join("checkpoint-journal.safe.json"))
                        .map_err(|_| "checkpoint_missing")?,
            "external_state_changed",
        )?;
        expected = TARGET;
    }
    check(
        generation_set(root)?
            == load::<Vec<String>>(&root.join("checkpoint-generations.safe.json"))?,
        "cold_started_candidate",
    )?;
    let cleaned = cleanup(root, &journal, "cold")?;
    verify_config(root)?;
    let version = public_version(root, expected, "recover").await?;
    Ok(
        json!({"case":manifest.case,"accepted":true,"public_version":version,"cold_recovery_process":true,
        "cleanup_confirmed":cleaned,"new_candidate_generation":false,"model_inputs_sent":0}),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要显式私有 cask fixture、固定官方映像及同源码签名监督程序"]
async fn real_claude_brew_backend_without_model() {
    let path =
        PathBuf::from(std::env::var_os("INFINISHELL_CLAUDE_BREW_MANIFEST").expect("缺少私有清单"));
    let scope = Arc::new(
        validate(load(&path).expect("私有清单无效"), &path).expect("私有 fixture 绑定无效"),
    );
    let root = scope.manifest.root.clone();
    save(&root.join(format!("started-{}.safe.json", step())), &json!({"scope":SCOPE,"case":scope.manifest.case,
        "source_sha256":scope.manifest.source_sha256,"pid":std::process::id(),"registration":"synthetic_private_fixture","consumer_source_discovery_covered":false})).unwrap();
    let result = FIXTURE
        .scope(Arc::clone(&scope), async {
            if step() == "recover" {
                cold_recover(&scope.manifest).await
            } else {
                exercise(Arc::clone(&scope)).await
            }
        })
        .await;
    let evidence = match &result {
        Ok(value) => value.clone(),
        Err(error) => {
            json!({"case":scope.manifest.case,"accepted":false,"error":error,"model_inputs_sent":0})
        }
    };
    save(
        &root.join(format!("result-{}.safe.json", step())),
        &evidence,
    )
    .unwrap();
    assert!(
        result.is_ok(),
        "cask 后端验收失败，原始证据已保留：{result:?}"
    );
}

fn unit_scope() -> (tempfile::TempDir, Arc<Scope>) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    fs::create_dir(root.join("prefix")).unwrap();
    raw(
        &root.join("manifest.private.json"),
        b"unit fixture identity\n",
    )
    .unwrap();
    raw(&root.join(".infinishell-cask-live"), MARKER).unwrap();
    let unused = Binary {
        path: root.join("never-executed"),
        sha256: String::new(),
    };
    let scope = Scope {
        manifest: Manifest {
            schema: 1,
            scope: SCOPE.into(),
            case: "updated".into(),
            root: root.clone(),
            worker: unused.clone(),
            supervisor: unused,
            inputs: BTreeMap::new(),
            source_sha256: BTreeMap::new(),
        },
        manifest_stamp: sources::stamp(&root.join("manifest.private.json")).unwrap(),
        manifest_identity: private_file_identity(&root.join("manifest.private.json")).unwrap(),
        marker_identity: private_file_identity(&root.join(".infinishell-cask-live")).unwrap(),
        root_identity: Directory::open(&root).unwrap().identity().unwrap(),
        prefix_identity: Directory::open(&root.join("prefix"))
            .unwrap()
            .identity()
            .unwrap(),
    };
    (temporary, Arc::new(scope))
}

#[tokio::test]
async fn claude_brew_private_prefix_stays_in_validated_task() {
    let (_temporary, scope) = unit_scope();
    let root = scope.manifest.root.clone();
    assert!(!allows_private_prefix(&root.join("prefix"), &state(&root)));
    FIXTURE
        .scope(scope, async {
            assert!(allows_private_prefix(&root.join("prefix"), &state(&root)));
            assert!(!allows_private_prefix(
                &root.join("prefix/other"),
                &state(&root)
            ));
            assert!(!allows_private_prefix(
                &root.join("prefix"),
                &root.join("other-state")
            ));
            let other_root = root.clone();
            assert!(
                !tokio::spawn(async move {
                    allows_private_prefix(&other_root.join("prefix"), &state(&other_root))
                })
                .await
                .unwrap()
            );
        })
        .await;
    assert!(!allows_private_prefix(&root.join("prefix"), &state(&root)));
}

#[tokio::test]
async fn claude_brew_private_prefix_rejects_marker_change() {
    let (_temporary, scope) = unit_scope();
    let root = scope.manifest.root.clone();
    FIXTURE
        .scope(scope, async {
            fs::write(root.join(".infinishell-cask-live"), b"changed").unwrap();
            assert!(!allows_private_prefix(&root.join("prefix"), &state(&root)));
        })
        .await;
}

#[tokio::test]
async fn claude_brew_private_prefix_rejects_manifest_change() {
    let (_temporary, scope) = unit_scope();
    let root = scope.manifest.root.clone();
    FIXTURE
        .scope(scope, async {
            fs::write(root.join("manifest.private.json"), b"changed").unwrap();
            assert!(!allows_private_prefix(&root.join("prefix"), &state(&root)));
        })
        .await;
}

#[tokio::test]
async fn claude_brew_private_prefix_rejects_directory_replacement() {
    let (_temporary, scope) = unit_scope();
    let root = scope.manifest.root.clone();
    FIXTURE
        .scope(scope, async {
            fs::rename(root.join("prefix"), root.join("original-prefix")).unwrap();
            fs::create_dir(root.join("prefix")).unwrap();
            assert!(!allows_private_prefix(&root.join("prefix"), &state(&root)));
        })
        .await;
}

#[tokio::test]
async fn claude_brew_fixed_download_rejects_unlisted_url_without_fallback() {
    let (_temporary, scope) = unit_scope();
    let mut output = tempfile::tempfile().unwrap();
    assert!(copy_fixed_download("https://invalid.example/unknown", &mut output, 100).is_none());
    FIXTURE
        .scope(scope, async {
            assert_eq!(
                copy_fixed_download("https://invalid.example/unknown", &mut output, 100),
                Some(Err(Error::InvalidRelease))
            );
            assert_eq!(output.metadata().unwrap().len(), 0);
        })
        .await;
}
