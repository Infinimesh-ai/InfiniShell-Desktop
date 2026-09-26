//! 固定 Grok 生产入口的多技能、空闲新增与冷恢复；原生正文另由运行器逐字节审核。

use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;
use warp_cli::agent::Harness;

use super::connect;
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
use crate::ai::cli_agent_runtime::managed_input::prepare_managed_input;
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, PermissionPolicy, RuntimeAction, RuntimeCommand, RuntimeController,
    RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome,
    managed_process,
};

const PROMPT: &str = "请按本轮所选技能的顺序执行。只读取本轮选中技能的 SKILL.md；没有在本回合展开的技能必须重新 read_file，不能使用历史记忆。最终每个技能只输出其独立标记，每行一个，不解释。不执行命令、不修改文件、不访问网络。";

struct AbortOnDrop(JoinHandle<Result<(), RuntimeError>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn record(file: &mut File, value: Value) -> Result<(), String> {
    writeln!(file, "{value}")
        .and_then(|()| file.flush())
        .map_err(|_| "技能证据写入失败".into())
}

fn selection(root: &Path, keys: &[&str]) -> Result<Vec<SelectedLocalSkill>, String> {
    keys.iter()
        .map(|key| {
            let name = format!("isp-g06-{key}");
            let path = root
                .join("project/.grok/skills")
                .join(&name)
                .join("SKILL.md");
            Ok(SelectedLocalSkill {
                name,
                path: dunce::canonicalize(path).map_err(|_| "选中技能路径不可解析")?,
            })
        })
        .collect()
}

fn expected(root: &Path, keys: &[&str]) -> Result<String, String> {
    selection(root, keys)?
        .iter()
        .map(|skill| {
            let bytes = fs::read_to_string(&skill.path).map_err(|_| "技能夹具不可读")?;
            let markers: Vec<_> = bytes
                .lines()
                .filter(|line| line.starts_with("G06_"))
                .collect();
            if markers.len() != 1 || PROMPT.contains(markers[0]) {
                return Err("技能隐藏标记数量异常或泄漏到输入".into());
            }
            Ok(markers[0].to_owned())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|markers| markers.join("\n"))
}

fn exact_skill_read_permission(call: &Value, cwd: &Path, skill: &Path) -> bool {
    let Some(input) = call["rawInput"].as_object() else {
        return false;
    };
    let Some(target) = input.get("target_file").and_then(Value::as_str) else {
        return false;
    };
    let path = Path::new(target);
    call["kind"] == "read"
        && call["_meta"]["x.ai/tool"]["name"] == "read_file"
        && input
            .keys()
            .all(|key| key == "target_file" || key == "variant")
        && input.get("variant").is_none_or(|value| value == "ReadFile")
        && path.is_absolute()
        && path.starts_with(cwd)
        && dunce::canonicalize(path).ok().as_deref() == Some(skill)
}

async fn exercise_turn(
    root: &Path,
    case: &str,
    keys: &[&str],
    generation: Uuid,
    native: &str,
    controller: &RuntimeController,
    events: &mut mpsc::Receiver<RuntimeEvent>,
    file: &mut File,
) -> Result<(), String> {
    let skills = selection(root, keys)?;
    let parsed = skills
        .iter()
        .map(|skill| ai::skills::parse_skill(&skill.path).map_err(|_| "技能解析失败".to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    let input = prepare_managed_input(
        Harness::Grok,
        PROMPT.into(),
        &[],
        parsed,
        &root.join("attachments"),
    )?;
    let message = Uuid::new_v4();
    let command = RuntimeCommand {
        generation,
        message_id: message,
        action: RuntimeAction::Submit { input },
    };
    let answer = expected(root, keys)?;
    let serialized = serde_json::to_string(&command).map_err(|_| "技能输入序列化失败")?;
    if answer.lines().any(|marker| serialized.contains(marker)) {
        return Err("技能秘密正文进入提交参数".into());
    }
    let mut stale = command.clone();
    stale.generation = Uuid::new_v4();
    if !matches!(
        controller.send(stale).await,
        Err(RuntimeError::StaleGeneration)
    ) {
        return Err("旧代次输入未被控制器拒绝".into());
    }
    record(
        file,
        json!({"event":"submitted","case":case,"generation":generation,
        "native_session_id":native,"message_id":message,"input":command.action,
        "selected_skills":skills,"stale_generation_rejected":true,"markers_absent_from_input":true}),
    )?;
    controller
        .send(command.clone())
        .await
        .map_err(|_| "技能提交失败")?;
    controller
        .send(command.clone())
        .await
        .map_err(|_| "运行中消息重投失败")?;
    let mut turn = None;
    let mut started = false;
    let mut controls = HashSet::new();
    let mut approvals = HashSet::new();
    let mut approval_count = 0;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(150), events.recv())
            .await
            .map_err(|_| "技能回合事件超时")?
            .ok_or("技能回合事件流关闭")?;
        if event.generation != generation || event.native_session_id.as_deref() != Some(native) {
            return Err("技能事件关联错误会话或旧代次".into());
        }
        match event.kind {
            RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id,
            } => {
                if message_id != message || turn.is_some() || turn_id.is_none() {
                    return Err("技能 ACK 缺失身份、重复或属于其他消息".into());
                }
                turn = turn_id;
                record(
                    file,
                    json!({"event":"accepted","case":case,"generation":generation,
                    "native_session_id":native,"message_id":message,"turn_id":turn}),
                )?;
            }
            RuntimeEventKind::TurnStarted { turn_id } => {
                if started || turn.as_ref() != Some(&turn_id) {
                    return Err("技能回合重复或身份错误".into());
                }
                started = true;
            }
            RuntimeEventKind::TextDelta { turn_id, .. }
            | RuntimeEventKind::Progress { turn_id, .. } => {
                if turn.as_ref() != Some(&turn_id) {
                    return Err("技能输出越过回合边界".into());
                }
            }
            RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } => {
                approval_count += 1;
                let matched = skills.iter().find(|skill| {
                    exact_skill_read_permission(
                        &details["toolCall"],
                        &root.join("project"),
                        &skill.path,
                    )
                });
                let allowed = approval_count <= 8
                    && turn.as_ref() == Some(&turn_id)
                    && method == "session/request_permission"
                    && details["sessionId"] == native
                    && matched.is_some()
                    && !approvals.contains(&approval_id);
                let decision = if allowed {
                    ApprovalDecision::AllowOnce
                } else {
                    ApprovalDecision::DenyOnce
                };
                record(
                    file,
                    json!({"event":"approval","case":case,"generation":generation,
                    "native_session_id":native,"turn_id":turn_id,"approval_id":approval_id,
                    "tool_call_id":details["toolCall"]["toolCallId"],"allowed_once":allowed,
                    "matched_skill":matched.map(|skill| &skill.name),"method":method}),
                )?;
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
                    .map_err(|_| "技能审批响应发送失败")?;
                if !allowed {
                    return Err("额外或未精确绑定的工具已拒绝".into());
                }
                approvals.insert(approval_id);
            }
            RuntimeEventKind::ApprovalResolved {
                approval_id,
                decision,
            } => {
                if decision != ApprovalDecision::AllowOnce || !approvals.remove(&approval_id) {
                    return Err("审批回执未匹配本轮一次性授权".into());
                }
            }
            RuntimeEventKind::CommandDispatched { message_id, .. } => {
                if !controls.remove(&message_id) {
                    return Err("收到未知控制回执".into());
                }
            }
            RuntimeEventKind::TurnFinished {
                turn_id,
                outcome,
                output,
            } => {
                record(
                    file,
                    json!({"event":"finished","case":case,"generation":generation,
                    "native_session_id":native,"message_id":message,"turn_id":turn_id,
                    "outcome":outcome,"output":output,"expected":answer,"approval_count":approval_count}),
                )?;
                if !started
                    || turn.as_ref() != Some(&turn_id)
                    || outcome != TurnOutcome::Completed
                    // 生产输出包含工具执行前的说明；原生最终答案由收据脚本单独核验。
                    || !output.trim().ends_with(&answer)
                    || !approvals.is_empty()
                    || !controls.is_empty()
                {
                    return Err("技能终态、标记、回合关联或审批闭合不匹配".into());
                }
                break;
            }
            RuntimeEventKind::LocalToolRequested { request } => {
                controller
                    .send(RuntimeCommand {
                        generation,
                        message_id: Uuid::new_v4(),
                        action: RuntimeAction::RespondLocalTool {
                            turn_id: request.turn_id,
                            call_id: request.call_id,
                            result: Err("技能验收不授权应用本地工具".into()),
                        },
                    })
                    .await
                    .map_err(|_| "额外本地工具拒绝失败")?;
                return Err("意外应用本地工具请求已拒绝".into());
            }
            RuntimeEventKind::RequestFailed {
                message_id,
                message,
            } => {
                record(
                    file,
                    json!({"event":"request_failed","case":case,"message_id":message_id,"message":message}),
                )?;
                return Err("原生技能请求失败，已保留原始错误".into());
            }
            RuntimeEventKind::SessionReady { .. }
            | RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::Disconnected { .. } => {
                return Err("技能回合出现未预期事件".into());
            }
        }
    }
    controller
        .send(command)
        .await
        .map_err(|_| "完成后重复消息发送失败")?;
    // 空闲观察只用于发现重复执行；最终仍用原生持久历史核对恰好三次模型输入。
    match tokio::time::timeout(Duration::from_millis(750), events.recv()).await {
        Err(_) => {}
        Ok(Some(_)) => return Err("完成后相同消息产生额外事件".into()),
        Ok(None) => return Err("完成后连接意外关闭".into()),
    }
    record(
        file,
        json!({"event":"replay_quiet","case":case,"message_id":message}),
    )?;
    Ok(())
}

async fn exercise_connection(
    root: &Path,
    previous: Option<String>,
    file: &mut File,
) -> Result<String, String> {
    let generation = Uuid::new_v4();
    let resumed = previous.is_some();
    let selected = if resumed {
        selection(root, &["alpha", "beta", "gamma"])?
    } else {
        selection(root, &["alpha", "beta"])?
    };
    let state_dir = root.join("state");
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("缺少固定 CLI 路径")?,
        ),
        cwd: dunce::canonicalize(root.join("project")).map_err(|_| "隔离项目路径不可解析")?,
        state_dir: state_dir.clone(),
        target: previous
            .clone()
            .map_or(SessionTarget::New, |native_session_id| {
                SessionTarget::Resume { native_session_id }
            }),
        generation,
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: selected.clone(),
    };
    record(
        file,
        json!({"event":"connecting","generation":generation,"resumed":resumed,
        "requested_native_session_id":previous,"selected_skills":selected,"policy":"inherit",
        "parent_ceiling":null,"profile_override":false,"model_override":false,"local_tools":false}),
    )?;
    let connection = connect(options).map_err(|_| "生产 Grok 连接拒绝此配置")?;
    let controller = connection.controller;
    let mut events = connection.events;
    let mut task = AbortOnDrop(tokio::spawn(connection.task));
    let result = async {
        let ready = tokio::time::timeout(Duration::from_secs(90), events.recv()).await
            .map_err(|_| "生产握手超时")?.ok_or("生产握手事件流关闭")?;
        let native = ready.native_session_id.ok_or("握手缺少原生会话")?;
        let RuntimeEventKind::SessionReady { verified_cli_version, effective_permissions } = ready.kind else {
            return Err("首个生产事件不是初始化成功".to_owned());
        };
        let model = &effective_permissions["reportedMetadata"]["models"]["currentModelId"];
        if ready.generation != generation || verified_cli_version.as_deref() != Some("1.0.41")
            || effective_permissions["requestedPolicy"] != "inherit" || model != "grok-4.7"
            || previous.as_ref().is_some_and(|id| id != &native) {
            return Err("固定版本、实际模型、权限或恢复会话不匹配".into());
        }
        record(file, json!({"event":"ready","generation":generation,"resumed":resumed,
            "native_session_id":native,"version":verified_cli_version,"model":model,"policy":"inherit"}))?;
        if resumed {
            // 此处明确由验收调用方提供已接受技能；不冒充协调器已经持久恢复。
            match tokio::time::timeout(Duration::from_millis(750), events.recv()).await {
                Err(_) => {},
                Ok(Some(_)) => return Err("恢复后未提交新消息却出现执行事件".into()),
                Ok(None) => return Err("冷恢复空闲连接关闭".into()),
            }
            record(file, json!({"event":"resume_idle_without_replay","generation":generation,"native_session_id":native}))?;
            exercise_turn(root, "cold", &["gamma", "beta"], generation, &native, &controller, &mut events, file).await?;
        } else {
            exercise_turn(root, "initial", &["alpha", "beta"], generation, &native, &controller, &mut events, file).await?;
            let target = root.join("project/.grok/skills/isp-g06-gamma");
            if target.exists() { return Err("热新增技能在首轮前已经发布".into()); }
            fs::rename(root.join("unpublished/isp-g06-gamma"), &target).map_err(|_| "第三技能发布失败")?;
            record(file, json!({"event":"skill_published","generation":generation,"native_session_id":native,
                "path":target.join("SKILL.md"),"after_case":"initial","same_connection":true}))?;
            exercise_turn(root, "hot", &["gamma"], generation, &native, &controller, &mut events, file).await?;
        }
        Ok(native)
    }.await;
    // 无论模型或证据断言是否失败，都先请求同一生产连接关闭；失败仍原样报告。
    let shutdown = Uuid::new_v4();
    let _ = controller
        .send(RuntimeCommand {
            generation,
            message_id: shutdown,
            action: RuntimeAction::Shutdown,
        })
        .await;
    let closed = tokio::time::timeout(Duration::from_secs(35), &mut task.0).await;
    let task_ok = matches!(&closed, Ok(Ok(Ok(()))));
    let receipt = managed_process::confirmed_exit(&state_dir, generation)
        .ok()
        .flatten();
    let mut extra_turns = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(
            event.kind,
            RuntimeEventKind::TurnStarted { .. }
                | RuntimeEventKind::TurnFinished { .. }
                | RuntimeEventKind::MessageAccepted { .. }
        ) {
            extra_turns += 1;
        }
    }
    record(
        file,
        json!({"event":"cleanup","generation":generation,"resumed":resumed,
        "task_finished_ok":task_ok,"receipt":receipt,"extra_turn_events":extra_turns}),
    )?;
    if let Err(reason) = result {
        return Err(reason);
    }
    if !task_ok
        || extra_turns != 0
        || !receipt
            .as_ref()
            .is_some_and(|value| value.cleanup_confirmed && value.exit_code == Some(0))
    {
        return Err("生产进程未自然退出或清理未确认，不能用强制回收代替".into());
    }
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要固定且已认证的 Grok、生产监督者和三次在线模型输入；原生证据须由配套运行器审核"]
async fn real_grok_multi_skill_hot_add_and_resume() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-multi-skill-probe")).unwrap(),
        "isolated Grok multi skill verification\n"
    );
    let mut file = File::create(root.join("events.ndjson")).unwrap();
    record(&mut file, json!({"event":"started","max_native_inputs":3,"production_connect":true,
        "candidate_flags_used":false,"coordinator_persistence_verified":false,"gui_verified":false})).unwrap();
    let result = async {
        let first = exercise_connection(&root, None, &mut file).await?;
        let resumed = exercise_connection(&root, Some(first.clone()), &mut file).await?;
        if resumed != first { return Err("冷恢复更换了原生会话".to_owned()); }
        record(&mut file, json!({"event":"adapter_flow_passed","native_session_id":first,"native_inputs":3,
            "native_byte_audit_pending":true,"runtime_generations":2,"coordinator_persistence_verified":false}))
    }.await;
    if let Err(reason) = &result {
        record(
            &mut file,
            json!({"event":"acceptance_failed","reason":reason}),
        )
        .unwrap();
    }
    assert!(result.is_ok(), "Grok 生产技能流程失败，原始证据已保留");
}
