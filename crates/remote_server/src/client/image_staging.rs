//! 只在当前已握手连接上传未投递字节；不接受主机级故障转移或路径降级。

use super::{ClientError, RemoteServerClient};
use crate::proto::{
    CliImageStagingRequest, CliImageStagingResponse, CliImageStagingRevoke, CliImageStagingScope,
    ClientMessage, RemoteServerCapability, cli_image_staging_request, cli_image_staging_response,
    notification, server_message, session_scoped_request,
};
use crate::protocol::RequestId;
use uuid::Uuid;
use warp_core::SessionId;

pub(super) struct ImageStagingSession {
    pub(super) epoch: Uuid,
    pub(super) epoch_revision: u64,
    pub(super) next_submission: u64,
}

impl RemoteServerClient {
    pub fn cli_image_codex_queue_available(&self) -> bool {
        !self.is_disconnected()
            && self
                .initialize_response
                .read()
                .expect("initialize response lock poisoned")
                .as_ref()
                .is_some_and(|response| {
                    response.supports(RemoteServerCapability::CliImageCodexQueueV1)
                })
    }

    pub fn cli_image_claude_read_available(&self) -> bool {
        !self.is_disconnected()
            && self
                .initialize_response
                .read()
                .expect("initialize response lock poisoned")
                .as_ref()
                .is_some_and(|response| {
                    response.supports(RemoteServerCapability::CliImageClaudeReadV1)
                })
    }
    /// 缓存的握手主机身份可用于断连后的本地释放意图，不能作为当前连接可用证明。
    pub fn cli_image_reference_host(&self) -> Option<String> {
        self.initialize_response
            .read()
            .expect("initialize response lock poisoned")
            .as_ref()
            .filter(|initialized| {
                initialized.supports(RemoteServerCapability::CliImageReferenceRecoveryV1)
            })
            .map(|initialized| initialized.host_id.clone())
    }

    /// UI 派发前也要复核同一终端实例，不能只比较仍存活的 client 指针。
    pub fn cli_image_scope_is_current(&self, scope: &CliImageStagingScope) -> bool {
        !self.is_disconnected()
            && self
                .require_image_staging_connection(&CliImageStagingRequest {
                    scope: Some(scope.clone()),
                    revision: 0,
                    action: None,
                })
                .is_ok()
    }

    pub(super) fn advance_image_staging_session(&self, session_id: SessionId) -> (String, u64) {
        let mut sessions = self
            .image_staging_sessions
            .write()
            .expect("image staging sessions lock poisoned");
        let revision = sessions
            .get(&session_id)
            .map_or(Some(1), |current| current.epoch_revision.checked_add(1))
            .unwrap_or(0);
        let epoch = Uuid::new_v4();
        sessions.insert(
            session_id,
            ImageStagingSession {
                epoch,
                epoch_revision: revision,
                next_submission: 0,
            },
        );
        (epoch.to_string(), revision)
    }

    /// 仅分配当前 transport/终端实例内的稳定编号，不代表原生 CLI 输入权限。
    pub fn allocate_cli_image_scope(
        &self,
        session_id: SessionId,
        native_session: &str,
        input_generation: Uuid,
        submission_id: Uuid,
    ) -> Result<(CliImageStagingScope, u64), ClientError> {
        if self.is_disconnected()
            || input_generation.is_nil()
            || submission_id.is_nil()
            || native_session.is_empty()
            || native_session.len() > 512
            || native_session.chars().any(char::is_control)
        {
            return Err(ClientError::FileOperationFailed(
                "invalid image submission binding".into(),
            ));
        }
        let host_id = {
            let initialized = self
                .initialize_response
                .read()
                .expect("initialize response lock poisoned");
            initialized
                .as_ref()
                .filter(|response| {
                    response.supports(RemoteServerCapability::CliImageUnpublishedStagingV1)
                })
                .map(|response| response.host_id.clone())
                .ok_or_else(|| {
                    ClientError::FileOperationFailed(
                        "remote image staging v1 is unavailable".into(),
                    )
                })?
        };
        let mut sessions = self
            .image_staging_sessions
            .write()
            .expect("image staging sessions lock poisoned");
        let session = sessions
            .get_mut(&session_id)
            .filter(|session| session.epoch_revision != 0)
            .ok_or_else(|| {
                ClientError::FileOperationFailed("image terminal is not bootstrapped".into())
            })?;
        session.next_submission = session.next_submission.checked_add(1).ok_or_else(|| {
            ClientError::FileOperationFailed("image submission revision overflow".into())
        })?;
        Ok((
            CliImageStagingScope {
                host_id,
                terminal_session_id: session_id.as_u64(),
                cli_session_id: native_session.to_owned(),
                input_generation: input_generation.to_string(),
                submission_id: submission_id.to_string(),
                terminal_epoch: session.epoch.to_string(),
            },
            session.next_submission,
        ))
    }

    /// 取消采用连接内通知，不依赖即将丢弃的 future；旧 client 不会向新 transport 转发。
    pub fn revoke_cli_image_scope(&self, scope: CliImageStagingScope, revision: u64) {
        if self.is_disconnected() {
            return;
        }
        self.send_notification(ClientMessage::notification(
            notification::Message::RevokeCliImageStaging(CliImageStagingRequest {
                scope: Some(scope),
                revision,
                action: Some(cli_image_staging_request::Action::Revoke(
                    CliImageStagingRevoke {},
                )),
            }),
        ));
    }

    /// 仅用于已知没有派发或明确结束的引用；断连时绝不向其他连接重放。
    pub fn release_cli_image_reference(
        &self,
        scope: CliImageStagingScope,
        revision: u64,
        transfer_id: Uuid,
        recovery_key: Uuid,
    ) {
        if self.is_disconnected() {
            return;
        }
        self.send_notification(ClientMessage::notification(
            notification::Message::RevokeCliImageStaging(CliImageStagingRequest {
                scope: Some(scope),
                revision,
                action: Some(cli_image_staging_request::Action::Release(
                    crate::proto::CliImageStagingRelease {
                        transfer_id: transfer_id.to_string(),
                        recovery_key: recovery_key.to_string(),
                    },
                )),
            }),
        ));
    }

    /// 收据只证明暂存状态，不代表原生 CLI 已接收或理解图片。
    pub async fn stage_unpublished_cli_image(
        &self,
        request: CliImageStagingRequest,
    ) -> Result<CliImageStagingResponse, ClientError> {
        if self.is_disconnected() {
            return Err(ClientError::Disconnected);
        }
        self.require_image_staging_connection(&request)?;
        if request.action.is_none() {
            return Err(ClientError::FileOperationFailed(
                "image staging action is missing".into(),
            ));
        }
        let request_id = RequestId::new();
        let message = ClientMessage::session_scoped(
            request_id.to_string(),
            session_scoped_request::Message::CliImageStaging(request.clone()),
        );
        let response = self.send_request_internal(request_id, message).await?;
        let Some(server_message::Message::CliImageStaging(response)) = response.message else {
            return Err(ClientError::UnexpectedResponse);
        };
        if self.is_disconnected() {
            return Err(ClientError::Disconnected);
        }
        self.require_image_staging_connection(&request)?;
        validate_response(&request, &response)?;
        Ok(response)
    }
    fn require_image_staging_connection(
        &self,
        request: &CliImageStagingRequest,
    ) -> Result<(), ClientError> {
        {
            let initialized = self
                .initialize_response
                .read()
                .expect("initialize response lock poisoned");
            let Some(initialized) = initialized.as_ref() else {
                return Err(ClientError::FileOperationFailed(
                    "image staging requires Initialize".into(),
                ));
            };
            if !initialized.supports(RemoteServerCapability::CliImageUnpublishedStagingV1) {
                return Err(ClientError::FileOperationFailed(
                    "remote image staging v1 is unavailable".into(),
                ));
            }
            if matches!(
                request.action,
                Some(
                    cli_image_staging_request::Action::Publish(_)
                        | cli_image_staging_request::Action::Release(_)
                        | cli_image_staging_request::Action::Recover(_)
                )
            ) && !initialized.supports(RemoteServerCapability::CliImageReferenceRecoveryV1)
            {
                return Err(ClientError::FileOperationFailed(
                    "remote image references v1 is unavailable".into(),
                ));
            }
            if initialized.host_id.is_empty()
                || request
                    .scope
                    .as_ref()
                    .is_none_or(|scope| scope.host_id != initialized.host_id)
            {
                return Err(ClientError::FileOperationFailed(
                    "image staging host does not match this connection".into(),
                ));
            }
            if matches!(
                request.action,
                Some(
                    cli_image_staging_request::Action::CodexQueue(_)
                        | cli_image_staging_request::Action::CodexQueueStatus(_)
                )
            ) && !initialized.supports(RemoteServerCapability::CliImageCodexQueueV1)
            {
                return Err(ClientError::FileOperationFailed(
                    "remote Codex image queue is unavailable".into(),
                ));
            }
            if matches!(
                request.action,
                Some(
                    cli_image_staging_request::Action::ClaudeQueue(_)
                        | cli_image_staging_request::Action::ClaudeQueueStatus(_)
                )
            ) && !initialized.supports(RemoteServerCapability::CliImageClaudeReadV1)
            {
                return Err(ClientError::FileOperationFailed(
                    "remote Claude image queue is unavailable".into(),
                ));
            }
        }
        let scope = request
            .scope
            .as_ref()
            .ok_or(ClientError::UnexpectedResponse)?;
        let sessions = self
            .image_staging_sessions
            .read()
            .expect("image staging sessions lock poisoned");
        if !sessions
            .get(&SessionId::from(scope.terminal_session_id))
            .is_some_and(|session| {
                session.epoch.to_string() == scope.terminal_epoch && session.epoch_revision != 0
            })
        {
            return Err(ClientError::FileOperationFailed(
                "image terminal instance has changed".into(),
            ));
        }
        Ok(())
    }
}

fn validate_response(
    request: &CliImageStagingRequest,
    response: &CliImageStagingResponse,
) -> Result<(), ClientError> {
    use cli_image_staging_request::Action;
    use cli_image_staging_response::Result;

    if response.scope != request.scope {
        return Err(ClientError::UnexpectedResponse);
    }
    if matches!(&response.result, Some(Result::Error(_))) {
        return if response.revision == request.revision {
            Ok(())
        } else {
            Err(ClientError::UnexpectedResponse)
        };
    }
    let expected_revision = if matches!(&request.action, Some(Action::Activate(_))) {
        request
            .revision
            .checked_add(1)
            .ok_or(ClientError::UnexpectedResponse)?
    } else {
        request.revision
    };
    if response.revision != expected_revision {
        return Err(ClientError::UnexpectedResponse);
    }
    let valid = match request.action.as_ref() {
        Some(Action::Activate(_)) => matches!(&response.result, Some(Result::Activated(_))),
        Some(Action::Begin(begin)) => matches!(&response.result, Some(Result::Progress(progress))
            if progress.transfer_id == begin.transfer_id
                && begin.spec.as_ref().is_some_and(|spec| progress.next_offset <= spec.byte_len)),
        Some(Action::Chunk(chunk)) => matches!(&response.result, Some(Result::Progress(progress))
            if progress.transfer_id == chunk.transfer_id
                && chunk.offset.checked_add(chunk.bytes.len() as u64).is_some_and(|end| progress.next_offset >= end)),
        Some(Action::Verify(verify)) => matches!(&response.result, Some(Result::Verified(verified))
            if verify.transfer_id == verified.transfer_id
                && verify.expected_spec.is_some() && verify.expected_spec == verified.spec),
        Some(Action::Revoke(_)) => matches!(&response.result, Some(Result::Revoked(_))),
        Some(Action::Publish(publish)) => {
            matches!(&response.result, Some(Result::Published(published))
            if published.transfer_id == publish.transfer_id && !published.remote_path.is_empty()
                && !published.remote_path.chars().any(char::is_control) && published.spec.is_some())
        }
        Some(Action::Release(release)) => {
            matches!(&response.result, Some(Result::Released(released)) if released.transfer_id == release.transfer_id)
        }
        Some(Action::Recover(recover)) => {
            matches!(&response.result, Some(Result::Recovered(recovered))
            if recovered.transfer_id == recover.transfer_id && !(recovered.released && recovered.reference.is_some())
                && recovered.reference.as_ref().is_none_or(|reference| reference.transfer_id == recover.transfer_id && reference.spec.is_some()))
        }
        Some(Action::Cancel(cancel)) => {
            matches!(&response.result, Some(Result::Cancelled(cancelled)) if cancel.transfer_id == cancelled.transfer_id)
        }
        Some(Action::CodexQueue(queue)) => {
            matches!(&response.result, Some(Result::CodexQueue(result))
                if request.scope.as_ref().is_some_and(|scope| scope.submission_id == result.submission_id)
                    && queue.subject_sha256 == result.subject_sha256
                    && matches!(result.status.as_str(), "confirmed" | "rejected" | "unknown")
                    && (result.status != "confirmed" || !result.native_queue_id.is_empty()))
        }
        Some(Action::CodexQueueStatus(status)) => {
            matches!(&response.result, Some(Result::CodexQueue(result))
                if status.submission_id == result.submission_id
                    && matches!(result.status.as_str(), "confirmed" | "rejected" | "unknown" | "absent")
                    && (result.status == "absent" || result.subject_sha256.len() == 32)
                    && (result.status != "confirmed" || !result.native_queue_id.is_empty()))
        }
        Some(Action::ClaudeQueue(queue)) => {
            matches!(&response.result, Some(Result::ClaudeQueue(result))
                if request.scope.as_ref().is_some_and(|scope| scope.submission_id == result.submission_id)
                    && queue.subject_sha256 == result.subject_sha256
                    && matches!(result.status.as_str(), "confirmed" | "rejected" | "unknown")
                    && (result.status != "confirmed" || !result.native_queue_id.is_empty()))
        }
        Some(Action::ClaudeQueueStatus(status)) => {
            matches!(&response.result, Some(Result::ClaudeQueue(result))
                if status.submission_id == result.submission_id
                    && matches!(result.status.as_str(), "confirmed" | "rejected" | "unknown" | "absent")
                    && (result.status == "absent" || result.subject_sha256.len() == 32)
                    && (result.status != "confirmed" || !result.native_queue_id.is_empty()))
        }
        None => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ClientError::UnexpectedResponse)
    }
}

#[cfg(test)]
#[path = "image_staging_tests.rs"]
mod tests;
