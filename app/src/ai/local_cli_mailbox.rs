//! 本地父子任务消息：提交确认后派发，只接受原生协议的接收确认。

use std::future::Future;
use std::sync::mpsc::SyncSender;

use uuid::Uuid;
use warp_cli::agent::Harness;

use crate::ai::cli_agent_runtime::coordinator::{ManagedTaskEndpoint, PreparedManagedSend};
use crate::ai::cli_agent_runtime::{InputContent, RuntimeAction, RuntimeCommand, RuntimeEventKind};
use crate::persistence::ModelEvent;
use crate::persistence::local_cli_tasks::{
    LocalCliEnqueueOutcome, acknowledge_message, claim_task_result, enqueue_message, load_messages,
};
use crate::persistence::model::{LocalCliMessage, LocalCliMessageState};

/// 普通 PTY 没有可靠的接收确认，因此此入口只接受已就绪的托管连接。
pub(crate) async fn send_local_message(
    sender: &SyncSender<ModelEvent>,
    endpoint: ManagedTaskEndpoint,
    message: LocalCliMessage,
) -> Result<LocalCliMessageState, String> {
    send_local_message_if_current(sender, endpoint, message, |prepared| async {
        Ok(prepared.commit())
    })
    .await
}

/// SQLite 已记下交付尝试后再次验证动作身份，旧父轮不能跨异步窗口发出追加指令。
pub(crate) async fn send_local_message_if_current<F, Fut>(
    sender: &SyncSender<ModelEvent>,
    endpoint: ManagedTaskEndpoint,
    message: LocalCliMessage,
    validate: F,
) -> Result<LocalCliMessageState, String>
where
    F: FnOnce(PreparedManagedSend) -> Fut,
    Fut: Future<Output = Result<futures::channel::oneshot::Receiver<Result<(), String>>, String>>,
{
    let command = message_command(&endpoint, &message)?;
    dispatch_once(sender, message, || async move {
        let prepared = endpoint.prepare_send(command).await?;
        let receiver = validate(prepared).await?;
        receiver
            .await
            .map_err(|_| crate::t!("cli-agent-status-disconnected"))?
    })
    .await
}

pub(crate) async fn send_prepared_result(
    endpoint: ManagedTaskEndpoint,
    message: LocalCliMessage,
) -> Result<(), String> {
    let command = message_command(&endpoint, &message)?;
    endpoint.send_result(command, message).await
}

fn message_command(
    endpoint: &ManagedTaskEndpoint,
    message: &LocalCliMessage,
) -> Result<RuntimeCommand, String> {
    if endpoint.task_id != message.recipient_task_id
        || endpoint.generation != message.recipient_generation
    {
        return Err("消息接收任务或运行代数不匹配".to_owned());
    }
    let message_id =
        Uuid::parse_str(&message.message_id).map_err(|_| "托管消息 ID 必须是 UUID".to_owned())?;
    let input = vec![InputContent::Text(format!(
        "Subject: {}\n\n{}",
        message.subject, message.body
    ))];
    let action = match endpoint.active_turn_id.clone() {
        Some(_) if endpoint.harness == Harness::Claude => RuntimeAction::Submit { input },
        Some(expected_turn_id) => RuntimeAction::Steer {
            expected_turn_id,
            input,
        },
        None => RuntimeAction::Submit { input },
    };
    Ok(RuntimeCommand {
        generation: endpoint.runtime_generation,
        message_id,
        action,
    })
}

async fn dispatch_once<F, Fut>(
    sender: &SyncSender<ModelEvent>,
    message: LocalCliMessage,
    dispatch: F,
) -> Result<LocalCliMessageState, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let outcome = enqueue_message(sender, message.clone())?
        .await
        .map_err(|_| "消息入队确认通道已关闭".to_owned())??;
    match outcome {
        LocalCliEnqueueOutcome::Existing(state) => return Ok(state),
        LocalCliEnqueueOutcome::Created => {}
    }
    // 先记录一次交付尝试，防止进程在发送与落盘之间退出后被重投。
    // Sent 只表示交付尝试；只有原生 MessageAccepted 才能变为 Acknowledged。
    set_message_state(sender, &message, LocalCliMessageState::Sent).await?;
    if let Err(error) = dispatch().await {
        return Err(crate::t!(
            "cli-agent-message-unconfirmed-id",
            message_id = message.message_id,
            error = error
        ));
    }
    Ok(LocalCliMessageState::Sent)
}

pub(crate) async fn dispatch_prepared_result_once<F, Fut>(
    sender: &SyncSender<ModelEvent>,
    message: LocalCliMessage,
    dispatch: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let Some(claimed) = claim_task_result(sender, message)?
        .await
        .map_err(|_| "结果派发领取确认通道已关闭".to_owned())??
    else {
        return Ok(());
    };
    dispatch().await.map_err(|error| {
        crate::t!(
            "cli-agent-message-unconfirmed-id",
            message_id = claimed.message_id,
            error = error
        )
    })
}

async fn set_message_state(
    sender: &SyncSender<ModelEvent>,
    message: &LocalCliMessage,
    state: LocalCliMessageState,
) -> Result<(), String> {
    acknowledge_message(
        sender,
        message.message_id.clone(),
        message.recipient_task_id.clone(),
        message.recipient_generation,
        state,
    )?
    .await
    .map_err(|_| "消息状态确认通道已关闭".to_owned())?
}

/// 协调器验证运行 token 后调用；普通用户输入没有邮箱记录时无需写入。
pub(crate) async fn acknowledge_runtime_message(
    sender: &SyncSender<ModelEvent>,
    task_id: &str,
    generation: i64,
    event: &RuntimeEventKind,
) -> Result<Option<LocalCliMessageState>, String> {
    let (message_id, state) = match event {
        RuntimeEventKind::MessageAccepted { message_id, .. } => {
            (message_id, LocalCliMessageState::Acknowledged)
        }
        RuntimeEventKind::RequestFailed { message_id, .. } => {
            (message_id, LocalCliMessageState::Failed)
        }
        RuntimeEventKind::SessionReady { .. }
        | RuntimeEventKind::InputJoined { .. }
        | RuntimeEventKind::CommandDispatched { .. }
        | RuntimeEventKind::TurnStarted { .. }
        | RuntimeEventKind::TextDelta { .. }
        | RuntimeEventKind::Progress { .. }
        | RuntimeEventKind::LocalToolRequested { .. }
        | RuntimeEventKind::LocalToolCancelled { .. }
        | RuntimeEventKind::ApprovalRequested { .. }
        | RuntimeEventKind::ApprovalResolved { .. }
        | RuntimeEventKind::ApprovalCancelled { .. }
        | RuntimeEventKind::TurnFinished { .. }
        | RuntimeEventKind::Disconnected { .. } => return Ok(None),
    };
    let messages = load_messages(sender, task_id.to_owned(), generation)?
        .await
        .map_err(|_| "消息读取确认通道已关闭".to_owned())??;
    let message_id = message_id.to_string();
    let Some(message) = messages
        .into_iter()
        .find(|message| message.message_id == message_id)
    else {
        return Ok(None);
    };
    set_message_state(sender, &message, state).await?;
    Ok(Some(state))
}

#[cfg(test)]
#[path = "local_cli_mailbox_tests.rs"]
mod tests;
