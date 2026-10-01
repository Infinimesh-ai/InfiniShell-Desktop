//! 终端输出线路挑战只属于原连接和原 epoch，不参与主机级故障转移。

use super::{ClientError, RemoteServerClient};
use crate::proto::{
    ClientMessage, RemoteServerCapability, TerminalBindingAck, TerminalBindingBegin,
    TerminalBindingCancel, TerminalBindingRequest, TerminalBindingScope, notification,
    server_message, session_scoped_request, terminal_binding_request, terminal_binding_response,
};
use crate::protocol::RequestId;
use uuid::Uuid;
use warp_core::SessionId;

impl RemoteServerClient {
    pub fn terminal_binding_available(&self) -> bool {
        !self.is_disconnected()
            && self
                .initialize_response
                .read()
                .expect("initialize response lock poisoned")
                .as_ref()
                .is_some_and(|response| {
                    response.supports(RemoteServerCapability::TerminalBindingV1)
                })
    }

    /// 只复用 bootstrap 的连接代次，不分配或读取图片提交序号。
    pub fn terminal_binding_scope(&self, session_id: SessionId) -> Option<TerminalBindingScope> {
        if !self.terminal_binding_available() {
            return None;
        }
        let host_id = self
            .initialize_response
            .read()
            .expect("initialize response lock poisoned")
            .as_ref()?
            .host_id
            .clone();
        if host_id.is_empty() {
            return None;
        }
        let sessions = self
            .image_staging_sessions
            .read()
            .expect("session epochs lock poisoned");
        let session = sessions.get(&session_id)?;
        (session.epoch_revision != 0).then(|| TerminalBindingScope {
            host_id,
            terminal_session_id: session_id.as_u64(),
            terminal_epoch: session.epoch.to_string(),
        })
    }

    pub fn terminal_binding_scope_is_current(&self, scope: &TerminalBindingScope) -> bool {
        self.terminal_binding_scope(SessionId::from(scope.terminal_session_id))
            .as_ref()
            == Some(scope)
    }

    async fn terminal_binding_request(
        &self,
        scope: TerminalBindingScope,
        attempt: Uuid,
        action: terminal_binding_request::Action,
    ) -> Result<terminal_binding_response::Result, ClientError> {
        if attempt.is_nil() || !self.terminal_binding_scope_is_current(&scope) {
            return Err(ClientError::Disconnected);
        }
        let request_id = RequestId::new();
        let request = ClientMessage::session_scoped(
            request_id.to_string(),
            session_scoped_request::Message::TerminalBinding(TerminalBindingRequest {
                scope: Some(scope.clone()),
                attempt_id: attempt.to_string(),
                action: Some(action),
            }),
        );
        let response = self.send_request_internal(request_id, request).await?;
        let Some(server_message::Message::TerminalBinding(response)) = response.message else {
            return Err(ClientError::UnexpectedResponse);
        };
        if !self.terminal_binding_scope_is_current(&scope) {
            return Err(ClientError::Disconnected);
        }
        if response.scope.as_ref() != Some(&scope) || response.attempt_id != attempt.to_string() {
            return Err(ClientError::UnexpectedResponse);
        }
        response.result.ok_or(ClientError::UnexpectedResponse)
    }

    /// 成功仅表示 daemon 已写挑战；调用方仍须等待目标终端的 OSC。
    pub async fn begin_terminal_binding(
        &self,
        scope: TerminalBindingScope,
        attempt: Uuid,
    ) -> Result<(), ClientError> {
        match self
            .terminal_binding_request(
                scope,
                attempt,
                terminal_binding_request::Action::Begin(TerminalBindingBegin {}),
            )
            .await?
        {
            terminal_binding_response::Result::ChallengeWritten(_) => Ok(()),
            terminal_binding_response::Result::Bound(_)
            | terminal_binding_response::Result::Failed(_)
            | terminal_binding_response::Result::Cancelled(_) => {
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// nonce 必须来自同一 view 的专用 OSC 事件；此接口不读取或推测 nonce。
    pub async fn ack_terminal_binding(
        &self,
        scope: TerminalBindingScope,
        attempt: Uuid,
        nonce: Uuid,
    ) -> Result<String, ClientError> {
        if nonce.is_nil() {
            return Err(ClientError::UnexpectedResponse);
        }
        match self
            .terminal_binding_request(
                scope,
                attempt,
                terminal_binding_request::Action::Ack(TerminalBindingAck {
                    nonce: nonce.to_string(),
                }),
            )
            .await?
        {
            terminal_binding_response::Result::Bound(bound)
                if !bound.opaque_binding_id.is_empty()
                    && bound.opaque_binding_id.len() <= 256
                    && bound
                        .opaque_binding_id
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic()) =>
            {
                Ok(bound.opaque_binding_id)
            }
            terminal_binding_response::Result::Bound(_)
            | terminal_binding_response::Result::ChallengeWritten(_)
            | terminal_binding_response::Result::Failed(_)
            | terminal_binding_response::Result::Cancelled(_) => {
                Err(ClientError::UnexpectedResponse)
            }
        }
    }

    /// 丢弃 future 后也发出精确撤销；旧 attempt 不能撤销后续 attempt。
    pub fn cancel_terminal_binding(&self, scope: TerminalBindingScope, attempt: Uuid) {
        if attempt.is_nil() || !self.terminal_binding_scope_is_current(&scope) {
            return;
        }
        self.send_notification(ClientMessage::notification(
            notification::Message::CancelTerminalBinding(TerminalBindingRequest {
                scope: Some(scope),
                attempt_id: attempt.to_string(),
                action: Some(terminal_binding_request::Action::Cancel(
                    TerminalBindingCancel {},
                )),
            }),
        ));
    }
}

#[cfg(test)]
#[path = "terminal_binding_tests.rs"]
mod tests;
