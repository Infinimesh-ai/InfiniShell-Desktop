//! SSH 服务器编辑器中央 pane 的 BackingView 实现。
//!
//! Phase 2:可编辑表单(name / host / port / user / auth / password / key_path)+
//! 顶部右上角 "Save" 按钮 + Auth 类型切换(密码 / 私钥)。
//!
//! Phase 3 起加 "连接" 按钮 → emit OpenSshTerminal → SecretInjector。

use markdown_parser::parse_markdown;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_ssh_manager::{
    AuthType, ConnectionStatus, KeychainSecretStore, MachineMemory, MachineMemoryRepository,
    NodeKind, OneKeyCredentialKind, SecretKind, SshNode, SshOneKeyCredential, SshRepository,
    SshSecretStore, SshSecretStoreError, SshServerInfo, resolve_machine_key,
};
use warpui::elements::{
    Align, Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable,
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Element, Fill, Flex,
    FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, ScrollbarWidth,
    Shrinkable, Stack, Text, Wrap,
};
use warpui::fonts::Weight;
use warpui::platform::{Cursor, FilePickerConfiguration};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};
use zeroize::Zeroizing;

use crate::editor::{
    EditorView, Event as EditorEvent, SingleLineEditorOptions, TextColors, TextOptions,
};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::ssh_manager::{SshConnectionTarget, SshTreeChangedEvent, SshTreeChangedNotifier};
use crate::view_components::dropdown::{Dropdown, DropdownItem};

const FIELD_LABEL_MARGIN_TOP: f32 = 6.0;
const FIELD_LABEL_MARGIN_BOTTOM: f32 = 4.0;
const FIELD_BLOCK_MARGIN_BOTTOM: f32 = 12.0;
const SAVE_BUTTON_WIDTH: f32 = 96.0;
const SAVE_BUTTON_HEIGHT: f32 = 28.0;
const AUTH_TOGGLE_PADDING_H: f32 = 14.0;
const AUTH_TOGGLE_PADDING_V: f32 = 6.0;
const ONEKEY_MANAGER_WIDTH: f32 = 680.0;
const ONEKEY_MANAGER_HEIGHT: f32 = 500.0;
const ONEKEY_MANAGER_LIST_WIDTH: f32 = 220.0;
const MACHINE_MEMORY_CONTENT_HEIGHT: f32 = 220.0;
const MACHINE_MEMORY_DIALOG_WIDTH: f32 = 450.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshServerAction {
    Save,
    Connect,
    TestConnection,
    SetAuthPassword,
    SetAuthKey,
    SetAuthOneKey,
    /// 打开系统文件选择器选私钥文件,把路径写入 key_path editor。
    PickKeyFile,
    /// 选择分组(None 表示根级,Some(index) 表示 self.folders[index])。
    SelectGroup(Option<usize>),
    SelectOneKeyCredential(Option<usize>),
    PickOneKeyKeyFile,
    OpenOneKeyManager,
    CloseOneKeyManager,
    NewOneKeyCredential,
    SelectManagedOneKeyCredential(Option<usize>),
    SetManagedOneKeyPassword,
    SetManagedOneKeyKey,
    SaveManagedOneKeyCredential,
    DeleteManagedOneKeyCredential,
    OpenClearMachineMemoryConfirmation,
    CancelClearMachineMemoryConfirmation,
    ConfirmClearMachineMemory,
}

/// 一次性显示在 Save 按钮上方/下方的状态标签。
#[derive(Debug, Clone)]
enum StatusBanner {
    Saved,
    Success(String),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthSpecificField {
    Password,
    KeyPath,
    Passphrase,
    OneKeyCredential,
}

pub struct SshServerView {
    node_id: String,
    /// 节点元信息(主要用 name 当 header title)。
    node: Option<SshNode>,
    /// 缓存上次从 DB 读到的 server,用于占位文本和初值。folder 节点会是 None。
    server: Option<SshServerInfo>,
    /// 当前服务器对应的 AI 记忆；读取失败按无记忆降级，不影响服务器表单。
    machine_memory: Option<MachineMemory>,
    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,

    name_editor: ViewHandle<EditorView>,
    host_editor: ViewHandle<EditorView>,
    port_editor: ViewHandle<EditorView>,
    user_editor: ViewHandle<EditorView>,
    password_editor: ViewHandle<EditorView>,
    key_path_editor: ViewHandle<EditorView>,
    onekey_label_editor: ViewHandle<EditorView>,
    onekey_user_editor: ViewHandle<EditorView>,
    onekey_key_path_editor: ViewHandle<EditorView>,
    root_password_editor: ViewHandle<EditorView>,
    startup_command_editor: ViewHandle<EditorView>,
    notes_editor: ViewHandle<EditorView>,

    /// 当前选中的认证方式。Save 按钮提交此值到 DB。
    auth_type: AuthType,

    save_btn_state: MouseStateHandle,
    connect_btn_state: MouseStateHandle,
    test_btn_state: MouseStateHandle,
    auth_password_btn_state: MouseStateHandle,
    auth_key_btn_state: MouseStateHandle,
    auth_onekey_btn_state: MouseStateHandle,
    key_path_picker_btn_state: MouseStateHandle,
    onekey_manager_btn_state: MouseStateHandle,
    onekey_manager_close_btn_state: MouseStateHandle,
    onekey_manager_new_btn_state: MouseStateHandle,
    onekey_manager_save_btn_state: MouseStateHandle,
    onekey_manager_delete_btn_state: MouseStateHandle,
    onekey_manager_password_btn_state: MouseStateHandle,
    onekey_manager_key_btn_state: MouseStateHandle,
    onekey_key_path_picker_btn_state: MouseStateHandle,
    onekey_manager_row_states: Vec<MouseStateHandle>,
    clear_machine_memory_btn_state: MouseStateHandle,
    clear_machine_memory_confirm_btn_state: MouseStateHandle,
    clear_machine_memory_cancel_btn_state: MouseStateHandle,

    /// 分组下拉选择器。
    group_dropdown: ViewHandle<Dropdown<SshServerAction>>,
    onekey_credential_dropdown: ViewHandle<Dropdown<SshServerAction>>,
    onekey_credentials: Vec<SshOneKeyCredential>,
    selected_onekey_credential_id: Option<String>,
    show_onekey_manager: bool,
    show_clear_machine_memory_confirmation: bool,
    managed_onekey_credential_id: Option<String>,
    managed_onekey_kind: OneKeyCredentialKind,
    /// 缓存所有文件夹节点 (id, name),用于重建下拉列表。
    folders: Vec<(String, String)>,
    /// 当前选中的分组 ID(None 表示根级)。
    current_group_id: Option<String>,
    /// 初始从 DB 读到的 parent_id,用于判断 save 时是否需要 move_node。
    original_parent_id: Option<String>,

    status: Option<StatusBanner>,
    connection_status: ConnectionStatus,
    latency_ms: Option<u64>,
    is_testing: bool,
    scroll_state: ClippedScrollStateHandle,
    machine_memory_scroll_state: ClippedScrollStateHandle,
}

impl SshServerView {
    pub fn new(node_id: String, ctx: &mut ViewContext<Self>) -> Self {
        let name_editor = make_editor(false, &crate::t!("common-name"), ctx);
        let host_editor = make_editor(false, "example.com", ctx);
        let port_editor = make_editor(false, "22", ctx);
        let user_editor = make_editor(false, "root", ctx);
        let password_editor = make_editor(true, "•••••••", ctx);
        let key_path_editor = make_editor(false, "/home/user/.ssh/id_ed25519", ctx);
        let onekey_label_editor = make_editor(
            false,
            &crate::t!("workspace-left-panel-ssh-manager-onekey-new"),
            ctx,
        );
        let onekey_user_editor = make_editor(false, "root", ctx);
        let onekey_key_path_editor = make_editor(false, "/home/user/.ssh/id_ed25519", ctx);
        let root_password_editor = make_editor(
            true,
            &crate::t!("workspace-left-panel-ssh-manager-root-password-placeholder"),
            ctx,
        );
        let startup_command_editor = make_editor(
            false,
            &crate::t!("workspace-left-panel-ssh-manager-startup-command-placeholder"),
            ctx,
        );
        let notes_editor = make_editor(
            false,
            &crate::t!("workspace-left-panel-ssh-manager-notes-placeholder"),
            ctx,
        );

        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("SSH server"));

        let group_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dd = Dropdown::new(ctx);
            dd.set_main_axis_size(MainAxisSize::Max, ctx);
            dd
        });
        let onekey_credential_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dd = Dropdown::new(ctx);
            dd.set_main_axis_size(MainAxisSize::Max, ctx);
            dd
        });

        let mut me = Self {
            node_id,
            node: None,
            server: None,
            machine_memory: None,
            pane_configuration,
            focus_handle: None,
            name_editor,
            host_editor,
            port_editor,
            user_editor,
            password_editor,
            key_path_editor,
            onekey_label_editor,
            onekey_user_editor,
            onekey_key_path_editor,
            root_password_editor,
            startup_command_editor,
            notes_editor,
            auth_type: AuthType::Password,
            save_btn_state: MouseStateHandle::default(),
            connect_btn_state: MouseStateHandle::default(),
            test_btn_state: MouseStateHandle::default(),
            auth_password_btn_state: MouseStateHandle::default(),
            auth_key_btn_state: MouseStateHandle::default(),
            auth_onekey_btn_state: MouseStateHandle::default(),
            key_path_picker_btn_state: MouseStateHandle::default(),
            onekey_manager_btn_state: MouseStateHandle::default(),
            onekey_manager_close_btn_state: MouseStateHandle::default(),
            onekey_manager_new_btn_state: MouseStateHandle::default(),
            onekey_manager_save_btn_state: MouseStateHandle::default(),
            onekey_manager_delete_btn_state: MouseStateHandle::default(),
            onekey_manager_password_btn_state: MouseStateHandle::default(),
            onekey_manager_key_btn_state: MouseStateHandle::default(),
            onekey_key_path_picker_btn_state: MouseStateHandle::default(),
            onekey_manager_row_states: Vec::new(),
            clear_machine_memory_btn_state: MouseStateHandle::default(),
            clear_machine_memory_confirm_btn_state: MouseStateHandle::default(),
            clear_machine_memory_cancel_btn_state: MouseStateHandle::default(),
            group_dropdown,
            onekey_credential_dropdown,
            onekey_credentials: Vec::new(),
            selected_onekey_credential_id: None,
            show_onekey_manager: false,
            show_clear_machine_memory_confirmation: false,
            managed_onekey_credential_id: None,
            managed_onekey_kind: OneKeyCredentialKind::Password,
            folders: Vec::new(),
            current_group_id: None,
            original_parent_id: None,
            status: None,
            connection_status: ConnectionStatus::Unknown,
            latency_ms: None,
            is_testing: false,
            scroll_state: ClippedScrollStateHandle::default(),
            machine_memory_scroll_state: ClippedScrollStateHandle::default(),
        };
        me.reload(ctx);

        // 监听每个 editor:编辑 → 清掉 status banner;ClearParentSelections →
        // 清空所有其他 editor 的 selection(否则切换字段时多个输入框会同时高亮)。
        let editors = [
            me.name_editor.clone(),
            me.host_editor.clone(),
            me.port_editor.clone(),
            me.user_editor.clone(),
            me.password_editor.clone(),
            me.key_path_editor.clone(),
            me.onekey_label_editor.clone(),
            me.onekey_user_editor.clone(),
            me.onekey_key_path_editor.clone(),
            me.root_password_editor.clone(),
            me.startup_command_editor.clone(),
            me.notes_editor.clone(),
        ];
        for editor in editors {
            ctx.subscribe_to_view(&editor, |me, source, event, ctx| match event {
                EditorEvent::Edited(_) | EditorEvent::Enter => {
                    if me.status.is_some() {
                        me.status = None;
                        ctx.notify();
                    }
                }
                EditorEvent::Blurred => {
                    // 失焦时把自身的 selection 也清掉,防止"点别的 editor 后,
                    // 旧 editor 仍是高亮全选"。
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

    fn clear_other_editors_selections(
        &mut self,
        active: &ViewHandle<EditorView>,
        ctx: &mut ViewContext<Self>,
    ) {
        let all = [
            self.name_editor.clone(),
            self.host_editor.clone(),
            self.port_editor.clone(),
            self.user_editor.clone(),
            self.password_editor.clone(),
            self.key_path_editor.clone(),
            self.onekey_label_editor.clone(),
            self.onekey_user_editor.clone(),
            self.onekey_key_path_editor.clone(),
            self.root_password_editor.clone(),
            self.startup_command_editor.clone(),
            self.notes_editor.clone(),
        ];
        for editor in all {
            if editor != *active {
                editor.update(ctx, |e, ctx| e.clear_selections(ctx));
            }
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    /// 从 DB 读节点 + server,把当前 buffer 写入各 editor。
    fn reload(&mut self, ctx: &mut ViewContext<Self>) {
        let id = self.node_id.clone();
        let result = warp_ssh_manager::with_conn(|c| {
            let nodes = SshRepository::list_nodes(c)?;
            let node = nodes.iter().find(|n| n.id == id).cloned();
            let server = match node.as_ref().map(|n| n.kind) {
                Some(NodeKind::Server) => SshRepository::get_server(c, &id)?,
                _ => None,
            };
            // 收集所有 folder 节点(id, name)
            let folders: Vec<(String, String)> = nodes
                .iter()
                .filter(|n| matches!(n.kind, NodeKind::Folder))
                .map(|n| (n.id.clone(), n.name.clone()))
                .collect();
            let onekey_credentials = SshRepository::list_onekey_credentials(c)?;
            Ok((node, server, folders, onekey_credentials))
        });
        match result {
            Ok((node, server, folders, onekey_credentials)) => {
                self.original_parent_id = node.as_ref().and_then(|n| n.parent_id.clone());
                self.current_group_id = self.original_parent_id.clone();
                self.node = node;
                self.server = server;
                self.folders = folders;
                self.onekey_credentials = onekey_credentials;
            }
            Err(e) => {
                log::error!("ssh_server_view: reload failed: {e:?}");
                self.node = None;
                self.server = None;
                self.folders = Vec::new();
                self.onekey_credentials = Vec::new();
                self.original_parent_id = None;
                self.current_group_id = None;
            }
        }
        self.reload_machine_memory();

        // 把节点名 / server 字段写入 editor buffer
        let name = self
            .node
            .as_ref()
            .map(|n| n.name.clone())
            .unwrap_or_default();
        self.name_editor
            .update(ctx, |e, ctx| e.set_buffer_text(&name, ctx));

        if let Some(srv) = self.server.clone() {
            self.auth_type = srv.auth_type;
            self.selected_onekey_credential_id = srv.credential_id.clone();
            let host = srv.host.clone();
            let port_str = srv.port.to_string();
            let user = srv.username.clone();
            let key_path = srv.key_path.clone().unwrap_or_default();
            self.host_editor
                .update(ctx, |e, ctx| e.set_buffer_text(&host, ctx));
            self.port_editor
                .update(ctx, |e, ctx| e.set_buffer_text(&port_str, ctx));
            self.user_editor
                .update(ctx, |e, ctx| e.set_buffer_text(&user, ctx));
            self.key_path_editor
                .update(ctx, |e, ctx| e.set_buffer_text(&key_path, ctx));
            self.sync_managed_onekey_selection(ctx);

            // 密码:仅显示 keychain 里有内容时填一次,否则保持空(用户输入新值才覆盖)。
            // 注意:不展示明文密码,只在 keychain 里"存在"时给一个全是 • 的占位 — 不
            // 影响保存语义(空字符串保持密码不变;非空字符串覆盖)。
            // 这里直接清空 buffer,密码保留在 keychain 里;Save 时只在 buffer 非空才写。
            // 占位模式镜像 root_password_editor(keychain 已存 → "●●●●●●●";
            // 未存 → 回到 new() 时设的 "•••••••"),给用户一个"留空也能 Test"
            // 的视觉提示。
            let (password_lookup_id, password_kind) = password_lookup_for_server_form(&srv);
            let pw_saved = match password_lookup_id.as_deref() {
                Some(id) => KeychainSecretStore
                    .get(id, password_kind)
                    .unwrap_or(None)
                    .is_some(),
                None => false,
            };
            self.password_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("", ctx);
                if pw_saved {
                    e.set_placeholder_text("●●●●●●●", ctx);
                } else {
                    e.set_placeholder_text("•••••••", ctx);
                }
            });
            let startup_command = srv.startup_command.clone().unwrap_or_default();
            self.startup_command_editor
                .update(ctx, |e, ctx| e.set_buffer_text(&startup_command, ctx));
            let notes = srv.notes.clone().unwrap_or_default();
            self.notes_editor
                .update(ctx, |e, ctx| e.set_buffer_text(&notes, ctx));
            // Root 密码:检测 keychain 是否已保存,已保存时显示占位提示。
            let root_pw_saved = KeychainSecretStore
                .get(&srv.node_id, SecretKind::RootPassword)
                .unwrap_or(None)
                .is_some();
            self.root_password_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("", ctx);
                if root_pw_saved {
                    e.set_placeholder_text("●●●●●●●", ctx);
                } else {
                    e.set_placeholder_text(
                        &crate::t!("workspace-left-panel-ssh-manager-root-password-placeholder"),
                        ctx,
                    );
                }
            });
        }

        // `set_buffer_text` 默认让所有 editor 处于"全选"状态(buffer 替换 +
        // 默认 selection),首次渲染会看到多个输入框同时被高亮。逐个 clear。
        let editors = [
            self.name_editor.clone(),
            self.host_editor.clone(),
            self.port_editor.clone(),
            self.user_editor.clone(),
            self.password_editor.clone(),
            self.key_path_editor.clone(),
            self.onekey_label_editor.clone(),
            self.onekey_user_editor.clone(),
            self.onekey_key_path_editor.clone(),
            self.root_password_editor.clone(),
            self.startup_command_editor.clone(),
            self.notes_editor.clone(),
        ];
        for editor in editors {
            editor.update(ctx, |e, ctx| e.clear_selections(ctx));
        }

        self.rebuild_group_dropdown(ctx);
        self.rebuild_onekey_credential_dropdown(ctx);
        self.sync_onekey_manager_row_states();
        ctx.notify();
    }

    /// 记忆读取独立于主 reload；数据库异常只隐藏记忆区内容，不影响服务器详情。
    fn reload_machine_memory(&mut self) {
        let Some(machine_key) = self.server.as_ref().and_then(machine_key_for_server) else {
            self.machine_memory = None;
            return;
        };
        match warp_ssh_manager::with_conn(|conn| {
            Ok(MachineMemoryRepository::get(conn, &machine_key)?)
        }) {
            Ok(memory) => self.machine_memory = memory,
            Err(error) => {
                log::error!("ssh_server_view: reload machine memory failed: {error:?}");
                self.machine_memory = None;
            }
        }
    }

    /// 根据 self.folders 重建下拉列表并设置当前选中项。
    fn rebuild_group_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let root_label = crate::t!("workspace-left-panel-ssh-manager-group-root");
        let mut items: Vec<DropdownItem<SshServerAction>> = vec![DropdownItem::new(
            root_label,
            SshServerAction::SelectGroup(None),
        )];
        for (i, (_, name)) in self.folders.iter().enumerate() {
            items.push(DropdownItem::new(
                name.clone(),
                SshServerAction::SelectGroup(Some(i)),
            ));
        }

        // 查找当前分组对应的 index
        let selected_index = if let Some(ref gid) = self.current_group_id {
            self.folders
                .iter()
                .position(|(id, _)| id == gid)
                .map(|pos| pos + 1) // +1 因为 index 0 是 "Root"
                .unwrap_or_else(|| {
                    // 文件夹已被外部删除,回退到根级
                    self.current_group_id = None;
                    0
                })
        } else {
            0 // Root
        };

        self.group_dropdown.update(ctx, |dd, ctx| {
            dd.set_items(items, ctx);
            dd.set_selected_by_index(selected_index, ctx);
        });
    }

    fn rebuild_onekey_credential_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let mut items: Vec<DropdownItem<SshServerAction>> = vec![DropdownItem::new(
            crate::t!("workspace-left-panel-ssh-manager-onekey-select"),
            SshServerAction::SelectOneKeyCredential(None),
        )];
        for (index, credential) in self.onekey_credentials.iter().enumerate() {
            items.push(DropdownItem::new(
                credential.display_label(),
                SshServerAction::SelectOneKeyCredential(Some(index)),
            ));
        }

        let selected_index = self
            .selected_onekey_credential_id
            .as_ref()
            .and_then(|id| {
                self.onekey_credentials
                    .iter()
                    .position(|credential| credential.id == *id)
                    .map(|index| index + 1)
            })
            .unwrap_or(0);

        self.onekey_credential_dropdown.update(ctx, |dd, ctx| {
            dd.set_items(items, ctx);
            dd.set_selected_by_index(selected_index, ctx);
        });
    }

    fn reload_onekey_credentials(&mut self, ctx: &mut ViewContext<Self>) {
        match warp_ssh_manager::with_conn(|c| Ok(SshRepository::list_onekey_credentials(c)?)) {
            Ok(credentials) => {
                self.onekey_credentials = credentials;
            }
            Err(e) => {
                log::error!("ssh_server_view: reload onekey credentials failed: {e:?}");
                self.onekey_credentials = Vec::new();
            }
        }
        if let Some(selected_id) = self.selected_onekey_credential_id.as_ref() {
            if !self
                .onekey_credentials
                .iter()
                .any(|credential| credential.id == *selected_id)
            {
                self.selected_onekey_credential_id = None;
            }
        }
        if let Some(managed_id) = self.managed_onekey_credential_id.as_ref() {
            if !self
                .onekey_credentials
                .iter()
                .any(|credential| credential.id == *managed_id)
            {
                self.managed_onekey_credential_id = None;
            }
        }
        self.rebuild_onekey_credential_dropdown(ctx);
        self.sync_onekey_manager_row_states();
    }

    fn sync_managed_onekey_selection(&mut self, ctx: &mut ViewContext<Self>) {
        let selected = self.selected_onekey_credential_id.as_ref().and_then(|id| {
            self.onekey_credentials
                .iter()
                .find(|credential| credential.id == *id)
                .cloned()
        });
        if let Some(credential) = selected.as_ref() {
            self.set_managed_onekey_form_from_credential(credential, ctx);
        } else {
            self.clear_managed_onekey_form(ctx);
        }
        self.managed_onekey_credential_id = selected.map(|credential| credential.id);
    }

    fn sync_onekey_manager_row_states(&mut self) {
        self.onekey_manager_row_states
            .resize_with(self.onekey_credentials.len(), MouseStateHandle::default);
    }

    fn set_managed_onekey_form_from_credential(
        &mut self,
        credential: &SshOneKeyCredential,
        ctx: &mut ViewContext<Self>,
    ) {
        self.managed_onekey_credential_id = Some(credential.id.clone());
        self.managed_onekey_kind = credential.kind;
        self.onekey_label_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&credential.label, ctx)
        });
        self.onekey_user_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&credential.username, ctx)
        });
        self.onekey_key_path_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(credential.key_path.as_deref().unwrap_or_default(), ctx)
        });
        self.password_editor
            .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
    }

    fn clear_managed_onekey_form(&mut self, ctx: &mut ViewContext<Self>) {
        self.managed_onekey_credential_id = None;
        self.managed_onekey_kind = OneKeyCredentialKind::Password;
        self.onekey_label_editor
            .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
        self.onekey_user_editor
            .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
        self.onekey_key_path_editor
            .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
        self.password_editor
            .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
    }

    fn current_text(&self, editor: &ViewHandle<EditorView>, app: &AppContext) -> String {
        editor.as_ref(app).buffer_text(app)
    }

    /// 获取当前选中的分组 ID。
    pub fn current_group_id(&self) -> &Option<String> {
        &self.current_group_id
    }

    /// 获取所有文件夹 (id, name) 的引用（用于测试）。
    pub fn folders(&self) -> &[(String, String)] {
        &self.folders
    }

    fn on_save(&mut self, ctx: &mut ViewContext<Self>) {
        // 1. 收集字段
        let name = self.current_text(&self.name_editor.clone(), ctx);
        let host = self.current_text(&self.host_editor.clone(), ctx);
        let port_str = self.current_text(&self.port_editor.clone(), ctx);
        let user = self.current_text(&self.user_editor.clone(), ctx);
        let key_path_text = self.current_text(&self.key_path_editor.clone(), ctx);
        let root_password = self.current_text(&self.root_password_editor.clone(), ctx);
        let startup_command_text = self.current_text(&self.startup_command_editor.clone(), ctx);
        let notes_text = self.current_text(&self.notes_editor.clone(), ctx);

        let name = name.trim().to_string();
        if name.is_empty() {
            self.status = Some(StatusBanner::Error(crate::t!(
                "workspace-left-panel-ssh-manager-error-name-required"
            )));
            ctx.notify();
            return;
        }

        let port: u16 = match port_str.trim().parse() {
            Ok(p) => p,
            Err(_) => {
                self.status = Some(StatusBanner::Error(crate::t!(
                    "workspace-left-panel-ssh-manager-error-port-invalid"
                )));
                ctx.notify();
                return;
            }
        };

        let credential_id = if self.auth_type == AuthType::OneKey {
            if self.selected_onekey_credential_id.is_none() {
                self.status = Some(StatusBanner::Error(crate::t!(
                    "workspace-left-panel-ssh-manager-onekey-select-required"
                )));
                ctx.notify();
                return;
            }
            self.selected_onekey_credential_id.clone()
        } else {
            None
        };

        let key_path = key_path_text.trim().to_string();
        let info = SshServerInfo {
            node_id: self.node_id.clone(),
            host: host.trim().to_string(),
            port,
            username: user.trim().to_string(),
            auth_type: self.auth_type,
            key_path: if key_path.is_empty() {
                None
            } else {
                Some(key_path)
            },
            credential_id,
            startup_command: if startup_command_text.trim().is_empty() {
                None
            } else {
                Some(startup_command_text.trim().to_string())
            },
            notes: if notes_text.trim().is_empty() {
                None
            } else {
                Some(notes_text.trim().to_string())
            },
            last_connected_at: self.server.as_ref().and_then(|s| s.last_connected_at),
        };

        // 2. 写 DB(rename + update_server + 可能的 move_node)
        let id = self.node_id.clone();
        let info_for_db = info.clone();
        let name_for_db = name.clone();
        let group_changed = self.current_group_id != self.original_parent_id;
        let new_parent_id = self.current_group_id.clone();
        let result = warp_ssh_manager::with_conn(move |c| {
            SshRepository::rename_node(c, &id, &name_for_db)?;
            SshRepository::update_server(c, &info_for_db)?;
            if group_changed {
                let new_parent = new_parent_id.as_deref();
                SshRepository::move_node_to_end(c, &id, new_parent)?;
            }
            Ok(())
        });
        if let Err(e) = result {
            log::error!("ssh_server_view: save failed: {e:?}");
            self.status = Some(StatusBanner::Error(format!("{e}")));
            ctx.notify();
            return;
        }

        // 3. 写 keychain(buffer 非空才覆盖)。auth_type 切到密码时如果用户没填,
        //    保留原有 keychain 条目;切到私钥时不动密码 entry(用户可单独删)。
        let store = KeychainSecretStore;
        let password = self.current_text(&self.password_editor.clone(), ctx);
        if self.auth_type != AuthType::OneKey && !password.is_empty() {
            let (secret_lookup_id, kind) = password_lookup_for_server_form(&info);
            let Some(secret_lookup_id) = secret_lookup_id else {
                self.status = Some(StatusBanner::Error(
                    "OneKey credential is missing".to_string(),
                ));
                ctx.notify();
                return;
            };
            if let Err(e) = store.set(&secret_lookup_id, kind, &password) {
                log::error!("ssh_server_view: keychain write failed: {e:?}");
                self.status = Some(StatusBanner::Error(format!("keychain: {e}")));
                ctx.notify();
                return;
            }
            // 密码字段写入后清空 buffer,避免明文长时间停留在内存。
            self.password_editor
                .update(ctx, |e, ctx| e.set_buffer_text("", ctx));
        }

        // Root password
        if !root_password.is_empty() {
            if let Err(e) = store.set(&self.node_id, SecretKind::RootPassword, &root_password) {
                log::error!("ssh_server_view: root password keychain write failed: {e:?}");
                self.status = Some(StatusBanner::Error(format!("keychain: {e}")));
                ctx.notify();
                return;
            }
            self.root_password_editor
                .update(ctx, |e, ctx| e.set_buffer_text("", ctx));
        }

        // 4. reload + 状态提示 + 通知所有 SshManagerPanel 刷新树
        self.reload(ctx);
        self.status = Some(StatusBanner::Saved);
        SshTreeChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(SshTreeChangedEvent::TreeChanged);
        });
        ctx.notify();
    }

    /// 触发 SSH 连接 — 把当前节点 + server 配置丢给 Workspace,后者开新
    /// terminal pane 跑 `ssh ...`。**优先用编辑器里的当前值**(可能用户改了
    /// 字段还没 Save),这样连接的是"用户屏幕上看到"的配置,而不是 DB 里旧的。
    fn on_connect(&mut self, ctx: &mut ViewContext<Self>) {
        let host = self.current_text(&self.host_editor.clone(), ctx);
        let port_str = self.current_text(&self.port_editor.clone(), ctx);
        let user = self.current_text(&self.user_editor.clone(), ctx);
        let key_path_text = self.current_text(&self.key_path_editor.clone(), ctx);
        let credential_id = self.selected_onekey_credential_id.clone();
        if self.auth_type == AuthType::OneKey && credential_id.is_none() {
            self.status = Some(StatusBanner::Error(crate::t!(
                "workspace-left-panel-ssh-manager-onekey-save-before-connect"
            )));
            ctx.notify();
            return;
        }
        let startup_command_text = self.current_text(&self.startup_command_editor.clone(), ctx);
        let notes_text = self.current_text(&self.notes_editor.clone(), ctx);

        let port: u16 = port_str.trim().parse().unwrap_or(22);
        let host = host.trim().to_string();
        if host.is_empty() {
            self.status = Some(StatusBanner::Error(crate::t!(
                "workspace-left-panel-ssh-manager-error-host-required"
            )));
            ctx.notify();
            return;
        }
        let key_path = key_path_text.trim().to_string();
        let server = SshServerInfo {
            node_id: self.node_id.clone(),
            host,
            port,
            username: user.trim().to_string(),
            auth_type: self.auth_type,
            key_path: if key_path.is_empty() {
                None
            } else {
                Some(key_path)
            },
            credential_id,
            startup_command: if startup_command_text.trim().is_empty() {
                None
            } else {
                Some(startup_command_text.trim().to_string())
            },
            notes: if notes_text.trim().is_empty() {
                None
            } else {
                Some(notes_text.trim().to_string())
            },
            last_connected_at: self.server.as_ref().and_then(|s| s.last_connected_at),
        };
        ctx.dispatch_typed_action(&crate::workspace::WorkspaceAction::OpenSshTerminal {
            node_id: self.node_id.clone(),
            server,
            target: SshConnectionTarget::NewTab,
        });
    }

    fn on_test_connection(&mut self, ctx: &mut ViewContext<Self>) {
        let host = self.current_text(&self.host_editor.clone(), ctx);
        let port_str = self.current_text(&self.port_editor.clone(), ctx);
        let user = self.current_text(&self.user_editor.clone(), ctx);
        let password = self.current_text(&self.password_editor.clone(), ctx);
        let key_path_text = self.current_text(&self.key_path_editor.clone(), ctx);
        let credential_id = self.selected_onekey_credential_id.clone();

        let port: u16 = port_str.trim().parse().unwrap_or(22);
        let host = host.trim().to_string();
        if host.is_empty() {
            self.status = Some(StatusBanner::Error(crate::t!(
                "workspace-left-panel-ssh-manager-error-host-required"
            )));
            ctx.notify();
            return;
        }

        let key_path = key_path_text.trim().to_string();
        let server = SshServerInfo {
            node_id: self.node_id.clone(),
            host,
            port,
            username: user.trim().to_string(),
            auth_type: self.auth_type,
            key_path: if key_path.is_empty() {
                None
            } else {
                Some(key_path)
            },
            credential_id,
            startup_command: None,
            notes: None,
            last_connected_at: None,
        };

        let (server, password) = match resolve_test_server_and_password(
            server,
            &self.onekey_credentials,
            &password,
            &KeychainSecretStore,
        ) {
            Ok(resolved) => resolved,
            Err(message) => {
                self.status = Some(StatusBanner::Error(message));
                ctx.notify();
                return;
            }
        };

        self.is_testing = true;
        self.status = None;
        ctx.notify();

        let node_id = self.node_id.clone();
        ctx.spawn(
            async move {
                let result =
                    warp_ssh_manager::ssh_command::test_connection(&server, password).await;
                (node_id, result)
            },
            |me, (_node_id, result), ctx| {
                me.is_testing = false;
                me.connection_status = result.status;
                me.latency_ms = result.latency_ms;
                match result.status {
                    ConnectionStatus::Online => {
                        let latency_str = result
                            .latency_ms
                            .map(|ms| format!("{ms}ms"))
                            .unwrap_or_else(|| "N/A".into());
                        let msg = result.error_message.unwrap_or_default();
                        if msg.contains("password auth required") {
                            me.status = Some(StatusBanner::Success(format!(
                                "Server reachable - latency: {latency_str}"
                            )));
                        } else {
                            me.status = Some(StatusBanner::Success(format!(
                                "Online - latency: {latency_str}"
                            )));
                        }
                    }
                    ConnectionStatus::Offline => {
                        me.latency_ms = None;
                        let err = result
                            .error_message
                            .unwrap_or_else(|| "Unknown error".into());
                        me.status = Some(StatusBanner::Error(err));
                    }
                    ConnectionStatus::Unknown => {
                        me.latency_ms = None;
                        me.status = None;
                    }
                }
                ctx.notify();
            },
        );
    }

    /// 打开系统文件选择器选私钥文件,选完写入 key_path editor。回调 ctx
    /// 是 ViewContext<Self>(框架自动维持原 view 上下文)。
    fn on_pick_key_file(&mut self, ctx: &mut ViewContext<Self>) {
        let editor = self.key_path_editor.clone();
        ctx.open_file_picker(
            move |result, ctx| match result {
                Ok(paths) => {
                    if let Some(path) = paths.into_iter().next() {
                        editor.update(ctx, |e, ctx| e.set_buffer_text(&path, ctx));
                    }
                }
                Err(e) => {
                    log::warn!("ssh: file picker failed: {e}");
                }
            },
            FilePickerConfiguration::new(),
        );
    }

    fn on_pick_onekey_key_file(&mut self, ctx: &mut ViewContext<Self>) {
        let editor = self.onekey_key_path_editor.clone();
        ctx.open_file_picker(
            move |result, ctx| match result {
                Ok(paths) => {
                    if let Some(path) = paths.into_iter().next() {
                        editor.update(ctx, |e, ctx| e.set_buffer_text(&path, ctx));
                    }
                }
                Err(e) => {
                    log::warn!("ssh: OneKey key file picker failed: {e}");
                }
            },
            FilePickerConfiguration::new(),
        );
    }

    fn on_set_auth(&mut self, auth: AuthType, ctx: &mut ViewContext<Self>) {
        if self.auth_type != auth {
            self.auth_type = auth;
            // 切换 auth 类型时清空密码 buffer — 密码和 passphrase 语义不同。
            self.password_editor
                .update(ctx, |e, ctx| e.set_buffer_text("", ctx));
            self.status = None;
            ctx.notify();
        }
    }

    fn on_save_managed_onekey_credential(&mut self, ctx: &mut ViewContext<Self>) {
        let label = self.current_text(&self.onekey_label_editor.clone(), ctx);
        let username = self.current_text(&self.onekey_user_editor.clone(), ctx);
        let secret = self.current_text(&self.password_editor.clone(), ctx);
        let key_path = self.current_text(&self.onekey_key_path_editor.clone(), ctx);

        let label = label.trim().to_string();
        if label.is_empty() {
            self.status = Some(StatusBanner::Error(crate::t!(
                "workspace-left-panel-ssh-manager-onekey-label-required"
            )));
            ctx.notify();
            return;
        }

        let key_path = key_path.trim().to_string();
        if self.managed_onekey_kind == OneKeyCredentialKind::Key && key_path.is_empty() {
            self.status = Some(StatusBanner::Error(crate::t!(
                "workspace-left-panel-ssh-manager-onekey-key-path-required"
            )));
            ctx.notify();
            return;
        }

        let key_path_for_db = match self.managed_onekey_kind {
            OneKeyCredentialKind::Password => None,
            OneKeyCredentialKind::Key => Some(key_path),
        };
        let username = username.trim().to_string();
        let credential_result = if let Some(id) = self.managed_onekey_credential_id.clone() {
            let Some(existing) = self
                .onekey_credentials
                .iter()
                .find(|credential| credential.id == id)
                .cloned()
            else {
                self.status = Some(StatusBanner::Error(crate::t!(
                    "workspace-left-panel-ssh-manager-onekey-select-required"
                )));
                ctx.notify();
                return;
            };
            let mut credential = existing;
            credential.label = label;
            credential.username = username;
            credential.kind = self.managed_onekey_kind;
            credential.key_path = key_path_for_db;
            warp_ssh_manager::with_conn(move |conn| {
                SshRepository::update_onekey_credential(conn, &credential)?;
                credential = SshRepository::get_onekey_credential(conn, &id)?
                    .ok_or_else(|| warp_ssh_manager::SshRepositoryError::NotFound(id.clone()))?;
                Ok(credential)
            })
        } else {
            let kind = self.managed_onekey_kind;
            warp_ssh_manager::with_conn(move |conn| {
                Ok(SshRepository::create_onekey_credential(
                    conn,
                    &label,
                    &username,
                    kind,
                    key_path_for_db.as_deref(),
                )?)
            })
        };

        let credential = match credential_result {
            Ok(credential) => credential,
            Err(e) => {
                log::error!("ssh_server_view: save OneKey credential failed: {e:?}");
                self.status = Some(StatusBanner::Error(format!("{e}")));
                ctx.notify();
                return;
            }
        };

        if !secret.is_empty() {
            let kind = secret_kind_for_onekey_credential(credential.kind);
            if let Err(e) = KeychainSecretStore.set(&credential.id, kind, &secret) {
                log::error!("ssh_server_view: OneKey keychain write failed: {e:?}");
                self.status = Some(StatusBanner::Error(format!("keychain: {e}")));
                ctx.notify();
                return;
            }
            self.password_editor
                .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
        }

        self.managed_onekey_credential_id = Some(credential.id.clone());
        self.selected_onekey_credential_id = Some(credential.id);
        self.reload_onekey_credentials(ctx);
        if let Some(selected) = self.selected_onekey_credential_id.as_ref().and_then(|id| {
            self.onekey_credentials
                .iter()
                .find(|credential| credential.id == *id)
                .cloned()
        }) {
            self.set_managed_onekey_form_from_credential(&selected, ctx);
        }
        self.status = Some(StatusBanner::Saved);
        ctx.notify();
    }

    fn on_delete_managed_onekey_credential(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(id) = self.managed_onekey_credential_id.clone() else {
            return;
        };

        if let Err(e) = warp_ssh_manager::with_conn(|conn| {
            SshRepository::delete_onekey_credential(conn, &id)?;
            Ok(())
        }) {
            log::error!("ssh_server_view: delete OneKey credential failed: {e:?}");
            self.status = Some(StatusBanner::Error(format!("{e}")));
            ctx.notify();
            return;
        }

        let store = KeychainSecretStore;
        for kind in [SecretKind::OneKeyPassword, SecretKind::Passphrase] {
            if let Err(e) = store.delete(&id, kind) {
                log::warn!("ssh_server_view: delete OneKey secret failed: {e:?}");
            }
        }
        if self.selected_onekey_credential_id.as_deref() == Some(id.as_str()) {
            self.selected_onekey_credential_id = None;
        }
        self.clear_managed_onekey_form(ctx);
        self.reload_onekey_credentials(ctx);
        ctx.notify();
    }

    fn on_clear_machine_memory(&mut self, ctx: &mut ViewContext<Self>) {
        let machine_key = self
            .machine_memory
            .as_ref()
            .map(|memory| memory.machine_key.clone())
            .or_else(|| self.server.as_ref().and_then(machine_key_for_server));
        let Some(machine_key) = machine_key else {
            self.show_clear_machine_memory_confirmation = false;
            ctx.notify();
            return;
        };

        let result = warp_ssh_manager::with_conn(|conn| {
            MachineMemoryRepository::delete(conn, &machine_key)?;
            Ok(())
        });
        self.show_clear_machine_memory_confirmation = false;
        if let Err(error) = result {
            log::error!("ssh_server_view: clear machine memory failed: {error:?}");
            self.status = Some(StatusBanner::Error(format!("{error}")));
            ctx.notify();
            return;
        }

        self.reload_machine_memory();
        SshTreeChangedNotifier::handle(ctx).update(ctx, |_, ctx| {
            ctx.emit(SshTreeChangedEvent::TreeChanged);
        });
        ctx.notify();
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

    fn render_text_field(
        &self,
        label: &str,
        editor: &ViewHandle<EditorView>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_input = appearance
            .ui_builder()
            .text_input(editor.clone())
            .with_style(UiComponentStyles {
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
            })
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

    /// 私钥路径字段:label + (输入框 + 浏览按钮) 一行。
    fn render_key_path_field(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_input = appearance
            .ui_builder()
            .text_input(self.key_path_editor.clone())
            .with_style(UiComponentStyles {
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
            })
            .build()
            .finish();

        let icon_color = theme.sub_text_color(theme.background());
        let icon_el = ConstrainedBox::new(
            crate::ui_components::icons::Icon::Folder
                .to_warpui_icon(icon_color)
                .finish(),
        )
        .with_width(16.0)
        .with_height(16.0)
        .finish();
        let browse_btn = Hoverable::new(self.key_path_picker_btn_state.clone(), move |_| {
            Container::new(
                ConstrainedBox::new(icon_el)
                    .with_width(32.0)
                    .with_height(32.0)
                    .finish(),
            )
            .with_uniform_padding(2.0)
            .with_background(theme.surface_2())
            .with_border(
                warpui::elements::Border::all(1.0)
                    .with_border_color(internal_colors::neutral_3(theme)),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(SshServerAction::PickKeyFile);
        })
        .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(Shrinkable::new(1.0, text_input).finish())
            .with_child(browse_btn)
            .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.render_label(
                    &crate::t!("workspace-left-panel-ssh-manager-detail-key-path"),
                    appearance,
                ))
                .with_child(row)
                .finish(),
        )
        .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
        .finish()
    }

    fn render_onekey_key_path_field(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_input = appearance
            .ui_builder()
            .text_input(self.onekey_key_path_editor.clone())
            .with_style(UiComponentStyles {
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
            })
            .build()
            .finish();

        let icon_color = theme.sub_text_color(theme.background());
        let icon_el = ConstrainedBox::new(
            crate::ui_components::icons::Icon::Folder
                .to_warpui_icon(icon_color)
                .finish(),
        )
        .with_width(16.0)
        .with_height(16.0)
        .finish();
        let browse_btn = Hoverable::new(self.onekey_key_path_picker_btn_state.clone(), move |_| {
            Container::new(
                ConstrainedBox::new(icon_el)
                    .with_width(32.0)
                    .with_height(32.0)
                    .finish(),
            )
            .with_uniform_padding(2.0)
            .with_background(theme.surface_2())
            .with_border(Border::all(1.0).with_border_color(internal_colors::neutral_3(theme)))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(SshServerAction::PickOneKeyKeyFile);
        })
        .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.0)
            .with_child(Shrinkable::new(1.0, text_input).finish())
            .with_child(browse_btn)
            .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.render_label(
                    &crate::t!("workspace-left-panel-ssh-manager-onekey-key-path"),
                    appearance,
                ))
                .with_child(row)
                .finish(),
        )
        .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
        .finish()
    }

    fn render_auth_toggle(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let make_pill = |label: String,
                         active: bool,
                         state: MouseStateHandle,
                         action: SshServerAction|
         -> Box<dyn Element> {
            let main_color = if active {
                theme.main_text_color(theme.accent())
            } else {
                theme.sub_text_color(theme.background())
            };
            let bg = if active {
                theme.accent()
            } else {
                theme.surface_2()
            };
            let label_el = Text::new_inline(
                label,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(main_color.into())
            .finish();

            Hoverable::new(state, move |_| {
                Container::new(label_el)
                    .with_padding_left(AUTH_TOGGLE_PADDING_H)
                    .with_padding_right(AUTH_TOGGLE_PADDING_H)
                    .with_padding_top(AUTH_TOGGLE_PADDING_V)
                    .with_padding_bottom(AUTH_TOGGLE_PADDING_V)
                    .with_background(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action))
            .finish()
        };

        let mut auth_row = Wrap::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_run_spacing(8.0)
            .with_main_axis_size(MainAxisSize::Min);
        for auth_type in auth_toggle_options() {
            auth_row.add_child(make_pill(
                auth_toggle_label(auth_type),
                self.auth_type == auth_type,
                self.auth_toggle_button_state(auth_type),
                auth_toggle_action(auth_type),
            ));
        }

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.render_label(
                    &crate::t!("workspace-left-panel-ssh-manager-detail-auth"),
                    appearance,
                ))
                .with_child(auth_row.finish())
                .finish(),
        )
        .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
        .finish()
    }

    fn auth_toggle_button_state(&self, auth_type: AuthType) -> MouseStateHandle {
        match auth_type {
            AuthType::Password => self.auth_password_btn_state.clone(),
            AuthType::Key => self.auth_key_btn_state.clone(),
            AuthType::OneKey => self.auth_onekey_btn_state.clone(),
        }
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
            .with_centered_text_label(crate::t!("workspace-left-panel-ssh-manager-save"))
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(SshServerAction::Save))
            .finish()
    }

    fn render_connect_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, self.connect_btn_state.clone())
            .with_style(UiComponentStyles {
                font_weight: Some(Weight::Bold),
                width: Some(SAVE_BUTTON_WIDTH),
                height: Some(SAVE_BUTTON_HEIGHT),
                font_size: Some(13.0),
                ..Default::default()
            })
            .with_centered_text_label(crate::t!("workspace-left-panel-ssh-manager-connect"))
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(SshServerAction::Connect))
            .finish()
    }

    fn render_test_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = if self.is_testing {
            crate::t!("workspace-left-panel-ssh-manager-testing")
        } else {
            crate::t!("workspace-left-panel-ssh-manager-test")
        };
        appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, self.test_btn_state.clone())
            .with_style(UiComponentStyles {
                font_weight: Some(Weight::Bold),
                width: Some(SAVE_BUTTON_WIDTH),
                height: Some(SAVE_BUTTON_HEIGHT),
                font_size: Some(13.0),
                ..Default::default()
            })
            .with_centered_text_label(label)
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(SshServerAction::TestConnection))
            .finish()
    }

    fn render_connection_status(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg = theme.background();
        let (icon, color, text) = match self.connection_status {
            ConnectionStatus::Online => {
                let latency_str = self
                    .latency_ms
                    .map(|ms| format!(" ({ms}ms)"))
                    .unwrap_or_default();
                (
                    "●",
                    theme.ui_green_color().into(),
                    format!(
                        "{}{latency_str}",
                        crate::t!("workspace-left-panel-ssh-manager-status-online")
                    ),
                )
            }
            ConnectionStatus::Offline => (
                "●",
                theme.ui_error_color().into(),
                crate::t!("workspace-left-panel-ssh-manager-status-offline"),
            ),
            ConnectionStatus::Unknown => (
                "○",
                theme.sub_text_color(bg),
                crate::t!("workspace-left-panel-ssh-manager-status-unknown"),
            ),
        };

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.0)
            .with_child(
                Text::new_inline(icon, appearance.ui_font_family(), 12.0)
                    .with_color(color.into())
                    .finish(),
            )
            .with_child(
                Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
                    .with_color(color.into())
                    .finish(),
            )
            .with_main_axis_size(MainAxisSize::Min)
            .finish()
    }

    fn render_status_banner(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let (text, color) = match self.status.as_ref()? {
            StatusBanner::Saved => (
                crate::t!("workspace-left-panel-ssh-manager-status-saved"),
                theme.ui_green_color(),
            ),
            StatusBanner::Success(msg) => (msg.clone(), theme.ui_green_color()),
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

    fn render_machine_memory_content(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color = theme.main_text_color(background).into_solid();
        let content = self
            .machine_memory
            .as_ref()
            .map(|memory| memory.content.as_str())
            .filter(|content| !content.trim().is_empty());

        match content {
            Some(content) => match parse_markdown(content) {
                Ok(formatted) => FormattedTextElement::new(
                    formatted,
                    appearance.ui_font_size(),
                    appearance.ui_font_family(),
                    appearance.monospace_font_family(),
                    text_color,
                    Default::default(),
                )
                .with_heading_to_font_size_multipliers(
                    appearance.heading_font_size_multipliers().clone(),
                )
                .with_hyperlink_font_color(theme.accent().into_solid())
                .set_selectable(true)
                .finish(),
                Err(error) => {
                    log::warn!("ssh_server_view: parse machine memory markdown failed: {error:?}");
                    Text::new(
                        content.to_string(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(text_color)
                    .with_selectable(true)
                    .finish()
                }
            },
            None => Text::new(
                crate::t!("workspace-left-panel-ssh-manager-memory-empty"),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.sub_text_color(background).into_solid())
            .with_selectable(true)
            .finish(),
        }
    }

    fn render_machine_memory_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(self.render_label(
                &crate::t!("workspace-left-panel-ssh-manager-memory"),
                appearance,
            ));
        if self.machine_memory.is_some() {
            let clear_button = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Warn,
                    self.clear_machine_memory_btn_state.clone(),
                )
                .with_style(UiComponentStyles {
                    height: Some(SAVE_BUTTON_HEIGHT),
                    font_size: Some(13.0),
                    ..Default::default()
                })
                .with_centered_text_label(crate::t!(
                    "workspace-left-panel-ssh-manager-memory-clear"
                ))
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(SshServerAction::OpenClearMachineMemoryConfirmation)
                })
                .finish();
            header.add_child(clear_button);
        }

        let mut section = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        section.add_child(header.finish());
        if let Some(memory) = self.machine_memory.as_ref() {
            section.add_child(
                Container::new(
                    Text::new_inline(
                        crate::t!(
                            "workspace-left-panel-ssh-manager-memory-updated",
                            updated_at = memory.updated_at.to_rfc3339()
                        ),
                        appearance.ui_font_family(),
                        12.0,
                    )
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
                )
                .with_margin_bottom(6.0)
                .finish(),
            );
        }

        let memory_content = Container::new(self.render_machine_memory_content(appearance))
            .with_uniform_padding(12.0)
            .finish();
        let scrollable = ConstrainedBox::new(
            ClippedScrollable::vertical(
                self.machine_memory_scroll_state.clone(),
                memory_content,
                ScrollbarWidth::Auto,
                theme.disabled_text_color(theme.surface_2()).into(),
                theme.main_text_color(theme.surface_2()).into(),
                Fill::None,
            )
            .finish(),
        )
        .with_height(MACHINE_MEMORY_CONTENT_HEIGHT)
        .finish();
        section.add_child(
            Container::new(scrollable)
                .with_background(theme.surface_2())
                .with_border(Border::all(1.0).with_border_fill(internal_colors::neutral_3(theme)))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                .finish(),
        );

        Container::new(section.finish())
            .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
            .finish()
    }

    fn render_clear_machine_memory_confirmation(
        &self,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        use crate::ui_components::dialog::{Dialog, dialog_styles};

        let cancel_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.clear_machine_memory_cancel_btn_state.clone(),
            )
            .with_text_label(crate::t!("common-cancel"))
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(SshServerAction::CancelClearMachineMemoryConfirmation)
            })
            .finish();
        let clear_button = Container::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Warn,
                    self.clear_machine_memory_confirm_btn_state.clone(),
                )
                .with_text_label(crate::t!(
                    "workspace-left-panel-ssh-manager-memory-clear-confirm-button"
                ))
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(SshServerAction::ConfirmClearMachineMemory)
                })
                .finish(),
        )
        .with_margin_left(12.0)
        .finish();
        let dialog = Dialog::new(
            crate::t!("workspace-left-panel-ssh-manager-memory-clear-confirm-title"),
            Some(crate::t!(
                "workspace-left-panel-ssh-manager-memory-clear-confirm-description"
            )),
            dialog_styles(appearance),
        )
        .with_bottom_row_child(cancel_button)
        .with_bottom_row_child(clear_button)
        .with_width(MACHINE_MEMORY_DIALOG_WIDTH)
        .build()
        .finish();

        Dismiss::new(dialog)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(SshServerAction::CancelClearMachineMemoryConfirmation);
            })
            .finish()
    }

    /// 分组下拉字段:label + dropdown。
    fn render_group_field(&self, appearance: &Appearance) -> Box<dyn Element> {
        let label = self.render_label(
            &crate::t!("workspace-left-panel-ssh-manager-field-group"),
            appearance,
        );
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(label)
                .with_child(ChildView::new(&self.group_dropdown).finish())
                .finish(),
        )
        .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
        .finish()
    }

    fn render_onekey_credential_field(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let label = self.render_label(
            &crate::t!("workspace-left-panel-ssh-manager-onekey-credential"),
            appearance,
        );
        let icon =
            warpui::elements::Icon::new("bundled/svg/gear.svg", theme.active_ui_text_color());
        let manager_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.onekey_manager_btn_state.clone(),
            )
            .with_icon_label(icon)
            .with_style(UiComponentStyles {
                font_color: Some(theme.active_ui_text_color().into_solid()),
                width: Some(34.0),
                height: Some(34.0),
                padding: Some(Coords::uniform(7.0)),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SshServerAction::OpenOneKeyManager)
            })
            .finish();
        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(
                Shrinkable::new(
                    1.0,
                    ChildView::new(&self.onekey_credential_dropdown).finish(),
                )
                .finish(),
            )
            .with_child(manager_button)
            .finish();
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(label)
                .with_child(row)
                .finish(),
        )
        .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
        .finish()
    }

    fn render_onekey_kind_toggle(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let make_pill = |label: String,
                         active: bool,
                         state: MouseStateHandle,
                         action: SshServerAction|
         -> Box<dyn Element> {
            let main_color = if active {
                theme.main_text_color(theme.accent())
            } else {
                theme.sub_text_color(theme.background())
            };
            let bg = if active {
                theme.accent()
            } else {
                theme.surface_2()
            };
            let label_el = Text::new_inline(
                label,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(main_color.into())
            .finish();

            Hoverable::new(state, move |_| {
                Container::new(label_el)
                    .with_padding_left(AUTH_TOGGLE_PADDING_H)
                    .with_padding_right(AUTH_TOGGLE_PADDING_H)
                    .with_padding_top(AUTH_TOGGLE_PADDING_V)
                    .with_padding_bottom(AUTH_TOGGLE_PADDING_V)
                    .with_background(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action))
            .finish()
        };

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(make_pill(
                crate::t!("workspace-left-panel-ssh-manager-onekey-type-password"),
                self.managed_onekey_kind == OneKeyCredentialKind::Password,
                self.onekey_manager_password_btn_state.clone(),
                SshServerAction::SetManagedOneKeyPassword,
            ))
            .with_child(make_pill(
                crate::t!("workspace-left-panel-ssh-manager-onekey-type-key"),
                self.managed_onekey_kind == OneKeyCredentialKind::Key,
                self.onekey_manager_key_btn_state.clone(),
                SshServerAction::SetManagedOneKeyKey,
            ))
            .finish();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.render_label(
                    &crate::t!("workspace-left-panel-ssh-manager-onekey-type"),
                    appearance,
                ))
                .with_child(row)
                .finish(),
        )
        .with_margin_bottom(FIELD_BLOCK_MARGIN_BOTTOM)
        .finish()
    }

    fn render_onekey_manager_row(
        &self,
        index: usize,
        credential: &SshOneKeyCredential,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let is_selected = self.managed_onekey_credential_id.as_deref() == Some(&credential.id);
        let bg = if is_selected {
            theme.surface_3()
        } else {
            theme.surface_2()
        };
        let title_color = if is_selected {
            theme.active_ui_text_color()
        } else {
            theme.main_text_color(theme.background())
        };
        let subtitle = match credential.kind {
            OneKeyCredentialKind::Password => credential.username.clone(),
            OneKeyCredentialKind::Key => credential
                .key_path
                .as_deref()
                .unwrap_or_default()
                .to_string(),
        };
        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Text::new_inline(
                    credential.label.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(title_color.into())
                .finish(),
            );
        if !subtitle.is_empty() {
            content = content.with_child(
                Text::new_inline(subtitle, appearance.ui_font_family(), 12.0)
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish(),
            );
        }
        let state = self
            .onekey_manager_row_states
            .get(index)
            .cloned()
            .unwrap_or_default();
        Hoverable::new(state, {
            let content = content.finish();
            move |_| {
                Container::new(content)
                    .with_uniform_padding(8.0)
                    .with_background(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                    .finish()
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(SshServerAction::SelectManagedOneKeyCredential(Some(index)))
        })
        .finish()
    }

    fn render_onekey_manager(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let title = Text::new_inline(
            crate::t!("workspace-left-panel-ssh-manager-onekey-manager-title"),
            appearance.ui_font_family(),
            appearance.ui_font_heading_2(),
        )
        .with_color(theme.main_text_color(theme.background()).into())
        .finish();
        let close_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Text,
                self.onekey_manager_close_btn_state.clone(),
            )
            .with_icon_label(warpui::elements::Icon::new(
                "bundled/svg/x-close.svg",
                theme.active_ui_text_color(),
            ))
            .with_style(UiComponentStyles {
                font_color: Some(theme.active_ui_text_color().into_solid()),
                width: Some(28.0),
                height: Some(28.0),
                padding: Some(Coords::uniform(6.0)),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SshServerAction::CloseOneKeyManager)
            })
            .finish();
        let header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(title)
            .with_child(close_button)
            .finish();

        let add_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.onekey_manager_new_btn_state.clone(),
            )
            .with_centered_text_label(crate::t!("workspace-left-panel-ssh-manager-onekey-add"))
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SshServerAction::NewOneKeyCredential)
            })
            .finish();
        let mut list = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        list.add_child(Container::new(add_button).with_margin_bottom(8.0).finish());
        for (index, credential) in self.onekey_credentials.iter().enumerate() {
            list.add_child(
                Container::new(self.render_onekey_manager_row(index, credential, appearance))
                    .with_margin_bottom(4.0)
                    .finish(),
            );
        }
        let list_panel = ConstrainedBox::new(
            Container::new(list.finish())
                .with_padding_right(12.0)
                .finish(),
        )
        .with_width(ONEKEY_MANAGER_LIST_WIDTH)
        .finish();

        let secret_label = match self.managed_onekey_kind {
            OneKeyCredentialKind::Password => {
                crate::t!("workspace-left-panel-ssh-manager-onekey-secret")
            }
            OneKeyCredentialKind::Key => {
                crate::t!("workspace-left-panel-ssh-manager-passphrase")
            }
        };
        let mut form = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        form.add_child(self.render_text_field(
            &crate::t!("workspace-left-panel-ssh-manager-onekey-label"),
            &self.onekey_label_editor,
            appearance,
        ));
        form.add_child(self.render_text_field(
            &crate::t!("workspace-left-panel-ssh-manager-onekey-user"),
            &self.onekey_user_editor,
            appearance,
        ));
        form.add_child(self.render_onekey_kind_toggle(appearance));
        if self.managed_onekey_kind == OneKeyCredentialKind::Key {
            form.add_child(self.render_onekey_key_path_field(appearance));
        }
        form.add_child(self.render_text_field(&secret_label, &self.password_editor, appearance));

        let save_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.onekey_manager_save_btn_state.clone(),
            )
            .with_centered_text_label(crate::t!("workspace-left-panel-ssh-manager-onekey-save"))
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SshServerAction::SaveManagedOneKeyCredential)
            })
            .finish();
        let mut footer = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_spacing(8.0);
        if self.managed_onekey_credential_id.is_some() {
            let delete_button = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Warn,
                    self.onekey_manager_delete_btn_state.clone(),
                )
                .with_centered_text_label(crate::t!(
                    "workspace-left-panel-ssh-manager-onekey-delete"
                ))
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(SshServerAction::DeleteManagedOneKeyCredential)
                })
                .finish();
            footer.add_child(delete_button);
        }
        footer.add_child(save_button);
        form.add_child(footer.finish());

        let body = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(list_panel)
            .with_child(Shrinkable::new(1.0, form.finish()).finish())
            .finish();

        let panel = ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(Container::new(header).with_margin_bottom(16.0).finish())
                    .with_child(Shrinkable::new(1.0, body).finish())
                    .finish(),
            )
            .with_uniform_padding(20.0)
            .with_background(theme.background())
            .with_border(Border::all(1.0).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.0)))
            .finish(),
        )
        .with_width(ONEKEY_MANAGER_WIDTH)
        .with_height(ONEKEY_MANAGER_HEIGHT)
        .finish();

        let panel = Hoverable::new(MouseStateHandle::default(), move |_| panel)
            .on_mouse_down(|_, _, _| {})
            .finish();

        Dismiss::new(panel)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(SshServerAction::CloseOneKeyManager);
            })
            .finish()
    }
}

fn make_editor(
    is_password: bool,
    placeholder: &str,
    ctx: &mut ViewContext<SshServerView>,
) -> ViewHandle<EditorView> {
    let placeholder = placeholder.to_string();
    ctx.add_typed_action_view(move |ctx| {
        let options = {
            let appearance = Appearance::as_ref(ctx);
            let theme = appearance.theme();
            SingleLineEditorOptions {
                is_password,
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

fn machine_key_for_server(server: &SshServerInfo) -> Option<String> {
    let port = server.port.to_string();
    resolve_machine_key(Some(&server.host), Some(&port))
}

fn password_lookup_for_server_form(server: &SshServerInfo) -> (Option<String>, SecretKind) {
    match server.auth_type {
        AuthType::Password => (Some(server.node_id.clone()), SecretKind::Password),
        AuthType::Key => (Some(server.node_id.clone()), SecretKind::Passphrase),
        AuthType::OneKey => (server.credential_id.clone(), SecretKind::OneKeyPassword),
    }
}

fn secret_kind_for_onekey_credential(kind: OneKeyCredentialKind) -> SecretKind {
    match kind {
        OneKeyCredentialKind::Password => SecretKind::OneKeyPassword,
        OneKeyCredentialKind::Key => SecretKind::Passphrase,
    }
}

fn auth_toggle_options() -> [AuthType; 3] {
    [AuthType::Password, AuthType::Key, AuthType::OneKey]
}

fn auth_specific_fields(auth_type: AuthType) -> Vec<AuthSpecificField> {
    match auth_type {
        AuthType::Password => vec![AuthSpecificField::Password],
        AuthType::Key => vec![AuthSpecificField::KeyPath, AuthSpecificField::Passphrase],
        AuthType::OneKey => vec![AuthSpecificField::OneKeyCredential],
    }
}

fn auth_toggle_action(auth_type: AuthType) -> SshServerAction {
    match auth_type {
        AuthType::Password => SshServerAction::SetAuthPassword,
        AuthType::Key => SshServerAction::SetAuthKey,
        AuthType::OneKey => SshServerAction::SetAuthOneKey,
    }
}

fn auth_toggle_label(auth_type: AuthType) -> String {
    match auth_type {
        AuthType::Password => crate::t!("workspace-left-panel-ssh-manager-auth-password"),
        AuthType::Key => crate::t!("workspace-left-panel-ssh-manager-auth-key"),
        AuthType::OneKey => crate::t!("workspace-left-panel-ssh-manager-auth-onekey"),
    }
}

impl Entity for SshServerView {
    type Event = PaneEvent;
}

impl TypedActionView for SshServerView {
    type Action = SshServerAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshServerAction::Save => self.on_save(ctx),
            SshServerAction::Connect => self.on_connect(ctx),
            SshServerAction::TestConnection => self.on_test_connection(ctx),
            SshServerAction::SetAuthPassword => self.on_set_auth(AuthType::Password, ctx),
            SshServerAction::SetAuthKey => self.on_set_auth(AuthType::Key, ctx),
            SshServerAction::SetAuthOneKey => self.on_set_auth(AuthType::OneKey, ctx),
            SshServerAction::PickKeyFile => self.on_pick_key_file(ctx),
            SshServerAction::PickOneKeyKeyFile => self.on_pick_onekey_key_file(ctx),
            SshServerAction::OpenOneKeyManager => {
                if self.managed_onekey_credential_id.is_none() {
                    self.sync_managed_onekey_selection(ctx);
                }
                self.show_onekey_manager = true;
                ctx.notify();
            }
            SshServerAction::CloseOneKeyManager => {
                self.show_onekey_manager = false;
                ctx.notify();
            }
            SshServerAction::NewOneKeyCredential => {
                self.clear_managed_onekey_form(ctx);
                ctx.notify();
            }
            SshServerAction::SelectManagedOneKeyCredential(index) => {
                if let Some(credential) =
                    index.and_then(|i| self.onekey_credentials.get(i).cloned())
                {
                    self.set_managed_onekey_form_from_credential(&credential, ctx);
                } else {
                    self.clear_managed_onekey_form(ctx);
                }
                ctx.notify();
            }
            SshServerAction::SetManagedOneKeyPassword => {
                if self.managed_onekey_kind != OneKeyCredentialKind::Password {
                    self.managed_onekey_kind = OneKeyCredentialKind::Password;
                    self.onekey_key_path_editor
                        .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
                    self.password_editor
                        .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
                    ctx.notify();
                }
            }
            SshServerAction::SetManagedOneKeyKey => {
                if self.managed_onekey_kind != OneKeyCredentialKind::Key {
                    self.managed_onekey_kind = OneKeyCredentialKind::Key;
                    self.password_editor
                        .update(ctx, |editor, ctx| editor.set_buffer_text("", ctx));
                    ctx.notify();
                }
            }
            SshServerAction::SaveManagedOneKeyCredential => {
                self.on_save_managed_onekey_credential(ctx)
            }
            SshServerAction::DeleteManagedOneKeyCredential => {
                self.on_delete_managed_onekey_credential(ctx)
            }
            SshServerAction::OpenClearMachineMemoryConfirmation => {
                self.show_clear_machine_memory_confirmation = true;
                ctx.notify();
            }
            SshServerAction::CancelClearMachineMemoryConfirmation => {
                self.show_clear_machine_memory_confirmation = false;
                ctx.notify();
            }
            SshServerAction::ConfirmClearMachineMemory => self.on_clear_machine_memory(ctx),
            SshServerAction::SelectGroup(index) => {
                let new_group_id =
                    index.and_then(|i| self.folders.get(i).map(|(id, _)| id.clone()));
                if new_group_id != self.current_group_id {
                    self.current_group_id = new_group_id;
                    ctx.notify();
                }
            }
            SshServerAction::SelectOneKeyCredential(index) => {
                let selected = index.and_then(|i| self.onekey_credentials.get(i).cloned());
                self.selected_onekey_credential_id =
                    selected.as_ref().map(|credential| credential.id.clone());
                if let Some(credential) = selected {
                    if self.managed_onekey_credential_id.is_none() || !self.show_onekey_manager {
                        self.set_managed_onekey_form_from_credential(&credential, ctx);
                    }
                } else {
                    if !self.show_onekey_manager {
                        self.clear_managed_onekey_form(ctx);
                    }
                }
                self.rebuild_onekey_credential_dropdown(ctx);
                ctx.notify();
            }
        }
    }
}

impl View for SshServerView {
    fn ui_name() -> &'static str {
        "SshServerView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // folder 节点 / 找不到 server → 简单提示 + 隐藏表单
        if !matches!(self.node.as_ref().map(|n| n.kind), Some(NodeKind::Server)) {
            let body_text = match self.node.as_ref().map(|n| n.kind) {
                Some(NodeKind::Folder) => {
                    crate::t!("workspace-left-panel-ssh-manager-pane-folder-body")
                }
                _ => crate::t!("workspace-left-panel-ssh-manager-server-missing"),
            };
            let theme = appearance.theme();
            let body = Text::new_inline(
                body_text,
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
        }

        // ---- header row: title + 右侧 Save 按钮 + status banner ----
        let title_text = self
            .node
            .as_ref()
            .map(|n| n.name.clone())
            .unwrap_or_default();
        let title = Text::new_inline(
            title_text,
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

        // Title 在左 / [Test] [Connect] [Save] 按钮在右。
        let buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.0)
            .with_child(self.render_test_button(appearance))
            .with_child(self.render_connect_button(appearance))
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

        col.add_child(
            Container::new(self.render_connection_status(appearance))
                .with_margin_bottom(8.0)
                .finish(),
        );

        if let Some(banner) = self.render_status_banner(appearance) {
            col.add_child(banner);
        }

        col.add_child(self.render_text_field(
            &crate::t!("workspace-left-panel-ssh-manager-field-name"),
            &self.name_editor,
            appearance,
        ));

        // 分组下拉
        col.add_child(self.render_group_field(appearance));

        col.add_child(self.render_text_field(
            &crate::t!("workspace-left-panel-ssh-manager-detail-host"),
            &self.host_editor,
            appearance,
        ));
        col.add_child(self.render_text_field(
            &crate::t!("workspace-left-panel-ssh-manager-detail-port"),
            &self.port_editor,
            appearance,
        ));
        if self.auth_type != AuthType::OneKey {
            col.add_child(self.render_text_field(
                &crate::t!("workspace-left-panel-ssh-manager-detail-user"),
                &self.user_editor,
                appearance,
            ));
        }
        col.add_child(self.render_auth_toggle(appearance));

        for field in auth_specific_fields(self.auth_type) {
            match field {
                AuthSpecificField::Password => {
                    col.add_child(self.render_text_field(
                        &crate::t!("workspace-left-panel-ssh-manager-auth-password"),
                        &self.password_editor,
                        appearance,
                    ));
                }
                AuthSpecificField::KeyPath => {
                    col.add_child(self.render_key_path_field(appearance));
                }
                AuthSpecificField::Passphrase => {
                    col.add_child(self.render_text_field(
                        &crate::t!("workspace-left-panel-ssh-manager-passphrase"),
                        &self.password_editor,
                        appearance,
                    ));
                }
                AuthSpecificField::OneKeyCredential => {
                    col.add_child(self.render_onekey_credential_field(appearance));
                }
            }
        }

        // 启动命令
        col.add_child(self.render_text_field(
            &crate::t!("workspace-left-panel-ssh-manager-startup-command"),
            &self.startup_command_editor,
            appearance,
        ));
        // Root 密码
        col.add_child(self.render_text_field(
            &crate::t!("workspace-left-panel-ssh-manager-root-password"),
            &self.root_password_editor,
            appearance,
        ));
        // 备注
        col.add_child(self.render_text_field(
            &crate::t!("workspace-left-panel-ssh-manager-notes"),
            &self.notes_editor,
            appearance,
        ));
        // AI 记忆与人工备注分离，只读展示并单独滚动。
        col.add_child(self.render_machine_memory_section(appearance));

        let theme = appearance.theme();
        let inner = ConstrainedBox::new(
            Container::new(col.finish())
                .with_uniform_padding(24.0)
                .finish(),
        )
        .with_max_width(640.0)
        .finish();

        // 用 ClippedScrollable 包一层,内容溢出时垂直滚动,避免和下方 pane 重叠。
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
        if self.show_onekey_manager || self.show_clear_machine_memory_confirmation {
            let mut stack = Stack::new().with_child(content);
            if self.show_onekey_manager {
                stack.add_positioned_overlay_child(
                    self.render_onekey_manager(appearance),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0.0, 0.0),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::Center,
                        ChildAnchor::Center,
                    ),
                );
            }
            if self.show_clear_machine_memory_confirmation {
                stack.add_positioned_overlay_child(
                    self.render_clear_machine_memory_confirmation(appearance),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0.0, 0.0),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::Center,
                        ChildAnchor::Center,
                    ),
                );
            }
            stack.finish()
        } else {
            content
        }
    }
}

impl BackingView for SshServerView {
    type PaneHeaderOverflowMenuAction = SshServerAction;
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
        ctx.focus(&self.host_editor);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        let title = self
            .node
            .as_ref()
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "SSH server".to_string());
        view::HeaderContent::simple(title)
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

/// 解析"测试连接"用的密码来源,优先级固化:
/// 1. form 文本非空 → 用 form 值(用户已敲,**不要求**先 Save)
/// 2. form 空 + keychain 有 → 用 keychain 存的值
/// 3. form 空 + keychain 无/错 → `None`,后端会返 "Password not provided"
///
/// form 永远胜过 keychain — 用户改 host/port 后想测,正在敲的密码就是
/// 新 host 的,不应被旧 keychain 值盖掉。
///
/// author: logic
/// date: 2026-06-01
fn resolve_test_server_and_password(
    mut server: SshServerInfo,
    onekey_credentials: &[SshOneKeyCredential],
    editor_text: &str,
    store: &dyn SshSecretStore,
) -> Result<(SshServerInfo, Option<Zeroizing<String>>), String> {
    let (secret_lookup_id, secret_kind) = if server.auth_type == AuthType::OneKey {
        let credential_id = server
            .credential_id
            .as_ref()
            .ok_or_else(|| crate::t!("workspace-left-panel-ssh-manager-onekey-select-required"))?;
        let credential = onekey_credentials
            .iter()
            .find(|credential| credential.id == *credential_id)
            .ok_or_else(|| crate::t!("workspace-left-panel-ssh-manager-onekey-select-required"))?;
        server.username = credential.username.clone();
        server.auth_type = match credential.kind {
            OneKeyCredentialKind::Password => AuthType::Password,
            OneKeyCredentialKind::Key => AuthType::Key,
        };
        server.key_path = credential.key_path.clone();
        (
            Some(credential.id.clone()),
            secret_kind_for_onekey_credential(credential.kind),
        )
    } else {
        password_lookup_for_server_form(&server)
    };
    let password =
        resolve_test_password(secret_lookup_id.as_deref(), secret_kind, editor_text, store);
    Ok((server, password))
}

fn resolve_test_password(
    secret_lookup_id: Option<&str>,
    secret_kind: SecretKind,
    editor_text: &str,
    store: &dyn SshSecretStore,
) -> Option<Zeroizing<String>> {
    if !editor_text.is_empty() {
        return Some(Zeroizing::new(editor_text.to_string()));
    }
    let secret_lookup_id = secret_lookup_id?;
    match store.get(secret_lookup_id, secret_kind) {
        Ok(Some(secret)) => Some(secret),
        Ok(None) => None,
        Err(SshSecretStoreError::NoBackend) => None,
        Err(SshSecretStoreError::Keyring(msg)) => {
            log::warn!("keychain 读取失败,fallback 失败: {msg}");
            None
        }
    }
}

#[cfg(test)]
#[path = "server_view_tests.rs"]
mod tests;
