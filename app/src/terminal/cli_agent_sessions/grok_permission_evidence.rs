//! 原生权限字段只补充同一活动 SessionStart 的证据，不认证进程、不决定审批或输入就绪。

use super::event::{CLIAgentEvent, CLIAgentEventSource, CLIAgentEventType};
use crate::terminal::CLIAgent;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrokPermissionObservation {
    pub session_id: String,
    pub cwd: String,
    pub session_start_event_id: String,
    pub mode: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum GrokPermissionEvidence {
    #[default]
    Unobserved,
    Observed(GrokPermissionObservation),
    /// 同一监听期间不可恢复；只有新活动会话才能重新观察原生 SessionStart。
    Invalidated,
}

impl GrokPermissionEvidence {
    pub(super) fn invalidate(&mut self) -> bool {
        let changed = *self != Self::Invalidated;
        *self = Self::Invalidated;
        changed
    }

    /// 只在 EventCursor 接受事件后调用，丢弃的旧回合不能改变证据。
    pub(super) fn observe(
        &mut self,
        event: &CLIAgentEvent,
        active_session_id: Option<&str>,
        active_cwd: Option<&str>,
    ) -> bool {
        if event.agent != CLIAgent::Grok {
            return false;
        }
        if event.source == CLIAgentEventSource::LocalRichInput {
            // 尚未观察 SessionStart 就已提交输入，后到的开始通知不能补授本次证据。
            return matches!(self, Self::Unobserved)
                && event.event == CLIAgentEventType::PromptSubmit
                && self.invalidate();
        }
        if event.source != CLIAgentEventSource::RichPlugin || *self == Self::Invalidated {
            return false;
        }
        let session_id = event
            .session_id
            .as_deref()
            .filter(|value| !value.is_empty());
        let cwd = event.cwd.as_deref().filter(|value| !value.is_empty());
        let mode = event
            .payload
            .permission_mode
            .as_deref()
            .filter(|value| !value.is_empty());
        if session_id.is_none()
            || cwd.is_none()
            || mode.is_none()
            || session_id != active_session_id
            || cwd != active_cwd
        {
            return self.invalidate();
        }
        match self {
            Self::Unobserved => {
                let Some(event_id) = event
                    .payload
                    .event_id
                    .as_deref()
                    .filter(|value| !value.is_empty())
                else {
                    return self.invalidate();
                };
                if event.event != CLIAgentEventType::SessionStart {
                    return self.invalidate();
                }
                *self = Self::Observed(GrokPermissionObservation {
                    session_id: session_id.expect("已校验会话标识").to_owned(),
                    cwd: cwd.expect("已校验会话目录").to_owned(),
                    session_start_event_id: event_id.to_owned(),
                    mode: mode.expect("已校验原生权限字段").to_owned(),
                });
                true
            }
            Self::Observed(observed) => {
                // 重投开始事件已由 cursor 丢弃；新的开始事件不能替代原会话权限证据。
                if event.event == CLIAgentEventType::SessionStart
                    || session_id != Some(observed.session_id.as_str())
                    || cwd != Some(observed.cwd.as_str())
                    || mode != Some(observed.mode.as_str())
                {
                    self.invalidate()
                } else {
                    false
                }
            }
            Self::Invalidated => false,
        }
    }
}

#[cfg(test)]
#[path = "grok_permission_evidence_tests.rs"]
mod tests;
