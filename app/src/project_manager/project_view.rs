//! 项目详情编辑器中央 pane 的 BackingView 实现(M2 part B)。
//!
//! 表单字段:名称 / Git 地址 / 本地目录 / 项目规则 / 备注 + 关联服务器
//! 勾选列表。「保存」写 `infinishell_projects` 数据层并广播
//! [`ProjectsChangedNotifier`];「删除项目」走 confirmation → 软删 → 关 pane。
//!
//! default_profile_id 字段 MVP 阶段不提供 UI(执行 profile 集成在 M3),
//! 保存时原样透传已加载的值。

use std::collections::HashSet;

use infinishell_projects::{Project, ProjectRepository};
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_ssh_manager::{NodeKind, SshRepository};
use warpui::elements::{
    Align, Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Dismiss, Element, Fill, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, ScrollbarWidth, Stack, Text,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::editor::{
    EditorOptions, EditorView, EnterAction, EnterSettings, Event as EditorEvent,
    SingleLineEditorOptions, TextColors, TextOptions,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::project_manager::ProjectsChangedNotifier;

const FIELD_LABEL_MARGIN_TOP: f32 = 6.0;
const FIELD_LABEL_MARGIN_BOTTOM: f32 = 4.0;
const FIELD_BLOCK_MARGIN_BOTTOM: f32 = 12.0;
const SAVE_BUTTON_WIDTH: f32 = 96.0;
const SAVE_BUTTON_HEIGHT: f32 = 28.0;
const MULTILINE_FIELD_MIN_HEIGHT: f32 = 96.0;
const SERVER_ROW_CHECK_SIZE: f32 = 16.0;
const DELETE_DIALOG_WIDTH: f32 = 450.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectViewAction {
    Save,
    /// 切换第 index 台服务器(`self.servers[index]`)的关联状态。
    ToggleServer(usize),
    OpenDeleteConfirmation,
    CancelDeleteConfirmation,
    ConfirmDelete,
}

/// 一次性显示在表单顶部的状态标签。
#[derive(Debug, Clone)]
enum StatusBanner {
    Saved,
    Error(String),
}

/// 关联服务器候选行 — 加载时已把 SSH 树节点与 server 详情拍平成显示字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectServerRow {
    pub node_id: String,
    /// SSH 树节点名称(用户命名)。
    pub name: String,
    pub username: String,
    pub host: String,
    pub port: u16,
}

pub struct ProjectView {
    project_id: String,
    /// 缓存上次从 DB 读到的项目。已被删除/找不到时为 None,渲染占位提示。
    project: Option<Project>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,

    name_editor: ViewHandle<EditorView>,
    git_url_editor: ViewHandle<EditorView>,
    root_path_editor: ViewHandle<EditorView>,
    rules_editor: ViewHandle<EditorView>,
    notes_editor: ViewHandle<EditorView>,

    /// 全部 SSH 服务器候选(树顺序),供勾选关联。
    servers: Vec<ProjectServerRow>,
    /// 当前勾选的关联服务器 node_id 集合。保存时按 `servers` 的树顺序展开
    /// 成有序列表(见 [`linked_ids_in_tree_order`])。
    linked_node_ids: HashSet<String>,

    save_btn_state: MouseStateHandle,
    delete_btn_state: MouseStateHandle,
    delete_confirm_btn_state: MouseStateHandle,
    delete_cancel_btn_state: MouseStateHandle,
    server_row_states: Vec<MouseStateHandle>,

    show_delete_confirmation: bool,
    status: Option<StatusBanner>,
    scroll_state: ClippedScrollStateHandle,
}

impl ProjectView {
    pub fn new(project_id: String, ctx: &mut ViewContext<Self>) -> Self {
        let name_editor = make_single_line_editor(&crate::t!("common-name"), ctx);
        let git_url_editor = make_single_line_editor("https://github.com/example/repo.git", ctx);
        let root_path_editor = make_single_line_editor("/home/user/projects/app", ctx);
        let rules_editor = make_multiline_editor(&crate::t!("project-view-rules-placeholder"), ctx);
        let notes_editor = make_multiline_editor(&crate::t!("project-view-notes-placeholder"), ctx);

        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("Project"));

        let mut me = Self {
            project_id,
            project: None,
            pane_configuration,
            focus_handle: None,
            name_editor,
            git_url_editor,
            root_path_editor,
            rules_editor,
            notes_editor,
            servers: Vec::new(),
            linked_node_ids: HashSet::new(),
            save_btn_state: MouseStateHandle::default(),
            delete_btn_state: MouseStateHandle::default(),
            delete_confirm_btn_state: MouseStateHandle::default(),
            delete_cancel_btn_state: MouseStateHandle::default(),
            server_row_states: Vec::new(),
            show_delete_confirmation: false,
            status: None,
            scroll_state: ClippedScrollStateHandle::default(),
        };
        me.reload(ctx);

        // 监听每个 editor:编辑 → 清掉 status banner;失焦/切换字段时清
        // selection,防止多个输入框同时保持高亮(套路同 SshServerView)。
        for editor in me.all_editors() {
            ctx.subscribe_to_view(&editor, |me, source, event, ctx| match event {
                EditorEvent::Edited(_) | EditorEvent::Enter => {
                    if me.status.is_some() {
                        me.status = None;
                        ctx.notify();
                    }
                }
                EditorEvent::Blurred => {
                    source.update(ctx, |e, ctx| e.clear_selections(ctx));
                    if me.status.is_some() {
                        me.status = None;
                        ctx.notify();
                    }
                }
                EditorEvent::Focused | EditorEvent::ClearParentSelections => {
                    me.clear_other_editors_selections(&source, ctx);
                }
                _ => {}
            });
        }

        me
    }

    fn all_editors(&self) -> [ViewHandle<EditorView>; 5] {
        [
            self.name_editor.clone(),
            self.git_url_editor.clone(),
            self.root_path_editor.clone(),
            self.rules_editor.clone(),
            self.notes_editor.clone(),
        ]
    }

    fn clear_other_editors_selections(
        &mut self,
        active: &ViewHandle<EditorView>,
        ctx: &mut ViewContext<Self>,
    ) {
        for editor in self.all_editors() {
            if editor != *active {
                editor.update(ctx, |e, ctx| e.clear_selections(ctx));
            }
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    /// 从 DB 读项目 + 服务器候选 + 已关联集合,把当前值写入各 editor。
    fn reload(&mut self, ctx: &mut ViewContext<Self>) {
        let id = self.project_id.clone();
        let project_result = infinishell_projects::with_conn(|conn| {
            let project = ProjectRepository::get(conn, &id)?;
            let linked = ProjectRepository::servers_for_project(conn, &id)?;
            Ok((project, linked))
        });
        let (project, linked_ids) = match project_result {
            Ok((project, linked)) => (project, linked),
            Err(e) => {
                log::error!("project_view: reload project failed: {e:?}");
                (None, Vec::new())
            }
        };
        self.project = project;

        // SSH 服务器候选列表(树顺序)。读取失败降级为空列表,不影响表单。
        let servers_result = warp_ssh_manager::with_conn(|conn| {
            let nodes = SshRepository::list_nodes(conn)?;
            let mut rows = Vec::new();
            for node in nodes {
                if !matches!(node.kind, NodeKind::Server) {
                    continue;
                }
                let Some(server) = SshRepository::get_server(conn, &node.id)? else {
                    continue;
                };
                rows.push(ProjectServerRow {
                    node_id: node.id,
                    name: node.name,
                    username: server.username,
                    host: server.host,
                    port: server.port,
                });
            }
            Ok(rows)
        });
        self.servers = match servers_result {
            Ok(rows) => rows,
            Err(e) => {
                log::error!("project_view: reload ssh servers failed: {e:?}");
                Vec::new()
            }
        };
        // 悬挂引用(SSH 节点已删)在此静默过滤 — 与左侧面板的行为一致;
        // 下一次保存会把这些残留关联从关联表清掉。
        let known_ids: HashSet<&str> = self.servers.iter().map(|s| s.node_id.as_str()).collect();
        self.linked_node_ids = linked_ids
            .into_iter()
            .filter(|id| known_ids.contains(id.as_str()))
            .collect();
        self.server_row_states
            .resize_with(self.servers.len(), MouseStateHandle::default);

        // 把项目字段写入 editor buffer。
        let name = self
            .project
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let git_url = self
            .project
            .as_ref()
            .and_then(|p| p.git_url.clone())
            .unwrap_or_default();
        let root_path = self
            .project
            .as_ref()
            .and_then(|p| p.root_path.clone())
            .unwrap_or_default();
        let rules = self
            .project
            .as_ref()
            .map(|p| p.rules.clone())
            .unwrap_or_default();
        let notes = self
            .project
            .as_ref()
            .map(|p| p.notes.clone())
            .unwrap_or_default();
        self.name_editor
            .update(ctx, |e, ctx| e.set_buffer_text(&name, ctx));
        self.git_url_editor
            .update(ctx, |e, ctx| e.set_buffer_text(&git_url, ctx));
        self.root_path_editor
            .update(ctx, |e, ctx| e.set_buffer_text(&root_path, ctx));
        self.rules_editor
            .update(ctx, |e, ctx| e.set_buffer_text(&rules, ctx));
        self.notes_editor
            .update(ctx, |e, ctx| e.set_buffer_text(&notes, ctx));

        // `set_buffer_text` 默认全选,首次渲染会看到多个输入框同时高亮,逐个清掉。
        for editor in self.all_editors() {
            editor.update(ctx, |e, ctx| e.clear_selections(ctx));
        }

        // pane 标题(vertical tabs / header)同步为项目名。
        let title = if name.is_empty() {
            "Project".to_string()
        } else {
            name
        };
        self.pane_configuration
            .update(ctx, |pc, ctx| pc.set_title(title, ctx));
        ctx.notify();
    }

    fn current_text(&self, editor: &ViewHandle<EditorView>, app: &AppContext) -> String {
        editor.as_ref(app).buffer_text(app)
    }

    fn on_save(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(loaded) = self.project.clone() else {
            return;
        };

        let name = self.current_text(&self.name_editor.clone(), ctx);
        let git_url = self.current_text(&self.git_url_editor.clone(), ctx);
        let root_path = self.current_text(&self.root_path_editor.clone(), ctx);
        let rules = self.current_text(&self.rules_editor.clone(), ctx);
        let notes = self.current_text(&self.notes_editor.clone(), ctx);

        let name = name.trim().to_string();
        if name.is_empty() {
            self.status = Some(StatusBanner::Error(crate::t!(
                "project-view-error-name-required"
            )));
            ctx.notify();
            return;
        }

        // default_profile_id / sort_order 保持已加载值不动(profile 下拉在 M3)。
        let project = Project {
            id: loaded.id.clone(),
            name,
            git_url: normalize_optional_field(&git_url),
            root_path: normalize_optional_field(&root_path),
            rules: rules.trim().to_string(),
            notes: notes.trim().to_string(),
            default_profile_id: loaded.default_profile_id.clone(),
            sort_order: loaded.sort_order,
            created_at: loaded.created_at,
            updated_at: loaded.updated_at,
        };
        // 关联顺序采用 SSH 树顺序(与勾选列表的展示顺序一致,保存后左侧
        // 面板的主机排序稳定可预期;点击顺序则会随勾选先后漂移)。
        let linked = linked_ids_in_tree_order(&self.servers, &self.linked_node_ids);

        let project_for_db = project.clone();
        let result = infinishell_projects::with_conn(move |conn| {
            ProjectRepository::update(conn, &project_for_db)?;
            ProjectRepository::set_servers(conn, &project_for_db.id, &linked)?;
            Ok(())
        });
        if let Err(e) = result {
            log::error!("project_view: save failed: {e:?}");
            self.status = Some(StatusBanner::Error(format!("{e}")));
            ctx.notify();
            return;
        }

        self.reload(ctx);
        self.status = Some(StatusBanner::Saved);
        ProjectsChangedNotifier::notify_changed(ctx);
        ctx.notify();
    }

    fn on_toggle_server(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let Some(row) = self.servers.get(index) else {
            return;
        };
        if !self.linked_node_ids.remove(&row.node_id) {
            self.linked_node_ids.insert(row.node_id.clone());
        }
        if self.status.is_some() {
            self.status = None;
        }
        ctx.notify();
    }

    /// 确认删除:软删 + 广播 + 关闭本 pane(pane 不持久化,直接关最干净)。
    fn on_confirm_delete(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_delete_confirmation = false;
        let id = self.project_id.clone();
        let result = infinishell_projects::with_conn(move |conn| {
            ProjectRepository::soft_delete(conn, &id)?;
            Ok(())
        });
        if let Err(e) = result {
            log::error!("project_view: delete failed: {e:?}");
            self.status = Some(StatusBanner::Error(format!("{e}")));
            ctx.notify();
            return;
        }
        ProjectsChangedNotifier::notify_changed(ctx);
        ctx.emit(PaneEvent::Close);
    }

    // ---------- 渲染 helpers ---------- //

    fn render_label(&self, text: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Text::new_inline(
                text.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish(),
        )
        .with_margin_top(FIELD_LABEL_MARGIN_TOP)
        .with_margin_bottom(FIELD_LABEL_MARGIN_BOTTOM)
        .finish()
    }

    fn field_input_styles(&self, appearance: &Appearance) -> UiComponentStyles {
        let theme = appearance.theme();
        UiComponentStyles {
            padding: Some(Coords {
                left: 10.,
                right: 10.,
                top: 6.,
                bottom: 6.,
            }),
            background: Some(theme.surface_2().into()),
            border_color: Some(internal_colors::neutral_3(theme).into()),
            border_width: Some(1.0),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.0))),
            ..Default::default()
        }
    }

    fn render_text_field(
        &self,
        label: &str,
        editor: &ViewHandle<EditorView>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let text_input = appearance
            .ui_builder()
            .text_input(editor.clone())
            .with_style(self.field_input_styles(appearance))
            .build()
            .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.render_label(label, appearance))
                .with_child(text_input)
                .finish(),
        )
        .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
        .finish()
    }

    /// 多行字段:同 text_field,但输入区保底高度更高(规则/备注需要空间)。
    fn render_multiline_field(
        &self,
        label: &str,
        editor: &ViewHandle<EditorView>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let text_input = appearance
            .ui_builder()
            .text_input(editor.clone())
            .with_style(self.field_input_styles(appearance))
            .build()
            .finish();
        let sized_input = ConstrainedBox::new(text_input)
            .with_min_height(MULTILINE_FIELD_MIN_HEIGHT)
            .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.render_label(label, appearance))
                .with_child(sized_input)
                .finish(),
        )
        .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
        .finish()
    }

    /// 单台服务器的勾选行:check 图标(未勾选留白占位)+ 节点名 + user@host:port。
    fn render_server_row(
        &self,
        index: usize,
        row: &ProjectServerRow,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let is_linked = self.linked_node_ids.contains(&row.node_id);

        let check_el: Box<dyn Element> = if is_linked {
            ConstrainedBox::new(
                crate::ui_components::icons::Icon::Check
                    .to_warpui_icon(theme.accent())
                    .finish(),
            )
            .with_width(SERVER_ROW_CHECK_SIZE)
            .with_height(SERVER_ROW_CHECK_SIZE)
            .finish()
        } else {
            ConstrainedBox::new(warpui::elements::Empty::new().finish())
                .with_width(SERVER_ROW_CHECK_SIZE)
                .with_height(SERVER_ROW_CHECK_SIZE)
                .finish()
        };

        let title_color = if is_linked {
            theme.active_ui_text_color()
        } else {
            theme.main_text_color(theme.background())
        };
        let name_el = Text::new_inline(
            row.name.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(title_color.into())
        .finish();
        let subtitle_el = Text::new_inline(
            server_row_subtitle(&row.username, &row.host, row.port),
            appearance.ui_font_family(),
            12.0,
        )
        .with_color(theme.sub_text_color(theme.background()).into())
        .finish();

        let content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(check_el)
            .with_child(name_el)
            .with_child(subtitle_el)
            .finish();
        let state = self
            .server_row_states
            .get(index)
            .cloned()
            .unwrap_or_default();
        Hoverable::new(state, move |mouse| {
            let mut container = Container::new(content)
                .with_uniform_padding(6.0)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)));
            if mouse.is_hovered() {
                container = container.with_background(internal_colors::fg_overlay_2(theme));
            }
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ProjectViewAction::ToggleServer(index));
        })
        .finish()
    }

    /// 关联服务器区块:label + 全部候选服务器的勾选行(空态给提示文案)。
    fn render_linked_servers_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut section = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        section.add_child(self.render_label(&crate::t!("project-view-linked-servers"), appearance));
        if self.servers.is_empty() {
            section.add_child(
                Container::new(
                    Text::new_inline(
                        crate::t!("project-view-no-servers"),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(theme.sub_text_color(theme.surface_2()).into())
                    .finish(),
                )
                .with_uniform_padding(8.0)
                .finish(),
            );
        } else {
            for (index, row) in self.servers.iter().enumerate() {
                section.add_child(self.render_server_row(index, row, appearance));
            }
        }

        Container::new(
            Container::new(section.finish())
                .with_uniform_padding(6.0)
                .with_background(theme.surface_2())
                .with_border(Border::all(1.0).with_border_fill(internal_colors::neutral_3(theme)))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                .finish(),
        )
        .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
        .finish()
    }

    fn render_save_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.save_btn_state.clone())
            .with_style(UiComponentStyles {
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().accent())
                        .into_solid(),
                ),
                font_weight: Some(Weight::Bold),
                width: Some(SAVE_BUTTON_WIDTH),
                height: Some(SAVE_BUTTON_HEIGHT),
                font_size: Some(13.0),
                ..Default::default()
            })
            .with_centered_text_label(crate::t!("project-view-save"))
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(ProjectViewAction::Save))
            .finish()
    }

    fn render_delete_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(ButtonVariant::Warn, self.delete_btn_state.clone())
            .with_style(UiComponentStyles {
                font_weight: Some(Weight::Bold),
                height: Some(SAVE_BUTTON_HEIGHT),
                font_size: Some(13.0),
                ..Default::default()
            })
            .with_centered_text_label(crate::t!("project-view-delete"))
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ProjectViewAction::OpenDeleteConfirmation)
            })
            .finish()
    }

    fn render_status_banner(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let (text, color) = match self.status.as_ref()? {
            StatusBanner::Saved => (
                crate::t!("project-view-status-saved"),
                theme.ui_green_color(),
            ),
            StatusBanner::Error(msg) => (msg.clone(), theme.ui_error_color()),
        };
        Some(
            Container::new(
                Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
                    .with_color(color)
                    .finish(),
            )
            .with_margin_top(8.0)
            .with_margin_bottom(8.0)
            .finish(),
        )
    }

    /// 删除 confirmation 弹层(套路同 SshServerView 的 ClearMachineMemory)。
    fn render_delete_confirmation(&self, appearance: &Appearance) -> Box<dyn Element> {
        use crate::ui_components::dialog::{Dialog, dialog_styles};

        let cancel_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.delete_cancel_btn_state.clone(),
            )
            .with_text_label(crate::t!("common-cancel"))
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ProjectViewAction::CancelDeleteConfirmation)
            })
            .finish();
        let delete_button = Container::new(
            appearance
                .ui_builder()
                .button(ButtonVariant::Warn, self.delete_confirm_btn_state.clone())
                .with_text_label(crate::t!("project-view-delete-confirm-button"))
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(ProjectViewAction::ConfirmDelete))
                .finish(),
        )
        .with_margin_left(12.0)
        .finish();
        let dialog = Dialog::new(
            crate::t!("project-view-delete-confirm-title"),
            Some(crate::t!("project-view-delete-confirm-description")),
            dialog_styles(appearance),
        )
        .with_bottom_row_child(cancel_button)
        .with_bottom_row_child(delete_button)
        .with_width(DELETE_DIALOG_WIDTH)
        .build()
        .finish();

        Dismiss::new(dialog)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(ProjectViewAction::CancelDeleteConfirmation);
            })
            .finish()
    }
}

fn make_single_line_editor(
    placeholder: &str,
    ctx: &mut ViewContext<ProjectView>,
) -> ViewHandle<EditorView> {
    let placeholder = placeholder.to_string();
    ctx.add_typed_action_view(move |ctx| {
        let options = {
            let appearance = Appearance::as_ref(ctx);
            let theme = appearance.theme();
            SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    font_family_override: Some(appearance.monospace_font_family()),
                    text_colors_override: Some(TextColors {
                        default_color: theme.active_ui_text_color(),
                        disabled_color: theme.disabled_ui_text_color(),
                        hint_color: theme.disabled_ui_text_color(),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }
        };
        let mut editor = EditorView::single_line(options, ctx);
        editor.set_placeholder_text(&placeholder, ctx);
        editor
    })
}

/// 多行编辑器(规则/备注):Enter 换行、软换行、随内容自增高。
fn make_multiline_editor(
    placeholder: &str,
    ctx: &mut ViewContext<ProjectView>,
) -> ViewHandle<EditorView> {
    let placeholder = placeholder.to_string();
    ctx.add_typed_action_view(move |ctx| {
        let options = {
            let appearance = Appearance::as_ref(ctx);
            let theme = appearance.theme();
            EditorOptions {
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    font_family_override: Some(appearance.monospace_font_family()),
                    text_colors_override: Some(TextColors {
                        default_color: theme.active_ui_text_color(),
                        disabled_color: theme.disabled_ui_text_color(),
                        hint_color: theme.disabled_ui_text_color(),
                    }),
                    ..Default::default()
                },
                enter_settings: EnterSettings {
                    enter: EnterAction::InsertNewLineIfMultiLine,
                    ..Default::default()
                },
                autogrow: true,
                soft_wrap: true,
                ..Default::default()
            }
        };
        let mut editor = EditorView::new(options, ctx);
        editor.set_placeholder_text(&placeholder, ctx);
        editor
    })
}

/// 服务器行的副标题:`user@host:port`;user 为空时省略 `user@`。
fn server_row_subtitle(username: &str, host: &str, port: u16) -> String {
    if username.is_empty() {
        format!("{host}:{port}")
    } else {
        format!("{username}@{host}:{port}")
    }
}

/// trim 后为空则视作"未填",写库为 NULL。
fn normalize_optional_field(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 把勾选集合按候选列表(SSH 树)的顺序展开成有序 node_id 列表。
fn linked_ids_in_tree_order(servers: &[ProjectServerRow], linked: &HashSet<String>) -> Vec<String> {
    servers
        .iter()
        .filter(|row| linked.contains(&row.node_id))
        .map(|row| row.node_id.clone())
        .collect()
}

impl Entity for ProjectView {
    type Event = PaneEvent;
}

impl TypedActionView for ProjectView {
    type Action = ProjectViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ProjectViewAction::Save => self.on_save(ctx),
            ProjectViewAction::ToggleServer(index) => self.on_toggle_server(*index, ctx),
            ProjectViewAction::OpenDeleteConfirmation => {
                self.show_delete_confirmation = true;
                ctx.notify();
            }
            ProjectViewAction::CancelDeleteConfirmation => {
                self.show_delete_confirmation = false;
                ctx.notify();
            }
            ProjectViewAction::ConfirmDelete => self.on_confirm_delete(ctx),
        }
    }
}

impl View for ProjectView {
    fn ui_name() -> &'static str {
        "ProjectView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // 项目不存在(已被删除/加载失败)→ 简单提示 + 隐藏表单。
        let Some(project) = self.project.as_ref() else {
            let theme = appearance.theme();
            let body = Text::new_inline(
                crate::t!("project-view-missing"),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish();
            return Align::new(
                ConstrainedBox::new(Container::new(body).with_uniform_padding(24.0).finish())
                    .with_max_width(560.0)
                    .finish(),
            )
            .top_center()
            .finish();
        };

        // ---- header row: 项目名 + 右侧 [删除] [保存] 按钮 ----
        let title = Text::new_inline(
            project.name.clone(),
            appearance.ui_font_family(),
            appearance.ui_font_heading_2(),
        )
        .with_color(
            appearance
                .theme()
                .main_text_color(appearance.theme().background())
                .into(),
        )
        .finish();
        let buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(self.render_delete_button(appearance))
            .with_child(self.render_save_button(appearance))
            .with_main_axis_size(MainAxisSize::Min)
            .finish();
        let header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(title)
            .with_child(buttons)
            .finish();

        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        col.add_child(Container::new(header).with_margin_bottom(8.0).finish());

        if let Some(banner) = self.render_status_banner(appearance) {
            col.add_child(banner);
        }

        col.add_child(self.render_text_field(
            &crate::t!("project-view-name-label"),
            &self.name_editor,
            appearance,
        ));
        col.add_child(self.render_text_field(
            &crate::t!("project-view-git-url-label"),
            &self.git_url_editor,
            appearance,
        ));
        col.add_child(self.render_text_field(
            &crate::t!("project-view-root-path-label"),
            &self.root_path_editor,
            appearance,
        ));
        col.add_child(self.render_multiline_field(
            &crate::t!("project-view-rules-label"),
            &self.rules_editor,
            appearance,
        ));
        col.add_child(self.render_multiline_field(
            &crate::t!("project-view-notes-label"),
            &self.notes_editor,
            appearance,
        ));
        col.add_child(self.render_linked_servers_section(appearance));

        let theme = appearance.theme();
        let inner = ConstrainedBox::new(
            Container::new(col.finish())
                .with_uniform_padding(24.0)
                .finish(),
        )
        .with_max_width(640.0)
        .finish();

        // 内容溢出时垂直滚动(套路同 SshServerView)。
        let scrollbar_color = theme.disabled_text_color(theme.background()).into();
        let scrollbar_thumb_hover = theme.main_text_color(theme.background()).into();
        let scrollable = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            inner,
            ScrollbarWidth::Auto,
            scrollbar_color,
            scrollbar_thumb_hover,
            Fill::None,
        )
        .finish();

        let content = Align::new(scrollable).top_center().finish();
        if self.show_delete_confirmation {
            let mut stack = Stack::new().with_child(content);
            stack.add_positioned_overlay_child(
                self.render_delete_confirmation(appearance),
                OffsetPositioning::offset_from_parent(
                    vec2f(0.0, 0.0),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
            stack.finish()
        } else {
            content
        }
    }
}

impl BackingView for ProjectView {
    type PaneHeaderOverflowMenuAction = ProjectViewAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(PaneEvent::Close);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.name_editor);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        let title = self
            .project
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Project".to_string());
        view::HeaderContent::simple(title)
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

#[cfg(test)]
#[path = "project_view_tests.rs"]
mod tests;
