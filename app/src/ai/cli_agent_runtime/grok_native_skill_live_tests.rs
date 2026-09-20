//! 隔离 profile 的原生技能黑盒验收；不打开产品 selected_skills 能力。

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

const SCOPE: &str = "authenticated_native_skill_black_box";
const PROMPT: &str = "/infinishell-native-skill";
const PROFILE: &str =
    include_str!("../../../../specs/cli-agent-parity/fixtures/grok-native-skill-profile.md");
const SKILL_PATH: &str = "project/.grok/skills/infinishell-native-skill/SKILL.md";

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .map_err(|_| "证据写入失败".into())
}

fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn skill_document(case: &str, sentinel: &str) -> String {
    let visible = case == "visible";
    format!(
        "---\nname: infinishell-native-skill\ndescription: 仅手动触发的技能验收。\nuser-invocable: {visible}\ndisable-model-invocation: true\n---\n已经读到本技能后不要继续调用工具、读取其它文件、搜索、执行命令或修改文件。\n最终只输出下一行，不解释：\n{sentinel}\n"
    )
}

fn sentinel_from_skill(content: &str, case: &str) -> Result<String, String> {
    let sentinel = content.lines().last().ok_or("技能正文为空")?;
    let nonce = sentinel
        .strip_prefix("INFINISHELL_SKILL_")
        .ok_or("技能随机标记缺失")?;
    if nonce.len() != 64
        || !nonce
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || content != skill_document(case, sentinel)
        || PROMPT.contains(sentinel)
        || PROFILE.contains(sentinel)
    {
        return Err("技能夹具边界不满足".into());
    }
    Ok(sentinel.to_owned())
}

// 模型历史的参数无 variant；ACP 已解析的 ReadFile 输入包含 variant。
fn exact_skill_read_input(input: &Value, cwd: &Path, skill: &Path, tagged: bool) -> bool {
    let Some(input) = input.as_object() else {
        return false;
    };
    if input.len() != if tagged { 2 } else { 1 }
        || (tagged && input.get("variant") != Some(&json!("ReadFile")))
    {
        return false;
    }
    let Some(target) = input.get("target_file").and_then(Value::as_str) else {
        return false;
    };
    let target = cwd.join(target);
    dunce::canonicalize(target).is_ok_and(|path| path == skill)
}

pub(super) fn exact_skill_read_permission(call: &Value, cwd: &Path, skill: &Path) -> bool {
    call["kind"] == "read"
        && call["_meta"]["x.ai/tool"]["name"] == "read_file"
        && exact_skill_read_input(&call["rawInput"], cwd, skill, true)
}

pub(super) fn verify_skill_read_history(
    updates: &[Value],
    cwd: &Path,
    skill: &Path,
    case: &str,
    approved_calls: &HashSet<String>,
) -> Result<(u64, u64), String> {
    let mut calls = HashMap::new();
    let mut frame_count = 0;
    for row in updates {
        let update = &row["params"]["update"];
        let initial = match update["sessionUpdate"].as_str() {
            Some("tool_call") => true,
            Some("tool_call_update") => false,
            Some(_) | None => continue,
        };
        frame_count += 1;
        if case != "visible" {
            return Err("隐藏技能对照不允许读取秘密文件或调用其它工具".into());
        }
        let id = update["toolCallId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("技能工具缺少身份")?;
        if initial {
            // 原生先公布最小工具身份,完整参数可在后续 tool_call_update 中到达。
            let input = update.get("rawInput").filter(|input| {
                !input.is_null() && !input.as_object().is_some_and(|object| object.is_empty())
            });
            if !calls.is_empty()
                || update["_meta"]["x.ai/tool"]["name"] != "read_file"
                || input.is_some_and(|input| !exact_skill_read_input(input, cwd, skill, false))
            {
                return Err("出现非唯一的精确技能读取".into());
            }
            calls.insert(id.to_owned(), (input.is_some(), false));
        }
        let (input_verified, completed) = calls.get_mut(id).ok_or("未关联的技能工具更新")?;
        if !initial && let Some(input) = update.get("rawInput") {
            if !exact_skill_read_input(input, cwd, skill, true) {
                return Err("技能工具输入发生改变".into());
            }
            *input_verified = true;
        }
        if let Some(name) = update["_meta"]["x.ai/tool"].get("name")
            && name != "read_file"
        {
            return Err("技能工具身份改变".into());
        }
        if let Some(kind) = update.get("kind")
            && kind != "read"
            && !(initial && kind == "other")
        {
            return Err("技能工具不是只读操作".into());
        }
        match update["status"].as_str() {
            Some("completed") => *completed = true,
            Some("pending" | "in_progress") if !*completed => {}
            None => {}
            Some(_) => return Err("技能读取没有可信的成功状态".into()),
        }
    }
    if calls
        .values()
        .any(|(input_verified, completed)| !input_verified || !completed)
        || approved_calls.iter().any(|id| !calls.contains_key(id))
    {
        return Err("技能读取或审批缺少原生完成记录".into());
    }
    Ok((frame_count, calls.len() as u64))
}

const SKILL_NAME: &str = "infinishell-native-skill";
const MAX_CATALOG_SNAPSHOTS: usize = 64;

struct CatalogSnapshot {
    session_id: String,
    fingerprint: String,
    projection: Value,
}

#[derive(Default)]
struct CatalogObservations {
    snapshots: Vec<CatalogSnapshot>,
    overflow: bool,
}

#[derive(Clone)]
pub(super) struct SkillCatalogProbe {
    skill_path: PathBuf,
    observations: Arc<Mutex<CatalogObservations>>,
}

impl SkillCatalogProbe {
    pub(super) fn new(skill_path: PathBuf) -> Self {
        Self {
            skill_path,
            observations: Arc::new(Mutex::new(CatalogObservations::default())),
        }
    }

    pub(super) fn observe(&self, message: &Value, before_first_submit: bool) {
        if message["jsonrpc"] != "2.0"
            || message.get("id").is_some()
            || message["method"] != "session/update"
            || message["params"]["update"]["sessionUpdate"] != "available_commands_update"
            || message["params"]["_meta"]["isReplay"] == true
        {
            return;
        }
        let Some(session_id) = message["params"]["sessionId"]
            .as_str()
            .filter(|id| super::valid_native_id(id))
        else {
            return;
        };
        let commands = &message["params"]["update"]["availableCommands"];
        let Some(projection) =
            project_skill_catalog(commands, &self.skill_path, before_first_submit)
        else {
            return;
        };
        let fingerprint = hash(message.to_string().as_bytes());
        let mut observations = self.observations.lock().expect("技能目录锁未损坏");
        if observations
            .snapshots
            .iter()
            .any(|snapshot| snapshot.fingerprint == fingerprint)
        {
            return;
        }
        if observations.snapshots.len() == MAX_CATALOG_SNAPSHOTS {
            observations.overflow = true;
            return;
        }
        observations.snapshots.push(CatalogSnapshot {
            session_id: session_id.to_owned(),
            fingerprint,
            projection,
        });
    }

    pub(super) fn evidence(&self, session_id: Option<&str>, case: &str) -> Value {
        let observations = self.observations.lock().expect("技能目录锁未损坏");
        let matched: Vec<_> = observations
            .snapshots
            .iter()
            .filter(|snapshot| Some(snapshot.session_id.as_str()) == session_id)
            .map(|snapshot| snapshot.projection.clone())
            .collect();
        json!({"event":"skill_catalog_observed","scope":SCOPE,"case":case,
            "snapshots":matched,"overflow":observations.overflow,
            "unassociated_snapshot_count":observations.snapshots.len()-matched.len()})
    }
}

fn project_skill_catalog(
    commands: &Value,
    skill: &Path,
    before_first_submit: bool,
) -> Option<Value> {
    let commands = commands
        .as_array()
        .filter(|commands| commands.len() <= 16384)?;
    let selected: Vec<_> = commands
        .iter()
        .filter(|command| command["name"] == SKILL_NAME)
        .collect();
    // 仅保留合成技能的已知字面量与判断结果，不记录任意名称、路径、描述或元数据值。
    let command = (selected.len() == 1).then(|| selected[0]);
    let metadata = command
        .and_then(|command| command.get("_meta"))
        .and_then(Value::as_object);
    let path = metadata
        .and_then(|metadata| metadata.get("path"))
        .and_then(Value::as_str);
    let metadata_scope = match metadata
        .and_then(|metadata| metadata.get("scope"))
        .and_then(Value::as_str)
    {
        Some("local") => "local",
        Some("repo") => "repo",
        Some("user") => "user",
        Some("server") => "server",
        Some("bundled") => "bundled",
        Some("plugin") => "plugin",
        Some(_) => "other",
        None => "absent",
    };
    Some(
        json!({"selected_name":SKILL_NAME,"command_count":commands.len(),"selected_name_count":selected.len(),
        "before_first_submit":before_first_submit,"metadata_present":metadata.is_some(),
        "path_present":path.is_some(),"path_absolute":path.is_some_and(|path| Path::new(path).is_absolute()),
        "path_matches_selected":path.is_some_and(|path| Path::new(path).is_absolute()
            && dunce::canonicalize(path).is_ok_and(|path| path==skill)),
        "metadata_scope":metadata_scope,
        "bare_name_matches":metadata.and_then(|metadata| metadata.get("bareName")).is_some_and(|name| name==SKILL_NAME),
        "qualified_name_matches":metadata.and_then(|metadata| metadata.get("qualifiedName")).is_some_and(|name| name==SKILL_NAME)}),
    )
}

async fn exercise(root: &Path, case: &str, file: &mut File) -> Result<(), String> {
    let before = fs::read_to_string(root.join(SKILL_PATH)).map_err(|_| "技能夹具不可读")?;
    let sentinel = sentinel_from_skill(&before, case)?;
    let skill_path = dunce::canonicalize(root.join(SKILL_PATH)).map_err(|_| "技能路径不可解析")?;
    let cwd = dunce::canonicalize(root.join("project")).map_err(|_| "项目路径不可解析")?;
    let profile = root.join("policy/profile.md");
    if fs::read_to_string(&profile).map_err(|_| "缺少隔离 profile")? != PROFILE {
        return Err("隔离 profile 与编译快照不一致".into());
    }
    let generation = Uuid::new_v4();
    let state_dir = root.join("state");
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("缺少原生入口")?,
        ),
        cwd: root
            .join("project")
            .canonicalize()
            .map_err(|_| "缺少隔离项目")?,
        state_dir: state_dir.clone(),
        target: SessionTarget::New,
        generation,
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    };
    validate_options(&options).map_err(|_| "生产参数拒绝")?;
    let mut protocol = GrokProtocol::new(options);
    // 仅切换验收进程的启动 profile，生产消息、解析器及状态机保持原路径。
    protocol.skill_profile_for_live = Some(profile);
    let catalog = SkillCatalogProbe::new(skill_path.clone());
    protocol.skill_catalog_for_live = Some(catalog.clone());
    let histories = Arc::new(Mutex::new(HashMap::new()));
    protocol.verified_final_histories_for_live = Some(histories.clone());
    let connection = connect_protocol(protocol);
    let controller = connection.controller;
    let mut events = connection.events;
    let mut task = tokio::spawn(connection.task);
    let input_id = Uuid::new_v4();
    let mut session = None;
    let mut turn = None;
    let mut submitted = 0_u64;
    let mut accepted = 0_u64;
    let mut approval_count = 0_u64;
    let mut tool_count = 0_u64;
    let mut skill_read_count = 0_u64;
    let mut read_scope_verified = false;
    let mut approved_calls = HashSet::new();
    let mut pending_approvals = HashSet::new();
    let mut controls = HashSet::new();
    let mut started = false;
    let mut final_verified = false;
    let mut matched = false;
    let mut final_sha256 = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(360);
    let result: Result<(), String> = async {
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .map_err(|_| "技能验收期限耗尽")?
                .ok_or("事件流已关闭")?;
            if event.generation != generation {
                return Err("收到旧连接事件".into());
            }
            if let Some(id) = event.native_session_id {
                if Uuid::parse_str(&id).is_err() || session.as_ref().is_some_and(|old| old != &id) {
                    return Err("会话身份改变".into());
                }
                session = Some(id);
            }
            match event.kind {
                RuntimeEventKind::SessionReady { .. } => {
                    if submitted != 0 || session.is_none() {
                        return Err("重复或无身份初始化".into());
                    }
                    submitted += 1;
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: input_id,
                            action: RuntimeAction::Submit {
                                input: vec![InputContent::Text(PROMPT.into())],
                            },
                        })
                        .await
                        .map_err(|_| "唯一输入发送失败")?;
                }
                RuntimeEventKind::MessageAccepted {
                    message_id,
                    turn_id,
                } => {
                    if message_id != input_id
                        || accepted != 0
                        || turn_id
                            .as_deref()
                            .is_none_or(|id| Uuid::parse_str(id).is_err())
                    {
                        return Err("接收回执不匹配".into());
                    }
                    accepted += 1;
                    turn = turn_id;
                }
                RuntimeEventKind::TurnStarted { turn_id } => {
                    if started || turn.as_deref() != Some(turn_id.as_str()) {
                        return Err("运行回合不匹配".into());
                    }
                    started = true;
                }
                RuntimeEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    method,
                    details,
                } => {
                    approval_count += 1;
                    let safe = case == "visible"
                        && approval_count == 1
                        && turn.as_deref() == Some(turn_id.as_str())
                        && method == "session/request_permission"
                        && details["sessionId"].as_str() == session.as_deref()
                        && details["toolCall"]["toolCallId"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty())
                        && exact_skill_read_permission(&details["toolCall"], &cwd, &skill_path);
                    let decision = if safe {
                        ApprovalDecision::AllowOnce
                    } else {
                        ApprovalDecision::DenyOnce
                    };
                    let control = Uuid::new_v4();
                    controls.insert(control);
                    controller
                        .send(RuntimeCommand {
                            generation,
                            message_id: control,
                            action: RuntimeAction::RespondApproval {
                                approval_id: approval_id.clone(),
                                decision,
                            },
                        })
                        .await
                        .map_err(|_| "技能只读审批发送失败")?;
                    if !safe {
                        return Err("已拒绝非精确技能文件的读取或其它工具".into());
                    }
                    let call_id = details["toolCall"]["toolCallId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or("读取审批缺少工具身份")?;
                    approved_calls.insert(call_id.to_owned());
                    pending_approvals.insert(approval_id);
                }
                RuntimeEventKind::ApprovalResolved {
                    approval_id,
                    decision,
                } => {
                    if decision != ApprovalDecision::AllowOnce
                        || !pending_approvals.remove(&approval_id)
                    {
                        return Err("读取审批回执不匹配".into());
                    }
                }
                RuntimeEventKind::CommandDispatched {
                    message_id,
                    turn_id,
                } => {
                    if !controls.remove(&message_id) || turn_id.as_deref() != turn.as_deref() {
                        return Err("读取审批写入回执不匹配".into());
                    }
                }
                RuntimeEventKind::LocalToolRequested { .. }
                | RuntimeEventKind::LocalToolCancelled { .. } => {
                    tool_count += 1;
                    return Err("技能验收出现应用工具".into());
                }
                RuntimeEventKind::TurnFinished {
                    turn_id,
                    outcome,
                    output,
                } => {
                    if !started
                        || turn.as_deref() != Some(turn_id.as_str())
                        || outcome != TurnOutcome::Completed
                    {
                        return Err("缺少真实完成条件".into());
                    }
                    let snapshots = histories.lock().expect("历史锁未损坏");
                    let snapshot = snapshots.get(&turn_id).ok_or("缺少生产最终回放")?;
                    let receipt = super::live_tests::final_response_receipt(
                        snapshot,
                        session.as_deref().ok_or("缺少会话")?,
                        &turn_id,
                        &outcome,
                    )?;
                    let updates = snapshot.replay["updates"]
                        .as_array()
                        .ok_or("最终回放无事件")?;
                    (tool_count, skill_read_count) = verify_skill_read_history(
                        updates,
                        &cwd,
                        &skill_path,
                        case,
                        &approved_calls,
                    )?;
                    read_scope_verified = true;
                    // 回放必须确实收到唯一原始 slash，不能由期望文本直接喂给模型。
                    let inputs: Vec<_> = updates
                        .iter()
                        .filter(|row| {
                            row["method"] == "session/update"
                                && row["params"]["update"]["sessionUpdate"] == "user_message_chunk"
                        })
                        .collect();
                    if inputs.len() != 1
                        || inputs[0]["params"]["update"]["content"]["type"] != "text"
                        || inputs[0]["params"]["update"]["content"]["text"] != PROMPT
                        || receipt.full_output != output
                        || !pending_approvals.is_empty()
                        || !controls.is_empty()
                    {
                        return Err("原生输入或精确读取边界失败".into());
                    }
                    let response = receipt.final_response.trim();
                    matched = response == sentinel;
                    final_verified = true;
                    final_sha256 = Some(hash(response.as_bytes()));
                    if response.is_empty()
                        || (case == "visible" && !matched)
                        || (case == "hidden" && receipt.full_output.contains(&sentinel))
                    {
                        return Err("技能随机标记对照失败".into());
                    }
                    return Ok(());
                }
                RuntimeEventKind::TextDelta { turn_id, .. }
                | RuntimeEventKind::Progress { turn_id, .. } => {
                    if turn.as_deref() != Some(turn_id.as_str()) {
                        return Err("其它回合输出".into());
                    }
                }
                RuntimeEventKind::ApprovalCancelled { .. }
                | RuntimeEventKind::InputJoined { .. }
                | RuntimeEventKind::RequestFailed { .. }
                | RuntimeEventKind::Disconnected { .. } => return Err("未允许的运行事件".into()),
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
    let transport_ok = matches!(&joined, Ok(Ok(Ok(()))));
    if joined.is_err() {
        task.abort();
    }
    let cleanup = managed_process::confirmed_exit(&state_dir, generation)
        .ok()
        .flatten()
        .is_some_and(|receipt| receipt.cleanup_confirmed);
    let skill_unchanged =
        fs::read_to_string(root.join(SKILL_PATH)).is_ok_and(|after| after == before);
    let passed = result.is_ok()
        && transport_ok
        && cleanup
        && skill_unchanged
        && submitted == 1
        && accepted == 1
        && final_verified
        && read_scope_verified
        && approval_count <= skill_read_count
        && (case == "visible" || (approval_count == 0 && tool_count == 0))
        && pending_approvals.is_empty()
        && controls.is_empty();
    record(file, catalog.evidence(session.as_deref(), case))?;
    record(
        file,
        json!({"event":"skill_finished","scope":SCOPE,"case":case,"passed":passed,
        "production_connect_path":true,"test_argv_override":true,"product_selected_skills_verified":false,
        "final_history_verified":final_verified,"native_skill_expansion_verified":passed && case=="visible",
        "hidden_control_verified":passed && case=="hidden","sentinel_matched":matched,
        "sentinel_sha256":hash(sentinel.as_bytes()),"final_response_sha256":final_sha256,
        "submitted_input_count":submitted,"accepted_input_count":accepted,"approval_count":approval_count,
        "native_tool_event_count":tool_count,"native_skill_read_count":skill_read_count,
        "exact_skill_read_only_verified":read_scope_verified,"transport_closed":transport_ok,"cleanup_confirmed":cleanup,
        "skill_snapshot_unchanged":skill_unchanged,"full_cli_parity_acceptance_passed":false}),
    )?;
    if passed {
        Ok(())
    } else {
        Err("原生技能黑盒验收未通过".into())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "必须由隔离运行器启动；每次只提交一个真实输入，会消耗已授权额度"]
async fn authenticated_native_skill_black_box() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("缺少隔离根目录"))
        .canonicalize()
        .unwrap();
    let case = env::var("INFINISHELL_GROK_SKILL_CASE").expect("缺少对照类型");
    assert!(matches!(case.as_str(), "visible" | "hidden"));
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
    for name in ["XAI_API_KEY", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
        assert!(env::var_os(name).is_none());
    }
    let artifact = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").unwrap());
    assert_eq!(artifact, root.join("private-evidence.ndjson"));
    let mut file = File::create(artifact).unwrap();
    record(&mut file, json!({"event":"skill_started","scope":SCOPE,"case":case,"max_native_inputs":1,
        "production_connect_path":true,"test_argv_override":true,"product_selected_skills_verified":false,
        "secret_in_submitted_prompt":false,"credential_values_recorded":false})).unwrap();
    assert!(
        exercise(&root, &case, &mut file).await.is_ok(),
        "原生技能验收失败；请查安全投影"
    );
}

#[path = "grok_native_skill_protocol_tests.rs"]
mod protocol_tests;
