//! 固定原生 Grok 的 SDK 注册与来源探针；只返回常量，不访问本地任务协调器。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tokio::sync::Notify;
use uuid::Uuid;

use super::{Effects, GrokProtocol, cancelled_permission, run_process};
use crate::ai::cli_agent_runtime::{
    InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand, RuntimeError, RuntimeEventKind,
    SessionOptions, SessionTarget, TurnOutcome, channels, managed_process,
};

const SCOPE: &str = "grok_native_sdk_origin_probe";
const SERVER_NAME: &str = "infinishell-sdk-origin-probe";
const REPLY: &str = "INFINISHELL_SDK_INSPECT_OK";
const MCP_VERSION: &str = "2025-11-25";
const MODERN_MCP_VERSION: &str = "2026-07-28";
const PROTOCOL_VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const MAX_RECORDS: usize = 128;
const PROMPT: &str = "INFINISHELL_GROK_SDK_ORIGIN：只调用 infinishell-sdk-origin-probe 服务的 SDK MCP 工具 inspect 一次，参数必须为 {}。若需要发现工具，只允许调用 search_tool 一次，参数必须严格为 {\"limit\":5,\"query\":\"infinishell-sdk-origin-probe inspect\"}。调用 inspect 成功后只回复 INFINISHELL_SDK_INSPECT_OK。除此之外不得调用其他工具，不得读取或修改文件、创建终端、执行命令、派发任务或发送消息。";

#[derive(Default)]
struct ProbeState {
    // 仅隔离验收根目录保留原生工具入参，运行器不公开或归档此文件。
    private_tool_frames: Option<File>,
    private_tool_frame_bytes: usize,
    private_tool_frame_count: usize,
    initialize_observed: bool,
    known_native_session: Option<String>,
    acknowledged_native_prompt: Option<String>,
    started_native_prompt: Option<String>,
    approval_prompt_open: bool,
    permission_responses: HashMap<String, ([u8; 32], Value)>,
    approval_allow_count: usize,
    search_allow_count: usize,
    inspect_allow_count: usize,
    approval_deny_count: usize,
    approval_duplicate_count: usize,
    mcp_status_fingerprints: HashSet<[u8; 32]>,
    frames: Vec<Value>,
    requests: HashMap<String, ([u8; 32], Value)>,
    outer_requests: HashMap<String, [u8; 32]>,
    tools: BTreeMap<String, NativeToolObservation>,
    origins: Vec<Vec<OriginObservation>>,
    initializations: usize,
    discoveries: usize,
    negotiated_protocol_version: Option<&'static str>,
    tools_lists: usize,
    sdk_requests: usize,
    inspect_calls: usize,
    duplicate_requests: usize,
    permission_requests: usize,
    unexpected_tools: usize,
    unsafe_keys: usize,
    failure: Option<&'static str>,
}

#[derive(Default)]
struct NativeToolObservation {
    session_id: Option<String>,
    prompt_id: Option<String>,
    contract_kind: Option<CalibratedToolKind>,
    probe_tool: bool,
    initial_input_seen: bool,
    final_input_seen: bool,
    completed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CalibratedToolKind {
    Inspect,
    SearchDiscovery,
}

impl CalibratedToolKind {
    fn native_name(self) -> &'static str {
        match self {
            Self::Inspect => "use_tool",
            Self::SearchDiscovery => "search_tool",
        }
    }

    fn public_kind(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::SearchDiscovery => "search_discovery",
        }
    }
}

struct OriginObservation {
    carrier: &'static str,
    fields: Value,
    session_id: Option<String>,
    prompt_id: Option<String>,
    tool_call_id: Option<String>,
}

/// GrokProtocol 仅在测试构建装入此钩子；发行版不注册探针服务。
#[derive(Clone)]
pub(super) struct SdkOriginProbe {
    server_id: String,
    state: Arc<Mutex<ProbeState>>,
    changed: Arc<Notify>,
}

impl SdkOriginProbe {
    fn new(generation: Uuid) -> Self {
        Self {
            server_id: format!("infinishell-sdk-origin-{generation}"),
            state: Arc::new(Mutex::new(ProbeState::default())),
            changed: Arc::new(Notify::new()),
        }
    }

    pub(super) fn decorate_initialize(&self, message: &mut Value) {
        message["params"]["clientCapabilities"]["_meta"]["x.ai/mcp/sdk"] = json!(true);
    }

    pub(super) fn decorate_open_session(&self, params: &mut Value) {
        params["_meta"]["x.ai/mcp/servers"] =
            json!([{"name":SERVER_NAME,"serverId":self.server_id}]);
    }

    /// 已校验运行代的真实运行事件仅用于诊断匹配，不补入 SDK 请求来源。
    fn confirm_runtime_session(&self, session_id: &str) -> Result<(), String> {
        let mut state = self.state.lock().expect("探针状态锁未被破坏");
        if Uuid::parse_str(session_id).is_err()
            || state
                .known_native_session
                .as_deref()
                .is_some_and(|previous| previous != session_id)
        {
            return Err("MCP 诊断会话身份无法确认".into());
        }
        state.known_native_session = Some(session_id.into());
        Ok(())
    }

    /// 仅真实接收确认可设置审批核对身份，不回填 SDK 来源。
    fn confirm_runtime_ack(&self, prompt_id: &str) -> Result<(), String> {
        let mut state = self.state.lock().expect("探针状态锁未被破坏");
        if state.known_native_session.is_none()
            || Uuid::parse_str(prompt_id).is_err()
            || state.acknowledged_native_prompt.is_some()
        {
            return Err("探针审批接收身份无效或重复".into());
        }
        state.acknowledged_native_prompt = Some(prompt_id.into());
        Ok(())
    }

    /// Started 仅打开本测试的一次精确审批窗口，不能成为 SDK 请求来源。
    fn confirm_runtime_started(&self, prompt_id: &str) -> Result<(), String> {
        let mut state = self.state.lock().expect("探针状态锁未被破坏");
        if state.acknowledged_native_prompt.as_deref() != Some(prompt_id)
            || state.started_native_prompt.is_some()
        {
            return Err("探针审批 Started 身份无效或重复".into());
        }
        state.started_native_prompt = Some(prompt_id.into());
        state.approval_prompt_open = true;
        refresh_probe_tools(&mut state);
        Ok(())
    }

    fn close_approval_prompt(&self) {
        self.state
            .lock()
            .expect("探针状态锁未被破坏")
            .approval_prompt_open = false;
    }

    /// 不传入当前回合；SDK 请求的来源只能取自原生帧本身。
    pub(super) fn receive(&mut self, message: &Value) -> Result<Option<Effects>, RuntimeError> {
        let result = {
            let mut state = self.state.lock().expect("探针状态锁未被破坏");
            if !state.initialize_observed
                && message["id"].as_u64() == Some(1)
                && message.get("method").is_none()
                && message.get("error").is_none()
                && message["result"]["protocolVersion"].as_u64() == Some(1)
                && message["result"]["_meta"]["agentVersion"] == super::VERIFIED_VERSION
            {
                let flag = &message["result"]["_meta"]["x.ai/mcp/sdk"];
                state.initialize_observed = true;
                let sequence = state.frames.len() + 1;
                state.frames.push(json!({"event":"probe_initialize_observed", "sequence":sequence,
                    "sdk_capability_present":message["result"]["_meta"].get("x.ai/mcp/sdk").is_some(),
                    "sdk_capability_type":value_type(flag),
                    "sdk_capability_enabled":flag == true}));
            }
            record_private_native_tool_frame(&mut state, message)?;
            observe_native_mcp_status(&mut state, message)?;
            observe_native_tool(&mut state, message)?;
            match message["method"].as_str() {
                Some("x.ai/mcp/sdk_call" | "_x.ai/mcp/sdk_call") => {
                    sdk_request(&mut state, &self.server_id, message).map(Some)
                }
                Some("session/request_permission") => {
                    exact_probe_permission(&mut state, message).map(Some)
                }
                Some(_) | None => Ok(None),
            }
        };
        self.changed.notify_one();
        result
    }

    fn check(&self) -> Result<(), String> {
        self.state
            .lock()
            .expect("探针状态锁未被破坏")
            .failure
            .map_or(Ok(()), |reason| Err(reason.into()))
    }

    fn report(&self, session_id: Option<&str>, prompt_id: Option<&str>) -> Value {
        let state = self.state.lock().expect("探针状态锁未被破坏");
        // 已确认的身份只用于核对，绝不写回缺失来源的 SDK 请求。
        let complete_fields = !state.origins.is_empty()
            && state.origins.iter().all(|origins| {
                origins.iter().any(|origin| {
                    origin.session_id.is_some()
                        && origin.prompt_id.is_some()
                        && origin.tool_call_id.is_some()
                })
            });
        let relation_verified = complete_fields
            && state.failure.is_none()
            && session_id.is_some()
            && prompt_id.is_some()
            && state.origins.iter().all(|origins| {
                origins.iter().all(|origin| {
                    origin
                        .session_id
                        .as_deref()
                        .is_none_or(|id| Some(id) == session_id)
                        && origin
                            .prompt_id
                            .as_deref()
                            .is_none_or(|id| Some(id) == prompt_id)
                        && origin.tool_call_id.as_ref().is_none_or(|id| {
                            state.tools.get(id).is_some_and(|tool| {
                                tool.probe_tool
                                    && tool.session_id.as_deref() == session_id
                                    && tool.prompt_id.as_deref() == prompt_id
                            })
                        })
                        && origin.fields.as_object().is_some_and(|fields| {
                            fields.values().all(|kind| {
                                matches!(kind.as_str(), Some("string" | "absent_or_null"))
                            })
                        })
                }) && origins.iter().any(|origin| {
                    origin.session_id.as_deref() == session_id
                        && origin.prompt_id.as_deref() == prompt_id
                        && origin.tool_call_id.as_ref().is_some_and(|id| {
                            state.tools.get(id).is_some_and(|tool| {
                                tool.probe_tool
                                    && tool.final_input_seen
                                    && tool.completed
                                    && tool.session_id.as_deref() == session_id
                                    && tool.prompt_id.as_deref() == prompt_id
                            })
                        })
                })
            });
        let candidate = !relation_verified
            && state.inspect_calls == 1
            && state.tools.values().filter(|tool| tool.probe_tool).count() == 1;
        let verification = if relation_verified {
            "verified_native_fields"
        } else if candidate {
            "candidate_mapping_only"
        } else if state.inspect_calls > 0 {
            "missing_native_origin"
        } else {
            "unknown"
        };
        json!({
            "sdk_request_count":state.sdk_requests,"inspect_call_count":state.inspect_calls,
            "initialization_count":state.initializations,"tools_list_count":state.tools_lists,
            "discovery_count":state.discoveries,
            "negotiatedProtocolVersion":state.negotiated_protocol_version,
            "servedToolNames":if state.tools_lists > 0 {json!(["inspect"])} else {json!([])},
            "duplicate_request_count":state.duplicate_requests,
            "permission_request_count":state.permission_requests,
            "approval_allow_count":state.approval_allow_count,
            "search_allow_count":state.search_allow_count,"inspect_allow_count":state.inspect_allow_count,
            "approval_deny_count":state.approval_deny_count,
            "approval_duplicate_count":state.approval_duplicate_count,
            "approval_all_denied":state.approval_allow_count == 0,
            "native_contract_verified":native_contract_verified(&state),
            "discovery_native_contract_verified":discovery_native_contract_verified(&state),
            "unexpected_tool_count":state.unexpected_tools,"unsafe_key_name_count":state.unsafe_keys,
            "mcp_status_observation_count":state.mcp_status_fingerprints.len(),
            "origin_verification":verification,"full_native_origin_fields_observed":complete_fields,
            "native_origin_ledger_relation_verified":relation_verified,"candidate_mapping_only":candidate,
            "origin_observations":state.origins.iter().map(|origins| origins.iter().map(|origin| json!({
                "carrier":origin.carrier,"fields":origin.fields,
                "session_id_matches":origin.session_id.is_some() && origin.session_id.as_deref()==session_id,
                "prompt_id_matches":origin.prompt_id.is_some() && origin.prompt_id.as_deref()==prompt_id,
                "tool_call_id_in_native_ledger":origin.tool_call_id.as_ref().is_some_and(|id|state.tools.contains_key(id))
            })).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "frames":state.frames
        })
    }
}

fn digest(value: &Value) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(value).expect("JSON 值可以序列化")).into()
}

fn rpc_id(value: &Value) -> bool {
    value.as_u64().is_some()
        || value
            .as_str()
            .is_some_and(|id| !id.is_empty() && id.len() <= 256)
}

fn projected_id(value: &Value) -> Value {
    if value.as_u64().is_some() {
        return value.clone();
    }
    match value.as_str() {
        Some(id) if Uuid::parse_str(id).is_ok() => json!(id),
        Some(_) => {
            json!({"type":"string","sha256":format!("{:x}",Sha256::digest(value.to_string().as_bytes()))})
        }
        None => json!({"type":value_type(value)}),
    }
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "absent_or_null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn finite_counter(value: &Value) -> Value {
    value
        .as_u64()
        .filter(|value| *value <= i64::MAX as u64)
        .map_or(Value::Null, |value| json!(value))
}

fn fixed_status_marker(value: &Value, allowed: &[&str]) -> Value {
    if value.as_str().is_some_and(|value| allowed.contains(&value)) {
        return value.clone();
    }
    json!({"type":value_type(value),"sha256":format!("{:x}",Sha256::digest(value.to_string().as_bytes()))})
}

fn observe_native_mcp_status(state: &mut ProbeState, message: &Value) -> Result<(), RuntimeError> {
    let method = match message["method"].as_str() {
        Some(
            method @ ("_x.ai/mcp/init_progress"
            | "_x.ai/mcp_initialized"
            | "_x.ai/mcp/server_status"),
        ) => method,
        Some(_) | None => return Ok(()),
    };
    let params = &message["params"];
    let mut frame = json!({"event":"native_mcp_status_observed","notification_method":method,
        "params_type":value_type(params),"session_known":state.known_native_session.is_some(),
        "session_id_present":params.get("sessionId").is_some(),"session_id_type":value_type(&params["sessionId"]),
        "session_id_matches":state.known_native_session.as_deref().is_some_and(|id|params["sessionId"].as_str()==Some(id))});
    match method {
        "_x.ai/mcp/init_progress" => {
            frame["total"] = finite_counter(&params["total"]);
            frame["total_type"] = json!(value_type(&params["total"]));
            frame["connected"] = finite_counter(&params["connected"]);
            frame["connected_type"] = json!(value_type(&params["connected"]));
        }
        "_x.ai/mcp_initialized" => {
            frame["mcp_tool_count"] = finite_counter(&params["mcpToolCount"]);
            frame["mcp_tool_count_type"] = json!(value_type(&params["mcpToolCount"]));
            frame["elapsed_ms"] = finite_counter(&params["elapsedMs"]);
            frame["elapsed_ms_type"] = json!(value_type(&params["elapsedMs"]));
        }
        "_x.ai/mcp/server_status" => {
            frame["name_type"] = json!(value_type(&params["name"]));
            frame["probe_server_name_matches"] = json!(params["name"] == SERVER_NAME);
            frame["mcp_status"] = fixed_status_marker(
                &params["status"],
                &["ready", "initializing", "unavailable", "needsauth"],
            );
            frame["mcp_reason"] = fixed_status_marker(
                &params["reason"],
                &[
                    "transport_closed",
                    "handshake_failed",
                    "config_added",
                    "config_removed",
                    "config_changed",
                    "disabled",
                    "auth_expired",
                    "initialized",
                    "restart_succeeded",
                    "restart_failed",
                    "managed_token_refreshed",
                ],
            );
            frame["detail_present"] = json!(params.get("detail").is_some());
            frame["detail_type"] = json!(value_type(&params["detail"]));
            frame["detail_sha256"] = params.get("detail").map_or(Value::Null, |detail| {
                json!(format!(
                    "{:x}",
                    Sha256::digest(detail.to_string().as_bytes())
                ))
            });
        }
        _ => return Err(RuntimeError::Protocol("MCP 状态通知分类非法".into())),
    }
    // 只保存有界的安全投影；同一通知重复递送不会占用新的诊断槽。
    let fingerprint = digest(&frame);
    if state.mcp_status_fingerprints.contains(&fingerprint) {
        return Ok(());
    }
    if state.frames.len() >= MAX_RECORDS || state.mcp_status_fingerprints.len() >= MAX_RECORDS {
        state.failure = Some("原生 MCP 状态诊断超出预算");
        return Err(RuntimeError::Protocol("原生 MCP 状态诊断超出预算".into()));
    }
    state.mcp_status_fingerprints.insert(fingerprint);
    frame["sequence"] = json!(state.frames.len() + 1);
    state.frames.push(frame);
    Ok(())
}

fn keys(state: &mut ProbeState, value: &Value) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    object
        .keys()
        .filter_map(|key| {
            if key.len() <= 64
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_./".contains(&byte))
                && !key.starts_with("sk")
                && !(key.len() >= 32 && key.bytes().all(|byte| byte.is_ascii_hexdigit()))
            {
                Some(key.clone())
            } else {
                state.unsafe_keys += 1;
                None
            }
        })
        .collect()
}

fn origins(message: &Value) -> Vec<OriginObservation> {
    let params = &message["params"];
    let inner = &params["message"];
    [
        ("outer_meta", &message["_meta"]),
        ("sdk_params", params),
        ("sdk_params_meta", &params["_meta"]),
        ("mcp_envelope", inner),
        ("mcp_envelope_meta", &inner["_meta"]),
        ("mcp_params_meta", &inner["params"]["_meta"]),
    ]
    .into_iter()
    .map(|(carrier, value)| OriginObservation {
        carrier,
        fields: json!({"sessionId":value_type(&value["sessionId"]),
            "promptId":value_type(&value["promptId"]),"toolCallId":value_type(&value["toolCallId"])}),
        session_id: value["sessionId"].as_str().map(str::to_owned),
        prompt_id: value["promptId"].as_str().map(str::to_owned),
        tool_call_id: value["toolCallId"].as_str().map(str::to_owned),
    })
    .collect()
}

fn sdk_request(
    state: &mut ProbeState,
    server_id: &str,
    message: &Value,
) -> Result<Effects, RuntimeError> {
    if state.sdk_requests >= MAX_RECORDS
        || message["jsonrpc"] != "2.0"
        || message["params"]["serverId"] != server_id
        || !rpc_id(&message["id"])
        || message.get("result").is_some()
        || message.get("error").is_some()
    {
        return Err(RuntimeError::Protocol("SDK 探针请求无效或超过预算".into()));
    }
    let inner = &message["params"]["message"];
    if inner["jsonrpc"] != "2.0"
        || !rpc_id(&inner["id"])
        || inner.get("result").is_some()
        || inner.get("error").is_some()
    {
        return Err(RuntimeError::Protocol("SDK 探针内部 MCP 请求无效".into()));
    }
    state.sdk_requests += 1;
    let method = match inner["method"].as_str() {
        Some("initialize") => "initialize",
        Some("tools/list") => "tools/list",
        Some("tools/call") => "tools/call",
        Some("ping") => "ping",
        Some("server/discover") => "server/discover",
        Some(_) | None => "unknown",
    };
    let outer_keys = keys(state, message);
    let params_keys = keys(state, &message["params"]);
    let inner_keys = keys(state, inner);
    let tool_params_keys = keys(state, &inner["params"]);
    let (version_value, protocol_version_carrier) = if method == "initialize" {
        (
            &inner["params"]["protocolVersion"],
            "params.protocolVersion",
        )
    } else {
        (
            &inner["params"]["_meta"][PROTOCOL_VERSION_META],
            "params._meta",
        )
    };
    let requested_protocol_version = version_value
        .as_str()
        .filter(|version| matches!(*version, MCP_VERSION | MODERN_MCP_VERSION));
    let modern = method != "initialize"
        && (method == "server/discover"
            || state.negotiated_protocol_version == Some(MODERN_MCP_VERSION)
            || [
                PROTOCOL_VERSION_META,
                "io.modelcontextprotocol/clientInfo",
                "io.modelcontextprotocol/clientCapabilities",
            ]
            .iter()
            .any(|key| inner["params"]["_meta"].get(*key).is_some()));
    let metadata_error = if inner["params"]
        .get("_meta")
        .is_some_and(|meta| !meta.is_object())
    {
        Some(json!({"code":-32602,"message":"Invalid MCP request metadata"}))
    } else {
        modern
            .then(|| validate_modern_probe_metadata(&inner["params"]))
            .and_then(|result| result.err())
    };
    let metadata_keys = json!({
        "outer":keys(state,&message["_meta"]),"sdk_params":keys(state,&message["params"]["_meta"]),
        "mcp_envelope":keys(state,&inner["_meta"]),"mcp_params":keys(state,&inner["params"]["_meta"])
    });
    state.frames.push(json!({"event":"sdk_request_received","sequence":state.frames.len()+1,
        "outer_id":projected_id(&message["id"]),"inner_id":projected_id(&inner["id"]),
        "mcp_method":method,"wire_method":message["method"],"outer_keys":outer_keys,"params_keys":params_keys,
        "requested_protocol_version":requested_protocol_version,
        "protocol_version_carrier":protocol_version_carrier,
        "requested_protocol_version_sha256":(!version_value.is_null()).then(||format!("{:x}",Sha256::digest(version_value.to_string().as_bytes()))),
        "metadata_schema_valid":modern && metadata_error.is_none(),
        "inner_keys":inner_keys,"tool_params_keys":tool_params_keys,"metadata_keys":metadata_keys,
        "origin_fields":origins(message).iter().map(|origin|json!({"carrier":origin.carrier,"fields":origin.fields})).collect::<Vec<_>>()}));
    let outer_key = message["id"].to_string();
    let fingerprint = digest(message);
    if state
        .outer_requests
        .get(&outer_key)
        .is_some_and(|previous| previous != &fingerprint)
    {
        return Err(RuntimeError::Protocol(
            "SDK 外层请求 ID 被不同内容复用".into(),
        ));
    }
    state.outer_requests.insert(outer_key, fingerprint);
    let key = inner["id"].to_string();
    let fingerprint = digest(inner);
    let response = if let Some((previous, response)) = state.requests.get(&key) {
        if previous != &fingerprint {
            return Err(RuntimeError::Protocol(
                "SDK 内层请求 ID 被不同内容复用".into(),
            ));
        }
        state.duplicate_requests += 1;
        response.clone()
    } else {
        if let Some(error) = metadata_error {
            state.unexpected_tools += 1;
            state.failure = Some("SDK MCP 请求缺少合法的逐请求元数据或版本");
            let response = json!({"jsonrpc":"2.0","id":inner["id"],"error":error});
            state.requests.insert(key, (fingerprint, response.clone()));
            return Ok(Effects {
                writes: vec![json!({"jsonrpc":"2.0","id":message["id"],"result":response})],
                events: Vec::new(),
            });
        }
        let result = match method {
            "server/discover"
                if state.discoveries == 0
                    && inner["params"]
                        .as_object()
                        .is_some_and(|params| params.keys().all(|key| key == "_meta")) =>
            {
                state.discoveries += 1;
                state.negotiated_protocol_version = Some(MODERN_MCP_VERSION);
                state.frames.push(
                    json!({"event":"registration_received","framework":"sdk_mcp",
                    "protocol_version":MODERN_MCP_VERSION,"initialization_mode":"modern_discover"}),
                );
                json!({"resultType":"complete","supportedVersions":[MODERN_MCP_VERSION],
                    "capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private",
                    "_meta":{"io.modelcontextprotocol/serverInfo":{"name":SERVER_NAME,"version":"0.1.0"}}})
            }
            "initialize" if inner["params"]["protocolVersion"] == MCP_VERSION => {
                state.initializations += 1;
                state.negotiated_protocol_version = Some(MCP_VERSION);
                state.frames.push(
                    json!({"event":"registration_received","framework":"sdk_mcp",
                    "protocol_version":MCP_VERSION,"initialization_mode":"legacy_initialize"}),
                );
                json!({"protocolVersion":MCP_VERSION,"capabilities":{"tools":{}},
                    "serverInfo":{"name":SERVER_NAME,"version":"0.1.0"}})
            }
            "tools/list"
                if !modern
                    || (state.tools_lists == 0
                        && inner["params"]
                            .as_object()
                            .is_some_and(|params| params.keys().all(|key| key == "_meta"))) =>
            {
                state.tools_lists += 1;
                json!({"tools":[{"name":"inspect","description":"Read-only constant SDK origin probe",
                    "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
                    "annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}}]})
            }
            "tools/call"
                if inner["params"]["name"] == "inspect"
                    && inner["params"]
                        .get("arguments")
                        .is_none_or(|args| args == &json!({}))
                    && inner["params"].as_object().is_some_and(|params| {
                        params
                            .keys()
                            .all(|key| matches!(key.as_str(), "name" | "arguments" | "_meta"))
                    })
                    && state.inspect_calls == 0 =>
            {
                state.inspect_calls += 1;
                state.origins.push(origins(message));
                json!({"content":[{"type":"text","text":REPLY}],"isError":false})
            }
            "ping" if !modern => json!({}),
            "initialize" | "tools/list" | "tools/call" | "ping" | "server/discover" | "unknown" => {
                state.unexpected_tools += 1;
                state.failure = Some("SDK 探针遇到未允许的方法、参数、版本或额外工具调用");
                let response = json!({"jsonrpc":"2.0","id":inner["id"],
                    "error":{"code":-32601,"message":"SDK origin probe rejects this request"}});
                state.requests.insert(key, (fingerprint, response.clone()));
                return Ok(Effects {
                    writes: vec![json!({"jsonrpc":"2.0","id":message["id"],"result":response})],
                    events: Vec::new(),
                });
            }
            _ => return Err(RuntimeError::Protocol("SDK 探针方法分类非法".into())),
        };
        let mut result = result;
        if modern {
            state.negotiated_protocol_version = Some(MODERN_MCP_VERSION);
            result["resultType"] = json!("complete");
            if method == "tools/list" {
                result["ttlMs"] = json!(0);
                result["cacheScope"] = json!("private");
            }
            result["_meta"] = json!({"io.modelcontextprotocol/serverInfo":{"name":SERVER_NAME,"version":"0.1.0"}});
        }
        let response = json!({"jsonrpc":"2.0","id":inner["id"],"result":result});
        state.requests.insert(key, (fingerprint, response.clone()));
        response
    };
    Ok(Effects {
        writes: vec![json!({"jsonrpc":"2.0","id":message["id"],"result":response})],
        events: Vec::new(),
    })
}

fn validate_modern_probe_metadata(params: &Value) -> Result<(), Value> {
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

fn probe_name(name: &str) -> bool {
    [
        format!("{SERVER_NAME}__inspect"),
        format!("{SERVER_NAME}____inspect"),
        format!("{SERVER_NAME}::inspect"),
        format!("mcp__{SERVER_NAME}__inspect"),
    ]
    .iter()
    .any(|qualified| qualified == name)
}

fn native_tool_kind(name: &Value) -> &'static str {
    if name.as_str().is_some_and(probe_name) {
        return "probe_inspect";
    }
    match name.as_str() {
        Some("ToolSearch") => "ToolSearch",
        Some("McpToolSearch") => "McpToolSearch",
        Some("SearchTools") => "SearchTools",
        Some("ToolLookup") => "ToolLookup",
        Some("DiscoverTools") => "DiscoverTools",
        Some("ListTools") => "ListTools",
        Some("Read") => "Read",
        Some("Write") => "Write",
        Some("Bash") => "Bash",
        Some("inspect") => "inspect",
        Some(_) | None => "other",
    }
}

#[test]
fn native_tool_name_classification_is_closed_and_does_not_grant_permission() {
    for name in [
        "ToolSearch",
        "McpToolSearch",
        "SearchTools",
        "ToolLookup",
        "DiscoverTools",
        "ListTools",
        "Read",
        "Write",
        "Bash",
        "inspect",
    ] {
        let mut state = ProbeState::default();
        observe_native_tool(
            &mut state,
            &json!({"method":"session/update","params":{
            "update":{"sessionUpdate":"tool_call","toolCallId":"native-name-diagnostic",
                "_meta":{"x.ai/tool":{"name":name}}}}}),
        )
        .unwrap();
        assert_eq!(state.frames[0]["native_tool_kind"], name);
        assert_eq!(state.frames[0]["probe_tool"], false);
        assert_eq!(state.unexpected_tools, 1);
        assert_eq!(state.failure, Some("原生工具未匹配完整探针输入与来源账本"));
    }
    assert_eq!(native_tool_kind(&json!("toolsearch")), "other");
    for name in [
        format!("{SERVER_NAME}__inspect"),
        format!("{SERVER_NAME}____inspect"),
        format!("{SERVER_NAME}::inspect"),
        format!("mcp__{SERVER_NAME}__inspect"),
    ] {
        assert_eq!(native_tool_kind(&json!(name)), "probe_inspect");
    }
}

#[test]
fn native_tool_name_diagnostics_omit_arbitrary_name_title_and_input() {
    let canary = "OFFLINE_NATIVE_NAME_SECRET_CANARY";
    for name in [
        json!(canary),
        json!({"name":canary}),
        json!([canary]),
        Value::Null,
    ] {
        let mut state = ProbeState::default();
        observe_native_tool(
            &mut state,
            &json!({"method":"session/update","params":{
            "update":{"sessionUpdate":"tool_call","toolCallId":"native-name-canary",
                "title":canary,"rawInput":{"argument":canary},
                "_meta":{"x.ai/tool":{"name":name}}}}}),
        )
        .unwrap();
        let frame = &state.frames[0];
        assert_eq!(frame["native_tool_name_type"], value_type(&name));
        assert_eq!(
            frame["native_tool_name_sha256"],
            format!("{:x}", Sha256::digest(name.to_string().as_bytes()))
        );
        assert_eq!(frame["native_tool_kind"], "other");
        assert!(!frame.to_string().contains(canary));
        assert_eq!(state.unexpected_tools, 1);
        assert_eq!(state.sdk_requests, 0);
        assert_eq!(state.inspect_calls, 0);
    }
}

#[test]
fn native_tool_name_diagnostics_do_not_infer_name_from_title() {
    let mut state = ProbeState::default();
    observe_native_tool(
        &mut state,
        &json!({"method":"session/update","params":{
        "update":{"sessionUpdate":"tool_call","toolCallId":"native-title-only",
            "title":format!("{SERVER_NAME}__inspect")}}}),
    )
    .unwrap();
    assert_eq!(state.frames[0]["native_tool_name_type"], "absent");
    assert_eq!(state.frames[0]["native_tool_name_sha256"], Value::Null);
    assert_eq!(state.frames[0]["native_tool_kind"], "other");
    assert_eq!(state.frames[0]["probe_tool"], false);
    assert_eq!(state.failure, Some("原生工具未匹配完整探针输入与来源账本"));
}

fn record_private_native_tool_frame(
    state: &mut ProbeState,
    message: &Value,
) -> Result<(), RuntimeError> {
    if state.private_tool_frames.is_none()
        || !(message["method"] == "session/request_permission"
            || (message["method"] == "session/update"
                && matches!(
                    message["params"]["update"]["sessionUpdate"].as_str(),
                    Some("tool_call" | "tool_call_update")
                )))
    {
        return Ok(());
    }
    let mut encoded = serde_json::to_vec(message)
        .map_err(|_| RuntimeError::Protocol("原生工具私有诊断无法编码".into()))?;
    encoded.push(b'\n');
    if encoded.len() > 64 * 1024
        || state.private_tool_frame_count >= 64
        || state.private_tool_frame_bytes.saturating_add(encoded.len()) > 1024 * 1024
    {
        return Err(RuntimeError::Protocol(
            "原生工具私有诊断超过有界预算".into(),
        ));
    }
    let file = state.private_tool_frames.as_mut().expect("诊断文件已存在");
    file.write_all(&encoded)
        .and_then(|()| file.flush())
        .map_err(|_| RuntimeError::Protocol("原生工具私有诊断写入失败".into()))?;
    state.private_tool_frame_count += 1;
    state.private_tool_frame_bytes += encoded.len();
    Ok(())
}

fn exact_probe_input(input: &Value, final_input: bool) -> Option<CalibratedToolKind> {
    let fields = input.as_object()?;
    let expected = if final_input { 3 } else { 2 };
    if fields.len() != expected {
        return None;
    }
    if input["tool_name"] == format!("{SERVER_NAME}__inspect")
        && input["tool_input"] == json!({})
        && (!final_input || input["variant"] == "UseTool")
    {
        return Some(CalibratedToolKind::Inspect);
    }
    if input["limit"].as_u64() == Some(5)
        && input["query"] == "infinishell-sdk-origin-probe inspect"
        && (!final_input || input["variant"] == "SearchTool")
    {
        return Some(CalibratedToolKind::SearchDiscovery);
    }
    None
}

fn tool_matches_confirmed_prompt(state: &ProbeState, tool: &NativeToolObservation) -> bool {
    state.known_native_session.is_some()
        && state.acknowledged_native_prompt.is_some()
        && state.acknowledged_native_prompt == state.started_native_prompt
        && tool.session_id == state.known_native_session
        && tool.prompt_id == state.started_native_prompt
        && tool.contract_kind.is_some()
        && tool.initial_input_seen
        && tool.final_input_seen
}

fn refresh_probe_tools(state: &mut ProbeState) {
    let session = state.known_native_session.as_deref();
    let prompt = state.started_native_prompt.as_deref();
    let confirmed = session.is_some()
        && prompt.is_some()
        && state.acknowledged_native_prompt.as_deref() == prompt;
    for tool in state.tools.values_mut() {
        // 只把 use_tool inspect 账本用于 SDK 来源关联，搜索不能替代它。
        tool.probe_tool = confirmed
            && tool.contract_kind == Some(CalibratedToolKind::Inspect)
            && tool.initial_input_seen
            && tool.final_input_seen
            && tool.session_id.as_deref() == session
            && tool.prompt_id.as_deref() == prompt;
    }
}

fn native_tool_ledger_scope_verified(state: &ProbeState) -> bool {
    state.failure.is_none()
        && !state.tools.is_empty()
        && state.tools.len() <= 2
        && state.tools.values().all(|tool| {
            state.known_native_session.is_some()
                && state.started_native_prompt.is_some()
                && state.acknowledged_native_prompt == state.started_native_prompt
                && tool.initial_input_seen
                && tool.contract_kind.is_some()
                && tool.session_id == state.known_native_session
                && tool.prompt_id == state.started_native_prompt
        })
        && [
            CalibratedToolKind::Inspect,
            CalibratedToolKind::SearchDiscovery,
        ]
        .iter()
        .all(|kind| {
            state
                .tools
                .values()
                .filter(|tool| tool.contract_kind == Some(*kind))
                .count()
                <= 1
        })
}

fn native_contract_verified(state: &ProbeState) -> bool {
    native_tool_ledger_scope_verified(state)
        && state.tools.values().any(|tool| {
            tool.contract_kind == Some(CalibratedToolKind::Inspect)
                && tool_matches_confirmed_prompt(state, tool)
        })
}

fn discovery_native_contract_verified(state: &ProbeState) -> bool {
    native_tool_ledger_scope_verified(state)
        && state.tools.values().any(|tool| {
            tool.contract_kind == Some(CalibratedToolKind::SearchDiscovery)
                && tool_matches_confirmed_prompt(state, tool)
        })
}

fn exact_allow_option(options: &Value) -> bool {
    let Some(options) = options.as_array() else {
        return false;
    };
    if options.is_empty() || options.len() > 4 {
        return false;
    }
    let mut ids = HashSet::new();
    let mut allow = 0;
    for option in options {
        let Some(fields) = option.as_object() else {
            return false;
        };
        if fields.len() != 3
            || !fields.contains_key("name")
            || option["name"].as_str().is_none_or(|name| name.len() > 4096)
            || !rpc_id(&option["optionId"])
            || !option["optionId"].is_string()
            || !ids.insert(option["optionId"].as_str().expect("已验证选项 ID"))
        {
            return false;
        }
        match option["kind"].as_str() {
            Some("allow_once") if option["optionId"] == "allow-once" => allow += 1,
            // 已校准的永久许可仅可出现，响应始终只选 allow-once。
            Some("allow_always") if option["optionId"] == "always-allow" => {}
            Some("reject_once") if option["optionId"] == "reject-once" => {}
            Some(_) | None => return false,
        }
    }
    allow == 1
}

fn exact_probe_permission(
    state: &mut ProbeState,
    message: &Value,
) -> Result<Effects, RuntimeError> {
    if state.frames.len() >= MAX_RECORDS || state.permission_requests >= MAX_RECORDS {
        return Err(RuntimeError::Protocol("探针审批超过有界预算".into()));
    }
    state.permission_requests += 1;
    if !rpc_id(&message["id"]) {
        state.failure = Some("探针审批缺少有效请求 ID");
        return Err(RuntimeError::Protocol("探针审批缺少有效请求 ID".into()));
    }
    let params = &message["params"];
    let call = &params["toolCall"];
    let session_matches = params["sessionId"].as_str().is_some()
        && params["sessionId"].as_str() == state.known_native_session.as_deref();
    let prompt_matches = state.started_native_prompt.is_some()
        && state.acknowledged_native_prompt == state.started_native_prompt
        && params["_meta"]
            .get("promptId")
            .is_none_or(|id| id.as_str() == state.started_native_prompt.as_deref());
    let tool_matches = call["toolCallId"].as_str().is_some_and(|id| {
        state
            .tools
            .get(id)
            .is_some_and(|tool| tool_matches_confirmed_prompt(state, tool))
    });
    let contract_kind = exact_probe_input(&call["rawInput"], true);
    // 原生权限帧没有 promptId，只能核对已确认 Started 和同 ID 的完整前后账本。
    let verified = state.failure.is_none()
        && state.approval_prompt_open
        && native_tool_ledger_scope_verified(state)
        && session_matches
        && prompt_matches
        && tool_matches
        && message["jsonrpc"] == "2.0"
        && params["_meta"]["isReplay"] != true
        && call["_meta"]["isReplay"] != true
        && message.get("result").is_none()
        && message.get("error").is_none()
        && call["kind"] == "other"
        && contract_kind.is_some_and(|kind| {
            call["toolCallId"].as_str().is_some_and(|id| {
                state
                    .tools
                    .get(id)
                    .is_some_and(|tool| tool.contract_kind == Some(kind))
            }) && call["_meta"]["x.ai/tool"]
                .get("name")
                .is_none_or(|name| name == kind.native_name())
        })
        && exact_allow_option(&params["options"]);
    let key = message["id"].to_string();
    let fingerprint = digest(message);
    let previous = state.permission_responses.get(&key);
    let duplicate = verified && previous.is_some_and(|(hash, _)| hash == &fingerprint);
    let remaining_allowance = match contract_kind {
        Some(CalibratedToolKind::Inspect) => state.inspect_allow_count == 0,
        Some(CalibratedToolKind::SearchDiscovery) => state.search_allow_count == 0,
        None => false,
    };
    let allowed = verified && (duplicate || (previous.is_none() && remaining_allowance));
    let response = if allowed {
        if duplicate {
            state.approval_duplicate_count += 1;
            state
                .permission_responses
                .get(&key)
                .expect("重复审批缓存存在")
                .1
                .clone()
        } else {
            state.approval_allow_count += 1;
            match contract_kind.expect("许可已匹配完整原生契约") {
                CalibratedToolKind::Inspect => state.inspect_allow_count += 1,
                CalibratedToolKind::SearchDiscovery => state.search_allow_count += 1,
            }
            let response = json!({"jsonrpc":"2.0","id":message["id"],"result":{
                "outcome":{"outcome":"selected","optionId":"allow-once"}}});
            state
                .permission_responses
                .insert(key, (fingerprint, response.clone()));
            response
        }
    } else {
        state.approval_deny_count += 1;
        state.failure = Some("探针审批未匹配有界原生搜索或 inspect 契约，已拒绝");
        cancelled_permission(&message["id"]).writes.remove(0)
    };
    state.frames.push(json!({"event":"probe_approval_observed","sequence":state.frames.len()+1,
        "request_id":projected_id(&message["id"]),"session_id":projected_id(&params["sessionId"]),
        "prompt_id":projected_id(&json!(state.started_native_prompt)),
        "tool_call_id":projected_id(&call["toolCallId"]),
        "decision":if allowed {"allow_once"} else {"denied"},"duplicate":duplicate,
        "selected_option_id":if allowed {json!("allow-once")} else {Value::Null},
        "request_payload_sha256":format!("{:x}",Sha256::digest(message.to_string().as_bytes())),
        "approval_contract_verified":verified,
        "native_contract_verified":verified && contract_kind == Some(CalibratedToolKind::Inspect),
        "discovery_native_contract_verified":verified && contract_kind == Some(CalibratedToolKind::SearchDiscovery),
        "calibrated_tool_kind":contract_kind.map_or("other",CalibratedToolKind::public_kind),
        "session_id_matches":session_matches,
        "prompt_id_matches":prompt_matches,"tool_call_id_in_native_ledger":tool_matches,
        "raw_input_sha256":format!("{:x}",Sha256::digest(call["rawInput"].to_string().as_bytes()))}));
    Ok(Effects {
        writes: vec![response],
        events: Vec::new(),
    })
}

fn observe_native_tool(state: &mut ProbeState, message: &Value) -> Result<(), RuntimeError> {
    let params = &message["params"];
    let update = &params["update"];
    if message["method"] != "session/update"
        || !matches!(
            update["sessionUpdate"].as_str(),
            Some("tool_call" | "tool_call_update")
        )
        || params["_meta"]["isReplay"] == true
    {
        return Ok(());
    }
    if state.frames.len() >= MAX_RECORDS || state.tools.len() >= MAX_RECORDS {
        return Err(RuntimeError::Protocol("SDK 探针工具账本超过预算".into()));
    }
    let id = update["toolCallId"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .ok_or_else(|| RuntimeError::Protocol("原生工具缺少调用 ID".into()))?;
    let native_name = update["_meta"]["x.ai/tool"].get("name");
    let initial = update["sessionUpdate"] == "tool_call";
    let session = params["sessionId"].as_str();
    let prompt = params["_meta"]["promptId"].as_str();
    let existing = state.tools.get(id);
    let input_kind = update
        .get("rawInput")
        .and_then(|input| exact_probe_input(input, !initial));
    let contract_kind = input_kind.or_else(|| existing.and_then(|tool| tool.contract_kind));
    let identity_valid = session.is_some_and(|id| !id.is_empty() && id.len() <= 256)
        && prompt.is_some_and(|id| !id.is_empty() && id.len() <= 256)
        && state
            .known_native_session
            .as_deref()
            .is_none_or(|known| Some(known) == session)
        && state
            .started_native_prompt
            .as_deref()
            .is_none_or(|known| Some(known) == prompt)
        && existing.is_none_or(|tool| {
            tool.session_id.as_deref() == session && tool.prompt_id.as_deref() == prompt
        })
        && (existing.is_some()
            || (state.tools.len() < 2
                && contract_kind.is_some()
                && !state
                    .tools
                    .values()
                    .any(|tool| tool.contract_kind == contract_kind)));
    let input_valid = if initial {
        input_kind.is_some_and(|kind| native_name.is_some_and(|name| name == kind.native_name()))
            && existing.is_none_or(|tool| tool.contract_kind == input_kind)
            && update.get("kind").is_none_or(|kind| kind == "other")
    } else {
        existing.is_some_and(|tool| tool.initial_input_seen)
            && contract_kind
                .is_some_and(|kind| native_name.is_none_or(|name| name == kind.native_name()))
            && existing.is_some_and(|tool| tool.contract_kind == contract_kind)
            && update
                .get("rawInput")
                .is_none_or(|_| input_kind.is_some() && update["kind"] == "other")
    };
    if !identity_valid || !input_valid {
        state.unexpected_tools += 1;
        state.failure = Some("原生工具未匹配完整探针输入与来源账本");
    }
    let tool = state.tools.entry(id.to_owned()).or_default();
    if identity_valid && input_valid {
        tool.session_id = session.map(str::to_owned);
        tool.prompt_id = prompt.map(str::to_owned);
        tool.contract_kind = contract_kind;
        tool.initial_input_seen |= initial;
        tool.final_input_seen |= !initial && update.get("rawInput").is_some();
    }
    if update["status"] == "failed" {
        state.failure = Some("原生探针工具报告失败");
    }
    tool.completed |= identity_valid && input_valid && update["status"] == "completed";
    let status = match update["status"].as_str() {
        Some("pending") => "pending",
        Some("in_progress") => "in_progress",
        Some("completed") => "completed",
        Some("failed") => "failed",
        Some(_) | None => "unknown_or_absent",
    };
    refresh_probe_tools(state);
    state.frames.push(json!({"event":"native_tool_update_received","sequence":state.frames.len()+1,
        "session_id":projected_id(&params["sessionId"]),"prompt_id":projected_id(&params["_meta"]["promptId"]),
        "event_id":projected_id(&params["_meta"]["eventId"]),"tool_call_id":projected_id(&update["toolCallId"]),
        "native_tool_name_type":native_name.map_or("absent",value_type),
        "native_tool_name_sha256":native_name.map(|name|format!("{:x}",Sha256::digest(name.to_string().as_bytes()))),
        "native_tool_kind":native_tool_kind(native_name.unwrap_or(&Value::Null)),
        "status":status,"probe_tool":state.tools[id].probe_tool,
        "discovery_tool":state.tools[id].contract_kind == Some(CalibratedToolKind::SearchDiscovery)
            && tool_matches_confirmed_prompt(state,&state.tools[id]),
        "calibrated_tool_kind":state.tools[id].contract_kind.map_or("other",CalibratedToolKind::public_kind),
        "raw_input_present":update.get("rawInput").is_some(),
        "raw_input_sha256":update.get("rawInput").map(|input|format!("{:x}",Sha256::digest(input.to_string().as_bytes())))}));
    Ok(())
}

fn record(file: &mut File, value: &Value) -> Result<(), String> {
    writeln!(file, "{value}").map_err(|_| "写入探针证据失败".to_owned())?;
    file.flush().map_err(|_| "刷新探针证据失败".to_owned())
}

async fn exercise(root: &Path, file: &mut File) -> Result<(), String> {
    let generation = Uuid::new_v4();
    let probe = SdkOriginProbe::new(generation);
    let mut private_options = OpenOptions::new();
    private_options.write(true).create_new(true);
    #[cfg(unix)]
    private_options.mode(0o600);
    probe
        .state
        .lock()
        .expect("探针状态锁未被破坏")
        .private_tool_frames = Some(
        private_options
            .open(root.join("private-native-tool-frames.ndjson"))
            .map_err(|_| "创建原生工具私有诊断失败")?,
    );
    let state_dir = root.join("state");
    let (controller, commands, sender, mut events) = channels(generation);
    let final_histories = Arc::new(Mutex::new(HashMap::new()));
    let mut protocol = GrokProtocol::new(SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("缺少固定 CLI 入口")?,
        ),
        cwd: root
            .join("project")
            .canonicalize()
            .map_err(|_| "缺少空项目目录")?,
        state_dir: state_dir.clone(),
        target: SessionTarget::New,
        generation,
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    });
    protocol.sdk_origin_probe = Some(probe.clone());
    protocol.verified_final_histories_for_live = Some(final_histories.clone());
    let mut task = tokio::spawn(async move { run_process(&mut protocol, commands, &sender).await });
    let message_id = Uuid::new_v4();
    let mut session_id = None;
    let mut prompt_id = None;
    let mut submitted_inputs = 0;
    let mut started = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    let result: Result<(), String> = async {
        loop {
            probe.check()?;
            let event = tokio::time::timeout_at(deadline, async {
                tokio::select! {
                    event = events.recv() => Ok(event),
                    () = probe.changed.notified() => Err(()),
                }
            })
            .await
            .map_err(|_| "探针总时限耗尽")?;
            let Ok(event) = event else {
                continue;
            };
            let event = event.ok_or("原生事件流已关闭")?;
            if event.generation != generation {
                return Err("探针收到过时连接事件".into());
            }
            if let Some(id) = &event.native_session_id {
                if Uuid::parse_str(id).is_err()
                    || session_id.as_ref().is_some_and(|previous| previous != id)
                {
                    return Err("原生会话身份无法确认".into());
                }
                session_id = Some(id.clone());
                probe.confirm_runtime_session(id)?;
            }
            match event.kind {
                RuntimeEventKind::SessionReady {
                    effective_permissions,
                } => {
                    if submitted_inputs != 0 || session_id.is_none() {
                        return Err("探针初始化重复或没有会话身份".into());
                    }
                    for capability in ["steer", "localTools", "childTasks"] {
                        if effective_permissions["verifiedCapabilities"][capability] != false {
                            return Err("探针不得打开 SDK、子任务或同轮追加能力门禁".into());
                        }
                    }
                    submitted_inputs += 1;
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id,
                            action: RuntimeAction::Submit {
                                input: vec![InputContent::Text(PROMPT.into())],
                            },
                        })
                        .await
                        .map_err(|_| "探针输入发送失败")?;
                }
                RuntimeEventKind::MessageAccepted {
                    message_id: received,
                    turn_id,
                } => {
                    if received != message_id
                        || prompt_id.is_some()
                        || turn_id
                            .as_deref()
                            .is_none_or(|id| Uuid::parse_str(id).is_err())
                    {
                        return Err("探针原生接收身份无效或重复".into());
                    }
                    probe.confirm_runtime_ack(turn_id.as_deref().expect("已验证真实 ACK 回合"))?;
                    prompt_id = turn_id;
                }
                RuntimeEventKind::TurnStarted { turn_id } => {
                    if Some(turn_id.as_str()) != prompt_id.as_deref() || started {
                        return Err("探针原生运行身份无效或重复".into());
                    }
                    probe.confirm_runtime_started(&turn_id)?;
                    started = true;
                }
                RuntimeEventKind::TurnFinished {
                    turn_id,
                    outcome,
                    output,
                } => {
                    let histories = final_histories.lock().expect("原生最终回放锁未被破坏");
                    let snapshot = histories
                        .get(&turn_id)
                        .ok_or("探针没有已验证的原生最终回放")?;
                    let receipt = super::live_tests::final_response_receipt(
                        snapshot,
                        session_id.as_deref().ok_or("探针没有会话身份")?,
                        &turn_id,
                        &outcome,
                    )?;
                    if Some(turn_id.as_str()) != prompt_id.as_deref()
                        || !started
                        || outcome != TurnOutcome::Completed
                        || receipt.final_response.trim() != REPLY
                        || receipt.full_output != output
                    {
                        return Err("探针没有取得真实完整回执".into());
                    }
                    return Ok(());
                }
                RuntimeEventKind::TextDelta { turn_id, .. }
                | RuntimeEventKind::Progress { turn_id, .. } => {
                    if Some(turn_id.as_str()) != prompt_id.as_deref() {
                        return Err("探针收到旧回合输出或进度".into());
                    }
                }
                RuntimeEventKind::RequestFailed { .. }
                | RuntimeEventKind::Disconnected { .. }
                | RuntimeEventKind::ApprovalRequested { .. }
                | RuntimeEventKind::ApprovalResolved { .. }
                | RuntimeEventKind::ApprovalCancelled { .. }
                | RuntimeEventKind::LocalToolRequested { .. }
                | RuntimeEventKind::LocalToolCancelled { .. }
                | RuntimeEventKind::InputJoined { .. }
                | RuntimeEventKind::CommandDispatched { .. } => {
                    return Err("探针遇到未允许的运行事件".into());
                }
            }
        }
    }
    .await;
    probe.close_approval_prompt();
    // 任一路径都先关闭生产连接并核对退出回执，再写最终探针结果。
    let _ = controller
        .send(RuntimeCommand {
            generation,
            message_id: Uuid::new_v4(),
            action: RuntimeAction::Shutdown,
        })
        .await;
    let joined = tokio::time::timeout(Duration::from_secs(30), &mut task).await;
    let transport_ok = matches!(&joined, Ok(Ok(Ok(()))));
    if joined.is_err() {
        task.abort();
    }
    let receipt = managed_process::confirmed_exit(&state_dir, generation);
    let cleanup_confirmed = receipt
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
        .is_some_and(|receipt| receipt.cleanup_confirmed);
    let mut report = probe.report(session_id.as_deref(), prompt_id.as_deref());
    for frame in report["frames"].as_array().expect("探针帧列表存在") {
        record(file, frame)?;
    }
    report
        .as_object_mut()
        .expect("探针报告为对象")
        .remove("frames");
    let no_project_files = fs::read_dir(root.join("project"))
        .ok()
        .is_some_and(|mut entries| entries.next().is_none());
    let native_inputs = usize::from(prompt_id.is_some());
    let passed = result.is_ok()
        && probe.check().is_ok()
        && transport_ok
        && cleanup_confirmed
        && native_inputs == 1
        && submitted_inputs == 1
        && ((report["initialization_count"] == 1 && report["discovery_count"] == 0)
            || (report["initialization_count"] == 0
                && report["discovery_count"] == 1
                && report["negotiatedProtocolVersion"] == MODERN_MCP_VERSION))
        && report["tools_list_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
        && report["inspect_call_count"] == 1
        && report["unexpected_tool_count"] == 0
        && report["native_contract_verified"] == true
        && report["full_native_origin_fields_observed"] == true
        && report["native_origin_ledger_relation_verified"] == true
        && report["approval_allow_count"]
            .as_u64()
            .is_some_and(|count| count <= 2)
        && report["search_allow_count"]
            .as_u64()
            .is_some_and(|count| count <= 1)
        && report["inspect_allow_count"]
            .as_u64()
            .is_some_and(|count| count <= 1)
        && report["approval_deny_count"] == 0
        && no_project_files;
    report["event"] = json!("probe_finished");
    report["scope"] = json!(SCOPE);
    report["passed"] = json!(passed);
    report["native_inputs"] = json!(native_inputs);
    report["submitted_input_count"] = json!(submitted_inputs);
    report["max_native_inputs"] = json!(1);
    report["native_session_id"] = json!(session_id);
    report["native_prompt_id"] = json!(prompt_id);
    report["no_side_effects"] = json!(passed && no_project_files);
    report["no_project_files"] = json!(no_project_files);

    report["cleanup_confirmed"] = json!(cleanup_confirmed);
    report["transport_closed"] = json!(transport_ok);
    report["cleanup_receipt_read"] = json!(receipt.is_ok());
    report["public_product_gate_open"] = json!(false);
    report["parent_permission_ceiling_verified"] = json!(false);
    report["credential_files_read_by_probe"] = json!(false);
    if let Err(reason) = &result {
        report["failure_reason"] = json!(reason);
    }
    record(file, &report)?;
    if passed {
        Ok(())
    } else {
        Err("SDK 探针未通过；来源缺失本身不伪造为完整能力通过".into())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由独立隔离运行器启动；仅一轮真实 SDK 探针，会消耗已授权模型额度"]
async fn native_sdk_registration_and_origin_probe() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-live-probe")).unwrap(),
        "isolated Grok Rust adapter verification\n"
    );
    assert_eq!(
        PathBuf::from(env::var_os("GROK_HOME").expect("缺少隔离 HOME"))
            .canonicalize()
            .unwrap(),
        root.join("home/.grok")
    );
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_AUTH_MODE").unwrap(),
        "official-cached-token"
    );
    for name in ["XAI_API_KEY", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
        assert!(env::var_os(name).is_none());
    }
    assert!(
        fs::read_dir(root.join("project")).unwrap().next().is_none(),
        "探针只允许空项目"
    );
    let mut file = File::create(PathBuf::from(
        env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").expect("缺少探针证据路径"),
    ))
    .unwrap();
    record(&mut file,&json!({"event":"probe_started","scope":SCOPE,"max_native_inputs":1,
        "origin_verification":"unknown","credential_files_read_by_probe":false,"public_product_gate_open":false,
        "production_run_process":true,"production_run_transport":true,"test_only_sdk_hook":true})).unwrap();
    assert!(
        exercise(&root, &mut file).await.is_ok(),
        "原生 SDK 注册探针未通过；请检查安全投影证据"
    );
}

#[test]
fn missing_sdk_origin_stays_missing_with_a_single_native_tool_candidate() {
    let generation = Uuid::new_v4();
    let mut probe = SdkOriginProbe::new(generation);
    let session = Uuid::new_v4().to_string();
    let prompt = Uuid::new_v4().to_string();
    confirm_probe_prompt(&probe, &session, &prompt);
    install_exact_tool_ledger(&mut probe, &session, &prompt, "native-tool-1");
    probe.receive(&json!({"jsonrpc":"2.0","id":9,"method":"x.ai/mcp/sdk_call","params":{"serverId":probe.server_id,
        "message":{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect","arguments":{}}}}})).unwrap();
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["origin_verification"], "candidate_mapping_only");
    assert_eq!(report["full_native_origin_fields_observed"], false);
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
}

#[test]
fn sdk_probe_rejects_message_tools_without_calling_a_coordinator() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let response=probe.receive(&json!({"jsonrpc":"2.0","id":8,"method":"x.ai/mcp/sdk_call","params":{"serverId":probe.server_id,
        "message":{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send_message_to_local_agent","arguments":{}}}}})).unwrap().unwrap();
    assert_eq!(response.writes[0]["result"]["error"]["code"], -32601);
    assert!(probe.check().is_err());
    assert_eq!(probe.report(None, None)["inspect_call_count"], 0);
}

#[test]
fn complete_sdk_origin_requires_matching_final_native_tool_ledger() {
    // 合成帧只校验来源分类逻辑，不计作官方 CLI 的真实 SDK 证据。
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let session = Uuid::new_v4().to_string();
    let prompt = Uuid::new_v4().to_string();
    let tool = "native-tool-1";
    confirm_probe_prompt(&probe, &session, &prompt);
    install_exact_tool_ledger(&mut probe, &session, &prompt, tool);
    probe.receive(&json!({"jsonrpc":"2.0","id":9,"method":"x.ai/mcp/sdk_call","params":{
        "serverId":probe.server_id,"sessionId":session,"promptId":prompt,"toolCallId":tool,
        "message":{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect","arguments":{}}}}})).unwrap();
    assert_eq!(
        probe.report(Some(&session), Some(&prompt))["native_origin_ledger_relation_verified"],
        false
    );
    probe
        .receive(
            &json!({"method":"session/update","params":{"sessionId":session,
        "_meta":{"promptId":prompt},"update":{"sessionUpdate":"tool_call_update","toolCallId":tool,
        "status":"completed"}}}),
        )
        .unwrap();
    assert_eq!(
        probe.report(Some(&session), Some(&prompt))["native_origin_ledger_relation_verified"],
        true
    );
    probe.state.lock().unwrap().origins[0].push(OriginObservation {
        carrier: "sdk_params_meta",
        fields: json!({"sessionId":"string","promptId":"string","toolCallId":"string"}),
        session_id: Some(session.clone()),
        prompt_id: Some(Uuid::new_v4().to_string()),
        tool_call_id: Some(tool.into()),
    });
    assert_eq!(
        probe.report(Some(&session), Some(&prompt))["native_origin_ledger_relation_verified"],
        false
    );
}

#[test]
fn repeated_mcp_request_does_not_add_another_inspect_invocation() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let mut message = json!({"jsonrpc":"2.0","id":7,"method":"x.ai/mcp/sdk_call","params":{"serverId":probe.server_id,
        "message":{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect","arguments":{}}}}});
    let first = probe.receive(&message).unwrap().unwrap();
    message["id"] = json!(8);
    let repeated = probe.receive(&message).unwrap().unwrap();
    assert_eq!(first.writes[0]["result"], repeated.writes[0]["result"]);
    let report = probe.report(None, None);
    assert_eq!(report["inspect_call_count"], 1);
    assert_eq!(report["duplicate_request_count"], 1);
    assert!(probe.check().is_ok());
}

#[test]
fn unsupported_sdk_version_is_observed_without_exposing_arbitrary_values() {
    for version in ["2024-11-05", "dummy-private-version-value"] {
        let mut probe = SdkOriginProbe::new(Uuid::new_v4());
        let response = probe
            .receive(&json!({"jsonrpc":"2.0","id":7,"method":"x.ai/mcp/sdk_call",
                "params":{"serverId":probe.server_id,"message":{"jsonrpc":"2.0","id":3,
                    "method":"initialize","params":{"protocolVersion":version}}}}))
            .unwrap()
            .unwrap();
        assert_eq!(response.writes[0]["result"]["error"]["code"], -32601);
        assert!(probe.check().is_err());
        let report = probe.report(None, None);
        assert_eq!(report["initialization_count"], 0);
        assert_eq!(
            report["frames"][0]["requested_protocol_version"],
            Value::Null
        );
        assert!(!report.to_string().contains("dummy-private-version-value"));
    }
}

#[test]
fn sdk_probe_matches_only_two_explicit_wire_methods_and_records_the_actual_method() {
    for wire in ["x.ai/mcp/sdk_call", "_x.ai/mcp/sdk_call"] {
        let mut probe = SdkOriginProbe::new(Uuid::new_v4());
        let frame = json!({"jsonrpc":"2.0", "id":"outer", "method":wire,
            "params":{"serverId":probe.server_id, "message":{"jsonrpc":"2.0", "id":1,
                "method":"initialize", "params":{"protocolVersion":MCP_VERSION}}}});
        assert!(probe.receive(&frame).unwrap().is_some());
        let state = probe.state.lock().unwrap();
        assert_eq!(state.initializations, 1);
        assert_eq!(state.frames[0]["wire_method"], wire);
    }
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    assert!(
        probe
            .receive(&json!({"jsonrpc":"2.0", "id":1, "method":"__x.ai/mcp/sdk_call", "params":{}}))
            .unwrap()
            .is_none()
    );
    assert_eq!(probe.state.lock().unwrap().sdk_requests, 0);
}

#[test]
fn sdk_initialize_observation_records_only_presence_type_and_boolean_enablement() {
    for value in [
        Value::Null,
        json!(false),
        json!(true),
        json!("do-not-record-this-value"),
    ] {
        let mut probe = SdkOriginProbe::new(Uuid::new_v4());
        let frame = json!({"jsonrpc":"2.0", "id":1, "result":{"protocolVersion":1,
            "_meta":{"agentVersion":super::VERIFIED_VERSION, "x.ai/mcp/sdk":value}}});
        probe.receive(&frame).unwrap();
        probe.receive(&frame).unwrap();
        let state = probe.state.lock().unwrap();
        assert_eq!(state.frames.len(), 1);
        assert_eq!(state.frames[0]["sdk_capability_present"], true);
        assert_eq!(state.frames[0]["sdk_capability_type"], value_type(&value));
        assert_eq!(state.frames[0]["sdk_capability_enabled"], value == true);
        assert!(
            !state.frames[0]
                .to_string()
                .contains("do-not-record-this-value")
        );
    }
}

#[test]
fn mcp_status_session_match_requires_a_confirmed_present_native_session() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    probe
        .receive(&json!({"method":"_x.ai/mcp/init_progress","params":{}}))
        .unwrap();
    let first = probe.report(None, None)["frames"][0].clone();
    assert_eq!(first["session_known"], false);
    assert_eq!(first["session_id_matches"], false);
    let session = Uuid::new_v4().to_string();
    probe.confirm_runtime_session(&session).unwrap();
    assert!(
        probe
            .confirm_runtime_session(&Uuid::new_v4().to_string())
            .is_err()
    );
    assert!(
        probe
            .confirm_runtime_session("Bearer OFFLINE_SESSION_CANARY")
            .is_err()
    );
    probe
        .receive(&json!({"method":"_x.ai/mcp/init_progress","params":{}}))
        .unwrap();
    probe.receive(&json!({"method":"_x.ai/mcp/init_progress","params":{"sessionId":Uuid::new_v4().to_string()}})).unwrap();
    probe.receive(&json!({"method":"_x.ai/mcp/init_progress","params":{"sessionId":session,"total":1,"connected":0}})).unwrap();
    let report = probe.report(None, None);
    assert_eq!(report["frames"][1]["session_known"], true);
    assert_eq!(report["frames"][1]["session_id_matches"], false);
    assert_eq!(report["frames"][2]["session_id_matches"], false);
    assert_eq!(report["frames"][3]["session_id_matches"], true);
    assert_eq!(report["origin_verification"], "unknown");
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
}

#[test]
fn mcp_progress_counters_are_finite_integers_and_repeat_notifications_are_deduplicated() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let frame = json!({"method":"_x.ai/mcp/init_progress","params":{"total":1,"connected":true}});
    for _ in 0..MAX_RECORDS * 2 {
        probe.receive(&frame).unwrap();
    }
    probe.receive(&json!({"method":"_x.ai/mcp_initialized","params":{"mcpToolCount":-1,"elapsedMs":u64::MAX}})).unwrap();
    let report = probe.report(None, None);
    assert_eq!(report["mcp_status_observation_count"], 2);
    assert_eq!(report["frames"][0]["total"], 1);
    assert_eq!(report["frames"][0]["connected"], Value::Null);
    assert_eq!(report["frames"][0]["connected_type"], "boolean");
    assert_eq!(report["frames"][1]["mcp_tool_count"], Value::Null);
    assert_eq!(report["frames"][1]["elapsed_ms"], Value::Null);
    for value in [
        json!(true),
        json!(-1),
        json!(1.5),
        json!(u64::MAX),
        json!("OFFLINE_COUNTER_CANARY"),
    ] {
        assert_eq!(finite_counter(&value), Value::Null);
    }
    assert_eq!(finite_counter(&json!(i64::MAX)), json!(i64::MAX));
}

#[test]
fn mcp_server_status_omits_names_details_and_unknown_marker_contents() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let session = Uuid::new_v4().to_string();
    probe.confirm_runtime_session(&session).unwrap();
    let frame = json!({"method":"_x.ai/mcp/server_status","params":{"sessionId":session,
        "name":"OFFLINE_SERVER_CANARY","status":"Bearer OFFLINE_STATUS_CANARY",
        "reason":{"token":"OFFLINE_REASON_CANARY"},"detail":"Bearer OFFLINE_DETAIL_CANARY",
        "serverId":"OFFLINE_FABRICATED_SERVER_ID_CANARY"}});
    probe.receive(&frame).unwrap();
    probe.receive(&frame).unwrap();
    let report = probe.report(None, None);
    assert_eq!(report["mcp_status_observation_count"], 1);
    let observed = &report["frames"][0];
    assert_eq!(observed["session_id_matches"], true);
    assert_eq!(observed["probe_server_name_matches"], false);
    assert_eq!(observed["mcp_status"]["type"], "string");
    assert_eq!(observed["mcp_reason"]["type"], "object");
    assert_eq!(observed["detail_present"], true);
    assert_eq!(observed["detail_type"], "string");
    assert_eq!(observed["detail_sha256"].as_str().unwrap().len(), 64);
    assert!(observed.get("serverId").is_none());
    assert!(!observed.to_string().contains("CANARY"));
}

#[test]
fn ready_mcp_server_status_does_not_verify_sdk_origin_or_registration() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let session = Uuid::new_v4().to_string();
    probe.confirm_runtime_session(&session).unwrap();
    probe
        .receive(
            &json!({"method":"_x.ai/mcp/server_status","params":{"sessionId":session,
        "name":SERVER_NAME,"status":"ready","reason":"initialized"}}),
        )
        .unwrap();
    let report = probe.report(Some(&session), None);
    assert_eq!(report["frames"][0]["probe_server_name_matches"], true);
    assert_eq!(report["frames"][0]["mcp_status"], "ready");
    assert_eq!(report["frames"][0]["mcp_reason"], "initialized");
    assert_eq!(report["initialization_count"], 0);
    assert_eq!(report["origin_verification"], "unknown");
    assert_eq!(report["full_native_origin_fields_observed"], false);
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
}

#[test]
fn mcp_status_observations_are_bounded_and_only_match_three_exact_native_methods() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    for method in [
        "x.ai/mcp/init_progress",
        "__x.ai/mcp/init_progress",
        "_x.ai/mcp/server_status/extra",
    ] {
        probe
            .receive(&json!({"method":method,"params":{}}))
            .unwrap();
    }
    assert_eq!(probe.report(None, None)["mcp_status_observation_count"], 0);
    for total in 0..MAX_RECORDS {
        probe
            .receive(&json!({"method":"_x.ai/mcp/init_progress","params":{"total":total}}))
            .unwrap();
    }
    assert!(
        probe
            .receive(&json!({"method":"_x.ai/mcp/init_progress","params":{"total":MAX_RECORDS}}))
            .is_err()
    );
    assert_eq!(
        probe.report(None, None)["mcp_status_observation_count"],
        MAX_RECORDS
    );
    assert!(probe.check().is_err());
}

#[test]
fn server_discover_without_standard_metadata_is_invalid_params() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let response = probe
        .receive(
            &json!({"jsonrpc":"2.0","id":7,"method":"_x.ai/mcp/sdk_call",
        "params":{"serverId":probe.server_id,"message":{"jsonrpc":"2.0","id":3,
            "method":"server/discover","params":{"protocolVersion":"2026-07-28"}}}}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(response.writes[0]["result"]["error"]["code"], -32602);
    let report = probe.report(None, None);
    assert_eq!(report["frames"][0]["mcp_method"], "server/discover");
    assert_eq!(report["initialization_count"], 0);
    assert_eq!(report["inspect_call_count"], 0);
    assert_eq!(report["origin_verification"], "unknown");
    assert!(probe.check().is_err());
}

fn modern_probe_params() -> Value {
    json!({"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientCapabilities":{},
        "io.modelcontextprotocol/clientInfo":{"name":"public-conformance-fixture","version":"1.0.0"}}})
}

fn modern_probe_request(probe: &SdkOriginProbe, id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"_x.ai/mcp/sdk_call","params":{
        "serverId":probe.server_id,"message":{"jsonrpc":"2.0","id":id,"method":method,"params":params}}})
}

#[test]
fn modern_discovery_and_tool_results_follow_schema_without_claiming_native_origin() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let discovery = modern_probe_request(&probe, 0, "server/discover", modern_probe_params());
    let response = probe.receive(&discovery).unwrap().unwrap();
    assert_eq!(
        response.writes[0]["result"]["result"],
        json!({
        "resultType":"complete","supportedVersions":["2026-07-28"],
        "capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private",
        "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"infinishell-sdk-origin-probe","version":"0.1.0"}}})
    );

    let list = modern_probe_request(&probe, 1, "tools/list", modern_probe_params());
    let list_response = probe.receive(&list).unwrap().unwrap();
    assert_eq!(
        list_response.writes[0]["result"]["result"]["resultType"],
        "complete"
    );
    assert_eq!(list_response.writes[0]["result"]["result"]["ttlMs"], 0);
    assert_eq!(
        list_response.writes[0]["result"]["result"]["cacheScope"],
        "private"
    );
    assert_eq!(
        list_response.writes[0]["result"]["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let mut params = modern_probe_params();
    params["name"] = json!("inspect");
    params["arguments"] = json!({});
    let inspect = modern_probe_request(&probe, 2, "tools/call", params);
    let inspect_response = probe.receive(&inspect).unwrap().unwrap();
    assert_eq!(
        inspect_response.writes[0]["result"]["result"]["resultType"],
        "complete"
    );
    assert_eq!(
        inspect_response.writes[0]["result"]["result"]["isError"],
        false
    );

    let report = probe.report(None, None);
    assert_eq!(report["initialization_count"], 0);
    assert_eq!(report["discovery_count"], 1);
    assert_eq!(report["tools_list_count"], 1);
    assert_eq!(report["inspect_call_count"], 1);
    assert_eq!(report["negotiatedProtocolVersion"], "2026-07-28");
    assert_eq!(report["servedToolNames"], json!(["inspect"]));
    assert_eq!(
        report["frames"][0]["protocol_version_carrier"],
        "params._meta"
    );
    assert_eq!(report["frames"][0]["metadata_schema_valid"], true);
    assert_eq!(report["origin_verification"], "missing_native_origin");
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
}

#[test]
fn modern_discovery_duplicate_is_cached_without_another_registration() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let mut message = modern_probe_request(&probe, 0, "server/discover", modern_probe_params());
    let first = probe.receive(&message).unwrap().unwrap();
    message["id"] = json!(1);

    let replay = probe.receive(&message).unwrap().unwrap();

    assert_eq!(first.writes[0]["result"], replay.writes[0]["result"]);
    assert_eq!(probe.report(None, None)["discovery_count"], 1);
    assert_eq!(probe.report(None, None)["duplicate_request_count"], 1);
}

#[test]
fn modern_discovery_conflicting_inner_id_is_rejected() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let mut message = modern_probe_request(&probe, 0, "server/discover", modern_probe_params());
    probe.receive(&message).unwrap();
    message["id"] = json!(1);
    message["params"]["message"]["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"] =
        json!({"roots":{}});

    assert!(probe.receive(&message).is_err());
    assert_eq!(probe.report(None, None)["discovery_count"], 1);
}

#[test]
fn modern_unknown_version_does_not_guess_sdk7_metadata_values() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let mut params = modern_probe_params();
    params["_meta"][PROTOCOL_VERSION_META] = json!("2030-01-01");
    let message = modern_probe_request(&probe, 0, "server/discover", params);

    let response = probe.receive(&message).unwrap().unwrap();

    assert_eq!(response.writes[0]["result"]["error"]["code"], -32022);
    assert_eq!(probe.report(None, None)["discovery_count"], 0);
    assert_eq!(
        probe.report(None, None)["negotiatedProtocolVersion"],
        Value::Null
    );
    assert_eq!(
        probe.report(None, None)["frames"][0]["requested_protocol_version"],
        Value::Null
    );
    assert!(probe.check().is_err());
}

#[test]
fn modern_requests_cannot_inherit_missing_metadata_from_discovery() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let discovery = modern_probe_request(&probe, 0, "server/discover", modern_probe_params());
    probe.receive(&discovery).unwrap();
    let list = modern_probe_request(&probe, 1, "tools/list", json!({}));

    let response = probe.receive(&list).unwrap().unwrap();

    assert_eq!(response.writes[0]["result"]["error"]["code"], -32602);
    assert_eq!(probe.report(None, None)["tools_list_count"], 0);
    assert!(probe.check().is_err());
}

#[test]
fn modern_discovery_rejects_malformed_client_identity() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let mut params = modern_probe_params();
    params["_meta"]["io.modelcontextprotocol/clientInfo"]["name"] = json!(1);
    let discovery = modern_probe_request(&probe, 0, "server/discover", params);

    let response = probe.receive(&discovery).unwrap().unwrap();

    assert_eq!(response.writes[0]["result"]["error"]["code"], -32602);
    assert_eq!(probe.report(None, None)["discovery_count"], 0);
}

#[test]
fn modern_probe_tool_input_rejects_extra_body_fields() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let mut params = modern_probe_params();
    params["name"] = json!("inspect");
    params["arguments"] = json!({});
    params["origin"] = json!("forged");
    let inspect = modern_probe_request(&probe, 0, "tools/call", params);

    let response = probe.receive(&inspect).unwrap().unwrap();

    assert_eq!(response.writes[0]["result"]["error"]["code"], -32601);
    assert_eq!(probe.report(None, None)["inspect_call_count"], 0);
}

#[test]
fn modern_probe_has_one_tool_list_and_rejects_legacy_ping() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let list = modern_probe_request(&probe, 0, "tools/list", modern_probe_params());
    probe.receive(&list).unwrap();
    let second_list = modern_probe_request(&probe, 1, "tools/list", modern_probe_params());
    let response = probe.receive(&second_list).unwrap().unwrap();

    assert_eq!(response.writes[0]["result"]["error"]["code"], -32601);
    assert_eq!(probe.report(None, None)["tools_list_count"], 1);
    let ping = modern_probe_request(&probe, 2, "ping", modern_probe_params());
    let response = probe.receive(&ping).unwrap().unwrap();
    assert_eq!(response.writes[0]["result"]["error"]["code"], -32601);
}

#[test]
fn inline_modern_probe_cannot_downgrade_when_version_is_missing() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let params = json!({"name":"inspect","arguments":{},
        "_meta":{"io.modelcontextprotocol/clientCapabilities":{}}});
    let inspect = modern_probe_request(&probe, 0, "tools/call", params);

    let response = probe.receive(&inspect).unwrap().unwrap();

    assert_eq!(response.writes[0]["result"]["error"]["code"], -32602);
    assert_eq!(probe.report(None, None)["inspect_call_count"], 0);
}

#[test]
fn modern_probe_requires_capabilities_on_each_request() {
    let mut probe = SdkOriginProbe::new(Uuid::nil());
    let params = json!({"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}});
    let discovery = modern_probe_request(&probe, 0, "server/discover", params);

    let response = probe.receive(&discovery).unwrap().unwrap();

    assert_eq!(response.writes[0]["result"]["error"]["code"], -32602);
    assert_eq!(probe.report(None, None)["discovery_count"], 0);
}

#[test]
fn private_native_tool_diagnostics_ignore_model_text_and_other_non_tool_frames() {
    let private = NamedTempFile::new().unwrap();
    let mut state = ProbeState::default();
    state.private_tool_frames = Some(private.as_file().try_clone().unwrap());
    for frame in [
        json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk",
            "content":{"type":"text","text":"OFFLINE_MODEL_BODY_CANARY"}}}}),
        json!({"method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk",
            "content":{"type":"text","text":"OFFLINE_THOUGHT_BODY_CANARY"}}}}),
        json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call/extra",
            "rawInput":{"payload":"OFFLINE_FALSE_TOOL_CANARY"}}}}),
        json!({"method":"session/request_permission/extra","params":{"payload":"OFFLINE_FALSE_PERMISSION_CANARY"}}),
        json!({"method":"_x.ai/mcp/server_status","params":{"detail":"OFFLINE_STATUS_BODY_CANARY"}}),
        json!({"method":"x.ai/mcp/sdk_call","params":{"message":{"method":"tools/call",
            "params":{"arguments":{"payload":"OFFLINE_SDK_BODY_CANARY"}}}}}),
        json!({"id":1,"result":{"protocolVersion":1}}),
    ] {
        record_private_native_tool_frame(&mut state, &frame).unwrap();
    }
    assert_eq!(state.private_tool_frame_count, 0);
    assert_eq!(state.private_tool_frame_bytes, 0);
    assert_eq!(private.as_file().metadata().unwrap().len(), 0);
    assert!(fs::read(private.path()).unwrap().is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            private.as_file().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn private_native_tool_and_approval_frames_stay_out_of_public_probe_reports() {
    let private = NamedTempFile::new().unwrap();
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    probe.state.lock().unwrap().private_tool_frames = Some(private.as_file().try_clone().unwrap());
    let session = Uuid::new_v4().to_string();
    let prompt = Uuid::new_v4().to_string();
    let diagnostic_path = private.path().to_string_lossy().into_owned();
    let raw_name = "OFFLINE_PRIVATE_RAW_TOOL_NAME_CANARY";
    let raw_input = "OFFLINE_PRIVATE_RAW_TOOL_INPUT_CANARY";
    let filepath = "/private/offline/native-tool-filepath-canary";
    let frames = [
        json!({"method":"session/update","params":{"sessionId":session,"_meta":{"promptId":prompt},
            "update":{"sessionUpdate":"tool_call","toolCallId":"offline-private-tool",
                "title":"OFFLINE_PRIVATE_TOOL_TITLE_CANARY","rawInput":{"payload":raw_input,"path":filepath},
                "_meta":{"x.ai/tool":{"name":raw_name},"privateDiagnosticPath":diagnostic_path}}}}),
        json!({"method":"session/update","params":{"sessionId":session,"_meta":{"promptId":prompt},
            "update":{"sessionUpdate":"tool_call_update","toolCallId":"offline-private-tool","status":"completed",
                "rawInput":{"payload":raw_input,"path":filepath},"_meta":{"x.ai/tool":{"name":raw_name}}}}}),
        json!({"id":7,"method":"session/request_permission","params":{"sessionId":session,
            "toolCall":{"toolCallId":"offline-private-tool","title":raw_name,
                "rawInput":{"payload":raw_input,"path":filepath},"privateDiagnosticPath":diagnostic_path}}}),
    ];
    for frame in &frames[..2] {
        assert!(probe.receive(frame).unwrap().is_none());
    }
    let denied = probe.receive(&frames[2]).unwrap().unwrap();
    assert_eq!(denied.writes.len(), 1);
    assert_eq!(denied.writes[0]["id"], 7);
    assert_eq!(
        denied.writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    assert!(probe.check().is_err());
    let bytes = fs::read(private.path()).unwrap();
    let actual = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    // 私有文件必须保存完整原生帧；公开报告只使用既有安全投影。
    assert!(actual.as_slice() == frames.as_slice());
    let private_text = std::str::from_utf8(&bytes).unwrap();
    for canary in [raw_name, raw_input, filepath, diagnostic_path.as_str()] {
        assert!(private_text.contains(canary));
    }
    let state = probe.state.lock().unwrap();
    assert_eq!(state.private_tool_frame_count, 3);
    assert_eq!(state.private_tool_frame_bytes, bytes.len());
    drop(state);
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["permission_request_count"], 1);
    assert_eq!(report["unexpected_tool_count"], 2);
    assert_eq!(report["sdk_request_count"], 0);
    assert_eq!(report["inspect_call_count"], 0);
    assert_eq!(report["full_native_origin_fields_observed"], false);
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
    assert_eq!(report["origin_verification"], "unknown");
    let public = report.to_string();
    for canary in [
        "CANARY",
        raw_name,
        raw_input,
        filepath,
        diagnostic_path.as_str(),
        "private-native-tool-frames.ndjson",
    ] {
        assert!(!public.contains(canary));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            private.as_file().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn private_native_tool_diagnostic_budgets_reject_before_appending_or_counting() {
    for (accepted_frames, frame_bytes) in [(64, 256), (1, 64 * 1024), (16, 64 * 1024)] {
        let private = NamedTempFile::new().unwrap();
        let mut probe = SdkOriginProbe::new(Uuid::new_v4());
        probe.state.lock().unwrap().private_tool_frames =
            Some(private.as_file().try_clone().unwrap());
        let mut frame = json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update",
            "toolCallId":"offline-budget-tool","rawInput":{"payload":""}}}});
        let overhead = serde_json::to_vec(&frame).unwrap().len() + 1;
        frame["params"]["update"]["rawInput"]["payload"] =
            json!("x".repeat(frame_bytes - overhead));
        assert_eq!(serde_json::to_vec(&frame).unwrap().len() + 1, frame_bytes);
        for _index in 0..accepted_frames {
            assert!(probe.receive(&frame).unwrap().is_none());
        }
        let before = fs::read(private.path()).unwrap();
        assert_eq!(before.len(), accepted_frames * frame_bytes);
        let before_report = probe.report(None, None);
        // 分别越过条数、单帧字节和总字节门槛；边界内真实记录先成功。
        if accepted_frames == 1 {
            let payload = frame["params"]["update"]["rawInput"]["payload"]
                .as_str()
                .unwrap();
            frame["params"]["update"]["rawInput"]["payload"] = json!(format!("{payload}x"));
            assert_eq!(serde_json::to_vec(&frame).unwrap().len() + 1, 64 * 1024 + 1);
        } else if accepted_frames == 16 {
            frame["params"]["update"]["rawInput"]["payload"] = json!("");
            assert_eq!(before.len(), 1024 * 1024);
        }
        let rejected = probe.receive(&frame);
        assert!(matches!(rejected, Err(RuntimeError::Protocol(message))
            if message == "原生工具私有诊断超过有界预算"));
        let state = probe.state.lock().unwrap();
        assert_eq!(state.private_tool_frame_count, accepted_frames);
        assert_eq!(state.private_tool_frame_bytes, before.len());
        drop(state);
        assert_eq!(
            private.as_file().metadata().unwrap().len() as usize,
            before.len()
        );
        assert!(fs::read(private.path()).unwrap() == before);
        assert!(probe.report(None, None) == before_report);
    }
}

fn confirm_probe_prompt(probe: &SdkOriginProbe, session: &str, prompt: &str) {
    probe.confirm_runtime_session(session).unwrap();
    probe.confirm_runtime_ack(prompt).unwrap();
    probe.confirm_runtime_started(prompt).unwrap();
}

fn exact_native_tool_frame(session: &str, prompt: &str, tool: &str, initial: bool) -> Value {
    let mut input = json!({"tool_name":format!("{SERVER_NAME}__inspect"),"tool_input":{}});
    if !initial {
        input["variant"] = json!("UseTool");
    }
    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session,
        "_meta":{"promptId":prompt},"update":{"sessionUpdate":if initial {"tool_call"} else {"tool_call_update"},
            "toolCallId":tool,"kind":"other","rawInput":input,"_meta":{"x.ai/tool":{"name":"use_tool"}}}}})
}

fn install_exact_tool_ledger(probe: &mut SdkOriginProbe, session: &str, prompt: &str, tool: &str) {
    for initial in [true, false] {
        assert!(
            probe
                .receive(&exact_native_tool_frame(session, prompt, tool, initial))
                .unwrap()
                .is_none()
        );
    }
}

fn exact_permission_frame(session: &str, tool: &str) -> Value {
    json!({"jsonrpc":"2.0","id":71,"method":"session/request_permission","params":{"sessionId":session,
        "toolCall":{"toolCallId":tool,"kind":"other","rawInput":{
            "tool_name":format!("{SERVER_NAME}__inspect"),"tool_input":{},"variant":"UseTool"}},
        "options":[{"optionId":"allow-once","kind":"allow_once","name":"OFFLINE_OPTION_LABEL_CANARY"},
            {"optionId":"reject-once","kind":"reject_once","name":"OFFLINE_REJECT_LABEL_CANARY"},
            {"optionId":"always-allow","kind":"allow_always","name":"OFFLINE_PERMANENT_LABEL_CANARY"}]}})
}

fn exact_permission_probe() -> (SdkOriginProbe, String, String, Value) {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let session = Uuid::new_v4().to_string();
    let prompt = Uuid::new_v4().to_string();
    confirm_probe_prompt(&probe, &session, &prompt);
    install_exact_tool_ledger(&mut probe, &session, &prompt, "native-inspect-1");
    let request = exact_permission_frame(&session, "native-inspect-1");
    (probe, session, prompt, request)
}

#[test]
fn exact_native_inspect_approval_allows_once_and_identical_retries_do_not_expand_allowance() {
    let (mut probe, session, prompt, request) = exact_permission_probe();
    let first = probe.receive(&request).unwrap().unwrap();
    let repeated = probe.receive(&request).unwrap().unwrap();
    assert_eq!(first.writes, repeated.writes);
    assert_eq!(
        first.writes[0]["result"]["outcome"],
        json!({"outcome":"selected","optionId":"allow-once"})
    );
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["permission_request_count"], 2);
    assert_eq!(report["approval_allow_count"], 1);
    assert_eq!(report["approval_duplicate_count"], 1);
    assert_eq!(report["approval_deny_count"], 0);
    assert_eq!(report["approval_all_denied"], false);
    assert_eq!(report["native_contract_verified"], true);
    assert_eq!(report["origin_verification"], "unknown");
    assert_eq!(report["full_native_origin_fields_observed"], false);
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
    assert!(!report.to_string().contains("OPTION_LABEL_CANARY"));
}

#[test]
fn exact_inspect_approval_rejects_missing_confirmed_runtime_ledger_and_closed_window() {
    for mutation in 0..8 {
        let (mut probe, session, prompt, request) = exact_permission_probe();
        {
            let mut state = probe.state.lock().unwrap();
            match mutation {
                0 => state.known_native_session = None,
                1 => state.acknowledged_native_prompt = None,
                2 => state.started_native_prompt = None,
                3 => state.approval_prompt_open = false,
                4 => {
                    state
                        .tools
                        .get_mut("native-inspect-1")
                        .unwrap()
                        .initial_input_seen = false
                }
                5 => {
                    state
                        .tools
                        .get_mut("native-inspect-1")
                        .unwrap()
                        .final_input_seen = false
                }
                6 => {
                    state.tools.get_mut("native-inspect-1").unwrap().session_id =
                        Some(Uuid::new_v4().to_string())
                }
                7 => {
                    state.tools.get_mut("native-inspect-1").unwrap().prompt_id =
                        Some(Uuid::new_v4().to_string())
                }
                _ => unreachable!("有限测试范围"),
            }
        }
        let denied = probe.receive(&request).unwrap().unwrap();
        assert_eq!(
            denied.writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
        let report = probe.report(Some(&session), Some(&prompt));
        assert_eq!(report["approval_allow_count"], 0);
        assert_eq!(report["approval_deny_count"], 1);
        assert!(probe.check().is_err());
    }
}

#[test]
fn exact_inspect_approval_rejects_changed_payload_extra_keys_wrong_identity_and_other_allow_options()
 {
    for mutation in 0..19 {
        let (mut probe, session, prompt, mut request) = exact_permission_probe();
        match mutation {
            0 => request["params"]["sessionId"] = json!(Uuid::new_v4()),
            1 => request["params"]["_meta"]["promptId"] = json!(Uuid::new_v4()),
            2 => request["params"]["toolCall"]["toolCallId"] = json!("other-tool"),
            3 => {
                request["params"]["toolCall"]["rawInput"]["extra"] =
                    json!("OFFLINE_PRIVATE_ARGUMENT_CANARY")
            }
            4 => {
                request["params"]["toolCall"]["rawInput"]["tool_input"] =
                    json!({"write":"OFFLINE_PRIVATE_ARGUMENT_CANARY"})
            }
            5 => request["params"]["toolCall"]["rawInput"]["tool_name"] = json!("other__inspect"),
            6 => {
                request["params"]["toolCall"]["rawInput"]
                    .as_object_mut()
                    .unwrap()
                    .remove("variant");
            }
            7 => request["params"]["toolCall"]["rawInput"]["variant"] = json!("OtherTool"),
            8 => request["params"]["toolCall"]["kind"] = json!("read"),
            9 => request["params"]["toolCall"]["_meta"]["x.ai/tool"]["name"] = json!("inspect"),
            10 => request["params"]["_meta"]["isReplay"] = json!(true),
            11 => request["params"]["options"][0]["optionId"] = json!("enable-always-approve"),
            12 => request["params"]["options"][0]["kind"] = json!("allow_always"),
            13 => {
                let copy = request["params"]["options"][0].clone();
                request["params"]["options"]
                    .as_array_mut()
                    .unwrap()
                    .push(copy);
            }
            14 => request["params"]["options"][0]["extra"] = json!(true),
            15 => {
                request["params"]["options"][0]["name"] =
                    json!({"credential":"OFFLINE_PRIVATE_ARGUMENT_CANARY"})
            }
            16 => request["params"]["options"][1]["kind"] = json!("allow_once"),
            17 => request["params"]["toolCall"]["rawInput"]["tool_input"] = Value::Null,
            18 => request["jsonrpc"] = json!("1.0"),
            _ => unreachable!("有限测试范围"),
        }
        let denied = probe.receive(&request).unwrap().unwrap();
        assert_eq!(
            denied.writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
        let report = probe.report(Some(&session), Some(&prompt));
        assert_eq!(report["approval_allow_count"], 0);
        assert!(!report.to_string().contains("PRIVATE_ARGUMENT_CANARY"));
        assert!(probe.check().is_err());
    }
}

#[test]
fn approved_request_id_cannot_be_reused_with_changed_payload_or_after_turn_close() {
    for mutation in 0..3 {
        let (mut probe, session, prompt, mut request) = exact_permission_probe();
        probe.receive(&request).unwrap();
        match mutation {
            0 => request["params"]["options"][0]["name"] = json!("changed label"),
            1 => request["id"] = json!(72),
            2 => probe.close_approval_prompt(),
            _ => unreachable!("有限测试范围"),
        }
        let denied = probe.receive(&request).unwrap().unwrap();
        assert_eq!(
            denied.writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
        let report = probe.report(Some(&session), Some(&prompt));
        assert_eq!(report["approval_allow_count"], 1);
        assert_eq!(report["approval_deny_count"], 1);
        assert_eq!(report["approval_duplicate_count"], 0);
        assert!(probe.check().is_err());
    }
}

#[test]
fn native_use_tool_name_without_exact_initial_final_input_and_source_never_grants_approval() {
    for mutation in 0..9 {
        let mut probe = SdkOriginProbe::new(Uuid::new_v4());
        let session = Uuid::new_v4().to_string();
        let prompt = Uuid::new_v4().to_string();
        confirm_probe_prompt(&probe, &session, &prompt);
        let mut initial = exact_native_tool_frame(&session, &prompt, "native-inspect-1", true);
        match mutation {
            0 => initial["params"]["update"]["rawInput"]["variant"] = json!("UseTool"),
            1 => initial["params"]["update"]["rawInput"]["tool_name"] = json!("other__inspect"),
            2 => {
                initial["params"]["update"]["rawInput"]["tool_input"] =
                    json!({"secret":"OFFLINE_LEDGER_CANARY"})
            }
            3 => initial["params"]["sessionId"] = json!(Uuid::new_v4()),
            4 => initial["params"]["_meta"]["promptId"] = json!(Uuid::new_v4()),
            5 => {
                initial["params"]["_meta"]
                    .as_object_mut()
                    .unwrap()
                    .remove("promptId");
            }
            6 => {
                initial["params"]["update"]["_meta"]["x.ai/tool"]["name"] =
                    json!(format!("{SERVER_NAME}__inspect"))
            }
            7 => initial["params"]["update"]["kind"] = json!("execute"),
            8 => {
                initial["params"]["update"]["rawInput"] =
                    json!({"tool_name":format!("{SERVER_NAME}__inspect")})
            }
            _ => unreachable!("有限测试范围"),
        }
        probe.receive(&initial).unwrap();
        probe
            .receive(&exact_native_tool_frame(
                &session,
                &prompt,
                "native-inspect-1",
                false,
            ))
            .unwrap();
        let denied = probe
            .receive(&exact_permission_frame(&session, "native-inspect-1"))
            .unwrap()
            .unwrap();
        assert_eq!(
            denied.writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
        let report = probe.report(Some(&session), Some(&prompt));
        assert_eq!(report["approval_allow_count"], 0);
        assert!(!report.to_string().contains("LEDGER_CANARY"));
    }
}

#[test]
fn confirmed_started_prompt_is_not_inserted_into_sdk_origin_fields() {
    let (mut probe, session, prompt, request) = exact_permission_probe();
    probe.receive(&request).unwrap();
    probe
        .receive(
            &json!({"jsonrpc":"2.0","id":9,"method":"_x.ai/mcp/sdk_call","params":{
        "serverId":probe.server_id,"message":{"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"inspect","arguments":{}}}}}),
        )
        .unwrap();
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["origin_verification"], "candidate_mapping_only");
    assert_eq!(report["full_native_origin_fields_observed"], false);
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
}

#[test]
fn final_native_input_must_match_initial_tool_id_source_and_exact_variant_contract() {
    for mutation in 0..8 {
        let mut probe = SdkOriginProbe::new(Uuid::new_v4());
        let session = Uuid::new_v4().to_string();
        let prompt = Uuid::new_v4().to_string();
        confirm_probe_prompt(&probe, &session, &prompt);
        probe
            .receive(&exact_native_tool_frame(
                &session,
                &prompt,
                "native-inspect-1",
                true,
            ))
            .unwrap();
        let mut final_frame = exact_native_tool_frame(&session, &prompt, "native-inspect-1", false);
        match mutation {
            0 => final_frame["params"]["update"]["toolCallId"] = json!("other-tool"),
            1 => final_frame["params"]["sessionId"] = json!(Uuid::new_v4()),
            2 => final_frame["params"]["_meta"]["promptId"] = json!(Uuid::new_v4()),
            3 => final_frame["params"]["update"]["rawInput"]["extra"] = json!(true),
            4 => {
                final_frame["params"]["update"]["rawInput"]
                    .as_object_mut()
                    .unwrap()
                    .remove("variant");
            }
            5 => final_frame["params"]["update"]["rawInput"]["tool_input"] = Value::Null,
            6 => final_frame["params"]["update"]["kind"] = json!("execute"),
            7 => final_frame["params"]["update"]["_meta"]["x.ai/tool"]["name"] = json!("other"),
            _ => unreachable!("有限测试范围"),
        }
        probe.receive(&final_frame).unwrap();
        let denied = probe
            .receive(&exact_permission_frame(&session, "native-inspect-1"))
            .unwrap()
            .unwrap();
        assert_eq!(
            denied.writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
        assert!(probe.check().is_err());
        assert_eq!(
            probe.report(Some(&session), Some(&prompt))["approval_allow_count"],
            0
        );
    }
}

#[test]
fn initial_final_ledger_before_started_does_not_authorize_until_real_runtime_confirmation() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let session = Uuid::new_v4().to_string();
    let prompt = Uuid::new_v4().to_string();
    probe.confirm_runtime_session(&session).unwrap();
    install_exact_tool_ledger(&mut probe, &session, &prompt, "native-inspect-1");
    assert_eq!(
        probe.report(Some(&session), Some(&prompt))["native_contract_verified"],
        false
    );
    assert!(probe.confirm_runtime_started(&prompt).is_err());
    probe.confirm_runtime_ack(&prompt).unwrap();
    assert!(probe.confirm_runtime_ack(&prompt).is_err());
    assert!(
        probe
            .confirm_runtime_started(&Uuid::new_v4().to_string())
            .is_err()
    );
    probe.confirm_runtime_started(&prompt).unwrap();
    assert!(probe.confirm_runtime_started(&prompt).is_err());
    let allowed = probe
        .receive(&exact_permission_frame(&session, "native-inspect-1"))
        .unwrap()
        .unwrap();
    assert_eq!(
        allowed.writes[0]["result"]["outcome"]["optionId"],
        "allow-once"
    );
    assert_eq!(
        probe.report(Some(&session), Some(&prompt))["full_native_origin_fields_observed"],
        false
    );
}

#[test]
fn replay_only_tool_ledger_cannot_authorize_live_permission_request() {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let session = Uuid::new_v4().to_string();
    let prompt = Uuid::new_v4().to_string();
    confirm_probe_prompt(&probe, &session, &prompt);
    for initial in [true, false] {
        let mut frame = exact_native_tool_frame(&session, &prompt, "native-inspect-1", initial);
        frame["params"]["_meta"]["isReplay"] = json!(true);
        probe.receive(&frame).unwrap();
    }
    let denied = probe
        .receive(&exact_permission_frame(&session, "native-inspect-1"))
        .unwrap()
        .unwrap();
    assert_eq!(
        denied.writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    assert_eq!(
        probe.report(Some(&session), Some(&prompt))["approval_allow_count"],
        0
    );
}

#[test]
fn calibrated_three_options_offer_permanent_permission_but_select_only_allow_once() {
    let (mut probe, session, prompt, mut request) = exact_permission_probe();
    // 永久许可排在第一项也不能成为选中项，显示标签不授权。
    request["params"]["options"]
        .as_array_mut()
        .unwrap()
        .rotate_right(1);
    assert_eq!(request["params"]["options"][0]["optionId"], "always-allow");
    let response = probe.receive(&request).unwrap().unwrap();
    assert_eq!(
        response.writes[0]["result"]["outcome"],
        json!({"outcome":"selected","optionId":"allow-once"})
    );
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["approval_allow_count"], 1);
    assert_eq!(report["approval_deny_count"], 0);
    assert_eq!(report["approval_all_denied"], false);
    assert!(
        report["frames"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|frame| frame["event"] == "probe_approval_observed")
            .all(|frame| frame["selected_option_id"] == "allow-once")
    );
    assert!(!report.to_string().contains("PERMANENT_LABEL_CANARY"));
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
}

#[test]
fn offered_permanent_option_requires_unique_calibrated_id_and_kind_without_granting_it() {
    for mutation in 0..6 {
        let (mut probe, session, prompt, mut request) = exact_permission_probe();
        match mutation {
            0 => request["params"]["options"][2]["optionId"] = json!("unknown-always"),
            1 => request["params"]["options"][2]["kind"] = json!("reject_once"),
            2 => request["params"]["options"][2]["kind"] = json!("allow_once"),
            3 => {
                let copy = request["params"]["options"][2].clone();
                request["params"]["options"]
                    .as_array_mut()
                    .unwrap()
                    .push(copy);
            }
            4 => {
                let copy = request["params"]["options"][0].clone();
                request["params"]["options"]
                    .as_array_mut()
                    .unwrap()
                    .push(copy);
            }
            5 => request["params"]["options"][2]["extra"] = json!(true),
            _ => unreachable!("有限测试范围"),
        }
        let response = probe.receive(&request).unwrap().unwrap();
        assert_eq!(
            response.writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
        assert_eq!(
            probe.report(Some(&session), Some(&prompt))["approval_allow_count"],
            0
        );
        assert!(probe.check().is_err());
    }
}

#[test]
fn changed_offered_permanent_option_label_conflicts_with_same_request_id_cached_once_response() {
    let (mut probe, session, prompt, mut request) = exact_permission_probe();
    let first = probe.receive(&request).unwrap().unwrap();
    assert_eq!(
        first.writes[0]["result"]["outcome"]["optionId"],
        "allow-once"
    );
    request["params"]["options"][2]["name"] = json!("changed permanent label");
    let denied = probe.receive(&request).unwrap().unwrap();
    assert_eq!(
        denied.writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["approval_allow_count"], 1);
    assert_eq!(report["approval_deny_count"], 1);
    assert_eq!(report["approval_duplicate_count"], 0);
}

fn exact_search_native_tool_frame(session: &str, prompt: &str, tool: &str, initial: bool) -> Value {
    let mut frame = exact_native_tool_frame(session, prompt, tool, initial);
    let update = &mut frame["params"]["update"];
    update["_meta"]["x.ai/tool"]["name"] = json!("search_tool");
    update["rawInput"] = json!({"limit":5,"query":"infinishell-sdk-origin-probe inspect"});
    if initial {
        update.as_object_mut().unwrap().remove("kind");
    } else {
        update["rawInput"]["variant"] = json!("SearchTool");
    }
    frame
}

fn install_search_ledger(probe: &mut SdkOriginProbe, session: &str, prompt: &str, tool: &str) {
    for initial in [true, false] {
        assert!(
            probe
                .receive(&exact_search_native_tool_frame(
                    session, prompt, tool, initial
                ))
                .unwrap()
                .is_none()
        );
    }
}

fn exact_search_permission_frame(session: &str, tool: &str) -> Value {
    let mut request = exact_permission_frame(session, tool);
    request["id"] = json!(81);
    request["params"]["toolCall"]["rawInput"] = json!({"limit":5,
        "query":"infinishell-sdk-origin-probe inspect","variant":"SearchTool"});
    request
}

fn exact_discovery_probe() -> (SdkOriginProbe, String, String, Value) {
    let mut probe = SdkOriginProbe::new(Uuid::new_v4());
    let session = Uuid::new_v4().to_string();
    let prompt = Uuid::new_v4().to_string();
    confirm_probe_prompt(&probe, &session, &prompt);
    install_search_ledger(&mut probe, &session, &prompt, "native-search-1");
    let request = exact_search_permission_frame(&session, "native-search-1");
    (probe, session, prompt, request)
}

#[test]
fn exact_readonly_search_allows_once_without_becoming_an_inspect_or_sdk_origin() {
    let (mut probe, session, prompt, request) = exact_discovery_probe();
    let first = probe.receive(&request).unwrap().unwrap();
    let retry = probe.receive(&request).unwrap().unwrap();
    assert_eq!(first.writes, retry.writes);
    assert_eq!(
        first.writes[0]["result"]["outcome"]["optionId"],
        "allow-once"
    );
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["search_allow_count"], 1);
    assert_eq!(report["inspect_allow_count"], 0);
    assert_eq!(report["approval_allow_count"], 1);
    assert_eq!(report["approval_duplicate_count"], 1);
    assert_eq!(report["approval_deny_count"], 0);
    assert_eq!(report["native_contract_verified"], false);
    assert_eq!(report["discovery_native_contract_verified"], true);
    assert_eq!(report["full_native_origin_fields_observed"], false);
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
    assert_eq!(report["origin_verification"], "unknown");
    assert!(!probe.state.lock().unwrap().tools["native-search-1"].probe_tool);
    assert!(
        !report
            .to_string()
            .contains("infinishell-sdk-origin-probe inspect")
    );
    assert!(probe.check().is_ok());
}

#[test]
fn search_permission_rejects_nonliteral_query_limit_variant_and_extra_arguments() {
    for mutation in 0..10 {
        let (mut probe, session, prompt, mut request) = exact_discovery_probe();
        let raw = &mut request["params"]["toolCall"]["rawInput"];
        match mutation {
            0 => raw["query"] = json!("inspect"),
            1 => raw["query"] = json!("infinishell-sdk-origin-probe inspect "),
            2 => raw["limit"] = json!(6),
            3 => raw["limit"] = json!(5.0),
            4 => raw["limit"] = json!("5"),
            5 => raw["limit"] = json!(true),
            6 => raw["variant"] = json!("UseTool"),
            7 => raw["extra"] = json!("OFFLINE_SEARCH_ARGUMENT_CANARY"),
            8 => {
                raw.as_object_mut().unwrap().remove("variant");
            }
            9 => raw["query"] = json!({"credential":"OFFLINE_SEARCH_ARGUMENT_CANARY"}),
            _ => unreachable!("有限测试范围"),
        }
        let denied = probe.receive(&request).unwrap().unwrap();
        assert_eq!(
            denied.writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
        let report = probe.report(Some(&session), Some(&prompt));
        assert_eq!(report["search_allow_count"], 0);
        assert_eq!(report["approval_deny_count"], 1);
        assert!(!report.to_string().contains("SEARCH_ARGUMENT_CANARY"));
        assert!(probe.check().is_err());
    }
}

#[test]
fn search_initial_final_contract_requires_real_identity_and_literal_native_shape() {
    for initial_mutation in [true, false] {
        for mutation in 0..8 {
            let mut probe = SdkOriginProbe::new(Uuid::new_v4());
            let session = Uuid::new_v4().to_string();
            let prompt = Uuid::new_v4().to_string();
            confirm_probe_prompt(&probe, &session, &prompt);
            let mut initial =
                exact_search_native_tool_frame(&session, &prompt, "native-search-1", true);
            let mut final_frame =
                exact_search_native_tool_frame(&session, &prompt, "native-search-1", false);
            let frame = if initial_mutation {
                &mut initial
            } else {
                &mut final_frame
            };
            match mutation {
                0 => frame["params"]["sessionId"] = json!(Uuid::new_v4()),
                1 => frame["params"]["_meta"]["promptId"] = json!(Uuid::new_v4()),
                2 => frame["params"]["update"]["toolCallId"] = json!("unrelated-tool"),
                3 => frame["params"]["update"]["rawInput"]["query"] = json!("other query"),
                4 => frame["params"]["update"]["rawInput"]["limit"] = json!(4),
                5 => frame["params"]["update"]["rawInput"]["extra"] = json!(true),
                6 => frame["params"]["update"]["_meta"]["x.ai/tool"]["name"] = json!("use_tool"),
                7 => frame["params"]["update"]["kind"] = json!("execute"),
                _ => unreachable!("有限测试范围"),
            }
            probe.receive(&initial).unwrap();
            probe.receive(&final_frame).unwrap();
            let denied = probe
                .receive(&exact_search_permission_frame(&session, "native-search-1"))
                .unwrap()
                .unwrap();
            assert_eq!(
                denied.writes[0]["result"]["outcome"]["outcome"],
                "cancelled"
            );
            assert_eq!(
                probe.report(Some(&session), Some(&prompt))["search_allow_count"],
                0
            );
            assert!(probe.check().is_err());
        }
    }
}

#[test]
fn search_and_inspect_have_two_distinct_single_use_allowances_and_independent_retry_caches() {
    let (mut probe, session, prompt, search) = exact_discovery_probe();
    let first_search = probe.receive(&search).unwrap().unwrap();
    install_exact_tool_ledger(&mut probe, &session, &prompt, "native-inspect-1");
    let inspect = exact_permission_frame(&session, "native-inspect-1");
    let first_inspect = probe.receive(&inspect).unwrap().unwrap();
    assert_eq!(
        probe.receive(&search).unwrap().unwrap().writes,
        first_search.writes
    );
    assert_eq!(
        probe.receive(&inspect).unwrap().unwrap().writes,
        first_inspect.writes
    );
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["approval_allow_count"], 2);
    assert_eq!(report["search_allow_count"], 1);
    assert_eq!(report["inspect_allow_count"], 1);
    assert_eq!(report["approval_duplicate_count"], 2);
    assert_eq!(report["native_contract_verified"], true);
    assert_eq!(report["discovery_native_contract_verified"], true);
    assert_eq!(report["full_native_origin_fields_observed"], false);
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
    assert_eq!(probe.state.lock().unwrap().tools.len(), 2);
    assert!(probe.check().is_ok());
}

#[test]
fn second_search_new_native_id_or_new_approval_id_never_gets_another_allowance() {
    for second_native_tool in [true, false] {
        let (mut probe, session, prompt, search) = exact_discovery_probe();
        probe.receive(&search).unwrap();
        let mut request = search;
        request["id"] = json!(82);
        if second_native_tool {
            install_search_ledger(&mut probe, &session, &prompt, "native-search-2");
            request["params"]["toolCall"]["toolCallId"] = json!("native-search-2");
        }
        let denied = probe.receive(&request).unwrap().unwrap();
        assert_eq!(
            denied.writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
        let report = probe.report(Some(&session), Some(&prompt));
        assert_eq!(report["search_allow_count"], 1);
        assert_eq!(report["approval_deny_count"], 1);
        assert!(probe.check().is_err());
    }
}

#[test]
fn search_cached_permission_is_rejected_after_identity_payload_or_window_changes() {
    for mutation in 0..7 {
        let (mut probe, session, prompt, mut request) = exact_discovery_probe();
        probe.receive(&request).unwrap();
        match mutation {
            0 => request["params"]["options"][0]["name"] = json!("changed label"),
            1 => request["params"]["toolCall"]["rawInput"]["limit"] = json!(6),
            2 => request["params"]["sessionId"] = json!(Uuid::new_v4()),
            3 => request["params"]["_meta"]["promptId"] = json!(Uuid::new_v4()),
            4 => probe.close_approval_prompt(),
            5 => probe.state.lock().unwrap().acknowledged_native_prompt = None,
            6 => probe.state.lock().unwrap().started_native_prompt = None,
            _ => unreachable!("有限测试范围"),
        }
        let denied = probe.receive(&request).unwrap().unwrap();
        assert_eq!(
            denied.writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
        let report = probe.report(Some(&session), Some(&prompt));
        assert_eq!(report["search_allow_count"], 1);
        assert_eq!(report["approval_duplicate_count"], 0);
        assert_eq!(report["approval_deny_count"], 1);
        assert!(probe.check().is_err());
    }
}

#[test]
fn cached_search_retry_during_inspect_initial_keeps_cache_but_does_not_authorize_partial_inspect() {
    let (mut probe, session, prompt, search) = exact_discovery_probe();
    let first = probe.receive(&search).unwrap().unwrap();
    probe
        .receive(&exact_native_tool_frame(
            &session,
            &prompt,
            "native-inspect-1",
            true,
        ))
        .unwrap();
    assert_eq!(
        probe.receive(&search).unwrap().unwrap().writes,
        first.writes
    );
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["search_allow_count"], 1);
    assert_eq!(report["approval_duplicate_count"], 1);
    assert_eq!(report["native_contract_verified"], false);
    assert_eq!(report["discovery_native_contract_verified"], true);
    let denied = probe
        .receive(&exact_permission_frame(&session, "native-inspect-1"))
        .unwrap()
        .unwrap();
    assert_eq!(
        denied.writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    assert_eq!(
        probe.report(Some(&session), Some(&prompt))["inspect_allow_count"],
        0
    );
}

#[test]
fn completed_search_cannot_substitute_for_inspect_in_full_native_sdk_origin_relation() {
    let (mut probe, session, prompt, search) = exact_discovery_probe();
    probe.receive(&search).unwrap();
    install_exact_tool_ledger(&mut probe, &session, &prompt, "native-inspect-1");
    for tool in ["native-search-1", "native-inspect-1"] {
        probe
            .receive(
                &json!({"method":"session/update","params":{"sessionId":session,
            "_meta":{"promptId":prompt},"update":{"sessionUpdate":"tool_call_update",
                "toolCallId":tool,"status":"completed"}}}),
            )
            .unwrap();
    }
    // 合成 SDK 来源字段完整，但指向搜索账本；不能把来源改成当前 inspect。
    probe.receive(&json!({"jsonrpc":"2.0","id":9,"method":"_x.ai/mcp/sdk_call","params":{
        "serverId":probe.server_id,"sessionId":session,"promptId":prompt,"toolCallId":"native-search-1",
        "message":{"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"inspect","arguments":{}}}}})).unwrap();
    let report = probe.report(Some(&session), Some(&prompt));
    assert_eq!(report["inspect_call_count"], 1);
    assert_eq!(report["full_native_origin_fields_observed"], true);
    assert_eq!(report["native_origin_ledger_relation_verified"], false);
    assert_eq!(report["origin_verification"], "candidate_mapping_only");
    assert_eq!(report["native_contract_verified"], true);
    assert_eq!(report["discovery_native_contract_verified"], true);
    assert!(probe.check().is_ok());
}
