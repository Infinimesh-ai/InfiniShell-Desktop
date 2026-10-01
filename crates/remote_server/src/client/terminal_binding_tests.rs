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
        Self::with_owned(capability, false).await
    }

    async fn with_owned(capability: bool, owned: bool) -> Self {
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
                                let mut capabilities =
                                    vec![RemoteServerCapability::TerminalBindingV1.into()];
                                if owned {
                                    capabilities.push(RemoteServerCapability::TmuxOwnedV1.into());
                                }
                                capabilities
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
                        pane_cwd: "/fixture/project".into(),
                        opaque_binding_id: "opaque-fixture".into(),
                    },
                )),
            },
        ))
        .unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap().opaque_binding_id,
        "opaque-fixture"
    );
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
                        pane_cwd: "/fixture/project".into(),
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
                        pane_cwd: "/fixture/project".into(),
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

fn owned_request() -> TerminalBindingStartOwned {
    TerminalBindingStartOwned {
        opaque_binding_id: Uuid::new_v4().to_string(),
        launch_id: Uuid::new_v4().to_string(),
        launch_key: Uuid::new_v4().to_string(),
        agent: TerminalBindingOwnedAgent::Claude as i32,
    }
}

#[tokio::test]
async fn tmux_owned_requires_separate_capability_without_sending_start() {
    let fixture = Fixture::new(true).await;
    let scope = fixture.bootstrap().await;
    assert!(!fixture.client.tmux_owned_available());
    assert!(
        fixture
            .client
            .start_terminal_owned(scope, Uuid::new_v4(), owned_request())
            .await
            .is_err()
    );
    assert!(fixture.wire.try_recv().is_err());
}

#[tokio::test]
async fn tmux_owned_unknown_start_is_followed_only_by_explicit_same_launch_status() {
    let fixture = Fixture::with_owned(true, true).await;
    let scope = fixture.bootstrap().await;
    let attempt = Uuid::new_v4();
    let launch = owned_request();
    for status in [false, true] {
        let client = fixture.client.clone();
        let pending_scope = scope.clone();
        let request = launch.clone();
        let pending = tokio::spawn(async move {
            if status {
                client
                    .status_terminal_owned(pending_scope, attempt, request)
                    .await
            } else {
                client
                    .start_terminal_owned(pending_scope, attempt, request)
                    .await
            }
        });
        let wire = fixture.wire.recv().await.unwrap();
        let request = binding_request(&wire);
        assert_eq!(request.scope.as_ref(), Some(&scope));
        match &request.action {
            Some(terminal_binding_request::Action::StartOwned(actual)) if !status => {
                assert_eq!(actual, &launch);
            }
            Some(terminal_binding_request::Action::StatusOwned(actual)) if status => {
                assert_eq!(actual.launch_id, launch.launch_id);
                assert_eq!(actual.launch_key, launch.launch_key);
                assert_eq!(actual.opaque_binding_id, launch.opaque_binding_id);
            }
            _ => panic!("只允许单次 Start 和显式 Status"),
        }
        wire.reply
            .send(server_message::Message::TerminalBinding(
                TerminalBindingResponse {
                    scope: Some(scope.clone()),
                    attempt_id: attempt.to_string(),
                    result: Some(terminal_binding_response::Result::Owned(
                        TerminalBindingOwned {
                            launch_id: launch.launch_id.clone(),
                            agent: launch.agent,
                            phase: "unknown".into(),
                            owned_reply_json: vec![],
                            native_session_id: None,
                        },
                    )),
                },
            ))
            .unwrap();
        assert_eq!(pending.await.unwrap().unwrap().phase, "unknown");
        assert!(fixture.wire.try_recv().is_err());
    }
}

#[tokio::test]
async fn tmux_owned_reply_for_other_launch_or_agent_cannot_bind_current_intent() {
    let fixture = Fixture::with_owned(true, true).await;
    let scope = fixture.bootstrap().await;
    for wrong_agent in [false, true] {
        let client = fixture.client.clone();
        let pending_scope = scope.clone();
        let launch = owned_request();
        let request = launch.clone();
        let attempt = Uuid::new_v4();
        let pending = tokio::spawn(async move {
            client
                .status_terminal_owned(pending_scope, attempt, request)
                .await
        });
        let wire = fixture.wire.recv().await.unwrap();
        wire.reply
            .send(server_message::Message::TerminalBinding(
                TerminalBindingResponse {
                    scope: Some(scope.clone()),
                    attempt_id: attempt.to_string(),
                    result: Some(terminal_binding_response::Result::Owned(
                        TerminalBindingOwned {
                            launch_id: if wrong_agent {
                                launch.launch_id
                            } else {
                                Uuid::new_v4().to_string()
                            },
                            agent: if wrong_agent {
                                TerminalBindingOwnedAgent::Grok as i32
                            } else {
                                launch.agent
                            },
                            phase: "running".into(),
                            owned_reply_json: vec![],
                            native_session_id: None,
                        },
                    )),
                },
            ))
            .unwrap();
        assert!(pending.await.unwrap().is_err());
    }
}

#[test]
fn tmux_owned_messages_redact_key_binding_and_nested_reply() {
    let launch = owned_request();
    let message = ClientMessage::session_scoped(
        "request".into(),
        session_scoped_request::Message::TerminalBinding(TerminalBindingRequest {
            scope: None,
            attempt_id: "attempt".into(),
            action: Some(terminal_binding_request::Action::StartOwned(launch.clone())),
        }),
    );
    let debug = format!("{message:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains(&launch.launch_key));
    assert!(!debug.contains(&launch.opaque_binding_id));
    let owned = TerminalBindingOwned {
        launch_id: launch.launch_id,
        agent: launch.agent,
        phase: "running".into(),
        owned_reply_json: b"private-ticket-key".to_vec(),
        native_session_id: None,
    };
    assert!(!format!("{owned:?}").contains("private-ticket-key"));
    let reference = crate::proto::TerminalBindingOwnedReference {
        opaque_binding_id: launch.opaque_binding_id.clone(),
        launch_id: "launch".into(),
        launch_key: launch.launch_key.clone(),
    };
    assert!(!format!("{reference:?}").contains(&launch.launch_key));
}
