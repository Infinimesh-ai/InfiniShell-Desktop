//! 每个远端终端独立的 Codex 启动凭据；查询永不重放启动或模型输入。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const REMOTE_CODEX_COMMAND: &str = "--infinishell-remote-owned-codex";

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
pub(crate) struct Owner {
    pub ticket_id: Uuid,
    pub manifest_sha256: String,
    pub socket_path: String,
    pub tui_pid: i32,
    pub server_pid: i32,
    pub tty_device: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Reserve { ticket: Ticket, cwd: String },
    Status { ticket: Ticket },
    Cancel { ticket: Ticket },
    Revoke { ticket: Ticket, generation: Uuid },
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
        phase: String,
        owner: Option<Owner>,
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
