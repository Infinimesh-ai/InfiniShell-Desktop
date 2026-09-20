//! 原生技能目录只用于当前会话的路径绑定，不进入展示快照或自动授予项目信任。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use ai::skills::{SkillProvider, SkillScope, parse_skill_content_at_location};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::super::InputContent;
use super::MAX_NATIVE_IDENTITIES;
use crate::terminal::CLIAgent;
use crate::terminal::input::skills::is_user_invocable;

pub(super) struct SelectedSkill {
    name: String,
    path: PathBuf,
    canonical_path: PathBuf,
    content_sha256: [u8; 32],
}

impl SelectedSkill {
    pub(super) fn new(name: String, path: PathBuf) -> Result<Self, String> {
        let (canonical_path, content_sha256) = validate_reference(&name, &path)?;
        Ok(Self {
            name,
            path,
            canonical_path,
            content_sha256,
        })
    }
}

/// 临发前重新检查内容和路径；不复制技能目录、不注入技能正文。
fn validate_reference(name: &str, path: &Path) -> Result<(PathBuf, [u8; 32]), String> {
    let validate = || -> anyhow::Result<(PathBuf, [u8; 32])> {
        if !path.is_absolute() || path.file_name().is_none_or(|name| name != "SKILL.md") {
            anyhow::bail!("技能路径无效");
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024
        {
            anyhow::bail!("技能文件类型或大小无效");
        }
        let mut content = String::new();
        fs::File::open(path)?
            .take(1024 * 1024 + 1)
            .read_to_string(&mut content)?;
        if content.len() > 1024 * 1024 {
            anyhow::bail!("技能文件超过大小限制");
        }
        // 此解析只验证原始文件声明；来源是否可调用仍由原生目录的唯一路径证明。
        let parsed = parse_skill_content_at_location(
            LocalOrRemotePath::Local(path.to_path_buf()),
            &content,
            SkillProvider::Grok,
            SkillScope::Project,
        )?;
        if parsed.name != name || !is_user_invocable(&parsed.user_invocable(), CLIAgent::Grok) {
            anyhow::bail!("技能名称或人工可见性已变更");
        }
        Ok((
            path.canonicalize()?,
            Sha256::digest(content.as_bytes()).into(),
        ))
    };
    validate().map_err(|_| unavailable())
}

pub(super) fn unavailable() -> String {
    crate::t!("cli-agent-grok-skill-unavailable")
}

struct NativeCommand {
    name: String,
    bare_name: Option<String>,
    path: Option<PathBuf>,
    canonical_path: Option<PathBuf>,
}

pub(super) struct SkillCatalog {
    commands: Vec<NativeCommand>,
}

impl SkillCatalog {
    pub(super) fn from_native(commands: &Value) -> Result<Self, String> {
        let entries = commands
            .as_array()
            .filter(|entries| entries.len() <= MAX_NATIVE_IDENTITIES)
            .ok_or_else(unavailable)?;
        let commands = entries
            .iter()
            .map(|entry| {
                let path = entry["_meta"]["path"]
                    .as_str()
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute());
                NativeCommand {
                    name: entry["name"].as_str().unwrap_or_default().to_owned(),
                    bare_name: entry["_meta"]["bareName"].as_str().map(str::to_owned),
                    canonical_path: path.as_ref().and_then(|path| path.canonicalize().ok()),
                    path,
                }
            })
            .collect();
        Ok(Self { commands })
    }

    fn command_for(&self, selected: &SelectedSkill) -> Result<&str, String> {
        let (canonical_path, content_sha256) = validate_reference(&selected.name, &selected.path)?;
        if canonical_path != selected.canonical_path || content_sha256 != selected.content_sha256 {
            return Err(unavailable());
        }
        let mut matches = self
            .commands
            .iter()
            .filter(|command| command.canonical_path.as_ref() == Some(&selected.canonical_path));
        let command = matches.next().ok_or_else(unavailable)?;
        if matches.next().is_some()
            || command.bare_name.as_deref() != Some(selected.name.as_str())
            || command
                .path
                .as_ref()
                .and_then(|path| path.canonicalize().ok())
                .as_ref()
                != Some(&selected.canonical_path)
            || command.name.is_empty()
            || command.name.len() > 128
            || command.name.starts_with('/')
            || command
                .name
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
            || self
                .commands
                .iter()
                .filter(|other| other.name == command.name)
                .count()
                != 1
        {
            return Err(unavailable());
        }
        Ok(&command.name)
    }

    pub(super) fn encode(
        &self,
        input: Vec<InputContent>,
        selected: &SelectedSkill,
    ) -> Result<Vec<Value>, String> {
        let command = self.command_for(selected)?;
        let mut content = Vec::new();
        let mut prefixed = false;
        for part in input {
            match part {
                InputContent::Text(text) => {
                    let text = if prefixed {
                        text
                    } else {
                        prefixed = true;
                        format!("/{command} {text}")
                    };
                    content.push(json!({"type":"text", "text":text}));
                }
                InputContent::Skill { .. } => {}
                InputContent::LocalImage(_) => return Err(unavailable()),
            }
        }
        if !prefixed {
            content.push(json!({"type":"text", "text":format!("/{command}")}));
        }
        Ok(content)
    }
}

#[cfg(test)]
#[path = "grok_skills_tests.rs"]
mod tests;
