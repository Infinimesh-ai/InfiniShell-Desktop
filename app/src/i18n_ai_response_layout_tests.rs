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

#[test]
fn response_headers_and_repeat_error_fit_both_locales() {
    // 使用真实 CoreText 字形和独立语言加载器，不改用户或其它测试的全局语言。
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
    for locale in ["en", "zh-CN"] {
        let loader = fluent_language_loader!();
        loader.load_fallback_language(&MergedLocalizations).unwrap();
        i18n_embed::select(&loader, &MergedLocalizations, &[locale.parse().unwrap()]).unwrap();
        loader.set_use_isolating(false);
        for key in [
            "ai-long-response-scroll",
            "ai-long-response-expand",
            "ai-error-repeated-response",
        ] {
            let text = loader.get(key);
            assert_ne!(text, key);
            let count = text.chars().count();
            let styles = [(
                0..count,
                StyleAndFont::new(font_family, Properties::default(), TextStyle::new()),
            )];
            for font_size in [14., 16.] {
                for width in [240., 520.] {
                    let frame = font_db.text_layout_system().layout_text(
                        &text,
                        LineStyle {
                            font_size,
                            line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
                            baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                            fixed_width_tab_size: None,
                        },
                        &styles,
                        width,
                        f32::MAX,
                        Default::default(),
                        None,
                    );
                    assert_eq!(
                        frame.lines().last().unwrap().end_index(),
                        count,
                        "{locale} {key} 文字被截断"
                    );
                    if key != "ai-error-repeated-response" {
                        assert_eq!(
                            frame.lines().len(),
                            1,
                            "{locale} 长回复按钮在 {width}px 时换行"
                        );
                    }
                    for line in frame.lines() {
                        assert!(
                            line.width - line.trailing_whitespace_width <= width,
                            "{locale} {key} 溢出"
                        );
                        assert!(
                            line.chars_with_missing_glyphs.is_empty(),
                            "{locale} {key} 缺少字形"
                        );
                    }
                }
            }
        }
    }
}
