use super::*;
use crate::client::{ClientEvent, InitializeParams};
use crate::proto::{
    InitializeResponse, ServerMessage, TerminalBindingBound, TerminalBindingChallengeWritten,
    TerminalBindingResponse, client_message,
};
use crate::protocol;
use futures::channel::oneshot;
use futures::io::AsyncWriteExt;
use std::sync::Arc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use warpui_core::r#async::executor;

struct Wire {
    message: ClientMessage,
    reply: oneshot::Sender<server_message::Message>,
}

struct Fixture {
    client: Arc<RemoteServerClient>,
    wire: async_channel::Receiver<Wire>,
    events: async_channel::Receiver<ClientEvent>,
    server: tokio::task::JoinHandle<()>,
    _executor: executor::Background,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Fixture {
    async fn new(capability: bool) -> Self {
        let (client_stream, server_stream) = tokio::io::duplex(8192);
        let (client_read, client_write) = tokio::io::split(client_stream);
        let (server_read, server_write) = tokio::io::split(server_stream);
        let executor = executor::Background::default();
        let (client, events, _, _) =
            RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);
        let client = Arc::new(client);
        let (tx, wire) = async_channel::unbounded();
        let server = tokio::spawn(async move {
            let mut reader = server_read.compat();
            let mut writer = server_write.compat_write();
            loop {
                let message = match protocol::read_client_message(&mut reader).await {
                    Ok(message) => message,
                    Err(protocol::ProtocolError::UnexpectedEof) => break,
                    Err(error) => panic!("内存协议读取失败：{error:?}"),
                };
                let request_id = message.request_id.clone();
                let response = if matches!(message.message.as_ref(), Some(client_message::Message::SessionScoped(envelope)) if matches!(envelope.message, Some(session_scoped_request::Message::Initialize(_))))
                {
                    Some(server_message::Message::InitializeResponse(
                        InitializeResponse {
                            server_version: "test".into(),
                            host_id: "terminal-daemon".into(),
                            capabilities: if capability {
                                vec![RemoteServerCapability::TerminalBindingV1.into()]
                            } else {
                                vec![]
                            },
                        },
                    ))
                } else {
                    let notification = matches!(
                        message.message,
                        Some(client_message::Message::Notification(_))
                    );
                    let (reply, response) = oneshot::channel();
                    if tx.send(Wire { message, reply }).await.is_err() {
                        break;
                    }
                    if notification {
                        None
                    } else {
                        Some(response.await.unwrap())
                    }
                };
                if let Some(message) = response {
                    protocol::write_server_message(
                        &mut writer,
                        &ServerMessage {
                            request_id,
                            message: Some(message),
                        },
                    )
                    .await
                    .unwrap();
                    writer.flush().await.unwrap();
                }
            }
        });
        client
            .initialize(
                None,
                InitializeParams {
                    user_id: String::new(),
                    user_email: String::new(),
                    crash_reporting_enabled: false,
                    codebase_index_limits: None,
                },
            )
            .await
            .unwrap();
        Self {
            client,
            wire,
            events,
            server,
            _executor: executor,
        }
    }

    async fn bootstrap(&self) -> TerminalBindingScope {
        self.client
            .notify_session_bootstrapped_with_terminal_candidate(
                SessionId::from(17),
                "zsh",
                Some("/bin/zsh"),
                Some(1234),
                Some("/dev/pts/7"),
            );
        let message = self.wire.recv().await.unwrap().message;
        let Some(client_message::Message::Notification(envelope)) = message.message else {
            panic!("必须是 bootstrap 通知")
        };
        let Some(notification::Message::SessionBootstrapped(bootstrap)) = envelope.message else {
            panic!("只允许 bootstrap")
        };
        assert_eq!(bootstrap.shell_pid, Some(1234));
        assert_eq!(bootstrap.shell_tty.as_deref(), Some("/dev/pts/7"));
        let scope = self
            .client
            .terminal_binding_scope(SessionId::from(17))
            .unwrap();
        assert_eq!(scope.terminal_epoch, bootstrap.image_staging_epoch);
        scope
    }
}

fn binding_request(wire: &Wire) -> &TerminalBindingRequest {
    let Some(client_message::Message::SessionScoped(envelope)) = &wire.message.message else {
        panic!("禁止主机级请求")
    };
    let Some(session_scoped_request::Message::TerminalBinding(request)) = &envelope.message else {
        panic!("只允许终端绑定请求")
    };
    request
}

#[tokio::test]
async fn terminal_binding_roundtrip_echoes_scope_and_attempt_without_image_capability() {
    let fixture = Fixture::new(true).await;
    assert!(
        fixture
            .client
            .terminal_binding_scope(SessionId::from(17))
            .is_none()
    );
    let scope = fixture.bootstrap().await;
    let attempt = Uuid::new_v4();
    let client = fixture.client.clone();
    let pending_scope = scope.clone();
    let pending =
        tokio::spawn(async move { client.begin_terminal_binding(pending_scope, attempt).await });
    let wire = fixture.wire.recv().await.unwrap();
    let request = binding_request(&wire);
    assert_eq!(request.scope.as_ref(), Some(&scope));
    assert_eq!(request.attempt_id, attempt.to_string());
    assert!(matches!(
        request.action,
        Some(terminal_binding_request::Action::Begin(_))
    ));
    wire.reply
        .send(server_message::Message::TerminalBinding(
            TerminalBindingResponse {
                scope: Some(scope.clone()),
                attempt_id: attempt.to_string(),
                result: Some(terminal_binding_response::Result::ChallengeWritten(
                    TerminalBindingChallengeWritten {},
                )),
            },
        ))
        .unwrap();
    pending.await.unwrap().unwrap();
    let nonce = Uuid::new_v4();
    let client = fixture.client.clone();
    let pending_scope = scope.clone();
    let pending = tokio::spawn(async move {
        client
            .ack_terminal_binding(pending_scope, attempt, nonce)
            .await
    });
    let wire = fixture.wire.recv().await.unwrap();
    let request = binding_request(&wire);
    assert!(
        matches!(&request.action, Some(terminal_binding_request::Action::Ack(ack)) if ack.nonce == nonce.to_string())
    );
    assert!(!format!("{:?}", wire.message).contains(&nonce.to_string()));
    wire.reply
        .send(server_message::Message::TerminalBinding(
            TerminalBindingResponse {
                scope: Some(scope.clone()),
                attempt_id: attempt.to_string(),
                result: Some(terminal_binding_response::Result::Bound(
                    TerminalBindingBound {
                        opaque_binding_id: "opaque-fixture".into(),
                    },
                )),
            },
        ))
        .unwrap();
    assert_eq!(pending.await.unwrap().unwrap(), "opaque-fixture");
    fixture
        .client
        .cancel_terminal_binding(scope.clone(), attempt);
    let message = fixture.wire.recv().await.unwrap().message;
    let Some(client_message::Message::Notification(envelope)) = message.message else {
        panic!("取消不能依赖 future")
    };
    let Some(notification::Message::CancelTerminalBinding(request)) = envelope.message else {
        panic!("只允许精确取消")
    };
    assert_eq!(request.scope, Some(scope));
    assert_eq!(request.attempt_id, attempt.to_string());
    assert!(matches!(
        request.action,
        Some(terminal_binding_request::Action::Cancel(_))
    ));
    assert_eq!(
        fixture
            .client
            .image_staging_sessions
            .read()
            .unwrap()
            .get(&SessionId::from(17))
            .unwrap()
            .next_submission,
        0
    );
}

#[tokio::test]
async fn terminal_binding_rejects_wrong_echoes_and_reply_kind() {
    for mismatch in 0..4 {
        let fixture = Fixture::new(true).await;
        let scope = fixture.bootstrap().await;
        let attempt = Uuid::new_v4();
        let client = fixture.client.clone();
        let pending_scope = scope.clone();
        let pending =
            tokio::spawn(
                async move { client.begin_terminal_binding(pending_scope, attempt).await },
            );
        let wire = fixture.wire.recv().await.unwrap();
        let mut response = TerminalBindingResponse {
            scope: Some(scope),
            attempt_id: attempt.to_string(),
            result: Some(terminal_binding_response::Result::ChallengeWritten(
                TerminalBindingChallengeWritten {},
            )),
        };
        match mismatch {
            0 => response.scope.as_mut().unwrap().host_id = "other-host".into(),
            1 => response.scope.as_mut().unwrap().terminal_session_id += 1,
            2 => response.attempt_id = Uuid::new_v4().to_string(),
            3 => {
                response.result = Some(terminal_binding_response::Result::Bound(
                    TerminalBindingBound {
                        opaque_binding_id: "unexpected".into(),
                    },
                ))
            }
            _ => unreachable!(),
        }
        wire.reply
            .send(server_message::Message::TerminalBinding(response))
            .unwrap();
        assert!(matches!(
            pending.await.unwrap(),
            Err(ClientError::UnexpectedResponse)
        ));
    }
}

#[tokio::test]
async fn terminal_binding_rejects_reply_after_epoch_rotation_and_disconnect() {
    let fixture = Fixture::new(true).await;
    let scope = fixture.bootstrap().await;
    let attempt = Uuid::new_v4();
    let client = fixture.client.clone();
    let pending_scope = scope.clone();
    let pending =
        tokio::spawn(async move { client.begin_terminal_binding(pending_scope, attempt).await });
    let wire = fixture.wire.recv().await.unwrap();
    fixture
        .client
        .notify_session_bootstrapped(SessionId::from(17), "zsh", Some("/bin/zsh"));
    assert!(!fixture.client.terminal_binding_scope_is_current(&scope));
    wire.reply
        .send(server_message::Message::TerminalBinding(
            TerminalBindingResponse {
                scope: Some(scope),
                attempt_id: attempt.to_string(),
                result: Some(terminal_binding_response::Result::ChallengeWritten(
                    TerminalBindingChallengeWritten {},
                )),
            },
        ))
        .unwrap();
    assert!(matches!(
        pending.await.unwrap(),
        Err(ClientError::Disconnected)
    ));
    fixture.server.abort();
    while !matches!(
        fixture.events.recv().await.unwrap(),
        ClientEvent::Disconnected
    ) {}
    assert!(
        fixture
            .client
            .terminal_binding_scope(SessionId::from(17))
            .is_none()
    );
}

#[tokio::test]
async fn terminal_binding_requires_own_capability() {
    let fixture = Fixture::new(false).await;
    fixture
        .client
        .notify_session_bootstrapped(SessionId::from(17), "zsh", None);
    let _ = fixture.wire.recv().await.unwrap();
    assert!(
        fixture
            .client
            .terminal_binding_scope(SessionId::from(17))
            .is_none()
    );
}

#[test]
fn terminal_binding_bound_debug_redacts_all_parent_envelopes() {
    let message = ServerMessage {
        request_id: "request".into(),
        message: Some(server_message::Message::TerminalBinding(
            TerminalBindingResponse {
                scope: None,
                attempt_id: "attempt".into(),
                result: Some(terminal_binding_response::Result::Bound(
                    TerminalBindingBound {
                        opaque_binding_id: "secret-binding-value".into(),
                    },
                )),
            },
        )),
    };
    let debug = format!("{message:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("secret-binding-value"));
}
