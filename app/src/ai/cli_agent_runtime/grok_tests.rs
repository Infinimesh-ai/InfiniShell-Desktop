use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use uuid::Uuid;

use super::{GrokProtocol, REQUEST_TIMEOUT, validate_options, verified_version};
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand,
    RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome,
};

fn options() -> SessionOptions {
    SessionOptions {
        executable: std::env::current_exe().unwrap(),
        cwd: std::env::temp_dir(),
        state_dir: std::env::temp_dir(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    }
}

fn authenticated_fixture() -> Vec<Value> {
    include_str!(
        "../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-authenticated-quota-error.ndjson"
    )
    .lines()
    .map(|line| serde_json::from_str::<Value>(line).unwrap())
    .filter(|record| record["direction"] == "stdout")
    .map(|record| record["message"].clone())
    .collect()
}

fn fixture_response(id: u64) -> Value {
    authenticated_fixture()
        .into_iter()
        .find(|message| message["id"] == id)
        .unwrap()
}

fn ready_protocol() -> GrokProtocol {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    for id in 1..=3 {
        protocol.receive(fixture_response(id)).unwrap();
    }
    protocol
}

#[test]
fn only_the_observed_cli_version_is_accepted() {
    assert!(verified_version("grok 1.0.30 (04b7ffed98c6)\n"));
    assert!(!verified_version("grok 1.0.29 (other)"));
    assert!(!verified_version("grok 1.0.31"));
    assert!(!verified_version("grok 1.0.30-beta"));
}

#[test]
fn real_handshake_preserves_reported_capabilities_without_opening_unverified_operations() {
    let mut protocol = GrokProtocol::new(options());
    let initialize = protocol.initialize();
    assert_eq!(initialize["jsonrpc"], "2.0");
    assert_eq!(initialize["params"]["protocolVersion"], 1);
    assert_eq!(
        initialize["params"]["clientCapabilities"]["terminal"],
        false
    );
    let authenticated = protocol.receive(fixture_response(1)).unwrap();
    assert_eq!(authenticated.writes[0]["method"], "authenticate");
    assert_eq!(
        authenticated.writes[0]["params"]["methodId"],
        "cached_token"
    );
    let created = protocol.receive(fixture_response(2)).unwrap();
    assert_eq!(created.writes[0]["method"], "session/new");
    assert_eq!(created.writes[0]["params"]["mcpServers"], json!([]));
    let ready = protocol.receive(fixture_response(3)).unwrap();
    let [
        RuntimeEventKind::SessionReady {
            effective_permissions,
        },
    ] = ready.events.as_slice()
    else {
        panic!("握手应只返回原生会话就绪");
    };
    assert_eq!(
        effective_permissions["reportedCapabilities"]["loadSession"],
        true
    );
    assert_eq!(
        effective_permissions["reportedCapabilities"]["promptCapabilities"]["image"],
        false
    );
    assert_eq!(
        effective_permissions["verifiedCapabilities"]["resume"],
        false
    );
    assert_eq!(
        effective_permissions["verifiedCapabilities"]["submit"],
        false
    );
    assert_eq!(
        effective_permissions["permissionEnforcementVerified"],
        false
    );
    assert!(effective_permissions["effectiveNativePolicy"].is_null());
    assert!(!effective_permissions.to_string().contains("email"));
    assert_eq!(
        protocol.session_id.as_deref(),
        fixture_response(3)["result"]["sessionId"].as_str()
    );
}

#[test]
fn model_dependent_image_advertisement_never_implies_verified_image_input() {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    let mut initialize = fixture_response(1);
    initialize["result"]["agentCapabilities"]["promptCapabilities"]["image"] = json!(true);
    protocol.receive(initialize).unwrap();
    assert_eq!(
        protocol.reported_capabilities["promptCapabilities"]["image"],
        true
    );
    let effects = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::LocalImage(PathBuf::from("image.png"))],
        },
    });
    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}

#[test]
fn missing_headless_credentials_does_not_start_interactive_auth_or_a_session() {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    let mut initialize = fixture_response(1);
    initialize["result"]["authMethods"] = json!([{"id": "grok.com", "name": "Grok"}]);
    assert!(protocol.receive(initialize).is_err());
    assert_eq!(protocol.next_id, 1);
    assert!(protocol.session_id.is_none());
}

#[test]
fn native_byok_authentication_uses_only_advertised_method_without_credentials() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-byok-authentication.json"
    ))
    .unwrap();
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    let effects = protocol.receive(fixture["initialize"].clone()).unwrap();
    assert_eq!(effects.writes.len(), 1);
    assert_eq!(effects.writes[0]["method"], "authenticate");
    assert_eq!(
        effects.writes[0]["params"],
        json!({"methodId":"xai.api_key", "_meta":{"headless":true}})
    );
    assert!(effects.events.is_empty());
    assert!(protocol.session_id.is_none());
    let opened = protocol.receive(fixture["authenticate"].clone()).unwrap();
    assert_eq!(opened.writes[0]["method"], "session/new");
    assert!(opened.events.is_empty());
    // 认证可用不提升未经过模型验收的执行或权限能力。
    assert!(protocol.session_id.is_none());
}

#[test]
fn cached_login_precedence_is_stable_and_api_authentication_failure_does_not_fallback() {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    let mut initialize = fixture_response(1);
    initialize["result"]["authMethods"] = json!([
        {"id":"xai.api_key"}, {"id":"cached_token"}, {"id":"grok.com"}
    ]);
    let effects = protocol.receive(initialize).unwrap();
    assert_eq!(effects.writes[0]["params"]["methodId"], "cached_token");

    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    let mut initialize = fixture_response(1);
    initialize["result"]["authMethods"] = json!([{"id":"xai.api_key"}, {"id":"grok.com"}]);
    protocol.receive(initialize).unwrap();
    assert!(
        protocol
            .receive(json!({"jsonrpc":"2.0", "id":2,
        "error":{"code":-32000, "message":"Authentication required"}}))
            .is_err()
    );
    assert_eq!(protocol.next_id, 2);
    assert!(protocol.session_id.is_none());
}

#[test]
fn mismatched_agent_and_protocol_versions_do_not_authenticate() {
    for (key, value) in [
        ("protocolVersion", json!(2)),
        ("agentVersion", json!("1.0.31")),
    ] {
        let mut protocol = GrokProtocol::new(options());
        protocol.initialize();
        let mut initialize = fixture_response(1);
        if key == "protocolVersion" {
            initialize["result"][key] = value;
        } else {
            initialize["result"]["_meta"][key] = value;
        }
        assert!(protocol.receive(initialize).is_err());
        assert_eq!(protocol.next_id, 1);
    }
}

#[test]
fn duplicate_responses_do_not_create_another_session_and_conflicts_fail_closed() {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    let initialize = fixture_response(1);
    protocol.receive(initialize.clone()).unwrap();
    assert!(
        protocol
            .receive(initialize.clone())
            .unwrap()
            .writes
            .is_empty()
    );
    protocol.receive(fixture_response(2)).unwrap();
    assert!(
        protocol
            .receive(initialize.clone())
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(protocol.next_id, 3);
    let mut conflict = initialize;
    conflict["result"]["protocolVersion"] = json!(9);
    assert!(protocol.receive(conflict).is_err());
}

#[test]
fn out_of_order_response_cannot_replace_pending_authentication() {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    assert!(protocol.receive(fixture_response(3)).is_err());
    assert_eq!(protocol.pending.as_ref().unwrap().id, 1);
    assert!(protocol.session_id.is_none());
}

#[test]
fn native_quota_error_and_session_activity_never_become_completion() {
    let mut protocol = ready_protocol();
    for message in authenticated_fixture()
        .into_iter()
        .filter(|message| message.get("method").is_some())
    {
        let effects = protocol.receive(message).unwrap();
        assert!(effects.events.is_empty());
    }
    // 真实 402 是未发送 prompt 的旧回调；无论其错误字段如何，都不能创造成功回合。
    let quota_error = fixture_response(4);
    assert_eq!(quota_error["error"]["data"]["http_status"], 402);
    assert!(protocol.receive(quota_error).is_err());
}

#[test]
fn all_unverified_lifecycle_commands_are_rejected_without_delivery_or_acknowledgement() {
    let mut protocol = ready_protocol();
    let actions = [
        RuntimeAction::Submit {
            input: vec![InputContent::Text("第一行\nsecond line".into())],
        },
        RuntimeAction::Steer {
            expected_turn_id: "turn".into(),
            input: vec![InputContent::Text("追加".into())],
        },
        RuntimeAction::Interrupt {
            turn_id: "turn".into(),
        },
        RuntimeAction::RespondApproval {
            approval_id: "approval".into(),
            decision: ApprovalDecision::AllowOnce,
        },
        RuntimeAction::RespondApproval {
            approval_id: "approval".into(),
            decision: ApprovalDecision::DenyOnce,
        },
    ];
    for action in actions {
        let command = RuntimeCommand {
            generation: protocol.options.generation,
            message_id: Uuid::new_v4(),
            action,
        };
        let first = protocol.command(command.clone());
        let replay = protocol.command(command);
        assert!(first.writes.is_empty());
        assert!(matches!(
            first.events.as_slice(),
            [RuntimeEventKind::RequestFailed { .. }]
        ));
        assert_eq!(first.events, replay.events);
    }
}

#[test]
fn unadvertised_client_tools_and_permissions_never_execute_or_gain_approval() {
    let mut protocol = ready_protocol();
    for method in [
        "terminal/create",
        "fs/write_text_file",
        "session/request_permission",
    ] {
        let effects = protocol
            .receive(json!({
                "jsonrpc": "2.0", "id": "server-id", "method": method,
                "params": {"sessionId": protocol.session_id}
            }))
            .unwrap();
        assert!(effects.events.is_empty());
        assert_eq!(effects.writes[0]["id"], "server-id");
        assert_eq!(effects.writes[0]["error"]["code"], -32601);
        assert!(effects.writes[0].get("result").is_none());
    }
}

#[test]
fn stale_generation_and_incomplete_native_identity_do_not_open_a_task() {
    let mut protocol = ready_protocol();
    let effects = protocol.command(RuntimeCommand {
        generation: Uuid::new_v4(),
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Shutdown,
    });
    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    protocol = GrokProtocol::new(options());
    protocol.initialize();
    protocol.receive(fixture_response(1)).unwrap();
    protocol.receive(fixture_response(2)).unwrap();
    let mut response = fixture_response(3);
    response["result"]["sessionId"] = json!("");
    assert!(protocol.receive(response).is_err());
    assert!(protocol.session_id.is_none());
}

#[test]
fn resume_and_sandbox_claims_fail_before_spawning_a_new_process() {
    let mut options = options();
    options.target = SessionTarget::Resume {
        native_session_id: "existing-session".into(),
    };
    assert!(validate_options(&options).is_err());
    options.target = SessionTarget::New;
    for policy in [PermissionPolicy::ReadOnly, PermissionPolicy::WorkspaceWrite] {
        options.permission_policy = policy;
        assert!(validate_options(&options).is_err());
    }
    options.permission_policy = PermissionPolicy::Inherit;
    options.model = Some("grok-4.5".into());
    assert!(validate_options(&options).is_err());
}

#[test]
fn handshake_timeout_remains_uncertain_and_does_not_retry() {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    protocol.pending.as_mut().unwrap().sent_at =
        Instant::now() - REQUEST_TIMEOUT - Duration::from_secs(1);
    assert!(protocol.request_timed_out());
    assert_eq!(protocol.next_id, 1);
    assert!(protocol.session_id.is_none());
}

fn quota_notifications() -> Vec<Value> {
    authenticated_fixture()
        .into_iter()
        .filter(|message| message["method"] == "_x.ai/queue/changed")
        .collect()
}

fn prompt_complete_notifications() -> Vec<Value> {
    authenticated_fixture()
        .into_iter()
        .filter(|message| message["method"] == "_x.ai/session/prompt_complete")
        .collect()
}

fn internal_submit(protocol: &mut GrokProtocol, message_id: Uuid, text: &str) -> super::Effects {
    protocol.acp_command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id,
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text(text.to_owned())],
        },
    })
}

#[test]
fn raw_quota_fixture_emits_one_failure_per_prompt_despite_multiple_terminal_reports() {
    let mut protocol = ready_protocol();
    let records = include_str!(
        "../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-authenticated-quota-error.ndjson"
    )
    .lines()
    .map(|line| serde_json::from_str::<Value>(line).unwrap());
    let mut outcomes = Vec::new();
    let mut accepted_ids = Vec::new();
    for record in records {
        let message = &record["message"];
        if record["direction"] == "stdin" && message["method"] == "session/prompt" {
            let effects = internal_submit(
                &mut protocol,
                Uuid::new_v4(),
                message["params"]["prompt"][0]["text"].as_str().unwrap(),
            );
            assert_eq!(effects.writes.as_slice(), &[message.clone()]);
            assert!(effects.events.is_empty());
        } else if record["direction"] == "stdout" && protocol.prompt.is_some() {
            let effects = protocol.receive(message.clone()).unwrap();
            for event in effects.events {
                match event {
                    RuntimeEventKind::TurnFinished {
                        turn_id, outcome, ..
                    } => {
                        outcomes.push((turn_id, outcome));
                    }
                    RuntimeEventKind::MessageAccepted { turn_id, .. } => {
                        accepted_ids.push(turn_id.unwrap());
                    }
                    RuntimeEventKind::TurnStarted { .. } | RuntimeEventKind::Progress { .. } => {}
                    unexpected => panic!("夹具不应产生额外事件：{unexpected:?}"),
                }
            }
        }
    }
    assert_eq!(
        accepted_ids,
        [
            "f271bc00-cc67-4e17-bd57-7d6a8ace67f4",
            "6fa00db2-ad9e-428f-9089-67d8cd525e13",
        ]
    );
    assert_eq!(outcomes, vec![
        ("f271bc00-cc67-4e17-bd57-7d6a8ace67f4".into(), TurnOutcome::Failed {
            message: "API error (status 402 Payment Required): Grok Build usage balance exhausted".into(),
        }),
        ("6fa00db2-ad9e-428f-9089-67d8cd525e13".into(), TurnOutcome::Failed {
            message: "API error (status 402 Payment Required): Grok Build usage balance exhausted".into(),
        }),
    ]);
    assert!(protocol.pending.is_none());
    assert!(protocol.prompt.is_none());
}

#[test]
fn queue_versions_are_not_sequences_and_equal_text_turns_keep_distinct_native_ids() {
    let mut protocol = ready_protocol();
    let queue = quota_notifications();
    let completed = prompt_complete_notifications();
    let first_message = Uuid::new_v4();
    let second_message = Uuid::new_v4();
    internal_submit(&mut protocol, first_message, "完全相同的内容\nSame input");
    assert_eq!(queue[0]["params"]["entries"][0]["version"], 0);
    let queued = protocol.receive(queue[0].clone()).unwrap();
    assert!(
        matches!(&queued.events[0], RuntimeEventKind::MessageAccepted { message_id, turn_id }
        if *message_id == first_message && turn_id.as_deref() == Some("f271bc00-cc67-4e17-bd57-7d6a8ace67f4"))
    );
    assert!(
        protocol
            .receive(queue[0].clone())
            .unwrap()
            .events
            .is_empty()
    );
    let running = protocol.receive(queue[1].clone()).unwrap();
    assert!(
        matches!(&running.events[0], RuntimeEventKind::TurnStarted { turn_id }
        if turn_id == "f271bc00-cc67-4e17-bd57-7d6a8ace67f4")
    );
    assert!(
        protocol
            .receive(queue[1].clone())
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        protocol
            .receive(queue[2].clone())
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(
        protocol.receive(completed[0].clone()).unwrap().events.len(),
        1
    );
    // 通知终态出现后仍要排空原 RPC，不能把晚到响应算到第二轮。
    assert!(
        internal_submit(&mut protocol, second_message, "完全相同的内容\nSame input")
            .writes
            .is_empty()
    );
    assert!(
        protocol
            .receive(fixture_response(4))
            .unwrap()
            .events
            .is_empty()
    );
    let submitted = internal_submit(&mut protocol, second_message, "完全相同的内容\nSame input");
    assert_eq!(submitted.writes[0]["id"], 5);
    assert!(
        protocol
            .receive(queue[0].clone())
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        protocol
            .receive(completed[0].clone())
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(queue[3]["params"]["entries"][0]["version"], 0);
    let next = protocol.receive(queue[3].clone()).unwrap();
    assert!(
        matches!(&next.events[0], RuntimeEventKind::MessageAccepted { message_id, turn_id }
        if *message_id == second_message && turn_id.as_deref() == Some("6fa00db2-ad9e-428f-9089-67d8cd525e13"))
    );
    protocol.receive(queue[4].clone()).unwrap();
    assert_eq!(
        protocol.receive(fixture_response(5)).unwrap().events.len(),
        1
    );
    assert!(
        protocol
            .receive(completed[1].clone())
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        protocol
            .receive(fixture_response(5))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn ambiguous_queue_and_running_without_a_correlated_entry_never_acknowledge_input() {
    let mut protocol = ready_protocol();
    let message_id = Uuid::new_v4();
    internal_submit(&mut protocol, message_id, "一条应用输入");
    let queue = quota_notifications();
    let mut ambiguous = queue[0].clone();
    ambiguous["params"]["entries"]
        .as_array_mut()
        .unwrap()
        .push(queue[3]["params"]["entries"][0].clone());
    assert!(protocol.receive(ambiguous).unwrap().events.is_empty());
    assert!(
        protocol
            .receive(queue[0].clone())
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        protocol
            .receive(queue[1].clone())
            .unwrap()
            .events
            .is_empty()
    );
    let failure = protocol.receive(fixture_response(4)).unwrap();
    assert!(
        matches!(failure.events.as_slice(), [RuntimeEventKind::RequestFailed { message_id: failed_id, .. }] if *failed_id == message_id)
    );
    assert!(
        protocol
            .receive(prompt_complete_notifications()[0].clone())
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn unknown_stop_reason_and_empty_queue_never_become_success_or_cancellation() {
    let mut protocol = ready_protocol();
    internal_submit(
        &mut protocol,
        Uuid::new_v4(),
        "不能从 complete 名称推断成功",
    );
    let queue = quota_notifications();
    protocol.receive(queue[0].clone()).unwrap();
    protocol.receive(queue[1].clone()).unwrap();
    assert!(
        protocol
            .receive(queue[2].clone())
            .unwrap()
            .events
            .is_empty()
    );
    let mut complete = prompt_complete_notifications()[0].clone();
    complete["params"]["stopReason"] = json!("end_turn");
    complete["params"]["agentResult"] = json!("看似成功的内容");
    assert!(protocol.receive(complete).unwrap().events.is_empty());
    assert!(!protocol.prompt.as_ref().unwrap().finished);
    assert!(
        protocol
            .receive(json!({"jsonrpc": "2.0", "id": 4, "result": {"stopReason": "end_turn"}}))
            .is_err()
    );
}

#[test]
fn real_notification_ids_deduplicate_without_treating_timestamps_as_sequence_numbers() {
    let mut protocol = ready_protocol();
    internal_submit(&mut protocol, Uuid::new_v4(), "乱序扩展通知");
    protocol.receive(quota_notifications()[0].clone()).unwrap();
    let mut completed = authenticated_fixture()
        .into_iter()
        .find(|message| {
            message["method"] == "_x.ai/session_notification"
                && message["params"]["update"]["sessionUpdate"] == "turn_completed"
        })
        .unwrap();
    completed["params"]["_meta"]["agentTimestampMs"] = json!(1);
    let retry = authenticated_fixture()
        .into_iter()
        .find(|message| {
            message["method"] == "_x.ai/session_notification"
                && message["params"]["update"]["sessionUpdate"] == "retry_state"
        })
        .unwrap();
    protocol.receive(retry).unwrap();
    assert_eq!(protocol.receive(completed.clone()).unwrap().events.len(), 1);
    assert!(
        protocol
            .receive(completed.clone())
            .unwrap()
            .events
            .is_empty()
    );
    completed["params"]["update"]["agent_result"] = json!("同 eventId 的冲突载荷");
    assert!(protocol.receive(completed).is_err());
    assert!(
        protocol
            .receive(fixture_response(4))
            .unwrap()
            .events
            .is_empty()
    );
}

fn empty_recovery_responses(phase: usize) -> Vec<Value> {
    include_str!(
        "../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-empty-session-recovery.ndjson"
    )
    .lines()
    .map(|line| serde_json::from_str::<Value>(line).unwrap())
    .filter(|record| record["direction"] == "stdout" && record["message"].get("id").is_some())
    .skip(phase * 4)
    .take(4)
    .map(|record| record["message"].clone())
    .collect()
}

#[test]
fn real_empty_load_and_resume_retain_the_requested_identity_and_close_without_a_turn() {
    for (phase, method) in [(1, "session/load"), (2, "session/resume")] {
        let records = empty_recovery_responses(phase);
        let mut options = options();
        options.target = SessionTarget::Resume {
            native_session_id: "01a0a8ef-22d4-70f2-91d3-0cd052d7e80f".into(),
        };
        // 内部协议验证不能解除 public connect 的真实历史恢复门禁。
        assert!(validate_options(&options).is_err());
        let mut protocol = GrokProtocol::new(options);
        protocol.initialize();
        protocol.receive(records[0].clone()).unwrap();
        if method == "session/load" {
            // 夹具同时验证了两个合法方法；没有 resume 声明时才选择 load。
            protocol.reported_capabilities["sessionCapabilities"]["resume"] = Value::Null;
        }
        let opened = protocol.receive(records[1].clone()).unwrap();
        assert_eq!(opened.writes[0]["method"], method);
        assert_eq!(
            opened.writes[0]["params"]["sessionId"],
            "01a0a8ef-22d4-70f2-91d3-0cd052d7e80f"
        );
        assert!(records[2]["result"].get("sessionId").is_none());
        let ready = protocol.receive(records[2].clone()).unwrap();
        assert_eq!(
            protocol.session_id.as_deref(),
            Some("01a0a8ef-22d4-70f2-91d3-0cd052d7e80f")
        );
        assert!(matches!(
            ready.events.as_slice(),
            [RuntimeEventKind::SessionReady { .. }]
        ));
        let message_id = Uuid::new_v4();
        let shutdown = RuntimeCommand {
            generation: protocol.options.generation,
            message_id,
            action: RuntimeAction::Shutdown,
        };
        let close = protocol.command(shutdown.clone());
        assert_eq!(close.writes[0]["method"], "session/close");
        assert_eq!(
            close.writes[0]["params"]["sessionId"],
            "01a0a8ef-22d4-70f2-91d3-0cd052d7e80f"
        );
        assert_eq!(
            close.events,
            vec![RuntimeEventKind::CommandDispatched {
                message_id,
                turn_id: None
            }]
        );
        assert!(protocol.command(shutdown).writes.is_empty());
        assert!(!protocol.closed);
        assert!(
            protocol
                .receive(records[3].clone())
                .unwrap()
                .events
                .is_empty()
        );
        assert!(protocol.closed);
        assert!(
            protocol
                .receive(records[3].clone())
                .unwrap()
                .events
                .is_empty()
        );
    }
}

#[test]
fn recovery_error_or_changed_identity_cannot_fall_back_to_a_new_session() {
    for response in [
        json!({"jsonrpc": "2.0", "id": 3, "error": {"code": -32603, "message": "missing history"}}),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"sessionId": "another-session"}}),
    ] {
        let mut options = options();
        options.target = SessionTarget::Resume {
            native_session_id: "original-session".into(),
        };
        let mut protocol = GrokProtocol::new(options);
        protocol.initialize();
        protocol.receive(fixture_response(1)).unwrap();
        assert_eq!(
            protocol.receive(fixture_response(2)).unwrap().writes[0]["method"],
            "session/resume"
        );
        assert!(protocol.receive(response).is_err());
        assert_eq!(protocol.next_id, 3);
        assert!(protocol.session_id.is_none());
    }
}

#[test]
fn native_display_metadata_is_retained_without_copying_account_or_extension_fields() {
    let mut protocol = ready_protocol();
    assert_eq!(
        protocol
            .reported_metadata
            .models
            .as_ref()
            .unwrap()
            .current_model_id,
        "grok-4.6"
    );
    assert_eq!(
        protocol.reported_metadata.config_options[1].id,
        "reasoning_effort"
    );
    let models = authenticated_fixture()
        .into_iter()
        .find(|message| message["method"] == "_x.ai/models/update")
        .unwrap();
    assert!(protocol.receive(models).unwrap().events.is_empty());
    let mut commands = authenticated_fixture()
        .into_iter()
        .find(|message| message["params"]["update"]["sessionUpdate"] == "available_commands_update")
        .unwrap();
    commands["params"]["update"]["availableCommands"][0]["_meta"] =
        json!({"credential": "must-not-copy"});
    assert!(
        protocol
            .receive(commands.clone())
            .unwrap()
            .events
            .is_empty()
    );
    assert!(protocol.receive(commands).unwrap().events.is_empty());
    let metadata = serde_json::to_value(&protocol.reported_metadata).unwrap();
    assert_eq!(
        metadata["models"]["availableModels"][1]["modelId"],
        "grok-4.5"
    );
    assert!(
        metadata["availableCommands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["name"] == "always-approve")
    );
    let serialized = metadata.to_string();
    assert!(!serialized.contains("_meta"));
    assert!(!serialized.contains("credential"));
    assert!(!serialized.contains("currentWorkingDirectory"));
    assert!(!serialized.contains("email"));
    // 原生命令名称不能自动激活对应的高风险能力。
    let denied = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::RespondApproval {
            approval_id: "native-request".into(),
            decision: ApprovalDecision::AllowOnce,
        },
    });
    assert!(denied.writes.is_empty());
}

#[test]
fn queue_and_failure_for_other_sessions_cannot_mutate_the_current_prompt() {
    let mut protocol = ready_protocol();
    internal_submit(&mut protocol, Uuid::new_v4(), "当前会话");
    let mut queued = quota_notifications()[0].clone();
    queued["params"]["sessionId"] = json!("another-session");
    assert!(protocol.receive(queued).unwrap().events.is_empty());
    let mut completed = prompt_complete_notifications()[0].clone();
    completed["params"]["sessionId"] = json!("another-session");
    assert!(protocol.receive(completed).unwrap().events.is_empty());
    assert!(protocol.prompt.as_ref().unwrap().native_id.is_none());
    assert!(!protocol.prompt.as_ref().unwrap().finished);
}

#[test]
fn message_redelivery_never_sends_a_second_prompt_after_the_original_failed() {
    let mut protocol = ready_protocol();
    let message_id = Uuid::new_v4();
    let first = internal_submit(&mut protocol, message_id, "同一条消息");
    assert_eq!(first.writes.len(), 1);
    assert!(
        internal_submit(&mut protocol, message_id, "同一条消息")
            .writes
            .is_empty()
    );
    protocol.receive(quota_notifications()[0].clone()).unwrap();
    protocol.receive(fixture_response(4)).unwrap();
    assert!(
        internal_submit(&mut protocol, message_id, "同一条消息")
            .writes
            .is_empty()
    );
    assert!(
        internal_submit(&mut protocol, message_id, "修改过的消息")
            .writes
            .is_empty()
    );
    assert_eq!(protocol.next_id, 4);
}

#[test]
fn an_unconfirmed_close_response_is_not_treated_as_closed_or_cancelled() {
    let mut protocol = ready_protocol();
    let effects = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Shutdown,
    });
    assert_eq!(effects.writes[0]["method"], "session/close");
    assert!(
        protocol
            .receive(json!({"jsonrpc": "2.0", "id": 4, "result": {}}))
            .is_err()
    );
    assert!(!protocol.closed);
}
