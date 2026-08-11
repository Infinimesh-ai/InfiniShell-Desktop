use chrono::Utc;
use persistence::model::ConversationUsageMetadata;
use warp_cli::agent::Harness;
use warpui::{App, EntityId, SingletonEntity};

use super::*;
use crate::FeatureFlag;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIAgentHarness, ServerAIConversationMetadata};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::task::{
    AgentConfigSnapshot, HarnessConfig, TaskPrincipalInfo, TaskStatusErrorCode, TaskStatusMessage,
};
use crate::ai::ambient_agents::{
    AgentSource, AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState,
};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::auth::{AuthStateProvider, TEST_USER_UID};
use crate::terminal::TerminalModel;
use crate::terminal::shared_session::{SharedSessionSource, SharedSessionStatus};
use crate::workspaces::user_workspaces::UserWorkspaces;

const CONVERSATION_TOKEN: &str = "server-conversation-token";

// Zap:上游这里还有一组 `ConversationPermissionFixture`(个人/他人/团队所有者、团队 guest),
// 用来驱动 `conversation_access` 的云端 ACL 判定。Zap 已剥离云端 ACL:
// `ServerAIConversationMetadata` 不再带 `metadata` / `creator` / `permissions`,
// `conversation_access` 退化为「登录即 Edit、未登录为 Unknown」。
// 因此这里只保留登录态这一个维度,依赖所有权差异的用例(view-only tombstone、
// 团队所有者/团队 guest、非所有者只读路由)已随该能力一并删除。
#[derive(Clone, Copy)]
enum AuthFixture {
    LoggedIn,
    LoggedOut,
}

struct TestHandles {
    terminal_view_id: EntityId,
    task_id: AmbientAgentTaskId,
}

fn setup_app(app: &mut App, auth_fixture: AuthFixture, harness: AIAgentHarness) -> TestHandles {
    let _agent_management_guard = FeatureFlag::AgentManagementView.override_enabled(false);
    match auth_fixture {
        AuthFixture::LoggedIn => {
            app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        }
        AuthFixture::LoggedOut => {
            app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        }
    }
    app.add_singleton_model(|ctx| UserWorkspaces::mock(vec![], ctx));
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    // `AgentConversationsModel::new` 在构造期就取 `ActiveAgentViewsModel` 句柄。
    app.add_singleton_model(|_| crate::ai::active_agent_views_model::ActiveAgentViewsModel::new());
    app.add_singleton_model(AgentConversationsModel::new);

    let terminal_view_id = EntityId::new();
    let task_id = ambient_task_id(1);
    let task = ambient_agent_task(
        task_id,
        CONVERSATION_TOKEN,
        AmbientAgentTaskState::Succeeded,
    );

    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(task);
    });
    BlocklistAIHistoryModel::handle(app).update(app, |model, ctx| {
        let conversation_id =
            model.start_new_conversation(terminal_view_id, false, false, false, ctx);
        model.set_server_conversation_token_for_conversation(
            conversation_id,
            CONVERSATION_TOKEN.to_string(),
        );
        model.set_server_metadata_for_conversation(
            conversation_id,
            server_conversation_metadata(harness, Some(task_id)),
            ctx,
        );
    });

    TestHandles {
        terminal_view_id,
        task_id,
    }
}

fn setup_task_without_server_metadata(app: &mut App) -> TestHandles {
    setup_task_without_server_metadata_for_creator(app, "other-user")
}

fn setup_owned_task_without_server_metadata(app: &mut App) -> TestHandles {
    setup_task_without_server_metadata_for_creator(app, TEST_USER_UID)
}

fn setup_task_without_server_metadata_for_creator(app: &mut App, creator_uid: &str) -> TestHandles {
    let _agent_management_guard = FeatureFlag::AgentManagementView.override_enabled(false);
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    // `AgentConversationsModel::new` 在构造期就取 `ActiveAgentViewsModel` 句柄。
    app.add_singleton_model(|_| crate::ai::active_agent_views_model::ActiveAgentViewsModel::new());
    app.add_singleton_model(AgentConversationsModel::new);

    let terminal_view_id = EntityId::new();
    let task_id = ambient_task_id(1);
    let task = ambient_agent_task(
        task_id,
        CONVERSATION_TOKEN,
        AmbientAgentTaskState::Succeeded,
    )
    .with_creator(creator_uid);
    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(task);
    });

    TestHandles {
        terminal_view_id,
        task_id,
    }
}

fn ambient_task_id(index: usize) -> AmbientAgentTaskId {
    format!("550e8400-e29b-41d4-a716-{index:012}")
        .parse()
        .unwrap()
}

fn ambient_agent_task(
    task_id: AmbientAgentTaskId,
    conversation_token: &str,
    state: AmbientAgentTaskState,
) -> AmbientAgentTask {
    let now = Utc::now();
    AmbientAgentTask {
        task_id,
        parent_run_id: None,
        title: "Task".to_string(),
        state,
        prompt: "test".to_string(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: None,
        execution_location: None,
        session_id: None,
        session_link: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: TEST_USER_UID.to_string(),
            display_name: None,
        }),
        executor: None,
        conversation_id: Some(conversation_token.to_string()),
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

fn active_ambient_agent_task(task_id: AmbientAgentTaskId) -> AmbientAgentTask {
    let mut task = ambient_agent_task(
        task_id,
        CONVERSATION_TOKEN,
        AmbientAgentTaskState::InProgress,
    );
    task.session_link = Some("https://example.com/session/active".to_string());
    task.is_sandbox_running = true;
    task
}

trait AmbientAgentTaskTestExt {
    fn with_creator(self, creator_uid: &str) -> Self;
    fn with_harness(self, harness: Harness) -> Self;
}

impl AmbientAgentTaskTestExt for AmbientAgentTask {
    fn with_creator(mut self, creator_uid: &str) -> Self {
        self.creator = Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: creator_uid.to_string(),
            display_name: None,
        });
        self
    }

    fn with_harness(mut self, harness: Harness) -> Self {
        self.agent_config_snapshot = Some(AgentConfigSnapshot {
            harness: (harness != Harness::Oz).then_some(HarnessConfig {
                harness_type: harness,
                model_id: None,
                reasoning_level: None,
            }),
            ..Default::default()
        });
        self
    }
}

fn server_conversation_metadata(
    harness: AIAgentHarness,
    ambient_agent_task_id: Option<AmbientAgentTaskId>,
) -> ServerAIConversationMetadata {
    ServerAIConversationMetadata {
        title: "Conversation".to_string(),
        working_directory: None,
        harness,
        usage: ConversationUsageMetadata {
            was_summarized: false,
            context_window_usage: 0.0,
            credits_spent: 0.0,
            platform_credits_spent: 0.0,
            total_provider_cost_in_cents: None,
            credits_spent_for_last_block: None,
            token_usage: vec![],
            tool_usage_metadata: Default::default(),
            context_window_segments: Vec::new(),
        },
        ambient_agent_task_id,
        server_conversation_token: ServerConversationToken::new(CONVERSATION_TOKEN.to_string()),
        artifacts: vec![],
    }
}

#[test]
fn missing_task_returns_error() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id, ..
        } = setup_task_without_server_metadata(&mut app);
        let missing_task_id = ambient_task_id(2);

        app.update(|ctx| {
            let state = resolve_cloud_conversation_continuation_ui_state(
                terminal_view_id,
                missing_task_id,
                ctx,
            );
            assert_eq!(state, Err(CloudConversationContinuationError::MissingTask));
        });
    });
}

#[test]
fn routing_is_live_remote_vm_for_retained_failed_execution() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
        );
        let mut task = active_ambient_agent_task(task_id);
        task.state = AmbientAgentTaskState::Error;
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });
        let model = ambient_pane_model(task_id, SharedSessionStatus::NotShared);

        app.update(|ctx| {
            assert_eq!(
                resolve_ai_query_routing(terminal_view_id, None, &model, ctx),
                AIQueryRouting::LiveRemoteVm {
                    is_executor: false,
                    ambient_agent_task_id: Some(task_id),
                }
            );
        });
    });
}

#[test]
fn routing_starts_new_cloud_vm_for_ended_failed_execution() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
        );
        let mut task =
            ambient_agent_task(task_id, CONVERSATION_TOKEN, AmbientAgentTaskState::Failed);
        task.session_link = Some("https://example.com/session/stale".to_string());
        task.is_sandbox_running = false;
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });
        let model = ambient_pane_model(task_id, SharedSessionStatus::NotShared);

        app.update(|ctx| {
            assert_eq!(
                resolve_ai_query_routing(terminal_view_id, None, &model, ctx),
                AIQueryRouting::NewCloudVm { task_id }
            );
        });
    });
}

#[test]
fn github_action_source_shows_tombstone_without_cta() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_owned_task_without_server_metadata(&mut app);
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            let mut task = ambient_agent_task(
                task_id,
                CONVERSATION_TOKEN,
                AmbientAgentTaskState::Succeeded,
            )
            .with_creator(TEST_USER_UID);
            task.source = Some(AgentSource::GitHubAction);
            model.insert_task_for_test(task);
        });

        app.update(|ctx| {
            assert_eq!(
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx),
                Ok(CloudConversationContinuationUiState::Tombstone { cta: None })
            );
        });
    });
}

#[test]
fn oz_conversation_with_edit_access_shows_inline_followup_input() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);
            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::FollowupInput)
            );
        });
    });
}

#[test]
fn third_party_conversation_with_edit_access_shows_continue_in_cloud_tombstone() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id }),
                })
            );
        });
    });
}

#[test]
fn environment_setup_failure_without_conversation_shows_tombstone_without_cta() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
        );
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            let mut task =
                ambient_agent_task(task_id, CONVERSATION_TOKEN, AmbientAgentTaskState::Failed);
            task.conversation_id = None;
            task.status_message = Some(TaskStatusMessage {
                message: "Environment setup failed: Failed to run setup command: hi".to_string(),
                error_code: Some(TaskStatusErrorCode::EnvironmentSetupFailed),
            });
            model.insert_task_for_test(task);
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone { cta: None })
            );
        });
    });
}

#[test]
fn environment_setup_failure_with_conversation_shows_continue_cta() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::ClaudeCode,
        );
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            let mut task =
                ambient_agent_task(task_id, CONVERSATION_TOKEN, AmbientAgentTaskState::Failed);
            task.status_message = Some(TaskStatusMessage {
                message: "Environment setup failed: Failed to run setup command: hi".to_string(),
                error_code: Some(TaskStatusErrorCode::EnvironmentSetupFailed),
            });
            model.insert_task_for_test(task);
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id }),
                })
            );
        });
    });
}

#[test]
fn unknown_access_returns_error() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedOut,
            AIAgentHarness::ClaudeCode,
        );

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Err(CloudConversationContinuationError::UnknownConversationAccess)
            );
        });
    });
}

#[test]
fn missing_metadata_returns_error() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_task_without_server_metadata(&mut app);

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Err(CloudConversationContinuationError::MissingServerConversationMetadata)
            );
        });
    });
}

#[test]
fn owned_oz_task_without_metadata_shows_inline_followup_input() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_owned_task_without_server_metadata(&mut app);

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::FollowupInput)
            );
        });
    });
}

#[test]
fn owned_third_party_task_without_metadata_shows_continue_in_cloud_tombstone() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_owned_task_without_server_metadata(&mut app);
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(
                ambient_agent_task(
                    task_id,
                    CONVERSATION_TOKEN,
                    AmbientAgentTaskState::Succeeded,
                )
                .with_creator(TEST_USER_UID)
                .with_harness(Harness::Claude),
            );
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id }),
                })
            );
        });
    });
}

#[test]
fn active_task_execution_returns_error() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
        );
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(active_ambient_agent_task(task_id));
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Err(CloudConversationContinuationError::ActiveTaskExecution)
            );
        });
    });
}
#[test]
fn retained_failed_task_execution_returns_active_execution_error() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
        );
        let mut task = active_ambient_agent_task(task_id);
        task.state = AmbientAgentTaskState::Failed;
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Err(CloudConversationContinuationError::ActiveTaskExecution)
            );
        });
    });
}

#[test]
fn ended_failed_task_uses_ordinary_cloud_continuation() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
        );
        let mut task =
            ambient_agent_task(task_id, CONVERSATION_TOKEN, AmbientAgentTaskState::Failed);
        task.session_link = Some("https://example.com/session/stale".to_string());
        task.is_sandbox_running = false;
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });

        app.update(|ctx| {
            let state =
                resolve_cloud_conversation_continuation_ui_state(terminal_view_id, task_id, ctx);

            assert_eq!(
                state,
                Ok(CloudConversationContinuationUiState::FollowupInput)
            );
        });
    });
}

/// Builds a disconnected ambient cloud pane model carrying `task_id` as its shared-session source
/// orchestrator id, with the given collaboration `status`.
fn ambient_pane_model(task_id: AmbientAgentTaskId, status: SharedSessionStatus) -> TerminalModel {
    let mut model = TerminalModel::mock(None, None);
    model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
        task_id.to_string(),
    )));
    model.set_shared_session_status(status);
    model
}

#[test]
fn routing_is_local_for_non_cloud_pane() {
    App::test((), |mut app| async move {
        let model = TerminalModel::mock(None, None);
        app.update(|ctx| {
            assert_eq!(
                resolve_ai_query_routing(EntityId::new(), None, &model, ctx),
                AIQueryRouting::Local
            );
        });
    });
}

#[test]
fn routing_is_live_remote_vm_for_active_viewer() {
    App::test((), |mut app| async move {
        let model = ambient_pane_model(ambient_task_id(1), SharedSessionStatus::reader());
        app.update(|ctx| {
            assert_eq!(
                resolve_ai_query_routing(EntityId::new(), None, &model, ctx),
                AIQueryRouting::LiveRemoteVm {
                    is_executor: false,
                    ambient_agent_task_id: Some(ambient_task_id(1)),
                }
            );
        });
    });
}

#[test]
fn routing_omits_task_id_for_non_ambient_shared_session_viewer() {
    App::test((), |mut app| async move {
        // A viewer of a shared *local* session (no ambient task) still forwards to the sharer, but
        // carries no ambient task id, so the footer live-VM indicator stays hidden.
        let mut model = TerminalModel::mock(None, None);
        model.set_shared_session_status(SharedSessionStatus::executor());
        app.update(|ctx| {
            assert_eq!(
                resolve_ai_query_routing(EntityId::new(), None, &model, ctx),
                AIQueryRouting::LiveRemoteVm {
                    is_executor: true,
                    ambient_agent_task_id: None,
                }
            );
        });
    });
}

#[test]
fn routing_is_local_for_active_sharer_local_orchestration_child() {
    App::test((), |mut app| async move {
        let model = ambient_pane_model(ambient_task_id(1), SharedSessionStatus::ActiveSharer);
        app.update(|ctx| {
            assert_eq!(
                resolve_ai_query_routing(EntityId::new(), None, &model, ctx),
                AIQueryRouting::Local
            );
        });
    });
}

#[test]
fn routing_is_new_cloud_vm_for_owned_oz_disconnected_pane() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
        );
        let model = ambient_pane_model(task_id, SharedSessionStatus::NotShared);
        app.update(|ctx| {
            assert_eq!(
                resolve_ai_query_routing(terminal_view_id, None, &model, ctx),
                AIQueryRouting::NewCloudVm { task_id }
            );
        });
    });
}

#[test]
fn routing_is_live_remote_vm_for_active_execution_without_attached_viewer() {
    App::test((), |mut app| async move {
        let TestHandles {
            terminal_view_id,
            task_id,
        } = setup_app(
            &mut app,
            AuthFixture::LoggedIn,
            AIAgentHarness::Oz,
        );
        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(active_ambient_agent_task(task_id));
        });
        let model = ambient_pane_model(task_id, SharedSessionStatus::NotShared);
        app.update(|ctx| {
            assert_eq!(
                resolve_ai_query_routing(terminal_view_id, None, &model, ctx),
                AIQueryRouting::LiveRemoteVm {
                    is_executor: false,
                    ambient_agent_task_id: Some(task_id),
                }
            );
        });
    });
}
