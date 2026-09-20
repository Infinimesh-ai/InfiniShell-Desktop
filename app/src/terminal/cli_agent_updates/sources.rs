//! 来源必须同时绑定当前入口和管理器记录；PATH 中存在管理器并不能证明 CLI 归它管理。

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "local_fs")]
use crate::ai::cli_agent_runtime::managed_process::{self, ManagedEnvironment};
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
use crate::terminal::cli_agent::{CLIAgent, CLIAgentVersionStatus, probe_cli_agent_version};

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);
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

#[derive(Clone, Debug)]
struct Installation {
    source: Source,
    entry: PathBuf,
    stamp: Stamp,
    manager: Option<Stamp>,
    helper: Option<Stamp>,
    registration: Option<(PathBuf, Stamp)>,
    invocation: Option<Invocation>,
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
    if let Some(invocation) = installation.invocation.as_mut() {
        for (_, value) in &mut invocation.env {
            if value == OsStr::new("@TARGET@") {
                *value = target_version.clone().into();
            }
        }
        for argument in &mut invocation.args {
            if argument == OsStr::new("@TARGET@") {
                *argument = target_version.clone().into();
            } else if let Some(package) = argument
                .to_str()
                .and_then(|arg| arg.strip_suffix("@@TARGET@"))
            {
                *argument = format!("{package}@{target_version}").into();
            }
        }
    }
    let config = snapshot_config(&installation, &installed_version, &target_version, channel)?;
    if agent == CLIAgent::Claude && source_is_native_claude(&installation) && !version_matches {
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
    match probe_cli_agent_version(agent, entry).await {
        CLIAgentVersionStatus::Detected(version) => {
            parse_version(&version)?;
            Ok(version)
        }
        CLIAgentVersionStatus::NotInstalled
        | CLIAgentVersionStatus::NotProbed
        | CLIAgentVersionStatus::Unknown => Err(Error::ProbeFailed),
    }
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
                    let mut invocation = Invocation::new(&installation.entry, ["update"]);
                    invocation.env_remove.extend(
                        [
                            "CODEX_MANAGED_BY_NPM",
                            "CODEX_MANAGED_BY_BUN",
                            "CODEX_MANAGED_BY_PNPM",
                            "CODEX_MANAGED_BY_VITE_PLUS",
                        ]
                        .map(OsString::from),
                    );
                    invocation.env.extend([
                        (
                            "PATH".into(),
                            std::env::join_paths(paths).map_err(|_| Error::UnsupportedSource)?,
                        ),
                        ("CODEX_INSTALL_DIR".into(), parent.as_os_str().to_owned()),
                        ("CODEX_HOME".into(), codex_home.as_os_str().to_owned()),
                        ("CODEX_NON_INTERACTIVE".into(), "1".into()),
                        ("CODEX_RELEASE".into(), "@TARGET@".into()),
                    ]);
                    // Windows 安装器还会改写持久 PATH，未取得同安装无副作用证据前不开放。
                    if cfg!(unix) {
                        installation.config = Some((
                            codex_home.join("packages/standalone/auto-update-version"),
                            ConfigKind::CodexUpdateMarker,
                        ));
                        installation.invocation = Some(invocation);
                        installation.error = None;
                    } else {
                        installation.error = Some(Error::UnsupportedPlatform);
                    }
                    return Ok(installation);
                }
            }
            CLIAgent::Claude => {
                let versions = home.join(".local/share/claude/versions");
                if same_tree(&installation.stamp.canonical, &versions) {
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
                    installation.invocation = Some(Invocation::new(
                        &installation.entry,
                        ["--settings", settings.as_str(), "update"],
                    ));
                    installation.config = Some((config_path, ConfigKind::Claude));
                    installation.error = None;
                    return Ok(installation);
                }
            }
            CLIAgent::Grok => {
                let grok_home = absolute_env("GROK_HOME").unwrap_or_else(|| home.join(".grok"));
                if same_tree(&installation.stamp.canonical, &grok_home.join("downloads"))
                    && installation
                        .entry
                        .parent()
                        .and_then(|parent| parent.canonicalize().ok())
                        == grok_home.join("bin").canonicalize().ok()
                {
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
                    let args = if installation.channel == current_channel {
                        vec!["update"]
                    } else if installation.channel == Channel::Alpha {
                        vec!["update", "--alpha"]
                    } else {
                        vec!["update", "--stable"]
                    };
                    installation.invocation = Some(Invocation::new(&installation.entry, args));
                    installation.config = Some((grok_home.join("config.toml"), ConfigKind::Grok));
                    installation.error = None;
                    return Ok(installation);
                }
            }
            _ => return Err(Error::UnsupportedSource),
        }
    }
    if let Some(npm) = discover_npm(agent, &installation, requested).await? {
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
    requested: Channel,
) -> Result<Option<Installation>, Error> {
    let package = match agent {
        CLIAgent::Codex => "@openai/codex",
        CLIAgent::Claude => "@anthropic-ai/claude-code",
        _ => return Ok(None),
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
    let package_root = if cfg!(windows) {
        prefix.join("node_modules")
    } else {
        prefix.join("lib/node_modules")
    }
    .join(package);
    let manifest = package_root.join("package.json");
    if !manifest.is_file() {
        return Ok(None);
    }
    let manifest_bytes = read_limited(&manifest, MAX_CONFIG)?;
    let value: Value = serde_json::from_slice(&manifest_bytes).map_err(|_| Error::ProbeFailed)?;
    if value.get("name").and_then(Value::as_str) != Some(package) {
        return Ok(None);
    }
    let bin = value.get("bin").and_then(|bin| {
        bin.as_str()
            .or_else(|| bin.get(agent.command_prefix()).and_then(Value::as_str))
    });
    let Some(bin) = bin else {
        return Ok(None);
    };
    let bin_path = package_root.join(bin);
    if !same_tree(
        &bin_path
            .canonicalize()
            .map_err(|_| Error::UnsupportedSource)?,
        &package_root,
    ) {
        return Ok(None);
    }
    // 不把名称相同的自定义 shim 或另一个 Node 前缀误判成此 npm 安装。
    if bin_path.canonicalize().ok().as_ref() != Some(&installation.stamp.canonical) {
        return Ok(None);
    }
    let mut found = installation.clone();
    found.source = Source::Npm;
    found.channel = match requested {
        Channel::Stable | Channel::Alpha | Channel::Latest => requested,
        Channel::FollowInstallation => {
            if agent == CLIAgent::Codex
                && value
                    .get("version")
                    .and_then(Value::as_str)
                    .is_some_and(|version| version.contains("-alpha"))
            {
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
    found.registration = Some((manifest.clone(), stamp(&manifest)?));
    let spec = format!("{package}@@TARGET@");
    found.invocation = Some(Invocation::new(
        node,
        [
            npm_cli.as_os_str(),
            OsStr::new("install"),
            OsStr::new("--global"),
            OsStr::new("--prefix"),
            prefix.as_os_str(),
            OsStr::new(&spec),
            OsStr::new("--registry=https://registry.npmjs.org/"),
            OsStr::new("--no-audit"),
            OsStr::new("--no-fund"),
        ],
    ));
    found.error = None;
    if agent == CLIAgent::Claude {
        if requested == Channel::FollowInstallation {
            let home = user_home().ok_or(Error::UnsupportedSource)?;
            let config = absolute_env("CLAUDE_CONFIG_DIR")
                .unwrap_or_else(|| home.join(".claude"))
                .join("settings.json");
            found.channel = claude_channel(&config)?;
        }
        // npm install 不经过 Claude 的托管版本约束；原生 npm 更新尚未实证前保持明确降级。
        found.invocation = None;
        found.error = Some(Error::UnsupportedSource);
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
            let mut invocation = Invocation::new(&brew, ["upgrade", "--cask", "--greedy", cask]);
            invocation.env.extend([
                ("HOMEBREW_NO_AUTO_UPDATE".into(), "1".into()),
                ("HOMEBREW_NO_INSTALL_CLEANUP".into(), "1".into()),
            ]);
            found.invocation = Some(invocation);
            found.error = None;
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

#[derive(Debug)]
enum RunFailure {
    NotStarted(Error),
    Started(Error),
}

async fn run(invocation: &Invocation, timeout: Duration) -> Result<Vec<u8>, Error> {
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
    managed_process::confirmed_exit(root, scope.generation)
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
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::PersistenceFailed),
        }
    }
    Ok(())
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

fn preflight_recovery(agent: CLIAgent, entry: &Path, root: &Path) -> Result<bool, Error> {
    let path = root.join(format!("{}.json", agent.command_prefix()));
    if !path.try_exists().map_err(|_| Error::PersistenceFailed)? {
        return Ok(false);
    }
    let journal: Journal = serde_json::from_slice(&read_limited(&path, 12 * MAX_CONFIG)?)
        .map_err(|_| Error::RecoveryRequired)?;
    if journal.schema != 1 || journal.agent != agent.command_prefix() || journal.entry != entry {
        return Err(Error::RecoveryRequired);
    }
    if let Some(generation) = journal.generation {
        #[cfg(feature = "local_fs")]
        {
            if journal.claude_update.is_some()
                && !root
                    .join("cli-agent-processes")
                    .join(generation.to_string())
                    .try_exists()
                    .map_err(|_| Error::RecoveryRequired)?
            {
                validate_claude_journal(&journal)?;
                // 尚无代次账本时先永久认领“未启动”，排除任何延迟启动后再回收私有配置。
                managed_process::record_not_started(root, generation, &journal.entry, &[], root)
                    .map_err(|_| Error::RecoveryRequired)?;
            }
            managed_process::confirmed_exit(root, generation)
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
    if journal.schema != 1 || journal.agent != agent.command_prefix() || journal.entry != entry {
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

pub(super) async fn execute(plan: UpdatePlan) -> Result<String, Error> {
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
    if stamp(&installation.entry)? != installation.stamp {
        return Err(Error::SourceChanged);
    }
    if let Some(manager) = &installation.manager {
        if stamp(&manager.canonical)? != *manager {
            return Err(Error::SourceChanged);
        }
    }
    if let Some(helper) = &installation.helper {
        if stamp(&helper.canonical)? != *helper {
            return Err(Error::SourceChanged);
        }
    }
    if let Some((path, expected)) = &installation.registration {
        if stamp(path)? != *expected {
            return Err(Error::SourceChanged);
        }
    }
    if version(plan.agent, &installation.entry).await? != plan.installed_version {
        return Err(Error::SourceChanged);
    }
    let mut invocation = installation
        .invocation
        .as_ref()
        .ok_or(Error::UnsupportedSource)?
        .clone();
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
        schema: 1,
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
    };
    if !requires_native_update {
        // 同版本只同步渠道，不调用原生安装器，也不伪造进程退出回执。
        journal.phase = "config_prepared".to_owned();
        if let Some(config) = journal.config.as_mut() {
            config.after = config.before.clone();
        }
        save_journal(&journal_path, &journal)?;
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
                managed_process::record_not_started(
                    &root,
                    scope.generation,
                    &invocation.program,
                    &invocation.args,
                    &root,
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
    let failure = match run_supervised_update(&invocation, &root, journal.generation.unwrap()).await
    {
        Ok(failure) => failure,
        Err(error) => {
            // 只有真实退出可确认时才能回收副本；未知存活状态继续由 journal 阻止重投。
            if journal.claude_update.is_some() {
                finish_claude_scope(&root, &journal)?;
            }
            return Err(error);
        }
    };
    journal.command_failed = failure.is_some();
    save_journal(&journal_path, &journal)?;
    finish_supervised_update(&journal_path, &mut journal, plan.agent, &root).await?;
    if let Some(error) = failure {
        return Err(error);
    }
    if version(plan.agent, &installation.entry).await? != plan.target_version {
        return Err(Error::VersionMismatch);
    }
    Ok(plan.target_version)
}

#[cfg(feature = "local_fs")]
async fn run_supervised_update(
    invocation: &Invocation,
    root: &Path,
    generation: Uuid,
) -> Result<Option<Error>, Error> {
    let environment = ManagedEnvironment {
        values: invocation.env.clone(),
        remove: invocation.env_remove.clone(),
    };
    let child = managed_process::spawn_with_environment(
        root,
        generation,
        &invocation.program,
        &invocation.args,
        root,
        environment,
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
                managed_process::record_not_started(
                    root,
                    generation,
                    &invocation.program,
                    &invocation.args,
                    root,
                )
                .map_err(|_| Error::RecoveryRequired)?;
            }
            if managed_process::confirmed_exit(root, generation)
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
        (&mut stdout)
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| Error::CommandFailed)?;
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
    root: &Path,
    generation: Uuid,
) -> Result<Option<Error>, Error> {
    let _ = (invocation, root, generation);
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
        managed_process::confirmed_exit(root, generation)
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
