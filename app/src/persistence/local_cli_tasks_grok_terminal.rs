//! 普通 owned Grok TUI 输入的一次领取与原生回执；复用消息表，不启动进程或重投输入。
//! owned 配置由启动接线提供，不能单独作为进程、权限或当前 PTY 的认证依据。

use serde::{Deserialize, Serialize};

use super::*;

pub(crate) const INPUT_SUBJECT: &str = "grok_owned_terminal_input";
const EXECUTION_KIND: &str = "grok_owned_terminal";

/// input_revision 是当前编辑器及附件快照的独立 UUID，不能复用任务 generation。
/// 调用方在失效、编辑或重新绑定时须先更新任务配置，再领取新输入。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct GrokTerminalOwner {
    pub version: u32,
    pub launch_id: Uuid,
    pub binding_id: Uuid,
    pub session_id: Uuid,
    pub input_revision: Uuid,
    pub permission_revision: Uuid,
    pub cli_version: String,
    pub model_id: String,
    pub permission_mode: String,
}

#[derive(Clone, Debug)]
pub struct GrokTerminalInputClaim {
    pub message: LocalCliMessage,
    pub expected_task: LocalCliTask,
    pub binding_id: Uuid,
    pub session_id: Uuid,
    pub input_revision: Uuid,
    pub body_sha256: String,
    pub rpc_id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrokTerminalOutcome {
    EndTurn,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrokTerminalDeliveryStatus {
    /// 在任何 socket 写入前落库；断连、超时或恢复都不能把它解释为可重投。
    Unknown,
    Finished {
        native_prompt_id: Uuid,
        outcome: GrokTerminalOutcome,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GrokTerminalDelivery {
    pub version: u32,
    pub task_id: String,
    pub task_generation: i64,
    pub task_revision: i64,
    pub config_sha256: String,
    pub launch_id: Uuid,
    pub binding_id: Uuid,
    pub session_id: Uuid,
    pub input_revision: Uuid,
    pub permission_revision: Uuid,
    pub message_id: Uuid,
    pub rpc_id: Uuid,
    pub body_sha256: String,
    pub status: GrokTerminalDeliveryStatus,
}

/// 扩展字段保存在既有 data JSON；通用消息读取仍能取得原始正文，专用读取保留完整回执。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct GrokTerminalInputRecord {
    #[serde(flatten)]
    pub message: LocalCliMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grok_terminal_delivery: Option<GrokTerminalDelivery>,
}

#[derive(Debug)]
pub enum GrokTerminalPersistenceRequest {
    Claim {
        claim: GrokTerminalInputClaim,
        completion: oneshot::Sender<Result<Option<GrokTerminalInputRecord>, String>>,
    },
    Acknowledge {
        expected: GrokTerminalDelivery,
        response: Value,
        completion: oneshot::Sender<Result<Option<GrokTerminalInputRecord>, String>>,
    },
    Load {
        message_id: Uuid,
        completion: oneshot::Sender<Result<Option<GrokTerminalInputRecord>, String>>,
    },
}

type RecordReceiver = oneshot::Receiver<Result<Option<GrokTerminalInputRecord>, String>>;

/// Some 只会授予一次写入资格；None 表示此前已领取或取消，不得换 RPC ID 自动重试。
pub(crate) fn claim_input(
    sender: &SyncSender<ModelEvent>,
    claim: GrokTerminalInputClaim,
) -> Result<RecordReceiver, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::GrokTerminal(GrokTerminalPersistenceRequest::Claim {
            claim,
            completion,
        }),
    )?;
    Ok(receiver)
}

/// response 必须来自该侧车收到的原生最终 JSON-RPC 响应，队列通知、文本和 hook 均不接受。
pub(crate) fn acknowledge_input(
    sender: &SyncSender<ModelEvent>,
    expected: GrokTerminalDelivery,
    response: Value,
) -> Result<RecordReceiver, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::GrokTerminal(GrokTerminalPersistenceRequest::Acknowledge {
            expected,
            response,
            completion,
        }),
    )?;
    Ok(receiver)
}

/// 恢复仅加载记录；Unknown 没有重新领取或转换回 Queued 的 API。
pub(crate) fn load_input(
    sender: &SyncSender<ModelEvent>,
    message_id: Uuid,
) -> Result<RecordReceiver, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::GrokTerminal(GrokTerminalPersistenceRequest::Load {
            message_id,
            completion,
        }),
    )?;
    Ok(receiver)
}

pub(super) fn handle_request(
    request: GrokTerminalPersistenceRequest,
    connection: &mut SqliteConnection,
) -> Result<()> {
    match request {
        GrokTerminalPersistenceRequest::Claim { claim, completion } => {
            complete(claim_once(connection, claim), completion)
        }
        GrokTerminalPersistenceRequest::Acknowledge {
            expected,
            response,
            completion,
        } => complete(
            acknowledge_exact(connection, expected, response),
            completion,
        ),
        GrokTerminalPersistenceRequest::Load {
            message_id,
            completion,
        } => complete(
            read_record(connection, message_id).map(|row| row.map(|(_, record)| record)),
            completion,
        ),
    }
}

pub(super) fn reject_request(request: GrokTerminalPersistenceRequest, error: String) {
    let completion = match request {
        GrokTerminalPersistenceRequest::Claim { completion, .. }
        | GrokTerminalPersistenceRequest::Acknowledge { completion, .. }
        | GrokTerminalPersistenceRequest::Load { completion, .. } => completion,
    };
    let _ = completion.send(Err(error));
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn owner(task: &LocalCliTask) -> Result<GrokTerminalOwner> {
    validate_task(task)?;
    let config: Value = serde_json::from_str(&task.config_json)?;
    if task.version != 1
        || task.harness != "grok"
        || task.parent_task_id.is_some()
        || task.parent_generation.is_some()
        || config["execution_kind"] != EXECUTION_KIND
    {
        bail!("输入不属于普通 owned Grok 根会话");
    }
    let owned: GrokTerminalOwner = serde_json::from_value(config["grok_terminal"].clone())?;
    if owned.version != 1
        || owned.launch_id.is_nil()
        || owned.binding_id.is_nil()
        || owned.session_id.is_nil()
        || owned.input_revision.is_nil()
        || owned.permission_revision.is_nil()
        || owned.cli_version != "1.0.41"
        || owned.model_id != "grok-4.7"
        || task.native_session_id.as_deref() != Some(owned.session_id.to_string().as_str())
    {
        bail!("普通 Grok 启动绑定不完整或不兼容");
    }
    Ok(owned)
}

fn read_record(
    connection: &mut SqliteConnection,
    message_id: Uuid,
) -> Result<Option<(String, GrokTerminalInputRecord)>> {
    let row = local_cli_messages::table
        .filter(local_cli_messages::message_id.eq(message_id.to_string()))
        .select((local_cli_messages::data, local_cli_messages::state))
        .first::<(String, String)>(connection)
        .optional()?;
    let Some((raw, state)) = row else {
        return Ok(None);
    };
    let record: GrokTerminalInputRecord = serde_json::from_str(&raw)?;
    let message = &record.message;
    if message.version != 1
        || message.subject != INPUT_SUBJECT
        || message.sender_generation < 1
        || message.message_id != message_id.to_string()
        || message.sender_task_id != message.recipient_task_id
        || message.sender_generation != message.recipient_generation
        || state != state_name(message.state)?
    {
        bail!("普通 Grok 输入记录身份不一致");
    }
    match &record.grok_terminal_delivery {
        Some(delivery) => {
            if delivery.version != 1
                || delivery.message_id != message_id
                || delivery.task_id != message.recipient_task_id
                || delivery.task_generation != message.recipient_generation
                || delivery.task_revision < 0
                || delivery.rpc_id.is_nil()
                || delivery.launch_id.is_nil()
                || delivery.binding_id.is_nil()
                || delivery.session_id.is_nil()
                || delivery.input_revision.is_nil()
                || delivery.permission_revision.is_nil()
                || delivery.config_sha256.len() != 64
                || delivery.body_sha256 != digest(&message.body)
            {
                bail!("普通 Grok 输入领取记录损坏");
            }
            let consistent = match &delivery.status {
                GrokTerminalDeliveryStatus::Unknown => {
                    message.state == LocalCliMessageState::Sent && message.receipt_kind.is_none()
                }
                GrokTerminalDeliveryStatus::Finished {
                    native_prompt_id, ..
                } => {
                    !native_prompt_id.is_nil()
                        && message.state == LocalCliMessageState::Acknowledged
                        && message.receipt_kind == Some(LocalCliReceiptKind::NativeProtocol)
                }
            };
            if !consistent {
                bail!("普通 Grok 输入状态与回执不一致");
            }
        }
        None => {
            if !matches!(
                message.state,
                LocalCliMessageState::Queued | LocalCliMessageState::Cancelled
            ) || message.receipt_kind.is_some()
            {
                bail!("普通 Grok 已派发输入缺少领取记录");
            }
        }
    }
    Ok(Some((raw, record)))
}

fn claimed_inputs(
    connection: &mut SqliteConnection,
    task_id: &str,
) -> Result<Vec<GrokTerminalDelivery>> {
    let messages = read_task_messages(connection, task_id)?;
    let mut deliveries = Vec::new();
    for message in messages
        .into_iter()
        .filter(|message| message.subject == INPUT_SUBJECT)
    {
        let id = Uuid::parse_str(&message.message_id)?;
        let (_, record) = read_record(connection, id)?.context("普通 Grok 输入记录已丢失")?;
        if let Some(delivery) = record.grok_terminal_delivery {
            deliveries.push(delivery);
        }
    }
    Ok(deliveries)
}

pub(super) fn validate_unclaimed_cancellation(
    connection: &mut SqliteConnection,
    message: &LocalCliMessage,
) -> Result<()> {
    let id = Uuid::parse_str(&message.message_id)?;
    let (_, previous) = read_record(connection, id)?.context("普通 Grok 输入不存在")?;
    let mut expected = previous.message;
    if expected.state != LocalCliMessageState::Queued
        || previous.grok_terminal_delivery.is_some()
        || message.state != LocalCliMessageState::Cancelled
    {
        bail!("普通 Grok 已领取输入禁止通用状态改写");
    }
    expected.state = LocalCliMessageState::Cancelled;
    if expected != *message {
        bail!("取消普通 Grok 输入不能修改正文或来源");
    }
    Ok(())
}

fn write_record(
    connection: &mut SqliteConnection,
    original: &str,
    record: &GrokTerminalInputRecord,
) -> Result<()> {
    let previous: LocalCliMessage = serde_json::from_str(original)?;
    let count = diesel::update(
        local_cli_messages::table
            .filter(local_cli_messages::message_id.eq(&record.message.message_id))
            .filter(local_cli_messages::state.eq(state_name(previous.state)?))
            .filter(local_cli_messages::data.eq(original)),
    )
    .set((
        local_cli_messages::state.eq(state_name(record.message.state)?),
        local_cli_messages::data.eq(serde_json::to_string(record)?),
    ))
    .execute(connection)?;
    if count != 1 {
        bail!("普通 Grok 输入已由其他写入取代");
    }
    Ok(())
}

fn claim_once(
    connection: &mut SqliteConnection,
    claim: GrokTerminalInputClaim,
) -> Result<Option<GrokTerminalInputRecord>> {
    // 先取得 SQLite 写锁；两个连接的竞争领取不能都在旧 Queued 快照上写入。
    connection.immediate_transaction(|connection| {
        let id = Uuid::parse_str(&claim.message.message_id)?;
        if id.is_nil()
            || claim.rpc_id.is_nil()
            || claim.message.state != LocalCliMessageState::Queued
            || claim.message.receipt_kind.is_some()
            || claim.message.body.trim().is_empty()
            || claim.body_sha256 != digest(&claim.message.body)
        {
            bail!("普通 Grok 输入领取参数无效");
        }
        let (raw, mut record) = read_record(connection, id)?.context("普通 Grok 输入尚未入队")?;
        let mut original = record.message.clone();
        original.state = LocalCliMessageState::Queued;
        original.receipt_kind = None;
        if original != claim.message {
            bail!("普通 Grok 输入正文或来源与持久化记录不同");
        }
        if record.message.state != LocalCliMessageState::Queued {
            return Ok(None);
        }
        let task = read_task(connection, &record.message.recipient_task_id)?
            .context("普通 Grok 会话不存在")?;
        let owned = owner(&task)?;
        if task != claim.expected_task
            || task.generation != record.message.recipient_generation
            || task.state != LocalCliTaskState::Running
            || owned.permission_mode != "default"
            || owned.binding_id != claim.binding_id
            || owned.session_id != claim.session_id
            || owned.input_revision != claim.input_revision
        {
            bail!("普通 Grok 输入代际、配置、权限证据或编辑快照已变化");
        }
        if claimed_inputs(connection, &task.task_id)?
            .iter()
            .any(|previous| {
                previous.binding_id == owned.binding_id
                    && (previous.rpc_id == claim.rpc_id
                        || previous.input_revision == claim.input_revision)
            })
        {
            bail!("普通 Grok RPC 或编辑快照已经领取，不能换消息 ID 重投");
        }
        record.grok_terminal_delivery = Some(GrokTerminalDelivery {
            version: 1,
            task_id: task.task_id.clone(),
            task_generation: task.generation,
            task_revision: task.revision,
            config_sha256: digest(&task.config_json),
            launch_id: owned.launch_id,
            binding_id: owned.binding_id,
            session_id: owned.session_id,
            input_revision: owned.input_revision,
            permission_revision: owned.permission_revision,
            message_id: id,
            rpc_id: claim.rpc_id,
            body_sha256: claim.body_sha256,
            status: GrokTerminalDeliveryStatus::Unknown,
        });
        record.message.state = LocalCliMessageState::Sent;
        write_record(connection, &raw, &record)?;
        Ok(Some(record))
    })
}

fn native_outcome(
    response: &Value,
    delivery: &GrokTerminalDelivery,
) -> Result<GrokTerminalDeliveryStatus> {
    let meta = &response["result"]["_meta"];
    let prompt = meta["promptId"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .filter(|id| !id.is_nil())
        .context("普通 Grok 最终响应缺少原生 promptId")?;
    if response["jsonrpc"] != "2.0"
        || response.get("method").is_some()
        || response.get("error").is_some()
        || response["id"].as_str() != Some(delivery.rpc_id.to_string().as_str())
        || meta["sessionId"].as_str() != Some(delivery.session_id.to_string().as_str())
        || meta["requestId"].as_str() != Some(prompt.to_string().as_str())
        || meta["modelId"] != "grok-4.7"
    {
        bail!("普通 Grok 最终响应不属于此 RPC、会话或模型");
    }
    let outcome = match response["result"]["stopReason"].as_str() {
        Some("end_turn") => GrokTerminalOutcome::EndTurn,
        Some("cancelled") => GrokTerminalOutcome::Cancelled,
        Some(_) | None => bail!("普通 Grok 最终响应语义尚未验证"),
    };
    Ok(GrokTerminalDeliveryStatus::Finished {
        native_prompt_id: prompt,
        outcome,
    })
}

fn acknowledge_exact(
    connection: &mut SqliteConnection,
    mut expected: GrokTerminalDelivery,
    response: Value,
) -> Result<Option<GrokTerminalInputRecord>> {
    connection.immediate_transaction(|connection| {
        let (raw, mut record) = read_record(connection, expected.message_id)?.context("普通 Grok 输入不存在")?;
        let stored = record.grok_terminal_delivery.as_ref().context("普通 Grok 输入尚未领取")?;
        let mut original = stored.clone(); original.status = GrokTerminalDeliveryStatus::Unknown;
        expected.status = GrokTerminalDeliveryStatus::Unknown;
        if original != expected { bail!("普通 Grok 回执属于另一条输入或领取快照"); }
        let task = read_task(connection, &stored.task_id)?.context("普通 Grok 会话不存在")?;
        let owned = owner(&task)?;
        // 新草稿不改变旧回执归属；新启动、换代或重新绑定绝不能借旧响应确认。
        if task.generation != stored.task_generation || owned.launch_id != stored.launch_id
            || owned.binding_id != stored.binding_id || owned.session_id != stored.session_id
        { bail!("普通 Grok 回执对应的原运行已被替换"); }
        let next = native_outcome(&response, stored)?;
        match &stored.status {
            GrokTerminalDeliveryStatus::Unknown => {}
            GrokTerminalDeliveryStatus::Finished { .. } if stored.status == next => return Ok(Some(record)),
            GrokTerminalDeliveryStatus::Finished { .. } => bail!("同一普通 Grok RPC 收到冲突的原生 prompt 或终态"),
        }
        if claimed_inputs(connection, &stored.task_id)?.iter().any(|other| {
            other.message_id != stored.message_id && other.session_id == stored.session_id
                && matches!((&other.status, &next),
                    (GrokTerminalDeliveryStatus::Finished { native_prompt_id: first, .. },
                     GrokTerminalDeliveryStatus::Finished { native_prompt_id: second, .. }) if first == second)
        }) { bail!("原生 promptId 已属于另一条普通 Grok 输入"); }
        record.grok_terminal_delivery.as_mut().expect("已核对领取记录").status = next;
        record.message.state = LocalCliMessageState::Acknowledged;
        record.message.receipt_kind = Some(LocalCliReceiptKind::NativeProtocol);
        write_record(connection, &raw, &record)?;
        Ok(Some(record))
    })
}

#[cfg(test)]
#[path = "local_cli_tasks_grok_terminal_tests.rs"]
mod tests;
