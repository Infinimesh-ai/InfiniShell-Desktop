//! 普通 Grok PTY 的输入账本；身份快照不授予传输权限，也不建立 owned 会话。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::*;

pub(crate) const INPUT_SUBJECT: &str = "grok_native_terminal_input_v1";
const EXECUTION_KIND: &str = INPUT_SUBJECT;
const MAX_TEXT_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBridgeProcessSnapshot {
    pub pid: i32,
    pub pid_version: u32,
    pub unique_id: u64,
    pub resource_cid: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBridgeBindingSnapshot {
    pub instance_id: Uuid,
    pub native_process: NativeBridgeProcessSnapshot,
    pub boot_session: String,
    pub shell: NativeBridgeProcessSnapshot,
    pub slave_device: u64,
    pub artifact_sha256: String,
}

impl NativeBridgeBindingSnapshot {
    fn validate(&self) -> Result<()> {
        let process_valid = |process: &NativeBridgeProcessSnapshot| {
            process.pid > 0 && process.pid_version > 0 && process.unique_id > 0
        };
        if self.instance_id.is_nil()
            || !process_valid(&self.native_process)
            || !process_valid(&self.shell)
            || self.slave_device == 0
            || Uuid::parse_str(&self.boot_session).is_err()
            || self.artifact_sha256.len() != 64
            || !self
                .artifact_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("普通 Grok 原生桥身份快照无效");
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct NativeBridgeInput {
    pub binding: Value,
    pub session_id: Uuid,
    pub working_directory: String,
    pub input_revision: Uuid,
    pub message_id: Uuid,
    pub replaces: Option<Uuid>,
    pub body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeBridgeDeliveryState {
    /// 写 socket 之前已提交；断连、冷恢复和再次点击均不得产生新的投递资格。
    Unknown,
    /// 宿主持有未进入写入资格领取的证明；不能从断连或未知回执推导。
    NotSent,
    NativeAcknowledged,
    NativeRejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBridgeDelivery {
    pub version: u32,
    pub task_id: String,
    pub binding: NativeBridgeBindingSnapshot,
    pub session_id: Uuid,
    pub input_revision: Uuid,
    pub message_id: Uuid,
    pub replaces: Option<Uuid>,
    pub payload_digest: String,
    pub state: NativeBridgeDeliveryState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeBridgeInputRecord {
    #[serde(flatten)]
    pub message: LocalCliMessage,
    pub delivery: NativeBridgeDelivery,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeBridgeClaimOutcome {
    /// 仅此返回值授予一次写入资格，接收方仍须核验实际原生连接和当前草稿。
    Claimed(NativeBridgeInputRecord),
    /// 只能查询旧 message_id；已确认也不能据此清除后来编辑的新草稿。
    Existing(NativeBridgeInputRecord),
}

#[derive(Debug)]
pub enum NativeBridgePersistenceRequest {
    Claim {
        input: NativeBridgeInput,
        completion: oneshot::Sender<Result<NativeBridgeClaimOutcome, String>>,
    },
    Lookup {
        binding: Value,
        session_id: Option<Uuid>,
        body: String,
        completion: oneshot::Sender<Result<Option<NativeBridgeInputRecord>, String>>,
    },
    Acknowledge {
        expected: NativeBridgeDelivery,
        current_binding: Value,
        response: Value,
        completion: oneshot::Sender<Result<Option<NativeBridgeInputRecord>, String>>,
    },
    MarkNotSent {
        expected: NativeBridgeDelivery,
        completion: oneshot::Sender<Result<Option<NativeBridgeInputRecord>, String>>,
    },
}

type RecordReceiver = oneshot::Receiver<Result<Option<NativeBridgeInputRecord>, String>>;

pub(crate) fn claim_input(
    sender: &SyncSender<ModelEvent>,
    input: NativeBridgeInput,
) -> Result<oneshot::Receiver<Result<NativeBridgeClaimOutcome, String>>, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::GrokNativeBridge(NativeBridgePersistenceRequest::Claim {
            input,
            completion,
        }),
    )?;
    Ok(receiver)
}

/// 已知会话时跨旧实例防重；无会话时仅查询同绑定唯一记录，均不授予新投递资格。
pub(crate) fn lookup_input(
    sender: &SyncSender<ModelEvent>,
    binding: Value,
    session_id: Option<Uuid>,
    body: String,
) -> Result<RecordReceiver, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::GrokNativeBridge(NativeBridgePersistenceRequest::Lookup {
            binding,
            session_id,
            body,
            completion,
        }),
    )?;
    Ok(receiver)
}

/// 只有原生精确接收回执返回 Some；任何队列状态、一般拒绝或超时都不能清稿。
pub(crate) fn acknowledge_input(
    sender: &SyncSender<ModelEvent>,
    expected: NativeBridgeDelivery,
    current_binding: Value,
    response: Value,
) -> Result<RecordReceiver, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::GrokNativeBridge(NativeBridgePersistenceRequest::Acknowledge {
            expected,
            current_binding,
            response,
            completion,
        }),
    )?;
    Ok(receiver)
}

/// 仅宿主证明从未进入传输写入资格领取时调用，不能用网络错误或超时替代此证明。
pub(crate) fn mark_not_sent(
    sender: &SyncSender<ModelEvent>,
    expected: NativeBridgeDelivery,
) -> Result<RecordReceiver, String> {
    let (completion, receiver) = oneshot::channel();
    enqueue_request(
        sender,
        LocalCliPersistenceRequest::GrokNativeBridge(NativeBridgePersistenceRequest::MarkNotSent {
            expected,
            completion,
        }),
    )?;
    Ok(receiver)
}

pub(super) fn handle_request(
    request: NativeBridgePersistenceRequest,
    connection: &mut SqliteConnection,
) -> Result<()> {
    match request {
        NativeBridgePersistenceRequest::Claim { input, completion } => {
            complete(claim_once(connection, input), completion)
        }
        NativeBridgePersistenceRequest::Lookup {
            binding,
            session_id,
            body,
            completion,
        } => complete(
            lookup_exact(connection, binding, session_id, &body),
            completion,
        ),
        NativeBridgePersistenceRequest::Acknowledge {
            expected,
            current_binding,
            response,
            completion,
        } => complete(
            acknowledge_exact(connection, expected, current_binding, response),
            completion,
        ),
        NativeBridgePersistenceRequest::MarkNotSent {
            expected,
            completion,
        } => complete(mark_not_sent_exact(connection, expected), completion),
    }
}

pub(super) fn reject_request(request: NativeBridgePersistenceRequest, error: String) {
    match request {
        NativeBridgePersistenceRequest::Claim { completion, .. } => {
            let _ = completion.send(Err(error));
        }
        NativeBridgePersistenceRequest::Lookup { completion, .. }
        | NativeBridgePersistenceRequest::Acknowledge { completion, .. }
        | NativeBridgePersistenceRequest::MarkNotSent { completion, .. } => {
            let _ = completion.send(Err(error));
        }
    }
}

pub(super) fn is_task(task: &LocalCliTask) -> bool {
    serde_json::from_str::<Value>(&task.config_json)
        .is_ok_and(|value| value["execution_kind"] == EXECUTION_KIND)
}

fn binding_snapshot(value: Value) -> Result<NativeBridgeBindingSnapshot> {
    // 严格字段解码同时排除发现凭据；这些值只用于比较，不能恢复原生连接或授权。
    let binding: NativeBridgeBindingSnapshot = serde_json::from_value(value)?;
    binding.validate()?;
    Ok(binding)
}

fn payload_digest(body: &str) -> String {
    format!("blake3:{}", blake3::hash(body.as_bytes()).to_hex())
}

fn validate_input(session_id: Uuid, body: &str) -> Result<()> {
    if session_id.is_nil() || body.trim().is_empty() || body.len() > MAX_TEXT_BYTES {
        bail!("普通 Grok 原生桥会话或输入正文无效");
    }
    Ok(())
}

fn task_id(binding: &NativeBridgeBindingSnapshot, session_id: Uuid) -> Result<String> {
    let identity = serde_json::to_vec(&(binding, session_id))?;
    Ok(format!(
        "grok-native-terminal:{}",
        blake3::hash(&identity).to_hex()
    ))
}

fn binding_task(
    binding: &NativeBridgeBindingSnapshot,
    session_id: Uuid,
    working_directory: String,
) -> Result<LocalCliTask> {
    let task = LocalCliTask {
        version: 1,
        task_id: task_id(binding, session_id)?,
        parent_task_id: None,
        parent_generation: None,
        harness: "grok".to_owned(),
        working_directory,
        config_json: serde_json::json!({
            "execution_kind": EXECUTION_KIND,
            "binding": binding,
            "session_id": session_id,
        })
        .to_string(),
        // 此字段占用活动任务的唯一会话所有权；输入账本只在配置和投递记录中绑定会话。
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Running,
        result: None,
        terminal_evidence: None,
    };
    validate_task(&task)?;
    Ok(task)
}

fn read_record(
    connection: &mut SqliteConnection,
    message_id: Uuid,
) -> Result<Option<(String, NativeBridgeInputRecord)>> {
    let row = local_cli_messages::table
        .filter(local_cli_messages::message_id.eq(message_id.to_string()))
        .select((local_cli_messages::data, local_cli_messages::state))
        .first::<(String, String)>(connection)
        .optional()?;
    let Some((raw, state)) = row else {
        return Ok(None);
    };
    let record: NativeBridgeInputRecord = serde_json::from_str(&raw)?;
    let message = &record.message;
    let delivery = &record.delivery;
    delivery.binding.validate()?;
    validate_input(delivery.session_id, &message.body)?;
    if message.version != 1
        || message.subject != INPUT_SUBJECT
        || message.message_id != message_id.to_string()
        || delivery.version != 1
        || delivery.message_id != message_id
        || delivery.message_id.is_nil()
        || delivery.input_revision.is_nil()
        || delivery.task_id != task_id(&delivery.binding, delivery.session_id)?
        || message.sender_task_id != delivery.task_id
        || message.recipient_task_id != delivery.task_id
        || message.sender_generation != 1
        || message.recipient_generation != 1
        || delivery.payload_digest != payload_digest(&message.body)
        || state != state_name(message.state)?
    {
        bail!("普通 Grok 原生桥输入账本身份或正文不一致");
    }
    let consistent = match delivery.state {
        NativeBridgeDeliveryState::Unknown => {
            message.state == LocalCliMessageState::Unknown && message.receipt_kind.is_none()
        }
        NativeBridgeDeliveryState::NotSent => {
            message.state == LocalCliMessageState::Failed && message.receipt_kind.is_none()
        }
        NativeBridgeDeliveryState::NativeAcknowledged => {
            message.state == LocalCliMessageState::Acknowledged
                && message.receipt_kind == Some(LocalCliReceiptKind::NativeProtocol)
        }
        NativeBridgeDeliveryState::NativeRejected => {
            message.state == LocalCliMessageState::Failed
                && message.receipt_kind == Some(LocalCliReceiptKind::NativeProtocol)
        }
    };
    if !consistent {
        bail!("普通 Grok 原生桥输入状态与回执不一致");
    }
    let task =
        read_task(connection, &delivery.task_id)?.context("普通 Grok 原生桥绑定任务不存在")?;
    if task
        != binding_task(
            &delivery.binding,
            delivery.session_id,
            task.working_directory.clone(),
        )?
    {
        bail!("普通 Grok 原生桥绑定任务已被替换");
    }
    Ok(Some((raw, record)))
}

fn native_records(connection: &mut SqliteConnection) -> Result<Vec<NativeBridgeInputRecord>> {
    local_cli_messages::table
        .filter(local_cli_messages::recipient_task_id.like("grok-native-terminal:%"))
        .select(local_cli_messages::message_id)
        .load::<String>(connection)?
        .into_iter()
        .map(|message_id| {
            let id = Uuid::parse_str(&message_id)?;
            let (_, record) =
                read_record(connection, id)?.context("普通 Grok 原生桥输入记录丢失")?;
            Ok(record)
        })
        .collect()
}

fn lookup_exact(
    connection: &mut SqliteConnection,
    binding: Value,
    session_id: Option<Uuid>,
    body: &str,
) -> Result<Option<NativeBridgeInputRecord>> {
    let binding = binding_snapshot(binding)?;
    if session_id.is_some_and(|session_id| session_id.is_nil())
        || body.trim().is_empty()
        || body.len() > MAX_TEXT_BYTES
    {
        bail!("普通 Grok 原生桥查询会话或正文无效");
    }
    let mut matched = None;
    for record in latest_inputs(connection, session_id, body)? {
        let matches_scope = match session_id {
            Some(session_id) => record.delivery.session_id == session_id,
            None => record.delivery.binding == binding,
        };
        if !matches_scope {
            continue;
        }
        if matched.is_some() {
            bail!("普通 Grok 原生桥正文在多个会话中存在，不能猜测目标会话");
        }
        matched = Some(record);
    }
    Ok(matched)
}

fn latest_inputs(
    connection: &mut SqliteConnection,
    session_id: Option<Uuid>,
    body: &str,
) -> Result<Vec<NativeBridgeInputRecord>> {
    let digest = payload_digest(body);
    let mut sessions = BTreeMap::<Uuid, Vec<NativeBridgeInputRecord>>::new();
    for record in native_records(connection)? {
        if session_id.is_some_and(|session_id| record.delivery.session_id != session_id)
            || record.delivery.payload_digest != digest
        {
            continue;
        }
        if record.message.body != body {
            bail!("普通 Grok 原生桥输入摘要冲突");
        }
        sessions
            .entry(record.delivery.session_id)
            .or_default()
            .push(record);
    }
    sessions.into_values().map(chain_head).collect()
}

fn chain_head(records: Vec<NativeBridgeInputRecord>) -> Result<NativeBridgeInputRecord> {
    let records = records
        .into_iter()
        .map(|record| (record.delivery.message_id, record))
        .collect::<BTreeMap<_, _>>();
    let mut replaced = HashSet::new();
    for record in records.values() {
        if let Some(previous_id) = record.delivery.replaces {
            let previous = records
                .get(&previous_id)
                .context("普通 Grok 输入尝试链缺少前序记录")?;
            if previous.delivery.state == NativeBridgeDeliveryState::Unknown
                || !replaced.insert(previous_id)
            {
                bail!("普通 Grok 输入尝试链重复领取或前序交付仍未知");
            }
        }
    }
    let mut heads = records
        .values()
        .filter(|record| !replaced.contains(&record.delivery.message_id));
    let head = heads.next().context("普通 Grok 输入尝试链不存在最新记录")?;
    if heads.next().is_some() {
        bail!("普通 Grok 输入尝试链存在多个最新记录");
    }
    let mut visited = HashSet::new();
    let mut cursor = Some(head.delivery.message_id);
    while let Some(message_id) = cursor {
        if !visited.insert(message_id) {
            bail!("普通 Grok 输入尝试链包含循环");
        }
        cursor = records
            .get(&message_id)
            .context("普通 Grok 输入尝试链不完整")?
            .delivery
            .replaces;
    }
    if visited.len() != records.len() {
        bail!("普通 Grok 输入尝试链存在孤立记录");
    }
    Ok(head.clone())
}

fn claim_once(
    connection: &mut SqliteConnection,
    input: NativeBridgeInput,
) -> Result<NativeBridgeClaimOutcome> {
    let binding = binding_snapshot(input.binding)?;
    validate_input(input.session_id, &input.body)?;
    if input.message_id.is_nil()
        || input.input_revision.is_nil()
        || input
            .replaces
            .is_some_and(|id| id.is_nil() || id == input.message_id)
    {
        bail!("普通 Grok 原生桥输入或草稿修订标识无效");
    }
    let task = binding_task(&binding, input.session_id, input.working_directory)?;
    connection.immediate_transaction(|connection| {
        if let Some((_, record)) = read_record(connection, input.message_id)? {
            if record.delivery.binding != binding
                || record.delivery.session_id != input.session_id
                || record.message.body != input.body
            {
                bail!("同一普通 Grok 消息 ID 不能用于另一绑定、会话或正文");
            }
            return Ok(NativeBridgeClaimOutcome::Existing(record));
        }
        // TUI 重启不会产生投递资格；只有明确引用已确认链头的新用户动作可续链。
        let previous = latest_inputs(connection, Some(input.session_id), &input.body)?.pop();
        match (previous, input.replaces) {
            (Some(record), Some(replaces)) if record.delivery.message_id == replaces => {
                if record.delivery.state == NativeBridgeDeliveryState::Unknown {
                    bail!("交付状态未知的普通 Grok 输入不能再次投递");
                }
            }
            (Some(record), Some(_)) | (Some(record), None) => {
                return Ok(NativeBridgeClaimOutcome::Existing(record));
            }
            (None, Some(_)) => bail!("普通 Grok 新输入引用的前次尝试不存在"),
            (None, None) => {}
        }
        for record in native_records(connection)? {
            if record.delivery.session_id == input.session_id
                && record.delivery.input_revision == input.input_revision
                && record.message.body != input.body
            {
                bail!("同一普通 Grok 草稿修订不能提交不同正文");
            }
        }
        if let Some(previous) = read_task(connection, &task.task_id)? {
            if previous != task {
                bail!("普通 Grok 原生桥绑定或工作目录已经改变");
            }
        } else {
            write_task(connection, &task)?;
        }
        let record = NativeBridgeInputRecord {
            message: LocalCliMessage {
                version: 1,
                message_id: input.message_id.to_string(),
                sender_task_id: task.task_id.clone(),
                recipient_task_id: task.task_id.clone(),
                sender_generation: 1,
                recipient_generation: 1,
                subject: INPUT_SUBJECT.to_owned(),
                body: input.body,
                state: LocalCliMessageState::Unknown,
                receipt_kind: None,
            },
            delivery: NativeBridgeDelivery {
                version: 1,
                task_id: task.task_id.clone(),
                binding: binding.clone(),
                session_id: input.session_id,
                input_revision: input.input_revision,
                message_id: input.message_id,
                replaces: input.replaces,
                payload_digest: String::new(),
                state: NativeBridgeDeliveryState::Unknown,
            },
        };
        let mut record = record;
        record.delivery.payload_digest = payload_digest(&record.message.body);
        diesel::insert_into(local_cli_messages::table)
            .values((
                local_cli_messages::message_id.eq(&record.message.message_id),
                local_cli_messages::sender_task_id.eq(&task.task_id),
                local_cli_messages::recipient_task_id.eq(&task.task_id),
                local_cli_messages::sender_generation.eq(1_i64),
                local_cli_messages::recipient_generation.eq(1_i64),
                local_cli_messages::state.eq(state_name(record.message.state)?),
                local_cli_messages::data.eq(serde_json::to_string(&record)?),
            ))
            .execute(connection)?;
        Ok(NativeBridgeClaimOutcome::Claimed(record))
    })
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Response {
    Receipt { receipt: Receipt },
    Unknown { message_id: Uuid },
    Rejected { reason: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    message_id: Uuid,
    prompt_id: Uuid,
    session_id: Uuid,
    payload_digest: String,
    state: ReceiptState,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReceiptState {
    Claimed,
    Dispatched,
    NativeAcknowledged,
    NativeRejected,
    Unknown,
}

fn acknowledge_exact(
    connection: &mut SqliteConnection,
    mut expected: NativeBridgeDelivery,
    current_binding: Value,
    response: Value,
) -> Result<Option<NativeBridgeInputRecord>> {
    let binding = binding_snapshot(current_binding)?;
    let response: Response = serde_json::from_value(response)?;
    connection.immediate_transaction(|connection| {
        let (raw, mut record) = read_record(connection, expected.message_id)?
            .context("普通 Grok 原生桥输入尚未领取")?;
        let mut stored = record.delivery.clone();
        stored.state = NativeBridgeDeliveryState::Unknown;
        expected.state = NativeBridgeDeliveryState::Unknown;
        if expected != stored || binding != stored.binding {
            bail!("普通 Grok 原生桥回执属于旧绑定或不同输入快照");
        }
        let next = match response {
            Response::Unknown { message_id } => {
                if message_id != stored.message_id {
                    bail!("普通 Grok 未知回执属于另一条输入");
                }
                return Ok(None);
            }
            Response::Rejected { reason } => {
                if reason.is_empty() || reason.len() > 1024 {
                    bail!("普通 Grok 原生桥拒绝响应无效");
                }
                return Ok(None);
            }
            Response::Receipt { receipt } => {
                if receipt.message_id != stored.message_id
                    || receipt.prompt_id != stored.message_id
                    || receipt.session_id != stored.session_id
                    || receipt.payload_digest != stored.payload_digest
                {
                    bail!("普通 Grok 原生桥精确回执身份或正文摘要不符");
                }
                match receipt.state {
                    ReceiptState::Claimed | ReceiptState::Dispatched | ReceiptState::Unknown => {
                        return Ok(None);
                    }
                    ReceiptState::NativeAcknowledged => {
                        NativeBridgeDeliveryState::NativeAcknowledged
                    }
                    ReceiptState::NativeRejected => NativeBridgeDeliveryState::NativeRejected,
                }
            }
        };
        if record.delivery.state != NativeBridgeDeliveryState::Unknown
            && record.delivery.state != next
        {
            bail!("普通 Grok 原生桥同一输入收到冲突的原生回执");
        }
        if record.delivery.state == NativeBridgeDeliveryState::Unknown {
            record.delivery.state = next;
            record.message.state = match next {
                NativeBridgeDeliveryState::NativeAcknowledged => LocalCliMessageState::Acknowledged,
                NativeBridgeDeliveryState::NativeRejected => LocalCliMessageState::Failed,
                NativeBridgeDeliveryState::Unknown | NativeBridgeDeliveryState::NotSent => {
                    unreachable!("原生回执不能推导宿主未发送状态")
                }
            };
            record.message.receipt_kind = Some(LocalCliReceiptKind::NativeProtocol);
            let count = diesel::update(
                local_cli_messages::table
                    .filter(local_cli_messages::message_id.eq(&record.message.message_id))
                    .filter(local_cli_messages::state.eq("unknown"))
                    .filter(local_cli_messages::data.eq(&raw)),
            )
            .set((
                local_cli_messages::state.eq(state_name(record.message.state)?),
                local_cli_messages::data.eq(serde_json::to_string(&record)?),
            ))
            .execute(connection)?;
            if count != 1 {
                bail!("普通 Grok 原生桥领取记录已由其他事务取代");
            }
        }
        Ok((next == NativeBridgeDeliveryState::NativeAcknowledged).then_some(record))
    })
}

fn mark_not_sent_exact(
    connection: &mut SqliteConnection,
    expected: NativeBridgeDelivery,
) -> Result<Option<NativeBridgeInputRecord>> {
    if expected.state != NativeBridgeDeliveryState::Unknown {
        bail!("只能将尚未确认的普通 Grok 输入记为未发送");
    }
    connection.immediate_transaction(|connection| {
        let (raw, mut record) = read_record(connection, expected.message_id)?
            .context("普通 Grok 原生桥输入尚未领取")?;
        if record.delivery != expected {
            bail!("普通 Grok 未发送证明不属于当前领取记录");
        }
        record.delivery.state = NativeBridgeDeliveryState::NotSent;
        record.message.state = LocalCliMessageState::Failed;
        let count = diesel::update(
            local_cli_messages::table
                .filter(local_cli_messages::message_id.eq(&record.message.message_id))
                .filter(local_cli_messages::state.eq("unknown"))
                .filter(local_cli_messages::data.eq(raw)),
        )
        .set((
            local_cli_messages::state.eq("failed"),
            local_cli_messages::data.eq(serde_json::to_string(&record)?),
        ))
        .execute(connection)?;
        if count != 1 {
            bail!("普通 Grok 未发送记录已由其他事务取代");
        }
        Ok(Some(record))
    })
}

#[cfg(test)]
#[path = "local_cli_tasks_grok_native_bridge_tests.rs"]
mod tests;
