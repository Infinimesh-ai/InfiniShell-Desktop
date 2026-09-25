//! 子任务技能只读取已启用的本地目录，按原生 CLI 能力交付。

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use ai::skills::{ParsedSkill, SkillReference, parse_skill, parse_skill_content_at_location};
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use warp_cli::agent::Harness;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, SingletonEntity};

use super::InputContent;
use crate::ai::skills::SkillManager;
use crate::terminal::CLIAgent;
use crate::terminal::input::skills::is_user_invocable;

pub(crate) const CLAUDE_SKILL_PLUGIN_NAME: &str = "infinishell-local-skills";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectedLocalSkill {
    pub name: String,
    pub path: PathBuf,
}

pub(crate) struct PreparedClaudeSkillPlugin {
    directory: TempDir,
    selected: Vec<SelectedLocalSkill>,
    budget: SkillCopyBudget,
}

impl PreparedClaudeSkillPlugin {
    pub(crate) fn plugin_directory(&self) -> &Path {
        self.directory.path()
    }

    pub(crate) fn selected(&self) -> &[SelectedLocalSkill] {
        &self.selected
    }

    /// 只暂存尚未注册的目录；原生确认前不暴露为可调用技能。
    pub(crate) fn stage_additions(
        &self,
        selected: &[SelectedLocalSkill],
    ) -> Result<Option<PreparedClaudeSkillUpdate>, String> {
        let mut additions = Vec::new();
        for skill in selected {
            if let Some(previous) = self.selected.iter().find(|item| item.name == skill.name) {
                if previous != skill {
                    return Err(crate::t!("cli-agent-claude-skill-path-conflict"));
                }
            } else if !additions.contains(skill) {
                additions.push(skill.clone());
            }
        }
        if additions.is_empty() {
            return Ok(None);
        }
        if self.selected.len() + additions.len() > 32 {
            return Err(crate::t!("cli-agent-claude-skill-limit"));
        }
        let staged = prepare_claude_skill_directory(&additions, self.budget.clone())?;
        let mut update = PreparedClaudeSkillUpdate {
            selected: additions,
            paths: Vec::new(),
            budget: staged.budget.clone(),
        };
        for skill in &update.selected {
            let destination = self.directory.path().join("skills").join(&skill.name);
            if destination.exists() {
                return Err(crate::t!("cli-agent-claude-skill-path-conflict"));
            }
            std::fs::rename(
                staged.directory.path().join("skills").join(&skill.name),
                &destination,
            )
            .map_err(|error| {
                crate::t!(
                    "cli-agent-task-skill-plugin-failed",
                    error = error.to_string()
                )
            })?;
            update.paths.push(destination);
        }
        Ok(Some(update))
    }

    pub(crate) fn command_names(&self) -> Vec<String> {
        self.selected
            .iter()
            .map(|skill| format!("{CLAUDE_SKILL_PLUGIN_NAME}:{}", skill.name))
            .collect()
    }

    /// 返回不带斜线的注册命令名，必须同时匹配选中的名称与原始路径。
    pub(crate) fn command_for(&self, name: &str, path: &Path) -> Option<String> {
        self.selected
            .iter()
            .find(|skill| skill.name == name && skill.path == path)
            .map(|skill| format!("{CLAUDE_SKILL_PLUGIN_NAME}:{}", skill.name))
    }
}

/// 每次连接都从源引用生成独立插件；返回值持有临时目录，进程结束后自动清理。
pub(crate) fn prepare_claude_skill_plugin(
    selected: &[SelectedLocalSkill],
) -> Result<Option<PreparedClaudeSkillPlugin>, String> {
    if selected.is_empty() {
        return Ok(None);
    }
    prepare_empty_or_selected_claude_skill_plugin(selected).map(Some)
}

pub(crate) fn prepare_empty_or_selected_claude_skill_plugin(
    selected: &[SelectedLocalSkill],
) -> Result<PreparedClaudeSkillPlugin, String> {
    prepare_claude_skill_directory(
        selected,
        SkillCopyBudget {
            bytes: 32 * 1024 * 1024,
            entries: 1024,
        },
    )
}

/// 注册失败或连接丢弃时，仅删除本次新增且由本对象拥有的副本。
pub(crate) struct PreparedClaudeSkillUpdate {
    selected: Vec<SelectedLocalSkill>,
    paths: Vec<PathBuf>,
    budget: SkillCopyBudget,
}

impl PreparedClaudeSkillUpdate {
    pub(crate) fn command_names(&self) -> Vec<String> {
        self.selected
            .iter()
            .map(|skill| format!("{CLAUDE_SKILL_PLUGIN_NAME}:{}", skill.name))
            .collect()
    }

    pub(crate) fn commit(mut self, plugin: &mut PreparedClaudeSkillPlugin) {
        plugin.selected.append(&mut self.selected);
        plugin.budget = self.budget.clone();
        self.paths.clear();
    }
}

impl Drop for PreparedClaudeSkillUpdate {
    fn drop(&mut self) {
        for path in &self.paths {
            // 目标全部由 stage_additions 创建；不触碰原始技能或既有注册目录。
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn prepare_claude_skill_directory(
    selected: &[SelectedLocalSkill],
    mut budget: SkillCopyBudget,
) -> Result<PreparedClaudeSkillPlugin, String> {
    let prepare = || -> anyhow::Result<PreparedClaudeSkillPlugin> {
        if selected.len() > 32 {
            bail!("技能数量超过限制");
        }
        let directory = tempfile::Builder::new()
            .prefix("infinishell-claude-skills-")
            .tempdir()?;
        std::fs::create_dir(directory.path().join(".claude-plugin"))?;
        std::fs::create_dir(directory.path().join("skills"))?;
        std::fs::write(
            directory.path().join(".claude-plugin/plugin.json"),
            serde_json::to_vec(
                &serde_json::json!({"name":CLAUDE_SKILL_PLUGIN_NAME,"version":"0.1.0"}),
            )?,
        )?;
        let mut names = HashSet::new();
        for skill in selected {
            if !skill.path.is_absolute()
                || skill.path.file_name().is_none_or(|name| name != "SKILL.md")
                || skill.name.is_empty()
                || skill.name.len() > 64
                || skill.name.bytes().any(|byte| {
                    !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && byte != b'-'
                })
                || !names.insert(&skill.name)
            {
                bail!("技能名称、路径无效或重复");
            }
            let source = skill.path.parent().context("技能缺少资源目录")?;
            let destination = directory.path().join("skills").join(&skill.name);
            copy_skill_directory(source, &destination, 0, &mut budget)?;
            let copied_skill = parse_skill(&destination.join("SKILL.md"))?;
            if copied_skill.name != skill.name {
                bail!("技能名称已变更，请重新选择技能");
            }
        }
        Ok(PreparedClaudeSkillPlugin {
            directory,
            selected: selected.to_vec(),
            budget,
        })
    };
    prepare().map_err(|error| {
        crate::t!(
            "cli-agent-task-skill-plugin-failed",
            error = error.to_string()
        )
    })
}

#[derive(Clone)]
struct SkillCopyBudget {
    bytes: u64,
    entries: usize,
}

fn copy_skill_directory(
    source: &Path,
    destination: &Path,
    depth: usize,
    budget: &mut SkillCopyBudget,
) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || depth > 16 {
        bail!("技能资源目录不是普通目录或嵌套超过限制");
    }
    std::fs::create_dir(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        budget.entries = budget
            .entries
            .checked_sub(1)
            .context("技能资源数量超过限制")?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("技能资源包含未支持的符号链接");
        }
        if metadata.is_dir() {
            copy_skill_directory(&path, &target, depth + 1, budget)?;
        } else if metadata.is_file() {
            let mut input = std::fs::File::open(&path)?.take(budget.bytes + 1);
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            let copied = std::io::copy(&mut input, &mut output)?;
            budget.bytes = budget
                .bytes
                .checked_sub(copied)
                .context("技能资源超过大小限制")?;
            // Unix 脚本保留执行位；Windows 临时副本不复制只读属性，以便退出时清理。
            #[cfg(unix)]
            std::fs::set_permissions(&target, metadata.permissions())?;
        } else {
            bail!("技能资源包含非普通文件");
        }
    }
    Ok(())
}

pub(crate) fn collect_local_child_skills(
    references: &[SkillReference],
    ctx: &AppContext,
) -> Result<Vec<ParsedSkill>, String> {
    let mut seen = HashSet::new();
    references
        .iter()
        .map(|reference| {
            if !seen.insert(reference) {
                return Err(crate::t!(
                    "cli-agent-task-skill-unavailable",
                    skill = reference.display_label()
                ));
            }
            let SkillReference::Path(LocalOrRemotePath::Local(path)) = reference else {
                return Err(crate::t!(
                    "cli-agent-task-skill-local-required",
                    skill = reference.display_label()
                ));
            };
            if !path.is_absolute() || references.len() > 32 {
                return Err(crate::t!(
                    "cli-agent-task-skill-unavailable",
                    skill = reference.display_label()
                ));
            }
            SkillManager::as_ref(ctx)
                .active_skill_by_reference(reference, ctx)
                .filter(|skill| {
                    !skill.is_bundled() && skill.path == LocalOrRemotePath::Local(path.clone())
                })
                .cloned()
                .ok_or_else(|| {
                    crate::t!(
                        "cli-agent-task-skill-unavailable",
                        skill = reference.display_label()
                    )
                })
        })
        .collect()
}

/// 在启动准备的后台任务中重读文件，避免目录缓存把删除或损坏的技能当作可用。
pub(crate) fn prepare_local_cli_skill_inputs(
    skills: Vec<ParsedSkill>,
    harness: Harness,
    managed: bool,
) -> Result<Vec<InputContent>, String> {
    if !skills.is_empty()
        && (!managed || !matches!(harness, Harness::Codex | Harness::Claude | Harness::Grok))
    {
        return Err(crate::t!("cli-agent-task-skills-require-managed"));
    }
    if harness == Harness::Grok && skills.len() > 1 {
        return Err(crate::t!("cli-agent-task-skill-one-per-turn"));
    }
    if harness == Harness::Claude {
        let mut names = HashSet::new();
        if skills.iter().any(|skill| !names.insert(&skill.name)) {
            return Err(crate::t!("cli-agent-claude-skill-duplicate"));
        }
    }
    skills
        .into_iter()
        .map(|skill| {
            let failure = || {
                crate::t!(
                    "cli-agent-task-skill-unavailable",
                    skill = skill.path.display_path()
                )
            };
            let LocalOrRemotePath::Local(path) = &skill.path else {
                return Err(failure());
            };
            if !path.is_absolute() || skill.is_bundled() {
                return Err(failure());
            }
            if harness == Harness::Grok {
                let metadata = std::fs::symlink_metadata(path).map_err(|_| failure())?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(failure());
                }
            }
            let mut content = String::new();
            std::fs::File::open(path)
                .map_err(|_| failure())?
                .take(1024 * 1024 + 1)
                .read_to_string(&mut content)
                .map_err(|_| failure())?;
            if content.len() > 1024 * 1024 {
                return Err(failure());
            }
            let parsed = parse_skill_content_at_location(
                skill.path.clone(),
                &content,
                skill.provider,
                skill.scope,
            )
            .map_err(|_| failure())?;
            if harness == Harness::Grok
                && (parsed.name != skill.name
                    || !is_user_invocable(&parsed.user_invocable(), CLIAgent::Grok))
            {
                return Err(failure());
            }
            Ok(InputContent::Skill {
                name: parsed.name,
                path: path.clone(),
            })
        })
        .collect()
}
#[cfg(test)]
#[path = "local_skills_tests.rs"]
mod tests;
