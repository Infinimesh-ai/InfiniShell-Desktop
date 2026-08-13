use super::*;

#[test]
fn unknown_item_roundtrips_without_losing_fields() {
    let raw = serde_json::json!({
        "type": "future_tool_call",
        "id": "item_1",
        "future_field": { "nested": true }
    });

    let item = ResponseItem::from_value(raw.clone()).unwrap();

    assert_eq!(
        item.kind,
        ResponseItemKind::Unknown("future_tool_call".to_owned())
    );
    assert_eq!(serde_json::to_value(item).unwrap(), raw);
}

#[test]
fn unknown_event_roundtrips_and_keeps_sequence_number() {
    let raw = serde_json::json!({
        "type": "response.future.delta",
        "sequence_number": 42,
        "delta": "value"
    });

    let event = ResponseStreamEvent::from_value(raw.clone()).unwrap();

    assert_eq!(event.sequence_number, Some(42));
    assert_eq!(
        event.kind,
        ResponseEventKind::Unknown("response.future.delta".to_owned())
    );
    assert_eq!(serde_json::to_value(event).unwrap(), raw);
}

#[test]
fn create_request_rejects_two_state_handles() {
    let request = ResponseCreateRequest {
        previous_response_id: Some("resp_1".to_owned()),
        conversation: Some(serde_json::json!("conv_1")),
        ..Default::default()
    };

    assert_eq!(
        request.validate(),
        Err(ResponseRequestValidationError::ConflictingStateHandles)
    );
}

#[test]
fn terminal_status_classification_is_explicit() {
    for status in ["completed", "failed", "incomplete", "cancelled"] {
        let response: ResponseObject = serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "status": status,
            "output": []
        }))
        .unwrap();
        assert!(response.is_terminal());
        assert_eq!(response.is_success(), status == "completed");
    }
}
