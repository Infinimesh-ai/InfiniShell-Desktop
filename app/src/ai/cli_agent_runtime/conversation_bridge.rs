//! 将已提交的托管任务状态和结果接到现有会话与父任务输入队列。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use warp_cli::agent::Harness;
use warp_multi_agent_api::message::Message;
use warp_multi_agent_api::{self as api};
use warpui::{Entity, ModelContext, ModelHandle, SingletonEntity};

use super::coordinator::{LocalCLITaskCoordinator, LocalCLITaskCoordinatorEvent};
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::{AIAgentInput, RenderableAIError};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::persistence::local_cli_tasks::{
    acknowledge_application_history, checkpoint_task, claim_parent_history_message,
    claim_task_result, load_tasks,
};
use crate::persistence::model::{
    LocalCliMessage, LocalCliMessageState, LocalCliTask, LocalCliTaskState,
};

pub(crate) struct LocalCLIConversationBridge {
    revisions: HashMap<String, (i64, i64)>,
    pending_results: HashMap<String, LocalCliMessage>,
    errors: HashMap<String, String>,
}

impl Entity for LocalCLIConversationBridge {
    type Event = ();
}

impl SingletonEntity for LocalCLIConversationBridge {}

impl LocalCLIConversationBridge {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&LocalCLITaskCoordinator::handle(ctx), Self::handle_runtime);
        ctx.subscribe_to_model(&BlocklistAIHistoryModel::handle(ctx), Self::handle_history);
        Self {
            revisions: HashMap::new(),
            pending_results: HashMap::new(),
            errors: HashMap::new(),
        }
    }

    fn handle_runtime(
        &mut self,
        _: ModelHandle<LocalCLITaskCoordinator>,
        event: &LocalCLITaskCoordinatorEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            LocalCLITaskCoordinatorEvent::Runtime { task, .. } => {
                let revision = (task.generation, task.revision);
                if self
                    .revisions
                    .get(&task.task_id)
                    .is_some_and(|previous| *previous >= revision)
                {
                    return;
                }
                self.revisions.insert(task.task_id.clone(), revision);
                let history = BlocklistAIHistoryModel::as_ref(ctx);
                let Some(conversation_id) = history.conversation_id_for_agent_id(&task.task_id)
                else {
                    return;
                };
                let Some(terminal_id) =
                    history.terminal_surface_id_for_conversation(&conversation_id)
                else {
                    return;
                };
                let (status, error) = status_for_task(task);
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.update_conversation_status_with_error(
                        terminal_id,
                        conversation_id,
                        status,
                        error.map(|error| RenderableAIError::other(error, false)),
                        ctx,
                    );
                });
            }
            LocalCLITaskCoordinatorEvent::ResultReady { task, message } => {
                if message.sender_task_id != task.task_id
                    || message.sender_generation != task.generation
                {
                    return;
                }
                self.deliver_message(message.clone(), true, ctx);
            }
            LocalCLITaskCoordinatorEvent::ParentMessageReady { task, message } => {
                if message.sender_task_id == task.task_id
                    && message.sender_generation == task.generation
                    && task.parent_task_id.as_ref() == Some(&message.recipient_task_id)
                    && task.parent_generation == Some(message.recipient_generation)
                    && message.subject != "local_task_result"
                {
                    self.deliver_message(message.clone(), false, ctx);
                }
            }
            LocalCLITaskCoordinatorEvent::Changed
            | LocalCLITaskCoordinatorEvent::MessagesChanged { .. } => {}
        }
    }

    fn deliver_message(
        &mut self,
        message: LocalCliMessage,
        is_result: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if !matches!(
            message.state,
            LocalCliMessageState::Queued | LocalCliMessageState::Sent
        ) || self.pending_results.contains_key(&message.message_id)
        {
            return;
        }
        let Some((conversation_id, identity)) = current_result_destination(&message, ctx) else {
            return;
        };
        let already_received = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| {
                conversation_has_message(conversation, &message.message_id)
            });
        if message.state == LocalCliMessageState::Sent && !already_received {
            return;
        }
        let Some(sender) = LocalCLITaskCoordinator::as_ref(ctx).sender() else {
            return;
        };
        self.pending_results
            .insert(message.message_id.clone(), message.clone());
        let message_id = message.message_id.clone();
        let saved_identity = identity.clone();
        ctx.spawn(
            async move {
                let tasks = load_tasks(&sender, false)?
                    .await
                    .map_err(|_| "父运行记录读取确认已关闭".to_owned())??;
                let parent = tasks
                    .iter()
                    .find(|task| task.task_id == message.recipient_task_id)
                    .ok_or_else(|| "父运行记录不存在".to_owned())?;
                if parent.generation != message.recipient_generation
                    || !parent.state.is_active()
                    || recorded_history_identity(parent).as_ref() != Some(&identity)
                {
                    return Err("父会话已经结束或进入另一轮，结果仅保留在任务历史".to_owned());
                }
                let claimed = if message.state == LocalCliMessageState::Queued {
                    let claim = if is_result {
                        claim_task_result(&sender, message.clone())?
                    } else {
                        claim_parent_history_message(&sender, message.clone())?
                    };
                    claim
                        .await
                        .map_err(|_| "父结果领取确认已关闭".to_owned())??
                        .is_some()
                } else {
                    false
                };
                Ok((message, claimed || already_received))
            },
            move |model, result, ctx| {
                match result {
                    Ok((message, true)) => {
                        if current_result_destination(&message, ctx)
                            != Some((conversation_id, saved_identity))
                        {
                            model.pending_results.remove(&message_id);
                            return;
                        }
                        model.append_result_to_history(conversation_id, message, ctx);
                    }
                    Ok((_, false)) => {
                        model.pending_results.remove(&message_id);
                    }
                    Err(error) => {
                        model.pending_results.remove(&message_id);
                        model.errors.insert(message_id, error);
                    }
                }
                ctx.notify();
            },
        );
    }

    fn append_result_to_history(
        &mut self,
        conversation_id: AIConversationId,
        message: LocalCliMessage,
        ctx: &mut ModelContext<Self>,
    ) {
        // 校验与同步插入之间没有异步间隙，不再依赖未被消费的事件队列。
        let checkpoint = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation = history
                .conversation(&conversation_id)
                .ok_or_else(|| "父会话不存在".to_owned())?;
            let root_task_id = conversation.get_root_task_id().clone();
            let already_received = conversation_has_message(conversation, &message.message_id);
            if !already_received {
                history
                    .append_byop_preflight_messages_to_task(
                        conversation_id,
                        root_task_id.clone(),
                        vec![history_result_message(&root_task_id.to_string(), &message)],
                        ctx,
                    )
                    .map_err(|error| error.to_string())?;
            }
            // source-history 的普通写入仅入队；另取现有检查点的事务确认后才能记应用接收。
            history
                .conversation(&conversation_id)
                .ok_or_else(|| "父会话不存在".to_owned())?
                .checkpoint_compaction_recovery_state(ctx)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "父会话持久化未启用，结果仍未确认".to_owned())
        });
        let message_id = message.message_id.clone();
        let Some(sender) = LocalCLITaskCoordinator::as_ref(ctx).sender() else {
            self.pending_results.remove(&message_id);
            return;
        };
        ctx.spawn(
            async move {
                checkpoint?
                    .await
                    .map_err(|_| "父会话提交确认已关闭".to_owned())??;
                acknowledge_application_history(
                    &sender,
                    message.message_id,
                    message.recipient_task_id,
                    message.recipient_generation,
                )?
                .await
                .map_err(|_| "父结果回执确认已关闭".to_owned())?
            },
            move |model, result, ctx| {
                model.pending_results.remove(&message_id);
                match result {
                    Ok(()) => {
                        model.errors.remove(&message_id);
                    }
                    Err(error) => {
                        model.errors.insert(message_id, error);
                    }
                }
                ctx.notify();
            },
        );
    }

    fn handle_history(
        &mut self,
        _: ModelHandle<BlocklistAIHistoryModel>,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let conversation_id = match event {
            BlocklistAIHistoryEvent::AppendedExchange {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id, ..
            } => *conversation_id,
            _ => return,
        };
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            return;
        };
        if conversation
            .orchestration_harness()
            .is_some_and(|harness| harness != Harness::Oz)
        {
            return;
        }
        let Some(run_id) = conversation.run_id() else {
            return;
        };
        let identity = local_parent_history_identity(conversation);
        let active = history_can_receive(conversation.status());
        let Some(sender) = LocalCLITaskCoordinator::as_ref(ctx).sender() else {
            return;
        };
        // 只使过期的本地父记录失活；恢复或下一用户轮由实际派发时明确登记新代。
        let expected_run = run_id.clone();
        ctx.spawn(
            async move {
                let tasks = load_tasks(&sender, false)?
                    .await
                    .map_err(|_| "父运行状态读取确认已关闭".to_owned())??;
                Ok::<_, String>(tasks.into_iter().find(|task| task.task_id == run_id))
            },
            move |_, result, ctx| {
                let Ok(Some(mut task)) = result else {
                    return;
                };
                let Some(conversation) =
                    BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
                else {
                    return;
                };
                // 异步读取回来后再次核对用户轮，旧 UI 回调不能改写已经开始的新父代。
                if conversation.run_id().as_deref() != Some(expected_run.as_str())
                    || local_parent_history_identity(conversation) != identity
                    || history_can_receive(conversation.status()) != active
                    || !task.state.is_active()
                    || recorded_history_identity(&task).is_none()
                    || (active && recorded_history_identity(&task) == identity)
                {
                    return;
                }
                let Some(sender) = LocalCLITaskCoordinator::as_ref(ctx).sender() else {
                    return;
                };
                let generation = task.generation;
                let Some(revision) = task.revision.checked_add(1) else {
                    return;
                };
                task.revision = revision;
                // UI 终态没有外部 CLI 原生完成证据，此父关联记录明确降级，不能伪造 CLI 成功。
                task.state = LocalCliTaskState::Disconnected;
                ctx.spawn(
                    async move {
                        checkpoint_task(&sender, task, Some(generation))?
                            .await
                            .map_err(|_| "父运行状态提交确认已关闭".to_owned())?
                    },
                    |_, _, _| {},
                );
            },
        );
    }

    pub(crate) fn delivery_error(&self, message_id: &str) -> Option<&str> {
        self.errors.get(message_id).map(String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalParentHistoryIdentity {
    pub root_task_id: String,
    pub user_exchange_id: String,
}

pub(crate) fn local_parent_history_identity(
    conversation: &AIConversation,
) -> Option<LocalParentHistoryIdentity> {
    let exchange = conversation
        .root_task_exchanges()
        .filter(|exchange| exchange.input.iter().any(input_starts_user_turn))
        .last()?;
    Some(LocalParentHistoryIdentity {
        root_task_id: conversation.get_root_task_id().to_string(),
        user_exchange_id: exchange.id.to_string(),
    })
}

fn input_starts_user_turn(input: &AIAgentInput) -> bool {
    matches!(
        input,
        AIAgentInput::UserQuery { .. }
            | AIAgentInput::ResumeConversation { .. }
            | AIAgentInput::CodeReview { .. }
            | AIAgentInput::InvokeSkill { .. }
            | AIAgentInput::CreateNewProject { .. }
            | AIAgentInput::CloneRepository { .. }
    )
}

pub(crate) fn recorded_history_identity(task: &LocalCliTask) -> Option<LocalParentHistoryIdentity> {
    if task.harness != "oz" {
        return None;
    }
    let config: serde_json::Value = serde_json::from_str(&task.config_json).ok()?;
    if config.get("execution_kind")?.as_str()? != "local_parent" {
        return None;
    }
    serde_json::from_value(config.get("history_identity")?.clone()).ok()
}

fn history_can_receive(status: &ConversationStatus) -> bool {
    matches!(
        status,
        ConversationStatus::InProgress
            | ConversationStatus::Blocked { .. }
            | ConversationStatus::WaitingForEvents
    )
}

fn current_result_destination(
    message: &LocalCliMessage,
    ctx: &warpui::AppContext,
) -> Option<(AIConversationId, LocalParentHistoryIdentity)> {
    let history = BlocklistAIHistoryModel::as_ref(ctx);
    let conversation_id = history.conversation_id_for_agent_id(&message.recipient_task_id)?;
    let conversation = history.conversation(&conversation_id)?;
    if conversation.run_id().as_deref() != Some(message.recipient_task_id.as_str())
        || !history_can_receive(conversation.status())
        || conversation
            .orchestration_harness()
            .is_some_and(|harness| harness != Harness::Oz)
    {
        return None;
    }
    Some((
        conversation_id,
        local_parent_history_identity(conversation)?,
    ))
}

fn history_result_message(task_id: &str, message: &LocalCliMessage) -> api::Message {
    let is_result = message.subject == "local_task_result";
    api::Message {
        id: message.message_id.clone(),
        task_id: task_id.to_owned(),
        request_id: format!(
            "local-task-{}:{}",
            if is_result { "result" } else { "message" },
            message.message_id
        ),
        server_message_data: String::new(),
        citations: Vec::new(),
        timestamp: None,
        fetched_memories: Vec::new(),
        message: Some(Message::MessagesReceivedFromAgents(
            api::message::MessagesReceivedFromAgents {
                messages: vec![
                    api::message::messages_received_from_agents::ReceivedMessage {
                        message_id: message.message_id.clone(),
                        sender_agent_id: message.sender_task_id.clone(),
                        addresses: vec![message.recipient_task_id.clone()],
                        subject: if is_result {
                            crate::t!("cli-agent-task-result-subject")
                        } else {
                            message.subject.clone()
                        },
                        message_body: message.body.clone(),
                    },
                ],
            },
        )),
    }
}

fn conversation_has_message(conversation: &AIConversation, message_id: &str) -> bool {
    conversation.all_tasks().flat_map(|task| task.messages()).any(|message| {
        matches!(&message.message, Some(Message::MessagesReceivedFromAgents(received)) if received.messages.iter().any(|message| message.message_id == message_id))
    })
}

fn status_for_task(task: &LocalCliTask) -> (ConversationStatus, Option<String>) {
    match task.state {
        LocalCliTaskState::Queued | LocalCliTaskState::Running => {
            (ConversationStatus::InProgress, None)
        }
        LocalCliTaskState::WaitingForUser => (
            ConversationStatus::Blocked {
                blocked_action: crate::t!("cli-agent-task-waiting-native-approval"),
            },
            None,
        ),
        LocalCliTaskState::Unconfirmed => (
            ConversationStatus::Blocked {
                blocked_action: crate::t!("cli-agent-task-outcome-unconfirmed"),
            },
            None,
        ),
        LocalCliTaskState::Completed
            if task.native_session_id.is_some() && task.terminal_evidence.is_some() =>
        {
            (ConversationStatus::Success, None)
        }
        LocalCliTaskState::Completed
        | LocalCliTaskState::Disconnected
        | LocalCliTaskState::Unknown => (
            ConversationStatus::Blocked {
                blocked_action: crate::t!("cli-agent-task-disconnected"),
            },
            None,
        ),
        LocalCliTaskState::Failed => (
            ConversationStatus::Error,
            Some(crate::t!("cli-agent-task-runtime-failed")),
        ),
        LocalCliTaskState::Cancelled => (ConversationStatus::Cancelled, None),
    }
}

#[cfg(test)]
#[path = "conversation_bridge_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "conversation_bridge_message_tests.rs"]
mod message_tests;
