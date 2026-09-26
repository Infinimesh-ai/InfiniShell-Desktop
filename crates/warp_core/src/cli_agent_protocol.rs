use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// Sentinel title that identifies structured CLI-agent events sent via OSC 777.
pub const CLI_AGENT_NOTIFICATION_SENTINEL: &str = "warp://cli-agent";

/// Schema version emitted by the current CLI-agent notification protocol.
pub const CLI_AGENT_PROTOCOL_VERSION: u32 = 1;

/// Environment variable that advertises the host's CLI-agent protocol version.
pub const WARP_CLI_AGENT_PROTOCOL_VERSION_ENV: &str = "WARP_CLI_AGENT_PROTOCOL_VERSION";

/// 当前 shell 的 Unix PTY 路径，供脱离控制终端的 CLI hook 发送状态。
pub const WARP_CLI_AGENT_TTY_ENV: &str = "WARP_CLI_AGENT_TTY";

/// 当前平台宿主的通知 worker；不能将本机路径直接透传到 SSH、容器或 WSL。
pub const WARP_CLI_AGENT_NOTIFY_EXECUTABLE_ENV: &str = "WARP_CLI_AGENT_NOTIFY_EXECUTABLE";

/// Environment variable that identifies the hosting Warp client version.
pub const WARP_CLIENT_VERSION_ENV: &str = "WARP_CLIENT_VERSION";

/// Codex hook 观察到的进程候选，不代表 TUI 身份、权限或输入授权。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexProcessEvidence {
    pub daemon_pid_candidate: u32,
    pub codex_home: String,
    pub tty_path: String,
}

/// Claude hook 提供的进程候选；必须由远端内核、注册表和会话历史共同核验。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeProcessEvidence {
    pub process_id_candidate: u32,
    pub claude_config_directory: String,
    pub tty_path: String,
}

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
    /// 原生权限观察值；缺失或异常不能默认成 default，也不能单独证明会话身份。
    #[serde(default, deserialize_with = "deserialize_permission_mode")]
    pub permission_mode: Option<String>,
    /// 只用于当前监听器的原生会话；服务端必须再核对内核进程和终端归属。
    #[serde(default, deserialize_with = "deserialize_codex_process_evidence")]
    pub codex_process_evidence: Option<CodexProcessEvidence>,
    #[serde(default, deserialize_with = "deserialize_claude_process_evidence")]
    pub claude_process_evidence: Option<ClaudeProcessEvidence>,
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
            permission_mode: None,
            codex_process_evidence: None,
            claude_process_evidence: None,
        }
    }
}

fn deserialize_codex_process_evidence<'de, D>(
    deserializer: D,
) -> Result<Option<CodexProcessEvidence>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // 字段损坏时保留通知并撤销旧候选，避免整条解析失败后沿用旧身份。
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<CodexProcessEvidence>(raw)
        .ok()
        .filter(|evidence| {
            (2..=i32::MAX as u32).contains(&evidence.daemon_pid_candidate)
                && absolute_unix_candidate(&evidence.codex_home, 4096)
                && absolute_unix_candidate(&evidence.tty_path, 256)
                && evidence.tty_path.starts_with("/dev/")
                && evidence.tty_path != "/dev/tty"
        }))
}

fn deserialize_claude_process_evidence<'de, D>(
    deserializer: D,
) -> Result<Option<ClaudeProcessEvidence>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<ClaudeProcessEvidence>(raw)
        .ok()
        .filter(|evidence| {
            (2..=i32::MAX as u32).contains(&evidence.process_id_candidate)
                && absolute_unix_candidate(&evidence.claude_config_directory, 4096)
                && absolute_unix_candidate(&evidence.tty_path, 256)
                && evidence.tty_path.starts_with("/dev/")
                && evidence.tty_path != "/dev/tty"
        }))
}

fn absolute_unix_candidate(value: &str, limit: usize) -> bool {
    // 远端 Unix 路径不能用本地 Windows Path 规则判断，也不能先在本机 canonicalize。
    value.starts_with('/')
        && value.len() > 1
        && value.len() <= limit
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .skip(1)
            .all(|component| !matches!(component, "" | "." | ".."))
}

// 异常字段保留为无证据通知，避免整条解析失败后应用继续保留旧权限证据。
fn deserialize_permission_mode<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(raw
        .as_str()
        .filter(|mode| {
            !mode.is_empty()
                && mode.len() <= 64
                && mode.as_bytes()[0].is_ascii_alphabetic()
                && mode
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
        .map(str::to_owned))
}
