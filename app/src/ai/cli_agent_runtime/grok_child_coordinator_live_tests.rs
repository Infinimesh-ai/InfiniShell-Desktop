//! 真实 Grok 固定创建策略的父子协调器验收；冷加载不代表应用进程重启。

use std::env;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};

use super::super::local_tools::{LocalToolPermissions, MCP_SERVER_NAME, NativeLocalToolRequest};
use super::super::managed_process::confirmed_exit;
use super::super::permissions::{GrokCreationPolicyV1, ceiling_from_parent};
use super::super::{ApprovalDecision, current_state_dir};
use super::*;
use crate::persistence::local_cli_tasks::load_task_messages;
use crate::persistence::model::LocalCliReceiptKind;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent::{
    CLIAgentInstallModel, CLIAgentInstallation, CLIAgentVersionStatus,
};

const SCOPE: &str = "real_grok_fixed_policy_parent_child";
const MAX_INPUTS: usize = 6;
const MAX_TOOLS: usize = 4;
const MAX_SECONDS: u64 = 450;

fn sha(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

struct Evidence(File);
impl Evidence {
    fn record(&mut self, mut value: Value) -> Result<(), String> {
        value["scope"] = json!(SCOPE);
        writeln!(
            self.0,
            "{}",
            serde_json::to_string(&value).map_err(|_| "证据编码失败")?
        )
        .map_err(|_| "证据写入失败")?;
        self.0.flush().map_err(|_| "证据刷新失败".into())
    }
}

struct Markers {
    initial: String,
    final_result: String,
    collected: String,
    progress_result: String,
    automatic_result: String,
    followup: String,
    progress: String,
}
impl Markers {
    fn new() -> Self {
        let id = Uuid::new_v4();
        Self {
            initial: format!("GROK_CHILD_INITIAL_{id}"),
            final_result: format!("GROK_CHILD_FINAL_{id}"),
            collected: format!("GROK_PARENT_COLLECTED_{id}"),
            progress_result: format!("GROK_PARENT_PROGRESS_{id}"),
            automatic_result: format!("GROK_PARENT_AUTOMATIC_{id}"),
            followup: format!("followup-{id}"),
            progress: format!("progress-{id}"),
        }
    }
    fn discovery() -> &'static str {
        "Use the SDK MCP server infinishell-local-tasks via use_tool. If discovery is needed, call search_tool at most once with exactly {\"limit\":5,\"query\":\"infinishell-local-tasks\"}. Do not use any other tools."
    }
    fn spawn(&self, parent: &str, file: &Path) -> Value {
        json!({"summary":"isolated Grok parent-child verification", "base_prompt":"", "harness":"grok", "model_id":"", "skills":[],
            "agent_run_configs":[{"name":"verified-child","prompt":format!(
                "This is an authorized isolated integration test. Your parent task ID is {parent}. In this first turn call read_file exactly once with target_file {}. Wait normally for host approval, then finish with only {}. Do not spawn agents or use other tools in this first turn. In any later follow-up obey its instructions. {}",
                json!(file), self.initial, Self::discovery())}]})
    }
    fn followup_text(&self, parent: &str) -> String {
        format!(
            "This is your second input. Call send_message_to_agent exactly once with addresses [{parent:?}], subject {:?}, message {:?}. A result may be unconfirmed while the parent is busy; do not resend, retry or call inspect. Finish with only {}. Do not read files or spawn tasks. {}",
            self.progress,
            format!("Do not use tools. Reply only {}.", self.progress_result),
            self.final_result,
            Self::discovery()
        )
    }
    fn prompt(&self, parent: &str, file: &Path) -> String {
        format!(
            "This is an authorized isolated production-coordinator test. {} Call run_agents exactly once with {}. Retain the returned child task_id. Next call send_message_to_agent exactly once with addresses containing only that child ID, subject {:?}, and message {}. The host allows the send only while the child is awaiting its read approval. Sent and unconfirmed are attempts, not ACK; never resend. Then call inspect_local_tasks exactly once with exactly one argument, task_ids, containing only that child ID; omit result_generation and result_offset. The host holds that approval until the child's second input has finished. Require the inspected result to equal {} and finish with only {}. Do not spawn again. Later inputs with Subject: local_task_result are automatic results; do not call tools for them, reply only {}. Later progress messages specify their own reply marker. Keep these later inputs separate from this initial task.",
            Self::discovery(),
            self.spawn(parent, file),
            self.followup,
            json!(self.followup_text(parent)),
            self.final_result,
            self.collected,
            self.automatic_result
        )
    }
}

type Wake = Option<(LocalCliTask, RuntimeEvent)>;
async fn records(sender: &SyncSender<ModelEvent>) -> Result<Vec<LocalCliTask>, String> {
    load_tasks(sender, false)?
        .await
        .map_err(|_| "读取任务确认关闭")?
}
async fn messages(
    sender: &SyncSender<ModelEvent>,
    id: &str,
) -> Result<Vec<LocalCliMessage>, String> {
    load_task_messages(sender, id.to_owned())?
        .await
        .map_err(|_| "读取消息确认关闭")?
}
async fn history(sender: &SyncSender<ModelEvent>, id: &str) -> Result<Vec<LocalCliTask>, String> {
    load_task_generations(sender, id.to_owned())?
        .await
        .map_err(|_| "读取历史确认关闭")?
}
fn native_ack(message: &LocalCliMessage) -> bool {
    message.state == LocalCliMessageState::Acknowledged
        && message.receipt_kind == Some(LocalCliReceiptKind::NativeProtocol)
}
async fn approve(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    task: &LocalCliTask,
    approval: &ManagedApproval,
) -> Result<(), String> {
    coordinator
        .update(app, |model, _| {
            model.request_for_generation(
                &task.task_id,
                task.generation,
                Uuid::new_v4(),
                RuntimeAction::RespondApproval {
                    approval_id: approval.approval_id.clone(),
                    decision: ApprovalDecision::AllowOnce,
                },
            )
        })?
        .with_timeout(Duration::from_secs(15))
        .await
        .map_err(|_| "审批确认超时")?
        .map_err(|_| "审批确认关闭")?
}

async fn verify_saved_chain(
    sender: &SyncSender<ModelEvent>,
    parent_id: &str,
    child_id: &str,
    markers: &Markers,
    evidence: &mut Evidence,
) -> Result<bool, String> {
    let saved = records(sender).await?;
    let parent = saved
        .iter()
        .find(|task| task.task_id == parent_id)
        .ok_or("父记录丢失")?;
    let child = saved
        .iter()
        .find(|task| task.task_id == child_id)
        .ok_or("子记录丢失")?;
    let parents = history(sender, parent_id).await?;
    let children = history(sender, child_id).await?;
    let source = parents
        .iter()
        .find(|task| task.generation == 1)
        .ok_or("创建父代丢失")?;
    let parent_config: Value =
        serde_json::from_str(&source.config_json).map_err(|_| "父配置无效")?;
    let child_config: Value = serde_json::from_str(&child.config_json).map_err(|_| "子配置无效")?;
    let parent_profile: GrokCreationPolicyV1 =
        serde_json::from_value(parent_config["grok_profile"].clone())
            .map_err(|_| "父固定策略丢失")?;
    let child_profile: GrokCreationPolicyV1 =
        serde_json::from_value(child_config["grok_profile"].clone())
            .map_err(|_| "子固定策略丢失")?;
    parent_profile
        .validate_child(&child_profile)
        .map_err(|_| "子创建策略越界")?;
    let ceiling = ceiling_from_parent(source, "grok").map_err(|_| "父权限上限丢失")?;
    if saved.len() != 2
        || child.parent_task_id.as_deref() != Some(parent_id)
        || child.parent_generation != Some(1)
        || child_config["permission_ceiling"] != json!(ceiling)
        || parent_config["grok_profile"] == child_config["grok_profile"]
        || child_config["effective_permissions"]["grokCreationPolicyV1"]
            != child_config["grok_profile"]
        || child_config["effective_permissions"]["appCreationPolicyApplied"] != true
        || child_config["effective_permissions"]["permissionEnforcementVerified"] != false
    {
        return Err("实际亲缘、独占存储或创建策略不符".into());
    }
    let rows = messages(sender, parent_id).await?;
    let child_rows = messages(sender, child_id).await?;
    let all = rows
        .iter()
        .chain(&child_rows)
        .map(|row| (row.message_id.clone(), row))
        .collect::<HashMap<_, _>>();
    let calls = all
        .values()
        .filter(|row| row.subject == "native_tool_call")
        .collect::<Vec<_>>();
    if calls.len() > MAX_TOOLS {
        return Err("原生 SDK 调用超过固定预算".into());
    }
    let mut unique = HashSet::new();
    for row in &calls {
        let request: NativeLocalToolRequest =
            serde_json::from_str(&row.body).map_err(|_| "工具记录无效")?;
        let correct = match request.tool.as_str() {
            "run_agents" => row.sender_task_id == parent_id && row.sender_generation == 1,
            "inspect_local_tasks" => {
                row.sender_task_id == parent_id
                    && row.sender_generation == 1
                    && request.arguments["task_ids"] == json!([child_id])
            }
            "send_message_to_agent" => {
                (row.sender_task_id == parent_id
                    && row.sender_generation == 1
                    && request.arguments["subject"] == markers.followup
                    && request.arguments["addresses"] == json!([child_id]))
                    || (row.sender_task_id == child_id
                        && row.sender_generation == 2
                        && request.arguments["subject"] == markers.progress
                        && request.arguments["addresses"] == json!([parent_id]))
            }
            _ => false,
        };
        if !correct || !unique.insert((row.sender_task_id.clone(), request.tool)) {
            return Err("重复或范围外原生工具".into());
        }
    }
    let followups = rows
        .iter()
        .filter(|row| row.subject == markers.followup)
        .collect::<Vec<_>>();
    let progress = rows
        .iter()
        .filter(|row| row.subject == markers.progress)
        .collect::<Vec<_>>();
    let results = rows
        .iter()
        .filter(|row| row.subject == "local_task_result" && row.sender_task_id == child_id)
        .collect::<Vec<_>>();
    let inspected = rows
        .iter()
        .filter(|row| row.subject == "native_tool_result" && row.sender_task_id == parent_id)
        .filter_map(|row| serde_json::from_str::<Result<Value, String>>(&row.body).ok())
        .filter_map(Result::ok)
        .any(|value| {
            value["tasks"].as_array().is_some_and(|tasks| {
                tasks.iter().any(|task| {
                    task["task_id"] == child_id
                        && task["generation"] == 2
                        && task["state"] == "completed"
                        && task["result"] == markers.final_result
                })
            })
        });
    let complete = |task: &LocalCliTask, marker: &str| {
        task.state == LocalCliTaskState::Completed
            && task.result.as_deref() == Some(marker)
            && task.terminal_evidence.is_some()
    };
    if !(calls.len() == MAX_TOOLS
        && parents.len() == 4
        && children.len() == 2
        && children
            .iter()
            .any(|task| task.generation == 1 && complete(task, &markers.initial))
        && children
            .iter()
            .any(|task| task.generation == 2 && complete(task, &markers.final_result))
        && complete(source, &markers.collected)
        && parents
            .iter()
            .filter(|task| complete(task, &markers.automatic_result))
            .count()
            == 2
        && parents
            .iter()
            .filter(|task| complete(task, &markers.progress_result))
            .count()
            == 1
        && inspected
        && followups.len() == 1
        && progress.len() == 1
        && native_ack(followups[0])
        && native_ack(progress[0])
        && results.len() == 2
        && children.iter().all(|task| {
            results
                .iter()
                .filter(|row| {
                    row.sender_generation == task.generation
                        && row.recipient_task_id == parent_id
                        && row.recipient_generation == 1
                        && native_ack(row)
                })
                .count()
                == 1
        }))
    {
        return Ok(false);
    }
    if parent.native_session_id.is_none()
        || child.native_session_id.is_none()
        || parent.native_session_id == child.native_session_id
    {
        return Err("独立父子原生会话证明不成立".into());
    }
    evidence.record(json!({"event":"chain_verified","tasks":2,"parent_generations":4,"child_generations":2,
        "native_inputs":6,"sdk_tool_calls":4,"native_ack_both_directions":true,"automatic_result_ack":true,
        "final_result_via_inspect":true,"parent_binding_verified":true,"child_creation_policy_verified":true,
        "parent_native_sha256":sha(parent.native_session_id.as_deref().unwrap()),"child_native_sha256":sha(child.native_session_id.as_deref().unwrap()),
        "parent_result_sha256":sha(&markers.collected),"child_result_sha256":sha(&markers.final_result)}))?;
    Ok(true)
}

async fn drive(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    sender: &SyncSender<ModelEvent>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    root: &Path,
    evidence: &mut Evidence,
) -> Result<String, String> {
    let parent_id = Uuid::new_v4().to_string();
    let cwd = root
        .join("project")
        .canonicalize()
        .map_err(|_| "项目目录缺失")?;
    let file = cwd.join("child-read.txt");
    let markers = Markers::new();
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("固定 CLI 缺失")?,
        ),
        cwd: cwd.clone(),
        state_dir: current_state_dir(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::GrokRestrictedReadV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: Some(LocalToolPermissions {
            allow_spawn: true,
            allow_message: true,
        }),
        selected_skills: Vec::new(),
    };
    let task = LocalCliTask {
        version: 1,
        task_id: parent_id.clone(),
        parent_task_id: None,
        parent_generation: None,
        harness: "grok".into(),
        working_directory: cwd.to_string_lossy().into(),
        config_json: "{}".into(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    coordinator.update(app, |model, ctx| {
        model.start_with_input(
            task,
            options,
            None,
            Uuid::new_v4(),
            vec![InputContent::Text(markers.prompt(&parent_id, &file))],
            ctx,
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(MAX_SECONDS);
    let mut accepted = HashMap::new();
    let mut started = HashSet::new();
    let mut finished = HashSet::new();
    let mut calls = HashSet::new();
    let mut approved = HashSet::new();
    let mut operations = HashSet::new();
    let mut searches = HashSet::new();
    let mut gate = false;
    loop {
        let wake = receiver
            .recv()
            .with_timeout(deadline.saturating_duration_since(Instant::now()))
            .await
            .map_err(|_| "父子整链超时")?
            .ok_or("协调器事件关闭")?;
        if let Some((task, event)) = wake {
            match event.kind {
                RuntimeEventKind::MessageAccepted {
                    message_id,
                    turn_id,
                } => {
                    let turn = turn_id.ok_or("真实 ACK 缺少回合身份")?;
                    if accepted
                        .insert((task.task_id.clone(), message_id), turn)
                        .is_some()
                        || accepted.len() > MAX_INPUTS
                    {
                        return Err("输入重复或超过六条预算".into());
                    }
                }
                RuntimeEventKind::TurnStarted { turn_id } => {
                    if !accepted
                        .iter()
                        .any(|((id, _), turn)| id == &task.task_id && turn == &turn_id)
                        || !started.insert((task.task_id.clone(), turn_id))
                    {
                        return Err("运行未关联 ACK 或重复".into());
                    }
                }
                RuntimeEventKind::TurnFinished {
                    turn_id, outcome, ..
                } => {
                    if outcome != TurnOutcome::Completed
                        || !started.contains(&(task.task_id.clone(), turn_id.clone()))
                        || !finished.insert((task.task_id.clone(), turn_id))
                    {
                        return Err("父子回合没有真实唯一完成".into());
                    }
                }
                RuntimeEventKind::LocalToolRequested { request } => {
                    if !calls.insert((task.task_id.clone(), request.call_id))
                        || calls.len() > MAX_TOOLS
                    {
                        return Err("SDK 调用重复或超预算".into());
                    }
                }
                RuntimeEventKind::SessionReady { .. }
                | RuntimeEventKind::TextDelta { .. }
                | RuntimeEventKind::Progress { .. }
                | RuntimeEventKind::ApprovalRequested { .. }
                | RuntimeEventKind::ApprovalResolved { .. }
                | RuntimeEventKind::CommandDispatched { .. } => {}
                RuntimeEventKind::InputJoined { .. }
                | RuntimeEventKind::RequestFailed { .. }
                | RuntimeEventKind::Disconnected { .. }
                | RuntimeEventKind::ApprovalCancelled { .. }
                | RuntimeEventKind::LocalToolCancelled { .. } => {
                    return Err("父子运行发生非预期取消、合并、失败或断开".into());
                }
            }
        }
        let snapshots = coordinator.read(app, |model, _| {
            model.snapshots().cloned().collect::<Vec<_>>()
        });
        if snapshots.iter().any(|task| task.error.is_some()) {
            return Err("生产协调器报错".into());
        }
        let children = snapshots
            .iter()
            .filter(|task| task.task.parent_task_id.as_deref() == Some(&parent_id))
            .collect::<Vec<_>>();
        if children.len() > 1 {
            return Err("产生多余子任务".into());
        }
        let child = children.first().copied();
        let waiting = child.is_some_and(|task| {
            task.approvals.iter().any(|approval| {
                approval.details["toolCall"]["_meta"]["x.ai/tool"]["name"] == "read_file"
            })
        });
        let mut sent = false;
        if let Some(child) = child {
            let rows = messages(sender, &child.task.task_id).await?;
            let rows = rows
                .iter()
                .filter(|row| row.subject == markers.followup)
                .collect::<Vec<_>>();
            if rows.len() > 1 {
                return Err("追加指令被重投".into());
            }
            sent = rows.len() == 1
                && rows[0].sender_task_id == parent_id
                && rows[0].recipient_task_id == child.task.task_id
                && rows[0].state == LocalCliMessageState::Sent
                && rows[0].receipt_kind.is_none();
            if sent && waiting && !gate {
                evidence.record(json!({"event":"queued_before_read_allow","sent":true,"native_ack":false,"child_waiting_approval":true}))?;
                gate = true;
            }
        }
        for snapshot in &snapshots {
            for approval in &snapshot.approvals {
                let identity = (snapshot.task.task_id.clone(), approval.approval_id.clone());
                if approved.contains(&identity) {
                    continue;
                }
                let call = &approval.details["toolCall"];
                let name = call["_meta"]["x.ai/tool"]["name"]
                    .as_str()
                    .ok_or("审批缺少原生工具名")?;
                let raw = &call["rawInput"];
                let parent = snapshot.task.task_id == parent_id;
                match name {
                    "read_file"
                        if !parent
                            && call["kind"] == "read"
                            && *raw == json!({"variant":"ReadFile","target_file":file}) =>
                    {
                        // ACP 在原回合结束后才确认后续输入，不能在这里等 NativeProtocol ACK。
                        if !sent {
                            continue;
                        }
                    }
                    "search_tool"
                        if *raw
                            == json!({"variant":"SearchTool","limit":5,"query":MCP_SERVER_NAME})
                            || *raw == json!({"limit":5,"query":MCP_SERVER_NAME}) =>
                    {
                        if !searches.insert(snapshot.task.task_id.clone()) {
                            return Err("重复工具发现".into());
                        }
                    }
                    "use_tool"
                        if call["kind"] == "other"
                            && raw["variant"] == "UseTool"
                            && raw.as_object().is_some_and(|raw| raw.len() == 3) =>
                    {
                        let target = raw["tool_name"].as_str().ok_or("SDK 目标缺失")?;
                        let input = &raw["tool_input"];
                        if target == format!("{MCP_SERVER_NAME}__run_agents")
                            && parent
                            && child.is_none()
                            && *input == markers.spawn(&parent_id, &file)
                        {
                        } else if target == format!("{MCP_SERVER_NAME}__send_message_to_agent")
                            && parent
                            && child.is_some_and(|child| {
                                input["addresses"] == json!([child.task.task_id])
                            })
                            && input["subject"] == markers.followup
                            && input["message"] == markers.followup_text(&parent_id)
                            && input.as_object().is_some_and(|input| input.len() == 3)
                        {
                            if !waiting {
                                continue;
                            }
                        } else if target == format!("{MCP_SERVER_NAME}__send_message_to_agent")
                            && !parent
                            && *input
                                == json!({"addresses":[parent_id],"subject":markers.progress,"message":format!("Do not use tools. Reply only {}.",markers.progress_result)})
                        {
                        } else if target == format!("{MCP_SERVER_NAME}__inspect_local_tasks")
                            && parent
                            && child.is_some_and(|child| {
                                *input == json!({"task_ids":[child.task.task_id]})
                            })
                        {
                            if !child.is_some_and(|child| {
                                child.task.generation == 2
                                    && child.task.state == LocalCliTaskState::Completed
                                    && child.task.result.as_deref()
                                        == Some(markers.final_result.as_str())
                            }) {
                                continue;
                            }
                        } else {
                            return Err("范围外 SDK 目标或参数".into());
                        }
                    }
                    _ => return Err("范围外原生工具审批".into()),
                }
                let operation = if name == "use_tool" {
                    raw["tool_name"].as_str().ok_or("SDK 目标缺失")?
                } else {
                    name
                };
                if !operations.insert((snapshot.task.task_id.clone(), operation.to_owned())) {
                    return Err("禁止重复允许同一验收操作".into());
                }
                if approved.len() >= MAX_TOOLS + 3 {
                    return Err("审批超过固定预算".into());
                }
                approve(app, coordinator, &snapshot.task, approval).await?;
                approved.insert(identity);
            }
        }
        if gate
            && accepted.len() == MAX_INPUTS
            && started.len() == MAX_INPUTS
            && finished.len() == MAX_INPUTS
            && calls.len() == MAX_TOOLS
            && snapshots
                .iter()
                .all(|task| task.task.state == LocalCliTaskState::Completed)
            && let Some(child) = child
            && verify_saved_chain(sender, &parent_id, &child.task.task_id, &markers, evidence)
                .await?
        {
            return Ok(parent_id);
        }
    }
}

async fn shutdown(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    cleaned: &mut HashSet<Uuid>,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let ids = coordinator.read(app, |model, _| {
        model
            .snapshots()
            .filter(|task| task.connected)
            .map(|task| task.task.task_id.clone())
            .collect::<Vec<_>>()
    });
    for id in ids {
        if let Ok(reply) = coordinator.update(app, |model, _| {
            model.request(&id, Uuid::new_v4(), RuntimeAction::Shutdown)
        }) {
            let _ = reply.with_timeout(Duration::from_secs(5)).await;
        }
    }
    let deadline = Instant::now() + Duration::from_secs(35);
    while coordinator.read(app, |model, _| model.snapshots().any(|task| task.connected)) {
        receiver
            .recv()
            .with_timeout(deadline.saturating_duration_since(Instant::now()))
            .await
            .map_err(|_| "协调器断开超时")?
            .ok_or("清理事件关闭")?;
    }
    let snapshots = coordinator.read(app, |model, _| {
        model.snapshots().cloned().collect::<Vec<_>>()
    });
    for snapshot in snapshots {
        let config: Value =
            serde_json::from_str(&snapshot.task.config_json).map_err(|_| "退出配置无效")?;
        let generation: Uuid = serde_json::from_value(config["runtime_generation"].clone())
            .map_err(|_| "退出代次无效")?;
        if cleaned.contains(&generation) {
            continue;
        }
        loop {
            if let Some(receipt) =
                confirmed_exit(&current_state_dir(), generation).map_err(|_| "退出回执无效")?
            {
                if !receipt.cleanup_confirmed {
                    return Err("真实清理未确认".into());
                }
                break;
            }
            if Instant::now() >= deadline {
                return Err("真实监督退出回执缺失".into());
            }
            Timer::after(Duration::from_millis(50)).await;
        }
        evidence.record(json!({"event":"cleanup_verified","runtime_sha256":sha(&generation.to_string()),"cleanup_confirmed":true}))?;
        cleaned.insert(generation);
    }
    while let Ok(wake) = receiver.try_recv() {
        if wake.is_some_and(|(_, event)| {
            !matches!(
                event.kind,
                RuntimeEventKind::Disconnected { .. } | RuntimeEventKind::CommandDispatched { .. }
            )
        }) {
            return Err("关闭后有未消费执行事件".into());
        }
    }
    Ok(())
}

async fn resume_ready(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    sender: &SyncSender<ModelEvent>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    parent_id: &str,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let previous = records(sender)
        .await?
        .into_iter()
        .find(|task| task.task_id == parent_id)
        .ok_or("继续前父记录缺失")?;
    let before =
        serde_json::to_value(messages(sender, parent_id).await?).map_err(|_| "消息摘要失败")?;
    let config: Value = serde_json::from_str(&previous.config_json).map_err(|_| "继续配置无效")?;
    let native = previous
        .native_session_id
        .clone()
        .ok_or("继续原生会话缺失")?;
    let token = Uuid::new_v4();
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").ok_or("固定 CLI 缺失")?,
        ),
        cwd: PathBuf::from(&previous.working_directory),
        state_dir: current_state_dir(),
        target: SessionTarget::Resume {
            native_session_id: native.clone(),
        },
        generation: token,
        permission_policy: PermissionPolicy::GrokRestrictedReadV1,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: Some(
            serde_json::from_value(config["grok_profile"].clone())
                .map_err(|_| "保存创建策略缺失")?,
        ),
        model: None,
        local_tools: Some(LocalToolPermissions {
            allow_spawn: true,
            allow_message: true,
        }),
        selected_skills: Vec::new(),
    };
    coordinator.update(app, |model, ctx| {
        model.start(
            next_task_generation(&previous)?,
            options,
            Some(previous.generation),
            ctx,
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let wake = receiver
            .recv()
            .with_timeout(deadline.saturating_duration_since(Instant::now()))
            .await
            .map_err(|_| "继续 Ready 超时")?
            .ok_or("继续事件关闭")?;
        if let Some((task, event)) = wake {
            if task.task_id != parent_id
                || event.generation != token
                || event.native_session_id.as_deref() != Some(&native)
                || !matches!(event.kind, RuntimeEventKind::SessionReady { .. })
            {
                return Err("冷继续发生重投或身份变化".into());
            }
            break;
        }
        if coordinator.read(app, |model, _| {
            model
                .snapshot(parent_id)
                .is_some_and(|task| task.error.is_some())
        }) {
            return Err("继续连接失败".into());
        }
    }
    // Ready 后继续观察短暂静默期，避免只看握手就漏掉随后自动重投。
    let quiet = Instant::now() + Duration::from_secs(1);
    loop {
        match receiver
            .recv()
            .with_timeout(quiet.saturating_duration_since(Instant::now()))
            .await
        {
            Err(_) => break,
            Ok(Some(None)) => {}
            Ok(Some(Some(_))) => return Err("冷继续 Ready 后出现执行事件".into()),
            Ok(None) => return Err("冷继续 Ready 后事件流关闭".into()),
        }
    }
    if serde_json::to_value(messages(sender, parent_id).await?).map_err(|_| "消息摘要失败")?
        != before
    {
        return Err("冷继续重写旧消息".into());
    }
    evidence.record(json!({"event":"cold_resume_verified","same_native_session":true,"no_new_input":true,"messages_unchanged":true,"app_restart_verified":false}))
}

#[test]
#[ignore = "仅由官方隔离运行器执行；真实模型调用会产生费用"]
fn real_grok_fixed_policy_parent_child() {
    let root = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ROOT").expect("必须由运行器启动"))
        .canonicalize()
        .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-grok-child-coordinator-probe")).unwrap(),
        SCOPE
    );
    assert_eq!(
        env::var("INFINISHELL_GROK_LIVE_AUTH_MODE").unwrap(),
        "official-cached-token"
    );
    assert!(env::var_os("XAI_API_KEY").is_none() && env::var_os("ANTHROPIC_API_KEY").is_none());
    let home = PathBuf::from(env::var_os("HOME").unwrap())
        .canonicalize()
        .unwrap();
    assert!(current_state_dir().starts_with(&home));
    let artifact = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_ARTIFACT").unwrap())
        .canonicalize()
        .unwrap();
    assert!(artifact.starts_with(&root));
    let mut evidence = Evidence(File::create(artifact).unwrap());
    evidence.record(json!({"event":"acceptance_started","max_native_inputs":6,"production_prepare":true,"test_argv_override":false,
        "real_gui_verified":false,"app_restart_verified":false,"native_effective_policy_verified":false,"filesystem_sandbox_verified":false})).unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
        let executable = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").unwrap());
        app.add_singleton_model(|_| {
            CLIAgentInstallModel::with_installation_for_test(
                CLIAgent::Grok,
                CLIAgentInstallation {
                    executable: Some(executable),
                    version: CLIAgentVersionStatus::Detected("1.0.30".into()),
                },
            )
        });
        let writer =
            crate::persistence::start_test_writer(&root.join("coordinator.sqlite")).unwrap();
        let coordinator =
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(writer.sender.clone())));
        let (wake, mut receiver) = mpsc::unbounded_channel();
        app.update(|ctx| {
            ctx.subscribe_to_model(&coordinator, move |_, event, _| {
                let event = match event {
                    LocalCLITaskCoordinatorEvent::Runtime { task, event } => {
                        Some((task.clone(), event.clone()))
                    }
                    LocalCLITaskCoordinatorEvent::Changed
                    | LocalCLITaskCoordinatorEvent::MessagesChanged { .. }
                    | LocalCLITaskCoordinatorEvent::ResultReady { .. }
                    | LocalCLITaskCoordinatorEvent::ParentMessageReady { .. } => None,
                };
                let _ = wake.send(event);
            })
        });
        let mut cleaned = HashSet::new();
        let result = async {
            let parent = drive(
                &mut app,
                &coordinator,
                &writer.sender,
                &mut receiver,
                &root,
                &mut evidence,
            )
            .await?;
            shutdown(
                &mut app,
                &coordinator,
                &mut receiver,
                &mut cleaned,
                &mut evidence,
            )
            .await?;
            resume_ready(
                &mut app,
                &coordinator,
                &writer.sender,
                &mut receiver,
                &parent,
                &mut evidence,
            )
            .await
        }
        .await;
        let cleanup = shutdown(
            &mut app,
            &coordinator,
            &mut receiver,
            &mut cleaned,
            &mut evidence,
        )
        .await;
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        match result.and(cleanup) {
            Ok(()) if cleaned.len() == 3 => evidence.record(json!({"event":"acceptance_passed","native_inputs":6,"cleanup_count":3,
                "native_ack_both_directions":true,"automatic_result_ack":true,"final_result_via_inspect":true,"cold_resume_ready":true,
                "app_restart_verified":false,"real_gui_verified":false,"native_effective_policy_verified":false,"filesystem_sandbox_verified":false,
                "full_cli_parity_acceptance_passed":false})).unwrap(),
            failure => {
                let error = failure.err().unwrap_or_else(|| "清理进程计数不符".into());
                evidence.record(json!({"event":"acceptance_failed","reason_bytes":error.len(),"reason_sha256":sha(&error)})).unwrap();
                panic!("真实 Grok 父子协调器验收未通过，查看安全证据");
            }
        }
    });
}
