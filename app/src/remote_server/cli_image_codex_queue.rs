//! 固定 Codex 的原生图片队列消费者；只连接已绑定的守护进程，不启动或恢复会话。
//!
//! 0.156.1 的 thread/queue/add 会先把 localImage 保存为内联图片再返回回执。
//! 只有逐张校验该回执后，上层才可释放暂存引用。排队不代表当前回合追加或模型理解。

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use websocket::tungstenite::{
    Message, WebSocket, client::client_with_config, protocol::WebSocketConfig,
};

const FIXED_VERSION: &str = "0.156.1";
const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGES: usize = 20;
const MAX_READ_MESSAGES: usize = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 图片已经由暂存服务封存；路径不能来自用户文本或客户端直接传入的任意路径。
pub(crate) struct CodexQueueImage {
    pub(crate) path: PathBuf,
    pub(crate) byte_len: u64,
    pub(crate) sha256: [u8; 32],
}

/// 在第一个可能改变队列的字节写出之前持久化；该编号不是原生去重保证。
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CodexQueueAttempt {
    pub(crate) rpc_id: Uuid,
    pub(crate) thread_id: Uuid,
    pub(crate) client_message_id: Uuid,
    pub(crate) request_sha256: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CodexQueueReceipt {
    pub(crate) attempt: CodexQueueAttempt,
    pub(crate) queued_submission_id: String,
    pub(crate) image_sha256: Vec<[u8; 32]>,
    pub(crate) raw_ack_sha256: [u8; 32],
}

pub(crate) struct CodexImageQueue {
    socket: WebSocket<BoundedSocket>,
    thread_id: Uuid,
    cwd: PathBuf,
}

impl CodexImageQueue {
    /// validate 必须核对内核对端、当前终端绑定及固定可执行映像，不能只比 socket 路径。
    /// 此方法需在后台线程调用，不能持有 TerminalModel 锁。
    pub(crate) fn connect(
        stream: UnixStream,
        thread_id: Uuid,
        cwd: &Path,
        validate: &impl Fn(&UnixStream) -> io::Result<()>,
    ) -> io::Result<Self> {
        if thread_id.is_nil() || !cwd.is_absolute() || cwd.to_str().is_none() {
            return Err(rejected("invalid_binding"));
        }
        validate(&stream)?;
        stream.set_nonblocking(false)?;
        let config = WebSocketConfig {
            write_buffer_size: 0,
            max_write_buffer_size: MAX_MESSAGE_BYTES + 1024,
            max_message_size: Some(MAX_MESSAGE_BYTES),
            max_frame_size: Some(MAX_MESSAGE_BYTES),
            ..WebSocketConfig::default()
        };
        // 原生控制套接字使用 WebSocket；URL 仅生成握手头，不进行 TCP 或代理连接。
        let (socket, _) =
            client_with_config("ws://localhost/", BoundedSocket::new(stream), Some(config))
                .map_err(|_| rejected("websocket_handshake_failed"))?;
        let mut client = Self {
            socket,
            thread_id,
            cwd: cwd.to_owned(),
        };
        validate(&client.socket.get_ref().inner)?;
        let initialized = client.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "infinishell_remote_images",
                    "title": "InfiniShell",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": { "experimentalApi": true },
            }),
        )?;
        let version = initialized
            .get("userAgent")
            .and_then(Value::as_str)
            .and_then(|agent| agent.split_once('/'))
            .filter(|(product, _)| !product.trim().is_empty())
            .and_then(|(_, remainder)| remainder.split_once(' '))
            .map(|(version, _)| version);
        if version != Some(FIXED_VERSION) {
            return Err(rejected("incompatible_running_version"));
        }
        client.send_value(json!({ "method": "initialized" }))?;
        client.verify_loaded_thread()?;
        validate(&client.socket.get_ref().inner)?;
        Ok(client)
    }

    /// 消耗连接以限制一次派发；响应丢失、原生拒绝和断连均不得自动重投。
    /// before_write 需原子核对当前 lease 并持久化 Unknown；失败时不能发送队列请求。
    pub(crate) fn submit_once(
        mut self,
        client_message_id: Uuid,
        text: &str,
        images: &[CodexQueueImage],
        before_write: impl FnOnce(&CodexQueueAttempt) -> io::Result<()>,
        validate: &impl Fn(&UnixStream) -> io::Result<()>,
    ) -> io::Result<CodexQueueReceipt> {
        if client_message_id.is_nil() {
            return Err(rejected("invalid_client_message_id"));
        }
        let (input, _image_leases) = prepare_input(text, images)?;
        self.verify_loaded_thread()?;
        validate(&self.socket.get_ref().inner)?;
        let rpc_id = Uuid::new_v4();
        let request = json!({
            "id": rpc_id.to_string(),
            "method": "thread/queue/add",
            "params": {
                "threadId": self.thread_id.to_string(),
                "clientUserMessageId": client_message_id.to_string(),
                "input": input,
            },
        });
        let encoded = serde_json::to_string(&request).map_err(|_| rejected("request_encoding"))?;
        if encoded.len() > MAX_MESSAGE_BYTES {
            return Err(rejected("request_too_large"));
        }
        let attempt = CodexQueueAttempt {
            rpc_id,
            thread_id: self.thread_id,
            client_message_id,
            request_sha256: Sha256::digest(encoded.as_bytes()).into(),
        };
        before_write(&attempt)?;
        // 本次连接没有 start/resume/steer 或审批响应；最后一次核验失败也保留 Unknown。
        validate(&self.socket.get_ref().inner)?;
        self.socket.get_mut().renew_deadline();
        self.socket
            .send(Message::Text(encoded))
            .map_err(|_| rejected("queue_write_outcome_unknown"))?;
        let (response, raw_ack_sha256) = self.read_response(rpc_id)?;
        let queued = response
            .get("queuedSubmission")
            .ok_or_else(|| rejected("missing_queue_receipt"))?;
        let queued_submission_id = queued
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
            .ok_or_else(|| rejected("invalid_queue_receipt_id"))?;
        if queued.get("clientUserMessageId").and_then(Value::as_str)
            != Some(client_message_id.to_string().as_str())
        {
            return Err(rejected("queue_client_id_mismatch"));
        }
        verify_snapshots(queued.get("input"), text, images)?;
        // 同一已绑定连接的精确回执必须落盘；用户此时换 pane 或取消不能抹掉已接收事实。
        // UI 展示另外核对当前代次，不能用这里的回执向新会话补发输入。
        Ok(CodexQueueReceipt {
            attempt,
            queued_submission_id: queued_submission_id.to_owned(),
            image_sha256: images.iter().map(|image| image.sha256).collect(),
            raw_ack_sha256,
        })
    }

    fn verify_loaded_thread(&mut self) -> io::Result<()> {
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let thread_id = self.thread_id.to_string();
        let mut loaded = false;
        let mut complete = false;
        // 只读现有实例；不以 thread/read 能读取磁盘历史冒充已加载线程。
        // 固定版本的 hook 不含发起客户端身份；共享 daemon 多线程不能凭 TTY 环境猜归属。
        for _ in 0..32 {
            let page = self.request(
                "thread/loaded/list",
                json!({ "cursor": cursor, "limit": 100 }),
            )?;
            let ids = page
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| rejected("invalid_loaded_threads"))?;
            if ids.len() > 1
                || ids.iter().any(|id| id.as_str() != Some(thread_id.as_str()))
                || loaded && !ids.is_empty()
            {
                return Err(rejected("shared_daemon_thread_binding_ambiguous"));
            }
            if !ids.is_empty() {
                loaded = true;
            }
            match page.get("nextCursor") {
                Some(Value::String(next)) if !next.is_empty() && seen.insert(next.clone()) => {
                    cursor = Some(next.clone());
                }
                Some(Value::Null) => {
                    complete = true;
                    break;
                }
                _ => return Err(rejected("invalid_loaded_cursor")),
            }
        }
        if !loaded || !complete {
            return Err(rejected("thread_not_loaded_in_bound_daemon"));
        }
        let response = self.request(
            "thread/read",
            json!({ "threadId": thread_id, "includeTurns": false }),
        )?;
        let thread = response
            .get("thread")
            .ok_or_else(|| rejected("missing_thread"))?;
        if thread.get("id").and_then(Value::as_str) != Some(thread_id.as_str())
            || thread.get("cwd").and_then(Value::as_str) != self.cwd.to_str()
            || thread.get("ephemeral").and_then(Value::as_bool) != Some(false)
            || thread.get("parentThreadId") != Some(&Value::Null)
            || !matches!(
                thread.pointer("/status/type").and_then(Value::as_str),
                Some("idle" | "active")
            )
        {
            return Err(rejected("loaded_thread_binding_mismatch"));
        }
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value) -> io::Result<Value> {
        let id = Uuid::new_v4();
        self.send_value(json!({ "id": id.to_string(), "method": method, "params": params }))?;
        self.read_response(id).map(|(result, _)| result)
    }

    fn send_value(&mut self, value: Value) -> io::Result<()> {
        let encoded = serde_json::to_string(&value).map_err(|_| rejected("request_encoding"))?;
        self.socket.get_mut().renew_deadline();
        self.socket
            .send(Message::Text(encoded))
            .map_err(|_| rejected("native_write_failed"))
    }

    fn read_response(&mut self, id: Uuid) -> io::Result<(Value, [u8; 32])> {
        let id = Value::String(id.to_string());
        let mut bytes_seen = 0_usize;
        for _ in 0..MAX_READ_MESSAGES {
            let message = self
                .socket
                .read()
                .map_err(|_| rejected("native_response_unknown"))?;
            match message {
                Message::Text(raw) => {
                    bytes_seen = bytes_seen.saturating_add(raw.len());
                    if bytes_seen > MAX_MESSAGE_BYTES * 2 {
                        return Err(rejected("native_response_budget_exceeded"));
                    }
                    let mut message: Value =
                        serde_json::from_str(&raw).map_err(|_| rejected("invalid_native_json"))?;
                    // 通知和服务器请求留给原有终端处理；这里绝不代答审批。
                    if message.get("method").is_some() || message.get("id") != Some(&id) {
                        continue;
                    }
                    if message.get("error").is_some() || message.get("result").is_none() {
                        return Err(rejected("native_request_rejected"));
                    }
                    let digest = Sha256::digest(raw.as_bytes()).into();
                    return Ok((message["result"].take(), digest));
                }
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(_) => return Err(rejected("native_connection_closed")),
                Message::Binary(_) | Message::Frame(_) => {
                    return Err(rejected("invalid_native_frame"));
                }
            }
        }
        Err(rejected("native_response_limit_exceeded"))
    }
}

fn prepare_input(text: &str, images: &[CodexQueueImage]) -> io::Result<(Vec<Value>, Vec<File>)> {
    if images.is_empty() || images.len() > MAX_IMAGES {
        return Err(rejected("invalid_image_count"));
    }
    let mut input = Vec::with_capacity(images.len() + 1);
    let mut leases = Vec::with_capacity(images.len());
    let mut reply_budget = 64 * 1024;
    if !text.is_empty() {
        let value = json!({ "type": "text", "text": text, "text_elements": [] });
        reply_budget += serde_json::to_vec(&value)
            .map_err(|_| rejected("text_encoding"))?
            .len();
        input.push(value);
    }
    for image in images {
        let encoded_length = image
            .byte_len
            .checked_add(2)
            .and_then(|length| length.checked_div(3))
            .and_then(|length| length.checked_mul(4))
            .and_then(|length| usize::try_from(length).ok())
            .ok_or_else(|| rejected("image_size_overflow"))?;
        reply_budget = reply_budget.saturating_add(encoded_length);
        if reply_budget > MAX_MESSAGE_BYTES || image.byte_len < 8 || !image.path.is_absolute() {
            return Err(rejected("image_reply_budget_exceeded"));
        }
        let path = image
            .path
            .to_str()
            .ok_or_else(|| rejected("image_path_not_utf8"))?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&image.path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.len() != image.byte_len
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(rejected("image_file_identity_mismatch"));
        }
        let mut signature = [0_u8; 8];
        file.read_exact(&mut signature)?;
        if signature != *b"\x89PNG\r\n\x1a\n" {
            return Err(rejected("image_not_sealed_png"));
        }
        let mut digest = Sha256::new();
        digest.update(signature);
        let copied = io::copy(
            &mut (&mut file).take(image.byte_len - 8),
            &mut DigestWriter(&mut digest),
        )?;
        let actual: [u8; 32] = digest.finalize().into();
        if copied != image.byte_len - 8
            || actual != image.sha256
            || file.metadata()?.len() != image.byte_len
        {
            return Err(rejected("image_digest_mismatch"));
        }
        input.push(json!({ "type": "localImage", "path": path, "detail": "original" }));
        leases.push(file);
    }
    Ok((input, leases))
}

fn verify_snapshots(
    input: Option<&Value>,
    text: &str,
    images: &[CodexQueueImage],
) -> io::Result<()> {
    let input = input
        .and_then(Value::as_array)
        .ok_or_else(|| rejected("missing_native_images"))?;
    let offset = usize::from(!text.is_empty());
    if input.len() != images.len() + offset {
        return Err(rejected("native_input_count_mismatch"));
    }
    if !text.is_empty() && input[0] != json!({ "type": "text", "text": text, "text_elements": [] })
    {
        return Err(rejected("native_text_mismatch"));
    }
    for (value, image) in input[offset..].iter().zip(images) {
        if value.get("type").and_then(Value::as_str) != Some("image")
            || value.get("detail").and_then(Value::as_str) != Some("original")
            || value.get("fileId").is_some()
            || value.get("path").is_some()
        {
            return Err(rejected("native_image_not_snapshotted"));
        }
        let encoded = value
            .get("url")
            .and_then(Value::as_str)
            .and_then(|url| url.strip_prefix("data:image/png;base64,"))
            .ok_or_else(|| rejected("native_image_not_inline_png"))?;
        // 分块解码避免额外复制整批图片；原生回执包含自己的持久化快照。
        let mut decoder = base64::read::DecoderReader::new(encoded.as_bytes(), &STANDARD);
        let mut digest = Sha256::new();
        let decoded = io::copy(
            &mut (&mut decoder).take(image.byte_len + 1),
            &mut DigestWriter(&mut digest),
        )?;
        let actual: [u8; 32] = digest.finalize().into();
        if decoded != image.byte_len || actual != image.sha256 {
            return Err(rejected("native_image_snapshot_mismatch"));
        }
    }
    Ok(())
}

struct DigestWriter<'a>(&'a mut Sha256);

impl Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 每一次底层读写重新扣除绝对期限，碎片消息不能无限延长后台任务。
struct BoundedSocket {
    inner: UnixStream,
    deadline: Instant,
}

impl BoundedSocket {
    fn new(inner: UnixStream) -> Self {
        Self {
            inner,
            deadline: Instant::now() + REQUEST_TIMEOUT,
        }
    }

    fn renew_deadline(&mut self) {
        self.deadline = Instant::now() + REQUEST_TIMEOUT;
    }

    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "codex_queue_deadline"))
    }
}

impl Read for BoundedSocket {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.inner.set_read_timeout(Some(self.remaining()?))?;
        self.inner.read(bytes)
    }
}

impl Write for BoundedSocket {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.set_write_timeout(Some(self.remaining()?))?;
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.set_write_timeout(Some(self.remaining()?))?;
        self.inner.flush()
    }
}

fn rejected(code: &'static str) -> io::Error {
    // 只返回稳定内部码；原生错误正文可能含提示词、路径或认证信息。
    io::Error::other(code)
}
