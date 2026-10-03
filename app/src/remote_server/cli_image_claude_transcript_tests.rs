use super::super::ClaudeQueueImage;
use super::*;
use base64::Engine;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

struct Fixture {
    _directory: tempfile::TempDir,
    target: ClaudeInboxTarget,
    attempt: ClaudeImageAttempt,
    records: Vec<Value>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let session = Uuid::from_u128(1);
        let request = Uuid::from_u128(2);
        let context = Uuid::from_u128(3);
        let date = Uuid::from_u128(4);
        let remote_change = Uuid::from_u128(5);
        let snapshot = Uuid::from_u128(6);
        let assistant = Uuid::from_u128(7);
        let result = Uuid::from_u128(8);
        let transcript_path = root.join(format!("{session}.jsonl"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&transcript_path)
            .unwrap();
        let image_path = root.join("image.png");
        let image = STANDARD.decode(PNG).unwrap();
        let target = ClaudeInboxTarget {
            session_id: session,
            cwd: root.clone(),
            process_id: std::process::id(),
            socket_path: root.join("absent-native.sock"),
            registry_directory: root.join("sessions"),
            transcript_path: transcript_path.clone(),
        };
        let attempt = ClaudeImageAttempt {
            client_message_id: request,
            session_id: session,
            request_sha256: [7; 32],
            transcript_path,
            transcript_identity: identity(&file.metadata().unwrap()),
            transcript_offset: 0,
            images: vec![ClaudeQueueImage {
                path: image_path.clone(),
                byte_len: image.len() as u64,
                sha256: Sha256::digest(image).into(),
            }],
        };
        // 形状来自 2.1.280 首图实测第 8–14 行；UUID、路径及内容均为合成夹具。
        let records = vec![
            json!({"type": "user", "uuid": request, "parentUuid": null,
                "sessionId": session, "isSidechain": false,
                "origin": {"kind": "peer", "msg_id": request},
                "message": {"content": "读取图片"}}),
            json!({"type": "attachment", "uuid": context, "parentUuid": request,
                "sessionId": session, "isSidechain": false,
                "attachment": {"type": "session_context", "context": "合成上下文"}}),
            json!({"type": "attachment", "uuid": date, "parentUuid": context,
                "sessionId": session, "isSidechain": false,
                "attachment": {"type": "date", "date": "2026-10-01"}}),
            json!({"type": "attachment", "uuid": remote_change, "parentUuid": date,
                "sessionId": session, "isSidechain": false,
                "attachment": {"type": "remote_session_change"}}),
            json!({"type": "attachment", "uuid": snapshot, "parentUuid": remote_change,
                "sessionId": session, "isSidechain": false,
                "attachment": {"type": "prompt_snapshot", "systemPrompt": "合成夹具"}}),
            json!({"type": "assistant", "uuid": assistant, "parentUuid": snapshot,
                "sessionId": session, "isSidechain": false,
                "message": {"content": [{"type": "tool_use", "id": "read-image",
                    "name": "Read", "input": {"file_path": image_path}}]}}),
            json!({"type": "user", "uuid": result, "parentUuid": assistant,
                "sessionId": session, "isSidechain": false,
                "sourceToolAssistantUUID": assistant,
                "message": {"content": [{"type": "tool_result", "tool_use_id": "read-image",
                    "content": [{"type": "image", "source": {"type": "base64",
                        "media_type": "image/png", "data": PNG}}]}]}}),
        ];
        Self {
            _directory: directory,
            target,
            attempt,
            records,
        }
    }

    fn poll(&self) -> io::Result<Option<ClaudeImageReceipt>> {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.target.transcript_path)
            .unwrap();
        for record in &self.records {
            writeln!(file, "{record}").unwrap();
        }
        file.sync_all().unwrap();
        ClaudeTranscript::recover(&self.target, &self.attempt)?.poll_available()
    }
}

#[test]
fn native_attachment_ancestry_connects_exact_read_image_consumption() {
    let fixture = Fixture::new();
    let receipt = fixture.poll().unwrap().unwrap();

    assert_eq!(receipt.reads.len(), 1);
    assert_eq!(receipt.reads[0].assistant_message_uuid, Uuid::from_u128(7));
    assert_eq!(receipt.reads[0].result_message_uuid, Uuid::from_u128(8));
    assert_eq!(
        receipt.reads[0].image_sha256,
        fixture.attempt.images[0].sha256
    );
}

#[test]
fn attachment_content_cannot_supply_a_read_tool_call() {
    let mut fixture = Fixture::new();
    fixture.records[5]["type"] = json!("attachment");

    assert!(fixture.poll().unwrap().is_none());
}

#[test]
fn attachment_content_cannot_supply_an_image_tool_result() {
    let mut fixture = Fixture::new();
    fixture.records[6]["type"] = json!("attachment");

    assert!(fixture.poll().unwrap().is_none());
}

#[test]
fn missing_attachment_keeps_read_ancestry_disconnected() {
    let mut fixture = Fixture::new();
    fixture.records.remove(2);

    assert!(fixture.poll().unwrap().is_none());
}

#[test]
fn attachment_from_another_session_cannot_connect_read_ancestry() {
    let mut fixture = Fixture::new();
    fixture.records[2]["sessionId"] = json!(Uuid::from_u128(99));

    assert!(fixture.poll().unwrap().is_none());
}

#[test]
fn sidechain_attachment_cannot_connect_read_ancestry() {
    let mut fixture = Fixture::new();
    fixture.records[2]["isSidechain"] = json!(true);

    assert!(fixture.poll().unwrap().is_none());
}

#[test]
fn conflicting_attachment_uuid_rejects_consumption() {
    let mut fixture = Fixture::new();
    let mut conflicting = fixture.records[2].clone();
    conflicting["attachment"]["date"] = json!("2026-10-02");
    fixture.records.push(conflicting);

    assert_eq!(
        fixture.poll().err().unwrap().to_string(),
        "transcript_duplicate_uuid_conflict"
    );
}

#[test]
fn identical_attachment_uuid_replay_preserves_consumption() {
    let mut fixture = Fixture::new();
    fixture.records.push(fixture.records[2].clone());

    assert_eq!(fixture.poll().unwrap().unwrap().reads.len(), 1);
}

#[test]
fn attachment_cannot_replace_the_peer_request_root() {
    let mut fixture = Fixture::new();
    fixture.records[0]["type"] = json!("attachment");

    assert_eq!(
        fixture.poll().err().unwrap().to_string(),
        "transcript_request_origin_mismatch"
    );
}

#[test]
fn unsupported_system_record_cannot_bridge_read_ancestry() {
    let mut fixture = Fixture::new();
    fixture.records[2]["type"] = json!("system");

    assert!(fixture.poll().unwrap().is_none());
}
