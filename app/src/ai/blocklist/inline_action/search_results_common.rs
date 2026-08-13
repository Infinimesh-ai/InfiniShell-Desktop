use warp_core::ui::appearance::Appearance;
use warpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Flex, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, EventContext, SingletonEntity};

use super::inline_action_header::{
    ICON_MARGIN, INLINE_ACTION_HEADER_VERTICAL_PADDING, INLINE_ACTION_HORIZONTAL_PADDING,
};
use super::inline_action_icons::icon_size;
use crate::ui_components::icons::Icon;
use crate::ui_components::spinner::{BrailleSpinner, SpinnerStateHandle};

pub fn render_search_results_header(
    title_text: String,
    right_label_text: String,
    is_expanded: bool,
    mouse_state: warpui::elements::MouseStateHandle,
    on_toggle: impl Fn(&mut EventContext) + 'static,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let header_background = theme.surface_2();

    let mut header_row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Left side: icon + title
    let mut left_side = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    let search_icon = ConstrainedBox::new(
        warpui::elements::Icon::new(
            Icon::SearchSmall.into(),
            appearance.theme().main_text_color(header_background),
        )
        .finish(),
    )
    .with_width(icon_size(app) - 4.)
    .with_height(icon_size(app) - 4.)
    .finish();
    left_side.add_child(
        Container::new(search_icon)
            .with_margin_right(ICON_MARGIN)
            .finish(),
    );

    let title = Text::new_inline(
        title_text,
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(appearance.theme().main_text_color(header_background).into())
    .finish();
    left_side.add_child(Shrinkable::new(1.0, title).finish());
    header_row.add_child(Shrinkable::new(1.0, left_side.finish()).finish());

    // Right side: results label + chevron
    let mut right_side = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    let right_label = Text::new_inline(
        right_label_text,
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(appearance.theme().sub_text_color(header_background).into())
    .finish();
    right_side.add_child(Container::new(right_label).with_margin_right(8.).finish());

    let chevron_icon = ConstrainedBox::new(
        warpui::elements::Icon::new(
            if is_expanded {
                Icon::ChevronDown.into()
            } else {
                Icon::ChevronRight.into()
            },
            appearance.theme().sub_text_color(header_background),
        )
        .finish(),
    )
    .with_width(icon_size(app))
    .with_height(icon_size(app))
    .finish();
    right_side.add_child(chevron_icon);

    let inner = Container::new(header_row.with_child(right_side.finish()).finish())
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
        .with_background(header_background)
        .with_corner_radius(if is_expanded {
            CornerRadius::with_top(Radius::Pixels(8.))
        } else {
            CornerRadius::with_all(Radius::Pixels(8.))
        })
        .finish();

    warpui::elements::Hoverable::new(mouse_state, |_| inner)
        .on_click(move |ctx, _, _| {
            on_toggle(ctx);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
}

pub fn render_results_body_container(body: Box<dyn Element>, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    Container::new(body)
        .with_background(appearance.theme().surface_1())
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
        .finish()
}

/// Renders a single-line status header (icon + title) for a loading, cancelled, or
/// error state; the caller-supplied icon conveys the status.
pub fn render_status_header(
    text: String,
    icon: warpui::elements::Icon,
    app: &AppContext,
) -> Box<dyn Element> {
    // 旧签名 fallback:不传 shimmer handle 时退化为静态 Text。
    // 新代码请用 `render_loading_header_shimmer` 让标题文字带 shimmer 反馈,
    // 或 `render_loading_header_animated` 让 icon 也带 spinner 帧动画。
    render_loading_header_inner(text, IconOrSpinner::Icon(icon), None, app)
}

/// Loading 卡 header,标题文字 shimmer 动画(icon 仍为静态)。
/// caller 必须在 view 结构里持久化 `ShimmeringTextStateHandle`(否则每帧 new
/// 会让 animation_start_time 永远归零、动画不动)。
pub fn render_loading_header_shimmer(
    text: String,
    icon: warpui::elements::Icon,
    shimmer_handle: ShimmeringTextStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    render_loading_header_inner(text, IconOrSpinner::Icon(icon), Some(shimmer_handle), app)
}

/// Loading 卡 header,**双动画**:icon = `BrailleSpinner`(80ms 帧切换 ⠋⠙⠹⠸⠼...),
/// 标题文字 = `ShimmeringTextElement`。视觉等价于 opencode TUI 的 `<spinner>` + 文字。
///
/// caller 必须在 view 结构里持久化 `SpinnerStateHandle` 和 `ShimmeringTextStateHandle`,
/// 否则每帧 new 让 `Instant::now()` 重置 → spinner 永远停在第 0 帧、shimmer 永远归零。
pub fn render_loading_header_animated(
    text: String,
    spinner_handle: SpinnerStateHandle,
    shimmer_handle: ShimmeringTextStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    render_loading_header_inner(
        text,
        IconOrSpinner::Spinner(spinner_handle),
        Some(shimmer_handle),
        app,
    )
}

/// 终态 header(cancelled / denied):icon 为静态,标题文字带**删除线**。
/// 对齐 opencode TUI 的 STRIKETHROUGH 用法:permission denied / 取消的工具
/// 卡片用删除线表达"这个操作没发生过 / 被驳回"的视觉语义。
pub fn render_terminal_header_strikethrough(
    text: String,
    icon: warpui::elements::Icon,
    app: &AppContext,
) -> Box<dyn Element> {
    use warpui::elements::{Highlight, HighlightedRange};
    use warpui::text_layout::TextStyle;
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let header_background = theme.surface_2();

    let mut header_row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Start)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    let icon_box = ConstrainedBox::new(icon.finish())
        .with_width(icon_size(app))
        .with_height(icon_size(app))
        .finish();
    header_row.add_child(
        Container::new(icon_box)
            .with_margin_right(ICON_MARGIN)
            .finish(),
    );

    let text_color = appearance
        .theme()
        .sub_text_color(header_background)
        .into_solid();
    let strike_style = TextStyle::new()
        .with_show_strikethrough(true)
        .with_foreground_color(text_color);
    let highlight = Highlight::default().with_text_style(strike_style);
    let text_len = text.chars().count();
    let title = Text::new_inline(
        text,
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(text_color)
    .with_highlights(vec![HighlightedRange {
        highlight,
        highlight_indices: (0..text_len).collect(),
    }])
    .finish();

    header_row.add_child(Shrinkable::new(1.0, title).finish());

    Container::new(header_row.finish())
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
        .with_background(header_background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish()
}

enum IconOrSpinner {
    Icon(warpui::elements::Icon),
    Spinner(SpinnerStateHandle),
}

fn render_loading_header_inner(
    text: String,
    icon_or_spinner: IconOrSpinner,
    shimmer_handle: Option<ShimmeringTextStateHandle>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let header_background = theme.surface_2();

    let mut header_row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Start)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    let icon_element: Box<dyn Element> = match icon_or_spinner {
        IconOrSpinner::Icon(icon) => icon.finish(),
        IconOrSpinner::Spinner(spinner_state) => {
            // Spinner 颜色对齐原 yellow_running_icon:Yellow ANSI。
            // 颜色直接复用 sub_text_color 在 surface_2 上的视觉(浅黄/橙)避免对比过强。
            use warp_core::ui::theme::AnsiColorIdentifier;
            let color = AnsiColorIdentifier::Yellow.to_ansi_color(&theme.terminal_colors().normal);
            Box::new(BrailleSpinner::new(
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
                color,
                spinner_state,
            ))
        }
    };

    let icon_box = ConstrainedBox::new(icon_element)
        .with_width(icon_size(app))
        .with_height(icon_size(app))
        .finish();
    header_row.add_child(
        Container::new(icon_box)
            .with_margin_right(ICON_MARGIN)
            .finish(),
    );

    let title: Box<dyn Element> = if let Some(handle) = shimmer_handle {
        // shimmer base = 暗 / shimmer_color = 主前景,绕一圈高亮波。
        let base_color = appearance
            .theme()
            .sub_text_color(header_background)
            .into_solid();
        let shimmer_color = appearance
            .theme()
            .main_text_color(header_background)
            .into_solid();
        ShimmeringTextElement::new(
            text,
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
            base_color,
            shimmer_color,
            ShimmerConfig::default(),
            handle,
        )
        .finish()
    } else {
        Text::new_inline(
            text,
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(appearance.theme().main_text_color(header_background).into())
        .finish()
    };
    header_row.add_child(Shrinkable::new(1.0, title).finish());

    Container::new(header_row.finish())
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
        .with_background(header_background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish()
}

pub fn render_expanded_layout(
    header: Box<dyn Element>,
    body_container: Box<dyn Element>,
) -> Box<dyn Element> {
    Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(header)
        .with_child(body_container)
        .finish()
}

/// A composable helper for managing collapsible search results state.
/// Parent views should store this and call `toggle_expanded()` on user interaction.
pub struct CollapsibleSearchResultsState {
    pub is_expanded: bool,
    pub mouse_state: MouseStateHandle,
}

impl CollapsibleSearchResultsState {
    pub fn new() -> Self {
        Self {
            is_expanded: false,
            mouse_state: MouseStateHandle::default(),
        }
    }

    pub fn toggle_expanded(&mut self) {
        self.is_expanded = !self.is_expanded;
    }
}

impl Default for CollapsibleSearchResultsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render a complete collapsible search results view with header and optional body.
/// This is a convenience wrapper around the individual rendering functions.
/// The on_toggle callback will be called when the user clicks the header.
pub fn render_collapsible_search_results<F>(
    title_text: String,
    results_count: usize,
    results_label: &str,
    state: &CollapsibleSearchResultsState,
    body: Option<Box<dyn Element>>,
    on_toggle: F,
    app: &AppContext,
) -> Box<dyn Element>
where
    F: Fn(&mut EventContext) + 'static,
{
    let right_label = format!("{results_count} {results_label}");

    let header = render_search_results_header(
        title_text,
        right_label,
        state.is_expanded,
        state.mouse_state.clone(),
        on_toggle,
        app,
    );

    if state.is_expanded {
        if let Some(body_content) = body {
            let body_container = render_results_body_container(body_content, app);
            render_expanded_layout(header, body_container)
        } else {
            header
        }
    } else {
        header
    }
}
