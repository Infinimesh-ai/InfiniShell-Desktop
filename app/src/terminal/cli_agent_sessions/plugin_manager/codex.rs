use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::{env, fs, io};

use async_trait::async_trait;
use serde_json::Value;

use super::codex_source;
use super::notification_patch::{self, PatchKind, VerifiedRuntime};
use super::{
    CliAgentPluginManager, NativeAuthorizationStatus, PluginInstallError, PluginInstructionStep,
    PluginInstructions, compare_versions,
};
use crate::features::FeatureFlag;

const PLUGIN_NAME: &str = "warp";
const PLUGIN_KEY: &str = "warp@codex-warp";
const MARKETPLACE_NAME: &str = "codex-warp";

const PLATFORM_PLUGIN_NAME: &str = "orchestration";
const PLATFORM_PLUGIN_KEY: &str = "orchestration@codex-warp";

const CODEX_CONFIG_DIR: &str = ".codex";
const CODEX_HOME_ENV: &str = "CODEX_HOME";

// Keep in sync with the plugin version in warpdotdev/codex-warp.
const MINIMUM_PLUGIN_VERSION: &str = "0.4.0";
// Keep in sync with the orchestration plugin version in warpdotdev/codex-warp.
const MINIMUM_PLATFORM_PLUGIN_VERSION: &str = "0.4.0";

pub(super) struct CodexPluginManager {
    path_env_var: Option<String>,
}

impl CodexPluginManager {
    pub(super) fn new(path_env_var: Option<String>) -> Self {
        Self { path_env_var }
    }

    async fn notification_operation(&self) -> Result<(), PluginInstallError> {
        if self.is_disabled() {
            return Err(PluginInstallError {
                message: crate::t!("cli-agent-plugin-disabled"),
                log: String::new(),
            });
        }
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return Err(notification_patch::unsupported());
        }
        if self.has_local_marketplace_override() {
            return Err(notification_patch::modified());
        }
        let home = codex_home_dir()?;
        super::codex_hook_trust::invalidate(&home);
        let mut log = String::new();
        let runtime = VerifiedRuntime::probe(
            PatchKind::Codex,
            &home,
            self.path_env_var.as_deref(),
            &mut log,
        )
        .await?;
        codex_source::install(&home, PLUGIN_NAME, &runtime, &mut log).await?;
        // 原生 CLI 可能拒绝操作、留下旧版本或遇到禁用配置；不能仅凭退出码报告成功。
        if self.is_disabled() || !self.is_installed() {
            return Err(PluginInstallError {
                message: crate::t!("cli-agent-plugin-update-not-effective"),
                log,
            });
        }
        super::codex_hook_trust::invalidate(&home);
        if !codex_source::is_current(&home)
            || !notification_patch::full_tree_is_applied(&home, PatchKind::Codex)
        {
            return Err(notification_patch::modified());
        }
        Ok(())
    }

    async fn platform_operation(&self) -> Result<(), PluginInstallError> {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return Ok(());
        }
        // 编排插件不在本次 Windows 通知契约内，保留原平台门槛且不创建 HOME。
        if !notification_patch::auto_install_supported() {
            return Err(notification_patch::unsupported());
        }
        let home = ensure_codex_home_dir()?;
        let mut log = String::new();
        let runtime = VerifiedRuntime::probe(
            PatchKind::Codex,
            &home,
            self.path_env_var.as_deref(),
            &mut log,
        )
        .await?;
        codex_source::install(&home, PLATFORM_PLUGIN_NAME, &runtime, &mut log).await?;
        if !platform_plugin_version_is_current(&home) {
            return Err(PluginInstallError {
                message: crate::t!("cli-agent-platform-plugin-install-not-effective"),
                log,
            });
        }
        Ok(())
    }
}

#[async_trait]
impl CliAgentPluginManager for CodexPluginManager {
    fn minimum_plugin_version(&self) -> &'static str {
        if FeatureFlag::CodexPlugin.is_enabled() {
            MINIMUM_PLUGIN_VERSION
        } else {
            "0.0.0"
        }
    }

    fn can_auto_install(&self) -> bool {
        notification_patch::auto_install_supported_for(PatchKind::Codex)
            && FeatureFlag::CodexPlugin.is_enabled()
            && !self.is_disabled()
            && !self.has_local_marketplace_override()
    }

    fn is_disabled(&self) -> bool {
        codex_home_dir()
            .ok()
            .is_some_and(|dir| plugin_enabled_setting(&dir, PLUGIN_KEY) == Some(false))
    }

    fn native_authorization_status(&self) -> NativeAuthorizationStatus {
        if !FeatureFlag::CodexPlugin.is_enabled() || !self.is_installed() {
            return NativeAuthorizationStatus::NotApplicable;
        }
        if cfg!(all(windows, not(target_arch = "x86_64"))) {
            // 正式 Windows 原生契约只验证了 x64，其他架构仍保持未知。
            return NativeAuthorizationStatus::Unknown;
        }
        match codex_home_dir() {
            Ok(home) => super::codex_hook_trust::status(&home),
            Err(_) => NativeAuthorizationStatus::Unknown,
        }
    }

    fn native_authorization_instructions(&self) -> Option<&'static PluginInstructions> {
        FeatureFlag::CodexPlugin
            .is_enabled()
            .then_some(&*NATIVE_AUTHORIZATION_INSTRUCTIONS)
    }

    fn is_installed(&self) -> bool {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return false;
        }
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        check_installed(&codex_dir)
    }

    fn needs_update(&self) -> bool {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return false;
        }
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        if codex_source::has_custom_source(&codex_dir) {
            return false;
        }
        check_installed(&codex_dir)
            && (!codex_source::is_current(&codex_dir)
                || !notification_patch::is_applied(&codex_dir, PatchKind::Codex))
    }

    fn is_platform_plugin_installed(&self) -> bool {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return false;
        }
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        check_platform_plugin_installed(&codex_dir)
    }

    fn platform_plugin_needs_update(&self) -> bool {
        if !FeatureFlag::CodexPlugin.is_enabled() {
            return false;
        }
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        if codex_source::has_custom_source(&codex_dir) {
            return false;
        }
        plugin_needs_update(
            &codex_dir,
            PLATFORM_PLUGIN_NAME,
            PLATFORM_PLUGIN_KEY,
            MINIMUM_PLATFORM_PLUGIN_VERSION,
        )
    }

    fn has_local_marketplace_override(&self) -> bool {
        let Ok(codex_dir) = codex_home_dir() else {
            return false;
        };
        codex_source::has_custom_source(&codex_dir)
    }

    async fn install(&self) -> Result<(), PluginInstallError> {
        self.notification_operation().await
    }

    async fn update(&self) -> Result<(), PluginInstallError> {
        self.notification_operation().await
    }

    fn install_success_message(&self) -> &'static str {
        crate::t_static!("cli-agent-plugin-codex-warp-installed")
    }

    fn update_success_message(&self) -> &'static str {
        crate::t_static!("cli-agent-plugin-codex-warp-updated")
    }

    fn install_instructions(&self) -> &'static PluginInstructions {
        if self.is_disabled() {
            &ENABLE_INSTRUCTIONS
        } else if FeatureFlag::CodexPlugin.is_enabled() {
            &PLUGIN_INSTALL_INSTRUCTIONS
        } else {
            &NATIVE_INSTALL_INSTRUCTIONS
        }
    }

    fn update_instructions(&self) -> &'static PluginInstructions {
        if FeatureFlag::CodexPlugin.is_enabled() {
            &PLUGIN_UPDATE_INSTRUCTIONS
        } else {
            &EMPTY_INSTRUCTIONS
        }
    }

    fn remote_install_instructions(&self) -> &'static PluginInstructions {
        if FeatureFlag::CodexPlugin.is_enabled() {
            &PLUGIN_INSTALL_INSTRUCTIONS
        } else {
            &NATIVE_INSTALL_INSTRUCTIONS
        }
    }

    fn supports_update(&self) -> bool {
        FeatureFlag::CodexPlugin.is_enabled()
    }

    async fn install_platform_plugin(&self) -> Result<(), PluginInstallError> {
        self.platform_operation().await
    }

    async fn update_platform_plugin(&self) -> Result<(), PluginInstallError> {
        self.platform_operation().await
    }
}

static ENABLE_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: crate::t_static!("cli-agent-plugin-codex-install-title"),
    subtitle: crate::t_static!("cli-agent-plugin-disabled"),
    steps: vec![PluginInstructionStep {
        description: crate::t_static!("cli-agent-plugin-enable-codex-config-step"),
        command: "[plugins.\"warp@codex-warp\"]\nenabled = true",
        executable: false,
        link: None,
    }],
    post_install_notes: vec![crate::t_static!(
        "cli-agent-plugin-installed-restart-session"
    )],
});

static NATIVE_AUTHORIZATION_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| {
    PluginInstructions {
        title: crate::t_static!("cli-agent-plugin-codex-hooks-title"),
        subtitle: crate::t_static!("cli-agent-plugin-codex-hooks-subtitle"),
        steps: vec![PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-codex-hooks-review-step"),
            command: "/hooks",
            // 这是 CLI 内的交互命令，不能作为普通 shell 命令自动执行。
            executable: false,
            link: None,
        }],
        post_install_notes: vec![crate::t_static!(
            "cli-agent-plugin-codex-hooks-activation-note"
        )],
    }
});

static PLUGIN_INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> =
    LazyLock::new(|| PluginInstructions {
        title: crate::t_static!("cli-agent-plugin-codex-warp-install-title"),
        subtitle: crate::t_static!("cli-agent-plugin-codex-run-commands-restart"),
        steps: vec![PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-codex-persistent-source-step"),
            command: "python3 apply_notification_patch.py --agent codex",
            // 导出包所在目录未知，不能在用户当前目录自动执行同名文件。
            executable: false,
            link: None,
        }],
        post_install_notes: vec![
            crate::t_static!("cli-agent-plugin-codex-activate-note"),
            crate::t_static!("cli-agent-plugin-codex-patch-manual-note"),
        ],
    });

static NATIVE_INSTALL_INSTRUCTIONS: LazyLock<PluginInstructions> =
    LazyLock::new(|| PluginInstructions {
        title: crate::t_static!("cli-agent-plugin-codex-install-title"),
        subtitle: crate::t_static!("cli-agent-plugin-codex-install-subtitle"),
        steps: vec![
            PluginInstructionStep {
                description: crate::t_static!("cli-agent-plugin-codex-update-step"),
                command: "",
                executable: false,
                link: Some("https://developers.openai.com/codex/cli#upgrade"),
            },
            PluginInstructionStep {
                description: crate::t_static!("cli-agent-plugin-codex-notification-step"),
                command: "[tui]\nnotification_condition = \"always\"",
                executable: false,
                link: None,
            },
        ],
        post_install_notes: vec![crate::t_static!("cli-agent-plugin-codex-restart-note")],
    });

static EMPTY_INSTRUCTIONS: LazyLock<PluginInstructions> = LazyLock::new(|| PluginInstructions {
    title: "",
    subtitle: "",
    steps: vec![],
    post_install_notes: vec![],
});

static PLUGIN_UPDATE_INSTRUCTIONS: LazyLock<PluginInstructions> =
    LazyLock::new(|| PluginInstructions {
        title: crate::t_static!("cli-agent-plugin-codex-warp-update-title"),
        subtitle: crate::t_static!("cli-agent-plugin-codex-run-commands-restart"),
        steps: vec![PluginInstructionStep {
            description: crate::t_static!("cli-agent-plugin-codex-persistent-source-step"),
            command: "python3 apply_notification_patch.py --agent codex",
            // 导出包所在目录未知，不能在用户当前目录自动执行同名文件。
            executable: false,
            link: None,
        }],
        post_install_notes: vec![
            crate::t_static!("cli-agent-plugin-codex-activate-update-note"),
            crate::t_static!("cli-agent-plugin-codex-marketplace-recovery-note"),
            crate::t_static!("cli-agent-plugin-codex-patch-manual-note"),
        ],
    });

fn check_installed(codex_dir: &Path) -> bool {
    check_plugin_enabled(codex_dir, PLUGIN_KEY)
}

fn check_platform_plugin_installed(codex_dir: &Path) -> bool {
    check_plugin_enabled(codex_dir, PLATFORM_PLUGIN_KEY)
}

/// Whether `config.toml` marks the given plugin key as enabled.
fn check_plugin_enabled(codex_dir: &Path, plugin_key: &str) -> bool {
    plugin_enabled_setting(codex_dir, plugin_key).unwrap_or(false)
}

fn plugin_enabled_setting(codex_dir: &Path, plugin_key: &str) -> Option<bool> {
    let config_path = codex_dir.join("config.toml");
    let Ok(contents) = fs::read_to_string(config_path) else {
        return None;
    };
    let Ok(parsed) = contents.parse::<toml_edit::DocumentMut>() else {
        return None;
    };
    parsed
        .get("plugins")
        .and_then(|plugins| plugins.get(plugin_key))
        .and_then(|plugin| plugin.get("enabled"))
        .and_then(|enabled| enabled.as_bool())
}

/// Reads the latest cached Warp plugin version, if present.
#[cfg(test)]
fn installed_version(codex_dir: &Path) -> Option<String> {
    installed_plugin_version(codex_dir, PLUGIN_NAME)
}

/// Reads the latest cached orchestration plugin version, if present.
fn installed_platform_plugin_version(codex_dir: &Path) -> Option<String> {
    installed_plugin_version(codex_dir, PLATFORM_PLUGIN_NAME)
}

fn platform_plugin_version_is_current(codex_dir: &Path) -> bool {
    installed_platform_plugin_version(codex_dir)
        .map(|v| !compare_versions(&v, MINIMUM_PLATFORM_PLUGIN_VERSION).is_lt())
        .unwrap_or(false)
}

/// Reads the latest cached version for `plugin_name` from
/// `plugins/cache/codex-warp/<plugin_name>/<version>/.codex-plugin/plugin.json`.
fn installed_plugin_version(codex_dir: &Path, plugin_name: &str) -> Option<String> {
    let cache_dir = codex_dir
        .join("plugins")
        .join("cache")
        .join(MARKETPLACE_NAME)
        .join(plugin_name);
    let entries = fs::read_dir(cache_dir).ok()?;
    let mut latest: Option<String> = None;
    for entry in entries.flatten() {
        let manifest_path = entry.path().join(".codex-plugin").join("plugin.json");
        let Some(version) = plugin_manifest_version(manifest_path) else {
            continue;
        };
        if latest
            .as_deref()
            .map(|current| compare_versions(&version, current).is_gt())
            .unwrap_or(true)
        {
            latest = Some(version);
        }
    }
    latest
}

fn plugin_manifest_version(manifest_path: impl AsRef<Path>) -> Option<String> {
    let contents = fs::read_to_string(manifest_path).ok()?;
    let parsed = serde_json::from_str::<Value>(&contents).ok()?;
    parsed
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

fn plugin_needs_update(
    codex_dir: &Path,
    plugin_name: &str,
    plugin_key: &str,
    minimum_version: &str,
) -> bool {
    if !check_plugin_enabled(codex_dir, plugin_key) {
        return false;
    }
    match installed_plugin_version(codex_dir, plugin_name) {
        Some(v) => compare_versions(&v, minimum_version).is_lt(),
        // No version field means very old plugin.
        None => true,
    }
}

/// Checks `CODEX_HOME` first, falls back to `~/.codex`.
fn codex_home_dir() -> io::Result<PathBuf> {
    if let Ok(codex_home) = env::var(CODEX_HOME_ENV)
        && !codex_home.is_empty()
    {
        return Ok(PathBuf::from(codex_home));
    }
    dirs::home_dir()
        .map(|home| home.join(CODEX_CONFIG_DIR))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "could not determine home directory",
            )
        })
}

/// Creates the resolved Codex home directory if it does not yet exist.
/// The Codex CLI expects `CODEX_HOME` to exist before running plugin commands, we need
/// this for self-hosted direct backend workers.
fn ensure_codex_home_dir() -> io::Result<PathBuf> {
    let dir = codex_home_dir()?;
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
