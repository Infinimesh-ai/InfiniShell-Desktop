use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// Sentinel title that identifies structured CLI-agent events sent via OSC 777.
pub const CLI_AGENT_NOTIFICATION_SENTINEL: &str = "warp://cli-agent";

/// Schema version emitted by the current CLI-agent notification protocol.
pub const CLI_AGENT_PROTOCOL_VERSION: u32 = 1;

/// Environment variable that advertises the host's CLI-agent protocol version.
pub const WARP_CLI_AGENT_PROTOCOL_VERSION_ENV: &str = "WARP_CLI_AGENT_PROTOCOL_VERSION";

/// Environment variable that identifies the hosting Warp client version.
pub const WARP_CLIENT_VERSION_ENV: &str = "WARP_CLIENT_VERSION";

/// Wire representation of a structured CLI-agent notification.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CLIAgentNotification {
    pub v: Option<u32>,
    pub agent: Option<String>,
    pub event: String,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub query: Option<String>,
    pub response: Option<String>,
    pub transcript_path: Option<String>,
    pub summary: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub plugin_version: Option<String>,
    pub error_type: Option<String>,
    /// 可选的上游事件标识和会话内单调序号，供接收方去重。
    pub event_id: Option<String>,
    pub sequence: Option<u64>,
    /// Codex 原生回合标识；不能用本地生成值替代。
    pub turn_id: Option<String>,
    /// Claude 原生输入标识，与 Codex 的回合字段分别传递。
    pub prompt_id: Option<String>,
    /// 插件只能确认发生过结束通知，但缺少原生回合关联；不能视为成功。
    pub terminal_unverified: Option<bool>,
}

impl CLIAgentNotification {
    pub fn new(agent: impl Into<String>, event: impl Into<String>) -> Self {
        Self {
            v: Some(CLI_AGENT_PROTOCOL_VERSION),
            agent: Some(agent.into()),
            event: event.into(),
            session_id: None,
            cwd: None,
            project: None,
            query: None,
            response: None,
            transcript_path: None,
            summary: None,
            tool_name: None,
            tool_input: None,
            plugin_version: None,
            error_type: None,
            event_id: None,
            sequence: None,
            turn_id: None,
            prompt_id: None,
            terminal_unverified: None,
        }
    }
}
