//! 真实客户端握手、终端 block 和 CLI 通知绑定的只读消费回执门禁。

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use async_compat::CompatExt;
use futures::channel::oneshot;
use futures::future::{AbortHandle, Abortable, Either, select};
use futures::io::AsyncWriteExt;
use warp_core::SessionId;
use warp_core::channel::ChannelState;
use warpui::r#async::{Timer, executor};
use warpui::{App, ViewHandle};

use super::*;
use crate::remote_server::auth::RemoteServerAuthContext;
use crate::remote_server::client::InitializeParams;
use crate::remote_server::manager::RemoteServerExitStatus;
use crate::remote_server::proto::{
    InitializeResponse, RemoteServerCapability, ServerMessage, client_message, server_message,
    session_scoped_request,
};
use crate::remote_server::protocol;
use crate::remote_server::setup::{PreinstallCheckResult, RemotePlatform};
use crate::remote_server::transport::{
    Connection, ControlPath, Error, InstallOutcome, RemoteTransport, TransportConnection,
};
use crate::terminal::cli_agent_sessions::CLIAgentRichInputCloseReason;
use crate::terminal::cli_agent_sessions::listener::CLIAgentSessionListener;
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::{BootstrapSessionType, SessionInfo};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn remote_session() -> SessionId {
    SessionId::from(7301)
}
const NOTIFICATION: &str = r#"{"v":1,"agent":"claude","event":"session_start","session_id":"10000000-0000-4000-8000-000000000001","cwd":"/fixture/project","transcript_path":"/fixture/claude/history.jsonl","claude_process_evidence":{"process_id_candidate":101,"claude_config_directory":"/fixture/claude","tty_path":"/dev/pts/7"},"plugin_version":"0.5.0"}"#;

#[derive(Debug, Clone)]
struct MemoryTransport {
    done: async_channel::Sender<()>,
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
                    loop {
                        let message = match protocol::read_client_message(&mut reader).await {
                            Ok(message) => message,
                            Err(protocol::ProtocolError::UnexpectedEof) => break,
                            Err(error) => panic!("内存协议读取失败：{error:?}"),
                        };
                        match message.message {
                            Some(client_message::Message::SessionScoped(envelope)) => {
                                assert!(matches!(envelope.message, Some(session_scoped_request::Message::Initialize(_))), "测试禁止上传或原生队列请求");
                                protocol::write_server_message(&mut writer, &ServerMessage {
                                    request_id: message.request_id,
                                    message: Some(server_message::Message::InitializeResponse(InitializeResponse {
                                        server_version: ChannelState::app_version().unwrap_or_default().to_owned(),
                                        host_id: "predicate-fixture-daemon".to_owned(),
                                        capabilities: vec![
                                            RemoteServerCapability::CliImageUnpublishedStagingV1.into(),
                                            RemoteServerCapability::CliImageClaudeReadV1.into(),
                                        ],
                                    })),
                                }).await.unwrap();
                                writer.flush().await.unwrap();
                            }
                            Some(client_message::Message::Notification(_)) => {}
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

fn next_visibility(app: &mut App, terminal: &ViewHandle<TerminalView>) -> oneshot::Receiver<()> {
    let view_id = terminal.read(app, |view, _| view.view_id);
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    app.update(|ctx| {
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), move |_, event, _| {
            if matches!(event, CLIAgentSessionsModelEvent::InputSessionChanged { terminal_view_id, .. } if *terminal_view_id == view_id)
                && let Some(sender) = sender.take()
            {
                sender.send(()).unwrap();
            }
        });
    });
    receiver
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
        model.block_list().active_block_index()
    };
    // shell 元数据既更新 block，也经过事件同步 view/Input 缓存；只改 block 不构成远端会话。
    view.handle_terminal_event(
        &ModelEvent::BlockMetadataReceived(BlockMetadataReceivedEvent {
            block_metadata: BlockMetadata::new(Some(session_id), None),
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
    submission: Arc<RemoteImageSubmission>,
    binding: NativeImageBinding,
    transport: MemoryTransport,
    done: async_channel::Receiver<()>,
}

impl Fixture {
    async fn new(app: &mut App, claimed: bool) -> Self {
        initialize_app_for_terminal_view(app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        crate::features::FeatureFlag::CLIAgentRichInput.set_enabled(true);
        let (done_sender, done) = async_channel::unbounded();
        let transport = MemoryTransport { done: done_sender };
        // 先握手，再创建终端；未 bootstrap 时的通知无法分配恢复 scope，不会读取真实账本。
        let client = connect(app, transport.clone()).await;
        let terminal = add_window_with_terminal(app, None);
        let (sender, receiver) = oneshot::channel();
        let mut sender = Some(sender);
        let view_id = terminal.read(app, |view, _| view.view_id);
        app.update(|ctx| {
            ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), move |_, event, _| {
                if matches!(event, CLIAgentSessionsModelEvent::InputSessionChanged { terminal_view_id, .. } if *terminal_view_id == view_id)
                    && let Some(sender) = sender.take()
                {
                    sender.send(()).unwrap();
                }
            });
        });
        terminal.update(app, |view, ctx| {
            let mut info = SessionInfo::new_for_test()
                .with_session_type(BootstrapSessionType::WarpifiedRemote);
            info.session_id = remote_session();
            view.sessions
                .update(ctx, |sessions, _| sessions.register_session_for_test(info));
            view.model.lock().simulate_long_running_block("claude", "");
            change_terminal_session(view, remote_session(), ctx);
            assert!(view.active_session_remote_host(ctx).is_some());
            view.handle_cli_agent_notification(Some("warp://cli-agent"), NOTIFICATION, ctx);
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        });
        bounded(receiver).await.unwrap();
        let (submission, binding) = terminal.update(app, |view, ctx| {
            RemoteServerManager::handle(ctx).update(ctx, |manager, _| {
                manager.notify_session_bootstrapped(remote_session(), "bash", Some("/bin/bash"));
            });
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("原图片草稿", ctx)
            });
            let revision = view
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .buffer_revision(ctx);
            let generation = CLIAgentSessionsModel::as_ref(ctx)
                .input_generation(view.view_id)
                .unwrap();
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .unwrap();
            assert_eq!(session.agent, CLIAgent::Claude);
            assert!(
                session.remote_host.is_some(),
                "SessionStart 必须属于真实远端 session 元数据"
            );
            assert!(session.received_rich_notification);
            assert!(
                session.session_context.claude_image_evidence.is_some(),
                "原生 Claude 进程与历史候选缺失"
            );
            assert_eq!(
                session.session_context.cwd.as_deref(),
                Some("/fixture/project")
            );
            let listener = session
                .listener
                .as_ref()
                .expect("SessionStart 未注册监听器");
            let target = view
                .cli_agent_hook_input_target
                .as_ref()
                .expect("SessionStart 未绑定输入目标");
            assert_eq!(target.listener_id, listener.id());
            assert_eq!(target.model_events_id, view.model_events_handle.id());
            assert_eq!(
                Some(&target.native_session_id),
                session.session_context.session_id.as_ref()
            );
            {
                let model = view.model.lock();
                let block = model.block_list().active_block();
                assert_eq!(block.session_id(), Some(remote_session()));
                assert_eq!(block.id(), &target.block_id);
                assert!(block.is_active_and_long_running());
            }
            let (current_client, binding) = view.remote_image_binding(generation, ctx).unwrap();
            assert!(Arc::ptr_eq(&client, &current_client));
            let submission =
                RemoteImageSubmission::begin(view.view_id, current_client, binding.clone())
                    .unwrap();
            assert!(
                submission
                    .client
                    .cli_image_scope_is_current(&submission.scope)
            );
            let id = Uuid::parse_str(&submission.scope.submission_id).unwrap();
            if claimed {
                assert!(submission.claim_native_write([7; 32]));
            }
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                assert!(sessions.begin_input_submission(view.view_id, generation));
                assert!(sessions.register_remote_image_submission_owner(
                    view.view_id,
                    generation,
                    id,
                    Some(revision),
                ));
                // false 场景故意登记本地凭证，证明生产谓词仍独立要求真实领取位。
                sessions.register_remote_image_consumption(view.view_id, generation, id);
            });
            (submission, binding)
        });
        Self {
            terminal,
            submission,
            binding,
            transport,
            done,
        }
    }

    fn current(&self, app: &App) -> bool {
        self.terminal.read(app, |view, ctx| {
            view.remote_image_consumption_target_is_current(&self.submission, &self.binding, ctx)
        })
    }

    async fn close(self, app: &mut App) {
        self.terminal.update(app, |view, ctx| {
            cli_image_submission::revoke(view.view_id);
            RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.deregister_session(remote_session(), ctx);
            });
        });
        bounded(self.done.recv()).await.unwrap();
    }
}

#[test]
fn remote_image_consumption_predicate_requires_native_claim() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, false).await;
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn remote_image_consumption_predicate_accepts_only_automatic_generation_migration() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, true).await;
        assert!(fixture.current(&app));
        let changed = next_visibility(&mut app, &fixture.terminal);
        fixture.terminal.update(&mut app, |view, ctx| {
            view.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::AutoToggle, ctx);
            assert_ne!(
                CLIAgentSessionsModel::as_ref(ctx).input_generation(view.view_id),
                Some(fixture.binding.generation)
            );
            assert!(!view.remote_image_target_is_current(
                &fixture.submission,
                &fixture.binding,
                ctx
            ));
        });
        bounded(changed).await.unwrap();
        assert!(fixture.current(&app));
        let changed = next_visibility(&mut app, &fixture.terminal);
        fixture.terminal.update(&mut app, |view, ctx| {
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        });
        bounded(changed).await.unwrap();
        assert!(fixture.current(&app));
        let changed = next_visibility(&mut app, &fixture.terminal);
        fixture.terminal.update(&mut app, |view, ctx| {
            view.close_cli_agent_rich_input_and_disable_auto_toggle(ctx);
        });
        bounded(changed).await.unwrap();
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn remote_image_consumption_predicate_rejects_bootstrap_epoch_rotation() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, true).await;
        assert!(fixture.current(&app));
        RemoteServerManager::handle(&app).update(&mut app, |manager, _| {
            manager.notify_session_bootstrapped(remote_session(), "bash", Some("/bin/bash"));
        });
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn remote_image_consumption_predicate_rejects_disconnected_or_replaced_client() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, true).await;
        assert!(fixture.current(&app));
        RemoteServerManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.deregister_session(remote_session(), ctx)
        });
        bounded(fixture.done.recv()).await.unwrap();
        assert!(!fixture.current(&app));
        // 新连接建立事件不能触发旧终端的账本恢复；恢复绑定后单独核验 Arc 身份。
        fixture.terminal.update(&mut app, |view, ctx| {
            change_terminal_session(view, SessionId::from(7302), ctx);
        });
        let replacement = connect(&mut app, fixture.transport.clone()).await;
        RemoteServerManager::handle(&app).update(&mut app, |manager, _| {
            manager.notify_session_bootstrapped(remote_session(), "bash", Some("/bin/bash"))
        });
        assert!(!Arc::ptr_eq(&replacement, &fixture.submission.client));
        fixture.terminal.update(&mut app, |view, ctx| {
            change_terminal_session(view, remote_session(), ctx);
        });
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn remote_image_consumption_predicate_rejects_replaced_terminal_block() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, true).await;
        assert!(fixture.current(&app));
        fixture.terminal.update(&mut app, |view, _| {
            let mut model = view.model.lock();
            model.finish_block();
            model.simulate_long_running_block("claude", "");
            model
                .block_list_mut()
                .active_block_mut()
                .set_session_id(remote_session());
        });
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn remote_image_consumption_predicate_rejects_replaced_model_listener() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, true).await;
        assert!(fixture.current(&app));
        fixture.terminal.update(&mut app, |view, ctx| {
            let (_, receiver) = async_channel::unbounded();
            let sessions = view.sessions.clone();
            view.model_events_handle =
                ctx.add_model(|ctx| ModelEventDispatcher::new(receiver, sessions, ctx));
        });
        assert!(!fixture.current(&app));
        fixture.close(&mut app).await;
    });
}

#[test]
fn remote_image_consumption_predicate_rejects_replaced_cli_listener() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, true).await;
        assert!(fixture.current(&app));
        fixture.terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.register_listener(
                    view.view_id,
                    CLIAgent::Claude,
                    None,
                    None,
                    None,
                    None,
                    Some("fixture@host".to_owned()),
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
fn remote_image_consumption_predicate_rejects_replaced_native_session() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, true).await;
        assert!(fixture.current(&app));
        // 合并事件发布和清理；先在活连接上断言，再让通知订阅看到连接已移除，禁止触发真实账本恢复。
        app.update(|ctx| {
            fixture.terminal.update(ctx, |view, ctx| {
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.remove_session(view.view_id, ctx)
                });
                view.handle_cli_agent_notification(
                    Some("warp://cli-agent"),
                    &NOTIFICATION.replace(
                        "10000000-0000-4000-8000-000000000001",
                        "10000000-0000-4000-8000-000000000002",
                    ),
                    ctx,
                );
                assert!(
                    fixture
                        .submission
                        .client
                        .cli_image_scope_is_current(&fixture.submission.scope)
                );
                assert!(!view.remote_image_consumption_target_is_current(
                    &fixture.submission,
                    &fixture.binding,
                    ctx
                ));
                cli_image_submission::revoke(view.view_id);
            });
            RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.deregister_session(remote_session(), ctx);
            });
        });
        bounded(fixture.done.recv()).await.unwrap();
    });
}

#[test]
fn remote_image_consumption_predicate_rejects_changed_native_consumer_notification() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, true).await;
        assert!(fixture.current(&app));
        // 实际通知更新在断连前接受判定；退出外层 update 才分发恢复订阅，此时 manager 已无连接。
        app.update(|ctx| {
            fixture.terminal.update(ctx, |view, ctx| {
                let mut event = crate::terminal::cli_agent_sessions::event::parse_event(
                    Some("warp://cli-agent"),
                    NOTIFICATION,
                )
                .unwrap();
                event.event =
                    crate::terminal::cli_agent_sessions::event::CLIAgentEventType::PromptSubmit;
                event
                    .payload
                    .claude_process_evidence
                    .as_mut()
                    .unwrap()
                    .process_id_candidate = 102;
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.update_from_event(view.view_id, &event, ctx)
                });
                assert!(
                    fixture
                        .submission
                        .client
                        .cli_image_scope_is_current(&fixture.submission.scope)
                );
                assert!(!view.remote_image_consumption_target_is_current(
                    &fixture.submission,
                    &fixture.binding,
                    ctx
                ));
                cli_image_submission::revoke(view.view_id);
            });
            RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.deregister_session(remote_session(), ctx);
            });
        });
        bounded(fixture.done.recv()).await.unwrap();
    });
}

#[test]
fn remote_image_consumption_predicate_rejects_another_live_client_with_identical_binding() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app, false).await;
        let executor = app.read(|ctx| ctx.background_executor().clone());
        let connection = bounded(fixture.transport.connect(executor)).await.unwrap();
        let other_client = Arc::new(connection.client);
        bounded(other_client.initialize(
            None,
            InitializeParams {
                user_id: String::new(),
                user_email: String::new(),
                crash_reporting_enabled: false,
                codebase_index_limits: None,
            },
        ))
        .await
        .unwrap();
        other_client.notify_session_bootstrapped(remote_session(), "bash", Some("/bin/bash"));
        let other = fixture.terminal.update(&mut app, |view, ctx| {
            let other =
                RemoteImageSubmission::begin(view.view_id, other_client, fixture.binding.clone())
                    .unwrap();
            assert!(other.claim_native_write([8; 32]));
            let old_id = Uuid::parse_str(&fixture.submission.scope.submission_id).unwrap();
            let id = Uuid::parse_str(&other.scope.submission_id).unwrap();
            let revision = view
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .buffer_revision(ctx);
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                assert!(sessions.finish_remote_image_submission(
                    view.view_id,
                    fixture.binding.generation,
                    old_id
                ));
                assert!(sessions.begin_input_submission(view.view_id, fixture.binding.generation));
                assert!(sessions.register_remote_image_submission_owner(
                    view.view_id,
                    fixture.binding.generation,
                    id,
                    Some(revision),
                ));
                sessions.register_remote_image_consumption(
                    view.view_id,
                    fixture.binding.generation,
                    id,
                );
            });
            // 两个连接都仍活着，scope 各自在本连接有效；只靠同主机/SID不能冒充当前 Arc。
            assert!(other.client.cli_image_scope_is_current(&other.scope));
            assert!(!fixture.submission.client.is_disconnected());
            assert!(!view.remote_image_consumption_target_is_current(
                &other,
                &fixture.binding,
                ctx
            ));
            other
        });
        drop(connection.resource);
        bounded(fixture.done.recv()).await.unwrap();
        other.revoke();
        fixture.close(&mut app).await;
    });
}
