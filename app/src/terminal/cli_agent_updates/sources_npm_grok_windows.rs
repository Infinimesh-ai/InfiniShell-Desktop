//! Windows x64 Grok npm 的整包候选、双公共入口探针和可恢复发布。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::windows::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};

use futures::AsyncReadExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

use super::npm_grok_windows_contract as contract;
use super::npm_grok_windows_tree as tree;
use super::{
    ArtifactRole, CLIAgent, ConfigBackup, Error, Installation, Invocation, MAX_CONFIG, MAX_OUTPUT,
    Source, Stamp, UPDATE_TIMEOUT, UpdatePlan, VerificationProgress, managed_process, npm,
    read_limited, stamp,
};

/// 已安装入口查询保留官方 PowerShell shim；候选验收仍必须经过下方独立 AppContainer。
pub(super) fn version_invocation(entry: &Path) -> Result<Option<Invocation>, Error> {
    if !entry
        .extension()
        .is_some_and(|suffix| suffix.as_encoded_bytes().eq_ignore_ascii_case(b"ps1"))
    {
        return Ok(None);
    }
    let before = stamp(entry)?;
    if before
        .canonical
        .file_name()
        .is_none_or(|name| name != "grok.ps1")
        || read_limited(entry, 16384)? != contract::powershell_shim().as_bytes()
        || stamp(entry)? != before
    {
        return Err(Error::UnsupportedSource);
    }
    let mut buffer = [0u16; 32768];
    let count = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if count == 0 || count >= buffer.len() {
        return Err(Error::UnsupportedPlatform);
    }
    let program = PathBuf::from(OsString::from_wide(&buffer[..count]))
        .join("WindowsPowerShell/v1.0/powershell.exe")
        .canonicalize()
        .map_err(|_| Error::UnsupportedPlatform)?;
    Ok(Some(Invocation::new(
        &program,
        [
            OsString::from("-NoLogo"),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            entry.as_os_str().to_owned(),
            "--version".into(),
        ],
    )))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    entry: PathBuf,
    prefix: PathBuf,
    package: PathBuf,
    prefix_identity: tree::Identity,
    parent_identity: tree::Identity,
    node: Stamp,
    dependencies: Vec<Stamp>,
    shims: BTreeMap<String, Stamp>,
    home: PathBuf,
    home_identity: tree::Identity,
    bin_identity: tree::Identity,
}

impl Owner {
    fn capture(
        installation: &Installation,
        old: &str,
        config: &ConfigBackup,
    ) -> Result<Self, Error> {
        super::verify_installation_identity(installation)?;
        let prefix = installation
            .invocation
            .as_ref()
            .and_then(|invocation| invocation.artifacts.get(&ArtifactRole::InstallRoot))
            .ok_or(Error::UnsupportedSource)?;
        let registration =
            npm::registered_installation(CLIAgent::Grok, installation, old, prefix, true)?
                .ok_or(Error::UnsupportedSource)?;
        if ![
            registration.prefix.join("grok.cmd"),
            registration.prefix.join("grok.ps1"),
            config
                .path
                .parent()
                .ok_or(Error::UnsupportedSource)?
                .join("bin/grok.exe"),
        ]
        .contains(&installation.stamp.canonical)
        {
            return Err(Error::UnsupportedSource);
        }
        let manager = installation
            .manager
            .clone()
            .ok_or(Error::UnsupportedSource)?;
        let helper = installation
            .helper
            .clone()
            .ok_or(Error::UnsupportedSource)?;
        if !npm::registered_manager(&helper.canonical)? {
            return Err(Error::SourceChanged);
        }
        let npm_manifest = helper
            .canonical
            .parent()
            .and_then(Path::parent)
            .ok_or(Error::SourceChanged)?
            .join("package.json");
        let mut shims = BTreeMap::new();
        for (name, expected) in contract::shims() {
            let path = registration.prefix.join(name);
            if read_limited(&path, 16384)? != expected.as_bytes() {
                return Err(Error::UnsupportedSource);
            }
            shims.insert(name.to_owned(), stamp(&path)?);
        }
        let local_node = registration.prefix.join("node.exe");
        let node = if local_node.try_exists().map_err(|_| Error::SourceChanged)? {
            stamp(&local_node)?
        } else {
            manager.clone()
        };
        if node
            .canonical
            .file_name()
            .is_none_or(|name| !name.as_encoded_bytes().eq_ignore_ascii_case(b"node.exe"))
        {
            return Err(Error::UnsupportedSource);
        }
        let home = config
            .path
            .parent()
            .ok_or(Error::UnsupportedSource)?
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?;
        if config.path != home.join("config.toml")
            || !matches!(config.kind, super::ConfigKind::Grok)
        {
            return Err(Error::UnsupportedSource);
        }
        let bytes = config.before.as_ref().ok_or(Error::UnsupportedSource)?;
        let value: toml::Value = std::str::from_utf8(bytes)
            .map_err(|_| Error::UnsupportedSource)?
            .parse()
            .map_err(|_| Error::UnsupportedSource)?;
        if value
            .get("cli")
            .and_then(|cli| cli.get("installer"))
            .and_then(toml::Value::as_str)
            != Some("npm")
        {
            return Err(Error::UnsupportedSource);
        }
        let owner = Self {
            entry: installation.entry.clone(),
            prefix: registration.prefix.clone(),
            prefix_identity: tree::path_identity(&registration.prefix)?,
            parent_identity: tree::path_identity(
                registration
                    .package_root
                    .parent()
                    .ok_or(Error::SourceChanged)?,
            )?,
            package: registration.package_root,
            node,
            dependencies: vec![manager, helper, stamp(&npm_manifest)?],
            shims,
            home_identity: tree::path_identity(&home)?,
            bin_identity: tree::path_identity(&home.join("bin"))?,
            home,
        };
        owner.verify()?;
        Ok(owner)
    }

    fn verify(&self) -> Result<(), Error> {
        if tree::path_identity(&self.home)? != self.home_identity
            || tree::path_identity(&self.home.join("bin"))? != self.bin_identity
            || self.dependencies.len() != 3
            || self.package != self.prefix.join("node_modules/@xai-official/grok")
            || tree::path_identity(&self.prefix)? != self.prefix_identity
            || tree::path_identity(self.package.parent().ok_or(Error::SourceChanged)?)?
                != self.parent_identity
            || ![
                self.prefix.join("grok.cmd"),
                self.prefix.join("grok.ps1"),
                self.home.join("bin/grok.exe"),
            ]
            .contains(
                &self
                    .entry
                    .canonicalize()
                    .map_err(|_| Error::SourceChanged)?,
            )
            || stamp(&self.node.canonical)? != self.node
        {
            return Err(Error::SourceChanged);
        }
        for dependency in &self.dependencies {
            if stamp(&dependency.canonical)? != *dependency {
                return Err(Error::SourceChanged);
            }
        }
        if self.shims.len() != 3 {
            return Err(Error::SourceChanged);
        }
        for (name, contents) in contract::shims() {
            let path = self.prefix.join(name);
            if self.shims.get(name) != Some(&stamp(&path)?)
                || read_limited(&path, 16384)? != contents.as_bytes()
            {
                return Err(Error::SourceChanged);
            }
        }
        // 本地 node.exe 出现/消失会改变官方 shim 的选路，不能只核原 Node 的字节。
        let local = self.prefix.join("node.exe");
        let actual = if local.try_exists().map_err(|_| Error::SourceChanged)? {
            stamp(&local)?
        } else {
            self.dependencies
                .first()
                .ok_or(Error::SourceChanged)?
                .clone()
        };
        if actual != self.node {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Phase {
    Allocating,
    Prepared,
    Publishing,
    ConfigPublishing,
    Committed,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Root {
    target: PathBuf,
    stage: PathBuf,
    backup: PathBuf,
    original: Option<tree::Snapshot>,
    prepared: Option<tree::Snapshot>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    generation: Uuid,
    input: managed_process::WindowsGrokNpmProbeInputs,
    digest: String,
    observed: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    id: Uuid,
    owner: Owner,
    old_version: String,
    target_version: String,
    roots: Vec<Root>,
    probes: Vec<Probe>,
    config: ConfigBackup,
    phase: Phase,
    intent: String,
}
pub(super) fn supports(agent: CLIAgent, target: &str) -> Result<(), Error> {
    if agent != CLIAgent::Grok {
        return Err(Error::UnsupportedSource);
    }
    if !cfg!(target_arch = "x86_64") {
        return Err(Error::UnsupportedPlatform);
    }
    super::npm_grok_contract::supports(target)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn journal_path(root: &Path) -> PathBuf {
    root.join("grok-npm-windows.json")
}
fn save(root: &Path, journal: &Journal) -> Result<(), Error> {
    let mut file = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    file.write_all(&serde_json::to_vec(journal).map_err(|_| Error::PersistenceFailed)?)
        .map_err(|_| Error::PersistenceFailed)?;
    file.as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    file.persist(journal_path(root))
        .map_err(|_| Error::PersistenceFailed)?;
    super::sync_config_directory(root)
}
fn unchanged(journal: &Journal, published: bool) -> Result<(), Error> {
    journal.owner.verify()?;
    if journal.config.path != journal.owner.home.join("config.toml")
        || !matches!(journal.config.kind, super::ConfigKind::Grok)
        || super::read_optional_config(&journal.config.path)?.as_ref()
            != journal.config.publication_bytes(published)
    {
        return Err(Error::SourceChanged);
    }
    Ok(())
}
fn verify_package(snapshot: &tree::Snapshot, version: &str) -> Result<(), Error> {
    let mut files = contract::files(version).map_err(|_| Error::InvalidRelease)?;
    let mut directories = BTreeSet::from([PathBuf::new()]);
    for path in files.keys() {
        directories.extend(path.ancestors().skip(1).map(Path::to_owned));
    }
    for (path, identity) in &snapshot.members {
        if identity.directory {
            if !directories.remove(path) {
                return Err(Error::SourceChanged);
            }
        } else {
            let (length, digest, _mode) = files.remove(path).ok_or(Error::SourceChanged)?;
            if identity.length != length || hex(&identity.digest) != digest {
                return Err(Error::SourceChanged);
            }
        }
    }
    if !files.is_empty() || !directories.is_empty() {
        return Err(Error::SourceChanged);
    }
    Ok(())
}
fn verify_native(snapshot: &tree::Snapshot, version: &str) -> Result<(), Error> {
    let expected = contract::native(version).map_err(|_| Error::InvalidRelease)?;
    let image = snapshot
        .members
        .get(Path::new(""))
        .ok_or(Error::SourceChanged)?;
    if snapshot.members.len() != 1
        || image.directory
        || image.length != expected.0
        || hex(&image.digest) != expected.1
    {
        return Err(Error::SourceChanged);
    }
    Ok(())
}
fn optional_snapshot(path: &Path) -> Result<Option<tree::Snapshot>, Error> {
    if path.try_exists().map_err(|_| Error::SourceChanged)? {
        tree::snapshot(path).map(Some)
    } else {
        Ok(None)
    }
}
fn paths(owner: &Owner, id: Uuid) -> Vec<(PathBuf, PathBuf, PathBuf)> {
    let bin = owner.home.join("bin");
    let parent = owner.package.parent().expect("已核验包父目录");
    vec![
        (
            bin.join(format!("grok-{}.exe", contract::VERSION)),
            bin.join(format!(".infinishell-grok-version-{id}.exe")),
            bin.join(format!(".infinishell-grok-version-old-{id}.exe")),
        ),
        (
            bin.join("grok.exe"),
            bin.join(format!(".infinishell-grok-current-{id}.exe")),
            bin.join(format!(".infinishell-grok-current-old-{id}.exe")),
        ),
        (
            owner.package.clone(),
            parent.join(format!(".infinishell-npm-{id}")),
            parent.join(format!(".infinishell-npm-old-{id}")),
        ),
    ]
}
fn probe_digest(
    input: &managed_process::WindowsGrokNpmProbeInputs,
    id: Uuid,
) -> Result<String, Error> {
    Ok(hex(&Sha256::digest(
        serde_json::to_vec(&(input, id)).map_err(|_| Error::PersistenceFailed)?,
    )))
}

async fn probe(root: &Path, journal: &mut Journal, mode: &str) -> Result<(), Error> {
    let input = managed_process::capture_windows_grok_npm_probe(
        mode,
        &journal.owner.node.canonical,
        &journal.roots[2].stage,
        &journal.owner.prefix,
        &journal.roots[1].stage,
        &journal.roots[0].stage,
    )
    .map_err(|_| Error::SourceChanged)?;
    let digest = probe_digest(&input, journal.id)?;
    let binding =
        managed_process::PreparedLaunchBinding::grok_windows_npm_version_probe(digest.clone())
            .map_err(|_| Error::RecoveryRequired)?;
    let generation = Uuid::new_v4();
    journal.probes.push(Probe {
        generation,
        input: input.clone(),
        digest,
        observed: None,
    });
    save(root, journal)?;
    let mut child =
        managed_process::spawn_bound_windows_grok_npm_probe(root, generation, &input, &binding)
            .await
            .map_err(|_| Error::RecoveryRequired)?;
    let mut bytes = Vec::new();
    let mut stdout = child.stdout.take().ok_or(Error::RecoveryRequired)?;
    let result = (&mut stdout)
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .with_timeout(UPDATE_TIMEOUT)
        .await;
    drop(stdout);
    let receipt = if result.is_ok() {
        child.finish_after_stdin_close().await
    } else {
        child.finish().await
    }
    .map_err(|_| Error::RecoveryRequired)?;
    if !receipt.cleanup_confirmed {
        return Err(Error::RecoveryRequired);
    }
    if !matches!(result, Ok(Ok(_)))
        || receipt.exit_code != Some(0)
        || bytes.len() as u64 > MAX_OUTPUT
    {
        return Err(Error::ProbeFailed);
    }
    let version = super::parse_cli_agent_version(
        CLIAgent::Grok,
        std::str::from_utf8(&bytes).map_err(|_| Error::ProbeFailed)?,
    )
    .ok_or(Error::ProbeFailed)?;
    if version != contract::VERSION {
        return Err(Error::VersionMismatch);
    }
    journal
        .probes
        .last_mut()
        .ok_or(Error::RecoveryRequired)?
        .observed = Some(version);
    save(root, journal)
}

fn verified_exit(root: &Path, journal: &Journal, success: bool) -> Result<(), Error> {
    if journal.probes.len() > 2 || success && journal.probes.len() != 2 {
        return Err(Error::RecoveryRequired);
    }
    for (index, probe) in journal.probes.iter().enumerate() {
        let mode = ["cmd", "powershell"][index];
        if probe.input.mode() != mode || probe.digest != probe_digest(&probe.input, journal.id)? {
            return Err(Error::RecoveryRequired);
        }
        managed_process::validate_windows_grok_npm_probe(&probe.input)
            .map_err(|_| Error::RecoveryRequired)?;
        if probe.input.stage() != journal.roots[2].stage
            || probe.input.canonical_stage() != journal.roots[1].stage
            || probe.input.version_stage() != journal.roots[0].stage
            || probe.input.node() != journal.owner.node.canonical
            || probe.input.prefix() != journal.owner.prefix
        {
            return Err(Error::RecoveryRequired);
        }
        let binding = managed_process::PreparedLaunchBinding::grok_windows_npm_version_probe(
            probe.digest.clone(),
        )
        .map_err(|_| Error::RecoveryRequired)?;
        super::record_not_started_if_missing(
            root,
            probe.generation,
            probe.input.program(),
            probe.input.arguments(),
            &binding,
        )?;
        let receipt =
            managed_process::confirmed_exit_with_binding(root, probe.generation, &binding)
                .map_err(|_| Error::RecoveryRequired)?
                .ok_or(Error::RecoveryRequired)?;
        managed_process::verify_windows_grok_npm_probe_dependencies(&probe.input)
            .map_err(|_| Error::SourceChanged)?;
        if !receipt.cleanup_confirmed
            || success
                && (receipt.exit_code != Some(0)
                    || probe.observed.as_deref() != Some(contract::VERSION))
        {
            return Err(Error::RecoveryRequired);
        }
    }
    Ok(())
}

pub(super) async fn execute(
    plan: &UpdatePlan,
    root: &Path,
    progress: Option<VerificationProgress>,
) -> Result<String, Error> {
    supports(plan.agent, &plan.target_version)?;
    if plan.installation.channel != super::Channel::Stable {
        return Err(Error::ChannelMismatch);
    }
    if plan.installation.source != Source::Npm
        || !matches!(plan.installed_version.as_str(), "1.0.40" | "1.0.41")
    {
        return Err(Error::UnsupportedSource);
    }
    if journal_path(root)
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    let config = plan.config.clone().ok_or(Error::UnsupportedSource)?;
    let owner = Owner::capture(&plan.installation, &plan.installed_version, &config)?;
    if !plan.requires_native_update() {
        let mut config = config;
        if super::read_optional_config(&config.path)? != config.before {
            return Err(Error::SourceChanged);
        }
        config.after = config.before.clone();
        let journal = super::Journal {
            schema: 2,
            agent: "grok".into(),
            entry: plan.installation.entry.clone(),
            old_version: plan.installed_version.clone(),
            target_version: plan.target_version.clone(),
            phase: "config_prepared".into(),
            config: Some(config),
            channel: super::channel_name(plan.installation.channel)?.into(),
            publish_desired: false,
            command_failed: false,
            intent: Some(plan.intent.clone()),
            generation: None,
            old_stamp: Some(plan.installation.stamp.clone()),
            claude_update: None,
            launch: None,
            binding_digest: None,
            binding_kind: None,
        };
        super::save_journal(&root.join("grok.json"), &journal)?;
        if let Some(progress) = progress {
            progress.enter().await;
        }
        super::reconcile_journal_locked(
            plan.agent,
            &plan.installation.entry,
            &plan.installed_version,
            root,
        )?;
        return Ok(plan.target_version.clone());
    }
    let id = Uuid::new_v4();
    let mut roots = Vec::new();
    for (index, (target, stage, backup)) in paths(&owner, id).into_iter().enumerate() {
        if stage.try_exists().map_err(|_| Error::SourceChanged)?
            || backup.try_exists().map_err(|_| Error::SourceChanged)?
        {
            return Err(Error::SourceChanged);
        }
        let original = optional_snapshot(&target)?;
        if let Some(snapshot) = &original {
            if index == 2 {
                verify_package(snapshot, &plan.installed_version)?;
            } else {
                verify_native(
                    snapshot,
                    if index == 0 {
                        contract::VERSION
                    } else {
                        &plan.installed_version
                    },
                )?;
            }
        } else if index != 0 {
            return Err(Error::UnsupportedSource);
        }
        roots.push(Root {
            target,
            stage,
            backup,
            original,
            prepared: None,
        });
    }
    let mut journal = Journal {
        schema: 1,
        id,
        owner,
        old_version: plan.installed_version.clone(),
        target_version: plan.target_version.clone(),
        roots,
        probes: Vec::new(),
        config,
        phase: Phase::Allocating,
        intent: plan.intent.clone(),
    };
    unchanged(&journal, false)?;
    // 全部归档先按固定 SRI 与成员摘要核验，再产生任何安装位置的候选文件。
    let mut packages = super::npm_grok_contract::download_release(root).await?;
    unchanged(&journal, false)?;
    save(root, &journal)?;
    let result = async {
        let package_stage = journal.roots[2].stage.clone();
        let _parents = tree::parents(&package_stage)?;
        fs::create_dir(&package_stage).map_err(|_| Error::PersistenceFailed)?;
        tree::copy_security(&journal.owner.package, &package_stage)?;
        let mut directories = BTreeSet::from([PathBuf::new()]);
        for package in &mut packages {
            let prefix = package.prefix.clone();
            package.artifact.extract_verified(
                package.archive.as_file_mut(),
                |path, entry, contents| {
                    let relative = prefix.join(path);
                    tree::safe_relative(&relative)?;
                    let mut directory = PathBuf::new();
                    for part in relative.parent().ok_or(Error::InvalidRelease)?.components() {
                        directory.push(part);
                        if directories.insert(directory.clone()) {
                            fs::create_dir(package_stage.join(&directory))
                                .map_err(|_| Error::PersistenceFailed)?;
                        }
                    }
                    let target = package_stage.join(&relative);
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&target)
                        .map_err(|_| Error::PersistenceFailed)?;
                    let size =
                        std::io::copy(contents, &mut file).map_err(|_| Error::PersistenceFailed)?;
                    file.sync_all().map_err(|_| Error::PersistenceFailed)?;
                    drop(file);
                    if size != entry.length || stamp(&target)?.digest != entry.sha256 {
                        return Err(Error::InvalidRelease);
                    }
                    if journal.roots[2]
                        .original
                        .as_ref()
                        .and_then(|tree| tree.members.get(&relative))
                        .is_some_and(|file| !file.directory)
                    {
                        tree::copy_security(&journal.owner.package.join(&relative), &target)?;
                    }
                    Ok(())
                },
            )?;
        }
        for directory in &directories {
            if !directory.as_os_str().is_empty()
                && journal.roots[2]
                    .original
                    .as_ref()
                    .and_then(|tree| tree.members.get(directory))
                    .is_some_and(|file| file.directory)
            {
                tree::copy_security(
                    &journal.owner.package.join(directory),
                    &package_stage.join(directory),
                )?;
            }
        }
        let package = tree::snapshot(&package_stage)?;
        verify_package(&package, contract::VERSION)?;
        journal.roots[2].prepared = Some(package);
        save(root, &journal)?;
        // Windows 官方布局是两个独立副本；保留 Node wrapper、压缩平台包和 TOML 依赖。
        for index in [0, 1] {
            let stage = journal.roots[index].stage.clone();
            let _parents = tree::parents(&stage)?;
            let mut input = OpenOptions::new()
                .read(true)
                .open(package_stage.join(contract::COMPRESSED))
                .map_err(|_| Error::SourceChanged)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)
                .map_err(|_| Error::PersistenceFailed)?;
            super::npm_grok_contract::decompress(&mut input, &mut output)?;
            drop(output);
            tree::copy_security(&journal.roots[1].target, &stage)?;
            let prepared = tree::snapshot(&stage)?;
            verify_native(&prepared, contract::VERSION)?;
            journal.roots[index].prepared = Some(prepared);
            save(root, &journal)?;
        }
        journal.phase = Phase::Prepared;
        save(root, &journal)?;
        probe(root, &mut journal, "cmd").await?;
        probe(root, &mut journal, "powershell").await?;
        verified_exit(root, &journal, true)?;
        unchanged(&journal, false)?;
        let mut images = Vec::new();
        for item in &journal.roots {
            if let Some(original) = &item.original {
                images.extend(tree::freeze_images(&item.target, original)?);
            }
            images.extend(tree::freeze_images(
                &item.stage,
                item.prepared.as_ref().ok_or(Error::RecoveryRequired)?,
            )?);
        }
        journal.phase = Phase::Publishing;
        save(root, &journal)?;
        // 每个根移动前再次检查 config 与全部身份；中断恢复读取实际身份，不依赖最后一条阶段字符串。
        for index in 0..journal.roots.len() {
            unchanged(&journal, false)?;
            let item = &journal.roots[index];
            if let Some(original) = &item.original {
                tree::rename(&item.target, &item.backup, original)?;
            } else if item.target.try_exists().map_err(|_| Error::SourceChanged)? {
                return Err(Error::SourceChanged);
            }
            tree::rename(
                &item.stage,
                &item.target,
                item.prepared.as_ref().ok_or(Error::RecoveryRequired)?,
            )?;
            save(root, &journal)?;
        }
        drop(images);
        if let Some(progress) = progress {
            progress.enter().await;
        }
        finish_publish(root, &mut journal)?;
        Ok(contract::VERSION.to_owned())
    }
    .await;
    if result.is_err()
        && recover(CLIAgent::Grok, &plan.installation.entry, root)?
            == Some(contract::VERSION.to_owned())
    {
        return Ok(contract::VERSION.to_owned());
    }
    result
}

fn finish_publish(root: &Path, journal: &mut Journal) -> Result<(), Error> {
    verified_exit(root, journal, true)?;
    journal.owner.verify()?;
    for item in &journal.roots {
        if tree::snapshot(&item.target)?
            != *item.prepared.as_ref().ok_or(Error::RecoveryRequired)?
        {
            return Err(Error::RecoveryRequired);
        }
    }
    if !matches!(journal.phase, Phase::ConfigPublishing | Phase::Committed) {
        unchanged(journal, false)?;
        journal.config.after = journal.config.before.clone();
        super::plan_config_publish(&mut journal.config, true)?;
        journal.phase = Phase::ConfigPublishing;
        save(root, journal)?;
    }
    // 安装器来源和 npm_registry 不由本事务重写；只有用户选择的渠道按现有配置事务发布。
    super::publish_config(&journal.config, true)?;
    unchanged(journal, true)?;
    journal.phase = Phase::Committed;
    save(root, journal)?;
    for item in &journal.roots {
        if let Some(original) = &item.original {
            tree::remove_matching(&item.backup, original)?;
        }
    }
    super::cleanup_config_restore(&journal.config)?;
    match fs::remove_file(super::failure_path(root, CLIAgent::Grok)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    fs::remove_file(journal_path(root)).map_err(|_| Error::PersistenceFailed)?;
    super::sync_config_directory(root)
}

pub(super) fn recover(agent: CLIAgent, entry: &Path, root: &Path) -> Result<Option<String>, Error> {
    if agent != CLIAgent::Grok
        || !journal_path(root)
            .try_exists()
            .map_err(|_| Error::RecoveryRequired)?
    {
        return Ok(None);
    }
    let mut journal: Journal =
        serde_json::from_slice(&read_limited(&journal_path(root), 24 * MAX_CONFIG)?)
            .map_err(|_| Error::RecoveryRequired)?;
    if journal.schema != 1
        || journal.id.is_nil()
        || journal.owner.entry != entry
        || journal.target_version != contract::VERSION
        || !matches!(journal.old_version.as_str(), "1.0.40" | "1.0.41")
        || journal.roots.len() != 3
    {
        return Err(Error::RecoveryRequired);
    }
    journal.owner.verify()?;
    for (index, (item, expected)) in journal
        .roots
        .iter()
        .zip(paths(&journal.owner, journal.id))
        .enumerate()
    {
        if (&item.target, &item.stage, &item.backup) != (&expected.0, &expected.1, &expected.2) {
            return Err(Error::RecoveryRequired);
        }
        if let Some(original) = &item.original {
            if index == 2 {
                verify_package(original, &journal.old_version)?;
            } else {
                verify_native(
                    original,
                    if index == 0 {
                        contract::VERSION
                    } else {
                        &journal.old_version
                    },
                )?;
            }
        } else if index != 0 {
            return Err(Error::RecoveryRequired);
        }
        if let Some(prepared) = &item.prepared {
            if index == 2 {
                verify_package(prepared, contract::VERSION)?;
            } else {
                verify_native(prepared, contract::VERSION)?;
            }
        }
    }
    verified_exit(root, &journal, false)?;
    if matches!(journal.phase, Phase::ConfigPublishing | Phase::Committed) {
        finish_publish(root, &mut journal)?;
        return Ok(Some(contract::VERSION.to_owned()));
    }
    unchanged(&journal, false)?;
    let mut retain = false;
    for item in journal.roots.iter().rev() {
        let current = optional_snapshot(&item.target)?;
        if current != item.original {
            if let Some(current) = &current {
                if Some(current) != item.prepared.as_ref()
                    || item
                        .stage
                        .try_exists()
                        .map_err(|_| Error::RecoveryRequired)?
                {
                    return Err(Error::RecoveryRequired);
                }
                let _images = tree::freeze_images(&item.target, current)?;
                tree::rename(&item.target, &item.stage, current)?;
            }
            if let Some(original) = &item.original {
                let _images = tree::freeze_images(&item.backup, original)?;
                tree::rename(&item.backup, &item.target, original)?;
            }
        }
        if optional_snapshot(&item.target)? != item.original
            || item
                .backup
                .try_exists()
                .map_err(|_| Error::RecoveryRequired)?
        {
            return Err(Error::RecoveryRequired);
        }
        if let Some(prepared) = &item.prepared {
            tree::remove_matching(&item.stage, prepared)?;
        } else if item
            .stage
            .try_exists()
            .map_err(|_| Error::RecoveryRequired)?
        {
            retain = true;
        }
    }
    if retain {
        // 候选尚未取得完整清单时不删除；保留所有根与阶段，便于后续确认并回收。
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(format!("grok-npm-windows-retained-{}.json", journal.id)))
            .map_err(|_| Error::RecoveryRequired)?;
        file.write_all(&serde_json::to_vec(&journal).map_err(|_| Error::PersistenceFailed)?)
            .map_err(|_| Error::PersistenceFailed)?;
        file.sync_all().map_err(|_| Error::PersistenceFailed)?;
    }
    super::save_failure_with_intent(
        root,
        CLIAgent::Grok,
        entry,
        contract::VERSION,
        Some(journal.intent),
    )?;
    fs::remove_file(journal_path(root)).map_err(|_| Error::PersistenceFailed)?;
    super::sync_config_directory(root)?;
    Ok(None)
}
