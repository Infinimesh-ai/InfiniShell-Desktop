//! [`RootTuiView`]: the local `infinishell-tui` session root.

use warpui::SingletonEntity as _;
use warpui_core::elements::tui::{TuiChildView, TuiElement};
use warpui_core::keymap::FixedBinding;
use warpui_core::keymap::macros::*;
use warpui_core::platform::TerminationMode;
use warpui_core::{
    AppContext, Entity, EntityId, FocusContext, TuiView, TypedActionView, ViewContext, keymap,
};

use crate::keybindings::TUI_BINDING_GROUP;
use crate::session_registry::{TuiSessionView, TuiSessions};
use crate::ui::terminal_starting;

/// Typed actions handled by [`RootTuiView`].
#[derive(Debug, Clone)]
pub enum RootTuiAction {
    /// Exits the local app.
    ExitApp,
}

/// The app-level TUI shell, projecting only the focused full session view.
pub struct RootTuiView;

/// Registers the root view's keybindings.
pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-c",
        RootTuiAction::ExitApp,
        id!(RootTuiView::ui_name()),
    )
    .with_group(TUI_BINDING_GROUP)]);
}

impl RootTuiView {
    /// Creates the local session root.
    pub(crate) fn new() -> Self {
        Self
    }

    fn focused_session_view(&self, ctx: &AppContext) -> Option<TuiSessionView> {
        if !ctx.has_singleton_model::<TuiSessions>() {
            return None;
        }

        TuiSessions::as_ref(ctx)
            .focused_session()
            .map(|session| session.view().clone())
    }
}

impl Entity for RootTuiView {
    type Event = ();
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn child_view_ids(&self, ctx: &AppContext) -> Vec<EntityId> {
        self.focused_session_view(ctx)
            .map(|view| vec![view.id()])
            .unwrap_or_default()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused()
            && let Some(view) = self.focused_session_view(ctx)
        {
            view.activate(ctx);
        }
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        self.focused_session_view(ctx)
            .map(|view| match view {
                TuiSessionView::Terminal(view) => TuiChildView::new(&view).finish(),
                TuiSessionView::Cloud(view) => TuiChildView::new(&view).finish(),
            })
            .unwrap_or_else(terminal_starting)
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        let mut context = keymap::Context::default();
        context.set.insert("RootTuiView");
        context
    }
}

impl TypedActionView for RootTuiView {
    type Action = RootTuiAction;

    fn handle_action(&mut self, action: &RootTuiAction, ctx: &mut ViewContext<Self>) {
        match action {
            RootTuiAction::ExitApp => {
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
        }
    }
}

#[cfg(test)]
#[path = "root_view_tests.rs"]
mod tests;
