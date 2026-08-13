use serde_json::{Value, json};

use super::*;
use crate::chat::{ChatMessage, ContentPart, MessageContent, ReasoningEffort, Tool};

#[test]
fn 请求序列化保留云会话和原始条目顺序() {
	let raw_reasoning = json!({
		"type": "reasoning",
		"id": "rs_1",
		"encrypted_content": "encrypted",
		"future_field": {"kept": true}
	});
	let request = ChatRequest::from_user("第一轮")
		.append_message(ChatMessage::assistant(MessageContent::from_parts(vec![
			ContentPart::from_custom(raw_reasoning.clone(), None),
		])))
		.append_message(ChatMessage::user("第二轮"))
		.with_conversation("conv_1")
		.with_store(true);
	let options = ChatOptions::default().with_extra_body(json!({
		"background": true,
		"context_management": [{"type": "compaction", "compact_threshold": 64000}],
		"future_option": "preserved"
	}));

	let payload = build_request_payload("gpt-test", request, &options, true).unwrap();
	let input = payload["input"].as_array().expect("input 应为数组");
	let raw_index = input
		.iter()
		.position(|item| item == &raw_reasoning)
		.expect("原始 reasoning item 应被完整保留");
	let second_user_index = input
		.iter()
		.rposition(|item| item["role"] == "user")
		.expect("应有第二轮用户消息");

	assert!(raw_index < second_user_index);
	assert_eq!(payload["conversation"], "conv_1");
	assert_eq!(payload["store"], true);
	assert_eq!(payload["background"], true);
	assert_eq!(payload["future_option"], "preserved");
	assert_eq!(payload["stream"], true);
}

#[test]
fn 状态句柄切换保持互斥() {
	let request = ChatRequest::default()
		.with_previous_response_id("resp_1")
		.with_conversation(json!({"id": "conv_1"}));
	assert!(request.previous_response_id.is_none());
	assert_eq!(request.conversation, Some(json!({"id": "conv_1"})));

	let request = request.with_previous_response_id("resp_2");
	assert_eq!(request.previous_response_id.as_deref(), Some("resp_2"));
	assert_eq!(request.conversation, None::<Value>);
}

#[test]
fn 程序化工具调用保留官方caller值() {
	let request = ChatRequest::from_user("读取文件").with_tools([
		Tool::new("read_files").with_config(json!({"allowed_callers": ["programmatic"]})),
		Tool::new("programmatic_tool_calling")
			.with_config(json!({"type": "programmatic_tool_calling"})),
	]);

	let payload = build_request_payload("gpt-5.6", request, &ChatOptions::default(), true).unwrap();
	let tools = payload["tools"].as_array().expect("应有 tools 数组");

	assert_eq!(tools[0]["allowed_callers"], json!(["programmatic"]));
	assert_eq!(tools[1]["type"], "programmatic_tool_calling");
}

#[test]
fn 程序化工具结果原样回放caller链() {
	let caller = json!({"type": "program", "program_id": "prog_1"});
	let response = crate::chat::ToolResponse::new("call_1", "{}")
		.with_response_caller(Some(caller.clone()), Some("agent_1".to_owned()));
	let request = ChatRequest::from_user("继续").append_message(ChatMessage::from(response));

	let payload = build_request_payload("gpt-5.6", request, &ChatOptions::default(), true).unwrap();
	let output = payload["input"]
		.as_array()
		.expect("应有 input 数组")
		.iter()
		.find(|item| item["type"] == "function_call_output")
		.expect("应有工具结果");

	assert_eq!(output["caller"], caller);
	assert_eq!(output["caller_id"], "agent_1");
}

#[test]
fn gpt56推理扩展与effort和summary合并() {
	let options = ChatOptions::default()
		.with_reasoning_effort(ReasoningEffort::High)
		.with_capture_reasoning_content(true)
		.with_extra_body(json!({
			"reasoning": {"mode": "pro", "context": "all_turns"}
		}));

	let payload = build_request_payload("gpt-5.6", ChatRequest::from_user("分析"), &options, true).unwrap();

	assert_eq!(payload["reasoning"]["effort"], "high");
	assert_eq!(payload["reasoning"]["summary"], "detailed");
	assert_eq!(payload["reasoning"]["mode"], "pro");
	assert_eq!(payload["reasoning"]["context"], "all_turns");
}
