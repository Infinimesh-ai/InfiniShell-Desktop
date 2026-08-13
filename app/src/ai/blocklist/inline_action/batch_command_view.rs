//! Zap M4-B:`run_command_on_hosts` 批量命令的专属确认卡片。
//!
//! 该工具在 M4-A 中复用 `CallMCPTool` 通道,但命令未过 shell allowlist 时
//! 落在通用 MCP 工具卡(裸 JSON 树)上,可读性差。本视图按 action 惰性创建
//! (与 `RunAgentsCardView` 同模式),完整接管该 action 的渲染生命周期:
//! 流式期占位 → Blocked 时渲染确认卡(命令 / 主机清单 / 超时 / 金丝雀徽标
//! / 三按钮)→ 执行中与结束后渲染状态行。
//!
//! 三按钮语义与 `requested_command` 对齐:
//! - Accept:仅执行本次(父级 `AIBlock` 收到 [`BatchCommandViewEvent::Accepted`]
//!   后走既有的 `execute_action` 链路);
//! - AcceptAndAutoExecute(拆分按钮菜单里的「始终允许」):执行本次并把命令
//!   加入 `can_autoexecute_command` 检查的命令 allowlist,此后相同命令的批量
//!   调用免确认;
//! - Reject:与通用 MCP 卡一致,父级 `cancel_action` 把拒绝回传给模型。

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Border, ChildView, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, OffsetPositioning,
    ParentElement, Radius, Stack, Text,
};
use warpui::keymap::FixedBinding;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::ai::agent::{AIAgentActionId, AIAgentActionResultType, CallMCPToolResult, icons};
use crate::ai::agent_providers::tools::project_hosts::{self, BatchArgs};
use crate::ai::blocklist::action_model::{AIActionStatus, BlocklistAIActionModel};
use crate::ai::blocklist::block::AIBlock;
use crate::ai::blocklist::block::model::AIBlockModel;
use crate::ai::blocklist::block::view_impl::WithContentItemSpacing;
use crate::ai::blocklist::inline_action::inline_action_header::{HeaderConfig, InteractionMode};
use crate::ai::blocklist::inline_action::inline_action_icons;
use crate::ai::blocklist::inline_action::requested_action::{
    CTRL_C_KEYSTROKE, ENTER_KEYSTROKE, render_requested_action_row_for_text,
};
use crate::appearance::Appearance;
use crate::menu::{Event as MenuEvent, Menu, MenuItemFields, MenuVariant};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ButtonSize, KeystrokeSource, NakedTheme};
use crate::view_components::compactible_action_button::{
    CompactibleActionButton, MEDIUM_SIZE_SWITCH_THRESHOLD, RenderCompactibleActionButton,
};
use crate::view_components::compactible_split_action_button::CompactibleSplitActionButton;

/// 主机清单展示行数上限(与工具参数上限一致,防御性再钳制一次)。
pub(crate) const MAX_HOST_ROWS: usize = project_hosts::MAX_NODE_IDS;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            BatchCommandViewAction::Accept,
            id!(BatchCommandView::ui_name()),
        ),
        FixedBinding::new(
            "numpadenter",
            BatchCommandViewAction::Accept,
            id!(BatchCommandView::ui_name()),
        ),
        FixedBinding::new(
            "ctrl-c",
            BatchCommandViewAction::Reject,
            id!(BatchCommandView::ui_name()),
        ),
    ]);
}

/// 主机连接端点(`user@host:port` 的结构化形式)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostEndpoint {
    pub(crate) username: String,
    pub(crate) host: String,
    pub(crate) port: u16,
}

/// 单台主机的展示行数据:`name`/`endpoint` 任一缺失都可表达(悬空 node_id)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostRow {
    pub(crate) node_id: String,
    pub(crate) name: Option<String>,
    pub(crate) endpoint: Option<HostEndpoint>,
}

/// `user@host:port` 展示串。
pub(crate) fn format_endpoint(endpoint: &HostEndpoint) -> String {
    let HostEndpoint {
        username,
        host,
        port,
    } = endpoint;
    format!("{username}@{host}:{port}")
}

/// 主机行文案:优先「名称 — user@host:port」;server 记录缺失(悬空 node_id)
/// 时回退「node_id + 未知主机标注」。`unknown_label` 由调用方经 `crate::t!`
/// 注入,保证本函数纯可测。
pub(crate) fn host_row_text(row: &HostRow, unknown_label: &str) -> String {
    let display_name = row.name.as_deref().unwrap_or(row.node_id.as_str());
    match &row.endpoint {
        Some(endpoint) => format!("{display_name} — {}", format_endpoint(endpoint)),
        None => format!("{display_name} {unknown_label}"),
    }
}

/// args → 展示行的纯映射:按 `node_ids` 顺序逐个查名字与端点,查不到的字段
/// 置 `None`(渲染层据此显示未知主机回退)。
pub(crate) fn build_host_rows(
    node_ids: &[String],
    name_by_id: &HashMap<String, String>,
    endpoint_by_id: &HashMap<String, HostEndpoint>,
) -> Vec<HostRow> {
    node_ids
        .iter()
        .map(|node_id| HostRow {
            node_id: node_id.clone(),
            name: name_by_id.get(node_id).cloned(),
            endpoint: endpoint_by_id.get(node_id).cloned(),
        })
        .collect()
}

/// 展示行钳制:返回可见行切片与被截断的行数。
pub(crate) fn capped_host_rows(rows: &[HostRow]) -> (&[HostRow], usize) {
    if rows.len() > MAX_HOST_ROWS {
        (&rows[..MAX_HOST_ROWS], rows.len() - MAX_HOST_ROWS)
    } else {
        (rows, 0)
    }
}

/// 批量执行结束后的展示结论。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BatchOutcome {
    /// 聚合状态 ok;`counts` 为 `(成功台数, 总台数)`,payload 缺失时为 `None`。
    Success {
        counts: Option<(usize, usize)>,
    },
    /// 聚合状态 error(逐主机失败/金丝雀中止)。
    Failed {
        counts: Option<(usize, usize)>,
    },
    /// 传输层 / 参数错误(`CallMCPToolResult::Error`)。
    Error(String),
    Cancelled,
}

/// 解析执行端聚合 JSON(`session_router::aggregate_results` 的形状:
/// `{"status": "ok"|"error"|"cancelled", "results": [{"status": ...}, ...]}`)。
/// 文本不可解析时按无计数成功兜底(payload 已在结果树/历史里完整保留)。
pub(crate) fn parse_batch_summary(text: &str) -> BatchOutcome {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return BatchOutcome::Success { counts: None };
    };
    let counts = value
        .get("results")
        .and_then(serde_json::Value::as_array)
        .map(|results| {
            let total = results.len();
            let ok = results
                .iter()
                .filter(|result| {
                    result.get("status").and_then(serde_json::Value::as_str) == Some("ok")
                })
                .count();
            (ok, total)
        });
    match value.get("status").and_then(serde_json::Value::as_str) {
        Some("ok") => BatchOutcome::Success { counts },
        Some("cancelled") => BatchOutcome::Cancelled,
        // "error" 与未知状态一律按失败展示。
        Some(other_status) => {
            let _ = other_status;
            BatchOutcome::Failed { counts }
        }
        None => BatchOutcome::Success { counts },
    }
}

/// `CallMCPToolResult` → 展示结论。Success 时从 MCP text content 里取聚合 JSON。
pub(crate) fn batch_outcome(result: &CallMCPToolResult) -> BatchOutcome {
    match result {
        CallMCPToolResult::Success { result } => {
            let text = result
                .content
                .iter()
                .filter_map(|content| {
                    if let rmcp::model::RawContent::Text(text_content) = &content.raw {
                        Some(text_content.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<&str>>()
                .join("\n");
            parse_batch_summary(&text)
        }
        CallMCPToolResult::Error(error) => BatchOutcome::Error(error.clone()),
        CallMCPToolResult::Cancelled => BatchOutcome::Cancelled,
    }
}

/// 逐个 node_id 解析展示信息;DB 访问失败时整体按“未知主机”回退。
fn resolve_host_rows(node_ids: &[String]) -> Vec<HostRow> {
    let resolved = warp_ssh_manager::with_conn(|conn| {
        let nodes = warp_ssh_manager::SshRepository::list_nodes(conn)?;
        let name_by_id: HashMap<String, String> =
            nodes.into_iter().map(|node| (node.id, node.name)).collect();
        let mut endpoint_by_id: HashMap<String, HostEndpoint> =
            HashMap::with_capacity(node_ids.len());
        for node_id in node_ids {
            if let Some(server) = warp_ssh_manager::SshRepository::get_server(conn, node_id)? {
                endpoint_by_id.insert(
                    node_id.clone(),
                    HostEndpoint {
                        username: server.username,
                        host: server.host,
                        port: server.port,
                    },
                );
            }
        }
        Ok((name_by_id, endpoint_by_id))
    });
    match resolved {
        Ok((name_by_id, endpoint_by_id)) => build_host_rows(node_ids, &name_by_id, &endpoint_by_id),
        Err(error) => {
            log::warn!("run_command_on_hosts 主机展示信息解析失败,按未知主机回退:{error}");
            build_host_rows(node_ids, &HashMap::new(), &HashMap::new())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchCommandViewAction {
    Accept,
    /// 「始终允许」:执行并把命令写入自动执行 allowlist。
    AcceptAndAutoExecute,
    ToggleAcceptMenu,
    Reject,
}

#[derive(Clone, Debug)]
pub enum BatchCommandViewEvent {
    /// 仅执行本次。
    Accepted,
    /// 执行本次并把 `command` 加入命令自动执行 allowlist。
    AcceptedAndAllowlisted { command: String },
    /// 拒绝,父级走 `cancel_action` 把结果回传给模型。
    Rejected,
}

pub struct BatchCommandView {
    action_id: AIAgentActionId,
    action_model: ModelHandle<BlocklistAIActionModel>,
    block_model: Rc<dyn AIBlockModel<View = AIBlock>>,
    /// 最近一次成功解析的批量参数;流式早期 / 参数非法时为 `None`。
    args: Option<BatchArgs>,
    /// 与 `args.node_ids` 对齐的主机展示行。
    host_rows: Vec<HostRow>,

    reject_button: CompactibleActionButton,
    accept_split_button: CompactibleSplitActionButton,
    is_accept_menu_open: bool,
    accept_menu: ViewHandle<Menu<BatchCommandViewAction>>,
    position_id_prefix: String,
}

impl BatchCommandView {
    pub fn new(
        action_id: AIAgentActionId,
        action_model: ModelHandle<BlocklistAIActionModel>,
        block_model: Rc<dyn AIBlockModel<View = AIBlock>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let reject_button = CompactibleActionButton::new(
            crate::t!("common-reject"),
            Some(KeystrokeSource::Fixed(CTRL_C_KEYSTROKE.clone())),
            ButtonSize::InlineActionHeader,
            BatchCommandViewAction::Reject,
            Icon::X,
            Arc::new(NakedTheme),
            ctx,
        );
        let position_id_prefix = format!("{action_id:?}");
        let accept_split_button = CompactibleSplitActionButton::new(
            crate::t!("ai-batch-command-run"),
            Some(KeystrokeSource::Fixed(ENTER_KEYSTROKE.clone())),
            ButtonSize::InlineActionHeader,
            BatchCommandViewAction::Accept,
            BatchCommandViewAction::ToggleAcceptMenu,
            Icon::Check,
            true,
            Some(Self::accept_split_button_position_id(&position_id_prefix)),
            ctx,
        );

        let accept_menu = ctx.add_typed_action_view(|ctx| {
            let theme = Appearance::as_ref(ctx).theme();
            Menu::new()
                .with_menu_variant(MenuVariant::Fixed)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&accept_menu, |me, _menu, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.is_accept_menu_open = false;
                ctx.notify();
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        // 本 action 的任何状态迁移(入队 / Blocked / 执行中 / 完成)都重绘,
        // 使卡片在占位 → 确认卡 → 状态行之间切换。
        let action_id_for_events = action_id.clone();
        ctx.subscribe_to_model(&action_model, move |_me, _, event, ctx| {
            if *event.action_id() == action_id_for_events {
                ctx.notify();
            }
        });

        Self {
            action_id,
            action_model,
            block_model,
            args: None,
            host_rows: Vec::new(),
            reject_button,
            accept_split_button,
            is_accept_menu_open: false,
            accept_menu,
            position_id_prefix,
        }
    }

    /// 应用流式参数更新:能解析则同步刷新主机展示行,解析失败保持现状
    /// (流式 JSON 尚不完整属常态)。
    pub fn update_args(&mut self, input: &serde_json::Value, ctx: &mut ViewContext<Self>) {
        let parsed = project_hosts::parse_batch_args(input).ok();
        if parsed.is_none() || parsed == self.args {
            return;
        }
        if let Some(args) = &parsed {
            self.host_rows = resolve_host_rows(&args.node_ids);
        }
        self.args = parsed;
        ctx.notify();
    }

    fn is_blocked(&self, app: &AppContext) -> bool {
        matches!(
            self.action_model
                .as_ref(app)
                .get_action_status(&self.action_id),
            Some(AIActionStatus::Blocked)
        )
    }

    fn toggle_accept_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_accept_menu_open = !self.is_accept_menu_open;
        if self.is_accept_menu_open {
            let accept_item = MenuItemFields::new_with_label(
                crate::t!("ai-batch-command-run"),
                ENTER_KEYSTROKE.displayed(),
            )
            .with_on_select_action(BatchCommandViewAction::Accept)
            .into_item();
            let auto_item = MenuItemFields::new(crate::t!("ai-block-always-allow"))
                .with_on_select_action(BatchCommandViewAction::AcceptAndAutoExecute)
                .into_item();
            self.accept_menu.update(ctx, |menu, ctx| {
                menu.set_items(vec![accept_item, auto_item], ctx);
            });
            self.accept_menu
                .update(ctx, |menu, ctx| menu.set_selected_by_index(0, ctx));
            ctx.focus(&self.accept_menu);
        }
        ctx.notify();
    }

    fn accept_split_button_position_id(prefix: &str) -> String {
        format!("BatchCommandView-{prefix}-accept-split")
    }

    /// Blocked 状态的完整确认卡:头部(标题 + 金丝雀徽标 + 三按钮)+ 正文
    /// (命令 monospace 块 / 主机清单 / 超时)。
    fn render_confirmation_card(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut config = HeaderConfig::new(crate::t!("ai-batch-command-title"), app)
            .with_icon(icons::yellow_stop_icon(appearance))
            .with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(8.)));
        if self.args.as_ref().is_some_and(|args| args.canary) {
            config = config.with_badge(crate::t!("ai-batch-command-canary-badge"));
        }
        let action_buttons: Vec<Rc<dyn RenderCompactibleActionButton>> = vec![
            Rc::new(self.reject_button.clone()),
            Rc::new(self.accept_split_button.clone()),
        ];
        config = config.with_interaction_mode(InteractionMode::ActionButtons {
            action_buttons,
            size_switch_threshold: MEDIUM_SIZE_SWITCH_THRESHOLD,
        });
        let header = config.render(app);

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(self.render_body(app))
            .finish();

        Container::new(content)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_fill(theme.accent()))
            .finish()
            .with_content_item_spacing()
            .finish()
    }

    fn render_body(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        match &self.args {
            Some(args) => {
                // 命令本体:monospace 块(与 requested_command 的命令展示同风格)。
                let command_text = Text::new(
                    args.command.clone(),
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(blended_colors::text_main(theme, theme.background()))
                .with_selectable(true)
                .finish();
                column.add_child(
                    Container::new(command_text)
                        .with_horizontal_padding(12.)
                        .with_vertical_padding(8.)
                        .with_background_color(blended_colors::neutral_2(theme))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                        .with_margin_bottom(12.)
                        .finish(),
                );

                // 主机清单:名称 + user@host:port,悬空 node_id 显示未知主机回退。
                let unknown_label = crate::t!("ai-batch-command-unknown-host");
                let hosts_label = Text::new(
                    crate::t!("ai-batch-command-hosts-label", count = self.host_rows.len()),
                    appearance.ui_font_family(),
                    appearance.monospace_font_size() - 1.,
                )
                .with_color(blended_colors::text_disabled(theme, theme.background()))
                .finish();
                let mut hosts_column = Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(Container::new(hosts_label).with_margin_bottom(6.).finish());
                let (visible_rows, hidden_count) = capped_host_rows(&self.host_rows);
                for row in visible_rows {
                    hosts_column.add_child(
                        Container::new(
                            Text::new(
                                host_row_text(row, &unknown_label),
                                appearance.monospace_font_family(),
                                appearance.monospace_font_size() - 1.,
                            )
                            .with_color(blended_colors::text_main(theme, theme.background()))
                            .with_selectable(true)
                            .finish(),
                        )
                        .with_margin_bottom(2.)
                        .finish(),
                    );
                }
                if hidden_count > 0 {
                    hosts_column.add_child(
                        Text::new(
                            crate::t!("ai-batch-command-hosts-more", count = hidden_count),
                            appearance.ui_font_family(),
                            appearance.monospace_font_size() - 1.,
                        )
                        .with_color(blended_colors::text_disabled(theme, theme.background()))
                        .finish(),
                    );
                }
                column.add_child(
                    Container::new(hosts_column.finish())
                        .with_margin_bottom(12.)
                        .finish(),
                );

                // 超时展示。
                column.add_child(
                    Text::new(
                        crate::t!("ai-batch-command-timeout", seconds = args.timeout_seconds),
                        appearance.ui_font_family(),
                        appearance.monospace_font_size() - 1.,
                    )
                    .with_color(blended_colors::text_disabled(theme, theme.background()))
                    .finish(),
                );
            }
            None => {
                // 理论上不可达:Blocked 前执行端已完成参数校验;流式中途本卡
                // 尚未进入 Blocked 分支。仍留兜底文案避免空白卡。
                column.add_child(
                    Text::new(
                        crate::t!("ai-batch-command-args-pending"),
                        appearance.ui_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(blended_colors::text_disabled(theme, theme.background()))
                    .finish(),
                );
            }
        }

        Container::new(column.finish())
            .with_horizontal_padding(16.)
            .with_vertical_padding(12.)
            .with_background_color(theme.background().into_solid())
            .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(8.)))
            .finish()
    }

    /// 终态 / 执行中的单行状态卡(与 `RunAgentsCardView` 的状态行同风格)。
    fn render_status_card(
        label: String,
        icon: Box<dyn Element>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let row = render_requested_action_row_for_text(
            label.into(),
            appearance.ui_font_family(),
            Some(icon),
            None,
            false,
            false,
            app,
        );
        Container::new(row)
            .with_background_color(blended_colors::neutral_2(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish()
            .with_agent_output_item_spacing(app)
            .finish()
    }

    fn render_finished_card(result: &CallMCPToolResult, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let (label, icon) = match batch_outcome(result) {
            BatchOutcome::Success { counts } => (
                match counts {
                    Some((ok, total)) => {
                        crate::t!("ai-batch-command-finished-counts", ok = ok, total = total)
                    }
                    None => crate::t!("ai-batch-command-finished"),
                },
                inline_action_icons::green_check_icon(appearance).finish(),
            ),
            BatchOutcome::Failed { counts } => (
                match counts {
                    Some((ok, total)) => {
                        crate::t!("ai-batch-command-failed-counts", ok = ok, total = total)
                    }
                    None => crate::t!("ai-batch-command-failed"),
                },
                inline_action_icons::red_x_icon(appearance).finish(),
            ),
            BatchOutcome::Error(error) => (
                crate::t!("ai-batch-command-error", error = error),
                inline_action_icons::red_x_icon(appearance).finish(),
            ),
            BatchOutcome::Cancelled => (
                crate::t!("ai-tool-call-cancelled"),
                inline_action_icons::cancelled_icon(appearance).finish(),
            ),
        };
        Self::render_status_card(label, icon, app)
    }
}

impl Entity for BatchCommandView {
    type Event = BatchCommandViewEvent;
}

impl View for BatchCommandView {
    fn ui_name() -> &'static str {
        "BatchCommandView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action_id);

        if let Some(AIActionStatus::Finished(result)) = &status {
            if let AIAgentActionResultType::CallMCPTool(mcp_result) = &result.result {
                return Self::render_finished_card(mcp_result, app);
            }
            // 批量工具恒走 CallMCPTool 结果通道;其它结果类型说明上游接线错了。
            log::warn!(
                "BatchCommandView 收到非 CallMCPTool 结果类型:{:?}",
                result.result
            );
            return Empty::new().finish();
        }

        if matches!(status, Some(AIActionStatus::RunningAsync)) {
            let count = self
                .args
                .as_ref()
                .map(|args| args.node_ids.len())
                .unwrap_or_default();
            return Self::render_status_card(
                crate::t!("ai-batch-command-running", count = count),
                icons::yellow_running_icon(appearance).finish(),
                app,
            );
        }

        // 历史恢复:确认/执行状态已丢失,按取消展示(与 RunAgents 卡一致)。
        if self.block_model.is_restored() {
            return Self::render_status_card(
                crate::t!("ai-tool-call-cancelled"),
                inline_action_icons::cancelled_icon(appearance).finish(),
                app,
            );
        }

        // 仍在流式:占位行,等待 action 进入 Blocked。
        if !matches!(status, Some(AIActionStatus::Blocked)) {
            return Self::render_status_card(
                crate::t!("ai-batch-command-streaming"),
                icons::yellow_running_icon(appearance).finish(),
                app,
            );
        }

        let mut root_stack = Stack::new();
        root_stack.add_child(self.render_confirmation_card(app));

        if self.is_accept_menu_open {
            root_stack.add_positioned_child(
                ChildView::new(&self.accept_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    Self::accept_split_button_position_id(&self.position_id_prefix),
                    vec2f(0., 8.),
                    warpui::elements::PositionedElementOffsetBounds::WindowByPosition,
                    warpui::elements::PositionedElementAnchor::BottomRight,
                    warpui::elements::ChildAnchor::TopRight,
                ),
            );
        }

        root_stack.finish()
    }
}

impl TypedActionView for BatchCommandView {
    type Action = BatchCommandViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BatchCommandViewAction::Accept => {
                if !self.is_blocked(ctx) {
                    return;
                }
                ctx.emit(BatchCommandViewEvent::Accepted);
            }
            BatchCommandViewAction::AcceptAndAutoExecute => {
                if !self.is_blocked(ctx) {
                    return;
                }
                match &self.args {
                    Some(args) => ctx.emit(BatchCommandViewEvent::AcceptedAndAllowlisted {
                        command: args.command.clone(),
                    }),
                    // 参数未就绪时退化为普通接受(不写 allowlist)。
                    None => ctx.emit(BatchCommandViewEvent::Accepted),
                }
            }
            BatchCommandViewAction::ToggleAcceptMenu => self.toggle_accept_menu(ctx),
            BatchCommandViewAction::Reject => {
                if !self.is_blocked(ctx) {
                    return;
                }
                ctx.emit(BatchCommandViewEvent::Rejected);
            }
        }
    }
}

#[cfg(test)]
#[path = "batch_command_view_tests.rs"]
mod tests;
