use super::*;
use crate::adapter::AdapterKind;

fn response_with_status(status: &str) -> RespResponse {
	RespResponse {
		id: "resp_test".to_owned(),
		status: status.to_owned(),
		model: "gpt-test".to_owned(),
		..Default::default()
	}
}

#[test]
fn parses_top_level_error_event() {
	let event: RespStreamEvent =
		serde_json::from_str(r#"{"type":"error","code":"server_error","message":"failed","param":null}"#).unwrap();

	match event {
		RespStreamEvent::TopLevelError { code, message, .. } => {
			assert_eq!(code.as_deref(), Some("server_error"));
			assert_eq!(message, "failed");
		}
		other => panic!("unexpected event: {other:?}"),
	}
}

#[test]
fn parses_content_part_added_event() {
	let event: RespStreamEvent = serde_json::from_str(
		r#"{"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}"#,
	)
	.unwrap();

	assert!(matches!(event, RespStreamEvent::ContentPartAdded { .. }));
}

#[test]
fn incomplete_response_is_a_terminal_error_with_recovery_metadata() {
	let mut response = response_with_status("incomplete");
	response.incomplete_details = Some(serde_json::json!({ "reason": "max_output_tokens" }));

	let error = response_terminal_error(
		"response.incomplete",
		response,
		ModelIden::new(AdapterKind::OpenAIResp, "gpt-test"),
	);

	match error {
		Error::ResponsesStreamTerminal {
			event,
			response_id,
			message,
			..
		} => {
			assert_eq!(event, "response.incomplete");
			assert_eq!(response_id.as_deref(), Some("resp_test"));
			assert_eq!(message, "max_output_tokens");
		}
		other => panic!("unexpected error: {other:?}"),
	}
}

#[test]
fn failed_response_preserves_provider_error_code() {
	let mut response = response_with_status("failed");
	response.error = Some(serde_json::json!({
		"code": "rate_limit_exceeded",
		"message": "retry later"
	}));

	let error = response_terminal_error(
		"response.failed",
		response,
		ModelIden::new(AdapterKind::OpenAIResp, "gpt-test"),
	);

	match error {
		Error::ResponsesStreamTerminal { code, message, .. } => {
			assert_eq!(code.as_deref(), Some("rate_limit_exceeded"));
			assert_eq!(message, "retry later");
		}
		other => panic!("unexpected error: {other:?}"),
	}
}
