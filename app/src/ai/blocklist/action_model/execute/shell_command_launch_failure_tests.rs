use async_channel::unbounded;
use futures::future::{pending, ready};
use warpui::App;

use super::*;
use crate::terminal::model::session::Sessions;

#[test]
fn rejected_launch_wins_over_cleanup_cancellation_when_both_are_ready() {
    let (sender, receiver) = oneshot::channel();
    sender.send("updating".to_owned()).unwrap();
    let result = wait_for_requested_command(ready(ActionResult::Cancelled), receiver)
        .now_or_never()
        .expect("失败应立即结束等待");
    assert!(matches!(result, Err(reason) if reason == "updating"));
}

#[test]
fn rejected_launch_does_not_wait_for_a_block_that_will_never_exist() {
    let (sender, receiver) = oneshot::channel();
    sender.send("updating".to_owned()).unwrap();
    let result = wait_for_requested_command(pending::<ActionResult>(), receiver)
        .now_or_never()
        .expect("拒绝派发不能继续等待 block");
    assert!(result.is_err());
}

#[test]
fn dropping_launch_sender_preserves_explicit_user_cancellation() {
    let (sender, receiver) = oneshot::channel::<String>();
    drop(sender);
    let result = wait_for_requested_command(ready(ActionResult::Cancelled), receiver)
        .now_or_never()
        .expect("原取消结果应可返回");
    assert!(matches!(result, Ok(ActionResult::Cancelled)));
}

#[test]
fn reject_launch_cleans_only_matching_action_and_ignores_late_duplicates() {
    App::test((), |mut app| async move {
        let sessions = app.add_model(|_| Sessions::new_for_test());
        let (_events_tx, events_rx) = unbounded();
        let dispatcher =
            app.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));
        let active_session =
            app.add_model(|ctx| ActiveSession::new(sessions, dispatcher.clone(), ctx));
        let terminal = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
        let executor = app.add_model(|ctx| {
            ShellCommandExecutor::new(active_session, terminal, &dispatcher, EntityId::new(), ctx)
        });
        executor.update(&mut app, |executor, _| {
            let first = AIAgentActionId::from("first".to_owned());
            let second = AIAgentActionId::from("second".to_owned());
            let (first_tx, first_rx) = oneshot::channel();
            let (second_tx, mut second_rx) = oneshot::channel();
            executor
                .launch_failure_senders
                .insert(first.clone(), first_tx);
            executor
                .launch_failure_senders
                .insert(second.clone(), second_tx);
            let first_completion = executor.action_result_future(
                BlockSelector::RequestedCommandId(first.clone()),
                ActionResultDelay::UntilCompletion,
            );
            let _second_completion = executor.action_result_future(
                BlockSelector::RequestedCommandId(second.clone()),
                ActionResultDelay::UntilCompletion,
            );
            executor.reject_launch(&first, "updating".to_owned());
            executor.reject_launch(&first, "late duplicate".to_owned());
            let result = wait_for_requested_command(first_completion, first_rx)
                .now_or_never()
                .unwrap();
            assert!(matches!(result, Err(reason) if reason == "updating"));
            assert!(
                !executor
                    .block_finished_senders
                    .contains_key(&BlockSelector::RequestedCommandId(first.clone()))
            );
            assert!(
                !executor
                    .force_refresh_senders
                    .contains_key(&BlockSelector::RequestedCommandId(first))
            );
            assert!(
                executor
                    .block_finished_senders
                    .contains_key(&BlockSelector::RequestedCommandId(second.clone()))
            );
            assert!(executor.launch_failure_senders.contains_key(&second));
            assert_eq!(second_rx.try_recv().unwrap(), None);
        });
    });
}
