use super::*;
use crate::remote_server::proto::{
    TerminalBindingFailed, terminal_binding_request, terminal_binding_response,
};
use crate::terminal::model::ansi::Processor;
use crate::terminal::model_events::ModelEvent;
use crate::terminal::view::terminal_binding::tests::{Fixture, Wire, bounded, state_changed};
use crate::workspace::ToastStackEvent;
use futures::channel::oneshot;
use std::cell::RefCell;
use std::rc::Rc;
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
        "cwd":"/fixture/project","plugin_version":"0.5.0"}).to_string();
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
