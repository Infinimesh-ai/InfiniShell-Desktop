use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Response produced by a tool invocation, paired with the originating tool call ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponse {
	/// Identifier of the originating tool call.
	pub call_id: String,
	/// Tool output payload as a string. Providers may use JSON-serialized content.
	// For now, just a string (would probably be serialized JSON)
	pub content: String,
	/// Responses PTC / Multi-agent 嵌套调用的调用者对象，工具结果必须原样带回。
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub caller: Option<Value>,
	/// 部分 Responses 变体使用扁平 `caller_id`，同样保留以便前向兼容。
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub caller_id: Option<String>,
}

/// Constructor
impl ToolResponse {
	/// Creates a new ToolResponse with the provided tool_call_id and content.
	pub fn new(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
		Self {
			call_id: tool_call_id.into(),
			content: content.into(),
			caller: None,
			caller_id: None,
		}
	}

	pub fn with_response_caller(mut self, caller: Option<Value>, caller_id: Option<String>) -> Self {
		self.caller = caller;
		self.caller_id = caller_id;
		self
	}
}

/// Computed accessors
impl ToolResponse {
	/// Returns an approximate in-memory size of this `ToolResponse`, in bytes,
	/// computed as the sum of the UTF-8 lengths of:
	/// - `call_id`
	/// - `content`
	pub fn size(&self) -> usize {
		self.call_id.len()
			+ self.content.len()
			+ self
				.caller
				.as_ref()
				.and_then(|caller| serde_json::to_string(caller).ok())
				.map(|caller| caller.len())
				.unwrap_or_default()
			+ self.caller_id.as_ref().map(String::len).unwrap_or_default()
	}
}

/// Getters
#[allow(unused)]
impl ToolResponse {
	fn tool_call_id(&self) -> &str {
		&self.call_id
	}

	fn content(&self) -> &str {
		&self.content
	}
}
