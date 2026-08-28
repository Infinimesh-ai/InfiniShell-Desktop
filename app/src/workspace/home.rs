//! Zap Home
//!
//! This is the landing page for new tabs if session creation isn't supported (e.g. on the web).
//! It's intentionally small and local-only.

use warpui::ViewContext;

use super::view::Workspace;
use crate::pane_group::{AnyPaneContent, FilePane};

/// Create a static "home page" pane.
pub fn create_home_pane(ctx: &mut ViewContext<Workspace>) -> Box<dyn AnyPaneContent> {
    let pane = FilePane::new(
        None,
        None,
        #[cfg(feature = "local_fs")]
        None,
        ctx,
    );
    pane.file_view(ctx).update(ctx, |pane, ctx| {
        let content = crate::t!("workspace-home-content");
        pane.open_static(crate::t!("workspace-home-title"), &content, ctx);
    });
    Box::new(pane)
}
