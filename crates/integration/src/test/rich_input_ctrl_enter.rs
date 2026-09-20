//! Integration tests for the Rich Input Ctrl+Enter submit toggle (issue #11588).
//!
//! These tests drive the full keystroke-dispatch path and complement the unit
//! tests in `app/src/terminal/input_tests.rs`.

use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

use settings::Setting as _;
use warp::features::FeatureFlag;
use warp::integration_testing::input::{
    open_cli_agent_rich_input, rich_input_buffer_contains_newline,
    rich_input_buffer_does_not_contain_newline, rich_input_buffer_text_is_empty,
};
use warp::integration_testing::step::{
    assert_no_pending_model_events, new_step_with_default_assertions,
};
use warp::integration_testing::terminal::{
    execute_long_running_command, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warp::settings::SubmitRichInputOnCtrlEnter;
use warpui_core::async_assert;

use super::new_builder;
use crate::Builder;

fn new_step_while_cli_agent_is_running(name: &str) -> warpui_core::integration::TestStep {
    warpui_core::integration::TestStep::new(name)
        .add_named_assertion("no pending model events", assert_no_pending_model_events())
        .add_named_assertion(
            "active CLI agent accepts multiline paste",
            |app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |view, _ctx| {
                    let mut model = view.model.lock();
                    let command = model
                        .block_list()
                        .active_block()
                        .command_with_secrets_obfuscated(false);
                    async_assert!(
                        model.needs_bracketed_paste(),
                        "Fake Claude CLI did not enable bracketed paste for command {command:?}"
                    )
                })
            },
        )
}

// ---------------------------------------------------------------------------
// Setting = true: end-to-end wiring guard
// ---------------------------------------------------------------------------

/// With `submit_on_ctrl_enter = true`, Enter inserts a newline and Ctrl+Enter
/// submits (buffer cleared).  This is the full-stack wiring guard for the
/// toggle: it proves that the setting actually propagates to editor behaviour
/// (issue #11588).
pub fn test_rich_input_toggle_on_enter_inserts_newline_and_ctrl_enter_submits() -> Builder {
    FeatureFlag::CLIAgentRichInput.set_enabled(true);

    #[cfg(unix)]
    let claude_command = "~/claude";
    #[cfg(windows)]
    let claude_command = "~/claude.cmd";

    new_builder()
        .with_setup(|utils| {
            #[cfg(unix)]
            {
                let executable = utils.test_dir().join("claude");
                std::fs::write(&executable, b"#!/bin/sh\nprintf '\\033[?2004h'\nexec cat\n")
                    .expect("must be able to write fake Claude CLI");
                let mut permissions = std::fs::metadata(&executable)
                    .expect("fake Claude CLI must exist")
                    .permissions();
                permissions.set_mode(0o755);
                std::fs::set_permissions(executable, permissions)
                    .expect("fake Claude CLI must be executable");
            }
            #[cfg(windows)]
            std::fs::write(
                utils.test_dir().join("claude.cmd"),
                b"@echo off\r\n<nul set /p \"=\x1b[?2004h\"\r\nmore\r\n",
            )
            .expect("must be able to write fake Claude CLI");
        })
        .with_user_defaults(HashMap::from([(
            SubmitRichInputOnCtrlEnter::storage_key().to_string(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_long_running_command(0, claude_command.to_owned()))
        .with_step(open_cli_agent_rich_input(0))
        // Enter inserts a newline (not a submit).
        .with_step(
            new_step_while_cli_agent_is_running(
                "Type 'line1', press Enter — buffer should contain newline (no submit)",
            )
            .with_typed_characters(&["line1"])
            .with_keystrokes(&["enter"])
            .add_assertion(rich_input_buffer_contains_newline(0)),
        )
        // Ctrl+Enter submits: buffer is cleared.
        .with_step(
            new_step_while_cli_agent_is_running(
                "Type 'line2', press Ctrl+Enter — buffer cleared (submit fired)",
            )
            .with_typed_characters(&["line2"])
            .with_keystrokes(&["ctrl-enter"])
            .add_assertion(rich_input_buffer_text_is_empty(0)),
        )
}

// ---------------------------------------------------------------------------
// Setting = true: Enter while slash-commands menu is open accepts the menu
// ---------------------------------------------------------------------------

/// Regression (#11588): with toggle ON, typing `/` opens the slash-commands
/// menu and pressing Enter must route to menu acceptance, not newline insertion.
pub fn test_rich_input_enter_accepts_menu_item_when_toggle_is_true() -> Builder {
    FeatureFlag::CLIAgentRichInput.set_enabled(true);

    new_builder()
        .with_user_defaults(HashMap::from([(
            SubmitRichInputOnCtrlEnter::storage_key().to_string(),
            true.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(open_cli_agent_rich_input(0))
        .with_step(
            new_step_with_default_assertions(
                "Type '/', press Enter — buffer must NOT contain a newline (menu branch taken)",
            )
            .with_typed_characters(&["/"])
            .with_keystrokes(&["enter"])
            .add_assertion(rich_input_buffer_does_not_contain_newline(0)),
        )
}
