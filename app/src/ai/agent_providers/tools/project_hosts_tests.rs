//! `project_hosts` 纯逻辑单测:参数校验 / 钳制 / from_args 哨兵构造。

use serde_json::json;
use warp_multi_agent_api as api;

use super::{
    DEFAULT_TIMEOUT_SECONDS, MAX_NODE_IDS, MAX_TIMEOUT_SECONDS, SENTINEL_SERVER_ID, TOOL_NAME,
    parse_batch_args, sentinel_uuid,
};

#[test]
fn parse_batch_args_applies_defaults() {
    let args = parse_batch_args(&json!({
        "node_ids": ["n1", "n2"],
        "command": "uptime"
    }))
    .unwrap();
    assert_eq!(args.node_ids, vec!["n1".to_owned(), "n2".to_owned()]);
    assert_eq!(args.command, "uptime");
    assert!(args.canary);
    assert_eq!(args.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
}

#[test]
fn parse_batch_args_rejects_empty_node_ids() {
    let err = parse_batch_args(&json!({ "node_ids": [], "command": "uptime" })).unwrap_err();
    assert!(err.to_string().contains("node_ids"));
}

#[test]
fn parse_batch_args_rejects_too_many_node_ids() {
    let ids: Vec<String> = (0..=MAX_NODE_IDS).map(|i| format!("n{i}")).collect();
    let err = parse_batch_args(&json!({ "node_ids": ids, "command": "uptime" })).unwrap_err();
    assert!(err.to_string().contains("limit"));
}

#[test]
fn parse_batch_args_rejects_blank_command() {
    let err = parse_batch_args(&json!({ "node_ids": ["n1"], "command": "   " })).unwrap_err();
    assert!(err.to_string().contains("command"));
}

#[test]
fn parse_batch_args_clamps_timeout() {
    let args = parse_batch_args(&json!({
        "node_ids": ["n1"],
        "command": "uptime",
        "timeout_seconds": 100000
    }))
    .unwrap();
    assert_eq!(args.timeout_seconds, MAX_TIMEOUT_SECONDS);

    let args = parse_batch_args(&json!({
        "node_ids": ["n1"],
        "command": "uptime",
        "timeout_seconds": 0
    }))
    .unwrap();
    assert_eq!(args.timeout_seconds, 1);
}

#[test]
fn sentinel_uuid_is_valid_and_stable() {
    assert_eq!(sentinel_uuid().to_string(), SENTINEL_SERVER_ID);
}

#[test]
fn from_args_builds_sentinel_call_mcp_tool() {
    let args = json!({
        "node_ids": ["n1"],
        "command": "uptime",
        "canary": false
    })
    .to_string();
    let tool = (super::RUN_COMMAND_ON_HOSTS.from_args)(&args).unwrap();
    let api::message::tool_call::Tool::CallMcpTool(call) = tool else {
        panic!("expected CallMcpTool, got {tool:?}");
    };
    assert_eq!(call.name, TOOL_NAME);
    assert_eq!(call.server_id, SENTINEL_SERVER_ID);
    let fields = &call.args.unwrap().fields;
    assert!(fields.contains_key("node_ids"));
    assert!(fields.contains_key("command"));
    assert!(fields.contains_key("canary"));
}

#[test]
fn from_args_rejects_invalid_payload() {
    assert!((super::RUN_COMMAND_ON_HOSTS.from_args)(r#"{"command":"x"}"#).is_err());
    assert!((super::RUN_COMMAND_ON_HOSTS.from_args)("not json").is_err());
}

#[test]
fn registry_contains_tool() {
    assert!(crate::ai::agent_providers::tools::lookup(TOOL_NAME).is_some());
}
