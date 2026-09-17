use super::*;

fn context() -> TrustedLocalToolContext {
    TrustedLocalToolContext {
        task_id: "child".to_owned(),
        generation: 1,
        runtime_generation: Uuid::nil(),
        active_turn_id: "turn-1".to_owned(),
        allow_spawn: false,
        allow_message: true,
        related_task_ids: HashSet::from(["parent".to_owned()]),
    }
}

fn codex_message(tool: &str, arguments: Value) -> Value {
    json!({"id":17,"method":"item/tool/call","params":{
        "threadId":"native-session","turnId":"turn-1","callId":"call-1",
        "tool":tool,"arguments":arguments}})
}

#[test]
fn codex_tools_require_matching_native_session_turn_and_namespace() {
    let message = codex_message(INSPECT_TOOL_NAME, json!({}));
    assert!(codex_tool_request(&message, "another-session", "turn-1").is_err());
    assert!(codex_tool_request(&message, "native-session", "old-turn").is_err());
    let mut namespaced = message.clone();
    namespaced["params"]["namespace"] = json!("external");
    assert!(codex_tool_request(&namespaced, "native-session", "turn-1").is_err());
    assert!(codex_tool_request(&message, "native-session", "turn-1").is_ok());
}

#[test]
fn connection_identity_cannot_be_supplied_or_expanded_by_tool_arguments() {
    let message = codex_message(
        SEND_MESSAGE.name,
        json!({"addresses":["parent"],"subject":"进度","message":"中文\nSecond line"}),
    );
    let request = codex_tool_request(&message, "native-session", "turn-1").unwrap();
    let bound = bind_local_tool_call(request.clone(), &context()).unwrap();
    assert_eq!(bound.sender_task_id, "child");
    assert_eq!(bound.generation, 1);
    let LocalToolOperation::Send(sent) = bound.operation else {
        panic!("工具类型错误");
    };
    assert_eq!(sent.message, "中文\nSecond line");
    let mut forged = request.clone();
    forged.arguments["sender_task_id"] = json!("parent");
    assert!(bind_local_tool_call(forged, &context()).is_err());
    let mut unrelated = request;
    unrelated.arguments["addresses"] = json!(["sibling"]);
    assert!(bind_local_tool_call(unrelated, &context()).is_err());
}

#[test]
fn local_tool_receipt_ids_are_stable_within_generation_and_separate_after_resume() {
    let request = codex_tool_request(
        &codex_message(INSPECT_TOOL_NAME, json!({})),
        "native-session",
        "turn-1",
    )
    .unwrap();
    let first = bind_local_tool_call(request.clone(), &context()).unwrap();
    assert_eq!(
        first.message_id,
        bind_local_tool_call(request.clone(), &context())
            .unwrap()
            .message_id
    );
    let mut next = context();
    next.generation = 2;
    assert_ne!(
        first.message_id,
        bind_local_tool_call(request, &next).unwrap().message_id
    );
}

#[test]
fn spawn_and_message_gates_apply_at_call_time() {
    let request = codex_tool_request(
        &codex_message(
            RUN_AGENTS.name,
            json!({"summary":"审查","base_prompt":"检查","harness":"codex",
                "agent_run_configs":[{"name":"review","prompt":"检查改动"}]}),
        ),
        "native-session",
        "turn-1",
    )
    .unwrap();
    assert!(bind_local_tool_call(request.clone(), &context()).is_err());
    let mut allowed = context();
    allowed.allow_spawn = true;
    let bound = bind_local_tool_call(request, &allowed).unwrap();
    assert!(matches!(bound.operation, LocalToolOperation::Spawn(_)));
    let tools = tool_definitions(false, false);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], INSPECT_TOOL_NAME);
}

#[test]
fn inspect_restricts_records_to_connection_relations_and_bounds_requests() {
    let request = codex_tool_request(
        &codex_message(INSPECT_TOOL_NAME, json!({"task_ids":["parent", "child"]})),
        "native-session",
        "turn-1",
    )
    .unwrap();
    assert!(bind_local_tool_call(request.clone(), &context()).is_ok());
    let mut duplicate = request.clone();
    duplicate.arguments["task_ids"] = json!(["parent", "parent"]);
    assert!(bind_local_tool_call(duplicate, &context()).is_err());
    let mut unrelated = request;
    unrelated.arguments["task_ids"] = json!(["other"]);
    assert!(bind_local_tool_call(unrelated, &context()).is_err());
    let oversized = codex_message(
        INSPECT_TOOL_NAME,
        json!({"data":"x".repeat(MAX_ARGUMENT_BYTES)}),
    );
    assert!(codex_tool_request(&oversized, "native-session", "turn-1").is_err());
}

#[test]
fn historical_result_pages_require_one_related_task_and_a_stable_generation() {
    for arguments in [
        json!({"task_ids":["parent"],"result_generation":1,"result_offset":8190}),
        json!({"result_generation":1}),
    ] {
        let request = codex_tool_request(
            &codex_message(INSPECT_TOOL_NAME, arguments),
            "native-session",
            "turn-1",
        )
        .unwrap();
        assert!(bind_local_tool_call(request, &context()).is_ok());
    }
    for arguments in [
        json!({"task_ids":["parent", "child"],"result_generation":1}),
        json!({"result_offset":8190}),
        json!({"result_generation":0}),
        json!({"result_generation":-1}),
        json!({"result_generation":1,"result_offset":-1}),
        json!({"task_ids":["other"],"result_generation":1}),
        json!({"result_generation":1,"sender_task_id":"parent"}),
    ] {
        let request = codex_tool_request(
            &codex_message(INSPECT_TOOL_NAME, arguments),
            "native-session",
            "turn-1",
        )
        .unwrap();
        assert!(bind_local_tool_call(request, &context()).is_err());
    }
}

#[test]
fn claude_registered_fixture_bootstraps_sdk_mcp_without_a_model_turn() {
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../specs/cli-agent-parity/fixtures/claude-2.1.273-local-tools-registration.ndjson"
    ));
    let mut methods = HashSet::new();
    for line in fixture.lines() {
        let record: Value = serde_json::from_str(line).unwrap();
        let message = &record["message"];
        if record["direction"] != "stdout" || message["request"]["subtype"] != "mcp_message" {
            continue;
        }
        methods.insert(
            message["request"]["message"]["method"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
        let ClaudeMcpRequest::Immediate(response) =
            claude_mcp_request(message, None, false, false).unwrap()
        else {
            panic!("初始化不应派发工具");
        };
        assert_eq!(response["response"]["request_id"], message["request_id"]);
    }
    assert_eq!(
        methods,
        HashSet::from([
            "initialize".to_owned(),
            "notifications/initialized".to_owned(),
            "tools/list".to_owned(),
        ])
    );
}

#[test]
fn claude_tool_call_requires_registered_server_and_active_turn() {
    // 原生调用尚待认证模型验证，此处只验证官方协议形状的拒绝边界。
    let message = json!({"type":"control_request","request_id":"control-1","request":{
    "subtype":"mcp_message","server_name":MCP_SERVER_NAME,"message":{
        "jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":INSPECT_TOOL_NAME,"arguments":{}}
    }}});
    assert!(claude_mcp_request(&message, None, false, false).is_err());
    let ClaudeMcpRequest::Tool(request) =
        claude_mcp_request(&message, Some("turn-1"), false, false).unwrap()
    else {
        panic!("工具调用未转换");
    };
    assert_eq!(request.call_id, "control-1");
    assert!(bind_local_tool_call(request, &context()).is_ok());
    let mut wrong_server = message;
    wrong_server["request"]["server_name"] = json!("external");
    assert!(claude_mcp_request(&wrong_server, Some("turn-1"), false, false).is_err());
}

#[test]
fn failures_remain_native_tool_errors_on_both_protocols() {
    let codex = tool_reply(
        LocalToolReplyTarget::Codex {
            request_id: json!(17),
        },
        Err("未确认接收，请查询原消息".to_owned()),
    );
    assert_eq!(codex["result"]["success"], false);
    let claude = tool_reply(
        LocalToolReplyTarget::Claude {
            request_id: "control-1".to_owned(),
            mcp_id: json!(9),
        },
        Err("任务已过时".to_owned()),
    );
    assert_eq!(
        claude["response"]["response"]["mcp_response"]["result"]["isError"],
        true
    );
    assert_eq!(claude["response"]["response"]["mcp_response"]["id"], 9);
}

#[test]
fn grok_tool_error_remains_a_nested_mcp_error_without_claiming_native_receipt() {
    // Grok 回复目标和转换模块只存在于测试编译，不代表生产适配器已接线。
    let response = tool_reply(
        LocalToolReplyTarget::Grok {
            request_id: json!("reverse-7"),
            mcp_id: json!(9),
        },
        Err("父任务权限上限未验证".to_owned()),
    );
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], "reverse-7");
    assert_eq!(response["result"]["jsonrpc"], "2.0");
    assert_eq!(response["result"]["id"], 9);
    assert_eq!(response["result"]["result"]["isError"], true);
    assert!(response["result"].get("message").is_none());
    assert!(response.get("native_receipt").is_none());
}
