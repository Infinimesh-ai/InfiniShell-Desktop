//! Grok Unix npm 双目录事务：包入口和用户 bin 同步发布，恢复只认已记录的身份。

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Seek as _, SeekFrom, Write as _};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use futures::AsyncReadExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;

use super::npm_grok_contract as contract;
use super::package_tree::{Directory, GrokSnapshot, Identity, grok_mirror::Mirror};
use super::{
    ArtifactRole, CLIAgent, ConfigBackup, Error, Installation, MAX_CONFIG, MAX_OUTPUT, Stamp,
    UPDATE_TIMEOUT, UpdatePlan, VerificationProgress, managed_process, npm, plain_ancestors,
    read_limited, read_optional_config, stamp, verify_installation_identity,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryLink {
    path: PathBuf,
    target: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}

fn entry_link(path: &Path) -> Result<EntryLink, Error> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Error::SourceChanged)?;
    if !metadata.file_type().is_symlink() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(Error::UnsupportedSource);
    }
    Ok(EntryLink {
        path: path.to_owned(),
        target: fs::read_link(path).map_err(|_| Error::SourceChanged)?,
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        mode: metadata.mode(),
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    prefix: PathBuf,
    prefix_identity: Identity,
    parent_identity: Identity,
    package: PathBuf,
    entry: PathBuf,
    external_link: Option<EntryLink>,
    external_files: Vec<Stamp>,
    home: PathBuf,
}

impl Owner {
    fn capture(installation: &Installation, old: &str) -> Result<Self, Error> {
        verify_installation_identity(installation)?;
        let invocation = installation
            .invocation
            .as_ref()
            .ok_or(Error::UnsupportedSource)?;
        let prefix = invocation
            .artifacts
            .get(&ArtifactRole::InstallRoot)
            .ok_or(Error::UnsupportedSource)?;
        let registration =
            npm::registered_installation(CLIAgent::Grok, installation, old, prefix, false)?
                .ok_or(Error::UnsupportedSource)?;
        let home = contract::grok_home()?;
        contract::verify_installer_config(&home)?;
        let parent = registration
            .package_root
            .parent()
            .ok_or(Error::UnsupportedSource)?;
        let external_link = if installation.entry.starts_with(&registration.package_root)
            || installation.entry == home.join("bin/grok")
        {
            None
        } else {
            Some(entry_link(&installation.entry)?)
        };
        Ok(Self {
            prefix: registration.prefix.clone(),
            prefix_identity: Directory::open(&registration.prefix)?.identity()?,
            parent_identity: Directory::open(parent)?.identity()?,
            package: registration.package_root,
            entry: installation.entry.clone(),
            external_link,
            external_files: vec![
                installation
                    .manager
                    .clone()
                    .ok_or(Error::UnsupportedSource)?,
                installation
                    .helper
                    .clone()
                    .ok_or(Error::UnsupportedSource)?,
            ],
            home,
        })
    }

    fn validate(&self, entry: &Path) -> Result<(), Error> {
        if !self.prefix.is_absolute()
            || self.package != self.prefix.join("lib/node_modules/@xai-official/grok")
            || self.entry != entry
            || self.home != contract::grok_home()?
            || self.external_files.len() != 2
        {
            return Err(Error::RecoveryRequired);
        }
        if let Some(link) = &self.external_link {
            if link.path != self.entry || self.entry != self.prefix.join("bin/grok") {
                return Err(Error::RecoveryRequired);
            }
        } else if self.entry != self.home.join("bin/grok")
            && self.entry != self.package.join("bin/grok")
            && self.entry != self.package.join("bin/grok-native")
        {
            return Err(Error::RecoveryRequired);
        }
        Ok(())
    }

    fn parent(&self) -> Result<Directory, Error> {
        if Directory::open(&self.prefix)?.identity()? != self.prefix_identity {
            return Err(Error::SourceChanged);
        }
        let parent = Directory::open(self.package.parent().ok_or(Error::SourceChanged)?)?;
        if parent.identity()? != self.parent_identity {
            return Err(Error::SourceChanged);
        }
        for expected in &self.external_files {
            if stamp(&expected.canonical)? != *expected {
                return Err(Error::SourceChanged);
            }
        }
        if let Some(expected) = &self.external_link
            && entry_link(&expected.path)? != *expected
        {
            return Err(Error::SourceChanged);
        }
        Ok(parent)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Phase {
    Allocating,
    Preparing,
    SwapIntent,
    PublishingConfig,
    Committed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    generation: Uuid,
    program: PathBuf,
    identity: Stamp,
    binding: String,
    version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    id: Uuid,
    old_version: String,
    target_version: String,
    intent: String,
    owner: Owner,
    stage: OsString,
    phase: Phase,
    original: GrokSnapshot,
    prepared: Option<GrokSnapshot>,
    mirror: Mirror,
    config: ConfigBackup,
    probe: Option<Probe>,
    archives: Vec<[u8; 32]>,
}

pub(super) fn supports(agent: CLIAgent, version: &str) -> Result<(), Error> {
    if agent != CLIAgent::Grok {
        return Err(Error::UnsupportedSource);
    }
    contract::supports(version)
}

fn path(root: &Path) -> PathBuf {
    root.join("grok-npm.json")
}

fn save(path: &Path, journal: &Journal) -> Result<(), Error> {
    plain_ancestors(path)?;
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
    super::sync_config_directory(parent)
}

fn verify_release(snapshot: &GrokSnapshot, version: &str) -> Result<(), Error> {
    if snapshot.link.is_none() {
        return Err(Error::UnsupportedSource);
    }
    let files = contract::installed_files(version)?
        .into_iter()
        .map(|(path, file)| Ok((path, (file.length, file.digest()?))))
        .collect::<Result<BTreeMap<_, _>, Error>>()?;
    snapshot.tree.verify_release_files(&files)
}

fn config_unchanged(journal: &Journal, published: bool) -> Result<(), Error> {
    if read_optional_config(&journal.config.path)?.as_ref()
        != journal.config.publication_bytes(published)
        || journal.config.before_mode.is_some_and(|mode| {
            fs::metadata(&journal.config.path)
                .ok()
                .is_none_or(|metadata| metadata.mode() & 0o777 != mode)
        })
    {
        return Err(Error::SourceChanged);
    }
    Ok(())
}

pub(super) async fn execute(
    plan: &UpdatePlan,
    root: &Path,
    progress: Option<VerificationProgress>,
) -> Result<String, Error> {
    supports(plan.agent, &plan.target_version)?;
    let path = path(root);
    if path.try_exists().map_err(|_| Error::RecoveryRequired)? {
        return Err(Error::RecoveryRequired);
    }
    let owner = Owner::capture(&plan.installation, &plan.installed_version)?;
    owner.validate(&plan.installation.entry)?;
    let parent = owner.parent()?;
    let original = parent.child(OsStr::new("grok"))?.grok_snapshot()?;
    verify_release(&original, &plan.installed_version)?;
    let config = plan.config.clone().ok_or(Error::UnsupportedSource)?;
    if config.path != owner.home.join("config.toml") {
        return Err(Error::UnsupportedSource);
    }
    let id = Uuid::new_v4();
    let mirror = Mirror::capture(&owner.home, &plan.installed_version, id)?;
    let mut packages = contract::download_release(root).await?;
    let mut journal = Journal {
        schema: 1,
        id,
        old_version: plan.installed_version.clone(),
        target_version: plan.target_version.clone(),
        intent: plan.intent.clone(),
        owner,
        stage: format!(".infinishell-grok-npm-{id}").into(),
        phase: Phase::Allocating,
        original,
        prepared: None,
        mirror,
        config,
        probe: None,
        archives: packages
            .iter()
            .map(|package| package.verified.compressed_sha256)
            .collect(),
    };
    config_unchanged(&journal, false)?;
    save(&path, &journal)?;
    let result = async {
        let stage = parent.create(&journal.stage)?;
        journal.phase = Phase::Preparing;
        save(&path, &journal)?;
        let mut native = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
        for package in &mut packages {
            let prefix = &package.prefix;
            package.artifact.extract_verified(
                package.archive.as_file_mut(),
                |path, file, bytes| {
                    if prefix.as_os_str().is_empty() && path == Path::new("bin/grok") {
                        // 官方脚本将此占位入口替换为链接；仍读取完整成员供归档校验。
                        std::io::copy(bytes, &mut std::io::sink())
                            .map_err(|_| Error::InvalidRelease)?;
                        return Ok(());
                    }
                    stage.write_new(&prefix.join(path), bytes, file.length, file.sha256)
                },
            )?;
            if prefix.starts_with("node_modules/@xai-official") {
                // 第二遍完整校验 tarball 后才把唯一 .br 交给宿主解压器。
                package.artifact.extract_verified(
                    package.archive.as_file_mut(),
                    |path, _, bytes| {
                        if path == contract::compressed_entry() {
                            contract::decompress(bytes, native.as_file_mut())
                        } else {
                            std::io::copy(bytes, &mut std::io::sink())
                                .map(|_| ())
                                .map_err(|_| Error::InvalidRelease)
                        }
                    },
                )?;
            }
        }
        let expected = contract::native(contract::VERSION)?;
        native
            .as_file_mut()
            .seek(SeekFrom::Start(0))
            .map_err(|_| Error::PersistenceFailed)?;
        stage.write_new(
            Path::new("bin/grok-native"),
            native.as_file_mut(),
            expected.length,
            expected.digest()?,
        )?;
        stage.create_grok_link()?;
        let executables = contract::installed_files(contract::VERSION)?
            .into_iter()
            .map(|(path, file)| (path, file.executable))
            .collect();
        stage.apply_grok_permissions(&journal.original.tree, &executables)?;
        let prepared = stage.grok_snapshot()?;
        verify_release(&prepared, contract::VERSION)?;
        journal.prepared = Some(prepared);
        native
            .as_file_mut()
            .seek(SeekFrom::Start(0))
            .map_err(|_| Error::PersistenceFailed)?;
        journal.mirror.prepare(native.as_file_mut())?;
        save(&path, &journal)?;
        #[cfg(test)]
        live_tests::checkpoint(live_tests::Point::Prepared).await;
        probe(root, &path, &mut journal).await?;
        config_unchanged(&journal, false)?;
        let parent = journal.owner.parent()?;
        if parent.child(OsStr::new("grok"))?.grok_snapshot()? != journal.original
            || Some(&parent.child(&journal.stage)?.grok_snapshot()?) != journal.prepared.as_ref()
        {
            return Err(Error::SourceChanged);
        }
        journal.phase = Phase::SwapIntent;
        save(&path, &journal)?;
        parent.exchange(OsStr::new("grok"), &journal.stage)?;
        journal.mirror.publish()?;
        #[cfg(test)]
        live_tests::checkpoint(live_tests::Point::Exchanged).await;
        if let Some(progress) = progress {
            progress.enter().await;
        }
        journal.phase = Phase::PublishingConfig;
        journal.config.after = journal.config.before.clone();
        super::plan_config_publish(&mut journal.config, true)?;
        save(&path, &journal)?;
        super::publish_config(&journal.config, true)?;
        config_unchanged(&journal, true)?;
        journal.mirror.verify_published()?;
        if Some(&parent.child(OsStr::new("grok"))?.grok_snapshot()?) != journal.prepared.as_ref() {
            return Err(Error::SourceChanged);
        }
        journal.phase = Phase::Committed;
        save(&path, &journal)?;
        finish(root, &path, &journal)?;
        Ok(plan.target_version.clone())
    }
    .await;
    if result.is_err()
        && recover(plan.agent, &plan.installation.entry, root)? == Some(plan.target_version.clone())
    {
        return Ok(plan.target_version.clone());
    }
    result
}

fn binding(probe: &Probe) -> Result<managed_process::PreparedLaunchBinding, Error> {
    managed_process::PreparedLaunchBinding::grok_npm_version_probe(probe.binding.clone())
        .map_err(|_| Error::RecoveryRequired)
}

async fn probe(root: &Path, path: &Path, journal: &mut Journal) -> Result<(), Error> {
    let program = journal
        .owner
        .package
        .parent()
        .ok_or(Error::RecoveryRequired)?
        .join(&journal.stage)
        .join("bin/grok-native");
    let identity = stamp(&program)?;
    let expected = contract::native(contract::VERSION)?;
    if identity.canonical != program || identity.digest != expected.digest()? {
        return Err(Error::SourceChanged);
    }
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&identity, "--version", journal.id))
                .map_err(|_| Error::PersistenceFailed)?
        )
    );
    let generation = Uuid::new_v4();
    journal.probe = Some(Probe {
        generation,
        program: program.clone(),
        identity,
        binding: digest,
        version: None,
    });
    save(path, journal)?;
    let launch = binding(journal.probe.as_ref().ok_or(Error::RecoveryRequired)?)?;
    let expected = managed_process::ExpectedFileIdentity::capture_release_image(
        &program,
        expected.length,
        expected.digest()?,
    )
    .map_err(|_| Error::SourceChanged)?;
    let mut child =
        managed_process::spawn_bound_version_probe(root, generation, &program, expected, &launch)
            .await
            .map_err(|_| Error::RecoveryRequired)?;
    let mut stdout = child.stdout.take().ok_or(Error::RecoveryRequired)?;
    let mut bytes = Vec::new();
    let result = (&mut stdout)
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .with_timeout(UPDATE_TIMEOUT)
        .await;
    #[cfg(test)]
    live_tests::preserve_probe_stdout(generation, &bytes);
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
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => return Err(Error::ProbeFailed),
        Err(_) => return Err(Error::TimedOut),
    }
    if receipt.exit_code != Some(0) || bytes.len() as u64 > MAX_OUTPUT {
        return Err(Error::ProbeFailed);
    }
    let actual = super::parse_cli_agent_version(
        CLIAgent::Grok,
        std::str::from_utf8(&bytes).map_err(|_| Error::ProbeFailed)?,
    )
    .ok_or(Error::ProbeFailed)?;
    if actual != contract::VERSION {
        return Err(Error::VersionMismatch);
    }
    journal
        .probe
        .as_mut()
        .ok_or(Error::RecoveryRequired)?
        .version = Some(actual);
    save(path, journal)
}

fn confirmed(root: &Path, journal: &Journal) -> Result<(), Error> {
    if let Some(probe) = &journal.probe {
        let launch = binding(probe)?;
        super::record_not_started_if_missing(
            root,
            probe.generation,
            &probe.program,
            &["--version".into()],
            &launch,
        )?;
        managed_process::confirmed_exit_with_binding(root, probe.generation, &launch)
            .map_err(|_| Error::RecoveryRequired)?
            .ok_or(Error::RecoveryRequired)?;
    }
    Ok(())
}

fn validate(entry: &Path, journal: &Journal) -> Result<(), Error> {
    contract::supports(&journal.target_version)?;
    if journal.schema != 1
        || journal.id.is_nil()
        || journal.stage != OsString::from(format!(".infinishell-grok-npm-{}", journal.id))
        || journal.archives.len() != 3
        || journal.config.path != journal.owner.home.join("config.toml")
        || !matches!(journal.config.kind, super::ConfigKind::Grok)
    {
        return Err(Error::RecoveryRequired);
    }
    journal.owner.validate(entry)?;
    journal
        .mirror
        .validate(&journal.owner.home, &journal.old_version, journal.id)?;
    verify_release(&journal.original, &journal.old_version)?;
    if let Some(prepared) = &journal.prepared {
        verify_release(prepared, &journal.target_version)?;
    }
    if let Some(probe) = &journal.probe {
        let program = journal
            .owner
            .package
            .parent()
            .ok_or(Error::RecoveryRequired)?
            .join(&journal.stage)
            .join("bin/grok-native");
        let digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&probe.identity, "--version", journal.id))
                    .map_err(|_| Error::RecoveryRequired)?
            )
        );
        if probe.generation.is_nil()
            || probe.program != program
            || probe.identity.canonical != program
            || probe.identity.digest != contract::native(contract::VERSION)?.digest()?
            || probe.binding != digest
            || probe
                .version
                .as_ref()
                .is_some_and(|version| version != contract::VERSION)
        {
            return Err(Error::RecoveryRequired);
        }
    }
    Ok(())
}

pub(super) fn recover(agent: CLIAgent, entry: &Path, root: &Path) -> Result<Option<String>, Error> {
    if agent != CLIAgent::Grok
        || !path(root)
            .try_exists()
            .map_err(|_| Error::RecoveryRequired)?
    {
        return Ok(None);
    }
    let path = path(root);
    let mut journal: Journal = serde_json::from_slice(&read_limited(&path, 12 * MAX_CONFIG)?)
        .map_err(|_| Error::RecoveryRequired)?;
    validate(entry, &journal)?;
    confirmed(root, &journal)?;
    if journal.phase == Phase::PublishingConfig {
        let parent = journal.owner.parent()?;
        if Some(&parent.child(OsStr::new("grok"))?.grok_snapshot()?) != journal.prepared.as_ref()
            || parent.child(&journal.stage)?.grok_snapshot()? != journal.original
            || journal
                .probe
                .as_ref()
                .and_then(|probe| probe.version.as_deref())
                != Some(contract::VERSION)
        {
            return Err(Error::RecoveryRequired);
        }
        journal.mirror.verify_published()?;
        super::publish_config(&journal.config, true)?;
        config_unchanged(&journal, true)?;
        journal.phase = Phase::Committed;
        save(&path, &journal)?;
    }
    if journal.phase == Phase::Committed {
        finish(root, &path, &journal)?;
        return Ok(Some(journal.target_version));
    }
    let parent = journal.owner.parent()?;
    let current = parent.child(OsStr::new("grok"))?.grok_snapshot()?;
    if current != journal.original {
        if Some(&current) != journal.prepared.as_ref()
            || parent.child(&journal.stage)?.grok_snapshot()? != journal.original
        {
            return Err(Error::RecoveryRequired);
        }
        parent.exchange(OsStr::new("grok"), &journal.stage)?;
    }
    journal.mirror.finish(false)?;
    config_unchanged(&journal, false)?;
    if let Some(prepared) = &journal.prepared {
        if parent.has_child(&journal.stage)? {
            parent.remove_grok_matching(&journal.stage, prepared)?;
        }
    }
    // 准备中断的未完成私有文件保留到维护清理，不猜测其当前归属。
    if matches!(journal.phase, Phase::Allocating | Phase::Preparing) {
        save(
            &root.join(format!("grok-npm-retained-{}.json", journal.id)),
            &journal,
        )?;
    }
    super::cleanup_config_restore(&journal.config)?;
    super::save_failure_with_intent(
        root,
        agent,
        entry,
        &journal.target_version,
        Some(journal.intent.clone()),
    )?;
    fs::remove_file(&path).map_err(|_| Error::RecoveryRequired)?;
    super::sync_config_directory(root)?;
    Ok(None)
}

fn finish(root: &Path, path: &Path, journal: &Journal) -> Result<(), Error> {
    if journal
        .probe
        .as_ref()
        .and_then(|probe| probe.version.as_deref())
        != Some(contract::VERSION)
    {
        return Err(Error::RecoveryRequired);
    }
    confirmed(root, journal)?;
    config_unchanged(journal, true)?;
    let parent = journal.owner.parent()?;
    if Some(&parent.child(OsStr::new("grok"))?.grok_snapshot()?) != journal.prepared.as_ref() {
        return Err(Error::RecoveryRequired);
    }
    journal.mirror.finish(true)?;
    if parent.has_child(&journal.stage)? {
        parent.remove_grok_matching(&journal.stage, &journal.original)?;
    }
    super::cleanup_config_restore(&journal.config)?;
    match fs::remove_file(super::failure_path(root, CLIAgent::Grok)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
    super::sync_config_directory(root)
}

#[cfg(test)]
#[path = "sources_grok_npm_live_tests.rs"]
pub(super) mod live_tests;
