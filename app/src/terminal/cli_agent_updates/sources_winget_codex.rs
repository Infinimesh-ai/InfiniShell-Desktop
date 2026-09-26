//! Codex WinGet x64 多文件 portable 的完整候选、目录切换与 ARP 事务恢复。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Seek as _, Write as _};
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
use super::winget_codex_contract as contract;
use super::winget_codex_dependencies::{self as dependencies, Values, path_key, text};
use super::winget_codex_index as index;
use super::winget_codex_tree as tree;
use super::{
    ArtifactRole, BoundInvocation, CLIAgent, Channel, Error, Installation, MAX_CONFIG, MAX_OUTPUT,
    Source, UPDATE_TIMEOUT, UpdatePlan, VerificationProgress, absolute_env, managed_process,
    read_limited,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    entry: PathBuf,
    root: PathBuf,
    prefix: PathBuf,
    parent: tree::Identity,
    links: Vec<tree::Link>,
    registration: Values,
    version: String,
    dependencies: dependencies::Dependencies,
}

fn expected_values(owner: &Owner, version: &str) -> Result<Values, Error> {
    let mut expected = owner.registration.clone();
    let value = expected
        .get_mut("DisplayVersion")
        .ok_or(Error::SourceChanged)?;
    if value.0 != 1 {
        return Err(Error::UnsupportedSource);
    }
    value.1 = version
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    Ok(expected)
}

impl Owner {
    fn capture(entry: &Path, installed: &str) -> Result<Self, Error> {
        if !matches!(installed, contract::OLD_VERSION | contract::VERSION) {
            return Err(Error::InvalidRelease);
        }
        let prefix = absolute_env("LOCALAPPDATA")
            .ok_or(Error::UnsupportedSource)?
            .join("Microsoft/WinGet")
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?;
        let key = dependencies::registry(contract::PRODUCT)?;
        let root = PathBuf::from(text(&key, "InstallLocation")?)
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?;
        if text(&key, "WinGetPackageIdentifier")? != contract::PACKAGE
            || text(&key, "WinGetSourceIdentifier")? != contract::SOURCE
            || text(&key, "WinGetInstallerType")? != "portable"
            || text(&key, "DisplayVersion")? != installed
            || path_key(&root)? != path_key(&prefix.join("Packages").join(contract::PRODUCT))?
        {
            return Err(Error::SourceChanged);
        }
        let allowed = [
            prefix.join("Links/codex.exe"),
            root.join(contract::NATIVE),
            root.join("codex.exe"),
        ];
        if !allowed
            .iter()
            .any(|allowed| path_key(allowed).ok() == path_key(entry).ok())
        {
            return Err(Error::UnsupportedSource);
        }
        let links = contract::LINKS
            .into_iter()
            .map(|(name, _)| tree::Link::capture(&prefix.join("Links").join(name)))
            .collect::<Result<_, _>>()?;
        let owner = Self {
            entry: entry.to_owned(),
            parent: tree::path_identity(root.parent().ok_or(Error::SourceChanged)?)?,
            root,
            prefix,
            links,
            registration: dependencies::values(&key)?,
            version: installed.to_owned(),
            dependencies: dependencies::Dependencies::capture()?,
        };
        owner.verify(installed)?;
        let snapshot = tree::snapshot(&owner.root)?;
        verify_tree(&snapshot, installed, false)?;
        verify_index(
            &owner.root,
            &owner,
            installed,
            snapshot.members.contains_key(Path::new("codex.exe")),
        )?;
        Ok(owner)
    }

    fn verify(&self, version: &str) -> Result<(), Error> {
        let prefix = absolute_env("LOCALAPPDATA")
            .ok_or(Error::UnsupportedSource)?
            .join("Microsoft/WinGet")
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?;
        if prefix != self.prefix
            || self.root != prefix.join("Packages").join(contract::PRODUCT)
            || tree::path_identity(self.root.parent().ok_or(Error::SourceChanged)?)? != self.parent
            || self.links.len() != 3
            || dependencies::values(&dependencies::registry(contract::PRODUCT)?)?
                != expected_values(self, version)?
        {
            return Err(Error::SourceChanged);
        }
        for (link, (name, target)) in self.links.iter().zip(contract::LINKS) {
            if link.path != self.prefix.join("Links").join(name)
                || path_key(&link.target)? != path_key(&self.root.join(target))?
                || tree::Link::capture(&link.path)? != *link
            {
                return Err(Error::SourceChanged);
            }
        }
        self.dependencies.verify()
    }

    fn register(&self, before: &str, after: &str) -> Result<(), Error> {
        self.verify(before)?;
        let transaction = Transaction::new().map_err(|_| Error::RecoveryRequired)?;
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_transacted_with_flags(
                format!(r"{UNINSTALL}\{}", contract::PRODUCT),
                &transaction,
                KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY,
            )
            .map_err(|_| Error::SourceChanged)?;
        if dependencies::values(&key)? != expected_values(self, before)? {
            return Err(Error::SourceChanged);
        }
        key.set_value("DisplayVersion", &after)
            .map_err(|_| Error::PersistenceFailed)?;
        if dependencies::values(&key)? != expected_values(self, after)? {
            return Err(Error::SourceChanged);
        }
        drop(key);
        transaction.commit().map_err(|_| Error::SourceChanged)?;
        self.verify(after)
    }
}

pub(super) fn supports(agent: CLIAgent, target: &str) -> Result<(), Error> {
    if agent != CLIAgent::Codex {
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
    if agent != CLIAgent::Codex || !cfg!(target_arch = "x86_64") {
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
    result.channel = Channel::Latest;
    result.invocation = Some(BoundInvocation::manual_only(
        [
            (ArtifactRole::Program, owner.root.join(contract::NATIVE)),
            (ArtifactRole::InstallRoot, owner.root),
        ],
        ArtifactRole::Program,
        Vec::new(),
        "Codex WinGet 多文件包由宿主事务发布",
    ));
    result.error = if matches!(channel, Channel::FollowInstallation | Channel::Latest) {
        None
    } else {
        Some(Error::ChannelMismatch)
    };
    Ok(Some(result))
}

fn verify_tree(snapshot: &tree::Snapshot, version: &str, require_alias: bool) -> Result<(), Error> {
    let mut files = contract::files(version).ok_or(Error::InvalidRelease)?;
    let mut directories = contract::directories();
    let alias = snapshot.members.contains_key(Path::new("codex.exe"));
    if require_alias && !alias {
        return Err(Error::SourceChanged);
    }
    if alias {
        files.insert(
            "codex.exe".into(),
            *files
                .get(Path::new(contract::NATIVE))
                .ok_or(Error::InvalidRelease)?,
        );
    }
    let database = snapshot
        .members
        .get(Path::new(contract::INDEX))
        .ok_or(Error::SourceChanged)?;
    if database.directory || database.length == 0 || database.length > 1024 * 1024 {
        return Err(Error::SourceChanged);
    }
    for (path, identity) in &snapshot.members {
        if path == Path::new(contract::INDEX) {
            continue;
        }
        if identity.directory {
            if !directories.remove(path) {
                return Err(Error::SourceChanged);
            }
        } else {
            let (length, digest) = files.remove(path).ok_or(Error::SourceChanged)?;
            if identity.length != length || identity.digest != super::brew::decode_sha256(digest)? {
                return Err(Error::SourceChanged);
            }
        }
    }
    if !files.is_empty() || !directories.is_empty() {
        return Err(Error::SourceChanged);
    }
    Ok(())
}

fn index_entries(owner: &Owner, version: &str, alias: bool) -> Result<Vec<index::Entry>, Error> {
    let mut entries = Vec::new();
    for (path, (_, sha)) in contract::files(version).ok_or(Error::InvalidRelease)? {
        if path.components().count() == 1 {
            entries.push(index::Entry {
                path: owner.root.join(path),
                kind: 1,
                sha: sha.to_owned(),
                target: PathBuf::new(),
            });
        }
    }
    for path in contract::directories() {
        if path.components().count() == 1 {
            entries.push(index::Entry {
                path: owner.root.join(path),
                kind: 2,
                sha: String::new(),
                target: PathBuf::new(),
            });
        }
    }
    if alias {
        let files = contract::files(version).ok_or(Error::InvalidRelease)?;
        entries.push(index::Entry {
            path: owner.root.join("codex.exe"),
            kind: 4,
            sha: files
                .get(Path::new(contract::NATIVE))
                .ok_or(Error::InvalidRelease)?
                .1
                .to_owned(),
            target: PathBuf::new(),
        });
    }
    for (name, target) in contract::LINKS {
        entries.push(index::Entry {
            path: owner.prefix.join("Links").join(name),
            kind: 3,
            sha: String::new(),
            target: owner.root.join(target),
        });
    }
    Ok(entries)
}

fn verify_index(directory: &Path, owner: &Owner, version: &str, alias: bool) -> Result<(), Error> {
    let normalized =
        |entries: Vec<index::Entry>| -> Result<BTreeMap<String, (u32, String, String)>, Error> {
            let mut result = BTreeMap::new();
            for entry in entries {
                let target = if entry.target.as_os_str().is_empty() {
                    String::new()
                } else {
                    path_key(&entry.target)?
                };
                if result
                    .insert(path_key(&entry.path)?, (entry.kind, entry.sha, target))
                    .is_some()
                {
                    return Err(Error::SourceChanged);
                }
            }
            Ok(result)
        };
    if normalized(index::read(&directory.join(contract::INDEX))?)?
        != normalized(index_entries(owner, version, alias)?)?
    {
        return Err(Error::SourceChanged);
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
    input: managed_process::WindowsCodexWingetProbeInputs,
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
    original: tree::Snapshot,
    prepared: Option<tree::Snapshot>,
    probe: Option<Probe>,
    phase: Phase,
    intent: String,
}
fn journal_path(root: &Path) -> PathBuf {
    root.join("codex-winget-v1.json")
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
    input: &managed_process::WindowsCodexWingetProbeInputs,
    id: Uuid,
) -> Result<String, Error> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(input, id)).map_err(|_| Error::PersistenceFailed)?)
    ))
}
async fn probe(root: &Path, journal: &mut Journal) -> Result<(), Error> {
    let input = managed_process::capture_windows_codex_winget_probe(
        &journal.stage,
        &journal.owner.prefix,
        journal.owner.dependencies.rg.path(),
        &journal.owner.dependencies.runtime,
    )
    .map_err(|_| Error::SourceChanged)?;
    let digest = digest(&input, journal.id)?;
    let binding =
        managed_process::PreparedLaunchBinding::codex_windows_winget_version_probe(digest.clone())
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
        managed_process::spawn_bound_windows_codex_winget_probe(root, generation, &input, &binding)
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
        CLIAgent::Codex,
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
    managed_process::validate_windows_codex_winget_probe(&probe.input)
        .map_err(|_| Error::RecoveryRequired)?;
    let binding = managed_process::PreparedLaunchBinding::codex_windows_winget_version_probe(
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
    managed_process::verify_windows_codex_winget_probe_dependencies(&probe.input)
        .map_err(|_| Error::SourceChanged)?;
    if !receipt.cleanup_confirmed
        || success
            && (receipt.exit_code != Some(0)
                || probe.observed.as_deref() != Some(contract::VERSION))
    {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

async fn download(root: &Path) -> Result<NamedTempFile, Error> {
    // GitHub 固定 release 的一次 HTTPS 跳转仅允许官方资源域；最终还必须匹配独立固定摘要。
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        let url = attempt.url();
        if attempt.previous().len() <= 2
            && url.scheme() == "https"
            && url.host_str() == Some("release-assets.githubusercontent.com")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none_or(|port| port == 443)
        {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });
    let builder = http_client::proxy::current_proxy_config()
        .apply(reqwest::Client::builder().redirect(redirect));
    let client = http_client::Client::from_client_builder(builder).map_err(|_| Error::Network)?;
    let response = client
        .get(contract::URL)
        .timeout(UPDATE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() {
        return Err(Error::Network);
    }
    let mut file = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    let mut size = 0u64;
    let mut hash = Sha256::new();
    while let Some(bytes) = stream.next().await {
        let bytes = bytes.map_err(|_| Error::Network)?;
        size = size
            .checked_add(bytes.len() as u64)
            .ok_or(Error::InvalidRelease)?;
        if size > contract::ARCHIVE_SIZE {
            return Err(Error::InvalidRelease);
        }
        hash.update(&bytes);
        file.write_all(&bytes)
            .map_err(|_| Error::PersistenceFailed)?;
    }
    if size != contract::ARCHIVE_SIZE || format!("{:x}", hash.finalize()) != contract::ARCHIVE_SHA {
        return Err(Error::InvalidRelease);
    }
    file.as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    Ok(file)
}

fn extract(archive: &mut File, journal: &Journal) -> Result<(), Error> {
    archive.rewind().map_err(|_| Error::InvalidRelease)?;
    let mut zip = zip::ZipArchive::new(archive).map_err(|_| Error::InvalidRelease)?;
    let mut files = contract::files(contract::VERSION).ok_or(Error::InvalidRelease)?;
    let mut directories = contract::directories();
    directories.remove(Path::new(""));
    if zip.len() != files.len() + directories.len() {
        return Err(Error::InvalidRelease);
    }
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|_| Error::InvalidRelease)?;
        let path = entry.enclosed_name().ok_or(Error::InvalidRelease)?;
        tree::safe_relative(&path)?;
        if entry.encrypted()
            || entry.is_symlink()
            || !matches!(
                entry.compression(),
                zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
            )
        {
            return Err(Error::InvalidRelease);
        }
        let destination = journal.stage.join(&path);
        if entry.is_dir() {
            if !directories.remove(&path) || entry.size() != 0 {
                return Err(Error::InvalidRelease);
            }
            fs::create_dir(&destination).map_err(|_| Error::PersistenceFailed)?;
        } else {
            let (length, sha) = files.remove(&path).ok_or(Error::InvalidRelease)?;
            if entry.size() != length {
                return Err(Error::InvalidRelease);
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map_err(|_| Error::PersistenceFailed)?;
            let mut hash = Sha256::new();
            let mut count = 0u64;
            let mut buffer = [0; 65536];
            loop {
                let size = entry.read(&mut buffer).map_err(|_| Error::InvalidRelease)?;
                if size == 0 {
                    break;
                }
                count += size as u64;
                if count > length {
                    return Err(Error::InvalidRelease);
                }
                hash.update(&buffer[..size]);
                output
                    .write_all(&buffer[..size])
                    .map_err(|_| Error::PersistenceFailed)?;
            }
            output.sync_all().map_err(|_| Error::PersistenceFailed)?;
            drop(output);
            if count != length || format!("{:x}", hash.finalize()) != sha {
                return Err(Error::InvalidRelease);
            }
        }
        if journal.original.members.contains_key(&path) {
            tree::copy_security(&journal.owner.root.join(&path), &destination)?;
        }
    }
    if !files.is_empty() || !directories.is_empty() {
        return Err(Error::InvalidRelease);
    }
    // WinGet 的 Hardlink 记录允许复制回退；候选采用独立文件以保留两个旧入口各自的 ACL。
    let alias = journal.stage.join("codex.exe");
    let mut source =
        File::open(journal.stage.join(contract::NATIVE)).map_err(|_| Error::SourceChanged)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&alias)
        .map_err(|_| Error::PersistenceFailed)?;
    std::io::copy(&mut source, &mut output).map_err(|_| Error::PersistenceFailed)?;
    output.sync_all().map_err(|_| Error::PersistenceFailed)?;
    drop(output);
    let original_alias = if journal
        .original
        .members
        .contains_key(Path::new("codex.exe"))
    {
        "codex.exe"
    } else {
        contract::NATIVE
    };
    tree::copy_security(&journal.owner.root.join(original_alias), &alias)?;
    index::write(
        &journal.stage.join(contract::INDEX),
        &index_entries(&journal.owner, contract::VERSION, true)?,
    )?;
    tree::copy_security(
        &journal.owner.root.join(contract::INDEX),
        &journal.stage.join(contract::INDEX),
    )?;
    verify_index(&journal.stage, &journal.owner, contract::VERSION, true)
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
    let original = tree::snapshot(&owner.root)?;
    let id = Uuid::new_v4();
    let parent = owner.root.parent().ok_or(Error::SourceChanged)?;
    let stage = parent.join(format!(".infinishell-winget-{id}"));
    let backup = parent.join(format!(".infinishell-winget-old-{id}"));
    let mut journal = Journal {
        schema: 1,
        id,
        owner,
        stage,
        backup,
        original,
        prepared: None,
        probe: None,
        phase: Phase::Allocating,
        intent: plan.intent.clone(),
    };
    let mut archive = download(root).await?;
    journal.owner.verify(&journal.owner.version)?;
    save(root, &journal)?;
    let result = async {
        let _parents = tree::parents(&journal.stage)?;
        fs::create_dir(&journal.stage).map_err(|_| Error::PersistenceFailed)?;
        tree::copy_security(&journal.owner.root, &journal.stage)?;
        extract(archive.as_file_mut(), &journal)?;
        let prepared = tree::snapshot(&journal.stage)?;
        verify_tree(&prepared, contract::VERSION, true)?;
        journal.prepared = Some(prepared);
        journal.phase = Phase::Prepared;
        save(root, &journal)?;
        probe(root, &mut journal).await?;
        verified_exit(root, &journal, true)?;
        journal.owner.verify(&journal.owner.version)?;
        let _links = journal
            .owner
            .links
            .iter()
            .map(tree::Link::hold)
            .collect::<Result<Vec<_>, _>>()?;
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
        if let Some(progress) = progress {
            progress.enter().await;
        }
        finish_publish(root, &mut journal)?;
        Ok(contract::VERSION.to_owned())
    }
    .await;
    if result.is_err()
        && recover(CLIAgent::Codex, &plan.installation.entry, root)?
            == Some(contract::VERSION.to_owned())
    {
        return Ok(contract::VERSION.to_owned());
    }
    result
}

fn finish_publish(root: &Path, journal: &mut Journal) -> Result<(), Error> {
    verified_exit(root, journal, true)?;
    if tree::snapshot(&journal.owner.root)?
        != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    verify_index(&journal.owner.root, &journal.owner, contract::VERSION, true)?;
    let values = dependencies::values(&dependencies::registry(contract::PRODUCT)?)?;
    if values == expected_values(&journal.owner, &journal.owner.version)? {
        journal.owner.verify(&journal.owner.version)?;
        journal.phase = Phase::Registering;
        save(root, journal)?;
        journal
            .owner
            .register(&journal.owner.version, contract::VERSION)?;
    } else if !matches!(journal.phase, Phase::Registering | Phase::Committed)
        || values != expected_values(&journal.owner, contract::VERSION)?
    {
        return Err(Error::SourceChanged);
    }
    journal.owner.verify(contract::VERSION)?;
    journal.phase = Phase::Committed;
    save(root, journal)?;
    tree::remove_matching(&journal.backup, &journal.original)?;
    match fs::remove_file(super::failure_path(root, CLIAgent::Codex)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    fs::remove_file(journal_path(root)).map_err(|_| Error::PersistenceFailed)?;
    super::sync_config_directory(root)
}

pub(super) fn recover(agent: CLIAgent, entry: &Path, root: &Path) -> Result<Option<String>, Error> {
    if agent != CLIAgent::Codex
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
    if journal.schema != 1
        || journal.id.is_nil()
        || journal.owner.entry != entry
        || !matches!(
            journal.owner.version.as_str(),
            contract::OLD_VERSION | contract::VERSION
        )
        || journal.stage != parent.join(format!(".infinishell-winget-{}", journal.id))
        || journal.backup != parent.join(format!(".infinishell-winget-old-{}", journal.id))
    {
        return Err(Error::RecoveryRequired);
    }
    verify_tree(&journal.original, &journal.owner.version, false)?;
    verified_exit(root, &journal, false)?;
    if let Some(prepared) = &journal.prepared {
        verify_tree(prepared, contract::VERSION, true)?;
    }
    if matches!(journal.phase, Phase::Registering | Phase::Committed) {
        finish_publish(root, &mut journal)?;
        return Ok(Some(contract::VERSION.to_owned()));
    }
    journal.owner.verify(&journal.owner.version)?;
    let _links = journal
        .owner
        .links
        .iter()
        .map(tree::Link::hold)
        .collect::<Result<Vec<_>, _>>()?;
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
    verify_index(
        &journal.owner.root,
        &journal.owner,
        &journal.owner.version,
        journal
            .original
            .members
            .contains_key(Path::new("codex.exe")),
    )?;
    if let Some(prepared) = &journal.prepared {
        tree::remove_matching(&journal.stage, prepared)?;
    } else if journal
        .stage
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        // 未形成完整身份的部分候选保留原地，单独保存引用；不猜测删除范围。
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(format!("codex-winget-retained-{}.json", journal.id)))
            .map_err(|_| Error::RecoveryRequired)?;
        file.write_all(&serde_json::to_vec(&journal).map_err(|_| Error::PersistenceFailed)?)
            .map_err(|_| Error::PersistenceFailed)?;
        file.sync_all().map_err(|_| Error::PersistenceFailed)?;
    }
    super::save_failure_with_intent(
        root,
        CLIAgent::Codex,
        entry,
        contract::VERSION,
        Some(journal.intent),
    )?;
    fs::remove_file(journal_path(root)).map_err(|_| Error::PersistenceFailed)?;
    super::sync_config_directory(root)?;
    Ok(None)
}
