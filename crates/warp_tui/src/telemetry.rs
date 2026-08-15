//! TUI 本地事件元数据。
//!
//! Zap 不发送遥测；这些类型只保留调用点的结构和单测所需的低基数载荷。
use std::ffi::{OsStr, OsString};

#[cfg(test)]
use serde_json::{Value, json};

const MAX_TERM_PROGRAM_CHARS: usize = 64;

#[derive(Clone, Copy, Debug)]
enum TuiHostMultiplexer {
    None,
    Tmux,
    Screen,
    Zellij,
}

impl TuiHostMultiplexer {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Tmux => "tmux",
            Self::Screen => "screen",
            Self::Zellij => "zellij",
        }
    }
}

#[derive(Debug)]
pub(crate) struct TuiStartupTelemetryEvent {
    term_program: Option<String>,
    multiplexer: TuiHostMultiplexer,
}

impl TuiStartupTelemetryEvent {
    pub(crate) fn from_environment() -> Self {
        Self {
            term_program: sanitize_term_program(std::env::var_os("TERM_PROGRAM")),
            multiplexer: detect_multiplexer(
                std::env::var_os("TMUX").as_deref(),
                std::env::var_os("STY").as_deref(),
                std::env::var_os("ZELLIJ").as_deref(),
                std::env::var_os("ZELLIJ_SESSION_NAME").as_deref(),
            ),
        }
    }

    #[cfg(test)]
    fn payload(&self) -> Option<Value> {
        Some(json!({
            "term_program": self.term_program,
            "multiplexer": self.multiplexer.as_str(),
        }))
    }
}

fn sanitize_term_program(value: Option<OsString>) -> Option<String> {
    let value = value?.into_string().ok()?;
    let sanitized = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_TERM_PROGRAM_CHARS)
        .collect::<String>();
    (!sanitized.is_empty()).then_some(sanitized)
}

fn detect_multiplexer(
    tmux: Option<&OsStr>,
    screen: Option<&OsStr>,
    zellij: Option<&OsStr>,
    zellij_session_name: Option<&OsStr>,
) -> TuiHostMultiplexer {
    let is_present = |value: Option<&OsStr>| value.is_some_and(|value| !value.is_empty());
    if is_present(tmux) {
        TuiHostMultiplexer::Tmux
    } else if is_present(screen) {
        TuiHostMultiplexer::Screen
    } else if is_present(zellij) || is_present(zellij_session_name) {
        TuiHostMultiplexer::Zellij
    } else {
        TuiHostMultiplexer::None
    }
}

#[derive(Debug)]
pub(crate) enum TuiConversationMenuTelemetryEvent {
    Opened,
    ItemSelected,
}

impl TuiConversationMenuTelemetryEvent {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        match self {
            Self::Opened => "TUI.ConversationMenu.Opened",
            Self::ItemSelected => "TUI.ConversationMenu.ItemSelected",
        }
    }

    #[cfg(test)]
    fn payload(&self) -> Option<Value> {
        None
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiConversationRestoreTelemetryState {
    Started,
    Succeeded,
    Failed,
    Cancelled,
}

impl TuiConversationRestoreTelemetryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TuiConversationRestoreTelemetryTarget {
    Local,
    Server,
}

impl TuiConversationRestoreTelemetryTarget {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Server => "server",
        }
    }
}

#[derive(Debug)]
pub(crate) struct TuiConversationRestoreTelemetryEvent {
    pub state: TuiConversationRestoreTelemetryState,
    pub target: TuiConversationRestoreTelemetryTarget,
}

impl TuiConversationRestoreTelemetryEvent {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        "TUI.ConversationRestore"
    }

    #[cfg(test)]
    fn payload(&self) -> Option<Value> {
        Some(json!({
            "state": self.state.as_str(),
            "target": self.target.as_str(),
        }))
    }

    #[cfg(test)]
    fn contains_ugc(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub(crate) enum TuiAutoupdateTelemetryEvent {
    CheckCompleted {
        outcome: &'static str,
        version: Option<String>,
    },
    CheckFailed {
        error: String,
    },
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
