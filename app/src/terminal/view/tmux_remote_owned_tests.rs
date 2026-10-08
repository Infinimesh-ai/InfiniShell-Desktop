use super::*;
use crate::remote_server::proto::{
    TerminalBindingFailed, terminal_binding_request, terminal_binding_response,
};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSessionStatus, CLIAgentSessionsModelEvent,
};
use crate::terminal::model::ansi::Processor;
use crate::terminal::model_events::ModelEvent;
use crate::terminal::view::terminal_binding::tests::{Fixture, Wire, bounded, state_changed};
use crate::workspace::ToastStackEvent;
use futures::channel::oneshot;
use std::cell::RefCell;
use std::rc::Rc;
use warp_core::SessionId;
use warpui::App;

fn toast_messages(app: &mut App) -> Rc<RefCell<Vec<String>>> {
    let messages = Rc::new(RefCell::new(Vec::new()));
    let observed = messages.clone();
    app.update(|ctx| {
        ctx.subscribe_to_model(&ToastStack::handle(ctx), move |_, event, _| {
            if let ToastStackEvent::AddEphemeralToast { toast, .. } = event {
                observed.borrow_mut().push(toast.main_text().to_owned());
            }
        });
    });
    messages
}

fn seed_restore(fixture: &Fixture, app: &mut App) -> Uuid {
    fixture.terminal.update(app, |view, ctx| {
        let (snapshot, client) = view.terminal_binding_snapshot(ctx).unwrap();
        let scope = client.terminal_binding_scope(snapshot.session).unwrap();
        let launch = Launch {
            version: 1,
            host: scope.host_id.clone(),
            terminal_session: snapshot.session.as_u64(),
            block_id: snapshot.block.to_string(),
            agent: TerminalBindingOwnedAgent::Claude as i32,
            id: Uuid::new_v4(),
            key: Uuid::new_v4(),
        };
        let id = launch.id;
        view.tmux_remote_owned = Some(TmuxOwned {
            operation: Uuid::new_v4(),
            snapshot,
            client,
            scope,
            launch: Some(launch),
            agent: CLIAgent::Claude,
            attempt: None,
            pane_cwd: None,
            phase: Phase::Preparing,
            restore: true,
            explicit_restore: true,
            launched: false,
            native_session: None,
            starts: Vec::new(),
            observed_identity: None,
            restored_identity: None,
        });
        view.begin_tmux_owned_binding(ctx);
        id
    })
}

async fn bound_status(fixture: &Fixture, app: &mut App) -> Wire {
    let begin = bounded(fixture.requests.recv()).await.unwrap();
    let attempt = Uuid::parse_str(&begin.request.attempt_id).unwrap();
    assert!(matches!(
        begin.request.action,
        Some(terminal_binding_request::Action::Begin(_))
    ));
    let nonce = Uuid::new_v4();
    fixture.osc(app, attempt, nonce);
    begin.written();
    let ack = bounded(fixture.requests.recv()).await.unwrap();
    assert!(
        matches!(&ack.request.action, Some(terminal_binding_request::Action::Ack(value)) if value.nonce == nonce.to_string())
    );
    ack.bound();
    let status = bounded(fixture.requests.recv()).await.unwrap();
    assert!(matches!(
        status.request.action,
        Some(terminal_binding_request::Action::StatusOwned(_))
    ));
    status
}

fn send_start(fixture: &Fixture, app: &mut App, native: Uuid) {
    let body = serde_json::json!({"v":1,"agent":"claude","event":"session_start", "session_id":native.to_string(),
        "cwd":"/fixture/project","plugin_version":"0.5.0",
        "transcript_path":"/fixture/claude/history.jsonl",
        "claude_process_evidence":{"process_id_candidate":101,"claude_config_directory":"/fixture/claude","tty_path":"/dev/pts/7"}}).to_string();
    fixture.terminal.update(app, |view, _| {
        let mut model = view.model.lock();
        let sequence = format!("\x1b]777;notify;warp://cli-agent;{body}\x07");
        Processor::new().parse_bytes(&mut *model, sequence.as_bytes(), &mut std::io::sink());
    });
}

fn respond_status(wire: Wire, id: Uuid, native: Uuid) {
    wire.respond(terminal_binding_response::Result::Owned(
        TerminalBindingOwned {
            launch_id: id.to_string(),
            agent: TerminalBindingOwnedAgent::Claude as i32,
            phase: "running".into(),
            owned_reply_json: vec![],
            native_session_id: Some(native.to_string()),
        },
    ));
}

#[test]
fn tmux_restore_waits_for_real_challenge_and_only_replays_matching_real_hook() {
    App::test((), |mut app| async move {
        let fixture = Fixture::with_owned(&mut app, true).await;
        crate::features::FeatureFlag::CLIAgentRichInput.set_enabled(true);
        crate::features::FeatureFlag::PluggableNotifications.set_enabled(true);
        let id = seed_restore(&fixture, &mut app);
        let status = bound_status(&fixture, &mut app).await;
        let native = Uuid::new_v4();
        let sibling = Uuid::new_v4();
        let (observed, notifications) = oneshot::channel();
        let mut observed = Some(observed);
        let mut seen = Vec::new();
        let events = fixture
            .terminal
            .read(&app, |view, _| view.model_events_handle.clone());
        app.update(|ctx| {
            ctx.subscribe_to_model(&events, move |_, event, _| {
                if let ModelEvent::PluggableNotification { title, body } = event {
                    let event = super::super::parse_event(title.as_deref(), body).unwrap();
                    let id = Uuid::parse_str(event.session_id.as_deref().unwrap()).unwrap();
                    assert!(id == sibling || id == native);
                    assert!(!seen.contains(&id));
                    seen.push(id);
                    if seen.len() == 2
                        && let Some(observed) = observed.take()
                    {
                        let _ = observed.send(());
                    }
                }
            });
        });
        send_start(&fixture, &mut app, sibling);
        send_start(&fixture, &mut app, native);
        // 隐藏缓存不需要重绘；以实际模型通知完成为屏障，再检查 view 已缓存两条原始事件。
        bounded(notifications).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            assert_eq!(view.tmux_remote_owned.as_ref().unwrap().starts.len(), 2);
            assert!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .is_none()
            )
        });
        let ready = state_changed(&mut app, &fixture.terminal, move |view| {
            view.tmux_remote_owned.as_ref().is_some_and(|owned| {
                owned.native_session == Some(native) && owned.phase == Phase::Active
            })
        });
        respond_status(status, id, native);
        bounded(ready).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .unwrap();
            assert_eq!(
                session.session_context.session_id.as_deref(),
                Some(native.to_string().as_str())
            );
            assert!(session.received_rich_notification);
            assert!(
                view.tmux_owned_image_reference(CLIAgent::Claude, &native.to_string(), ctx)
                    .unwrap()
                    .is_some()
            );
            assert!(
                view.tmux_owned_image_reference(CLIAgent::Claude, &sibling.to_string(), ctx)
                    .is_err()
            );
        });
        assert!(fixture.requests.try_recv().is_err());
        fixture.close(&mut app).await;
    });
}

#[test]
fn tmux_failed_begin_never_requests_owned_start_or_status() {
    App::test((), |mut app| async move {
        let fixture = Fixture::with_owned(&mut app, true).await;
        seed_restore(&fixture, &mut app);
        let wire = bounded(fixture.requests.recv()).await.unwrap();
        let failed = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.phase == Phase::Failed)
        });
        wire.respond(terminal_binding_response::Result::Failed(
            TerminalBindingFailed {
                code: "unavailable".into(),
            },
        ));
        bounded(failed).await.unwrap();
        assert!(fixture.requests.try_recv().is_err());
        fixture.close(&mut app).await;
    });
}

#[test]
fn tmux_old_status_callback_cannot_bind_replacement_intent() {
    App::test((), |mut app| async move {
        let fixture = Fixture::with_owned(&mut app, true).await;
        let old_id = seed_restore(&fixture, &mut app);
        let old_status = bound_status(&fixture, &mut app).await;
        let launch = fixture.terminal.read(&app, |view, _| {
            view.tmux_remote_owned
                .as_ref()
                .unwrap()
                .launch
                .clone()
                .unwrap()
        });
        seed_restore(&fixture, &mut app);
        fixture.terminal.update(&mut app, |view, _| {
            view.tmux_remote_owned.as_mut().unwrap().launch = Some(launch);
        });
        let new_id = old_id;
        let old_native = Uuid::new_v4();
        respond_status(old_status, old_id, old_native);
        let new_status = bound_status(&fixture, &mut app).await;
        fixture.terminal.read(&app, |view, _| {
            let owned = view.tmux_remote_owned.as_ref().unwrap();
            assert_eq!(owned.launch.as_ref().unwrap().id, new_id);
            assert!(owned.native_session.is_none());
        });
        let new_native = Uuid::new_v4();
        let ready = state_changed(&mut app, &fixture.terminal, move |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.native_session == Some(new_native))
        });
        respond_status(new_status, new_id, new_native);
        bounded(ready).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            assert!(
                view.tmux_owned_image_reference(CLIAgent::Claude, &old_native.to_string(), ctx)
                    .is_err()
            )
        });
        fixture.close(&mut app).await;
    });
}

#[test]
fn tmux_same_intent_restore_retires_the_original_instance() {
    App::test((), |mut app| async move {
        let fixture = Fixture::with_owned(&mut app, true).await;
        let id = seed_restore(&fixture, &mut app);
        let status = bound_status(&fixture, &mut app).await;
        let native = Uuid::new_v4();
        let ready = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.phase == Phase::Active)
        });
        respond_status(status, id, native);
        bounded(ready).await.unwrap();
        let (original, launch) = fixture.terminal.read(&app, |view, ctx| {
            (
                view.tmux_owned_instance(id, ctx).unwrap(),
                view.tmux_remote_owned
                    .as_ref()
                    .unwrap()
                    .launch
                    .clone()
                    .unwrap(),
            )
        });
        seed_restore(&fixture, &mut app);
        fixture.terminal.update(&mut app, |view, _| {
            view.tmux_remote_owned.as_mut().unwrap().launch = Some(launch);
        });
        let status = bound_status(&fixture, &mut app).await;
        assert!(matches!(&status.request.action,
            Some(terminal_binding_request::Action::StatusOwned(request)) if request.launch_id == id.to_string()));
        let ready = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.phase == Phase::Active)
        });
        respond_status(status, id, native);
        bounded(ready).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            assert!(!view.tmux_owned_instance_is_current(&original, ctx));
            let current = view.tmux_owned_instance(id, ctx).unwrap();
            assert!(view.tmux_owned_instance_is_current(&current, ctx));
            assert_ne!(original.operation, current.operation);
            assert_ne!(original.attempt, current.attempt);
            assert_eq!(original.launch, current.launch);
            assert!(Arc::ptr_eq(&original.client, &current.client));
        });
        fixture.close(&mut app).await;
    });
}

#[test]
fn tmux_exited_status_retires_binding_without_starting_another_pane() {
    App::test((), |mut app| async move {
        let fixture = Fixture::with_owned(&mut app, true).await;
        let messages = toast_messages(&mut app);
        let id = seed_restore(&fixture, &mut app);
        let status = bound_status(&fixture, &mut app).await;
        let ended = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.phase == Phase::Exited)
        });
        status.respond(terminal_binding_response::Result::Owned(
            TerminalBindingOwned {
                launch_id: id.to_string(),
                agent: TerminalBindingOwnedAgent::Claude as i32,
                phase: "exited".into(),
                owned_reply_json: Vec::new(),
                native_session_id: None,
            },
        ));
        bounded(ended).await.unwrap();
        fixture.terminal.update(&mut app, |view, ctx| {
            view.invalidate_remote_tmux_owned(ctx);
            let owned = view.tmux_remote_owned.as_ref().unwrap();
            assert!(owned.phase == Phase::Exited);
            assert_eq!(owned.launch.as_ref().unwrap().id, id);
            assert!(!owned.launched);
            assert!(owned.native_session.is_none());
            assert!(owned.starts.is_empty());
            assert!(view.current_remote_terminal_binding(ctx).is_none());
            assert!(view.tmux_owned_instance(id, ctx).is_none());
        });
        assert_eq!(
            &*messages.borrow(),
            &[crate::t!("cli-agent-tmux-owned-exited")]
        );
        assert!(fixture.requests.try_recv().is_err());
        fixture.close(&mut app).await;
    });
}

#[test]
fn tmux_unknown_status_keeps_the_original_intent_pending() {
    App::test((), |mut app| async move {
        let fixture = Fixture::with_owned(&mut app, true).await;
        let id = seed_restore(&fixture, &mut app);
        let status = bound_status(&fixture, &mut app).await;
        status.respond(terminal_binding_response::Result::Owned(
            TerminalBindingOwned {
                launch_id: id.to_string(),
                agent: TerminalBindingOwnedAgent::Claude as i32,
                phase: "unknown".into(),
                owned_reply_json: Vec::new(),
                native_session_id: None,
            },
        ));
        let retry = bounded(fixture.requests.recv()).await.unwrap();
        assert!(matches!(&retry.request.action,
            Some(terminal_binding_request::Action::StatusOwned(request)) if request.launch_id == id.to_string()));
        fixture.terminal.read(&app, |view, ctx| {
            let owned = view.tmux_remote_owned.as_ref().unwrap();
            assert!(owned.phase == Phase::Requesting);
            assert_eq!(owned.launch.as_ref().unwrap().id, id);
            assert!(view.current_remote_terminal_binding(ctx).is_some());
        });
        let failed = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.phase == Phase::Failed)
        });
        retry.respond(terminal_binding_response::Result::Owned(
            TerminalBindingOwned {
                launch_id: id.to_string(),
                agent: TerminalBindingOwnedAgent::Claude as i32,
                phase: "failed".into(),
                owned_reply_json: Vec::new(),
                native_session_id: None,
            },
        ));
        bounded(failed).await.unwrap();
        fixture.close(&mut app).await;
    });
}

#[test]
fn tmux_automatic_exit_does_not_show_an_explicit_restore_notice() {
    App::test((), |mut app| async move {
        let fixture = Fixture::with_owned(&mut app, true).await;
        let messages = toast_messages(&mut app);
        let id = seed_restore(&fixture, &mut app);
        fixture.terminal.update(&mut app, |view, _| {
            view.tmux_remote_owned.as_mut().unwrap().explicit_restore = false;
        });
        let status = bound_status(&fixture, &mut app).await;
        let ended = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.phase == Phase::Exited)
        });
        status.respond(terminal_binding_response::Result::Owned(
            TerminalBindingOwned {
                launch_id: id.to_string(),
                agent: TerminalBindingOwnedAgent::Claude as i32,
                phase: "exited".into(),
                owned_reply_json: Vec::new(),
                native_session_id: None,
            },
        ));
        bounded(ended).await.unwrap();
        assert!(messages.borrow().is_empty());
        assert!(fixture.requests.try_recv().is_err());
        fixture.close(&mut app).await;
    });
}

async fn complete_original_block(fixture: &Fixture, app: &mut App) {
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    let view_id = fixture.terminal.read(app, |view, _| view.view_id);
    app.update(|ctx| {
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), move |_, event, _| {
            if matches!(event, CLIAgentSessionsModelEvent::Ended {terminal_view_id, ..} if *terminal_view_id == view_id)
                && let Some(sender) = sender.take()
            { let _ = sender.send(()); }
        });
    });
    fixture
        .terminal
        .update(app, |view, _| view.model.lock().finish_block());
    bounded(receiver).await.unwrap();
    fixture.terminal.read(app, |view, ctx| {
        let sessions = CLIAgentSessionsModel::as_ref(ctx);
        assert!(sessions.session(view.view_id).is_none());
        assert!(sessions.input_generation(view.view_id).is_none());
        assert!(view.cli_agent_hook_input_target.is_none());
    });
}

// 只跳过已有 Journal 的磁盘读取；内存恢复准备、线路挑战及 Status 均走生产函数。
fn prepare_retained_restore(fixture: &Fixture, app: &mut App) -> Uuid {
    fixture.terminal.update(app, |view, ctx| {
        let (_, _, retained) = view.prepare_tmux_owned_restore(ctx).unwrap();
        let launch = retained.unwrap();
        let id = launch.id;
        let owned = view.tmux_remote_owned.as_mut().unwrap();
        owned.agent = cli_agent(launch.agent).unwrap();
        owned.launch = Some(launch);
        view.begin_tmux_owned_binding(ctx);
        id
    })
}

#[test]
fn tmux_claude_restore_after_completed_block_and_new_ssh_needs_no_second_hook() {
    App::test((), |mut app| async move {
        let mut fixture = Fixture::with_owned(&mut app, true).await;
        crate::features::FeatureFlag::CLIAgentRichInput.set_enabled(true);
        crate::features::FeatureFlag::PluggableNotifications.set_enabled(true);
        let id = seed_restore(&fixture, &mut app);
        let status = bound_status(&fixture, &mut app).await;
        let native = Uuid::new_v4();
        let active = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.phase == Phase::Active)
        });
        respond_status(status, id, native);
        bounded(active).await.unwrap();
        let observed = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.observed_identity.is_some())
        });
        send_start(&fixture, &mut app, native);
        bounded(observed).await.unwrap();
        let (old_listener, old_generation, old_instance) =
            fixture.terminal.read(&app, |view, ctx| {
                let sessions = CLIAgentSessionsModel::as_ref(ctx);
                let session = sessions.session(view.view_id).unwrap();
                assert!(session.received_rich_notification);
                (
                    session.listener.as_ref().unwrap().id(),
                    sessions.input_generation(view.view_id).unwrap(),
                    view.tmux_owned_instance(id, ctx).unwrap(),
                )
            });
        complete_original_block(&fixture, &mut app).await;
        fixture.reconnect_as(&mut app, SessionId::from(7502)).await;
        assert_eq!(prepare_retained_restore(&fixture, &mut app), id);
        let status = bound_status(&fixture, &mut app).await;
        let restored = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.restored_identity.is_some())
        });
        respond_status(status, id, native);
        bounded(restored).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            let session = sessions.session(view.view_id).unwrap();
            assert_eq!(session.status, CLIAgentSessionStatus::Unknown);
            assert_eq!(session.input_state, CLIAgentInputState::Closed);
            assert!(!session.should_auto_toggle_input);
            assert!(!session.received_rich_notification);
            assert_ne!(session.listener.as_ref().unwrap().id(), old_listener);
            assert_ne!(
                sessions.input_generation(view.view_id),
                Some(old_generation)
            );
            assert!(!sessions.is_input_submission_current(view.view_id, old_generation));
            assert!(session.session_context.claude_image_evidence.is_none());
            assert!(session.session_context.query.is_none());
            assert!(!view.tmux_owned_instance_is_current(&old_instance, ctx));
            assert_eq!(
                view.tmux_restored_native_session(CLIAgent::Claude, ctx),
                Some(native)
            );
            assert!(view.tmux_restored_claude_image(ctx).is_some());
            assert!(view.remote_image_binding_is_available_for_test(ctx));
            assert!(
                view.cli_agent_hook_input_target
                    .as_ref()
                    .is_some_and(|target| target.listener_id != old_listener)
            );
        });
        let messages = toast_messages(&mut app);
        fixture.terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("恢复后的文字", ctx)
            });
            view.submit_text_to_cli_agent_pty("恢复后的文字".into(), ctx);
        });
        fixture.terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "恢复后的文字");
            assert!(view.remote_image_binding_is_available_for_test(ctx));
        });
        assert_eq!(messages.borrow().len(), 1);
        assert!(fixture.requests.try_recv().is_err());
        fixture.close(&mut app).await;
    });
}

#[test]
fn tmux_restored_hook_identity_rejects_another_native_session_or_launch() {
    for replace_launch in [false, true] {
        App::test((), |mut app| async move {
            let mut fixture = Fixture::with_owned(&mut app, true).await;
            crate::features::FeatureFlag::CLIAgentRichInput.set_enabled(true);
            crate::features::FeatureFlag::PluggableNotifications.set_enabled(true);
            let id = seed_restore(&fixture, &mut app);
            let status = bound_status(&fixture, &mut app).await;
            let native = Uuid::new_v4();
            let active = state_changed(&mut app, &fixture.terminal, |view| {
                view.tmux_remote_owned
                    .as_ref()
                    .is_some_and(|owned| owned.phase == Phase::Active)
            });
            respond_status(status, id, native);
            bounded(active).await.unwrap();
            let observed = state_changed(&mut app, &fixture.terminal, |view| {
                view.tmux_remote_owned
                    .as_ref()
                    .is_some_and(|owned| owned.observed_identity.is_some())
            });
            send_start(&fixture, &mut app, native);
            bounded(observed).await.unwrap();
            complete_original_block(&fixture, &mut app).await;
            fixture.reconnect_as(&mut app, SessionId::from(7502)).await;
            if replace_launch {
                fixture.terminal.update(&mut app, |view, _| {
                    let launch = view
                        .tmux_remote_owned
                        .as_mut()
                        .unwrap()
                        .launch
                        .as_mut()
                        .unwrap();
                    launch.id = Uuid::new_v4();
                    launch.key = Uuid::new_v4();
                });
            }
            let restored_id = prepare_retained_restore(&fixture, &mut app);
            let status = bound_status(&fixture, &mut app).await;
            let active = state_changed(&mut app, &fixture.terminal, |view| {
                view.tmux_remote_owned
                    .as_ref()
                    .is_some_and(|owned| owned.phase == Phase::Active)
            });
            respond_status(
                status,
                restored_id,
                if replace_launch {
                    native
                } else {
                    Uuid::new_v4()
                },
            );
            bounded(active).await.unwrap();
            fixture.terminal.read(&app, |view, ctx| {
                assert!(
                    CLIAgentSessionsModel::as_ref(ctx)
                        .session(view.view_id)
                        .is_none()
                );
                assert!(view.tmux_restored_claude_image(ctx).is_none());
                assert!(view.cli_agent_hook_input_target.is_none());
            });
            fixture.close(&mut app).await;
        });
    }
}

#[test]
fn tmux_grok_readonly_observe_never_replays_old_hook_permission_or_state() {
    for valid_mode in [true, false] {
        check_grok_restored_identity(valid_mode, None, false);
    }
}

#[test]
fn tmux_grok_real_permission_event_revokes_cached_and_pending_readonly_proof() {
    for mode in [Some("plan"), None] {
        for while_pending in [true, false] {
            check_grok_restored_identity(true, Some(mode), while_pending);
        }
    }
}

async fn send_grok_permission_event(
    fixture: &Fixture,
    app: &mut App,
    native: Uuid,
    kind: &str,
    mode: Option<&str>,
) {
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    let view_id = fixture.terminal.read(app, |view, _| view.view_id);
    app.update(|ctx| {
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), move |_, event, _| {
            if matches!(event, CLIAgentSessionsModelEvent::SessionUpdated {terminal_view_id, ..} if *terminal_view_id == view_id)
                && let Some(sender) = sender.take()
            {
                let _ = sender.send(());
            }
        });
    });
    let body = serde_json::json!({"v":1,"agent":"grok","event":kind,"session_id":native,
        "cwd":"/fixture/project","plugin_version":"0.1.5","event_id":Uuid::new_v4(),
        "prompt_id":"restored-turn","permission_mode":mode})
    .to_string();
    fixture.terminal.update(app, |view, _| {
        let mut model = view.model.lock();
        Processor::new().parse_bytes(
            &mut *model,
            format!("\x1b]777;notify;warp://cli-agent;{body}\x07").as_bytes(),
            &mut std::io::sink(),
        );
    });
    bounded(receiver).await.unwrap();
}

fn check_grok_restored_identity(
    valid_mode: bool,
    changed_mode: Option<Option<&'static str>>,
    while_pending: bool,
) {
    App::test((), |mut app| async move {
        let fixture = Fixture::with_grok_owned(&mut app).await;
        crate::features::FeatureFlag::CLIAgentRichInput.set_enabled(true);
        crate::features::FeatureFlag::PluggableNotifications.set_enabled(true);
        let native = Uuid::new_v4();
        let ticket = crate::remote_server::cli_image_grok_protocol::Ticket {
            id: Uuid::new_v4(),
            key: Uuid::new_v4(),
        };
        let (attempt, begin) = fixture.begin(&mut app).await;
        begin.written();
        let nonce = Uuid::new_v4();
        fixture.osc(&mut app, attempt, nonce);
        fixture.finish_ack(&mut app, attempt, nonce).await;
        // 仅装配已经通过 Status/Journal 的内存票据；本用例专验真实 Observe 回调。
        fixture.terminal.update(&mut app, |view, ctx| {
            let (snapshot, client) = view.terminal_binding_snapshot(ctx).unwrap();
            let scope = client.terminal_binding_scope(snapshot.session).unwrap();
            view.tmux_remote_owned = Some(TmuxOwned {
                operation: Uuid::new_v4(),
                snapshot: snapshot.clone(),
                client,
                scope: scope.clone(),
                launch: Some(Launch {
                    version: 1,
                    host: scope.host_id,
                    terminal_session: snapshot.session.as_u64(),
                    block_id: snapshot.block.to_string(),
                    agent: TerminalBindingOwnedAgent::Grok as i32,
                    id: ticket.id,
                    key: ticket.key,
                }),
                agent: CLIAgent::Grok,
                attempt: Some(attempt),
                pane_cwd: Some("/fixture/project".into()),
                phase: Phase::Active,
                restore: false,
                explicit_restore: false,
                launched: true,
                native_session: Some(native),
                starts: Vec::new(),
                observed_identity: None,
                restored_identity: None,
            });
            view.bind_tmux_owned_grok_for_test(ticket.clone(), native, ctx);
        });
        let observed = state_changed(&mut app, &fixture.terminal, |view| {
            view.tmux_remote_owned
                .as_ref()
                .is_some_and(|owned| owned.observed_identity.is_some())
        });
        let body = serde_json::json!({"v":1,"agent":"grok","event":"session_start","session_id":native,
                "cwd":"/fixture/project","plugin_version":"0.1.5","event_id":"original-start","permission_mode":"default"}).to_string();
        fixture.terminal.update(&mut app, |view, _| {
            let mut model = view.model.lock();
            Processor::new().parse_bytes(
                &mut *model,
                format!("\x1b]777;notify;warp://cli-agent;{body}\x07").as_bytes(),
                &mut std::io::sink(),
            );
        });
        bounded(observed).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            assert!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .is_remote()
            );
        });
        let (ended, ended_receiver) = oneshot::channel();
        let mut ended = Some(ended);
        let view_id = fixture.terminal.read(&app, |view, _| view.view_id);
        app.update(|ctx| {
                ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), move |_, event, _| {
                    if matches!(event, CLIAgentSessionsModelEvent::Ended {terminal_view_id, ..} if *terminal_view_id == view_id)
                        && let Some(ended) = ended.take() { let _ = ended.send(()); }
                });
            });
        fixture.terminal.update(&mut app, |view, ctx| {
            view.grok_remote_owned.take();
            // 专测 Observe 回调时直接移除 listener；同步真实 BlockCompleted 的目标清理。
            view.cli_agent_hook_input_target = None;
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.remove_session(view.view_id, ctx)
            });
        });
        bounded(ended_receiver).await.unwrap();
        let (new_attempt, begin) = fixture.begin(&mut app).await;
        begin.written();
        let nonce = Uuid::new_v4();
        fixture.osc(&mut app, new_attempt, nonce);
        fixture.finish_ack(&mut app, new_attempt, nonce).await;
        fixture.terminal.update(&mut app, |view, ctx| {
            let owned = view.tmux_remote_owned.as_mut().unwrap();
            owned.operation = Uuid::new_v4();
            owned.attempt = Some(new_attempt);
            owned.phase = Phase::Active;
            owned.restore = true;
            owned.native_session = Some(native);
            view.bind_tmux_owned_grok_for_test(ticket.clone(), native, ctx);
            view.restore_tmux_hook_identity(ctx);
        });
        let observation = bounded(fixture.grok_observations.recv()).await.unwrap();
        assert!(observation.ticket == ticket);
        if let Some(mode) = changed_mode.filter(|_| while_pending) {
            send_grok_permission_event(&fixture, &mut app, native, "notification", mode).await;
            fixture.terminal.read(&app, |view, ctx| {
                assert!(view.remote_owned_grok_readonly_identity(ctx).is_none());
                assert!(!view.remote_grok_identity_query_pending_for_test());
                assert!(
                    CLIAgentSessionsModel::as_ref(ctx)
                        .session(view.view_id)
                        .unwrap()
                        .session_context
                        .grok_owned_identity_epoch
                        .is_none()
                );
            });
        }
        // 权限事件已完成后才订阅；等实际回复回调的界面刷新，不能用等待时长代替完成。
        let completed = state_changed(&mut app, &fixture.terminal, |view| {
            !view.remote_grok_identity_query_pending_for_test()
        });
        observation
            .reply
            .send(
                crate::remote_server::cli_image_grok_protocol::Reply::OwnedIdentity {
                    ticket,
                    native_session: native,
                    cwd: "/fixture/project".into(),
                    manifest_sha256: "a".repeat(64),
                    permission_mode: if valid_mode { "default" } else { "auto" }.into(),
                },
            )
            .unwrap_or_else(|_| panic!("当前只读回复接收端已关闭"));
        bounded(completed).await.unwrap();
        fixture.terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .unwrap();
            assert_eq!(session.status, CLIAgentSessionStatus::Unknown);
            assert_eq!(session.input_state, CLIAgentInputState::Closed);
            assert!(!session.should_auto_toggle_input);
            if !while_pending {
                assert!(!session.received_rich_notification);
                assert_eq!(
                    session.session_context.grok_permission_evidence,
                    crate::terminal::cli_agent_sessions::GrokPermissionEvidence::Unobserved
                );
            }
            assert_eq!(
                view.remote_owned_grok_readonly_identity(ctx).is_some(),
                valid_mode && !while_pending
            );
            if !while_pending {
                assert_eq!(view.cli_agent_hook_input_target.is_some(), valid_mode);
            }
        });
        if let Some(mode) = changed_mode.filter(|_| !while_pending) {
            // 首个真实 default PromptSubmit 只更新当前状态，不应撤销只读原生身份。
            send_grok_permission_event(
                &fixture,
                &mut app,
                native,
                "prompt_submit",
                Some("default"),
            )
            .await;
            fixture.terminal.read(&app, |view, ctx| {
                assert!(view.remote_owned_grok_readonly_identity(ctx).is_some());
                assert!(
                    CLIAgentSessionsModel::as_ref(ctx)
                        .session(view.view_id)
                        .unwrap()
                        .session_context
                        .grok_owned_identity_epoch
                        .is_some()
                );
            });
            send_grok_permission_event(&fixture, &mut app, native, "notification", mode).await;
            fixture.terminal.read(&app, |view, ctx| {
                assert!(view.remote_owned_grok_readonly_identity(ctx).is_none());
                assert!(!view.remote_grok_identity_query_pending_for_test());
                assert!(
                    CLIAgentSessionsModel::as_ref(ctx)
                        .session(view.view_id)
                        .unwrap()
                        .session_context
                        .grok_owned_identity_epoch
                        .is_none()
                );
            });
        }
        assert!(fixture.grok_observations.try_recv().is_err());
        fixture.terminal.update(&mut app, |view, _| {
            view.grok_remote_owned.take();
        });
        fixture.close(&mut app).await;
    });
}
