//! Claude Code 2.1.273 双向 stream-json；输入排队与同回合 steering 保持区别。

use std::collections::HashMap;
use std::ffi::OsString;
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

use super::local_skills::{PreparedClaudeSkillPlugin, prepare_claude_skill_plugin};
use super::local_tools::{ClaudeMcpRequest, NativeLocalToolRequest};
use super::permissions::verify_effective_permissions;
use super::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeConnection, RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome, channels, local_tools,
};

#[path = "claude_permission_snapshot.rs"]
mod permission_snapshot;

use permission_snapshot::{Observation, Rejection};

const PERMISSION_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(5);
const VERIFIED_VERSION: &str = "2.1.273 (Claude Code)";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_MESSAGE_RECORDS: usize = 4096;

pub fn connect(options: SessionOptions) -> Result<RuntimeConnection, RuntimeError> {
    if !options.executable.is_absolute() || !options.cwd.is_absolute() {
        return Err(RuntimeError::InvalidConfiguration(
            "executable and cwd must be absolute".into(),
        ));
    }
    validate_options(&options)?;
    let (controller, commands, sender, events) = channels(options.generation);
    let task = Box::pin(async move {
        let mut protocol = ClaudeProtocol::new(options);
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
    protocol: &mut ClaudeProtocol,
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
    if !output.status.success() || detected != VERIFIED_VERSION {
        return Err(RuntimeError::UnsupportedVersion(detected));
    }

    protocol.skill_plugin = prepare_claude_skill_plugin(&protocol.options.selected_skills)
        .map_err(RuntimeError::InvalidConfiguration)?;
    let mut arguments: Vec<OsString> = launch_arguments(&protocol.options)
        .into_iter()
        .map(OsString::from)
        .collect();
    if let Some(plugin) = &protocol.skill_plugin {
        arguments.push(OsString::from("--plugin-dir"));
        arguments.push(plugin.plugin_directory().as_os_str().to_owned());
    }
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
    let result = run_transport(protocol, &mut stdin, &mut stdout, commands, events).await;
    drop(stdin);
    drop(stdout);
    child.finish().await?;
    result
}

async fn run_transport(
    protocol: &mut ClaudeProtocol,
    stdin: &mut (impl AsyncWrite + Unpin),
    stdout: &mut (impl AsyncRead + Unpin),
    mut commands: mpsc::Receiver<RuntimeCommand>,
    events: &mpsc::Sender<RuntimeEvent>,
) -> Result<(), RuntimeError> {
    let initialize = protocol.initialize();
    write_message(stdin, &initialize).await?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let effects = protocol.expire_permission_observation();
        flush_effects(protocol, stdin, events, effects).await?;
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
                        "Claude stream-json stdout closed; delivery may be uncertain".into(),
                    ));
                }
                buffer.extend_from_slice(&chunk[..count]);
                while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
                    if newline > MAX_LINE_BYTES {
                        return Err(RuntimeError::Protocol(
                            "Claude stream-json message exceeds the size limit".into(),
                        ));
                    }
                    let line = buffer.drain(..=newline).collect::<Vec<_>>();
                    let message: Value = serde_json::from_slice(&line)
                        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
                    let effects = protocol.receive(message)?;
                    flush_effects(protocol, stdin, events, effects).await?;
                }
                if buffer.len() > MAX_LINE_BYTES {
                    return Err(RuntimeError::Protocol(
                        "Claude stream-json message exceeds the size limit".into(),
                    ));
                }
            }
            Incoming::Command(Some(command)) => {
                let shutdown = command.action == RuntimeAction::Shutdown
                    && command.generation == protocol.options.generation;
                let effects = protocol.command(command);
                flush_effects(protocol, stdin, events, effects).await?;
                if shutdown {
                    return Ok(());
                }
            }
            Incoming::Command(None) | Incoming::ConsumerClosed => return Ok(()),
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
    protocol: &ClaudeProtocol,
    stdin: &mut (impl AsyncWrite + Unpin),
    events: &mpsc::Sender<RuntimeEvent>,
    effects: Effects,
) -> Result<(), RuntimeError> {
    for message in effects.writes {
        write_message(stdin, &message).await?;
    }
    for kind in effects.events {
        events
            .try_send(protocol.event(kind))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => RuntimeError::EventBackpressure,
                mpsc::error::TrySendError::Closed(_) => RuntimeError::ControllerClosed,
            })?;
    }
    Ok(())
}

fn validate_options(options: &SessionOptions) -> Result<(), RuntimeError> {
    if options.permission_policy != PermissionPolicy::Inherit {
        return Err(RuntimeError::InvalidConfiguration(
            "Claude permission modes are not equivalent to the requested filesystem sandbox".into(),
        ));
    }
    if let SessionTarget::Resume { native_session_id } = &options.target {
        Uuid::parse_str(native_session_id).map_err(|_| {
            RuntimeError::InvalidConfiguration(
                "Claude resume requires an exact session UUID".into(),
            )
        })?;
    }
    Ok(())
}

fn launch_arguments(options: &SessionOptions) -> Vec<String> {
    let mut arguments = [
        "--print",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--replay-user-messages",
        "--permission-prompt-tool",
        "stdio",
        "--permission-prompts",
        "host",
    ]
    .map(str::to_string)
    .to_vec();
    if let SessionTarget::Resume { native_session_id } = &options.target {
        // 可选参数使用等号绑定，避免值被 CLI 当成另一项开关。
        arguments.push(format!("--resume={native_session_id}"));
    }
    if let Some(model) = &options.model {
        arguments.push(format!("--model={model}"));
    }
    if options.local_tools.is_some() {
        arguments.push(format!("--mcp-config={}", local_tools::claude_mcp_config()));
    }
    arguments
}

#[derive(Default)]
struct Effects {
    writes: Vec<Value>,
    events: Vec<RuntimeEventKind>,
}

#[derive(Clone, Copy)]
enum PermissionStage {
    Settings,
    Rules,
    RecheckMode,
}

enum PendingKind {
    Initialize,
    PermissionObservation(PermissionStage),
    Interrupt { message_id: Uuid, turn_id: Uuid },
}

struct PendingRequest {
    kind: PendingKind,
    sent_at: Instant,
}

struct MessageRecord {
    fingerprint: [u8; 32],
    response: Option<RuntimeEventKind>,
}

struct UserTurn {
    expected_replay: String,
    sent_at: Instant,
    accepted: bool,
    started: bool,
    finished: bool,
    output: String,
    error: Option<String>,
}

struct PendingApproval {
    fingerprint: [u8; 32],
    turn_id: Uuid,
    input: Value,
    response: Option<Value>,
    cancelled: bool,
}

struct PendingLocalTool {
    fingerprint: [u8; 32],
    request: NativeLocalToolRequest,
    response: Option<Value>,
    cancelled: bool,
}

struct ClaudeProtocol {
    options: SessionOptions,
    initialized: bool,
    session_id: Option<String>,
    next_request_id: u64,
    pending: HashMap<String, PendingRequest>,
    messages: HashMap<Uuid, MessageRecord>,
    turns: HashMap<Uuid, UserTurn>,
    active_turn: Option<Uuid>,
    approvals: HashMap<String, PendingApproval>,
    seen_events: HashMap<String, [u8; 32]>,
    local_tools: HashMap<String, PendingLocalTool>,
    mcp_replies: HashMap<String, ([u8; 32], Value)>,
    skill_plugin: Option<PreparedClaudeSkillPlugin>,
    permission_observation: Option<Observation>,
    permission_observation_started: Option<Instant>,
    ready_permissions: Value,
}

impl ClaudeProtocol {
    fn new(options: SessionOptions) -> Self {
        Self {
            options,
            initialized: false,
            session_id: None,
            next_request_id: 0,
            pending: HashMap::new(),
            messages: HashMap::new(),
            turns: HashMap::new(),
            active_turn: None,
            approvals: HashMap::new(),
            seen_events: HashMap::new(),
            local_tools: HashMap::new(),
            mcp_replies: HashMap::new(),
            skill_plugin: None,
            permission_observation: None,
            permission_observation_started: None,
            ready_permissions: Value::Null,
        }
    }

    fn event(&self, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent {
            generation: self.options.generation,
            native_session_id: self.session_id.clone(),
            kind,
        }
    }

    fn request(&mut self, kind: PendingKind, request: Value) -> Value {
        self.next_request_id += 1;
        let request_id = format!(
            "infinishell-{}-{}",
            self.options.generation, self.next_request_id
        );
        self.pending.insert(
            request_id.clone(),
            PendingRequest {
                kind,
                sent_at: Instant::now(),
            },
        );
        json!({"type":"control_request", "request_id":request_id, "request":request})
    }

    fn initialize(&mut self) -> Value {
        self.request(PendingKind::Initialize, json!({"subtype":"initialize"}))
    }

    fn request_timed_out(&self) -> bool {
        self.pending.values().any(|request| {
            !matches!(request.kind, PendingKind::PermissionObservation(_))
                && request.sent_at.elapsed() >= REQUEST_TIMEOUT
        }) || self.turns.values().any(|turn| {
            !turn.accepted && !turn.finished && turn.sent_at.elapsed() >= REQUEST_TIMEOUT
        })
    }

    fn start_permission_observation(&mut self, initialize: &Value) -> Effects {
        self.permission_observation = Some(Observation::new(self.options.generation, initialize));
        self.permission_observation_started = Some(Instant::now());
        Effects {
            writes: vec![self.request(
                PendingKind::PermissionObservation(PermissionStage::Settings),
                json!({"subtype":"get_settings"}),
            )],
            events: Vec::new(),
        }
    }

    fn finish_permission_observation(&mut self, rejection: Option<Rejection>) -> Effects {
        if let Some(rejection) = rejection
            && let Some(observation) = &mut self.permission_observation
        {
            observation.reject(rejection);
        }
        self.permission_observation_started = None;
        self.pending
            .retain(|_, request| !matches!(request.kind, PendingKind::PermissionObservation(_)));
        self.initialized = true;
        Effects {
            writes: Vec::new(),
            events: vec![self.ready_event()],
        }
    }

    fn ready_event(&self) -> RuntimeEventKind {
        let mut effective_permissions = self.ready_permissions.clone();
        if let Some(observation) = &self.permission_observation {
            effective_permissions["permissionObservation"] = observation.value();
        }
        RuntimeEventKind::SessionReady {
            effective_permissions,
        }
    }

    fn expire_permission_observation(&mut self) -> Effects {
        if self
            .permission_observation_started
            .is_some_and(|started| started.elapsed() >= PERMISSION_OBSERVATION_TIMEOUT)
        {
            // 查询失败不替用户变更 Inherit 策略，也不能永久阻止正常对话启动。
            self.finish_permission_observation(Some(Rejection::TimedOut))
        } else {
            Effects::default()
        }
    }

    fn remember_response(&mut self, event: &RuntimeEventKind) {
        if let RuntimeEventKind::MessageAccepted { message_id, .. }
        | RuntimeEventKind::CommandDispatched { message_id, .. }
        | RuntimeEventKind::RequestFailed { message_id, .. } = event
            && let Some(record) = self.messages.get_mut(message_id)
        {
            record.response = Some(event.clone());
        }
    }

    fn command(&mut self, command: RuntimeCommand) -> Effects {
        let message_id = command.message_id;
        if command.generation != self.options.generation {
            return failed(message_id, "the runtime generation is stale");
        }
        let fingerprint = fingerprint(
            &serde_json::to_value(&command.action).expect("runtime actions are serializable"),
        );
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
            };
        }
        if self.messages.len() >= MAX_MESSAGE_RECORDS {
            return failed(
                message_id,
                "message deduplication limit reached; delivered input must not be replayed",
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
        if !self.initialized {
            return failed(message_id, "session initialization has not completed");
        }
        match action {
            RuntimeAction::Submit { input } => {
                let (content, expected_replay) =
                    match encode_input(input, self.skill_plugin.as_ref()) {
                        Ok(content) => content,
                        Err(error) => return failed(message_id, &error),
                    };
                let session_id =
                    self.session_id
                        .as_deref()
                        .unwrap_or_else(|| match &self.options.target {
                            SessionTarget::New => "",
                            SessionTarget::Resume { native_session_id } => native_session_id,
                        });
                let message = json!({"type":"user", "message":{"role":"user", "content":content},
                    "parent_tool_use_id":null, "session_id":session_id, "uuid":message_id.to_string()});
                self.turns.insert(
                    message_id,
                    UserTurn {
                        expected_replay,
                        sent_at: Instant::now(),
                        accepted: false,
                        started: false,
                        finished: false,
                        output: String::new(),
                        error: None,
                    },
                );
                // 写入成功并非接收确认；只相信原生 replay 或 command_lifecycle。
                Effects {
                    writes: vec![message],
                    events: Vec::new(),
                }
            }
            RuntimeAction::Steer { .. } => failed(
                message_id,
                "Claude supports queued streaming input, not verified same-turn steering; use Submit",
            ),
            RuntimeAction::Interrupt { turn_id } => {
                let Ok(turn_id) = Uuid::parse_str(&turn_id) else {
                    return failed(message_id, "invalid turn id");
                };
                if self.active_turn != Some(turn_id) || !self.turn_is_running(turn_id) {
                    return failed(
                        message_id,
                        "interrupt requires the currently running command",
                    );
                }
                let request = self.request(
                    PendingKind::Interrupt {
                        message_id,
                        turn_id,
                    },
                    json!({"subtype":"interrupt"}),
                );
                Effects {
                    writes: vec![request],
                    events: Vec::new(),
                }
            }
            RuntimeAction::RespondApproval {
                approval_id,
                decision,
            } => {
                let Some(approval) = self.approvals.get(&approval_id) else {
                    return failed(message_id, "unknown approval request");
                };
                if approval.cancelled
                    || approval.response.is_some()
                    || !self.turn_is_running(approval.turn_id)
                {
                    return failed(
                        message_id,
                        "approval is no longer pending for a running command",
                    );
                }
                let turn_id = approval.turn_id;
                let response = match decision {
                    ApprovalDecision::AllowOnce => {
                        json!({"behavior":"allow", "updatedInput":approval.input})
                    }
                    ApprovalDecision::DenyOnce => {
                        json!({"behavior":"deny", "message":"The user denied this tool invocation"})
                    }
                };
                let response = control_response(&approval_id, response);
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
                            turn_id: Some(turn_id.to_string()),
                        },
                    ],
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
                let active = self.active_turn.filter(|id| self.turn_is_running(*id));
                if call.request.turn_id != turn_id
                    || active.map(|id| id.to_string()).as_deref() != Some(turn_id.as_str())
                    || call.cancelled
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
                }
            }
            RuntimeAction::Shutdown => dispatched(message_id, None),
        }
    }

    fn turn_is_running(&self, turn_id: Uuid) -> bool {
        self.turns
            .get(&turn_id)
            .is_some_and(|turn| turn.started && !turn.finished)
    }

    fn accept_turn(&mut self, turn_id: Uuid, events: &mut Vec<RuntimeEventKind>) {
        if let Some(turn) = self.turns.get_mut(&turn_id)
            && !turn.accepted
        {
            turn.accepted = true;
            let event = RuntimeEventKind::MessageAccepted {
                message_id: turn_id,
                turn_id: Some(turn_id.to_string()),
            };
            self.remember_response(&event);
            events.push(event);
        }
    }

    fn receive(&mut self, message: Value) -> Result<Effects, RuntimeError> {
        let kind = required_string(&message, "type")?;
        if kind == "control_response" {
            return self.receive_control_response(&message["response"]);
        }
        if kind == "control_request" {
            if message["request"]["subtype"] == "mcp_message" {
                return self.receive_local_tool(&message);
            }
            return self.receive_approval(&message);
        }
        if kind == "control_cancel_request" {
            let id = required_string(&message, "request_id")?;
            let mut effects = Effects::default();
            if let Some(call) = self.local_tools.get_mut(id)
                && !call.cancelled
                && call.response.is_none()
            {
                call.cancelled = true;
                effects.events.push(RuntimeEventKind::LocalToolCancelled {
                    turn_id: call.request.turn_id.clone(),
                    call_id: call.request.call_id.clone(),
                });
            }
            if let Some(approval) = self.approvals.get_mut(id)
                && !approval.cancelled
                && approval.response.is_none()
            {
                approval.cancelled = true;
                effects.events.push(RuntimeEventKind::ApprovalCancelled {
                    approval_id: id.to_string(),
                });
            }
            return Ok(effects);
        }
        if let Some(session_id) = message
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            if let SessionTarget::Resume { native_session_id } = &self.options.target
                && session_id != native_session_id
            {
                return Err(RuntimeError::Protocol(
                    "Claude returned a different session while resuming".into(),
                ));
            }
            if self
                .session_id
                .as_deref()
                .is_some_and(|current| current != session_id)
            {
                return Ok(Effects::default());
            }
            self.session_id = Some(session_id.to_string());
        }
        if let Some(uuid) = message.get("uuid").and_then(Value::as_str) {
            let key = format!("{kind}:{uuid}");
            let fingerprint = fingerprint(&message);
            if let Some(previous) = self.seen_events.get(&key) {
                if *previous != fingerprint {
                    return Err(RuntimeError::Protocol(
                        "event UUID was reused with different contents".into(),
                    ));
                }
                return Ok(Effects::default());
            }
            if self.seen_events.len() >= MAX_MESSAGE_RECORDS * 8 {
                return Err(RuntimeError::Protocol(
                    "event deduplication limit reached".into(),
                ));
            }
            self.seen_events.insert(key, fingerprint);
        }
        let mut effects = Effects::default();
        match kind {
            "command_lifecycle" => {
                let turn_id = parse_uuid(&message, "command_uuid")?;
                if !self.turns.contains_key(&turn_id) || self.turns[&turn_id].finished {
                    return Ok(effects);
                }
                let state = required_string(&message, "state")?;
                match state {
                    "queued" => self.accept_turn(turn_id, &mut effects.events),
                    "started" => {
                        self.accept_turn(turn_id, &mut effects.events);
                        let turn = self.turns.get_mut(&turn_id).expect("turn exists");
                        if !turn.started {
                            turn.started = true;
                            self.active_turn = Some(turn_id);
                            effects.events.push(RuntimeEventKind::TurnStarted {
                                turn_id: turn_id.to_string(),
                            });
                        }
                    }
                    "cancelled" => {
                        self.finish_turn(turn_id, TurnOutcome::Cancelled, None, &mut effects.events)
                    }
                    // 未知生命周期不升级成功；需要原生 result 才能确认完成。
                    _ => {}
                }
            }
            "user" => {
                if message.get("isReplay").and_then(Value::as_bool) == Some(true) {
                    let turn_id = parse_uuid(&message, "uuid")?;
                    if let Some(turn) = self.turns.get(&turn_id) {
                        if message.pointer("/message/content").and_then(Value::as_str)
                            != Some(turn.expected_replay.as_str())
                        {
                            return Err(RuntimeError::Protocol(
                                "replayed input differs from the submitted command".into(),
                            ));
                        }
                        self.accept_turn(turn_id, &mut effects.events);
                    }
                }
            }
            "assistant" => {
                // 子代理文本不属于父回合结果，不使用当前回合兜底去误关联。
                if message
                    .get("parent_tool_use_id")
                    .is_some_and(|value| !value.is_null())
                {
                    return Ok(effects);
                }
                let Some(turn_id) =
                    correlated_turn(&message).filter(|id| self.turns.contains_key(id))
                else {
                    return Ok(effects);
                };
                let turn = self.turns.get_mut(&turn_id).expect("turn exists");
                if turn.finished {
                    return Ok(effects);
                }
                let output = assistant_text(&message);
                if message.get("is_api_error_message").and_then(Value::as_bool) == Some(true)
                    || message.get("error").is_some_and(|value| !value.is_null())
                {
                    turn.error = Some(if output.is_empty() {
                        "Claude returned an assistant error".into()
                    } else {
                        output.clone()
                    });
                }
                if !output.is_empty() {
                    if !turn.output.is_empty() {
                        turn.output.push('\n');
                    }
                    turn.output.push_str(&output);
                    effects.events.push(RuntimeEventKind::TextDelta {
                        turn_id: turn_id.to_string(),
                        item_id: required_string(&message, "uuid")?.to_string(),
                        text: output,
                    });
                }
            }
            "result" => {
                let outcome = result_outcome(&message)?;
                if !self.initialized {
                    return Err(RuntimeError::Protocol(match outcome {
                        TurnOutcome::Failed { message } => message,
                        TurnOutcome::Completed | TurnOutcome::Cancelled => {
                            "Claude ended before initialization completed".into()
                        }
                    }));
                }
                let turn_ids = correlated_turns(&message);
                if turn_ids.is_empty() {
                    return Err(RuntimeError::Protocol(
                        "result is missing a command UUID; completion is uncertain".into(),
                    ));
                }
                for turn_id in turn_ids {
                    self.finish_turn(
                        turn_id,
                        outcome.clone(),
                        message.get("result").and_then(Value::as_str),
                        &mut effects.events,
                    );
                }
            }
            "system" if message["subtype"] == "init" => {
                if message.get("claude_code_version").and_then(Value::as_str) != Some("2.1.273") {
                    return Err(RuntimeError::UnsupportedVersion(
                        message["claude_code_version"].to_string(),
                    ));
                }
                if self.session_id.is_none() {
                    return Err(RuntimeError::Protocol(
                        "Claude init has no native session id".into(),
                    ));
                }
                let effective_permissions = json!({
                    "permissionMode":message["permissionMode"], "capabilities":message["capabilities"],
                });
                verify_effective_permissions(
                    self.options.permission_ceiling.as_ref(),
                    "claude",
                    &self.options.cwd,
                    &effective_permissions,
                )?;
                if let Some(observation) = &mut self.permission_observation {
                    observation.invalidate_mode(&message["permissionMode"]);
                }
                self.ready_permissions = effective_permissions;
                if self.initialized {
                    effects.events.push(self.ready_event());
                }
            }
            "system"
                if message["subtype"] == "status" && message.get("permissionMode").is_some() =>
            {
                if let Some(observation) = &mut self.permission_observation {
                    observation.invalidate_mode(&message["permissionMode"]);
                }
                if self.ready_permissions.is_object() {
                    if let Some(mode) = permission_snapshot::mode(message.get("permissionMode")) {
                        self.ready_permissions["permissionMode"] = json!(mode);
                    }
                    if self.initialized {
                        effects.events.push(self.ready_event());
                    }
                }
            }
            // 原生系统状态、速率限制等扩展不等价于回合完成。
            _ => {}
        }
        Ok(effects)
    }

    fn receive_control_response(&mut self, response: &Value) -> Result<Effects, RuntimeError> {
        let id = match required_string(response, "request_id") {
            Ok(id) => id,
            Err(error) => {
                if self.permission_observation_started.is_some() {
                    return Ok(self.finish_permission_observation(Some(Rejection::InvalidShape)));
                }
                return Err(error);
            }
        };
        let Some(pending) = self.pending.remove(id) else {
            return Ok(Effects::default());
        };
        let success = if matches!(pending.kind, PendingKind::PermissionObservation(_)) {
            // 权限资料未知或格式错误只使观察无效，不中断普通 Inherit 对话。
            response.get("subtype").and_then(Value::as_str) == Some("success")
        } else {
            required_string(response, "subtype")? == "success"
        };
        let mut effects = Effects::default();
        match pending.kind {
            PendingKind::Initialize => {
                if !success {
                    return Err(RuntimeError::Protocol(error_text(response)));
                }
                if response
                    .pointer("/response/session_state")
                    .and_then(Value::as_str)
                    .is_some_and(|state| state != "idle")
                {
                    return Err(RuntimeError::Protocol(
                        "Claude initialization returned a non-idle session".into(),
                    ));
                }
                if let Some(plugin) = &self.skill_plugin {
                    let commands = response
                        .pointer("/response/commands")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            RuntimeError::Protocol(
                                "Claude initialization did not confirm selected skill commands"
                                    .into(),
                            )
                        })?;
                    if plugin.command_names().iter().any(|name| {
                        !commands
                            .iter()
                            .any(|command| command["name"].as_str() == Some(name.as_str()))
                    }) {
                        return Err(RuntimeError::Protocol(
                            "Claude did not register every selected skill command".into(),
                        ));
                    }
                }
                let effective_permissions = json!({
                    "permissionMode":response["response"]["current_permission_mode"],
                    "sessionAssociationConfirmed":false,
                });
                verify_effective_permissions(
                    self.options.permission_ceiling.as_ref(),
                    "claude",
                    &self.options.cwd,
                    &effective_permissions,
                )?;
                self.ready_permissions = effective_permissions;
                // 首次握手不提供原生 ID；观察只绑定当前连接代次和原生 PID。
                // 后续重复 initialize 只含 subtype，不重新注册 MCP、技能或 hooks。
                effects = self.start_permission_observation(&response["response"]);
            }
            PendingKind::PermissionObservation(stage) => {
                if !success {
                    return Ok(
                        self.finish_permission_observation(Some(Rejection::NativeRequestFailed))
                    );
                }
                let Some(observation) = &mut self.permission_observation else {
                    return Ok(Effects::default());
                };
                match stage {
                    PermissionStage::Settings => {
                        observation.settings(&response["response"]);
                        effects.writes.push(self.request(
                            PendingKind::PermissionObservation(PermissionStage::Rules),
                            json!({"subtype":"list_permission_rules"}),
                        ));
                    }
                    PermissionStage::Rules => {
                        observation.rules(&response["response"]);
                        effects.writes.push(self.request(
                            PendingKind::PermissionObservation(PermissionStage::RecheckMode),
                            json!({"subtype":"initialize"}),
                        ));
                    }
                    PermissionStage::RecheckMode => {
                        observation.finish(&response["response"], &self.options.cwd);
                        if let Some(mode) = permission_snapshot::mode(
                            response["response"].get("current_permission_mode"),
                        ) {
                            self.ready_permissions["permissionMode"] = json!(mode);
                        }
                        effects = self.finish_permission_observation(None);
                    }
                }
            }
            PendingKind::Interrupt {
                message_id,
                turn_id,
            } => {
                let effect = if success {
                    accepted(message_id, Some(turn_id.to_string()))
                } else {
                    failed(message_id, &error_text(response))
                };
                effects.events.extend(effect.events);
            }
        }
        for event in &effects.events {
            self.remember_response(event);
        }
        Ok(effects)
    }

    fn receive_local_tool(&mut self, message: &Value) -> Result<Effects, RuntimeError> {
        let id = required_string(message, "request_id")?.to_string();
        let Some(permissions) = self.options.local_tools else {
            return Ok(control_error(
                &id,
                "local task tools were not authorized for this connection",
            ));
        };
        let fingerprint = fingerprint(&message["request"]);
        if self.approvals.contains_key(&id) || self.pending.contains_key(&id) {
            return Ok(control_error(&id, "native request id was already used"));
        }
        if let Some(previous) = self.local_tools.get(&id) {
            if previous.fingerprint != fingerprint {
                return Ok(control_error(
                    &id,
                    "local tool id was reused with different content",
                ));
            }
            // 已结束或取消的调用不在新回合重放任何结果。
            let active = self.active_turn.filter(|turn| self.turn_is_running(*turn));
            if previous.cancelled
                || active.map(|turn| turn.to_string()).as_deref()
                    != Some(previous.request.turn_id.as_str())
            {
                return Ok(control_error(
                    &id,
                    "local tool invocation belongs to an inactive turn",
                ));
            }
            return Ok(Effects {
                writes: previous.response.clone().into_iter().collect(),
                events: Vec::new(),
            });
        }
        if let Some((previous, response)) = self.mcp_replies.get(&id) {
            if *previous != fingerprint {
                return Ok(control_error(
                    &id,
                    "MCP request id was reused with different content",
                ));
            }
            return Ok(Effects {
                writes: vec![response.clone()],
                events: Vec::new(),
            });
        }
        if self.local_tools.len() + self.mcp_replies.len() >= MAX_MESSAGE_RECORDS {
            return Ok(control_error(&id, "MCP history limit reached"));
        }
        let active_turn = self
            .active_turn
            .filter(|turn| self.turn_is_running(*turn))
            .map(|turn| turn.to_string());
        let request = match local_tools::claude_mcp_request(
            message,
            active_turn.as_deref(),
            permissions.allow_spawn,
            permissions.allow_message,
        ) {
            Ok(request) => request,
            Err(reason) => return Ok(control_error(&id, &reason)),
        };
        match request {
            ClaudeMcpRequest::Immediate(response) => {
                self.mcp_replies.insert(id, (fingerprint, response.clone()));
                Ok(Effects {
                    writes: vec![response],
                    events: Vec::new(),
                })
            }
            ClaudeMcpRequest::Tool(request) => {
                if !local_tools::tool_definitions(
                    permissions.allow_spawn,
                    permissions.allow_message,
                )
                .iter()
                .any(|tool| tool["name"] == request.tool)
                {
                    return Ok(control_error(&id, "local task tool is not authorized"));
                }
                self.local_tools.insert(
                    id,
                    PendingLocalTool {
                        fingerprint,
                        request: request.clone(),
                        response: None,
                        cancelled: false,
                    },
                );
                Ok(Effects {
                    writes: Vec::new(),
                    events: vec![RuntimeEventKind::LocalToolRequested { request }],
                })
            }
        }
    }

    fn receive_approval(&mut self, message: &Value) -> Result<Effects, RuntimeError> {
        let id = required_string(message, "request_id")?.to_string();
        let request = &message["request"];
        if self.local_tools.contains_key(&id) || self.mcp_replies.contains_key(&id) {
            return Ok(control_error(&id, "native request id was already used"));
        }
        let fingerprint = fingerprint(request);
        if let Some(previous) = self.approvals.get(&id) {
            if previous.fingerprint != fingerprint {
                return Ok(control_error(
                    &id,
                    "approval id reused with different contents",
                ));
            }
            return Ok(Effects {
                writes: previous.response.clone().into_iter().collect(),
                events: Vec::new(),
            });
        }
        if request["subtype"] != "can_use_tool" {
            return Ok(control_error(&id, "unsupported control request subtype"));
        }
        let Some(turn_id) = self.active_turn.filter(|id| self.turn_is_running(*id)) else {
            return Ok(control_error(
                &id,
                "no current running command for approval",
            ));
        };
        if !request["input"].is_object()
            || request.get("tool_name").and_then(Value::as_str).is_none()
        {
            return Ok(control_error(&id, "invalid tool approval request"));
        }
        if self.approvals.len() >= MAX_MESSAGE_RECORDS {
            return Err(RuntimeError::Protocol("approval limit reached".into()));
        }
        self.approvals.insert(
            id.clone(),
            PendingApproval {
                fingerprint,
                turn_id,
                input: request["input"].clone(),
                response: None,
                cancelled: false,
            },
        );
        Ok(Effects {
            writes: Vec::new(),
            events: vec![RuntimeEventKind::ApprovalRequested {
                approval_id: id,
                turn_id: turn_id.to_string(),
                method: "can_use_tool".into(),
                details: request.clone(),
            }],
        })
    }

    fn finish_turn(
        &mut self,
        turn_id: Uuid,
        outcome: TurnOutcome,
        output: Option<&str>,
        events: &mut Vec<RuntimeEventKind>,
    ) {
        let Some(turn) = self.turns.get_mut(&turn_id) else {
            return;
        };
        if turn.finished {
            return;
        }
        turn.finished = true;
        let outcome = match &turn.error {
            Some(message) => TurnOutcome::Failed {
                message: message.clone(),
            },
            None => outcome,
        };
        if self.active_turn == Some(turn_id) {
            self.active_turn = None;
        }
        for (id, approval) in &mut self.approvals {
            if approval.turn_id == turn_id && approval.response.is_none() && !approval.cancelled {
                approval.cancelled = true;
                events.push(RuntimeEventKind::ApprovalCancelled {
                    approval_id: id.clone(),
                });
            }
        }
        events.push(RuntimeEventKind::TurnFinished {
            turn_id: turn_id.to_string(),
            outcome,
            output: output.unwrap_or(&turn.output).to_string(),
        });
    }
}

fn fingerprint(value: &Value) -> [u8; 32] {
    Sha256::digest(serde_json::to_vec(value).expect("JSON value is serializable")).into()
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, RuntimeError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| RuntimeError::Protocol(format!("missing or invalid {key}")))
}

fn parse_uuid(value: &Value, key: &str) -> Result<Uuid, RuntimeError> {
    Uuid::parse_str(required_string(value, key)?)
        .map_err(|_| RuntimeError::Protocol(format!("invalid {key}")))
}

fn correlated_turn(message: &Value) -> Option<Uuid> {
    message
        .get("user_message_uuid")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn correlated_turns(message: &Value) -> Vec<Uuid> {
    let mut turns = correlated_turn(message).into_iter().collect::<Vec<_>>();
    if let Some(uuids) = message.get("user_message_uuids").and_then(Value::as_array) {
        for value in uuids {
            if let Some(id) = value.as_str().and_then(|value| Uuid::parse_str(value).ok())
                && !turns.contains(&id)
            {
                turns.push(id);
            }
        }
    }
    turns
}

fn assistant_text(message: &Value) -> String {
    message
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn result_outcome(message: &Value) -> Result<TurnOutcome, RuntimeError> {
    let subtype = required_string(message, "subtype")?;
    let terminal_reason = message.get("terminal_reason").and_then(Value::as_str);
    if message.get("is_error").and_then(Value::as_bool) == Some(true)
        || subtype.starts_with("error")
        || terminal_reason == Some("api_error")
    {
        return Ok(TurnOutcome::Failed {
            message: error_text(message),
        });
    }
    if matches!(terminal_reason, Some("interrupted" | "cancelled")) {
        return Ok(TurnOutcome::Cancelled);
    }
    if subtype == "success"
        && message.get("is_error").and_then(Value::as_bool) == Some(false)
        && matches!(terminal_reason, None | Some("completed" | "end_turn"))
    {
        return Ok(TurnOutcome::Completed);
    }
    Err(RuntimeError::Protocol(
        "unverified Claude terminal outcome; completion is uncertain".into(),
    ))
}

fn error_text(message: &Value) -> String {
    if let Some(errors) = message.get("errors").and_then(Value::as_array) {
        let errors = errors
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("; ");
        if !errors.is_empty() {
            return errors;
        }
    }
    for key in ["result", "error", "subtype"] {
        if let Some(text) = message
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty() && *text != "success")
        {
            return text.to_string();
        }
    }
    "Claude returned an error without details".into()
}

fn encode_input(
    input: Vec<InputContent>,
    plugin: Option<&PreparedClaudeSkillPlugin>,
) -> Result<(String, String), String> {
    let mut texts = Vec::new();
    let mut skill_command = None;
    for part in input {
        match part {
            InputContent::Text(text) => texts.push(text),
            InputContent::Skill { name, path } => {
                if skill_command.is_some() {
                    return Err("Claude supports only one selected skill per turn".into());
                }
                skill_command = Some(
                    plugin
                        .and_then(|plugin| plugin.command_for(&name, &path))
                        .ok_or("skill was not selected and registered for this connection")?,
                );
            }
            InputContent::LocalImage(_) => {
                return Err("Claude managed image input is not verified for this version".into());
            }
        }
    }
    let text = texts.join("\n\n");
    let (content, expected_replay) = match skill_command {
        Some(name) => {
            let mut content = format!("/{name}");
            let mut replay = format!(
                "<command-message>{name}</command-message>\n<command-name>/{name}</command-name>"
            );
            if !text.is_empty() {
                content.push(' ');
                content.push_str(&text);
                replay.push_str(&format!("\n<command-args>{text}</command-args>"));
            }
            (content, replay)
        }
        None => (text.clone(), text),
    };
    if content.is_empty()
        || content.len() > MAX_INPUT_BYTES
        || expected_replay.len() > MAX_INPUT_BYTES
    {
        return Err("input is empty or exceeds the size limit".into());
    }
    Ok((content, expected_replay))
}

fn control_response(id: &str, response: Value) -> Value {
    json!({"type":"control_response", "response":{"subtype":"success", "request_id":id, "response":response}})
}

fn control_error(id: &str, error: &str) -> Effects {
    Effects {
        writes: vec![
            json!({"type":"control_response", "response":{"subtype":"error", "request_id":id, "error":error}}),
        ],
        events: Vec::new(),
    }
}

fn failed(message_id: Uuid, message: &str) -> Effects {
    Effects {
        writes: Vec::new(),
        events: vec![RuntimeEventKind::RequestFailed {
            message_id,
            message: message.into(),
        }],
    }
}

fn accepted(message_id: Uuid, turn_id: Option<String>) -> Effects {
    Effects {
        writes: Vec::new(),
        events: vec![RuntimeEventKind::MessageAccepted {
            message_id,
            turn_id,
        }],
    }
}

fn dispatched(message_id: Uuid, turn_id: Option<String>) -> Effects {
    Effects {
        writes: Vec::new(),
        events: vec![RuntimeEventKind::CommandDispatched {
            message_id,
            turn_id,
        }],
    }
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
