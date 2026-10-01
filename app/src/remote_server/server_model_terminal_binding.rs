//! 终端线路挑战先同步撤销旧状态，再在后台读写原生终端和 tmux。

#[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
use std::sync::Arc;

use super::{ConnectionId, HandlerOutcome, ModelContext, RequestId, ServerModel, server_message};
use crate::remote_server::proto::{
    TerminalBindingFailed, TerminalBindingRequest, TerminalBindingResponse,
    terminal_binding_request, terminal_binding_response,
};

fn unavailable(request: &TerminalBindingRequest) -> TerminalBindingResponse {
    TerminalBindingResponse {
        scope: request.scope.clone(),
        attempt_id: request.attempt_id.clone(),
        result: Some(terminal_binding_response::Result::Failed(
            TerminalBindingFailed {
                code: "unavailable".into(),
            },
        )),
    }
}

impl ServerModel {
    pub(super) fn cancel_terminal_binding(
        &self,
        request: TerminalBindingRequest,
        connection_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) {
        if !matches!(
            request.action,
            Some(terminal_binding_request::Action::Cancel(_))
        ) {
            return;
        }
        #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
        if let Some(connection) = self.terminal_binding_connections.get(&connection_id) {
            let work = connection.prepare(request);
            ctx.background_executor()
                .spawn(async move {
                    let _ = work.execute();
                })
                .detach();
        }
        #[cfg(not(all(feature = "local_fs", any(target_os = "macos", target_os = "linux"))))]
        let _ = (request, connection_id, ctx);
    }

    pub(super) fn handle_terminal_binding(
        &mut self,
        request: TerminalBindingRequest,
        request_id: &RequestId,
        connection_id: ConnectionId,
        ctx: &mut ModelContext<Self>,
    ) -> HandlerOutcome {
        #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
        {
            let Some(connection) = self.terminal_binding_connections.get(&connection_id) else {
                return HandlerOutcome::Sync(server_message::Message::TerminalBinding(
                    unavailable(&request),
                ));
            };
            // 回调只保留 Weak，不能让它延长已从模型移除的连接。
            let origin = Arc::downgrade(connection);
            let failed = unavailable(&request);
            let work = connection.prepare(request);
            let (sender, receiver) = futures::channel::oneshot::channel();
            ctx.background_executor()
                .spawn(async move {
                    let _ = sender.send(work.execute());
                })
                .detach();
            let response_id = request_id.clone();
            let handle = self.spawn_request_handler(
                request_id.clone(),
                receiver,
                move |me, result, _| {
                    let response = result
                        .ok()
                        .filter(|response| {
                            let Some(origin) = origin.upgrade() else {
                                return false;
                            };
                            let Some(current) = me.terminal_binding_connections.get(&connection_id)
                            else {
                                return false;
                            };
                            Arc::ptr_eq(current, &origin)
                                && origin.completed_response_is_current(response)
                        })
                        .unwrap_or(failed);
                    me.send_server_message(
                        Some(connection_id),
                        Some(&response_id),
                        server_message::Message::TerminalBinding(response),
                    );
                },
                ctx,
            );
            HandlerOutcome::Async(Some(handle))
        }
        #[cfg(not(all(feature = "local_fs", any(target_os = "macos", target_os = "linux"))))]
        {
            let _ = (request_id, connection_id, ctx);
            HandlerOutcome::Sync(server_message::Message::TerminalBinding(unavailable(
                &request,
            )))
        }
    }
}
