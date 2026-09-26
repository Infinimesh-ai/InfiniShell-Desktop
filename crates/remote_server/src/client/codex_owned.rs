//! 专属 Codex 请求只走当前 SSH 连接；图片和 ticket 均不参与主机级故障转移。

use super::{ClientError, RemoteServerClient};
use crate::proto::{
    CliCodexOwnedRequest, CliImageStagingScope, ClientMessage, RemoteServerCapability,
    notification, server_message, session_scoped_request,
};
use crate::protocol::RequestId;

const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

impl RemoteServerClient {
    pub fn cli_codex_owned_available(&self) -> bool {
        !self.is_disconnected()
            && self
                .initialize_response
                .read()
                .expect("initialize response lock poisoned")
                .as_ref()
                .is_some_and(|response| response.supports(RemoteServerCapability::CliCodexOwnedV1))
    }

    fn require_codex_owned_scope(
        &self,
        scope: &CliImageStagingScope,
        body: &[u8],
    ) -> Result<(), ClientError> {
        if !self.cli_codex_owned_available() || !self.cli_image_scope_is_current(scope) {
            return Err(ClientError::Disconnected);
        }
        if body.is_empty() || body.len() > MAX_BODY_BYTES {
            return Err(ClientError::FileOperationFailed(
                "invalid Codex request size".into(),
            ));
        }
        Ok(())
    }

    pub async fn cli_codex_owned_request(
        &self,
        scope: CliImageStagingScope,
        body_json: Vec<u8>,
    ) -> Result<Vec<u8>, ClientError> {
        self.require_codex_owned_scope(&scope, &body_json)?;
        let request_id = RequestId::new();
        let request = ClientMessage::session_scoped(
            request_id.to_string(),
            session_scoped_request::Message::CliCodexOwned(CliCodexOwnedRequest {
                scope: Some(scope.clone()),
                body_json,
            }),
        );
        let response = self.send_request_internal(request_id, request).await?;
        let Some(server_message::Message::CliCodexOwned(response)) = response.message else {
            return Err(ClientError::UnexpectedResponse);
        };
        if response.scope.as_ref() != Some(&scope) {
            return Err(ClientError::UnexpectedResponse);
        }
        self.require_codex_owned_scope(&scope, &response.body_json)?;
        Ok(response.body_json)
    }

    /// future 丢弃仍可撤销未领取输入；已写入原生协议的输入继续收取自己的回执。
    pub fn revoke_cli_codex_owned(&self, scope: CliImageStagingScope, body_json: Vec<u8>) {
        if self.require_codex_owned_scope(&scope, &body_json).is_err() {
            return;
        }
        self.send_notification(ClientMessage::notification(
            notification::Message::RevokeCliCodexOwned(CliCodexOwnedRequest {
                scope: Some(scope),
                body_json,
            }),
        ));
    }
}
