//! Claude 的上传文件、原生 Read 和历史消费恢复；绝不重发未知输入。

use std::cell::RefCell;
use std::path::PathBuf;

use super::super::cli_image_claude_queue::{
    ClaudeImageInbox, ClaudeInboxTarget, ClaudeQueueImage, ClaudeQueuedImageAttempt,
    ClaudeTranscript, ClaudeWriteState,
};
use super::super::cli_image_native_claude_binding::Binding;
use super::super::cli_image_native_lifetime::NativeLifetime;
use super::super::cli_image_staging::PreflightRejection;
use super::super::proto::CliImageClaudeQueue;
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recovery {
    session: Uuid,
    cwd: PathBuf,
    process: u32,
    socket: PathBuf,
    registry: PathBuf,
    transcript: PathBuf,
    attempt: ClaudeQueuedImageAttempt,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifetime: Option<NativeLifetime>,
}
impl Recovery {
    fn new(
        target: &ClaudeInboxTarget,
        attempt: &ClaudeQueuedImageAttempt,
        lifetime: Option<NativeLifetime>,
    ) -> Self {
        Self {
            session: target.session_id,
            cwd: target.cwd.clone(),
            process: target.process_id,
            socket: target.socket_path.clone(),
            registry: target.registry_directory.clone(),
            transcript: target.transcript_path.clone(),
            attempt: attempt.clone(),
            lifetime,
        }
    }
    fn target(&self) -> ClaudeInboxTarget {
        ClaudeInboxTarget {
            session_id: self.session,
            cwd: self.cwd.clone(),
            process_id: self.process,
            socket_path: self.socket.clone(),
            registry_directory: self.registry.clone(),
            transcript_path: self.transcript.clone(),
        }
    }
}

pub(super) fn recover(
    staging: &mut RemoteImageStaging,
    scope: &RemoteImageScope,
    submission: Uuid,
    key: Uuid,
) -> Result<CliImageCodexQueueResult, CliImageStagingError> {
    if let Some(rejection) = staging
        .preflight_rejection(scope, submission, key)
        .map_err(storage_error)?
    {
        staging
            .reject_preflight(scope, &rejection)
            .map_err(storage_error)?;
        return Ok(preflight_response(&rejection));
    }
    let Some(mut result) = staging
        .queue_status(scope, submission, key)
        .map_err(storage_error)?
    else {
        return Ok(queue_response(submission, None));
    };
    let data = result
        .claim
        .claude_recovery
        .as_ref()
        .ok_or_else(|| storage_error(io::Error::other("native image consumer changed")))?;
    let recovery: Recovery = serde_json::from_value(data.clone())
        .map_err(|_| storage_error(io::Error::other("invalid image recovery")))?;
    if recovery.session.to_string() != scope.cli_session_id
        || recovery.attempt.client_message_id() != submission
        || recovery.attempt.request_sha256() != result.claim.native_request_sha256
        || recovery
            .lifetime
            .as_ref()
            .is_some_and(|lifetime| lifetime.pid() != recovery.process as i32)
    {
        return Err(storage_error(io::Error::other(
            "image recovery identity changed",
        )));
    }
    if result.status == "unknown" {
        // 原生历史缺失或被替换不证明消费；仅原消费者明确结束后可退休引用。
        let history = match &recovery.attempt {
            ClaudeQueuedImageAttempt::Existing(attempt) => {
                ClaudeTranscript::recover(&recovery.target(), attempt)
            }
            ClaudeQueuedImageAttempt::Pending(pending) => {
                if pending.session_id != recovery.session
                    || pending.transcript_path != recovery.transcript
                {
                    return Err(storage_error(io::Error::other(
                        "pending transcript identity changed",
                    )));
                }
                staging
                    .bind_claude_transcript(&result.claim, pending)
                    .and_then(|(file, attempt, binding)| {
                        ClaudeTranscript::recover_bound(&recovery.target(), &attempt, file, binding)
                    })
            }
        };
        if let Ok(mut history) = history
            && let Ok(Some(receipt)) = history.poll_available()
        {
            let encoded = serde_json::to_vec(&receipt)
                .map_err(|_| storage_error(io::Error::other("image receipt unavailable")))?;
            result.status = "confirmed".to_owned();
            result.native_queue_id = receipt.attempt.client_message_id.to_string();
            result.native_ack_sha256 = Some(Sha256::digest(encoded).into());
        } else if recovery
            .lifetime
            .as_ref()
            .is_some_and(|lifetime| matches!(lifetime.exited(), Ok(true)))
        {
            // 原消费者已经结束，只退休引用；这不证明未发送或已消费。
            result.status = "retired".to_owned();
        }
    }
    if matches!(result.status.as_str(), "confirmed" | "rejected" | "retired") {
        staging
            .finish_queue(scope, &result)
            .map_err(storage_error)?;
    }
    Ok(queue_response(submission, Some(result)))
}

pub(super) fn submit(
    staging: &mut RemoteImageStaging,
    connection: &ImageStagingConnection,
    lease: &SubmissionLease,
    scope: &RemoteImageScope,
    request: &CliImageClaudeQueue,
) -> Result<CliImageCodexQueueResult, CliImageStagingError> {
    let key = parse_scope_uuid(&request.recovery_key)?;
    let subject: [u8; 32] = request.subject_sha256.as_slice().try_into().map_err(|_| {
        error(
            CliImageStagingErrorCode::InvalidTransfer,
            "invalid queue subject",
        )
    })?;
    if let Some(previous) = staging
        .preflight_rejection(scope, scope.submission_id, key)
        .map_err(storage_error)?
    {
        if previous.subject != subject {
            return Err(error(
                CliImageStagingErrorCode::InvalidTransfer,
                "queue subject changed",
            ));
        }
        return recover(staging, scope, scope.submission_id, key);
    }
    if let Some(previous) = staging
        .queue_status(scope, scope.submission_id, key)
        .map_err(storage_error)?
    {
        if previous.claim.subject != subject {
            return Err(error(
                CliImageStagingErrorCode::InvalidTransfer,
                "queue subject changed",
            ));
        }
        return recover(staging, scope, scope.submission_id, key);
    }
    let candidate = request.binding.as_ref().ok_or_else(|| {
        error(
            CliImageStagingErrorCode::InvalidScope,
            "Claude process binding is missing",
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
        images.push(ClaudeQueueImage {
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
    let rejection = PreflightRejection {
        version: 1,
        host: scope.host_id.as_str().to_owned(),
        native_session: scope.cli_session_id.clone(),
        submission: scope.submission_id,
        key_hash: Sha256::digest(key.as_bytes()).into(),
        subject,
        references: references.clone(),
    };
    let saved_claim = RefCell::new(None);
    let write_state = ClaudeWriteState::default();
    let result = (|| -> io::Result<()> {
        let session = Uuid::parse_str(&scope.cli_session_id)
            .map_err(|_| io::Error::other("invalid Claude session"))?;
        let (binding, stream) = Binding::connect(candidate, session)?;
        let lifetime = binding.lifetime(&stream)?;
        let target = binding.target();
        let native =
            ClaudeImageInbox::connect(stream, target.clone(), &|stream| binding.validate(stream))?;
        native.submit_once(
            scope.submission_id,
            &request.text,
            &images,
            &write_state,
            |attempt| {
                let claude_recovery =
                    serde_json::to_value(Recovery::new(&target, attempt, Some(lifetime)))
                        .map_err(|_| io::Error::other("image attempt unavailable"))?;
                let claim = QueueClaim {
                    version: 1,
                    host: scope.host_id.as_str().to_owned(),
                    native_session: scope.cli_session_id.clone(),
                    submission: scope.submission_id,
                    key_hash: Sha256::digest(key.as_bytes()).into(),
                    subject,
                    native_request_sha256: attempt.request_sha256(),
                    references,
                    claude_recovery: Some(claude_recovery),
                };
                staging.claim_queue(&claim)?;
                *saved_claim.borrow_mut() = Some(claim);
                lease.claim_native(connection)
            },
            &|stream| binding.validate(stream),
        )?;
        Ok(())
    })();
    if result.is_err() && !write_state.started() {
        if let Some(claim) = saved_claim.into_inner() {
            let rejected = QueueResult {
                claim,
                status: "rejected".to_owned(),
                native_queue_id: String::new(),
                native_ack_sha256: None,
            };
            staging
                .finish_queue(scope, &rejected)
                .map_err(storage_error)?;
            return Ok(queue_response(scope.submission_id, Some(rejected)));
        }
        staging
            .reject_preflight(scope, &rejection)
            .map_err(storage_error)?;
        return Ok(preflight_response(&rejection));
    }
    if saved_claim.into_inner().is_none() {
        return Err(error(
            CliImageStagingErrorCode::InvalidScope,
            "native Claude claim unavailable",
        ));
    }
    // 写出成功仍未知；不把消息入队当作图片消费。
    recover(staging, scope, scope.submission_id, key)
}

fn preflight_response(rejection: &PreflightRejection) -> CliImageCodexQueueResult {
    CliImageCodexQueueResult {
        submission_id: rejection.submission.to_string(),
        subject_sha256: rejection.subject.to_vec(),
        status: "rejected".to_owned(),
        native_queue_id: String::new(),
    }
}

#[cfg(test)]
#[path = "cli_image_claude_dispatch_tests.rs"]
mod tests;
