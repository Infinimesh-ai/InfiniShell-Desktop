use super::super::super::cli_image_claude_queue::{
    ClaudePendingKind, ClaudeQueueImage, TranscriptPathGuard,
};
use super::*;
use std::sync::{Arc, Barrier};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    store: ReferenceStore,
    claim: QueueClaim,
    pending: ClaudePendingImageAttempt,
    key: Uuid,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let store = ReferenceStore::new(&root).unwrap();
        let image_directory = root.join("cli-image-unpublished-fixture");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&image_directory)
            .unwrap();
        let image_path = image_directory.join("attachment-fixture.png");
        let mut image = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&image_path)
            .unwrap();
        image.write_all(b"\x89PNG\r\n\x1a\n").unwrap();
        let key = Uuid::new_v4();
        let session = Uuid::new_v4();
        let reference = Reference {
            version: 1,
            host: "fixture-host".into(),
            native_session: session.to_string(),
            transfer_id: Uuid::new_v4(),
            key_hash: key_hash(key).unwrap(),
            directory: "cli-image-unpublished-fixture".into(),
            directory_identity: identity(&fs::metadata(&image_directory).unwrap()),
            filename: "attachment-fixture.png".into(),
            file_identity: identity(&image.metadata().unwrap()),
            byte_len: 8,
            sha256: Sha256::digest(b"\x89PNG\r\n\x1a\n").into(),
        };
        store.persist(&reference).unwrap();
        let transcript_path = root
            .join("projects/project")
            .join(format!("{session}.jsonl"));
        let pending = ClaudePendingImageAttempt {
            version: 1,
            kind: ClaudePendingKind::AwaitingTranscript,
            client_message_id: Uuid::new_v4(),
            session_id: session,
            request_sha256: [7; 32],
            anchor: TranscriptPathGuard::capture(&root, &transcript_path, session)
                .unwrap()
                .anchor,
            transcript_path,
            images: vec![ClaudeQueueImage {
                path: image_path,
                byte_len: 8,
                sha256: reference.sha256,
            }],
        };
        let claim = QueueClaim {
            version: 1,
            host: reference.host.clone(),
            native_session: session.to_string(),
            submission: pending.client_message_id,
            key_hash: key_hash(key).unwrap(),
            subject: [9; 32],
            native_request_sha256: pending.request_sha256,
            references: vec![(reference.transfer_id, key)],
            claude_recovery: Some(serde_json::json!({"attempt": pending})),
        };
        store.claim_queue(&claim).unwrap();
        Self {
            _directory: directory,
            root,
            store,
            claim,
            pending,
            key,
        }
    }

    fn create_history(&self) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(self.pending.transcript_path.parent().unwrap())
            .unwrap();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&self.pending.transcript_path)
            .unwrap();
        file.write_all(b"{}\n").unwrap();
    }

    fn status(&self) -> QueueResult {
        self.store
            .queue_status(
                &self.claim.host,
                &self.claim.native_session,
                self.claim.submission,
                self.key,
            )
            .unwrap()
            .unwrap()
    }
}

#[test]
fn missing_history_keeps_unknown_claim_and_published_image() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .store
            .bind_claude_transcript(&fixture.claim, &fixture.pending)
            .is_err()
    );
    assert_eq!(fixture.status().status, "unknown");
    assert!(fixture.pending.images[0].path.exists());
    assert!(!fixture.pending.transcript_path.parent().unwrap().exists());
    assert!(fixture.store.claim_queue(&fixture.claim).is_err());
}

#[test]
fn first_inode_is_durable_and_claim_remains_byte_for_byte_unchanged() {
    let fixture = Fixture::new();
    let path = fixture
        .store
        .root
        .join(format!("queue-{}.json", fixture.claim.submission));
    let before = fs::read(&path).unwrap();
    fixture.create_history();
    let (file, attempt, first) = fixture
        .store
        .bind_claude_transcript(&fixture.claim, &fixture.pending)
        .unwrap();
    assert_eq!(attempt.transcript_offset, 0);
    assert_eq!(first.observed_len, 3);
    assert_eq!(
        first.claim_sha256,
        <[u8; 32]>::from(Sha256::digest(serde_json::to_vec(&fixture.claim).unwrap()))
    );
    drop(file);
    let cold = ReferenceStore::new(&fixture.root).unwrap();
    let (_, recovered, second) = cold
        .bind_claude_transcript(&fixture.claim, &fixture.pending)
        .unwrap();
    assert_eq!(attempt.transcript_identity, recovered.transcript_identity);
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(fixture.status().status, "unknown");
}

#[test]
fn replaced_inode_cannot_be_chosen_again_after_cold_restart() {
    let fixture = Fixture::new();
    fixture.create_history();
    drop(
        fixture
            .store
            .bind_claude_transcript(&fixture.claim, &fixture.pending)
            .unwrap(),
    );
    fs::rename(
        &fixture.pending.transcript_path,
        fixture.root.join("old-history"),
    )
    .unwrap();
    fixture.create_history();
    let cold = ReferenceStore::new(&fixture.root).unwrap();
    assert!(
        cold.bind_claude_transcript(&fixture.claim, &fixture.pending)
            .is_err()
    );
    assert_eq!(fixture.status().status, "unknown");
}

#[test]
fn truncated_history_cannot_confirm_or_rebind() {
    let fixture = Fixture::new();
    fixture.create_history();
    drop(
        fixture
            .store
            .bind_claude_transcript(&fixture.claim, &fixture.pending)
            .unwrap(),
    );
    OpenOptions::new()
        .write(true)
        .open(&fixture.pending.transcript_path)
        .unwrap()
        .set_len(0)
        .unwrap();
    assert!(
        fixture
            .store
            .bind_claude_transcript(&fixture.claim, &fixture.pending)
            .is_err()
    );
    assert_eq!(fixture.status().status, "unknown");
}

#[test]
fn replaced_new_parent_is_rejected_even_when_original_file_inode_is_moved_back() {
    let fixture = Fixture::new();
    fixture.create_history();
    drop(
        fixture
            .store
            .bind_claude_transcript(&fixture.claim, &fixture.pending)
            .unwrap(),
    );
    fs::rename(
        fixture.root.join("projects"),
        fixture.root.join("old-projects"),
    )
    .unwrap();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(fixture.pending.transcript_path.parent().unwrap())
        .unwrap();
    fs::rename(
        fixture
            .root
            .join("old-projects/project")
            .join(fixture.pending.transcript_path.file_name().unwrap()),
        &fixture.pending.transcript_path,
    )
    .unwrap();
    assert!(
        fixture
            .store
            .bind_claude_transcript(&fixture.claim, &fixture.pending)
            .is_err()
    );
}

#[test]
fn sidecar_failure_keeps_unknown_and_rejects_changed_claim() {
    let fixture = Fixture::new();
    fixture.create_history();
    let mut changed = fixture.claim.clone();
    changed.subject = [1; 32];
    assert!(
        fixture
            .store
            .bind_claude_transcript(&changed, &fixture.pending)
            .is_err()
    );
    fs::DirBuilder::new()
        .mode(0o700)
        .create(fixture.store.root.join(format!(
            "queue-claude-transcript-{}.json",
            fixture.claim.submission
        )))
        .unwrap();
    assert!(
        fixture
            .store
            .bind_claude_transcript(&fixture.claim, &fixture.pending)
            .is_err()
    );
    assert_eq!(fixture.status().status, "unknown");
    assert!(fixture.pending.images[0].path.exists());
}

#[test]
fn concurrent_first_bindings_share_one_persisted_inode() {
    let fixture = Fixture::new();
    fixture.create_history();
    let barrier = Arc::new(Barrier::new(2));
    let root = fixture.root.clone();
    let claim = fixture.claim.clone();
    let pending = fixture.pending.clone();
    let other_barrier = barrier.clone();
    let other = std::thread::spawn(move || {
        let store = ReferenceStore::new(&root).unwrap();
        other_barrier.wait();
        store.bind_claude_transcript(&claim, &pending).unwrap().2
    });
    barrier.wait();
    let first = fixture
        .store
        .bind_claude_transcript(&fixture.claim, &fixture.pending)
        .unwrap()
        .2;
    let second = other.join().unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
    assert_eq!(fixture.status().status, "unknown");
}
