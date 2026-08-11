use warp_core::ui::appearance::Appearance;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{
    AddSingletonModel, App, AppContext, Element, Entity, TypedActionView, View, WindowId,
};

use super::CreateEnvironmentModal;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;

#[derive(Default)]
struct TestRootView;

impl Entity for TestRootView {
    type Event = ();
}

impl View for TestRootView {
    fn ui_name() -> &'static str {
        "TestRootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for TestRootView {
    type Action = ();
}

fn create_test_window(app: &mut App) -> WindowId {
    let (window_id, _root_view) = app.add_window(WindowStyle::NotStealFocus, |_| TestRootView);
    window_id
}

/// Zap:`HandoffEnvironmentCreationModal` 在本地优先形态下已收缩成纯视图外壳
/// (云端环境创建链路整条下线),构造它不再需要 `ServerApiProvider` / `SyncQueue` /
/// `TeamTesterStatus` / `CloudModel` 等云端 singleton —— 这些类型本身也已被删除。
/// 这里只保留仍然存在、且弹窗渲染真正会用到的最小依赖集。
fn init_create_environment_modal_test_models(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
}

#[test]
fn test_create_environment_modal_uses_orchestration_form_configuration() {
    App::test((), |mut app| async move {
        init_create_environment_modal_test_models(&mut app);
        let window_id = create_test_window(&mut app);

        app.update(|ctx| {
            let view_handle = ctx.add_typed_action_view(window_id, CreateEnvironmentModal::new);
            let modal = view_handle.as_ref(ctx);

            assert!(
                modal
                    .handoff_modal
                    .as_ref(ctx)
                    .uses_orchestration_form_configuration_for_test(ctx),
                "Expected CreateEnvironmentModal to construct the handoff modal with orchestration form configuration"
            );
        });
    })
}
