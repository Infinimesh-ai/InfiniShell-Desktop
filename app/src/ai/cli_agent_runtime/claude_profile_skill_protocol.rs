//! 固定技能策略只发送原生命名引用；用内容块避开斜线展开，使每次加载都经过 Skill 审批。

use super::*;

impl ClaudeProtocol {
    pub(super) fn verify_fixed_skill_plugin(&self) -> Result<(), RuntimeError> {
        let Some(profile) = self
            .options
            .claude_profile
            .as_ref()
            .and_then(|profile| profile.skills_profile())
        else {
            return Ok(());
        };
        if !self.fixed_skill_registration_verified {
            return Err(super::super::claude_profile::reject(
                "claude_skills_registration_unconfirmed",
            ));
        }
        let plugin = self
            .skill_plugin
            .as_ref()
            .ok_or_else(|| super::super::claude_profile::reject("claude_skills_plugin_missing"))?;
        profile.verify_plugin(plugin)
    }

    pub(super) fn encode_fixed_skill_input(
        &self,
        input: Vec<InputContent>,
    ) -> Result<InputProjection, String> {
        self.verify_fixed_skill_plugin()
            .map_err(|error| error.to_string())?;
        let mut commands = Vec::new();
        let mut parts = Vec::new();
        let has_images = input
            .iter()
            .any(|part| matches!(part, InputContent::LocalImage(_)));
        for part in input {
            match part {
                InputContent::Skill { name, path } => {
                    let command = self
                        .skill_plugin
                        .as_ref()
                        .and_then(|plugin| plugin.command_for(&name, &path))
                        .ok_or_else(|| crate::t!("cli-agent-claude-fixed-skills-unavailable"))?;
                    if commands.contains(&command) {
                        return Err(crate::t!("cli-agent-claude-skill-duplicate"));
                    }
                    commands.push(command);
                }
                InputContent::Text(_) | InputContent::LocalImage(_) => parts.push(part),
            }
        }
        if has_images && commands.len() > 1 {
            return Err(crate::t!(
                "cli-agent-claude-image-multiple-skills-unverified"
            ));
        }
        if !commands.is_empty() {
            let names = serde_json::to_string(&commands).expect("技能名称可以序列化");
            parts.insert(0, InputContent::Text(format!(
                "Invoke the native Skill tool separately for each registered skill: {names}. Wait for its individual approval before applying the skill."
            )));
        }
        match encode_input(parts, self.skill_plugin.as_ref(), &self.attachment_store)? {
            InputProjection::Text { content, .. } => {
                Ok(InputProjection::FixedSkillText { content })
            }
            fixed @ InputProjection::FixedSkillText { .. } => Ok(fixed),
            blocks @ InputProjection::Blocks { .. } => Ok(blocks),
        }
    }
}
