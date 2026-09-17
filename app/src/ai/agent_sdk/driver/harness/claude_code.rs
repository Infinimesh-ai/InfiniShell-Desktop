use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use warp_cli::agent::Harness;
use warpui::ModelHandle;

use super::super::AgentDriverError;
use super::super::terminal::TerminalDriver;
use super::{HarnessRunner, ThirdPartyHarness};
use crate::ai::agent_events::AgentEventStreamClient;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::terminal::CLIAgent;

// 保留父子消息桥的独立回归；旧独立 driver 不再创建或启动该桥。
#[cfg(test)]
mod parent_bridge;

#[cfg(test)]
use parent_bridge::{
    MESSAGE_BRIDGE_CONTEXT_PREAMBLE, MessageBridgeHookOutput, MessageBridgeMessageRecord,
    acknowledge_parent_bridge_hook_output, ensure_parent_bridge_state_dir,
    parent_bridge_char_count, parent_bridge_hook_output_ack_file, parent_bridge_hook_output_file,
    parent_bridge_root, parent_bridge_staged_message_path, parent_bridge_surfaced_message_path,
    prepare_parent_bridge_hook_output, render_parent_bridge_message_block,
    stage_parent_bridge_message,
};

#[cfg(test)]
use super::super::OZ_MESSAGE_LISTENER_STATE_ROOT_ENV;

pub(crate) struct ClaudeHarness;

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl ThirdPartyHarness for ClaudeHarness {
    fn harness(&self) -> Harness {
        Harness::Claude
    }

    fn cli_agent(&self) -> CLIAgent {
        CLIAgent::Claude
    }

    fn install_docs_url(&self) -> Option<&'static str> {
        Some("https://code.claude.com/docs/en/quickstart")
    }

    fn auth_check_command(&self) -> Option<String> {
        Some("claude auth status".to_string())
    }

    fn runtime_error_patterns(&self) -> &'static [&'static str] {
        &[
            // Out-of-credits / billing.
            "Credit balance too low",
            // Plan/usage limits emitted as `You've hit your <kind> limit`.
            // We match on the common prefix so the variants (session,
            // weekly, Opus, etc.) all hit.
            "You've hit your",
            // Invalid or malformed API key.
            "Invalid API key",
            "This organization has been disabled",
            "belongs to a disabled organization",
            // OAuth / login state.
            "Not logged in",
            "OAuth token revoked",
            "OAuth token has expired",
            // Routines disabled by org policy.
            "Routines are disabled by your organization's policy",
            // Generic upstream API failures Claude Code surfaces verbatim.
            "API Error: Request rejected (429)",
            "authentication_error",
        ]
    }

    // 固定 trait 签名保留参数位；拒绝路径不读取输入、不创建文件或启动进程。
    fn build_runner(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: &Path,
        _: Option<AmbientAgentTaskId>,
        _: Arc<dyn AgentEventStreamClient>,
        _: ModelHandle<TerminalDriver>,
    ) -> Result<Box<dyn HarnessRunner>, AgentDriverError> {
        // 即使绕过 harness_kind 直接调用 trait，也不能进入缺少审批回路的旧 TUI。
        Err(AgentDriverError::HarnessSetupFailed {
            harness: self.harness().to_string(),
            reason: crate::t!(
                "cli-agent-standalone-harness-unavailable",
                cli = self.cli_agent().display_name()
            ),
        })
    }
}

#[cfg(test)]
#[path = "claude_code_tests.rs"]
mod tests;
