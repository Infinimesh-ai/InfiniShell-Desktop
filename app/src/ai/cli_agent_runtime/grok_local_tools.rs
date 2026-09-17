//! 固定 Grok SDK MCP 反向通道；只转换已注册连接的工具，不把观察快照当作子任务权限。

use std::collections::HashMap;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    INSPECT_TOOL_NAME, LocalToolPermissions, LocalToolReplyTarget, MAX_ARGUMENT_BYTES,
    MCP_SERVER_NAME, NativeLocalToolRequest, SEND_MESSAGE, arguments, identifier, request_id,
    tool_definitions, tool_reply,
};

const SDK_CALL: &str = "x.ai/mcp/sdk_call";
const MCP_VERSION: &str = "2025-11-25";
const MODERN_MCP_VERSION: &str = "2026-07-28";
const PROTOCOL_VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const MAX_REQUEST_RECORDS: usize = 256;
const MAX_RETAINED_REPLY_BYTES: usize = 4 * MAX_ARGUMENT_BYTES;

#[derive(Debug, PartialEq)]
pub(crate) enum GrokMcpRequest {
    Immediate(Value),
    Tool(NativeLocalToolRequest),
    /// 同一工具仍在执行，不重复发出协调器事件；完成时回复所有原生请求 ID。
    Duplicate,
}

struct OuterRequest {
    id: Value,
    fingerprint: [u8; 32],
    inner_key: String,
}

struct InnerRequest {
    id: Value,
    fingerprint: [u8; 32],
    turn_id: Option<String>,
    call_id: Option<String>,
    tool_fingerprint: Option<[u8; 32]>,
    response: Option<Value>,
    cancelled: bool,
    modern: bool,
}

pub(crate) struct GrokMcpBridge {
    server_id: String,
    allow_message: bool,
    outer_requests: HashMap<String, OuterRequest>,
    inner_requests: HashMap<String, InnerRequest>,
    retained_reply_bytes: usize,
    modern_discovered: bool,
}

impl GrokMcpBridge {
    pub(crate) fn new(runtime_generation: Uuid, permissions: LocalToolPermissions) -> Self {
        // allow_spawn 请求不能代替 Grok 创建时固定的权限证明；当前始终拒绝派发。
        Self {
            server_id: format!("infinishell-{runtime_generation}"),
            allow_message: permissions.allow_message,
            outer_requests: HashMap::new(),
            inner_requests: HashMap::new(),
            retained_reply_bytes: 0,
            modern_discovered: false,
        }
    }

    pub(crate) fn server_id(&self) -> &str {
        &self.server_id
    }

    /// 写入 session/new 或 session/load 的 _meta["x.ai/mcp/servers"]，不改用户配置。
    pub(crate) fn registration(&self) -> Value {
        json!([{"name":MCP_SERVER_NAME,"serverId":self.server_id}])
    }

    /// 回合来源须由调用方先完成协议关联验证；不能用当前活跃回合代替缺失的来源标记。
    /// 工具输入和未经验证的 MCP 元数据都不能声明身份；当前真实 SDK 来源链尚待验证。
    pub(crate) fn receive(
        &mut self,
        message: &Value,
        verified_turn_id: Option<&str>,
    ) -> Result<GrokMcpRequest, String> {
        if message["jsonrpc"] != "2.0"
            || !matches!(
                message["method"].as_str(),
                Some(SDK_CALL | "_x.ai/mcp/sdk_call")
            )
            || message.get("result").is_some()
            || message.get("error").is_some()
            || !message["params"].is_object()
            || message["params"]["serverId"] != self.server_id
            || message.to_string().len() > MAX_ARGUMENT_BYTES
        {
            return Err("不是当前已注册的 Grok SDK MCP 请求".into());
        }
        let outer_id = request_id(&message["id"])?;
        let outer_key = outer_id.to_string();
        let outer_fingerprint = fingerprint(message)?;
        let mcp = &message["params"]["message"];
        if mcp["jsonrpc"] != "2.0" || mcp.get("result").is_some() || mcp.get("error").is_some() {
            return Err("Grok SDK MCP 请求版本或消息类型无效".into());
        }
        let inner_id = request_id(&mcp["id"])?;
        let inner_key = inner_id.to_string();
        let inner_fingerprint = fingerprint(mcp)?;
        if let Some(previous) = self.outer_requests.get(&outer_key) {
            if previous.fingerprint != outer_fingerprint {
                return Err("Grok ACP 请求 ID 被不同内容复用".into());
            }
            return self.duplicate(&previous.inner_key, outer_id, verified_turn_id);
        }
        if self.outer_requests.len() >= MAX_REQUEST_RECORDS
            || self.inner_requests.len() >= MAX_REQUEST_RECORDS
        {
            return Err("Grok MCP 身份记录已达到上限，不能删除旧身份后继续执行".into());
        }
        if let Some(previous) = self.inner_requests.get(&inner_key) {
            if previous.fingerprint != inner_fingerprint {
                return Err("Grok MCP 请求 ID 被不同工具或参数复用".into());
            }
            self.outer_requests.insert(
                outer_key,
                OuterRequest {
                    id: outer_id.clone(),
                    fingerprint: outer_fingerprint,
                    inner_key: inner_key.clone(),
                },
            );
            return self.duplicate(&inner_key, outer_id, verified_turn_id);
        }
        let method = mcp["method"].as_str().ok_or("Grok MCP 请求缺少方法")?;
        // 现代协议的上下文来自每次请求；已发现现代端点也不能为缺字段的请求补版本。
        let modern = method != "initialize"
            && (method == "server/discover"
                || self.modern_discovered
                || [
                    PROTOCOL_VERSION_META,
                    "io.modelcontextprotocol/clientInfo",
                    "io.modelcontextprotocol/clientCapabilities",
                ]
                .iter()
                .any(|key| mcp["params"]["_meta"].get(*key).is_some()));
        let metadata_error = if mcp["params"]
            .get("_meta")
            .is_some_and(|meta| !meta.is_object())
        {
            Some(json!({"code":-32602,"message":"Invalid MCP request metadata"}))
        } else {
            modern
                .then(|| validate_modern_metadata(&mcp["params"]))
                .and_then(|result| result.err())
        };
        let mut turn_id = None;
        let mut outcome = if let Some(error) = metadata_error {
            GrokMcpRequest::Immediate(envelope(
                outer_id.clone(),
                json!({"jsonrpc":"2.0","id":inner_id,"error":error}),
            ))
        } else {
            match method {
                "server/discover" => {
                    if !mcp["params"]
                        .as_object()
                        .is_some_and(|params| params.keys().all(|key| key == "_meta"))
                    {
                        return Err("Grok MCP discovery 不能携带额外正文参数".into());
                    }
                    self.modern_discovered = true;
                    GrokMcpRequest::Immediate(envelope(
                        outer_id.clone(),
                        json!({"jsonrpc":"2.0","id":inner_id,"result":{
                            "resultType":"complete","supportedVersions":[MODERN_MCP_VERSION],
                            "capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private",
                            "_meta":{"io.modelcontextprotocol/serverInfo":{
                                "name":MCP_SERVER_NAME,"version":"0.1.0"}}
                        }}),
                    ))
                }
                "initialize" => {
                    if mcp["params"]["protocolVersion"] != MCP_VERSION {
                        return Err("Grok SDK MCP 版本尚未验证".into());
                    }
                    self.modern_discovered = false;
                    GrokMcpRequest::Immediate(envelope(
                        outer_id.clone(),
                        json!({"jsonrpc":"2.0","id":inner_id,"result":{
                            "protocolVersion":MCP_VERSION,"capabilities":{"tools":{}},
                            "serverInfo":{"name":MCP_SERVER_NAME,"version":"0.1.0"}
                        }}),
                    ))
                }
                "tools/list" => {
                    if modern
                        && !mcp["params"].as_object().is_some_and(|params| {
                            params
                                .keys()
                                .all(|key| matches!(key.as_str(), "_meta" | "cursor"))
                                && params.get("cursor").is_none_or(Value::is_string)
                        })
                    {
                        return Err("Grok MCP 工具列表包含未支持的正文参数".into());
                    }
                    GrokMcpRequest::Immediate(envelope(
                        outer_id.clone(),
                        json!({"jsonrpc":"2.0","id":inner_id,"result":{
                            "tools":tool_definitions(false, self.allow_message)
                        }}),
                    ))
                }
                "ping" if !modern => GrokMcpRequest::Immediate(envelope(
                    outer_id.clone(),
                    json!({"jsonrpc":"2.0","id":inner_id,"result":{}}),
                )),
                "tools/call" => {
                    if modern
                        && !mcp["params"].as_object().is_some_and(|params| {
                            params
                                .keys()
                                .all(|key| matches!(key.as_str(), "name" | "arguments" | "_meta"))
                        })
                    {
                        return Err("Grok MCP 工具调用包含未支持的正文参数".into());
                    }
                    let tool = identifier(&mcp["params"]["name"])?;
                    let inputs = arguments(mcp["params"].get("arguments").unwrap_or(&json!({})))?;
                    turn_id = verified_turn_id
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned);
                    let rejection = if turn_id.is_none() {
                        Some("没有已验证的原生回合来源，不能执行 Grok 本地任务工具")
                    } else if tool == "run_agents" {
                        Some("Grok 父任务缺少创建时固定的权限上限，不能派发子任务")
                    } else if tool != INSPECT_TOOL_NAME
                        && !(tool == SEND_MESSAGE.name && self.allow_message)
                    {
                        Some("本地工具不存在或当前连接权限不允许调用")
                    } else {
                        None
                    };
                    let target = LocalToolReplyTarget::Grok {
                        request_id: outer_id.clone(),
                        mcp_id: inner_id.clone(),
                    };
                    match rejection {
                        Some(reason) => {
                            GrokMcpRequest::Immediate(tool_reply(target, Err(reason.into())))
                        }
                        None => GrokMcpRequest::Tool(NativeLocalToolRequest {
                            reply_target: target,
                            call_id: format!(
                                "grok-mcp-{:x}",
                                Sha256::digest(
                                    format!("{}:{inner_key}", self.server_id).as_bytes()
                                )
                            ),
                            turn_id: turn_id.clone().expect("已检查活跃原生回合"),
                            tool,
                            arguments: inputs,
                        }),
                    }
                }
                _ => GrokMcpRequest::Immediate(envelope(
                    outer_id.clone(),
                    json!({"jsonrpc":"2.0","id":inner_id,"error":{
                        "code":-32601,"message":"Grok SDK MCP 方法未实现"
                    }}),
                )),
            }
        };
        if modern && let GrokMcpRequest::Immediate(response) = &mut outcome {
            complete_modern_response(&mut response["result"], method);
        }
        let (call_id, tool_fingerprint, response) = match &outcome {
            GrokMcpRequest::Tool(request) => (
                Some(request.call_id.clone()),
                Some(fingerprint(&json!(request))?),
                None,
            ),
            GrokMcpRequest::Immediate(response) => {
                self.reserve_reply(&response["result"])?;
                (None, None, Some(response["result"].clone()))
            }
            GrokMcpRequest::Duplicate => return Err("Grok MCP 新请求不能处于重复状态".into()),
        };
        self.outer_requests.insert(
            outer_key,
            OuterRequest {
                id: outer_id,
                fingerprint: outer_fingerprint,
                inner_key: inner_key.clone(),
            },
        );
        self.inner_requests.insert(
            inner_key,
            InnerRequest {
                id: inner_id,
                fingerprint: inner_fingerprint,
                turn_id,
                call_id,
                tool_fingerprint,
                response,
                cancelled: false,
                modern,
            },
        );
        Ok(outcome)
    }

    /// 只回复来源已验证、仍未关闭且内容未变的原生工具；派发写入不是 CLI 接收 ACK。
    pub(crate) fn reply(
        &mut self,
        request: &NativeLocalToolRequest,
        result: Result<Value, String>,
        verified_turn_id: Option<&str>,
    ) -> Result<Vec<Value>, String> {
        let LocalToolReplyTarget::Grok { mcp_id, .. } = &request.reply_target else {
            return Err("工具回复不属于 Grok SDK MCP".into());
        };
        let inner_key = mcp_id.to_string();
        let previous = self
            .inner_requests
            .get(&inner_key)
            .ok_or("Grok MCP 工具没有已登记请求")?;
        if previous.cancelled
            || previous.response.is_some()
            || previous.tool_fingerprint != Some(fingerprint(&json!(request))?)
            || previous.turn_id.as_deref() != verified_turn_id
            || verified_turn_id != Some(request.turn_id.as_str())
        {
            return Err("Grok MCP 回复内容已改变、已结束或属于过期回合".into());
        }
        let mut response = tool_reply(request.reply_target.clone(), result)["result"].clone();
        if previous.modern {
            complete_modern_response(&mut response, "tools/call");
        }
        self.reserve_reply(&response)?;
        self.inner_requests
            .get_mut(&inner_key)
            .expect("已检查登记请求")
            .response = Some(response.clone());
        let replies = self
            .outer_requests
            .values()
            .filter(|outer| outer.inner_key == inner_key)
            .map(|outer| envelope(outer.id.clone(), response.clone()))
            .collect();
        Ok(replies)
    }

    /// 留下取消身份，后到请求不能再次触发工具；由适配器发出 LocalToolCancelled。
    pub(crate) fn cancel_turn(&mut self, turn_id: &str) -> Vec<String> {
        let mut calls = Vec::new();
        for request in self.inner_requests.values_mut() {
            if request.turn_id.as_deref() == Some(turn_id)
                && request.response.is_none()
                && !request.cancelled
            {
                request.cancelled = true;
                if let Some(call_id) = &request.call_id {
                    calls.push(call_id.clone());
                }
            }
        }
        calls
    }

    fn duplicate(
        &self,
        inner_key: &str,
        outer_id: Value,
        verified_turn_id: Option<&str>,
    ) -> Result<GrokMcpRequest, String> {
        let previous = self
            .inner_requests
            .get(inner_key)
            .ok_or("Grok MCP 重复请求缺少原始身份")?;
        if previous.cancelled
            || previous
                .turn_id
                .as_deref()
                .is_some_and(|turn| Some(turn) != verified_turn_id)
        {
            let mut response = tool_reply(
                LocalToolReplyTarget::Grok {
                    request_id: outer_id,
                    mcp_id: previous.id.clone(),
                },
                Err("Grok MCP 请求已取消或属于过期回合，不能重投".into()),
            );
            if previous.modern {
                complete_modern_response(&mut response["result"], "tools/call");
            }
            return Ok(GrokMcpRequest::Immediate(response));
        }
        Ok(match &previous.response {
            Some(response) => GrokMcpRequest::Immediate(envelope(outer_id, response.clone())),
            None => GrokMcpRequest::Duplicate,
        })
    }

    fn reserve_reply(&mut self, response: &Value) -> Result<(), String> {
        let bytes = response.to_string().len();
        if bytes > MAX_ARGUMENT_BYTES
            || self.retained_reply_bytes.saturating_add(bytes) > MAX_RETAINED_REPLY_BYTES
        {
            return Err("Grok MCP 回复缓存达到上限，不能删除旧身份后重放工具".into());
        }
        self.retained_reply_bytes += bytes;
        Ok(())
    }
}

fn validate_modern_metadata(params: &Value) -> Result<(), Value> {
    let invalid = || json!({"code":-32602,"message":"Invalid MCP request metadata"});
    let meta = params
        .get("_meta")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let version = meta
        .get(PROTOCOL_VERSION_META)
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    if version != MODERN_MCP_VERSION {
        return Err(
            json!({"code":-32022,"message":"Unsupported MCP protocol version",
            "data":{"supported":[MODERN_MCP_VERSION],"requested":version}}),
        );
    }
    if !meta
        .get("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(Value::is_object)
    {
        return Err(invalid());
    }
    // clientInfo 按规范可省略；出现时必须满足 Implementation 的两个必填字符串。
    if meta
        .get("io.modelcontextprotocol/clientInfo")
        .is_some_and(|info| {
            !info.is_object()
                || !info.get("name").is_some_and(Value::is_string)
                || !info.get("version").is_some_and(Value::is_string)
        })
    {
        return Err(invalid());
    }
    Ok(())
}

fn complete_modern_response(response: &mut Value, method: &str) {
    let Some(result) = response.get_mut("result").and_then(Value::as_object_mut) else {
        return;
    };
    result.insert("resultType".into(), json!("complete"));
    if method == "tools/list" {
        result.insert("ttlMs".into(), json!(0));
        result.insert("cacheScope".into(), json!("private"));
    }
    result.entry("_meta").or_insert_with(|| {
        json!({"io.modelcontextprotocol/serverInfo":{
        "name":MCP_SERVER_NAME,"version":"0.1.0"}})
    });
}

fn envelope(id: Value, mcp_response: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":mcp_response})
}

fn fingerprint(value: &Value) -> Result<[u8; 32], String> {
    Ok(Sha256::digest(serde_json::to_vec(value).map_err(|error| error.to_string())?).into())
}

#[cfg(test)]
#[path = "grok_local_tools_tests.rs"]
mod tests;
