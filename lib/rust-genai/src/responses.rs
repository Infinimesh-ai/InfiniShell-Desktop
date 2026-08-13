//! OpenAI Responses 协议的公开请求序列化入口。

use crate::adapter::{Adapter, AdapterKind, ResponsesAdapter, ServiceType};
use crate::chat::{ChatOptions, ChatOptionsSet, ChatRequest};
use crate::resolver::{AuthData, Endpoint};
use crate::{ModelIden, Result, ServiceTarget};
use serde_json::Value;

/// 把完整 reasoning item 编码成可持久化的签名，保留所有未知字段。
pub fn reasoning_item_signature(item: &Value) -> Option<String> {
	crate::adapter::encode_reasoning_item(item)
}

/// 使用与正式 OpenAI Responses adapter 完全相同的逻辑生成请求 JSON。
/// 原生 HTTP/WS transport 复用这个入口，避免另写一套 message/tool 转换后漂移。
pub fn build_request_payload(
	model_name: &str,
	chat_request: ChatRequest,
	chat_options: &ChatOptions,
	stream: bool,
) -> Result<Value> {
	let model = ModelIden::new(AdapterKind::OpenAIResp, model_name.to_owned());
	let target = ServiceTarget {
		endpoint: Endpoint::from_static("https://api.openai.com/v1/"),
		auth: AuthData::from_single("request-payload-only"),
		model,
	};
	let options = ChatOptionsSet::default().with_chat_options(Some(chat_options));
	let service_type = if stream {
		ServiceType::ChatStream
	} else {
		ServiceType::Chat
	};
	Ok(ResponsesAdapter::to_web_request_data(target, service_type, chat_request, options)?.payload)
}

#[cfg(test)]
#[path = "responses_tests.rs"]
mod tests;
