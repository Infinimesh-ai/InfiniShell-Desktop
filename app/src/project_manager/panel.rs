//! 项目管理器主 panel — 左侧 Tool Panel 内容:项目列表 + 可展开的关联主机行
//! + 顶部「新建项目」按钮 + 每行「从项目发起 Agent 对话」快捷按钮。
//!
//! MVP 交互规则:
//! - **单击项目行**:选中 + emit `OpenProjectDetail`(中央区打开详情编辑,
//!   part B 落地;当前 workspace 侧为 stub)。
//! - **单击 chevron**:折叠/展开该项目的关联主机列表(纯 UI 态,不持久化)。
//! - **单击主机行**:emit `OpenHostSession`(workspace 复用 SSH 连接链路)。
//! - **重命名/编辑字段不在本面板做** — 全部走详情编辑器。
//!
//! 数据层在独立 crate `zap_projects`;主机显示信息经 `warp_ssh_manager` 解析,
//! 悬挂引用(主机已被删除)在加载时静默过滤。

use std::collections::{HashMap, HashSet};

use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_ssh_manager::{NodeKind, SshRepository, SshServerInfo};
use warpui::elements::{
    Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Empty, Fill as ElementFill, Flex, Hoverable, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};
use zap_projects::{Project, ProjectRepository};

use crate::project_manager::{ProjectsChangedEvent, ProjectsChangedNotifier};
use crate::ui_components::buttons::icon_button;

// ---- 视觉常量(对齐 ssh_manager/panel.rs / skill_manager/panel.rs) ----
const PANEL_PADDING: f32 = 8.0;
const ITEM_PADDING_VERTICAL: f32 = 5.0;
const ITEM_PADDING_HORIZONTAL: f32 = 8.0;
const ITEM_ICON_TEXT_SPACING: f32 = 8.0;
const ITEM_ICON_SIZE: f32 = 14.0;
const HOST_ROW_INDENT: f32 = 16.0;

#[derive(Clone, Debug)]
pub enum ProjectManagerPanelAction {
    /// 顶部「新建项目」按钮:用默认名建项目,随后直接进详情编辑。
    CreateProject,
    /// 单击 chevron:折叠/展开该项目的关联主机列表。
    ToggleExpanded(String),
    /// 单击项目行:选中 + 打开详情。
    OpenProject(String),
    /// 单击主机行:请求按 SSH 链路连接该主机。
    OpenHost { project_id: String, node_id: String },
    /// 行尾按钮「从项目发起 Agent 对话」。
    StartConversation(String),
}

#[derive(Clone, Debug)]
pub enum ProjectManagerPanelEvent {
    /// 用户点击项目行 / 新建项目后,中央 pane 应打开该项目的详情编辑。
    OpenProjectDetail {
        project_id: String,
    },
    /// 用户点击项目下的主机行,请求开 terminal pane 跑 ssh(复用
    /// `Workspace::open_ssh_terminal` 链路)。
    OpenHostSession {
        project_id: String,
        node_id: String,
        server: SshServerInfo,
    },
    /// 用户点击「从项目发起 Agent 对话」(M3 落地;当前 workspace 侧为 stub)。
    StartProjectConversation {
        project_id: String,
    },
    PersistenceError(String),
}

/// 项目行下方展示的关联主机 — 加载时已过滤悬挂引用并解析好显示字段。
#[derive(Clone, Debug)]
pub struct ProjectHostRow {
    pub node_id: String,
    /// SSH 树节点名称(用户命名)。
    pub name: String,
    pub server: SshServerInfo,
}

pub struct ProjectManagerPanel {
    projects: Vec<Project>,
    /// project_id → 已解析的关联主机行。
    hosts_by_project: HashMap<String, Vec<ProjectHostRow>>,
    /// 展开态项目 id 集合(纯 UI 态,不持久化)。
    expanded_ids: HashSet<String>,
    selected_id: Option<String>,

    new_project_btn: MouseStateHandle,
    row_states: HashMap<String, MouseStateHandle>,
    chevron_states: HashMap<String, MouseStateHandle>,
    conversation_btn_states: HashMap<String, MouseStateHandle>,
    /// key = `"{project_id}/{node_id}"`(主机可被多个项目关联,须带项目前缀)。
    host_row_states: HashMap<String, MouseStateHandle>,

    list_scroll_state: ClippedScrollStateHandle,
}

impl ProjectManagerPanel {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let mut me = Self {
            projects: Vec::new(),
            hosts_by_project: HashMap::new(),
            expanded_ids: HashSet::new(),
            selected_id: None,
            new_project_btn: MouseStateHandle::default(),
            row_states: HashMap::new(),
            chevron_states: HashMap::new(),
            conversation_btn_states: HashMap::new(),
            host_row_states: HashMap::new(),
            list_scroll_state: ClippedScrollStateHandle::default(),
        };
        me.reload(ctx);

        ctx.subscribe_to_model(
            &ProjectsChangedNotifier::handle(ctx),
            |me, _, event, ctx| match event {
                ProjectsChangedEvent::ProjectsChanged => me.reload(ctx),
            },
        );

        ctx.subscribe_to_model(&Appearance::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        me
    }

    /// 重新读项目列表 + 关联主机。任一步失败保留旧数据并 emit
    /// `PersistenceError`。
    fn reload(&mut self, ctx: &mut ViewContext<Self>) {
        match load_projects_with_hosts() {
            Ok((projects, hosts_by_project)) => {
                self.projects = projects;
                self.hosts_by_project = hosts_by_project;
                if let Some(id) = self.selected_id.clone() {
                    if !self.projects.iter().any(|p| p.id == id) {
                        self.selected_id = None;
                    }
                }
                self.sync_mouse_states();
            }
            Err(e) => {
                log::error!("project_manager: failed to load projects: {e:?}");
                ctx.emit(ProjectManagerPanelEvent::PersistenceError(e.to_string()));
            }
        }
        ctx.notify();
    }

    /// 让各 hover-state map 的 key 集合与当前数据一致:多余的删掉(释放内存),
    /// 缺的补默认值。展开集合同步剪掉已删项目。
    fn sync_mouse_states(&mut self) {
        let project_ids: HashSet<&str> = self.projects.iter().map(|p| p.id.as_str()).collect();
        self.expanded_ids
            .retain(|id| project_ids.contains(id.as_str()));
        self.row_states
            .retain(|k, _| project_ids.contains(k.as_str()));
        self.chevron_states
            .retain(|k, _| project_ids.contains(k.as_str()));
        self.conversation_btn_states
            .retain(|k, _| project_ids.contains(k.as_str()));
        for project in &self.projects {
            self.row_states.entry(project.id.clone()).or_default();
            self.chevron_states.entry(project.id.clone()).or_default();
            self.conversation_btn_states
                .entry(project.id.clone())
                .or_default();
        }

        let host_keys: HashSet<String> = self
            .hosts_by_project
            .iter()
            .flat_map(|(project_id, hosts)| {
                hosts
                    .iter()
                    .map(move |host| host_row_key(project_id, &host.node_id))
            })
            .collect();
        self.host_row_states.retain(|k, _| host_keys.contains(k));
        for key in host_keys {
            self.host_row_states.entry(key).or_default();
        }
    }

    fn on_create_project(&mut self, ctx: &mut ViewContext<Self>) {
        let name = crate::t!("project-manager-default-name").to_string();
        let result = zap_projects::with_conn(|conn| Ok(ProjectRepository::create(conn, &name)?));
        match result {
            Ok(project) => {
                let project_id = project.id.clone();
                self.selected_id = Some(project_id.clone());
                self.reload(ctx);
                ProjectsChangedNotifier::notify_changed(ctx);
                // 新建后直接进详情编辑,让用户改名/填字段。
                ctx.emit(ProjectManagerPanelEvent::OpenProjectDetail { project_id });
            }
            Err(e) => {
                log::error!("project_manager: create project failed: {e:?}");
                ctx.emit(ProjectManagerPanelEvent::PersistenceError(e.to_string()));
            }
        }
    }

    fn on_toggle_expanded(&mut self, project_id: &str, ctx: &mut ViewContext<Self>) {
        if !self.expanded_ids.remove(project_id) {
            self.expanded_ids.insert(project_id.to_string());
        }
        ctx.notify();
    }

    fn on_open_project(&mut self, project_id: String, ctx: &mut ViewContext<Self>) {
        self.selected_id = Some(project_id.clone());
        ctx.emit(ProjectManagerPanelEvent::OpenProjectDetail { project_id });
        ctx.notify();
    }

    fn on_open_host(&mut self, project_id: &str, node_id: &str, ctx: &mut ViewContext<Self>) {
        let host = self
            .hosts_by_project
            .get(project_id)
            .and_then(|hosts| hosts.iter().find(|h| h.node_id == node_id));
        if let Some(host) = host {
            ctx.emit(ProjectManagerPanelEvent::OpenHostSession {
                project_id: project_id.to_string(),
                node_id: node_id.to_string(),
                server: host.server.clone(),
            });
        }
    }

    // ---- 渲染 --------------------------------------------------------------

    fn render_label(
        text: impl Into<String>,
        appearance: &Appearance,
        font_size: f32,
        color: impl Into<pathfinder_color::ColorU>,
    ) -> Box<dyn Element> {
        Text::new_inline(text.into(), appearance.ui_font_family(), font_size)
            .with_color(color.into())
            .with_clip(ClipConfig::ellipsis())
            .finish()
    }

    /// 顶部工具条:整行宽的「新建项目」按钮(Plus 图标 + 文案)。
    fn render_toolbar(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.background());
        let icon_el = ConstrainedBox::new(
            crate::ui_components::icons::Icon::Plus
                .to_warpui_icon(theme.sub_text_color(theme.background()))
                .finish(),
        )
        .with_width(ITEM_ICON_SIZE)
        .with_height(ITEM_ICON_SIZE)
        .finish();
        let label = Self::render_label(
            crate::t!("project-manager-new-project"),
            appearance,
            appearance.ui_font_subheading(),
            text_color,
        );

        Hoverable::new(self.new_project_btn.clone(), move |mouse| {
            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(ITEM_ICON_TEXT_SPACING)
                .with_child(icon_el)
                .with_child(label)
                .finish();
            let mut button = Container::new(row)
                .with_padding_top(ITEM_PADDING_VERTICAL)
                .with_padding_bottom(ITEM_PADDING_VERTICAL)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if mouse.is_hovered() {
                button = button.with_background(internal_colors::fg_overlay_2(theme));
            }
            button.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ProjectManagerPanelAction::CreateProject);
        })
        .finish()
    }

    fn render_project_row(
        &self,
        project: &Project,
        is_selected: bool,
        is_expanded: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon_color = theme.sub_text_color(theme.background());

        // chevron:展开态 ▼ / 折叠态 ▶,单击仅切换展开,不打开详情。
        let chevron_icon = if is_expanded {
            crate::ui_components::icons::Icon::ChevronDown
        } else {
            crate::ui_components::icons::Icon::ChevronRight
        };
        let chevron_el = ConstrainedBox::new(chevron_icon.to_warpui_icon(icon_color).finish())
            .with_width(ITEM_ICON_SIZE)
            .with_height(ITEM_ICON_SIZE)
            .finish();
        let chevron_state = self
            .chevron_states
            .get(&project.id)
            .cloned()
            .unwrap_or_default();
        let project_id_for_toggle = project.id.clone();
        let chevron_btn = Hoverable::new(chevron_state, move |_| {
            Container::new(chevron_el)
                .with_uniform_padding(2.0)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ProjectManagerPanelAction::ToggleExpanded(
                project_id_for_toggle.clone(),
            ));
        })
        .finish();

        let name_label = Self::render_label(
            project.name.clone(),
            appearance,
            appearance.ui_font_subheading(),
            theme.main_text_color(theme.background()),
        );

        // 行尾「从项目发起 Agent 对话」按钮,带 tooltip。
        let conversation_state = self
            .conversation_btn_states
            .get(&project.id)
            .cloned()
            .unwrap_or_default();
        let tooltip = appearance
            .ui_builder()
            .clone()
            .tool_tip(crate::t!("project-manager-start-conversation"))
            .build()
            .finish();
        let project_id_for_conversation = project.id.clone();
        let conversation_btn = icon_button(
            appearance,
            crate::ui_components::icons::Icon::AgentMode,
            false,
            conversation_state,
        )
        .with_tooltip(move || tooltip)
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ProjectManagerPanelAction::StartConversation(
                project_id_for_conversation.clone(),
            ));
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        let row_state = self
            .row_states
            .get(&project.id)
            .cloned()
            .unwrap_or_default();
        let project_id_for_open = project.id.clone();
        Hoverable::new(row_state, move |mouse| {
            let background = if is_selected && mouse.is_hovered() {
                Some(internal_colors::fg_overlay_4(theme))
            } else if is_selected {
                Some(internal_colors::fg_overlay_3(theme))
            } else if mouse.is_hovered() {
                Some(internal_colors::fg_overlay_2(theme))
            } else {
                None
            };
            let content = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(ITEM_ICON_TEXT_SPACING)
                .with_child(chevron_btn)
                .with_child(Shrinkable::new(1.0, Clipped::new(name_label).finish()).finish())
                .with_child(conversation_btn)
                .finish();
            let mut row = Container::new(content)
                .with_padding_top(ITEM_PADDING_VERTICAL)
                .with_padding_bottom(ITEM_PADDING_VERTICAL)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if let Some(background) = background {
                row = row.with_background(background);
            }
            row.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ProjectManagerPanelAction::OpenProject(
                project_id_for_open.clone(),
            ));
        })
        .finish()
    }

    fn render_host_row(
        &self,
        project_id: &str,
        host: &ProjectHostRow,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let main = theme.main_text_color(theme.background());
        let muted = theme.sub_text_color(theme.background());

        let icon_el = ConstrainedBox::new(
            crate::ui_components::icons::Icon::Server01
                .to_warpui_icon(muted)
                .finish(),
        )
        .with_width(ITEM_ICON_SIZE)
        .with_height(ITEM_ICON_SIZE)
        .finish();

        let name_label = Self::render_label(
            host.name.clone(),
            appearance,
            appearance.ui_font_body(),
            main,
        );
        let subtitle_label = Self::render_label(
            host_subtitle(&host.server),
            appearance,
            appearance.ui_font_footnote(),
            muted,
        );

        let state = self
            .host_row_states
            .get(&host_row_key(project_id, &host.node_id))
            .cloned()
            .unwrap_or_default();
        let project_id_for_click = project_id.to_string();
        let node_id_for_click = host.node_id.clone();
        Hoverable::new(state, move |mouse| {
            let label_col = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(name_label)
                .with_child(subtitle_label)
                .with_main_axis_size(MainAxisSize::Min)
                .finish();
            let content = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(ITEM_ICON_TEXT_SPACING)
                .with_child(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(HOST_ROW_INDENT)
                        .finish(),
                )
                .with_child(icon_el)
                .with_child(Shrinkable::new(1.0, Clipped::new(label_col).finish()).finish())
                .finish();
            let mut row = Container::new(content)
                .with_padding_top(ITEM_PADDING_VERTICAL)
                .with_padding_bottom(ITEM_PADDING_VERTICAL)
                .with_padding_left(ITEM_PADDING_HORIZONTAL)
                .with_padding_right(ITEM_PADDING_HORIZONTAL)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if mouse.is_hovered() {
                row = row.with_background(internal_colors::fg_overlay_2(theme));
            }
            row.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ProjectManagerPanelAction::OpenHost {
                project_id: project_id_for_click.clone(),
                node_id: node_id_for_click.clone(),
            });
        })
        .finish()
    }

    fn render_project_list(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        if self.projects.is_empty() {
            return Container::new(Self::render_label(
                crate::t!("project-manager-empty"),
                appearance,
                appearance.ui_font_body(),
                theme.sub_text_color(theme.background()),
            ))
            .with_uniform_padding(12.0)
            .finish();
        }

        let mut rows = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for project in &self.projects {
            let is_selected = self.selected_id.as_deref() == Some(project.id.as_str());
            let is_expanded = self.expanded_ids.contains(&project.id);
            rows.add_child(self.render_project_row(project, is_selected, is_expanded, appearance));
            if is_expanded {
                let hosts = self
                    .hosts_by_project
                    .get(&project.id)
                    .map(|hosts| hosts.as_slice())
                    .unwrap_or_default();
                if hosts.is_empty() {
                    // 展开但无关联主机 → muted 提示行,避免展开无反馈。
                    rows.add_child(
                        Container::new(Self::render_label(
                            crate::t!("project-manager-no-hosts"),
                            appearance,
                            appearance.ui_font_footnote(),
                            theme.sub_text_color(theme.background()),
                        ))
                        .with_padding_top(ITEM_PADDING_VERTICAL)
                        .with_padding_bottom(ITEM_PADDING_VERTICAL)
                        .with_padding_left(
                            ITEM_PADDING_HORIZONTAL + HOST_ROW_INDENT + ITEM_ICON_SIZE,
                        )
                        .with_padding_right(ITEM_PADDING_HORIZONTAL)
                        .finish(),
                    );
                }
                for host in hosts {
                    rows.add_child(self.render_host_row(&project.id, host, appearance));
                }
            }
        }

        ClippedScrollable::vertical(
            self.list_scroll_state.clone(),
            rows.finish(),
            ScrollbarWidth::Auto,
            theme.disabled_text_color(theme.background()).into(),
            theme.main_text_color(theme.background()).into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish()
    }
}

impl TypedActionView for ProjectManagerPanel {
    type Action = ProjectManagerPanelAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ProjectManagerPanelAction::CreateProject => self.on_create_project(ctx),
            ProjectManagerPanelAction::ToggleExpanded(project_id) => {
                self.on_toggle_expanded(project_id, ctx);
            }
            ProjectManagerPanelAction::OpenProject(project_id) => {
                self.on_open_project(project_id.clone(), ctx);
            }
            ProjectManagerPanelAction::OpenHost {
                project_id,
                node_id,
            } => {
                self.on_open_host(project_id, node_id, ctx);
            }
            ProjectManagerPanelAction::StartConversation(project_id) => {
                ctx.emit(ProjectManagerPanelEvent::StartProjectConversation {
                    project_id: project_id.clone(),
                });
            }
        }
    }
}

impl View for ProjectManagerPanel {
    fn ui_name() -> &'static str {
        "ProjectManagerPanel"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(4.0)
                .with_child(self.render_toolbar(appearance))
                .with_child(Shrinkable::new(1.0, self.render_project_list(appearance)).finish())
                .finish(),
        )
        .with_uniform_padding(PANEL_PADDING)
        .finish()
    }
}

impl Entity for ProjectManagerPanel {
    type Event = ProjectManagerPanelEvent;
}

// ---- 纯函数(供 panel_tests.rs 直接测试) ----------------------------------

/// hover-state map 的主机行 key:主机可被多个项目关联,须带项目前缀。
fn host_row_key(project_id: &str, node_id: &str) -> String {
    format!("{project_id}/{node_id}")
}

/// 主机行副标题 "user@host:port";user 为空时省略 "user@"。
fn host_subtitle(server: &SshServerInfo) -> String {
    let host = &server.host;
    let port = server.port;
    let username = &server.username;
    if username.is_empty() {
        format!("{host}:{port}")
    } else {
        format!("{username}@{host}:{port}")
    }
}

/// 把项目关联的 node_id 列表解析为可显示的主机行,保持关联表里的顺序;
/// 悬挂引用(节点已删 / 已不是 server)被静默过滤。
fn resolve_host_rows(
    node_ids: &[String],
    servers_by_node: &HashMap<String, (String, SshServerInfo)>,
) -> Vec<ProjectHostRow> {
    node_ids
        .iter()
        .filter_map(|node_id| {
            servers_by_node
                .get(node_id)
                .map(|(name, server)| ProjectHostRow {
                    node_id: node_id.clone(),
                    name: name.clone(),
                    server: server.clone(),
                })
        })
        .collect()
}

/// 一次性读全:项目列表 + 每个项目的已解析主机行。
fn load_projects_with_hosts() -> anyhow::Result<(Vec<Project>, HashMap<String, Vec<ProjectHostRow>>)>
{
    let (projects, links) = zap_projects::with_conn(|conn| {
        let projects = ProjectRepository::list(conn)?;
        let mut links: HashMap<String, Vec<String>> = HashMap::new();
        for project in &projects {
            links.insert(
                project.id.clone(),
                ProjectRepository::servers_for_project(conn, &project.id)?,
            );
        }
        Ok((projects, links))
    })?;

    // node_id → (节点名, server 连接信息)。folder 节点与查不到 server info 的
    // 节点一并跳过 —— 与"悬挂引用过滤"共用同一条路径。
    let servers_by_node = warp_ssh_manager::with_conn(|conn| {
        let nodes = SshRepository::list_nodes(conn)?;
        let mut map: HashMap<String, (String, SshServerInfo)> = HashMap::new();
        for node in nodes {
            if !matches!(node.kind, NodeKind::Server) {
                continue;
            }
            if let Some(server) = SshRepository::get_server(conn, &node.id)? {
                map.insert(node.id.clone(), (node.name, server));
            }
        }
        Ok(map)
    })?;

    let hosts_by_project = links
        .into_iter()
        .map(|(project_id, node_ids)| {
            let hosts = resolve_host_rows(&node_ids, &servers_by_node);
            (project_id, hosts)
        })
        .collect();
    Ok((projects, hosts_by_project))
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
