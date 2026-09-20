//! 原生工具使用连接身份执行；副作用前记录调用，重复调用只读取已存结果。

use std::time::Duration;

use ai::skills::{SkillPathOrigin, skill_reference_from_api_skill_ref};
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::Value;
use sha2::{Digest, Sha256};
use warp_multi_agent_api as api;
use warpui::r#async::Timer;

use super::*;
use crate::ai::agent_providers::tools::local_orchestration::{LocalHarness, LocalRunAgents};
use crate::ai::cli_agent_runtime::PermissionPolicy;
use crate::ai::cli_agent_runtime::conversation_bridge::recorded_history_identity;
use crate::ai::cli_agent_runtime::local_skills::{
    collect_local_child_skills, prepare_local_cli_skill_inputs,
};
use crate::ai::cli_agent_runtime::local_tools::{
    BoundLocalToolCall, LocalToolOperation, NativeLocalToolRequest, ResultPage,
    TrustedLocalToolContext, bind_local_tool_call,
};
use crate::ai::cli_agent_runtime::permissions::ceiling_from_parent;
use crate::ai::local_cli_mailbox::send_local_message;
use crate::persistence::local_cli_tasks::{load_task_generations, load_task_messages};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent::CLIAgentInstallModel;

pub(super) struct ToolCompletion {
    pub generation: i64,
    pub turn_id: String,
    pub call_id: String,
    pub receipt_id: Option<Uuid>,
    pub result: Result<Value, String>,
}

pub(super) fn execute(
    request: NativeLocalToolRequest,
    snapshot: ManagedTaskSnapshot,
    options: SessionOptions,
    sender: SyncSender<ModelEvent>,
    spawner: ModelSpawner<LocalCLITaskCoordinator>,
) -> BoxFuture<'static, ToolCompletion> {
    async move {
        let generation = snapshot.task.generation;
        let turn_id = request.turn_id.clone();
        let call_id = request.call_id.clone();
        let (receipt_id, result) =
            match execute_inner(request, snapshot, options, sender, spawner).await {
                Ok((id, result)) => (Some(id), result),
                Err(error) => (None, Err(error)),
            };
        ToolCompletion {
            generation,
            turn_id,
            call_id,
            receipt_id,
            result,
        }
    }
    .boxed()
}

async fn execute_inner(
    request: NativeLocalToolRequest,
    snapshot: ManagedTaskSnapshot,
    options: SessionOptions,
    sender: SyncSender<ModelEvent>,
    spawner: ModelSpawner<LocalCLITaskCoordinator>,
) -> Result<(Uuid, Result<Value, String>), String> {
    let permissions = options
        .local_tools
        .as_ref()
        .ok_or("当前连接未授权本地任务工具")?;
    let records = records(&sender).await?;
    let source = records
        .iter()
        .find(|task| task.task_id == snapshot.task.task_id)
        .ok_or("本地工具调用方没有已提交任务记录")?;
    if source.generation != snapshot.task.generation || !source.state.is_active() {
        return Err("本地工具调用方已换代或不再运行".into());
    }
    let related_task_ids = records
        .iter()
        .filter(|task| {
            task.parent_task_id.as_ref() == Some(&source.task_id)
                || source.parent_task_id.as_ref() == Some(&task.task_id)
        })
        .map(|task| task.task_id.clone())
        .collect();
    let context = TrustedLocalToolContext {
        task_id: source.task_id.clone(),
        generation: source.generation,
        runtime_generation: options.generation,
        active_turn_id: snapshot
            .active_turn_id
            .clone()
            .ok_or("本地工具没有当前回合")?,
        allow_spawn: permissions.allow_spawn,
        allow_message: permissions.allow_message,
        grok_creation_policy_bound: source.harness == "grok"
            && ceiling_from_parent(source, "grok").is_ok(),
        related_task_ids,
    };
    let call = bind_local_tool_call(request, &context)?;
    ensure_current(&spawner, &call).await?;
    let receipt_id = derived_id(call.message_id, "result");
    let message = tool_record(
        source,
        call.message_id,
        "native_tool_call",
        serde_json::to_string(&call.request).map_err(|error| error.to_string())?,
    );
    let outcome = enqueue_message(&sender, message.clone())?
        .await
        .map_err(|_| "本地工具调用入队确认已关闭")??;
    if matches!(outcome, LocalCliEnqueueOutcome::Existing(_)) {
        let stored = load_messages(&sender, source.task_id.clone(), source.generation)?
            .await
            .map_err(|_| "本地工具结果读取确认已关闭")??;
        return match stored
            .iter()
            .find(|message| message.message_id == receipt_id.to_string())
        {
            Some(receipt) => Ok((
                receipt_id,
                serde_json::from_str::<Result<Value, String>>(&receipt.body)
                    .map_err(|error| error.to_string())?,
            )),
            None => Err(format!(
                "调用 {} 已尝试执行但结果未确认，请检查已有任务与消息，不能自动重放",
                call.message_id
            )),
        };
    }
    set_record_sent(&sender, &message).await?;
    publish_message_change(&spawner, &message).await;
    let result = match &call.operation {
        LocalToolOperation::Inspect {
            task_ids,
            result_page,
        } => inspect(&sender, &records, task_ids, result_page).await,
        LocalToolOperation::Send(message) => send(&sender, &spawner, &call, message).await,
        LocalToolOperation::Spawn(run) => {
            spawn_children(&sender, &spawner, &call, source, &records, &options, run).await
        }
    };
    let receipt = tool_record(
        source,
        receipt_id,
        "native_tool_result",
        serde_json::to_string(&result).map_err(|error| error.to_string())?,
    );
    enqueue_message(&sender, receipt.clone())?
        .await
        .map_err(|_| "本地工具结果提交确认已关闭")??;
    publish_message_change(&spawner, &receipt).await;
    Ok((receipt_id, result))
}

/// 先记录交付尝试再回写原生工具；管道写入事件不能把它升级为已确认。
pub(super) async fn prepare_tool_reply(
    sender: &SyncSender<ModelEvent>,
    task: &LocalCliTask,
    completion: &ToolCompletion,
) -> Result<Uuid, String> {
    let receipt_id = match completion.receipt_id {
        Some(receipt_id) => receipt_id,
        None => {
            // 验证失败、并发限额和结果写入失败也必须先持久化；不能绕过交付记录。
            let identity = serde_json::to_string(&(
                &task.task_id,
                task.generation,
                &completion.turn_id,
                &completion.call_id,
            ))
            .map_err(|error| error.to_string())?;
            let receipt_id = derived_id(Uuid::nil(), &identity);
            let receipt = tool_record(
                task,
                receipt_id,
                "native_tool_result",
                serde_json::to_string(&completion.result).map_err(|error| error.to_string())?,
            );
            enqueue_message(sender, receipt)?
                .await
                .map_err(|_| "工具错误结果提交确认已关闭")??;
            receipt_id
        }
    };
    let stored = load_messages(sender, task.task_id.clone(), task.generation)?
        .await
        .map_err(|_| "工具结果读取确认已关闭")??;
    let receipt = stored
        .iter()
        .find(|message| message.message_id == receipt_id.to_string())
        .ok_or("工具结果缺少已提交记录")?;
    if receipt.subject != "native_tool_result"
        || receipt.sender_task_id != task.task_id
        || receipt.recipient_task_id != task.task_id
        || receipt.body
            != serde_json::to_string(&completion.result).map_err(|error| error.to_string())?
        || receipt.state != LocalCliMessageState::Queued
    {
        return Err("工具结果已经尝试交付或与保存记录不一致，不能自动重投".into());
    }
    set_record_sent(sender, receipt).await?;
    Ok(receipt_id)
}

async fn records(sender: &SyncSender<ModelEvent>) -> Result<Vec<LocalCliTask>, String> {
    load_tasks(sender, false)?
        .await
        .map_err(|_| "本地任务读取确认已关闭")?
}

async fn ensure_current(
    spawner: &ModelSpawner<LocalCLITaskCoordinator>,
    call: &BoundLocalToolCall,
) -> Result<(), String> {
    let task_id = call.sender_task_id.clone();
    let token = call.runtime_generation;
    let generation = call.generation;
    let turn_id = call.request.turn_id.clone();
    let call_id = call.request.call_id.clone();
    spawner
        .spawn(move |model, _| {
            check_current(model, &task_id, token, generation, &turn_id, &call_id)
        })
        .await
        .map_err(|_| "本地任务应用已关闭")?
}

fn check_current(
    model: &LocalCLITaskCoordinator,
    task_id: &str,
    token: Uuid,
    generation: i64,
    turn_id: &str,
    call_id: &str,
) -> Result<(), String> {
    if model.entries.get(task_id).is_some_and(|entry| {
        entry.token == token
            && entry.snapshot.connected
            && entry.snapshot.task.generation == generation
            && entry.snapshot.task.state.is_active()
            && entry.snapshot.active_turn_id.as_deref() == Some(turn_id)
            && entry.pending_tool_calls.contains(call_id)
    }) {
        Ok(())
    } else {
        Err("本地工具调用属于已结束的连接或回合".into())
    }
}

fn tool_record(task: &LocalCliTask, id: Uuid, subject: &str, body: String) -> LocalCliMessage {
    LocalCliMessage {
        version: 1,
        message_id: id.to_string(),
        sender_task_id: task.task_id.clone(),
        recipient_task_id: task.task_id.clone(),
        sender_generation: task.generation,
        recipient_generation: task.generation,
        subject: subject.into(),
        body,
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    }
}

async fn set_record_sent(
    sender: &SyncSender<ModelEvent>,
    message: &LocalCliMessage,
) -> Result<(), String> {
    acknowledge_message(
        sender,
        message.message_id.clone(),
        message.recipient_task_id.clone(),
        message.recipient_generation,
        LocalCliMessageState::Sent,
    )?
    .await
    .map_err(|_| "本地工具交付记录确认已关闭")?
}

fn derived_id(parent: Uuid, key: &str) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(parent.as_bytes());
    digest.update(key.as_bytes());
    let bytes = digest.finalize();
    Uuid::from_bytes(bytes[..16].try_into().expect("SHA-256 至少包含 16 字节"))
}

async fn inspect(
    sender: &SyncSender<ModelEvent>,
    records: &[LocalCliTask],
    task_ids: &[String],
    result_page: &ResultPage,
) -> Result<Value, String> {
    let mut tasks = Vec::new();
    for id in task_ids {
        let task = records
            .iter()
            .find(|task| &task.task_id == id)
            .ok_or("被查询任务不存在")?;
        let generations = load_task_generations(sender, id.clone())?
            .await
            .map_err(|_| "任务历史读取确认已关闭")??;
        let history_truncated = generations.len() > 16;
        let selected = match result_page.result_generation {
            Some(generation) => generations
                .iter()
                .find(|previous| previous.generation == generation)
                .ok_or("被查询任务不存在指定代次")?,
            None => task,
        };
        let full_result = selected.result.as_deref().unwrap_or_default();
        let offset = result_page.result_offset;
        if offset > full_result.len() || !full_result.is_char_boundary(offset) {
            return Err("结果偏移必须是完整结果范围内的 UTF-8 字符边界".into());
        }
        let end =
            full_result.floor_char_boundary(offset.saturating_add(8192).min(full_result.len()));
        let result = &full_result[offset..end];
        let truncated = end < full_result.len();
        let mut messages = load_task_messages(sender, id.clone())?
            .await
            .map_err(|_| "消息状态读取确认已关闭")??;
        messages.reverse();
        tasks.push(json!({"task_id":id,"parent_task_id":selected.parent_task_id,"generation":selected.generation,
            "current_generation":task.generation,
            "state":selected.state,"native_session_id":selected.native_session_id,"result":result,"result_truncated":truncated,
            "result_available":selected.result.is_some(),"result_offset":offset,"result_total_bytes":full_result.len(),
            "result_next_offset":truncated.then_some(end),
            "generations":generations.iter().rev().take(16).map(|previous| json!({
                "generation":previous.generation,"state":previous.state,"result_available":previous.result.is_some()
            })).collect::<Vec<_>>(),
            "history_truncated":history_truncated,"messages_truncated":messages.len()>64,
            "messages":messages.iter().take(64).map(|message| {
                let end=message.body.floor_char_boundary(message.body.len().min(1024));
                json!({"message_id":message.message_id,"generation":message.recipient_generation,
                    "sender_task_id":message.sender_task_id,"recipient_task_id":message.recipient_task_id,
                    "sender_generation":message.sender_generation,"recipient_generation":message.recipient_generation,
                    "subject":message.subject,"state":message.state,
                    "receipt_kind":message.receipt_kind,
                    "body":&message.body[..end],"body_truncated":end<message.body.len()})
            }).collect::<Vec<_>>() }));
        if serde_json::to_vec(&tasks)
            .map_err(|error| error.to_string())?
            .len()
            > 512 * 1024
        {
            return Err("查询结果过大，请减少一次查询的任务数；完整记录仍保存在本地".into());
        }
    }
    Ok(
        json!({"tasks":tasks,"delivery_note":"Sent is an attempt. For Acknowledged, inspect receipt_kind: native_protocol confirms CLI receipt; application_history confirms saved application context for its next normal request, not immediate execution. A missing or unknown receipt kind does not establish native receipt. Do not automatically resend."}),
    )
}

async fn send(
    sender: &SyncSender<ModelEvent>,
    spawner: &ModelSpawner<LocalCLITaskCoordinator>,
    call: &BoundLocalToolCall,
    request: &api::SendMessageToAgent,
) -> Result<Value, String> {
    if request.addresses.contains(&call.sender_task_id) {
        return Err("父子消息不能发给当前任务自身".into());
    }
    if matches!(
        request.subject.as_str(),
        "local_task_result" | "native_tool_call" | "native_tool_result"
    ) {
        return Err("普通消息不能使用内部结果或工具记录主题".into());
    }
    let mut dispatched = Vec::new();
    for recipient in &request.addresses {
        ensure_current(spawner, call).await?;
        let saved = records(sender).await?;
        let source = saved
            .iter()
            .find(|task| task.task_id == call.sender_task_id)
            .ok_or("消息子任务不存在")?
            .clone();
        let target = saved
            .iter()
            .find(|task| &task.task_id == recipient)
            .ok_or("消息接收任务不存在")?;
        let application_history = target.harness == "oz";
        if application_history
            && (source.generation != call.generation
                || source.parent_task_id.as_ref() != Some(recipient)
                || source.parent_generation != Some(target.generation)
                || !target.state.is_active()
                || recorded_history_identity(target).is_none())
        {
            return Err("普通进度只能发送到创建时的活动 Oz 父运行".into());
        }
        let address = recipient.clone();
        let endpoint = if application_history {
            None
        } else {
            Some(
                spawner
                    .spawn(move |model, _| model.endpoint(&address))
                    .await
                    .map_err(|_| "本地任务应用已关闭")?
                    .ok_or("接收任务没有活跃托管连接")?,
            )
        };
        ensure_current(spawner, call).await?;
        let message = LocalCliMessage {
            version: 1,
            message_id: derived_id(call.message_id, recipient).to_string(),
            sender_task_id: call.sender_task_id.clone(),
            recipient_task_id: recipient.clone(),
            sender_generation: call.generation,
            recipient_generation: endpoint
                .as_ref()
                .map_or(target.generation, |endpoint| endpoint.generation),
            subject: request.subject.clone(),
            body: request.message.clone(),
            state: LocalCliMessageState::Queued,
            receipt_kind: None,
        };
        let status = if let Some(endpoint) = endpoint {
            send_local_message(sender, endpoint, message.clone())
                .await
                .map(|_| ())
        } else {
            let outcome = enqueue_message(sender, message.clone())?
                .await
                .map_err(|_| "普通父消息入队确认已关闭")??;
            // 已发送而未入历史的消息不能重投；重复 Queued 仍由 SQLite 原子领取。
            if matches!(
                outcome,
                LocalCliEnqueueOutcome::Created
                    | LocalCliEnqueueOutcome::Existing(LocalCliMessageState::Queued)
            ) {
                let call = call.clone();
                let message = message.clone();
                spawner
                    .spawn(move |model, ctx| {
                        check_current(
                            model,
                            &call.sender_task_id,
                            call.runtime_generation,
                            call.generation,
                            &call.request.turn_id,
                            &call.request.call_id,
                        )?;
                        ctx.emit(LocalCLITaskCoordinatorEvent::ParentMessageReady {
                            task: source,
                            message,
                        });
                        Ok::<_, String>(())
                    })
                    .await
                    .map_err(|_| "本地任务应用已关闭")?
            } else {
                Ok(())
            }
        };
        publish_message_change(spawner, &message).await;
        dispatched.push(message);
        if let Err(error) = status {
            return Err(json!({"error":error,"attempted_messages":dispatched.iter().map(|message| &message.message_id).collect::<Vec<_>>()}).to_string());
        }
    }
    for attempt in 0..31 {
        let mut confirmed = true;
        for message in &mut dispatched {
            let stored = load_messages(
                sender,
                message.recipient_task_id.clone(),
                message.recipient_generation,
            )?
            .await
            .map_err(|_| "消息回执读取确认已关闭")??;
            *message = stored
                .iter()
                .find(|item| item.message_id == message.message_id)
                .ok_or("已提交消息丢失")?
                .clone();
            confirmed &= message.state == LocalCliMessageState::Acknowledged
                && message.receipt_kind.is_some();
        }
        if confirmed {
            return Ok(
                json!({"status":"acknowledged","message_ids":dispatched.iter().map(|message| &message.message_id).collect::<Vec<_>>(),
                    "receipts":dispatched.iter().map(|message|json!({"message_id":message.message_id,"receipt_kind":message.receipt_kind})).collect::<Vec<_>>(),
                    "delivery_note":"application_history means saved context for the parent's next normal request, not immediate execution; native_protocol means the CLI accepted the input."}),
            );
        }
        if attempt < 30 {
            Timer::after(Duration::from_millis(100)).await;
        }
    }
    Err(json!({"status":"unconfirmed","messages":dispatched.iter().map(|message|json!({"message_id":message.message_id,"state":message.state,"receipt_kind":message.receipt_kind})).collect::<Vec<_>>(),"retry":"inspect existing message IDs; do not resend automatically"}).to_string())
}

async fn spawn_children(
    sender: &SyncSender<ModelEvent>,
    spawner: &ModelSpawner<LocalCLITaskCoordinator>,
    call: &BoundLocalToolCall,
    source: &LocalCliTask,
    records: &[LocalCliTask],
    options: &SessionOptions,
    run: &LocalRunAgents,
) -> Result<Value, String> {
    let (harness, agent) = match run.harness {
        LocalHarness::Codex => ("codex", CLIAgent::Codex),
        LocalHarness::Claude => ("claude", CLIAgent::Claude),
        LocalHarness::Grok => ("grok", CLIAgent::Grok),
    };
    if harness != source.harness && options.permission_policy == PermissionPolicy::Inherit {
        return Err("不同 CLI 的继承权限不等价，不能自动扩大子任务权限".into());
    }
    if harness == "claude" && options.permission_policy != PermissionPolicy::ClaudeRestrictedFilesV1
    {
        return Err(crate::t!("cli-agent-task-permission-ceiling-unavailable"));
    }
    validate_spawn_scope(source, records, run.agent_run_configs.len())?;
    let permission_ceiling =
        ceiling_from_parent(source, harness).map_err(|error| error.to_string())?;
    let references = run
        .skills
        .iter()
        .cloned()
        .map(|path| {
            let skill = api::SkillRef {
                skill_reference: Some(api::skill_ref::SkillReference::Path(path)),
            };
            skill_reference_from_api_skill_ref(skill, &SkillPathOrigin::Local)
                .ok_or_else(|| "子任务技能引用无效".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let saved_references = serde_json::to_value(&references).map_err(|error| error.to_string())?;
    let skills = spawner
        .spawn(move |_, ctx| collect_local_child_skills(&references, ctx))
        .await
        .map_err(|_| "本地任务应用已关闭")??;
    let skill_inputs = prepare_local_cli_skill_inputs(
        skills,
        Harness::parse_orchestration_harness(harness).ok_or("子任务 CLI 无效")?,
        true,
    )?;
    let mut children = Vec::new();
    for (index, child) in run.agent_run_configs.iter().enumerate() {
        ensure_current(spawner, call).await?;
        let executable = spawner
            .spawn(move |_, ctx| {
                CLIAgentInstallModel::as_ref(ctx)
                    .installation(agent)
                    .and_then(|installation| installation.executable.clone())
            })
            .await
            .map_err(|_| "本地任务应用已关闭")?
            .ok_or("子任务 CLI 未安装")?;
        let child_id = derived_id(call.message_id, &format!("child-{index}")).to_string();
        let mut config: Value =
            serde_json::from_str(&source.config_json).map_err(|error| error.to_string())?;
        // 原生版本与实际权限必须由子任务自己的连接确认，不能继承父任务的证据。
        if let Some(config) = config.as_object_mut() {
            config.remove("effective_permissions");
            config.remove("cli_version");
        }
        config["model"] = if run.model_id.is_empty() {
            Value::Null
        } else {
            json!(run.model_id)
        };
        config["skill_references"] = saved_references.clone();
        config["permission_ceiling"] =
            serde_json::to_value(&permission_ceiling).map_err(|error| error.to_string())?;
        let grok_profile = permission_ceiling
            .grok_profile()
            .map(|profile| profile.derive_child(options.local_tools))
            .transpose()
            .map_err(|error| error.to_string())?;
        config["grok_profile"] =
            serde_json::to_value(&grok_profile).map_err(|error| error.to_string())?;
        config["claude_profile"] = serde_json::to_value(permission_ceiling.claude_profile())
            .map_err(|error| error.to_string())?;
        config["name"] = json!(child.name);
        config["title"] = json!(child.title);
        let task = LocalCliTask {
            version: 1,
            task_id: child_id.clone(),
            parent_task_id: Some(source.task_id.clone()),
            parent_generation: Some(source.generation),
            harness: harness.into(),
            working_directory: source.working_directory.clone(),
            config_json: config.to_string(),
            native_session_id: None,
            generation: 1,
            revision: 0,
            state: LocalCliTaskState::Queued,
            result: None,
            terminal_evidence: None,
        };
        checkpoint_task(sender, task.clone(), None)?
            .await
            .map_err(|_| "子任务提交确认已关闭")??;
        let mut child_options = options.clone();
        child_options.permission_ceiling = Some(permission_ceiling.clone());
        child_options.claude_profile = permission_ceiling.claude_profile().cloned();
        child_options.grok_profile = grok_profile;
        child_options.executable = executable;
        child_options.target = SessionTarget::New;
        child_options.generation = Uuid::new_v4();
        child_options.model = (!run.model_id.is_empty()).then(|| run.model_id.clone());
        let mut input = vec![InputContent::Text(format!(
            "{}\n\n{}",
            run.base_prompt, child.prompt
        ))];
        input.extend(skill_inputs.clone());
        let task_id = call.sender_task_id.clone();
        let token = call.runtime_generation;
        let generation = call.generation;
        let turn_id = call.request.turn_id.clone();
        let call_id = call.request.call_id.clone();
        let prepared = task.clone();
        let start = spawner
            .spawn(move |model, ctx| {
                check_current(model, &task_id, token, generation, &turn_id, &call_id)?;
                let active_children = model
                    .entries
                    .values()
                    .filter(|entry| {
                        entry.snapshot.connected
                            && entry.snapshot.task.parent_task_id.as_ref() == Some(&task_id)
                    })
                    .count();
                let active_tasks = model
                    .entries
                    .values()
                    .filter(|entry| entry.snapshot.connected)
                    .count();
                if active_children >= 8 || active_tasks >= 64 {
                    return Err("本地任务并发数已达到上限".to_owned());
                }
                model.start_prepared_with_input(task, child_options, input, ctx)
            })
            .await
            .map_err(|_| "本地任务应用已关闭")?;
        if let Err(error) = start {
            let mut failed = prepared;
            failed.revision += 1;
            failed.state = LocalCliTaskState::Failed;
            failed.result = Some(error.clone());
            failed.terminal_evidence =
                Some(json!({"source":"application_dispatch","process_started":false}).to_string());
            checkpoint_task(sender, failed, Some(1))?
                .await
                .map_err(|_| "子任务启动失败记录确认已关闭")??;
            return Err(
                json!({"error":error,"failed_task_id":child_id,"queued_children":children})
                    .to_string(),
            );
        }
        children.push(json!({"name":child.name,"task_id":child_id,"status":"queued","native_receipt":"pending"}));
    }
    Ok(json!({"status":"queued","children":children}))
}

fn validate_spawn_scope(
    source: &LocalCliTask,
    records: &[LocalCliTask],
    count: usize,
) -> Result<(), String> {
    let active = records
        .iter()
        .filter(|task| {
            task.parent_task_id.as_ref() == Some(&source.task_id) && task.state.is_active()
        })
        .count();
    if count == 0 || active + count > 8 {
        return Err("每个本地任务最多同时运行八个子任务".into());
    }
    let mut parent = source.parent_task_id.as_deref();
    let mut seen = HashSet::from([source.task_id.as_str()]);
    while let Some(id) = parent {
        if !seen.insert(id) || seen.len() > 8 {
            return Err("本地任务层级已达上限或父子记录存在循环".into());
        }
        parent = records
            .iter()
            .find(|task| task.task_id == id)
            .ok_or("父任务记录缺失")?
            .parent_task_id
            .as_deref();
    }
    Ok(())
}

#[cfg(test)]
#[path = "coordinator_tools_tests.rs"]
mod tests;
