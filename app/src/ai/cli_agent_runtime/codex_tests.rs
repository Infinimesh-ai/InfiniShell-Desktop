use std::path::PathBuf;
use std::{env, fs};

use futures::io::Cursor;

use super::*;
use crate::ai::cli_agent_runtime::managed_process::{
    self, ExitReason,
    ExitReason::{HostDisconnected, NativeExit, StdioClosed, StopRequested},
};

const TWO_TURNS: &str =
    include_str!("../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-two-turns.ndjson");
const CANCEL_RESUME: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-steer-cancel-resume.ndjson"
);
const APPROVALS: &str = include_str!(
    "../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-approvals-race-resume.ndjson"
);
const HANDSHAKE: &str =
    include_str!("../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-handshake.ndjson");

fn options() -> SessionOptions {
    SessionOptions {
        executable: std::env::temp_dir().join("codex-fixture"),
        cwd: std::env::temp_dir(),
        state_dir: std::env::temp_dir(),
        target: SessionTarget::New,
        generation: Uuid::from_u128(1),
        permission_policy: PermissionPolicy::WorkspaceWrite,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    }
}

fn legacy_probed_protocol(options: SessionOptions) -> CodexProtocol {
    let mut protocol = CodexProtocol::new(options);
    protocol.record_version_probe("codex-cli 0.147.0").unwrap();
    protocol
}

fn captured_output(capture: &str, predicate: impl Fn(&Value) -> bool) -> Value {
    capture
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|record| record["direction"] == "stdout")
        .map(|record| record["message"].clone())
        .find(predicate)
        .expect("capture contains expected output")
}

fn ready_protocol(capture: &str) -> CodexProtocol {
    let mut protocol = legacy_probed_protocol(options());
    protocol.initialize();
    protocol
        .receive(captured_output(capture, |message| message["id"] == 1))
        .unwrap();
    protocol
        .receive(captured_output(capture, |message| {
            message["id"] == 2 && message.get("result").is_some()
        }))
        .unwrap();
    protocol
}

fn command(action: RuntimeAction, id: u128) -> RuntimeCommand {
    RuntimeCommand {
        generation: Uuid::from_u128(1),
        message_id: Uuid::from_u128(id),
        action,
    }
}

fn start_captured_turn(protocol: &mut CodexProtocol, capture: &str) -> String {
    protocol.command(command(
        RuntimeAction::Submit {
            input: vec![InputContent::Text("fixture prompt".into())],
        },
        10,
    ));
    let mut response = captured_output(capture, |message| {
        message.pointer("/result/turn/id").is_some()
    });
    // 捕获中的探测器请求编号可不同，按当前连接的实际编号关联同一原生响应。
    response["id"] = json!(3);
    let turn_id = response["result"]["turn"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    protocol.receive(response).unwrap();
    turn_id
}

#[test]
fn real_two_turn_capture_only_finishes_at_native_completed_events() {
    let mut protocol = legacy_probed_protocol(options());
    protocol.initialize();
    let mut outcomes = Vec::new();
    for line in TWO_TURNS.lines() {
        let record: Value = serde_json::from_str(line).unwrap();
        let message = &record["message"];
        if record["direction"] == "stdin" && message["method"] == "turn/start" {
            let id = message["id"].as_u64().unwrap();
            let text = message["params"]["input"][0]["text"]
                .as_str()
                .unwrap()
                .to_string();
            protocol.command(command(
                RuntimeAction::Submit {
                    input: vec![InputContent::Text(text)],
                },
                u128::from(id),
            ));
        } else if record["direction"] == "stdout" {
            let effects = protocol.receive(message.clone()).unwrap();
            outcomes.extend(effects.events.into_iter().filter_map(|event| match event {
                RuntimeEventKind::TurnFinished {
                    outcome, output, ..
                } => Some((outcome, output)),
                _ => None,
            }));
        }
    }
    assert_eq!(
        outcomes,
        vec![
            (TurnOutcome::Completed, "PROBE_ONE".into()),
            (TurnOutcome::Completed, "PROBE_TWO".into())
        ]
    );
}

#[test]
fn steering_waits_for_the_real_turn_started_notification() {
    let mut protocol = ready_protocol(CANCEL_RESUME);
    let turn_id = start_captured_turn(&mut protocol, CANCEL_RESUME);
    let too_early = protocol.command(command(
        RuntimeAction::Steer {
            expected_turn_id: turn_id.clone(),
            input: vec![InputContent::Text("新指令".into())],
        },
        11,
    ));
    assert!(too_early.writes.is_empty());
    assert!(matches!(
        too_early.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    protocol
        .receive(captured_output(CANCEL_RESUME, |message| {
            message["method"] == "turn/started"
        }))
        .unwrap();
    let steer = protocol.command(command(
        RuntimeAction::Steer {
            expected_turn_id: turn_id.clone(),
            input: vec![InputContent::Text("新指令".into())],
        },
        12,
    ));
    assert_eq!(steer.writes[0]["params"]["expectedTurnId"], turn_id);
    assert_eq!(
        steer.writes[0]["params"]["clientUserMessageId"],
        Uuid::from_u128(12).to_string()
    );
    let mut response = captured_output(CANCEL_RESUME, |message| {
        message["id"] == 6 && message.get("result").is_some()
    });
    response["id"] = steer.writes[0]["id"].clone();
    let acknowledgement = protocol.receive(response).unwrap();
    assert!(matches!(
        acknowledgement.events.as_slice(),
        [RuntimeEventKind::MessageAccepted { .. }]
    ));
}

#[test]
fn interrupt_acknowledgement_is_not_cancellation_completion() {
    let mut protocol = ready_protocol(CANCEL_RESUME);
    let turn_id = start_captured_turn(&mut protocol, CANCEL_RESUME);
    let started = captured_output(CANCEL_RESUME, |message| message["method"] == "turn/started");
    protocol.receive(started.clone()).unwrap();
    let interrupt = protocol.command(command(RuntimeAction::Interrupt { turn_id }, 13));
    let mut response = captured_output(CANCEL_RESUME, |message| {
        message["id"] == 7 && message.get("result").is_some()
    });
    response["id"] = interrupt.writes[0]["id"].clone();
    assert!(matches!(
        protocol.receive(response).unwrap().events.as_slice(),
        [RuntimeEventKind::MessageAccepted { .. }]
    ));
    let completed = captured_output(CANCEL_RESUME, |message| {
        message["method"] == "turn/completed"
    });
    assert!(matches!(
        protocol
            .receive(completed.clone())
            .unwrap()
            .events
            .as_slice(),
        [RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Cancelled,
            ..
        }]
    ));
    assert!(protocol.receive(completed).unwrap().events.is_empty());
    assert!(protocol.receive(started).unwrap().events.is_empty());
}

#[test]
fn approval_allow_once_preserves_numeric_request_id_and_rejects_changed_replay() {
    let mut protocol = ready_protocol(APPROVALS);
    start_captured_turn(&mut protocol, APPROVALS);
    protocol
        .receive(captured_output(APPROVALS, |message| {
            message["method"] == "turn/started"
        }))
        .unwrap();
    let request = captured_output(APPROVALS, |message| {
        message["method"] == "item/commandExecution/requestApproval"
    });
    let effects = protocol.receive(request.clone()).unwrap();
    let RuntimeEventKind::ApprovalRequested { approval_id, .. } = &effects.events[0] else {
        panic!("expected approval");
    };
    let reply_command = command(
        RuntimeAction::RespondApproval {
            approval_id: approval_id.clone(),
            decision: ApprovalDecision::AllowOnce,
        },
        14,
    );
    let reply = protocol.command(reply_command.clone());
    assert!(
        reply
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEventKind::CommandDispatched { .. }))
    );
    assert!(
        !reply
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEventKind::MessageAccepted { .. }))
    );
    assert_eq!(
        reply.writes,
        vec![json!({"id": 0, "result": {"decision": "accept"}})]
    );
    assert!(protocol.command(reply_command).writes.is_empty());
    let mut changed = request;
    changed["params"]["command"] = json!("different command");
    let rejected = protocol.receive(changed).unwrap();
    assert!(rejected.writes[0].get("error").is_some());
    assert!(rejected.writes[0].get("result").is_none());
}

#[test]
fn approval_deny_once_does_not_fail_the_whole_turn() {
    let mut protocol = ready_protocol(APPROVALS);
    start_captured_turn(&mut protocol, APPROVALS);
    protocol
        .receive(captured_output(APPROVALS, |message| {
            message["method"] == "turn/started"
        }))
        .unwrap();
    let effects = protocol
        .receive(captured_output(APPROVALS, |message| {
            message["method"] == "item/commandExecution/requestApproval"
        }))
        .unwrap();
    let RuntimeEventKind::ApprovalRequested { approval_id, .. } = &effects.events[0] else {
        panic!("expected approval");
    };
    let reply = protocol.command(command(
        RuntimeAction::RespondApproval {
            approval_id: approval_id.clone(),
            decision: ApprovalDecision::DenyOnce,
        },
        15,
    ));
    assert_eq!(
        reply.writes,
        vec![json!({"id": 0, "result": {"decision": "decline"}})]
    );
    assert!(protocol.active_turn.is_some());
    assert!(
        !reply
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEventKind::TurnFinished { .. }))
    );
}

#[test]
fn unknown_server_approval_requests_receive_errors_without_permission_grants() {
    let mut protocol = ready_protocol(APPROVALS);
    let rejected = protocol
        .receive(json!({"id": "approval-x", "method": "future/requestApproval", "params": {}}))
        .unwrap();
    assert_eq!(rejected.writes[0]["id"], "approval-x");
    assert_eq!(rejected.writes[0]["error"]["code"], -32601);
    assert!(rejected.events.is_empty());
}

#[test]
fn missing_resume_never_creates_a_replacement_thread() {
    let mut settings = options();
    settings.target = SessionTarget::Resume {
        native_session_id: "missing".into(),
    };
    let mut protocol = legacy_probed_protocol(settings);
    protocol.initialize();
    let open = protocol
        .receive(captured_output(HANDSHAKE, |message| message["id"] == 1))
        .unwrap();
    assert_eq!(open.writes[1]["method"], "thread/resume");
    let mut missing = captured_output(HANDSHAKE, |message| {
        message["id"] == 4 && message.get("error").is_some()
    });
    missing["id"] = json!(2);
    assert!(matches!(
        protocol.receive(missing),
        Err(RuntimeError::Protocol(_))
    ));
    assert!(protocol.session_id.is_none());
}

#[test]
fn inherited_permissions_are_not_replaced_with_bypass_or_sandbox_overrides() {
    let mut settings = options();
    settings.permission_policy = PermissionPolicy::Inherit;
    let mut protocol = legacy_probed_protocol(settings);
    protocol.initialize();
    let open = protocol
        .receive(captured_output(HANDSHAKE, |message| message["id"] == 1))
        .unwrap();
    assert!(open.writes[1]["params"].get("approvalPolicy").is_none());
    assert!(open.writes[1]["params"].get("sandbox").is_none());
}

#[test]
fn stale_generation_and_duplicate_messages_cannot_start_extra_turns() {
    let mut protocol = ready_protocol(TWO_TURNS);
    let submit = command(
        RuntimeAction::Submit {
            input: vec![InputContent::Text("中文任务".into())],
        },
        16,
    );
    let mut stale = submit.clone();
    stale.generation = Uuid::from_u128(2);
    assert!(protocol.command(stale).writes.is_empty());
    assert_eq!(protocol.command(submit.clone()).writes.len(), 1);
    assert!(protocol.command(submit).writes.is_empty());
}

#[test]
fn explicit_error_wins_over_a_completed_status() {
    assert_eq!(
        turn_outcome(&json!({"status": "completed", "error": {"message": "upstream failed"}}))
            .unwrap(),
        TurnOutcome::Failed {
            message: "upstream failed".into()
        }
    );
}

#[test]
fn eof_without_a_terminal_event_is_a_connection_failure() {
    let mut protocol = legacy_probed_protocol(options());
    let (controller, receiver, sender, events) = channels(protocol.options.generation);
    let result = futures::executor::block_on(run_transport(
        &mut protocol,
        &mut Cursor::new(Vec::new()),
        &mut Cursor::new(Vec::<u8>::new()),
        receiver,
        &sender,
    ));
    assert!(matches!(result, Err(RuntimeError::Protocol(_))));
    drop(controller);
    drop(events);
}

#[test]
fn a_full_event_queue_fails_without_blocking_approval_control() {
    let protocol = legacy_probed_protocol(options());
    let (sender, events) = mpsc::channel(1);
    sender
        .try_send(protocol.event(RuntimeEventKind::TurnStarted {
            turn_id: "turn-1".into(),
        }))
        .unwrap();
    let effects = Effects {
        writes: Vec::new(),
        events: vec![RuntimeEventKind::TurnStarted {
            turn_id: "turn-2".into(),
        }],
    };
    let result = futures::executor::block_on(flush_effects(
        &protocol,
        &mut Cursor::new(Vec::new()),
        &sender,
        effects,
    ));
    assert!(matches!(result, Err(RuntimeError::EventBackpressure)));
    drop(events);
}

#[test]
fn input_uses_native_blocks_without_shell_interpolation() {
    let input = encode_input(vec![InputContent::Text(
        "第一行\n$(touch should-not-run)".into(),
    )])
    .unwrap();
    assert_eq!(
        input,
        vec![json!({"type": "text", "text": "第一行\n$(touch should-not-run)"})]
    );
    assert!(
        encode_input(vec![InputContent::LocalImage(PathBuf::from(
            "relative.png"
        ))])
        .is_err()
    );
}

#[tokio::test]
#[ignore = "需要隔离无凭据运行器、真实 Codex 和同提交监督 worker"]
async fn live_codex_missing_session_is_not_replaced() {
    let root = PathBuf::from(
        env::var_os("INFINISHELL_CODEX_MISSING_ROOT")
            .expect("必须通过 missing-session 隔离运行器执行"),
    )
    .canonicalize()
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".infinishell-missing-session-probe")).unwrap(),
        "isolated unauthenticated missing-session verification\n"
    );
    let codex_home = PathBuf::from(env::var_os("CODEX_HOME").unwrap());
    assert_eq!(codex_home.canonicalize().unwrap(), root.join("codex"));
    assert_eq!(
        fs::read_to_string(codex_home.join("config.toml")).unwrap(),
        "cli_auth_credentials_store = \"file\"\n"
    );
    assert!(!codex_home.join("auth.json").exists());
    // 检查运行器的边界；不修改 Rust 测试进程的全局环境。
    assert!(env::vars_os().all(|(key, _)| {
        let key = key.to_string_lossy().to_ascii_uppercase();
        !["TOKEN", "API_KEY", "AUTH", "SECRET"]
            .iter()
            .any(|part| key.contains(*part))
    }));
    for name in ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
        let path = PathBuf::from(env::var_os(name).expect("隔离运行器必须设置配置路径"));
        assert!(path.canonicalize().unwrap().starts_with(&root));
    }
    let executable = PathBuf::from(
        env::var_os("INFINISHELL_CODEX_LIVE_EXECUTABLE").expect("必须指定真实 Codex 可执行文件"),
    );
    assert!(executable.is_absolute() && executable.is_file());
    let directory = tempfile::Builder::new()
        .prefix("missing-session-")
        .tempdir_in(&root)
        .unwrap();
    let generation = Uuid::new_v4();
    let native_session_id = Uuid::new_v4().to_string();
    let expected_error = format!("no rollout found for thread id {native_session_id}");
    let mut settings = options();
    settings.executable = executable;
    settings.cwd = directory.path().canonicalize().unwrap();
    settings.state_dir = settings.cwd.join("state");
    fs::create_dir(&settings.state_dir).unwrap();
    settings.generation = generation;
    settings.permission_policy = PermissionPolicy::Inherit;
    settings.target = SessionTarget::Resume {
        native_session_id: native_session_id.clone(),
    };
    let state_dir = settings.state_dir.clone();
    let RuntimeConnection {
        controller,
        mut events,
        task,
    } = connect(settings).unwrap();
    // 保留 controller，但不发送 Submit、Steer 或任何模型输入。
    let result = tokio::time::timeout(Duration::from_secs(60), task).await;
    let mut observed_events = Vec::new();
    let event_channel_closed = loop {
        match events.try_recv() {
            Ok(event) => observed_events.push(event),
            Err(mpsc::error::TryRecvError::Disconnected) => break true,
            Err(mpsc::error::TryRecvError::Empty) => break false,
        }
    };
    let confirmed_receipt = managed_process::confirmed_exit(&state_dir, generation);
    let artifact = PathBuf::from(
        env::var_os("INFINISHELL_CODEX_LIVE_ARTIFACT").expect("必须指定验收证据文件"),
    );
    // 先保留结果和回执，再执行断言；失败不能只剩 libtest 的退出码。
    let observed = json!({
        "event": "missing_session_probe_observed", "passed": false,
        "native_session_id": native_session_id, "generation": generation,
        "runtime_result": format!("{result:?}"), "runtime_events": observed_events,
        "event_channel_closed": event_channel_closed,
        "exit_receipt": confirmed_receipt.as_ref().ok().and_then(Option::as_ref),
        "receipt_error": confirmed_receipt.as_ref().err().map(ToString::to_string),
        "credentials_provided": false, "model_commands_sent": 0,
        "native_exit_code_origin_verified": false,
    });
    fs::write(&artifact, format!("{observed}\n")).unwrap();
    let result = result.expect("真实缺失会话恢复及监督退出超时");
    let Err(RuntimeError::Protocol(native_error)) = result else {
        panic!("必须收到原生缺失会话错误，而不是成功或其他启动失败：{result:?}");
    };
    assert_eq!(native_error, expected_error);
    assert_eq!(observed_events.len(), 1, "必须仅收到连接终止事件");
    let disconnected = &observed_events[0];
    assert_eq!(disconnected.generation, generation);
    assert!(disconnected.native_session_id.is_none());
    assert_eq!(
        disconnected.kind,
        RuntimeEventKind::Disconnected {
            reason: format!("CLI protocol error: {expected_error}"),
        }
    );
    assert!(event_channel_closed);
    drop(controller);
    let receipt = confirmed_receipt
        .expect("必须核对真实监督退出证明")
        .expect("必须存在真实监督退出回执");
    assert_eq!(receipt.generation, generation);
    assert!(receipt.cleanup_confirmed);
    assert!(
        missing_session_exit_is_expected(
            receipt.exit_code,
            receipt.exit_reason,
            &receipt.containment,
            cfg!(windows),
        ),
        "缺失会话后的退出不符合受控收尾契约：{receipt:?}"
    );
    assert_ne!(receipt.containment, "not_started");
    assert!(!codex_home.join("auth.json").exists());
    let evidence = json!({
        "event": "missing_session_probe_finished", "passed": true,
        "native_session_id": native_session_id, "native_error": native_error,
        "generation": generation, "credentials_provided": false,
        "model_commands_sent": 0, "runtime_event_count": 1,
        "exit_receipt": receipt,
        "native_exit_code_origin_verified": false,
    });
    fs::write(artifact, format!("{observed}\n{evidence}\n")).unwrap();
}

fn missing_session_exit_is_expected(
    exit_code: Option<i32>,
    reason: ExitReason,
    containment: &str,
    windows: bool,
) -> bool {
    match reason {
        ExitReason::HostDisconnected => false,
        ExitReason::NativeExit => exit_code == Some(0),
        ExitReason::StopRequested | ExitReason::StdioClosed => {
            // Windows Job 清理指定终止码 1；回执未区分此码与自然退出，不能声称原生成功。
            exit_code == Some(0)
                || (windows && containment == "windows_job" && exit_code == Some(1))
        }
    }
}

#[test]
fn missing_session_exit_allows_only_known_windows_job_cleanup_status() {
    // 预期值直接列出，避免在测试里复制退出判定表达式。
    let cases = [
        (Some(0), NativeExit, "windows_job", true, true),
        (Some(1), NativeExit, "windows_job", true, false),
        (Some(0), StopRequested, "windows_job", true, true),
        (Some(1), StopRequested, "windows_job", true, true),
        (Some(1), StdioClosed, "windows_job", true, true),
        (Some(1), StopRequested, "windows_job", false, false),
        (Some(1), StopRequested, "linux_subtree", true, false),
        (
            Some(1),
            StdioClosed,
            "macos_resource_coalition",
            false,
            false,
        ),
        (
            Some(0),
            StdioClosed,
            "macos_resource_coalition",
            false,
            true,
        ),
        (None, StopRequested, "windows_job", true, false),
        (Some(-1), StopRequested, "windows_job", true, false),
        (Some(37), StdioClosed, "windows_job", true, false),
        (Some(0), HostDisconnected, "windows_job", true, false),
        (Some(1), HostDisconnected, "windows_job", true, false),
    ];
    for (exit_code, reason, containment, windows, expected) in cases {
        assert_eq!(
            missing_session_exit_is_expected(exit_code, reason, containment, windows),
            expected,
            "{exit_code:?}, {reason:?}, {containment}, windows={windows}",
        );
    }
}

#[test]
fn captured_dynamic_tool_registration_is_opt_in_and_never_added_to_resume() {
    let fixture = include_str!(
        "../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-local-tools-registration.ndjson"
    );
    let mut settings = options();
    settings.permission_policy = PermissionPolicy::Inherit;
    settings.local_tools = Some(local_tools::LocalToolPermissions {
        allow_spawn: false,
        allow_message: true,
    });
    let mut protocol = legacy_probed_protocol(settings.clone());
    assert_eq!(
        protocol.initialize()["params"]["capabilities"]["experimentalApi"],
        true
    );
    let effects = protocol
        .receive(captured_output(fixture, |message| message["id"] == 1))
        .unwrap();
    let tools = effects.writes[1]["params"]["dynamicTools"]
        .as_array()
        .unwrap();
    assert_eq!(tools.len(), 2);
    assert!(
        tools
            .iter()
            .all(|tool| tool["type"] == "function" && tool["name"] != "run_agents")
    );
    protocol
        .receive(captured_output(fixture, |message| message["id"] == 2))
        .unwrap();
    settings.target = SessionTarget::Resume {
        native_session_id: protocol.session_id.clone().unwrap(),
    };
    assert!(connect(settings.clone()).is_ok());
    let mut probe = legacy_probed_protocol(settings);
    probe.initialize();
    let effects = probe
        .receive(captured_output(fixture, |message| message["id"] == 1))
        .unwrap();
    assert_eq!(effects.writes[1]["method"], "thread/resume");
    assert!(effects.writes[1]["params"].get("dynamicTools").is_none());
    assert!(
        legacy_probed_protocol(options()).initialize()["params"]
            .get("capabilities")
            .is_none()
    );
}

#[test]
fn native_tool_ledger_rejects_conflicts_stale_turns_and_ungranted_tools() {
    let mut protocol = ready_protocol(TWO_TURNS);
    protocol.options.local_tools = Some(local_tools::LocalToolPermissions::default());
    let turn_id = start_captured_turn(&mut protocol, TWO_TURNS);
    protocol.receive(json!({"method":"turn/started", "params":{"threadId":protocol.session_id,"turn":{"id":turn_id}}})).unwrap();
    // 工具调用为协议夹具，真实模型调用验收由独立 ignored 测试负责。
    let request = json!({"id":"native-tool-1", "method":"item/tool/call", "params":{
        "threadId":protocol.session_id, "turnId":turn_id, "callId":"call-1", "tool":"inspect_local_tasks", "arguments":{}}});
    let first = protocol.receive(request.clone()).unwrap();
    assert!(
        matches!(&first.events[..], [RuntimeEventKind::LocalToolRequested { request }] if request.call_id == "call-1")
    );
    assert!(protocol.receive(request.clone()).unwrap().events.is_empty());
    let response = command(
        RuntimeAction::RespondLocalTool {
            turn_id: turn_id.clone(),
            call_id: "call-1".into(),
            result: Ok(json!({"state":"running"})),
        },
        201,
    );
    let effects = protocol.command(response.clone());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::CommandDispatched { .. }]
    ));
    assert_eq!(effects.writes[0]["id"], "native-tool-1");
    assert_eq!(effects.writes[0]["result"]["success"], true);
    assert_eq!(
        protocol.receive(request.clone()).unwrap().writes,
        effects.writes
    );
    assert!(protocol.command(response).writes.is_empty());
    let mut conflict = request.clone();
    conflict["params"]["arguments"] = json!({"task_ids":["other"]});
    assert!(
        protocol.receive(conflict).unwrap().writes[0]
            .get("error")
            .is_some()
    );
    let mut denied = request.clone();
    denied["id"] = json!("native-tool-2");
    denied["params"]["callId"] = json!("call-2");
    denied["params"]["tool"] = json!("run_agents");
    assert!(
        protocol.receive(denied).unwrap().writes[0]
            .get("error")
            .is_some()
    );
    protocol.active_turn = None;
    assert!(
        protocol.receive(request).unwrap().writes[0]
            .get("error")
            .is_some()
    );
    assert!(matches!(
        protocol
            .command(command(
                RuntimeAction::RespondLocalTool {
                    turn_id,
                    call_id: "call-1".into(),
                    result: Ok(json!({}))
                },
                202
            ))
            .events[0],
        RuntimeEventKind::RequestFailed { .. }
    ));
}

#[test]
fn native_skill_input_preserves_name_and_absolute_path() {
    let path = std::env::temp_dir().join("中文 skill/SKILL.md");
    assert_eq!(
        encode_input(vec![InputContent::Skill {
            name: "审查".into(),
            path: path.clone()
        }])
        .unwrap(),
        vec![json!({"type":"skill","name":"审查","path":path})]
    );
    assert!(
        encode_input(vec![InputContent::Skill {
            name: "review".into(),
            path: PathBuf::from("relative/SKILL.md")
        }])
        .is_err()
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn captured_child_permission_drift_is_rejected_before_ready_or_initial_input() {
    let parent_reply = captured_output(HANDSHAKE, |message| {
        message.pointer("/result/sandbox").is_some()
    });
    let permissions = json!({
        "approvalPolicy":parent_reply["result"]["approvalPolicy"],
        "sandbox":parent_reply["result"]["sandbox"],
        "approvalsReviewer":parent_reply["result"]["approvalsReviewer"],
    });
    let mut settings = options();
    settings.permission_policy = PermissionPolicy::Inherit;
    let parent = crate::persistence::model::LocalCliTask {
        version: 1,
        task_id: "parent".into(),
        parent_task_id: None,
        parent_generation: None,
        harness: "codex".into(),
        working_directory: settings.cwd.to_string_lossy().into(),
        config_json: json!({"cli_version":"0.147.0","effective_permissions":permissions})
            .to_string(),
        native_session_id: Some("parent-native".into()),
        generation: 1,
        revision: 1,
        state: crate::persistence::model::LocalCliTaskState::Running,
        result: None,
        terminal_evidence: None,
    };
    settings.permission_ceiling =
        Some(super::super::permissions::ceiling_from_parent(&parent, "codex").unwrap());
    let mut protocol = legacy_probed_protocol(settings.clone());
    protocol.initialize();
    protocol
        .receive(captured_output(HANDSHAKE, |message| message["id"] == 1))
        .unwrap();
    let drifted = captured_output(TWO_TURNS, |message| {
        message.pointer("/result/sandbox").is_some()
    });
    assert!(matches!(
        protocol.receive(drifted),
        Err(RuntimeError::PermissionCeilingRejected { .. })
    ));
    assert!(protocol.session_id.is_none());
    assert!(
        protocol
            .command(command(
                RuntimeAction::Submit {
                    input: vec![InputContent::Text("must not run".into())]
                },
                99
            ))
            .writes
            .is_empty()
    );

    let mut matching = legacy_probed_protocol(settings);
    matching.initialize();
    matching
        .receive(captured_output(HANDSHAKE, |message| message["id"] == 1))
        .unwrap();
    let effects = matching.receive(parent_reply).unwrap();
    assert!(matches!(
        &effects.events[..],
        [RuntimeEventKind::SessionReady { .. }]
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "codex_idle_crash_tests.rs"]
mod idle_crash;

#[test]
fn latest_version_probe_and_matching_handshake_preserve_requested_permissions() {
    let mut protocol = CodexProtocol::new(options());
    protocol.record_version_probe("codex-cli 0.155.1").unwrap();
    protocol.initialize();
    // 新版握手是协议形状夹具，不是原生执行记录。
    let effects = protocol
        .receive(json!({"id":1,"result":{
            "userAgent":"infinishell/0.155.1 (Linux; x86_64)"
        }}))
        .unwrap();
    assert_eq!(effects.writes[1]["method"], "thread/start");
    assert_eq!(effects.writes[1]["params"]["approvalPolicy"], "untrusted");
    assert_eq!(effects.writes[1]["params"]["approvalsReviewer"], "user");
    assert_eq!(effects.writes[1]["params"]["sandbox"], "workspace-write");
    assert!(effects.events.is_empty());
    let ready = protocol
        .receive(captured_output(TWO_TURNS, |message| message["id"] == 2))
        .unwrap();
    assert!(
        matches!(ready.events.as_slice(), [RuntimeEventKind::SessionReady {
        verified_cli_version: Some(version), ..
    }] if version == "0.155.1")
    );
}

#[test]
fn unknown_version_probe_does_not_retain_a_previous_supported_version() {
    let mut protocol = legacy_probed_protocol(options());
    assert!(matches!(
        protocol.record_version_probe("codex-cli 0.156.0"),
        Err(RuntimeError::UnsupportedVersion(_))
    ));
    assert!(protocol.probed_version.is_none());
    assert!(matches!(
        protocol.record_version_probe("codex-cli 0.155.10"),
        Err(RuntimeError::UnsupportedVersion(_))
    ));
    assert!(matches!(
        protocol.record_version_probe("codex-cli 0.155.1-alpha.1"),
        Err(RuntimeError::UnsupportedVersion(_))
    ));
}

#[test]
fn latest_probe_cannot_initialize_an_older_app_server() {
    let mut protocol = CodexProtocol::new(options());
    protocol.record_version_probe("codex-cli 0.155.1").unwrap();
    protocol.initialize();
    assert!(matches!(
        protocol.receive(captured_output(HANDSHAKE, |message| message["id"] == 1)),
        Err(RuntimeError::UnsupportedVersion(_))
    ));
    assert!(protocol.pending.is_empty());
    assert!(protocol.session_id.is_none());
}

#[test]
fn older_probe_cannot_initialize_a_newer_app_server() {
    let mut protocol = legacy_probed_protocol(options());
    protocol.initialize();
    assert!(matches!(
        protocol.receive(json!({"id":1,"result":{
            "userAgent":"infinishell/0.155.1 (Linux; x86_64)"
        }})),
        Err(RuntimeError::UnsupportedVersion(_))
    ));
    assert!(protocol.pending.is_empty());
    assert!(protocol.session_id.is_none());
}

#[test]
fn known_version_in_user_agent_details_does_not_override_its_actual_version() {
    for user_agent in [
        "infinishell/0.999.0 (Linux; x86_64) other/0.155.1 extra",
        "/0.155.1 extra",
    ] {
        let mut protocol = CodexProtocol::new(options());
        protocol.record_version_probe("codex-cli 0.155.1").unwrap();
        protocol.initialize();
        assert!(matches!(
            protocol.receive(json!({"id":1,"result":{"userAgent":user_agent}})),
            Err(RuntimeError::UnsupportedVersion(_))
        ));
        assert!(protocol.pending.is_empty());
    }
}

#[test]
fn matching_handshake_cannot_replace_a_missing_version_probe() {
    let mut protocol = CodexProtocol::new(options());
    protocol.initialize();
    assert!(matches!(
        protocol.receive(captured_output(HANDSHAKE, |message| message["id"] == 1)),
        Err(RuntimeError::UnsupportedVersion(_))
    ));
    assert!(protocol.pending.is_empty());
}

#[test]
fn legacy_version_still_resumes_the_requested_native_session() {
    let mut settings = options();
    settings.target = SessionTarget::Resume {
        native_session_id: "legacy-native-session".into(),
    };
    let mut protocol = legacy_probed_protocol(settings);
    protocol.initialize();
    let effects = protocol
        .receive(captured_output(HANDSHAKE, |message| message["id"] == 1))
        .unwrap();
    assert_eq!(effects.writes[1]["method"], "thread/resume");
    assert_eq!(
        effects.writes[1]["params"]["threadId"],
        "legacy-native-session"
    );
    let mut reply = captured_output(TWO_TURNS, |message| message["id"] == 2);
    reply["result"]["thread"]["id"] = json!("legacy-native-session");
    let ready = protocol.receive(reply).unwrap();
    assert!(
        matches!(ready.events.as_slice(), [RuntimeEventKind::SessionReady {
        verified_cli_version: Some(version), ..
    }] if version == "0.147.0")
    );
}

#[test]
fn latest_version_does_not_enable_unimplemented_approval_or_elicitation_requests() {
    let mut protocol = CodexProtocol::new(options());
    protocol.record_version_probe("codex-cli 0.155.1").unwrap();
    protocol.initialize();
    protocol
        .receive(json!({"id":1,"result":{
            "userAgent":"infinishell/0.155.1 (Linux; x86_64)"
        }}))
        .unwrap();
    let approval = protocol
        .receive(json!({"id":20,"method":"item/permissions/requestApproval","params":{}}))
        .unwrap();
    let question = protocol
        .receive(json!({"id":21,"method":"item/tool/requestUserInput","params":{}}))
        .unwrap();
    let elicitation = protocol
        .receive(json!({"id":22,"method":"mcpServer/elicitation/request","params":{}}))
        .unwrap();
    assert_eq!(approval.writes[0]["error"]["code"], -32601);
    assert_eq!(question.writes[0]["error"]["code"], -32601);
    assert_eq!(elicitation.writes[0]["error"]["code"], -32601);
    assert!(approval.events.is_empty());
    assert!(question.events.is_empty());
    assert!(elicitation.events.is_empty());
    assert!(protocol.approvals.is_empty());
}

#[test]
fn probe_replacement_after_handshake_cannot_claim_a_paired_ready_version() {
    let mut protocol = legacy_probed_protocol(options());
    protocol.initialize();
    protocol
        .receive(captured_output(HANDSHAKE, |message| message["id"] == 1))
        .unwrap();
    protocol.record_version_probe("codex-cli 0.155.1").unwrap();
    assert!(
        protocol
            .receive(captured_output(TWO_TURNS, |message| message["id"] == 2))
            .is_err()
    );
    assert!(protocol.session_id.is_none());
    assert!(protocol.paired_version.is_none());
}
