//! Zap M4:`run_command_on_hosts` — 项目会话内的跨主机批量命令工具。
//!
//! 与普通内置工具不同,本工具复用 `Tool::CallMcpTool` 的 protobuf 通道
//! (带一个保留哨兵 server_id),从而免费获得既有的 MCP 工具卡确认 UI 与
//! 会话持久化;执行端(`execute/call_mcp_tool.rs`)按 **工具名** 分流到
//! `ProjectHostSessionRouter` 做逐主机串行执行。
//!
//! ## 为什么按名字而不是 server_id 分流
//!
//! protobuf→action 转换(`crates/ai/src/agent/action/convert.rs`)里
//! `server_id` 的解析被 `FeatureFlag::MCPGroupedServerContext` 门控,关闭时
//! 一律是 `None`;因此 M4 全链路只能以 `name == TOOL_NAME` 作为分支键,
//! 哨兵 server_id 仅用于历史回放(`mcp.rs::serialize_outgoing_call`)时把
//! 该调用还原为顶层 function name。

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use warp_multi_agent_api as api;

use super::OpenAiTool;

pub const TOOL_NAME: &str = "run_command_on_hosts";

/// Zap 项目批量工具的保留哨兵 server_id,绝不能与真实 MCP server 冲突。
/// 固定的 v4 形状 UUID(version/variant 位合法),便于任何按 UUID 解析的
/// 代码路径正常通过;真实 MCP server 的 id 由 UUID v4 随机生成,与该常量
/// 碰撞概率可忽略。
pub const SENTINEL_SERVER_ID: &str = "00000000-0000-4000-8000-5a70de70015a";

/// 哨兵 server_id 的 `Uuid` 形式(需要与 `Option<Uuid>` 比较的调用方使用)。
pub fn sentinel_uuid() -> uuid::Uuid {
    uuid::Uuid::parse_str(SENTINEL_SERVER_ID)
        .expect("SENTINEL_SERVER_ID 是合法的 UUID 字面量")
}

/// 单次批量调用的主机数上限(与 JSON Schema 中 maxItems 保持一致)。
pub const MAX_NODE_IDS: usize = 20;
/// 单主机命令超时缺省值(秒)。
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
/// 单主机命令超时上限(秒)。
pub const MAX_TIMEOUT_SECONDS: u64 = 600;

fn default_canary() -> bool {
    true
}

fn default_timeout_seconds() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

/// 批量执行请求参数(模型出参反序列化 + 执行端共用)。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct BatchArgs {
    pub node_ids: Vec<String>,
    pub command: String,
    #[serde(default = "default_canary")]
    pub canary: bool,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

/// 从 JSON Value 解析并校验批量参数;`timeout_seconds` 超限时静默钳制。
pub fn parse_batch_args(value: &Value) -> Result<BatchArgs> {
    let mut args: BatchArgs = serde_json::from_value(value.clone())
        .map_err(|error| anyhow!("run_command_on_hosts args parse error: {error}"))?;
    if args.node_ids.is_empty() {
        return Err(anyhow!("node_ids must not be empty"));
    }
    if args.node_ids.len() > MAX_NODE_IDS {
        return Err(anyhow!("node_ids exceeds the limit of {MAX_NODE_IDS}"));
    }
    if args.command.trim().is_empty() {
        return Err(anyhow!("command must not be empty"));
    }
    args.timeout_seconds = args.timeout_seconds.clamp(1, MAX_TIMEOUT_SECONDS);
    Ok(args)
}

fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "node_ids": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "maxItems": MAX_NODE_IDS,
                "description": "要执行命令的主机 node_id 列表(取自 <project_context> 的主机清单)。"
            },
            "command": {
                "type": "string",
                "description": "要在每台主机上执行的 shell 命令(完整命令行)。避免 pager / 交互式命令。"
            },
            "canary": {
                "type": "boolean",
                "description": "金丝雀模式:先在第一台主机执行,失败(非零退出码或错误)则中止其余主机。默认 true。",
                "default": true
            },
            "timeout_seconds": {
                "type": "integer",
                "description": "单台主机的命令超时秒数,超时返回当前输出快照。默认 120,上限 600。",
                "default": DEFAULT_TIMEOUT_SECONDS,
                "maximum": MAX_TIMEOUT_SECONDS
            }
        },
        "required": ["node_ids", "command"],
        "additionalProperties": false
    })
}

/// 模型出参 → `Tool::CallMcpTool`(哨兵 server_id)。经由既有的
/// `parse_incoming_tool_call` → `tools::lookup` 链路调用,无需在
/// chat_stream 增加拦截分支。
fn from_args(args: &str) -> Result<api::message::tool_call::Tool> {
    let parsed: Value = serde_json::from_str(args)?;
    // 先做参数校验,尽早把明显非法的调用打回给模型。
    parse_batch_args(&parsed)?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| anyhow!("run_command_on_hosts args must be a JSON object"))?;
    Ok(api::message::tool_call::Tool::CallMcpTool(
        api::message::tool_call::CallMcpTool {
            name: TOOL_NAME.to_owned(),
            args: Some(super::mcp::json_object_to_prost_struct(obj)),
            server_id: SENTINEL_SERVER_ID.to_owned(),
        },
    ))
}

/// 结果走 `CallMcpTool` 通道,与真实 MCP 结果在 `tool_call_result::Result`
/// 层不可区分(该层没有工具名/哨兵上下文),因此这里恒返回 `None`,统一由
/// `tools::mcp::serialize_result` 兜底序列化 —— 执行端已把聚合 JSON 作为
/// MCP text content 塞入 Success,debug 序列化仍会完整携带 payload。
fn result_to_json(_result: &api::message::tool_call_result::Result) -> Option<Value> {
    None
}

pub static RUN_COMMAND_ON_HOSTS: OpenAiTool = OpenAiTool {
    name: TOOL_NAME,
    description: include_str!("../prompts/tool_descriptions/run_command_on_hosts.md"),
    parameters,
    from_args,
    result_to_json,
};

#[cfg(test)]
#[path = "project_hosts_tests.rs"]
mod tests;
