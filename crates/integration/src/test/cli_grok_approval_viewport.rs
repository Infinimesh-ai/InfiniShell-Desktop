//! 固定 Grok 的英中真实审批卡片视口；原生审批由独立在线验收覆盖。

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use warp::i18n;
use warp::integration_testing::clipboard::{
    ApprovalDecision, PermissionPolicy, click_v05_approval, finish_v05_approval_evidence,
    open_cli_clipboard_composer, prepare_v05_approval_fixture, reveal_v05_approval,
    select_v05_grok_policy, setup_cli_system_clipboard, wait_until_cli_clipboard_bootstrapped,
    wait_v05_fixed_grok_installation,
};
use warpui_core::integration::TestStep;

use crate::Builder;

pub fn test_cli_grok_approval_viewport() -> Builder {
    let locale = std::env::var("WARP_TEST_GUI_LOCALE").expect("必须指定英中界面语言");
    assert!(matches!(locale.as_str(), "en" | "zh-CN"));
    let size = std::env::var("WARP_TEST_GUI_SIZE").expect("必须指定视口尺寸");
    let dimensions = match size.as_str() {
        "compact" => vec2f(800.0, 600.0),
        "normal" => vec2f(1280.0, 800.0),
        _ => panic!("未知视口尺寸：{size}"),
    };
    i18n::init(Some(&locale));
    i18n::set_locale(&locale);

    Builder::new()
        .with_real_display()
        .with_setup(setup_cli_system_clipboard)
        .with_step(wait_until_cli_clipboard_bootstrapped())
        .with_step(
            TestStep::new("设置审批验收视口尺寸").with_action(move |app, window_id, _| {
                let bounds = app.window_bounds(&window_id).expect("真实窗口尺寸");
                app.update(|ctx| {
                    ctx.set_and_cache_window_bounds(
                        window_id,
                        RectF::new(bounds.origin(), dimensions),
                    );
                });
            }),
        )
        .with_step(open_cli_clipboard_composer())
        .with_step(wait_v05_fixed_grok_installation())
        .with_step(select_v05_grok_policy(
            PermissionPolicy::GrokRestrictedFilesV1,
        ))
        .with_step(prepare_v05_approval_fixture())
        .with_step(reveal_v05_approval(0, ApprovalDecision::AllowOnce))
        .with_step(TestStep::new("允许按钮待点击原图").with_take_screenshot("allow-ready.png"))
        .with_step(click_v05_approval(0, ApprovalDecision::AllowOnce))
        .with_step(TestStep::new("允许动作待确认原图").with_take_screenshot("allow-pending.png"))
        .with_step(reveal_v05_approval(1, ApprovalDecision::DenyOnce))
        .with_step(TestStep::new("拒绝按钮待点击原图").with_take_screenshot("deny-ready.png"))
        .with_step(click_v05_approval(1, ApprovalDecision::DenyOnce))
        .with_step(TestStep::new("拒绝动作待确认原图").with_take_screenshot("deny-pending.png"))
        .with_step(finish_v05_approval_evidence())
}
