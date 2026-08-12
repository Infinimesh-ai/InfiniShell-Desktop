//! `session_router` 纯逻辑单测:输出截断 / 金丝雀判定 / 结果聚合。

use serde_json::json;

use super::{
    aggregate_results, host_result_to_json, is_canary_failure, truncate_output, BatchHostResult,
    BatchHostStatus, OUTPUT_MAX_CHARS,
};

fn ok_result(node_id: &str) -> BatchHostResult {
    BatchHostResult {
        node_id: node_id.to_owned(),
        host: format!("{node_id}.example.com:22"),
        status: BatchHostStatus::Ok,
        exit_code: Some(0),
        output: "done".to_owned(),
        duration_ms: 42,
    }
}

#[test]
fn truncate_output_passes_short_output_through() {
    assert_eq!(truncate_output("hello"), "hello");
}

#[test]
fn truncate_output_truncates_by_chars_and_appends_marker() {
    // 用多字节字符验证按字符而非字节截断。
    let long: String = "汉".repeat(OUTPUT_MAX_CHARS + 5);
    let truncated = truncate_output(&long);
    assert_eq!(
        truncated.chars().count(),
        OUTPUT_MAX_CHARS + "…(输出超限已截断)".chars().count()
    );
    assert!(truncated.ends_with("…(输出超限已截断)"));
}

#[test]
fn is_canary_failure_on_nonzero_exit() {
    let mut result = ok_result("n1");
    assert!(!is_canary_failure(&result));
    result.exit_code = Some(1);
    result.status = BatchHostStatus::Error;
    assert!(is_canary_failure(&result));
}

#[test]
fn is_canary_failure_on_missing_exit_code() {
    let mut result = ok_result("n1");
    result.exit_code = None;
    // 状态 ok 但没有退出码(理论边界)也算失败,保持保守。
    assert!(is_canary_failure(&result));
}

#[test]
fn is_canary_failure_on_non_ok_status() {
    let mut result = ok_result("n1");
    result.status = BatchHostStatus::Timeout;
    assert!(is_canary_failure(&result));
}

#[test]
fn host_result_to_json_shape() {
    let value = host_result_to_json(&ok_result("n1"));
    assert_eq!(
        value,
        json!({
            "node_id": "n1",
            "host": "n1.example.com:22",
            "status": "ok",
            "exit_code": 0,
            "output": "done",
            "duration_ms": 42,
        })
    );
}

#[test]
fn aggregate_results_all_ok() {
    let value = aggregate_results(&[ok_result("n1"), ok_result("n2")], false);
    assert_eq!(value["status"], "ok");
    assert_eq!(value["canary_aborted"], false);
    assert_eq!(value["results"].as_array().unwrap().len(), 2);
}

#[test]
fn aggregate_results_error_when_any_failed() {
    let mut failed = ok_result("n2");
    failed.status = BatchHostStatus::Error;
    failed.exit_code = Some(1);
    let value = aggregate_results(&[ok_result("n1"), failed], true);
    assert_eq!(value["status"], "error");
    assert_eq!(value["canary_aborted"], true);
    assert_eq!(value["results"][1]["status"], "error");
}

#[test]
fn status_strings_are_stable() {
    assert_eq!(BatchHostStatus::Ok.as_str(), "ok");
    assert_eq!(BatchHostStatus::Error.as_str(), "error");
    assert_eq!(BatchHostStatus::Timeout.as_str(), "timeout");
    assert_eq!(BatchHostStatus::Busy.as_str(), "busy");
    assert_eq!(BatchHostStatus::SessionNotReady.as_str(), "session_not_ready");
    assert_eq!(BatchHostStatus::CanaryAborted.as_str(), "canary_aborted");
}
