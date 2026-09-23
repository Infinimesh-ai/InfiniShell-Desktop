//! Codex app-server 的本机管道适配；终态只来自原生回合事件。

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::future::Future;
use std::time::{Duration, Instant};

use command::Stdio;
use command::r#async::Command;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use futures::{FutureExt, pin_mut, select};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use uuid::Uuid;
use warpui::r#async::{FutureExt as _, Timer};

use super::local_tools::NativeLocalToolRequest;
use super::permissions::verify_effective_permissions;
use super::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeConnection, RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome, channels, local_tools,
};

const SUPPORTED_VERSIONS: [&str; 2] = ["0.147.0", "0.155.1"];
#[cfg(test)]
const TEST_CANDIDATE_VERSION: &str = "0.156.1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_MESSAGE_RECORDS: usize = 4096;
const PENDING_NATIVE_EXECUTION_ERROR: &str =
    "app-server ended the turn before every native execution reached a terminal state";

pub(crate) fn supported_version(version: &str) -> bool {
    SUPPORTED_VERSIONS.contains(&version)
}

pub fn connect(options: SessionOptions) -> Result<RuntimeConnection, RuntimeError> {
    connect_protocol(CodexProtocol::new(options))
}

fn connect_protocol(protocol: CodexProtocol) -> Result<RuntimeConnection, RuntimeError> {
    let options = &protocol.options;
    if options.grok_profile.is_some()
        || !options.executable.is_absolute()
        || !options.cwd.is_absolute()
    {
        return Err(RuntimeError::InvalidConfiguration(
            "executable and cwd must be absolute".into(),
        ));
    }
    if let SessionTarget::Resume { native_session_id } = &options.target
        && native_session_id.trim().is_empty()
    {
        return Err(RuntimeError::InvalidConfiguration(
            "resume requires a native session id".into(),
        ));
    }
    let (controller, commands, sender, events) = channels(options.generation);
    let task = Box::pin(async move {
        let mut protocol = protocol;
        let result = run_process(&mut protocol, commands, &sender).await;
        let reason = match &result {
            Ok(()) => "runtime connection closed".to_string(),
            Err(error) => error.to_string(),
        };
        // 满队列时不能阻塞控制循环；调用方也会通过 task 得到同一个失败。
        let _ = sender.try_send(protocol.event(RuntimeEventKind::Disconnected { reason }));
        result
    });
    Ok(RuntimeConnection {
        controller,
        events,
        task,
    })
}

async fn run_process(
    protocol: &mut CodexProtocol,
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
    let detected = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        return Err(RuntimeError::UnsupportedVersion(detected));
    }
    protocol.record_version_probe(&detected)?;

    let arguments = [OsString::from("app-server"), OsString::from("--stdio")];
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
        .ok_or_else(|| RuntimeError::Protocol("missing stdin".into()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| RuntimeError::Protocol("missing stdout".into()))?;
    let transport = run_transport(protocol, &mut stdin, &mut stdout, commands, events).await;
    drop(stdin);
    // 保留读端至有界 EOF；丢弃退出尾部字节，不生成新的原生事件或保存正文。
    let drain = async move {
        let mut budget = MAX_LINE_BYTES;
        let mut bytes = [0u8; 8192];
        loop {
            let limit = budget.saturating_add(1).min(bytes.len());
            let count = stdout.read(&mut bytes[..limit]).await?;
            if count == 0 {
                return Ok::<(), RuntimeError>(());
            }
            budget = budget.checked_sub(count).ok_or_else(|| {
                RuntimeError::Protocol(crate::t!("cli-agent-runtime-data-too-large"))
            })?;
        }
    };
    let graceful = matches!(&transport, Ok(TransportCompletion::Graceful));
    let finish = async move {
        if graceful {
            child.finish_after_stdin_close().await
        } else {
            child.finish().await
        }
    };
    let (finished, drained) = futures::join!(finish, drain.with_timeout(Duration::from_secs(30)));
    match transport {
        // 清理和读取均结束后保留原传输失败；失败路径绝不补发终态。
        Err(error) => Err(error),
        Ok(TransportCompletion::Graceful) => {
            finished?;
            drained.map_err(|_| RuntimeError::RequestTimedOut)??;
            Ok(())
        }
        Ok(TransportCompletion::CleanupThenPublish(terminal)) => {
            let cleanup = async move {
                let receipt = finished?;
                if !receipt.cleanup_confirmed {
                    return Err(RuntimeError::Protocol(
                        "managed process cleanup was not confirmed".into(),
                    ));
                }
                drained.map_err(|_| RuntimeError::RequestTimedOut)??;
                Ok(())
            };
            publish_terminal_after_cleanup(protocol, events, terminal, cleanup).await?;
            Err(RuntimeError::Protocol(
                "native execution outlived its turn; the managed process tree was cleaned".into(),
            ))
        }
    }
}

enum TransportCompletion {
    Graceful,
    CleanupThenPublish(RuntimeEventKind),
}

async fn run_transport(
    protocol: &mut CodexProtocol,
    stdin: &mut (impl AsyncWrite + Unpin),
    stdout: &mut (impl AsyncRead + Unpin),
    mut commands: mpsc::Receiver<RuntimeCommand>,
    events: &mpsc::Sender<RuntimeEvent>,
) -> Result<TransportCompletion, RuntimeError> {
    let initialize = protocol.initialize();
    write_message(stdin, &initialize).await?;
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
        match incoming {
            Incoming::Bytes(result) => {
                let count = result?;
                if count == 0 {
                    return Err(RuntimeError::Protocol(
                        "app-server stdout closed; delivery may be uncertain".into(),
                    ));
                }
                buffer.extend_from_slice(&chunk[..count]);
                while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
                    if newline > MAX_LINE_BYTES {
                        return Err(RuntimeError::Protocol(
                            "app-server message exceeds the size limit".into(),
                        ));
                    }
                    let line = buffer.drain(..=newline).collect::<Vec<_>>();
                    let message: Value = serde_json::from_slice(&line)
                        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
                    let effects = protocol.receive(message)?;
                    if let FlushOutcome::CleanupThenPublish(terminal) =
                        flush_effects(protocol, stdin, events, effects).await?
                    {
                        return Ok(TransportCompletion::CleanupThenPublish(terminal));
                    }
                }
                if buffer.len() > MAX_LINE_BYTES {
                    return Err(RuntimeError::Protocol(
                        "app-server message exceeds the size limit".into(),
                    ));
                }
            }
            Incoming::Command(Some(command)) => {
                let shutdown = command.action == RuntimeAction::Shutdown
                    && command.generation == protocol.options.generation;
                let effects = protocol.command(command);
                if let FlushOutcome::CleanupThenPublish(terminal) =
                    flush_effects(protocol, stdin, events, effects).await?
                {
                    return Ok(TransportCompletion::CleanupThenPublish(terminal));
                }
                if shutdown {
                    return Ok(TransportCompletion::Graceful);
                }
            }
            Incoming::Command(None) | Incoming::ConsumerClosed => {
                return Ok(TransportCompletion::Graceful);
            }
            Incoming::Tick => {}
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
    protocol: &CodexProtocol,
    stdin: &mut (impl AsyncWrite + Unpin),
    events: &mpsc::Sender<RuntimeEvent>,
    effects: Effects,
) -> Result<FlushOutcome, RuntimeError> {
    let Effects {
        writes,
        events: runtime_events,
        terminal_after_cleanup,
    } = effects;
    for message in writes {
        write_message(stdin, &message).await?;
    }
    for kind in runtime_events {
        events
            .try_send(protocol.event(kind))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => RuntimeError::EventBackpressure,
                mpsc::error::TrySendError::Closed(_) => RuntimeError::ControllerClosed,
            })?;
    }
    Ok(match terminal_after_cleanup {
        Some(terminal) => FlushOutcome::CleanupThenPublish(terminal),
        None => FlushOutcome::Continue,
    })
}

async fn publish_terminal_after_cleanup(
    protocol: &CodexProtocol,
    events: &mpsc::Sender<RuntimeEvent>,
    terminal: RuntimeEventKind,
    cleanup: impl Future<Output = Result<(), RuntimeError>>,
) -> Result<(), RuntimeError> {
    cleanup.await?;
    events
        .try_send(protocol.event(terminal))
        .map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => RuntimeError::EventBackpressure,
            mpsc::error::TrySendError::Closed(_) => RuntimeError::ControllerClosed,
        })
}

enum FlushOutcome {
    Continue,
    CleanupThenPublish(RuntimeEventKind),
}

#[derive(Default)]
struct Effects {
    writes: Vec<Value>,
    events: Vec<RuntimeEventKind>,
    terminal_after_cleanup: Option<RuntimeEventKind>,
}

#[derive(Clone)]
enum PendingKind {
    Initialize,
    OpenThread,
    Submit(Uuid),
    Steer { message_id: Uuid, turn_id: String },
    Interrupt { message_id: Uuid, turn_id: String },
}

struct PendingRequest {
    kind: PendingKind,
    sent_at: Instant,
}

struct MessageRecord {
    fingerprint: [u8; 32],
    response: Option<RuntimeEventKind>,
}

struct ActiveTurn {
    id: String,
    started: bool,
    final_messages: Vec<(String, String)>,
    native_executions: HashMap<String, String>,
}

struct PendingApproval {
    fingerprint: [u8; 32],
    native_id: Value,
    turn_id: String,
    response: Option<Value>,
}

struct PendingLocalTool {
    fingerprint: [u8; 32],
    request: NativeLocalToolRequest,
    response: Option<Value>,
}

struct CodexProtocol {
    options: SessionOptions,
    probed_version: Option<&'static str>,
    paired_version: Option<&'static str>,
    session_id: Option<String>,
    next_request_id: u64,
    pending: HashMap<u64, PendingRequest>,
    messages: HashMap<Uuid, MessageRecord>,
    active_turn: Option<ActiveTurn>,
    finished_turns: HashSet<String>,
    approvals: HashMap<String, PendingApproval>,
    local_tools: HashMap<String, PendingLocalTool>,
    #[cfg(test)]
    test_only_01561_profile: bool,
}

impl CodexProtocol {
    fn new(options: SessionOptions) -> Self {
        Self {
            options,
            probed_version: None,
            paired_version: None,
            session_id: None,
            next_request_id: 0,
            pending: HashMap::new(),
            messages: HashMap::new(),
            active_turn: None,
            finished_turns: HashSet::new(),
            approvals: HashMap::new(),
            local_tools: HashMap::new(),
            #[cfg(test)]
            test_only_01561_profile: false,
        }
    }

    fn record_version_probe(&mut self, detected: &str) -> Result<(), RuntimeError> {
        self.paired_version = None;
        self.probed_version = detected.strip_prefix("codex-cli ").and_then(|version| {
            #[cfg(test)]
            if self.test_only_01561_profile {
                return (version == TEST_CANDIDATE_VERSION).then_some(TEST_CANDIDATE_VERSION);
            }
            supported_version(version)
                .then(|| {
                    SUPPORTED_VERSIONS
                        .into_iter()
                        .find(|supported| *supported == version)
                })
                .flatten()
        });
        if self.probed_version.is_none() {
            return Err(RuntimeError::UnsupportedVersion(detected.to_owned()));
        }
        Ok(())
    }

    fn event(&self, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent {
            generation: self.options.generation,
            native_session_id: self.session_id.clone(),
            kind,
        }
    }

    fn request(&mut self, kind: PendingKind, method: &str, params: Value) -> Value {
        self.next_request_id += 1;
        let id = self.next_request_id;
        self.pending.insert(
            id,
            PendingRequest {
                kind,
                sent_at: Instant::now(),
            },
        );
        json!({"id": id, "method": method, "params": params})
    }

    fn initialize(&mut self) -> Value {
        let mut params =
            json!({"clientInfo": {"name": "infinishell", "version": env!("CARGO_PKG_VERSION")}});
        if self.options.local_tools.is_some() {
            params["capabilities"] = json!({"experimentalApi": true});
        }
        self.request(PendingKind::Initialize, "initialize", params)
    }

    fn request_timed_out(&self) -> bool {
        self.pending
            .values()
            .any(|request| request.sent_at.elapsed() >= REQUEST_TIMEOUT)
    }

    fn command(&mut self, command: RuntimeCommand) -> Effects {
        let message_id = command.message_id;
        if command.generation != self.options.generation {
            return failed(message_id, "the runtime generation is stale");
        }
        let fingerprint: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&command.action).expect("runtime actions are serializable"),
        )
        .into();
        if let Some(record) = self.messages.get(&message_id) {
            if record.fingerprint != fingerprint {
                return failed(
                    message_id,
                    "message id was already used for a different command",
                );
            }
            return Effects {
                writes: Vec::new(),
                events: record.response.clone().into_iter().collect(),
                terminal_after_cleanup: None,
            };
        }
        if self.messages.len() >= MAX_MESSAGE_RECORDS {
            return failed(
                message_id,
                "message deduplication limit reached; open a new connection without replaying delivered input",
            );
        }
        self.messages.insert(
            message_id,
            MessageRecord {
                fingerprint,
                response: None,
            },
        );
        let effects = self.apply_command(message_id, command.action);
        for event in &effects.events {
            self.remember_response(event);
        }
        effects
    }

    fn apply_command(&mut self, message_id: Uuid, action: RuntimeAction) -> Effects {
        if action == RuntimeAction::Shutdown {
            return accepted(message_id, None);
        }
        let Some(session_id) = self.session_id.clone() else {
            return failed(message_id, "session initialization has not completed");
        };
        match action {
            RuntimeAction::Submit { input } => {
                if self.active_turn.is_some()
                    || self
                        .pending
                        .values()
                        .any(|request| matches!(request.kind, PendingKind::Submit(_)))
                {
                    return failed(
                        message_id,
                        "a turn is already starting or running; use steering after TurnStarted",
                    );
                }
                let input = match encode_input(input) {
                    Ok(input) => input,
                    Err(error) => return failed(message_id, &error),
                };
                let request = self.request(PendingKind::Submit(message_id), "turn/start", json!({
                    "threadId": session_id, "clientUserMessageId": message_id.to_string(), "input": input,
                }));
                Effects {
                    writes: vec![request],
                    events: Vec::new(),
                    terminal_after_cleanup: None,
                }
            }
            RuntimeAction::Steer {
                expected_turn_id,
                input,
            } => {
                if !self.turn_is_started(&expected_turn_id) {
                    return failed(
                        message_id,
                        "steering requires the current turn's TurnStarted event",
                    );
                }
                let input = match encode_input(input) {
                    Ok(input) => input,
                    Err(error) => return failed(message_id, &error),
                };
                let request = self.request(
                    PendingKind::Steer {
                        message_id,
                        turn_id: expected_turn_id.clone(),
                    },
                    "turn/steer",
                    json!({
                        "threadId": session_id, "expectedTurnId": expected_turn_id,
                        "clientUserMessageId": message_id.to_string(), "input": input,
                    }),
                );
                Effects {
                    writes: vec![request],
                    events: Vec::new(),
                    terminal_after_cleanup: None,
                }
            }
            RuntimeAction::Interrupt { turn_id } => {
                if !self.turn_is_started(&turn_id) {
                    return failed(
                        message_id,
                        "interrupt requires the current turn's TurnStarted event",
                    );
                }
                let request = self.request(
                    PendingKind::Interrupt {
                        message_id,
                        turn_id: turn_id.clone(),
                    },
                    "turn/interrupt",
                    json!({"threadId": session_id, "turnId": turn_id}),
                );
                Effects {
                    writes: vec![request],
                    events: Vec::new(),
                    terminal_after_cleanup: None,
                }
            }
            RuntimeAction::RespondApproval {
                approval_id,
                decision,
            } => {
                let Some(approval) = self.approvals.get(&approval_id) else {
                    return failed(message_id, "approval is unknown or no longer pending");
                };
                if approval.response.is_some() || !self.turn_is_started(&approval.turn_id) {
                    return failed(
                        message_id,
                        "approval was already answered or belongs to an inactive turn",
                    );
                }
                let decision_value = match decision {
                    ApprovalDecision::AllowOnce => "accept",
                    ApprovalDecision::DenyOnce => "decline",
                };
                let response =
                    json!({"id": approval.native_id, "result": {"decision": decision_value}});
                self.approvals
                    .get_mut(&approval_id)
                    .expect("approval exists")
                    .response = Some(response.clone());
                Effects {
                    writes: vec![response],
                    events: vec![
                        RuntimeEventKind::ApprovalResolved {
                            approval_id,
                            decision,
                        },
                        RuntimeEventKind::CommandDispatched {
                            message_id,
                            turn_id: self.active_turn.as_ref().map(|turn| turn.id.clone()),
                        },
                    ],
                    terminal_after_cleanup: None,
                }
            }
            RuntimeAction::RespondLocalTool {
                turn_id,
                call_id,
                result,
            } => {
                let Some(call) = self.local_tools.get(&call_id) else {
                    return failed(message_id, "unknown local tool invocation");
                };
                if call.request.turn_id != turn_id
                    || !self.turn_is_started(&turn_id)
                    || call.response.is_some()
                {
                    return failed(
                        message_id,
                        "local tool invocation is no longer pending for this turn",
                    );
                }
                let response = local_tools::tool_reply(call.request.reply_target.clone(), result);
                self.local_tools
                    .get_mut(&call_id)
                    .expect("local tool exists")
                    .response = Some(response.clone());
                Effects {
                    writes: vec![response],
                    events: dispatched(message_id, Some(turn_id)).events,
                    terminal_after_cleanup: None,
                }
            }
            RuntimeAction::Shutdown => dispatched(message_id, None),
        }
    }

    fn turn_is_started(&self, turn_id: &str) -> bool {
        self.active_turn
            .as_ref()
            .is_some_and(|turn| turn.started && turn.id == turn_id)
    }

    fn remember_response(&mut self, event: &RuntimeEventKind) {
        let message_id = match event {
            RuntimeEventKind::MessageAccepted { message_id, .. }
            | RuntimeEventKind::CommandDispatched { message_id, .. }
            | RuntimeEventKind::RequestFailed { message_id, .. } => Some(message_id),
            RuntimeEventKind::SessionReady { .. }
            | RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::TurnStarted { .. }
            | RuntimeEventKind::TextDelta { .. }
            | RuntimeEventKind::Progress { .. }
            | RuntimeEventKind::ApprovalRequested { .. }
            | RuntimeEventKind::ApprovalResolved { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolRequested { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::TurnFinished { .. }
            | RuntimeEventKind::Disconnected { .. } => None,
        };
        if let Some(record) = message_id.and_then(|id| self.messages.get_mut(id)) {
            record.response = Some(event.clone());
        }
    }

    fn receive(&mut self, message: Value) -> Result<Effects, RuntimeError> {
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            let method = method.to_string();
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            if let Some(id) = message.get("id") {
                return self.server_request(&method, id.clone(), params);
            }
            return self.notification(&method, params);
        }
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            return Err(RuntimeError::Protocol(
                "response is missing a numeric request id".into(),
            ));
        };
        let Some(pending) = self.pending.remove(&id) else {
            // 重复或过时的响应不能重复确认消息，也不能重开已结束回合。
            return Ok(Effects::default());
        };
        if let Some(error) = message.get("error") {
            let error = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("app-server rejected the request");
            let message_id = match pending.kind {
                PendingKind::Initialize | PendingKind::OpenThread => {
                    return Err(RuntimeError::Protocol(error.to_string()));
                }
                PendingKind::Submit(message_id)
                | PendingKind::Steer { message_id, .. }
                | PendingKind::Interrupt { message_id, .. } => message_id,
            };
            let effects = failed(message_id, error);
            for event in &effects.events {
                self.remember_response(event);
            }
            return Ok(effects);
        }
        let result = message.get("result").ok_or_else(|| {
            RuntimeError::Protocol("response has neither result nor error".into())
        })?;
        let effects = match pending.kind {
            PendingKind::Initialize => {
                let user_agent = required_string(result, "/userAgent")?;
                // 只读取开头产品标识中的版本，不能把后文夹带的版本视为本次握手。
                let handshake_version = user_agent
                    .split_once('/')
                    .filter(|(product, _)| !product.trim().is_empty())
                    .and_then(|(_, remainder)| remainder.split_once(' '))
                    .map(|(version, _)| version);
                if self.probed_version.is_none() || handshake_version != self.probed_version {
                    return Err(RuntimeError::UnsupportedVersion(user_agent));
                }
                self.paired_version = self.probed_version;
                let mut params = json!({"cwd": self.options.cwd});
                if let Some(model) = &self.options.model {
                    params["model"] = json!(model);
                }
                match self.options.permission_policy {
                    PermissionPolicy::ClaudeRestrictedFilesV1
                    | PermissionPolicy::GrokRestrictedReadV1
                    | PermissionPolicy::GrokRestrictedFilesV1 => {
                        return Err(RuntimeError::InvalidConfiguration(
                            "Claude fixed file policy cannot be applied to Codex".into(),
                        ));
                    }
                    PermissionPolicy::Inherit => {}
                    PermissionPolicy::ReadOnly => {
                        params["approvalPolicy"] = json!("untrusted");
                        params["approvalsReviewer"] = json!("user");
                        params["sandbox"] = json!("read-only");
                    }
                    PermissionPolicy::WorkspaceWrite => {
                        params["approvalPolicy"] = json!("untrusted");
                        params["approvalsReviewer"] = json!("user");
                        params["sandbox"] = json!("workspace-write");
                    }
                }
                if let Some(permissions) = self.options.local_tools
                    && self.options.target == SessionTarget::New
                {
                    // 受测版本在原生会话中保存工具；恢复使用已存定义，不能向 resume 填入未知字段。
                    params["dynamicTools"] = json!(local_tools::codex_dynamic_tools(
                        permissions.allow_spawn,
                        permissions.allow_message
                    ));
                }
                let method = match &self.options.target {
                    SessionTarget::New => "thread/start",
                    SessionTarget::Resume { native_session_id } => {
                        params["threadId"] = json!(native_session_id);
                        "thread/resume"
                    }
                };
                let request = self.request(PendingKind::OpenThread, method, params);
                Effects {
                    writes: vec![json!({"method": "initialized"}), request],
                    events: Vec::new(),
                    terminal_after_cleanup: None,
                }
            }
            PendingKind::OpenThread => {
                let version = self.paired_version.ok_or_else(|| {
                    RuntimeError::Protocol("Codex session has no paired CLI version".into())
                })?;
                let session_id = required_string(result, "/thread/id")?;
                if let SessionTarget::Resume { native_session_id } = &self.options.target
                    && *native_session_id != session_id
                {
                    return Err(RuntimeError::Protocol(
                        "resume returned a different native session id".into(),
                    ));
                }
                if result
                    .pointer("/thread/status/type")
                    .and_then(Value::as_str)
                    == Some("active")
                    || result
                        .pointer("/thread/canAcceptDirectInput")
                        .and_then(Value::as_bool)
                        == Some(false)
                {
                    return Err(RuntimeError::Protocol("resume target is active or cannot accept direct input; reattach its owning runtime".into()));
                }
                let expected_sandbox = match self.options.permission_policy {
                    PermissionPolicy::ClaudeRestrictedFilesV1
                    | PermissionPolicy::GrokRestrictedReadV1
                    | PermissionPolicy::GrokRestrictedFilesV1 => {
                        return Err(RuntimeError::InvalidConfiguration(
                            "Claude fixed file policy cannot be applied to Codex".into(),
                        ));
                    }
                    PermissionPolicy::Inherit => None,
                    PermissionPolicy::ReadOnly => Some("readOnly"),
                    PermissionPolicy::WorkspaceWrite => Some("workspaceWrite"),
                };
                if let Some(expected_sandbox) = expected_sandbox
                    && (result.pointer("/sandbox/type").and_then(Value::as_str)
                        != Some(expected_sandbox)
                        || result.get("approvalPolicy").and_then(Value::as_str)
                            != Some("untrusted")
                        || result.get("approvalsReviewer").and_then(Value::as_str) != Some("user"))
                {
                    return Err(RuntimeError::Protocol(
                        "app-server did not apply the requested permission policy".into(),
                    ));
                }
                let effective_permissions = json!({
                    "approvalPolicy": result.get("approvalPolicy"), "sandbox": result.get("sandbox"), "approvalsReviewer": result.get("approvalsReviewer"),
                });
                verify_effective_permissions(
                    self.options.permission_ceiling.as_ref(),
                    "codex",
                    &self.options.cwd,
                    &effective_permissions,
                )?;
                self.session_id = Some(session_id);
                Effects {
                    writes: Vec::new(),
                    events: vec![RuntimeEventKind::SessionReady {
                        verified_cli_version: Some(version.to_owned()),
                        effective_permissions,
                    }],
                    terminal_after_cleanup: None,
                }
            }
            PendingKind::Submit(message_id) => {
                let turn_id = required_string(result, "/turn/id")?;
                if !self.finished_turns.contains(&turn_id) {
                    if let Some(active) = &self.active_turn {
                        if active.id != turn_id {
                            return Err(RuntimeError::Protocol(
                                "turn/start returned a different active turn".into(),
                            ));
                        }
                    } else {
                        self.active_turn = Some(ActiveTurn {
                            id: turn_id.clone(),
                            started: false,
                            final_messages: Vec::new(),
                            native_executions: HashMap::new(),
                        });
                    }
                }
                accepted(message_id, Some(turn_id))
            }
            PendingKind::Steer {
                message_id,
                turn_id,
            } => {
                let returned_turn = required_string(result, "/turnId")?;
                if returned_turn != turn_id {
                    return Err(RuntimeError::Protocol(
                        "steer acknowledged a different turn".into(),
                    ));
                }
                accepted(message_id, Some(turn_id))
            }
            PendingKind::Interrupt {
                message_id,
                turn_id,
            } => accepted(message_id, Some(turn_id)),
        };
        for event in &effects.events {
            self.remember_response(event);
        }
        Ok(effects)
    }

    fn receive_local_tool(&mut self, message: Value) -> Result<Effects, RuntimeError> {
        let reject = |reason: &str| Effects {
            writes: vec![json!({"id":message["id"], "error":{"code":-32601,"message":reason}})],
            events: Vec::new(),
            terminal_after_cleanup: None,
        };
        let Some(permissions) = self.options.local_tools else {
            return Ok(reject(
                "local task tools were not authorized for this connection",
            ));
        };
        let Some(turn) = self.active_turn.as_ref().filter(|turn| turn.started) else {
            return Ok(reject("local task tools require an active turn"));
        };
        let request = match local_tools::codex_tool_request(
            &message,
            self.session_id.as_deref().unwrap_or_default(),
            &turn.id,
        ) {
            Ok(request) => request,
            Err(reason) => return Ok(reject(&reason)),
        };
        let fingerprint: [u8; 32] =
            Sha256::digest(serde_json::to_vec(&message).expect("JSON values are serializable"))
                .into();
        if let Some(previous) = self.local_tools.get(&request.call_id) {
            if previous.fingerprint != fingerprint {
                return Ok(reject(
                    "local tool call id was reused with different content",
                ));
            }
            return Ok(Effects {
                writes: previous.response.clone().into_iter().collect(),
                events: Vec::new(),
                terminal_after_cleanup: None,
            });
        }
        if self
            .local_tools
            .values()
            .any(|call| call.request.reply_target == request.reply_target)
            || self
                .approvals
                .values()
                .any(|approval| approval.native_id == message["id"])
        {
            return Ok(reject("native request id was already used"));
        }
        if !local_tools::tool_definitions(permissions.allow_spawn, permissions.allow_message)
            .iter()
            .any(|tool| tool["name"] == request.tool)
        {
            return Ok(reject("local task tool is not authorized"));
        }
        if self.local_tools.len() >= MAX_MESSAGE_RECORDS {
            return Ok(reject("local tool history limit reached"));
        }
        self.local_tools.insert(
            request.call_id.clone(),
            PendingLocalTool {
                fingerprint,
                request: request.clone(),
                response: None,
            },
        );
        Ok(Effects {
            writes: Vec::new(),
            events: vec![RuntimeEventKind::LocalToolRequested { request }],
            terminal_after_cleanup: None,
        })
    }

    fn server_request(
        &mut self,
        method: &str,
        id: Value,
        params: Value,
    ) -> Result<Effects, RuntimeError> {
        if method == "item/tool/call" {
            return self.receive_local_tool(json!({"method": method, "id": id, "params": params}));
        }
        let reject = |message: &str| Effects {
            writes: vec![json!({"id": id, "error": {"code": -32601, "message": message}})],
            events: Vec::new(),
            terminal_after_cleanup: None,
        };
        if !matches!(
            method,
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
        ) {
            return Ok(reject(
                "InfiniShell does not support this app-server request",
            ));
        }
        if self.local_tools.values().any(|call| matches!(&call.request.reply_target, local_tools::LocalToolReplyTarget::Codex { request_id } if *request_id == id)) {
            return Ok(reject("native request id was already used for a local tool"));
        }
        if !id.is_string() && !id.is_number() {
            return Ok(reject("Unsupported approval request id"));
        }
        let turn_id = required_string(&params, "/turnId")?;
        if params.get("threadId").and_then(Value::as_str) != self.session_id.as_deref()
            || !self.turn_is_started(&turn_id)
        {
            return Ok(reject("Approval does not belong to the active turn"));
        }
        let approval_id = format!("{turn_id}:{id}");
        let fingerprint: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&json!([method, params])).expect("JSON values are serializable"),
        )
        .into();
        if let Some(approval) = self.approvals.get(&approval_id) {
            if approval.fingerprint != fingerprint {
                return Ok(reject(
                    "Approval request id was reused with different content",
                ));
            }
            return Ok(Effects {
                writes: approval.response.clone().into_iter().collect(),
                events: Vec::new(),
                terminal_after_cleanup: None,
            });
        }
        if self.approvals.len() >= MAX_MESSAGE_RECORDS {
            return Ok(reject("Approval history limit reached"));
        }
        self.approvals.insert(
            approval_id.clone(),
            PendingApproval {
                fingerprint,
                native_id: id,
                turn_id: turn_id.clone(),
                response: None,
            },
        );
        Ok(Effects {
            writes: Vec::new(),
            events: vec![RuntimeEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                method: method.to_string(),
                details: params,
            }],
            terminal_after_cleanup: None,
        })
    }

    fn notification(&mut self, method: &str, params: Value) -> Result<Effects, RuntimeError> {
        if params.get("threadId").and_then(Value::as_str) != self.session_id.as_deref()
            || self.session_id.is_none()
        {
            return Ok(Effects::default());
        }
        match method {
            "turn/started" => {
                let turn_id = required_string(&params, "/turn/id")?;
                if self.finished_turns.contains(&turn_id) {
                    return Ok(Effects::default());
                }
                if let Some(active) = &mut self.active_turn {
                    if active.id != turn_id || active.started {
                        return Ok(Effects::default());
                    }
                    active.started = true;
                } else if self
                    .pending
                    .values()
                    .any(|request| matches!(request.kind, PendingKind::Submit(_)))
                {
                    self.active_turn = Some(ActiveTurn {
                        id: turn_id.clone(),
                        started: true,
                        final_messages: Vec::new(),
                        native_executions: HashMap::new(),
                    });
                } else {
                    return Ok(Effects::default());
                }
                Ok(Effects {
                    writes: Vec::new(),
                    events: vec![RuntimeEventKind::TurnStarted { turn_id }],
                    terminal_after_cleanup: None,
                })
            }
            "item/agentMessage/delta" => {
                let turn_id = required_string(&params, "/turnId")?;
                if !self.turn_is_started(&turn_id) {
                    return Ok(Effects::default());
                }
                Ok(Effects {
                    writes: Vec::new(),
                    events: vec![RuntimeEventKind::TextDelta {
                        turn_id,
                        item_id: required_string(&params, "/itemId")?,
                        text: required_string(&params, "/delta")?,
                    }],
                    terminal_after_cleanup: None,
                })
            }
            "error" => {
                let turn_id = required_string(&params, "/turnId")?;
                if !self.turn_is_started(&turn_id) {
                    return Ok(Effects::default());
                }
                let message = required_string(&params, "/error/message")?;
                match params.get("willRetry").and_then(Value::as_bool) {
                    Some(true) => Ok(Effects {
                        writes: Vec::new(),
                        events: vec![RuntimeEventKind::Progress { turn_id, message }],
                        terminal_after_cleanup: None,
                    }),
                    Some(false) => {
                        let active = self.active_turn.take().expect("active turn exists");
                        let pending_native_execution = !active.native_executions.is_empty();
                        self.finished_turns.insert(turn_id.clone());
                        Ok(terminal_effects(
                            turn_id,
                            TurnOutcome::Failed { message },
                            String::new(),
                            pending_native_execution,
                        ))
                    }
                    None => Err(RuntimeError::Protocol(
                        "error notification is missing willRetry".into(),
                    )),
                }
            }
            "item/started" => {
                let turn_id = required_string(&params, "/turnId")?;
                if !self.turn_is_started(&turn_id) {
                    return Ok(Effects::default());
                }
                let item_type = params
                    .pointer("/item/type")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if is_native_execution_type(item_type) {
                    let item_id = required_string(&params, "/item/id")?;
                    let native_executions = &mut self
                        .active_turn
                        .as_mut()
                        .expect("active turn exists")
                        .native_executions;
                    if native_executions
                        .get(&item_id)
                        .is_some_and(|started_type| started_type != item_type)
                    {
                        return Err(RuntimeError::Protocol(
                            "native execution item id was reused with a different type".into(),
                        ));
                    }
                    native_executions.insert(item_id, item_type.to_string());
                    let message = params
                        .pointer("/item/command")
                        .and_then(Value::as_str)
                        .unwrap_or(item_type)
                        .to_string();
                    return Ok(Effects {
                        writes: Vec::new(),
                        events: vec![RuntimeEventKind::Progress { turn_id, message }],
                        terminal_after_cleanup: None,
                    });
                }
                Ok(Effects::default())
            }
            "item/completed" => {
                let turn_id = required_string(&params, "/turnId")?;
                let item_type = params
                    .pointer("/item/type")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if self.turn_is_started(&turn_id) && is_native_execution_type(item_type) {
                    let item_id = required_string(&params, "/item/id")?;
                    let native_executions = &mut self
                        .active_turn
                        .as_mut()
                        .expect("active turn exists")
                        .native_executions;
                    if native_executions
                        .get(&item_id)
                        .is_some_and(|started_type| started_type != item_type)
                    {
                        return Err(RuntimeError::Protocol(
                            "native execution completed with a different item type".into(),
                        ));
                    }
                    native_executions.remove(&item_id);
                }
                if self.turn_is_started(&turn_id)
                    && matches!(
                        item_type,
                        "commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall"
                    )
                {
                    // 保留原生执行状态与错误细节，最终成功仍只由 turn/completed 决定。
                    let message = serde_json::to_string(&params["item"])
                        .expect("JSON values are serializable");
                    return Ok(Effects {
                        writes: Vec::new(),
                        events: vec![RuntimeEventKind::Progress {
                            turn_id,
                            message: message.chars().take(16_384).collect(),
                        }],
                        terminal_after_cleanup: None,
                    });
                }
                if self.turn_is_started(&turn_id)
                    && params.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage")
                    && params.pointer("/item/phase").and_then(Value::as_str) != Some("commentary")
                {
                    let id = required_string(&params, "/item/id")?;
                    let text = required_string(&params, "/item/text")?;
                    let messages = &mut self
                        .active_turn
                        .as_mut()
                        .expect("active turn exists")
                        .final_messages;
                    if let Some(message) = messages.iter_mut().find(|(existing, _)| *existing == id)
                    {
                        message.1 = text;
                    } else {
                        messages.push((id, text));
                    }
                }
                Ok(Effects::default())
            }
            "turn/completed" => {
                let turn_id = required_string(&params, "/turn/id")?;
                if self.finished_turns.contains(&turn_id)
                    || !self
                        .active_turn
                        .as_ref()
                        .is_some_and(|turn| turn.id == turn_id)
                {
                    return Ok(Effects::default());
                }
                let active = self.active_turn.take().expect("active turn exists");
                let turn = params.get("turn").expect("turn id was parsed");
                let outcome = turn_outcome(turn)?;
                let pending_native_execution = !active.native_executions.is_empty();
                let output = final_output(turn).unwrap_or_else(|| {
                    active
                        .final_messages
                        .into_iter()
                        .map(|(_, text)| text)
                        .collect::<Vec<_>>()
                        .join("\n")
                });
                self.finished_turns.insert(turn_id.clone());
                Ok(terminal_effects(
                    turn_id,
                    outcome,
                    output,
                    pending_native_execution,
                ))
            }
            // app-server 同时发送旧版 codex/event 通道，只消费上面的 v2 生命周期，避免双报。
            _ => Ok(Effects::default()),
        }
    }
}

fn terminal_effects(
    turn_id: String,
    outcome: TurnOutcome,
    output: String,
    pending_native_execution: bool,
) -> Effects {
    let outcome = if pending_native_execution && outcome != TurnOutcome::Cancelled {
        TurnOutcome::Failed {
            message: PENDING_NATIVE_EXECUTION_ERROR.into(),
        }
    } else {
        outcome
    };
    let terminal = RuntimeEventKind::TurnFinished {
        turn_id,
        outcome,
        output,
    };
    if pending_native_execution {
        // 原生回合终态不能证明仍在运行的工具已退出；由调用层先收回托管进程树，
        // 并核验同代退出回执，再发布这里保留的终态。
        Effects {
            writes: Vec::new(),
            events: Vec::new(),
            terminal_after_cleanup: Some(terminal),
        }
    } else {
        Effects {
            writes: Vec::new(),
            events: vec![terminal],
            terminal_after_cleanup: None,
        }
    }
}

fn required_string(value: &Value, pointer: &str) -> Result<String, RuntimeError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| RuntimeError::Protocol(format!("missing string field {pointer}")))
}

fn is_native_execution_type(item_type: &str) -> bool {
    matches!(item_type, "commandExecution" | "fileChange" | "mcpToolCall")
}

fn encode_input(input: Vec<InputContent>) -> Result<Vec<Value>, String> {
    let mut length = 0;
    let encoded = input
        .into_iter()
        .map(|content| match content {
            InputContent::Text(text) => {
                length += text.len();
                Ok(json!({"type": "text", "text": text}))
            }
            InputContent::Skill { name, path } => {
                if name.trim().is_empty() || !path.is_absolute() {
                    return Err("skills require a name and an absolute local path".into());
                }
                length += name.len() + path.as_os_str().len();
                Ok(json!({"type": "skill", "name": name, "path": path}))
            }
            InputContent::LocalImage(path) => {
                if !path.is_absolute() {
                    return Err("image paths must be absolute".to_string());
                }
                length += path.as_os_str().len();
                Ok(json!({"type": "localImage", "path": path}))
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    if encoded.is_empty() || length == 0 || length > MAX_INPUT_BYTES {
        return Err("input is empty or exceeds the size limit".into());
    }
    Ok(encoded)
}

fn turn_outcome(turn: &Value) -> Result<TurnOutcome, RuntimeError> {
    if let Some(error) = turn.get("error").filter(|error| !error.is_null()) {
        return Ok(TurnOutcome::Failed {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Codex turn failed")
                .to_string(),
        });
    }
    match turn.get("status").and_then(Value::as_str) {
        Some("completed") => Ok(TurnOutcome::Completed),
        Some("interrupted") => Ok(TurnOutcome::Cancelled),
        Some("failed") => Ok(TurnOutcome::Failed {
            message: "Codex turn failed".to_string(),
        }),
        status => Err(RuntimeError::Protocol(format!(
            "unexpected terminal turn status: {status:?}"
        ))),
    }
}

fn final_output(turn: &Value) -> Option<String> {
    let messages = turn
        .get("items")?
        .as_array()?
        .iter()
        .filter(|item| {
            item.get("type").and_then(Value::as_str) == Some("agentMessage")
                && item.get("phase").and_then(Value::as_str) != Some("commentary")
        })
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>();
    (!messages.is_empty()).then(|| messages.join("\n"))
}

fn failed(message_id: Uuid, message: &str) -> Effects {
    Effects {
        writes: Vec::new(),
        events: vec![RuntimeEventKind::RequestFailed {
            message_id,
            message: message.to_string(),
        }],
        terminal_after_cleanup: None,
    }
}

fn accepted(message_id: Uuid, turn_id: Option<String>) -> Effects {
    Effects {
        writes: Vec::new(),
        events: vec![RuntimeEventKind::MessageAccepted {
            message_id,
            turn_id,
        }],
        terminal_after_cleanup: None,
    }
}

fn dispatched(message_id: Uuid, turn_id: Option<String>) -> Effects {
    Effects {
        writes: Vec::new(),
        events: vec![RuntimeEventKind::CommandDispatched {
            message_id,
            turn_id,
        }],
        terminal_after_cleanup: None,
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "codex_live_tests.rs"]
mod live_tests;
