//! 显式组合固定原生技能与受审命令；两组父能力都必须保持原来源与范围。

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::super::commands::verify_live_command_ask;
use super::{ClaudeRestrictedFilesV1, ClaudeRestrictedSkillsV1, RuntimeError, reject};
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::reviewed_project_commands::ReviewedCommandCeilingV1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClaudeReviewedCommandsSkillsV1 {
    version: u32,
    skills: ClaudeRestrictedSkillsV1,
    commands: ReviewedCommandCeilingV1,
}

impl ClaudeReviewedCommandsSkillsV1 {
    pub(crate) fn compile(
        base: ClaudeRestrictedFilesV1,
        selected: &[SelectedLocalSkill],
    ) -> Result<Self, RuntimeError> {
        let commands = ReviewedCommandCeilingV1::capture(&base.working_directory)?;
        let result = Self {
            version: 1,
            skills: ClaudeRestrictedSkillsV1::compile(base, selected)?,
            commands,
        };
        result.validate(&result.skills.base.base.base.working_directory)?;
        Ok(result)
    }

    pub(crate) fn validate(&self, cwd: &Path) -> Result<(), RuntimeError> {
        self.skills.validate(cwd)?;
        self.commands.validate()?;
        if self
            .skills
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
            return Err(reject("claude_command_skills_identity_changed"));
        }
        Ok(())
    }

    pub(crate) fn skills_profile(&self) -> &ClaudeRestrictedSkillsV1 {
        &self.skills
    }

    pub(crate) fn derive_child(
        &self,
        selected: &[SelectedLocalSkill],
    ) -> Result<Self, RuntimeError> {
        let mut child = self.clone();
        child.skills = self.skills.derive_child(selected)?;
        self.commands.verify_sources()?;
        Ok(child)
    }

    pub(crate) fn allows_child(&self, child: &Self) -> bool {
        self.version == child.version
            && self.skills.allows_child(&child.skills)
            && self.commands.allows_child(&child.commands)
    }

    fn blocked_tools(&self) -> Vec<&'static str> {
        self.skills.blocked_tools().into_iter().collect()
    }

    fn settings(&self) -> Value {
        let mut settings = self.skills.settings();
        settings["sandbox"] = json!({"enabled":false,"autoAllowBashIfSandboxed":false});
        settings
    }

    pub(crate) fn arguments(&self) -> Vec<String> {
        let mut arguments = self.skills.base.base.base.arguments_for(
            "Read,Edit,Write,Grep,Glob,Skill",
            &self.blocked_tools(),
            self.settings(),
        );
        arguments.retain(|argument| argument != "--disable-slash-commands");
        arguments.extend([
            "--append-system-prompt".into(),
            self.commands.instruction_with_skills(),
        ]);
        arguments
    }

    pub(crate) fn verify_live(
        &self,
        settings: &Value,
        rules: &Value,
        hooks: &Value,
        mcp: &Value,
    ) -> Result<(), RuntimeError> {
        self.validate(&self.skills.base.base.base.working_directory)?;
        self.commands.verify_sources()?;
        self.skills.base.base.base.verify_live_for(
            settings,
            rules,
            hooks,
            mcp,
            &self.settings(),
            &self.blocked_tools(),
        )?;
        verify_live_command_ask(rules)?;
        if rules["state"]["rules"].as_array().is_none_or(|rows| {
            !rows.iter().any(|rule| {
                rule["source"] == "flagSettings"
                    && rule["behavior"] == "ask"
                    && rule["rule"] == "Skill"
                    && rule["notInEffect"] != true
            })
        }) {
            return Err(reject("claude_command_skills_live_ask_missing"));
        }
        Ok(())
    }

    pub(crate) fn verify_system_init(&self, message: &Value) -> Result<(), RuntimeError> {
        let required: &[&str] = &["Skill"];
        if message["tools"].as_array().is_none_or(|tools| {
            required
                .iter()
                .any(|expected| !tools.iter().any(|tool| tool.as_str() == Some(*expected)))
        }) {
            return Err(reject("claude_command_skills_native_tool_missing"));
        }
        let tools: &[&str] = &[
            "Read",
            "Edit",
            "Write",
            "Grep",
            "Glob",
            "Skill",
            "EndConversation",
        ];
        self.skills
            .base
            .base
            .base
            .verify_system_init_for(message, tools)
    }

    pub(crate) fn host_commands(&self) -> Option<&ReviewedCommandCeilingV1> {
        self.commands.is_host().then_some(&self.commands)
    }

    pub(crate) fn approval_allowed(&self, tool: &str, input: &Value) -> bool {
        if tool == "Bash" {
            false
        } else {
            self.skills.approval_allowed(tool, input)
        }
    }
}
