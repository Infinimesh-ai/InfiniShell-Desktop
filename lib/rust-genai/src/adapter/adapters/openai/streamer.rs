use crate::adapter::AdapterKind;
use crate::adapter::adapters::support::{StreamerCapturedData, StreamerOptions};
use crate::adapter::inter_stream::{InterStreamEnd, InterStreamEvent};
use crate::adapter::openai::OpenAIAdapter;
use crate::chat::{ChatOptionsSet, StopReason, ToolCall};
use crate::webc::{Event, EventSourceStream};
use crate::{Error, ModelIden, Result};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;
use std::task::{Context, Poll};
use value_ext::JsonValueExt;

fn take_stream_error(message_data: &mut Value, model_iden: &ModelIden) -> Option<Error> {
	let error_body = message_data.x_take::<Value>("error").ok()?;
	Some(Error::ChatResponse {
		model_iden: model_iden.clone(),
		body: error_body,
	})
}

fn take_finish_reason_usage(
	message_data: &mut Value,
	adapter_kind: AdapterKind,
	capture_usage: bool,
) -> Option<crate::chat::Usage> {
	if !capture_usage {
		return None;
	}

	match adapter_kind {
		AdapterKind::Groq => Some(
			message_data
				.x_take("/x_groq/usage")
				.map(|v| OpenAIAdapter::into_usage(adapter_kind, v))
				.unwrap_or_default(),
		),
		AdapterKind::DeepSeek | AdapterKind::Zai | AdapterKind::Fireworks | AdapterKind::Together => Some(
			message_data
				.x_take("usage")
				.map(|v| OpenAIAdapter::into_usage(adapter_kind, v))
				.unwrap_or_default(),
		),
		_ => message_data
			.x_take("usage")
			.ok()
			.map(|v| OpenAIAdapter::into_usage(adapter_kind, v)),
	}
}

pub struct OpenAIStreamer {
	inner: EventSourceStream,
	options: StreamerOptions,

	// -- Set by the poll_next
	/// Flag to prevent polling the EventSource after a MessageStop event
	done: bool,
	captured_data: StreamerCapturedData,
	captured_tool_calls: BTreeMap<usize, ToolCall>,
	pending_events: VecDeque<InterStreamEvent>,
}

impl OpenAIStreamer {
	pub fn new(inner: EventSourceStream, model_iden: ModelIden, options_set: ChatOptionsSet<'_, '_>) -> Self {
		Self {
			inner,
			done: false,
			options: StreamerOptions::new(model_iden, options_set),
			captured_data: Default::default(),
			captured_tool_calls: BTreeMap::new(),
			pending_events: VecDeque::new(),
		}
	}

	/// 按原生索引合并工具参数，同时保留乱序和稀疏索引，避免填充出不存在的调用。
	fn capture_tool_call(&mut self, index: usize, call_id: String, fn_name: String, arguments: String) -> ToolCall {
		let tool_call = ToolCall {
			call_id: call_id.clone(),
			fn_name: fn_name.clone(),
			fn_arguments: Value::String(arguments.clone()),
			thought_signatures: None,
		};

		if !self.options.capture_tool_calls {
			return tool_call;
		}

		if let Some(existing_call) = self.captured_tool_calls.get_mut(&index) {
			// Merge with existing: accumulate arguments as strings
			if let Some(existing_args) = existing_call.fn_arguments.as_str() {
				let accumulated = format!("{existing_args}{arguments}");
				existing_call.fn_arguments = Value::String(accumulated);
			}
			// Update call_id and fn_name on first chunk that has them
			if !fn_name.is_empty() {
				existing_call.call_id = call_id;
				existing_call.fn_name = fn_name;
			}
			existing_call.clone()
		} else {
			self.captured_tool_calls.insert(index, tool_call.clone());
			tool_call
		}
	}
}

impl futures::Stream for OpenAIStreamer {
	type Item = Result<InterStreamEvent>;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		// 同一个 SSE 包可能产生多个事件；先排空它们，再读取下一包或终止标记。
		if let Some(event) = self.pending_events.pop_front() {
			return Poll::Ready(Some(Ok(event)));
		}
		if self.done {
			// The last poll was definitely the end, so end the stream.
			// This will prevent triggering a stream ended error
			return Poll::Ready(None);
		}
		while let Poll::Ready(event) = Pin::new(&mut self.inner).poll_next(cx) {
			match event {
				Some(Ok(Event::Open)) => return Poll::Ready(Some(Ok(InterStreamEvent::Start))),
				Some(Ok(Event::Message(message))) => {
					// -- End Message
					// According to OpenAI Spec, this is the end message
					if message.data == "[DONE]" {
						self.done = true;

						// -- Build the usage and captured_content
						// TODO: Needs to clarify wh for usage we do not adopt the same strategy from captured content below
						let captured_usage = if self.options.capture_usage {
							self.captured_data.usage.take()
						} else {
							None
						};

						// -- Process the captured_tool_calls
						// NOTE: here we attempt to parse the `fn_arguments` if it is string, because it means that it was accumulated
						let captured_tool_calls = if !self.captured_tool_calls.is_empty() {
							let tools_calls: Vec<ToolCall> = std::mem::take(&mut self.captured_tool_calls)
								.into_values()
								.map(|tool_call| {
									// extrat
									let ToolCall {
										call_id,
										fn_name,
										fn_arguments,
										..
									} = tool_call;
									// parse fn_arguments if needed
									let fn_arguments = match fn_arguments {
										Value::String(fn_arguments_string) => {
											// NOTE: Here we are resilient for now, if we cannot parse, just return the original String
											match serde_json::from_str::<Value>(&fn_arguments_string) {
												Ok(fn_arguments) => fn_arguments,
												Err(_) => Value::String(fn_arguments_string),
											}
										}
										_ => fn_arguments,
									};

									ToolCall {
										call_id,
										fn_name,
										fn_arguments,
										thought_signatures: None,
									}
								})
								.collect();
							Some(tools_calls)
						} else {
							None
						};

						// Return the internal stream end
						let inter_stream_end = InterStreamEnd {
							captured_usage,
							captured_stop_reason: self.captured_data.stop_reason.take().map(StopReason::from),
							captured_text_content: self.captured_data.content.take(),
							captured_reasoning_content: self.captured_data.reasoning_content.take(),
							captured_tool_calls,
							captured_thought_signatures: None,
							captured_response_id: None,
						};

						return Poll::Ready(Some(Ok(InterStreamEvent::End(inter_stream_end))));
					}

					// -- Other Content Messages
					// Parse to get the choice
					let mut message_data: Value =
						serde_json::from_str(&message.data).map_err(|serde_error| Error::StreamParse {
							model_iden: self.options.model_iden.clone(),
							serde_error,
						})?;

					if let Some(error) = take_stream_error(&mut message_data, &self.options.model_iden) {
						return Poll::Ready(Some(Err(error)));
					}

					let first_choice: Option<Value> = message_data.x_take("/choices/0").ok();

					let adapter_kind = self.options.model_iden.adapter_kind;

					// If we have a first choice, then it's a normal message
					if let Some(mut first_choice) = first_choice {
						let finish_reason = first_choice.x_take::<Option<String>>("finish_reason").ok().flatten();
						let has_finish_reason = finish_reason.is_some();
						if let Some(finish_reason) = finish_reason {
							self.captured_data.stop_reason = Some(finish_reason);
							if let Some(usage) =
								take_finish_reason_usage(&mut message_data, adapter_kind, self.options.capture_usage)
							{
								self.captured_data.usage = Some(usage);
							}
						}

						// 同包字段没有先后顺序，统一先思考、再正文、最后按数组顺序发工具；
						// finish_reason 只记录终止原因，不能跳过任何同包增量。
						let reasoning_content = first_choice
							.x_take::<Option<String>>("/delta/reasoning_content")
							.ok()
							.flatten()
							.or_else(|| first_choice.x_take::<Option<String>>("/delta/reasoning").ok().flatten());
						if let Some(reasoning_content) = reasoning_content.filter(|content| !content.is_empty()) {
							if self.options.capture_reasoning_content {
								self.captured_data
									.reasoning_content
									.get_or_insert_with(String::new)
									.push_str(&reasoning_content);
							}
							self.pending_events
								.push_back(InterStreamEvent::ReasoningChunk(reasoning_content));
						}

						let content = first_choice.x_take::<Option<String>>("/delta/content").ok().flatten();
						if let Some(content) = content.filter(|content| !content.is_empty()) {
							if self.options.capture_content {
								self.captured_data.content.get_or_insert_with(String::new).push_str(&content);
							}
							self.pending_events.push_back(InterStreamEvent::Chunk(content));
						}

						let delta_tool_calls = first_choice.x_take::<Value>("/delta/tool_calls").ok();
						let has_tool_calls_field = delta_tool_calls.as_ref().is_some_and(|calls| !calls.is_null());
						if let Some(Value::Array(tool_calls)) = delta_tool_calls {
							for mut tool_call in tool_calls {
								if let (Ok(index), Ok(mut function)) =
									(tool_call.x_take::<u32>("index"), tool_call.x_take::<Value>("function"))
								{
									let call_id =
										tool_call.x_take::<String>("id").unwrap_or_else(|_| format!("call_{index}"));
									let fn_name = function.x_take::<String>("name").unwrap_or_default();
									let arguments = function.x_take::<String>("arguments").unwrap_or_default();
									let tool_call = self.capture_tool_call(index as usize, call_id, fn_name, arguments);
									self.pending_events.push_back(InterStreamEvent::ToolCallChunk(tool_call));
								}
							}
						}

						if let Some(event) = self.pending_events.pop_front() {
							return Poll::Ready(Some(Ok(event)));
						}
						if !has_finish_reason && !has_tool_calls_field {
							tracing::warn!("EMPTY CHOICE CONTENT");
						}
					}
					// -- Usage message
					else {
						// If it's not Groq, xAI, DeepSeek the usage is captured at the end when choices are empty or null
						if !matches!(adapter_kind, AdapterKind::Groq)
							&& !matches!(adapter_kind, AdapterKind::DeepSeek)
							&& self.captured_data.usage.is_none() // this might be redundant
							&& self.options.capture_usage
						{
							// permissive for now
							let usage = message_data
								.x_take("usage")
								.map(|v| OpenAIAdapter::into_usage(adapter_kind, v))
								.unwrap_or_default();
							self.captured_data.usage = Some(usage);
						}
					}
				}
				Some(Err(err)) => {
					tracing::error!("Error: {}", err);
					return Poll::Ready(Some(Err(Error::WebStream {
						model_iden: self.options.model_iden.clone(),
						cause: err.to_string(),
						error: err,
					})));
				}
				None => {
					return Poll::Ready(None);
				}
			}
		}
		Poll::Pending
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::adapter::AdapterKind;

	fn test_model() -> ModelIden {
		ModelIden::new(AdapterKind::OpenAI, "test-model")
	}

	#[test]
	fn test_take_stream_error_reads_openai_error_payload() {
		let mut message_data = serde_json::json!({
			"error": {
				"message": "Error in input stream",
				"type": "server_error",
			}
		});

		let err = take_stream_error(&mut message_data, &test_model()).expect("expected stream error");
		match err {
			Error::ChatResponse { body, .. } => {
				assert_eq!(body["message"], "Error in input stream");
				assert_eq!(body["type"], "server_error");
			}
			other => panic!("unexpected error variant: {other:?}"),
		}
	}

	#[test]
	fn test_take_stream_error_none_when_error_key_missing() {
		let mut message_data = serde_json::json!({
			"choices": [{"delta": {"content": "hi"}}]
		});
		assert!(take_stream_error(&mut message_data, &test_model()).is_none());
	}

	#[test]
	fn test_take_finish_reason_usage_reads_inline_openai_usage() {
		let mut message_data = serde_json::json!({
			"usage": {
				"prompt_tokens": 11,
				"completion_tokens": 3,
				"total_tokens": 14
			}
		});

		let usage =
			take_finish_reason_usage(&mut message_data, AdapterKind::OpenAI, true).expect("usage should be captured");

		assert_eq!(usage.prompt_tokens, Some(11));
		assert_eq!(usage.completion_tokens, Some(3));
		assert_eq!(usage.total_tokens, Some(14));
		assert!(message_data.get("usage").is_some_and(Value::is_null));
	}

	#[test]
	fn test_take_finish_reason_usage_respects_capture_flag() {
		let mut message_data = serde_json::json!({
			"usage": {
				"prompt_tokens": 11,
				"completion_tokens": 3,
				"total_tokens": 14
			}
		});

		let usage = take_finish_reason_usage(&mut message_data, AdapterKind::OpenAI, false);

		assert!(usage.is_none());
		assert_eq!(message_data["usage"]["prompt_tokens"], 11);
	}
}
