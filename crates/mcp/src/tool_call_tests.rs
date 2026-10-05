use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures::future::{pending, ready};
use rmcp::model::{CallToolRequestParams, ErrorData};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ServiceError, ServiceExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;

use super::{ToolCallError, call_tool_with_deadline_inner};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum Reply {
    Success,
    ProtocolError,
    HttpError,
    WrongId,
    EmptySse,
    ResumableSse,
    StuckSend,
    StuckCancellation,
}

struct ServerState {
    reply: Reply,
    calls: AtomicUsize,
    cancellations: AtomicUsize,
    resumes: AtomicUsize,
    request_seen: Notify,
    cancellation_seen: Notify,
    request_id: Mutex<Option<Value>>,
}

struct TestServer {
    uri: String,
    state: Arc<ServerState>,
    task: JoinHandle<()>,
}

impl TestServer {
    fn transport(&self) -> StreamableHttpClientTransport<reqwest::Client> {
        crate::install_test_crypto_provider();
        // 本地回环夹具不使用宿主代理，避免网络环境影响协议回归测试。
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        StreamableHttpClientTransport::with_client(
            client,
            StreamableHttpClientTransportConfig::with_uri(self.uri.clone()),
        )
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start_server(reply: Reply) -> TestServer {
    let state = Arc::new(ServerState {
        reply,
        calls: AtomicUsize::new(0),
        cancellations: AtomicUsize::new(0),
        resumes: AtomicUsize::new(0),
        request_seen: Notify::new(),
        cancellation_seen: Notify::new(),
        request_id: Mutex::new(None),
    });
    let router = Router::new()
        .route(
            "/mcp",
            post(handle_post)
                .get(handle_get)
                .delete(|| async { StatusCode::OK }),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let uri = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    TestServer { uri, state, task }
}

fn result(id: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {"content": [], "isError": false}
    })
}

fn sse(body: String) -> Response {
    ([("content-type", "text/event-stream")], Body::from(body)).into_response()
}

async fn handle_post(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<Value>,
) -> Response {
    match request["method"].as_str().unwrap() {
        "initialize" => {
            let mut response = Json(json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "deadline-test", "version": "1"}
                }
            }))
            .into_response();
            if matches!(state.reply, Reply::ResumableSse) {
                response
                    .headers_mut()
                    .insert("mcp-session-id", "test-session".parse().unwrap());
            }
            response
        }
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        "notifications/cancelled" => {
            state.cancellations.fetch_add(1, Ordering::SeqCst);
            state.cancellation_seen.notify_one();
            if matches!(state.reply, Reply::StuckCancellation) {
                pending().await
            } else {
                StatusCode::ACCEPTED.into_response()
            }
        }
        "tools/call" => {
            // 模拟不可重复的副作用；结果是否送达都必须只有一次。
            state.calls.fetch_add(1, Ordering::SeqCst);
            *state.request_id.lock().unwrap() = Some(request["id"].clone());
            state.request_seen.notify_one();
            if request["params"]["name"] == "succeed" {
                return Json(result(request["id"].clone())).into_response();
            }
            match state.reply {
                Reply::Success => Json(result(request["id"].clone())).into_response(),
                Reply::ProtocolError => Json(json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "error": {"code": -32602, "message": "test protocol error"}
                }))
                .into_response(),
                Reply::HttpError => StatusCode::BAD_GATEWAY.into_response(),
                Reply::WrongId | Reply::StuckCancellation => {
                    Json(result(json!("unrelated"))).into_response()
                }
                Reply::EmptySse => sse(String::new()),
                Reply::ResumableSse => {
                    let progress = json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/progress",
                        "params": {
                            "progressToken": request["params"]["_meta"]["progressToken"],
                            "progress": 1
                        }
                    });
                    sse(format!("id: resume-1\nretry: 0\ndata: {progress}\n\n"))
                }
                Reply::StuckSend => pending().await,
            }
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn handle_get(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if headers.get("last-event-id").is_none() {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    state.resumes.fetch_add(1, Ordering::SeqCst);
    let id = state.request_id.lock().unwrap().clone().unwrap();
    sse(format!("data: {}\n\n", result(id)))
}

async fn wait_for_notification(notification: &Notify) {
    tokio::time::timeout(TEST_TIMEOUT, notification.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn deadline_covers_connection_before_dispatch() {
    let result = call_tool_with_deadline_inner(
        CallToolRequestParams::new("test"),
        pending(),
        ready(()),
        pending,
    )
    .await;

    assert!(matches!(
        result,
        Err(ToolCallError::DeadlineExceeded {
            may_have_executed: false
        })
    ));
}

#[tokio::test]
async fn connection_error_is_preserved() {
    let result = call_tool_with_deadline_inner(
        CallToolRequestParams::new("test"),
        ready(Err(ServiceError::McpError(ErrorData::internal_error(
            "test connection error",
            None,
        )))),
        pending(),
        pending,
    )
    .await;

    assert!(matches!(
        result,
        Err(ToolCallError::Service(ServiceError::McpError(error)))
            if error.message == "test connection error"
    ));
}

#[tokio::test]
async fn transport_closed_before_dispatch_is_not_an_unknown_tool_outcome() {
    let result = call_tool_with_deadline_inner(
        CallToolRequestParams::new("test"),
        ready(Err(ServiceError::TransportClosed)),
        pending(),
        pending,
    )
    .await;

    assert!(matches!(
        result,
        Err(ToolCallError::Service(ServiceError::TransportClosed))
    ));
}

#[tokio::test]
async fn successful_response_is_preserved() {
    let server = start_server(Reply::Success).await;
    let service = ().serve(server.transport()).await.unwrap();

    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        call_tool_with_deadline_inner(
            CallToolRequestParams::new("test"),
            ready(Ok(service.peer().clone())),
            pending(),
            pending,
        ),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(result.is_error, Some(false));
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.state.cancellations.load(Ordering::SeqCst), 0);
    service.cancel().await.unwrap();
}

#[tokio::test]
async fn protocol_error_is_preserved_without_replay() {
    let server = start_server(Reply::ProtocolError).await;
    let service = ().serve(server.transport()).await.unwrap();

    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        call_tool_with_deadline_inner(
            CallToolRequestParams::new("test"),
            ready(Ok(service.peer().clone())),
            pending(),
            pending,
        ),
    )
    .await
    .unwrap();

    assert!(matches!(
        result,
        Err(ToolCallError::Service(ServiceError::McpError(error)))
            if error.message == "test protocol error"
    ));
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    service.cancel().await.unwrap();
}

#[tokio::test]
async fn transport_failure_after_side_effect_does_not_replay_tool() {
    let server = start_server(Reply::HttpError).await;
    let service = ().serve(server.transport()).await.unwrap();

    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        call_tool_with_deadline_inner(
            CallToolRequestParams::new("test"),
            ready(Ok(service.peer().clone())),
            pending(),
            pending,
        ),
    )
    .await
    .unwrap();

    assert!(matches!(result, Err(ToolCallError::OutcomeUnknown(_))));
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    service.cancel().await.unwrap();
}

#[tokio::test]
async fn transport_closed_after_side_effect_has_unknown_outcome_without_retry() {
    let (client, server) = tokio::io::duplex(4096);
    let calls = Arc::new(AtomicUsize::new(0));
    let server_calls = calls.clone();
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut lines = BufReader::new(reader).lines();
        let initialize: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        let response = json!({
            "jsonrpc": "2.0",
            "id": initialize["id"],
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "disconnect-test", "version": "1"}
            }
        });
        writer
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
        let initialized: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(initialized["method"], "notifications/initialized");
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["method"], "tools/call");
        server_calls.fetch_add(1, Ordering::SeqCst);
        // 已产生副作用，然后不返回结果而关闭真实传输。
    });
    let service = ().serve(client).await.unwrap();

    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        call_tool_with_deadline_inner(
            CallToolRequestParams::new("test"),
            ready(Ok(service.peer().clone())),
            pending(),
            pending,
        ),
    )
    .await
    .unwrap();

    assert!(matches!(
        result,
        Err(ToolCallError::OutcomeUnknown(ServiceError::TransportClosed))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server_task.await.unwrap();
    service.cancel().await.unwrap();
}

#[tokio::test]
async fn response_deadline_preserves_other_calls_on_the_connection() {
    let server = start_server(Reply::WrongId).await;
    let service = ().serve(server.transport()).await.unwrap();
    let peer = service.peer().clone();
    let (expire, deadline) = oneshot::channel();
    let task = tokio::spawn(call_tool_with_deadline_inner(
        CallToolRequestParams::new("test"),
        ready(Ok(peer.clone())),
        async { deadline.await.unwrap() },
        pending,
    ));
    wait_for_notification(&server.state.request_seen).await;

    let successful = tokio::time::timeout(
        TEST_TIMEOUT,
        call_tool_with_deadline_inner(
            CallToolRequestParams::new("succeed"),
            ready(Ok(peer)),
            pending(),
            pending,
        ),
    )
    .await
    .unwrap();
    assert!(successful.is_ok());
    expire.send(()).unwrap();
    let result = tokio::time::timeout(TEST_TIMEOUT, task)
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(
        result,
        Err(ToolCallError::DeadlineExceeded {
            may_have_executed: true
        })
    ));
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 2);
    assert_eq!(server.state.cancellations.load(Ordering::SeqCst), 1);
    assert!(!service.is_transport_closed());
    service.cancel().await.unwrap();
}

#[tokio::test]
async fn ended_sse_without_response_is_bounded_without_replay() {
    let server = start_server(Reply::EmptySse).await;
    let service = ().serve(server.transport()).await.unwrap();
    let (expire, deadline) = oneshot::channel();
    let task = tokio::spawn(call_tool_with_deadline_inner(
        CallToolRequestParams::new("test"),
        ready(Ok(service.peer().clone())),
        async { deadline.await.unwrap() },
        pending,
    ));
    wait_for_notification(&server.state.request_seen).await;
    expire.send(()).unwrap();

    let result = tokio::time::timeout(TEST_TIMEOUT, task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(ToolCallError::DeadlineExceeded {
            may_have_executed: true
        })
    ));
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    service.cancel().await.unwrap();
}

#[tokio::test]
async fn stalled_send_cannot_block_the_deadline() {
    let server = start_server(Reply::StuckSend).await;
    let service = ().serve(server.transport()).await.unwrap();
    let (expire, deadline) = oneshot::channel();
    let task = tokio::spawn(call_tool_with_deadline_inner(
        CallToolRequestParams::new("test"),
        ready(Ok(service.peer().clone())),
        async { deadline.await.unwrap() },
        || ready(()),
    ));
    wait_for_notification(&server.state.request_seen).await;
    expire.send(()).unwrap();

    let result = tokio::time::timeout(TEST_TIMEOUT, task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(ToolCallError::DeadlineExceeded {
            may_have_executed: true
        })
    ));
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.state.cancellations.load(Ordering::SeqCst), 0);
    service.cancellation_token().cancel();
}

#[tokio::test]
async fn stalled_cancellation_cannot_extend_its_budget() {
    let server = start_server(Reply::StuckCancellation).await;
    let service = ().serve(server.transport()).await.unwrap();
    let (expire, deadline) = oneshot::channel();
    let (cancel_expire, cancel_deadline) = oneshot::channel();
    let task = tokio::spawn(call_tool_with_deadline_inner(
        CallToolRequestParams::new("test"),
        ready(Ok(service.peer().clone())),
        async { deadline.await.unwrap() },
        || async { cancel_deadline.await.unwrap() },
    ));
    wait_for_notification(&server.state.request_seen).await;
    expire.send(()).unwrap();
    wait_for_notification(&server.state.cancellation_seen).await;
    cancel_expire.send(()).unwrap();

    let result = tokio::time::timeout(TEST_TIMEOUT, task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(ToolCallError::DeadlineExceeded {
            may_have_executed: true
        })
    ));
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.state.cancellations.load(Ordering::SeqCst), 1);
    service.cancellation_token().cancel();
}

#[tokio::test]
async fn resumable_sse_uses_get_without_replaying_the_tool() {
    let server = start_server(Reply::ResumableSse).await;
    let service = ().serve(server.transport()).await.unwrap();
    let result = tokio::time::timeout(
        TEST_TIMEOUT,
        call_tool_with_deadline_inner(
            CallToolRequestParams::new("test"),
            ready(Ok(service.peer().clone())),
            pending(),
            pending,
        ),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(result.is_error, Some(false));
    assert_eq!(server.state.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.state.resumes.load(Ordering::SeqCst), 1);
    service.cancel().await.unwrap();
}
