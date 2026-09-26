//! 固定 Grok WinGet x64 用户级 portable 的两文件、公共链接与 ARP 事务。
//! 单文件下载的官方布局不建 SQLite；不把 ZIP 索引或 npm 映像当作同一来源。

use std::fs::{self, File, OpenOptions};
use std::io::{Seek as _, Write as _};
use std::path::{Path, PathBuf};

use futures::{AsyncReadExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY};
use winreg::transaction::Transaction;

use super::winget::UNINSTALL;
use super::winget_codex_dependencies::{self as registry, Values, path_key, text};
use super::winget_grok_contract as contract;
use super::winget_grok_dependencies::Runtime;
use super::winget_grok_link as link;
use super::winget_grok_tree as tree;
use super::{
    ArtifactRole, BoundInvocation, CLIAgent, Channel, Error, Installation, MAX_CONFIG, MAX_OUTPUT,
    Source, UPDATE_TIMEOUT, UpdatePlan, VerificationProgress, absolute_env, managed_process,
    read_limited,
};

fn native(version: &str) -> Result<&'static str, Error> {
    contract::native(version).ok_or(Error::InvalidRelease)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    entry: PathBuf,
    root: PathBuf,
    prefix: PathBuf,
    parent: tree::Identity,
    public: tree::Link,
    registration: Values,
    version: String,
    runtime: Runtime,
}

fn put(values: &mut Values, name: &str, text: &str) -> Result<(), Error> {
    let value = values.get_mut(name).ok_or(Error::SourceChanged)?;
    if value.0 != 1 {
        return Err(Error::UnsupportedSource);
    }
    value.1 = text
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    Ok(())
}

fn expected_values(owner: &Owner, version: &str) -> Result<Values, Error> {
    if version == owner.version {
        return Ok(owner.registration.clone());
    }
    if owner.version != contract::OLD_VERSION || version != contract::VERSION {
        return Err(Error::RecoveryRequired);
    }
    let mut values = owner.registration.clone();
    put(&mut values, "DisplayVersion", version)?;
    put(
        &mut values,
        "SHA256",
        &contract::image(version)
            .ok_or(Error::InvalidRelease)?
            .1
            .to_ascii_uppercase(),
    )?;
    // 沿用安装者使用的绝对 DOS 路径形式，不将内核前缀写回用户可见 ARP。
    let original = PathBuf::from(text(
        &registry::registry(contract::PRODUCT)?,
        "InstallLocation",
    )?);
    if path_key(&original)? != path_key(&owner.root)? {
        return Err(Error::SourceChanged);
    }
    put(
        &mut values,
        "TargetFullPath",
        original
            .join(native(version)?)
            .to_str()
            .ok_or(Error::SourceChanged)?,
    )?;
    Ok(values)
}

impl Owner {
    fn capture(entry: &Path, installed: &str) -> Result<Self, Error> {
        native(installed)?;
        let prefix = absolute_env("LOCALAPPDATA")
            .ok_or(Error::UnsupportedSource)?
            .join("Microsoft/WinGet")
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?;
        let key = registry::registry(contract::PRODUCT)?;
        let root = PathBuf::from(text(&key, "InstallLocation")?)
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?;
        let target = PathBuf::from(text(&key, "TargetFullPath")?);
        let public = tree::Link::capture(&prefix.join("Links").join(contract::ALIAS))?;
        if text(&key, "WinGetPackageIdentifier")? != contract::PACKAGE
            || text(&key, "WinGetSourceIdentifier")? != contract::SOURCE
            || text(&key, "WinGetInstallerType")? != "portable"
            || text(&key, "DisplayVersion")? != installed
            || !text(&key, "SHA256")?
                .eq_ignore_ascii_case(contract::image(installed).ok_or(Error::InvalidRelease)?.1)
            || key.get_value::<u32, _>("InstallDirectoryAddedToPath").ok() != Some(0)
            || path_key(&root)? != path_key(&prefix.join("Packages").join(contract::PRODUCT))?
            || path_key(&target)? != path_key(&root.join(native(installed)?))?
            || path_key(&public.target)? != path_key(&target)?
            || path_key(Path::new(&text(&key, "SymlinkFullPath")?))? != path_key(&public.path)?
        {
            return Err(Error::UnsupportedSource);
        }
        if ![public.path.clone(), root.join(contract::ALIAS), target]
            .iter()
            .any(|allowed| path_key(allowed).ok() == path_key(entry).ok())
        {
            return Err(Error::UnsupportedSource);
        }
        let owner = Self {
            entry: entry.to_owned(),
            parent: tree::path_identity(root.parent().ok_or(Error::SourceChanged)?)?,
            root,
            prefix,
            public,
            registration: registry::values(&key)?,
            version: installed.to_owned(),
            runtime: Runtime::capture()?,
        };
        owner.verify(installed)?;
        verify_tree(&tree::snapshot(&owner.root)?, installed)?;
        Ok(owner)
    }

    fn verify(&self, version: &str) -> Result<(), Error> {
        let prefix = absolute_env("LOCALAPPDATA")
            .ok_or(Error::UnsupportedSource)?
            .join("Microsoft/WinGet")
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?;
        if self.prefix != prefix
            || self.root != prefix.join("Packages").join(contract::PRODUCT)
            || tree::path_identity(self.root.parent().ok_or(Error::SourceChanged)?)? != self.parent
            || registry::values(&registry::registry(contract::PRODUCT)?)?
                != expected_values(self, version)?
            || self.public.path != prefix.join("Links").join(contract::ALIAS)
            || path_key(&self.public.target)? != path_key(&self.root.join(native(&self.version)?))?
        {
            return Err(Error::SourceChanged);
        }
        self.runtime.verify()
    }

    fn register(&self) -> Result<(), Error> {
        self.verify(&self.version)?;
        let target = expected_values(self, contract::VERSION)?;
        let transaction = Transaction::new().map_err(|_| Error::RecoveryRequired)?;
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_transacted_with_flags(
                format!(r"{UNINSTALL}\{}", contract::PRODUCT),
                &transaction,
                KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY,
            )
            .map_err(|_| Error::SourceChanged)?;
        if registry::values(&key)? != self.registration {
            return Err(Error::SourceChanged);
        }
        for name in ["DisplayVersion", "SHA256", "TargetFullPath"] {
            let (_, bytes) = target.get(name).ok_or(Error::RecoveryRequired)?;
            key.set_raw_value(
                name,
                &winreg::RegValue {
                    bytes: bytes.clone(),
                    vtype: winreg::enums::REG_SZ,
                },
            )
            .map_err(|_| Error::PersistenceFailed)?;
        }
        if registry::values(&key)? != target {
            return Err(Error::SourceChanged);
        }
        drop(key);
        transaction.commit().map_err(|_| Error::SourceChanged)?;
        self.verify(contract::VERSION)
    }
}

pub(super) fn supports(agent: CLIAgent, target: &str) -> Result<(), Error> {
    if agent != CLIAgent::Grok {
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

pub(super) fn discover(
    agent: CLIAgent,
    installation: &Installation,
    installed: &str,
    channel: Channel,
) -> Result<Option<Installation>, Error> {
    if agent != CLIAgent::Grok || !cfg!(target_arch = "x86_64") {
        return Ok(None);
    }
    let prefix = absolute_env("LOCALAPPDATA")
        .ok_or(Error::UnsupportedSource)?
        .join("Microsoft/WinGet");
    if !path_key(&installation.entry)?.starts_with(&format!("{}\\", path_key(&prefix)?)) {
        return Ok(None);
    }
    let owner = Owner::capture(&installation.entry, installed)?;
    super::verify_installation_identity(installation)?;
    let mut result = installation.clone();
    result.source = Source::WinGet;
    result.channel = Channel::Stable;
    result.invocation = Some(BoundInvocation::manual_only(
        [
            (ArtifactRole::Program, owner.root.join(native(installed)?)),
            (ArtifactRole::InstallRoot, owner.root),
        ],
        ArtifactRole::Program,
        Vec::new(),
        "Grok WinGet 固定 portable 由宿主事务发布",
    ));
    result.error = if matches!(channel, Channel::FollowInstallation | Channel::Stable) {
        None
    } else {
        Some(Error::ChannelMismatch)
    };
    result.config = None;
    Ok(Some(result))
}

fn verify_tree(snapshot: &tree::Snapshot, version: &str) -> Result<(), Error> {
    let (length, sha) = contract::image(version).ok_or(Error::InvalidRelease)?;
    let digest = super::brew::decode_sha256(sha)?;
    let expected = [
        PathBuf::new(),
        native(version)?.into(),
        contract::ALIAS.into(),
    ];
    if snapshot.members.len() != expected.len() {
        return Err(Error::SourceChanged);
    }
    for path in expected {
        let entry = snapshot.members.get(&path).ok_or(Error::SourceChanged)?;
        if path.as_os_str().is_empty() {
            if !entry.directory {
                return Err(Error::SourceChanged);
            }
        } else if entry.directory || entry.length != length || entry.digest != digest {
            return Err(Error::SourceChanged);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Phase {
    Allocating,
    Prepared,
    OldMoved,
    Published,
    Registering,
    Committed,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    generation: Uuid,
    input: managed_process::WindowsGrokWingetProbeInputs,
    digest: String,
    observed: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    id: Uuid,
    owner: Owner,
    stage: PathBuf,
    backup: PathBuf,
    link_stage: PathBuf,
    link_backup: PathBuf,
    prepared_link: Option<tree::Link>,
    original: tree::Snapshot,
    prepared: Option<tree::Snapshot>,
    probe: Option<Probe>,
    phase: Phase,
    intent: String,
}
fn journal_path(root: &Path) -> PathBuf {
    root.join("grok-winget-v1.json")
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
fn digest(
    input: &managed_process::WindowsGrokWingetProbeInputs,
    id: Uuid,
) -> Result<String, Error> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(input, id)).map_err(|_| Error::PersistenceFailed)?)
    ))
}
async fn probe(root: &Path, journal: &mut Journal) -> Result<(), Error> {
    let input = managed_process::capture_windows_grok_winget_probe(
        &journal.stage,
        &journal.owner.prefix,
        &journal.owner.runtime.files,
    )
    .map_err(|_| Error::SourceChanged)?;
    let digest = digest(&input, journal.id)?;
    let binding =
        managed_process::PreparedLaunchBinding::grok_windows_winget_version_probe(digest.clone())
            .map_err(|_| Error::RecoveryRequired)?;
    let generation = Uuid::new_v4();
    journal.probe = Some(Probe {
        generation,
        input: input.clone(),
        digest,
        observed: None,
    });
    save(root, journal)?;
    let mut child =
        managed_process::spawn_bound_windows_grok_winget_probe(root, generation, &input, &binding)
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
        .probe
        .as_mut()
        .ok_or(Error::RecoveryRequired)?
        .observed = Some(version);
    save(root, journal)
}
fn verified_exit(root: &Path, journal: &Journal, success: bool) -> Result<(), Error> {
    let Some(probe) = &journal.probe else {
        return if success {
            Err(Error::RecoveryRequired)
        } else {
            Ok(())
        };
    };
    if probe.digest != digest(&probe.input, journal.id)?
        || probe.input.stage() != journal.stage
        || probe.input.prefix() != journal.owner.prefix
    {
        return Err(Error::RecoveryRequired);
    }
    managed_process::validate_windows_grok_winget_probe(&probe.input)
        .map_err(|_| Error::RecoveryRequired)?;
    let binding = managed_process::PreparedLaunchBinding::grok_windows_winget_version_probe(
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
    let receipt = managed_process::confirmed_exit_with_binding(root, probe.generation, &binding)
        .map_err(|_| Error::RecoveryRequired)?
        .ok_or(Error::RecoveryRequired)?;
    journal.owner.runtime.verify()?;
    if !receipt.cleanup_confirmed
        || success
            && (receipt.exit_code != Some(0)
                || probe.observed.as_deref() != Some(contract::VERSION))
    {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

async fn download(
    url: &str,
    length: u64,
    expected: &str,
    root: &Path,
) -> Result<NamedTempFile, Error> {
    let response = http_client::Client::new()
        .get(url)
        .timeout(UPDATE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() || response.url().as_str() != url {
        return Err(Error::Network);
    }
    let mut file = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    let mut count = 0u64;
    let mut digest = Sha256::new();
    while let Some(bytes) = stream.next().await {
        let bytes = bytes.map_err(|_| Error::Network)?;
        count = count
            .checked_add(bytes.len() as u64)
            .ok_or(Error::InvalidRelease)?;
        if count > length {
            return Err(Error::InvalidRelease);
        }
        digest.update(&bytes);
        file.write_all(&bytes)
            .map_err(|_| Error::PersistenceFailed)?;
    }
    if count != length || format!("{:x}", digest.finalize()) != expected {
        return Err(Error::InvalidRelease);
    }
    file.as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    Ok(file)
}

fn materialize(image: &mut File, journal: &Journal) -> Result<(), Error> {
    let (length, digest) = contract::image(contract::VERSION).ok_or(Error::InvalidRelease)?;
    for (target, original) in [
        (native(contract::VERSION)?, native(&journal.owner.version)?),
        (contract::ALIAS, contract::ALIAS),
    ] {
        let destination = journal.stage.join(target);
        image.rewind().map_err(|_| Error::PersistenceFailed)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|_| Error::PersistenceFailed)?;
        if std::io::copy(image, &mut file).map_err(|_| Error::PersistenceFailed)? != length {
            return Err(Error::InvalidRelease);
        }
        file.sync_all().map_err(|_| Error::PersistenceFailed)?;
        drop(file);
        if super::stamp(&destination)?.digest != super::brew::decode_sha256(digest)? {
            return Err(Error::SourceChanged);
        }
        // 复制回退是原生 WinGet 的合法别名实现，旧 ACL 和 MotW 均逐入口保留。
        tree::copy_zone(
            journal
                .original
                .members
                .get(Path::new(original))
                .ok_or(Error::SourceChanged)?,
            &destination,
        )?;
        tree::copy_security(&journal.owner.root.join(original), &destination)?;
    }
    Ok(())
}

fn publish_link(journal: &Journal) -> Result<(), Error> {
    let original = &journal.owner.public;
    let prepared = journal
        .prepared_link
        .as_ref()
        .ok_or(Error::RecoveryRequired)?;
    if link::matches(original, &original.path)? {
        if !link::absent(&journal.link_backup)? || !link::matches(prepared, &journal.link_stage)? {
            return Err(Error::SourceChanged);
        }
        link::rename(original, &journal.link_backup)?;
    }
    if link::absent(&original.path)? && link::matches(original, &journal.link_backup)? {
        link::rename(prepared, &original.path)?;
    }
    if !link::matches(prepared, &original.path)? || !link::matches(original, &journal.link_backup)?
    {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

fn restore_link(journal: &Journal) -> Result<(), Error> {
    let original = &journal.owner.public;
    if link::matches(original, &original.path)? {
        if !link::absent(&journal.link_backup)? {
            return Err(Error::SourceChanged);
        }
        return Ok(());
    }
    let prepared = journal
        .prepared_link
        .as_ref()
        .ok_or(Error::RecoveryRequired)?;
    if link::matches(prepared, &original.path)? {
        link::rename(&link::at(prepared, &original.path), &journal.link_stage)?;
    }
    if !link::absent(&original.path)? || !link::matches(original, &journal.link_backup)? {
        return Err(Error::SourceChanged);
    }
    link::rename(&link::at(original, &journal.link_backup), &original.path)
}

pub(super) async fn execute(
    plan: &UpdatePlan,
    root: &Path,
    progress: Option<VerificationProgress>,
) -> Result<String, Error> {
    supports(plan.agent, &plan.target_version)?;
    if plan.installation.source != Source::WinGet || plan.config.is_some() {
        return Err(Error::UnsupportedSource);
    }
    if journal_path(root)
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    let owner = Owner::capture(&plan.installation.entry, &plan.installed_version)?;
    super::verify_installation_identity(&plan.installation)?;
    if !plan.requires_native_update() {
        return Ok(contract::VERSION.to_owned());
    }
    if owner.version != contract::OLD_VERSION {
        return Err(Error::InvalidRelease);
    }
    // 逐字节固定 Microsoft 清单，再固定其官方 URL 的完整单文件映像。
    let _manifest = download(contract::MANIFEST, 849, contract::MANIFEST_SHA, root).await?;
    let (length, sha) = contract::image(contract::VERSION).ok_or(Error::InvalidRelease)?;
    let mut image = download(contract::IMAGE_URL, length, sha, root).await?;
    owner.verify(&owner.version)?;
    let original = tree::snapshot(&owner.root)?;
    verify_tree(&original, &owner.version)?;
    let id = Uuid::new_v4();
    let parent = owner.root.parent().ok_or(Error::SourceChanged)?;
    let stage = parent.join(format!(".infinishell-winget-grok-{id}"));
    let backup = parent.join(format!(".infinishell-winget-grok-old-{id}"));
    let link_stage = owner
        .prefix
        .join("Links")
        .join(format!(".infinishell-grok-link-{id}"));
    let link_backup = owner
        .prefix
        .join("Links")
        .join(format!(".infinishell-grok-link-old-{id}"));
    let mut journal = Journal {
        schema: 1,
        id,
        owner,
        stage,
        backup,
        link_stage,
        link_backup,
        original,
        prepared: None,
        prepared_link: None,
        probe: None,
        phase: Phase::Allocating,
        intent: plan.intent.clone(),
    };
    save(root, &journal)?;
    let result = async {
        let _parents = tree::parents(&journal.stage)?;
        fs::create_dir(&journal.stage).map_err(|_| Error::PersistenceFailed)?;
        tree::copy_security(&journal.owner.root, &journal.stage)?;
        materialize(image.as_file_mut(), &journal)?;
        let prepared = tree::snapshot(&journal.stage)?;
        verify_tree(&prepared, contract::VERSION)?;
        journal.prepared = Some(prepared);
        save(root, &journal)?;
        journal.prepared_link = Some(link::create(
            &journal.owner.public,
            &journal.link_stage,
            &journal.owner.root.join(native(contract::VERSION)?),
        )?);
        journal.phase = Phase::Prepared;
        save(root, &journal)?;
        probe(root, &mut journal).await?;
        verified_exit(root, &journal, true)?;
        journal.owner.verify(&journal.owner.version)?;
        let _link = journal.owner.public.hold()?;
        let _images = tree::freeze_images(&journal.owner.root, &journal.original)?;
        if tree::snapshot(&journal.stage)?
            != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
        {
            return Err(Error::SourceChanged);
        }
        tree::rename(&journal.owner.root, &journal.backup, &journal.original)?;
        journal.phase = Phase::OldMoved;
        save(root, &journal)?;
        tree::rename(
            &journal.stage,
            &journal.owner.root,
            journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?,
        )?;
        journal.phase = Phase::Published;
        save(root, &journal)?;
        drop(_images);
        drop(_link);
        publish_link(&journal)?;
        journal.phase = Phase::Registering;
        save(root, &journal)?;
        if let Some(progress) = progress {
            progress.enter().await;
        }
        finish(root, &mut journal)?;
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

fn finish(root: &Path, journal: &mut Journal) -> Result<(), Error> {
    verified_exit(root, journal, true)?;
    if tree::snapshot(&journal.owner.root)?
        != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    let prepared = journal
        .prepared_link
        .as_ref()
        .ok_or(Error::RecoveryRequired)?;
    if !link::matches(prepared, &journal.owner.public.path)? {
        return Err(Error::SourceChanged);
    }
    let _link = link::at(prepared, &journal.owner.public.path).hold()?;
    let values = registry::values(&registry::registry(contract::PRODUCT)?)?;
    if values == journal.owner.registration {
        journal.owner.register()?;
    } else if values != expected_values(&journal.owner, contract::VERSION)? {
        return Err(Error::SourceChanged);
    }
    journal.owner.verify(contract::VERSION)?;
    journal.phase = Phase::Committed;
    save(root, journal)?;
    tree::remove_matching(&journal.backup, &journal.original)?;
    link::remove(&journal.owner.public, &journal.link_backup)?;
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
    let parent = journal.owner.root.parent().ok_or(Error::RecoveryRequired)?;
    let target_native = native(contract::VERSION)?;
    if journal.schema != 1
        || journal.id.is_nil()
        || journal.owner.entry != entry
        || journal.owner.version != contract::OLD_VERSION
        || journal.stage != parent.join(format!(".infinishell-winget-grok-{}", journal.id))
        || journal.backup != parent.join(format!(".infinishell-winget-grok-old-{}", journal.id))
        || journal.link_stage
            != journal
                .owner
                .prefix
                .join("Links")
                .join(format!(".infinishell-grok-link-{}", journal.id))
        || journal.link_backup
            != journal
                .owner
                .prefix
                .join("Links")
                .join(format!(".infinishell-grok-link-old-{}", journal.id))
        || journal.prepared_link.as_ref().is_some_and(|value| {
            value.path != journal.link_stage
                || value.target != journal.owner.root.join(target_native)
        })
    {
        return Err(Error::RecoveryRequired);
    }
    verify_tree(&journal.original, &journal.owner.version)?;
    if let Some(prepared) = &journal.prepared {
        verify_tree(prepared, contract::VERSION)?;
    }
    verified_exit(root, &journal, false)?;
    if matches!(journal.phase, Phase::Registering | Phase::Committed) {
        finish(root, &mut journal)?;
        return Ok(Some(contract::VERSION.to_owned()));
    }
    journal.owner.verify(&journal.owner.version)?;
    restore_link(&journal)?;
    let _link = journal.owner.public.hold()?;
    if !journal
        .owner
        .root
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        tree::rename(&journal.backup, &journal.owner.root, &journal.original)?;
    } else if tree::snapshot(&journal.owner.root)? != journal.original {
        let prepared = journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?;
        if tree::snapshot(&journal.backup)? != journal.original {
            return Err(Error::RecoveryRequired);
        }
        let _images = tree::freeze_images(&journal.owner.root, prepared)?;
        tree::rename(&journal.owner.root, &journal.stage, prepared)?;
        tree::rename(&journal.backup, &journal.owner.root, &journal.original)?;
    }
    if tree::snapshot(&journal.owner.root)? != journal.original {
        return Err(Error::RecoveryRequired);
    }
    if let Some(prepared) = &journal.prepared {
        tree::remove_matching(&journal.stage, prepared)?;
    }
    if let Some(prepared) = &journal.prepared_link {
        link::remove(prepared, &journal.link_stage)?;
    }
    if journal
        .stage
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
        || !link::absent(&journal.link_stage)?
    {
        // 身份尚未落盘的候选保留原处；只留一份可恢复引用，不推测应删除的文件。
        let retained = root.join(format!("grok-winget-retained-{}.json", journal.id));
        let bytes = serde_json::to_vec(&journal).map_err(|_| Error::PersistenceFailed)?;
        if retained.try_exists().map_err(|_| Error::RecoveryRequired)? {
            if read_limited(&retained, 24 * MAX_CONFIG)? != bytes {
                return Err(Error::SourceChanged);
            }
        } else {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&retained)
                .map_err(|_| Error::PersistenceFailed)?;
            file.write_all(&bytes)
                .map_err(|_| Error::PersistenceFailed)?;
            file.sync_all().map_err(|_| Error::PersistenceFailed)?;
        }
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
