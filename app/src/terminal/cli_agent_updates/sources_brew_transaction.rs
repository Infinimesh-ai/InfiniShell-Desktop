//! Homebrew 单二进制 cask 事务：保留官方登记布局，目录与公共链接分别记录交换状态。
//! 不派生 brew、Ruby 或安装脚本；补全仅运行固定原生探针，未知布局和外部改动保留现场。

use std::collections::BTreeMap;
use std::ffi::{CString, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, symlink};
use std::path::{Path, PathBuf};

use futures::{AsyncReadExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;

use super::brew_completions::Completion;
use super::brew_grok_aliases::Alias;

#[cfg(target_os = "linux")]
#[path = "sources_brew_claude_linux_probes.rs"]
mod claude_linux_probes;
#[path = "sources_brew_grok_probes.rs"]
mod grok_probes;
use super::package_tree::{Directory, Identity, Snapshot};
use super::{
    ArtifactRole, CLIAgent, Error, MAX_CONFIG, MAX_OUTPUT, Stamp, UPDATE_TIMEOUT, UpdatePlan,
    VerificationProgress, brew, managed_process, read_limited, stamp,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    target: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}

fn link(path: &Path) -> Result<Link, Error> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Error::SourceChanged)?;
    if !metadata.file_type().is_symlink() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(Error::UnsupportedSource);
    }
    Ok(Link {
        target: fs::read_link(path).map_err(|_| Error::SourceChanged)?,
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        mode: metadata.mode(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Phase {
    Preparing,
    Prepared,
    ExchangeIntent,
    Committed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    generation: Uuid,
    program: PathBuf,
    digest: String,
    version: Option<String>,
    arguments: Vec<OsString>,
    completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_sha256: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    id: Uuid,
    agent: String,
    prefix: PathBuf,
    prefix_identity: Identity,
    parent_identity: Identity,
    bin_identity: Identity,
    token: String,
    entry: PathBuf,
    manager: Stamp,
    old_version: String,
    target_version: String,
    intent: String,
    original: Snapshot,
    prepared: Option<Snapshot>,
    original_link: Link,
    prepared_link: Option<Link>,
    phase: Phase,
    probe: Option<Probe>,
    extra_probes: Vec<Probe>,
    completions: Vec<Completion>,
    #[serde(default)]
    aliases: Vec<Alias>,
    #[serde(default)]
    entry_probes: Vec<Probe>,
}

impl Journal {
    fn cli(&self) -> Result<CLIAgent, Error> {
        match self.agent.as_str() {
            "claude" => Ok(CLIAgent::Claude),
            "codex" => Ok(CLIAgent::Codex),
            "grok" => Ok(CLIAgent::Grok),
            _ => Err(Error::RecoveryRequired),
        }
    }
    fn parent(&self) -> PathBuf {
        self.prefix.join("Caskroom")
    }
    fn stage_name(&self) -> OsString {
        format!(".infinishell-brew-{}", self.id).into()
    }
    fn link_stage(&self) -> PathBuf {
        self.prefix
            .join("bin")
            .join(format!(".infinishell-brew-link-{}", self.id))
    }
    fn verify_external(&self) -> Result<Directory, Error> {
        if Directory::open(&self.prefix)?.identity()? != self.prefix_identity
            || Directory::open(&self.prefix.join("bin"))?.identity()? != self.bin_identity
            || stamp(&self.manager.canonical)? != self.manager
        {
            return Err(Error::SourceChanged);
        }
        let parent = Directory::open(&self.parent())?;
        if parent.identity()? != self.parent_identity {
            return Err(Error::SourceChanged);
        }
        Ok(parent)
    }
}

fn is_linux_claude(agent: CLIAgent) -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64")) && agent == CLIAgent::Claude
}

fn journal_path(root: &Path, agent: CLIAgent) -> PathBuf {
    root.join(format!("{}-homebrew.json", agent.command_prefix()))
}

fn save(path: &Path, journal: &Journal) -> Result<(), Error> {
    super::plain_ancestors(path)?;
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

// Homebrew CaskLock 使用 prefix/var/homebrew/locks/<token>.cask.lock 与 flock。
// 不删除管理器锁文件；锁的 inode 改变时拒绝继续。
fn lock_cask(prefix: &Path, token: &str) -> Result<File, Error> {
    if !matches!(
        token,
        "claude-code" | "claude-code@latest" | "codex" | "grok-build"
    ) {
        return Err(Error::UnsupportedSource);
    }
    let path = prefix
        .join("var/homebrew/locks")
        .join(format!("{token}.cask.lock"));
    super::plain_ancestors(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|_| Error::PermissionDenied)?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(Error::RecoveryRequired);
    }
    let opened = file.metadata().map_err(|_| Error::SourceChanged)?;
    let actual = fs::symlink_metadata(path).map_err(|_| Error::SourceChanged)?;
    if !actual.is_file()
        || opened.dev() != actual.dev()
        || opened.ino() != actual.ino()
        || opened.uid() != unsafe { libc::geteuid() }
        || opened.nlink() != 1
        || opened.mode() & 0o022 != 0
    {
        return Err(Error::SourceChanged);
    }
    Ok(file)
}

async fn download(url: &str, output: &mut File, limit: u64) -> Result<(), Error> {
    let response = http_client::Client::new()
        .get(url)
        .timeout(UPDATE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    let github_asset = url.starts_with(
        "https://github.com/openai/codex/releases/download/rust-v0.156.1/codex-package-",
    ) && response.url().scheme() == "https"
        && response.url().host_str() == Some("release-assets.githubusercontent.com");
    if !response.status().is_success() || response.url().as_str() != url && !github_asset {
        return Err(Error::Network);
    }
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    let mut length = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Network)?;
        length = length
            .checked_add(chunk.len() as u64)
            .ok_or(Error::InvalidRelease)?;
        if length > limit {
            return Err(Error::InvalidRelease);
        }
        output
            .write_all(&chunk)
            .map_err(|_| Error::PersistenceFailed)?;
    }
    if length == 0 {
        return Err(Error::InvalidRelease);
    }
    output.sync_all().map_err(|_| Error::PersistenceFailed)
}

fn bytes_file(
    stage: &Directory,
    path: &Path,
    bytes: &[u8],
    files: &mut BTreeMap<PathBuf, (u64, [u8; 32])>,
) -> Result<(), Error> {
    let digest = Sha256::digest(bytes).into();
    stage.write_new(path, &mut &*bytes, bytes.len() as u64, digest)?;
    files.insert(path.to_owned(), (bytes.len() as u64, digest));
    Ok(())
}

pub(super) async fn execute(
    plan: &UpdatePlan,
    root: &Path,
    progress: Option<VerificationProgress>,
) -> Result<String, Error> {
    brew::supports(plan.agent, &plan.target_version)?;
    if plan.agent == CLIAgent::Codex
        && !matches!(plan.installed_version.as_str(), "0.155.1" | "0.156.1")
    {
        return Err(Error::UnsupportedSource);
    }
    if plan.agent == CLIAgent::Grok {
        super::brew_grok::native(&plan.installed_version)?;
    }
    // cask 渠道由包名决定，不能借更新触发第二套安装或改用户 Claude 全局设置。
    if plan.config.is_some() {
        return Err(Error::ChannelMismatch);
    }
    let invocation = plan
        .installation
        .invocation
        .as_ref()
        .ok_or(Error::UnsupportedSource)?;
    let prefix = invocation
        .artifacts
        .get(&ArtifactRole::InstallRoot)
        .ok_or(Error::UnsupportedSource)?;
    let package = invocation
        .artifacts
        .get(&ArtifactRole::DependencyRoot)
        .ok_or(Error::UnsupportedSource)?;
    let token = package
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Error::UnsupportedSource)?;
    let _cask_lock = lock_cask(prefix, token)?;
    super::verify_installation_identity(&plan.installation)?;
    let registered = brew::registration(
        plan.agent,
        &plan.installation,
        prefix,
        token,
        &plan.installed_version,
    )?;
    let target_entry = relative_entry(plan.agent, &plan.target_version)?;
    if registered.entry_relative != relative_entry(plan.agent, &plan.installed_version)?
        || registered.root != *package
    {
        return Err(Error::UnsupportedSource);
    }
    if plan.agent != CLIAgent::Grok
        && !is_linux_claude(plan.agent)
        && super::version(plan.agent, &plan.installation.entry).await? != plan.installed_version
    {
        return Err(Error::SourceChanged);
    }
    let path = journal_path(root, plan.agent);
    if path.try_exists().map_err(|_| Error::RecoveryRequired)? {
        return Err(Error::RecoveryRequired);
    }
    let mut metadata = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    let metadata_url = if is_linux_claude(plan.agent) {
        #[cfg(target_os = "linux")]
        {
            super::brew_claude_linux::METADATA_URL.to_owned()
        }
        #[cfg(not(target_os = "linux"))]
        {
            return Err(Error::UnsupportedPlatform);
        }
    } else if plan.agent == CLIAgent::Codex {
        super::brew_codex::METADATA_URL.to_owned()
    } else if plan.agent == CLIAgent::Grok {
        super::brew_grok::METADATA_URL.to_owned()
    } else {
        format!("https://formulae.brew.sh/api/cask/{token}.json")
    };
    download(&metadata_url, metadata.as_file_mut(), MAX_CONFIG).await?;
    let (cask, url, digest) = brew::release(
        token,
        &plan.target_version,
        &read_limited(metadata.path(), MAX_CONFIG)?,
    )?;
    let mut image = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    download(&url, image.as_file_mut(), 512 * 1024 * 1024).await?;
    let candidate = stamp(image.path())?;
    if candidate.digest != digest {
        return Err(Error::InvalidRelease);
    }
    let length = image
        .as_file()
        .metadata()
        .map_err(|_| Error::PersistenceFailed)?
        .len();
    let parent = Directory::open(&prefix.join("Caskroom"))?;
    let original = parent.child(token.as_ref())?.snapshot()?;
    let original_link = link(&plan.installation.entry)?;
    let receipt_bytes = read_limited(&registered.receipt, MAX_CONFIG)?;
    let mut receipt: Value =
        serde_json::from_slice(&receipt_bytes).map_err(|_| Error::UnsupportedSource)?;
    if receipt
        .get("runtime_dependencies")
        .and_then(Value::as_object)
        .is_none_or(|value| !value.is_empty())
        || receipt.get("homebrew_version").and_then(Value::as_str) != Some("7.0.4")
    {
        return Err(Error::UnsupportedSource);
    }
    let id = Uuid::new_v4();
    let mut journal = Journal {
        schema: 1,
        id,
        agent: plan.agent.command_prefix().to_owned(),
        prefix: prefix.clone(),
        prefix_identity: Directory::open(prefix)?.identity()?,
        parent_identity: parent.identity()?,
        bin_identity: Directory::open(&prefix.join("bin"))?.identity()?,
        token: token.to_owned(),
        entry: plan.installation.entry.clone(),
        manager: plan
            .installation
            .manager
            .clone()
            .ok_or(Error::UnsupportedSource)?,
        old_version: plan.installed_version.clone(),
        target_version: plan.target_version.clone(),
        intent: plan.intent.clone(),
        original,
        original_link,
        prepared: None,
        prepared_link: None,
        phase: Phase::Preparing,
        probe: None,
        extra_probes: Vec::new(),
        completions: Vec::new(),
        aliases: if plan.agent == CLIAgent::Grok {
            vec![Alias::capture(
                prefix,
                id,
                &plan.installation.stamp.canonical,
            )?]
        } else {
            Vec::new()
        },
        entry_probes: Vec::new(),
    };
    save(&path, &journal)?;
    let result = async {
        if plan.agent == CLIAgent::Grok {
            grok_probes::entries(root, &path, &mut journal, false).await?;
        }
        #[cfg(target_os = "linux")]
        if is_linux_claude(plan.agent) {
            claude_linux_probes::entries(root, &path, &mut journal, false).await?;
        }
        let stage = parent.create(&journal.stage_name())?;
        let native_relative = PathBuf::from(&plan.target_version).join(&target_entry);
        let mut input = File::open(image.path()).map_err(|_| Error::PersistenceFailed)?;
        let (mut files, mut executables) = if plan.agent == CLIAgent::Codex {
            super::brew_codex::unpack(&mut input, &stage, &plan.target_version)?
        } else {
            stage.write_new(&native_relative, &mut input, length, digest)?;
            (
                BTreeMap::from([(native_relative.clone(), (length, digest))]),
                BTreeMap::from([(native_relative.clone(), true)]),
            )
        };
        let timestamp = chrono::Utc::now().format("%Y%m%d%H%M%S.%3f").to_string();
        let cask_relative = PathBuf::from(".metadata")
            .join(&plan.target_version)
            .join(timestamp)
            .join("Casks")
            .join(format!("{token}.json"));
        // Homebrew 7.0.4 的无脚本 cask 使用紧凑 installed JSON；卸载信息保存在 tab 中。
        let cask_bytes = b"{}\n";
        receipt["source"]["version"] = json!(plan.target_version);
        receipt["source"]["tap_git_head"] =
            cask.get("tap_git_head").cloned().unwrap_or(Value::Null);
        receipt["time"] = json!(chrono::Utc::now().timestamp());
        receipt["uninstall_artifacts"] = cask["artifacts"].clone();
        bytes_file(&stage, &cask_relative, cask_bytes, &mut files)?;
        bytes_file(
            &stage,
            Path::new(".metadata/INSTALL_RECEIPT.json"),
            &serde_json::to_vec_pretty(&receipt).map_err(|_| Error::InvalidRelease)?,
            &mut files,
        )?;
        let config = read_limited(&registered.root.join(".metadata/config.json"), MAX_CONFIG)?;
        bytes_file(
            &stage,
            Path::new(".metadata/config.json"),
            &config,
            &mut files,
        )?;
        for path in files.keys() {
            executables.entry(path.clone()).or_insert(false);
        }
        stage.apply_permissions(&journal.original, &executables)?;
        #[cfg(target_os = "linux")]
        if is_linux_claude(plan.agent) {
            super::brew_claude_linux::verify_tree(
                &journal.parent().join(journal.stage_name()),
                &journal.target_version,
            )?;
        }
        let prepared = stage.snapshot()?;
        prepared.verify_release_files(&files)?;
        journal.prepared = Some(prepared);
        let target = registered.root.join(&native_relative);
        symlink(&target, journal.link_stage()).map_err(|_| Error::PersistenceFailed)?;
        journal.prepared_link = Some(link(&journal.link_stage())?);
        for alias in &mut journal.aliases {
            alias.prepare(&target)?;
        }
        super::sync_config_directory(&prefix.join("bin"))?;
        journal.phase = Phase::Prepared;
        save(&path, &journal)?;
        let program = journal
            .parent()
            .join(journal.stage_name())
            .join(&native_relative);
        let (native_length, native_digest) =
            *files.get(&native_relative).ok_or(Error::InvalidRelease)?;
        probe(
            root,
            &path,
            &mut journal,
            &program,
            native_length,
            native_digest,
            None,
        )
        .await?;
        if matches!(plan.agent, CLIAgent::Codex | CLIAgent::Grok) {
            let old_program = plan.installation.stamp.canonical.clone();
            let old_length = fs::metadata(&old_program)
                .map_err(|_| Error::SourceChanged)?
                .len();
            for (shell, completion_path) in completion_paths(plan.agent, prefix) {
                let old_generated = probe(
                    root,
                    &path,
                    &mut journal,
                    &old_program,
                    old_length,
                    plan.installation.stamp.digest,
                    Some(shell),
                )
                .await?;
                let generated = probe(
                    root,
                    &path,
                    &mut journal,
                    &program,
                    native_length,
                    native_digest,
                    Some(shell),
                )
                .await?;
                journal.completions.push(Completion::prepare(
                    completion_path,
                    &old_generated,
                    generated,
                )?);
                save(&path, &journal)?;
            }
        }
        let parent = journal.verify_external()?;
        if parent.child(token.as_ref())?.snapshot()? != journal.original
            || parent.child(&journal.stage_name())?.snapshot()?
                != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
            || link(&journal.entry)? != journal.original_link
            || link(&journal.link_stage())?
                != *journal
                    .prepared_link
                    .as_ref()
                    .ok_or(Error::RecoveryRequired)?
        {
            return Err(Error::SourceChanged);
        }
        for completion in &journal.completions {
            completion.verify_original()?;
        }
        for alias in &journal.aliases {
            alias.verify_original()?;
        }
        journal.phase = Phase::ExchangeIntent;
        save(&path, &journal)?;
        parent.exchange(token.as_ref(), &journal.stage_name())?;
        exchange_links(&journal)?;
        for alias in &journal.aliases {
            alias.publish()?;
        }
        for completion in &journal.completions {
            completion.publish()?;
        }
        verify_published(&journal)?;
        if plan.agent == CLIAgent::Grok {
            grok_probes::entries(root, &path, &mut journal, true).await?;
        }
        #[cfg(target_os = "linux")]
        if is_linux_claude(plan.agent) {
            claude_linux_probes::entries(root, &path, &mut journal, true).await?;
        }
        if let Some(progress) = progress {
            progress.enter().await;
        }
        journal.phase = Phase::Committed;
        save(&path, &journal)?;
        finish(root, &path, &journal)?;
        Ok(plan.target_version.clone())
    }
    .await;
    if result.is_err() {
        // 释放原生 cask 锁后由恢复函数重新获取；不递归在同进程重复领取 flock。
        drop(_cask_lock);
        if recover(plan.agent, &plan.installation.entry, root)? == Some(plan.target_version.clone())
        {
            return Ok(plan.target_version.clone());
        }
    }
    result
}

fn exchange_links(journal: &Journal) -> Result<(), Error> {
    journal.verify_external()?;
    exchange_paths(&journal.entry, &journal.link_stage())?;
    super::sync_config_directory(&journal.prefix.join("bin"))
}

/// 两侧身份由各调用方核对；交换必须是同一文件系统内的内核原子操作。
pub(super) fn exchange_paths(left: &Path, right: &Path) -> Result<(), Error> {
    let left = CString::new(left.as_os_str().as_bytes()).map_err(|_| Error::SourceChanged)?;
    let right = CString::new(right.as_os_str().as_bytes()).map_err(|_| Error::SourceChanged)?;
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(left.as_ptr(), right.as_ptr(), libc::RENAME_SWAP) };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result != 0 {
        return Err(Error::PersistenceFailed);
    }
    Ok(())
}

async fn probe(
    root: &Path,
    path: &Path,
    journal: &mut Journal,
    program: &Path,
    length: u64,
    sha256: [u8; 32],
    shell: Option<&str>,
) -> Result<Vec<u8>, Error> {
    let agent = journal.cli()?;
    let arguments = match shell {
        Some(shell) => vec![completion_command(agent).into(), OsString::from(shell)],
        None => vec!["--version".into()],
    };
    let expected =
        managed_process::ExpectedFileIdentity::capture_release_image(program, length, sha256)
            .map_err(|_| Error::SourceChanged)?;
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(journal.id, program, sha256, &arguments))
                .map_err(|_| Error::PersistenceFailed)?
        )
    );
    let binding = probe_binding(agent, digest.clone())?;
    let generation = Uuid::new_v4();
    let probe = Probe {
        generation,
        program: program.to_owned(),
        digest,
        version: None,
        arguments,
        completed: false,
        output_sha256: None,
    };
    if shell.is_some() {
        journal.extra_probes.push(probe);
    } else {
        journal.probe = Some(probe);
    }
    save(path, journal)?;
    let mut child = if let Some(shell) = shell {
        if agent == CLIAgent::Grok {
            managed_process::spawn_bound_grok_cask_completion(
                root, generation, program, expected, &binding, shell,
            )
            .await
        } else {
            managed_process::spawn_bound_codex_cask_completion(
                root, generation, program, expected, &binding, shell,
            )
            .await
        }
    } else {
        managed_process::spawn_bound_version_probe(root, generation, program, expected, &binding)
            .await
    }
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
    if receipt.exit_code != Some(0) || bytes.len() as u64 > MAX_OUTPUT {
        return Err(Error::ProbeFailed);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| Error::ProbeFailed)?;
    if shell.is_some() {
        if text.is_empty() || text.contains('\0') {
            return Err(Error::ProbeFailed);
        }
        journal
            .extra_probes
            .last_mut()
            .ok_or(Error::RecoveryRequired)?
            .completed = true;
    } else {
        let observed = super::parse_cli_agent_version(agent, text).ok_or(Error::ProbeFailed)?;
        if observed != journal.target_version {
            return Err(Error::VersionMismatch);
        }
        let probe = journal.probe.as_mut().ok_or(Error::RecoveryRequired)?;
        probe.version = Some(observed);
        probe.completed = true;
    }
    let probe = if shell.is_some() {
        journal.extra_probes.last_mut()
    } else {
        journal.probe.as_mut()
    }
    .ok_or(Error::RecoveryRequired)?;
    probe.output_sha256 = Some(format!("{:x}", Sha256::digest(&bytes)));
    save(path, journal)?;
    Ok(bytes)
}

fn probe_binding(
    agent: CLIAgent,
    digest: String,
) -> Result<managed_process::PreparedLaunchBinding, Error> {
    if agent == CLIAgent::Codex {
        managed_process::PreparedLaunchBinding::codex_homebrew_version_probe(digest)
    } else if agent == CLIAgent::Grok {
        managed_process::PreparedLaunchBinding::grok_homebrew_version_probe(digest)
    } else if agent == CLIAgent::Claude {
        managed_process::PreparedLaunchBinding::claude_homebrew_version_probe(digest)
    } else {
        return Err(Error::UnsupportedSource);
    }
    .map_err(|_| Error::RecoveryRequired)
}

fn probe_exited(root: &Path, journal: &Journal) -> Result<(), Error> {
    for probe in journal
        .probe
        .iter()
        .chain(journal.extra_probes.iter())
        .chain(journal.entry_probes.iter())
    {
        let binding = probe_binding(journal.cli()?, probe.digest.clone())?;
        super::record_not_started_if_missing(
            root,
            probe.generation,
            &probe.program,
            &probe.arguments,
            &binding,
        )?;
        let receipt =
            managed_process::confirmed_exit_with_binding(root, probe.generation, &binding)
                .map_err(|_| Error::RecoveryRequired)?
                .ok_or(Error::RecoveryRequired)?;
        if !receipt.cleanup_confirmed || probe.completed && receipt.exit_code != Some(0) {
            return Err(Error::RecoveryRequired);
        }
    }
    Ok(())
}

fn verify_published(journal: &Journal) -> Result<Directory, Error> {
    for alias in &journal.aliases {
        alias.verify_published()?;
    }
    for completion in &journal.completions {
        completion.verify_published()?;
    }
    let parent = journal.verify_external()?;
    if parent.child(journal.token.as_ref())?.snapshot()?
        != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
        || link(&journal.entry)?
            != *journal
                .prepared_link
                .as_ref()
                .ok_or(Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    Ok(parent)
}

fn finish(root: &Path, path: &Path, journal: &Journal) -> Result<(), Error> {
    probe_exited(root, journal)?;
    if journal
        .probe
        .as_ref()
        .and_then(|probe| probe.version.as_deref())
        != Some(journal.target_version.as_str())
    {
        return Err(Error::RecoveryRequired);
    }
    if matches!(journal.cli()?, CLIAgent::Codex | CLIAgent::Grok)
        && (journal.completions.len() != 3
            || journal.extra_probes.len() != 6
            || journal.extra_probes.iter().any(|probe| !probe.completed))
    {
        return Err(Error::RecoveryRequired);
    }
    if journal.cli()? == CLIAgent::Grok
        && (journal.entry_probes.len() != 4
            || journal.entry_probes.iter().any(|probe| !probe.completed)
            || journal.aliases.len() != 1)
    {
        return Err(Error::RecoveryRequired);
    }
    #[cfg(target_os = "linux")]
    if is_linux_claude(journal.cli()?) {
        if journal.entry_probes.len() != 2
            || journal.entry_probes.iter().any(|probe| !probe.completed)
        {
            return Err(Error::RecoveryRequired);
        }
        super::brew_claude_linux::verify_tree(
            &journal.parent().join(&journal.token),
            &journal.target_version,
        )?;
    }
    let parent = verify_published(journal)?;
    if parent.has_child(&journal.stage_name())? {
        parent.remove_matching(&journal.stage_name(), &journal.original)?;
    }
    match fs::symlink_metadata(journal.link_stage()) {
        Ok(_) => {
            if link(&journal.link_stage())? != journal.original_link {
                return Err(Error::SourceChanged);
            }
            fs::remove_file(journal.link_stage()).map_err(|_| Error::RecoveryRequired)?;
            super::sync_config_directory(&journal.prefix.join("bin"))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    for completion in &journal.completions {
        completion.cleanup()?;
    }
    for alias in &journal.aliases {
        alias.cleanup(true)?;
    }
    if journal.cli()? == CLIAgent::Grok {
        grok_probes::receipt(root, journal, true)?;
    }
    #[cfg(target_os = "linux")]
    if is_linux_claude(journal.cli()?) {
        claude_linux_probes::receipt(root, journal, true)?;
    }
    match fs::remove_file(super::failure_path(root, journal.cli()?)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
    super::sync_config_directory(root)
}

pub(super) fn recover(agent: CLIAgent, entry: &Path, root: &Path) -> Result<Option<String>, Error> {
    let path = journal_path(root, agent);
    if !path.try_exists().map_err(|_| Error::RecoveryRequired)? {
        return Ok(None);
    }
    let journal: Journal = serde_json::from_slice(&read_limited(&path, 12 * MAX_CONFIG)?)
        .map_err(|_| Error::RecoveryRequired)?;
    brew::supports(agent, &journal.target_version)?;
    #[cfg(target_os = "linux")]
    if is_linux_claude(agent) {
        super::brew_claude_linux::verify_recovery(
            &journal.prefix,
            &journal.token,
            &journal.old_version,
            &journal.target_version,
        )?;
    }
    let target_entry = relative_entry(agent, &journal.target_version)?;
    let old_entry = relative_entry(agent, &journal.old_version)?;
    let candidate_program = journal
        .parent()
        .join(journal.stage_name())
        .join(&journal.target_version)
        .join(target_entry);
    let old_program = journal
        .parent()
        .join(&journal.token)
        .join(&journal.old_version)
        .join(old_entry);
    if journal.schema != 1
        || journal.agent != agent.command_prefix()
        || journal.id.is_nil()
        || !(agent == CLIAgent::Codex && journal.token == "codex"
            || agent == CLIAgent::Grok && journal.token == "grok-build"
            || agent == CLIAgent::Claude
                && matches!(journal.token.as_str(), "claude-code" | "claude-code@latest"))
        || journal.entry != entry
        || journal.entry != journal.prefix.join("bin").join(agent.command_prefix())
        || journal
            .prefix
            .to_str()
            .is_none_or(|prefix| !super::brew_prefixes().contains(&prefix))
        || journal.probe.as_ref().is_some_and(|probe| {
            probe.program != candidate_program || probe.arguments != [OsString::from("--version")]
        })
        || agent == CLIAgent::Grok
            && (journal.prefix != super::brew_grok::prefix()?
                || journal.aliases.len() != 1
                || journal.entry_probes.len() > 4)
        || agent != CLIAgent::Grok
            && (!journal.aliases.is_empty()
                || !is_linux_claude(agent) && !journal.entry_probes.is_empty())
        || is_linux_claude(agent) && journal.entry_probes.len() > 2
        || journal.completions.len() > 3
        || journal.extra_probes.len() > 6
        || agent == CLIAgent::Claude
            && (!journal.completions.is_empty() || !journal.extra_probes.is_empty())
    {
        return Err(Error::RecoveryRequired);
    }
    for (index, probe) in journal.extra_probes.iter().enumerate() {
        let expected_program = if index % 2 == 0 {
            &old_program
        } else {
            &candidate_program
        };
        if &probe.program != expected_program
            || probe.arguments
                != [
                    OsString::from(completion_command(agent)),
                    OsString::from(["bash", "zsh", "fish"][index / 2]),
                ]
            || probe.version.is_some()
        {
            return Err(Error::RecoveryRequired);
        }
    }
    for (completion, (_, expected)) in journal
        .completions
        .iter()
        .zip(completion_paths(agent, &journal.prefix))
    {
        completion.validate(&expected)?;
    }
    let new_program = journal
        .parent()
        .join(&journal.token)
        .join(&journal.target_version)
        .join(relative_entry(agent, &journal.target_version)?);
    for alias in &journal.aliases {
        alias.validate(&journal.prefix, journal.id, &old_program, &new_program)?;
    }
    for (index, probe) in journal.entry_probes.iter().enumerate() {
        let (program, version) = if index < if is_linux_claude(agent) { 1 } else { 2 } {
            (&old_program, &journal.old_version)
        } else {
            (&new_program, &journal.target_version)
        };
        if &probe.program != program
            || probe.arguments != [OsString::from("--version")]
            || probe
                .version
                .as_ref()
                .is_some_and(|actual| actual != version)
            || probe.completed && probe.version.as_ref() != Some(version)
        {
            return Err(Error::RecoveryRequired);
        }
    }
    let _lock = lock_cask(&journal.prefix, &journal.token)?;
    probe_exited(root, &journal)?;
    if journal.phase == Phase::Committed {
        finish(root, &path, &journal)?;
        return Ok(Some(journal.target_version));
    }
    let parent = journal.verify_external()?;
    let actual = parent.child(journal.token.as_ref())?.snapshot()?;
    let current_link = link(&journal.entry)?;
    #[cfg(target_os = "linux")]
    if is_linux_claude(agent) {
        if actual == journal.original {
            super::brew_claude_linux::verify_tree(
                &journal.parent().join(&journal.token),
                &journal.old_version,
            )?;
            if journal.prepared.is_some() && parent.has_child(&journal.stage_name())? {
                super::brew_claude_linux::verify_tree(
                    &journal.parent().join(journal.stage_name()),
                    &journal.target_version,
                )?;
            }
        } else if Some(&actual) == journal.prepared.as_ref() {
            super::brew_claude_linux::verify_tree(
                &journal.parent().join(&journal.token),
                &journal.target_version,
            )?;
            super::brew_claude_linux::verify_tree(
                &journal.parent().join(journal.stage_name()),
                &journal.old_version,
            )?;
        } else {
            return Err(Error::RecoveryRequired);
        }
    }
    for completion in journal.completions.iter().rev() {
        completion.rollback()?;
    }
    for alias in journal.aliases.iter().rev() {
        alias.rollback()?;
    }
    // 链接、目录各自可能在提交前后，恢复由完整身份决定，不根据 phase 猜测是否交换。
    if current_link != journal.original_link {
        if Some(&current_link) != journal.prepared_link.as_ref()
            || link(&journal.link_stage())? != journal.original_link
        {
            return Err(Error::RecoveryRequired);
        }
        exchange_links(&journal)?;
    }
    if actual != journal.original {
        if Some(&actual) != journal.prepared.as_ref()
            || parent.child(&journal.stage_name())?.snapshot()? != journal.original
        {
            return Err(Error::RecoveryRequired);
        }
        parent.exchange(journal.token.as_ref(), &journal.stage_name())?;
    }
    if parent.child(journal.token.as_ref())?.snapshot()? != journal.original
        || link(&journal.entry)? != journal.original_link
    {
        return Err(Error::RecoveryRequired);
    }
    if parent.has_child(&journal.stage_name())? {
        // 构造尚未完成时没有完整删除清单，保留现场与 journal 等待明确恢复。
        parent.remove_matching(
            &journal.stage_name(),
            journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?,
        )?;
    }
    match fs::symlink_metadata(journal.link_stage()) {
        Ok(_) => {
            if Some(&link(&journal.link_stage())?) != journal.prepared_link.as_ref() {
                return Err(Error::RecoveryRequired);
            }
            fs::remove_file(journal.link_stage()).map_err(|_| Error::RecoveryRequired)?;
            super::sync_config_directory(&journal.prefix.join("bin"))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    super::save_failure_with_intent(
        root,
        agent,
        entry,
        &journal.target_version,
        Some(journal.intent.clone()),
    )?;
    for completion in &journal.completions {
        completion.cleanup()?;
    }
    for alias in &journal.aliases {
        alias.cleanup(false)?;
    }
    if agent == CLIAgent::Grok {
        grok_probes::receipt(root, &journal, false)?;
    }
    #[cfg(target_os = "linux")]
    if is_linux_claude(agent) {
        claude_linux_probes::receipt(root, &journal, false)?;
    }
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
    super::sync_config_directory(root)?;
    Ok(None)
}

fn relative_entry(agent: CLIAgent, version: &str) -> Result<PathBuf, Error> {
    match agent {
        CLIAgent::Codex => Ok("bin/codex".into()),
        CLIAgent::Claude => {
            #[cfg(target_os = "linux")]
            super::brew_claude_linux::native(version)?;
            Ok("claude".into())
        }
        CLIAgent::Grok => super::brew_grok::entry(version),
        CLIAgent::Gemini
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::OpenCode
        | CLIAgent::Copilot
        | CLIAgent::Pi
        | CLIAgent::OhMyPi
        | CLIAgent::Auggie
        | CLIAgent::CursorCli
        | CLIAgent::Goose
        | CLIAgent::DeepSeek
        | CLIAgent::Hermes
        | CLIAgent::Vibe
        | CLIAgent::Antigravity
        | CLIAgent::Omp
        | CLIAgent::WarpTui
        | CLIAgent::Unknown => Err(Error::UnsupportedSource),
    }
}

fn completion_command(agent: CLIAgent) -> &'static str {
    if agent == CLIAgent::Grok {
        "completions"
    } else {
        "completion"
    }
}

fn completion_paths(agent: CLIAgent, prefix: &Path) -> [(&'static str, PathBuf); 3] {
    if agent == CLIAgent::Grok {
        super::brew_grok::completion_paths(prefix)
    } else {
        super::brew_codex::completion_paths(prefix)
    }
}
