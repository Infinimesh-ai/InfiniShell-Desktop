//! 固定原生版本的一次图片校准；不经过字符串回放适配器，也不开放生产图片门禁。

use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use command::Stdio;
use command::r#async::Command;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::super::{assistant_text, live_native_protocol_ids, write_message};
use super::Evidence;
use crate::ai::cli_agent_runtime::managed_process::{self, ExitReason};

const SCOPE: &str = "claude_native_image_input_probe";
const VERSION: &str = "2.1.273 (Claude Code)";
const MAX_LINE: usize = 8 * 1024 * 1024;
const MAX_ROWS: usize = 256;
const PROMPT: &str = "Inspect only the attached image. Identify the solid color of each quadrant in this order: top-left, top-right, bottom-left, bottom-right. Reply with exactly four uppercase color names separated by single spaces. Use the names RED, GREEN, BLUE, YELLOW. Do not use tools, read files, explain, or add punctuation.";

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut value = u32::MAX;
    for byte in bytes {
        value ^= u32::from(*byte);
        for _bit in 0..8 {
            value = (value >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(value & 1));
        }
    }
    !value
}

fn png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], bytes: &[u8]) {
    png.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(bytes);
    let mut checked = kind.to_vec();
    checked.extend_from_slice(bytes);
    png.extend_from_slice(&crc32(&checked).to_be_bytes());
}

pub(super) fn quadrant_png(seed: Uuid) -> (Vec<u8>, String) {
    // 标准库生成 RGB PNG，随机排列只存在像素中，提示词不携带正确顺序。
    let colors = [
        ("RED", [255, 0, 0]),
        ("GREEN", [0, 255, 0]),
        ("BLUE", [0, 0, 255]),
        ("YELLOW", [255, 255, 0]),
    ];
    let mut order = [0_usize, 1, 2, 3];
    for index in (1..4).rev() {
        order.swap(index, usize::from(seed.as_bytes()[index]) % (index + 1));
    }
    let mut pixels = Vec::new();
    for y in 0..64 {
        pixels.push(0);
        for x in 0..64 {
            let quadrant = usize::from(y >= 32) * 2 + usize::from(x >= 32);
            pixels.extend_from_slice(&colors[order[quadrant]].1);
        }
    }
    // 一块无压缩 DEFLATE，加上标准 Adler-32；不依赖图片编辑器或外部库。
    let length = pixels.len() as u16;
    let mut compressed = vec![0x78, 0x01, 0x01];
    compressed.extend_from_slice(&length.to_le_bytes());
    compressed.extend_from_slice(&(!length).to_le_bytes());
    compressed.extend_from_slice(&pixels);
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in &pixels {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    compressed.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = 64_u32.to_be_bytes().to_vec();
    header.extend_from_slice(&64_u32.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_chunk(&mut png, b"IHDR", &header);
    png_chunk(&mut png, b"IDAT", &compressed);
    png_chunk(&mut png, b"IEND", &[]);
    let expected = order
        .iter()
        .map(|index| colors[*index].0)
        .collect::<Vec<_>>()
        .join(" ");
    (png, expected)
}

fn content_projection(content: &Value, exact_array: bool) -> Value {
    match content {
        Value::Array(blocks) => {
            let projected = blocks.iter().map(|block| match block["type"].as_str() {
                Some("text") => json!({"type":"text", "text_sha256":block["text"].as_str().map(|text|sha(text.as_bytes())),
                    "text_bytes":block["text"].as_str().map(str::len)}),
                Some("image") => {
                    let decoded = block["source"]["data"].as_str().and_then(|data|STANDARD.decode(data).ok());
                    json!({"type":"image", "source_type":match block["source"]["type"].as_str() {
                        Some("base64") => "base64", Some(_) | None => "unknown",
                    }, "media_type":match block["source"]["media_type"].as_str() {
                        Some("image/png") => "image/png", Some(_) | None => "unknown",
                    }, "image_sha256":decoded.as_ref().map(|bytes|sha(bytes)), "image_bytes":decoded.as_ref().map(Vec::len)})
                }
                Some("thinking" | "redacted_thinking") => json!({"type":"thinking"}),
                Some("tool_use") => json!({"type":"tool_use"}),
                Some(_) | None => json!({"type":"unknown"}),
            }).collect::<Vec<_>>();
            // 只有用户图片回放保留整份数组散列；assistant 不散列思考正文。
            json!({"shape":"array", "array_sha256":exact_array.then(||sha(&serde_json::to_vec(content).expect("JSON 数组可编码"))), "blocks":projected})
        }
        Value::String(text) => {
            json!({"shape":"string", "text_sha256":sha(text.as_bytes()), "text_bytes":text.len()})
        }
        Value::Null => json!({"shape":"missing"}),
        Value::Bool(_) | Value::Number(_) | Value::Object(_) => json!({"shape":"other"}),
    }
}

fn projection(message: &Value, direction: &str) -> Value {
    let mut ids = live_native_protocol_ids(message);
    ids["direction"] = json!(direction);
    ids["is_replay"] = json!(message["isReplay"].as_bool());
    ids["message_role"] = json!(match message["message"]["role"].as_str() {
        Some("user") => Some("user"),
        Some("assistant") => Some("assistant"),
        Some(_) => Some("unknown"),
        None => None,
    });
    ids["assistant_error_present"] = json!(
        message["is_api_error_message"] == true
            || message.get("error").is_some_and(|error| !error.is_null())
    );
    ids["parent_tool_use_present"] = json!(
        message
            .get("parent_tool_use_id")
            .is_some_and(|id| !id.is_null())
    );
    ids["content"] = content_projection(&message["message"]["content"], message["type"] == "user");
    ids["assistant_text_sha256"] =
        json!((message["type"] == "assistant").then(|| sha(assistant_text(message).as_bytes())));
    ids["assistant_trimmed_sha256"] = json!(
        (message["type"] == "assistant").then(|| sha(assistant_text(message).trim().as_bytes()))
    );
    ids["result_text_sha256"] = json!(message["result"].as_str().map(|text| sha(text.as_bytes())));
    ids["result_trimmed_sha256"] = json!(
        message["result"]
            .as_str()
            .map(|text| sha(text.trim().as_bytes()))
    );
    ids["system_tool_count"] = json!(
        (message["type"] == "system" && message["subtype"] == "init")
            .then(|| message["tools"].as_array().map(Vec::len))
            .flatten()
    );
    ids["system_mcp_count"] = json!(
        (message["type"] == "system" && message["subtype"] == "init")
            .then(|| message["mcp_servers"].as_array().map(Vec::len))
            .flatten()
    );
    ids["native_version"] = json!(
        if message["type"] == "system" && message["subtype"] == "init" {
            Some(if message["claude_code_version"] == "2.1.273" {
                "2.1.273"
            } else {
                "unknown"
            })
        } else {
            None
        }
    );
    ids["initialize_session_state"] = json!(match message
        .pointer("/response/response/session_state")
        .and_then(Value::as_str)
    {
        Some("idle") => Some("idle"),
        Some(_) => Some("unknown"),
        None => None,
    });
    let mode = message
        .pointer("/response/response/current_permission_mode")
        .or_else(|| message.get("permissionMode"));
    ids["permission_mode"] = json!(match mode.and_then(Value::as_str) {
        Some("default") => Some("default"),
        Some(_) => Some("unknown"),
        None => None,
    });
    ids
}

fn uuid(value: &Value) -> Result<Uuid, &'static str> {
    let text = value.as_str().ok_or("missing_native_uuid")?;
    let id = Uuid::parse_str(text).map_err(|_| "invalid_native_uuid")?;
    if id.is_nil() || id.to_string() != text {
        return Err("invalid_native_uuid");
    }
    Ok(id)
}

struct Framing {
    buffer: Vec<u8>,
    sequence: usize,
}

impl Framing {
    fn record(
        &mut self,
        evidence: &mut Evidence,
        message: &Value,
        direction: &str,
    ) -> Result<(), String> {
        self.sequence += 1;
        if self.sequence > MAX_ROWS {
            return Err("native_ledger_limit".into());
        }
        if message["user_message_uuids"]
            .as_array()
            .is_some_and(|ids| ids.len() > 4)
            || message["message"]["content"]
                .as_array()
                .is_some_and(|blocks| blocks.len() > 8)
        {
            return Err("native_projection_limit".into());
        }
        evidence.record(json!({"event":"native_protocol_ids", "sequence":self.sequence, "native":projection(message,direction)}))
    }

    async fn send(
        &mut self,
        evidence: &mut Evidence,
        stdin: &mut (impl AsyncWrite + Unpin),
        message: &Value,
    ) -> Result<(), String> {
        write_message(stdin, message)
            .await
            .map_err(|_| "native_transport_write_failed".to_owned())?;
        self.record(evidence, message, "stdin")
    }

    async fn next(
        &mut self,
        evidence: &mut Evidence,
        stdout: &mut (impl AsyncRead + Unpin),
    ) -> Result<Value, String> {
        loop {
            if let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                if end > MAX_LINE {
                    return Err("native_frame_limit".into());
                }
                let line = self.buffer.drain(..=end).collect::<Vec<_>>();
                let message: Value = serde_json::from_slice(&line)
                    .map_err(|_| "native_frame_not_json".to_owned())?;
                if !message.is_object() {
                    return Err("native_frame_not_object".into());
                }
                self.record(evidence, &message, "stdout")?;
                return Ok(message);
            }
            if self.buffer.len() > MAX_LINE {
                return Err("native_frame_limit".into());
            }
            let mut chunk = [0; 8192];
            let count = stdout
                .read(&mut chunk)
                .await
                .map_err(|_| "native_transport_read_failed".to_owned())?;
            if count == 0 {
                return Err("native_transport_closed_before_result".into());
            }
            self.buffer.extend_from_slice(&chunk[..count]);
        }
    }
}

async fn observe(
    evidence: &mut Evidence,
    generation: Uuid,
    stdin: &mut (impl AsyncWrite + Unpin),
    stdout: &mut (impl AsyncRead + Unpin),
) -> Result<Value, String> {
    let mut framing = Framing {
        buffer: Vec::new(),
        sequence: 0,
    };
    let request_id = format!("infinishell-{generation}-1");
    framing.send(evidence,stdin,&json!({"type":"control_request","request_id":request_id,"request":{"subtype":"initialize"}})).await?;
    let input_id = Uuid::new_v4();
    let (png, expected) = quadrant_png(Uuid::new_v4());
    let content = json!([{ "type":"text", "text":PROMPT }, {"type":"image", "source":{
        "type":"base64", "media_type":"image/png", "data":STANDARD.encode(&png)
    }}]);
    evidence.record(json!({"event":"png_generated", "image_sha256":sha(&png), "image_bytes":png.len(),
        "width":64,"height":64,"quadrants":4,"expected_reply_sha256":sha(expected.as_bytes()), "prompt_sha256":sha(PROMPT.as_bytes())}))?;
    let mut initialized = false;
    let mut native = None;
    let mut acknowledged = false;
    let mut started = false;
    let mut replay = false;
    let mut system_verified = false;
    let mut assistant_ids = HashSet::new();
    let mut output = String::new();
    loop {
        let message = framing.next(evidence, stdout).await?;
        if message["type"] == "control_request" {
            let id = message["request_id"]
                .as_str()
                .filter(|id| id.len() <= 256 && !id.chars().any(char::is_control))
                .ok_or("invalid_permission_request_id")?;
            let response = if message["request"]["subtype"] == "can_use_tool" {
                json!({"type":"control_response","response":{"subtype":"success","request_id":id,
                    "response":{"behavior":"deny","message":"Image calibration forbids all tools","tool_use_id":message["request"]["tool_use_id"]}}})
            } else {
                json!({"type":"control_response","response":{"subtype":"error","request_id":id,"error":"Image calibration forbids control requests"}})
            };
            framing.send(evidence, stdin, &response).await?;
            evidence.record(json!({"event":"tool_request_rejected","transport_write_confirmed":true,"probe_failed":true}))?;
            return Err("unexpected_tool_or_permission_request".into());
        }
        if message["type"] == "control_response" {
            if initialized
                || message["response"]["request_id"] != request_id
                || message["response"]["subtype"] != "success"
                || message["response"]["response"]["session_state"] != "idle"
                || message["response"]["response"]["current_permission_mode"] != "default"
            {
                return Err("native_initialize_not_verified".into());
            }
            initialized = true;
            // 只写入一次。初始空 session ID 沿用生产新建会话的实际 framing。
            framing
                .send(
                    evidence,
                    stdin,
                    &json!({"type":"user","message":{"role":"user","content":content},
                "parent_tool_use_id":null,"session_id":"","uuid":input_id}),
                )
                .await?;
            evidence.record(
                json!({"event":"input_submitted","message_id":input_id,"submitted_input_count":1,
                "array_content":true,"transport_write_confirmed":true}),
            )?;
            continue;
        }
        if !initialized {
            return Err("native_event_before_initialize".into());
        }
        if matches!(
            message["type"].as_str(),
            Some("command_lifecycle" | "user" | "system" | "assistant" | "result")
        ) && !message.get("session_id").is_some_and(Value::is_string)
        {
            return Err("native_event_missing_session".into());
        }
        if let Some(session) = message.get("session_id") {
            let session = uuid(session).map_err(str::to_owned)?;
            if native.is_some_and(|current| current != session) {
                return Err("native_session_changed".into());
            }
            native = Some(session);
        }
        match message["type"].as_str() {
            Some("command_lifecycle") => {
                if uuid(&message["command_uuid"]).map_err(str::to_owned)? != input_id
                    || native.is_none()
                {
                    return Err("native_lifecycle_wrong_input".into());
                }
                match message["state"].as_str() {
                    Some("queued") => acknowledged = true,
                    Some("started") => {
                        acknowledged = true;
                        started = true;
                    }
                    Some("completed") => {}
                    Some("cancelled") => return Err("native_input_cancelled".into()),
                    Some(_) | None => return Err("native_lifecycle_unknown".into()),
                }
            }
            Some("user") => {
                if replay
                    || message["isReplay"] != true
                    || uuid(&message["uuid"]).map_err(str::to_owned)? != input_id
                    || message["message"]["role"] != "user"
                    || message["message"]["content"] != content
                    || native.is_none()
                    || message
                        .get("parent_tool_use_id")
                        .is_some_and(|id| !id.is_null())
                {
                    return Err("native_image_replay_not_exact".into());
                }
                replay = true;
            }
            Some("system") if message["subtype"] == "init" => {
                if system_verified
                    || message["claude_code_version"] != "2.1.273"
                    || message["permissionMode"] != "default"
                    || message["tools"]
                        .as_array()
                        .is_none_or(|tools| !tools.is_empty())
                    || message["mcp_servers"]
                        .as_array()
                        .is_none_or(|servers| !servers.is_empty())
                    || native.is_none()
                {
                    return Err("native_no_tools_not_verified".into());
                }
                system_verified = true;
            }
            Some("assistant") => {
                if !started
                    || native.is_none()
                    || message["message"]["role"] != "assistant"
                    || uuid(&message["user_message_uuid"]).map_err(str::to_owned)? != input_id
                    || message
                        .get("parent_tool_use_id")
                        .is_some_and(|id| !id.is_null())
                    || message["is_api_error_message"] == true
                    || message.get("error").is_some_and(|error| !error.is_null())
                    || !assistant_ids.insert(uuid(&message["uuid"]).map_err(str::to_owned)?)
                {
                    return Err("native_assistant_origin_not_verified".into());
                }
                let blocks = message["message"]["content"]
                    .as_array()
                    .ok_or("native_assistant_content_not_array")?;
                if blocks.iter().any(|block| {
                    !matches!(
                        block["type"].as_str(),
                        Some("text" | "thinking" | "redacted_thinking")
                    )
                }) {
                    return Err("unexpected_assistant_tool_or_block".into());
                }
                let text = assistant_text(&message);
                if output.len() + text.len() > 4096 {
                    return Err("native_assistant_output_limit".into());
                }
                if !text.is_empty() {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&text);
                }
            }
            Some("result") => {
                let result_id = uuid(&message["uuid"]).map_err(str::to_owned)?;
                let ids = message["user_message_uuids"]
                    .as_array()
                    .ok_or("native_result_missing_full_input_array")?;
                if !acknowledged
                    || !started
                    || !replay
                    || !system_verified
                    || assistant_ids.is_empty()
                    || native.is_none()
                    || ids.len() != 1
                    || uuid(&ids[0]).map_err(str::to_owned)? != input_id
                    || uuid(&message["user_message_uuid"]).map_err(str::to_owned)? != input_id
                    || message["subtype"] != "success"
                    || message["is_error"] != false
                    || !matches!(
                        message["terminal_reason"].as_str(),
                        None | Some("completed" | "end_turn")
                    )
                    || output.trim() != expected
                    || message["result"].as_str().map(str::trim) != Some(expected.as_str())
                {
                    return Err("native_image_completion_not_verified".into());
                }
                return Ok(
                    json!({"event":"native_image_proof", "native_session_id":native,"message_id":input_id,
                    "native_result_uuid":result_id,"user_message_uuids":[input_id],"native_inputs":1,
                    "replay_array_verified":true,"native_ack_verified":true,"native_started_verified":true,
                    "native_no_tools_verified":true,"assistant_origin_verified":true,"assistant_frames":assistant_ids.len(),
                    "assistant_output_sha256":sha(output.as_bytes()),"assistant_trimmed_sha256":sha(output.trim().as_bytes()),
                    "result_trimmed_sha256":sha(expected.as_bytes()),"image_pixels_verified":true}),
                );
            }
            Some("keep_alive" | "rate_limit_event") => {}
            // 未知 frame 不作为成功依据；包含工具进度、部分输出或额外任务时直接失败。
            Some(_) | None => return Err("unexpected_native_frame".into()),
        }
    }
}

async fn exercise(root: &Path, evidence: &mut Evidence, generation: Uuid) -> Result<Value, String> {
    let executable = env::var_os("INFINISHELL_CLAUDE_LIVE_EXECUTABLE")
        .map(PathBuf::from)
        .ok_or("missing_native_executable")?;
    if !executable.is_absolute() || !executable.is_file() {
        return Err("invalid_native_executable".into());
    }
    let model = env::var("INFINISHELL_CLAUDE_LIVE_MODEL")
        .map_err(|_| "missing_explicit_model".to_owned())?;
    if model.is_empty() || model.chars().any(char::is_control) {
        return Err("invalid_explicit_model".into());
    }
    let mut version = Command::new(&executable);
    version
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let detected = tokio::time::timeout(Duration::from_secs(5), version.output())
        .await
        .map_err(|_| "native_version_timeout".to_owned())?
        .map_err(|_| "native_version_read_failed".to_owned())?;
    if !detected.status.success() || String::from_utf8_lossy(&detected.stdout).trim() != VERSION {
        return Err("unverified_native_version".into());
    }
    let arguments = [
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
        "--permission-mode",
        "default",
        "--bare",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--model",
        &model,
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    let mut child = managed_process::spawn(
        root,
        generation,
        &executable,
        &arguments,
        &root.join("project"),
    )
    .await
    .map_err(|_| "native_spawn_failed".to_owned())?;
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take();
    let result = match (&mut stdin, &mut stdout) {
        (Some(stdin), Some(stdout)) => tokio::time::timeout(
            Duration::from_secs(180),
            observe(evidence, generation, stdin, stdout),
        )
        .await
        .map_err(|_| "native_probe_timeout".to_owned())
        .and_then(|result| result),
        (Some(_) | None, None) | (None, Some(_)) => Err("native_pipes_missing".into()),
    };
    drop(stdin);
    // 保留输出读端直至 EOF；过早释放会令监督者转发剩余终态时失败。
    let drain = async {
        let mut bytes = Vec::new();
        if let Some(stdout) = stdout {
            stdout
                .take(MAX_LINE as u64 + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "native_shutdown_output_read_failed".to_owned())?;
        }
        if bytes.len() > MAX_LINE {
            return Err("native_shutdown_output_budget_exceeded".to_owned());
        }
        Ok(bytes)
    };
    let (finished, drained) = futures::join!(child.finish(), async {
        tokio::time::timeout(Duration::from_secs(30), drain)
            .await
            .map_err(|_| "native_shutdown_output_timeout".to_owned())
            .and_then(|result| result)
    });
    let shutdown_verified = match &drained {
        Ok(bytes) => {
            let mut rows = 0;
            let mut verified = true;
            let mut projections = Vec::new();
            for line in bytes.split(|byte| *byte == b'\n') {
                if line.is_empty() {
                    continue;
                }
                rows += 1;
                let message = serde_json::from_slice::<Value>(line);
                match message {
                    Ok(message) => {
                        if rows <= MAX_ROWS {
                            projections.push(projection(&message, "stdout"));
                        }
                        verified &= rows == 1
                            && message["type"] == "command_lifecycle"
                            && message["state"] == "completed"
                            && result.as_ref().is_ok_and(|proof| {
                                message["session_id"] == proof["native_session_id"]
                                    && message["command_uuid"] == proof["message_id"]
                            });
                    }
                    Err(_) => verified = false,
                }
            }
            evidence.record(
                json!({"event":"shutdown_output_drained","bytes":bytes.len(),
                "sha256":sha(bytes),"frames":rows,"native":projections,"verified":verified}),
            )?;
            verified
        }
        Err(reason) => {
            evidence.record(json!({"event":"shutdown_output_failed","reason":reason}))?;
            false
        }
    };
    let receipt = managed_process::confirmed_exit(root, generation)
        .map_err(|_| "cleanup_receipt_read_failed".to_owned())?;
    let confirmed = finished
        .as_ref()
        .is_ok_and(|exit| exit.generation == generation && exit.cleanup_confirmed)
        && receipt
            .as_ref()
            .is_some_and(|exit| exit.generation == generation && exit.cleanup_confirmed);
    // 正确图片结果与清理完成不能代替正常退出；两次真实回执均须确认标准输入关闭后的零退出码。
    let normal_exit = finished
        .as_ref()
        .is_ok_and(|exit| exit.exit_code == Some(0) && exit.exit_reason == ExitReason::StdioClosed)
        && receipt.as_ref().is_some_and(|exit| {
            exit.exit_code == Some(0) && exit.exit_reason == ExitReason::StdioClosed
        });
    evidence.record(json!({"event":"cleanup_checked","generation":generation,"cleanup_confirmed":confirmed,
        "transport_closed":finished.is_ok(),"cleanup_receipt_read":true,"receipt":receipt.map(|exit|json!({
            "generation":exit.generation,"containment":exit.containment,"cleanup_confirmed":exit.cleanup_confirmed,
            "exit_code":exit.exit_code,"exit_reason":exit.exit_reason}))}))?;
    if let Err(error) = &finished {
        evidence.record(
            json!({"event":"supervisor_finish_failed","error_kind":format!("{:?}",error.kind()),
            "error_sha256":sha(error.to_string().as_bytes())}),
        )?;
    }
    if !confirmed {
        return Err("native_cleanup_not_confirmed".into());
    }
    if !normal_exit {
        return Err("native_probe_exit_not_clean".into());
    }
    if !shutdown_verified {
        return Err("native_shutdown_output_not_verified".into());
    }
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "仅隔离图片校准运行器显式启动；真实模型会产生费用"]
async fn real_claude_image_input_probe() -> Result<(), String> {
    let root = env::var_os("INFINISHELL_CLAUDE_LIVE_ROOT")
        .map(PathBuf::from)
        .ok_or("missing_probe_root")?
        .canonicalize()
        .map_err(|_| "invalid_probe_root".to_owned())?;
    if fs::read(root.join(".infinishell-claude-live-probe"))
        .map_err(|_| "missing_isolation_marker".to_owned())?
        != b"isolated Claude Rust adapter verification\n"
    {
        return Err("invalid_isolation_marker".into());
    }
    let declared = env::var_os("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR")
        .map(PathBuf::from)
        .ok_or("missing_private_config")?
        .canonicalize()
        .map_err(|_| "invalid_private_config".to_owned())?;
    let actual = env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .ok_or("missing_native_private_config")?
        .canonicalize()
        .map_err(|_| "invalid_native_private_config".to_owned())?;
    if actual != declared || root == declared || root.starts_with(&declared) {
        return Err("private_config_boundary_invalid".into());
    }
    let artifact = env::var_os("INFINISHELL_CLAUDE_LIVE_ARTIFACT")
        .map(PathBuf::from)
        .ok_or("missing_artifact")?;
    let mut evidence = Evidence {
        file: File::create(artifact).map_err(|_| "artifact_create_failed".to_owned())?,
        root: root.clone(),
    };
    let generation = Uuid::new_v4();
    evidence.record(json!({"event":"acceptance_started","scope":SCOPE,"generation":generation,"max_native_inputs":1,
        "native_framing_only":true,"production_image_gate_open":false,"credential_files_read_by_probe":false,
        "real_gui_verified":false,"persistence_verified":false,"http_request_count_verified":false}))?;
    match exercise(&root, &mut evidence, generation).await {
        Ok(proof) => {
            let native = proof["native_session_id"].clone();
            let input = proof["message_id"].clone();
            evidence.record(proof)?;
            evidence.record(json!({"event":"acceptance_passed","scope":SCOPE,"native_session_id":native,"message_id":input,
                "native_inputs":1,"native_framing_only":true,"production_image_gate_open":false,"cleanup_confirmed":true,
                "transport_closed":true,"credential_files_read_by_probe":false,"persistence_verified":false,
                "app_restart_and_ui_verified":false,"parent_permission_ceiling_verified":false,"http_request_count_verified":false}))?;
            Ok(())
        }
        Err(reason) => {
            evidence.record(json!({"event":"acceptance_failed","scope":SCOPE,"reason":reason}))?;
            Err(reason)
        }
    }
}
