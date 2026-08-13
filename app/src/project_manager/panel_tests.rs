//! panel.rs 的单元测试 — 覆盖主机行解析(悬挂引用过滤、顺序保持)与
//! 副标题格式化等纯逻辑。

use warp_ssh_manager::SshServerInfo;

use super::*;

// --- 测试辅助 --------------------------------------------------------------

fn server_info(host: &str, port: u16, username: &str) -> SshServerInfo {
    let mut info = SshServerInfo::new_default(String::new());
    info.host = host.to_string();
    info.port = port;
    info.username = username.to_string();
    info
}

fn lookup_of(entries: &[(&str, &str, SshServerInfo)]) -> HashMap<String, (String, SshServerInfo)> {
    entries
        .iter()
        .map(|(node_id, name, server)| (node_id.to_string(), (name.to_string(), server.clone())))
        .collect()
}

// --- host_subtitle 测试 ----------------------------------------------------

#[test]
fn subtitle_with_username() {
    let server = server_info("example.com", 22, "root");
    assert_eq!(host_subtitle(&server), "root@example.com:22");
}

#[test]
fn subtitle_without_username_omits_at_sign() {
    let server = server_info("example.com", 2222, "");
    assert_eq!(host_subtitle(&server), "example.com:2222");
}

// --- host_row_key 测试 -----------------------------------------------------

#[test]
fn host_row_key_is_project_scoped() {
    // 同一主机被两个项目关联时,hover-state key 不能撞。
    assert_ne!(host_row_key("p1", "n1"), host_row_key("p2", "n1"));
    assert_eq!(host_row_key("p1", "n1"), "p1/n1");
}

// --- resolve_host_rows 测试 ------------------------------------------------

#[test]
fn resolve_empty_node_ids() {
    let lookup = lookup_of(&[]);
    assert!(resolve_host_rows(&[], &lookup).is_empty());
}

#[test]
fn resolve_preserves_association_order() {
    let lookup = lookup_of(&[
        ("n1", "web", server_info("web.example.com", 22, "deploy")),
        ("n2", "db", server_info("db.example.com", 22, "deploy")),
    ]);
    let node_ids = ["n2".to_string(), "n1".to_string()];
    let rows = resolve_host_rows(&node_ids, &lookup);
    let ids: Vec<&str> = rows.iter().map(|r| r.node_id.as_str()).collect();
    assert_eq!(ids, &["n2", "n1"]);
    assert_eq!(rows[0].name, "db");
    assert_eq!(rows[1].name, "web");
}

#[test]
fn resolve_filters_dangling_node_ids() {
    // n-deleted 已从 SSH 树里删掉 → lookup 无此 key → 行被静默过滤。
    let lookup = lookup_of(&[("n1", "web", server_info("web.example.com", 22, "deploy"))]);
    let node_ids = [
        "n-deleted".to_string(),
        "n1".to_string(),
        "n-also-gone".to_string(),
    ];
    let rows = resolve_host_rows(&node_ids, &lookup);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].node_id, "n1");
    assert_eq!(host_subtitle(&rows[0].server), "deploy@web.example.com:22");
}
