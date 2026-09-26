//! 独立的受审项目命令策略；旧文件与技能策略不自动获得 Bash。

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{ClaudeRestrictedFilesV1, ClaudeRestrictedFilesV3, RuntimeError, reject};
use crate::ai::cli_agent_runtime::reviewed_project_commands::ReviewedCommandCeilingV1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClaudeReviewedCommandsV1 {
    version: u32,
    base: ClaudeRestrictedFilesV3,
    commands: ReviewedCommandCeilingV1,
}

impl ClaudeReviewedCommandsV1 {
    pub(crate) fn compile(base: ClaudeRestrictedFilesV1) -> Result<Self, RuntimeError> {
        // 基础策略拒绝未验证的设置来源，不能忽略来源中的 Bash 拒绝规则。
        base.validate(&base.working_directory)?;
        let commands = ReviewedCommandCeilingV1::capture(&base.working_directory)?;
        Ok(Self {
            version: 1,
            base: ClaudeRestrictedFilesV3::compile(base)?,
            commands,
        })
    }

    pub(crate) fn validate(&self, cwd: &Path) -> Result<(), RuntimeError> {
        self.base.validate(cwd)?;
        self.commands.validate()?;
        if self
            .base
            .base
            .base
            .local_tools
            .is_some_and(|tools| tools.allow_project_commands)
            != self.commands.is_host()
        {
            return Err(reject("claude_reviewed_commands_mcp_permission_mismatch"));
        }
        if self.version != 1 || !self.commands.matches_directory(cwd) {
            return Err(reject("claude_reviewed_commands_identity"));
        }
        Ok(())
    }

    pub(crate) fn allows_child(&self, child: &Self) -> bool {
        self.version == child.version
            && self.base == child.base
            && self.commands.allows_child(&child.commands)
    }

    fn blocked_tools(&self) -> Vec<&'static str> {
        self.base.base.blocked_tools().into_iter().collect()
    }

    fn settings(&self) -> Value {
        let mut settings = self.base.settings();
        settings["disableAllHooks"] = json!(true);
        settings["disableSkillShellExecution"] = json!(true);
        settings["sandbox"] = json!({"enabled":false, "autoAllowBashIfSandboxed":false});
        settings
    }

    pub(crate) fn arguments(&self) -> Vec<String> {
        let mut arguments = self.base.base.base.arguments_for(
            "Read,Edit,Write,Grep,Glob",
            &self.blocked_tools(),
            self.settings(),
        );
        arguments.extend(["--append-system-prompt".into(), self.commands.instruction()]);
        arguments
    }

    pub(crate) fn verify_live(
        &self,
        settings: &Value,
        rules: &Value,
        hooks: &Value,
        mcp: &Value,
    ) -> Result<(), RuntimeError> {
        self.commands.verify_sources()?;
        self.base.base.base.verify_live_for(
            settings,
            rules,
            hooks,
            mcp,
            &self.settings(),
            &self.blocked_tools(),
        )?;
        verify_live_command_ask(rules)
    }

    pub(crate) fn verify_system_init(&self, message: &Value) -> Result<(), RuntimeError> {
        let tools: &[&str] = &["Read", "Edit", "Write", "Grep", "Glob", "EndConversation"];
        self.base.base.base.verify_system_init_for(message, tools)
    }

    pub(crate) fn host_commands(&self) -> Option<&ReviewedCommandCeilingV1> {
        self.commands.is_host().then_some(&self.commands)
    }

    pub(crate) fn approval_allowed(&self, tool: &str, input: &Value) -> bool {
        if tool == "Bash" {
            false
        } else {
            self.base.approval_allowed(tool, input)
        }
    }
}

pub(super) fn verify_live_command_ask(rules: &Value) -> Result<(), RuntimeError> {
    let rules = rules["state"]["rules"]
        .as_array()
        .ok_or_else(|| reject("claude_reviewed_commands_rules_missing"))?;
    if !rules.iter().any(|rule| {
        rule["source"] == "flagSettings"
            && rule["behavior"] == "ask"
            && rule["rule"]
                == crate::ai::cli_agent_runtime::reviewed_project_commands_windows::CLAUDE_TOOL
            && rule["notInEffect"] != true
    }) {
        return Err(reject("claude_reviewed_commands_live_ask_missing"));
    }
    Ok(())
}
