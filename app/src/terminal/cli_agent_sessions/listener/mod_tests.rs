use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use warpui::App;
use warpui::r#async::Timer;

use super::*;
use crate::terminal::cli_agent_sessions::event::{
    CLI_AGENT_NOTIFICATION_SENTINEL, CLIAgentEventSource, CLIAgentEventType,
};
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModelEvent};
use crate::terminal::event::Event as TerminalEvent;
use crate::terminal::model::session::Sessions;

#[test]
fn codex_parses_text_without_inferring_completion() {
    let event = CodexSessionHandler::parse_osc9_text("Agent turn complete").unwrap();
    assert_eq!(event.event, CLIAgentEventType::Notification);
    assert_eq!(event.agent, CLIAgent::Codex);
    assert_eq!(
        event.payload.summary.as_deref(),
        Some("Agent turn complete")
    );
}

#[test]
fn codex_body_becomes_notification_summary() {
    let event =
        CodexSessionHandler::parse_osc9_text("I've updated the README with the new instructions.")
            .unwrap();
    assert_eq!(event.event, CLIAgentEventType::Notification);
    assert_eq!(
        event.payload.summary.as_deref(),
        Some("I've updated the README with the new instructions.")
    );
}

#[test]
fn codex_approval_text_is_not_a_completion() {
    let event =
        CodexSessionHandler::parse_osc9_text("Approval requested: rm -rf /tmp/foo").unwrap();
    assert_eq!(event.event, CLIAgentEventType::Notification);
    assert_eq!(
        event.payload.summary.as_deref(),
        Some("Approval requested: rm -rf /tmp/foo")
    );
}

#[test]
fn codex_ignores_empty_body() {
    assert!(CodexSessionHandler::parse_osc9_text("").is_none());
    assert!(CodexSessionHandler::parse_osc9_text("   ").is_none());
}

#[test]
fn codex_try_parse_ignores_titled_notifications() {
    let mut handler = CodexSessionHandler;
    assert!(
        handler
            .try_parse(Some("some-title"), "Agent turn complete", false)
            .is_none()
    );
}

#[test]
fn codex_try_parse_handles_osc9() {
    let mut handler = CodexSessionHandler;
    let event = handler
        .try_parse(None, "Agent turn complete", false)
        .unwrap();
    assert_eq!(event.event, CLIAgentEventType::Notification);
}

#[test]
fn codex_try_parse_ignores_osc9_when_plugin_already_active() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    let mut handler = CodexSessionHandler;
    let body = r#"{"v":1,"agent":"codex","event":"permission_request","summary":"Approve?","tool_name":"Bash"}"#;

    let event = handler
        .try_parse(Some(CLI_AGENT_NOTIFICATION_SENTINEL), body, false)
        .unwrap();

    assert_eq!(event.event, CLIAgentEventType::PermissionRequest);
    // Once the session is rich, OSC 9 fallback is dropped.
    assert!(
        handler
            .try_parse(None, "Agent turn complete", true)
            .is_none()
    );
}

#[test]
fn codex_try_parse_ignores_structured_event_without_codex_plugin() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(false);
    let mut handler = CodexSessionHandler;
    let body = r#"{"v":1,"agent":"codex","event":"permission_request","summary":"Approve?","tool_name":"Bash"}"#;

    assert!(
        handler
            .try_parse(Some(CLI_AGENT_NOTIFICATION_SENTINEL), body, false)
            .is_none()
    );
    assert!(
        handler
            .try_parse(None, "Agent turn complete", false)
            .is_some()
    );
}

#[test]
fn codex_try_parse_ignores_other_structured_agents() {
    let mut handler = CodexSessionHandler;
    let body = r#"{"v":1,"agent":"claude","event":"stop"}"#;

    assert!(
        handler
            .try_parse(Some(CLI_AGENT_NOTIFICATION_SENTINEL), body, false)
            .is_none()
    );
    assert!(
        handler
            .try_parse(None, "Agent turn complete", false)
            .is_some()
    );
}

#[test]
fn auggie_is_supported() {
    assert!(is_agent_supported(&CLIAgent::Auggie));
}

#[test]
fn auggie_default_handler_skips_session_start() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Auggie,
        event: CLIAgentEventType::SessionStart,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_none());
}

#[test]
fn auggie_default_handler_forwards_stop() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Auggie,
        event: CLIAgentEventType::Stop,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_some());
}

#[test]
fn pi_is_supported() {
    assert!(is_agent_supported(&CLIAgent::Pi));
}

#[test]
fn oh_my_pi_is_supported() {
    assert!(is_agent_supported(&CLIAgent::OhMyPi));
}

#[test]
fn pi_default_handler_skips_session_start() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Pi,
        event: CLIAgentEventType::SessionStart,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_none());
}

#[test]
fn pi_default_handler_forwards_stop() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Pi,
        event: CLIAgentEventType::Stop,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_some());
}

#[test]
fn droid_is_supported() {
    assert!(is_agent_supported(&CLIAgent::Droid));
}

#[test]
fn droid_default_handler_skips_session_start() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Droid,
        event: CLIAgentEventType::SessionStart,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_none());
}

#[test]
fn droid_default_handler_forwards_stop() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Droid,
        event: CLIAgentEventType::Stop,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_some());
}

#[test]
fn droid_default_handler_forwards_permission_request() {
    let mut handler = DefaultSessionListener;
    let event = CLIAgentEvent {
        source: CLIAgentEventSource::RichPlugin,
        v: 1,
        agent: CLIAgent::Droid,
        event: CLIAgentEventType::PermissionRequest,
        session_id: None,
        cwd: None,
        project: None,
        payload: CLIAgentEventPayload::default(),
    };
    assert!(handler.handle_event(event).is_some());
}

#[test]
fn warp_tui_notifications_are_supported() {
    assert!(is_agent_supported(&CLIAgent::WarpTui));
    let mut handler = create_handler(&CLIAgent::WarpTui).expect("should create handler");
    let stop_body = r#"{"v":1,"agent":"warp-tui","event":"stop","session_id":"sess-42"}"#;
    let parsed_stop = handler
        .try_parse(Some(CLI_AGENT_NOTIFICATION_SENTINEL), stop_body, false)
        .expect("should parse stop");
    assert_eq!(parsed_stop.agent, CLIAgent::WarpTui);
    assert_eq!(parsed_stop.event, CLIAgentEventType::Stop);
    assert!(handler.handle_event(parsed_stop).is_some());
}

#[test]
fn oh_my_pi_end_to_end_parsing_and_handling() {
    let mut handler = create_handler(&CLIAgent::OhMyPi).expect("should create handler");

    // Test session_start payload: proves SessionStart is skipped
    let start_body = r#"{"v":1,"agent":"omp","event":"session_start"}"#;
    let parsed_start = handler
        .try_parse(Some(CLI_AGENT_NOTIFICATION_SENTINEL), start_body, false)
        .expect("should successfully parse session_start payload");
    assert_eq!(parsed_start.agent, CLIAgent::OhMyPi);
    assert_eq!(parsed_start.event, CLIAgentEventType::SessionStart);
    assert!(handler.handle_event(parsed_start).is_none());

    // Test stop payload: proves Stop forwards with CLIAgent::OhMyPi
    let stop_body = r#"{"v":1,"agent":"omp","event":"stop"}"#;
    let parsed_stop = handler
        .try_parse(Some(CLI_AGENT_NOTIFICATION_SENTINEL), stop_body, false)
        .expect("should successfully parse stop payload");
    assert_eq!(parsed_stop.agent, CLIAgent::OhMyPi);
    assert_eq!(parsed_stop.event, CLIAgentEventType::Stop);

    let handled_stop = handler
        .handle_event(parsed_stop)
        .expect("should forward stop event");
    assert_eq!(handled_stop.agent, CLIAgent::OhMyPi);
    assert_eq!(handled_stop.event, CLIAgentEventType::Stop);
}

async fn listener_dispatch_received(receiver: &async_channel::Receiver<String>) -> String {
    match futures::future::select(
        Box::pin(receiver.recv()),
        Box::pin(Timer::after(Duration::from_secs(5))),
    )
    .await
    {
        futures::future::Either::Left((Ok(body), _)) => body,
        futures::future::Either::Left((Err(error), _)) => {
            panic!("真实 dispatcher 的观察通道提前关闭: {error}")
        }
        futures::future::Either::Right(_) => panic!("真实 dispatcher 未在期限内派发通知"),
    }
}

#[test]
fn replaced_listener_queued_and_late_dispatches_cannot_poison_current_turn() {
    let _guard = FeatureFlag::CodexPlugin.override_enabled(true);
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let terminal_sessions = app.add_model(|_| Sessions::new_for_test());
        let view_id = EntityId::new();
        let (old_tx, old_rx) = async_channel::unbounded();
        let (current_tx, current_rx) = async_channel::unbounded();
        let old_dispatcher =
            app.add_model(|ctx| ModelEventDispatcher::new(old_rx, terminal_sessions.clone(), ctx));
        let current_dispatcher = app
            .add_model(|ctx| ModelEventDispatcher::new(current_rx, terminal_sessions.clone(), ctx));
        let old_listener = app.add_model(|ctx| {
            CLIAgentSessionListener::new(view_id, CLIAgent::Codex, &old_dispatcher, ctx)
        });
        model.update(&mut app, |model, ctx| {
            model.register_listener(
                view_id,
                CLIAgent::Codex,
                None,
                None,
                None,
                None,
                None,
                false,
                old_listener.clone(),
                ctx,
            );
        });
        let current_listener = app.add_model(|ctx| {
            CLIAgentSessionListener::new(view_id, CLIAgent::Codex, &current_dispatcher, ctx)
        });

        let (old_seen_tx, old_seen_rx) = async_channel::unbounded();
        let (current_seen_tx, current_seen_rx) = async_channel::unbounded();
        let unverified_stop_count = Rc::new(Cell::new(0));
        let observed_unverified_stop_count = unverified_stop_count.clone();
        app.update(|ctx| {
            // 观察真实 dispatcher 的广播作为屏障，不直接调用 listener 或会话事件处理器。
            ctx.subscribe_to_model(&old_dispatcher, move |_, event, _| {
                if let ModelEvent::PluggableNotification { body, .. } = event {
                    old_seen_tx
                        .try_send(body.clone())
                        .expect("旧通知观察接收端仍存在");
                }
            });
            ctx.subscribe_to_model(&current_dispatcher, move |_, event, _| {
                if let ModelEvent::PluggableNotification { body, .. } = event {
                    current_seen_tx
                        .try_send(body.clone())
                        .expect("当前通知观察接收端仍存在");
                }
            });
            ctx.subscribe_to_model(&model, move |_, event, _| {
                if let CLIAgentSessionsModelEvent::StatusChanged {
                    terminal_view_id,
                    status: CLIAgentSessionStatus::Unknown,
                    ..
                } = event
                    && *terminal_view_id == view_id
                {
                    observed_unverified_stop_count.set(observed_unverified_stop_count.get() + 1);
                }
            });
        });

        // 旧终态先进入实际 terminal event 队列；当前前台回合尚未让出执行权。
        // 缺原生关联标识模拟旧插件，避免测试被 session/turn 比较提前拦下而漏测实例保护。
        let queued_stop =
            r#"{"v":1,"agent":"codex","event":"stop","sequence":999,"response":"旧排队结果"}"#;
        old_tx
            .try_send(TerminalEvent::PluggableNotification {
                title: Some(CLI_AGENT_NOTIFICATION_SENTINEL.to_owned()),
                body: queued_stop.to_owned(),
            })
            .expect("旧 dispatcher 输入应仍有效");
        model.update(&mut app, |model, ctx| {
            model.remove_session(view_id, ctx);
            model.register_listener(
                view_id,
                CLIAgent::Codex,
                None,
                None,
                None,
                None,
                None,
                false,
                current_listener.clone(),
                ctx,
            );
        });
        let prompt = r#"{"v":1,"agent":"codex","event":"prompt_submit","session_id":"current-session","turn_id":"current-turn","sequence":1,"query":"当前请求"}"#;
        current_tx
            .try_send(TerminalEvent::PluggableNotification {
                title: Some(CLI_AGENT_NOTIFICATION_SENTINEL.to_owned()),
                body: prompt.to_owned(),
            })
            .expect("当前 dispatcher 输入应有效");
        assert_eq!(listener_dispatch_received(&old_seen_rx).await, queued_stop);
        assert_eq!(listener_dispatch_received(&current_seen_rx).await, prompt);
        model.read(&app, |model, _| {
            let session = model.session(view_id).expect("替换后的会话应存在");
            assert_eq!(
                session.listener.as_ref().unwrap().id(),
                current_listener.id()
            );
            assert_ne!(session.listener.as_ref().unwrap().id(), old_listener.id());
            assert_eq!(session.status, CLIAgentSessionStatus::InProgress);
            assert_eq!(
                session.session_context.session_id.as_deref(),
                Some("current-session")
            );
            assert_eq!(session.session_context.query.as_deref(), Some("当前请求"));
            assert_eq!(session.session_context.response, None);
            assert!(session.received_rich_notification);
        });
        assert_eq!(unverified_stop_count.get(), 0);

        // 当前 PromptSubmit 已经确认后，保留强句柄的旧 listener 再收到一条迟到终态。
        let late_stop =
            r#"{"v":1,"agent":"codex","event":"stop","sequence":1000,"response":"旧迟到结果"}"#;
        old_tx
            .try_send(TerminalEvent::PluggableNotification {
                title: Some(CLI_AGENT_NOTIFICATION_SENTINEL.to_owned()),
                body: late_stop.to_owned(),
            })
            .expect("被替换但仍持有句柄的 dispatcher 应继续派发");
        assert_eq!(listener_dispatch_received(&old_seen_rx).await, late_stop);
        model.read(&app, |model, _| {
            let session = model.session(view_id).unwrap();
            assert_eq!(session.status, CLIAgentSessionStatus::InProgress);
            assert_eq!(session.session_context.response, None);
        });
        assert_eq!(unverified_stop_count.get(), 0);

        // 用较小的当前序号证明旧事件没有消费游标，而不窥探游标的私有字段。
        let stop = r#"{"v":1,"agent":"codex","event":"stop","session_id":"current-session","turn_id":"current-turn","sequence":2,"response":"当前结果"}"#;
        current_tx
            .try_send(TerminalEvent::PluggableNotification {
                title: Some(CLI_AGENT_NOTIFICATION_SENTINEL.to_owned()),
                body: stop.to_owned(),
            })
            .expect("当前 Stop 应进入真实 dispatcher");
        assert_eq!(listener_dispatch_received(&current_seen_rx).await, stop);
        model.read(&app, |model, _| {
            let session = model.session(view_id).unwrap();
            assert_eq!(session.status, CLIAgentSessionStatus::Unknown);
            assert_eq!(session.session_context.query.as_deref(), Some("当前请求"));
            assert_eq!(
                session.session_context.response.as_deref(),
                Some("当前结果")
            );
        });
        assert_eq!(unverified_stop_count.get(), 1);

        current_tx
            .try_send(TerminalEvent::PluggableNotification {
                title: Some(CLI_AGENT_NOTIFICATION_SENTINEL.to_owned()),
                body: stop.to_owned(),
            })
            .expect("重复终态应进入真实 dispatcher");
        assert_eq!(listener_dispatch_received(&current_seen_rx).await, stop);
        assert_eq!(unverified_stop_count.get(), 1);
        drop(old_listener);
    });
}

#[test]
fn grok_listener_preserves_late_session_start_for_permission_invalidation() {
    let mut handler = create_handler(&CLIAgent::Grok).unwrap();
    let body = r#"{"v":1,"agent":"grok","event":"session_start","session_id":"current","permission_mode":"auto","event_id":"late-start"}"#;
    let parsed = handler
        .try_parse(Some(CLI_AGENT_NOTIFICATION_SENTINEL), body, true)
        .unwrap();
    let handled = handler
        .handle_event(parsed)
        .expect("Grok 权限变化不得在 listener 提前丢弃");
    assert_eq!(handled.event, CLIAgentEventType::SessionStart);
    assert_eq!(handled.payload.permission_mode.as_deref(), Some("auto"));
}
