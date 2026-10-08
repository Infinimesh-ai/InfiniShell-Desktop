//! OSS-channel `infinishell-tui` binary and `default-run` target.
//!
//! This is what bare `cargo run -p warp_tui` builds, so it hand-builds a
//! production config and needs no internal `warp-channel-config` generator
//! (mirrors `app/src/bin/oss.rs`). It is a console application (no GUI window,
//! no app bundle), so unlike the GUI binaries it sets no `windows_subsystem`
//! attribute and embeds no `Info.plist`.

use anyhow::Result;
use warp_core::AppId;
use warp_core::channel::{Channel, ChannelConfig, ChannelState};

fn main() -> Result<()> {
    #[cfg(all(target_os = "linux", feature = "cli-agent-notify-trace"))]
    warp::cli_agent_notify_trace::initialize();
    let mut state = ChannelState::new(
        Channel::Oss,
        ChannelConfig {
            app_id: AppId::new("dev", "infinishell", "InfiniShellTUI"),
            logfile_name: "infinishell-tui.log".into(),
            autoupdate_config: None,
            mcp_static_config: None,
        },
    );
    if cfg!(debug_assertions) {
        state = state.with_additional_features(warp_core::features::DEBUG_FLAGS);
    }
    ChannelState::set(state);

    let result = warp_tui::run();
    #[cfg(all(target_os = "linux", feature = "cli-agent-notify-trace"))]
    warp::cli_agent_notify_trace::emit(if result.is_ok() {
        warp::cli_agent_notify_trace::Stage::MainOk
    } else {
        warp::cli_agent_notify_trace::Stage::MainError
    });
    result
}
