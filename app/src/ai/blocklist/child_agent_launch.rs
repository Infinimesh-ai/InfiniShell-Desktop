//! Frontend-neutral preparation and settings propagation for local Oz children.
#[cfg(not(target_family = "wasm"))]
use std::future::Future;

use warpui::{AppContext, EntityId, SingletonEntity as _};
#[cfg(not(target_family = "wasm"))]
// Zap:`crate::server::server_api` 云端网关已删除,子 agent 的 task 行改为本地生成。
use {
    crate::ai::ambient_agents::AmbientAgentTaskId,
    crate::ai::ambient_agents::task::normalize_orchestrator_agent_name,
};

use crate::AIExecutionProfilesModel;
#[cfg(not(target_family = "wasm"))]
use crate::ai::llms::LLMId;
use crate::ai::llms::LLMPreferences;

/// Server-side state prepared before a frontend creates the child's surface.
#[cfg(not(target_family = "wasm"))]
pub struct PreparedLocalOzChildLaunch {
    pub task_id: AmbientAgentTaskId,
    pub conversation_name: String,
}

/// Prepares the task identity shared by the GUI hidden-pane and TUI
/// background-session launch paths.
///
/// Zap:上游在这里调用云端 `ai_client.create_agent_task` 预建任务行。云端 API 网关
/// (`crate::server::server_api`)已删除,子 agent 完全跑在本地,因此改为用
/// `AmbientAgentTaskId::new_local()` 本地生成 task id(与 ambient agent 本地化路径一致)。
/// 返回值仍是 Future,保持调用点(`ctx.spawn(launch, ...)`)不变。
#[cfg(not(target_family = "wasm"))]
pub fn prepare_local_oz_child_launch(
    name: &str,
    _prompt: &str,
    _parent_run_id: Option<&str>,
    _ctx: &AppContext,
) -> impl Future<Output = anyhow::Result<PreparedLocalOzChildLaunch>> + 'static + use<> {
    let agent_name = normalize_orchestrator_agent_name(name);
    let conversation_name = agent_name.unwrap_or_default();
    async move {
        Ok(PreparedLocalOzChildLaunch {
            task_id: AmbientAgentTaskId::new_local(),
            conversation_name,
        })
    }
}

/// Copies the parent's execution profile and effective base model to a child
/// surface before its first request is sent.
pub fn inherit_child_agent_settings(
    parent_surface_id: EntityId,
    child_surface_id: EntityId,
    ctx: &mut AppContext,
) {
    let parent_profile_id = AIExecutionProfilesModel::as_ref(ctx)
        .active_profile(Some(parent_surface_id), ctx)
        .id()
        .clone();
    AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
        profiles.set_active_profile(child_surface_id, parent_profile_id, ctx);
    });

    let parent_base_model_id = LLMPreferences::as_ref(ctx)
        .get_active_base_model(ctx, Some(parent_surface_id))
        .id
        .clone();
    LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
        preferences.update_preferred_agent_mode_llm(&parent_base_model_id, child_surface_id, ctx);
    });
}

/// Applies a non-empty run-wide model override after parent settings have
/// been inherited.
#[cfg(not(target_family = "wasm"))]
pub fn apply_child_agent_model_override(
    child_surface_id: EntityId,
    model_id: Option<&str>,
    ctx: &mut AppContext,
) {
    let Some(model_id) = model_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return;
    };
    let model_id = LLMId::from(model_id);
    LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
        preferences.set_agent_mode_llm_override(child_surface_id, model_id, ctx);
    });
}
