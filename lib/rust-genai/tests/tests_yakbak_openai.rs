//! Replay integration tests for the OpenAI (Chat Completions API) adapter.
//!
//! These tests use pre-recorded cassettes from `tests/data/yakbak/openai/`
//! and assert that tool call streaming events flow through correctly.

mod support;

use futures::TryStreamExt;
use genai::chat::*;
use serde_json::json;
use support::yakbak::replay_client;
use support::{TestResult, extract_stream_end};

/// Regression test for the "tool_calls + finish_reason in same SSE chunk" case.
///
/// Some OpenAI-compatible providers (e.g. ZhiPu/BigModel GLM-4 series, Ollama)
/// return the full tool call and `finish_reason: "tool_calls"` in the first
/// content chunk, rather than splitting arguments across multiple delta chunks
/// like OpenAI's native Chat Completions stream.
///
/// The streamer must emit a `ToolCallChunk` event for these combined chunks
/// so downstream consumers (agent loops, stream collectors) can react to
/// tool calls in real time. Before the fix, the streamer captured the tool
/// call into `captured_data` but skipped the `InterStreamEvent::ToolCallChunk`
/// emit, so streams appeared to terminate with `NaturalEnd` and no tool calls.
#[tokio::test]
async fn test_yakbak_openai_stream_tool_call_with_finish_reason() -> TestResult<()> {
	let (client, _server) = replay_client("openai", "stream_tool_call_with_finish_reason").await?;

	let chat_req = ChatRequest::new(vec![
		ChatMessage::system("You are a helpful assistant. Use tools when needed."),
		ChatMessage::user("What is the weather in Paris, France in Celsius?"),
	])
	.append_tool(Tool::new("get_weather").with_schema(json!({
		"type": "object",
		"properties": {
			"city": { "type": "string", "description": "The city name" },
			"country": { "type": "string", "description": "The most likely country of this city name" },
			"unit": { "type": "string", "enum": ["C", "F"], "description": "Temperature unit" }
		},
		"required": ["city", "country", "unit"],
	})));

	let options = ChatOptions::default()
		.with_capture_content(true)
		.with_capture_tool_calls(true)
		.with_capture_usage(true);

	let stream_res = client.exec_chat_stream("gpt-4o-mini", chat_req, Some(&options)).await?;
	let extract = extract_stream_end(stream_res.stream).await?;

	// -- Verify that a ToolCallChunk event was emitted during streaming.
	//    This is the core regression: before the fix, the streamer captured
	//    the tool call into `captured_data` but never produced a stream event,
	//    so agent loops saw NaturalEnd with no tool calls.
	let chunks = &extract.tool_call_chunks;
	assert_eq!(
		chunks.len(),
		1,
		"Should have exactly 1 tool call chunk emitted during streaming, got {}",
		chunks.len()
	);

	let chunk = &chunks[0];
	assert_eq!(chunk.fn_name, "get_weather");
	assert_eq!(chunk.call_id, "call_yakbak_fixture_0");
	// Arguments are accumulated as Value::String during streaming.
	let chunk_args_str = chunk
		.fn_arguments
		.as_str()
		.expect("Streamed chunk args should be Value::String");
	assert!(
		chunk_args_str.contains("Paris"),
		"Streamed chunk args should contain 'Paris', got: {chunk_args_str}"
	);

	// -- Verify captured tool calls in StreamEnd (parsed JSON).
	let tool_calls = extract
		.stream_end
		.captured_tool_calls()
		.ok_or("Should have captured tool calls")?;
	assert_eq!(tool_calls.len(), 1);

	let tc = &tool_calls[0];
	assert_eq!(tc.fn_name, "get_weather");
	assert_eq!(tc.call_id, "call_yakbak_fixture_0");
	assert_eq!(
		tc.fn_arguments,
		json!({"city": "Paris", "country": "France", "unit": "C"})
	);

	// -- Verify stop reason.
	assert_eq!(
		extract.stream_end.captured_stop_reason,
		Some(StopReason::ToolCall("tool_calls".to_string()))
	);

	// -- Verify usage was captured from the second chunk (content: "" + usage).
	let usage = extract.stream_end.captured_usage.as_ref().ok_or("Should have usage")?;
	assert_eq!(usage.prompt_tokens, Some(163));
	assert_eq!(usage.completion_tokens, Some(11));
	assert_eq!(usage.total_tokens, Some(174));

	// -- No text content should be present (tool-only response).
	assert!(
		extract.content.is_none() || extract.content.as_deref() == Some(""),
		"Tool-only response should have no text content, got: {:?}",
		extract.content
	);

	Ok(())
}

async fn replay_mixed_events(scenario: &str, model: &str, options: ChatOptions) -> TestResult<Vec<ChatStreamEvent>> {
	let (client, mut server) = replay_client("openai", scenario).await?;
	let request = ChatRequest::new(vec![ChatMessage::user("Return the fixture response.")]);
	let stream = client.exec_chat_stream(model, request, Some(&options)).await?;
	let events = stream.stream.try_collect().await;
	server.shutdown().await;
	Ok(events?)
}

#[tokio::test]
async fn mixed_deltas_preserve_reasoning_text_and_parallel_tool_arguments() -> TestResult<()> {
	let options = ChatOptions::default()
		.with_capture_content(true)
		.with_capture_reasoning_content(true)
		.with_capture_tool_calls(true)
		.with_capture_usage(true);
	let events = replay_mixed_events("mixed_deltas", "gpt-4o-mini", options).await?;
	let [
		ChatStreamEvent::Start,
		ChatStreamEvent::ReasoningChunk(first_reasoning),
		ChatStreamEvent::Chunk(first_text),
		ChatStreamEvent::ToolCallChunk(first_tool),
		ChatStreamEvent::ToolCallChunk(second_tool),
		ChatStreamEvent::ReasoningChunk(last_reasoning),
		ChatStreamEvent::Chunk(last_text),
		ChatStreamEvent::ToolCallChunk(first_complete),
		ChatStreamEvent::ToolCallChunk(second_complete),
		ChatStreamEvent::End(end),
	] = events.as_slice()
	else {
		panic!("同包内容必须完整发出，再读取下一包或结束事件：{events:?}");
	};
	assert_eq!(first_reasoning.content, "reasoning one");
	assert_eq!(first_text.content, "answer one");
	assert_eq!(first_tool.tool_call.call_id, "call_a");
	assert_eq!(first_tool.tool_call.fn_arguments, json!("{\"path\":"));
	assert_eq!(second_tool.tool_call.call_id, "call_b");
	assert_eq!(second_tool.tool_call.fn_arguments, json!("{\"path\":"));
	assert_eq!(last_reasoning.content, " reasoning two");
	assert_eq!(last_text.content, " answer two");
	assert_eq!(first_complete.tool_call.call_id, "call_a");
	assert_eq!(first_complete.tool_call.fn_name, "read_file");
	assert_eq!(first_complete.tool_call.fn_arguments, json!("{\"path\":\"a\"}"));
	assert_eq!(second_complete.tool_call.call_id, "call_b");
	assert_eq!(second_complete.tool_call.fn_name, "list_files");
	assert_eq!(second_complete.tool_call.fn_arguments, json!("{\"path\":\"b\"}"));
	assert_eq!(end.captured_first_text(), Some("answer one answer two"));
	assert_eq!(
		end.captured_reasoning_content.as_deref(),
		Some("reasoning one reasoning two")
	);
	let tools = end.captured_tool_calls().ok_or("缺少最终工具调用")?;
	assert_eq!(tools.len(), 2);
	assert_eq!(tools[0].fn_arguments, json!({"path":"a"}));
	assert_eq!(tools[1].fn_arguments, json!({"path":"b"}));
	assert_eq!(
		end.captured_stop_reason,
		Some(StopReason::ToolCall("tool_calls".into()))
	);
	let usage = end.captured_usage.as_ref().ok_or("缺少独立 usage 包")?;
	assert_eq!(usage.prompt_tokens, Some(20));
	assert_eq!(usage.completion_tokens, Some(7));
	assert_eq!(usage.total_tokens, Some(27));
	Ok(())
}

#[tokio::test]
async fn mixed_finish_delta_emits_every_event_before_end() -> TestResult<()> {
	let options = ChatOptions::default()
		.with_capture_content(true)
		.with_capture_reasoning_content(true)
		.with_capture_tool_calls(true)
		.with_capture_usage(true);
	let events = replay_mixed_events("mixed_finish_delta", "grok-4", options).await?;
	let [
		ChatStreamEvent::Start,
		ChatStreamEvent::ReasoningChunk(reasoning),
		ChatStreamEvent::Chunk(text),
		ChatStreamEvent::ToolCallChunk(first_tool),
		ChatStreamEvent::ToolCallChunk(second_tool),
		ChatStreamEvent::End(end),
	] = events.as_slice()
	else {
		panic!("finish_reason 不能吞掉同包内容：{events:?}");
	};
	assert_eq!(reasoning.content, "reasoning");
	assert_eq!(text.content, "answer");
	assert_eq!(first_tool.tool_call.call_id, "call_a");
	assert_eq!(second_tool.tool_call.call_id, "call_b");
	assert_eq!(end.captured_first_text(), Some("answer"));
	assert_eq!(end.captured_reasoning_content.as_deref(), Some("reasoning"));
	assert_eq!(end.captured_tool_calls().ok_or("缺少最终工具调用")?.len(), 2);
	assert_eq!(
		end.captured_stop_reason,
		Some(StopReason::ToolCall("tool_calls".into()))
	);
	let usage = end.captured_usage.as_ref().ok_or("缺少同包 usage")?;
	assert_eq!(usage.prompt_tokens, Some(20));
	assert_eq!(usage.completion_tokens, Some(7));
	assert_eq!(usage.total_tokens, Some(27));
	Ok(())
}

#[tokio::test]
async fn mixed_delta_events_do_not_depend_on_capture_options() -> TestResult<()> {
	let options = ChatOptions::default()
		.with_capture_content(false)
		.with_capture_reasoning_content(false)
		.with_capture_tool_calls(false)
		.with_capture_usage(false);
	let events = replay_mixed_events("mixed_finish_delta", "gpt-4o-mini", options).await?;
	let [
		ChatStreamEvent::Start,
		ChatStreamEvent::ReasoningChunk(reasoning),
		ChatStreamEvent::Chunk(text),
		ChatStreamEvent::ToolCallChunk(first_tool),
		ChatStreamEvent::ToolCallChunk(second_tool),
		ChatStreamEvent::End(end),
	] = events.as_slice()
	else {
		panic!("关闭最终内容捕获不应影响流式事件：{events:?}");
	};
	assert_eq!(reasoning.content, "reasoning");
	assert_eq!(text.content, "answer");
	assert_eq!(first_tool.tool_call.call_id, "call_a");
	assert_eq!(second_tool.tool_call.call_id, "call_b");
	assert!(end.captured_content.is_none());
	assert!(end.captured_reasoning_content.is_none());
	assert!(end.captured_usage.is_none());
	assert_eq!(
		end.captured_stop_reason,
		Some(StopReason::ToolCall("tool_calls".into()))
	);
	Ok(())
}

#[tokio::test]
async fn empty_tool_calls_do_not_hide_reasoning_or_text() -> TestResult<()> {
	let events = replay_mixed_events("empty_tools_with_text", "deepseek-reasoner", ChatOptions::default()).await?;
	let [
		ChatStreamEvent::Start,
		ChatStreamEvent::ReasoningChunk(reasoning),
		ChatStreamEvent::Chunk(text),
		ChatStreamEvent::End(end),
	] = events.as_slice()
	else {
		panic!("空工具数组不应吞掉同包文本：{events:?}");
	};
	assert_eq!(reasoning.content, "reasoning");
	assert_eq!(text.content, "answer");
	assert_eq!(end.captured_stop_reason, Some(StopReason::Completed("stop".into())));
	Ok(())
}

#[tokio::test]
async fn sparse_tool_indices_keep_array_order_without_fabricating_calls() -> TestResult<()> {
	let options = ChatOptions::default().with_capture_tool_calls(true);
	let events = replay_mixed_events("sparse_tool_indices", "gpt-4o-mini", options).await?;
	let [
		ChatStreamEvent::Start,
		ChatStreamEvent::ToolCallChunk(first),
		ChatStreamEvent::ToolCallChunk(second),
		ChatStreamEvent::ToolCallChunk(first_complete),
		ChatStreamEvent::ToolCallChunk(second_complete),
		ChatStreamEvent::End(end),
	] = events.as_slice()
	else {
		panic!("工具事件必须按数组顺序完整发出：{events:?}");
	};
	assert_eq!(first.tool_call.call_id, "call_d");
	assert_eq!(second.tool_call.call_id, "call_a");
	assert_eq!(first_complete.tool_call.call_id, "call_a");
	assert_eq!(first_complete.tool_call.fn_name, "read_file");
	assert_eq!(first_complete.tool_call.fn_arguments, json!("{\"path\":\"a\"}"));
	assert_eq!(second_complete.tool_call.call_id, "call_d");
	assert_eq!(second_complete.tool_call.fn_name, "list_files");
	assert_eq!(second_complete.tool_call.fn_arguments, json!("{\"path\":\"d\"}"));
	let tools = end.captured_tool_calls().ok_or("缺少最终工具调用")?;
	assert_eq!(tools.len(), 2);
	assert_eq!(tools[0].call_id, "call_a");
	assert_eq!(tools[0].fn_arguments, json!({"path":"a"}));
	assert_eq!(tools[1].call_id, "call_d");
	assert_eq!(tools[1].fn_arguments, json!({"path":"d"}));
	Ok(())
}
