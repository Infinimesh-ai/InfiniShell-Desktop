//! 复用托管连接派发本地消息，不读取或写入云端邮箱。

use std::collections::HashSet;
use std::time::Duration;

use ai::agent::action_result::SendMessageToAgentResult;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use warpui::{ModelContext, SingletonEntity};

use super::{ActionExecution, BlocklistAIActionExecutor, ExecuteActionInput};
#[cfg(feature = "local_fs")]
use crate::ai::agent::AIAgentActionId;
#[cfg(feature = "local_fs")]
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::{AIAgentActionResultType, AIAgentActionType};
use crate::ai::blocklist::BlocklistAIHistoryModel;
#[cfg(feature = "local_fs")]
use crate::ai::cli_agent_runtime::conversation_bridge::{
    LocalParentHistoryIdentity, local_parent_history_identity, recorded_history_identity,
};
#[cfg(feature = "local_fs")]
use crate::ai::cli_agent_runtime::coordinator::LocalCLITaskCoordinator;
#[cfg(feature = "local_fs")]
use crate::ai::local_cli_mailbox::send_local_message_if_current;
#[cfg(feature = "local_fs")]
use crate::pane_group::pane::persist_local_harness_parent;
#[cfg(feature = "local_fs")]
use crate::persistence::local_cli_tasks::{load_messages, load_tasks};
#[cfg(feature = "local_fs")]
use crate::persistence::model::{LocalCliMessage, LocalCliMessageState, LocalCliReceiptKind};

pub(super) fn execute(
    input: ExecuteActionInput<'_>,
    ctx: &mut ModelContext<BlocklistAIActionExecutor>,
) -> ActionExecution<SendMessageToAgentResult> {
    let AIAgentActionType::SendMessageToAgent {
        addresses,
        subject,
        message,
    } = &input.action.action
    else {
        return ActionExecution::InvalidAction;
    };
    let error = |error| {
        ActionExecution::Sync(AIAgentActionResultType::SendMessageToAgent(
            SendMessageToAgentResult::Error(error),
        ))
    };
    let unique: HashSet<_> = addresses.iter().collect();
    if addresses.is_empty() || addresses.len() > 16 || unique.len() != addresses.len() {
        return error(crate::t!("cli-agent-message-invalid-recipients"));
    }
    #[cfg(not(feature = "local_fs"))]
    {
        let _ = (subject, message, ctx);
        error(crate::t!("cli-agent-message-local-only"))
    }
    #[cfg(feature = "local_fs")]
    {
        let Some(sender_task_id) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&input.conversation_id)
            .and_then(|conversation| conversation.run_id())
        else {
            return error(crate::t!("cli-agent-message-local-only"));
        };
        let parent_identity = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&input.conversation_id)
            .and_then(|conversation| current_action_identity(conversation, &input.action.id));
        let Some(parent_identity) = parent_identity else {
            return error(crate::t!("cli-agent-task-parent-changed"));
        };
        let coordinator = LocalCLITaskCoordinator::as_ref(ctx);
        let Some(sender) = coordinator.sender() else {
            return error(crate::t!("cli-agent-message-local-only"));
        };
        let mut endpoints = Vec::with_capacity(addresses.len());
        for address in addresses {
            let Some(endpoint) = coordinator.endpoint(address) else {
                return error(crate::t!("cli-agent-message-recipient-unavailable"));
            };
            endpoints.push(endpoint);
        }
        let action_id = input.action.id.to_string();
        let conversation_id = input.conversation_id;
        let source_action = input.action.id.clone();
        let spawner = ctx.spawner();
        let subject = subject.clone();
        let body = message.clone();
        ActionExecution::new_async(
            async move {
                let result = async {
                    let tasks = load_tasks(&sender, false)?
                        .await
                        .map_err(|_| "本地任务读取确认通道已关闭".to_owned())??;
                    let mut source = tasks
                        .iter()
                        .find(|task| task.task_id == sender_task_id)
                        .cloned()
                        .ok_or_else(|| crate::t!("cli-agent-message-local-only"))?;
                    if recorded_history_identity(&source).is_none() {
                        return Err(crate::t!("cli-agent-task-parent-changed"));
                    }
                    let mut config: serde_json::Value =
                        serde_json::from_str(&source.config_json)
                            .map_err(|_| crate::t!("cli-agent-task-parent-changed"))?;
                    config["history_identity"] = serde_json::to_value(&parent_identity)
                        .map_err(|_| crate::t!("cli-agent-task-parent-changed"))?;
                    source.config_json = config.to_string();
                    let source = persist_local_harness_parent(&sender, source, || {
                        let expected_identity = parent_identity.clone();
                        let expected_run = sender_task_id.clone();
                        let expected_action = source_action.clone();
                        let validation = spawner.spawn(move |_, ctx| {
                            let current = BlocklistAIHistoryModel::as_ref(ctx)
                                .conversation(&conversation_id)
                                .is_some_and(|conversation| {
                                    conversation.run_id().as_ref() == Some(&expected_run)
                                        && current_action_identity(conversation, &expected_action)
                                            .as_ref()
                                            == Some(&expected_identity)
                                });
                            current
                                .then_some(())
                                .ok_or_else(|| crate::t!("cli-agent-task-parent-changed"))
                        });
                        async move {
                            validation
                                .await
                                .map_err(|_| crate::t!("cli-agent-task-parent-changed"))?
                        }
                    })
                    .await?;
                    if !source.state.is_active()
                        || recorded_history_identity(&source).as_ref() != Some(&parent_identity)
                    {
                        return Err(crate::t!("cli-agent-task-parent-changed"));
                    }
                    let batch_id = stable_message_id(&sender_task_id, &action_id, "batch");
                    let mut messages = Vec::with_capacity(endpoints.len());
                    for endpoint in endpoints {
                        let message_id =
                            stable_message_id(&sender_task_id, &action_id, &endpoint.task_id);
                        let message = LocalCliMessage {
                            version: 1,
                            message_id: message_id.to_string(),
                            sender_task_id: sender_task_id.clone(),
                            recipient_task_id: endpoint.task_id.clone(),
                            sender_generation: source.generation,
                            recipient_generation: endpoint.generation,
                            subject: subject.clone(),
                            body: body.clone(),
                            state: LocalCliMessageState::Queued,
                            receipt_kind: None,
                        };
                        let expected_identity = parent_identity.clone();
                        let expected_run = sender_task_id.clone();
                        let expected_action = source_action.clone();
                        let state = send_local_message_if_current(
                            &sender,
                            endpoint,
                            message.clone(),
                            |prepared| async {
                                spawner
                                    .spawn(move |_, ctx| {
                                        let current = BlocklistAIHistoryModel::as_ref(ctx)
                                            .conversation(&conversation_id)
                                            .is_some_and(|conversation| {
                                                conversation.run_id().as_ref()
                                                    == Some(&expected_run)
                                                    && current_action_identity(
                                                        conversation,
                                                        &expected_action,
                                                    )
                                                    .as_ref()
                                                        == Some(&expected_identity)
                                            });
                                        if !current {
                                            return Err(crate::t!("cli-agent-task-parent-changed"));
                                        }
                                        Ok(prepared.commit())
                                    })
                                    .await
                                    .map_err(|_| crate::t!("cli-agent-task-parent-changed"))?
                            },
                        )
                        .await?;
                        if matches!(
                            state,
                            LocalCliMessageState::Failed
                                | LocalCliMessageState::Cancelled
                                | LocalCliMessageState::Unknown
                        ) {
                            return Err(crate::t!("cli-agent-message-delivery-failed"));
                        }
                        messages.push(message);
                    }
                    // 等待原生回执的窗口有界；超时保留原消息 ID，绝不重发输入。
                    for attempt in 0..31 {
                        let mut all_acknowledged = true;
                        for message in &messages {
                            let stored = load_messages(
                                &sender,
                                message.recipient_task_id.clone(),
                                message.recipient_generation,
                            )?
                            .await
                            .map_err(|_| "消息读取确认通道已关闭".to_owned())??;
                            let saved = stored
                                .iter()
                                .find(|stored| stored.message_id == message.message_id)
                                .ok_or_else(|| "已提交消息记录丢失".to_owned())?;
                            match saved.state {
                                LocalCliMessageState::Acknowledged => {
                                    all_acknowledged &= saved.receipt_kind
                                        == Some(LocalCliReceiptKind::NativeProtocol);
                                }
                                LocalCliMessageState::Queued | LocalCliMessageState::Sent => {
                                    all_acknowledged = false
                                }
                                LocalCliMessageState::Failed
                                | LocalCliMessageState::Cancelled
                                | LocalCliMessageState::Unknown => {
                                    return Err(crate::t!("cli-agent-message-delivery-failed"));
                                }
                            }
                        }
                        if all_acknowledged {
                            return Ok(SendMessageToAgentResult::Acknowledged {
                                message_id: batch_id.to_string(),
                            });
                        }
                        if attempt < 30 {
                            warpui::r#async::Timer::after(Duration::from_millis(100)).await;
                        }
                    }
                    Ok(SendMessageToAgentResult::Unconfirmed {
                        message_id: batch_id.to_string(),
                    })
                }
                .await;
                result.unwrap_or_else(SendMessageToAgentResult::Error)
            },
            |result, _| AIAgentActionResultType::SendMessageToAgent(result),
        )
    }
}

#[cfg(feature = "local_fs")]
fn current_action_identity(
    conversation: &AIConversation,
    action_id: &AIAgentActionId,
) -> Option<LocalParentHistoryIdentity> {
    if !matches!(
        conversation.status(),
        ConversationStatus::InProgress
            | ConversationStatus::Blocked { .. }
            | ConversationStatus::WaitingForEvents
    ) {
        return None;
    }
    let identity = local_parent_history_identity(conversation)?;
    let action_exchange = conversation.exchange_id_for_action(action_id)?;
    // 工具结果续流可以产生新 exchange，但不能跨过下一次明确用户输入。
    conversation
        .root_task_exchanges()
        .skip_while(|exchange| exchange.id.to_string() != identity.user_exchange_id)
        .any(|exchange| exchange.id == action_exchange)
        .then_some(identity)
}

fn stable_message_id(sender_task_id: &str, action_id: &str, recipient: &str) -> Uuid {
    let mut digest = Sha256::new();
    for value in [
        "infinishell-local-message-v1",
        sender_task_id,
        action_id,
        recipient,
    ] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    let digest = digest.finalize();
    Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 至少包含 16 字节"))
}

#[cfg(test)]
#[path = "send_message_to_agent_tests.rs"]
mod tests;
