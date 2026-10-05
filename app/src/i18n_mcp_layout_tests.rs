use std::collections::HashMap;

use i18n_embed::LanguageLoader;
use i18n_embed::fluent::fluent_language_loader;
use warpui::AssetProvider as _;
use warpui::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
use warpui::fonts::Properties;
use warpui::platform::mac::{AutoreleasePoolGuard, FontDB};
use warpui::platform::{FontDB as _, LineStyle};
use warpui::text_layout::{DEFAULT_TOP_BOTTOM_RATIO, StyleAndFont, TextStyle};

use super::super::MergedLocalizations;
use crate::ASSETS;
use crate::ui_components::json_tree::TREE_FONT_SIZE;

#[test]
fn mcp_error_messages_wrap_without_truncation_in_both_locales() {
    // 复用 GUI 的原生 CoreText 排版与默认等宽字体；App::test 的字体是 mock。
    let _pool = AutoreleasePoolGuard::new();
    let mut font_db = FontDB::new();
    let font_family = font_db
        .load_from_bytes(
            "Hack",
            vec![
                ASSETS
                    .get("bundled/fonts/hack/Hack-Regular.ttf")
                    .unwrap()
                    .to_vec(),
            ],
        )
        .unwrap();
    let line_style = LineStyle {
        font_size: TREE_FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };

    for locale in ["en", "zh-CN"] {
        // 使用独立加载器，避免改变其它并行测试或用户界面的全局语言。
        let loader = fluent_language_loader!();
        loader.load_fallback_language(&MergedLocalizations).unwrap();
        i18n_embed::select(&loader, &MergedLocalizations, &[locale.parse().unwrap()]).unwrap();
        loader.set_use_isolating(false);

        for key in [
            "ai-mcp-tool-timeout-before-dispatch",
            "ai-mcp-tool-timeout-after-dispatch",
            "ai-mcp-tool-outcome-unknown",
        ] {
            let error = loader.get_args(key, HashMap::from([("seconds", 1800)]));
            assert_ne!(error, key, "{locale} 缺少 MCP 消息 {key}");
            let text = loader.get_args(
                "ai-requested-command-error",
                HashMap::from([("error", error.as_str())]),
            );
            let char_count = text.chars().count();
            let style_runs = [(
                0..char_count,
                StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
            )];

            for outer_width in [320., 600.] {
                // 对应 RequestedCommandView 错误正文左右各 16px 的内边距。
                let inner_width = outer_width - 32.;
                let frame = font_db.text_layout_system().layout_text(
                    &text,
                    line_style,
                    &style_runs,
                    inner_width,
                    f32::MAX,
                    Default::default(),
                    None,
                );
                assert_eq!(
                    frame.lines().last().unwrap().end_index(),
                    char_count,
                    "{locale} {key} 在 {outer_width}px 时未排版到最后一个字符"
                );
                assert!(
                    frame.height().is_finite() && frame.height() > 0. && frame.height() < 500.,
                    "{locale} {key} 在 {outer_width}px 时正文高度超出滚动视口：{}",
                    frame.height()
                );
                for line in frame.lines() {
                    // CoreText 总会给末行配置 ClipConfig，即使整行已完全放下。
                    // Line::paint_internal 仅在去掉尾部空白后的宽度溢出时启用淡出；
                    // 因此检查实际可见宽度，不能把存在裁剪策略当作已经发生截断。
                    let visible_width = line.width - line.trailing_whitespace_width;
                    assert!(
                        visible_width.is_finite()
                            && visible_width >= 0.
                            && visible_width <= inner_width,
                        "{locale} {key} 在 {outer_width}px 时文字超过可用宽度：{visible_width}"
                    );
                    assert!(
                        line.chars_with_missing_glyphs.is_empty(),
                        "{locale} {key} 在 {outer_width}px 时出现缺失字形"
                    );
                }
            }
        }
    }
}
