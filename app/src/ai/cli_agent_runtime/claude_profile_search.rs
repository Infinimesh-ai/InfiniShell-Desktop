//! 独立 V3 策略允许原生 Grep/Glob，旧 V1/V2 记录不会自动扩展。

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{ClaudeRestrictedFilesV1, ClaudeRestrictedFilesV2, RuntimeError, reject};
use crate::ai::cli_agent_runtime::file_search_scope::{
    bounded_pattern, relative_glob, resolve_search_path,
};

#[path = "claude_profile_skills.rs"]
mod skills;
pub use skills::{ClaudeRestrictedSkillsV1, ClaudeReviewedCommandsSkillsV1};
#[path = "claude_profile_commands.rs"]
mod commands;
pub use commands::ClaudeReviewedCommandsV1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClaudeRestrictedFilesV3 {
    version: u32,
    cli_version: String,
    base: ClaudeRestrictedFilesV2,
}

impl ClaudeRestrictedFilesV3 {
    pub(super) fn compile(base: ClaudeRestrictedFilesV1) -> Result<Self, RuntimeError> {
        Ok(Self {
            version: 3,
            cli_version: "2.1.280".into(),
            base: ClaudeRestrictedFilesV2::compile(base)?,
        })
    }

    pub(super) fn validate(&self, cwd: &Path) -> Result<(), RuntimeError> {
        self.base.validate(cwd)?;
        if self.version != 3 || self.cli_version != "2.1.280" {
            return Err(reject("claude_files_v3_identity_invalid"));
        }
        Ok(())
    }

    fn settings(&self) -> Value {
        let mut settings = self.base.settings();
        settings["permissions"]["ask"]
            .as_array_mut()
            .expect("固定策略包含 ask")
            .extend([json!("Grep"), json!("Glob")]);
        settings
    }

    pub(super) fn arguments(&self) -> Vec<String> {
        let mut arguments = self.base.base.arguments_for(
            "Read,Edit,Write,Grep,Glob",
            &self.base.blocked_tools(),
            self.settings(),
        );
        arguments.extend([
            "--append-system-prompt".into(),
            "This managed task permits project file tools. Use Glob to find paths, then Grep with an explicit regular file path inside the project; recursive directory content searches are unavailable. Search and write operations require individual approval. Shell commands and skills are unavailable.".into(),
        ]);
        arguments
    }

    pub(super) fn verify_live(
        &self,
        settings: &Value,
        rules: &Value,
        hooks: &Value,
        mcp: &Value,
    ) -> Result<(), RuntimeError> {
        self.base.base.verify_live_for(
            settings,
            rules,
            hooks,
            mcp,
            &self.settings(),
            &self.base.blocked_tools(),
        )
    }

    pub(super) fn verify_system_init(&self, message: &Value) -> Result<(), RuntimeError> {
        self.base.base.verify_system_init_for(
            message,
            &["Read", "Edit", "Write", "Grep", "Glob", "EndConversation"],
        )
    }

    pub(super) fn approval_allowed(&self, tool: &str, input: &Value) -> bool {
        if !matches!(tool, "Grep" | "Glob") {
            return self.base.approval_allowed(tool, input);
        }
        if self.validate(&self.base.base.working_directory).is_err() {
            return false;
        }
        let Some(fields) = input.as_object() else {
            return false;
        };
        if input.get("path").is_some_and(|value| !value.is_string()) {
            return false;
        }
        let Some(path) = resolve_search_path(
            &self.base.base.canonical_working_directory,
            input["path"].as_str(),
        ) else {
            return false;
        };
        let Some(pattern) = input["pattern"].as_str() else {
            return false;
        };
        if tool == "Glob" {
            return path.is_dir()
                && relative_glob(pattern)
                && fields
                    .keys()
                    .all(|key| matches!(key.as_str(), "pattern" | "path"));
        }
        // 内容搜索逐次绑定普通文件，避免目录递归穿过受保护配置或链接。
        path.is_file()
            && bounded_pattern(pattern)
            && fields.iter().all(|(key, value)| match key.as_str() {
                "pattern" | "path" => value.is_string(),
                "glob" => value.as_str().is_some_and(relative_glob),
                "type" => value.as_str().is_some_and(|kind| {
                    !kind.is_empty()
                        && kind.len() <= 64
                        && kind
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                }),
                "output_mode" => matches!(
                    value.as_str(),
                    Some("content" | "files_with_matches" | "count")
                ),
                "-i" | "-n" | "multiline" => value.is_boolean(),
                "-A" | "-B" | "-C" | "context" | "head_limit" | "offset" => {
                    value.as_u64().is_some_and(|count| count <= 100_000)
                }
                _ => false,
            })
    }
}
