//! Zap: `update_machine_memory` BYOP 本地拦截工具 descriptor。
//!
//! 该工具不映射 protobuf executor variant。`chat_stream.rs` 会在
//! `parse_incoming_tool_call` 前按名称拦截，解析完整记忆文档并写入本地数据库。

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use warp_multi_agent_api as api;
use warp_ssh_manager::memory::MAX_MEMORY_CHARS;

use super::OpenAiTool;

pub const TOOL_NAME: &str = "update_machine_memory";

#[derive(Debug, Deserialize, Eq, PartialEq)]
pub struct Args {
    pub content: String,
}

fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "string",
                "description": "The complete revised machine memory document. This replaces the previously stored document."
            }
        },
        "required": ["content"],
        "additionalProperties": false
    })
}

fn from_args(_args: &str) -> Result<api::message::tool_call::Tool> {
    Err(anyhow!(
        "update_machine_memory is intercepted by chat_stream BYOP machine memory dispatcher; \
         from_args should never be called"
    ))
}

fn result_to_json(_result: &api::message::tool_call_result::Result) -> Option<Value> {
    None
}

pub static UPDATE_MACHINE_MEMORY: OpenAiTool = OpenAiTool {
    name: TOOL_NAME,
    description: include_str!("../prompts/tool_descriptions/update_machine_memory.md"),
    parameters,
    from_args,
    result_to_json,
};

pub fn parse_args(args: &str) -> Result<Args> {
    serde_json::from_str(args)
        .map_err(|error| anyhow!("update_machine_memory args parse error: {error}"))
}

/// 按 Unicode 字符截断，避免在 CJK 或 emoji 的 UTF-8 编码中间切断。
pub fn truncate_content(content: &str) -> String {
    content.chars().take(MAX_MEMORY_CHARS).collect()
}

/// 本地拦截工具结果必须带 sentinel，controller 才会自动续轮。
pub fn success_result_to_json(stored_chars: usize) -> Value {
    json!({
        "_byop_intercepted": true,
        "status": "ok",
        "stored_chars": stored_chars,
    })
}

pub fn error_result_to_json(message: impl Into<String>) -> Value {
    json!({
        "_byop_intercepted": true,
        "status": "error",
        "message": message.into(),
    })
}

pub fn missing_machine_key_result_to_json() -> Value {
    error_result_to_json("not in an ssh session with machine identity")
}

pub fn invalid_arguments_result_to_json(detail: impl Into<String>) -> Value {
    let detail = detail.into();
    error_result_to_json(format!("invalid arguments: {detail}"))
}

#[cfg(test)]
#[path = "machine_memory_tests.rs"]
mod tests;
