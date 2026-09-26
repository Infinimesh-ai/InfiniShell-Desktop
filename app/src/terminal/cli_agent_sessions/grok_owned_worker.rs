//! 普通 owned Grok 的一次侧车工作线程；SQLite Unknown 必须先提交，任何断连都不重投。
#![cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(windows, target_arch = "x86_64")
))]

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::SyncSender;
use std::thread;

use async_channel::{Receiver, Sender};
use futures::executor::block_on;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::grok_leader_input::{GrokLeaderInput, GrokLeaderInputError, GrokLeaderInputEvent};
use super::grok_owned_launch::GrokOwnedBinding;
use crate::persistence::ModelEvent;
use crate::persistence::local_cli_tasks::grok_terminal::{
    self, GrokTerminalDelivery, GrokTerminalInputClaim, GrokTerminalInputRecord, GrokTerminalOwner,
};
use crate::persistence::model::{
    LocalCliMessage, LocalCliMessageState, LocalCliTask, LocalCliTaskState,
};

const AVAILABLE: u8 = 0;
const REVOKED: u8 = 1;
const WRITE_CLAIMED: u8 = 2;

/// 每个编辑快照只有一个 lease；撤销不可恢复，换侧车也不能重新取得旧 lease 的写入资格。
/// claim_write 是发送授权的线性化点；若之后会话变化，已发生的写入只能保留 Unknown。
#[derive(Clone)]
pub(crate) struct GrokOwnedInputLease {
    binding_id: Uuid,
    input_revision: Uuid,
    permission_revision: Uuid,
    state: Arc<AtomicU8>,
}

impl GrokOwnedInputLease {
    pub(crate) fn new(
        binding_id: Uuid,
        input_revision: Uuid,
        permission_revision: Uuid,
    ) -> io::Result<Self> {
        if binding_id.is_nil() || input_revision.is_nil() || permission_revision.is_nil() {
            return Err(io::Error::other("owned Grok 输入 lease 身份不完整"));
        }
        Ok(Self {
            binding_id,
            input_revision,
            permission_revision,
            state: Arc::new(AtomicU8::new(AVAILABLE)),
        })
    }
    pub(crate) fn revoke(&self) {
        self.state.store(REVOKED, Ordering::SeqCst);
    }
    /// 关闭编辑器只撤销尚未写入的授权；已领取的原生回合继续收取回执。
    pub(crate) fn revoke_before_write(&self) {
        let _ = self
            .state
            .compare_exchange(AVAILABLE, REVOKED, Ordering::SeqCst, Ordering::SeqCst);
    }

    pub(crate) fn revoked(&self) -> bool {
        self.state.load(Ordering::SeqCst) == REVOKED
    }
    pub(crate) fn claim_write(&self) -> Result<(), GrokLeaderInputError> {
        self.state
            .compare_exchange(AVAILABLE, WRITE_CLAIMED, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| GrokLeaderInputError::StaleBinding)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GrokOwnedWorkerStop {
    Revoked,
    AlreadyClaimed,
    InvalidBinding,
    Persistence,
    Native,
}

pub(crate) enum GrokOwnedWorkerEvent {
    Claimed(GrokTerminalInputRecord),
    Finished(GrokTerminalInputRecord),
    NativePermissionPending {
        tool_call_id: String,
    },
    Stopped {
        reason: GrokOwnedWorkerStop,
        record: Option<GrokTerminalInputRecord>,
    },
}

/// 丢弃 handle 只让本线程关闭侧车；不终止 TUI/leader，不释放 updater 的原生会话占用。
pub(crate) struct GrokOwnedWorker {
    disconnected: Arc<AtomicBool>,
}

impl Drop for GrokOwnedWorker {
    fn drop(&mut self) {
        self.disconnected.store(true, Ordering::SeqCst);
    }
}

impl GrokOwnedWorker {
    pub(crate) fn start(
        binding: GrokOwnedBinding,
        task: LocalCliTask,
        message: LocalCliMessage,
        lease: GrokOwnedInputLease,
        database: SyncSender<ModelEvent>,
    ) -> io::Result<(Self, Receiver<GrokOwnedWorkerEvent>)> {
        let expected = GrokTerminalOwner {
            version: 1,
            launch_id: binding.launch_id,
            binding_id: binding.binding_id,
            session_id: binding.session_id,
            input_revision: lease.input_revision,
            permission_revision: binding.permission_revision,
            cli_version: "1.0.41".into(),
            model_id: "grok-4.7".into(),
            permission_mode: "default".into(),
        };
        if lease.binding_id != binding.binding_id
            || lease.permission_revision != binding.permission_revision
            || lease.state.load(Ordering::SeqCst) != AVAILABLE
            || !input_snapshot_matches(&task, &message, &expected, &binding.working_directory)
        {
            return Err(io::Error::other("owned Grok 输入绑定已变化"));
        }
        let disconnected = Arc::new(AtomicBool::new(false));
        let cancelled = disconnected.clone();
        // GUI 可直接 await 接收；同步原生 I/O 始终留在本次侧车线程。
        let (events, receiver) = async_channel::unbounded();
        thread::Builder::new()
            .name("grok-owned-input".into())
            .spawn(move || {
                let message_id =
                    Uuid::parse_str(&message.message_id).expect("入口已经校验消息 UUID");
                let mut recorded = None;
                let result = run(
                    binding,
                    task,
                    message,
                    &lease,
                    &database,
                    &events,
                    &cancelled,
                    &mut recorded,
                );
                if let Err(reason) = result {
                    // 查询仅为恢复可见性；失败或 Unknown 绝不回到 Queued，也不再创建侧车。
                    let stored = grok_terminal::load_input(&database, message_id)
                        .ok()
                        .and_then(|read| block_on(read).ok())
                        .and_then(Result::ok)
                        .flatten()
                        .or(recorded);
                    let _ = events.send_blocking(GrokOwnedWorkerEvent::Stopped {
                        reason,
                        record: stored,
                    });
                }
            })?;
        Ok((Self { disconnected }, receiver))
    }
}

fn input_snapshot_matches(
    task: &LocalCliTask,
    message: &LocalCliMessage,
    expected: &GrokTerminalOwner,
    working_directory: &Path,
) -> bool {
    let Ok(config) = serde_json::from_str::<Value>(&task.config_json) else {
        return false;
    };
    task.version == 1
        && task.generation > 0
        && task.revision >= 0
        && Uuid::parse_str(&task.task_id).is_ok_and(|id| !id.is_nil())
        && Path::new(&task.working_directory) == working_directory
        && task.harness == "grok"
        && task.parent_task_id.is_none()
        && task.parent_generation.is_none()
        && task.state == LocalCliTaskState::Running
        && task.native_session_id.as_deref() == Some(expected.session_id.to_string().as_str())
        && config["execution_kind"] == "grok_owned_terminal"
        && serde_json::from_value::<GrokTerminalOwner>(config["grok_terminal"].clone())
            .is_ok_and(|owned| owned == *expected)
        && message.version == 1
        && message.sender_task_id == task.task_id
        && message.recipient_task_id == task.task_id
        && message.sender_generation == task.generation
        && message.recipient_generation == task.generation
        && grok_terminal::is_input_subject(&message.subject)
        && message.state == LocalCliMessageState::Queued
        && message.receipt_kind.is_none()
        && Uuid::parse_str(&message.message_id).is_ok_and(|id| !id.is_nil())
        && !message.body.trim().is_empty()
        && message.body.len() <= 1024 * 1024
}

fn run(
    binding: GrokOwnedBinding,
    task: LocalCliTask,
    message: LocalCliMessage,
    lease: &GrokOwnedInputLease,
    database: &SyncSender<ModelEvent>,
    events: &Sender<GrokOwnedWorkerEvent>,
    cancelled: &AtomicBool,
    recorded: &mut Option<GrokTerminalInputRecord>,
) -> Result<(), GrokOwnedWorkerStop> {
    let current = || !lease.revoked() && !cancelled.load(Ordering::SeqCst);
    if !current() {
        return Err(GrokOwnedWorkerStop::Revoked);
    }
    let message_id =
        Uuid::parse_str(&message.message_id).map_err(|_| GrokOwnedWorkerStop::InvalidBinding)?;
    let existing = block_on(
        grok_terminal::load_input(database, message_id)
            .map_err(|_| GrokOwnedWorkerStop::Persistence)?,
    )
    .map_err(|_| GrokOwnedWorkerStop::Persistence)?
    .map_err(|_| GrokOwnedWorkerStop::Persistence)?
    .ok_or(GrokOwnedWorkerStop::Persistence)?;
    if existing.message.state != LocalCliMessageState::Queued
        || existing.grok_terminal_delivery.is_some()
    {
        *recorded = Some(existing);
        return Err(GrokOwnedWorkerStop::AlreadyClaimed);
    }
    let prompt = super::grok_owned_prompt::decode(&message.subject, &message.body)
        .map_err(|_| GrokOwnedWorkerStop::InvalidBinding)?;
    let mut bridge = GrokLeaderInput::connect(binding.into_target(), None)
        .map_err(|_| GrokOwnedWorkerStop::Native)?;
    let mut claimed: Option<GrokTerminalDelivery> = None;
    let submitted = bridge.submit_prompt_once_checked(
        lease.binding_id,
        message_id,
        &prompt,
        |delivery| {
            if !current() {
                return Err(io::Error::other("owned Grok 输入已撤销"));
            }
            let claim = GrokTerminalInputClaim {
                message: message.clone(),
                expected_task: task.clone(),
                binding_id: delivery.binding_id,
                session_id: delivery.session_id,
                input_revision: lease.input_revision,
                body_sha256: format!("{:x}", Sha256::digest(message.body.as_bytes())),
                rpc_id: delivery.rpc_id,
            };
            let record = block_on(
                grok_terminal::claim_input(database, claim)
                    .map_err(|_| io::Error::other("输入领取未提交"))?,
            )
            .map_err(|_| io::Error::other("输入领取通道关闭"))?
            .map_err(|_| io::Error::other("输入领取被拒绝"))?
            .ok_or_else(|| io::Error::other("输入已经领取或取消"))?;
            claimed = record.grok_terminal_delivery.clone();
            *recorded = Some(record.clone());
            let _ = events.send_blocking(GrokOwnedWorkerEvent::Claimed(record));
            Ok(())
        },
        || {
            if !current() {
                return Err(GrokLeaderInputError::StaleBinding);
            }
            lease.claim_write()
        },
    );
    if submitted.is_err() {
        return Err(if current() {
            GrokOwnedWorkerStop::Native
        } else {
            GrokOwnedWorkerStop::Revoked
        });
    }
    let expected = claimed.ok_or(GrokOwnedWorkerStop::Persistence)?;
    loop {
        if !current() {
            bridge.disconnect();
            return Err(GrokOwnedWorkerStop::Revoked);
        }
        match bridge
            .poll(lease.binding_id)
            .map_err(|_| GrokOwnedWorkerStop::Native)?
        {
            Some(GrokLeaderInputEvent::NativePermissionPending { tool_call_id }) => {
                // 只报告原生审批；worker 没有 RespondApproval 或 PTY 按键通道。
                let _ = events
                    .send_blocking(GrokOwnedWorkerEvent::NativePermissionPending { tool_call_id });
            }
            Some(GrokLeaderInputEvent::Delivery(_)) => {
                let response = bridge
                    .take_final_response()
                    .ok_or(GrokOwnedWorkerStop::Native)?;
                if !current() {
                    return Err(GrokOwnedWorkerStop::Revoked);
                }
                let result = block_on(
                    grok_terminal::acknowledge_input(database, expected, response)
                        .map_err(|_| GrokOwnedWorkerStop::Persistence)?,
                )
                .map_err(|_| GrokOwnedWorkerStop::Persistence)?
                .map_err(|_| GrokOwnedWorkerStop::Persistence)?
                .ok_or(GrokOwnedWorkerStop::Persistence)?;
                *recorded = Some(result.clone());
                let _ = events.send_blocking(GrokOwnedWorkerEvent::Finished(result));
                bridge.disconnect();
                return Ok(());
            }
            None => {}
        }
    }
}

#[cfg(test)]
#[path = "grok_owned_worker_tests.rs"]
mod tests;
