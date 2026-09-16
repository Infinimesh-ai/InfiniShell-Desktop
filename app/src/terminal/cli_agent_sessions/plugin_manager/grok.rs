use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::{env, fs, io};

use async_trait::async_trait;
use serde_json::Value;
#[cfg(not(target_family = "wasm"))]
use {
    command::{Output, r#async::Command},
    std::time::Duration,
    warpui::r#async::FutureExt as _,
};

use super::{CliAgentPluginManager, PluginInstallError, PluginInstructionStep, PluginInstructions};
#[cfg(not(target_family = "wasm"))]
use crate::util::path::resolve_executable_in_path;

const PLUGIN_NAME: &str = "infinishell-grok";
const PLUGIN_VERSION: &str = "0.1.0";
const TESTED_GROK_VERSION: &str = "1.0.30";
const BUNDLED_FILES: &[(&str, &str)] = &[
    (
        ".grok-plugin/plugin.json",
        include_str!("../../../../assets/bundled/cli-agent-plugins/grok/.grok-plugin/plugin.json"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../../../../assets/bundled/cli-agent-plugins/grok/hooks/hooks.json"),
    ),
    (
        "hooks/notify.cjs",
        include_str!("../../../../assets/bundled/cli-agent-plugins/grok/hooks/notify.cjs"),
    ),
    (
        "README.md",
        include_str!("../../../../assets/bundled/cli-agent-plugins/grok/README.md"),
    ),
];

pub(super) struct GrokPluginManager {
    path_env_var: Option<String>,
}

#[derive(Debug)]
struct InstalledPlugin {
    path: PathBuf,
    source: PathBuf,
    version: String,
}

impl GrokPluginManager {
    pub(super) fn new(path_env_var: Option<String>) -> Self {
        Self { path_env_var }
    }

    fn executable(&self, program: &str) -> Option<PathBuf> {
        #[cfg(not(target_family = "wasm"))]
        {
            let search_path = self
                .path_env_var
                .as_ref()
                .map(OsString::from)
                .unwrap_or_else(|| env::var_os("PATH").unwrap_or_default());
            resolve_executable_in_path(program, &search_path).map(|path| path.into_owned())
        }
        #[cfg(target_family = "wasm")]
        {
            None
        }
    }

    #[cfg(not(target_family = "wasm"))]
    async fn run(
        &self,
        executable: &Path,
        args: &[&OsStr],
        timeout: Duration,
        log: &mut String,
    ) -> Result<Output, PluginInstallError> {
        log.push_str(&format!("$ {} {args:?}\n", executable.display()));
        let mut command = Command::new(executable);
        command.args(args).kill_on_drop(true);
        if let Some(search_path) = &self.path_env_var {
            command.env("PATH", search_path);
        }
        let result = command.output().with_timeout(timeout).await;
        let Ok(Ok(output)) = result else {
            log.push_str(&format!("command failed or timed out: {result:?}\n"));
            return Err(operation_error(log));
        };
        for stream in [&output.stdout, &output.stderr] {
            let length = stream.len().min(65536);
            log.push_str(&String::from_utf8_lossy(&stream[..length]));
            log.push('\n');
        }
        if !output.status.success() {
            return Err(operation_error(log));
        }
        Ok(output)
    }

    #[cfg(not(target_family = "wasm"))]
    async fn verify_runtime(&self, log: &mut String) -> Result<PathBuf, PluginInstallError> {
        let grok = self
            .executable("grok")
            .ok_or_else(|| incompatible_error(log))?;
        let node = self
            .executable("node")
            .ok_or_else(|| incompatible_error(log))?;
        let grok_version = self
            .run(
                &grok,
                &[OsStr::new("--version")],
                Duration::from_secs(3),
                log,
            )
            .await?;
        let node_version = self
            .run(
                &node,
                &[OsStr::new("--version")],
                Duration::from_secs(3),
                log,
            )
            .await?;
        if !runtime_is_compatible(
            &String::from_utf8_lossy(&grok_version.stdout),
            &String::from_utf8_lossy(&node_version.stdout),
        ) {
            return Err(incompatible_error(log));
        }
        Ok(grok)
    }

    #[cfg(not(target_family = "wasm"))]
    async fn install_source(
        &self,
        executable: &Path,
        source: &Path,
        log: &mut String,
    ) -> Result<(), PluginInstallError> {
        self.run(
            executable,
            &[
                OsStr::new("plugin"),
                OsStr::new("install"),
                OsStr::new("--trust"),
                source.as_os_str(),
            ],
            Duration::from_secs(30),
            log,
        )
        .await?;
        Ok(())
    }

    #[cfg(not(target_family = "wasm"))]
    async fn apply(&self, updating: bool) -> Result<(), PluginInstallError> {
        let mut log = String::new();
        if env::var_os("GROK_CONFIG_PATH").is_some() || env::var_os("GROK_CONFIG").is_some() {
            return Err(invalid_state_error(&log));
        }
        let grok_home = grok_home_dir().map_err(|error| file_error(error, &log))?;
        if plugin_disabled(&grok_home).map_err(|error| file_error(error, &log))? {
            return Err(PluginInstallError {
                message: crate::t!("cli-agent-plugin-disabled"),
                log,
            });
        }
        // 在创建目录或修改注册表前确认真实 CLI 和 hook 运行时。
        let executable = self.verify_runtime(&mut log).await?;
        let previous = installed_plugin(&grok_home).map_err(|_| invalid_state_error(&log))?;
        let source_root = bundled_source_root().map_err(|error| file_error(error, &log))?;
        let source = write_bundle(&source_root).map_err(|error| file_error(error, &log))?;
        self.run(
            &executable,
            &[
                OsStr::new("plugin"),
                OsStr::new("validate"),
                source.as_os_str(),
            ],
            Duration::from_secs(5),
            &mut log,
        )
        .await?;

        let Some(previous) = previous else {
            self.install_source(&executable, &source, &mut log).await?;
            return verify_installed(&grok_home, PLUGIN_VERSION, &log);
        };
        if !updating || previous.version == PLUGIN_VERSION {
            return verify_installed(&grok_home, PLUGIN_VERSION, &log);
        }
        // 不接管同名的自定义来源，也不允许多插件仓库进入单插件恢复事务。
        if !previous.source.canonicalize().ok().is_some_and(|source| {
            source_root
                .canonicalize()
                .ok()
                .is_some_and(|root| source.starts_with(root))
        }) {
            return Err(invalid_state_error(&log));
        }
        let recovery =
            backup_plugin(&previous.path, &source_root).map_err(|error| file_error(error, &log))?;
        let removed = self
            .run(
                &executable,
                &[
                    OsStr::new("plugin"),
                    OsStr::new("uninstall"),
                    OsStr::new("--keep-data"),
                    OsStr::new(PLUGIN_NAME),
                ],
                Duration::from_secs(30),
                &mut log,
            )
            .await;
        if removed.is_err() && verify_installed(&grok_home, &previous.version, &log).is_ok() {
            return Err(operation_error(&log));
        }
        let attempted = if removed.is_ok() {
            self.install_source(&executable, &source, &mut log).await
        } else {
            Err(operation_error(&log))
        };
        if attempted.is_ok() && verify_installed(&grok_home, PLUGIN_VERSION, &log).is_ok() {
            return Ok(());
        }

        // 原生 local update 不更新复制文件；安装失败后用受控备份恢复，不回写全局配置快照。
        match registered_source(&grok_home) {
            Ok(Some(registered)) if registered == source || registered == previous.source => {
                // 安装中断可能留下本次操作的注册项，仅清理确知来源的单插件。
                self.run(
                    &executable,
                    &[
                        OsStr::new("plugin"),
                        OsStr::new("uninstall"),
                        OsStr::new("--keep-data"),
                        OsStr::new(PLUGIN_NAME),
                    ],
                    Duration::from_secs(30),
                    &mut log,
                )
                .await?;
            }
            Ok(None) => {}
            Ok(Some(_)) | Err(_) => {
                return Err(PluginInstallError {
                    message: crate::t!("cli-agent-plugin-grok-restore-failed"),
                    log,
                });
            }
        }
        let was_disabled = plugin_disabled(&grok_home).unwrap_or(true);
        let restored = self.install_source(&executable, &recovery, &mut log).await;
        if was_disabled && restored.is_ok() {
            self.run(
                &executable,
                &[
                    OsStr::new("plugin"),
                    OsStr::new("disable"),
                    OsStr::new(PLUGIN_NAME),
                ],
                Duration::from_secs(5),
                &mut log,
            )
            .await?;
        }
        let restored_version = installed_plugin(&grok_home)
            .ok()
            .flatten()
            .is_some_and(|plugin| plugin.version == previous.version);
        Err(PluginInstallError {
            message: if restored.is_ok() && restored_version {
                crate::t!("cli-agent-plugin-grok-update-restored")
            } else {
                crate::t!("cli-agent-plugin-grok-restore-failed")
            },
            log,
        })
    }
}

#[async_trait]
impl CliAgentPluginManager for GrokPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        PLUGIN_VERSION
    }

    fn can_auto_install(&self) -> bool {
        !self.is_disabled()
            && env::var_os("GROK_CONFIG_PATH").is_none()
            && env::var_os("GROK_CONFIG").is_none()
            && grok_home_dir().is_ok_and(|root| installed_plugin(&root).is_ok())
            && self.executable("grok").is_some()
            && self.executable("node").is_some()
    }

    fn is_installed(&self) -> bool {
        grok_home_dir().ok().is_some_and(|root| {
            plugin_enabled(&root).unwrap_or(false)
                && installed_plugin(&root).ok().flatten().is_some()
        })
    }

    fn is_disabled(&self) -> bool {
        grok_home_dir()
            .ok()
            .is_some_and(|root| plugin_disabled(&root).unwrap_or(true))
    }

    fn needs_update(&self) -> bool {
        grok_home_dir()
            .ok()
            .and_then(|root| installed_plugin(&root).ok().flatten())
            .is_some_and(|plugin| parse_version(&plugin.version) < parse_version(PLUGIN_VERSION))
    }

    async fn install(&self) -> Result<(), PluginInstallError> {
        #[cfg(not(target_family = "wasm"))]
        {
            self.apply(false).await
        }
        #[cfg(target_family = "wasm")]
        {
            Err(incompatible_error(""))
        }
    }

    async fn update(&self) -> Result<(), PluginInstallError> {
        #[cfg(not(target_family = "wasm"))]
        {
            self.apply(true).await
        }
        #[cfg(target_family = "wasm")]
        {
            Err(incompatible_error(""))
        }
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        if self.is_disabled() {
            &ENABLE_INSTRUCTIONS
        } else {
            &INSTALL_INSTRUCTIONS
        }
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }

    fn remote_install_instructions(&self) -> &'static PluginInstructions {
        &INSTALL_INSTRUCTIONS
    }
}

fn grok_home_dir() -> io::Result<PathBuf> {
    env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".grok")))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Grok home unavailable"))
}

fn bundled_source_root() -> io::Result<PathBuf> {
    dirs::data_local_dir()
        .map(|dir| dir.join("InfiniShell/cli-agent-plugins/grok"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Application data directory unavailable",
            )
        })
}

fn parse_version(version: &str) -> Option<[u64; 3]> {
    let parts = version
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    parts.try_into().ok()
}

fn runtime_is_compatible(grok: &str, node: &str) -> bool {
    grok.trim()
        .strip_prefix("grok ")
        .and_then(|value| value.split_whitespace().next())
        == Some(TESTED_GROK_VERSION)
        && node
            .trim()
            .strip_prefix('v')
            .and_then(parse_version)
            .is_some_and(|version| version[0] >= 18)
}

fn read_config(root: &Path) -> io::Result<toml::Value> {
    match fs::read_to_string(root.join("config.toml")) {
        Ok(contents) => toml::from_str(&contents)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(Default::default()))
        }
        Err(error) => Err(error),
    }
}

fn includes_plugin(config: &toml::Value, list: &str) -> io::Result<bool> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid Grok plugin configuration",
        )
    };
    let Some(plugins) = config.get("plugins") else {
        return Ok(false);
    };
    let plugins = plugins.as_table().ok_or_else(invalid)?;
    let Some(names) = plugins.get(list) else {
        return Ok(false);
    };
    let names = names.as_array().ok_or_else(invalid)?;
    let names = names
        .iter()
        .map(|name| name.as_str().ok_or_else(invalid))
        .collect::<io::Result<Vec<_>>>()?;
    Ok(names
        .into_iter()
        .any(|name| name == PLUGIN_NAME || name.ends_with(&format!("/{PLUGIN_NAME}"))))
}

fn plugin_disabled(root: &Path) -> io::Result<bool> {
    let config = read_config(root)?;
    includes_plugin(&config, "enabled")?;
    includes_plugin(&config, "disabled")
}

fn plugin_enabled(root: &Path) -> io::Result<bool> {
    let config = read_config(root)?;
    Ok(!includes_plugin(&config, "disabled")? && includes_plugin(&config, "enabled")?)
}

fn installed_plugin(root: &Path) -> io::Result<Option<InstalledPlugin>> {
    let contents = match fs::read_to_string(root.join("installed-plugins/registry.json")) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let registry: Value = serde_json::from_str(&contents)?;
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Ambiguous or invalid Grok plugin registry",
        )
    };
    if registry.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(invalid());
    }
    let repos = registry
        .get("repos")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let mut installed = None;
    for repo in repos.values() {
        let Some(plugins) = repo.get("plugins").and_then(Value::as_object) else {
            return Err(invalid());
        };
        let Some(plugin) = plugins.get(PLUGIN_NAME) else {
            continue;
        };
        if installed.is_some() || plugins.len() != 1 {
            return Err(invalid());
        }
        let kind = repo.get("kind").ok_or_else(invalid)?;
        if kind.get("type").and_then(Value::as_str) != Some("Local") {
            return Err(invalid());
        }
        let source = PathBuf::from(
            kind.get("source_path")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?,
        );
        let path = PathBuf::from(
            repo.get("path")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?,
        );
        let version = plugin
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?
            .to_owned();
        if !source.is_absolute() || !path.is_absolute() || parse_version(&version).is_none() {
            return Err(invalid());
        }
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(path.join(".grok-plugin/plugin.json"))?)?;
        if manifest.get("name").and_then(Value::as_str) != Some(PLUGIN_NAME)
            || manifest.get("version").and_then(Value::as_str) != Some(version.as_str())
            || BUNDLED_FILES
                .iter()
                .any(|(relative, _)| !path.join(relative).is_file())
        {
            return Err(invalid());
        }
        installed = Some(InstalledPlugin {
            path,
            source,
            version,
        });
    }
    Ok(installed)
}

fn registered_source(root: &Path) -> io::Result<Option<PathBuf>> {
    let contents = match fs::read_to_string(root.join("installed-plugins/registry.json")) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let registry: Value = serde_json::from_str(&contents)?;
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Ambiguous Grok recovery registration",
        )
    };
    if registry.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(invalid());
    }
    let repos = registry
        .get("repos")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let mut source = None;
    for repo in repos.values() {
        let plugins = repo
            .get("plugins")
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        if !plugins.contains_key(PLUGIN_NAME) {
            continue;
        }
        if plugins.len() != 1 || source.is_some() {
            return Err(invalid());
        }
        let kind = repo.get("kind").ok_or_else(invalid)?;
        if kind.get("type").and_then(Value::as_str) != Some("Local") {
            return Err(invalid());
        }
        source = Some(PathBuf::from(
            kind.get("source_path")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?,
        ));
    }
    Ok(source)
}

fn write_bundle(root: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(root)?;
    let destination = root.join(PLUGIN_VERSION);
    if destination.exists() {
        for (relative, contents) in BUNDLED_FILES {
            if fs::read_to_string(destination.join(relative))? != *contents {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Bundled Grok source was modified",
                ));
            }
        }
        return Ok(destination);
    }
    let staging = tempfile::tempdir_in(root)?;
    for (relative, contents) in BUNDLED_FILES {
        let path = staging.path().join(relative);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, contents)?;
    }
    fs::rename(staging.path(), &destination)?;
    Ok(destination)
}

fn backup_plugin(installed: &Path, root: &Path) -> io::Result<PathBuf> {
    let recovery_root = root.join("recovery");
    fs::create_dir_all(&recovery_root)?;
    let recovery = tempfile::tempdir_in(recovery_root)?;
    for (relative, _) in BUNDLED_FILES {
        let target = recovery.path().join(relative);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(installed.join(relative), target)?;
    }
    Ok(recovery.keep())
}

fn verify_installed(root: &Path, expected: &str, log: &str) -> Result<(), PluginInstallError> {
    if plugin_enabled(root).unwrap_or(false)
        && installed_plugin(root)
            .ok()
            .flatten()
            .is_some_and(|plugin| plugin.version == expected)
    {
        return Ok(());
    }
    Err(PluginInstallError {
        message: crate::t!("cli-agent-plugin-update-not-effective"),
        log: log.to_owned(),
    })
}

fn operation_error(log: &str) -> PluginInstallError {
    PluginInstallError {
        message: crate::t!("cli-agent-plugin-grok-operation-failed"),
        log: log.to_owned(),
    }
}

fn file_error(error: io::Error, log: &str) -> PluginInstallError {
    let mut result = operation_error(log);
    result.log.push_str(&format!("filesystem error: {error}\n"));
    result
}

fn incompatible_error(log: &str) -> PluginInstallError {
    PluginInstallError {
        message: crate::t!("cli-agent-plugin-grok-incompatible"),
        log: log.to_owned(),
    }
}

fn invalid_state_error(log: &str) -> PluginInstallError {
    PluginInstallError {
        message: crate::t!("cli-agent-plugin-grok-invalid-state"),
        log: log.to_owned(),
    }
}

static INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: crate::t_static!("cli-agent-plugin-grok-install-title"),
    subtitle: crate::t_static!("cli-agent-plugin-grok-install-subtitle"),
    steps: vec![
        PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-grok-cli-version-step"),
            command: "grok --version",
            executable: true,
            link: None,
        },
        PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-grok-node-version-step"),
            command: "node --version",
            executable: true,
            link: None,
        },
    ],
    post_install_notes: vec![crate::t_static!("cli-agent-plugin-grok-restart-note")],
});

static ENABLE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: crate::t_static!("cli-agent-plugin-grok-install-title"),
    subtitle: crate::t_static!("cli-agent-plugin-disabled"),
    steps: vec![PluginInstructionStep {
        description: crate::t_static!("cli-agent-plugin-grok-enable-step"),
        command: "grok plugin enable infinishell-grok",
        executable: true,
        link: None,
    }],
    post_install_notes: vec![crate::t_static!("cli-agent-plugin-grok-restart-note")],
});

#[cfg(test)]
#[path = "grok_tests.rs"]
mod tests;
