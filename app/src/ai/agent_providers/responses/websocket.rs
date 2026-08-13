use futures::{SinkExt, StreamExt};
use serde_json::Value;
use websocket::{Message, WebSocket, WebsocketMessage};

#[cfg(not(target_family = "wasm"))]
use std::collections::HashMap;
#[cfg(not(target_family = "wasm"))]
use std::sync::LazyLock;

use super::{
    ResponseCreateRequest, ResponseItem, ResponseStreamEvent, ResponsesClient, ResponsesClientError,
};

/// Responses WebSocket 长连接。一个连接可顺序执行多轮 `response.create`，
/// 并利用连接内最近响应缓存继续 `store:false` 会话。
pub struct ResponsesWebSocket {
    sink: Box<dyn websocket::Sink>,
    stream: Box<dyn websocket::Stream>,
}

#[cfg(not(target_family = "wasm"))]
static CONNECTION_POOL: LazyLock<tokio::sync::Mutex<HashMap<String, ResponsesWebSocket>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

impl ResponsesWebSocket {
    #[cfg(not(target_family = "wasm"))]
    pub async fn connect(client: &ResponsesClient) -> Result<Self, ResponsesWebSocketError> {
        let url = client.websocket_url()?;
        let socket = WebSocket::connect_with_headers(
            url.as_str(),
            std::iter::empty::<&str>(),
            client.websocket_headers(),
        )
        .await
        .map_err(ResponsesWebSocketError::Connect)?;
        let (sink, stream) = socket.split().await;
        Ok(Self {
            sink: Box::new(sink),
            stream: Box::new(stream),
        })
    }

    #[cfg(not(target_family = "wasm"))]
    pub async fn connect_reused(
        client: &ResponsesClient,
    ) -> Result<(Self, String), ResponsesWebSocketError> {
        let key = client.websocket_connection_key();
        if let Some(socket) = CONNECTION_POOL.lock().await.remove(&key) {
            return Ok((socket, key));
        }
        Ok((Self::connect(client).await?, key))
    }

    #[cfg(target_family = "wasm")]
    pub async fn connect_reused(
        client: &ResponsesClient,
    ) -> Result<(Self, String), ResponsesWebSocketError> {
        Ok((Self::connect(client).await?, String::new()))
    }

    #[cfg(not(target_family = "wasm"))]
    pub async fn recycle(self, key: String) {
        let mut pool = CONNECTION_POOL.lock().await;
        if pool.len() < 8 {
            pool.insert(key, self);
        }
    }

    #[cfg(target_family = "wasm")]
    pub async fn recycle(self, _key: String) {}

    #[cfg(target_family = "wasm")]
    pub async fn connect(_client: &ResponsesClient) -> Result<Self, ResponsesWebSocketError> {
        Err(ResponsesWebSocketError::UnsupportedOnWasm)
    }

    pub async fn create(
        &mut self,
        request: ResponseCreateRequest,
    ) -> Result<(), ResponsesWebSocketError> {
        request.validate_websocket()?;
        let mut value = serde_json::to_value(request).map_err(ResponsesWebSocketError::Encode)?;
        let object = value
            .as_object_mut()
            .ok_or(ResponsesWebSocketError::InvalidClientEvent)?;
        object.insert(
            "type".to_owned(),
            Value::String("response.create".to_owned()),
        );
        self.send(value).await
    }

    pub async fn inject(
        &mut self,
        items: Vec<ResponseItem>,
    ) -> Result<(), ResponsesWebSocketError> {
        self.send(serde_json::json!({
            "type": "response.inject",
            "items": items,
        }))
        .await
    }

    pub async fn next_event(&mut self) -> Result<ResponseStreamEvent, ResponsesWebSocketError> {
        loop {
            let Some(message) = self.stream.next().await else {
                return Err(ResponsesWebSocketError::Ended);
            };
            let message = message.map_err(ResponsesWebSocketError::Receive)?;
            let text = if let Some(text) = message.text() {
                text
            } else if let Some(bytes) = message.binary() {
                std::str::from_utf8(bytes).map_err(ResponsesWebSocketError::Utf8)?
            } else {
                continue;
            };
            let raw = serde_json::from_str(text).map_err(ResponsesWebSocketError::Decode)?;
            return ResponseStreamEvent::from_value(raw).map_err(ResponsesWebSocketError::Protocol);
        }
    }

    async fn send(&mut self, value: Value) -> Result<(), ResponsesWebSocketError> {
        let text = serde_json::to_string(&value).map_err(ResponsesWebSocketError::Encode)?;
        self.sink
            .send(Message::new_text(text))
            .await
            .map_err(ResponsesWebSocketError::Send)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResponsesWebSocketError {
    #[error(transparent)]
    Client(#[from] ResponsesClientError),
    #[error(transparent)]
    InvalidRequest(#[from] super::ResponseRequestValidationError),
    #[error("当前 WASM 运行时不能在 WebSocket 握手中安全发送 provider API key")]
    UnsupportedOnWasm,
    #[error("Responses WebSocket 连接失败")]
    Connect(#[source] anyhow::Error),
    #[error("Responses WebSocket 发送失败")]
    Send(#[source] websocket::Error),
    #[error("Responses WebSocket 接收失败")]
    Receive(#[source] websocket::Error),
    #[error("Responses WebSocket 已关闭")]
    Ended,
    #[error("Responses WebSocket 客户端事件必须是 JSON object")]
    InvalidClientEvent,
    #[error("Responses WebSocket 编码失败")]
    Encode(#[source] serde_json::Error),
    #[error("Responses WebSocket 返回无效 JSON")]
    Decode(#[source] serde_json::Error),
    #[error("Responses WebSocket 返回无效 UTF-8")]
    Utf8(#[source] std::str::Utf8Error),
    #[error(transparent)]
    Protocol(#[from] super::ResponseProtocolError),
}
