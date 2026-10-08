//! tmux 专属启动先永久领取，再派发一次；重连只查询并重新证明当前 pane。

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::cli_image_codex_owned::Service as CodexService;
use super::cli_image_codex_owned_launch::{private_metadata, read_json, read_private, write_new};
use super::cli_image_codex_owned_protocol as codex;
use super::cli_image_grok::Service as GrokService;
use super::cli_image_grok_launch::digest;
use super::cli_image_grok_protocol as grok;
use super::cli_image_staging::TmuxImageRecovery;
use super::proto::{TerminalBindingOwned, TerminalBindingOwnedAgent, TerminalBindingScope};
use super::terminal_binding_service::Bound;
use super::tmux_native::{SplitOutcome, TmuxTarget};
use crate::terminal::{CLIAgent, cli_agent::discover_cli_agent_executable};

const MAX_AUTHORIZATIONS: usize = 4096;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    version: u32,
    id: Uuid,
    key_sha256: String,
    agent: i32,
    host: String,
    terminal_session: u64,
    terminal_epoch: Uuid,
    source: TmuxTarget,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    version: u32,
    id: Uuid,
    claim_sha256: String,
    phase: Phase,
    target: Option<TmuxTarget>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Started,
    Unknown,
    Failed,
}

struct Store {
    root: PathBuf,
    identity: (u64, u64),
}

impl Store {
    fn new(parent: &Path) -> io::Result<Self> {
        private_metadata(parent, true)?;
        let root = parent.join("tmux-owned-v1");
        match fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => File::open(parent)?.sync_all()?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = private_metadata(&root, true)?;
        Ok(Self {
            root,
            identity: (metadata.dev(), metadata.ino()),
        })
    }

    fn directory(&self, id: Uuid) -> io::Result<PathBuf> {
        let metadata = private_metadata(&self.root, true)?;
        if id.is_nil() || (metadata.dev(), metadata.ino()) != self.identity {
            return Err(invalid());
        }
        Ok(self.root.join(id.to_string()))
    }

    /// 目录创建就是永久领取点；即使后续 claim 写入中断，也绝不重用这个编号。
    fn claim(&self, claim: &Claim) -> io::Result<bool> {
        let directory = self.directory(claim.id)?;
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
            Err(error) => return Err(error),
        }
        File::open(&self.root)?.sync_all()?;
        let before = private_metadata(&directory, true)?;
        // 临时文件不授予派发；只有完整、持久的最终名才能成为重启后的 claim。
        write_new(&directory.join("claim.prepare.json"), claim)?;
        let parent = File::open(&directory)?;
        #[cfg(target_os = "macos")]
        let published = unsafe {
            libc::renameatx_np(
                parent.as_raw_fd(),
                c"claim.prepare.json".as_ptr(),
                parent.as_raw_fd(),
                c"claim.json".as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        let published = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                parent.as_raw_fd(),
                c"claim.prepare.json".as_ptr(),
                parent.as_raw_fd(),
                c"claim.json".as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if published != 0 {
            return Err(io::Error::last_os_error());
        }
        parent.sync_all()?;
        let after = private_metadata(&self.directory(claim.id)?, true)?;
        let opened = parent.metadata()?;
        if (before.dev(), before.ino()) != (after.dev(), after.ino())
            || (before.dev(), before.ino()) != (opened.dev(), opened.ino())
        {
            return Err(invalid());
        }
        if read_private(&directory.join("claim.json"))?
            != serde_json::to_vec(claim).map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        Ok(true)
    }

    fn load(&self, id: Uuid, key: Uuid, agent: TerminalBindingOwnedAgent) -> io::Result<Claim> {
        if key.is_nil() {
            return Err(invalid());
        }
        self.load_digest(id, &digest(key.as_bytes()), agent)
    }

    fn load_digest(
        &self,
        id: Uuid,
        key_sha256: &str,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<Claim> {
        let directory = self.directory(id)?;
        private_metadata(&directory, true)?;
        let claim: Claim = read_json(&directory.join("claim.json"))?;
        if claim.version != 1
            || claim.id != id
            || key_sha256.len() != 64
            || !key_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || claim.key_sha256 != key_sha256
            || agent == TerminalBindingOwnedAgent::Unspecified
            || claim.agent != agent as i32
            || claim.host.is_empty()
            || claim.host.len() > 512
            || claim.terminal_epoch.is_nil()
        {
            return Err(invalid());
        }
        Ok(claim)
    }

    fn validate_image_recovery(
        &self,
        recovery: &TmuxImageRecovery,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<()> {
        let claim = self.load_digest(recovery.launch_id, &recovery.launch_key_sha256, agent)?;
        if claim.recovery_identity() != *recovery {
            return Err(invalid());
        }
        Ok(())
    }

    fn finish(&self, claim: &Claim, phase: Phase, target: Option<TmuxTarget>) -> io::Result<()> {
        let directory = self.directory(claim.id)?;
        let claim_sha256 = digest(&read_private(&directory.join("claim.json"))?);
        write_new(
            &directory.join("outcome.json"),
            &Outcome {
                version: 1,
                id: claim.id,
                claim_sha256,
                phase,
                target,
            },
        )
    }

    fn finish_launch(&self, claim: &Claim, launched: io::Result<SplitOutcome>) -> io::Result<()> {
        let (phase, target) = match launched {
            Ok(SplitOutcome::Started(pane)) => match pane.export_target() {
                Ok(target) => (Phase::Started, Some(target)),
                Err(_) => (Phase::Unknown, None),
            },
            // split 的 Err 仅发生在控制写入前；已可能写出的错误由原生层转为 OutcomeUnknown。
            Ok(SplitOutcome::SelectionChanged) | Err(_) => (Phase::Failed, None),
            Ok(SplitOutcome::OutcomeUnknown) => (Phase::Unknown, None),
        };
        self.finish(claim, phase, target)
    }

    fn outcome(&self, claim: &Claim) -> io::Result<Outcome> {
        let directory = self.directory(claim.id)?;
        let value: Outcome = read_json(&directory.join("outcome.json"))?;
        if value.version != 1
            || value.id != claim.id
            || value.claim_sha256 != digest(&read_private(&directory.join("claim.json"))?)
            || (value.phase == Phase::Started) != value.target.is_some()
        {
            return Err(invalid());
        }
        Ok(value)
    }

    fn owns(&self, id: Uuid) -> io::Result<bool> {
        match fs::symlink_metadata(self.directory(id)?) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn owns_consumer(&self, pid: i32, tty: u64) -> io::Result<bool> {
        // 重启会清空内存授权，不能因客户端省略引用就绕过持久专属记录。
        self.directory(Uuid::new_v4())?;
        let started = Instant::now();
        for (index, entry) in fs::read_dir(&self.root)?.enumerate() {
            if index >= MAX_AUTHORIZATIONS || started.elapsed() >= Duration::from_secs(5) {
                return Err(invalid());
            }
            let entry = entry?;
            let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) else {
                return Err(invalid());
            };
            let directory = self.directory(id)?;
            private_metadata(&directory, true)?;
            let claim: Claim = match read_json(&directory.join("claim.json")) {
                Ok(claim) => claim,
                // 占位或临时文件从未许可 reserve/split，仍永久禁止复用编号。
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if claim.version != 1
                || claim.id != id
                || claim.terminal_epoch.is_nil()
                || !matches!(
                    TerminalBindingOwnedAgent::try_from(claim.agent),
                    Ok(TerminalBindingOwnedAgent::Claude
                        | TerminalBindingOwnedAgent::Codex
                        | TerminalBindingOwnedAgent::Grok)
                )
            {
                return Err(invalid());
            }
            let outcome = match self.outcome(&claim) {
                Ok(outcome) => Some(outcome),
                // 完整 claim 已知原 server；不完整回执仅限制这个准确相关的范围。
                Err(_) => None,
            };
            let Some(outcome) = outcome else {
                if claim.source.matches_server_consumer(pid, tty)? {
                    return Ok(true);
                }
                continue;
            };
            match outcome.target {
                Some(target) => {
                    if target.matches_consumer(pid, tty)? {
                        return Ok(true);
                    }
                }
                None if outcome.phase == Phase::Failed => {}
                None => {
                    if claim.source.matches_server_consumer(pid, tty)? {
                        return Ok(true);
                    }
                }
            }
        }
        self.directory(Uuid::new_v4())?;
        Ok(false)
    }
}

#[derive(Clone)]
pub(super) struct OriginalScope {
    host: String,
    terminal_session: u64,
}

impl OriginalScope {
    pub(super) fn host(&self) -> &str {
        &self.host
    }
    pub(super) fn terminal_session(&self) -> u64 {
        self.terminal_session
    }
    pub(super) fn matches(&self, host: &str, session: u64) -> bool {
        self.host == host && self.terminal_session == session
    }
}

#[derive(Clone)]
pub(super) struct ImageGuard {
    bound: Bound,
    target: TmuxTarget,
    original: OriginalScope,
    launch: Uuid,
    agent: TerminalBindingOwnedAgent,
    recovery: TmuxImageRecovery,
}

impl ImageGuard {
    pub(super) fn recovery_identity(&self) -> io::Result<TmuxImageRecovery> {
        self.validate()?;
        Ok(self.recovery.clone())
    }

    pub(super) fn validate_ticket(
        &self,
        host: &str,
        session: u64,
        id: Uuid,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<()> {
        if id != self.launch || agent != self.agent || !self.original.matches(host, session) {
            return Err(invalid());
        }
        self.validate()
    }
    pub(super) fn validate(&self) -> io::Result<()> {
        self.bound.snapshot()?.validate_target(&self.target)
    }

    pub(super) fn validate_process(&self, pid: i32, tty: u64) -> io::Result<()> {
        self.validate()?;
        self.target.validate_consumer(pid, tty)?;
        self.validate()
    }
}

#[derive(Hash, PartialEq, Eq)]
struct AuthorizationKey {
    session: u64,
    epoch: Uuid,
    launch: Uuid,
    agent: i32,
}

struct Authorization {
    guard: ImageGuard,
    original: OriginalScope,
    key_sha256: String,
}

pub(super) struct Service {
    host: String,
    parent: PathBuf,
    store: Mutex<Option<Arc<Store>>>,
    codex: Arc<CodexService>,
    grok: Arc<GrokService>,
    authorizations: Mutex<HashMap<AuthorizationKey, Authorization>>,
}

impl Service {
    /// 初始化只保存配置；目录与原生进程查询均延迟到后台 Work。
    pub(super) fn new(
        host: String,
        parent: &Path,
        codex: Arc<CodexService>,
        grok: Arc<GrokService>,
    ) -> Self {
        Self {
            host,
            parent: parent.into(),
            store: Mutex::new(None),
            codex,
            grok,
            authorizations: Mutex::new(HashMap::new()),
        }
    }

    fn store(&self) -> io::Result<Arc<Store>> {
        let mut store = self.store.lock().map_err(|_| invalid())?;
        if let Some(store) = store.as_ref() {
            return Ok(store.clone());
        }
        let value = Arc::new(Store::new(&self.parent)?);
        *store = Some(value.clone());
        Ok(value)
    }

    /// 专属记录即使尚未完成，也不能降级成不带 tmux guard 的普通图片票据。
    pub(super) fn owns_ticket(&self, id: Uuid) -> io::Result<bool> {
        self.store()?.owns(id)
    }

    pub(super) fn owns_consumer(&self, pid: i32, tty: u64) -> io::Result<bool> {
        self.store()?.owns_consumer(pid, tty)
    }

    /// 已派发输入的 ACK/回收只核原票据，不把查历史结果变成新的 pane 输入授权。
    pub(super) fn ticket_terminal(
        &self,
        scope: &TerminalBindingScope,
        launch: Uuid,
        key: Uuid,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<OriginalScope> {
        self.authorization_key(scope, launch, agent)?;
        let claim = self.store()?.load(launch, key, agent)?;
        // 原票据凭证只允许读取结果和精确回收；目标退出也不阻断，且不发布输入授权。
        Ok(claim.original_scope())
    }

    pub(super) fn validate_image_recovery(
        &self,
        scope: &TerminalBindingScope,
        recovery: &TmuxImageRecovery,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<()> {
        self.authorization_key(scope, recovery.launch_id, agent)?;
        self.store()?.validate_image_recovery(recovery, agent)
    }

    fn authorization_key(
        &self,
        scope: &TerminalBindingScope,
        launch: Uuid,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<AuthorizationKey> {
        let epoch = Uuid::parse_str(&scope.terminal_epoch).map_err(|_| invalid())?;
        if scope.host_id != self.host
            || epoch.is_nil()
            || launch.is_nil()
            || agent == TerminalBindingOwnedAgent::Unspecified
        {
            return Err(invalid());
        }
        Ok(AuthorizationKey {
            session: scope.terminal_session_id,
            epoch,
            launch,
            agent: agent as i32,
        })
    }

    pub(super) fn image_guard(
        &self,
        scope: &TerminalBindingScope,
        launch: Uuid,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<(ImageGuard, OriginalScope)> {
        let key = self.authorization_key(scope, launch, agent)?;
        let (guard, original) = {
            let authorizations = self.authorizations.lock().map_err(|_| invalid())?;
            let value = authorizations.get(&key).ok_or_else(invalid)?;
            (value.guard.clone(), value.original.clone())
        };
        guard.validate()?;
        Ok((guard, original))
    }

    pub(super) fn image_guard_for_reference(
        &self,
        scope: &TerminalBindingScope,
        launch: Uuid,
        launch_key: Uuid,
        binding_id: &str,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<(ImageGuard, OriginalScope)> {
        let key = self.authorization_key(scope, launch, agent)?;
        let (guard, original) = {
            let authorizations = self.authorizations.lock().map_err(|_| invalid())?;
            let value = authorizations.get(&key).ok_or_else(invalid)?;
            if launch_key.is_nil()
                || value.key_sha256 != digest(launch_key.as_bytes())
                || !value.guard.bound.has_id(binding_id)
            {
                return Err(invalid());
            }
            (value.guard.clone(), value.original.clone())
        };
        guard.validate()?;
        Ok((guard, original))
    }

    pub(super) fn execute(
        &self,
        bound: Bound,
        scope: &TerminalBindingScope,
        id: Uuid,
        key: Uuid,
        agent: TerminalBindingOwnedAgent,
        start: bool,
    ) -> io::Result<TerminalBindingOwned> {
        if scope.host_id != self.host
            || id.is_nil()
            || key.is_nil()
            || agent == TerminalBindingOwnedAgent::Unspecified
        {
            return Err(invalid());
        }
        let store = self.store()?;
        if start && !store.owns(id)? {
            let claim = Claim {
                version: 1,
                id,
                key_sha256: digest(key.as_bytes()),
                agent: agent as i32,
                host: self.host.clone(),
                terminal_session: scope.terminal_session_id,
                terminal_epoch: Uuid::parse_str(&scope.terminal_epoch).map_err(|_| invalid())?,
                source: bound.source_target()?,
            };
            if store.claim(&claim)? {
                // 从此无论 reserve、split 或回执落盘失败，所有后续请求都只能查这个编号。
                let launched = self.launch(&bound, &claim, key, agent);
                // 不完整 outcome 保持 unknown；绝不以写失败作为重新 split 的依据。
                let _ = store.finish_launch(&claim, launched);
            }
        }
        self.query(&store, bound, scope, id, key, agent)
    }

    fn launch(
        &self,
        bound: &Bound,
        claim: &Claim,
        key: Uuid,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<SplitOutcome> {
        let cwd = bound.cwd().to_str().ok_or_else(invalid)?;
        let argv = match agent {
            TerminalBindingOwnedAgent::Claude => {
                let executable = discover_cli_agent_executable(CLIAgent::Claude)
                    .ok_or_else(invalid)?
                    .canonicalize()?;
                vec![executable.into_os_string()]
            }
            TerminalBindingOwnedAgent::Codex => match self.codex.reserve_tmux(
                &claim.codex_scope(),
                &codex::Ticket { id: claim.id, key },
                cwd,
            )? {
                codex::Reply::Reserved { argv, .. } => {
                    argv.into_iter().map(OsString::from).collect()
                }
                codex::Reply::Launch { .. }
                | codex::Reply::Observed { .. }
                | codex::Reply::Revoked { .. }
                | codex::Reply::Failed { .. } => return Err(invalid()),
            },
            TerminalBindingOwnedAgent::Grok => match self.grok.reserve_tmux(
                &claim.grok_scope(),
                &grok::Ticket { id: claim.id, key },
                cwd,
            )? {
                grok::Reply::Reserved { argv, .. } => {
                    argv.into_iter().map(OsString::from).collect()
                }
                grok::Reply::Launch { .. }
                | grok::Reply::OwnedIdentity { .. }
                | grok::Reply::Input { .. }
                | grok::Reply::Revoked { .. }
                | grok::Reply::Failed { .. } => return Err(invalid()),
            },
            TerminalBindingOwnedAgent::Unspecified => return Err(invalid()),
        };
        let (program, args) = argv.split_first().ok_or_else(invalid)?;
        let result = bound.split_owned(Path::new(program), args);
        if matches!(result, Ok(SplitOutcome::SelectionChanged) | Err(_)) {
            // 取消只封闭尚未进入的 wrapper；已派发进程由原生退出和既有回收链处理。
            match agent {
                TerminalBindingOwnedAgent::Codex => {
                    let _ = self
                        .codex
                        .cancel_tmux(&claim.codex_scope(), &codex::Ticket { id: claim.id, key });
                }
                TerminalBindingOwnedAgent::Grok => {
                    let _ = self
                        .grok
                        .cancel_tmux(&claim.grok_scope(), &grok::Ticket { id: claim.id, key });
                }
                TerminalBindingOwnedAgent::Claude | TerminalBindingOwnedAgent::Unspecified => {}
            }
        }
        result
    }

    fn query(
        &self,
        store: &Store,
        bound: Bound,
        scope: &TerminalBindingScope,
        id: Uuid,
        key: Uuid,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<TerminalBindingOwned> {
        let mut reply = TerminalBindingOwned {
            launch_id: id.to_string(),
            agent: agent as i32,
            phase: "unknown".into(),
            owned_reply_json: Vec::new(),
            native_session_id: None,
        };
        let claim = match store.load(id, key, agent) {
            Ok(claim) => claim,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(reply),
            Err(error) => return Err(error),
        };
        let outcome = match store.outcome(&claim) {
            Ok(outcome) => outcome,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(reply),
            Err(error) => return Err(error),
        };
        let Some(target) = outcome.target else {
            if outcome.phase == Phase::Failed {
                reply.phase = "failed".into();
            }
            // unknown 不恢复票据或输入授权；当前 pane 可能正是没有完整回执的新 pane。
            return Ok(reply);
        };
        let original = claim.original_scope();
        // 先读取受保护原票据的终态；退出后的 pane 无需重新获得输入授权。
        let (phase, bytes) = match agent {
            TerminalBindingOwnedAgent::Claude => ("started", Vec::new()),
            TerminalBindingOwnedAgent::Codex => {
                let value = self.codex.status_tmux(
                    &claim.codex_scope(),
                    &codex::Ticket { id, key },
                    &original,
                )?;
                let phase = match &value {
                    codex::Reply::Launch { phase, owner, .. } => {
                        owned_phase(phase, owner.is_some())
                    }
                    codex::Reply::Reserved { .. }
                    | codex::Reply::Observed { .. }
                    | codex::Reply::Revoked { .. }
                    | codex::Reply::Failed { .. } => return Err(invalid()),
                };
                (phase, codex::encode(&value).map_err(|_| invalid())?)
            }
            TerminalBindingOwnedAgent::Grok => {
                let value = self.grok.status_tmux(
                    &claim.grok_scope(),
                    &grok::Ticket { id, key },
                    &original,
                )?;
                let phase = match &value {
                    grok::Reply::Launch { phase, .. } => owned_phase(phase, false),
                    grok::Reply::Reserved { .. }
                    | grok::Reply::OwnedIdentity { .. }
                    | grok::Reply::Input { .. }
                    | grok::Reply::Revoked { .. }
                    | grok::Reply::Failed { .. } => return Err(invalid()),
                };
                (phase, grok::encode(&value).map_err(|_| invalid())?)
            }
            TerminalBindingOwnedAgent::Unspecified => return Err(invalid()),
        };
        reply.phase = phase.into();
        reply.owned_reply_json = bytes;
        if !matches!(phase, "started" | "running") {
            return Ok(reply);
        }
        let guard = ImageGuard {
            bound,
            target,
            original: original.clone(),
            launch: id,
            agent,
            recovery: claim.recovery_identity(),
        };
        guard.validate()?;
        if agent == TerminalBindingOwnedAgent::Claude {
            reply.native_session_id =
                super::cli_image_native_claude_binding::session_for_tmux(&guard.target)?
                    .map(|id| id.to_string());
            guard.validate()?;
        }
        let authorization_key = self.authorization_key(scope, id, agent)?;
        let mut authorizations = self.authorizations.lock().map_err(|_| invalid())?;
        authorizations.retain(|_, value| value.guard.bound.is_live());
        if !authorizations.contains_key(&authorization_key)
            && authorizations.len() >= MAX_AUTHORIZATIONS
        {
            return Err(invalid());
        }
        authorizations.insert(
            authorization_key,
            Authorization {
                guard,
                original,
                key_sha256: claim.key_sha256,
            },
        );
        Ok(reply)
    }
}

impl Claim {
    fn recovery_identity(&self) -> TmuxImageRecovery {
        TmuxImageRecovery {
            version: 1,
            launch_id: self.id,
            launch_key_sha256: self.key_sha256.clone(),
            agent: self.agent,
            original_host: self.host.clone(),
            original_terminal_session: self.terminal_session,
        }
    }

    fn original_scope(&self) -> OriginalScope {
        OriginalScope {
            host: self.host.clone(),
            terminal_session: self.terminal_session,
        }
    }
    fn codex_scope(&self) -> codex::Scope {
        codex::Scope {
            host: self.host.clone(),
            terminal_session: self.terminal_session,
            terminal_epoch: self.terminal_epoch,
            generation: self.id,
        }
    }
    fn grok_scope(&self) -> grok::Scope {
        grok::Scope {
            host: self.host.clone(),
            terminal_session: self.terminal_session,
            terminal_epoch: self.terminal_epoch,
            generation: self.id,
        }
    }
}

fn owned_phase(phase: &str, owner: bool) -> &'static str {
    match phase {
        "released" | "cancelled" => "exited",
        "failed" => "failed",
        "active" if owner => "running",
        "reserved" | "entered_unknown" | "dispatched_unknown" => "started",
        _ => "unknown",
    }
}

fn invalid() -> io::Error {
    io::Error::other("tmux 专属启动记录或当前目标无效")
}

#[cfg(test)]
#[path = "tmux_owned_tests.rs"]
mod tests;
