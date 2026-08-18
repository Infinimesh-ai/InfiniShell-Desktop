use std::sync::Arc;

use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Border, ChildAnchor, ChildView, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Stack,
};
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::AgentInputButtonTheme;
use crate::ai::agent::conversation::AIConversationAutoexecuteMode;
use crate::ai::blocklist::{
    BlocklistAIContextEvent, BlocklistAIContextModel, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel,
};
use crate::appearance::Appearance;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields, MenuVariant};
use crate::terminal::input::{MenuPositioning, MenuPositioningProvider};
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, TooltipAlignment};

const MENU_WIDTH: f32 = 360.;

pub struct ApprovalModeSelector {
    button: ViewHandle<ActionButton>,
    menu: ViewHandle<Menu<ApprovalModeSelectorAction>>,
    is_menu_open: bool,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    context_model: ModelHandle<BlocklistAIContextModel>,
    current_mode: AIConversationAutoexecuteMode,
}

pub enum ApprovalModeSelectorEvent {
    MenuVisibilityChanged { open: bool },
}

#[derive(Debug, Clone)]
pub enum ApprovalModeSelectorAction {
    ToggleMenu,
    SelectMode(AIConversationAutoexecuteMode),
}

impl ApprovalModeSelector {
    pub fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        context_model: ModelHandle<BlocklistAIContextModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new(
                crate::t!("ai-footer-approval-mode-ask-label"),
                AgentInputButtonTheme,
            )
            .with_icon(Icon::FastForward)
            .with_tooltip(crate::t!("ai-footer-approval-mode-tooltip"))
            .with_size(ButtonSize::AgentInputButton)
            .with_tooltip_alignment(TooltipAlignment::Left)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(ApprovalModeSelectorAction::ToggleMenu);
            })
        });

        let menu = ctx.add_typed_action_view(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            Menu::new()
                .with_width(MENU_WIDTH)
                .with_menu_variant(MenuVariant::Fixed)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&menu, |me, _, event, ctx| match event {
            MenuEvent::Close { .. } => me.set_menu_visibility(false, ctx),
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        ctx.subscribe_to_model(&context_model, |me, _, event, ctx| {
            if matches!(event, BlocklistAIContextEvent::PendingQueryStateUpdated) {
                me.refresh(ctx);
            }
        });
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| match event {
                BlocklistAIHistoryEvent::StartedNewConversation { .. }
                | BlocklistAIHistoryEvent::SetActiveConversation { .. }
                | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
                | BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface { .. }
                | BlocklistAIHistoryEvent::RemoveConversation { .. }
                | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. } => me.refresh(ctx),
                _ => {}
            },
        );

        let mut selector = Self {
            button,
            menu,
            is_menu_open: false,
            menu_positioning_provider,
            context_model,
            current_mode: AIConversationAutoexecuteMode::default(),
        };
        selector.refresh(ctx);
        selector
    }

    fn set_menu_visibility(&mut self, open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_menu_open == open {
            return;
        }
        self.is_menu_open = open;
        if open {
            ctx.focus(&self.menu);
        }
        ctx.emit(ApprovalModeSelectorEvent::MenuVisibilityChanged { open });
        ctx.notify();
    }

    fn mode_label(mode: AIConversationAutoexecuteMode) -> String {
        match mode {
            AIConversationAutoexecuteMode::RespectUserSettings => {
                crate::t!("ai-footer-approval-mode-ask-label")
            }
            AIConversationAutoexecuteMode::RunToCompletion => {
                crate::t!("ai-footer-approval-mode-auto-label")
            }
            AIConversationAutoexecuteMode::FullAccess => {
                crate::t!("ai-footer-approval-mode-full-access-label")
            }
        }
    }

    fn menu_item(
        mode: AIConversationAutoexecuteMode,
        current_mode: AIConversationAutoexecuteMode,
    ) -> MenuItem<ApprovalModeSelectorAction> {
        let (title, description) = match mode {
            AIConversationAutoexecuteMode::RespectUserSettings => (
                crate::t!("ai-footer-approval-mode-ask-title"),
                crate::t!("ai-footer-approval-mode-ask-description"),
            ),
            AIConversationAutoexecuteMode::RunToCompletion => (
                crate::t!("ai-block-auto-approve-this-conversation"),
                crate::t!("ai-footer-approval-mode-auto-description"),
            ),
            AIConversationAutoexecuteMode::FullAccess => (
                crate::t!("ai-block-full-access-this-conversation"),
                crate::t!("ai-footer-approval-mode-full-access-description"),
            ),
        };
        let mut fields = MenuItemFields::new_with_stacked_label(title, description)
            .with_on_select_action(ApprovalModeSelectorAction::SelectMode(mode));
        if mode == current_mode {
            fields = fields.with_right_side_icon(Icon::Check);
        }
        fields.into_item()
    }

    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        self.current_mode = self
            .context_model
            .as_ref(ctx)
            .pending_query_autoexecute_override(ctx);
        let label = Self::mode_label(self.current_mode);
        let is_active = self.current_mode != AIConversationAutoexecuteMode::RespectUserSettings;
        let icon = if is_active {
            Icon::FastForwardFilled
        } else {
            Icon::FastForward
        };
        self.button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
            button.set_icon(Some(icon), ctx);
            button.set_active(is_active, ctx);
        });

        let current_mode = self.current_mode;
        self.menu.update(ctx, |menu, ctx| {
            menu.set_items(
                vec![
                    Self::menu_item(
                        AIConversationAutoexecuteMode::RespectUserSettings,
                        current_mode,
                    ),
                    Self::menu_item(AIConversationAutoexecuteMode::RunToCompletion, current_mode),
                    Self::menu_item(AIConversationAutoexecuteMode::FullAccess, current_mode),
                ],
                ctx,
            );
        });
        ctx.notify();
    }

    fn get_menu_positioning(&self, app: &AppContext) -> OffsetPositioning {
        match self.menu_positioning_provider.menu_position(app) {
            MenuPositioning::BelowInputBox => OffsetPositioning::offset_from_parent(
                vec2f(0., 4.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::BottomLeft,
                ChildAnchor::TopLeft,
            ),
            MenuPositioning::AboveInputBox => OffsetPositioning::offset_from_parent(
                vec2f(0., -4.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::BottomLeft,
            ),
        }
    }
}

impl TypedActionView for ApprovalModeSelector {
    type Action = ApprovalModeSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ApprovalModeSelectorAction::ToggleMenu => {
                self.set_menu_visibility(!self.is_menu_open, ctx);
            }
            ApprovalModeSelectorAction::SelectMode(mode) => {
                self.context_model.update(ctx, |context_model, ctx| {
                    context_model.set_pending_query_autoexecute_override(*mode, ctx);
                });
                self.set_menu_visibility(false, ctx);
                self.refresh(ctx);
            }
        }
    }
}

impl View for ApprovalModeSelector {
    fn ui_name() -> &'static str {
        "ApprovalModeSelector"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new().with_child(ChildView::new(&self.button).finish());
        if self.is_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.menu).finish(),
                self.get_menu_positioning(app),
            );
        }
        stack.finish()
    }
}

impl Entity for ApprovalModeSelector {
    type Event = ApprovalModeSelectorEvent;
}
