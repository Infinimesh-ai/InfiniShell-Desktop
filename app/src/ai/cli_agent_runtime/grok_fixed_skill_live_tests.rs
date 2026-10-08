//! 固定技能创建集合的真实审批与冷恢复；拒绝证据不能替代原生执行正例。

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
use crate::ai::cli_agent_runtime::grok_profile::GrokCreationPolicyV1;
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeController, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome,
    managed_process,
};

const SCOPE: &str = "authenticated_fixed_skill_approval_and_cold_restore";
const ALPHA: &str = "isp-g06-fixed-alpha";
const BETA: &str = "isp-g06-fixed-beta";

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .map_err(|_| "证据写入失败".into())
}

fn selected(root: &Path, name: &str) -> SelectedLocalSkill {
    SelectedLocalSkill {
        name: name.into(),
        path: root.join("skills").join(name).join("SKILL.md"),
    }
}

async fn send(
    controller: &RuntimeController,
    generation: Uuid,
    action: RuntimeAction,
) -> Result<Uuid, String> {
    let message_id = Uuid::new_v4();
    controller
        .send(RuntimeCommand {
            generation,
            message_id,
            action,
        })
        .await
        .map_err(|_| "控制器发送失败")?;
    Ok(message_id)
}

async fn next(
    events: &mut tokio::sync::mpsc::Receiver<RuntimeEvent>,
    generation: Uuid,
    session: &str,
    deadline: tokio::time::Instant,
) -> Result<RuntimeEventKind, String> {
    let event = tokio::time::timeout_at(deadline, events.recv())
        .await
        .map_err(|_| "原生阶段超时")?
        .ok_or("生产事件流提前关闭")?;
    if event.generation != generation
        || event
            .native_session_id
            .as_deref()
            .is_some_and(|id| id != session)
    {
        return Err("事件会话或代次改变".into());
    }
    Ok(event.kind)
}

fn exact_skill(call: &Value) -> bool {
    call["kind"] == "other"
        && call["_meta"]["x.ai/tool"]["name"] == "skill"
        && call["rawInput"] == json!({"variant":"Dynamic","name":format!("user:{ALPHA}")})
}

fn strings_contain(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(text) => text.contains(expected),
        Value::Array(values) => values.iter().any(|value| strings_contain(value, expected)),
        Value::Object(values) => values
            .values()
            .any(|value| strings_contain(value, expected)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn expected_skill_output(body: &str, directory: &Path) -> Value {
    // 公开 1.0.41 的 OpenCode SkillOutput 经 ToolOutput::Skill 原样序列化到 rawOutput。
    // 夹具只有 SKILL.md，原生列表排除该文件；正文与来源目录必须同时精确匹配。
    let body = body.trim();
    let directory = directory.display();
    let message = format!(
        "<skill_content name=\"{ALPHA}\">\n# Skill: {ALPHA}\n\n{body}\n\nBase directory for this skill: file://{directory}\nRelative paths in this skill (e.g., scripts/, reference/) are relative to this base directory.\nNote: file list is sampled.\n\n<skill_files>\n\n</skill_files>\n</skill_content>"
    );
    json!({"type":"Skill","success":true,"tool_result":format!("Loaded skill: {ALPHA}"),
        "skill_name":ALPHA,"skill_message":message,"error":null})
}

fn verify_history(
    replay: &Value,
    session: &str,
    turn: &str,
    call: &str,
    allow: bool,
    body: &str,
    directory: &Path,
    beta_marker: &str,
) -> Result<Vec<Value>, String> {
    let updates = replay["updates"].as_array().ok_or("原生回放缺少事件")?;
    let rows: Vec<_> = updates
        .iter()
        .filter(|row| {
            row["params"]["_meta"]["promptId"] == turn
                || row["params"]["update"]["prompt_id"] == turn
        })
        .cloned()
        .collect();
    let mut calls = HashSet::new();
    let mut terminal = 0;
    let mut completions = 0;
    let mut loaded = false;
    let mut typed = false;
    let expected_output = expected_skill_output(body, directory);
    let mut requested = false;
    for row in &rows {
        let params = &row["params"];
        let update = &params["update"];
        if params["sessionId"] != session || strings_contain(row, beta_marker) {
            return Err("回放会话改变或泄漏未选技能正文".into());
        }
        if row["method"] == "session/update" {
            match update["sessionUpdate"].as_str() {
                Some("tool_call") => {
                    // 真实 4220f3b 首次事件只有模型参数，原生类型在下一条关联更新补齐。
                    if update.get("kind").is_some()
                        || params["_meta"]["updateParams"]["kind"] != "Other"
                        || params["_meta"]["updateParams"]["toolCallId"] != call
                        || update["_meta"]["x.ai/tool"]["name"] != "skill"
                        || update["rawInput"] != json!({"name":format!("user:{ALPHA}")})
                        || update.get("rawOutput").is_some()
                        || update["toolCallId"] != call
                        || completions != 0
                        || !calls.insert(call.to_owned())
                    {
                        return Err("原生技能调用不是唯一精确工具".into());
                    }
                    requested = true;
                }
                Some("tool_call_update") => {
                    if update["toolCallId"] != call
                        || !requested
                        || completions != 0
                        || terminal != 0
                    {
                        return Err("工具输出身份或顺序不匹配".into());
                    }
                    if update.get("rawInput").is_some() {
                        if typed
                            || !exact_skill(update)
                            || update.get("status").is_some()
                            || update.get("rawOutput").is_some()
                        {
                            return Err("原生类型更新重复、身份改变或顺序错误".into());
                        }
                        typed = true;
                    } else if matches!(
                        update["status"].as_str(),
                        Some("completed" | "failed" | "cancelled")
                    ) && !typed
                    {
                        return Err("原生工具终态之前缺少精确类型更新".into());
                    }
                    if allow {
                        if update["status"] == "completed" {
                            terminal += 1;
                            loaded |= update.get("rawOutput") == Some(&expected_output);
                        } else if matches!(update["status"].as_str(), Some("failed" | "cancelled"))
                        {
                            return Err("允许的原生技能执行失败".into());
                        }
                    } else {
                        if update.get("rawOutput").is_some()
                            || strings_contain(update, body.trim())
                            || update["status"] == "completed"
                        {
                            return Err("拒绝后出现技能执行结果".into());
                        }
                        terminal += usize::from(matches!(
                            update["status"].as_str(),
                            Some("failed" | "cancelled")
                        ));
                    }
                }
                Some("agent_message_chunk") if !allow && requested => {
                    return Err("拒绝后模型继续输出".into());
                }
                Some(_) | None => {}
            }
        } else if row["method"] == "_x.ai/session/update"
            && update["sessionUpdate"] == "turn_completed"
        {
            if update["stop_reason"] != if allow { "end_turn" } else { "cancelled" }
                || (!allow
                    && (params["_meta"]["cancellationCategory"] != "PermissionRejected"
                        || params["_meta"]["cancellationContext"]["tool_name"] != "skill"))
            {
                return Err("原生回合结束原因不匹配".into());
            }
            completions += 1;
        }
    }
    if calls.len() != 1 || !typed || terminal != 1 || completions != 1 || (allow && !loaded) {
        return Err("缺少唯一原生工具终态或完整技能正文".into());
    }
    // 思考内容不作为验收或归档材料。
    Ok(rows
        .into_iter()
        .filter(|row| row["params"]["update"]["sessionUpdate"] != "agent_thought_chunk")
        .collect())
}

fn denied_history_fixture() -> Value {
    json!({"updates":[
        {"method":"session/update","params":{"sessionId":"session","_meta":{"promptId":"turn","updateParams":{"toolCallId":"call","kind":"Other"}},
            "update":{"sessionUpdate":"tool_call","toolCallId":"call","rawInput":{"name":"user:isp-g06-fixed-alpha"},"_meta":{"x.ai/tool":{"name":"skill"}}}}},
        {"method":"session/update","params":{"sessionId":"session","_meta":{"promptId":"turn"},
            "update":{"sessionUpdate":"tool_call_update","toolCallId":"call","kind":"other","rawInput":{"variant":"Dynamic","name":"user:isp-g06-fixed-alpha"},"_meta":{"x.ai/tool":{"name":"skill"}}}}},
        {"method":"session/update","params":{"sessionId":"session","_meta":{"promptId":"turn"},
            "update":{"sessionUpdate":"tool_call_update","toolCallId":"call","status":"failed"}}},
        {"method":"_x.ai/session/update","params":{"sessionId":"session","_meta":{"cancellationCategory":"PermissionRejected","cancellationContext":{"tool_name":"skill"}},
            "update":{"sessionUpdate":"turn_completed","prompt_id":"turn","stop_reason":"cancelled"}}}
    ]})
}

#[test]
fn native_skill_history_requires_typed_update_before_terminal() {
    let mut replay = denied_history_fixture();
    assert!(
        verify_history(
            &replay,
            "session",
            "turn",
            "call",
            false,
            "正文",
            Path::new("/skill"),
            "未选标记"
        )
        .is_ok()
    );
    replay["updates"].as_array_mut().unwrap().remove(1);
    assert!(
        verify_history(
            &replay,
            "session",
            "turn",
            "call",
            false,
            "正文",
            Path::new("/skill"),
            "未选标记"
        )
        .is_err()
    );
}

#[test]
fn native_skill_history_rejects_typed_update_for_another_call() {
    let mut replay = denied_history_fixture();
    replay["updates"][1]["params"]["update"]["toolCallId"] = json!("other-call");
    assert!(
        verify_history(
            &replay,
            "session",
            "turn",
            "call",
            false,
            "正文",
            Path::new("/skill"),
            "未选标记"
        )
        .is_err()
    );
}

fn verify_profile(root: &Path, profile: &GrokCreationPolicyV1) -> Result<PathBuf, String> {
    let alpha = selected(root, ALPHA);
    let beta = selected(root, BETA);
    profile.validate().map_err(|_| "固定策略无效")?;
    profile
        .verify_files(&root.join("state"))
        .map_err(|_| "固定策略文件改变")?;
    if !profile.skill_selection_matches(std::slice::from_ref(&alpha))
        || profile
            .derive_child_with_skills(None, std::slice::from_ref(&alpha))
            .is_err()
        || profile.derive_child_with_skills(None, &[]).is_err()
        || profile
            .derive_child_with_skills(None, std::slice::from_ref(&beta))
            .is_ok()
        || profile
            .derive_child_with_skills(None, &[alpha.clone(), beta])
            .is_ok()
    {
        return Err("固定父集合被扩大或合法子集合不可用".into());
    }
    let value = serde_json::to_value(profile).map_err(|_| "策略序列化失败")?;
    let storage = value["storageId"].as_str().ok_or("策略缺少存储身份")?;
    Uuid::parse_str(storage).map_err(|_| "存储身份无效")?;
    let home = root.join("state/grok-managed").join(storage);
    let skills = home.join("grok/skills");
    let entries = fs::read_dir(&skills)
        .map_err(|_| "技能目录缺失")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "技能目录不可读")?;
    if entries.len() != 1
        || entries[0].file_name() != ALPHA
        || fs::read(skills.join(ALPHA).join("SKILL.md")).map_err(|_| "技能副本不可读")?
            != fs::read(alpha.path).map_err(|_| "技能来源不可读")?
    {
        return Err("原生私有集合或技能字节改变".into());
    }
    Ok(home)
}

async fn generation(
    root: &Path,
    options: SessionOptions,
    file: &mut File,
) -> Result<(String, GrokCreationPolicyV1), String> {
    let generation = options.generation;
    let resumed = matches!(options.target, SessionTarget::Resume { .. });
    let saved_profile = options.grok_profile.clone();
    let saved_session = match &options.target {
        SessionTarget::New => None,
        SessionTarget::Resume { native_session_id } => Some(native_session_id.clone()),
    };
    validate_options(&options).map_err(|_| "固定生产参数验证失败")?;
    let mut protocol = GrokProtocol::new(options);
    let histories = Arc::new(Mutex::new(HashMap::new()));
    protocol.verified_final_histories_for_live = Some(histories.clone());
    let connection = connect_protocol(protocol);
    let controller = connection.controller;
    let mut events = connection.events;
    let mut task = tokio::spawn(connection.task);
    let mut identity = None;
    let mut managed_home = None;
    let result: Result<(String, GrokCreationPolicyV1), String> = async {
        let ready = tokio::time::timeout(Duration::from_secs(90), events.recv()).await.map_err(|_| "等待固定技能会话超时")?
            .ok_or("固定技能启动事件缺失")?;
        let session = ready.native_session_id.ok_or("原生会话缺失")?;
        if ready.generation != generation || Uuid::parse_str(&session).is_err()
            || saved_session.as_ref().is_some_and(|saved| saved != &session) {
            return Err("原生新建或恢复身份错误".into());
        }
        identity = Some(session.clone());
        let RuntimeEventKind::SessionReady { effective_permissions, verified_cli_version } = ready.kind else {
            return Err("首次事件不是生产就绪".into());
        };
        if verified_cli_version.as_deref() != Some("1.0.41")
            || effective_permissions["requestedPolicy"] != "GrokRestrictedSkillsV1"
            || effective_permissions["appCreationPolicyApplied"] != true
            || effective_permissions["permissionEnforcementVerified"] != false {
            return Err("生产固定技能策略未确认".into());
        }
        let profile: GrokCreationPolicyV1 = serde_json::from_value(effective_permissions["grokCreationPolicyV1"].clone()).map_err(|_| "固定技能快照缺失")?;
        if saved_profile.as_ref().is_some_and(|saved| saved != &profile) {
            return Err("冷恢复扩大或替换了固定集合".into());
        }
        let home = verify_profile(root, &profile)?;
        managed_home = Some(home.clone());
        record(file, json!({"event":"ready","scope":SCOPE,"generation":generation,"native_session_id":session,"resumed":resumed,
            "profile":profile,"managed_home":home,"selected_skill_bytes_verified":true,"child_subset_verified":true}))?;
        if tokio::time::timeout(Duration::from_millis(750), events.recv()).await.is_ok() {
            return Err("明确输入前发生自发业务事件".into());
        }
        // 未选 B 通过真实控制器提交；任何审批、接收或原生回合事件都不能当成拒绝。
        let beta = selected(root, BETA);
        let rejected = send(&controller, generation, RuntimeAction::Submit { input: vec![
            InputContent::Text("只执行本轮选择的技能。".into()), InputContent::Skill { name: beta.name, path: beta.path },
        ] }).await?;
        match next(&mut events, generation, &session, tokio::time::Instant::now() + Duration::from_secs(10)).await? {
            RuntimeEventKind::RequestFailed { message_id, .. } if message_id == rejected => {}
            _ => return Err("未选技能未在生产输入边界拒绝".into()),
        }
        if tokio::time::timeout(Duration::from_millis(500), events.recv()).await.is_ok() {
            return Err("未选技能拒绝后仍出现业务事件".into());
        }
        record(file, json!({"event":"unselected_rejected","scope":SCOPE,"generation":generation,"message_id":rejected,"resumed":resumed}))?;
        let document = fs::read_to_string(selected(root, ALPHA).path).map_err(|_| "技能来源缺失")?;
        let body = document.splitn(3, "---\n").nth(2).ok_or("技能正文缺失")?;
        let marker = body.lines().last().filter(|line| line.starts_with("G06_FIXED_ALPHA_")).ok_or("技能隐藏标记缺失")?;
        let beta_document = fs::read_to_string(selected(root, BETA).path).map_err(|_| "未选技能来源缺失")?;
        let beta_marker = beta_document.lines().last().ok_or("未选技能标记缺失")?;
        let cases: &[(&str, bool)] = if resumed { &[("cold_allow", true)] } else { &[("deny", false), ("allow", true)] };
        for &(case, allow) in cases {
            let prompt = format!("本轮 {case}：只调用本轮所选技能的原生 skill 工具一次，等待逐次审批。拒绝后不要重试或改用其它工具。允许后必须读取本轮工具返回的正文，再严格只输出该技能末尾标记一行；不能从历史记忆复述。不要读取文件、运行命令、修改文件或调用其它工具。");
            if prompt.contains(marker) || prompt.contains(beta_marker) { return Err("隐藏标记进入输入".into()); }
            let alpha = selected(root, ALPHA);
            let input = send(&controller, generation, RuntimeAction::Submit { input: vec![InputContent::Text(prompt),
                InputContent::Skill { name: alpha.name, path: alpha.path }] }).await?;
            record(file, json!({"event":"submitted","scope":SCOPE,"case":case,"generation":generation,"message_id":input}))?;
            let mut turn: Option<String> = None;
            let mut accepted = false;
            let mut started = false;
            let mut approval: Option<String> = None;
            let mut call: Option<String> = None;
            let mut resolved = false;
            let mut control: Option<Uuid> = None;
            let decision = if allow { ApprovalDecision::AllowOnce } else { ApprovalDecision::DenyOnce };
            let deadline = tokio::time::Instant::now() + Duration::from_secs(220);
            loop {
                match next(&mut events, generation, &session, deadline).await? {
                    RuntimeEventKind::MessageAccepted { message_id, turn_id } => {
                        if message_id != input || accepted || turn_id.as_deref().is_none_or(|id| Uuid::parse_str(id).is_err()) { return Err("输入回执错误".into()); }
                        accepted = true; turn = turn_id;
                    }
                    RuntimeEventKind::TurnStarted { turn_id } => {
                        if started || turn.as_deref() != Some(turn_id.as_str()) { return Err("运行回合错误".into()); }
                        started = true;
                    }
                    RuntimeEventKind::ApprovalRequested { approval_id, turn_id, method, details } => {
                        let exact = approval.is_none() && started && turn.as_deref() == Some(turn_id.as_str())
                            && method == "session/request_permission" && details["sessionId"] == session
                            && exact_skill(&details["toolCall"])
                            && details["toolCall"]["toolCallId"].as_str().is_some_and(|id| !id.is_empty());
                        control = Some(send(&controller, generation, RuntimeAction::RespondApproval { approval_id: approval_id.clone(),
                            decision: if exact { decision } else { ApprovalDecision::DenyOnce } }).await?);
                        record(file, json!({"event":"approval","scope":SCOPE,"case":case,"generation":generation,"exact":exact,
                            "decision":if exact { decision } else { ApprovalDecision::DenyOnce },"details":details}))?;
                        if !exact { return Err("已拒绝非精确技能或重复调用".into()); }
                        call = details["toolCall"]["toolCallId"].as_str().map(str::to_owned); approval = Some(approval_id);
                    }
                    RuntimeEventKind::ApprovalResolved { approval_id, decision: actual } => {
                        if resolved || approval.as_ref() != Some(&approval_id) || actual != decision { return Err("审批写入回执错误".into()); }
                        resolved = true;
                    }
                    RuntimeEventKind::CommandDispatched { message_id, turn_id } => {
                        if control.take() != Some(message_id) || turn_id != turn { return Err("控制响应身份错误".into()); }
                    }
                    RuntimeEventKind::TextDelta { turn_id, .. } | RuntimeEventKind::Progress { turn_id, .. } => {
                        if turn.as_deref() != Some(turn_id.as_str()) { return Err("旧回合输出".into()); }
                    }
                    RuntimeEventKind::TurnFinished { turn_id, outcome, output } => {
                        if !accepted || !started || !resolved || control.is_some() || turn.as_deref() != Some(turn_id.as_str())
                            || outcome != if allow { TurnOutcome::Completed } else { TurnOutcome::Cancelled } { return Err("原生终态不满足".into()); }
                        let snapshots = histories.lock().expect("历史锁未损坏");
                        let snapshot = snapshots.get(&turn_id).ok_or("缺少生产最终历史")?;
                        let receipt = super::live_tests::final_response_receipt(snapshot, &session, &turn_id, &outcome)?;
                        if receipt.full_output != output || (allow && receipt.final_response.trim() != marker) { return Err("真实最终答案不匹配".into()); }
                        let rows = verify_history(&snapshot.replay, &session, &turn_id, call.as_deref().ok_or("缺少工具身份")?, allow, body, &home.join("grok/skills").join(ALPHA), beta_marker)?;
                        let replay = serde_json::to_vec(&rows).map_err(|_| "历史序列化失败")?;
                        fs::write(root.join(format!("{case}.replay.json")), &replay).map_err(|_| "历史证据写入失败")?;
                        verify_profile(root, &profile)?;
                        record(file, json!({"event":"turn_verified","scope":SCOPE,"case":case,"generation":generation,
                            "native_session_id":session,"turn_id":turn_id,"message_id":input,"call_id":call,"allow":allow,
                            "outcome":outcome,"replay_sha256":digest(&replay),"skill_sha256":digest(document.as_bytes()),
                            "final_sha256":digest(receipt.final_response.trim().as_bytes())}))?;
                        break;
                    }
                    RuntimeEventKind::SessionReady { .. } | RuntimeEventKind::ApprovalCancelled { .. }
                    | RuntimeEventKind::LocalToolCancelled { .. } | RuntimeEventKind::LocalToolRequested { .. }
                    | RuntimeEventKind::InputJoined { .. } | RuntimeEventKind::RequestFailed { .. }
                    | RuntimeEventKind::Disconnected { .. } => return Err("固定技能验收出现意外业务事件".into()),
                }
            }
        }
        Ok((session, profile))
    }.await;
    let _ = send(&controller, generation, RuntimeAction::Shutdown).await;
    let joined = tokio::time::timeout(Duration::from_secs(45), &mut task).await;
    let finished = matches!(&joined, Ok(Ok(Ok(()))));
    if joined.is_err() {
        task.abort();
        let _ = task.await;
    }
    let receipt = managed_process::confirmed_exit(&root.join("state"), generation)
        .ok()
        .flatten();
    let cleaned = receipt
        .as_ref()
        .is_some_and(|receipt| receipt.cleanup_confirmed && receipt.exit_code == Some(0));
    let auth_removed = managed_home
        .as_ref()
        .is_some_and(|home| !home.join("grok/auth.json").exists());
    record(
        file,
        json!({"event":"cleanup","scope":SCOPE,"generation":generation,"native_session_id":identity,"resumed":resumed,
        "task_finished_ok":finished,"receipt":receipt,"managed_auth_removed":auth_removed,"phase_error":result.as_ref().err()}),
    )?;
    if !finished || !cleaned || !auth_removed {
        return Err("原生自然退出、资源清理或认证清理未确认".into());
    }
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "由私有短目录运行器执行；三次真实 Grok 模型输入"]
async fn authenticated_fixed_skill_approval_and_cold_restore() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("需要隔离运行器"))
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-fixed-skill-probe")).unwrap(),
        "isolated Grok fixed skill verification\n"
    );
    assert_eq!(
        PathBuf::from(env::var_os("GROK_HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("home/.grok")
    );
    let mut file = File::create(root.join("events.ndjson")).unwrap();
    record(&mut file, json!({"event":"started","scope":SCOPE,"max_native_inputs":3,"production_prepare":true,"test_argv_override":false})).unwrap();
    let options = SessionOptions {
        executable: PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").unwrap()),
        cwd: root.join("project"),
        state_dir: root.join("state"),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::GrokRestrictedSkillsV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: vec![selected(&root, ALPHA)],
    };
    let result: Result<(), String> = async {
        let (session, profile) = generation(&root, options.clone(), &mut file).await?;
        let saved = root.join("saved-session.json");
        fs::write(
            &saved,
            serde_json::to_vec(&json!({"native_session_id":session,"profile":profile})).unwrap(),
        )
        .map_err(|_| "保存失败")?;
        let persisted: Value =
            serde_json::from_slice(&fs::read(saved).map_err(|_| "恢复读取失败")?)
                .map_err(|_| "恢复解析失败")?;
        let mut resumed = options;
        resumed.generation = Uuid::new_v4();
        resumed.target = SessionTarget::Resume {
            native_session_id: persisted["native_session_id"]
                .as_str()
                .ok_or("缺少原生身份")?
                .into(),
        };
        resumed.grok_profile =
            Some(serde_json::from_value(persisted["profile"].clone()).map_err(|_| "策略恢复失败")?);
        generation(&root, resumed, &mut file).await?;
        Ok(())
    }
    .await;
    record(&mut file, json!({"event":"finished","scope":SCOPE,"passed":result.is_ok(),"failure":result.as_ref().err(),
        "gui_verified":false,"coordinator_restart_verified":false,"child_spawn_verified":false,"filesystem_sandbox_verified":false})).unwrap();
    assert!(result.is_ok(), "固定技能验收失败；查看本轮安全证据");
}
