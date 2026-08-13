//! Private OpenAI Responses related types to use in the openai_resp chat adapter
//!
//!
//! ## Notes:
//!
//! This is just a subset implementation of the OpenAI Responses API to match to the chat API.
//!
//! At some point, genai, will have a full OpenAI Responses support, and those types
//! and the related parsing logic might move to the `src/resp/...` module (and we will have client.exec_responses(..))

// region:    --- Modules

mod resp_output_helper;
mod resp_response;
mod resp_usage;

pub use resp_response::*;
pub use resp_usage::*;

/// `ContentPart::ThoughtSignature` 中携带完整 Responses reasoning item 的前缀。
/// 其他 provider 的签名保持原字符串；OpenAIResp adapter 看到此前缀时原样回放 JSON。
pub const REASONING_ITEM_SIGNATURE_PREFIX: &str = "openai-responses-reasoning-item-v1:";

pub fn reasoning_item_signature(item: &serde_json::Value) -> Option<String> {
	serde_json::to_string(item)
		.ok()
		.map(|json| format!("{REASONING_ITEM_SIGNATURE_PREFIX}{json}"))
}

pub fn reasoning_item_from_signature(signature: &str) -> Option<serde_json::Value> {
	let json = signature.strip_prefix(REASONING_ITEM_SIGNATURE_PREFIX)?;
	serde_json::from_str(json).ok()
}

#[cfg(test)]
#[path = "resp_reasoning_item_tests.rs"]
mod tests;

// endregion: --- Modules
