//! V2 在独立版本合同中增加 Write，旧 V1 记录和父上限不会自动取得新能力。

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::{BLOCKED_TOOLS, ClaudeRestrictedFilesV1, RuntimeError, reject};
use crate::ai::cli_agent_runtime::PermissionPolicy;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClaudeRestrictedFilesV2 {
    version: u32,
    cli_version: String,
    base: ClaudeRestrictedFilesV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClaudeFileProfile {
    V1(ClaudeRestrictedFilesV1),
    V2(ClaudeRestrictedFilesV2),
}

impl From<ClaudeRestrictedFilesV1> for ClaudeFileProfile {
    fn from(profile: ClaudeRestrictedFilesV1) -> Self {
        Self::V1(profile)
    }
}

impl ClaudeRestrictedFilesV2 {
    pub(crate) fn compile(base: ClaudeRestrictedFilesV1) -> Result<Self, RuntimeError> {
        let profile = Self {
            version: 2,
            cli_version: "2.1.280".into(),
            base,
        };
        profile.validate(&profile.base.working_directory)?;
        Ok(profile)
    }

    fn validate(&self, cwd: &Path) -> Result<(), RuntimeError> {
        self.base.validate(cwd)?;
        if self.version != 2
            || self.cli_version != "2.1.280"
            || !Self::executable_verified(&self.base.executable_sha256)
        {
            return Err(reject("claude_files_v2_identity_invalid"));
        }
        Ok(())
    }

    pub(crate) fn executable_verified(digest: &str) -> bool {
        let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => {
                Some("387a5c5dcdbb815085edf0baf79591f9d8894efe922bceaf3d75b1b08055229d")
            }
            ("macos", "x86_64") => {
                Some("c1d32d87630482250633208ab77855429b24010ae3086a7ff7539b57b93168d4")
            }
            ("linux", "x86_64") => {
                Some("1e08503dbdf3c2cb0d706d32f3408277388d1c76ef108673e8fe42c1b322925b")
            }
            ("windows", "x86_64") => {
                Some("0e4195524b73eb77efbdf3e2b36de5322a29f0ca575dfd2d9b4f946b1d425469")
            }
            _ => None,
        };
        expected == Some(digest)
    }

    fn settings(&self) -> Value {
        let mut settings = self.base.fixed_settings();
        settings["permissions"]["ask"]
            .as_array_mut()
            .expect("固定 V1 设置包含 ask 数组")
            .push(json!("Write"));
        settings
    }

    fn blocked_tools(&self) -> Vec<&'static str> {
        BLOCKED_TOOLS
            .into_iter()
            .filter(|tool| *tool != "Write")
            .collect()
    }

    fn approval_allowed(&self, tool: &str, input: &Value) -> bool {
        if tool != "Write" {
            return self.base.approval_allowed(tool, input);
        }
        if input.as_object().is_none_or(|fields| fields.len() != 2)
            || !input["content"].is_string()
            || !self.base.approval_allowed("Edit", input)
        {
            return false;
        }
        let Some(path) = input["file_path"].as_str().map(Path::new) else {
            return false;
        };
        let relative = match path
            .strip_prefix(&self.base.canonical_working_directory)
            .or_else(|_| path.strip_prefix(&self.base.working_directory))
        {
            Ok(relative) => relative,
            Err(_) => return false,
        };
        let destination = self.base.canonical_working_directory.join(relative);
        let mut current = self.base.canonical_working_directory.clone();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return false;
            };
            // 拒绝 Windows 别名和备用数据流，不能借此绕过配置目录保护。
            if name
                .to_str()
                .is_none_or(|name| name.ends_with(['.', ' ']) || name.contains(':'))
            {
                return false;
            }
            current.push(name);
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return false;
                    }
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt as _;
                        if metadata.file_attributes() & 0x400 != 0 {
                            return false;
                        }
                    }
                    if current == destination {
                        if !metadata.is_file() || !single_link_file(&current) {
                            return false;
                        }
                    } else if !metadata.is_dir() {
                        return false;
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && current == destination => {}
                Err(_) => return false,
            }
        }
        !relative.as_os_str().is_empty()
    }
}

// 不允许通过文件硬链接覆盖授权根以外或受保护目录中的同一文件。
fn single_link_file(path: &Path) -> bool {
    let mut options = std::fs::OpenOptions::new();
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
    let Ok(file) = options.open(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        file.metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.nlink() == 1)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle as _;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.is_ok()
            && info.nNumberOfLinks == 1
            && info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

impl ClaudeFileProfile {
    pub(crate) fn compile(
        policy: PermissionPolicy,
        base: ClaudeRestrictedFilesV1,
    ) -> Result<Self, RuntimeError> {
        match policy {
            PermissionPolicy::ClaudeRestrictedFilesV1 => Ok(Self::V1(base)),
            PermissionPolicy::ClaudeRestrictedFilesV2 => {
                ClaudeRestrictedFilesV2::compile(base).map(Self::V2)
            }
            PermissionPolicy::Inherit
            | PermissionPolicy::ReadOnly
            | PermissionPolicy::WorkspaceWrite
            | PermissionPolicy::GrokRestrictedReadV1
            | PermissionPolicy::GrokRestrictedFilesV1 => Err(reject("claude_profile_wrong_policy")),
        }
    }

    pub(crate) fn policy(&self) -> PermissionPolicy {
        match self {
            Self::V1(_) => PermissionPolicy::ClaudeRestrictedFilesV1,
            Self::V2(_) => PermissionPolicy::ClaudeRestrictedFilesV2,
        }
    }

    pub(crate) fn evidence_key(&self) -> &'static str {
        match self {
            Self::V1(_) => "claudeRestrictedFilesV1",
            Self::V2(_) => "claudeRestrictedFilesV2",
        }
    }

    pub(crate) fn validate(&self, cwd: &Path) -> Result<(), RuntimeError> {
        match self {
            Self::V1(profile) => profile.validate(cwd),
            Self::V2(profile) => profile.validate(cwd),
        }
    }

    pub(crate) fn verify_source(&self, observed: &Self) -> Result<(), RuntimeError> {
        if self != observed {
            return Err(reject("claude_profile_source_changed"));
        }
        Ok(())
    }

    pub(crate) fn arguments(&self) -> Vec<String> {
        match self {
            Self::V1(profile) => profile.arguments(),
            Self::V2(profile) => profile.base.arguments_for(
                "Read,Edit,Write",
                &profile.blocked_tools(),
                profile.settings(),
            ),
        }
    }

    pub(crate) fn verify_live(
        &self,
        settings: &Value,
        rules: &Value,
        hooks: &Value,
        mcp: &Value,
    ) -> Result<(), RuntimeError> {
        match self {
            Self::V1(profile) => profile.verify_live(settings, rules, hooks, mcp),
            Self::V2(profile) => profile.base.verify_live_for(
                settings,
                rules,
                hooks,
                mcp,
                &profile.settings(),
                &profile.blocked_tools(),
            ),
        }
    }

    pub(crate) fn approval_allowed(&self, tool: &str, input: &Value) -> bool {
        match self {
            Self::V1(profile) => profile.approval_allowed(tool, input),
            Self::V2(profile) => profile.approval_allowed(tool, input),
        }
    }

    pub(crate) fn verify_system_init(&self, message: &Value) -> Result<(), RuntimeError> {
        match self {
            Self::V1(profile) => profile.verify_system_init(message),
            Self::V2(profile) => profile
                .base
                .verify_system_init_for(message, &["Read", "Edit", "Write", "EndConversation"]),
        }
    }

    pub(crate) fn same_scope(&self, other: &Self) -> bool {
        self == other
    }

    pub(crate) fn digest(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("版本化 Claude 文件策略可以序列化"))
        )
    }
}

#[cfg(test)]
#[path = "claude_profile_v2_tests.rs"]
mod tests;
