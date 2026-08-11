use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE;
use prost::Message as _;

use super::{Error, decode_response_event, endpoint_url, is_passive_suggestion_request};

#[test]
fn detects_passive_suggestion_requests() {
    let regular = warp_multi_agent_api::Request::default();
    let passive = warp_multi_agent_api::Request {
        input: Some(warp_multi_agent_api::request::Input {
            r#type: Some(
                warp_multi_agent_api::request::input::Type::GeneratePassiveSuggestions(
                    Default::default(),
                ),
            ),
            ..Default::default()
        }),
        ..Default::default()
    };

    assert!(!is_passive_suggestion_request(&regular));
    assert!(is_passive_suggestion_request(&passive));
}

#[test]
fn routes_regular_and_passive_requests_to_distinct_endpoints() {
    let prefix = if cfg!(feature = "agent_mode_evals") {
        "agent-mode-evals"
    } else {
        "ai"
    };

    assert!(endpoint_url(false).ends_with(&format!("/{prefix}/multi-agent")));
    assert!(endpoint_url(true).ends_with(&format!("/{prefix}/passive-suggestions")));
}

/// Zap 不接云端 agent 网关:入口必须是 no-op,返回一个立即结束的空事件流。
#[cfg(not(target_family = "wasm"))]
#[test]
fn cloud_multi_agent_entry_point_is_a_no_op() {
    use futures::StreamExt as _;

    use super::generate_multi_agent_output;

    let events = futures::executor::block_on(async {
        let stream = generate_multi_agent_output(&warp_multi_agent_api::Request::default())
            .await
            .unwrap();
        stream.collect::<Vec<_>>().await
    });

    assert!(events.is_empty());
}

#[test]
fn decodes_quoted_base64_protobuf_response_event() {
    let expected = warp_multi_agent_api::ResponseEvent::default();
    let encoded = BASE64_URL_SAFE.encode(expected.encode_to_vec());

    let decoded = decode_response_event(&format!("\"{encoded}\"")).unwrap();

    assert_eq!(decoded, expected);
}

#[test]
fn distinguishes_base64_and_protobuf_decode_errors() {
    assert!(matches!(
        decode_response_event("%"),
        Err(Error::Base64Decode(_))
    ));

    let invalid_protobuf = BASE64_URL_SAFE.encode([0xff]);
    assert!(matches!(
        decode_response_event(&invalid_protobuf),
        Err(Error::ProtobufDecode(_))
    ));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn native_output_stream_is_send() {
    fn assert_send<T: Send>() {}

    assert_send::<super::OutputStream>();
}
