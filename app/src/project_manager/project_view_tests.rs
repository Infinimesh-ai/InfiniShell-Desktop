//! `project_view` 纯函数单测(不依赖 warpui 运行时)。

use std::collections::HashSet;

use super::{
    linked_ids_in_tree_order, normalize_optional_field, server_row_subtitle, ProjectServerRow,
};

fn row(node_id: &str) -> ProjectServerRow {
    ProjectServerRow {
        node_id: node_id.to_string(),
        name: format!("server-{node_id}"),
        username: "root".to_string(),
        host: "example.com".to_string(),
        port: 22,
    }
}

#[test]
fn server_row_subtitle_includes_username_when_present() {
    assert_eq!(
        server_row_subtitle("root", "example.com", 22),
        "root@example.com:22"
    );
}

#[test]
fn server_row_subtitle_omits_empty_username() {
    assert_eq!(
        server_row_subtitle("", "example.com", 2222),
        "example.com:2222"
    );
}

#[test]
fn normalize_optional_field_trims_and_maps_empty_to_none() {
    assert_eq!(normalize_optional_field("   "), None);
    assert_eq!(normalize_optional_field(""), None);
    assert_eq!(
        normalize_optional_field("  https://example.com/repo.git  "),
        Some("https://example.com/repo.git".to_string())
    );
}

#[test]
fn linked_ids_follow_tree_order_not_selection_order() {
    let servers = vec![row("a"), row("b"), row("c")];
    // 勾选顺序 c → a,保存顺序仍按候选列表(树)顺序 a → c。
    let linked: HashSet<String> = ["c".to_string(), "a".to_string()].into_iter().collect();
    assert_eq!(
        linked_ids_in_tree_order(&servers, &linked),
        vec!["a".to_string(), "c".to_string()]
    );
}

#[test]
fn linked_ids_skip_unknown_selection() {
    let servers = vec![row("a")];
    let linked: HashSet<String> = ["a".to_string(), "ghost".to_string()].into_iter().collect();
    assert_eq!(
        linked_ids_in_tree_order(&servers, &linked),
        vec!["a".to_string()]
    );
}
