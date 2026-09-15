use super::*;
use crate::ai::agent::AIAgentActionId;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use warpui::App;

fn request_input(conversation_id: AIConversationId, task_id: TaskId) -> RequestInput {
    RequestInput {
        conversation_id,
        input_messages: HashMap::from([(task_id, vec![])]),
        working_directory: None,
        model_id: LLMId::from("test-model"),
        coding_model_id: LLMId::from("test-model"),
        cli_agent_model_id: LLMId::from("test-model"),
        computer_use_model_id: LLMId::from("test-model"),
        shared_session_response_initiator: None,
        request_start_ts: Local::now(),
        supported_tools_override: None,
    }
}

#[test]
fn pending_stream_can_be_cancelled_without_history_association() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let conversation_id = AIConversationId::new();
            let stream_id = ResponseStreamId::new_local();
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id.clone(),
                    conversation_id,
                    stream,
                    ctx,
                );
                assert!(controller.has_active_stream_for_conversation(conversation_id, ctx));
                assert!(controller.cancel_request(
                    &stream_id,
                    CancellationReason::ManuallyCancelled,
                    ctx
                ));
                assert!(!controller.has_active_stream_for_conversation(conversation_id, ctx));
            });
        });
    });
}

#[test]
fn cancelling_conversation_stops_stream_after_history_was_lost() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let conversation_id = AIConversationId::new();
            let stream_id = ResponseStreamId::new_local();
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(stream_id, conversation_id, stream, ctx);
                assert!(
                    controller
                        .in_flight_response_streams
                        .try_cancel_streams_for_conversation(
                            conversation_id,
                            CancellationReason::ManuallyCancelled,
                            ctx,
                        )
                );
                assert!(!controller.has_active_stream_for_conversation(conversation_id, ctx));
            });
        });
    });
}

#[test]
fn finished_stream_is_cleaned_up_without_history_association() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let conversation_id = AIConversationId::new();
            let stream_id = ResponseStreamId::new_local();
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id.clone(),
                    conversation_id,
                    stream.clone(),
                    ctx,
                );
                assert!(controller.has_active_stream_for_conversation(conversation_id, ctx));
                controller.handle_response_stream_event(
                    false,
                    &ResponseStreamEvent::AfterStreamFinished { cancellation: None },
                    &stream,
                    ctx,
                );
                assert!(!controller.has_active_stream_for_conversation(conversation_id, ctx));
            });
        });
    });
}

#[test]
fn reassigned_stream_keeps_latest_owner_after_history_mapping_is_removed() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let original_owner = AIConversationId::new();
            let stream_id = ResponseStreamId::new_local();
            let new_owner = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let id = history.start_new_conversation(terminal.id(), false, false, false, ctx);
                let task_id = history
                    .conversation(&id)
                    .unwrap()
                    .get_root_task_id()
                    .clone();
                history
                    .update_conversation_for_new_request_input(
                        request_input(id, task_id),
                        stream_id.clone(),
                        terminal.id(),
                        ctx,
                    )
                    .unwrap();
                id
            });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id.clone(),
                    original_owner,
                    stream,
                    ctx,
                );
                assert!(!controller.has_active_stream_for_conversation(original_owner, ctx));
                assert!(controller.has_active_stream_for_conversation(new_owner, ctx));
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _ctx| {
                    history
                        .conversation_mut(&new_owner)
                        .unwrap()
                        .cleanup_completed_response_stream(&stream_id);
                });
                assert!(
                    controller
                        .in_flight_response_streams
                        .try_cancel_streams_for_conversation(
                            new_owner,
                            CancellationReason::ManuallyCancelled,
                            ctx,
                        )
                );
            });
        });
    });
}

#[test]
fn invalid_request_task_does_not_start_response_stream() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(terminal.id(), false, false, false, ctx)
                });
            {
                let mut model = terminal.model.lock();
                model.simulate_long_running_block("while true; do date; sleep 10; done", "");
                model
                    .block_list_mut()
                    .active_block_mut()
                    .set_agent_interaction_mode_for_requested_command(
                        AIAgentActionId::from("unregistered-poll-command".to_owned()),
                        None,
                        conversation_id,
                    );
            }
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let resume = ctx.spawn(futures::future::pending::<()>(), |_, _, _| {
                    panic!("历史写入失败后不应自动恢复会话");
                });
                controller
                    .pending_auto_resume_handles
                    .insert(conversation_id, resume);
                let result = controller.send_request_input(
                    request_input(conversation_id, TaskId::new("missing-task".to_owned())),
                    None,
                    false,
                    false,
                    false,
                    ctx,
                );
                let error = result.unwrap_err();
                // 明确覆盖写入 exchange 的失败，不能被更早的输入校验错误替代。
                assert!(error.downcast_ref::<UpdateHistoryError>().is_some());
                assert!(!controller.has_active_stream_for_conversation(conversation_id, ctx));
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&conversation_id)
                        .unwrap()
                        .status(),
                    &ConversationStatus::Error,
                );
                assert!(
                    !controller
                        .pending_auto_resume_handles
                        .contains_key(&conversation_id)
                );
                let model = controller.terminal_model.lock();
                let block = model.block_list().active_block();
                assert!(!block.is_agent_in_control());
                assert!(!block.is_eligible_for_agent_handoff());
            });
        });
    });
}

#[test]
fn agent_started_long_command_follow_up_initializes_optimistic_cli_task() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let block_id = {
                let mut model = terminal.model.lock();
                model.simulate_long_running_block("while true; do date; sleep 10; done", "");
                model.block_list().active_block().id().clone()
            };
            let (conversation_id, task_id) =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let id =
                        history.start_new_conversation(terminal.id(), false, false, false, ctx);
                    let task_id = history
                        .conversation_mut(&id)
                        .unwrap()
                        .create_optimistic_cli_subagent_task_for_test(&block_id);
                    (id, task_id)
                });
            terminal
                .model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_requested_command(
                    AIAgentActionId::from("poll-command".to_owned()),
                    None,
                    conversation_id,
                );
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let mut params = api::RequestParams::new_for_test(vec![], vec![]);
                params.byop_target_task_id = Some(task_id.to_string());
                controller.populate_lrc_request_params(&mut params, conversation_id, ctx);
                assert_eq!(params.lrc_command_id, Some(block_id.to_string()));
                assert!(params.lrc_should_spawn_subagent);
                assert_eq!(params.byop_target_task_id, Some(task_id.to_string()));
            });
        });
    });
}

fn invalid_task_event() -> ResponseStreamEvent {
    ResponseStreamEvent::ReceivedEvent(response_stream::Consumable::new(Ok(
        warp_multi_agent_api::ResponseEvent {
            r#type: Some(warp_multi_agent_api::response_event::Type::ClientActions(
                warp_multi_agent_api::response_event::ClientActions {
                    actions: vec![ClientAction {
                        action: Some(Action::AddMessagesToTask(
                            warp_multi_agent_api::client_action::AddMessagesToTask {
                                task_id: "missing-cli-task".to_owned(),
                                messages: vec![],
                            },
                        )),
                    }],
                },
            )),
        },
    )))
}

#[test]
fn invalid_client_action_fails_only_its_conversation_and_blocks_late_events() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let stream_id = ResponseStreamId::new_local();
            let healthy_stream_id = ResponseStreamId::new_local();
            let (conversation_id, healthy_conversation_id) = BlocklistAIHistoryModel::handle(ctx)
                .update(ctx, |history, ctx| {
                    let mut ids = Vec::new();
                    for id in [&stream_id, &healthy_stream_id] {
                        let conversation_id =
                            history.start_new_conversation(terminal.id(), false, false, false, ctx);
                        let task_id = history
                            .conversation(&conversation_id)
                            .unwrap()
                            .get_root_task_id()
                            .clone();
                        history
                            .update_conversation_for_new_request_input(
                                request_input(conversation_id, task_id),
                                id.clone(),
                                terminal.id(),
                                ctx,
                            )
                            .unwrap();
                        history.update_conversation_status(
                            terminal.id(),
                            conversation_id,
                            ConversationStatus::InProgress,
                            ctx,
                        );
                        ids.push(conversation_id);
                    }
                    (ids[0], ids[1])
                });
            {
                let mut model = terminal.model.lock();
                model.simulate_long_running_block("while true; do date; sleep 10; done", "");
                model
                    .block_list_mut()
                    .active_block_mut()
                    .set_agent_interaction_mode_for_requested_command(
                        AIAgentActionId::from("failed-poll-command".to_owned()),
                        None,
                        conversation_id,
                    );
            }
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            let healthy_stream =
                ctx.add_model(|_| ResponseStream::new_for_test(healthy_stream_id.clone()));
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id.clone(),
                    conversation_id,
                    stream.clone(),
                    ctx,
                );
                controller.register_mock_stream_for_test(
                    healthy_stream_id,
                    healthy_conversation_id,
                    healthy_stream,
                    ctx,
                );
                let resume = ctx.spawn(futures::future::pending::<()>(), |_, _, _| {
                    panic!("失败后不应自动恢复会话");
                });
                controller
                    .pending_auto_resume_handles
                    .insert(conversation_id, resume);
                controller.handle_response_stream_event(false, &invalid_task_event(), &stream, ctx);
                assert!(!controller.has_active_stream_for_conversation(conversation_id, ctx));
                assert!(
                    controller.has_active_stream_for_conversation(healthy_conversation_id, ctx)
                );
                assert!(
                    !controller
                        .pending_auto_resume_handles
                        .contains_key(&conversation_id)
                );
                assert!(
                    !stream
                        .as_ref(ctx)
                        .should_resume_conversation_after_stream_finished()
                );
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&conversation_id)
                        .unwrap()
                        .status(),
                    &ConversationStatus::Error
                );
                assert!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&healthy_conversation_id)
                        .unwrap()
                        .status()
                        .is_in_progress()
                );
                assert!(
                    !controller
                        .terminal_model
                        .lock()
                        .block_list()
                        .active_block()
                        .is_eligible_for_agent_handoff()
                );
                controller.handle_response_stream_event(false, &invalid_task_event(), &stream, ctx);
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&conversation_id)
                        .unwrap()
                        .status(),
                    &ConversationStatus::Error
                );
                assert!(
                    controller.has_active_stream_for_conversation(healthy_conversation_id, ctx)
                );
            });
        });
    });
}

#[test]
fn received_event_without_history_mapping_cancels_registered_stream() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(terminal.id(), false, false, false, ctx)
                });
            let stream_id = ResponseStreamId::new_local();
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id,
                    conversation_id,
                    stream.clone(),
                    ctx,
                );
                controller.handle_response_stream_event(false, &invalid_task_event(), &stream, ctx);
                assert!(!controller.has_active_stream_for_conversation(conversation_id, ctx));
                assert_eq!(
                    BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(&conversation_id)
                        .unwrap()
                        .status(),
                    &ConversationStatus::Error
                );
            });
        });
    });
}
