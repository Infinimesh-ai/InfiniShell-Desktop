use super::super::cli_image_staging::TmuxImageRecovery;
use super::super::proto::{
    CliImageCodexQueueStatus, CliImageStagingRecover, TerminalBindingOwnedAgent,
};
use super::*;
use std::fs;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::PathBuf;

struct Fixture {
    _parent: tempfile::TempDir,
    service: ImageStagingService,
    tmux: Arc<super::super::tmux_owned::Service>,
    connection: ImageStagingConnection,
    scope: CliImageStagingScope,
    claim: QueueClaim,
    queue_key: Uuid,
    transfer: Uuid,
    transfer_key: Uuid,
    image: PathBuf,
}

impl Fixture {
    fn new(associated: bool, claimed: bool, confirmed: bool) -> Self {
        let parent = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let host = HostId::new("current-daemon".into());
        let connection = ImageStagingConnection::new(Uuid::new_v4());
        connection.initialize();
        let epoch = Uuid::new_v4();
        connection.bootstrap(SessionId::from(8), &epoch.to_string(), 1);
        let scope = CliImageStagingScope {
            host_id: host.as_str().into(),
            terminal_session_id: 8,
            terminal_epoch: epoch.to_string(),
            cli_session_id: Uuid::new_v4().to_string(),
            input_generation: Uuid::new_v4().to_string(),
            submission_id: Uuid::new_v4().to_string(),
        };
        let mut prior = parse_scope(Some(&scope), connection.id, &host).unwrap();
        prior.host_id = HostId::new("publishing-daemon".into());
        let mut staging =
            RemoteImageStaging::new(prior.host_id.clone(), parent.path(), 1024).unwrap();
        staging.activate_scope(prior.clone()).unwrap();
        let transfer = Uuid::new_v4();
        let transfer_key = Uuid::new_v4();
        let bytes = b"\x89PNG\r\n\x1a\n";
        staging
            .begin(
                &prior,
                transfer,
                RemoteImageSpec {
                    byte_len: bytes.len() as u64,
                    sha256: Sha256::digest(bytes).into(),
                },
            )
            .unwrap();
        staging.write_chunk(&prior, transfer, 0, bytes).unwrap();
        let (image, _) = staging.publish(&prior, transfer, transfer_key).unwrap();
        let launch_key = Uuid::new_v4();
        let recovery = TmuxImageRecovery {
            version: 1,
            launch_id: Uuid::new_v4(),
            launch_key_sha256: super::super::cli_image_grok_launch::digest(launch_key.as_bytes()),
            agent: TerminalBindingOwnedAgent::Codex as i32,
            original_host: "launching-daemon".into(),
            original_terminal_session: 3,
        };
        let queue_key = Uuid::new_v4();
        let claim = QueueClaim {
            version: 1,
            host: prior.host_id.as_str().into(),
            native_session: prior.cli_session_id.clone(),
            submission: prior.submission_id,
            key_hash: Sha256::digest(queue_key.as_bytes()).into(),
            subject: [1; 32],
            native_request_sha256: [2; 32],
            references: vec![(transfer, transfer_key)],
            claude_recovery: None,
            tmux_recovery: associated.then_some(recovery.clone()),
        };
        if claimed {
            staging.claim_queue(&claim).unwrap();
            if confirmed {
                staging
                    .finish_queue(
                        &prior,
                        &QueueResult {
                            claim: claim.clone(),
                            status: "confirmed".into(),
                            native_queue_id: "native-ack".into(),
                            native_ack_sha256: Some([3; 32]),
                        },
                    )
                    .unwrap();
            }
        }
        drop(staging);

        let codex = Arc::new(
            super::super::cli_image_codex_owned::Service::without_reaper_for_test(
                host.as_str().into(),
                parent.path(),
            )
            .unwrap(),
        );
        let grok = Arc::new(
            super::super::cli_image_grok::Service::without_reaper_for_test(
                host.as_str().into(),
                parent.path(),
            )
            .unwrap(),
        );
        let tmux = Arc::new(super::super::tmux_owned::Service::new(
            host.as_str().into(),
            parent.path(),
            codex,
            grok,
        ));
        let root = parent.path().join("tmux-owned-v1");
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let directory = root.join(recovery.launch_id.to_string());
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        // 只用于私有记录恢复；故意没有存活 pane 或授权表，不能用来执行原生输入。
        let source = serde_json::json!({
            "version":1,
            "server":{"token":{"pid":101,"boot":"fixture","values":[1,2,3]},"uid":501,"executable":"/fixture/tmux","executable_file":[1,2]},
            "pane":{"token":{"pid":102,"boot":"fixture","values":[4,5,6]},"uid":501,"executable":"/fixture/shell","executable_file":[1,3]},
            "session_id":"$1","window_id":"@1","pane_id":"%1","pane_session":102,
            "tty":"/fixture/tty","tty_file":[1,4,5]
        });
        super::super::cli_image_codex_owned_launch::write_new(&directory.join("claim.json"), &serde_json::json!({
            "version":1,"id":recovery.launch_id,"key_sha256":recovery.launch_key_sha256,
            "agent":recovery.agent,"host":recovery.original_host,
            "terminal_session":recovery.original_terminal_session,"terminal_epoch":Uuid::new_v4(),"source":source
        })).unwrap();
        let service = ImageStagingService::new(host, parent.path())
            .unwrap()
            .with_tmux_owned(Some(tmux.clone()));
        Self {
            _parent: parent,
            service,
            tmux,
            connection,
            scope,
            claim,
            queue_key,
            transfer,
            transfer_key,
            image,
        }
    }

    fn status(&self, key: Uuid, session: &str) -> CliImageStagingResponse {
        let mut scope = self.scope.clone();
        scope.cli_session_id = session.into();
        self.service.handle(
            &self.connection,
            CliImageStagingRequest {
                scope: Some(scope),
                revision: 1,
                action: Some(cli_image_staging_request::Action::CodexQueueStatus(
                    CliImageCodexQueueStatus {
                        submission_id: self.claim.submission.to_string(),
                        recovery_key: key.to_string(),
                    },
                )),
            },
        )
    }

    fn release(&self, key: Uuid) -> CliImageStagingResponse {
        self.service.handle(
            &self.connection,
            CliImageStagingRequest {
                scope: Some(self.scope.clone()),
                revision: 1,
                action: Some(cli_image_staging_request::Action::Recover(
                    CliImageStagingRecover {
                        transfer_id: self.transfer.to_string(),
                        recovery_key: key.to_string(),
                        release: true,
                    },
                )),
            },
        )
    }
}

#[test]
fn tmux_ack_survives_daemon_restart_without_a_live_pane_or_input_authorization() {
    let fixture = Fixture::new(true, true, true);
    let response = fixture.status(fixture.queue_key, &fixture.scope.cli_session_id);
    let Some(cli_image_staging_response::Result::CodexQueue(result)) = response.result else {
        panic!("原提交 ACK 应可恢复")
    };
    assert_eq!(result.status, "confirmed");
    let scope = parse_scope(
        Some(&fixture.scope),
        fixture.connection.id,
        &fixture.service.host_id,
    )
    .unwrap();
    assert!(
        fixture
            .tmux
            .image_guard(
                &tmux_scope(&scope),
                fixture.claim.tmux_recovery.as_ref().unwrap().launch_id,
                TerminalBindingOwnedAgent::Codex,
            )
            .is_err()
    );
    assert!(
        fixture
            .service
            .state
            .lock()
            .unwrap()
            .registrations
            .is_empty()
    );
}

#[test]
fn tmux_reference_recovery_uses_exact_prior_submission_after_pane_exit() {
    let fixture = Fixture::new(true, true, false);
    assert!(fixture.image.exists());
    assert!(matches!(
        fixture.release(Uuid::new_v4()).result,
        Some(cli_image_staging_response::Result::Error(_))
    ));
    assert!(fixture.image.exists());
    assert!(
        matches!(fixture.release(fixture.transfer_key).result, Some(cli_image_staging_response::Result::Recovered(value)) if value.released)
    );
    assert!(!fixture.image.exists());
    assert!(
        matches!(fixture.release(fixture.transfer_key).result, Some(cli_image_staging_response::Result::Recovered(value)) if value.released)
    );
}

#[test]
fn tmux_ack_recovery_refuses_wrong_key_session_agent_and_connection_scope() {
    let fixture = Fixture::new(true, true, true);
    assert!(matches!(
        fixture
            .status(Uuid::new_v4(), &fixture.scope.cli_session_id)
            .result,
        Some(cli_image_staging_response::Result::Error(_))
    ));
    assert!(matches!(
        fixture
            .status(fixture.queue_key, &Uuid::new_v4().to_string())
            .result,
        Some(cli_image_staging_response::Result::Error(_))
    ));
    let scope = parse_scope(
        Some(&fixture.scope),
        fixture.connection.id,
        &fixture.service.host_id,
    )
    .unwrap();
    assert!(
        fixture
            .service
            .recovered_scope(
                &scope,
                Some(&fixture.claim),
                Some(TerminalBindingOwnedAgent::Claude)
            )
            .is_err()
    );
    let mut wrong = fixture.claim.clone();
    wrong.tmux_recovery.as_mut().unwrap().agent = TerminalBindingOwnedAgent::Claude as i32;
    assert!(
        fixture
            .service
            .recovered_scope(&scope, Some(&wrong), Some(TerminalBindingOwnedAgent::Codex))
            .is_err()
    );
    fixture.connection.revoke();
    assert!(matches!(
        fixture
            .status(fixture.queue_key, &fixture.scope.cli_session_id)
            .result,
        Some(cli_image_staging_response::Result::Error(_))
    ));
}

#[test]
fn unassociated_queue_and_unclaimed_upload_cannot_gain_cross_daemon_recovery() {
    let legacy = Fixture::new(false, true, false);
    assert!(matches!(
        legacy
            .status(legacy.queue_key, &legacy.scope.cli_session_id)
            .result,
        Some(cli_image_staging_response::Result::Error(_))
    ));
    assert!(matches!(
        legacy.release(legacy.transfer_key).result,
        Some(cli_image_staging_response::Result::Error(_))
    ));
    assert!(legacy.image.exists());
    let upload = Fixture::new(true, false, false);
    assert!(matches!(
        upload.release(upload.transfer_key).result,
        Some(cli_image_staging_response::Result::Error(_))
    ));
    assert!(upload.image.exists());
}

#[test]
fn legacy_queue_claim_decodes_without_granting_tmux_recovery() {
    let fixture = Fixture::new(false, true, false);
    let mut encoded = serde_json::to_value(&fixture.claim).unwrap();
    encoded.as_object_mut().unwrap().remove("tmux_recovery");
    let decoded: QueueClaim = serde_json::from_value(encoded).unwrap();
    assert!(decoded.tmux_recovery.is_none());
    assert!(decoded == fixture.claim);
}
