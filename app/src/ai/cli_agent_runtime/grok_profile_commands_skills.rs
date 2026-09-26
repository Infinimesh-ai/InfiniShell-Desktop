//! Grok 组合策略保留两组固定能力；技能不能授予额外 Shell 命令。

use std::path::Path;

use super::super::RuntimeError;
use super::super::reviewed_project_commands::{ReviewedCommandCeilingV1, reject};
use super::skills::GrokSkillCeilingV1;

pub(super) fn validate(
    commands: &ReviewedCommandCeilingV1,
    skills: &GrokSkillCeilingV1,
    cwd: &Path,
) -> Result<(), RuntimeError> {
    commands.validate()?;
    skills.validate()?;
    if !commands.matches_directory(cwd) {
        return Err(reject("grok_command_skills_directory_changed"));
    }
    Ok(())
}

pub(super) fn instruction(
    commands: &ReviewedCommandCeilingV1,
    skills: &GrokSkillCeilingV1,
) -> String {
    let names: Vec<_> = skills
        .selected()
        .into_iter()
        .map(|skill| format!("user:{}", skill.name))
        .collect();
    format!(
        "{} Invoke selected skills only through the native skill tool using these exact names: {}. Each skill invocation requires separate approval. A skill cannot expand the approved command set.",
        commands.instruction_with_skills(),
        serde_json::to_string(&names).expect("技能引用可序列化")
    )
}
