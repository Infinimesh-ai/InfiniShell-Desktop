//! 只验证无模型守卫与解析边界；合成响应不构成原生接口或清理证明。

use std::path::Path;

use futures::executor::block_on;
use futures::io::Cursor;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    ALLOWED_METHODS, DIAGNOSTIC_METHODS, MAX_BYTES, MAX_FRAMES, Observations, Plan, RequestKind,
    SCOPE, WireReader, confirm_info, guard_outbound, hash, mcp_counts, normal_receipt,
    notification_diagnostic, outbound, phase_outcome, response_body,
};
use crate::ai::cli_agent_runtime::managed_process::ExitReceipt;

const SESSION: &str = "11111111-1111-4111-8111-111111111111";
const FOREIGN: &str = "22222222-2222-4222-8222-222222222222";
#[cfg(not(windows))]
const CWD: &str = "/isolated/project";
#[cfg(windows)]
const CWD: &str = r"C:\isolated\project";

fn plan_value() -> Value {
    json!({"schema_version":1,"scope":SCOPE,"max_native_inputs":0,
        "max_protocol_requests":12,"max_session_processes":2,"allowed_methods":ALLOWED_METHODS,
        "diagnostic_methods":DIAGNOSTIC_METHODS,"profile_sha256":hash(b"profile"),"config_sha256":hash(b"config"),
        "always_approve_requested":false,"auto_mode_requested":false,"protocol_guard_required_before_write":true,
        "reject_reverse_tool_requests":true,"model_http_count_measured":false,"candidate_source_is_exact_binary":false})
}

#[test]
fn initialization_advertises_no_reverse_execution_capabilities() {
    let request = outbound(RequestKind::Initialize, 1, Path::new(CWD), None).unwrap();
    assert_eq!(
        request["params"],
        json!({"protocolVersion":1,"clientCapabilities":{
        "fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}})
    );
}

#[test]
fn creation_cannot_add_automatic_approval_or_execution_sources() {
    let mut request = outbound(RequestKind::New, 2, Path::new(CWD), None).unwrap();
    request["params"]["_meta"]["yoloMode"] = json!(true);
    assert_eq!(
        guard_outbound(&request, RequestKind::New, 2, Path::new(CWD), None),
        Err("outbound_parameters_rejected")
    );

    let mut request = outbound(RequestKind::New, 2, Path::new(CWD), None).unwrap();
    request["params"]["mcpServers"] =
        json!([{"name":"unregistered-source","command":"do-not-execute"}]);
    assert_eq!(
        guard_outbound(&request, RequestKind::New, 2, Path::new(CWD), None),
        Err("outbound_parameters_rejected")
    );
}

#[test]
fn mcp_diagnostic_cannot_refresh_or_change_native_session() {
    let mut request = outbound(RequestKind::McpList, 5, Path::new(CWD), Some(SESSION)).unwrap();
    request["params"]["cache"] = json!(false);
    assert_eq!(
        guard_outbound(
            &request,
            RequestKind::McpList,
            5,
            Path::new(CWD),
            Some(SESSION)
        ),
        Err("outbound_parameters_rejected")
    );

    let mut request = outbound(RequestKind::McpList, 5, Path::new(CWD), Some(SESSION)).unwrap();
    request["params"]["sessionId"] = json!(FOREIGN);
    assert_eq!(
        guard_outbound(
            &request,
            RequestKind::McpList,
            5,
            Path::new(CWD),
            Some(SESSION)
        ),
        Err("outbound_parameters_rejected")
    );
}

#[test]
fn method_replacement_cannot_introduce_prompt_or_tool_execution() {
    let mut request = outbound(RequestKind::Info, 3, Path::new(CWD), Some(SESSION)).unwrap();
    request["method"] = json!("session/prompt");
    assert_eq!(
        guard_outbound(
            &request,
            RequestKind::Info,
            3,
            Path::new(CWD),
            Some(SESSION)
        ),
        Err("outbound_parameters_rejected")
    );

    let mut request = outbound(RequestKind::DebugAgent, 6, Path::new(CWD), Some(SESSION)).unwrap();
    request["method"] = json!("x.ai/debug/trigger_feedback");
    assert_eq!(
        guard_outbound(
            &request,
            RequestKind::DebugAgent,
            6,
            Path::new(CWD),
            Some(SESSION)
        ),
        Err("outbound_parameters_rejected")
    );
}

#[test]
fn request_identity_and_extra_fields_fail_before_serialized_write() {
    assert_eq!(
        outbound(RequestKind::Info, 0, Path::new(CWD), Some(SESSION)),
        Err("request_identity_invalid")
    );
    assert_eq!(
        outbound(RequestKind::Info, 13, Path::new(CWD), Some(SESSION)),
        Err("request_identity_invalid")
    );
    assert_eq!(
        outbound(RequestKind::Info, 3, Path::new(CWD), None),
        Err("native_session_missing")
    );
    let mut request = outbound(RequestKind::State, 4, Path::new(CWD), Some(SESSION)).unwrap();
    request["params"]["raw_instruction"] = json!("OFFLINE_DO_NOT_EXECUTE");
    assert_eq!(
        guard_outbound(
            &request,
            RequestKind::State,
            4,
            Path::new(CWD),
            Some(SESSION)
        ),
        Err("outbound_parameters_rejected")
    );
}

#[test]
fn nonzero_native_input_and_zero_request_budget_never_prepare_transport() {
    let mut value = plan_value();
    value["max_native_inputs"] = json!(1);
    assert_eq!(
        serde_json::from_value::<Plan>(value).unwrap().validate(),
        Err("plan_budget_invalid")
    );
    let mut value = plan_value();
    value["max_protocol_requests"] = json!(0);
    assert_eq!(
        serde_json::from_value::<Plan>(value).unwrap().validate(),
        Err("plan_budget_invalid")
    );
    let mut value = plan_value();
    value["max_native_inputs"] = json!(false);
    assert!(serde_json::from_value::<Plan>(value).is_err());
}

#[test]
fn unknown_plan_fields_and_relaxed_reverse_guard_are_rejected() {
    let mut value = plan_value();
    value["enable_parent_ceiling"] = json!(true);
    assert!(serde_json::from_value::<Plan>(value).is_err());
    let mut value = plan_value();
    value["reject_reverse_tool_requests"] = json!(false);
    assert_eq!(
        serde_json::from_value::<Plan>(value).unwrap().validate(),
        Err("plan_guard_invalid")
    );
}

#[test]
fn missing_method_is_failure_and_does_not_become_empty_tool_catalog() {
    let response =
        json!({"jsonrpc":"2.0","id":5,"error":{"code":-32601,"message":"OFFLINE_METHOD_CANARY"}});
    assert_eq!(
        response_body(&response, 5, RequestKind::McpList),
        Err("method_not_found")
    );
}

#[test]
fn extension_partial_error_and_guessed_data_envelope_are_rejected() {
    let response = json!({"jsonrpc":"2.0","id":3,"result":{
        "result":{"sessionId":SESSION},"error":"OFFLINE_PARTIAL_ERROR_CANARY"}});
    assert_eq!(
        response_body(&response, 3, RequestKind::Info),
        Err("extension_result_failed")
    );
    let response = json!({"jsonrpc":"2.0","id":3,"result":{"data":{"sessionId":SESSION}}});
    assert_eq!(
        response_body(&response, 3, RequestKind::Info),
        Err("extension_result_missing")
    );
}

#[test]
fn stale_response_id_or_extra_native_fields_cannot_satisfy_request() {
    let response = json!({"jsonrpc":"2.0","id":2,"result":{"result":{}}});
    assert_eq!(
        response_body(&response, 3, RequestKind::Info),
        Err("native_response_uncorrelated")
    );
    let response =
        json!({"jsonrpc":"2.0","id":3,"result":{"result":{}},"raw_body":"OFFLINE_BODY_CANARY"});
    assert_eq!(
        response_body(&response, 3, RequestKind::Info),
        Err("native_response_uncorrelated")
    );
}

#[test]
fn metadata_binding_rejects_a_foreign_session() {
    let mut observations = Observations::default();
    observations.bind(SESSION).unwrap();
    assert_eq!(observations.bind(FOREIGN), Err("native_session_changed"));
    assert_eq!(observations.native_session.as_deref(), Some(SESSION));
}

#[test]
fn reverse_permission_mcp_and_filesystem_requests_never_get_allowance() {
    let mut observations = Observations::default();
    let permission = json!({"jsonrpc":"2.0","id":99,"method":"session/request_permission",
        "params":{"sessionId":SESSION,"toolCall":{"toolCallId":"OFFLINE_TOOL"}}});
    assert_eq!(
        observations.notification(&permission),
        Err("native_reverse_request_rejected")
    );
    let sdk = json!({"jsonrpc":"2.0","id":99,"method":"x.ai/mcp/sdk_call","params":{}});
    assert_eq!(
        observations.notification(&sdk),
        Err("native_reverse_request_rejected")
    );
    let filesystem = json!({"jsonrpc":"2.0","id":99,"method":"fs/write_text_file","params":{}});
    assert_eq!(
        observations.notification(&filesystem),
        Err("native_reverse_request_rejected")
    );
    assert_eq!(observations.received_notifications, 0);
}

#[test]
fn agent_text_and_tool_lifecycle_are_not_harmless_metadata() {
    let mut observations = Observations::default();
    let text = json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":SESSION,"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"OFFLINE_TEXT"}}}});
    assert_eq!(
        observations.notification(&text),
        Err("native_turn_or_tool_activity_rejected")
    );
    let tool = json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":SESSION,"update":{"sessionUpdate":"tool_call","toolCallId":"OFFLINE_TOOL"}}});
    assert_eq!(
        observations.notification(&tool),
        Err("native_turn_or_tool_activity_rejected")
    );
    assert_eq!(observations.received_notifications, 0);
}

#[test]
fn initial_available_commands_do_not_prove_any_permission_mode() {
    let mut observations = Observations::default();
    let metadata = json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":SESSION,"update":{"sessionUpdate":"available_commands_update",
            "availableCommands":[{"name":"always-approve"}]}}});
    observations.notification(&metadata).unwrap();
    assert_eq!(observations.received_notifications, 1);
    assert_eq!(observations.native_session.as_deref(), Some(SESSION));
}

#[test]
fn unknown_notification_diagnostic_hashes_method_without_accepting_it() {
    let generation = Uuid::new_v4();
    let frame = json!({"jsonrpc":"2.0","method":"OFFLINE_PRIVATE_METHOD",
        "params":{"sessionId":SESSION,"body":"OFFLINE_MODEL_BODY"}});
    let diagnostic = notification_diagnostic(&frame, generation);
    assert_eq!(diagnostic["method"]["type"], "string");
    assert_eq!(
        diagnostic["method"]["sha256"],
        hash(b"OFFLINE_PRIVATE_METHOD")
    );
    assert_eq!(diagnostic["generation"], json!(generation));
    for forbidden in ["OFFLINE_PRIVATE_METHOD", "OFFLINE_MODEL_BODY", SESSION] {
        assert!(!diagnostic.to_string().contains(forbidden));
    }
    assert_eq!(
        Observations::default().notification(&frame),
        Err("native_unknown_notification_rejected")
    );
}

#[test]
fn known_mcp_initialization_diagnostic_does_not_expand_policy_white_list() {
    let frame = json!({"jsonrpc":"2.0","method":"_x.ai/mcp/init_progress",
        "params":{"sessionId":SESSION,"total":0,"connected":0}});
    let diagnostic = notification_diagnostic(&frame, Uuid::nil());
    assert_eq!(diagnostic["method"], "_x.ai/mcp/init_progress");
    assert_eq!(diagnostic["method_summary"]["type"], "string");
    assert_eq!(
        Observations::default().notification(&frame),
        Err("native_unknown_notification_rejected")
    );
}

#[test]
fn empty_global_catalog_preserves_the_unbound_session_and_pending_rpc() {
    let mut observations = Observations {
        pending_response: Some((2, RequestKind::New)),
        ..Observations::default()
    };
    let frame = json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated",
        "params":{"mcpServers":[]}});

    assert_eq!(observations.notification(&frame), Ok(()));
    assert_eq!(observations.native_session, None);
    assert_eq!(observations.pending_response, Some((2, RequestKind::New)));
    assert!(observations.completed_responses.is_empty());
    assert_eq!(observations.received_notifications, 1);
    let diagnostic = notification_diagnostic(&frame, Uuid::nil());
    assert_eq!(diagnostic["global_catalog_closed_empty"], true);
    assert_eq!(diagnostic["session_id"], json!({"type":"absent"}));
}

#[test]
fn global_catalog_accepts_only_the_actual_new_and_load_registered_lists() {
    let new = outbound(RequestKind::New, 2, Path::new(CWD), None).unwrap();
    let load = outbound(RequestKind::Load, 8, Path::new(CWD), Some(SESSION)).unwrap();
    assert_eq!(new["params"]["mcpServers"], json!([]));
    assert_eq!(load["params"]["mcpServers"], json!([]));
    let mut observations = Observations::default();
    observations.bind(SESSION).unwrap();

    assert_eq!(
        observations.notification(
            &json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated",
            "params":{"mcpServers":new["params"]["mcpServers"]}})
        ),
        Ok(())
    );
    assert_eq!(
        observations.notification(
            &json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated",
            "params":{"mcpServers":load["params"]["mcpServers"]}})
        ),
        Ok(())
    );
    assert_eq!(observations.native_session.as_deref(), Some(SESSION));
    assert_eq!(observations.received_notifications, 2);
}

#[test]
fn nonempty_global_catalog_cannot_introduce_unregistered_servers() {
    let mut observations = Observations::default();
    let frame = json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated",
        "params":{"mcpServers":[{"name":"unregistered-source","source":"local",
            "type":"stdio","command":"OFFLINE_DO_NOT_EXECUTE"}]}});

    assert_eq!(
        observations.notification(&frame),
        Err("native_extra_mcp_sources_observed")
    );
    assert_eq!(observations.received_notifications, 0);
    assert_eq!(observations.native_session, None);
}

#[test]
fn global_catalog_rejects_missing_malformed_and_extra_metadata() {
    for params in [
        json!({}),
        json!({"mcpServers":null}),
        json!({"mcpServers":{}}),
        json!({"mcpServers":"OFFLINE_NOT_A_LIST"}),
        json!({"mcpServers":[],"servers":[]}),
        json!({"mcpServers":[],"_meta":{"capabilities":{}}}),
        json!({"mcpServers":[],"tools":[]}),
    ] {
        let mut observations = Observations::default();
        let frame = json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated","params":params});

        assert_eq!(
            observations.notification(&frame),
            Err("native_mcp_catalog_invalid")
        );
        assert_eq!(observations.received_notifications, 0);
        assert_eq!(observations.native_session, None);
    }
}

#[test]
fn global_catalog_cannot_claim_a_foreign_session_or_native_generation() {
    for params in [
        json!({"mcpServers":[],"sessionId":FOREIGN}),
        json!({"mcpServers":[],"sessionId":SESSION}),
        json!({"mcpServers":[],"generation":FOREIGN}),
        json!({"mcpServers":[],"_meta":{"generation":FOREIGN}}),
    ] {
        let mut observations = Observations {
            native_session: Some(SESSION.to_owned()),
            pending_response: Some((3, RequestKind::Info)),
            ..Observations::default()
        };
        let frame = json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated","params":params});

        assert_eq!(
            observations.notification(&frame),
            Err("native_mcp_catalog_invalid")
        );
        assert_eq!(observations.native_session.as_deref(), Some(SESSION));
        assert_eq!(observations.pending_response, Some((3, RequestKind::Info)));
        assert_eq!(observations.received_notifications, 0);
    }
}

#[test]
fn global_catalog_rejects_response_request_and_outer_identity_injection() {
    for frame in [
        json!({"jsonrpc":"1.0","method":"_x.ai/mcp/servers_updated","params":{"mcpServers":[]}}),
        json!({"jsonrpc":"2.0","id":2,"method":"_x.ai/mcp/servers_updated","params":{"mcpServers":[]}}),
        json!({"jsonrpc":"2.0","id":"skills-reload","method":"_x.ai/mcp/servers_updated","params":{"mcpServers":[]}}),
        json!({"jsonrpc":"2.0","result":{},"method":"_x.ai/mcp/servers_updated","params":{"mcpServers":[]}}),
        json!({"jsonrpc":"2.0","error":{},"method":"_x.ai/mcp/servers_updated","params":{"mcpServers":[]}}),
        json!({"jsonrpc":"2.0","generation":FOREIGN,"method":"_x.ai/mcp/servers_updated","params":{"mcpServers":[]}}),
    ] {
        let mut observations = Observations {
            pending_response: Some((2, RequestKind::New)),
            ..Observations::default()
        };

        assert_eq!(
            observations.notification(&frame),
            Err("native_reverse_request_rejected")
        );
        assert_eq!(observations.pending_response, Some((2, RequestKind::New)));
        assert!(observations.completed_responses.is_empty());
        assert_eq!(observations.received_notifications, 0);
    }
}

#[test]
fn repeated_and_reordered_global_catalogs_do_not_complete_owned_rpcs() {
    let mut observations = Observations {
        pending_response: Some((1, RequestKind::Initialize)),
        ..Observations::default()
    };
    let catalog = json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated",
        "params":{"mcpServers":[]}});
    let initialized = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}});

    observations.notification(&catalog).unwrap();
    assert_eq!(
        observations.pending_response,
        Some((1, RequestKind::Initialize))
    );
    assert_eq!(
        observations.response_transaction(&initialized),
        Ok(Some(RequestKind::Initialize))
    );
    observations.pending_response = Some((2, RequestKind::New));
    observations.notification(&catalog).unwrap();
    observations.notification(&catalog).unwrap();
    assert_eq!(observations.response_transaction(&initialized), Ok(None));
    assert_eq!(observations.pending_response, Some((2, RequestKind::New)));
    assert_eq!(observations.completed_responses.len(), 1);
    assert_eq!(observations.received_notifications, 3);
    assert_eq!(observations.native_session, None);
}

#[test]
fn global_catalog_server_and_notification_budgets_preserve_pending_identity() {
    let mut observations = Observations {
        pending_response: Some((2, RequestKind::New)),
        ..Observations::default()
    };
    let too_many = json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated",
        "params":{"mcpServers":vec![json!({});65]}});
    assert_eq!(
        observations.notification(&too_many),
        Err("native_mcp_catalog_budget_exceeded")
    );
    assert_eq!(observations.received_notifications, 0);

    observations.received_notifications = MAX_FRAMES;
    let catalog = json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated",
        "params":{"mcpServers":[]}});
    assert_eq!(
        observations.notification(&catalog),
        Err("native_mcp_catalog_budget_exceeded")
    );
    assert_eq!(observations.received_notifications, MAX_FRAMES);
    assert_eq!(observations.pending_response, Some((2, RequestKind::New)));
    assert_eq!(observations.native_session, None);
}

#[test]
fn global_catalog_method_variants_do_not_expand_the_notification_contract() {
    for method in [
        "x.ai/mcp/servers_updated",
        "__x.ai/mcp/servers_updated",
        "_x.ai/mcp/servers_updated/extra",
        "_x.ai/mcp/tools_changed",
    ] {
        let mut observations = Observations::default();
        let frame = json!({"jsonrpc":"2.0","method":method,"params":{"mcpServers":[]}});
        assert_eq!(
            observations.notification(&frame),
            Err("native_unknown_notification_rejected")
        );
        assert_eq!(observations.received_notifications, 0);
    }
}

#[test]
fn global_catalog_diagnostic_projects_the_verified_method_without_catalog_body() {
    let frame = json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated",
        "params":{"mcpServers":[{"name":"OFFLINE_PRIVATE_SERVER",
            "env":[{"name":"OFFLINE_PRIVATE_ENV","value":"OFFLINE_PRIVATE_VALUE"}]}]}});
    let diagnostic = notification_diagnostic(&frame, Uuid::nil());

    assert_eq!(diagnostic["method"], "_x.ai/mcp/servers_updated");
    assert_eq!(diagnostic["global_catalog_closed_empty"], false);
    assert_eq!(diagnostic["method_summary"]["bytes"], 25);
    assert_eq!(
        diagnostic["method_summary"]["sha256"],
        "5ad0b9eadd8fadb2225bf5c00b21c1cf42ab869332b86333e722aa248a106580"
    );
    assert!(!diagnostic.to_string().contains("OFFLINE_PRIVATE_SERVER"));
    assert!(!diagnostic.to_string().contains("OFFLINE_PRIVATE_ENV"));
    assert!(!diagnostic.to_string().contains("OFFLINE_PRIVATE_VALUE"));
    assert_eq!(
        Observations::default().notification(&frame),
        Err("native_extra_mcp_sources_observed")
    );
}

#[test]
fn late_declared_response_and_exact_replay_do_not_complete_another_pending_rpc() {
    let mut observations = Observations {
        pending_response: Some((2, RequestKind::New)),
        ..Observations::default()
    };
    let late = json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":SESSION}});
    assert_eq!(
        observations.response_transaction(&late),
        Ok(Some(RequestKind::New))
    );
    response_body(&late, 2, RequestKind::New).unwrap();
    assert!(observations.pending_response.is_none());
    // 已失败调查的迟到回复只闭合事务，不将其会话或权限补成通过。
    assert!(observations.native_session.is_none());
    observations.pending_response = Some((3, RequestKind::Info));
    assert_eq!(observations.response_transaction(&late), Ok(None));
    assert_eq!(observations.pending_response, Some((3, RequestKind::Info)));
}

#[test]
fn future_string_and_unowned_response_ids_preserve_declared_pending_rpc() {
    for id in [
        json!(1),
        json!(3),
        json!("2"),
        json!("OFFLINE_UNKNOWN_ID"),
        Value::Null,
    ] {
        let mut observations = Observations {
            pending_response: Some((2, RequestKind::New)),
            ..Observations::default()
        };
        let frame = json!({"jsonrpc":"2.0","id":id,"result":{}});
        assert_eq!(
            observations.response_transaction(&frame),
            Err("native_response_uncorrelated")
        );
        assert_eq!(observations.pending_response, Some((2, RequestKind::New)));
        assert!(observations.completed_responses.is_empty());
    }
    assert_eq!(
        Observations::default().response_transaction(&json!({"jsonrpc":"2.0","id":2,"result":{}})),
        Err("native_response_uncorrelated")
    );
}

#[test]
fn conflicting_replay_cannot_replace_a_new_rpc_or_completed_fingerprint() {
    let mut observations = Observations {
        pending_response: Some((1, RequestKind::Initialize)),
        ..Observations::default()
    };
    let first = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}});
    observations.response_transaction(&first).unwrap();
    observations.pending_response = Some((2, RequestKind::New));
    let fingerprint = observations.completed_responses[&1].0;
    let conflict = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":99}});
    assert_eq!(
        observations.response_transaction(&conflict),
        Err("native_response_conflict")
    );
    assert_eq!(observations.pending_response, Some((2, RequestKind::New)));
    assert_eq!(observations.completed_responses[&1].0, fingerprint);
}

#[test]
fn malformed_late_response_cannot_consume_the_owned_request() {
    for frame in [
        json!({"jsonrpc":"1.0","id":2,"result":{}}),
        json!({"jsonrpc":"2.0","id":2,"result":[]}),
        json!({"jsonrpc":"2.0","id":2,"error":"OFFLINE_ERROR"}),
        json!({"jsonrpc":"2.0","id":2,"body":{}}),
        json!({"jsonrpc":"2.0","id":2,"result":{},"error":{}}),
    ] {
        let mut observations = Observations {
            pending_response: Some((2, RequestKind::New)),
            ..Observations::default()
        };
        assert_eq!(
            observations.response_transaction(&frame),
            Err("native_response_uncorrelated")
        );
        assert_eq!(observations.pending_response, Some((2, RequestKind::New)));
        assert!(observations.completed_responses.is_empty());
    }
}

#[test]
fn native_rpc_error_closes_only_its_declared_transaction_without_becoming_success() {
    let mut observations = Observations {
        pending_response: Some((2, RequestKind::New)),
        ..Observations::default()
    };
    let frame =
        json!({"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"OFFLINE_PRIVATE_ERROR"}});
    assert_eq!(
        observations.response_transaction(&frame),
        Ok(Some(RequestKind::New))
    );
    assert_eq!(response_body(&frame, 2, RequestKind::New), Err("rpc_error"));
    assert!(observations.native_session.is_none());
}

#[test]
fn primary_failure_survives_missing_exit_code_cleanup_timeout_and_drain_error() {
    let generation = Uuid::new_v4();
    let mut exit = receipt(
        "stop_requested",
        "macos_resource_coalition",
        &generation.to_string(),
    );
    exit.exit_code = None;
    let cleanup = super::cleanup_event(&exit).map(|_| ());
    assert_eq!(cleanup, Err("native_exit_code_missing"));
    assert_eq!(
        phase_outcome::<()>(
            Err("native_unknown_notification_rejected"),
            cleanup,
            Err("native_response_uncorrelated")
        ),
        Err("native_unknown_notification_rejected")
    );
    assert_eq!(
        phase_outcome::<()>(
            Err("native_unknown_notification_rejected"),
            Err("production_cleanup_timeout"),
            Ok(())
        ),
        Err("native_unknown_notification_rejected")
    );
    assert_eq!(
        phase_outcome(Ok("session"), Err("production_cleanup_timeout"), Ok(())),
        Err("production_cleanup_timeout")
    );
    assert_eq!(
        phase_outcome(Ok("session"), Ok(()), Err("native_response_uncorrelated")),
        Err("native_response_uncorrelated")
    );
    assert_eq!(phase_outcome(Ok("session"), Ok(()), Ok(())), Ok("session"));
}

#[test]
fn nonempty_history_or_different_native_identity_cannot_confirm_empty_recovery() {
    let body = json!({"sessionId":SESSION,"cwd":CWD,"turns":1,"turnIndex":1});
    assert_eq!(
        confirm_info(&body, SESSION, Path::new(CWD)),
        Err("empty_native_history_unconfirmed")
    );
    let body = json!({"sessionId":FOREIGN,"cwd":CWD,"turns":0,"turnIndex":0});
    assert_eq!(
        confirm_info(&body, SESSION, Path::new(CWD)),
        Err("empty_native_history_unconfirmed")
    );
}

#[test]
fn mcp_partial_catalog_reports_extra_origin_without_claiming_builtin_closure() {
    let body = json!({"servers":[{"source":"user","session":{"tools":[{"name":"ReadFile"},{"name":"UseTool"}]}}]});
    assert_eq!(mcp_counts(&body), Ok((2, 1)));
    let unknown = json!({"tools":[{"name":"ReadFile"}]});
    assert_eq!(mcp_counts(&unknown), Err("mcp_catalog_shape_unknown"));
}

#[test]
fn session_info_requires_the_native_flattened_data_fields() {
    // SessionInfoResponse 的 data 使用 serde(flatten)，不能按 Rust 字段名套 JSON 包装。
    let body = json!({"sessionId":SESSION,"cwd":CWD,"turns":0,"turnIndex":0});
    assert_eq!(confirm_info(&body, SESSION, Path::new(CWD)), Ok(()));
    let wrapped = json!({"sessionId":SESSION,"cwd":CWD,"data":{"turns":0,"turnIndex":0}});
    assert_eq!(
        confirm_info(&wrapped, SESSION, Path::new(CWD)),
        Err("empty_native_history_unconfirmed")
    );
}

#[test]
fn omitted_empty_mcp_tools_still_preserves_the_extra_source() {
    let body = json!({"servers":[{"source":"local","session":{"enabled":true,"status":"ready"}}]});
    assert_eq!(mcp_counts(&body), Ok((0, 1)));
    let malformed = json!({"servers":[{"source":"local","session":{"tools":null}}]});
    assert_eq!(mcp_counts(&malformed), Err("mcp_tools_shape_unknown"));
}

#[test]
fn reader_budget_rejects_bytes_before_decoding_or_archiving_a_frame() {
    let mut reader = WireReader::new(
        Cursor::new(b"{\"raw\":\"OFFLINE_BODY_CANARY\"}\n".to_vec()),
        0,
        MAX_FRAMES,
    );
    assert_eq!(block_on(reader.next()), Err("native_byte_budget_exceeded"));
    assert_eq!(reader.bytes, 0);
    assert_eq!(reader.frames, 0);
}

#[test]
fn unterminated_native_frame_cannot_be_treated_as_normal_eof() {
    let mut reader = WireReader::new(Cursor::new(b"{}".to_vec()), MAX_BYTES, MAX_FRAMES);
    assert_eq!(block_on(reader.next()), Err("native_unterminated_frame"));
}

#[test]
fn extra_native_frame_is_rejected_by_frame_budget() {
    let mut reader = WireReader::new(Cursor::new(b"{}\n{}\n".to_vec()), MAX_BYTES, 1);
    assert!(block_on(reader.next()).unwrap().is_some());
    assert_eq!(block_on(reader.next()), Err("native_frame_budget_exceeded"));
}

fn receipt(reason: &str, containment: &str, generation: &str) -> ExitReceipt {
    // 仅用于纯条件负例；真实夹具还必须调用生产 confirmed_exit 核对资源域证据。
    serde_json::from_value(json!({"version":1,"generation":generation,"cleanup_confirmed":true,
        "containment":containment,"exit_reason":reason,"exit_code":0,"manifest_sha256":hash(b"offline-manifest")})).unwrap()
}

#[test]
fn stop_ack_legacy_process_group_or_wrong_generation_never_proves_normal_stdio_cleanup() {
    let generation = Uuid::parse_str(SESSION).unwrap();
    assert_eq!(
        normal_receipt(
            &receipt("stop_requested", "macos_resource_coalition", SESSION),
            generation
        ),
        Err("normal_stdio_cleanup_unconfirmed")
    );
    assert_eq!(
        normal_receipt(
            &receipt("stdio_closed", "unix_process_group", SESSION),
            generation
        ),
        Err("normal_stdio_cleanup_unconfirmed")
    );
    assert_eq!(
        normal_receipt(
            &receipt("stdio_closed", "macos_resource_coalition", FOREIGN),
            generation
        ),
        Err("normal_stdio_cleanup_unconfirmed")
    );
}
