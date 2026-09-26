//! 固定 Grok 普通 TUI 的显式 leader 侧车；不接管审批、不发送 PTY 按键。
//!
//! 调用者必须先绑定本地活动 PTY、原生 SessionStart 和输入 generation，且在目标或权限
//! 改变时立即废弃该绑定。本模块不发现其他 leader，不启动进程，也不恢复或重投历史输入。
//! 首次校准仅覆盖 macOS arm64、1.0.41、grok-4.7、default 权限和文本（可含文件引用）。
//! 文件仍由原生工具按原生权限读取；本模块不读取附件正文，也不替代附件有效性检查。

use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

const CLI_VERSION: &str = "1.0.41";
const MODEL_ID: &str = "grok-4.7";
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

/// 协议错误只返回稳定分类；不得把可能包含输入或认证材料的原生错误正文写入日志。
#[derive(Debug)]
pub(crate) enum GrokLeaderInputError {
    UnsupportedPlatform,
    InvalidTarget,
    IdentityChanged,
    IncompatibleNative,
    AuthenticationUnavailable,
    Protocol,
    FrameTooLarge,
    EmptyPrompt,
    DeliveryAlreadyRecorded,
    StaleBinding,
    Disconnected,
    NativeRequestFailed { code: Option<i64> },
    Persistence(io::Error),
    Io(io::Error),
}

impl From<io::Error> for GrokLeaderInputError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(crate) fn calibrated_platform() -> Result<(), GrokLeaderInputError> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok(())
    } else {
        Err(GrokLeaderInputError::UnsupportedPlatform)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum GrokLeaderDeliveryStatus {
    /// 必须在第一次写入之前持久化；即使只写出部分帧，也不得自动重投。
    Unknown,
    /// 仅精确匹配 RPC ID、sessionId 和原生 promptId 的最终响应可以到达这里。
    Finished {
        native_prompt_id: Uuid,
        outcome: GrokLeaderOutcome,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum GrokLeaderOutcome {
    EndTurn,
    Cancelled,
}

/// 与现有消息记录一起持久化；恢复读取不能成为再次提交同一消息的依据。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct GrokLeaderDelivery {
    pub binding_id: Uuid,
    pub session_id: Uuid,
    pub message_id: Uuid,
    pub rpc_id: Uuid,
    pub status: GrokLeaderDeliveryStatus,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum GrokLeaderInputEvent {
    Delivery(GrokLeaderDelivery),
    /// 仅供显示状态；侧车没有发送审批响应的接口。
    NativePermissionPending {
        tool_call_id: String,
    },
}

/// 每个侧车只发送一条消息。后续消息必须创建新侧车，并由数据库保证消息 ID 不重领。
struct DeliveryTracker {
    binding_id: Uuid,
    session_id: Uuid,
    delivery: Option<GrokLeaderDelivery>,
    final_response: Option<Value>,
}

impl DeliveryTracker {
    fn new(
        binding_id: Uuid,
        session_id: Uuid,
        restored: Option<GrokLeaderDelivery>,
    ) -> Result<Self, GrokLeaderInputError> {
        if binding_id.is_nil()
            || session_id.is_nil()
            || restored.as_ref().is_some_and(|record| {
                record.session_id != session_id
                    || record.binding_id != binding_id
                    || record.message_id.is_nil()
                    || record.rpc_id.is_nil()
            })
        {
            return Err(GrokLeaderInputError::InvalidTarget);
        }
        Ok(Self {
            binding_id,
            session_id,
            delivery: restored,
            final_response: None,
        })
    }

    fn check_binding(&self, current_binding_id: Uuid) -> Result<(), GrokLeaderInputError> {
        if self.binding_id != current_binding_id {
            return Err(GrokLeaderInputError::StaleBinding);
        }
        Ok(())
    }

    fn record_before_write(
        &mut self,
        message_id: Uuid,
        persist: impl FnOnce(&GrokLeaderDelivery) -> io::Result<()>,
    ) -> Result<&GrokLeaderDelivery, GrokLeaderInputError> {
        if self.delivery.is_some() {
            return Err(GrokLeaderInputError::DeliveryAlreadyRecorded);
        }
        if message_id.is_nil() {
            return Err(GrokLeaderInputError::InvalidTarget);
        }
        let record = GrokLeaderDelivery {
            binding_id: self.binding_id,
            session_id: self.session_id,
            message_id,
            rpc_id: Uuid::new_v4(),
            status: GrokLeaderDeliveryStatus::Unknown,
        };
        // 持久化回调必须原子校验当前 generation 和消息尚未领取，再保存 Unknown。
        // 回调失败时不会写 socket；若调用者无法判断事务结果，必须先查询数据库。
        persist(&record).map_err(GrokLeaderInputError::Persistence)?;
        self.delivery = Some(record);
        Ok(self.delivery.as_ref().expect("刚保存的消息必须存在"))
    }

    fn observe(
        &mut self,
        rpc: &Value,
    ) -> Result<Option<GrokLeaderInputEvent>, GrokLeaderInputError> {
        if rpc.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(GrokLeaderInputError::Protocol);
        }
        if let Some(method) = rpc.get("method").and_then(Value::as_str) {
            // 队列、文本 echo 和 prompt_complete 通知都没有调用方 RPC ID，不能作为投递 ACK。
            if method == "session/request_permission"
                && rpc.pointer("/params/sessionId").and_then(Value::as_str)
                    == Some(self.session_id.to_string().as_str())
            {
                let tool_call_id = rpc
                    .pointer("/params/toolCall/toolCallId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or(GrokLeaderInputError::Protocol)?;
                return Ok(Some(GrokLeaderInputEvent::NativePermissionPending {
                    tool_call_id: tool_call_id.to_owned(),
                }));
            }
            return Ok(None);
        }
        let Some(record) = self.delivery.as_mut() else {
            return Ok(None);
        };
        if rpc.get("id").and_then(Value::as_str) != Some(record.rpc_id.to_string().as_str()) {
            return Ok(None);
        }
        if rpc.get("error").is_some() {
            return Err(GrokLeaderInputError::NativeRequestFailed {
                code: rpc.pointer("/error/code").and_then(Value::as_i64),
            });
        }
        let result = rpc.get("result").ok_or(GrokLeaderInputError::Protocol)?;
        let meta = result.get("_meta").ok_or(GrokLeaderInputError::Protocol)?;
        let native_prompt_id = meta
            .get("promptId")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .filter(|id| !id.is_nil())
            .ok_or(GrokLeaderInputError::Protocol)?;
        if meta.get("sessionId").and_then(Value::as_str)
            != Some(record.session_id.to_string().as_str())
            || meta.get("requestId").and_then(Value::as_str)
                != Some(native_prompt_id.to_string().as_str())
            || meta.get("modelId").and_then(Value::as_str) != Some(MODEL_ID)
        {
            return Err(GrokLeaderInputError::Protocol);
        }
        let outcome = match result.get("stopReason").and_then(Value::as_str) {
            Some("end_turn") => GrokLeaderOutcome::EndTurn,
            Some("cancelled") => GrokLeaderOutcome::Cancelled,
            Some(_) | None => return Err(GrokLeaderInputError::IncompatibleNative),
        };
        let status = GrokLeaderDeliveryStatus::Finished {
            native_prompt_id,
            outcome,
        };
        match &record.status {
            GrokLeaderDeliveryStatus::Unknown => record.status = status,
            GrokLeaderDeliveryStatus::Finished { .. } => {
                // 重复相同终态可以忽略；同 RPC ID 的第二个 promptId 是重复执行负例。
                if record.status != status {
                    return Err(GrokLeaderInputError::Protocol);
                }
                return Ok(None);
            }
        }
        self.final_response = Some(rpc.clone());
        Ok(Some(GrokLeaderInputEvent::Delivery(record.clone())))
    }
}

fn acp_frame(rpc: Value) -> Value {
    json!({"type":"acp", "payload":rpc.to_string()})
}

fn encode_frame(value: &Value) -> Result<Vec<u8>, GrokLeaderInputError> {
    let payload = serde_json::to_vec(value).map_err(|_| GrokLeaderInputError::Protocol)?;
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(GrokLeaderInputError::FrameTooLarge);
    }
    let mut bytes = Vec::with_capacity(payload.len() + 4);
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

#[derive(Default)]
struct FrameDecoder {
    bytes: Vec<u8>,
}

impl FrameDecoder {
    fn append(&mut self, bytes: &[u8]) -> Result<(), GrokLeaderInputError> {
        if self.bytes.len().saturating_add(bytes.len()) > MAX_FRAME_BYTES + 4 + 8192 {
            return Err(GrokLeaderInputError::FrameTooLarge);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn next(&mut self) -> Result<Option<Value>, GrokLeaderInputError> {
        let Some(prefix) = self.bytes.get(..4) else {
            return Ok(None);
        };
        let length = u32::from_be_bytes(prefix.try_into().expect("帧头必须为四字节")) as usize;
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(GrokLeaderInputError::FrameTooLarge);
        }
        if self.bytes.len() < length + 4 {
            return Ok(None);
        }
        let frame = serde_json::from_slice(&self.bytes[4..length + 4])
            .map_err(|_| GrokLeaderInputError::Protocol)?;
        self.bytes.drain(..length + 4);
        Ok(Some(frame))
    }
}

fn decode_acp(frame: &Value) -> Result<Value, GrokLeaderInputError> {
    let payload = frame
        .get("payload")
        .and_then(Value::as_str)
        .ok_or(GrokLeaderInputError::Protocol)?;
    serde_json::from_str(payload).map_err(|_| GrokLeaderInputError::Protocol)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(crate) use native::{GrokLeaderInput, GrokLeaderTarget};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native {
    use std::ffi::OsString;
    use std::fs::{self, File};
    use std::io::{Read as _, Write as _};
    use std::net::Shutdown;
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use command::managed::{
        MacosProcessIdentity, macos_boot_session, macos_peer_identity, macos_process_identity,
    };
    use sha2::{Digest as _, Sha256};
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    use super::*;

    const FIXED_BINARY_SHA256: &str =
        "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d";
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

    /// 必须来自同一活动 PTY 的启动记录和 SessionStart，不能用 leader 枚举结果拼装。
    /// 首增量严格匹配已经实测的显式启动参数；未来新增模式需要独立原生校准。
    pub(crate) struct GrokLeaderTarget {
        binding_id: Uuid,
        session_id: Uuid,
        cwd: PathBuf,
        socket_path: PathBuf,
        executable: PathBuf,
        boot_session: String,
        tui: MacosProcessIdentity,
        leader: MacosProcessIdentity,
        socket_identity: (u64, u64),
    }

    impl GrokLeaderTarget {
        pub(crate) fn capture(
            binding_id: Uuid,
            session_id: Uuid,
            cwd: &Path,
            socket_path: &Path,
            executable: &Path,
            tui_pid: i32,
            leader_pid: i32,
            native_permission_mode: &str,
        ) -> Result<Self, GrokLeaderInputError> {
            if binding_id.is_nil()
                || session_id.is_nil()
                || tui_pid <= 0
                || leader_pid <= 0
                || tui_pid == leader_pid
                || native_permission_mode != "default"
                || !cwd.is_absolute()
                || !socket_path.is_absolute()
                || !executable.is_absolute()
            {
                return Err(GrokLeaderInputError::InvalidTarget);
            }
            let cwd = cwd.canonicalize()?;
            if !cwd.is_dir() {
                return Err(GrokLeaderInputError::InvalidTarget);
            }
            let executable = executable.canonicalize()?;
            let mut binary = File::open(&executable)?;
            let mut digest = Sha256::new();
            let mut chunk = [0u8; 65536];
            loop {
                let length = binary.read(&mut chunk)?;
                if length == 0 {
                    break;
                }
                digest.update(&chunk[..length]);
            }
            if format!("{:x}", digest.finalize()) != FIXED_BINARY_SHA256 {
                return Err(GrokLeaderInputError::IncompatibleNative);
            }
            let target = Self {
                binding_id,
                session_id,
                cwd,
                socket_path: socket_path.to_owned(),
                executable,
                boot_session: macos_boot_session()?,
                tui: macos_process_identity(tui_pid)?,
                leader: macos_process_identity(leader_pid)?,
                socket_identity: socket_identity(socket_path)?,
            };
            target.validate()?;
            Ok(target)
        }

        fn validate(&self) -> Result<(), GrokLeaderInputError> {
            if macos_boot_session()? != self.boot_session
                || macos_process_identity(self.tui.pid)? != self.tui
                || macos_process_identity(self.leader.pid)? != self.leader
                || socket_identity(&self.socket_path)? != self.socket_identity
                || self.cwd.canonicalize()? != self.cwd
            {
                return Err(GrokLeaderInputError::IdentityChanged);
            }
            let pids = [
                Pid::from_u32(self.tui.pid as u32),
                Pid::from_u32(self.leader.pid as u32),
            ];
            let mut system = System::new();
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&pids),
                true,
                ProcessRefreshKind::nothing()
                    .with_exe(UpdateKind::Always)
                    .with_cmd(UpdateKind::Always),
            );
            for pid in pids {
                let process = system
                    .process(pid)
                    .ok_or(GrokLeaderInputError::IdentityChanged)?;
                let path = process.exe().ok_or(GrokLeaderInputError::IdentityChanged)?;
                if path.canonicalize()? != self.executable {
                    return Err(GrokLeaderInputError::IdentityChanged);
                }
            }
            let expected: Vec<OsString> = vec![
                "--leader".into(),
                "--minimal".into(),
                "--no-alt-screen".into(),
                "--cwd".into(),
                self.cwd.as_os_str().to_owned(),
                "--leader-socket".into(),
                self.socket_path.as_os_str().to_owned(),
                "--session-id".into(),
                self.session_id.to_string().into(),
                "--model".into(),
                MODEL_ID.into(),
            ];
            let tui = system
                .process(pids[0])
                .ok_or(GrokLeaderInputError::IdentityChanged)?;
            if tui.cmd().get(1..) != Some(expected.as_slice()) {
                return Err(GrokLeaderInputError::InvalidTarget);
            }
            Ok(())
        }
    }

    fn socket_identity(path: &Path) -> Result<(u64, u64), GrokLeaderInputError> {
        let parent = path.parent().ok_or(GrokLeaderInputError::InvalidTarget)?;
        let directory = fs::symlink_metadata(parent)?;
        let socket = fs::symlink_metadata(path)?;
        let uid = unsafe { libc::geteuid() };
        // 原生 socket 本身为 0755；独占父目录 0700 和内核 peer 凭据共同保护访问边界。
        if parent.canonicalize()? != parent
            || !directory.is_dir()
            || directory.uid() != uid
            || directory.mode() & 0o077 != 0
            || !socket.file_type().is_socket()
            || socket.uid() != uid
            || socket.mode() & 0o022 != 0
        {
            return Err(GrokLeaderInputError::InvalidTarget);
        }
        Ok((socket.dev(), socket.ino()))
    }

    /// 同步且有界的侧车，必须在后台工作线程调用；不要持有 TerminalModel 锁。
    /// 析构、错误及 disconnect 仅关闭本连接，绝不终止 TUI 或 leader 进程。
    pub(crate) struct GrokLeaderInput {
        target: GrokLeaderTarget,
        tracker: DeliveryTracker,
        connection: Option<UnixStream>,
        decoder: FrameDecoder,
    }

    impl GrokLeaderInput {
        pub(crate) fn connect(
            target: GrokLeaderTarget,
            restored: Option<GrokLeaderDelivery>,
        ) -> Result<Self, GrokLeaderInputError> {
            target.validate()?;
            let connection = UnixStream::connect(&target.socket_path)?;
            connection.set_write_timeout(Some(Duration::from_secs(2)))?;
            if macos_peer_identity(&connection)? != target.leader {
                return Err(GrokLeaderInputError::IdentityChanged);
            }
            target.validate()?;
            let tracker = DeliveryTracker::new(target.binding_id, target.session_id, restored)?;
            let mut bridge = Self {
                target,
                tracker,
                connection: Some(connection),
                decoder: FrameDecoder::default(),
            };
            bridge.handshake()?;
            Ok(bridge)
        }

        pub(crate) fn delivery(&self) -> Option<&GrokLeaderDelivery> {
            self.tracker.delivery.as_ref()
        }

        /// persist 必须原子拒绝已领取的 message_id；一次写入尝试后所有错误仍保留 Unknown。
        /// 相同原生 RPC ID 并不幂等，不提供重试接口。调用方不得用新侧车绕过持久化去重。
        pub(crate) fn submit_once(
            &mut self,
            current_binding_id: Uuid,
            message_id: Uuid,
            text: &str,
            persist: impl FnOnce(&GrokLeaderDelivery) -> io::Result<()>,
        ) -> Result<(), GrokLeaderInputError> {
            self.submit_once_checked(current_binding_id, message_id, text, persist, || Ok(()))
        }

        /// 当前输入与权限 lease 的最后领取必须在 SQLite Unknown 落盘之后、首个 socket 写入之前。
        pub(crate) fn submit_once_checked(
            &mut self,
            current_binding_id: Uuid,
            message_id: Uuid,
            text: &str,
            persist: impl FnOnce(&GrokLeaderDelivery) -> io::Result<()>,
            authorize_write: impl FnOnce() -> Result<(), GrokLeaderInputError>,
        ) -> Result<(), GrokLeaderInputError> {
            self.tracker.check_binding(current_binding_id)?;
            if text.trim().is_empty() {
                return Err(GrokLeaderInputError::EmptyPrompt);
            }
            if text.len() > MAX_PROMPT_BYTES {
                return Err(GrokLeaderInputError::FrameTooLarge);
            }
            self.validate_connection()?;
            let record = self.tracker.record_before_write(message_id, persist)?;
            let frame = acp_frame(json!({
                "jsonrpc":"2.0", "id":record.rpc_id.to_string(), "method":"session/prompt",
                "params":{"sessionId":record.session_id.to_string(),
                    "prompt":[{"type":"text","text":text}]}
            }));
            // 持久化可能耗时，再核对进程；即便此时失败，也保留已经领取的 Unknown。
            let result = self
                .validate_connection()
                .and_then(|()| authorize_write())
                .and_then(|()| self.write_frame(&frame));
            if result.is_err() {
                self.close_connection();
            }
            result
        }

        /// 无事件或半帧时返回 None；半帧会跨 poll 保留，超时从不意味着输入就绪。
        pub(crate) fn poll(
            &mut self,
            current_binding_id: Uuid,
        ) -> Result<Option<GrokLeaderInputEvent>, GrokLeaderInputError> {
            let result = self
                .tracker
                .check_binding(current_binding_id)
                .and_then(|()| self.validate_connection())
                .and_then(|()| self.read_frame(Duration::from_millis(100)))
                .and_then(|frame| match frame {
                    Some(frame) if frame.get("type").and_then(Value::as_str) == Some("acp") => {
                        self.tracker.observe(&decode_acp(&frame)?)
                    }
                    Some(frame) if frame.get("type").and_then(Value::as_str) == Some("error") => {
                        Err(GrokLeaderInputError::Protocol)
                    }
                    Some(_) | None => Ok(None),
                });
            if result.is_err() {
                self.close_connection();
            }
            result
        }

        /// 原始最终 RPC 仅供持久化校验，不依据解析后的状态重建或合成 ACK。
        pub(crate) fn take_final_response(&mut self) -> Option<Value> {
            self.tracker.final_response.take()
        }

        pub(crate) fn disconnect(mut self) -> Option<GrokLeaderDelivery> {
            self.close_connection();
            self.tracker.delivery.take()
        }

        fn close_connection(&mut self) {
            if let Some(connection) = self.connection.take() {
                let _ = connection.shutdown(Shutdown::Both);
            }
        }

        fn validate_connection(&self) -> Result<(), GrokLeaderInputError> {
            let connection = self
                .connection
                .as_ref()
                .ok_or(GrokLeaderInputError::Disconnected)?;
            self.target.validate()?;
            if macos_peer_identity(connection)? != self.target.leader {
                return Err(GrokLeaderInputError::IdentityChanged);
            }
            Ok(())
        }

        fn write_frame(&mut self, frame: &Value) -> Result<(), GrokLeaderInputError> {
            let bytes = encode_frame(frame)?;
            self.connection
                .as_mut()
                .ok_or(GrokLeaderInputError::Disconnected)?
                .write_all(&bytes)?;
            Ok(())
        }

        fn read_frame(&mut self, timeout: Duration) -> Result<Option<Value>, GrokLeaderInputError> {
            if let Some(frame) = self.decoder.next()? {
                return Ok(Some(frame));
            }
            let connection = self
                .connection
                .as_mut()
                .ok_or(GrokLeaderInputError::Disconnected)?;
            connection.set_read_timeout(Some(timeout))?;
            let mut chunk = [0u8; 8192];
            match connection.read(&mut chunk) {
                Ok(0) => Err(GrokLeaderInputError::Disconnected),
                Ok(length) => {
                    self.decoder.append(&chunk[..length])?;
                    self.decoder.next()
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    Ok(None)
                }
                Err(error) => Err(error.into()),
            }
        }

        fn wait_for(
            &mut self,
            deadline: Instant,
            matches: impl Fn(&Value) -> bool,
        ) -> Result<Value, GrokLeaderInputError> {
            for _ in 0..4096 {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .filter(|duration| !duration.is_zero())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "原生 leader 握手超时")
                    })?;
                if let Some(frame) = self.read_frame(remaining.min(Duration::from_millis(100)))? {
                    if matches(&frame) {
                        return Ok(frame);
                    }
                    if frame.get("type").and_then(Value::as_str) == Some("error") {
                        return Err(GrokLeaderInputError::Protocol);
                    }
                    if frame.get("type").and_then(Value::as_str) == Some("acp") {
                        // 若恢复观察时到达精确关联的旧终态，保留到 delivery() 供调用方落库。
                        self.tracker.observe(&decode_acp(&frame)?)?;
                    }
                    // 历史回放和待审批请求留给原生 TUI；握手期间绝不发送审批回复。
                }
            }
            Err(GrokLeaderInputError::Protocol)
        }

        fn handshake(&mut self) -> Result<(), GrokLeaderInputError> {
            let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
            self.write_frame(
                &json!({"type":"register", "client_type":"infinishell-readonly-probe",
                "mode":"stdio", "capabilities":{"client_version":CLI_VERSION,
                    "yolo_mode":false,"auto_mode":false,"terminal":false,
                    "fs_read":false,"fs_write":false,"user_message_echo":true}}),
            )?;
            let registered = self.wait_for(deadline, |frame| frame["type"] == "registered")?;
            if registered["ready"] != true
                || registered["leader_protocol_version"] != 1
                || registered["leader_binary_version"] != CLI_VERSION
                || registered["leader_capabilities"]["control_v1"] != true
            {
                return Err(GrokLeaderInputError::IncompatibleNative);
            }
            let info_id = Uuid::new_v4().to_string();
            self.write_frame(&json!({"type":"control","request_id":info_id,
                "command":{"type":"get_leader_info"}}))?;
            let info = self.wait_for(deadline, |frame| {
                frame["type"] == "control_result" && frame["request_id"] == info_id
            })?;
            let native = &info["result"]["Ok"];
            if native["type"] != "leader_info"
                || native["pid"] != self.target.leader.pid
                || native["leader_protocol_version"] != 1
                || native["leader_binary_version"] != CLI_VERSION
                || native["socket_path"].as_str() != self.target.socket_path.to_str()
            {
                return Err(GrokLeaderInputError::IdentityChanged);
            }
            let initialized = self.rpc_readonly(
                deadline,
                "initialize",
                json!({
                    "protocolVersion":1,"clientCapabilities":{
                        "fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},
                    "clientInfo":{"name":"infinishell-readonly-probe","version":"0.1"}
                }),
            )?;
            if initialized["protocolVersion"] != 1
                || initialized["agentCapabilities"]["loadSession"] != true
                || initialized["_meta"]["agentVersion"] != CLI_VERSION
                || initialized["_meta"]["currentWorkingDirectory"].as_str()
                    != self.target.cwd.to_str()
                || initialized["_meta"]["modelState"]["currentModelId"] != MODEL_ID
            {
                return Err(GrokLeaderInputError::IncompatibleNative);
            }
            if initialized["_meta"]["defaultAuthMethodId"] != "cached_token"
                || !initialized["authMethods"]
                    .as_array()
                    .is_some_and(|methods| {
                        methods.iter().any(|method| method["id"] == "cached_token")
                    })
            {
                return Err(GrokLeaderInputError::AuthenticationUnavailable);
            }
            let loaded = self.rpc_readonly(
                deadline,
                "session/load",
                json!({
                    "sessionId":self.target.session_id.to_string(), "cwd":self.target.cwd,
                    "mcpServers":[]
                }),
            )?;
            let meta = &loaded["_meta"];
            let detail = &meta["x.ai/sessionDetail"];
            let session_id = self.target.session_id.to_string();
            if meta["sessionId"] != session_id
                || detail["sessionId"] != session_id
                || detail["kind"] != "build"
                || detail["currentModelId"] != MODEL_ID
                || detail["cwd"].as_str() != self.target.cwd.to_str()
            {
                return Err(GrokLeaderInputError::IdentityChanged);
            }
            self.validate_connection()
        }

        fn rpc_readonly(
            &mut self,
            deadline: Instant,
            method: &str,
            params: Value,
        ) -> Result<Value, GrokLeaderInputError> {
            let id = Uuid::new_v4().to_string();
            self.write_frame(&acp_frame(
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            ))?;
            let frame = self.wait_for(deadline, |frame| {
                frame["type"] == "acp" && decode_acp(frame).is_ok_and(|rpc| rpc["id"] == id)
            })?;
            let rpc = decode_acp(&frame)?;
            if rpc["jsonrpc"] != "2.0" || rpc.get("error").is_some() {
                return Err(GrokLeaderInputError::Protocol);
            }
            rpc.get("result")
                .cloned()
                .ok_or(GrokLeaderInputError::Protocol)
        }
    }
}

#[cfg(test)]
#[path = "grok_leader_input_tests.rs"]
mod tests;
