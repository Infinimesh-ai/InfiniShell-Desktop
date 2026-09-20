use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::{env, fs, io};

use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};
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
const PLUGIN_VERSION: &str = "0.1.3";
const TESTED_GROK_VERSION: &str = "1.0.30";
// 只认可此次发布前的完整 0.1.0 配方，不能把任意旧版说明当作应用来源。
const LEGACY_README_SHA256: &str =
    "2db65e9c54c35725ba1164edb39daf9645bfdf7f1d3cd2689b8e338853c2e8b7";
// 历史通知脚本只按固定 0.1.0 原字节认可，当前脚本的版本修复不能写回旧来源。
const LEGACY_NOTIFY_SHA256: &str =
    "134fd490e7396157c80c2a33bd9d8a88397881e6f926743319fafe6c0df5ccd8";
// 0.1.1 的通知原字节及两份已发布到验收构建的说明，升级时保留原来源和恢复副本。
const LEGACY_011_NOTIFY_SHA256: &str =
    "9d100f0aad5ce14e9237a15f39537278e8208082bc298acb2883662d8cd9580a";
const LEGACY_011_README_SHA256: [&str; 2] = [
    "13a5451348ef9278f243c18ea3f8c8af714cebfe49e2eb0979263de841531167",
    "adf7a48c57b108a3b1d6c02fc33c077df6437324df40c4664a35f883bfa9cf41",
];
// 0.1.0–0.1.2 共用的九项 hook 必须按旧字节核验，不能套用新版本的第十项。
const LEGACY_HOOKS_SHA256: &str =
    "2ec75e0fc4daf1da6e649d7b11cfb2e3d836973455ad4b14852366f1392d6326";
const LEGACY_012_MANIFEST: &str =
    include_str!("../../../../../specs/cli-agent-parity/fixtures/grok-plugin-0.1.2-plugin.json");
const LEGACY_012_SHA256: &[(&str, &str)] = &[
    (".grok-plugin/plugin.json", "8cdbd6179378a60e8de4195c80aa64982936b11d3d66d2cb3d9fc5843b62b571"),
    ("hooks/hooks.json", LEGACY_HOOKS_SHA256),
    ("hooks/notify.cjs", "741c25075a63e3a03b0b9921e98c5685531dd296d66b0156b909497b37dd0188"),
    ("README.md", "2c275ebe80cd620aad36563c0eeff4e38c302dcf9ca324f1962df9b4134b60ca"),
];
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
        let previous = registered_plugin(&grok_home).map_err(|_| invalid_state_error(&log))?;
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

        // 只读校验的 await 期间也可能收到新的禁用或注册意图，不能沿用起始状态。
        if plugin_disabled(&grok_home).map_err(|error| file_error(error, &log))?
            || registered_plugin(&grok_home).map_err(|_| invalid_state_error(&log))? != previous
        {
            return Err(invalid_state_error(&log));
        }

        let Some(previous) = previous else {
            self.install_source(&executable, &source, &mut log).await?;
            return verify_installed(&grok_home, PLUGIN_VERSION, &log);
        };
        if previous.version == PLUGIN_VERSION {
            // 同版本修复不重装原生插件，避免安装命令重新启用用户禁用的配置。
            repair_current_plugin(&grok_home, &previous, &source_root, |_, _| Ok(()))
                .map_err(|error| file_error(error, &log))?;
            return verify_installed(&grok_home, PLUGIN_VERSION, &log);
        }
        if !updating {
            return Err(invalid_state_error(&log));
        }
        upgrade_plugin(
            &grok_home,
            &previous,
            &source_root,
            &source,
            &(self, executable.as_path()),
            &mut log,
        )
        .await
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
            && grok_home_dir().is_ok_and(|root| {
                registered_plugin(&root).is_ok_and(|plugin| match plugin {
                    Some(plugin) => {
                        plugin_tree(&plugin.path, true).is_ok()
                            && bundled_source_root()
                                .is_ok_and(|source| validate_owned_source(&plugin, &source).is_ok())
                    }
                    None => true,
                })
            })
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
        grok_home_dir().ok().is_some_and(|root| {
            registered_plugin(&root)
                .ok()
                .flatten()
                .is_some_and(|plugin| {
                    parse_version(&plugin.version) < parse_version(PLUGIN_VERSION)
                        || installed_plugin(&root).is_err()
                })
        })
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

    fn install_success_message(&self) -> &'static str {
        crate::t_static!("cli-agent-plugin-grok-installed")
    }

    fn update_success_message(&self) -> &'static str {
        crate::t_static!("cli-agent-plugin-grok-updated")
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
    Ok(names.into_iter().any(plugin_name_matches))
}

fn plugin_name_matches(name: &str) -> bool {
    name == PLUGIN_NAME || name.ends_with(&format!("/{PLUGIN_NAME}"))
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

fn registered_plugin(root: &Path) -> io::Result<Option<InstalledPlugin>> {
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
    for (repo_key, repo) in repos {
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
        // 原生缓存只能属于当前配置目录中的这一条注册记录，不能据 JSON 路径改写外部文件。
        let mut components = Path::new(repo_key).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err(invalid());
        }
        let cache_root = root.canonicalize()?.join("installed-plugins");
        plain_directory(&cache_root)?;
        plain_directory(&path)?;
        if path.canonicalize()? != cache_root.join(repo_key) {
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

fn installed_plugin(root: &Path) -> io::Result<Option<InstalledPlugin>> {
    let plugin = registered_plugin(root)?;
    if let Some(plugin) = &plugin {
        validate_expected_tree(&plugin.path, &plugin.version)?;
    }
    Ok(plugin)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PluginFile {
    contents: Vec<u8>,
    permissions: fs::Permissions,
}

type PluginTree = BTreeMap<String, PluginFile>;

fn invalid_tree() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Grok 受控插件目录不匹配")
}

fn plain_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_tree());
    }
    Ok(())
}

fn plugin_tree(root: &Path, allow_missing: bool) -> io::Result<PluginTree> {
    plain_directory(root)?;
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeMap::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| invalid_tree())?
                .components()
                .map(|part| match part {
                    Component::Normal(name) => name.to_str().ok_or_else(invalid_tree),
                    Component::Prefix(_)
                    | Component::RootDir
                    | Component::CurDir
                    | Component::ParentDir => Err(invalid_tree()),
                })
                .collect::<io::Result<Vec<_>>>()?
                .join("/");
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(invalid_tree());
            }
            if metadata.is_dir() {
                if !matches!(relative.as_str(), ".grok-plugin" | "hooks") {
                    return Err(invalid_tree());
                }
                pending.push(path);
                continue;
            }
            if !metadata.is_file()
                || metadata.len() > 1024 * 1024
                || !BUNDLED_FILES.iter().any(|(name, _)| *name == relative)
            {
                return Err(invalid_tree());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt as _;
                if metadata.nlink() != 1 {
                    return Err(invalid_tree());
                }
            }
            files.insert(
                relative,
                PluginFile {
                    contents: fs::read(path)?,
                    permissions: metadata.permissions(),
                },
            );
        }
    }
    if !allow_missing && files.len() != BUNDLED_FILES.len() {
        return Err(invalid_tree());
    }
    Ok(files)
}

fn validate_expected_tree(root: &Path, version: &str) -> io::Result<PluginTree> {
    let tree = plugin_tree(root, false)?;
    if !matches!(version, "0.1.0" | "0.1.1" | "0.1.2") && version != PLUGIN_VERSION {
        return Err(invalid_tree());
    }
    for (name, expected) in BUNDLED_FILES {
        let contents = &tree.get(*name).ok_or_else(invalid_tree)?.contents;
        if version == PLUGIN_VERSION {
            if contents.as_slice() != expected.as_bytes() {
                return Err(invalid_tree());
            }
            continue;
        }
        if matches!(version, "0.1.0" | "0.1.1") && *name == ".grok-plugin/plugin.json" {
            // 保留两旧版原有的 manifest 格式兼容，但字段必须来自已固定的历史配方。
            let mut manifest: Value = serde_json::from_str(LEGACY_012_MANIFEST)?;
            manifest["version"] = Value::String(version.to_owned());
            if serde_json::from_slice::<Value>(contents)? != manifest {
                return Err(invalid_tree());
            }
            continue;
        }
        let digest = format!("{:x}", Sha256::digest(contents));
        let known = match (version, *name) {
            ("0.1.0", "README.md") => digest == LEGACY_README_SHA256,
            ("0.1.0", "hooks/notify.cjs") => digest == LEGACY_NOTIFY_SHA256,
            ("0.1.1", "README.md") => LEGACY_011_README_SHA256.contains(&digest.as_str()),
            ("0.1.1", "hooks/notify.cjs") => digest == LEGACY_011_NOTIFY_SHA256,
            ("0.1.0" | "0.1.1", "hooks/hooks.json") => digest == LEGACY_HOOKS_SHA256,
            ("0.1.2", name) => LEGACY_012_SHA256
                .iter()
                .any(|(expected_name, expected_digest)| name == *expected_name && digest == *expected_digest),
            // 未知配方或文件不得借较小版本号取得应用所有权。
            _ => false,
        };
        if !known {
            return Err(invalid_tree());
        }
    }
    Ok(tree)
}

fn validate_owned_source(plugin: &InstalledPlugin, source_root: &Path) -> io::Result<()> {
    plain_directory(source_root)?;
    plain_directory(&plugin.source)?;
    if !plugin
        .source
        .canonicalize()?
        .starts_with(source_root.canonicalize()?)
    {
        return Err(invalid_tree());
    }
    validate_expected_tree(&plugin.source, &plugin.version)?;
    Ok(())
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Copy)]
enum PluginMutation<'a> {
    Uninstall,
    Install(&'a Path),
}

#[cfg(not(target_family = "wasm"))]
#[async_trait]
trait PluginMutationRunner: Sync {
    async fn mutate(
        &self,
        mutation: PluginMutation<'_>,
        log: &mut String,
    ) -> Result<(), PluginInstallError>;
}

#[cfg(not(target_family = "wasm"))]
#[async_trait]
impl PluginMutationRunner for (&GrokPluginManager, &Path) {
    async fn mutate(
        &self,
        mutation: PluginMutation<'_>,
        log: &mut String,
    ) -> Result<(), PluginInstallError> {
        match mutation {
            PluginMutation::Install(source) => self.0.install_source(self.1, source, log).await,
            PluginMutation::Uninstall => {
                self.0
                    .run(
                        self.1,
                        &[
                            OsStr::new("plugin"),
                            OsStr::new("uninstall"),
                            OsStr::new("--keep-data"),
                            OsStr::new(PLUGIN_NAME),
                        ],
                        Duration::from_secs(30),
                        log,
                    )
                    .await?;
                Ok(())
            }
        }
    }
}

#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Debug, PartialEq)]
struct PluginMutationState {
    registry: Option<Vec<u8>>,
    config_contents: Option<Vec<u8>>,
    config: toml::Value,
    plugin: Option<InstalledPlugin>,
    files: Option<PluginTree>,
    tracked_cache: Option<PluginTree>,
}

#[cfg(not(target_family = "wasm"))]
fn optional_file(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(not(target_family = "wasm"))]
fn plugin_mutation_state(home: &Path, tracked_cache: &Path) -> io::Result<PluginMutationState> {
    let registry_path = home.join("installed-plugins/registry.json");
    let config_path = home.join("config.toml");
    let registry = optional_file(&registry_path)?;
    let config_contents = optional_file(&config_path)?;
    let config = read_config(home)?;
    includes_plugin(&config, "enabled")?;
    includes_plugin(&config, "disabled")?;
    let plugin = registered_plugin(home)?;
    if registered_source(home)?.as_deref() != plugin.as_ref().map(|plugin| plugin.source.as_path())
    {
        return Err(invalid_tree());
    }
    let files = plugin
        .as_ref()
        .map(|plugin| plugin_tree(&plugin.path, true))
        .transpose()?;
    let tracked_files = match fs::symlink_metadata(tracked_cache) {
        Ok(_) => Some(plugin_tree(tracked_cache, true)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if plugin
        .as_ref()
        .is_some_and(|plugin| plugin.path.as_path() == tracked_cache)
        && files != tracked_files
    {
        return Err(invalid_tree());
    }
    if optional_file(&registry_path)? != registry || optional_file(&config_path)? != config_contents
    {
        return Err(invalid_tree());
    }
    Ok(PluginMutationState {
        registry,
        config_contents,
        config,
        plugin,
        files,
        tracked_cache: tracked_files,
    })
}

#[cfg(not(target_family = "wasm"))]
fn config_without_plugin(config: &toml::Value) -> io::Result<toml::Value> {
    let mut config = config.clone();
    let table = config.as_table_mut().ok_or_else(invalid_tree)?;
    if let Some(plugins) = table.get_mut("plugins") {
        let plugins = plugins.as_table_mut().ok_or_else(invalid_tree)?;
        for name in ["enabled", "disabled"] {
            if let Some(names) = plugins.get_mut(name) {
                let names = names.as_array_mut().ok_or_else(invalid_tree)?;
                if names.iter().any(|name| name.as_str().is_none()) {
                    return Err(invalid_tree());
                }
                names.retain(|name| !name.as_str().is_some_and(plugin_name_matches));
                if names.is_empty() {
                    plugins.remove(name);
                }
            }
        }
        if plugins.is_empty() {
            table.remove("plugins");
        }
    }
    Ok(config)
}

#[cfg(not(target_family = "wasm"))]
fn registry_without_plugin(contents: Option<&[u8]>) -> io::Result<Value> {
    let mut registry = match contents {
        Some(contents) => serde_json::from_slice::<Value>(contents)?,
        None => serde_json::json!({"version":1,"repos":{}}),
    };
    let repos = registry
        .get_mut("repos")
        .and_then(Value::as_object_mut)
        .ok_or_else(invalid_tree)?;
    repos.retain(|_, repo| {
        !repo
            .get("plugins")
            .and_then(Value::as_object)
            .is_some_and(|plugins| plugins.contains_key(PLUGIN_NAME))
    });
    Ok(registry)
}

#[cfg(not(target_family = "wasm"))]
struct PluginUpgradeGuard<'a> {
    home: &'a Path,
    tracked_cache: &'a Path,
    original_cache: PluginTree,
    sources: Vec<(&'a Path, PluginTree)>,
    registry_without_plugin: Value,
    config_without_plugin: toml::Value,
}

#[cfg(not(target_family = "wasm"))]
impl PluginUpgradeGuard<'_> {
    fn check_boundary(&self, state: &PluginMutationState) -> io::Result<()> {
        if plugin_mutation_state(self.home, self.tracked_cache)? != *state
            || includes_plugin(&state.config, "disabled")?
            || registry_without_plugin(state.registry.as_deref())? != self.registry_without_plugin
            || config_without_plugin(&state.config)? != self.config_without_plugin
        {
            return Err(invalid_tree());
        }
        for (source, expected) in &self.sources {
            if plugin_tree(source, false)? != *expected {
                return Err(invalid_tree());
            }
        }
        Ok(())
    }

    async fn mutate(
        &self,
        state: &mut PluginMutationState,
        mutation: PluginMutation<'_>,
        runner: &impl PluginMutationRunner,
        log: &mut String,
    ) -> Result<Result<(), PluginInstallError>, PluginInstallError> {
        self.check_boundary(state)
            .map_err(|_| upgrade_state_error(log))?;
        match mutation {
            PluginMutation::Uninstall => {
                if state.plugin.is_none() {
                    return Err(upgrade_state_error(log));
                }
            }
            PluginMutation::Install(source) => {
                if state.plugin.is_some() || !self.sources.iter().any(|(path, _)| *path == source) {
                    return Err(upgrade_state_error(log));
                }
            }
        }
        let result = runner.mutate(mutation, log).await;
        let next = plugin_mutation_state(self.home, self.tracked_cache)
            .map_err(|_| upgrade_state_error(log))?;
        self.check_boundary(&next)
            .map_err(|_| upgrade_state_error(log))?;
        // 只允许本命令能产生的注册与已知缓存子集；同 source_path 不能证明用户新内容属于本操作。
        if let Some(plugin) = &next.plugin {
            let expected = match mutation {
                PluginMutation::Uninstall => {
                    if state.plugin.as_ref() != Some(plugin) {
                        return Err(upgrade_state_error(log));
                    }
                    state
                        .files
                        .as_ref()
                        .ok_or_else(|| upgrade_state_error(log))?
                }
                PluginMutation::Install(source) => {
                    let actual_source = plugin
                        .source
                        .canonicalize()
                        .map_err(|_| upgrade_state_error(log))?;
                    let expected_source = source
                        .canonicalize()
                        .map_err(|_| upgrade_state_error(log))?;
                    if state.plugin.is_some() || actual_source != expected_source {
                        return Err(upgrade_state_error(log));
                    }
                    let expected = self
                        .sources
                        .iter()
                        .find(|(path, _)| *path == source)
                        .map(|(_, tree)| tree)
                        .ok_or_else(|| upgrade_state_error(log))?;
                    let manifest = expected
                        .get(".grok-plugin/plugin.json")
                        .ok_or_else(|| upgrade_state_error(log))?;
                    let manifest: Value = serde_json::from_slice(&manifest.contents)
                        .map_err(|_| upgrade_state_error(log))?;
                    if manifest.get("version").and_then(Value::as_str)
                        != Some(plugin.version.as_str())
                    {
                        return Err(upgrade_state_error(log));
                    }
                    expected
                }
            };
            let files = next
                .files
                .as_ref()
                .ok_or_else(|| upgrade_state_error(log))?;
            // 新缓存的原生权限没有提前声明；记录真实权限用于下一边界，不猜复制策略。
            let permissions_must_match = matches!(mutation, PluginMutation::Uninstall);
            let unknown_file = files.iter().any(|(name, file)| {
                !expected.get(name).is_some_and(|expected| {
                    expected.contents == file.contents
                        && (!permissions_must_match || expected.permissions == file.permissions)
                })
            });
            if unknown_file {
                return Err(upgrade_state_error(log));
            }
        }
        let tracked_is_registered = next
            .plugin
            .as_ref()
            .is_some_and(|plugin| plugin.path.as_path() == self.tracked_cache);
        if !tracked_is_registered {
            if let Some(files) = &next.tracked_cache {
                if files
                    .iter()
                    .any(|(name, file)| self.original_cache.get(name) != Some(file))
                {
                    return Err(upgrade_state_error(log));
                }
            }
            let tracked_was_registered = state
                .plugin
                .as_ref()
                .is_some_and(|plugin| plugin.path.as_path() == self.tracked_cache);
            if !tracked_was_registered && next.tracked_cache != state.tracked_cache {
                return Err(upgrade_state_error(log));
            }
        }
        *state = next;
        Ok(result)
    }
}

#[cfg(not(target_family = "wasm"))]
fn upgrade_state_error(log: &str) -> PluginInstallError {
    PluginInstallError {
        message: crate::t!("cli-agent-plugin-grok-restore-failed"),
        log: log.to_owned(),
    }
}

#[cfg(not(target_family = "wasm"))]
async fn upgrade_plugin(
    home: &Path,
    previous: &InstalledPlugin,
    source_root: &Path,
    source: &Path,
    runner: &impl PluginMutationRunner,
    log: &mut String,
) -> Result<(), PluginInstallError> {
    validate_owned_source(previous, source_root).map_err(|_| invalid_state_error(log))?;
    let old_tree = validate_expected_tree(&previous.path, &previous.version)
        .map_err(|_| invalid_state_error(log))?;
    let old_source = validate_expected_tree(&previous.source, &previous.version)
        .map_err(|_| invalid_state_error(log))?;
    let new_source =
        validate_expected_tree(source, PLUGIN_VERSION).map_err(|_| invalid_state_error(log))?;
    let mut state =
        plugin_mutation_state(home, &previous.path).map_err(|_| invalid_state_error(log))?;
    if state.plugin.as_ref() != Some(previous)
        || state.files.as_ref() != Some(&old_tree)
        || !includes_plugin(&state.config, "enabled").map_err(|_| invalid_state_error(log))?
        || includes_plugin(&state.config, "disabled").map_err(|_| invalid_state_error(log))?
    {
        return Err(invalid_state_error(log));
    }
    let recovery =
        backup_plugin(&previous.path, source_root).map_err(|error| file_error(error, log))?;
    let recovery_tree = validate_expected_tree(&recovery, &previous.version)
        .map_err(|_| invalid_state_error(log))?;
    if recovery_tree != old_tree {
        return Err(invalid_state_error(log));
    }
    let guard = PluginUpgradeGuard {
        home,
        tracked_cache: &previous.path,
        original_cache: old_tree.clone(),
        sources: vec![
            (&previous.source, old_source),
            (source, new_source),
            (&recovery, recovery_tree),
        ],
        registry_without_plugin: registry_without_plugin(state.registry.as_deref())
            .map_err(|_| invalid_state_error(log))?,
        config_without_plugin: config_without_plugin(&state.config)
            .map_err(|_| invalid_state_error(log))?,
    };
    let removed = guard
        .mutate(&mut state, PluginMutation::Uninstall, runner, log)
        .await?;
    if state.plugin.as_ref() == Some(previous) && state.files.as_ref() == Some(&old_tree) {
        return Err(operation_error(log));
    }
    if removed.is_ok() && state.plugin.is_none() {
        let attempted = guard
            .mutate(&mut state, PluginMutation::Install(source), runner, log)
            .await?;
        if attempted.is_ok() && verify_installed(home, PLUGIN_VERSION, log).is_ok() {
            guard
                .check_boundary(&state)
                .map_err(|_| upgrade_state_error(log))?;
            return Ok(());
        }
    }
    if state.plugin.is_some() {
        let cleanup = guard
            .mutate(&mut state, PluginMutation::Uninstall, runner, log)
            .await?;
        if cleanup.is_err() || state.plugin.is_some() {
            return Err(upgrade_state_error(log));
        }
    }
    let restored = guard
        .mutate(&mut state, PluginMutation::Install(&recovery), runner, log)
        .await?;
    let restored_version =
        restored.is_ok() && verify_installed(home, &previous.version, log).is_ok();
    guard
        .check_boundary(&state)
        .map_err(|_| upgrade_state_error(log))?;
    Err(PluginInstallError {
        message: if restored_version {
            crate::t!("cli-agent-plugin-grok-update-restored")
        } else {
            crate::t!("cli-agent-plugin-grok-restore-failed")
        },
        log: log.to_owned(),
    })
}

fn replace_plugin_file(path: &Path, file: &PluginFile) -> io::Result<()> {
    let parent = path.parent().ok_or_else(invalid_tree)?;
    fs::create_dir_all(parent)?;
    plain_directory(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary
        .as_file()
        .set_permissions(file.permissions.clone())?;
    temporary.write_all(&file.contents)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn repair_current_plugin(
    home: &Path,
    plugin: &InstalledPlugin,
    source_root: &Path,
    mut before_replace: impl FnMut(usize, &Path) -> io::Result<()>,
) -> io::Result<()> {
    if plugin.version != PLUGIN_VERSION || !plugin_enabled(home)? {
        return Err(invalid_tree());
    }
    validate_owned_source(plugin, source_root)?;
    let source = validate_expected_tree(&plugin.source, PLUGIN_VERSION)?;
    let original = plugin_tree(&plugin.path, true)?;
    let registry_path = home.join("installed-plugins/registry.json");
    let registry = fs::read(&registry_path)?;
    if registered_plugin(home)?.as_ref() != Some(plugin) || fs::read(&registry_path)? != registry {
        return Err(invalid_tree());
    }
    let mut expected = original.clone();
    let mut changed = Vec::new();
    let outcome = (|| {
        for (index, (name, _)) in BUNDLED_FILES.iter().enumerate() {
            let mut replacement = source.get(*name).ok_or_else(invalid_tree)?.clone();
            if let Some(previous) = original.get(*name) {
                replacement.permissions = previous.permissions.clone();
                if replacement.contents == previous.contents {
                    continue;
                }
            }
            let path = plugin.path.join(name);
            before_replace(index, &path)?;
            // 每次写前重读禁用、注册及完整目录，不能拿开始时的授权覆盖后续变化。
            if !plugin_enabled(home)?
                || fs::read(&registry_path)? != registry
                || plugin_tree(&plugin.path, true)? != expected
            {
                return Err(invalid_tree());
            }
            replace_plugin_file(&path, &replacement)?;
            expected.insert((*name).to_owned(), replacement);
            changed.push(*name);
        }
        if !plugin_enabled(home)?
            || fs::read(&registry_path)? != registry
            || plugin_tree(&plugin.path, false)? != expected
        {
            return Err(invalid_tree());
        }
        validate_expected_tree(&plugin.path, PLUGIN_VERSION)?;
        validate_owned_source(plugin, source_root)?;
        Ok(())
    })();
    if let Err(error) = outcome {
        let mut recovery_failed = false;
        for name in changed.into_iter().rev() {
            let current = plugin_tree(&plugin.path, true);
            if fs::read(&registry_path).ok().as_deref() != Some(registry.as_slice())
                || current.as_ref().ok().and_then(|tree| tree.get(name)) != expected.get(name)
            {
                recovery_failed = true;
                continue;
            }
            let path = plugin.path.join(name);
            let restored = match original.get(name) {
                Some(file) => replace_plugin_file(&path, file),
                None => fs::remove_file(&path),
            };
            if restored.is_err() {
                recovery_failed = true;
            }
        }
        return if recovery_failed {
            Err(io::Error::other(format!(
                "{error}; 部分文件未恢复，已保留并发修改"
            )))
        } else {
            Err(error)
        };
    }
    Ok(())
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
    plain_directory(root)?;
    let destination = root.join(PLUGIN_VERSION);
    if destination.exists() {
        validate_expected_tree(&destination, PLUGIN_VERSION)?;
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
    plugin_tree(installed, false)?;
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
