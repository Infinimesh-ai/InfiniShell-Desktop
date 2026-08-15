//! The headless `infinishell-tui` front-end's app-side entry point.
//!
//! `warp_tui` boots the local app via [`crate::run_tui`]. Once shared
//! initialization is done, [`init`] registers the TUI-facing managers and
//! mounts the terminal session directly; the local edition has no account
//! authentication gate.
mod mcp;

pub use mcp::{
    TuiMcpAction, TuiMcpConfigDiagnostic, TuiMcpFileScope, TuiMcpFileSource, TuiMcpInstallRequest,
    TuiMcpManager, TuiMcpManagerEvent, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerSource,
    TuiMcpServerStatus, TuiMcpSnapshot, TuiMcpSyncedTemplateProvenance, TuiMcpTemplateVariable,
    TuiMcpTransport, TuiMcpVariableValue,
};
use warpui::{AppContext, SingletonEntity};

use crate::TuiMountFn;
use crate::ai::mcp::FileBasedMCPManager;
use crate::tui_onboarding_markers::TuiOnboardingMarkers;

/// Entry point invoked once the local headless app is initialized.
pub(crate) fn init(mount: TuiMountFn, ctx: &mut AppContext) {
    ctx.add_singleton_model(TuiMcpManager::new);
    let onboarding_markers = ctx.add_singleton_model(TuiOnboardingMarkers::new);
    onboarding_markers.update(ctx, |markers, ctx| {
        markers.load_current_account(ctx);
    });

    mount(ctx);
    FileBasedMCPManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.activate_global_warp_servers(ctx);
    });
}
