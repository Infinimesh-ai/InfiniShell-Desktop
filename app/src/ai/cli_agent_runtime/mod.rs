//! 本机 CLI 托管会话的最小控制契约；各适配器保留原生协议与权限语义。

use std::path::PathBuf;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

pub(crate) mod claude;
pub(crate) mod codex;
#[cfg(feature = "local_fs")]
pub(crate) mod conversation_bridge;
#[cfg(feature = "local_fs")]
pub(crate) mod coordinator;
pub(crate) mod grok;
pub(crate) mod local_skills;
pub(crate) mod local_tools;
pub(crate) mod managed_input;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) mod managed_process;
pub(crate) mod permissions;
#[cfg(feature = "local_fs")]
pub(crate) mod task_manager_view;

const COMMAND_CAPACITY: usize = 32;
const EVENT_CAPACITY: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionTarget {
    New,
    /// 只继续已有历史，失败时绝不自动新建。
    Resume {
        native_session_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionPolicy {
    /// 使用 CLI 原有配置，实际生效值由 SessionReady 返回。
    Inherit,
    /// 只读沙箱，对不可信操作保留用户审批。
    ReadOnly,
    /// 项目写入沙箱，对不可信操作保留用户审批。
    WorkspaceWrite,
}

#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub executable: PathBuf,
    pub cwd: PathBuf,
    /// 与当前任务数据库同属一个应用数据域，不从 CLI 工作目录推导。
    pub state_dir: PathBuf,
    pub target: SessionTarget,
    pub generation: Uuid,
    pub permission_policy: PermissionPolicy,
    pub permission_ceiling: Option<permissions::ParentPermissionCeiling>,
    pub model: Option<String>,
    pub local_tools: Option<local_tools::LocalToolPermissions>,
    pub selected_skills: Vec<local_skills::SelectedLocalSkill>,
}

#[cfg(feature = "local_fs")]
pub(crate) fn current_state_dir() -> PathBuf {
    crate::persistence::database_file_path_for_current_scope()
        .parent()
        .expect("任务数据库路径必须包含父目录")
        .to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputContent {
    Text(String),
    LocalImage(PathBuf),
    Skill { name: String, path: PathBuf },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    AllowOnce,
    DenyOnce,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAction {
    Submit {
        input: Vec<InputContent>,
    },
    Steer {
        expected_turn_id: String,
        input: Vec<InputContent>,
    },
    Interrupt {
        turn_id: String,
    },
    RespondApproval {
        approval_id: String,
        decision: ApprovalDecision,
    },
    RespondLocalTool {
        turn_id: String,
        call_id: String,
        result: Result<Value, String>,
    },
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCommand {
    pub generation: Uuid,
    pub message_id: Uuid,
    pub action: RuntimeAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnOutcome {
    Completed,
    Cancelled,
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub generation: Uuid,
    /// 握手或恢复失败时尚无原生 ID；不得生成假的 ID 冒充成功恢复。
    pub native_session_id: Option<String>,
    pub kind: RuntimeEventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RuntimeEventKind {
    SessionReady {
        effective_permissions: Value,
    },
    MessageAccepted {
        message_id: Uuid,
        turn_id: Option<String>,
    },
    /// 控制响应已写入传输或已交给本地关闭流程；不能作为用户输入的原生接收回执。
    CommandDispatched {
        message_id: Uuid,
        turn_id: Option<String>,
    },
    TurnStarted {
        turn_id: String,
    },
    TextDelta {
        turn_id: String,
        item_id: String,
        text: String,
    },
    Progress {
        turn_id: String,
        message: String,
    },
    ApprovalRequested {
        approval_id: String,
        turn_id: String,
        method: String,
        details: Value,
    },
    ApprovalResolved {
        approval_id: String,
        decision: ApprovalDecision,
    },
    ApprovalCancelled {
        approval_id: String,
    },
    LocalToolCancelled {
        turn_id: String,
        call_id: String,
    },
    LocalToolRequested {
        request: local_tools::NativeLocalToolRequest,
    },
    TurnFinished {
        turn_id: String,
        outcome: TurnOutcome,
        output: String,
    },
    RequestFailed {
        message_id: Uuid,
        message: String,
    },
    /// 连接终止与回合结束是不同事件；空闲时退出也不是任务成功。
    Disconnected {
        reason: String,
    },
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("The runtime generation is no longer current")]
    StaleGeneration,
    #[error("The runtime controller is closed")]
    ControllerClosed,
    #[error("CLI version is not verified for managed sessions: {0}")]
    UnsupportedVersion(String),
    #[error("Invalid runtime configuration: {0}")]
    InvalidConfiguration(String),
    #[error("{message}")]
    PermissionCeilingRejected { message: String, details: Value },
    #[error("CLI protocol error: {0}")]
    Protocol(String),
    #[error("CLI process I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("CLI request timed out; input delivery is uncertain")]
    RequestTimedOut,
    #[error("CLI event consumer is too slow; the connection was stopped without dropping events")]
    EventBackpressure,
}

impl RuntimeError {
    pub(crate) fn permission_ceiling_evidence(&self) -> Option<&Value> {
        match self {
            Self::PermissionCeilingRejected { details, .. } => Some(details),
            Self::StaleGeneration
            | Self::ControllerClosed
            | Self::UnsupportedVersion(_)
            | Self::InvalidConfiguration(_)
            | Self::Protocol(_)
            | Self::Io(_)
            | Self::RequestTimedOut
            | Self::EventBackpressure => None,
        }
    }
}

#[derive(Clone)]
pub struct RuntimeController {
    generation: Uuid,
    commands: mpsc::Sender<RuntimeCommand>,
}

impl RuntimeController {
    pub async fn send(&self, command: RuntimeCommand) -> Result<(), RuntimeError> {
        if command.generation != self.generation {
            return Err(RuntimeError::StaleGeneration);
        }
        self.commands
            .send(command)
            .await
            .map_err(|_| RuntimeError::ControllerClosed)
    }
}

pub struct RuntimeConnection {
    pub controller: RuntimeController,
    pub events: mpsc::Receiver<RuntimeEvent>,
    /// 调用者负责调度并观察结果；队列满导致的失败仍通过该结果可靠回收。
    pub task: BoxFuture<'static, Result<(), RuntimeError>>,
}

fn channels(
    generation: Uuid,
) -> (
    RuntimeController,
    mpsc::Receiver<RuntimeCommand>,
    mpsc::Sender<RuntimeEvent>,
    mpsc::Receiver<RuntimeEvent>,
) {
    let (commands, receiver) = mpsc::channel(COMMAND_CAPACITY);
    let (sender, events) = mpsc::channel(EVENT_CAPACITY);
    (
        RuntimeController {
            generation,
            commands,
        },
        receiver,
        sender,
        events,
    )
}
