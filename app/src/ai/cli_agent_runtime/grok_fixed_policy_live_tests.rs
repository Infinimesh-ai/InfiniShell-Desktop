//! 固定创建策略的真实根任务验收；不把测试进程沙箱算作产品能力。

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

const SCOPE: &str = "authenticated_fixed_policy_read_approval_and_cold_restore";

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .map_err(|_| "安全证据写入失败".into())
}

fn exact_read(details: &Value, project: &Path, expected: &Path, session: &str) -> bool {
    let Some(input) = details["toolCall"]["rawInput"].as_object() else {
        return false;
    };
    let path = input.get("target_file").and_then(Value::as_str);
    details["sessionId"] == session
        && details["toolCall"]["kind"] == "read"
        && details["toolCall"]["_meta"]["x.ai/tool"]["name"] == "read_file"
        && input.get("variant") == Some(&json!("ReadFile"))
        && path
            .is_some_and(|path| project.join(path).canonicalize().ok().as_deref() == Some(expected))
        && input.keys().all(|key| {
            matches!(
                key.as_str(),
                "variant" | "target_file" | "offset" | "limit" | "pages" | "format"
            )
        })
        && ["offset", "limit"].iter().all(|key| {
            input
                .get(*key)
                .is_none_or(|value| value.is_null() || value.as_u64() == Some(1))
        })
        && ["pages", "format"]
            .iter()
            .all(|key| input.get(*key).is_none_or(Value::is_null))
}

fn native_tool_terminal(replay: &Value, turn: &str, call: &str, allow: bool) -> bool {
    let Some(updates) = replay["updates"].as_array() else {
        return false;
    };
    let mut calls = HashSet::new();
    let mut terminal = false;
    for record in updates {
        let params = &record["params"];
        if record["method"] != "session/update" || params["_meta"]["promptId"] != turn {
            continue;
        }
        let update = &params["update"];
        if update["sessionUpdate"] == "tool_call" {
            let Some(id) = update["toolCallId"].as_str() else {
                return false;
            };
            calls.insert(id.to_owned());
        }
        if update["sessionUpdate"] == "tool_call_update" && update["toolCallId"] == call {
            terminal |= if allow {
                update["status"] == "completed"
            } else {
                matches!(update["status"].as_str(), Some("failed" | "cancelled"))
            };
        }
    }
    calls.len() == 1 && calls.contains(call) && terminal
}

// 拒绝后取消回合；保留工具调用前的说明，但不允许工具输出或之后的新模型文本。
fn denied_read_not_executed(replay: &Value, turn: &str, call: &str) -> bool {
    let Some(updates) = replay["updates"].as_array() else {
        return false;
    };
    let mut rejected = 0;
    let mut requested = false;
    for record in updates {
        let params = &record["params"];
        let update = &params["update"];
        if record["method"] == "session/update" && params["_meta"]["promptId"] == turn {
            if update["sessionUpdate"] == "tool_call" && update["toolCallId"] == call {
                requested = true;
            }
            if requested && update["sessionUpdate"] == "agent_message_chunk" {
                return false;
            }
        }
        if record["method"] == "session/update"
            && params["_meta"]["promptId"] == turn
            && update["toolCallId"] == call
            && (update["status"] == "completed" || update.get("rawOutput").is_some())
        {
            return false;
        }
        if record["method"] == "_x.ai/session/update"
            && update["sessionUpdate"] == "turn_completed"
            && update["prompt_id"] == turn
        {
            if update["stop_reason"] != "cancelled"
                || params["_meta"]["cancellationCategory"] != "PermissionRejected"
                || params["_meta"]["cancellationContext"]["tool_name"] != "read_file"
            {
                return false;
            }
            rejected += 1;
        }
    }
    rejected == 1 && native_tool_terminal(replay, turn, call, false)
}

async fn phase(
    options: SessionOptions,
    allow: bool,
    file: &mut File,
) -> Result<(String, super::super::permissions::GrokCreationPolicyV1), String> {
    let generation = options.generation;
    let state = options.state_dir.clone();
    let project = options.cwd.clone();
    let resumed = matches!(options.target, SessionTarget::Resume { .. });
    let expected_session = match &options.target {
        SessionTarget::New => None,
        SessionTarget::Resume { native_session_id } => Some(native_session_id.clone()),
    };
    let expected_profile = options.grok_profile.clone();
    let target = project.join(if allow { "allow.txt" } else { "deny.txt" });
    let allowed_token =
        fs::read_to_string(project.join("allow.txt")).map_err(|_| "缺少允许文件")?;
    let denied_token = fs::read_to_string(project.join("deny.txt")).map_err(|_| "缺少拒绝文件")?;
    let prompt = if allow {
        "只用 read_file 读取 allow.txt 一次，参数只含 target_file=allow.txt。不要调用其他工具。读取成功后只输出该文件唯一一行的原文，不解释。"
    } else {
        "只用 read_file 读取 deny.txt 一次，参数只含 target_file=deny.txt。等待用户审批；如果拒绝，不要重试或用其它方法读取。不要调用其他工具。"
    };
    validate_options(&options).map_err(|_| "固定生产参数验证失败")?;
    let mut protocol = GrokProtocol::new(options);
    let histories = Arc::new(Mutex::new(HashMap::new()));
    protocol.verified_final_histories_for_live = Some(histories.clone());
    let connection = connect_protocol(protocol);
    let controller = connection.controller;
    let mut events = connection.events;
    let mut task = tokio::spawn(connection.task);
    let input_id = Uuid::new_v4();
    let decision = if allow {
        ApprovalDecision::AllowOnce
    } else {
        ApprovalDecision::DenyOnce
    };
    let mut session = None;
    let mut profile = None;
    let mut turn = None;
    let mut approval = None;
    let mut call = None;
    let mut ready = false;
    let mut submitted = 0;
    let mut accepted = 0;
    let mut approval_count = 0;
    let mut resolved = 0;
    let mut final_verified = false;
    let mut native_terminal = false;
    let mut no_replay_before_input = false;
    let mut final_hash = None;
    let mut native_outcome = None;
    let mut denied_read_unexecuted = false;
    let mut stage = "connect";
    let mut hook_absent_at_ready = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(220);
    let result: Result<(), String> = async {
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .map_err(|_| "固定策略阶段超时")?
                .ok_or("生产事件流提前关闭")?;
            if event.generation != generation {
                return Err("连接代次改变".into());
            }
            if let Some(id) = event.native_session_id {
                if Uuid::parse_str(&id).is_err()
                    || session.as_ref().is_some_and(|previous| previous != &id)
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
                        || effective_permissions["requestedPolicy"] != "GrokRestrictedReadV1"
                    {
                        return Err("固定创建输入尚未确认".into());
                    }
                    let observed: super::super::permissions::GrokCreationPolicyV1 =
                        serde_json::from_value(
                            effective_permissions["grokCreationPolicyV1"].clone(),
                        )
                        .map_err(|_| "固定策略快照缺失")?;
                    observed.validate().map_err(|_| "固定策略快照无效")?;
                    if expected_profile
                        .as_ref()
                        .is_some_and(|saved| saved != &observed)
                    {
                        return Err("恢复改写了固定创建策略".into());
                    }
                    profile = Some(observed);
                    ready = true;
                    stage = "ready";
                    hook_absent_at_ready = !project.join(".project-hook-ran").exists();
                    if !hook_absent_at_ready {
                        return Err("项目启动 hook 被执行".into());
                    }
                    // 冷恢复期间绝不发送 prompt；先观察空闲窗口，任何自发业务事件都判失败。
                    match tokio::time::timeout(Duration::from_millis(750), events.recv()).await {
                        Err(_) => no_replay_before_input = true,
                        Ok(_) => return Err("输入之前出现自发业务事件".into()),
                    }
                    submitted += 1;
                    stage = "submitted";
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: input_id,
                            action: RuntimeAction::Submit {
                                input: vec![InputContent::Text(prompt.into())],
                            },
                        })
                        .await
                        .map_err(|_| "输入发送失败")?;
                }
                RuntimeEventKind::MessageAccepted {
                    message_id,
                    turn_id,
                } => {
                    if message_id != input_id || accepted != 0 || !ready || submitted != 1 {
                        return Err("输入接收回执不匹配".into());
                    }
                    accepted += 1;
                    if let Some(id) = turn_id {
                        turn = Some(id);
                    }
                }
                RuntimeEventKind::TurnStarted { turn_id } => {
                    if !ready || turn.as_ref().is_some_and(|prior| prior != &turn_id) {
                        return Err("回合身份不匹配".into());
                    }
                    turn = Some(turn_id);
                }
                RuntimeEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    method,
                    details,
                } => {
                    let exact = approval_count == 0
                        && turn.as_deref() == Some(turn_id.as_str())
                        && method == "session/request_permission"
                        && session.as_deref().is_some_and(|session| {
                            exact_read(&details, &project, &target, session)
                        });
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: Uuid::new_v4(),
                            action: RuntimeAction::RespondApproval {
                                approval_id: approval_id.clone(),
                                decision: if exact {
                                    decision
                                } else {
                                    ApprovalDecision::DenyOnce
                                },
                            },
                        })
                        .await
                        .map_err(|_| "审批发送失败")?;
                    if !exact {
                        return Err("出现非预期工具或重复审批".into());
                    }
                    approval_count += 1;
                    stage = "approval_requested";
                    approval = Some(approval_id);
                    call = details["toolCall"]["toolCallId"]
                        .as_str()
                        .map(str::to_owned);
                    if call.is_none() {
                        return Err("原生工具身份缺失".into());
                    }
                }
                RuntimeEventKind::ApprovalResolved {
                    approval_id,
                    decision: actual,
                } => {
                    if approval.as_ref() != Some(&approval_id)
                        || actual != decision
                        || resolved != 0
                    {
                        return Err("审批实际写入回执不匹配".into());
                    }
                    resolved += 1;
                    stage = "approval_resolved";
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
                    let expected_outcome = if allow {
                        TurnOutcome::Completed
                    } else {
                        TurnOutcome::Cancelled
                    };
                    if turn.as_deref() != Some(turn_id.as_str())
                        || outcome != expected_outcome
                        || accepted != 1
                        || resolved != 1
                    {
                        return Err("原生终态未满足".into());
                    }
                    let snapshots = histories.lock().expect("历史验收锁未损坏");
                    let snapshot = snapshots.get(&turn_id).ok_or("缺少生产最终历史")?;
                    let receipt = super::live_tests::final_response_receipt(
                        snapshot,
                        session.as_deref().ok_or("缺少原生会话")?,
                        &turn_id,
                        &outcome,
                    )?;
                    if (allow && receipt.final_response.trim() != allowed_token.trim())
                        || receipt.full_output != output
                        || snapshot.replay.to_string().contains(denied_token.trim())
                    {
                        return Err("最终结果或拒绝读取边界不匹配".into());
                    }
                    native_terminal = native_tool_terminal(
                        &snapshot.replay,
                        &turn_id,
                        call.as_deref().ok_or("缺少工具身份")?,
                        allow,
                    );
                    if !native_terminal {
                        return Err("缺少真实工具完成或拒绝终态".into());
                    }
                    if !allow {
                        denied_read_unexecuted = denied_read_not_executed(
                            &snapshot.replay,
                            &turn_id,
                            call.as_deref().ok_or("缺少工具身份")?,
                        );
                        if !denied_read_unexecuted {
                            return Err("拒绝回合缺少权限拒绝取消或存在读取结果".into());
                        }
                    }
                    final_hash = Some(digest(receipt.final_response.trim()));
                    final_verified = true;
                    stage = "verified";
                    return Ok(());
                }
                RuntimeEventKind::TextDelta { turn_id, .. }
                | RuntimeEventKind::Progress { turn_id, .. } => {
                    if turn.as_deref() != Some(turn_id.as_str()) {
                        return Err("收到旧回合内容".into());
                    }
                }
                RuntimeEventKind::CommandDispatched { .. } => {}
                RuntimeEventKind::ApprovalCancelled { .. }
                | RuntimeEventKind::LocalToolCancelled { .. }
                | RuntimeEventKind::LocalToolRequested { .. }
                | RuntimeEventKind::InputJoined { .. }
                | RuntimeEventKind::RequestFailed { .. }
                | RuntimeEventKind::Disconnected { .. } => {
                    return Err("生产连接提前结束或业务越界".into());
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
    let joined = tokio::time::timeout(Duration::from_secs(35), &mut task).await;
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
    let no_hook = !project.join(".project-hook-ran").exists();
    let passed = result.is_ok()
        && cleanup
        && transport_closed
        && auth_removed
        && no_hook
        && final_verified
        && native_terminal;
    record(
        file,
        json!({"event":"fixed_policy_phase","scope":SCOPE,"phase":if resumed {"load_deny"} else {"new_allow"},
        "passed":passed,"ready":ready,"same_saved_profile":expected_profile.is_none() || profile==expected_profile,
        "same_native_session":expected_session.is_none() || session==expected_session,
        "no_replay_before_input":no_replay_before_input,"hook_absent_at_ready":hook_absent_at_ready,"hook_absent_after_shutdown":no_hook,
        "submitted":submitted,"accepted":accepted,"approvals":approval_count,"approval_resolved":resolved,
        "final_history_verified":final_verified,"native_tool_terminal":native_terminal,"cleanup_confirmed":cleanup,
        "transport_closed":transport_closed,"managed_auth_removed":auth_removed,"final_sha256":final_hash,
        "native_session_sha256":session.as_deref().map(digest),"allow":allow,
        "native_outcome":native_outcome,"denied_read_not_executed":denied_read_unexecuted,
        "failure_stage":if passed {None} else {Some(stage)}}),
    )?;
    if !passed {
        return Err("固定策略阶段验收失败".into());
    }
    Ok((session.ok_or("缺少会话")?, profile.ok_or("缺少策略")?))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由隔离运行器启动；两次真实模型输入，会消耗已授权额度"]
async fn authenticated_fixed_policy_read_approval_and_cold_restore() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("需要隔离运行器"))
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
    let artifact = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").unwrap());
    assert_eq!(artifact, root.join("private-evidence.ndjson"));
    let mut file = File::create(artifact).unwrap();
    record(
        &mut file,
        json!({"event":"fixed_policy_started","scope":SCOPE,"max_native_inputs":2,
        "production_prepare":true,"test_argv_override":false,"credential_values_recorded":false}),
    )
    .unwrap();
    let options = SessionOptions {
        executable: PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").unwrap()),
        cwd: root.join("project"),
        state_dir: root.join("state"),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::GrokRestrictedReadV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    };
    let result: Result<(), String> = async {
        let (id, profile) = phase(options.clone(), true, &mut file).await?;
        // 私有文件真实保存并重新读取；本夹具不声称协调器数据库或 GUI 应用重启已经验证。
        let saved = root.join("saved-session.json");
        fs::write(
            &saved,
            serde_json::to_vec(&json!({"native_session_id":id,"profile":profile})).unwrap(),
        )
        .map_err(|_| "会话保存失败")?;
        let persisted: Value =
            serde_json::from_slice(&fs::read(saved).map_err(|_| "会话读取失败")?)
                .map_err(|_| "会话解析失败")?;
        let mut resumed = options;
        resumed.generation = Uuid::new_v4();
        resumed.target = SessionTarget::Resume {
            native_session_id: persisted["native_session_id"]
                .as_str()
                .ok_or("会话ID缺失")?
                .to_owned(),
        };
        resumed.grok_profile =
            Some(serde_json::from_value(persisted["profile"].clone()).map_err(|_| "策略解析失败")?);
        phase(resumed, false, &mut file).await?;
        Ok(())
    }
    .await;
    record(&mut file,json!({"event":"fixed_policy_finished","scope":SCOPE,"passed":result.is_ok(),
        "system_managed_policies_apply":true,"native_effective_policy_verified":false,"filesystem_sandbox_verified":false,
        "app_restart_verified":false,"coordinator_verified":false,"spawn_verified":false,"full_cli_parity_acceptance_passed":false})).unwrap();
    assert!(result.is_ok(), "固定策略验收失败；只查看安全事件投影");
}

#[test]
fn read_approval_requires_real_target_file_schema_and_exact_target() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().canonicalize().unwrap();
    let expected = project.join("allow.txt");
    fs::write(&expected, "离线夹具").unwrap();
    let mut details = json!({"sessionId":"native-session", "toolCall":{
        "kind":"read", "_meta":{"x.ai/tool":{"name":"read_file"}},
        "rawInput":{"variant":"ReadFile", "target_file":"allow.txt"}
    }});
    assert!(exact_read(&details, &project, &expected, "native-session"));
    details["toolCall"]["rawInput"] = json!({"variant":"ReadFile", "path":"allow.txt"});
    assert!(!exact_read(&details, &project, &expected, "native-session"));
    details["toolCall"]["rawInput"] = json!({"variant":"ReadFile", "target_file":"other.txt"});
    assert!(!exact_read(&details, &project, &expected, "native-session"));
    details["toolCall"]["rawInput"] =
        json!({"variant":"ReadFile", "target_file":"allow.txt", "command":"unexpected"});
    assert!(!exact_read(&details, &project, &expected, "native-session"));
}

#[test]
fn native_tool_receipt_requires_only_the_approved_call_and_its_terminal_state() {
    let mut replay = json!({"updates":[
        {"method":"session/update", "params":{"_meta":{"promptId":"turn"},"update":{
            "sessionUpdate":"tool_call", "toolCallId":"call"}}},
        {"method":"session/update", "params":{"_meta":{"promptId":"turn"},"update":{
            "sessionUpdate":"tool_call_update", "toolCallId":"call", "status":"completed"}}}
    ]});
    assert!(native_tool_terminal(&replay, "turn", "call", true));
    assert!(!native_tool_terminal(&replay, "turn", "call", false));
    replay["updates"][1]["params"]["update"]["status"] = json!("failed");
    assert!(native_tool_terminal(&replay, "turn", "call", false));
    assert!(!native_tool_terminal(&replay, "old-turn", "call", false));
    replay["updates"][0]["params"]["update"]["toolCallId"] = json!("other-call");
    assert!(!native_tool_terminal(&replay, "turn", "call", false));
}

#[test]
fn native_deny_receipt_requires_permission_rejection_and_no_read_output() {
    let replay = json!({"updates":[
        {"method":"session/update", "params":{"_meta":{"promptId":"turn"},"update":{
            "sessionUpdate":"tool_call", "toolCallId":"call"}}},
        {"method":"session/update", "params":{"_meta":{"promptId":"turn"},"update":{
            "sessionUpdate":"tool_call_update", "toolCallId":"call", "status":"failed"}}},
        {"method":"_x.ai/session/update", "params":{
            "_meta":{"cancellationCategory":"PermissionRejected", "cancellationContext":{"tool_name":"read_file"}},
            "update":{"sessionUpdate":"turn_completed", "prompt_id":"turn", "stop_reason":"cancelled"}}}
    ]});
    assert!(denied_read_not_executed(&replay, "turn", "call"));
    let narration = json!({"method":"session/update", "params":{"_meta":{"promptId":"turn"},"update":{
        "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"读取前说明"}}}});
    let mut before = replay.clone();
    before["updates"]
        .as_array_mut()
        .unwrap()
        .insert(0, narration.clone());
    assert!(denied_read_not_executed(&before, "turn", "call"));
    let mut after = replay.clone();
    after["updates"]
        .as_array_mut()
        .unwrap()
        .insert(2, narration);
    assert!(!denied_read_not_executed(&after, "turn", "call"));
    assert!(!denied_read_not_executed(&replay, "old-turn", "call"));
    assert!(!denied_read_not_executed(&replay, "turn", "other-call"));
    for category in ["MidTurnAbort", "unknown"] {
        let mut changed = replay.clone();
        changed["updates"][2]["params"]["_meta"]["cancellationCategory"] = json!(category);
        assert!(!denied_read_not_executed(&changed, "turn", "call"));
    }
    let mut completed_turn = replay.clone();
    completed_turn["updates"][2]["params"]["update"]["stop_reason"] = json!("end_turn");
    assert!(!denied_read_not_executed(&completed_turn, "turn", "call"));
    let mut missing_marker = replay.clone();
    missing_marker["updates"].as_array_mut().unwrap().pop();
    assert!(!denied_read_not_executed(&missing_marker, "turn", "call"));
    let mut completed = replay.clone();
    completed["updates"][1]["params"]["update"]["status"] = json!("completed");
    assert!(!denied_read_not_executed(&completed, "turn", "call"));
    let mut leaked = replay.clone();
    leaked["updates"][1]["params"]["update"]["rawOutput"] = json!({"text":"不应读取"});
    assert!(!denied_read_not_executed(&leaked, "turn", "call"));
    let mut wrong_tool = replay.clone();
    wrong_tool["updates"][2]["params"]["_meta"]["cancellationContext"]["tool_name"] =
        json!("write");
    assert!(!denied_read_not_executed(&wrong_tool, "turn", "call"));
    let mut duplicate = replay.clone();
    duplicate["updates"]
        .as_array_mut()
        .unwrap()
        .push(replay["updates"][2].clone());
    assert!(!denied_read_not_executed(&duplicate, "turn", "call"));
}
