use super::super::super::cli_image_claude_queue::{
    ClaudePendingImageAttempt, ClaudePendingKind, TranscriptPathGuard,
};
use super::super::super::cli_image_staging::RemoteImageSpec;
use super::super::super::proto::{
    CliImageClaudeBinding, CliImageQueueReference, CliImageStagingScope,
};
use super::*;
use command::blocking::Command;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::process::{Child, Stdio};

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    staging: RemoteImageStaging,
    scope: RemoteImageScope,
    target: ClaudeInboxTarget,
    pending: ClaudePendingImageAttempt,
    key: Uuid,
    transfer: Uuid,
}

impl Fixture {
    fn new() -> Self {
        let mut fixture = Self::unclaimed();
        fixture.claim(None);
        fixture
    }

    fn unclaimed() -> Self {
        let directory = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let session = Uuid::new_v4();
        let scope = RemoteImageScope {
            host_id: warp_core::HostId::new("fixture-host".into()),
            connection_id: Uuid::new_v4(),
            terminal_session_id: warp_core::SessionId::from(7),
            terminal_epoch: Uuid::new_v4(),
            cli_session_id: session.to_string(),
            input_generation: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
        };
        let mut staging = RemoteImageStaging::new(scope.host_id.clone(), &root, 1024).unwrap();
        staging.activate_scope(scope.clone()).unwrap();
        let transfer = Uuid::new_v4();
        let key = Uuid::new_v4();
        let spec = RemoteImageSpec {
            byte_len: 8,
            sha256: Sha256::digest(b"\x89PNG\r\n\x1a\n").into(),
        };
        staging.begin(&scope, transfer, spec).unwrap();
        staging
            .write_chunk(&scope, transfer, 0, b"\x89PNG\r\n\x1a\n")
            .unwrap();
        let (image_path, spec) = staging.publish(&scope, transfer, key).unwrap();
        let target = ClaudeInboxTarget {
            session_id: session,
            cwd: root.clone(),
            process_id: std::process::id(),
            socket_path: root.join("absent-native.sock"),
            registry_directory: root.join("sessions"),
            transcript_path: root
                .join("projects/project")
                .join(format!("{session}.jsonl")),
        };
        let pending = ClaudePendingImageAttempt {
            version: 1,
            kind: ClaudePendingKind::AwaitingTranscript,
            client_message_id: scope.submission_id,
            session_id: session,
            request_sha256: [7; 32],
            transcript_path: target.transcript_path.clone(),
            anchor: TranscriptPathGuard::capture(&root, &target.transcript_path, session)
                .unwrap()
                .anchor,
            images: vec![ClaudeQueueImage {
                path: image_path,
                byte_len: spec.byte_len,
                sha256: spec.sha256,
            }],
        };
        Self {
            _directory: directory,
            root,
            staging,
            scope,
            target,
            pending,
            key,
            transfer,
        }
    }

    fn claim(&mut self, lifetime: Option<NativeLifetime>) {
        let recovery = Recovery::new(
            &self.target,
            &ClaudeQueuedImageAttempt::Pending(self.pending.clone()),
            lifetime,
        );
        self.staging
            .claim_queue(&QueueClaim {
                version: 1,
                host: self.scope.host_id.as_str().into(),
                native_session: self.target.session_id.to_string(),
                submission: self.scope.submission_id,
                key_hash: Sha256::digest(self.key.as_bytes()).into(),
                subject: [9; 32],
                native_request_sha256: self.pending.request_sha256,
                references: vec![(self.transfer, self.key)],
                claude_recovery: Some(serde_json::to_value(recovery).unwrap()),
                tmux_recovery: None,
            })
            .unwrap();
    }

    fn write_history(&self, session: Uuid, image_data: &str) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(self.target.transcript_path.parent().unwrap())
            .unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&self.target.transcript_path)
            .unwrap();
        let message = self.scope.submission_id;
        let assistant = Uuid::new_v4();
        let user = serde_json::json!({"sessionId": session, "isSidechain": false, "type": "user", "uuid": message,
            "origin": {"kind": "peer", "msg_id": message}, "message": {"content": "fixture"}});
        let read = serde_json::json!({"sessionId": session, "isSidechain": false, "type": "assistant", "uuid": assistant,
            "parentUuid": message, "message": {"content": [{"type": "tool_use", "id": "read-image", "name": "Read", "input": {"file_path": self.pending.images[0].path}}]}});
        let result = serde_json::json!({"sessionId": session, "isSidechain": false, "type": "user", "uuid": Uuid::new_v4(),
            "parentUuid": assistant, "sourceToolAssistantUUID": assistant, "message": {"content": [{"type": "tool_result", "tool_use_id": "read-image", "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": image_data}}]}]}});
        writeln!(file, "{user}\n{read}\n{result}").unwrap();
        file.sync_all().unwrap();
    }

    fn recover(&mut self) -> CliImageCodexQueueResult {
        recover(
            &mut self.staging,
            &self.scope,
            self.scope.submission_id,
            self.key,
        )
        .unwrap()
    }
}

#[test]
fn pending_first_history_confirms_after_cold_restart_without_native_socket() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.recover().status, "unknown");
    fixture.write_history(fixture.target.session_id, "iVBORw0KGgo=");
    fixture.staging =
        RemoteImageStaging::new(fixture.scope.host_id.clone(), &fixture.root, 1024).unwrap();
    let result = fixture.recover();
    assert_eq!(result.status, "confirmed");
    assert_eq!(
        result.native_queue_id,
        fixture.scope.submission_id.to_string()
    );
    assert!(!fixture.pending.images[0].path.exists());
    assert!(!fixture.target.socket_path.exists());
    assert_eq!(fixture.recover().status, "confirmed");
}

#[test]
fn empty_first_inode_is_bound_before_delayed_consumption_and_cold_recovery() {
    let mut fixture = Fixture::new();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(fixture.target.transcript_path.parent().unwrap())
        .unwrap();
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&fixture.target.transcript_path)
        .unwrap();
    assert_eq!(fixture.recover().status, "unknown");
    let binding = fixture.root.join("cli-image-references-v1").join(format!(
        "queue-claude-transcript-{}.json",
        fixture.scope.submission_id
    ));
    let first_binding = fs::read(&binding).unwrap();
    fixture.write_history(fixture.target.session_id, "iVBORw0KGgo=");
    fixture.staging =
        RemoteImageStaging::new(fixture.scope.host_id.clone(), &fixture.root, 1024).unwrap();
    assert_eq!(fixture.recover().status, "confirmed");
    assert_eq!(fs::read(binding).unwrap(), first_binding);
    assert!(!fixture.target.socket_path.exists());
}

#[test]
fn another_session_history_cannot_release_the_original_image() {
    let mut fixture = Fixture::new();
    fixture.write_history(Uuid::new_v4(), "iVBORw0KGgo=");
    assert_eq!(fixture.recover().status, "unknown");
    assert!(fixture.pending.images[0].path.exists());
    assert!(!fixture.target.socket_path.exists());
}

#[test]
fn path_only_or_changed_image_bytes_do_not_count_as_consumption() {
    let mut fixture = Fixture::new();
    fixture.write_history(fixture.target.session_id, "bm90LXRoZS1waWN0dXJl");
    assert_eq!(fixture.recover().status, "unknown");
    assert!(fixture.pending.images[0].path.exists());
}

struct WaitingChild(Child);
impl WaitingChild {
    fn new() -> Self {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "printf x; IFS= read -r line || :"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = [0];
        child
            .stdout
            .as_mut()
            .unwrap()
            .read_exact(&mut ready)
            .unwrap();
        assert_eq!(ready, [b'x']);
        Self(child)
    }
    fn finish(&mut self) {
        self.0.stdin.take();
        assert!(self.0.wait().unwrap().success());
    }
}
impl Drop for WaitingChild {
    fn drop(&mut self) {
        self.0.stdin.take();
        let _ = self.0.wait();
    }
}

#[test]
fn natural_exit_retires_unconsumed_images_after_cold_recovery_without_claiming_consumption() {
    let mut child = WaitingChild::new();
    let mut fixture = Fixture::unclaimed();
    fixture.target.process_id = child.0.id();
    fixture.claim(Some(NativeLifetime::capture(child.0.id() as i32).unwrap()));
    assert_eq!(fixture.recover().status, "unknown");
    assert!(fixture.pending.images[0].path.exists());
    child.finish();
    fixture.staging =
        RemoteImageStaging::new(fixture.scope.host_id.clone(), &fixture.root, 1024).unwrap();

    let result = fixture.recover();
    assert_eq!(result.status, "retired");
    assert!(result.native_queue_id.is_empty());
    assert!(!fixture.pending.images[0].path.exists());
    assert!(!fixture.target.transcript_path.exists());
    assert_eq!(fixture.recover().status, "retired");
}

#[test]
fn legacy_claim_without_lifetime_is_not_retired_when_its_pid_disappears() {
    let mut child = WaitingChild::new();
    let mut fixture = Fixture::unclaimed();
    fixture.target.process_id = child.0.id();
    fixture.claim(None);
    child.finish();
    fixture.staging =
        RemoteImageStaging::new(fixture.scope.host_id.clone(), &fixture.root, 1024).unwrap();

    assert_eq!(fixture.recover().status, "unknown");
    assert!(fixture.pending.images[0].path.exists());
}

#[test]
fn exact_consumption_remains_confirmed_even_after_the_consumer_exits() {
    let mut child = WaitingChild::new();
    let mut fixture = Fixture::unclaimed();
    fixture.target.process_id = child.0.id();
    fixture.claim(Some(NativeLifetime::capture(child.0.id() as i32).unwrap()));
    fixture.write_history(fixture.target.session_id, "iVBORw0KGgo=");
    child.finish();

    assert_eq!(fixture.recover().status, "confirmed");
    assert!(!fixture.pending.images[0].path.exists());
}

#[test]
fn native_preflight_rejection_survives_lost_response_and_rejects_changed_identity() {
    let mut fixture = Fixture::unclaimed();
    let service = ImageStagingService::new(fixture.scope.host_id.clone(), &fixture.root).unwrap();
    let connection = ImageStagingConnection::new(fixture.scope.connection_id);
    let lease = SubmissionLease {
        scope: CliImageStagingScope {
            host_id: fixture.scope.host_id.as_str().into(),
            terminal_session_id: 7,
            cli_session_id: fixture.scope.cli_session_id.clone(),
            input_generation: fixture.scope.input_generation.to_string(),
            submission_id: fixture.scope.submission_id.to_string(),
            terminal_epoch: fixture.scope.terminal_epoch.to_string(),
        },
        revision: 1,
        live: AtomicBool::new(true),
        native_claimed: Mutex::new(false),
    };
    let mut digest = Sha256::new();
    digest.update(0u64.to_le_bytes());
    digest.update(8u64.to_le_bytes());
    digest.update(b"\x89PNG\r\n\x1a\n");
    let mut request = CliImageClaudeQueue {
        binding: Some(CliImageClaudeBinding {
            tmux_owned: None,
            process_id_candidate: 0,
            claude_config_directory: fixture.root.to_string_lossy().into_owned(),
            tty_path: "/dev/invalid".into(),
            working_directory: fixture.root.to_string_lossy().into_owned(),
            transcript_path: fixture
                .target
                .transcript_path
                .to_string_lossy()
                .into_owned(),
        }),
        text: String::new(),
        references: vec![CliImageQueueReference {
            transfer_id: fixture.transfer.to_string(),
            recovery_key: fixture.key.to_string(),
        }],
        subject_sha256: digest.finalize().to_vec(),
        recovery_key: fixture.key.to_string(),
    };
    // 不依赖返回帧，只从重新打开的持久存储恢复精确拒绝。
    submit(
        &service,
        &mut fixture.staging,
        &connection,
        &lease,
        &fixture.scope,
        &request,
    )
    .unwrap();
    assert!(!*lease.native_claimed.lock().unwrap());
    fixture.staging =
        RemoteImageStaging::new(fixture.scope.host_id.clone(), &fixture.root, 1024).unwrap();
    let result = fixture.recover();
    assert_eq!(result.status, "rejected");
    assert_eq!(result.subject_sha256, request.subject_sha256);
    assert!(!fixture.pending.images[0].path.exists());
    assert!(
        fixture
            .staging
            .queue_status(&fixture.scope, fixture.scope.submission_id, fixture.key)
            .unwrap()
            .is_none()
    );
    assert!(
        recover(
            &mut fixture.staging,
            &fixture.scope,
            fixture.scope.submission_id,
            Uuid::new_v4()
        )
        .is_err()
    );
    let mut other = fixture.scope.clone();
    other.cli_session_id = Uuid::new_v4().to_string();
    assert!(
        recover(
            &mut fixture.staging,
            &other,
            other.submission_id,
            fixture.key
        )
        .is_err()
    );
    request.subject_sha256 = vec![0; 32];
    assert!(
        submit(
            &service,
            &mut fixture.staging,
            &connection,
            &lease,
            &fixture.scope,
            &request
        )
        .is_err()
    );
}
