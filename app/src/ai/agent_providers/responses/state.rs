use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    ResponseCreateRequest, ResponseEventKind, ResponseItem, ResponseItemKind, ResponseObject,
    ResponseStreamEvent,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStateMode {
    /// 本地历史是唯一事实源；请求使用 `store:false` 并显式回放 item。
    #[default]
    LocalReplay,
    /// 使用 `previous_response_id` 让 provider 保存链式状态。
    PreviousResponse,
    /// 使用 Conversations API 的持久 conversation。
    Conversation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseTransportMode {
    Http,
    WebSocket,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResponseSessionState {
    pub mode: ResponseStateMode,
    pub current_response_id: Option<String>,
    pub conversation_id: Option<String>,
    pub last_sequence_number: Option<u64>,
    pub last_status: Option<String>,
    pub request_context_fingerprint: Option<String>,
    /// 完整的 canonical replay window。standalone compact 后必须整体替换，不能自行裁剪。
    pub replay_items: Vec<ResponseItem>,
    /// 未知事件同样保留，供 trace、回放测试和未来协议升级使用。
    pub unknown_events: Vec<ResponseStreamEvent>,
}

impl ResponseSessionState {
    pub fn prepare_request(
        &self,
        request: &mut ResponseCreateRequest,
        transport: ResponseTransportMode,
    ) -> Result<(), ResponseStateError> {
        ensure_include(&mut request.include, "reasoning.encrypted_content");
        match self.mode {
            ResponseStateMode::LocalReplay => {
                if request.background == Some(true) {
                    return Err(ResponseStateError::BackgroundRequiresServerState);
                }
                request.store = Some(false);
                request.conversation = None;
                // 当前产品每轮建立新的 WS 连接；connection-local 的 previous response
                // 不会跨连接存在，因此两种传输都必须完整回放本地 canonical window。
                let _ = transport;
                request.previous_response_id = None;
                prepend_replay_items(&mut request.input, &self.replay_items);
            }
            ResponseStateMode::PreviousResponse => {
                request.store = Some(true);
                request.conversation = None;
                request.previous_response_id = self.current_response_id.clone();
            }
            ResponseStateMode::Conversation => {
                let conversation_id = self
                    .conversation_id
                    .as_ref()
                    .ok_or(ResponseStateError::MissingConversationId)?;
                request.store = Some(true);
                request.previous_response_id = None;
                request.conversation = Some(Value::String(conversation_id.clone()));
            }
        }
        request.validate()?;
        Ok(())
    }

    pub fn record_event(&mut self, event: ResponseStreamEvent) -> ResponseEventDisposition {
        if let Some(sequence) = event.sequence_number {
            if self
                .last_sequence_number
                .is_some_and(|last_sequence| sequence <= last_sequence)
            {
                return ResponseEventDisposition::Duplicate;
            }
            let gap = self
                .last_sequence_number
                .is_some_and(|last_sequence| sequence > last_sequence.saturating_add(1));
            self.last_sequence_number = Some(sequence);
            if gap {
                self.record_event_payload(&event);
                return ResponseEventDisposition::GapDetected;
            }
        }
        self.record_event_payload(&event);
        ResponseEventDisposition::Applied
    }

    pub fn record_response(&mut self, response: &ResponseObject) {
        self.current_response_id = Some(response.id.clone());
        self.last_status = Some(response.status.clone());
        if response.is_success() {
            self.extend_replay(response.output.iter().cloned());
        }
    }

    pub fn replace_with_compaction(&mut self, items: Vec<ResponseItem>) {
        self.replay_items = items;
        self.current_response_id = None;
        self.last_sequence_number = None;
        self.last_status = Some("compacted".to_owned());
    }

    pub fn fall_back_to_local_replay(&mut self) {
        self.mode = ResponseStateMode::LocalReplay;
        self.current_response_id = None;
        self.conversation_id = None;
        self.last_sequence_number = None;
    }

    pub fn invalidate_if_context_changed(&mut self, fingerprint: String) -> bool {
        if self.request_context_fingerprint.as_ref() == Some(&fingerprint) {
            return false;
        }
        self.current_response_id = None;
        self.conversation_id = None;
        self.last_sequence_number = None;
        self.last_status = None;
        self.request_context_fingerprint = Some(fingerprint);
        true
    }

    pub fn resume_cursor(&self) -> Option<u64> {
        self.last_sequence_number
    }

    fn record_event_payload(&mut self, event: &ResponseStreamEvent) {
        if let Some(response) = event.response().and_then(Result::ok) {
            self.current_response_id = Some(response.id.clone());
            self.last_status = Some(response.status.clone());
            if matches!(event.kind, ResponseEventKind::Completed) {
                self.extend_replay(response.output);
            }
        } else if let Some(response_id) = &event.response_id {
            self.current_response_id = Some(response_id.clone());
        }

        if matches!(event.kind, ResponseEventKind::OutputItemDone)
            && let Some(raw_item) = event.raw.get("item").cloned()
            && let Ok(item) = ResponseItem::from_value(raw_item)
        {
            self.extend_replay(std::iter::once(item));
        }

        if matches!(event.kind, ResponseEventKind::Unknown(_)) {
            self.unknown_events.push(event.clone());
        }
    }

    fn extend_replay(&mut self, items: impl IntoIterator<Item = ResponseItem>) {
        for item in items {
            let duplicate = item.id.as_ref().is_some_and(|id| {
                self.replay_items
                    .iter()
                    .any(|existing| existing.id.as_ref() == Some(id))
            });
            if !duplicate {
                self.replay_items.push(item);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseEventDisposition {
    Applied,
    Duplicate,
    GapDetected,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResponseStateError {
    #[error("本地/ZDR 优先模式不能使用 background")]
    BackgroundRequiresServerState,
    #[error("Conversation 模式缺少 conversation id")]
    MissingConversationId,
    #[error(transparent)]
    InvalidRequest(#[from] super::ResponseRequestValidationError),
}

/// 对会影响 provider state 兼容性的请求上下文生成稳定指纹。
/// endpoint、模型、instructions 和工具 schema 任一变化都会令旧链失效。
pub fn response_request_context_fingerprint(
    endpoint: &str,
    model: Option<&str>,
    instructions: Option<&str>,
    tools: Option<&[Value]>,
) -> String {
    let value = serde_json::json!({
        "endpoint": endpoint,
        "model": model,
        "instructions": instructions,
        "tools": tools,
    });
    let canonical = canonical_json(&value);
    let digest = Sha256::digest(canonical.as_bytes());
    hex::encode(digest)
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned()),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_owned()),
                        canonical_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn ensure_include(include: &mut Option<Vec<String>>, value: &str) {
    let include = include.get_or_insert_default();
    if !include.iter().any(|existing| existing == value) {
        include.push(value.to_owned());
    }
}

fn prepend_replay_items(input: &mut Option<Value>, replay_items: &[ResponseItem]) {
    if replay_items.is_empty() {
        return;
    }
    let mut values: Vec<Value> = replay_items.iter().map(|item| item.raw.clone()).collect();
    match input.take() {
        Some(Value::Array(mut input_items)) => values.append(&mut input_items),
        Some(input) => values.push(input),
        None => {}
    }
    *input = Some(Value::Array(values));
}

/// 仅供产品策略层快速判断哪些 replay item 必须保留。
pub fn is_reasoning_or_tool_chain_item(item: &ResponseItem) -> bool {
    matches!(
        item.kind,
        ResponseItemKind::Reasoning
            | ResponseItemKind::FunctionCall
            | ResponseItemKind::FunctionCallOutput
            | ResponseItemKind::CustomToolCall
            | ResponseItemKind::CustomToolCallOutput
            | ResponseItemKind::Program
            | ResponseItemKind::ProgramOutput
    )
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
