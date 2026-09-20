use super::super::{TrustedLocalToolContext, bind_local_tool_call};
use super::*;
use std::collections::HashSet;

fn bridge() -> GrokMcpBridge {
    GrokMcpBridge::new(
        Uuid::nil(),
        LocalToolPermissions {
            allow_spawn: true,
            allow_message: true,
        },
    )
}

fn request(outer_id: Value, inner_id: Value, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":outer_id,"method":SDK_CALL,"params":{
        "serverId":"infinishell-00000000-0000-0000-0000-000000000000",
        "message":{"jsonrpc":"2.0","id":inner_id,"method":method,"params":params}
    }})
}

fn inspect_request() -> Value {
    request(
        json!(1),
        json!(11),
        "tools/call",
        json!({"name":"inspect_local_tasks","arguments":{}}),
    )
}

fn pending_tool(bridge: &mut GrokMcpBridge) -> NativeLocalToolRequest {
    let GrokMcpRequest::Tool(tool) = bridge
        .receive(&inspect_request(), Some("native-turn-1"))
        .unwrap()
    else {
        panic!("未转换原生工具请求");
    };
    tool
}

#[test]
fn source_derived_sdk_contract_keeps_outer_and_inner_ids_separate() {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../specs/cli-agent-parity/fixtures/grok-1.0.30-local-tools-source-contract.json"
    )))
    .unwrap();
    assert_eq!(fixture["source_derived"], true);
    assert_eq!(fixture["native_cli_executed"], false);
    let mut bridge = bridge();

    let GrokMcpRequest::Immediate(response) = bridge
        .receive(&fixture["initialize_request"], None)
        .unwrap()
    else {
        panic!("SDK 初始化不能执行任务工具");
    };

    assert_eq!(response, fixture["initialize_response"]);
    assert_eq!(bridge.registration(), fixture["session_registration"]);
}

#[test]
fn registration_is_unique_to_runtime_generation() {
    let first = bridge();
    let second = GrokMcpBridge::new(Uuid::from_u128(1), LocalToolPermissions::default());

    assert_ne!(first.server_id(), second.server_id());
    assert_ne!(first.registration(), second.registration());
}

#[test]
fn unregistered_server_cannot_request_tools() {
    let mut bridge = bridge();
    let mut message = inspect_request();
    message["params"]["serverId"] = json!("external-server");

    assert!(bridge.receive(&message, Some("native-turn-1")).is_err());
}

#[test]
fn request_result_cannot_be_mistaken_for_reverse_tool_request() {
    let mut bridge = bridge();
    let mut message = inspect_request();
    message["result"] = json!({});

    assert!(bridge.receive(&message, Some("native-turn-1")).is_err());
}

#[test]
fn inner_notification_without_id_is_rejected_by_half_duplex_contract() {
    let mut bridge = bridge();
    let message = json!({"jsonrpc":"2.0","id":1,"method":SDK_CALL,"params":{
        "serverId":bridge.server_id(),"message":{"jsonrpc":"2.0","method":"notifications/initialized"}
    }});

    assert!(bridge.receive(&message, None).is_err());
}

#[test]
fn unverified_mcp_version_cannot_silently_negotiate_more_capabilities() {
    let mut bridge = bridge();
    let message = request(
        json!(1),
        json!(11),
        "initialize",
        json!({"protocolVersion":"2025-03-26"}),
    );

    assert!(bridge.receive(&message, None).is_err());
}

#[test]
fn tool_request_without_proven_active_turn_returns_mcp_error() {
    let mut bridge = bridge();

    let GrokMcpRequest::Immediate(response) = bridge.receive(&inspect_request(), None).unwrap()
    else {
        panic!("无活跃回合不能执行工具");
    };

    assert_eq!(response["result"]["id"], 11);
    assert_eq!(response["result"]["result"]["isError"], true);
}

#[test]
fn tool_metadata_cannot_override_connection_turn_or_sender() {
    let mut bridge = bridge();
    let mut message = inspect_request();
    message["params"]["message"]["_meta"] = json!({"turn_id":"forged-turn","task_id":"parent"});

    let GrokMcpRequest::Tool(tool) = bridge.receive(&message, Some("native-turn-1")).unwrap()
    else {
        panic!("未转换工具请求");
    };

    assert_eq!(tool.turn_id, "native-turn-1");
}

#[test]
fn inspect_uses_existing_parent_child_scope_validation() {
    let mut bridge = bridge();
    let message = request(
        json!(1),
        json!(11),
        "tools/call",
        json!({"name":"inspect_local_tasks","arguments":{"task_ids":["other-task"]}}),
    );
    let GrokMcpRequest::Tool(tool) = bridge.receive(&message, Some("native-turn-1")).unwrap()
    else {
        panic!("未转换工具请求");
    };
    let context = TrustedLocalToolContext {
        task_id: "child".into(),
        generation: 1,
        runtime_generation: Uuid::nil(),
        active_turn_id: "native-turn-1".into(),
        allow_spawn: false,
        allow_message: true,
        grok_creation_policy_bound: false,
        related_task_ids: HashSet::from(["parent".into()]),
    };

    assert!(bind_local_tool_call(tool, &context).is_err());
}

#[test]
fn run_agents_is_not_advertised_without_fixed_grok_permission_ceiling() {
    let mut bridge = bridge();
    let message = request(json!(1), json!(11), "tools/list", json!({}));

    let GrokMcpRequest::Immediate(response) = bridge.receive(&message, None).unwrap() else {
        panic!("工具列表不能派发任务");
    };

    assert_eq!(
        response["result"]["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        response["result"]["result"]["tools"][0]["name"],
        "inspect_local_tasks"
    );
    assert_eq!(
        response["result"]["result"]["tools"][1]["name"],
        "send_message_to_agent"
    );
}

#[test]
fn unproven_spawn_request_returns_mcp_error_even_when_requested_permission_is_true() {
    let mut bridge = bridge();
    let message = request(
        json!(1),
        json!(11),
        "tools/call",
        json!({"name":"run_agents","arguments":{
            "summary":"测试","base_prompt":"检查","harness":"codex",
            "agent_run_configs":[{"name":"child","prompt":"检查"}]
        }}),
    );

    let GrokMcpRequest::Immediate(response) =
        bridge.receive(&message, Some("native-turn-1")).unwrap()
    else {
        panic!("不能用权限请求位代替固定上限证明");
    };

    assert_eq!(response["result"]["result"]["isError"], true);
}

#[test]
fn message_request_is_rejected_when_connection_did_not_grant_message_permission() {
    let mut bridge = GrokMcpBridge::new(Uuid::nil(), LocalToolPermissions::default());
    let message = request(
        json!(1),
        json!(11),
        "tools/call",
        json!({"name":"send_message_to_agent",
        "arguments":{"addresses":["parent"],"subject":"进度","message":"中文\nSecond line"}}),
    );

    let GrokMcpRequest::Immediate(response) =
        bridge.receive(&message, Some("native-turn-1")).unwrap()
    else {
        panic!("未授权消息工具不能执行");
    };

    assert_eq!(response["result"]["result"]["isError"], true);
}

#[test]
fn authorized_message_keeps_existing_parent_address_and_multilingual_text() {
    let mut bridge = bridge();
    let message = request(
        json!(1),
        json!(11),
        "tools/call",
        json!({"name":"send_message_to_agent",
        "arguments":{"addresses":["parent"],"subject":"进度","message":"中文\nSecond line"}}),
    );
    let GrokMcpRequest::Tool(tool) = bridge.receive(&message, Some("native-turn-1")).unwrap()
    else {
        panic!("未转换获准消息请求");
    };
    let context = TrustedLocalToolContext {
        task_id: "child".into(),
        generation: 1,
        runtime_generation: Uuid::nil(),
        active_turn_id: "native-turn-1".into(),
        allow_spawn: false,
        allow_message: true,
        grok_creation_policy_bound: false,
        related_task_ids: HashSet::from(["parent".into()]),
    };

    let bound = bind_local_tool_call(tool, &context).unwrap();
    let super::super::LocalToolOperation::Send(message) = bound.operation else {
        panic!("未绑定现有消息语义");
    };

    assert_eq!(bound.sender_task_id, "child");
    assert_eq!(message.addresses, vec!["parent".to_owned()]);
    assert_eq!(message.subject, "进度");
    assert_eq!(message.message, "中文\nSecond line");
}

#[test]
fn pending_duplicate_does_not_execute_tool_twice() {
    let mut bridge = bridge();
    let tool = pending_tool(&mut bridge);

    assert_eq!(
        bridge
            .receive(&inspect_request(), Some("native-turn-1"))
            .unwrap(),
        GrokMcpRequest::Duplicate
    );
    let replies = bridge
        .reply(&tool, Ok(json!({"tasks":[]})), Some("native-turn-1"))
        .unwrap();
    assert_eq!(replies.len(), 1);
}

#[test]
fn repeated_inner_request_with_new_outer_id_receives_same_result_without_reexecution() {
    let mut bridge = bridge();
    let tool = pending_tool(&mut bridge);
    let mut alias = inspect_request();
    alias["id"] = json!(2);
    assert_eq!(
        bridge.receive(&alias, Some("native-turn-1")).unwrap(),
        GrokMcpRequest::Duplicate
    );

    let mut replies = bridge
        .reply(&tool, Ok(json!({"tasks":[]})), Some("native-turn-1"))
        .unwrap();
    replies.sort_by_key(|reply| reply["id"].as_i64().unwrap());

    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[1]["id"], 2);
    assert_eq!(replies[0]["result"], replies[1]["result"]);
}

#[test]
fn completed_duplicate_replays_reply_without_emitting_new_tool() {
    let mut bridge = bridge();
    let tool = pending_tool(&mut bridge);
    let replies = bridge
        .reply(&tool, Ok(json!({"tasks":[]})), Some("native-turn-1"))
        .unwrap();

    let GrokMcpRequest::Immediate(replay) = bridge
        .receive(&inspect_request(), Some("native-turn-1"))
        .unwrap()
    else {
        panic!("已完成请求应重用回复");
    };

    assert_eq!(replay, replies[0]);
}

#[test]
fn changed_input_under_same_outer_id_is_rejected() {
    let mut bridge = bridge();
    pending_tool(&mut bridge);
    let mut changed = inspect_request();
    changed["params"]["message"]["params"]["arguments"] = json!({"task_ids":["parent"]});

    assert!(bridge.receive(&changed, Some("native-turn-1")).is_err());
}

#[test]
fn changed_input_under_same_inner_id_is_rejected_even_with_new_outer_id() {
    let mut bridge = bridge();
    pending_tool(&mut bridge);
    let mut changed = inspect_request();
    changed["id"] = json!(2);
    changed["params"]["message"]["params"]["arguments"] = json!({"task_ids":["parent"]});

    assert!(bridge.receive(&changed, Some("native-turn-1")).is_err());
}

#[test]
fn numeric_and_string_inner_ids_are_distinct_tool_calls() {
    let mut bridge = bridge();
    let first = pending_tool(&mut bridge);
    let message = request(
        json!(2),
        json!("11"),
        "tools/call",
        json!({"name":"inspect_local_tasks","arguments":{}}),
    );

    let GrokMcpRequest::Tool(second) = bridge.receive(&message, Some("native-turn-1")).unwrap()
    else {
        panic!("数字与字符串 ID 不能合并");
    };

    assert_ne!(first.call_id, second.call_id);
}

#[test]
fn changed_reply_arguments_cannot_complete_original_tool() {
    let mut bridge = bridge();
    let mut tool = pending_tool(&mut bridge);
    tool.arguments = json!({"task_ids":["parent"]});

    assert!(
        bridge
            .reply(&tool, Ok(json!({})), Some("native-turn-1"))
            .is_err()
    );
}

#[test]
fn reply_from_old_turn_is_rejected() {
    let mut bridge = bridge();
    let tool = pending_tool(&mut bridge);

    assert!(
        bridge
            .reply(&tool, Ok(json!({})), Some("native-turn-2"))
            .is_err()
    );
}

#[test]
fn old_request_cannot_be_rebound_to_a_new_active_turn() {
    let mut bridge = bridge();
    pending_tool(&mut bridge);
    let mut alias = inspect_request();
    alias["id"] = json!(2);

    let GrokMcpRequest::Immediate(response) =
        bridge.receive(&alias, Some("native-turn-2")).unwrap()
    else {
        panic!("旧工具不能绑定新回合");
    };

    assert_eq!(response["result"]["result"]["isError"], true);
}

#[test]
fn cancelled_tool_cannot_accept_late_reply_or_execute_on_redelivery() {
    let mut bridge = bridge();
    let tool = pending_tool(&mut bridge);

    assert_eq!(
        bridge.cancel_turn("native-turn-1"),
        vec![tool.call_id.clone()]
    );
    assert!(
        bridge
            .reply(&tool, Ok(json!({})), Some("native-turn-1"))
            .is_err()
    );
    let GrokMcpRequest::Immediate(response) = bridge
        .receive(&inspect_request(), Some("native-turn-1"))
        .unwrap()
    else {
        panic!("取消后重投不能执行");
    };
    assert_eq!(response["result"]["result"]["isError"], true);
    assert!(bridge.cancel_turn("native-turn-1").is_empty());
}

#[test]
fn unsupported_method_is_a_correlated_inner_mcp_error() {
    let mut bridge = bridge();
    let message = request(json!("outer-7"), json!(8), "resources/list", json!({}));

    let GrokMcpRequest::Immediate(response) = bridge.receive(&message, None).unwrap() else {
        panic!("不支持的方法不能执行工具");
    };

    assert_eq!(response["id"], "outer-7");
    assert_eq!(response["result"]["id"], 8);
    assert_eq!(response["result"]["error"]["code"], -32601);
}

#[test]
fn oversized_request_is_rejected_before_execution() {
    let mut bridge = bridge();
    let message = request(
        json!(1),
        json!(11),
        "tools/call",
        json!({"name":"inspect_local_tasks","arguments":{"data":"x".repeat(MAX_ARGUMENT_BYTES)}}),
    );

    assert!(bridge.receive(&message, Some("native-turn-1")).is_err());
}

#[test]
fn oversized_reply_keeps_pending_identity_and_can_be_replaced_with_bounded_error() {
    let mut bridge = bridge();
    let tool = pending_tool(&mut bridge);

    assert!(
        bridge
            .reply(
                &tool,
                Ok(json!({"data":"x".repeat(MAX_ARGUMENT_BYTES)})),
                Some("native-turn-1")
            )
            .is_err()
    );
    assert_eq!(
        bridge
            .receive(&inspect_request(), Some("native-turn-1"))
            .unwrap(),
        GrokMcpRequest::Duplicate
    );
    let replies = bridge
        .reply(
            &tool,
            Err("结果超过原生通道大小限制".into()),
            Some("native-turn-1"),
        )
        .unwrap();
    assert_eq!(replies[0]["result"]["result"]["isError"], true);
}

#[test]
fn aggregate_reply_limit_does_not_evict_completed_requests() {
    let mut bridge = bridge();
    let large_result = json!({"data":"x".repeat(900_000)});
    for id in 1..=4 {
        let message = request(
            json!(id),
            json!(10 + id),
            "tools/call",
            json!({"name":"inspect_local_tasks","arguments":{}}),
        );
        let GrokMcpRequest::Tool(tool) = bridge.receive(&message, Some("native-turn-1")).unwrap()
        else {
            panic!("未转换原生工具请求");
        };
        bridge
            .reply(&tool, Ok(large_result.clone()), Some("native-turn-1"))
            .unwrap();
    }
    let message = request(
        json!(5),
        json!(15),
        "tools/call",
        json!({"name":"inspect_local_tasks","arguments":{}}),
    );
    let GrokMcpRequest::Tool(tool) = bridge.receive(&message, Some("native-turn-1")).unwrap()
    else {
        panic!("未转换原生工具请求");
    };

    assert!(
        bridge
            .reply(&tool, Ok(large_result), Some("native-turn-1"))
            .is_err()
    );
    let GrokMcpRequest::Immediate(replay) = bridge
        .receive(&inspect_request(), Some("native-turn-1"))
        .unwrap()
    else {
        panic!("缓存达到上限不能删除已完成身份");
    };
    assert_eq!(replay["result"]["result"]["isError"], false);
}

#[test]
fn identity_capacity_does_not_evict_old_requests_to_allow_new_execution() {
    let mut bridge = bridge();
    let tool = pending_tool(&mut bridge);
    for id in 2..=MAX_REQUEST_RECORDS {
        bridge
            .receive(
                &request(json!(id), json!(1000 + id), "ping", json!({})),
                None,
            )
            .unwrap();
    }

    assert!(
        bridge
            .receive(&request(json!(1001), json!(9999), "ping", json!({})), None)
            .is_err()
    );
    assert_eq!(
        bridge
            .receive(&inspect_request(), Some("native-turn-1"))
            .unwrap(),
        GrokMcpRequest::Duplicate
    );
    assert_eq!(
        bridge
            .reply(&tool, Ok(json!({})), Some("native-turn-1"))
            .unwrap()
            .len(),
        1
    );
}

fn modern_params() -> Value {
    json!({"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientCapabilities":{},
        "io.modelcontextprotocol/clientInfo":{"name":"public-conformance-fixture","version":"1.0.0"}}})
}

#[test]
fn modern_source_contract_discovers_without_fabricating_a_native_turn() {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../specs/cli-agent-parity/fixtures/grok-1.0.30-modern-discovery-source-contract.json"
    )))
    .unwrap();
    let mut bridge = bridge();

    let GrokMcpRequest::Immediate(response) =
        bridge.receive(&fixture["discovery_request"], None).unwrap()
    else {
        panic!("discovery 不应执行本地工具");
    };

    assert_eq!(response, fixture["discovery_response"]);
    assert_eq!(
        fixture["sdk7_public_observation"]["native_protocol_version_value_verified"],
        false
    );
    assert_eq!(fixture["native_cli_executed"], false);
}

#[test]
fn modern_tool_list_includes_cache_and_result_discriminators() {
    let mut bridge = bridge();
    let message = request(json!(1), json!(11), "tools/list", modern_params());

    let GrokMcpRequest::Immediate(response) = bridge.receive(&message, None).unwrap() else {
        panic!("工具列表不应执行本地工具");
    };

    assert_eq!(response["result"]["result"]["resultType"], "complete");
    assert_eq!(response["result"]["result"]["ttlMs"], 0);
    assert_eq!(response["result"]["result"]["cacheScope"], "private");
    assert_eq!(
        response["result"]["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        MCP_SERVER_NAME
    );
    assert_eq!(
        response["result"]["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn modern_tool_reply_keeps_complete_format_and_deduplicates() {
    let mut bridge = bridge();
    let mut params = modern_params();
    params["name"] = json!("inspect_local_tasks");
    params["arguments"] = json!({});
    let message = request(json!(1), json!(11), "tools/call", params);
    let GrokMcpRequest::Tool(tool) = bridge.receive(&message, Some("native-turn-1")).unwrap()
    else {
        panic!("未转换有来源的工具调用");
    };

    let replies = bridge
        .reply(&tool, Ok(json!({"tasks":[]})), Some("native-turn-1"))
        .unwrap();
    let replay = bridge.receive(&message, Some("native-turn-1")).unwrap();

    assert_eq!(replies[0]["result"]["result"]["resultType"], "complete");
    assert_eq!(replies[0]["result"]["result"]["isError"], false);
    assert_eq!(replay, GrokMcpRequest::Immediate(replies[0].clone()));
}

#[test]
fn modern_metadata_cannot_replace_missing_native_tool_origin() {
    let mut bridge = bridge();
    let mut params = modern_params();
    params["name"] = json!("inspect_local_tasks");
    params["arguments"] = json!({});

    let GrokMcpRequest::Immediate(response) = bridge
        .receive(&request(json!(1), json!(11), "tools/call", params), None)
        .unwrap()
    else {
        panic!("标准元数据不能代替原生工具来源");
    };

    assert_eq!(response["result"]["result"]["isError"], true);
    assert_eq!(response["result"]["result"]["resultType"], "complete");
}

#[test]
fn modern_unknown_version_returns_standard_negotiation_error() {
    let mut bridge = bridge();
    let mut params = modern_params();
    params["_meta"][PROTOCOL_VERSION_META] = json!("2030-01-01");

    let GrokMcpRequest::Immediate(response) = bridge
        .receive(
            &request(json!(1), json!(11), "server/discover", params),
            None,
        )
        .unwrap()
    else {
        panic!("未知版本不能执行本地工具");
    };

    assert_eq!(response["result"]["error"]["code"], -32022);
    assert_eq!(
        response["result"]["error"]["data"],
        json!({"supported":["2026-07-28"],"requested":"2030-01-01"})
    );
}

#[test]
fn modern_discovery_requires_typed_client_capabilities() {
    let mut bridge = bridge();
    let mut params = modern_params();
    params["_meta"]["io.modelcontextprotocol/clientCapabilities"] = json!(true);

    let GrokMcpRequest::Immediate(response) = bridge
        .receive(
            &request(json!(1), json!(11), "server/discover", params),
            None,
        )
        .unwrap()
    else {
        panic!("错误类型不能完成 discovery");
    };

    assert_eq!(response["result"]["error"]["code"], -32602);
}

#[test]
fn modern_client_info_is_optional_under_the_official_schema() {
    let mut bridge = bridge();
    let params = json!({"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientCapabilities":{}}});

    let GrokMcpRequest::Immediate(response) = bridge
        .receive(
            &request(json!(1), json!(11), "server/discover", params),
            None,
        )
        .unwrap()
    else {
        panic!("规范可选字段缺失不应拒绝 discovery");
    };

    assert_eq!(
        response["result"]["result"]["supportedVersions"],
        json!(["2026-07-28"])
    );
}

#[test]
fn modern_discovery_cannot_supply_metadata_for_a_later_request() {
    let mut bridge = bridge();
    bridge
        .receive(
            &request(json!(1), json!(11), "server/discover", modern_params()),
            None,
        )
        .unwrap();

    let GrokMcpRequest::Immediate(response) = bridge
        .receive(&request(json!(2), json!(12), "tools/list", json!({})), None)
        .unwrap()
    else {
        panic!("每次现代请求都需要真实元数据");
    };

    assert_eq!(response["result"]["error"]["code"], -32602);
}

#[test]
fn discovery_duplicate_reuses_reply_but_conflicting_id_is_rejected() {
    let mut bridge = bridge();
    let message = request(json!(1), json!(11), "server/discover", modern_params());
    let GrokMcpRequest::Immediate(first) = bridge.receive(&message, None).unwrap() else {
        panic!("discovery 应返回立即响应");
    };
    let mut alias = message.clone();
    alias["id"] = json!(2);
    let GrokMcpRequest::Immediate(replay) = bridge.receive(&alias, None).unwrap() else {
        panic!("discovery 重投应复用响应");
    };

    assert_eq!(first["result"], replay["result"]);
    alias["id"] = json!(3);
    alias["params"]["message"]["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"] =
        json!({"roots":{}});
    assert!(bridge.receive(&alias, None).is_err());
}

#[test]
fn modern_extra_tool_body_is_rejected_before_tool_dispatch() {
    let mut bridge = bridge();
    let mut params = modern_params();
    params["name"] = json!("inspect_local_tasks");
    params["arguments"] = json!({});
    params["origin"] = json!("forged");

    assert!(
        bridge
            .receive(
                &request(json!(1), json!(11), "tools/call", params),
                Some("native-turn-1")
            )
            .is_err()
    );
}

#[test]
fn inline_modern_call_cannot_downgrade_when_its_version_is_missing() {
    let mut bridge = bridge();
    let params = json!({"name":"inspect_local_tasks","arguments":{},
        "_meta":{"io.modelcontextprotocol/clientCapabilities":{}}});

    let GrokMcpRequest::Immediate(response) = bridge
        .receive(
            &request(json!(1), json!(11), "tools/call", params),
            Some("native-turn-1"),
        )
        .unwrap()
    else {
        panic!("缺少现代版本不能通过 legacy 路径执行");
    };

    assert_eq!(response["result"]["error"]["code"], -32602);
}

fn lease_ledger(bridge: &GrokMcpBridge, tool: &str) -> GrokToolLeaseLedger {
    lease_ledger_with_arguments(bridge, tool, json!({}))
}

fn lease_ledger_with_arguments(
    bridge: &GrokMcpBridge,
    tool: &str,
    inputs: Value,
) -> GrokToolLeaseLedger {
    let now = Instant::now();
    let mut ledger = GrokToolLeaseLedger::new(
        Uuid::nil(),
        Uuid::nil(),
        bridge.server_id().into(),
        MCP_SERVER_NAME.into(),
        "native-session".into(),
    )
    .unwrap();
    ledger.begin_turn(Uuid::nil(), "native-turn-1").unwrap();
    ledger.observe_native_tool(&json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":"native-session","_meta":{"promptId":"native-turn-1","eventId":"initial"},
        "update":{"sessionUpdate":"tool_call","toolCallId":"native-call","_meta":{"x.ai/tool":{"name":"use_tool"}},
        "rawInput":{"tool_name":format!("{MCP_SERVER_NAME}__{tool}"),"tool_input":inputs}}}}), now).unwrap();
    ledger.observe_native_tool(&json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":"native-session","_meta":{"promptId":"native-turn-1","eventId":"final"},
        "update":{"sessionUpdate":"tool_call_update","toolCallId":"native-call","kind":"other",
        "rawInput":{"tool_name":format!("{MCP_SERVER_NAME}__{tool}"),"tool_input":inputs,"variant":"UseTool"}}}}), now).unwrap();
    ledger.record_permission(&json!({"jsonrpc":"2.0","id":1,"method":"session/request_permission","params":{
        "sessionId":"native-session","toolCall":{"toolCallId":"native-call","kind":"other",
        "rawInput":{"tool_name":format!("{MCP_SERVER_NAME}__{tool}"),"tool_input":inputs,"variant":"UseTool"}}}}), true, now).unwrap();
    ledger
}

#[test]
fn authenticated_native_lease_bridge_replays_cached_reply_without_another_tool_effect() {
    let now = Instant::now();
    let mut bridge = bridge();
    let mut ledger = lease_ledger(&bridge, INSPECT_TOOL_NAME);
    let (first, proof) = bridge
        .receive_with_lease(&inspect_request(), &mut ledger, now)
        .unwrap();
    let GrokMcpRequest::Tool(tool) = first else {
        panic!("已绑定租约未产生工具请求");
    };
    assert_eq!(proof.native_call_id(), "native-call");
    assert_eq!(tool.turn_id, "native-turn-1");
    assert_eq!(
        bridge
            .receive_with_lease(&inspect_request(), &mut ledger, now)
            .unwrap()
            .0,
        GrokMcpRequest::Duplicate
    );
    let responses = bridge
        .reply_with_lease(&tool, Ok(json!({"tasks":[]})), &ledger, &proof)
        .unwrap();
    ledger.record_reply_written(&proof).unwrap();
    let (retry, retry_proof) = bridge
        .receive_with_lease(&inspect_request(), &mut ledger, now)
        .unwrap();
    assert_eq!(retry, GrokMcpRequest::Immediate(responses[0].clone()));
    assert_eq!(retry_proof, proof);
    ledger.retire();
    assert!(
        bridge
            .receive_with_lease(&inspect_request(), &mut ledger, now)
            .is_err()
    );
    assert!(
        bridge
            .reply_with_lease(&tool, Ok(json!({})), &ledger, &proof)
            .is_err()
    );
}

#[test]
fn authenticated_native_lease_does_not_prove_spawn_floor_or_replace_missing_turn() {
    let now = Instant::now();
    let mut without_lease = bridge();
    let GrokMcpRequest::Immediate(rejected) =
        without_lease.receive(&inspect_request(), None).unwrap()
    else {
        panic!("无来源请求未拒绝");
    };
    assert_eq!(rejected["result"]["result"]["isError"], true);
    let mut bridge = bridge();
    let mut ledger = lease_ledger(&bridge, "run_agents");
    let spawn = request(
        json!(2),
        json!(12),
        "tools/call",
        json!({"name":"run_agents","arguments":{}}),
    );
    let (result, _) = bridge.receive_with_lease(&spawn, &mut ledger, now).unwrap();
    let GrokMcpRequest::Immediate(response) = result else {
        panic!("租约不能代替父任务权限上限");
    };
    assert_eq!(response["result"]["result"]["isError"], true);
}

#[test]
fn bridge_protocol_rollback_keeps_lease_but_never_revives_expired_epoch() {
    let now = Instant::now();
    let mut correct = bridge();
    let mut ledger = lease_ledger(&correct, INSPECT_TOOL_NAME);
    let mut foreign = GrokMcpBridge::new(
        Uuid::from_u128(1),
        LocalToolPermissions {
            allow_spawn: false,
            allow_message: false,
        },
    );
    assert!(
        foreign
            .receive_with_lease(&inspect_request(), &mut ledger, now)
            .is_err()
    );
    assert!(!ledger.is_retired());
    assert!(matches!(
        correct
            .receive_with_lease(&inspect_request(), &mut ledger, now)
            .unwrap()
            .0,
        GrokMcpRequest::Tool(_)
    ));
    let mut expired_bridge = bridge();
    let mut expired = lease_ledger(&expired_bridge, INSPECT_TOOL_NAME);
    assert!(
        expired_bridge
            .receive_with_lease(
                &inspect_request(),
                &mut expired,
                now + std::time::Duration::from_secs(31)
            )
            .is_err()
    );
    assert!(expired.is_retired());
    assert!(
        expired_bridge
            .receive_with_lease(&inspect_request(), &mut expired, now)
            .is_err()
    );
}

#[test]
fn production_registration_entry_cannot_dispatch_or_replay_business_calls() {
    let now = Instant::now();
    let mut bridge = bridge();
    let discovery = request(json!(100), json!(100), "server/discover", modern_params());
    let discovered = bridge.receive_registration(&discovery).unwrap();
    assert!(matches!(discovered, GrokMcpRequest::Immediate(_)));
    assert_eq!(bridge.receive_registration(&discovery).unwrap(), discovered);
    let list = request(json!(101), json!(101), "tools/list", modern_params());
    let GrokMcpRequest::Immediate(response) = bridge.receive_registration(&list).unwrap() else {
        panic!("目录不应进入分发器");
    };
    let tools = response["result"]["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == INSPECT_TOOL_NAME));
    assert!(tools.iter().all(|tool| tool["name"] != "run_agents"));
    let mut params = modern_params();
    params["name"] = json!(INSPECT_TOOL_NAME);
    params["arguments"] = json!({});
    let business = request(json!(1), json!(11), "tools/call", params);
    assert!(bridge.receive_registration(&business).is_err());
    let mut ledger = lease_ledger(&bridge, INSPECT_TOOL_NAME);
    assert!(matches!(
        bridge
            .receive_with_lease(&business, &mut ledger, now)
            .unwrap()
            .0,
        GrokMcpRequest::Tool(_)
    ));
    assert!(bridge.receive_registration(&business).is_err());
}

#[test]
fn production_registration_entry_rejects_identity_or_response_injection_before_caching() {
    let clean = request(json!(100), json!(100), "server/discover", modern_params());
    let mut frames = Vec::new();
    let mut top = clean.clone();
    top["turnId"] = json!("claimed-turn");
    frames.push(top);
    let mut session = clean.clone();
    session["params"]["sessionId"] = json!("claimed-session");
    frames.push(session);
    let mut generation = clean.clone();
    generation["params"]["message"]["generation"] = json!("claimed-generation");
    frames.push(generation);
    let mut result = clean.clone();
    result["result"] = json!({});
    frames.push(result);
    let mut error = clean.clone();
    error["params"]["message"]["error"] = json!({"code":1});
    frames.push(error);
    let mut foreign = clean.clone();
    foreign["params"]["serverId"] = json!("another-server");
    frames.push(foreign);
    let mut unknown = clean.clone();
    unknown["params"]["message"]["method"] = json!("unknown/extension");
    frames.push(unknown);
    let mut bridge = bridge();
    for frame in frames {
        assert!(bridge.receive_registration(&frame).is_err());
    }
    assert!(matches!(
        bridge.receive_registration(&clean).unwrap(),
        GrokMcpRequest::Immediate(_)
    ));
}

#[test]
fn leased_grok_message_uses_common_dispatcher_without_expanding_sender_relations() {
    let now = Instant::now();
    let mut bridge = bridge();
    let inputs = json!({"addresses":["parent"],"subject":"进度","message":"中文\nSecond line"});
    let mut ledger = lease_ledger_with_arguments(&bridge, SEND_MESSAGE.name, inputs.clone());
    let frame = request(
        json!(2),
        json!(12),
        "tools/call",
        json!({"name":SEND_MESSAGE.name,"arguments":inputs}),
    );
    let (received, proof) = bridge.receive_with_lease(&frame, &mut ledger, now).unwrap();
    let GrokMcpRequest::Tool(tool) = received else {
        panic!("已绑定消息未进入工具分发");
    };
    assert_eq!(proof.native_call_id(), "native-call");
    let context = TrustedLocalToolContext {
        task_id: "child".into(),
        generation: 1,
        runtime_generation: Uuid::nil(),
        active_turn_id: "native-turn-1".into(),
        allow_spawn: true,
        allow_message: true,
        grok_creation_policy_bound: false,
        related_task_ids: HashSet::from(["parent".into()]),
    };
    let bound = bind_local_tool_call(tool.clone(), &context).unwrap();
    assert_eq!(bound.sender_task_id, "child");
    let super::super::LocalToolOperation::Send(message) = bound.operation else {
        panic!("消息工具类型错误");
    };
    assert_eq!(message.addresses, vec!["parent".to_string()]);
    assert_eq!(message.message, "中文\nSecond line");
    let mut forged = tool.clone();
    forged.arguments["sender_task_id"] = json!("parent");
    assert!(bind_local_tool_call(forged, &context).is_err());
    let mut unrelated = tool;
    unrelated.arguments["addresses"] = json!(["stranger"]);
    assert!(bind_local_tool_call(unrelated, &context).is_err());
}

#[test]
fn production_lease_reply_rejects_changed_targets_and_keeps_modern_cached_result() {
    let now = Instant::now();
    let mut bridge = bridge();
    let mut ledger = lease_ledger(&bridge, INSPECT_TOOL_NAME);
    let mut params = modern_params();
    params["name"] = json!(INSPECT_TOOL_NAME);
    params["arguments"] = json!({});
    let frame = request(json!(1), json!(11), "tools/call", params);
    let (outcome, proof) = bridge.receive_with_lease(&frame, &mut ledger, now).unwrap();
    let GrokMcpRequest::Tool(tool) = outcome else {
        panic!("未建立回复事务");
    };
    let mut wrong_outer = tool.clone();
    wrong_outer.reply_target = LocalToolReplyTarget::Grok {
        request_id: json!(9),
        mcp_id: json!(11),
    };
    assert!(
        bridge
            .reply_with_lease(&wrong_outer, Ok(json!({})), &ledger, &proof)
            .is_err()
    );
    let mut wrong_inner = tool.clone();
    wrong_inner.reply_target = LocalToolReplyTarget::Grok {
        request_id: json!(1),
        mcp_id: json!(12),
    };
    assert!(
        bridge
            .reply_with_lease(&wrong_inner, Ok(json!({})), &ledger, &proof)
            .is_err()
    );
    let mut changed = tool.clone();
    changed.arguments = json!({"task_ids":["stranger"]});
    assert!(
        bridge
            .reply_with_lease(&changed, Ok(json!({})), &ledger, &proof)
            .is_err()
    );
    let replies = bridge
        .reply_with_lease(&tool, Err("执行失败".into()), &ledger, &proof)
        .unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["result"]["id"], 11);
    assert_eq!(replies[0]["result"]["result"]["isError"], true);
    assert_eq!(replies[0]["result"]["result"]["resultType"], "complete");
    assert!(replies[0].get("native_receipt").is_none());
    ledger.record_reply_written(&proof).unwrap();
    let mut retry = frame;
    retry["id"] = json!(3);
    let (cached, cached_proof) = bridge.receive_with_lease(&retry, &mut ledger, now).unwrap();
    assert_eq!(cached_proof, proof);
    let GrokMcpRequest::Immediate(response) = cached else {
        panic!("已回复工具被再次分发");
    };
    assert_eq!(response["id"], 3);
    assert_eq!(response["result"], replies[0]["result"]);
}

#[test]
fn catalog_registration_requires_actual_matching_discovery_and_list_writes() {
    let mut bridge = bridge();
    let discover = request(json!(201), json!(301), "server/discover", modern_params());
    let list = request(json!(202), json!(302), "tools/list", modern_params());
    let GrokMcpRequest::Immediate(discovered) = bridge.receive_registration(&discover).unwrap()
    else {
        panic!("注册不能生成业务工具");
    };
    let GrokMcpRequest::Immediate(listed) = bridge.receive_registration(&list).unwrap() else {
        panic!("目录不能生成业务工具");
    };
    assert!(bridge.served_catalog_names().is_empty());
    let mut tampered = discovered.clone();
    tampered["result"]["result"]["ttlMs"] = json!(1);
    bridge.record_registration_written(&tampered);
    bridge.record_registration_written(&listed);
    assert!(bridge.served_catalog_names().is_empty());
    bridge.record_registration_written(&discovered);
    assert!(bridge.served_catalog_names().is_empty());
    bridge.record_registration_written(&listed);
    assert_eq!(
        bridge.served_catalog_names(),
        vec![
            "infinishell-local-tasks__inspect_local_tasks",
            "infinishell-local-tasks__send_message_to_agent",
        ]
    );
    let mut other = GrokMcpBridge::new(Uuid::new_v4(), LocalToolPermissions::default());
    assert!(other.receive_registration(&discover).is_err());
    other.record_registration_written(&discovered);
    other.record_registration_written(&listed);
    assert!(other.served_catalog_names().is_empty());
}

#[test]
fn failed_discovery_and_unregistered_reply_cannot_confirm_catalog() {
    let mut bridge = bridge();
    let mut invalid = request(json!(201), json!(301), "server/discover", modern_params());
    invalid["params"]["message"]["params"]["_meta"][PROTOCOL_VERSION_META] = json!("unknown");
    let GrokMcpRequest::Immediate(reply) = bridge.receive_registration(&invalid).unwrap() else {
        panic!("失败发现仍为协议响应");
    };
    bridge.record_registration_written(&reply);
    assert!(bridge.served_catalog_names().is_empty());
    let forged = json!({"jsonrpc":"2.0","id":999,"result":{"jsonrpc":"2.0","id":999,
        "result":{"tools":[{"name":"run_agents"}]}}});
    bridge.record_registration_written(&forged);
    assert!(bridge.served_catalog_names().is_empty());
}

#[test]
fn local_catalog_qualification_preserves_hyphens_and_rejects_ambiguous_names() {
    assert_eq!(
        qualify_local_catalog_name("send_message_to_agent").as_deref(),
        Some("infinishell-local-tasks__send_message_to_agent")
    );
    assert_eq!(
        qualify_local_catalog_name("2fa-enable").as_deref(),
        Some("infinishell-local-tasks__2fa-enable")
    );
    for name in [
        "",
        "_inspect",
        "inspect__tasks",
        "inspect___tasks",
        "a/b",
        "含中文",
        "with space",
    ] {
        assert!(qualify_local_catalog_name(name).is_none());
    }
    let allowed = "a".repeat(256 - MCP_SERVER_NAME.len() - 2);
    assert!(qualify_local_catalog_name(&allowed).is_some());
    assert!(qualify_local_catalog_name(&(allowed + "a")).is_none());
}
