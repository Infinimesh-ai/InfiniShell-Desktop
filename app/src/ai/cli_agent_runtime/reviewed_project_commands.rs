//! 固定项目命令能力集合；它限定可审批的入口，不提供操作系统沙箱。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::RuntimeError;

pub(super) const MAX_TIMEOUT_MS: u64 = 120_000;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum ProjectCommand {
    CargoCheck,
    CargoTest,
    CargoFmtCheck,
    Pytest,
    PackageScript(String),
}

impl ProjectCommand {
    pub(super) fn argv(&self) -> Vec<String> {
        match self {
            Self::CargoCheck => vec!["cargo".into(), "check".into()],
            Self::CargoTest => vec!["cargo".into(), "test".into()],
            Self::CargoFmtCheck => vec!["cargo".into(), "fmt".into(), "--check".into()],
            Self::Pytest => vec!["python".into(), "-m".into(), "pytest".into()],
            Self::PackageScript(name) => vec!["npm".into(), "run".into(), name.clone()],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct ReviewedCommandCeilingV1 {
    version: u32,
    platform: String,
    cwd: PathBuf,
    commands: BTreeSet<ProjectCommand>,
    package_sha256: Option<String>,
    max_timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    windows: Option<super::reviewed_project_commands_windows::WindowsCommandCeiling>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unix: Option<super::reviewed_project_commands_unix::UnixCommandCeiling>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewedCommand {
    pub command: String,
    pub cwd: PathBuf,
    pub argv: Vec<String>,
    pub timeout_ms: u64,
    pub ceiling_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows: Option<super::reviewed_project_commands_windows::WindowsCommandBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unix: Option<super::reviewed_project_commands_unix::UnixCommandBinding>,
}

pub(super) fn reject(reason: &str) -> RuntimeError {
    super::permissions::rejected(
        None,
        &json!({"profile":"ReviewedCommandsV1", "osSandbox":false}),
        reason,
        false,
    )
}

impl ReviewedCommandCeilingV1 {
    pub(super) fn capture(cwd: &Path) -> Result<Self, RuntimeError> {
        let cwd = if cfg!(windows) {
            dunce::canonicalize(cwd)?
        } else {
            cwd.canonicalize()?
        };
        let mut commands = BTreeSet::from([
            ProjectCommand::CargoCheck,
            ProjectCommand::CargoTest,
            ProjectCommand::CargoFmtCheck,
            ProjectCommand::Pytest,
        ]);
        let package = read_package(&cwd)?;
        if let Some((value, _)) = &package {
            if let Some(scripts) = value.get("scripts") {
                for (name, body) in scripts
                    .as_object()
                    .ok_or_else(|| reject("reviewed_commands_package_scripts_shape"))?
                {
                    if valid_script_name(name) && body.is_string() {
                        commands.insert(ProjectCommand::PackageScript(name.clone()));
                    }
                }
            }
        }
        let windows = if cfg!(windows) {
            Some(
                super::reviewed_project_commands_windows::WindowsCommandCeiling::capture(
                    &mut commands,
                    &cwd,
                )?,
            )
        } else {
            None
        };
        let unix = if cfg!(unix) {
            Some(
                super::reviewed_project_commands_unix::UnixCommandCeiling::capture(
                    &mut commands,
                    &cwd,
                )?,
            )
        } else {
            None
        };
        let ceiling = Self {
            version: 1,
            platform: std::env::consts::OS.into(),
            cwd,
            commands,
            package_sha256: package.map(|(_, digest)| digest),
            max_timeout_ms: MAX_TIMEOUT_MS,
            windows,
            unix,
        };
        ceiling.validate()?;
        Ok(ceiling)
    }

    pub(super) fn validate(&self) -> Result<(), RuntimeError> {
        // Windows 仅使用创建时捕获的宿主 MCP 后端，不能发送 POSIX 前缀。
        if !matches!(
            (std::env::consts::OS, std::env::consts::ARCH),
            ("macos", "aarch64") | ("linux", "x86_64") | ("windows", "x86_64")
        ) || self.platform != std::env::consts::OS
            || self.version != 1
            || !self.cwd.is_absolute()
            || self.cwd.parent().is_none()
            || self
                .cwd
                .to_str()
                .is_none_or(|cwd| cwd.chars().any(char::is_control))
            || (if cfg!(windows) {
                dunce::canonicalize(&self.cwd)
            } else {
                self.cwd.canonicalize()
            })
            .ok()
            .as_ref()
                != Some(&self.cwd)
            || self.commands.is_empty()
            || self.commands.len() > 256
            || !(1..=MAX_TIMEOUT_MS).contains(&self.max_timeout_ms)
            || self.commands.iter().any(|command| match command {
                ProjectCommand::PackageScript(name) => {
                    !valid_script_name(name) || self.package_sha256.is_none()
                }
                ProjectCommand::CargoCheck
                | ProjectCommand::CargoTest
                | ProjectCommand::CargoFmtCheck
                | ProjectCommand::Pytest => false,
            })
        {
            return Err(reject("reviewed_commands_identity_or_platform_unverified"));
        }
        match (&self.windows, &self.unix) {
            (Some(windows), None) if self.platform == "windows" => {
                windows.validate(&self.commands)?
            }
            (None, Some(unix)) if matches!(self.platform.as_str(), "macos" | "linux") => {
                unix.validate(&self.commands)?
            }
            // 未交付旧草稿中没有精确来源的命令记录不能自动获得宿主执行能力。
            (Some(_), None) | (None, Some(_)) | (Some(_), Some(_)) | (None, None) => {
                return Err(reject("reviewed_commands_backend_mismatch"));
            }
        }
        Ok(())
    }

    pub(super) fn verify_sources(&self) -> Result<(), RuntimeError> {
        self.validate()?;
        if let Some(windows) = &self.windows {
            windows.verify_sources()?;
        }
        if let Some(unix) = &self.unix {
            unix.verify_sources()?;
        }
        if read_package(&self.cwd)?.map(|(_, digest)| digest) != self.package_sha256 {
            return Err(reject("reviewed_commands_package_changed"));
        }
        Ok(())
    }

    pub(super) fn allows_child(&self, child: &Self) -> bool {
        self.validate().is_ok()
            && child.validate().is_ok()
            && self.version == child.version
            && self.platform == child.platform
            && self.cwd == child.cwd
            && self.package_sha256 == child.package_sha256
            && child.commands.is_subset(&self.commands)
            && child.max_timeout_ms <= self.max_timeout_ms
            && match (&self.windows, &child.windows) {
                (Some(parent), Some(child)) => parent.allows_child(child),
                (None, None) => true,
                (Some(_), None) | (None, Some(_)) => false,
            }
            && match (&self.unix, &child.unix) {
                (Some(parent), Some(child)) => parent.allows_child(child),
                (None, None) => true,
                (Some(_), None) | (None, Some(_)) => false,
            }
    }

    pub(super) fn max_timeout_ms(&self) -> u64 {
        self.max_timeout_ms
    }

    pub(super) fn matches_directory(&self, cwd: &Path) -> bool {
        (if cfg!(windows) {
            dunce::canonicalize(cwd)
        } else {
            cwd.canonicalize()
        })
        .is_ok_and(|cwd| cwd == self.cwd)
    }

    pub(super) fn instruction(&self) -> String {
        self.instruction_for_skills(false)
    }

    pub(super) fn instruction_with_skills(&self) -> String {
        self.instruction_for_skills(true)
    }

    fn instruction_for_skills(&self, skills: bool) -> String {
        let mut instruction = match (&self.windows, &self.unix) {
            (Some(windows), None) => windows.instruction(&self.cwd, self.max_timeout_ms),
            (None, Some(unix)) => unix.instruction(&self.cwd, self.max_timeout_ms),
            (Some(_), Some(_)) | (None, None) => return String::new(),
        };
        if skills {
            instruction.push_str(" Invoke only registered native skills, each with individual approval. A skill cannot expand the approved command set. Hooks and skill shell interpolation remain unavailable.");
        }
        instruction
    }

    pub(super) fn approve_host(&self, input: &Value) -> Option<ReviewedCommand> {
        self.verify_sources().ok()?;
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(self).ok()?));
        match (&self.windows, &self.unix) {
            (Some(windows), None) => windows.approve(input, &self.cwd, self.max_timeout_ms, digest),
            (None, Some(unix)) => unix.approve(input, &self.cwd, self.max_timeout_ms, digest),
            (Some(_), Some(_)) | (None, None) => None,
        }
    }

    pub(super) fn is_host(&self) -> bool {
        self.windows.is_some() ^ self.unix.is_some()
    }
}

impl ReviewedCommand {
    pub(super) fn approval_context(&self) -> Value {
        json!({"reviewedProjectCommand":self, "backend":"supervised_host_mcp", "osSandbox":false,
            "scope":"current_os_account", "approval":"allow_once_or_deny_once",
            "cleanup":"confirm_each_command_generation_before_next_approval"})
    }
}

fn valid_script_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.'))
}

fn read_package(cwd: &Path) -> Result<Option<(Value, String)>, RuntimeError> {
    let path = cwd.join("package.json");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file()
        || metadata.len() > 1024 * 1024
        || (if cfg!(windows) {
            dunce::canonicalize(&path)?
        } else {
            path.canonicalize()?
        }) != path
    {
        return Err(reject("reviewed_commands_package_file_invalid"));
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() > 1024 * 1024 {
        return Err(reject("reviewed_commands_package_file_invalid"));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| reject("reviewed_commands_package_json_invalid"))?;
    if !value.is_object() {
        return Err(reject("reviewed_commands_package_json_invalid"));
    }
    Ok(Some((value, format!("{:x}", Sha256::digest(bytes)))))
}
