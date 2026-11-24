use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::IntoResponse as _;
use axum::response::sse::{Event, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use cloud_object_models::{ServerSentEvents, StaticHeader, TransportType};
use futures::StreamExt as _;
use serde_json::{Value, json};
use simple_logger::manager::LogManager;
use uuid::Uuid;
use warpui::r#async::executor::Background;

use super::spawn_server;

fn rpc_response(request: &Value) -> Option<Value> {
    let result = match request["method"].as_str() {
        Some("initialize") => json!({
            "protocolVersion": request["params"]["protocolVersion"],
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "user-agent-fixture", "version": "1.0" },
        }),
        Some("tools/list") => json!({ "tools": [{
            "name": "user-agent-check", "inputSchema": { "type": "object" },
        }] }),
        Some("notifications/initialized") => return None,
        method => panic!("测试服务收到未知方法：{method:?}"),
    };
    Some(json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }))
}

/// 以真实运输完成初始化和工具发现，覆盖预检与后续请求使用不同 client 的回归。
async fn assert_startup_headers(
    legacy_sse: bool,
    configured: &[(&str, &str)],
    expected_user_agent: &str,
) {
    crate::install_test_crypto_provider();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("测试服务器应能绑定");
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let app = if legacy_sse {
        let (events_tx, events_rx) = async_channel::unbounded::<Value>();
        Router::new()
            .route(
                "/mcp",
                get(move || {
                    let events = events_rx.clone();
                    async move {
                        let endpoint = futures::stream::once(async {
                            Ok::<_, Infallible>(
                                Event::default().event("endpoint").data("/messages"),
                            )
                        });
                        Sse::new(endpoint.chain(events.map(|response| {
                            Ok(Event::default().event("message").data(response.to_string()))
                        })))
                    }
                })
                .post(|| async { StatusCode::NOT_FOUND }),
            )
            .route(
                "/messages",
                post(move |Json(request): Json<Value>| {
                    let events = events_tx.clone();
                    async move {
                        if let Some(response) = rpc_response(&request) {
                            events.send(response).await.expect("SSE 接收端应保持连接");
                        }
                        StatusCode::ACCEPTED
                    }
                }),
            )
    } else {
        Router::new().route(
            "/mcp",
            post(|Json(request): Json<Value>| async move {
                match rpc_response(&request) {
                    Some(response) => Json(response).into_response(),
                    None => StatusCode::ACCEPTED.into_response(),
                }
            }),
        )
    }
    .layer(middleware::from_fn(move |request: Request, next: Next| {
        captured.lock().expect("请求记录锁应可用").push((
            request.method().clone(),
            request.uri().path().to_string(),
            request.headers().clone(),
        ));
        async move {
            // 模拟拒绝空 User-Agent 的网关；预检成功不能掩盖运输层丢失请求头。
            if request
                .headers()
                .get(header::USER_AGENT)
                .is_none_or(|value| value.is_empty())
            {
                return StatusCode::BAD_REQUEST.into_response();
            }
            next.run(request).await
        }
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("测试服务器应正常运行");
    });

    let mut logs = LogManager::new();
    logs.register_namespace("mcp-tests", false);
    let executor = Arc::new(Background::new(1, |index| {
        format!("mcp-test-logger-{index}")
    }));
    // 日志保留在本轮 TMPDIR，交由测试运行器在进程退出后统一归档和清理。
    let logger = logs
        .register(
            "mcp-tests",
            std::env::temp_dir().join(format!("mcp-user-agent-{}.log", Uuid::new_v4())),
            executor.clone(),
        )
        .expect("测试日志应可注册");
    // 系统 HTTP 客户端初始化会同步读取平台代理配置；护栏需覆盖其并发开销。
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        spawn_server(
            "user-agent-fixture".to_string(),
            None,
            Uuid::new_v4(),
            TransportType::ServerSentEvents(ServerSentEvents {
                url,
                headers: configured
                    .iter()
                    .map(|(name, value)| StaticHeader {
                        name: (*name).to_string(),
                        value: (*value).to_string(),
                    })
                    .collect(),
            }),
            logger.clone(),
            None,
        ),
    )
    .await;
    let info = result.expect("MCP 启动不应超时").expect("MCP 启动应成功");
    assert_eq!(
        info.tools
            .iter()
            .map(|tool| &*tool.name)
            .collect::<Vec<_>>(),
        vec!["user-agent-check"]
    );
    info.service.cancel().await.expect("MCP 服务应能关闭");
    logger.close();
    server.abort();
    let _ = server.await;

    let requests = requests.lock().expect("请求记录锁应可用");
    assert!(requests.len() >= 4, "应覆盖预检、初始化和工具发现请求");
    if legacy_sse {
        assert!(
            requests
                .iter()
                .any(|(method, path, _)| method == Method::GET && path == "/mcp")
        );
        assert!(
            requests
                .iter()
                .any(|(method, path, _)| method == Method::POST && path == "/messages")
        );
    }
    for (_, _, headers) in requests.iter() {
        assert_eq!(
            headers.get(header::USER_AGENT).unwrap(),
            expected_user_agent
        );
        assert_configured_headers(headers, configured);
    }
}

fn assert_configured_headers(actual: &HeaderMap, configured: &[(&str, &str)]) {
    for (name, value) in configured {
        assert_eq!(actual.get(*name).unwrap(), *value, "配置头 {name} 应保留");
    }
}

#[tokio::test]
async fn http_startup_sends_default_user_agent_without_configured_headers() {
    assert_startup_headers(false, &[], "InfiniShell-MCP-Client/1.0").await;
}

#[tokio::test]
async fn http_startup_sends_default_user_agent_with_static_credentials() {
    assert_startup_headers(
        false,
        &[
            ("Authorization", "Bearer test-token"),
            ("X-Api-Key", "test-key"),
        ],
        "InfiniShell-MCP-Client/1.0",
    )
    .await;
}

#[tokio::test]
async fn http_startup_preserves_configured_user_agent() {
    assert_startup_headers(
        false,
        &[("uSeR-aGeNt", "custom-agent/2.0")],
        "custom-agent/2.0",
    )
    .await;
}

#[tokio::test]
async fn sse_startup_sends_default_user_agent_without_configured_headers() {
    assert_startup_headers(true, &[], "InfiniShell-MCP-Client/1.0").await;
}

#[tokio::test]
async fn sse_startup_sends_default_user_agent_with_static_credentials() {
    assert_startup_headers(
        true,
        &[
            ("Authorization", "Bearer test-token"),
            ("X-Api-Key", "test-key"),
        ],
        "InfiniShell-MCP-Client/1.0",
    )
    .await;
}

#[tokio::test]
async fn sse_startup_preserves_configured_user_agent() {
    assert_startup_headers(
        true,
        &[("uSeR-aGeNt", "custom-agent/2.0")],
        "custom-agent/2.0",
    )
    .await;
}
