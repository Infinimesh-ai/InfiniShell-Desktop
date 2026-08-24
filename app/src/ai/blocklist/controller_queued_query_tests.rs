use warpui::{App, SingletonEntity};

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentActionId, AIAgentActionResult, AIAgentActionResultType, GrepResult};
use crate::ai::blocklist::{QueuedQuery, QueuedQueryModel, QueuedQueryOrigin};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

#[test]
fn finished_action_unlocks_pending_lrc_queue_rows() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let (conversation_id, action_model) = terminal.update(&mut app, |terminal, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(terminal.id(), false, false, false, ctx)
                });
            QueuedQueryModel::handle(ctx).update(ctx, |queue_model, ctx| {
                queue_model.append(
                    conversation_id,
                    QueuedQuery::new(
                        "queued prompt".to_owned(),
                        QueuedQueryOrigin::PendingLrcAutoQueue,
                    ),
                    ctx,
                );
            });
            let action_model = terminal.ai_controller().as_ref(ctx).action_model.clone();
            (conversation_id, action_model)
        });

        action_model.update(&mut app, |action_model, ctx| {
            action_model.apply_finished_action_result(
                conversation_id,
                AIAgentActionResult {
                    id: AIAgentActionId::from("finished-action".to_owned()),
                    task_id: TaskId::new("task".to_owned()),
                    result: AIAgentActionResultType::Grep(GrepResult::Cancelled),
                },
                ctx,
            );
        });

        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert_eq!(
                queue_model.queue(conversation_id)[0].origin(),
                QueuedQueryOrigin::LrcAutoQueue
            );
            assert!(queue_model.peek_autofire(conversation_id).is_some());
        });
    });
}

#[test]
fn cancelling_conversation_removes_pending_lrc_queue_rows() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let conversation_id = terminal.update(&mut app, |terminal, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(terminal.id(), false, false, false, ctx)
                });
            QueuedQueryModel::handle(ctx).update(ctx, |queue_model, ctx| {
                queue_model.append(
                    conversation_id,
                    QueuedQuery::new(
                        "queued prompt".to_owned(),
                        QueuedQueryOrigin::PendingLrcAutoQueue,
                    ),
                    ctx,
                );
            });
            conversation_id
        });

        terminal.update(&mut app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.cancel_conversation_progress(
                    conversation_id,
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
            });
        });

        QueuedQueryModel::handle(&app).read(&app, |queue_model, _| {
            assert!(queue_model.queue(conversation_id).is_empty());
        });
    });
}
