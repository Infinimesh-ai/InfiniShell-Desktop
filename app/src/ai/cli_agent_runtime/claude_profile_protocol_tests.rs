use super::*;

fn protocol(cwd: &std::path::Path) -> ClaudeProtocol {
    let profile = serde_json::from_value(json!({"version":1,"workingDirectory":cwd,
        "canonicalWorkingDirectory":std::fs::canonicalize(cwd).unwrap(),
        "executableSha256":"953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb",
        "denyRules":[],"sourceRules":[],"localTools":null}))
    .unwrap();
    ClaudeProtocol::new(SessionOptions {
        executable: cwd.join("claude"),
        cwd: cwd.to_owned(),
        state_dir: cwd.to_owned(),
        target: SessionTarget::New,
        generation: Uuid::from_u128(1),
        permission_policy: PermissionPolicy::ClaudeRestrictedFilesV1,
        permission_ceiling: None,
        claude_profile: Some(profile),
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    })
}

fn reply(protocol: &mut ClaudeProtocol, request: &Value, response: Value) -> Effects {
    protocol.receive(json!({"type":"control_response","response":{"subtype":"success","request_id":request["request_id"],"response":response}})).unwrap()
}

fn initialize() -> Value {
    json!({"pid":42,"session_state":"idle","current_permission_mode":"plan"})
}

fn finish_check(protocol: &mut ClaudeProtocol, request: &Value) -> Effects {
    let fixed = json!({"permissions":{"ask":["Edit"]},"sandbox":{"enabled":false}});
    let settings = reply(
        protocol,
        request,
        json!({"effective":fixed,"sources":[{"source":"flagSettings","settings":fixed}]}),
    );
    let captured: Value = serde_json::from_str(include_str!(
        "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-fixed-profile-control.json"
    ))
    .unwrap();
    let mut scope = captured["rules"].clone();
    scope["state"]["originalCwd"] = json!(protocol.options.cwd);
    let rules = reply(protocol, &settings.writes[0], scope);
    let hooks = reply(
        protocol,
        &rules.writes[0],
        json!({"hooks":[],"policy":{"policyHookCount":0},"bareMode":{"exitHint":"restart without --bare"}}),
    );
    let mcp = reply(protocol, &hooks.writes[0], json!({"mcpServers":[]}));
    reply(protocol, &mcp.writes[0], initialize())
}

fn ready(protocol: &mut ClaudeProtocol) -> Effects {
    let request = protocol.initialize();
    let effects = reply(protocol, &request, initialize());
    finish_check(protocol, &effects.writes[0])
}

fn action(id: u128, action: RuntimeAction) -> RuntimeCommand {
    RuntimeCommand {
        generation: Uuid::from_u128(1),
        message_id: Uuid::from_u128(id),
        action,
    }
}

#[test]
fn fixed_profile_must_be_verified_before_any_model_input_is_written() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    let ready = ready(&mut protocol);
    assert!(
        matches!(&ready.events[0], RuntimeEventKind::SessionReady { effective_permissions } if effective_permissions["fixedProfileVerified"] == true)
    );
    let pending = protocol.command(action(
        2,
        RuntimeAction::Submit {
            input: vec![InputContent::Text("next".into())],
        },
    ));
    assert_eq!(pending.writes[0]["type"], "control_request");
    assert_eq!(pending.writes[0]["request"]["subtype"], "get_settings");
    let submitted = finish_check(&mut protocol, &pending.writes[0]);
    assert_eq!(submitted.writes[0]["type"], "user");
    assert_eq!(submitted.writes[0]["message"]["content"], "next");
}

#[test]
fn failed_fixed_profile_check_never_releases_the_pending_input() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    let pending = protocol.command(action(
        2,
        RuntimeAction::Submit {
            input: vec![InputContent::Text("must stay unsent".into())],
        },
    ));
    let effects = protocol.receive(json!({"type":"control_response","response":{"subtype":"error","request_id":pending.writes[0]["request_id"],"error":"not available"}})).unwrap();
    assert!(effects.writes.is_empty());
    assert!(effects.events.is_empty());
    assert!(protocol.profile_error.is_some());
    assert!(protocol.turns.is_empty());
}

#[test]
fn fixed_profile_mode_change_does_not_become_a_new_ready_event() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    assert!(
        protocol
            .receive(json!({"type":"system","subtype":"status","permissionMode":"acceptEdits"}))
            .is_err()
    );
}

#[test]
fn saved_parent_profile_is_not_replaced_by_an_inherit_mode_label() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut options = protocol(&cwd).options;
    options.permission_policy = PermissionPolicy::Inherit;
    assert!(validate_options(&options).is_err());
    options.permission_policy = PermissionPolicy::ClaudeRestrictedFilesV1;
    assert!(validate_options(&options).is_ok());
}

#[test]
fn approval_from_an_old_tool_invocation_is_not_offered_for_the_current_turn() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    let pending = protocol.command(action(
        2,
        RuntimeAction::Submit {
            input: vec![InputContent::Text("write".into())],
        },
    ));
    finish_check(&mut protocol, &pending.writes[0]);
    protocol
        .receive(
            json!({"type":"command_lifecycle","command_uuid":Uuid::from_u128(2),"state":"started"}),
        )
        .unwrap();
    protocol.profile_tool_calls.insert(
        "old-tool".into(),
        (Uuid::from_u128(99), "Edit".into(), [0; 32]),
    );
    let effects = protocol.receive(json!({"type":"control_request","request_id":"approval-old","request":{"subtype":"can_use_tool","tool_name":"Edit","tool_use_id":"old-tool","input":{"file_path":cwd.join("safe.txt")}}})).unwrap();
    assert!(effects.events.is_empty());
    assert_eq!(effects.writes[0]["response"]["subtype"], "error");
    assert!(protocol.approvals.is_empty());
}

#[test]
fn resolved_allow_is_never_replayed_after_consumption_or_turn_completion() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    let pending = protocol.command(action(
        2,
        RuntimeAction::Submit {
            input: vec![InputContent::Text("write".into())],
        },
    ));
    finish_check(&mut protocol, &pending.writes[0]);
    protocol
        .receive(
            json!({"type":"command_lifecycle","command_uuid":Uuid::from_u128(2),"state":"started"}),
        )
        .unwrap();
    protocol.profile_tool_calls.insert(
        "current-tool".into(),
        (
            Uuid::from_u128(2),
            "Edit".into(),
            fingerprint(&json!({"file_path":cwd.join("safe.txt")})),
        ),
    );
    let request = json!({"type":"control_request","request_id":"allow-once","request":{"subtype":"can_use_tool","tool_name":"Edit","tool_use_id":"current-tool","input":{"file_path":cwd.join("safe.txt")}}});
    let requested = protocol.receive(request.clone()).unwrap();
    assert!(matches!(
        &requested.events[0],
        RuntimeEventKind::ApprovalRequested { .. }
    ));
    let pending = protocol.command(action(
        3,
        RuntimeAction::RespondApproval {
            approval_id: "allow-once".into(),
            decision: ApprovalDecision::AllowOnce,
        },
    ));
    let allowed = finish_check(&mut protocol, &pending.writes[0]);
    assert_eq!(
        allowed.writes[0]["response"]["response"]["behavior"],
        "allow"
    );
    let duplicate = protocol.receive(request.clone()).unwrap();
    assert_eq!(duplicate.writes[0]["response"]["subtype"], "error");
    let mut events = Vec::new();
    protocol.finish_turn(
        Uuid::from_u128(2),
        TurnOutcome::Cancelled,
        None,
        &mut events,
    );
    // 普通模式同样不能重放已经结束回合的允许响应。
    protocol.options.claude_profile = None;
    let stale = protocol.receive(request).unwrap();
    assert_eq!(stale.writes[0]["response"]["subtype"], "error");
}

#[test]
fn timed_out_profile_check_fails_without_releasing_the_input() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    protocol.command(action(
        2,
        RuntimeAction::Submit {
            input: vec![InputContent::Text("unsent".into())],
        },
    ));
    protocol.permission_observation_started = Some(Instant::now() - PERMISSION_OBSERVATION_TIMEOUT);
    let expired = protocol.expire_permission_observation();
    assert!(expired.writes.is_empty());
    assert!(expired.events.is_empty());
    assert!(protocol.profile_error.is_some());
    assert!(protocol.turns.is_empty());
}

fn start_turn(protocol: &mut ClaudeProtocol, id: u128) {
    let pending = protocol.command(action(
        id,
        RuntimeAction::Submit {
            input: vec![InputContent::Text("test command".into())],
        },
    ));
    finish_check(protocol, &pending.writes[0]);
    protocol
        .receive(
            json!({"type":"command_lifecycle","command_uuid":Uuid::from_u128(id),
            "state":"started","session_id":"profile-session"}),
        )
        .unwrap();
}

fn assistant(event_id: u128, message_id: &str, origin: Option<u128>, content: Value) -> Value {
    let mut message = json!({"type":"assistant","uuid":Uuid::from_u128(event_id),
        "session_id":"profile-session","parent_tool_use_id":null,
        "message":{"id":message_id,"role":"assistant","content":content}});
    if let Some(origin) = origin {
        message["user_message_uuid"] = json!(Uuid::from_u128(origin));
    }
    message
}

fn edit_request(cwd: &std::path::Path, request_id: &str, tool_id: &str) -> Value {
    json!({"type":"control_request","request_id":request_id,
        "request":{"subtype":"can_use_tool","tool_name":"Edit","tool_use_id":tool_id,
            "input":{"file_path":cwd.join("safe.txt"),"old_string":"before","new_string":"after"}}})
}

#[test]
fn later_unstamped_tool_invocation_uses_the_confirmed_native_command_origin() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    start_turn(&mut protocol, 2);
    protocol.receive(assistant(10, "msg-read", Some(2),
        json!([{"type":"tool_use","id":"read-first","name":"Read","input":{"file_path":cwd.join("safe.txt")}}]))).unwrap();
    let request = edit_request(&cwd, "edit-approval", "edit-later");
    protocol.receive(assistant(11, "msg-edit", None,
        json!([{"type":"tool_use","id":"edit-later","name":"Edit","input":request["request"]["input"]}]))).unwrap();

    let requested = protocol.receive(request).unwrap();
    assert!(matches!(
        &requested.events[0],
        RuntimeEventKind::ApprovalRequested { .. }
    ));
    let pending = protocol.command(action(
        3,
        RuntimeAction::RespondApproval {
            approval_id: "edit-approval".into(),
            decision: ApprovalDecision::AllowOnce,
        },
    ));
    let allowed = finish_check(&mut protocol, &pending.writes[0]);
    assert_eq!(
        allowed.writes[0]["response"]["response"]["behavior"],
        "allow"
    );
}

#[test]
fn running_command_does_not_supply_an_origin_for_unstamped_assistant_messages() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    start_turn(&mut protocol, 2);
    let request = edit_request(&cwd, "no-origin", "unproven-edit");
    let output = protocol.receive(assistant(10, "msg-unproven", None,
        json!([{"type":"text","text":"unproven"},{"type":"tool_use","id":"unproven-edit","name":"Edit","input":request["request"]["input"]}]))).unwrap();

    assert!(output.events.is_empty());
    let rejected = protocol.receive(request).unwrap();
    assert!(rejected.events.is_empty());
    assert_eq!(rejected.writes[0]["response"]["subtype"], "error");
}

#[test]
fn repeated_old_api_message_with_a_new_event_uuid_cannot_move_to_a_new_turn() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    start_turn(&mut protocol, 2);
    let request = edit_request(&cwd, "old-edit", "old-tool");
    protocol.receive(assistant(10, "msg-old", Some(2),
        json!([{"type":"tool_use","id":"old-tool","name":"Edit","input":request["request"]["input"]}]))).unwrap();
    protocol.finish_turn(
        Uuid::from_u128(2),
        TurnOutcome::Completed,
        None,
        &mut Vec::new(),
    );
    start_turn(&mut protocol, 3);
    protocol
        .receive(assistant(
            11,
            "msg-current",
            Some(3),
            json!([{"type":"text","text":"current"}]),
        ))
        .unwrap();

    let replay = protocol
        .receive(assistant(
            12,
            "msg-old",
            None,
            json!([{"type":"text","text":"stale"}]),
        ))
        .unwrap();
    assert!(replay.events.is_empty());
    let rejected = protocol.receive(request).unwrap();
    assert_eq!(rejected.writes[0]["response"]["subtype"], "error");
    assert!(protocol.approvals.is_empty());
}

#[test]
fn command_lifecycle_completion_closes_the_unstamped_assistant_origin() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    start_turn(&mut protocol, 2);
    protocol
        .receive(assistant(
            10,
            "msg-first",
            Some(2),
            json!([{"type":"text","text":"first"}]),
        ))
        .unwrap();
    protocol.receive(json!({"type":"command_lifecycle","command_uuid":Uuid::from_u128(2),"state":"completed"})).unwrap();

    let late = protocol
        .receive(assistant(
            11,
            "msg-late",
            None,
            json!([{"type":"text","text":"late"}]),
        ))
        .unwrap();
    assert!(late.events.is_empty());
    assert!(!protocol.turns[&Uuid::from_u128(2)].finished);
}

#[test]
fn cancellation_request_rejects_even_previously_correlated_tool_approval() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    start_turn(&mut protocol, 2);
    let request = edit_request(&cwd, "cancelled-edit", "cancelled-tool");
    protocol.receive(assistant(10, "msg-edit", Some(2),
        json!([{"type":"tool_use","id":"cancelled-tool","name":"Edit","input":request["request"]["input"]}]))).unwrap();
    protocol.command(action(
        3,
        RuntimeAction::Interrupt {
            turn_id: Uuid::from_u128(2).to_string(),
        },
    ));

    let rejected = protocol.receive(request).unwrap();
    assert!(rejected.events.is_empty());
    assert_eq!(rejected.writes[0]["response"]["subtype"], "error");
    let late = protocol
        .receive(assistant(
            11,
            "msg-late",
            None,
            json!([{"type":"text","text":"late"}]),
        ))
        .unwrap();
    assert!(late.events.is_empty());
}

#[test]
fn tool_approval_cannot_replace_the_correlated_invocation_input() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = std::fs::canonicalize(directory.path()).unwrap();
    let mut protocol = protocol(&cwd);
    ready(&mut protocol);
    start_turn(&mut protocol, 2);
    let mut request = edit_request(&cwd, "changed-input", "edit-tool");
    protocol.receive(assistant(10, "msg-edit", Some(2),
        json!([{"type":"tool_use","id":"edit-tool","name":"Edit","input":request["request"]["input"]}]))).unwrap();
    request["request"]["input"]["new_string"] = json!("replacement");

    let rejected = protocol.receive(request).unwrap();
    assert!(rejected.events.is_empty());
    assert_eq!(rejected.writes[0]["response"]["subtype"], "error");
    assert!(protocol.approvals.is_empty());
}
