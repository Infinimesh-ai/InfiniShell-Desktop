use warpui::App;

use super::*;
use crate::settings_view::settings_page::FilteredPageType;

#[test]
fn cli_agent_update_search_keeps_the_page_title_and_only_the_matching_cli() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let widgets = [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok]
                .into_iter()
                .map(|agent| {
                    Box::new(CLIAgentUpdateWidget::new(agent))
                        as Box<dyn SettingsWidget<View = AISettingsPageView>>
                })
                .collect();
            let mut page = PageType::new_uncategorized(widgets, Some("Third-party agents"));
            for (query, expected) in [
                ("codex channel", "cli-codex-auto-update"),
                ("Claude 渠道", "cli-claude-auto-update"),
                ("grok alpha", "cli-grok-auto-update"),
            ] {
                page.update_filter(query, ctx);
                let FilteredPageType::Uncategorized { widgets, title, .. } = page.get_filtered()
                else {
                    panic!("升级设置必须保持可分别搜索的列表");
                };
                assert_eq!(title, Some("Third-party agents"));
                assert_eq!(widgets.len(), 1, "{query}");
                assert_eq!(widgets[0].widget_id(), expected);
            }
            page.update_filter("", ctx);
            let FilteredPageType::Uncategorized { widgets, .. } = page.get_filtered() else {
                panic!("升级设置必须保持可分别搜索的列表");
            };
            assert_eq!(widgets.len(), 3, "清除搜索应恢复全部升级控件");
            page.update_filter("claude alpha", ctx);
            let FilteredPageType::Uncategorized { widgets, .. } = page.get_filtered() else {
                panic!("升级设置必须保持可分别搜索的列表");
            };
            assert!(widgets.is_empty(), "不应为 Claude 宣传不存在的 alpha 渠道");
        });
    });
}
