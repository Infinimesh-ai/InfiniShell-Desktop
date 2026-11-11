use std::sync::Arc;

use futures::StreamExt;
use genai::chat::{ChatStreamEvent, StreamChunk, StreamEnd, ToolCall, ToolChunk};
use serde_json::json;
use warp_multi_agent_api as api;

use super::super::{
    ByopOutputInput, generate_byop_output, into_ai_api_error, make_user_query_message,
};
use super::{RepetitionDetector, guard};
use crate::ai::agent::api::RequestParams;
use crate::ai::agent_providers::openai_compatible::OpenAiCompatibleError;
use crate::ai::api_error::AIApiError;
use crate::settings::{
    AgentProviderApiType, AgentProviderResponsesOptions, ResponsesStateModeSetting,
};

const REPEATED_PAIR: &str = "已完成当前状态检查，接下来准备继续处理任务并等待下一步结果，随后会报告进展。\n\n正在准备继续处理任务并等待下一步结果，当前没有其他进展，随后会报告详细状态。\n\n";

fn chunk(text: impl Into<String>) -> Result<ChatStreamEvent, OpenAiCompatibleError> {
    Ok(ChatStreamEvent::Chunk(StreamChunk {
        content: text.into(),
    }))
}

fn tool() -> Result<ChatStreamEvent, OpenAiCompatibleError> {
    Ok(ChatStreamEvent::ToolCallChunk(ToolChunk {
        tool_call: ToolCall {
            call_id: "call-1".to_owned(),
            fn_name: "read_files".to_owned(),
            fn_arguments: json!({"paths": ["example.txt"]}),
            thought_signatures: None,
        },
    }))
}

#[test]
fn repetition_detects_alternating_paragraphs_after_distinct_progress() {
    let mut detector = RepetitionDetector::default();
    assert!(!detector.push("这是一段独立的任务进展，当前已经完成前置检查并得到正常结果。\n\n"));

    assert!(detector.push(&REPEATED_PAIR.repeat(12)));
}

#[test]
fn repetition_detects_periods_one_through_four() {
    let paragraphs = [
        "第一项检查已经完成，现在等待后续步骤继续处理，并将在执行完毕之后汇报完整结果。\n\n",
        "第二项检查已经完成，现在等待后续步骤继续处理，并将在执行完毕之后汇报完整结果。\n\n",
        "第三项检查已经完成，现在等待后续步骤继续处理，并将在执行完毕之后汇报完整结果。\n\n",
        "第四项检查已经完成，现在等待后续步骤继续处理，并将在执行完毕之后汇报完整结果。\n\n",
    ];
    for period in 1..=4 {
        let mut detector = RepetitionDetector::default();
        let text = paragraphs[..period].concat();

        assert!(detector.push(&text.repeat(20)), "未识别周期 {period}");
    }
}

#[test]
fn repetition_handles_character_chunks_and_crlf_boundaries() {
    let mut detector = RepetitionDetector::default();
    let text = REPEATED_PAIR.repeat(12).replace('\n', "\r\n");

    assert!(
        text.chars()
            .any(|character| detector.push(&character.to_string()))
    );
}

#[test]
fn repetition_does_not_stop_occasional_repeated_updates() {
    let mut detector = RepetitionDetector::default();

    assert!(!detector.push(&REPEATED_PAIR.repeat(11)));
}

#[test]
fn repetition_requires_a_substantial_repeated_region() {
    let mut detector = RepetitionDetector::default();

    assert!(!detector.push(&"abcdefghijklmnop\n\n".repeat(12)));
}

#[test]
fn repetition_ignores_fenced_code_even_with_blank_lines() {
    let mut detector = RepetitionDetector::default();

    assert!(!detector.push(&format!("```text\n{}\n```\n\n", REPEATED_PAIR.repeat(30))));
    assert!(!detector.push(&REPEATED_PAIR.repeat(11)));
}

#[test]
fn repetition_ignores_tilde_fences_split_across_chunks() {
    let mut detector = RepetitionDetector::default();
    assert!(!detector.push("~~"));
    assert!(!detector.push("~text\n"));

    assert!(!detector.push(&REPEATED_PAIR.repeat(30)));
    assert!(!detector.push("~~~\n\n"));
}

#[test]
fn repetition_ignores_indented_code_and_short_symbol_lines() {
    let mut detector = RepetitionDetector::default();
    assert!(!detector.push(&"    repeated_code_block_with_long_names();\n\n".repeat(100)));

    assert!(!detector.push(&"---===---===---===---===---===---===---\n\n".repeat(100)));
}

#[test]
fn repetition_does_not_join_paragraphs_across_code() {
    let mut detector = RepetitionDetector::default();
    assert!(!detector.push(&REPEATED_PAIR.repeat(8)));
    assert!(!detector.push("```text\nexample\n```\n\n"));

    assert!(!detector.push(&REPEATED_PAIR.repeat(8)));
}

#[test]
fn repetition_bounds_unbroken_text_without_stopping_it() {
    let mut detector = RepetitionDetector::default();

    assert!(!detector.push(&"a".repeat(100_000)));
    assert!(!detector.push("\n\n"));
    assert!(detector.line.is_empty());
    assert!(detector.paragraph.is_empty());
}

#[test]
fn repetition_guard_preserves_received_text_and_blocks_later_tools() {
    let text = REPEATED_PAIR.repeat(12);
    let events = vec![
        chunk(text.clone()),
        tool(),
        Ok(ChatStreamEvent::End(StreamEnd::default())),
    ];
    let output = futures::executor::block_on(
        guard(Box::pin(futures::stream::iter(events)), None).collect::<Vec<_>>(),
    );

    assert_eq!(output.len(), 2);
    assert!(matches!(&output[0], Ok(ChatStreamEvent::Chunk(chunk)) if chunk.content == text));
    assert!(matches!(
        &output[1],
        Err(OpenAiCompatibleError::RepeatedResponse)
    ));
}

#[test]
fn repetition_error_cannot_automatically_retry_or_resume() {
    let output = futures::executor::block_on(
        guard(
            Box::pin(futures::stream::iter([chunk(REPEATED_PAIR.repeat(12))])),
            None,
        )
        .collect::<Vec<_>>(),
    );
    let error = output.into_iter().find_map(Result::err).unwrap();

    let api_error = into_ai_api_error("BYOP stream failed", error);
    assert!(matches!(
        &api_error,
        AIApiError::ProviderProtocol(message) if message == &crate::t!("ai-error-repeated-response")
    ));
    assert!(!api_error.is_retryable());
}

#[test]
fn repetition_guard_resets_at_structured_tool_progress() {
    let events = vec![
        chunk(REPEATED_PAIR.repeat(8)),
        tool(),
        chunk(REPEATED_PAIR.repeat(8)),
        Ok(ChatStreamEvent::End(StreamEnd::default())),
    ];
    let output = futures::executor::block_on(
        guard(Box::pin(futures::stream::iter(events)), None).collect::<Vec<_>>(),
    );

    assert_eq!(output.len(), 4);
    assert!(output.iter().all(Result::is_ok));
}

#[tokio::test]
async fn repetition_provider_abort_keeps_text_without_dispatching_buffered_tools() {
    let text = REPEATED_PAIR.repeat(12);
    let sse = format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"buffered-call","type":"function","function":{"name":"run_shell_command","arguments":"{\"command\":\"pwd\",\"is_read_only\":true}"}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"later-call","type":"function","function":{"name":"run_shell_command","arguments":"{\"command\":\"pwd\",\"is_read_only\":true}"}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
    );
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/v1/chat/completions")
        .with_header("content-type", "text/event-stream")
        .with_body(sse)
        .expect(1)
        .create_async()
        .await;
    let events = collect_provider_stream(
        format!("{}/v1", server.url()),
        AgentProviderApiType::OpenAi,
        Default::default(),
    )
    .await;
    mock.assert_async().await;

    let error = events.last().unwrap().as_ref().unwrap_err();
    assert!(matches!(error.as_ref(), AIApiError::ProviderProtocol(_)));
    assert!(!error.is_retryable());
    assert!(
        !events.iter().any(|event| event.as_ref().is_ok_and(|event| {
            matches!(event.r#type, Some(api::response_event::Type::Finished(_)))
        }))
    );
    let messages: Vec<_> = events
        .iter()
        .filter_map(|event| event.as_ref().ok())
        .filter_map(|event| match &event.r#type {
            Some(api::response_event::Type::ClientActions(actions)) => Some(&actions.actions),
            _ => None,
        })
        .flatten()
        .filter_map(|action| match &action.action {
            Some(api::client_action::Action::AddMessagesToTask(add)) => Some(&add.messages),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(messages.iter().any(|message| matches!(
        &message.message,
        Some(api::message::Message::AgentOutput(output)) if output.text == text
    )));
    let tool_ids: Vec<_> = messages
        .iter()
        .filter_map(|message| match &message.message {
            Some(api::message::Message::ToolCall(call)) => Some(call.tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    // 流结束前的卡片只是占位；缺少 Finished 时 controller 不会启动已缓冲工具。
    assert_eq!(tool_ids, ["buffered-call"]);
    assert!(!messages.iter().any(|message| matches!(
        message.message,
        Some(api::message::Message::ToolCallResult(_))
    )));
}

#[tokio::test]
async fn repetition_provider_abort_cancels_background_response() {
    let mut server = mockito::Server::new_async().await;
    let sse = format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"type":"response.created","sequence_number":1,"response":{"id":"resp-repeated","status":"in_progress"}}),
        json!({"type":"response.output_text.delta","sequence_number":2,"delta":REPEATED_PAIR.repeat(12)}),
    );
    let create = server
        .mock("POST", "/v1/responses")
        .match_body(mockito::Matcher::PartialJson(json!({"background":true})))
        .with_header("content-type", "text/event-stream")
        .with_body(sse)
        .expect(1)
        .create_async()
        .await;
    let cancel = server
        .mock("POST", "/v1/responses/resp-repeated/cancel")
        .with_header("content-type", "application/json")
        .with_body(r#"{"id":"resp-repeated","status":"cancelled"}"#)
        .expect(1)
        .create_async()
        .await;

    let events = collect_provider_stream(
        format!("{}/v1", server.url()),
        AgentProviderApiType::OpenAiResp,
        AgentProviderResponsesOptions {
            state_mode: ResponsesStateModeSetting::PreviousResponse,
            background: true,
            ..Default::default()
        },
    )
    .await;

    create.assert_async().await;
    cancel.assert_async().await;
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap_err().as_ref(),
        AIApiError::ProviderProtocol(_)
    ));
    assert!(
        !events.iter().any(|event| event.as_ref().is_ok_and(|event| {
            matches!(event.r#type, Some(api::response_event::Type::Finished(_)))
        }))
    );
}

async fn collect_provider_stream(
    base_url: String,
    api_type: AgentProviderApiType,
    responses: AgentProviderResponsesOptions,
) -> Vec<Result<api::ResponseEvent, Arc<AIApiError>>> {
    let params = RequestParams::new_for_test(
        vec![],
        vec![api::Task {
            id: "task-1".to_owned(),
            messages: vec![make_user_query_message(
                "task-1",
                "request-1",
                "检查当前目录".to_owned(),
                &[],
            )],
            ..Default::default()
        }],
    );
    let (_cancel, cancellation_rx) = futures::channel::oneshot::channel();
    generate_byop_output(ByopOutputInput {
        params,
        base_url,
        api_key: "test-key".to_owned(),
        model_id: "test-model".to_owned(),
        api_type,
        reasoning_effort: crate::settings::ReasoningEffortSetting::Auto,
        extra_headers: vec![],
        responses,
        task_id: "task-1".to_owned(),
        target_task_id: "task-1".to_owned(),
        needs_create_task: false,
        lrc_command_id: None,
        lrc_should_spawn_subagent: false,
        context_window: Some(200_000),
        cancellation_rx,
        attachment_caps: Default::default(),
    })
    .await
    .unwrap()
    .collect()
    .await
}
