//! `AppContext`-backed catalog lookups, default resolution, and
//! persistence helpers for orchestration edit flows. No GUI types.

use ai::agent::action::RunAgentsRequest;
use warp_cli::agent::Harness;
use warpui::{AppContext, SingletonEntity};

use crate::LLMPreferences;
use crate::ai::auth_secret_types::auth_secret_types_for_harness;
use crate::ai::connected_self_hosted_workers::WARP_WORKER_HOST;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::ai::llms::LLMInfo;
use crate::ai::orchestration::config_state::AuthSecretSelection;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Env var override for the workspace default host (developer testing).
/// Mirrors the single-agent ambient flow.
const DEFAULT_HOST_ENV_VAR: &str = "WARP_CLOUD_MODE_DEFAULT_HOST";

pub const ORCHESTRATION_WARP_WORKER_HOST: &str = WARP_WORKER_HOST;
/// Returns Warp base-model choices for orchestration.
pub(crate) fn get_base_model_choices<'a>(
    llm_prefs: &'a LLMPreferences,
    app: &'a AppContext,
    is_local: bool,
) -> impl Iterator<Item = &'a LLMInfo> {
    llm_prefs
        .get_base_llm_choices_for_agent_mode(app)
        .filter(move |llm| is_local || llm_prefs.custom_llm_info_for_id(&llm.id).is_none())
}

/// Returns whether the given model_id is present in the harness-filtered
/// model choices. Used to detect when a harness change invalidates the
/// current model selection.
pub fn is_model_in_filtered_choices(
    model_id: &str,
    harness_type: &str,
    is_local: bool,
    ctx: &AppContext,
) -> bool {
    let harness = Harness::parse_orchestration_harness(harness_type);
    match harness {
        Some(Harness::Oz) | None => {
            let llm_prefs = LLMPreferences::as_ref(ctx);
            get_base_model_choices(llm_prefs, ctx, is_local)
                .any(|llm| llm.id.to_string() == model_id)
        }
        Some(Harness::Codex) if is_local => model_id.is_empty(),
        Some(harness) => {
            // Empty string is always valid (the "Default model" entry).
            if model_id.is_empty() {
                return true;
            }
            let availability = HarnessAvailabilityModel::as_ref(ctx);
            availability
                .models_for(harness)
                .is_some_and(|models| models.iter().any(|m| m.id == model_id))
        }
    }
}

/// Returns the default model_id for the given harness.
///
/// For Oz this is the first Warp LLM; for non-Oz harnesses it is an empty
/// string (the "Default model" entry).
pub fn first_filtered_model_id(harness_type: &str, ctx: &AppContext) -> Option<String> {
    let harness = Harness::parse_orchestration_harness(harness_type);
    match harness {
        Some(Harness::Oz) | None => {
            let llm_prefs = LLMPreferences::as_ref(ctx);
            llm_prefs
                .get_base_llm_choices_for_agent_mode(ctx)
                .next()
                .map(|llm| llm.id.to_string())
        }
        Some(_) => Some(String::new()),
    }
}

/// Resolves the workspace-configured default host slug, honoring the
/// `WARP_CLOUD_MODE_DEFAULT_HOST` env var override for developer
/// testing. Mirrors the single-agent ambient flow.
pub fn resolve_default_host_slug(ctx: &AppContext) -> Option<String> {
    if let Ok(slug) = std::env::var(DEFAULT_HOST_ENV_VAR) {
        let trimmed = slug.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    UserWorkspaces::as_ref(ctx)
        .default_host_slug()
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

/// 上游从 `CloudAgentSettings.last_selected_host` 取"最近使用的自定义 host"。
/// Zap 未引入 `ai::cloud_agent_settings`(云端 agent 设置组),故无处可读,恒返回
/// `None` —— host 菜单只保留 "warp" 与工作区默认两行。
pub fn resolve_recent_host_slug(_ctx: &AppContext) -> Option<String> {
    None
}

/// 上游把 host 选择写回 `CloudAgentSettings.last_selected_host`。该设置组随云端
/// agent 链路一并未引入,这里退化为 no-op(签名保留,GUI/TUI 调用点不变)。
pub fn persist_host_selection(_worker_host: &str, _ctx: &mut AppContext) {}

/// Normalizes a harness_type string for use as a HashMap key in
/// per-harness model memory. Empty string (the wire representation
/// of Oz) is mapped to "oz" so saves and lookups are consistent.
pub fn harness_save_key(harness_type: &str) -> &str {
    if harness_type.is_empty() {
        "oz"
    } else {
        harness_type
    }
}

/// 上游从 `CloudEnvironmentCatalog` 解析默认云端环境。Zap 未挂载
/// `ai::cloud_environments`(云端环境目录与准备链路已下线),没有可选环境,
/// 因此恒返回 `None` —— 环境选择器会落到 "Empty environment"。
pub fn resolve_default_environment_id(_ctx: &AppContext) -> Option<String> {
    None
}

/// 上游把环境选择写回 `CloudEnvironmentCatalog`。目录模块未挂载,这里退化为
/// no-op(签名保留,plan card / confirmation card 调用点不变)。
pub fn persist_environment_selection(_environment_id: &str, _ctx: &mut AppContext) {}

/// 上游从 `CloudAgentSettings.last_selected_auth_secret` 取该 harness 记住的
/// managed secret 名,再与云端已加载的 secret 列表校验。Zap 未引入该设置组,
/// 没有持久化来源,恒返回 `None`(签名保留)。
pub fn resolve_default_auth_secret_for_harness(
    _harness_type: &str,
    _ctx: &AppContext,
) -> Option<String> {
    None
}

/// Returns the full persisted selection (Named / Inherit / Unset) for
/// this harness. Prefers an explicit `Inherit` choice over a `Named`
/// fallback so the plan card's "Inherit" survives across the RunAgents
/// handoff (the `OrchestrationConfig` proto doesn't carry auth state).
pub fn resolve_auth_secret_selection_for_harness(
    harness_type: &str,
    ctx: &AppContext,
) -> AuthSecretSelection {
    let Some(harness) = Harness::parse_orchestration_harness(harness_type) else {
        return AuthSecretSelection::Unset;
    };
    if harness == Harness::Oz {
        return AuthSecretSelection::Unset;
    }
    // Zap:上游此处先读 `CloudAgentSettings.inherit_auth_secret_harnesses`
    // 判断用户是否显式选了 Inherit。该设置组未引入,只保留 Named/Unset 两态。
    match resolve_default_auth_secret_for_harness(harness_type, ctx) {
        Some(name) => AuthSecretSelection::Named(name),
        None => AuthSecretSelection::Unset,
    }
}

/// 上游把 auth-secret 选择写回 `CloudAgentSettings`
/// (`last_selected_auth_secret` / `inherit_auth_secret_harnesses`)。
/// Zap 未引入该设置组,这里退化为 no-op —— 选择只在本次会话的编辑态内有效。
pub(crate) fn persist_auth_secret_selection(
    _harness_type: &str,
    _selection: &AuthSecretSelection,
    _ctx: &mut AppContext,
) {
}

/// Whether Remote execution of `request` requires a managed auth secret
/// (non-Oz cloud harness with at least one supported secret type).
fn requires_default_auth_secret_for_execution(request: &RunAgentsRequest) -> bool {
    if !request.execution_mode.is_remote() {
        return false;
    }
    let Some(harness) = Harness::parse_orchestration_harness(&request.harness_type) else {
        return false;
    };
    harness != Harness::Oz && !auth_secret_types_for_harness(harness).is_empty()
}

/// Whether the request can execute as-is: either it doesn't need a
/// managed auth secret, already carries one, or a persisted default
/// exists for the harness.
pub(crate) fn can_execute_with_auth_secret(request: &RunAgentsRequest, ctx: &AppContext) -> bool {
    if !requires_default_auth_secret_for_execution(request) {
        return true;
    }
    if request
        .harness_auth_secret_name
        .as_deref()
        .is_some_and(|name| !name.trim().is_empty())
    {
        return true;
    }
    default_auth_secret_name_for_harness(&request.harness_type, ctx).is_some()
}

/// 上游返回 `CloudAgentSettings` 中记住的 managed secret 名。设置组未引入,
/// 恒返回 `None`(签名保留,`can_execute_with_auth_secret` 等调用点不变)。
pub(crate) fn default_auth_secret_name_for_harness(
    _harness_type: &str,
    _ctx: &AppContext,
) -> Option<String> {
    None
}

/// Fills `harness_auth_secret_name` from the persisted per-harness default
/// when the request needs one and doesn't already carry a name.
pub(crate) fn populate_default_auth_secret_for_execution(
    request: &mut RunAgentsRequest,
    ctx: &AppContext,
) {
    if !requires_default_auth_secret_for_execution(request)
        || request
            .harness_auth_secret_name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
    {
        return;
    }
    request.harness_auth_secret_name =
        default_auth_secret_name_for_harness(&request.harness_type, ctx);
}
