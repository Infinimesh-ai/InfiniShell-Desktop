use super::*;

fn item(id: &str, kind: &str) -> ResponseItem {
    ResponseItem::from_value(serde_json::json!({ "id": id, "type": kind })).unwrap()
}

#[test]
fn local_http_mode_replays_items_and_disables_storage() {
    let state = ResponseSessionState {
        current_response_id: Some("resp_previous".to_owned()),
        replay_items: vec![item("reasoning_1", "reasoning")],
        ..Default::default()
    };
    let mut request = ResponseCreateRequest::local_private(
        "gpt-test",
        serde_json::json!([{ "role": "user", "content": "next" }]),
    );

    state
        .prepare_request(&mut request, ResponseTransportMode::Http)
        .unwrap();

    assert_eq!(request.store, Some(false));
    assert!(request.previous_response_id.is_none());
    assert_eq!(request.input.unwrap().as_array().unwrap().len(), 2);
    assert_eq!(
        request.include.unwrap(),
        vec!["reasoning.encrypted_content".to_owned()]
    );
}

#[test]
fn websocket_local_mode跨连接仍完整回放() {
    let state = ResponseSessionState {
        current_response_id: Some("resp_previous".to_owned()),
        replay_items: vec![item("reasoning_1", "reasoning")],
        ..Default::default()
    };
    let mut request = ResponseCreateRequest::local_private("gpt-test", "next");

    state
        .prepare_request(&mut request, ResponseTransportMode::WebSocket)
        .unwrap();

    assert!(request.previous_response_id.is_none());
    assert_eq!(request.store, Some(false));
    assert_eq!(request.input.unwrap().as_array().unwrap().len(), 2);
}

#[test]
fn duplicate_sequence_is_ignored_and_gap_is_detected() {
    let mut state = ResponseSessionState::default();
    let event = |sequence_number| {
        ResponseStreamEvent::from_value(serde_json::json!({
            "type": "response.output_text.delta",
            "sequence_number": sequence_number,
            "delta": "x"
        }))
        .unwrap()
    };

    assert_eq!(
        state.record_event(event(1)),
        ResponseEventDisposition::Applied
    );
    assert_eq!(
        state.record_event(event(1)),
        ResponseEventDisposition::Duplicate
    );
    assert_eq!(
        state.record_event(event(3)),
        ResponseEventDisposition::GapDetected
    );
}

#[test]
fn compaction_replaces_canonical_window_instead_of_appending() {
    let mut state = ResponseSessionState {
        replay_items: vec![item("old", "message")],
        current_response_id: Some("resp_old".to_owned()),
        ..Default::default()
    };

    state.replace_with_compaction(vec![item("compact", "compaction")]);

    assert_eq!(state.replay_items.len(), 1);
    assert_eq!(state.replay_items[0].id.as_deref(), Some("compact"));
    assert!(state.current_response_id.is_none());
}

#[test]
fn context_fingerprint_is_independent_of_object_key_order() {
    let left = vec![serde_json::json!({ "type": "function", "name": "x" })];
    let right = vec![serde_json::json!({ "name": "x", "type": "function" })];

    assert_eq!(
        response_request_context_fingerprint("endpoint", Some("model"), Some("i"), Some(&left)),
        response_request_context_fingerprint("endpoint", Some("model"), Some("i"), Some(&right)),
    );
}
