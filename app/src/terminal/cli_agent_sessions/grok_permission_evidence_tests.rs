use warpui::{App, EntityId};

use super::*;
use crate::terminal::cli_agent_sessions::event::parse_event;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};

fn event(kind: &str, event_id: &str, mode: serde_json::Value) -> CLIAgentEvent {
    parse_event(
        Some("warp://cli-agent"),
        &serde_json::json!({
            "v":1,"agent":"grok","event":kind,"session_id":"active-session",
            "cwd":"/private/tmp/project","event_id":event_id,"permission_mode":mode,
            "plugin_version":"0.1.5"
        })
        .to_string(),
    )
    .unwrap()
}

fn session() -> CLIAgentSession {
    CLIAgentSession {
        agent: CLIAgent::Grok,
        status: CLIAgentSessionStatus::InProgress,
        session_context: CLIAgentSessionContext {
            session_id: Some("active-session".to_owned()),
            cwd: Some("/private/tmp/project".to_owned()),
            ..Default::default()
        },
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        plugin_version: None,
        remote_host: None,
        draft_text: None,
        custom_command_prefix: None,
        received_rich_notification: false,
    }
}

#[test]
fn native_permission_field_survives_v1_parser_without_becoming_approval() {
    let parsed = event("session_start", "start", serde_json::json!("default"));
    assert_eq!(parsed.payload.permission_mode.as_deref(), Some("default"));
    assert_eq!(parsed.event, CLIAgentEventType::SessionStart);
}

#[test]
fn malformed_permission_field_keeps_event_for_invalidation() {
    let parsed = event(
        "notification",
        "malformed",
        serde_json::json!({"default":true}),
    );
    assert_eq!(parsed.event, CLIAgentEventType::Notification);
    assert_eq!(parsed.payload.permission_mode, None);
}

#[test]
fn absent_permission_field_is_not_default() {
    let parsed = parse_event(
        Some("warp://cli-agent"),
        r#"{"v":1,"agent":"grok","event":"session_start"}"#,
    )
    .unwrap();
    assert_eq!(parsed.payload.permission_mode, None);
}

#[test]
fn empty_permission_field_is_not_evidence() {
    assert_eq!(
        event("session_start", "empty", serde_json::json!(""))
            .payload
            .permission_mode,
        None
    );
}

#[test]
fn context_records_exact_session_start_identity_and_native_mode() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |model, ctx| {
            model.set_session(view_id, session(), ctx);
            let start = event(
                "session_start",
                "native-start",
                serde_json::json!("default"),
            );
            model.update_from_event(view_id, &start, ctx);
            model.update_from_event(view_id, &start, ctx);
            assert_eq!(
                model
                    .session(view_id)
                    .unwrap()
                    .session_context
                    .grok_permission_evidence,
                GrokPermissionEvidence::Observed(GrokPermissionObservation {
                    session_id: "active-session".to_owned(),
                    cwd: "/private/tmp/project".to_owned(),
                    session_start_event_id: "native-start".to_owned(),
                    mode: "default".to_owned(),
                })
            );
        });
    });
}

#[test]
fn missing_mode_invalidates_and_late_session_start_cannot_restore() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |model, ctx| {
            model.set_session(view_id, session(), ctx);
            model.update_from_event(
                view_id,
                &event("session_start", "start", serde_json::json!("default")),
                ctx,
            );
            model.update_from_event(
                view_id,
                &event("notification", "missing", serde_json::Value::Null),
                ctx,
            );
            model.update_from_event(
                view_id,
                &event("session_start", "late-start", serde_json::json!("default")),
                ctx,
            );
            assert_eq!(
                model
                    .session(view_id)
                    .unwrap()
                    .session_context
                    .grok_permission_evidence,
                GrokPermissionEvidence::Invalidated
            );
        });
    });
}

#[test]
fn unscoped_permission_reminder_invalidates_changed_mode_before_early_return() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |model, ctx| {
            model.set_session(view_id, session(), ctx);
            model.update_from_event(
                view_id,
                &event("session_start", "start", serde_json::json!("default")),
                ctx,
            );
            let mut prompt = event("prompt_submit", "prompt", serde_json::json!("default"));
            prompt.payload.prompt_id = Some("active-prompt".to_owned());
            model.update_from_event(view_id, &prompt, ctx);
            model.update_from_event(
                view_id,
                &event(
                    "permission_request",
                    "permission",
                    serde_json::json!("auto"),
                ),
                ctx,
            );
            let active = model.session(view_id).unwrap();
            assert_eq!(
                active.session_context.grok_permission_evidence,
                GrokPermissionEvidence::Invalidated
            );
            assert_eq!(active.status, CLIAgentSessionStatus::InProgress);
        });
    });
}

#[test]
fn rejected_old_event_does_not_change_current_permission_evidence() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let view_id = EntityId::new();
        model.update(&mut app, |model, ctx| {
            model.set_session(view_id, session(), ctx);
            let mut start = event("session_start", "start", serde_json::json!("default"));
            start.payload.sequence = Some(2);
            model.update_from_event(view_id, &start, ctx);
            let before = model
                .session(view_id)
                .unwrap()
                .session_context
                .grok_permission_evidence
                .clone();
            let mut old = event("notification", "old", serde_json::Value::Null);
            old.payload.sequence = Some(1);
            model.update_from_event(view_id, &old, ctx);
            assert_eq!(
                model
                    .session(view_id)
                    .unwrap()
                    .session_context
                    .grok_permission_evidence,
                before
            );
        });
    });
}

#[test]
fn first_event_after_session_start_was_missed_cannot_grant_evidence_later() {
    let mut evidence = GrokPermissionEvidence::default();
    let notice = event("notification", "notice", serde_json::json!("default"));
    evidence.observe(
        &notice,
        Some("active-session"),
        Some("/private/tmp/project"),
    );
    evidence.observe(
        &event("session_start", "late-start", serde_json::json!("default")),
        Some("active-session"),
        Some("/private/tmp/project"),
    );
    assert_eq!(evidence, GrokPermissionEvidence::Invalidated);
}

#[test]
fn listener_invalidation_cannot_be_reversed_by_new_start_callback() {
    let mut evidence = GrokPermissionEvidence::default();
    evidence.observe(
        &event("session_start", "start", serde_json::json!("default")),
        Some("active-session"),
        Some("/private/tmp/project"),
    );
    evidence.invalidate();
    evidence.observe(
        &event(
            "session_start",
            "another-start",
            serde_json::json!("default"),
        ),
        Some("active-session"),
        Some("/private/tmp/project"),
    );
    assert_eq!(evidence, GrokPermissionEvidence::Invalidated);
}

#[test]
fn foreign_agent_cannot_grant_grok_permission_evidence() {
    let mut evidence = GrokPermissionEvidence::default();
    let mut inherited = event("session_start", "inherited", serde_json::json!("default"));
    inherited.agent = CLIAgent::Claude;
    assert!(!evidence.observe(
        &inherited,
        Some("active-session"),
        Some("/private/tmp/project")
    ));
    assert_eq!(evidence, GrokPermissionEvidence::Unobserved);
}
