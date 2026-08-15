use warpui_core::{AppContext, ViewContext, ViewHandle};

use super::TuiHandoffBlock;
use crate::terminal_session_view::TuiTerminalSessionView;

impl TuiTerminalSessionView {
    pub(super) fn active_handoff(&self, _ctx: &AppContext) -> Option<ViewHandle<TuiHandoffBlock>> {
        None
    }

    pub(super) fn start_handoff(
        &mut self,
        _argument: Option<&String>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }
}
