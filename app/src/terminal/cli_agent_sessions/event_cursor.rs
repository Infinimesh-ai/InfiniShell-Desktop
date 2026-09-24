use std::collections::HashSet;

use super::event::{CLIAgentEvent, CLIAgentEventSource, CLIAgentEventType};
use crate::terminal::CLIAgent;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum EventDisposition {
    Accept,
    Drop,
    UnverifiedTerminal,
    /// 原生通知只证明当前会话需要操作，不提供回合状态证据。
    SessionAttention,
}

/// 标识仅在当前 pane 的原生会话内有效；不能从转录末尾推断当前回合。
#[derive(Default)]
pub(super) struct EventCursor {
    seen_event_ids: HashSet<String>,
    sequence: Option<u64>,
    seen_turns: HashSet<String>,
    active_turn: Option<String>,
    awaiting_native_prompt: bool,
}

impl EventCursor {
    pub(super) fn accept(&mut self, event: &CLIAgentEvent) -> EventDisposition {
        if event
            .payload
            .sequence
            .is_some_and(|sequence| self.sequence.is_some_and(|previous| sequence <= previous))
            || event
                .payload
                .event_id
                .as_ref()
                .is_some_and(|id| self.seen_event_ids.contains(id))
        {
            return EventDisposition::Drop;
        }
        let disposition = self.correlate(event);
        if disposition != EventDisposition::Drop {
            if let Some(sequence) = event.payload.sequence {
                self.sequence = Some(sequence);
            }
            if let Some(id) = &event.payload.event_id {
                self.seen_event_ids.insert(id.clone());
            }
        }
        disposition
    }

    fn correlate(&mut self, event: &CLIAgentEvent) -> EventDisposition {
        if event.source == CLIAgentEventSource::LocalRichInput {
            if event.event == CLIAgentEventType::PromptSubmit {
                self.awaiting_native_prompt = true;
            }
            return EventDisposition::Accept;
        }
        if event.source != CLIAgentEventSource::RichPlugin {
            return EventDisposition::Accept;
        }
        let turn_id = match event.agent {
            CLIAgent::Codex => event.payload.turn_id.as_deref(),
            CLIAgent::Claude | CLIAgent::Grok => event.payload.prompt_id.as_deref(),
            _ => return EventDisposition::Accept,
        }
        .filter(|id| !id.is_empty());
        if event.event == CLIAgentEventType::PromptSubmit {
            if let Some(turn_id) = turn_id {
                if !self.seen_turns.insert(turn_id.to_owned()) {
                    return EventDisposition::Drop;
                }
            }
            self.active_turn = turn_id.map(str::to_owned);
            self.awaiting_native_prompt = false;
            return EventDisposition::Accept;
        }
        let unverified = event.event == CLIAgentEventType::Notification
            && event.payload.terminal_unverified == Some(true);
        let terminal = unverified
            || matches!(
                event.event,
                CLIAgentEventType::Stop
                    | CLIAgentEventType::StopFailure
                    | CLIAgentEventType::Cancelled
            );
        let scoped = unverified
            || match event.event {
                CLIAgentEventType::Stop
                | CLIAgentEventType::StopFailure
                | CLIAgentEventType::Cancelled
                | CLIAgentEventType::ToolComplete
                | CLIAgentEventType::PermissionRequest
                | CLIAgentEventType::PermissionReplied
                | CLIAgentEventType::QuestionAsked => true,
                CLIAgentEventType::SessionStart
                | CLIAgentEventType::PromptSubmit
                | CLIAgentEventType::IdlePrompt
                | CLIAgentEventType::Notification
                | CLIAgentEventType::Unknown(_) => false,
            };
        if !scoped {
            return EventDisposition::Accept;
        }
        // 本地已开始提交下一轮时，旧轮仍可能迟到；必须等待新的原生输入标识。
        if self.awaiting_native_prompt {
            return EventDisposition::Drop;
        }
        // Grok 1.0.41 的 permission_prompt 没有 promptId；不能补造回合标识。
        if event.agent == CLIAgent::Grok
            && event.event == CLIAgentEventType::PermissionRequest
            && turn_id.is_none()
            && self.active_turn.is_some()
        {
            return EventDisposition::SessionAttention;
        }
        match (turn_id, self.active_turn.as_deref()) {
            (Some(incoming), Some(active)) if incoming == active => {
                if unverified {
                    EventDisposition::UnverifiedTerminal
                } else {
                    EventDisposition::Accept
                }
            }
            (Some(_), Some(_)) => EventDisposition::Drop,
            (None, Some(_)) | (Some(_), None) | (None, None) => {
                if terminal {
                    EventDisposition::UnverifiedTerminal
                } else {
                    EventDisposition::Drop
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "event_cursor_tests.rs"]
mod tests;
