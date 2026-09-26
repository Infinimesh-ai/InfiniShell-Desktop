//! 固定 Grok 原生技能工具的父集合、注册来源与逐次审批；不使用斜杠预展开技能正文。

use std::collections::BTreeSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::super::local_skills::SelectedLocalSkill;
use super::super::{InputContent, RuntimeError};

#[path = "grok_profile_skills_sources.rs"]
mod sources;
use sources::SkillSource;

pub(super) const NATIVE_TOOL: &str = "OpenCode:skill";

fn reject(reason: &str) -> RuntimeError {
    super::super::permissions::rejected(
        None,
        &json!({"creationPolicy":"GrokRestrictedSkillsV1"}),
        reason,
        false,
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct GrokSkillCeilingV1 {
    version: u32,
    skills: Vec<SkillSource>,
}

impl GrokSkillCeilingV1 {
    pub(super) fn capture(selected: &[SelectedLocalSkill]) -> Result<Self, RuntimeError> {
        if selected.len() > 32 {
            return Err(reject("grok_skills_selection_limit"));
        }
        let mut skills = selected
            .iter()
            .map(SkillSource::capture)
            .collect::<Result<Vec<_>, _>>()?;
        skills.sort_by(|left, right| left.selected.name.cmp(&right.selected.name));
        let result = Self { version: 1, skills };
        result.validate()?;
        Ok(result)
    }

    pub(super) fn validate(&self) -> Result<(), RuntimeError> {
        if self.version != 1
            || self.skills.len() > 32
            || self
                .skills
                .windows(2)
                .any(|pair| pair[0].selected.name >= pair[1].selected.name)
        {
            return Err(reject("grok_skills_manifest_invalid"));
        }
        for skill in &self.skills {
            skill.validate_identity()?;
        }
        Ok(())
    }

    pub(super) fn selected(&self) -> Vec<SelectedLocalSkill> {
        self.skills
            .iter()
            .map(|skill| skill.selected.clone())
            .collect()
    }

    pub(super) fn matches_selection(&self, selected: &[SelectedLocalSkill]) -> bool {
        let mut selected = selected.to_vec();
        selected.sort_by(|left, right| left.name.cmp(&right.name));
        self.selected() == selected
    }

    pub(super) fn contains(&self, name: &str, path: &Path) -> bool {
        self.skills
            .iter()
            .any(|skill| skill.selected.name == name && skill.selected.path == path)
    }

    pub(super) fn derive_child(
        &self,
        selected: &[SelectedLocalSkill],
    ) -> Result<Self, RuntimeError> {
        self.validate()?;
        let child = Self::capture(selected)?;
        if !self.allows_child(&child) {
            return Err(reject("grok_skills_parent_ceiling"));
        }
        Ok(child)
    }

    pub(super) fn allows_child(&self, child: &Self) -> bool {
        self.version == child.version
            && child.skills.iter().all(|skill| self.skills.contains(skill))
    }

    pub(super) fn prepare_home(&self, home: &Path) -> Result<(), RuntimeError> {
        self.validate()?;
        for skill in &self.skills {
            skill.verify_source()?;
        }
        let target = home.join("grok/skills");
        if target.try_exists()? {
            return self.verify_home(home);
        }
        // 同一应用数据域内原子注册，不覆盖历史会话的技能副本。
        let staging = tempfile::Builder::new()
            .prefix(".skills-")
            .tempdir_in(home.join("grok"))?;
        for skill in &self.skills {
            skill.copy_to(&staging.path().join(&skill.selected.name))?;
        }
        std::fs::rename(staging.path(), &target)?;
        self.verify_home(home)
    }

    pub(super) fn verify_home(&self, home: &Path) -> Result<(), RuntimeError> {
        self.validate()?;
        for skill in &self.skills {
            skill.verify_source()?;
        }
        let directory = home.join("grok/skills");
        let metadata = std::fs::symlink_metadata(&directory)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || directory.canonicalize()? != directory
        {
            return Err(reject("grok_skills_copy_changed"));
        }
        let actual = std::fs::read_dir(&directory)?
            .map(|entry| {
                entry
                    .ok()
                    .and_then(|entry| entry.file_name().into_string().ok())
            })
            .collect::<Option<BTreeSet<_>>>()
            .ok_or_else(|| reject("grok_skills_copy_changed"))?;
        let expected: BTreeSet<_> = self
            .skills
            .iter()
            .map(|skill| skill.selected.name.clone())
            .collect();
        if actual != expected {
            return Err(reject("grok_skills_copy_changed"));
        }
        for skill in &self.skills {
            skill.verify_copy(&directory.join(&skill.selected.name))?;
        }
        Ok(())
    }

    pub(super) fn verify_catalog(&self, home: &Path, commands: &Value) -> Result<(), RuntimeError> {
        self.verify_home(home)?;
        let commands = commands
            .as_array()
            .filter(|entries| entries.len() <= 4096)
            .ok_or_else(|| reject("grok_skills_catalog_shape"))?;
        for skill in &self.skills {
            let qualified = format!("user:{}", skill.selected.name);
            let path = home
                .join("grok/skills")
                .join(&skill.selected.name)
                .join("SKILL.md");
            let mut matches = commands
                .iter()
                .filter(|command| command["_meta"]["qualifiedName"] == qualified);
            let command = matches
                .next()
                .ok_or_else(|| reject("grok_skills_catalog_missing"))?;
            if matches.next().is_some()
                || command["_meta"]["scope"] != "user"
                || command["_meta"]["bareName"] != skill.selected.name
                || command.get("input").is_none_or(|input| !input.is_null())
                || command["_meta"]["path"].as_str().map(Path::new) != Some(path.as_path())
                || command["_meta"]["path"]
                    .as_str()
                    .and_then(|path| std::fs::canonicalize(path).ok())
                    != Some(path)
            {
                return Err(reject("grok_skills_catalog_source_changed"));
            }
        }
        Ok(())
    }

    pub(super) fn permits_request(&self, call: &Value) -> bool {
        let input = &call["rawInput"];
        let Some(fields) = input.as_object() else {
            return false;
        };
        // OpenCode SkillInput 只有 name；ACP 另带 ToolInput::Dynamic 的固定标记。
        // 不兼容的原生结构必须校准新合同，不能借工具标题猜测调用目标。
        call["kind"] == "other"
            && call["_meta"]["x.ai/tool"]["name"] == "skill"
            && fields.len() == 2
            && input["variant"] == "Dynamic"
            && input["name"].as_str().is_some_and(|name| {
                self.skills
                    .iter()
                    .any(|skill| name == format!("user:{}", skill.selected.name))
            })
            && self.validate().is_ok()
    }

    pub(super) fn encode_prompt(
        &self,
        home: &Path,
        commands: &Value,
        input: Vec<InputContent>,
    ) -> Result<Vec<Value>, RuntimeError> {
        self.verify_catalog(home, commands)?;
        let mut names = BTreeSet::new();
        let mut text = Vec::new();
        for part in input {
            match part {
                InputContent::Text(value) => {
                    // ACP 原生会预展开已知 /技能，包括句中引用；当前策略要求使用卡片产生原生工具调用。
                    // 绝对多段路径不属于合法技能名，可以保持原文；单段 slash token 保守拒绝。
                    if has_slash_skill_token(&value, commands) {
                        return Err(reject("grok_skills_slash_requires_tool"));
                    }
                    text.push(value);
                }
                InputContent::Skill { name, path } => {
                    if !self.contains(&name, &path) || !names.insert(name) {
                        return Err(reject("grok_skills_parent_ceiling"));
                    }
                }
                InputContent::LocalImage(_) => {
                    return Err(reject("grok_skills_image_scope_unverified"));
                }
            }
        }
        if !names.is_empty() {
            let names: Vec<_> = names
                .into_iter()
                .map(|name| format!("user:{name}"))
                .collect();
            text.push(format!(
                "Invoke the native skill tool separately for each exact name in this JSON list: {}. Wait for each permission decision. Read the native tool result before following the skill. Do not expand slash commands or substitute a read_file call for the requested skill invocation.",
                serde_json::to_string(&names).expect("技能名称列表可以序列化")
            ));
        }
        Ok(vec![json!({"type":"text","text":text.join("\n\n")})])
    }

    /// 新策略的写工具只能改项目普通路径，不能改注册副本、源技能或原生配置目录。
    pub(super) fn permits_write(&self, cwd: &Path, input: &Value) -> bool {
        let Some(path) = input["file_path"].as_str().map(Path::new) else {
            return false;
        };
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            cwd.join(path)
        };
        let Ok(relative) = path.strip_prefix(cwd) else {
            return false;
        };
        if relative.components().any(|component| match component {
            Component::Normal(name) => name.to_str().is_none_or(|name| {
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    ".grok" | ".claude" | ".agents" | ".codex" | ".git"
                ) || name.contains([':', '\\'])
                    || name.ends_with(['.', ' '])
            }),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => true,
        }) {
            return false;
        }
        let resolved = if path.exists() {
            path.canonicalize().ok()
        } else {
            path.parent()
                .and_then(|parent| parent.canonicalize().ok())
                .zip(path.file_name())
                .map(|(parent, name)| parent.join(name))
        };
        resolved.is_some_and(|resolved| {
            resolved.starts_with(cwd)
                && resolved == path
                && !self.skills.iter().any(|skill| skill.contains(&resolved))
        })
    }
}

fn has_slash_skill_token(text: &str, commands: &Value) -> bool {
    let text = text.trim();
    text.match_indices('/').any(|(index, _)| {
        if index != 0 && !text.as_bytes()[index - 1].is_ascii_whitespace() {
            return false;
        }
        let token = text[index + 1..]
            .split_whitespace()
            .next()
            .unwrap_or_default();
        !token.is_empty()
            && (!token.contains('/')
                || commands.as_array().is_some_and(|commands| {
                    commands.iter().any(|command| {
                        command["name"] == token
                            || command["_meta"]["qualifiedName"] == token
                            || command["_meta"]["bareName"] == token
                    })
                }))
    })
}
