//! CLI 原生工具回调的纯转换与任务边界；执行身份只能由托管连接提供。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use warp_multi_agent_api as api;

use crate::ai::agent_providers::tools::local_orchestration::{RUN_AGENTS, SEND_MESSAGE};

pub(crate) const MCP_SERVER_NAME: &str = "infinishell-local-tasks";
const INSPECT_TOOL_NAME: &str = "inspect_local_tasks";
const MAX_ARGUMENT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalToolPermissions {
    pub allow_spawn: bool,
    pub allow_message: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LocalToolReplyTarget {
    Codex { request_id: Value },
    Claude { request_id: String, mcp_id: Value },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NativeLocalToolRequest {
    pub reply_target: LocalToolReplyTarget,
    pub call_id: String,
    pub turn_id: String,
    pub tool: String,
    pub arguments: Value,
}

/// 这些字段由协调器从当前连接和已提交任务生成，不从工具参数反序列化。
pub(crate) struct TrustedLocalToolContext {
    pub task_id: String,
    pub generation: i64,
    pub runtime_generation: Uuid,
    pub active_turn_id: String,
    pub allow_spawn: bool,
    pub allow_message: bool,
    pub related_task_ids: HashSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LocalToolOperation {
    Spawn(api::RunAgents),
    Send(api::SendMessageToAgent),
    Inspect {
        task_ids: Vec<String>,
        result_page: ResultPage,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BoundLocalToolCall {
    pub sender_task_id: String,
    pub generation: i64,
    pub runtime_generation: Uuid,
    pub message_id: Uuid,
    pub request: NativeLocalToolRequest,
    pub operation: LocalToolOperation,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ClaudeMcpRequest {
    Immediate(Value),
    Tool(NativeLocalToolRequest),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectArgs {
    #[serde(default)]
    task_ids: Vec<String>,
    result_generation: Option<i64>,
    #[serde(default)]
    result_offset: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResultPage {
    pub result_generation: Option<i64>,
    pub result_offset: usize,
}

pub(crate) fn tool_definitions(allow_spawn: bool, allow_message: bool) -> Vec<Value> {
    let mut tools = vec![json!({
        "name": INSPECT_TOOL_NAME,
        "description": "Inspect saved status, delivery receipts and results for this task and its local parent or children. To retrieve a complete historical result, select one task with result_generation and repeat with the returned result_next_offset until null. Offsets are UTF-8 bytes. An unconfirmed message must not be resent automatically.",
        "inputSchema": {"type":"object", "additionalProperties":false,
            "properties":{"task_ids":{"type":"array","maxItems":16,"uniqueItems":true,"items":{"type":"string"}},
                "result_generation":{"type":"integer","minimum":1},
                "result_offset":{"type":"integer","minimum":0}}},
    })];
    for tool in [
        allow_spawn.then_some(&RUN_AGENTS),
        allow_message.then_some(&SEND_MESSAGE),
    ]
    .into_iter()
    .flatten()
    {
        tools.push(json!({"name":tool.name,"description":tool.description,"inputSchema":(tool.parameters)()}));
    }
    tools
}

pub(crate) fn codex_dynamic_tools(allow_spawn: bool, allow_message: bool) -> Vec<Value> {
    tool_definitions(allow_spawn, allow_message)
        .into_iter()
        .map(|mut tool| {
            tool["type"] = json!("function");
            tool
        })
        .collect()
}

pub(crate) fn claude_mcp_config() -> Value {
    json!({"mcpServers":{MCP_SERVER_NAME:{"type":"sdk","name":MCP_SERVER_NAME}}})
}

pub(crate) fn codex_tool_request(
    message: &Value,
    native_session_id: &str,
    active_turn_id: &str,
) -> Result<NativeLocalToolRequest, String> {
    if message["method"] != "item/tool/call" {
        return Err("不是 Codex 本地工具回调".to_owned());
    }
    let params = &message["params"];
    if params["threadId"] != native_session_id
        || params["turnId"] != active_turn_id
        || params
            .get("namespace")
            .is_some_and(|value| !value.is_null())
    {
        return Err("工具回调属于不同会话、过时回合或未知命名空间".to_owned());
    }
    let request_id = request_id(&message["id"])?;
    Ok(NativeLocalToolRequest {
        reply_target: LocalToolReplyTarget::Codex { request_id },
        call_id: identifier(&params["callId"])?,
        turn_id: active_turn_id.to_owned(),
        tool: identifier(&params["tool"])?,
        arguments: arguments(&params["arguments"])?,
    })
}

pub(crate) fn claude_mcp_request(
    message: &Value,
    active_turn_id: Option<&str>,
    allow_spawn: bool,
    allow_message: bool,
) -> Result<ClaudeMcpRequest, String> {
    let request = &message["request"];
    if message["type"] != "control_request"
        || request["subtype"] != "mcp_message"
        || request["server_name"] != MCP_SERVER_NAME
    {
        return Err("不是已注册的 Claude 本地 MCP 请求".to_owned());
    }
    let control_id = identifier(&message["request_id"])?;
    let mcp = &request["message"];
    if mcp["jsonrpc"] != "2.0" {
        return Err("本地 MCP 请求版本无效".to_owned());
    }
    let method = mcp["method"].as_str().ok_or("本地 MCP 请求缺少方法")?;
    if method == "notifications/initialized" {
        if mcp.get("id").is_some() {
            return Err("MCP 初始化通知不能携带请求 ID".to_owned());
        }
        return Ok(ClaudeMcpRequest::Immediate(claude_control_response(
            control_id,
            json!({"jsonrpc":"2.0","result":{}}),
        )));
    }
    let mcp_id = request_id(&mcp["id"])?;
    let result = match method {
        "initialize" => {
            if mcp["params"]["protocolVersion"] != "2025-11-25" {
                return Err("Claude MCP 版本尚未验证".to_owned());
            }
            json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}},
                "serverInfo":{"name":MCP_SERVER_NAME,"version":"0.1.0"}})
        }
        "tools/list" => json!({"tools":tool_definitions(allow_spawn, allow_message)}),
        "ping" => json!({}),
        "tools/call" => {
            let turn_id = active_turn_id
                .filter(|turn_id| !turn_id.is_empty())
                .ok_or("没有活动回合，不能执行本地任务工具")?;
            return Ok(ClaudeMcpRequest::Tool(NativeLocalToolRequest {
                reply_target: LocalToolReplyTarget::Claude {
                    request_id: control_id.clone(),
                    mcp_id,
                },
                call_id: control_id,
                turn_id: turn_id.to_owned(),
                tool: identifier(&mcp["params"]["name"])?,
                arguments: arguments(mcp["params"].get("arguments").unwrap_or(&json!({})))?,
            }));
        }
        _ => return Err("本地 MCP 方法未实现".to_owned()),
    };
    Ok(ClaudeMcpRequest::Immediate(claude_control_response(
        control_id,
        json!({"jsonrpc":"2.0","id":mcp_id,"result":result}),
    )))
}

pub(crate) fn bind_local_tool_call(
    request: NativeLocalToolRequest,
    context: &TrustedLocalToolContext,
) -> Result<BoundLocalToolCall, String> {
    if context.task_id.is_empty()
        || context.generation < 1
        || request.turn_id != context.active_turn_id
    {
        return Err("本地工具调用的任务身份或回合已过时".to_owned());
    }
    let encoded = serde_json::to_string(&request.arguments).map_err(|error| error.to_string())?;
    if encoded.len() > MAX_ARGUMENT_BYTES {
        return Err("本地工具参数超过大小限制".to_owned());
    }
    let operation = match request.tool.as_str() {
        "run_agents" if context.allow_spawn => {
            let api::message::tool_call::Tool::RunAgents(run) =
                (RUN_AGENTS.from_args)(&encoded).map_err(|error| error.to_string())?
            else {
                return Err("本地派发工具解析类型错误".to_owned());
            };
            LocalToolOperation::Spawn(run)
        }
        "send_message_to_agent" if context.allow_message => {
            let api::message::tool_call::Tool::SendMessageToAgent(message) =
                (SEND_MESSAGE.from_args)(&encoded).map_err(|error| error.to_string())?
            else {
                return Err("本地消息工具解析类型错误".to_owned());
            };
            validate_related(&message.addresses, context)?;
            LocalToolOperation::Send(message)
        }
        INSPECT_TOOL_NAME => {
            let args: InspectArgs =
                serde_json::from_str(&encoded).map_err(|error| error.to_string())?;
            let task_ids = if args.task_ids.is_empty() {
                vec![context.task_id.clone()]
            } else {
                args.task_ids
            };
            validate_related(&task_ids, context)?;
            if args
                .result_generation
                .is_some_and(|generation| generation < 1)
                || ((args.result_generation.is_some() || args.result_offset != 0)
                    && task_ids.len() != 1)
                || (args.result_offset != 0 && args.result_generation.is_none())
            {
                return Err("结果分页必须指定一个关联任务和正整数代次".into());
            }
            LocalToolOperation::Inspect {
                task_ids,
                result_page: ResultPage {
                    result_generation: args.result_generation,
                    result_offset: args.result_offset,
                },
            }
        }
        _ => return Err("本地工具不存在或当前权限不允许调用".to_owned()),
    };
    let mut digest = Sha256::new();
    for value in [
        "infinishell-local-tool-v1",
        context.task_id.as_str(),
        &context.generation.to_string(),
        request.turn_id.as_str(),
        request.call_id.as_str(),
    ] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    let digest = digest.finalize();
    Ok(BoundLocalToolCall {
        sender_task_id: context.task_id.clone(),
        generation: context.generation,
        runtime_generation: context.runtime_generation,
        message_id: Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 至少包含 16 字节")),
        request,
        operation,
    })
}

fn validate_related(task_ids: &[String], context: &TrustedLocalToolContext) -> Result<(), String> {
    let unique: HashSet<_> = task_ids.iter().collect();
    if task_ids.is_empty()
        || task_ids.len() > 16
        || unique.len() != task_ids.len()
        || task_ids.iter().any(|task_id| {
            task_id != &context.task_id && !context.related_task_ids.contains(task_id)
        })
    {
        return Err("本地工具只能访问当前任务与已记录的父子任务".to_owned());
    }
    Ok(())
}

fn identifier(value: &Value) -> Result<String, String> {
    value
        .as_str()
        .filter(|value| !value.trim().is_empty() && value.len() <= 4096)
        .map(str::to_owned)
        .ok_or_else(|| "本地工具标识无效".to_owned())
}

fn request_id(value: &Value) -> Result<Value, String> {
    if value.as_i64().is_some() || identifier(value).is_ok() {
        Ok(value.clone())
    } else {
        Err("本地工具请求 ID 无效".to_owned())
    }
}

fn arguments(value: &Value) -> Result<Value, String> {
    if !value.is_object() || value.to_string().len() > MAX_ARGUMENT_BYTES {
        return Err("本地工具参数必须是大小受限的 JSON 对象".to_owned());
    }
    Ok(value.clone())
}

pub(crate) fn tool_reply(target: LocalToolReplyTarget, result: Result<Value, String>) -> Value {
    let (success, value) = match result {
        Ok(value) => (true, value),
        Err(error) => (false, json!({"error":error})),
    };
    let text = value.to_string();
    match target {
        LocalToolReplyTarget::Codex { request_id } => json!({"id":request_id,"result":{
            "success":success,"contentItems":[{"type":"inputText","text":text}]}}),
        LocalToolReplyTarget::Claude { request_id, mcp_id } => claude_control_response(
            request_id,
            json!({"jsonrpc":"2.0","id":mcp_id,"result":{
                "isError":!success,"content":[{"type":"text","text":text}]}}),
        ),
    }
}

fn claude_control_response(request_id: String, mcp_response: Value) -> Value {
    json!({"type":"control_response","response":{"subtype":"success","request_id":request_id,
        "response":{"mcp_response":mcp_response}}})
}

#[cfg(test)]
#[path = "local_tools_tests.rs"]
mod tests;
