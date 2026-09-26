//! Unix 项目命令的创建时来源；工具角色、公开入口、实际映像与 argv[0] 分别绑定。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::RuntimeError;
use super::managed_process::ExpectedFileIdentity;
use super::reviewed_project_commands::{ProjectCommand, ReviewedCommand, reject};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct UnixDirectoryIdentity {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
}

impl UnixDirectoryIdentity {
    pub(super) fn capture(path: &Path) -> Result<Self, RuntimeError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let path = path.canonicalize()?;
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || !safe_path(&path) {
                return Err(reject("unix_reviewed_commands_directory_invalid"));
            }
            return Ok(Self {
                path,
                device: metadata.dev(),
                inode: metadata.ino(),
            });
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(reject("unix_reviewed_commands_platform"))
        }
    }

    pub(super) fn verify(&self) -> Result<(), RuntimeError> {
        if Self::capture(&self.path)? != *self {
            return Err(reject("unix_reviewed_commands_directory_changed"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct UnixCommandBinding {
    pub capability: ProjectCommand,
    pub entry: PathBuf,
    pub executable: PathBuf,
    pub argv0: String,
    pub arguments: Vec<String>,
    pub files: Vec<ExpectedFileIdentity>,
    pub directory: UnixDirectoryIdentity,
    pub npm_entry: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct UnixCommandCeiling {
    version: u32,
    bindings: Vec<UnixCommandBinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    executable: PathBuf,
    argv: Vec<String>,
    cwd: PathBuf,
    timeout_ms: u64,
}

impl UnixCommandCeiling {
    pub(super) fn capture(
        commands: &mut BTreeSet<ProjectCommand>,
        cwd: &Path,
    ) -> Result<Self, RuntimeError> {
        if !supported_platform() {
            return Err(reject("unix_reviewed_commands_platform"));
        }
        let directory = UnixDirectoryIdentity::capture(cwd)?;
        let mut bindings = Vec::new();
        let mut identities = BTreeMap::<PathBuf, ExpectedFileIdentity>::new();
        for capability in commands.iter() {
            let role = match capability {
                ProjectCommand::CargoCheck
                | ProjectCommand::CargoTest
                | ProjectCommand::CargoFmtCheck => "cargo",
                ProjectCommand::Pytest => "python",
                ProjectCommand::PackageScript(_) => "node",
            };
            let Some((entry, executable)) = find_executable(role) else {
                continue;
            };
            let mut paths = vec![executable.clone()];
            let (arguments, npm_entry) = if let ProjectCommand::PackageScript(name) = capability {
                let Some((npm_entry, cli)) = find_executable("npm") else {
                    continue;
                };
                let Some(root) = cli.parent().and_then(Path::parent) else {
                    continue;
                };
                let package = root.join("package.json");
                let value = match package_json(&package) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if cli.file_name().is_none_or(|name| name != "npm-cli.js")
                    || cli
                        .parent()
                        .and_then(Path::file_name)
                        .is_none_or(|name| name != "bin")
                    || value["name"] != "npm"
                    || value["bin"]["npm"] != "bin/npm-cli.js"
                    || package.canonicalize().ok().as_ref() != Some(&package)
                {
                    continue;
                }
                // 项目脚本正文由创建时 package.json 固定，最终 worker 也核对此源文件。
                paths.extend([cli.clone(), package, directory.path.join("package.json")]);
                (
                    vec![
                        cli.to_str()
                            .ok_or_else(|| reject("unix_reviewed_commands_npm_path"))?
                            .into(),
                        "run".into(),
                        name.clone(),
                    ],
                    Some(npm_entry),
                )
            } else {
                (capability.argv()[1..].to_vec(), None)
            };
            let mut files = Vec::new();
            for path in paths {
                let identity = match identities.entry(path.clone()) {
                    std::collections::btree_map::Entry::Occupied(entry) => entry.get().clone(),
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(ExpectedFileIdentity::capture(&path)?).clone()
                    }
                };
                files.push(identity);
            }
            bindings.push(UnixCommandBinding {
                capability: capability.clone(),
                entry,
                executable,
                argv0: role.into(),
                arguments,
                files,
                directory: directory.clone(),
                npm_entry,
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
        if !supported_platform()
            || self.version != 1
            || self.bindings.is_empty()
            || self.bindings.len() != commands.len()
            || self
                .bindings
                .iter()
                .map(|binding| &binding.capability)
                .collect::<BTreeSet<_>>()
                .len()
                != self.bindings.len()
            || self.bindings.iter().any(|binding| {
                !commands.contains(&binding.capability) || binding.validate().is_err()
            })
            || self
                .bindings
                .iter()
                .any(|binding| binding.directory != self.bindings[0].directory)
        {
            return Err(reject("unix_reviewed_commands_binding_invalid"));
        }
        Ok(())
    }

    pub(super) fn verify_sources(&self) -> Result<(), RuntimeError> {
        let mut identities = BTreeMap::new();
        for binding in &self.bindings {
            binding.validate()?;
            binding.verify_entry()?;
            for file in &binding.files {
                if identities
                    .insert(file.path(), file)
                    .is_some_and(|previous| previous != file)
                {
                    return Err(reject("unix_reviewed_commands_conflicting_source"));
                }
            }
        }
        for (path, expected) in identities {
            if ExpectedFileIdentity::capture(path)? != *expected {
                return Err(reject("unix_reviewed_commands_source_changed"));
            }
        }
        Ok(())
    }

    pub(super) fn allows_child(&self, child: &Self) -> bool {
        self.version == child.version
            && child
                .bindings
                .iter()
                .all(|binding| self.bindings.contains(binding))
    }

    pub(super) fn instruction(&self, cwd: &Path, timeout: u64) -> String {
        let calls: Vec<_> = self.bindings.iter().map(|binding| json!({"executable":binding.executable,"argv":binding.arguments,"cwd":cwd,"timeoutMs":timeout})).collect();
        format!(
            "Use only the SDK MCP tool reviewed_project_command with one of these exact argument objects (timeoutMs may be reduced): {}. Do not invoke native Bash or another shell tool. The app requests separate approval for every command and supervises each command separately. Wait for the actual process cleanup result before requesting another command; an unknown result does not authorize retry. Commands run as the current OS account without an OS sandbox. Project code, packages and scripts may access that account's files and network. Other tools cannot expand the approved command set.",
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
            binding.directory.path == cwd
                && binding.executable == request.executable
                && binding.arguments == request.argv
        })?;
        Some(ReviewedCommand { command: serde_json::to_string(&json!({"executable":binding.executable,"argv0":binding.argv0,"argv":binding.arguments})).ok()?, cwd: cwd.to_owned(), argv: binding.arguments.clone(), timeout_ms: request.timeout_ms, ceiling_sha256: digest, windows: None, unix: Some(binding.clone()) })
    }
}

impl UnixCommandBinding {
    pub(super) fn validate(&self) -> Result<(), RuntimeError> {
        let role = match &self.capability {
            ProjectCommand::CargoCheck
            | ProjectCommand::CargoTest
            | ProjectCommand::CargoFmtCheck => "cargo",
            ProjectCommand::Pytest => "python",
            ProjectCommand::PackageScript(_) => "node",
        };
        if !supported_platform()
            || !safe_path(&self.entry)
            || !safe_path(&self.executable)
            || self.entry.file_name().is_none_or(|name| name != role)
            || self.argv0 != role
            || !safe_path(&self.directory.path)
            || self.directory.inode == 0
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
            return Err(reject("unix_reviewed_commands_executable_invalid"));
        }
        let expected = if let ProjectCommand::PackageScript(name) = &self.capability {
            if !valid_script(name)
                || self.files.len() != 4
                || self.npm_entry.as_ref().is_none_or(|path| {
                    !safe_path(path) || path.file_name().is_none_or(|name| name != "npm")
                })
            {
                return Err(reject("unix_reviewed_commands_npm_binding"));
            }
            let cli = self.files[1].path();
            let root = cli
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| reject("unix_reviewed_commands_npm_root"))?;
            if cli != root.join("bin/npm-cli.js")
                || self.files[2].path() != root.join("package.json")
                || self.files[3].path() != self.directory.path.join("package.json")
            {
                return Err(reject("unix_reviewed_commands_npm_binding"));
            }
            vec![
                cli.to_str()
                    .ok_or_else(|| reject("unix_reviewed_commands_npm_path"))?
                    .into(),
                "run".into(),
                name.clone(),
            ]
        } else {
            if self.files.len() != 1 || self.npm_entry.is_some() {
                return Err(reject("unix_reviewed_commands_dependency_invalid"));
            }
            self.capability.argv()[1..].to_vec()
        };
        if self.arguments != expected {
            return Err(reject("unix_reviewed_commands_argv_changed"));
        }
        Ok(())
    }

    pub(super) fn verify_entry(&self) -> Result<(), RuntimeError> {
        self.directory.verify()?;
        if self.entry.canonicalize()? != self.executable
            || self
                .files
                .iter()
                .any(|file| file.path().canonicalize().ok().as_deref() != Some(file.path()))
        {
            return Err(reject("unix_reviewed_commands_entry_changed"));
        }
        if let Some(entry) = &self.npm_entry {
            if entry.canonicalize()? != self.files[1].path() {
                return Err(reject("unix_reviewed_commands_npm_entry_changed"));
            }
        }
        Ok(())
    }
}

fn supported_platform() -> bool {
    cfg!(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64")
    ))
}
fn safe_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|value| !value.chars().any(char::is_control))
        && !path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
}
fn valid_script(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':' | b'.'))
}
fn find_executable(name: &str) -> Option<(PathBuf, PathBuf)> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|path| path.is_absolute())
        .find_map(|parent| {
            let entry = parent.join(name);
            let canonical = entry.canonicalize().ok()?;
            if !safe_path(&entry) || !safe_path(&canonical) || !canonical.is_file() {
                return None;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                if std::fs::metadata(&canonical).ok()?.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            Some((entry, canonical))
        })
}
fn package_json(path: &Path) -> std::io::Result<Value> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(std::io::Error::other("npm 注册清单无效"));
    }
    serde_json::from_slice(&std::fs::read(path)?).map_err(std::io::Error::other)
}
