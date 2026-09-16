//! 本地 CLI 任务和信箱复用应用 SQLite 写入线程；确认只在事务提交后返回。

use std::sync::mpsc::SyncSender;

use anyhow::{Context, Result, bail};
use diesel::prelude::*;
use futures::channel::oneshot;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::ModelEvent;
pub(crate) use super::model::{
    LocalCliMessage, LocalCliMessageState, LocalCliReceiptKind, LocalCliTask, LocalCliTaskState,
};
use super::schema::{local_cli_messages, local_cli_task_generations, local_cli_tasks};

const TASK_RESULT_SUBJECT: &str = "local_task_result";

pub(crate) type CommitReceiver = oneshot::Receiver<Result<(), String>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalCliEnqueueOutcome {
    Created,
    Existing(LocalCliMessageState),
}

#[derive(Debug)]
pub enum LocalCliPersistenceRequest {
    CheckpointTask {
        task: LocalCliTask,
        expected_generation: Option<i64>,
        completion: oneshot::Sender<Result<(), String>>,
    },
    CheckpointTaskWithMessage {
        task: LocalCliTask,
        expected_generation: Option<i64>,
        message: LocalCliMessage,
        completion: oneshot::Sender<Result<LocalCliEnqueueOutcome, String>>,
    },
    EnqueueTaskResult {
        task_id: String,
        generation: i64,
        completion: oneshot::Sender<Result<Option<LocalCliMessage>, String>>,
    },
    ClaimTaskResult {
        message: LocalCliMessage,
        completion: oneshot::Sender<Result<Option<LocalCliMessage>, String>>,
    },
    ClaimParentHistoryMessage {
        message: LocalCliMessage,
        completion: oneshot::Sender<Result<Option<LocalCliMessage>, String>>,
    },
    EnqueueMessage {
        message: LocalCliMessage,
        completion: oneshot::Sender<Result<LocalCliEnqueueOutcome, String>>,
    },
    AcknowledgeMessage {
        message_id: String,
        task_id: String,
        generation: i64,
        state: LocalCliMessageState,
        completion: oneshot::Sender<Result<(), String>>,
    },
    AcknowledgeApplicationHistory {
        message_id: String,
        task_id: String,
        generation: i64,
        completion: oneshot::Sender<Result<(), String>>,
    },
    LoadTasks {
        recover_interrupted: bool,
        completion: oneshot::Sender<Result<Vec<LocalCliTask>, String>>,
    },
    LoadTaskGenerations {
        task_id: String,
        completion: oneshot::Sender<Result<Vec<LocalCliTask>, String>>,
    },
    LoadMessages {
        task_id: String,
        generation: i64,
        completion: oneshot::Sender<Result<Vec<LocalCliMessage>, String>>,
    },
    LoadTaskMessages {
        task_id: String,
        completion: oneshot::Sender<Result<Vec<LocalCliMessage>, String>>,
    },
}

/// 初建任务用 None；更新需给出已读取的 generation，revision 必须恰好递增一次。
pub(crate) fn checkpoint_task(
    sender: &SyncSender<ModelEvent>,
    task: LocalCliTask,
    expected_generation: Option<i64>,
) -> Result<CommitReceiver, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::CheckpointTask {
            task,
            expected_generation,
            completion,
        },
    )?;
    Ok(receiver)
}

pub(crate) fn enqueue_message(
    sender: &SyncSender<ModelEvent>,
    message: LocalCliMessage,
) -> Result<oneshot::Receiver<Result<LocalCliEnqueueOutcome, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::EnqueueMessage {
            message,
            completion,
        },
    )?;
    Ok(receiver)
}

/// 新运行与首条输入在同一事务提交，入队失败不能清空上一代结果或推进代数。
pub(crate) fn checkpoint_task_with_message(
    sender: &SyncSender<ModelEvent>,
    task: LocalCliTask,
    expected_generation: Option<i64>,
    message: LocalCliMessage,
) -> Result<oneshot::Receiver<Result<LocalCliEnqueueOutcome, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::CheckpointTaskWithMessage {
            task,
            expected_generation,
            message,
            completion,
        },
    )?;
    Ok(receiver)
}

/// 从已提交的真实终态构造唯一结果消息；无父任务时返回 None。
pub(crate) fn enqueue_task_result(
    sender: &SyncSender<ModelEvent>,
    task_id: String,
    generation: i64,
) -> Result<oneshot::Receiver<Result<Option<LocalCliMessage>, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::EnqueueTaskResult {
            task_id,
            generation,
            completion,
        },
    )?;
    Ok(receiver)
}

/// 只领取已提交且未派发的真实结果；同一结果最多一个调用者得到 Some。
pub(crate) fn claim_task_result(
    sender: &SyncSender<ModelEvent>,
    message: LocalCliMessage,
) -> Result<oneshot::Receiver<Result<Option<LocalCliMessage>, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::ClaimTaskResult {
            message,
            completion,
        },
    )?;
    Ok(receiver)
}

/// 普通子任务消息只允许一个历史投递者领取；Sent 不会重新入队。
pub(crate) fn claim_parent_history_message(
    sender: &SyncSender<ModelEvent>,
    message: LocalCliMessage,
) -> Result<oneshot::Receiver<Result<Option<LocalCliMessage>, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::ClaimParentHistoryMessage {
            message,
            completion,
        },
    )?;
    Ok(receiver)
}

/// 传输写入只标为 Sent；仅接收方的真实回执可标为 Acknowledged。
pub(crate) fn acknowledge_message(
    sender: &SyncSender<ModelEvent>,
    message_id: String,
    task_id: String,
    generation: i64,
    state: LocalCliMessageState,
) -> Result<CommitReceiver, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::AcknowledgeMessage {
            message_id,
            task_id,
            generation,
            state,
            completion,
        },
    )?;
    Ok(receiver)
}

/// 仅用于父 Oz 会话历史已提交的结果或授权消息，不代表 CLI 接收或模型已处理。
pub(crate) fn acknowledge_application_history(
    sender: &SyncSender<ModelEvent>,
    message_id: String,
    task_id: String,
    generation: i64,
) -> Result<CommitReceiver, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::AcknowledgeApplicationHistory {
            message_id,
            task_id,
            generation,
            completion,
        },
    )?;
    Ok(receiver)
}

/// 应用启动时传 true：旧活动记录降级为 Disconnected，不启动进程或重新发送消息。
pub(crate) fn load_tasks(
    sender: &SyncSender<ModelEvent>,
    recover_interrupted: bool,
) -> Result<oneshot::Receiver<Result<Vec<LocalCliTask>, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::LoadTasks {
            recover_interrupted,
            completion,
        },
    )?;
    Ok(receiver)
}

pub(crate) fn load_messages(
    sender: &SyncSender<ModelEvent>,
    task_id: String,
    generation: i64,
) -> Result<oneshot::Receiver<Result<Vec<LocalCliMessage>, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::LoadMessages {
            task_id,
            generation,
            completion,
        },
    )?;
    Ok(receiver)
}

/// 人类查看任务双向消息与所有历史代次；不参与运行中的邮箱投递。
pub(crate) fn load_task_messages(
    sender: &SyncSender<ModelEvent>,
    task_id: String,
) -> Result<oneshot::Receiver<Result<Vec<LocalCliMessage>, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::LoadTaskMessages {
            task_id,
            completion,
        },
    )?;
    Ok(receiver)
}

/// 继续任务不会覆盖上一轮已经回收的结果和终态证据。
pub(crate) fn load_task_generations(
    sender: &SyncSender<ModelEvent>,
    task_id: String,
) -> Result<oneshot::Receiver<Result<Vec<LocalCliTask>, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::LoadTaskGenerations {
            task_id,
            completion,
        },
    )?;
    Ok(receiver)
}

fn enqueue_request(
    sender: &SyncSender<ModelEvent>,
    request: LocalCliPersistenceRequest,
) -> Result<(), String> {
    // UI 线程不能等待写入队列腾出空间；失败时由调用方停止派发并保留草稿。
    sender
        .try_send(ModelEvent::LocalCliPersistence(request))
        .map_err(|_| "本地任务写入队列不可用".to_owned())
}

pub(super) fn handle_request(
    request: LocalCliPersistenceRequest,
    connection: &mut SqliteConnection,
) -> Result<()> {
    match request {
        LocalCliPersistenceRequest::CheckpointTask {
            task,
            expected_generation,
            completion,
        } => complete(
            checkpoint(connection, task, expected_generation),
            completion,
        ),
        LocalCliPersistenceRequest::EnqueueMessage {
            message,
            completion,
        } => complete(insert_message(connection, message), completion),
        LocalCliPersistenceRequest::CheckpointTaskWithMessage {
            task,
            expected_generation,
            message,
            completion,
        } => complete(
            checkpoint_with_message(connection, task, expected_generation, message),
            completion,
        ),
        LocalCliPersistenceRequest::EnqueueTaskResult {
            task_id,
            generation,
            completion,
        } => complete(
            insert_task_result(connection, &task_id, generation),
            completion,
        ),
        LocalCliPersistenceRequest::ClaimTaskResult {
            message,
            completion,
        } => complete(claim_result(connection, message), completion),
        LocalCliPersistenceRequest::ClaimParentHistoryMessage {
            message,
            completion,
        } => complete(claim_parent_message(connection, message), completion),
        LocalCliPersistenceRequest::AcknowledgeMessage {
            message_id,
            task_id,
            generation,
            state,
            completion,
        } => complete(
            update_message_state(connection, &message_id, &task_id, generation, state),
            completion,
        ),
        LocalCliPersistenceRequest::AcknowledgeApplicationHistory {
            message_id,
            task_id,
            generation,
            completion,
        } => complete(
            update_message_state_with_receipt(
                connection,
                &message_id,
                &task_id,
                generation,
                LocalCliMessageState::Acknowledged,
                Some(LocalCliReceiptKind::ApplicationHistory),
            ),
            completion,
        ),
        LocalCliPersistenceRequest::LoadTasks {
            recover_interrupted,
            completion,
        } => complete(read_tasks(connection, recover_interrupted), completion),
        LocalCliPersistenceRequest::LoadTaskGenerations {
            task_id,
            completion,
        } => complete(read_task_generations(connection, &task_id), completion),
        LocalCliPersistenceRequest::LoadMessages {
            task_id,
            generation,
            completion,
        } => complete(read_messages(connection, &task_id, generation), completion),
        LocalCliPersistenceRequest::LoadTaskMessages {
            task_id,
            completion,
        } => complete(read_task_messages(connection, &task_id), completion),
    }
}

pub(super) fn reject_request(request: LocalCliPersistenceRequest) {
    let error = "SQLite 写入器已暂停".to_owned();
    match request {
        LocalCliPersistenceRequest::CheckpointTask { completion, .. }
        | LocalCliPersistenceRequest::AcknowledgeApplicationHistory { completion, .. }
        | LocalCliPersistenceRequest::AcknowledgeMessage { completion, .. } => {
            let _ = completion.send(Err(error));
        }
        LocalCliPersistenceRequest::EnqueueTaskResult { completion, .. }
        | LocalCliPersistenceRequest::ClaimParentHistoryMessage { completion, .. }
        | LocalCliPersistenceRequest::ClaimTaskResult { completion, .. } => {
            let _ = completion.send(Err(error));
        }
        LocalCliPersistenceRequest::EnqueueMessage { completion, .. }
        | LocalCliPersistenceRequest::CheckpointTaskWithMessage { completion, .. } => {
            let _ = completion.send(Err(error));
        }
        LocalCliPersistenceRequest::LoadTasks { completion, .. }
        | LocalCliPersistenceRequest::LoadTaskGenerations { completion, .. } => {
            let _ = completion.send(Err(error));
        }
        LocalCliPersistenceRequest::LoadMessages { completion, .. }
        | LocalCliPersistenceRequest::LoadTaskMessages { completion, .. } => {
            let _ = completion.send(Err(error));
        }
    }
}

fn complete<T>(result: Result<T>, completion: oneshot::Sender<Result<T, String>>) -> Result<()> {
    match result {
        Ok(value) => {
            let _ = completion.send(Ok(value));
            Ok(())
        }
        Err(error) => {
            let _ = completion.send(Err(error.to_string()));
            Err(error)
        }
    }
}

fn state_name<T: serde::Serialize>(state: T) -> Result<String> {
    serde_json::to_value(state)?
        .as_str()
        .map(str::to_owned)
        .context("本地任务状态不是字符串")
}

fn read_task(connection: &mut SqliteConnection, task_id: &str) -> Result<Option<LocalCliTask>> {
    local_cli_tasks::table
        .find(task_id)
        .select(local_cli_tasks::data)
        .first::<String>(connection)
        .optional()?
        .map(|data| serde_json::from_str(&data).context("本地任务记录无法解析"))
        .transpose()
}

fn validate_task(task: &LocalCliTask) -> Result<()> {
    if task.version != 1
        || task.generation < 1
        || task.revision < 0
        || task.task_id.trim().is_empty()
        || task.harness.trim().is_empty()
        || task.working_directory.is_empty()
        || task.state == LocalCliTaskState::Unknown
        || task.parent_task_id.as_deref() == Some(task.task_id.as_str())
        || task.parent_task_id.is_some() != task.parent_generation.is_some()
        || task
            .parent_generation
            .is_some_and(|generation| generation < 1)
        || task
            .native_session_id
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
    {
        bail!("本地任务记录字段无效或版本不兼容");
    }
    if task.config_json.len() > 1024 * 1024
        || task
            .result
            .as_ref()
            .is_some_and(|result| result.len() > 10 * 1024 * 1024)
    {
        bail!("本地任务记录超过持久化大小限制");
    }
    let config: serde_json::Value = serde_json::from_str(&task.config_json)?;
    if !config.is_object() {
        bail!("本地任务配置必须是 JSON 对象");
    }
    if task.state == LocalCliTaskState::Completed
        && (task.native_session_id.is_none()
            || task
                .terminal_evidence
                .as_deref()
                .is_none_or(|evidence| evidence.trim().is_empty()))
    {
        bail!("缺少原生会话或可信完成证据，不能将本地任务记为完成");
    }
    Ok(())
}

fn checkpoint(
    connection: &mut SqliteConnection,
    task: LocalCliTask,
    expected_generation: Option<i64>,
) -> Result<()> {
    validate_task(&task)?;
    connection.transaction(|connection| {
        let previous = read_task(connection, &task.task_id)?;
        if let Some(previous) = previous.as_ref() {
            if previous.version != 1
                || expected_generation != Some(previous.generation)
                || task.harness != previous.harness
                || task.parent_task_id != previous.parent_task_id
                || task.parent_generation != previous.parent_generation
            {
                bail!("本地任务已由其他运行或更新取代");
            }
            if previous == &task && task.state != LocalCliTaskState::Queued {
                return Ok(());
            }
            if task.generation == previous.generation {
                if previous.revision.checked_add(1) != Some(task.revision)
                    || (previous.state.is_terminal() && task.state != previous.state)
                    || (previous.native_session_id.is_some()
                        && task.native_session_id != previous.native_session_id)
                    || (previous.state != LocalCliTaskState::Queued
                        && task.state == LocalCliTaskState::Queued)
                    || (previous.result.is_some() && task.result.is_none())
                    || (previous.terminal_evidence.is_some()
                        && task.terminal_evidence != previous.terminal_evidence)
                {
                    bail!("本地任务更新过时或状态回退");
                }
            } else if previous.generation.checked_add(1) != Some(task.generation)
                || previous.state.is_active()
                || previous.state == LocalCliTaskState::Unknown
                || task.revision != 0
                || task.state != LocalCliTaskState::Queued
                || task.result.is_some()
                || task.terminal_evidence.is_some()
            {
                bail!("活动任务不能重复继续，或新运行的起始状态无效");
            }
        } else if expected_generation.is_some()
            || task.generation != 1
            || task.revision != 0
            || task.state != LocalCliTaskState::Queued
            || task.result.is_some()
            || task.terminal_evidence.is_some()
        {
            bail!("本地任务初始检查点无效");
        }
        if let Some(parent_id) = task.parent_task_id.as_ref() {
            let parent = read_task(connection, parent_id)?.context("本地父任务尚未持久化")?;
            let generation = task.parent_generation.context("缺少创建时的父运行标识")?;
            if previous.is_none()
                && (parent.version != 1
                    || parent.generation != generation
                    || !parent.state.is_active())
            {
                bail!("创建子任务时的父运行已过时或结束");
            }
            if read_task_generation(connection, parent_id, generation)?.is_none() {
                bail!("本地父运行记录不存在");
            }
        }
        write_task(connection, &task)?;
        let is_new_generation = previous
            .as_ref()
            .is_some_and(|old| old.generation != task.generation);
        if task.state.is_terminal() || is_new_generation {
            cancel_pending_messages(
                connection,
                &task.task_id,
                is_new_generation || task.state == LocalCliTaskState::Cancelled,
            )?;
        }
        Ok(())
    })
}

fn checkpoint_with_message(
    connection: &mut SqliteConnection,
    task: LocalCliTask,
    expected_generation: Option<i64>,
    message: LocalCliMessage,
) -> Result<LocalCliEnqueueOutcome> {
    if message.recipient_task_id != task.task_id || message.recipient_generation != task.generation
    {
        bail!("首条输入必须属于同一事务创建的运行");
    }
    connection.transaction(|connection| {
        checkpoint(connection, task, expected_generation)?;
        insert_message(connection, message)
    })
}

fn write_task(connection: &mut SqliteConnection, task: &LocalCliTask) -> Result<()> {
    let values = (
        local_cli_tasks::task_id.eq(&task.task_id),
        local_cli_tasks::parent_task_id.eq(&task.parent_task_id),
        local_cli_tasks::harness.eq(&task.harness),
        local_cli_tasks::native_session_id.eq(&task.native_session_id),
        local_cli_tasks::generation.eq(task.generation),
        local_cli_tasks::revision.eq(task.revision),
        local_cli_tasks::state.eq(state_name(task.state)?),
        local_cli_tasks::data.eq(serde_json::to_string(task)?),
    );
    diesel::insert_into(local_cli_tasks::table)
        .values(values.clone())
        .on_conflict(local_cli_tasks::task_id)
        .do_update()
        .set(values)
        .execute(connection)?;
    let generation_values = (
        local_cli_task_generations::task_id.eq(&task.task_id),
        local_cli_task_generations::generation.eq(task.generation),
        local_cli_task_generations::data.eq(serde_json::to_string(task)?),
    );
    diesel::insert_into(local_cli_task_generations::table)
        .values(generation_values.clone())
        .on_conflict((
            local_cli_task_generations::task_id,
            local_cli_task_generations::generation,
        ))
        .do_update()
        .set(generation_values)
        .execute(connection)?;
    Ok(())
}

fn read_task_generations(
    connection: &mut SqliteConnection,
    task_id: &str,
) -> Result<Vec<LocalCliTask>> {
    local_cli_task_generations::table
        .filter(local_cli_task_generations::task_id.eq(task_id))
        .order(local_cli_task_generations::generation.asc())
        .select(local_cli_task_generations::data)
        .load::<String>(connection)?
        .into_iter()
        .map(|row| serde_json::from_str(&row).context("本地任务历史无法解析"))
        .collect()
}

fn read_task_generation(
    connection: &mut SqliteConnection,
    task_id: &str,
    generation: i64,
) -> Result<Option<LocalCliTask>> {
    local_cli_task_generations::table
        .find((task_id, generation))
        .select(local_cli_task_generations::data)
        .first::<String>(connection)
        .optional()?
        .map(|row| serde_json::from_str(&row).context("本地父运行历史无法解析"))
        .transpose()
}

fn read_tasks(
    connection: &mut SqliteConnection,
    recover_interrupted: bool,
) -> Result<Vec<LocalCliTask>> {
    connection.transaction(|connection| {
        let rows = local_cli_tasks::table
            .order(local_cli_tasks::task_id.asc())
            .select(local_cli_tasks::data)
            .load::<String>(connection)?;
        let mut tasks = rows
            .into_iter()
            .map(|row| serde_json::from_str::<LocalCliTask>(&row))
            .collect::<Result<Vec<_>, _>>()?;
        if recover_interrupted {
            for task in &mut tasks {
                if task.version == 1
                    && task.state.is_active()
                    && task.parent_task_id.is_some() == task.parent_generation.is_some()
                {
                    task.state = LocalCliTaskState::Disconnected;
                    task.revision = task
                        .revision
                        .checked_add(1)
                        .context("本地任务 revision 溢出")?;
                    write_task(connection, task)?;
                }
            }
        }
        Ok(tasks)
    })
}

fn read_messages(
    connection: &mut SqliteConnection,
    task_id: &str,
    generation: i64,
) -> Result<Vec<LocalCliMessage>> {
    let rows = local_cli_messages::table
        .filter(local_cli_messages::recipient_task_id.eq(task_id))
        .filter(local_cli_messages::recipient_generation.eq(generation))
        .order(local_cli_messages::sequence.asc())
        .select(local_cli_messages::data)
        .load::<String>(connection)?;
    rows.into_iter()
        .map(|row| serde_json::from_str(&row).context("本地消息记录无法解析"))
        .collect()
}

fn read_task_messages(
    connection: &mut SqliteConnection,
    task_id: &str,
) -> Result<Vec<LocalCliMessage>> {
    let rows = local_cli_messages::table
        .filter(
            local_cli_messages::recipient_task_id
                .eq(task_id)
                .or(local_cli_messages::sender_task_id.eq(task_id)),
        )
        .order(local_cli_messages::sequence.asc())
        .select(local_cli_messages::data)
        .load::<String>(connection)?;
    rows.into_iter()
        .map(|row| serde_json::from_str(&row).context("本地消息记录无法解析"))
        .collect()
}

fn insert_message(
    connection: &mut SqliteConnection,
    message: LocalCliMessage,
) -> Result<LocalCliEnqueueOutcome> {
    if message.subject == TASK_RESULT_SUBJECT {
        bail!("结果消息只能从已提交任务记录生成");
    }
    insert_message_with_origin(connection, message, false)
}

fn insert_message_with_origin(
    connection: &mut SqliteConnection,
    message: LocalCliMessage,
    is_task_result: bool,
) -> Result<LocalCliEnqueueOutcome> {
    if message.version != 1
        || message.message_id.trim().is_empty()
        || message.state != LocalCliMessageState::Queued
        || message.receipt_kind.is_some()
        || message.body.len() > 1024 * 1024
        || message.subject.len() > 4096
    {
        bail!("本地消息字段无效或版本不兼容");
    }
    connection.transaction(|connection| {
        if let Some(existing) = read_message(connection, &message.message_id)? {
            let state = existing.state;
            let mut original = existing;
            original.state = LocalCliMessageState::Queued;
            original.receipt_kind = None;
            if original == message {
                return Ok(LocalCliEnqueueOutcome::Existing(state));
            }
            bail!("相同消息 ID 不能用于不同内容或运行");
        }
        let sender =
            read_task(connection, &message.sender_task_id)?.context("消息发送任务不存在")?;
        let recipient = if is_task_result {
            read_task_generation(
                connection,
                &message.recipient_task_id,
                message.recipient_generation,
            )?
        } else {
            read_task(connection, &message.recipient_task_id)?
        }
        .context("消息接收运行不存在")?;
        if sender.version != 1
            || recipient.version != 1
            || sender.generation != message.sender_generation
            || recipient.generation != message.recipient_generation
            || (sender.state == LocalCliTaskState::Cancelled && !is_task_result)
            || sender.state == LocalCliTaskState::Unknown
            || (recipient.state.is_terminal() && !is_task_result)
            || (recipient.state == LocalCliTaskState::Unknown && !is_task_result)
            || sender.parent_task_id.is_some() != sender.parent_generation.is_some()
            || (recipient.parent_task_id.is_some() != recipient.parent_generation.is_some()
                && !is_task_result)
        {
            bail!("消息对应的任务已结束或运行已过时");
        }
        if sender.task_id != recipient.task_id
            && sender.parent_task_id.as_deref() != Some(recipient.task_id.as_str())
            && recipient.parent_task_id.as_deref() != Some(sender.task_id.as_str())
        {
            bail!("本地消息只能投递到同一任务或已记录的父子任务");
        }
        diesel::insert_into(local_cli_messages::table)
            .values((
                local_cli_messages::message_id.eq(&message.message_id),
                local_cli_messages::sender_task_id.eq(&message.sender_task_id),
                local_cli_messages::recipient_task_id.eq(&message.recipient_task_id),
                local_cli_messages::sender_generation.eq(message.sender_generation),
                local_cli_messages::recipient_generation.eq(message.recipient_generation),
                local_cli_messages::state.eq(state_name(message.state)?),
                local_cli_messages::data.eq(serde_json::to_string(&message)?),
            ))
            .execute(connection)?;
        Ok(LocalCliEnqueueOutcome::Created)
    })
}

fn insert_task_result(
    connection: &mut SqliteConnection,
    task_id: &str,
    generation: i64,
) -> Result<Option<LocalCliMessage>> {
    connection.transaction(|connection| {
        let task = read_task(connection, task_id)?.context("结果任务不存在")?;
        if task.version != 1 || task.generation != generation || !task.state.is_terminal() {
            bail!("只能回收当前已提交的真实终态");
        }
        let Some(parent_id) = task.parent_task_id.as_ref() else {
            return Ok(None);
        };
        let parent_generation = task
            .parent_generation
            .context("结果缺少创建时的父运行标识")?;
        let parent = read_task_generation(connection, parent_id, parent_generation)?
            .context("结果接收父运行不存在")?;
        let digest =
            Sha256::digest(format!("infinishell-result:{task_id}:{generation}").as_bytes());
        let message_id =
            Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 至少包含 16 字节"))
                .to_string();
        if let Some(message) = read_message(connection, &message_id)? {
            return Ok(Some(message));
        }
        let body = task_result_message_body(&task);
        let message = LocalCliMessage {
            version: 1,
            message_id,
            sender_task_id: task.task_id,
            recipient_task_id: parent.task_id,
            sender_generation: generation,
            recipient_generation: parent.generation,
            subject: TASK_RESULT_SUBJECT.to_owned(),
            body,
            state: LocalCliMessageState::Queued,
            receipt_kind: None,
        };
        insert_message_with_origin(connection, message.clone(), true)?;
        Ok(Some(message))
    })
}

fn task_result_message_body(task: &LocalCliTask) -> String {
    // JSON 转义最多将一个字节扩展为六字节，保守限制摘录后仍低于消息上限。
    // 完整原文保留在任务及每代记录中，接收者可按定位字段读取。
    let result = task
        .result
        .as_deref()
        .map(|result| utf8_excerpt(result, 128 * 1024));
    let evidence = task
        .terminal_evidence
        .as_deref()
        .map(|evidence| utf8_excerpt(evidence, 4096));
    serde_json::json!({
        "task_id": task.task_id,
        "generation": task.generation,
        "state": task.state,
        "result": result,
        "result_bytes": task.result.as_ref().map(String::len),
        "truncated": result != task.result.as_deref(),
        "terminal_evidence": evidence,
        "evidence_truncated": evidence != task.terminal_evidence.as_deref(),
    })
    .to_string()
}

fn claim_result(
    connection: &mut SqliteConnection,
    message: LocalCliMessage,
) -> Result<Option<LocalCliMessage>> {
    connection.transaction(|connection| {
        if message.subject != TASK_RESULT_SUBJECT || message.state != LocalCliMessageState::Queued {
            bail!("只能领取已生成的待派发任务结果");
        }
        let mut stored =
            read_message(connection, &message.message_id)?.context("任务结果尚未持久化")?;
        let state = stored.state;
        stored.state = LocalCliMessageState::Queued;
        stored.receipt_kind = None;
        if stored != message {
            bail!("结果消息内容与持久化记录不一致");
        }
        if state != LocalCliMessageState::Queued {
            return Ok(None);
        }
        let recipient =
            read_task(connection, &message.recipient_task_id)?.context("结果接收任务不存在")?;
        if recipient.generation != message.recipient_generation || !recipient.state.is_active() {
            bail!("结果接收运行已结束、断开或被继续操作取代");
        }
        update_message_state(
            connection,
            &message.message_id,
            &message.recipient_task_id,
            message.recipient_generation,
            LocalCliMessageState::Sent,
        )?;
        stored.state = LocalCliMessageState::Sent;
        Ok(Some(stored))
    })
}

fn validate_parent_message(
    message: &LocalCliMessage,
    source: &LocalCliTask,
    parent: &LocalCliTask,
) -> Result<()> {
    let permissions: serde_json::Value = serde_json::from_str(&source.config_json)?;
    let parent_config: serde_json::Value = serde_json::from_str(&parent.config_json)?;
    if message.subject == TASK_RESULT_SUBJECT
        || matches!(
            message.subject.as_str(),
            "native_tool_call" | "native_tool_result"
        )
        || source.version != 1
        || parent.version != 1
        || source.generation != message.sender_generation
        || parent.generation != message.recipient_generation
        || source.parent_task_id.as_deref() != Some(parent.task_id.as_str())
        || source.parent_generation != Some(parent.generation)
        || parent.harness != "oz"
        || !parent.state.is_active()
        || parent_config["execution_kind"] != "local_parent"
        || !parent_config["history_identity"].is_object()
        || permissions["local_tools"]["allow_message"] != true
    {
        bail!("普通消息未获授权或不属于创建时的活动父运行");
    }
    Ok(())
}

fn claim_parent_message(
    connection: &mut SqliteConnection,
    message: LocalCliMessage,
) -> Result<Option<LocalCliMessage>> {
    connection.transaction(|connection| {
        if message.state != LocalCliMessageState::Queued {
            bail!("只能领取待派发的普通父消息");
        }
        let mut stored =
            read_message(connection, &message.message_id)?.context("普通父消息尚未入队")?;
        let state = stored.state;
        stored.state = LocalCliMessageState::Queued;
        stored.receipt_kind = None;
        if stored != message {
            bail!("普通父消息内容与持久化记录不一致");
        }
        if state != LocalCliMessageState::Queued {
            return Ok(None);
        }
        let source = read_task(connection, &message.sender_task_id)?.context("消息子任务不存在")?;
        let parent =
            read_task(connection, &message.recipient_task_id)?.context("消息父任务不存在")?;
        validate_parent_message(&message, &source, &parent)?;
        if !source.state.is_active() {
            bail!("消息子任务已停止，不能开始新的历史投递");
        }
        update_message_state(
            connection,
            &message.message_id,
            &message.recipient_task_id,
            message.recipient_generation,
            LocalCliMessageState::Sent,
        )?;
        stored.state = LocalCliMessageState::Sent;
        Ok(Some(stored))
    })
}

fn utf8_excerpt(text: &str, maximum_bytes: usize) -> &str {
    let mut end = text.len().min(maximum_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn read_message(
    connection: &mut SqliteConnection,
    message_id: &str,
) -> Result<Option<LocalCliMessage>> {
    local_cli_messages::table
        .filter(local_cli_messages::message_id.eq(message_id))
        .select(local_cli_messages::data)
        .first::<String>(connection)
        .optional()?
        .map(|row| serde_json::from_str(&row).context("本地消息记录无法解析"))
        .transpose()
}

fn update_message_state(
    connection: &mut SqliteConnection,
    message_id: &str,
    task_id: &str,
    generation: i64,
    state: LocalCliMessageState,
) -> Result<()> {
    let receipt = (state == LocalCliMessageState::Acknowledged)
        .then_some(LocalCliReceiptKind::NativeProtocol);
    update_message_state_with_receipt(connection, message_id, task_id, generation, state, receipt)
}

fn update_message_state_with_receipt(
    connection: &mut SqliteConnection,
    message_id: &str,
    task_id: &str,
    generation: i64,
    state: LocalCliMessageState,
    receipt: Option<LocalCliReceiptKind>,
) -> Result<()> {
    connection.transaction(|connection| {
        let mut message = read_message(connection, message_id)?.context("本地消息不存在")?;
        let recipient = read_task(connection, task_id)?.context("本地任务不存在")?;
        let is_native_ack = state == LocalCliMessageState::Acknowledged
            && receipt == Some(LocalCliReceiptKind::NativeProtocol);
        let is_delivery_outcome = is_native_ack || state == LocalCliMessageState::Failed;
        // 已派发消息的发送方可能先进入下一轮，回执仍属于原来的发送记录。
        // 接收方必须保持当前 generation，不能让旧连接写入新运行。
        let sender = if is_delivery_outcome {
            read_task_generation(
                connection,
                &message.sender_task_id,
                message.sender_generation,
            )?
        } else {
            read_task(connection, &message.sender_task_id)?
        }
        .context("消息发送运行不存在")?;
        if message.version != 1
            || recipient.version != 1
            || sender.version != 1
            || sender.parent_task_id.is_some() != sender.parent_generation.is_some()
            || recipient.parent_task_id.is_some() != recipient.parent_generation.is_some()
            || message.recipient_task_id != task_id
            || message.recipient_generation != generation
            || recipient.generation != generation
            || (sender.generation != message.sender_generation
                && message.subject != TASK_RESULT_SUBJECT)
        {
            bail!("本地消息回执属于过时或不同的运行");
        }
        if message.state == state {
            if receipt.is_some() && message.receipt_kind != receipt {
                bail!("已有消息回执来源不匹配");
            }
            return Ok(());
        }
        if receipt == Some(LocalCliReceiptKind::ApplicationHistory) {
            if recipient.harness != "oz" {
                bail!("应用历史只能确认 Oz 父会话消息");
            }
            if message.subject != TASK_RESULT_SUBJECT {
                validate_parent_message(&message, &sender, &recipient)?;
                if message.state != LocalCliMessageState::Sent {
                    bail!("普通父消息必须先领取再确认历史提交");
                }
            }
        }
        let is_late_delivery_outcome =
            is_delivery_outcome && message.state == LocalCliMessageState::Sent;
        if (recipient.state.is_terminal() && !is_late_delivery_outcome)
            || recipient.state == LocalCliTaskState::Unknown
        {
            bail!("已结束任务不能确认新消息");
        }
        let allowed = match message.state {
            LocalCliMessageState::Queued => matches!(
                state,
                LocalCliMessageState::Sent
                    | LocalCliMessageState::Acknowledged
                    | LocalCliMessageState::Failed
                    | LocalCliMessageState::Cancelled
            ),
            LocalCliMessageState::Sent => matches!(
                state,
                LocalCliMessageState::Acknowledged
                    | LocalCliMessageState::Failed
                    | LocalCliMessageState::Cancelled
            ),
            LocalCliMessageState::Acknowledged
            | LocalCliMessageState::Failed
            | LocalCliMessageState::Cancelled
            | LocalCliMessageState::Unknown => false,
        };
        if !allowed {
            bail!("本地消息状态不能回退");
        }
        message.state = state;
        message.receipt_kind = receipt;
        write_message_state(connection, &message)
    })
}

fn write_message_state(connection: &mut SqliteConnection, message: &LocalCliMessage) -> Result<()> {
    diesel::update(
        local_cli_messages::table.filter(local_cli_messages::message_id.eq(&message.message_id)),
    )
    .set((
        local_cli_messages::state.eq(state_name(message.state)?),
        local_cli_messages::data.eq(serde_json::to_string(message)?),
    ))
    .execute(connection)?;
    Ok(())
}

fn cancel_pending_messages(
    connection: &mut SqliteConnection,
    task_id: &str,
    cancel_outgoing: bool,
) -> Result<()> {
    let query = local_cli_messages::table
        // 只有尚未派发的消息可由任务结束取消。Sent 可能已被原生 CLI 接收，
        // 完成、取消或换代均不能倒推出交付失败，必须继续保持未确认。
        .filter(local_cli_messages::state.eq("queued"))
        .into_boxed();
    let query = if cancel_outgoing {
        query.filter(
            local_cli_messages::recipient_task_id
                .eq(task_id)
                .or(local_cli_messages::sender_task_id.eq(task_id)),
        )
    } else {
        query.filter(local_cli_messages::recipient_task_id.eq(task_id))
    };
    let rows = query
        .select(local_cli_messages::data)
        .load::<String>(connection)?;
    for row in rows {
        let mut message: LocalCliMessage = serde_json::from_str(&row)?;
        // 已完成一代的结果不因发送方开始下一代而丢失。
        if message.subject == TASK_RESULT_SUBJECT
            && message.sender_task_id == task_id
            && message.recipient_task_id != task_id
        {
            continue;
        }
        message.state = LocalCliMessageState::Cancelled;
        write_message_state(connection, &message)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "local_cli_tasks_tests.rs"]
mod tests;
