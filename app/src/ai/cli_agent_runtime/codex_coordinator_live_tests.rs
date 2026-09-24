//! 固定 Codex 的父子派发、托管邮箱双向原生确认与显式继续取回结果验收，不等同于 GUI 验收。

use std::env;
use std::fs::{self, File};
use std::io::Write as _;

use serde_json::Value;
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};

use super::super::local_tools::LocalToolPermissions;
use super::super::permissions::{ceiling_from_parent, verify_effective_permissions};
use super::super::runtime_host::confirmed_exit;
use super::super::{ApprovalDecision, current_state_dir};
use super::*;
use crate::terminal::cli_agent::{
    CLIAgentInstallModel, CLIAgentInstallation, CLIAgentVersionStatus,
};

const SCOPE: &str = "codex_01561_production_parent_child_duplex_inspect";
const MAX_INPUTS: usize = 6;
const MAX_SECONDS: u64 = 180;
type Wake = Option<(LocalCliTask, RuntimeEvent)>;

struct Evidence(File);

impl Evidence {
    fn record(&mut self, value: Value) -> Result<(), String> {
        // 只写显式构造的计数和布尔值，不复制任务、原生身份或模型正文。
        writeln!(self.0, "{value}").map_err(|error| error.to_string())?;
        self.0.flush().map_err(|error| error.to_string())
    }
}

struct Exchange {
    to_child: String,
    to_parent: String,
    child_input: String,
    parent_input: String,
}

impl Exchange {
    fn new(parent_id: &str, child_marker: &str) -> Self {
        let id = Uuid::new_v4();
        let to_parent = format!("child-progress-{id}");
        let parent_input = "This is the child's progress update. Keep waiting for your already requested approval. After it completes, finish with DISPATCHED. Do not call tools or send messages.".to_owned();
        let arguments = json!({"addresses":[parent_id],"subject":to_parent,"message":parent_input});
        Self {
            to_child: format!("parent-followup-{id}"),
            to_parent,
            child_input: format!(
                "This is the authorized parent follow-up. After your already requested command approval completes, call send_message_to_agent exactly once with these exact arguments: {arguments}. Never resend, including on an unconfirmed result. Then finish with {child_marker}. Do not call other tools, edit files, or create agents."
            ),
            parent_input,
        }
    }
}

fn native_message_ack(
    messages: &[LocalCliMessage],
    source: &str,
    recipient: &str,
    subject: &str,
    body: &str,
) -> bool {
    let matching = messages
        .iter()
        .filter(|message| message.subject == subject)
        .collect::<Vec<_>>();
    matching.len() == 1
        && matching[0].sender_task_id == source
        && matching[0].recipient_task_id == recipient
        && matching[0].sender_generation == 1
        && matching[0].recipient_generation == 1
        && matching[0].body == body
        && matching[0].state == LocalCliMessageState::Acknowledged
        && matching[0].receipt_kind == Some(LocalCliReceiptKind::NativeProtocol)
}

fn gate_approval_matches(details: &Value, command: &str, argv: &Value, cwd: &Path) -> bool {
    // 仅允许本夹具只打印标记的固定命令；完整 shell 包装必须逐项相同。
    let shell = details["command"]
        .as_str()
        .and_then(|actual| shell_words::split(actual).ok())
        .filter(|parts| {
            parts.len() == 3
                && matches!(parts[0].as_str(), "/bin/zsh" | "/bin/bash" | "/bin/sh")
                && matches!(parts[1].as_str(), "-c" | "-lc")
                && parts[2] == command
        });
    (details["command"] == command || shell.is_some())
        && (details["proposedExecpolicyAmendment"] == *argv
            || shell.is_some_and(|parts| details["proposedExecpolicyAmendment"] == json!(parts)))
        && details["commandActions"]
            .as_array()
            .is_some_and(|actions| actions.len() == 1 && actions[0]["command"] == command)
        && details["cwd"]
            .as_str()
            .is_some_and(|path| Path::new(path).canonicalize().ok().as_deref() == Some(cwd))
}

async fn verified_chain(
    sender: &SyncSender<ModelEvent>,
    parent_id: &str,
    child_marker: &str,
    parent_marker: &str,
    continuation: Uuid,
    parent_generation: i64,
    exchange: &Exchange,
) -> Result<Option<bool>, String> {
    let saved = load_tasks(sender, false)?
        .await
        .map_err(|_| "任务读取确认已关闭")??;
    if saved.len() > 2 {
        return Err("生成了多余任务".into());
    }
    let Some(parent) = saved.iter().find(|task| task.task_id == parent_id) else {
        return Ok(None);
    };
    let Some(child) = saved
        .iter()
        .find(|task| task.parent_task_id.as_deref() == Some(parent_id))
    else {
        return Ok(None);
    };
    if parent.state != LocalCliTaskState::Completed
        || child.state != LocalCliTaskState::Completed
        || !parent
            .result
            .as_ref()
            .is_some_and(|text| text.contains(parent_marker) && text.contains(child_marker))
        || !child
            .result
            .as_ref()
            .is_some_and(|text| text.contains(child_marker))
    {
        return Ok(None);
    }
    let generations = load_task_generations(sender, parent_id.to_owned())?
        .await
        .map_err(|_| "父代读取确认已关闭")??;
    let source = generations
        .iter()
        .find(|task| task.generation == 1)
        .ok_or("缺少创建子任务时的父代")?;
    let parent_config: Value =
        serde_json::from_str(&source.config_json).map_err(|_| "父配置无效")?;
    let child_config: Value = serde_json::from_str(&child.config_json).map_err(|_| "子配置无效")?;
    if child.parent_generation != Some(1)
        || child.generation != 1
        || parent.generation != parent_generation
        || source.harness != "codex"
        || child.harness != "codex"
        || parent_config["cli_version"] != "0.156.1"
        || child_config["cli_version"] != "0.156.1"
        || parent_config["permission_policy"] != "WorkspaceWrite"
        || child_config["permission_policy"] != "WorkspaceWrite"
        || child.working_directory != source.working_directory
        || source.native_session_id.is_none()
        || child.native_session_id.is_none()
        || parent.native_session_id != source.native_session_id
        || source.native_session_id == child.native_session_id
        || source.terminal_evidence.is_none()
        || child.terminal_evidence.is_none()
    {
        return Err("父子版本、原生身份或创建代次不匹配".into());
    }
    let ceiling = ceiling_from_parent(source, "codex").map_err(|_| "父权限不可验证")?;
    if child_config["permission_ceiling"] != json!(ceiling) {
        return Err("子任务未保存创建父代的完整权限上限".into());
    }
    verify_effective_permissions(
        Some(&ceiling),
        "codex",
        Path::new(&child.working_directory),
        &child_config["effective_permissions"],
    )
    .map_err(|_| "子任务实际权限扩大或无法验证")?;
    let messages = load_task_messages(sender, parent_id.to_owned())?
        .await
        .map_err(|_| "结果消息读取确认已关闭")??;
    let results = messages
        .iter()
        .filter(|message| message.subject == "local_task_result")
        .collect::<Vec<_>>();
    let matching_result = results.len() == 1
        && results[0].sender_task_id == child.task_id
        && results[0].sender_generation == 1
        && results[0].recipient_task_id == parent_id
        && results[0].recipient_generation == 1;
    let automatic_ack = matching_result
        && results[0].state == LocalCliMessageState::Acknowledged
        && results[0].receipt_kind == Some(LocalCliReceiptKind::NativeProtocol);
    let spawn_result = messages
        .iter()
        .filter(|message| message.subject == "native_tool_result" && message.sender_generation == 1)
        .filter_map(|message| serde_json::from_str::<Result<Value, String>>(&message.body).ok())
        .filter_map(Result::ok)
        .any(|result| {
            result["status"] == "queued"
                && result["children"].as_array().is_some_and(|children| {
                    children.len() == 1 && children[0]["task_id"] == child.task_id
                })
        });
    let inspect_result = messages
        .iter()
        .filter(|message| {
            message.subject == "native_tool_result"
                && message.sender_generation == parent_generation
        })
        .filter_map(|message| serde_json::from_str::<Result<Value, String>>(&message.body).ok())
        .filter_map(Result::ok)
        .any(|result| {
            result["tasks"].as_array().is_some_and(|tasks| {
                tasks.len() == 1
                    && tasks[0]["task_id"] == child.task_id
                    && tasks[0]["generation"] == 1
                    && tasks[0]["state"] == "completed"
                    && tasks[0]["result"] == child.result.as_deref().unwrap_or_default()
                    && tasks[0]["result_truncated"] == false
            })
        });
    let continuation_ack = messages.iter().any(|message| {
        message.message_id == continuation.to_string()
            && message.subject == "user_input"
            && message.recipient_generation == parent_generation
            && message.state == LocalCliMessageState::Acknowledged
            && message.receipt_kind == Some(LocalCliReceiptKind::NativeProtocol)
    });
    let duplex_ack = native_message_ack(
        &messages,
        parent_id,
        &child.task_id,
        &exchange.to_child,
        &exchange.child_input,
    ) && native_message_ack(
        &messages,
        &child.task_id,
        parent_id,
        &exchange.to_parent,
        &exchange.parent_input,
    );
    let child_messages = load_task_messages(sender, child.task_id.clone())?
        .await
        .map_err(|_| "结果消息读取确认已关闭")??;
    let child_send_result = child_messages.iter()
        .filter(|message| message.subject == "native_tool_result" && message.sender_task_id == child.task_id && message.sender_generation == 1)
        .filter_map(|message| serde_json::from_str::<Result<Value, String>>(&message.body).ok())
        .filter_map(Result::ok)
        .any(|result| {
            let progress = messages.iter().find(|message| message.subject == exchange.to_parent);
            progress.is_some_and(|progress| result["status"] == "acknowledged"
                && result["message_ids"] == json!([progress.message_id])
                && result["receipts"] == json!([{"message_id":progress.message_id,"receipt_kind":"native_protocol"}]))
        });
    Ok((matching_result
        && spawn_result
        && inspect_result
        && continuation_ack
        && duplex_ack
        && child_send_result)
        .then_some(automatic_ack))
}

fn event_stage(event: &RuntimeEventKind) -> &'static str {
    match event {
        RuntimeEventKind::SessionReady { .. } => "session_ready",
        RuntimeEventKind::MessageAccepted { .. } => "input_accepted",
        RuntimeEventKind::CommandDispatched { .. } => "command_dispatched",
        RuntimeEventKind::TurnStarted { .. } => "turn_started",
        RuntimeEventKind::InputJoined { .. } => "input_joined",
        RuntimeEventKind::TextDelta { .. } => "text_delta",
        RuntimeEventKind::Progress { .. } => "progress",
        RuntimeEventKind::ApprovalRequested { .. } => "approval_requested",
        RuntimeEventKind::ApprovalResolved { .. } => "approval_resolved",
        RuntimeEventKind::ApprovalCancelled { .. } => "approval_cancelled",
        RuntimeEventKind::LocalToolCancelled { .. } => "local_tool_cancelled",
        RuntimeEventKind::LocalToolRequested { .. } => "local_tool_requested",
        RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        } => "turn_completed",
        RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Failed { .. },
            ..
        } => "turn_failed",
        RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Cancelled,
            ..
        } => "turn_cancelled",
        RuntimeEventKind::RequestFailed { .. } => "request_failed",
        RuntimeEventKind::Disconnected { .. } => "disconnected",
    }
}

fn failure_code(error: &str) -> &'static str {
    // 只映射固定测试阶段，不输出外部错误中的路径、身份或模型正文。
    match error {
        "父子链超过时间预算" => "chain_timeout",
        "协调器事件已关闭" => "coordinator_events_closed",
        "生成了多余任务" => "unexpected_task_count",
        "请求了范围外的本地工具或参数" => "unexpected_native_tool",
        "此无文件操作验收不允许任何原生审批" => "unexpected_native_approval",
        "真实连接提前关闭或回合失败" => "native_execution_failed",
        "原生工具或输入超过预算" => "native_budget_exceeded",
        "生产协调器报告错误" => "coordinator_reported_error",
        "子任务完成但结果标记不符" => "child_result_mismatch",
        "显式继续确认超时" => "continuation_timeout",
        "显式继续确认已关闭" => "continuation_closed",
        "缺少创建子任务时的父代" => "creation_generation_missing",
        "父子版本、原生身份或创建代次不匹配" => "parent_child_identity_mismatch",
        "父权限不可验证" => "parent_permissions_unverified",
        "子任务未保存创建父代的完整权限上限" => "saved_child_permissions_mismatch",
        "子任务实际权限扩大或无法验证" => "effective_child_permissions_mismatch",
        "任务读取确认已关闭" | "父代读取确认已关闭" | "结果消息读取确认已关闭" => {
            "persistence_read_closed"
        }
        "父配置无效" | "子配置无效" => "saved_config_invalid",
        "审批停点超出固定命令范围" => "unsafe_readonly_gate",
        "审批停点准备失败" | "审批停点确认超时" | "审批停点确认已关闭" => {
            "readonly_gate_failed"
        }
        "子任务没有活动消息入口" | "父子邮箱派发超时" => {
            "mailbox_dispatch_failed"
        }
        "项目目录无效" | "缺少固定 CLI" | "缺少固定 Python" => "fixture_invalid",
        _ => "coordinator_operation_failed",
    }
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
    let child_marker = format!("CHILD_RESULT_{}", Uuid::new_v4());
    let parent_marker = format!("PARENT_RECEIVED_{}", Uuid::new_v4());
    let cwd = root
        .join("project")
        .canonicalize()
        .map_err(|_| "项目目录无效")?;
    let python =
        PathBuf::from(env::var_os("INFINISHELL_CODEX_LIVE_PYTHON").ok_or("缺少固定 Python")?)
            .canonicalize()
            .map_err(|_| "缺少固定 Python")?;
    let gate_script = cwd.join("duplex-gate.py");
    let gate_source = "print('DUPLEX_GATE')\n";
    fs::write(&gate_script, gate_source).map_err(|_| "审批停点准备失败")?;
    let argv = json!([python, gate_script]);
    let command = format!(
        "'{}' '{}'",
        python.to_string_lossy().replace('\'', "'\"'\"'"),
        gate_script.to_string_lossy().replace('\'', "'\"'\"'")
    );
    let exchange = Exchange::new(&parent_id, &child_marker);
    let spawn = json!({"summary":"isolated parent-child verification","base_prompt":"",
        "harness":"codex","model_id":"","skills":[],"plan_id":"",
        "agent_run_configs":[{"name":"verified-child","title":"",
            "prompt":format!("This is an authorized isolated test. Call exec_command exactly once with this exact command: {command}. Do not substitute other tools. The host will hold approval until a parent follow-up arrives. After approval, obey that follow-up, then finish with {child_marker}. Do not edit files or create agents.")}]});
    let prompt = format!(
        "This is an authorized isolated integration test. Call run_agents exactly once with \
         these exact arguments: {spawn}. After it returns queued, call exec_command exactly once \
         with this exact command: {command}. The host holds approval until the child's progress \
         message has been accepted. After approval, finish with DISPATCHED. \
         Do not call any other tool or edit files. If a local_task_result or child progress message arrives, \
         retain these instructions and finish with DISPATCHED without calling additional tools. \
         Wait for an explicit follow-up before inspecting the child. Never create another child or resend a message."
    );
    let task = LocalCliTask {
        version: 1,
        task_id: parent_id.clone(),
        parent_task_id: None,
        parent_generation: None,
        harness: "codex".into(),
        working_directory: cwd.to_string_lossy().into(),
        config_json: json!({"permission_policy":"WorkspaceWrite"}).to_string(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    let options = SessionOptions {
        executable: PathBuf::from(
            env::var_os("INFINISHELL_CODEX_LIVE_EXECUTABLE").ok_or("缺少固定 CLI")?,
        ),
        cwd: cwd.clone(),
        state_dir: current_state_dir(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::WorkspaceWrite,
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
    let deadline = Instant::now() + Duration::from_secs(MAX_SECONDS);
    let mut tools = HashSet::new();
    let mut inputs = HashSet::new();
    let mut inspect = None;
    let mut mailbox_sent = false;
    let mut approved = HashSet::new();
    let mut seen_approvals = HashSet::new();
    let mut operations = HashSet::new();
    let continuation = Uuid::new_v4();
    let mut parent_generation = 0;
    let mut last_state = Value::Null;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wake = match receiver
            .recv()
            .with_timeout(remaining.min(Duration::from_secs(2)))
            .await
        {
            Ok(wake) => wake.ok_or("协调器事件已关闭")?,
            Err(_) => None,
        };
        if let Some((task, event)) = wake {
            if let RuntimeEventKind::LocalToolRequested { request } = &event.kind {
                let expected = match request.tool.as_str() {
                    "run_agents" => {
                        task.task_id == parent_id
                            && task.generation == 1
                            && request.arguments == spawn
                    }
                    "send_message_to_agent" => {
                        task.parent_task_id.as_deref() == Some(parent_id.as_str())
                            && task.generation == 1
                            && request.arguments
                                == json!({"addresses":[parent_id],"subject":exchange.to_parent,"message":exchange.parent_input})
                    }
                    "inspect_local_tasks" => {
                        task.task_id == parent_id
                            && task.generation == parent_generation
                            && inspect.as_ref() == Some(&request.arguments)
                    }
                    _ => false,
                };
                if !expected || !operations.insert((task.task_id.clone(), request.tool.clone())) {
                    return Err("请求了范围外的本地工具或参数".into());
                }
                tools.insert(request.call_id.clone());
            }
            if let RuntimeEventKind::MessageAccepted { message_id, .. } = &event.kind {
                inputs.insert((task.task_id.clone(), *message_id));
            }
            if let RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method,
                details,
            } = &event.kind
            {
                let safe = task.generation == 1
                    && (task.task_id == parent_id
                        || task.parent_task_id.as_deref() == Some(parent_id.as_str()))
                    && method == "item/commandExecution/requestApproval"
                    && coordinator.read(app, |model, _| {
                        model.snapshot(&task.task_id).is_some_and(|snapshot| {
                            snapshot.active_turn_id.as_deref() == Some(turn_id)
                        })
                    })
                    && gate_approval_matches(details, &command, &argv, &cwd)
                    && seen_approvals.insert(task.task_id.clone());
                if !safe {
                    if let Ok(reply) = coordinator.update(app, |model, _| {
                        model.request_for_generation(
                            &task.task_id,
                            task.generation,
                            Uuid::new_v4(),
                            RuntimeAction::RespondApproval {
                                approval_id: approval_id.clone(),
                                decision: ApprovalDecision::DenyOnce,
                            },
                        )
                    }) {
                        let _ = reply.with_timeout(Duration::from_secs(2)).await;
                    }
                    return Err("审批停点超出固定命令范围".into());
                }
            }
            if matches!(
                &event.kind,
                RuntimeEventKind::Disconnected { .. }
                    | RuntimeEventKind::RequestFailed { .. }
                    | RuntimeEventKind::TurnFinished {
                        outcome: TurnOutcome::Failed { .. } | TurnOutcome::Cancelled,
                        ..
                    }
            ) {
                return Err("真实连接提前关闭或回合失败".into());
            }
            evidence.record(
                json!({"event":"runtime_observed","parent":task.task_id == parent_id,
                "runtime_stage":event_stage(&event.kind),"generation":task.generation,
                "state":task.state,"has_result":task.result.is_some(),
                "has_terminal_evidence":task.terminal_evidence.is_some(),
                "native_tool_calls":tools.len(),"accepted_inputs":inputs.len()}),
            )?;
        }
        if tools.len() > 3 || inputs.len() > MAX_INPUTS || seen_approvals.len() > 2 {
            return Err("原生工具或输入超过预算".into());
        }
        if coordinator.read(app, |model, _| {
            model.snapshots().any(|snapshot| snapshot.error.is_some())
        }) {
            return Err("生产协调器报告错误".into());
        }
        let saved = load_tasks(sender, false)?
            .await
            .map_err(|_| "任务读取确认已关闭")??;
        let parent = saved.iter().find(|task| task.task_id == parent_id);
        let children = saved
            .iter()
            .filter(|task| task.parent_task_id.as_deref() == Some(parent_id.as_str()))
            .collect::<Vec<_>>();
        if saved.len() > 2 || children.len() > 1 {
            return Err("生成了多余任务".into());
        }
        let messages = load_task_messages(sender, parent_id.clone())?
            .await
            .map_err(|_| "结果消息读取确认已关闭")??;
        let snapshots = coordinator.read(app, |model, _| {
            model.snapshots().cloned().collect::<Vec<_>>()
        });
        if let (Some(parent), Some(child)) = (parent, children.first()) {
            let down_ack = native_message_ack(
                &messages,
                &parent_id,
                &child.task_id,
                &exchange.to_child,
                &exchange.child_input,
            );
            let up_ack = native_message_ack(
                &messages,
                &child.task_id,
                &parent_id,
                &exchange.to_parent,
                &exchange.parent_input,
            );
            let parent_waiting = snapshots.iter().any(|snapshot| {
                snapshot.task.task_id == parent_id && snapshot.approvals.len() == 1
            });
            let child_waiting = snapshots.iter().any(|snapshot| {
                snapshot.task.task_id == child.task_id && snapshot.approvals.len() == 1
            });
            if !mailbox_sent && parent_waiting && child_waiting {
                // 复用生产托管邮箱事务；来源、接收代次及创建父权限由同一领取函数核对。
                let endpoint = coordinator
                    .read(app, |model, _| model.endpoint(&child.task_id))
                    .ok_or("子任务没有活动消息入口")?;
                let message = LocalCliMessage {
                    version: 1,
                    message_id: Uuid::new_v4().to_string(),
                    sender_task_id: parent_id.clone(),
                    recipient_task_id: child.task_id.clone(),
                    sender_generation: parent.generation,
                    recipient_generation: child.generation,
                    subject: exchange.to_child.clone(),
                    body: exchange.child_input.clone(),
                    state: LocalCliMessageState::Queued,
                    receipt_kind: None,
                };
                tools::dispatch_managed_message(
                    sender,
                    endpoint,
                    parent.clone(),
                    (*child).clone(),
                    message,
                )
                .with_timeout(Duration::from_secs(10))
                .await
                .map_err(|_| "父子邮箱派发超时")??;
                mailbox_sent = true;
                evidence.record(json!({"event":"parent_mailbox_dispatched","source":"production_managed_mailbox","native_ack_verified":false}))?;
            }
            for snapshot in &snapshots {
                let is_parent = snapshot.task.task_id == parent_id;
                let ready = if is_parent {
                    up_ack && child.state == LocalCliTaskState::Completed
                } else {
                    down_ack && parent_waiting
                };
                if ready
                    && !approved.contains(&snapshot.task.task_id)
                    && let Some(approval) = snapshot.approvals.first()
                {
                    if snapshot.task.generation != 1
                        || !seen_approvals.contains(&snapshot.task.task_id)
                        || !gate_approval_matches(&approval.details, &command, &argv, &cwd)
                        || fs::read_to_string(&gate_script).ok().as_deref() != Some(gate_source)
                    {
                        return Err("审批停点超出固定命令范围".into());
                    }
                    let reply = coordinator.update(app, |model, _| {
                        model.request_for_generation(
                            &snapshot.task.task_id,
                            1,
                            Uuid::new_v4(),
                            RuntimeAction::RespondApproval {
                                approval_id: approval.approval_id.clone(),
                                decision: ApprovalDecision::AllowOnce,
                            },
                        )
                    })?;
                    reply
                        .with_timeout(Duration::from_secs(10))
                        .await
                        .map_err(|_| "审批停点确认超时")?
                        .map_err(|_| "审批停点确认已关闭")??;
                    approved.insert(snapshot.task.task_id.clone());
                    evidence.record(json!({"event":"readonly_gate_released","parent":is_parent,"native_message_ack_verified":true}))?;
                }
            }
        }
        let automatic = messages
            .iter()
            .filter(|message| message.subject == "local_task_result")
            .collect::<Vec<_>>();
        let state = json!({"event":"state_observed","phase":if inspect.is_some() {"inspect_result"} else {"child_completion"},
            "parent_state":parent.map(|task| task.state),"parent_generation":parent.map(|task| task.generation),
            "child_state":children.first().map(|task| task.state),
            "child_result_matches":children.first().is_some_and(|task|task.result.as_ref().is_some_and(|result|result.contains(&child_marker))),
            "parent_result_matches":parent.is_some_and(|task|task.result.as_ref().is_some_and(|result|result.contains(&parent_marker))),
            "automatic_result_states":automatic.iter().map(|message|message.state).collect::<Vec<_>>(),
            "automatic_result_native_ack_verified":automatic.iter().any(|message|message.state==LocalCliMessageState::Acknowledged&&message.receipt_kind==Some(LocalCliReceiptKind::NativeProtocol))});
        if state != last_state {
            evidence.record(state.clone())?;
            last_state = state;
        }
        if inspect.is_none()
            && let (Some(parent), Some(child)) = (parent, children.first())
            && parent.state == LocalCliTaskState::Completed
            && child.state == LocalCliTaskState::Completed
            && !automatic.is_empty()
        {
            if !child
                .result
                .as_ref()
                .is_some_and(|result| result.contains(&child_marker))
            {
                return Err("子任务完成但结果标记不符".into());
            }
            // 已结束父代不会被结果自动唤醒；显式继续沿用生产输入和原生 inspect 工具。
            let arguments =
                json!({"task_ids":[child.task_id],"result_generation":1,"result_offset":0});
            let followup = format!(
                "Explicitly continue this isolated test. Call inspect_local_tasks exactly once with these exact arguments: {arguments}. Read the completed child's actual result from that tool response, then reply {parent_marker} followed by that retrieved result. Do not create agents, send messages, edit files, or call any other tool."
            );
            parent_generation = parent.generation + 1;
            let reply = coordinator.update(app, |model, _| {
                model.request_for_generation(
                    &parent_id,
                    parent.generation,
                    continuation,
                    RuntimeAction::Submit {
                        input: vec![InputContent::Text(followup)],
                    },
                )
            })?;
            reply
                .with_timeout(Duration::from_secs(10))
                .await
                .map_err(|_| "显式继续确认超时")?
                .map_err(|_| "显式继续确认已关闭")??;
            inspect = Some(arguments);
            evidence.record(
                json!({"event":"explicit_inspect_requested","parent_generation":parent_generation,
                "automatic_result_enqueued_verified":true}),
            )?;
        }
        if tools.len() == 3
            && approved.len() == 2
            && inspect.is_some()
            && let Some(automatic_ack) = verified_chain(
                sender,
                &parent_id,
                &child_marker,
                &parent_marker,
                continuation,
                parent_generation,
                &exchange,
            )
            .await?
        {
            evidence.record(json!({"event":"parent_child_chain_verified","native_tool_calls":3,"readonly_approvals":approved.len(),
                "parent_to_child_native_ack_verified":true,"child_to_parent_native_ack_verified":true,
                "parent_message_source":"production_managed_mailbox","child_message_source":"native_send_message_to_agent",
                "accepted_inputs":inputs.len(),"child_count":1,"parent_permission_ceiling_verified":true,
                "automatic_result_native_ack_verified":automatic_ack,"automatic_result_enqueued_verified":true,
                "explicit_continuation_native_ack_verified":true,"native_inspect_result_verified":true,"child_result_verified":true,
                "parent_result_verified":true}))?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("父子链超过时间预算".into());
        }
    }
}

async fn shutdown(
    app: &mut App,
    coordinator: &ModelHandle<LocalCLITaskCoordinator>,
) -> Result<usize, String> {
    let ids = coordinator.read(app, |model, _| {
        model
            .snapshots()
            .filter(|snapshot| snapshot.connected)
            .map(|snapshot| snapshot.task.task_id.clone())
            .collect::<Vec<_>>()
    });
    for id in ids {
        if let Ok(reply) = coordinator.update(app, |model, _| {
            model.request(&id, Uuid::new_v4(), RuntimeAction::Shutdown)
        }) {
            let _ = reply.with_timeout(Duration::from_secs(5)).await;
        }
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let snapshots = coordinator.read(app, |model, _| {
            model.snapshots().cloned().collect::<Vec<_>>()
        });
        let mut confirmed = 0;
        for snapshot in &snapshots {
            let config: Value =
                serde_json::from_str(&snapshot.task.config_json).map_err(|_| "清理配置无效")?;
            let generation: Uuid = serde_json::from_value(config["runtime_generation"].clone())
                .map_err(|_| "缺少监督代次")?;
            if !snapshot.connected
                && confirmed_exit(&current_state_dir(), generation)
                    .map_err(|_| "监督回执无效")?
                    .is_some()
            {
                confirmed += 1;
            }
        }
        if confirmed == snapshots.len() {
            return Ok(confirmed);
        }
        if Instant::now() >= deadline {
            return Err("缺少完整父子监督退出回执".into());
        }
        Timer::after(Duration::from_millis(50)).await;
    }
}

#[test]
#[ignore = "仅由 parent-child-01561 隔离运行器显式启用；调用真实模型"]
fn real_codex_01561_parent_child() {
    assert_eq!(
        env::var("INFINISHELL_CODEX_PARENT_CHILD_01561").as_deref(),
        Ok("1")
    );
    assert!(env::var_os("INFINISHELL_CODEX_TEST_CANDIDATE_01561").is_none());
    let root = PathBuf::from(env::var_os("INFINISHELL_CODEX_LIVE_ROOT").expect("缺少私有根目录"))
        .canonicalize()
        .unwrap();
    assert!(root.join(".infinishell-live-probe").is_file());
    assert_eq!(
        PathBuf::from(env::var_os("HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("home")
    );
    assert_eq!(
        PathBuf::from(env::var_os("CODEX_HOME").unwrap())
            .canonicalize()
            .unwrap(),
        root.join("codex")
    );
    assert!(current_state_dir().starts_with(root.join("home")));
    let mut evidence = Evidence(
        File::create(PathBuf::from(
            env::var_os("INFINISHELL_CODEX_LIVE_ARTIFACT").unwrap(),
        ))
        .unwrap(),
    );
    evidence
        .record(
            json!({"event":"acceptance_started","scope":SCOPE,"cli_version":"0.156.1",
        "test_only_candidate_01561":false,"max_native_inputs":MAX_INPUTS,"max_native_tools":3,"max_readonly_approvals":2,
        "max_seconds":MAX_SECONDS,"gui_verified":false,"bidirectional_messages_verified":false,
        "child_file_effect_verified":false}),
        )
        .unwrap();
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        let _enabled = FeatureFlag::LocalCLIManagedTasks.override_enabled(true);
        let executable = PathBuf::from(env::var_os("INFINISHELL_CODEX_LIVE_EXECUTABLE").unwrap());
        app.add_singleton_model(|_| {
            CLIAgentInstallModel::with_installation_for_test(
                CLIAgent::Codex,
                CLIAgentInstallation {
                    executable: Some(executable),
                    version: CLIAgentVersionStatus::Detected("0.156.1".into()),
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
                let value = if let LocalCLITaskCoordinatorEvent::Runtime { task, event } = event {
                    Some((task.clone(), event.clone()))
                } else {
                    None
                };
                let _ = wake.send(value);
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
        let cleanup = shutdown(&mut app, &coordinator).await;
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        let cleanup_count = cleanup.as_ref().copied().unwrap_or_default();
        let passed = result.is_ok() && cleanup.is_ok() && cleanup_count == 2;
        let failure = result
            .as_ref()
            .err()
            .map(|error| failure_code(error))
            .or_else(|| cleanup.is_err().then_some("cleanup_failed"))
            .or_else(|| (cleanup_count != 2).then_some("cleanup_receipt_count"));
        evidence.record(json!({"event":"parent_child_finished","scope":SCOPE,"passed":passed,
            "failure_code":failure,
            "cli_version":"0.156.1","test_only_candidate_01561":false,
            "chain_verified":result.is_ok(),"cleanup_confirmed":cleanup.is_ok(),"cleanup_receipts":cleanup_count,
            "gui_verified":false,"bidirectional_messages_verified":result.is_ok(),"child_file_effect_verified":false})).unwrap();
        assert!(
            passed,
            "固定 Codex 父子派发或监督清理未通过，参见安全事件记录"
        );
    });
}

#[test]
fn duplex_ack_requires_one_exact_native_receipt_in_the_creation_generation() {
    let message = LocalCliMessage {
        version: 1,
        message_id: Uuid::new_v4().to_string(),
        sender_task_id: "parent".into(),
        recipient_task_id: "child".into(),
        sender_generation: 1,
        recipient_generation: 1,
        subject: "followup".into(),
        body: "bound input".into(),
        state: LocalCliMessageState::Acknowledged,
        receipt_kind: Some(LocalCliReceiptKind::NativeProtocol),
    };
    let matches = |messages: &[LocalCliMessage]| {
        native_message_ack(messages, "parent", "child", "followup", "bound input")
    };
    assert!(matches(&[message.clone()]));
    assert!(!matches(&[message.clone(), message.clone()]));
    let mut wrong = message.clone();
    wrong.state = LocalCliMessageState::Sent;
    assert!(!matches(&[wrong]));
    let mut wrong = message.clone();
    wrong.receipt_kind = Some(LocalCliReceiptKind::ApplicationHistory);
    assert!(!matches(&[wrong]));
    let mut wrong = message.clone();
    wrong.sender_generation = 2;
    assert!(!matches(&[wrong]));
    let mut wrong = message.clone();
    wrong.recipient_generation = 2;
    assert!(!matches(&[wrong]));
    let mut wrong = message.clone();
    wrong.recipient_task_id = "other-child".into();
    assert!(!matches(&[wrong]));
    let mut wrong = message;
    wrong.body = "other input".into();
    assert!(!matches(&[wrong]));
}

#[test]
fn duplex_gate_rejects_extra_commands_policy_and_wrong_directory() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let command = "'/usr/bin/python3' 'duplex-gate.py'";
    let argv = json!(["/usr/bin/python3", "duplex-gate.py"]);
    let details = json!({"command":command,"proposedExecpolicyAmendment":argv,
        "commandActions":[{"command":command}],"cwd":cwd});
    assert!(gate_approval_matches(&details, command, &argv, &cwd));
    let mut wrapped = details.clone();
    wrapped["command"] = json!(shell_words::join(["/bin/zsh", "-lc", command]));
    wrapped["proposedExecpolicyAmendment"] = json!(["/bin/zsh", "-lc", command]);
    assert!(gate_approval_matches(&wrapped, command, &argv, &cwd));
    let mut wrong = details.clone();
    wrong["command"] = json!(format!("{command}; another-command"));
    assert!(!gate_approval_matches(&wrong, command, &argv, &cwd));
    let mut wrong = details.clone();
    wrong["proposedExecpolicyAmendment"] = json!(["/usr/bin/python3"]);
    assert!(!gate_approval_matches(&wrong, command, &argv, &cwd));
    let mut wrong = details;
    wrong["cwd"] = json!(cwd.join("elsewhere"));
    assert!(!gate_approval_matches(&wrong, command, &argv, &cwd));
}
