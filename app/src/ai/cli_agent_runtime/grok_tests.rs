use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures::io::Cursor;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{GrokProtocol, REQUEST_TIMEOUT, flush_effects, validate_options, verified_version};
use crate::ai::cli_agent_runtime::local_tools::LocalToolPermissions;
use crate::ai::cli_agent_runtime::{
    ApprovalDecision, InputContent, PermissionPolicy, RuntimeAction, RuntimeCommand, RuntimeError,
    RuntimeEventKind, SessionOptions, SessionTarget, TurnOutcome,
};

pub(super) fn options() -> SessionOptions {
    SessionOptions {
        executable: std::env::current_exe().unwrap(),
        cwd: std::env::temp_dir(),
        state_dir: std::env::temp_dir(),
        target: SessionTarget::New,
        generation: Uuid::new_v4(),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
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
fn real_handshake_reports_verified_text_lifecycle_and_preserves_permission_limits() {
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
        true
    );
    assert_eq!(
        effective_permissions["verifiedCapabilities"]["submit"],
        true
    );
    for key in ["steer", "localTools", "childTasks"] {
        assert_eq!(effective_permissions["verifiedCapabilities"][key], false);
    }
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
fn unknown_string_response_cannot_replace_a_numeric_pending_request() {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    let response = json!({"jsonrpc":"2.0","id":"OFFLINE_UNKNOWN_RESPONSE_ID",
        "error":{"code":-32603,"message":"OFFLINE_PRIVATE_ERROR_BODY"}});

    let error = protocol.receive(response.clone()).err().unwrap();
    let diagnostic = super::runtime_error_diagnostic(&error);
    assert_eq!(diagnostic["runtime_error_kind"], "protocol");
    assert_eq!(diagnostic["protocol_failure_kind"], "invalid_response_id");
    assert!(
        !diagnostic
            .to_string()
            .contains("OFFLINE_PRIVATE_ERROR_BODY")
    );
    // 重投未知响应仍失败，不能建立身份或消耗原 pending 请求。
    assert!(protocol.receive(response).is_err());
    assert_eq!(protocol.pending.as_ref().unwrap().id, 1);
    assert!(protocol.responses.is_empty());
    assert!(protocol.session_id.is_none());
    let context = protocol.transaction_context();
    assert_eq!(context["pending_id"], 1);
    assert_eq!(context["pending_kind"], "initialize");
    assert_eq!(context["generation"], json!(protocol.options.generation));
}

#[tokio::test]
async fn outbound_diagnostic_records_written_identity_without_parameters() {
    let mut protocol = GrokProtocol::new(options());
    let probe = super::sdk_origin_live_tests::SdkOriginProbe::new(protocol.options.generation);
    protocol.sdk_origin_probe = Some(probe.clone());
    let request = protocol.request(
        super::PendingKind::Prompt,
        "session/prompt",
        json!({"prompt":"OFFLINE_PRIVATE_PROMPT","sessionId":"OFFLINE_PRIVATE_SESSION"}),
    );
    let (sender, _) = mpsc::channel(4);
    let mut stdin = Cursor::new(Vec::new());
    flush_effects(
        &protocol,
        &mut stdin,
        &sender,
        super::Effects {
            writes: vec![request.clone()],
            events: Vec::new(),
        },
    )
    .await
    .unwrap();

    let report = probe.report(None, None);
    let diagnostic = &report["frames"][0];
    assert_eq!(diagnostic["event"], "outbound_transaction_observed");
    assert_eq!(diagnostic["method"], "session/prompt");
    assert_eq!(diagnostic["id"]["type"], "number");
    assert_eq!(diagnostic["id_number"], request["id"]);
    assert_eq!(diagnostic["transaction_context"]["pending_kind"], "prompt");
    assert!(!report.to_string().contains("OFFLINE_PRIVATE_PROMPT"));
    assert!(!report.to_string().contains("OFFLINE_PRIVATE_SESSION"));
    let actual: Value = serde_json::from_slice(stdin.get_ref()).unwrap();
    assert_eq!(actual, request);
}

#[tokio::test]
async fn failed_write_does_not_claim_an_outbound_transaction() {
    let mut protocol = GrokProtocol::new(options());
    let probe = super::sdk_origin_live_tests::SdkOriginProbe::new(protocol.options.generation);
    protocol.sdk_origin_probe = Some(probe.clone());
    let request = protocol.initialize();
    let (sender, _) = mpsc::channel(4);
    let mut storage = [];
    let mut stdin = Cursor::new(&mut storage[..]);
    assert!(
        flush_effects(
            &protocol,
            &mut stdin,
            &sender,
            super::Effects {
                writes: vec![request],
                events: Vec::new(),
            }
        )
        .await
        .is_err()
    );
    assert_eq!(probe.report(None, None)["frames"], json!([]));
    assert_eq!(protocol.pending.as_ref().unwrap().id, 1);
}

#[test]
fn transaction_context_does_not_expose_history_session_or_recovery_arguments() {
    let mut protocol = GrokProtocol::new(options());
    protocol.session_id = Some("OFFLINE_PRIVATE_NATIVE_SESSION".into());
    protocol.request(
        super::PendingKind::OpenSession {
            requested_id: Some("OFFLINE_PRIVATE_RECOVERY_ID".into()),
        },
        "session/load",
        json!({}),
    );
    let context = protocol.transaction_context();
    assert_eq!(context["pending_kind"], "load_session");
    assert_eq!(context["pending_id"], 1);
    assert!(
        !context
            .to_string()
            .contains("OFFLINE_PRIVATE_NATIVE_SESSION")
    );
    assert!(!context.to_string().contains("OFFLINE_PRIVATE_RECOVERY_ID"));
    protocol.pending = None;
    assert_eq!(protocol.transaction_context()["pending_kind"], Value::Null);
}

#[test]
fn an_old_error_response_cannot_replace_current_authentication() {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    protocol.receive(fixture_response(1)).unwrap();
    let error = protocol
        .receive(json!({"jsonrpc":"2.0","id":1,
        "error":{"code":-32603,"message":"OFFLINE_OLD_ERROR_BODY"}}))
        .err()
        .unwrap();

    assert_eq!(
        super::runtime_error_diagnostic(&error)["protocol_failure_kind"],
        "conflicting_response"
    );
    assert_eq!(protocol.pending.as_ref().unwrap().id, 2);
    assert!(protocol.session_id.is_none());
}

#[test]
fn response_error_diagnostics_omit_arbitrary_bodies_and_malformed_codes() {
    let message = json!({"jsonrpc":"2.0","id":"OFFLINE_PRIVATE_ID",
        "error":{"code":true,"message":"OFFLINE_ERROR_BODY",
            "data":{"message":{"private":"OFFLINE_NESTED_BODY"},"http_status":true}},
        "result":{"stopReason":["OFFLINE_STOP_BODY"]}});
    let diagnostic = super::native_response_diagnostic(&message);

    assert_eq!(diagnostic["jsonrpc_is_2_0"], true);
    assert_eq!(diagnostic["response_id"]["type"], "string");
    assert_eq!(diagnostic["error_code"], Value::Null);
    assert_eq!(diagnostic["error_code_type"], "boolean");
    assert_eq!(diagnostic["native_error_http_status"], Value::Null);
    assert_eq!(diagnostic["native_error_category"], "unknown");
    assert_eq!(diagnostic["error_message"]["type"], "string");
    assert_eq!(diagnostic["error_message"]["bytes"], 18);
    assert_eq!(diagnostic["error_data_message"]["type"], "object");
    assert_eq!(diagnostic["result_stop_reason"]["type"], "array");
    assert_eq!(diagnostic["result_error_conflict"], true);
    assert!(!diagnostic.to_string().contains("OFFLINE_"));
}

#[test]
fn native_http_failure_category_only_uses_a_typed_status_field() {
    let diagnostic = super::native_response_diagnostic(&json!({"jsonrpc":"2.0","id":4,
        "error":{"code":-32603,"message":"OFFLINE_HTTP_ERROR_BODY","data":{"http_status":429}}}));

    assert_eq!(diagnostic["native_error_http_status"], 429);
    assert_eq!(diagnostic["native_error_category"], "http_429");
    assert!(!diagnostic.to_string().contains("OFFLINE_HTTP_ERROR_BODY"));
}

#[test]
fn runtime_error_diagnostics_do_not_expose_unrecognized_protocol_details() {
    let diagnostic = super::runtime_error_diagnostic(&RuntimeError::Protocol(
        "OFFLINE_PROTOCOL_PRIVATE_BODY".into(),
    ));

    assert_eq!(diagnostic["runtime_error_kind"], "protocol");
    assert_eq!(diagnostic["protocol_failure_kind"], "unknown");
    assert_eq!(diagnostic["runtime_error_message"]["type"], "string");
    assert!(
        !diagnostic
            .to_string()
            .contains("OFFLINE_PROTOCOL_PRIVATE_BODY")
    );
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
fn unsupported_or_unrelated_controls_are_rejected_without_delivery_or_acknowledgement() {
    let mut protocol = ready_protocol();
    let actions = [
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
        assert!(replay.writes.is_empty());
        assert!(matches!(
            first.events.as_slice(),
            [RuntimeEventKind::RequestFailed { .. }]
        ));
        // 无效控制重投可以被去重，但不能出现派发或原生确认。
        assert!(
            replay
                .events
                .iter()
                .all(|event| matches!(event, RuntimeEventKind::RequestFailed { .. }))
        );
    }
}

#[test]
fn unadvertised_client_tools_and_permissions_never_execute_or_gain_approval() {
    let mut protocol = ready_protocol();
    for method in ["terminal/create", "fs/write_text_file"] {
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
fn root_history_resume_is_valid_but_unverified_policies_fail_before_spawning() {
    let mut options = options();
    options.target = SessionTarget::Resume {
        native_session_id: "existing-session".into(),
    };
    // ACP 会话身份是有界 opaque 字符串，不能擅自限定为 UUID。
    assert!(validate_options(&options).is_ok());
    for invalid in ["".to_owned(), "session\nother".to_owned(), "x".repeat(4097)] {
        options.target = SessionTarget::Resume {
            native_session_id: invalid,
        };
        assert!(validate_options(&options).is_err());
    }
    options.target = SessionTarget::Resume {
        native_session_id: Uuid::new_v4().to_string(),
    };
    assert!(validate_options(&options).is_ok());
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
    protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id,
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text(text.to_owned())],
        },
    })
}

// 旧生命周期实录没有查询历史；这些合成响应仅用于原有状态测试，不计作原生查询证据。
fn unit_history(protocol: &GrokProtocol, output: &str) -> Value {
    let prompt = protocol.prompt.as_ref().unwrap();
    let session = protocol.session_id.as_ref().unwrap();
    let turn = prompt.native_id.as_ref().unwrap();
    let reason = match prompt.completion.as_ref().unwrap().outcome {
        TurnOutcome::Completed => "end_turn",
        TurnOutcome::Cancelled => "cancelled",
        TurnOutcome::Failed { .. } => panic!("失败回合不查询最终文本"),
    };
    json!({"updates": [
        {"method":"session/update", "params":{"sessionId":session,
            "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":output}},
            "_meta":{"eventId":format!("{session}-900000"),"promptId":turn}}},
        {"method":"_x.ai/session/update", "params":{"sessionId":session,
            "update":{"sessionUpdate":"turn_completed","prompt_id":turn,"stop_reason":reason},
            "_meta":{"eventId":format!("{session}-900001")}}}
    ],"totalCount":2,"hasMore":false,"lastEventId":format!("{session}-900001")})
}

fn finish_with_unit_history(protocol: &mut GrokProtocol, response: Value) -> super::Effects {
    let mut effects = protocol.receive(response).unwrap();
    if effects
        .writes
        .first()
        .is_some_and(|request| request["method"] == "_x.ai/session/updates")
    {
        let output = protocol.prompt.as_ref().unwrap().output.clone();
        let snapshot = unit_history(protocol, &output);
        let request = effects.writes.remove(0);
        let recovered = protocol
            .receive(json!({"jsonrpc":"2.0","id":request["id"],"result":snapshot}))
            .unwrap();
        effects.events.extend(recovered.events);
        effects.writes.extend(recovered.writes);
    }
    effects
}

fn receive_recorded_with_unit_history(
    protocol: &mut GrokProtocol,
    mut message: Value,
) -> super::Effects {
    if message.get("method").is_none() && message.get("id").is_some() {
        // 新增只读查询会改变客户端 RPC 序号，原生 prompt/event/tool 身份保持原样。
        message["id"] = json!(protocol.pending.as_ref().unwrap().id);
    }
    finish_with_unit_history(protocol, message)
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
    let drained = protocol.receive(fixture_response(4)).unwrap();
    assert!(drained.events.is_empty());
    assert_eq!(drained.writes[0]["id"], 5);
    let submitted = internal_submit(&mut protocol, second_message, "完全相同的内容\nSame input");
    assert!(submitted.writes.is_empty());
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
fn real_empty_load_retains_the_requested_identity_and_closes_without_a_turn() {
    let records = empty_recovery_responses(1);
    let mut options = options();
    options.target = SessionTarget::Resume {
        native_session_id: "01a0a8ef-22d4-70f2-91d3-0cd052d7e80f".into(),
    };
    assert!(validate_options(&options).is_ok());
    let mut protocol = GrokProtocol::new(options);
    protocol.initialize();
    protocol.receive(records[0].clone()).unwrap();
    let opened = protocol.receive(records[1].clone()).unwrap();
    assert_eq!(opened.writes[0]["method"], "session/load");
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
            "session/load"
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

fn approval_lifecycle_fixture() -> Vec<Value> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-byok-approval-lifecycle.json"
    ))
    .unwrap();
    fixture["records"].as_array().unwrap().clone()
}

fn byok_response(id: u64) -> Value {
    approval_lifecycle_fixture()
        .into_iter()
        .find(|record| {
            record["direction"] == "stdout"
                && record["message"]["id"] == id
                && record["message"].get("method").is_none()
        })
        .unwrap()["message"]
        .clone()
}

fn active_byok_prompt() -> GrokProtocol {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    for id in 1..=3 {
        protocol.receive(byok_response(id)).unwrap();
    }
    internal_submit(&mut protocol, Uuid::new_v4(), "固定回执");
    for record in approval_lifecycle_fixture()
        .into_iter()
        .filter(|record| {
            record["direction"] == "stdout" && record["message"]["method"] == "_x.ai/queue/changed"
        })
        .take(2)
    {
        protocol.receive(record["message"].clone()).unwrap();
    }
    protocol
}

fn byok_text_chunks() -> Vec<Value> {
    approval_lifecycle_fixture()
        .into_iter()
        .filter(|record| {
            record["direction"] == "stdout"
                && record["message"]["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
                && record["message"]["params"]["_meta"]["promptId"]
                    == "f2387e8a-6460-446b-83f1-66bee5fd8efb"
        })
        .map(|record| record["message"].clone())
        .collect()
}

fn pending_native_approval() -> (GrokProtocol, Value) {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    for record in approval_lifecycle_fixture() {
        let message = &record["message"];
        if record["direction"] == "stdin" && message["method"] == "session/prompt" {
            internal_submit(
                &mut protocol,
                Uuid::new_v4(),
                message["params"]["prompt"][0]["text"].as_str().unwrap(),
            );
        } else if record["direction"] == "stdout" {
            if message["method"] == "session/request_permission" {
                return (protocol, message.clone());
            }
            receive_recorded_with_unit_history(&mut protocol, message.clone());
        }
    }
    panic!("真实夹具必须包含审批请求")
}

fn internal_action(
    protocol: &mut GrokProtocol,
    message_id: Uuid,
    action: RuntimeAction,
) -> super::Effects {
    protocol.acp_command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id,
        action,
    })
}

#[test]
fn native_byok_fixture_recovers_two_round_results_and_distinct_approval_outcomes() {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    let mut events = Vec::new();
    for record in approval_lifecycle_fixture() {
        let message = &record["message"];
        if record["direction"] == "stdin" && message["method"] == "session/prompt" {
            let effects = internal_submit(
                &mut protocol,
                Uuid::new_v4(),
                message["params"]["prompt"][0]["text"].as_str().unwrap(),
            );
            assert_eq!(effects.writes[0]["method"], message["method"]);
            assert_eq!(effects.writes[0]["params"], message["params"]);
            assert!(effects.events.is_empty());
        } else if record["direction"] == "stdin" && message.get("result").is_some() {
            let (approval_id, decision) = match message["id"].as_u64().unwrap() {
                0 => ("grok:0", ApprovalDecision::AllowOnce),
                1 => ("grok:1", ApprovalDecision::DenyOnce),
                unexpected => panic!("夹具中出现未审核的审批：{unexpected}"),
            };
            let effects = internal_action(
                &mut protocol,
                Uuid::new_v4(),
                RuntimeAction::RespondApproval {
                    approval_id: approval_id.into(),
                    decision,
                },
            );
            assert_eq!(effects.writes.as_slice(), &[message.clone()]);
            events.extend(effects.events);
        } else if record["direction"] == "stdout" {
            events
                .extend(receive_recorded_with_unit_history(&mut protocol, message.clone()).events);
        }
    }
    let finished: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RuntimeEventKind::TurnFinished {
                outcome, output, ..
            } => Some((outcome.clone(), output.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        finished,
        [
            (TurnOutcome::Completed, "INFINISHELL_GROK_APPROVAL_ROUND_1"),
            (TurnOutcome::Completed, "INFINISHELL_GROK_APPROVAL_ROUND_2"),
            (TurnOutcome::Completed, "ALLOW_CONFIRMED"),
            (TurnOutcome::Cancelled, ""),
        ]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RuntimeEventKind::ApprovalRequested { .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RuntimeEventKind::MessageAccepted { .. }))
            .count(),
        4
    );
    assert!(protocol.pending.is_none());
    assert!(protocol.prompt.is_none());
}

#[test]
fn approval_uses_exact_one_time_option_ids_even_when_another_option_claims_allow_once() {
    let (mut protocol, mut request) = pending_native_approval();
    request["params"]["options"].as_array_mut().unwrap().insert(
        0,
        json!({
            "optionId": "enable-always-approve", "kind": "allow_once", "name": "Always approve"
        }),
    );
    let requested = protocol.receive(request.clone()).unwrap();
    assert!(
        matches!(requested.events.as_slice(), [RuntimeEventKind::ApprovalRequested { approval_id, .. }] if approval_id == "grok:0")
    );
    assert!(protocol.receive(request.clone()).unwrap().events.is_empty());
    let message_id = Uuid::new_v4();
    let action = RuntimeAction::RespondApproval {
        approval_id: "grok:0".into(),
        decision: ApprovalDecision::AllowOnce,
    };
    let selected = internal_action(&mut protocol, message_id, action.clone());
    assert_eq!(
        selected.writes,
        vec![
            json!({"jsonrpc":"2.0","id":0,"result":{"outcome":{"outcome":"selected","optionId":"allow-once"}}})
        ]
    );
    assert!(
        internal_action(&mut protocol, message_id, action)
            .writes
            .is_empty()
    );
    let replay = protocol.receive(request.clone()).unwrap();
    assert!(replay.writes.is_empty());
    assert!(replay.events.is_empty());
    request["params"]["toolCall"]["rawInput"]["content"] = json!("被替换的审批内容");
    assert!(protocol.receive(request).is_err());
}

#[test]
fn mismatched_permission_options_are_cancelled_and_cannot_reopen_after_redelivery() {
    let (mut protocol, mut request) = pending_native_approval();
    request["params"]["options"] = json!([
        {"optionId":"allow-once","kind":"allow_always","name":"Allow"},
        {"optionId":"reject-once","kind":"reject_once","name":"Reject"}
    ]);
    let effects = protocol.receive(request.clone()).unwrap();
    assert!(effects.events.is_empty());
    assert_eq!(
        effects.writes,
        vec![json!({"jsonrpc":"2.0","id":0,"result":{"outcome":{"outcome":"cancelled"}}})]
    );
    assert!(protocol.receive(request.clone()).unwrap().events.is_empty());
    request["params"]["options"][0]["kind"] = json!("allow_once");
    assert!(protocol.receive(request).is_err());
}

#[test]
fn permission_without_a_correlated_active_tool_is_cancelled() {
    let (mut protocol, mut request) = pending_native_approval();
    request["params"]["toolCall"]["toolCallId"] = json!("unrelated-tool");
    let effects = protocol.receive(request).unwrap();
    assert!(effects.events.is_empty());
    assert_eq!(
        effects.writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
}

#[test]
fn finished_tool_revokes_unresolved_approval_and_late_allow_cannot_execute() {
    let (mut protocol, request) = pending_native_approval();
    protocol.receive(request).unwrap();
    let completion = approval_lifecycle_fixture()
        .into_iter()
        .find(|record| {
            record["message"]["params"]["update"]["toolCallId"] == "toolu_01XMbMy7tv7b3NaV5Cmqa1tB"
                && record["message"]["params"]["update"]["status"] == "completed"
        })
        .unwrap()["message"]
        .clone();
    assert_eq!(
        protocol.receive(completion).unwrap().events,
        vec![RuntimeEventKind::ApprovalCancelled {
            approval_id: "grok:0".into()
        }]
    );
    let late = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::RespondApproval {
            approval_id: "grok:0".into(),
            decision: ApprovalDecision::AllowOnce,
        },
    );
    assert!(late.writes.is_empty());
    assert!(matches!(
        late.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}

#[test]
fn text_chunks_wait_for_missing_predecessors_and_duplicate_output_is_not_emitted() {
    let mut protocol = active_byok_prompt();
    let chunks = byok_text_chunks();
    assert!(
        protocol
            .receive(chunks[1].clone())
            .unwrap()
            .events
            .is_empty()
    );
    let first = protocol.receive(chunks[0].clone()).unwrap();
    let text: Vec<_> = first
        .events
        .iter()
        .filter_map(|event| match event {
            RuntimeEventKind::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, ["INFINISHELL_GROK", "_APPROVAL_ROUND_"]);
    assert!(
        protocol
            .receive(chunks[1].clone())
            .unwrap()
            .events
            .is_empty()
    );
    protocol.receive(chunks[2].clone()).unwrap();
    let finished = finish_with_unit_history(&mut protocol, byok_response(4));
    assert_eq!(
        finished.events,
        vec![RuntimeEventKind::TurnFinished {
            turn_id: "f2387e8a-6460-446b-83f1-66bee5fd8efb".into(),
            outcome: TurnOutcome::Completed,
            output: "INFINISHELL_GROK_APPROVAL_ROUND_1".into(),
        }]
    );
    assert!(
        protocol
            .receive(byok_response(4))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn missing_text_chunks_prevent_success_without_verified_history() {
    let mut protocol = active_byok_prompt();
    protocol.receive(byok_text_chunks()[1].clone()).unwrap();
    let effects = protocol.receive(byok_response(4)).unwrap();
    assert!(effects.events.is_empty());
    assert_eq!(effects.writes[0]["method"], "_x.ai/session/updates");
    assert!(protocol.prompt.as_ref().unwrap().completion.is_some());
}

fn shared_chunk(kind: &str, stream: i64, chunk: u64, sequence: u64, text: &str) -> Value {
    let mut message = byok_text_chunks()[0].clone();
    let session_id = message["params"]["sessionId"].as_str().unwrap().to_owned();
    message["params"]["_meta"]["eventId"] = json!(format!("{session_id}-{sequence}"));
    message["params"]["_meta"]["streamStartMs"] = json!(stream);
    message["params"]["_meta"]["chunkId"] = json!(chunk);
    message["params"]["update"]["sessionUpdate"] = json!(kind);
    message["params"]["update"]["content"]["text"] = json!(text);
    message
}

#[test]
fn shared_thought_chunk_unblocks_body_without_publishing_thought_text() {
    let mut protocol = active_byok_prompt();
    let body = shared_chunk("agent_message_chunk", 100, 2, 101, "BODY");
    assert!(protocol.receive(body.clone()).unwrap().events.is_empty());
    let thought = shared_chunk(
        "agent_thought_chunk",
        100,
        1,
        100,
        "PRIVATE_THOUGHT_FIXTURE",
    );
    let effects = protocol.receive(thought.clone()).unwrap();
    assert!(
        matches!(effects.events.as_slice(), [RuntimeEventKind::TextDelta { text, .. }] if text == "BODY")
    );
    assert!(protocol.receive(thought).unwrap().events.is_empty());
    assert!(protocol.receive(body).unwrap().events.is_empty());
    assert_eq!(protocol.prompt.as_ref().unwrap().output, "BODY");
}

#[test]
fn official_thought_and_body_numbering_survives_a_tool_response_boundary() {
    let (mut protocol, after) = multistream_before_final_text();
    let mut thought = after.clone();
    thought["params"]["update"]["sessionUpdate"] = json!("agent_thought_chunk");
    thought["params"]["update"]["content"]["text"] = json!("PRIVATE_THOUGHT_FIXTURE");
    assert!(protocol.receive(thought).unwrap().events.is_empty());
    let mut body = after;
    body["params"]["_meta"]["chunkId"] = json!(2);
    let session_id = body["params"]["sessionId"].as_str().unwrap().to_owned();
    body["params"]["_meta"]["eventId"] = json!(format!("{session_id}-1000"));
    assert!(
        matches!(protocol.receive(body).unwrap().events.as_slice(), [RuntimeEventKind::TextDelta { text, .. }] if text == "AFTER_TOOL")
    );
    assert_eq!(
        protocol.prompt.as_ref().unwrap().output,
        "BEFORE_TOOLAFTER_TOOL"
    );
}

#[test]
fn missing_thought_predecessor_cannot_be_hidden_by_a_new_stream() {
    let mut protocol = active_byok_prompt();
    protocol
        .receive(shared_chunk("agent_message_chunk", 100, 2, 101, "BODY"))
        .unwrap();
    assert!(
        protocol
            .receive(shared_chunk(
                "agent_thought_chunk",
                200,
                1,
                102,
                "PRIVATE_THOUGHT_FIXTURE"
            ))
            .is_err()
    );
}

#[test]
fn thought_identity_cannot_be_reused_as_visible_body() {
    let mut protocol = active_byok_prompt();
    protocol
        .receive(shared_chunk(
            "agent_thought_chunk",
            100,
            1,
            100,
            "PRIVATE_THOUGHT_FIXTURE",
        ))
        .unwrap();
    assert!(
        protocol
            .receive(shared_chunk("agent_message_chunk", 100, 1, 101, "BODY"))
            .is_err()
    );
}

#[test]
fn unseen_thought_fragment_from_a_closed_stream_is_rejected() {
    let mut protocol = active_byok_prompt();
    protocol
        .receive(shared_chunk(
            "agent_thought_chunk",
            100,
            1,
            100,
            "PRIVATE_THOUGHT_FIXTURE",
        ))
        .unwrap();
    protocol
        .receive(shared_chunk("agent_message_chunk", 200, 1, 101, "BODY"))
        .unwrap();
    assert!(
        protocol
            .receive(shared_chunk(
                "agent_thought_chunk",
                100,
                2,
                102,
                "PRIVATE_THOUGHT_FIXTURE"
            ))
            .is_err()
    );
}

#[test]
fn conflicting_chunk_identity_cannot_replace_an_existing_text_fragment() {
    let mut protocol = active_byok_prompt();
    let mut chunk = byok_text_chunks()[0].clone();
    protocol.receive(chunk.clone()).unwrap();
    chunk["params"]["_meta"]["eventId"] = json!("new-event-same-chunk");
    chunk["params"]["update"]["content"]["text"] = json!("replacement");
    assert!(protocol.receive(chunk).is_err());
}

#[test]
fn a_second_stream_with_regressed_event_order_cannot_replace_published_output() {
    let mut protocol = active_byok_prompt();
    let mut chunk = byok_text_chunks()[0].clone();
    protocol.receive(chunk.clone()).unwrap();
    chunk["params"]["_meta"]["eventId"] = json!("01a0adcd-47e8-7453-ac34-a09827726016-3");
    chunk["params"]["_meta"]["streamStartMs"] = json!(1);
    assert!(protocol.receive(chunk).is_err());
}

#[test]
fn stale_and_replayed_chunks_cannot_become_new_turn_output() {
    let mut protocol = active_byok_prompt();
    let mut chunk = byok_text_chunks()[0].clone();
    chunk["params"]["_meta"]["promptId"] = json!("previous-turn");
    assert!(protocol.receive(chunk).unwrap().events.is_empty());
    let mut replay = byok_text_chunks()[1].clone();
    replay["params"]["_meta"]["isReplay"] = json!(true);
    assert!(protocol.receive(replay).unwrap().events.is_empty());
    assert_eq!(protocol.prompt.as_ref().unwrap().output, "");
}

#[test]
fn prompt_result_cannot_change_native_turn_or_session_identity() {
    for (field, value) in [("promptId", "old-turn"), ("sessionId", "another-session")] {
        let mut protocol = active_byok_prompt();
        let mut response = byok_response(4);
        response["result"]["_meta"][field] = json!(value);
        assert!(protocol.receive(response).is_err());
    }
}

#[test]
fn interrupt_dispatch_is_not_a_cancelled_terminal_event() {
    let mut protocol = active_byok_prompt();
    let message_id = Uuid::new_v4();
    let action = RuntimeAction::Interrupt {
        turn_id: "f2387e8a-6460-446b-83f1-66bee5fd8efb".into(),
    };
    let effects = internal_action(&mut protocol, message_id, action.clone());
    assert_eq!(
        effects.writes,
        vec![
            json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"01a0adcd-47e8-7453-ac34-a09827726016"}})
        ]
    );
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::CommandDispatched { .. }]
    ));
    assert!(!protocol.prompt.as_ref().unwrap().finished);
    assert!(
        internal_action(&mut protocol, message_id, action)
            .writes
            .is_empty()
    );
    let late = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::Interrupt {
            turn_id: "older-turn".into(),
        },
    );
    assert!(late.writes.is_empty());
    assert!(matches!(
        late.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}

#[test]
fn unverified_cancellation_category_does_not_claim_cancellation() {
    let mut protocol = active_byok_prompt();
    let mut response = byok_response(4);
    response["result"]["stopReason"] = json!("cancelled");
    response["result"]["_meta"]["cancellationCategory"] = json!("FutureReason");
    assert!(protocol.receive(response).is_err());
}

#[test]
fn confirmed_long_turn_waits_for_user_but_cancel_has_a_bounded_confirmation_timeout() {
    let mut protocol = active_byok_prompt();
    protocol.pending.as_mut().unwrap().sent_at =
        Instant::now() - REQUEST_TIMEOUT - Duration::from_secs(1);
    assert!(!protocol.request_timed_out());
    internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::Interrupt {
            turn_id: "f2387e8a-6460-446b-83f1-66bee5fd8efb".into(),
        },
    );
    protocol.prompt.as_mut().unwrap().cancel_sent =
        Some(Instant::now() - REQUEST_TIMEOUT - Duration::from_secs(1));
    assert!(protocol.request_timed_out());
}

#[test]
fn queued_next_round_is_not_acknowledged_until_native_queue_identifies_it() {
    let mut protocol = active_byok_prompt();
    let message_id = Uuid::new_v4();
    let queued = internal_submit(&mut protocol, message_id, "下一轮");
    assert!(queued.writes.is_empty());
    assert!(queued.events.is_empty());
    assert!(
        internal_submit(&mut protocol, message_id, "下一轮")
            .writes
            .is_empty()
    );
    let complete = finish_with_unit_history(&mut protocol, byok_response(4));
    assert_eq!(complete.writes[0]["method"], "session/prompt");
    assert_eq!(
        complete.writes[0]["params"]["prompt"],
        json!([{"type":"text","text":"下一轮"}])
    );
    assert!(matches!(
        complete.events.as_slice(),
        [RuntimeEventKind::TurnFinished { .. }]
    ));
    assert!(protocol.queued.is_empty());
    assert_eq!(protocol.prompt.as_ref().unwrap().message_id, message_id);
    assert!(protocol.prompt.as_ref().unwrap().native_id.is_none());
}

#[test]
fn shutdown_fails_unsent_inputs_without_claiming_the_running_turn_was_cancelled() {
    let mut protocol = active_byok_prompt();
    let message_id = Uuid::new_v4();
    internal_submit(&mut protocol, message_id, "待发送输入");
    let effects = internal_action(&mut protocol, Uuid::new_v4(), RuntimeAction::Shutdown);
    assert!(effects.writes.is_empty());
    assert!(
        matches!(effects.events.as_slice(), [RuntimeEventKind::CommandDispatched { .. }, RuntimeEventKind::RequestFailed { message_id: id, .. }] if *id == message_id)
    );
    assert!(protocol.queued.is_empty());
    assert!(protocol.closed);
}

#[test]
fn native_cancel_continue_and_process_restart_recover_the_original_session_marker() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-byok-cancel-recovery.json"
    ))
    .unwrap();
    let records = fixture["records"].as_array().unwrap();
    let mut all_events = Vec::new();
    for process in ["first", "second"] {
        let mut options = options();
        if process == "second" {
            options.target = SessionTarget::Resume {
                native_session_id: "01a0add1-d9d9-7411-9a11-590ebd81db56".into(),
            };
        }
        let mut protocol = GrokProtocol::new(options);
        protocol.initialize();
        for record in records.iter().filter(|record| record["process"] == process) {
            let mut message = record["message"].clone();
            if record["direction"] == "stdin" {
                match message["method"].as_str().unwrap() {
                    "initialize" | "authenticate" | "session/new" | "session/load" => {}
                    "session/prompt" => {
                        let effects = internal_submit(
                            &mut protocol,
                            Uuid::new_v4(),
                            message["params"]["prompt"][0]["text"].as_str().unwrap(),
                        );
                        assert_eq!(effects.writes[0]["method"], "session/prompt");
                        assert_eq!(effects.writes[0]["params"], message["params"]);
                        assert!(effects.events.is_empty());
                    }
                    "session/cancel" => {
                        assert_eq!(protocol.prompt.as_ref().unwrap().output, "READY\n\n1");
                        let effects = internal_action(
                            &mut protocol,
                            Uuid::new_v4(),
                            RuntimeAction::Interrupt {
                                turn_id: "43292eea-d88e-42a4-8e56-32420af0fbae".into(),
                            },
                        );
                        assert_eq!(effects.writes, vec![message]);
                        assert!(matches!(
                            effects.events.as_slice(),
                            [RuntimeEventKind::CommandDispatched { .. }]
                        ));
                        assert!(!protocol.prompt.as_ref().unwrap().finished);
                    }
                    "session/close" => {
                        let effects =
                            internal_action(&mut protocol, Uuid::new_v4(), RuntimeAction::Shutdown);
                        assert_eq!(effects.writes[0]["method"], "session/close");
                        assert!(!protocol.closed);
                    }
                    unexpected => panic!("夹具中出现未审核的操作：{unexpected}"),
                }
            } else {
                if let Some(id) = message["id"].as_str() {
                    // 只转换客户端生成的 RPC ID；保留原生会话、回合、事件及工具身份。
                    message["id"] = json!(match id {
                        "first-initialize" | "second-initialize" => 1,
                        "first-authenticate" | "second-authenticate" => 2,
                        "first-new" | "second-load" => 3,
                        "first-running_cancel" | "second-recovered_marker" => 4,
                        "first-same_process_continue" | "second-close" => 5,
                        "first-close" => 6,
                        unexpected => panic!("夹具中出现未知响应：{unexpected}"),
                    });
                }
                let effects = receive_recorded_with_unit_history(&mut protocol, message.clone());
                if process == "second" && message["id"] == 2 {
                    assert_eq!(effects.writes[0]["method"], "session/load");
                    assert_eq!(
                        effects.writes[0]["params"]["sessionId"],
                        "01a0add1-d9d9-7411-9a11-590ebd81db56"
                    );
                }
                if message["params"]["_meta"]["isReplay"] == true {
                    assert!(effects.events.is_empty());
                }
                all_events.extend(effects.events);
            }
        }
        assert!(protocol.closed);
        assert_eq!(
            protocol.session_id.as_deref(),
            Some("01a0add1-d9d9-7411-9a11-590ebd81db56")
        );
        if process == "second" {
            assert!(
                protocol
                    .observed_prompt_ids
                    .contains("43292eea-d88e-42a4-8e56-32420af0fbae")
            );
            assert!(
                protocol
                    .observed_prompt_ids
                    .contains("2183bb44-ddb2-431c-aaf6-111ea3dd8a4b")
            );
        }
    }
    let finished: Vec<_> = all_events
        .iter()
        .filter_map(|event| match event {
            RuntimeEventKind::TurnFinished {
                outcome, output, ..
            } => Some((outcome.clone(), output.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        finished,
        [
            (TurnOutcome::Cancelled, "READY\n\n1"),
            (
                TurnOutcome::Completed,
                "SESSIONMEMO_64f29a9ea6a12f7ddb739a69"
            ),
            (
                TurnOutcome::Completed,
                "SESSIONMEMO_64f29a9ea6a12f7ddb739a69"
            ),
        ]
    );
    assert_eq!(
        all_events
            .iter()
            .filter(|event| matches!(event, RuntimeEventKind::MessageAccepted { .. }))
            .count(),
        3
    );
    assert_eq!(
        all_events
            .iter()
            .filter(|event| matches!(event, RuntimeEventKind::ApprovalRequested { .. }))
            .count(),
        0
    );
}

#[test]
fn advertised_resume_without_load_does_not_replace_the_verified_recovery_method() {
    let mut options = options();
    options.target = SessionTarget::Resume {
        native_session_id: "original-session".into(),
    };
    let mut protocol = GrokProtocol::new(options);
    protocol.initialize();
    let mut initialize = fixture_response(1);
    initialize["result"]["agentCapabilities"]["loadSession"] = json!(false);
    protocol.receive(initialize).unwrap();
    assert!(protocol.receive(fixture_response(2)).is_err());
    assert_eq!(protocol.next_id, 2);
    assert!(protocol.session_id.is_none());
}

#[tokio::test]
async fn next_prompt_write_failure_preserves_the_previous_terminal_result() {
    let mut protocol = active_byok_prompt();
    internal_submit(&mut protocol, Uuid::new_v4(), "下一轮");
    let effects = finish_with_unit_history(&mut protocol, byok_response(4));
    let (sender, mut receiver) = mpsc::channel(4);
    let mut storage = [];
    let mut stdin = Cursor::new(&mut storage[..]);
    assert!(
        flush_effects(&protocol, &mut stdin, &sender, effects)
            .await
            .is_err()
    );
    assert!(matches!(
        receiver.try_recv().unwrap().kind,
        RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        }
    ));
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn failed_cancel_write_never_reports_dispatch_or_cancellation() {
    let mut protocol = active_byok_prompt();
    let effects = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::Interrupt {
            turn_id: "f2387e8a-6460-446b-83f1-66bee5fd8efb".into(),
        },
    );
    let (sender, mut receiver) = mpsc::channel(4);
    let mut storage = [];
    let mut stdin = Cursor::new(&mut storage[..]);
    assert!(
        flush_effects(&protocol, &mut stdin, &sender, effects)
            .await
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
}

fn multistream_fixture() -> Vec<Value> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-byok-multistream.json"
    ))
    .unwrap();
    fixture["records"].as_array().unwrap().clone()
}

fn multistream_before_final_text() -> (GrokProtocol, Value) {
    let mut protocol = GrokProtocol::new(options());
    protocol.initialize();
    for record in multistream_fixture() {
        let message = &record["message"];
        if record["direction"] == "stdin" && message["method"] == "session/prompt" {
            internal_submit(
                &mut protocol,
                Uuid::new_v4(),
                message["params"]["prompt"][0]["text"].as_str().unwrap(),
            );
        } else if record["direction"] == "stdin" && message.get("result").is_some() {
            let effects = internal_action(
                &mut protocol,
                Uuid::new_v4(),
                RuntimeAction::RespondApproval {
                    approval_id: "grok:0".into(),
                    decision: ApprovalDecision::AllowOnce,
                },
            );
            assert_eq!(effects.writes, vec![message.clone()]);
        } else if record["direction"] == "stdout" {
            if message["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
                && message["params"]["update"]["content"]["text"] == "AFTER_TOOL"
            {
                return (protocol, message.clone());
            }
            protocol.receive(message.clone()).unwrap();
        }
    }
    panic!("原生夹具必须包含写入后的第二条文本流")
}

fn multistream_terminal() -> Value {
    multistream_fixture()
        .into_iter()
        .find(|record| record["direction"] == "stdout" && record["message"]["id"] == 4)
        .unwrap()["message"]
        .clone()
}

#[test]
fn native_text_tool_text_uses_distinct_streams_and_preserves_the_complete_result() {
    let (mut protocol, after) = multistream_before_final_text();
    assert_eq!(protocol.prompt.as_ref().unwrap().output, "BEFORE_TOOL");
    assert_eq!(after["params"]["_meta"]["chunkId"], 1);
    assert_eq!(
        protocol.receive(after.clone()).unwrap().events,
        vec![RuntimeEventKind::TextDelta {
            turn_id: "b0c42eab-53e4-4d9e-9f88-02c3c86ad155".into(),
            item_id: "b0c42eab-53e4-4d9e-9f88-02c3c86ad155:1789624350968".into(),
            text: "AFTER_TOOL".into(),
        }]
    );
    assert!(protocol.receive(after).unwrap().events.is_empty());
    for record in multistream_fixture().into_iter().filter(|record| {
        record["message"]["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
            && record["message"]["params"]["_meta"]["streamStartMs"] == 1789624347715_i64
    }) {
        assert!(
            protocol
                .receive(record["message"].clone())
                .unwrap()
                .events
                .is_empty()
        );
    }
    assert_eq!(
        finish_with_unit_history(&mut protocol, multistream_terminal()).events,
        vec![RuntimeEventKind::TurnFinished {
            turn_id: "b0c42eab-53e4-4d9e-9f88-02c3c86ad155".into(),
            outcome: TurnOutcome::Completed,
            output: "BEFORE_TOOLAFTER_TOOL".into(),
        }]
    );
}

#[test]
fn second_stream_chunks_are_buffered_in_chunk_order_without_sorting_timestamps() {
    let (mut protocol, mut first) = multistream_before_final_text();
    // 在保留原始夹具之外注入乱序分片和回拨时间戳，验证顺序来自事件与分片身份。
    first["params"]["_meta"]["streamStartMs"] = json!(1);
    first["params"]["update"]["content"]["text"] = json!("AFTER");
    let mut second = first.clone();
    second["params"]["_meta"]["chunkId"] = json!(2);
    second["params"]["_meta"]["eventId"] = json!("01a0adec-6149-7ff3-a254-1e786010a9b4-13");
    second["params"]["update"]["content"]["text"] = json!("_TOOL");
    assert!(protocol.receive(second).unwrap().events.is_empty());
    let output = protocol.receive(first).unwrap();
    let text: Vec<_> = output
        .events
        .iter()
        .filter_map(|event| match event {
            RuntimeEventKind::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, ["AFTER", "_TOOL"]);
    assert!(
        matches!(finish_with_unit_history(&mut protocol, multistream_terminal()).events.as_slice(),
        [RuntimeEventKind::TurnFinished { outcome: TurnOutcome::Completed, output, .. }] if output == "BEFORE_TOOLAFTER_TOOL")
    );
}

#[test]
fn unseen_old_stream_fragment_after_a_new_stream_fails_instead_of_appending_old_text() {
    let (mut protocol, after) = multistream_before_final_text();
    protocol.receive(after).unwrap();
    let mut old = multistream_fixture()
        .into_iter()
        .find(|record| {
            record["message"]["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
        })
        .unwrap()["message"]
        .clone();
    old["params"]["_meta"]["eventId"] = json!("01a0adec-6149-7ff3-a254-1e786010a9b4-9999");
    old["params"]["_meta"]["chunkId"] = json!(3);
    old["params"]["update"]["content"]["text"] = json!("迟到旧文本");
    assert!(protocol.receive(old).is_err());
    assert_eq!(
        protocol.prompt.as_ref().unwrap().output,
        "BEFORE_TOOLAFTER_TOOL"
    );
}

#[test]
fn changing_streams_cannot_hide_a_known_missing_fragment() {
    let mut protocol = active_byok_prompt();
    let mut next_stream = byok_text_chunks()[1].clone();
    protocol.receive(next_stream.clone()).unwrap();
    next_stream["params"]["_meta"]["eventId"] = json!("01a0adcd-47e8-7453-ac34-a09827726016-100");
    next_stream["params"]["_meta"]["streamStartMs"] = json!(2);
    next_stream["params"]["_meta"]["chunkId"] = json!(1);
    assert!(protocol.receive(next_stream).is_err());
}

fn multistream_history() -> Value {
    // 把真实增量及终态按已核验的磁盘 envelope 形式组合；这是故障注入数据，非真实查询回执。
    let updates: Vec<_> = multistream_fixture()
        .into_iter()
        .filter_map(|record| {
            let message = &record["message"];
            let update = &message["params"]["update"];
            if record["direction"] != "stdout" {
                return None;
            }
            let method = match update["sessionUpdate"].as_str() {
                Some("agent_message_chunk") => "session/update",
                Some("turn_completed") => "_x.ai/session/update",
                Some(_) | None => return None,
            };
            Some(json!({"method":method,"params":message["params"]}))
        })
        .collect();
    json!({"lastEventId":updates.last().unwrap()["params"]["_meta"]["eventId"],
        "totalCount":updates.len(),"hasMore":false,"updates":updates})
}

fn reply_history(protocol: &mut GrokProtocol, result: Value) -> super::Effects {
    protocol
        .receive(
            json!({"jsonrpc":"2.0","id":protocol.pending.as_ref().unwrap().id,"result":result}),
        )
        .unwrap()
}

#[test]
fn native_offline_query_proves_empty_snapshot_is_not_a_completion_watermark() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../specs/cli-agent-parity/fixtures/grok-1.0.30-final-history-offline.json"
    ))
    .unwrap();
    let result = &fixture["builtinTurn"]["result"];
    let session = result["_meta"]["sessionId"].as_str().unwrap();
    let turn = result["_meta"]["promptId"].as_str().unwrap();
    assert_eq!(
        super::verified_final_snapshot(
            &fixture["immediateSnapshot"]["result"],
            session,
            turn,
            &TurnOutcome::Completed
        )
        .unwrap(),
        None
    );
    let (text, watermark) = super::verified_final_snapshot(
        &fixture["verifiedSnapshot"]["result"],
        session,
        turn,
        &TurnOutcome::Completed,
    )
    .unwrap()
    .unwrap();
    assert!(text.contains(session));
    assert_eq!(watermark, format!("{session}-4"));
}

#[test]
fn prompt_rpc_before_an_unknown_text_stream_waits_for_authoritative_history() {
    let (mut protocol, after) = multistream_before_final_text();
    let response = protocol.receive(multistream_terminal()).unwrap();
    assert!(response.events.is_empty());
    assert_eq!(response.writes[0]["method"], "_x.ai/session/updates");
    assert!(protocol.receive(after).unwrap().events.iter().any(
        |event| matches!(event,RuntimeEventKind::TextDelta { text, .. } if text=="AFTER_TOOL")
    ));
    let completed = reply_history(&mut protocol, multistream_history());
    assert!(
        matches!(completed.events.as_slice(),[RuntimeEventKind::TurnFinished {outcome:TurnOutcome::Completed,output,..}] if output=="BEFORE_TOOLAFTER_TOOL")
    );
}

#[test]
fn authoritative_history_recovers_an_unseen_stream_and_late_transport_cannot_duplicate_it() {
    let (mut protocol, after) = multistream_before_final_text();
    protocol.receive(multistream_terminal()).unwrap();
    let id = protocol.pending.as_ref().unwrap().id;
    let result = multistream_history();
    let completed = reply_history(&mut protocol, result.clone());
    assert!(matches!(completed.events.as_slice(),[
        RuntimeEventKind::TextDelta{text,..},RuntimeEventKind::TurnFinished{outcome:TurnOutcome::Completed,output,..}
    ] if text=="AFTER_TOOL" && output=="BEFORE_TOOLAFTER_TOOL"));
    assert!(protocol.receive(after).unwrap().events.is_empty());
    assert!(
        protocol
            .receive(json!({"jsonrpc":"2.0","id":id,"result":result}))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(protocol.prompt.is_none());
}

#[tokio::test]
async fn recovered_text_is_published_before_the_terminal_event() {
    let (mut protocol, _) = multistream_before_final_text();
    protocol.receive(multistream_terminal()).unwrap();
    let effects = reply_history(&mut protocol, multistream_history());
    let (sender, mut receiver) = mpsc::channel(4);
    let mut stdin = Cursor::new(Vec::new());
    flush_effects(&protocol, &mut stdin, &sender, effects)
        .await
        .unwrap();
    assert!(
        matches!(receiver.try_recv().unwrap().kind,RuntimeEventKind::TextDelta{text,..} if text=="AFTER_TOOL")
    );
    assert!(matches!(
        receiver.try_recv().unwrap().kind,
        RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        }
    ));
}

#[test]
fn empty_snapshot_retries_without_acking_or_sending_the_queued_prompt() {
    let (mut protocol, _) = multistream_before_final_text();
    protocol.receive(multistream_terminal()).unwrap();
    let empty = json!({"updates":[],"totalCount":0,"hasMore":false});
    let first_id = protocol.pending.as_ref().unwrap().id;
    let first = reply_history(&mut protocol, empty.clone());
    assert!(first.events.is_empty() && first.writes.is_empty());
    let queued_id = Uuid::new_v4();
    let queued = internal_submit(&mut protocol, queued_id, "下一轮");
    assert!(queued.events.is_empty() && queued.writes.is_empty());
    protocol
        .prompt
        .as_mut()
        .unwrap()
        .completion
        .as_mut()
        .unwrap()
        .retry_at = Some(Instant::now());
    let retry = protocol.poll_final_output();
    assert_eq!(retry.writes[0]["method"], "_x.ai/session/updates");
    let retry_id = protocol.pending.as_ref().unwrap().id;
    assert!(retry_id > first_id);
    assert!(
        protocol
            .receive(json!({"jsonrpc":"2.0","id":first_id,"result":empty}))
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(protocol.pending.as_ref().unwrap().id, retry_id);
    let completed = reply_history(&mut protocol, multistream_history());
    assert_eq!(completed.writes[0]["method"], "session/prompt");
    assert_eq!(protocol.prompt.as_ref().unwrap().message_id, queued_id);
    assert!(!completed.events.iter().any(|event|matches!(event,RuntimeEventKind::MessageAccepted{message_id,..} if *message_id==queued_id)));
}

#[test]
fn an_old_prompt_watermark_never_completes_the_current_prompt() {
    let (mut protocol, _) = multistream_before_final_text();
    protocol.receive(multistream_terminal()).unwrap();
    let mut result = multistream_history();
    result["updates"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["params"]["update"]["prompt_id"] = json!("old-prompt");
    let effects = reply_history(&mut protocol, result);
    assert!(effects.events.is_empty());
    assert!(
        protocol
            .prompt
            .as_ref()
            .unwrap()
            .completion
            .as_ref()
            .unwrap()
            .retry_at
            .is_some()
    );
}

#[test]
fn inconsistent_or_truncated_snapshots_explicitly_fail_output_recovery() {
    for mutation in 0..8 {
        let (mut protocol, _) = multistream_before_final_text();
        protocol.receive(multistream_terminal()).unwrap();
        let mut result = multistream_history();
        match mutation {
            0 => result["hasMore"] = json!(true),
            1 => result["totalCount"] = json!(super::MAX_NATIVE_IDENTITIES + 1),
            2 => result["lastEventId"] = json!("different-session-16"),
            3 => result["updates"][0]["params"]["sessionId"] = json!("different-session"),
            4 => result["updates"][0]["params"]["update"]["content"]["text"] = json!("conflicting"),
            5 => result["updates"][3]["params"]["update"]["stop_reason"] = json!("cancelled"),
            6 => {
                result["updates"][1]["params"]["_meta"]["eventId"] =
                    result["updates"][0]["params"]["_meta"]["eventId"].clone()
            }
            7 => result["updates"].as_array_mut().unwrap().swap(2, 3),
            unexpected => panic!("未知变体：{unexpected}"),
        }
        let effects = reply_history(&mut protocol, result);
        assert!(
            matches!(effects.events.as_slice(),[RuntimeEventKind::TurnFinished{outcome:TurnOutcome::Failed{..},output,..}] if output=="BEFORE_TOOL")
        );
        assert!(protocol.prompt.is_none());
    }
}

#[test]
fn unsupported_history_query_preserves_partial_text_and_never_reports_success() {
    let (mut protocol, _) = multistream_before_final_text();
    protocol.receive(multistream_terminal()).unwrap();
    let effects=protocol.receive(json!({"jsonrpc":"2.0","id":protocol.pending.as_ref().unwrap().id,"error":{"code":-32601,"message":"unsupported"}})).unwrap();
    assert!(
        matches!(effects.events.as_slice(),[RuntimeEventKind::TurnFinished{outcome:TurnOutcome::Failed{..},output,..}] if output=="BEFORE_TOOL")
    );
}

#[test]
fn malformed_history_response_reports_output_recovery_failure() {
    let (mut protocol, _) = multistream_before_final_text();
    protocol.receive(multistream_terminal()).unwrap();
    let effects = protocol
        .receive(json!({"jsonrpc":"2.0","id":protocol.pending.as_ref().unwrap().id,"result":null}))
        .unwrap();
    assert!(
        matches!(effects.events.as_slice(),[RuntimeEventKind::TurnFinished{outcome:TurnOutcome::Failed{..},output,..}] if output=="BEFORE_TOOL")
    );
    assert!(protocol.prompt.is_none());
}

#[test]
fn missing_watermark_deadline_fails_and_closes_without_reexecuting_queued_work() {
    let (mut protocol, _) = multistream_before_final_text();
    protocol.receive(multistream_terminal()).unwrap();
    internal_submit(&mut protocol, Uuid::new_v4(), "排队但不允许重复执行");
    protocol
        .prompt
        .as_mut()
        .unwrap()
        .completion
        .as_mut()
        .unwrap()
        .started_at = Instant::now() - REQUEST_TIMEOUT - Duration::from_secs(1);
    let effects = protocol.poll_final_output();
    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [
            RuntimeEventKind::TurnFinished {
                outcome: TurnOutcome::Failed { .. },
                ..
            },
            RuntimeEventKind::RequestFailed { .. }
        ]
    ));
    assert!(protocol.closed && protocol.prompt.is_none() && protocol.queued.is_empty());
}

#[test]
fn a_native_completed_turn_cannot_receive_a_new_approval_or_cancel() {
    let (mut protocol, request) = pending_native_approval();
    let turn = protocol.prompt.as_ref().unwrap().native_id.clone().unwrap();
    let id = protocol.pending.as_ref().unwrap().id;
    protocol.receive(json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"end_turn","_meta":{"sessionId":protocol.session_id,"promptId":turn}}})).unwrap();
    assert_eq!(
        protocol.receive(request).unwrap().writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    let cancelled = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::Interrupt { turn_id: turn },
    );
    assert!(cancelled.writes.is_empty());
    assert!(matches!(
        cancelled.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}

#[test]
fn shutdown_during_final_history_reports_partial_failure_exactly_once() {
    let (mut protocol, late_text) = multistream_before_final_text();
    protocol.receive(multistream_terminal()).unwrap();
    let queued = [Uuid::new_v4(), Uuid::new_v4()];
    for id in queued {
        internal_submit(&mut protocol, id, "尚未发送的排队请求");
    }
    let mut effects = internal_action(&mut protocol, Uuid::new_v4(), RuntimeAction::Shutdown);
    assert!(protocol.closed);
    let cleanup = protocol.finish_transport(&Ok(()));
    effects.events.extend(cleanup.events);
    effects.writes.extend(cleanup.writes);
    assert!(effects.writes.is_empty());
    assert_eq!(
        effects
            .events
            .iter()
            .filter_map(|event| match event {
                RuntimeEventKind::TurnFinished {
                    outcome, output, ..
                } => Some((outcome, output.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>()
            .as_slice(),
        &[(
            &TurnOutcome::Failed {
                message: crate::t!("cli-agent-grok-output-unverified"),
            },
            "BEFORE_TOOL"
        )]
    );
    for id in queued {
        assert_eq!(effects.events.iter().filter(|event| matches!(event,RuntimeEventKind::RequestFailed {message_id,..} if message_id==&id)).count(), 1);
    }
    assert!(protocol.pending.is_none() && protocol.prompt.is_none() && protocol.queued.is_empty());
    assert!(protocol.finish_transport(&Ok(())).events.is_empty());
    assert!(protocol.receive(late_text).unwrap().events.is_empty());
}

#[test]
fn normal_transport_exit_during_final_history_rejects_all_unsent_work() {
    let (mut protocol, _) = multistream_before_final_text();
    protocol.receive(multistream_terminal()).unwrap();
    let queued = [Uuid::new_v4(), Uuid::new_v4()];
    for id in queued {
        internal_submit(&mut protocol, id, "控制器关闭后不得执行");
    }
    // 控制通道关闭走传输的正常返回，必须与错误退出一样收束未核验的输出。
    let effects = protocol.finish_transport(&Ok(()));
    assert!(effects.writes.is_empty());
    assert!(
        matches!(effects.events.first(),Some(RuntimeEventKind::TurnFinished {outcome:TurnOutcome::Failed {..}, output,..}) if output=="BEFORE_TOOL")
    );
    assert_eq!(effects.events.len(), 1 + queued.len());
    for id in queued {
        assert_eq!(effects.events.iter().filter(|event| matches!(event,RuntimeEventKind::RequestFailed {message_id,..} if message_id==&id)).count(), 1);
    }
    assert!(
        protocol.closed
            && protocol.pending.is_none()
            && protocol.prompt.is_none()
            && protocol.queued.is_empty()
    );
    assert!(protocol.finish_transport(&Ok(())).events.is_empty());
}

#[test]
fn production_text_command_dispatches_before_native_ack_and_queues_without_claiming_receipt() {
    let mut protocol = ready_protocol();
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let input = "中文与English\n第二行";
    let dispatch = internal_submit(&mut protocol, first, input);
    assert_eq!(dispatch.writes.len(), 1);
    assert_eq!(dispatch.writes[0]["method"], "session/prompt");
    assert_eq!(
        dispatch.writes[0]["params"]["prompt"],
        json!([{"type":"text", "text":input}])
    );
    assert!(
        !dispatch
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEventKind::MessageAccepted { .. }))
    );
    let queued = internal_submit(&mut protocol, second, "只在后续回合执行");
    assert!(queued.writes.is_empty());
    assert!(queued.events.is_empty());
    assert_eq!(protocol.queued.len(), 1);
    assert!(
        internal_submit(&mut protocol, second, "只在后续回合执行")
            .writes
            .is_empty()
    );
    assert_eq!(protocol.queued.len(), 1);
    assert!(matches!(
        internal_submit(&mut protocol, second, "禁止复用消息ID改变内容")
            .events
            .as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}

#[test]
fn root_only_adapter_rejects_parent_ceiling_and_local_tools_before_spawn() {
    let mut launch = options();
    let ceiling = serde_json::from_value(json!({
        "parent_task_id":"parent", "parent_generation":1,
        "parent_native_session_id":"native-parent", "working_directory":launch.cwd,
        "permissions":{"approvalPolicy":"on-request", "approvalsReviewer":"user",
            "sandbox":{"type":"readOnly", "networkAccess":false}}
    }))
    .unwrap();
    launch.permission_ceiling = Some(ceiling);
    assert!(validate_options(&launch).is_err());
    launch.permission_ceiling = None;
    launch.local_tools = Some(LocalToolPermissions::default());
    assert!(validate_options(&launch).is_err());
    launch.local_tools = None;
    assert!(validate_options(&launch).is_ok());
}
