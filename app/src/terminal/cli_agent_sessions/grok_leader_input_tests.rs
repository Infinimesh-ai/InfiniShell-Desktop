use super::*;

fn tracker() -> DeliveryTracker {
    DeliveryTracker::new(Uuid::new_v4(), Uuid::new_v4(), None).unwrap()
}

fn final_response(record: &GrokLeaderDelivery, native_prompt_id: Uuid) -> Value {
    json!({"jsonrpc":"2.0","id":record.rpc_id.to_string(),"result":{
        "stopReason":"end_turn","_meta":{"sessionId":record.session_id.to_string(),
            "requestId":native_prompt_id.to_string(),"promptId":native_prompt_id.to_string(),
            "modelId":"grok-4.7"}}})
}

#[test]
fn persistence_failure_does_not_claim_input() {
    let mut tracker = tracker();

    let error =
        tracker.record_before_write(Uuid::new_v4(), |_| Err(io::Error::other("测试事务失败")));

    assert!(matches!(error, Err(GrokLeaderInputError::Persistence(_))));
    assert_eq!(tracker.delivery, None);
}

#[test]
fn record_is_unknown_before_any_write_and_cannot_be_claimed_again() {
    let mut tracker = tracker();
    let record = tracker
        .record_before_write(Uuid::new_v4(), |record| {
            assert_eq!(record.status, GrokLeaderDeliveryStatus::Unknown);
            Ok(())
        })
        .unwrap()
        .clone();

    let second = tracker.record_before_write(record.message_id, |_| {
        panic!("已经领取的输入不得再次进入持久化或发送流程")
    });

    assert!(matches!(
        second,
        Err(GrokLeaderInputError::DeliveryAlreadyRecorded)
    ));
    assert_eq!(tracker.delivery, Some(record));
}

#[test]
fn restored_unknown_input_is_observation_only() {
    let mut previous = tracker();
    let record = previous
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    let mut resumed =
        DeliveryTracker::new(record.binding_id, record.session_id, Some(record.clone())).unwrap();

    assert!(matches!(
        resumed.record_before_write(record.message_id, |_| {
            panic!("恢复观察不能重投未知输入")
        }),
        Err(GrokLeaderInputError::DeliveryAlreadyRecorded)
    ));
    assert_eq!(resumed.delivery, Some(record));
}

#[test]
fn restored_record_must_belong_to_current_binding() {
    let mut previous = tracker();
    let record = previous
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();

    assert!(matches!(
        DeliveryTracker::new(Uuid::new_v4(), record.session_id, Some(record)),
        Err(GrokLeaderInputError::InvalidTarget)
    ));
}

#[test]
fn stale_input_generation_is_rejected() {
    assert!(matches!(
        tracker().check_binding(Uuid::new_v4()),
        Err(GrokLeaderInputError::StaleBinding)
    ));
}

#[test]
fn queue_and_native_completion_notification_do_not_acknowledge_input() {
    let mut tracker = tracker();
    let record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    let queue = json!({"jsonrpc":"2.0","method":"_x.ai/queue/changed","params":{
        "sessionId":record.session_id.to_string(),"entries":[],"runningPromptId":Uuid::new_v4(),
        "runningText":"相同文本也不能绑定 RPC"}});
    let completed = json!({"jsonrpc":"2.0","method":"_x.ai/session/prompt_complete","params":{
        "sessionId":record.session_id.to_string(),"promptId":Uuid::new_v4(),"stopReason":"end_turn"}});

    assert_eq!(tracker.observe(&queue).unwrap(), None);
    assert_eq!(tracker.observe(&completed).unwrap(), None);
    assert_eq!(tracker.delivery, Some(record));
}

#[test]
fn exact_final_response_binds_native_prompt_id_once() {
    let mut tracker = tracker();
    let mut record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    let native_prompt_id = Uuid::new_v4();
    let response = final_response(&record, native_prompt_id);
    record.status = GrokLeaderDeliveryStatus::Finished {
        native_prompt_id,
        outcome: GrokLeaderOutcome::EndTurn,
    };

    assert_eq!(
        tracker.observe(&response).unwrap(),
        Some(GrokLeaderInputEvent::Delivery(record.clone()))
    );
    assert_eq!(tracker.observe(&response).unwrap(), None);
    assert_eq!(tracker.delivery, Some(record));
}

#[test]
fn second_native_prompt_for_same_rpc_is_not_treated_as_duplicate_ack() {
    let mut tracker = tracker();
    let record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    tracker
        .observe(&final_response(&record, Uuid::new_v4()))
        .unwrap();
    let first = tracker.delivery.clone();

    assert!(matches!(
        tracker.observe(&final_response(&record, Uuid::new_v4())),
        Err(GrokLeaderInputError::Protocol)
    ));
    assert_eq!(tracker.delivery, first);
}

#[test]
fn unrelated_rpc_response_preserves_unknown_input() {
    let mut tracker = tracker();
    let record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    let mut response = final_response(&record, Uuid::new_v4());
    response["id"] = json!(Uuid::new_v4().to_string());

    assert_eq!(tracker.observe(&response).unwrap(), None);
    assert_eq!(tracker.delivery, Some(record));
}

#[test]
fn wrong_native_session_preserves_unknown_input() {
    let mut tracker = tracker();
    let record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    let mut response = final_response(&record, Uuid::new_v4());
    response["result"]["_meta"]["sessionId"] = json!(Uuid::new_v4().to_string());

    assert!(matches!(
        tracker.observe(&response),
        Err(GrokLeaderInputError::Protocol)
    ));
    assert_eq!(tracker.delivery, Some(record));
}

#[test]
fn missing_native_prompt_binding_preserves_unknown_input() {
    let mut tracker = tracker();
    let record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    let mut response = final_response(&record, Uuid::new_v4());
    response["result"]["_meta"]["requestId"] = Value::Null;

    assert!(matches!(
        tracker.observe(&response),
        Err(GrokLeaderInputError::Protocol)
    ));
    assert_eq!(tracker.delivery, Some(record));
}

#[test]
fn native_error_does_not_prove_input_was_never_executed() {
    let mut tracker = tracker();
    let record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();

    assert!(matches!(
        tracker.observe(&json!({"jsonrpc":"2.0","id":record.rpc_id.to_string(),
        "error":{"code":-32603,"message":"不应进入产品日志的原生正文"}})),
        Err(GrokLeaderInputError::NativeRequestFailed { code: Some(-32603) })
    ));
    assert_eq!(tracker.delivery, Some(record));
}

#[test]
fn native_cancelled_response_is_a_bound_terminal_outcome() {
    let mut tracker = tracker();
    let mut record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    let native_prompt_id = Uuid::new_v4();
    let mut response = final_response(&record, native_prompt_id);
    response["result"]["stopReason"] = json!("cancelled");
    record.status = GrokLeaderDeliveryStatus::Finished {
        native_prompt_id,
        outcome: GrokLeaderOutcome::Cancelled,
    };

    assert_eq!(
        tracker.observe(&response).unwrap(),
        Some(GrokLeaderInputEvent::Delivery(record))
    );
}

#[test]
fn permission_request_is_observed_without_marking_delivery_complete() {
    let mut tracker = tracker();
    let record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    let permission = json!({"jsonrpc":"2.0","id":0,"method":"session/request_permission",
        "params":{"sessionId":record.session_id.to_string(),"toolCall":{"toolCallId":"原生工具-1"},
            "options":[{"optionId":"allow_once","kind":"allow_once"}]}});

    assert_eq!(
        tracker.observe(&permission).unwrap(),
        Some(GrokLeaderInputEvent::NativePermissionPending {
            tool_call_id: "原生工具-1".to_owned(),
        })
    );
    assert_eq!(tracker.delivery, Some(record));
}

#[test]
fn fragmented_frame_preserves_multiline_chinese_and_file_path() {
    let rpc = json!({"jsonrpc":"2.0","method":"session/prompt","params":{"prompt":[{
        "type":"text","text":"请读取 /private/tmp/中文 目录/附件.txt\n保留第二行。"}]}});
    let bytes = encode_frame(&acp_frame(rpc.clone())).unwrap();
    let mut decoder = FrameDecoder::default();
    decoder.append(&bytes[..2]).unwrap();
    assert_eq!(decoder.next().unwrap(), None);
    decoder.append(&bytes[2..13]).unwrap();
    assert_eq!(decoder.next().unwrap(), None);
    decoder.append(&bytes[13..]).unwrap();

    assert_eq!(decode_acp(&decoder.next().unwrap().unwrap()).unwrap(), rpc);
    assert_eq!(decoder.next().unwrap(), None);
}

#[test]
fn oversized_length_is_rejected_before_allocating_payload() {
    let mut decoder = FrameDecoder::default();
    decoder.append(&[0xff, 0xff, 0xff, 0xff]).unwrap();

    assert!(matches!(
        decoder.next(),
        Err(GrokLeaderInputError::FrameTooLarge)
    ));
}

#[test]
fn invalid_json_frame_is_rejected() {
    let mut decoder = FrameDecoder::default();
    decoder.append(&[0, 0, 0, 2, b'{', b'!']).unwrap();

    assert!(matches!(
        decoder.next(),
        Err(GrokLeaderInputError::Protocol)
    ));
}

#[test]
fn consecutive_frames_are_decoded_independently() {
    let mut decoder = FrameDecoder::default();
    decoder
        .append(&encode_frame(&json!({"type":"registered"})).unwrap())
        .unwrap();
    decoder
        .append(&encode_frame(&json!({"type":"leader_ready"})).unwrap())
        .unwrap();

    assert_eq!(decoder.next().unwrap(), Some(json!({"type":"registered"})));
    assert_eq!(
        decoder.next().unwrap(),
        Some(json!({"type":"leader_ready"}))
    );
    assert_eq!(decoder.next().unwrap(), None);
}

#[test]
fn only_valid_final_response_retains_raw_ack_for_sqlite_validation() {
    let mut tracker = tracker();
    let record = tracker
        .record_before_write(Uuid::new_v4(), |_| Ok(()))
        .unwrap()
        .clone();
    let mut response = final_response(&record, Uuid::new_v4());
    response["result"]["_meta"]["test_preserved_field"] = json!({"原始":"不能用状态重建"});
    let mut mismatched = response.clone();
    mismatched["result"]["_meta"]["sessionId"] = json!(Uuid::new_v4());
    assert!(tracker.observe(&mismatched).is_err());
    assert_eq!(tracker.final_response, None);
    assert!(matches!(
        tracker.observe(&response).unwrap(),
        Some(GrokLeaderInputEvent::Delivery(_))
    ));
    assert_eq!(tracker.final_response.take(), Some(response.clone()));
    assert_eq!(tracker.observe(&response).unwrap(), None);
    assert_eq!(tracker.final_response, None);
}
