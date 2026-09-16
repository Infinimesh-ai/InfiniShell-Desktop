//! Grok 1.0.30 的 ACP 连接验证；未实测的模型生命周期保持关闭。

use std::collections::{HashMap, HashSet};
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
    InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand, RuntimeConnection, RuntimeError,
    RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome, channels,
};

const VERIFIED_VERSION: &str = "1.0.30";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_NATIVE_IDENTITIES: usize = 16_384;

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
    // Grok 的权限模式不等同于 Codex 沙箱；空历史恢复不能证明完整模型会话恢复。
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
        self.pending
            .as_ref()
            .is_some_and(|request| request.sent_at.elapsed() >= REQUEST_TIMEOUT)
    }

    fn command(&mut self, command: RuntimeCommand) -> Effects {
        if command.generation != self.options.generation {
            return rejected_command(
                command.message_id,
                RuntimeError::StaleGeneration.to_string(),
            );
        }
        // 产品门禁不随底层 ACP 解析能力改变；未完成审批验收前不发送任何模型请求。
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
                    return Effects {
                        writes: Vec::new(),
                        events: vec![RuntimeEventKind::CommandDispatched {
                            message_id: command.message_id,
                            turn_id: None,
                        }],
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
                // 空历史恢复证据不包括历史模型回合；恢复连接上的提交也保持关闭。
                if self.closed
                    || self.pending.is_some()
                    || self.session_id.is_none()
                    || self.options.target != SessionTarget::New
                {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-agent-grok-managed-unverified"),
                    );
                }
                let mut prompt = Vec::new();
                for content in input {
                    match content {
                        InputContent::Text(text) => {
                            prompt.push(json!({"type": "text", "text": text}))
                        }
                        InputContent::LocalImage(_) | InputContent::Skill { .. } => {
                            return rejected_command(
                                command.message_id,
                                crate::t!("cli-agent-grok-managed-unverified"),
                            );
                        }
                    }
                }
                if prompt.is_empty() {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-task-manager-empty-prompt"),
                    );
                }
                self.submitted_messages
                    .insert(command.message_id, fingerprint);
                self.prompt = Some(PendingPrompt {
                    message_id: command.message_id,
                    native_id: None,
                    started: false,
                    finished: false,
                });
                Effects {
                    writes: vec![self.request(
                        PendingKind::Prompt,
                        "session/prompt",
                        json!({"sessionId": self.session_id, "prompt": prompt}),
                    )],
                    events: Vec::new(),
                }
            }
            RuntimeAction::Steer { .. }
            | RuntimeAction::Interrupt { .. }
            | RuntimeAction::RespondApproval { .. }
            | RuntimeAction::RespondLocalTool { .. } => rejected_command(
                command.message_id,
                crate::t!("cli-agent-grok-managed-unverified"),
            ),
        }
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
                let method =
                    if self.reported_capabilities["sessionCapabilities"]["resume"].is_object() {
                        "session/resume"
                    } else if self.reported_capabilities["loadSession"] == true {
                        "session/load"
                    } else {
                        return Err(RuntimeError::Protocol(
                            "Grok session recovery is not advertised".into(),
                        ));
                    };
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
            // 未声明的客户端工具调用不执行，也不伪造审批决定。
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
                let effects = self.finish_prompt_failure(detail);
                self.prompt = None;
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
                let cached_login = result["authMethods"].as_array().is_some_and(|methods| {
                    methods
                        .iter()
                        .any(|method| method["id"].as_str() == Some("cached_token"))
                });
                if !cached_login {
                    return Err(RuntimeError::InvalidConfiguration(crate::t!(
                        "cli-agent-grok-managed-login-required"
                    )));
                }
                effects.writes.push(self.request(
                    PendingKind::Authenticate,
                    "authenticate",
                    json!({"methodId": "cached_token", "_meta": {"headless": true}}),
                ));
            }
            PendingKind::Authenticate => effects.writes.push(self.open_session()?),
            PendingKind::OpenSession { requested_id } => {
                let id = match requested_id {
                    Some(requested_id) => {
                        if result
                            .get("sessionId")
                            .is_some_and(|id| id.as_str() != Some(&requested_id))
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
                // 当前实证只有 stopReason=error；未知成功/取消值必须等待独立验收。
                if result["stopReason"] != "error" {
                    return Err(RuntimeError::Protocol(
                        "unverified Grok prompt stop reason".into(),
                    ));
                }
                effects = self.finish_prompt_failure(
                    result["agentResult"]
                        .as_str()
                        .unwrap_or("Grok prompt failed")
                        .to_owned(),
                );
                self.prompt = None;
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
        if self.session_id.is_none() || params["sessionId"].as_str() != self.session_id.as_deref() {
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
                Ok(Effects::default())
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
            // 未出现过的 stopReason 不能宣称成功；保留未确认状态等 RPC 或断线。
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
        Effects {
            writes: Vec::new(),
            events: vec![event],
        }
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
