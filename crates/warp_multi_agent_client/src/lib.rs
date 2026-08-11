//! Warp 云端 multi-agent(Agent Mode)网关的协议辅助层。
//!
//! Zap 是本地优先 fork,不提供强制云端:上游的 `warp_server_client::base_client::BaseClient`
//! (账号鉴权 + ambient headers + IAP 探测)已随 `warp_server_client` 一并剥离,因此这里
//! **不再发起任何云端 RPC**。保留下来的是与云端无关的纯逻辑:错误类型、事件流类型别名、
//! 请求分类、端点拼接与 protobuf/base64 事件解码——它们对本地/自建网关同样有意义。
//! 入口 `generate_multi_agent_output` 保留签名(去掉已剥离的 client 参数)并做成 no-op。

use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE;
use futures::StreamExt as _;
use prost::Message as _;
use warp_core::channel::ChannelState;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to authenticate multi-agent request")]
    Authentication(#[source] anyhow::Error),

    #[error("Failed to resolve ambient headers for multi-agent request")]
    AmbientHeaders(#[source] anyhow::Error),

    #[error("Failed to decode base64 multi-agent response event")]
    Base64Decode(#[source] base64::DecodeError),

    #[error("Failed to decode protobuf multi-agent response event")]
    ProtobufDecode(#[source] prost::DecodeError),

    #[error("Multi-agent eventsource stream failed: {0:?}")]
    EventSource(Box<reqwest_eventsource::Error>),
}

cfg_if::cfg_if! {
    if #[cfg(target_family = "wasm")] {
        /// A multi-agent response event stream without an unnecessary `Send` bound on WASM.
        pub type OutputStream = futures::stream::LocalBoxStream<
            'static,
            Result<warp_multi_agent_api::ResponseEvent, Error>,
        >;
    } else {
        /// A multi-agent response event stream that can be sent between native threads.
        pub type OutputStream = futures::stream::BoxStream<
            'static,
            Result<warp_multi_agent_api::ResponseEvent, Error>,
        >;
    }
}

/// Opens a decoded multi-agent response event stream.
///
/// Zap 剥离了 Warp 云端 agent 网关(需要账号鉴权与云端 RPC),此处保留签名但不发起
/// 任何网络请求,直接返回一个立即结束的空事件流。调用方按“无结果”处理即可。
pub async fn generate_multi_agent_output(
    request: &warp_multi_agent_api::Request,
) -> Result<OutputStream, Error> {
    tracing::debug!(
        is_passive = is_passive_suggestion_request(request),
        "InfiniShell 未启用云端 multi-agent 网关,返回空事件流"
    );

    let output_stream = futures::stream::empty();

    cfg_if::cfg_if! {
        if #[cfg(target_family = "wasm")] {
            Ok(output_stream.boxed_local())
        } else {
            Ok(output_stream.boxed())
        }
    }
}

/// Whether the request only asks for passive suggestions (read-only for the conversation).
pub fn is_passive_suggestion_request(request: &warp_multi_agent_api::Request) -> bool {
    request.input.as_ref().is_some_and(|input| {
        matches!(
            input.r#type,
            Some(warp_multi_agent_api::request::input::Type::GeneratePassiveSuggestions(_))
        )
    })
}

/// Builds the endpoint URL for a multi-agent request against the configured server root.
pub fn endpoint_url(is_passive: bool) -> String {
    format!(
        "{}/{}/{}",
        ChannelState::server_root_url(),
        if cfg!(feature = "agent_mode_evals") {
            "agent-mode-evals"
        } else {
            "ai"
        },
        if is_passive {
            "passive-suggestions"
        } else {
            "multi-agent"
        }
    )
}

/// Decodes one SSE payload (a quoted, URL-safe base64 protobuf blob) into a response event.
pub fn decode_response_event(data: &str) -> Result<warp_multi_agent_api::ResponseEvent, Error> {
    let decoded_data = BASE64_URL_SAFE
        .decode(data.trim_matches('"'))
        .map_err(Error::Base64Decode)?;
    warp_multi_agent_api::ResponseEvent::decode(decoded_data.as_slice())
        .map_err(Error::ProtobufDecode)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
