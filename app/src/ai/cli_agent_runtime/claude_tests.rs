use futures::io::Cursor;

use super::*;

const AUTH_FAILURE: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-authentication-error.ndjson"
);
const MISSING_RESUME: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-missing-resume.ndjson"
);
const STREAMING_INTERRUPT: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-streaming-interrupt.ndjson"
);
const EARLY_INTERRUPT: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/claude-2.1.273-early-interrupt.ndjson"
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
        claude_profile: None,
        grok_profile: None,
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

fn aborted_result(turn_id: Uuid) -> Value {
    let mut result = capture(STREAMING_INTERRUPT)
        .into_iter()
        .find(|message| message["type"] == "result")
        .unwrap();
    result["session_id"] = json!("3ddff71c-4062-4198-a130-502e4c15684e");
    result["user_message_uuid"] = json!(turn_id.to_string());
    result["user_message_uuids"] = json!([turn_id.to_string()]);
    result
}

fn interrupt(protocol: &mut ClaudeProtocol, turn_id: Uuid) -> Value {
    protocol
        .command(command(
            Uuid::from_u128(12),
            RuntimeAction::Interrupt {
                turn_id: turn_id.to_string(),
            },
        ))
        .writes[0]
        .clone()
}

fn replay_native_cancellation(fixture: &str) -> Vec<RuntimeEventKind> {
    let mut protocol = ready_protocol();
    let mut request_id = None;
    let mut events = Vec::new();
    for record in fixture
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
    {
        let mut message = record["message"].clone();
        match record["direction"].as_str().unwrap() {
            "metadata" => {}
            "stdin" if message["type"] == "user" => {
                let turn_id = parse_uuid(&message, "uuid").unwrap();
                protocol.command(submit(
                    turn_id,
                    message["message"]["content"].as_str().unwrap(),
                ));
            }
            "stdin" => {
                let turn_id = protocol.active_turn.unwrap();
                request_id = Some(interrupt(&mut protocol, turn_id)["request_id"].clone());
            }
            "stdout" => {
                if message["type"] == "control_response" {
                    message["response"]["request_id"] = request_id.clone().unwrap();
                }
                events.extend(protocol.receive(message.clone()).unwrap().events);
                assert!(protocol.receive(message).unwrap().events.is_empty());
            }
            _ => panic!("取消夹具含有未知方向"),
        }
    }
    events
        .into_iter()
        .filter(|event| matches!(event, RuntimeEventKind::TurnFinished { .. }))
        .collect()
}

// 仅用于版本绑定回归的合成 init，不代表新版本已通过真实任务链验收。
fn version_init(version: Value) -> Value {
    json!({
        "type":"system", "subtype":"init", "claude_code_version":version,
        "permissionMode":"default", "tools":[], "mcp_servers":[],
        "session_id":"3ddff71c-4062-4198-a130-502e4c15684e"
    })
}

#[test]
fn supported_versions_require_matching_probe_and_init() {
    for &(output, version) in SUPPORTED_VERSIONS {
        let mut protocol = ready_protocol();
        protocol.bind_probed_version(true, output).unwrap();
        let effects = protocol.receive(version_init(json!(version))).unwrap();
        assert!(matches!(
            effects.events.as_slice(),
            [RuntimeEventKind::SessionReady { verified_cli_version: Some(actual), .. }]
                if actual == version
        ));
        assert_eq!(
            protocol.event(effects.events[0].clone()).native_session_id,
            Some("3ddff71c-4062-4198-a130-502e4c15684e".into())
        );
    }
}

#[test]
fn candidate_21280_and_production_keep_exact_version_pairing() {
    assert_eq!(
        supported_version("2.1.280"),
        cfg!(any(target_os = "macos", target_os = "linux", windows))
    );
    let mut protocol = ready_protocol();
    assert_eq!(
        protocol
            .bind_probed_version(true, TEST_CANDIDATE_OUTPUT)
            .is_ok(),
        cfg!(any(target_os = "macos", target_os = "linux", windows))
    );
    protocol.test_candidate_21280 = true;
    for output in [
        "2.1.280",
        "2.1.280-beta (Claude Code)",
        "2.1.281 (Claude Code)",
    ] {
        assert!(matches!(
            protocol.bind_probed_version(true, output),
            Err(RuntimeError::UnsupportedVersion(_))
        ));
    }
    assert!(matches!(
        protocol.bind_probed_version(false, TEST_CANDIDATE_OUTPUT),
        Err(RuntimeError::UnsupportedVersion(_))
    ));
    protocol
        .bind_probed_version(true, TEST_CANDIDATE_OUTPUT)
        .unwrap();
    assert!(matches!(
        protocol.receive(version_init(json!("2.1.278"))),
        Err(RuntimeError::UnsupportedVersion(_))
    ));
    let ready = protocol
        .receive(version_init(json!(TEST_CANDIDATE_VERSION)))
        .unwrap();
    assert!(matches!(
        ready.events.as_slice(),
        [RuntimeEventKind::SessionReady { verified_cli_version: Some(version), .. }]
            if version == TEST_CANDIDATE_VERSION
    ));
}

#[test]
fn test_candidate_21280_parent_proof_requires_gate_and_version_pair() {
    let Some(executable_digest) = test_candidate_executable_digest() else {
        return;
    };
    let mut protocol = ready_protocol();
    let profile: super::super::claude_profile::ClaudeRestrictedFilesV1 =
        serde_json::from_value(json!({
            "version":1,"workingDirectory":protocol.options.cwd,
            "canonicalWorkingDirectory":std::fs::canonicalize(&protocol.options.cwd).unwrap(),
            "executableSha256":executable_digest,
            "denyRules":[],"sourceRules":[],
            "localTools":{"allow_spawn":true,"allow_message":true},
        }))
        .unwrap();
    let digest = profile.digest();
    protocol.options.claude_profile = Some(profile.into());
    protocol.test_candidate_21280 = true;
    protocol.session_id = Some("3ddff71c-4062-4198-a130-502e4c15684e".into());
    protocol.paired_version = Some(TEST_CANDIDATE_VERSION);
    let RuntimeEventKind::SessionReady {
        effective_permissions,
        ..
    } = protocol.ready_event()
    else {
        panic!("候选必须产生 SessionReady");
    };
    assert_eq!(
        effective_permissions["claudeTestCandidate21280Proof"],
        json!({"runtimeGeneration":protocol.options.generation,
            "nativeSessionId":protocol.session_id.as_deref(),
            "profileSha256":digest})
    );
    protocol.paired_version = None;
    let RuntimeEventKind::SessionReady {
        effective_permissions,
        ..
    } = protocol.ready_event()
    else {
        panic!("候选必须产生 SessionReady");
    };
    assert!(
        effective_permissions
            .get("claudeTestCandidate21280Proof")
            .is_none()
    );
    protocol.paired_version = Some(TEST_CANDIDATE_VERSION);
    protocol.test_candidate_21280 = false;
    let RuntimeEventKind::SessionReady {
        effective_permissions,
        ..
    } = protocol.ready_event()
    else {
        panic!("候选必须产生 SessionReady");
    };
    assert!(
        effective_permissions
            .get("claudeTestCandidate21280Proof")
            .is_none()
    );
}

#[test]
fn test_candidate_coordinator_state_root_requires_exact_host_shape() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::write(state.join(TEST_CANDIDATE_MARKER), b"candidate").unwrap();
    assert_eq!(test_candidate_state_root(&state), Some(state.as_path()));
    let native = state
        .join("cli-agent-hosts")
        .join(Uuid::new_v4().to_string())
        .join("native");
    assert_eq!(test_candidate_state_root(&native), Some(state.as_path()));
    assert!(test_candidate_state_root(&state.join("other/native")).is_none());
    assert!(test_candidate_state_root(&state.join("cli-agent-hosts/not-a-uuid/native")).is_none());
    assert!(
        test_candidate_state_root(
            &state.join("wrong-hosts/00000000-0000-4000-8000-000000000001/native")
        )
        .is_none()
    );
}

#[test]
fn supported_versions_cannot_mix_probe_and_init() {
    for &(output, probed) in SUPPORTED_VERSIONS {
        for &(_, version) in SUPPORTED_VERSIONS {
            if version == probed {
                continue;
            }
            let mut protocol = ready_protocol();
            protocol.bind_probed_version(true, output).unwrap();
            assert!(matches!(
                protocol.receive(version_init(json!(version))),
                Err(RuntimeError::UnsupportedVersion(_))
            ));
        }
    }
}

#[test]
fn unknown_or_failed_probe_clears_previous_version_binding() {
    for (succeeded, output) in [
        (true, "2.1.279 (Claude Code)"),
        (true, "2.1.274 (Claude Code)"),
        (true, "2.1.278-beta (Claude Code)"),
        (true, "2.1.280-beta (Claude Code)"),
        (true, "2.1.281 (Claude Code)"),
        (true, "2.1.278"),
        (true, ""),
        (false, "2.1.273 (Claude Code)"),
        (false, "2.1.278 (Claude Code)"),
        (false, "2.1.280 (Claude Code)"),
    ] {
        let mut protocol = ready_protocol();
        protocol
            .bind_probed_version(true, "2.1.273 (Claude Code)")
            .unwrap();
        assert!(matches!(
            protocol.bind_probed_version(succeeded, output),
            Err(RuntimeError::UnsupportedVersion(_))
        ));
        assert!(matches!(
            protocol.receive(version_init(json!("2.1.273"))),
            Err(RuntimeError::UnsupportedVersion(_))
        ));
    }
}

#[test]
fn init_requires_successful_probe_even_for_known_version() {
    for &(_, version) in SUPPORTED_VERSIONS {
        let mut protocol = ready_protocol();
        // 清除旧夹具的默认绑定，复现生产进程尚未完成探测的状态。
        protocol.probed_version = None;
        assert!(matches!(
            protocol.receive(version_init(json!(version))),
            Err(RuntimeError::UnsupportedVersion(_))
        ));
    }
}

#[test]
fn init_rejects_unknown_or_malformed_version_after_known_probe() {
    for version in [
        json!("2.1.279"),
        json!("2.1.278-beta"),
        json!(278),
        Value::Null,
    ] {
        let mut protocol = ready_protocol();
        protocol
            .bind_probed_version(true, "2.1.278 (Claude Code)")
            .unwrap();
        assert!(matches!(
            protocol.receive(version_init(version)),
            Err(RuntimeError::UnsupportedVersion(_))
        ));
    }
    let mut protocol = ready_protocol();
    protocol
        .bind_probed_version(true, "2.1.278 (Claude Code)")
        .unwrap();
    let mut message = version_init(Value::Null);
    message
        .as_object_mut()
        .unwrap()
        .remove("claude_code_version");
    assert!(matches!(
        protocol.receive(message),
        Err(RuntimeError::UnsupportedVersion(_))
    ));
}

#[test]
fn captured_streaming_interrupt_finishes_once_as_cancelled() {
    let events = replay_native_cancellation(STREAMING_INTERRUPT);
    assert!(
        matches!(events.as_slice(), [RuntimeEventKind::TurnFinished {
        outcome: TurnOutcome::Cancelled, turn_id, ..
    }] if turn_id == "16fd486b-fd06-43a3-b10e-4d6a6eac59e1")
    );
}

#[test]
fn captured_early_interrupt_finishes_once_as_cancelled() {
    let events = replay_native_cancellation(EARLY_INTERRUPT);
    assert!(
        matches!(events.as_slice(), [RuntimeEventKind::TurnFinished {
        outcome: TurnOutcome::Cancelled, turn_id, ..
    }] if turn_id == "1af014f1-17ac-4700-bd42-4104abedefec")
    );
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
fn queued_ack_and_joined_input_keep_the_same_command_uuid() {
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
        matches!(&started.events[0], RuntimeEventKind::InputJoined { message_id, turn_id }
            if *message_id == queued && turn_id == &running.to_string())
    );
    assert_eq!(protocol.active_turn, Some(running));
    assert!(
        protocol
            .receive(lifecycle(queued, "queued"))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn joined_input_requires_a_native_result_for_the_complete_batch() {
    let (mut protocol, running) = running_protocol();
    let joined = Uuid::from_u128(11);
    protocol.command(submit(joined, "追加输入"));
    protocol.receive(lifecycle(joined, "started")).unwrap();
    let result = json!({"type":"result","subtype":"success","is_error":false,
        "user_message_uuid":joined,"user_message_uuids":[joined],"result":"JOINT_RESULT"});
    assert!(protocol.receive(result).is_err());
    assert!(!protocol.turns[&running].finished);
    assert!(!protocol.turns[&joined].finished);
}

#[test]
fn native_batch_result_finishes_joined_inputs_before_the_active_execution() {
    let (mut protocol, running) = running_protocol();
    let joined = Uuid::from_u128(11);
    protocol.command(submit(joined, "追加输入"));
    protocol.receive(lifecycle(joined, "started")).unwrap();
    let result = json!({"type":"result","subtype":"success","is_error":false,
        "user_message_uuid":joined,"user_message_uuids":[running,joined],"result":"JOINT_RESULT"});
    let effects = protocol.receive(result).unwrap();
    assert_eq!(
        effects.events,
        vec![
            RuntimeEventKind::TurnFinished {
                turn_id: joined.to_string(),
                outcome: TurnOutcome::Completed,
                output: "JOINT_RESULT".into()
            },
            RuntimeEventKind::TurnFinished {
                turn_id: running.to_string(),
                outcome: TurnOutcome::Completed,
                output: "JOINT_RESULT".into()
            },
        ]
    );
    assert_eq!(protocol.active_turn, None);
}

#[test]
fn enabled_native_result_evidence_records_the_complete_joined_batch() {
    let (mut protocol, running) = running_protocol();
    let joined = Uuid::from_u128(11);
    protocol.native_result_evidence = true;
    protocol.command(submit(joined, "追加输入"));
    protocol.receive(lifecycle(joined, "started")).unwrap();
    let result_id = Uuid::from_u128(12).to_string();
    let session_id = protocol.session_id.clone().unwrap();
    let result = json!({"type":"result","subtype":"success","is_error":false,
        "uuid":result_id,"session_id":session_id,"user_message_uuid":joined,
        "user_message_uuids":[running,joined],"result":"JOINT_RESULT"});

    let effects = protocol.receive(result).unwrap();

    let RuntimeEventKind::Progress { turn_id, message } = &effects.events[0] else {
        panic!("缺少原生结果批次证据");
    };
    assert_eq!(turn_id, &running.to_string());
    assert_eq!(
        serde_json::from_str::<Value>(message).unwrap(),
        json!({"kind":"native_result_correlated_v1","result_id":result_id,"subtype":"success",
            "primary_input_id":joined,"input_ids":[joined,running]})
    );
}

#[test]
fn native_result_evidence_marker_reaches_the_runtime_host_native_state() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join(NATIVE_RESULT_EVIDENCE_MARKER),
        b"enabled\n",
    )
    .unwrap();
    let native = root
        .path()
        .join("cli-agent-hosts")
        .join(Uuid::from_u128(1).to_string())
        .join("native");
    std::fs::create_dir_all(&native).unwrap();

    assert!(native_result_evidence_enabled(&native));
    assert!(!native_result_evidence_enabled(
        &root.path().join("unrelated").join("native")
    ));
}

fn joined_cancellation_frames() -> (ClaudeProtocol, Value, Value, Value) {
    let (mut protocol, running) = running_protocol();
    let joined = Uuid::from_u128(11);
    protocol.command(submit(joined, "合并输入"));
    protocol.receive(lifecycle(joined, "started")).unwrap();
    let request = interrupt(&mut protocol, running);
    let mut result = aborted_result(running);
    result["user_message_uuid"] = json!(joined);
    result["user_message_uuids"] = json!([running, joined]);
    let ack = control_response(request["request_id"].as_str().unwrap(), json!({}));
    let cancelled = lifecycle(running, "cancelled");
    (protocol, result, ack, cancelled)
}

// 这两种结果形态尚未被固定版本真实捕获；回归仅验证不能绕过取消证据门槛。
fn terminal_cancellation_frames(reason: &str) -> (ClaudeProtocol, Value, Value, Value) {
    let (protocol, mut result, ack, cancelled) = joined_cancellation_frames();
    result["terminal_reason"] = json!(reason);
    result["subtype"] = json!("success");
    result["is_error"] = json!(false);
    result["errors"] = Value::Null;
    (protocol, result, ack, cancelled)
}

#[test]
fn terminal_cancelled_joined_results_require_all_three_native_signals_in_any_order() {
    for reason in ["interrupted", "cancelled"] {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let (protocol, result, ack, cancelled) = terminal_cancellation_frames(reason);
            let frames = [result, ack, cancelled];
            assert_joined_cancellation_order(
                protocol,
                frames[order[0]].clone(),
                frames[order[1]].clone(),
                frames[order[2]].clone(),
            );
        }
    }
}

#[test]
fn terminal_cancelled_joined_results_missing_any_signal_fail_the_complete_batch() {
    for reason in ["interrupted", "cancelled"] {
        for missing in 0..3 {
            let (mut protocol, result, ack, cancelled) = terminal_cancellation_frames(reason);
            for (index, frame) in [result, ack, cancelled].into_iter().enumerate() {
                if index != missing {
                    assert!(finished_events(protocol.receive(frame).unwrap()).is_empty());
                }
            }
            assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
        }
    }
}

#[test]
fn terminal_cancelled_joined_result_without_authorized_interrupt_is_failed() {
    for reason in ["interrupted", "cancelled"] {
        let (mut protocol, running) = running_protocol();
        let joined = Uuid::from_u128(11);
        protocol.command(submit(joined, "追加输入"));
        protocol.receive(lifecycle(joined, "started")).unwrap();
        let mut result = aborted_result(running);
        result["terminal_reason"] = json!(reason);
        result["subtype"] = json!("success");
        result["is_error"] = json!(false);
        result["user_message_uuid"] = json!(joined);
        result["user_message_uuids"] = json!([running, joined]);
        assert_joined_batch_failed(protocol.receive(result).unwrap());
        assert!(
            protocol
                .receive(lifecycle(running, "cancelled"))
                .unwrap()
                .events
                .is_empty()
        );
    }
}

#[test]
fn terminal_cancelled_joined_input_lifecycle_cannot_confirm_execution_cancellation() {
    for reason in ["interrupted", "cancelled"] {
        let (mut protocol, result, ack, _) = terminal_cancellation_frames(reason);
        protocol.receive(ack).unwrap();
        assert!(finished_events(protocol.receive(result).unwrap()).is_empty());
        assert!(
            protocol
                .receive(lifecycle(Uuid::from_u128(11), "cancelled"))
                .unwrap()
                .events
                .is_empty()
        );
        assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
    }
}

#[test]
fn terminal_cancelled_joined_result_cannot_override_rejected_interrupt() {
    for reason in ["interrupted", "cancelled"] {
        let (mut protocol, result, ack, cancelled) = terminal_cancellation_frames(reason);
        assert!(finished_events(protocol.receive(result).unwrap()).is_empty());
        protocol.receive(cancelled).unwrap();
        let rejected = json!({"type":"control_response","response":{
            "subtype":"error","request_id":ack["response"]["request_id"],
            "error":"interrupt rejected"}});
        assert_joined_batch_failed(protocol.receive(rejected.clone()).unwrap());
        assert!(protocol.receive(rejected).unwrap().events.is_empty());
    }
}

#[test]
fn terminal_cancelled_joined_result_requires_each_input_uuid() {
    for reason in ["interrupted", "cancelled"] {
        let (mut protocol, mut result, ack, cancelled) = terminal_cancellation_frames(reason);
        result["user_message_uuid"] = json!(Uuid::from_u128(11));
        result["user_message_uuids"] = json!([Uuid::from_u128(11)]);
        assert!(protocol.receive(result).is_err());
        protocol.receive(ack).unwrap();
        assert!(finished_events(protocol.receive(cancelled).unwrap()).is_empty());
        assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
    }
}

#[test]
fn confirmed_terminal_cancelled_batch_keeps_the_native_result_output() {
    for reason in ["interrupted", "cancelled"] {
        let (mut protocol, mut result, ack, cancelled) = terminal_cancellation_frames(reason);
        result["result"] = json!("完整原生取消结果\nFull native cancelled result");
        assert!(finished_events(protocol.receive(result).unwrap()).is_empty());
        protocol.receive(ack).unwrap();
        let finished = finished_events(protocol.receive(cancelled).unwrap());
        assert_eq!(finished.len(), 2);
        assert!(finished.iter().all(|event| matches!(event,
            RuntimeEventKind::TurnFinished { outcome: TurnOutcome::Cancelled, output, .. }
            if output == "完整原生取消结果\nFull native cancelled result")));
    }
}

#[test]
fn terminal_cancelled_batch_with_unknown_or_missing_outcome_fields_is_unverified() {
    for reason in ["interrupted", "cancelled"] {
        for missing in ["subtype", "is_error"] {
            let (mut protocol, mut result, ack, cancelled) = terminal_cancellation_frames(reason);
            result[missing] = Value::Null;
            protocol.receive(ack).unwrap();
            protocol.receive(cancelled).unwrap();
            assert!(matches!(
                protocol.receive(result),
                Err(RuntimeError::Protocol(_))
            ));
            assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
        }
        let (mut protocol, mut result, _, _) = terminal_cancellation_frames(reason);
        result["subtype"] = json!("unrecognized_native_outcome");
        assert!(matches!(
            protocol.receive(result),
            Err(RuntimeError::Protocol(_))
        ));
    }
}

#[test]
fn terminal_cancelled_joined_old_result_cannot_finish_the_next_execution() {
    for reason in ["interrupted", "cancelled"] {
        let (protocol, result, ack, cancelled) = terminal_cancellation_frames(reason);
        let mut protocol = assert_joined_cancellation_order(
            protocol,
            result.clone(),
            ack.clone(),
            cancelled.clone(),
        );
        let next = Uuid::from_u128(13);
        protocol.command(submit(next, "继续会话"));
        protocol.receive(lifecycle(next, "started")).unwrap();
        for frame in [result, ack, cancelled] {
            assert!(protocol.receive(frame).unwrap().events.is_empty());
        }
        assert_eq!(protocol.active_turn, Some(next));
        assert!(!protocol.turns[&next].finished);
    }
}

#[test]
fn live_native_id_projection_keeps_cancellation_proofs_without_text_or_errors() {
    let id = Uuid::from_u128(10);
    let request_id = format!("infinishell-{}-3", Uuid::from_u128(1));
    let frame = json!({"type":"result","subtype":"error_during_execution",
        "uuid":Uuid::from_u128(20),"session_id":Uuid::from_u128(21),
        "user_message_uuid":id,"user_message_uuids":[id,Uuid::from_u128(11)],
        "terminal_reason":"aborted_streaming","is_error":true,
        "errors":["PRIVATE_TEXT_CANARY"],"result":"PRIVATE_RESULT_CANARY",
        "response":{"request_id":request_id,"subtype":"success","response":{"private":"PRIVATE_RESPONSE_CANARY"}}});
    let ids = live_native_protocol_ids(&frame);
    assert_eq!(ids["terminal_reason"], "aborted_streaming");
    assert_eq!(ids["is_error"], true);
    assert_eq!(ids["response_request_id"], request_id);
    assert_eq!(ids["response_subtype"], "success");
    assert_eq!(ids["user_message_uuids"], json!([id, Uuid::from_u128(11)]));
    assert!(!ids.to_string().contains("PRIVATE_"));
    let unknown = live_native_protocol_ids(&json!({"type":"PRIVATE_TYPE_CANARY",
        "subtype":"PRIVATE_SUBTYPE_CANARY","state":"PRIVATE_STATE_CANARY",
        "terminal_reason":"PRIVATE_REASON_CANARY",
        "request":{"subtype":"PRIVATE_REQUEST_CANARY"},
        "message":{"content":[{"type":"tool_use","id":"PRIVATE_TOOL_ID_CANARY",
            "name":"PRIVATE_TOOL_NAME_CANARY","input":{"private":"PRIVATE_TOOL_INPUT_CANARY"}}]},
        "response":{"request_id":"PRIVATE_ID_CANARY","subtype":"PRIVATE_STATUS_CANARY"}}));
    assert_eq!(unknown["terminal_reason"], "unknown");
    assert_eq!(unknown["response_subtype"], "unknown");
    assert!(
        unknown["response_request_id"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(!unknown.to_string().contains("PRIVATE_"));
}

fn finished_events(effects: Effects) -> Vec<RuntimeEventKind> {
    effects
        .events
        .into_iter()
        .filter(|event| matches!(event, RuntimeEventKind::TurnFinished { .. }))
        .collect()
}

fn assert_joined_cancellation_order(
    mut protocol: ClaudeProtocol,
    first: Value,
    second: Value,
    third: Value,
) -> ClaudeProtocol {
    assert!(finished_events(protocol.receive(first).unwrap()).is_empty());
    assert!(finished_events(protocol.receive(second).unwrap()).is_empty());
    let events = finished_events(protocol.receive(third).unwrap());
    assert_eq!(
        events,
        vec![
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(11).to_string(),
                outcome: TurnOutcome::Cancelled,
                output: String::new(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(10).to_string(),
                outcome: TurnOutcome::Cancelled,
                output: String::new(),
            },
        ]
    );
    assert_eq!(protocol.active_turn, None);
    protocol
}

fn expire_joined_cancellation(protocol: &mut ClaudeProtocol) -> Effects {
    protocol
        .turns
        .get_mut(&Uuid::from_u128(10))
        .unwrap()
        .cancellation
        .as_mut()
        .unwrap()
        .started_at = Instant::now() - REQUEST_TIMEOUT;
    protocol.expire_cancellations()
}

fn assert_joined_batch_failed(effects: Effects) {
    let events = finished_events(effects);
    assert!(matches!(events.as_slice(), [
        RuntimeEventKind::TurnFinished { turn_id: joined, outcome: TurnOutcome::Failed { .. }, .. },
        RuntimeEventKind::TurnFinished { turn_id: execution, outcome: TurnOutcome::Failed { .. }, .. },
    ] if joined == &Uuid::from_u128(11).to_string()
        && execution == &Uuid::from_u128(10).to_string()));
}

#[test]
fn joined_cancellation_waits_for_result_ack_then_lifecycle() {
    let (protocol, result, ack, cancelled) = joined_cancellation_frames();
    assert_joined_cancellation_order(protocol, result, ack, cancelled);
}

#[test]
fn joined_cancellation_waits_for_result_lifecycle_then_ack() {
    let (protocol, result, ack, cancelled) = joined_cancellation_frames();
    assert_joined_cancellation_order(protocol, result, cancelled, ack);
}

#[test]
fn joined_cancellation_waits_for_ack_result_then_lifecycle() {
    let (protocol, result, ack, cancelled) = joined_cancellation_frames();
    assert_joined_cancellation_order(protocol, ack, result, cancelled);
}

#[test]
fn joined_cancellation_waits_for_ack_lifecycle_then_result() {
    let (protocol, result, ack, cancelled) = joined_cancellation_frames();
    assert_joined_cancellation_order(protocol, ack, cancelled, result);
}

#[test]
fn joined_cancellation_waits_for_lifecycle_result_then_ack() {
    let (protocol, result, ack, cancelled) = joined_cancellation_frames();
    assert_joined_cancellation_order(protocol, cancelled, result, ack);
}

#[test]
fn joined_cancellation_waits_for_lifecycle_ack_then_result() {
    let (protocol, result, ack, cancelled) = joined_cancellation_frames();
    assert_joined_cancellation_order(protocol, cancelled, ack, result);
}

#[test]
fn joined_cancellation_missing_execution_ack_fails_the_whole_batch() {
    let (mut protocol, result, _, cancelled) = joined_cancellation_frames();
    assert!(protocol.receive(result).unwrap().events.is_empty());
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
}

#[test]
fn joined_cancellation_missing_execution_lifecycle_fails_the_whole_batch() {
    let (mut protocol, result, ack, _) = joined_cancellation_frames();
    protocol.receive(ack).unwrap();
    assert!(protocol.receive(result).unwrap().events.is_empty());
    assert!(
        protocol
            .receive(lifecycle(Uuid::from_u128(11), "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
}

#[test]
fn joined_cancellation_missing_native_result_fails_the_whole_batch() {
    let (mut protocol, _, ack, cancelled) = joined_cancellation_frames();
    protocol.receive(ack).unwrap();
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
}

#[test]
fn joined_cancellation_rejects_a_result_missing_a_batch_input() {
    let (mut protocol, mut result, ack, cancelled) = joined_cancellation_frames();
    result["user_message_uuid"] = json!(Uuid::from_u128(10));
    result["user_message_uuids"] = json!([Uuid::from_u128(10)]);
    assert!(
        matches!(protocol.receive(result), Err(RuntimeError::Protocol(message))
        if message.contains("complete joined input batch"))
    );
    protocol.receive(ack).unwrap();
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
}

#[test]
fn joined_cancellation_rejected_interrupt_fails_the_whole_batch() {
    let (mut protocol, result, ack, cancelled) = joined_cancellation_frames();
    assert!(protocol.receive(result).unwrap().events.is_empty());
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    let request_id = &ack["response"]["request_id"];
    let error = json!({"type":"control_response","response":{
        "subtype":"error","request_id":request_id,"error":"interrupt rejected"}});
    let effects = protocol.receive(error.clone()).unwrap();
    assert_joined_batch_failed(effects);
    assert!(protocol.receive(error).unwrap().events.is_empty());
}

#[test]
fn joined_aborted_batch_without_an_interrupt_remains_failed() {
    let (mut protocol, running) = running_protocol();
    let joined = Uuid::from_u128(11);
    protocol.command(submit(joined, "合并输入"));
    protocol.receive(lifecycle(joined, "started")).unwrap();
    let mut result = aborted_result(running);
    result["user_message_uuid"] = json!(joined);
    result["user_message_uuids"] = json!([running, joined]);
    assert_joined_batch_failed(protocol.receive(result).unwrap());
    assert!(
        protocol
            .receive(lifecycle(running, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn joined_api_error_after_deferred_abort_prevents_batch_cancellation() {
    let (mut protocol, result, ack, cancelled) = joined_cancellation_frames();
    protocol.receive(ack).unwrap();
    assert!(protocol.receive(result).unwrap().events.is_empty());
    let effects = protocol
        .receive(json!({"type":"assistant", "uuid":"joined-api-error",
        "user_message_uuid":Uuid::from_u128(10), "is_api_error_message":true,
        "message":{"content":[{"type":"text","text":"Not logged in"}]}}))
        .unwrap();
    assert_eq!(
        finished_events(effects),
        vec![
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(11).to_string(),
                outcome: TurnOutcome::Failed {
                    message: "Not logged in".into()
                },
                output: "Not logged in".into(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(10).to_string(),
                outcome: TurnOutcome::Failed {
                    message: "Not logged in".into()
                },
                output: "Not logged in".into(),
            },
        ]
    );
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
}

#[test]
fn joined_cancellation_waits_for_a_result_covering_inputs_added_after_the_abort() {
    let (mut protocol, result, ack, cancelled) = joined_cancellation_frames();
    assert!(protocol.receive(result.clone()).unwrap().events.is_empty());
    let later = Uuid::from_u128(13);
    protocol.command(submit(later, "晚到的合并输入"));
    protocol.receive(lifecycle(later, "started")).unwrap();
    protocol.receive(ack).unwrap();
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    assert!(protocol.turn_is_running(Uuid::from_u128(10)));
    let mut complete_result = result;
    complete_result["uuid"] = json!(Uuid::new_v4());
    complete_result["user_message_uuid"] = json!(later);
    complete_result["user_message_uuids"] =
        json!([Uuid::from_u128(10), Uuid::from_u128(11), later,]);
    assert_eq!(
        finished_events(protocol.receive(complete_result).unwrap()),
        vec![
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(11).to_string(),
                outcome: TurnOutcome::Cancelled,
                output: String::new(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: later.to_string(),
                outcome: TurnOutcome::Cancelled,
                output: String::new(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(10).to_string(),
                outcome: TurnOutcome::Cancelled,
                output: String::new(),
            },
        ]
    );
}

#[test]
fn joined_cancellation_duplicate_and_old_callbacks_cannot_finish_a_new_execution() {
    let (protocol, result, ack, cancelled) = joined_cancellation_frames();
    let mut protocol =
        assert_joined_cancellation_order(protocol, result.clone(), ack.clone(), cancelled.clone());
    assert!(protocol.receive(result.clone()).unwrap().events.is_empty());
    assert!(protocol.receive(ack).unwrap().events.is_empty());
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    assert!(protocol.expire_cancellations().events.is_empty());

    let next = Uuid::from_u128(13);
    protocol.command(submit(next, "新的执行"));
    protocol.receive(lifecycle(next, "started")).unwrap();
    let mut old_result = result;
    old_result["uuid"] = json!(Uuid::new_v4());
    assert!(protocol.receive(old_result).unwrap().events.is_empty());
    assert!(
        protocol
            .receive(lifecycle(Uuid::from_u128(11), "started"))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        protocol
            .receive(lifecycle(Uuid::from_u128(10), "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(protocol.active_turn, Some(next));
    assert!(protocol.turn_is_running(next));
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
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    let cancelled = protocol.receive(aborted_result(turn_id)).unwrap();
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
fn cancellation_waits_for_ack_after_both_native_terminal_signals() {
    let (mut protocol, turn_id) = running_protocol();
    let request = interrupt(&mut protocol, turn_id);
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        protocol
            .receive(aborted_result(turn_id))
            .unwrap()
            .events
            .is_empty()
    );
    let events = protocol
        .receive(control_response(
            request["request_id"].as_str().unwrap(),
            json!({}),
        ))
        .unwrap()
        .events;
    assert!(matches!(
        events.as_slice(),
        [
            RuntimeEventKind::MessageAccepted { .. },
            RuntimeEventKind::TurnFinished {
                outcome: TurnOutcome::Cancelled,
                ..
            }
        ]
    ));
}

#[test]
fn cancellation_without_native_lifecycle_fails_after_deadline() {
    let (mut protocol, turn_id) = running_protocol();
    let request = interrupt(&mut protocol, turn_id);
    protocol
        .receive(control_response(
            request["request_id"].as_str().unwrap(),
            json!({}),
        ))
        .unwrap();
    assert!(
        protocol
            .receive(aborted_result(turn_id))
            .unwrap()
            .events
            .is_empty()
    );
    protocol
        .turns
        .get_mut(&turn_id)
        .unwrap()
        .cancellation
        .as_mut()
        .unwrap()
        .started_at = Instant::now() - REQUEST_TIMEOUT;
    let effects = protocol.expire_cancellations();
    assert!(
        matches!(effects.events.as_slice(), [RuntimeEventKind::TurnFinished { outcome: TurnOutcome::Failed { message }, .. }] if message.contains("ede_diagnostic"))
    );
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(protocol.expire_cancellations().events.is_empty());
}

#[test]
fn cancellation_ack_without_native_result_fails_after_deadline() {
    let (mut protocol, turn_id) = running_protocol();
    let request = interrupt(&mut protocol, turn_id);
    protocol
        .receive(control_response(
            request["request_id"].as_str().unwrap(),
            json!({}),
        ))
        .unwrap();
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    protocol
        .turns
        .get_mut(&turn_id)
        .unwrap()
        .cancellation
        .as_mut()
        .unwrap()
        .started_at = Instant::now() - REQUEST_TIMEOUT;
    assert!(
        matches!(protocol.expire_cancellations().events.as_slice(), [RuntimeEventKind::TurnFinished { outcome: TurnOutcome::Failed { message }, .. }] if message.contains("completion is uncertain"))
    );
}

#[test]
fn aborted_result_without_requested_interrupt_remains_failed() {
    let (mut protocol, turn_id) = running_protocol();
    let effects = protocol.receive(aborted_result(turn_id)).unwrap();
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Failed { .. },
            ..
        }]
    ));
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn ordinary_execution_error_with_interrupt_ack_remains_failed() {
    let (mut protocol, turn_id) = running_protocol();
    let request = interrupt(&mut protocol, turn_id);
    protocol
        .receive(control_response(
            request["request_id"].as_str().unwrap(),
            json!({}),
        ))
        .unwrap();
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    let mut result = aborted_result(turn_id);
    result["terminal_reason"] = json!("api_error");
    result["errors"] = json!(["API authentication failed"]);
    let effects = protocol.receive(result).unwrap();
    assert!(
        matches!(effects.events.as_slice(), [RuntimeEventKind::TurnFinished {
        outcome: TurnOutcome::Failed { message }, ..
    }] if message == "API authentication failed")
    );
}

#[test]
fn rejected_interrupt_cannot_upgrade_deferred_abort_to_cancelled() {
    let (mut protocol, turn_id) = running_protocol();
    let request = interrupt(&mut protocol, turn_id);
    assert!(
        protocol
            .receive(aborted_result(turn_id))
            .unwrap()
            .events
            .is_empty()
    );
    let effects = protocol
        .receive(json!({"type":"control_response", "response":{
            "subtype":"error", "request_id":request["request_id"], "error":"interrupt rejected"
        }}))
        .unwrap();
    assert!(matches!(
        effects.events.as_slice(),
        [
            RuntimeEventKind::RequestFailed { .. },
            RuntimeEventKind::TurnFinished {
                outcome: TurnOutcome::Failed { .. },
                ..
            }
        ]
    ));
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn api_error_overrides_cancelled_lifecycle_before_aborted_result() {
    let (mut protocol, turn_id) = running_protocol();
    let request = interrupt(&mut protocol, turn_id);
    protocol
        .receive(control_response(
            request["request_id"].as_str().unwrap(),
            json!({}),
        ))
        .unwrap();
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    protocol.receive(json!({"type":"assistant", "uuid":"api-error-before-abort", "user_message_uuid":turn_id.to_string(),
        "is_api_error_message":true, "message":{"content":[{"type":"text","text":"Not logged in"}]}})).unwrap();
    let effects = protocol.receive(aborted_result(turn_id)).unwrap();
    assert!(
        matches!(effects.events.as_slice(), [RuntimeEventKind::TurnFinished { outcome: TurnOutcome::Failed { message }, .. }] if message == "Not logged in")
    );
}

#[test]
fn api_error_after_deferred_abort_prevents_late_cancellation() {
    let (mut protocol, turn_id) = running_protocol();
    let request = interrupt(&mut protocol, turn_id);
    protocol
        .receive(control_response(
            request["request_id"].as_str().unwrap(),
            json!({}),
        ))
        .unwrap();
    assert!(
        protocol
            .receive(aborted_result(turn_id))
            .unwrap()
            .events
            .is_empty()
    );
    let effects = protocol.receive(json!({"type":"assistant", "uuid":"api-error-after-abort", "user_message_uuid":turn_id.to_string(),
        "is_api_error_message":true, "message":{"content":[{"type":"text","text":"Not logged in"}]}})).unwrap();
    assert!(
        matches!(effects.events.last(), Some(RuntimeEventKind::TurnFinished { outcome: TurnOutcome::Failed { message }, .. }) if message == "Not logged in")
    );
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn unrelated_cancelled_command_cannot_confirm_interrupted_turn() {
    let (mut protocol, turn_id) = running_protocol();
    let request = interrupt(&mut protocol, turn_id);
    protocol
        .receive(control_response(
            request["request_id"].as_str().unwrap(),
            json!({}),
        ))
        .unwrap();
    assert!(
        protocol
            .receive(aborted_result(turn_id))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        protocol
            .receive(lifecycle(Uuid::from_u128(90), "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(protocol.turn_is_running(turn_id));
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
fn text_preserves_utf8_multiline_and_invalid_images_and_steering_are_rejected() {
    let (mut protocol, turn_id) = running_protocol();
    let text = "中文🧪\nEnglish\n$(touch should-not-run)";
    let input = protocol.command(submit(Uuid::from_u128(12), text));
    assert_eq!(input.writes[0]["message"]["content"], text);
    assert!(
        encode_input(
            vec![InputContent::LocalImage(
                std::env::temp_dir().join("image.png")
            )],
            None,
            &options().state_dir.join("local-cli-attachments")
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
    let mut protocol = ready_protocol();
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
        flush_effects(&mut protocol, &mut stdin, &events, effects).await,
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
            None,
            &options().state_dir.join("local-cli-attachments")
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
        ExpectedReplay::Text(
            native_user["message"]["content"]
                .as_str()
                .unwrap()
                .to_owned()
        )
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
    assert!(
        encode_input(
            multiple,
            protocol.skill_plugin.as_ref(),
            &protocol.options.state_dir.join("local-cli-attachments")
        )
        .is_err()
    );
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
            ..
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
        ..
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

#[path = "claude_image_tests.rs"]
mod image_tests;

// 固定实际二进制已核验该形态；这里是合成回归，不冒充历史协议捕获。
fn aborted_tools_result(turn_id: Uuid) -> Value {
    let mut result = aborted_result(turn_id);
    result["terminal_reason"] = json!("aborted_tools");
    result
}

fn tools_cancellation_frames() -> (ClaudeProtocol, Value, Value, Value) {
    let (protocol, mut result, ack, cancelled) = joined_cancellation_frames();
    result["terminal_reason"] = json!("aborted_tools");
    (protocol, result, ack, cancelled)
}

#[test]
fn tools_joined_cancellation_waits_for_result_ack_then_lifecycle() {
    let (protocol, result, ack, cancelled) = tools_cancellation_frames();
    assert_joined_cancellation_order(protocol, result, ack, cancelled);
}

#[test]
fn tools_joined_cancellation_waits_for_result_lifecycle_then_ack() {
    let (protocol, result, ack, cancelled) = tools_cancellation_frames();
    assert_joined_cancellation_order(protocol, result, cancelled, ack);
}

#[test]
fn tools_joined_cancellation_waits_for_ack_result_then_lifecycle() {
    let (protocol, result, ack, cancelled) = tools_cancellation_frames();
    assert_joined_cancellation_order(protocol, ack, result, cancelled);
}

#[test]
fn tools_joined_cancellation_waits_for_ack_lifecycle_then_result() {
    let (protocol, result, ack, cancelled) = tools_cancellation_frames();
    assert_joined_cancellation_order(protocol, ack, cancelled, result);
}

#[test]
fn tools_joined_cancellation_waits_for_lifecycle_result_then_ack() {
    let (protocol, result, ack, cancelled) = tools_cancellation_frames();
    assert_joined_cancellation_order(protocol, cancelled, result, ack);
}

#[test]
fn tools_joined_cancellation_waits_for_lifecycle_ack_then_result() {
    let (protocol, result, ack, cancelled) = tools_cancellation_frames();
    assert_joined_cancellation_order(protocol, cancelled, ack, result);
}

#[test]
fn tools_joined_cancellation_missing_execution_ack_fails_the_whole_batch() {
    let (mut protocol, result, _, cancelled) = tools_cancellation_frames();
    assert!(protocol.receive(result).unwrap().events.is_empty());
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
}

#[test]
fn tools_joined_cancellation_missing_execution_lifecycle_fails_the_whole_batch() {
    let (mut protocol, result, ack, _) = tools_cancellation_frames();
    protocol.receive(ack).unwrap();
    assert!(protocol.receive(result).unwrap().events.is_empty());
    assert!(
        protocol
            .receive(lifecycle(Uuid::from_u128(11), "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
}

#[test]
fn tools_joined_cancellation_missing_native_result_fails_the_whole_batch() {
    let (mut protocol, _, ack, cancelled) = tools_cancellation_frames();
    protocol.receive(ack).unwrap();
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
}

#[test]
fn tools_joined_cancellation_rejects_a_result_missing_a_batch_input() {
    let (mut protocol, mut result, ack, cancelled) = tools_cancellation_frames();
    result["user_message_uuid"] = json!(Uuid::from_u128(10));
    result["user_message_uuids"] = json!([Uuid::from_u128(10)]);
    assert!(
        matches!(protocol.receive(result), Err(RuntimeError::Protocol(message))
        if message.contains("complete joined input batch"))
    );
    protocol.receive(ack).unwrap();
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
}

#[test]
fn tools_joined_cancellation_rejected_interrupt_fails_the_whole_batch() {
    let (mut protocol, result, ack, cancelled) = tools_cancellation_frames();
    assert!(protocol.receive(result).unwrap().events.is_empty());
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    let request_id = &ack["response"]["request_id"];
    let error = json!({"type":"control_response","response":{
        "subtype":"error","request_id":request_id,"error":"interrupt rejected"}});
    let effects = protocol.receive(error.clone()).unwrap();
    assert_joined_batch_failed(effects);
    assert!(protocol.receive(error).unwrap().events.is_empty());
}

#[test]
fn tools_joined_aborted_batch_without_an_interrupt_remains_failed() {
    let (mut protocol, running) = running_protocol();
    let joined = Uuid::from_u128(11);
    protocol.command(submit(joined, "合并输入"));
    protocol.receive(lifecycle(joined, "started")).unwrap();
    let mut result = aborted_tools_result(running);
    result["user_message_uuid"] = json!(joined);
    result["user_message_uuids"] = json!([running, joined]);
    assert_joined_batch_failed(protocol.receive(result).unwrap());
    assert!(
        protocol
            .receive(lifecycle(running, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
}

#[test]
fn tools_joined_api_error_after_deferred_abort_prevents_batch_cancellation() {
    let (mut protocol, result, ack, cancelled) = tools_cancellation_frames();
    protocol.receive(ack).unwrap();
    assert!(protocol.receive(result).unwrap().events.is_empty());
    let effects = protocol
        .receive(json!({"type":"assistant", "uuid":"joined-api-error",
        "user_message_uuid":Uuid::from_u128(10), "is_api_error_message":true,
        "message":{"content":[{"type":"text","text":"Not logged in"}]}}))
        .unwrap();
    assert_eq!(
        finished_events(effects),
        vec![
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(11).to_string(),
                outcome: TurnOutcome::Failed {
                    message: "Not logged in".into()
                },
                output: "Not logged in".into(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(10).to_string(),
                outcome: TurnOutcome::Failed {
                    message: "Not logged in".into()
                },
                output: "Not logged in".into(),
            },
        ]
    );
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
}

#[test]
fn tools_joined_cancellation_duplicate_and_old_callbacks_cannot_finish_a_new_execution() {
    let (protocol, result, ack, cancelled) = tools_cancellation_frames();
    let mut protocol =
        assert_joined_cancellation_order(protocol, result.clone(), ack.clone(), cancelled.clone());
    assert!(protocol.receive(result.clone()).unwrap().events.is_empty());
    assert!(protocol.receive(ack).unwrap().events.is_empty());
    assert!(protocol.receive(cancelled).unwrap().events.is_empty());
    assert!(protocol.expire_cancellations().events.is_empty());

    let next = Uuid::from_u128(13);
    protocol.command(submit(next, "新的执行"));
    protocol.receive(lifecycle(next, "started")).unwrap();
    let mut old_result = result;
    old_result["uuid"] = json!(Uuid::new_v4());
    assert!(protocol.receive(old_result).unwrap().events.is_empty());
    assert!(
        protocol
            .receive(lifecycle(Uuid::from_u128(11), "started"))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        protocol
            .receive(lifecycle(Uuid::from_u128(10), "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(protocol.active_turn, Some(next));
    assert!(protocol.turn_is_running(next));
}

#[test]
fn tools_api_error_overrides_cancelled_lifecycle_before_aborted_tools_result() {
    let (mut protocol, turn_id) = running_protocol();
    let request = interrupt(&mut protocol, turn_id);
    protocol
        .receive(control_response(
            request["request_id"].as_str().unwrap(),
            json!({}),
        ))
        .unwrap();
    assert!(
        protocol
            .receive(lifecycle(turn_id, "cancelled"))
            .unwrap()
            .events
            .is_empty()
    );
    protocol.receive(json!({"type":"assistant", "uuid":"api-error-before-abort", "user_message_uuid":turn_id.to_string(),
        "is_api_error_message":true, "message":{"content":[{"type":"text","text":"Not logged in"}]}})).unwrap();
    let effects = protocol.receive(aborted_tools_result(turn_id)).unwrap();
    assert!(
        matches!(effects.events.as_slice(), [RuntimeEventKind::TurnFinished { outcome: TurnOutcome::Failed { message }, .. }] if message == "Not logged in")
    );
}

#[test]
fn tools_cancellation_old_generation_ack_cannot_confirm_current_execution() {
    let (mut protocol, result, mut ack, cancelled) = tools_cancellation_frames();
    ack["response"]["request_id"] = json!(format!("infinishell-{}-3", Uuid::from_u128(99)));
    assert!(finished_events(protocol.receive(result).unwrap()).is_empty());
    assert!(finished_events(protocol.receive(ack).unwrap()).is_empty());
    assert!(finished_events(protocol.receive(cancelled).unwrap()).is_empty());
    assert_joined_batch_failed(expire_joined_cancellation(&mut protocol));
}

#[test]
fn tools_cancellation_unknown_error_reason_remains_failed() {
    let (mut protocol, mut result, ack, cancelled) = tools_cancellation_frames();
    result["terminal_reason"] = json!("unknown_native_error");
    protocol.receive(ack).unwrap();
    protocol.receive(cancelled).unwrap();
    assert_joined_batch_failed(protocol.receive(result).unwrap());
}

#[test]
fn tools_cancellation_rejects_an_error_result_with_a_success_subtype() {
    let (mut protocol, mut result, ack, cancelled) = tools_cancellation_frames();
    result["subtype"] = json!("success");
    protocol.receive(ack).unwrap();
    protocol.receive(cancelled).unwrap();
    assert_joined_batch_failed(protocol.receive(result).unwrap());
}

#[test]
fn live_cancel_diagnostics_keep_unknown_reason_and_errors_without_body() {
    let (mut protocol, _) = running_protocol();
    let records = Arc::new(Mutex::new(Vec::new()));
    protocol.native_ids_for_live = Some(records.clone());
    let reason = "私密取消原因_CANARY";
    let errors = json!(["私密错误_CANARY", {"private":"私密字段_CANARY"}]);
    let frame = json!({"type":"result","terminal_reason":reason,"errors":errors});
    record_live_native_ids(&protocol, &frame, "stdout").unwrap();
    let records = records.lock().unwrap();
    let summary = &records[0]["cancel_diagnostics"];
    assert_eq!(records[0]["terminal_reason"], "unknown");
    assert_eq!(summary["terminal_reason"]["type"], "string");
    assert_eq!(summary["terminal_reason"]["bytes"], reason.len());
    assert_eq!(
        summary["terminal_reason"]["sha256"],
        format!("{:x}", Sha256::digest(reason.as_bytes()))
    );
    assert_eq!(summary["errors"]["type"], "array");
    assert_eq!(summary["errors"]["count"], 2);
    assert_eq!(
        summary["errors"]["sha256"],
        format!("{:x}", Sha256::digest(errors.to_string().as_bytes()))
    );
    assert!(!records[0].to_string().contains("CANARY"));
    assert!(
        live_native_protocol_ids(&frame)
            .get("cancel_diagnostics")
            .is_none()
    );
}

#[test]
fn live_cancel_diagnostics_distinguish_missing_null_and_non_string_values() {
    let missing = live_native_cancel_diagnostics(&json!({"type":"result"}));
    assert_eq!(
        missing["terminal_reason"],
        json!({"type":"missing","bytes":0,"sha256":null})
    );
    assert_eq!(missing["errors"]["count"], 0);
    let shaped = live_native_cancel_diagnostics(
        &json!({"type":"result","terminal_reason":null,"errors":false}),
    );
    assert_eq!(shaped["terminal_reason"]["type"], "null");
    assert_eq!(shaped["terminal_reason"]["bytes"], 4);
    assert_eq!(shaped["errors"]["type"], "boolean");
    assert_eq!(shaped["errors"]["bytes"], 5);
    assert_eq!(shaped["errors"]["count"], 0);
    assert_eq!(
        live_native_protocol_ids(&json!({"terminal_reason":"aborted_tools"}))["terminal_reason"],
        "aborted_tools"
    );
}

#[test]
fn initial_control_ready_has_no_paired_version() {
    let protocol = ready_protocol();
    assert!(matches!(
        protocol.ready_event(),
        RuntimeEventKind::SessionReady {
            verified_cli_version: None,
            ..
        }
    ));
}

#[test]
fn control_handshake_without_a_successful_probe_cannot_become_ready() {
    let mut protocol = ClaudeProtocol::new(options());
    protocol.probed_version = None;
    let initialize = protocol.initialize();
    let mut reply = capture(AUTH_FAILURE)
        .into_iter()
        .find(|message| message["type"] == "control_response")
        .unwrap();
    reply["response"]["request_id"] = initialize["request_id"].clone();
    assert!(protocol.receive(reply).is_err());
    assert!(!protocol.initialized);
    assert!(protocol.paired_version.is_none());
}

#[test]
fn paired_init_after_first_input_does_not_replay_that_input() {
    let mut protocol = ready_protocol();
    protocol
        .bind_probed_version(true, "2.1.278 (Claude Code)")
        .unwrap();
    let first = protocol.command(submit(Uuid::from_u128(72), "只提交一次"));
    assert_eq!(first.writes.len(), 1);
    assert_eq!(first.writes[0]["type"], "user");
    let ready = protocol.receive(version_init(json!("2.1.278"))).unwrap();
    assert!(ready.writes.is_empty());
    assert!(
        matches!(ready.events.as_slice(), [RuntimeEventKind::SessionReady {
        verified_cli_version: Some(version), ..
    }] if version == "2.1.278")
    );
    let repeated = protocol.receive(version_init(json!("2.1.278"))).unwrap();
    assert!(repeated.writes.is_empty());
    assert_eq!(protocol.turns.len(), 1);
    assert_eq!(protocol.messages.len(), 1);
}
