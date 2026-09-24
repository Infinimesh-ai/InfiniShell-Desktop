//! Claude Code 精确版本的双向 stream-json；输入排队与同回合 steering 保持区别。

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(test)]
use base64::{Engine as _, engine::general_purpose::STANDARD};
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
use super::managed_input::restore_managed_images;
use super::permissions::verify_effective_permissions;
use super::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeConnection, RuntimeError, RuntimeEvent, RuntimeEventKind, SessionOptions, SessionTarget,
    TurnOutcome, channels, local_tools,
};
use crate::ai::agent::ImageContext;
use crate::util::image::MAX_IMAGE_COUNT_FOR_QUERY;

#[path = "claude_permission_snapshot.rs"]
mod permission_snapshot;
#[path = "claude_profile_preflight.rs"]
mod profile_preflight;

use permission_snapshot::{Observation, Rejection};

const PERMISSION_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(5);
// 固定新版在三个桌面平台使用相同的精确版本配对，未知版本仍拒绝。
const SUPPORTED_VERSIONS: &[(&str, &str)] = &[
    ("2.1.273 (Claude Code)", "2.1.273"),
    ("2.1.278 (Claude Code)", "2.1.278"),
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    ("2.1.280 (Claude Code)", "2.1.280"),
];
#[cfg(any(test, feature = "claude_21280_test_candidate"))]
const TEST_CANDIDATE_VERSION: &str = "2.1.280";
// 无凭据 init/EOF 只覆盖 P0；候选完整任务链仍须单独真实验收。
#[cfg(any(test, feature = "claude_21280_test_candidate"))]
const TEST_CANDIDATE_OUTPUT: &str = "2.1.280 (Claude Code)";
#[cfg(any(test, feature = "claude_21280_test_candidate"))]
const TEST_CANDIDATE_MARKER: &str = ".infinishell-claude-21280-candidate";

#[cfg(any(test, feature = "claude_21280_test_candidate"))]
fn test_candidate_state_root(state_dir: &Path) -> Option<&Path> {
    if state_dir.join(TEST_CANDIDATE_MARKER).is_file() {
        return Some(state_dir);
    }
    if state_dir.file_name() != Some(std::ffi::OsStr::new("native")) {
        return None;
    }
    let generation = state_dir.parent()?;
    Uuid::parse_str(generation.file_name()?.to_str()?).ok()?;
    let hosts = generation.parent()?;
    if hosts.file_name() != Some(std::ffi::OsStr::new("cli-agent-hosts")) {
        return None;
    }
    hosts.parent()
}

#[cfg(any(test, feature = "claude_21280_test_candidate"))]
fn test_candidate_enabled(options: &SessionOptions) -> bool {
    if !cfg!(debug_assertions) {
        return false;
    }
    let Some(root) = std::env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT") else {
        return false;
    };
    let root = Path::new(&root);
    let Ok(project) = root.join("project").canonicalize() else {
        return false;
    };
    if std::env::var("INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION").as_deref()
        != Ok(TEST_CANDIDATE_VERSION)
        || std::env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE").as_deref()
            != Some(options.executable.as_os_str())
        || !root.is_absolute()
        || options.cwd.canonicalize().ok().as_deref() != Some(project.as_path())
        || !options.selected_skills.is_empty()
        || std::fs::read(root.join(TEST_CANDIDATE_MARKER))
            .ok()
            .as_deref()
            != Some(b"isolated Claude Code 2.1.280 test candidate\n".as_slice())
    {
        return false;
    }
    if std::env::var("INFINISHELL_CLAUDE_LIVE_CANDIDATE_21280").as_deref() == Ok("1")
        && options.state_dir == root
        && options.permission_policy == PermissionPolicy::Inherit
        && options.claude_profile.is_none()
        && options.local_tools.is_none()
    {
        return true;
    }
    if std::env::var("INFINISHELL_CLAUDE_COORDINATOR_CANDIDATE_21280").as_deref() != Ok("1")
        || std::env::var("INFINISHELL_CLAUDE_LIVE_AUTH_MODE").as_deref()
            != Ok("authorized_default_account")
        || std::env::var_os("CLAUDE_CONFIG_DIR").is_some()
        || std::env::var_os("ANTHROPIC_API_KEY").is_some()
        || std::env::var_os("ANTHROPIC_AUTH_TOKEN").is_some()
        || options.permission_policy != PermissionPolicy::ClaudeRestrictedFilesV1
        || options.local_tools
            != Some(super::local_tools::LocalToolPermissions {
                allow_spawn: true,
                allow_message: true,
            })
        || std::fs::read_to_string(root.join(".infinishell-claude-coordinator-probe"))
            .ok()
            .as_deref()
            != Some("real_claude_production_coordinator")
    {
        return false;
    }
    let Some(profile) = std::env::var("WARP_DATA_PROFILE").ok() else {
        return false;
    };
    let Some(suffix) = profile.strip_prefix("claude-coordinator-") else {
        return false;
    };
    if suffix.len() != 32
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        || std::env::var("INFINISHELL_CLAUDE_LIVE_STATE_PROFILE")
            .ok()
            .as_deref()
            != Some(profile.as_str())
    {
        return false;
    }
    let Some(state_root) = test_candidate_state_root(&options.state_dir) else {
        return false;
    };
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    let (Ok(state_root), Ok(home)) = (state_root.canonicalize(), Path::new(&home).canonicalize())
    else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let Ok(metadata) = std::fs::metadata(&state_root) else {
            return false;
        };
        if metadata.permissions().mode() & 0o077 != 0 {
            return false;
        }
    }
    state_root.starts_with(home)
        && state_root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(&profile))
        && std::fs::read(state_root.join(TEST_CANDIDATE_MARKER))
            .ok()
            .as_deref()
            == Some(b"isolated Claude Code 2.1.280 coordinator candidate\n".as_slice())
}

pub(crate) fn supported_version(version: &str) -> bool {
    SUPPORTED_VERSIONS
        .iter()
        .any(|(_, supported)| *supported == version)
}

#[cfg(any(test, feature = "claude_21280_test_candidate"))]
pub(super) fn test_candidate_executable_digest() -> Option<&'static str> {
    profile_preflight::test_candidate_executable_digest(
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
// 图片回放会增加原生字段；保留 64 KiB 余量，入站仍按 8 MiB 严格封顶。
const MAX_IMAGE_MESSAGE_BYTES: usize = MAX_LINE_BYTES - 64 * 1024;
const MAX_MESSAGE_RECORDS: usize = 4096;
pub(crate) const NATIVE_RESULT_EVIDENCE_MARKER: &str = ".claude-native-result-evidence-v1";

fn native_result_evidence_enabled(state_dir: &Path) -> bool {
    if state_dir.join(NATIVE_RESULT_EVIDENCE_MARKER).is_file() {
        return true;
    }
    let Some(generation_dir) = state_dir.parent() else {
        return false;
    };
    let Some(hosts_dir) = generation_dir.parent() else {
        return false;
    };
    if state_dir.file_name().and_then(|name| name.to_str()) != Some("native")
        || hosts_dir.file_name().and_then(|name| name.to_str()) != Some("cli-agent-hosts")
        || generation_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|generation| Uuid::parse_str(generation).is_err())
    {
        return false;
    }
    hosts_dir
        .parent()
        .is_some_and(|root| root.join(NATIVE_RESULT_EVIDENCE_MARKER).is_file())
}

pub fn connect(options: SessionOptions) -> Result<RuntimeConnection, RuntimeError> {
    let attachment_store = options.state_dir.join("local-cli-attachments");
    connect_with_attachment_store(options, attachment_store)
}

pub(super) fn connect_with_attachment_store(
    options: SessionOptions,
    attachment_store: PathBuf,
) -> Result<RuntimeConnection, RuntimeError> {
    if !options.executable.is_absolute() || !options.cwd.is_absolute() {
        return Err(RuntimeError::InvalidConfiguration(
            "executable and cwd must be absolute".into(),
        ));
    }
    validate_options(&options)?;
    let (controller, commands, sender, events) = channels(options.generation);
    let task = Box::pin(async move {
        let mut protocol = ClaudeProtocol::new(options);
        protocol.attachment_store = attachment_store;
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
    #[cfg(any(test, feature = "claude_21280_test_candidate"))]
    {
        protocol.test_candidate_21280 = test_candidate_enabled(&protocol.options);
    }
    if protocol.options.permission_policy == PermissionPolicy::ClaudeRestrictedFilesV1 {
        let profile = profile_preflight::prepare(&protocol.options).await?;
        protocol.options.claude_profile = Some(profile);
    }
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
    protocol.bind_probed_version(output.status.success(), &detected)?;

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
    // 保留读端至 EOF，让监督者转发尚未读完的末帧；读取有总预算与超时。
    let drain = async move {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > MAX_LINE_BYTES {
            return Err(RuntimeError::Protocol(crate::t!(
                "cli-agent-runtime-data-too-large"
            )));
        }
        Ok::<(), RuntimeError>(())
    };
    let finish = async {
        if result.is_ok() {
            child.finish_after_stdin_close().await
        } else {
            child.finish().await
        }
    };
    let (finished, drained) = futures::join!(finish, drain.with_timeout(Duration::from_secs(30)));
    // 完成清理后保留原传输失败；监督或 EOF 失败仍阻止正常关闭被计为成功。
    result?;
    finished?;
    drained.map_err(|_| RuntimeError::RequestTimedOut)??;
    Ok(())
}

async fn run_transport(
    protocol: &mut ClaudeProtocol,
    stdin: &mut (impl AsyncWrite + Unpin),
    stdout: &mut (impl AsyncRead + Unpin),
    mut commands: mpsc::Receiver<RuntimeCommand>,
    events: &mpsc::Sender<RuntimeEvent>,
) -> Result<(), RuntimeError> {
    let initialize = protocol.initialize();
    #[cfg(test)]
    record_live_native_ids(protocol, &initialize, "stdin")?;
    write_message(stdin, &initialize).await?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if let Some(error) = protocol.profile_error.take() {
            return Err(error);
        }
        let effects = protocol.expire_permission_observation();
        if let Some(error) = protocol.profile_error.take() {
            return Err(error);
        }
        flush_effects(protocol, stdin, events, effects).await?;
        let effects = protocol.expire_cancellations();
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
                    #[cfg(test)]
                    trace_live_protocol_ids(&message, protocol.options.generation);
                    #[cfg(test)]
                    record_live_native_ids(protocol, &message, "stdout")?;
                    let effects = protocol.receive(message)?;
                    if let Some(error) = protocol.profile_error.take() {
                        return Err(error);
                    }
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

#[cfg(test)]
fn trace_live_protocol_ids(message: &Value, generation: Uuid) {
    if std::env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT").is_none() {
        return;
    }
    let mut identifiers = live_native_protocol_ids(message);
    if std::env::var_os("INFINISHELL_CLAUDE_MANAGED_IMAGE_TRACE").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        // 专用夹具只记录实际输出来源与内容摘要；旧验收字段保持原协议。
        identifiers["runtime_generation"] = json!(generation);
        identifiers["image_content_projection"] = live_native_image_projection(message);
        identifiers["result_text_sha256"] = json!(
            (message["type"] == "result")
                .then(|| message["result"].as_str())
                .flatten()
                .map(|text| format!("{:x}", Sha256::digest(text.as_bytes())))
        );
    }
    eprintln!("CLAUDE_NATIVE_PROTOCOL_IDS {identifiers}");
}

#[cfg(test)]
fn live_native_image_projection(message: &Value) -> Value {
    if message["type"] != "user" || message["message"]["role"] != "user" {
        return Value::Null;
    }
    let content = &message["message"]["content"];
    let Some(blocks) = content.as_array().filter(|blocks| blocks.len() == 2) else {
        return Value::Null;
    };
    if summarize_blocks(content).is_err() {
        return Value::Null;
    }
    let text = blocks[0]["text"].as_str().expect("已验证文本块");
    let data = blocks[1]["source"]["data"].as_str().expect("已验证图片块");
    let Ok(image) = STANDARD.decode(data) else {
        return Value::Null;
    };
    let encoded = serde_json::to_vec(content).expect("JSON 数组可编码");
    json!({"array_sha256":format!("{:x}", Sha256::digest(&encoded)),
        "text_sha256":format!("{:x}", Sha256::digest(text.as_bytes())), "text_bytes":text.len(),
        "image_sha256":format!("{:x}", Sha256::digest(&image)), "image_bytes":image.len(),
        "block_types":["text", "image"], "media_type":"image/png"})
}

#[cfg(test)]
fn live_native_protocol_ids(message: &Value) -> Value {
    let tools = message["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "tool_use")
        .map(|block| {
            json!({"id":live_native_id(&block["id"]),"name":live_native_variant(&block["name"], &[
                "Read", "Edit", "Write", "Bash", "Grep", "Glob", "LS", "Task", "Agent",
                "TaskCreate", "TaskGet", "TaskUpdate", "TaskList", "TodoWrite", "WebFetch",
                "WebSearch", "NotebookEdit", "AskUserQuestion", "EnterPlanMode", "ExitPlanMode",
                "mcp__infinishell-local-tasks__inspect_local_tasks",
                "mcp__infinishell-local-tasks__run_agents",
                "mcp__infinishell-local-tasks__send_message_to_agent",
            ])})
        })
        .collect::<Vec<_>>();
    json!({
        "type":live_native_variant(&message["type"], &[
            "assistant", "user", "result", "system", "control_request", "control_response",
            "command_lifecycle", "stream_event", "control_cancel_request", "keep_alive",
            "rate_limit_event", "tool_progress", "tool_use_summary", "auth_status",
        ]),
        "subtype":live_native_variant(&message["subtype"], &[
            "success", "error_during_execution", "error_max_turns", "error_max_budget_usd",
            "error_max_structured_output_retries", "init", "status", "hook_started",
            "hook_progress", "hook_response", "task_started", "task_progress", "task_notification",
            "compact_boundary", "elicitation_complete", "permission_denied", "permission_mode_changed",
        ]),
        "uuid":live_native_id(&message["uuid"]),"session_id":live_native_id(&message["session_id"]),
        "message_id":live_native_id(&message["message"]["id"]),
        "user_message_uuid":live_native_id(&message["user_message_uuid"]),
        "user_message_uuids":message["user_message_uuids"].as_array().map(|ids|
            ids.iter().map(live_native_id).collect::<Vec<_>>()),
        "command_uuid":live_native_id(&message["command_uuid"]),
        "state":live_native_variant(&message["state"], &["queued", "started", "completed", "cancelled"]),
        "request_id":live_native_id(&message["request_id"]),
        "request_subtype":live_native_variant(&message["request"]["subtype"], &[
            "initialize", "interrupt", "can_use_tool", "mcp_message", "get_settings",
            "list_permission_rules", "get_hooks", "list_hooks", "mcp_status", "set_permission_mode",
            "set_model", "set_max_thinking_tokens", "rewind_files", "reload_plugins",
        ]),
        "tool_use_id":live_native_id(&message["request"]["tool_use_id"]),"tools":tools,
        "terminal_reason":match message["terminal_reason"].as_str() {
            Some("aborted_streaming" | "aborted_tools" | "interrupted" | "cancelled" | "api_error"
                | "completed" | "end_turn") => message["terminal_reason"].clone(),
            Some(_) => json!("unknown"),
            None => Value::Null,
        },
        "is_error":message["is_error"].as_bool(),
        "response_request_id":live_native_id(&message["response"]["request_id"]),
        "response_subtype":match message["response"]["subtype"].as_str() {
            Some("success" | "error") => message["response"]["subtype"].clone(),
            Some(_) => json!("unknown"),
            None => Value::Null,
        },
    })
}

#[cfg(test)]
fn live_native_variant(value: &Value, variants: &[&str]) -> Value {
    match value.as_str() {
        Some(value) if variants.contains(&value) => json!(value),
        Some(_) => json!("unknown"),
        None => Value::Null,
    }
}

#[cfg(test)]
fn live_native_id(value: &Value) -> Value {
    let Some(id) = value.as_str() else {
        return Value::Null;
    };
    let sdk_id = (id.starts_with("msg_") || id.starts_with("toolu_"))
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    let host_id = id.strip_prefix("infinishell-").is_some_and(|tail| {
        tail.rsplit_once('-').is_some_and(|(generation, counter)| {
            Uuid::parse_str(generation).is_ok()
                && counter
                    .parse::<u64>()
                    .is_ok_and(|number| number.to_string() == counter)
        })
    });
    if Uuid::parse_str(id).is_ok() || sdk_id || host_id {
        json!(id)
    } else {
        // 非原生 ID 形态只保留散列，防止任意正文借诊断字段流出。
        json!(format!("sha256:{:x}", Sha256::digest(id.as_bytes())))
    }
}

#[cfg(test)]
fn live_native_value_shape(value: Option<&Value>) -> Value {
    let (kind, bytes) = match value {
        None => ("missing", None),
        Some(Value::Null) => ("null", Some(b"null".to_vec())),
        Some(Value::Bool(value)) => ("boolean", Some(value.to_string().into_bytes())),
        Some(Value::Number(value)) => ("number", Some(value.to_string().into_bytes())),
        Some(Value::String(value)) => ("string", Some(value.as_bytes().to_vec())),
        Some(value @ Value::Array(..)) => ("array", Some(value.to_string().into_bytes())),
        Some(value @ Value::Object(..)) => ("object", Some(value.to_string().into_bytes())),
    };
    json!({"type":kind,"bytes":bytes.as_ref().map_or(0, Vec::len),
        "sha256":bytes.map(|bytes| format!("{:x}", Sha256::digest(bytes)))})
}

#[cfg(test)]
fn live_native_cancel_diagnostics(message: &Value) -> Value {
    let mut errors = live_native_value_shape(message.get("errors"));
    errors["count"] = json!(message["errors"].as_array().map_or(0, Vec::len));
    // 取消诊断只保留形状和散列；未知原因及错误正文不会进入公共账本。
    json!({"terminal_reason":live_native_value_shape(message.get("terminal_reason")),
        "errors":errors})
}

#[cfg(test)]
fn record_live_native_ids(
    protocol: &ClaudeProtocol,
    message: &Value,
    direction: &str,
) -> Result<(), RuntimeError> {
    if let Some(records) = &protocol.native_ids_for_live {
        let mut records = records
            .lock()
            .map_err(|_| RuntimeError::Protocol("live native ID ledger lock failed".into()))?;
        if records.len() >= MAX_MESSAGE_RECORDS * 8 {
            return Err(RuntimeError::Protocol(
                "live native ID ledger limit reached".into(),
            ));
        }
        let mut ids = live_native_protocol_ids(message);
        ids["direction"] = json!(direction);
        if message["type"] == "result" {
            ids["cancel_diagnostics"] = live_native_cancel_diagnostics(message);
        }
        records.push(ids);
    }
    Ok(())
}

fn encode_message(message: &Value) -> Result<Vec<u8>, RuntimeError> {
    let mut encoded =
        serde_json::to_vec(message).map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    encoded.push(b'\n');
    let limit = if message["type"] == "user" && message["message"]["content"].is_array() {
        MAX_IMAGE_MESSAGE_BYTES
    } else {
        MAX_LINE_BYTES
    };
    if encoded.len() > limit {
        return Err(RuntimeError::Protocol(crate::t!(
            "cli-agent-runtime-data-too-large"
        )));
    }
    Ok(encoded)
}

fn user_message(content: Value, id: Uuid, session_id: &str) -> Value {
    json!({"type":"user", "message":{"role":"user", "content":content},
        "parent_tool_use_id":null, "session_id":session_id, "uuid":id.to_string()})
}

/// 已逐张验证的 base64 无 JSON 转义；在保存附件和创建任务前核对整批原生帧预算。
pub(super) fn verify_prepared_png_budget(
    text: &str,
    images: &[ImageContext],
) -> Result<(), String> {
    if text.len() > MAX_INPUT_BYTES {
        return Err(crate::t!("cli-agent-input-text-too-large"));
    }
    let payload_bytes = images.iter().fold(0usize, |bytes, image| {
        bytes.saturating_add(image.data.len())
    });
    if payload_bytes > MAX_IMAGE_MESSAGE_BYTES {
        return Err(crate::t!("editor-image-too-large"));
    }
    let mut blocks = vec![json!({"type":"text", "text":text})];
    for _ in 0..images.len() {
        blocks.push(json!({"type":"image", "source":{"type":"base64", "media_type":"image/png", "data":""}}));
    }
    // 固定已验证版本的原生会话和消息 ID 均为 UUID；预算包含完整信封与换行。
    let placeholder = user_message(json!(blocks), Uuid::nil(), &Uuid::nil().to_string());
    let envelope_bytes = encode_message(&placeholder)
        .map_err(|_| crate::t!("editor-image-too-large"))?
        .len();
    if envelope_bytes.saturating_add(payload_bytes) > MAX_IMAGE_MESSAGE_BYTES {
        return Err(crate::t!("editor-image-too-large"));
    }
    Ok(())
}

async fn write_message(
    stdin: &mut (impl AsyncWrite + Unpin),
    message: &Value,
) -> Result<(), RuntimeError> {
    let encoded = encode_message(message)?;
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
        #[cfg(test)]
        record_live_native_ids(protocol, &message, "stdin")?;
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
    if options.grok_profile.is_some()
        || !matches!(
            options.permission_policy,
            PermissionPolicy::Inherit | PermissionPolicy::ClaudeRestrictedFilesV1
        )
    {
        return Err(RuntimeError::InvalidConfiguration(
            "Claude permission modes are not equivalent to the requested filesystem sandbox".into(),
        ));
    }
    if options.permission_policy == PermissionPolicy::ClaudeRestrictedFilesV1 {
        if !options.selected_skills.is_empty() {
            return Err(super::claude_profile::reject(
                "claude_profile_skills_unsupported",
            ));
        }
        if let Some(profile) = &options.claude_profile {
            profile.validate(&options.cwd)?;
        }
        if let Some(ceiling) = &options.permission_ceiling {
            let expected = ceiling
                .claude_profile()
                .ok_or_else(|| super::claude_profile::reject("claude_profile_wrong_parent"))?;
            if options.claude_profile.as_ref() != Some(expected) {
                return Err(super::claude_profile::reject(
                    "claude_profile_parent_mismatch",
                ));
            }
        }
    } else if options.claude_profile.is_some() {
        return Err(super::claude_profile::reject("claude_profile_wrong_policy"));
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
    if let Some(profile) = &options.claude_profile {
        arguments.extend(profile.arguments());
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
    Hooks,
    Mcp,
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
    expected_replay: ExpectedReplay,
    sent_at: Instant,
    accepted: bool,
    started: bool,
    finished: bool,
    joined_to: Option<Uuid>,
    output: String,
    error: Option<String>,
    cancellation: Option<PendingCancellation>,
}

struct PendingCancellation {
    started_at: Instant,
    interrupt_requested: bool,
    interrupt_acknowledged: bool,
    native_cancelled: bool,
    aborted_result: Option<String>,
    aborted_output: Option<String>,
    aborted_inputs: Vec<Uuid>,
}

impl PendingCancellation {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            interrupt_requested: false,
            interrupt_acknowledged: false,
            native_cancelled: false,
            aborted_result: None,
            aborted_output: None,
            aborted_inputs: Vec::new(),
        }
    }
}

struct PendingApproval {
    fingerprint: [u8; 32],
    turn_id: Uuid,
    input: Value,
    tool_name: String,
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
    attachment_store: PathBuf,
    probed_version: Option<&'static str>,
    paired_version: Option<&'static str>,
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
    profile_settings: Value,
    profile_rules: Value,
    profile_hooks: Value,
    profile_mcp: Value,
    profile_mcp_checks: u8,
    initialize_info: Value,
    profile_error: Option<RuntimeError>,
    profile_tool_calls: HashMap<String, (Uuid, String, [u8; 32])>,
    profile_assistant_origin: Option<Uuid>,
    profile_assistant_messages: HashMap<String, Uuid>,
    profile_pending_command: Option<(Uuid, RuntimeAction)>,
    profile_command_authorized: bool,
    native_result_evidence: bool,
    #[cfg(any(test, feature = "claude_21280_test_candidate"))]
    test_candidate_21280: bool,
    #[cfg(test)]
    native_ids_for_live: Option<Arc<Mutex<Vec<Value>>>>,
}

impl ClaudeProtocol {
    fn new(options: SessionOptions) -> Self {
        let native_result_evidence = native_result_evidence_enabled(&options.state_dir);
        Self {
            attachment_store: options.state_dir.join("local-cli-attachments"),
            options,
            // 旧离线协议夹具不派生进程；真实运行始终由 run_process 的本次探测重新绑定。
            #[cfg(test)]
            probed_version: Some("2.1.273"),
            #[cfg(not(test))]
            probed_version: None,
            paired_version: None,
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
            profile_settings: Value::Null,
            profile_rules: Value::Null,
            profile_hooks: Value::Null,
            profile_mcp: Value::Null,
            profile_mcp_checks: 0,
            initialize_info: Value::Null,
            profile_error: None,
            profile_tool_calls: HashMap::new(),
            profile_assistant_origin: None,
            profile_assistant_messages: HashMap::new(),
            profile_pending_command: None,
            profile_command_authorized: false,
            native_result_evidence,
            #[cfg(any(test, feature = "claude_21280_test_candidate"))]
            test_candidate_21280: false,
            #[cfg(test)]
            native_ids_for_live: None,
        }
    }

    fn bind_probed_version(&mut self, succeeded: bool, detected: &str) -> Result<(), RuntimeError> {
        self.probed_version = None;
        self.paired_version = None;
        #[cfg(any(test, feature = "claude_21280_test_candidate"))]
        if succeeded && self.test_candidate_21280 && detected == TEST_CANDIDATE_OUTPUT {
            self.probed_version = Some(TEST_CANDIDATE_VERSION);
            return Ok(());
        }
        if succeeded
            && let Some((_, version)) = SUPPORTED_VERSIONS
                .iter()
                .find(|(output, _)| *output == detected)
        {
            self.probed_version = Some(*version);
            return Ok(());
        }
        Err(RuntimeError::UnsupportedVersion(detected.to_owned()))
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
        self.profile_mcp_checks = 0;
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
        if self.options.claude_profile.is_some() && rejection.is_some() {
            self.profile_error = Some(super::claude_profile::reject(
                "claude_profile_observation_failed",
            ));
            self.permission_observation_started = None;
            return Effects::default();
        }
        if let Some(rejection) = rejection
            && let Some(observation) = &mut self.permission_observation
        {
            observation.reject(rejection);
        }
        self.permission_observation_started = None;
        self.pending
            .retain(|_, request| !matches!(request.kind, PendingKind::PermissionObservation(_)));
        if let Some(profile) = &self.options.claude_profile {
            if let Err(error) = profile.verify_live(
                &self.profile_settings,
                &self.profile_rules,
                &self.profile_hooks,
                &self.profile_mcp,
            ) {
                self.profile_error = Some(error);
                return Effects::default();
            }
            let mut effective = self.ready_permissions.clone();
            effective["claudeRestrictedFilesV1"] = json!(profile);
            effective["fixedProfileVerified"] = json!(true);
            if let Err(error) = verify_effective_permissions(
                self.options.permission_ceiling.as_ref(),
                "claude",
                &self.options.cwd,
                &effective,
            ) {
                self.profile_error = Some(error);
                return Effects::default();
            }
        }
        self.initialized = true;
        if let Some((message_id, action)) = self.profile_pending_command.take() {
            self.profile_command_authorized = true;
            let effects = self.apply_command(message_id, action);
            self.profile_command_authorized = false;
            return effects;
        }
        Effects {
            writes: Vec::new(),
            events: vec![self.ready_event()],
        }
    }

    fn ready_event(&self) -> RuntimeEventKind {
        let mut effective_permissions = self.ready_permissions.clone();
        if let Some(profile) = &self.options.claude_profile {
            effective_permissions["claudeRestrictedFilesV1"] = json!(profile);
            effective_permissions["fixedProfileVerified"] = json!(self.profile_error.is_none());
            effective_permissions["fixedProfileSha256"] = json!(profile.digest());
            #[cfg(any(test, feature = "claude_21280_test_candidate"))]
            if self.test_candidate_21280
                && self.paired_version == Some(TEST_CANDIDATE_VERSION)
                && self.profile_error.is_none()
                && let Some(native_session_id) = &self.session_id
            {
                effective_permissions["claudeTestCandidate21280Proof"] = json!({
                    "runtimeGeneration":self.options.generation,
                    "nativeSessionId":native_session_id,
                    "profileSha256":profile.digest(),
                });
            }
        }
        if let Some(observation) = &self.permission_observation {
            effective_permissions["permissionObservation"] = observation.value();
        }
        RuntimeEventKind::SessionReady {
            verified_cli_version: self.paired_version.map(str::to_owned),
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

    fn expire_cancellations(&mut self) -> Effects {
        let expired =
            self.turns
                .iter()
                .filter_map(|(id, turn)| {
                    let cancellation = turn.cancellation.as_ref()?;
                    (!turn.finished && cancellation.started_at.elapsed() >= REQUEST_TIMEOUT).then(
                        || {
                            (
                        *id,
                        cancellation.aborted_result.clone().unwrap_or_else(|| {
                            "Claude did not confirm cancellation; completion is uncertain".into()
                        }),
                    )
                        },
                    )
                })
                .collect::<Vec<_>>();
        let mut effects = Effects::default();
        for (turn_id, message) in expired {
            self.finish_execution_batch(
                turn_id,
                TurnOutcome::Failed { message },
                None,
                &mut effects.events,
            );
        }
        effects
    }

    fn finish_confirmed_cancellation(&mut self, turn_id: Uuid, events: &mut Vec<RuntimeEventKind>) {
        let confirmed = self
            .turns
            .get(&turn_id)
            .and_then(|turn| turn.cancellation.as_ref())
            .is_some_and(|cancellation| {
                cancellation.interrupt_requested
                    && cancellation.interrupt_acknowledged
                    && cancellation.native_cancelled
                    && cancellation.aborted_result.is_some()
                    && self.turns.iter().all(|(id, turn)| {
                        turn.finished
                            || (*id != turn_id && turn.joined_to != Some(turn_id))
                            || (turn.error.is_none() && cancellation.aborted_inputs.contains(id))
                    })
            });
        if confirmed {
            let output = self
                .turns
                .get(&turn_id)
                .and_then(|turn| turn.cancellation.as_ref())
                .and_then(|cancellation| cancellation.aborted_output.clone());
            self.finish_execution_batch(turn_id, TurnOutcome::Cancelled, output.as_deref(), events);
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
        if !self.initialized || self.profile_error.is_some() {
            return failed(message_id, "session initialization has not completed");
        }
        if self.options.claude_profile.is_some()
            && !self.profile_command_authorized
            && matches!(
                action,
                RuntimeAction::Submit { .. } | RuntimeAction::RespondApproval { .. }
            )
        {
            if self.profile_pending_command.is_some() {
                return failed(message_id, "fixed profile verification is already pending");
            }
            self.profile_pending_command = Some((message_id, action));
            return self.start_permission_observation(&self.initialize_info.clone());
        }
        match action {
            RuntimeAction::Submit { input } => {
                let projection =
                    match encode_input(input, self.skill_plugin.as_ref(), &self.attachment_store) {
                        Ok(projection) => projection,
                        Err(error) => return failed(message_id, &error),
                    };
                let (content, expected_replay) = projection.into_parts();
                let session_id =
                    self.session_id
                        .as_deref()
                        .unwrap_or_else(|| match &self.options.target {
                            SessionTarget::New => "",
                            SessionTarget::Resume { native_session_id } => native_session_id,
                        });
                let message = user_message(content, message_id, session_id);
                if encode_message(&message).is_err() {
                    return failed(message_id, &crate::t!("editor-image-too-large"));
                }
                self.turns.insert(
                    message_id,
                    UserTurn {
                        expected_replay,
                        sent_at: Instant::now(),
                        accepted: false,
                        started: false,
                        finished: false,
                        joined_to: None,
                        output: String::new(),
                        error: None,
                        cancellation: None,
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
                self.clear_assistant_origin(turn_id);
                self.turns
                    .get_mut(&turn_id)
                    .expect("running turn exists")
                    .cancellation
                    .get_or_insert_with(PendingCancellation::new)
                    .interrupt_requested = true;
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
                    || self.turns[&approval.turn_id]
                        .cancellation
                        .as_ref()
                        .is_some_and(|cancellation| cancellation.interrupt_requested)
                {
                    return failed(
                        message_id,
                        "approval is no longer pending for a running command",
                    );
                }
                let turn_id = approval.turn_id;
                let decision = if decision == ApprovalDecision::AllowOnce
                    && self.options.claude_profile.as_ref().is_some_and(|profile| {
                        !profile.approval_allowed(&approval.tool_name, &approval.input)
                    }) {
                    ApprovalDecision::DenyOnce
                } else {
                    decision
                };
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
                        let previous = self
                            .active_turn
                            .filter(|id| *id != turn_id && self.turn_is_running(*id));
                        let turn = self.turns.get_mut(&turn_id).expect("turn exists");
                        if !turn.started {
                            turn.started = true;
                            if let Some(previous) = previous {
                                // 固定版本会在工具轮次间合并已接收输入；保留真实执行，不虚构前一轮完成。
                                turn.joined_to = Some(previous);
                                effects.events.push(RuntimeEventKind::InputJoined {
                                    message_id: turn_id,
                                    turn_id: previous.to_string(),
                                });
                            } else {
                                // 独立命令必须等自己的原生 assistant 标记。
                                self.profile_assistant_origin = None;
                                self.active_turn = Some(turn_id);
                                effects.events.push(RuntimeEventKind::TurnStarted {
                                    turn_id: turn_id.to_string(),
                                });
                            }
                        }
                    }
                    "cancelled" => {
                        if self.turns[&turn_id].joined_to.is_some() {
                            // 单条合并输入的生命周期不能确认共享执行已取消，也不单独计时失败。
                            return Ok(effects);
                        }
                        self.clear_assistant_origin(turn_id);
                        // 认证失败也会发 cancelled；必须等待同回合的原生取消结果和确认。
                        self.turns
                            .get_mut(&turn_id)
                            .expect("turn exists")
                            .cancellation
                            .get_or_insert_with(PendingCancellation::new)
                            .native_cancelled = true;
                        self.finish_confirmed_cancellation(turn_id, &mut effects.events);
                    }
                    // 生命周期结束仅关闭连续输出归属；成功仍需原生 result。
                    "completed" => self.clear_assistant_origin(turn_id),
                    // 未知生命周期不升级成功，也不继续信任省略标记的输出。
                    _ => self.clear_assistant_origin(turn_id),
                }
            }
            "user" => {
                if message.get("isReplay").and_then(Value::as_bool) == Some(true) {
                    let turn_id = parse_uuid(&message, "uuid")?;
                    if let Some(turn) = self.turns.get(&turn_id) {
                        let content = &message["message"]["content"];
                        if matches!(turn.expected_replay, ExpectedReplay::Blocks(_))
                            && (message["message"]["role"] != "user"
                                || message
                                    .get("parent_tool_use_id")
                                    .is_some_and(|id| !id.is_null())
                                || message["session_id"]
                                    .as_str()
                                    .is_none_or(|id| id.is_empty())
                                || message["session_id"].as_str() != self.session_id.as_deref())
                        {
                            return Err(RuntimeError::Protocol(crate::t!(
                                "cli-agent-claude-image-replay-invalid"
                            )));
                        }
                        if !turn.expected_replay.matches(content) {
                            let message = match &turn.expected_replay {
                                ExpectedReplay::Blocks(_) => {
                                    crate::t!("cli-agent-claude-image-replay-invalid")
                                }
                                ExpectedReplay::Text(_) => {
                                    "replayed input differs from the submitted command".into()
                                }
                            };
                            return Err(RuntimeError::Protocol(message));
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
                let origin = if self.options.claude_profile.is_some() {
                    self.profile_assistant_turn(&message)?
                } else {
                    correlated_turn(&message).map(|id| self.execution_turn(id))
                };
                let Some(turn_id) = origin.filter(|id| self.turns.contains_key(id)) else {
                    return Ok(effects);
                };
                let turn = self.turns.get_mut(&turn_id).expect("turn exists");
                if turn.finished {
                    return Ok(effects);
                }
                if self.options.claude_profile.is_some() {
                    for tool in message["message"]["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|block| block["type"] == "tool_use")
                    {
                        let id = required_string(tool, "id")?.to_owned();
                        let name = required_string(tool, "name")?.to_owned();
                        let call = (turn_id, name, fingerprint(&tool["input"]));
                        if self
                            .profile_tool_calls
                            .get(&id)
                            .is_some_and(|previous| previous != &call)
                        {
                            return Err(super::claude_profile::reject(
                                "claude_profile_tool_identity_reused",
                            ));
                        }
                        if self.profile_tool_calls.len() >= MAX_MESSAGE_RECORDS {
                            return Err(super::claude_profile::reject(
                                "claude_profile_tool_history_limit",
                            ));
                        }
                        self.profile_tool_calls.insert(id, call);
                    }
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
                if let Some(message) = turn.error.clone()
                    && turn
                        .cancellation
                        .as_ref()
                        .is_some_and(|cancellation| cancellation.aborted_result.is_some())
                {
                    self.finish_execution_batch(
                        turn_id,
                        TurnOutcome::Failed { message },
                        None,
                        &mut effects.events,
                    );
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
                let mut turn_ids = correlated_turns(&message);
                if turn_ids.is_empty() {
                    return Err(RuntimeError::Protocol(
                        "result is missing a command UUID; completion is uncertain".into(),
                    ));
                }
                // 一份原生结果必须明确覆盖同批输入；仅最后一个 UUID 不能证明整批完成。
                for id in &turn_ids {
                    let execution = self.execution_turn(*id);
                    if !turn_ids.contains(&execution)
                        || self.turns.iter().any(|(joined_id, turn)| {
                            !turn.finished
                                && turn.joined_to == Some(execution)
                                && !turn_ids.contains(joined_id)
                        })
                    {
                        return Err(RuntimeError::Protocol(
                            "Claude result does not identify the complete joined input batch"
                                .into(),
                        ));
                    }
                }
                if self.native_result_evidence {
                    effects.events.push(RuntimeEventKind::Progress {
                        turn_id: self.execution_turn(turn_ids[0]).to_string(),
                        message: json!({
                            "kind":"native_result_correlated_v1",
                            "result_id":required_string(&message, "uuid")?,
                            "subtype":required_string(&message, "subtype")?,
                            "primary_input_id":correlated_turn(&message),
                            "input_ids":turn_ids,
                        })
                        .to_string(),
                    });
                }
                // 先保存每条合并输入的原生结果，再关闭真实活跃执行。
                turn_ids.sort_by_key(|id| {
                    self.turns
                        .get(id)
                        .is_none_or(|turn| turn.joined_to.is_none())
                });
                let mut deferred_executions = Vec::new();
                if matches!(&outcome, TurnOutcome::Cancelled)
                    || (matches!(
                        message["terminal_reason"].as_str(),
                        Some("aborted_streaming" | "aborted_tools")
                    ) && message["subtype"] == "error_during_execution"
                        && message["is_error"] == true)
                {
                    for id in &turn_ids {
                        let execution = self.execution_turn(*id);
                        if !deferred_executions.contains(&execution)
                            && self.turns.get(&execution).is_some_and(|turn| {
                                !turn.finished
                                    && turn.error.is_none()
                                    && turn.cancellation.as_ref().is_some_and(|cancellation| {
                                        cancellation.interrupt_requested
                                    })
                            })
                            && self.turns.iter().all(|(id, turn)| {
                                turn.finished
                                    || (*id != execution && turn.joined_to != Some(execution))
                                    || turn.error.is_none()
                            })
                        {
                            let cancellation = self
                                .turns
                                .get_mut(&execution)
                                .and_then(|turn| turn.cancellation.as_mut())
                                .expect("取消请求已经核对");
                            // 完整批次的中止结果共同等待真实执行的 ACK 和取消生命周期。
                            cancellation.aborted_result = Some(error_text(&message));
                            cancellation.aborted_output = message
                                .get("result")
                                .and_then(Value::as_str)
                                .map(str::to_owned);
                            cancellation.aborted_inputs = turn_ids.clone();
                            deferred_executions.push(execution);
                        }
                    }
                }
                for turn_id in turn_ids {
                    self.clear_assistant_origin(turn_id);
                    if deferred_executions.contains(&self.execution_turn(turn_id)) {
                        continue;
                    }
                    self.finish_turn(
                        turn_id,
                        match &outcome {
                            // 所有取消形态都需要已授权请求、ACK、执行生命周期和全批结果。
                            TurnOutcome::Cancelled => TurnOutcome::Failed {
                                message:
                                    "Claude cancellation has no authorized interrupt confirmation"
                                        .into(),
                            },
                            TurnOutcome::Completed | TurnOutcome::Failed { .. } => outcome.clone(),
                        },
                        message.get("result").and_then(Value::as_str),
                        &mut effects.events,
                    );
                }
                for execution in deferred_executions {
                    self.finish_confirmed_cancellation(execution, &mut effects.events);
                }
            }
            "system" if message["subtype"] == "init" => {
                self.paired_version = None;
                if let Some(profile) = &self.options.claude_profile {
                    profile.verify_system_init(&message)?;
                }
                // 两个已知版本也不能混配，避免探测后入口被替换却沿用旧能力判断。
                if self.probed_version.is_none()
                    || message.get("claude_code_version").and_then(Value::as_str)
                        != self.probed_version
                {
                    return Err(RuntimeError::UnsupportedVersion(
                        message["claude_code_version"].to_string(),
                    ));
                }
                if self.session_id.is_none() {
                    return Err(RuntimeError::Protocol(
                        "Claude init has no native session id".into(),
                    ));
                }
                self.paired_version = self.probed_version;
                let effective_permissions = json!({
                    "permissionMode":message["permissionMode"], "capabilities":message["capabilities"],
                });
                if self.options.claude_profile.is_none() {
                    verify_effective_permissions(
                        self.options.permission_ceiling.as_ref(),
                        "claude",
                        &self.options.cwd,
                        &effective_permissions,
                    )?;
                }
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
                if self.options.claude_profile.is_some()
                    && message["permissionMode"]
                        != super::claude_profile::FIXED_PROTOCOL_PERMISSION_MODE
                {
                    return Err(super::claude_profile::reject("claude_profile_mode_changed"));
                }
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
                if self.probed_version.is_none() {
                    return Err(RuntimeError::Protocol(
                        "Claude initialization has no successful CLI version probe".into(),
                    ));
                }
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
                if self.options.claude_profile.is_none() {
                    verify_effective_permissions(
                        self.options.permission_ceiling.as_ref(),
                        "claude",
                        &self.options.cwd,
                        &effective_permissions,
                    )?;
                }
                self.initialize_info = response["response"].clone();
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
                        self.profile_settings = response["response"].clone();
                        observation.settings(&response["response"]);
                        effects.writes.push(self.request(
                            PendingKind::PermissionObservation(PermissionStage::Rules),
                            json!({"subtype":"list_permission_rules"}),
                        ));
                    }
                    PermissionStage::Rules => {
                        self.profile_rules = response["response"].clone();
                        observation.rules(&response["response"]);
                        let (stage, subtype) = if self.options.claude_profile.is_some() {
                            (PermissionStage::Hooks, "get_hooks_listing")
                        } else {
                            (PermissionStage::RecheckMode, "initialize")
                        };
                        effects.writes.push(self.request(
                            PendingKind::PermissionObservation(stage),
                            json!({"subtype":subtype}),
                        ));
                    }
                    PermissionStage::Hooks => {
                        self.profile_hooks = response["response"].clone();
                        effects.writes.push(self.request(
                            PendingKind::PermissionObservation(PermissionStage::Mcp),
                            json!({"subtype":"mcp_status"}),
                        ));
                    }
                    PermissionStage::Mcp => {
                        self.profile_mcp = response["response"].clone();
                        self.profile_mcp_checks += 1;
                        // SDK 工具注册可晚于首次查询；仅为空的短暂状态允许有界重查。
                        if self.options.local_tools.is_some()
                            && self.profile_mcp["mcpServers"] == json!([])
                            && self.profile_mcp_checks < 4
                        {
                            effects.writes.push(self.request(
                                PendingKind::PermissionObservation(PermissionStage::Mcp),
                                json!({"subtype":"mcp_status"}),
                            ));
                            return Ok(effects);
                        }
                        effects.writes.push(self.request(
                            PendingKind::PermissionObservation(PermissionStage::RecheckMode),
                            json!({"subtype":"initialize"}),
                        ));
                    }
                    PermissionStage::RecheckMode => {
                        if self.options.claude_profile.is_some()
                            && (response["response"]["pid"] != self.initialize_info["pid"]
                                || !response["response"]["pid"]
                                    .as_u64()
                                    .is_some_and(|pid| pid > 0)
                                || response["response"]["current_permission_mode"]
                                    != super::claude_profile::FIXED_PROTOCOL_PERMISSION_MODE)
                        {
                            return Err(super::claude_profile::reject(
                                "claude_profile_native_identity_changed",
                            ));
                        }
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
                let mut rejected_abort = None;
                if let Some(turn) = self.turns.get_mut(&turn_id)
                    && !turn.finished
                    && let Some(cancellation) = &mut turn.cancellation
                {
                    if success {
                        cancellation.interrupt_acknowledged = true;
                    } else if !cancellation.interrupt_acknowledged {
                        cancellation.interrupt_requested = false;
                        rejected_abort = cancellation.aborted_result.clone();
                    }
                }
                let effect = if success {
                    accepted(message_id, Some(turn_id.to_string()))
                } else {
                    failed(message_id, &error_text(response))
                };
                effects.events.extend(effect.events);
                if let Some(message) = rejected_abort {
                    self.finish_execution_batch(
                        turn_id,
                        TurnOutcome::Failed { message },
                        None,
                        &mut effects.events,
                    );
                } else {
                    self.finish_confirmed_cancellation(turn_id, &mut effects.events);
                }
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

    fn clear_assistant_origin(&mut self, turn_id: Uuid) {
        if self.profile_assistant_origin == Some(turn_id) {
            self.profile_assistant_origin = None;
        }
    }

    fn execution_turn(&self, id: Uuid) -> Uuid {
        self.turns
            .get(&id)
            .and_then(|turn| turn.joined_to)
            .unwrap_or(id)
    }

    fn profile_assistant_turn(&mut self, message: &Value) -> Result<Option<Uuid>, RuntimeError> {
        // 固定 2.1.273 仅给每轮首个主 assistant 添加 user_message_uuid。
        // 同一有序 stdout 的后续消息沿用该显式归属；生命周期边界清空它，
        // active_turn 只校验归属，绝不为缺少原生标记的消息建立归属。
        if message.get("session_id").and_then(Value::as_str) != self.session_id.as_deref()
            || self.session_id.is_none()
        {
            return Err(super::claude_profile::reject(
                "claude_profile_assistant_session_missing",
            ));
        }
        required_string(message, "uuid")?;
        let message_id = required_string(&message["message"], "id")?;
        let explicit = correlated_turn(message).map(|id| self.execution_turn(id));
        if message.get("user_message_uuid").is_some() && explicit.is_none() {
            return Err(super::claude_profile::reject(
                "claude_profile_assistant_origin_invalid",
            ));
        }
        let Some(turn_id) = explicit.or(self.profile_assistant_origin) else {
            return Ok(None);
        };
        if let Some(previous) = self.profile_assistant_messages.get(message_id) {
            if *previous != turn_id {
                if explicit.is_some() {
                    return Err(super::claude_profile::reject(
                        "claude_profile_assistant_identity_reused",
                    ));
                }
                // 旧 API 消息换一个事件 UUID 重放，也不能迁入当前回合。
                return Ok(None);
            }
        } else {
            if self.profile_assistant_messages.len() >= MAX_MESSAGE_RECORDS {
                return Err(super::claude_profile::reject(
                    "claude_profile_assistant_history_limit",
                ));
            }
            self.profile_assistant_messages
                .insert(message_id.to_owned(), turn_id);
        }
        if self.active_turn != Some(turn_id)
            || !self.turn_is_running(turn_id)
            || self.turns[&turn_id]
                .cancellation
                .as_ref()
                .is_some_and(|cancellation| cancellation.interrupt_requested)
        {
            return Ok(None);
        }
        if explicit.is_some() {
            self.profile_assistant_origin = Some(turn_id);
        }
        Ok(Some(turn_id))
    }

    fn receive_approval(&mut self, message: &Value) -> Result<Effects, RuntimeError> {
        let id = required_string(message, "request_id")?.to_string();
        let request = &message["request"];
        if self.local_tools.contains_key(&id) || self.mcp_replies.contains_key(&id) {
            return Ok(control_error(&id, "native request id was already used"));
        }
        let request_fingerprint = fingerprint(request);
        if let Some(previous) = self.approvals.get(&id) {
            if previous.fingerprint != request_fingerprint {
                return Ok(control_error(
                    &id,
                    "approval id reused with different contents",
                ));
            }
            if previous.cancelled
                || self.active_turn != Some(previous.turn_id)
                || !self.turn_is_running(previous.turn_id)
                || self.turns[&previous.turn_id]
                    .cancellation
                    .as_ref()
                    .is_some_and(|cancellation| cancellation.interrupt_requested)
            {
                return Ok(control_error(
                    &id,
                    "approval belongs to an inactive command",
                ));
            }
            // 逐次允许只能消费一次，重复回调不能绕过新的路径与权限复核。
            if self.options.claude_profile.is_some()
                && previous
                    .response
                    .as_ref()
                    .is_some_and(|response| response["response"]["response"]["behavior"] == "allow")
            {
                return Ok(control_error(&id, "one-time approval was already consumed"));
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
        if self.turns[&turn_id]
            .cancellation
            .as_ref()
            .is_some_and(|cancellation| cancellation.interrupt_requested)
        {
            return Ok(control_error(&id, "command cancellation is pending"));
        }
        if !request["input"].is_object()
            || request.get("tool_name").and_then(Value::as_str).is_none()
        {
            return Ok(control_error(&id, "invalid tool approval request"));
        }
        if self.options.claude_profile.is_some() {
            let related = request["tool_use_id"]
                .as_str()
                .and_then(|id| self.profile_tool_calls.get(id));
            if !related.is_some_and(|(turn, tool, input)| {
                *turn == turn_id
                    && request["tool_name"] == *tool
                    && *input == fingerprint(&request["input"])
            }) {
                return Ok(control_error(
                    &id,
                    "tool approval does not match this running command",
                ));
            }
        }
        if self.approvals.len() >= MAX_MESSAGE_RECORDS {
            return Err(RuntimeError::Protocol("approval limit reached".into()));
        }
        self.approvals.insert(
            id.clone(),
            PendingApproval {
                fingerprint: request_fingerprint,
                turn_id,
                input: request["input"].clone(),
                tool_name: request["tool_name"]
                    .as_str()
                    .expect("validated tool name")
                    .to_owned(),
                response: None,
                cancelled: false,
            },
        );
        if self.options.claude_profile.as_ref().is_some_and(|profile| {
            !profile.approval_allowed(
                request["tool_name"].as_str().expect("validated tool name"),
                &request["input"],
            )
        }) {
            let response = control_response(
                &id,
                json!({"behavior":"deny","message":"The fixed task permission profile denies this tool invocation"}),
            );
            self.approvals
                .get_mut(&id)
                .expect("approval exists")
                .response = Some(response.clone());
            return Ok(Effects {
                writes: vec![response],
                events: Vec::new(),
            });
        }
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

    fn finish_execution_batch(
        &mut self,
        turn_id: Uuid,
        outcome: TurnOutcome,
        output: Option<&str>,
        events: &mut Vec<RuntimeEventKind>,
    ) {
        let mut joined = self
            .turns
            .iter()
            .filter_map(|(id, turn)| {
                (!turn.finished && turn.joined_to == Some(turn_id)).then_some(*id)
            })
            .collect::<Vec<_>>();
        joined.sort_unstable();
        let output = output
            .map(str::to_owned)
            .or_else(|| self.turns.get(&turn_id).map(|turn| turn.output.clone()));
        // 合并输入先提交终态，随后才能关闭共享执行；未知取消只能报告失败。
        for id in joined {
            self.finish_turn(id, outcome.clone(), output.as_deref(), events);
        }
        self.finish_turn(turn_id, outcome, output.as_deref(), events);
    }

    fn finish_turn(
        &mut self,
        turn_id: Uuid,
        outcome: TurnOutcome,
        output: Option<&str>,
        events: &mut Vec<RuntimeEventKind>,
    ) {
        self.clear_assistant_origin(turn_id);
        let Some(turn) = self.turns.get_mut(&turn_id) else {
            return;
        };
        if turn.finished {
            return;
        }
        turn.finished = true;
        turn.cancellation = None;
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
    if matches!(terminal_reason, Some("interrupted" | "cancelled"))
        && subtype == "success"
        && message.get("is_error").and_then(Value::as_bool) == Some(false)
    {
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

enum InputProjection {
    Text {
        content: String,
        expected_replay: String,
    },
    Blocks {
        content: Value,
        expected_replay: BlockReplay,
    },
}

impl InputProjection {
    fn into_parts(self) -> (Value, ExpectedReplay) {
        match self {
            Self::Text {
                content,
                expected_replay,
            } => (json!(content), ExpectedReplay::Text(expected_replay)),
            Self::Blocks {
                content,
                expected_replay,
            } => (content, ExpectedReplay::Blocks(expected_replay)),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ExpectedReplay {
    Text(String),
    Blocks(BlockReplay),
}

impl ExpectedReplay {
    fn matches(&self, content: &Value) -> bool {
        match self {
            Self::Text(expected) => content.as_str() == Some(expected.as_str()),
            Self::Blocks(expected) => summarize_blocks(content).as_ref() == Ok(expected),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct BlockReplay {
    sha256: [u8; 32],
    bytes: usize,
    kinds: Vec<InputBlockKind>,
}

#[derive(Debug, PartialEq, Eq)]
enum InputBlockKind {
    Text,
    PngImage,
}

/// 长期账本只保留有界摘要；严格形状后重新构造规范 JSON，字段顺序不影响关联。
fn summarize_blocks(content: &Value) -> Result<BlockReplay, String> {
    let blocks = content
        .as_array()
        .ok_or_else(|| crate::t!("cli-agent-claude-image-replay-invalid"))?;
    if blocks.len() < 2 || blocks.len() > MAX_IMAGE_COUNT_FOR_QUERY + 1 {
        return Err(crate::t!("cli-agent-claude-image-replay-invalid"));
    }
    let mut canonical = Vec::with_capacity(blocks.len());
    let mut kinds = Vec::with_capacity(blocks.len());
    let mut payload_bytes = 0usize;
    for (index, block) in blocks.iter().enumerate() {
        let fields = block
            .as_object()
            .ok_or_else(|| crate::t!("cli-agent-claude-image-replay-invalid"))?;
        if fields.len() != 2 {
            return Err(crate::t!("cli-agent-claude-image-replay-invalid"));
        }
        match block["type"].as_str() {
            Some("text") if index == 0 => {
                let text = block["text"]
                    .as_str()
                    .ok_or_else(|| crate::t!("cli-agent-claude-image-replay-invalid"))?;
                if text.is_empty() || text.len() > MAX_INPUT_BYTES {
                    return Err(crate::t!("cli-agent-claude-image-replay-invalid"));
                }
                payload_bytes = payload_bytes.saturating_add(text.len());
                if payload_bytes > MAX_LINE_BYTES {
                    return Err(crate::t!("editor-image-too-large"));
                }
                canonical.push(json!({"type":"text","text":text}));
                kinds.push(InputBlockKind::Text);
            }
            Some("image") if index > 0 => {
                let source = &block["source"];
                if source.as_object().is_none_or(|fields| fields.len() != 3)
                    || source["type"] != "base64"
                    || source["media_type"] != "image/png"
                    || source["data"]
                        .as_str()
                        .is_none_or(|data| data.is_empty() || data.len() > MAX_LINE_BYTES)
                {
                    return Err(crate::t!("cli-agent-claude-image-replay-invalid"));
                }
                payload_bytes = payload_bytes
                    .saturating_add(source["data"].as_str().expect("已验证图片块").len());
                if payload_bytes > MAX_LINE_BYTES {
                    return Err(crate::t!("editor-image-too-large"));
                }
                canonical.push(json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":source["data"]}}));
                kinds.push(InputBlockKind::PngImage);
            }
            Some(_) | None => return Err(crate::t!("cli-agent-claude-image-replay-invalid")),
        }
    }
    let encoded = serde_json::to_vec(&canonical)
        .map_err(|_| crate::t!("cli-agent-claude-image-replay-invalid"))?;
    if encoded.len() > MAX_LINE_BYTES {
        return Err(crate::t!("editor-image-too-large"));
    }
    Ok(BlockReplay {
        sha256: Sha256::digest(&encoded).into(),
        bytes: encoded.len(),
        kinds,
    })
}

fn encode_input(
    input: Vec<InputContent>,
    plugin: Option<&PreparedClaudeSkillPlugin>,
    attachment_store: &Path,
) -> Result<InputProjection, String> {
    if input
        .iter()
        .any(|part| matches!(part, InputContent::LocalImage(_)))
        && input
            .iter()
            .any(|part| matches!(part, InputContent::Skill { .. }))
    {
        return Err(crate::t!("cli-agent-claude-image-skill-unverified"));
    }
    let mut texts = Vec::new();
    let mut images = Vec::new();
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
            InputContent::LocalImage(path) => images.push(path),
        }
    }
    let text = texts.join("\n\n");
    if !images.is_empty() {
        if text.len() > MAX_INPUT_BYTES {
            return Err(crate::t!("cli-agent-input-text-too-large"));
        }
        if text.trim().is_empty() {
            return Err(crate::t!("cli-task-manager-empty-prompt"));
        }
        if images.len() > MAX_IMAGE_COUNT_FOR_QUERY {
            return Err(crate::t!(
                "editor-images-disabled-query-limit",
                limit = MAX_IMAGE_COUNT_FOR_QUERY
            ));
        }
        let mut payload_bytes = text.len();
        let mut blocks = vec![json!({"type":"text","text":text})];
        for path in images {
            // 逐张验证并检查总预算，失败不派发，避免同时加载二十张大图。
            let image = restore_managed_images(vec![path], attachment_store)?
                .pop()
                .expect("单张图片还原完整返回");
            if image.mime_type != "image/png" {
                return Err(crate::t!("cli-agent-claude-png-only"));
            }
            payload_bytes = payload_bytes.saturating_add(image.data.len());
            if payload_bytes > MAX_LINE_BYTES {
                return Err(crate::t!("editor-image-too-large"));
            }
            blocks.push(json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":image.data}}));
        }
        let content = json!(blocks);
        let expected_replay = summarize_blocks(&content)?;
        return Ok(InputProjection::Blocks {
            content,
            expected_replay,
        });
    }
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
    Ok(InputProjection::Text {
        content,
        expected_replay,
    })
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

#[cfg(test)]
#[path = "claude_live_tests.rs"]
mod live_tests;

#[cfg(test)]
#[path = "claude_profile_protocol_tests.rs"]
mod profile_tests;
