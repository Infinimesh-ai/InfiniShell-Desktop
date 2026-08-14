use crate::adapter::adapters::support::{StreamerCapturedData, StreamerOptions};
use crate::adapter::inter_stream::{InterStreamEnd, InterStreamEvent};
use crate::adapter::openai_resp::resp_types::{RespResponse, reasoning_item_signature};
use crate::chat::{ChatOptionsSet, StopReason, ToolCall};
use crate::webc::{Event, EventSourceStream};
use crate::{Error, ModelIden, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::pin::Pin;
use std::task::{Context, Poll};
use value_ext::JsonValueExt;

pub struct OpenAIRespStreamer {
	inner: EventSourceStream,
	options: StreamerOptions,

	// -- Set by the poll_next
	/// Flag to prevent polling the EventSource after a MessageStop event
	done: bool,
	captured_data: StreamerCapturedData,

	in_progress_tool_calls: BTreeMap<usize, ToolCall>,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum RespStreamEvent {
	#[serde(rename = "response.created")]
	ResponseCreated {
		#[serde(default)]
		_response: Value,
	},

	#[serde(rename = "response.output_item.added")]
	OutputItemAdded { output_index: usize, item: Value },

	#[serde(rename = "response.content_part.added")]
	ContentPartAdded {
		#[serde(rename = "output_index")]
		_output_index: usize,
		#[serde(rename = "content_index")]
		_content_index: usize,
		#[serde(rename = "part", default)]
		_part: Value,
	},

	#[serde(rename = "response.output_text.delta")]
	OutputTextDelta {
		#[serde(default)]
		_output_index: usize,
		#[serde(default)]
		_content_index: usize,
		delta: String,
	},

	#[serde(rename = "response.reasoning_text.delta")]
	ReasoningTextDelta {
		#[serde(default)]
		_output_index: usize,
		#[serde(default)]
		_content_index: usize,
		delta: String,
	},

	// Responses API emits distilled reasoning *summaries* under a
	// separate event family when the request opts into
	// `reasoning.summary = "detailed"`. These are not identical to
	// the raw reasoning-text stream; they're a provider-side summary
	// of the reasoning. Treat them the same way at the adapter layer
	// — append into `captured_data.reasoning_content` — so callers
	// get a single normalized stream regardless of which family the
	// provider chose to emit. Without this handler the summary
	// events fell through to `Unknown` and the reasoning_content
	// field came back empty despite a correct request.
	#[serde(rename = "response.reasoning_summary_text.delta")]
	ReasoningSummaryTextDelta {
		#[serde(default)]
		_output_index: usize,
		#[serde(default)]
		_summary_index: usize,
		delta: String,
	},

	#[serde(rename = "response.function_call_arguments.delta")]
	FunctionCallArgumentsDelta {
		#[serde(default)]
		output_index: usize,
		delta: String,
	},

	#[serde(rename = "response.completed")]
	ResponseCompleted { response: RespResponse },

	#[serde(rename = "response.failed")]
	ResponseFailed { response: RespResponse },

	#[serde(rename = "response.incomplete")]
	ResponseIncomplete { response: RespResponse },

	#[serde(rename = "error")]
	TopLevelError {
		#[serde(default)]
		code: Option<String>,
		message: String,
		#[serde(default)]
		_param: Option<String>,
	},

	#[serde(other)]
	Unknown,
}

fn response_terminal_error(event: &'static str, response: RespResponse, model_iden: ModelIden) -> Error {
	let code = response
		.error
		.as_ref()
		.and_then(|error| error.get("code"))
		.and_then(Value::as_str)
		.map(str::to_owned);
	let message = response
		.error
		.as_ref()
		.and_then(|error| error.get("message"))
		.and_then(Value::as_str)
		.or_else(|| {
			response
				.incomplete_details
				.as_ref()
				.and_then(|details| details.get("reason"))
				.and_then(Value::as_str)
		})
		.unwrap_or("Responses API returned a non-completed terminal response")
		.to_owned();

	Error::ResponsesStreamTerminal {
		model_iden,
		event: event.to_owned(),
		response_id: (!response.id.is_empty()).then_some(response.id),
		code,
		message,
	}
}

impl OpenAIRespStreamer {
	pub fn new(inner: EventSourceStream, model_iden: ModelIden, options_set: ChatOptionsSet<'_, '_>) -> Self {
		Self {
			inner,
			done: false,
			options: StreamerOptions::new(model_iden, options_set),
			captured_data: Default::default(),
			in_progress_tool_calls: BTreeMap::new(),
		}
	}
}

impl futures::Stream for OpenAIRespStreamer {
	type Item = Result<InterStreamEvent>;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		if self.done {
			return Poll::Ready(None);
		}

		while let Poll::Ready(event) = Pin::new(&mut self.inner).poll_next(cx) {
			match event {
				Some(Ok(Event::Open)) => return Poll::Ready(Some(Ok(InterStreamEvent::Start))),
				Some(Ok(Event::Message(message))) => {
					let stream_event: RespStreamEvent = match serde_json::from_str(&message.data) {
						Ok(stream_event) => stream_event,
						Err(serde_error) => {
							self.done = true;
							return Poll::Ready(Some(Err(Error::StreamParse {
								model_iden: self.options.model_iden.clone(),
								serde_error,
							})));
						}
					};

					match stream_event {
						RespStreamEvent::ResponseCreated { .. } => {
							// For now, we don't need to do anything with the response object here
							continue;
						}

						RespStreamEvent::OutputItemAdded { output_index, item } => {
							if item.x_get_str("type").ok() == Some("function_call") {
								let call_id = item.x_get_str("call_id").unwrap_or_default().to_string();
								let fn_name = item.x_get_str("name").unwrap_or_default().to_string();

								let tool_call = ToolCall {
									call_id,
									fn_name,
									fn_arguments: Value::String(String::new()),
									thought_signatures: None,
								};

								self.in_progress_tool_calls.insert(output_index, tool_call);
							}
							continue;
						}

						RespStreamEvent::ContentPartAdded { .. } => {
							// We can ignore this as deltas will follow
							continue;
						}

						RespStreamEvent::OutputTextDelta { delta, .. } => {
							if self.options.capture_content {
								match self.captured_data.content {
									Some(ref mut c) => c.push_str(&delta),
									None => self.captured_data.content = Some(delta.clone()),
								}
							}
							return Poll::Ready(Some(Ok(InterStreamEvent::Chunk(delta))));
						}

						RespStreamEvent::ReasoningTextDelta { delta, .. } => {
							if self.options.capture_reasoning_content {
								match self.captured_data.reasoning_content {
									Some(ref mut c) => c.push_str(&delta),
									None => self.captured_data.reasoning_content = Some(delta.clone()),
								}
							}
							return Poll::Ready(Some(Ok(InterStreamEvent::ReasoningChunk(delta))));
						}

						RespStreamEvent::ReasoningSummaryTextDelta { delta, .. } => {
							if self.options.capture_reasoning_content {
								match self.captured_data.reasoning_content {
									Some(ref mut c) => c.push_str(&delta),
									None => self.captured_data.reasoning_content = Some(delta.clone()),
								}
							}
							return Poll::Ready(Some(Ok(InterStreamEvent::ReasoningChunk(delta))));
						}

						RespStreamEvent::FunctionCallArgumentsDelta { output_index, delta } => {
							if let Some(tool_call) = self.in_progress_tool_calls.get_mut(&output_index) {
								if let Some(args) = tool_call.fn_arguments.as_str() {
									let new_args = format!("{}{}", args, delta);
									tool_call.fn_arguments = Value::String(new_args);
								}

								let tool_call_to_send = tool_call.clone();
								return Poll::Ready(Some(Ok(InterStreamEvent::ToolCallChunk(tool_call_to_send))));
							}
							continue;
						}

						RespStreamEvent::ResponseCompleted { response } => {
							self.done = true;
							self.captured_data.stop_reason = Some(response.status.clone());

							if self.options.capture_usage {
								self.captured_data.usage = response.usage.map(Into::into);
							}

							let mut tool_calls = Vec::new();
							for (_, mut tc) in std::mem::take(&mut self.in_progress_tool_calls) {
								// Parse arguments if they are strings
								if let Some(args_str) = tc.fn_arguments.as_str()
									&& let Ok(args_val) = serde_json::from_str(args_str)
								{
									tc.fn_arguments = args_val;
								}
								tool_calls.push(tc);
							}

							if self.options.capture_tool_calls && !tool_calls.is_empty() {
								self.captured_data.tool_calls = Some(tool_calls.clone());
							}

							// Extract encrypted reasoning content from output items
							// (OpenAI equivalent of Gemini thought signatures).
							if self.options.capture_reasoning_content {
								let mut thought_sigs: Vec<String> = Vec::new();
								for item in &response.output {
									if item.x_get_str("type").ok() == Some("reasoning")
										&& item.get("encrypted_content").is_some()
										&& let Some(signature) = reasoning_item_signature(item)
									{
										thought_sigs.push(signature);
									}
								}
								if !thought_sigs.is_empty() {
									self.captured_data.thought_signatures = Some(thought_sigs);
								}
							}

							let inter_stream_end = InterStreamEnd {
								captured_usage: self.captured_data.usage.take(),
								captured_stop_reason: self.captured_data.stop_reason.take().map(StopReason::from),
								captured_text_content: self.captured_data.content.take(),
								captured_reasoning_content: self.captured_data.reasoning_content.take(),
								captured_tool_calls: self.captured_data.tool_calls.take(),
								captured_thought_signatures: self.captured_data.thought_signatures.take(),
								captured_response_id: Some(response.id),
							};

							return Poll::Ready(Some(Ok(InterStreamEvent::End(inter_stream_end))));
						}

						RespStreamEvent::ResponseFailed { response } => {
							self.done = true;
							return Poll::Ready(Some(Err(response_terminal_error(
								"response.failed",
								response,
								self.options.model_iden.clone(),
							))));
						}

						RespStreamEvent::ResponseIncomplete { response } => {
							self.done = true;
							return Poll::Ready(Some(Err(response_terminal_error(
								"response.incomplete",
								response,
								self.options.model_iden.clone(),
							))));
						}

						RespStreamEvent::TopLevelError { code, message, .. } => {
							self.done = true;
							return Poll::Ready(Some(Err(Error::ResponsesStreamTerminal {
								model_iden: self.options.model_iden.clone(),
								event: "error".to_owned(),
								response_id: None,
								code,
								message,
							})));
						}

						RespStreamEvent::Unknown => {
							continue;
						}
					}
				}
				Some(Err(err)) => {
					return Poll::Ready(Some(Err(Error::WebStream {
						model_iden: self.options.model_iden.clone(),
						cause: err.to_string(),
						error: err,
					})));
				}
				None => {
					if !self.done {
						self.done = true;
						return Poll::Ready(Some(Err(Error::ResponsesStreamEnded {
							model_iden: self.options.model_iden.clone(),
						})));
					}
					return Poll::Ready(None);
				}
			}
		}

		Poll::Pending
	}
}

#[cfg(test)]
#[path = "streamer_tests.rs"]
mod tests;
