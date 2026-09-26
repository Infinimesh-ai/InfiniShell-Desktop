use super::*;
use serde_json::json;
use std::sync::Barrier;

fn lease() -> GrokOwnedInputLease {
    GrokOwnedInputLease::new(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()).unwrap()
}

#[test]
fn concurrent_clones_cannot_authorize_two_socket_writes() {
    let input = lease();
    let other = input.clone();
    let barrier = Arc::new(Barrier::new(2));
    let ready = barrier.clone();
    let worker = thread::spawn(move || {
        ready.wait();
        other.claim_write().is_ok()
    });
    barrier.wait();
    let own = input.claim_write().is_ok();
    let raced = worker.join().unwrap();
    assert_ne!(own, raced);
    assert!(matches!(
        input.claim_write(),
        Err(GrokLeaderInputError::StaleBinding)
    ));
}

#[test]
fn revoked_input_cannot_regain_write_authorization() {
    let input = lease();
    let other = input.clone();
    input.revoke();
    assert!(other.revoked());
    assert!(matches!(
        other.claim_write(),
        Err(GrokLeaderInputError::StaleBinding)
    ));
    other.revoke();
    assert!(matches!(
        input.claim_write(),
        Err(GrokLeaderInputError::StaleBinding)
    ));
}

#[test]
fn revocation_after_claim_never_makes_an_unknown_input_retryable() {
    let input = lease();
    input.claim_write().unwrap();
    input.revoke();
    assert!(matches!(
        input.claim_write(),
        Err(GrokLeaderInputError::StaleBinding)
    ));
}

#[test]
fn dropping_worker_only_cancels_its_sidecar_handle() {
    let disconnected = Arc::new(AtomicBool::new(false));
    let input = lease();
    let handle = GrokOwnedWorker {
        disconnected: disconnected.clone(),
    };
    drop(handle);
    assert!(disconnected.load(Ordering::SeqCst));
    assert!(!input.revoked());
}

fn snapshot() -> (LocalCliTask, LocalCliMessage, GrokTerminalOwner) {
    let owner = GrokTerminalOwner {
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
    let task = LocalCliTask {
        version: 1,
        task_id: Uuid::new_v4().to_string(),
        parent_task_id: None,
        parent_generation: None,
        harness: "grok".into(),
        working_directory: "/private/tmp/中文 项目".into(),
        config_json: json!({"execution_kind":"grok_owned_terminal","grok_terminal":owner})
            .to_string(),
        native_session_id: Some(owner.session_id.to_string()),
        generation: 1,
        revision: 2,
        state: LocalCliTaskState::Running,
        result: None,
        terminal_evidence: None,
    };
    let message = LocalCliMessage {
        version: 1,
        message_id: Uuid::new_v4().to_string(),
        sender_task_id: task.task_id.clone(),
        recipient_task_id: task.task_id.clone(),
        sender_generation: 1,
        recipient_generation: 1,
        subject: grok_terminal::INPUT_SUBJECT.into(),
        body: "请读取 /private/tmp/中文 项目/附件.txt".into(),
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    };
    (task, message, owner)
}

#[test]
fn stale_editor_and_permission_revisions_cannot_start_worker() {
    let (task, message, owner) = snapshot();
    let cwd = Path::new("/private/tmp/中文 项目");
    assert!(input_snapshot_matches(&task, &message, &owner, cwd));
    let mut edited = owner.clone();
    edited.input_revision = Uuid::new_v4();
    assert!(!input_snapshot_matches(&task, &message, &edited, cwd));
    let mut permission = owner.clone();
    permission.permission_revision = Uuid::new_v4();
    assert!(!input_snapshot_matches(&task, &message, &permission, cwd));
    assert!(!input_snapshot_matches(
        &task,
        &message,
        &owner,
        Path::new("/private/tmp/其他 项目")
    ));
}

#[test]
fn another_session_or_parent_child_message_cannot_use_owned_input_path() {
    let (mut task, mut message, owner) = snapshot();
    let cwd = Path::new("/private/tmp/中文 项目");
    message.sender_task_id = Uuid::new_v4().to_string();
    assert!(!input_snapshot_matches(&task, &message, &owner, cwd));
    message.sender_task_id = task.task_id.clone();
    task.native_session_id = Some(Uuid::new_v4().to_string());
    assert!(!input_snapshot_matches(&task, &message, &owner, cwd));
    task.native_session_id = Some(owner.session_id.to_string());
    task.parent_task_id = Some(Uuid::new_v4().to_string());
    task.parent_generation = Some(1);
    assert!(!input_snapshot_matches(&task, &message, &owner, cwd));
}

#[test]
fn dispatched_or_old_generation_message_cannot_start_worker() {
    let (task, mut message, owner) = snapshot();
    let cwd = Path::new("/private/tmp/中文 项目");
    message.state = LocalCliMessageState::Sent;
    assert!(!input_snapshot_matches(&task, &message, &owner, cwd));
    message.state = LocalCliMessageState::Unknown;
    assert!(!input_snapshot_matches(&task, &message, &owner, cwd));
    message.state = LocalCliMessageState::Queued;
    message.recipient_generation = 2;
    assert!(!input_snapshot_matches(&task, &message, &owner, cwd));
}

#[test]
fn incompatible_version_and_permission_policy_cannot_match_fixed_owner() {
    let (mut task, message, owner) = snapshot();
    let cwd = Path::new("/private/tmp/中文 项目");
    let mut changed = owner.clone();
    changed.cli_version = "1.0.42".into();
    task.config_json =
        json!({"execution_kind":"grok_owned_terminal","grok_terminal":changed}).to_string();
    assert!(!input_snapshot_matches(&task, &message, &owner, cwd));
    changed = owner.clone();
    changed.permission_mode = "bypassPermissions".into();
    task.config_json =
        json!({"execution_kind":"grok_owned_terminal","grok_terminal":changed}).to_string();
    assert!(!input_snapshot_matches(&task, &message, &owner, cwd));
}

#[test]
fn closing_editor_before_write_revokes_all_lease_clones() {
    let input = lease();
    let worker = input.clone();
    input.revoke_before_write();
    assert!(worker.revoked());
    assert!(matches!(
        worker.claim_write(),
        Err(GrokLeaderInputError::StaleBinding)
    ));
}

#[test]
fn native_permission_ui_can_close_editor_after_write_without_losing_receipt() {
    let input = lease();
    input.claim_write().unwrap();
    input.revoke_before_write();
    assert!(!input.revoked());
    assert!(matches!(
        input.claim_write(),
        Err(GrokLeaderInputError::StaleBinding)
    ));
    input.revoke();
    assert!(input.revoked());
}
