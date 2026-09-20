//! 固定 Grok 的无模型接口调查；只复用生产进程监督，不建立权限上限。

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{ReportedModels, decode_metadata};
use crate::ai::cli_agent_runtime::managed_process::{self, ExitReason, ExitReceipt};

type ProbeResult<T> = Result<T, &'static str>;

const SCOPE: &str = "grok_fixed_policy_interface_preflight";
const FIXED_VERSION: &str = "grok 1.0.30 (04b7ffed98c6)";
const FIXED_BINARY_SHA256: &str =
    "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb";
const MAX_REQUESTS: usize = 14;
const MAX_PROCESSES: usize = 2;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_FRAMES: usize = 256;
const MAX_FILE_BYTES: usize = 65536;
// ACP 扩展在线上传输时带一个下划线；SDK 去除后才交给 x.ai/… handler。
const DIAGNOSTIC_METHODS: [&str; 4] = [
    "_x.ai/session/info",
    "_x.ai/session/state",
    "_x.ai/mcp/list",
    "_x.ai/debug/agent",
];
const ALLOWED_METHODS: [&str; 8] = [
    "initialize",
    "authenticate",
    "session/new",
    "session/load",
    "_x.ai/session/info",
    "_x.ai/session/state",
    "_x.ai/mcp/list",
    "_x.ai/debug/agent",
];

fn check(condition: bool, reason: &'static str) -> ProbeResult<()> {
    if condition { Ok(()) } else { Err(reason) }
}

fn hash(bytes: &[u8]) -> String {
    let checksum = Sha256::digest(bytes);
    format!("{checksum:x}")
}

fn encoded(value: &Value) -> ProbeResult<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| "json_encode_failed")
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_session_id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u8,
    scope: String,
    max_native_inputs: usize,
    max_protocol_requests: usize,
    max_session_processes: usize,
    allowed_methods: Vec<String>,
    diagnostic_methods: Vec<String>,
    profile_sha256: String,
    config_sha256: String,
    always_approve_requested: bool,
    auto_mode_requested: bool,
    protocol_guard_required_before_write: bool,
    reject_reverse_tool_requests: bool,
    model_http_count_measured: bool,
    candidate_source_is_exact_binary: bool,
}

impl Plan {
    fn validate(&self) -> ProbeResult<()> {
        check(
            self.schema_version == 1 && self.scope == SCOPE,
            "plan_scope_invalid",
        )?;
        check(
            self.max_native_inputs == 0
                && self.max_protocol_requests == MAX_REQUESTS
                && self.max_session_processes == MAX_PROCESSES,
            "plan_budget_invalid",
        )?;
        check(
            self.allowed_methods
                .iter()
                .map(String::as_str)
                .eq(ALLOWED_METHODS)
                && self
                    .diagnostic_methods
                    .iter()
                    .map(String::as_str)
                    .eq(DIAGNOSTIC_METHODS),
            "plan_methods_invalid",
        )?;
        check(
            is_hash(&self.profile_sha256) && is_hash(&self.config_sha256),
            "plan_snapshot_invalid",
        )?;
        check(
            !self.always_approve_requested
                && !self.auto_mode_requested
                && self.protocol_guard_required_before_write
                && self.reject_reverse_tool_requests
                && !self.model_http_count_measured
                && !self.candidate_source_is_exact_binary,
            "plan_guard_invalid",
        )
    }
}

struct Paths {
    root: PathBuf,
    cwd: PathBuf,
    executable: PathBuf,
    state: PathBuf,
    profile: PathBuf,
    config: PathBuf,
}

fn named_path(name: &str) -> ProbeResult<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or("explicit_path_missing")
}

fn private_path(path: &Path, root: &Path, directory: bool, readonly: bool) -> ProbeResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "private_path_missing")?;
    check(
        path.is_absolute()
            && path.canonicalize().map_err(|_| "private_path_invalid")? == path
            && path.starts_with(root)
            && !metadata.file_type().is_symlink()
            && metadata.is_dir() == directory
            && (directory || metadata.is_file())
            && (!readonly || metadata.permissions().readonly()),
        "private_path_invalid",
    )?;
    #[cfg(unix)]
    {
        // 与生产监督者同一 UID；只核对私有材料，不读取认证文件。
        let owner = unsafe { libc::geteuid() };
        check(
            metadata.uid() == owner
                && metadata.mode() & 0o077 == 0
                && (directory || metadata.nlink() == 1)
                && (!readonly || metadata.mode() & 0o222 == 0),
            "private_path_permissions_invalid",
        )?;
    }
    Ok(())
}

fn bounded_file(path: &Path) -> ProbeResult<Vec<u8>> {
    check(
        fs::metadata(path).map_err(|_| "snapshot_missing")?.len() <= MAX_FILE_BYTES as u64,
        "snapshot_budget_exceeded",
    )?;
    let bytes = fs::read(path).map_err(|_| "snapshot_read_failed")?;
    check(
        !bytes.is_empty() && bytes.len() <= MAX_FILE_BYTES,
        "snapshot_budget_exceeded",
    )?;
    Ok(bytes)
}

impl Paths {
    fn load() -> ProbeResult<(Self, Plan, PathBuf)> {
        check(cfg!(target_os = "macos"), "platform_not_prepared")?;
        let root = named_path("INFINISHELL_GROK_POLICY_PREFLIGHT_ROOT")?;
        private_path(&root, &root, true, false)?;
        let plan_path = named_path("INFINISHELL_GROK_POLICY_PREFLIGHT_PLAN")?;
        check(
            plan_path == root.join("policy/plan.json"),
            "plan_path_invalid",
        )?;
        private_path(&plan_path, &root, false, true)?;
        let plan: Plan =
            serde_json::from_slice(&bounded_file(&plan_path)?).map_err(|_| "plan_decode_failed")?;
        plan.validate()?;
        let executable = named_path("INFINISHELL_GROK_POLICY_PREFLIGHT_EXECUTABLE")?;
        check(
            executable == root.join("grok-sandbox"),
            "executable_path_invalid",
        )?;
        let paths = Self {
            cwd: root.join("project"),
            state: root.join("state"),
            profile: root.join("policy/profile.md"),
            config: root.join("home/.grok/config.toml"),
            executable,
            root,
        };
        private_path(&paths.executable, &paths.root, false, false)?;
        private_path(&paths.cwd, &paths.root, true, false)?;
        private_path(&paths.state, &paths.root, true, false)?;
        check(
            named_path("GROK_HOME")? == paths.root.join("home/.grok"),
            "private_config_boundary_invalid",
        )?;
        paths.verify_snapshots(&plan)?;
        let artifact = named_path("INFINISHELL_GROK_POLICY_PREFLIGHT_ARTIFACT")?;
        check(
            artifact == paths.root.join("evidence.ndjson"),
            "artifact_path_invalid",
        )?;
        private_path(&artifact, &paths.root, false, false)?;
        check(
            fs::metadata(&artifact)
                .map_err(|_| "artifact_missing")?
                .len()
                == 0,
            "artifact_not_empty",
        )?;
        Ok((paths, plan, artifact))
    }

    fn verify_snapshots(&self, plan: &Plan) -> ProbeResult<()> {
        private_path(&self.profile, &self.root, false, true)?;
        private_path(&self.config, &self.root, false, true)?;
        check(
            hash(&bounded_file(&self.profile)?) == plan.profile_sha256
                && hash(&bounded_file(&self.config)?) == plan.config_sha256,
            "snapshot_changed",
        )
    }

    fn arguments(&self) -> Vec<OsString> {
        vec![
            "agent".into(),
            "--no-leader".into(),
            "--agent-profile".into(),
            self.profile.clone().into_os_string(),
            "stdio".into(),
        ]
    }
}

struct Evidence {
    file: File,
    bytes: usize,
    records: usize,
}

impl Evidence {
    fn open(path: &Path) -> ProbeResult<Self> {
        let mut options = OpenOptions::new();
        options.append(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        Ok(Self {
            file: options.open(path).map_err(|_| "artifact_open_failed")?,
            bytes: 0,
            records: 0,
        })
    }

    fn record(&mut self, value: Value) -> ProbeResult<()> {
        let mut bytes = encoded(&value)?;
        bytes.push(b'\n');
        check(
            self.bytes + bytes.len() <= MAX_BYTES && self.records < MAX_FRAMES,
            "evidence_budget_exceeded",
        )?;
        self.file
            .write_all(&bytes)
            .map_err(|_| "evidence_write_failed")?;
        self.file.flush().map_err(|_| "evidence_flush_failed")?;
        self.bytes += bytes.len();
        self.records += 1;
        Ok(())
    }

    fn failure(&mut self, bytes: &[u8]) -> ProbeResult<()> {
        self.record(
            json!({"event":"probe_failed","reason_sha256":hash(bytes),"reason_bytes":bytes.len()}),
        )
    }

    fn phase_failure(&mut self, generation: Uuid, stage: &str, reason: &str) -> ProbeResult<()> {
        self.record(
            json!({"event":"phase_failure","generation":generation,"stage":stage,
            "reason_sha256":hash(reason.as_bytes()),"reason_bytes":reason.len()}),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestKind {
    Initialize,
    Authenticate,
    New,
    Load,
    Info,
    State,
    McpList,
    DebugAgent,
}

impl RequestKind {
    fn method(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::Authenticate => "authenticate",
            Self::New => "session/new",
            Self::Load => "session/load",
            Self::Info => "_x.ai/session/info",
            Self::State => "_x.ai/session/state",
            Self::McpList => "_x.ai/mcp/list",
            Self::DebugAgent => "_x.ai/debug/agent",
        }
    }

    fn diagnostic(self) -> bool {
        matches!(
            self,
            Self::Info | Self::State | Self::McpList | Self::DebugAgent
        )
    }

    fn params(self, cwd: &Path, session_id: Option<&str>) -> ProbeResult<Value> {
        let session = || {
            session_id
                .filter(|id| is_session_id(id))
                .ok_or("native_session_missing")
        };
        Ok(match self {
            Self::Initialize => json!({"protocolVersion":1,"clientCapabilities":{
                "fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}}),
            // 只消费原生公布的缓存登录，不读取凭据或把历史会话写入认证请求。
            Self::Authenticate => json!({"methodId":"cached_token","_meta":{"headless":true}}),
            Self::New => {
                check(session_id.is_none(), "new_session_already_bound")?;
                json!({"cwd":cwd,"mcpServers":[],"_meta":{"yoloMode":false,"autoMode":false}})
            }
            Self::Load => json!({"cwd":cwd,"mcpServers":[],"sessionId":session()?,
                "_meta":{"yoloMode":false,"autoMode":false}}),
            // 固定主源 handler 的准确签名；不从 response 或名称猜请求字段。
            Self::Info => json!({"sessionId":session()?}),
            Self::State => json!({"sessionId":session()?,"cwd":cwd}),
            Self::McpList => json!({"sessionId":session()?,"cache":true}),
            // 此方法读取当前独立 agent 的 registry，不接收 session 参数。
            Self::DebugAgent => json!({}),
        })
    }
}

fn phase_requests(resume: bool) -> [RequestKind; 7] {
    [
        RequestKind::Initialize,
        RequestKind::Authenticate,
        if resume {
            RequestKind::Load
        } else {
            RequestKind::New
        },
        RequestKind::Info,
        RequestKind::State,
        RequestKind::McpList,
        RequestKind::DebugAgent,
    ]
}

fn confirm_cached_token_method(initialized: &Value) -> ProbeResult<()> {
    let methods = initialized["authMethods"]
        .as_array()
        .filter(|methods| methods.len() <= 64)
        .ok_or("native_auth_methods_invalid")?;
    check(
        methods.iter().all(|method| {
            method.is_object() && method["id"].as_str().is_some_and(|id| !id.is_empty())
        }),
        "native_auth_methods_invalid",
    )?;
    // 无交互探针只选择唯一的 cached_token；不退回 API key 或浏览器授权。
    check(
        methods
            .iter()
            .filter(|method| method["id"] == "cached_token")
            .count()
            == 1,
        "native_cached_token_unavailable",
    )
}

fn outbound(
    kind: RequestKind,
    id: usize,
    cwd: &Path,
    session_id: Option<&str>,
) -> ProbeResult<Value> {
    check(
        id > 0 && id <= MAX_REQUESTS && cwd.is_absolute(),
        "request_identity_invalid",
    )?;
    Ok(
        json!({"jsonrpc":"2.0","id":id,"method":kind.method(),"params":kind.params(cwd,session_id)?}),
    )
}

fn guard_outbound(
    message: &Value,
    kind: RequestKind,
    id: usize,
    cwd: &Path,
    session: Option<&str>,
) -> ProbeResult<()> {
    check(
        message == &outbound(kind, id, cwd, session)?,
        "outbound_parameters_rejected",
    )
}

struct WireReader<R> {
    stream: R,
    buffer: Vec<u8>,
    bytes: usize,
    frames: usize,
    byte_limit: usize,
    frame_limit: usize,
}

impl<R: AsyncRead + Unpin> WireReader<R> {
    fn new(stream: R, byte_limit: usize, frame_limit: usize) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
            bytes: 0,
            frames: 0,
            byte_limit,
            frame_limit,
        }
    }

    async fn next(&mut self) -> ProbeResult<Option<(Value, Vec<u8>)>> {
        loop {
            if let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let raw: Vec<u8> = self.buffer.drain(..=end).collect();
                check(
                    self.frames < self.frame_limit && !raw.iter().all(u8::is_ascii_whitespace),
                    "native_frame_budget_exceeded",
                )?;
                let value = serde_json::from_slice(&raw).map_err(|_| "native_frame_invalid")?;
                self.frames += 1;
                return Ok(Some((value, raw)));
            }
            let mut bytes = [0u8; 8192];
            let limit = self
                .byte_limit
                .saturating_sub(self.bytes)
                .saturating_add(1)
                .min(bytes.len());
            let count = self
                .stream
                .read(&mut bytes[..limit])
                .await
                .map_err(|_| "native_read_failed")?;
            if count == 0 {
                check(self.buffer.is_empty(), "native_unterminated_frame")?;
                return Ok(None);
            }
            check(
                self.bytes + count <= self.byte_limit,
                "native_byte_budget_exceeded",
            )?;
            self.bytes += count;
            self.buffer.extend_from_slice(&bytes[..count]);
        }
    }
}

#[derive(Default)]
struct Observations {
    native_session: Option<String>,
    received_notifications: usize,
    metadata_notification_ids: HashMap<String, [u8; 32]>,
    pending_response: Option<(usize, RequestKind)>,
    completed_responses: HashMap<usize, ([u8; 32], RequestKind)>,
}

fn session_notification_metadata(params: &Value) -> bool {
    // 真实 1.0.30 夹具中的模型变更属于元数据；不会执行工具、绑定会话或证明权限。
    let Some(params) = params.as_object() else {
        return false;
    };
    let Some(update) = params.get("update").and_then(Value::as_object) else {
        return false;
    };
    params
        .keys()
        .all(|key| matches!(key.as_str(), "sessionId" | "update" | "_meta"))
        && params
            .get("sessionId")
            .and_then(Value::as_str)
            .is_some_and(is_session_id)
        && params.get("_meta").is_none_or(Value::is_object)
        && update.keys().all(|key| {
            matches!(
                key.as_str(),
                "sessionUpdate" | "model_id" | "reasoning_effort"
            )
        })
        && update
            .get("sessionUpdate")
            .is_some_and(|kind| kind == "model_changed")
        && update
            .get("model_id")
            .and_then(Value::as_str)
            .is_some_and(|model| !model.is_empty() && model.len() <= 256)
        && update.get("reasoning_effort").is_none_or(|effort| {
            effort
                .as_str()
                .is_some_and(|effort| !effort.is_empty() && effort.len() <= 256)
        })
}

fn global_models_metadata(params: &Value) -> ProbeResult<()> {
    check(
        params.as_object().is_some_and(|params| {
            params.len() == 2
                && params.contains_key("currentModelId")
                && params.contains_key("availableModels")
        }),
        "native_models_metadata_invalid",
    )?;
    check(
        encoded(params)?.len() <= MAX_FILE_BYTES,
        "native_models_metadata_budget_exceeded",
    )?;
    let available = params["availableModels"]
        .as_array()
        .ok_or("native_models_metadata_invalid")?;
    check(
        available.len() <= 64,
        "native_models_metadata_budget_exceeded",
    )?;
    check(
        available.iter().all(|model| {
            model.as_object().is_some_and(|model| {
                model
                    .keys()
                    .all(|key| matches!(key.as_str(), "modelId" | "name" | "description" | "_meta"))
                    && model.get("_meta").is_none_or(Value::is_object)
            })
        }),
        "native_models_metadata_invalid",
    )?;
    // 复用生产展示类型；扩展模型描述只受字节预算约束，不解释为权限或工具活动。
    let models: ReportedModels =
        decode_metadata(params).map_err(|_| "native_models_metadata_invalid")?;
    check(
        !models.current_model_id.is_empty()
            && models.current_model_id.len() <= 256
            && models.available_models.iter().all(|model| {
                !model.model_id.is_empty()
                    && model.model_id.len() <= 256
                    && !model.name.is_empty()
                    && model.name.len() <= 1024
                    && model
                        .description
                        .as_ref()
                        .is_none_or(|text| text.len() <= 8192)
            }),
        "native_models_metadata_invalid",
    )
}

fn global_catalog_empty(params: &Value) -> ProbeResult<()> {
    check(
        params
            .as_object()
            .is_some_and(|params| params.len() == 1 && params.contains_key("mcpServers")),
        "native_mcp_catalog_invalid",
    )?;
    let servers = params["mcpServers"]
        .as_array()
        .ok_or("native_mcp_catalog_invalid")?;
    check(servers.len() <= 64, "native_mcp_catalog_budget_exceeded")?;
    // 实际 profile 及 New/Load 均注册空列表；目录不能引入其他服务域。
    check(servers.is_empty(), "native_extra_mcp_sources_observed")
}

fn ignored_extension_notification(frame: &Value) -> bool {
    // ACP 建议忽略未知扩展通知；这里只识别信封，不解释正文或证明权限。
    // https://agentclientprotocol.com/protocol/v1/extensibility#custom-notifications
    let Some(object) = frame.as_object() else {
        return false;
    };
    let Some(method) = frame.get("method").and_then(Value::as_str) else {
        return false;
    };
    if !method.starts_with('_') || method.len() <= 1 || method.len() > 256 {
        return false;
    }
    if matches!(
        method,
        "_x.ai/mcp/servers_updated"
            | "_x.ai/models/update"
            | "_x.ai/session_notification"
            | "_x.ai/mcp_initialized"
            | "_x.ai/mcp/init_progress"
            | "_x.ai/mcp/server_status"
            | "_x.ai/queue/changed"
            | "_x.ai/session/prompt_complete"
            | "_x.ai/session/updates"
            | "_x.ai/session/update"
            | "_x.ai/mcp/sdk_call"
    ) {
        // 已识别的生命周期、SDK 和目录通知继续走原有严格合同。
        return false;
    }
    frame["jsonrpc"] == "2.0"
        && object
            .keys()
            .all(|key| matches!(key.as_str(), "jsonrpc" | "method" | "params"))
        && frame
            .get("params")
            .is_none_or(|params| params.is_object() || params.is_array())
        && encoded(frame).is_ok_and(|bytes| bytes.len() <= MAX_FILE_BYTES)
}

impl Observations {
    fn response_transaction(&mut self, frame: &Value) -> ProbeResult<Option<RequestKind>> {
        check(
            frame.as_object().is_some_and(|object| object.len() == 3)
                && frame["jsonrpc"] == "2.0"
                && frame.get("method").is_none()
                && ((frame.get("result").is_some_and(Value::is_object)
                    && frame.get("error").is_none())
                    || (frame.get("error").is_some_and(Value::is_object)
                        && frame.get("result").is_none())),
            "native_response_uncorrelated",
        )?;
        let id = frame["id"]
            .as_u64()
            .and_then(|id| usize::try_from(id).ok())
            .ok_or("native_response_uncorrelated")?;
        let fingerprint: [u8; 32] = Sha256::digest(encoded(frame)?).into();
        if let Some((previous, _)) = self.completed_responses.get(&id) {
            check(*previous == fingerprint, "native_response_conflict")?;
            return Ok(None);
        }
        let (pending, kind) = self
            .pending_response
            .ok_or("native_response_uncorrelated")?;
        check(pending == id, "native_response_uncorrelated")?;
        // 只消费本进程已实际发出的事务；迟到回复不补成权限或历史验证成功。
        self.pending_response = None;
        self.completed_responses.insert(id, (fingerprint, kind));
        Ok(Some(kind))
    }

    fn bind(&mut self, session: &str) -> ProbeResult<()> {
        check(
            is_session_id(session)
                && self
                    .native_session
                    .as_deref()
                    .is_none_or(|old| old == session),
            "native_session_changed",
        )?;
        self.native_session = Some(session.to_owned());
        Ok(())
    }

    fn notification(&mut self, frame: &Value) -> ProbeResult<()> {
        if ignored_extension_notification(frame) {
            check(
                self.received_notifications < MAX_FRAMES,
                "native_notification_budget_exceeded",
            )?;
            // 不消费 pending，不绑定 session，不保存 eventId，也不向原生端发送回复。
            self.received_notifications += 1;
            return Ok(());
        }
        check(
            frame.as_object().is_some_and(|object| object.len() == 3)
                && frame["jsonrpc"] == "2.0"
                && frame.get("id").is_none()
                && frame.get("result").is_none()
                && frame.get("error").is_none()
                && frame["params"].is_object(),
            "native_reverse_request_rejected",
        )?;
        check(
            frame["params"]["_meta"].get("promptId").is_none()
                && frame["params"].get("promptId").is_none(),
            "native_prompt_activity_rejected",
        )?;
        match frame["method"].as_str() {
            Some("_x.ai/mcp/servers_updated") => {
                // 全局目录属于当前阶段独占的传输；不声明会话、回合或原生 generation。
                global_catalog_empty(&frame["params"])?;
                check(
                    self.received_notifications < MAX_FRAMES,
                    "native_mcp_catalog_budget_exceeded",
                )?;
                self.received_notifications += 1;
                return Ok(());
            }
            Some("_x.ai/models/update") => {
                global_models_metadata(&frame["params"])?;
                check(
                    self.received_notifications < MAX_FRAMES,
                    "native_notification_budget_exceeded",
                )?;
                // 官方 New/Load 都会推送全局模型目录；目录不能绑定会话或消费请求。
                self.received_notifications += 1;
                return Ok(());
            }
            Some("_x.ai/session_notification") => {
                check(
                    session_notification_metadata(&frame["params"]),
                    "native_session_notification_activity_rejected",
                )?;
                check(
                    self.received_notifications < MAX_FRAMES,
                    "native_notification_budget_exceeded",
                )?;
                if let Some(session) = self.native_session.as_deref() {
                    check(
                        frame["params"]["sessionId"] == session,
                        "native_session_changed",
                    )?;
                    // Load 的旧模型元数据与当前连接同样只忽略；实际 New/Load 回复才推进事务。
                    if let Some(id) = frame["params"]["_meta"]["eventId"].as_str() {
                        check(super::valid_native_id(id), "native_notification_id_invalid")?;
                        let fingerprint: [u8; 32] = Sha256::digest(encoded(frame)?).into();
                        if let Some(previous) = self.metadata_notification_ids.get(id) {
                            check(*previous == fingerprint, "native_notification_conflict")?;
                        } else {
                            self.metadata_notification_ids
                                .insert(id.to_owned(), fingerprint);
                        }
                    }
                }
                // New 回复前没有可信会话，不能从兼容通知推导身份。
                self.received_notifications += 1;
                return Ok(());
            }
            Some("session/update") => {
                check(
                    matches!(
                        frame["params"]["update"]["sessionUpdate"].as_str(),
                        Some(
                            "available_commands_update"
                                | "current_mode_update"
                                | "config_option_update"
                                | "session_info_update"
                        )
                    ),
                    "native_turn_or_tool_activity_rejected",
                )?;
                check(
                    ["content", "toolCallId", "rawInput", "rawOutput"]
                        .into_iter()
                        .all(|key| frame["params"]["update"].get(key).is_none()),
                    "native_tool_payload_rejected",
                )?;
            }
            Some("_x.ai/mcp_initialized") => {
                check(
                    frame["params"]["mcpToolCount"].as_u64().is_some()
                        && frame["params"]["elapsedMs"].as_u64().is_some(),
                    "native_metadata_invalid",
                )?;
            }
            Some(_) | None => return Err("native_unknown_notification_rejected"),
        }
        self.bind(
            frame["params"]["sessionId"]
                .as_str()
                .ok_or("native_metadata_session_missing")?,
        )?;
        self.received_notifications += 1;
        Ok(())
    }
}

fn notification_diagnostic(frame: &Value, generation: Uuid, ignored_extension: bool) -> Value {
    let method = match frame["method"].as_str() {
        Some(
            "session/update"
            | "_x.ai/mcp_initialized"
            | "_x.ai/mcp/init_progress"
            | "_x.ai/mcp/server_status"
            | "_x.ai/mcp/servers_updated"
            | "_x.ai/session_notification"
            | "_x.ai/models/update",
        ) => frame["method"].clone(),
        Some(_) | None => super::diagnostic_value(frame.get("method")),
    };
    let mut diagnostic = json!({"event":"native_notification_diagnostic","generation":generation,"method":method,
        "method_summary":super::diagnostic_value(frame.get("method")),
        "frame_type":super::diagnostic_value_type(Some(frame)),
        "params_type":super::diagnostic_value_type(frame.get("params")),
        "id_present":frame.get("id").is_some(),"result_present":frame.get("result").is_some(),
        "error_present":frame.get("error").is_some(),
        "session_id":super::diagnostic_value(frame.get("params").and_then(|params|params.get("sessionId")))});
    if ignored_extension {
        diagnostic["ignored_extension_notification"] = json!(true);
    }
    if frame["method"] == "_x.ai/session_notification" {
        diagnostic["session_notification_metadata_only"] = json!(
            frame.as_object().is_some_and(|frame| frame.len() == 3)
                && frame["jsonrpc"] == "2.0"
                && frame["params"]["_meta"].get("promptId").is_none()
                && session_notification_metadata(&frame["params"])
        );
    }
    if frame["method"] == "_x.ai/models/update" {
        // 公开实际封闭元数据校验的布尔值，不输出模型目录或扩展描述。
        diagnostic["global_models_metadata_only"] = json!(
            frame.as_object().is_some_and(|frame| frame.len() == 3)
                && frame["jsonrpc"] == "2.0"
                && global_models_metadata(&frame["params"]).is_ok()
        );
    }
    if frame["method"] == "_x.ai/mcp/servers_updated" {
        // 仅公开封闭空目录的校验结论；失败参数或环境值不进入投影。
        diagnostic["global_catalog_closed_empty"] = json!(
            frame.as_object().is_some_and(|frame| frame.len() == 3)
                && frame["jsonrpc"] == "2.0"
                && global_catalog_empty(&frame["params"]).is_ok()
        );
    }
    diagnostic
}

fn phase_outcome<T>(
    primary: ProbeResult<T>,
    cleanup: ProbeResult<()>,
    drained: ProbeResult<()>,
) -> ProbeResult<T> {
    // 收尾失败独立报告；已有语义失败始终优先，清理成功也不能升级为接口通过。
    let value = primary?;
    cleanup?;
    drained?;
    Ok(value)
}

fn rpc_error_diagnostic(
    frame: &Value,
    generation: Uuid,
    id: usize,
    kind: RequestKind,
    consumed: Option<RequestKind>,
    raw: &[u8],
) -> ProbeResult<Option<Value>> {
    // consumed 只来自实际账本本次消费；重复回复的 None 不构成当前事务。
    check(consumed == Some(kind), "error_diagnostic_not_owned")?;
    check(
        id > 0
            && id <= MAX_REQUESTS
            && frame.as_object().is_some_and(|frame| frame.len() == 3)
            && frame["jsonrpc"] == "2.0"
            && frame["id"].as_u64() == Some(id as u64)
            && frame.get("method").is_none(),
        "error_diagnostic_not_owned",
    )?;
    let Some(error) = frame.get("error") else {
        return Ok(None);
    };
    check(
        error.is_object() && frame.get("result").is_none(),
        "error_diagnostic_invalid",
    )?;
    // 只公开有界标准错误码；message/data 无论形状都只保留类型、字节和散列。
    let code = error["code"]
        .as_i64()
        .filter(|code| i32::try_from(*code).is_ok());
    Ok(Some(
        json!({"event":"rpc_error_diagnostic","generation":generation,
        "sequence":id,"rpc_id":id,"method":kind.method(),"owned_current_request":true,
        "error_code":code,"message":super::diagnostic_value(error.get("message")),
        "data":super::diagnostic_value(error.get("data")),
        "response_bytes":raw.len(),"response_sha256":hash(raw)}),
    ))
}

fn response_body(frame: &Value, id: usize, kind: RequestKind) -> ProbeResult<&Value> {
    let object = frame.as_object().ok_or("native_response_invalid")?;
    check(
        object.len() == 3
            && frame["jsonrpc"] == "2.0"
            && frame["id"].as_u64() == Some(id as u64)
            && frame.get("method").is_none(),
        "native_response_uncorrelated",
    )?;
    if frame.get("error").is_some() {
        return Err(if frame["error"]["code"].as_i64() == Some(-32601) {
            "method_not_found"
        } else {
            "rpc_error"
        });
    }
    let result = frame
        .get("result")
        .filter(|value| value.is_object())
        .ok_or("native_result_invalid")?;
    match kind {
        RequestKind::Info | RequestKind::McpList | RequestKind::DebugAgent => {
            // 这些候选 handler 使用 ExtMethodResult：result 内再包 result，不猜 data/status 包装。
            check(
                result.as_object().is_some_and(|object| object.len() == 1)
                    && result.get("error").is_none(),
                "extension_result_failed",
            )?;
            result
                .get("result")
                .filter(|value| value.is_object())
                .ok_or("extension_result_missing")
        }
        RequestKind::Initialize
        | RequestKind::Authenticate
        | RequestKind::New
        | RequestKind::Load
        | RequestKind::State => Ok(result),
    }
}

/// 缺少诊断接口属于调查结果，不能伪装成成功响应或空工具目录。
fn rpc_outcome(kind: RequestKind, body: ProbeResult<&Value>) -> ProbeResult<Option<Value>> {
    match body {
        Ok(body) => Ok(Some(body.clone())),
        Err("method_not_found") if kind.diagnostic() => Ok(None),
        Err(reason) => Err(reason),
    }
}

fn info_confirmation_diagnostic(body: &Value, session: &str, cwd: &Path) -> Value {
    // 仅公开类型、散列、计数和精确比较；未知正文、路径与会话 ID 不进入证据。
    json!({
        "session_id":super::diagnostic_value(body.get("sessionId")),
        "cwd":super::diagnostic_value(body.get("cwd")),
        "turns":super::diagnostic_value(body.get("turns")),
        "turn_index":super::diagnostic_value(body.get("turnIndex")),
        "session_id_matches":body["sessionId"] == session,
        "cwd_matches":body["cwd"] == json!(cwd),
        "turns_u64":body["turns"].as_u64(),
        "turn_index_u64":body["turnIndex"].as_u64()
    })
}

fn confirm_info(body: &Value, session: &str, cwd: &Path) -> ProbeResult<()> {
    // 固定 1.0.30 的无输入 New 实测为一个初始回合、索引零；回合数量不是已发送输入数。
    // 零输入另由所有写入前的封闭 RPC 守卫保证，不能仅凭此 info 推断历史正文为空。
    check(
        body["sessionId"] == session
            && body["cwd"] == json!(cwd)
            && body["turns"].as_u64() == Some(1)
            && body["turnIndex"].as_u64() == Some(0),
        "empty_native_history_unconfirmed",
    )
}

fn mcp_counts(body: &Value) -> ProbeResult<(usize, usize)> {
    let servers = body["servers"]
        .as_array()
        .filter(|servers| servers.len() <= 64)
        .ok_or("mcp_catalog_shape_unknown")?;
    let mut count = 0;
    for server in servers {
        check(server["source"].is_string(), "mcp_source_shape_unknown")?;
        if let Some(session) = server.get("session").filter(|session| !session.is_null()) {
            // 固定候选 schema 对空工具数组使用 skip_serializing_if，缺键表示空 Vec。
            if let Some(tools) = session.get("tools") {
                count += tools.as_array().ok_or("mcp_tools_shape_unknown")?.len();
            }
        }
        check(count <= 256, "mcp_catalog_budget_exceeded")?;
    }
    // 创建时没有注册任何 MCP；出现条目只记录额外来源，不推出完整来源已封闭。
    Ok((count, servers.len()))
}

struct Budget {
    requests: usize,
    processes: usize,
    deadline: Instant,
    native_bytes: usize,
    native_frames: usize,
}

impl Budget {
    fn new() -> Self {
        Self {
            requests: 0,
            processes: 0,
            deadline: Instant::now() + Duration::from_secs(360),
            native_bytes: 0,
            native_frames: 0,
        }
    }

    fn remaining(&self, phase: Duration) -> ProbeResult<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .map(|left| left.min(phase))
            .ok_or("probe_deadline_exceeded")
    }

    fn claim_process(&mut self) -> ProbeResult<()> {
        check(self.processes < MAX_PROCESSES, "process_budget_exceeded")?;
        self.processes += 1;
        Ok(())
    }
}

async fn rpc<W: AsyncWrite + Unpin, R: AsyncRead + Unpin>(
    stdin: &mut W,
    reader: &mut WireReader<R>,
    observations: &mut Observations,
    kind: RequestKind,
    generation: Uuid,
    paths: &Paths,
    plan: &Plan,
    budget: &mut Budget,
    evidence: &mut Evidence,
) -> ProbeResult<Option<Value>> {
    paths.verify_snapshots(plan)?;
    check(
        budget.requests < plan.max_protocol_requests,
        "request_budget_exceeded",
    )?;
    let id = budget.requests + 1;
    let message = outbound(kind, id, &paths.cwd, observations.native_session.as_deref())?;
    guard_outbound(
        &message,
        kind,
        id,
        &paths.cwd,
        observations.native_session.as_deref(),
    )?;
    check(
        observations.pending_response.is_none(),
        "native_request_already_pending",
    )?;
    let mut bytes = encoded(&message)?;
    bytes.push(b'\n');
    check(
        bytes.len() <= MAX_FILE_BYTES,
        "request_byte_budget_exceeded",
    )?;
    // 最后一次守卫就在真实写入之前，记录的散列覆盖实际换行及全部参数。
    budget.requests += 1;
    evidence.record(json!({"event":"rpc_sent","generation":generation,"sequence":id,
        "method":kind.method(),"rpc_id":id,"request_bytes":bytes.len(),"request_sha256":hash(&bytes),
        "guard_checked_before_write":true}))?;
    tokio::time::timeout(budget.remaining(Duration::from_secs(5))?, async {
        stdin.write_all(&bytes).await?;
        stdin.flush().await
    })
    .await
    .map_err(|_| "native_write_timeout")?
    .map_err(|_| "native_write_failed")?;
    observations.pending_response = Some((id, kind));
    let incoming = tokio::time::timeout(budget.remaining(Duration::from_secs(30))?, async {
        loop {
            let (frame, raw) = reader.next().await?.ok_or("native_eof_before_response")?;
            if frame.get("method").is_some() {
                let observed = observations.notification(&frame);
                evidence.record(notification_diagnostic(
                    &frame,
                    generation,
                    observed.is_ok() && ignored_extension_notification(&frame),
                ))?;
                if let Err(reason) = observed {
                    evidence.failure(&raw)?;
                    return Err(reason);
                }
                continue;
            }
            let consumed = observations.response_transaction(&frame)?;
            if consumed.is_none() {
                continue;
            }
            return Ok((frame, raw, consumed));
        }
    })
    .await;
    let (frame, raw, consumed) = match incoming {
        Ok(result) => result?,
        Err(_) => {
            evidence.record(json!({"event":"rpc_response","generation":generation,
            "sequence":id,"rpc_id":id,"status":"timeout","response_bytes":0,"response_sha256":null}))?;
            return Err("native_response_timeout");
        }
    };
    let body = response_body(&frame, id, kind);
    let status = match &body {
        Ok(_) => "ok",
        Err(reason) if *reason == "method_not_found" => "method_not_found",
        Err(_) => "rpc_error",
    };
    evidence.record(
        json!({"event":"rpc_response","generation":generation,"sequence":id,
        "rpc_id":id,"status":status,"response_bytes":raw.len(),"response_sha256":hash(&raw)}),
    )?;
    if let Some(diagnostic) = rpc_error_diagnostic(&frame, generation, id, kind, consumed, &raw)? {
        evidence.record(diagnostic)?;
    }
    if kind.diagnostic() {
        let keys: Vec<String> = body
            .as_ref()
            .ok()
            .and_then(|body| body.as_object())
            .map(|object| object.keys().map(|key| hash(key.as_bytes())).collect())
            .unwrap_or_default();
        check(keys.len() <= 128, "diagnostic_key_budget_exceeded")?;
        evidence.record(
            json!({"event":"diagnostic_observed","generation":generation,
            "method":kind.method(),"status":status,"top_level_key_sha256s":keys}),
        )?;
    }
    if kind == RequestKind::Info
        && let Ok(info) = &body
    {
        let session = observations
            .native_session
            .as_deref()
            .ok_or("native_session_missing")?;
        // 此时回复已由当前账本消费，失败形状先记录，再由原有空会话检查拒绝。
        evidence.record(
            json!({"event":"session_info_diagnostic","generation":generation,
            "sequence":id,"rpc_id":id,"method":kind.method(),
            "response_bytes":raw.len(),"response_sha256":hash(&raw),
            "confirmation":info_confirmation_diagnostic(info,session,&paths.cwd)}),
        )?;
    }
    rpc_outcome(kind, body)
}

fn cleanup_event(receipt: &ExitReceipt) -> ProbeResult<Value> {
    let reason = match receipt.exit_reason {
        ExitReason::NativeExit => "native_exit",
        ExitReason::StopRequested => "stop_requested",
        ExitReason::HostDisconnected => "host_disconnected",
        ExitReason::StdioClosed => "stdio_closed",
    };
    let bytes = serde_json::to_vec(receipt).map_err(|_| "receipt_encode_failed")?;
    // 失败取消可产生无数值退出码的真实收据；保留 null，不补成正常退出。
    Ok(
        json!({"event":"process_cleanup","generation":receipt.generation,
        "exit_code":receipt.exit_code,"exit_reason":reason,
        "cleanup_confirmed":receipt.cleanup_confirmed,"receipt_sha256":hash(&bytes)}),
    )
}

fn normal_receipt(receipt: &ExitReceipt, generation: Uuid) -> ProbeResult<()> {
    check(
        receipt.version == 1
            && receipt.generation == generation
            && receipt.cleanup_confirmed
            && receipt.exit_code == Some(0)
            && receipt.exit_reason == ExitReason::StdioClosed
            && receipt.containment == "macos_resource_coalition",
        "normal_stdio_cleanup_unconfirmed",
    )
}

async fn phase(
    previous_session: Option<&str>,
    paths: &Paths,
    plan: &Plan,
    budget: &mut Budget,
    evidence: &mut Evidence,
) -> ProbeResult<String> {
    paths.verify_snapshots(plan)?;
    budget.claim_process()?;
    let generation = Uuid::new_v4();
    evidence.record(json!({"event":"launch_snapshot","generation":generation,
        "phase":if previous_session.is_some(){"resume"}else{"new"},"cli_version":FIXED_VERSION,
        "cli_sha256":FIXED_BINARY_SHA256,"profile_sha256":plan.profile_sha256,"config_sha256":plan.config_sha256,
        "always_approve_requested":false,"auto_mode_requested":false}))?;
    let mut child = tokio::time::timeout(
        budget.remaining(Duration::from_secs(30))?,
        managed_process::spawn(
            &paths.state,
            generation,
            &paths.executable,
            &paths.arguments(),
            &paths.cwd,
        ),
    )
    .await
    .map_err(|_| "production_spawn_timeout")?
    .map_err(|_| "production_spawn_failed")?;
    let mut stdin = child.stdin.take().ok_or("native_stdin_missing")?;
    let stdout = child.stdout.take().ok_or("native_stdout_missing")?;
    let mut reader = WireReader::new(
        stdout,
        MAX_BYTES.saturating_sub(budget.native_bytes),
        MAX_FRAMES.saturating_sub(budget.native_frames),
    );
    let mut observations = Observations {
        native_session: previous_session.map(str::to_owned),
        ..Default::default()
    };
    let result: ProbeResult<String> = async {
        let [initialize, authenticate, opening, info_request, state_request, mcp_request, debug_request] =
            phase_requests(previous_session.is_some());
        let initialized = rpc(&mut stdin,&mut reader,&mut observations,initialize,
            generation,paths,plan,budget,evidence).await?.ok_or("native_initialize_missing")?;
        check(initialized["protocolVersion"].as_u64() == Some(1)
            && initialized["_meta"]["agentVersion"] == "1.0.30"
            && initialized["agentCapabilities"]["loadSession"] == true, "native_identity_unconfirmed")?;
        confirm_cached_token_method(&initialized)?;
        evidence.record(json!({"event":"auth_method_selected","generation":generation,
            "method_id":"cached_token","advertised":true,"headless":true}))?;
        // 认证只完成当前 RPC，不把响应中的会话字段绑定为 New/Load 的原生身份。
        rpc(&mut stdin,&mut reader,&mut observations,authenticate,
            generation,paths,plan,budget,evidence).await?.ok_or("native_authentication_missing")?;
        let opened = rpc(&mut stdin,&mut reader,&mut observations,opening,
            generation,paths,plan,budget,evidence).await?.ok_or("native_opening_missing")?;
        if let Some(previous) = previous_session {
            for candidate in [opened.get("sessionId"), opened["_meta"].get("sessionId"),
                opened["_meta"]["x.ai/sessionDetail"].get("sessionId")].into_iter().flatten() {
                check(candidate.as_str() == Some(previous), "native_recovery_session_changed")?;
            }
        } else {
            observations.bind(opened["sessionId"].as_str().ok_or("native_new_session_missing")?)?;
        }
        let session = observations.native_session.clone().ok_or("native_session_missing")?;
        let info = rpc(&mut stdin,&mut reader,&mut observations,info_request,
            generation,paths,plan,budget,evidence).await?;
        if let Some(info) = &info {
            confirm_info(info,&session,&paths.cwd)?;
        }
        let state = rpc(&mut stdin,&mut reader,&mut observations,state_request,
            generation,paths,plan,budget,evidence).await?;
        let mcp = rpc(&mut stdin,&mut reader,&mut observations,mcp_request,
            generation,paths,plan,budget,evidence).await?;
        let _ = rpc(&mut stdin,&mut reader,&mut observations,debug_request,
            generation,paths,plan,budget,evidence).await?;
        // 没有接口时数量和摘要未知，不能把 null 填成零而声称目录为空。
        let counts = mcp.as_ref().map(mcp_counts).transpose()?;
        let mode_hash = state.as_ref().map(encoded).transpose()?.map(|bytes| hash(&bytes));
        let catalog_hash = mcp.as_ref().map(encoded).transpose()?.map(|bytes| hash(&bytes));
        evidence.record(json!({"event":"mode_observation","generation":generation,
            "state":"unknown","value_sha256":mode_hash}))?;
        evidence.record(json!({"event":"catalog_observation","generation":generation,
            "state":"unknown","coverage":if mcp.is_some(){"mcp_only"}else{"unavailable"},
            "tool_count":counts.map(|(tools,_)|tools),
            "catalog_sha256":catalog_hash,"fixed_tool_closure_verified":false}))?;
        evidence.record(json!({"event":"source_observation","generation":generation,
            "state":"unknown","extra_source_count":counts.map(|(_,sources)|sources),
            "all_configuration_sources_verified":false}))?;
        check(counts.is_none_or(|(_,sources)|sources == 0), "native_extra_mcp_sources_observed")?;
        if previous_session.is_some() {
            evidence.record(json!({"event":"resume_checked","generation":generation,
                "native_session_id_sha256":hash(session.as_bytes()),"original_profile_sha256":plan.profile_sha256,
                "current_profile_sha256":plan.profile_sha256,"same_profile":true,"inputs_replayed":0}))?;
        }
        paths.verify_snapshots(plan)?;
        Ok(session)
    }.await;
    if let Err(reason) = &result {
        evidence.failure(reason.as_bytes())?;
        evidence.phase_failure(generation, "primary", reason)?;
    }
    drop(stdin);
    let drain = async {
        while let Some((frame, raw)) = reader.next().await? {
            if frame.get("method").is_some() {
                let observed = observations.notification(&frame);
                evidence.record(notification_diagnostic(
                    &frame,
                    generation,
                    observed.is_ok() && ignored_extension_notification(&frame),
                ))?;
                if let Err(reason) = observed {
                    evidence.failure(&raw)?;
                    return Err(reason);
                }
            } else {
                let kind = observations.response_transaction(&frame)?;
                if let Some(kind) = kind {
                    let id = frame["id"].as_u64().ok_or("native_response_uncorrelated")? as usize;
                    response_body(&frame, id, kind)?;
                }
                evidence.record(
                    json!({"event":"drain_response_observed","generation":generation,
                    "rpc_id":frame["id"],"duplicate":kind.is_none(),
                    "response_bytes":raw.len(),"response_sha256":hash(&raw)}),
                )?;
            }
        }
        Ok(())
    };
    let graceful = result.is_ok();
    let finish = async move {
        if graceful {
            child.finish_after_stdin_close().await
        } else {
            child.finish().await
        }
    };
    let finish_and_drain = async { futures::join!(finish, drain) };
    let finished = match budget.remaining(Duration::from_secs(30)) {
        Ok(remaining) => tokio::time::timeout(remaining, finish_and_drain)
            .await
            .map_err(|_| "production_cleanup_timeout"),
        Err(reason) => {
            drop(finish_and_drain);
            Err(reason)
        }
    };
    budget.native_bytes += reader.bytes;
    budget.native_frames += reader.frames;
    let (cleanup, drained) = match finished {
        Ok((receipt, drained)) => {
            let cleanup = (|| {
                let receipt = receipt.map_err(|_| "production_cleanup_failed")?;
                let confirmed = managed_process::confirmed_exit(&paths.state, generation)
                    .map_err(|_| "production_receipt_invalid")?
                    .ok_or("production_receipt_missing")?;
                check(receipt == confirmed, "production_receipt_changed")?;
                evidence.record(cleanup_event(&receipt)?)?;
                if result.is_ok() {
                    normal_receipt(&receipt, generation)?;
                }
                Ok(())
            })();
            (cleanup, drained)
        }
        Err(reason) => (Err(reason), Ok(())),
    };
    if let Err(reason) = cleanup {
        evidence.phase_failure(generation, "cleanup", reason)?;
    }
    if let Err(reason) = drained {
        evidence.phase_failure(generation, "drain", reason)?;
    }
    let session = phase_outcome(result, cleanup, drained)?;
    paths.verify_snapshots(plan)?;
    Ok(session)
}

async fn exercise(paths: &Paths, plan: &Plan, evidence: &mut Evidence) -> ProbeResult<()> {
    let mut budget = Budget::new();
    evidence.record(json!({"event":"probe_started","scope":SCOPE,"max_native_inputs":0,
        "request_budget":plan.max_protocol_requests,"allowed_methods_sha256":hash(&encoded(&json!(ALLOWED_METHODS))?),
        "guard_before_native_write":true,"production_supervision":true,"candidate_source_is_exact_binary":false}))?;
    let session = phase(None, paths, plan, &mut budget, evidence).await?;
    let restored = phase(Some(&session), paths, plan, &mut budget, evidence).await?;
    check(
        restored == session && budget.requests == MAX_REQUESTS && budget.processes == MAX_PROCESSES,
        "probe_ledger_incomplete",
    )?;
    evidence.record(json!({"event":"probe_finished","scope":SCOPE,"native_inputs":0,"tool_exec_count":0,
        "unexpected_native_activity":0,"protocol_request_count":budget.requests,"guard_before_native_write":true,
        "transport_closed":true,"parent_permission_ceiling_verified":false,"filesystem_sandbox_verified":false,
        "profile_loading":"unknown","effective_mode":"unknown","configuration_sources":"unknown",
        "builtin_catalog":"unknown","wrapper_closure":"unknown"}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "需要固定Grok、隔离无模型运行器与已构建生产监督者；接口候选尚未native验证"]
async fn native_fixed_policy_interfaces() {
    let (paths, plan, artifact) =
        Paths::load().expect("无模型调查输入未通过前置守卫；没有启用权限策略");
    let mut evidence = Evidence::open(&artifact).expect("无法打开私有白名单证据文件");
    let result = exercise(&paths, &plan, &mut evidence).await;
    if let Err(reason) = result {
        let _ = evidence.failure(reason.as_bytes());
        panic!("Grok 无模型接口调查失败；查看散列证据，不以清理或模板声明权限上限");
    }
}

#[cfg(test)]
#[path = "grok_policy_preflight_protocol_tests.rs"]
mod protocol_tests;
