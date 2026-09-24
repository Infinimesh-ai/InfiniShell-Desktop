//! 固定文件工具策略的真实六输入验收；不把测试夹具视为文件系统沙箱。

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{GrokProtocol, connect_protocol, validate_options};
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome, managed_process,
};

const SCOPE: &str = "authenticated_files_policy_six_input_lifecycle";
const PHASES: [&str; 6] = [
    "write_allow",
    "write_deny",
    "edit_allow",
    "edit_deny",
    "pending_cancel",
    "cold_read",
];

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .map_err(|_| "安全证据写入失败".into())
}

struct ExpectedTool {
    name: &'static str,
    namespace: &'static str,
    input: Value,
    decision: Option<ApprovalDecision>,
}

impl ExpectedTool {
    fn read(path: &Path) -> Self {
        Self {
            name: "read_file",
            namespace: "grok_build",
            input: json!({"variant":"ReadFile","target_file":path}),
            decision: Some(ApprovalDecision::AllowOnce),
        }
    }

    fn exact_permission(&self, details: &Value, session: &str) -> bool {
        let call = &details["toolCall"];
        let meta = &call["_meta"]["x.ai/tool"];
        let read = self.name == "read_file";
        if details["sessionId"] != session
            || call["kind"] != if read { "read" } else { "edit" }
            || meta["version"] != 1
            || meta["name"] != self.name
            || meta["namespace"] != self.namespace
            || meta["read_only"] != read
        {
            return false;
        }
        self.parameters_match(&call["rawInput"])
    }

    fn parameters_match(&self, input: &Value) -> bool {
        let read = self.name == "read_file";
        let Some(mut input) = input.as_object().cloned() else {
            return false;
        };
        // 原生可补上已核实的默认字段；未知字段或其它默认值均拒绝。
        if read {
            for key in ["offset", "limit"] {
                if let Some(value) = input.remove(key)
                    && !value.is_null()
                    && value.as_u64() != Some(1)
                {
                    return false;
                }
            }
            for key in ["pages", "format"] {
                if input.remove(key).is_some_and(|value| !value.is_null()) {
                    return false;
                }
            }
        } else if self.name == "search_replace"
            && input
                .remove("replace_all")
                .is_some_and(|value| value != false)
        {
            return false;
        }
        Value::Object(input) == self.input
    }
}

fn permission_diagnostic(
    expected: Option<&ExpectedTool>,
    details: &Value,
    session: Option<&str>,
    request_identity_matches: bool,
) -> Value {
    let call = &details["toolCall"];
    let meta = &call["_meta"]["x.ai/tool"];
    let input = &call["rawInput"];
    let required = expected.and_then(|tool| tool.input.as_object());
    let fields = input.as_object();
    let missing = required.map_or(0, |keys| {
        keys.keys()
            .filter(|key| fields.is_none_or(|values| !values.contains_key(*key)))
            .count()
    });
    let extra = fields.map_or(0, |keys| {
        keys.keys()
            .filter(|key| {
                !required.is_some_and(|values| values.contains_key(*key))
                    && !expected.is_some_and(|tool| {
                        (tool.name == "read_file"
                            && matches!(key.as_str(), "offset" | "limit" | "pages" | "format"))
                            || (tool.name == "search_replace" && key.as_str() == "replace_all")
                    })
            })
            .count()
    });
    // 不公开工具名、路径、参数值或未知键名，只保留固定比较项和 JSON 类型。
    let mut types = serde_json::Map::new();
    for key in [
        "variant",
        "target_file",
        "file_path",
        "content",
        "old_string",
        "new_string",
        "replace_all",
        "offset",
        "limit",
        "pages",
        "format",
    ] {
        types.insert(
            key.into(),
            json!(super::diagnostic_value_type(input.get(key))),
        );
    }
    json!({
        "expected_tool_present":expected.is_some(),
        "request_identity_matches":request_identity_matches,
        "session_id_matches":session.is_some_and(|id| details["sessionId"] == id),
        "name_matches":expected.is_some_and(|tool| meta["name"] == tool.name),
        "namespace_matches":expected.is_some_and(|tool| meta["namespace"] == tool.namespace),
        "version_matches":meta["version"] == 1,
        "kind_matches":expected.is_some_and(|tool| call["kind"] == if tool.name == "read_file" {"read"} else {"edit"}),
        "read_only_matches":expected.is_some_and(|tool| meta["read_only"] == (tool.name == "read_file")),
        "variant_matches":expected.is_some_and(|tool| input["variant"] == tool.input["variant"]),
        "parameters_match":expected.is_some_and(|tool| tool.parameters_match(input)),
        "missing_parameter_count":missing,"extra_parameter_count":extra,
        "raw_input_type":super::diagnostic_value_type(call.get("rawInput")),
        "name_type":super::diagnostic_value_type(meta.get("name")),
        "namespace_type":super::diagnostic_value_type(meta.get("namespace")),
        "version_type":super::diagnostic_value_type(meta.get("version")),
        "kind_type":super::diagnostic_value_type(call.get("kind")),
        "parameter_types":types
    })
}

fn file_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => fs::read(path)
            .map(Some)
            .map_err(|_| "合成文件读取失败".into()),
        Ok(_) => Err("合成目标不是普通文件".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("合成文件属性不可读".into()),
    }
}

struct Case {
    phase: &'static str,
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    tools: Vec<ExpectedTool>,
    prompt: String,
    final_text: String,
    outcome: TurnOutcome,
    cancellation_category: Option<&'static str>,
}

fn case(root: &Path, index: usize) -> Result<Case, String> {
    let phase = *PHASES.get(index).ok_or("阶段超出六输入合同")?;
    let fixture: Value = serde_json::from_slice(
        &fs::read(root.join("file-cases.json")).map_err(|_| "缺少合成文件合同")?,
    )
    .map_err(|_| "合成文件合同无效")?;
    let row = &fixture[phase];
    let name = row["name"].as_str().ok_or("合成文件名缺失")?;
    let expected_name = match phase {
        "write_allow" | "cold_read" => "write-allow.txt",
        "write_deny" => "write-deny.txt",
        "edit_allow" => "edit-allow.txt",
        "edit_deny" => "edit-deny.txt",
        "pending_cancel" => "cancel.txt",
        _ => return Err("未知合成阶段".into()),
    };
    if name != expected_name {
        return Err("合成文件名越界".into());
    }
    let path = root.join("project").join(name);
    let before = row["before"].as_str().map(str::to_owned);
    let after = row["after"].as_str().map(str::to_owned);
    if (!row["before"].is_null() && before.is_none())
        || (!row["after"].is_null() && after.is_none())
    {
        return Err("合成文件字节合同非法".into());
    }
    let proposed = row["proposed"].as_str().ok_or("合成操作内容缺失")?;
    let allowed = matches!(phase, "write_allow" | "edit_allow" | "cold_read");
    let cancel = phase == "pending_cancel";
    let edit = matches!(phase, "edit_allow" | "edit_deny");
    let final_text = if phase == "cold_read" {
        after.as_deref().ok_or("冷读取结果缺失")?.trim().to_owned()
    } else if allowed {
        phase.to_uppercase()
    } else {
        String::new()
    };
    let mut tools = Vec::new();
    let prompt;
    if phase == "cold_read" {
        tools.push(ExpectedTool::read(&path));
        prompt = format!(
            "仅调用 read_file 一次，target_file={}，不要其它参数或工具。只输出文件唯一一行的原文，不解释。",
            path.display()
        );
    } else {
        if edit {
            tools.push(ExpectedTool::read(&path));
        }
        let (tool, variant, namespace) = if edit {
            ("search_replace", "SearchReplace", "grok_build")
        } else {
            ("write", "Write", "opencode")
        };
        let input = if edit {
            json!({"variant":variant,"file_path":path,"old_string":before.as_deref().ok_or("编辑前文件不存在")?,"new_string":proposed})
        } else {
            json!({"variant":variant,"file_path":path,"content":proposed})
        };
        let mut parameters = input.clone();
        parameters.as_object_mut().unwrap().remove("variant");
        tools.push(ExpectedTool {
            name: tool,
            namespace,
            input,
            decision: if cancel {
                None
            } else {
                Some(if allowed {
                    ApprovalDecision::AllowOnce
                } else {
                    ApprovalDecision::DenyOnce
                })
            },
        });
        let read_first = if edit {
            format!(
                "先仅用 read_file 读取 {} 一次；读取成功后，",
                path.display()
            )
        } else {
            String::new()
        };
        prompt = format!(
            "{read_first}仅调用 {tool} 一次，完整参数为 {parameters}。等待用户审批；拒绝或取消后不要重试或调用其它工具。成功后只输出 {final_text}，不解释。禁止其它工具或文件。"
        );
    }
    Ok(Case {
        phase,
        path,
        before: before.map(String::into_bytes),
        after: after.map(String::into_bytes),
        tools,
        prompt,
        final_text,
        outcome: if allowed {
            TurnOutcome::Completed
        } else {
            TurnOutcome::Cancelled
        },
        cancellation_category: if cancel {
            Some("MidTurnAbort")
        } else if !allowed {
            Some("PermissionRejected")
        } else {
            None
        },
    })
}

/// 取消保留审批前已有输出；以同回合原生事件顺序区分前言和工具后的新文本。
fn phase_response_verified(
    receipt: &super::live_tests::NativeResponseReceipt,
    replay: &Value,
    turn: &str,
    output: &str,
    calls: &[String],
    case: &Case,
) -> bool {
    if receipt.full_output != output {
        return false;
    }
    match &case.outcome {
        TurnOutcome::Completed => receipt.final_response.trim() == case.final_text,
        TurnOutcome::Cancelled => {
            let (Some(call), Some(updates)) = (calls.last(), replay["updates"].as_array()) else {
                return false;
            };
            let mut requested = false;
            for row in updates {
                let params = &row["params"];
                let update = &params["update"];
                if row["method"] != "session/update" || params["_meta"]["promptId"] != turn {
                    continue;
                }
                if update["sessionUpdate"] == "tool_call" && update["toolCallId"] == *call {
                    requested = true;
                }
                if requested && update["sessionUpdate"] == "agent_message_chunk" {
                    return false;
                }
            }
            requested
        }
        TurnOutcome::Failed { .. } => false,
    }
}

fn native_tools_verified(replay: &Value, turn: &str, calls: &[String], case: &Case) -> bool {
    let Some(updates) = replay["updates"].as_array() else {
        return false;
    };
    if calls.len() != case.tools.len() {
        return false;
    }
    // 真实待审批取消只有回合取消水位，工具行没有 status；仅接受此已观察形状。
    let pending_approval_cancel = case.phase == "pending_cancel"
        && case.outcome == TurnOutcome::Cancelled
        && case.cancellation_category == Some("MidTurnAbort")
        && case.tools.len() == 1
        && case.tools[0].decision.is_none();
    let mut statusless_cancel = pending_approval_cancel;
    let mut seen = HashSet::new();
    let mut updated = HashSet::new();
    let mut terminal = HashSet::new();
    let mut completions = 0;
    for row in updates {
        let params = &row["params"];
        let update = &params["update"];
        if row["method"] == "_x.ai/session/update"
            && update["sessionUpdate"] == "turn_completed"
            && update["prompt_id"] == turn
        {
            if let Some(category) = case.cancellation_category {
                if params["_meta"]["cancellationCategory"] != category
                    || update["stop_reason"] != "cancelled"
                {
                    return false;
                }
            } else if update["stop_reason"] != "end_turn" {
                return false;
            }
            completions += 1;
        }
        if row["method"] != "session/update" || params["_meta"]["promptId"] != turn {
            continue;
        }
        if pending_approval_cancel
            && !seen.is_empty()
            && update["sessionUpdate"] == "agent_message_chunk"
        {
            return false;
        }
        if !matches!(
            update["sessionUpdate"].as_str(),
            Some("tool_call" | "tool_call_update")
        ) {
            continue;
        }
        let Some(id) = update["toolCallId"].as_str() else {
            return false;
        };
        let Some(index) = calls.iter().position(|call| call == id) else {
            return false;
        };
        if completions != 0 {
            return false;
        }
        let expected = &case.tools[index];
        if update["sessionUpdate"] == "tool_call" {
            seen.insert(id.to_owned());
        } else {
            updated.insert(id.to_owned());
        }
        // null、pending 和 in_progress 均不是本次原生观察到的「字段缺失」。
        statusless_cancel &= update.get("status").is_none();
        if statusless_cancel {
            let path = expected.input["file_path"].as_str();
            let content = expected.input["content"].as_str();
            let old_text = std::str::from_utf8(case.before.as_deref().unwrap_or_default()).ok();
            let preview = json!([{"type":"diff","path":path,"oldText":old_text,"newText":content}]);
            // 待审批 diff 是提议变更，不是执行输出；只允许与原始写入参数完全一致的预览。
            statusless_cancel = expected.name == "write"
                && expected.namespace == "opencode"
                && expected.input["variant"] == "Write"
                && path.is_some()
                && path == case.path.to_str()
                && content.is_some()
                && old_text.is_some()
                && update.get("content").is_none_or(|value| {
                    value.as_array().is_some_and(|items| items.is_empty()) || value == &preview
                })
                && update
                    .get("locations")
                    .is_none_or(|value| value == &json!([{"path":path}]))
                && update.as_object().is_some_and(|fields| {
                    fields.keys().all(|key| {
                        matches!(
                            key.as_str(),
                            "sessionUpdate"
                                | "toolCallId"
                                | "title"
                                | "rawInput"
                                | "_meta"
                                | "kind"
                                | "content"
                                | "locations"
                        )
                    })
                });
        }
        if let Some(name) = update["_meta"]["x.ai/tool"].get("name")
            && name != expected.name
        {
            return false;
        }
        if expected.decision != Some(ApprovalDecision::AllowOnce)
            && update.get("rawOutput").is_some()
        {
            return false;
        }
        match update["status"].as_str() {
            Some("completed") if expected.decision == Some(ApprovalDecision::AllowOnce) => {
                terminal.insert(id.to_owned());
            }
            Some("failed" | "cancelled")
                if expected.decision != Some(ApprovalDecision::AllowOnce) =>
            {
                terminal.insert(id.to_owned());
            }
            Some("pending" | "in_progress") | None => {}
            Some(_) => return false,
        }
    }
    // 此分支验证待审批调用由原生回合取消闭合，不补造 failed/cancelled 工具状态。
    completions == 1
        && seen.len() == calls.len()
        && (terminal.len() == calls.len() || (statusless_cancel && updated.len() == calls.len()))
}

fn catalogue_verified(catalog: &Value) -> bool {
    let Some(rows) = catalog.as_array() else {
        return false;
    };
    if rows.len() != 3 {
        return false;
    }
    let mut names = HashSet::new();
    for row in rows {
        let function = &row["function"];
        let Some(name) = function["name"].as_str() else {
            return false;
        };
        if !names.insert(name) {
            return false;
        }
        let schema = &function["parameters"];
        let (required, properties): (&[&str], &[&str]) = match name {
            "read_file" => (
                &["target_file"],
                &["target_file", "offset", "limit", "pages", "format"],
            ),
            "write" => (&["file_path", "content"], &["file_path", "content"]),
            "search_replace" => (
                &["file_path", "old_string", "new_string"],
                &["file_path", "old_string", "new_string", "replace_all"],
            ),
            _ => return false,
        };
        if schema["type"] != "object"
            || schema["required"] != json!(required)
            || schema["properties"].as_object().is_none_or(|values| {
                values.len() != properties.len()
                    || properties.iter().any(|key| !values.contains_key(*key))
            })
            || required
                .iter()
                .any(|key| schema["properties"][*key]["type"] != "string")
        {
            return false;
        }
        if name == "read_file"
            && (schema["properties"]["offset"]["type"] != "integer"
                || schema["properties"]["offset"]["default"] != 1
                || schema["properties"]["limit"]["type"] != "integer"
                || schema["properties"]["pages"]["type"] != json!(["string", "null"])
                || schema["properties"]["format"]["type"] != json!(["string", "null"]))
        {
            return false;
        }
        if name == "search_replace"
            && (schema["properties"]["replace_all"]["type"] != "boolean"
                || schema["properties"]["replace_all"]["default"] != false)
        {
            return false;
        }
    }
    true
}

fn persisted_catalog(state: &Path, profile: &Value, session: &str) -> Result<Value, String> {
    let storage = profile["storageId"].as_str().ok_or("固定存储身份缺失")?;
    Uuid::parse_str(storage).map_err(|_| "固定存储身份非法")?;
    Uuid::parse_str(session).map_err(|_| "原生会话身份非法")?;
    let base = state
        .join("grok-managed")
        .join(storage)
        .join("grok/sessions");
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&base).map_err(|_| "原生目录尚未落盘")? {
        let entry = entry.map_err(|_| "原生目录读取失败")?;
        let path = entry.path().join(session).join("tool_definitions.json");
        if path.is_file() {
            candidates.push(path);
        }
    }
    if candidates.len() != 1 {
        return Err("原生工具目录不唯一".into());
    }
    let path = &candidates[0];
    if path
        .symlink_metadata()
        .map_err(|_| "原生目录属性不可读")?
        .file_type()
        .is_symlink()
        || !path
            .canonicalize()
            .map_err(|_| "原生目录路径不可读")?
            .starts_with(base.canonicalize().map_err(|_| "原生目录根不可读")?)
    {
        return Err("原生工具目录越界".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|_| "原生工具目录读取失败")?)
        .map_err(|_| "原生工具目录无效".into())
}

async fn phase(
    options: SessionOptions,
    case: &Case,
    file: &mut File,
) -> Result<(String, super::super::permissions::GrokCreationPolicyV1), String> {
    let generation = options.generation;
    let state = options.state_dir.clone();
    let project = options.cwd.clone();
    let expected_profile = options.grok_profile.clone();
    let expected_session = match &options.target {
        SessionTarget::New => None,
        SessionTarget::Resume { native_session_id } => Some(native_session_id.clone()),
    };
    if file_bytes(&case.path)? != case.before {
        return Err("阶段开始前文件字节已改变".into());
    }
    validate_options(&options).map_err(|_| "固定文件策略参数无效")?;
    let mut protocol = GrokProtocol::new(options);
    let histories = Arc::new(Mutex::new(HashMap::new()));
    protocol.verified_final_histories_for_live = Some(histories.clone());
    let connection = connect_protocol(protocol);
    let controller = connection.controller;
    let mut events = connection.events;
    let mut task = tokio::spawn(connection.task);
    let input_id = Uuid::new_v4();
    let mut session = None;
    let mut profile = None;
    let mut turn = None;
    let mut calls = Vec::new();
    let mut approvals = HashMap::new();
    let mut resolved = HashSet::new();
    let mut cancelled = HashSet::new();
    let mut ready = false;
    let mut submitted = 0;
    let mut accepted = 0;
    let mut no_replay = false;
    let mut hook_absent = false;
    let mut terminal = false;
    let mut final_verified = false;
    let mut native_catalog = false;
    let mut native_outcome = None;
    let mut final_hash = None;
    let mut approval_diagnostic = None;
    let mut stage = "connect";
    let deadline = tokio::time::Instant::now() + Duration::from_secs(110);
    let result: Result<(), String> = async {
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .map_err(|_| "文件策略阶段超时")?
                .ok_or("生产事件流提前关闭")?;
            if event.generation != generation {
                return Err("连接代次不匹配".into());
            }
            if let Some(id) = event.native_session_id {
                if Uuid::parse_str(&id).is_err()
                    || session.as_ref().is_some_and(|prior| prior != &id)
                    || expected_session
                        .as_ref()
                        .is_some_and(|expected| expected != &id)
                {
                    return Err("原生会话身份改变".into());
                }
                session = Some(id);
            }
            match event.kind {
                RuntimeEventKind::SessionReady {
                    effective_permissions,
                    verified_cli_version,
                } => {
                    if env::var("INFINISHELL_GROK_FIXED_VERSION")
                        .ok()
                        .as_deref()
                        .is_some_and(|expected| verified_cli_version.as_deref() != Some(expected))
                    {
                        return Err("固定策略原生版本回执不匹配".into());
                    }
                    if ready
                        || session.is_none()
                        || effective_permissions["appCreationPolicyApplied"] != true
                        || effective_permissions["permissionEnforcementVerified"] != false
                        || effective_permissions["requestedPolicy"] != "GrokRestrictedFilesV1"
                    {
                        return Err("固定文件策略未就绪".into());
                    }
                    let observed: super::super::permissions::GrokCreationPolicyV1 =
                        serde_json::from_value(
                            effective_permissions["grokCreationPolicyV1"].clone(),
                        )
                        .map_err(|_| "缺少固定文件策略快照")?;
                    observed.validate().map_err(|_| "固定文件策略快照无效")?;
                    if observed.permission_policy() != PermissionPolicy::GrokRestrictedFilesV1
                        || expected_profile
                            .as_ref()
                            .is_some_and(|expected| expected != &observed)
                    {
                        return Err("冷恢复改变固定文件策略".into());
                    }
                    profile = Some(observed);
                    ready = true;
                    stage = "ready";
                    hook_absent = !project.join(".project-hook-ran").exists();
                    if !hook_absent {
                        return Err("项目 hook 被执行".into());
                    }
                    match tokio::time::timeout(Duration::from_millis(750), events.recv()).await {
                        Err(_) => no_replay = true,
                        Ok(_) => return Err("显式输入前出现业务事件".into()),
                    }
                    if file_bytes(&case.path)? != case.before {
                        return Err("显式输入前文件改变".into());
                    }
                    submitted += 1;
                    stage = "submitted";
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: input_id,
                            action: RuntimeAction::Submit {
                                input: vec![InputContent::Text(case.prompt.clone())],
                            },
                        })
                        .await
                        .map_err(|_| "输入写入失败")?;
                }
                RuntimeEventKind::MessageAccepted {
                    message_id,
                    turn_id,
                } => {
                    if message_id != input_id || accepted != 0 || !ready || submitted != 1 {
                        return Err("输入接收身份不匹配".into());
                    }
                    accepted += 1;
                    if let Some(id) = turn_id {
                        turn = Some(id);
                    }
                }
                RuntimeEventKind::TurnStarted { turn_id } => {
                    if !ready || turn.as_ref().is_some_and(|prior| prior != &turn_id) {
                        return Err("回合身份改变".into());
                    }
                    turn = Some(turn_id);
                }
                RuntimeEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    method,
                    details,
                } => {
                    let expected = case.tools.get(calls.len());
                    let call = details["toolCall"]["toolCallId"].as_str();
                    let identity_matches = turn.as_deref() == Some(turn_id.as_str())
                        && method == "session/request_permission"
                        && call.is_some_and(|id| !calls.iter().any(|old| old == id))
                        && !approvals.contains_key(&approval_id);
                    let exact = identity_matches
                        && expected.is_some_and(|tool| {
                            session
                                .as_deref()
                                .is_some_and(|id| tool.exact_permission(&details, id))
                        });
                    stage = "approval_requested";
                    if !exact {
                        approval_diagnostic = Some(permission_diagnostic(
                            expected,
                            &details,
                            session.as_deref(),
                            identity_matches,
                        ));
                    }
                    let action = if exact && expected.unwrap().decision.is_none() {
                        RuntimeAction::Interrupt {
                            turn_id: turn_id.clone(),
                        }
                    } else {
                        RuntimeAction::RespondApproval {
                            approval_id: approval_id.clone(),
                            decision: if exact {
                                expected.unwrap().decision.unwrap()
                            } else {
                                ApprovalDecision::DenyOnce
                            },
                        }
                    };
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: Uuid::new_v4(),
                            action,
                        })
                        .await
                        .map_err(|_| "审批或取消写入失败")?;
                    if !exact {
                        return Err("未知工具元信息、输入结构或重复审批".into());
                    }
                    approvals.insert(approval_id, expected.unwrap().decision);
                    calls.push(call.unwrap().to_owned());
                    stage = "approval_requested";
                }
                RuntimeEventKind::ApprovalResolved {
                    approval_id,
                    decision,
                } => {
                    if approvals.get(&approval_id) != Some(&Some(decision))
                        || !resolved.insert(approval_id)
                    {
                        return Err("审批写入回执不匹配".into());
                    }
                    stage = "approval_resolved";
                }
                RuntimeEventKind::ApprovalCancelled { approval_id } => {
                    if approvals.get(&approval_id) != Some(&None) || !cancelled.insert(approval_id)
                    {
                        return Err("非预期审批取消".into());
                    }
                }
                RuntimeEventKind::TurnFinished {
                    turn_id,
                    outcome,
                    output,
                } => {
                    stage = "turn_finished";
                    native_outcome = Some(match &outcome {
                        TurnOutcome::Completed => "Completed",
                        TurnOutcome::Cancelled => "Cancelled",
                        TurnOutcome::Failed { .. } => "Failed",
                    });
                    if turn.as_deref() != Some(turn_id.as_str())
                        || outcome != case.outcome
                        || accepted != 1
                        || calls.len() != case.tools.len()
                        || resolved.len()
                            != case
                                .tools
                                .iter()
                                .filter(|tool| tool.decision.is_some())
                                .count()
                        || cancelled.len()
                            != case
                                .tools
                                .iter()
                                .filter(|tool| tool.decision.is_none())
                                .count()
                    {
                        return Err("原生终态或审批回执不完整".into());
                    }
                    let histories = histories.lock().expect("历史验收锁未损坏");
                    let snapshot = histories.get(&turn_id).ok_or("缺少已验证原生历史")?;
                    let receipt = super::live_tests::final_response_receipt(
                        snapshot,
                        session.as_deref().ok_or("会话身份缺失")?,
                        &turn_id,
                        &outcome,
                    )?;
                    // 历史回执与工具闭合分别记录，后续失败不能掩盖已通过的历史验证。
                    final_hash = Some(digest(receipt.final_response.trim().as_bytes()));
                    final_verified = true;
                    let response_verified = phase_response_verified(
                        &receipt,
                        &snapshot.replay,
                        &turn_id,
                        &output,
                        &calls,
                        case,
                    );
                    terminal = native_tools_verified(&snapshot.replay, &turn_id, &calls, case);
                    eprintln!(
                        "GROK_FILES_FINAL_DIAGNOSTIC {}",
                        json!({"phase":case.phase,"final_history_verified":final_verified,
                            "phase_response_verified":response_verified,
                            "native_tools_verified":terminal})
                    );
                    if !response_verified {
                        return Err("原生最终结果不匹配".into());
                    }
                    if !terminal {
                        return Err("原生工具或取消水位不匹配".into());
                    }
                    let profile_value =
                        serde_json::to_value(profile.as_ref().ok_or("策略快照缺失")?)
                            .map_err(|_| "策略序列化失败")?;
                    native_catalog = catalogue_verified(&persisted_catalog(
                        &state,
                        &profile_value,
                        session.as_deref().unwrap(),
                    )?);
                    if !native_catalog {
                        return Err("真实原生工具目录或参数 schema 不匹配".into());
                    }
                    if file_bytes(&case.path)? != case.after {
                        return Err("实际文件字节不匹配".into());
                    }
                    stage = "verified";
                    return Ok(());
                }
                RuntimeEventKind::TextDelta { turn_id, .. }
                | RuntimeEventKind::Progress { turn_id, .. } => {
                    if turn.as_deref() != Some(turn_id.as_str()) {
                        return Err("收到旧回合事件".into());
                    }
                }
                RuntimeEventKind::CommandDispatched { .. } => {}
                RuntimeEventKind::LocalToolCancelled { .. }
                | RuntimeEventKind::LocalToolRequested { .. }
                | RuntimeEventKind::InputJoined { .. }
                | RuntimeEventKind::RequestFailed { .. }
                | RuntimeEventKind::Disconnected { .. } => {
                    return Err("生产协议提前结束或业务越界".into());
                }
            }
        }
    }
    .await;
    let _ = controller
        .send(RuntimeCommand {
            generation,
            message_id: Uuid::new_v4(),
            action: RuntimeAction::Shutdown,
        })
        .await;
    let joined = tokio::time::timeout(Duration::from_secs(30), &mut task).await;
    let transport_closed = matches!(&joined, Ok(Ok(Ok(()))));
    if joined.is_err() {
        task.abort();
        let _ = task.await;
    }
    let cleanup = managed_process::confirmed_exit(&state, generation)
        .ok()
        .flatten()
        .is_some_and(|receipt| receipt.cleanup_confirmed);
    let auth_removed = profile
        .as_ref()
        .and_then(|profile| serde_json::to_value(profile).ok())
        .and_then(|value| value["storageId"].as_str().map(str::to_owned))
        .is_some_and(|id| {
            !state
                .join("grok-managed")
                .join(id)
                .join("grok/auth.json")
                .exists()
        });
    let hook_after = !project.join(".project-hook-ran").exists();
    let observed = file_bytes(&case.path);
    let bytes_match = observed.as_ref().is_ok_and(|actual| actual == &case.after);
    let actual = observed.ok().flatten();
    let passed =
        result.is_ok() && cleanup && transport_closed && auth_removed && hook_after && bytes_match;
    record(
        file,
        json!({"event":"files_policy_phase","scope":SCOPE,"phase":case.phase,"passed":passed,
        "ready":ready,"same_saved_profile":expected_profile.is_none() || profile==expected_profile,
        "same_native_session":expected_session.is_none() || session==expected_session,"no_replay_before_input":no_replay,
        "hook_absent_at_ready":hook_absent,"hook_absent_after_shutdown":hook_after,"submitted":submitted,"accepted":accepted,
        "approvals":calls.len(),"approval_resolved":resolved.len(),"approval_cancelled":cancelled.len(),
        "final_history_verified":final_verified,"native_tool_terminal":terminal,"native_catalog_verified":native_catalog,
        "file_bytes_match":bytes_match,"file_before_sha256":case.before.as_ref().map(|bytes|digest(bytes)),"file_after_sha256":actual.as_ref().map(|bytes|digest(bytes)),
        "cleanup_confirmed":cleanup,"transport_closed":transport_closed,"managed_auth_removed":auth_removed,
        "final_sha256":final_hash,"native_session_sha256":session.as_deref().map(|id|digest(id.as_bytes())),
        "native_outcome":native_outcome,"approval_diagnostic":approval_diagnostic,
        "failure_stage":if passed {None} else {Some(stage)}}),
    )?;
    if !passed {
        return Err("固定文件工具阶段验收失败".into());
    }
    Ok((session.ok_or("会话缺失")?, profile.ok_or("策略缺失")?))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由隔离运行器启动；最多六次真实模型输入，会消耗已授权额度"]
async fn authenticated_files_policy_six_input_lifecycle() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("需要独立运行器"))
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-live-probe")).unwrap(),
        "isolated Grok Rust adapter verification\n"
    );
    assert_eq!(
        PathBuf::from(env::var_os("GROK_HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("home/.grok")
    );
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_AUTH_MODE").unwrap(),
        "official-cached-token"
    );
    assert_eq!(
        env::var("INFINISHELL_GROK_FILES_INPUT_BUDGET").unwrap(),
        "6"
    );
    let artifact = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").unwrap());
    assert_eq!(artifact, root.join("private-evidence.ndjson"));
    let mut file = File::create(artifact).unwrap();
    record(&mut file,json!({"event":"files_policy_started","scope":SCOPE,"max_native_inputs":6,"production_prepare":true,"test_argv_override":false,"credential_values_recorded":false})).unwrap();
    let mut options = SessionOptions {
        executable: PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").unwrap()),
        cwd: root.join("project"),
        state_dir: root.join("state"),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::GrokRestrictedFilesV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    };
    let result: Result<(), String> = async {
        for index in 0..6 {
            let (id, profile) = phase(options.clone(), &case(&root, index)?, &mut file).await?;
            let saved = root.join("saved-session.json");
            fs::write(
                &saved,
                serde_json::to_vec(&json!({"native_session_id":id,"profile":profile})).unwrap(),
            )
            .map_err(|_| "会话保存失败")?;
            let persisted: Value =
                serde_json::from_slice(&fs::read(saved).map_err(|_| "会话读取失败")?)
                    .map_err(|_| "会话解析失败")?;
            options.generation = Uuid::new_v4();
            options.target = SessionTarget::Resume {
                native_session_id: persisted["native_session_id"]
                    .as_str()
                    .ok_or("会话身份缺失")?
                    .to_owned(),
            };
            options.grok_profile = Some(
                serde_json::from_value(persisted["profile"].clone()).map_err(|_| "策略快照缺失")?,
            );
        }
        Ok(())
    }
    .await;
    record(&mut file,json!({"event":"files_policy_finished","scope":SCOPE,"passed":result.is_ok(),"system_managed_policies_apply":true,
        "native_effective_policy_verified":false,"filesystem_sandbox_verified":false,"app_restart_verified":false,"coordinator_verified":false,"spawn_verified":false,
        "shell_build_tools_verified":false,"full_cli_parity_acceptance_passed":false})).unwrap();
    assert!(result.is_ok(), "固定文件工具验收失败；只查看安全证据投影");
}

#[test]
fn exact_file_permission_rejects_unobserved_variants_and_changed_bytes() {
    let expected = ExpectedTool {
        name: "write",
        namespace: "opencode",
        input: json!({"variant":"Write","file_path":"/synthetic/write.txt","content":"精确字节\n"}),
        decision: Some(ApprovalDecision::AllowOnce),
    };
    let details = json!({"sessionId":"session","toolCall":{"kind":"edit","_meta":{"x.ai/tool":{"version":1,"name":"write","namespace":"opencode","read_only":false}},"rawInput":expected.input}});
    assert!(expected.exact_permission(&details, "session"));
    for variant in ["WriteFile", "SearchReplace", "unknown"] {
        let mut changed = details.clone();
        changed["toolCall"]["rawInput"]["variant"] = json!(variant);
        assert!(!expected.exact_permission(&changed, "session"));
    }
    let mut changed = details.clone();
    changed["toolCall"]["rawInput"]["content"] = json!("其它字节");
    assert!(!expected.exact_permission(&changed, "session"));
    changed = details.clone();
    changed["toolCall"]["_meta"]["x.ai/tool"]["namespace"] = json!("grok_build");
    assert!(!expected.exact_permission(&changed, "session"));
    let edit = ExpectedTool {
        name: "search_replace",
        namespace: "grok_build",
        input: json!({"variant":"SearchReplace","file_path":"/synthetic/edit.txt","old_string":"old\n","new_string":"new\n"}),
        decision: Some(ApprovalDecision::DenyOnce),
    };
    let mut details = json!({"sessionId":"session","toolCall":{"kind":"edit","_meta":{"x.ai/tool":{"version":1,"name":"search_replace","namespace":"grok_build","read_only":false}},"rawInput":edit.input}});
    assert!(edit.exact_permission(&details, "session"));
    details["toolCall"]["rawInput"]["replace_all"] = json!(false);
    assert!(edit.exact_permission(&details, "session"));
    details["toolCall"]["rawInput"]["replace_all"] = json!(true);
    assert!(!edit.exact_permission(&details, "session"));
    details["toolCall"]["rawInput"]["replace_all"] = json!(null);
    assert!(!edit.exact_permission(&details, "session"));
}

#[test]
fn native_catalog_requires_the_actual_three_tool_schemas() {
    let rows = json!([
        {"function":{"name":"read_file","parameters":{"required":["target_file"],"type":"object","properties":{"target_file":{"type":"string"},"offset":{"type":"integer","default":1},"limit":{"type":"integer"},"pages":{"type":["string","null"]},"format":{"type":["string","null"]}}}}},
        {"function":{"name":"write","parameters":{"required":["file_path","content"],"type":"object","properties":{"file_path":{"type":"string"},"content":{"type":"string"}}}}},
        {"function":{"name":"search_replace","parameters":{"required":["file_path","old_string","new_string"],"type":"object","properties":{"file_path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean","default":false}}}}}
    ]);
    assert!(catalogue_verified(&rows));
    let mut unexpected = rows.clone();
    unexpected[2]["function"]["name"] = json!("edit_file");
    assert!(!catalogue_verified(&unexpected));
    let mut changed = rows.clone();
    changed[2]["function"]["parameters"]["properties"]["replace_all"]["default"] = json!(true);
    assert!(!catalogue_verified(&changed));
    let mut missing = rows.clone();
    missing.as_array_mut().unwrap().pop();
    assert!(!catalogue_verified(&missing));
    let mut extra = rows.clone();
    extra
        .as_array_mut()
        .unwrap()
        .push(json!({"function":{"name":"run_terminal_command"}}));
    assert!(!catalogue_verified(&extra));
}

#[test]
fn cancelled_tool_receipt_cannot_include_execution_or_wrong_cancel_reason() {
    let case = Case {
        phase: "pending_cancel",
        path: PathBuf::from("/synthetic/cancel.txt"),
        before: None,
        after: None,
        tools: vec![ExpectedTool {
            name: "write",
            namespace: "opencode",
            input: json!({}),
            decision: None,
        }],
        prompt: String::new(),
        final_text: String::new(),
        outcome: TurnOutcome::Cancelled,
        cancellation_category: Some("MidTurnAbort"),
    };
    let calls = vec!["call".to_owned()];
    let replay = json!({"updates":[
        {"method":"session/update","params":{"_meta":{"promptId":"turn"},"update":{"sessionUpdate":"tool_call","toolCallId":"call"}}},
        {"method":"session/update","params":{"_meta":{"promptId":"turn"},"update":{"sessionUpdate":"tool_call_update","toolCallId":"call","status":"failed"}}},
        {"method":"_x.ai/session/update","params":{"_meta":{"cancellationCategory":"MidTurnAbort"},"update":{"sessionUpdate":"turn_completed","prompt_id":"turn","stop_reason":"cancelled"}}}
    ]});
    assert!(native_tools_verified(&replay, "turn", &calls, &case));
    let mut executed = replay.clone();
    executed["updates"][1]["params"]["update"]["rawOutput"] = json!({});
    assert!(!native_tools_verified(&executed, "turn", &calls, &case));
    let mut wrong_reason = replay.clone();
    wrong_reason["updates"][2]["params"]["_meta"]["cancellationCategory"] =
        json!("PermissionRejected");
    assert!(!native_tools_verified(&wrong_reason, "turn", &calls, &case));
    let mut extra = replay.clone();
    extra["updates"].as_array_mut().unwrap().push(json!({"method":"session/update","params":{"_meta":{"promptId":"turn"},"update":{"sessionUpdate":"tool_call","toolCallId":"other"}}}));
    assert!(!native_tools_verified(&extra, "turn", &calls, &case));
    assert!(!native_tools_verified(&replay, "old-turn", &calls, &case));
}

#[test]
fn unknown_approval_diagnostic_reveals_only_comparisons_counts_and_types() {
    let expected = ExpectedTool {
        name: "search_replace",
        namespace: "grok_build",
        input: json!({"variant":"SearchReplace","file_path":"/synthetic/edit.txt","old_string":"before","new_string":"after"}),
        decision: Some(ApprovalDecision::AllowOnce),
    };
    let private = "OFFLINE_PRIVATE_APPROVAL_CANARY";
    let details = json!({"sessionId":"session","toolCall":{"kind":"edit",
        "_meta":{"x.ai/tool":{"name":private,"namespace":private,"version":1,"read_only":false}},
        "rawInput":{"variant":private,"file_path":private,"old_string":private,"unknown_private_key":private}}});
    let diagnostic = permission_diagnostic(Some(&expected), &details, Some("session"), true);
    assert_eq!(diagnostic["name_matches"], false);
    assert_eq!(diagnostic["namespace_matches"], false);
    assert_eq!(diagnostic["variant_matches"], false);
    assert_eq!(diagnostic["parameters_match"], false);
    assert_eq!(diagnostic["missing_parameter_count"], 1);
    assert_eq!(diagnostic["extra_parameter_count"], 1);
    assert_eq!(diagnostic["parameter_types"]["new_string"], "absent");
    assert_eq!(diagnostic["parameter_types"]["variant"], "string");
    assert_eq!(diagnostic["version_matches"], true);
    assert!(!diagnostic.to_string().contains(private));
    assert!(!diagnostic.to_string().contains("unknown_private_key"));
    assert!(!expected.exact_permission(&details, "session"));
}

#[test]
fn missing_tool_or_malformed_parameters_remain_rejected_and_diagnosable() {
    let details = json!({"toolCall":{"rawInput":"OFFLINE_PRIVATE_PARAMETERS"}});
    let diagnostic = permission_diagnostic(None, &details, None, false);
    assert_eq!(diagnostic["expected_tool_present"], false);
    assert_eq!(diagnostic["request_identity_matches"], false);
    assert_eq!(diagnostic["raw_input_type"], "string");
    assert_eq!(diagnostic["name_type"], "absent");
    assert!(
        !diagnostic
            .to_string()
            .contains("OFFLINE_PRIVATE_PARAMETERS")
    );
}

fn cancelled_response_fixture(
    decision: Option<ApprovalDecision>,
    category: &'static str,
) -> (Case, super::live_tests::NativeFinalHistory, Vec<String>) {
    let case = Case {
        phase: if decision.is_some() {
            "write_deny"
        } else {
            "pending_cancel"
        },
        path: PathBuf::from("/synthetic/unchanged.txt"),
        before: None,
        after: None,
        tools: vec![ExpectedTool {
            name: "write",
            namespace: "opencode",
            input: json!({"variant":"Write","file_path":"/synthetic/unchanged.txt","content":"合成待写入内容\n"}),
            decision,
        }],
        prompt: String::new(),
        final_text: String::new(),
        outcome: TurnOutcome::Cancelled,
        cancellation_category: Some(category),
    };
    let rows = json!([
        {"method":"session/update","params":{"sessionId":"session","_meta":{"eventId":"session-1","promptId":"old-turn","streamStartMs":9000},"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"此前成功结果"}}}},
        {"method":"_x.ai/session/update","params":{"sessionId":"session","_meta":{"eventId":"session-2"},"update":{"sessionUpdate":"turn_completed","prompt_id":"old-turn","stop_reason":"end_turn"}}},
        {"method":"session/update","params":{"sessionId":"session","_meta":{"eventId":"session-3","promptId":"turn","streamStartMs":1000},"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"请求写入，等待审批。"}}}},
        {"method":"session/update","params":{"sessionId":"session","_meta":{"eventId":"session-4","promptId":"turn","streamStartMs":1000},"update":{"sessionUpdate":"tool_call","toolCallId":"call"}}},
        {"method":"session/update","params":{"sessionId":"session","_meta":{"eventId":"session-5","promptId":"turn","streamStartMs":1000},"update":{"sessionUpdate":"tool_call_update","toolCallId":"call","status":"failed"}}},
        {"method":"_x.ai/session/update","params":{"sessionId":"session","_meta":{"eventId":"session-6","cancellationCategory":category},"update":{"sessionUpdate":"turn_completed","prompt_id":"turn","stop_reason":"cancelled"}}}
    ]);
    let snapshot = super::live_tests::NativeFinalHistory {
        session_id: "session".into(),
        turn_id: "turn".into(),
        completion_watermark: "session-6".into(),
        replay: json!({"updates":rows,"totalCount":6,"hasMore":false,"lastEventId":"session-6"}),
    };
    (case, snapshot, vec!["call".to_owned()])
}

fn assert_cancelled_response_boundary(decision: Option<ApprovalDecision>, category: &'static str) {
    let (case, snapshot, calls) = cancelled_response_fixture(decision, category);
    let receipt =
        super::live_tests::final_response_receipt(&snapshot, "session", "turn", &case.outcome)
            .unwrap();
    assert_eq!(receipt.final_response, "请求写入，等待审批。");
    assert_eq!(receipt.full_output, receipt.final_response);
    assert!(phase_response_verified(
        &receipt,
        &snapshot.replay,
        "turn",
        &receipt.full_output,
        &calls,
        &case
    ));
    assert!(native_tools_verified(
        &snapshot.replay,
        "turn",
        &calls,
        &case
    ));
    assert!(!phase_response_verified(
        &receipt,
        &snapshot.replay,
        "turn",
        "",
        &calls,
        &case
    ));
    assert!(!phase_response_verified(
        &receipt,
        &snapshot.replay,
        "turn",
        &receipt.full_output,
        &[],
        &case
    ));
    let mut after_tool = snapshot.replay.clone();
    // 事件顺序才是依据；复用或降低 streamStartMs 都不能放过工具后的文本。
    for stream in [1000, 1] {
        after_tool["updates"][4] = json!({"method":"session/update","params":{"_meta":{"promptId":"turn","streamStartMs":stream},"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"拒绝后的新文本"}}}});
        assert!(!phase_response_verified(
            &receipt,
            &after_tool,
            "turn",
            &receipt.full_output,
            &calls,
            &case
        ));
        after_tool["updates"][4]["params"]["_meta"]["promptId"] = json!("old-turn");
        assert!(phase_response_verified(
            &receipt,
            &after_tool,
            "turn",
            &receipt.full_output,
            &calls,
            &case
        ));
    }
    let mut executed = snapshot.replay.clone();
    executed["updates"][4]["params"]["update"]["rawOutput"] = json!({});
    assert!(!native_tools_verified(&executed, "turn", &calls, &case));
    let mut wrong_reason = snapshot.replay.clone();
    wrong_reason["updates"][5]["params"]["_meta"]["cancellationCategory"] = json!("Other");
    assert!(!native_tools_verified(&wrong_reason, "turn", &calls, &case));
}

#[test]
fn denied_write_keeps_only_current_turn_text_before_its_tool() {
    assert_cancelled_response_boundary(Some(ApprovalDecision::DenyOnce), "PermissionRejected");
}

#[test]
fn user_cancel_keeps_only_current_turn_text_before_its_tool() {
    assert_cancelled_response_boundary(None, "MidTurnAbort");
}

#[test]
fn pending_cancel_accepts_statusless_tool_only_at_exact_native_cancel_watermark() {
    let (case, mut snapshot, calls) = cancelled_response_fixture(None, "MidTurnAbort");
    snapshot.replay["updates"][4]["params"]["update"]
        .as_object_mut()
        .unwrap()
        .remove("status");
    let receipt =
        super::live_tests::final_response_receipt(&snapshot, "session", "turn", &case.outcome)
            .unwrap();
    assert!(phase_response_verified(
        &receipt,
        &snapshot.replay,
        "turn",
        &receipt.full_output,
        &calls,
        &case
    ));
    assert!(native_tools_verified(
        &snapshot.replay,
        "turn",
        &calls,
        &case
    ));

    let mut denied = case;
    denied.phase = "write_deny";
    denied.tools[0].decision = Some(ApprovalDecision::DenyOnce);
    denied.cancellation_category = Some("PermissionRejected");
    snapshot.replay["updates"][5]["params"]["_meta"]["cancellationCategory"] =
        json!("PermissionRejected");
    assert!(!native_tools_verified(
        &snapshot.replay,
        "turn",
        &calls,
        &denied
    ));
}

#[test]
fn statusless_pending_cancel_rejects_changed_identity_output_and_terminal_shape() {
    let (case, mut snapshot, calls) = cancelled_response_fixture(None, "MidTurnAbort");
    snapshot.replay["updates"][4]["params"]["update"]
        .as_object_mut()
        .unwrap()
        .remove("status");
    let baseline = snapshot.replay;
    let mut rejected = Vec::new();
    for category in ["PermissionRejected", "Other"] {
        let mut replay = baseline.clone();
        replay["updates"][5]["params"]["_meta"]["cancellationCategory"] = json!(category);
        rejected.push(replay);
    }
    let mut old_turn = baseline.clone();
    old_turn["updates"][5]["params"]["update"]["prompt_id"] = json!("old-turn");
    rejected.push(old_turn);
    let mut old_tool = baseline.clone();
    old_tool["updates"][3]["params"]["_meta"]["promptId"] = json!("old-turn");
    rejected.push(old_tool);
    let mut extra_tool = baseline.clone();
    let mut extra = extra_tool["updates"][3].clone();
    extra["params"]["update"]["toolCallId"] = json!("unexpected-call");
    extra_tool["updates"]
        .as_array_mut()
        .unwrap()
        .insert(4, extra);
    rejected.push(extra_tool);
    for status in [
        json!(null),
        json!("pending"),
        json!("in_progress"),
        json!("completed"),
    ] {
        let mut replay = baseline.clone();
        replay["updates"][4]["params"]["update"]["status"] = status;
        rejected.push(replay);
    }
    let mut output = baseline.clone();
    output["updates"][4]["params"]["update"]["rawOutput"] = json!(null);
    rejected.push(output);
    let mut missing_update = baseline.clone();
    missing_update["updates"].as_array_mut().unwrap().remove(4);
    rejected.push(missing_update);
    let mut after_cancel = baseline.clone();
    let tool_update = after_cancel["updates"].as_array_mut().unwrap().remove(4);
    after_cancel["updates"]
        .as_array_mut()
        .unwrap()
        .push(tool_update);
    rejected.push(after_cancel);
    let mut post_tool_text = baseline.clone();
    post_tool_text["updates"].as_array_mut().unwrap().insert(
        5,
        json!({"method":"session/update","params":{"_meta":{"promptId":"turn"},
            "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"取消后新正文"}}}}),
    );
    rejected.push(post_tool_text);
    let mut post_cancel_text = baseline.clone();
    post_cancel_text["updates"].as_array_mut().unwrap().push(
        json!({"method":"session/update","params":{"_meta":{"promptId":"turn"},
            "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"水位后的新正文"}}}}),
    );
    rejected.push(post_cancel_text);
    for (index, replay) in rejected.into_iter().enumerate() {
        assert!(
            !native_tools_verified(&replay, "turn", &calls, &case),
            "负向取消夹具 {index} 未被拒绝"
        );
    }
    assert!(!native_tools_verified(
        &baseline,
        "turn",
        &["call".to_owned(), "unexpected-call".to_owned()],
        &case
    ));
}

#[test]
fn statusless_pending_cancel_accepts_only_exact_write_preview_without_output() {
    let (case, mut snapshot, calls) = cancelled_response_fixture(None, "MidTurnAbort");
    let update = &mut snapshot.replay["updates"][4]["params"]["update"];
    update.as_object_mut().unwrap().remove("status");
    let preview = json!([{"type":"diff","path":case.tools[0].input["file_path"],
        "oldText":"","newText":case.tools[0].input["content"]}]);
    update["locations"] = json!([{"path":case.tools[0].input["file_path"]}]);
    for content in [None, Some(json!([])), Some(preview)] {
        let update = &mut snapshot.replay["updates"][4]["params"]["update"];
        if let Some(content) = content {
            update["content"] = content;
        } else {
            update.as_object_mut().unwrap().remove("content");
        }
        assert!(native_tools_verified(
            &snapshot.replay,
            "turn",
            &calls,
            &case
        ));
    }
    // 已有明确工具终态的分支保持原义，不套用待审批预览的严格形状。
    let update = &mut snapshot.replay["updates"][4]["params"]["update"];
    update["status"] = json!("failed");
    update["content"] = json!([{"type":"content","content":{"type":"text","text":"工具错误"}}]);
    update["locations"] = json!([]);
    assert!(native_tools_verified(
        &snapshot.replay,
        "turn",
        &calls,
        &case
    ));
}

#[test]
fn statusless_pending_cancel_rejects_unknown_content_and_changed_write_preview() {
    let (case, mut snapshot, calls) = cancelled_response_fixture(None, "MidTurnAbort");
    let update = &mut snapshot.replay["updates"][4]["params"]["update"];
    update.as_object_mut().unwrap().remove("status");
    let preview = json!([{"type":"diff","path":case.tools[0].input["file_path"],
        "oldText":"","newText":case.tools[0].input["content"]}]);
    update["content"] = preview.clone();
    update["locations"] = json!([{"path":case.tools[0].input["file_path"]}]);
    let baseline = snapshot.replay;
    let mut invalid_content = vec![
        json!(null),
        json!({}),
        json!([{"type":"content","content":{"type":"text","text":"执行结果"}}]),
        json!([{"type":"terminal","terminalId":"terminal"}]),
        json!([{"type":"resource","resource":{"uri":"file:///synthetic/result"}}]),
        json!([preview[0], preview[0]]),
    ];
    for (key, value) in [
        ("type", json!("unknown")),
        ("path", json!("/synthetic/other.txt")),
        ("oldText", json!("不同旧内容")),
        ("newText", json!("不同新内容")),
        ("unknown", json!("额外字段")),
    ] {
        let mut content = preview.clone();
        content[0][key] = value;
        invalid_content.push(content);
    }
    let mut missing = preview.clone();
    missing[0].as_object_mut().unwrap().remove("oldText");
    invalid_content.push(missing);
    for (index, content) in invalid_content.into_iter().enumerate() {
        let mut replay = baseline.clone();
        replay["updates"][4]["params"]["update"]["content"] = content;
        assert!(
            !native_tools_verified(&replay, "turn", &calls, &case),
            "负向取消预览 {index} 未被拒绝"
        );
    }
    for locations in [
        json!(null),
        json!([]),
        json!([{"path":"/synthetic/other.txt"}]),
        json!([{"path":case.tools[0].input["file_path"],"line":1}]),
        json!([{"path":case.tools[0].input["file_path"]},{"path":case.tools[0].input["file_path"]}]),
    ] {
        let mut replay = baseline.clone();
        replay["updates"][4]["params"]["update"]["locations"] = locations;
        assert!(!native_tools_verified(&replay, "turn", &calls, &case));
    }
    for key in ["output", "result", "error", "structuredContent", "unknown"] {
        let mut replay = baseline.clone();
        replay["updates"][4]["params"]["update"][key] = json!({});
        assert!(!native_tools_verified(&replay, "turn", &calls, &case));
    }
    let mut initial_tool_output = baseline;
    initial_tool_output["updates"][3]["params"]["update"]["content"] = json!([{"type":"text"}]);
    assert!(!native_tools_verified(
        &initial_tool_output,
        "turn",
        &calls,
        &case
    ));
}

#[test]
fn completed_file_response_still_requires_exact_final_text_and_full_output() {
    let (mut case, snapshot, calls) = cancelled_response_fixture(None, "MidTurnAbort");
    case.outcome = TurnOutcome::Completed;
    case.final_text = "WRITE_ALLOW".into();
    let mut receipt = super::live_tests::NativeResponseReceipt {
        final_response: "WRITE_ALLOW\n".into(),
        stream_start_ms: Some(1000),
        completion_watermark: "session-6".into(),
        full_output: "审批前言\nWRITE_ALLOW\n".into(),
    };
    assert!(phase_response_verified(
        &receipt,
        &snapshot.replay,
        "turn",
        &receipt.full_output,
        &calls,
        &case
    ));
    assert!(!phase_response_verified(
        &receipt,
        &snapshot.replay,
        "turn",
        "WRITE_ALLOW\n",
        &calls,
        &case
    ));
    receipt.final_response = "审批前言 WRITE_ALLOW".into();
    assert!(!phase_response_verified(
        &receipt,
        &snapshot.replay,
        "turn",
        &receipt.full_output,
        &calls,
        &case
    ));
}
