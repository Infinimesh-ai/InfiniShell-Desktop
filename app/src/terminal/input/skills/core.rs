use ai::skills::{SkillProvider, SkillReference, SkillScope, SkillUserInvocable};
use fuzzy_match::{FuzzyMatchResult, match_indices_case_insensitive};
use ordered_float::OrderedFloat;
use warp_core::ui::icons::Icon;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, EntityId, SingletonEntity as _};

use crate::ai::skills::{SkillDescriptor, SkillManager};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::{CLIAgentInputState, CLIAgentSessionsModel};
pub fn local_skills_remote_execution_error_message() -> String {
    crate::t!("terminal-local-skills-remote-error")
}

/// 所有人工技能菜单共用来源筛选和 CLI 原生调用规则。
pub(crate) fn selectable_cli_skill(
    mut skill: SkillDescriptor,
    cli_agent: Option<CLIAgent>,
    skill_manager: &SkillManager,
) -> Option<SkillDescriptor> {
    if let Some(agent) = cli_agent {
        let providers = agent.supported_skill_providers_for_scope(skill.scope);
        if !skill_manager.skill_exists_for_any_provider(&skill, providers)
            || !is_user_invocable(&skill.user_invocable, agent)
        {
            return None;
        }
        skill.provider = skill_manager.best_supported_provider(&skill, providers);
    }
    Some(skill)
}

pub(crate) fn is_user_invocable(value: &SkillUserInvocable, agent: CLIAgent) -> bool {
    if agent == CLIAgent::Grok {
        // Grok 只接受 YAML true 或精确字符串 "true"，缺字段默认显示。
        matches!(
            value,
            SkillUserInvocable::Unspecified | SkillUserInvocable::Boolean(true)
        ) || matches!(value, SkillUserInvocable::String(value) if value == "true")
    } else if agent == CLIAgent::Claude {
        // Claude 已证明的布尔 false 只隐藏人工菜单，不影响模型侧技能调用。
        !matches!(value, SkillUserInvocable::Boolean(false))
    } else {
        // Codex 等 CLI 使用各自的调用元数据，不能套用 Claude/Grok 的限制。
        true
    }
}

/// Surface-neutral skill selection result shared by GUI and TUI menus.
#[derive(Clone)]
pub struct SelectableSkill {
    pub name: String,
    pub reference: SkillReference,
    pub description: String,
    pub scope: SkillScope,
    pub provider: SkillProvider,
    pub icon_override: Option<Icon>,
    pub name_match_result: Option<FuzzyMatchResult>,
    pub score: OrderedFloat<f64>,
}

/// Returns skills available for selection in the active input surface.
///
/// This owns the shared discovery, CLI-agent provider filtering, bundled-skill
/// policy, fuzzy matching, and ordering used by both frontend adapters.
pub fn query_selectable_skills(
    working_directory: Option<&LocalOrRemotePath>,
    terminal_view_id: EntityId,
    include_bundled: bool,
    query_text: &str,
    app: &AppContext,
) -> Vec<SelectableSkill> {
    let cli_agent = CLIAgentSessionsModel::as_ref(app)
        .session(terminal_view_id)
        .filter(|session| matches!(session.input_state, CLIAgentInputState::Open { .. }))
        .map(|session| session.agent);
    let skill_manager = SkillManager::as_ref(app);
    let query_text = query_text.trim();
    let mut results = skill_manager
        .get_skills_for_working_directory(working_directory, app)
        .into_iter()
        .filter(|skill| {
            cli_agent.is_some() || include_bundled || skill.scope != SkillScope::Bundled
        })
        .filter_map(|skill| selectable_cli_skill(skill, cli_agent, skill_manager))
        .filter_map(|skill| {
            let (name_match_result, score) = if query_text.is_empty() {
                (None, OrderedFloat(f64::MIN))
            } else {
                let match_result = match_indices_case_insensitive(skill.name.as_str(), query_text)?;
                if query_text.len() > 1 && match_result.score < 10 {
                    return None;
                }
                let score = OrderedFloat(match_result.score as f64);
                (Some(match_result), score)
            };

            Some(SelectableSkill {
                name: skill.name,
                reference: skill.reference,
                description: skill.description,
                scope: skill.scope,
                provider: skill.provider,
                icon_override: skill.icon_override,
                name_match_result,
                score,
            })
        })
        .collect::<Vec<_>>();

    // Inline menus render lower-ranked results first and select from the end.
    // Reverse alphabetical tie-breaking puts the alphabetically first skill at
    // the selected end of an unfiltered result list.
    results.sort_by(|left, right| {
        left.score
            .cmp(&right.score)
            .then_with(|| right.name.to_lowercase().cmp(&left.name.to_lowercase()))
    });
    results
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
