use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::theme::Fill;
use warp_ssh_manager::{SshNode, SshServerInfo};
use warpui::elements::{Align, ConstrainedBox, Flex, Highlight, ParentElement, Shrinkable, Text};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::action::search_item::styles;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::ui_components::icons::Icon as UiIcon;

#[derive(Debug)]
pub struct SshServerSearchItem {
    pub node: SshNode,
    pub server: SshServerInfo,
    /// 显示用的 user@host(或仅 host)。
    pub host_user: String,
    /// 节点名(用作主标签)。
    pub display_name: String,
    pub match_result: FuzzyMatchResult,
}

impl SshServerSearchItem {
    pub fn new(
        node: SshNode,
        server: SshServerInfo,
        host_user: String,
        display_name: String,
    ) -> Self {
        Self {
            node,
            server,
            host_user,
            display_name,
            match_result: FuzzyMatchResult::no_match(),
        }
    }

    fn render_label(
        &self,
        item_highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        // 主标签 "name + 灰色 user@host",fuzzy match 高亮 name 部分(host 部分
        // 不画高亮,仅作辅助信息),所以 indices 只取落在 name 范围内的。
        let main_color = item_highlight_state.main_text_fill(appearance).into_solid();
        let sub_color = item_highlight_state.sub_text_fill(appearance).into_solid();

        // 注意:match_result.matched_indices 是相对于
        // `format!("{display_name} {host_user}")`(单空格)的索引。combined 用
        // 双空格分隔,索引会偏。我们重新只在 display_name 部分做高亮,host_user
        // 部分作为附属信息单独画(更直观)。
        let name_part = Text::new_inline(
            self.display_name.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(main_color)
        .with_style(Properties::default().weight(Weight::Bold))
        .with_single_highlight(
            Highlight::new()
                .with_properties(Properties::default().weight(Weight::Bold))
                .with_foreground_color(main_color),
            // 仅取落在 display_name 范围内的 indices(fuzzy 匹配的是整条 haystack,
            // 但视觉上只在 name 部分加粗高亮就够,host 那段不改样式)。
            self.match_result
                .matched_indices
                .iter()
                .copied()
                .filter(|i| *i < self.display_name.len())
                .collect(),
        )
        .finish();

        if self.host_user.is_empty() {
            return name_part;
        }

        let host_part = Text::new_inline(
            self.host_user.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(sub_color)
        .finish();

        Flex::row()
            .with_spacing(8.0)
            .with_child(name_part)
            .with_child(host_part)
            .finish()
    }

    fn render(
        &self,
        item_highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let label = self.render_label(item_highlight_state, appearance);
        let mut row = Flex::row();
        row.add_child(Shrinkable::new(1., Align::new(label).left().finish()).finish());
        ConstrainedBox::new(row.finish())
            .with_height(styles::SEARCH_ITEM_HEIGHT)
            .finish()
    }
}

impl SearchItem for SshServerSearchItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color: Fill = appearance.theme().terminal_colors().normal.cyan.into();
        render_util::render_search_item_icon(
            appearance,
            UiIcon::Key,
            icon_color.into_solid(),
            highlight_state,
        )
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        self.render(highlight_state, appearance)
    }

    fn render_details(&self, _ctx: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> CommandPaletteItemAction {
        CommandPaletteItemAction::OpenSshServer {
            node_id: self.node.id.clone(),
            server: self.server.clone(),
        }
    }

    fn execute_result(&self) -> CommandPaletteItemAction {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("SSH server: {} {}", self.display_name, self.host_user)
    }
}
