//! Grok 1.0.30 的 ACP 适配；原生协议实证与产品能力门禁分别维护。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::time::{Duration, Instant};

use command::Stdio;
use command::r#async::Command;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use futures::{FutureExt, pin_mut, select};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use uuid::Uuid;
use warpui::r#async::{FutureExt as _, Timer};

use super::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeConnection, RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome, channels,
};

const VERIFIED_VERSION: &str = "1.0.30";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_NATIVE_IDENTITIES: usize = 16_384;
const MAX_QUEUED_PROMPTS: usize = 32;

/// 仅验证连接、新建与能力来源；产品入口仍须关闭未通过真实验收的托管回合。
pub fn connect(options: SessionOptions) -> Result<RuntimeConnection, RuntimeError> {
    validate_options(&options)?;
    let (controller, commands, sender, events) = channels(options.generation);
    let task = Box::pin(async move {
        let mut protocol = GrokProtocol::new(options);
        let result = run_process(&mut protocol, commands, &sender).await;
        let reason = match &result {
            Ok(()) => "runtime connection closed".to_owned(),
            Err(error) => error.to_string(),
        };
        let _ = sender.try_send(protocol.event(RuntimeEventKind::Disconnected { reason }));
        result
    });
    Ok(RuntimeConnection {
        controller,
        events,
        task,
    })
}

fn validate_options(options: &SessionOptions) -> Result<(), RuntimeError> {
    if !options.executable.is_absolute() || !options.cwd.is_absolute() {
        return Err(RuntimeError::InvalidConfiguration(
            "executable and cwd must be absolute".into(),
        ));
    }
    // Grok 的权限模式不等同于 Codex 沙箱；no-leader 实证不能打开产品 leader 恢复门禁。
    if options.permission_policy != PermissionPolicy::Inherit
        || options.model.is_some()
        || options.local_tools.is_some()
        || !options.selected_skills.is_empty()
        || matches!(options.target, SessionTarget::Resume { .. })
    {
        return Err(RuntimeError::InvalidConfiguration(crate::t!(
            "cli-agent-grok-managed-unverified"
        )));
    }
    Ok(())
}

fn verified_version(output: &str) -> bool {
    output
        .trim()
        .strip_prefix("grok ")
        .and_then(|suffix| suffix.split_whitespace().next())
        == Some(VERIFIED_VERSION)
}

async fn run_process(
    protocol: &mut GrokProtocol,
    commands: mpsc::Receiver<RuntimeCommand>,
    events: &mpsc::Sender<RuntimeEvent>,
) -> Result<(), RuntimeError> {
    let mut version = Command::new(&protocol.options.executable);
    version
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = version
        .output()
        .with_timeout(Duration::from_secs(3))
        .await
        .map_err(|_| RuntimeError::RequestTimedOut)??;
    let detected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || !verified_version(&detected) {
        return Err(RuntimeError::UnsupportedVersion(detected));
    }

    // 独立 leader socket 防止此次连接意外关联用户已有的 Grok 进程。
    let directory = tempfile::tempdir()?;
    let arguments = [
        OsString::from("agent"),
        OsString::from("stdio"),
        OsString::from("--leader-socket"),
        directory.path().join("leader.sock").into_os_string(),
    ];
    let mut child = super::managed_process::spawn(
        &protocol.options.state_dir,
        protocol.options.generation,
        &protocol.options.executable,
        &arguments,
        &protocol.options.cwd,
    )
    .await?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| RuntimeError::Protocol("missing Grok stdin".into()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| RuntimeError::Protocol("missing Grok stdout".into()))?;
    let result = run_transport(protocol, &mut stdin, &mut stdout, commands, events).await;
    drop(stdin);
    drop(stdout);
    child.finish().await?;
    result
}

async fn run_transport(
    protocol: &mut GrokProtocol,
    stdin: &mut (impl AsyncWrite + Unpin),
    stdout: &mut (impl AsyncRead + Unpin),
    mut commands: mpsc::Receiver<RuntimeCommand>,
    events: &mpsc::Sender<RuntimeEvent>,
) -> Result<(), RuntimeError> {
    write_message(stdin, &protocol.initialize()).await?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if protocol.request_timed_out() {
            return Err(RuntimeError::RequestTimedOut);
        }
        enum Incoming {
            Bytes(std::io::Result<usize>),
            Command(Option<RuntimeCommand>),
            Tick,
            ConsumerClosed,
        }
        let incoming = {
            let read = stdout.read(&mut chunk).fuse();
            let command = commands.recv().fuse();
            let tick = Timer::after(Duration::from_millis(250)).fuse();
            let closed = events.closed().fuse();
            pin_mut!(read, command, tick, closed);
            select! {
                result = read => Incoming::Bytes(result),
                result = command => Incoming::Command(result),
                _ = tick => Incoming::Tick,
                () = closed => Incoming::ConsumerClosed,
            }
        };
        let effects = match incoming {
            Incoming::Bytes(result) => {
                let count = result?;
                if count == 0 {
                    return Err(RuntimeError::Protocol("Grok ACP stdout closed".into()));
                }
                buffer.extend_from_slice(&chunk[..count]);
                while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
                    if newline > MAX_LINE_BYTES {
                        return Err(RuntimeError::Protocol(
                            "Grok ACP message is too large".into(),
                        ));
                    }
                    let line = buffer.drain(..=newline).collect::<Vec<_>>();
                    let message = serde_json::from_slice(&line)
                        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
                    let effects = protocol.receive(message)?;
                    flush_effects(protocol, stdin, events, effects).await?;
                    if protocol.closed {
                        return Ok(());
                    }
                }
                if buffer.len() > MAX_LINE_BYTES {
                    return Err(RuntimeError::Protocol(
                        "Grok ACP message is too large".into(),
                    ));
                }
                continue;
            }
            Incoming::Command(Some(command)) => protocol.command(command),
            Incoming::Command(None) | Incoming::ConsumerClosed => return Ok(()),
            Incoming::Tick => continue,
        };
        flush_effects(protocol, stdin, events, effects).await?;
        if protocol.closed {
            return Ok(());
        }
    }
}

async fn write_message(
    stdin: &mut (impl AsyncWrite + Unpin),
    message: &Value,
) -> Result<(), RuntimeError> {
    let mut encoded =
        serde_json::to_vec(message).map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    encoded.push(b'\n');
    stdin
        .write_all(&encoded)
        .with_timeout(WRITE_TIMEOUT)
        .await
        .map_err(|_| RuntimeError::RequestTimedOut)??;
    stdin
        .flush()
        .with_timeout(WRITE_TIMEOUT)
        .await
        .map_err(|_| RuntimeError::RequestTimedOut)??;
    Ok(())
}

async fn flush_effects(
    protocol: &GrokProtocol,
    stdin: &mut (impl AsyncWrite + Unpin),
    events: &mpsc::Sender<RuntimeEvent>,
    effects: Effects,
) -> Result<(), RuntimeError> {
    let publish = |kind| {
        events
            .try_send(protocol.event(kind))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => RuntimeError::EventBackpressure,
                mpsc::error::TrySendError::Closed(_) => RuntimeError::ControllerClosed,
            })
    };
    let (confirmed, after_write): (Vec<_>, Vec<_>) = effects.events.into_iter().partition(|kind| {
        matches!(
            kind,
            RuntimeEventKind::TurnFinished { .. }
                | RuntimeEventKind::RequestFailed { .. }
                | RuntimeEventKind::ApprovalCancelled { .. }
        )
    });
    // 下一轮写入失败不能丢失上一轮真实终态；派发确认与审批决定仍在写入成功后发布。
    for kind in confirmed {
        publish(kind)?;
    }
    for message in effects.writes {
        write_message(stdin, &message).await?;
    }
    for kind in after_write {
        publish(kind)?;
    }
    Ok(())
}

#[derive(Default)]
struct Effects {
    writes: Vec<Value>,
    events: Vec<RuntimeEventKind>,
}

#[derive(Clone)]
enum PendingKind {
    Initialize,
    Authenticate,
    OpenSession { requested_id: Option<String> },
    Prompt,
    CloseSession,
}

struct PendingRequest {
    id: u64,
    kind: PendingKind,
    sent_at: Instant,
}

struct PendingPrompt {
    message_id: Uuid,
    native_id: Option<String>,
    started: bool,
    finished: bool,
    stream_id: Option<i64>,
    closed_streams: HashSet<i64>,
    last_text_sequence: Option<u64>,
    chunks: BTreeMap<u64, (u64, String)>,
    next_chunk: u64,
    retained_text_bytes: usize,
    output: String,
    cancel_sent: Option<Instant>,
}

struct QueuedPrompt {
    message_id: Uuid,
    content: Vec<Value>,
}

struct NativeTool {
    turn_id: String,
    finished: bool,
}

struct PendingApproval {
    native_id: Value,
    turn_id: String,
    tool_call_id: String,
    resolved: bool,
}

// 仅反序列化展示所需的实际原生字段，不把账号、路径或扩展凭据元数据带入快照。
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportedModels {
    current_model_id: String,
    available_models: Vec<ReportedModel>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportedModel {
    model_id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportedConfig {
    id: String,
    name: String,
    category: String,
    #[serde(rename = "type")]
    kind: String,
    current_value: String,
    options: Vec<ReportedConfigOption>,
}

#[derive(Clone, Deserialize, Serialize)]
struct ReportedConfigOption {
    value: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct ReportedCommand {
    name: String,
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<ReportedCommandInput>,
}

#[derive(Clone, Deserialize, Serialize)]
struct ReportedCommandInput {
    hint: String,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportedMetadata {
    models: Option<ReportedModels>,
    config_options: Vec<ReportedConfig>,
    available_commands: Vec<ReportedCommand>,
}

struct GrokProtocol {
    options: SessionOptions,
    session_id: Option<String>,
    next_id: u64,
    pending: Option<PendingRequest>,
    prompt: Option<PendingPrompt>,
    queued: VecDeque<QueuedPrompt>,
    controls: HashMap<Uuid, [u8; 32]>,
    tools: HashMap<String, NativeTool>,
    approvals: HashMap<String, PendingApproval>,
    permission_requests: HashMap<String, [u8; 32]>,
    responses: HashMap<u64, [u8; 32]>,
    submitted_messages: HashMap<Uuid, [u8; 32]>,
    notification_ids: HashMap<String, [u8; 32]>,
    observed_prompt_ids: HashSet<String>,
    reported_capabilities: Value,
    reported_metadata: ReportedMetadata,
    closed: bool,
}

impl GrokProtocol {
    fn new(options: SessionOptions) -> Self {
        Self {
            options,
            session_id: None,
            next_id: 0,
            pending: None,
            prompt: None,
            queued: VecDeque::new(),
            controls: HashMap::new(),
            tools: HashMap::new(),
            approvals: HashMap::new(),
            permission_requests: HashMap::new(),
            responses: HashMap::new(),
            submitted_messages: HashMap::new(),
            notification_ids: HashMap::new(),
            observed_prompt_ids: HashSet::new(),
            reported_capabilities: Value::Null,
            reported_metadata: ReportedMetadata::default(),
            closed: false,
        }
    }

    fn event(&self, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent {
            generation: self.options.generation,
            native_session_id: self.session_id.clone(),
            kind,
        }
    }

    fn request(&mut self, kind: PendingKind, method: &str, params: Value) -> Value {
        self.next_id += 1;
        self.pending = Some(PendingRequest {
            id: self.next_id,
            kind,
            sent_at: Instant::now(),
        });
        json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
    }

    fn initialize(&mut self) -> Value {
        self.request(
            PendingKind::Initialize,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false
                }
            }),
        )
    }

    fn request_timed_out(&self) -> bool {
        self.pending.as_ref().is_some_and(|request| {
            if matches!(request.kind, PendingKind::Prompt) {
                if let Some(prompt) = &self.prompt {
                    if let Some(sent) = prompt.cancel_sent {
                        return sent.elapsed() >= REQUEST_TIMEOUT;
                    }
                    // 原生已经关联的模型运行与用户审批没有三十秒完成保证。
                    if prompt.native_id.is_some() && !prompt.finished {
                        return false;
                    }
                }
            }
            request.sent_at.elapsed() >= REQUEST_TIMEOUT
        })
    }

    fn command(&mut self, command: RuntimeCommand) -> Effects {
        if command.generation != self.options.generation {
            return rejected_command(
                command.message_id,
                RuntimeError::StaleGeneration.to_string(),
            );
        }
        // 产品门禁不随独立 no-leader 协议验收改变；产品 leader 路径尚待真实验证。
        if command.action != RuntimeAction::Shutdown {
            return rejected_command(
                command.message_id,
                crate::t!("cli-agent-grok-managed-unverified"),
            );
        }
        self.acp_command(command)
    }

    /// 已有真实夹具覆盖的 ACP 子集；调用方的产品执行门禁独立保留。
    fn acp_command(&mut self, command: RuntimeCommand) -> Effects {
        if command.generation != self.options.generation {
            return rejected_command(
                command.message_id,
                RuntimeError::StaleGeneration.to_string(),
            );
        }
        if self.closed && command.action != RuntimeAction::Shutdown {
            return rejected_command(
                command.message_id,
                RuntimeError::ControllerClosed.to_string(),
            );
        }
        match command.action {
            RuntimeAction::Shutdown => {
                if self.closed
                    || matches!(
                        self.pending.as_ref().map(|request| &request.kind),
                        Some(PendingKind::CloseSession)
                    )
                {
                    return Effects::default();
                }
                if self.session_id.is_none()
                    || self.pending.is_some()
                    || !self.reported_capabilities["sessionCapabilities"]["close"].is_object()
                {
                    // 握手尚未结束或仍有请求时只退出连接，不推断远端回合已经取消。
                    self.closed = true;
                    let mut events = vec![RuntimeEventKind::CommandDispatched {
                        message_id: command.message_id,
                        turn_id: None,
                    }];
                    self.cancel_approvals(&mut events);
                    events.extend(self.queued.drain(..).map(|queued| {
                        RuntimeEventKind::RequestFailed {
                            message_id: queued.message_id,
                            message: RuntimeError::ControllerClosed.to_string(),
                        }
                    }));
                    return Effects {
                        writes: Vec::new(),
                        events,
                    };
                }
                let request = self.request(
                    PendingKind::CloseSession,
                    "session/close",
                    json!({"sessionId": self.session_id}),
                );
                Effects {
                    writes: vec![request],
                    events: vec![RuntimeEventKind::CommandDispatched {
                        message_id: command.message_id,
                        turn_id: None,
                    }],
                }
            }
            RuntimeAction::Submit { input } => {
                let fingerprint = match message_fingerprint(&json!(&input)) {
                    Ok(fingerprint) => fingerprint,
                    Err(error) => return rejected_command(command.message_id, error.to_string()),
                };
                if let Some(previous) = self.submitted_messages.get(&command.message_id) {
                    return if previous == &fingerprint {
                        Effects::default()
                    } else {
                        rejected_command(
                            command.message_id,
                            "Grok message id was reused with different input".into(),
                        )
                    };
                }
                if self.closed
                    || self.session_id.is_none()
                    || self.controls.contains_key(&command.message_id)
                    || self
                        .pending
                        .as_ref()
                        .is_some_and(|request| !matches!(request.kind, PendingKind::Prompt))
                    || self.queued.len() >= MAX_QUEUED_PROMPTS
                    || self.submitted_messages.len() >= MAX_NATIVE_IDENTITIES
                {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-agent-grok-managed-unverified"),
                    );
                }
                let mut content = Vec::new();
                for item in input {
                    match item {
                        InputContent::Text(text) => {
                            content.push(json!({"type": "text", "text": text}))
                        }
                        InputContent::LocalImage(_) | InputContent::Skill { .. } => {
                            return rejected_command(
                                command.message_id,
                                crate::t!("cli-agent-grok-managed-unverified"),
                            );
                        }
                    }
                }
                if content.is_empty() {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-task-manager-empty-prompt"),
                    );
                }
                self.submitted_messages
                    .insert(command.message_id, fingerprint);
                let prompt = QueuedPrompt {
                    message_id: command.message_id,
                    content,
                };
                if self.pending.is_some() {
                    // 本地等待不代表原生接收，也不向正在运行的回合发送第二个 prompt。
                    self.queued.push_back(prompt);
                    Effects::default()
                } else {
                    self.start_prompt(prompt)
                }
            }
            RuntimeAction::Interrupt { turn_id } => {
                if let Some(effects) =
                    self.control_duplicate(command.message_id, &json!({"interrupt": turn_id}))
                {
                    return effects;
                }
                let Some(prompt) = self.prompt.as_mut().filter(|prompt| {
                    !prompt.finished && prompt.native_id.as_deref() == Some(turn_id.as_str())
                }) else {
                    return rejected_command(
                        command.message_id,
                        "Grok turn is no longer active".into(),
                    );
                };
                if prompt.cancel_sent.is_some() {
                    return Effects::default();
                }
                prompt.cancel_sent = Some(Instant::now());
                Effects {
                    writes: vec![
                        json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": self.session_id}}),
                    ],
                    events: vec![RuntimeEventKind::CommandDispatched {
                        message_id: command.message_id,
                        turn_id: Some(turn_id),
                    }],
                }
            }
            RuntimeAction::RespondApproval {
                approval_id,
                decision,
            } => {
                if let Some(effects) = self.control_duplicate(
                    command.message_id,
                    &json!({"approval": approval_id, "decision": decision}),
                ) {
                    return effects;
                }
                let Some(approval) = self.approvals.get_mut(&approval_id) else {
                    return rejected_command(command.message_id, "Unknown Grok approval".into());
                };
                if approval.resolved
                    || !self.prompt.as_ref().is_some_and(|prompt| {
                        !prompt.finished
                            && prompt.cancel_sent.is_none()
                            && prompt.native_id.as_deref() == Some(approval.turn_id.as_str())
                    })
                    || !self
                        .tools
                        .get(&approval.tool_call_id)
                        .is_some_and(|tool| !tool.finished && tool.turn_id == approval.turn_id)
                {
                    return rejected_command(
                        command.message_id,
                        "Grok approval is no longer active".into(),
                    );
                }
                let option_id = match decision {
                    ApprovalDecision::AllowOnce => "allow-once",
                    ApprovalDecision::DenyOnce => "reject-once",
                };
                approval.resolved = true;
                Effects {
                    writes: vec![
                        json!({"jsonrpc": "2.0", "id": approval.native_id, "result": {"outcome": {"outcome": "selected", "optionId": option_id}}}),
                    ],
                    events: vec![
                        RuntimeEventKind::CommandDispatched {
                            message_id: command.message_id,
                            turn_id: Some(approval.turn_id.clone()),
                        },
                        RuntimeEventKind::ApprovalResolved {
                            approval_id,
                            decision,
                        },
                    ],
                }
            }
            RuntimeAction::Steer { .. } | RuntimeAction::RespondLocalTool { .. } => {
                rejected_command(
                    command.message_id,
                    crate::t!("cli-agent-grok-managed-unverified"),
                )
            }
        }
    }

    fn start_prompt(&mut self, queued: QueuedPrompt) -> Effects {
        self.prompt = Some(PendingPrompt {
            message_id: queued.message_id,
            native_id: None,
            started: false,
            finished: false,
            stream_id: None,
            closed_streams: HashSet::new(),
            last_text_sequence: None,
            chunks: BTreeMap::new(),
            next_chunk: 1,
            retained_text_bytes: 0,
            output: String::new(),
            cancel_sent: None,
        });
        Effects {
            writes: vec![self.request(
                PendingKind::Prompt,
                "session/prompt",
                json!({"sessionId": self.session_id, "prompt": queued.content}),
            )],
            events: Vec::new(),
        }
    }

    fn complete_rpc(&mut self, effects: &mut Effects) {
        self.cancel_approvals(&mut effects.events);
        self.prompt = None;
        if !self.closed {
            if let Some(queued) = self.queued.pop_front() {
                effects.writes.extend(self.start_prompt(queued).writes);
            }
        }
    }

    fn control_duplicate(&mut self, message_id: Uuid, value: &Value) -> Option<Effects> {
        let fingerprint = match message_fingerprint(value) {
            Ok(fingerprint) => fingerprint,
            Err(error) => return Some(rejected_command(message_id, error.to_string())),
        };
        if let Some(previous) = self.controls.get(&message_id) {
            return Some(if previous == &fingerprint {
                Effects::default()
            } else {
                rejected_command(
                    message_id,
                    "Grok control id was reused with different content".into(),
                )
            });
        }
        if self.controls.len() >= MAX_NATIVE_IDENTITIES
            || self.submitted_messages.contains_key(&message_id)
        {
            return Some(rejected_command(
                message_id,
                "Grok control identity cannot be reused".into(),
            ));
        }
        self.controls.insert(message_id, fingerprint);
        None
    }

    fn permission_request(&mut self, message: &Value) -> Result<Effects, RuntimeError> {
        let id = &message["id"];
        if !(id.as_u64().is_some() || id.as_str().is_some_and(valid_native_id)) {
            return Err(RuntimeError::Protocol(
                "invalid Grok approval request id".into(),
            ));
        }
        let approval_id = format!(
            "grok:{}",
            serde_json::to_string(id).map_err(|error| RuntimeError::Protocol(error.to_string()))?
        );
        let fingerprint = message_fingerprint(message)?;
        if let Some(previous) = self.permission_requests.get(&approval_id) {
            return if previous == &fingerprint {
                // 同一原生请求重投不重开审批，也不重复发送已经决定的允许。
                Ok(Effects::default())
            } else {
                Err(RuntimeError::Protocol(
                    "conflicting Grok approval request identity".into(),
                ))
            };
        }
        self.permission_requests
            .insert(approval_id.clone(), fingerprint);
        let params = &message["params"];
        let call_id = params["toolCall"]["toolCallId"].as_str();
        let turn_id = self
            .prompt
            .as_ref()
            .filter(|prompt| !prompt.finished && prompt.cancel_sent.is_none())
            .and_then(|prompt| prompt.native_id.as_deref());
        let correlated = params["sessionId"].as_str() == self.session_id.as_deref()
            && self.session_id.is_some()
            && call_id.is_some_and(|id| {
                self.tools
                    .get(id)
                    .is_some_and(|tool| !tool.finished && Some(tool.turn_id.as_str()) == turn_id)
            });
        if !correlated {
            return Ok(cancelled_permission(id));
        }
        let Some(options) = params["options"].as_array() else {
            return Ok(cancelled_permission(id));
        };
        let mut ids = HashSet::new();
        if options.iter().any(|option| {
            option["optionId"]
                .as_str()
                .is_none_or(|id| !valid_native_id(id) || !ids.insert(id))
        }) {
            return Ok(cancelled_permission(id));
        }
        // 某些客户端的 enable-always-approve 也声明 allow_once，只认已实测的精确选项 ID。
        let allow_once = options
            .iter()
            .any(|option| option["optionId"] == "allow-once" && option["kind"] == "allow_once");
        let reject_once = options
            .iter()
            .any(|option| option["optionId"] == "reject-once" && option["kind"] == "reject_once");
        if !allow_once || !reject_once {
            return Ok(cancelled_permission(id));
        }
        let turn_id = turn_id.expect("关联成功后必须存在回合").to_owned();
        self.approvals.insert(
            approval_id.clone(),
            PendingApproval {
                native_id: id.clone(),
                turn_id: turn_id.clone(),
                tool_call_id: call_id.expect("关联成功后必须存在工具调用").to_owned(),
                resolved: false,
            },
        );
        Ok(Effects {
            writes: Vec::new(),
            events: vec![RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method: "session/request_permission".into(),
                details: params.clone(),
            }],
        })
    }

    fn cancel_approvals(&mut self, events: &mut Vec<RuntimeEventKind>) {
        let turn_id = self
            .prompt
            .as_ref()
            .and_then(|prompt| prompt.native_id.as_deref());
        for (id, approval) in &mut self.approvals {
            if !approval.resolved && Some(approval.turn_id.as_str()) == turn_id {
                approval.resolved = true;
                events.push(RuntimeEventKind::ApprovalCancelled {
                    approval_id: id.clone(),
                });
            }
        }
    }

    fn prompt_result(&mut self, result: &Value) -> Result<Effects, RuntimeError> {
        if result["_meta"].get("sessionId").is_some()
            && result["_meta"]["sessionId"].as_str() != self.session_id.as_deref()
        {
            return Err(RuntimeError::Protocol(
                "Grok prompt response changed the session id".into(),
            ));
        }
        if result["_meta"].get("promptId").is_some()
            && result["_meta"]["promptId"].as_str()
                != self
                    .prompt
                    .as_ref()
                    .and_then(|prompt| prompt.native_id.as_deref())
        {
            return Err(RuntimeError::Protocol(
                "Grok prompt response changed the turn id".into(),
            ));
        }
        if result["stopReason"] == "error" {
            return Ok(self.finish_prompt_failure(
                result["agentResult"]
                    .as_str()
                    .unwrap_or("Grok prompt failed")
                    .to_owned(),
            ));
        }
        let outcome = match result["stopReason"].as_str() {
            Some("end_turn") => TurnOutcome::Completed,
            Some("cancelled")
                if matches!(
                    result["_meta"]["cancellationCategory"].as_str(),
                    Some("MidTurnAbort" | "PermissionRejected")
                ) =>
            {
                TurnOutcome::Cancelled
            }
            Some(_) | None => {
                return Err(RuntimeError::Protocol(
                    "unverified Grok prompt stop reason".into(),
                ));
            }
        };
        let Some(prompt) = self.prompt.as_mut() else {
            return Err(RuntimeError::Protocol(
                "Grok prompt response has no active request".into(),
            ));
        };
        let native_id = result["_meta"]["promptId"]
            .as_str()
            .filter(|id| valid_native_id(id));
        if native_id.is_none() || native_id != prompt.native_id.as_deref() || prompt.finished {
            return Err(RuntimeError::Protocol(
                "Grok prompt completion does not match the active turn".into(),
            ));
        }
        if prompt
            .chunks
            .keys()
            .next_back()
            .is_some_and(|last| *last >= prompt.next_chunk)
        {
            return Err(RuntimeError::Protocol(
                "Grok prompt completed with missing text chunks".into(),
            ));
        }
        prompt.finished = true;
        Ok(Effects {
            writes: Vec::new(),
            events: vec![RuntimeEventKind::TurnFinished {
                turn_id: native_id.expect("已校验原生回合 ID").to_owned(),
                outcome,
                output: prompt.output.clone(),
            }],
        })
    }

    fn session_update(&mut self, params: &Value) -> Result<Effects, RuntimeError> {
        let update = &params["update"];
        let session_id = self.session_id.as_deref();
        let Some(prompt) = self.prompt.as_mut().filter(|prompt| !prompt.finished) else {
            return Ok(Effects::default());
        };
        let Some(turn_id) = prompt.native_id.as_deref() else {
            return Ok(Effects::default());
        };
        if params["_meta"]["promptId"].as_str() != Some(turn_id) {
            return Ok(Effects::default());
        }
        let mut effects = Effects::default();
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                let event_id = params["_meta"]["eventId"]
                    .as_str()
                    .filter(|id| valid_native_id(id));
                let sequence = event_id.and_then(|id| {
                    session_id.and_then(|session| {
                        let suffix = id.strip_prefix(session)?.strip_prefix('-')?;
                        let sequence = suffix.parse::<u64>().ok()?;
                        (sequence.to_string() == suffix).then_some(sequence)
                    })
                });
                let stream_id = params["_meta"]["streamStartMs"].as_i64();
                let chunk_id = params["_meta"]["chunkId"].as_u64().filter(|id| *id > 0);
                let text = update["content"]["text"]
                    .as_str()
                    .filter(|_| update["content"]["type"] == "text");
                let (Some(sequence), Some(stream_id), Some(chunk_id), Some(text)) =
                    (sequence, stream_id, chunk_id, text)
                else {
                    return Err(RuntimeError::Protocol(
                        "Grok text chunk has no verified identity".into(),
                    ));
                };
                if let Some(previous) = prompt.stream_id.filter(|previous| *previous != stream_id) {
                    // streamStartMs 只标识响应，不参与排序；已实测 eventId 的计数后缀保持递增。
                    if prompt.closed_streams.contains(&stream_id)
                        || prompt
                            .chunks
                            .keys()
                            .next_back()
                            .is_some_and(|last| *last >= prompt.next_chunk)
                        || prompt
                            .last_text_sequence
                            .is_some_and(|last| sequence <= last)
                    {
                        return Err(RuntimeError::Protocol(
                            "Grok text stream arrived across an unresolved or closed boundary"
                                .into(),
                        ));
                    }
                    prompt.closed_streams.insert(previous);
                    prompt.chunks.clear();
                    prompt.next_chunk = 1;
                }
                prompt.stream_id = Some(stream_id);
                if let Some(previous) = prompt.chunks.get(&chunk_id) {
                    return if previous.0 == sequence && previous.1 == text {
                        Ok(Effects::default())
                    } else {
                        Err(RuntimeError::Protocol(
                            "conflicting Grok text chunk identity".into(),
                        ))
                    };
                }
                if prompt
                    .last_text_sequence
                    .is_some_and(|last| sequence <= last)
                {
                    return Err(RuntimeError::Protocol(
                        "Grok text event regressed behind published output".into(),
                    ));
                }
                if prompt.chunks.len() >= MAX_NATIVE_IDENTITIES
                    || prompt.retained_text_bytes + text.len() > MAX_LINE_BYTES
                {
                    return Err(RuntimeError::Protocol(
                        "Grok output retention limit reached".into(),
                    ));
                }
                prompt.retained_text_bytes += text.len();
                prompt.chunks.insert(chunk_id, (sequence, text.to_owned()));
                if !prompt.started {
                    prompt.started = true;
                    effects.events.push(RuntimeEventKind::TurnStarted {
                        turn_id: turn_id.to_owned(),
                    });
                }
                while let Some((sequence, text)) = prompt.chunks.get(&prompt.next_chunk) {
                    if prompt
                        .last_text_sequence
                        .is_some_and(|last| *sequence <= last)
                    {
                        return Err(RuntimeError::Protocol(
                            "Grok chunk order conflicts with native event order".into(),
                        ));
                    }
                    prompt.last_text_sequence = Some(*sequence);
                    prompt.output.push_str(text);
                    effects.events.push(RuntimeEventKind::TextDelta {
                        turn_id: turn_id.to_owned(),
                        item_id: format!("{turn_id}:{stream_id}"),
                        text: text.clone(),
                    });
                    prompt.next_chunk += 1;
                }
            }
            Some("tool_call" | "tool_call_update") => {
                if !params["_meta"]["eventId"]
                    .as_str()
                    .is_some_and(valid_native_id)
                {
                    return Err(RuntimeError::Protocol(
                        "Grok tool event has no verified identity".into(),
                    ));
                }
                let call_id = update["toolCallId"]
                    .as_str()
                    .filter(|id| valid_native_id(id))
                    .ok_or_else(|| {
                        RuntimeError::Protocol("missing Grok tool call identity".into())
                    })?;
                let tool = self
                    .tools
                    .entry(call_id.to_owned())
                    .or_insert_with(|| NativeTool {
                        turn_id: turn_id.to_owned(),
                        finished: false,
                    });
                if tool.turn_id != turn_id {
                    return Err(RuntimeError::Protocol(
                        "Grok tool call id was reused in another turn".into(),
                    ));
                }
                if matches!(update["status"].as_str(), Some("completed" | "failed")) {
                    tool.finished = true;
                    for (id, approval) in &mut self.approvals {
                        if !approval.resolved && approval.tool_call_id == call_id {
                            approval.resolved = true;
                            effects.events.push(RuntimeEventKind::ApprovalCancelled {
                                approval_id: id.clone(),
                            });
                        }
                    }
                }
            }
            Some(_) | None => {}
        }
        Ok(effects)
    }

    fn open_session(&mut self) -> Result<Value, RuntimeError> {
        let (method, requested_id) = match &self.options.target {
            SessionTarget::New => ("session/new", None),
            SessionTarget::Resume { native_session_id } => {
                if !valid_native_id(native_session_id) {
                    return Err(RuntimeError::Protocol(
                        "invalid Grok requested session id".into(),
                    ));
                }
                if self.reported_capabilities["loadSession"] != true {
                    return Err(RuntimeError::Protocol(
                        "Grok session load is not advertised".into(),
                    ));
                }
                let method = "session/load";
                (method, Some(native_session_id.clone()))
            }
        };
        let mut params = json!({"cwd": self.options.cwd, "mcpServers": []});
        if let Some(id) = &requested_id {
            params["sessionId"] = json!(id);
        }
        Ok(self.request(PendingKind::OpenSession { requested_id }, method, params))
    }

    fn receive(&mut self, message: Value) -> Result<Effects, RuntimeError> {
        if self.responses.len() >= MAX_NATIVE_IDENTITIES
            || self.notification_ids.len() >= MAX_NATIVE_IDENTITIES
            || self.observed_prompt_ids.len() >= MAX_NATIVE_IDENTITIES
            || self.tools.len() >= MAX_NATIVE_IDENTITIES
            || self.permission_requests.len() >= MAX_NATIVE_IDENTITIES
        {
            // 到达上限时中止连接，不删除旧身份后误认旧回调为新的请求。
            return Err(RuntimeError::Protocol(
                "Grok native identity ledger limit reached".into(),
            ));
        }
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(RuntimeError::Protocol(
                "Grok response is not JSON-RPC 2.0".into(),
            ));
        }
        if let Some(method) = message.get("method") {
            if !method.is_string()
                || message.get("result").is_some()
                || message.get("error").is_some()
            {
                return Err(RuntimeError::Protocol("invalid Grok server message".into()));
            }
            if method == "session/request_permission" && message.get("id").is_some() {
                return self.permission_request(&message);
            }
            // 未声明的文件与终端客户端工具不执行。
            if let Some(id) = message.get("id") {
                return Ok(Effects {
                    writes: vec![
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Client method is not supported"}}),
                    ],
                    events: Vec::new(),
                });
            }
            return self.notification(&message);
        }
        let id = message
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| RuntimeError::Protocol("invalid Grok response id".into()))?;
        let fingerprint = message_fingerprint(&message)?;
        if let Some(previous) = self.responses.get(&id) {
            return if previous == &fingerprint {
                Ok(Effects::default())
            } else {
                Err(RuntimeError::Protocol(
                    "conflicting Grok response for completed request".into(),
                ))
            };
        }
        let kind = self
            .pending
            .as_ref()
            .filter(|pending| pending.id == id)
            .ok_or_else(|| RuntimeError::Protocol("unsolicited Grok response".into()))?
            .kind
            .clone();
        if let Some(error) = message.get("error") {
            if message.get("result").is_some() || !error.is_object() {
                return Err(RuntimeError::Protocol("invalid Grok error response".into()));
            }
            self.pending = None;
            self.responses.insert(id, fingerprint);
            let code = error["code"].as_i64();
            let detail = error["data"]["message"]
                .as_str()
                .or_else(|| error["message"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Grok ACP request failed (code {code:?})"));
            if matches!(kind, PendingKind::Prompt) {
                let mut effects = self.finish_prompt_failure(detail);
                self.complete_rpc(&mut effects);
                return Ok(effects);
            }
            // 认证、恢复和关闭失败都不重新新建会话或继续尝试其他请求。
            return Err(RuntimeError::Protocol(format!(
                "Grok ACP request failed (code {code:?})"
            )));
        }
        let result = message
            .get("result")
            .filter(|value| value.is_object())
            .ok_or_else(|| RuntimeError::Protocol("Grok response has no result object".into()))?;
        self.pending = None;
        self.responses.insert(id, fingerprint);
        let mut effects = Effects::default();
        match kind {
            PendingKind::Initialize => {
                if result["protocolVersion"].as_u64() != Some(1)
                    || result["_meta"]["agentVersion"].as_str() != Some(VERIFIED_VERSION)
                {
                    return Err(RuntimeError::Protocol(
                        "unverified Grok ACP protocol or agent version".into(),
                    ));
                }
                let capabilities = result
                    .get("agentCapabilities")
                    .filter(|value| value.is_object())
                    .ok_or_else(|| {
                        RuntimeError::Protocol("missing Grok agent capabilities".into())
                    })?;
                self.reported_capabilities = json!({
                    "loadSession": capabilities.get("loadSession"),
                    "promptCapabilities": capabilities.get("promptCapabilities"),
                    "sessionCapabilities": capabilities.get("sessionCapabilities")
                });
                // 只选择原生明确公布的无交互认证，不读取凭据或代替用户配置模型后端。
                let method = ["cached_token", "xai.api_key"].into_iter().find(|id| {
                    result["authMethods"].as_array().is_some_and(|methods| {
                        methods
                            .iter()
                            .any(|method| method["id"].as_str() == Some(*id))
                    })
                });
                let Some(method) = method else {
                    return Err(RuntimeError::InvalidConfiguration(crate::t!(
                        "cli-agent-grok-managed-login-required"
                    )));
                };
                effects.writes.push(self.request(
                    PendingKind::Authenticate,
                    "authenticate",
                    json!({"methodId": method, "_meta": {"headless": true}}),
                ));
            }
            PendingKind::Authenticate => effects.writes.push(self.open_session()?),
            PendingKind::OpenSession { requested_id } => {
                let id = match requested_id {
                    Some(requested_id) => {
                        if [
                            result.get("sessionId"),
                            result["_meta"].get("sessionId"),
                            result["_meta"]["x.ai/sessionDetail"].get("sessionId"),
                        ]
                        .into_iter()
                        .flatten()
                        .any(|id| id.as_str() != Some(requested_id.as_str()))
                        {
                            return Err(RuntimeError::Protocol(
                                "Grok recovery changed the requested session id".into(),
                            ));
                        }
                        requested_id
                    }
                    None => result["sessionId"]
                        .as_str()
                        .filter(|id| valid_native_id(id))
                        .ok_or_else(|| {
                            RuntimeError::Protocol("missing Grok native session id".into())
                        })?
                        .to_owned(),
                };
                self.session_id = Some(id);
                self.update_metadata(result)?;
                effects.events.push(RuntimeEventKind::SessionReady {
                    effective_permissions: json!({
                        "requestedPolicy": "inherit", "effectiveNativePolicy": null,
                        "permissionEnforcementVerified": false,
                        "reportedCapabilities": self.reported_capabilities,
                        "reportedMetadata": self.reported_metadata,
                        "verifiedCapabilities": {
                            "newSession": true, "emptyHistoryRecovery": true, "closeSession": true,
                            "submit": false, "steer": false, "approval": false,
                            "cancel": false, "resume": false
                        }
                    }),
                });
            }
            PendingKind::Prompt => {
                effects = self.prompt_result(result)?;
                self.complete_rpc(&mut effects);
            }
            PendingKind::CloseSession => {
                if result["_meta"]["x.ai/closeOutcome"] != "closed" {
                    return Err(RuntimeError::Protocol(
                        "Grok session close was not confirmed".into(),
                    ));
                }
                self.closed = true;
            }
        }
        Ok(effects)
    }

    fn update_metadata(&mut self, value: &Value) -> Result<(), RuntimeError> {
        if let Some(models) = value.get("models") {
            self.reported_metadata.models = Some(decode_metadata(models)?);
        }
        if let Some(config) = value.get("configOptions") {
            self.reported_metadata.config_options = decode_metadata(config)?;
        }
        Ok(())
    }

    fn notification(&mut self, message: &Value) -> Result<Effects, RuntimeError> {
        let method = message["method"].as_str().unwrap_or_default();
        let params = &message["params"];
        // 模型列表是当前连接的无 sessionId 通知，其余更新必须匹配当前原生会话。
        if method == "_x.ai/models/update" {
            if params.get("sessionId").is_some()
                && params["sessionId"].as_str() != self.session_id.as_deref()
            {
                return Ok(Effects::default());
            }
            self.reported_metadata.models = Some(decode_metadata(params)?);
            return Ok(Effects::default());
        }
        if self.session_id.is_none() {
            if let Some(PendingRequest {
                kind:
                    PendingKind::OpenSession {
                        requested_id: Some(id),
                    },
                ..
            }) = &self.pending
            {
                if params["sessionId"].as_str() == Some(id.as_str()) {
                    // 历史回放只登记旧回合身份；不伪造输入确认、运行状态或新输出。
                    for id in [
                        params["_meta"]["promptId"].as_str(),
                        params["promptId"].as_str(),
                        params["update"]["prompt_id"].as_str(),
                        params["runningPromptId"].as_str(),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|id| valid_native_id(id))
                    {
                        self.observed_prompt_ids.insert(id.to_owned());
                    }
                    if let Some(entries) = params["entries"].as_array() {
                        for id in entries
                            .iter()
                            .filter_map(|entry| entry["id"].as_str())
                            .filter(|id| valid_native_id(id))
                        {
                            self.observed_prompt_ids.insert(id.to_owned());
                        }
                    }
                }
            }
            return Ok(Effects::default());
        }
        if params["sessionId"].as_str() != self.session_id.as_deref() {
            return Ok(Effects::default());
        }
        if params["_meta"]["isReplay"] == true {
            // 历史整段文本可能复用最后一个分片 ID，不能与当前增量输出混用。
            if let Some(id) = params["_meta"]["promptId"]
                .as_str()
                .filter(|id| valid_native_id(id))
            {
                self.observed_prompt_ids.insert(id.to_owned());
            }
            return Ok(Effects::default());
        }
        if let Some(event_id) = params["_meta"]["eventId"].as_str() {
            if !valid_native_id(event_id) {
                return Err(RuntimeError::Protocol(
                    "invalid Grok notification id".into(),
                ));
            }
            let fingerprint = message_fingerprint(message)?;
            if let Some(previous) = self.notification_ids.get(event_id) {
                return if previous == &fingerprint {
                    Ok(Effects::default())
                } else {
                    Err(RuntimeError::Protocol(
                        "conflicting Grok native notification id".into(),
                    ))
                };
            }
            self.notification_ids
                .insert(event_id.to_owned(), fingerprint);
        }
        match method {
            "_x.ai/queue/changed" => self.queue_changed(params),
            "_x.ai/session/prompt_complete" => Ok(self.prompt_complete(
                params["promptId"].as_str(),
                params["stopReason"].as_str(),
                params["agentResult"].as_str(),
            )),
            "_x.ai/session_notification" => {
                let update = &params["update"];
                if update["sessionUpdate"] == "turn_completed" {
                    return Ok(self.prompt_complete(
                        update["prompt_id"].as_str(),
                        update["stop_reason"].as_str(),
                        update["agent_result"].as_str(),
                    ));
                }
                Ok(Effects::default())
            }
            "session/update" => {
                let update = &params["update"];
                if update["sessionUpdate"] == "available_commands_update" {
                    self.reported_metadata.available_commands =
                        decode_metadata(&update["availableCommands"])?;
                }
                self.update_metadata(update)?;
                self.session_update(params)
            }
            // 未知扩展、working/空队列、摘要等不能驱动成功或取消。
            _ => Ok(Effects::default()),
        }
    }

    fn queue_changed(&mut self, params: &Value) -> Result<Effects, RuntimeError> {
        let entries = params["entries"]
            .as_array()
            .ok_or_else(|| RuntimeError::Protocol("invalid Grok queue snapshot".into()))?;
        let queued_id = if entries.len() == 1
            && entries[0]["kind"] == "prompt"
            && params.get("runningPromptId").is_none()
        {
            entries[0]["id"].as_str().filter(|id| valid_native_id(id))
        } else {
            None
        };
        let running_id = params["runningPromptId"]
            .as_str()
            .filter(|id| valid_native_id(id));
        let mut effects = Effects::default();
        if let Some(prompt) = self.prompt.as_mut().filter(|prompt| !prompt.finished) {
            if let Some(id) = queued_id.filter(|id| !self.observed_prompt_ids.contains(*id)) {
                if prompt.native_id.is_none() {
                    prompt.native_id = Some(id.to_owned());
                    effects.events.push(RuntimeEventKind::MessageAccepted {
                        message_id: prompt.message_id,
                        turn_id: Some(id.to_owned()),
                    });
                    effects.events.push(RuntimeEventKind::Progress {
                        turn_id: id.to_owned(),
                        message: crate::t!("cli-task-manager-state-queued"),
                    });
                }
            }
            if let Some(id) = running_id.filter(|id| Some(*id) == prompt.native_id.as_deref()) {
                if !prompt.started {
                    prompt.started = true;
                    effects.events.push(RuntimeEventKind::TurnStarted {
                        turn_id: id.to_owned(),
                    });
                    effects.events.push(RuntimeEventKind::Progress {
                        turn_id: id.to_owned(),
                        message: crate::t!("cli-task-manager-state-running"),
                    });
                }
            }
        }
        // version 是条目版本，时间戳也不是全局顺序；只记原生身份，绝不依提示词文本匹配。
        for entry in entries {
            if let Some(id) = entry["id"].as_str().filter(|id| valid_native_id(id)) {
                self.observed_prompt_ids.insert(id.to_owned());
            }
        }
        if let Some(id) = running_id {
            self.observed_prompt_ids.insert(id.to_owned());
        }
        Ok(effects)
    }

    fn prompt_complete(
        &mut self,
        native_id: Option<&str>,
        stop_reason: Option<&str>,
        output: Option<&str>,
    ) -> Effects {
        if native_id.is_none()
            || !self
                .prompt
                .as_ref()
                .is_some_and(|prompt| prompt.native_id.as_deref() == native_id)
        {
            return Effects::default();
        }
        if stop_reason == Some("error") {
            self.finish_prompt_failure(output.unwrap_or("Grok prompt failed").to_owned())
        } else {
            // 通知不是当前请求的 RPC 确认；成功或取消等待对应 RPC，未知原因也不推断终态。
            Effects::default()
        }
    }

    fn finish_prompt_failure(&mut self, detail: String) -> Effects {
        let Some(prompt) = self.prompt.as_mut().filter(|prompt| !prompt.finished) else {
            return Effects::default();
        };
        prompt.finished = true;
        let event = match &prompt.native_id {
            Some(id) => RuntimeEventKind::TurnFinished {
                turn_id: id.clone(),
                outcome: TurnOutcome::Failed {
                    message: detail.clone(),
                },
                output: detail,
            },
            None => RuntimeEventKind::RequestFailed {
                message_id: prompt.message_id,
                message: detail,
            },
        };
        let mut events = vec![event];
        self.cancel_approvals(&mut events);
        Effects {
            writes: Vec::new(),
            events,
        }
    }
}

fn cancelled_permission(id: &Value) -> Effects {
    Effects {
        writes: vec![
            json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": {"outcome": "cancelled"}}}),
        ],
        events: Vec::new(),
    }
}

fn rejected_command(message_id: Uuid, message: String) -> Effects {
    Effects {
        writes: Vec::new(),
        events: vec![RuntimeEventKind::RequestFailed {
            message_id,
            message,
        }],
    }
}

fn valid_native_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 4096 && !id.chars().any(char::is_control)
}

fn message_fingerprint(message: &Value) -> Result<[u8; 32], RuntimeError> {
    Ok(Sha256::digest(
        serde_json::to_vec(message).map_err(|error| RuntimeError::Protocol(error.to_string()))?,
    )
    .into())
}

fn decode_metadata<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, RuntimeError> {
    serde_json::from_value(value.clone())
        .map_err(|error| RuntimeError::Protocol(format!("invalid Grok display metadata: {error}")))
}

#[cfg(test)]
#[path = "grok_tests.rs"]
mod tests;
