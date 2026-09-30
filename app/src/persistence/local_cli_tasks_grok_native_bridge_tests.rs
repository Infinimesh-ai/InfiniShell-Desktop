use std::sync::{Arc, Barrier};
use std::thread;

use diesel::connection::SimpleConnection;
use diesel_migrations::MigrationHarness;
use serde_json::json;

use super::*;

fn connection(path: &str) -> SqliteConnection {
    let mut connection = SqliteConnection::establish(path).unwrap();
    connection
        .batch_execute("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000")
        .unwrap();
    connection
}

fn input() -> NativeBridgeInput {
    NativeBridgeInput {
        binding: json!({
            "instance_id": Uuid::from_u128(1),
            "native_process": {"pid":101,"pid_version":2,"unique_id":3,"resource_cid":0},
            "boot_session":"ABCDEFAB-1234-5678-9012-ABCDEF123456",
            "shell": {"pid":102,"pid_version":4,"unique_id":5,"resource_cid":0},
            "slave_device":6,
            "artifact_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        }),
        session_id: Uuid::from_u128(2),
        working_directory: "/project/中文 空格".into(),
        input_revision: Uuid::from_u128(3),
        message_id: Uuid::from_u128(4),
        replaces: None,
        body: "第一行中文\n第二行按字面提交".into(),
    }
}

fn fixture() -> SqliteConnection {
    let mut connection = connection(":memory:");
    connection
        .run_pending_migrations(::persistence::MIGRATIONS)
        .unwrap();
    connection
}

fn claimed(connection: &mut SqliteConnection, input: NativeBridgeInput) -> NativeBridgeInputRecord {
    match claim_once(connection, input).unwrap() {
        NativeBridgeClaimOutcome::Claimed(record) => record,
        NativeBridgeClaimOutcome::Existing(record) => panic!("意外返回已有记录 {record:?}"),
    }
}

fn response(record: &NativeBridgeInputRecord, state: &str) -> Value {
    json!({"status":"receipt","receipt":{
        "message_id":record.delivery.message_id,
        "prompt_id":record.delivery.message_id,
        "session_id":record.delivery.session_id,
        "payload_digest":record.delivery.payload_digest,
        "state":state,
    }})
}

fn acknowledge(
    connection: &mut SqliteConnection,
    record: &NativeBridgeInputRecord,
) -> NativeBridgeInputRecord {
    acknowledge_exact(
        connection,
        record.delivery.clone(),
        serde_json::to_value(&record.delivery.binding).unwrap(),
        response(record, "native_acknowledged"),
    )
    .unwrap()
    .unwrap()
}

fn next_attempt(previous: &NativeBridgeInputRecord) -> NativeBridgeInput {
    NativeBridgeInput {
        message_id: Uuid::new_v4(),
        input_revision: Uuid::new_v4(),
        replaces: Some(previous.delivery.message_id),
        ..input()
    }
}

#[test]
fn first_claim_is_unknown_before_transport_and_repeated_click_cannot_write() {
    let mut connection = fixture();
    let record = claimed(&mut connection, input());
    assert_eq!(record.delivery.state, NativeBridgeDeliveryState::Unknown);
    assert_eq!(record.message.state, LocalCliMessageState::Unknown);
    assert_eq!(record.message.receipt_kind, None);
    assert_eq!(
        read_record(&mut connection, record.delivery.message_id)
            .unwrap()
            .unwrap()
            .1,
        record
    );
    assert_eq!(
        claim_once(&mut connection, input()).unwrap(),
        NativeBridgeClaimOutcome::Existing(record)
    );
}

#[test]
fn two_sqlite_connections_grant_one_initial_transport_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite");
    let mut setup = connection(path.to_str().unwrap());
    setup
        .run_pending_migrations(::persistence::MIGRATIONS)
        .unwrap();
    drop(setup);
    let start = Arc::new(Barrier::new(3));
    let first_path = path.clone();
    let first_start = start.clone();
    let first = thread::spawn(move || {
        let mut connection = connection(first_path.to_str().unwrap());
        first_start.wait();
        claim_once(&mut connection, input()).unwrap()
    });
    let second_path = path.clone();
    let second_start = start.clone();
    let second = thread::spawn(move || {
        let mut connection = connection(second_path.to_str().unwrap());
        let input = NativeBridgeInput {
            message_id: Uuid::new_v4(),
            ..input()
        };
        second_start.wait();
        claim_once(&mut connection, input).unwrap()
    });
    start.wait();
    match (first.join().unwrap(), second.join().unwrap()) {
        (NativeBridgeClaimOutcome::Claimed(first), NativeBridgeClaimOutcome::Existing(second))
        | (NativeBridgeClaimOutcome::Existing(first), NativeBridgeClaimOutcome::Claimed(second)) => {
            assert_eq!(first, second)
        }
        (NativeBridgeClaimOutcome::Claimed(_), NativeBridgeClaimOutcome::Claimed(_))
        | (NativeBridgeClaimOutcome::Existing(_), NativeBridgeClaimOutcome::Existing(_)) => {
            panic!("领取必须恰好允许一次传输")
        }
    }
    let mut connection = connection(path.to_str().unwrap());
    assert_eq!(native_records(&mut connection).unwrap().len(), 1);
}

#[test]
fn cold_recovery_and_new_instance_do_not_replay_unknown_body() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite");
    let mut first = connection(path.to_str().unwrap());
    first
        .run_pending_migrations(::persistence::MIGRATIONS)
        .unwrap();
    let original = claimed(&mut first, input());
    drop(first);
    let mut recovered = connection(path.to_str().unwrap());
    let tasks = read_tasks(&mut recovered, true).unwrap();
    assert!(tasks.is_empty());
    let internal = read_task(&mut recovered, &original.delivery.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(internal.state, LocalCliTaskState::Running);
    assert_eq!(internal.revision, 0);
    assert_eq!(internal.native_session_id, None);
    assert_eq!(
        serde_json::from_str::<Value>(&internal.config_json).unwrap()["session_id"],
        json!(input().session_id)
    );
    let mut current = input();
    current.message_id = Uuid::new_v4();
    current.input_revision = Uuid::new_v4();
    current.binding["instance_id"] = json!(Uuid::from_u128(50));
    assert_eq!(
        lookup_exact(
            &mut recovered,
            current.binding.clone(),
            Some(current.session_id),
            &current.body
        )
        .unwrap(),
        Some(original.clone())
    );
    assert_eq!(
        claim_once(&mut recovered, current.clone()).unwrap(),
        NativeBridgeClaimOutcome::Existing(original.clone())
    );
    assert_eq!(
        local_cli_tasks::table
            .count()
            .get_result::<i64>(&mut recovered)
            .unwrap(),
        1
    );
    assert!(
        acknowledge_exact(
            &mut recovered,
            original.delivery.clone(),
            current.binding.clone(),
            response(&original, "native_acknowledged")
        )
        .is_err()
    );
    current.body = "新的明确输入".into();
    let new_record = claimed(&mut recovered, current);
    assert_ne!(new_record.delivery.task_id, original.delivery.task_id);
    assert_eq!(new_record.delivery.session_id, original.delivery.session_id);
    assert_eq!(
        read_task(&mut recovered, &new_record.delivery.task_id)
            .unwrap()
            .unwrap()
            .native_session_id,
        None
    );
    assert_eq!(native_records(&mut recovered).unwrap().len(), 2);
}

#[test]
fn lookup_without_session_requires_one_matching_binding_session_head() {
    let mut connection = fixture();
    let original = claimed(&mut connection, input());
    assert_eq!(
        lookup_exact(&mut connection, input().binding, None, &input().body).unwrap(),
        Some(original.clone())
    );
    let other = NativeBridgeInput {
        session_id: Uuid::from_u128(20),
        message_id: Uuid::from_u128(21),
        input_revision: Uuid::from_u128(22),
        ..input()
    };
    claimed(&mut connection, other);
    assert!(lookup_exact(&mut connection, input().binding, None, &input().body).is_err());
    assert_eq!(
        lookup_exact(
            &mut connection,
            input().binding,
            Some(input().session_id),
            &input().body
        )
        .unwrap(),
        Some(original)
    );
}

#[test]
fn message_id_or_input_revision_cannot_be_reused_for_different_body() {
    let mut connection = fixture();
    let original = claimed(&mut connection, input());
    let mut changed = input();
    changed.body = "改变后的草稿".into();
    assert!(claim_once(&mut connection, changed.clone()).is_err());
    changed.message_id = Uuid::new_v4();
    assert!(claim_once(&mut connection, changed.clone()).is_err());
    changed.input_revision = Uuid::new_v4();
    let current = claimed(&mut connection, changed);
    acknowledge(&mut connection, &original);
    assert_eq!(
        read_record(&mut connection, current.delivery.message_id)
            .unwrap()
            .unwrap()
            .1,
        current
    );
}

#[test]
fn generic_task_and_message_paths_cannot_acknowledge_or_reset_bridge_ledger() {
    let mut connection = fixture();
    let record = claimed(&mut connection, input());
    assert!(
        update_message_state(
            &mut connection,
            &record.message.message_id,
            &record.delivery.task_id,
            1,
            LocalCliMessageState::Acknowledged
        )
        .is_err()
    );
    let mut queued = record.message.clone();
    queued.state = LocalCliMessageState::Queued;
    assert!(insert_message(&mut connection, queued.clone()).is_err());
    queued.message_id = Uuid::new_v4().to_string();
    queued.subject = "user_input".into();
    assert!(insert_message(&mut connection, queued).is_err());
    let mut failed = record.message.clone();
    failed.state = LocalCliMessageState::Failed;
    assert!(write_message_state(&mut connection, &failed).is_err());
    let mut task = read_task(&mut connection, &record.delivery.task_id)
        .unwrap()
        .unwrap();
    task.config_json = "{}".into();
    task.revision = 1;
    task.state = LocalCliTaskState::Cancelled;
    assert!(checkpoint(&mut connection, task, Some(1)).is_err());
    assert_eq!(
        read_record(&mut connection, record.delivery.message_id)
            .unwrap()
            .unwrap()
            .1,
        record
    );
}

#[test]
fn exact_ack_confirms_input_without_completing_model_task() {
    let mut connection = fixture();
    let record = claimed(&mut connection, input());
    let acknowledged = acknowledge(&mut connection, &record);
    assert_eq!(
        acknowledged.delivery.state,
        NativeBridgeDeliveryState::NativeAcknowledged
    );
    assert_eq!(
        acknowledged.message.state,
        LocalCliMessageState::Acknowledged
    );
    assert_eq!(acknowledge(&mut connection, &record), acknowledged);
    let task = read_task(&mut connection, &record.delivery.task_id)
        .unwrap()
        .unwrap();
    assert_eq!(task.state, LocalCliTaskState::Running);
    assert_eq!(task.result, None);
    assert_eq!(task.terminal_evidence, None);
    assert_eq!(
        claim_once(
            &mut connection,
            NativeBridgeInput {
                message_id: Uuid::new_v4(),
                ..input()
            }
        )
        .unwrap(),
        NativeBridgeClaimOutcome::Existing(acknowledged)
    );
}

fn assert_mismatched_receipt_rejected(field: &str, value: Value) {
    let mut connection = fixture();
    let record = claimed(&mut connection, input());
    let mut receipt = response(&record, "native_acknowledged");
    receipt["receipt"][field] = value;
    assert!(
        acknowledge_exact(
            &mut connection,
            record.delivery.clone(),
            input().binding,
            receipt
        )
        .is_err()
    );
    assert_eq!(
        read_record(&mut connection, record.delivery.message_id)
            .unwrap()
            .unwrap()
            .1,
        record
    );
}

#[test]
fn receipt_must_match_message_prompt_session_and_body_digest() {
    assert_mismatched_receipt_rejected("message_id", json!(Uuid::from_u128(70)));
    assert_mismatched_receipt_rejected("prompt_id", json!(Uuid::from_u128(71)));
    assert_mismatched_receipt_rejected("session_id", json!(Uuid::from_u128(72)));
    assert_mismatched_receipt_rejected("payload_digest", json!("blake3:wrong"));
}

#[test]
fn queued_dispatched_unknown_and_unbound_rejection_are_not_acknowledgements() {
    let mut connection = fixture();
    let record = claimed(&mut connection, input());
    assert_eq!(
        acknowledge_exact(
            &mut connection,
            record.delivery.clone(),
            input().binding,
            response(&record, "claimed")
        )
        .unwrap(),
        None
    );
    assert_eq!(
        acknowledge_exact(
            &mut connection,
            record.delivery.clone(),
            input().binding,
            response(&record, "dispatched")
        )
        .unwrap(),
        None
    );
    assert_eq!(
        acknowledge_exact(
            &mut connection,
            record.delivery.clone(),
            input().binding,
            response(&record, "unknown")
        )
        .unwrap(),
        None
    );
    assert_eq!(
        acknowledge_exact(
            &mut connection,
            record.delivery.clone(),
            input().binding,
            json!({"status":"unknown","message_id":record.delivery.message_id})
        )
        .unwrap(),
        None
    );
    assert_eq!(
        acknowledge_exact(
            &mut connection,
            record.delivery.clone(),
            input().binding,
            json!({"status":"rejected","reason":"draft_changed"})
        )
        .unwrap(),
        None
    );
    assert_eq!(
        read_record(&mut connection, record.delivery.message_id)
            .unwrap()
            .unwrap()
            .1,
        record
    );
}

#[test]
fn acknowledged_body_requires_explicit_replaces_and_lookup_returns_latest_chain_head() {
    let mut connection = fixture();
    let original = claimed(&mut connection, input());
    let ack = acknowledge(&mut connection, &original);
    let second_input = next_attempt(&ack);
    let second = claimed(&mut connection, second_input.clone());
    assert_eq!(second.delivery.replaces, Some(original.delivery.message_id));
    assert_eq!(
        lookup_exact(&mut connection, input().binding, None, &input().body).unwrap(),
        Some(second.clone())
    );
    let stale = NativeBridgeInput {
        message_id: Uuid::new_v4(),
        ..second_input
    };
    assert_eq!(
        claim_once(&mut connection, stale).unwrap(),
        NativeBridgeClaimOutcome::Existing(second.clone())
    );
    assert_eq!(
        read_record(&mut connection, original.delivery.message_id)
            .unwrap()
            .unwrap()
            .1,
        ack
    );
    assert!(claim_once(&mut connection, next_attempt(&second)).is_err());
}

#[test]
fn exact_native_rejection_is_persisted_and_allows_only_explicit_new_attempt() {
    let mut connection = fixture();
    let original = claimed(&mut connection, input());
    assert_eq!(
        acknowledge_exact(
            &mut connection,
            original.delivery.clone(),
            input().binding,
            response(&original, "native_rejected")
        )
        .unwrap(),
        None
    );
    let rejected = read_record(&mut connection, original.delivery.message_id)
        .unwrap()
        .unwrap()
        .1;
    assert_eq!(
        rejected.delivery.state,
        NativeBridgeDeliveryState::NativeRejected
    );
    assert_eq!(
        rejected.message.receipt_kind,
        Some(LocalCliReceiptKind::NativeProtocol)
    );
    assert_eq!(
        claim_once(
            &mut connection,
            NativeBridgeInput {
                message_id: Uuid::new_v4(),
                ..input()
            }
        )
        .unwrap(),
        NativeBridgeClaimOutcome::Existing(rejected.clone())
    );
    let new_record = claimed(&mut connection, next_attempt(&rejected));
    assert_eq!(
        new_record.delivery.state,
        NativeBridgeDeliveryState::Unknown
    );
    assert!(
        acknowledge_exact(
            &mut connection,
            original.delivery.clone(),
            input().binding,
            response(&original, "native_acknowledged")
        )
        .is_err()
    );
}

#[test]
fn not_sent_requires_exact_unknown_snapshot_and_preserves_prior_attempt() {
    let mut connection = fixture();
    let original = claimed(&mut connection, input());
    let mut old = original.delivery.clone();
    old.input_revision = Uuid::new_v4();
    assert!(mark_not_sent_exact(&mut connection, old).is_err());
    let not_sent = mark_not_sent_exact(&mut connection, original.delivery.clone())
        .unwrap()
        .unwrap();
    assert_eq!(not_sent.delivery.state, NativeBridgeDeliveryState::NotSent);
    assert_eq!(not_sent.message.state, LocalCliMessageState::Failed);
    assert_eq!(not_sent.message.receipt_kind, None);
    assert!(mark_not_sent_exact(&mut connection, original.delivery.clone()).is_err());
    let new_record = claimed(&mut connection, next_attempt(&not_sent));
    assert_eq!(
        new_record.delivery.replaces,
        Some(original.delivery.message_id)
    );
    assert_eq!(
        read_record(&mut connection, original.delivery.message_id)
            .unwrap()
            .unwrap()
            .1,
        not_sent
    );
}

#[test]
fn concurrent_explicit_replacements_grant_only_one_new_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite");
    let mut setup = connection(path.to_str().unwrap());
    setup
        .run_pending_migrations(::persistence::MIGRATIONS)
        .unwrap();
    let original = claimed(&mut setup, input());
    let previous = acknowledge(&mut setup, &original);
    drop(setup);
    let start = Arc::new(Barrier::new(3));
    let first_path = path.clone();
    let first_start = start.clone();
    let first_input = next_attempt(&previous);
    let first = thread::spawn(move || {
        let mut connection = connection(first_path.to_str().unwrap());
        first_start.wait();
        claim_once(&mut connection, first_input).unwrap()
    });
    let second_path = path.clone();
    let second_start = start.clone();
    let second_input = next_attempt(&previous);
    let second = thread::spawn(move || {
        let mut connection = connection(second_path.to_str().unwrap());
        second_start.wait();
        claim_once(&mut connection, second_input).unwrap()
    });
    start.wait();
    match (first.join().unwrap(), second.join().unwrap()) {
        (NativeBridgeClaimOutcome::Claimed(first), NativeBridgeClaimOutcome::Existing(second))
        | (NativeBridgeClaimOutcome::Existing(first), NativeBridgeClaimOutcome::Claimed(second)) => {
            assert_eq!(first, second)
        }
        (NativeBridgeClaimOutcome::Claimed(_), NativeBridgeClaimOutcome::Claimed(_))
        | (NativeBridgeClaimOutcome::Existing(_), NativeBridgeClaimOutcome::Existing(_)) => {
            panic!("显式重发只能产生一个新尝试")
        }
    }
    let mut connection = connection(path.to_str().unwrap());
    assert_eq!(native_records(&mut connection).unwrap().len(), 2);
}

#[test]
fn identity_snapshot_rejects_credentials_and_cannot_restore_authority() {
    let mut connection = fixture();
    let mut with_token = input();
    with_token.binding["token"] = json!("fixture-not-a-real-token");
    assert!(claim_once(&mut connection, with_token).is_err());
    let mut with_ready = input();
    with_ready.binding["native_process"]["ready"] = json!(true);
    assert!(claim_once(&mut connection, with_ready).is_err());
    assert!(native_records(&mut connection).unwrap().is_empty());
    assert!(read_tasks(&mut connection, false).unwrap().is_empty());
    let record = claimed(&mut connection, input());
    let NativeBridgeProcessSnapshot::Macos(process) = &record.delivery.binding.native_process
    else {
        panic!("旧 Mac 身份必须按原形状读取");
    };
    assert_eq!(process.resource_cid, 0);
    assert_eq!(
        record.delivery.binding.boot_session.as_deref(),
        Some("ABCDEFAB-1234-5678-9012-ABCDEF123456")
    );
}

#[test]
fn legacy_macos_task_identity_keeps_exact_serialized_bytes() {
    // 这是平台分型前实际使用的字段顺序；持久 task_id 必须继续绑定同一字节序列。
    let old = r#"[{"instance_id":"00000000-0000-0000-0000-000000000001","native_process":{"pid":101,"pid_version":2,"unique_id":3,"resource_cid":0},"boot_session":"ABCDEFAB-1234-5678-9012-ABCDEF123456","shell":{"pid":102,"pid_version":4,"unique_id":5,"resource_cid":0},"slave_device":6,"artifact_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"00000000-0000-0000-0000-000000000002"]"#;
    let input = input();
    let binding: NativeBridgeBindingSnapshot = serde_json::from_value(input.binding).unwrap();
    binding.validate().unwrap();
    assert_eq!(
        serde_json::to_string(&(&binding, input.session_id)).unwrap(),
        old
    );
    assert_eq!(
        task_id(&binding, input.session_id).unwrap(),
        format!(
            "grok-native-terminal:{}",
            blake3::hash(old.as_bytes()).to_hex()
        )
    );
}

fn linux_binding() -> Value {
    let mut binding = input().binding;
    let process = json!({"platform":"linux", "pid":101, "start_time_ticks":12,
        "proc_inode":13, "pid_namespace_device":14, "pid_namespace_inode":15,
        "uid":1000, "executable_device":16, "executable_inode":17});
    binding["native_process"] = process.clone();
    binding["shell"] = process;
    binding["shell"]["pid"] = json!(102);
    binding["shell"]["proc_inode"] = json!(18);
    binding
}

fn windows_binding() -> Value {
    let mut binding = input().binding;
    binding.as_object_mut().unwrap().remove("boot_session");
    binding.as_object_mut().unwrap().remove("slave_device");
    binding["terminal_generation"] = json!(Uuid::from_u128(25));
    let process = json!({"platform":"windows", "pid":101, "created_at":12,
        "logon_low":14, "logon_high":0, "session_id":1});
    binding["native_process"] = process.clone();
    binding["shell"] = process;
    binding["shell"]["pid"] = json!(102);
    binding["shell"]["created_at"] = json!(13);
    binding
}

fn valid_binding(value: Value) -> bool {
    serde_json::from_value::<NativeBridgeBindingSnapshot>(value)
        .is_ok_and(|binding| binding.validate().is_ok())
}

#[test]
fn platform_bindings_reject_mixed_or_incomplete_terminal_identity() {
    for original in [linux_binding(), windows_binding()] {
        assert!(valid_binding(original.clone()));
        for (pointer, value) in [
            ("/native_process/pid", json!(0)),
            ("/shell/pid", json!(0)),
            ("/native_process/platform", json!("macos")),
        ] {
            let mut binding = original.clone();
            *binding.pointer_mut(pointer).unwrap() = value;
            assert!(!valid_binding(binding));
        }
        let mut mixed = original.clone();
        mixed["shell"] = input().binding["shell"].clone();
        assert!(!valid_binding(mixed));
        let mut authority = original;
        authority["native_process"]["token"] = json!("不得恢复授权");
        assert!(!valid_binding(authority));
    }
    for field in ["uid", "pid_namespace_device", "pid_namespace_inode"] {
        let mut value = linux_binding();
        value["shell"][field] = json!(9999);
        assert!(!valid_binding(value));
    }
    for field in ["logon_low", "logon_high", "session_id"] {
        let mut value = windows_binding();
        value["shell"][field] = json!(9999);
        assert!(!valid_binding(value));
    }
    for field in ["boot_session", "slave_device"] {
        let mut linux = linux_binding();
        linux.as_object_mut().unwrap().remove(field);
        assert!(!valid_binding(linux));
        let mut windows = windows_binding();
        windows[field] = input().binding[field].clone();
        assert!(!valid_binding(windows));
    }
    let mut windows = windows_binding();
    windows["terminal_generation"] = json!(Uuid::nil());
    assert!(!valid_binding(windows));
    let mut linux = linux_binding();
    linux["terminal_generation"] = json!(Uuid::new_v4());
    assert!(!valid_binding(linux));
}

#[test]
fn linux_and_windows_ledger_recovery_never_regrants_transport() {
    for binding in [linux_binding(), windows_binding()] {
        let mut connection = fixture();
        let input = NativeBridgeInput { binding, ..input() };
        let record = claimed(&mut connection, input.clone());
        assert_eq!(record.delivery.state, NativeBridgeDeliveryState::Unknown);
        assert_eq!(
            claim_once(
                &mut connection,
                NativeBridgeInput {
                    input_revision: Uuid::new_v4(),
                    message_id: Uuid::new_v4(),
                    ..input.clone()
                }
            )
            .unwrap(),
            NativeBridgeClaimOutcome::Existing(record.clone())
        );
        assert_eq!(
            lookup_exact(
                &mut connection,
                input.binding,
                Some(input.session_id),
                &input.body
            )
            .unwrap(),
            Some(record.clone())
        );
        assert_eq!(
            acknowledge(&mut connection, &record).delivery.state,
            NativeBridgeDeliveryState::NativeAcknowledged
        );
    }
}

#[test]
fn internal_ledger_coexists_with_normal_session_task_and_preserves_recovery() {
    let mut connection = fixture();
    let internal = claimed(&mut connection, input());
    let task = LocalCliTask {
        version: 1,
        task_id: "normal-cli-task".into(),
        parent_task_id: None,
        parent_generation: None,
        harness: "grok".into(),
        working_directory: "/project".into(),
        config_json: "{}".into(),
        native_session_id: Some(input().session_id.to_string()),
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    checkpoint(&mut connection, task.clone(), None).unwrap();
    assert_eq!(read_tasks(&mut connection, false).unwrap(), vec![task]);
    let recovered = read_tasks(&mut connection, true).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].task_id, "normal-cli-task");
    assert_eq!(recovered[0].state, LocalCliTaskState::Disconnected);
    assert_eq!(
        read_record(&mut connection, internal.delivery.message_id)
            .unwrap()
            .unwrap()
            .1,
        internal
    );
}

#[test]
fn existing_owned_session_task_and_mailbox_coexist_with_native_input_ledger() {
    let mut connection = fixture();
    let owner = grok_terminal::GrokTerminalOwner {
        version: 1,
        launch_id: Uuid::from_u128(80),
        binding_id: Uuid::from_u128(81),
        session_id: input().session_id,
        input_revision: Uuid::from_u128(82),
        permission_revision: Uuid::from_u128(83),
        cli_version: "1.0.41".into(),
        model_id: "grok-4.7".into(),
        permission_mode: "default".into(),
    };
    let mut task = LocalCliTask {
        version: 1,
        task_id: "owned-cli-task".into(),
        parent_task_id: None,
        parent_generation: None,
        harness: "grok".into(),
        working_directory: "/project".into(),
        config_json: json!({"execution_kind":"grok_owned_terminal","grok_terminal":owner})
            .to_string(),
        native_session_id: Some(owner.session_id.to_string()),
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    checkpoint(&mut connection, task.clone(), None).unwrap();
    task.state = LocalCliTaskState::Running;
    task.revision = 1;
    checkpoint(&mut connection, task.clone(), Some(1)).unwrap();
    let internal = claimed(&mut connection, input());
    acknowledge(&mut connection, &internal);
    assert_eq!(
        read_task(&mut connection, &task.task_id).unwrap(),
        Some(task.clone())
    );
    assert_eq!(
        read_tasks(&mut connection, false).unwrap(),
        vec![task.clone()]
    );
    let message = LocalCliMessage {
        version: 1,
        message_id: Uuid::from_u128(84).to_string(),
        sender_task_id: task.task_id.clone(),
        recipient_task_id: task.task_id.clone(),
        sender_generation: 1,
        recipient_generation: 1,
        subject: grok_terminal::INPUT_SUBJECT.into(),
        body: "owned 会话的独立用户输入".into(),
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    };
    assert_eq!(
        insert_message(&mut connection, message.clone()).unwrap(),
        LocalCliEnqueueOutcome::Created
    );
    assert_eq!(
        read_task_messages(&mut connection, &task.task_id).unwrap(),
        vec![message]
    );
    assert_eq!(native_records(&mut connection).unwrap().len(), 1);
}
