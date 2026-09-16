use diesel::connection::SimpleConnection;
use diesel_migrations::MigrationHarness;
use futures::executor::block_on;
use serde_json::json;

use super::*;

fn connection() -> SqliteConnection {
    let mut connection = SqliteConnection::establish(":memory:").unwrap();
    connection
        .run_pending_migrations(::persistence::MIGRATIONS)
        .unwrap();
    connection
        .batch_execute("PRAGMA foreign_keys = ON")
        .unwrap();
    connection
}

fn task(task_id: &str, parent_task_id: Option<&str>) -> LocalCliTask {
    LocalCliTask {
        version: 1,
        task_id: task_id.to_owned(),
        parent_task_id: parent_task_id.map(str::to_owned),
        parent_generation: parent_task_id.map(|_| 1),
        harness: "claude".to_owned(),
        working_directory: "/project".to_owned(),
        config_json: "{}".to_owned(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    }
}

fn message() -> LocalCliMessage {
    LocalCliMessage {
        version: 1,
        message_id: "message-1".to_owned(),
        sender_task_id: "parent".to_owned(),
        recipient_task_id: "child".to_owned(),
        sender_generation: 1,
        recipient_generation: 1,
        subject: "追加指令".to_owned(),
        body: "先检查修改，再运行测试。".to_owned(),
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    }
}

fn parent_and_child(connection: &mut SqliteConnection) {
    checkpoint(connection, task("parent", None), None).unwrap();
    checkpoint(connection, task("child", Some("parent")), None).unwrap();
}

#[test]
fn local_cli_task_message_history_is_bidirectional_and_keeps_inbox_scope_unchanged() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    let first = message();
    insert_message(&mut connection, first.clone()).unwrap();
    let mut second = first.clone();
    second.message_id = "message-2".into();
    second.sender_task_id = "child".into();
    second.recipient_task_id = "parent".into();
    insert_message(&mut connection, second.clone()).unwrap();
    let mut self_message = second.clone();
    self_message.message_id = "message-3".into();
    self_message.sender_task_id = "parent".into();
    insert_message(&mut connection, self_message.clone()).unwrap();
    checkpoint(&mut connection, task("unrelated", None), None).unwrap();
    let mut unrelated = self_message.clone();
    unrelated.message_id = "unrelated-message".into();
    unrelated.sender_task_id = "unrelated".into();
    unrelated.recipient_task_id = "unrelated".into();
    insert_message(&mut connection, unrelated).unwrap();
    assert_eq!(
        read_task_messages(&mut connection, "parent").unwrap(),
        [first.clone(), second.clone(), self_message.clone()]
    );
    assert_eq!(read_messages(&mut connection, "child", 1).unwrap(), [first]);
    assert_eq!(
        read_messages(&mut connection, "parent", 1).unwrap(),
        [second, self_message]
    );
    assert!(
        read_messages(&mut connection, "parent", 2)
            .unwrap()
            .is_empty()
    );
    assert!(
        read_task_messages(&mut connection, "missing")
            .unwrap()
            .is_empty()
    );
    let mut parent = task("parent", None);
    parent.state = LocalCliTaskState::Disconnected;
    parent.revision = 1;
    checkpoint(&mut connection, parent.clone(), Some(1)).unwrap();
    parent.generation = 2;
    parent.revision = 0;
    parent.state = LocalCliTaskState::Queued;
    checkpoint(&mut connection, parent, Some(1)).unwrap();
    let mut next = message();
    next.message_id = "message-4".into();
    next.recipient_task_id = "parent".into();
    next.sender_generation = 2;
    next.recipient_generation = 2;
    insert_message(&mut connection, next.clone()).unwrap();
    let history = read_task_messages(&mut connection, "parent").unwrap();
    assert_eq!(
        history
            .iter()
            .map(|message| message.message_id.as_str())
            .collect::<Vec<_>>(),
        ["message-1", "message-2", "message-3", "message-4"]
    );
    assert!(
        history[..3]
            .iter()
            .all(|message| message.state == LocalCliMessageState::Cancelled
                && message.receipt_kind.is_none())
    );
    assert_eq!(read_messages(&mut connection, "parent", 2).unwrap(), [next]);
}

#[test]
fn local_cli_task_rejects_duplicate_launch_and_stale_revision() {
    let mut connection = connection();
    let initial = task("task", None);
    checkpoint(&mut connection, initial.clone(), None).unwrap();

    assert!(checkpoint(&mut connection, initial.clone(), None).is_err());
    let mut running = initial.clone();
    running.state = LocalCliTaskState::Running;
    running.revision = 1;
    checkpoint(&mut connection, running.clone(), Some(1)).unwrap();
    assert!(checkpoint(&mut connection, initial, Some(1)).is_err());
    assert_eq!(read_task(&mut connection, "task").unwrap(), Some(running));
}

#[test]
fn local_cli_task_requires_true_completion_evidence() {
    let mut connection = connection();
    let initial = task("task", None);
    checkpoint(&mut connection, initial.clone(), None).unwrap();
    let mut completed = initial.clone();
    completed.state = LocalCliTaskState::Completed;
    completed.revision = 1;
    completed.native_session_id = Some("native-session".to_owned());

    assert!(checkpoint(&mut connection, completed.clone(), Some(1)).is_err());
    completed.terminal_evidence = Some("native-result:event-123".to_owned());
    completed.result = Some("测试通过".to_owned());
    checkpoint(&mut connection, completed.clone(), Some(1)).unwrap();
    assert_eq!(read_task(&mut connection, "task").unwrap(), Some(completed));
}

#[test]
fn local_cli_task_cannot_resume_while_active_or_change_native_session() {
    let mut connection = connection();
    let mut initial = task("task", None);
    initial.native_session_id = Some("native-session".to_owned());
    checkpoint(&mut connection, initial.clone(), None).unwrap();
    let mut duplicate = initial.clone();
    duplicate.generation = 2;
    assert!(checkpoint(&mut connection, duplicate, Some(1)).is_err());
    let mut changed_session = initial.clone();
    changed_session.state = LocalCliTaskState::Running;
    changed_session.revision = 1;
    changed_session.native_session_id = Some("different-session".to_owned());
    assert!(checkpoint(&mut connection, changed_session, Some(1)).is_err());
    assert_eq!(read_task(&mut connection, "task").unwrap(), Some(initial));
}

#[test]
fn local_cli_task_preserves_previous_generation_result() {
    let mut connection = connection();
    let initial = task("task", None);
    checkpoint(&mut connection, initial.clone(), None).unwrap();
    let mut completed = initial.clone();
    completed.state = LocalCliTaskState::Completed;
    completed.revision = 1;
    completed.native_session_id = Some("native-session".to_owned());
    completed.terminal_evidence = Some("native-result:event-123".to_owned());
    completed.result = Some("上一轮结果".to_owned());
    checkpoint(&mut connection, completed.clone(), Some(1)).unwrap();
    let mut resumed = initial;
    resumed.generation = 2;
    resumed.native_session_id = Some("native-session".to_owned());
    checkpoint(&mut connection, resumed.clone(), Some(1)).unwrap();

    assert_eq!(
        read_task_generations(&mut connection, "task").unwrap(),
        [completed, resumed]
    );
}

#[test]
fn local_cli_task_cannot_claim_another_active_native_session() {
    let mut connection = connection();
    let mut first = task("first", None);
    first.native_session_id = Some("shared-session".to_owned());
    checkpoint(&mut connection, first, None).unwrap();
    let mut second = task("second", None);
    second.native_session_id = Some("shared-session".to_owned());

    assert!(checkpoint(&mut connection, second, None).is_err());
    assert!(read_task(&mut connection, "second").unwrap().is_none());
}

#[test]
fn local_cli_unconfirmed_run_keeps_native_session_and_generation_ownership() {
    let mut connection = connection();
    let mut first = task("first", None);
    first.native_session_id = Some("shared-session".to_owned());
    checkpoint(&mut connection, first.clone(), None).unwrap();
    first.state = LocalCliTaskState::Unconfirmed;
    first.revision = 1;
    first.result = Some("完成尚未确认的响应".to_owned());
    checkpoint(&mut connection, first.clone(), Some(1)).unwrap();
    assert_eq!(
        serde_json::to_value(first.state).unwrap(),
        json!("unconfirmed")
    );
    assert!(first.state.is_active());
    assert!(!first.state.is_terminal());

    let mut second = task("second", None);
    second.native_session_id = Some("shared-session".to_owned());
    assert!(checkpoint(&mut connection, second, None).is_err());
    let mut resumed = first.clone();
    resumed.generation = 2;
    resumed.revision = 0;
    resumed.state = LocalCliTaskState::Queued;
    resumed.result = None;
    assert!(checkpoint(&mut connection, resumed, Some(1)).is_err());
    assert!(insert_task_result(&mut connection, "first", 1).is_err());
    assert_eq!(read_task(&mut connection, "first").unwrap(), Some(first));
    assert!(read_task(&mut connection, "second").unwrap().is_none());
}

#[test]
fn local_cli_unconfirmed_restart_changes_only_connection_state_without_replaying_messages() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("tasks.sqlite");
    let mut connection = SqliteConnection::establish(database.to_str().unwrap()).unwrap();
    connection
        .run_pending_migrations(::persistence::MIGRATIONS)
        .unwrap();
    parent_and_child(&mut connection);
    insert_message(&mut connection, message()).unwrap();
    update_message_state(
        &mut connection,
        "message-1",
        "child",
        1,
        LocalCliMessageState::Sent,
    )
    .unwrap();
    let mut queued = message();
    queued.message_id = "message-2".to_owned();
    insert_message(&mut connection, queued.clone()).unwrap();
    let mut child = task("child", Some("parent"));
    child.state = LocalCliTaskState::Unconfirmed;
    child.revision = 1;
    child.result = Some("仍需核对的结果".to_owned());
    checkpoint(&mut connection, child.clone(), Some(1)).unwrap();
    let messages = read_messages(&mut connection, "child", 1).unwrap();
    assert_eq!(messages[0].state, LocalCliMessageState::Sent);
    assert_eq!(messages[0].receipt_kind, None);
    assert_eq!(messages[1], queued);
    drop(connection);

    let mut reopened = SqliteConnection::establish(database.to_str().unwrap()).unwrap();
    assert_eq!(read_tasks(&mut reopened, false).unwrap()[0], child);
    assert_eq!(read_messages(&mut reopened, "child", 1).unwrap(), messages);
    assert_eq!(
        insert_message(&mut reopened, message()).unwrap(),
        LocalCliEnqueueOutcome::Existing(LocalCliMessageState::Sent)
    );
    assert!(
        update_message_state(
            &mut reopened,
            "message-1",
            "child",
            2,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
    assert!(insert_task_result(&mut reopened, "child", 1).is_err());
    assert_eq!(read_messages(&mut reopened, "child", 1).unwrap(), messages);

    let recovered = read_tasks(&mut reopened, true).unwrap();
    assert_eq!(recovered[0].state, LocalCliTaskState::Disconnected);
    assert_eq!(recovered[0].generation, 1);
    assert_eq!(recovered[0].revision, 2);
    assert_eq!(recovered[0].result, child.result);
    assert_eq!(recovered[0].terminal_evidence, None);
    assert_eq!(read_messages(&mut reopened, "child", 1).unwrap(), messages);
    assert_eq!(read_tasks(&mut reopened, true).unwrap(), recovered);

    let mut resumed = recovered[0].clone();
    resumed.generation = 2;
    resumed.revision = 0;
    resumed.state = LocalCliTaskState::Queued;
    resumed.result = None;
    checkpoint(&mut reopened, resumed, Some(1)).unwrap();
    assert!(
        update_message_state(
            &mut reopened,
            "message-1",
            "child",
            1,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
    let old_messages = read_messages(&mut reopened, "child", 1).unwrap();
    assert_eq!(old_messages[0].state, LocalCliMessageState::Sent);
    assert_eq!(old_messages[0].receipt_kind, None);
    assert_eq!(old_messages[1].state, LocalCliMessageState::Cancelled);
}

#[test]
fn local_cli_unconfirmed_parent_can_claim_a_verified_child_result_only_once() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    let mut parent = task("parent", None);
    parent.state = LocalCliTaskState::Unconfirmed;
    parent.revision = 1;
    checkpoint(&mut connection, parent, Some(1)).unwrap();
    let mut child = task("child", Some("parent"));
    child.state = LocalCliTaskState::Completed;
    child.revision = 1;
    child.native_session_id = Some("native-child".to_owned());
    child.terminal_evidence = Some("native-result:child-turn".to_owned());
    checkpoint(&mut connection, child, Some(1)).unwrap();
    let result = insert_task_result(&mut connection, "child", 1)
        .unwrap()
        .unwrap();

    let claimed = claim_result(&mut connection, result.clone())
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state, LocalCliMessageState::Sent);
    assert_eq!(claimed.receipt_kind, None);
    assert!(claim_result(&mut connection, result).unwrap().is_none());
    assert_eq!(
        read_task(&mut connection, "parent").unwrap().unwrap().state,
        LocalCliTaskState::Unconfirmed
    );
}

#[test]
fn local_cli_future_unknown_still_rejects_messages_and_receipts() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    insert_message(&mut connection, message()).unwrap();
    update_message_state(
        &mut connection,
        "message-1",
        "child",
        1,
        LocalCliMessageState::Sent,
    )
    .unwrap();
    let mut future = serde_json::to_value(task("child", Some("parent"))).unwrap();
    future["state"] = json!("future_state");
    let future: LocalCliTask = serde_json::from_value(future).unwrap();
    assert_eq!(future.state, LocalCliTaskState::Unknown);
    assert!(checkpoint(&mut connection, future.clone(), Some(1)).is_err());
    // 模拟由未来版本留下的只读记录，不能通过新的未确认状态放宽兼容性检查。
    write_task(&mut connection, &future).unwrap();
    let mut next = message();
    next.message_id = "new-message".to_owned();
    assert!(insert_message(&mut connection, next).is_err());
    assert!(
        update_message_state(
            &mut connection,
            "message-1",
            "child",
            1,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
    let saved = read_messages(&mut connection, "child", 1).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].state, LocalCliMessageState::Sent);
    assert_eq!(saved[0].receipt_kind, None);
}

#[test]
fn local_cli_unconfirmed_index_migration_upgrades_existing_rows_without_rewriting_them() {
    let mut connection = connection();
    let reverted = connection
        .revert_last_migration(::persistence::MIGRATIONS)
        .unwrap();
    assert_eq!(reverted.to_string(), "20260916000001");
    parent_and_child(&mut connection);
    insert_message(&mut connection, message()).unwrap();
    let before = read_tasks(&mut connection, false).unwrap();
    let before_messages = read_messages(&mut connection, "child", 1).unwrap();

    let applied = connection
        .run_pending_migrations(::persistence::MIGRATIONS)
        .unwrap();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].to_string(), "20260916000001");
    assert_eq!(read_tasks(&mut connection, false).unwrap(), before);
    assert_eq!(
        read_messages(&mut connection, "child", 1).unwrap(),
        before_messages
    );

    let mut child = task("child", Some("parent"));
    child.state = LocalCliTaskState::Unconfirmed;
    child.native_session_id = Some("native-child".to_owned());
    child.revision = 1;
    checkpoint(&mut connection, child.clone(), Some(1)).unwrap();
    let mut duplicate = task("duplicate", None);
    duplicate.native_session_id = Some("native-child".to_owned());
    assert!(checkpoint(&mut connection, duplicate, None).is_err());
    assert!(
        connection
            .revert_last_migration(::persistence::MIGRATIONS)
            .is_err()
    );
    assert_eq!(read_task(&mut connection, "child").unwrap(), Some(child));
}

#[test]
fn local_cli_task_recovery_marks_disconnected_without_resending_messages() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    assert_eq!(
        insert_message(&mut connection, message()).unwrap(),
        LocalCliEnqueueOutcome::Created
    );
    update_message_state(
        &mut connection,
        "message-1",
        "child",
        1,
        LocalCliMessageState::Sent,
    )
    .unwrap();

    let restored = read_tasks(&mut connection, true).unwrap();

    assert_eq!(restored[0].state, LocalCliTaskState::Disconnected);
    assert_eq!(restored[1].state, LocalCliTaskState::Disconnected);
    assert_eq!(restored[0].revision, 1);
    assert_eq!(
        read_messages(&mut connection, "child", 1).unwrap()[0].state,
        LocalCliMessageState::Sent
    );
    assert_eq!(read_tasks(&mut connection, true).unwrap(), restored);
}

#[test]
fn local_cli_messages_deduplicate_without_reverting_acknowledgement() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    insert_message(&mut connection, message()).unwrap();
    update_message_state(
        &mut connection,
        "message-1",
        "child",
        1,
        LocalCliMessageState::Acknowledged,
    )
    .unwrap();
    assert_eq!(
        insert_message(&mut connection, message()).unwrap(),
        LocalCliEnqueueOutcome::Existing(LocalCliMessageState::Acknowledged)
    );

    let messages = read_messages(&mut connection, "child", 1).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].state, LocalCliMessageState::Acknowledged);
    assert!(
        update_message_state(
            &mut connection,
            "message-1",
            "child",
            1,
            LocalCliMessageState::Sent
        )
        .is_err()
    );
    let mut different = message();
    different.body = "另一个请求".to_owned();
    assert!(insert_message(&mut connection, different).is_err());
}

#[test]
fn local_cli_messages_reject_unrelated_tasks_and_wrong_run_receipts() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    checkpoint(&mut connection, task("unrelated", None), None).unwrap();
    let mut unrelated = message();
    unrelated.sender_task_id = "unrelated".to_owned();
    assert!(insert_message(&mut connection, unrelated).is_err());
    insert_message(&mut connection, message()).unwrap();

    assert!(
        update_message_state(
            &mut connection,
            "message-1",
            "child",
            2,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
    assert!(
        update_message_state(
            &mut connection,
            "message-1",
            "unrelated",
            1,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
    assert_eq!(
        read_messages(&mut connection, "child", 1).unwrap()[0].state,
        LocalCliMessageState::Queued
    );
}

#[test]
fn local_cli_cancelled_task_cancels_pending_outgoing_and_incoming_messages() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    insert_message(&mut connection, message()).unwrap();
    let mut outgoing = message();
    outgoing.message_id = "outgoing".to_owned();
    outgoing.sender_task_id = "child".to_owned();
    outgoing.recipient_task_id = "parent".to_owned();
    insert_message(&mut connection, outgoing).unwrap();
    let mut cancelled = task("child", Some("parent"));
    cancelled.revision = 1;
    cancelled.state = LocalCliTaskState::Cancelled;
    checkpoint(&mut connection, cancelled, Some(1)).unwrap();

    assert_eq!(
        read_messages(&mut connection, "child", 1).unwrap()[0].state,
        LocalCliMessageState::Cancelled
    );
    assert_eq!(
        read_messages(&mut connection, "parent", 1).unwrap()[0].state,
        LocalCliMessageState::Cancelled
    );
    assert!(
        update_message_state(
            &mut connection,
            "message-1",
            "child",
            1,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
}

#[test]
fn local_cli_new_generation_does_not_receive_old_queued_messages() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    insert_message(&mut connection, message()).unwrap();
    read_tasks(&mut connection, true).unwrap();
    let mut resumed = task("child", Some("parent"));
    resumed.generation = 2;
    checkpoint(&mut connection, resumed, Some(1)).unwrap();

    assert!(
        read_messages(&mut connection, "child", 2)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        read_messages(&mut connection, "child", 1).unwrap()[0].state,
        LocalCliMessageState::Cancelled
    );
    assert!(
        update_message_state(
            &mut connection,
            "message-1",
            "child",
            1,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
}

#[test]
fn local_cli_unknown_state_and_record_version_are_not_treated_as_success() {
    let mut connection = connection();
    let mut value = serde_json::to_value(task("task", None)).unwrap();
    value["state"] = json!("future-state");
    let unknown: LocalCliTask = serde_json::from_value(value).unwrap();
    assert_eq!(unknown.state, LocalCliTaskState::Unknown);
    assert!(checkpoint(&mut connection, unknown, None).is_err());
    let mut future = task("future", None);
    future.version = 2;
    assert!(checkpoint(&mut connection, future, None).is_err());
}

#[test]
fn local_cli_checkpoint_acknowledges_a_commit_visible_to_another_connection() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("tasks.sqlite");
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    let expected = task("task", None);
    let receiver = checkpoint_task(&writer.sender, expected.clone(), None).unwrap();

    assert_eq!(block_on(receiver).unwrap(), Ok(()));
    let mut reader = SqliteConnection::establish(database.to_str().unwrap()).unwrap();
    assert_eq!(read_task(&mut reader, "task").unwrap(), Some(expected));
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn local_cli_failed_checkpoint_rolls_back_and_does_not_acknowledge_success() {
    let mut connection = connection();
    let original = task("task", None);
    checkpoint(&mut connection, original.clone(), None).unwrap();
    connection.batch_execute("CREATE TRIGGER reject_cli_task BEFORE UPDATE ON local_cli_task_generations BEGIN SELECT RAISE(ABORT, 'rejected'); END;").unwrap();
    let mut running = original.clone();
    running.state = LocalCliTaskState::Running;
    running.revision = 1;
    let (completion, receiver) = oneshot::channel();

    assert!(
        handle_request(
            LocalCliPersistenceRequest::CheckpointTask {
                task: running,
                expected_generation: Some(1),
                completion,
            },
            &mut connection
        )
        .is_err()
    );
    assert!(block_on(receiver).unwrap().is_err());
    assert_eq!(read_task(&mut connection, "task").unwrap(), Some(original));
}

#[test]
fn local_cli_full_writer_queue_rejects_without_blocking() {
    let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
    sender.send(ModelEvent::Terminate).unwrap();

    assert!(checkpoint_task(&sender, task("task", None), None).is_err());
}

#[test]
fn local_cli_user_input_requires_the_same_current_generation() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    let mut input = message();
    input.sender_task_id = "child".to_owned();
    input.subject = "user_input".to_owned();
    assert_eq!(
        insert_message(&mut connection, input.clone()).unwrap(),
        LocalCliEnqueueOutcome::Created
    );
    input.message_id = "old-user-input".to_owned();
    input.sender_generation = 2;
    assert!(insert_message(&mut connection, input).is_err());
}

#[test]
fn local_cli_result_recovery_is_unique_and_cannot_be_forged_by_a_message() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    assert!(insert_task_result(&mut connection, "child", 1).is_err());
    let mut child = task("child", Some("parent"));
    child.state = LocalCliTaskState::Cancelled;
    child.revision = 1;
    child.terminal_evidence = Some("native_cancelled".to_owned());
    checkpoint(&mut connection, child, Some(1)).unwrap();
    let result = insert_task_result(&mut connection, "child", 1)
        .unwrap()
        .unwrap();
    assert_eq!(
        insert_task_result(&mut connection, "child", 1).unwrap(),
        Some(result.clone())
    );
    assert_eq!(result.sender_task_id, "child");
    assert_eq!(result.recipient_task_id, "parent");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result.body).unwrap()["state"],
        json!("cancelled")
    );
    assert!(insert_message(&mut connection, result.clone()).is_err());
    let mut resumed = task("child", Some("parent"));
    resumed.generation = 2;
    checkpoint(&mut connection, resumed, Some(1)).unwrap();
    update_message_state(
        &mut connection,
        &result.message_id,
        "parent",
        1,
        LocalCliMessageState::Acknowledged,
    )
    .unwrap();
    assert_eq!(
        read_messages(&mut connection, "parent", 1).unwrap()[0].state,
        LocalCliMessageState::Acknowledged
    );
}

#[test]
fn local_cli_terminal_events_do_not_invent_delivery_failure_or_acknowledgement() {
    for state in [
        LocalCliTaskState::Completed,
        LocalCliTaskState::Failed,
        LocalCliTaskState::Cancelled,
        LocalCliTaskState::Disconnected,
    ] {
        let mut connection = connection();
        parent_and_child(&mut connection);
        insert_message(&mut connection, message()).unwrap();
        update_message_state(
            &mut connection,
            "message-1",
            "child",
            1,
            LocalCliMessageState::Sent,
        )
        .unwrap();
        let mut child = task("child", Some("parent"));
        child.state = state;
        child.revision = 1;
        child.native_session_id = Some("native-session".to_owned());
        child.terminal_evidence = Some("native-event".to_owned());
        checkpoint(&mut connection, child, Some(1)).unwrap();
        assert_eq!(
            read_message(&mut connection, "message-1")
                .unwrap()
                .unwrap()
                .state,
            LocalCliMessageState::Sent,
            "任务状态 {state:?} 不能推断消息是否接收"
        );
        let mut next = task("child", Some("parent"));
        next.generation = 2;
        checkpoint(&mut connection, next, Some(1)).unwrap();
        assert!(
            update_message_state(
                &mut connection,
                "message-1",
                "child",
                1,
                LocalCliMessageState::Acknowledged,
            )
            .is_err()
        );
        assert_eq!(
            read_messages(&mut connection, "child", 1).unwrap()[0].state,
            LocalCliMessageState::Sent
        );
        assert!(
            read_messages(&mut connection, "child", 2)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn local_cli_large_result_is_recovered_with_utf8_excerpt_and_durable_locator() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    let result = "中文🙂\u{0}\u{1}".repeat(100_000);
    assert!(result.len() > 1024 * 1024);
    let mut child = task("child", Some("parent"));
    child.revision = 1;
    child.state = LocalCliTaskState::Completed;
    child.native_session_id = Some("native-session".to_owned());
    child.terminal_evidence = Some("原生终态".repeat(1000));
    child.result = Some(result.clone());
    checkpoint(&mut connection, child.clone(), Some(1)).unwrap();
    let message = insert_task_result(&mut connection, "child", 1)
        .unwrap()
        .unwrap();
    assert!(message.body.len() < 1024 * 1024);
    let body: serde_json::Value = serde_json::from_str(&message.body).unwrap();
    assert_eq!(body["task_id"], "child");
    assert_eq!(body["generation"], 1);
    assert_eq!(body["truncated"], true);
    assert_eq!(body["evidence_truncated"], true);
    assert_eq!(body["result_bytes"], result.len());
    assert!(result.starts_with(body["result"].as_str().unwrap()));
    assert!(body["result"].as_str().unwrap().len() <= 128 * 1024);
    assert_eq!(
        read_task(&mut connection, "child").unwrap(),
        Some(child.clone())
    );
    assert_eq!(
        read_task_generations(&mut connection, "child").unwrap(),
        [child]
    );
    assert_eq!(
        insert_task_result(&mut connection, "child", 1).unwrap(),
        Some(message)
    );
}

#[test]
fn local_cli_result_claim_is_atomic_and_rejects_ordinary_or_modified_messages() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    insert_message(&mut connection, message()).unwrap();
    assert!(claim_result(&mut connection, message()).is_err());
    let mut child = task("child", Some("parent"));
    child.state = LocalCliTaskState::Cancelled;
    child.revision = 1;
    checkpoint(&mut connection, child, Some(1)).unwrap();
    let result = insert_task_result(&mut connection, "child", 1)
        .unwrap()
        .unwrap();
    let mut modified = result.clone();
    modified.body.push_str("伪造结果");
    assert!(claim_result(&mut connection, modified).is_err());
    let claimed = claim_result(&mut connection, result.clone())
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state, LocalCliMessageState::Sent);
    assert_eq!(claim_result(&mut connection, result).unwrap(), None);
    assert_eq!(
        read_message(&mut connection, &claimed.message_id).unwrap(),
        Some(claimed)
    );
}

#[test]
fn local_cli_finished_parent_preserves_child_result_without_dispatch() {
    for state in [
        LocalCliTaskState::Completed,
        LocalCliTaskState::Failed,
        LocalCliTaskState::Cancelled,
    ] {
        let mut connection = connection();
        parent_and_child(&mut connection);
        let mut parent = task("parent", None);
        parent.revision = 1;
        parent.state = state;
        parent.native_session_id = Some("parent-session".to_owned());
        parent.terminal_evidence = Some("native-parent-terminal".to_owned());
        checkpoint(&mut connection, parent, Some(1)).unwrap();
        let mut child = task("child", Some("parent"));
        child.revision = 1;
        child.state = LocalCliTaskState::Completed;
        child.native_session_id = Some("child-session".to_owned());
        child.terminal_evidence = Some("native-child-terminal".to_owned());
        child.result = Some("子任务结果仍可回收".to_owned());
        checkpoint(&mut connection, child, Some(1)).unwrap();

        let result = insert_task_result(&mut connection, "child", 1)
            .unwrap()
            .unwrap();
        assert_eq!(
            insert_task_result(&mut connection, "child", 1).unwrap(),
            Some(result.clone())
        );
        assert!(claim_result(&mut connection, result.clone()).is_err());
        assert_eq!(
            read_message(&mut connection, &result.message_id).unwrap(),
            Some(result.clone())
        );
        assert_eq!(
            read_messages(&mut connection, "parent", 1).unwrap(),
            [result]
        );
    }
}

#[test]
fn local_cli_result_keeps_original_parent_run_after_resume_but_new_messages_are_allowed() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    let mut parent = task("parent", None);
    parent.revision = 1;
    parent.state = LocalCliTaskState::Disconnected;
    checkpoint(&mut connection, parent, Some(1)).unwrap();
    let mut resumed = task("parent", None);
    resumed.generation = 2;
    checkpoint(&mut connection, resumed, Some(1)).unwrap();

    let mut instruction = message();
    instruction.sender_generation = 2;
    assert_eq!(
        insert_message(&mut connection, instruction).unwrap(),
        LocalCliEnqueueOutcome::Created
    );
    let mut child = task("child", Some("parent"));
    child.revision = 1;
    child.state = LocalCliTaskState::Completed;
    child.native_session_id = Some("child-native".to_owned());
    child.terminal_evidence = Some("native-result".to_owned());
    checkpoint(&mut connection, child, Some(1)).unwrap();
    let result = insert_task_result(&mut connection, "child", 1)
        .unwrap()
        .unwrap();
    assert_eq!(result.recipient_generation, 1);
    assert!(claim_result(&mut connection, result.clone()).is_err());
    assert_eq!(
        read_messages(&mut connection, "parent", 1).unwrap(),
        [result]
    );
    assert!(
        read_messages(&mut connection, "parent", 2)
            .unwrap()
            .is_empty()
    );

    let mut child_resume = task("child", Some("parent"));
    child_resume.generation = 2;
    child_resume.parent_generation = Some(2);
    assert!(checkpoint(&mut connection, child_resume, Some(1)).is_err());
    let stale_child = task("new-child", Some("parent"));
    assert!(checkpoint(&mut connection, stale_child, None).is_err());
}

#[test]
fn local_cli_legacy_child_without_parent_run_is_read_only() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    let mut encoded = serde_json::to_value(task("child", Some("parent"))).unwrap();
    encoded.as_object_mut().unwrap().remove("parent_generation");
    let legacy: LocalCliTask = serde_json::from_value(encoded).unwrap();
    assert_eq!(legacy.parent_generation, None);
    write_task(&mut connection, &legacy).unwrap();
    let recovered = read_tasks(&mut connection, true).unwrap();
    assert!(recovered.contains(&legacy));
    assert!(insert_message(&mut connection, message()).is_err());
    let mut update = legacy.clone();
    update.revision = 1;
    update.state = LocalCliTaskState::Disconnected;
    assert!(checkpoint(&mut connection, update, Some(1)).is_err());
    let mut terminal = legacy;
    terminal.state = LocalCliTaskState::Cancelled;
    write_task(&mut connection, &terminal).unwrap();
    assert!(insert_task_result(&mut connection, "child", 1).is_err());
}

#[test]
fn local_cli_unknown_parent_state_preserves_child_result_for_inspection() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    let mut parent = task("parent", None);
    parent.state = LocalCliTaskState::Unknown;
    write_task(&mut connection, &parent).unwrap();
    let mut child = task("child", Some("parent"));
    child.revision = 1;
    child.state = LocalCliTaskState::Cancelled;
    checkpoint(&mut connection, child, Some(1)).unwrap();
    let result = insert_task_result(&mut connection, "child", 1)
        .unwrap()
        .unwrap();
    assert!(claim_result(&mut connection, result.clone()).is_err());
    assert_eq!(
        read_messages(&mut connection, "parent", 1).unwrap(),
        [result]
    );
}

#[test]
fn local_cli_new_generation_and_input_roll_back_together_on_reused_id_or_oversized_body() {
    let mut connection = connection();
    parent_and_child(&mut connection);
    insert_message(&mut connection, message()).unwrap();
    let mut child = task("child", Some("parent"));
    child.state = LocalCliTaskState::Completed;
    child.revision = 1;
    child.native_session_id = Some("native-previous".to_owned());
    child.terminal_evidence = Some("native-completed".to_owned());
    child.result = Some("上一代完整结果".to_owned());
    checkpoint(&mut connection, child.clone(), Some(1)).unwrap();
    let old_messages = read_messages(&mut connection, "child", 1).unwrap();
    let mut next = task("child", Some("parent"));
    next.generation = 2;
    let mut input = message();
    input.sender_task_id = "child".to_owned();
    input.sender_generation = 2;
    input.recipient_generation = 2;
    input.subject = "user_input".to_owned();
    let reused = input.clone();
    input.message_id = "large-message".to_owned();
    input.body = "a".repeat(1024 * 1024 + 1);
    for rejected in [reused, input] {
        assert!(checkpoint_with_message(&mut connection, next.clone(), Some(1), rejected).is_err());
        assert_eq!(
            read_task(&mut connection, "child").unwrap(),
            Some(child.clone())
        );
        assert_eq!(
            read_task_generations(&mut connection, "child").unwrap(),
            [child.clone()]
        );
        assert_eq!(
            read_messages(&mut connection, "child", 1).unwrap(),
            old_messages
        );
        assert!(
            read_messages(&mut connection, "child", 2)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn local_cli_atomic_input_ack_is_visible_only_after_task_and_message_commit() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("atomic-input.sqlite");
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    let initial = task("task", None);
    let input = LocalCliMessage {
        message_id: "initial-input".to_owned(),
        sender_task_id: "task".to_owned(),
        recipient_task_id: "task".to_owned(),
        subject: "user_input".to_owned(),
        ..message()
    };
    let receiver =
        checkpoint_task_with_message(&writer.sender, initial.clone(), None, input.clone()).unwrap();
    assert_eq!(
        block_on(receiver).unwrap(),
        Ok(LocalCliEnqueueOutcome::Created)
    );
    let mut reader = SqliteConnection::establish(database.to_str().unwrap()).unwrap();
    assert_eq!(read_task(&mut reader, "task").unwrap(), Some(initial));
    assert_eq!(
        read_message(&mut reader, "initial-input").unwrap(),
        Some(input)
    );
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn local_cli_application_history_receipt_is_distinct_from_native_protocol() {
    let mut connection = connection();
    let mut parent = task("parent", None);
    parent.harness = "oz".to_owned();
    checkpoint(&mut connection, parent, None).unwrap();
    let mut child = task("child", Some("parent"));
    checkpoint(&mut connection, child.clone(), None).unwrap();
    child.state = LocalCliTaskState::Cancelled;
    child.revision = 1;
    checkpoint(&mut connection, child, Some(1)).unwrap();
    let result = insert_task_result(&mut connection, "child", 1)
        .unwrap()
        .unwrap();
    claim_result(&mut connection, result.clone())
        .unwrap()
        .unwrap();
    update_message_state_with_receipt(
        &mut connection,
        &result.message_id,
        "parent",
        1,
        LocalCliMessageState::Acknowledged,
        Some(LocalCliReceiptKind::ApplicationHistory),
    )
    .unwrap();
    let stored = read_message(&mut connection, &result.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, LocalCliMessageState::Acknowledged);
    assert_eq!(
        stored.receipt_kind,
        Some(LocalCliReceiptKind::ApplicationHistory)
    );
    assert!(
        update_message_state(
            &mut connection,
            &result.message_id,
            "parent",
            1,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
    assert_eq!(
        read_message(&mut connection, &result.message_id).unwrap(),
        Some(stored)
    );
}

#[test]
fn ordinary_parent_history_message_requires_authorization_and_one_atomic_claim() {
    let mut connection = connection();
    let mut parent = task("parent", None);
    parent.harness = "oz".into();
    parent.config_json = serde_json::json!({"execution_kind":"local_parent",
        "history_identity":{"root_task_id":"root","user_exchange_id":"exchange"}})
    .to_string();
    checkpoint(&mut connection, parent, None).unwrap();
    let mut child = task("child", Some("parent"));
    checkpoint(&mut connection, child.clone(), None).unwrap();
    let progress = LocalCliMessage {
        sender_task_id: "child".into(),
        recipient_task_id: "parent".into(),
        subject: "progress".into(),
        body: "进度 中文\nEnglish 🧪".into(),
        ..message()
    };
    insert_message(&mut connection, progress.clone()).unwrap();
    assert!(claim_parent_message(&mut connection, progress.clone()).is_err());
    assert_eq!(
        read_message(&mut connection, &progress.message_id).unwrap(),
        Some(progress.clone())
    );
    child.revision += 1;
    child.config_json =
        serde_json::json!({"local_tools":{"allow_spawn":false,"allow_message":true}}).to_string();
    checkpoint(&mut connection, child, Some(1)).unwrap();
    assert!(
        update_message_state_with_receipt(
            &mut connection,
            &progress.message_id,
            "parent",
            1,
            LocalCliMessageState::Acknowledged,
            Some(LocalCliReceiptKind::ApplicationHistory)
        )
        .is_err()
    );
    let mut altered = progress.clone();
    altered.body.push_str(" altered");
    assert!(claim_parent_message(&mut connection, altered).is_err());
    let claimed = claim_parent_message(&mut connection, progress.clone())
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state, LocalCliMessageState::Sent);
    assert_eq!(claimed.receipt_kind, None);
    assert_eq!(
        claim_parent_message(&mut connection, progress).unwrap(),
        None
    );
    update_message_state_with_receipt(
        &mut connection,
        &claimed.message_id,
        "parent",
        1,
        LocalCliMessageState::Acknowledged,
        Some(LocalCliReceiptKind::ApplicationHistory),
    )
    .unwrap();
    let acknowledged = read_message(&mut connection, &claimed.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        acknowledged.receipt_kind,
        Some(LocalCliReceiptKind::ApplicationHistory)
    );
    assert!(
        update_message_state(
            &mut connection,
            &claimed.message_id,
            "parent",
            1,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
}

#[test]
fn ordinary_child_progress_cannot_follow_its_parent_into_a_new_generation() {
    let mut connection = connection();
    let mut parent = task("parent", None);
    parent.harness = "oz".into();
    parent.config_json = serde_json::json!({"execution_kind":"local_parent",
        "history_identity":{"root_task_id":"root","user_exchange_id":"exchange"}})
    .to_string();
    checkpoint(&mut connection, parent.clone(), None).unwrap();
    let mut child = task("child", Some("parent"));
    child.config_json =
        serde_json::json!({"local_tools":{"allow_spawn":false,"allow_message":true}}).to_string();
    checkpoint(&mut connection, child, None).unwrap();
    parent.revision += 1;
    parent.state = LocalCliTaskState::Disconnected;
    checkpoint(&mut connection, parent.clone(), Some(1)).unwrap();
    parent.generation = 2;
    parent.revision = 0;
    parent.state = LocalCliTaskState::Queued;
    checkpoint(&mut connection, parent, Some(1)).unwrap();
    let progress = LocalCliMessage {
        sender_task_id: "child".into(),
        recipient_task_id: "parent".into(),
        recipient_generation: 2,
        subject: "progress".into(),
        ..message()
    };
    insert_message(&mut connection, progress.clone()).unwrap();
    assert!(claim_parent_message(&mut connection, progress.clone()).is_err());
    assert_eq!(
        read_message(&mut connection, &progress.message_id).unwrap(),
        Some(progress)
    );
    // 新父代的明确指令仍可发给原有子任务，不全局改变普通信箱规则。
    let instruction = LocalCliMessage {
        message_id: "new-instruction".into(),
        sender_generation: 2,
        ..message()
    };
    assert_eq!(
        insert_message(&mut connection, instruction).unwrap(),
        LocalCliEnqueueOutcome::Created
    );
}
