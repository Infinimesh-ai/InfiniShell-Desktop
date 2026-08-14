use super::*;

#[test]
fn provider_state完整回放未知responses条目() {
    let raw_item = json!({
        "type": "program",
        "id": "prog_1",
        "caller_id": "root_1",
        "future_field": [1, 2, 3]
    });
    let state = ProviderResponseState {
        response_id: Some("resp_1".to_owned()),
        conversation_id: Some("conv_1".to_owned()),
        request_context_fingerprint: Some("fingerprint".to_owned()),
        last_sequence_number: Some(42),
        state_mode: Some(crate::settings::ResponsesStateModeSetting::Conversation),
        encrypted_reasoning_items: vec!["signature".to_owned()],
        response_items: vec![raw_item.clone()],
        unknown_events: vec![json!({"type": "response.future.delta", "value": true})],
    };

    let encoded = encode_provider_response_state(&state).expect("状态应能编码");
    let decoded = decode_provider_response_state(&encoded).expect("状态应能解码");

    assert_eq!(decoded.response_id.as_deref(), Some("resp_1"));
    assert_eq!(decoded.last_sequence_number, Some(42));
    assert_eq!(decoded.response_items, vec![raw_item]);
    assert_eq!(decoded.unknown_events[0]["value"], true);
}

#[test]
fn provider_state向后兼容旧sidecar() {
    let encoded = format!(
        "{PROVIDER_RESPONSE_STATE_PREFIX}{}",
        json!({
            "response_id": "resp_old",
            "encrypted_reasoning_items": ["legacy-signature"]
        })
    );

    let decoded = decode_provider_response_state(&encoded).expect("旧 sidecar 应能解码");

    assert_eq!(decoded.response_id.as_deref(), Some("resp_old"));
    assert_eq!(decoded.encrypted_reasoning_items, ["legacy-signature"]);
    assert!(decoded.response_items.is_empty());
    assert!(decoded.unknown_events.is_empty());
}

#[test]
fn ptc只允许无副作用且可验证的工具() {
    for name in [
        "read_files",
        "grep",
        "file_glob",
        "read_shell_command_output",
        "read_skill",
        "read_documents",
        tools::webfetch::TOOL_NAME,
        tools::websearch::TOOL_NAME,
    ] {
        assert!(ptc_allows_tool(name), "{name} 应允许由 PTC 调用");
    }
    for name in [
        "run_shell_command",
        "apply_file_diffs",
        "write_to_long_running_shell_command",
        "create_documents",
        "edit_documents",
        "update_machine_memory",
        "run_command_on_hosts",
        "todowrite",
        "ask_user_question",
    ] {
        assert!(!ptc_allows_tool(name), "{name} 不应允许由 PTC 调用");
    }
}

#[test]
fn gpt56高级能力识别别名变体和快照() {
    for model in [
        "gpt-5.6",
        "gpt-5.6-sol",
        "gpt-5.6-terra-2026-08-01",
        "openai/gpt-5.6-luna",
    ] {
        assert!(is_gpt56_model(model), "{model} 应识别为 GPT-5.6 系列");
    }
    for model in ["gpt-5.5", "gpt-5.4", "gpt-4o"] {
        assert!(!is_gpt56_model(model), "{model} 不应识别为 GPT-5.6 系列");
    }
}

#[test]
fn responses出站工具使用规范化后的strict_schema() {
    let params = RequestParams::new_for_test(Vec::new(), Vec::new());
    let tools = build_tools_array(&params, false, true);
    let shell = tools
        .iter()
        .find(|tool| tool.name == "run_shell_command".into())
        .expect("应包含 run_shell_command");

    assert_eq!(shell.strict, Some(true));
    let schema = shell.schema.as_ref().expect("strict tool 应包含 schema");
    assert_eq!(
        schema["required"],
        json!([
            "command",
            "is_read_only",
            "is_risky",
            "uses_pager",
            "wait_until_complete"
        ])
    );
    assert_eq!(
        schema["properties"]["is_read_only"]["type"],
        json!(["boolean", "null"])
    );
}

#[test]
fn responses工具调用在本地解析前删除可选null() {
    let mut call = ToolCall {
        call_id: "call_1".to_owned(),
        fn_name: "run_shell_command".to_owned(),
        fn_arguments: json!({
            "command": "pwd",
            "is_read_only": null,
            "uses_pager": null,
            "is_risky": null,
            "wait_until_complete": null
        }),
        thought_signatures: None,
    };

    normalize_responses_tool_call_arguments(&mut call);

    assert_eq!(call.fn_arguments, json!({"command": "pwd"}));
    parse_incoming_tool_call(&call, None).expect("恢复缺省参数后应能转换为内部工具调用");
}
