//! Windows x64 Claude npm 的整包候选、双公共入口探针和可恢复发布。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::windows::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};

use futures::{AsyncReadExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

use super::npm_claude_windows_contract as contract;
use super::npm_claude_windows_tree as tree;
use super::npm_release::{NpmRelease, VerifiedNpmArchive};
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
        .is_none_or(|name| name != "claude.ps1")
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
    dependencies: Vec<Stamp>,
    shims: BTreeMap<String, Stamp>,
}

impl Owner {
    fn capture(installation: &Installation, old: &str) -> Result<Self, Error> {
        super::verify_installation_identity(installation)?;
        let prefix = installation
            .invocation
            .as_ref()
            .and_then(|invocation| invocation.artifacts.get(&ArtifactRole::InstallRoot))
            .ok_or(Error::UnsupportedSource)?;
        let registration =
            npm::registered_installation(CLIAgent::Claude, installation, old, prefix, true)?
                .ok_or(Error::UnsupportedSource)?;
        if ![
            registration.prefix.join("claude.cmd"),
            registration.prefix.join("claude.ps1"),
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
            dependencies: vec![manager, helper, stamp(&npm_manifest)?],
            shims,
        };
        owner.verify()?;
        Ok(owner)
    }

    fn verify(&self) -> Result<(), Error> {
        if self.dependencies.len() != 3
            || self.package != self.prefix.join("node_modules/@anthropic-ai/claude-code")
            || tree::path_identity(&self.prefix)? != self.prefix_identity
            || tree::path_identity(self.package.parent().ok_or(Error::SourceChanged)?)?
                != self.parent_identity
            || ![
                self.prefix.join("claude.cmd"),
                self.prefix.join("claude.ps1"),
            ]
            .contains(
                &self
                    .entry
                    .canonicalize()
                    .map_err(|_| Error::SourceChanged)?,
            )
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
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Phase {
    Allocating,
    Prepared,
    OldMoved,
    Published,
    ConfigPublishing,
    Committed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    generation: Uuid,
    input: managed_process::WindowsClaudeNpmProbeInputs,
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
    stage: PathBuf,
    backup: PathBuf,
    original: tree::Snapshot,
    prepared: Option<tree::Snapshot>,
    probes: Vec<Probe>,
    config: Option<ConfigBackup>,
    claude_policy: super::ClaudeUpdateScope,
    phase: Phase,
    intent: String,
}

pub(super) fn supports(agent: CLIAgent, target: &str) -> Result<(), Error> {
    if agent != CLIAgent::Claude {
        return Err(Error::UnsupportedSource);
    }
    if !cfg!(target_arch = "x86_64") {
        return Err(Error::UnsupportedPlatform);
    }
    if target != contract::VERSION {
        return Err(Error::InvalidRelease);
    }
    Ok(())
}

fn journal_path(root: &Path) -> PathBuf {
    root.join("claude-npm-windows.json")
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

async fn download(url: &str, root: &Path, limit: u64) -> Result<NamedTempFile, Error> {
    let mut file = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    let response = http_client::Client::new()
        .get(url)
        .timeout(UPDATE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() || response.url().as_str() != url {
        return Err(Error::Network);
    }
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    let mut length = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Network)?;
        length = length
            .checked_add(chunk.len() as u64)
            .ok_or(Error::InvalidRelease)?;
        if length > limit {
            return Err(Error::InvalidRelease);
        }
        file.write_all(&chunk)
            .map_err(|_| Error::PersistenceFailed)?;
    }
    if length == 0 {
        return Err(Error::InvalidRelease);
    }
    file.as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    Ok(file)
}

fn verify_archives(
    wrapper: &VerifiedNpmArchive,
    platform: &VerifiedNpmArchive,
) -> Result<(), Error> {
    let mut expected = contract::archive_files();
    for (prefix, archive) in [
        (PathBuf::new(), wrapper),
        (PathBuf::from(contract::DEPENDENCY), platform),
    ] {
        for (path, file) in &archive.files {
            let path = prefix.join(path);
            if let Some((length, digest, executable)) = expected.remove(&path) {
                if file.length != length
                    || hex(&file.sha256) != digest
                    || file.executable != executable
                {
                    return Err(Error::InvalidRelease);
                }
            } else {
                return Err(Error::InvalidRelease);
            }
        }
    }
    if !expected.is_empty() {
        return Err(Error::InvalidRelease);
    }
    Ok(())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn verify_candidate(snapshot: &tree::Snapshot) -> Result<(), Error> {
    let mut files = contract::files();
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
            let (length, digest, _executable) = files.remove(path).ok_or(Error::SourceChanged)?;
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

fn unchanged(journal: &Journal, published: bool) -> Result<(), Error> {
    if journal
        .config
        .as_ref()
        .is_none_or(|config| config.path != journal.claude_policy.settings_path)
    {
        return Err(Error::RecoveryRequired);
    }
    super::verify_claude_originals(&journal.claude_policy)?;
    if let Some(config) = &journal.config
        && super::read_optional_config(&config.path)?.as_ref()
            != config.publication_bytes(published)
    {
        return Err(Error::SourceChanged);
    }
    Ok(())
}

fn probe_digest(
    input: &managed_process::WindowsClaudeNpmProbeInputs,
    id: Uuid,
) -> Result<String, Error> {
    Ok(hex(&Sha256::digest(
        serde_json::to_vec(&(input, id)).map_err(|_| Error::PersistenceFailed)?,
    )))
}

async fn probe(root: &Path, journal: &mut Journal, mode: &str) -> Result<(), Error> {
    let input = managed_process::capture_windows_claude_npm_probe(
        mode,
        &journal.stage,
        &journal.owner.prefix,
    )
    .map_err(|_| Error::SourceChanged)?;
    let digest = probe_digest(&input, journal.id)?;
    let binding =
        managed_process::PreparedLaunchBinding::claude_windows_npm_version_probe(digest.clone())
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
        managed_process::spawn_bound_windows_claude_npm_probe(root, generation, &input, &binding)
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
        CLIAgent::Claude,
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
        managed_process::validate_windows_claude_npm_probe(&probe.input)
            .map_err(|_| Error::RecoveryRequired)?;
        if probe.input.stage() != journal.stage || probe.input.prefix() != journal.owner.prefix {
            return Err(Error::RecoveryRequired);
        }
        let binding = managed_process::PreparedLaunchBinding::claude_windows_npm_version_probe(
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
        managed_process::verify_windows_claude_npm_probe_dependencies(&probe.input)
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
    if plan.installation.source != Source::Npm {
        return Err(Error::UnsupportedSource);
    }
    if !matches!(plan.installed_version.as_str(), "2.1.278" | "2.1.280") {
        return Err(Error::InvalidRelease);
    }
    if journal_path(root)
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    let owner = Owner::capture(&plan.installation, &plan.installed_version)?;
    if !plan.requires_native_update() {
        let mut config = plan.config.clone().ok_or(Error::UnsupportedSource)?;
        if super::read_optional_config(&config.path)? != config.before {
            return Err(Error::SourceChanged);
        }
        config.after = config.before.clone();
        let journal = super::Journal {
            schema: 2,
            agent: plan.agent.command_prefix().to_owned(),
            entry: plan.installation.entry.clone(),
            old_version: plan.installed_version.clone(),
            target_version: plan.target_version.clone(),
            phase: "config_prepared".to_owned(),
            config: Some(config),
            channel: super::channel_name(plan.installation.channel)?.to_owned(),
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
        super::save_journal(&root.join("claude.json"), &journal)?;
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
    let original = tree::snapshot(&owner.package)?;
    let id = Uuid::new_v4();
    let parent = owner.package.parent().ok_or(Error::SourceChanged)?;
    let stage = parent.join(format!(".infinishell-npm-{id}"));
    let backup = parent.join(format!(".infinishell-npm-old-{id}"));
    let config = plan.config.as_ref().ok_or(Error::UnsupportedSource)?;
    let claude_policy = super::snapshot_claude_scope(
        config,
        Uuid::new_v4(),
        super::claude_metadata_path(config)?,
        &plan.target_version,
    )?;
    let mut journal = Journal {
        schema: 1,
        id,
        owner,
        old_version: plan.installed_version.clone(),
        target_version: plan.target_version.clone(),
        stage,
        backup,
        original,
        prepared: None,
        probes: Vec::new(),
        config: plan.config.clone(),
        claude_policy,
        phase: Phase::Allocating,
        intent: plan.intent.clone(),
    };
    unchanged(&journal, false)?;
    let wrapper_metadata = download(
        "https://registry.npmjs.org/@anthropic-ai/claude-code/2.1.280",
        root,
        1024 * 1024,
    )
    .await?;
    let platform_metadata = download(
        "https://registry.npmjs.org/@anthropic-ai/claude-code-win32-x64/2.1.280",
        root,
        1024 * 1024,
    )
    .await?;
    let wrapper_bytes = read_limited(wrapper_metadata.path(), 1024 * 1024)?;
    let platform_bytes = read_limited(platform_metadata.path(), 1024 * 1024)?;
    for (bytes, integrity) in [
        (&wrapper_bytes, contract::WRAPPER_INTEGRITY),
        (&platform_bytes, contract::PLATFORM_INTEGRITY),
    ] {
        let metadata: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| Error::InvalidRelease)?;
        if metadata["dist"]["integrity"] != integrity {
            return Err(Error::InvalidRelease);
        }
    }
    let release = NpmRelease::from_metadata(
        CLIAgent::Claude,
        contract::VERSION,
        "win32-x64",
        &wrapper_bytes,
        &platform_bytes,
    )?;
    let mut wrapper = download(&release.wrapper.tarball_url, root, 512 * 1024 * 1024).await?;
    let mut platform = download(&release.platform.tarball_url, root, 512 * 1024 * 1024).await?;
    let wrapper_archive = release.wrapper.verify_archive(wrapper.as_file_mut())?;
    let platform_archive = release.platform.verify_archive(platform.as_file_mut())?;
    release.verify_entries(&wrapper_archive, &platform_archive)?;
    verify_archives(&wrapper_archive, &platform_archive)?;
    journal.owner.verify()?;
    save(root, &journal)?;
    let result = async {
        let _parents = tree::parents(&journal.stage)?;
        fs::create_dir(&journal.stage).map_err(|_| Error::PersistenceFailed)?;
        tree::copy_security(&journal.owner.package, &journal.stage)?;
        let mut directories = BTreeSet::from([PathBuf::new()]);
        for (prefix, artifact, input) in [
            (PathBuf::new(), &release.wrapper, wrapper.as_file_mut()),
            (
                PathBuf::from(contract::DEPENDENCY),
                &release.platform,
                platform.as_file_mut(),
            ),
        ] {
            artifact.extract_verified(input, |path, entry, contents| {
                let relative = prefix.join(path);
                tree::safe_relative(&relative)?;
                if relative == Path::new(contract::PUBLIC) {
                    // 官方占位文件已在归档检查中固定；下方只复制已审核的 native。
                    let length = std::io::copy(contents, &mut std::io::sink())
                        .map_err(|_| Error::InvalidRelease)?;
                    if length != entry.length {
                        return Err(Error::InvalidRelease);
                    }
                    return Ok(());
                }
                let mut directory = PathBuf::new();
                for component in relative.parent().ok_or(Error::InvalidRelease)?.components() {
                    directory.push(component);
                    if directories.insert(directory.clone()) {
                        fs::create_dir(journal.stage.join(&directory))
                            .map_err(|_| Error::PersistenceFailed)?;
                    }
                }
                let path = journal.stage.join(&relative);
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|_| Error::PersistenceFailed)?;
                let bytes =
                    std::io::copy(contents, &mut file).map_err(|_| Error::PersistenceFailed)?;
                file.sync_all().map_err(|_| Error::PersistenceFailed)?;
                drop(file);
                if bytes != entry.length || stamp(&path)?.digest != entry.sha256 {
                    return Err(Error::InvalidRelease);
                }
                if journal
                    .original
                    .members
                    .get(&relative)
                    .is_some_and(|file| !file.directory)
                {
                    tree::copy_security(&journal.owner.package.join(&relative), &path)?;
                }
                Ok(())
            })?;
        }
        let public = journal.stage.join(contract::PUBLIC);
        let native = journal.stage.join(contract::NATIVE);
        fs::create_dir_all(public.parent().ok_or(Error::InvalidRelease)?)
            .map_err(|_| Error::PersistenceFailed)?;
        directories.insert(PathBuf::from("bin"));
        // 私有新树用独立文件，旧安装的内部硬链接仍按完整别名集合核验与恢复。
        let mut source = OpenOptions::new()
            .read(true)
            .open(&native)
            .map_err(|_| Error::SourceChanged)?;
        let mut destination = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&public)
            .map_err(|_| Error::PersistenceFailed)?;
        std::io::copy(&mut source, &mut destination).map_err(|_| Error::PersistenceFailed)?;
        destination
            .sync_all()
            .map_err(|_| Error::PersistenceFailed)?;
        drop(destination);
        if stamp(&public)?.digest != stamp(&native)?.digest {
            return Err(Error::InvalidRelease);
        }
        tree::copy_security(&journal.owner.package.join(contract::PUBLIC), &public)?;
        for relative in &directories {
            if !relative.as_os_str().is_empty()
                && journal
                    .original
                    .members
                    .get(relative)
                    .is_some_and(|file| file.directory)
            {
                tree::copy_security(
                    &journal.owner.package.join(relative),
                    &journal.stage.join(relative),
                )?;
            }
        }
        let prepared = tree::snapshot(&journal.stage)?;
        verify_candidate(&prepared)?;
        journal.prepared = Some(prepared);
        journal.phase = Phase::Prepared;
        save(root, &journal)?;
        probe(root, &mut journal, "cmd").await?;
        probe(root, &mut journal, "powershell").await?;
        verified_exit(root, &journal, true)?;
        journal.owner.verify()?;
        unchanged(&journal, false)?;
        let _images = tree::freeze_images(&journal.owner.package, &journal.original)?;
        if tree::snapshot(&journal.stage)?
            != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
        {
            return Err(Error::SourceChanged);
        }
        tree::rename(&journal.owner.package, &journal.backup, &journal.original)?;
        journal.phase = Phase::OldMoved;
        save(root, &journal)?;
        tree::rename(
            &journal.stage,
            &journal.owner.package,
            journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?,
        )?;
        journal.phase = Phase::Published;
        save(root, &journal)?;
        drop(_images);
        if let Some(progress) = progress {
            progress.enter().await;
        }
        finish_publish(root, &mut journal)?;
        Ok(contract::VERSION.to_owned())
    }
    .await;
    if result.is_err()
        && recover(CLIAgent::Claude, &plan.installation.entry, root)?
            == Some(contract::VERSION.to_owned())
    {
        return Ok(contract::VERSION.to_owned());
    }
    result
}

fn finish_publish(root: &Path, journal: &mut Journal) -> Result<(), Error> {
    verified_exit(root, journal, true)?;
    journal.owner.verify()?;
    if tree::snapshot(&journal.owner.package)?
        != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    if journal.phase != Phase::ConfigPublishing && journal.phase != Phase::Committed {
        unchanged(journal, false)?;
        if let Some(config) = journal.config.as_mut() {
            config.after = config.before.clone();
            super::plan_config_publish(config, true)?;
        }
        journal.phase = Phase::ConfigPublishing;
        save(root, journal)?;
    }
    if let Some(config) = &journal.config {
        super::publish_config(config, true)?;
    }
    unchanged(journal, true)?;
    journal.phase = Phase::Committed;
    save(root, journal)?;
    tree::remove_matching(&journal.backup, &journal.original)?;
    if let Some(config) = &journal.config {
        super::cleanup_config_restore(config)?;
    }
    match fs::remove_file(super::failure_path(root, CLIAgent::Claude)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    fs::remove_file(journal_path(root)).map_err(|_| Error::PersistenceFailed)?;
    super::sync_config_directory(root)
}

pub(super) fn recover(agent: CLIAgent, entry: &Path, root: &Path) -> Result<Option<String>, Error> {
    if agent != CLIAgent::Claude
        || !journal_path(root)
            .try_exists()
            .map_err(|_| Error::RecoveryRequired)?
    {
        return Ok(None);
    }
    let mut journal: Journal =
        serde_json::from_slice(&read_limited(&journal_path(root), 12 * MAX_CONFIG)?)
            .map_err(|_| Error::RecoveryRequired)?;
    let parent = journal
        .owner
        .package
        .parent()
        .ok_or(Error::RecoveryRequired)?;
    if journal.schema != 1
        || journal.id.is_nil()
        || journal.target_version != contract::VERSION
        || !matches!(journal.old_version.as_str(), "2.1.278" | "2.1.280")
        || journal.owner.entry != entry
        || journal.stage != parent.join(format!(".infinishell-npm-{}", journal.id))
        || journal.backup != parent.join(format!(".infinishell-npm-old-{}", journal.id))
    {
        return Err(Error::RecoveryRequired);
    }
    journal.owner.verify()?;
    verified_exit(root, &journal, false)?;
    if let Some(prepared) = &journal.prepared {
        verify_candidate(prepared)?;
    }
    if matches!(journal.phase, Phase::ConfigPublishing | Phase::Committed) {
        finish_publish(root, &mut journal)?;
        return Ok(Some(contract::VERSION.to_owned()));
    }
    unchanged(&journal, false)?;
    let target_exists = journal
        .owner
        .package
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?;
    if !target_exists {
        tree::rename(&journal.backup, &journal.owner.package, &journal.original)?;
    } else if tree::snapshot(&journal.owner.package)? != journal.original {
        let prepared = journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?;
        if tree::snapshot(&journal.backup)? != journal.original {
            return Err(Error::RecoveryRequired);
        }
        let _images = tree::freeze_images(&journal.owner.package, prepared)?;
        tree::rename(&journal.owner.package, &journal.stage, prepared)?;
        tree::rename(&journal.backup, &journal.owner.package, &journal.original)?;
    }
    if tree::snapshot(&journal.owner.package)? != journal.original {
        return Err(Error::RecoveryRequired);
    }
    if let Some(prepared) = &journal.prepared {
        tree::remove_matching(&journal.stage, prepared)?;
    } else if journal
        .stage
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        // 部分归档还没有完整身份清单，不删除；保存独立引用后解除当前事务锁。
        let retained = root.join(format!("claude-npm-windows-retained-{}.json", journal.id));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(retained)
            .map_err(|_| Error::RecoveryRequired)?;
        file.write_all(&serde_json::to_vec(&journal).map_err(|_| Error::PersistenceFailed)?)
            .map_err(|_| Error::PersistenceFailed)?;
        file.sync_all().map_err(|_| Error::PersistenceFailed)?;
    }
    super::save_failure_with_intent(
        root,
        CLIAgent::Claude,
        entry,
        contract::VERSION,
        Some(journal.intent),
    )?;
    fs::remove_file(journal_path(root)).map_err(|_| Error::PersistenceFailed)?;
    super::sync_config_directory(root)?;
    Ok(None)
}
