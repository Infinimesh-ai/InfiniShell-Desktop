//! `batch_command_view` 纯逻辑单测:主机行文案 / 行数钳制 / args→展示映射 /
//! 聚合结果摘要。

use std::collections::HashMap;

use crate::ai::agent::CallMCPToolResult;

use super::{
    batch_outcome, build_host_rows, capped_host_rows, format_endpoint, host_row_text,
    parse_batch_summary, BatchOutcome, HostEndpoint, HostRow, MAX_HOST_ROWS,
};

fn endpoint(username: &str, host: &str, port: u16) -> HostEndpoint {
    HostEndpoint {
        username: username.to_owned(),
        host: host.to_owned(),
        port,
    }
}

#[test]
fn format_endpoint_includes_user_host_port() {
    assert_eq!(
        format_endpoint(&endpoint("root", "10.0.0.1", 2222)),
        "root@10.0.0.1:2222"
    );
}

#[test]
fn host_row_text_prefers_name_and_endpoint() {
    let row = HostRow {
        node_id: "n1".to_owned(),
        name: Some("web-1".to_owned()),
        endpoint: Some(endpoint("deploy", "web1.internal", 22)),
    };
    assert_eq!(
        host_row_text(&row, "(未知主机)"),
        "web-1 — deploy@web1.internal:22"
    );
}

#[test]
fn host_row_text_dangling_node_falls_back_to_node_id() {
    let row = HostRow {
        node_id: "gone-node".to_owned(),
        name: None,
        endpoint: None,
    };
    assert_eq!(host_row_text(&row, "(未知主机)"), "gone-node (未知主机)");
}

#[test]
fn host_row_text_named_node_without_server_uses_name_with_unknown_label() {
    let row = HostRow {
        node_id: "n2".to_owned(),
        name: Some("db-1".to_owned()),
        endpoint: None,
    };
    assert_eq!(host_row_text(&row, "(unknown host)"), "db-1 (unknown host)");
}

#[test]
fn build_host_rows_maps_args_order_and_dangling_ids() {
    let node_ids = vec!["n1".to_owned(), "missing".to_owned()];
    let mut name_by_id = HashMap::new();
    name_by_id.insert("n1".to_owned(), "web-1".to_owned());
    let mut endpoint_by_id = HashMap::new();
    endpoint_by_id.insert("n1".to_owned(), endpoint("root", "h1", 22));

    let rows = build_host_rows(&node_ids, &name_by_id, &endpoint_by_id);
    assert_eq!(
        rows,
        vec![
            HostRow {
                node_id: "n1".to_owned(),
                name: Some("web-1".to_owned()),
                endpoint: Some(endpoint("root", "h1", 22)),
            },
            HostRow {
                node_id: "missing".to_owned(),
                name: None,
                endpoint: None,
            },
        ]
    );
}

#[test]
fn capped_host_rows_passes_through_under_limit() {
    let rows = vec![
        HostRow {
            node_id: "n1".to_owned(),
            name: None,
            endpoint: None,
        };
        3
    ];
    let (visible, hidden) = capped_host_rows(&rows);
    assert_eq!(visible.len(), 3);
    assert_eq!(hidden, 0);
}

#[test]
fn capped_host_rows_truncates_over_limit() {
    let rows = vec![
        HostRow {
            node_id: "n".to_owned(),
            name: None,
            endpoint: None,
        };
        MAX_HOST_ROWS + 5
    ];
    let (visible, hidden) = capped_host_rows(&rows);
    assert_eq!(visible.len(), MAX_HOST_ROWS);
    assert_eq!(hidden, 5);
}

#[test]
fn parse_batch_summary_reads_ok_status_and_counts() {
    let text = r#"{"status":"ok","canary_aborted":false,"results":[{"status":"ok"},{"status":"ok"}]}"#;
    assert_eq!(
        parse_batch_summary(text),
        BatchOutcome::Success {
            counts: Some((2, 2))
        }
    );
}

#[test]
fn parse_batch_summary_reads_error_status_with_partial_counts() {
    let text = r#"{"status":"error","results":[{"status":"ok"},{"status":"error"},{"status":"skipped_canary"}]}"#;
    assert_eq!(
        parse_batch_summary(text),
        BatchOutcome::Failed {
            counts: Some((1, 3))
        }
    );
}

#[test]
fn parse_batch_summary_reads_cancelled_status() {
    assert_eq!(
        parse_batch_summary(r#"{"status":"cancelled"}"#),
        BatchOutcome::Cancelled
    );
}

#[test]
fn parse_batch_summary_falls_back_on_unparsable_payload() {
    assert_eq!(
        parse_batch_summary("not json"),
        BatchOutcome::Success { counts: None }
    );
}

#[test]
fn batch_outcome_maps_error_and_cancelled_variants() {
    assert_eq!(
        batch_outcome(&CallMCPToolResult::Error("boom".to_owned())),
        BatchOutcome::Error("boom".to_owned())
    );
    assert_eq!(
        batch_outcome(&CallMCPToolResult::Cancelled),
        BatchOutcome::Cancelled
    );
}

#[test]
fn batch_outcome_parses_success_text_content() {
    let result = CallMCPToolResult::Success {
        result: rmcp::model::CallToolResult::success(vec![rmcp::model::Content::text(
            r#"{"status":"ok","results":[{"status":"ok"}]}"#,
        )]),
    };
    assert_eq!(
        batch_outcome(&result),
        BatchOutcome::Success {
            counts: Some((1, 1))
        }
    );
}
