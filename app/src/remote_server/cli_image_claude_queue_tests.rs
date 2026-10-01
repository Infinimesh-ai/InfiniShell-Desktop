use super::*;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

struct InboxFixture {
    _directory: tempfile::TempDir,
    target: ClaudeInboxTarget,
    image: ClaudeQueueImage,
}

impl InboxFixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let config = fs::canonicalize(directory.path()).unwrap();
        let registry_directory = config.join("sessions");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&registry_directory)
            .unwrap();
        let session_id = Uuid::new_v4();
        let target = ClaudeInboxTarget {
            session_id,
            cwd: config.clone(),
            process_id: std::process::id(),
            socket_path: config.join("native.sock"),
            registry_directory,
            transcript_path: config
                .join("projects/project")
                .join(format!("{session_id}.jsonl")),
        };
        let registry = json!({"pid": target.process_id, "messagingSocketPath": target.socket_path,
            "cwd": target.cwd, "sessionId": session_id, "procStart": "fixture-generation", "kind": "interactive"});
        private_file(
            &target
                .registry_directory
                .join(format!("{}.json", target.process_id)),
            &serde_json::to_vec(&registry).unwrap(),
        );
        let socket_sha = Sha256::digest(target.socket_path.to_str().unwrap().as_bytes());
        let key = json!({"peerToken": "0123456789abcdef0123456789abcdef", "procStart": "fixture-generation"});
        private_file(
            &target
                .registry_directory
                .join(format!("{}.{socket_sha:x}.key", target.process_id)),
            &serde_json::to_vec(&key).unwrap(),
        );
        let path = config.join("image.png");
        private_file(&path, b"\x89PNG\r\n\x1a\n");
        let image = ClaudeQueueImage {
            path,
            byte_len: 8,
            sha256: Sha256::digest(b"\x89PNG\r\n\x1a\n").into(),
        };
        Self {
            _directory: directory,
            target,
            image,
        }
    }

    fn connect(&self) -> (ClaudeImageInbox, UnixStream) {
        let (client, server) = UnixStream::pair().unwrap();
        let inbox = ClaudeImageInbox::connect(client, self.target.clone(), &|_| Ok(())).unwrap();
        (inbox, server)
    }
}

fn private_file(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

#[test]
fn first_image_sends_once_without_creating_native_history_or_parents() {
    let fixture = InboxFixture::new();
    let (inbox, mut peer) = fixture.connect();
    let message = Uuid::new_v4();
    let attempt = inbox
        .submit_once(
            message,
            "首图输入",
            &[fixture.image.clone()],
            |attempt| {
                assert!(matches!(attempt, ClaudeQueuedImageAttempt::Pending(_)));
                assert!(!fixture.target.transcript_path.parent().unwrap().exists());
                Ok(())
            },
            &|_| Ok(()),
        )
        .unwrap();
    let mut wire = String::new();
    peer.read_to_string(&mut wire).unwrap();
    let frames = wire
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1]["uuid"], message.to_string());
    assert_eq!(attempt.client_message_id(), message);
    assert!(!fixture.target.transcript_path.parent().unwrap().exists());
}

#[test]
fn failed_durable_claim_sends_no_auth_or_user_bytes() {
    let fixture = InboxFixture::new();
    let (inbox, mut peer) = fixture.connect();
    assert!(
        inbox
            .submit_once(
                Uuid::new_v4(),
                "首图",
                &[fixture.image.clone()],
                |_| Err(io::Error::other("fixture claim failure")),
                &|_| Ok(())
            )
            .is_err()
    );
    let mut wire = Vec::new();
    peer.read_to_end(&mut wire).unwrap();
    assert!(wire.is_empty());
}

#[test]
fn legacy_attempt_round_trips_without_new_fields_or_pending_fallback() {
    let fixture = InboxFixture::new();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(fixture.target.transcript_path.parent().unwrap())
        .unwrap();
    private_file(&fixture.target.transcript_path, b"{}\n");
    let (inbox, mut peer) = fixture.connect();
    let attempt = inbox
        .submit_once(
            Uuid::new_v4(),
            "既有历史",
            &[fixture.image.clone()],
            |_| Ok(()),
            &|_| Ok(()),
        )
        .unwrap();
    let ClaudeQueuedImageAttempt::Existing(existing) = &attempt else {
        panic!("必须保留旧类型")
    };
    assert_eq!(existing.transcript_offset, 3);
    assert!(
        ClaudeTranscript::recover(&fixture.target, existing)
            .unwrap()
            .poll_available()
            .unwrap()
            .is_none()
    );
    let old = serde_json::to_value(existing).unwrap();
    assert_eq!(serde_json::to_value(&attempt).unwrap(), old);
    let decoded: ClaudeQueuedImageAttempt = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), old);
    let mut damaged = old;
    damaged
        .as_object_mut()
        .unwrap()
        .remove("transcript_identity");
    assert!(serde_json::from_value::<ClaudeQueuedImageAttempt>(damaged).is_err());
    peer.read_to_end(&mut Vec::new()).unwrap();
}

#[test]
fn legacy_recovery_does_not_treat_replaced_history_as_pending() {
    let fixture = InboxFixture::new();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(fixture.target.transcript_path.parent().unwrap())
        .unwrap();
    private_file(&fixture.target.transcript_path, b"{}\n");
    let (inbox, mut peer) = fixture.connect();
    let attempt = inbox
        .submit_once(
            Uuid::new_v4(),
            "旧记录",
            &[fixture.image.clone()],
            |_| Ok(()),
            &|_| Ok(()),
        )
        .unwrap();
    let ClaudeQueuedImageAttempt::Existing(existing) = attempt else {
        panic!("必须保留旧类型")
    };
    fs::rename(
        &fixture.target.transcript_path,
        fixture.target.cwd.join("original-history"),
    )
    .unwrap();
    private_file(&fixture.target.transcript_path, b"{}\n");
    assert!(ClaudeTranscript::recover(&fixture.target, &existing).is_err());
    peer.read_to_end(&mut Vec::new()).unwrap();
}

#[test]
fn creation_during_claim_does_not_change_the_frozen_send_boundary() {
    let fixture = InboxFixture::new();
    let (inbox, mut peer) = fixture.connect();
    assert!(
        inbox
            .submit_once(
                Uuid::new_v4(),
                "首图",
                &[fixture.image.clone()],
                |_| {
                    fs::DirBuilder::new()
                        .recursive(true)
                        .mode(0o700)
                        .create(fixture.target.transcript_path.parent().unwrap())
                        .unwrap();
                    private_file(&fixture.target.transcript_path, b"{}\n");
                    Ok(())
                },
                &|_| Ok(())
            )
            .is_err()
    );
    let mut wire = Vec::new();
    peer.read_to_end(&mut wire).unwrap();
    assert!(wire.is_empty());
}
