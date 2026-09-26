//! Grok 的请求在原连接回复，撤销在模型线程先执行；磁盘和原生 I/O 留在后台。

use super::{ConnectionId, HandlerOutcome, ModelContext, RequestId, ServerModel, server_message};
use crate::remote_server::cli_image_grok_protocol::{Action, Reply, Request, encode};
use crate::remote_server::proto::{CliGrokOwnedRequest, CliGrokOwnedResponse};

fn decode_scope(request: &CliGrokOwnedRequest, host: &str) -> Option<Request> {
    let scope = request.scope.as_ref()?;
    let body = Request::decode(&request.body_json).ok()?;
    (scope.host_id == host
        && body.scope.host == scope.host_id
        && body.scope.terminal_session == scope.terminal_session_id
        && body.scope.terminal_epoch.to_string() == scope.terminal_epoch
        && body.scope.generation.to_string() == scope.input_generation)
        .then_some(body)
}

fn failed_response(request: CliGrokOwnedRequest) -> CliGrokOwnedResponse {
    CliGrokOwnedResponse {
        scope: request.scope,
        body_json: encode(&Reply::Failed {
            code: "unavailable".into(),
        })
        .expect("固定错误回执不超过协议上限"),
    }
}

impl ServerModel {
    pub(super) fn revoke_cli_grok_owned(
        &self,
        request: CliGrokOwnedRequest,
        connection_id: ConnectionId,
    ) {
        if !decode_scope(&request, &self.host_id)
            .is_some_and(|request| matches!(request.action, Action::Revoke { .. }))
        {
            return;
        }
        #[cfg(all(
            feature = "local_fs",
            any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "x86_64")
            )
        ))]
        if let Some(connection) = self.grok_owned_connections.get(&connection_id) {
            connection.revoke_request(&request.body_json);
        }
        #[cfg(not(all(
            feature = "local_fs",
            any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "x86_64")
            )
        )))]
        let _ = connection_id;
    }

    pub(super) fn handle_cli_grok_owned(
        &mut self,
        request: CliGrokOwnedRequest,
        request_id: &RequestId,
        connection_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        if decode_scope(&request, &self.host_id).is_none() {
            return HandlerOutcome::Sync(server_message::Message::CliGrokOwned(failed_response(
                request,
            )));
        }
        #[cfg(all(
            feature = "local_fs",
            any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "x86_64")
            )
        ))]
        {
            let (Some(service), Some(connection)) = (
                self.grok_owned.clone(),
                self.grok_owned_connections.get(&connection_id).cloned(),
            ) else {
                return HandlerOutcome::Sync(server_message::Message::CliGrokOwned(
                    failed_response(request),
                ));
            };
            connection.revoke_request(&request.body_json);
            let unavailable = failed_response(request.clone());
            let (sender, receiver) = futures::channel::oneshot::channel();
            ctx.background_executor()
                .spawn(async move {
                    let body_json = service.handle(&connection, &request.body_json);
                    let _ = sender.send(CliGrokOwnedResponse {
                        scope: request.scope,
                        body_json,
                    });
                })
                .detach();
            let response_id = request_id.clone();
            let handle = self.spawn_request_handler(
                request_id.clone(),
                receiver,
                move |me, result, _| {
                    me.send_server_message(
                        Some(connection_id),
                        Some(&response_id),
                        server_message::Message::CliGrokOwned(result.unwrap_or(unavailable)),
                    );
                },
                ctx,
            );
            HandlerOutcome::Async(Some(handle))
        }
        #[cfg(not(all(
            feature = "local_fs",
            any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(target_os = "linux", target_arch = "x86_64")
            )
        )))]
        {
            let _ = (request_id, connection_id, ctx);
            HandlerOutcome::Sync(server_message::Message::CliGrokOwned(failed_response(
                request,
            )))
        }
    }
}
