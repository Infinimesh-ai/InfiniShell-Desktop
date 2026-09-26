//! 官方 npm 单包树事务。宿主构建完整树；只有固定 Codex launcher 在隔离版本探针中运行 Node。

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use futures::{AsyncReadExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;

use super::npm_release::{LAYOUT_VERSION, NpmRelease};
use super::{
    ArtifactRole, CLIAgent, ConfigBackup, Error, Installation, MAX_CONFIG, MAX_OUTPUT, Stamp,
    UPDATE_TIMEOUT, UpdatePlan, VerificationProgress, managed_process, npm, plain_ancestors,
    read_limited, read_optional_config, stamp, verify_installation_identity,
};
use super::{claude_downgrade, npm_codex};

use super::package_tree as tree;
use tree::{Directory, Identity, Snapshot};

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    prefix: PathBuf,
    prefix_identity: Identity,
    parent_identity: Identity,
    package_root: PathBuf,
    public_relative: PathBuf,
    entry: PathBuf,
    entry_link: Option<EntryLink>,
    external_files: Vec<Stamp>,
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
    program_stamp: Stamp,
    binding_digest: String,
    observed_version: Option<String>,
    #[serde(default)]
    codex_closure: Option<npm_codex::ProbeClosure>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    layout_version: u32,
    agent: String,
    id: Uuid,
    owner: Owner,
    old_version: String,
    target_version: String,
    intent: String,
    #[serde(default)]
    downgrade: Option<claude_downgrade::Intent>,
    stage_name: OsString,
    phase: Phase,
    original: Snapshot,
    prepared: Option<Snapshot>,
    stage_identity: Option<Identity>,
    probe: Option<Probe>,
    config: Option<ConfigBackup>,
    protected_config: Option<ConfigBackup>,
    claude_policy: Option<super::ClaudeUpdateScope>,
    wrapper_archive_sha256: [u8; 32],
    platform_archive_sha256: [u8; 32],
}

pub(super) fn supports(agent: CLIAgent, version: &str) -> Result<(), Error> {
    target()?;
    if agent == CLIAgent::Codex {
        return npm_codex::supports(version);
    }
    if agent != CLIAgent::Claude {
        return Err(Error::UnsupportedSource);
    }
    match version {
        "2.1.280" => Ok(()),
        claude_downgrade::TO => claude_downgrade::platform().map(|_| ()),
        _ => Err(Error::InvalidRelease),
    }
}

fn target() -> Result<String, Error> {
    // Linux 本增量只选择官方 glibc 包；真实加载器闭包由版本 worker 验证后才可交换。
    // 宿主应用自身可静态链接 musl，不能据编译 target_env 推断用户系统 libc。
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        _ => return Err(Error::UnsupportedPlatform),
    };
    let cpu = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        _ => return Err(Error::UnsupportedPlatform),
    };
    Ok(format!("{os}-{cpu}"))
}

fn journal_path(root: &Path, agent: CLIAgent) -> PathBuf {
    root.join(format!("{}-npm.json", agent.command_prefix()))
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

impl Owner {
    fn capture(
        agent: CLIAgent,
        installation: &Installation,
        old: &str,
        public_relative: &Path,
    ) -> Result<Self, Error> {
        verify_installation_identity(installation)?;
        let invocation = installation
            .invocation
            .as_ref()
            .ok_or(Error::UnsupportedSource)?;
        let prefix = invocation
            .artifacts
            .get(&ArtifactRole::InstallRoot)
            .ok_or(Error::UnsupportedSource)?;
        let registration = npm::registered_installation(agent, installation, old, prefix, false)?
            .ok_or(Error::UnsupportedSource)?;
        if installation.stamp.canonical != registration.package_root.join(public_relative) {
            return Err(Error::UnsupportedSource);
        }
        let helper = installation
            .helper
            .as_ref()
            .ok_or(Error::UnsupportedSource)?;
        if !npm::registered_manager(&helper.canonical)? {
            return Err(Error::SourceChanged);
        }
        let manager_manifest = helper
            .canonical
            .parent()
            .and_then(Path::parent)
            .ok_or(Error::UnsupportedSource)?
            .join("package.json");
        let parent = registration
            .package_root
            .parent()
            .ok_or(Error::UnsupportedSource)?;
        let link = if installation.entry == installation.stamp.canonical {
            None
        } else {
            Some(entry_link(&installation.entry)?)
        };
        Ok(Self {
            prefix: registration.prefix.clone(),
            prefix_identity: Directory::open(&registration.prefix)?.identity()?,
            parent_identity: Directory::open(parent)?.identity()?,
            package_root: registration.package_root,
            public_relative: public_relative.to_owned(),
            entry: installation.entry.clone(),
            entry_link: link,
            external_files: vec![
                installation
                    .manager
                    .clone()
                    .ok_or(Error::UnsupportedSource)?,
                helper.clone(),
                stamp(&manager_manifest)?,
            ],
        })
    }

    fn verify_external(&self) -> Result<Directory, Error> {
        if Directory::open(&self.prefix)?.identity()? != self.prefix_identity {
            return Err(Error::SourceChanged);
        }
        let parent = Directory::open(self.package_root.parent().ok_or(Error::SourceChanged)?)?;
        if parent.identity()? != self.parent_identity {
            return Err(Error::SourceChanged);
        }
        for expected in &self.external_files {
            if stamp(&expected.canonical)? != *expected {
                return Err(Error::SourceChanged);
            }
        }
        if let Some(expected) = &self.entry_link {
            let actual = entry_link(&expected.path)?;
            if actual.target != expected.target
                || actual.device != expected.device
                || actual.inode != expected.inode
                || actual.uid != expected.uid
                || actual.gid != expected.gid
                || actual.mode != expected.mode
            {
                return Err(Error::SourceChanged);
            }
        } else if self.entry != self.package_root.join(&self.public_relative) {
            return Err(Error::SourceChanged);
        }
        if self
            .entry
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?
            != self.package_root.join(&self.public_relative)
        {
            return Err(Error::SourceChanged);
        }
        Ok(parent)
    }
}

async fn download(url: &str, output: &mut File, limit: u64) -> Result<(), Error> {
    let client = http_client::Client::new();
    let response = client
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
    let mut size = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Network)?;
        size = size
            .checked_add(chunk.len() as u64)
            .ok_or(Error::InvalidRelease)?;
        if size > limit {
            return Err(Error::InvalidRelease);
        }
        output
            .write_all(&chunk)
            .map_err(|_| Error::PersistenceFailed)?;
    }
    if size == 0 {
        return Err(Error::InvalidRelease);
    }
    output.sync_all().map_err(|_| Error::PersistenceFailed)
}

async fn metadata(url: &str, root: &Path) -> Result<Vec<u8>, Error> {
    let mut file = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    download(url, file.as_file_mut(), MAX_CONFIG).await?;
    read_limited(file.path(), MAX_CONFIG)
}

fn protected_config(agent: CLIAgent) -> Result<Option<ConfigBackup>, Error> {
    let home = super::user_home().ok_or(Error::UnsupportedSource)?;
    let path = match agent {
        CLIAgent::Codex => super::absolute_env("CODEX_HOME")
            .unwrap_or_else(|| home.join(".codex"))
            .join("config.toml"),
        CLIAgent::Claude => return Ok(None),
        _ => return Err(Error::UnsupportedSource),
    };
    plain_ancestors(&path)?;
    let before = read_optional_config(&path)?;
    let mode = before
        .as_ref()
        .map(|_| {
            fs::metadata(&path)
                .map(|m| m.mode() & 0o777)
                .map_err(|_| Error::SourceChanged)
        })
        .transpose()?;
    Ok(Some(ConfigBackup {
        kind: super::ConfigKind::CodexUpdateMarker,
        path,
        before,
        after: None,
        desired: None,
        before_mode: mode,
        restore_stage: None,
    }))
}

fn unchanged(config: &Option<ConfigBackup>, published: bool) -> Result<(), Error> {
    if let Some(config) = config {
        let expected = config.publication_bytes(published);
        if read_optional_config(&config.path)?.as_ref() != expected {
            return Err(Error::SourceChanged);
        }
        if let Some(mode) = config.before_mode {
            if fs::metadata(&config.path)
                .map_err(|_| Error::SourceChanged)?
                .mode()
                & 0o777
                != mode
            {
                return Err(Error::SourceChanged);
            }
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
    if plan.agent == CLIAgent::Claude {
        claude_downgrade::validate(
            plan.downgrade,
            &plan.installed_version,
            &plan.target_version,
            &plan.config,
        )?;
    }
    if journal_path(root, plan.agent)
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    if !plan.requires_native_update() {
        unchanged(&plan.config, false)?;
        let mut config = plan.config.clone().ok_or(Error::UnsupportedSource)?;
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
        let path = root.join(format!("{}.json", plan.agent.command_prefix()));
        super::save_journal(&path, &journal)?;
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
    let platform = target()?;
    if plan.agent == CLIAgent::Codex
        && !matches!(plan.installed_version.as_str(), "0.155.1" | "0.156.1")
    {
        return Err(Error::InvalidRelease);
    }
    let package = npm::package_name(plan.agent).ok_or(Error::UnsupportedSource)?;
    let platform_name = if plan.agent == CLIAgent::Codex {
        package.to_owned()
    } else {
        format!("{package}-{platform}")
    };
    let platform_version = if plan.agent == CLIAgent::Codex {
        format!("{}-{platform}", plan.target_version)
    } else {
        plan.target_version.clone()
    };
    let wrapper_meta = metadata(
        &format!(
            "https://registry.npmjs.org/{package}/{}",
            plan.target_version
        ),
        root,
    )
    .await?;
    let platform_meta = metadata(
        &format!("https://registry.npmjs.org/{platform_name}/{platform_version}"),
        root,
    )
    .await?;
    if plan.agent == CLIAgent::Codex {
        npm_codex::verify_metadata(&wrapper_meta, &platform_meta)?;
    } else if plan.target_version == claude_downgrade::TO {
        claude_downgrade::verify_metadata(&platform, &wrapper_meta, &platform_meta)?;
    }
    let release = NpmRelease::from_metadata(
        plan.agent,
        &plan.target_version,
        &platform,
        &wrapper_meta,
        &platform_meta,
    )?;
    let owner = Owner::capture(
        plan.agent,
        &plan.installation,
        &plan.installed_version,
        &release.public_entry,
    )?;
    let parent = owner.verify_external()?;
    let package_name = owner
        .package_root
        .file_name()
        .ok_or(Error::UnsupportedSource)?
        .to_owned();
    let original = parent.child(&package_name)?.snapshot()?;
    let protected_config = protected_config(plan.agent)?;
    let claude_policy = if plan.agent == CLIAgent::Claude {
        let config = plan.config.as_ref().ok_or(Error::UnsupportedSource)?;
        Some(super::snapshot_claude_scope(
            config,
            Uuid::new_v4(),
            super::claude_metadata_path(config)?,
            &plan.target_version,
        )?)
    } else {
        None
    };
    unchanged(&protected_config, false)?;
    unchanged(&plan.config, false)?;
    let mut wrapper = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    let mut platform_file = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    download(
        &release.wrapper.tarball_url,
        wrapper.as_file_mut(),
        512 * 1024 * 1024,
    )
    .await?;
    download(
        &release.platform.tarball_url,
        platform_file.as_file_mut(),
        512 * 1024 * 1024,
    )
    .await?;
    let wrapper_verified = release.wrapper.verify_archive(wrapper.as_file_mut())?;
    let platform_verified = release
        .platform
        .verify_archive(platform_file.as_file_mut())?;
    release.verify_entries(&wrapper_verified, &platform_verified)?;
    if plan.agent == CLIAgent::Codex {
        npm_codex::verify_archives(&release, &wrapper_verified, &platform_verified)?;
    } else if plan.target_version == claude_downgrade::TO {
        claude_downgrade::verify_archives(&platform, &wrapper_verified, &platform_verified)?;
    }
    #[cfg(test)]
    live_tests::preserve_verified_inputs(
        &wrapper_meta,
        &platform_meta,
        wrapper.path(),
        platform_file.path(),
    );

    let id = Uuid::new_v4();
    let stage_name = OsString::from(format!(".infinishell-npm-{id}"));
    let path = journal_path(root, plan.agent);
    if path.try_exists().map_err(|_| Error::RecoveryRequired)? {
        return Err(Error::RecoveryRequired);
    }
    let mut journal = Journal {
        schema: 1,
        layout_version: LAYOUT_VERSION,
        agent: plan.agent.command_prefix().to_owned(),
        id,
        owner,
        old_version: plan.installed_version.clone(),
        target_version: plan.target_version.clone(),
        intent: plan.intent.clone(),
        downgrade: plan.downgrade,
        stage_name,
        phase: Phase::Allocating,
        original,
        prepared: None,
        stage_identity: None,
        probe: None,
        config: plan.config.clone(),
        protected_config,
        claude_policy,
        wrapper_archive_sha256: wrapper_verified.compressed_sha256,
        platform_archive_sha256: platform_verified.compressed_sha256,
    };
    save(&path, &journal)?;
    let result = async {
        let stage = parent.create(&journal.stage_name)?;
        journal.stage_identity = Some(stage.identity()?);
        journal.phase = Phase::Preparing;
        save(&path, &journal)?;
        release
            .wrapper
            .extract_verified(wrapper.as_file_mut(), |path, entry, bytes| {
                stage.write_new(path, bytes, entry.length, entry.sha256)
            })?;
        release
            .platform
            .extract_verified(platform_file.as_file_mut(), |path, entry, bytes| {
                stage.write_new(
                    &release.dependency_directory.join(path),
                    bytes,
                    entry.length,
                    entry.sha256,
                )
            })?;
        let native_relative = release.dependency_directory.join(&release.native_entry);
        let native = platform_verified
            .files
            .get(&release.native_entry)
            .ok_or(Error::InvalidRelease)?;
        if release.materialize_native_entry {
            stage.replace_private_file(
                &release.public_entry,
                &native_relative,
                native.length,
                native.sha256,
            )?;
        }
        let mut executables: BTreeMap<_, _> = wrapper_verified
            .files
            .iter()
            .map(|(path, entry)| (path.clone(), entry.executable))
            .collect();
        executables.extend(
            platform_verified
                .files
                .iter()
                .map(|(path, entry)| (release.dependency_directory.join(path), entry.executable)),
        );
        if release.materialize_native_entry {
            executables.insert(release.public_entry.clone(), true);
        }
        stage.apply_permissions(&journal.original, &executables)?;
        let mut expected_files: BTreeMap<_, _> = wrapper_verified
            .files
            .iter()
            .map(|(path, entry)| (path.clone(), (entry.length, entry.sha256)))
            .collect();
        expected_files.extend(platform_verified.files.iter().map(|(path, entry)| {
            (
                release.dependency_directory.join(path),
                (entry.length, entry.sha256),
            )
        }));
        if release.materialize_native_entry {
            expected_files.insert(release.public_entry.clone(), (native.length, native.sha256));
        }
        let prepared = stage.snapshot()?;
        prepared.verify_release_files(&expected_files)?;
        journal.prepared = Some(prepared);
        save(&path, &journal)?;
        #[cfg(test)]
        live_tests::checkpoint(live_tests::Point::Prepared).await;
        // 精确执行即将发布的公共入口，不以依赖包中的同名 native 代替入口验收。
        let public_program = journal
            .owner
            .package_root
            .parent()
            .ok_or(Error::SourceChanged)?
            .join(&journal.stage_name)
            .join(&release.public_entry);
        probe_version(
            plan.agent,
            root,
            &path,
            &mut journal,
            &public_program,
            native.length,
            native.sha256,
            &expected_files,
        )
        .await?;
        unchanged(&journal.config, false)?;
        unchanged(&journal.protected_config, false)?;
        if let Some(scope) = &journal.claude_policy {
            super::verify_claude_originals(scope)?;
        }
        verify_probe_dependencies(&journal)?;
        let current_parent = journal.owner.verify_external()?;
        if current_parent.child(&package_name)?.snapshot()? != journal.original
            || current_parent.child(&journal.stage_name)?.snapshot()?
                != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
        {
            return Err(Error::SourceChanged);
        }
        journal.phase = Phase::SwapIntent;
        save(&path, &journal)?;
        current_parent.exchange(&package_name, &journal.stage_name)?;
        #[cfg(test)]
        live_tests::checkpoint(live_tests::Point::Exchanged).await;
        // 在交换和后续记录之间退出时，恢复通过两侧树身份判断是否交换，不能依赖 phase 猜测。
        verify_swapped(&journal)?;
        if let Some(progress) = progress {
            progress.enter().await;
        }
        journal.phase = Phase::PublishingConfig;
        if let Some(config) = journal.config.as_mut() {
            config.after = config.before.clone();
            super::plan_config_publish(config, true)?;
        }
        save(&path, &journal)?;
        if let Some(config) = &journal.config {
            super::publish_config(config, true)?;
        }
        unchanged(&journal.config, true)?;
        unchanged(&journal.protected_config, false)?;
        verify_swapped(&journal)?;
        journal.phase = Phase::Committed;
        save(&path, &journal)?;
        finish_committed(root, plan.agent, &path, &journal)?;
        Ok(plan.target_version.clone())
    }
    .await;
    if result.is_err() {
        // 恢复也失败时保留 journal；不删除未知树或覆盖外部 npm 的后续修改。
        if recover(plan.agent, &plan.installation.entry, root)? == Some(plan.target_version.clone())
        {
            return Ok(plan.target_version.clone());
        }
    }
    result
}

async fn probe_version(
    agent: CLIAgent,
    root: &Path,
    path: &Path,
    journal: &mut Journal,
    program: &Path,
    release_length: u64,
    release_digest: [u8; 32],
    release_files: &BTreeMap<PathBuf, (u64, [u8; 32])>,
) -> Result<(), Error> {
    let codex_closure = if agent == CLIAgent::Codex {
        let stage = program
            .parent()
            .and_then(Path::parent)
            .ok_or(Error::SourceChanged)?;
        let node = journal
            .owner
            .external_files
            .first()
            .ok_or(Error::SourceChanged)?;
        Some(npm_codex::capture_probe(node, stage, release_files)?)
    } else {
        None
    };
    let program = if codex_closure.is_some() {
        journal
            .owner
            .external_files
            .first()
            .ok_or(Error::SourceChanged)?
            .canonical
            .clone()
    } else {
        program.to_owned()
    };
    let program = program.as_path();
    let program_stamp = stamp(program)?;
    let digest_bytes = if codex_closure.is_some() {
        serde_json::to_vec(&(&program_stamp, "--version", journal.id, &codex_closure))
    } else {
        // Claude 保留既有收据绑定格式，不能因扩展 Codex 入口改变旧事务合同。
        serde_json::to_vec(&(&program_stamp, "--version", journal.id))
    }
    .map_err(|_| Error::PersistenceFailed)?;
    let digest = format!("{:x}", Sha256::digest(digest_bytes));
    let binding = probe_binding(agent == CLIAgent::Codex, digest.clone())?;
    let generation = Uuid::new_v4();
    journal.probe = Some(Probe {
        generation,
        program: program.to_owned(),
        program_stamp,
        binding_digest: digest,
        observed_version: None,
        codex_closure,
    });
    save(path, journal)?;
    if stamp(program)?
        != journal
            .probe
            .as_ref()
            .ok_or(Error::RecoveryRequired)?
            .program_stamp
    {
        return Err(Error::SourceChanged);
    }
    let mut child = if let Some(closure) = journal
        .probe
        .as_ref()
        .and_then(|probe| probe.codex_closure.as_ref())
    {
        managed_process::spawn_bound_codex_npm_version_probe(
            root,
            generation,
            program,
            &closure.arguments,
            closure.expected_files.clone(),
            &binding,
        )
        .await
        .map_err(|_| Error::RecoveryRequired)?
    } else {
        let expected = managed_process::ExpectedFileIdentity::capture_release_image(
            program,
            release_length,
            release_digest,
        )
        .map_err(|_| Error::SourceChanged)?;
        managed_process::spawn_bound_version_probe(root, generation, program, expected, &binding)
            .await
            .map_err(|_| Error::RecoveryRequired)?
    };
    let mut stdout = child.stdout.take().ok_or(Error::RecoveryRequired)?;
    let mut bytes = Vec::new();
    // 监督者就绪后仍需校验并原子执行候选镜像，不能套用普通版本查询的 15 秒预算。
    let output = (&mut stdout)
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .with_timeout(UPDATE_TIMEOUT)
        .await;
    drop(stdout);
    #[cfg(test)]
    live_tests::preserve_probe_stdout(generation, &bytes);
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
    let actual = super::parse_cli_agent_version(
        agent,
        std::str::from_utf8(&bytes).map_err(|_| Error::ProbeFailed)?,
    )
    .ok_or(Error::ProbeFailed)?;
    if actual != journal.target_version {
        return Err(Error::VersionMismatch);
    }
    journal
        .probe
        .as_mut()
        .ok_or(Error::RecoveryRequired)?
        .observed_version = Some(actual);
    save(path, journal)
}

fn verify_swapped(journal: &Journal) -> Result<Directory, Error> {
    verify_probe_dependencies(journal)?;
    let parent = journal.owner.verify_external()?;
    let name = journal
        .owner
        .package_root
        .file_name()
        .ok_or(Error::RecoveryRequired)?;
    if parent.child(name)?.snapshot()?
        != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
        || parent.child(&journal.stage_name)?.snapshot()? != journal.original
    {
        return Err(Error::RecoveryRequired);
    }
    Ok(parent)
}

fn probe_binding(
    codex: bool,
    digest: String,
) -> Result<managed_process::PreparedLaunchBinding, Error> {
    if codex {
        managed_process::PreparedLaunchBinding::codex_npm_version_probe(digest)
    } else {
        managed_process::PreparedLaunchBinding::claude_npm_version_probe(digest)
    }
    .map_err(|_| Error::RecoveryRequired)
}

fn validate(agent: CLIAgent, entry: &Path, journal: &Journal) -> Result<(), Error> {
    supports(agent, &journal.target_version)?;
    if agent == CLIAgent::Claude {
        claude_downgrade::validate(
            journal.downgrade,
            &journal.old_version,
            &journal.target_version,
            &journal.config,
        )?;
    } else if journal.downgrade.is_some() {
        return Err(Error::RecoveryRequired);
    }
    let package = npm::package_name(agent).ok_or(Error::RecoveryRequired)?;
    if journal.schema != 1
        || journal.layout_version != LAYOUT_VERSION
        || journal.agent != agent.command_prefix()
        || journal.owner.entry != entry
        || journal.stage_name != OsString::from(format!(".infinishell-npm-{}", journal.id))
        || journal.owner.package_root != journal.owner.prefix.join("lib/node_modules").join(package)
        || !journal.owner.prefix.is_absolute()
        || journal.owner.public_relative
            != Path::new(if agent == CLIAgent::Codex {
                "bin/codex.js"
            } else {
                "bin/claude.exe"
            })
    {
        return Err(Error::RecoveryRequired);
    }
    if let Some(probe) = &journal.probe {
        let stage = journal
            .owner
            .package_root
            .parent()
            .ok_or(Error::RecoveryRequired)?
            .join(&journal.stage_name);
        if probe.program_stamp.canonical != probe.program {
            return Err(Error::RecoveryRequired);
        }
        if agent == CLIAgent::Codex {
            let closure = probe
                .codex_closure
                .as_ref()
                .ok_or(Error::RecoveryRequired)?;
            let node = journal
                .owner
                .external_files
                .first()
                .ok_or(Error::RecoveryRequired)?;
            if probe.program != node.canonical
                || probe.program_stamp != *node
                || closure.arguments
                    != [
                        stage.join("bin/codex.js").into_os_string(),
                        "--version".into(),
                    ]
                || closure
                    .expected_files
                    .first()
                    .is_none_or(|file| file.path() != probe.program)
                || closure.expected_files.len() < 4
                || managed_process::validate_codex_npm_probe_contract(
                    &probe.program,
                    &closure.arguments,
                    &closure.expected_files,
                )
                .is_err()
                || probe.binding_digest
                    != format!(
                        "{:x}",
                        Sha256::digest(
                            serde_json::to_vec(&(
                                &probe.program_stamp,
                                "--version",
                                journal.id,
                                &probe.codex_closure
                            ))
                            .map_err(|_| Error::RecoveryRequired)?
                        )
                    )
            {
                return Err(Error::RecoveryRequired);
            }
        } else if probe.codex_closure.is_some()
            || probe.program != stage.join(&journal.owner.public_relative)
        {
            return Err(Error::RecoveryRequired);
        }
    }
    Ok(())
}

fn verified_probe_exit(root: &Path, journal: &Journal) -> Result<(), Error> {
    if let Some(probe) = &journal.probe {
        let binding = probe_binding(journal.agent == "codex", probe.binding_digest.clone())?;
        let arguments = probe.codex_closure.as_ref().map_or_else(
            || vec!["--version".into()],
            |closure| closure.arguments.clone(),
        );
        super::record_not_started_if_missing(
            root,
            probe.generation,
            &probe.program,
            &arguments,
            &binding,
        )?;
        let receipt =
            managed_process::confirmed_exit_with_binding(root, probe.generation, &binding)
                .map_err(|_| Error::RecoveryRequired)?
                .ok_or(Error::RecoveryRequired)?;
        if !receipt.cleanup_confirmed
            || probe.observed_version.is_some() && receipt.exit_code != Some(0)
        {
            return Err(Error::RecoveryRequired);
        }
    }
    Ok(())
}

fn verify_probe_dependencies(journal: &Journal) -> Result<(), Error> {
    if let Some(probe) = &journal.probe
        && let Some(closure) = &probe.codex_closure
    {
        managed_process::verify_codex_npm_probe_dependencies(
            &probe.program,
            &closure.arguments,
            &closure.expected_files,
        )
        .map_err(|_| Error::SourceChanged)?;
    }
    Ok(())
}

pub(super) fn recover(agent: CLIAgent, entry: &Path, root: &Path) -> Result<Option<String>, Error> {
    let path = journal_path(root, agent);
    if !path.try_exists().map_err(|_| Error::RecoveryRequired)? {
        return Ok(None);
    }
    let mut journal: Journal = serde_json::from_slice(&read_limited(&path, 12 * MAX_CONFIG)?)
        .map_err(|_| Error::RecoveryRequired)?;
    validate(agent, entry, &journal)?;
    verified_probe_exit(root, &journal)?;
    verify_probe_dependencies(&journal)?;
    if let Some(scope) = &journal.claude_policy {
        super::verify_claude_originals(scope)?;
    }
    if journal.phase == Phase::PublishingConfig {
        verify_swapped(&journal)?;
        if journal
            .probe
            .as_ref()
            .and_then(|probe| probe.observed_version.as_deref())
            != Some(journal.target_version.as_str())
        {
            return Err(Error::RecoveryRequired);
        }
        unchanged(&journal.protected_config, false)?;
        if let Some(config) = &journal.config {
            super::publish_config(config, true)?;
        }
        unchanged(&journal.config, true)?;
        journal.phase = Phase::Committed;
        save(&path, &journal)?;
    }
    if journal.phase == Phase::Committed {
        finish_committed(root, agent, &path, &journal)?;
        return Ok(Some(journal.target_version));
    }
    let parent = journal.owner.verify_external()?;
    let name = journal
        .owner
        .package_root
        .file_name()
        .ok_or(Error::RecoveryRequired)?;
    let actual = parent.child(name)?.snapshot()?;
    if actual != journal.original {
        let parent = verify_swapped(&journal)?;
        // 先确认外部安装没有再变动，才允许用完整旧树交换回去。
        parent.exchange(name, &journal.stage_name)?;
        if parent.child(name)?.snapshot()? != journal.original {
            return Err(Error::RecoveryRequired);
        }
    }
    unchanged(&journal.config, false)?;
    unchanged(&journal.protected_config, false)?;
    if let Some(prepared) = &journal.prepared {
        if parent.has_child(&journal.stage_name)? {
            parent.remove_matching(&journal.stage_name, prepared)?;
        }
    } else if parent.has_child(&journal.stage_name)? {
        // 尚未形成完整清单的目录只归档，不猜测崩溃之后是否被其他进程修改。
        let retained = root.join(format!(
            "{}-npm-retained-{}.json",
            agent.command_prefix(),
            journal.id
        ));
        save(&retained, &journal)?;
    }
    if let Some(config) = &journal.config {
        super::cleanup_config_restore(config)?;
    }
    super::save_failure_with_intent(
        root,
        agent,
        entry,
        &journal.target_version,
        Some(journal.intent.clone()),
    )?;
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
    super::sync_config_directory(root)?;
    Ok(None)
}

fn finish_committed(
    root: &Path,
    agent: CLIAgent,
    path: &Path,
    journal: &Journal,
) -> Result<(), Error> {
    if journal
        .probe
        .as_ref()
        .and_then(|probe| probe.observed_version.as_deref())
        != Some(journal.target_version.as_str())
    {
        return Err(Error::RecoveryRequired);
    }
    verified_probe_exit(root, journal)?;
    verify_probe_dependencies(journal)?;
    if let Some(scope) = &journal.claude_policy {
        super::verify_claude_originals(scope)?;
    }
    unchanged(&journal.config, true)?;
    unchanged(&journal.protected_config, false)?;
    let parent = journal.owner.verify_external()?;
    let name = journal
        .owner
        .package_root
        .file_name()
        .ok_or(Error::RecoveryRequired)?;
    if parent.child(name)?.snapshot()?
        != *journal.prepared.as_ref().ok_or(Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    if parent.has_child(&journal.stage_name)? {
        parent.remove_matching(&journal.stage_name, &journal.original)?;
    }
    if let Some(config) = &journal.config {
        super::cleanup_config_restore(config)?;
    }
    match fs::remove_file(super::failure_path(root, agent)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
    super::sync_config_directory(root)
}

#[cfg(test)]
#[path = "sources_npm_transaction_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sources_npm_live_tests.rs"]
mod live_tests;

#[cfg(test)]
pub(super) fn fixed_test_release(agent: CLIAgent, channel: super::Channel) -> Option<String> {
    live_tests::fixed_release(agent, channel)
}

#[cfg(test)]
pub(super) fn audit_test_probe(invocation: &super::Invocation) -> Result<(), Error> {
    live_tests::audit_probe(invocation)
}
