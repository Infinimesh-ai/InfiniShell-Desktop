use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use serde_with::skip_serializing_none;

/// `/v1/responses` 创建请求。字段与官方 Responses create schema 对齐；
/// `extra` 用于兼容 OpenAI-compatible provider 的扩展，同时保证未知字段可回放。
#[skip_serializing_none]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResponseCreateRequest {
    pub background: Option<bool>,
    pub context_management: Option<Vec<Value>>,
    pub conversation: Option<Value>,
    pub include: Option<Vec<String>>,
    pub input: Option<Value>,
    pub instructions: Option<String>,
    pub max_output_tokens: Option<u64>,
    pub max_tool_calls: Option<u64>,
    pub metadata: Option<BTreeMap<String, String>>,
    pub model: Option<String>,
    pub moderation: Option<Value>,
    pub parallel_tool_calls: Option<bool>,
    pub previous_response_id: Option<String>,
    pub prompt: Option<Value>,
    pub prompt_cache_key: Option<String>,
    pub prompt_cache_options: Option<Value>,
    pub prompt_cache_retention: Option<String>,
    pub reasoning: Option<Value>,
    pub safety_identifier: Option<String>,
    pub service_tier: Option<String>,
    pub store: Option<bool>,
    pub stream: Option<bool>,
    pub stream_options: Option<Value>,
    pub temperature: Option<f64>,
    pub text: Option<Value>,
    pub tool_choice: Option<Value>,
    pub tools: Option<Vec<Value>>,
    pub top_logprobs: Option<u32>,
    pub top_p: Option<f64>,
    pub truncation: Option<String>,
    pub user: Option<String>,
    /// WebSocket 模式专用：`false` 可只预热连接状态而不生成响应。
    pub generate: Option<bool>,
    /// Multi-agent Beta 配置。启用时调用方还必须显式发送 beta header。
    pub multi_agent: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ResponseCreateRequest {
    pub fn local_private(model: impl Into<String>, input: impl Into<Value>) -> Self {
        Self {
            model: Some(model.into()),
            input: Some(input.into()),
            store: Some(false),
            ..Default::default()
        }
    }

    pub fn validate(&self) -> Result<(), ResponseRequestValidationError> {
        if self.previous_response_id.is_some() && self.conversation.is_some() {
            return Err(ResponseRequestValidationError::ConflictingStateHandles);
        }
        Ok(())
    }

    pub fn validate_http(&self) -> Result<(), ResponseRequestValidationError> {
        self.validate()?;
        if self.generate.is_some() {
            return Err(ResponseRequestValidationError::GenerateOutsideWebSocket);
        }
        Ok(())
    }

    pub fn validate_websocket(&self) -> Result<(), ResponseRequestValidationError> {
        self.validate()?;
        if self.stream.is_some() || self.background.is_some() {
            return Err(ResponseRequestValidationError::HttpTransportFieldInWebSocket);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResponseRequestValidationError {
    #[error("previous_response_id 与 conversation 不能同时使用")]
    ConflictingStateHandles,
    #[error("generate 仅用于 WebSocket response.create")]
    GenerateOutsideWebSocket,
    #[error("stream/background 不能用于 WebSocket response.create")]
    HttpTransportFieldInWebSocket,
}

/// Responses 输出/输入 item 的判别类型。未知类型保留原始名称和完整 JSON。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseItemKind {
    Message,
    Reasoning,
    FunctionCall,
    FunctionCallOutput,
    CustomToolCall,
    CustomToolCallOutput,
    FileSearchCall,
    WebSearchCall,
    ComputerCall,
    ComputerCallOutput,
    ImageGenerationCall,
    CodeInterpreterCall,
    LocalShellCall,
    LocalShellCallOutput,
    ShellCall,
    ShellCallOutput,
    ApplyPatchCall,
    ApplyPatchCallOutput,
    McpCall,
    McpListTools,
    McpApprovalRequest,
    McpApprovalResponse,
    ToolSearchCall,
    ToolSearchOutput,
    ToolSearchAdditionalTools,
    Program,
    ProgramOutput,
    Compaction,
    MultiAgentCall,
    MultiAgentCallOutput,
    AgentMessage,
    Unknown(String),
}

impl ResponseItemKind {
    pub fn from_wire(value: &str) -> Self {
        match value {
            "message" => Self::Message,
            "reasoning" => Self::Reasoning,
            "function_call" => Self::FunctionCall,
            "function_call_output" => Self::FunctionCallOutput,
            "custom_tool_call" => Self::CustomToolCall,
            "custom_tool_call_output" => Self::CustomToolCallOutput,
            "file_search_call" => Self::FileSearchCall,
            "web_search_call" => Self::WebSearchCall,
            "computer_call" => Self::ComputerCall,
            "computer_call_output" => Self::ComputerCallOutput,
            "image_generation_call" => Self::ImageGenerationCall,
            "code_interpreter_call" => Self::CodeInterpreterCall,
            "local_shell_call" => Self::LocalShellCall,
            "local_shell_call_output" => Self::LocalShellCallOutput,
            "shell_call" => Self::ShellCall,
            "shell_call_output" => Self::ShellCallOutput,
            "apply_patch_call" => Self::ApplyPatchCall,
            "apply_patch_call_output" => Self::ApplyPatchCallOutput,
            "mcp_call" => Self::McpCall,
            "mcp_list_tools" => Self::McpListTools,
            "mcp_approval_request" => Self::McpApprovalRequest,
            "mcp_approval_response" => Self::McpApprovalResponse,
            "tool_search_call" => Self::ToolSearchCall,
            "tool_search_output" => Self::ToolSearchOutput,
            "tool_search_additional_tools" => Self::ToolSearchAdditionalTools,
            "program" => Self::Program,
            "program_output" => Self::ProgramOutput,
            "compaction" => Self::Compaction,
            "multi_agent_call" => Self::MultiAgentCall,
            "multi_agent_call_output" => Self::MultiAgentCallOutput,
            "agent_message" => Self::AgentMessage,
            other => Self::Unknown(other.to_owned()),
        }
    }

    pub fn as_wire(&self) -> &str {
        match self {
            Self::Message => "message",
            Self::Reasoning => "reasoning",
            Self::FunctionCall => "function_call",
            Self::FunctionCallOutput => "function_call_output",
            Self::CustomToolCall => "custom_tool_call",
            Self::CustomToolCallOutput => "custom_tool_call_output",
            Self::FileSearchCall => "file_search_call",
            Self::WebSearchCall => "web_search_call",
            Self::ComputerCall => "computer_call",
            Self::ComputerCallOutput => "computer_call_output",
            Self::ImageGenerationCall => "image_generation_call",
            Self::CodeInterpreterCall => "code_interpreter_call",
            Self::LocalShellCall => "local_shell_call",
            Self::LocalShellCallOutput => "local_shell_call_output",
            Self::ShellCall => "shell_call",
            Self::ShellCallOutput => "shell_call_output",
            Self::ApplyPatchCall => "apply_patch_call",
            Self::ApplyPatchCallOutput => "apply_patch_call_output",
            Self::McpCall => "mcp_call",
            Self::McpListTools => "mcp_list_tools",
            Self::McpApprovalRequest => "mcp_approval_request",
            Self::McpApprovalResponse => "mcp_approval_response",
            Self::ToolSearchCall => "tool_search_call",
            Self::ToolSearchOutput => "tool_search_output",
            Self::ToolSearchAdditionalTools => "tool_search_additional_tools",
            Self::Program => "program",
            Self::ProgramOutput => "program_output",
            Self::Compaction => "compaction",
            Self::MultiAgentCall => "multi_agent_call",
            Self::MultiAgentCallOutput => "multi_agent_call_output",
            Self::AgentMessage => "agent_message",
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResponseItem {
    pub kind: ResponseItemKind,
    pub id: Option<String>,
    pub call_id: Option<String>,
    pub caller_id: Option<String>,
    pub phase: Option<String>,
    pub raw: Value,
}

impl ResponseItem {
    pub fn from_value(raw: Value) -> Result<Self, ResponseProtocolError> {
        let object = raw
            .as_object()
            .ok_or(ResponseProtocolError::ExpectedObject("response item"))?;
        let wire_kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(ResponseProtocolError::MissingDiscriminator("response item"))?;
        Ok(Self {
            kind: ResponseItemKind::from_wire(wire_kind),
            id: object.get("id").and_then(Value::as_str).map(str::to_owned),
            call_id: object
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            caller_id: object
                .get("caller_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            phase: object
                .get("phase")
                .and_then(Value::as_str)
                .map(str::to_owned),
            raw,
        })
    }
}

impl Serialize for ResponseItem {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ResponseItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Value::deserialize(deserializer)?;
        Self::from_value(raw).map_err(serde::de::Error::custom)
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResponseObject {
    pub id: String,
    pub object: Option<String>,
    pub created_at: Option<u64>,
    pub status: String,
    pub error: Option<ResponseApiError>,
    pub incomplete_details: Option<Value>,
    pub instructions: Option<Value>,
    pub max_output_tokens: Option<u64>,
    pub max_tool_calls: Option<u64>,
    pub model: Option<String>,
    #[serde(default)]
    pub output: Vec<ResponseItem>,
    pub parallel_tool_calls: Option<bool>,
    pub previous_response_id: Option<String>,
    pub reasoning: Option<Value>,
    pub store: Option<bool>,
    pub temperature: Option<f64>,
    pub text: Option<Value>,
    pub tool_choice: Option<Value>,
    pub tools: Option<Vec<Value>>,
    pub top_logprobs: Option<u32>,
    pub top_p: Option<f64>,
    pub truncation: Option<String>,
    pub usage: Option<ResponseUsage>,
    pub background: Option<bool>,
    pub conversation: Option<Value>,
    pub service_tier: Option<String>,
    pub metadata: Option<BTreeMap<String, String>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ResponseObject {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "completed" | "failed" | "incomplete" | "cancelled"
        )
    }

    pub fn is_success(&self) -> bool {
        self.status == "completed"
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResponseApiError {
    pub code: Option<String>,
    pub message: String,
    pub param: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResponseUsage {
    pub input_tokens: Option<u64>,
    pub input_tokens_details: Option<Value>,
    pub output_tokens: Option<u64>,
    pub output_tokens_details: Option<Value>,
    pub total_tokens: Option<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseEventKind {
    Created,
    Queued,
    InProgress,
    Completed,
    Failed,
    Incomplete,
    Cancelled,
    OutputItemAdded,
    OutputItemDone,
    ContentPartAdded,
    ContentPartDone,
    OutputTextDelta,
    OutputTextDone,
    OutputTextAnnotationAdded,
    RefusalDelta,
    RefusalDone,
    FunctionCallArgumentsDelta,
    FunctionCallArgumentsDone,
    FileSearchCallInProgress,
    FileSearchCallSearching,
    FileSearchCallCompleted,
    WebSearchCallInProgress,
    WebSearchCallSearching,
    WebSearchCallCompleted,
    ReasoningSummaryPartAdded,
    ReasoningSummaryPartDone,
    ReasoningSummaryTextDelta,
    ReasoningSummaryTextDone,
    ReasoningTextDelta,
    ReasoningTextDone,
    ImageGenerationCallInProgress,
    ImageGenerationCallGenerating,
    ImageGenerationCallPartialImage,
    ImageGenerationCallCompleted,
    McpCallArgumentsDelta,
    McpCallArgumentsDone,
    McpCallInProgress,
    McpCallCompleted,
    McpCallFailed,
    McpListToolsInProgress,
    McpListToolsCompleted,
    McpListToolsFailed,
    CodeInterpreterCallInProgress,
    CodeInterpreterCallInterpreting,
    CodeInterpreterCallCompleted,
    CodeInterpreterCallCodeDelta,
    CodeInterpreterCallCodeDone,
    CustomToolCallInputDelta,
    CustomToolCallInputDone,
    AudioDelta,
    AudioDone,
    AudioTranscriptDelta,
    AudioTranscriptDone,
    InjectCreated,
    InjectFailed,
    Error,
    Unknown(String),
}

impl ResponseEventKind {
    pub fn from_wire(value: &str) -> Self {
        match value {
            "response.created" => Self::Created,
            "response.queued" => Self::Queued,
            "response.in_progress" => Self::InProgress,
            "response.completed" => Self::Completed,
            "response.failed" => Self::Failed,
            "response.incomplete" => Self::Incomplete,
            "response.cancelled" => Self::Cancelled,
            "response.output_item.added" => Self::OutputItemAdded,
            "response.output_item.done" => Self::OutputItemDone,
            "response.content_part.added" => Self::ContentPartAdded,
            "response.content_part.done" => Self::ContentPartDone,
            "response.output_text.delta" => Self::OutputTextDelta,
            "response.output_text.done" => Self::OutputTextDone,
            "response.output_text.annotation.added" => Self::OutputTextAnnotationAdded,
            "response.refusal.delta" => Self::RefusalDelta,
            "response.refusal.done" => Self::RefusalDone,
            "response.function_call_arguments.delta" => Self::FunctionCallArgumentsDelta,
            "response.function_call_arguments.done" => Self::FunctionCallArgumentsDone,
            "response.file_search_call.in_progress" => Self::FileSearchCallInProgress,
            "response.file_search_call.searching" => Self::FileSearchCallSearching,
            "response.file_search_call.completed" => Self::FileSearchCallCompleted,
            "response.web_search_call.in_progress" => Self::WebSearchCallInProgress,
            "response.web_search_call.searching" => Self::WebSearchCallSearching,
            "response.web_search_call.completed" => Self::WebSearchCallCompleted,
            "response.reasoning_summary_part.added" => Self::ReasoningSummaryPartAdded,
            "response.reasoning_summary_part.done" => Self::ReasoningSummaryPartDone,
            "response.reasoning_summary_text.delta" => Self::ReasoningSummaryTextDelta,
            "response.reasoning_summary_text.done" => Self::ReasoningSummaryTextDone,
            "response.reasoning_text.delta" => Self::ReasoningTextDelta,
            "response.reasoning_text.done" => Self::ReasoningTextDone,
            "response.image_generation_call.in_progress" => Self::ImageGenerationCallInProgress,
            "response.image_generation_call.generating" => Self::ImageGenerationCallGenerating,
            "response.image_generation_call.partial_image" => Self::ImageGenerationCallPartialImage,
            "response.image_generation_call.completed" => Self::ImageGenerationCallCompleted,
            "response.mcp_call_arguments.delta" => Self::McpCallArgumentsDelta,
            "response.mcp_call_arguments.done" => Self::McpCallArgumentsDone,
            "response.mcp_call.in_progress" => Self::McpCallInProgress,
            "response.mcp_call.completed" => Self::McpCallCompleted,
            "response.mcp_call.failed" => Self::McpCallFailed,
            "response.mcp_list_tools.in_progress" => Self::McpListToolsInProgress,
            "response.mcp_list_tools.completed" => Self::McpListToolsCompleted,
            "response.mcp_list_tools.failed" => Self::McpListToolsFailed,
            "response.code_interpreter_call.in_progress" => Self::CodeInterpreterCallInProgress,
            "response.code_interpreter_call.interpreting" => Self::CodeInterpreterCallInterpreting,
            "response.code_interpreter_call.completed" => Self::CodeInterpreterCallCompleted,
            "response.code_interpreter_call_code.delta" => Self::CodeInterpreterCallCodeDelta,
            "response.code_interpreter_call_code.done" => Self::CodeInterpreterCallCodeDone,
            "response.custom_tool_call_input.delta" => Self::CustomToolCallInputDelta,
            "response.custom_tool_call_input.done" => Self::CustomToolCallInputDone,
            "response.audio.delta" => Self::AudioDelta,
            "response.audio.done" => Self::AudioDone,
            "response.audio_transcript.delta" => Self::AudioTranscriptDelta,
            "response.audio_transcript.done" => Self::AudioTranscriptDone,
            "response.inject.created" => Self::InjectCreated,
            "response.inject.failed" => Self::InjectFailed,
            "error" => Self::Error,
            other => Self::Unknown(other.to_owned()),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Incomplete | Self::Cancelled | Self::Error
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResponseStreamEvent {
    pub kind: ResponseEventKind,
    pub sequence_number: Option<u64>,
    pub response_id: Option<String>,
    pub item_id: Option<String>,
    pub output_index: Option<u64>,
    pub content_index: Option<u64>,
    pub raw: Value,
}

impl ResponseStreamEvent {
    pub fn from_value(raw: Value) -> Result<Self, ResponseProtocolError> {
        let object = raw
            .as_object()
            .ok_or(ResponseProtocolError::ExpectedObject("stream event"))?;
        let wire_kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(ResponseProtocolError::MissingDiscriminator("stream event"))?;
        let response_id = object
            .get("response")
            .and_then(Value::as_object)
            .and_then(|response| response.get("id"))
            .and_then(Value::as_str)
            .or_else(|| object.get("response_id").and_then(Value::as_str))
            .map(str::to_owned);
        let item_id = object
            .get("item_id")
            .and_then(Value::as_str)
            .or_else(|| {
                object
                    .get("item")
                    .and_then(Value::as_object)
                    .and_then(|item| item.get("id"))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned);
        Ok(Self {
            kind: ResponseEventKind::from_wire(wire_kind),
            sequence_number: object.get("sequence_number").and_then(Value::as_u64),
            response_id,
            item_id,
            output_index: object.get("output_index").and_then(Value::as_u64),
            content_index: object.get("content_index").and_then(Value::as_u64),
            raw,
        })
    }

    pub fn response(&self) -> Option<Result<ResponseObject, serde_json::Error>> {
        self.raw
            .get("response")
            .cloned()
            .map(serde_json::from_value)
    }
}

impl Serialize for ResponseStreamEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ResponseStreamEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Value::deserialize(deserializer)?;
        Self::from_value(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResponseProtocolError {
    #[error("{0} 应为 JSON object")]
    ExpectedObject(&'static str),
    #[error("{0} 缺少 type 判别字段")]
    MissingDiscriminator(&'static str),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeletedResponse {
    pub id: String,
    pub deleted: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResponseItemPage {
    pub data: Vec<ResponseItem>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputTokenCount {
    pub input_tokens: u64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactResponse {
    #[serde(default)]
    pub output: Vec<ResponseItem>,
    pub usage: Option<ResponseUsage>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversationObject {
    pub id: String,
    #[serde(flatten)]
    pub raw: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversationItemPage {
    pub data: Vec<ResponseItem>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
