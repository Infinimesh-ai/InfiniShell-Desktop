//! 固定 Grok 的真实英中静态视口；审批状态另需已登录原生会话。

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use warp::i18n;
use warp::integration_testing::clipboard::{
    PermissionPolicy, advance_v05_grok_scroll, finish_v05_static_evidence,
    open_cli_clipboard_composer, scroll_v05_grok, select_v05_grok_policy,
    setup_cli_system_clipboard, wait_until_cli_clipboard_bootstrapped,
    wait_v05_fixed_grok_installation,
};
use warpui_core::integration::TestStep;

use crate::Builder;

pub fn test_cli_grok_static_viewport() -> Builder {
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

    let mut builder = Builder::new()
        .with_real_display()
        .with_setup(setup_cli_system_clipboard)
        .with_step(wait_until_cli_clipboard_bootstrapped())
        .with_step(
            TestStep::new("设置受支持的视口尺寸").with_action(move |app, window_id, _| {
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
        .with_step(select_v05_grok_policy(PermissionPolicy::Inherit))
        .with_step(TestStep::new("继承策略原图").with_take_screenshot("inherit.png"))
        .with_step(select_v05_grok_policy(
            PermissionPolicy::GrokRestrictedReadV1,
        ))
        .with_step(TestStep::new("固定只读策略原图").with_take_screenshot("fixed-read.png"))
        .with_step(select_v05_grok_policy(
            PermissionPolicy::GrokRestrictedFilesV1,
        ))
        .with_step(TestStep::new("固定文件策略原图").with_take_screenshot("fixed-files.png"))
        .with_step(select_v05_grok_policy(
            PermissionPolicy::GrokRestrictedFilesV2,
        ))
        .with_step(TestStep::new("技能禁用说明原图").with_take_screenshot("skill-disabled.png"));
    for _ in 0..12 {
        builder = builder.with_step(advance_v05_grok_scroll());
    }
    builder
        .with_step(scroll_v05_grok("bottom"))
        .with_step(TestStep::new("滚动区底部原图").with_take_screenshot("scroll-bottom.png"))
        .with_step(scroll_v05_grok("middle"))
        .with_step(TestStep::new("滚动区中部原图").with_take_screenshot("scroll-middle.png"))
        .with_step(scroll_v05_grok("top"))
        .with_step(TestStep::new("滚动区顶部原图").with_take_screenshot("scroll-top.png"))
        .with_step(finish_v05_static_evidence())
}
