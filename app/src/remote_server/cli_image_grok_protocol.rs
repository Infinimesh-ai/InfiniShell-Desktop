//! 远端专属 Grok 的有界协议；启动、关联和原生输入分别领取，查询永不重放。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const REMOTE_GROK_COMMAND: &str = "--infinishell-remote-owned-grok";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scope {
    pub host: String,
    pub terminal_session: u64,
    pub terminal_epoch: Uuid,
    pub generation: Uuid,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ticket {
    pub id: Uuid,
    pub key: Uuid,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Observation {
    pub native_session: Uuid,
    pub cwd: String,
    pub event_id: String,
    pub permission_mode: String,
    pub permission_revision: Uuid,
    pub binding_id: Uuid,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Png {
    pub data: String,
    pub sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub message_id: Uuid,
    pub text: String,
    pub images: Vec<Png>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Reserve {
        ticket: Ticket,
        cwd: String,
    },
    Status {
        ticket: Ticket,
    },
    Cancel {
        ticket: Ticket,
    },
    Submit {
        ticket: Ticket,
        observation: Observation,
        input: Input,
    },
    InputStatus {
        ticket: Ticket,
        message_id: Uuid,
    },
    Revoke {
        ticket: Ticket,
        generation: Uuid,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub version: u32,
    pub revision: u64,
    pub scope: Scope,
    pub action: Action,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Reply {
    Reserved {
        ticket: Ticket,
        argv: Vec<String>,
    },
    Launch {
        ticket: Ticket,
        native_session: Option<Uuid>,
        manifest_sha256: Option<String>,
        phase: String,
    },
    Input {
        ticket: Ticket,
        message_id: Uuid,
        subject_sha256: String,
        state: String,
        native_prompt_id: Option<Uuid>,
        native_ack_sha256: Option<String>,
    },
    Revoked {
        ticket: Ticket,
    },
    Failed {
        code: String,
    },
}

impl Request {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ()> {
        if bytes.is_empty() || bytes.len() > MAX_BODY_BYTES {
            return Err(());
        }
        let request: Self = serde_json::from_slice(bytes).map_err(|_| ())?;
        if request.version != 1
            || request.revision == 0
            || request.scope.host.is_empty()
            || request.scope.host.len() > 512
            || request.scope.terminal_epoch.is_nil()
            || request.scope.generation.is_nil()
        {
            return Err(());
        }
        Ok(request)
    }
}

pub(crate) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ()> {
    let bytes = serde_json::to_vec(value).map_err(|_| ())?;
    (bytes.len() <= MAX_BODY_BYTES).then_some(bytes).ok_or(())
}
