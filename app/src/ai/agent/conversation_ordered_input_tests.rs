use chrono::{Local, TimeZone as _};
use warp_multi_agent_api as api;
use warpui::{App, EntityId, ModelContext};

use super::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::{
    AIAgentInput, AIAgentOutputStatus, CancellationReason, FinishedAIAgentOutput, RenderableAIError,
};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::ai::blocklist::{RequestInput, ResponseStreamId};
use crate::auth::AuthStateProvider;
use crate::terminal::model::BlockId;
use crate::test_util::ai_agent_tasks::create_api_task;
use crate::test_util::settings::initialize_settings_for_tests;

fn user_query(query: &str) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.into(),
        context: Default::default(),
        static_query_type: None,
        referenced_attachments: Default::default(),
        user_query_mode: Default::default(),
        running_command: None,
        intended_agent: None,
    }
}

fn with_ordered_request(
    test: impl FnOnce(
        &mut AIConversation,
        &ResponseStreamId,
        EntityId,
        &mut ModelContext<BlocklistAIHistoryModel>,
    ) + 'static,
) {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        let history = app.add_model(|_| BlocklistAIHistoryModel::new_for_test());
        history.update(&mut app, |_, ctx| {
            let mut conversation = AIConversation::new_restored(
                AIConversationId::new(),
                vec![create_api_task("root", vec![])],
                None,
            )
            .unwrap();
            // 共享查看器也会提交空批次，仍需关联和结束该任务的 exchange。
            conversation.is_viewing_shared_session = true;
            let root = conversation.get_root_task_id().clone();
            let child = conversation.create_optimistic_cli_subagent_task_for_test(&BlockId::from(
                "ordered-child".to_owned(),
            ));
            let empty_child = conversation.create_optimistic_cli_subagent_task_for_test(
                &BlockId::from("ordered-empty-child".to_owned()),
            );
            let request = RequestInput {
                conversation_id: conversation.id(),
                input_messages: vec![
                    (root.clone(), vec![user_query("首批输入")]),
                    (
                        child,
                        vec![AIAgentInput::ResumeConversation {
                            context: Default::default(),
                        }],
                    ),
                    (root, vec![user_query("末批输入")]),
                    (empty_child, vec![]),
                ],
                working_directory: None,
                model_id: "test-model".into(),
                coding_model_id: "test-model".into(),
                cli_agent_model_id: "test-model".into(),
                computer_use_model_id: "test-model".into(),
                shared_session_response_initiator: None,
                request_start_ts: Local.timestamp_opt(1_700_000_000, 0).unwrap(),
                supported_tools_override: None,
            };
            let stream_id = ResponseStreamId::new_for_test();
            let terminal_surface_id = EntityId::new();
            conversation
                .update_for_new_request_input(request, stream_id.clone(), terminal_surface_id, ctx)
                .unwrap();
            test(&mut conversation, &stream_id, terminal_surface_id, ctx);
        });
    });
}

#[test]
fn ordered_batches_group_history_once_per_task_in_first_seen_order() {
    with_ordered_request(|conversation, stream_id, _, _| {
        let exchanges = conversation
            .new_exchange_ids_for_response(stream_id)
            .map(|id| conversation.exchange_with_id(id).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(
            conversation
                .all_tasks()
                .flat_map(|task| task.exchanges())
                .count(),
            3
        );
        assert_eq!(exchanges.len(), 3);
        assert!(matches!(
            exchanges[0].input.as_slice(),
            [AIAgentInput::UserQuery { query: first, .. }, AIAgentInput::UserQuery { query: last, .. }]
                if first == "首批输入" && last == "末批输入"
        ));
        assert!(matches!(
            exchanges[1].input.as_slice(),
            [AIAgentInput::ResumeConversation { .. }]
        ));
        assert!(exchanges[2].input.is_empty());
        assert_eq!(conversation.root_task_exchanges().count(), 1);
    });
}

#[test]
fn ordered_response_initializes_and_completes_every_task_exchange() {
    with_ordered_request(|conversation, stream_id, terminal_surface_id, ctx| {
        conversation
            .initialize_output_for_response_stream(
                stream_id,
                api::response_event::StreamInit {
                    request_id: "ordered-request".into(),
                    ..Default::default()
                },
                terminal_surface_id,
                ctx,
            )
            .unwrap();
        assert_eq!(
            conversation
                .all_tasks()
                .flat_map(|task| task.exchanges())
                .filter(|exchange| exchange.output_status.output().is_some())
                .count(),
            3,
        );

        conversation
            .mark_request_completed(stream_id, terminal_surface_id, ctx)
            .unwrap();

        assert_eq!(
            conversation
                .all_tasks()
                .flat_map(|task| task.exchanges())
                .filter(|exchange| exchange.output_status.is_finished_and_successful())
                .count(),
            3,
        );
        assert_eq!(conversation.status(), &ConversationStatus::Success);
    });
}

#[test]
fn ordered_response_cancellation_finishes_every_task_before_stream_init() {
    with_ordered_request(|conversation, stream_id, terminal_surface_id, ctx| {
        conversation
            .mark_request_cancelled(
                stream_id,
                terminal_surface_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            )
            .unwrap();

        assert_eq!(
            conversation
                .all_tasks()
                .flat_map(|task| task.exchanges())
                .filter(|exchange| exchange.output_status.is_cancelled())
                .count(),
            3,
        );
        assert_eq!(conversation.status(), &ConversationStatus::Cancelled);
    });
}

#[test]
fn ordered_response_failure_finishes_every_task_before_stream_init() {
    with_ordered_request(|conversation, stream_id, terminal_surface_id, ctx| {
        conversation
            .mark_request_completed_with_error(
                stream_id,
                RenderableAIError::InternalWarpError,
                false,
                terminal_surface_id,
                ctx,
            )
            .unwrap();

        assert_eq!(
            conversation
                .all_tasks()
                .flat_map(|task| task.exchanges())
                .filter(|exchange| matches!(
                    exchange.output_status,
                    AIAgentOutputStatus::Finished {
                        finished_output: FinishedAIAgentOutput::Error { .. },
                    }
                ))
                .count(),
            3,
        );
        assert_eq!(conversation.status(), &ConversationStatus::Error);
    });
}
