//! 显式隔离运行器驱动真实 Claude 与生产协调器；不把 App::test 计作 GUI 验收。

use std::env;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;

use serde_json::Value;
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};

use super::super::local_tools::{LocalToolPermissions, NativeLocalToolRequest};
use super::super::managed_process::confirmed_exit;
use super::super::permissions::ceiling_from_parent;
use super::super::{ApprovalDecision, current_state_dir};
use super::*;
use crate::persistence::local_cli_tasks::load_task_messages;
use crate::persistence::model::LocalCliReceiptKind;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent::{
    CLIAgentInstallModel, CLIAgentInstallation, CLIAgentVersionStatus,
};

const SCOPE: &str = "real_claude_production_coordinator";
const MAX_NATIVE_TOOLS: usize = 4;
const MAX_NATIVE_INPUTS: usize = 6;
const BEFORE: &str = "CHILD_BEFORE";

struct Evidence {
    // 完整配置仅写隔离目录；运行器审核全树后生成带哈希的公开摘要。
    file: File,
    root: PathBuf,
}

impl Evidence {
    fn record(&mut self, value: Value) -> Result<(), String> {
        let encoded = serde_json::to_string(&value).map_err(|error| error.to_string())?;
        let encoded = encoded.replace(&*self.root.to_string_lossy(), "<probe-root>");
        writeln!(self.file, "{encoded}").map_err(|error| error.to_string())?;
        self.file.flush().map_err(|error| error.to_string())
    }
}

struct Markers {
    initial: String,
    final_result: String,
    collected: String,
    parent_progress: String,
    automatic_result: String,
    followup: String,
    progress: String,
}

impl Markers {
    fn new() -> Self {
        let id = Uuid::new_v4();
        Self {
            initial: format!("CHILD_INITIAL_{id}"),
            final_result: format!("CHILD_FINAL_{id}"),
            collected: format!("PARENT_COLLECTED_{id}"),
            parent_progress: format!("PARENT_PROGRESS_{id}"),
            automatic_result: format!("PARENT_AUTOMATIC_{id}"),
            followup: format!("followup-{id}"),
            progress: format!("progress-{id}"),
        }
    }

    fn prompt(&self, parent: &str, file: &Path, model: &str) -> String {
        let child_prompt = format!(
            "This is an authorized isolated integration test. Your parent task ID is {parent}. \
             In this first turn use Read to read {}, then Edit exactly once with old_string \
             CHILD_BEFORE and \
             new_string exactly {}, replace_all=false. Do not use Write or any other file. Wait normally for host approval. \
             After the Edit result, finish with {} unless a follow-up is folded into this execution; \
             in that case complete its parent message and include its final marker. \
             Do not spawn any agents.",
            file.display(),
            self.initial,
            self.initial,
        );
        let spawn = json!({"summary":"isolated parent-child verification", "base_prompt":"",
            "harness":"claude", "model_id":model, "skills":[],
            "agent_run_configs":[{"name":"verified-child","prompt":child_prompt}]});
        let followup = format!(
            "This is the queued follow-up, which the native runtime may fold into your running execution. \
             Complete the previously authorized file operation before proceeding. \
             Call send_message_to_agent exactly once to parent \
             {parent}, subject {}, message: 'Receipt acknowledgement task: finish your queued \
             input with {}; if this input joins your running execution, retain all prior instructions \
             including inspect and include both its collection marker and this progress marker. \
             Do not send another message.' \
             After the send tool reports native acknowledgement, finish with {}. \
             Do not perform additional edits or spawn agents.",
            self.progress, self.parent_progress, self.final_result,
        );
        format!(
            "This is an authorized isolated production-coordinator test. Call run_agents once \
             with these exact arguments: {spawn}. Retain the returned child task_id. \
             Next call send_message_to_agent once to that child with subject {} and message {}. \
             The host will approve this send only when the child is awaiting its file approval. \
             Require the send result to report acknowledged with native_protocol; never resend. \
             Then call inspect_local_tasks exactly once for only that child. The host holds \
             that tool approval until the child's follow-up input has truly completed. Require its \
             actual saved result to contain {} and never create a second child. \
             Do not finish before you retrieve that actual final result. Then finish with only {}. \
             Do not edit files or call any other tools. If queued child progress joins this execution, \
             retain the inspect requirement and include both the collection and progress markers; \
             otherwise process progress in a later separate execution. \
             You will also receive automatically delivered messages with Subject: local_task_result \
             and a JSON body. For each such input verify the JSON belongs to this child and \
             finish with {} without calling additional tools. If it joins a running execution, \
             preserve its existing inspect requirement and include this automatic receipt marker \
             alongside any required collection or progress markers.",
            self.followup,
            json!(followup),
            self.final_result,
            self.collected,
            self.automatic_result,
        )
    }
}

type Wake = Option<(LocalCliTask, RuntimeEvent)>;

async fn records(sender: &SyncSender<ModelEvent>) -> Result<Vec<LocalCliTask>, String> {
    load_tasks(sender, false)?
        .await
        .map_err(|_| "任务读取确认已关闭")?
}

async fn messages(
    sender: &SyncSender<ModelEvent>,
    task: &str,
) -> Result<Vec<LocalCliMessage>, String> {
    load_task_messages(sender, task.to_owned())?
        .await
        .map_err(|_| "消息读取确认已关闭")?
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
        .await
        .map_err(|_| "审批提交确认已关闭")?
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
    let parent_history = load_task_generations(sender, parent_id.to_owned())?
        .await
        .map_err(|_| "父历史确认已关闭")??;
    let child_history = load_task_generations(sender, child_id.to_owned())?
        .await
        .map_err(|_| "子历史确认已关闭")??;
    let source = parent_history
        .iter()
        .find(|task| task.generation == 1)
        .ok_or("缺少创建子任务时的父代")?;
    let parent_config: Value =
        serde_json::from_str(&source.config_json).map_err(|error| error.to_string())?;
    let child_config: Value =
        serde_json::from_str(&child.config_json).map_err(|error| error.to_string())?;
    if child.parent_task_id.as_deref() != Some(parent_id)
        || child.parent_generation != Some(1)
        || child_config["claude_profile"] != parent_config["claude_profile"]
        || child_config["effective_permissions"]["claudeRestrictedFilesV1"]
            != parent_config["claude_profile"]
        || child_config["effective_permissions"]["fixedProfileVerified"] != true
    {
        return Err("实际子任务的亲缘或固定策略不匹配".into());
    }
    let ceiling = ceiling_from_parent(source, "claude").map_err(|error| error.to_string())?;
    if child_config["permission_ceiling"] != json!(ceiling) {
        return Err("实际子任务未保存创建父代的完整权限上限".into());
    }
    let rows = messages(sender, parent_id).await?;
    let child_rows = messages(sender, child_id).await?;
    let followups = rows
        .iter()
        .filter(|message| message.subject == markers.followup)
        .collect::<Vec<_>>();
    let progress = rows
        .iter()
        .filter(|message| message.subject == markers.progress)
        .collect::<Vec<_>>();
    let final_result = child_history.iter().any(|task| {
        matches!(task.generation, 1 | 2)
            && task.state == LocalCliTaskState::Completed
            && task
                .result
                .as_ref()
                .is_some_and(|result| result.contains(&markers.final_result))
            && task.terminal_evidence.is_some()
    });
    let parent_collected = parent_history.iter().any(|task| {
        task.generation == 1
            && task.state == LocalCliTaskState::Completed
            && task
                .result
                .as_ref()
                .is_some_and(|result| result.contains(&markers.collected))
    });
    let parent_progress = parent_history.iter().any(|task| {
        task.state == LocalCliTaskState::Completed
            && task
                .result
                .as_ref()
                .is_some_and(|result| result.contains(&markers.parent_progress))
    });
    let automatic_result = parent.state == LocalCliTaskState::Completed
        && parent
            .result
            .as_ref()
            .is_some_and(|result| result.contains(&markers.automatic_result));
    let inspected = rows
        .iter()
        .filter(|row| row.subject == "native_tool_result" && row.sender_task_id == parent_id)
        .filter_map(|row| serde_json::from_str::<Result<Value, String>>(&row.body).ok())
        .filter_map(Result::ok)
        .any(|result| {
            result["tasks"].as_array().is_some_and(|tasks| {
                tasks.iter().any(|task| {
                    task["task_id"] == child_id
                        && task["generation"] == child.generation
                        && task["state"] == "completed"
                        && task["result"]
                            .as_str()
                            .is_some_and(|value| value.contains(&markers.final_result))
                })
            })
        });
    let requests = rows
        .iter()
        .chain(&child_rows)
        .filter(|row| row.subject == "native_tool_call")
        .map(|row| (row.message_id.clone(), row))
        .collect::<HashMap<_, _>>();
    if requests.len() > MAX_NATIVE_TOOLS {
        return Err("持久化工具记录出现额外调用".into());
    }
    let mut operations = HashSet::new();
    for row in requests.values() {
        let request: NativeLocalToolRequest =
            serde_json::from_str(&row.body).map_err(|error| error.to_string())?;
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
                        && row.sender_generation == child.generation
                        && request.arguments["subject"] == markers.progress
                        && request.arguments["addresses"] == json!([parent_id]))
            }
            _ => false,
        };
        if !correct || !operations.insert((row.sender_task_id.clone(), request.tool.clone())) {
            return Err("持久化原生工具记录与唯一父子流程不符".into());
        }
    }
    let spawned = rows
        .iter()
        .filter(|row| row.subject == "native_tool_result" && row.sender_generation == 1)
        .filter_map(|row| serde_json::from_str::<Result<Value, String>>(&row.body).ok())
        .filter_map(Result::ok)
        .any(|result| {
            result["status"] == "queued"
                && result["children"].as_array().is_some_and(|children| {
                    children.len() == 1 && children[0]["task_id"] == child_id
                })
        });
    let result_messages = rows
        .iter()
        .filter(|message| {
            message.subject == "local_task_result" && message.sender_task_id == child_id
        })
        .collect::<Vec<_>>();
    let result_message = result_messages.len() == child_history.len()
        && child_history.iter().all(|task| {
            result_messages
                .iter()
                .filter(|message| {
                    message.sender_generation == task.generation
                        && message.recipient_task_id == parent_id
                        && message.recipient_generation == 1
                        && native_ack(message)
                })
                .count()
                == 1
        });
    if !(final_result
        && parent_collected
        && parent_progress
        && automatic_result
        && inspected
        && result_message
        && spawned
        && requests.len() == MAX_NATIVE_TOOLS
        && followups.len() == 1
        && progress.len() == 1
        && native_ack(followups[0])
        && native_ack(progress[0]))
    {
        return Ok(false);
    }
    if parent.native_session_id.is_none()
        || child.native_session_id.is_none()
        || parent.native_session_id == child.native_session_id
        || saved.len() != 2
    {
        return Err("父子原生会话或唯一派发证明不成立".into());
    }
    let all_messages = rows
        .iter()
        .chain(&child_rows)
        .map(|row| (row.message_id.clone(), row.clone()))
        .collect::<HashMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();
    evidence.record(json!({"event":"saved_chain_verified","parent":parent,"child":child,
        "parent_generations":parent_history,"child_generations":child_history,"messages":all_messages,
        "markers":{"initial":markers.initial,"final_result":markers.final_result,
            "collected":markers.collected,"parent_progress":markers.parent_progress,
            "automatic_result":markers.automatic_result,
            "followup":markers.followup,"progress":markers.progress},
        "fixed_profile_equal":true,"native_message_ack_both_directions":true,
        "final_result_retrieved_by_native_inspect":true,"automatic_result_delivery_ack_verified":true}))?;
    Ok(true)
}

async fn drive(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    sender: &SyncSender<ModelEvent>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    root: &Path,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let parent_id = Uuid::new_v4().to_string();
    let executable =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").ok_or("缺少固定CLI")?);
    let model =
        env::var("INFINISHELL_CLAUDE_LIVE_MODEL").map_err(|_| "父子验收必须显式固定模型")?;
    let cwd = fs::canonicalize(root.join("project")).map_err(|error| error.to_string())?;
    let output_file = cwd.join("child-approved.txt");
    let markers = Markers::new();
    let options = SessionOptions {
        executable,
        cwd: cwd.clone(),
        state_dir: current_state_dir(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::ClaudeRestrictedFilesV1,
        permission_ceiling: None,
        claude_profile: None,
        model: Some(model.clone()),
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
        harness: "claude".into(),
        working_directory: cwd.to_string_lossy().into(),
        config_json: json!({"model":model,"permission_policy":"ClaudeRestrictedFilesV1"})
            .to_string(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    let prompt = markers.prompt(&parent_id, &output_file, &model);
    coordinator.update(app, |model, ctx| {
        model.start_with_input(
            task,
            options,
            None,
            Uuid::new_v4(),
            vec![InputContent::Text(prompt)],
            ctx,
        )
    })?;
    let deadline = Instant::now() + Duration::from_secs(450);
    let mut approved = HashSet::new();
    let mut tool_calls = HashSet::new();
    let mut turns = HashSet::new();
    let mut joined_inputs = HashSet::new();
    let mut finished_turns = HashSet::new();
    let mut child_id = None;
    let mut gate_recorded = false;
    loop {
        if Instant::now() >= deadline {
            return Err("父子整链超过450秒，不能算通过".into());
        }
        let wake = receiver
            .recv()
            .with_timeout(deadline.saturating_duration_since(Instant::now()))
            .await
            .map_err(|_| "父子整链超过450秒，不能算通过")?
            .ok_or("协调器事件已关闭")?;
        if let Some((task, event)) = wake {
            evidence.record(json!({"event":"runtime","task":task,"runtime":event}))?;
            match &event.kind {
                RuntimeEventKind::LocalToolRequested { request } => {
                    tool_calls.insert((task.task_id.clone(), request.call_id.clone()));
                }
                RuntimeEventKind::TurnStarted { turn_id } => {
                    turns.insert((task.task_id.clone(), turn_id.clone()));
                }
                RuntimeEventKind::InputJoined { message_id, .. } => {
                    joined_inputs.insert((task.task_id.clone(), message_id.to_string()));
                }
                RuntimeEventKind::TurnFinished {
                    turn_id, outcome, ..
                } => match outcome {
                    TurnOutcome::Completed => {
                        finished_turns.insert((task.task_id.clone(), turn_id.clone()));
                    }
                    TurnOutcome::Failed { .. } | TurnOutcome::Cancelled => {
                        return Err("真实父子回合未成功完成".into());
                    }
                },
                RuntimeEventKind::Disconnected { .. } => {
                    return Err("验收中真实连接提前关闭".into());
                }
                _ => {}
            }
        }
        if tool_calls.len() > MAX_NATIVE_TOOLS
            || turns.len() + joined_inputs.len() > MAX_NATIVE_INPUTS
        {
            return Err("原生工具或回合数超过验收预算".into());
        }
        let snapshots = coordinator.read(app, |model, _| {
            model.snapshots().cloned().collect::<Vec<_>>()
        });
        if let Some(error) = snapshots.iter().find_map(|snapshot| snapshot.error.clone()) {
            return Err(error);
        }
        let children = snapshots
            .iter()
            .filter(|snapshot| snapshot.task.parent_task_id.as_deref() == Some(&parent_id))
            .collect::<Vec<_>>();
        if children.len() > 1 {
            return Err("产生了多余子任务".into());
        }
        if let Some(child) = children.first() {
            child_id = Some(child.task.task_id.clone());
        }
        let child_edit_waiting = children.iter().any(|child| {
            child
                .approvals
                .iter()
                .any(|approval| approval.details["tool_name"] == "Edit")
        });
        let mut followup_ack = false;
        if let Some(child_id) = &child_id {
            let rows = messages(sender, child_id).await?;
            followup_ack = rows.iter().any(|row| {
                row.subject == markers.followup
                    && row.sender_task_id == parent_id
                    && row.recipient_task_id == *child_id
                    && native_ack(row)
            });
            if followup_ack && child_edit_waiting && !gate_recorded {
                evidence.record(json!({"event":"native_ack_before_child_edit_allow","child_id":child_id,
                    "child_generation":1,"child_edit_waiting":true,
                    "matching_messages":rows.iter().filter(|row| row.subject == markers.followup).collect::<Vec<_>>() }))?;
                gate_recorded = true;
            }
        }
        for snapshot in &snapshots {
            for approval in &snapshot.approvals {
                let identity = (snapshot.task.task_id.clone(), approval.approval_id.clone());
                if approved.contains(&identity) {
                    continue;
                }
                if approved.len() >= MAX_NATIVE_TOOLS + 1 {
                    return Err("审批次数超过验收预算".into());
                }
                let tool = approval.details["tool_name"]
                    .as_str()
                    .ok_or("审批缺少工具名")?;
                let input = &approval.details["input"];
                let is_parent = snapshot.task.task_id == parent_id;
                match tool {
                    "Edit"
                        if !is_parent
                            && input["file_path"].as_str().map(Path::new)
                                == Some(output_file.as_path())
                            && input["old_string"] == BEFORE
                            && input["new_string"] == markers.initial
                            && input.as_object().is_some_and(|input| {
                                input.keys().all(|key| {
                                    matches!(
                                        key.as_str(),
                                        "file_path" | "old_string" | "new_string" | "replace_all"
                                    )
                                }) && input
                                    .get("replace_all")
                                    .is_none_or(|value| value.as_bool() == Some(false))
                            }) =>
                    {
                        if !followup_ack {
                            continue;
                        }
                    }
                    "mcp__infinishell-local-tasks__run_agents"
                        if is_parent
                            && child_id.is_none()
                            && input["harness"] == "claude"
                            && input["model_id"] == model
                            && input["agent_run_configs"]
                                .as_array()
                                .is_some_and(|children| children.len() == 1) => {}
                    "mcp__infinishell-local-tasks__send_message_to_agent"
                        if is_parent
                            && input["subject"] == markers.followup
                            && child_id
                                .as_ref()
                                .is_some_and(|child| input["addresses"] == json!([child])) =>
                    {
                        if !child_edit_waiting {
                            continue;
                        }
                    }
                    "mcp__infinishell-local-tasks__send_message_to_agent"
                        if !is_parent
                            && input["subject"] == markers.progress
                            && input["addresses"] == json!([parent_id]) => {}
                    "mcp__infinishell-local-tasks__inspect_local_tasks"
                        if is_parent
                            && child_id
                                .as_ref()
                                .is_some_and(|child| input["task_ids"] == json!([child])) =>
                    {
                        if !children.iter().any(|child| {
                            matches!(child.task.generation, 1 | 2)
                                && child.task.state == LocalCliTaskState::Completed
                                && child
                                    .task
                                    .result
                                    .as_ref()
                                    .is_some_and(|result| result.contains(&markers.final_result))
                        }) {
                            continue;
                        }
                    }
                    _ => return Err("模型请求了验收范围外的工具或参数".into()),
                }
                approved.insert(identity);
                evidence.record(json!({"event":"approval_allowed","task_id":snapshot.task.task_id,
                    "generation":snapshot.task.generation,"approval":{"approval_id":approval.approval_id,
                        "turn_id":approval.turn_id,"method":approval.method,"details":approval.details},
                    "decision":"AllowOnce"}))?;
                approve(app, coordinator, &snapshot.task, approval).await?;
            }
        }
        if gate_recorded
            && tool_calls.len() == MAX_NATIVE_TOOLS
            && (5..=MAX_NATIVE_INPUTS).contains(&(turns.len() + joined_inputs.len()))
            && finished_turns.len() == turns.len() + joined_inputs.len()
            && approved.len() == MAX_NATIVE_TOOLS + 1
            && let Some(child_id) = &child_id
            && snapshots
                .iter()
                .all(|snapshot| snapshot.task.state == LocalCliTaskState::Completed)
            && verify_saved_chain(sender, &parent_id, child_id, &markers, evidence).await?
        {
            if fs::read_to_string(&output_file).map_err(|error| error.to_string())?
                != markers.initial
            {
                return Err("已批准子任务文件效果不符".into());
            }
            evidence.record(
                json!({"event":"coordinator_chain_finished","native_tool_calls":tool_calls.len(),
                    "native_inputs":turns.len() + joined_inputs.len(),
                    "native_executions":turns.len(),"joined_inputs":joined_inputs.len(),
                    "approvals_allowed":approved.len(),
                    "child_edit_effect_verified":true,"expected_child_file_content":markers.initial}),
            )?;
            return Ok(());
        }
    }
}

async fn shutdown(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
    receiver: &mut mpsc::UnboundedReceiver<Wake>,
    evidence: &mut Evidence,
) -> Result<(), String> {
    let ids = coordinator.read(app, |model, _| {
        model
            .snapshots()
            .filter(|snapshot| snapshot.connected)
            .map(|snapshot| snapshot.task.task_id.clone())
            .collect::<Vec<_>>()
    });
    for id in ids {
        let reply = coordinator.update(app, |model, _| {
            model.request(&id, Uuid::new_v4(), RuntimeAction::Shutdown)
        });
        if let Ok(reply) = reply {
            let _ = reply.with_timeout(Duration::from_secs(5)).await;
        }
    }
    let deadline = Instant::now() + Duration::from_secs(35);
    while coordinator.read(app, |model, _| {
        model.snapshots().any(|snapshot| snapshot.connected)
    }) {
        receiver
            .recv()
            .with_timeout(deadline.saturating_duration_since(Instant::now()))
            .await
            .map_err(|_| "协调器清理未确认")?
            .ok_or("清理事件关闭")?;
    }
    let snapshots = coordinator.read(app, |model, _| {
        model.snapshots().cloned().collect::<Vec<_>>()
    });
    for snapshot in snapshots {
        let config: Value =
            serde_json::from_str(&snapshot.task.config_json).map_err(|error| error.to_string())?;
        let generation: Uuid = serde_json::from_value(config["runtime_generation"].clone())
            .map_err(|error| error.to_string())?;
        // 协调器断开可早于监督入口写出退出回执；等待真实回执，不从断开推断清理。
        let receipt = loop {
            if let Some(receipt) = confirmed_exit(&current_state_dir(), generation)
                .map_err(|error| error.to_string())?
            {
                break receipt;
            }
            if Instant::now() >= deadline {
                return Err("缺少父子原生进程退出回执".into());
            }
            Timer::after(Duration::from_millis(50)).await;
        };
        evidence.record(
            json!({"event":"cleanup_confirmed","task_id":snapshot.task.task_id,"receipt":receipt}),
        )?;
    }
    Ok(())
}

#[test]
#[ignore = "仅由显式私有API运行器执行，调用真实模型并产生费用"]
fn real_claude_fixed_profile_parent_child() {
    let root =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").expect("必须由隔离运行器启动"))
            .canonicalize()
            .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-claude-coordinator-probe")).unwrap(),
        SCOPE
    );
    let auth_home = PathBuf::from(env::var_os("HOME").expect("缺少私有HOME"))
        .canonicalize()
        .unwrap();
    let state = current_state_dir();
    assert!(
        state.starts_with(&auth_home),
        "监督记录必须位于运行器提供的私有HOME"
    );
    let artifact =
        PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT").expect("缺少私有证据路径"))
            .canonicalize()
            .unwrap();
    assert!(
        artifact.starts_with(&root),
        "完整原始证据必须留在私有工作目录"
    );
    let mut evidence = Evidence {
        file: File::create(artifact).unwrap(),
        root: root.clone(),
    };
    evidence
        .record(
            json!({"event":"acceptance_started","scope":SCOPE,"real_gui_verified":false,
        "max_native_tools":MAX_NATIVE_TOOLS,"max_native_inputs":MAX_NATIVE_INPUTS,
        "max_native_executions":MAX_NATIVE_INPUTS,"max_seconds":450,
        "http_request_count_verified":false,"automatic_result_delivery_ack_verified":false}),
        )
        .unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
        let executable = PathBuf::from(env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").unwrap());
        app.add_singleton_model(|_| {
            CLIAgentInstallModel::with_installation_for_test(
                CLIAgent::Claude,
                CLIAgentInstallation {
                    executable: Some(executable),
                    version: CLIAgentVersionStatus::Detected("2.1.273".into()),
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
                    _ => None,
                };
                let _ = wake.send(event);
            })
        });
        let result = drive(
            &mut app,
            &coordinator,
            &writer.sender,
            &mut receiver,
            &root,
            &mut evidence,
        )
        .await;
        let cleanup = shutdown(&mut app, &coordinator, &mut receiver, &mut evidence).await;
        if let Err(error) = &cleanup {
            evidence
                .record(json!({"event":"cleanup_failed","reason":error}))
                .unwrap();
        }
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        match result.and(cleanup) {
            Ok(()) => evidence
                .record(json!({"event":"acceptance_passed","scope":SCOPE,
                "production_spawn_verified":true,"saved_profile_equal":true,
                "native_message_ack_both_directions":true,"final_result_via_inspect":true,
                "real_gui_verified":false,"automatic_result_delivery_ack_verified":true}))
                .unwrap(),
            Err(error) => {
                evidence
                    .record(json!({"event":"acceptance_failed","reason":error}))
                    .unwrap();
                panic!("真实Claude生产父子协调器验收未通过；检查脱敏证据");
            }
        }
    });
}
