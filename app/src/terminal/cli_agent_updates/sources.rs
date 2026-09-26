//! 来源必须同时绑定当前入口和管理器记录；PATH 中存在管理器并不能证明 CLI 归它管理。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::ai::cli_agent_runtime::managed_process;
#[cfg(feature = "local_fs")]
use crate::ai::cli_agent_runtime::managed_process::ManagedEnvironment;
use command::Stdio;
use command::r#async::Command;
use futures::{AsyncReadExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;

use super::{
    CliAgentUpdateChannel as Channel, CliAgentUpdateError as Error, CliAgentUpdateSource as Source,
};
use crate::terminal::cli_agent::{CLIAgent, parse_cli_agent_version};
#[cfg(test)]
use crate::terminal::cli_agent_sessions::plugin_manager::plugin_manager_for;

#[path = "sources_npm.rs"]
mod npm;

#[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
#[path = "sources_npm_release.rs"]
mod npm_release;
#[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
#[path = "sources_npm_transaction.rs"]
mod npm_transaction;

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);
const VERIFICATION_ACK_TIMEOUT: Duration = Duration::from_secs(1);
// 真实收据会在监督二进制中直接查找这些编译输入，不能由外部报告代替同源证明。
#[used]
static SUPERVISOR_UPDATER_SOURCE_BINDING: [&[u8]; 22] = [
    include_bytes!("../cli_agent_updates.rs"),
    include_bytes!("sources.rs"),
    include_bytes!("sources_npm.rs"),
    include_bytes!("sources_npm_release.rs"),
    include_bytes!("sources_npm_transaction.rs"),
    include_bytes!("sources_npm_tree_unix.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process_version_probe.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_linux.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_linux_glibc.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_macos.rs"),
    include_bytes!("../../ai/cli_agent_runtime/managed_process_atomic_windows.rs"),
    include_bytes!("../../ai/cli_agent_runtime/codex.rs"),
    include_bytes!("../../ai/cli_agent_runtime/claude.rs"),
    include_bytes!("../../ai/cli_agent_runtime/grok.rs"),
    include_bytes!("../cli_agent_sessions/plugin_manager/mod.rs"),
    include_bytes!("../cli_agent_sessions/plugin_manager/codex.rs"),
    include_bytes!("../cli_agent_sessions/plugin_manager/claude.rs"),
    include_bytes!("../cli_agent_sessions/plugin_manager/grok.rs"),
    include_bytes!("../cli_agent_sessions/plugin_manager/codex_source.rs"),
    include_bytes!("../cli_agent_sessions/plugin_manager/codex_hook_trust.rs"),
    include_bytes!("../cli_agent_sessions/plugin_manager/notification_patch.rs"),
];

pub(super) struct VerificationProgress {
    entered: async_channel::Sender<()>,
    resume: async_channel::Receiver<()>,
}

impl VerificationProgress {
    pub(super) fn new(
        entered: async_channel::Sender<()>,
        resume: async_channel::Receiver<()>,
    ) -> Self {
        Self { entered, resume }
    }

    async fn enter(self) {
        self.enter_with_timeout(VERIFICATION_ACK_TIMEOUT).await;
    }

    async fn enter_with_timeout(self, timeout: Duration) {
        // UI 已关闭时不能让非关键进度通知阻塞事务收敛。
        if self.entered.send(()).await.is_ok() {
            // 主线程回调延迟或丢失只降级进度展示，不改变安装核验结果。
            let _ = self.resume.recv().with_timeout(timeout).await;
        }
    }
}

const MAX_OUTPUT: u64 = 1024 * 1024;
const MAX_CONFIG: u64 = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Stamp {
    canonical: PathBuf,
    digest: [u8; 32],
}

#[derive(Clone, Debug)]
struct Invocation {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    env_remove: Vec<OsString>,
}

impl Invocation {
    fn new(program: impl Into<PathBuf>, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        Self {
            program: program.into(),
            args: args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect(),
            env: Vec::new(),
            env_remove: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ArtifactRole {
    Program,
    Entry,
    Manager,
    Helper,
    Registration,
    InstallRoot,
    ConfigRoot,
    DependencyRoot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ArgumentRef {
    Literal(OsString),
    ArtifactPath {
        role: ArtifactRole,
        relative: PathBuf,
    },
    TargetVersion,
    PackageVersion {
        package: String,
    },
    PathList(Vec<ArgumentRef>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LaunchBindingSpec {
    NativeFile {
        program: ArtifactRole,
    },
    SnapshotTree {
        root: ArtifactRole,
        program: ArgumentRef,
    },
    WindowsLease {
        program: ArtifactRole,
        ancestors: Vec<ArtifactRole>,
    },
    ManualOnly {
        reason: String,
    },
}

#[derive(Clone, Debug)]
struct BoundInvocation {
    artifacts: BTreeMap<ArtifactRole, PathBuf>,
    program: ArtifactRole,
    arguments: Vec<ArgumentRef>,
    environment: Vec<(OsString, ArgumentRef)>,
    env_remove: Vec<OsString>,
    binding: LaunchBindingSpec,
}

#[derive(Clone, Debug)]
struct PreparedInvocation {
    invocation: Invocation,
    binding: managed_process::PreparedLaunchBinding,
}

impl BoundInvocation {
    fn manual_only(
        artifacts: impl IntoIterator<Item = (ArtifactRole, PathBuf)>,
        program: ArtifactRole,
        arguments: Vec<ArgumentRef>,
        reason: &str,
    ) -> Self {
        Self {
            artifacts: artifacts.into_iter().collect(),
            program,
            arguments,
            environment: Vec::new(),
            env_remove: Vec::new(),
            binding: LaunchBindingSpec::ManualOnly {
                reason: reason.to_owned(),
            },
        }
    }

    fn prepare(&self, target_version: &str) -> Result<PreparedInvocation, Error> {
        if matches!(self.binding, LaunchBindingSpec::ManualOnly { .. }) {
            return Err(Error::UnsupportedSource);
        }
        let program = self
            .artifacts
            .get(&self.program)
            .filter(|path| path.is_absolute())
            .ok_or(Error::UnsupportedSource)?
            .clone();
        let args = self
            .arguments
            .iter()
            .map(|argument| self.resolve(argument, target_version))
            .collect::<Result<Vec<_>, _>>()?;
        let env = self
            .environment
            .iter()
            .map(|(name, value)| Ok((name.clone(), self.resolve(value, target_version)?)))
            .collect::<Result<Vec<_>, Error>>()?;
        let digest = self.digest(target_version)?;
        let binding = match &self.binding {
            LaunchBindingSpec::NativeFile { .. } => {
                managed_process::PreparedLaunchBinding::native_file(digest)
            }
            LaunchBindingSpec::SnapshotTree { .. } | LaunchBindingSpec::WindowsLease { .. } => {
                managed_process::PreparedLaunchBinding::new(digest)
            }
            LaunchBindingSpec::ManualOnly { .. } => unreachable!("ManualOnly 已在物化前拒绝"),
        }
        .map_err(|_| Error::UnsupportedSource)?;
        Ok(PreparedInvocation {
            invocation: Invocation {
                program,
                args,
                env,
                env_remove: self.env_remove.clone(),
            },
            binding,
        })
    }

    fn resolve(&self, argument: &ArgumentRef, target_version: &str) -> Result<OsString, Error> {
        match argument {
            ArgumentRef::Literal(value) => Ok(value.clone()),
            ArgumentRef::ArtifactPath { role, relative } => {
                if relative.is_absolute()
                    || relative.components().any(|component| {
                        matches!(
                            component,
                            std::path::Component::ParentDir
                                | std::path::Component::RootDir
                                | std::path::Component::Prefix(_)
                        )
                    })
                {
                    return Err(Error::UnsupportedSource);
                }
                let base = self.artifacts.get(role).ok_or(Error::UnsupportedSource)?;
                Ok(if relative.as_os_str().is_empty() {
                    base.as_os_str().to_owned()
                } else {
                    base.join(relative).into_os_string()
                })
            }
            ArgumentRef::TargetVersion => Ok(target_version.into()),
            ArgumentRef::PackageVersion { package } => {
                Ok(format!("{package}@{target_version}").into())
            }
            ArgumentRef::PathList(items) => {
                let paths = items
                    .iter()
                    .map(|item| self.resolve(item, target_version).map(PathBuf::from))
                    .collect::<Result<Vec<_>, _>>()?;
                std::env::join_paths(paths).map_err(|_| Error::UnsupportedSource)
            }
        }
    }

    fn digest(&self, target_version: &str) -> Result<String, Error> {
        let mut digest = Sha256::new();
        digest_field(&mut digest, b"infinishell.launch-binding.v1");
        digest_field(&mut digest, target_version.as_bytes());
        digest_role(&mut digest, self.program);
        for (role, path) in &self.artifacts {
            digest_role(&mut digest, *role);
            digest_field(&mut digest, path.as_os_str().as_encoded_bytes());
        }
        for argument in &self.arguments {
            digest_argument(&mut digest, argument);
        }
        for (name, value) in &self.environment {
            digest_field(&mut digest, name.as_encoded_bytes());
            digest_argument(&mut digest, value);
        }
        for name in &self.env_remove {
            digest_field(&mut digest, name.as_encoded_bytes());
        }
        digest_binding(&mut digest, &self.binding);
        Ok(format!("{:x}", digest.finalize()))
    }
}

fn digest_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn digest_role(digest: &mut Sha256, role: ArtifactRole) {
    digest.update([role as u8]);
}

fn digest_argument(digest: &mut Sha256, argument: &ArgumentRef) {
    match argument {
        ArgumentRef::Literal(value) => {
            digest.update([0]);
            digest_field(digest, value.as_encoded_bytes());
        }
        ArgumentRef::ArtifactPath { role, relative } => {
            digest.update([1]);
            digest_role(digest, *role);
            digest_field(digest, relative.as_os_str().as_encoded_bytes());
        }
        ArgumentRef::TargetVersion => digest.update([2]),
        ArgumentRef::PackageVersion { package } => {
            digest.update([3]);
            digest_field(digest, package.as_bytes());
        }
        ArgumentRef::PathList(items) => {
            digest.update([4]);
            digest.update((items.len() as u64).to_be_bytes());
            for item in items {
                digest_argument(digest, item);
            }
        }
    }
}

fn digest_binding(digest: &mut Sha256, binding: &LaunchBindingSpec) {
    match binding {
        LaunchBindingSpec::NativeFile { program } => {
            digest.update([0]);
            digest_role(digest, *program);
        }
        LaunchBindingSpec::SnapshotTree { root, program } => {
            digest.update([1]);
            digest_role(digest, *root);
            digest_argument(digest, program);
        }
        LaunchBindingSpec::WindowsLease { program, ancestors } => {
            digest.update([2]);
            digest_role(digest, *program);
            digest.update((ancestors.len() as u64).to_be_bytes());
            for ancestor in ancestors {
                digest_role(digest, *ancestor);
            }
        }
        LaunchBindingSpec::ManualOnly { reason } => {
            digest.update([3]);
            digest_field(digest, reason.as_bytes());
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalLaunch {
    program: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
}

impl JournalLaunch {
    fn new(invocation: &Invocation, cwd: &Path) -> Self {
        Self {
            program: invocation.program.clone(),
            args: invocation.args.clone(),
            cwd: cwd.to_owned(),
        }
    }

    fn validate(&self, root: &Path) -> Result<(), Error> {
        if !self.program.is_absolute()
            || !self.cwd.is_absolute()
            || self.cwd != root
            || self.args.len() > 256
            || self.program.as_os_str().as_encoded_bytes().contains(&0)
            || self.args.iter().any(|argument| {
                argument.as_encoded_bytes().contains(&0)
                    || argument.as_encoded_bytes().len() > 32 * 1024
            })
        {
            return Err(Error::RecoveryRequired);
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Installation {
    source: Source,
    entry: PathBuf,
    stamp: Stamp,
    manager: Option<Stamp>,
    helper: Option<Stamp>,
    registration: Option<(PathBuf, Stamp)>,
    invocation: Option<BoundInvocation>,
    channel: Channel,
    config: Option<(PathBuf, ConfigKind)>,
    error: Option<Error>,
    source_target: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct UpdatePlan {
    agent: CLIAgent,
    installation: Installation,
    installed_version: String,
    target_version: String,
    config: Option<ConfigBackup>,
    intent: String,
}

impl UpdatePlan {
    pub(super) fn requires_native_update(&self) -> bool {
        self.installed_version != self.target_version
    }
}

pub(super) struct CheckReport {
    pub failed_target: Option<String>,
    pub installed_version: String,
    pub latest_version: String,
    pub source: Source,
    pub effective_channel: Channel,
    pub up_to_date: bool,
    pub error: Option<Error>,
    pub plan: Option<UpdatePlan>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PluginCompatibility {
    installed: bool,
    disabled: bool,
    needs_update: bool,
    platform_installed: bool,
    platform_needs_update: bool,
}

#[cfg(test)]
impl PluginCompatibility {
    fn verified(self) -> bool {
        self.installed
            && !self.disabled
            && !self.needs_update
            && self.platform_installed
            && !self.platform_needs_update
    }
}

#[cfg(test)]
fn plugin_compatibility(agent: CLIAgent) -> Option<PluginCompatibility> {
    let manager = plugin_manager_for(agent)?;
    Some(PluginCompatibility {
        installed: manager.is_installed(),
        disabled: manager.is_disabled(),
        needs_update: manager.needs_update(),
        platform_installed: manager.is_platform_plugin_installed(),
        platform_needs_update: manager.platform_plugin_needs_update(),
    })
}

pub(super) async fn inspect(
    agent: CLIAgent,
    executable: Option<PathBuf>,
    channel: Channel,
    client: &Arc<http_client::Client>,
) -> Result<CheckReport, Error> {
    let entry = executable.ok_or(Error::NotInstalled)?;
    let journal_root = journal_root()?;
    // 检查与恢复共用同一把锁，先确认旧更新已退出，再启动任何版本/来源探测。
    let _lock = lock_journal(&journal_root, agent)?;
    // npm 单包交换可能先于收据落盘；必须先按目录身份收敛，再读取 CLI 版本。
    #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
    npm_transaction::recover(agent, &entry, &journal_root)?;
    let pending = preflight_recovery(agent, &entry, &journal_root)?;
    let installed_version = version(agent, &entry).await?;
    if pending {
        // 恢复配置后再判断渠道，不能把认领期间的暂时缺失误判为默认渠道。
        reconcile_journal_locked(agent, &entry, &installed_version, &journal_root)?;
    }
    let mut installation = discover(agent, entry, &installed_version, channel).await?;
    if !cfg!(feature = "local_fs") {
        installation.invocation = None;
        installation.error = Some(Error::UnsupportedPlatform);
    }
    let target_version = latest(agent, installation.channel, client).await?;
    // 这里只开放宿主负责的已登记 npm 布局；ManualOnly 的 npm/Node 启动合同保持禁止执行。
    #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
    if installation.source == Source::Npm {
        installation.error = npm_transaction::supports(agent, &target_version).err();
    }
    let version_matches = installed_version == target_version;
    let mut error = installation.error;
    if installation
        .source_target
        .as_ref()
        .is_some_and(|version| version != &target_version)
    {
        error = Some(Error::ChannelMismatch);
    }
    if !version_matches
        && compare_versions(&installed_version, &target_version)? == std::cmp::Ordering::Greater
        && channel == Channel::FollowInstallation
    {
        error = Some(Error::ChannelMismatch);
    }
    let config = snapshot_config(&installation, &installed_version, &target_version, channel)?;
    if agent == CLIAgent::Claude
        && (source_is_native_claude(&installation) || installation.source == Source::Npm)
        && !version_matches
    {
        let compatibility = config
            .as_ref()
            .ok_or(Error::UnsupportedSource)
            .and_then(|config| {
                snapshot_claude_scope(
                    config,
                    Uuid::new_v4(),
                    claude_metadata_path(config)?,
                    &target_version,
                )
            });
        if let Err(reason) = compatibility {
            error = Some(reason);
        }
    }
    let up_to_date = version_matches
        && config
            .as_ref()
            .is_none_or(|config| config.publication_bytes(true) == config.before.as_ref());
    let source = installation.source;
    let effective_channel = installation.channel;
    let intent = update_intent(installation.channel, &target_version, config.as_ref())?;
    let failed_target = previous_failure_for_intent(
        &journal_root,
        agent,
        &installation.entry,
        &intent,
        channel == Channel::FollowInstallation,
    )?;
    let plan =
        (!up_to_date && error.is_none() && installation.invocation.is_some()).then(|| UpdatePlan {
            agent,
            installation,
            installed_version: installed_version.clone(),
            target_version: target_version.clone(),
            config,
            intent,
        });
    Ok(CheckReport {
        failed_target,
        installed_version,
        latest_version: target_version,
        source,
        effective_channel,
        up_to_date,
        error,
        plan,
    })
}

async fn version(agent: CLIAgent, entry: &Path) -> Result<String, Error> {
    version_with_timeout(agent, entry, PROBE_TIMEOUT).await
}

async fn version_with_timeout(
    agent: CLIAgent,
    entry: &Path,
    timeout: Duration,
) -> Result<String, Error> {
    let bytes = run(&Invocation::new(entry, ["--version"]), timeout).await?;
    let output = String::from_utf8(bytes).map_err(|_| Error::ProbeFailed)?;
    let version = parse_cli_agent_version(agent, &output).ok_or(Error::ProbeFailed)?;
    parse_version(&version)?;
    Ok(version)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Version {
    core: [u64; 3],
    alpha: Option<Vec<u64>>,
}

fn parse_version(version: &str) -> Result<Version, Error> {
    if version.len() > 64 {
        return Err(Error::InvalidRelease);
    }
    let (core, alpha) = match version.split_once("-alpha") {
        Some((core, suffix)) => {
            let parts = if suffix.is_empty() {
                Vec::new()
            } else {
                suffix
                    .strip_prefix('.')
                    .ok_or(Error::InvalidRelease)?
                    .split('.')
                    .map(version_number)
                    .collect::<Result<Vec<_>, _>>()?
            };
            if parts.len() > 2 {
                return Err(Error::InvalidRelease);
            }
            (core, Some(parts))
        }
        None => (version, None),
    };
    let components = core
        .split('.')
        .map(version_number)
        .collect::<Result<Vec<_>, _>>()?;
    let core = components.try_into().map_err(|_| Error::InvalidRelease)?;
    Ok(Version { core, alpha })
}

fn version_number(part: &str) -> Result<u64, Error> {
    if part.is_empty()
        || !part.bytes().all(|byte| byte.is_ascii_digit())
        || (part.len() > 1 && part.starts_with('0'))
    {
        return Err(Error::InvalidRelease);
    }
    part.parse().map_err(|_| Error::InvalidRelease)
}

fn compare_versions(left: &str, right: &str) -> Result<std::cmp::Ordering, Error> {
    let left = parse_version(left)?;
    let right = parse_version(right)?;
    let core = left.core.cmp(&right.core);
    if core != std::cmp::Ordering::Equal {
        return Ok(core);
    }
    Ok(match (left.alpha, right.alpha) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(left), Some(right)) => left.cmp(&right),
    })
}

async fn latest(
    agent: CLIAgent,
    channel: Channel,
    client: &http_client::Client,
) -> Result<String, Error> {
    #[cfg(all(
        test,
        feature = "local_fs",
        any(target_os = "macos", target_os = "linux")
    ))]
    if let Some(version) = npm_transaction::fixed_test_release(agent, channel) {
        return Ok(version);
    }

    #[cfg(all(test, unix))]
    if let Some(version) = live_tests::fixed_release(agent, channel) {
        return Ok(version);
    }
    #[cfg(all(test, windows))]
    if let Some(version) = windows_live_tests::fixed_release(agent, channel) {
        return Ok(version);
    }
    let (url, package) = match agent {
        CLIAgent::Codex if channel == Channel::Alpha => (
            "https://registry.npmjs.org/@openai/codex/alpha",
            Some("@openai/codex"),
        ),
        CLIAgent::Codex => (
            "https://registry.npmjs.org/@openai/codex/latest",
            Some("@openai/codex"),
        ),
        CLIAgent::Claude if channel == Channel::Stable => (
            "https://registry.npmjs.org/@anthropic-ai/claude-code/stable",
            Some("@anthropic-ai/claude-code"),
        ),
        CLIAgent::Claude => (
            "https://registry.npmjs.org/@anthropic-ai/claude-code/latest",
            Some("@anthropic-ai/claude-code"),
        ),
        CLIAgent::Grok if channel == Channel::Alpha => ("https://x.ai/cli/alpha", None),
        CLIAgent::Grok => ("https://x.ai/cli/stable", None),
        _ => return Err(Error::UnsupportedSource),
    };
    let response = client
        .get(url)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() || response.url().as_str() != url {
        return Err(Error::Network);
    }
    let mut bytes = Vec::new();
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Network)?;
        if bytes.len() + chunk.len() > MAX_OUTPUT as usize {
            return Err(Error::InvalidRelease);
        }
        bytes.extend_from_slice(&chunk);
    }
    let version = if let Some(package) = package {
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidRelease)?;
        if value.get("name").and_then(Value::as_str) != Some(package) {
            return Err(Error::InvalidRelease);
        }
        value
            .get("version")
            .and_then(Value::as_str)
            .ok_or(Error::InvalidRelease)?
            .to_owned()
    } else {
        String::from_utf8(bytes)
            .map_err(|_| Error::InvalidRelease)?
            .trim()
            .to_owned()
    };
    parse_version(&version)?;
    Ok(version)
}

// 官方 HTTPS 响应是此处的目标来源信任边界；这不是发布者签名校验。
// 摘要由应用独立流式计算，不能接受原生更新子进程提供的摘要或任意 URL。
#[cfg(windows)]
async fn grok_windows_child_image(
    version: &str,
) -> Result<managed_process::WindowsChildImage, Error> {
    parse_version(version)?;
    if !version
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err(Error::InvalidRelease);
    }
    #[cfg(test)]
    if let Some(image) = windows_live_tests::fixed_child_image(CLIAgent::Grok, version) {
        return Ok(image);
    }
    let url = format!("https://x.ai/cli/grok-{version}-windows-x86_64.exe");
    let client = http_client::Client::new();
    let response = client
        .get(&url)
        .timeout(UPDATE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() || response.url().as_str() != url {
        return Err(Error::Network);
    }
    let mut size = 0_u64;
    let mut digest = Sha256::new();
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Network)?;
        size = size
            .checked_add(chunk.len() as u64)
            .ok_or(Error::InvalidRelease)?;
        if size > 1024 * 1024 * 1024 {
            return Err(Error::InvalidRelease);
        }
        digest.update(&chunk);
    }
    if size == 0 {
        return Err(Error::InvalidRelease);
    }
    Ok(managed_process::WindowsChildImage {
        size,
        sha256: format!("{:x}", digest.finalize()),
    })
}

#[cfg(any(windows, test))]
fn codex_windows_asset_identity(
    release: &Value,
    version: &str,
    architecture: &str,
) -> Result<(u64, String), Error> {
    parse_version(version)?;
    let target = match architecture {
        "x86_64" => "x86_64-pc-windows-msvc",
        "aarch64" => "aarch64-pc-windows-msvc",
        _ => return Err(Error::UnsupportedPlatform),
    };
    let tag = format!("rust-v{version}");
    if release["tag_name"].as_str() != Some(tag.as_str())
        || release["draft"].as_bool() != Some(false)
    {
        return Err(Error::InvalidRelease);
    }
    let name = format!("codex-{target}.exe");
    let url = format!("https://github.com/openai/codex/releases/download/{tag}/{name}");
    let mut assets = release["assets"]
        .as_array()
        .ok_or(Error::InvalidRelease)?
        .iter()
        .filter(|asset| asset["name"].as_str() == Some(name.as_str()));
    let asset = assets.next().ok_or(Error::InvalidRelease)?;
    if assets.next().is_some() || asset["browser_download_url"].as_str() != Some(url.as_str()) {
        return Err(Error::InvalidRelease);
    }
    let size = asset["size"].as_u64().ok_or(Error::InvalidRelease)?;
    let digest = asset["digest"]
        .as_str()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or(Error::InvalidRelease)?;
    if size == 0 || size > 1024 * 1024 * 1024 {
        return Err(Error::InvalidRelease);
    }
    Ok((size, digest.to_owned()))
}

#[cfg(windows)]
async fn codex_windows_child_image(
    version: &str,
) -> Result<managed_process::WindowsChildImage, Error> {
    parse_version(version)?;
    #[cfg(test)]
    if let Some(image) = windows_live_tests::fixed_child_image(CLIAgent::Codex, version) {
        return Ok(image);
    }
    // 只信任官方仓库精确 tag 的直接 EXE 摘要；不接受更新子进程或重定向来源。
    let url = format!("https://api.github.com/repos/openai/codex/releases/tags/rust-v{version}");
    let response = http_client::Client::new()
        .get(&url)
        .header("User-Agent", "InfiniShell-CLI-updater")
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() || response.url().as_str() != url {
        return Err(Error::Network);
    }
    let mut bytes = Vec::new();
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Network)?;
        if bytes.len() + chunk.len() > MAX_OUTPUT as usize {
            return Err(Error::InvalidRelease);
        }
        bytes.extend_from_slice(&chunk);
    }
    let release = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidRelease)?;
    let (size, sha256) = codex_windows_asset_identity(&release, version, std::env::consts::ARCH)?;
    Ok(managed_process::WindowsChildImage { size, sha256 })
}

async fn discover(
    agent: CLIAgent,
    entry: PathBuf,
    installed: &str,
    requested: Channel,
) -> Result<Installation, Error> {
    if !channel_supported(agent, requested) {
        return Err(Error::ChannelMismatch);
    }
    let entry_for_stamp = entry.clone();
    let stamp = blocking::unblock(move || stamp(&entry_for_stamp)).await?;
    let mut installation = Installation {
        source: Source::Unknown,
        entry,
        stamp,
        manager: None,
        helper: None,
        registration: None,
        invocation: None,
        channel: Channel::Latest,
        config: None,
        error: Some(Error::UnsupportedSource),
        source_target: None,
    };
    if let Some(home) = user_home() {
        match agent {
            CLIAgent::Codex => {
                let codex_home = absolute_env("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
                let releases = codex_home.join("packages/standalone/releases");
                let current = codex_home.join("packages/standalone/current");
                if same_tree(&installation.stamp.canonical, &releases)
                    && same_tree(&installation.stamp.canonical, &current)
                    && !installation.entry.starts_with(&codex_home)
                    && installation.entry.file_name()
                        == Some(OsStr::new(if cfg!(windows) {
                            "codex.exe"
                        } else {
                            "codex"
                        }))
                {
                    installation.source = Source::Native;
                    installation.channel = if requested == Channel::FollowInstallation {
                        if parse_version(installed)?.alpha.is_some() {
                            Channel::Alpha
                        } else {
                            Channel::Latest
                        }
                    } else {
                        requested
                    };
                    let parent = installation
                        .entry
                        .parent()
                        .ok_or(Error::UnsupportedSource)?;
                    // 官方安装器在入口目录已处于 PATH 时不修改 shell profile。
                    let mut paths = vec![parent.to_path_buf()];
                    paths.extend(std::env::split_paths(
                        &std::env::var_os("PATH").unwrap_or_default(),
                    ));
                    let mut invocation = BoundInvocation {
                        artifacts: [
                            (ArtifactRole::Program, installation.stamp.canonical.clone()),
                            (ArtifactRole::Entry, installation.entry.clone()),
                            (ArtifactRole::InstallRoot, parent.to_path_buf()),
                            (ArtifactRole::ConfigRoot, codex_home.clone()),
                        ]
                        .into_iter()
                        .collect(),
                        program: ArtifactRole::Program,
                        arguments: vec![ArgumentRef::Literal("update".into())],
                        environment: Vec::new(),
                        env_remove: Vec::new(),
                        binding: LaunchBindingSpec::NativeFile {
                            program: ArtifactRole::Program,
                        },
                    };
                    invocation.env_remove.extend(
                        [
                            "CODEX_MANAGED_BY_NPM",
                            "CODEX_MANAGED_BY_BUN",
                            "CODEX_MANAGED_BY_PNPM",
                            "CODEX_MANAGED_BY_VITE_PLUS",
                        ]
                        .map(OsString::from),
                    );
                    invocation.environment.extend([
                        (
                            "PATH".into(),
                            ArgumentRef::PathList(
                                paths
                                    .into_iter()
                                    .map(|path| ArgumentRef::Literal(path.into_os_string()))
                                    .collect(),
                            ),
                        ),
                        (
                            "CODEX_INSTALL_DIR".into(),
                            ArgumentRef::ArtifactPath {
                                role: ArtifactRole::InstallRoot,
                                relative: PathBuf::new(),
                            },
                        ),
                        (
                            "CODEX_HOME".into(),
                            ArgumentRef::ArtifactPath {
                                role: ArtifactRole::ConfigRoot,
                                relative: PathBuf::new(),
                            },
                        ),
                        (
                            "CODEX_NON_INTERACTIVE".into(),
                            ArgumentRef::Literal("1".into()),
                        ),
                        ("CODEX_RELEASE".into(), ArgumentRef::TargetVersion),
                    ]);
                    installation.config = Some((
                        codex_home.join("packages/standalone/auto-update-version"),
                        ConfigKind::CodexUpdateMarker,
                    ));
                    installation.invocation = Some(invocation);
                    installation.error = None;
                    return Ok(installation);
                }
            }
            CLIAgent::Claude => {
                let versions = home.join(".local/share/claude/versions");
                #[cfg(windows)]
                let copied = windows_native_copy(
                    &installation,
                    &home.join(".local/bin/claude.exe"),
                    &versions.join(installed),
                );
                #[cfg(windows)]
                let native_layout = copied.is_some();
                #[cfg(not(windows))]
                let native_layout = same_tree(&installation.stamp.canonical, &versions);
                if native_layout {
                    #[cfg(windows)]
                    {
                        installation.manager = copied;
                    }
                    installation.source = Source::Native;
                    let config_dir =
                        absolute_env("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"));
                    let config_path = config_dir.join("settings.json");
                    let current_channel = claude_channel(&config_path)?;
                    installation.channel = if requested == Channel::FollowInstallation {
                        current_channel
                    } else {
                        requested
                    };
                    let selected = match installation.channel {
                        Channel::Stable => "stable",
                        Channel::FollowInstallation | Channel::Latest | Channel::Alpha => "latest",
                    };
                    let settings = format!("{{\"autoUpdatesChannel\":\"{selected}\"}}");
                    let invocation = BoundInvocation {
                        artifacts: [
                            (
                                ArtifactRole::Program,
                                installation
                                    .manager
                                    .as_ref()
                                    .unwrap_or(&installation.stamp)
                                    .canonical
                                    .clone(),
                            ),
                            (ArtifactRole::Entry, installation.entry.clone()),
                            (ArtifactRole::ConfigRoot, config_dir),
                        ]
                        .into_iter()
                        .collect(),
                        program: ArtifactRole::Program,
                        arguments: vec![
                            ArgumentRef::Literal("--settings".into()),
                            ArgumentRef::Literal(settings.into()),
                            ArgumentRef::Literal("install".into()),
                            ArgumentRef::TargetVersion,
                        ],
                        environment: Vec::new(),
                        env_remove: Vec::new(),
                        binding: LaunchBindingSpec::NativeFile {
                            program: ArtifactRole::Program,
                        },
                    };
                    installation.invocation = Some(invocation);
                    installation.config = Some((config_path, ConfigKind::Claude));
                    installation.error = None;
                    return Ok(installation);
                }
            }
            CLIAgent::Grok => {
                let grok_home = absolute_env("GROK_HOME").unwrap_or_else(|| home.join(".grok"));
                #[cfg(windows)]
                let copied = windows_grok_native_copy(&installation, &grok_home, installed);
                #[cfg(windows)]
                let native_layout = cfg!(target_arch = "x86_64") && copied.is_some();
                #[cfg(not(windows))]
                let native_layout =
                    same_tree(&installation.stamp.canonical, &grok_home.join("downloads"))
                        && installation
                            .entry
                            .parent()
                            .and_then(|parent| parent.canonicalize().ok())
                            == grok_home.join("bin").canonicalize().ok();
                if native_layout {
                    #[cfg(windows)]
                    {
                        installation.manager = copied;
                    }
                    installation.source = Source::Native;
                    let check = run(
                        &Invocation::new(&installation.entry, ["update", "--check", "--json"]),
                        PROBE_TIMEOUT,
                    )
                    .await?;
                    let current_channel = validate_grok_check(&check, installed)?;
                    installation.channel = if requested == Channel::FollowInstallation {
                        current_channel
                    } else {
                        requested
                    };
                    let invocation = BoundInvocation {
                        artifacts: [
                            (
                                ArtifactRole::Program,
                                installation
                                    .manager
                                    .as_ref()
                                    .unwrap_or(&installation.stamp)
                                    .canonical
                                    .clone(),
                            ),
                            (ArtifactRole::Entry, installation.entry.clone()),
                            (ArtifactRole::ConfigRoot, grok_home.clone()),
                        ]
                        .into_iter()
                        .collect(),
                        program: ArtifactRole::Program,
                        arguments: vec![
                            ArgumentRef::Literal("update".into()),
                            ArgumentRef::Literal("--version".into()),
                            ArgumentRef::TargetVersion,
                        ],
                        environment: Vec::new(),
                        env_remove: Vec::new(),
                        binding: LaunchBindingSpec::NativeFile {
                            program: ArtifactRole::Program,
                        },
                    };
                    installation.invocation = Some(invocation);
                    installation.config = Some((grok_home.join("config.toml"), ConfigKind::Grok));
                    installation.error = None;
                    return Ok(installation);
                }
            }
            _ => return Err(Error::UnsupportedSource),
        }
    }
    if let Some(npm) = discover_npm(agent, &installation, installed, requested).await? {
        return Ok(npm);
    }
    if cfg!(target_os = "macos")
        && let Some(brew) = discover_brew(agent, &installation, installed, requested).await?
    {
        return Ok(brew);
    }
    // WinGet 包记录与当前入口尚未形成可靠的一一对应，不能因 winget 在 PATH 中就更新别的安装。
    Ok(installation)
}

// Windows 原生安装器复制可见入口；只接受固定布局中与版本缓存逐字节一致的文件。
// 执行缓存原件，使可见入口可由原生安装器替换；二者会在最终派生前再次核对。
#[cfg(any(windows, test))]
fn windows_native_copy(
    installation: &Installation,
    entry: &Path,
    reference: &Path,
) -> Option<Stamp> {
    plain_ancestors(entry).ok()?;
    plain_ancestors(reference).ok()?;
    if installation.entry != entry || installation.stamp.canonical != entry.canonicalize().ok()? {
        return None;
    }
    let reference = stamp(reference).ok()?;
    (reference.digest == installation.stamp.digest
        && reference.canonical != installation.stamp.canonical)
        .then_some(reference)
}

#[cfg(any(windows, test))]
fn windows_grok_native_copy(
    installation: &Installation,
    grok_home: &Path,
    installed: &str,
) -> Option<Stamp> {
    // 官方 update 的缓存名没有 .exe；同时保留此前已识别的带扩展名安装布局。
    // 两种布局均须位于固定目录，并与可见入口逐字节一致。
    ["", ".exe"].into_iter().find_map(|suffix| {
        windows_native_copy(
            installation,
            &grok_home.join("bin/grok.exe"),
            &grok_home
                .join("downloads")
                .join(format!("grok-{installed}-windows-x86_64{suffix}")),
        )
    })
}

fn channel_supported(agent: CLIAgent, channel: Channel) -> bool {
    match agent {
        CLIAgent::Codex => matches!(
            channel,
            Channel::FollowInstallation | Channel::Latest | Channel::Alpha
        ),
        CLIAgent::Claude => matches!(
            channel,
            Channel::FollowInstallation | Channel::Latest | Channel::Stable
        ),
        CLIAgent::Grok => matches!(
            channel,
            Channel::FollowInstallation | Channel::Stable | Channel::Alpha
        ),
        _ => false,
    }
}

fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn user_home() -> Option<PathBuf> {
    absolute_env(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
}

fn same_tree(path: &Path, root: &Path) -> bool {
    root.canonicalize().is_ok_and(|root| path.starts_with(root))
}

fn path_program(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .filter(|path| path.is_absolute())
        .map(|path| path.join(name))
        .find(|path| path.is_file())
}

fn claude_channel(path: &Path) -> Result<Channel, Error> {
    if !path.try_exists().map_err(|_| Error::ProbeFailed)? {
        return Ok(Channel::Latest);
    }
    let value: Value =
        serde_json::from_slice(&read_limited(path, MAX_CONFIG)?).map_err(|_| Error::ProbeFailed)?;
    let object = value.as_object().ok_or(Error::ProbeFailed)?;
    match object.get("autoUpdatesChannel") {
        None => Ok(Channel::Latest),
        Some(Value::String(channel)) if channel == "latest" => Ok(Channel::Latest),
        Some(Value::String(channel)) if channel == "stable" => Ok(Channel::Stable),
        Some(_) => Err(Error::ChannelMismatch),
    }
}

fn source_is_native_claude(installation: &Installation) -> bool {
    installation.source == Source::Native
        && installation
            .config
            .as_ref()
            .is_some_and(|(_, kind)| matches!(kind, ConfigKind::Claude))
}

fn validate_grok_check(bytes: &[u8], installed: &str) -> Result<Channel, Error> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| Error::ProbeFailed)?;
    if value.get("currentVersion").and_then(Value::as_str) != Some(installed)
        || value.get("installer").and_then(Value::as_str) != Some("internal")
        || !value.get("error").is_some_and(Value::is_null)
    {
        return Err(Error::UnsupportedSource);
    }
    match value.get("channel").and_then(Value::as_str) {
        Some("stable") => Ok(Channel::Stable),
        Some("alpha") => Ok(Channel::Alpha),
        _ => Err(Error::ChannelMismatch),
    }
}

async fn discover_npm(
    agent: CLIAgent,
    installation: &Installation,
    installed: &str,
    requested: Channel,
) -> Result<Option<Installation>, Error> {
    let Some(package) = npm::package_name(agent) else {
        return Ok(None);
    };
    let Some(node) = path_program(if cfg!(windows) { "node.exe" } else { "node" }) else {
        return Ok(None);
    };
    let npm_cli = if cfg!(windows) {
        node.parent()
            .map(|parent| parent.join("node_modules/npm/bin/npm-cli.js"))
    } else {
        path_program("npm").and_then(|path| path.canonicalize().ok())
    };
    let Some(npm_cli) = npm_cli.filter(|path| path.is_file()) else {
        return Ok(None);
    };
    // 包清单必须声明当前 npm 入口；同名脚本不能充当管理器注册证明。
    if !npm::registered_manager(&npm_cli)? {
        return Ok(None);
    }
    let prefix_bytes = match run(
        &Invocation::new(
            &node,
            [
                npm_cli.as_os_str(),
                OsStr::new("prefix"),
                OsStr::new("--global"),
            ],
        ),
        PROBE_TIMEOUT,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(_) => return Ok(None),
    };
    let prefix = PathBuf::from(
        std::str::from_utf8(&prefix_bytes)
            .map_err(|_| Error::ProbeFailed)?
            .trim(),
    );
    if !prefix.is_absolute() {
        return Ok(None);
    }
    let Some(registration) =
        npm::registered_installation(agent, installation, installed, &prefix, cfg!(windows))?
    else {
        return Ok(None);
    };
    let mut found = installation.clone();
    found.source = Source::Npm;
    found.channel = match requested {
        Channel::Stable | Channel::Alpha | Channel::Latest => requested,
        Channel::FollowInstallation => {
            if agent == CLIAgent::Codex && installed.contains("-alpha") {
                Channel::Alpha
            } else {
                Channel::Latest
            }
        }
    };
    let node_identity = stamp(&node)?;
    let npm_identity = stamp(&npm_cli)?;
    let node = node_identity.canonical.clone();
    let npm_cli = npm_identity.canonical.clone();
    found.manager = Some(node_identity);
    found.helper = Some(npm_identity);
    found.registration = Some((registration.manifest.clone(), registration.stamp));
    found.invocation = Some(BoundInvocation::manual_only(
        [
            (ArtifactRole::Program, node.clone()),
            (ArtifactRole::Entry, installation.entry.clone()),
            (ArtifactRole::Manager, node),
            (ArtifactRole::Helper, npm_cli),
            (ArtifactRole::Registration, registration.manifest),
            (ArtifactRole::InstallRoot, registration.prefix),
            (ArtifactRole::DependencyRoot, registration.package_root),
        ],
        ArtifactRole::Program,
        vec![
            ArgumentRef::ArtifactPath {
                role: ArtifactRole::Helper,
                relative: PathBuf::new(),
            },
            ArgumentRef::Literal("install".into()),
            ArgumentRef::Literal("--global".into()),
            ArgumentRef::Literal("--prefix".into()),
            ArgumentRef::ArtifactPath {
                role: ArtifactRole::InstallRoot,
                relative: PathBuf::new(),
            },
            ArgumentRef::PackageVersion {
                package: package.to_owned(),
            },
            ArgumentRef::Literal("--registry=https://registry.npmjs.org/".into()),
            ArgumentRef::Literal("--no-audit".into()),
            ArgumentRef::Literal("--no-fund".into()),
        ],
        "npm 的 Node、依赖树、lifecycle script 与外部工具闭包尚未冻结",
    ));
    found.error = Some(Error::UnsupportedSource);
    if agent == CLIAgent::Claude {
        if requested == Channel::FollowInstallation {
            let home = user_home().ok_or(Error::UnsupportedSource)?;
            let config = absolute_env("CLAUDE_CONFIG_DIR")
                .unwrap_or_else(|| home.join(".claude"))
                .join("settings.json");
            found.channel = claude_channel(&config)?;
        }
        let home = user_home().ok_or(Error::UnsupportedSource)?;
        let config = absolute_env("CLAUDE_CONFIG_DIR")
            .unwrap_or_else(|| home.join(".claude"))
            .join("settings.json");
        found.config = Some((config, ConfigKind::Claude));
        // npm 本身仍不可派生；宿主单包事务另受相同的缓存版本约束与渠道配置检查。
    }
    Ok(Some(found))
}

async fn discover_brew(
    agent: CLIAgent,
    installation: &Installation,
    installed: &str,
    requested: Channel,
) -> Result<Option<Installation>, Error> {
    let casks: &[&str] = match agent {
        CLIAgent::Codex => &["codex"],
        CLIAgent::Claude => &["claude-code", "claude-code@latest"],
        _ => return Ok(None),
    };
    for base in ["/opt/homebrew", "/usr/local"] {
        let brew = Path::new(base).join("bin/brew");
        if !brew.is_file() {
            continue;
        }
        for cask in casks {
            let cask_root = Path::new(base).join("Caskroom").join(cask).join(installed);
            if !same_tree(&installation.stamp.canonical, &cask_root) {
                continue;
            }
            let bytes = run(
                &Invocation::new(&brew, ["info", "--json=v2", "--cask", cask]),
                PROBE_TIMEOUT,
            )
            .await?;
            let info: Value = serde_json::from_slice(&bytes).map_err(|_| Error::ProbeFailed)?;
            let metadata = info
                .get("casks")
                .and_then(Value::as_array)
                .filter(|items| items.len() == 1)
                .and_then(|items| items.first())
                .ok_or(Error::UnsupportedSource)?;
            if metadata.get("token").and_then(Value::as_str) != Some(cask)
                || metadata.get("tap").and_then(Value::as_str) != Some("homebrew/cask")
                || !metadata
                    .get("installed")
                    .and_then(Value::as_array)
                    .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(installed)))
            {
                return Err(Error::UnsupportedSource);
            }
            let mut found = installation.clone();
            found.source = Source::Homebrew;
            let manager = stamp(&brew)?;
            let brew = manager.canonical.clone();
            found.manager = Some(manager);
            found.source_target = metadata
                .get("version")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if found.source_target.is_none() {
                return Err(Error::InvalidRelease);
            }
            found.channel = if *cask == "claude-code" {
                Channel::Stable
            } else {
                Channel::Latest
            };
            if requested != Channel::FollowInstallation && requested != found.channel {
                found.error = Some(Error::ChannelMismatch);
                return Ok(Some(found));
            }
            let mut invocation = BoundInvocation::manual_only(
                [
                    (ArtifactRole::Program, brew.clone()),
                    (ArtifactRole::Entry, installation.entry.clone()),
                    (ArtifactRole::Manager, brew),
                    (ArtifactRole::InstallRoot, PathBuf::from(base)),
                ],
                ArtifactRole::Program,
                ["upgrade", "--cask", "--greedy", cask]
                    .into_iter()
                    .map(|argument| ArgumentRef::Literal(argument.into()))
                    .collect(),
                "Homebrew 根据自身 pathname 解析 repository/prefix，只允许手动更新",
            );
            invocation.environment.extend([
                (
                    "HOMEBREW_NO_AUTO_UPDATE".into(),
                    ArgumentRef::Literal("1".into()),
                ),
                (
                    "HOMEBREW_NO_INSTALL_CLEANUP".into(),
                    ArgumentRef::Literal("1".into()),
                ),
            ]);
            found.invocation = Some(invocation);
            found.error = Some(Error::UnsupportedSource);
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, Error> {
    let metadata = fs::symlink_metadata(path).map_err(|_| Error::ProbeFailed)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(Error::SourceChanged);
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| Error::ProbeFailed)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::ProbeFailed)?;
    if bytes.len() as u64 > limit {
        return Err(Error::SourceChanged);
    }
    Ok(bytes)
}

fn stamp(path: &Path) -> Result<Stamp, Error> {
    let canonical = path.canonicalize().map_err(|_| Error::SourceChanged)?;
    let mut file = File::open(&canonical).map_err(|_| Error::PermissionDenied)?;
    let metadata = file.metadata().map_err(|_| Error::SourceChanged)?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 * 1024 {
        return Err(Error::SourceChanged);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| Error::SourceChanged)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if file.metadata().map_err(|_| Error::SourceChanged)?.len() != metadata.len() {
        return Err(Error::SourceChanged);
    }
    Ok(Stamp {
        canonical,
        digest: digest.finalize().into(),
    })
}

fn verify_installation_identity(installation: &Installation) -> Result<(), Error> {
    if stamp(&installation.entry)? != installation.stamp {
        return Err(Error::SourceChanged);
    }
    if let Some(manager) = &installation.manager
        && stamp(&manager.canonical)? != *manager
    {
        return Err(Error::SourceChanged);
    }
    if let Some(helper) = &installation.helper
        && stamp(&helper.canonical)? != *helper
    {
        return Err(Error::SourceChanged);
    }
    if let Some((path, expected)) = &installation.registration
        && stamp(path)? != *expected
    {
        return Err(Error::SourceChanged);
    }
    Ok(())
}

fn expected_invocation_stamp<'a>(
    installation: &'a Installation,
    invocation: &Invocation,
) -> Result<&'a Stamp, Error> {
    let canonical = invocation
        .program
        .canonicalize()
        .map_err(|_| Error::SourceChanged)?;
    if canonical == installation.stamp.canonical {
        return Ok(&installation.stamp);
    }
    if let Some(manager) = &installation.manager
        && canonical == manager.canonical
    {
        return Ok(manager);
    }
    Err(Error::SourceChanged)
}

fn supervised_dependency_paths(
    installation: &Installation,
    invocation: &Invocation,
) -> Result<Vec<PathBuf>, Error> {
    let mut canonical_paths = BTreeSet::new();
    let mut paths = Vec::new();
    let mut add = |path: &Path| -> Result<(), Error> {
        let canonical = path.canonicalize().map_err(|_| Error::SourceChanged)?;
        if canonical_paths.insert(canonical) {
            paths.push(path.to_owned());
        }
        Ok(())
    };

    // 实际 executable 必须优先保留原路径，不能被指向同一文件的入口或管理器别名替代。
    add(&invocation.program)?;
    add(&installation.entry)?;
    if let Some(manager) = &installation.manager {
        add(&manager.canonical)?;
    }
    if let Some(helper) = &installation.helper {
        add(&helper.canonical)?;
    }
    if let Some((path, _)) = &installation.registration {
        add(path)?;
    }
    Ok(paths)
}

fn capture_supervised_dependencies(
    installation: &Installation,
    invocation: &Invocation,
    binding: &managed_process::PreparedLaunchBinding,
) -> Result<(Stamp, Vec<managed_process::ExpectedFileIdentity>), Error> {
    // 先捕获，再对原始发现结果复核；两者之间发生的任何替换都会由本地复核或最终 worker 拒绝。
    let dependency_paths = if binding.is_native_file() {
        vec![invocation.program.clone()]
    } else {
        supervised_dependency_paths(installation, invocation)?
    };
    let expected_files = dependency_paths
        .into_iter()
        .map(|path| {
            managed_process::ExpectedFileIdentity::capture(&path).map_err(|_| Error::SourceChanged)
        })
        .collect::<Result<Vec<_>, _>>()?;
    verify_installation_identity(installation)?;
    let expected_program = expected_invocation_stamp(installation, invocation)?.clone();
    Ok((expected_program, expected_files))
}

#[derive(Debug)]
enum RunFailure {
    NotStarted(Error),
    Started(Error),
}

async fn run(invocation: &Invocation, timeout: Duration) -> Result<Vec<u8>, Error> {
    #[cfg(all(
        test,
        feature = "local_fs",
        any(target_os = "macos", target_os = "linux")
    ))]
    npm_transaction::audit_test_probe(invocation)?;

    run_process(invocation, timeout)
        .await
        .map_err(|failure| match failure {
            RunFailure::NotStarted(error) | RunFailure::Started(error) => error,
        })
}

async fn run_process(invocation: &Invocation, timeout: Duration) -> Result<Vec<u8>, RunFailure> {
    let mut command = Command::new(&invocation.program);
    command
        .args(&invocation.args)
        .envs(invocation.env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for name in &invocation.env_remove {
        command.env_remove(name);
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            RunFailure::NotStarted(Error::PermissionDenied)
        } else {
            RunFailure::NotStarted(Error::CommandFailed)
        }
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or(RunFailure::Started(Error::CommandFailed))?;
    let result = async {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Error::CommandFailed)?;
        if bytes.len() as u64 > MAX_OUTPUT {
            return Err(Error::CommandFailed);
        }
        if !child
            .status()
            .await
            .map_err(|_| Error::CommandFailed)?
            .success()
        {
            return Err(Error::CommandFailed);
        }
        Ok(bytes)
    }
    .with_timeout(timeout)
    .await;
    match result {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => {
            let _ = child.kill();
            let _ = child.status().with_timeout(Duration::from_secs(3)).await;
            Err(RunFailure::Started(error))
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.status().with_timeout(Duration::from_secs(3)).await;
            Err(RunFailure::Started(Error::TimedOut))
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum ConfigKind {
    Grok,
    Claude,
    CodexUpdateMarker,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigBackup {
    kind: ConfigKind,
    path: PathBuf,
    before: Option<Vec<u8>>,
    /// 原生命令返回时的内容；不得与预期最终内容混用。
    after: Option<Vec<u8>>,
    #[serde(default)]
    desired: Option<ConfigDesired>,
    #[serde(default)]
    before_mode: Option<u32>,
    /// 移走配置前先随 journal 持久化；该目录只存放本事务认领的原文件。
    #[serde(default)]
    restore_stage: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigDesired {
    bytes: Option<Vec<u8>>,
}

impl ConfigBackup {
    fn publication_bytes(&self, publish_desired: bool) -> Option<&Vec<u8>> {
        if publish_desired && let Some(desired) = &self.desired {
            desired.bytes.as_ref()
        } else {
            self.before.as_ref()
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    agent: String,
    entry: PathBuf,
    old_version: String,
    target_version: String,
    phase: String,
    config: Option<ConfigBackup>,
    channel: String,
    /// 只有目标版本与真实成功退出均已确认，或同版本配置事务，才发布明确选择的渠道。
    #[serde(default)]
    publish_desired: bool,
    #[serde(default)]
    command_failed: bool,
    #[serde(default)]
    intent: Option<String>,
    #[serde(default)]
    generation: Option<Uuid>,
    #[serde(default)]
    old_stamp: Option<Stamp>,
    #[serde(default)]
    claude_update: Option<ClaudeUpdateScope>,
    #[serde(default)]
    launch: Option<JournalLaunch>,
    #[serde(default)]
    binding_digest: Option<String>,
    #[serde(default)]
    binding_kind: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaudeUpdateScope {
    generation: Uuid,
    owner: Uuid,
    settings_path: PathBuf,
    originals: Vec<ClaudeUpdateFile>,
    metadata: Option<Vec<u8>>,
}

const CLAUDE_POLICY_FILES: [&str; 9] = [
    "remote-settings.json",
    "remote-settings.json.signature.json",
    "remote-settings.json.signature-iat.json",
    "remote-settings-helper-consent",
    "remote-settings-consent.json",
    "policy-limits.json",
    "policy-limits.json.signature.json",
    "policy-limits.json.signature-iat.json",
    "policy-limits.json.stamp.json",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaudeUpdateFile {
    name: String,
    path: PathBuf,
    digest: Option<[u8; 32]>,
    mode: Option<u32>,
    identity: Option<(u64, u64)>,
}

fn claude_update_metadata(before: Option<&[u8]>) -> Result<Option<Vec<u8>>, Error> {
    let Some(before) = before else {
        return Ok(None);
    };
    let value: Value = serde_json::from_slice(before).map_err(|_| Error::ProbeFailed)?;
    let original = value.as_object().ok_or(Error::ProbeFailed)?;
    let mut selected = serde_json::Map::new();
    // 固定原生更新器只读取这些安装/更新策略；不携带账户、项目历史或审批确认。
    for name in [
        "installMethod",
        "autoUpdates",
        "autoUpdatesProtectedForNative",
        "autoUpdaterStatus",
    ] {
        let Some(value) = original.get(name) else {
            continue;
        };
        let valid = match name {
            "installMethod" => matches!(
                value.as_str(),
                Some("native" | "local" | "global" | "unknown" | "development")
            ),
            "autoUpdates" | "autoUpdatesProtectedForNative" => value.is_boolean(),
            "autoUpdaterStatus" => {
                matches!(value.as_str(), Some("migrated" | "disabled" | "enabled"))
            }
            _ => false,
        };
        if !valid {
            return Err(Error::ProbeFailed);
        }
        selected.insert(name.to_owned(), value.clone());
    }
    serde_json::to_vec(&selected)
        .map(Some)
        .map_err(|_| Error::ProbeFailed)
}

fn claude_cached_update_constraints(bytes: Option<&[u8]>, target: &str) -> Result<(), Error> {
    let Some(bytes) = bytes else {
        return Ok(());
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|_| Error::ProbeFailed)?;
    let settings = value.as_object().ok_or(Error::ProbeFailed)?;
    if ["policyHelper", "policyHelpers"]
        .into_iter()
        .any(|name| settings.get(name).is_some_and(|value| !value.is_null()))
    {
        return Err(Error::UnsupportedSource);
    }
    // 缓存资格可能随隔离身份改变；这些限制在宿主侧取交集，绝不依赖原生忽略缓存。
    for (name, forbidden) in [
        ("minimumVersion", std::cmp::Ordering::Less),
        ("requiredMinimumVersion", std::cmp::Ordering::Less),
        ("requiredMaximumVersion", std::cmp::Ordering::Greater),
    ] {
        if let Some(value) = settings.get(name) {
            let boundary = value.as_str().ok_or(Error::ProbeFailed)?;
            if compare_versions(target, boundary)? == forbidden {
                return Err(Error::PermissionDenied);
            }
        }
    }
    if let Some(environment) = settings.get("env") {
        let environment = environment.as_object().ok_or(Error::ProbeFailed)?;
        for (name, value) in environment {
            if ["CLAUDE_CONFIG_DIR", "HOME", "USERPROFILE"]
                .into_iter()
                .any(|reserved| name.eq_ignore_ascii_case(reserved))
            {
                return Err(Error::UnsupportedSource);
            }
            if name.eq_ignore_ascii_case("DISABLE_UPDATES") {
                if !matches!(value.as_str(), Some("" | "0" | "false")) {
                    return Err(Error::PermissionDenied);
                }
            }
        }
    }
    Ok(())
}

fn snapshot_claude_scope(
    config: &ConfigBackup,
    generation: Uuid,
    metadata_path: PathBuf,
    target: &str,
) -> Result<ClaudeUpdateScope, Error> {
    if !matches!(config.kind, ConfigKind::Claude) {
        return Err(Error::UnsupportedSource);
    }
    claude_cached_update_constraints(config.before.as_deref(), target)?;
    let parent = config.path.parent().ok_or(Error::UnsupportedSource)?;
    let name = metadata_path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or(Error::UnsupportedSource)?
        .to_owned();
    let (metadata_file, metadata) = read_claude_update_file(&name, metadata_path)?;
    let mut originals = vec![metadata_file];
    for name in CLAUDE_POLICY_FILES {
        let (original, bytes) = read_claude_update_file(name, parent.join(name))?;
        if name == "remote-settings.json" {
            claude_cached_update_constraints(bytes.as_deref(), target)?;
        }
        originals.push(original);
    }
    let scope = ClaudeUpdateScope {
        generation,
        owner: Uuid::new_v4(),
        settings_path: config.path.clone(),
        originals,
        metadata: claude_update_metadata(metadata.as_deref())?,
    };
    validate_claude_scope(&scope)?;
    verify_claude_originals(&scope)?;
    Ok(scope)
}

fn claude_metadata_path(config: &ConfigBackup) -> Result<PathBuf, Error> {
    let home = user_home().ok_or(Error::UnsupportedSource)?;
    let override_dir = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(value) if !value.is_empty() && Path::new(&value).is_absolute() => {
            Some(PathBuf::from(value))
        }
        Some(_) => return Err(Error::UnsupportedSource),
        None => None,
    };
    let config_dir = override_dir.clone().unwrap_or_else(|| home.join(".claude"));
    if config.path != config_dir.join("settings.json") {
        return Err(Error::SourceChanged);
    }
    let legacy = config_dir.join(".config.json");
    claude_plain_ancestors(&legacy)?;
    if legacy.try_exists().map_err(|_| Error::ProbeFailed)? {
        return Ok(legacy);
    }
    let name = if std::env::var_os("CLAUDE_CODE_CUSTOM_OAUTH_URL")
        .is_some_and(|value| !value.is_empty())
    {
        ".claude-custom-oauth.json"
    } else {
        ".claude.json"
    };
    Ok(override_dir.unwrap_or(home).join(name))
}

fn validate_claude_scope(scope: &ClaudeUpdateScope) -> Result<(), Error> {
    let parent = scope
        .settings_path
        .parent()
        .ok_or(Error::RecoveryRequired)?;
    if scope.generation.is_nil()
        || scope.owner.is_nil()
        || !scope.settings_path.is_absolute()
        || scope.settings_path.file_name() != Some(OsStr::new("settings.json"))
        || scope.originals.len() != CLAUDE_POLICY_FILES.len() + 1
        || claude_update_metadata(scope.metadata.as_deref())? != scope.metadata
    {
        return Err(Error::RecoveryRequired);
    }
    let metadata = &scope.originals[0];
    let metadata_parent = metadata.path.parent().ok_or(Error::RecoveryRequired)?;
    if !matches!(
        metadata.name.as_str(),
        ".claude.json" | ".claude-custom-oauth.json" | ".config.json"
    ) || metadata.path.file_name() != Some(OsStr::new(&metadata.name))
        || !metadata.path.is_absolute()
        || metadata_parent != parent
            && (metadata.name == ".config.json"
                || parent.file_name() != Some(OsStr::new(".claude"))
                || Some(metadata_parent) != parent.parent())
    {
        return Err(Error::RecoveryRequired);
    }
    for (original, expected) in scope.originals[1..].iter().zip(CLAUDE_POLICY_FILES) {
        if original.name != expected || original.path != parent.join(expected) {
            return Err(Error::RecoveryRequired);
        }
    }
    Ok(())
}

fn read_claude_update_file(
    name: &str,
    path: PathBuf,
) -> Result<(ClaudeUpdateFile, Option<Vec<u8>>), Error> {
    claude_plain_ancestors(&path)?;
    let mut record = ClaudeUpdateFile {
        name: name.to_owned(),
        path,
        digest: None,
        mode: None,
        identity: None,
    };
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    let file = match options.open(&record.path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((record, None)),
        Err(_) => return Err(Error::SourceChanged),
    };
    let metadata = file.metadata().map_err(|_| Error::SourceChanged)?;
    if !metadata.is_file() || metadata.len() > MAX_CONFIG {
        return Err(Error::SourceChanged);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.nlink() != 1 {
            return Err(Error::SourceChanged);
        }
        record.mode = Some(metadata.mode() & 0o777);
        record.identity = Some((metadata.dev(), metadata.ino()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle as _;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| Error::SourceChanged)?;
        if info.nNumberOfLinks != 1 || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(Error::SourceChanged);
        }
        record.mode = Some(info.dwFileAttributes);
        record.identity = Some((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::SourceChanged)?;
    if bytes.len() as u64 > MAX_CONFIG {
        return Err(Error::SourceChanged);
    }
    record.digest = Some(Sha256::digest(&bytes).into());
    Ok((record, Some(bytes)))
}

fn claude_plain_ancestors(path: &Path) -> Result<(), Error> {
    plain_ancestors(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        for ancestor in path.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata)
                    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 =>
                {
                    return Err(Error::SourceChanged);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::SourceChanged),
            }
        }
    }
    Ok(())
}

fn verify_claude_originals(scope: &ClaudeUpdateScope) -> Result<(), Error> {
    validate_claude_scope(scope)?;
    for (index, original) in scope.originals.iter().enumerate() {
        let (actual, bytes) = read_claude_update_file(&original.name, original.path.clone())?;
        if actual != *original
            || index == 0 && claude_update_metadata(bytes.as_deref())? != scope.metadata
        {
            return Err(Error::SourceChanged);
        }
    }
    Ok(())
}

fn claude_scope_directory(root: &Path, scope: &ClaudeUpdateScope) -> Result<PathBuf, Error> {
    validate_claude_scope(scope)?;
    let root = root.canonicalize().map_err(|_| Error::RecoveryRequired)?;
    let directory = root.join(format!("claude-update-{}", scope.generation));
    claude_plain_ancestors(&directory)?;
    Ok(directory)
}

fn claude_scope_owner(scope: &ClaudeUpdateScope) -> Vec<u8> {
    format!("claude-update-v1\n{}\n{}\n", scope.generation, scope.owner).into_bytes()
}

fn write_claude_private_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| Error::RecoveryRequired)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| Error::RecoveryRequired)
}

fn prepare_claude_scope(
    root: &Path,
    scope: &ClaudeUpdateScope,
    config: &ConfigBackup,
) -> Result<PathBuf, Error> {
    if !matches!(config.kind, ConfigKind::Claude) || config.path != scope.settings_path {
        return Err(Error::RecoveryRequired);
    }
    verify_claude_originals(scope)?;
    if read_optional_config(&config.path)? != config.before {
        return Err(Error::SourceChanged);
    }
    let directory = claude_scope_directory(root, scope)?;
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    // 父目录独占创建；即使已有同名目录也不认领、不清空。
    builder
        .create(&directory)
        .map_err(|_| Error::RecoveryRequired)?;
    write_claude_private_file(&directory.join("owner"), &claude_scope_owner(scope))?;
    let shadow = directory.join("config");
    builder
        .create(&shadow)
        .map_err(|_| Error::RecoveryRequired)?;
    if let Some(before) = &config.before {
        write_claude_private_file(&shadow.join("settings.json"), before)?;
    }
    for (index, original) in scope.originals.iter().enumerate() {
        let bytes = if index == 0 {
            scope.metadata.clone()
        } else {
            let (actual, bytes) = read_claude_update_file(&original.name, original.path.clone())?;
            if actual != *original {
                return Err(Error::SourceChanged);
            }
            bytes
        };
        if let Some(bytes) = bytes {
            write_claude_private_file(&shadow.join(&original.name), &bytes)?;
        }
    }
    verify_claude_originals(scope)?;
    sync_config_directory(&shadow)?;
    sync_config_directory(&directory)?;
    sync_config_directory(&root.canonicalize().map_err(|_| Error::RecoveryRequired)?)?;
    Ok(shadow)
}

fn cleanup_claude_scope(root: &Path, scope: &ClaudeUpdateScope) -> Result<(), Error> {
    let directory = claude_scope_directory(root, scope)?;
    if !directory
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        return Ok(());
    }
    let (owner, bytes) = read_claude_update_file("owner", directory.join("owner"))?;
    if bytes.as_deref() != Some(claude_scope_owner(scope).as_slice()) {
        return Err(Error::RecoveryRequired);
    }
    #[cfg(unix)]
    if owner.mode != Some(0o600) {
        return Err(Error::RecoveryRequired);
    }
    #[cfg(not(unix))]
    let _ = owner;
    for entry in fs::read_dir(&directory).map_err(|_| Error::RecoveryRequired)? {
        let entry = entry.map_err(|_| Error::RecoveryRequired)?;
        if entry.file_name() != "owner" && entry.file_name() != "config" {
            return Err(Error::RecoveryRequired);
        }
    }
    let shadow = directory.join("config");
    claude_plain_ancestors(&shadow)?;
    match fs::symlink_metadata(&shadow) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            if shadow.canonicalize().map_err(|_| Error::RecoveryRequired)? != shadow {
                return Err(Error::RecoveryRequired);
            }
            fs::remove_dir_all(&shadow).map_err(|_| Error::RecoveryRequired)?;
            sync_config_directory(&directory)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) | Err(_) => return Err(Error::RecoveryRequired),
    }
    // 仅保留无配置正文的所有权回执；后续恢复可反复清理，不必猜测空目录属于谁。
    Ok(())
}

fn validate_claude_journal(journal: &Journal) -> Result<(), Error> {
    let scope = journal
        .claude_update
        .as_ref()
        .ok_or(Error::RecoveryRequired)?;
    validate_claude_scope(scope)?;
    if journal.agent != "claude"
        || journal.generation != Some(scope.generation)
        || journal.config.as_ref().is_none_or(|config| {
            !matches!(config.kind, ConfigKind::Claude) || config.path != scope.settings_path
        })
        || !matches!(
            journal.phase.as_str(),
            "claude_config_preparing" | "prepared" | "command_returned" | "verified"
        )
    {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

fn finish_claude_scope(root: &Path, journal: &Journal) -> Result<(), Error> {
    let Some(scope) = &journal.claude_update else {
        return Ok(());
    };
    validate_claude_journal(journal)?;
    #[cfg(feature = "local_fs")]
    managed_process::confirmed_exit_with_binding(
        root,
        scope.generation,
        &journal_binding(journal)?,
    )
    .map_err(|_| Error::RecoveryRequired)?
    .ok_or(Error::RecoveryRequired)?;
    #[cfg(not(feature = "local_fs"))]
    return Err(Error::RecoveryRequired);
    let unchanged = verify_claude_originals(scope);
    // 已有退出回执时先清除副本；即使用户同时改配置导致 CAS 失败，也不遗留副本正文。
    cleanup_claude_scope(root, scope)?;
    unchanged.map_err(|_| Error::RecoveryRequired)
}

fn channel_name(channel: Channel) -> Result<&'static str, Error> {
    match channel {
        Channel::Latest => Ok("latest"),
        Channel::Stable => Ok("stable"),
        Channel::Alpha => Ok("alpha"),
        Channel::FollowInstallation => Err(Error::ChannelMismatch),
    }
}

fn snapshot_config(
    installation: &Installation,
    installed: &str,
    target: &str,
    requested: Channel,
) -> Result<Option<ConfigBackup>, Error> {
    let Some((path, kind)) = &installation.config else {
        return Ok(None);
    };
    plain_ancestors(path)?;
    let before = read_optional_config(path)?;
    let desired = match kind {
        ConfigKind::Grok | ConfigKind::Claude => selected_channel_config(
            *kind,
            before.as_deref(),
            (requested != Channel::FollowInstallation).then_some(installation.channel),
        )?,
        ConfigKind::CodexUpdateMarker => codex_marker_config(
            path,
            before.as_deref(),
            &installation.stamp.canonical,
            installed,
            target,
        )?,
    };
    #[cfg(unix)]
    let before_mode = {
        use std::os::unix::fs::PermissionsExt as _;
        if before.is_some() {
            Some(
                fs::metadata(path)
                    .map_err(|_| Error::ProbeFailed)?
                    .permissions()
                    .mode()
                    & 0o777,
            )
        } else {
            None
        }
    };
    #[cfg(not(unix))]
    let before_mode = None;
    Ok(Some(ConfigBackup {
        kind: *kind,
        path: path.clone(),
        before,
        after: None,
        desired: Some(ConfigDesired { bytes: desired }),
        before_mode,
        restore_stage: None,
    }))
}

fn selected_channel_config(
    kind: ConfigKind,
    before: Option<&[u8]>,
    requested: Option<Channel>,
) -> Result<Option<Vec<u8>>, Error> {
    let Some(requested) = requested else {
        return Ok(before.map(<[u8]>::to_vec));
    };
    let selected = channel_name(requested)?;
    match kind {
        ConfigKind::Grok => {
            if !matches!(requested, Channel::Stable | Channel::Alpha) {
                return Err(Error::ChannelMismatch);
            }
            let text =
                std::str::from_utf8(before.unwrap_or_default()).map_err(|_| Error::ProbeFailed)?;
            let mut document = text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| Error::ProbeFailed)?;
            if let Some(cli) = document.get("cli") {
                if !cli.is_table_like() {
                    return Err(Error::ProbeFailed);
                }
                if let Some(channel) = cli.get("channel") {
                    if !matches!(channel.as_str(), Some("stable" | "alpha")) {
                        return Err(Error::ProbeFailed);
                    }
                }
            }
            let current = document
                .get("cli")
                .and_then(|cli| cli.get("channel"))
                .and_then(toml_edit::Item::as_str)
                .unwrap_or("stable");
            if current == selected {
                return Ok(before.map(<[u8]>::to_vec));
            }
            document["cli"]["channel"] = toml_edit::value(selected);
            Ok(Some(document.to_string().into_bytes()))
        }
        ConfigKind::Claude => {
            if !matches!(requested, Channel::Latest | Channel::Stable) {
                return Err(Error::ChannelMismatch);
            }
            let mut document = match before {
                Some(bytes) => {
                    serde_json::from_slice::<Value>(bytes).map_err(|_| Error::ProbeFailed)?
                }
                None => serde_json::json!({}),
            };
            let object = document.as_object_mut().ok_or(Error::ProbeFailed)?;
            if let Some(current) = object.get("autoUpdatesChannel") {
                if !matches!(current.as_str(), Some("latest" | "stable")) {
                    return Err(Error::ProbeFailed);
                }
            }
            if object
                .get("autoUpdatesChannel")
                .and_then(Value::as_str)
                .unwrap_or("latest")
                == selected
            {
                return Ok(before.map(<[u8]>::to_vec));
            }
            object.insert(
                "autoUpdatesChannel".to_owned(),
                Value::String(selected.to_owned()),
            );
            serde_json::to_vec_pretty(&document)
                .map(Some)
                .map_err(|_| Error::ProbeFailed)
        }
        ConfigKind::CodexUpdateMarker => Err(Error::UnsupportedSource),
    }
}

fn codex_marker_config(
    marker: &Path,
    before: Option<&[u8]>,
    executable: &Path,
    installed: &str,
    target: &str,
) -> Result<Option<Vec<u8>>, Error> {
    let standalone = marker.parent().ok_or(Error::UnsupportedSource)?;
    let releases = standalone
        .join("releases")
        .canonicalize()
        .map_err(|_| Error::SourceChanged)?;
    let current = standalone
        .join("current")
        .canonicalize()
        .map_err(|_| Error::SourceChanged)?;
    let release = current
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or(Error::SourceChanged)?;
    let platform = [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-musl",
        "aarch64-pc-windows-msvc",
        "x86_64-pc-windows-msvc",
    ]
    .into_iter()
    .find(|platform| release == format!("{installed}-{platform}"))
    .ok_or(Error::SourceChanged)?;
    if current.parent() != Some(releases.as_path()) || !executable.starts_with(&current) {
        return Err(Error::SourceChanged);
    }
    let target_release = format!("{target}-{platform}");
    let enabled = parse_version(installed)?.alpha.is_none() && before == Some(release.as_bytes());
    if parse_version(target)?.alpha.is_some() {
        // 官方 alpha 没有原生后台更新渠道；不留下可在反向切换时意外生效的旧标记。
        Ok(None)
    } else if enabled {
        Ok(Some(target_release.into_bytes()))
    } else if before == Some(target_release.as_bytes()) {
        // 旧目录不匹配的标记属于关闭状态，不能因目标目录恰巧相同而重新开启。
        Ok(None)
    } else {
        Ok(before.map(<[u8]>::to_vec))
    }
}

fn validate_codex_publication(journal: &Journal) -> Result<(), Error> {
    let Some(config) = &journal.config else {
        return Ok(());
    };
    if !journal.publish_desired || !matches!(config.kind, ConfigKind::CodexUpdateMarker) {
        return Ok(());
    }
    let Some(desired) = config.publication_bytes(true) else {
        return Ok(());
    };
    if Some(desired) == config.before.as_ref() {
        return Ok(());
    }
    let standalone = config.path.parent().ok_or(Error::RecoveryRequired)?;
    let current = standalone
        .join("current")
        .canonicalize()
        .map_err(|_| Error::RecoveryRequired)?;
    let releases = standalone
        .join("releases")
        .canonicalize()
        .map_err(|_| Error::RecoveryRequired)?;
    if current.parent() != Some(releases.as_path())
        || current
            .file_name()
            .and_then(OsStr::to_str)
            .map(str::as_bytes)
            != Some(desired.as_slice())
        || !stamp(&journal.entry)?.canonical.starts_with(current)
        || parse_version(&journal.target_version)?.alpha.is_some()
    {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

fn journal_root() -> Result<PathBuf, Error> {
    #[cfg(all(test, windows))]
    if let Some(root) = windows_live_tests::journal_root() {
        return Ok(root);
    }
    let root = warp_core::paths::secure_state_dir().unwrap_or_else(warp_core::paths::state_dir);
    if !root.is_absolute() {
        return Err(Error::PersistenceFailed);
    }
    Ok(root.join("cli-agent-updates"))
}

fn plain_ancestors(path: &Path) -> Result<(), Error> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::PersistenceFailed);
            }
            Ok(metadata) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt as _;

                    if !windows_file_attributes_are_plain(metadata.file_attributes()) {
                        return Err(Error::PersistenceFailed);
                    }
                }
                #[cfg(not(windows))]
                let _ = metadata;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::PersistenceFailed),
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn windows_file_attributes_are_plain(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

fn lock_journal(root: &Path, agent: CLIAgent) -> Result<File, Error> {
    plain_ancestors(root)?;
    fs::create_dir_all(root).map_err(|_| Error::PersistenceFailed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))
            .map_err(|_| Error::PersistenceFailed)?;
    }
    let path = root.join(format!("{}.lock", agent.command_prefix()));
    plain_ancestors(&path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| Error::PersistenceFailed)?;
    file.try_lock().map_err(|_| Error::RecoveryRequired)?;
    Ok(file)
}

fn save_journal(path: &Path, journal: &Journal) -> Result<(), Error> {
    plain_ancestors(path)?;
    let parent = path.parent().ok_or(Error::PersistenceFailed)?;
    let bytes = serde_json::to_vec(journal).map_err(|_| Error::PersistenceFailed)?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|_| Error::PersistenceFailed)?;
    temporary
        .write_all(&bytes)
        .map_err(|_| Error::PersistenceFailed)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    temporary
        .persist(path)
        .map_err(|_| Error::PersistenceFailed)?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| Error::PersistenceFailed)?;
    Ok(())
}

#[cfg(feature = "local_fs")]
fn record_not_started_if_missing(
    root: &Path,
    generation: Uuid,
    program: &Path,
    args: &[OsString],
    binding: &managed_process::PreparedLaunchBinding,
) -> Result<bool, Error> {
    let generation_missing = !root
        .join("cli-agent-processes")
        .join(generation.to_string())
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?;
    if generation_missing {
        managed_process::record_not_started_with_binding(
            root, generation, program, args, root, binding,
        )
        .map_err(|_| Error::RecoveryRequired)?;
    }
    Ok(generation_missing)
}

fn journal_binding(journal: &Journal) -> Result<managed_process::PreparedLaunchBinding, Error> {
    let digest = journal
        .binding_digest
        .as_ref()
        .ok_or(Error::RecoveryRequired)?;
    managed_process::PreparedLaunchBinding::from_persisted(
        digest.clone(),
        journal.binding_kind.as_deref(),
    )
    .map_err(|_| Error::RecoveryRequired)
}

fn preflight_recovery(agent: CLIAgent, entry: &Path, root: &Path) -> Result<bool, Error> {
    let path = root.join(format!("{}.json", agent.command_prefix()));
    if !path.try_exists().map_err(|_| Error::PersistenceFailed)? {
        return Ok(false);
    }
    let journal: Journal = serde_json::from_slice(&read_limited(&path, 12 * MAX_CONFIG)?)
        .map_err(|_| Error::RecoveryRequired)?;
    if journal.schema != 2 || journal.agent != agent.command_prefix() || journal.entry != entry {
        return Err(Error::RecoveryRequired);
    }
    if let Some(generation) = journal.generation {
        #[cfg(feature = "local_fs")]
        {
            if matches!(
                journal.phase.as_str(),
                "prepared" | "claude_config_preparing"
            ) {
                if journal.phase == "claude_config_preparing"
                    && (agent != CLIAgent::Claude || journal.claude_update.is_none())
                {
                    return Err(Error::RecoveryRequired);
                }
                if journal.claude_update.is_some() {
                    validate_claude_journal(&journal)?;
                }
                let launch = journal.launch.as_ref().ok_or(Error::RecoveryRequired)?;
                launch.validate(root)?;
                let binding = journal_binding(&journal)?;
                // journal 锁证明旧派生方已退出；尚无代次账本时永久认领“未启动”，
                // 让三款 CLI 都能恢复派生前崩溃，而不是永远停在 RecoveryRequired。
                record_not_started_if_missing(
                    root,
                    generation,
                    &launch.program,
                    &launch.args,
                    &binding,
                )?;
            }
            let binding = journal_binding(&journal)?;
            managed_process::confirmed_exit_with_binding(root, generation, &binding)
                .map_err(|_| Error::RecoveryRequired)?
                .ok_or(Error::RecoveryRequired)?;
            finish_claude_scope(root, &journal)?;
        }
        #[cfg(not(feature = "local_fs"))]
        {
            let _ = generation;
            return Err(Error::RecoveryRequired);
        }
    } else if !matches!(
        journal.phase.as_str(),
        "config_prepared" | "command_returned" | "verified"
    ) {
        return Err(Error::RecoveryRequired);
    }
    Ok(true)
}

#[cfg(test)]
fn reconcile_journal(
    agent: CLIAgent,
    entry: &Path,
    installed: &str,
    root: &Path,
) -> Result<(), Error> {
    let _lock = lock_journal(root, agent)?;
    reconcile_journal_locked(agent, entry, installed, root)
}

fn reconcile_journal_locked(
    agent: CLIAgent,
    entry: &Path,
    installed: &str,
    root: &Path,
) -> Result<(), Error> {
    let path = root.join(format!("{}.json", agent.command_prefix()));
    let mut journal: Journal = serde_json::from_slice(&read_limited(&path, 12 * MAX_CONFIG)?)
        .map_err(|_| Error::RecoveryRequired)?;
    if journal.schema != 2 || journal.agent != agent.command_prefix() || journal.entry != entry {
        return Err(Error::RecoveryRequired);
    }
    if journal.generation.is_some() {
        return reconcile_confirmed_update(&path, &mut journal, agent, root, installed);
    }
    if journal.target_version != installed {
        return Err(Error::RecoveryRequired);
    }
    if journal.phase == "config_prepared" || journal.publish_desired {
        if journal.old_version != journal.target_version
            || journal.old_stamp.as_ref() != Some(&stamp(entry)?)
            || journal
                .config
                .as_ref()
                .is_none_or(|config| config.desired.is_none())
        {
            return Err(Error::RecoveryRequired);
        }
    }
    if journal.phase == "config_prepared" {
        let config = journal.config.as_mut().ok_or(Error::RecoveryRequired)?;
        if config.after != config.before {
            return Err(Error::RecoveryRequired);
        }
        journal.publish_desired = true;
        journal.phase = "command_returned".to_owned();
        plan_config_publish(config, true)?;
        save_journal(&path, &journal)?;
    }
    validate_codex_publication(&journal)?;
    match journal.phase.as_str() {
        "verified" => {
            if let Some(config) = &journal.config {
                if read_optional_config(&config.path)?.as_ref()
                    != config.publication_bytes(journal.publish_desired)
                {
                    return Err(Error::RecoveryRequired);
                }
            }
        }
        "command_returned" => {
            if let Some(config) = journal.config.as_mut() {
                // 原生输出已记录；认领后的路径缺失不能覆盖这个比较基准。
                if config.before != config.after
                    && !config_delta_allowed(config, config.after.as_deref(), &journal.channel)
                {
                    return Err(Error::RecoveryRequired);
                }
                plan_config_publish(config, journal.publish_desired)?;
            }
            save_journal(&path, &journal)?;
            if let Some(config) = &journal.config {
                publish_config(config, journal.publish_desired)?;
            }
            journal.phase = "verified".to_owned();
            save_journal(&path, &journal)?;
        }
        _ => return Err(Error::RecoveryRequired),
    }
    if let Some(config) = &journal.config {
        cleanup_config_restore(config)?;
    }

    if journal.publish_desired {
        match fs::remove_file(failure_path(root, agent)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::RecoveryRequired),
        }
    }
    fs::remove_file(path).map_err(|_| Error::PersistenceFailed)
}

pub(super) async fn execute(
    plan: UpdatePlan,
    mut verification_progress: Option<VerificationProgress>,
) -> Result<String, Error> {
    // 阻止链接器丢弃供独立构建来源核验使用的只读字节。
    std::hint::black_box(&SUPERVISOR_UPDATER_SOURCE_BINDING);
    let root = journal_root()?;
    let _lock = lock_journal(&root, plan.agent)?;
    let journal_path = root.join(format!("{}.json", plan.agent.command_prefix()));
    if journal_path
        .try_exists()
        .map_err(|_| Error::PersistenceFailed)?
    {
        return Err(Error::RecoveryRequired);
    }
    let installation = &plan.installation;
    verify_installation_identity(installation)?;
    #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
    if installation.source == Source::Npm {
        // 调用方已取得原有忙碌/启动预约许可与本 agent journal 锁，不能旁路更新状态机。
        return npm_transaction::execute(&plan, &root, verification_progress).await;
    }
    if version(plan.agent, &installation.entry).await? != plan.installed_version {
        return Err(Error::SourceChanged);
    }
    let prepared = installation
        .invocation
        .as_ref()
        .ok_or(Error::UnsupportedSource)?
        .prepare(&plan.target_version)?;
    // 数据合同可先落地，但平台原子执行原语未实现时必须在任何事务副作用前拒绝。
    prepared
        .binding
        .validate_update_execution()
        .map_err(|_| Error::UnsupportedPlatform)?;
    let mut invocation = prepared.invocation;
    let binding = prepared.binding;
    #[cfg(windows)]
    let binding = if binding.is_native_file()
        && plan.requires_native_update()
        && matches!(plan.agent, CLIAgent::Codex | CLIAgent::Grok)
    {
        let image = match plan.agent {
            CLIAgent::Codex => codex_windows_child_image(&plan.target_version).await?,
            CLIAgent::Grok => grok_windows_child_image(&plan.target_version).await?,
            CLIAgent::Claude
            | CLIAgent::Gemini
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
            | CLIAgent::Unknown => return Err(Error::UnsupportedSource),
        };
        binding
            .with_child_image(image)
            .map_err(|_| Error::InvalidRelease)?
    } else {
        binding
    };
    let config = plan.config.clone();
    if let Some(config) = &config {
        if read_optional_config(&config.path)? != config.before {
            return Err(Error::SourceChanged);
        }
        #[cfg(unix)]
        if let Some(expected_mode) = config.before_mode {
            use std::os::unix::fs::PermissionsExt as _;
            if fs::metadata(&config.path)
                .map_err(|_| Error::SourceChanged)?
                .permissions()
                .mode()
                & 0o777
                != expected_mode
            {
                return Err(Error::SourceChanged);
            }
        }
    }
    let requires_native_update = plan.requires_native_update();
    let generation = requires_native_update.then(Uuid::new_v4);
    let claude_update = if requires_native_update && source_is_native_claude(installation) {
        let config = config.as_ref().ok_or(Error::UnsupportedSource)?;
        Some(snapshot_claude_scope(
            config,
            generation.unwrap(),
            claude_metadata_path(config)?,
            &plan.target_version,
        )?)
    } else {
        None
    };
    let mut journal = Journal {
        claude_update,
        schema: 2,
        agent: plan.agent.command_prefix().to_owned(),
        entry: installation.entry.clone(),
        old_version: plan.installed_version.clone(),
        target_version: plan.target_version.clone(),
        phase: "prepared".to_owned(),
        generation,
        old_stamp: Some(installation.stamp.clone()),
        config,
        publish_desired: false,
        command_failed: false,
        intent: Some(plan.intent.clone()),
        channel: channel_name(installation.channel)?.to_owned(),
        launch: generation.map(|_| JournalLaunch::new(&invocation, &root)),
        binding_digest: generation.map(|_| binding.digest().to_owned()),
        binding_kind: generation.and_then(|_| binding.kind_name().map(ToOwned::to_owned)),
    };
    if !requires_native_update {
        // 同版本只同步渠道，不调用原生安装器，也不伪造进程退出回执。
        journal.phase = "config_prepared".to_owned();
        if let Some(config) = journal.config.as_mut() {
            config.after = config.before.clone();
        }
        save_journal(&journal_path, &journal)?;
        if let Some(progress) = verification_progress.take() {
            progress.enter().await;
        }
        reconcile_journal_locked(
            plan.agent,
            &installation.entry,
            &plan.installed_version,
            &root,
        )?;
        return Ok(plan.target_version);
    }
    if journal.claude_update.is_some() {
        journal.phase = "claude_config_preparing".to_owned();
    }
    save_journal(&journal_path, &journal)?;
    if let Some(scope) = &journal.claude_update {
        let shadow = prepare_claude_scope(
            &root,
            scope,
            journal.config.as_ref().ok_or(Error::RecoveryRequired)?,
        );
        let shadow = match shadow {
            Ok(shadow) => shadow,
            Err(error) => {
                #[cfg(feature = "local_fs")]
                managed_process::record_not_started_with_binding(
                    &root,
                    scope.generation,
                    &invocation.program,
                    &invocation.args,
                    &root,
                    &binding,
                )
                .map_err(|_| Error::RecoveryRequired)?;
                journal.command_failed = true;
                save_journal(&journal_path, &journal)?;
                reconcile_confirmed_update(
                    &journal_path,
                    &mut journal,
                    plan.agent,
                    &root,
                    &plan.installed_version,
                )?;
                return Err(error);
            }
        };
        invocation
            .env
            .push(("CLAUDE_CONFIG_DIR".into(), shadow.into_os_string()));
        journal.phase = "prepared".to_owned();
        save_journal(&journal_path, &journal)?;
    }
    // 更新命令必须有完整监督回执；只读版本与发行检查仍使用轻量进程探测。
    // 配置准备可能耗时，因此在最终派生前再次复核全部来源，并把实际程序摘要带到监督调用边界。
    let (expected_program, expected_files) =
        match capture_supervised_dependencies(installation, &invocation, &binding) {
            Ok(expected) => expected,
            Err(error) => {
                #[cfg(feature = "local_fs")]
                if !record_not_started_if_missing(
                    &root,
                    journal.generation.unwrap(),
                    &invocation.program,
                    &invocation.args,
                    &binding,
                )? {
                    return Err(Error::RecoveryRequired);
                }
                if journal.claude_update.is_some() {
                    finish_claude_scope(&root, &journal)?;
                }
                fs::remove_file(&journal_path).map_err(|_| Error::RecoveryRequired)?;
                return Err(error);
            }
        };
    let failure = match run_supervised_update(
        &invocation,
        &expected_program,
        expected_files,
        &root,
        journal.generation.unwrap(),
        &binding,
    )
    .await
    {
        Ok(failure) => failure,
        Err(error) => {
            // 只有真实退出可确认时才能回收副本；未知存活状态继续由 journal 阻止重投。
            if error == Error::SourceChanged {
                #[cfg(feature = "local_fs")]
                if !record_not_started_if_missing(
                    &root,
                    journal.generation.unwrap(),
                    &invocation.program,
                    &invocation.args,
                    &binding,
                )? {
                    return Err(Error::RecoveryRequired);
                }
            }
            if journal.claude_update.is_some() {
                finish_claude_scope(&root, &journal)?;
            }
            // 最终身份复核发生在派生调用之前；此错误明确证明没有监督代次可恢复。
            if error == Error::SourceChanged {
                fs::remove_file(&journal_path).map_err(|_| Error::RecoveryRequired)?;
            }
            return Err(error);
        }
    };
    journal.command_failed = failure.is_some();
    save_journal(&journal_path, &journal)?;
    if let Some(progress) = verification_progress.take() {
        progress.enter().await;
    }
    finish_supervised_update(&journal_path, &mut journal, plan.agent, &root).await?;
    if let Some(error) = failure {
        return Err(error);
    }
    if version(plan.agent, &installation.entry).await? != plan.target_version {
        return Err(Error::VersionMismatch);
    }
    // 安装成功不授予托管或插件能力；各自入口继续按实际版本和协议核验。
    Ok(plan.target_version)
}

#[cfg(feature = "local_fs")]
async fn run_supervised_update(
    invocation: &Invocation,
    expected_program: &Stamp,
    expected_files: Vec<managed_process::ExpectedFileIdentity>,
    root: &Path,
    generation: Uuid,
    binding: &managed_process::PreparedLaunchBinding,
) -> Result<Option<Error>, Error> {
    let environment = ManagedEnvironment {
        values: invocation.env.clone(),
        remove: invocation.env_remove.clone(),
    };
    // 这是现有监督 API 前的最后一个同步步骤；不允许较早的检查结果跨过派生边界复用。
    if stamp(&invocation.program)? != *expected_program {
        return Err(Error::SourceChanged);
    }
    let child = managed_process::spawn_bound_update(
        root,
        generation,
        &invocation.program,
        &invocation.args,
        root,
        environment,
        expected_files,
        binding,
    )
    .await;
    let mut child = match child {
        Ok(child) => child,
        Err(_) => {
            let generation_directory = root
                .join("cli-agent-processes")
                .join(generation.to_string());
            if generation_directory.try_exists().ok() == Some(false) {
                // 启动函数只会在先写代次账本后派生监督者；账本未创建明确表示尚未派生。
                managed_process::record_not_started_with_binding(
                    root,
                    generation,
                    &invocation.program,
                    &invocation.args,
                    root,
                    binding,
                )
                .map_err(|_| Error::RecoveryRequired)?;
            }
            if managed_process::confirmed_exit_with_binding(root, generation, binding)
                .ok()
                .flatten()
                .is_some()
            {
                return Ok(Some(Error::CommandFailed));
            }
            return Err(Error::RecoveryRequired);
        }
    };
    let mut stdout = child.stdout.take().ok_or(Error::RecoveryRequired)?;
    let read = async {
        let mut bytes = Vec::new();
        let output_result = (&mut stdout)
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .await;
        #[cfg(all(test, unix))]
        live_tests::preserve_supervised_stdout(root, generation, &bytes);
        output_result.map_err(|_| Error::CommandFailed)?;
        if bytes.len() as u64 > MAX_OUTPUT {
            return Err(Error::CommandFailed);
        }
        Ok(())
    }
    .with_timeout(UPDATE_TIMEOUT)
    .await;
    drop(stdout);
    let failure = match read {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(_) => Some(Error::TimedOut),
    };
    // stdin在输出结束前保持打开，避免监督者把提前EOF视为停止指令。
    let receipt = if failure.is_some() {
        child.finish().await
    } else {
        child.finish_after_stdin_close().await
    }
    .map_err(|_| Error::RecoveryRequired)?;
    if !receipt.cleanup_confirmed {
        return Err(Error::RecoveryRequired);
    }
    Ok(failure.or((receipt.exit_code != Some(0)).then_some(Error::CommandFailed)))
}

#[cfg(not(feature = "local_fs"))]
async fn run_supervised_update(
    invocation: &Invocation,
    expected_program: &Stamp,
    expected_files: Vec<managed_process::ExpectedFileIdentity>,
    root: &Path,
    generation: Uuid,
    binding: &managed_process::PreparedLaunchBinding,
) -> Result<Option<Error>, Error> {
    let _ = (
        invocation,
        expected_program,
        expected_files,
        root,
        generation,
        binding,
    );
    Err(Error::UnsupportedPlatform)
}

async fn finish_supervised_update(
    journal_path: &Path,
    journal: &mut Journal,
    agent: CLIAgent,
    root: &Path,
) -> Result<(), Error> {
    finish_claude_scope(root, journal)?;
    let installed = version(agent, &journal.entry)
        .await
        .map_err(|_| Error::RecoveryRequired)?;
    reconcile_confirmed_update(journal_path, journal, agent, root, &installed)
}

fn reconcile_confirmed_update(
    journal_path: &Path,
    journal: &mut Journal,
    agent: CLIAgent,
    root: &Path,
    installed: &str,
) -> Result<(), Error> {
    finish_claude_scope(root, journal)?;
    #[cfg(feature = "local_fs")]
    let command_succeeded = {
        let generation = journal.generation.ok_or(Error::RecoveryRequired)?;
        let binding = journal_binding(journal)?;
        managed_process::confirmed_exit_with_binding(root, generation, &binding)
            .map_err(|_| Error::RecoveryRequired)?
            .ok_or(Error::RecoveryRequired)?
            .exit_code
            == Some(0)
    };
    #[cfg(not(feature = "local_fs"))]
    let command_succeeded: bool = {
        return Err(Error::RecoveryRequired);
    };

    let reached_target = installed == journal.target_version;
    if !reached_target {
        let original = journal.old_stamp.as_ref().ok_or(Error::RecoveryRequired)?;
        if installed != journal.old_version || stamp(&journal.entry)? != *original {
            return Err(Error::RecoveryRequired);
        }
    }
    let command_succeeded = command_succeeded && !journal.command_failed;
    let publish_desired = reached_target
        && command_succeeded
        && journal
            .config
            .as_ref()
            .is_some_and(|config| config.desired.is_some());
    if matches!(
        journal.phase.as_str(),
        "prepared" | "claude_config_preparing"
    ) {
        journal.publish_desired = publish_desired;
    } else if journal.publish_desired != publish_desired {
        return Err(Error::RecoveryRequired);
    }
    validate_codex_publication(journal)?;
    match journal.phase.as_str() {
        "prepared" | "claude_config_preparing" => {
            if let Some(config) = journal.config.as_mut() {
                let after =
                    read_optional_config(&config.path).map_err(|_| Error::RecoveryRequired)?;
                if after != config.before
                    && !config_delta_allowed(config, after.as_deref(), &journal.channel)
                {
                    return Err(Error::RecoveryRequired);
                }
                config.after = after;
                plan_config_publish(config, journal.publish_desired)?;
            }
            journal.phase = "command_returned".to_owned();
            save_journal(journal_path, journal).map_err(|_| Error::RecoveryRequired)?;
        }
        "command_returned" => {
            if let Some(config) = journal.config.as_mut() {
                if config.before != config.after
                    && !config_delta_allowed(config, config.after.as_deref(), &journal.channel)
                {
                    return Err(Error::RecoveryRequired);
                }
                plan_config_publish(config, journal.publish_desired)?;
            }
            save_journal(journal_path, journal).map_err(|_| Error::RecoveryRequired)?;
        }
        "verified" => {
            if let Some(config) = &journal.config {
                if read_optional_config(&config.path)?.as_ref()
                    != config.publication_bytes(journal.publish_desired)
                {
                    return Err(Error::RecoveryRequired);
                }
            }
        }
        _ => return Err(Error::RecoveryRequired),
    }
    if journal.phase != "verified" {
        if let Some(config) = &journal.config {
            publish_config(config, journal.publish_desired).map_err(|_| Error::RecoveryRequired)?;
        }
        journal.phase = "verified".to_owned();
        save_journal(journal_path, journal).map_err(|_| Error::RecoveryRequired)?;
    }
    if let Some(config) = &journal.config {
        cleanup_config_restore(config)?;
    }
    if reached_target && command_succeeded {
        let failed_path = failure_path(root, agent);
        match fs::remove_file(failed_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::RecoveryRequired),
        }
    } else {
        // 旧安装与配置已核对且后代已退出；持久失败目标避免应用重启后自动重投。
        save_failure_with_intent(
            root,
            agent,
            &journal.entry,
            &journal.target_version,
            journal.intent.clone(),
        )?;
    }
    fs::remove_file(journal_path).map_err(|_| Error::RecoveryRequired)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailedUpdate {
    entry: PathBuf,
    target: String,
    #[serde(default)]
    intent: Option<String>,
}

fn failure_path(root: &Path, agent: CLIAgent) -> PathBuf {
    root.join(format!("{}-failed.json", agent.command_prefix()))
}

fn save_failure_with_intent(
    root: &Path,
    agent: CLIAgent,
    entry: &Path,
    target: &str,
    intent: Option<String>,
) -> Result<(), Error> {
    let path = failure_path(root, agent);
    plain_ancestors(&path)?;
    let mut file = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    let bytes = serde_json::to_vec(&FailedUpdate {
        entry: entry.to_owned(),
        target: target.to_owned(),
        intent,
    })
    .map_err(|_| Error::PersistenceFailed)?;
    file.write_all(&bytes)
        .map_err(|_| Error::PersistenceFailed)?;
    file.as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    file.persist(path).map_err(|_| Error::PersistenceFailed)?;
    #[cfg(unix)]
    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| Error::PersistenceFailed)?;
    Ok(())
}

fn update_intent(
    channel: Channel,
    target: &str,
    config: Option<&ConfigBackup>,
) -> Result<String, Error> {
    let config_digest = config
        .and_then(|config| config.publication_bytes(true))
        .map(|bytes| hex::encode(Sha256::digest(bytes)));
    let bytes = serde_json::to_vec(&(channel_name(channel)?, target, config_digest))
        .map_err(|_| Error::PersistenceFailed)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn previous_failure_for_intent(
    root: &Path,
    agent: CLIAgent,
    entry: &Path,
    intent: &str,
    legacy_follow: bool,
) -> Result<Option<String>, Error> {
    let Some(bytes) = read_optional_config(&failure_path(root, agent))? else {
        return Ok(None);
    };
    let record: FailedUpdate =
        serde_json::from_slice(&bytes).map_err(|_| Error::PersistenceFailed)?;
    let same_intent = match record.intent {
        Some(previous) => previous == intent,
        None => legacy_follow,
    };
    Ok((record.entry == entry && same_intent).then_some(record.target))
}

#[cfg(test)]
fn previous_failure(root: &Path, agent: CLIAgent, entry: &Path) -> Result<Option<String>, Error> {
    previous_failure_for_intent(root, agent, entry, "", true)
}

fn rollback_unstarted(path: &Path, journal: &Journal, original: &Stamp) -> Result<(), Error> {
    if stamp(&journal.entry).map_err(|_| Error::RecoveryRequired)? != *original {
        return Err(Error::RecoveryRequired);
    }
    if let Some(config) = &journal.config {
        if read_optional_config(&config.path).map_err(|_| Error::RecoveryRequired)? != config.before
        {
            return Err(Error::RecoveryRequired);
        }
    }
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)
}

fn grok_config_delta_allowed(before: &[u8], after: &[u8], channel: &str) -> bool {
    let Some(mut before) = std::str::from_utf8(before)
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok())
    else {
        return false;
    };
    let Some(mut after) = std::str::from_utf8(after)
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok())
    else {
        return false;
    };
    let original_channel = before
        .get("cli")
        .and_then(|cli| cli.get("channel"))
        .cloned();
    if let Some(cli) = after.get_mut("cli").and_then(toml::Value::as_table_mut) {
        if let Some(actual) = cli.get("channel") {
            if actual.as_str() != Some(channel) {
                return false;
            }
            if let Some(original) = original_channel {
                cli.insert("channel".to_owned(), original);
            } else {
                cli.remove("channel");
            }
        }
        if before
            .get("cli")
            .and_then(|cli| cli.get("installer"))
            .is_none()
        {
            if cli
                .get("installer")
                .is_some_and(|value| value.as_str() != Some("internal"))
            {
                return false;
            }
            cli.remove("installer");
        }
    }
    for (name, allowed) in [
        ("max_thoughts_width", toml::Value::Integer(120)),
        (
            "fork_secondary_model",
            toml::Value::String("grok-4.6".to_owned()),
        ),
        ("yolo", toml::Value::Boolean(false)),
        ("compact_mode", toml::Value::Boolean(false)),
    ] {
        if before.get("ui").and_then(|ui| ui.get(name)).is_none() {
            if let Some(ui) = after.get_mut("ui").and_then(toml::Value::as_table_mut) {
                if ui.get(name).is_some_and(|actual| actual != &allowed) {
                    return false;
                }
                ui.remove(name);
            }
        }
    }
    for value in [&mut before, &mut after] {
        for name in ["cli", "ui"] {
            if value
                .get(name)
                .and_then(toml::Value::as_table)
                .is_some_and(|table| table.is_empty())
            {
                value
                    .as_table_mut()
                    .expect("已由子表确认根为表")
                    .remove(name);
            }
        }
    }
    before == after
}

fn config_delta_allowed(config: &ConfigBackup, after: Option<&[u8]>, channel: &str) -> bool {
    match config.kind {
        ConfigKind::Grok => after.is_some_and(|after| {
            grok_config_delta_allowed(config.before.as_deref().unwrap_or_default(), after, channel)
        }),
        ConfigKind::Claude => after == config.before.as_deref(),
        // 官方精确版本安装删除该标记；最终值按旧有效状态与新目录重建。
        ConfigKind::CodexUpdateMarker => after.is_none() || after == config.before.as_deref(),
    }
}

fn plan_config_publish(config: &mut ConfigBackup, publish_desired: bool) -> Result<(), Error> {
    if config.publication_bytes(publish_desired) != config.after.as_ref()
        && config.restore_stage.is_none()
    {
        let parent = config.path.parent().ok_or(Error::RecoveryRequired)?;
        config.restore_stage =
            Some(parent.join(format!(".infinishell-cli-update-{}", Uuid::new_v4())));
    }
    Ok(())
}

fn config_restore_stage(config: &ConfigBackup) -> Result<Option<&Path>, Error> {
    let Some(stage) = config.restore_stage.as_deref() else {
        return Ok(None);
    };
    if stage.parent() != config.path.parent()
        || stage
            .file_name()
            .and_then(OsStr::to_str)
            .and_then(|name| name.strip_prefix(".infinishell-cli-update-"))
            .is_none_or(|name| Uuid::parse_str(name).is_err())
    {
        return Err(Error::RecoveryRequired);
    }
    plain_ancestors(stage).map_err(|_| Error::RecoveryRequired)?;
    Ok(Some(stage))
}

fn sync_config_directory(path: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| Error::RecoveryRequired)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ConfigRestorePoint {
    BeforeClaim,
    AfterClaim,
    BeforePublish,
    AfterPublish,
}

#[cfg(test)]
fn restore_config(config: &ConfigBackup) -> Result<(), Error> {
    restore_config_with_hook(config, |_| Ok(()))
}

#[cfg(test)]
fn restore_config_with_hook(
    config: &ConfigBackup,
    checkpoint: impl FnMut(ConfigRestorePoint) -> Result<(), Error>,
) -> Result<(), Error> {
    publish_config_with_hook(config, false, checkpoint)
}

fn publish_config(config: &ConfigBackup, publish_desired: bool) -> Result<(), Error> {
    publish_config_with_hook(config, publish_desired, |_| Ok(()))
}

fn publish_config_with_hook(
    config: &ConfigBackup,
    publish_desired: bool,
    mut checkpoint: impl FnMut(ConfigRestorePoint) -> Result<(), Error>,
) -> Result<(), Error> {
    let desired = config.publication_bytes(publish_desired);
    if config.before_mode.is_some_and(|mode| mode > 0o777) {
        return Err(Error::RecoveryRequired);
    }
    plain_ancestors(&config.path)?;
    let stage = config_restore_stage(config)?;
    let current = read_optional_config(&config.path)?;
    if desired == config.after.as_ref() {
        let stage_absent = match stage {
            None => true,
            Some(stage) => !stage.try_exists().map_err(|_| Error::RecoveryRequired)?,
        };
        return if current.as_ref() == desired && stage_absent {
            Ok(())
        } else {
            Err(Error::RecoveryRequired)
        };
    }
    let stage = stage.ok_or(Error::RecoveryRequired)?;
    let parent = config.path.parent().ok_or(Error::RecoveryRequired)?;
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    if !parent.try_exists().map_err(|_| Error::RecoveryRequired)? {
        builder
            .recursive(true)
            .create(parent)
            .map_err(|_| Error::RecoveryRequired)?;
        sync_config_directory(parent)?;
        builder.recursive(false);
    }
    match builder.create(stage) {
        Ok(()) => sync_config_directory(parent)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !fs::symlink_metadata(stage)
                .map_err(|_| Error::RecoveryRequired)?
                .is_dir()
            {
                return Err(Error::RecoveryRequired);
            }
        }
        Err(_) => return Err(Error::RecoveryRequired),
    }
    let claimed = stage.join("claimed");
    let mut staged = read_optional_config(&claimed)?;
    if staged.is_none() && current.as_ref() == desired {
        // 原路径已恢复（或用户自行恢复相同字节）；不再认领和替换它。
        return Ok(());
    }
    if staged.is_none() && config.after.is_some() {
        if current != config.after {
            return Err(Error::RecoveryRequired);
        }
        checkpoint(ConfigRestorePoint::BeforeClaim)?;
        fs::rename(&config.path, &claimed).map_err(|_| Error::RecoveryRequired)?;
        sync_config_directory(parent)?;
        sync_config_directory(stage)?;
        staged = read_optional_config(&claimed)?;
        if staged != config.after {
            // 原子保存可能发生在检查之后；认领到的新文件绝不删除，原路径空闲时原样放回。
            let _ = fs::hard_link(&claimed, &config.path);
            sync_config_directory(parent)?;
            return Err(Error::RecoveryRequired);
        }
        checkpoint(ConfigRestorePoint::AfterClaim)?;
    }
    if staged != config.after {
        return Err(Error::RecoveryRequired);
    }
    let current = read_optional_config(&config.path)?;
    if current.as_ref() == desired {
        return Ok(());
    }
    if current.is_some() {
        return Err(Error::RecoveryRequired);
    }
    if let Some(bytes) = desired {
        let mut temporary = NamedTempFile::new_in(parent).map_err(|_| Error::RecoveryRequired)?;
        temporary
            .write_all(bytes)
            .map_err(|_| Error::RecoveryRequired)?;
        if let Ok(metadata) = fs::metadata(&claimed) {
            temporary
                .as_file()
                .set_permissions(metadata.permissions())
                .map_err(|_| Error::RecoveryRequired)?;
        }
        #[cfg(unix)]
        if let Some(mode) = config.before_mode {
            use std::os::unix::fs::PermissionsExt as _;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(mode))
                .map_err(|_| Error::RecoveryRequired)?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| Error::RecoveryRequired)?;
        checkpoint(ConfigRestorePoint::BeforePublish)?;
        // 认领后用户新保存的路径优先，禁止任何覆盖式 rename 或删除。
        temporary
            .persist_noclobber(&config.path)
            .map_err(|_| Error::RecoveryRequired)?;
        sync_config_directory(parent)?;
    }
    checkpoint(ConfigRestorePoint::AfterPublish)?;
    if read_optional_config(&config.path)?.as_ref() != desired {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

fn cleanup_config_restore(config: &ConfigBackup) -> Result<(), Error> {
    let Some(stage) = config_restore_stage(config)? else {
        return Ok(());
    };
    let claimed = stage.join("claimed");
    if let Some(bytes) = read_optional_config(&claimed)? {
        if Some(bytes) != config.after {
            return Err(Error::RecoveryRequired);
        }
        fs::remove_file(&claimed).map_err(|_| Error::RecoveryRequired)?;
        sync_config_directory(stage)?;
    }
    match fs::remove_dir(stage) {
        Ok(()) => sync_config_directory(config.path.parent().ok_or(Error::RecoveryRequired)?)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::RecoveryRequired),
    }
    Ok(())
}

fn read_optional_config(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_limited(path, MAX_CONFIG).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::RecoveryRequired),
    }
}

pub(super) fn recovery_pending(agent: CLIAgent) -> bool {
    let Ok(root) = journal_root() else {
        return true;
    };
    recovery_pending_in(&root, agent)
}

fn recovery_pending_in(root: &Path, agent: CLIAgent) -> bool {
    if plain_ancestors(root).is_err() {
        return true;
    }
    let npm_path = root.join(format!("{}-npm.json", agent.command_prefix()));
    match fs::symlink_metadata(npm_path) {
        Ok(_) => return true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return true,
    }
    let path = root.join(format!("{}.json", agent.command_prefix()));
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

#[cfg(test)]
pub(super) fn plan_for_test() -> UpdatePlan {
    let entry = PathBuf::from("synthetic-cli-entry");
    UpdatePlan {
        agent: CLIAgent::Claude,
        installation: Installation {
            source: Source::Native,
            entry: entry.clone(),
            stamp: Stamp {
                canonical: entry,
                digest: [0; 32],
            },
            manager: None,
            helper: None,
            registration: None,
            invocation: None,
            channel: Channel::Latest,
            config: None,
            error: None,
            source_target: None,
        },
        installed_version: "2.1.273".to_owned(),
        target_version: "2.1.278".to_owned(),
        config: None,
        intent: "synthetic-intent".to_owned(),
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sources_live_tests.rs"]
mod live_tests;

#[cfg(all(test, windows))]
#[path = "sources_windows_live_tests.rs"]
mod windows_live_tests;
