//! panel.rs 的单元测试 — 覆盖树构建、父级解析和显示排序等纯逻辑。
//!
//! 作者：logic

use chrono::NaiveDateTime;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_ssh_manager::{NodeKind, SshNode};
use warpui::platform::WindowStyle;
use warpui::units::IntoPixels;
use warpui::{App, EntityIdSet, Presenter, WindowInvalidation};

use super::*;
use crate::test_util::settings::initialize_settings_for_tests;

// --- 测试辅助 --------------------------------------------------------------

fn ts() -> NaiveDateTime {
    chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc()
}

fn folder(id: &str, parent_id: Option<&str>, name: &str, sort_order: i32) -> SshNode {
    SshNode {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        kind: NodeKind::Folder,
        name: name.to_string(),
        sort_order,
        created_at: ts(),
        updated_at: ts(),
        is_collapsed: false,
    }
}

fn server(id: &str, parent_id: Option<&str>, name: &str, sort_order: i32) -> SshNode {
    SshNode {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        kind: NodeKind::Server,
        name: name.to_string(),
        sort_order,
        created_at: ts(),
        updated_at: ts(),
        is_collapsed: false,
    }
}

#[test]
fn saved_route_context_target_round_trips() {
    let target = route_context_target("route-1");
    assert_eq!(target, "saved-route:route-1");
    assert_eq!(route_id_from_context_target(&target), Some("route-1"));
    assert_eq!(route_id_from_context_target("server-1"), None);
}

#[test]
fn saved_route_hops_display_preserves_aliases_and_ports() {
    let hops = vec![
        SshRouteHop {
            node_id: None,
            target_alias: "bastion".to_string(),
            port: None,
        },
        SshRouteHop {
            node_id: None,
            target_alias: "deploy@internal".to_string(),
            port: Some(2222),
        },
    ];

    assert_eq!(route_hops_display(&hops), "bastion > deploy@internal:2222");
}

fn panel_with_nodes(
    ctx: &mut ViewContext<SshManagerPanel>,
    nodes: Vec<SshNode>,
) -> SshManagerPanel {
    let depths = compute_depths(&nodes);
    let candidates = ctx.add_model(|_| CandidatesViewModel::new());
    let mut row_states = HashMap::new();
    let mut row_drag_states = HashMap::new();
    for node in &nodes {
        row_states.insert(node.id.clone(), MouseStateHandle::default());
        row_drag_states.insert(node.id.clone(), DraggableState::default());
    }

    SshManagerPanel {
        nodes,
        routes: Vec::new(),
        depths,
        selected_id: None,
        add_folder_btn: MouseStateHandle::default(),
        add_server_btn: MouseStateHandle::default(),
        toggle_all_btn: MouseStateHandle::default(),
        row_states,
        row_drag_states,
        route_row_states: HashMap::new(),
        context_menu_position: None,
        context_menu_target: None,
        context_menu_item_states: (0..MAX_CONTEXT_MENU_ITEMS)
            .map(|_| MouseStateHandle::default())
            .collect(),
        rename_state: None,
        candidates,
        candidate_row_states: HashMap::new(),
        candidate_add_states: HashMap::new(),
        candidates_refresh_btn: MouseStateHandle::default(),
        candidates_toggle_btn: MouseStateHandle::default(),
        content_scroll_state: ClippedScrollStateHandle::default(),
    }
}

fn render_panel_scene(app: &mut App, presenter: &mut Presenter, window_id: warpui::WindowId) {
    // `WindowInvalidation::updated` 是 `EntityIdSet`(FxHasher),不是标准库默认 hasher 的 HashSet。
    let mut updated = EntityIdSet::default();
    updated.insert(app.root_view_id(window_id).unwrap());
    let invalidation = WindowInvalidation {
        updated,
        ..Default::default()
    };

    app.update(move |ctx| {
        presenter.invalidate(invalidation, ctx);
        presenter.build_scene(vec2f(240., 120.), 1., None, ctx);
    });
}

// --- resolve_parent_for_new_node 测试 ---------------------------------------

#[test]
fn parent_no_selection_returns_none() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(resolve_parent_for_new_node(None, &nodes), None);
}

#[test]
fn parent_folder_selected_returns_folder_id() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(
        resolve_parent_for_new_node(Some("f1"), &nodes),
        Some("f1".to_string())
    );
}

#[test]
fn parent_server_at_root_selected_returns_none() {
    let nodes = vec![server("s1", None, "srv", 0)];
    assert_eq!(resolve_parent_for_new_node(Some("s1"), &nodes), None);
}

#[test]
fn parent_server_under_folder_selected_returns_folder_id() {
    let nodes = vec![
        folder("f1", None, "Prod", 0),
        server("s1", Some("f1"), "web", 0),
    ];
    assert_eq!(
        resolve_parent_for_new_node(Some("s1"), &nodes),
        Some("f1".to_string())
    );
}

#[test]
fn parent_invalid_selected_id_returns_none() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(
        resolve_parent_for_new_node(Some("nonexistent"), &nodes),
        None
    );
}

#[test]
fn parent_empty_nodes_with_selection_returns_none() {
    assert_eq!(resolve_parent_for_new_node(Some("any"), &[]), None);
}

#[test]
fn parent_deeply_nested_folder_selected_returns_immediate_parent() {
    // f1(root) → f2(child) → s1(grandchild server)
    let nodes = vec![
        folder("f1", None, "L0", 0),
        folder("f2", Some("f1"), "L1", 0),
        server("s1", Some("f2"), "srv", 0),
    ];
    // 选中 f2 → 新节点创建在 f2 下
    assert_eq!(
        resolve_parent_for_new_node(Some("f2"), &nodes),
        Some("f2".to_string())
    );
    // 选中 s1 → 新节点创建在 s1 的父级(f2)下（兄弟语义）
    assert_eq!(
        resolve_parent_for_new_node(Some("s1"), &nodes),
        Some("f2".to_string())
    );
}

// --- compute_depths 测试 ---------------------------------------------------

#[test]
fn depths_empty_nodes() {
    let depths = compute_depths(&[]);
    assert!(depths.is_empty());
}

#[test]
fn depths_single_root() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
}

#[test]
fn depths_nested_tree() {
    let nodes = vec![
        folder("f1", None, "Root", 0),
        folder("f2", Some("f1"), "Child", 0),
        server("s1", Some("f2"), "Grandchild", 0),
    ];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
    assert_eq!(depths["f2"], 1);
    assert_eq!(depths["s1"], 2);
}

#[test]
fn depths_multiple_roots() {
    let nodes = vec![
        folder("f1", None, "Root1", 0),
        folder("f2", None, "Root2", 1),
        server("s1", Some("f1"), "srv", 0),
        server("s2", Some("f2"), "srv", 0),
    ];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
    assert_eq!(depths["f2"], 0);
    assert_eq!(depths["s1"], 1);
    assert_eq!(depths["s2"], 1);
}

// --- sort_for_display 测试 -------------------------------------------------

#[test]
fn sort_empty() {
    let depths = HashMap::new();
    let sorted = sort_for_display(vec![], &depths);
    assert!(sorted.is_empty());
}

#[test]
fn sort_single_root() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    assert_eq!(sorted.len(), 1);
    assert_eq!(sorted[0].id, "f1");
}

#[test]
fn sort_respects_parent_child_order() {
    let nodes = vec![
        server("s1", Some("f1"), "web", 0),
        folder("f1", None, "Prod", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    // f1 在前，s1 在后
    assert_eq!(sorted[0].id, "f1");
    assert_eq!(sorted[1].id, "s1");
}

#[test]
fn sort_preserves_existing_folder_children_in_tree_order() {
    let nodes = vec![
        server("s2", Some("f2"), "db", 1),
        folder("f2", None, "Stage", 1),
        server("s1", Some("f1"), "web", 0),
        folder("f1", None, "Prod", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();

    assert_eq!(ids, &["f1", "s1", "f2", "s2"]);
    assert_eq!(depths["s1"], 1);
    assert_eq!(depths["s2"], 1);
}

#[test]
fn sort_multiple_roots_by_sort_order() {
    let nodes = vec![folder("f2", None, "B", 1), folder("f1", None, "A", 0)];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    assert_eq!(sorted[0].id, "f1");
    assert_eq!(sorted[1].id, "f2");
}

#[test]
fn sort_deeply_nested() {
    let nodes = vec![
        folder("f1", None, "Root", 0),
        server("s2", Some("f2"), "deep", 1),
        folder("f2", Some("f1"), "Child", 0),
        server("s1", Some("f1"), "shallow", 1),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, &["f1", "f2", "s2", "s1"]);
}

#[test]
fn sort_multiple_roots_with_children() {
    let nodes = vec![
        folder("f2", None, "Stage", 1),
        folder("f1", None, "Prod", 0),
        server("s1", Some("f1"), "web", 0),
        server("s2", Some("f2"), "app", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();
    // f1(Prod) 及其子节点在前，f2(Stage) 及其子节点在后
    assert_eq!(ids, &["f1", "s1", "f2", "s2"]);
}

#[test]
fn sort_keeps_orphaned_existing_nodes_visible_as_roots() {
    let nodes = vec![
        server("s1", Some("missing-folder"), "legacy", 0),
        folder("f1", None, "New folder", 1),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();

    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"s1"));
    assert!(ids.contains(&"f1"));
    assert_eq!(depths["s1"], 0);
    assert_eq!(depths["f1"], 0);
}

#[test]
fn panel_content_can_scroll_when_ssh_list_is_taller_than_panel() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SshTreeChangedNotifier::new());

        let nodes = (0..60)
            .map(|i| server(&format!("s{i}"), None, &format!("server-{i}"), i))
            .collect::<Vec<_>>();
        let (window_id, panel) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            panel_with_nodes(ctx, nodes)
        });
        let mut presenter = Presenter::new(window_id);

        render_panel_scene(&mut app, &mut presenter, window_id);
        let scroll_state = panel.read(&app, |panel, _| panel.content_scroll_state.clone());

        scroll_state.scroll_by(10_000_f32.into_pixels());
        render_panel_scene(&mut app, &mut presenter, window_id);

        let scroll_start = scroll_state.scroll_start().as_f32();
        assert!(scroll_start > 0.0);
        assert!(scroll_start < 10_000.0);
    });
}
