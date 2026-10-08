use super::*;
use crate::client::InitializeParams;
use crate::proto::{
    CliImageClaudeQueue, CliImageCodexQueue, CliImageCodexQueueResult, CliImageCodexQueueStatus,
    CliImageStagingActivate, CliImageStagingActivated, CliImageStagingScope, InitializeResponse,
    ServerMessage, client_message,
};
use crate::protocol;
use futures::io::AsyncWriteExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use warpui_core::r#async::executor;

fn request() -> CliImageStagingRequest {
    CliImageStagingRequest {
        scope: Some(CliImageStagingScope {
            host_id: "daemon-a".into(),
            terminal_session_id: 7,
            terminal_epoch: "30000000-0000-4000-8000-000000000001".into(),
            cli_session_id: "native-a".into(),
            input_generation: "10000000-0000-4000-8000-000000000001".into(),
            submission_id: "20000000-0000-4000-8000-000000000001".into(),
        }),
        revision: 0,
        action: Some(cli_image_staging_request::Action::Activate(
            CliImageStagingActivate {},
        )),
    }
}

fn params() -> InitializeParams {
    InitializeParams {
        user_id: String::new(),
        user_email: String::new(),
        crash_reporting_enabled: false,
        codebase_index_limits: None,
    }
}

fn setup(
    capability: bool,
    wrong_scope: bool,
) -> (RemoteServerClient, executor::Background, Arc<AtomicUsize>) {
    setup_with_retirement(capability, wrong_scope, false)
}

fn setup_with_retirement(
    capability: bool,
    wrong_scope: bool,
    claude_retirement: bool,
) -> (RemoteServerClient, executor::Background, Arc<AtomicUsize>) {
    let (client_stream, server_stream) = tokio::io::duplex(8192);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let executor = executor::Background::default();
    let (client, _events, _failures, _host_responses) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);
    client.image_staging_sessions.write().unwrap().insert(
        SessionId::from(7),
        ImageStagingSession {
            epoch: Uuid::parse_str("30000000-0000-4000-8000-000000000001").unwrap(),
            epoch_revision: 1,
            next_submission: 0,
        },
    );
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    tokio::spawn(async move {
        let mut reader = server_read.compat();
        let mut writer = server_write.compat_write();
        loop {
            let message = match protocol::read_client_message(&mut reader).await {
                Ok(message) => message,
                Err(protocol::ProtocolError::UnexpectedEof) => break,
                Err(error) => panic!("协议读取失败：{error:?}"),
            };
            let Some(client_message::Message::SessionScoped(envelope)) = message.message else {
                panic!("图片暂存禁止 host-scoped envelope")
            };
            let response = match envelope.message {
                Some(session_scoped_request::Message::Initialize(_)) => {
                    server_message::Message::InitializeResponse(InitializeResponse {
                        server_version: "test".into(),
                        host_id: "daemon-a".into(),
                        capabilities: if claude_retirement {
                            vec![
                                RemoteServerCapability::CliImageUnpublishedStagingV1.into(),
                                RemoteServerCapability::CliImageClaudeReadV1.into(),
                            ]
                        } else if capability {
                            vec![RemoteServerCapability::CliImageUnpublishedStagingV1.into()]
                        } else {
                            Vec::new()
                        },
                    })
                }
                Some(session_scoped_request::Message::CliImageStaging(request)) => {
                    observed.fetch_add(1, Ordering::SeqCst);
                    let mut scope = request.scope;
                    if wrong_scope {
                        scope.as_mut().unwrap().terminal_session_id = 8;
                    }
                    let (revision, result) = if claude_retirement {
                        let submission_id = match request.action {
                            Some(cli_image_staging_request::Action::ClaudeQueue(_)) => {
                                scope.as_ref().unwrap().submission_id.clone()
                            }
                            Some(cli_image_staging_request::Action::ClaudeQueueStatus(status)) => {
                                status.submission_id
                            }
                            _ => panic!("退休夹具只接受 Claude 提交或状态查询"),
                        };
                        (
                            request.revision,
                            cli_image_staging_response::Result::ClaudeQueue(
                                CliImageCodexQueueResult {
                                    submission_id,
                                    subject_sha256: vec![7; 32],
                                    status: "retired".into(),
                                    native_queue_id: String::new(),
                                },
                            ),
                        )
                    } else {
                        (
                            request.revision + 1,
                            cli_image_staging_response::Result::Activated(
                                CliImageStagingActivated {},
                            ),
                        )
                    };
                    server_message::Message::CliImageStaging(CliImageStagingResponse {
                        scope,
                        revision,
                        result: Some(result),
                    })
                }
                _ => panic!("意外的协议请求"),
            };
            protocol::write_server_message(
                &mut writer,
                &ServerMessage {
                    request_id: message.request_id,
                    message: Some(response),
                },
            )
            .await
            .unwrap();
            writer.flush().await.unwrap();
        }
    });
    (client, executor, count)
}

#[tokio::test]
async fn older_daemon_is_rejected_without_sending_a_staging_request() {
    let (client, _executor, count) = setup(false, false);
    client.initialize(None, params()).await.unwrap();
    assert!(matches!(
        client.stage_unpublished_cli_image(request()).await,
        Err(ClientError::FileOperationFailed(_))
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn request_before_initialize_is_rejected_without_network_effects() {
    let (client, _executor, count) = setup(true, false);
    assert!(matches!(
        client.stage_unpublished_cli_image(request()).await,
        Err(ClientError::FileOperationFailed(_))
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn staging_round_trip_uses_a_session_scoped_frame_and_exact_scope() {
    let (client, _executor, count) = setup(true, false);
    client.initialize(None, params()).await.unwrap();
    let request = request();
    let response = client
        .stage_unpublished_cli_image(request.clone())
        .await
        .unwrap();
    assert_eq!(response.scope, request.scope);
    assert_eq!(response.revision, 1);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sibling_session_response_is_not_accepted() {
    let (client, _executor, count) = setup(true, true);
    client.initialize(None, params()).await.unwrap();
    assert!(matches!(
        client.stage_unpublished_cli_image(request()).await,
        Err(ClientError::UnexpectedResponse)
    ));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn wrong_host_is_rejected_before_sending() {
    let (client, _executor, count) = setup(true, false);
    client.initialize(None, params()).await.unwrap();
    let mut request = request();
    request.scope.as_mut().unwrap().host_id = "daemon-b".into();
    assert!(matches!(
        client.stage_unpublished_cli_image(request).await,
        Err(ClientError::FileOperationFailed(_))
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

fn claude_request(status_only: bool) -> CliImageStagingRequest {
    let mut request = request();
    request.revision = 4;
    request.action = Some(if status_only {
        cli_image_staging_request::Action::ClaudeQueueStatus(CliImageCodexQueueStatus {
            submission_id: request.scope.as_ref().unwrap().submission_id.clone(),
            recovery_key: "40000000-0000-4000-8000-000000000001".into(),
        })
    } else {
        cli_image_staging_request::Action::ClaudeQueue(CliImageClaudeQueue {
            subject_sha256: vec![7; 32],
            recovery_key: "40000000-0000-4000-8000-000000000001".into(),
            ..Default::default()
        })
    });
    request
}

fn retirement_response(request: &CliImageStagingRequest) -> CliImageStagingResponse {
    CliImageStagingResponse {
        scope: request.scope.clone(),
        revision: request.revision,
        result: Some(cli_image_staging_response::Result::ClaudeQueue(
            CliImageCodexQueueResult {
                submission_id: request.scope.as_ref().unwrap().submission_id.clone(),
                subject_sha256: vec![7; 32],
                status: "retired".into(),
                native_queue_id: String::new(),
            },
        )),
    }
}

#[tokio::test]
async fn claude_retirement_round_trip_preserves_unconfirmed_terminal_state() {
    let (client, _executor, count) = setup_with_retirement(true, false, true);
    client.initialize(None, params()).await.unwrap();
    for status_only in [false, true] {
        let request = claude_request(status_only);
        let response = client
            .stage_unpublished_cli_image(request.clone())
            .await
            .unwrap();
        assert_eq!(response.scope, request.scope);
        assert_eq!(response.revision, 4);
        let Some(cli_image_staging_response::Result::ClaudeQueue(result)) = response.result else {
            panic!("应保留原生 Claude 退休回包");
        };
        assert_eq!(result.status, "retired");
        assert_eq!(result.native_queue_id, "");
        assert_eq!(result.subject_sha256, vec![7; 32]);
        assert_eq!(result.submission_id, "20000000-0000-4000-8000-000000000001");
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn claude_retirement_response_requires_exact_scope_revision_and_submission() {
    for status_only in [false, true] {
        let request = claude_request(status_only);
        let mut response = retirement_response(&request);
        assert!(validate_response(&request, &response).is_ok());
        response.scope.as_mut().unwrap().terminal_epoch = Uuid::new_v4().to_string();
        assert!(validate_response(&request, &response).is_err());
        response.scope = request.scope.clone();
        response.revision = 5;
        assert!(validate_response(&request, &response).is_err());
        response.revision = request.revision;
        let Some(cli_image_staging_response::Result::ClaudeQueue(result)) = &mut response.result
        else {
            panic!("退休夹具类型改变");
        };
        result.submission_id = Uuid::new_v4().to_string();
        assert!(validate_response(&request, &response).is_err());
    }
}

#[test]
fn claude_retirement_requires_full_subject_and_cannot_claim_native_consumption() {
    for status_only in [false, true] {
        let request = claude_request(status_only);
        let original = retirement_response(&request);
        for (subject, status, native_queue_id) in [
            (vec![7; 31], "retired", ""),
            (vec![7; 33], "retired", ""),
            (vec![7; 32], "retired", "native-ack"),
            (vec![7; 32], "confirmed", ""),
        ] {
            let mut response = original.clone();
            let Some(cli_image_staging_response::Result::ClaudeQueue(result)) =
                &mut response.result
            else {
                panic!("退休夹具类型改变");
            };
            result.subject_sha256 = subject;
            result.status = status.into();
            result.native_queue_id = native_queue_id.into();
            assert!(validate_response(&request, &response).is_err());
        }
    }
    let mut request = claude_request(false);
    let mut response = retirement_response(&request);
    let Some(cli_image_staging_request::Action::ClaudeQueue(queue)) = &mut request.action else {
        panic!("提交夹具类型改变");
    };
    queue.subject_sha256 = vec![7; 31];
    let Some(cli_image_staging_response::Result::ClaudeQueue(result)) = &mut response.result else {
        panic!("退休夹具类型改变");
    };
    result.subject_sha256 = vec![7; 31];
    assert!(validate_response(&request, &response).is_err());
}

#[test]
fn codex_queue_does_not_accept_claude_retirement() {
    for status_only in [false, true] {
        let mut request = claude_request(status_only);
        let mut response = retirement_response(&request);
        request.action = Some(if status_only {
            cli_image_staging_request::Action::CodexQueueStatus(CliImageCodexQueueStatus {
                submission_id: request.scope.as_ref().unwrap().submission_id.clone(),
                recovery_key: "40000000-0000-4000-8000-000000000001".into(),
            })
        } else {
            cli_image_staging_request::Action::CodexQueue(CliImageCodexQueue {
                subject_sha256: vec![7; 32],
                ..Default::default()
            })
        });
        let Some(cli_image_staging_response::Result::ClaudeQueue(result)) = response.result else {
            panic!("退休夹具类型改变");
        };
        response.result = Some(cli_image_staging_response::Result::CodexQueue(result));
        assert!(validate_response(&request, &response).is_err());
    }
}
