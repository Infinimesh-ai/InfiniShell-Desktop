//! 真实 OSC→模型事件→view→原连接 ACK 的内存验收，不启动原生进程。

use super::*;
use crate::remote_server::auth::RemoteServerAuthContext;
use crate::remote_server::cli_image_grok_protocol::{
    Action as GrokAction, Reply as GrokReply, Request as GrokRequest, Ticket,
};
use crate::remote_server::manager::{RemoteServerExitStatus, RemoteServerManagerEvent};
use crate::remote_server::proto::{
    CliGrokOwnedResponse, InitializeResponse, RemoteServerCapability, ServerMessage,
    TerminalBindingBound, TerminalBindingChallengeWritten, TerminalBindingFailed,
    TerminalBindingRequest, TerminalBindingResponse, client_message, notification, server_message,
    session_scoped_request, terminal_binding_request, terminal_binding_response,
};
use crate::remote_server::protocol;
use crate::remote_server::setup::{PreinstallCheckResult, RemotePlatform};
use crate::remote_server::transport::{
    Connection, ControlPath, Error, InstallOutcome, RemoteTransport, TransportConnection,
};
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::ansi::Processor;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::{BootstrapSessionType, SessionInfo};
use crate::terminal::model_events::ModelEvent;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use async_compat::CompatExt;
use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Either, select};
use futures::io::AsyncWriteExt;
use std::future::Future;
use std::pin::Pin;
use warp_core::channel::ChannelState;
use warpui::r#async::executor;
use warpui::{App, ViewHandle};

fn session_id() -> SessionId {
    SessionId::from(7501)
}

#[derive(Debug)]
pub(in crate::terminal::view) struct Wire {
    pub(in crate::terminal::view) request: TerminalBindingRequest,
    pub(in crate::terminal::view) reply: oneshot::Sender<TerminalBindingResponse>,
}
impl Wire {
    pub(in crate::terminal::view) fn written(self) {
        self.respond(terminal_binding_response::Result::ChallengeWritten(
            TerminalBindingChallengeWritten {},
        ));
    }
    pub(in crate::terminal::view) fn bound(self) {
        self.respond(terminal_binding_response::Result::Bound(
            TerminalBindingBound {
                pane_cwd: "/fixture/project".into(),
                opaque_binding_id: "10000000-0000-4000-8000-000000000002".into(),
            },
        ));
    }
    pub(in crate::terminal::view) fn respond(self, result: terminal_binding_response::Result) {
        self.reply
            .send(TerminalBindingResponse {
                scope: self.request.scope,
                attempt_id: self.request.attempt_id,
                result: Some(result),
            })
            .unwrap();
    }
}

#[derive(Debug, Clone)]
struct MemoryTransport {
    done: async_channel::Sender<()>,
    requests: async_channel::Sender<Wire>,
    notifications: async_channel::Sender<notification::Message>,
    disconnects: async_channel::Sender<AbortHandle>,
    owned: bool,
    grok: bool,
    grok_observations: async_channel::Sender<GrokObservation>,
}

pub(in crate::terminal::view) struct GrokObservation {
    pub(in crate::terminal::view) ticket: Ticket,
    pub(in crate::terminal::view) reply: oneshot::Sender<GrokReply>,
}
impl std::fmt::Debug for GrokObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GrokObservation")
    }
}

#[derive(Debug)]
struct MemoryConnection(AbortHandle);

impl Drop for MemoryConnection {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl TransportConnection for MemoryConnection {
    fn terminate(&mut self) {
        self.0.abort();
    }

    fn try_exit_status(&mut self) -> Option<RemoteServerExitStatus> {
        None
    }

    fn wait_for_exit(
        self: Box<Self>,
    ) -> Pin<Box<dyn Future<Output = Option<RemoteServerExitStatus>> + Send>> {
        Box::pin(async move {
            drop(self);
            None
        })
    }

    fn stderr_tail(&self) -> Option<String> {
        None
    }
}

impl RemoteTransport for MemoryTransport {
    fn detect_platform(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemotePlatform, Error>> + Send>> {
        panic!("内存协议夹具不探测平台")
    }

    fn run_preinstall_check(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<PreinstallCheckResult, Error>> + Send>> {
        panic!("内存协议夹具不运行安装检查")
    }

    fn check_binary(&self) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send>> {
        panic!("内存协议夹具不读取二进制")
    }

    fn check_has_old_binary(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>> {
        panic!("内存协议夹具不读取二进制")
    }

    fn install_binary(&self) -> Pin<Box<dyn Future<Output = InstallOutcome> + Send>> {
        panic!("内存协议夹具不安装二进制")
    }

    fn remove_remote_server_binary(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
        panic!("内存协议夹具不删除二进制")
    }

    fn is_reconnectable(&self, _: Option<&RemoteServerExitStatus>) -> bool {
        false
    }

    fn connect(
        &self,
        executor: Arc<executor::Background>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Connection>> + Send>> {
        let done = self.done.clone();
        let requests = self.requests.clone();
        let notifications = self.notifications.clone();
        let disconnects = self.disconnects.clone();
        let owned = self.owned;
        let grok = self.grok;
        let grok_observations = self.grok_observations.clone();
        Box::pin(async move {
            let (client_stream, server_stream) = tokio::io::duplex(8192);
            let (client_read, client_write) = tokio::io::split(client_stream);
            let (server_read, server_write) = tokio::io::split(server_stream);
            let (client, event_rx, failure_rx, host_response_rx) =
                RemoteServerClient::new(client_read.compat(), client_write.compat(), &executor);
            let (abort, registration) = AbortHandle::new_pair();
            disconnects.send(abort.clone()).await.unwrap();
            executor
                .spawn(async move {
                    let server = async move {
                        let mut reader = server_read.compat();
                        let mut writer = server_write.compat();
                        loop {
                            let message = match protocol::read_client_message(&mut reader).await {
                                Ok(message) => message,
                                Err(protocol::ProtocolError::UnexpectedEof) => break,
                                Err(error) => panic!("内存协议读取失败：{error:?}"),
                            };
                            let response = match message.message {
                                Some(client_message::Message::SessionScoped(envelope)) => {
                                    match envelope.message {
                                        Some(session_scoped_request::Message::Initialize(_)) => {
                                            server_message::Message::InitializeResponse(
                                                InitializeResponse {
                                                    server_version: ChannelState::app_version()
                                                        .unwrap_or_default()
                                                        .to_owned(),
                                                    host_id: "terminal-binding-fixture".into(),
                                                    capabilities: if owned {
                                                        let mut capabilities = vec![RemoteServerCapability::TerminalBindingV1.into(), RemoteServerCapability::TmuxOwnedV1.into(), RemoteServerCapability::CliImageClaudeReadV1.into()];
                                                        if grok { capabilities.extend([i32::from(RemoteServerCapability::CliImageUnpublishedStagingV1), i32::from(RemoteServerCapability::CliImageReferenceRecoveryV1), i32::from(RemoteServerCapability::CliGrokOwnedV1)]); }
                                                        capabilities
                                                    } else { vec![RemoteServerCapability::TerminalBindingV1.into()] },
                                                },
                                            )
                                        }
                                        Some(session_scoped_request::Message::TerminalBinding(
                                            request,
                                        )) => {
                                            let (reply, response) = oneshot::channel();
                                            requests.send(Wire { request, reply }).await.unwrap();
                                            let Ok(response) = response.await else {
                                                break;
                                            };
                                            server_message::Message::TerminalBinding(response)
                                        }
                                        Some(session_scoped_request::Message::CliGrokOwned(request)) => {
                                            assert!(grok);
                                            let scope = request.scope.unwrap();
                                            let request = GrokRequest::decode(&request.body_json).unwrap();
                                            assert_eq!(request.scope.host, scope.host_id);
                                            assert_eq!(request.scope.terminal_session, scope.terminal_session_id);
                                            assert_eq!(request.scope.terminal_epoch.to_string(), scope.terminal_epoch);
                                            let GrokAction::Observe { ticket } = request.action else {
                                                panic!("恢复夹具只允许指定票据的只读 Observe，不接受启动或输入");
                                            };
                                            let (reply, response) = oneshot::channel();
                                            grok_observations.send(GrokObservation { ticket, reply }).await.unwrap();
                                            let Ok(reply) = response.await else { break; };
                                            server_message::Message::CliGrokOwned(CliGrokOwnedResponse {
                                                scope: Some(scope),
                                                body_json: serde_json::to_vec(&reply).unwrap(),
                                            })
                                        }
                                        _ => panic!(
                                            "终端绑定夹具禁止导航、图片、owned 启动和模型请求"
                                        ),
                                    }
                                }
                                Some(client_message::Message::Notification(envelope)) => {
                                    match envelope.message {
                                        Some(
                                            message @ (notification::Message::SessionBootstrapped(
                                                _,
                                            )
                                            | notification::Message::CancelTerminalBinding(
                                                _,
                                            )),
                                        ) => {
                                            let _ = notifications.send(message).await;
                                        }
                                        Some(notification::Message::Abort(_)) => {}
                                        _ => panic!("终端绑定夹具收到未授权通知"),
                                    }
                                    continue;
                                }
                                _ => panic!("终端绑定禁止主机级请求"),
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
                    };
                    let _ = Abortable::new(server, registration).await;
                    // 断言失败时接收端可能先析构，正常收尾不能制造第二次 panic。
                    let _ = done.send(()).await;
                })
                .detach();
            Ok(Connection {
                client,
                event_rx,
                failure_rx,
                host_response_rx,
                resource: Box::new(MemoryConnection(abort)),
                control_path: ControlPath::None,
            })
        })
    }
}

pub(in crate::terminal::view) async fn bounded<T>(future: impl Future<Output = T>) -> T {
    match select(
        Box::pin(future),
        Box::pin(Timer::after(Duration::from_secs(5))),
    )
    .await
    {
        Either::Left((result, _)) => result,
        Either::Right(_) => panic!("终端绑定协议或 UI 事件未完成"),
    }
}

pub(in crate::terminal::view) fn state_changed(
    app: &mut App,
    terminal: &ViewHandle<TerminalView>,
    predicate: impl Fn(&TerminalView) -> bool + 'static,
) -> oneshot::Receiver<()> {
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    let terminal = terminal.clone();
    let window_id = app.update(|ctx| terminal.window_id(ctx));
    app.on_window_invalidated(window_id, move |_, ctx| {
        if terminal.read(ctx, |view, _| predicate(view))
            && let Some(sender) = sender.take()
        {
            let _ = sender.send(());
        }
    });
    receiver
}

pub(in crate::terminal::view) struct Fixture {
    pub(in crate::terminal::view) terminal: ViewHandle<TerminalView>,
    pub(in crate::terminal::view) client: Arc<RemoteServerClient>,
    pub(in crate::terminal::view) requests: async_channel::Receiver<Wire>,
    notifications: async_channel::Receiver<notification::Message>,
    disconnects: async_channel::Receiver<AbortHandle>,
    done: async_channel::Receiver<()>,
    transport: MemoryTransport,
    current_session: SessionId,
    pub(in crate::terminal::view) grok_observations: async_channel::Receiver<GrokObservation>,
}

impl Fixture {
    pub(in crate::terminal::view) async fn new(app: &mut App) -> Self {
        Self::with_owned(app, false).await
    }

    pub(in crate::terminal::view) async fn with_owned(app: &mut App, owned: bool) -> Self {
        Self::with_capabilities(app, owned, false).await
    }

    pub(in crate::terminal::view) async fn with_grok_owned(app: &mut App) -> Self {
        Self::with_capabilities(app, true, true).await
    }

    async fn with_capabilities(app: &mut App, owned: bool, grok: bool) -> Self {
        initialize_app_for_terminal_view(app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        let (done_sender, done) = async_channel::unbounded();
        let (request_sender, requests) = async_channel::unbounded();
        let (notification_sender, notifications) = async_channel::unbounded();
        let (disconnect_sender, disconnects) = async_channel::unbounded();
        let (grok_sender, grok_observations) = async_channel::unbounded();
        let transport = MemoryTransport {
            done: done_sender,
            requests: request_sender,
            notifications: notification_sender,
            disconnects: disconnect_sender,
            owned,
            grok,
            grok_observations: grok_sender,
        };
        let (sender, receiver) = oneshot::channel();
        let mut sender = Some(sender);
        app.update(|ctx| {
            ctx.subscribe_to_model(&RemoteServerManager::handle(ctx), move |_, event, _| {
                if matches!(event, RemoteServerManagerEvent::SessionConnected { session_id: id, .. } if *id == session_id())
                    && let Some(sender) = sender.take()
                { let _ = sender.send(()); }
            });
        });
        RemoteServerManager::handle(app).update(app, |manager, ctx| {
            manager.connect_session(
                session_id(),
                transport.clone(),
                Arc::new(RemoteServerAuthContext::new(
                    || Box::pin(async { None }),
                    || "terminal-binding-fixture".into(),
                )),
                None,
                ctx,
            );
        });
        bounded(receiver).await.unwrap();
        let client = RemoteServerManager::handle(app).read(app, |manager, _| {
            manager.client_for_session(session_id()).unwrap().clone()
        });
        let terminal = add_window_with_terminal(app, None);
        terminal.update(app, |view, ctx| {
            let mut info = SessionInfo::new_for_test()
                .with_session_type(BootstrapSessionType::WarpifiedRemote)
                .with_shell_type(ShellType::Zsh);
            info.session_id = session_id();
            view.sessions
                .update(ctx, |sessions, _| sessions.register_session_for_test(info));
            let mut model = view.model.lock();
            model.simulate_long_running_block("tmux", "");
            model
                .block_list_mut()
                .active_block_mut()
                .set_session_id(session_id());
            model.register_session_id(session_id());
        });
        RemoteServerManager::handle(app).update(app, |manager, _| {
            manager.notify_session_bootstrapped_with_terminal_candidate(
                session_id(),
                "zsh",
                Some("/bin/zsh"),
                Some(1234),
                Some("/dev/pts/7"),
            );
        });
        assert!(matches!(
            bounded(notifications.recv()).await.unwrap(),
            notification::Message::SessionBootstrapped(_)
        ));
        terminal.update(app, |view, ctx| {
            let block_index = view.model.lock().block_list().active_block_index();
            view.handle_terminal_event(
                &ModelEvent::BlockMetadataReceived(BlockMetadataReceivedEvent {
                    block_metadata: BlockMetadata::new(Some(session_id()), None),
                    block_index,
                    is_after_in_band_command: false,
                    is_done_bootstrapping: true,
                }),
                ctx,
            );
        });
        Self {
            terminal,
            client,
            requests,
            notifications,
            disconnects,
            done,
            transport,
            current_session: session_id(),
            grok_observations,
        }
    }

    pub(in crate::terminal::view) async fn begin(&self, app: &mut App) -> (Uuid, Wire) {
        let attempt = self.terminal.update(app, |view, ctx| {
            view.begin_remote_terminal_binding(ctx).unwrap()
        });
        let wire = bounded(self.requests.recv()).await.unwrap();
        assert_eq!(wire.request.attempt_id, attempt.to_string());
        assert!(matches!(
            wire.request.action,
            Some(terminal_binding_request::Action::Begin(_))
        ));
        (attempt, wire)
    }

    pub(in crate::terminal::view) fn osc(&self, app: &mut App, attempt: Uuid, nonce: Uuid) {
        let session = self.current_session.as_u64();
        self.terminal.update(app, |view, _| {
            let mut model = view.model.lock();
            let body = format!("\x1b]9278;t;1;{session};{attempt};{nonce}\x07");
            Processor::new().parse_bytes(&mut *model, body.as_bytes(), &mut std::io::sink());
        });
    }

    /// 真实 EOF 后建立另一个 SSH session；模型块由调用方先完成并等待收尾。
    pub(in crate::terminal::view) async fn reconnect_as(
        &mut self,
        app: &mut App,
        new_session: SessionId,
    ) {
        let old_session = self.current_session;
        let (sender, receiver) = oneshot::channel();
        let mut sender = Some(sender);
        app.update(|ctx| {
            ctx.subscribe_to_model(&RemoteServerManager::handle(ctx), move |_, event, _| {
                if matches!(event, RemoteServerManagerEvent::SessionDisconnected { session_id, .. } if *session_id == old_session)
                    && let Some(sender) = sender.take()
                { let _ = sender.send(()); }
            });
        });
        bounded(self.disconnects.recv()).await.unwrap().abort();
        bounded(receiver).await.unwrap();
        bounded(self.done.recv()).await.unwrap();
        let (sender, receiver) = oneshot::channel();
        let mut sender = Some(sender);
        app.update(|ctx| {
            ctx.subscribe_to_model(&RemoteServerManager::handle(ctx), move |_, event, _| {
                if matches!(event, RemoteServerManagerEvent::SessionConnected { session_id, .. } if *session_id == new_session)
                    && let Some(sender) = sender.take()
                { let _ = sender.send(()); }
            });
        });
        RemoteServerManager::handle(app).update(app, |manager, ctx| {
            manager.deregister_session(old_session, ctx);
            manager.connect_session(
                new_session,
                self.transport.clone(),
                Arc::new(RemoteServerAuthContext::new(
                    || Box::pin(async { None }),
                    || "terminal-binding-fixture".into(),
                )),
                None,
                ctx,
            );
        });
        bounded(receiver).await.unwrap();
        self.current_session = new_session;
        self.client = RemoteServerManager::handle(app).read(app, |manager, _| {
            manager.client_for_session(new_session).unwrap().clone()
        });
        self.terminal.update(app, |view, ctx| {
            let mut info = SessionInfo::new_for_test()
                .with_session_type(BootstrapSessionType::WarpifiedRemote)
                .with_shell_type(ShellType::Zsh);
            info.session_id = new_session;
            view.sessions
                .update(ctx, |sessions, _| sessions.register_session_for_test(info));
            let mut model = view.model.lock();
            model.simulate_long_running_block("tmux", "");
            model
                .block_list_mut()
                .active_block_mut()
                .set_session_id(new_session);
            model.register_session_id(new_session);
        });
        RemoteServerManager::handle(app).update(app, |manager, _| {
            manager.notify_session_bootstrapped_with_terminal_candidate(
                new_session,
                "zsh",
                Some("/bin/zsh"),
                Some(1235),
                Some("/dev/pts/8"),
            );
        });
        loop {
            if matches!(bounded(self.notifications.recv()).await.unwrap(),
                notification::Message::SessionBootstrapped(message) if message.session_id == new_session.as_u64())
            {
                break;
            }
        }
        self.terminal.update(app, |view, ctx| {
            let block_index = view.model.lock().block_list().active_block_index();
            view.handle_terminal_event(
                &ModelEvent::BlockMetadataReceived(BlockMetadataReceivedEvent {
                    block_metadata: BlockMetadata::new(Some(new_session), None),
                    block_index,
                    is_after_in_band_command: false,
                    is_done_bootstrapping: true,
                }),
                ctx,
            );
        });
    }

    pub(in crate::terminal::view) async fn finish_ack(
        &self,
        app: &mut App,
        attempt: Uuid,
        nonce: Uuid,
    ) {
        let wire = bounded(self.requests.recv()).await.unwrap();
        assert_eq!(wire.request.attempt_id, attempt.to_string());
        assert!(
            matches!(&wire.request.action, Some(terminal_binding_request::Action::Ack(ack)) if ack.nonce == nonce.to_string())
        );
        let ready = state_changed(app, &self.terminal, |view| {
            matches!(
                view.remote_terminal_binding
                    .as_ref()
                    .map(|binding| &binding.phase),
                Some(Phase::Bound(_))
            )
        });
        wire.bound();
        bounded(ready).await.unwrap();
        self.terminal.read(app, |view, ctx| {
            let (client, scope, actual_attempt, id) =
                view.current_remote_terminal_binding(ctx).unwrap();
            assert!(Arc::ptr_eq(&client, &self.client));
            assert_eq!(scope.terminal_session_id, self.current_session.as_u64());
            assert_eq!(actual_attempt, attempt);
            assert_eq!(id, "10000000-0000-4000-8000-000000000002");
        });
    }

    pub(in crate::terminal::view) async fn close(self, app: &mut App) {
        self.terminal
            .update(app, |view, _| view.cancel_remote_terminal_binding());
        RemoteServerManager::handle(app).update(app, |manager, ctx| {
            manager.deregister_session(self.current_session, ctx)
        });
        bounded(self.done.recv()).await.unwrap();
    }
}

#[test]
fn terminal_binding_osc_before_written_waits_then_acks_original_connection() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let (attempt, wire) = fixture.begin(&mut app).await;
        let nonce = Uuid::new_v4();
        let seen = state_changed(&mut app, &fixture.terminal, |view| {
            matches!(
                view.remote_terminal_binding
                    .as_ref()
                    .map(|binding| &binding.phase),
                Some(Phase::Waiting {
                    written: false,
                    nonce: Some(_)
                })
            )
        });
        fixture.osc(&mut app, attempt, nonce);
        bounded(seen).await.unwrap();
        assert!(fixture.requests.try_recv().is_err());
        wire.written();
        fixture.finish_ack(&mut app, attempt, nonce).await;
        fixture.close(&mut app).await;
    });
}

#[test]
fn terminal_binding_written_before_osc_never_guesses_nonce() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let (attempt, wire) = fixture.begin(&mut app).await;
        let written = state_changed(&mut app, &fixture.terminal, |view| {
            matches!(
                view.remote_terminal_binding
                    .as_ref()
                    .map(|binding| &binding.phase),
                Some(Phase::Waiting {
                    written: true,
                    nonce: None
                })
            )
        });
        wire.written();
        bounded(written).await.unwrap();
        assert!(fixture.requests.try_recv().is_err());
        let nonce = Uuid::new_v4();
        fixture.osc(&mut app, attempt, nonce);
        fixture.finish_ack(&mut app, attempt, nonce).await;
        fixture.close(&mut app).await;
    });
}

#[test]
fn terminal_binding_failed_begin_does_not_ack_an_earlier_osc() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let (attempt, wire) = fixture.begin(&mut app).await;
        let seen = state_changed(&mut app, &fixture.terminal, |view| {
            matches!(
                view.remote_terminal_binding
                    .as_ref()
                    .map(|binding| &binding.phase),
                Some(Phase::Waiting { nonce: Some(_), .. })
            )
        });
        fixture.osc(&mut app, attempt, Uuid::new_v4());
        bounded(seen).await.unwrap();
        let failed = state_changed(&mut app, &fixture.terminal, |view| {
            view.remote_terminal_binding.is_none()
        });
        wire.respond(terminal_binding_response::Result::Failed(
            TerminalBindingFailed {
                code: "unavailable".into(),
            },
        ));
        bounded(failed).await.unwrap();
        assert!(fixture.requests.try_recv().is_err());
        let notification::Message::CancelTerminalBinding(cancel) =
            bounded(fixture.notifications.recv()).await.unwrap()
        else {
            panic!("失败必须取消原 attempt")
        };
        assert_eq!(cancel.attempt_id, attempt.to_string());
        fixture.close(&mut app).await;
    });
}

#[test]
fn terminal_binding_old_begin_callback_cannot_complete_new_attempt() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let (old_attempt, old_wire) = fixture.begin(&mut app).await;
        let new_attempt = fixture.terminal.update(&mut app, |view, ctx| {
            view.begin_remote_terminal_binding(ctx).unwrap()
        });
        old_wire.written();
        let notification::Message::CancelTerminalBinding(cancel) =
            bounded(fixture.notifications.recv()).await.unwrap()
        else {
            panic!("新 attempt 仅取消旧 attempt")
        };
        assert_eq!(cancel.attempt_id, old_attempt.to_string());
        let wire = bounded(fixture.requests.recv()).await.unwrap();
        assert_eq!(wire.request.attempt_id, new_attempt.to_string());
        // 先投递旧 OSC，再投递新 OSC；新 nonce 的状态回执同时证明前一事件已调度。
        fixture.osc(&mut app, old_attempt, Uuid::new_v4());
        let nonce = Uuid::new_v4();
        let seen = state_changed(
            &mut app,
            &fixture.terminal,
            move |view| matches!(view.remote_terminal_binding.as_ref().map(|binding| &binding.phase), Some(Phase::Waiting { written: false, nonce: Some(actual) }) if *actual == nonce),
        );
        fixture.osc(&mut app, new_attempt, nonce);
        bounded(seen).await.unwrap();
        assert!(fixture.requests.try_recv().is_err());
        wire.written();
        fixture.finish_ack(&mut app, new_attempt, nonce).await;
        fixture.close(&mut app).await;
    });
}

#[test]
fn terminal_binding_epoch_and_block_changes_invalidate_bound_identity() {
    for replace_block in [false, true] {
        App::test((), |mut app| async move {
            let fixture = Fixture::new(&mut app).await;
            let (attempt, wire) = fixture.begin(&mut app).await;
            wire.written();
            let nonce = Uuid::new_v4();
            fixture.osc(&mut app, attempt, nonce);
            fixture.finish_ack(&mut app, attempt, nonce).await;
            if replace_block {
                fixture.terminal.update(&mut app, |view, _| {
                    let mut model = view.model.lock();
                    model.finish_block();
                    model.simulate_long_running_block("tmux", "");
                    model
                        .block_list_mut()
                        .active_block_mut()
                        .set_session_id(session_id());
                });
            } else {
                fixture
                    .client
                    .notify_session_bootstrapped(session_id(), "zsh", None);
            }
            fixture.terminal.update(&mut app, |view, ctx| {
                assert!(view.current_remote_terminal_binding(ctx).is_none());
                view.invalidate_remote_terminal_binding(ctx);
                assert!(view.remote_terminal_binding.is_none());
            });
            fixture.close(&mut app).await;
        });
    }
}

#[test]
fn terminal_binding_reconnect_replays_candidates_but_never_bound_proof() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let (attempt, wire) = fixture.begin(&mut app).await;
        wire.written();
        let nonce = Uuid::new_v4();
        fixture.osc(&mut app, attempt, nonce);
        fixture.finish_ack(&mut app, attempt, nonce).await;
        let old_scope = fixture.client.terminal_binding_scope(session_id()).unwrap();
        let (disconnected, disconnected_receiver) = oneshot::channel();
        let mut disconnected = Some(disconnected);
        app.update(|ctx| {
            ctx.subscribe_to_model(&RemoteServerManager::handle(ctx), move |_, event, _| {
                if matches!(event, RemoteServerManagerEvent::SessionDisconnected { session_id: id, .. } if *id == session_id())
                    && let Some(sender) = disconnected.take()
                {
                    let _ = sender.send(());
                }
            });
        });
        // 先让真实旧传输 EOF 完成断连，再建立替代连接；不把两次连接安装叠在一起。
        bounded(fixture.disconnects.recv()).await.unwrap().abort();
        bounded(disconnected_receiver).await.unwrap();
        bounded(fixture.done.recv()).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            assert!(
                RemoteServerManager::as_ref(ctx)
                    .client_for_session(session_id())
                    .is_none()
            );
            assert!(view.current_remote_terminal_binding(ctx).is_none());
        });
        let (sender, receiver) = oneshot::channel();
        let mut sender = Some(sender);
        app.update(|ctx| {
            ctx.subscribe_to_model(&RemoteServerManager::handle(ctx), move |_, event, _| {
                if matches!(event, RemoteServerManagerEvent::SessionConnected { session_id: id, .. } if *id == session_id())
                    && let Some(sender) = sender.take()
                { let _ = sender.send(()); }
            });
        });
        RemoteServerManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.connect_session(
                session_id(),
                fixture.transport.clone(),
                Arc::new(RemoteServerAuthContext::new(
                    || Box::pin(async { None }),
                    || "terminal-binding-fixture".into(),
                )),
                None,
                ctx,
            );
        });
        bounded(receiver).await.unwrap();
        let notification::Message::SessionBootstrapped(bootstrap) =
            bounded(fixture.notifications.recv()).await.unwrap()
        else {
            panic!("重连必须重发缓存候选")
        };
        assert_eq!(bootstrap.shell_pid, Some(1234));
        assert_eq!(bootstrap.shell_tty.as_deref(), Some("/dev/pts/7"));
        assert_ne!(bootstrap.image_staging_epoch, old_scope.terminal_epoch);
        fixture.terminal.read(&app, |view, ctx| {
            let current = RemoteServerManager::as_ref(ctx)
                .client_for_session(session_id())
                .unwrap();
            assert!(!Arc::ptr_eq(current, &fixture.client));
            assert!(view.current_remote_terminal_binding(ctx).is_none());
        });
        assert!(fixture.requests.try_recv().is_err());
        let (new_attempt, begin) = fixture.begin(&mut app).await;
        assert_ne!(new_attempt, attempt);
        begin.written();
        let new_nonce = Uuid::new_v4();
        fixture.osc(&mut app, new_attempt, new_nonce);
        let ack = bounded(fixture.requests.recv()).await.unwrap();
        assert!(matches!(&ack.request.action,
            Some(terminal_binding_request::Action::Ack(value)) if value.nonce == new_nonce.to_string()));
        let ready = state_changed(&mut app, &fixture.terminal, move |view| {
            view.remote_terminal_binding
                .as_ref()
                .is_some_and(|binding| {
                    binding.attempt == new_attempt && matches!(binding.phase, Phase::Bound(_))
                })
        });
        ack.bound();
        bounded(ready).await.unwrap();
        assert!(
            fixture
                .client
                .begin_terminal_binding(old_scope, Uuid::new_v4())
                .await
                .is_err()
        );
        fixture.osc(&mut app, attempt, nonce);
        fixture.terminal.read(&app, |view, ctx| {
            let (client, _, current_attempt, _) =
                view.current_remote_terminal_binding(ctx).unwrap();
            assert!(!Arc::ptr_eq(&client, &fixture.client));
            assert_eq!(current_attempt, new_attempt);
        });
        assert!(fixture.requests.try_recv().is_err());
        fixture.close(&mut app).await;
    });
}

#[test]
fn terminal_binding_rejects_duplicate_and_foreign_challenges_and_acks_once() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let (attempt, wire) = fixture.begin(&mut app).await;
        fixture.terminal.update(&mut app, |view, _| {
            let binding = view.remote_terminal_binding.as_mut().unwrap();
            let nonce = Uuid::new_v4();
            let mut challenge = TerminalBindingChallenge {
                session_id: session_id(),
                attempt,
                nonce,
            };
            challenge.session_id = SessionId::from(7502);
            assert!(!binding.accept_challenge(&challenge));
            challenge.session_id = session_id();
            challenge.attempt = Uuid::new_v4();
            assert!(!binding.accept_challenge(&challenge));
            challenge.attempt = attempt;
            assert!(binding.accept_challenge(&challenge));
            assert!(!binding.accept_challenge(&challenge));
            challenge.nonce = Uuid::new_v4();
            assert!(!binding.accept_challenge(&challenge));
            assert!(binding.take_ack().is_none());
            if let Phase::Waiting { written, .. } = &mut binding.phase {
                *written = true;
            }
            assert_eq!(binding.take_ack(), Some(nonce));
            assert!(binding.take_ack().is_none());
            assert!(!binding.accept_challenge(&challenge));
        });
        wire.respond(terminal_binding_response::Result::Failed(
            TerminalBindingFailed {
                code: "fixture-stop".into(),
            },
        ));
        fixture.close(&mut app).await;
    });
}

#[test]
fn terminal_binding_old_ack_callback_cannot_publish_into_new_attempt() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let (old_attempt, wire) = fixture.begin(&mut app).await;
        wire.written();
        fixture.osc(&mut app, old_attempt, Uuid::new_v4());
        let old_ack = bounded(fixture.requests.recv()).await.unwrap();
        assert!(matches!(
            old_ack.request.action,
            Some(terminal_binding_request::Action::Ack(_))
        ));
        let new_attempt = fixture.terminal.update(&mut app, |view, ctx| {
            view.begin_remote_terminal_binding(ctx).unwrap()
        });
        old_ack.respond(terminal_binding_response::Result::Bound(
            TerminalBindingBound {
                pane_cwd: "/fixture/project".into(),
                opaque_binding_id: "stale-binding".into(),
            },
        ));
        let notification::Message::CancelTerminalBinding(cancel) =
            bounded(fixture.notifications.recv()).await.unwrap()
        else {
            panic!("只取消已替换的 attempt")
        };
        assert_eq!(cancel.attempt_id, old_attempt.to_string());
        let new_wire = bounded(fixture.requests.recv()).await.unwrap();
        assert_eq!(new_wire.request.attempt_id, new_attempt.to_string());
        let nonce = Uuid::new_v4();
        let seen = state_changed(&mut app, &fixture.terminal, move |view| {
            view.remote_terminal_binding.as_ref().is_some_and(|binding| binding.attempt == new_attempt
                && matches!(binding.phase, Phase::Waiting { written: false, nonce: Some(actual) } if actual == nonce))
        });
        fixture.osc(&mut app, new_attempt, nonce);
        bounded(seen).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            assert!(view.current_remote_terminal_binding(ctx).is_none())
        });
        new_wire.written();
        fixture.finish_ack(&mut app, new_attempt, nonce).await;
        fixture.close(&mut app).await;
    });
}
