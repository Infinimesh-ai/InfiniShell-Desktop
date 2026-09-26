//! Windows 固定 Grok 的显式 leader 侧车；只连接应用所拥有的会话。
//! 管道映射依据官方源码 75810042ca2762aa0b0fa17864f3f68823ccbea5。
//! 固定 1.0.41 的 Windows 字节已单独绑定，映射和完整原生链仍待 Windows 验收。

use std::ffi::OsString;
use std::fs::{self, File};
use std::hash::{Hash as _, Hasher as _};
use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use command::managed::{WindowsProcessIdentity, WindowsProcessLease};
use sha2::{Digest as _, Sha256};
use siphasher::sip::SipHasher13;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::runtime::{Builder, Runtime};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_READ};

use super::*;
use crate::terminal::cli_agent_sessions::grok_owned_history_source_windows::NpmGrokSource;

const FIXED_BINARY_SHA256: &str =
    "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 路径哈希只定位管道；连接后必须另核内核进程句柄、登录会话和原生协议。
pub(crate) fn pipe_name(path: &Path) -> OsString {
    let mut hasher = SipHasher13::new_with_keys(0x67726f6b_6c656164, 0x65725f70_69706521);
    path.hash(&mut hasher);
    let hash = hasher.finish();
    OsString::from(format!(r"\\.\pipe\grok-leader-{hash:016x}"))
}

pub(crate) struct GrokLeaderTarget {
    binding_id: Uuid,
    session_id: Uuid,
    cwd: PathBuf,
    socket_path: PathBuf,
    executable: PathBuf,
    tui: WindowsProcessLease,
    leader: WindowsProcessLease,
    // 禁止此进程存活期间替换或写入已核验的 CLI 映像。
    binary: File,
    // 普通npm启动保留完整公开资源的只读租约。
    _source_files: Vec<File>,
}

impl GrokLeaderTarget {
    /// 身份由当前 ConPTY 启动收据提供；不接受进程枚举推断出的会话。
    pub(crate) fn capture(
        binding_id: Uuid,
        session_id: Uuid,
        cwd: &Path,
        socket_path: &Path,
        executable: &Path,
        tui_identity: WindowsProcessIdentity,
        leader_identity: WindowsProcessIdentity,
        native_permission_mode: &str,
    ) -> Result<Self, GrokLeaderInputError> {
        if binding_id.is_nil()
            || session_id.is_nil()
            || tui_identity.pid == leader_identity.pid
            || native_permission_mode != "default"
            || !cwd.is_absolute()
            || !socket_path.is_absolute()
            || !executable.is_absolute()
            || socket_path.to_str().is_none()
        {
            return Err(GrokLeaderInputError::InvalidTarget);
        }
        let tui = WindowsProcessLease::capture(tui_identity.pid)?;
        let leader = WindowsProcessLease::capture(leader_identity.pid)?;
        if tui.identity() != tui_identity || leader.identity() != leader_identity {
            return Err(GrokLeaderInputError::IdentityChanged);
        }
        let cwd = dunce::canonicalize(cwd)?;
        let executable = executable.canonicalize()?;
        if !cwd.is_dir()
            || !plain_path(&executable)?
            || !plain_path(
                socket_path
                    .parent()
                    .ok_or(GrokLeaderInputError::InvalidTarget)?,
            )?
        {
            return Err(GrokLeaderInputError::InvalidTarget);
        }
        let mut binary = File::options()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(&executable)?;
        let mut digest = Sha256::new();
        let mut chunk = [0u8; 65536];
        loop {
            let length = std::io::Read::read(&mut binary, &mut chunk)?;
            if length == 0 {
                break;
            }
            digest.update(&chunk[..length]);
        }
        let observed = format!("{:x}", digest.finalize());
        let source_files = if observed == "6db593f5aeb1e12b1f4f36fac7fad34c2dfe8f9fcb853bff5fc01928c95c53fa" {
            NpmGrokSource::capture(&executable)?.ok_or(GrokLeaderInputError::IncompatibleNative)?.validate_and_hold()?
        } else { Vec::new() };
        if !binary.metadata()?.is_file()
            || observed != FIXED_BINARY_SHA256 && source_files.is_empty()
        {
            return Err(GrokLeaderInputError::IncompatibleNative);
        }
        let target = Self {
            binding_id,
            session_id,
            cwd,
            socket_path: socket_path.to_owned(),
            executable,
            tui,
            leader,
            binary,
            _source_files: source_files,
        };
        target.validate()?;
        Ok(target)
    }

    fn validate(&self) -> Result<(), GrokLeaderInputError> {
        self.tui.validate()?;
        self.leader.validate()?;
        if dunce::canonicalize(&self.cwd)? != self.cwd
            || !self.binary.metadata()?.is_file()
            || self.tui.image_path().canonicalize()? != self.executable
            || self.leader.image_path().canonicalize()? != self.executable
        {
            return Err(GrokLeaderInputError::IdentityChanged);
        }
        let pid = Pid::from_u32(self.tui.identity().pid);
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
        );
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
        if system.process(pid).and_then(|p| p.cmd().get(1..)) != Some(expected.as_slice()) && system.process(pid).and_then(|p| p.cmd().get(1..)) != Some(resumed.as_slice()) {
            return Err(GrokLeaderInputError::InvalidTarget);
        }
        self.tui.validate()?;
        self.leader.validate()?;
        Ok(())
    }
}

fn plain_path(path: &Path) -> io::Result<bool> {
    for component in path.ancestors() {
        if fs::symlink_metadata(component)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 同步且有界的侧车，必须在后台工作线程调用；不要持有 TerminalModel 锁。
/// 析构、错误及 disconnect 仅关闭本连接，绝不终止 TUI 或 leader 进程。
pub(crate) struct GrokLeaderInput {
    target: GrokLeaderTarget,
    tracker: DeliveryTracker,
    connection: Option<NamedPipeClient>,
    runtime: Runtime,
    decoder: FrameDecoder,
}

impl GrokLeaderInput {
    pub(crate) fn connect(
        target: GrokLeaderTarget,
        restored: Option<GrokLeaderDelivery>,
    ) -> Result<Self, GrokLeaderInputError> {
        target.validate()?;
        // 每个同步 worker 独占一个 reactor；不借用 GUI runtime，不阻塞 TerminalModel。
        let runtime = Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;
        let connection = {
            let _entered = runtime.enter();
            // Tokio 默认 SECURITY_IDENTIFICATION，服务端不能借此模拟本进程权限。
            ClientOptions::new().open(pipe_name(&target.socket_path))?
        };
        target.leader.validate_named_pipe_peer(&connection)?;
        target.validate()?;
        let tracker = DeliveryTracker::new(target.binding_id, target.session_id, restored)?;
        let mut bridge = Self {
            target,
            tracker,
            connection: Some(connection),
            runtime,
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
        self.connection.take();
    }

    fn validate_connection(&self) -> Result<(), GrokLeaderInputError> {
        let connection = self
            .connection
            .as_ref()
            .ok_or(GrokLeaderInputError::Disconnected)?;
        self.target.validate()?;
        self.target.leader.validate_named_pipe_peer(connection)?;
        Ok(())
    }

    fn write_frame(&mut self, frame: &Value) -> Result<(), GrokLeaderInputError> {
        let bytes = encode_frame(frame)?;
        let connection = self
            .connection
            .as_mut()
            .ok_or(GrokLeaderInputError::Disconnected)?;
        // 超时可能发生在部分写入之后，调用者保留 SQLite Unknown，绝不重投。
        self.runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(2), connection.write_all(&bytes)).await
            })
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "原生命名管道写入超时"))??;
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
        let mut chunk = [0u8; 8192];
        // 单次 read 可以取消；已收到的半帧始终保留在 decoder，不把超时当作输入就绪。
        let result = self
            .runtime
            .block_on(async { tokio::time::timeout(timeout, connection.read(&mut chunk)).await });
        match result {
            Ok(Ok(0)) => Err(GrokLeaderInputError::Disconnected),
            Ok(Ok(length)) => {
                self.decoder.append(&chunk[..length])?;
                self.decoder.next()
            }
            Err(_) => Ok(None),
            Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => Ok(None),
            Ok(Err(error)) => Err(error.into()),
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
            || native["pid"] != self.target.leader.identity().pid
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
            || !initialized["_meta"]["currentWorkingDirectory"]
                .as_str()
                .is_some_and(|cwd| dunce::canonicalize(cwd).is_ok_and(|cwd| cwd == self.target.cwd))
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
            || !detail["cwd"]
                .as_str()
                .is_some_and(|cwd| dunce::canonicalize(cwd).is_ok_and(|cwd| cwd == self.target.cwd))
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
