//! 复用托管连接派发本地消息，不读取或写入云端邮箱。

use std::collections::HashSet;
use std::time::Duration;

use ai::agent::action_result::SendMessageToAgentResult;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use warpui::{ModelContext, SingletonEntity};

use super::{ActionExecution, BlocklistAIActionExecutor, ExecuteActionInput};
use crate::ai::agent::{AIAgentActionResultType, AIAgentActionType};
use crate::ai::blocklist::BlocklistAIHistoryModel;
#[cfg(feature = "local_fs")]
use crate::ai::cli_agent_runtime::coordinator::LocalCLITaskCoordinator;
#[cfg(feature = "local_fs")]
use crate::ai::local_cli_mailbox::send_local_message;
#[cfg(feature = "local_fs")]
use crate::persistence::local_cli_tasks::{load_messages, load_tasks};
#[cfg(feature = "local_fs")]
use crate::persistence::model::{LocalCliMessage, LocalCliMessageState};

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
        let subject = subject.clone();
        let body = message.clone();
        ActionExecution::new_async(
            async move {
                let result = async {
                    let tasks = load_tasks(&sender, false)?
                        .await
                        .map_err(|_| "本地任务读取确认通道已关闭".to_owned())??;
                    let source = tasks
                        .iter()
                        .find(|task| task.task_id == sender_task_id)
                        .ok_or_else(|| crate::t!("cli-agent-message-local-only"))?;
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
                        let state = send_local_message(&sender, endpoint, message.clone()).await?;
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
                            let state = stored
                                .iter()
                                .find(|stored| stored.message_id == message.message_id)
                                .map(|message| message.state)
                                .ok_or_else(|| "已提交消息记录丢失".to_owned())?;
                            match state {
                                LocalCliMessageState::Acknowledged => {}
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
