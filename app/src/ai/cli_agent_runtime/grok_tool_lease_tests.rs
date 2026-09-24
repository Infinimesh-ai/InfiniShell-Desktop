use super::*;

fn ledger(generation: Uuid) -> GrokToolLeaseLedger {
    let mut ledger = GrokToolLeaseLedger::new(
        generation,
        generation,
        format!("infinishell-{generation}"),
        "infinishell-local-tasks".into(),
        "native-session".into(),
    )
    .unwrap();
    ledger.begin_turn(generation, "native-turn").unwrap();
    ledger
}

fn initial(call: &str, event: &str) -> Value {
    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"native-session",
        "_meta":{"promptId":"native-turn","eventId":event},"update":{"sessionUpdate":"tool_call",
        "toolCallId":call,"_meta":{"x.ai/tool":{"name":"use_tool"}},
        "rawInput":{"tool_name":"infinishell-local-tasks__inspect_local_tasks","tool_input":{}}}}})
}

fn final_input(call: &str, event: &str) -> Value {
    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"native-session",
        "_meta":{"promptId":"native-turn","eventId":event},"update":{"sessionUpdate":"tool_call_update",
        "toolCallId":call,"kind":"other","rawInput":{"tool_name":"infinishell-local-tasks__inspect_local_tasks",
        "tool_input":{},"variant":"UseTool"}}}})
}

fn permission(call: &str, id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"session/request_permission","params":{"sessionId":"native-session",
        "toolCall":{"toolCallId":call,"kind":"other","rawInput":{
        "tool_name":"infinishell-local-tasks__inspect_local_tasks","tool_input":{},"variant":"UseTool"}}}})
}

fn ready(
    ledger: &mut GrokToolLeaseLedger,
    call: &str,
    first: &str,
    final_id: &str,
    approval: u64,
    now: Instant,
) {
    ledger
        .observe_native_tool(&initial(call, first), now)
        .unwrap();
    ledger
        .observe_native_tool(&final_input(call, final_id), now)
        .unwrap();
    ledger
        .record_permission(&permission(call, approval), true, now)
        .unwrap();
}

fn sdk(generation: Uuid, outer: u64, inner: u64) -> Value {
    json!({"jsonrpc":"2.0","id":outer,"method":"_x.ai/mcp/sdk_call","params":{
        "serverId":format!("infinishell-{generation}"),"message":{"jsonrpc":"2.0","id":inner,
        "method":"tools/call","params":{"name":"inspect_local_tasks","arguments":{},"_meta":{}}}}})
}

fn completed(call: &str) -> Value {
    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"native-session",
        "_meta":{"promptId":"native-turn","eventId":"event-complete"},"update":{
        "sessionUpdate":"tool_call_update","toolCallId":call,"status":"completed"}}})
}

#[test]
fn native_lease_requires_real_reply_before_out_of_order_completion_can_close_it() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    ready(&mut ledger, "native-call", "initial", "final", 1, now);
    let proof = ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap();
    assert_eq!(proof.native_call_id(), "native-call");
    assert_eq!(proof.turn_id(), "native-turn");
    assert_eq!(ledger.state(&proof).unwrap(), GrokToolLeaseState::Bound);
    ledger
        .observe_native_tool(&completed("native-call"), now)
        .unwrap();
    assert_eq!(ledger.state(&proof).unwrap(), GrokToolLeaseState::Bound);
    ledger.record_reply_written(&proof, true).unwrap();
    assert_eq!(ledger.state(&proof).unwrap(), GrokToolLeaseState::Closed);
}

#[test]
fn written_business_error_closes_only_its_native_call_and_allows_the_next_turn() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    ready(&mut ledger, "native-call", "initial", "final", 1, now);
    let proof = ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap();
    ledger.record_reply_written(&proof, false).unwrap();
    let mut failed = completed("native-call");
    failed["params"]["update"]["status"] = json!("failed");
    ledger.observe_native_tool(&failed, now).unwrap();
    ledger.observe_native_tool(&failed, now).unwrap();
    assert_eq!(ledger.state(&proof).unwrap(), GrokToolLeaseState::Closed);
    assert!(!ledger.is_retired());
    ledger.record_reply_written(&proof, false).unwrap();
    assert!(ledger.record_reply_written(&proof, true).is_err());
    ledger.begin_turn(Uuid::nil(), "next-native-turn").unwrap();
}

#[test]
fn native_failure_without_a_matching_written_business_error_retires_capability() {
    let now = Instant::now();
    for written in [None, Some(true)] {
        let mut ledger = ledger(Uuid::nil());
        ready(&mut ledger, "native-call", "initial", "final", 1, now);
        let proof = ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap();
        if let Some(success) = written {
            ledger.record_reply_written(&proof, success).unwrap();
        }
        let mut failed = completed("native-call");
        failed["params"]["update"]["status"] = json!("failed");
        assert!(ledger.observe_native_tool(&failed, now).is_err());
        assert!(ledger.is_retired());
    }
}

#[test]
fn business_error_for_one_call_cannot_authorize_another_calls_failure() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    ready(
        &mut ledger,
        "native-first",
        "initial-first",
        "final-first",
        1,
        now,
    );
    let first = ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap();
    ledger.record_reply_written(&first, false).unwrap();
    ready(
        &mut ledger,
        "native-second",
        "initial-second",
        "final-second",
        3,
        now,
    );
    ledger.bind_sdk(&sdk(Uuid::nil(), 5, 3), now).unwrap();
    let mut failed = completed("native-second");
    failed["params"]["update"]["status"] = json!("failed");
    assert!(ledger.observe_native_tool(&failed, now).is_err());
    assert!(ledger.is_retired());
}

#[test]
fn active_turn_and_incomplete_or_unapproved_input_cannot_create_a_binding() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    assert!(ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).is_err());
    ledger
        .observe_native_tool(&initial("native-call", "initial"), now)
        .unwrap();
    assert!(
        ledger
            .record_permission(&permission("native-call", 1), true, now)
            .is_err()
    );
    assert!(ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).is_err());
    ledger
        .observe_native_tool(&final_input("native-call", "final"), now)
        .unwrap();
    assert!(ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).is_err());
    let delayed = now + Duration::from_secs(31);
    assert!(ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), delayed).is_err());
    assert!(!ledger.is_retired());
    ledger
        .record_permission(&permission("native-call", 1), true, delayed)
        .unwrap();
    assert!(ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), delayed).is_ok());
}

#[test]
fn two_native_calls_with_identical_complete_arguments_are_ambiguous() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    ready(
        &mut ledger,
        "native-first",
        "initial-first",
        "final-first",
        1,
        now,
    );
    ready(
        &mut ledger,
        "native-second",
        "initial-second",
        "final-second",
        2,
        now,
    );
    assert!(ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).is_err());
    assert!(ledger.begin_turn(Uuid::nil(), "native-turn").is_err());
}

#[test]
fn rpc_retry_keeps_original_binding_and_changed_payload_never_rebinds() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    ready(&mut ledger, "native-call", "initial", "final", 1, now);
    let proof = ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap();
    assert_eq!(
        ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap(),
        proof
    );
    assert_eq!(
        ledger.bind_sdk(&sdk(Uuid::nil(), 5, 2), now).unwrap(),
        proof
    );
    let mut conflict = sdk(Uuid::nil(), 6, 2);
    conflict["params"]["message"]["params"]["arguments"] = json!({"task_ids":["different-task"]});
    assert!(ledger.bind_sdk(&conflict, now).is_err());
    let mut outer_conflict = sdk(Uuid::nil(), 4, 3);
    outer_conflict["params"]["message"]["params"]["arguments"] = json!({});
    assert!(ledger.bind_sdk(&outer_conflict, now).is_err());
    assert_eq!(ledger.state(&proof).unwrap(), GrokToolLeaseState::Bound);
}

#[test]
fn cancelled_epoch_rejects_first_late_callback_and_reply_from_old_lease() {
    let now = Instant::now();
    let mut denied = ledger(Uuid::nil());
    denied
        .observe_native_tool(&initial("native-call", "initial"), now)
        .unwrap();
    denied
        .observe_native_tool(&final_input("native-call", "final"), now)
        .unwrap();
    denied
        .record_permission(&permission("native-call", 1), false, now)
        .unwrap();
    assert!(denied.is_retired());
    assert!(denied.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).is_err());
    let mut ledger = ledger(Uuid::nil());
    ready(&mut ledger, "native-call", "initial", "final", 1, now);
    let proof = ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap();
    ledger.retire();
    assert!(ledger.bind_sdk(&sdk(Uuid::nil(), 5, 3), now).is_err());
    assert!(ledger.record_reply_written(&proof, true).is_err());
    assert!(
        ledger
            .observe_native_tool(&completed("native-call"), now)
            .is_err()
    );
}

#[test]
fn expired_unbound_lease_retires_epoch_instead_of_using_a_later_matching_call() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    ready(&mut ledger, "native-call", "initial", "final", 1, now);
    assert!(
        ledger
            .bind_sdk(&sdk(Uuid::nil(), 4, 2), now + Duration::from_secs(31))
            .is_err()
    );
    assert!(ledger.begin_turn(Uuid::nil(), "native-turn").is_err());
}

#[test]
fn new_generation_supports_repeated_arguments_but_rejects_old_server_and_proof() {
    let now = Instant::now();
    let mut previous = ledger(Uuid::nil());
    ready(&mut previous, "native-call", "initial", "final", 1, now);
    let old = previous.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap();
    previous.retire();
    let mut current = ledger(Uuid::from_u128(1));
    ready(&mut current, "native-new-call", "initial", "final", 1, now);
    assert!(current.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).is_err());
    let new = current
        .bind_sdk(&sdk(Uuid::from_u128(1), 4, 2), now)
        .unwrap();
    assert_eq!(new.native_call_id(), "native-new-call");
    assert!(current.record_reply_written(&old, true).is_err());
    assert!(
        current
            .begin_turn(Uuid::from_u128(1), "new-turn-on-old-process")
            .is_err()
    );
}

#[test]
fn foreign_replay_and_sdk_identity_injection_cannot_supply_missing_native_origin() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    let mut foreign = initial("native-call", "foreign");
    foreign["params"]["sessionId"] = json!("foreign-session");
    assert!(ledger.observe_native_tool(&foreign, now).is_err());
    let mut replay = initial("native-call", "replay");
    replay["params"]["_meta"]["isReplay"] = json!(true);
    assert!(ledger.observe_native_tool(&replay, now).is_err());
    ready(&mut ledger, "native-call", "initial", "final", 1, now);
    let mut injected = sdk(Uuid::nil(), 4, 2);
    injected["params"]["promptId"] = json!("native-turn");
    assert!(ledger.bind_sdk(&injected, now).is_err());
    assert!(
        ledger
            .observe_native_tool(&initial("native-call", "initial"), now)
            .is_ok()
    );
    let mut changed_event = initial("native-call", "initial");
    changed_event["params"]["update"]["rawInput"]["tool_input"] = json!({"changed":true});
    assert!(ledger.observe_native_tool(&changed_event, now).is_err());
}

#[test]
fn completed_turn_reuses_process_epoch_and_nonce_without_rebinding_old_transactions() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    ready(
        &mut ledger,
        "native-old",
        "initial-old",
        "final-old",
        1,
        now,
    );
    let old = ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap();
    ledger.record_reply_written(&old, true).unwrap();
    ledger
        .observe_native_tool(&completed("native-old"), now)
        .unwrap();
    let generation = Uuid::from_u128(1);
    ledger.begin_turn(generation, "native-next-turn").unwrap();
    let mut first = initial("native-new", "initial-new");
    first["params"]["_meta"]["promptId"] = json!("native-next-turn");
    let mut final_frame = final_input("native-new", "final-new");
    final_frame["params"]["_meta"]["promptId"] = json!("native-next-turn");
    ledger.observe_native_tool(&first, now).unwrap();
    ledger.observe_native_tool(&final_frame, now).unwrap();
    ledger
        .record_permission(&permission("native-new", 2), true, now)
        .unwrap();
    assert_eq!(ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap(), old);
    assert_eq!(ledger.state(&old).unwrap(), GrokToolLeaseState::Closed);
    let next = ledger.bind_sdk(&sdk(Uuid::nil(), 8, 9), now).unwrap();
    assert_eq!(next.process_epoch(), old.process_epoch());
    assert_eq!(next.runtime_generation(), generation);
    assert_ne!(next.runtime_generation(), old.runtime_generation());
    assert_eq!(next.turn_id(), "native-next-turn");
    assert_eq!(next.native_call_id(), "native-new");
    assert_eq!(ledger.bind_sdk(&sdk(Uuid::nil(), 5, 2), now).unwrap(), old);
    assert_eq!(ledger.state(&next).unwrap(), GrokToolLeaseState::Bound);
}

#[test]
fn fixed_runtime_generation_accepts_next_confirmed_native_turn_after_all_leases_close() {
    let now = Instant::now();
    let mut ledger = ledger(Uuid::nil());
    ready(
        &mut ledger,
        "native-first",
        "initial-first",
        "final-first",
        1,
        now,
    );
    assert!(
        ledger
            .begin_turn(Uuid::nil(), "native-second-turn")
            .is_err()
    );
    let first = ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap();
    ledger.record_reply_written(&first, true).unwrap();
    ledger
        .observe_native_tool(&completed("native-first"), now)
        .unwrap();
    ledger
        .begin_turn(Uuid::nil(), "native-second-turn")
        .unwrap();
    let mut initial = initial("native-second", "initial-second");
    initial["params"]["_meta"]["promptId"] = json!("native-second-turn");
    let mut final_frame = final_input("native-second", "final-second");
    final_frame["params"]["_meta"]["promptId"] = json!("native-second-turn");
    ledger.observe_native_tool(&initial, now).unwrap();
    ledger.observe_native_tool(&final_frame, now).unwrap();
    ledger
        .record_permission(&permission("native-second", 2), true, now)
        .unwrap();
    assert_eq!(
        ledger.bind_sdk(&sdk(Uuid::nil(), 4, 2), now).unwrap(),
        first
    );
    assert_eq!(ledger.state(&first).unwrap(), GrokToolLeaseState::Closed);
    let second = ledger.bind_sdk(&sdk(Uuid::nil(), 8, 9), now).unwrap();
    assert_eq!(second.process_epoch(), first.process_epoch());
    assert_eq!(second.runtime_generation(), first.runtime_generation());
    assert_ne!(second.native_call_id(), first.native_call_id());
    assert_eq!(second.turn_id(), "native-second-turn");
    assert!(
        ledger
            .observe_native_tool(&completed("native-first"), now)
            .is_err()
    );
    assert_eq!(ledger.state(&second).unwrap(), GrokToolLeaseState::Bound);
}

#[test]
fn catalog_identity_never_reuses_retired_epoch_generation_nonce_or_session() {
    let epoch = Uuid::new_v4();
    let generation = Uuid::new_v4();
    let mut ledger = GrokToolLeaseLedger::new(
        epoch,
        generation,
        "registered-nonce".into(),
        super::super::local_tools::MCP_SERVER_NAME.into(),
        "native-session".into(),
    )
    .unwrap();
    assert!(ledger.matches_catalog_connection(
        epoch,
        generation,
        "registered-nonce",
        "native-session"
    ));
    assert!(!ledger.matches_catalog_connection(
        Uuid::new_v4(),
        generation,
        "registered-nonce",
        "native-session"
    ));
    assert!(!ledger.matches_catalog_connection(
        epoch,
        Uuid::new_v4(),
        "registered-nonce",
        "native-session"
    ));
    assert!(!ledger.matches_catalog_connection(epoch, generation, "old-nonce", "native-session"));
    assert!(!ledger.matches_catalog_connection(
        epoch,
        generation,
        "registered-nonce",
        "old-session"
    ));
    ledger.retire();
    assert!(!ledger.matches_catalog_connection(
        epoch,
        generation,
        "registered-nonce",
        "native-session"
    ));
}
