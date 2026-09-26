//! Linux x86_64 固定 Grok 1.0.41 后端；内核身份不满足时拒绝，不回退到裸 PID。

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read as _, Write as _};
use std::net::Shutdown;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use command::managed::{
    LinuxProcessHandle, LinuxProcessIdentity, linux_boot_session, linux_peer_identity,
    linux_verify_identity_support,
};
use sha2::{Digest as _, Sha256};

use super::*;

const FIXED_BINARY_SHA256: &str =
    "9ce03ed23e16ea01072b4496263d6213a27899e1e3e107f008d36edf82e70407";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn platform_available() -> Result<(), GrokLeaderInputError> {
    linux_verify_identity_support().map_err(GrokLeaderInputError::Io)
}

/// 必须来自同一活动 PTY 的启动记录和 SessionStart，不能用 leader 枚举结果拼装。
/// 仅接受固定版本的显式启动参数，连接后仍逐项核对 Linux 原生握手。
pub(crate) struct GrokLeaderTarget {
    binding_id: Uuid,
    session_id: Uuid,
    cwd: PathBuf,
    socket_path: PathBuf,
    executable: PathBuf,
    boot_session: String,
    tui: LinuxProcessIdentity,
    leader: LinuxProcessIdentity,
    socket_identity: (u64, u64),
    tty_device: u64,
    tui_handle: LinuxProcessHandle,
    leader_handle: LinuxProcessHandle,
    binary: File,
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
        let tui_handle = LinuxProcessHandle::capture(tui_pid)?;
        let leader_handle = LinuxProcessHandle::capture(leader_pid)?;
        let tui = tui_handle.snapshot()?;
        if tui.tty_device == 0 {
            return Err(GrokLeaderInputError::InvalidTarget);
        }
        let target = Self {
            binding_id,
            session_id,
            cwd,
            socket_path: socket_path.to_owned(),
            executable,
            boot_session: linux_boot_session()?,
            tui: tui.identity,
            leader: leader_handle.identity(),
            tty_device: tui.tty_device,
            tui_handle,
            leader_handle,
            binary,
            socket_identity: socket_identity(socket_path)?,
        };
        target.validate()?;
        Ok(target)
    }

    /// 启动收据和 socket 对端先独立取样；重新取得 pidfd 后必须仍是同一进程和 PTY。
    pub(in crate::terminal::cli_agent_sessions) fn verify_owned(
        &self,
        tui: LinuxProcessIdentity,
        leader: LinuxProcessIdentity,
        tty_device: u64,
    ) -> Result<(), GrokLeaderInputError> {
        if self.tui != tui || self.leader != leader || self.tty_device != tty_device {
            return Err(GrokLeaderInputError::IdentityChanged);
        }
        self.validate()
    }

    fn validate(&self) -> Result<(), GrokLeaderInputError> {
        let tui = self.tui_handle.snapshot()?;
        let leader = self.leader_handle.snapshot()?;
        let image = self.binary.metadata()?;
        if linux_boot_session()? != self.boot_session
            || tui.identity != self.tui
            || leader.identity != self.leader
            || socket_identity(&self.socket_path)? != self.socket_identity
            || self.cwd.canonicalize()? != self.cwd
            || tui.executable != self.executable
            || leader.executable != self.executable
            || tui.tty_device != self.tty_device
            || tui.process_group <= 0
            || tui.foreground_group != tui.process_group
            || [self.tui, self.leader].iter().any(|identity| {
                identity.executable_device != image.dev()
                    || identity.executable_inode != image.ino()
            })
        {
            return Err(GrokLeaderInputError::IdentityChanged);
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
        let mut resumed = expected.clone();
        resumed[7] = "--resume".into();
        if tui.arguments.get(1..) != Some(expected.as_slice()) && tui.arguments.get(1..) != Some(resumed.as_slice()) {
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
    // socket 模式不放宽独占父目录；身份由 SO_PEERPIDFD 和原生握手共同核对。
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
        if linux_peer_identity(&connection)? != target.leader {
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
        let prompt = GrokLeaderPrompt::text(text)?;
        self.submit_prompt_once_checked(
            current_binding_id,
            message_id,
            &prompt,
            persist,
            authorize_write,
        )
    }

    /// 图片与文本共用同一次领取和原生终态回执，不通过剪贴板或 PTY 模拟图片粘贴。
    pub(crate) fn submit_prompt_once_checked(
        &mut self,
        current_binding_id: Uuid,
        message_id: Uuid,
        prompt: &GrokLeaderPrompt,
        persist: impl FnOnce(&GrokLeaderDelivery) -> io::Result<()>,
        authorize_write: impl FnOnce() -> Result<(), GrokLeaderInputError>,
    ) -> Result<(), GrokLeaderInputError> {
        self.tracker.check_binding(current_binding_id)?;
        prompt.validate_budget()?;
        self.validate_connection()?;
        let record = self.tracker.record_before_write(message_id, persist)?;
        let frame = prompt.frame(record.rpc_id, record.session_id);
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
        if linux_peer_identity(connection)? != self.target.leader {
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
                .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "原生 leader 握手超时"))?;
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
            || initialized["_meta"]["currentWorkingDirectory"].as_str() != self.target.cwd.to_str()
            || initialized["_meta"]["modelState"]["currentModelId"] != MODEL_ID
        {
            return Err(GrokLeaderInputError::IncompatibleNative);
        }
        if initialized["_meta"]["defaultAuthMethodId"] != "cached_token"
            || !initialized["authMethods"]
                .as_array()
                .is_some_and(|methods| methods.iter().any(|method| method["id"] == "cached_token"))
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
