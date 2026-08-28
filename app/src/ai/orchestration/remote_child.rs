//! Prepares orchestrated remote-child launches and classifies their startup errors.
//!
//! This module owns frontend-neutral request construction and startup issue semantics;
//! frontend-specific callers remain responsible for lifecycle state and presentation.
use std::path::{Path, PathBuf};
#[cfg(not(target_family = "wasm"))]
use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use prost::Message as _;
use warp_cli::agent::Harness;
#[cfg(not(target_family = "wasm"))]
use warp_cli::skill::SkillSpec;
use warp_multi_agent_api as multi_agent_api;
#[cfg(not(target_family = "wasm"))]
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, SingletonEntity as _};

// Zap:云端 `server_api` 网关已删除,这里改用本地的 `ambient_agents::SpawnAgentRequest`
// 与 `ai::api_error` 中的错误类型;云端专属的 `CloudAgentCapacityError` 已随之下线。
use crate::ai::ambient_agents::task::{
    AgentConfigSnapshot, HarnessAuthSecretsConfig, HarnessConfig, normalize_orchestrator_agent_name,
};
use crate::ai::ambient_agents::{
    SpawnAgentRequest, github_auth_url, out_of_credits_task_failure_message,
    server_overloaded_task_failure_message,
};
use crate::ai::api_error::{AIApiError, ClientError};
use crate::ai::blocklist::StartAgentRequest;
#[cfg(not(target_family = "wasm"))]
use crate::ai::skills::resolve_skill_spec;
use crate::ai::skills::{SkillManager, SkillReference};

/// Remote execution fields carried by [`crate::ai::agent::StartAgentExecutionMode`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteChildLaunchConfig {
    pub environment_id: String,
    pub skill_references: Vec<SkillReference>,
    pub working_dir: PathBuf,
    pub model_id: String,
    pub computer_use_enabled: bool,
    pub worker_host: String,
    pub harness_type: String,
    pub title: String,
    pub auth_secret_name: Option<String>,
    pub runner_id: String,
    pub agent_identity_uid: Option<String>,
}

impl RemoteChildLaunchConfig {
    pub fn orchestration_harness(&self) -> Harness {
        if self.harness_type.trim().is_empty() {
            Harness::Oz
        } else {
            Harness::parse_orchestration_harness(&self.harness_type).unwrap_or(Harness::Unknown)
        }
    }
}

/// Fallback for repo-qualified skill specs (`repo:skill`, `org/repo:path`)
/// that miss the active-skill fast path in `resolve_runtime_skills`. Bundled
/// ids, remote paths, and active/absolute local paths stay on the fast path:
/// `resolve_skill_spec` only resolves repo-qualified specs off the local
/// filesystem and does not honor bundled-skill activation.
#[cfg(not(target_family = "wasm"))]
fn resolve_repo_qualified_skill(
    reference: &SkillReference,
    working_dir: &Path,
    ctx: &AppContext,
) -> Option<Result<ai::skills::ParsedSkill, String>> {
    let SkillReference::Path(LocalOrRemotePath::Local(path)) = reference else {
        return None;
    };
    let spec = SkillSpec::from_str(&path.display().to_string()).ok()?;
    // Bail unless the spec is repo-qualified (has a repo component).
    spec.repo.as_ref()?;
    Some(
        resolve_skill_spec(&spec, working_dir, ctx)
            .map(|resolved| resolved.parsed_skill)
            .map_err(|error| error.to_string()),
    )
}

#[cfg(target_family = "wasm")]
fn resolve_repo_qualified_skill(
    _reference: &SkillReference,
    _working_dir: &Path,
    _ctx: &AppContext,
) -> Option<Result<ai::skills::ParsedSkill, String>> {
    None
}

/// Frontend-neutral output used to launch one remote child.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
#[derive(Clone, Debug)]
pub struct PreparedRemoteChildLaunch {
    pub display_name: String,
    pub orchestration_harness: Harness,
    pub spawn_request: SpawnAgentRequest,
}

/// Failure while constructing the remote child request, before calling the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrepareRemoteChildLaunchError {
    MissingParentRunId,
    UnresolvedSkills { references: Vec<String> },
}

impl PrepareRemoteChildLaunchError {
    pub fn user_message(&self) -> String {
        match self {
            Self::MissingParentRunId => {
                crate::t!("ai-orchestration-parent-run-required")
            }
            Self::UnresolvedSkills { references } => {
                crate::t!(
                    "ai-orchestration-skill-resolution-failed",
                    references = references.join(", ")
                )
            }
        }
    }
}

/// A recoverable startup condition that requires user action.
///
/// The GUI represents this as `ambient_agent::Status::NeedsGithubAuth`.
/// Orchestrated children retain their surface so the user can follow the
/// remediation link, but the original child launch still resolves as failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudAgentStartupBlocker {
    GitHubAuthRequired { message: String, auth_url: String },
}

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
impl CloudAgentStartupBlocker {
    pub fn message(&self) -> &str {
        match self {
            Self::GitHubAuthRequired { message, .. } => message,
        }
    }

    pub fn primary_url(&self) -> &str {
        match self {
            Self::GitHubAuthRequired { auth_url, .. } => auth_url,
        }
    }
}

/// A terminal cloud-agent startup failure.
///
/// The GUI represents these as `ambient_agent::Status::Failed`. Unlike a
/// blocker, a failure has no remediation action that requires retaining an
/// optimistic orchestrated-child surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudAgentStartupFailure {
    Capacity { message: String },
    OutOfCredits { message: String },
    ServerOverloaded { message: String },
    Other { message: String },
}

#[cfg_attr(not(feature = "tui"), allow(dead_code))]
impl CloudAgentStartupFailure {
    pub fn message(&self) -> &str {
        match self {
            Self::Capacity { message }
            | Self::OutOfCredits { message }
            | Self::ServerOverloaded { message }
            | Self::Other { message } => message,
        }
    }
}

/// Whether authentication can resume a retained launch or requires the user to rerun it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudAgentStartupAuthFlow {
    RetryRetainedRequest,
    #[cfg_attr(not(feature = "tui"), allow(dead_code))]
    RerunOrchestrationRequest,
}

/// Renderer-neutral content for a cloud-agent startup card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudAgentStartupPresentation {
    pub title: String,
    pub detail: String,
    pub action_label: Option<String>,
    pub primary_url: Option<String>,
}

impl CloudAgentStartupPresentation {
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            title: crate::t!("ai-orchestration-cloud-environment-failed"),
            detail: message.into(),
            action_label: None,
            primary_url: None,
        }
    }

    pub fn github_auth(auth_url: impl Into<String>, flow: CloudAgentStartupAuthFlow) -> Self {
        let detail = match flow {
            CloudAgentStartupAuthFlow::RetryRetainedRequest => {
                crate::t!("ai-orchestration-github-auth-continue")
            }
            CloudAgentStartupAuthFlow::RerunOrchestrationRequest => {
                crate::t!("ai-orchestration-github-auth-rerun")
            }
        };
        Self {
            title: crate::t!("ai-orchestration-github-auth-required"),
            detail,
            action_label: Some(crate::t!("ai-orchestration-authenticate-github")),
            primary_url: Some(auth_url.into()),
        }
    }
}
/// Shared interpretation of an error returned while starting a cloud agent.
///
/// This distinction preserves the existing orchestrated-child contract:
/// blockers remain visible for user action, while terminal failures are
/// eligible for failed-launch cleanup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudAgentStartupIssue {
    Blocked(CloudAgentStartupBlocker),
    Failed(CloudAgentStartupFailure),
}

/// Builds the public API request for one remote child without owning frontend lifecycle state.
pub fn prepare_remote_child_launch(
    request: &StartAgentRequest,
    config: RemoteChildLaunchConfig,
    ctx: &AppContext,
) -> Result<PreparedRemoteChildLaunch, PrepareRemoteChildLaunchError> {
    let orchestration_harness = config.orchestration_harness();
    let RemoteChildLaunchConfig {
        environment_id,
        skill_references,
        working_dir,
        model_id,
        computer_use_enabled,
        worker_host,
        harness_type,
        title,
        auth_secret_name,
        runner_id,
        agent_identity_uid,
    } = config;
    let Some(parent_run_id) = request.parent_run_id.clone() else {
        return Err(PrepareRemoteChildLaunchError::MissingParentRunId);
    };
    let runtime_skills = resolve_runtime_skills(&skill_references, &working_dir, ctx)?;
    let agent_name = normalize_orchestrator_agent_name(&request.name);
    let display_name = agent_name.clone().unwrap_or_default();
    let environment_id = Some(environment_id).filter(|id| !id.trim().is_empty());
    let harness_override = if harness_type.is_empty() {
        None
    } else {
        match <Harness as clap::ValueEnum>::from_str(&harness_type, true) {
            Ok(harness) => Some(HarnessConfig::from_harness_type(harness)),
            Err(_) => {
                log::warn!(
                    "Unknown child-agent harness type: {harness_type:?}; omitting harness override so the server picks its default"
                );
                None
            }
        }
    };
    let computer_use_enabled =
        (orchestration_harness == Harness::Oz).then_some(computer_use_enabled);
    let harness_auth_secrets = auth_secret_name
        .filter(|name| !name.trim().is_empty())
        .and_then(|name| match orchestration_harness {
            Harness::Claude => Some(HarnessAuthSecretsConfig {
                claude_auth_secret_name: Some(name),
                codex_auth_secret_name: None,
            }),
            Harness::Codex => Some(HarnessAuthSecretsConfig {
                claude_auth_secret_name: None,
                codex_auth_secret_name: Some(name),
            }),
            Harness::Oz | Harness::OpenCode | Harness::Gemini | Harness::Unknown => None,
        });
    // Zap:本地 `SpawnAgentRequest` 不带云端字段(mode / conversation_id /
    // initial_snapshot_token / agent_identity_uid / snapshot_disabled /
    // orchestration_handoff),它们只服务于已删除的远端 run 接口,这里直接忽略。
    let _ = (agent_identity_uid, should_disable_snapshot(ctx));
    let spawn_request = SpawnAgentRequest {
        prompt: request.prompt.clone(),
        config: Some(AgentConfigSnapshot {
            name: agent_name,
            environment_id,
            runner_id: (!runner_id.is_empty()).then_some(runner_id),
            model_id: (!model_id.is_empty()).then_some(model_id),
            worker_host: (!worker_host.is_empty()).then_some(worker_host),
            computer_use_enabled,
            harness: harness_override,
            harness_auth_secrets,
            ..Default::default()
        }),
        title: (!title.is_empty()).then_some(title),
        team: None,
        skill: None,
        attachments: Vec::new(),
        interactive: Some(true),
        parent_run_id: Some(parent_run_id),
        runtime_skills,
        referenced_attachments: Vec::new(),
    };
    Ok(PreparedRemoteChildLaunch {
        display_name,
        orchestration_harness,
        spawn_request,
    })
}

/// Maps server/client launch failures into shared startup presentation.
pub fn classify_cloud_agent_startup_error(error: &anyhow::Error) -> CloudAgentStartupIssue {
    if let Some(client_error) = error.downcast_ref::<ClientError>()
        && let Some(auth_url) = &client_error.auth_url
    {
        return CloudAgentStartupIssue::Blocked(CloudAgentStartupBlocker::GitHubAuthRequired {
            message: client_error.error.clone(),
            auth_url: github_auth_url::cloud_setup_auth_url_with_next(auth_url),
        });
    }
    // Zap:云端容量错误(`CloudAgentCapacityError`)随 `server_api` 一并删除,
    // 对应的 `CloudAgentStartupFailure::Capacity` 分支不再有来源。
    if let Some(ai_api_error) = error.downcast_ref::<AIApiError>() {
        match ai_api_error {
            AIApiError::QuotaLimit => {
                return CloudAgentStartupIssue::Failed(CloudAgentStartupFailure::OutOfCredits {
                    message: out_of_credits_task_failure_message(),
                });
            }
            AIApiError::ServerOverloaded => {
                return CloudAgentStartupIssue::Failed(
                    CloudAgentStartupFailure::ServerOverloaded {
                        message: server_overloaded_task_failure_message(),
                    },
                );
            }
            AIApiError::Transport(_)
            | AIApiError::Deserialization(_)
            | AIApiError::NoContextFound
            | AIApiError::ErrorStatus(_, _)
            | AIApiError::ProviderProtocol(_)
            | AIApiError::Other(_)
            | AIApiError::Stream { .. } => {}
        }
    }
    CloudAgentStartupIssue::Failed(CloudAgentStartupFailure::Other {
        message: error.to_string(),
    })
}

/// Zap:会话快照上传到云端存储的开关(`PrivacySettings::is_cloud_conversation_storage_enabled`
/// 与工作区级 `AdminEnablementSetting`)随云端会话存储链路一并删除。本地优先分支从不上传
/// 快照,因此这里恒定返回 `true`(禁用快照),签名保留供既有调用点使用。
pub(crate) fn should_disable_snapshot(ctx: &AppContext) -> bool {
    let _ = ctx;
    true
}

/// Builds the Oz web URL for a server-assigned agent run ID.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub fn oz_run_url(run_id: &str) -> String {
    // Zap:`ChannelState::oz_root_url()`(Oz web 根地址)随云端配置删除,
    // 本地分支不存在可跳转的 Oz run 页面,返回空串表示 "无链接"。
    let _ = run_id;
    String::new()
}

fn resolve_runtime_skills(
    skill_references: &[SkillReference],
    working_dir: &Path,
    ctx: &AppContext,
) -> Result<Vec<String>, PrepareRemoteChildLaunchError> {
    let skill_manager = SkillManager::as_ref(ctx);
    let mut runtime_skills = Vec::with_capacity(skill_references.len());
    let mut unresolved_references = Vec::new();
    for reference in skill_references {
        if let Some(skill) = skill_manager.active_skill_by_reference(reference, ctx) {
            runtime_skills.push(
                BASE64_STANDARD.encode(multi_agent_api::Skill::from(skill.clone()).encode_to_vec()),
            );
            continue;
        }

        match resolve_repo_qualified_skill(reference, working_dir, ctx) {
            Some(Ok(skill)) => {
                runtime_skills.push(
                    BASE64_STANDARD.encode(multi_agent_api::Skill::from(skill).encode_to_vec()),
                );
            }
            Some(Err(error)) => {
                unresolved_references.push(format!("{reference} ({error})"));
            }
            None => {
                unresolved_references.push(reference.to_string());
            }
        }
    }
    if unresolved_references.is_empty() {
        Ok(runtime_skills)
    } else {
        Err(PrepareRemoteChildLaunchError::UnresolvedSkills {
            references: unresolved_references,
        })
    }
}

#[cfg(test)]
#[path = "remote_child_tests.rs"]
mod tests;
