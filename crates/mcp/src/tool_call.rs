use std::future::Future;
use std::time::Duration;

use futures::FutureExt;
use futures::future::{Either, select};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
    ClientRequest, ServerResult,
};
use rmcp::service::PeerRequestOptions;
use rmcp::{Peer, RoleClient, ServiceError};
use warpui::r#async::Timer;

/// 连接、派发与等待结果共用的期限；进度通知不延长此预算。
pub const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const CANCELLATION_TIMEOUT: Duration = Duration::from_secs(5);

/// 保留错误类型，由应用层生成本地化提示。
#[derive(Debug, thiserror::Error)]
pub enum ToolCallError {
    #[error(transparent)]
    Service(#[from] ServiceError),
    #[error("mcp_tool_call_outcome_unknown")]
    OutcomeUnknown(#[source] ServiceError),
    #[error("mcp_tool_call_deadline_exceeded")]
    DeadlineExceeded { may_have_executed: bool },
}

/// 工具至多派发一次；响应丢失时不能安全地重放可能产生副作用的调用。
pub async fn call_tool_with_deadline(
    params: CallToolRequestParams,
    connect: impl Future<Output = Result<Peer<RoleClient>, ServiceError>>,
) -> Result<CallToolResult, ToolCallError> {
    call_tool_with_deadline_inner(
        params,
        connect,
        Timer::after(TOOL_CALL_TIMEOUT).map(|_| ()),
        || Timer::after(CANCELLATION_TIMEOUT).map(|_| ()),
    )
    .await
}

async fn call_tool_with_deadline_inner<CancellationTimer: Future<Output = ()>>(
    params: CallToolRequestParams,
    connect: impl Future<Output = Result<Peer<RoleClient>, ServiceError>>,
    deadline: impl Future<Output = ()>,
    cancellation_deadline: impl FnOnce() -> CancellationTimer,
) -> Result<CallToolResult, ToolCallError> {
    let mut may_have_executed = false;
    let mut cancellation = None;
    let completed = {
        let operation = async {
            let peer = connect.await?;
            // 发送一旦开始，取消等待并不能证明请求尚未到达服务器。
            may_have_executed = true;
            let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
            // rmcp 自带超时会等待取消通知发送完成；外层期限同时覆盖发送阻塞。
            let handle = peer
                .send_cancellable_request(request, PeerRequestOptions::no_options())
                .await?;
            cancellation = Some((peer, handle.id.clone()));
            if let ServerResult::CallToolResult(result) = handle.await_response().await? {
                Ok(result)
            } else {
                Err(ServiceError::UnexpectedResponse)
            }
        };
        match select(Box::pin(operation), Box::pin(deadline)).await {
            Either::Left((result, _)) => Some(result),
            Either::Right(((), _)) => None,
        }
    };

    if let Some(result) = completed {
        return result.map_err(|error| {
            // 只有服务器明确返回的协议错误能说明本次请求结果；派发后的断线、
            // 发送失败、取消或异常响应都不能证明副作用未发生。
            if may_have_executed && !matches!(&error, ServiceError::McpError(_)) {
                ToolCallError::OutcomeUnknown(error)
            } else {
                ToolCallError::Service(error)
            }
        });
    }

    if let Some((peer, request_id)) = cancellation {
        let cancel = peer.notify_cancelled(CancelledNotificationParam::new(
            Some(request_id),
            Some("mcp_tool_call_deadline_exceeded".to_owned()),
        ));
        // 仅取消当前请求，不关闭共享连接；取消通知也必须有界。
        let _ = select(Box::pin(cancel), Box::pin(cancellation_deadline())).await;
    }

    Err(ToolCallError::DeadlineExceeded { may_have_executed })
}

#[cfg(test)]
#[path = "tool_call_tests.rs"]
mod tests;
