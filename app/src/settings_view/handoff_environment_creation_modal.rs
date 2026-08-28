//! Zap:上游这个弹窗用来在 Warp 云端创建 ambient agent / handoff 运行环境,
//! 内部完全构建在 `crate::ai::cloud_environments`(云端环境 owner 解析)、
//! `crate::settings_view::update_environment_form::UpdateEnvironmentForm`
//! (Zap Wave 7-2 已随 cloud ambient agent 主体物理删)与
//! `UpdateManager::create_ambient_agent_environment_online`(云端在线创建)之上。
//! 这三条依赖在本地优先形态下都已下线,因此这里只保留**视图外壳**:
//! 类型、事件、`Entity` / `TypedActionView` / `View` 实现与取消路径全部保留,
//! 让仍在引用它的调用点(workspace/view.rs、inline_action/create_environment_modal.rs)
//! 继续编译;创建流程本身变成 no-op —— 弹窗只说明该功能在本地版不可用。
//!
//! 后续清理建议:整个「云端环境创建」入口(本模块 + `settings_view/mod.rs` 的
//! 模块声明 + 上述两个调用点 + `WorkspaceAction::ShowHandoffEnvironmentCreationModal`)
//! 应当整体删除,详见交付报告中的 NOTE。

use pathfinder_color::ColorU;
use warpui::elements::{
    Align, CrossAxisAlignment, Dismiss, Element, Flex, MouseStateHandle, ParentElement, Text,
};
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::modal::MODAL_BACKDROP_OPACITY;
use crate::server::ids::SyncId;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::dialog::{Dialog, dialog_styles};
use crate::ui_components::icons::Icon;

const DIALOG_WIDTH: f32 = 600.;
const BODY_FONT_SIZE: f32 = 12.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HandoffEnvironmentCreationModalContext {
    Handoff,
    Orchestration,
}

#[derive(Debug, Clone)]
pub(crate) enum HandoffEnvironmentCreationModalEvent {
    // Zap:本地版永远不会再发出 `Created` —— 环境创建需要云端 API。
    // 保留 variant 是为了让调用点的 match 分支不必改动。
    #[allow(dead_code)]
    Created {
        env_id: SyncId,
    },
    Cancelled,
    #[allow(dead_code)]
    CreationFailed {
        error_message: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum HandoffEnvironmentCreationModalAction {
    Cancel,
}

pub(crate) struct HandoffEnvironmentCreationModal {
    context: HandoffEnvironmentCreationModalContext,
    close_button_mouse_state: MouseStateHandle,
}

impl HandoffEnvironmentCreationModal {
    pub(crate) fn new(ctx: &mut ViewContext<Self>) -> Self {
        Self::new_impl(HandoffEnvironmentCreationModalContext::Handoff, ctx)
    }

    pub(crate) fn new_for_orchestration(ctx: &mut ViewContext<Self>) -> Self {
        Self::new_impl(HandoffEnvironmentCreationModalContext::Orchestration, ctx)
    }

    fn new_impl(
        context: HandoffEnvironmentCreationModalContext,
        _ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self {
            context,
            close_button_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Zap:上游这里会重置滚动状态并把焦点交给 `UpdateEnvironmentForm`。
    /// 表单已删除,只保留重绘通知。
    pub(crate) fn show(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }

    #[cfg(test)]
    pub(crate) fn uses_orchestration_form_configuration_for_test(&self, _app: &AppContext) -> bool {
        self.context == HandoffEnvironmentCreationModalContext::Orchestration
    }

    fn render_dialog(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();

        let close_button = icon_button(
            appearance,
            Icon::X,
            false,
            self.close_button_mouse_state.clone(),
        )
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(HandoffEnvironmentCreationModalAction::Cancel);
        })
        .finish();

        let body = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Text::new_inline(
                    crate::t!("settings-env-modal-cloud-unavailable"),
                    appearance.ui_font_family(),
                    BODY_FONT_SIZE,
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .finish(),
            )
            .finish();

        let padded_body = warpui::elements::Container::new(body)
            .with_uniform_padding(8.)
            .finish();

        let dialog = Dialog::new(
            crate::t!("settings-env-modal-create-environment"),
            None,
            dialog_styles(appearance),
        )
        .with_close_button(close_button)
        .with_child(padded_body)
        .with_width(DIALOG_WIDTH)
        .build();

        let dialog = Dismiss::new(dialog.finish())
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(HandoffEnvironmentCreationModalAction::Cancel);
            })
            .finish();

        warpui::elements::Container::new(Align::new(dialog).finish())
            .with_background_color(ColorU::new(0, 0, 0, MODAL_BACKDROP_OPACITY))
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}

impl Entity for HandoffEnvironmentCreationModal {
    type Event = HandoffEnvironmentCreationModalEvent;
}

impl TypedActionView for HandoffEnvironmentCreationModal {
    type Action = HandoffEnvironmentCreationModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HandoffEnvironmentCreationModalAction::Cancel => {
                ctx.emit(HandoffEnvironmentCreationModalEvent::Cancelled);
            }
        }
    }
}

impl View for HandoffEnvironmentCreationModal {
    fn ui_name() -> &'static str {
        "HandoffEnvironmentCreationModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        self.render_dialog(appearance, app)
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {}
}
