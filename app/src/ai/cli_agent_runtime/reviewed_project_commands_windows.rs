//! Windows 项目命令经固定 SDK MCP 交给宿主；不依赖 CLI 的原生 Shell 选择。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::RuntimeError;
use super::managed_process::ExpectedFileIdentity;
use super::reviewed_project_commands::{ProjectCommand, ReviewedCommand, reject};

pub(super) const TOOL_NAME: &str = "reviewed_project_command";
pub(super) const CLAUDE_TOOL: &str = "mcp__infinishell-local-tasks__reviewed_project_command";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct WindowsCommandBinding {
    pub capability: ProjectCommand,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub files: Vec<ExpectedFileIdentity>,
    pub cwd: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct WindowsCommandCeiling {
    version: u32,
    bindings: Vec<WindowsCommandBinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    executable: PathBuf,
    argv: Vec<String>,
    cwd: PathBuf,
    timeout_ms: u64,
}

impl WindowsCommandCeiling {
    pub(super) fn capture(
        commands: &mut BTreeSet<ProjectCommand>,
        cwd: &Path,
    ) -> Result<Self, RuntimeError> {
        if !cfg!(all(windows, target_arch = "x86_64")) {
            return Err(reject("windows_reviewed_commands_platform"));
        }
        let mut bindings = Vec::new();
        let mut identities: BTreeMap<PathBuf, ExpectedFileIdentity> = BTreeMap::new();
        for capability in commands.iter() {
            let candidate = match capability {
                ProjectCommand::CargoCheck
                | ProjectCommand::CargoTest
                | ProjectCommand::CargoFmtCheck => find_executable("cargo.exe")
                    .map(|path| (path, capability.argv()[1..].to_vec(), Vec::new())),
                ProjectCommand::Pytest => find_executable("python.exe")
                    .map(|path| (path, vec!["-m".into(), "pytest".into()], Vec::new())),
                ProjectCommand::PackageScript(name) => {
                    find_executable("node.exe").and_then(|node| {
                        let cli = node.parent()?.join("node_modules/npm/bin/npm-cli.js");
                        let package = node.parent()?.join("node_modules/npm/package.json");
                        let value: Value =
                            serde_json::from_slice(&std::fs::read(&package).ok()?).ok()?;
                        if value["name"] != "npm" || value["bin"]["npm"] != "bin/npm-cli.js" {
                            return None;
                        }
                        let cli = plain_file(&cli).ok()?;
                        let package = plain_file(&package).ok()?;
                        Some((
                            node,
                            vec![cli.to_str()?.into(), "run".into(), name.clone()],
                            vec![cli, package, plain_file(&cwd.join("package.json")).ok()?],
                        ))
                    })
                }
            };
            let Some((executable, arguments, dependencies)) = candidate else {
                continue;
            };
            let mut files = Vec::new();
            for path in std::iter::once(executable.clone()).chain(dependencies) {
                let identity = match identities.entry(path.clone()) {
                    std::collections::btree_map::Entry::Occupied(entry) => entry.get().clone(),
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(ExpectedFileIdentity::capture(&path)?).clone()
                    }
                };
                files.push(identity);
            }
            bindings.push(WindowsCommandBinding {
                capability: capability.clone(),
                executable,
                arguments,
                files,
                cwd: cwd.to_owned(),
            });
        }
        commands.retain(|command| {
            bindings
                .iter()
                .any(|binding| &binding.capability == command)
        });
        let result = Self {
            version: 1,
            bindings,
        };
        result.validate(commands)?;
        Ok(result)
    }

    pub(super) fn validate(&self, commands: &BTreeSet<ProjectCommand>) -> Result<(), RuntimeError> {
        if !cfg!(all(windows, target_arch = "x86_64"))
            || self.version != 1
            || self.bindings.is_empty()
            || self.bindings.len() != commands.len()
            || self
                .bindings
                .iter()
                .map(|entry| &entry.capability)
                .collect::<BTreeSet<_>>()
                .len()
                != self.bindings.len()
            || self
                .bindings
                .iter()
                .any(|entry| !commands.contains(&entry.capability) || entry.validate().is_err())
        {
            return Err(reject("windows_reviewed_commands_binding_invalid"));
        }
        Ok(())
    }

    pub(super) fn verify_sources(&self) -> Result<(), RuntimeError> {
        let mut sources = BTreeMap::new();
        for binding in &self.bindings {
            binding.validate()?;
            for file in &binding.files {
                if sources
                    .insert(file.path(), file)
                    .is_some_and(|previous| previous != file)
                {
                    return Err(reject("windows_reviewed_commands_conflicting_source"));
                }
            }
        }
        // 多个命名 npm 脚本共享解释器，不为每个脚本重复读取同一 PE。
        for (path, expected) in sources {
            if ExpectedFileIdentity::capture(path)? != *expected {
                return Err(reject("windows_reviewed_commands_source_changed"));
            }
        }
        Ok(())
    }

    pub(super) fn allows_child(&self, child: &Self) -> bool {
        self.version == child.version
            && child
                .bindings
                .iter()
                .all(|entry| self.bindings.contains(entry))
    }

    pub(super) fn instruction(&self, cwd: &Path, timeout: u64) -> String {
        let calls: Vec<_> = self.bindings.iter().map(|binding| json!({
            "executable":binding.executable, "argv":binding.arguments, "cwd":cwd, "timeoutMs":timeout
        })).collect();
        format!(
            "On Windows invoke the SDK MCP tool reviewed_project_command with exactly one of these argument objects (timeoutMs may be reduced): {}. Do not invoke native Bash or PowerShell. The app will request separate approval showing the exact executable, argv, working directory and timeout. Every command runs as the current OS account, not in an OS sandbox; project code, packages and scripts can access that account's files and network. Commands run sequentially in independent supervised processes. Wait for each command result and confirmed descendant cleanup before requesting the next command; each command needs fresh approval. Other tools retain their fixed policy and cannot expand this command set.",
            serde_json::to_string(&calls).expect("固定命令可序列化")
        )
    }

    pub(super) fn approve(
        &self,
        input: &Value,
        cwd: &Path,
        timeout: u64,
        digest: String,
    ) -> Option<ReviewedCommand> {
        let request: Request = serde_json::from_value(input.clone()).ok()?;
        if request.cwd != cwd || !(1..=timeout).contains(&request.timeout_ms) {
            return None;
        }
        self.verify_sources().ok()?;
        let binding = self.bindings.iter().find(|binding| {
            binding.executable == request.executable
                && binding.arguments == request.argv
                && binding.cwd == cwd
        })?;
        Some(ReviewedCommand {
            command: serde_json::to_string(
                &json!({"executable":binding.executable,"argv":binding.arguments}),
            )
            .ok()?,
            cwd: cwd.to_owned(),
            argv: binding.arguments.clone(),
            timeout_ms: request.timeout_ms,
            ceiling_sha256: digest,
            windows: Some(binding.clone()),
            unix: None,
        })
    }
}

impl WindowsCommandBinding {
    pub(super) fn validate(&self) -> Result<(), RuntimeError> {
        let expected_name = match &self.capability {
            ProjectCommand::CargoCheck
            | ProjectCommand::CargoTest
            | ProjectCommand::CargoFmtCheck => "cargo.exe",
            ProjectCommand::Pytest => "python.exe",
            ProjectCommand::PackageScript(_) => "node.exe",
        };
        if !self.cwd.is_absolute()
            || self.cwd.parent().is_none()
            || dunce::canonicalize(&self.cwd).ok().as_ref() != Some(&self.cwd)
            || !self.executable.is_absolute()
            || self
                .executable
                .file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| !name.eq_ignore_ascii_case(expected_name))
            || self
                .files
                .first()
                .is_none_or(|file| file.path() != self.executable)
            || self
                .files
                .iter()
                .map(|file| file.path())
                .collect::<BTreeSet<_>>()
                .len()
                != self.files.len()
        {
            return Err(reject("windows_reviewed_commands_executable_invalid"));
        }
        let expected = match &self.capability {
            ProjectCommand::PackageScript(name) => {
                let root = self
                    .executable
                    .parent()
                    .ok_or_else(|| reject("windows_reviewed_commands_node_root"))?;
                let cli = root.join("node_modules/npm/bin/npm-cli.js");
                if self.files.len() != 4
                    || self.files[1].path() != cli
                    || self.files[2].path() != root.join("node_modules/npm/package.json")
                    || self.files[3].path() != self.cwd.join("package.json")
                {
                    return Err(reject("windows_reviewed_commands_npm_binding"));
                }
                vec![
                    cli.to_str()
                        .ok_or_else(|| reject("windows_reviewed_commands_npm_path"))?
                        .into(),
                    "run".into(),
                    name.clone(),
                ]
            }
            ProjectCommand::CargoCheck
            | ProjectCommand::CargoTest
            | ProjectCommand::CargoFmtCheck
            | ProjectCommand::Pytest => {
                if self.files.len() != 1 {
                    return Err(reject("windows_reviewed_commands_dependency_invalid"));
                }
                self.capability.argv()[1..].to_vec()
            }
        };
        if self.arguments != expected {
            return Err(reject("windows_reviewed_commands_argv_changed"));
        }
        Ok(())
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|directory| directory.is_absolute())
        .find_map(|directory| plain_file(&directory.join(name)).ok())
}

fn plain_file(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = dunce::canonicalize(path)?;
    if canonical != path || !std::fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(std::io::Error::other(
            "Windows 项目工具必须是未重定向的普通文件",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        if std::fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0 {
            return Err(std::io::Error::other(
                "Windows 项目工具不得是 reparse point",
            ));
        }
    }
    Ok(canonical)
}

pub(super) fn tool_definition() -> Value {
    json!({"name":TOOL_NAME,"description":"Request one foreground project command through the supervised host. Exact executable/argv/cwd/timeout require explicit app approval; runs as the current OS account without an OS sandbox.",
        "inputSchema":{"type":"object","additionalProperties":false,"required":["executable","argv","cwd","timeoutMs"],"properties":{
            "executable":{"type":"string"},"argv":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":3},
            "cwd":{"type":"string"},"timeoutMs":{"type":"integer","minimum":1,"maximum":120000}}}})
}
