use warp::tui_export::AIConversationId;
use warpui_core::elements::tui::{TuiElement, TuiLayoutContext, TuiText};
use warpui_core::{AppContext, Entity, TuiView, TypedActionView};

/// 云端 handoff 已移除后保留的空视图。
pub(crate) struct TuiHandoffBlock;

pub(crate) enum TuiHandoffBlockEvent {
    LayoutInvalidated,
}

pub(crate) fn init(_app: &mut AppContext) {}

impl TuiHandoffBlock {
    pub(crate) fn is_active(&self, _app: &AppContext) -> bool {
        false
    }

    pub(crate) fn source_conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
        None
    }

    pub(crate) fn needs_height_measurement(&self, _width: u16) -> bool {
        false
    }

    pub(crate) fn desired_height(
        &self,
        _width: u16,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> usize {
        0
    }

    pub(crate) fn record_height_measurement(&self, _width: u16) {}
}

impl Entity for TuiHandoffBlock {
    type Event = TuiHandoffBlockEvent;
}

impl TuiView for TuiHandoffBlock {
    fn ui_name() -> &'static str {
        "TuiHandoffBlock"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        TuiText::new(String::new()).finish()
    }
}

impl TypedActionView for TuiHandoffBlock {
    type Action = ();
}
