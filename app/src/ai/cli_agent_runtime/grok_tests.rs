use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures::io::Cursor;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{
    Effects, GrokProtocol, REQUEST_TIMEOUT, flush_effects, supported_version, validate_options,
    verified_version,
};
use crate::ai::cli_agent_runtime::grok_profile::GrokCreationPolicyV1;
use crate::ai::cli_agent_runtime::local_skills::SelectedLocalSkill;
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
        grok_profile: None,
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

fn current_initialize() -> Value {
    let mut initialize = fixture_response(1);
    initialize["result"]["_meta"]["agentVersion"] = json!("1.0.40");
    initialize
}

fn current_setup_messages(session_id: &str) -> Vec<Value> {
    super::CURRENT_SETUP_PHASES
        .iter()
        .enumerate()
        .map(|(index, phase)| {
            json!({
                "jsonrpc": "2.0",
                "method": super::CURRENT_SETUP_METHOD,
                "params": {
                    "method": "session/new",
                    "phase": phase,
                    "sessionId": if index < 5 { Value::Null } else { json!(session_id) }
                }
            })
        })
        .collect()
}

fn current_waiting_for_setup() -> GrokProtocol {
    let mut protocol = GrokProtocol::new(options());
    protocol
        .bind_cli_version("grok 1.0.40 (eb1a2256660d)")
        .unwrap();
    protocol.initialize().unwrap();
    protocol.receive(current_initialize()).unwrap();
    protocol
        .receive(json!({
            "jsonrpc": "2.0",
            "method": "_x.ai/mcp/servers_updated",
            "params": {"mcpServers": []}
        }))
        .unwrap();
    protocol.receive(fixture_response(2)).unwrap();
    protocol
}

fn current_display_notifications() -> Vec<Value> {
    let fixture = authenticated_fixture();
    let mut messages = vec![
        fixture
            .iter()
            .find(|message| message["method"] == "_x.ai/models/update")
            .unwrap()
            .clone(),
        fixture
            .iter()
            .find(|message| message["method"] == "_x.ai/settings/update")
            .unwrap()
            .clone(),
    ];
    messages.extend(
        fixture
            .into_iter()
            .filter(|message| message["method"] == "_x.ai/announcements/update")
            .take(2),
    );
    let session_id = fixture_response(3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    messages.push(current_command_catalog(&session_id));
    assert_eq!(messages.len(), 5);
    messages
}

fn current_command_catalog(session_id: &str) -> Value {
    const COMMAND_HAS_META: [bool; 29] = [
        false, false, false, false, false, true, false, false, false, true, true, true, true, true,
        true, true, true, true, true, true, true, true, true, true, true, true, true, true, true,
    ];
    const COMMAND_HAS_INPUT: [bool; 29] = [
        true, true, false, false, true, true, true, true, true, true, false, false, false, true,
        true, false, false, true, true, false, true, true, true, true, true, false, false, false,
        true,
    ];
    let commands = COMMAND_HAS_META
        .iter()
        .zip(COMMAND_HAS_INPUT)
        .enumerate()
        .map(|(index, (has_meta, has_input))| {
            let mut command = json!({
                "name": format!("command-{index}"),
                "description": "固定握手目录描述",
                "input": if has_input { json!({}) } else { Value::Null },
            });
            if *has_meta {
                command["_meta"] = json!({});
            }
            command
        })
        .collect::<Vec<_>>();
    let tools = (0..27)
        .map(|index| format!("tool-{index}"))
        .collect::<Vec<_>>();
    json!({
        "jsonrpc":"2.0",
        "method":"session/update",
        "params":{
            "sessionId":session_id,
            "_meta":{
                "agentTimestampMs":1,
                "eventId":format!("{session_id}-2"),
                "totalTokens":0,
                "updateParams":{},
                "updateType":"available_commands_update"
            },
            "update":{
                "sessionUpdate":"available_commands_update",
                "availableCommands":commands,
                "_meta":{"tools":tools}
            }
        }
    })
}

fn current_waiting_for_display_notifications() -> GrokProtocol {
    let mut protocol = current_waiting_for_setup();
    let session_id = fixture_response(3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let setup = current_setup_messages(&session_id);
    for message in &setup[..5] {
        protocol.receive(message.clone()).unwrap();
    }
    protocol.receive(fixture_response(3)).unwrap();
    protocol.receive(setup[5].clone()).unwrap();
    protocol
}

fn ready_protocol() -> GrokProtocol {
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
    for id in 1..=3 {
        protocol.receive(fixture_response(id)).unwrap();
    }
    protocol
}

#[test]
fn only_exact_observed_cli_versions_are_accepted() {
    assert_eq!(
        verified_version("grok 1.0.30 (04b7ffed98c6)\n"),
        Some("1.0.30")
    );
    assert_eq!(
        verified_version("grok 1.0.34 (3736acbc8658)\n"),
        Some("1.0.34")
    );
    assert_eq!(
        verified_version("grok 1.0.40 (eb1a2256660d)\n"),
        Some("1.0.40")
    );
    assert_eq!(verified_version("grok 1.0.29 (other)"), None);
    assert_eq!(verified_version("grok 1.0.31"), None);
    assert_eq!(verified_version("grok 1.0.35"), None);
    assert_eq!(verified_version("grok 1.0.30-beta"), None);
    assert_eq!(verified_version("grok 1.0.34-alpha"), None);
    assert_eq!(verified_version("grok 1.0.40-alpha"), None);
    assert!(supported_version("1.0.30"));
    assert!(supported_version("1.0.34"));
    assert!(supported_version("1.0.40"));
    assert!(!supported_version("1.0.41"));
}

#[test]
fn initialize_without_a_successful_probe_does_not_send_a_request() {
    let mut protocol = GrokProtocol::new(options());
    assert!(protocol.initialize().is_err());
    assert_eq!(protocol.next_id, 0);
    assert!(protocol.pending.is_none());
    assert!(protocol.session_id.is_none());
}

#[test]
fn supported_probe_and_initialize_versions_must_match_in_both_directions() {
    for (probe, handshake) in [("grok 1.0.30", "1.0.34"), ("grok 1.0.34", "1.0.30")] {
        let mut protocol = GrokProtocol::new(options());
        protocol.bind_cli_version(probe).unwrap();
        protocol.initialize().unwrap();
        let mut response = fixture_response(1);
        response["result"]["_meta"]["agentVersion"] = json!(handshake);
        assert!(protocol.receive(response).is_err());
        assert_eq!(protocol.next_id, 1);
        assert!(protocol.session_id.is_none());
    }
}

#[test]
fn unknown_probe_cannot_fall_back_to_the_historical_fixture_version() {
    let mut protocol = GrokProtocol::new(options());
    protocol.bind_cli_version("grok 1.0.30").unwrap();
    assert!(protocol.bind_cli_version("grok 1.0.35").is_err());
    assert!(protocol.initialize().is_err());
    assert!(protocol.probed_version.is_none());
    assert!(protocol.paired_version.is_none());
    assert_eq!(protocol.next_id, 0);
}

#[test]
fn a_probe_cannot_replace_the_version_after_initialization() {
    let mut protocol = GrokProtocol::new(options());
    protocol.bind_cli_version("grok 1.0.30").unwrap();
    protocol.initialize().unwrap();
    assert!(protocol.bind_cli_version("grok 1.0.34").is_err());
    assert_eq!(protocol.probed_version, Some("1.0.30"));
}

#[test]
fn p0_version_enables_only_the_receipted_fixed_read_scope() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let mut fixed = options();
    fixed.cwd.clone_from(&cwd);
    fixed.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    fixed.grok_profile = Some(
        GrokCreationPolicyV1::compile(
            &cwd,
            "1.0.34",
            "9cd26b579840f0f5c9148a8059ad651904c08b41b7f2ef0b4ec04b9ba898844e".into(),
            "b".repeat(64),
            None,
            PermissionPolicy::GrokRestrictedReadV1,
        )
        .unwrap(),
    );
    let mut fixed = GrokProtocol::new(fixed);
    fixed.bind_cli_version("grok 1.0.34").unwrap();
    assert!(fixed.fixed_read_policy_verified());
    assert!(!fixed.extended_lifecycle_verified());

    let mut missing_profile = options();
    missing_profile.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    assert!(
        GrokProtocol::new(missing_profile)
            .bind_cli_version("grok 1.0.34")
            .is_err()
    );

    let mut files = options();
    files.permission_policy = PermissionPolicy::GrokRestrictedFilesV1;
    assert!(
        GrokProtocol::new(files)
            .bind_cli_version("grok 1.0.34")
            .is_err()
    );
    let mut tools = options();
    tools.local_tools = Some(LocalToolPermissions {
        allow_spawn: false,
        allow_message: true,
    });
    assert!(
        GrokProtocol::new(tools)
            .bind_cli_version("grok 1.0.34")
            .is_err()
    );

    let mut skills = options();
    skills.selected_skills.push(SelectedLocalSkill {
        name: "not-opened".into(),
        path: skills.cwd.join("SKILL.md"),
    });
    assert!(
        GrokProtocol::new(skills)
            .bind_cli_version("grok 1.0.34")
            .is_err()
    );
}

#[test]
fn p0_synthetic_handshake_claims_only_the_native_p0_verified_capabilities() {
    let mut protocol = GrokProtocol::new(options());
    protocol
        .bind_cli_version("grok 1.0.34 (3736acbc8658)")
        .unwrap();
    protocol.initialize().unwrap();
    // 仅复用旧回执构造离线状态；不能将此测试计为新版认证或会话验收。
    let mut initialize = fixture_response(1);
    initialize["result"]["_meta"]["agentVersion"] = json!("1.0.34");
    let auth = protocol.receive(initialize).unwrap();
    assert_eq!(auth.writes[0]["method"], "authenticate");
    protocol.receive(fixture_response(2)).unwrap();
    let ready = protocol.receive(fixture_response(3)).unwrap();
    let [
        RuntimeEventKind::SessionReady {
            effective_permissions,
            verified_cli_version,
        },
    ] = ready.events.as_slice()
    else {
        panic!("合成握手应返回就绪状态");
    };
    assert_eq!(verified_cli_version.as_deref(), Some("1.0.34"));
    assert_eq!(
        effective_permissions["reportedCapabilities"]["promptCapabilities"]["image"],
        false
    );
    assert_eq!(
        effective_permissions["reportedInitializeExtensions"],
        json!({
            "queueChangeNotifications": false,
            "queueInterjectRequests": false,
            "skillsMethods": false,
            "availableCommands": true,
            "localMcpSdkAdvertised": true
        })
    );
    assert_eq!(
        effective_permissions["verifiedCapabilities"],
        json!({
            "newSession": true, "emptyHistoryRecovery": false, "closeSession": false,
            "submit": true, "queuedSubmit": false, "steer": false, "approval": true,
            "cancel": true, "resume": true, "localTools": false, "childTasks": false
        })
    );
    assert_eq!(
        effective_permissions["permissionEnforcementVerified"],
        false
    );
    let rejected = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::LocalImage(PathBuf::from("image.png"))],
        },
    });
    assert!(rejected.writes.is_empty());
    assert!(matches!(
        rejected.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    let skill = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Skill {
                name: "offline-skill".into(),
                path: protocol.options.cwd.join("SKILL.md"),
            }],
        },
    });
    assert!(skill.writes.is_empty());
    assert!(matches!(
        skill.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(protocol.submitted_messages.is_empty() && protocol.queued.is_empty());

    let first_id = Uuid::new_v4();
    let first = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: first_id,
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("已验证的单回合输入".into())],
        },
    });
    assert_eq!(first.writes[0]["method"], "session/prompt");
    let queued = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("未验证的排队输入".into())],
        },
    });
    assert!(queued.writes.is_empty());
    assert!(matches!(
        queued.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(protocol.queued.is_empty());

    let shutdown = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Shutdown,
    });
    assert!(shutdown.writes.is_empty());
    assert!(protocol.closed);
    assert!(shutdown.events.iter().any(|event| matches!(
        event,
        RuntimeEventKind::CommandDispatched { turn_id: None, .. }
    )));
}

#[test]
fn current_authenticated_shape_opens_only_after_the_exact_setup_sequence() {
    let mut protocol = current_waiting_for_setup();
    let session_id = fixture_response(3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let setup = current_setup_messages(&session_id);
    for setup in &setup[..5] {
        let effects = protocol.receive(setup.clone()).unwrap();
        assert!(effects.writes.is_empty() && effects.events.is_empty());
    }
    let response = protocol.receive(fixture_response(3)).unwrap();
    assert!(response.writes.is_empty() && response.events.is_empty());
    protocol.receive(setup[5].clone()).unwrap();
    for notification in current_display_notifications() {
        protocol.receive(notification).unwrap();
    }
    let mut ready = Effects::default();
    for setup in &setup[6..] {
        let effects = protocol.receive(setup.clone()).unwrap();
        ready = effects;
    }
    let [
        RuntimeEventKind::SessionReady {
            effective_permissions,
            verified_cli_version,
        },
    ] = ready.events.as_slice()
    else {
        panic!("当前版本精确 setup 完成后应返回就绪状态");
    };
    assert_eq!(verified_cli_version.as_deref(), Some("1.0.40"));
    assert_eq!(
        effective_permissions["verifiedCapabilities"],
        json!({
            "newSession": true, "emptyHistoryRecovery": false, "closeSession": false,
            "submit": false, "queuedSubmit": false, "steer": false, "approval": false,
            "cancel": false, "resume": false, "localTools": false, "childTasks": false
        })
    );
    assert!(protocol.current_setup.is_none());
    let rejected = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("未验收的当前版本回合".into())],
        },
    });
    assert!(rejected.writes.is_empty());
    assert!(matches!(
        rejected.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
}

#[test]
fn current_setup_accepts_independent_native_events_in_both_observed_orders() {
    let session_id = fixture_response(3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let setup = current_setup_messages(&session_id);
    let display = current_display_notifications();

    let mut models_before_response = current_waiting_for_setup();
    for message in &setup[..5] {
        models_before_response.receive(message.clone()).unwrap();
    }
    models_before_response.receive(display[0].clone()).unwrap();
    models_before_response.receive(fixture_response(3)).unwrap();
    models_before_response.receive(setup[5].clone()).unwrap();
    for message in &display[1..] {
        models_before_response.receive(message.clone()).unwrap();
    }
    let mut ready = Effects::default();
    for message in &setup[6..] {
        ready = models_before_response.receive(message.clone()).unwrap();
    }
    assert!(matches!(
        ready.events.as_slice(),
        [RuntimeEventKind::SessionReady { .. }]
    ));

    let mut display_before_response = current_waiting_for_setup();
    for message in &setup[..5] {
        display_before_response.receive(message.clone()).unwrap();
    }
    for index in [1, 2, 0, 3, 4] {
        display_before_response
            .receive(display[index].clone())
            .unwrap();
    }
    display_before_response
        .receive(fixture_response(3))
        .unwrap();
    display_before_response.receive(setup[5].clone()).unwrap();
    let mut ready = Effects::default();
    for message in &setup[6..] {
        ready = display_before_response.receive(message.clone()).unwrap();
    }
    assert!(matches!(
        ready.events.as_slice(),
        [RuntimeEventKind::SessionReady { .. }]
    ));
}

#[test]
fn current_root_candidate_is_confined_to_the_ignored_live_harness() {
    let mut protocol = current_waiting_for_setup();
    protocol.current_root_candidate_for_live = true;
    let session_id = fixture_response(3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let setup = current_setup_messages(&session_id);
    for setup in &setup[..5] {
        protocol.receive(setup.clone()).unwrap();
    }
    let response = protocol.receive(fixture_response(3)).unwrap();
    assert!(response.events.is_empty());
    protocol.receive(setup[5].clone()).unwrap();
    for notification in current_display_notifications() {
        protocol.receive(notification).unwrap();
    }
    let mut ready = Effects::default();
    for setup in &setup[6..] {
        ready = protocol.receive(setup.clone()).unwrap();
    }
    let [
        RuntimeEventKind::SessionReady {
            effective_permissions,
            ..
        },
    ] = ready.events.as_slice()
    else {
        panic!("候选 live 门禁应完成精确当前版握手");
    };
    assert_eq!(
        effective_permissions["verifiedCapabilities"],
        json!({
            "newSession": true, "emptyHistoryRecovery": false, "closeSession": false,
            "submit": true, "queuedSubmit": true, "steer": false, "approval": true,
            "cancel": true, "resume": true, "localTools": false, "childTasks": false
        })
    );

    let first = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("候选首轮".into())],
        },
    });
    assert_eq!(first.writes[0]["method"], "session/prompt");
    let second = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("候选运行中后续输入".into())],
        },
    });
    assert!(second.writes.is_empty() && second.events.is_empty());
    assert_eq!(protocol.queued.len(), 1);
}

#[test]
fn current_candidate_approval_is_open_only_inside_the_live_harness() {
    let (mut closed, request) = pending_native_approval();
    closed.probed_version = Some("1.0.40");
    let rejected = closed.receive(request.clone()).unwrap();
    assert!(rejected.events.is_empty());
    assert_eq!(
        rejected.writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );

    let (mut candidate, request) = pending_native_approval();
    candidate.probed_version = Some("1.0.40");
    candidate.current_root_candidate_for_live = true;
    let opened = candidate.receive(request).unwrap();
    assert!(opened.writes.is_empty());
    assert!(matches!(
        opened.events.as_slice(),
        [RuntimeEventKind::ApprovalRequested { .. }]
    ));
}

#[test]
fn current_setup_rejects_unknown_reordered_duplicate_and_old_phases() {
    let session_id = fixture_response(3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    for (name, mut messages) in [
        ("unknown", current_setup_messages(&session_id)),
        ("reordered", current_setup_messages(&session_id)),
        ("duplicate", current_setup_messages(&session_id)),
        ("old", current_setup_messages(&session_id)),
    ] {
        match name {
            "unknown" => messages[0]["params"]["phase"] = json!("future_phase"),
            "reordered" => messages.swap(0, 1),
            "duplicate" => messages[1] = messages[0].clone(),
            "old" => messages[0]["params"]["phase"] = json!("create_session"),
            _ => unreachable!(),
        }
        let mut protocol = current_waiting_for_setup();
        let result = messages
            .into_iter()
            .try_for_each(|message| protocol.receive(message).map(|_| ()));
        assert!(result.is_err(), "{name} setup 必须 fail-closed");
        assert!(protocol.session_id.is_none());
    }
}

#[test]
fn current_setup_requires_exact_fields_stable_identity_and_all_phases() {
    let session_id = fixture_response(3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut protocol = current_waiting_for_setup();
    let mut setup = current_setup_messages(&session_id);
    setup[0]["params"]["legacy"] = json!(true);
    assert!(protocol.receive(setup.remove(0)).is_err());

    let mut protocol = current_waiting_for_setup();
    let setup = current_setup_messages(&session_id);
    for message in setup.into_iter().take(4) {
        protocol.receive(message).unwrap();
    }
    assert!(protocol.receive(fixture_response(3)).is_err());

    let mut protocol = current_waiting_for_setup();
    let mut setup = current_setup_messages(&session_id);
    setup[6]["params"]["sessionId"] = json!(Uuid::new_v4().to_string());
    for message in &setup[..5] {
        protocol.receive(message.clone()).unwrap();
    }
    protocol.receive(fixture_response(3)).unwrap();
    protocol.receive(setup[5].clone()).unwrap();
    for notification in current_display_notifications() {
        protocol.receive(notification).unwrap();
    }
    let result = setup[6..]
        .iter()
        .cloned()
        .try_for_each(|message| protocol.receive(message).map(|_| ()));
    assert!(result.is_err());
    assert_eq!(protocol.session_id.as_deref(), Some(session_id.as_str()));

    let mut catalog_identity = current_waiting_for_setup();
    for message in &current_setup_messages(&session_id)[..5] {
        catalog_identity.receive(message.clone()).unwrap();
    }
    let foreign_session = Uuid::new_v4().to_string();
    catalog_identity
        .receive(current_command_catalog(&foreign_session))
        .unwrap();
    assert!(catalog_identity.receive(fixture_response(3)).is_err());
    assert!(catalog_identity.session_id.is_none());
}

#[test]
fn current_setup_requires_the_response_before_post_response_phases() {
    let session_id = fixture_response(3)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let setup = current_setup_messages(&session_id);

    let mut early_response = current_waiting_for_setup();
    for message in &setup[..4] {
        early_response.receive(message.clone()).unwrap();
    }
    assert!(early_response.receive(fixture_response(3)).is_err());

    let mut early_post_phase = current_waiting_for_setup();
    for message in &setup[..5] {
        early_post_phase.receive(message.clone()).unwrap();
    }
    assert!(early_post_phase.receive(setup[5].clone()).is_err());

    let mut duplicate_after_response = current_waiting_for_setup();
    for message in &setup[..5] {
        duplicate_after_response.receive(message.clone()).unwrap();
    }
    duplicate_after_response
        .receive(fixture_response(3))
        .unwrap();
    duplicate_after_response.receive(setup[5].clone()).unwrap();
    assert!(duplicate_after_response.receive(setup[5].clone()).is_err());

    let mut missing_response_ready = current_waiting_for_setup();
    for message in &setup[..5] {
        missing_response_ready.receive(message.clone()).unwrap();
    }
    missing_response_ready.receive(fixture_response(3)).unwrap();
    assert!(missing_response_ready.receive(setup[6].clone()).is_err());

    let mut changed_response_identity = current_waiting_for_setup();
    for message in &setup[..5] {
        changed_response_identity.receive(message.clone()).unwrap();
    }
    changed_response_identity
        .receive(fixture_response(3))
        .unwrap();
    let mut changed = setup[5].clone();
    changed["params"]["sessionId"] = json!(Uuid::new_v4().to_string());
    assert!(changed_response_identity.receive(changed).is_err());
}

#[test]
fn current_handshake_allows_only_the_exact_empty_mcp_refresh() {
    let mcp = json!({
        "jsonrpc": "2.0",
        "method": "_x.ai/mcp/servers_updated",
        "params": {"mcpServers": []}
    });
    let mut duplicate_mcp = current_waiting_for_setup();
    assert!(duplicate_mcp.receive(mcp).is_err());

    let display = current_display_notifications();
    let models = display[0].clone();
    let settings = display[1].clone();
    let announcements = display[2].clone();
    let announcements_2 = display[3].clone();
    let commands = display[4].clone();
    let mut too_early = current_waiting_for_setup();
    assert!(too_early.receive(models.clone()).is_err());

    let mut protocol = current_waiting_for_display_notifications();
    let accepted = protocol.receive(models.clone()).unwrap();
    assert!(accepted.writes.is_empty() && accepted.events.is_empty());
    assert_eq!(
        protocol
            .reported_metadata
            .models
            .as_ref()
            .unwrap()
            .current_model_id,
        "grok-4.6"
    );
    let accepted = protocol.receive(settings.clone()).unwrap();
    assert!(accepted.writes.is_empty() && accepted.events.is_empty());
    let accepted = protocol.receive(announcements.clone()).unwrap();
    assert!(accepted.writes.is_empty() && accepted.events.is_empty());
    let accepted = protocol.receive(announcements_2.clone()).unwrap();
    assert!(accepted.writes.is_empty() && accepted.events.is_empty());
    let accepted = protocol.receive(commands.clone()).unwrap();
    assert!(accepted.writes.is_empty() && accepted.events.is_empty());
    assert!(protocol.skill_catalog.is_none());
    assert!(protocol.creation_catalog_session.is_none());

    let mut duplicate = current_waiting_for_display_notifications();
    duplicate.receive(models.clone()).unwrap();
    assert!(duplicate.receive(models.clone()).is_err());

    let mut incomplete = current_waiting_for_display_notifications();
    incomplete.receive(models.clone()).unwrap();
    incomplete.receive(settings.clone()).unwrap();
    incomplete.receive(announcements.clone()).unwrap();
    incomplete.receive(announcements_2.clone()).unwrap();
    let setup =
        current_setup_messages(fixture_response(3)["result"]["sessionId"].as_str().unwrap());
    assert!(incomplete.receive(setup[6].clone()).is_err());

    for rejected in [
        json!({"jsonrpc":"2.0","method":"unknown","params":{}}),
        json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated","params":{"mcpServers":[],"extra":true}}),
        json!({"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated","params":{"mcpServers":["foreign"]}}),
    ] {
        let mut protocol = current_waiting_for_setup();
        assert!(protocol.receive(rejected).is_err());
    }

    let mut extra_model = models.clone();
    extra_model["params"]["extra"] = json!(true);
    let mut protocol = current_waiting_for_display_notifications();
    assert!(protocol.receive(extra_model).is_err());
    let mut changed_model = models.clone();
    changed_model["params"]["availableModels"][0]["foreign"] = json!(true);
    let mut protocol = current_waiting_for_display_notifications();
    assert!(protocol.receive(changed_model).is_err());

    let mut extra_setting = settings.clone();
    extra_setting["params"]["foreign"] = json!(true);
    let mut protocol = current_waiting_for_display_notifications();
    protocol.receive(models.clone()).unwrap();
    assert!(protocol.receive(extra_setting).is_err());
    let mut missing = settings.clone();
    missing["params"].as_object_mut().unwrap().remove("tips");
    let mut protocol = current_waiting_for_display_notifications();
    protocol.receive(models.clone()).unwrap();
    assert!(protocol.receive(missing).is_err());
    let mut permission = settings.clone();
    permission["params"]["permission_mode"] = json!("auto");
    let mut protocol = current_waiting_for_display_notifications();
    protocol.receive(models.clone()).unwrap();
    assert!(protocol.receive(permission).is_err());

    let mut changed_announcement = announcements.clone();
    changed_announcement["params"]["extra"] = json!(true);
    let mut protocol = current_waiting_for_display_notifications();
    protocol.receive(models.clone()).unwrap();
    protocol.receive(settings.clone()).unwrap();
    assert!(protocol.receive(changed_announcement).is_err());

    let mut duplicate_generation = announcements_2;
    duplicate_generation["params"]["gen"] = announcements["params"]["gen"].clone();
    let mut protocol = current_waiting_for_display_notifications();
    protocol.receive(models.clone()).unwrap();
    protocol.receive(settings.clone()).unwrap();
    protocol.receive(announcements).unwrap();
    assert!(protocol.receive(duplicate_generation).is_err());

    let mut independent_order = current_waiting_for_display_notifications();
    independent_order.receive(settings).unwrap();
    independent_order.receive(display[2].clone()).unwrap();
    independent_order.receive(models.clone()).unwrap();
    independent_order.receive(commands.clone()).unwrap();
    independent_order.receive(display[3].clone()).unwrap();

    for mutation in ["session", "event", "count", "tools", "extra", "duplicate"] {
        let mut protocol = current_waiting_for_display_notifications();
        protocol.receive(models.clone()).unwrap();
        protocol.receive(display[1].clone()).unwrap();
        protocol.receive(display[2].clone()).unwrap();
        protocol.receive(display[3].clone()).unwrap();
        let mut changed = commands.clone();
        match mutation {
            "session" => changed["params"]["sessionId"] = json!(Uuid::new_v4().to_string()),
            "event" => changed["params"]["_meta"]["eventId"] = json!("old-session-2"),
            "count" => {
                changed["params"]["update"]["availableCommands"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }
            "tools" => {
                changed["params"]["update"]["_meta"]["tools"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }
            "extra" => changed["params"]["update"]["permissionMode"] = json!("auto"),
            "duplicate" => {
                protocol.receive(changed.clone()).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            protocol.receive(changed).is_err(),
            "{mutation} 目录必须 fail-closed"
        );
        assert!(protocol.skill_catalog.is_none());
        assert!(protocol.creation_catalog_session.is_none());
    }
}

#[test]
fn current_oauth_shape_requires_cached_token_without_starting_interactive_login() {
    for mutate in ["missing", "reordered", "wrong_default"] {
        let mut initialize = current_initialize();
        match mutate {
            "missing" => {
                initialize["result"]["authMethods"] = json!([
                    {"id":"grok.com","name":"Grok","description":"Sign in with Grok"}
                ]);
                initialize["result"]["_meta"]["defaultAuthMethodId"] = Value::Null;
            }
            "reordered" => initialize["result"]["authMethods"]
                .as_array_mut()
                .unwrap()
                .reverse(),
            "wrong_default" => {
                initialize["result"]["_meta"]["defaultAuthMethodId"] = json!("grok.com")
            }
            _ => unreachable!(),
        }
        let mut protocol = GrokProtocol::new(options());
        protocol.bind_cli_version("grok 1.0.40").unwrap();
        protocol.initialize().unwrap();
        assert!(protocol.receive(initialize).is_err(), "{mutate} 必须拒绝");
        assert!(protocol.paired_version.is_none());
        assert!(protocol.session_id.is_none());
    }
}

#[test]
fn initialize_binds_exact_capabilities_without_inventing_extension_methods() {
    let mut protocol = GrokProtocol::new(options());
    protocol.bind_cli_version("grok 1.0.34").unwrap();
    protocol.initialize().unwrap();
    let mut initialize = fixture_response(1);
    initialize["result"]["_meta"]["agentVersion"] = json!("1.0.34");
    initialize["result"]["_meta"]["x.ai/queue/interject"] = json!(true);
    initialize["result"]["_meta"]["x.ai/skills"] = json!(true);
    let expected_capabilities = initialize["result"]["agentCapabilities"].clone();

    protocol.receive(initialize).unwrap();

    assert_eq!(protocol.reported_capabilities, expected_capabilities);
    assert!(protocol.reported_initialize_extensions.available_commands);
    assert!(
        protocol
            .reported_initialize_extensions
            .local_mcp_sdk_advertised
    );
    assert!(
        !protocol
            .reported_initialize_extensions
            .queue_change_notifications
    );
    assert!(
        !protocol
            .reported_initialize_extensions
            .queue_interject_requests
    );
    assert!(!protocol.reported_initialize_extensions.skills_methods);
    assert_eq!(protocol.reported_metadata.available_commands.len(), 7);
}

#[test]
fn duplicate_initialize_is_idempotent() {
    let mut protocol = GrokProtocol::new(options());
    protocol.bind_cli_version("grok 1.0.34").unwrap();
    protocol.initialize().unwrap();
    let mut initialize = fixture_response(1);
    initialize["result"]["_meta"]["agentVersion"] = json!("1.0.34");
    protocol.receive(initialize.clone()).unwrap();
    let capabilities = protocol.reported_capabilities.clone();
    let extensions = protocol.reported_initialize_extensions.clone();

    let duplicate = protocol.receive(initialize.clone()).unwrap();
    assert!(duplicate.writes.is_empty() && duplicate.events.is_empty());
    assert_eq!(protocol.reported_capabilities, capabilities);
    assert_eq!(protocol.reported_initialize_extensions, extensions);
    assert_eq!(protocol.pending.as_ref().unwrap().id, 2);
}

#[test]
fn conflicting_old_initialize_cannot_replace_capability_binding() {
    let mut protocol = GrokProtocol::new(options());
    protocol.bind_cli_version("grok 1.0.34").unwrap();
    protocol.initialize().unwrap();
    let mut initialize = fixture_response(1);
    initialize["result"]["_meta"]["agentVersion"] = json!("1.0.34");
    protocol.receive(initialize.clone()).unwrap();
    let capabilities = protocol.reported_capabilities.clone();
    let extensions = protocol.reported_initialize_extensions.clone();

    initialize["result"]["agentCapabilities"]["loadSession"] = json!(false);
    assert!(protocol.receive(initialize).is_err());
    assert_eq!(protocol.reported_capabilities, capabilities);
    assert_eq!(protocol.reported_initialize_extensions, extensions);
    assert_eq!(protocol.pending.as_ref().unwrap().id, 2);
}

#[test]
fn missing_agent_capabilities_do_not_partially_bind() {
    let mut missing = GrokProtocol::from_fixture(options());
    missing.initialize().unwrap();
    let mut initialize = fixture_response(1);
    initialize["result"]
        .as_object_mut()
        .unwrap()
        .remove("agentCapabilities");
    assert!(missing.receive(initialize).is_err());
    assert!(missing.paired_version.is_none());
    assert!(missing.reported_capabilities.is_null());
    assert_eq!(
        missing.reported_initialize_extensions,
        super::ReportedInitializeExtensions::default()
    );
}

#[test]
fn malformed_available_commands_do_not_partially_bind() {
    let mut malformed = GrokProtocol::from_fixture(options());
    malformed.initialize().unwrap();
    let mut initialize = fixture_response(1);
    initialize["result"]["_meta"]["availableCommands"] = json!("not-a-catalog");
    assert!(malformed.receive(initialize).is_err());
    assert!(malformed.paired_version.is_none());
    assert!(malformed.reported_capabilities.is_null());
    assert_eq!(
        malformed.reported_initialize_extensions,
        super::ReportedInitializeExtensions::default()
    );
}

#[test]
fn missing_mcp_advertisement_prevents_session_open() {
    let mut launch = options();
    launch.local_tools = Some(LocalToolPermissions::default());
    let mut protocol = GrokProtocol::from_fixture(launch);
    protocol.sdk.as_mut().unwrap().owned_process = true;
    protocol.initialize().unwrap();
    let mut initialize = fixture_response(1);
    initialize["result"]["_meta"]
        .as_object_mut()
        .unwrap()
        .remove("x.ai/mcp/sdk");

    let authentication = protocol.receive(initialize).unwrap();
    assert_eq!(authentication.writes[0]["method"], "authenticate");
    assert!(
        !protocol
            .reported_initialize_extensions
            .local_mcp_sdk_advertised
    );
    assert!(protocol.receive(fixture_response(2)).is_err());
    assert!(protocol.session_id.is_none());
    assert!(protocol.sdk.as_ref().unwrap().ledger.is_none());
}

#[test]
fn sdk_reverse_request_before_capability_binding_is_rejected() {
    let mut launch = options();
    launch.local_tools = Some(LocalToolPermissions::default());
    let mut protocol = GrokProtocol::from_fixture(launch);
    protocol.sdk.as_mut().unwrap().owned_process = true;
    let request = catalog_registration(&protocol, 77, "server/discover");

    assert!(protocol.receive(request).is_err());
    assert!(
        !protocol
            .reported_initialize_extensions
            .local_mcp_sdk_advertised
    );
}

#[test]
fn missing_available_commands_is_recorded_without_enabling_extension_methods() {
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
    let mut initialize = fixture_response(1);
    initialize["result"]["_meta"]
        .as_object_mut()
        .unwrap()
        .remove("availableCommands");
    protocol.receive(initialize).unwrap();
    protocol.receive(fixture_response(2)).unwrap();
    let ready = protocol.receive(fixture_response(3)).unwrap();
    assert!(!protocol.reported_initialize_extensions.available_commands);
    assert!(protocol.reported_metadata.available_commands.is_empty());
    let [
        RuntimeEventKind::SessionReady {
            effective_permissions,
            ..
        },
    ] = ready.events.as_slice()
    else {
        panic!("缺少原生会话就绪");
    };
    assert!(
        effective_permissions["verifiedCapabilities"]
            .get("skills")
            .is_none()
    );
    assert_eq!(
        effective_permissions["verifiedCapabilities"]["steer"],
        false
    );
}

#[test]
fn out_of_order_notification_does_not_create_capabilities() {
    let mut protocol = GrokProtocol::new(options());
    protocol.bind_cli_version("grok 1.0.34").unwrap();
    protocol.initialize().unwrap();
    let notification = json!({"jsonrpc":"2.0","method":"_x.ai/queue/changed","params":{
        "sessionId":"old-native-session","entries":[],"_meta":{"eventId":"old-event"}
    }});

    let early = protocol.receive(notification.clone()).unwrap();
    assert!(early.writes.is_empty() && early.events.is_empty());
    assert_eq!(
        protocol.reported_initialize_extensions,
        super::ReportedInitializeExtensions::default()
    );
    assert!(protocol.session_id.is_none());
}

#[test]
fn stale_notification_cannot_replace_bound_capabilities() {
    let mut protocol = GrokProtocol::new(options());
    protocol.bind_cli_version("grok 1.0.34").unwrap();
    protocol.initialize().unwrap();
    let mut initialize = fixture_response(1);
    initialize["result"]["_meta"]["agentVersion"] = json!("1.0.34");
    protocol.receive(initialize).unwrap();
    let extensions = protocol.reported_initialize_extensions.clone();
    let notification = json!({"jsonrpc":"2.0","method":"_x.ai/queue/changed","params":{
        "sessionId":"old-native-session","entries":[],"_meta":{"eventId":"old-event"}
    }});
    let late = protocol.receive(notification).unwrap();
    assert!(late.writes.is_empty() && late.events.is_empty());
    assert_eq!(protocol.reported_initialize_extensions, extensions);
    assert!(protocol.session_id.is_none());
}

#[test]
fn resumed_connection_does_not_accept_a_handshake_from_another_supported_version() {
    let mut resumed = options();
    resumed.target = SessionTarget::Resume {
        native_session_id: "OFFLINE_SAVED_SESSION".into(),
    };
    let mut protocol = GrokProtocol::new(resumed);
    protocol.bind_cli_version("grok 1.0.34").unwrap();
    protocol.initialize().unwrap();
    assert!(protocol.receive(fixture_response(1)).is_err());
    assert_eq!(protocol.next_id, 1);
    assert!(protocol.session_id.is_none());
}

#[test]
fn latest_version_cancels_a_correlated_but_unverified_write_approval() {
    let (mut protocol, request) = pending_native_approval();
    protocol.probed_version = Some("1.0.34");
    let result = protocol.receive(request.clone()).unwrap();
    assert!(result.events.is_empty());
    assert_eq!(
        result.writes,
        vec![json!({"jsonrpc":"2.0","id":0,
        "result":{"outcome":{"outcome":"cancelled"}}})]
    );
    assert!(protocol.approvals.is_empty());
    let repeated = protocol.receive(request).unwrap();
    assert!(repeated.events.is_empty() && repeated.writes.is_empty());
}

#[test]
fn latest_version_opens_only_a_correlated_exact_read_approval() {
    let (mut protocol, mut request) = pending_native_approval();
    protocol.probed_version = Some("1.0.34");
    let call_id = request["params"]["toolCall"]["toolCallId"]
        .as_str()
        .unwrap()
        .to_owned();
    let tool = json!({
        "toolCallId": call_id,
        "kind": "read",
        "rawInput": {"variant":"ReadFile", "target_file":"verified.txt"},
        "_meta": {"x.ai/tool": {
            "version":1, "name":"read_file", "namespace":"grok_build", "read_only":true
        }}
    });
    assert!(super::verified_latest_read_tool(&tool));
    request["params"]["toolCall"] = tool;

    let effects = protocol.receive(request).unwrap();
    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::ApprovalRequested { approval_id, .. }] if approval_id == "grok:0"
    ));
    assert!(protocol.approvals.contains_key("grok:0"));
}

#[test]
fn latest_fixed_read_policy_keeps_the_exact_read_approval_contract() {
    let (mut protocol, mut request) = pending_native_approval();
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    protocol.options.cwd.clone_from(&cwd);
    protocol.options.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    protocol.options.grok_profile = Some(
        GrokCreationPolicyV1::compile(
            &cwd,
            "1.0.34",
            "9cd26b579840f0f5c9148a8059ad651904c08b41b7f2ef0b4ec04b9ba898844e".into(),
            "b".repeat(64),
            None,
            PermissionPolicy::GrokRestrictedReadV1,
        )
        .unwrap(),
    );
    protocol.probed_version = Some("1.0.34");
    let call_id = request["params"]["toolCall"]["toolCallId"]
        .as_str()
        .unwrap()
        .to_owned();
    request["params"]["toolCall"] = json!({
        "toolCallId": call_id,
        "kind": "read",
        "rawInput": {"variant":"ReadFile", "target_file":cwd.join("verified.txt")},
        "_meta": {"x.ai/tool": {
            "version":1, "name":"read_file", "namespace":"grok_build", "read_only":true
        }}
    });

    let effects = protocol.receive(request).unwrap();

    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::ApprovalRequested { approval_id, .. }] if approval_id == "grok:0"
    ));
    assert!(protocol.approvals.contains_key("grok:0"));
}

#[test]
fn latest_read_approval_rejects_unverified_schema_or_tool_event_provenance() {
    let verified = json!({
        "kind": "read",
        "rawInput": {"variant":"ReadFile", "target_file":"verified.txt"},
        "_meta": {"x.ai/tool": {
            "version":1, "name":"read_file", "namespace":"grok_build", "read_only":true
        }}
    });
    for (pointer, value) in [
        ("/kind", json!("edit")),
        ("/_meta/x.ai~1tool/name", json!("write")),
        ("/_meta/x.ai~1tool/namespace", json!("opencode")),
        ("/_meta/x.ai~1tool/read_only", json!(false)),
        ("/rawInput/variant", json!("Write")),
        ("/rawInput/offset", json!(2)),
    ] {
        let mut rejected = verified.clone();
        if pointer == "/rawInput/offset" {
            rejected["rawInput"]["offset"] = value;
        } else {
            *rejected.pointer_mut(pointer).unwrap() = value;
        }
        assert!(!super::verified_latest_read_tool(&rejected));
    }
    let mut extra = verified;
    extra["rawInput"]["content"] = json!("unverified");
    assert!(!super::verified_latest_read_tool(&extra));

    let (mut protocol, mut request) = pending_native_approval();
    protocol.probed_version = Some("1.0.34");
    request["params"]["toolCall"] = json!({
        "toolCallId": "unrelated-tool",
        "kind": "read",
        "rawInput": {"variant":"ReadFile", "target_file":"verified.txt"},
        "_meta": {"x.ai/tool": {
            "version":1, "name":"read_file", "namespace":"grok_build", "read_only":true
        }}
    });
    let effects = protocol.receive(request).unwrap();
    assert!(effects.events.is_empty());
    assert_eq!(
        effects.writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    assert!(protocol.approvals.is_empty());
}

#[test]
fn real_handshake_reports_verified_text_lifecycle_and_preserves_permission_limits() {
    let mut protocol = GrokProtocol::from_fixture(options());
    let initialize = protocol.initialize().unwrap();
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
            ..
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
        effective_permissions["reportedCapabilities"]["mcpCapabilities"],
        json!({"http":true,"sse":true})
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
    let mut initialize = fixture_response(1);
    initialize["result"]["authMethods"] = json!([
        {"id":"xai.api_key"}, {"id":"cached_token"}, {"id":"grok.com"}
    ]);
    let effects = protocol.receive(initialize).unwrap();
    assert_eq!(effects.writes[0]["params"]["methodId"], "cached_token");

    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
        let mut protocol = GrokProtocol::from_fixture(options());
        protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
    assert!(protocol.receive(fixture_response(3)).is_err());
    assert_eq!(protocol.pending.as_ref().unwrap().id, 1);
    assert!(protocol.session_id.is_none());
}

#[test]
fn unknown_string_response_cannot_replace_a_numeric_pending_request() {
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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

fn internal_skills_reload_fixture() -> Value {
    json!({"jsonrpc":"2.0","id":"skills-reload","result":{"result":{"reloaded":1}}})
}

#[test]
fn internal_skills_reload_accepts_u64_boundaries_without_advancing_user_rpc() {
    for count in [0, u64::MAX] {
        let mut protocol = GrokProtocol::from_fixture(options());
        protocol.initialize().unwrap();
        let context = protocol.transaction_context();
        let sent_at = protocol.pending.as_ref().unwrap().sent_at;
        let mut response = internal_skills_reload_fixture();
        response["result"]["result"]["reloaded"] = json!(count);

        let effects = protocol.receive(response).unwrap();
        assert!(effects.writes.is_empty() && effects.events.is_empty());
        assert_eq!(protocol.transaction_context(), context);
        assert_eq!(protocol.pending.as_ref().unwrap().sent_at, sent_at);
        assert!(protocol.responses.is_empty() && protocol.session_id.is_none());
        // 只有真实的数值响应才能推进握手。
        let authenticated = protocol.receive(fixture_response(1)).unwrap();
        assert_eq!(authenticated.writes[0]["method"], "authenticate");
        assert_eq!(protocol.pending.as_ref().unwrap().id, 2);
    }
}

#[test]
fn duplicate_internal_skills_reload_preserves_authentication_and_native_identity() {
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
    protocol.receive(fixture_response(1)).unwrap();
    let context = protocol.transaction_context();
    let responses = protocol.responses.clone();
    let capabilities = protocol.reported_capabilities.clone();
    let response = internal_skills_reload_fixture();

    for _ in 0..3 {
        let effects = protocol.receive(response.clone()).unwrap();
        assert!(effects.writes.is_empty() && effects.events.is_empty());
        assert_eq!(protocol.transaction_context(), context);
        assert_eq!(protocol.responses, responses);
        assert_eq!(protocol.reported_capabilities, capabilities);
        assert!(protocol.session_id.is_none() && protocol.observed_prompt_ids.is_empty());
        assert!(protocol.notification_ids.is_empty() && protocol.permission_requests.is_empty());
    }
    assert_eq!(
        protocol.receive(fixture_response(2)).unwrap().writes[0]["method"],
        "session/new"
    );
}

#[test]
fn internal_skills_reload_cannot_acknowledge_or_finish_an_active_prompt() {
    let (mut protocol, _) = pending_native_approval();
    let probe = super::sdk_origin_live_tests::SdkOriginProbe::new(protocol.options.generation);
    protocol.sdk_origin_probe = Some(probe.clone());
    let context = protocol.transaction_context();
    let native_prompt = protocol.prompt.as_ref().unwrap().native_id.clone();
    let output = protocol.prompt.as_ref().unwrap().output.clone();
    let responses = protocol.responses.clone();
    let notification_ids = protocol.notification_ids.clone();
    let permission_requests = protocol.permission_requests.clone();
    let tools = protocol.tools.len();
    let approvals = protocol.approvals.len();

    for _ in 0..2 {
        let effects = protocol.receive(internal_skills_reload_fixture()).unwrap();
        assert!(effects.writes.is_empty() && effects.events.is_empty());
        assert_eq!(protocol.transaction_context(), context);
        assert_eq!(protocol.responses, responses);
        assert_eq!(protocol.notification_ids, notification_ids);
        assert_eq!(protocol.permission_requests, permission_requests);
        assert_eq!(protocol.tools.len(), tools);
        assert_eq!(protocol.approvals.len(), approvals);
        let prompt = protocol.prompt.as_ref().unwrap();
        assert_eq!(prompt.native_id, native_prompt);
        assert_eq!(prompt.output, output);
        assert!(!prompt.finished && prompt.completion.is_none());
        assert!(
            protocol.options.permission_ceiling.is_none() && protocol.options.local_tools.is_none()
        );
    }
    let report = probe.report(None, None);
    assert_eq!(report["sdk_request_count"], 0);
    assert_eq!(report["inspect_call_count"], 0);
    assert_eq!(report["permission_request_count"], 0);
    assert_eq!(report["origin_verification"], "unknown");
}

#[test]
fn internal_skills_reload_rejects_non_u64_counts_and_malformed_wrappers() {
    let mut rejected = Vec::new();
    for count in [
        json!(-1),
        json!(1.0),
        json!(true),
        json!("1"),
        Value::Null,
        json!([]),
        json!({"body":"OFFLINE_PRIVATE_MAINTENANCE_BODY"}),
        serde_json::from_str::<Value>("18446744073709551616").unwrap(),
    ] {
        let mut response = internal_skills_reload_fixture();
        response["result"]["result"]["reloaded"] = count;
        rejected.push(response);
    }
    rejected.extend([
        json!({"jsonrpc":"2.0","id":"skills-reload","result":{"reloaded":1}}),
        json!({"jsonrpc":"2.0","id":"skills-reload","result":{"result":{}}}),
        json!({"jsonrpc":"2.0","id":"skills-reload","result":{"result":null}}),
        json!({"jsonrpc":"2.0","id":"skills-reload","result":null}),
        json!({"jsonrpc":"2.0","id":"skills-reload"}),
    ]);
    for response in rejected {
        let mut protocol = GrokProtocol::from_fixture(options());
        protocol.initialize().unwrap();
        let error = protocol.receive(response).err().unwrap();
        assert_eq!(
            super::runtime_error_diagnostic(&error)["protocol_failure_kind"],
            "invalid_response_id"
        );
        assert_eq!(protocol.pending.as_ref().unwrap().id, 1);
        assert!(protocol.responses.is_empty());
    }
}

#[test]
fn internal_skills_reload_rejects_foreign_ids_errors_and_injected_identity() {
    let mut rejected = Vec::new();
    for id in [
        json!("OFFLINE_UNKNOWN_RESPONSE_ID"),
        json!("skills-reload "),
        Value::Null,
        json!(true),
    ] {
        let mut response = internal_skills_reload_fixture();
        response["id"] = id;
        rejected.push(response);
    }
    for error in [
        Value::Null,
        json!({"code":-32603,"message":"OFFLINE_PRIVATE_MAINTENANCE_ERROR"}),
    ] {
        let mut response = internal_skills_reload_fixture();
        response["error"] = error;
        rejected.push(response);
    }
    for field in [
        "sessionId",
        "promptId",
        "toolCallId",
        "eventId",
        "generation",
        "_meta",
        "responseId",
    ] {
        for path in ["", "/result", "/result/result"] {
            let mut response = internal_skills_reload_fixture();
            response.pointer_mut(path).unwrap()[field] =
                json!("OFFLINE_FABRICATED_NATIVE_IDENTITY");
            rejected.push(response);
        }
    }
    for response in rejected {
        let mut protocol = GrokProtocol::from_fixture(options());
        protocol.initialize().unwrap();
        let context = protocol.transaction_context();
        let error = protocol.receive(response).err().unwrap();
        assert_eq!(
            super::runtime_error_diagnostic(&error)["protocol_failure_kind"],
            "invalid_response_id"
        );
        assert_eq!(protocol.transaction_context(), context);
        assert!(protocol.responses.is_empty() && protocol.session_id.is_none());
    }
}

#[test]
fn mixed_internal_skills_reload_frame_cannot_become_a_client_tool_request() {
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
    let mut response = internal_skills_reload_fixture();
    response["method"] = json!("session/request_permission");
    response["params"] = json!({"sessionId":"OFFLINE_FABRICATED_SESSION"});

    assert!(protocol.receive(response).is_err());
    assert_eq!(protocol.pending.as_ref().unwrap().id, 1);
    assert!(protocol.responses.is_empty() && protocol.approvals.is_empty());
    assert!(protocol.permission_requests.is_empty() && protocol.tools.is_empty());
}

#[tokio::test]
async fn outbound_diagnostic_records_written_identity_without_parameters() {
    let mut protocol = GrokProtocol::from_fixture(options());
    let probe = super::sdk_origin_live_tests::SdkOriginProbe::new(protocol.options.generation);
    protocol.sdk_origin_probe = Some(probe.clone());
    let request = protocol.request(
        super::PendingKind::Prompt,
        "session/prompt",
        json!({"prompt":"OFFLINE_PRIVATE_PROMPT","sessionId":"OFFLINE_PRIVATE_SESSION"}),
    );
    assert!(!protocol.task_input_written);
    let (sender, _) = mpsc::channel(4);
    let mut stdin = Cursor::new(Vec::new());
    flush_effects(
        &mut protocol,
        &mut stdin,
        &sender,
        super::Effects {
            writes: vec![request.clone()],
            events: Vec::new(),
        },
    )
    .await
    .unwrap();

    assert!(protocol.task_input_written);
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
    let mut protocol = GrokProtocol::from_fixture(options());
    let probe = super::sdk_origin_live_tests::SdkOriginProbe::new(protocol.options.generation);
    protocol.sdk_origin_probe = Some(probe.clone());
    let request = protocol.initialize().unwrap();
    let (sender, _) = mpsc::channel(4);
    let mut storage = [];
    let mut stdin = Cursor::new(&mut storage[..]);
    assert!(
        flush_effects(
            &mut protocol,
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
    assert!(!protocol.task_input_written);
    assert_eq!(protocol.pending.as_ref().unwrap().id, 1);
}

#[tokio::test]
async fn task_input_evidence_requires_a_successful_prompt_write() {
    let mut protocol = ready_protocol();
    let first = internal_submit(&mut protocol, Uuid::new_v4(), "first offline input");
    assert_eq!(first.writes.len(), 1);
    assert_eq!(first.writes[0]["method"], "session/prompt");
    assert!(!protocol.task_input_written);
    let queued = internal_submit(&mut protocol, Uuid::new_v4(), "queued offline input");
    assert!(queued.writes.is_empty());
    assert_eq!(protocol.queued.len(), 1);
    assert!(!protocol.task_input_written);
    let (sender, _receiver) = mpsc::channel(4);
    let mut storage = [];
    let mut failed_stdin = Cursor::new(&mut storage[..]);
    assert!(
        flush_effects(&mut protocol, &mut failed_stdin, &sender, first)
            .await
            .is_err()
    );
    assert!(!protocol.task_input_written);

    let mut protocol = GrokProtocol::from_fixture(options());
    let initialize = protocol.initialize().unwrap();
    let mut stdin = Cursor::new(Vec::new());
    flush_effects(
        &mut protocol,
        &mut stdin,
        &sender,
        super::Effects {
            writes: vec![initialize],
            events: Vec::new(),
        },
    )
    .await
    .unwrap();
    assert!(!protocol.task_input_written);
}

#[tokio::test]
async fn refreshed_catalog_rejection_carries_successful_write_evidence() {
    let mut protocol = ready_protocol();
    let effects = internal_submit(&mut protocol, Uuid::new_v4(), "offline input");
    let (sender, _receiver) = mpsc::channel(8);
    let mut stdin = Cursor::new(Vec::new());
    flush_effects(&mut protocol, &mut stdin, &sender, effects)
        .await
        .unwrap();
    assert!(protocol.task_input_written);
    // 尚未收到原生输入确认，目录错误仍应准确反映本代次已经完成写入。
    protocol.options.grok_profile = Some(
        super::super::permissions::GrokCreationPolicyV1::compile(
            &protocol.options.cwd.canonicalize().unwrap(),
            "1.0.30",
            "a".repeat(64),
            "b".repeat(64),
            None,
            PermissionPolicy::GrokRestrictedReadV1,
        )
        .unwrap(),
    );
    let error = protocol
        .receive(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":protocol.session_id,"update":{"sessionUpdate":"available_commands_update",
            "_meta":{"tools":["read_file","OFFLINE_PRIVATE_UNKNOWN"]}}
        }}))
        .err()
        .unwrap();
    let evidence = error.permission_ceiling_evidence().unwrap();
    assert_eq!(evidence["reason"], "grok_creation_catalog_changed");
    assert_eq!(evidence["task_input_sent"], true);
    assert!(!evidence.to_string().contains("OFFLINE_PRIVATE_UNKNOWN"));
}

#[test]
fn transaction_context_does_not_expose_history_session_or_recovery_arguments() {
    let mut protocol = GrokProtocol::from_fixture(options());
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options);
    protocol.initialize().unwrap();
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
        let mut protocol = GrokProtocol::from_fixture(options);
        protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
        let mut protocol = GrokProtocol::from_fixture(options);
        protocol.initialize().unwrap();
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
    let mut protocol = GrokProtocol::from_fixture(options);
    protocol.initialize().unwrap();
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
        flush_effects(&mut protocol, &mut stdin, &sender, effects)
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
        flush_effects(&mut protocol, &mut stdin, &sender, effects)
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
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
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
    flush_effects(&mut protocol, &mut stdin, &sender, effects)
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
fn root_adapter_accepts_leased_tools_but_rejects_unknown_parent_ceiling_before_spawn() {
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
    assert!(validate_options(&launch).is_ok());
    launch.local_tools = None;
    assert!(validate_options(&launch).is_ok());
}

// 下列帧为离线协议测试，不计作真实 CLI 完成证据。
fn leased_protocol() -> GrokProtocol {
    let mut launch = options();
    launch.local_tools = Some(LocalToolPermissions {
        allow_spawn: true,
        allow_message: true,
    });
    let mut protocol = GrokProtocol::from_fixture(launch);
    protocol.sdk.as_mut().unwrap().owned_process = true;
    protocol.initialize().unwrap();
    protocol.receive(fixture_response(1)).unwrap();
    protocol.receive(fixture_response(2)).unwrap();
    protocol.receive(fixture_response(3)).unwrap();
    protocol
}

fn leased_begin(protocol: &mut GrokProtocol, turn: &str) {
    internal_submit(protocol, Uuid::new_v4(), "检查任务状态");
    let session = protocol.session_id.clone();
    protocol
        .receive(
            json!({"jsonrpc":"2.0","method":"_x.ai/queue/changed","params":{
        "sessionId":session,"entries":[{"kind":"prompt","id":turn}]}}),
        )
        .unwrap();
    protocol
        .receive(
            json!({"jsonrpc":"2.0","method":"_x.ai/queue/changed","params":{
        "sessionId":session,"entries":[],"runningPromptId":turn}}),
        )
        .unwrap();
    assert!(protocol.prompt.as_ref().unwrap().started);
}

fn leased_frame(protocol: &GrokProtocol, turn: &str, event: &str, update: Value) -> Value {
    json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":protocol.session_id,"_meta":{"promptId":turn,"eventId":event},"update":update}})
}

fn leased_inputs(protocol: &mut GrokProtocol, turn: &str, call: &str) {
    let initial = leased_frame(
        protocol,
        turn,
        &format!("{call}-initial"),
        json!({
        "sessionUpdate":"tool_call","toolCallId":call,"_meta":{"x.ai/tool":{"name":"use_tool"}},
        "rawInput":{"tool_name":"infinishell-local-tasks__inspect_local_tasks","tool_input":{}}}),
    );
    protocol.receive(initial).unwrap();
    let final_input = leased_frame(
        protocol,
        turn,
        &format!("{call}-final"),
        json!({
        "sessionUpdate":"tool_call_update","toolCallId":call,"kind":"other",
        "rawInput":{"tool_name":"infinishell-local-tasks__inspect_local_tasks","tool_input":{},"variant":"UseTool"}}),
    );
    protocol.receive(final_input).unwrap();
}

fn leased_permission(protocol: &GrokProtocol, call: &str, id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"session/request_permission","params":{
        "sessionId":protocol.session_id,"toolCall":{"toolCallId":call,"kind":"other",
        "rawInput":{"tool_name":"infinishell-local-tasks__inspect_local_tasks","tool_input":{},"variant":"UseTool"}},
        "options":[{"optionId":"allow-once","kind":"allow_once"},{"optionId":"reject-once","kind":"reject_once"}]}})
}

fn leased_sdk(protocol: &GrokProtocol, outer: u64, inner: u64) -> Value {
    json!({"jsonrpc":"2.0","id":outer,"method":"_x.ai/mcp/sdk_call","params":{
        "serverId":protocol.sdk.as_ref().unwrap().bridge.server_id(),"message":{"jsonrpc":"2.0","id":inner,
        "method":"tools/call","params":{"name":"inspect_local_tasks","arguments":{}}}}})
}

async fn leased_flush(protocol: &mut GrokProtocol, effects: super::Effects) {
    let (sender, _receiver) = mpsc::channel(32);
    flush_effects(protocol, &mut Cursor::new(Vec::new()), &sender, effects)
        .await
        .unwrap();
}

async fn leased_allow(protocol: &mut GrokProtocol, call: &str, approval: u64) {
    let permission = leased_permission(protocol, call, approval);
    protocol.receive(permission).unwrap();
    let effects = internal_action(
        protocol,
        Uuid::new_v4(),
        RuntimeAction::RespondApproval {
            approval_id: format!("grok:{approval}"),
            decision: ApprovalDecision::AllowOnce,
        },
    );
    assert_eq!(effects.writes.len(), 1);
    leased_flush(protocol, effects).await;
}

fn leased_request(
    protocol: &mut GrokProtocol,
    outer: u64,
    inner: u64,
) -> crate::ai::cli_agent_runtime::local_tools::NativeLocalToolRequest {
    let sdk = leased_sdk(protocol, outer, inner);
    let effects = protocol.receive(sdk).unwrap();
    let [RuntimeEventKind::LocalToolRequested { request }] = effects.events.as_slice() else {
        panic!("租约应只派发一次本地工具");
    };
    request.clone()
}

#[test]
fn production_sdk_registration_uses_process_nonce_and_never_advertises_spawn() {
    let mut launch = options();
    launch.local_tools = Some(LocalToolPermissions {
        allow_spawn: true,
        allow_message: true,
    });
    let mut first = GrokProtocol::from_fixture(launch.clone());
    let second = GrokProtocol::from_fixture(launch);
    assert_ne!(
        first.sdk.as_ref().unwrap().bridge.server_id(),
        second.sdk.as_ref().unwrap().bridge.server_id()
    );
    let initialize = first.initialize().unwrap();
    assert_eq!(
        initialize["params"]["clientCapabilities"]["_meta"]["x.ai/mcp/sdk"],
        true
    );
    first.sdk.as_mut().unwrap().owned_process = true;
    first.receive(fixture_response(1)).unwrap();
    let open = first.receive(fixture_response(2)).unwrap();
    assert_eq!(open.writes[0]["params"]["mcpServers"], json!([]));
    assert_eq!(
        open.writes[0]["params"]["_meta"]["x.ai/mcp/servers"],
        first.sdk.as_ref().unwrap().bridge.registration()
    );
    let ready = first.receive(fixture_response(3)).unwrap();
    let [
        RuntimeEventKind::SessionReady {
            effective_permissions,
            ..
        },
    ] = ready.events.as_slice()
    else {
        panic!("缺少原生会话就绪");
    };
    assert_eq!(
        effective_permissions["verifiedCapabilities"]["localTools"],
        true
    );
    assert_eq!(
        effective_permissions["verifiedCapabilities"]["childTasks"],
        false
    );
    assert_eq!(
        effective_permissions["permissionEnforcementVerified"],
        false
    );
    let mut request = leased_sdk(&first, 101, 101);
    request["params"]["message"]["method"] = json!("tools/list");
    request["params"]["message"]["params"] = json!({});
    let response = first.receive(request).unwrap();
    let names = response.writes[0]["result"]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["inspect_local_tasks", "send_message_to_agent"]);
}

#[test]
fn production_sdk_without_owned_process_cannot_register_or_open_session() {
    let mut launch = options();
    launch.local_tools = Some(LocalToolPermissions::default());
    let mut protocol = GrokProtocol::from_fixture(launch);
    protocol.initialize().unwrap();
    protocol.receive(fixture_response(1)).unwrap();
    protocol.receive(fixture_response(2)).unwrap();
    assert!(protocol.receive(fixture_response(3)).is_err());
    let mut registration = leased_sdk(&protocol, 80, 80);
    registration["params"]["message"]["method"] = json!("tools/list");
    registration["params"]["message"]["params"] = json!({});
    assert!(protocol.receive(registration).is_err());
}

#[tokio::test]
async fn production_lease_requires_written_approval_and_dispatches_duplicate_rpc_once() {
    let mut protocol = leased_protocol();
    leased_begin(&mut protocol, "lease-turn");
    leased_inputs(&mut protocol, "lease-turn", "lease-call");
    let permission = leased_permission(&protocol, "lease-call", 50);
    protocol.receive(permission).unwrap();
    let approval = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::RespondApproval {
            approval_id: "grok:50".into(),
            decision: ApprovalDecision::AllowOnce,
        },
    );
    let sdk = leased_sdk(&protocol, 90, 2);
    assert!(protocol.receive(sdk.clone()).is_err());
    leased_flush(&mut protocol, approval).await;
    let effects = protocol.receive(sdk.clone()).unwrap();
    assert!(
        matches!(effects.events.as_slice(), [RuntimeEventKind::LocalToolRequested { request }] if request.turn_id == "lease-turn")
    );
    assert!(protocol.receive(sdk).unwrap().events.is_empty());
    let retry = leased_sdk(&protocol, 91, 2);
    assert!(protocol.receive(retry).unwrap().events.is_empty());
    assert_eq!(protocol.sdk.as_ref().unwrap().calls.len(), 1);
}

#[test]
fn production_lease_rejects_changed_approval_before_any_allow_is_written() {
    let mut protocol = leased_protocol();
    leased_begin(&mut protocol, "lease-turn");
    leased_inputs(&mut protocol, "lease-turn", "lease-call");
    let mut permission = leased_permission(&protocol, "lease-call", 50);
    permission["params"]["toolCall"]["rawInput"]["tool_input"] = json!({"task_ids":["other"]});
    let rejected = protocol.receive(permission).unwrap();
    assert_eq!(
        rejected.writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    assert!(rejected.events.is_empty());
    assert!(protocol.approvals.is_empty());
    assert!(
        protocol
            .sdk
            .as_ref()
            .unwrap()
            .permissions_to_write
            .is_empty()
    );
    let sdk = leased_sdk(&protocol, 90, 2);
    assert!(protocol.receive(sdk).is_err());
}

#[tokio::test]
async fn production_lease_failed_reply_write_preserves_bound_state_and_emits_no_receipt() {
    let mut protocol = leased_protocol();
    leased_begin(&mut protocol, "lease-turn");
    leased_inputs(&mut protocol, "lease-turn", "lease-call");
    leased_allow(&mut protocol, "lease-call", 50).await;
    let request = leased_request(&mut protocol, 90, 2);
    let effects = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::RespondLocalTool {
            turn_id: request.turn_id.clone(),
            call_id: request.call_id.clone(),
            result: Ok(json!({"tasks":[]})),
        },
    );
    let (sender, mut events) = mpsc::channel(8);
    let mut storage = [];
    let mut writer = Cursor::new(&mut storage[..]);
    assert!(
        flush_effects(&mut protocol, &mut writer, &sender, effects)
            .await
            .is_err()
    );
    assert!(events.try_recv().is_err());
    let sdk = protocol.sdk.as_ref().unwrap();
    assert_eq!(
        sdk.ledger
            .as_ref()
            .unwrap()
            .state(&sdk.calls[&request.call_id].proof)
            .unwrap(),
        super::super::grok_tool_lease::GrokToolLeaseState::Bound
    );
    let cleanup = protocol.finish_transport(&Err(RuntimeError::RequestTimedOut));
    assert!(matches!(
        cleanup.events.as_slice(),
        [RuntimeEventKind::LocalToolCancelled { .. }]
    ));
    assert!(protocol.sdk.as_ref().unwrap().retired);
}

#[tokio::test]
async fn production_lease_native_completion_and_written_reply_enable_next_turn_without_rebinding_old_rpc()
 {
    let mut protocol = leased_protocol();
    leased_begin(&mut protocol, "first-turn");
    leased_inputs(&mut protocol, "first-turn", "first-call");
    leased_allow(&mut protocol, "first-call", 50).await;
    let old_rpc = leased_sdk(&protocol, 90, 2);
    let request = leased_request(&mut protocol, 90, 2);
    let reply = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::RespondLocalTool {
            turn_id: request.turn_id.clone(),
            call_id: request.call_id.clone(),
            result: Ok(json!({"tasks":[]})),
        },
    );
    leased_flush(&mut protocol, reply).await;
    let complete = leased_frame(
        &protocol,
        "first-turn",
        "first-complete",
        json!({"sessionUpdate":"tool_call_update","toolCallId":"first-call","status":"completed"}),
    );
    protocol.receive(complete).unwrap();
    let prompt_id = protocol.pending.as_ref().unwrap().id;
    finish_with_unit_history(
        &mut protocol,
        json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn","_meta":{"promptId":"first-turn"}}}),
    );
    leased_begin(&mut protocol, "second-turn");
    leased_inputs(&mut protocol, "second-turn", "second-call");
    leased_allow(&mut protocol, "second-call", 51).await;
    let replay = protocol.receive(old_rpc).unwrap();
    assert!(replay.events.is_empty());
    assert_eq!(replay.writes.len(), 1);
    leased_flush(&mut protocol, replay).await;
    let second = leased_request(&mut protocol, 91, 3);
    assert_ne!(request.call_id, second.call_id);
    assert_eq!(second.turn_id, "second-turn");
    let sdk = protocol.sdk.as_ref().unwrap();
    assert_eq!(
        sdk.calls[&request.call_id].proof.native_call_id(),
        "first-call"
    );
    assert_eq!(
        sdk.calls[&second.call_id].proof.native_call_id(),
        "second-call"
    );
}

#[tokio::test]
async fn production_lease_cancel_retires_callbacks_and_waits_for_native_cancelled_history() {
    let mut protocol = leased_protocol();
    leased_begin(&mut protocol, "lease-turn");
    leased_inputs(&mut protocol, "lease-turn", "lease-call");
    leased_allow(&mut protocol, "lease-call", 50).await;
    let request = leased_request(&mut protocol, 90, 2);
    let queued = Uuid::new_v4();
    internal_submit(&mut protocol, queued, "不得重投");
    let cancelled = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::Interrupt {
            turn_id: "lease-turn".into(),
        },
    );
    assert_eq!(cancelled.writes[0]["method"], "session/cancel");
    assert!(
        !cancelled
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEventKind::TurnFinished { .. }))
    );
    assert!(protocol.sdk.as_ref().unwrap().retired);
    let late = leased_sdk(&protocol, 91, 3);
    assert!(protocol.receive(late).is_err());
    let reply = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::RespondLocalTool {
            turn_id: request.turn_id,
            call_id: request.call_id,
            result: Ok(json!({"tasks":[]})),
        },
    );
    assert!(reply.writes.is_empty());
    assert!(matches!(
        reply.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    let prompt_id = protocol.pending.as_ref().unwrap().id;
    let final_events = finish_with_unit_history(
        &mut protocol,
        json!({"jsonrpc":"2.0","id":prompt_id,"result":{
        "stopReason":"cancelled","_meta":{"promptId":"lease-turn","cancellationCategory":"MidTurnAbort"}}}),
    );
    assert!(protocol.closed);
    assert!(final_events.events.iter().any(|event| matches!(
        event,
        RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Cancelled,
            ..
        }
    )));
    assert!(final_events.events.iter().any(|event| matches!(event,RuntimeEventKind::RequestFailed {message_id,..} if *message_id == queued)));
    assert!(final_events.writes.is_empty());
}

#[tokio::test]
async fn production_lease_permission_rejection_does_not_invent_task_cancellation() {
    let mut protocol = leased_protocol();
    leased_begin(&mut protocol, "lease-turn");
    leased_inputs(&mut protocol, "lease-turn", "lease-call");
    let permission = leased_permission(&protocol, "lease-call", 50);
    protocol.receive(permission).unwrap();
    let denied = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::RespondApproval {
            approval_id: "grok:50".into(),
            decision: ApprovalDecision::DenyOnce,
        },
    );
    leased_flush(&mut protocol, denied).await;
    assert!(protocol.sdk.as_ref().unwrap().retired);
    assert!(!protocol.prompt.as_ref().unwrap().finished);
    assert!(!protocol.closed);
    assert!(protocol.sdk_timed_out(Instant::now() + REQUEST_TIMEOUT + Duration::from_secs(1)));
    let callback = leased_sdk(&protocol, 90, 2);
    assert!(protocol.receive(callback).is_err());
}

#[tokio::test]
async fn production_lease_approved_callback_timeout_and_old_tool_frame_cannot_become_success() {
    let mut protocol = leased_protocol();
    leased_begin(&mut protocol, "lease-turn");
    leased_inputs(&mut protocol, "lease-turn", "lease-call");
    leased_allow(&mut protocol, "lease-call", 50).await;
    assert!(protocol.sdk_timed_out(Instant::now() + REQUEST_TIMEOUT + Duration::from_secs(1)));
    let old = leased_frame(
        &protocol,
        "old-turn",
        "old-event",
        json!({"sessionUpdate":"tool_call_update","toolCallId":"lease-call","status":"completed"}),
    );
    assert!(protocol.receive(old).unwrap().events.is_empty());
    assert!(!protocol.tools["lease-call"].finished);
    let cleanup = protocol.finish_transport(&Err(RuntimeError::RequestTimedOut));
    assert!(
        !cleanup
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEventKind::TurnFinished { .. }))
    );
    assert!(protocol.sdk.as_ref().unwrap().retired);
}

#[tokio::test]
async fn production_lease_written_reply_has_a_bounded_native_completion_wait() {
    let mut protocol = leased_protocol();
    leased_begin(&mut protocol, "lease-turn");
    leased_inputs(&mut protocol, "lease-turn", "lease-call");
    leased_allow(&mut protocol, "lease-call", 50).await;
    let request = leased_request(&mut protocol, 90, 2);
    let reply = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::RespondLocalTool {
            turn_id: request.turn_id.clone(),
            call_id: request.call_id.clone(),
            result: Ok(json!({"tasks":[]})),
        },
    );

    // 生成和缓存结果不代表回写，不能提前启动原生完成期限。
    assert!(
        protocol
            .sdk
            .as_ref()
            .unwrap()
            .native_completion_deadlines
            .is_empty()
    );
    assert!(!protocol.sdk_timed_out(Instant::now() + REQUEST_TIMEOUT + Duration::from_secs(1)));
    leased_flush(&mut protocol, reply).await;
    let deadline = protocol.sdk.as_ref().unwrap().native_completion_deadlines["lease-call"].1;
    assert!(!protocol.sdk_timed_out(deadline - Duration::from_millis(1)));
    assert!(protocol.sdk_timed_out(deadline));
    assert!(!protocol.prompt.as_ref().unwrap().finished);

    // 原生重投只重发缓存，不能刷新等待真实工具完成的期限。
    let replay = leased_sdk(&protocol, 91, 2);
    let cached = protocol.receive(replay).unwrap();
    leased_flush(&mut protocol, cached).await;
    assert_eq!(
        protocol.sdk.as_ref().unwrap().native_completion_deadlines["lease-call"].1,
        deadline
    );
    let completed = leased_frame(
        &protocol,
        "lease-turn",
        "lease-completed",
        json!({
        "sessionUpdate":"tool_call_update","toolCallId":"lease-call","status":"completed"}),
    );
    let effects = protocol.receive(completed).unwrap();
    assert!(effects.events.is_empty());
    assert!(protocol.sdk.as_ref().unwrap().calls[&request.call_id].closed);
    assert!(
        protocol
            .sdk
            .as_ref()
            .unwrap()
            .native_completion_deadlines
            .is_empty()
    );
    assert!(!protocol.sdk_timed_out(deadline + Duration::from_secs(1)));
    assert!(!protocol.prompt.as_ref().unwrap().finished);
    let closed_replay = leased_sdk(&protocol, 92, 2);
    let cached = protocol.receive(closed_replay).unwrap();
    leased_flush(&mut protocol, cached).await;
    assert!(
        protocol
            .sdk
            .as_ref()
            .unwrap()
            .native_completion_deadlines
            .is_empty()
    );
}

#[tokio::test]
async fn production_lease_immediate_error_reply_also_bounds_native_completion_wait() {
    let mut protocol = leased_protocol();
    leased_begin(&mut protocol, "lease-turn");
    leased_inputs(&mut protocol, "lease-turn", "lease-call");
    leased_allow(&mut protocol, "lease-call", 50).await;
    let mut request = leased_sdk(&protocol, 90, 2);
    // 合法原生租约仍可能收到不支持的 MCP 版本；错误回复没有应用工具调用对象。
    request["params"]["message"]["params"]["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion":"unsupported-version",
        "io.modelcontextprotocol/clientCapabilities":{}});
    let error_reply = protocol.receive(request.clone()).unwrap();
    assert!(error_reply.events.is_empty());
    assert_eq!(error_reply.writes.len(), 1);
    assert!(error_reply.writes[0]["result"]["error"].is_object());
    assert!(protocol.sdk.as_ref().unwrap().calls.is_empty());
    assert!(
        protocol
            .sdk
            .as_ref()
            .unwrap()
            .native_completion_deadlines
            .is_empty()
    );
    leased_flush(&mut protocol, error_reply).await;
    let deadline = protocol.sdk.as_ref().unwrap().native_completion_deadlines["lease-call"].1;
    assert!(!protocol.sdk_timed_out(deadline - Duration::from_millis(1)));
    assert!(protocol.sdk_timed_out(deadline));

    request["id"] = json!(91);
    let cached = protocol.receive(request.clone()).unwrap();
    leased_flush(&mut protocol, cached).await;
    assert_eq!(
        protocol.sdk.as_ref().unwrap().native_completion_deadlines["lease-call"].1,
        deadline
    );
    let completed = leased_frame(
        &protocol,
        "lease-turn",
        "lease-completed",
        json!({
        "sessionUpdate":"tool_call_update","toolCallId":"lease-call","status":"completed"}),
    );
    assert!(protocol.receive(completed).unwrap().events.is_empty());
    assert!(
        protocol
            .sdk
            .as_ref()
            .unwrap()
            .native_completion_deadlines
            .is_empty()
    );
    assert!(!protocol.sdk_timed_out(deadline + Duration::from_secs(1)));
    request["id"] = json!(92);
    let cached = protocol.receive(request).unwrap();
    leased_flush(&mut protocol, cached).await;
    assert!(
        protocol
            .sdk
            .as_ref()
            .unwrap()
            .native_completion_deadlines
            .is_empty()
    );
    assert!(!protocol.prompt.as_ref().unwrap().finished);
}

#[test]
fn fixed_grok_creation_does_not_convert_unknown_history_to_a_restricted_task() {
    let mut launch = options();
    launch.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    assert!(validate_options(&launch).is_ok());
    launch.target = SessionTarget::Resume {
        native_session_id: "existing-native-session".into(),
    };
    assert!(validate_options(&launch).is_err());
    launch.target = SessionTarget::New;
    launch.permission_policy = PermissionPolicy::Inherit;
    launch.local_tools = Some(LocalToolPermissions {
        allow_spawn: true,
        allow_message: true,
    });
    assert!(validate_options(&launch).is_err());
}

#[test]
fn fixed_grok_prompt_cannot_toggle_native_approval_mode() {
    let mut launch = options();
    launch.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    let mut protocol = GrokProtocol::from_fixture(launch);
    let effects = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text(" /always-approve on".into())],
        },
    });
    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(protocol.submitted_messages.is_empty());
}

#[test]
fn fixed_grok_waits_for_a_matching_native_tool_catalog_before_ready() {
    let directory = tempfile::tempdir().unwrap();
    let mut launch = options();
    launch.cwd = directory.path().canonicalize().unwrap();
    launch.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    launch.grok_profile = Some(
        super::super::permissions::GrokCreationPolicyV1::compile(
            &launch.cwd,
            "1.0.30",
            "a".repeat(64),
            "b".repeat(64),
            None,
            PermissionPolicy::GrokRestrictedReadV1,
        )
        .unwrap(),
    );
    let mut protocol = GrokProtocol::from_fixture(launch);
    protocol.initialize().unwrap();
    protocol.receive(fixture_response(1)).unwrap();
    protocol.receive(fixture_response(2)).unwrap();
    let response = protocol.receive(fixture_response(3)).unwrap();
    assert!(response.events.is_empty());
    assert!(protocol.deferred_ready.is_some());
    assert!(!protocol.task_input_written);
    let error = protocol.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":protocol.session_id,"update":{"sessionUpdate":"available_commands_update","_meta":{}}
    }})).err().unwrap();
    let evidence = error.permission_ceiling_evidence().unwrap();
    assert_eq!(evidence["reason"], "grok_creation_catalog_missing");
    assert_eq!(evidence["actual"]["catalog"]["value_type"], "missing");
    assert_eq!(evidence["task_input_sent"], false);
    assert!(protocol.deferred_ready.is_some());
    let ready = protocol.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":protocol.session_id,"update":{"sessionUpdate":"available_commands_update","_meta":{"tools":["read_file"]}}}})).unwrap();
    assert!(matches!(
        ready.events.as_slice(),
        [RuntimeEventKind::SessionReady { .. }]
    ));
    assert!(protocol.deferred_ready.is_none());
    assert!(!protocol.request_timed_out());
    assert!(protocol.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":protocol.session_id,"update":{"sessionUpdate":"available_commands_update","_meta":{"tools":["read_file","run_terminal_command"]}}}})).is_err());
    protocol.deferred_ready = Some((Value::Null, Instant::now() - REQUEST_TIMEOUT));
    assert!(protocol.request_timed_out());
    assert!(matches!(protocol.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":protocol.session_id,"update":{"sessionUpdate":"available_commands_update","_meta":{"tools":["read_file"]}}}})), Err(RuntimeError::RequestTimedOut)));
}

#[test]
fn fixed_file_policy_is_saved_before_ready_and_cannot_resume_as_read_only() {
    let directory = tempfile::tempdir().unwrap();
    let mut launch = options();
    launch.cwd = directory.path().canonicalize().unwrap();
    launch.permission_policy = PermissionPolicy::GrokRestrictedFilesV1;
    launch.grok_profile = Some(
        super::super::permissions::GrokCreationPolicyV1::compile(
            &launch.cwd,
            "1.0.30",
            "a".repeat(64),
            "b".repeat(64),
            None,
            PermissionPolicy::GrokRestrictedFilesV1,
        )
        .unwrap(),
    );
    assert!(validate_options(&launch).is_ok());
    let mut resumed = launch.clone();
    resumed.target = SessionTarget::Resume {
        native_session_id: "saved-file-session".into(),
    };
    assert!(validate_options(&resumed).is_ok());
    resumed.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    assert!(validate_options(&resumed).is_err());
    let mut unknown = launch.clone();
    unknown.target = resumed.target;
    unknown.grok_profile = None;
    assert!(validate_options(&unknown).is_err());
    let mut protocol = GrokProtocol::from_fixture(launch);
    protocol.initialize().unwrap();
    protocol.receive(fixture_response(1)).unwrap();
    protocol.receive(fixture_response(2)).unwrap();
    assert!(
        protocol
            .receive(fixture_response(3))
            .unwrap()
            .events
            .is_empty()
    );
    let ready = protocol
        .receive(json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":protocol.session_id,"update":{"sessionUpdate":"available_commands_update",
        "_meta":{"tools":["read_file","write","search_replace"]}}}}))
        .unwrap();
    let [
        RuntimeEventKind::SessionReady {
            effective_permissions,
            verified_cli_version,
        },
    ] = ready.events.as_slice()
    else {
        panic!("精确文件工具目录确认后才允许就绪");
    };
    assert_eq!(verified_cli_version.as_deref(), Some("1.0.30"));
    assert_eq!(
        effective_permissions["requestedPolicy"],
        "GrokRestrictedFilesV1"
    );
    assert_eq!(
        effective_permissions["grokCreationPolicyV1"]["toolSet"],
        "files"
    );
    assert_eq!(
        effective_permissions["permissionEnforcementVerified"],
        false
    );
}

#[test]
fn fixed_file_policy_cannot_change_native_approval_mode_through_a_slash_command() {
    let mut launch = options();
    launch.permission_policy = PermissionPolicy::GrokRestrictedFilesV1;
    let mut protocol = GrokProtocol::from_fixture(launch);
    let effects = protocol.command(RuntimeCommand {
        generation: protocol.options.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("/always-approve on".into())],
        },
    });
    assert!(effects.writes.is_empty());
    assert!(matches!(
        effects.events.as_slice(),
        [RuntimeEventKind::RequestFailed { .. }]
    ));
    assert!(protocol.submitted_messages.is_empty());
}

#[test]
fn fixed_file_write_approval_waits_for_the_user_and_denial_is_sent_once() {
    let directory = tempfile::tempdir().unwrap();
    let (mut protocol, mut request) = pending_native_approval();
    // 公开真实夹具将绝对路径脱敏；本测试替换为独立临时项目中的合成路径。
    request["params"]["toolCall"]["rawInput"]["file_path"] =
        json!(directory.path().join("approval.txt"));
    protocol.options.permission_policy = PermissionPolicy::GrokRestrictedFilesV1;
    protocol.options.grok_profile = Some(
        super::super::permissions::GrokCreationPolicyV1::compile(
            directory.path(),
            "1.0.30",
            "a".repeat(64),
            "b".repeat(64),
            None,
            PermissionPolicy::GrokRestrictedFilesV1,
        )
        .unwrap(),
    );
    let pending = protocol.receive(request.clone()).unwrap();
    assert!(pending.writes.is_empty());
    assert!(matches!(
        pending.events.as_slice(),
        [RuntimeEventKind::ApprovalRequested { .. }]
    ));
    assert!(protocol.receive(request.clone()).unwrap().events.is_empty());
    let denied = internal_action(
        &mut protocol,
        Uuid::new_v4(),
        RuntimeAction::RespondApproval {
            approval_id: "grok:0".into(),
            decision: ApprovalDecision::DenyOnce,
        },
    );
    assert_eq!(
        denied.writes,
        vec![json!({"jsonrpc":"2.0","id":0,
        "result":{"outcome":{"outcome":"selected","optionId":"reject-once"}}})]
    );
    assert!(protocol.receive(request).unwrap().writes.is_empty());
}

#[test]
fn fixed_file_unknown_write_variant_is_cancelled_without_opening_an_approval() {
    let directory = tempfile::tempdir().unwrap();
    let (mut protocol, mut request) = pending_native_approval();
    protocol.options.permission_policy = PermissionPolicy::GrokRestrictedFilesV1;
    protocol.options.grok_profile = Some(
        super::super::permissions::GrokCreationPolicyV1::compile(
            directory.path(),
            "1.0.30",
            "a".repeat(64),
            "b".repeat(64),
            None,
            PermissionPolicy::GrokRestrictedFilesV1,
        )
        .unwrap(),
    );
    request["params"]["toolCall"]["rawInput"]["variant"] = json!("EditFile");
    let cancelled = protocol.receive(request).unwrap();
    assert!(cancelled.events.is_empty());
    assert_eq!(
        cancelled.writes,
        vec![json!({"jsonrpc":"2.0","id":0,
        "result":{"outcome":{"outcome":"cancelled"}}})]
    );
    assert!(protocol.approvals.is_empty());
}

fn catalog_sdk_protocol() -> GrokProtocol {
    let mut options = options();
    options.local_tools = Some(LocalToolPermissions {
        allow_spawn: true,
        allow_message: true,
    });
    options.permission_policy = PermissionPolicy::GrokRestrictedReadV1;
    let profile = super::super::permissions::GrokCreationPolicyV1::compile(
        &options.cwd.canonicalize().unwrap(),
        "1.0.30",
        "a".repeat(64),
        "b".repeat(64),
        options.local_tools,
        PermissionPolicy::GrokRestrictedReadV1,
    )
    .unwrap();
    options.grok_profile = Some(profile.clone());
    let mut protocol = GrokProtocol::from_fixture(options);
    // 此离线帮助器从已完成握手后的状态起步；只补入真实 initialize 的精确布尔字段。
    protocol
        .reported_initialize_extensions
        .local_mcp_sdk_advertised = true;
    protocol.session_id = Some("catalog-native-session".into());
    let sdk = protocol.sdk.as_mut().unwrap();
    sdk.owned_process = true;
    sdk.bridge = super::GrokMcpBridge::with_creation_policy(sdk.process_epoch, &profile).unwrap();
    sdk.ledger = Some(
        super::GrokToolLeaseLedger::new(
            sdk.process_epoch,
            protocol.options.generation,
            sdk.bridge.server_id().into(),
            super::MCP_SERVER_NAME.into(),
            protocol.session_id.clone().unwrap(),
        )
        .unwrap(),
    );
    protocol
}

fn catalog_registration(protocol: &GrokProtocol, id: u64, method: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"_x.ai/mcp/sdk_call","params":{
        "serverId":protocol.sdk.as_ref().unwrap().bridge.server_id(),
        "message":{"jsonrpc":"2.0","id":id,"method":method,"params":{"_meta":{
            "io.modelcontextprotocol/protocolVersion":"2026-07-28",
            "io.modelcontextprotocol/clientCapabilities":{},
            "io.modelcontextprotocol/clientInfo":{"name":"offline-catalog-fixture","version":"1"}
        }}}
    }})
}

fn catalog_union_message() -> Value {
    json!({"jsonrpc":"2.0","method":"session/update","params":{
        "sessionId":"catalog-native-session","update":{"sessionUpdate":"available_commands_update",
        "availableCommands":[],
        "_meta":{"tools":["read_file","search_tool","use_tool",
            "infinishell-local-tasks__inspect_local_tasks","infinishell-local-tasks__run_agents",
            "infinishell-local-tasks__send_message_to_agent"]}}
    }})
}

async fn write_catalog_registration(protocol: &mut GrokProtocol) {
    let (sender, _receiver) = mpsc::channel(8);
    for (id, method) in [(301, "server/discover"), (302, "tools/list")] {
        let message = catalog_registration(protocol, id, method);
        let effects = protocol.receive(message).unwrap();
        let mut stdin = Cursor::new(Vec::new());
        flush_effects(protocol, &mut stdin, &sender, effects)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn catalog_union_needs_successful_transport_writes_and_same_live_ledger() {
    let mut protocol = catalog_sdk_protocol();
    assert!(protocol.receive(catalog_union_message()).is_err());
    let (sender, _receiver) = mpsc::channel(8);
    let message = catalog_registration(&protocol, 301, "server/discover");
    let effects = protocol.receive(message.clone()).unwrap();
    let mut empty = [];
    assert!(
        flush_effects(
            &mut protocol,
            &mut Cursor::new(&mut empty[..]),
            &sender,
            effects
        )
        .await
        .is_err()
    );
    assert!(protocol.receive(catalog_union_message()).is_err());
    write_catalog_registration(&mut protocol).await;
    protocol.receive(catalog_union_message()).unwrap();
    assert!(protocol.creation_catalog_session.is_some());
    assert!(protocol.sdk.as_ref().unwrap().calls.is_empty());
    assert!(!protocol.task_input_written);
}

#[tokio::test]
async fn catalog_union_rejects_replay_retirement_wrong_nonce_and_generation() {
    let mut replay = catalog_union_message();
    replay["params"]["_meta"]["isReplay"] = json!(true);
    let mut protocol = catalog_sdk_protocol();
    write_catalog_registration(&mut protocol).await;
    assert!(protocol.receive(replay).is_err());
    protocol.options.generation = Uuid::new_v4();
    assert!(protocol.receive(catalog_union_message()).is_err());

    let mut protocol = catalog_sdk_protocol();
    write_catalog_registration(&mut protocol).await;
    protocol
        .sdk
        .as_mut()
        .unwrap()
        .ledger
        .as_mut()
        .unwrap()
        .retire();
    assert!(protocol.receive(catalog_union_message()).is_err());

    let mut protocol = catalog_sdk_protocol();
    let mut message = catalog_registration(&protocol, 301, "server/discover");
    message["params"]["serverId"] = json!("old-process-nonce");
    assert!(protocol.receive(message).is_err());
    assert!(protocol.receive(catalog_union_message()).is_err());
}

#[tokio::test]
async fn old_session_catalog_cannot_confirm_new_creation_or_bypass_unknown_names() {
    let mut protocol = catalog_sdk_protocol();
    write_catalog_registration(&mut protocol).await;
    let mut old = catalog_union_message();
    old["params"]["sessionId"] = json!("old-native-session");
    assert!(protocol.receive(old).unwrap().events.is_empty());
    assert!(protocol.creation_catalog_session.is_none());
    let mut unknown = catalog_union_message();
    unknown["params"]["update"]["_meta"]["tools"][3] = json!("PRIVATE_UNKNOWN_CATALOG");
    let error = protocol.receive(unknown).err().unwrap();
    assert!(!error.to_string().contains("PRIVATE_UNKNOWN_CATALOG"));
    assert!(protocol.creation_catalog_session.is_none());
}

#[test]
fn legacy_grok_ready_carries_the_paired_native_version() {
    let mut protocol = GrokProtocol::from_fixture(options());
    protocol.initialize().unwrap();
    protocol.receive(fixture_response(1)).unwrap();
    protocol.receive(fixture_response(2)).unwrap();
    let ready = protocol.receive(fixture_response(3)).unwrap();
    assert!(
        matches!(ready.events.as_slice(), [RuntimeEventKind::SessionReady {
        verified_cli_version: Some(version), ..
    }] if version == "1.0.30")
    );
}
