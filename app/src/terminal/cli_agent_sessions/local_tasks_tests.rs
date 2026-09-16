use futures::executor::block_on;

use super::*;
use crate::persistence::local_cli_tasks::load_task_generations;

fn task() -> LocalCliTask {
    LocalCliTask {
        version: 1,
        task_id: "cli-task".to_owned(),
        parent_task_id: None,
        parent_generation: None,
        harness: "codex".to_owned(),
        working_directory: "/test-project".to_owned(),
        config_json: "{}".to_owned(),
        native_session_id: Some("native-session".to_owned()),
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    }
}

#[test]
fn local_cli_session_cannot_persist_unverified_success() {
    let temp = tempfile::tempdir().unwrap();
    let writer = crate::persistence::start_test_writer(&temp.path().join("tasks.sqlite")).unwrap();
    let mut task = task();
    block_on(checkpoint_task(&writer.sender, task.clone(), None).unwrap())
        .unwrap()
        .unwrap();

    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::Success,
            context: CLIAgentSessionContext::default(),
            evidence: None,
        },
    ))
    .unwrap();
    let restored = block_on(load_tasks(&writer.sender, false).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(restored[0].state, LocalCliTaskState::Unconfirmed);
    assert_eq!(restored[0].terminal_evidence, None);
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn unconfirmed_pty_stop_preserves_the_current_run_until_a_real_followup_event() {
    let temp = tempfile::tempdir().unwrap();
    let writer = crate::persistence::start_test_writer(&temp.path().join("tasks.sqlite")).unwrap();
    let mut task = task();
    block_on(checkpoint_task(&writer.sender, task.clone(), None).unwrap())
        .unwrap()
        .unwrap();
    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::InProgress,
            context: CLIAgentSessionContext::default(),
            evidence: None,
        },
    ))
    .unwrap();
    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::Unknown,
            context: CLIAgentSessionContext {
                session_id: Some("native-session".to_owned()),
                response: Some("Stop 阶段的响应，尚无真实完成结果".to_owned()),
                ..Default::default()
            },
            evidence: Some("osc777:Stop:turn-1".to_owned()),
        },
    ))
    .unwrap();
    let stored = block_on(load_tasks(&writer.sender, false).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stored[0].state, LocalCliTaskState::Unconfirmed);
    assert_eq!(stored[0].generation, 1);
    assert_eq!(stored[0].revision, 2);
    assert_eq!(
        stored[0].native_session_id.as_deref(),
        Some("native-session")
    );
    assert_eq!(
        stored[0].result.as_deref(),
        Some("Stop 阶段的响应，尚无真实完成结果")
    );
    assert_eq!(stored[0].terminal_evidence, None);

    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::Blocked { message: None },
            context: CLIAgentSessionContext::default(),
            evidence: None,
        },
    ))
    .unwrap();
    assert_eq!(task.state, LocalCliTaskState::WaitingForUser);
    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::InProgress,
            context: CLIAgentSessionContext::default(),
            evidence: None,
        },
    ))
    .unwrap();
    assert_eq!(task.state, LocalCliTaskState::Running);
    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::Failed {
                error_type: Some("native_failure".to_owned()),
                message: None,
            },
            context: CLIAgentSessionContext::default(),
            evidence: Some("osc777:StopFailure:turn-1".to_owned()),
        },
    ))
    .unwrap();
    let generations =
        block_on(load_task_generations(&writer.sender, "cli-task".to_owned()).unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(generations.len(), 1);
    assert_eq!(generations[0].state, LocalCliTaskState::Failed);
    assert_eq!(generations[0].generation, 1);
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn explicit_pty_disconnect_is_distinct_from_an_unconfirmed_result() {
    let temp = tempfile::tempdir().unwrap();
    let writer = crate::persistence::start_test_writer(&temp.path().join("tasks.sqlite")).unwrap();
    let mut task = task();
    block_on(checkpoint_task(&writer.sender, task.clone(), None).unwrap())
        .unwrap()
        .unwrap();
    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::Unknown,
            context: CLIAgentSessionContext::default(),
            evidence: None,
        },
    ))
    .unwrap();
    assert_eq!(task.state, LocalCliTaskState::Unconfirmed);
    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::Disconnected,
            context: CLIAgentSessionContext::default(),
            evidence: None,
        },
    ))
    .unwrap();
    let stored = block_on(load_tasks(&writer.sender, false).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stored[0].state, LocalCliTaskState::Disconnected);
    assert_eq!(stored[0].generation, 1);
    assert_eq!(stored[0].terminal_evidence, None);
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn local_cli_followup_preserves_previous_result_in_a_new_generation() {
    let temp = tempfile::tempdir().unwrap();
    let writer = crate::persistence::start_test_writer(&temp.path().join("tasks.sqlite")).unwrap();
    let mut task = task();
    block_on(checkpoint_task(&writer.sender, task.clone(), None).unwrap())
        .unwrap()
        .unwrap();

    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::Success,
            context: CLIAgentSessionContext {
                session_id: Some("native-session".to_owned()),
                response: Some("第一轮完成".to_owned()),
                ..Default::default()
            },
            evidence: Some("turn/completed:turn-1".to_owned()),
        },
    ))
    .unwrap();
    block_on(apply_update(
        &writer.sender,
        &mut task,
        TaskUpdate {
            status: CLIAgentSessionStatus::InProgress,
            context: CLIAgentSessionContext {
                session_id: Some("native-session".to_owned()),
                ..Default::default()
            },
            evidence: None,
        },
    ))
    .unwrap();
    let generations =
        block_on(load_task_generations(&writer.sender, "cli-task".to_owned()).unwrap())
            .unwrap()
            .unwrap();
    assert_eq!(generations.len(), 2);
    assert_eq!(generations[0].result.as_deref(), Some("第一轮完成"));
    assert_eq!(generations[0].state, LocalCliTaskState::Completed);
    assert_eq!(generations[1].generation, 2);
    assert_eq!(generations[1].state, LocalCliTaskState::Running);
    assert_eq!(generations[1].result, None);
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}
