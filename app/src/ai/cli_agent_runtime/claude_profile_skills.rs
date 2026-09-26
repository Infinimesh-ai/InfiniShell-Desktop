//! 固定 2.1.280 的原生 Skill 策略；技能清单属于父权限上限，旧文件策略不自动扩权。

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{ClaudeRestrictedFilesV1, ClaudeRestrictedFilesV3, RuntimeError, reject};
use crate::ai::cli_agent_runtime::local_skills::{
    CLAUDE_SKILL_PLUGIN_NAME, PreparedClaudeSkillPlugin, SelectedLocalSkill,
};

#[path = "claude_profile_skill_sources.rs"]
mod sources;
use sources::SkillSource;
#[path = "claude_profile_commands_skills.rs"]
mod commands_skills;
pub use commands_skills::ClaudeReviewedCommandsSkillsV1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClaudeRestrictedSkillsV1 {
    version: u32,
    cli_version: String,
    base: ClaudeRestrictedFilesV3,
    skills: Vec<SkillSource>,
}

impl ClaudeRestrictedSkillsV1 {
    pub(crate) fn compile(
        base: ClaudeRestrictedFilesV1,
        selected: &[SelectedLocalSkill],
    ) -> Result<Self, RuntimeError> {
        let mut names = BTreeSet::new();
        if selected.len() > 32 || selected.iter().any(|skill| !names.insert(&skill.name)) {
            return Err(reject("claude_skills_selection_invalid"));
        }
        let mut skills = selected
            .iter()
            .map(SkillSource::capture)
            .collect::<Result<Vec<_>, _>>()?;
        skills.sort_by(|left, right| left.selected.name.cmp(&right.selected.name));
        Ok(Self {
            version: 1,
            cli_version: "2.1.280".into(),
            base: ClaudeRestrictedFilesV3::compile(base)?,
            skills,
        })
    }

    pub(crate) fn validate(&self, cwd: &Path) -> Result<(), RuntimeError> {
        self.base.validate(cwd)?;
        if self.version != 1
            || self.cli_version != "2.1.280"
            || self.skills.len() > 32
            || self
                .skills
                .windows(2)
                .any(|pair| pair[0].selected.name >= pair[1].selected.name)
        {
            return Err(reject("claude_skills_profile_invalid"));
        }
        for skill in &self.skills {
            skill.verify_source()?;
        }
        Ok(())
    }

    pub(crate) fn selected(&self) -> Vec<SelectedLocalSkill> {
        self.skills
            .iter()
            .map(|skill| skill.selected.clone())
            .collect()
    }

    pub(crate) fn derive_child(
        &self,
        selected: &[SelectedLocalSkill],
    ) -> Result<Self, RuntimeError> {
        let mut names = BTreeSet::new();
        let mut skills = Vec::new();
        for selected in selected {
            if !names.insert(&selected.name) {
                return Err(reject("claude_skills_selection_invalid"));
            }
            let source = self
                .skills
                .iter()
                .find(|skill| skill.selected == *selected)
                .ok_or_else(|| reject("claude_skills_parent_ceiling"))?;
            source.verify_source()?;
            skills.push(source.clone());
        }
        skills.sort_by(|left, right| left.selected.name.cmp(&right.selected.name));
        let mut child = self.clone();
        child.skills = skills;
        Ok(child)
    }

    pub(crate) fn allows_child(&self, child: &Self) -> bool {
        self.version == child.version
            && self.cli_version == child.cli_version
            && self.base == child.base
            && child.skills.iter().all(|skill| self.skills.contains(skill))
    }

    fn blocked_tools(&self) -> Vec<&'static str> {
        self.base
            .base
            .blocked_tools()
            .into_iter()
            .filter(|tool| *tool != "Skill")
            .collect()
    }

    fn settings(&self) -> Value {
        let mut settings = self.base.settings();
        settings["permissions"]["ask"]
            .as_array_mut()
            .expect("固定策略包含 ask")
            .push(json!("Skill"));
        settings["disableAllHooks"] = json!(true);
        settings["disableSkillShellExecution"] = json!(true);
        settings
    }

    pub(crate) fn arguments(&self) -> Vec<String> {
        let mut arguments = self.base.base.base.arguments_for(
            "Read,Edit,Write,Grep,Glob,Skill",
            &self.blocked_tools(),
            self.settings(),
        );
        arguments.retain(|argument| argument != "--disable-slash-commands");
        arguments.extend([
            "--append-system-prompt".into(),
            "This managed task permits project file tools and the registered infinishell-local-skills commands. Invoke each skill through the Skill tool and wait for individual approval. Use Glob to find project paths, then Grep with an explicit regular file path. Shell commands, hooks, native subagents, skill permission grants and unregistered skills are unavailable.".into(),
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
        self.validate(&self.base.base.base.working_directory)?;
        self.base.base.base.verify_live_for(
            settings,
            rules,
            hooks,
            mcp,
            &self.settings(),
            &self.blocked_tools(),
        )
    }

    pub(crate) fn verify_system_init(&self, message: &Value) -> Result<(), RuntimeError> {
        if message["tools"]
            .as_array()
            .is_none_or(|tools| !tools.iter().any(|tool| tool == "Skill"))
        {
            return Err(reject("claude_skills_native_tool_missing"));
        }
        self.base.base.base.verify_system_init_for(
            message,
            &[
                "Read",
                "Edit",
                "Write",
                "Grep",
                "Glob",
                "Skill",
                "EndConversation",
            ],
        )
    }

    pub(crate) fn verify_plugin(
        &self,
        plugin: &PreparedClaudeSkillPlugin,
    ) -> Result<(), RuntimeError> {
        self.validate(&self.base.base.base.working_directory)?;
        let mut selected = plugin.selected().to_vec();
        selected.sort_by(|left, right| left.name.cmp(&right.name));
        if selected != self.selected() {
            return Err(reject("claude_skills_plugin_selection_changed"));
        }
        let directory = plugin.plugin_directory();
        for (path, expected) in [
            (
                directory.to_owned(),
                vec![".claude-plugin".to_owned(), "skills".to_owned()],
            ),
            (
                directory.join(".claude-plugin"),
                vec!["plugin.json".to_owned()],
            ),
            (
                directory.join("skills"),
                self.skills
                    .iter()
                    .map(|skill| skill.selected.name.clone())
                    .collect(),
            ),
        ] {
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|_| reject("claude_skills_plugin_unavailable"))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(reject("claude_skills_plugin_changed"));
            }
            let actual = std::fs::read_dir(path)
                .map_err(|_| reject("claude_skills_plugin_unavailable"))?
                .map(|entry| {
                    entry
                        .ok()
                        .and_then(|entry| entry.file_name().into_string().ok())
                })
                .collect::<Option<BTreeSet<_>>>()
                .ok_or_else(|| reject("claude_skills_plugin_changed"))?;
            if actual != expected.into_iter().collect() {
                return Err(reject("claude_skills_plugin_changed"));
            }
        }
        let descriptor = std::fs::read(directory.join(".claude-plugin/plugin.json"))
            .map_err(|_| reject("claude_skills_plugin_unavailable"))?;
        if descriptor
            != serde_json::to_vec(&json!({"name":CLAUDE_SKILL_PLUGIN_NAME,"version":"0.1.0"}))
                .expect("固定插件描述可以序列化")
        {
            return Err(reject("claude_skills_plugin_changed"));
        }
        for skill in &self.skills {
            skill.verify_copy(&directory.join("skills").join(&skill.selected.name))?;
        }
        Ok(())
    }

    pub(crate) fn verify_registration(
        &self,
        plugin: &PreparedClaudeSkillPlugin,
        response: &Value,
    ) -> Result<(), RuntimeError> {
        self.verify_plugin(plugin)?;
        let commands = response["commands"]
            .as_array()
            .ok_or_else(|| reject("claude_skills_registration_shape"))?;
        for skill in &self.skills {
            let name = format!("{CLAUDE_SKILL_PLUGIN_NAME}:{}", skill.selected.name);
            if commands
                .iter()
                .filter(|command| command["name"] == name)
                .count()
                != 1
            {
                return Err(reject("claude_skills_registration_missing"));
            }
        }
        let prefix = format!("{CLAUDE_SKILL_PLUGIN_NAME}:");
        if commands.iter().any(|command| {
            command["name"].as_str().is_some_and(|name| {
                name.starts_with(&prefix)
                    && !self
                        .skills
                        .iter()
                        .any(|skill| name == format!("{prefix}{}", skill.selected.name))
            })
        }) {
            return Err(reject("claude_skills_registration_changed"));
        }
        let plugins = response["plugins"]
            .as_array()
            .ok_or_else(|| reject("claude_skills_registration_shape"))?;
        if response["error_count"] != 0
            || plugins
                .iter()
                .filter(|entry| entry["name"] == CLAUDE_SKILL_PLUGIN_NAME)
                .count()
                != 1
            || plugins.iter().any(|entry| {
                if entry["name"] == CLAUDE_SKILL_PLUGIN_NAME {
                    entry["path"].as_str().map(Path::new) != Some(plugin.plugin_directory())
                        || entry["version"] != "0.1.0"
                        || entry["source"] != format!("{CLAUDE_SKILL_PLUGIN_NAME}@inline")
                } else {
                    entry["path"] != "builtin"
                        || entry["source"]
                            .as_str()
                            .is_none_or(|source| !source.ends_with("@builtin"))
                }
            })
        {
            return Err(reject("claude_skills_registration_source_changed"));
        }
        Ok(())
    }

    pub(crate) fn approval_allowed(&self, tool: &str, input: &Value) -> bool {
        if self
            .validate(&self.base.base.base.working_directory)
            .is_err()
        {
            return false;
        }
        if tool == "Skill" {
            return input.as_object().is_some_and(|fields| {
                fields
                    .keys()
                    .all(|key| matches!(key.as_str(), "skill" | "args"))
                    && input.get("args").is_none_or(|args| {
                        args.as_str().is_some_and(|value| {
                            value.len() <= 128 * 1024 && !value.contains(['\0', '@', '`', '!'])
                        })
                    })
                    && self.skills.iter().any(|skill| {
                        input["skill"]
                            == format!("{CLAUDE_SKILL_PLUGIN_NAME}:{}", skill.selected.name)
                    })
            });
        }
        // 已固定技能不能在同一任务内被 Edit/Write 改写，再由原生热加载突破来源摘要。
        if matches!(tool, "Edit" | "Write")
            && input["file_path"].as_str().is_some_and(|path| {
                let path = Path::new(path);
                self.skills.iter().any(|skill| {
                    skill.contains(path)
                        || std::fs::canonicalize(path).is_ok_and(|path| skill.contains(&path))
                })
            })
        {
            return false;
        }
        self.base.approval_allowed(tool, input)
    }
}
