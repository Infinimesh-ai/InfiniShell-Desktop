use super::super::proto::{
    CliImageStagingActivate, CliImageStagingBegin, CliImageStagingChunk, CliImageStagingVerify,
};
use super::*;
use sha2::{Digest, Sha256};

fn request(
    session: u64,
    revision: u64,
    action: cli_image_staging_request::Action,
) -> CliImageStagingRequest {
    CliImageStagingRequest {
        scope: Some(CliImageStagingScope {
            host_id: "daemon-a".into(),
            terminal_session_id: session,
            terminal_epoch: "30000000-0000-4000-8000-000000000001".into(),
            cli_session_id: "native-a".into(),
            input_generation: "10000000-0000-4000-8000-000000000001".into(),
            submission_id: "20000000-0000-4000-8000-000000000001".into(),
        }),
        revision,
        action: Some(action),
    }
}

fn activate(session: u64, previous_revision: u64) -> CliImageStagingRequest {
    request(
        session,
        previous_revision,
        cli_image_staging_request::Action::Activate(CliImageStagingActivate {}),
    )
}

fn begin(session: u64, revision: u64, transfer: Uuid) -> CliImageStagingRequest {
    request(
        session,
        revision,
        cli_image_staging_request::Action::Begin(CliImageStagingBegin {
            transfer_id: transfer.to_string(),
            spec: Some(CliImageStagingSpec {
                byte_len: 4,
                sha256: Sha256::digest(b"\0\xffab").to_vec(),
            }),
        }),
    )
}

fn connection() -> ImageStagingConnection {
    let connection = ImageStagingConnection::new(Uuid::new_v4());
    connection.initialize();
    connection.bootstrap(
        SessionId::from(7),
        "30000000-0000-4000-8000-000000000001",
        1,
    );
    connection
}

fn error_code(response: CliImageStagingResponse) -> CliImageStagingErrorCode {
    let Some(cli_image_staging_response::Result::Error(error)) = response.result else {
        panic!("预期明确的错误收据")
    };
    CliImageStagingErrorCode::try_from(error.code).unwrap()
}

#[test]
fn live_initialized_connection_stages_and_verifies_original_binary_bytes() {
    let parent = tempfile::tempdir().unwrap();
    let service = ImageStagingService::new(HostId::new("daemon-a".into()), parent.path()).unwrap();
    let connection = connection();
    let activated = service.handle(&connection, activate(7, 0));
    assert_eq!(activated.revision, 1);
    let transfer = Uuid::new_v4();
    let begin = begin(7, 1, transfer);
    let expected_spec = match begin.action.as_ref() {
        Some(cli_image_staging_request::Action::Begin(begin)) => begin.spec.clone(),
        _ => panic!("预期 begin"),
    };
    assert!(matches!(
        service.handle(&connection, begin).result,
        Some(cli_image_staging_response::Result::Progress(_))
    ));
    let chunk = request(
        7,
        1,
        cli_image_staging_request::Action::Chunk(CliImageStagingChunk {
            transfer_id: transfer.to_string(),
            offset: 0,
            bytes: b"\0\xffab".to_vec(),
        }),
    );
    assert!(
        matches!(service.handle(&connection, chunk.clone()).result, Some(cli_image_staging_response::Result::Progress(progress)) if progress.next_offset == 4)
    );
    service.handle(&connection, chunk);
    let verified = service.handle(
        &connection,
        request(
            7,
            1,
            cli_image_staging_request::Action::Verify(CliImageStagingVerify {
                transfer_id: transfer.to_string(),
                expected_spec: expected_spec.clone(),
            }),
        ),
    );
    assert!(
        matches!(verified.result, Some(cli_image_staging_response::Result::Verified(verified)) if verified.spec == expected_spec)
    );
}

#[test]
fn initialize_and_same_connection_bootstrap_are_both_required() {
    let parent = tempfile::tempdir().unwrap();
    let service = ImageStagingService::new(HostId::new("daemon-a".into()), parent.path()).unwrap();
    let connection = ImageStagingConnection::new(Uuid::new_v4());
    connection.bootstrap(
        SessionId::from(7),
        "30000000-0000-4000-8000-000000000001",
        1,
    );
    assert_eq!(
        error_code(service.handle(&connection, activate(7, 0))),
        CliImageStagingErrorCode::InvalidScope
    );
    connection.initialize();
    assert_eq!(
        error_code(service.handle(&connection, activate(8, 0))),
        CliImageStagingErrorCode::InvalidScope
    );
    assert!(matches!(
        service.handle(&connection, activate(7, 0)).result,
        Some(cli_image_staging_response::Result::Activated(_))
    ));
}

#[test]
fn stale_activation_cannot_restore_an_older_generation() {
    let parent = tempfile::tempdir().unwrap();
    let service = ImageStagingService::new(HostId::new("daemon-a".into()), parent.path()).unwrap();
    let connection = connection();
    let original = activate(7, 0);
    service.handle(&connection, original.clone());
    assert_eq!(service.handle(&connection, original.clone()).revision, 1);
    let mut next = activate(7, 1);
    next.scope.as_mut().unwrap().input_generation = Uuid::new_v4().to_string();
    assert_eq!(service.handle(&connection, next.clone()).revision, 2);
    assert_eq!(service.handle(&connection, next).revision, 2);

    assert_eq!(
        error_code(service.handle(&connection, original)),
        CliImageStagingErrorCode::StaleScope
    );
    assert_eq!(
        error_code(service.handle(&connection, begin(7, 1, Uuid::new_v4()))),
        CliImageStagingErrorCode::StaleScope
    );
}

#[test]
fn same_host_and_terminal_number_on_sibling_connection_cannot_read_existing_transfer() {
    let parent = tempfile::tempdir().unwrap();
    let service = ImageStagingService::new(HostId::new("daemon-a".into()), parent.path()).unwrap();
    let first = connection();
    let second = connection();
    service.handle(&first, activate(7, 0));
    service.handle(&second, activate(7, 0));
    let transfer = Uuid::new_v4();
    service.handle(&first, begin(7, 1, transfer));

    assert_eq!(
        error_code(service.handle(&second, begin(7, 1, transfer))),
        CliImageStagingErrorCode::InvalidTransfer
    );
    second.revoke();
    service.disconnect(second.id).unwrap();
    assert!(matches!(
        service.handle(&first, begin(7, 1, transfer)).result,
        Some(cli_image_staging_response::Result::Progress(_))
    ));
}

#[test]
fn different_terminal_in_same_connection_cannot_access_other_terminal_transfer() {
    let parent = tempfile::tempdir().unwrap();
    let service = ImageStagingService::new(HostId::new("daemon-a".into()), parent.path()).unwrap();
    let connection = connection();
    connection.bootstrap(
        SessionId::from(8),
        "30000000-0000-4000-8000-000000000001",
        1,
    );
    service.handle(&connection, activate(7, 0));
    service.handle(&connection, activate(8, 0));
    let transfer = Uuid::new_v4();
    service.handle(&connection, begin(7, 1, transfer));

    assert_eq!(
        error_code(service.handle(&connection, begin(8, 1, transfer))),
        CliImageStagingErrorCode::InvalidTransfer
    );
}

#[test]
fn revocation_rejects_requests_before_background_cleanup_completes() {
    let parent = tempfile::tempdir().unwrap();
    let service = ImageStagingService::new(HostId::new("daemon-a".into()), parent.path()).unwrap();
    let connection = connection();
    service.handle(&connection, activate(7, 0));
    let transfer = Uuid::new_v4();
    service.handle(&connection, begin(7, 1, transfer));
    connection.revoke();

    assert_eq!(
        error_code(service.handle(&connection, begin(7, 1, transfer))),
        CliImageStagingErrorCode::InvalidScope
    );
    service.disconnect(connection.id).unwrap();
    assert!(service.state.lock().unwrap().registrations.is_empty());
    assert_eq!(
        error_code(service.handle(&connection, activate(7, 0))),
        CliImageStagingErrorCode::InvalidScope
    );
}

#[test]
fn mismatched_host_is_rejected_on_an_authenticated_connection() {
    let parent = tempfile::tempdir().unwrap();
    let service = ImageStagingService::new(HostId::new("daemon-a".into()), parent.path()).unwrap();
    let connection = connection();
    let mut request = activate(7, 0);
    request.scope.as_mut().unwrap().host_id = "daemon-b".into();
    assert_eq!(
        error_code(service.handle(&connection, request)),
        CliImageStagingErrorCode::InvalidScope
    );
}
