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

use crate::ai::agent::ImageContext;
use crate::remote_server::auth::RemoteServerAuthContext;
use crate::remote_server::manager::{RemoteServerExitStatus, RemoteServerManagerEvent};
use crate::remote_server::proto::{
    InitializeResponse, RemoteServerCapability, ServerMessage, client_message, server_message,
    session_scoped_request,
};
use crate::remote_server::protocol;
use crate::remote_server::setup::{PreinstallCheckResult, RemotePlatform};
use crate::remote_server::transport::{
    Connection, ControlPath, Error, InstallOutcome, RemoteTransport, TransportConnection,
};
use crate::terminal::cli_agent_sessions::{CLIAgentInputEntrypoint, CLIAgentSessionsModelEvent};
use crate::terminal::event::BlockMetadataReceivedEvent;
use crate::terminal::model::ansi::Handler as _;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::{BootstrapSessionType, SessionInfo};
use crate::terminal::model_events::ModelEvent;
use crate::terminal::shell::ShellType;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

use super::*;

fn ticket() -> Ticket {
    Ticket {
        id: Uuid::new_v4(),
        key: Uuid::new_v4(),
    }
}

#[test]
fn same_ticket_from_old_ssh_client_cannot_finish_current_pending() {
    let ticket = ticket();
    let attempt = Uuid::new_v4();
    let old_client = Arc::new(1u8);
    let current_client = Arc::new(2u8);
    let generation = attempt.to_string();

    assert!(!same_remote_grok_attempt(
        &ticket,
        attempt,
        &old_client,
        &ticket,
        Some(attempt),
        Some((&current_client, generation.as_str())),
    ));
    assert!(same_remote_grok_attempt(
        &ticket,
        attempt,
        &current_client,
        &ticket,
        Some(attempt),
        Some((&current_client, generation.as_str())),
    ));
}

#[test]
fn old_attempt_cannot_finish_new_pending_on_same_client_and_ticket() {
    let ticket = ticket();
    let client = Arc::new(1u8);
    let old_attempt = Uuid::new_v4();
    let current_attempt = Uuid::new_v4();
    let generation = current_attempt.to_string();

    assert!(!same_remote_grok_attempt(
        &ticket,
        old_attempt,
        &client,
        &ticket,
        Some(current_attempt),
        Some((&client, generation.as_str())),
    ));
    assert!(same_remote_grok_attempt(
        &ticket,
        current_attempt,
        &client,
        &ticket,
        Some(current_attempt),
        Some((&client, generation.as_str())),
    ));
}

fn remote_session() -> SessionId {
    SessionId::from(7401)
}

const NOTIFICATION: &str = r#"{"v":1,"agent":"grok","event":"session_start","session_id":"20000000-0000-4000-8000-000000000001","cwd":"/fixture/project","event_id":"start-1","permission_mode":"default","plugin_version":"0.1.5"}"#;

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
                                            RemoteServerCapability::CliImageReferenceRecoveryV1.into(),
                                            RemoteServerCapability::CliGrokOwnedV1.into(),
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

struct Fixture {
    terminal: ViewHandle<TerminalView>,
    binding: Binding,
    scope: CliImageStagingScope,
    revision: u64,
    snapshot: Rc<CliInputSubmission>,
    message: Uuid,
    done: async_channel::Receiver<()>,
}

impl Fixture {
    async fn new(app: &mut App) -> Self {
        initialize_app_for_terminal_view(app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        crate::features::FeatureFlag::CLIAgentRichInput.set_enabled(true);
        let (done_sender, done) = async_channel::unbounded();
        let client = connect(app, MemoryTransport { done: done_sender }).await;
        let terminal = add_window_with_terminal(app, None);
        let changed = next_visibility(app, &terminal);
        terminal.update(app, |view, ctx| {
            let mut info = SessionInfo::new_for_test()
                .with_session_type(BootstrapSessionType::WarpifiedRemote)
                .with_shell_type(ShellType::Bash);
            info.session_id = remote_session();
            view.sessions
                .update(ctx, |sessions, _| sessions.register_session_for_test(info));
            let block_index = {
                let mut model = view.model.lock();
                model.simulate_long_running_block("grok", "");
                model
                    .block_list_mut()
                    .active_block_mut()
                    .set_session_id(remote_session());
                model
                    .block_list_mut()
                    .active_block_mut()
                    .set_current_working_directory("/fixture/project".into());
                model.block_list().active_block_index()
            };
            view.handle_terminal_event(
                &ModelEvent::BlockMetadataReceived(BlockMetadataReceivedEvent {
                    block_metadata: BlockMetadata::new(
                        Some(remote_session()),
                        Some("/fixture/project".into()),
                    ),
                    block_index,
                    is_after_in_band_command: false,
                    is_done_bootstrapping: true,
                }),
                ctx,
            );
            // 在真实 SessionStart 事件前提供已绑定 owned 记录，禁止触发磁盘恢复或 Status RPC。
            view.bind_remote_owned_grok_for_test(
                ticket(),
                Uuid::parse_str("20000000-0000-4000-8000-000000000001").unwrap(),
                ctx,
            );
            view.handle_cli_agent_notification(Some("warp://cli-agent"), NOTIFICATION, ctx);
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        });
        bounded(changed).await.unwrap();
        let (binding, scope, revision, snapshot) = terminal.update(app, |view, ctx| {
            RemoteServerManager::handle(ctx).update(ctx, |manager, _| {
                manager.notify_session_bootstrapped(remote_session(), "bash", Some("/bin/bash"));
            });
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("原图片草稿", ctx)
            });
            view.ai_context_model.update(ctx, |model, ctx| {
                model.append_pending_images(vec![image("original.png")], ctx);
            });
            let generation = CLIAgentSessionsModel::as_ref(ctx)
                .input_generation(view.view_id)
                .unwrap();
            let (current, binding) = view.remote_grok_input_binding(generation, ctx).unwrap();
            assert!(Arc::ptr_eq(&client, &current));
            let (scope, revision) =
                remote::scope(&client, &binding.launch, binding.attempt).unwrap();
            let snapshot = Rc::new(CliInputSubmission {
                agent: CLIAgent::Grok,
                query: "原图片草稿".into(),
                editor_revision: view
                    .input
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .buffer_revision(ctx),
                attachments_revision: view
                    .ai_context_model
                    .as_ref(ctx)
                    .pending_attachments_revision(),
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                assert!(sessions.begin_input_submission(view.view_id, generation));
                assert!(sessions.register_remote_image_submission_owner(
                    view.view_id,
                    generation,
                    binding.attempt,
                    Some(snapshot.editor_revision.clone())
                ));
                sessions.register_remote_image_consumption(
                    view.view_id,
                    generation,
                    binding.attempt,
                );
            });
            let owned = view.grok_remote_owned.as_mut().unwrap();
            owned.sending = true;
            owned.sending_generation = Some(binding.attempt);
            owned.pending = Some((client, scope.clone(), revision));
            assert!(view.remote_grok_binding_matches(&binding, ctx));
            (binding, scope, revision, snapshot)
        });
        Self {
            terminal,
            binding,
            scope,
            revision,
            snapshot,
            message: Uuid::new_v4(),
            done,
        }
    }

    async fn hide(&self, app: &mut App, reason: CLIAgentRichInputCloseReason) {
        let changed = next_visibility(app, &self.terminal);
        self.terminal.update(app, |view, ctx| {
            view.close_cli_agent_rich_input(reason, ctx)
        });
        bounded(changed).await.unwrap();
    }

    async fn reopen(&self, app: &mut App) {
        let changed = next_visibility(app, &self.terminal);
        self.terminal.update(app, |view, ctx| {
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx)
        });
        bounded(changed).await.unwrap();
    }

    fn acknowledge(&self, app: &mut App) {
        self.acknowledge_state(app, "finished");
    }

    fn acknowledge_state(&self, app: &mut App, state: &str) {
        self.terminal.update(app, |view, ctx| {
            view.receive_remote_grok_reply(
                self.binding.client.clone(),
                self.scope.clone(),
                self.revision,
                self.binding.clone(),
                self.snapshot.clone(),
                self.message,
                "subject".into(),
                Reply::Input {
                    ticket: self.binding.launch.ticket.clone(),
                    message_id: self.message,
                    subject_sha256: "subject".into(),
                    state: state.into(),
                    native_prompt_id: (state != "rejected_before_enqueue").then(Uuid::new_v4),
                    native_ack_sha256: Some("a".repeat(64)),
                },
                ctx,
            );
        });
    }

    fn assert_draft(&self, app: &App, text: &str, images: usize) {
        self.terminal.read(app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), text);
            assert_eq!(
                view.ai_context_model.as_ref(ctx).pending_images().len(),
                images
            );
        });
    }

    async fn close(self, app: &mut App) {
        self.terminal.update(app, |view, ctx| {
            // 保留远端撤销通知路径，但结束前移除本地 owned，避免终端析构走磁盘恢复。
            view.grok_remote_owned.take();
            RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.deregister_session(remote_session(), ctx)
            });
        });
        bounded(self.done.recv()).await.unwrap();
    }
}

fn image(name: &str) -> ImageContext {
    ImageContext {
        data: "aGVsbG8=".into(),
        mime_type: "image/png".into(),
        file_name: name.into(),
        is_figma: false,
    }
}

#[test]
fn finished_grok_receipt_clears_draft_after_real_auto_hide_and_restore() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture
            .hide(&mut app, CLIAgentRichInputCloseReason::AutoToggle)
            .await;
        fixture.reopen(&mut app).await;
        fixture.terminal.read(&app, |view, ctx| {
            assert!(!view.remote_grok_binding_matches(&fixture.binding, ctx));
            assert!(view.remote_grok_consumption_matches(&fixture.binding, ctx));
            assert!(
                !CLIAgentSessionsModel::as_ref(ctx)
                    .is_input_submission_current(view.view_id, fixture.binding.generation)
            );
        });
        fixture.acknowledge(&mut app);
        fixture.assert_draft(&app, "", 0);
        fixture.close(&mut app).await;
    });
}

#[test]
fn native_pre_enqueue_rejection_preserves_grok_draft_and_releases_only_current_lease() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.acknowledge_state(&mut app, "rejected_before_enqueue");
        fixture.assert_draft(&app, "原图片草稿", 1);
        fixture.terminal.read(&app, |view, ctx| {
            assert!(!view.grok_remote_owned.as_ref().unwrap().sending);
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            assert!(
                !sessions.is_input_submission_current(view.view_id, fixture.binding.generation)
            );
            assert!(
                sessions
                    .remote_image_consumption_revision(view.view_id, fixture.binding.attempt)
                    .is_none()
            );
        });
        fixture.close(&mut app).await;
    });
}

#[test]
fn finished_grok_receipt_clears_hidden_draft_before_restore() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture
            .hide(&mut app, CLIAgentRichInputCloseReason::AutoToggle)
            .await;
        fixture.terminal.read(&app, |view, ctx| {
            assert!(!view.remote_grok_binding_matches(&fixture.binding, ctx));
            assert!(view.remote_grok_consumption_matches(&fixture.binding, ctx));
        });
        fixture.acknowledge(&mut app);
        fixture.reopen(&mut app).await;
        fixture.assert_draft(&app, "", 0);
        fixture.close(&mut app).await;
    });
}

#[test]
fn finished_grok_receipt_preserves_edited_draft_and_image() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("新图片草稿", ctx)
            })
        });
        fixture.acknowledge(&mut app);
        fixture.assert_draft(&app, "新图片草稿", 1);
        fixture.close(&mut app).await;
    });
}

#[test]
fn finished_grok_receipt_preserves_manually_closed_draft() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture
            .hide(&mut app, CLIAgentRichInputCloseReason::Other)
            .await;
        fixture.reopen(&mut app).await;
        fixture.acknowledge(&mut app);
        fixture.assert_draft(&app, "原图片草稿", 1);
        fixture.close(&mut app).await;
    });
}

#[test]
fn finished_grok_receipt_preserves_draft_when_attachments_change() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.terminal.update(&mut app, |view, ctx| {
            view.ai_context_model.update(ctx, |model, ctx| {
                model.append_pending_images(vec![image("new.png")], ctx)
            })
        });
        fixture.acknowledge(&mut app);
        fixture.assert_draft(&app, "原图片草稿", 2);
        fixture.close(&mut app).await;
    });
}

#[test]
fn finished_grok_receipt_preserves_draft_after_native_session_changes() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.terminal.update(&mut app, |view, _| {
            view.grok_remote_owned.as_mut().unwrap().native_session = Some(Uuid::new_v4())
        });
        fixture.acknowledge(&mut app);
        fixture.assert_draft(&app, "原图片草稿", 1);
        fixture.close(&mut app).await;
    });
}

#[test]
fn finished_grok_receipt_preserves_draft_after_editing_back_to_original_text() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("临时编辑", ctx);
                input.replace_buffer_content("原图片草稿", ctx);
            })
        });
        fixture.acknowledge(&mut app);
        fixture.assert_draft(&app, "原图片草稿", 1);
        fixture.close(&mut app).await;
    });
}

#[test]
fn finished_grok_receipt_preserves_draft_after_bootstrap_epoch_changes() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        RemoteServerManager::handle(&app).update(&mut app, |manager, _| {
            manager.notify_session_bootstrapped(remote_session(), "bash", Some("/bin/bash"));
        });
        fixture.terminal.read(&app, |view, ctx| {
            assert!(
                !fixture
                    .binding
                    .client
                    .cli_image_scope_is_current(&fixture.scope)
            );
            assert!(!view.remote_grok_consumption_matches(&fixture.binding, ctx));
        });
        fixture.acknowledge(&mut app);
        fixture.assert_draft(&app, "原图片草稿", 1);
        fixture.close(&mut app).await;
    });
}

#[test]
fn cancelled_grok_receipt_preserves_draft_and_releases_original_lease() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.acknowledge_state(&mut app, "cancelled");
        fixture.assert_draft(&app, "原图片草稿", 1);
        fixture.terminal.read(&app, |view, ctx| {
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            assert!(
                !sessions.is_input_submission_current(view.view_id, fixture.binding.generation)
            );
            assert!(
                sessions
                    .remote_image_consumption_revision(view.view_id, fixture.binding.attempt)
                    .is_none()
            );
            assert!(!view.grok_remote_owned.as_ref().unwrap().sending);
        });
        fixture.close(&mut app).await;
    });
}

#[test]
fn finished_grok_receipt_without_original_draft_receipt_does_not_clear() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        fixture.terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                sessions.finish_remote_image_consumption(view.view_id, fixture.binding.attempt);
            });
            assert!(view.remote_grok_binding_matches(&fixture.binding, ctx));
            assert!(!view.remote_grok_consumption_matches(&fixture.binding, ctx));
        });
        fixture.acknowledge(&mut app);
        fixture.assert_draft(&app, "原图片草稿", 1);
        fixture.close(&mut app).await;
    });
}

#[test]
fn old_grok_ack_and_failure_do_not_release_new_attempt_in_same_generation() {
    App::test((), |mut app| async move {
        let fixture = Fixture::new(&mut app).await;
        let current = fixture.terminal.update(&mut app, |view, ctx| {
            let generation = fixture.binding.generation;
            let (client, binding) = view.remote_grok_input_binding(generation, ctx).unwrap();
            let (scope, revision) =
                remote::scope(&client, &binding.launch, binding.attempt).unwrap();
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                sessions.finish_input_submission(view.view_id, generation);
                assert!(sessions.begin_input_submission(view.view_id, generation));
                assert!(sessions.register_remote_image_submission_owner(
                    view.view_id,
                    generation,
                    binding.attempt,
                    Some(fixture.snapshot.editor_revision.clone()),
                ));
                sessions.register_remote_image_consumption(
                    view.view_id,
                    generation,
                    binding.attempt,
                );
            });
            let owned = view.grok_remote_owned.as_mut().unwrap();
            owned.sending_generation = Some(binding.attempt);
            owned.pending = Some((client, scope, revision));
            assert!(view.remote_grok_binding_matches(&binding, ctx));
            binding
        });
        fixture.acknowledge(&mut app);
        fixture.terminal.update(&mut app, |view, ctx| {
            view.fail_remote_grok_worker(&fixture.binding, ctx);
            assert!(view.remote_grok_binding_matches(&current, ctx));
            assert!(view.remote_grok_consumption_matches(&current, ctx));
            assert!(view.grok_remote_owned.as_ref().unwrap().sending);
            assert!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .is_input_submission_current(view.view_id, current.generation)
            );
        });
        fixture.assert_draft(&app, "原图片草稿", 1);
        fixture.close(&mut app).await;
    });
}
