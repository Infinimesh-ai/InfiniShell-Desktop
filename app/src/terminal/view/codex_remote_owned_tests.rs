//! 零轮只读证明经过真实远端握手、终端元数据与 UI 回调，不发送 RPC 图片或模型输入。
use super::*;
use crate::remote_server::auth::RemoteServerAuthContext;
use crate::remote_server::cli_image_codex_owned_protocol::{
    Request as OwnedRequest, Ticket, encode,
};
use crate::remote_server::manager::{RemoteServerExitStatus, RemoteServerManagerEvent};
use crate::remote_server::proto::{
    CliCodexOwnedResponse, InitializeResponse, NavigatedToDirectoryResponse,
    RemoteServerCapability, ServerMessage, client_message, notification, server_message,
    session_scoped_request,
};
use crate::remote_server::protocol;
use crate::remote_server::setup::{PreinstallCheckResult, RemotePlatform};
use crate::remote_server::transport::{
    Connection, ControlPath, Error, InstallOutcome, RemoteTransport, TransportConnection,
};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModelEvent;
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::ansi::Handler;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::{BootstrapSessionType, SessionInfo};
use crate::terminal::model_events::{AnsiHandlerEvent, ModelEvent};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use async_compat::CompatExt;
use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Either, select};
use futures::io::AsyncWriteExt;
use std::future::Future;
use std::pin::Pin;
use warp_core::channel::ChannelState;
use warpui::r#async::{Timer, executor};
use warpui::{App, ViewHandle};

fn remote_session() -> SessionId {
    SessionId::from(7401)
}
const NATIVE: &str = "20000000-0000-4000-8000-000000000001";
#[derive(Debug, Clone)]
struct MemoryTransport {
    done: async_channel::Sender<()>,
    observations: Option<async_channel::Sender<ObserveRequest>>,
}

struct ObserveRequest {
    ticket: Ticket,
    scope: CliImageStagingScope,
    revision: u64,
    reply: oneshot::Sender<Reply>,
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
        let observations = self.observations.clone();
        Box::pin(async move {
            let (client_stream, server_stream) = tokio::io::duplex(8192);
            let (client_read, client_write) = tokio::io::split(client_stream);
            let (server_read, server_write) = tokio::io::split(server_stream);
            let (client, event_rx, failure_rx, host_response_rx) =
                RemoteServerClient::new(client_read.compat(), client_write.compat(), &executor);
            let (abort, registration) = AbortHandle::new_pair();
            executor.spawn(async move {
                let server = async move {
                    let mut reader = server_read.compat();
                    let mut writer = server_write.compat();
                    let mut bootstrapped_session = None;
                    loop {
                        let message = match protocol::read_client_message(&mut reader).await {
                            Ok(message) => message,
                            Err(protocol::ProtocolError::UnexpectedEof) => break,
                            Err(error) => panic!("内存协议读取失败：{error:?}"),
                        };
                        match message.message {
                            Some(client_message::Message::SessionScoped(envelope)) => {
                                let response = match envelope.message {
                                    Some(session_scoped_request::Message::Initialize(_)) => {
                                        server_message::Message::InitializeResponse(InitializeResponse {
                                            server_version: ChannelState::app_version().unwrap_or_default().to_owned(),
                                            host_id: "predicate-fixture-daemon".to_owned(),
                                            capabilities: vec![
                                                RemoteServerCapability::CliImageUnpublishedStagingV1.into(),
                                                RemoteServerCapability::CliImageCodexQueueV1.into(),
                                                RemoteServerCapability::CliImageReferenceRecoveryV1.into(),
                                                RemoteServerCapability::CliCodexOwnedV1.into(),
                                            ],
                                        })
                                    }
                                    Some(session_scoped_request::Message::NavigatedToDirectory(request)) => {
                                        // 导航消息没有独立 SID，只能回应这条连接已 bootstrap 的固定会话和目录。
                                        assert_eq!(bootstrapped_session, Some(remote_session().as_u64()));
                                        assert_eq!(request.path, "/fixture/project");
                                        server_message::Message::NavigatedToDirectoryResponse(NavigatedToDirectoryResponse {
                                            indexed_path: request.path,
                                            is_git: false,
                                        })
                                    }
                                    Some(session_scoped_request::Message::CliCodexOwned(request)) => {
                                        let scope = request.scope.unwrap();
                                        let request = OwnedRequest::decode(&request.body_json).unwrap();
                                        assert_eq!(request.scope.host, scope.host_id);
                                        assert_eq!(request.scope.terminal_session, scope.terminal_session_id);
                                        assert_eq!(request.scope.terminal_epoch.to_string(), scope.terminal_epoch);
                                        assert_eq!(request.scope.generation.to_string(), scope.input_generation);
                                        let ticket = match request.action {
                                            Action::Observe { ticket, expected_session } => {
                                                assert!(expected_session.is_none());
                                                ticket
                                            }
                                            Action::Reserve { .. } | Action::Status { .. }
                                            | Action::Cancel { .. } | Action::Revoke { .. } => {
                                                panic!("重试夹具只接受只读 Observe")
                                            }
                                        };
                                        let (reply, receiver) = oneshot::channel();
                                        observations.as_ref().expect("当前用例未启用 Observe").send(ObserveRequest {
                                            ticket,
                                            scope: scope.clone(),
                                            revision: request.revision,
                                            reply,
                                        }).await.unwrap_or_else(|_| panic!("只读观察接收端已关闭"));
                                        let reply = receiver.await.expect("测试未回应只读 Observe");
                                        server_message::Message::CliCodexOwned(CliCodexOwnedResponse {
                                            scope: Some(scope),
                                            body_json: encode(&reply).unwrap(),
                                        })
                                    }
                                    _ => panic!("测试禁止上传或原生队列请求"),
                                };
                                protocol::write_server_message(&mut writer, &ServerMessage {
                                    request_id: message.request_id,
                                    message: Some(response),
                                }).await.unwrap();
                                writer.flush().await.unwrap();
                            }
                            Some(client_message::Message::Notification(notice)) => {
                                if let Some(notification::Message::SessionBootstrapped(session)) = notice.message {
                                    assert_eq!(session.session_id, remote_session().as_u64());
                                    bootstrapped_session = Some(session.session_id);
                                }
                            }
                            _ => panic!("内存协议夹具不接受其他消息"),
                        }
                    }
                };
                let _ = Abortable::new(server, registration).await;
                // 断言失败时接收端可能先析构，正常收尾不能制造第二次 panic。
                let _ = done.send(()).await;
            }).detach();
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

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    match select(
        Box::pin(future),
        Box::pin(Timer::after(Duration::from_secs(5))),
    )
    .await
    {
        Either::Left((result, _)) => result,
        Either::Right(_) => panic!("内存连接或输入事件未完成"),
    }
}

async fn finish_initial_bootstrap(app: &mut App, terminal: &ViewHandle<TerminalView>) {
    let events = terminal.read(app, |view, _| view.model_event_dispatcher().clone());
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    app.update(|ctx| {
        ctx.subscribe_to_model(&events, move |events, event, ctx| {
            // 构造器的最后一个 precmd 与前面的本地 metadata 经过同一 FIFO；
            // 等它完成后才设置远端会话，避免旧本地事件覆盖新的视图缓存。
            if matches!(event, ModelEvent::Handler(AnsiHandlerEvent::Precmd))
                && let Some(sender) = sender.take()
            {
                assert_eq!(
                    events.as_ref(ctx).active_session_id(),
                    Some(SessionId::from(123))
                );
                let _ = sender.send(());
            }
        });
    });
    bounded(receiver).await.unwrap();
    terminal.read(app, |view, _| {
        assert_eq!(view.active_block_session_id(), Some(SessionId::from(123)));
    });
}

async fn connect(app: &mut App, transport: MemoryTransport) -> Arc<RemoteServerClient> {
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    app.update(|ctx| {
        ctx.subscribe_to_model(&RemoteServerManager::handle(ctx), move |_, event, _| {
            if matches!(event, RemoteServerManagerEvent::SessionConnected { session_id, .. } if *session_id == remote_session())
                && let Some(sender) = sender.take()
            {
                sender.send(()).unwrap();
            }
        });
    });
    RemoteServerManager::handle(app).update(app, |manager, ctx| {
        manager.connect_session(
            remote_session(),
            transport,
            Arc::new(RemoteServerAuthContext::new(
                || Box::pin(async { None }),
                || "predicate-fixture".to_owned(),
            )),
            None,
            ctx,
        );
    });
    bounded(receiver).await.unwrap();
    RemoteServerManager::handle(app).read(app, |manager, _| {
        manager
            .client_for_session(remote_session())
            .unwrap()
            .clone()
    })
}

fn change_terminal_session(
    view: &mut TerminalView,
    session_id: SessionId,
    ctx: &mut ViewContext<TerminalView>,
) {
    let block_index = {
        let mut model = view.model.lock();
        model
            .block_list_mut()
            .active_block_mut()
            .set_session_id(session_id);
        model
            .block_list_mut()
            .active_block_mut()
            .set_current_working_directory("/fixture/project".into());
        model.block_list().active_block_index()
    };
    // shell 元数据既更新 block，也经过事件同步 view/Input 缓存；只改 block 不构成远端会话。
    view.handle_terminal_event(
        &ModelEvent::BlockMetadataReceived(BlockMetadataReceivedEvent {
            block_metadata: BlockMetadata::new(Some(session_id), Some("/fixture/project".into())),
            block_index,
            is_after_in_band_command: false,
            is_done_bootstrapping: true,
        }),
        ctx,
    );
    assert_eq!(view.active_block_session_id(), Some(session_id));
}

struct Fixture {
    terminal: ViewHandle<TerminalView>,
    client: Arc<RemoteServerClient>,
    launch: Launch,
    snapshot: Snapshot,
    scope: CliImageStagingScope,
    owner: Owner,
    session: ReadOnlySession,
    done: async_channel::Receiver<()>,
}

impl Fixture {
    async fn new(app: &mut App) -> Self {
        Self::with_observations(app, None).await
    }

    async fn with_observations(
        app: &mut App,
        observations: Option<async_channel::Sender<ObserveRequest>>,
    ) -> Self {
        initialize_app_for_terminal_view(app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        crate::features::FeatureFlag::CLIAgentRichInput.set_enabled(true);
        crate::features::FeatureFlag::CodexPlugin.set_enabled(true);
        let (sender, done) = async_channel::unbounded();
        let client = connect(
            app,
            MemoryTransport {
                done: sender,
                observations,
            },
        )
        .await;
        let terminal = add_window_with_terminal(app, None);
        finish_initial_bootstrap(app, &terminal).await;
        let (launch, snapshot, scope) = terminal.update(app, |view, ctx| {
            let mut info = SessionInfo::new_for_test()
                .with_session_type(BootstrapSessionType::WarpifiedRemote)
                .with_shell_type(ShellType::Bash);
            info.session_id = remote_session();
            view.sessions
                .update(ctx, |sessions, _| sessions.register_session_for_test(info));
            view.model
                .lock()
                .simulate_long_running_block("owned-codex-wrapper", "");
            change_terminal_session(view, remote_session(), ctx);
            RemoteServerManager::handle(ctx).update(ctx, |manager, _| {
                manager.notify_session_bootstrapped(remote_session(), "bash", Some("/bin/bash"));
            });
            let (snapshot, current_client) = view.remote_codex_snapshot(true, ctx).unwrap();
            assert!(Arc::ptr_eq(&client, &current_client));
            let launch = Launch {
                tmux_source: None,
                tmux_terminal_session: None,
                host: "predicate-fixture-daemon".into(),
                terminal_session: remote_session().as_u64(),
                block_id: snapshot.block.to_string(),
                cwd: snapshot.cwd.clone(),
                ticket: Ticket {
                    id: Uuid::new_v4(),
                    key: Uuid::new_v4(),
                },
            };
            let (scope, _) = remote::scope(&client, &launch, Uuid::new_v4()).unwrap();
            view.codex_remote_owned = Some(RemoteOwned {
                tmux_instance: None,
                launch: launch.clone(),
                owner: None,
                observation: None,
                native_session_id: None,
                read_only_scope: None,
                snapshot: snapshot.clone(),
                client: client.clone(),
            });
            (launch, snapshot, scope)
        });
        let owner = Owner {
            ticket_id: launch.ticket.id,
            manifest_sha256: "a".repeat(64),
            socket_path: "/fixture/control.sock".into(),
            tui_pid: 102,
            server_pid: 101,
            tty_device: 3,
            pinned_native_session_id: Some(Uuid::parse_str(NATIVE).unwrap()),
        };
        let session = ReadOnlySession {
            native_session_id: Uuid::parse_str(NATIVE).unwrap(),
            process: CodexProcessEvidence {
                daemon_pid_candidate: 101,
                codex_home: "/fixture/codex".into(),
                tty_path: "/dev/pts/7".into(),
            },
        };
        Self {
            terminal,
            client,
            launch,
            snapshot,
            scope,
            owner,
            session,
            done,
        }
    }

    fn accept(&self, app: &mut App) {
        self.terminal.update(app, |view, ctx| {
            view.accept_remote_owned_codex_session(
                &self.launch,
                &self.snapshot,
                &self.client,
                self.scope.clone(),
                self.owner.clone(),
                self.session.clone(),
                ctx,
            )
        });
    }

    fn current(&self, app: &App) -> bool {
        self.terminal.read(app, |view, ctx| {
            view.remote_owned_codex_image_proof(ctx).is_some()
        })
    }

    async fn close(self, app: &mut App) {
        RemoteServerManager::handle(app).update(app, |manager, ctx| {
            manager.deregister_session(remote_session(), ctx)
        });
        bounded(self.done.recv()).await.unwrap();
    }
}

fn started(app: &mut App, terminal: &ViewHandle<TerminalView>) -> oneshot::Receiver<()> {
    let view_id = terminal.read(app, |view, _| view.view_id);
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    app.update(|ctx| {
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), move |_, event, _| {
            if matches!(event, CLIAgentSessionsModelEvent::Started { terminal_view_id, .. } if *terminal_view_id == view_id)
                && let Some(sender) = sender.take() {
                let _ = sender.send(());
            }
        });
    });
    receiver
}

#[test]
fn readonly_session_opens_image_target_without_claiming_a_rich_notification() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let event = started(&mut app, &fixture.terminal);
        fixture.accept(&mut app);
        bounded(event).await.unwrap();
        assert!(fixture.current(&app));
        fixture.terminal.read(&app, |view, ctx| {
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            let session = sessions.session(view.view_id).unwrap();
            assert!(!session.received_rich_notification);
            assert!(!session.supports_rich_status());
            assert!(session.session_context.codex_process_evidence.is_none());
            assert_eq!(session.session_context.session_id.as_deref(), Some(NATIVE));
            assert!(view.cli_agent_input_target_matches(
                sessions.input_generation(view.view_id).unwrap(),
                ctx
            ));
        });
        fixture.close(&mut app).await;
    });
}

#[test]
fn bootstrap_callback_rejects_changed_epoch() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        RemoteServerManager::handle(&app).update(&mut app, |manager, _| {
            manager.notify_session_bootstrapped(remote_session(), "bash", Some("/bin/bash"));
        });
        fixture.accept(&mut app);
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn bootstrap_callback_rejects_replaced_block() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.terminal.update(&mut app, |view, _| {
            let mut model = view.model.lock();
            model.finish_block();
            model.simulate_long_running_block("new-wrapper", "");
            let block = model.block_list_mut().active_block_mut();
            block.set_session_id(remote_session());
            block.set_current_working_directory("/fixture/project".into());
        });
        fixture.accept(&mut app);
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn bootstrap_callback_cannot_overwrite_a_new_ticket() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.terminal.update(&mut app, |view, _| {
            view.codex_remote_owned.as_mut().unwrap().launch.ticket.id = Uuid::new_v4();
        });
        fixture.accept(&mut app);
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn accepted_readonly_proof_expires_on_epoch_change() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.accept(&mut app);
        assert!(fixture.current(&app));
        RemoteServerManager::handle(&app).update(&mut app, |manager, _| {
            manager.notify_session_bootstrapped(remote_session(), "bash", Some("/bin/bash"));
        });
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn readonly_proof_does_not_rebind_to_another_native_session() {
    App::test((), |mut app| async move {
        let mut fixture = Fixture::new(&mut app).await;
        fixture.accept(&mut app);
        fixture.session.native_session_id = Uuid::new_v4();
        fixture.accept(&mut app);
        fixture.terminal.read(&app, |view, ctx| {
            assert_eq!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .session_context
                    .session_id
                    .as_deref(),
                Some(NATIVE)
            );
        });
        assert!(fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn actual_hook_upgrades_readonly_session_without_changing_its_binding() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let event = started(&mut app, &fixture.terminal);
        fixture.accept(&mut app);
        bounded(event).await.unwrap();
        fixture.terminal.update(&mut app, |view, ctx| {
            view.model_events_handle.update(ctx, |_, ctx| {
                ctx.emit(ModelEvent::PluggableNotification {
                    title: Some("warp://cli-agent".into()),
                    body: r#"{"v":1,"agent":"codex","event":"session_start","session_id":"20000000-0000-4000-8000-000000000001","codex_process_evidence":{"daemon_pid_candidate":101,"codex_home":"/fixture/codex","tty_path":"/dev/pts/7"},"plugin_version":"0.4.0"}"#.into(),
                });
            });
        });
        fixture.terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .unwrap();
            assert!(session.received_rich_notification);
            assert!(session.supports_rich_status());
        });
        assert!(fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn stale_hook_for_another_session_cannot_replace_current_readonly_proof() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.accept(&mut app);
        fixture.terminal.update(&mut app, |view, ctx| {
            view.model_events_handle.update(ctx, |_, ctx| {
                ctx.emit(ModelEvent::PluggableNotification {
                    title: Some("warp://cli-agent".into()),
                    body: r#"{"v":1,"agent":"codex","event":"session_start","session_id":"20000000-0000-4000-8000-000000000002","codex_process_evidence":{"daemon_pid_candidate":101,"codex_home":"/fixture/codex","tty_path":"/dev/pts/7"},"plugin_version":"0.4.0"}"#.into(),
                });
            });
        });
        fixture.terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .unwrap();
            assert_eq!(session.session_context.session_id.as_deref(), Some(NATIVE));
            assert!(!session.received_rich_notification);
        });
        assert!(fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn new_session_after_listener_retirement_cannot_restore_the_old_image_binding() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.accept(&mut app);
        assert!(fixture.current(&app));
        // 先走已有会话退休事件，再经终端通知入口注册新 SID，不能把外来旧 hook 当成换会话。
        fixture.terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.remove_session(view.view_id, ctx);
            });
        });
        assert!(!fixture.current(&app));
        let event = started(&mut app, &fixture.terminal);
        fixture.terminal.update(&mut app, |view, ctx| {
            view.model_events_handle.update(ctx, |_, ctx| {
                ctx.emit(ModelEvent::PluggableNotification {
                    title: Some("warp://cli-agent".into()),
                    body: r#"{"v":1,"agent":"codex","event":"session_start","session_id":"20000000-0000-4000-8000-000000000002","codex_process_evidence":{"daemon_pid_candidate":101,"codex_home":"/fixture/codex","tty_path":"/dev/pts/7"},"plugin_version":"0.4.0"}"#.into(),
                });
            });
        });
        bounded(event).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .unwrap();
            assert_eq!(
                session.session_context.session_id.as_deref(),
                Some("20000000-0000-4000-8000-000000000002")
            );
            assert!(session.received_rich_notification);
            let owned = view.codex_remote_owned.as_ref().unwrap();
            assert_eq!(
                owned.native_session_id,
                Some(Uuid::parse_str(NATIVE).unwrap())
            );
            assert!(owned.owner.is_none());
            assert!(owned.read_only_scope.is_none());
        });
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn replacing_the_listener_invalidates_a_readonly_proof() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.accept(&mut app);
        assert!(fixture.current(&app));
        fixture.terminal.update(&mut app, |view, ctx| {
            let view_id = view.view_id;
            let events = view.model_events_handle.clone();
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(view_id, CLIAgent::Codex, &events, ctx)
            });
            let remote = view.active_session_remote_host(ctx);
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.register_listener(
                    view_id,
                    CLIAgent::Codex,
                    Some("/fixture/project".into()),
                    None,
                    Some(NATIVE.into()),
                    None,
                    remote,
                    false,
                    listener,
                    ctx,
                );
            });
        });
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn disconnect_invalidates_a_readonly_proof_before_any_new_input() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.accept(&mut app);
        assert!(fixture.current(&app));
        RemoteServerManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.deregister_session(remote_session(), ctx);
        });
        bounded(fixture.done.recv()).await.unwrap();
        assert!(!fixture.current(&app));
    });
}

#[test]
fn pending_bootstrap_survives_an_observer_call_before_native_context_exists() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let generation = Uuid::new_v4();
        fixture.terminal.update(&mut app, |view, ctx| {
            view.codex_remote_restore_generation = Some(generation);
            view.observe_remote_owned_codex_start(ctx);
            assert_eq!(view.codex_remote_restore_generation, Some(generation));
        });
        fixture.accept(&mut app);
        assert!(fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn pending_bootstrap_survives_a_command_detected_started_event() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let event = started(&mut app, &fixture.terminal);
        let generation = Uuid::new_v4();
        fixture.terminal.update(&mut app, |view, ctx| {
            view.codex_remote_restore_generation = Some(generation);
            let view_id = view.view_id;
            let events = view.model_events_handle.clone();
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(view_id, CLIAgent::Codex, &events, ctx)
            });
            let remote = view.active_session_remote_host(ctx);
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.register_listener(
                    view_id,
                    CLIAgent::Codex,
                    Some("/fixture/project".into()),
                    None,
                    None,
                    None,
                    remote,
                    false,
                    listener,
                    ctx,
                );
            });
        });
        bounded(event).await.unwrap();
        fixture.terminal.read(&app, |view, _| {
            assert_eq!(view.codex_remote_restore_generation, Some(generation))
        });
        fixture.accept(&mut app);
        assert!(fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn bootstrap_rejects_an_owner_pinned_to_another_native_session() {
    App::test((), |mut app| async move {
        let mut fixture = Fixture::new(&mut app).await;
        fixture.owner.pinned_native_session_id = Some(Uuid::new_v4());
        fixture.accept(&mut app);
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn unpinned_owner_does_not_grant_readonly_access() {
    App::test((), |mut app| async move {
        let mut fixture = Fixture::new(&mut app).await;
        fixture.owner.pinned_native_session_id = None;
        assert!(owner_session_matches(&fixture.owner, NATIVE));
        fixture.accept(&mut app);
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn status_restore_checks_the_daemon_pin_before_advertising_image_input() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        assert!(owner_session_matches(&fixture.owner, NATIVE));
        assert!(!owner_session_matches(
            &fixture.owner,
            "20000000-0000-4000-8000-000000000002"
        ));
        fixture.close(&mut app).await;
    });
}

#[test]
fn bootstrap_retries_after_not_ready_and_keeps_observing_through_started() {
    App::test((), |mut app| async move {
        let (sender, observations) = async_channel::unbounded();
        let fixture = Fixture::with_observations(&mut app, Some(sender)).await;
        fixture.terminal.update(&mut app, |view, ctx| {
            view.observe_remote_owned_codex_bootstrap(0, ctx);
        });
        let first = bounded(observations.recv()).await.unwrap();
        assert!(first.ticket == fixture.launch.ticket);
        assert!(!fixture.current(&app));

        // 真正的首次 Observe 尚未回应；命令检测的 Started 不得丢掉正在等候的任务。
        let detected = started(&mut app, &fixture.terminal);
        fixture.terminal.update(&mut app, |view, ctx| {
            let view_id = view.view_id;
            let events = view.model_events_handle.clone();
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(view_id, CLIAgent::Codex, &events, ctx)
            });
            let remote = view.active_session_remote_host(ctx);
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.register_listener(
                    view_id,
                    CLIAgent::Codex,
                    Some("/fixture/project".into()),
                    None,
                    None,
                    None,
                    remote,
                    false,
                    listener,
                    ctx,
                );
            });
        });
        bounded(detected).await.unwrap();
        fixture.terminal.read(&app, |view, _| {
            assert_eq!(
                view.codex_remote_restore_generation,
                Some(Uuid::parse_str(&first.scope.input_generation).unwrap())
            );
        });
        first
            .reply
            .send(Reply::Failed {
                code: "not_ready".into(),
            })
            .unwrap_or_else(|_| panic!("首次 Observe 已提前取消"));

        // 第二次 wire 请求证明失败结果确实经过 UI 回调并重新调度，不靠等待时长判成功。
        let second = bounded(observations.recv()).await.unwrap();
        assert!(second.ticket == fixture.launch.ticket);
        assert_eq!(second.scope.terminal_epoch, first.scope.terminal_epoch);
        assert_ne!(second.scope.input_generation, first.scope.input_generation);
        assert!(second.revision > first.revision);
        assert!(!fixture.current(&app));
        fixture.terminal.read(&app, |view, _| {
            assert_eq!(
                view.codex_remote_restore_generation,
                Some(Uuid::parse_str(&second.scope.input_generation).unwrap())
            );
        });
        let (sender, bound) = oneshot::channel();
        let mut sender = Some(sender);
        let terminal = fixture.terminal.clone();
        let window_id = app.update(|ctx| terminal.window_id(ctx));
        // 同 agent 的 listener 升级不重复 Started，等待 accept 最后 notify 后的真实证明。
        app.on_window_invalidated(window_id, move |_, ctx| {
            let ready = terminal.read(ctx, |view, ctx| {
                view.codex_remote_restore_generation.is_none()
                    && view.remote_owned_codex_image_proof(ctx).is_some()
                    && CLIAgentSessionsModel::as_ref(ctx)
                        .session(view.view_id)
                        .is_some_and(|session| {
                            session.session_context.session_id.as_deref() == Some(NATIVE)
                        })
            });
            if ready && let Some(sender) = sender.take() {
                let _ = sender.send(());
            }
        });
        second
            .reply
            .send(Reply::Observed {
                ticket: fixture.launch.ticket.clone(),
                owner: fixture.owner.clone(),
                session: fixture.session.clone(),
            })
            .unwrap_or_else(|_| panic!("第二次 Observe 已提前取消"));
        bounded(bound).await.unwrap();
        assert!(fixture.current(&app));
        fixture.terminal.read(&app, |view, ctx| {
            assert!(view.codex_remote_restore_generation.is_none());
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            let session = sessions.session(view.view_id).unwrap();
            assert!(!session.received_rich_notification);
            assert_eq!(session.session_context.session_id.as_deref(), Some(NATIVE));
            assert!(view.cli_agent_input_target_matches(
                sessions.input_generation(view.view_id).unwrap(),
                ctx
            ));
        });
        fixture.close(&mut app).await;
    });
}
