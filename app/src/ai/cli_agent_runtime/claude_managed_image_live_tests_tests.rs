use tokio::sync::mpsc;

use super::*;
use crate::ai::cli_agent_runtime::{RuntimeController, RuntimeEvent};

const NATIVE: &str = "00000000-0000-4000-8000-000000000007";
const OTHER_NATIVE: &str = "00000000-0000-4000-8000-000000000008";

fn accepted(id: Uuid) -> RuntimeEventKind {
    RuntimeEventKind::MessageAccepted {
        message_id: id,
        turn_id: Some(id.to_string()),
    }
}

fn paired() -> RuntimeEventKind {
    RuntimeEventKind::SessionReady {
        verified_cli_version: Some("2.1.280".into()),
        effective_permissions: json!({"permissionMode":"default"}),
    }
}

fn started(id: Uuid) -> RuntimeEventKind {
    RuntimeEventKind::TurnStarted {
        turn_id: id.to_string(),
    }
}

fn finished(id: Uuid) -> RuntimeEventKind {
    RuntimeEventKind::TurnFinished {
        turn_id: id.to_string(),
        outcome: TurnOutcome::Completed,
        output: "RED BLUE GREEN YELLOW".into(),
    }
}

async fn exercise_events(
    duplicate: bool,
    events: impl FnOnce(Uuid) -> Vec<(Option<&'static str>, RuntimeEventKind)> + Send + 'static,
) -> (Result<(), String>, Vec<Value>) {
    let root = tempfile::tempdir().unwrap();
    let evidence_path = root.path().join("events.ndjson");
    let mut evidence = Evidence {
        file: File::create(&evidence_path).unwrap(),
        root: root.path().to_owned(),
    };
    let generation = Uuid::from_u128(1);
    let (commands, mut submitted) = mpsc::channel(4);
    let (sender, receiver) = mpsc::channel(16);
    let mut session = LiveSession {
        generation,
        controller: RuntimeController {
            generation,
            commands,
            host_event_acks: None,
        },
        events: receiver,
        task: None,
        native_id: None,
        expected_native_id: None,
    };
    // 只向真实验收循环输入内存事件，不启动 CLI、监督者或模型。
    let fixture = tokio::spawn(async move {
        let command = submitted.recv().await.unwrap();
        if duplicate {
            let replay = submitted.recv().await.unwrap();
            assert_eq!(replay.message_id, command.message_id);
            assert_eq!(replay.action, command.action);
        }
        for (native, kind) in events(command.message_id) {
            sender
                .send(RuntimeEvent {
                    generation,
                    native_session_id: native.map(str::to_owned),
                    kind,
                })
                .await
                .unwrap();
        }
    });
    let result = turn(
        &mut session,
        "2.1.280",
        "image",
        vec![InputContent::Text("离线回执夹具".into())],
        "RED BLUE GREEN YELLOW",
        duplicate,
        &mut None,
        &mut evidence,
    )
    .await;
    fixture.await.unwrap();
    let receipts = fs::read_to_string(evidence_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (result, receipts)
}

#[tokio::test]
async fn queued_ack_waits_for_version_pairing_before_confirming_identity() {
    let (result, receipts) = exercise_events(false, |id| {
        vec![
            (None, accepted(id)),
            (None, started(id)),
            (Some(NATIVE), paired()),
            (Some(NATIVE), finished(id)),
        ]
    })
    .await;

    assert_eq!(result, Ok(()));
    assert_eq!(receipts[2]["event"], "message_accepted");
    assert_eq!(receipts[2]["native_session_id"], NATIVE);
    assert_eq!(receipts[2]["native_session_id_at_observation"], Value::Null);
    assert_eq!(receipts[2]["identity_bound_after_pairing"], true);
    assert_eq!(receipts[3]["event"], "turn_started");
    assert_eq!(receipts[3]["identity_bound_after_pairing"], true);
    assert_eq!(receipts[4]["event"], "turn_finished");
}

#[tokio::test]
async fn unpaired_native_ack_cannot_complete_the_acceptance() {
    let (result, receipts) = exercise_events(false, |id| {
        vec![
            (None, accepted(id)),
            (None, started(id)),
            (None, finished(id)),
        ]
    })
    .await;

    assert_eq!(result, Err("completion_before_native_pairing".into()));
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0]["event"], "input_submitted");
}

#[tokio::test]
async fn pairing_with_another_fixed_version_is_rejected() {
    let (result, _) = exercise_events(false, |id| {
        vec![
            (None, accepted(id)),
            (
                Some(NATIVE),
                RuntimeEventKind::SessionReady {
                    verified_cli_version: Some("2.1.278".into()),
                    effective_permissions: json!({"permissionMode":"default"}),
                },
            ),
        ]
    })
    .await;

    assert_eq!(
        result,
        Err("native_version_or_session_pairing_failed".into())
    );
}

#[tokio::test]
async fn duplicate_native_ack_without_retransmission_is_rejected() {
    let (result, _) =
        exercise_events(false, |id| vec![(None, accepted(id)), (None, accepted(id))]).await;

    assert_eq!(result, Err("wrong_or_duplicate_native_ack".into()));
}

#[tokio::test]
async fn native_ack_for_another_message_is_rejected() {
    let (result, _) = exercise_events(false, |_| vec![(None, accepted(Uuid::nil()))]).await;

    assert_eq!(result, Err("wrong_or_duplicate_native_ack".into()));
}

#[tokio::test]
async fn native_ack_for_another_turn_is_rejected() {
    let (result, _) = exercise_events(false, |id| {
        vec![(
            None,
            RuntimeEventKind::MessageAccepted {
                message_id: id,
                turn_id: Some(Uuid::nil().to_string()),
            },
        )]
    })
    .await;

    assert_eq!(result, Err("wrong_or_duplicate_native_ack".into()));
}

#[tokio::test]
async fn one_cached_ack_before_pairing_preserves_observed_identity() {
    let (result, receipts) = exercise_events(true, |id| {
        vec![
            (None, accepted(id)),
            (None, accepted(id)),
            (Some(NATIVE), paired()),
            (Some(NATIVE), started(id)),
            (Some(NATIVE), finished(id)),
        ]
    })
    .await;

    assert_eq!(result, Ok(()));
    assert_eq!(receipts[3]["event"], "cached_native_ack_replayed");
    assert_eq!(receipts[3]["native_session_id"], NATIVE);
    assert_eq!(receipts[3]["native_session_id_at_observation"], Value::Null);
    assert_eq!(receipts[3]["identity_bound_after_pairing"], true);
}

#[tokio::test]
async fn cached_ack_after_pairing_keeps_its_original_identity() {
    let (result, receipts) = exercise_events(true, |id| {
        vec![
            (None, accepted(id)),
            (Some(NATIVE), paired()),
            (Some(NATIVE), accepted(id)),
            (Some(NATIVE), started(id)),
            (Some(NATIVE), finished(id)),
        ]
    })
    .await;

    assert_eq!(result, Ok(()));
    assert_eq!(receipts[3]["event"], "cached_native_ack_replayed");
    assert_eq!(receipts[3]["native_session_id_at_observation"], NATIVE);
    assert_eq!(receipts[3]["identity_bound_after_pairing"], false);
}

#[tokio::test]
async fn a_second_cached_native_ack_is_rejected_before_pairing() {
    let (result, _) = exercise_events(true, |id| {
        vec![
            (None, accepted(id)),
            (None, accepted(id)),
            (None, accepted(id)),
        ]
    })
    .await;

    assert_eq!(result, Err("wrong_or_repeated_cached_native_ack".into()));
}

#[tokio::test]
async fn paired_native_identity_cannot_change_after_the_ack() {
    let (result, _) = exercise_events(false, |id| {
        vec![
            (None, accepted(id)),
            (Some(NATIVE), paired()),
            (Some(OTHER_NATIVE), started(id)),
        ]
    })
    .await;

    assert_eq!(result, Err("runtime_identity_or_stream_failed".into()));
}
