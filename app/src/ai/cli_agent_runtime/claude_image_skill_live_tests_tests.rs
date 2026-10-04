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
    events: impl FnOnce(Uuid) -> Vec<(Option<&'static str>, RuntimeEventKind)> + Send + 'static,
) -> (Result<(), String>, Vec<serde_json::Value>) {
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
    // 仅将内存事件送入真实验收循环；不启动 CLI、监督者、工具或模型。
    let fixture = tokio::spawn(async move {
        let command = submitted.recv().await.unwrap();
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
    // 空技能列表仅隔离本次 ACK 身份回归，不代表在线技能审批已获验收。
    let result = image_skill_turn(
        &mut session,
        vec![InputContent::Text("离线回执夹具".into())],
        "RED BLUE GREEN YELLOW",
        &[],
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
async fn queued_ack_waits_for_pairing_and_preserves_observed_identity() {
    let (result, receipts) = exercise_events(|id| {
        vec![
            (None, accepted(id)),
            (None, started(id)),
            (Some(NATIVE), paired()),
            (Some(NATIVE), finished(id)),
        ]
    })
    .await;

    assert_eq!(result, Ok(()));
    assert_eq!(receipts[1]["event"], "session_ready");
    assert_eq!(receipts[2]["event"], "accepted");
    assert_eq!(receipts[2]["native_session_id"], NATIVE);
    assert!(receipts[2]["native_session_id_at_observation"].is_null());
    assert_eq!(receipts[2]["identity_bound_after_pairing"], true);
    assert_eq!(receipts[3]["event"], "finished");
}

#[tokio::test]
async fn paired_ack_keeps_its_original_native_identity() {
    let (result, receipts) = exercise_events(|id| {
        vec![
            (Some(NATIVE), paired()),
            (Some(NATIVE), accepted(id)),
            (Some(NATIVE), started(id)),
            (Some(NATIVE), finished(id)),
        ]
    })
    .await;

    assert_eq!(result, Ok(()));
    assert_eq!(receipts[2]["native_session_id_at_observation"], NATIVE);
    assert_eq!(receipts[2]["identity_bound_after_pairing"], false);
}

#[tokio::test]
async fn unpaired_ack_cannot_complete_the_image_skill_acceptance() {
    let (result, receipts) = exercise_events(|id| {
        vec![
            (None, accepted(id)),
            (None, started(id)),
            (None, finished(id)),
        ]
    })
    .await;

    assert_eq!(result, Err("图片技能实际执行、识图或终态不匹配".into()));
    assert!(
        !receipts
            .iter()
            .any(|receipt| receipt["event"] == "accepted")
    );
}

#[tokio::test]
async fn duplicate_ack_before_pairing_is_rejected() {
    let (result, _) = exercise_events(|id| vec![(None, accepted(id)), (None, accepted(id))]).await;

    assert_eq!(result, Err("图片技能原生 ACK 重复或身份不匹配".into()));
}

#[tokio::test]
async fn ack_for_another_message_is_rejected() {
    let (result, _) = exercise_events(|_| vec![(None, accepted(Uuid::nil()))]).await;

    assert_eq!(result, Err("图片技能原生 ACK 重复或身份不匹配".into()));
}

#[tokio::test]
async fn ack_for_another_turn_is_rejected() {
    let (result, _) = exercise_events(|id| {
        vec![(
            None,
            RuntimeEventKind::MessageAccepted {
                message_id: id,
                turn_id: Some(Uuid::nil().to_string()),
            },
        )]
    })
    .await;

    assert_eq!(result, Err("图片技能原生 ACK 重复或身份不匹配".into()));
}

#[tokio::test]
async fn paired_native_identity_cannot_change_after_ack() {
    let (result, _) = exercise_events(|id| {
        vec![
            (None, accepted(id)),
            (Some(NATIVE), paired()),
            (Some(OTHER_NATIVE), started(id)),
        ]
    })
    .await;

    assert_eq!(result, Err("原生会话身份改变，不能视为成功恢复".into()));
}

#[tokio::test]
async fn another_cli_version_cannot_bind_the_ack() {
    let (result, _) = exercise_events(|id| {
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

    assert_eq!(result, Err("图片技能版本或原生会话配对不匹配".into()));
}
