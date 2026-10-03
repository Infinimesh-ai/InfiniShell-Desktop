//! 仅承接官方 Claude Latest 287 → Stable 285；两个 cask 与公共入口属于同一事务。
//! 恢复只读取已持久化身份，不依赖实时网络，不执行 brew/Ruby 或安装脚本。

use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt as _;

use super::super::super::sources;
use super::super::package_tree::{Directory, Snapshot};
use super::super::{
    CLIAgent, Error, MAX_CONFIG, MAX_OUTPUT, UPDATE_TIMEOUT, UpdatePlan, VerificationProgress,
    brew, managed_process, read_limited,
};
use super::super::{
    Channel, ClaudeUpdateScope, ConfigBackup, ConfigKind, Source, claude_downgrade,
};
use super::{
    Journal, Link, Phase, Probe, bytes_file, download, exchange_links, journal_path, link,
    lock_cask, probe_binding, probe_exited,
};
use futures::AsyncReadExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write as _;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;

const OLD_TOKEN: &str = "claude-code@latest";
const NEW_TOKEN: &str = "claude-code";
const OLD_VERSION: &str = "2.1.287";
const NEW_VERSION: &str = "2.1.285";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum MigrationPhase {
    Preparing,
    Prepared,
    PublishingTarget,
    PublishingLink,
    RetiringSource,
    PublishingConfig,
    Committed,
    RollingBack,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Migration {
    schema: u32,
    transaction: Journal,
    phase: MigrationPhase,
    downgrade: claude_downgrade::Intent,
    config: ConfigBackup,
    policy: ClaudeUpdateScope,
}

pub(in super::super) async fn metadata(old: &Value) -> Result<(Value, Value), Error> {
    if old["token"] != OLD_TOKEN || old["tap"] != "homebrew/cask" {
        return Err(Error::InvalidRelease);
    }
    #[cfg(target_os = "linux")]
    {
        sources::brew_claude_linux::migration_metadata().await
    }
    #[cfg(target_os = "macos")]
    {
        let target = brew::metadata(NEW_TOKEN).await?;
        verify_metadata(old, &target)?;
        Ok((old.clone(), target))
    }
}

fn native_image(version: &str) -> Result<(u64, [u8; 32]), Error> {
    #[cfg(target_os = "linux")]
    {
        sources::brew_claude_linux::native(version)
    }
    #[cfg(target_os = "macos")]
    {
        sources::claude_current_release::native(version, "darwin-arm64")
    }
}

fn target_image(target: &Value) -> Result<(String, [u8; 32]), Error> {
    #[cfg(target_os = "linux")]
    {
        let url = format!(
            "https://downloads.claude.ai/claude-code-releases/{NEW_VERSION}/linux-x64/claude"
        );
        let digest = native_image(NEW_VERSION)?.1;
        if target["token"] != NEW_TOKEN
            || target["version"] != NEW_VERSION
            || target["url"] != url
            || target["sha256"] != hex::encode(digest)
        {
            return Err(Error::InvalidRelease);
        }
        Ok((url, digest))
    }
    #[cfg(target_os = "macos")]
    {
        let (_, url, digest) = brew::release(
            NEW_TOKEN,
            NEW_VERSION,
            &serde_json::to_vec(target).map_err(|_| Error::InvalidRelease)?,
        )?;
        Ok((url, digest))
    }
}

#[cfg(target_os = "macos")]
fn verify_metadata(old: &Value, target: &Value) -> Result<(), Error> {
    for (value, token, version, other, checksum) in [
        (
            old,
            OLD_TOKEN,
            OLD_VERSION,
            NEW_TOKEN,
            "30834127bab1cf3bda09f66ca9243b10b8a864ef3b3e8cdd7af68a68f47a4d5b",
        ),
        (
            target,
            NEW_TOKEN,
            NEW_VERSION,
            OLD_TOKEN,
            "afd076e38f356a677ed9abfafca8305299f91454b36b30e50e635a4eb546dd12",
        ),
    ] {
        brew::release(
            token,
            version,
            &serde_json::to_vec(value).map_err(|_| Error::InvalidRelease)?,
        )?;
        if !valid_tap_head(&value["tap_git_head"])
            || value["ruby_source_path"] != format!("Casks/c/{token}.rb")
            || value["ruby_source_checksum"]["sha256"] != checksum
            || value["conflicts_with"] != json!({"cask": [other]})
            || value["url_specs"] != json!({})
        {
            return Err(Error::InvalidRelease);
        }
    }
    Ok(())
}

fn valid_tap_head(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|head| head.len() == 40 && head.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn path(root: &Path) -> PathBuf {
    root.join("claude-homebrew-migration.json")
}

fn save_migration(path: &Path, journal: &Migration) -> Result<(), Error> {
    sources::plain_ancestors(path)?;
    let parent = path.parent().ok_or(Error::PersistenceFailed)?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|_| Error::PersistenceFailed)?;
    temporary
        .write_all(&serde_json::to_vec(journal).map_err(|_| Error::PersistenceFailed)?)
        .map_err(|_| Error::PersistenceFailed)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    temporary
        .persist(path)
        .map_err(|_| Error::PersistenceFailed)?;
    sources::sync_config_directory(parent)
}

impl Migration {
    fn backup_name(&self) -> OsString {
        format!(".infinishell-brew-old-{}", self.transaction.id).into()
    }
    fn target(&self) -> PathBuf {
        self.transaction
            .parent()
            .join(NEW_TOKEN)
            .join(NEW_VERSION)
            .join("claude")
    }
    fn original_target(&self) -> PathBuf {
        self.transaction
            .parent()
            .join(OLD_TOKEN)
            .join(OLD_VERSION)
            .join("claude")
    }
    fn candidate(&self) -> PathBuf {
        self.transaction
            .parent()
            .join(self.transaction.stage_name())
            .join(NEW_VERSION)
            .join("claude")
    }
    fn verify_config(&self, published: bool) -> Result<(), Error> {
        if sources::read_optional_config(&self.config.path)?.as_ref()
            != self.config.publication_bytes(published)
            || self.config.before_mode.is_some_and(|mode| {
                fs::metadata(&self.config.path)
                    .map_or(true, |metadata| metadata.mode() & 0o777 != mode)
            })
        {
            return Err(Error::SourceChanged);
        }
        sources::verify_claude_originals(&self.policy)
    }
    fn validate(&self, entry: &Path) -> Result<(), Error> {
        let tx = &self.transaction;
        claude_downgrade::validate_homebrew(
            Some(self.downgrade),
            &tx.old_version,
            &tx.target_version,
            &Some(self.config.clone()),
        )?;
        if self.schema != 1
            || tx.schema != 1
            || tx.id.is_nil()
            || tx.agent != "claude"
            || tx.token != OLD_TOKEN
            || tx.entry != entry
            || tx.entry != tx.prefix.join("bin/claude")
            || brew::prefix_from_entry(CLIAgent::Claude, entry)? != tx.prefix
            || !tx.completions.is_empty()
            || !tx.aliases.is_empty()
            || !tx.extra_probes.is_empty()
            || !tx.entry_probes.is_empty()
            || tx.original_link.target != self.original_target()
            || tx
                .prepared_link
                .as_ref()
                .is_some_and(|link| link.target != self.target())
            || !matches!(self.config.kind, ConfigKind::Claude)
            || self.config.path != self.policy.settings_path
            || self.config.desired.as_ref().map(|desired| &desired.bytes)
                != Some(&sources::selected_channel_config(
                    ConfigKind::Claude,
                    self.config.before.as_deref(),
                    Some(Channel::Stable),
                )?)
        {
            return Err(Error::RecoveryRequired);
        }
        verify_tree_contract(&tx.original, OLD_VERSION, OLD_TOKEN)?;
        if let Some(prepared) = &tx.prepared {
            verify_tree_contract(prepared, NEW_VERSION, NEW_TOKEN)?;
        }
        sources::validate_claude_scope(&self.policy)?;
        sources::claude_metadata_path(&self.config)?;
        sources::config_restore_stage(&self.config)?;
        if let Some(probe) = &tx.probe {
            let digest = native_image(NEW_VERSION)?.1;
            if probe.generation.is_nil()
                || probe.program != self.candidate()
                || probe.arguments != [OsString::from("--version")]
                || probe.digest != probe_digest(tx.id, &probe.program, digest)?
                || probe
                    .version
                    .as_deref()
                    .is_some_and(|version| version != NEW_VERSION)
                || probe.completed && probe.version.as_deref() != Some(NEW_VERSION)
            {
                return Err(Error::RecoveryRequired);
            }
        }
        tx.verify_external()?;
        Ok(())
    }
}

fn verify_tree_contract(snapshot: &Snapshot, version: &str, token: &str) -> Result<(), Error> {
    let files = snapshot.file_manifest();
    let native = PathBuf::from(version).join("claude");
    let image = native_image(version)?;
    snapshot.verify_file(&native, image.0, image.1)?;
    if files.len() != 4
        || !files.contains_key(Path::new(".metadata/INSTALL_RECEIPT.json"))
        || !files.contains_key(Path::new(".metadata/config.json"))
    {
        return Err(Error::InvalidRelease);
    }
    let cask_root = PathBuf::from(".metadata").join(version);
    let casks = files
        .iter()
        .filter(|(path, _)| path.starts_with(&cask_root))
        .collect::<Vec<_>>();
    if casks.len() != 1 {
        return Err(Error::InvalidRelease);
    }
    let (cask, (length, digest)) = casks[0];
    let relative = cask
        .strip_prefix(&cask_root)
        .map_err(|_| Error::InvalidRelease)?;
    let parts = relative.iter().collect::<Vec<_>>();
    if parts.len() != 3
        || parts[1] != "Casks"
        || parts[2] != format!("{token}.json").as_str()
        || parts[0].to_str().is_none_or(|value| {
            value.is_empty()
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        })
        || ![
            (2, Sha256::digest(b"{}").into()),
            (3, Sha256::digest(b"{}\n").into()),
        ]
        .contains(&(*length, *digest))
    {
        return Err(Error::InvalidRelease);
    }
    snapshot.verify_release_files(&files)
}

/// 固定发行映像之外，只允许真实 Homebrew 无脚本登记的三份小文件。
fn verify_old_tree(root: &Path, snapshot: &Snapshot) -> Result<(), Error> {
    let native = PathBuf::from(OLD_VERSION).join("claude");
    let (length, digest) = native_image(OLD_VERSION)?;
    managed_process::ExpectedFileIdentity::capture_release_image(
        &root.join(&native),
        length,
        digest,
    )
    .map_err(|_| Error::InvalidRelease)?;
    let timestamps = root.join(".metadata").join(OLD_VERSION);
    let entries = fs::read_dir(&timestamps)
        .map_err(|_| Error::UnsupportedSource)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::UnsupportedSource)?;
    if entries.len() != 1 {
        return Err(Error::UnsupportedSource);
    }
    let timestamp = entries[0].file_name();
    if timestamp.to_str().is_none_or(|name| {
        name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
    }) {
        return Err(Error::UnsupportedSource);
    }
    let cask = PathBuf::from(".metadata")
        .join(OLD_VERSION)
        .join(timestamp)
        .join("Casks")
        .join(format!("{OLD_TOKEN}.json"));
    let mut files = BTreeMap::from([(native, (length, digest))]);
    for relative in [
        PathBuf::from(".metadata/INSTALL_RECEIPT.json"),
        PathBuf::from(".metadata/config.json"),
        cask.clone(),
    ] {
        let bytes = read_limited(&root.join(&relative), MAX_CONFIG)?;
        serde_json::from_slice::<Value>(&bytes).map_err(|_| Error::InvalidRelease)?;
        files.insert(
            relative,
            (bytes.len() as u64, Sha256::digest(&bytes).into()),
        );
    }
    if serde_json::from_slice::<Value>(&read_limited(&root.join(cask), MAX_CONFIG)?)
        .map_err(|_| Error::InvalidRelease)?
        != json!({})
    {
        return Err(Error::UnsupportedSource);
    }
    snapshot.verify_release_files(&files)
}

pub(super) async fn execute(
    plan: &UpdatePlan,
    root: &Path,
    progress: Option<VerificationProgress>,
) -> Result<String, Error> {
    if plan.agent != CLIAgent::Claude
        || plan.installation.source != Source::Homebrew
        || plan.installation.channel != Channel::Stable
        || plan.installation.source_target.as_deref() != Some(NEW_VERSION)
    {
        return Err(Error::ChannelMismatch);
    }
    claude_downgrade::validate_homebrew(
        plan.downgrade,
        &plan.installed_version,
        &plan.target_version,
        &plan.config,
    )?;
    claude_downgrade::revalidate(plan.downgrade).await?;
    let old = brew::metadata(OLD_TOKEN).await?;
    let (_, target) = metadata(&old).await?;
    let prefix = brew::prefix_from_entry(CLIAgent::Claude, &plan.installation.entry)?;
    // Homebrew 的两个原生 cask 锁保持固定顺序；不能与并发 brew 操作交错。
    let stable_lock = lock_cask(&prefix, NEW_TOKEN)?;
    let latest_lock = lock_cask(&prefix, OLD_TOKEN)?;
    sources::verify_installation_identity(&plan.installation)?;
    let registered = brew::registration(
        CLIAgent::Claude,
        &plan.installation,
        &prefix,
        OLD_TOKEN,
        OLD_VERSION,
    )?;
    if registered.entry_relative != Path::new("claude") {
        return Err(Error::UnsupportedSource);
    }
    let prefix_directory = Directory::open(&prefix)?;
    let prefix_identity = prefix_directory.identity()?;
    let (parent, parent_identity) = prefix_directory.homebrew_caskroom()?;
    if parent.has_child(OsStr::new(NEW_TOKEN))? {
        return Err(Error::SourceChanged);
    }
    let original = parent.child(OsStr::new(OLD_TOKEN))?.snapshot()?;
    verify_old_tree(&registered.root, &original)?;
    let mut receipt: Value =
        serde_json::from_slice(&read_limited(&registered.receipt, MAX_CONFIG)?)
            .map_err(|_| Error::UnsupportedSource)?;
    if receipt["homebrew_version"] != "7.0.4"
        || receipt["runtime_dependencies"] != json!({})
        || !valid_tap_head(&receipt["source"]["tap_git_head"])
    {
        return Err(Error::UnsupportedSource);
    }
    let config = plan.config.clone().ok_or(Error::UnsupportedSource)?;
    let id = Uuid::new_v4();
    let policy = sources::snapshot_claude_scope(
        &config,
        id,
        sources::claude_metadata_path(&config)?,
        NEW_VERSION,
    )?;
    let (url, digest) = target_image(&target)?;
    let (length, fixed_digest) = native_image(NEW_VERSION)?;
    if digest != fixed_digest {
        return Err(Error::InvalidRelease);
    }
    let mut image = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    download(&url, image.as_file_mut(), length).await?;
    managed_process::ExpectedFileIdentity::capture_release_image(image.path(), length, digest)
        .map_err(|_| Error::InvalidRelease)?;
    let path = path(root);
    if path.try_exists().map_err(|_| Error::RecoveryRequired)?
        || journal_path(root, CLIAgent::Claude)
            .try_exists()
            .map_err(|_| Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    let mut journal = Migration {
        schema: 1,
        phase: MigrationPhase::Preparing,
        downgrade: claude_downgrade::Intent::ClaudeHomebrewStable21287To21285,
        config,
        policy,
        transaction: Journal {
            schema: 1,
            id,
            agent: "claude".into(),
            prefix: prefix.clone(),
            prefix_identity,
            parent_identity,
            bin_identity: Directory::open(&prefix.join("bin"))?.identity()?,
            token: OLD_TOKEN.into(),
            entry: plan.installation.entry.clone(),
            manager: plan
                .installation
                .manager
                .clone()
                .ok_or(Error::UnsupportedSource)?,
            old_version: OLD_VERSION.into(),
            target_version: NEW_VERSION.into(),
            intent: plan.intent.clone(),
            original,
            prepared: None,
            original_link: link(&plan.installation.entry)?,
            prepared_link: None,
            phase: Phase::Preparing,
            probe: None,
            extra_probes: Vec::new(),
            completions: Vec::new(),
            aliases: Vec::new(),
            entry_probes: Vec::new(),
        },
    };
    journal.validate(&plan.installation.entry)?;
    journal.verify_config(false)?;
    save_migration(&path, &journal)?;
    let result = async {
        let tx = &journal.transaction;
        let stage = parent.create(&tx.stage_name())?;
        let native = PathBuf::from(NEW_VERSION).join("claude");
        stage.write_new(
            &native,
            &mut File::open(image.path()).map_err(|_| Error::PersistenceFailed)?,
            length,
            digest,
        )?;
        let mut files = BTreeMap::from([(native.clone(), (length, digest))]);
        let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S.%3f").to_string();
        let cask = PathBuf::from(".metadata")
            .join(NEW_VERSION)
            .join(timestamp)
            .join("Casks")
            .join(format!("{NEW_TOKEN}.json"));
        receipt["source"]["version"] = json!(NEW_VERSION);
        receipt["source"]["tap_git_head"] = target["tap_git_head"].clone();
        receipt["time"] = json!(chrono::Utc::now().timestamp());
        // 真实 Homebrew tab 的卸载记录使用默认 binary 目标，不存 API 的前缀占位符。
        receipt["uninstall_artifacts"] = target["artifacts"].clone();
        for artifact in receipt["uninstall_artifacts"]
            .as_array_mut()
            .ok_or(Error::InvalidRelease)?
        {
            if artifact.get("binary").is_some() {
                artifact
                    .as_object_mut()
                    .ok_or(Error::InvalidRelease)?
                    .remove("target");
            }
        }
        let source_path = receipt["source"]["path"]
            .as_str()
            .ok_or(Error::UnsupportedSource)?;
        let source_path = Path::new(source_path);
        if !source_path.is_absolute()
            || source_path.file_name() != Some(OsStr::new("claude-code@latest.rb"))
        {
            return Err(Error::UnsupportedSource);
        }
        receipt["source"]["path"] = json!(source_path.with_file_name("claude-code.rb"));
        bytes_file(&stage, &cask, b"{}\n", &mut files)?;
        bytes_file(
            &stage,
            Path::new(".metadata/INSTALL_RECEIPT.json"),
            &serde_json::to_vec_pretty(&receipt).map_err(|_| Error::InvalidRelease)?,
            &mut files,
        )?;
        bytes_file(
            &stage,
            Path::new(".metadata/config.json"),
            &read_limited(&registered.root.join(".metadata/config.json"), MAX_CONFIG)?,
            &mut files,
        )?;
        let executables = files
            .keys()
            .map(|path| (path.clone(), *path == native))
            .collect();
        stage.apply_permissions(&tx.original, &executables)?;
        let prepared = stage.snapshot()?;
        prepared.verify_release_files(&files)?;
        symlink(journal.target(), tx.link_stage()).map_err(|_| Error::PersistenceFailed)?;
        sources::sync_config_directory(&prefix.join("bin"))?;
        journal.transaction.prepared_link = Some(link(&tx.link_stage())?);
        journal.transaction.prepared = Some(prepared);
        journal.transaction.phase = Phase::Prepared;
        journal.phase = MigrationPhase::Prepared;
        save_migration(&path, &journal)?;
        probe_candidate(root, &path, &mut journal, length, digest).await?;
        journal.verify_config(false)?;
        let parent = journal.transaction.verify_external()?;
        verify_layout(&journal, &parent, false)?;
        journal.phase = MigrationPhase::PublishingTarget;
        save_migration(&path, &journal)?;
        parent.rename_noreplace(&journal.transaction.stage_name(), OsStr::new(NEW_TOKEN))?;
        journal.phase = MigrationPhase::PublishingLink;
        save_migration(&path, &journal)?;
        verify_layout(&journal, &parent, false)?;
        exchange_links(&journal.transaction)?;
        journal.phase = MigrationPhase::RetiringSource;
        save_migration(&path, &journal)?;
        verify_layout(&journal, &parent, false)?;
        parent.rename_noreplace(OsStr::new(OLD_TOKEN), &journal.backup_name())?;
        verify_layout(&journal, &parent, false)?;
        if let Some(progress) = progress {
            progress.enter().await;
        }
        journal.config.after = journal.config.before.clone();
        sources::plan_config_publish(&mut journal.config, true)?;
        journal.phase = MigrationPhase::PublishingConfig;
        save_migration(&path, &journal)?;
        sources::publish_config(&journal.config, true)?;
        journal.verify_config(true)?;
        journal.phase = MigrationPhase::Committed;
        save_migration(&path, &journal)?;
        finish(root, &path, &journal)?;
        Ok(NEW_VERSION.to_owned())
    }
    .await;
    drop(latest_lock);
    drop(stable_lock);
    if result.is_err()
        && recover_if_present(CLIAgent::Claude, &plan.installation.entry, root)?
            == Some(Some(NEW_VERSION.to_owned()))
    {
        return Ok(NEW_VERSION.to_owned());
    }
    result
}

fn probe_digest(id: Uuid, program: &Path, digest: [u8; 32]) -> Result<String, Error> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(id, program, digest, ["--version"]))
                .map_err(|_| Error::PersistenceFailed)?
        )
    ))
}

async fn probe_candidate(
    root: &Path,
    path: &Path,
    journal: &mut Migration,
    length: u64,
    digest: [u8; 32],
) -> Result<(), Error> {
    let program = journal.candidate();
    let expected =
        managed_process::ExpectedFileIdentity::capture_release_image(&program, length, digest)
            .map_err(|_| Error::SourceChanged)?;
    let generation = Uuid::new_v4();
    let binding_digest = probe_digest(journal.transaction.id, &program, digest)?;
    let binding = probe_binding(CLIAgent::Claude, binding_digest.clone())?;
    journal.transaction.probe = Some(Probe {
        generation,
        program: program.clone(),
        digest: binding_digest,
        arguments: vec!["--version".into()],
        version: None,
        completed: false,
        output_sha256: None,
    });
    save_migration(path, journal)?;
    let mut child =
        managed_process::spawn_bound_version_probe(root, generation, &program, expected, &binding)
            .await
            .map_err(|_| Error::RecoveryRequired)?;
    let mut stdout = child.stdout.take().ok_or(Error::RecoveryRequired)?;
    let mut bytes = Vec::new();
    let output = (&mut stdout)
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .with_timeout(UPDATE_TIMEOUT)
        .await;
    drop(stdout);
    let receipt = if output.is_ok() {
        child.finish_after_stdin_close().await
    } else {
        child.finish().await
    }
    .map_err(|_| Error::RecoveryRequired)?;
    if !receipt.cleanup_confirmed {
        return Err(Error::RecoveryRequired);
    }
    match output {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => return Err(Error::ProbeFailed),
        Err(_) => return Err(Error::TimedOut),
    }
    if receipt.exit_code != Some(0)
        || bytes.len() as u64 > MAX_OUTPUT
        || sources::parse_cli_agent_version(
            CLIAgent::Claude,
            std::str::from_utf8(&bytes).map_err(|_| Error::ProbeFailed)?,
        )
        .as_deref()
            != Some(NEW_VERSION)
    {
        return Err(Error::ProbeFailed);
    }
    let probe = journal
        .transaction
        .probe
        .as_mut()
        .ok_or(Error::RecoveryRequired)?;
    probe.version = Some(NEW_VERSION.into());
    probe.completed = true;
    probe.output_sha256 = Some(format!("{:x}", Sha256::digest(&bytes)));
    save_migration(path, journal)
}

/// 恢复先核整个布局；不能改完公共入口后才发现另一棵树已被外部修改。
fn verify_layout(journal: &Migration, parent: &Directory, cleaning: bool) -> Result<(), Error> {
    let tx = &journal.transaction;
    let source = parent.has_child(OsStr::new(OLD_TOKEN))?;
    let backup = parent.has_child(&journal.backup_name())?;
    let target = parent.has_child(OsStr::new(NEW_TOKEN))?;
    let stage = parent.has_child(&tx.stage_name())?;
    if source && backup
        || target && stage
        || !source && !backup && journal.phase != MigrationPhase::Committed
        || tx.prepared.is_some()
            && !target
            && !stage
            && journal.phase != MigrationPhase::RollingBack
    {
        return Err(Error::RecoveryRequired);
    }
    if source && parent.child(OsStr::new(OLD_TOKEN))?.snapshot()? != tx.original {
        return Err(Error::SourceChanged);
    }
    if backup {
        let actual = parent.child(&journal.backup_name())?.snapshot()?;
        if journal.phase == MigrationPhase::Committed {
            actual.verify_remaining(&tx.original)?;
        } else if actual != tx.original {
            return Err(Error::SourceChanged);
        }
    }
    let stage_name = tx.stage_name();
    for name in [
        target.then_some(OsStr::new(NEW_TOKEN)),
        stage.then_some(stage_name.as_os_str()),
    ]
    .into_iter()
    .flatten()
    {
        let actual = parent.child(name)?.snapshot()?;
        let expected = tx.prepared.as_ref().ok_or(Error::RecoveryRequired)?;
        if journal.phase == MigrationPhase::RollingBack && source && !backup && !target {
            actual.verify_remaining(expected)?;
        } else if actual != *expected {
            return Err(Error::SourceChanged);
        }
    }
    let public = link(&tx.entry)?;
    if public == tx.original_link {
        if !source {
            return Err(Error::RecoveryRequired);
        }
        if let Some(prepared) = &tx.prepared_link {
            match fs::symlink_metadata(tx.link_stage()) {
                Ok(_) => {
                    if link(&tx.link_stage())? != *prepared {
                        return Err(Error::SourceChanged);
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && journal.phase == MigrationPhase::RollingBack
                        && !stage
                        && !target => {}
                Err(_) => return Err(Error::RecoveryRequired),
            }
        }
    } else if Some(&public) == tx.prepared_link.as_ref() {
        if !target {
            return Err(Error::SourceChanged);
        }
        match fs::symlink_metadata(tx.link_stage()) {
            Ok(_) => {
                if link(&tx.link_stage())? != tx.original_link {
                    return Err(Error::SourceChanged);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && cleaning && !backup => {}
            Err(_) => return Err(Error::RecoveryRequired),
        }
    } else {
        return Err(Error::SourceChanged);
    }
    if cleaning && (source || !target || stage || Some(&public) != tx.prepared_link.as_ref()) {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

pub(super) fn recover_if_present(
    agent: CLIAgent,
    entry: &Path,
    root: &Path,
) -> Result<Option<Option<String>>, Error> {
    if agent != CLIAgent::Claude
        || !path(root)
            .try_exists()
            .map_err(|_| Error::RecoveryRequired)?
    {
        return Ok(None);
    }
    if journal_path(root, agent)
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    let path = path(root);
    let mut journal: Migration = serde_json::from_slice(&read_limited(&path, 12 * MAX_CONFIG)?)
        .map_err(|_| Error::RecoveryRequired)?;
    journal.validate(entry)?;
    recover_validated(root, &path, &mut journal).map(Some)
}

fn recover_validated(
    root: &Path,
    path: &Path,
    journal: &mut Migration,
) -> Result<Option<String>, Error> {
    let stable_lock = lock_cask(&journal.transaction.prefix, NEW_TOKEN)?;
    let latest_lock = lock_cask(&journal.transaction.prefix, OLD_TOKEN)?;
    probe_exited(root, &journal.transaction)?;
    sources::verify_claude_originals(&journal.policy)?;
    if journal.phase == MigrationPhase::Preparing
        && journal.transaction.prepared.is_none()
        && journal.transaction.probe.is_none()
    {
        recover_preparing(root, path, journal)?;
        return Ok(None);
    }
    if journal.phase == MigrationPhase::PublishingConfig {
        let parent = journal.transaction.verify_external()?;
        verify_layout(&journal, &parent, true)?;
        if journal
            .transaction
            .probe
            .as_ref()
            .is_none_or(|probe| !probe.completed)
        {
            return Err(Error::RecoveryRequired);
        }
        sources::publish_config(&journal.config, true)?;
        journal.verify_config(true)?;
        journal.phase = MigrationPhase::Committed;
        save_migration(&path, &journal)?;
    }
    let result = if journal.phase == MigrationPhase::Committed {
        finish(root, &path, &journal)?;
        Some(NEW_VERSION.to_owned())
    } else {
        rollback(root, &path, journal)?;
        None
    };
    drop(latest_lock);
    drop(stable_lock);
    Ok(result)
}

fn recover_preparing(root: &Path, path: &Path, journal: &Migration) -> Result<(), Error> {
    let tx = &journal.transaction;
    journal.verify_config(false)?;
    let parent = tx.verify_external()?;
    if parent.has_child(OsStr::new(NEW_TOKEN))?
        || parent.has_child(&journal.backup_name())?
        || parent.child(OsStr::new(OLD_TOKEN))?.snapshot()? != tx.original
        || link(&tx.entry)? != tx.original_link
        || tx.prepared_link.is_some()
    {
        return Err(Error::RecoveryRequired);
    }
    // 与 npm 准备中断相同：未登记 probe 即未派生候选；未知未发布树只保留，不认领删除。
    // 退下活动账本让原安装可用，失败意图阻止后台反复提交；原件保留给人工清理。
    let retained = root.join(format!("claude-homebrew-migration-{}.retained.json", tx.id));
    let bytes = serde_json::to_vec(journal).map_err(|_| Error::PersistenceFailed)?;
    if retained.try_exists().map_err(|_| Error::RecoveryRequired)? {
        if read_limited(&retained, 12 * MAX_CONFIG)? != bytes {
            return Err(Error::RecoveryRequired);
        }
    } else {
        let mut temporary = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
        temporary
            .write_all(&bytes)
            .map_err(|_| Error::PersistenceFailed)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| Error::PersistenceFailed)?;
        temporary
            .persist_noclobber(&retained)
            .map_err(|_| Error::RecoveryRequired)?;
        sources::sync_config_directory(root)?;
    }
    sources::save_failure_with_intent(
        root,
        CLIAgent::Claude,
        &tx.entry,
        NEW_VERSION,
        Some(tx.intent.clone()),
    )?;
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
    sources::sync_config_directory(root)
}

fn rollback(root: &Path, path: &Path, journal: &mut Migration) -> Result<(), Error> {
    journal.verify_config(false)?;
    let parent = journal.transaction.verify_external()?;
    verify_layout(journal, &parent, false)?;
    journal.phase = MigrationPhase::RollingBack;
    save_migration(path, journal)?;
    let tx = &journal.transaction;
    if parent.has_child(&journal.backup_name())? {
        parent.rename_noreplace(&journal.backup_name(), OsStr::new(OLD_TOKEN))?;
    }
    if link(&tx.entry)? != tx.original_link {
        exchange_links(tx)?;
    }
    if parent.has_child(OsStr::new(NEW_TOKEN))? {
        parent.rename_noreplace(OsStr::new(NEW_TOKEN), &tx.stage_name())?;
    }
    if parent.has_child(&tx.stage_name())? {
        parent.remove_matching(
            &tx.stage_name(),
            tx.prepared.as_ref().ok_or(Error::RecoveryRequired)?,
        )?;
    }
    if let Some(prepared) = &tx.prepared_link {
        remove_link_if_matching(&tx.link_stage(), prepared)?;
    }
    sources::save_failure_with_intent(
        root,
        CLIAgent::Claude,
        &tx.entry,
        NEW_VERSION,
        Some(tx.intent.clone()),
    )?;
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
    sources::sync_config_directory(root)
}

fn finish(root: &Path, path: &Path, journal: &Migration) -> Result<(), Error> {
    let tx = &journal.transaction;
    journal.verify_config(true)?;
    if tx.probe.as_ref().is_none_or(|probe| !probe.completed) {
        return Err(Error::RecoveryRequired);
    }
    probe_exited(root, tx)?;
    finish_files(root, path, journal)
}

fn finish_files(root: &Path, path: &Path, journal: &Migration) -> Result<(), Error> {
    journal.verify_config(true)?;
    let tx = &journal.transaction;
    let parent = tx.verify_external()?;
    verify_layout(journal, &parent, true)?;
    if parent.has_child(&journal.backup_name())? {
        parent.remove_matching(&journal.backup_name(), &tx.original)?;
    }
    remove_link_if_matching(&tx.link_stage(), &tx.original_link)?;
    sources::cleanup_config_restore(&journal.config)?;
    match fs::remove_file(sources::failure_path(root, CLIAgent::Claude)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
    sources::sync_config_directory(root)
}

fn remove_link_if_matching(path: &Path, expected: &Link) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            if link(path)? != *expected {
                return Err(Error::SourceChanged);
            }
            fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
            sources::sync_config_directory(path.parent().ok_or(Error::RecoveryRequired)?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::RecoveryRequired),
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "sources_brew_claude_migration_tests.rs"]
mod tests;
