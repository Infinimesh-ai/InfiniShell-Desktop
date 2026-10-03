mod assertion;
mod step;

pub use assertion::*;
pub use step::*;

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use crate::ai::cli_agent_runtime::{ApprovalDecision, PermissionPolicy};

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use crate::ai::cli_agent_runtime::task_manager_view::clipboard_integration::{
    advance_v05_grok_scroll, assert_cli_clipboard_draft, click_v05_approval,
    finish_cli_clipboard_evidence, finish_v05_approval_evidence, finish_v05_static_evidence,
    open_cli_clipboard_composer, prepare_v05_approval_fixture, reveal_cli_clipboard_draft,
    reveal_v05_approval, scroll_v05_grok, select_v05_grok_policy, setup_cli_system_clipboard,
    wait_until_cli_clipboard_bootstrapped, wait_v05_fixed_grok_installation,
    write_cli_system_clipboard_image, write_cli_system_clipboard_text,
};
