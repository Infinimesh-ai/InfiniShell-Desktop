use super::*;
use crate::client::InitializeParams;
use crate::proto::{
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
                        capabilities: if capability {
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
                    server_message::Message::CliImageStaging(CliImageStagingResponse {
                        scope,
                        revision: request.revision + 1,
                        result: Some(cli_image_staging_response::Result::Activated(
                            CliImageStagingActivated {},
                        )),
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
