use pathfinder_geometry::vector::vec2f;
use persistence::model::ConversationUsageMetadata;
use warpui::platform::WindowStyle;
use warpui::{App, EntityId, ViewHandle};

use super::*;
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIAgentHarness, ServerAIConversationMetadata};
use crate::ai::agent_conversations_model::{AgentConversationsModel, AgentConversationsModelEvent};
use crate::ai::ambient_agents::task::TaskPrincipalInfo;
use crate::ai::ambient_agents::{
    AgentSource, AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState,
};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
// Zap:`crate::auth` 是单文件 facade,`auth::user` 子模块不存在。
use crate::auth::TEST_USER_UID;
use crate::context_chips::prompt_type::PromptType;
use crate::editor::InteractionState;
use crate::terminal::TerminalView;
use crate::terminal::model::blocks::{INLINE_BANNER_HEIGHT, ToTotalIndex as _};
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::view::shared_session::test_utils::terminal_view_for_viewer;
use crate::terminal::view::{AIQueryRouting, TerminalAction, resolve_ai_query_routing};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::{FeatureFlag, assert_lines_approx_eq};

#[test]
fn test_prompt_context_menu_items_shared_session_viewer_no_edit_prompt() {
    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            view.current_prompt.update(ctx, |prompt, ctx| {
                model.set_shared_session_status(SharedSessionStatus::ActiveViewer {
                    role: Default::default(),
                });

                let PromptType::Dynamic { prompt } = prompt else {
                    return;
                };
                prompt.update(ctx, |prompt, ctx| {
                    prompt.update_context(model.block_list().active_block(), ctx)
                });
            })
        });

        let session_settings = SessionSettings::handle(&app);
        session_settings.update(&mut app, |settings, ctx| {
            let _ = settings.honor_ps1.set_value(false, ctx);
        });

        terminal.read(&app, |view, ctx| {
            let items: Vec<MenuItem<TerminalAction>> = view.prompt_context_menu_items(ctx);
            assert_eq!(items.len(), 3);

            // We expect the prompt menu items to be something like the following when no context chips exist:
            // Copy prompt
            // ------------
            // Edit prompt (disabled for shared-session viewers)
            assert_eq!(items[0].fields().unwrap().label(), "Copy prompt");
            assert!(items[1].is_separator());
            assert_eq!(items[2].fields().unwrap().label(), "Edit prompt");
            assert!(items[2].fields().unwrap().is_disabled());
        });
    })
}

// 此处原有 19 个上游云端能力测试(cloud mode / handoff / oz 共享会话),随该能力剥离一并删除。

#[test]
fn test_begin_viewing_ambient_session_reuses_existing_model_for_cloud_pane() {
    // The upfront cloud-mode path already created the ambient view model at construction;
    // begin_viewing_ambient_session must reuse it (idempotent) rather than replacing it.
    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task_id = "44444444-4444-4444-4444-444444444444"
            .parse::<AmbientAgentTaskId>()
            .expect("hardcoded task id parses");
        let session_id = SessionId::new();

        let original_model_id = terminal.read(&app, |view, _| {
            view.ambient_agent_view_model()
                .expect("cloud mode terminal has an ambient view model")
                .id()
        });

        terminal.update(&mut app, |view, ctx| {
            view.begin_viewing_ambient_session(task_id, session_id, ctx);
        });

        terminal.read(&app, |view, ctx| {
            let model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal still has an ambient view model");
            assert_eq!(
                model.id(),
                original_model_id,
                "begin_viewing_ambient_session must reuse the existing model, not replace it"
            );
            assert_eq!(model.as_ref(ctx).task_id(), Some(task_id));
        });
    });
}

#[test]
fn test_shared_session_banners() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        let mut expected_block_heights_len = terminal.read(&app, |view, _| {
            assert!(matches!(
                view.inline_banners_state.shared_session_banner_state,
                SharedSessionBanners::None
            ));
            view.model.lock().block_list().block_heights().items().len()
        });

        // Make a block and then insert the shared session starter banner.
        terminal.update(&mut app, |view, ctx| {
            view.model.lock().simulate_block("ls", "foo");
            view.insert_shared_session_started_banner(
                SharedSessionScrollbackType::All,
                false,
                Local::now(),
                ctx,
            );
            expected_block_heights_len += 2;
        });

        terminal.read(&app, |view, _ctx| {
            let model = view.model.lock();

            // Make sure the state has changed.
            assert!(matches!(
                view.inline_banners_state.shared_session_banner_state,
                SharedSessionBanners::ActiveShare { .. }
            ));

            // We should have inserted a block and a banner.
            let block_height_items = model.block_list().block_heights().items();
            assert_eq!(block_height_items.len(), expected_block_heights_len);

            // The banner should have been inserted before the first visible block.
            let first_block_total_index = model
                .block_list()
                .first_non_hidden_block_by_index()
                .unwrap()
                .to_total_index(model.block_list());
            assert_lines_approx_eq!(
                block_height_items[first_block_total_index.0 - 1]
                    .height()
                    .into_lines(),
                INLINE_BANNER_HEIGHT
            );
        });

        // Insert another block and then the shared session ended banner.
        terminal.update(&mut app, |view, ctx| {
            view.model.lock().simulate_block("ls", "foo");
            view.insert_shared_session_ended_banner(ctx);
            expected_block_heights_len += 2;
        });

        terminal.read(&app, |view, _ctx| {
            let model = view.model.lock();

            // Make sure the state has changed.
            assert!(matches!(
                view.inline_banners_state.shared_session_banner_state,
                SharedSessionBanners::LastShared { .. }
            ));

            // by now, we've inserted two new blocks and two new banners since the initialization of the view.
            let block_height_items = model.block_list().block_heights().items();
            assert_eq!(block_height_items.len(), expected_block_heights_len);

            // The first banner should continue to be at the start of the blocklist.
            let first_block_total_index = model
                .block_list()
                .first_non_hidden_block_by_index()
                .unwrap()
                .to_total_index(model.block_list());
            assert_lines_approx_eq!(
                block_height_items[first_block_total_index.0 - 1]
                    .height()
                    .into_lines(),
                INLINE_BANNER_HEIGHT
            );

            // The second banner should be at the end of the blocklist, before the active block.
            let last_block_total_index = model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap()
                .to_total_index(model.block_list());
            assert_lines_approx_eq!(
                block_height_items[last_block_total_index.0 + 1]
                    .height()
                    .into_lines(),
                INLINE_BANNER_HEIGHT
            );
        });

        // Mimic starting a shared session again in the same view.
        terminal.update(&mut app, |view, ctx| {
            view.insert_shared_session_started_banner(
                SharedSessionScrollbackType::None,
                false,
                Local::now(),
                ctx,
            );

            // We should have removed two banners and inserted one. So overall,
            // we lost one item in the blocklist since the last time.
            expected_block_heights_len -= 1;
        });

        terminal.read(&app, |view, _ctx| {
            let model = view.model.lock();

            // Make sure the state has changed.
            assert!(matches!(
                view.inline_banners_state.shared_session_banner_state,
                SharedSessionBanners::ActiveShare { .. }
            ));

            // We should have removed two banners and inserted one. So overall,
            // we lost one item in the blocklist since the last time.
            let block_height_items = model.block_list().block_heights().items();
            assert_eq!(block_height_items.len(), expected_block_heights_len);

            // The banner should have been inserted at the end of the blocklist, before the active block.
            let last_block_total_index = model
                .block_list()
                .last_non_hidden_block_by_index()
                .unwrap()
                .to_total_index(model.block_list());
            assert_lines_approx_eq!(
                block_height_items[last_block_total_index.0 + 1]
                    .height()
                    .into_lines(),
                INLINE_BANNER_HEIGHT
            );
        });
    })
}

#[test]
fn test_resize_shared_session_viewer_from_server() {
    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        terminal.update(&mut app, |view, ctx| {
            // Refresh the size at the start of the test to make sure
            // we're using a consistent size throughout.
            view.refresh_size(ctx);
        });

        let model = terminal.read(&app, |view, _| view.model.clone());
        model
            .lock()
            .set_shared_session_status(SharedSessionStatus::ActiveViewer {
                role: Default::default(),
            });

        // The viewer's current size info.
        let original_size_info = *model.lock().block_list().size();
        let original_num_rows = original_size_info.rows();
        let original_num_cols = original_size_info.columns();

        // Case 1: suppose the sharer has a larger size.
        // The size info we expect is the old one with the greater
        // number of rows and columns (nothing else changed).
        let new_num_rows = original_num_rows + 1;
        let new_num_cols = original_num_cols + 1;
        let expected_size_info =
            original_size_info.with_rows_and_columns(new_num_rows, new_num_cols);

        terminal.update(&mut app, |view, ctx| {
            view.resize_from_sharer_update(
                WindowSize {
                    num_rows: new_num_rows,
                    num_cols: new_num_cols,
                },
                ctx,
            );
        });

        // Make sure the view and model reflect the new, expected size info.
        terminal.read(&app, |view, _ctx| {
            assert_eq!(*view.size_info(), expected_size_info);
            assert_eq!(*view.model.lock().block_list().size(), expected_size_info);
        });

        // Case 2: suppose the sharer has a smaller size.
        // The size info we expect is our old, larger one; nothing changed.
        let new_num_rows = original_num_rows - 1;
        let new_num_cols = original_num_cols - 1;
        let expected_size_info = original_size_info;

        terminal.update(&mut app, |view, ctx| {
            view.resize_from_sharer_update(
                WindowSize {
                    num_rows: new_num_rows,
                    num_cols: new_num_cols,
                },
                ctx,
            );
        });

        // Make sure the view and model reflect the old, expected size info.
        terminal.read(&app, |view, _ctx| {
            assert_eq!(*view.size_info(), expected_size_info);
            assert_eq!(*view.model.lock().block_list().size(), expected_size_info);
        });
    })
}

#[test]
fn test_resize_shared_session_viewer_independent_of_sharer() {
    App::test((), |mut app| async move {
        let terminal = terminal_view_for_viewer(&mut app);
        terminal.update(&mut app, |view, ctx| {
            // Refresh the size at the start of the test to make sure
            // we're using a consistent size throughout.
            view.after_terminal_view_layout(vec2f(100., 100.), ctx);

            // Set the sharer's size.
            let num_rows = view.size_info().rows();
            let num_cols = view.size_info().columns();
            view.resize_from_sharer_update(WindowSize { num_rows, num_cols }, ctx);
        });

        let original_size_info = terminal.read(&app, |view, _| *view.size_info());
        let original_num_rows = original_size_info.rows();
        let original_num_cols = original_size_info.columns();

        // Case 1: make the viewer winsize smaller by making the pane narrower.
        terminal.update(&mut app, |view, ctx| {
            let narrower = vec2f(
                original_size_info.pane_width_px().as_f32() - 10.,
                original_size_info.pane_height_px().as_f32(),
            );
            view.after_terminal_view_layout(narrower, ctx);
        });

        // Make sure the overall size info was changed but the rows, columns
        // were unchanged because we're respecting the sharer's larger size.
        terminal.read(&app, |view, _ctx| {
            let new_size_info = *view.size_info();
            assert_ne!(original_size_info, new_size_info);

            let expected_size_info =
                new_size_info.with_rows_and_columns(original_num_rows, original_num_cols);
            assert_eq!(*view.size_info(), expected_size_info);
            assert_eq!(*view.model.lock().block_list().size(), expected_size_info);
        });

        // Case 2: make the viewer winsize larger by making the pane wider.
        terminal.update(&mut app, |view, ctx| {
            let wider = vec2f(
                original_size_info.pane_width_px().as_f32() + 10.,
                original_size_info.pane_height_px().as_f32(),
            );
            view.after_terminal_view_layout(wider, ctx);
        });

        // Make sure the overall size info was changed, and that the rows, columns
        // were updated because we're respecting the viewer's larger size.
        terminal.read(&app, |view, _ctx| {
            let new_size_info = *view.size_info();
            assert_ne!(original_size_info, new_size_info);

            assert!(new_size_info.columns() > original_num_cols);
            assert!(view.model.lock().block_list().size().columns() > original_num_cols);
        });
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_on_session_share_ended_restores_size_after_viewer_driven_resize() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            // Refresh the size at the start of the test to make sure
            // we're using a consistent size throughout.
            view.after_terminal_view_layout(vec2f(100., 100.), ctx);
        });

        let original_size = terminal.read(&app, |view, _| *view.size_info());
        let viewer_rows = original_size.rows().saturating_sub(2).max(1);
        let viewer_cols = original_size.columns().saturating_sub(4).max(1);
        assert!(viewer_rows < original_size.rows() || viewer_cols < original_size.columns());

        // Resize the view as if a viewer with a smaller winsize has joined the session.
        terminal.update(&mut app, |view, ctx| {
            view.resize_from_viewer_report(
                WindowSize {
                    num_rows: viewer_rows,
                    num_cols: viewer_cols,
                },
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert_eq!(view.size_info().rows(), viewer_rows);
            assert_eq!(view.size_info().columns(), viewer_cols);
            assert_eq!(
                view.active_viewer_driven_size,
                Some((viewer_rows, viewer_cols))
            );
            assert_eq!(*view.model.lock().block_list().size(), *view.size_info());
        });

        // End the session, assert that the winsize was restored to the original.
        terminal.update(&mut app, |view, ctx| {
            view.on_session_share_ended(ctx);
        });

        terminal.read(&app, |view, _| {
            assert_eq!(view.size_info().rows(), original_size.rows());
            assert_eq!(view.size_info().columns(), original_size.columns());
            assert_eq!(view.active_viewer_driven_size, None);
            assert_eq!(*view.model.lock().block_list().size(), original_size);
        });
    })
}

#[test]
fn test_on_session_share_ended_skips_cloud_continuation_for_user_share_with_task_id() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });
        let initial_block_height_items = terminal.read(&app, |view, _| {
            view.model.lock().block_list().block_heights().items().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::user(Some(task_id.to_string())));

            view.on_session_share_ended(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();
            assert_eq!(
                model.block_list().block_heights().items().len(),
                initial_block_height_items + 1
            );
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, None);
            assert!(view.is_input_box_visible(&model, ctx));
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

fn create_cloud_mode_task_for_user(creator_uid: &str) -> AmbientAgentTask {
    let now = chrono::Utc::now();
    AmbientAgentTask {
        task_id: uuid::Uuid::new_v4().to_string().parse().unwrap(),
        parent_run_id: None,
        title: "Owned task".to_string(),
        state: AmbientAgentTaskState::Succeeded,
        prompt: "test".to_string(),
        created_at: now,
        started_at: Some(now),
        updated_at: now,
        run_time: Some("PT1S".parse().unwrap()),
        status_message: None,
        source: Some(AgentSource::CloudMode),
        execution_location: None,
        session_id: None,
        session_link: None,
        creator: Some(TaskPrincipalInfo {
            creator_type: "USER".to_string(),
            uid: creator_uid.to_string(),
            display_name: None,
        }),
        executor: None,
        conversation_id: None,
        request_usage: None,
        is_sandbox_running: false,
        agent_config_snapshot: None,
        artifacts: vec![],
        last_event_sequence: None,
        children: vec![],
    }
}

fn insert_cloud_mode_task_with_server_metadata(
    app: &mut App,
    terminal_view_id: EntityId,
    mut task: AmbientAgentTask,
    harness: AIAgentHarness,
) {
    let task_id = task.task_id;
    let conversation_token = task_id.to_string();
    task.conversation_id = Some(conversation_token.clone());

    AgentConversationsModel::handle(app).update(app, |model, _| {
        model.insert_task_for_test(task);
    });
    BlocklistAIHistoryModel::handle(app).update(app, |model, ctx| {
        let conversation_id =
            model.start_new_conversation(terminal_view_id, false, false, false, ctx);
        model.set_server_conversation_token_for_conversation(
            conversation_id,
            conversation_token.clone(),
        );
        model.set_server_metadata_for_conversation(
            conversation_id,
            server_conversation_metadata(harness, task_id, conversation_token),
            ctx,
        );
    });
}

/// Zap:上游的 `ServerAIConversationMetadata` 还带 `metadata`(`ServerMetadata`)/
/// `creator` / `permissions`(`ServerPermissions`)三个云对象字段,随云端会话同步一起
/// 剥离,helper 的 `permissions` 入参也一并去掉。
fn server_conversation_metadata(
    harness: AIAgentHarness,
    ambient_agent_task_id: AmbientAgentTaskId,
    server_conversation_token: String,
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
        ambient_agent_task_id: Some(ambient_agent_task_id),
        server_conversation_token: ServerConversationToken::new(server_conversation_token),
        artifacts: vec![],
    }
}

fn cloud_mode_terminal_for_test(app: &mut App) -> ViewHandle<TerminalView> {
    initialize_app_for_terminal_view(app);
    let tips_model = app.add_model(|_| Default::default());
    let (_, terminal) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        TerminalView::new_for_test_with_cloud_mode(tips_model, None, true, ctx)
    });
    terminal
}

#[test]
fn test_restored_ambient_view_resolves_cta_from_view_model_task_id() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::ClaudeCode,
        );

        terminal.update(&mut app, |view, ctx| {
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            {
                let model = view.model.lock();
                assert!(!model.is_shared_ambient_agent_session());
                assert!(model.conversation_transcript_viewer_status().is_none());
            }

            let state = view
                .cloud_conversation_continuation_ui_state(ctx)
                .expect("ambient view model task ID should pass the continuation gate");
            assert!(matches!(
                state,
                CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id: resolved_task_id })
                } if resolved_task_id == task_id
            ));
        });
    });
}

/// Resolves the follow-up routing for `view` using the same source of truth as the submission
/// path (`Input::ai_query_routing`) and the footer live-VM indicator.
fn query_routing(view: &TerminalView, ctx: &AppContext) -> AIQueryRouting {
    let model = view.model.lock();
    resolve_ai_query_routing(view.id(), view.ambient_agent_view_model(), &model, ctx)
}

#[test]
fn test_continue_in_cloud_tombstone_routes_third_party_followup_to_new_cloud_vm() {
    // REMOTE-2047: a third-party harness (Claude Code, etc.) run that ended surfaces a "Continue"
    // tombstone instead of an inline follow-up input. While the pane is still a finished (read-only)
    // viewer the follow-up routing is `UnconnectedReadOnly` (submission blocked with a toast).
    // Clicking Continue (`start_cloud_followup_from_tombstone`) clears the finished-viewer state and
    // enables the input, so the routing must flip to `NewCloudVm` and the follow-up starts a new
    // cloud VM via cloud-to-cloud handoff.
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::ClaudeCode,
        );

        terminal.update(&mut app, |view, ctx| {
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });
            // Simulate the live shared session ending: the pane is now a finished (read-only)
            // viewer of the ended ambient run.
            {
                let mut model = view.model.lock();
                model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));
                model.set_shared_session_status(SharedSessionStatus::FinishedViewer);
            }

            // The ended third-party run resolves to the "Continue in cloud" tombstone.
            assert!(matches!(
                view.cloud_conversation_continuation_ui_state(ctx),
                Some(CloudConversationContinuationUiState::Tombstone {
                    cta: Some(TombstoneCta::ContinueInCloud { task_id: resolved }),
                }) if resolved == task_id
            ));

            // Before clicking Continue, the finished viewer is read-only and follow-ups are blocked.
            assert_eq!(
                query_routing(view, ctx),
                AIQueryRouting::UnconnectedReadOnly
            );

            // Click "Continue" on the tombstone (the real handler for that button).
            view.start_cloud_followup_from_tombstone(task_id, ctx);

            // Continue cleared the finished-viewer state, so the pane is editable...
            assert!(matches!(
                view.model.lock().shared_session_status(),
                SharedSessionStatus::NotShared
            ));
            // ...and the follow-up now starts a new cloud VM instead of being blocked.
            assert_eq!(
                query_routing(view, ctx),
                AIQueryRouting::NewCloudVm { task_id }
            );
        });
    });
}

#[test]
fn test_restored_oz_edit_access_non_owner_finished_view_uses_followup_input_without_tombstone() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user("another-user");
        let task_id = task.task_id;

        insert_cloud_mode_task_with_server_metadata(
            &mut app,
            terminal.id(),
            task,
            AIAgentHarness::Oz,
        );

        terminal.update(&mut app, |view, ctx| {
            view.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_interaction_state(InteractionState::Selectable, ctx);
                });
            });
            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });
            let initial_block_height_items = {
                let mut model = view.model.lock();
                model.set_shared_session_status(SharedSessionStatus::FinishedViewer);
                model.block_list().block_heights().items().len()
            };

            view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);

            assert_eq!(
                view.model.lock().block_list().block_heights().items().len(),
                initial_block_height_items
            );
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            {
                let model = view.model.lock();
                assert!(matches!(
                    model.shared_session_status(),
                    SharedSessionStatus::NotShared
                ));
                assert!(view.is_input_box_visible(&model, ctx));
            }
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

#[test]
fn test_restored_owned_tombstone_hides_input_until_continue() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::NotShared);
            drop(model);

            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });
            view.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_interaction_state(InteractionState::Selectable, ctx);
                });
            });

            view.insert_conversation_ended_tombstone_with_cta(
                Some(TombstoneCta::ContinueInCloud { task_id }),
                ctx,
            );
            assert!(view.conversation_ended_tombstone_view_id.is_some());
            {
                let model = view.model.lock();
                assert!(!view.is_input_box_visible(&model, ctx));
            }

            view.start_cloud_followup_from_tombstone(task_id, ctx);
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            {
                let model = view.model.lock();
                assert!(view.is_input_box_visible(&model, ctx));
            }
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

/// REMOTE-2208: a cloud run whose environment is retained after a failure keeps a reachable
/// shared session, but the pane may already have been switched to the ended-run view
/// (`FinishedViewer` status, ended-conversation tombstone, non-editable input). Reattaching to
/// the retained session must restore a writable, interactive terminal rather than leaving the
/// user on a read-only pane with no input box.
#[test]
fn test_prepare_for_live_session_reattach_restores_interactive_input() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task_id = create_cloud_mode_task_for_user(TEST_USER_UID).task_id;

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::FinishedViewer);
            drop(model);

            view.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_interaction_state(InteractionState::Selectable, ctx);
                });
            });
            view.insert_conversation_ended_tombstone_with_cta(None, ctx);

            assert!(view.conversation_ended_tombstone_view_id.is_some());
            {
                let model = view.model.lock();
                assert!(model.is_read_only());
                assert!(!view.is_input_box_visible(&model, ctx));
            }

            view.prepare_for_live_session_reattach(ctx);

            assert!(
                view.conversation_ended_tombstone_view_id.is_none(),
                "the ended-conversation tombstone must be cleared before rejoining"
            );
            {
                let model = view.model.lock();
                assert!(!model.is_read_only());
                assert!(view.is_input_box_visible(&model, ctx));
            }
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}

/// REMOTE-2208: the user-visible symptom of the bug was that the tab opened for a retained
/// failed run accepted no typing. This asserts the behavior itself rather than the flags behind
/// it: text typed into the pane's input is dropped while it is still in the ended-run state, and
/// lands in the buffer once the pane has been prepared for the retained-session rejoin.
#[test]
fn test_prepare_for_live_session_reattach_accepts_typed_text() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task_id = create_cloud_mode_task_for_user(TEST_USER_UID).task_id;

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::FinishedViewer);
            drop(model);

            view.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_interaction_state(InteractionState::Selectable, ctx);
                });
            });
            view.insert_conversation_ended_tombstone_with_cta(None, ctx);

            // The ended-run pane swallows typing: this is exactly what the reporter saw.
            view.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.insert_selected_text("echo hello-from-retained", ctx);
                });
            });
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                "",
                "an ended-run pane must not accept typed text"
            );

            view.prepare_for_live_session_reattach(ctx);

            view.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.insert_selected_text("echo hello-from-retained", ctx);
                });
            });
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                "echo hello-from-retained",
                "a pane rejoining a retained session must accept typed text"
            );
        });
    });
}

#[test]
fn test_deep_linked_ambient_continuation_refreshes_when_task_data_arrives() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let task = create_cloud_mode_task_for_user(TEST_USER_UID);
        let task_id = task.task_id;

        terminal.update(&mut app, |view, ctx| {
            // Mirrors opening a cloud conversation directly (for example, a
            // Warp-on-Web deep link) before AgentConversationsModel has loaded
            // the ambient task. The restored pane only has the task id from
            // conversation metadata, so it first renders the conservative
            // ended-session UI.
            let mut model = view.model.lock();
            model.set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                task_id.to_string(),
            )));
            model.set_shared_session_status(SharedSessionStatus::FinishedViewer);
            drop(model);

            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            view.insert_conversation_ended_tombstone_with_resolved_cta(ctx);

            assert!(view.conversation_ended_tombstone_view_id.is_some());
            assert_eq!(view.pending_cloud_followup_task_id, None);
            {
                let model = view.model.lock();
                assert!(matches!(
                    model.shared_session_status(),
                    SharedSessionStatus::FinishedViewer
                ));
                assert!(!view.is_input_box_visible(&model, ctx));
            }
        });

        AgentConversationsModel::handle(&app).update(&mut app, |model, ctx| {
            // Once the task fetch or initial task sync finishes, the terminal
            // subscription should re-resolve the continuation state and replace
            // the conservative tombstone with owned follow-up input.
            model.insert_task_for_test(task);
            ctx.emit(AgentConversationsModelEvent::TasksUpdated);
        });

        terminal.read(&app, |view, ctx| {
            assert!(view.conversation_ended_tombstone_view_id.is_none());
            assert_eq!(view.pending_cloud_followup_task_id, Some(task_id));
            {
                let model = view.model.lock();
                assert!(matches!(
                    model.shared_session_status(),
                    SharedSessionStatus::NotShared
                ));
                assert!(view.is_input_box_visible(&model, ctx));
            }
            assert_eq!(
                view.input()
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .interaction_state(ctx),
                InteractionState::Editable
            );
        });
    });
}
#[test]
fn test_try_submit_pending_cloud_followup_rejects_task_source_that_blocks_followups() {
    let _handoff_flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
    let _setup_v2_flag = FeatureFlag::CloudModeSetupV2.override_enabled(true);

    App::test((), |mut app| async move {
        let terminal = cloud_mode_terminal_for_test(&mut app);
        let mut task = create_cloud_mode_task_for_user(TEST_USER_UID);
        task.source = Some(AgentSource::GitHubAction);
        let task_id = task.task_id;

        AgentConversationsModel::handle(&app).update(&mut app, |model, _| {
            model.insert_task_for_test(task);
        });

        terminal.update(&mut app, |view, ctx| {
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(Some(
                    task_id.to_string(),
                )));

            let ambient_agent_view_model = view
                .ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .clone();
            ambient_agent_view_model.update(ctx, |model, ctx| {
                model.enter_viewing_existing_session(task_id, ctx);
            });

            view.enable_cloud_followup_input(task_id, ctx);
            assert!(!view.try_submit_pending_cloud_followup("follow up".to_string(), ctx));
            assert_eq!(view.pending_cloud_followup_task_id, None);
            // Zap:上游还断言 ambient model 上没有留下 pending follow-up prompt。
            // 云端 follow-up 提交链路已剥离,`AmbientAgentViewModel` 不再持有该状态。
        });
    });
}
// Zap:上游在此处有 4 个 "Copy link" / "Copy session sharing link" 用例
// (APP-5027 回归)。分享链接链路(`terminal::shared_session::manager::Manager`、
// `TerminalView::copy_shared_session_link`、pane 头部与右键菜单的复制入口)已随云端
// shared session 一起剥离,这些用例连同其菜单项一并删除。
