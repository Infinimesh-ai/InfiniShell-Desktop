use super::*;
use crate::terminal::cli_agent_sessions::event::CLIAgentEventPayload;

fn event(agent: CLIAgent, kind: CLIAgentEventType, turn: Option<&str>) -> CLIAgentEvent {
    let payload = CLIAgentEventPayload {
        turn_id: (agent == CLIAgent::Codex)
            .then_some(turn)
            .flatten()
            .map(str::to_owned),
        prompt_id: matches!(agent, CLIAgent::Claude | CLIAgent::Grok)
            .then_some(turn)
            .flatten()
            .map(str::to_owned),
        ..Default::default()
    };
    CLIAgentEvent {
        v: 1,
        agent,
        event: kind,
        session_id: Some("native-session".to_owned()),
        cwd: None,
        project: None,
        payload,
        source: CLIAgentEventSource::RichPlugin,
    }
}

#[test]
fn native_turns_reject_old_completion_and_replayed_prompt() {
    for agent in [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok] {
        let mut cursor = EventCursor::default();
        for turn in ["first", "second"] {
            assert_eq!(
                cursor.accept(&event(agent, CLIAgentEventType::PromptSubmit, Some(turn))),
                EventDisposition::Accept
            );
        }
        for kind in [
            CLIAgentEventType::Stop,
            CLIAgentEventType::StopFailure,
            CLIAgentEventType::Cancelled,
            CLIAgentEventType::PermissionRequest,
            CLIAgentEventType::ToolComplete,
            CLIAgentEventType::PromptSubmit,
        ] {
            assert_eq!(
                cursor.accept(&event(agent, kind, Some("first"))),
                EventDisposition::Drop
            );
        }
        assert_eq!(
            cursor.accept(&event(agent, CLIAgentEventType::Stop, Some("second"))),
            EventDisposition::Accept
        );
    }
}

#[test]
fn local_input_closes_previous_turn_before_native_prompt_arrives() {
    let mut cursor = EventCursor::default();
    let agent = CLIAgent::Claude;
    cursor.accept(&event(agent, CLIAgentEventType::PromptSubmit, Some("old")));
    let mut local = event(agent, CLIAgentEventType::PromptSubmit, None);
    local.source = CLIAgentEventSource::LocalRichInput;
    assert_eq!(cursor.accept(&local), EventDisposition::Accept);
    assert_eq!(
        cursor.accept(&event(agent, CLIAgentEventType::Stop, Some("old"))),
        EventDisposition::Drop
    );
    assert_eq!(
        cursor.accept(&event(agent, CLIAgentEventType::PromptSubmit, Some("old"))),
        EventDisposition::Drop
    );
    assert_eq!(
        cursor.accept(&event(agent, CLIAgentEventType::Stop, None)),
        EventDisposition::Drop
    );
    assert_eq!(
        cursor.accept(&event(agent, CLIAgentEventType::PromptSubmit, Some("new"))),
        EventDisposition::Accept
    );
    assert_eq!(
        cursor.accept(&event(agent, CLIAgentEventType::Stop, Some("new"))),
        EventDisposition::Accept
    );
}

#[test]
fn missing_prompt_or_native_correlation_never_confirms_terminal() {
    for agent in [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok] {
        for kind in [
            CLIAgentEventType::Stop,
            CLIAgentEventType::StopFailure,
            CLIAgentEventType::Cancelled,
        ] {
            let mut cursor = EventCursor::default();
            assert_eq!(
                cursor.accept(&event(agent, kind.clone(), Some("unseen"))),
                EventDisposition::UnverifiedTerminal
            );
            cursor.accept(&event(agent, CLIAgentEventType::PromptSubmit, None));
            assert_eq!(
                cursor.accept(&event(agent, kind, None)),
                EventDisposition::UnverifiedTerminal
            );
        }
    }
}

#[test]
fn claude_and_codex_native_keys_are_not_interchangeable() {
    let mut cursor = EventCursor::default();
    let mut prompt = event(CLIAgent::Claude, CLIAgentEventType::PromptSubmit, None);
    prompt.payload.turn_id = Some("codex-only".to_owned());
    cursor.accept(&prompt);
    let mut stop = event(CLIAgent::Claude, CLIAgentEventType::Stop, None);
    stop.payload.turn_id = prompt.payload.turn_id;
    assert_eq!(cursor.accept(&stop), EventDisposition::UnverifiedTerminal);
}

#[test]
fn stale_turn_does_not_consume_current_event_sequence() {
    let mut cursor = EventCursor::default();
    cursor.accept(&event(
        CLIAgent::Codex,
        CLIAgentEventType::PromptSubmit,
        Some("current"),
    ));
    let mut stop = event(CLIAgent::Codex, CLIAgentEventType::Stop, Some("old"));
    stop.payload.sequence = Some(999);
    assert_eq!(cursor.accept(&stop), EventDisposition::Drop);
    stop.payload.turn_id = Some("current".to_owned());
    stop.payload.sequence = Some(3);
    stop.payload.event_id = Some("complete".to_owned());
    assert_eq!(cursor.accept(&stop), EventDisposition::Accept);
    assert_eq!(cursor.accept(&stop), EventDisposition::Drop);
    stop.payload.sequence = Some(2);
    stop.payload.event_id = Some("other".to_owned());
    assert_eq!(cursor.accept(&stop), EventDisposition::Drop);
}

#[test]
fn marked_unverified_terminal_is_distinct_from_an_ordinary_notification() {
    let mut cursor = EventCursor::default();
    cursor.accept(&event(
        CLIAgent::Claude,
        CLIAgentEventType::PromptSubmit,
        Some("current"),
    ));
    let mut notification = event(CLIAgent::Claude, CLIAgentEventType::Notification, None);
    assert_eq!(cursor.accept(&notification), EventDisposition::Accept);
    notification.payload.terminal_unverified = Some(true);
    assert_eq!(
        cursor.accept(&notification),
        EventDisposition::UnverifiedTerminal
    );
    notification.payload.prompt_id = Some("previous".into());
    assert_eq!(cursor.accept(&notification), EventDisposition::Drop);
}

#[test]
fn grok_local_input_waits_for_native_prompt_before_accepting_cancel() {
    let mut cursor = EventCursor::default();
    cursor.accept(&event(
        CLIAgent::Grok,
        CLIAgentEventType::PromptSubmit,
        Some("old"),
    ));
    let mut local = event(CLIAgent::Grok, CLIAgentEventType::PromptSubmit, None);
    local.source = CLIAgentEventSource::LocalRichInput;
    cursor.accept(&local);

    let mut cancel = event(CLIAgent::Grok, CLIAgentEventType::Cancelled, Some("old"));
    cancel.payload.event_id = Some("old-cancel".into());
    assert_eq!(cursor.accept(&cancel), EventDisposition::Drop);
    assert_eq!(
        cursor.accept(&event(
            CLIAgent::Grok,
            CLIAgentEventType::PromptSubmit,
            Some("old")
        )),
        EventDisposition::Drop
    );
    assert_eq!(
        cursor.accept(&event(
            CLIAgent::Grok,
            CLIAgentEventType::PromptSubmit,
            Some("new")
        )),
        EventDisposition::Accept
    );
    assert_eq!(cursor.accept(&cancel), EventDisposition::Drop);

    cancel.payload.prompt_id = Some("new".into());
    cancel.payload.event_id = Some("new-cancel".into());
    assert_eq!(cursor.accept(&cancel), EventDisposition::Accept);
    assert_eq!(cursor.accept(&cancel), EventDisposition::Drop);
}

#[test]
fn grok_unknown_terminal_does_not_advance_active_prompt() {
    let mut cursor = EventCursor::default();
    cursor.accept(&event(
        CLIAgent::Grok,
        CLIAgentEventType::PromptSubmit,
        Some("current"),
    ));
    let mut previous = event(
        CLIAgent::Grok,
        CLIAgentEventType::Cancelled,
        Some("previous"),
    );
    previous.payload.sequence = Some(999);
    previous.payload.event_id = Some("previous-cancel".into());
    assert_eq!(cursor.accept(&previous), EventDisposition::Drop);

    let mut current = event(
        CLIAgent::Grok,
        CLIAgentEventType::Cancelled,
        Some("current"),
    );
    current.payload.sequence = Some(1);
    assert_eq!(cursor.accept(&current), EventDisposition::Accept);
    let mut idle = event(CLIAgent::Grok, CLIAgentEventType::Notification, None);
    idle.payload.terminal_unverified = Some(true);
    assert_eq!(cursor.accept(&idle), EventDisposition::UnverifiedTerminal);
}
