use super::*;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType, GrepResult};
use crate::ai::execution_profiles::ActionPermission;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use warpui::App;

fn completed_result(task_id: &TaskId) -> AIAgentActionResult {
    AIAgentActionResult {
        id: AIAgentActionId::from("completed-before-stop".to_owned()),
        task_id: task_id.clone(),
        result: AIAgentActionResultType::Grep(GrepResult::Success {
            matched_files: vec![],
        }),
    }
}

fn pending_command(task_id: &TaskId) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from("pending-at-stop".to_owned()),
        task_id: task_id.clone(),
        action: AIAgentActionType::RequestCommandOutput {
            command: "printf pending".to_owned(),
            is_read_only: Some(false),
            is_risky: Some(false),
            wait_until_completion: false,
            uses_pager: Some(false),
            rationale: None,
            citations: vec![],
        },
        requires_result: true,
    }
}

#[test]
fn stop_with_completed_and_preprocessing_actions_does_not_send_follow_up() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let (conversation_id, action_model) = terminal.update(&mut app, |terminal, ctx| {
            let (conversation_id, task_id) =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let id =
                        history.start_new_conversation(terminal.id(), false, false, false, ctx);
                    (
                        id,
                        history
                            .conversation(&id)
                            .unwrap()
                            .get_root_task_id()
                            .clone(),
                    )
                });
            let action_model = terminal.ai_controller().as_ref(ctx).action_model.clone();
            action_model.update(ctx, |actions, ctx| {
                actions.queue_actions(vec![pending_command(&task_id)], conversation_id, ctx);
                actions.apply_finished_action_result(
                    conversation_id,
                    completed_result(&task_id),
                    ctx,
                );
            });
            terminal.stop_local_agent_conversation(conversation_id, ctx);
            (conversation_id, action_model)
        });

        terminal.read(&app, |terminal, ctx| {
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .unwrap()
                    .status(),
                &ConversationStatus::Cancelled
            );
            assert!(
                !terminal
                    .ai_controller()
                    .as_ref(ctx)
                    .has_active_stream_for_conversation(conversation_id, ctx)
            );
            let results = action_model
                .as_ref(ctx)
                .get_finished_action_results(conversation_id)
                .unwrap();
            assert_eq!(
                results.len(),
                2,
                "停止后的结果应保留给用户主动继续，不能被自动请求消耗"
            );
            assert!(results.iter().any(|result| result.result.is_successful()));
            assert!(results.iter().any(|result| result.result.is_cancelled()));
        });
    });
}

#[test]
fn late_action_results_preserve_cancelled_and_error_conversations() {
    for status in [ConversationStatus::Cancelled, ConversationStatus::Error] {
        App::test((), |mut app| async move {
            initialize_app_for_terminal_view(&mut app);
            let terminal = add_window_with_terminal(&mut app, None);
            let (conversation_id, task_id, action_model) =
                terminal.update(&mut app, |terminal, ctx| {
                    let (id, task_id) =
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                            let id = history.start_new_conversation(
                                terminal.id(),
                                false,
                                false,
                                false,
                                ctx,
                            );
                            history.update_conversation_status(
                                terminal.id(),
                                id,
                                status.clone(),
                                ctx,
                            );
                            (
                                id,
                                history
                                    .conversation(&id)
                                    .unwrap()
                                    .get_root_task_id()
                                    .clone(),
                            )
                        });
                    (
                        id,
                        task_id,
                        terminal.ai_controller().as_ref(ctx).action_model.clone(),
                    )
                });
            action_model.update(&mut app, |actions, ctx| {
                // 模拟任务已经结束后才到达的成功回调，仍走实际结果收尾及 controller 订阅。
                actions.apply_finished_action_result(
                    conversation_id,
                    completed_result(&task_id),
                    ctx,
                );
                actions.queue_actions(vec![pending_command(&task_id)], conversation_id, ctx);
                // 取消回调同样走 handle_action_result，混合结果不能把终态改回 InProgress。
                actions.cancel_all_pending_actions(
                    conversation_id,
                    Some(CancellationReason::ManuallyCancelled),
                    ctx,
                );
            });
            terminal.read(&app, |terminal, ctx| {
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&conversation_id)
                        .unwrap()
                        .status(),
                    &status
                );
                assert!(
                    !terminal
                        .ai_controller()
                        .as_ref(ctx)
                        .has_active_stream_for_conversation(conversation_id, ctx)
                );
                assert_eq!(
                    action_model
                        .as_ref(ctx)
                        .get_finished_action_results(conversation_id)
                        .unwrap()
                        .len(),
                    2
                );
            });
        });
    }
}

#[test]
fn cancelling_only_one_action_keeps_normal_follow_up() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let (queued_tx, queued_rx) = oneshot::channel();
        let (conversation_id, task_id, action_model) =
            terminal.update(&mut app, |terminal, ctx| {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles, ctx| {
                    profiles.set_execute_commands(
                        profiles.active_profile(Some(terminal.id()), ctx).id(),
                        &ActionPermission::AlwaysAsk,
                        ctx,
                    );
                });
                let (id, task_id) =
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        let id =
                            history.start_new_conversation(terminal.id(), false, false, false, ctx);
                        (
                            id,
                            history
                                .conversation(&id)
                                .unwrap()
                                .get_root_task_id()
                                .clone(),
                        )
                    });
                let action_model = terminal.ai_controller().as_ref(ctx).action_model.clone();
                let mut queued_tx = Some(queued_tx);
                ctx.subscribe_to_model(&action_model, move |_, _, event, _| {
                    if matches!(event, BlocklistAIActionEvent::QueuedAction(_))
                        && let Some(tx) = queued_tx.take()
                    {
                        let _ = tx.send(());
                    }
                });
                action_model.update(ctx, |actions, ctx| {
                    actions.queue_actions(vec![pending_command(&task_id)], id, ctx);
                });
                (id, task_id, action_model)
            });
        queued_rx.await.unwrap();
        action_model.update(&mut app, |actions, ctx| {
            actions.apply_finished_action_result(conversation_id, completed_result(&task_id), ctx);
            actions.cancel_action_with_id(
                conversation_id,
                &pending_command(&task_id).id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });
        terminal.read(&app, |terminal, ctx| {
            assert!(
                terminal
                    .ai_controller()
                    .as_ref(ctx)
                    .has_active_stream_for_conversation(conversation_id, ctx),
                "只取消单个动作仍应把其他已完成结果交给模型"
            );
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .unwrap()
                    .status()
                    .is_in_progress()
            );
        });
    });
}

#[test]
fn whole_conversation_stop_discards_deferred_follow_ups() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let (conversation_id, task_id) =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let id =
                        history.start_new_conversation(terminal.id(), false, false, false, ctx);
                    (
                        id,
                        history
                            .conversation(&id)
                            .unwrap()
                            .get_root_task_id()
                            .clone(),
                    )
                });
            let other_conversation_id = AIConversationId::new();
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller
                    .pending_passive_follow_ups
                    .extend([conversation_id, other_conversation_id]);
                for id in [conversation_id, other_conversation_id] {
                    controller.pending_byop_requests.insert(
                        id,
                        PendingByopRequest {
                            request_input: RequestInput::for_task(
                                vec![],
                                task_id.clone(),
                                &controller.active_session,
                                None,
                                id,
                                terminal.id(),
                                ctx,
                            ),
                            query_metadata: None,
                            default_to_follow_up_on_success: false,
                            can_attempt_resume_on_error: true,
                            is_queued_prompt: false,
                            allow_auto_compaction: true,
                            recovery_id: None,
                            context_snapshot: None,
                        },
                    );
                }
            });
            terminal.stop_local_agent_conversation(conversation_id, ctx);
            terminal.ai_controller().read(ctx, |controller, _| {
                assert!(
                    !controller
                        .pending_byop_requests
                        .contains_key(&conversation_id)
                );
                assert!(
                    !controller
                        .pending_passive_follow_ups
                        .contains(&conversation_id)
                );
                assert!(
                    controller
                        .pending_byop_requests
                        .contains_key(&other_conversation_id)
                );
                assert!(
                    controller
                        .pending_passive_follow_ups
                        .contains(&other_conversation_id)
                );
            });
        });
    });
}

#[test]
fn cancelling_progress_finalizes_before_mixed_action_callbacks() {
    for reason in [
        CancellationReason::ManuallyCancelled,
        CancellationReason::FollowUpSubmitted {
            is_for_same_conversation: false,
        },
    ] {
        App::test((), |mut app| async move {
            initialize_app_for_terminal_view(&mut app);
            let terminal = add_window_with_terminal(&mut app, None);
            let (conversation_id, action_model) = terminal.update(&mut app, |terminal, ctx| {
                let (id, task_id) =
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        let id =
                            history.start_new_conversation(terminal.id(), false, false, false, ctx);
                        (
                            id,
                            history
                                .conversation(&id)
                                .unwrap()
                                .get_root_task_id()
                                .clone(),
                        )
                    });
                let action_model = terminal.ai_controller().as_ref(ctx).action_model.clone();
                action_model.update(ctx, |actions, ctx| {
                    actions.queue_actions(vec![pending_command(&task_id)], id, ctx);
                    actions.apply_finished_action_result(id, completed_result(&task_id), ctx);
                });
                // 覆盖直接的 controller 入口，不依赖某个停止按钮事后补写终态。
                terminal.ai_controller().update(ctx, |controller, ctx| {
                    controller.cancel_conversation_progress(id, reason, ctx);
                    assert_eq!(
                        BlocklistAIHistoryModel::as_ref(ctx)
                            .conversation(&id)
                            .unwrap()
                            .status(),
                        &ConversationStatus::Cancelled
                    );
                });
                (id, action_model)
            });
            terminal.read(&app, |terminal, ctx| {
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&conversation_id)
                        .unwrap()
                        .status(),
                    &ConversationStatus::Cancelled
                );
                assert!(
                    !terminal
                        .ai_controller()
                        .as_ref(ctx)
                        .has_active_stream_for_conversation(conversation_id, ctx)
                );
                assert_eq!(
                    action_model
                        .as_ref(ctx)
                        .get_finished_action_results(conversation_id)
                        .unwrap()
                        .len(),
                    2
                );
            });
        });
    }
}
