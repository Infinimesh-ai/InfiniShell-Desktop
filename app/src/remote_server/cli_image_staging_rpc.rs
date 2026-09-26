//! 未投递图片暂存的连接级协议；原生 CLI 消费与发布文件寿命不在此能力内。

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use uuid::Uuid;
use warp_core::{HostId, SessionId};

use super::cli_image_codex_binding::Binding;
use super::cli_image_codex_queue::{CodexImageQueue, CodexQueueImage};
use super::cli_image_staging::{
    QueueClaim, QueueResult, RemoteImageScope, RemoteImageSpec, RemoteImageStaging,
};
use super::proto::{
    CliImageCodexQueue, CliImageCodexQueueResult, CliImageStagingActivated,
    CliImageStagingCancelled, CliImageStagingError, CliImageStagingErrorCode,
    CliImageStagingProgress, CliImageStagingPublished, CliImageStagingRecovered,
    CliImageStagingReleased, CliImageStagingRequest, CliImageStagingResponse,
    CliImageStagingRevoked, CliImageStagingScope, CliImageStagingSpec, CliImageStagingVerified,
    cli_image_staging_request, cli_image_staging_response,
};

// 服务端全局上限沿用普通 CLI 单张图片字节上限；产品可在前端施加更小预算。
#[path = "cli_image_claude_dispatch.rs"]
mod claude;

const MAX_RESERVED_BYTES: u64 = 500_000_000;

pub(super) struct ImageStagingConnection {
    id: Uuid,
    live: AtomicBool,
    initialized: AtomicBool,
    sessions: Mutex<HashMap<SessionId, TerminalRegistration>>,
}

struct TerminalRegistration {
    epoch: Uuid,
    revision: u64,
    submission: Option<Arc<SubmissionLease>>,
}

struct SubmissionLease {
    scope: CliImageStagingScope,
    revision: u64,
    live: AtomicBool,
    native_claimed: Mutex<bool>,
}

impl SubmissionLease {
    fn revoke(&self) {
        let _guard = self
            .native_claimed
            .lock()
            .expect("image native claim poisoned");
        self.live.store(false, Ordering::Release);
    }

    fn claim_native(&self, connection: &ImageStagingConnection) -> io::Result<()> {
        let mut claimed = self
            .native_claimed
            .lock()
            .expect("image native claim poisoned");
        if *claimed
            || !self.live.load(Ordering::Acquire)
            || !connection.live.load(Ordering::Acquire)
        {
            return Err(io::Error::other("image native claim was revoked"));
        }
        *claimed = true;
        Ok(())
    }
}

impl ImageStagingConnection {
    pub(super) fn new(id: Uuid) -> Self {
        Self {
            id,
            live: AtomicBool::new(true),
            initialized: AtomicBool::new(false),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn initialize(&self) {
        self.initialized.store(true, Ordering::Release);
    }

    /// 实例版本由同一连接同步通知；重复旧通知不能重新打开已撤销的上传。
    pub(super) fn bootstrap(
        &self,
        session_id: SessionId,
        epoch: &str,
        revision: u64,
    ) -> Option<CliImageStagingScope> {
        let Ok(epoch) = parse_scope_uuid(epoch) else {
            return None;
        };
        if revision == 0 {
            return None;
        }
        let mut sessions = self
            .sessions
            .lock()
            .expect("image staging sessions lock poisoned");
        if sessions
            .get(&session_id)
            .is_some_and(|current| current.revision >= revision)
        {
            return None;
        }
        let previous = sessions.insert(
            session_id,
            TerminalRegistration {
                epoch,
                revision,
                submission: None,
            },
        );
        previous
            .and_then(|previous| previous.submission)
            .map(|lease| {
                lease.revoke();
                lease.scope.clone()
            })
    }

    /// 模型线程同步撤销，不等待磁盘锁或后台工作；已领取的字节也不成为原生权限。
    pub(super) fn revoke(&self) {
        self.live.store(false, Ordering::Release);
        for terminal in self
            .sessions
            .lock()
            .expect("image staging sessions lock poisoned")
            .values()
        {
            if let Some(lease) = &terminal.submission {
                lease.revoke();
            }
        }
    }

    pub(super) fn revoke_request(&self, request: &CliImageStagingRequest) {
        if matches!(
            request.action,
            Some(cli_image_staging_request::Action::Revoke(_))
        ) {
            let _ = self.admit(request);
        }
    }

    fn admit(
        &self,
        request: &CliImageStagingRequest,
    ) -> Result<Arc<SubmissionLease>, CliImageStagingError> {
        use cli_image_staging_request::Action;
        if !self.live.load(Ordering::Acquire) || !self.initialized.load(Ordering::Acquire) {
            return Err(error(
                CliImageStagingErrorCode::InvalidScope,
                "image connection is unavailable",
            ));
        }
        let scope = request.scope.as_ref().ok_or_else(|| {
            error(
                CliImageStagingErrorCode::InvalidScope,
                "image scope is missing",
            )
        })?;
        let epoch = parse_scope_uuid(&scope.terminal_epoch)?;
        let mut sessions = self
            .sessions
            .lock()
            .expect("image staging sessions lock poisoned");
        let terminal = sessions
            .get_mut(&SessionId::from(scope.terminal_session_id))
            .ok_or_else(|| {
                error(
                    CliImageStagingErrorCode::InvalidScope,
                    "terminal is not registered",
                )
            })?;
        if terminal.epoch != epoch {
            return Err(error(
                CliImageStagingErrorCode::StaleScope,
                "terminal instance is stale",
            ));
        }
        let action = request.action.as_ref().ok_or_else(|| {
            error(
                CliImageStagingErrorCode::InvalidTransfer,
                "image action is missing",
            )
        })?;
        let revision = if matches!(action, Action::Activate(_)) {
            request.revision.checked_add(1).ok_or_else(|| {
                error(
                    CliImageStagingErrorCode::StaleScope,
                    "image revision overflow",
                )
            })?
        } else {
            request.revision
        };
        if revision == 0 {
            return Err(error(
                CliImageStagingErrorCode::StaleScope,
                "image revision is empty",
            ));
        }
        let new_lease = || {
            Arc::new(SubmissionLease {
                scope: scope.clone(),
                revision,
                live: AtomicBool::new(matches!(action, Action::Activate(_))),
                native_claimed: Mutex::new(false),
            })
        };
        // 已发布引用只允许精确释放，不赋予上传、发布或原生执行资格。
        if matches!(
            action,
            Action::Release(_)
                | Action::Recover(_)
                | Action::CodexQueueStatus(_)
                | Action::ClaudeQueueStatus(_)
        ) {
            return Ok(new_lease());
        }
        if let Some(current) = &terminal.submission {
            if current.revision > revision {
                return if matches!(action, Action::Revoke(_)) {
                    Ok(new_lease())
                } else {
                    Err(error(
                        CliImageStagingErrorCode::StaleScope,
                        "image submission is stale",
                    ))
                };
            }
            if current.revision == revision {
                if current.scope != *scope {
                    return Err(error(
                        CliImageStagingErrorCode::StaleScope,
                        "image submission identity changed",
                    ));
                }
                if matches!(action, Action::Revoke(_)) {
                    current.revoke();
                    return Ok(current.clone());
                }
                if current.live.load(Ordering::Acquire) {
                    return Ok(current.clone());
                }
                return Err(error(
                    CliImageStagingErrorCode::StaleScope,
                    "image submission was revoked",
                ));
            }
        }
        if !matches!(action, Action::Activate(_) | Action::Revoke(_)) {
            return Err(error(
                CliImageStagingErrorCode::StaleScope,
                "image submission is not active",
            ));
        }
        let lease = new_lease();
        if let Some(previous) = terminal.submission.replace(lease.clone()) {
            previous.revoke();
        }
        Ok(lease)
    }
}

struct Registration {
    scope: RemoteImageScope,
    previous_revision: u64,
    active: bool,
    revision: u64,
}

struct State {
    staging: RemoteImageStaging,
    registrations: HashMap<(Uuid, SessionId), Registration>,
}

pub(super) struct ImageStagingService {
    #[cfg(all(
        feature = "local_fs",
        any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64")
        )
    ))]
    codex_owned: Option<Arc<super::cli_image_codex_owned::Service>>,
    host_id: HostId,
    state: Mutex<State>,
}

impl ImageStagingService {
    #[cfg(all(
        feature = "local_fs",
        any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64")
        )
    ))]
    pub(super) fn with_codex_owned(
        mut self,
        service: Option<Arc<super::cli_image_codex_owned::Service>>,
    ) -> Self {
        self.codex_owned = service;
        self
    }

    fn connect_codex(
        &self,
        scope: &RemoteImageScope,
        candidate: &super::proto::CliImageCodexBinding,
    ) -> io::Result<(NativeCodexQueueBinding, std::os::unix::net::UnixStream)> {
        if candidate.owned_ticket_id.is_empty() && candidate.owned_manifest_sha256.is_empty() {
            return Binding::connect(candidate)
                .map(|(binding, stream)| (NativeCodexQueueBinding::Shared(binding), stream));
        }
        #[cfg(all(
            feature = "local_fs",
            any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "x86_64")
            )
        ))]
        {
            use std::os::unix::fs::{FileTypeExt, MetadataExt};
            let service = self
                .codex_owned
                .as_ref()
                .ok_or_else(|| io::Error::other("owned Codex unavailable"))?;
            let ticket = Uuid::parse_str(&candidate.owned_ticket_id)
                .map_err(|_| io::Error::other("invalid Codex ticket"))?;
            let scope = super::cli_image_codex_owned_protocol::Scope {
                host: scope.host_id.as_str().to_owned(),
                terminal_session: scope.terminal_session_id.into(),
                terminal_epoch: scope.terminal_epoch,
                generation: scope.input_generation,
            };
            let lease = service.image_owner(&scope, ticket, &candidate.owned_manifest_sha256)?;
            let tty = std::fs::symlink_metadata(&candidate.tty_path)?;
            if lease.server_pid() as u32 != candidate.daemon_pid_candidate
                || lease.codex_home() != std::fs::canonicalize(&candidate.codex_home)?
                || lease.cwd() != std::fs::canonicalize(&candidate.working_directory)?
                || !tty.file_type().is_char_device()
                || tty.rdev() != lease.tty_device()
                || tty.uid() != unsafe { libc::geteuid() }
            {
                return Err(io::Error::other("owned Codex candidate changed"));
            }
            let stream = std::os::unix::net::UnixStream::connect(lease.socket_path())?;
            lease.validate(&stream)?;
            Ok((NativeCodexQueueBinding::Owned(lease), stream))
        }
        #[cfg(not(all(
            feature = "local_fs",
            any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "x86_64")
            )
        )))]
        {
            let _ = scope;
            Err(io::Error::other("owned Codex platform is unavailable"))
        }
    }

    pub(super) fn new(host_id: HostId, private_parent: &Path) -> io::Result<Self> {
        let staging = RemoteImageStaging::new(host_id.clone(), private_parent, MAX_RESERVED_BYTES)?;
        Ok(Self {
            host_id,
            #[cfg(all(
                feature = "local_fs",
                any(
                    all(target_os = "macos", target_arch = "aarch64"),
                    all(target_os = "linux", target_arch = "x86_64")
                )
            ))]
            codex_owned: None,
            state: Mutex::new(State {
                staging,
                registrations: HashMap::new(),
            }),
        })
    }

    /// 仅后台调用：哈希和文件读写不能占用 UI/模型线程。
    pub(super) fn handle(
        &self,
        connection: &ImageStagingConnection,
        request: CliImageStagingRequest,
    ) -> CliImageStagingResponse {
        let result = self.execute(connection, &request);
        match result {
            Ok((revision, result)) => CliImageStagingResponse {
                scope: request.scope,
                revision,
                result: Some(result),
            },
            Err(error) => CliImageStagingResponse {
                scope: request.scope,
                revision: request.revision,
                result: Some(cli_image_staging_response::Result::Error(error)),
            },
        }
    }

    fn execute(
        &self,
        connection: &ImageStagingConnection,
        request: &CliImageStagingRequest,
    ) -> Result<(u64, cli_image_staging_response::Result), CliImageStagingError> {
        use cli_image_staging_request::Action;
        use cli_image_staging_response::Result as Response;
        let scope = parse_scope(request.scope.as_ref(), connection.id, &self.host_id)?;
        let lease = connection.admit(request)?;
        let cleanup = matches!(
            request.action,
            Some(
                Action::Revoke(_)
                    | Action::Release(_)
                    | Action::Recover(_)
                    | Action::CodexQueueStatus(_)
                    | Action::ClaudeQueueStatus(_)
            )
        );
        let mut state = self
            .state
            .lock()
            .expect("image staging state lock poisoned");
        if !connection.live.load(Ordering::Acquire)
            || (!cleanup && !lease.live.load(Ordering::Acquire))
        {
            return Err(error(
                CliImageStagingErrorCode::Disconnected,
                "image staging connection was revoked",
            ));
        }
        let key = (connection.id, scope.terminal_session_id);
        let action = request.action.as_ref().ok_or_else(|| {
            error(
                CliImageStagingErrorCode::InvalidTransfer,
                "image staging action is missing",
            )
        })?;
        let output = match action {
            Action::Activate(_) => {
                if let Some(previous) = state.registrations.get(&key) {
                    if previous.active
                        && previous.scope == scope
                        && previous.previous_revision == request.revision
                    {
                        return Ok((
                            previous.revision,
                            Response::Activated(CliImageStagingActivated {}),
                        ));
                    }
                    if previous.revision > request.revision && previous.scope == scope {
                        return Err(error(
                            CliImageStagingErrorCode::StaleScope,
                            "image staging activation revision is stale",
                        ));
                    }
                }
                let revision = request.revision.checked_add(1).ok_or_else(|| {
                    error(
                        CliImageStagingErrorCode::StaleScope,
                        "image staging revision overflow",
                    )
                })?;
                // 先清理并登记内核，再推进 CAS；失败不能留下已确认的新版本。
                if let Some(previous) = state.registrations.get_mut(&key) {
                    previous.active = false;
                }
                state
                    .staging
                    .activate_scope(scope.clone())
                    .map_err(storage_error)?;
                state.registrations.insert(
                    key,
                    Registration {
                        scope,
                        previous_revision: request.revision,
                        revision,
                        active: true,
                    },
                );
                (revision, Response::Activated(CliImageStagingActivated {}))
            }
            Action::Begin(begin) => {
                require_registration(&state, key, &scope, request.revision)?;
                let transfer = transfer_id(&begin.transfer_id)?;
                let spec = parse_spec(begin.spec.as_ref())?;
                let next_offset = state
                    .staging
                    .begin(&scope, transfer, spec)
                    .map_err(storage_error)?;
                (
                    request.revision,
                    Response::Progress(CliImageStagingProgress {
                        transfer_id: transfer.to_string(),
                        next_offset,
                    }),
                )
            }
            Action::Chunk(chunk) => {
                require_registration(&state, key, &scope, request.revision)?;
                let transfer = transfer_id(&chunk.transfer_id)?;
                let next_offset = state
                    .staging
                    .write_chunk(&scope, transfer, chunk.offset, &chunk.bytes)
                    .map_err(storage_error)?;
                (
                    request.revision,
                    Response::Progress(CliImageStagingProgress {
                        transfer_id: transfer.to_string(),
                        next_offset,
                    }),
                )
            }
            Action::Verify(verify) => {
                require_registration(&state, key, &scope, request.revision)?;
                let transfer = transfer_id(&verify.transfer_id)?;
                let expected = parse_spec(verify.expected_spec.as_ref())?;
                let verified = state
                    .staging
                    .verify(&scope, transfer)
                    .map_err(storage_error)?;
                if verified.spec != expected {
                    return Err(error(
                        CliImageStagingErrorCode::InvalidTransfer,
                        "image staging expected spec does not match",
                    ));
                }
                (
                    request.revision,
                    Response::Verified(CliImageStagingVerified {
                        transfer_id: transfer.to_string(),
                        spec: Some(CliImageStagingSpec {
                            byte_len: verified.spec.byte_len,
                            sha256: verified.spec.sha256.to_vec(),
                        }),
                    }),
                )
            }
            Action::Revoke(_) => {
                if let Some(previous) = state.registrations.get_mut(&key)
                    && previous.scope == scope
                    && previous.revision == request.revision
                {
                    previous.active = false;
                }
                state.staging.revoke_scope(&scope).map_err(storage_error)?;
                (
                    request.revision,
                    Response::Revoked(CliImageStagingRevoked {}),
                )
            }
            Action::Publish(publish) => {
                require_registration(&state, key, &scope, request.revision)?;
                let transfer = transfer_id(&publish.transfer_id)?;
                let (path, spec) = state
                    .staging
                    .publish(&scope, transfer, parse_scope_uuid(&publish.recovery_key)?)
                    .map_err(storage_error)?;
                let path = path.to_str().ok_or_else(|| {
                    error(
                        CliImageStagingErrorCode::StorageFailure,
                        "image path is not UTF-8",
                    )
                })?;
                (
                    request.revision,
                    Response::Published(CliImageStagingPublished {
                        transfer_id: transfer.to_string(),
                        remote_path: path.to_owned(),
                        spec: Some(CliImageStagingSpec {
                            byte_len: spec.byte_len,
                            sha256: spec.sha256.to_vec(),
                        }),
                    }),
                )
            }
            Action::Release(release) => {
                let transfer = transfer_id(&release.transfer_id)?;
                state
                    .staging
                    .release_published(&scope, transfer, parse_scope_uuid(&release.recovery_key)?)
                    .map_err(storage_error)?;
                (
                    request.revision,
                    Response::Released(CliImageStagingReleased {
                        transfer_id: transfer.to_string(),
                    }),
                )
            }
            Action::Cancel(cancel) => {
                require_registration(&state, key, &scope, request.revision)?;
                let transfer = transfer_id(&cancel.transfer_id)?;
                state
                    .staging
                    .cancel(&scope, transfer)
                    .map_err(storage_error)?;
                (
                    request.revision,
                    Response::Cancelled(CliImageStagingCancelled {
                        transfer_id: transfer.to_string(),
                    }),
                )
            }
            Action::Recover(recover) => {
                let transfer = transfer_id(&recover.transfer_id)?;
                let (reference, released) = state
                    .staging
                    .recover_reference(
                        &scope,
                        transfer,
                        parse_scope_uuid(&recover.recovery_key)?,
                        recover.release,
                    )
                    .map_err(storage_error)?;
                let reference = reference
                    .map(|(path, spec)| {
                        let remote_path = path
                            .to_str()
                            .ok_or_else(|| {
                                error(
                                    CliImageStagingErrorCode::StorageFailure,
                                    "image path is not UTF-8",
                                )
                            })?
                            .to_owned();
                        Ok::<_, CliImageStagingError>(CliImageStagingPublished {
                            transfer_id: transfer.to_string(),
                            remote_path,
                            spec: Some(CliImageStagingSpec {
                                byte_len: spec.byte_len,
                                sha256: spec.sha256.to_vec(),
                            }),
                        })
                    })
                    .transpose()?;
                (
                    request.revision,
                    Response::Recovered(CliImageStagingRecovered {
                        transfer_id: transfer.to_string(),
                        reference,
                        released,
                    }),
                )
            }
            Action::ClaudeQueue(queue) => {
                require_registration(&state, key, &scope, request.revision)?;
                return Ok((
                    request.revision,
                    Response::ClaudeQueue(claude::submit(
                        &mut state.staging,
                        connection,
                        &lease,
                        &scope,
                        queue,
                    )?),
                ));
            }
            Action::ClaudeQueueStatus(status) => {
                let result = claude::recover(
                    &mut state.staging,
                    &scope,
                    parse_scope_uuid(&status.submission_id)?,
                    parse_scope_uuid(&status.recovery_key)?,
                )?;
                (request.revision, Response::ClaudeQueue(result))
            }
            Action::CodexQueue(queue) => {
                require_registration(&state, key, &scope, request.revision)?;
                // 原生请求的明确接收可在撤销后收尾；不能因旧 UI 代际丢弃这份精确回执。
                return Ok((
                    request.revision,
                    Response::CodexQueue(queue_input(
                        self,
                        &mut state.staging,
                        connection,
                        &lease,
                        &scope,
                        queue,
                    )?),
                ));
            }
            Action::CodexQueueStatus(status) => {
                let submission = parse_scope_uuid(&status.submission_id)?;
                let result = state
                    .staging
                    .queue_status(&scope, submission, parse_scope_uuid(&status.recovery_key)?)
                    .map_err(storage_error)?;
                if result
                    .as_ref()
                    .is_some_and(|result| result.claim.claude_recovery.is_some())
                {
                    return Err(storage_error(io::Error::other(
                        "native image consumer changed",
                    )));
                }
                if let Some(result) = &result
                    && matches!(result.status.as_str(), "confirmed" | "rejected")
                {
                    state
                        .staging
                        .finish_queue(&scope, result)
                        .map_err(storage_error)?;
                }
                (
                    request.revision,
                    Response::CodexQueue(queue_response(submission, result)),
                )
            }
        };
        if !connection.live.load(Ordering::Acquire)
            || (!cleanup && !lease.live.load(Ordering::Acquire))
        {
            return Err(error(
                CliImageStagingErrorCode::Disconnected,
                "image staging connection was revoked",
            ));
        }
        Ok(output)
    }

    /// bootstrap 已同步撤销旧 lease；只清理其精确作用域，不能删除新实例的上传。
    pub(super) fn retire_scope(
        &self,
        connection_id: Uuid,
        scope: &CliImageStagingScope,
    ) -> io::Result<()> {
        let scope = parse_scope(Some(scope), connection_id, &self.host_id)
            .map_err(|_| io::Error::other("invalid retired image scope"))?;
        let mut state = self
            .state
            .lock()
            .expect("image staging state lock poisoned");
        if let Some(previous) = state
            .registrations
            .get_mut(&(connection_id, scope.terminal_session_id))
            && previous.scope == scope
        {
            previous.active = false;
        }
        state.staging.revoke_scope(&scope)
    }

    /// revoke 必须先在模型线程完成；该函数只负责后台磁盘回收。
    pub(super) fn disconnect(&self, connection_id: Uuid) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .expect("image staging state lock poisoned");
        state
            .registrations
            .retain(|(id, _), _| *id != connection_id);
        state.staging.disconnect(connection_id)
    }
}

fn queue_response(submission: Uuid, result: Option<QueueResult>) -> CliImageCodexQueueResult {
    match result {
        Some(result) => CliImageCodexQueueResult {
            submission_id: submission.to_string(),
            subject_sha256: result.claim.subject.to_vec(),
            status: result.status,
            native_queue_id: result.native_queue_id,
        },
        None => CliImageCodexQueueResult {
            submission_id: submission.to_string(),
            subject_sha256: Vec::new(),
            status: "absent".to_owned(),
            native_queue_id: String::new(),
        },
    }
}

enum NativeCodexQueueBinding {
    Shared(Binding),
    #[cfg(all(
        feature = "local_fs",
        any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64")
        )
    ))]
    Owned(super::cli_image_codex_owned_launch::OwnedImageLease),
}
impl NativeCodexQueueBinding {
    fn validate(&self, stream: &std::os::unix::net::UnixStream) -> io::Result<()> {
        match self {
            Self::Shared(binding) => binding.validate(stream),
            #[cfg(all(
                feature = "local_fs",
                any(
                    all(target_os = "macos", target_arch = "aarch64"),
                    all(target_os = "linux", target_arch = "x86_64")
                )
            ))]
            Self::Owned(binding) => binding.validate(stream),
        }
    }
}

fn queue_input(
    service: &ImageStagingService,
    staging: &mut RemoteImageStaging,
    connection: &ImageStagingConnection,
    lease: &SubmissionLease,
    scope: &RemoteImageScope,
    request: &CliImageCodexQueue,
) -> Result<CliImageCodexQueueResult, CliImageStagingError> {
    let key = parse_scope_uuid(&request.recovery_key)?;
    let subject: [u8; 32] = request.subject_sha256.as_slice().try_into().map_err(|_| {
        error(
            CliImageStagingErrorCode::InvalidTransfer,
            "invalid queue subject",
        )
    })?;
    if let Some(previous) = staging
        .queue_status(scope, scope.submission_id, key)
        .map_err(storage_error)?
    {
        if previous.claim.subject != subject || previous.claim.claude_recovery.is_some() {
            return Err(error(
                CliImageStagingErrorCode::InvalidTransfer,
                "queue subject changed",
            ));
        }
        if matches!(previous.status.as_str(), "confirmed" | "rejected") {
            staging
                .finish_queue(scope, &previous)
                .map_err(storage_error)?;
        }
        return Ok(queue_response(scope.submission_id, Some(previous)));
    }
    let candidate = request.binding.as_ref().ok_or_else(|| {
        error(
            CliImageStagingErrorCode::InvalidScope,
            "Codex process binding is missing",
        )
    })?;
    if request.references.is_empty()
        || request.references.len() > 20
        || request.text.len() > 1024 * 1024
    {
        return Err(error(
            CliImageStagingErrorCode::InvalidTransfer,
            "invalid image queue budget",
        ));
    }
    let mut images = Vec::new();
    let mut references = Vec::new();
    let mut digest = Sha256::new();
    digest.update((request.text.len() as u64).to_le_bytes());
    digest.update(request.text.as_bytes());
    for reference in &request.references {
        let id = transfer_id(&reference.transfer_id)?;
        let recovery_key = parse_scope_uuid(&reference.recovery_key)?;
        if references.iter().any(|(previous, _)| previous == &id) {
            return Err(error(
                CliImageStagingErrorCode::InvalidTransfer,
                "duplicate queue image",
            ));
        }
        let (published, released) = staging
            .recover_reference(scope, id, recovery_key, false)
            .map_err(storage_error)?;
        let (path, spec) = published.filter(|_| !released).ok_or_else(|| {
            error(
                CliImageStagingErrorCode::InvalidTransfer,
                "queue image is unavailable",
            )
        })?;
        digest.update(spec.byte_len.to_le_bytes());
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(storage_error)?;
        let mut bytes = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut bytes).map_err(storage_error)?;
            if count == 0 {
                break;
            }
            digest.update(&bytes[..count]);
        }
        images.push(CodexQueueImage {
            path,
            byte_len: spec.byte_len,
            sha256: spec.sha256,
        });
        references.push((id, recovery_key));
    }
    if <[u8; 32]>::from(digest.finalize()) != subject {
        return Err(error(
            CliImageStagingErrorCode::InvalidTransfer,
            "queue content does not match",
        ));
    }
    let (binding, stream) = service
        .connect_codex(scope, candidate)
        .map_err(storage_error)?;
    let validate = |stream: &std::os::unix::net::UnixStream| binding.validate(stream);
    let native = CodexImageQueue::connect(
        stream,
        parse_scope_uuid(&scope.cli_session_id)?,
        Path::new(&candidate.working_directory),
        &validate,
    )
    .map_err(storage_error)?;
    let saved_claim = RefCell::new(None);
    let dispatched = std::cell::Cell::new(false);
    let result = native.submit_once(
        scope.submission_id,
        &request.text,
        &images,
        |attempt| {
            let claim = QueueClaim {
                version: 1,
                host: scope.host_id.as_str().to_owned(),
                native_session: scope.cli_session_id.clone(),
                submission: scope.submission_id,
                key_hash: Sha256::digest(key.as_bytes()).into(),
                subject,
                native_request_sha256: attempt.request_sha256,
                references,
                claude_recovery: None,
            };
            staging.claim_queue(&claim)?;
            *saved_claim.borrow_mut() = Some(claim);
            // 持久未知记录先于领取；与同步撤销互斥。领取后的请求可收尾，禁止重新派发。
            lease.claim_native(connection)?;
            dispatched.set(true);
            Ok(())
        },
        &validate,
    );
    let Some(claim) = saved_claim.into_inner() else {
        return Err(error(
            CliImageStagingErrorCode::InvalidScope,
            "native queue preflight failed",
        ));
    };
    let output = match result {
        Ok(receipt) => QueueResult {
            claim,
            status: "confirmed".to_owned(),
            native_queue_id: receipt.queued_submission_id,
            native_ack_sha256: Some(receipt.raw_ack_sha256),
        },
        Err(_) if !dispatched.get() => QueueResult {
            claim,
            status: "rejected".to_owned(),
            native_queue_id: String::new(),
            native_ack_sha256: None,
        },
        Err(_) => QueueResult {
            claim,
            status: "unknown".to_owned(),
            native_queue_id: String::new(),
            native_ack_sha256: None,
        },
    };
    if output.status != "unknown" {
        staging
            .finish_queue(scope, &output)
            .map_err(storage_error)?;
    }
    Ok(queue_response(scope.submission_id, Some(output)))
}

fn require_registration(
    state: &State,
    key: (Uuid, SessionId),
    scope: &RemoteImageScope,
    revision: u64,
) -> Result<(), CliImageStagingError> {
    if !state.registrations.get(&key).is_some_and(|registered| {
        registered.active && registered.revision == revision && registered.scope == *scope
    }) {
        return Err(error(
            CliImageStagingErrorCode::StaleScope,
            "image staging scope is stale",
        ));
    }
    Ok(())
}

fn parse_scope(
    scope: Option<&CliImageStagingScope>,
    connection_id: Uuid,
    host_id: &HostId,
) -> Result<RemoteImageScope, CliImageStagingError> {
    let scope = scope.ok_or_else(|| {
        error(
            CliImageStagingErrorCode::InvalidScope,
            "image staging scope is missing",
        )
    })?;
    if scope.host_id != host_id.as_str()
        || scope.cli_session_id.is_empty()
        || scope.cli_session_id.len() > 512
        || scope.cli_session_id.chars().any(char::is_control)
    {
        return Err(error(
            CliImageStagingErrorCode::InvalidScope,
            "invalid image staging host or native session",
        ));
    }
    Ok(RemoteImageScope {
        host_id: host_id.clone(),
        connection_id,
        terminal_session_id: SessionId::from(scope.terminal_session_id),
        terminal_epoch: parse_scope_uuid(&scope.terminal_epoch)?,
        cli_session_id: scope.cli_session_id.clone(),
        input_generation: parse_scope_uuid(&scope.input_generation)?,
        submission_id: parse_scope_uuid(&scope.submission_id)?,
    })
}

fn parse_scope_uuid(value: &str) -> Result<Uuid, CliImageStagingError> {
    let value = Uuid::parse_str(value).map_err(|_| {
        error(
            CliImageStagingErrorCode::InvalidScope,
            "invalid image staging scope identifier",
        )
    })?;
    if value.is_nil() {
        return Err(error(
            CliImageStagingErrorCode::InvalidScope,
            "empty image staging scope identifier",
        ));
    }
    Ok(value)
}

fn transfer_id(value: &str) -> Result<Uuid, CliImageStagingError> {
    let transfer = Uuid::parse_str(value).map_err(|_| {
        error(
            CliImageStagingErrorCode::InvalidTransfer,
            "invalid image transfer identifier",
        )
    })?;
    if transfer.is_nil() || transfer.to_string() != value {
        return Err(error(
            CliImageStagingErrorCode::InvalidTransfer,
            "noncanonical image transfer identifier",
        ));
    }
    Ok(transfer)
}

fn parse_spec(spec: Option<&CliImageStagingSpec>) -> Result<RemoteImageSpec, CliImageStagingError> {
    let spec = spec.ok_or_else(|| {
        error(
            CliImageStagingErrorCode::InvalidTransfer,
            "image staging spec is missing",
        )
    })?;
    let sha256 = spec.sha256.as_slice().try_into().map_err(|_| {
        error(
            CliImageStagingErrorCode::InvalidTransfer,
            "invalid image digest length",
        )
    })?;
    if spec.byte_len == 0 || spec.byte_len > MAX_RESERVED_BYTES {
        return Err(error(
            CliImageStagingErrorCode::InvalidTransfer,
            "invalid image byte budget",
        ));
    }
    Ok(RemoteImageSpec {
        byte_len: spec.byte_len,
        sha256,
    })
}

fn error(code: CliImageStagingErrorCode, message: &'static str) -> CliImageStagingError {
    CliImageStagingError {
        code: code.into(),
        message: message.into(),
    }
}

fn storage_error(error: io::Error) -> CliImageStagingError {
    let code = match error.kind() {
        io::ErrorKind::InvalidInput | io::ErrorKind::NotFound => {
            CliImageStagingErrorCode::InvalidTransfer
        }
        _ => CliImageStagingErrorCode::StorageFailure,
    };
    CliImageStagingError {
        code: code.into(),
        message: format!("image staging operation failed ({:?})", error.kind()),
    }
}

#[cfg(test)]
#[path = "cli_image_staging_rpc_tests.rs"]
mod tests;
