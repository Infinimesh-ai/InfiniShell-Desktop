use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use warpui::App;

use super::*;
use crate::ai::agent::AIAgentActionResultType;
use crate::ai::agent::task::TaskId;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn make_action_result(id: &str) -> Arc<AIAgentActionResult> {
    Arc::new(AIAgentActionResult {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task".to_owned()),
        result: AIAgentActionResultType::InitProject,
    })
}

fn count_startable_actions_for_pass(phases: &[(RunningActionPhase, bool)]) -> usize {
    let mut current_phase = None;
    let mut count = 0;

    for (phase, can_autoexecute) in phases {
        if let Some(current_phase) = current_phase
            && !can_start_action_with_current_phase(current_phase, *phase, *can_autoexecute)
        {
            break;
        }

        count += 1;
        current_phase = Some(*phase);

        if matches!(*phase, RunningActionPhase::Serial) {
            break;
        }
    }

    count
}

#[test]
fn parallel_phase_only_admits_matching_autoexecutable_actions() {
    let phase =
        RunningActionPhase::Parallel(execute::ParallelExecutionPolicy::ReadOnlyLocalContext);

    assert!(can_start_action_with_current_phase(phase, phase, true));
    assert!(!can_start_action_with_current_phase(phase, phase, false));
    assert!(!can_start_action_with_current_phase(
        phase,
        RunningActionPhase::Serial,
        true
    ));
    assert!(!can_start_action_with_current_phase(
        RunningActionPhase::Serial,
        phase,
        true
    ));
}

#[test]
fn phased_scheduling_stops_at_serial_barrier_and_resumes_afterward() {
    let read_only_phase =
        RunningActionPhase::Parallel(execute::ParallelExecutionPolicy::ReadOnlyLocalContext);
    let actions = vec![
        (read_only_phase, true),
        (read_only_phase, true),
        (RunningActionPhase::Serial, true),
        (read_only_phase, true),
        (read_only_phase, true),
    ];

    assert_eq!(count_startable_actions_for_pass(&actions), 2);
    assert_eq!(count_startable_actions_for_pass(&actions[2..]), 1);
    assert_eq!(count_startable_actions_for_pass(&actions[3..]), 2);
}

#[test]
fn finished_results_stay_in_original_action_order() {
    let action_order = HashMap::from([
        (AIAgentActionId::from("first".to_owned()), 0),
        (AIAgentActionId::from("second".to_owned()), 1),
        (AIAgentActionId::from("third".to_owned()), 2),
    ]);
    let mut finished_results = [
        make_action_result("third"),
        make_action_result("first"),
        make_action_result("second"),
    ];

    finished_results
        .sort_by_key(|result| action_order.get(&result.id).copied().unwrap_or(usize::MAX));

    assert_eq!(
        finished_results[0].id,
        AIAgentActionId::from("first".to_owned())
    );
    assert_eq!(
        finished_results[1].id,
        AIAgentActionId::from("second".to_owned())
    );
    assert_eq!(
        finished_results[2].id,
        AIAgentActionId::from("third".to_owned())
    );
}

#[test]
fn cancellation_finishes_preprocessing_and_late_results_leave_new_batches_untouched() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |terminal, ctx| {
            let action_model = ctx.add_model(|ctx| {
                BlocklistAIActionModel::new(
                    terminal.model.clone(),
                    terminal.active_session().clone(),
                    terminal.model_event_dispatcher(),
                    terminal.id(),
                    ctx,
                )
            });
            action_model.update(ctx, |model, ctx| {
                let conversation_id = AIConversationId::new();
                let healthy_conversation_id = AIConversationId::new();
                let action = AIAgentAction {
                    id: AIAgentActionId::from("late-command".to_owned()),
                    task_id: TaskId::new("task".to_owned()),
                    action: AIAgentActionType::RequestCommandOutput {
                        command: "true".to_owned(),
                        is_read_only: Some(true),
                        is_risky: Some(false),
                        rationale: None,
                        uses_pager: Some(false),
                        wait_until_completion: false,
                        citations: vec![],
                    },
                    requires_result: true,
                };
                let old_batch = model
                    .pending_preprocessed_actions
                    .entry(conversation_id)
                    .or_default()
                    .insert_preprocess_action_batch(vec![action.clone()]);
                let healthy_action = AIAgentActionId::from("healthy-command".to_owned());
                model
                    .pending_preprocessed_actions
                    .entry(healthy_conversation_id)
                    .or_default()
                    .insert_preprocess_action_batch(vec![AIAgentAction {
                        id: healthy_action.clone(),
                        ..action.clone()
                    }]);
                assert!(matches!(
                    model.get_action_status(&action.id),
                    Some(AIActionStatus::Preprocessing)
                ));
                model.cancel_all_pending_actions(
                    conversation_id,
                    Some(CancellationReason::ManuallyCancelled),
                    ctx,
                );
                assert!(model.get_action_status(&action.id).unwrap().is_cancelled());
                assert!(matches!(
                    model.get_action_status(&healthy_action),
                    Some(AIActionStatus::Preprocessing)
                ));

                model.handle_preprocess_actions_results(
                    conversation_id,
                    old_batch.clone(),
                    vec![action.clone()],
                    true,
                    ctx,
                );
                assert!(model.get_action_status(&action.id).unwrap().is_cancelled());
                assert!(!model.has_unfinished_actions_for_conversation(conversation_id));

                let new_action = AIAgentActionId::from("new-command".to_owned());
                let new_batch = model
                    .pending_preprocessed_actions
                    .entry(conversation_id)
                    .or_default()
                    .insert_preprocess_action_batch(vec![AIAgentAction {
                        id: new_action.clone(),
                        ..action.clone()
                    }]);
                // ID 由 UUID 生成，清空旧队列后也不能让旧回调匹配到新批次。
                assert_ne!(old_batch, new_batch);
                model.handle_preprocess_actions_results(
                    conversation_id,
                    old_batch,
                    vec![action.clone()],
                    true,
                    ctx,
                );
                assert!(model.get_action_status(&action.id).unwrap().is_cancelled());
                assert!(matches!(
                    model.get_action_status(&new_action),
                    Some(AIActionStatus::Preprocessing)
                ));
                assert!(matches!(
                    model.get_action_status(&healthy_action),
                    Some(AIActionStatus::Preprocessing)
                ));
                let results = model.drain_finished_action_results(conversation_id);
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].id, action.id);
                assert!(results[0].result.is_cancelled());
            });
        });
    });
}

#[test]
fn cancelling_preprocessing_immediately_finishes_actions() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let action_model = terminal.update(&mut app, |terminal, ctx| {
            ctx.add_model(|ctx| {
                BlocklistAIActionModel::new(
                    terminal.model.clone(),
                    terminal.active_session().clone(),
                    terminal.model_event_dispatcher(),
                    terminal.id(),
                    ctx,
                )
            })
        });
        let finished_events = Rc::new(RefCell::new(Vec::new()));
        let recorded_events = finished_events.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&action_model, move |_, event, _| {
                if let BlocklistAIActionEvent::FinishedAction {
                    action_id,
                    cancellation_reason,
                    ..
                } = event
                {
                    recorded_events
                        .borrow_mut()
                        .push((action_id.clone(), *cancellation_reason));
                }
            });
        });
        let conversation_id = AIConversationId::new();
        let action_id = AIAgentActionId::from("cancel-preprocessing-edit".to_owned());
        action_model.update(&mut app, |model, ctx| {
            model.queue_actions(
                vec![AIAgentAction {
                    id: action_id.clone(),
                    task_id: TaskId::new("task".to_owned()),
                    action: AIAgentActionType::RequestFileEdits {
                        file_edits: vec![],
                        title: None,
                    },
                    requires_result: true,
                }],
                conversation_id,
                ctx,
            );
            assert!(matches!(
                model.get_action_status(&action_id),
                Some(AIActionStatus::Preprocessing)
            ));

            // 不等待异步预处理返回；追问发送方会立即读取这些取消结果。
            model.cancel_all_pending_actions(
                conversation_id,
                Some(CancellationReason::FollowUpSubmitted {
                    is_for_same_conversation: true,
                }),
                ctx,
            );
            assert!(model.get_action_status(&action_id).unwrap().is_cancelled());
            let results = model.drain_finished_action_results(conversation_id);
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, action_id);
            assert!(matches!(
                results[0].result,
                AIAgentActionResultType::RequestFileEdits(
                    crate::ai::agent::RequestFileEditsResult::Cancelled
                )
            ));
            assert!(!model.has_unfinished_actions_for_conversation(conversation_id));
        });
        assert_eq!(
            *finished_events.borrow(),
            vec![(
                action_id,
                Some(CancellationReason::FollowUpSubmitted {
                    is_for_same_conversation: true,
                }),
            )]
        );
    });
}
