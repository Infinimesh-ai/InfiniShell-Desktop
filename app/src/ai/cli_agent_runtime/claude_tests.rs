use futures::io::Cursor;

use super::*;

const AUTH_FAILURE: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-authentication-error.ndjson"
);
const MISSING_RESUME: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-missing-resume.ndjson"
);

fn options() -> SessionOptions {
    SessionOptions {
        executable: std::env::temp_dir().join("claude-fixture"),
        cwd: std::env::temp_dir(),
        state_dir: std::env::temp_dir(),
        target: SessionTarget::New,
        generation: Uuid::from_u128(1),
        permission_policy: PermissionPolicy::Inherit,
        permission_ceiling: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    }
}

fn capture(capture: &str) -> Vec<Value> {
    capture
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|record| record["direction"] == "stdout")
        .map(|record| record["message"].clone())
        .collect()
}

fn command(id: Uuid, action: RuntimeAction) -> RuntimeCommand {
    RuntimeCommand {
        generation: Uuid::from_u128(1),
        message_id: id,
        action,
    }
}

fn submit(id: Uuid, text: &str) -> RuntimeCommand {
    command(
        id,
        RuntimeAction::Submit {
            input: vec![InputContent::Text(text.into())],
        },
    )
}

fn ready_protocol() -> ClaudeProtocol {
    let mut protocol = ClaudeProtocol::new(options());
    let initialize = protocol.initialize();
    let mut reply = capture(AUTH_FAILURE)
        .into_iter()
        .find(|message| message["type"] == "control_response")
        .unwrap();
    reply["response"]["request_id"] = initialize["request_id"].clone();
    let effects = protocol.receive(reply).unwrap();
    assert!(effects.events.is_empty());
    let effects = decline_permission_observation(&mut protocol, &effects);
    assert_eq!(effects.events.len(), 1);
    assert!(matches!(
        effects.events[0],
        RuntimeEventKind::SessionReady { .. }
    ));
    assert!(
        protocol
            .event(effects.events[0].clone())
            .native_session_id
            .is_none()
    );
    protocol
}

// 旧模型事件夹具没有权限查询；明确返回观察错误，仍验证普通 Inherit 路径。
fn decline_permission_observation(protocol: &mut ClaudeProtocol, effects: &Effects) -> Effects {
    protocol
        .receive(json!({"type":"control_response", "response":{
            "subtype":"error", "request_id":effects.writes[0]["request_id"],
            "error":"permission observation is unavailable in this fixture"
        }}))
        .unwrap()
}

fn lifecycle(id: Uuid, state: &str) -> Value {
    json!({"type":"command_lifecycle", "command_uuid":id.to_string(), "state":state,
        "uuid":Uuid::new_v4().to_string(), "session_id":"3ddff71c-4062-4198-a130-502e4c15684e"})
}

fn running_protocol() -> (ClaudeProtocol, Uuid) {
    let mut protocol = ready_protocol();
    let id = Uuid::from_u128(10);
    protocol.command(submit(id, "中文\nEnglish"));
    protocol.receive(lifecycle(id, "started")).unwrap();
    (protocol, id)
}

fn approval() -> Value {
    json!({"type":"control_request", "request_id":"permission-1", "request":{
        "subtype":"can_use_tool", "tool_name":"Bash", "input":{"command":"git status"},
        "tool_use_id":"tool-1", "permission_suggestions":[{"type":"setMode", "mode":"bypassPermissions"}]
    }})
}

#[test]
fn captured_authentication_error_never_becomes_success_or_late_cancellation() {
    let mut protocol = ready_protocol();
    let input = AUTH_FAILURE
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|record| record["direction"] == "stdin" && record["message"]["type"] == "user")
        .unwrap()["message"]
        .clone();
    let id = parse_uuid(&input, "uuid").unwrap();
    assert!(
        protocol
            .command(submit(id, input["message"]["content"].as_str().unwrap()))
            .events
            .is_empty()
    );
    let mut events = Vec::new();
    for message in capture(AUTH_FAILURE)
        .into_iter()
        .filter(|message| message["type"] != "control_response")
    {
        events.extend(protocol.receive(message.clone()).unwrap().events);
        assert!(protocol.receive(message).unwrap().events.is_empty());
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RuntimeEventKind::MessageAccepted { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RuntimeEventKind::TurnStarted { .. }))
            .count(),
        1
    );
    let outcomes = events
        .into_iter()
        .filter_map(|event| match event {
            RuntimeEventKind::TurnFinished { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(outcomes.len(), 1);
    assert!(
        matches!(&outcomes[0], TurnOutcome::Failed { message } if message.contains("Not logged in"))
    );
}

#[test]
fn captured_missing_resume_fails_before_ready_and_never_creates_new_session() {
    let id = "00000000-0000-4000-8000-000000000000";
    let mut options = options();
    options.target = SessionTarget::Resume {
        native_session_id: id.into(),
    };
    assert!(launch_arguments(&options).contains(&format!("--resume={id}")));
    let mut protocol = ClaudeProtocol::new(options);
    protocol.initialize();
    let result = capture(MISSING_RESUME)
        .into_iter()
        .find(|message| message["type"] == "result")
        .unwrap();
    assert!(
        matches!(protocol.receive(result), Err(RuntimeError::Protocol(message)) if message.contains("No conversation"))
    );
    assert!(!protocol.initialized);
}

#[test]
fn queued_ack_and_started_keep_the_same_command_uuid() {
    let (mut protocol, running) = running_protocol();
    let queued = Uuid::from_u128(11);
    let input = submit(queued, "下一轮\nNext turn 🧪");
    let effects = protocol.command(input.clone());
    assert_eq!(effects.writes[0]["uuid"], queued.to_string());
    assert!(effects.events.is_empty());
    assert!(protocol.command(input.clone()).writes.is_empty());
    let ack = protocol.receive(lifecycle(queued, "queued")).unwrap();
    assert!(
        matches!(&ack.events[0], RuntimeEventKind::MessageAccepted { message_id, turn_id: Some(turn_id) }
        if *message_id == queued && turn_id == &queued.to_string())
    );
    assert_eq!(protocol.active_turn, Some(running));
    let replay = protocol.command(input);
    assert!(replay.writes.is_empty());
    assert_eq!(replay.events, ack.events);
    let started = protocol.receive(lifecycle(queued, "started")).unwrap();
    assert!(
        matches!(&started.events[0], RuntimeEventKind::TurnStarted { turn_id } if turn_id == &queued.to_string())
    );
    assert!(
        protocol
            .receive(lifecycle(queued, "queued"))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn control_interrupt_ack_does_not_finish_the_turn() {
    let (mut protocol, turn_id) = running_protocol();
    let effects = protocol.command(command(
        Uuid::from_u128(12),
        RuntimeAction::Interrupt {
            turn_id: turn_id.to_string(),
        },
    ));
    let request_id = effects.writes[0]["request_id"].as_str().unwrap();
    let ack = protocol
        .receive(control_response(request_id, json!({"still_queued":[]})))
        .unwrap();
    assert!(
        ack.events
            .iter()
            .all(|event| matches!(event, RuntimeEventKind::MessageAccepted { .. }))
    );
    assert!(protocol.turn_is_running(turn_id));
    let cancelled = protocol.receive(lifecycle(turn_id, "cancelled")).unwrap();
    assert!(matches!(
        cancelled.events[0],
        RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Cancelled,
            ..
        }
    ));
    assert!(
        protocol
            .receive(lifecycle(turn_id, "started"))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn approval_is_single_use_and_does_not_install_permission_updates() {
    let (mut protocol, turn_id) = running_protocol();
    let request = approval();
    assert!(matches!(
        protocol.receive(request.clone()).unwrap().events[0],
        RuntimeEventKind::ApprovalRequested { .. }
    ));
    assert!(protocol.receive(request.clone()).unwrap().events.is_empty());
    let action = RuntimeAction::RespondApproval {
        approval_id: "permission-1".into(),
        decision: ApprovalDecision::AllowOnce,
    };
    let decision = protocol.command(command(Uuid::from_u128(12), action.clone()));
    assert!(
        decision
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEventKind::CommandDispatched { .. }))
    );
    assert!(
        !decision
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEventKind::MessageAccepted { .. }))
    );
    let native = &decision.writes[0];
    assert_eq!(native["response"]["request_id"], "permission-1");
    assert_eq!(native["response"]["response"]["behavior"], "allow");
    assert_eq!(
        native["response"]["response"]["updatedInput"],
        request["request"]["input"]
    );
    assert!(
        native["response"]["response"]
            .get("updatedPermissions")
            .is_none()
    );
    assert_eq!(
        protocol.receive(request.clone()).unwrap().writes,
        decision.writes
    );
    assert!(
        protocol
            .command(command(Uuid::from_u128(13), action))
            .writes
            .is_empty()
    );
    assert!(protocol.turn_is_running(turn_id));
    let mut changed = request;
    changed["request"]["input"]["command"] = json!("different command");
    assert_eq!(
        protocol.receive(changed).unwrap().writes[0]["response"]["subtype"],
        "error"
    );
}

#[test]
fn denial_and_native_permission_cancellation_are_distinct() {
    let (mut protocol, turn_id) = running_protocol();
    protocol.receive(approval()).unwrap();
    let denied = protocol.command(command(
        Uuid::from_u128(12),
        RuntimeAction::RespondApproval {
            approval_id: "permission-1".into(),
            decision: ApprovalDecision::DenyOnce,
        },
    ));
    assert_eq!(denied.writes[0]["response"]["response"]["behavior"], "deny");
    assert!(
        denied.writes[0]["response"]["response"]
            .get("interrupt")
            .is_none()
    );
    assert!(protocol.turn_is_running(turn_id));
    let mut pending = approval();
    pending["request_id"] = json!("permission-2");
    protocol.receive(pending).unwrap();
    let cancelled = json!({"type":"control_cancel_request", "request_id":"permission-2"});
    assert!(matches!(
        protocol.receive(cancelled.clone()).unwrap().events[0],
        RuntimeEventKind::ApprovalCancelled { .. }
    ));
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    let answer = protocol.command(command(
        Uuid::from_u128(13),
        RuntimeAction::RespondApproval {
            approval_id: "permission-2".into(),
            decision: ApprovalDecision::AllowOnce,
        },
    ));
    assert!(answer.writes.is_empty());
    assert!(matches!(
        answer.events[0],
        RuntimeEventKind::RequestFailed { .. }
    ));
}

#[test]
fn unknown_control_requests_receive_native_error_without_success() {
    let (mut protocol, _) = running_protocol();
    let reply = protocol.receive(json!({"type":"control_request", "request_id":"unknown", "request":{"subtype":"future_approval"}})).unwrap();
    assert_eq!(reply.writes[0]["response"]["request_id"], "unknown");
    assert_eq!(reply.writes[0]["response"]["subtype"], "error");
    assert!(reply.events.is_empty());
}

#[test]
fn stale_generation_conflicting_message_ids_and_wrong_resume_id_are_rejected() {
    let mut protocol = ready_protocol();
    let id = Uuid::from_u128(12);
    let mut stale = submit(id, "first");
    stale.generation = Uuid::from_u128(2);
    assert!(protocol.command(stale).writes.is_empty());
    assert_eq!(protocol.command(submit(id, "first")).writes.len(), 1);
    let conflicting = protocol.command(submit(id, "different"));
    assert!(conflicting.writes.is_empty());
    assert!(matches!(
        conflicting.events[0],
        RuntimeEventKind::RequestFailed { .. }
    ));
    protocol.options.target = SessionTarget::Resume {
        native_session_id: Uuid::from_u128(99).to_string(),
    };
    assert!(protocol.receive(lifecycle(id, "started")).is_err());
}

#[test]
fn terminal_error_wins_over_success_and_missing_result_correlation_is_not_guessed() {
    assert!(matches!(
        result_outcome(&json!({"subtype":"success", "is_error":true, "result":"API failure"}))
            .unwrap(),
        TurnOutcome::Failed { .. }
    ));
    assert!(result_outcome(&json!({"subtype":"success"})).is_err());
    let (mut protocol, _) = running_protocol();
    let result = json!({"type":"result", "subtype":"success", "is_error":false, "result":"answer"});
    assert!(protocol.receive(result).is_err());
}

#[test]
fn inherited_permissions_do_not_rewrite_cli_settings_or_pretend_to_be_a_sandbox() {
    let mut options = options();
    let arguments = launch_arguments(&options);
    for forbidden in [
        "--permission-mode",
        "--settings",
        "--setting-sources",
        "--dangerously-skip-permissions",
        "--allow-dangerously-skip-permissions",
    ] {
        assert!(
            !arguments
                .iter()
                .any(|argument| argument.starts_with(forbidden))
        );
    }
    assert!(arguments.iter().any(|argument| argument == "stdio"));
    options.permission_policy = PermissionPolicy::ReadOnly;
    assert!(matches!(
        connect(options),
        Err(RuntimeError::InvalidConfiguration(_))
    ));
}

#[test]
fn text_preserves_utf8_multiline_and_unverified_images_and_steering_are_rejected() {
    let (mut protocol, turn_id) = running_protocol();
    let text = "中文🧪\nEnglish\n$(touch should-not-run)";
    let input = protocol.command(submit(Uuid::from_u128(12), text));
    assert_eq!(input.writes[0]["message"]["content"], text);
    assert!(
        encode_input(
            vec![InputContent::LocalImage(
                std::env::temp_dir().join("image.png")
            )],
            None
        )
        .is_err()
    );
    let steer = protocol.command(command(
        Uuid::from_u128(13),
        RuntimeAction::Steer {
            expected_turn_id: turn_id.to_string(),
            input: vec![InputContent::Text("follow-up".into())],
        },
    ));
    assert!(steer.writes.is_empty());
    assert!(matches!(
        steer.events[0],
        RuntimeEventKind::RequestFailed { .. }
    ));
}

#[tokio::test]
async fn eof_does_not_complete_an_active_turn() {
    let (mut protocol, _) = running_protocol();
    let (_controller, commands, events, _receiver) = channels(protocol.options.generation);
    let mut stdin = Cursor::new(Vec::new());
    let mut stdout = Cursor::new(Vec::<u8>::new());
    assert!(matches!(
        run_transport(&mut protocol, &mut stdin, &mut stdout, commands, &events).await,
        Err(RuntimeError::Protocol(_))
    ));
}

#[tokio::test]
async fn full_event_queue_fails_instead_of_blocking_approvals() {
    let protocol = ready_protocol();
    let (events, _receiver) = mpsc::channel(1);
    events
        .try_send(protocol.event(RuntimeEventKind::Disconnected {
            reason: "fixture".into(),
        }))
        .unwrap();
    let mut stdin = Cursor::new(Vec::new());
    let effects = Effects {
        writes: Vec::new(),
        events: vec![RuntimeEventKind::TurnStarted {
            turn_id: "fixture".into(),
        }],
    };
    assert!(matches!(
        flush_effects(&protocol, &mut stdin, &events, effects).await,
        Err(RuntimeError::EventBackpressure)
    ));
}

#[test]
fn captured_sdk_mcp_bootstrap_preserves_host_approvals_and_opt_in() {
    let fixture = include_str!(
        "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-local-tools-registration.ndjson"
    );
    let mut settings = options();
    assert!(
        !launch_arguments(&settings)
            .iter()
            .any(|arg| arg.starts_with("--mcp-config"))
    );
    settings.local_tools = Some(local_tools::LocalToolPermissions::default());
    let arguments = launch_arguments(&settings);
    assert!(
        arguments
            .iter()
            .any(|arg| arg.starts_with("--mcp-config=") && arg.contains("infinishell-local-tasks"))
    );
    assert!(
        !arguments
            .iter()
            .any(|arg| arg.contains("bypassPermissions") || arg.contains("--settings"))
    );
    let mut protocol = ClaudeProtocol::new(settings);
    let mut methods = Vec::new();
    for message in capture(fixture)
        .into_iter()
        .filter(|message| message["type"] == "control_request")
    {
        methods.push(
            message["request"]["message"]["method"]
                .as_str()
                .unwrap()
                .to_string(),
        );
        let first = protocol.receive(message.clone()).unwrap();
        assert_eq!(
            first.writes[0]["response"]["request_id"],
            message["request_id"]
        );
        assert_eq!(first.writes[0]["response"]["subtype"], "success");
        assert!(first.events.is_empty());
        assert_eq!(protocol.receive(message).unwrap().writes, first.writes);
    }
    assert_eq!(
        methods,
        ["initialize", "notifications/initialized", "tools/list"]
    );
}

#[test]
fn sdk_tool_calls_require_active_turn_and_single_bound_response() {
    let (mut protocol, turn_id) = running_protocol();
    let request = json!({"type":"control_request","request_id":"mcp-call-1","request":{
        "subtype":"mcp_message","server_name":"infinishell-local-tasks","message":{
            "jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect_local_tasks","arguments":{}}}}});
    assert_eq!(
        protocol.receive(request.clone()).unwrap().writes[0]["response"]["subtype"],
        "error"
    );
    protocol.options.local_tools = Some(local_tools::LocalToolPermissions::default());
    assert!(matches!(
        protocol.receive(request.clone()).unwrap().events[0],
        RuntimeEventKind::LocalToolRequested { .. }
    ));
    assert!(protocol.receive(request.clone()).unwrap().events.is_empty());
    let action = RuntimeAction::RespondLocalTool {
        turn_id: turn_id.to_string(),
        call_id: "mcp-call-1".into(),
        result: Err("fixture failure".into()),
    };
    let effects = protocol.command(command(Uuid::from_u128(91), action.clone()));
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::CommandDispatched { .. }]
    ));
    assert_eq!(
        effects.writes[0]["response"]["response"]["mcp_response"]["id"],
        3
    );
    assert_eq!(
        effects.writes[0]["response"]["response"]["mcp_response"]["result"]["isError"],
        true
    );
    assert_eq!(
        protocol.receive(request.clone()).unwrap().writes,
        effects.writes
    );
    assert!(
        protocol
            .command(command(Uuid::from_u128(91), action))
            .writes
            .is_empty()
    );
    let mut changed = request.clone();
    changed["request"]["message"]["params"]["name"] = json!("run_agents");
    assert_eq!(
        protocol.receive(changed).unwrap().writes[0]["response"]["subtype"],
        "error"
    );
    let mut pending = request.clone();
    pending["request_id"] = json!("mcp-call-2");
    pending["request"]["message"]["id"] = json!(4);
    assert!(matches!(
        protocol.receive(pending.clone()).unwrap().events[0],
        RuntimeEventKind::LocalToolRequested { .. }
    ));
    let cancelled = protocol
        .receive(json!({"type":"control_cancel_request", "request_id":"mcp-call-2"}))
        .unwrap();
    assert!(
        matches!(&cancelled.events[..], [RuntimeEventKind::LocalToolCancelled { call_id, turn_id:cancelled_turn }] if call_id == "mcp-call-2" && cancelled_turn == &turn_id.to_string())
    );
    assert_eq!(
        protocol.receive(pending).unwrap().writes[0]["response"]["subtype"],
        "error"
    );
    assert!(matches!(
        protocol
            .command(command(
                Uuid::from_u128(93),
                RuntimeAction::RespondLocalTool {
                    turn_id: turn_id.to_string(),
                    call_id: "mcp-call-2".into(),
                    result: Ok(json!({}))
                }
            ))
            .events[0],
        RuntimeEventKind::RequestFailed { .. }
    ));
    assert!(
        encode_input(
            vec![InputContent::Skill {
                name: "review".into(),
                path: std::env::temp_dir().join("SKILL.md")
            }],
            None
        )
        .is_err()
    );
}

#[test]
fn captured_native_skill_arguments_replay_exactly_and_auth_failure_stays_failed() {
    let fixture = include_str!(
        "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-local-skill-command-args.ndjson"
    )
    .replace("infinishell-skill-probe:", "infinishell-local-skills:");
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("SKILL.md");
    std::fs::write(&path, "---\nname: review-local\ndescription: Test local review\n---\nOnly inspect the requested project.\n").unwrap();
    let selected = super::super::local_skills::SelectedLocalSkill {
        name: "review-local".into(),
        path: path.clone(),
    };
    let mut settings = options();
    settings.selected_skills = vec![selected];
    let mut protocol = ClaudeProtocol::new(settings.clone());
    protocol.skill_plugin = prepare_claude_skill_plugin(&settings.selected_skills).unwrap();
    let plugin_path = protocol
        .skill_plugin
        .as_ref()
        .unwrap()
        .plugin_directory()
        .to_path_buf();
    let initialize = protocol.initialize();
    let mut initialized = capture(&fixture)
        .into_iter()
        .find(|message| message["type"] == "control_response")
        .unwrap();
    initialized["response"]["request_id"] = initialize["request_id"].clone();
    let observation = protocol.receive(initialized).unwrap();
    decline_permission_observation(&mut protocol, &observation);
    let native_user = capture(&fixture)
        .into_iter()
        .find(|message| message["type"] == "user")
        .unwrap();
    let id = Uuid::parse_str(native_user["uuid"].as_str().unwrap()).unwrap();
    let arguments = "请保留中文和 English\n不要执行 `$()` <tag> \"quote\"";
    let input = vec![
        InputContent::Text(arguments.into()),
        InputContent::Skill {
            name: "review-local".into(),
            path: path.clone(),
        },
    ];
    let sent = protocol.command(command(
        id,
        RuntimeAction::Submit {
            input: input.clone(),
        },
    ));
    assert_eq!(
        sent.writes[0]["message"]["content"],
        format!("/infinishell-local-skills:review-local {arguments}")
    );
    assert_eq!(
        protocol.turns[&id].expected_replay,
        native_user["message"]["content"].as_str().unwrap()
    );
    let mut outcomes = Vec::new();
    for message in capture(&fixture)
        .into_iter()
        .filter(|message| message["type"] != "control_response")
    {
        outcomes.extend(protocol.receive(message).unwrap().events);
    }
    assert!(outcomes.iter().any(|event| matches!(event, RuntimeEventKind::MessageAccepted { message_id, .. } if *message_id == id)));
    assert!(outcomes.iter().any(|event| matches!(
        event,
        RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Failed { .. },
            ..
        }
    )));
    let mut multiple = input;
    multiple.push(InputContent::Skill {
        name: "review-local".into(),
        path,
    });
    assert!(encode_input(multiple, protocol.skill_plugin.as_ref()).is_err());
    drop(protocol);
    assert!(!plugin_path.exists());
}

#[test]
fn missing_native_skill_registration_never_marks_session_ready() {
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("SKILL.md");
    std::fs::write(
        &path,
        "---\nname: review-local\ndescription: Test review\n---\nReview locally.\n",
    )
    .unwrap();
    let mut protocol = ClaudeProtocol::new(options());
    protocol.skill_plugin =
        prepare_claude_skill_plugin(&[super::super::local_skills::SelectedLocalSkill {
            name: "review-local".into(),
            path,
        }])
        .unwrap();
    let initialize = protocol.initialize();
    assert!(protocol.receive(json!({"type":"control_response","response":{"subtype":"success","request_id":initialize["request_id"],"response":{"session_state":"idle","commands":[]}}})).is_err());
    assert!(!protocol.initialized);
}

#[test]
fn claude_native_mode_cannot_satisfy_a_filesystem_permission_ceiling() {
    let permissions = serde_json::from_value(json!({
        "parent_task_id":"parent","parent_generation":1,"parent_native_session_id":"native",
        "working_directory":std::env::temp_dir(),
        "permissions":{"approvalPolicy":"on-request","approvalsReviewer":"user",
            "sandbox":{"type":"readOnly","networkAccess":false}},
    }))
    .unwrap();
    let mut settings = options();
    settings.permission_ceiling = Some(permissions);
    let mut protocol = ClaudeProtocol::new(settings);
    let initialize = protocol.initialize();
    let mut reply = capture(AUTH_FAILURE)
        .into_iter()
        .find(|message| message["type"] == "control_response")
        .unwrap();
    reply["response"]["request_id"] = initialize["request_id"].clone();
    assert!(matches!(
        protocol.receive(reply),
        Err(RuntimeError::PermissionCeilingRejected { .. })
    ));
    assert!(!protocol.initialized);
    assert!(
        protocol
            .command(submit(Uuid::from_u128(99), "must not run"))
            .writes
            .is_empty()
    );
}

fn start_observation(protocol: &mut ClaudeProtocol) -> Effects {
    let initialize = protocol.initialize();
    protocol
        .receive(control_response(
            initialize["request_id"].as_str().unwrap(),
            json!({"pid":123, "current_permission_mode":"default", "session_state":"idle"}),
        ))
        .unwrap()
}

fn observation_settings() -> Value {
    json!({"effective":{"permissions":{"deny":["Bash"]}, "sandbox":{"enabled":false}},
        "sources":[{"source":"flagSettings", "settings":{"permissions":{"deny":["Bash"]}, "sandbox":{"enabled":false}}}]})
}

fn observation_rules() -> Value {
    json!({"state":{"rules":[{"behavior":"deny", "source":"flagSettings", "rule":"Bash", "editability":"readonly"}],
        "workspaceDirectories":[], "originalCwd":std::env::temp_dir(), "managedOnly":false}})
}

fn observation_mode_request(protocol: &mut ClaudeProtocol, first: &Effects) -> Effects {
    let settings = protocol
        .receive(control_response(
            first.writes[0]["request_id"].as_str().unwrap(),
            observation_settings(),
        ))
        .unwrap();
    protocol
        .receive(control_response(
            settings.writes[0]["request_id"].as_str().unwrap(),
            observation_rules(),
        ))
        .unwrap()
}

fn ready_observation(effects: &Effects) -> &Value {
    let [
        RuntimeEventKind::SessionReady {
            effective_permissions,
        },
    ] = effects.events.as_slice()
    else {
        panic!("expected one session-ready event");
    };
    &effective_permissions["permissionObservation"]
}

#[test]
fn permission_queries_finish_before_first_ready_without_authorizing_dispatch() {
    let mut protocol = ClaudeProtocol::new(options());
    let first = start_observation(&mut protocol);
    assert!(first.events.is_empty());
    assert_eq!(
        first.writes[0]["request"],
        json!({"subtype":"get_settings"})
    );
    assert!(!protocol.initialized);
    let settings = protocol
        .receive(control_response(
            first.writes[0]["request_id"].as_str().unwrap(),
            observation_settings(),
        ))
        .unwrap();
    assert!(settings.events.is_empty());
    assert_eq!(
        settings.writes[0]["request"],
        json!({"subtype":"list_permission_rules"})
    );
    let recheck = protocol
        .receive(control_response(
            settings.writes[0]["request_id"].as_str().unwrap(),
            observation_rules(),
        ))
        .unwrap();
    assert!(recheck.events.is_empty());
    assert_eq!(
        recheck.writes[0]["request"],
        json!({"subtype":"initialize"})
    );
    let ready = protocol
        .receive(control_response(
            recheck.writes[0]["request_id"].as_str().unwrap(),
            json!({"pid":123, "current_permission_mode":"default", "session_state":"idle"}),
        ))
        .unwrap();
    assert_eq!(ready_observation(&ready)["rejections"], json!([]));
    assert_eq!(ready_observation(&ready)["dispatchAuthorized"], false);
    assert_eq!(
        ready_observation(&ready)["connectionGeneration"],
        options().generation.to_string()
    );
    assert!(protocol.initialized);
    assert_eq!(
        protocol
            .command(submit(Uuid::from_u128(101), "普通对话仍可继续"))
            .writes[0]["type"],
        "user"
    );
}

#[test]
fn denied_permission_query_keeps_inherit_chat_usable_without_leaking_native_error() {
    let mut protocol = ClaudeProtocol::new(options());
    let first = start_observation(&mut protocol);
    let response = json!({"type":"control_response", "response":{"request_id":first.writes[0]["request_id"],
        "subtype":"error", "error":"private-settings-token"}});
    let ready = protocol.receive(response.clone()).unwrap();
    assert_eq!(
        ready_observation(&ready)["rejections"],
        json!(["native_request_failed"])
    );
    assert!(
        !ready_observation(&ready)
            .to_string()
            .contains("private-settings-token")
    );
    assert!(protocol.receive(response).unwrap().events.is_empty());
    assert!(protocol.pending.is_empty());
    assert_eq!(
        protocol
            .command(submit(Uuid::from_u128(102), "普通对话"))
            .writes[0]["type"],
        "user"
    );
}

#[test]
fn malformed_permission_response_does_not_permanently_block_inherit_chat() {
    let mut protocol = ClaudeProtocol::new(options());
    let first = start_observation(&mut protocol);
    let ready = protocol.receive(json!({"type":"control_response", "response":{"request_id":first.writes[0]["request_id"],
        "subtype":{"unexpected":"private-value"}}})).unwrap();
    assert_eq!(
        ready_observation(&ready)["rejections"],
        json!(["native_request_failed"])
    );
    assert_eq!(
        protocol
            .command(submit(Uuid::from_u128(103), "继续"))
            .writes[0]["type"],
        "user"
    );
}

#[test]
fn observation_total_deadline_releases_chat_and_old_reply_cannot_replace_result() {
    let mut protocol = ClaudeProtocol::new(options());
    let first = start_observation(&mut protocol);
    let recheck = observation_mode_request(&mut protocol, &first);
    protocol.permission_observation_started = Some(Instant::now() - PERMISSION_OBSERVATION_TIMEOUT);
    let ready = protocol.expire_permission_observation();
    assert_eq!(
        ready_observation(&ready)["rejections"],
        json!(["timed_out"])
    );
    assert!(protocol.expire_permission_observation().events.is_empty());
    assert!(!protocol.request_timed_out());
    let late = protocol
        .receive(control_response(
            recheck.writes[0]["request_id"].as_str().unwrap(),
            json!({"pid":123, "current_permission_mode":"default", "session_state":"idle"}),
        ))
        .unwrap();
    assert!(late.writes.is_empty());
    assert!(late.events.is_empty());
    assert_eq!(
        protocol
            .command(submit(Uuid::from_u128(104), "超时后继续"))
            .writes[0]["type"],
        "user"
    );
    assert_eq!(
        protocol.permission_observation.as_ref().unwrap().value()["rejections"],
        json!(["timed_out"])
    );
}

#[test]
fn previous_generation_reply_cannot_advance_current_permission_observation() {
    let mut protocol = ClaudeProtocol::new(options());
    let first = start_observation(&mut protocol);
    let old = format!("infinishell-{}-2", Uuid::from_u128(999));
    let effects = protocol
        .receive(control_response(&old, observation_settings()))
        .unwrap();
    assert!(effects.events.is_empty());
    assert!(effects.writes.is_empty());
    assert!(!protocol.initialized);
    assert!(
        protocol
            .pending
            .contains_key(first.writes[0]["request_id"].as_str().unwrap())
    );
    let next = protocol
        .receive(control_response(
            first.writes[0]["request_id"].as_str().unwrap(),
            observation_settings(),
        ))
        .unwrap();
    assert_eq!(
        next.writes[0]["request"]["subtype"],
        "list_permission_rules"
    );
}

#[test]
fn changed_mode_during_observation_rejects_ceiling_without_disabling_chat() {
    let mut protocol = ClaudeProtocol::new(options());
    let first = start_observation(&mut protocol);
    let recheck = observation_mode_request(&mut protocol, &first);
    let ready = protocol
        .receive(control_response(
            recheck.writes[0]["request_id"].as_str().unwrap(),
            json!({"pid":123, "current_permission_mode":"dontAsk", "session_state":"idle"}),
        ))
        .unwrap();
    assert_eq!(
        ready_observation(&ready)["rejections"],
        json!(["mode_changed"])
    );
    assert_eq!(ready_observation(&ready)["modeAfter"], "dontAsk");
    assert_eq!(
        protocol
            .command(submit(Uuid::from_u128(105), "保持原生当前权限"))
            .writes[0]["type"],
        "user"
    );
}

#[test]
fn live_settings_disagreement_is_persisted_as_unusable_observation() {
    let mut protocol = ClaudeProtocol::new(options());
    let first = start_observation(&mut protocol);
    let settings = protocol
        .receive(control_response(
            first.writes[0]["request_id"].as_str().unwrap(),
            observation_settings(),
        ))
        .unwrap();
    let mut changed_rules = observation_rules();
    changed_rules["state"]["rules"][0]["behavior"] = json!("allow");
    let recheck = protocol
        .receive(control_response(
            settings.writes[0]["request_id"].as_str().unwrap(),
            changed_rules,
        ))
        .unwrap();
    let ready = protocol
        .receive(control_response(
            recheck.writes[0]["request_id"].as_str().unwrap(),
            json!({"pid":123, "current_permission_mode":"default", "session_state":"idle"}),
        ))
        .unwrap();
    assert_eq!(
        ready_observation(&ready)["rejections"],
        json!(["settings_live_rules_mismatch"])
    );
    assert_eq!(
        protocol
            .command(submit(Uuid::from_u128(106), "不改变普通对话"))
            .writes[0]["type"],
        "user"
    );
}

#[test]
fn repeated_empty_initialize_keeps_registered_skill_and_mcp_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("SKILL.md");
    std::fs::write(&path, "---\nname: observation-skill\ndescription: Observation test\n---\nOnly review this fixture.\n").unwrap();
    let mut settings = options();
    settings.local_tools = Some(local_tools::LocalToolPermissions::default());
    settings.selected_skills = vec![super::super::local_skills::SelectedLocalSkill {
        name: "observation-skill".into(),
        path: path.clone(),
    }];
    let mut protocol = ClaudeProtocol::new(settings.clone());
    protocol.skill_plugin = prepare_claude_skill_plugin(&settings.selected_skills).unwrap();
    let initialize = protocol.initialize();
    let first = protocol
        .receive(control_response(
            initialize["request_id"].as_str().unwrap(),
            json!({"pid":123, "current_permission_mode":"default", "session_state":"idle",
            "commands":[{"name":"infinishell-local-skills:observation-skill"}]}),
        ))
        .unwrap();
    let mcp = json!({"type":"control_request", "request_id":"mcp-observation", "request":{
        "subtype":"mcp_message", "server_name":"infinishell-local-tasks", "message":{
            "jsonrpc":"2.0", "id":1, "method":"tools/list", "params":{}}}});
    let registered_mcp = protocol.receive(mcp.clone()).unwrap();
    assert_eq!(registered_mcp.writes[0]["response"]["subtype"], "success");
    let recheck = observation_mode_request(&mut protocol, &first);
    assert_eq!(
        recheck.writes[0]["request"],
        json!({"subtype":"initialize"})
    );
    // 重复空查询未再次发 commands/hooks/sdkMcpServers，也不要求新的空目录覆盖原注册。
    let ready = protocol.receive(control_response(recheck.writes[0]["request_id"].as_str().unwrap(),
        json!({"pid":123, "current_permission_mode":"default", "session_state":"idle", "commands":[]}))).unwrap();
    assert_eq!(ready_observation(&ready)["rejections"], json!([]));
    assert_eq!(protocol.receive(mcp).unwrap().writes, registered_mcp.writes);
    let submitted = protocol.command(command(
        Uuid::from_u128(107),
        RuntimeAction::Submit {
            input: vec![
                InputContent::Text("中文 arguments".into()),
                InputContent::Skill {
                    name: "observation-skill".into(),
                    path,
                },
            ],
        },
    ));
    assert_eq!(
        submitted.writes[0]["message"]["content"],
        "/infinishell-local-skills:observation-skill 中文 arguments"
    );
    assert_eq!(protocol.options.local_tools, settings.local_tools);
}

#[test]
fn missing_observation_request_id_degrades_without_persisting_unbound_data() {
    let mut protocol = ClaudeProtocol::new(options());
    start_observation(&mut protocol);
    let ready = protocol
        .receive(
            json!({"type":"control_response", "response":{"subtype":"success",
        "response":{"private":"must-not-persist"}}}),
        )
        .unwrap();
    assert_eq!(
        ready_observation(&ready)["rejections"],
        json!(["invalid_shape"])
    );
    assert!(
        !ready_observation(&ready)
            .to_string()
            .contains("must-not-persist")
    );
    assert_eq!(
        protocol
            .command(submit(Uuid::from_u128(108), "继续"))
            .writes[0]["type"],
        "user"
    );
}

#[test]
fn runtime_mode_notification_invalidates_observation_without_finishing_any_turn() {
    let mut protocol = ClaudeProtocol::new(options());
    let first = start_observation(&mut protocol);
    let recheck = observation_mode_request(&mut protocol, &first);
    protocol
        .receive(control_response(
            recheck.writes[0]["request_id"].as_str().unwrap(),
            json!({"pid":123, "current_permission_mode":"default", "session_state":"idle"}),
        ))
        .unwrap();
    let updated = protocol.receive(json!({"type":"system", "subtype":"status", "status":null,
        "permissionMode":"dontAsk", "uuid":"mode-notification", "session_id":"3ddff71c-4062-4198-a130-502e4c15684e"})).unwrap();
    assert_eq!(
        ready_observation(&updated)["rejections"],
        json!(["mode_changed"])
    );
    let RuntimeEventKind::SessionReady {
        effective_permissions,
    } = &updated.events[0]
    else {
        panic!("expected updated permission observation");
    };
    assert_eq!(effective_permissions["permissionMode"], "dontAsk");
    assert_eq!(
        effective_permissions["permissionObservation"]["modeAfter"],
        "default"
    );
}
