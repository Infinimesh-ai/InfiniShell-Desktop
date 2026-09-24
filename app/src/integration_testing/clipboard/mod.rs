mod assertion;
mod step;

pub use assertion::*;
pub use step::*;

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use crate::ai::cli_agent_runtime::task_manager_view::clipboard_integration::{
    assert_cli_clipboard_draft, finish_cli_clipboard_evidence, open_cli_clipboard_composer,
    setup_cli_system_clipboard, wait_until_cli_clipboard_bootstrapped,
    write_cli_system_clipboard_image, write_cli_system_clipboard_text,
};
