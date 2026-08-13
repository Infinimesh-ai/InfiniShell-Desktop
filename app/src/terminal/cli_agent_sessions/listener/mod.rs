use warpui::{EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::{CLIAgentEvent, CLIAgentSession, CLIAgentSessionsModel};
use crate::features::FeatureFlag;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::event::{
    CLIAgentEventPayload, CLIAgentEventSource, CLIAgentEventType, parse_event,
};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};

/// Per-agent handler that filters and transforms parsed CLI agent events.
/// Each CLI agent can have a different implementation depending on which events
/// it cares about.
trait CLIAgentSessionHandler {
    /// Attempt to parse a raw `PluggableNotification` into a typed event.
    /// The default implementation delegates to the structured JSON parser
    /// (`parse_event`); agents with non-JSON notification formats (e.g. Codex
    /// OSC 9 plain text) should override this.
    ///
    /// `plugin_already_active` is true when the session has already received a
    /// structured OSC 777 notification; Codex uses it to drop OSC 9 fallback
    /// once the rich plugin is active. Other handlers ignore it.
    fn try_parse(
        &mut self,
        title: Option<&str>,
        body: &str,
        plugin_already_active: bool,
    ) -> Option<CLIAgentEvent> {
        let _ = plugin_already_active;
        parse_event(title, body)
    }

    /// Decide whether a parsed event should be forwarded to the sessions model.
    /// Returns the event (possibly transformed) if it should be processed.
    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent>;

    /// Whether this handler provides meaningful, fine-grained status
    /// (e.g. in-progress / blocked / success) that should be shown in the UI.
    /// Handlers backed by the structured plugin protocol report rich status;
    /// handlers that only forward opaque OS notifications do not.
    fn supports_rich_status(&self) -> bool {
        true
    }
}

/// Whether the listener for the given agent provides rich status.
/// Returns `false` for agents without a handler or whose handler opts out.
pub fn agent_supports_rich_status(agent: &CLIAgent) -> bool {
    create_handler(agent).is_some_and(|h| h.supports_rich_status())
}

/// Returns whether this concrete session has enough event context to render
/// fine-grained status in UI surfaces.
pub fn session_supports_rich_status(session: &CLIAgentSession) -> bool {
    if !agent_supports_rich_status(&session.agent) {
        return false;
    }

    // DeepSeek has two listener paths:
    // - legacy OSC 9 completion notifications, registered from command detection,
    //   with no session id or lifecycle events;
    // - structured OSC 777 hooks, which include the DeepSeek hook session id.
    // Only the latter can drive rich status accurately.
    if matches!(session.agent, CLIAgent::DeepSeek) && session.session_context.session_id.is_none() {
        return false;
    }

    true
}

/// Returns `true` if the given CLI agent has a supported session handler.
pub fn is_agent_supported(agent: &CLIAgent) -> bool {
    matches!(
        agent,
        CLIAgent::Claude
            | CLIAgent::OpenCode
            | CLIAgent::Codex
            | CLIAgent::Gemini
            | CLIAgent::Auggie
            | CLIAgent::Droid
            | CLIAgent::Pi
            | CLIAgent::OhMyPi
            | CLIAgent::DeepSeek
            | CLIAgent::Antigravity
            | CLIAgent::WarpTui
    )
}

/// Creates the appropriate handler for the given CLI agent.
fn create_handler(agent: &CLIAgent) -> Option<Box<dyn CLIAgentSessionHandler>> {
    match agent {
        // Auggie and Pi are supported via community-maintained plugins
        // (https://github.com/augmentmoogi/auggie-warp,
        // https://github.com/badlogic/pi-mono). OhMyPi emits these structured
        // OSC 777 events natively. Droid can be supported by user-configured
        // hooks or future integrations that emit the same events. We don't ship
        // install flows for these agents here — we just listen.
        // WarpTui emits OSC 777 events directly (no external plugin needed).
        CLIAgent::Claude
        | CLIAgent::OpenCode
        | CLIAgent::Gemini
        | CLIAgent::Auggie
        | CLIAgent::Droid
        | CLIAgent::Pi
        | CLIAgent::OhMyPi
        | CLIAgent::Antigravity
        | CLIAgent::WarpTui => Some(Box::new(DefaultSessionListener)),
        CLIAgent::Codex => Some(Box::new(CodexSessionHandler)),
        CLIAgent::DeepSeek => Some(Box::new(DeepSeekSessionHandler)),
        CLIAgent::Hermes
        | CLIAgent::Amp
        | CLIAgent::Copilot
        | CLIAgent::CursorCli
        | CLIAgent::Goose
        | CLIAgent::Vibe
        | CLIAgent::Omp
        | CLIAgent::Unknown => None,
    }
}

/// Default handler shared by agents whose events need no special filtering
/// beyond skipping the initial `SessionStart`.
struct DefaultSessionListener;

impl CLIAgentSessionHandler for DefaultSessionListener {
    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent> {
        // Skip session_start events (handled during listener construction)
        if event.event == CLIAgentEventType::SessionStart {
            return None;
        }

        Some(event)
    }
}

/// Codex-specific handler that supports both native OSC 9 fallback and structured plugin events.
///
/// Codex sends notifications via OSC 9 (`\x1b]9;message\x07`) with
/// human-readable text. Since there's no way to distinguish notification types from the raw text,
/// OSC 9 fallback notifications are treated as `Stop` (success).
struct CodexSessionHandler;

impl CodexSessionHandler {
    /// Parse a plain-text OSC 9 notification body into a `CLIAgentEvent`.
    /// Returns `None` only for empty bodies.
    fn parse_osc9_text(body: &str) -> Option<CLIAgentEvent> {
        let body = body.trim();
        if body.is_empty() {
            return None;
        }

        Some(CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Codex,
            event: CLIAgentEventType::Stop,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload {
                query: Some(body.to_owned()),
                ..Default::default()
            },
            source: CLIAgentEventSource::CodexOsc9Fallback,
        })
    }
}

impl CLIAgentSessionHandler for CodexSessionHandler {
    /// Before Codex enabled support for hooks, we relied on OSC 9 to trigger notifications in Warp.
    /// Here, we try to parse an OSC 777 event if we can, and remember when we've seen one.
    /// This lets us ignore OSC 9 notifications if we are working with a client that is using
    /// the new plugin, but keeps them intact for legacy clients.
    fn try_parse(
        &mut self,
        title: Option<&str>,
        body: &str,
        plugin_already_active: bool,
    ) -> Option<CLIAgentEvent> {
        if let Some(event) = parse_event(title, body) {
            if event.agent == CLIAgent::Codex {
                if !FeatureFlag::CodexPlugin.is_enabled() {
                    return None;
                }
                return Some(event);
            }
            return None;
        }
        // OSC 9 notifications have no title. Skip OSC 9 once the rich plugin is
        // active, otherwise we'd process both OSC 777 and OSC 9 notifications.
        if title.is_some() || plugin_already_active {
            return None;
        }
        Self::parse_osc9_text(body)
    }

    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent> {
        Some(event)
    }

    /// Zap:我方原先对 Codex 一律返回 `false` —— 当时 Codex 只有 OSC 9 这种不带
    /// 结构的通知。上游为 Codex 加了 OSC 777 插件通道后,富状态是否可信取决于
    /// `CodexPlugin` 开关(见上面的 `try_parse`:开关关闭时 OSC 777 事件被整体丢弃)。
    fn supports_rich_status(&self) -> bool {
        FeatureFlag::CodexPlugin.is_enabled()
    }
}

/// DeepSeek-TUI handler: listens for structured OSC 777 events and legacy
/// OSC 9 plain-text notifications.
/// DeepSeek-TUI emits `\x1b]9;deepseek: turn complete\x07` (optionally with
/// elapsed time and cost) when a turn finishes. Those legacy notifications are
/// treated as `Stop` events. Rich status is only available when DeepSeek hooks
/// emit structured OSC 777 events with a session id.
struct DeepSeekSessionHandler;

impl DeepSeekSessionHandler {
    fn notification_title_from_body(body: &str) -> Option<String> {
        let title = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter(|line| !line.starts_with("deepseek: turn complete"))
            .collect::<Vec<_>>()
            .join("\n");

        if title.is_empty() { None } else { Some(title) }
    }
}

impl CLIAgentSessionHandler for DeepSeekSessionHandler {
    /// DeepSeek-TUI uses OSC 9 with no title (same channel as Codex).
    fn try_parse(
        &mut self,
        title: Option<&str>,
        body: &str,
        _plugin_already_active: bool,
    ) -> Option<CLIAgentEvent> {
        // Future-proof: try structured JSON first in case a plugin is added later.
        if let Some(parsed) = parse_event(title, body) {
            return Some(parsed);
        }
        // OSC 9 notifications have no title.
        if title.is_some() {
            return None;
        }
        let body = body.trim();
        if body.is_empty() {
            return None;
        }
        Some(CLIAgentEvent {
            v: 1,
            agent: CLIAgent::DeepSeek,
            event: CLIAgentEventType::Stop,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload {
                query: Self::notification_title_from_body(body),
                response: Some(body.to_owned()),
                ..Default::default()
            },
            // DeepSeek-TUI 与 Codex 共用 OSC 9 无标题通道,归为同一个 fallback 来源。
            source: CLIAgentEventSource::CodexOsc9Fallback,
        })
    }

    fn handle_event(&mut self, event: CLIAgentEvent) -> Option<CLIAgentEvent> {
        Some(event)
    }

    fn supports_rich_status(&self) -> bool {
        true
    }
}

/// Per-agent listener that subscribes to PTY events and forwards them to the
/// sessions model. Stored on [`super::CLIAgentSession`] so its lifetime is
/// tied to the session; dropping the handle cleans up the subscription.
pub struct CLIAgentSessionListener {
    terminal_view_id: EntityId,
    inner: Box<dyn CLIAgentSessionHandler>,
}

impl warpui::Entity for CLIAgentSessionListener {
    type Event = ();
}

impl CLIAgentSessionListener {
    pub fn new(
        terminal_view_id: EntityId,
        agent: CLIAgent,
        model_event_dispatcher: &ModelHandle<ModelEventDispatcher>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let handler =
            create_handler(&agent).expect("is_agent_supported must be checked before calling new");

        // Subscribe to subsequent OSC events from this terminal's PTY.
        // Parsing is delegated to the handler's `try_parse`; the handler's
        // `handle_event` then filters/transforms the result.
        ctx.subscribe_to_model(model_event_dispatcher, move |me, _, event, ctx| {
            if let ModelEvent::PluggableNotification { title, body } = event {
                let view_id = me.terminal_view_id;
                let plugin_already_active = CLIAgentSessionsModel::as_ref(ctx)
                    .session(view_id)
                    .is_some_and(|session| session.received_rich_notification);
                let Some(parsed) =
                    me.inner
                        .try_parse(title.as_deref(), body, plugin_already_active)
                else {
                    return;
                };
                if let Some(event) = me.inner.handle_event(parsed) {
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
                        sessions_model.update_from_event(view_id, &event, ctx);
                    });
                }
            }
        });

        Self {
            terminal_view_id,
            inner: handler,
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
mod zap_tests {
    use super::*;
    use crate::terminal::cli_agent_sessions::event::CLIAgentEventType;

    #[test]
    fn codex_parses_any_text_as_stop() {
        let event = CodexSessionHandler::parse_osc9_text("Agent turn complete").unwrap();
        assert_eq!(event.event, CLIAgentEventType::Stop);
        assert_eq!(event.agent, CLIAgent::Codex);
        assert_eq!(event.payload.query.as_deref(), Some("Agent turn complete"));
    }

    #[test]
    fn codex_body_becomes_query() {
        let event = CodexSessionHandler::parse_osc9_text(
            "I've updated the README with the new instructions.",
        )
        .unwrap();
        assert_eq!(event.event, CLIAgentEventType::Stop);
        assert_eq!(
            event.payload.query.as_deref(),
            Some("I've updated the README with the new instructions.")
        );
    }

    #[test]
    fn codex_approval_text_still_becomes_stop() {
        let event =
            CodexSessionHandler::parse_osc9_text("Approval requested: rm -rf /tmp/foo").unwrap();
        assert_eq!(event.event, CLIAgentEventType::Stop);
        assert_eq!(
            event.payload.query.as_deref(),
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
        assert_eq!(event.event, CLIAgentEventType::Stop);
    }

    #[test]
    fn auggie_is_supported() {
        assert!(is_agent_supported(&CLIAgent::Auggie));
    }

    #[test]
    fn auggie_uses_default_handler_with_rich_status() {
        assert!(agent_supports_rich_status(&CLIAgent::Auggie));
    }

    #[test]
    fn auggie_default_handler_skips_session_start() {
        let mut handler = DefaultSessionListener;
        let event = CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Auggie,
            event: CLIAgentEventType::SessionStart,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload::default(),
            source: CLIAgentEventSource::RichPlugin,
        };
        assert!(handler.handle_event(event).is_none());
    }

    #[test]
    fn auggie_default_handler_forwards_stop() {
        let mut handler = DefaultSessionListener;
        let event = CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Auggie,
            event: CLIAgentEventType::Stop,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload::default(),
            source: CLIAgentEventSource::RichPlugin,
        };
        assert!(handler.handle_event(event).is_some());
    }

    #[test]
    fn pi_is_supported() {
        assert!(is_agent_supported(&CLIAgent::Pi));
    }

    #[test]
    fn pi_uses_default_handler_with_rich_status() {
        assert!(agent_supports_rich_status(&CLIAgent::Pi));
    }

    #[test]
    fn pi_default_handler_skips_session_start() {
        let mut handler = DefaultSessionListener;
        let event = CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Pi,
            event: CLIAgentEventType::SessionStart,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload::default(),
            source: CLIAgentEventSource::RichPlugin,
        };
        assert!(handler.handle_event(event).is_none());
    }

    #[test]
    fn pi_default_handler_forwards_stop() {
        let mut handler = DefaultSessionListener;
        let event = CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Pi,
            event: CLIAgentEventType::Stop,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload::default(),
            source: CLIAgentEventSource::RichPlugin,
        };
        assert!(handler.handle_event(event).is_some());
    }

    #[test]
    fn antigravity_is_supported() {
        assert!(is_agent_supported(&CLIAgent::Antigravity));
    }

    #[test]
    fn antigravity_uses_default_handler_with_rich_status() {
        assert!(agent_supports_rich_status(&CLIAgent::Antigravity));
    }

    #[test]
    fn antigravity_default_handler_skips_session_start() {
        let mut handler = DefaultSessionListener;
        let event = CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Antigravity,
            event: CLIAgentEventType::SessionStart,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload::default(),
            source: CLIAgentEventSource::RichPlugin,
        };
        assert!(handler.handle_event(event).is_none());
    }

    #[test]
    fn antigravity_default_handler_forwards_stop() {
        let mut handler = DefaultSessionListener;
        let event = CLIAgentEvent {
            v: 1,
            agent: CLIAgent::Antigravity,
            event: CLIAgentEventType::Stop,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload::default(),
            source: CLIAgentEventSource::RichPlugin,
        };
        assert!(handler.handle_event(event).is_some());
    }

    #[test]
    fn deepseek_handler_supports_structured_rich_status() {
        assert!(agent_supports_rich_status(&CLIAgent::DeepSeek));
    }

    #[test]
    fn deepseek_osc9_completion_does_not_claim_prompt_text() {
        let mut handler = DeepSeekSessionHandler;
        let event = handler
            .try_parse(None, "deepseek: turn complete", false)
            .expect("DeepSeek OSC 9 body should parse");

        assert_eq!(event.event, CLIAgentEventType::Stop);
        assert_eq!(event.payload.query, None);
        assert_eq!(
            event.payload.response.as_deref(),
            Some("deepseek: turn complete")
        );
    }

    #[test]
    fn deepseek_osc9_response_text_becomes_notification_title() {
        let mut handler = DeepSeekSessionHandler;
        let event = handler
            .try_parse(
                None,
                "最新回复内容\ndeepseek: turn complete (1m 15s, $0.01)",
                false,
            )
            .expect("DeepSeek OSC 9 body should parse");

        assert_eq!(event.event, CLIAgentEventType::Stop);
        assert_eq!(event.payload.query.as_deref(), Some("最新回复内容"));
        assert_eq!(
            event.payload.response.as_deref(),
            Some("最新回复内容\ndeepseek: turn complete (1m 15s, $0.01)")
        );
    }

    #[test]
    fn deepseek_osc9_plain_response_text_becomes_notification_title() {
        let mut handler = DeepSeekSessionHandler;
        let event = handler
            .try_parse(None, "最新回复内容", false)
            .expect("DeepSeek OSC 9 body should parse");

        assert_eq!(event.event, CLIAgentEventType::Stop);
        assert_eq!(event.payload.query.as_deref(), Some("最新回复内容"));
        assert_eq!(event.payload.response.as_deref(), Some("最新回复内容"));
    }

    #[test]
    fn deepseek_legacy_osc9_session_is_not_rich_status() {
        let session = CLIAgentSession {
            agent: CLIAgent::DeepSeek,
            status: super::super::CLIAgentSessionStatus::InProgress,
            session_context: super::super::CLIAgentSessionContext::default(),
            input_state: super::super::CLIAgentInputState::Closed,
            should_auto_toggle_input: false,
            listener: None,
            remote_host: None,
            plugin_version: None,
            draft_text: None,
            custom_command_prefix: None,
            received_rich_notification: false,
        };

        assert!(!session_supports_rich_status(&session));
    }

    #[test]
    fn deepseek_structured_session_is_rich_status() {
        let session = CLIAgentSession {
            agent: CLIAgent::DeepSeek,
            status: super::super::CLIAgentSessionStatus::InProgress,
            session_context: super::super::CLIAgentSessionContext {
                session_id: Some("sess_1234".to_owned()),
                ..Default::default()
            },
            input_state: super::super::CLIAgentInputState::Closed,
            should_auto_toggle_input: false,
            listener: None,
            remote_host: None,
            plugin_version: None,
            draft_text: None,
            custom_command_prefix: None,
            received_rich_notification: false,
        };

        assert!(session_supports_rich_status(&session));
    }
}
