//! Grok 固定版本的 ACP 适配；按版本分别维护 P0 与扩展生命周期的验收边界。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::ffi::OsString;
#[cfg(test)]
use std::io::Read;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::{Arc, Mutex};
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
use warp_cli::agent::Harness;
use warpui::r#async::{FutureExt as _, Timer};

use super::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeConnection, RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome, channels,
};

use super::grok_tool_lease::{GrokToolLeaseLedger, GrokToolLeaseState, VerifiedGrokToolLease};
use super::local_skills::SelectedLocalSkill;
use super::local_tools::{GrokMcpBridge, GrokMcpRequest, MCP_SERVER_NAME, NativeLocalToolRequest};

pub(super) const VERIFIED_VERSION: &str = "1.0.30";
pub(super) const P0_VERIFIED_VERSION: &str = "1.0.34";
pub(super) const CURRENT_VERSION: &str = "1.0.40";
#[cfg(test)]
const TEST_CANDIDATE_VERSION: &str = "1.0.41";
#[cfg(test)]
const TEST_CANDIDATE_VERSION_OUTPUT: &str = "grok 1.0.41 (4220f3b224a6)";
#[cfg(test)]
const TEST_CANDIDATE_BYTES: u64 = 145_657_952;
#[cfg(test)]
const TEST_CANDIDATE_SHA256: &str =
    "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d";
const CURRENT_SETUP_METHOD: &str = "_x.ai/session/setup";
const CURRENT_SETUP_PHASES: [&str; 6] = [
    "auth",
    "resolve_workspace",
    "folder_trust",
    "plugin_registry",
    "mcp_merge",
    "response_ready",
];
#[cfg(all(test, unix))]
const DIRECT_CATALOG_SETUP_PHASES: [&str; 11] = [
    "auth",
    "resolve_workspace",
    "folder_trust",
    "plugin_registry",
    "mcp_merge",
    "persistence_init",
    "spawn_session_actor",
    "git_discovery",
    "finalize_response",
    "tool_overrides",
    "response_ready",
];
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_NATIVE_IDENTITIES: usize = 16_384;
const MAX_QUEUED_PROMPTS: usize = 32;
const SNAPSHOT_RETRY_INTERVAL: Duration = Duration::from_millis(250);

/// 验证固定版本、原生生命周期及独占 SDK 租约；产品能力另由协调器门禁控制。
pub fn connect(options: SessionOptions) -> Result<RuntimeConnection, RuntimeError> {
    validate_options(&options)?;
    Ok(connect_protocol(GrokProtocol::new(options)))
}

fn connect_protocol(mut protocol: GrokProtocol) -> RuntimeConnection {
    let (controller, commands, sender, events) = channels(protocol.options.generation);
    let task = Box::pin(async move {
        let result = run_process(&mut protocol, commands, &sender).await;
        #[cfg(test)]
        update_lease_audit(&protocol.lease_audit_for_live, |audit| {
            audit.protocol_errors += u64::from(result.is_err());
        });
        let reason = match &result {
            Ok(()) => "runtime connection closed".to_owned(),
            Err(error) => error.to_string(),
        };
        let _ = sender.try_send(protocol.event(RuntimeEventKind::Disconnected { reason }));
        result
    });
    RuntimeConnection {
        controller,
        events,
        task,
    }
}

fn validate_options(options: &SessionOptions) -> Result<(), RuntimeError> {
    if !options.executable.is_absolute() || !options.cwd.is_absolute() {
        return Err(RuntimeError::InvalidConfiguration(
            "executable and cwd must be absolute".into(),
        ));
    }
    if matches!(&options.target, SessionTarget::Resume { native_session_id } if !valid_native_id(native_session_id))
    {
        return Err(RuntimeError::InvalidConfiguration(crate::t!(
            "cli-agent-grok-managed-unverified"
        )));
    }
    if !matches!(
        options.permission_policy,
        PermissionPolicy::Inherit
            | PermissionPolicy::GrokRestrictedReadV1
            | PermissionPolicy::GrokRestrictedFilesV1
    ) || options.claude_profile.is_some()
        || options.model.is_some()
    {
        return Err(RuntimeError::InvalidConfiguration(crate::t!(
            "cli-agent-grok-managed-unverified"
        )));
    }
    if !options.selected_skills.is_empty() && options.permission_policy != PermissionPolicy::Inherit
    {
        return Err(RuntimeError::InvalidConfiguration(crate::t!(
            "cli-agent-grok-skill-policy-required"
        )));
    }
    if options.selected_skills.len() > 1 && options.target == SessionTarget::New {
        return Err(RuntimeError::InvalidConfiguration(crate::t!(
            "cli-agent-task-skill-one-per-turn"
        )));
    }
    match options.permission_policy {
        PermissionPolicy::Inherit => {
            if options.permission_ceiling.is_some()
                || options.grok_profile.is_some()
                || options.local_tools.is_some_and(|tools| tools.allow_spawn)
            {
                return Err(super::permissions::rejected(
                    None,
                    &Value::Null,
                    "grok_inherit_parent_unknown",
                    false,
                ));
            }
        }
        PermissionPolicy::GrokRestrictedReadV1 | PermissionPolicy::GrokRestrictedFilesV1 => {
            if options.target != SessionTarget::New && options.grok_profile.is_none() {
                return Err(super::permissions::rejected(
                    None,
                    &Value::Null,
                    "grok_resume_creation_policy_missing",
                    false,
                ));
            }
            if let Some(profile) = &options.grok_profile {
                profile.validate()?;
                if profile.permission_policy() != options.permission_policy {
                    return Err(super::permissions::rejected(
                        None,
                        &Value::Null,
                        "grok_saved_tool_set_changed",
                        false,
                    ));
                }
            }
            if let Some(parent) = &options.permission_ceiling {
                let parent = parent.grok_profile().ok_or_else(|| {
                    super::permissions::rejected(
                        None,
                        &Value::Null,
                        "grok_creation_parent_unknown",
                        false,
                    )
                })?;
                parent.validate_child(options.grok_profile.as_ref().ok_or_else(|| {
                    super::permissions::rejected(
                        None,
                        &Value::Null,
                        "grok_creation_child_missing",
                        false,
                    )
                })?)?;
            }
        }
        PermissionPolicy::ReadOnly
        | PermissionPolicy::WorkspaceWrite
        | PermissionPolicy::ClaudeRestrictedFilesV1 => unreachable!("已拒绝其他 CLI 策略"),
    }
    Ok(())
}

/// 只表示适配器有该版本的精确基础门禁；扩展生命周期仍由各能力的独立收据控制。
pub(crate) fn supported_version(version: &str) -> bool {
    matches!(
        version,
        VERIFIED_VERSION | P0_VERIFIED_VERSION | CURRENT_VERSION
    )
}

fn verified_version(output: &str) -> Option<&'static str> {
    match output
        .trim()
        .strip_prefix("grok ")
        .and_then(|suffix| suffix.split_whitespace().next())
    {
        Some(VERIFIED_VERSION) => Some(VERIFIED_VERSION),
        Some(P0_VERIFIED_VERSION) => Some(P0_VERIFIED_VERSION),
        Some(CURRENT_VERSION) => Some(CURRENT_VERSION),
        Some(_) | None => None,
    }
}

async fn run_process(
    protocol: &mut GrokProtocol,
    commands: mpsc::Receiver<RuntimeCommand>,
    events: &mpsc::Sender<RuntimeEvent>,
) -> Result<(), RuntimeError> {
    #[cfg(test)]
    if protocol.test_only_1041_profile {
        if !protocol.current_selected_skill_candidate_for_live() {
            return Err(RuntimeError::InvalidConfiguration(crate::t!(
                "cli-agent-grok-managed-unverified"
            )));
        }
        let native = protocol
            .test_only_1041_native_binary
            .as_deref()
            .ok_or_else(|| {
                RuntimeError::InvalidConfiguration("Grok 1.0.41 test native binary missing".into())
            })?;
        if native == protocol.options.executable.as_path() {
            return Err(RuntimeError::InvalidConfiguration(
                "Grok 1.0.41 test isolation wrapper missing".into(),
            ));
        }
        verify_test_candidate_binary(native)?;
    }
    let launch = if matches!(
        protocol.options.permission_policy,
        PermissionPolicy::GrokRestrictedReadV1 | PermissionPolicy::GrokRestrictedFilesV1
    ) {
        let launch = super::grok_profile::GrokCreationPolicyV1::prepare(&protocol.options)?;
        protocol.options.grok_profile = Some(launch.policy.clone());
        if let Some(sdk) = &mut protocol.sdk {
            sdk.bridge = GrokMcpBridge::with_creation_policy(sdk.process_epoch, &launch.policy)
                .map_err(RuntimeError::InvalidConfiguration)?;
        }
        Some(launch)
    } else {
        None
    };
    let mut version = Command::new(&protocol.options.executable);
    version
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(launch) = &launch {
        version
            .env_clear()
            .envs(super::managed_process::isolated_environment(&launch.home))
            .current_dir(launch.home.join("startup"));
    }
    let output = version
        .output()
        .with_timeout(Duration::from_secs(3))
        .await
        .map_err(|_| RuntimeError::RequestTimedOut)??;
    let detected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() {
        return Err(RuntimeError::UnsupportedVersion(detected));
    }
    protocol.bind_cli_version(&detected)?;

    // 独立 leader socket 防止此次连接意外关联用户已有的 Grok 进程。
    let mut directory_builder = tempfile::Builder::new();
    directory_builder.prefix("infinishell-grok-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        directory_builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let directory = directory_builder.tempdir()?;
    // SDK 反向请求不携带会话路由字段；独立 stdio 连接避免依赖 leader 会话路由。
    let direct_sdk = protocol.options.local_tools.is_some();
    #[cfg(test)]
    let direct_sdk = direct_sdk || protocol.sdk_origin_probe.is_some();
    #[cfg(all(test, unix))]
    let direct_sdk = direct_sdk || protocol.catalog_direct_for_live;
    let arguments = if let Some(launch) = &launch {
        vec![
            OsString::from("agent"),
            OsString::from("--no-leader"),
            OsString::from("--agent-profile"),
            launch.profile_path.clone().into_os_string(),
            OsString::from("stdio"),
        ]
    } else if direct_sdk {
        vec![
            OsString::from("agent"),
            OsString::from("--no-leader"),
            OsString::from("stdio"),
        ]
    } else {
        vec![
            OsString::from("agent"),
            OsString::from("stdio"),
            OsString::from("--leader-socket"),
            directory.path().join("leader.sock").into_os_string(),
        ]
    };
    #[cfg(all(test, unix))]
    let arguments = if let Some(profile) = &protocol.skill_profile_for_live {
        if protocol.options.permission_policy != PermissionPolicy::Inherit
            || protocol.options.grok_profile.is_some()
        {
            return Err(RuntimeError::InvalidConfiguration(
                "技能实验不能替换固定生产策略".into(),
            ));
        }
        vec![
            OsString::from("agent"),
            OsString::from("--no-leader"),
            OsString::from("--agent-profile"),
            profile.clone().into_os_string(),
            OsString::from("stdio"),
        ]
    } else {
        arguments
    };
    let process_cwd = launch
        .as_ref()
        .map(|launch| launch.home.join("startup"))
        .unwrap_or_else(|| protocol.options.cwd.clone());
    let mut child = super::managed_process::spawn_with_isolated_home(
        &protocol.options.state_dir,
        protocol.options.generation,
        &protocol.options.executable,
        &arguments,
        &process_cwd,
        launch.as_ref().map(|launch| launch.home.as_path()),
    )
    .await?;
    if let Some(sdk) = protocol.sdk.as_mut() {
        // 只有生产监督器成功派生的独占进程才能提供这次 SDK 能力来源。
        sdk.owned_process = true;
        #[cfg(test)]
        update_lease_audit(&protocol.lease_audit_for_live, |audit| {
            audit.owned_process_confirmed = true
        });
    }
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| RuntimeError::Protocol("missing Grok stdin".into()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| RuntimeError::Protocol("missing Grok stdout".into()))?;
    let result = run_transport(protocol, &mut stdin, &mut stdout, commands, events).await;
    let effects = protocol.finish_transport(&result);
    for kind in effects.events {
        let _ = events.try_send(protocol.event(kind));
    }
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
    let graceful = result.is_ok();
    let finish = async move {
        if graceful {
            child.finish_after_stdin_close().await
        } else {
            child.finish().await
        }
    };
    let (finished, drained) = futures::join!(finish, drain.with_timeout(Duration::from_secs(30)));
    // 清理和读取均结束后保留原传输失败；正常成功还必须具有可信回执及预算内 EOF。
    result?;
    finished?;
    drained.map_err(|_| RuntimeError::RequestTimedOut)??;
    Ok(())
}

#[cfg(test)]
fn verify_test_candidate_binary(path: &Path) -> Result<(), RuntimeError> {
    if !path.is_absolute()
        || path.canonicalize()? != path
        || !std::fs::symlink_metadata(path)?.file_type().is_file()
        || std::fs::metadata(path)?.len() != TEST_CANDIDATE_BYTES
    {
        return Err(RuntimeError::UnsupportedVersion(
            "Grok 1.0.41 test binary identity mismatch".into(),
        ));
    }
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.finalize()) != TEST_CANDIDATE_SHA256 {
        return Err(RuntimeError::UnsupportedVersion(
            "Grok 1.0.41 test binary identity mismatch".into(),
        ));
    }
    Ok(())
}

async fn run_transport(
    protocol: &mut GrokProtocol,
    stdin: &mut (impl AsyncWrite + Unpin),
    stdout: &mut (impl AsyncRead + Unpin),
    mut commands: mpsc::Receiver<RuntimeCommand>,
    events: &mpsc::Sender<RuntimeEvent>,
) -> Result<(), RuntimeError> {
    let initialize = protocol.initialize()?;
    #[cfg(test)]
    if let Some(probe) = &protocol.catalog_probe_for_live {
        probe.guard_write(&initialize)?;
    }
    write_message(stdin, &initialize).await?;
    #[cfg(test)]
    if let Some(probe) = &protocol.sdk_origin_probe {
        probe.observe_outbound_transaction(&initialize, protocol.transaction_context());
    }
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let effects = protocol.poll_final_output();
        flush_effects(protocol, stdin, events, effects).await?;
        if protocol.closed {
            return Ok(());
        }
        if protocol.sdk_timed_out(Instant::now()) {
            return Err(RuntimeError::RequestTimedOut);
        }
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
                    #[cfg(test)]
                    trace_live_protocol_ids(&message);
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

#[cfg(test)]
fn trace_live_protocol_ids(message: &Value) {
    if std::env::var_os("INFINISHELL_GROK_LIVE_ROOT").is_none() {
        return;
    }
    // 隔离验收仅记录协议身份和状态；不记录提示、输出、工具参数或凭据。
    let params = &message["params"];
    let update = &params["update"];
    let object_keys = |value: &Value| {
        value.as_object().map(|object| {
            let mut keys = object.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            keys
        })
    };
    let available_command_shapes = update["availableCommands"].as_array().map(|commands| {
        commands
            .iter()
            .map(|command| {
                json!({
                    "keys": object_keys(command),
                    "nameType": diagnostic_value_type(command.get("name")),
                    "descriptionType": diagnostic_value_type(command.get("description")),
                    "inputType": diagnostic_value_type(command.get("input")),
                })
            })
            .collect::<Vec<_>>()
    });
    let value_types = |value: &Value| {
        value.as_object().map(|object| {
            let mut types = object
                .iter()
                .map(|(key, value)| (key.clone(), diagnostic_value_type(Some(value))))
                .collect::<Vec<_>>();
            types.sort_by(|left, right| left.0.cmp(&right.0));
            types
        })
    };
    eprintln!(
        "GROK_NATIVE_PROTOCOL_IDS {}",
        json!({"id":message.get("id").map(|id| if id.is_string() {diagnostic_value(Some(id))} else {id.clone()}),"method":message.get("method"),
            "sessionId":params.get("sessionId"),"resultSessionId":message["result"].get("sessionId"),
            "setupMethod":params.get("method"),"setupPhase":params.get("phase"),
            "sessionUpdate":update.get("sessionUpdate"),
            "paramsKeys":object_keys(params),"paramsMetaKeys":object_keys(&params["_meta"]),
            "updateKeys":object_keys(update),"updateMetaKeys":object_keys(&update["_meta"]),
            "paramsMetaTypes":value_types(&params["_meta"]),
            "updateMetaTypes":value_types(&update["_meta"]),
            "updateMetaToolsCount":update["_meta"]["tools"].as_array().map(Vec::len),
            "availableCommandsCount":update["availableCommands"].as_array().map(Vec::len),
            "availableCommandShapes":available_command_shapes,
            "eventId":params["_meta"].get("eventId"),"promptId":params["_meta"].get("promptId"),
            "streamStartMs":params["_meta"].get("streamStartMs"),"chunkId":params["_meta"].get("chunkId"),
            "toolCallId":update.get("toolCallId"),"status":update.get("status"),
            "stopReason":message["result"].get("stopReason").map(|reason| match reason.as_str() {
                Some("end_turn" | "max_tokens" | "refusal" | "cancelled") => reason.clone(),
                Some(_) | None => diagnostic_value(Some(reason)),
            }),
            "response_diagnostic":native_response_diagnostic(message)})
    );
}

#[cfg(test)]
fn diagnostic_value(value: Option<&Value>) -> Value {
    let mut summary = json!({"type":diagnostic_value_type(value)});
    if let Some(value) = value {
        // 任意错误或畸形字段只保留长度和摘要，不保存正文、地址或查询。
        let bytes = match value {
            Value::String(text) => text.as_bytes().to_vec(),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => serde_json::to_vec(value).expect("JSON 值可序列化"),
        };
        summary["bytes"] = json!(bytes.len());
        summary["sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
    }
    summary
}

#[cfg(test)]
fn diagnostic_value_type(value: Option<&Value>) -> &'static str {
    match value {
        None => "absent",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

#[cfg(test)]
fn native_response_diagnostic(message: &Value) -> Value {
    let error = message.get("error");
    let result = message.get("result");
    let http_status = message["error"]["data"]["http_status"]
        .as_i64()
        .filter(|code| (100..=599).contains(code));
    let error_category = match http_status {
        Some(401) => "http_401",
        Some(402) => "http_402",
        Some(403) => "http_403",
        Some(429) => "http_429",
        Some(400..=499) => "http_4xx",
        Some(500..=599) => "http_5xx",
        Some(_) | None => "unknown",
    };
    json!({
        "jsonrpc":diagnostic_value(message.get("jsonrpc")),
        "jsonrpc_is_2_0":message["jsonrpc"] == "2.0",
        "response_id":diagnostic_value(message.get("id")),
        "response_id_number":message["id"].as_u64().filter(|id| *id <= i64::MAX as u64),
        "method_present":message.get("method").is_some(),
        "error_present":error.is_some(),"error_type":diagnostic_value_type(error),
        "error_code":message["error"]["code"].as_i64(),
        "native_error_http_status":http_status,"native_error_category":error_category,
        "error_code_type":diagnostic_value_type(error.and_then(|value| value.get("code"))),
        "error_message":diagnostic_value(error.and_then(|value| value.get("message"))),
        "error_data_message":diagnostic_value(error.and_then(|value| value.get("data")).and_then(|value| value.get("message"))),
        "result_present":result.is_some(),"result_type":diagnostic_value_type(result),
        "result_stop_reason":diagnostic_value(result.and_then(|value| value.get("stopReason"))),
        "result_error_conflict":result.is_some() && error.is_some()
    })
}

#[cfg(test)]
fn runtime_error_diagnostic(error: &RuntimeError) -> Value {
    let kind = match error {
        RuntimeError::StaleGeneration => "stale_generation",
        RuntimeError::ControllerClosed => "controller_closed",
        RuntimeError::UnsupportedVersion(_) => "unsupported_version",
        RuntimeError::InvalidConfiguration(_) => "invalid_configuration",
        RuntimeError::PermissionCeilingRejected { .. } => "permission_ceiling_rejected",
        RuntimeError::Protocol(_) => "protocol",
        RuntimeError::Io(_) => "io",
        RuntimeError::RequestTimedOut => "request_timed_out",
        RuntimeError::EventBackpressure => "event_backpressure",
    };
    let protocol_failure = match error {
        RuntimeError::Protocol(detail) => match detail.as_str() {
            "invalid Grok response id" => "invalid_response_id",
            "Grok response is not JSON-RPC 2.0" => "invalid_jsonrpc_version",
            "unsolicited Grok response" => "unsolicited_response",
            "conflicting Grok response for completed request" => "conflicting_response",
            "Grok ACP stdout closed" => "stdout_closed",
            "Grok native identity ledger limit reached" => "identity_ledger_limit",
            _ => "unknown",
        },
        RuntimeError::StaleGeneration
        | RuntimeError::ControllerClosed
        | RuntimeError::UnsupportedVersion(_)
        | RuntimeError::InvalidConfiguration(_)
        | RuntimeError::PermissionCeilingRejected { .. }
        | RuntimeError::Io(_)
        | RuntimeError::RequestTimedOut
        | RuntimeError::EventBackpressure => "not_protocol",
    };
    json!({"runtime_error_kind":kind,"protocol_failure_kind":protocol_failure,
        "runtime_error_message":diagnostic_value(Some(&Value::String(error.to_string())))})
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
    protocol: &mut GrokProtocol,
    stdin: &mut (impl AsyncWrite + Unpin),
    events: &mpsc::Sender<RuntimeEvent>,
    effects: Effects,
) -> Result<(), RuntimeError> {
    let generation = protocol.options.generation;
    let native_session_id = protocol.session_id.clone();
    let publish = |kind| {
        events
            .try_send(RuntimeEvent {
                generation,
                native_session_id: native_session_id.clone(),
                kind,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => RuntimeError::EventBackpressure,
                mpsc::error::TrySendError::Closed(_) => RuntimeError::ControllerClosed,
            })
    };
    let (confirmed, after_write): (Vec<_>, Vec<_>) = effects.events.into_iter().partition(|kind| {
        matches!(
            kind,
            RuntimeEventKind::TurnFinished { .. }
                | RuntimeEventKind::TextDelta { .. }
                | RuntimeEventKind::RequestFailed { .. }
                | RuntimeEventKind::ApprovalCancelled { .. }
                | RuntimeEventKind::LocalToolCancelled { .. }
        )
    });
    // 下一轮写入失败不能丢失上一轮真实终态；派发确认与审批决定仍在写入成功后发布。
    for kind in confirmed {
        publish(kind)?;
    }
    for message in effects.writes {
        #[cfg(test)]
        if let Some(probe) = &protocol.catalog_probe_for_live {
            probe.guard_write(&message)?;
        }
        write_message(stdin, &message).await?;
        if message["method"] == "session/prompt" {
            // 只证明本进程代次曾完整写入输入；原生 ACK 仍由独立接收事件确认。
            protocol.task_input_written = true;
        }
        // 缓存或生成回复不能证明真实写入；必须在 write_all 和 flush 都成功后登记。
        protocol.sdk_written_message(&message)?;
        #[cfg(test)]
        if let Some(probe) = &protocol.catalog_probe_for_live {
            probe.observe_written(&message);
        }
        #[cfg(test)]
        if let Some(probe) = &protocol.sdk_origin_probe {
            probe.observe_outbound_transaction(&message, protocol.transaction_context());
        }
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
    FinalOutput,
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
    last_chunk_sequence: Option<u64>,
    chunks: BTreeMap<u64, (u64, [u8; 32], Option<String>)>,
    next_chunk: u64,
    retained_text_bytes: usize,
    output: String,
    cancel_sent: Option<Instant>,
    completion: Option<PendingCompletion>,
}

struct PendingCompletion {
    outcome: TurnOutcome,
    started_at: Instant,
    retry_at: Option<Instant>,
}

struct QueuedPrompt {
    message_id: Uuid,
    input: Vec<InputContent>,
    skill: Option<skills::SelectedSkill>,
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
    lease_permission: Option<Value>,
}

#[derive(Default)]
struct CurrentSetupProgress {
    next_phase: usize,
    session_id: Option<String>,
    response_received: bool,
    models_received: bool,
    settings_received: bool,
    announcements_received: u8,
    announcement_generation: Option<u64>,
    commands_received: bool,
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

/// 只记录 initialize 实际返回且仓库已有精确字段证据的扩展能力。
///
/// queue/interject 目前只有二进制静态方法名，没有 initialize 字段或出站回执，
/// 因此即使未来响应夹带同名字段也保持关闭，不能由字符串存在性推导协议能力。
/// availableCommands 只是初始化目录，技能还必须由当前会话通知建立独立目录。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportedInitializeExtensions {
    queue_change_notifications: bool,
    queue_interject_requests: bool,
    skills_methods: bool,
    available_commands: bool,
    local_mcp_sdk_advertised: bool,
}

impl ReportedInitializeExtensions {
    fn decode(result: &Value) -> Result<(Self, Vec<ReportedCommand>), RuntimeError> {
        let metadata = result
            .get("_meta")
            .and_then(Value::as_object)
            .ok_or_else(|| RuntimeError::Protocol("missing Grok initialize metadata".into()))?;
        let available_commands = match metadata.get("availableCommands") {
            Some(commands) => decode_metadata(commands)?,
            None => Vec::new(),
        };
        Ok((
            Self {
                queue_change_notifications: false,
                queue_interject_requests: false,
                skills_methods: false,
                available_commands: metadata
                    .get("availableCommands")
                    .is_some_and(Value::is_array),
                local_mcp_sdk_advertised: metadata.get("x.ai/mcp/sdk") == Some(&Value::Bool(true)),
            },
            available_commands,
        ))
    }
}

fn internal_skills_reload_success(message: &Value) -> bool {
    let Some(outer) = message.as_object() else {
        return false;
    };
    if outer.len() != 3 || message["jsonrpc"] != "2.0" || message["id"] != "skills-reload" {
        return false;
    }
    let Some(result) = message.get("result").and_then(Value::as_object) else {
        return false;
    };
    let Some(inner) = result.get("result").and_then(Value::as_object) else {
        return false;
    };
    result.len() == 1 && inner.len() == 1 && inner.get("reloaded").and_then(Value::as_u64).is_some()
}

struct PendingGrokLocalTool {
    request: NativeLocalToolRequest,
    proof: VerifiedGrokToolLease,
    requested_at: Instant,
    replied: bool,
    cancelled: bool,
    closed: bool,
}

// 隔离原生验收只观察生产状态；计数不参与协议、审批、身份或结果判断。
#[cfg(test)]
#[derive(Clone, Default, Serialize)]
struct GrokLeaseAudit {
    owned_process_confirmed: bool,
    capability_confirmed: bool,
    registration_requests: u64,
    native_tool_frames: u64,
    native_initial_inputs: u64,
    native_complete_inputs: u64,
    permission_writes_allow: u64,
    permission_writes_deny: u64,
    business_dispatches: u64,
    reply_writes: u64,
    native_completions: u64,
    retired: bool,
    protocol_errors: u64,
    sdk_origin_observations: u64,
    full_native_sdk_origin_fields_observed: Option<bool>,
}

#[cfg(test)]
fn update_lease_audit(
    audit: &Option<Arc<Mutex<GrokLeaseAudit>>>,
    update: impl FnOnce(&mut GrokLeaseAudit),
) {
    if let Some(audit) = audit {
        update(&mut audit.lock().expect("验收计数锁不应损坏"));
    }
}

struct GrokSdkConnection {
    process_epoch: Uuid,
    bridge: GrokMcpBridge,
    ledger: Option<GrokToolLeaseLedger>,
    owned_process: bool,
    retired: bool,
    retired_at: Option<Instant>,
    native_calls: HashSet<String>,
    calls: HashMap<String, PendingGrokLocalTool>,
    permissions_to_write: HashMap<[u8; 32], (Value, bool)>,
    approved_until: HashMap<String, Instant>,
    replies_to_write: HashMap<String, (VerifiedGrokToolLease, HashSet<[u8; 32]>)>,
    native_completion_deadlines: HashMap<String, (VerifiedGrokToolLease, Instant)>,
}

impl GrokSdkConnection {
    fn schedule_replies(
        &mut self,
        proof: VerifiedGrokToolLease,
        writes: &[Value],
    ) -> Result<(), RuntimeError> {
        let fingerprints = writes
            .iter()
            .map(message_fingerprint)
            .collect::<Result<HashSet<_>, _>>()?;
        self.replies_to_write
            .insert(proof.native_call_id().to_owned(), (proof, fingerprints));
        Ok(())
    }
}

struct GrokProtocol {
    probed_version: Option<&'static str>,
    paired_version: Option<&'static str>,
    creation_catalog_session: Option<String>,
    skill_catalog: Option<skills::SkillCatalog>,
    early_skill_catalogs: HashMap<String, skills::SkillCatalog>,
    deferred_ready: Option<(Value, Instant)>,
    #[cfg(all(test, unix))]
    skill_profile_for_live: Option<std::path::PathBuf>,
    #[cfg(all(test, unix))]
    skill_catalog_for_live: Option<native_skill_live_tests::SkillCatalogProbe>,
    #[cfg(all(test, unix))]
    catalog_direct_for_live: bool,
    #[cfg(all(test, unix))]
    direct_mcp_initialized_received: bool,
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
    task_input_written: bool,
    notification_ids: HashMap<String, [u8; 32]>,
    observed_prompt_ids: HashSet<String>,
    current_mcp_refresh_received: bool,
    current_setup: Option<CurrentSetupProgress>,
    reported_capabilities: Value,
    reported_initialize_extensions: ReportedInitializeExtensions,
    reported_metadata: ReportedMetadata,
    closed: bool,
    sdk: Option<GrokSdkConnection>,
    #[cfg(test)]
    lease_audit_for_live: Option<Arc<Mutex<GrokLeaseAudit>>>,
    #[cfg(test)]
    catalog_probe_for_live: Option<catalog_preflight_live_tests::CatalogProbe>,
    #[cfg(test)]
    queued_submissions: usize,
    #[cfg(test)]
    sdk_origin_probe: Option<sdk_origin_live_tests::SdkOriginProbe>,
    #[cfg(test)]
    verified_final_histories_for_live: Option<live_tests::VerifiedFinalHistories>,
    #[cfg(test)]
    current_root_candidate_for_live: bool,
    #[cfg(test)]
    current_selected_skill_candidate_for_live: bool,
    #[cfg(test)]
    test_only_1041_profile: bool,
    #[cfg(test)]
    test_only_1041_native_binary: Option<PathBuf>,
}

impl GrokProtocol {
    fn current_version_candidate(&self, version: &str) -> bool {
        if version == CURRENT_VERSION {
            return true;
        }
        #[cfg(test)]
        {
            self.test_only_1041_profile && version == TEST_CANDIDATE_VERSION
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    fn current_protocol(&self) -> bool {
        self.probed_version
            .is_some_and(|version| self.current_version_candidate(version))
    }

    fn test_candidate_settings(&self) -> bool {
        #[cfg(test)]
        {
            self.test_only_1041_profile && self.probed_version == Some(TEST_CANDIDATE_VERSION)
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    fn baseline_lifecycle_verified(&self) -> bool {
        matches!(
            self.probed_version,
            Some(VERIFIED_VERSION | P0_VERIFIED_VERSION | CURRENT_VERSION)
        ) || (self.test_candidate_settings() && self.current_selected_skill_candidate_for_live())
    }

    fn extended_lifecycle_verified(&self) -> bool {
        self.probed_version == Some(VERIFIED_VERSION)
    }

    fn prompt_lifecycle_verified(&self) -> bool {
        self.baseline_lifecycle_verified()
            && (!self.current_protocol() || self.current_root_candidate_for_live())
    }

    fn queued_submit_verified(&self) -> bool {
        self.extended_lifecycle_verified()
            || (self.current_protocol() && self.current_root_candidate_for_live())
    }

    fn current_root_candidate_for_live(&self) -> bool {
        #[cfg(test)]
        {
            self.current_root_candidate_for_live
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    fn current_selected_skill_candidate_for_live(&self) -> bool {
        #[cfg(test)]
        {
            self.current_root_candidate_for_live
                && self.current_selected_skill_candidate_for_live
                && self.options.permission_policy == PermissionPolicy::Inherit
                && self.options.local_tools.is_none()
                && self.options.selected_skills.len() == 1
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    fn catalog_direct_for_live(&self) -> bool {
        #[cfg(all(test, unix))]
        {
            self.catalog_direct_for_live
        }
        #[cfg(not(all(test, unix)))]
        {
            false
        }
    }

    fn current_setup_phases(&self) -> &'static [&'static str] {
        #[cfg(all(test, unix))]
        if self.catalog_direct_for_live {
            return &DIRECT_CATALOG_SETUP_PHASES;
        }
        &CURRENT_SETUP_PHASES
    }

    fn fixed_read_policy_verified(&self) -> bool {
        self.probed_version == Some(P0_VERIFIED_VERSION)
            && self.options.selected_skills.is_empty()
            && self.options.grok_profile.as_ref().is_some_and(|profile| {
                profile.runtime_scope_verified(
                    P0_VERIFIED_VERSION,
                    self.options.local_tools,
                    self.options.permission_policy,
                )
            })
    }

    #[cfg(test)]
    fn transaction_context(&self) -> Value {
        let kind = self.pending.as_ref().map(|pending| match &pending.kind {
            PendingKind::Initialize => "initialize",
            PendingKind::Authenticate => "authenticate",
            PendingKind::OpenSession { requested_id: None } => "new_session",
            PendingKind::OpenSession {
                requested_id: Some(_),
            } => "load_session",
            PendingKind::Prompt => "prompt",
            PendingKind::FinalOutput => "final_output",
            PendingKind::CloseSession => "close_session",
        });
        json!({"generation":self.options.generation,"next_request_id":self.next_id,
            "pending_id":self.pending.as_ref().map(|pending|pending.id),"pending_kind":kind})
    }

    fn new(options: SessionOptions) -> Self {
        let sdk = options.local_tools.map(|permissions| {
            let process_epoch = Uuid::new_v4();
            GrokSdkConnection {
                process_epoch,
                bridge: GrokMcpBridge::new(process_epoch, permissions),
                ledger: None,
                owned_process: false,
                retired: false,
                retired_at: None,
                native_calls: HashSet::new(),
                calls: HashMap::new(),
                permissions_to_write: HashMap::new(),
                approved_until: HashMap::new(),
                replies_to_write: HashMap::new(),
                native_completion_deadlines: HashMap::new(),
            }
        });
        Self {
            probed_version: None,
            paired_version: None,
            creation_catalog_session: None,
            skill_catalog: None,
            early_skill_catalogs: HashMap::new(),
            deferred_ready: None,
            #[cfg(all(test, unix))]
            skill_profile_for_live: None,
            #[cfg(all(test, unix))]
            skill_catalog_for_live: None,
            #[cfg(all(test, unix))]
            catalog_direct_for_live: false,
            #[cfg(all(test, unix))]
            direct_mcp_initialized_received: false,
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
            task_input_written: false,
            notification_ids: HashMap::new(),
            observed_prompt_ids: HashSet::new(),
            current_mcp_refresh_received: false,
            current_setup: None,
            reported_capabilities: Value::Null,
            reported_initialize_extensions: ReportedInitializeExtensions::default(),
            reported_metadata: ReportedMetadata::default(),
            closed: false,
            sdk,
            #[cfg(test)]
            lease_audit_for_live: None,
            #[cfg(test)]
            catalog_probe_for_live: None,
            #[cfg(test)]
            queued_submissions: 0,
            #[cfg(test)]
            sdk_origin_probe: None,
            #[cfg(test)]
            verified_final_histories_for_live: None,
            #[cfg(test)]
            current_root_candidate_for_live: false,
            #[cfg(test)]
            current_selected_skill_candidate_for_live: false,
            #[cfg(test)]
            test_only_1041_profile: false,
            #[cfg(test)]
            test_only_1041_native_binary: None,
        }
    }

    #[cfg(test)]
    fn from_fixture(options: SessionOptions) -> Self {
        let mut protocol = Self::new(options);
        // 历史纯离线夹具显式绑定旧版本；真实进程只能走 run_process 的版本探测。
        protocol.bind_cli_version("grok 1.0.30").unwrap();
        protocol
    }

    fn bind_cli_version(&mut self, output: &str) -> Result<(), RuntimeError> {
        if self.next_id != 0 {
            return Err(RuntimeError::Protocol(
                "Grok version probe arrived after protocol initialization".into(),
            ));
        }
        self.probed_version = None;
        self.paired_version = None;
        #[cfg(test)]
        let version = if self.test_only_1041_profile {
            (output.trim() == TEST_CANDIDATE_VERSION_OUTPUT).then_some(TEST_CANDIDATE_VERSION)
        } else {
            verified_version(output)
        };
        #[cfg(not(test))]
        let version = verified_version(output);
        let version =
            version.ok_or_else(|| RuntimeError::UnsupportedVersion(output.trim().to_owned()))?;
        // 1.0.34 只开放已有原生收据覆盖的精确读取策略；写入、技能、SDK 租约和
        // 扩展生命周期仍使用各自门禁，不能把一次读取审批外推为整版兼容。
        let latest_scope_verified = self.options.selected_skills.is_empty()
            && self.options.grok_profile.as_ref().is_some_and(|profile| {
                profile.runtime_scope_verified(
                    version,
                    self.options.local_tools,
                    self.options.permission_policy,
                )
            });
        if version != VERIFIED_VERSION
            && self.options.permission_policy != PermissionPolicy::Inherit
            && !latest_scope_verified
        {
            return Err(RuntimeError::InvalidConfiguration(crate::t!(
                "cli-agent-grok-managed-unverified"
            )));
        }
        if version != VERIFIED_VERSION
            && (self.options.local_tools.is_some()
                || (!self.options.selected_skills.is_empty()
                    && !(self.current_version_candidate(version)
                        && self.current_selected_skill_candidate_for_live())))
        {
            return Err(RuntimeError::InvalidConfiguration(crate::t!(
                "cli-agent-grok-managed-unverified"
            )));
        }
        self.probed_version = Some(version);
        Ok(())
    }

    fn served_catalog_names(&self, session: &str) -> Vec<String> {
        self.sdk
            .as_ref()
            .filter(|sdk| {
                sdk.owned_process
                    && !sdk.retired
                    && sdk.ledger.as_ref().is_some_and(|ledger| {
                        ledger.matches_catalog_connection(
                            sdk.process_epoch,
                            self.options.generation,
                            sdk.bridge.server_id(),
                            session,
                        )
                    })
            })
            .map(|sdk| sdk.bridge.served_catalog_names())
            .unwrap_or_default()
    }

    fn sdk_timed_out(&self, now: Instant) -> bool {
        self.sdk.as_ref().is_some_and(|sdk| {
            sdk.retired_at
                .is_some_and(|at| now.duration_since(at) >= REQUEST_TIMEOUT)
                || sdk.approved_until.values().any(|until| now >= *until)
                || sdk
                    .native_completion_deadlines
                    .values()
                    .any(|(_, deadline)| now >= *deadline)
                || sdk.calls.values().any(|call| {
                    !call.replied && now.duration_since(call.requested_at) >= REQUEST_TIMEOUT
                })
        })
    }

    fn retire_sdk(&mut self, events: &mut Vec<RuntimeEventKind>) {
        let Some(sdk) = &mut self.sdk else {
            return;
        };

        sdk.retired = true;
        #[cfg(test)]
        update_lease_audit(&self.lease_audit_for_live, |audit| audit.retired = true);
        sdk.retired_at.get_or_insert_with(Instant::now);
        if let Some(turn_id) = self
            .prompt
            .as_ref()
            .and_then(|prompt| prompt.native_id.as_deref())
        {
            // 已缓存的回复仍可能写入失败；应用调用账本负责回收所有未闭合调用。
            sdk.bridge.cancel_turn(turn_id);
            for (call_id, call) in &mut sdk.calls {
                if call.request.turn_id == turn_id && !call.cancelled && !call.closed {
                    call.cancelled = true;
                    events.push(RuntimeEventKind::LocalToolCancelled {
                        turn_id: turn_id.to_owned(),
                        call_id: call_id.clone(),
                    });
                }
            }
        }
        if let Some(ledger) = &mut sdk.ledger {
            ledger.retire();
        }
    }

    fn confirm_sdk_turn(&mut self) -> Result<(), RuntimeError> {
        let Some(prompt) = self
            .prompt
            .as_ref()
            .filter(|prompt| prompt.started && !prompt.finished)
        else {
            return Ok(());
        };
        let Some(turn_id) = prompt.native_id.as_deref() else {
            return Ok(());
        };
        let Some(sdk) = &mut self.sdk else {
            return Ok(());
        };

        if !sdk.retired {
            sdk.ledger
                .as_mut()
                .ok_or_else(|| {
                    RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified"))
                })?
                .begin_turn(self.options.generation, turn_id)
                .map_err(|_| {
                    RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified"))
                })?;
        }
        Ok(())
    }

    fn observe_sdk_native_tool(&mut self, message: &Value) -> Result<(), RuntimeError> {
        let current_turn = self
            .prompt
            .as_ref()
            .filter(|prompt| prompt.started && !prompt.finished)
            .and_then(|prompt| prompt.native_id.as_deref());
        if current_turn.is_none() || message["params"]["_meta"]["promptId"].as_str() != current_turn
        {
            return Ok(());
        }
        let Some(sdk) = &mut self.sdk else {
            return Ok(());
        };

        let update = &message["params"]["update"];
        let Some(call_id) = update["toolCallId"].as_str() else {
            return Ok(());
        };
        let ours = update["sessionUpdate"] == "tool_call"
            && update["_meta"]["x.ai/tool"]["name"] == "use_tool"
            && update["rawInput"]["tool_name"]
                .as_str()
                .is_some_and(|name| name.starts_with(&format!("{MCP_SERVER_NAME}__")));
        if !ours && !sdk.native_calls.contains(call_id) {
            return Ok(());
        }
        if sdk.retired {
            return Ok(());
        }
        #[cfg(test)]
        let was_closed = sdk
            .calls
            .values()
            .find(|call| call.proof.native_call_id() == call_id)
            .is_some_and(|call| {
                sdk.ledger
                    .as_ref()
                    .and_then(|ledger| ledger.state(&call.proof).ok())
                    == Some(GrokToolLeaseState::Closed)
            });
        sdk.ledger
            .as_mut()
            .ok_or_else(|| RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified")))?
            .observe_native_tool(message, Instant::now())
            .map_err(|_| RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified")))?;
        if update["status"] == "completed" {
            sdk.approved_until.remove(call_id);
        }
        if sdk
            .native_completion_deadlines
            .get(call_id)
            .is_some_and(|(proof, _)| {
                sdk.ledger
                    .as_ref()
                    .and_then(|ledger| ledger.state(proof).ok())
                    == Some(GrokToolLeaseState::Closed)
            })
        {
            sdk.native_completion_deadlines.remove(call_id);
        }
        sdk.native_calls.insert(call_id.to_owned());
        for call in sdk
            .calls
            .values_mut()
            .filter(|call| call.proof.native_call_id() == call_id)
        {
            call.closed = sdk
                .ledger
                .as_ref()
                .and_then(|ledger| ledger.state(&call.proof).ok())
                == Some(GrokToolLeaseState::Closed);
        }
        #[cfg(test)]
        update_lease_audit(&self.lease_audit_for_live, |audit| {
            audit.native_tool_frames += 1;
            audit.native_initial_inputs += u64::from(update["sessionUpdate"] == "tool_call");
            audit.native_complete_inputs += u64::from(
                update["sessionUpdate"] == "tool_call_update" && update.get("rawInput").is_some(),
            );
            let is_closed = sdk
                .calls
                .values()
                .find(|call| call.proof.native_call_id() == call_id)
                .is_some_and(|call| {
                    sdk.ledger
                        .as_ref()
                        .and_then(|ledger| ledger.state(&call.proof).ok())
                        == Some(GrokToolLeaseState::Closed)
                });
            audit.native_completions += u64::from(!was_closed && is_closed);
        });
        Ok(())
    }

    fn receive_sdk(&mut self, message: &Value) -> Result<Effects, RuntimeError> {
        if !self.reported_initialize_extensions.local_mcp_sdk_advertised {
            return Err(RuntimeError::Protocol(crate::t!(
                "cli-agent-grok-managed-unverified"
            )));
        }
        let active_turn = self
            .prompt
            .as_ref()
            .filter(|prompt| {
                prompt.started
                    && !prompt.finished
                    && prompt.completion.is_none()
                    && prompt.cancel_sent.is_none()
            })
            .and_then(|prompt| prompt.native_id.as_deref());
        let sdk = self.sdk.as_mut().expect("仅注册 SDK 通道进入此分支");
        if !sdk.owned_process || sdk.retired {
            return Err(RuntimeError::Protocol(crate::t!(
                "cli-agent-grok-managed-unverified"
            )));
        }
        if message["params"]["message"]["method"] != "tools/call" {
            let registration = sdk.bridge.receive_registration(message).map_err(|_| {
                RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified"))
            })?;
            #[cfg(test)]
            update_lease_audit(&self.lease_audit_for_live, |audit| {
                audit.registration_requests += 1;
                audit.capability_confirmed = true;
            });
            return match registration {
                GrokMcpRequest::Immediate(reply) => Ok(Effects {
                    writes: vec![reply],
                    events: Vec::new(),
                }),
                GrokMcpRequest::Duplicate => Ok(Effects::default()),
                GrokMcpRequest::Tool(_) => Err(RuntimeError::Protocol(crate::t!(
                    "cli-agent-grok-managed-unverified"
                ))),
            };
        }
        let ledger = sdk.ledger.as_mut().ok_or_else(|| {
            RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified"))
        })?;
        let (outcome, proof) = sdk
            .bridge
            .receive_with_lease(message, ledger, Instant::now())
            .map_err(|_| RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified")))?;
        #[cfg(test)]
        update_lease_audit(&self.lease_audit_for_live, |audit| {
            let params = &message["params"];
            let fields_present = [
                &message["_meta"],
                params,
                &params["_meta"],
                &params["message"],
                &params["message"]["_meta"],
                &params["message"]["params"]["_meta"],
            ]
            .into_iter()
            .any(|carrier| {
                ["sessionId", "promptId", "toolCallId"]
                    .into_iter()
                    .all(|key| carrier[key].as_str().is_some())
            });
            audit.sdk_origin_observations += 1;
            audit.full_native_sdk_origin_fields_observed = Some(
                audit.full_native_sdk_origin_fields_observed.unwrap_or(true) && fields_present,
            );
        });
        match outcome {
            GrokMcpRequest::Tool(request) => {
                if proof.process_epoch() != sdk.process_epoch
                    || proof.runtime_generation() != self.options.generation
                    || Some(proof.turn_id()) != active_turn
                    || request.turn_id != proof.turn_id()
                    || sdk.calls.contains_key(&request.call_id)
                {
                    return Err(RuntimeError::Protocol(crate::t!(
                        "cli-agent-grok-managed-unverified"
                    )));
                }
                sdk.approved_until.remove(proof.native_call_id());
                sdk.calls.insert(
                    request.call_id.clone(),
                    PendingGrokLocalTool {
                        request: request.clone(),
                        proof,
                        requested_at: Instant::now(),
                        replied: false,
                        cancelled: false,
                        closed: false,
                    },
                );
                #[cfg(test)]
                update_lease_audit(&self.lease_audit_for_live, |audit| {
                    audit.business_dispatches += 1
                });
                Ok(Effects {
                    writes: Vec::new(),
                    events: vec![RuntimeEventKind::LocalToolRequested { request }],
                })
            }
            GrokMcpRequest::Immediate(reply) => {
                let writes = vec![reply];
                sdk.approved_until.remove(proof.native_call_id());
                sdk.schedule_replies(proof, &writes)?;
                Ok(Effects {
                    writes,
                    events: Vec::new(),
                })
            }
            GrokMcpRequest::Duplicate => Ok(Effects::default()),
        }
    }

    fn respond_local_tool(
        &mut self,
        message_id: Uuid,
        turn_id: &str,
        call_id: &str,
        result: Result<Value, String>,
    ) -> Effects {
        let active = self.prompt.as_ref().is_some_and(|prompt| {
            prompt.started
                && !prompt.finished
                && prompt.completion.is_none()
                && prompt.cancel_sent.is_none()
                && prompt.native_id.as_deref() == Some(turn_id)
        });
        let Some(sdk) = &mut self.sdk else {
            return rejected_command(message_id, crate::t!("cli-agent-grok-managed-unverified"));
        };

        let Some(call) = sdk.calls.get(call_id) else {
            return rejected_command(message_id, crate::t!("cli-agent-grok-managed-unverified"));
        };
        if !active
            || sdk.retired
            || call.replied
            || call.request.turn_id != turn_id
            || call.proof.runtime_generation() != self.options.generation
        {
            return rejected_command(message_id, crate::t!("cli-agent-grok-managed-unverified"));
        }
        let proof = call.proof.clone();
        let writes = match sdk.bridge.reply_with_lease(
            &call.request,
            result,
            sdk.ledger.as_ref().expect("业务已有账本"),
            &proof,
        ) {
            Ok(writes) => writes,
            Err(_) => {
                return rejected_command(
                    message_id,
                    crate::t!("cli-agent-grok-managed-unverified"),
                );
            }
        };
        if sdk.schedule_replies(proof, &writes).is_err() {
            return rejected_command(message_id, crate::t!("cli-agent-grok-managed-unverified"));
        }
        sdk.calls.get_mut(call_id).expect("已核对本地调用").replied = true;
        Effects {
            writes,
            events: vec![RuntimeEventKind::CommandDispatched {
                message_id,
                turn_id: Some(turn_id.to_owned()),
            }],
        }
    }

    fn sdk_written_message(&mut self, message: &Value) -> Result<(), RuntimeError> {
        let Some(sdk) = &mut self.sdk else {
            return Ok(());
        };
        sdk.bridge.record_registration_written(message);
        let hash = message_fingerprint(message)?;
        if let Some((permission, allowed)) = sdk.permissions_to_write.remove(&hash) {
            let ledger = sdk.ledger.as_mut().ok_or_else(|| {
                RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified"))
            })?;
            ledger
                .record_permission(&permission, allowed, Instant::now())
                .map_err(|_| {
                    RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified"))
                })?;
            #[cfg(test)]
            update_lease_audit(&self.lease_audit_for_live, |audit| {
                audit.permission_writes_allow += u64::from(allowed);
                audit.permission_writes_deny += u64::from(!allowed);
                audit.retired |= !allowed;
            });
            if allowed {
                sdk.approved_until.insert(
                    permission["params"]["toolCall"]["toolCallId"]
                        .as_str()
                        .expect("审批已核对工具")
                        .to_owned(),
                    Instant::now() + REQUEST_TIMEOUT,
                );
            } else {
                sdk.retired = true;
                sdk.retired_at.get_or_insert_with(Instant::now);
            }
        }
        let mut written = Vec::new();
        for (call, (_, remaining)) in &mut sdk.replies_to_write {
            if remaining.remove(&hash) && remaining.is_empty() {
                written.push(call.clone());
            }
        }
        for call in written {
            let (proof, _) = sdk.replies_to_write.remove(&call).expect("待写回执存在");
            #[cfg(test)]
            let was_closed = sdk
                .ledger
                .as_ref()
                .and_then(|ledger| ledger.state(&proof).ok())
                == Some(GrokToolLeaseState::Closed);
            sdk.ledger
                .as_mut()
                .expect("已绑定回执已有账本")
                .record_reply_written(&proof)
                .map_err(|_| {
                    RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified"))
                })?;
            let native_closed = sdk
                .ledger
                .as_ref()
                .and_then(|ledger| ledger.state(&proof).ok())
                == Some(GrokToolLeaseState::Closed);
            if native_closed {
                sdk.native_completion_deadlines
                    .remove(proof.native_call_id());
            } else {
                // 错误回复也等待原生闭合；真实回写起算，缓存重发不能延长首次期限。
                sdk.native_completion_deadlines
                    .entry(proof.native_call_id().to_owned())
                    .or_insert_with(|| (proof.clone(), Instant::now() + REQUEST_TIMEOUT));
            }
            for call in sdk.calls.values_mut().filter(|call| call.proof == proof) {
                call.closed = sdk
                    .ledger
                    .as_ref()
                    .and_then(|ledger| ledger.state(&proof).ok())
                    == Some(GrokToolLeaseState::Closed);
            }
            #[cfg(test)]
            update_lease_audit(&self.lease_audit_for_live, |audit| {
                audit.reply_writes += 1;
                let is_closed = sdk
                    .ledger
                    .as_ref()
                    .and_then(|ledger| ledger.state(&proof).ok())
                    == Some(GrokToolLeaseState::Closed);
                audit.native_completions += u64::from(!was_closed && is_closed);
            });
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

    fn ready_event(&self, effective_permissions: Value) -> Result<RuntimeEventKind, RuntimeError> {
        let version = self.paired_version.ok_or_else(|| {
            RuntimeError::Protocol("Grok session has no paired CLI version".into())
        })?;
        Ok(RuntimeEventKind::SessionReady {
            verified_cli_version: Some(version.to_owned()),
            effective_permissions,
        })
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

    fn initialize(&mut self) -> Result<Value, RuntimeError> {
        if self.probed_version.is_none() {
            return Err(RuntimeError::Protocol(
                "Grok CLI version was not probed".into(),
            ));
        }
        let mut message = self.request(
            PendingKind::Initialize,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false
                }
            }),
        );
        if self.sdk.is_some() {
            message["params"]["clientCapabilities"]["_meta"]["x.ai/mcp/sdk"] = json!(true);
        }
        #[cfg(test)]
        if let Some(probe) = &self.sdk_origin_probe {
            let mut message = message;
            probe.decorate_initialize(&mut message);
            return Ok(message);
        }
        Ok(message)
    }

    fn request_timed_out(&self) -> bool {
        if self
            .deferred_ready
            .as_ref()
            .is_some_and(|(_, began)| began.elapsed() >= REQUEST_TIMEOUT)
        {
            return true;
        }
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
        // 追加输入只排队到后续回合；本地工具另由独占进程与原生租约验证。
        self.acp_command(command)
    }

    /// 已由官方 leader 验收覆盖的 ACP 子集；协调器另行限制根任务及版本。
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
                let mut sdk_events = Vec::new();
                self.retire_sdk(&mut sdk_events);
                if self.sdk.is_some() {
                    self.closed = true;
                    sdk_events.push(RuntimeEventKind::CommandDispatched {
                        message_id: command.message_id,
                        turn_id: None,
                    });
                    sdk_events.extend(self.queued.drain(..).map(|queued| {
                        RuntimeEventKind::RequestFailed {
                            message_id: queued.message_id,
                            message: RuntimeError::ControllerClosed.to_string(),
                        }
                    }));
                    self.cancel_approvals(&mut sdk_events);
                    return Effects {
                        writes: Vec::new(),
                        events: sdk_events,
                    };
                }
                if self.closed
                    || matches!(
                        self.pending.as_ref().map(|request| &request.kind),
                        Some(PendingKind::CloseSession)
                    )
                {
                    return Effects::default();
                }
                if !self.extended_lifecycle_verified()
                    || self.session_id.is_none()
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
                if !self.prompt_lifecycle_verified() {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-agent-grok-managed-unverified"),
                    );
                }
                if self.deferred_ready.is_some() {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-agent-grok-managed-unverified"),
                    );
                }
                if let Some(profile) = &self.options.grok_profile {
                    if let Err(error) = profile.verify_files(&self.options.state_dir) {
                        return rejected_command(command.message_id, error.to_string());
                    }
                }
                if matches!(self.options.permission_policy,
                    PermissionPolicy::GrokRestrictedReadV1 | PermissionPolicy::GrokRestrictedFilesV1
                )
                    && input.iter().any(|part| matches!(part, InputContent::Text(text) if text.trim_start().starts_with('/'))) {
                    return rejected_command(command.message_id, crate::t!("cli-agent-grok-fixed-command-unavailable"));
                }
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
                    || self.sdk.as_ref().is_some_and(|sdk| sdk.retired)
                    || self.session_id.is_none()
                    || self.controls.contains_key(&command.message_id)
                    || self.pending.as_ref().is_some_and(|request| {
                        !matches!(request.kind, PendingKind::Prompt | PendingKind::FinalOutput)
                    })
                    || self.queued.len() >= MAX_QUEUED_PROMPTS
                    || self.submitted_messages.len() >= MAX_NATIVE_IDENTITIES
                {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-agent-grok-managed-unverified"),
                    );
                }
                if !self.queued_submit_verified()
                    && (self.pending.is_some() || self.prompt.is_some())
                {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-agent-grok-managed-unverified"),
                    );
                }
                let mut skill = None;
                for item in &input {
                    match item {
                        InputContent::Text(_) => {}
                        InputContent::LocalImage(_) => {
                            return rejected_command(
                                command.message_id,
                                crate::t!(
                                    "cli-agent-input-images-unverified",
                                    cli = Harness::Grok.display_name()
                                ),
                            );
                        }
                        InputContent::Skill { name, path } => {
                            if self.probed_version != Some(VERIFIED_VERSION)
                                && !(self.current_protocol()
                                    && self.current_selected_skill_candidate_for_live())
                            {
                                return rejected_command(
                                    command.message_id,
                                    crate::t!("cli-agent-grok-managed-unverified"),
                                );
                            }
                            if self.options.permission_policy != PermissionPolicy::Inherit {
                                return rejected_command(
                                    command.message_id,
                                    crate::t!("cli-agent-grok-skill-policy-required"),
                                );
                            }
                            if skill.is_some() {
                                return rejected_command(
                                    command.message_id,
                                    crate::t!("cli-agent-task-skill-one-per-turn"),
                                );
                            }
                            match skills::SelectedSkill::new(name.clone(), path.clone()) {
                                Ok(selected) => skill = Some(selected),
                                Err(error) => return rejected_command(command.message_id, error),
                            }
                        }
                    }
                }
                if input.is_empty() {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-task-manager-empty-prompt"),
                    );
                }
                self.submitted_messages
                    .insert(command.message_id, fingerprint);
                let prompt = QueuedPrompt {
                    message_id: command.message_id,
                    input,
                    skill,
                };
                if self.pending.is_some() || self.prompt.is_some() {
                    // 本地等待不代表原生接收，也不向正在运行的回合发送第二个 prompt。
                    self.queued.push_back(prompt);
                    #[cfg(test)]
                    {
                        self.queued_submissions += 1;
                    }
                    Effects::default()
                } else {
                    self.start_prompt(prompt)
                }
            }
            RuntimeAction::Interrupt { turn_id } => {
                if !self.prompt_lifecycle_verified() {
                    return rejected_command(
                        command.message_id,
                        crate::t!("cli-agent-grok-managed-unverified"),
                    );
                }
                if let Some(effects) =
                    self.control_duplicate(command.message_id, &json!({"interrupt": turn_id}))
                {
                    return effects;
                }
                let Some(prompt) = self.prompt.as_mut().filter(|prompt| {
                    !prompt.finished
                        && prompt.completion.is_none()
                        && prompt.native_id.as_deref() == Some(turn_id.as_str())
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
                let mut cancelled = Vec::new();
                self.retire_sdk(&mut cancelled);
                cancelled.push(RuntimeEventKind::CommandDispatched {
                    message_id: command.message_id,
                    turn_id: Some(turn_id),
                });
                Effects {
                    writes: vec![
                        json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": self.session_id}}),
                    ],
                    events: cancelled,
                }
            }
            RuntimeAction::RespondApproval {
                approval_id,
                decision,
            } => {
                if decision == ApprovalDecision::AllowOnce {
                    if let Some(profile) = &self.options.grok_profile {
                        if let Err(error) = profile.verify_files(&self.options.state_dir) {
                            return rejected_command(command.message_id, error.to_string());
                        }
                    }
                }
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
                            && prompt.completion.is_none()
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
                if let (Some(sdk), Some(permission)) = (&self.sdk, &approval.lease_permission) {
                    let verified = !sdk.retired
                        && sdk.ledger.as_ref().is_some_and(|ledger| {
                            ledger
                                .clone()
                                .record_permission(
                                    permission,
                                    decision == ApprovalDecision::AllowOnce,
                                    Instant::now(),
                                )
                                .is_ok()
                        });
                    if !verified {
                        return rejected_command(
                            command.message_id,
                            crate::t!("cli-agent-grok-managed-unverified"),
                        );
                    }
                }
                let option_id = match decision {
                    ApprovalDecision::AllowOnce => "allow-once",
                    ApprovalDecision::DenyOnce => "reject-once",
                };
                approval.resolved = true;
                let response = json!({"jsonrpc": "2.0", "id": approval.native_id, "result": {"outcome": {"outcome": "selected", "optionId": option_id}}});
                if let (Some(sdk), Some(permission)) = (&mut self.sdk, &approval.lease_permission) {
                    sdk.permissions_to_write.insert(
                        message_fingerprint(&response).expect("JSON 审批响应可计算摘要"),
                        (permission.clone(), decision == ApprovalDecision::AllowOnce),
                    );
                }
                Effects {
                    writes: vec![response],
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
            RuntimeAction::RespondLocalTool {
                turn_id,
                call_id,
                result,
            } => self.respond_local_tool(command.message_id, &turn_id, &call_id, result),
            RuntimeAction::Steer { .. } => rejected_command(
                command.message_id,
                crate::t!("cli-agent-grok-managed-unverified"),
            ),
        }
    }

    fn start_prompt(&mut self, queued: QueuedPrompt) -> Effects {
        let content = if let Some(skill) = &queued.skill {
            let result = self
                .skill_catalog
                .as_ref()
                .ok_or_else(skills::unavailable)
                .and_then(|catalog| catalog.encode(queued.input, skill));
            match result {
                Ok(content) => content,
                Err(error) => return rejected_command(queued.message_id, error),
            }
        } else {
            queued
                .input
                .into_iter()
                .filter_map(|part| match part {
                    InputContent::Text(text) => Some(json!({"type":"text", "text":text})),
                    InputContent::LocalImage(_) | InputContent::Skill { .. } => None,
                })
                .collect()
        };
        self.prompt = Some(PendingPrompt {
            message_id: queued.message_id,
            native_id: None,
            started: false,
            finished: false,
            stream_id: None,
            closed_streams: HashSet::new(),
            last_chunk_sequence: None,
            chunks: BTreeMap::new(),
            next_chunk: 1,
            retained_text_bytes: 0,
            output: String::new(),
            cancel_sent: None,
            completion: None,
        });
        Effects {
            writes: vec![self.request(
                PendingKind::Prompt,
                "session/prompt",
                json!({"sessionId": self.session_id, "prompt": content}),
            )],
            events: Vec::new(),
        }
    }

    fn complete_rpc(&mut self, effects: &mut Effects) {
        self.cancel_approvals(&mut effects.events);
        if self.sdk.as_ref().is_some_and(|sdk| sdk.retired) {
            self.closed = true;
            effects.events.extend(self.queued.drain(..).map(|queued| {
                RuntimeEventKind::RequestFailed {
                    message_id: queued.message_id,
                    message: RuntimeError::ControllerClosed.to_string(),
                }
            }));
        }
        self.prompt = None;
        while !self.closed && self.prompt.is_none() {
            let Some(queued) = self.queued.pop_front() else {
                break;
            };
            // 坏技能只失败其所属消息，保留拒绝事件并继续检查后续排队输入。
            let next = self.start_prompt(queued);
            effects.writes.extend(next.writes);
            effects.events.extend(next.events);
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
            .filter(|prompt| {
                !prompt.finished && prompt.completion.is_none() && prompt.cancel_sent.is_none()
            })
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
        let permission_verified = match self.probed_version {
            Some(VERIFIED_VERSION) => true,
            Some(P0_VERIFIED_VERSION)
                if self.options.permission_policy == PermissionPolicy::Inherit
                    || self.fixed_read_policy_verified() =>
            {
                verified_latest_read_tool(&params["toolCall"])
            }
            Some(CURRENT_VERSION) if self.current_root_candidate_for_live() => true,
            #[cfg(test)]
            Some(TEST_CANDIDATE_VERSION)
                if self.test_candidate_settings()
                    && self.current_selected_skill_candidate_for_live() =>
            {
                verified_latest_read_tool(&params["toolCall"])
            }
            Some(_) | None => false,
        };
        if !permission_verified {
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
        if self
            .options
            .grok_profile
            .as_ref()
            .is_some_and(|profile| !profile.permits_native_request(&params["toolCall"]))
        {
            return Ok(cancelled_permission(id));
        }
        let lease_permission = if let Some(sdk) = &self.sdk {
            let ours = params["toolCall"]["rawInput"]["tool_name"]
                .as_str()
                .is_some_and(|name| name.starts_with(&format!("{MCP_SERVER_NAME}__")));
            if ours || sdk.native_calls.contains(call_id.expect("已关联工具")) {
                if sdk.retired
                    || !sdk.ledger.as_ref().is_some_and(|ledger| {
                        ledger
                            .clone()
                            .record_permission(message, true, Instant::now())
                            .is_ok()
                    })
                {
                    return Ok(cancelled_permission(id));
                }
                Some(message.clone())
            } else {
                None
            }
        } else {
            None
        };
        let turn_id = turn_id.expect("关联成功后必须存在回合").to_owned();
        self.approvals.insert(
            approval_id.clone(),
            PendingApproval {
                native_id: id.clone(),
                turn_id: turn_id.clone(),
                tool_call_id: call_id.expect("关联成功后必须存在工具调用").to_owned(),
                resolved: false,
                lease_permission,
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
        // RPC 结束不代表 gateway 已发送全部文本；保留当前回合，直到原生历史包含对应终态。
        prompt.completion = Some(PendingCompletion {
            outcome,
            started_at: Instant::now(),
            retry_at: None,
        });
        let mut effects = Effects::default();
        self.cancel_approvals(&mut effects.events);
        effects.writes.push(self.request_final_output());
        Ok(effects)
    }

    fn request_final_output(&mut self) -> Value {
        self.request(
            PendingKind::FinalOutput,
            "_x.ai/session/updates",
            json!({
                "sessionId": self.session_id, "cwd": self.options.cwd,
                "offset": 0, "limit": MAX_NATIVE_IDENTITIES
            }),
        )
    }

    fn poll_final_output(&mut self) -> Effects {
        #[cfg(test)]
        if let Some(probe) = &self.catalog_probe_for_live {
            if let Some(session) = self.session_id.as_deref() {
                if let Some(request) = probe.poll_pull(session, &self.served_catalog_names(session))
                {
                    return Effects {
                        writes: vec![request],
                        events: Vec::new(),
                    };
                }
            }
        }
        let Some(completion) = self
            .prompt
            .as_ref()
            .and_then(|prompt| prompt.completion.as_ref())
        else {
            return Effects::default();
        };
        if completion.started_at.elapsed() >= REQUEST_TIMEOUT {
            // 收束期限失败只能降级，不能以安静或超时证明成功；退出防止迟到查询污染下一轮。
            self.closed = true;
            self.pending = None;
            let mut effects = self.unverified_final_output("native history watermark timed out");
            effects.events.extend(self.queued.drain(..).map(|queued| {
                RuntimeEventKind::RequestFailed {
                    message_id: queued.message_id,
                    message: RuntimeError::ControllerClosed.to_string(),
                }
            }));
            return effects;
        }
        if self.pending.is_none() && completion.retry_at.is_some_and(|at| Instant::now() >= at) {
            if let Some(completion) = self
                .prompt
                .as_mut()
                .and_then(|prompt| prompt.completion.as_mut())
            {
                completion.retry_at = None;
            }
            return Effects {
                writes: vec![self.request_final_output()],
                events: Vec::new(),
            };
        }
        Effects::default()
    }

    fn finish_transport(&mut self, result: &Result<(), RuntimeError>) -> Effects {
        let mut sdk_events = Vec::new();
        self.retire_sdk(&mut sdk_events);
        if self.sdk.is_some() {
            self.cancel_approvals(&mut sdk_events);
        }
        if !self
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.completion.is_some())
        {
            return Effects {
                writes: Vec::new(),
                events: sdk_events,
            };
        }
        // 正常断开也可能打断最终快照；关闭连接不等于结果已完整，更不等于原生取消。
        self.closed = true;
        self.pending = None;
        let reason = match result {
            Ok(()) => "native history collection stopped",
            Err(_) => "native history connection lost",
        };
        let mut effects = self.unverified_final_output(reason);
        effects.events.extend(sdk_events);
        effects.events.extend(self.queued.drain(..).map(|queued| {
            RuntimeEventKind::RequestFailed {
                message_id: queued.message_id,
                message: RuntimeError::ControllerClosed.to_string(),
            }
        }));
        effects
    }

    fn unverified_final_output(&mut self, reason: &'static str) -> Effects {
        log::debug!("Grok final history verification failed: reason={reason}");
        #[cfg(test)]
        if std::env::var_os("INFINISHELL_GROK_LIVE_ROOT").is_some() {
            // 隔离验收仅记录内部固定原因，不记录模型正文、工具参数或凭据。
            eprintln!(
                "GROK_FINAL_HISTORY_UNVERIFIED {}",
                json!({"reason": reason, "reason_sha256": format!("{:x}", Sha256::digest(reason)),
                    "received_output_bytes": self.prompt.as_ref().map(|prompt| prompt.output.len()),
                    "received_output_sha256": self.prompt.as_ref().map(|prompt| format!("{:x}", Sha256::digest(&prompt.output)))})
            );
        }
        let Some(prompt) = self.prompt.as_mut() else {
            return Effects::default();
        };
        let Some(turn_id) = prompt.native_id.clone() else {
            return Effects::default();
        };
        prompt.finished = true;
        let mut effects = Effects {
            writes: Vec::new(),
            events: vec![RuntimeEventKind::TurnFinished {
                turn_id,
                outcome: TurnOutcome::Failed {
                    message: crate::t!("cli-agent-grok-output-unverified"),
                },
                // 原生已经结束但结果完整性未知，保留实际收到的部分文本供用户查看。
                output: prompt.output.clone(),
            }],
        };
        self.complete_rpc(&mut effects);
        effects
    }

    fn final_output_result(&mut self, result: &Value) -> Effects {
        let Some(prompt) = self.prompt.as_ref() else {
            return Effects::default();
        };
        let Some(completion) = prompt.completion.as_ref() else {
            return Effects::default();
        };
        let Some(turn_id) = prompt.native_id.clone() else {
            return Effects::default();
        };
        let outcome = completion.outcome.clone();
        let snapshot = verified_final_snapshot(
            result,
            self.session_id.as_deref().unwrap_or_default(),
            &turn_id,
            &outcome,
        );
        #[cfg(test)]
        if std::env::var_os("INFINISHELL_GROK_LIVE_ROOT").is_some() {
            // 只保留回放边界、数量与摘要，用于定位历史继续失败；正文不进入验收日志。
            let (verdict, reason, output_bytes) = match &snapshot {
                Ok(Some((output, _))) if output.starts_with(&prompt.output) => {
                    ("verified", None, Some(output.len()))
                }
                Ok(Some((output, _))) => (
                    "rejected",
                    Some("native history conflicts with received text"),
                    Some(output.len()),
                ),
                Ok(None) => ("pending", None, None),
                Err(reason) => ("rejected", Some(*reason), None),
            };
            eprintln!(
                "GROK_FINAL_HISTORY_SNAPSHOT {}",
                json!({"verdict":verdict,"reason":reason,
                    "update_count":result["updates"].as_array().map(Vec::len),
                    "has_more":result["hasMore"].as_bool(),"total_count":result["totalCount"].as_u64(),
                    "last_event_id_sha256":result["lastEventId"].as_str().map(|id|format!("{:x}",Sha256::digest(id))),
                    "native_turn_id_sha256":format!("{:x}",Sha256::digest(&turn_id)),
                    "received_output_bytes":prompt.output.len(),"collected_output_bytes":output_bytes})
            );
        }
        let (output, watermark) = match snapshot {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                self.prompt
                    .as_mut()
                    .and_then(|prompt| prompt.completion.as_mut())
                    .expect("正在等待终态回放")
                    .retry_at = Some(Instant::now() + SNAPSHOT_RETRY_INTERVAL);
                return Effects::default();
            }
            Err(reason) => return self.unverified_final_output(reason),
        };
        if !output.starts_with(&prompt.output) {
            return self.unverified_final_output("native history conflicts with received text");
        }
        #[cfg(test)]
        if std::env::var_os("INFINISHELL_GROK_LIVE_ROOT").is_some() {
            // 协调器验收只记录真实历史完成水位与输出摘要，不记录模型正文。
            eprintln!(
                "GROK_NATIVE_FINAL_HISTORY_VERIFIED {}",
                json!({"runtime_generation":self.options.generation,
                    "session_id":self.session_id,"turn_id":turn_id,
                    "completion_watermark":watermark,"outcome":outcome,
                    "full_output_bytes":output.len(),
                    "full_output_sha256":format!("{:x}",Sha256::digest(&output))})
            );
        }
        #[cfg(test)]
        if let Some(histories) = &self.verified_final_histories_for_live {
            histories.lock().expect("原生最终回放锁未被破坏").insert(
                turn_id.clone(),
                live_tests::NativeFinalHistory {
                    session_id: self.session_id.clone().expect("最终回放已有会话身份"),
                    turn_id: turn_id.clone(),
                    completion_watermark: watermark.clone(),
                    replay: result.clone(),
                },
            );
        }
        let suffix = output[prompt.output.len()..].to_owned();
        let mut events = Vec::new();
        if !suffix.is_empty() {
            events.push(RuntimeEventKind::TextDelta {
                turn_id: turn_id.clone(),
                item_id: format!("{turn_id}:history:{watermark}"),
                text: suffix,
            });
        }
        events.push(RuntimeEventKind::TurnFinished {
            turn_id,
            outcome,
            output,
        });
        let mut effects = Effects {
            writes: Vec::new(),
            events,
        };
        self.complete_rpc(&mut effects);
        effects
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
            Some("agent_message_chunk" | "agent_thought_chunk") => {
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
                // 官方模型的思考与正文共用分片编号；思考仅保留摘要参与排序，不保留或展示正文。
                let fingerprint = message_fingerprint(update)?;
                let visible_text =
                    (update["sessionUpdate"] == "agent_message_chunk").then(|| text.to_owned());
                if let Some(previous) = prompt.stream_id.filter(|previous| *previous != stream_id) {
                    // streamStartMs 只标识响应，不参与排序；已实测 eventId 的计数后缀保持递增。
                    if prompt.closed_streams.contains(&stream_id)
                        || prompt
                            .chunks
                            .keys()
                            .next_back()
                            .is_some_and(|last| *last >= prompt.next_chunk)
                        || prompt
                            .last_chunk_sequence
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
                    return if previous.0 == sequence && previous.1 == fingerprint {
                        Ok(Effects::default())
                    } else {
                        Err(RuntimeError::Protocol(
                            "conflicting Grok text chunk identity".into(),
                        ))
                    };
                }
                if prompt
                    .last_chunk_sequence
                    .is_some_and(|last| sequence <= last)
                {
                    return Err(RuntimeError::Protocol(
                        "Grok text event regressed behind published output".into(),
                    ));
                }
                if prompt.chunks.len() >= MAX_NATIVE_IDENTITIES
                    || prompt.retained_text_bytes + visible_text.as_ref().map_or(0, String::len)
                        > MAX_LINE_BYTES
                {
                    return Err(RuntimeError::Protocol(
                        "Grok output retention limit reached".into(),
                    ));
                }
                prompt.retained_text_bytes += visible_text.as_ref().map_or(0, String::len);
                prompt
                    .chunks
                    .insert(chunk_id, (sequence, fingerprint, visible_text));
                if !prompt.started {
                    prompt.started = true;
                    effects.events.push(RuntimeEventKind::TurnStarted {
                        turn_id: turn_id.to_owned(),
                    });
                }
                while let Some((sequence, _, text)) = prompt.chunks.get(&prompt.next_chunk) {
                    if prompt
                        .last_chunk_sequence
                        .is_some_and(|last| *sequence <= last)
                    {
                        return Err(RuntimeError::Protocol(
                            "Grok chunk order conflicts with native event order".into(),
                        ));
                    }
                    prompt.last_chunk_sequence = Some(*sequence);
                    if let Some(text) = text {
                        prompt.output.push_str(text);
                        effects.events.push(RuntimeEventKind::TextDelta {
                            turn_id: turn_id.to_owned(),
                            item_id: format!("{turn_id}:{stream_id}"),
                            text: text.clone(),
                        });
                    }
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
                if self.current_protocol() && !self.current_root_candidate_for_live() {
                    return Err(RuntimeError::Protocol(crate::t!(
                        "cli-agent-grok-managed-unverified"
                    )));
                }
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
        if self.options.grok_profile.is_some() {
            // 独占冷进程同一固定输入；此请求不作为 warm load 降权或 native policy ACK。
            params["_meta"] = super::grok_profile::GrokCreationPolicyV1::creation_meta();
        }
        if self.sdk.is_some() && !self.reported_initialize_extensions.local_mcp_sdk_advertised {
            return Err(RuntimeError::Protocol(crate::t!(
                "cli-agent-grok-managed-unverified"
            )));
        }
        if let Some(sdk) = &self.sdk {
            params["_meta"]["x.ai/mcp/servers"] = sdk.bridge.registration();
        }
        #[cfg(test)]
        if let Some(probe) = &self.sdk_origin_probe {
            probe.decorate_open_session(&mut params);
        }
        self.current_setup =
            (self.current_protocol() && requested_id.is_none()).then(CurrentSetupProgress::default);
        Ok(self.request(PendingKind::OpenSession { requested_id }, method, params))
    }

    fn receive(&mut self, message: Value) -> Result<Effects, RuntimeError> {
        #[cfg(test)]
        if let Some(probe) = &self.catalog_probe_for_live {
            probe.guard_receive(&message)?;
            if let Some(tools) = probe.pull_response(&message, self.session_id.as_deref())? {
                let session = self
                    .session_id
                    .as_deref()
                    .ok_or_else(|| RuntimeError::Protocol("目录预检缺少会话".into()))?;
                let names = self.served_catalog_names(session);
                self.options
                    .grok_profile
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Protocol("目录预检缺少固定策略".into()))?
                    .verify_catalog_with_mcp(Some(&tools), self.task_input_written, &names)?;
                probe.observe_pull(tools, &names);
                return Ok(Effects::default());
            }
        }
        #[cfg(test)]
        if let Some(probe) = &self.sdk_origin_probe {
            // 在版本与身份前置校验之前保存安全形状，畸形帧也不能丢失终止诊断。
            let unexpected = message.get("method").is_none()
                && message.get("id").is_some()
                && message["id"].as_u64().is_none_or(|id| {
                    self.pending.as_ref().is_none_or(|pending| pending.id != id)
                        && !self.responses.contains_key(&id)
                });
            probe.observe_response_diagnostic(&message, self.transaction_context(), unexpected);
        }
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
        #[cfg(test)]
        if let Some(probe) = self.sdk_origin_probe.as_mut()
            && let Some(effects) = probe.receive(&message)?
        {
            return Ok(effects);
        }
        if let Some(method) = message.get("method") {
            if !method.is_string()
                || message.get("result").is_some()
                || message.get("error").is_some()
            {
                return Err(RuntimeError::Protocol("invalid Grok server message".into()));
            }
            if matches!(
                method.as_str(),
                Some("x.ai/mcp/sdk_call" | "_x.ai/mcp/sdk_call")
            ) && self.sdk.is_some()
            {
                return self.receive_sdk(&message);
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
        // 固定原生版本会转发 CLI 自己的技能维护响应；它不确认或完成应用请求。
        if internal_skills_reload_success(&message) {
            return Ok(Effects::default());
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
            if matches!(kind, PendingKind::FinalOutput) {
                return Ok(self.unverified_final_output("native history query failed"));
            }
            // 认证、恢复和关闭失败都不重新新建会话或继续尝试其他请求。
            return Err(RuntimeError::Protocol(format!(
                "Grok ACP request failed (code {code:?})"
            )));
        }
        let result = match message.get("result").filter(|value| value.is_object()) {
            Some(result) => result,
            None if matches!(kind, PendingKind::FinalOutput) => {
                self.pending = None;
                self.responses.insert(id, fingerprint);
                return Ok(self.unverified_final_output("native history response is malformed"));
            }
            None => {
                return Err(RuntimeError::Protocol(
                    "Grok response has no result object".into(),
                ));
            }
        };
        self.pending = None;
        self.responses.insert(id, fingerprint);
        let mut effects = Effects::default();
        match kind {
            PendingKind::Initialize => {
                if result["protocolVersion"].as_u64() != Some(1)
                    || self.probed_version.is_none()
                    || result["_meta"]["agentVersion"].as_str() != self.probed_version
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
                let (initialize_extensions, available_commands) =
                    ReportedInitializeExtensions::decode(result)?;
                // 只选择原生明确公布的无交互认证，不读取凭据或代替用户配置模型后端。
                // 1.0.40 已登录 OAuth 的实测形状必须同时包含 cached_token 与 grok.com；
                // 仅有交互式 grok.com 时继续关闭产品适配器，不能在后台弹出登录流程。
                let methods = result["authMethods"].as_array();
                let method = if self.current_protocol() {
                    let ids = methods.map(|methods| {
                        methods
                            .iter()
                            .map(|method| method["id"].as_str())
                            .collect::<Vec<_>>()
                    });
                    if ids.as_deref() == Some(&[Some("cached_token"), Some("grok.com")])
                        && result["_meta"]["defaultAuthMethodId"] == "cached_token"
                    {
                        Some("cached_token")
                    } else {
                        None
                    }
                } else {
                    ["cached_token", "xai.api_key"].into_iter().find(|id| {
                        methods.is_some_and(|methods| {
                            methods
                                .iter()
                                .any(|method| method["id"].as_str() == Some(*id))
                        })
                    })
                };
                let Some(method) = method else {
                    return Err(RuntimeError::InvalidConfiguration(crate::t!(
                        "cli-agent-grok-managed-login-required"
                    )));
                };
                // 所有字段完成同一响应的版本和形状校验后再原子绑定；旧 initialize、
                // 重投或缺字段响应不能替换当前连接的能力快照。
                self.paired_version = self.probed_version;
                self.reported_capabilities = capabilities.clone();
                self.reported_initialize_extensions = initialize_extensions;
                self.reported_metadata.available_commands = available_commands;
                effects.writes.push(self.request(
                    PendingKind::Authenticate,
                    "authenticate",
                    json!({"methodId": method, "_meta": {"headless": true}}),
                ));
            }
            PendingKind::Authenticate => effects.writes.push(self.open_session()?),
            PendingKind::OpenSession { requested_id } => {
                let new_session = requested_id.is_none();
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
                if self.current_protocol() && new_session {
                    let direct_catalog = self.catalog_direct_for_live();
                    let setup = self.current_setup.as_mut().ok_or_else(|| {
                        RuntimeError::Protocol("missing Grok 1.0.40 setup sequence".into())
                    })?;
                    if (!direct_catalog && setup.next_phase != 5)
                        || (direct_catalog && !(5..=11).contains(&setup.next_phase))
                        || setup.response_received
                    {
                        return Err(RuntimeError::Protocol(
                            "incomplete or mismatched Grok 1.0.40 setup sequence".into(),
                        ));
                    }
                    if setup
                        .session_id
                        .as_deref()
                        .is_some_and(|existing| existing != id)
                    {
                        return Err(RuntimeError::Protocol(
                            "Grok setup changed its session id".into(),
                        ));
                    }
                    setup.session_id.get_or_insert_with(|| id.clone());
                    setup.response_received = true;
                }
                self.session_id = Some(id.clone());
                self.skill_catalog = self.early_skill_catalogs.remove(&id);
                self.early_skill_catalogs.clear();
                if let Some(sdk) = &mut self.sdk {
                    if !sdk.owned_process || sdk.retired {
                        return Err(RuntimeError::Protocol(crate::t!(
                            "cli-agent-grok-managed-unverified"
                        )));
                    }
                    sdk.ledger = Some(
                        GrokToolLeaseLedger::new(
                            sdk.process_epoch,
                            self.options.generation,
                            sdk.bridge.server_id().to_owned(),
                            MCP_SERVER_NAME.into(),
                            id,
                        )
                        .map_err(|_| {
                            RuntimeError::Protocol(crate::t!("cli-agent-grok-managed-unverified"))
                        })?,
                    );
                }
                self.update_metadata(result)?;
                let baseline_lifecycle_verified = self.baseline_lifecycle_verified();
                let prompt_lifecycle_verified = self.prompt_lifecycle_verified();
                let extended_lifecycle_verified = self.extended_lifecycle_verified();
                let effective_permissions = json!({
                    "requestedPolicy": self.options.grok_profile.as_ref().map(|profile| json!(profile.permission_policy())).unwrap_or_else(|| json!("inherit")), "effectiveNativePolicy": null,
                    "appCreationPolicyApplied": self.options.grok_profile.is_some(),
                    "grokCreationPolicyV1": self.options.grok_profile,
                    "permissionEnforcementVerified": false,
                    "reportedCapabilities": self.reported_capabilities,
                    "reportedInitializeExtensions": self.reported_initialize_extensions,
                    "reportedMetadata": self.reported_metadata,
                    "verifiedCapabilities": {
                        "newSession": baseline_lifecycle_verified, "emptyHistoryRecovery": extended_lifecycle_verified, "closeSession": extended_lifecycle_verified,
                        "submit": prompt_lifecycle_verified, "queuedSubmit": self.queued_submit_verified(), "steer": false, "approval": prompt_lifecycle_verified,
                        "cancel": prompt_lifecycle_verified, "resume": prompt_lifecycle_verified && self.reported_capabilities["loadSession"] == true,
                        "localTools": extended_lifecycle_verified && self.reported_initialize_extensions.local_mcp_sdk_advertised && self.sdk.is_some(),
                        "childTasks": extended_lifecycle_verified && self.reported_initialize_extensions.local_mcp_sdk_advertised && self.options.grok_profile.as_ref().and_then(|profile| profile.local_tools()).is_some_and(|tools| tools.allow_spawn)
                    }
                });
                super::permissions::verify_effective_permissions(
                    self.options.permission_ceiling.as_ref(),
                    "grok",
                    &self.options.cwd,
                    &effective_permissions,
                )?;
                if (self.current_protocol() && new_session)
                    || (self.options.grok_profile.is_some()
                        && self.creation_catalog_session != self.session_id)
                    || (self.options.target == SessionTarget::New
                        && !self.options.selected_skills.is_empty()
                        && self.skill_catalog.is_none())
                {
                    self.deferred_ready = Some((effective_permissions, Instant::now()));
                } else {
                    effects
                        .events
                        .push(self.ready_event(effective_permissions)?);
                }
                if self.current_protocol() && new_session && self.catalog_direct_for_live() {
                    let ready = self.finish_current_setup()?;
                    effects.writes.extend(ready.writes);
                    effects.events.extend(ready.events);
                }
            }
            PendingKind::Prompt => {
                effects = self.prompt_result(result)?;
                if self.prompt.as_ref().is_some_and(|prompt| prompt.finished) {
                    self.complete_rpc(&mut effects);
                }
            }
            PendingKind::FinalOutput => effects = self.final_output_result(result),
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

    fn record_notification_id(&mut self, message: &Value) -> Result<bool, RuntimeError> {
        let params = &message["params"];
        if let Some(event_id) = params["_meta"]["eventId"].as_str() {
            if !valid_native_id(event_id) {
                return Err(RuntimeError::Protocol(
                    "invalid Grok notification id".into(),
                ));
            }
            let fingerprint = message_fingerprint(message)?;
            if let Some(previous) = self.notification_ids.get(event_id) {
                return if previous == &fingerprint {
                    Ok(false)
                } else {
                    Err(RuntimeError::Protocol(
                        "conflicting Grok native notification id".into(),
                    ))
                };
            }
            self.notification_ids
                .insert(event_id.to_owned(), fingerprint);
        }
        Ok(true)
    }

    fn current_setup_notification(&mut self, message: &Value) -> Result<Effects, RuntimeError> {
        if !self.current_protocol() {
            return Err(RuntimeError::Protocol(
                "unexpected Grok session setup notification".into(),
            ));
        }
        let outer = message
            .as_object()
            .filter(|outer| {
                outer.len() == 3
                    && outer.contains_key("jsonrpc")
                    && outer.contains_key("method")
                    && outer.contains_key("params")
            })
            .ok_or_else(|| RuntimeError::Protocol("invalid Grok setup envelope".into()))?;
        let params = outer["params"]
            .as_object()
            .filter(|params| {
                params.len() == 3
                    && params.contains_key("method")
                    && params.contains_key("phase")
                    && params.contains_key("sessionId")
            })
            .ok_or_else(|| RuntimeError::Protocol("invalid Grok setup fields".into()))?;
        if params["method"] != "session/new" {
            return Err(RuntimeError::Protocol(
                "unexpected Grok setup method".into(),
            ));
        }
        let direct_catalog = self.catalog_direct_for_live();
        let setup_phases = self.current_setup_phases();
        let setup = self
            .current_setup
            .as_mut()
            .ok_or_else(|| RuntimeError::Protocol("missing Grok 1.0.40 setup state".into()))?;
        let waiting_for_response = matches!(
            self.pending.as_ref().map(|pending| &pending.kind),
            Some(PendingKind::OpenSession { requested_id: None })
        );
        if setup.next_phase < 5 {
            if !waiting_for_response
                || setup.response_received
                || setup.session_id.is_some()
                || setup.next_phase == 0 && !self.current_mcp_refresh_received
            {
                return Err(RuntimeError::Protocol(
                    "unexpected Grok pre-response setup phase".into(),
                ));
            }
        } else if (!direct_catalog && (waiting_for_response || !setup.response_received))
            || (direct_catalog && !waiting_for_response && !setup.response_received)
        {
            return Err(RuntimeError::Protocol(
                "Grok post-response setup phase arrived before session/new response".into(),
            ));
        }
        let expected = setup_phases
            .get(setup.next_phase)
            .ok_or_else(|| RuntimeError::Protocol("duplicate Grok 1.0.40 setup phase".into()))?;
        if params["phase"].as_str() != Some(expected) {
            return Err(RuntimeError::Protocol(
                "unknown or out-of-order Grok 1.0.40 setup phase".into(),
            ));
        }
        if setup.next_phase < 5 {
            if !params["sessionId"].is_null() {
                return Err(RuntimeError::Protocol(
                    "Grok setup declared a session before persistence".into(),
                ));
            }
        } else {
            let session_id = params["sessionId"]
                .as_str()
                .filter(|session_id| valid_native_id(session_id))
                .ok_or_else(|| {
                    RuntimeError::Protocol("Grok setup has no valid session id".into())
                })?;
            if setup
                .session_id
                .as_deref()
                .is_some_and(|existing| existing != session_id)
            {
                return Err(RuntimeError::Protocol(
                    "Grok setup changed its session id".into(),
                ));
            }
            setup
                .session_id
                .get_or_insert_with(|| session_id.to_owned());
        }
        setup.next_phase += 1;
        self.finish_current_setup()
    }

    fn finish_current_setup(&mut self) -> Result<Effects, RuntimeError> {
        let setup_phase_count = self.current_setup_phases().len();
        let complete = self.current_setup.as_ref().is_some_and(|setup| {
            setup.next_phase == setup_phase_count
                && setup.response_received
                && setup.session_id.is_some()
                && setup.models_received
                && setup.settings_received
                && setup.announcements_received == 2
                && setup.commands_received
        });
        if !complete {
            return Ok(Effects::default());
        }
        self.current_setup = None;
        let (effective_permissions, began) = self.deferred_ready.take().ok_or_else(|| {
            RuntimeError::Protocol("Grok setup completed without a deferred ready event".into())
        })?;
        if began.elapsed() >= REQUEST_TIMEOUT {
            return Err(RuntimeError::RequestTimedOut);
        }
        Ok(Effects {
            writes: Vec::new(),
            events: vec![self.ready_event(effective_permissions)?],
        })
    }

    fn notification(&mut self, message: &Value) -> Result<Effects, RuntimeError> {
        if message["method"] == CURRENT_SETUP_METHOD {
            return self.current_setup_notification(message);
        }
        #[cfg(all(test, unix))]
        if self.catalog_direct_for_live && message["method"] == "_x.ai/mcp_initialized" {
            let valid = message.as_object().is_some_and(|outer| outer.len() == 3)
                && message["jsonrpc"] == "2.0"
                && message["params"].as_object().is_some_and(|params| {
                    params.len() == 3
                        && params["sessionId"].as_str() == self.session_id.as_deref()
                        && params["mcpToolCount"].as_u64() == Some(0)
                        && params["elapsedMs"].as_u64().is_some()
                })
                && self.current_setup.is_some()
                && !self.direct_mcp_initialized_received;
            if !valid {
                return Err(RuntimeError::Protocol(
                    "unexpected Grok direct catalog MCP initialization".into(),
                ));
            }
            self.direct_mcp_initialized_received = true;
            return Ok(Effects::default());
        }
        let current_resume_replay = self.current_protocol()
            && self.session_id.is_none()
            && self.current_setup.is_none()
            && matches!(
                message["method"].as_str(),
                Some("session/update" | "_x.ai/session/update")
            )
            && message["params"]["_meta"]["isReplay"] == true
            && self.pending.as_ref().is_some_and(|pending| {
                matches!(
                    &pending.kind,
                    PendingKind::OpenSession {
                        requested_id: Some(requested_id)
                    } if message["params"]["sessionId"].as_str() == Some(requested_id)
                )
            });
        if self.current_protocol()
            && (self.session_id.is_none() || self.current_setup.is_some())
            && !current_resume_replay
        {
            if message
                == &json!({
                    "jsonrpc": "2.0",
                    "method": "_x.ai/mcp/servers_updated",
                    "params": {"mcpServers": []}
                })
            {
                if self.current_mcp_refresh_received
                    || self
                        .current_setup
                        .as_ref()
                        .is_some_and(|setup| setup.next_phase != 0)
                {
                    return Err(RuntimeError::Protocol(
                        "duplicate or late Grok MCP refresh".into(),
                    ));
                }
                self.current_mcp_refresh_received = true;
                return Ok(Effects::default());
            }
            if message["method"] == "_x.ai/models/update" {
                let models = current_models_update(message)?;
                let setup = self
                    .current_setup
                    .as_mut()
                    .filter(|setup| setup.next_phase >= 5 && !setup.models_received);
                let Some(setup) = setup else {
                    return Err(RuntimeError::Protocol(
                        "unexpected Grok models update position".into(),
                    ));
                };
                setup.models_received = true;
                self.reported_metadata.models = Some(models);
                return self.finish_current_setup();
            }
            if message["method"] == "_x.ai/settings/update" {
                validate_current_settings_update(message, self.test_candidate_settings())?;
                let setup = self
                    .current_setup
                    .as_mut()
                    .filter(|setup| setup.next_phase >= 5 && !setup.settings_received);
                let Some(setup) = setup else {
                    return Err(RuntimeError::Protocol(
                        "unexpected Grok settings update position".into(),
                    ));
                };
                setup.settings_received = true;
                return self.finish_current_setup();
            }
            if message["method"] == "_x.ai/announcements/update" {
                let generation = validate_current_announcements_update(message)?;
                let setup = self.current_setup.as_mut().filter(|setup| {
                    setup.next_phase >= 5
                        && setup.announcements_received < 2
                        && setup
                            .announcement_generation
                            .is_none_or(|previous| generation > previous)
                });
                let Some(setup) = setup else {
                    return Err(RuntimeError::Protocol(
                        "unexpected Grok announcements update position".into(),
                    ));
                };
                setup.announcements_received += 1;
                setup.announcement_generation = Some(generation);
                return self.finish_current_setup();
            }
            if message["method"] == "session/update"
                && message["params"]["update"]["sessionUpdate"] == "available_commands_update"
            {
                let session_id = message["params"]["sessionId"]
                    .as_str()
                    .filter(|session_id| valid_native_id(session_id))
                    .ok_or_else(|| {
                        RuntimeError::Protocol(
                            "Grok command catalog arrived without a session".into(),
                        )
                    })?
                    .to_owned();
                let setup = self
                    .current_setup
                    .as_ref()
                    .filter(|setup| setup.next_phase >= 5 && !setup.commands_received)
                    .ok_or_else(|| {
                        RuntimeError::Protocol("unexpected Grok command catalog position".into())
                    })?;
                if setup
                    .session_id
                    .as_deref()
                    .is_some_and(|existing| existing != session_id)
                {
                    return Err(RuntimeError::Protocol(
                        "Grok command catalog changed its session id".into(),
                    ));
                }
                #[cfg(all(test, unix))]
                if let Some(probe) = &self.skill_catalog_for_live {
                    probe.observe(message, self.submitted_messages.is_empty());
                }
                validate_current_available_commands_update(
                    message,
                    &session_id,
                    self.options.selected_skills.first(),
                )?;
                if self.options.permission_policy == PermissionPolicy::Inherit {
                    let catalog = skills::SkillCatalog::from_native(
                        &message["params"]["update"]["availableCommands"],
                    )
                    .map_err(RuntimeError::Protocol)?;
                    if self.session_id.as_deref() == Some(session_id.as_str()) {
                        self.skill_catalog = Some(catalog);
                    } else {
                        self.early_skill_catalogs
                            .insert(session_id.clone(), catalog);
                    }
                }
                let setup = self.current_setup.as_mut().expect("setup 已验证存在");
                setup.session_id.get_or_insert(session_id);
                setup.commands_received = true;
                return self.finish_current_setup();
            }
            return Err(RuntimeError::Protocol(
                "unexpected Grok 1.0.40 handshake notification".into(),
            ));
        }
        // 原生可能先发目录再回复 session/new；探针和生产目录都按真实会话关联。
        #[cfg(all(test, unix))]
        if let Some(probe) = &self.skill_catalog_for_live {
            probe.observe(message, self.submitted_messages.is_empty());
        }
        let method = message["method"].as_str().unwrap_or_default();
        let params = &message["params"];
        if method == "session/update"
            && params["update"]["sessionUpdate"] == "available_commands_update"
        {
            if let Some(profile) = &self.options.grok_profile {
                if self
                    .deferred_ready
                    .as_ref()
                    .is_some_and(|(_, began)| began.elapsed() >= REQUEST_TIMEOUT)
                {
                    return Err(RuntimeError::RequestTimedOut);
                }
                let session = params["sessionId"]
                    .as_str()
                    .filter(|session| valid_native_id(session))
                    .ok_or_else(|| {
                        super::permissions::rejected(
                            None,
                            &Value::Null,
                            "grok_creation_catalog_session_missing",
                            false,
                        )
                    })?;
                if self
                    .session_id
                    .as_deref()
                    .is_some_and(|current| current != session)
                {
                    return Ok(Effects::default());
                }
                let served_names = if params["_meta"]
                    .get("isReplay")
                    .is_none_or(|value| value == false)
                {
                    self.served_catalog_names(session)
                } else {
                    Vec::new()
                };
                #[cfg(test)]
                if let Some(probe) = &self.catalog_probe_for_live {
                    probe.observe_catalog(params, &served_names);
                }
                profile.verify_catalog_with_mcp(
                    params["update"]["_meta"].get("tools"),
                    self.task_input_written,
                    &served_names,
                )?;
                self.creation_catalog_session = Some(session.to_owned());
                if self.session_id.is_some() {
                    if let Some((effective_permissions, _)) = self.deferred_ready.take() {
                        return Ok(Effects {
                            writes: Vec::new(),
                            events: vec![self.ready_event(effective_permissions)?],
                        });
                    }
                }
            }
        }
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
            if method == "session/update"
                && params["update"]["sessionUpdate"] == "available_commands_update"
                && params["_meta"]
                    .get("isReplay")
                    .is_none_or(|value| value == false)
                && self.options.permission_policy == PermissionPolicy::Inherit
            {
                if let Some(PendingRequest {
                    kind: PendingKind::OpenSession { requested_id },
                    ..
                }) = &self.pending
                {
                    if let Some(id) = params["sessionId"]
                        .as_str()
                        .filter(|id| valid_native_id(id))
                    {
                        if requested_id
                            .as_deref()
                            .is_none_or(|requested| requested == id)
                        {
                            if self.early_skill_catalogs.len() >= 8
                                && !self.early_skill_catalogs.contains_key(id)
                            {
                                return Err(RuntimeError::Protocol(skills::unavailable()));
                            }
                            if !self.record_notification_id(message)? {
                                return Ok(Effects::default());
                            }
                            let catalog = skills::SkillCatalog::from_native(
                                &params["update"]["availableCommands"],
                            )
                            .map_err(RuntimeError::Protocol)?;
                            self.early_skill_catalogs.insert(id.to_owned(), catalog);
                        }
                    }
                }
            }
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
        if !self.record_notification_id(message)? {
            return Ok(Effects::default());
        }
        match method {
            "_x.ai/queue/changed" => {
                let effects = self.queue_changed(params)?;
                self.confirm_sdk_turn()?;
                Ok(effects)
            }
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
                    if self.options.permission_policy == PermissionPolicy::Inherit {
                        self.skill_catalog = Some(
                            skills::SkillCatalog::from_native(&update["availableCommands"])
                                .map_err(RuntimeError::Protocol)?,
                        );
                        if let Some((effective_permissions, began)) = self.deferred_ready.take() {
                            if began.elapsed() >= REQUEST_TIMEOUT {
                                return Err(RuntimeError::RequestTimedOut);
                            }
                            return Ok(Effects {
                                writes: Vec::new(),
                                events: vec![self.ready_event(effective_permissions)?],
                            });
                        }
                    }
                }
                self.update_metadata(update)?;
                let effects = self.session_update(params)?;
                self.confirm_sdk_turn()?;
                self.observe_sdk_native_tool(message)?;
                Ok(effects)
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

/// 持久化文本会合并并复用最后一个分片 ID，因此以完整回放和同回合终态核对，不重放增量分片。
fn verified_final_snapshot(
    result: &Value,
    session_id: &str,
    turn_id: &str,
    outcome: &TurnOutcome,
) -> Result<Option<(String, String)>, &'static str> {
    let updates = result["updates"]
        .as_array()
        .ok_or("native history has no updates")?;
    if updates.len() > MAX_NATIVE_IDENTITIES
        || result["hasMore"] != false
        || result["totalCount"].as_u64() != Some(updates.len() as u64)
    {
        return Err("native history is truncated or exceeds the verified limit");
    }
    let expected_reason = match outcome {
        TurnOutcome::Completed => "end_turn",
        TurnOutcome::Cancelled => "cancelled",
        TurnOutcome::Failed { .. } => return Err("invalid native history completion state"),
    };
    let mut output = String::new();
    let mut last_sequence = None;
    let mut last_event_id = None;
    let mut watermark = None;
    for record in updates {
        let params = &record["params"];
        if params["sessionId"].as_str() != Some(session_id) {
            return Err("native history changed the session id");
        }
        let event_id = params["_meta"]["eventId"]
            .as_str()
            .ok_or("native history has no event identity")?;
        let sequence = event_id
            .strip_prefix(session_id)
            .and_then(|id| id.strip_prefix('-'))
            .and_then(|suffix| {
                suffix
                    .parse::<u64>()
                    .ok()
                    .filter(|sequence| sequence.to_string() == suffix)
            })
            .ok_or("native history has an invalid event identity")?;
        if last_sequence.is_some_and(|previous| previous >= sequence) {
            return Err("native history event order is inconsistent");
        }
        last_sequence = Some(sequence);
        last_event_id = Some(event_id);
        let update = &params["update"];
        if record["method"] == "session/update"
            && update["sessionUpdate"] == "agent_message_chunk"
            && params["_meta"]["promptId"].as_str() == Some(turn_id)
        {
            if watermark.is_some() {
                return Err("native history contains text after its completion watermark");
            }
            let text = update["content"]["text"]
                .as_str()
                .filter(|_| update["content"]["type"] == "text")
                .ok_or("native history contains unsupported output")?;
            if output.len().saturating_add(text.len()) > MAX_LINE_BYTES {
                return Err("native history output exceeds the verified limit");
            }
            output.push_str(text);
        }
        if record["method"] == "_x.ai/session/update"
            && update["sessionUpdate"] == "turn_completed"
            && update["prompt_id"].as_str() == Some(turn_id)
        {
            if watermark.is_some() || update["stop_reason"].as_str() != Some(expected_reason) {
                return Err("native history completion does not match the RPC result");
            }
            watermark = Some(event_id.to_owned());
        }
    }
    if result["lastEventId"].as_str() != last_event_id {
        return Err("native history last event identity is inconsistent");
    }
    Ok(watermark.map(|watermark| (output, watermark)))
}

fn verified_latest_read_tool(tool: &Value) -> bool {
    let meta = &tool["_meta"]["x.ai/tool"];
    let Some(raw) = tool["rawInput"].as_object() else {
        return false;
    };
    tool["kind"] == "read"
        && meta["name"] == "read_file"
        && meta["namespace"] == "grok_build"
        && meta["version"].as_u64() == Some(1)
        && meta["read_only"] == true
        && raw.keys().all(|key| {
            matches!(
                key.as_str(),
                "variant" | "target_file" | "offset" | "limit" | "pages" | "format"
            )
        })
        && raw.get("variant").is_some_and(|value| value == "ReadFile")
        && raw
            .get("target_file")
            .and_then(Value::as_str)
            .is_some_and(|path| !path.is_empty())
        && ["offset", "limit"].into_iter().all(|key| {
            raw.get(key)
                .is_none_or(|value| value.is_null() || value.as_i64() == Some(1))
        })
        && ["pages", "format"]
            .into_iter()
            .all(|key| raw.get(key).is_none_or(Value::is_null))
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

fn current_models_update(message: &Value) -> Result<ReportedModels, RuntimeError> {
    let outer = message
        .as_object()
        .filter(|outer| {
            outer.len() == 3
                && outer.contains_key("jsonrpc")
                && outer.contains_key("method")
                && outer.contains_key("params")
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok models update envelope".into()))?;
    let params = outer["params"]
        .as_object()
        .filter(|params| {
            params.len() == 2
                && params.contains_key("currentModelId")
                && params.contains_key("availableModels")
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok models update fields".into()))?;
    let entries = params["availableModels"]
        .as_array()
        .filter(|entries| !entries.is_empty() && entries.len() <= 64)
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok models update entries".into()))?;
    for entry in entries {
        let entry = entry.as_object().filter(|entry| {
            entry.contains_key("modelId")
                && entry.contains_key("name")
                && entry
                    .keys()
                    .all(|key| matches!(key.as_str(), "modelId" | "name" | "description" | "_meta"))
        });
        let Some(entry) = entry else {
            return Err(RuntimeError::Protocol(
                "invalid Grok model display entry".into(),
            ));
        };
        if entry["modelId"]
            .as_str()
            .is_none_or(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
            || entry["name"].as_str().is_none_or(|name| {
                name.trim().is_empty() || name.len() > 512 || name.chars().any(char::is_control)
            })
            || entry.get("description").is_some_and(|description| {
                description
                    .as_str()
                    .is_none_or(|description| description.len() > 4 * 1024)
            })
            || entry.get("_meta").is_some_and(|meta| !meta.is_object())
            || serde_json::to_vec(entry).map_or(true, |value| value.len() > 16 * 1024)
        {
            return Err(RuntimeError::Protocol(
                "invalid Grok model display values".into(),
            ));
        }
    }
    let models: ReportedModels = decode_metadata(&outer["params"])?;
    let mut model_ids = HashSet::new();
    if models.current_model_id.is_empty()
        || models.current_model_id.len() > 128
        || models.current_model_id.chars().any(char::is_control)
        || models
            .available_models
            .iter()
            .any(|model| !model_ids.insert(model.model_id.as_str()))
        || !model_ids.contains(models.current_model_id.as_str())
    {
        return Err(RuntimeError::Protocol(
            "unexpected Grok 1.0.40 model display identity".into(),
        ));
    }
    Ok(models)
}

fn validate_current_settings_update(
    message: &Value,
    test_candidate_settings: bool,
) -> Result<(), RuntimeError> {
    const KEYS: [&str; 23] = [
        "allow_access",
        "announcements",
        "auto_permission_mode_enabled",
        "campaigns",
        "collapsed_edit_blocks",
        "consent_gate",
        "dock_enabled",
        "gate_label",
        "gate_message",
        "gate_url",
        "group_tool_verbs",
        "permission_mode",
        "privacy_banner_reshow_days",
        "privacy_notice_rollout",
        "prompt_suggestions_enabled",
        "session_picker_grouped",
        "sharing_enabled",
        "show_resolved_model",
        "slash_command_tags",
        "subscription_tier_display",
        "subscription_watch_interval_secs",
        "terminal_theme_enabled",
        "tips",
    ];
    let outer = message
        .as_object()
        .filter(|outer| {
            outer.len() == 3
                && outer.contains_key("jsonrpc")
                && outer.contains_key("method")
                && outer.contains_key("params")
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok settings update envelope".into()))?;
    let params = outer["params"]
        .as_object()
        .filter(|params| {
            params.len() == KEYS.len() + usize::from(test_candidate_settings)
                && KEYS.iter().all(|key| params.contains_key(*key))
                && (!test_candidate_settings
                    || params.contains_key("subagent_model_inheritance_enabled"))
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok settings update fields".into()))?;
    let bool_fields = [
        "allow_access",
        "privacy_notice_rollout",
        "sharing_enabled",
        "show_resolved_model",
    ];
    let array_fields = ["announcements", "campaigns", "tips"];
    let number_fields = [
        "privacy_banner_reshow_days",
        "subscription_watch_interval_secs",
    ];
    if bool_fields.iter().any(|key| !params[*key].is_boolean())
        || array_fields.iter().any(|key| !params[*key].is_array())
        || number_fields.iter().any(|key| !params[*key].is_number())
        || !params["slash_command_tags"].is_object()
        || !params["permission_mode"].is_null()
        || !params["auto_permission_mode_enabled"].is_null()
        || (test_candidate_settings && !params["subagent_model_inheritance_enabled"].is_boolean())
    {
        return Err(RuntimeError::Protocol(
            "invalid Grok settings update value types".into(),
        ));
    }
    Ok(())
}

fn validate_current_announcements_update(message: &Value) -> Result<u64, RuntimeError> {
    const ANNOUNCEMENT_KEYS: [&str; 9] = [
        "id",
        "message",
        "severity",
        "title",
        "cta",
        "updated_at",
        "expires_at",
        "dismissible",
        "persistent",
    ];
    let outer = message
        .as_object()
        .filter(|outer| {
            outer.len() == 3
                && outer.contains_key("jsonrpc")
                && outer.contains_key("method")
                && outer.contains_key("params")
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok announcements envelope".into()))?;
    let params = outer["params"]
        .as_object()
        .filter(|params| {
            params.len() == 2 && params.contains_key("gen") && params.contains_key("announcements")
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok announcements fields".into()))?;
    let announcements = params["announcements"]
        .as_array()
        .filter(|announcements| announcements.len() <= 32)
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok announcements list".into()))?;
    let generation = params["gen"]
        .as_u64()
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok announcement generation".into()))?;
    if announcements.iter().any(|announcement| {
        announcement.as_object().is_none_or(|announcement| {
            announcement.len() != ANNOUNCEMENT_KEYS.len()
                || !ANNOUNCEMENT_KEYS
                    .iter()
                    .all(|key| announcement.contains_key(*key))
                || !announcement["id"].is_string()
                || !announcement["message"].is_string()
                || !announcement["severity"].is_string()
                || !announcement["title"].is_string()
        })
    }) {
        return Err(RuntimeError::Protocol(
            "invalid Grok announcements values".into(),
        ));
    }
    Ok(generation)
}

fn validate_current_available_commands_update(
    message: &Value,
    session_id: &str,
    selected_skill: Option<&SelectedLocalSkill>,
) -> Result<(), RuntimeError> {
    const COMMAND_HAS_META: [bool; 29] = [
        false, false, false, false, false, true, false, false, false, true, true, true, true, true,
        true, true, true, true, true, true, true, true, true, true, true, true, true, true, true,
    ];
    const COMMAND_HAS_INPUT: [bool; 29] = [
        true, true, false, false, true, true, true, true, true, true, false, false, false, true,
        true, false, false, true, true, false, true, true, true, true, true, false, false, false,
        true,
    ];
    let outer = message
        .as_object()
        .filter(|outer| {
            outer.len() == 3
                && outer["jsonrpc"] == "2.0"
                && outer["method"] == "session/update"
                && outer.contains_key("params")
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok command catalog envelope".into()))?;
    let params = outer["params"]
        .as_object()
        .filter(|params| {
            params.len() == 3
                && params.contains_key("sessionId")
                && params.contains_key("update")
                && params.contains_key("_meta")
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok command catalog fields".into()))?;
    if params["sessionId"].as_str() != Some(session_id) {
        return Err(RuntimeError::Protocol(
            "Grok command catalog changed its session id".into(),
        ));
    }
    let expected_event_id = format!("{session_id}-2");
    let params_meta = params["_meta"]
        .as_object()
        .filter(|meta| {
            meta.len() == 5
                && meta.contains_key("agentTimestampMs")
                && meta.contains_key("eventId")
                && meta.contains_key("totalTokens")
                && meta.contains_key("updateParams")
                && meta.contains_key("updateType")
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok command catalog metadata".into()))?;
    if !params_meta["agentTimestampMs"].is_number() {
        return Err(RuntimeError::Protocol(
            "invalid Grok command catalog timestamp".into(),
        ));
    }
    if params_meta["eventId"].as_str() != Some(expected_event_id.as_str()) {
        return Err(RuntimeError::Protocol(
            "invalid Grok command catalog event id".into(),
        ));
    }
    if !params_meta["totalTokens"].is_number() {
        return Err(RuntimeError::Protocol(
            "invalid Grok command catalog token count".into(),
        ));
    }
    if !params_meta["updateParams"].is_object() {
        return Err(RuntimeError::Protocol(
            "invalid Grok command catalog update params".into(),
        ));
    }
    if params_meta["updateType"] != "AvailableCommandsUpdate" {
        return Err(RuntimeError::Protocol(
            "invalid Grok command catalog update type".into(),
        ));
    }
    if serde_json::to_vec(&params_meta["updateParams"])
        .map_or(true, |value| value.len() > 128 * 1024)
    {
        return Err(RuntimeError::Protocol(
            "Grok command catalog update params are too large".into(),
        ));
    }
    let update = params["update"]
        .as_object()
        .filter(|update| {
            update.len() == 3
                && update["sessionUpdate"] == "available_commands_update"
                && update.contains_key("availableCommands")
                && update.contains_key("_meta")
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok command catalog update".into()))?;
    let commands = update["availableCommands"]
        .as_array()
        .filter(|commands| {
            commands.len() == COMMAND_HAS_META.len() + usize::from(selected_skill.is_some())
        })
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok command catalog entries".into()))?;
    for (index, command) in commands.iter().enumerate() {
        let is_selected_skill = selected_skill.is_some() && index == 9;
        let base_index = index - usize::from(selected_skill.is_some() && index > 9);
        let has_meta = is_selected_skill || COMMAND_HAS_META[base_index];
        let has_input = !is_selected_skill && COMMAND_HAS_INPUT[base_index];
        let command = command.as_object().filter(|command| {
            command.len() == if has_meta { 4 } else { 3 }
                && command.contains_key("name")
                && command.contains_key("description")
                && command.contains_key("input")
                && command.contains_key("_meta") == has_meta
        });
        let Some(command) = command else {
            return Err(RuntimeError::Protocol(
                "invalid Grok command catalog entry shape".into(),
            ));
        };
        if command["name"]
            .as_str()
            .is_none_or(|name| name.is_empty() || name.len() > 128)
            || command["description"]
                .as_str()
                .is_none_or(|description| description.len() > 4 * 1024)
            || command["input"].is_object() != has_input
            || command["input"].is_null() == has_input
            || command.get("_meta").is_some_and(|meta| !meta.is_object())
            || serde_json::to_vec(command).map_or(true, |value| value.len() > 16 * 1024)
        {
            return Err(RuntimeError::Protocol(
                "invalid Grok command catalog entry values".into(),
            ));
        }
        if let Some(selected_skill) = selected_skill.filter(|_| is_selected_skill) {
            let selected_path = selected_skill.path.canonicalize().ok();
            let command_path = command["_meta"]["path"]
                .as_str()
                .map(Path::new)
                .filter(|path| path.is_absolute())
                .and_then(|path| path.canonicalize().ok());
            if command["_meta"]["bareName"].as_str() != Some(selected_skill.name.as_str())
                || command["_meta"]["scope"] != "local"
                || !selected_path.is_some_and(|selected_path| command_path == Some(selected_path))
            {
                return Err(RuntimeError::Protocol(
                    "Grok selected skill catalog path or identity changed".into(),
                ));
            }
        }
    }
    let update_meta = update["_meta"]
        .as_object()
        .filter(|meta| meta.len() == 1 && meta.contains_key("tools"))
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok command tool metadata".into()))?;
    let tools = update_meta["tools"]
        .as_array()
        .filter(|tools| tools.len() == 27)
        .ok_or_else(|| RuntimeError::Protocol("invalid Grok command tool count".into()))?;
    let mut names = HashSet::new();
    if tools.iter().any(|tool| {
        tool.as_str()
            .is_none_or(|tool| tool.is_empty() || tool.len() > 128 || !names.insert(tool))
    }) {
        return Err(RuntimeError::Protocol(
            "invalid Grok command tool values".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "grok_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "grok_live_tests.rs"]
mod live_tests;

#[cfg(test)]
#[path = "grok_sdk_origin_live_tests.rs"]
mod sdk_origin_live_tests;

#[cfg(test)]
#[path = "grok_policy_preflight_live_tests.rs"]
mod policy_preflight_live_tests;

#[cfg(all(test, unix))]
#[path = "grok_native_tool_lease_live_tests.rs"]
mod native_tool_lease_live_tests;

#[cfg(all(test, unix))]
#[path = "grok_native_skill_live_tests.rs"]
mod native_skill_live_tests;

#[cfg(all(test, unix))]
#[path = "grok_fixed_policy_live_tests.rs"]
mod fixed_policy_live_tests;

#[path = "grok_skills.rs"]
mod skills;

#[cfg(all(test, unix))]
#[path = "grok_files_policy_live_tests.rs"]
mod files_policy_live_tests;

#[cfg(all(test, unix))]
#[path = "grok_selected_skill_live_tests.rs"]
mod selected_skill_live_tests;

#[cfg(test)]
#[path = "grok_catalog_preflight_live_tests.rs"]
mod catalog_preflight_live_tests;
