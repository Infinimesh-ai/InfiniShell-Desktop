//! 本机 CLI 托管会话的最小控制契约；各适配器保留原生协议与权限语义。

use std::path::PathBuf;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

pub(crate) mod claude;
mod claude_profile;
pub(crate) mod codex;
#[cfg(feature = "local_fs")]
pub(crate) mod conversation_bridge;
#[cfg(feature = "local_fs")]
pub(crate) mod coordinator;
pub(crate) mod grok;
mod grok_profile;
pub(crate) mod grok_tool_lease;
pub(crate) mod local_skills;
pub(crate) mod local_tools;
pub(crate) mod managed_input;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) mod managed_process;
pub(crate) mod permissions;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) mod runtime_host;
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
    /// 固定 Claude 文件工具及逐次审批，不等价于操作系统沙箱。
    ClaudeRestrictedFilesV1,
    /// 固定 2.1.280 的文件审批策略，额外允许经审批创建文件。
    ClaudeRestrictedFilesV2,
    /// 固定 Grok 只读工具与应用 SDK 子集，逐次审批；不等价于操作系统沙箱。
    GrokRestrictedReadV1,
    /// 应用固定 Grok 文件工具及逐次审批，不声明原生文件系统沙箱。
    GrokRestrictedFilesV1,
}

impl PermissionPolicy {
    pub(crate) fn is_claude_file_profile(self) -> bool {
        matches!(
            self,
            Self::ClaudeRestrictedFilesV1 | Self::ClaudeRestrictedFilesV2
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionOptions {
    pub executable: PathBuf,
    pub cwd: PathBuf,
    /// 与当前任务数据库同属一个应用数据域，不从 CLI 工作目录推导。
    pub state_dir: PathBuf,
    pub target: SessionTarget,
    pub generation: Uuid,
    pub permission_policy: PermissionPolicy,
    pub permission_ceiling: Option<permissions::ParentPermissionCeiling>,
    pub claude_profile: Option<permissions::ClaudeFileProfile>,
    pub grok_profile: Option<permissions::GrokCreationPolicyV1>,
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
        /// 仅记录本次成功探测并与原生握手配对的版本；Claude 首次控制握手尚不提供版本。
        verified_cli_version: Option<String>,
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
    /// 原生队列在工具轮次之间并入当前执行；不代表独立回合或上一轮完成。
    InputJoined {
        message_id: Uuid,
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
    host_event_acks: Option<mpsc::Sender<()>>,
}

impl RuntimeController {
    pub(crate) async fn reserve_command(
        &self,
        generation: Uuid,
    ) -> Result<mpsc::OwnedPermit<RuntimeCommand>, RuntimeError> {
        if generation != self.generation {
            return Err(RuntimeError::StaleGeneration);
        }
        self.commands
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| RuntimeError::ControllerClosed)
    }

    pub async fn send(&self, command: RuntimeCommand) -> Result<(), RuntimeError> {
        if command.generation != self.generation {
            return Err(RuntimeError::StaleGeneration);
        }
        self.commands
            .send(command)
            .await
            .map_err(|_| RuntimeError::ControllerClosed)
    }

    /// 宿主事件只有在协调器完成对应 SQLite 提交后才能推进持久 ACK 水位。
    pub(crate) async fn acknowledge_host_event(&self) -> Result<(), RuntimeError> {
        let Some(acks) = &self.host_event_acks else {
            return Ok(());
        };
        acks.send(())
            .await
            .map_err(|_| RuntimeError::ControllerClosed)
    }
}

pub struct RuntimeConnection {
    pub controller: RuntimeController,
    pub events: mpsc::Receiver<RuntimeEvent>,
    /// 调用者负责调度并观察结果；命令队列使用有界背压，adapter 退出仍通过该结果可靠回收。
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
            host_event_acks: None,
        },
        receiver,
        sender,
        events,
    )
}
