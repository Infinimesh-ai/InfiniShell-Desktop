//! Preview-channel `warp-tui` binary.
//!
//! Mirrors `app/src/bin/preview.rs`: loads the `preview` channel config and
//! enables the preview feature flags, then hands off to the
//! shared TUI entry point.

use anyhow::Result;
use warp_core::channel::{Channel, ChannelState};
use warp_core::features;

fn main() -> Result<()> {
    ChannelState::set(
        ChannelState::new(
            Channel::Preview,
            warp_channel_config::load_config!("preview"),
        )
        .with_additional_features(features::PREVIEW_FLAGS),
    );

    warp_tui::run()
}
