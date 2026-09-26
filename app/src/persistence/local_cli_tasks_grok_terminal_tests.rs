use std::sync::{Arc, Barrier};
use std::thread;

use diesel::connection::SimpleConnection;
use diesel_migrations::MigrationHarness;
use futures::executor::block_on;
use serde_json::json;

use super::*;

fn connection(path: &str) -> SqliteConnection {
    let mut connection = SqliteConnection::establish(path).unwrap();
    connection
        .batch_execute("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000")
        .unwrap();
    connection
}

fn setup(connection: &mut SqliteConnection) -> GrokTerminalInputClaim {
    connection
        .run_pending_migrations(::persistence::MIGRATIONS)
        .unwrap();
    let owned = GrokTerminalOwner {
        version: 1,
        launch_id: Uuid::new_v4(),
        binding_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        input_revision: Uuid::new_v4(),
        permission_revision: Uuid::new_v4(),
        cli_version: "1.0.41".into(),
        model_id: "grok-4.7".into(),
        permission_mode: "default".into(),
    };
    let mut task = LocalCliTask {
        version: 1,
        task_id: Uuid::new_v4().to_string(),
        parent_task_id: None,
        parent_generation: None,
        harness: "grok".into(),
        working_directory: "/project/中文 空格".into(),
        config_json: json!({"execution_kind":EXECUTION_KIND,"grok_terminal":owned}).to_string(),
        native_session_id: Some(owned.session_id.to_string()),
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    checkpoint(connection, task.clone(), None).unwrap();
    task.state = LocalCliTaskState::Running;
    task.revision = 1;
    checkpoint(connection, task.clone(), Some(1)).unwrap();
    let message = LocalCliMessage {
        version: 1,
        message_id: Uuid::new_v4().to_string(),
        sender_task_id: task.task_id.clone(),
        recipient_task_id: task.task_id.clone(),
        sender_generation: 1,
        recipient_generation: 1,
        subject: INPUT_SUBJECT.into(),
        body: "第一行中文\n第二行 空格".into(),
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    };
    insert_message(connection, message.clone()).unwrap();
    GrokTerminalInputClaim {
        body_sha256: digest(&message.body),
        message,
        expected_task: task,
        binding_id: owned.binding_id,
        session_id: owned.session_id,
        input_revision: owned.input_revision,
        rpc_id: Uuid::new_v4(),
    }
}

fn fixture() -> (SqliteConnection, GrokTerminalInputClaim) {
    let mut connection = connection(":memory:");
    let claim = setup(&mut connection);
    (connection, claim)
}

fn claimed(
    connection: &mut SqliteConnection,
    claim: &GrokTerminalInputClaim,
) -> GrokTerminalDelivery {
    claim_once(connection, claim.clone())
        .unwrap()
        .unwrap()
        .grok_terminal_delivery
        .unwrap()
}

fn response(delivery: &GrokTerminalDelivery, prompt_id: Uuid) -> Value {
    json!({"jsonrpc":"2.0","id":delivery.rpc_id,"result":{"stopReason":"end_turn",
        "_meta":{"sessionId":delivery.session_id,"promptId":prompt_id,"requestId":prompt_id,"modelId":"grok-4.7"}}})
}

fn assert_queued(connection: &mut SqliteConnection, claim: &GrokTerminalInputClaim) {
    let (_, record) = read_record(
        connection,
        Uuid::parse_str(&claim.message.message_id).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(record.message, claim.message);
    assert_eq!(record.grok_terminal_delivery, None);
}

fn advance_config(
    connection: &mut SqliteConnection,
    task: &mut LocalCliTask,
    mutate: impl FnOnce(&mut Value),
) {
    let mut config: Value = serde_json::from_str(&task.config_json).unwrap();
    mutate(&mut config);
    task.config_json = config.to_string();
    task.revision += 1;
    checkpoint(connection, task.clone(), Some(task.generation)).unwrap();
}

#[test]
fn two_sqlite_connections_grant_exactly_one_transport_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("owned.sqlite");
    let mut database = connection(path.to_str().unwrap());
    let claim = setup(&mut database);
    let start = Arc::new(Barrier::new(3));
    let first_path = path.clone();
    let first_claim = claim.clone();
    let first_start = start.clone();
    let first = thread::spawn(move || {
        let mut database = connection(first_path.to_str().unwrap());
        first_start.wait();
        claim_once(&mut database, first_claim).unwrap().is_some()
    });
    let second_path = path.clone();
    let second_claim = claim.clone();
    let second_start = start.clone();
    let second = thread::spawn(move || {
        let mut database = connection(second_path.to_str().unwrap());
        second_start.wait();
        claim_once(&mut database, second_claim).unwrap().is_some()
    });
    start.wait();
    assert_ne!(first.join().unwrap(), second.join().unwrap());
    let (_, saved) = read_record(
        &mut database,
        Uuid::parse_str(&claim.message.message_id).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(saved.message.state, LocalCliMessageState::Sent);
    assert_eq!(
        saved.grok_terminal_delivery.unwrap().status,
        GrokTerminalDeliveryStatus::Unknown
    );
}

#[test]
fn unknown_delivery_survives_reopen_without_becoming_claimable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("owned.sqlite");
    let mut database = connection(path.to_str().unwrap());
    let claim = setup(&mut database);
    let delivery = claimed(&mut database, &claim);
    drop(database);
    let mut restored = connection(path.to_str().unwrap());
    assert_eq!(claim_once(&mut restored, claim.clone()).unwrap(), None);
    let (_, record) = read_record(&mut restored, delivery.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.grok_terminal_delivery, Some(delivery));
    assert_eq!(record.message.state, LocalCliMessageState::Sent);
}

#[test]
fn another_message_id_cannot_replay_the_same_editor_snapshot() {
    let (mut database, claim) = fixture();
    claimed(&mut database, &claim);
    let mut repeat = claim;
    repeat.message.message_id = Uuid::new_v4().to_string();
    repeat.rpc_id = Uuid::new_v4();
    insert_message(&mut database, repeat.message.clone()).unwrap();
    assert!(claim_once(&mut database, repeat.clone()).is_err());
    assert_queued(&mut database, &repeat);
}

#[test]
fn another_editor_snapshot_cannot_reuse_a_native_rpc_id() {
    let (mut database, claim) = fixture();
    claimed(&mut database, &claim);
    let mut repeat = claim;
    repeat.input_revision = Uuid::new_v4();
    advance_config(&mut database, &mut repeat.expected_task, |config| {
        config["grok_terminal"]["input_revision"] = json!(repeat.input_revision);
    });
    repeat.message.message_id = Uuid::new_v4().to_string();
    insert_message(&mut database, repeat.message.clone()).unwrap();
    assert!(claim_once(&mut database, repeat.clone()).is_err());
    assert_queued(&mut database, &repeat);
}

#[test]
fn task_revision_changed_after_snapshot_leaves_message_queued() {
    let (mut database, claim) = fixture();
    let mut current = claim.expected_task.clone();
    current.revision += 1;
    checkpoint(&mut database, current, Some(1)).unwrap();
    assert!(claim_once(&mut database, claim.clone()).is_err());
    assert_queued(&mut database, &claim);
}

#[test]
fn changed_config_cannot_be_hidden_by_a_matching_task_snapshot() {
    let (mut database, mut claim) = fixture();
    advance_config(&mut database, &mut claim.expected_task, |config| {
        config["grok_terminal"]["binding_id"] = json!(Uuid::new_v4());
    });
    assert!(claim_once(&mut database, claim.clone()).is_err());
    assert_queued(&mut database, &claim);
}

#[test]
fn changed_input_revision_is_not_authorized_by_fresh_task_revision() {
    let (mut database, mut claim) = fixture();
    advance_config(&mut database, &mut claim.expected_task, |config| {
        config["grok_terminal"]["input_revision"] = json!(Uuid::new_v4());
    });
    assert!(claim_once(&mut database, claim.clone()).is_err());
    assert_queued(&mut database, &claim);
}

#[test]
fn changed_permission_mode_cannot_claim_even_with_fresh_task_snapshot() {
    let (mut database, mut claim) = fixture();
    advance_config(&mut database, &mut claim.expected_task, |config| {
        config["grok_terminal"]["permission_mode"] = json!("bypassPermissions");
    });
    assert!(claim_once(&mut database, claim.clone()).is_err());
    assert_queued(&mut database, &claim);
}

#[test]
fn body_hash_and_exact_persisted_body_are_both_required() {
    let (mut database, claim) = fixture();
    let mut wrong_hash = claim.clone();
    wrong_hash.body_sha256 = "0".repeat(64);
    assert!(claim_once(&mut database, wrong_hash).is_err());
    let mut replaced_body = claim.clone();
    replaced_body.message.body = "不同的待发送内容".into();
    replaced_body.body_sha256 = digest(&replaced_body.message.body);
    assert!(claim_once(&mut database, replaced_body).is_err());
    assert_queued(&mut database, &claim);
}

#[test]
fn claimed_unknown_is_not_cancelled_by_task_disconnect_or_restart_recovery() {
    let (mut database, claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    let mut current = claim.expected_task;
    current.state = LocalCliTaskState::Cancelled;
    current.revision += 1;
    checkpoint(&mut database, current, Some(1)).unwrap();
    read_tasks(&mut database, true).unwrap();
    let (_, record) = read_record(&mut database, delivery.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.message.state, LocalCliMessageState::Sent);
    assert_eq!(record.grok_terminal_delivery, Some(delivery));
}

#[test]
fn exact_native_final_is_durable_and_duplicate_final_is_idempotent() {
    let (mut database, claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    let prompt = Uuid::new_v4();
    let native = response(&delivery, prompt);
    let first = acknowledge_exact(&mut database, delivery.clone(), native.clone())
        .unwrap()
        .unwrap();
    let duplicate = acknowledge_exact(&mut database, delivery, native)
        .unwrap()
        .unwrap();
    assert_eq!(duplicate, first);
    assert_eq!(first.message.state, LocalCliMessageState::Acknowledged);
    assert_eq!(
        first.message.receipt_kind,
        Some(LocalCliReceiptKind::NativeProtocol)
    );
    assert_eq!(
        first.grok_terminal_delivery.unwrap().status,
        GrokTerminalDeliveryStatus::Finished {
            native_prompt_id: prompt,
            outcome: GrokTerminalOutcome::EndTurn
        }
    );
}

#[test]
fn wrong_rpc_session_or_request_id_cannot_acknowledge_a_sent_input() {
    let (mut database, claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    let mut wrong_rpc = response(&delivery, Uuid::new_v4());
    wrong_rpc["id"] = json!(Uuid::new_v4());
    assert!(acknowledge_exact(&mut database, delivery.clone(), wrong_rpc).is_err());
    let mut wrong_session = response(&delivery, Uuid::new_v4());
    wrong_session["result"]["_meta"]["sessionId"] = json!(Uuid::new_v4());
    assert!(acknowledge_exact(&mut database, delivery.clone(), wrong_session).is_err());
    let mut wrong_request = response(&delivery, Uuid::new_v4());
    wrong_request["result"]["_meta"]["requestId"] = json!(Uuid::new_v4());
    assert!(acknowledge_exact(&mut database, delivery.clone(), wrong_request).is_err());
    let (_, record) = read_record(&mut database, delivery.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.grok_terminal_delivery, Some(delivery));
}

#[test]
fn queue_notification_and_rpc_error_preserve_unknown_delivery() {
    let (mut database, claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    let queued = json!({"jsonrpc":"2.0","method":"_x.ai/queue/changed","params":{"sessionId":delivery.session_id,"entries":[]}});
    assert!(acknowledge_exact(&mut database, delivery.clone(), queued).is_err());
    let error =
        json!({"jsonrpc":"2.0","id":delivery.rpc_id,"error":{"code":-32603,"message":"失败"}});
    assert!(acknowledge_exact(&mut database, delivery.clone(), error).is_err());
    let (_, record) = read_record(&mut database, delivery.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.grok_terminal_delivery, Some(delivery));
}

#[test]
fn conflicting_second_native_prompt_does_not_replace_first_receipt() {
    let (mut database, claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    let first = acknowledge_exact(
        &mut database,
        delivery.clone(),
        response(&delivery, Uuid::new_v4()),
    )
    .unwrap()
    .unwrap();
    assert!(
        acknowledge_exact(
            &mut database,
            delivery.clone(),
            response(&delivery, Uuid::new_v4())
        )
        .is_err()
    );
    let (_, saved) = read_record(&mut database, delivery.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(saved, first);
}

#[test]
fn final_after_new_draft_updates_only_the_original_message() {
    let (mut database, mut claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    advance_config(&mut database, &mut claim.expected_task, |config| {
        config["grok_terminal"]["input_revision"] = json!(Uuid::new_v4());
    });
    let current = read_task(&mut database, &claim.expected_task.task_id).unwrap();
    acknowledge_exact(
        &mut database,
        delivery.clone(),
        response(&delivery, Uuid::new_v4()),
    )
    .unwrap();
    assert_eq!(
        read_task(&mut database, &claim.expected_task.task_id).unwrap(),
        current
    );
}

#[test]
fn old_callback_cannot_acknowledge_after_owned_binding_is_replaced() {
    let (mut database, mut claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    advance_config(&mut database, &mut claim.expected_task, |config| {
        config["grok_terminal"]["binding_id"] = json!(Uuid::new_v4());
    });
    assert!(
        acknowledge_exact(
            &mut database,
            delivery.clone(),
            response(&delivery, Uuid::new_v4())
        )
        .is_err()
    );
    let (_, record) = read_record(&mut database, delivery.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.grok_terminal_delivery, Some(delivery));
}

#[test]
fn generic_ack_and_managed_claim_do_not_bypass_owned_input_contract() {
    let (mut database, claim) = fixture();
    assert!(
        claim_managed_message(
            &mut database,
            claim.message.clone(),
            &claim.expected_task,
            &claim.expected_task
        )
        .is_err()
    );
    assert_queued(&mut database, &claim);
    let delivery = claimed(&mut database, &claim);
    assert!(
        update_message_state(
            &mut database,
            &claim.message.message_id,
            &claim.expected_task.task_id,
            1,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
    let (_, record) = read_record(&mut database, delivery.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.grok_terminal_delivery, Some(delivery));
}

#[test]
fn native_cancelled_receipt_confirms_delivery_without_claiming_successful_answer() {
    let (mut database, claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    let prompt = Uuid::new_v4();
    let mut native = response(&delivery, prompt);
    native["result"]["stopReason"] = json!("cancelled");
    let record = acknowledge_exact(&mut database, delivery, native)
        .unwrap()
        .unwrap();
    assert_eq!(
        record.grok_terminal_delivery.unwrap().status,
        GrokTerminalDeliveryStatus::Finished {
            native_prompt_id: prompt,
            outcome: GrokTerminalOutcome::Cancelled
        }
    );
}

#[test]
fn persisted_body_corruption_does_not_erase_original_delivery_evidence() {
    let (mut database, claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    let (raw, _) = read_record(&mut database, delivery.message_id)
        .unwrap()
        .unwrap();
    let mut value: Value = serde_json::from_str(&raw).unwrap();
    value["body"] = json!("被外部改写");
    diesel::update(
        local_cli_messages::table
            .filter(local_cli_messages::message_id.eq(&claim.message.message_id)),
    )
    .set(local_cli_messages::data.eq(value.to_string()))
    .execute(&mut database)
    .unwrap();
    assert!(read_record(&mut database, delivery.message_id).is_err());
    assert!(claim_once(&mut database, claim).is_err());
}

#[test]
fn sqlite_writer_rejection_reports_error_without_granting_a_claim() {
    let (mut database, claim) = fixture();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let result = claim_input(&sender, claim.clone()).unwrap();
    let ModelEvent::LocalCliPersistence(request) = receiver.recv().unwrap() else {
        panic!("必须使用现有 SQLite writer")
    };
    super::super::reject_request(request);
    assert!(block_on(result).unwrap().is_err());
    assert_queued(&mut database, &claim);
}

#[test]
fn writer_api_commits_before_exposing_claim_and_loads_exact_delivery() {
    let (mut database, claim) = fixture();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let completion = claim_input(&sender, claim.clone()).unwrap();
    let ModelEvent::LocalCliPersistence(request) = receiver.recv().unwrap() else {
        panic!("必须使用现有 SQLite writer")
    };
    super::super::handle_request(request, &mut database).unwrap();
    let record = block_on(completion).unwrap().unwrap().unwrap();
    let delivery = record.grok_terminal_delivery.clone().unwrap();
    let loaded = load_input(&sender, delivery.message_id).unwrap();
    let ModelEvent::LocalCliPersistence(request) = receiver.recv().unwrap() else {
        panic!("必须使用现有 SQLite writer")
    };
    super::super::handle_request(request, &mut database).unwrap();
    assert_eq!(block_on(loaded).unwrap().unwrap(), Some(record));
    let completed = acknowledge_input(
        &sender,
        delivery.clone(),
        response(&delivery, Uuid::new_v4()),
    )
    .unwrap();
    let ModelEvent::LocalCliPersistence(request) = receiver.recv().unwrap() else {
        panic!("必须使用现有 SQLite writer")
    };
    super::super::handle_request(request, &mut database).unwrap();
    assert_eq!(
        block_on(completed).unwrap().unwrap().unwrap().message.state,
        LocalCliMessageState::Acknowledged
    );
}

#[test]
fn ordinary_task_stop_cancels_only_unclaimed_owned_input() {
    let (mut database, claim) = fixture();
    let mut stopped = claim.expected_task.clone();
    stopped.state = LocalCliTaskState::Cancelled;
    stopped.revision += 1;
    checkpoint(&mut database, stopped, Some(1)).unwrap();
    assert_eq!(claim_once(&mut database, claim.clone()).unwrap(), None);
    let (_, record) = read_record(
        &mut database,
        Uuid::parse_str(&claim.message.message_id).unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(record.message.state, LocalCliMessageState::Cancelled);
    assert_eq!(record.grok_terminal_delivery, None);
}

#[test]
fn generic_message_writer_cannot_drop_claim_metadata() {
    let (mut database, claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    let mut forged = claim.message;
    forged.state = LocalCliMessageState::Cancelled;
    assert!(write_message_state(&mut database, &forged).is_err());
    let (_, record) = read_record(&mut database, delivery.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.grok_terminal_delivery, Some(delivery));
    assert_eq!(record.message.state, LocalCliMessageState::Sent);
}

#[test]
fn old_generation_final_preserves_original_unknown_record() {
    let (mut database, mut claim) = fixture();
    let delivery = claimed(&mut database, &claim);
    claim.expected_task.state = LocalCliTaskState::Disconnected;
    claim.expected_task.revision += 1;
    checkpoint(&mut database, claim.expected_task.clone(), Some(1)).unwrap();
    claim.expected_task.state = LocalCliTaskState::Queued;
    claim.expected_task.generation = 2;
    claim.expected_task.revision = 0;
    checkpoint(&mut database, claim.expected_task, Some(1)).unwrap();
    assert!(
        acknowledge_exact(
            &mut database,
            delivery.clone(),
            response(&delivery, Uuid::new_v4())
        )
        .is_err()
    );
    let (_, record) = read_record(&mut database, delivery.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.grok_terminal_delivery, Some(delivery));
}
