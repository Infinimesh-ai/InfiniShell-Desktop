use super::*;
use futures::executor::block_on;
#[cfg(unix)]
use futures::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

#[cfg(unix)]
use std::fs::{self, File, OpenOptions};
#[cfg(unix)]
use std::io::BufRead as _;
#[cfg(target_os = "macos")]
use std::io::{self, Read as _};
#[cfg(unix)]
use std::process::Child;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use command::Stdio;
#[cfg(unix)]
use command::blocking::Command as BlockingCommand;
#[cfg(unix)]
use serde::{Deserialize, Serialize};

use crate::ai::cli_agent_runtime::local_tools::{LocalToolReplyTarget, NativeLocalToolRequest};
use crate::ai::cli_agent_runtime::{
    InputContent, PermissionPolicy, RuntimeAction, RuntimeEventKind, SessionTarget,
};

#[cfg(unix)]
const PROCESS_HELPER_TEST: &str =
    "ai::cli_agent_runtime::runtime_host::tests::runtime_host_process_helper";
#[cfg(unix)]
const PROCESS_ROOT_ENV: &str = "INFINISHELL_RUNTIME_HOST_TEST_ROOT";
#[cfg(unix)]
const PROCESS_ROLE_ENV: &str = "INFINISHELL_RUNTIME_HOST_TEST_ROLE";
#[cfg(unix)]
const PROCESS_MANIFEST_ENV: &str = "INFINISHELL_RUNTIME_HOST_TEST_MANIFEST";
#[cfg(unix)]
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(unix)]
const FAKE_ACK_MARKER: &str = "INFINISHELL_FAKE_ACK:";

#[cfg(unix)]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProcessHandoff {
    generation: Uuid,
    first_client_id: Uuid,
    first_message: Uuid,
    second_message: Uuid,
    acknowledged_sequence: u64,
}

#[cfg(unix)]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct FakeProcessReady {
    process_id: u32,
    process_start_time: u64,
    executable: PathBuf,
}

#[cfg(unix)]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct OfflineRecovery {
    recovered_through: u64,
    output: String,
    finished_turn_ids: std::collections::BTreeSet<String>,
}

fn manifest(state_dir: &Path, generation: Uuid) -> RuntimeHostManifest {
    RuntimeHostManifest {
        version: HOST_MANIFEST_VERSION,
        task_id: "task-a".to_owned(),
        task_generation: 1,
        harness: Harness::Codex,
        runtime_generation: generation,
        host_instance_id: Uuid::new_v4(),
        token: Uuid::new_v4(),
        endpoint: endpoint_name(Uuid::new_v4()),
        options: SessionOptions {
            executable: std::env::current_exe().unwrap(),
            cwd: state_dir.to_owned(),
            state_dir: state_dir.to_owned(),
            target: SessionTarget::New,
            generation,
            permission_policy: PermissionPolicy::ReadOnly,
            permission_ceiling: None,
            claude_profile: None,
            grok_profile: None,
            model: None,
            local_tools: None,
            selected_skills: Vec::new(),
        },
    }
}

fn service(
    state_dir: &Path,
    generation: Uuid,
) -> (
    RuntimeHostServiceImpl,
    tokio::sync::mpsc::Receiver<RuntimeCommand>,
    Arc<Mutex<RuntimeHostState>>,
) {
    service_with_command_capacity(state_dir, generation, super::super::COMMAND_CAPACITY)
}

fn service_with_command_capacity(
    state_dir: &Path,
    generation: Uuid,
    command_capacity: usize,
) -> (
    RuntimeHostServiceImpl,
    tokio::sync::mpsc::Receiver<RuntimeCommand>,
    Arc<Mutex<RuntimeHostState>>,
) {
    let manifest = manifest(state_dir, generation);
    let state = Arc::new(Mutex::new(
        RuntimeHostState::new(state_dir, manifest, "manifest-digest".to_owned()).unwrap(),
    ));
    let (sender, commands) = tokio::sync::mpsc::channel(command_capacity);
    let controller = RuntimeController {
        generation,
        commands: sender,
        host_event_acks: None,
    };
    (
        RuntimeHostServiceImpl {
            state: state.clone(),
            controller,
        },
        commands,
        state,
    )
}

fn claim(
    service: &RuntimeHostServiceImpl,
    token: Uuid,
    generation: Uuid,
    observed_epoch: u64,
    client_id: Uuid,
) -> RuntimeHostResponse {
    block_on(service.handle_request(RuntimeHostRequest::ClaimOwner {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        observed_epoch,
        client_id,
    }))
}

#[test]
fn runtime_host_manifest_version_is_v2() {
    assert_eq!(HOST_MANIFEST_VERSION, 2);
}

#[test]
fn exit_receipt_seals_the_journal_before_late_owner_mutations() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let host_manifest = manifest(&state_dir, generation);
    let record = RuntimeHostRecord {
        manifest: host_manifest.clone(),
        manifest_sha256: "manifest-digest".to_owned(),
        directory: state_dir.clone(),
    };
    let state = Arc::new(Mutex::new(
        RuntimeHostState::new(
            &state_dir,
            host_manifest.clone(),
            record.manifest_sha256.clone(),
        )
        .unwrap(),
    ));
    let (commands, _receiver) = tokio::sync::mpsc::channel(1);
    let service = RuntimeHostServiceImpl {
        state: state.clone(),
        controller: RuntimeController {
            generation,
            commands,
            host_event_acks: None,
        },
    };

    let receipt = write_exit_receipt(&record, &state, true, true, true).unwrap();
    assert!(matches!(
        claim(&service, host_manifest.token, generation, 0, Uuid::new_v4()),
        RuntimeHostResponse::Rejected {
            code: RuntimeHostRejection::OwnerJournalFailed
        }
    ));
    let summary = verify_journal(&record).unwrap();
    assert_eq!(summary.final_sha256, receipt.journal_sha256);
    assert_eq!(summary.last_event_sequence, receipt.last_event_sequence);
    assert_eq!(summary.acknowledged_sequence, receipt.acknowledged_sequence);
}

#[test]
fn manifest_failure_receipt_proves_host_and_adapter_were_not_started() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let host_directory = host_directory(&state_dir, generation);
    std::fs::create_dir_all(host_directory.parent().unwrap()).unwrap();
    create_private_directory(&host_directory).unwrap();
    let manifest = manifest(&state_dir, generation);
    let receipt =
        not_started_startup_receipt(&manifest, None, RuntimeHostStartupFailurePhase::Manifest);

    write_startup_exit_receipt(&host_directory, &receipt).unwrap();

    assert_eq!(
        read_startup_exit_receipt(&state_dir, generation).unwrap(),
        Some(receipt)
    );
    assert!(exit_confirmed_for_update(&state_dir, generation).unwrap());
    assert!(!exit_confirmed_for_update(&state_dir, Uuid::new_v4()).unwrap());
    assert!(matches!(
        block_on(classify_startup(&state_dir, generation)).unwrap(),
        RuntimeHostStartupState::Unconfirmed(RuntimeHostUnconfirmed::AdapterNotStarted)
    ));
    assert_eq!(
        startup_evidence_matches_task(&state_dir, generation, "task-a", 1).unwrap(),
        Some(true)
    );
    assert_eq!(
        startup_evidence_matches_task(&state_dir, generation, "task-b", 1).unwrap(),
        Some(false)
    );
}

#[test]
fn spawn_failure_receipt_is_bound_to_the_written_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let options = manifest(&state_dir, generation).options;
    let record = create_record("task-a".to_owned(), 1, Harness::Codex, options).unwrap();
    let receipt = not_started_startup_receipt(
        &record.manifest,
        Some(record.manifest_sha256.clone()),
        RuntimeHostStartupFailurePhase::Spawn,
    );

    write_startup_exit_receipt(&record.directory, &receipt).unwrap();

    assert_eq!(
        read_startup_exit_receipt(&state_dir, generation).unwrap(),
        Some(receipt)
    );
    assert!(matches!(
        block_on(classify_startup(&state_dir, generation)).unwrap(),
        RuntimeHostStartupState::Unconfirmed(RuntimeHostUnconfirmed::AdapterNotStarted)
    ));
}

#[test]
fn ready_failure_receipt_requires_observed_host_exit() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let options = manifest(&state_dir, generation).options;
    let record = create_record("task-a".to_owned(), 1, Harness::Codex, options).unwrap();
    let mut receipt = not_started_startup_receipt(
        &record.manifest,
        Some(record.manifest_sha256.clone()),
        RuntimeHostStartupFailurePhase::ReadyHandshake,
    );
    receipt.host_process_id = None;
    receipt.host_exit_success = None;

    write_startup_exit_receipt(&record.directory, &receipt).unwrap();

    assert!(exit_confirmed_for_update(&state_dir, generation).is_err());
    let error = read_startup_exit_receipt(&state_dir, generation).unwrap_err();
    assert!(error.to_string().contains("缺少进程退出证据"));
}

#[test]
fn observed_host_exit_and_missing_native_domain_prove_adapter_not_started() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let options = manifest(&state_dir, generation).options;
    let record = create_record("task-a".to_owned(), 1, Harness::Codex, options).unwrap();
    let receipt = observed_startup_exit_receipt(
        &record,
        ObservedHostExit {
            process_id: 42,
            success: false,
            exit_code: None,
        },
        super::super::managed_process::ProcessCompletion::NotStarted,
    )
    .unwrap();

    write_startup_exit_receipt(&record.directory, &receipt).unwrap();

    assert_eq!(
        read_startup_exit_receipt(&state_dir, generation).unwrap(),
        Some(receipt)
    );
    assert!(matches!(
        block_on(classify_startup(&state_dir, generation)).unwrap(),
        RuntimeHostStartupState::Unconfirmed(RuntimeHostUnconfirmed::AdapterNotStarted)
    ));
}

#[cfg(unix)]
#[test]
fn ready_failure_terminates_and_reaps_the_spawned_host_process() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let mut command = Command::new(&executable);
    command
        .args([
            "--exact",
            PROCESS_HELPER_TEST,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(&root)
        .env(PROCESS_ROOT_ENV, &root)
        .env(PROCESS_ROLE_ENV, "startup_stall")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let generation = Uuid::new_v4();
    let record = create_record(
        "task-a".to_owned(),
        1,
        Harness::Codex,
        manifest(&root, generation).options,
    )
    .unwrap();
    write_startup_incomplete_marker(&record).unwrap();
    let mut child = SpawnedHostGuard::new(command.spawn().unwrap(), record);
    wait_for_file(&root.join("startup-stall-ready"));
    let process_id = child.process_mut().id();
    let (process_start_time, process_executable) = process_identity(process_id).unwrap();

    let observed = block_on(observe_spawned_host_exit(child.process_mut(), process_id)).unwrap();

    assert_eq!(observed.process_id, process_id);
    assert!(!observed.success);
    assert!(!identity_alive(
        process_id,
        process_start_time,
        &process_executable
    ));
    child.release().unwrap();
}

#[cfg(unix)]
#[test]
fn cancelled_spawn_monitor_persists_observed_exit_before_clearing_marker() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let mut command = Command::new(&executable);
    command
        .args([
            "--exact",
            PROCESS_HELPER_TEST,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(&root)
        .env(PROCESS_ROOT_ENV, &root)
        .env(PROCESS_ROLE_ENV, "startup_stall")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let generation = Uuid::new_v4();
    let record = create_record(
        "task-a".to_owned(),
        1,
        Harness::Codex,
        manifest(&root, generation).options,
    )
    .unwrap();
    write_startup_incomplete_marker(&record).unwrap();
    let mut child = SpawnedHostGuard::new(command.spawn().unwrap(), record.clone());
    wait_for_file(&root.join("startup-stall-ready"));
    let process_id = child.process_mut().id();
    let (process_start_time, process_executable) = process_identity(process_id).unwrap();

    drop(child);

    wait_identity_exit(process_id, process_start_time, &process_executable);
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    while record
        .directory
        .join(STARTUP_INCOMPLETE_RECORD)
        .try_exists()
        .unwrap()
    {
        assert!(Instant::now() < deadline, "取消监视线程未收敛启动证据");
        thread::sleep(Duration::from_millis(20));
    }
    let receipt = read_startup_exit_receipt(&root, generation)
        .unwrap()
        .expect("取消监视线程必须持久化启动退出收据");
    assert_eq!(
        receipt.phase,
        RuntimeHostStartupFailurePhase::ReadyHandshake
    );
    assert_eq!(receipt.host_process_id, Some(process_id));
    assert_eq!(receipt.native_process, NativeProcessCompletion::NotStarted);
    assert!(matches!(
        block_on(classify_startup(&root, generation)).unwrap(),
        RuntimeHostStartupState::Unconfirmed(RuntimeHostUnconfirmed::AdapterNotStarted)
    ));
}

#[cfg(unix)]
#[test]
fn cancelled_spawn_monitor_keeps_marker_when_receipt_cannot_be_persisted() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let mut command = Command::new(&executable);
    command
        .args([
            "--exact",
            PROCESS_HELPER_TEST,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(&root)
        .env(PROCESS_ROOT_ENV, &root)
        .env(PROCESS_ROLE_ENV, "startup_stall")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let generation = Uuid::new_v4();
    let record = create_record(
        "task-a".to_owned(),
        1,
        Harness::Codex,
        manifest(&root, generation).options,
    )
    .unwrap();
    write_startup_incomplete_marker(&record).unwrap();
    fs::create_dir(record.directory.join(STARTUP_EXIT_RECORD)).unwrap();
    let process = command.spawn().unwrap();
    wait_for_file(&root.join("startup-stall-ready"));
    let process_id = process.id();
    let (process_start_time, process_executable) = process_identity(process_id).unwrap();

    let monitor = spawn_cancelled_startup_monitor(record.clone(), process).unwrap();
    let error = monitor
        .join()
        .expect("取消监视线程不应 panic")
        .expect_err("收据无法持久化时必须保留失败");

    assert!(!identity_alive(
        process_id,
        process_start_time,
        &process_executable
    ));
    assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(record.directory.join(STARTUP_INCOMPLETE_RECORD).is_file());
    assert!(matches!(
        block_on(classify_startup(&root, generation)).unwrap(),
        RuntimeHostStartupState::Unconfirmed(RuntimeHostUnconfirmed::NativeExitUnconfirmed)
    ));
}

#[test]
fn cancelled_spawn_is_unconfirmed_even_if_adapter_never_started() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let options = manifest(&state_dir, generation).options;
    let record = create_record("task-a".to_owned(), 1, Harness::Codex, options).unwrap();

    write_startup_incomplete_marker(&record).unwrap();

    assert_eq!(
        startup_evidence_matches_task(&state_dir, generation, "task-a", 1).unwrap(),
        Some(true)
    );
    assert!(matches!(
        block_on(classify_startup(&state_dir, generation)).unwrap(),
        RuntimeHostStartupState::Unconfirmed(RuntimeHostUnconfirmed::NativeExitUnconfirmed)
    ));
}

#[test]
fn journal_failure_cancels_the_adapter_instead_of_running_without_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let options = manifest(directory.path(), generation).options;
    let record = create_record("task-a".to_owned(), 1, Harness::Codex, options).unwrap();
    let state = Arc::new(Mutex::new(
        RuntimeHostState::new(
            &record.directory,
            record.manifest.clone(),
            record.manifest_sha256.clone(),
        )
        .unwrap(),
    ));
    state.lock().unwrap().append_failure_countdown = Some(0);
    let (controller, _commands, sender, events) = super::super::channels(generation);
    let event = RuntimeEvent {
        generation,
        native_session_id: Some("session-a".to_owned()),
        kind: RuntimeEventKind::Progress {
            turn_id: "turn-a".to_owned(),
            message: "must-be-durable".to_owned(),
        },
    };
    let task = Box::pin(async move {
        sender
            .send(event)
            .await
            .map_err(|_| RuntimeError::ControllerClosed)?;
        futures::future::pending::<Result<(), RuntimeError>>().await
    });

    let result = run_connection(
        &record,
        state,
        super::super::RuntimeConnection {
            controller,
            events,
            task,
        },
    )
    .unwrap();

    assert!(matches!(
        result.adapter,
        Err(RuntimeError::Protocol(ref code)) if code == "runtime_host.event_journal_failed"
    ));
    assert!(result.journal.is_err());
}

#[test]
fn ipc_wire_roundtrip_supports_json_values_in_commands_events_and_snapshots() {
    let generation = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    let request = RuntimeHostRequest::Command {
        token: Uuid::new_v4(),
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id: Uuid::new_v4(),
        command: RuntimeCommand {
            generation,
            message_id,
            action: RuntimeAction::RespondLocalTool {
                turn_id: "turn-a".to_owned(),
                call_id: "call-a".to_owned(),
                result: Ok(serde_json::json!({"nested":[true, 2, "three"]})),
            },
        },
    };
    let request_bytes = bincode::serialize(&request).unwrap();
    assert_eq!(
        bincode::deserialize::<RuntimeHostRequest>(&request_bytes).unwrap(),
        request
    );

    let event = HostedEvent {
        sequence: 1,
        digest: "digest-a".to_owned(),
        event: RuntimeEvent {
            generation,
            native_session_id: Some("session-a".to_owned()),
            kind: RuntimeEventKind::SessionReady {
                verified_cli_version: Some("test-v2".to_owned()),
                effective_permissions: serde_json::json!({"policy":{"allow":["read"]}}),
            },
        },
    };
    let response = RuntimeHostResponse::Events(vec![event]);
    let response_bytes = bincode::serialize(&response).unwrap();
    assert_eq!(
        bincode::deserialize::<RuntimeHostResponse>(&response_bytes).unwrap(),
        response
    );

    let mut snapshot = RuntimeHostSnapshot::default();
    snapshot.approvals.insert(
        "approval-a".to_owned(),
        HostedApproval {
            turn_id: "turn-a".to_owned(),
            method: "command_execution".to_owned(),
            details: serde_json::json!({
                "command": {"argv": ["echo", "hello"]},
                "risk": {"categories": ["filesystem", "network"]},
            }),
        },
    );
    for response in [
        RuntimeHostResponse::Inspection {
            host_instance_id: Uuid::new_v4(),
            owner_epoch: 1,
            last_event_sequence: 3,
            acknowledged_sequence: 2,
            snapshot: snapshot.clone(),
        },
        RuntimeHostResponse::OwnerClaimed {
            owner_epoch: 2,
            last_event_sequence: 3,
            acknowledged_sequence: 2,
            snapshot,
        },
    ] {
        let response_bytes = bincode::serialize(&response).unwrap();
        assert_eq!(
            bincode::deserialize::<RuntimeHostResponse>(&response_bytes).unwrap(),
            response
        );
    }
}

#[test]
fn runtime_host_frame_limit_reserves_space_for_bincode_envelopes() {
    let generation = Uuid::new_v4();
    let command = RuntimeCommand {
        generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::RespondLocalTool {
            turn_id: "turn-a".to_owned(),
            call_id: "call-a".to_owned(),
            result: Ok(serde_json::json!({"nested":[true, 2, "three"]})),
        },
    };
    let command_json_bytes = serde_json::to_vec(&command).unwrap().len();
    let request_bytes = bincode::serialize(&RuntimeHostRequest::Command {
        token: Uuid::new_v4(),
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id: Uuid::new_v4(),
        command,
    })
    .unwrap()
    .len();

    let events = vec![HostedEvent {
        sequence: 1,
        digest: "digest-a".to_owned(),
        event: RuntimeEvent {
            generation,
            native_session_id: Some("session-a".to_owned()),
            kind: RuntimeEventKind::Progress {
                turn_id: "turn-a".to_owned(),
                message: "progress-a".to_owned(),
            },
        },
    }];
    let events_json_bytes = serde_json::to_vec(&events).unwrap().len();
    let response_bytes = bincode::serialize(&RuntimeHostResponse::Events(events))
        .unwrap()
        .len();

    // json_wire 的内容长度变化不会扩大固定 bincode 包装；额外预算还覆盖外层 IPC
    // Request/Response 的 UUID、service id 与长度字段。
    assert!(request_bytes - command_json_bytes < RUNTIME_HOST_FRAME_ENVELOPE_BYTES);
    assert!(response_bytes - events_json_bytes < RUNTIME_HOST_FRAME_ENVELOPE_BYTES);
    assert_eq!(
        MAX_RUNTIME_HOST_FRAME_BYTES,
        MAX_IPC_JSON_BYTES + RUNTIME_HOST_FRAME_ENVELOPE_BYTES
    );
}

#[cfg(unix)]
fn send_oversized_runtime_host_header(endpoint: &str) {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(endpoint).unwrap();
    stream
        .write_all(&(MAX_RUNTIME_HOST_FRAME_BYTES + 1).to_be_bytes())
        .unwrap();
    stream.flush().unwrap();
}

#[cfg(windows)]
fn send_oversized_runtime_host_header(endpoint: &str) {
    use std::io::Write as _;

    let mut stream = interprocess::local_socket::LocalSocketStream::connect(endpoint).unwrap();
    stream
        .write_all(&(MAX_RUNTIME_HOST_FRAME_BYTES + 1).to_be_bytes())
        .unwrap();
    stream.flush().unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn runtime_host_continues_serving_after_an_oversized_frame_header() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let options = manifest(&state_dir, generation).options;
    let record = create_record("task-a".to_owned(), 1, Harness::Codex, options).unwrap();
    let state = Arc::new(Mutex::new(
        RuntimeHostState::new(
            &record.directory,
            record.manifest.clone(),
            record.manifest_sha256.clone(),
        )
        .unwrap(),
    ));
    let (controller, commands, sender, events) = super::super::channels(generation);
    let executor = Arc::new(Background::new(2, |_| {
        "runtime-host-frame-limit-test".to_owned()
    }));
    let server = start_runtime_host_ipc(&record, state, controller, executor).unwrap();

    send_oversized_runtime_host_header(&record.manifest.endpoint);
    let client = block_on(RuntimeHostClient::connect(record.clone())).unwrap();
    let response = block_on(client.inspect()).unwrap();

    assert!(matches!(response, RuntimeHostResponse::Inspection { .. }));
    drop(client);
    drop(server);
    drop((commands, sender, events));
    remove_endpoint(&record.manifest.endpoint);
}

#[test]
fn owner_epoch_fences_the_previous_client() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, _, state) = service(directory.path(), generation);
    let token = state.lock().unwrap().manifest.token;
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();

    assert!(matches!(
        claim(&service, token, generation, 0, first),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    assert!(matches!(
        claim(&service, token, generation, 1, second),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 2, .. }
    ));
    let response = block_on(service.handle_request(RuntimeHostRequest::PullEvents {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id: first,
        after_sequence: 0,
    }));

    assert_eq!(
        response,
        RuntimeHostResponse::Rejected {
            code: RuntimeHostRejection::StaleOwner
        }
    );
}

#[test]
fn duplicate_command_is_delivered_to_the_adapter_once() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, mut commands, state) = service(directory.path(), generation);
    let token = state.lock().unwrap().manifest.token;
    let client_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, client_id),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    let command = RuntimeCommand {
        generation,
        message_id,
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("first".to_owned())],
        },
    };
    let request = || RuntimeHostRequest::Command {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        command: command.clone(),
    };

    let first = block_on(service.handle_request(request()));
    let duplicate = block_on(service.handle_request(request()));

    assert_eq!(first, duplicate);
    assert_eq!(commands.try_recv().unwrap(), command);
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[test]
fn full_command_queue_applies_backpressure_before_recording() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, mut commands, state) =
        service_with_command_capacity(directory.path(), generation, 1);
    let token = state.lock().unwrap().manifest.token;
    let client_id = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, client_id),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    let first = RuntimeCommand {
        generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("first".to_owned())],
        },
    };
    let second = RuntimeCommand {
        generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("second".to_owned())],
        },
    };
    let request = |command| RuntimeHostRequest::Command {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        command,
    };

    assert!(matches!(
        block_on(service.handle_request(request(first.clone()))),
        RuntimeHostResponse::CommandStatus(HostCommandStatus {
            disposition: HostCommandDisposition::DeliveredToAdapter,
            ..
        })
    ));
    let response = block_on(async {
        let pending = service.handle_request(request(second.clone()));
        futures::pin_mut!(pending);
        assert!(matches!(
            futures::poll!(&mut pending),
            std::task::Poll::Pending
        ));
        {
            let state = state.lock().unwrap();
            assert!(!state.commands.contains_key(&second.message_id));
            assert!(state.events.is_empty());
        }
        assert_eq!(commands.recv().await.unwrap(), first);
        pending.await
    });

    assert!(matches!(
        response,
        RuntimeHostResponse::CommandStatus(HostCommandStatus {
            disposition: HostCommandDisposition::DeliveredToAdapter,
            ..
        })
    ));
    assert_eq!(block_on(commands.recv()).unwrap(), second);
    let state = state.lock().unwrap();
    assert!(state.events.is_empty());
    assert_eq!(
        state.commands[&second.message_id].disposition,
        HostCommandDisposition::DeliveredToAdapter
    );
}

#[test]
fn owner_claim_while_waiting_for_capacity_fences_the_old_command() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, mut commands, state) =
        service_with_command_capacity(directory.path(), generation, 1);
    let token = state.lock().unwrap().manifest.token;
    let first_client = Uuid::new_v4();
    let second_client = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, first_client),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    let first = RuntimeCommand {
        generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("first".to_owned())],
        },
    };
    let stale = RuntimeCommand {
        generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("stale".to_owned())],
        },
    };
    let request = |command| RuntimeHostRequest::Command {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id: first_client,
        command,
    };

    assert!(matches!(
        block_on(service.handle_request(request(first.clone()))),
        RuntimeHostResponse::CommandStatus(HostCommandStatus {
            disposition: HostCommandDisposition::DeliveredToAdapter,
            ..
        })
    ));
    let response = block_on(async {
        let pending = service.handle_request(request(stale.clone()));
        futures::pin_mut!(pending);
        assert!(matches!(
            futures::poll!(&mut pending),
            std::task::Poll::Pending
        ));
        assert!(matches!(
            service
                .handle_request(RuntimeHostRequest::ClaimOwner {
                    token,
                    task_id: "task-a".to_owned(),
                    runtime_generation: generation,
                    observed_epoch: 1,
                    client_id: second_client,
                })
                .await,
            RuntimeHostResponse::OwnerClaimed { owner_epoch: 2, .. }
        ));
        assert_eq!(commands.recv().await.unwrap(), first);
        pending.await
    });

    assert_eq!(
        response,
        RuntimeHostResponse::Rejected {
            code: RuntimeHostRejection::StaleOwner
        }
    );
    assert!(matches!(
        commands.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    let state = state.lock().unwrap();
    assert!(!state.commands.contains_key(&stale.message_id));
    assert!(state.events.is_empty());
}

#[test]
fn full_command_queue_records_failure_when_adapter_exits_while_waiting() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, commands, state) = service_with_command_capacity(directory.path(), generation, 1);
    let token = state.lock().unwrap().manifest.token;
    let client_id = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, client_id),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    let first = RuntimeCommand {
        generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("first".to_owned())],
        },
    };
    let second = RuntimeCommand {
        generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("second".to_owned())],
        },
    };
    let request = |command| RuntimeHostRequest::Command {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        command,
    };

    assert!(matches!(
        block_on(service.handle_request(request(first))),
        RuntimeHostResponse::CommandStatus(HostCommandStatus {
            disposition: HostCommandDisposition::DeliveredToAdapter,
            ..
        })
    ));
    let response = block_on(async {
        let pending = service.handle_request(request(second.clone()));
        futures::pin_mut!(pending);
        assert!(matches!(
            futures::poll!(&mut pending),
            std::task::Poll::Pending
        ));
        drop(commands);
        pending.await
    });

    assert!(matches!(
        response,
        RuntimeHostResponse::CommandStatus(HostCommandStatus {
            disposition: HostCommandDisposition::Failed,
            ..
        })
    ));
    let state = state.lock().unwrap();
    assert!(state.events.iter().any(|event| matches!(
        &event.event.kind,
        RuntimeEventKind::RequestFailed { message_id, message }
            if *message_id == second.message_id && message == COMMAND_NOT_DELIVERED_CODE
    )));
}

#[test]
fn closed_adapter_records_a_reliable_request_failure_without_hanging() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, commands, state) = service(directory.path(), generation);
    drop(commands);
    let token = state.lock().unwrap().manifest.token;
    let client_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, client_id),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));

    let response = block_on(service.handle_request(RuntimeHostRequest::Command {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        command: RuntimeCommand {
            generation,
            message_id,
            action: RuntimeAction::Submit {
                input: vec![InputContent::Text("will-fail".to_owned())],
            },
        },
    }));

    assert!(matches!(
        response,
        RuntimeHostResponse::CommandStatus(HostCommandStatus {
            disposition: HostCommandDisposition::Failed,
            ..
        })
    ));
    let state = state.lock().unwrap();
    assert!(state.events.iter().any(|event| matches!(
        &event.event.kind,
        RuntimeEventKind::RequestFailed { message_id: actual, message }
            if *actual == message_id && message == COMMAND_NOT_DELIVERED_CODE
    )));
}

#[test]
fn failed_dispatch_without_a_durable_failure_event_stays_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, commands, state) = service(directory.path(), generation);
    drop(commands);
    let token = state.lock().unwrap().manifest.token;
    let client_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, client_id),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    state.lock().unwrap().append_failure_countdown = Some(1);
    let request = || RuntimeHostRequest::Command {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        command: RuntimeCommand {
            generation,
            message_id,
            action: RuntimeAction::Submit {
                input: vec![InputContent::Text("unknown-failure".to_owned())],
            },
        },
    };

    let first = block_on(service.handle_request(request()));
    let retry = block_on(service.handle_request(request()));

    assert!(matches!(
        &first,
        RuntimeHostResponse::CommandStatus(HostCommandStatus {
            disposition: HostCommandDisposition::Recorded,
            ..
        })
    ));
    assert_eq!(first, retry);
    let state = state.lock().unwrap();
    assert!(state.events.is_empty());
    assert_eq!(
        state.commands[&message_id].disposition,
        HostCommandDisposition::Recorded
    );
}

#[test]
fn durable_failure_event_remains_visible_when_command_status_update_fails() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, commands, state) = service(directory.path(), generation);
    drop(commands);
    let token = state.lock().unwrap().manifest.token;
    let client_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, client_id),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    // CommandRecorded 和 RequestFailed Event 均成功，紧随其后的 CommandUpdated 失败。
    state.lock().unwrap().append_failure_countdown = Some(2);
    let request = || RuntimeHostRequest::Command {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        command: RuntimeCommand {
            generation,
            message_id,
            action: RuntimeAction::Submit {
                input: vec![InputContent::Text("status-write-fails".to_owned())],
            },
        },
    };

    let first = block_on(service.handle_request(request()));
    let retry = block_on(service.handle_request(request()));

    assert!(matches!(
        &first,
        RuntimeHostResponse::CommandStatus(HostCommandStatus {
            disposition: HostCommandDisposition::Recorded,
            ..
        })
    ));
    assert_eq!(first, retry);
    let events = block_on(service.handle_request(RuntimeHostRequest::PullEvents {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        after_sequence: 0,
    }));
    assert!(matches!(
        events,
        RuntimeHostResponse::Events(events)
            if matches!(
                &events.as_slice(),
                [HostedEvent {
                    event: RuntimeEvent {
                        kind: RuntimeEventKind::RequestFailed {
                            message_id: actual,
                            message,
                        },
                        ..
                    },
                    ..
                }] if *actual == message_id && message == COMMAND_NOT_DELIVERED_CODE
            )
    ));
    let state = state.lock().unwrap();
    assert_eq!(
        state.commands[&message_id].disposition,
        HostCommandDisposition::Recorded
    );
}

#[test]
fn uncertain_dispatch_is_recorded_and_never_replayed() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, mut commands, state) = service(directory.path(), generation);
    let token = state.lock().unwrap().manifest.token;
    let client_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, client_id),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    state.lock().unwrap().append_failure_countdown = Some(1);
    let command = RuntimeCommand {
        generation,
        message_id,
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("unknown".to_owned())],
        },
    };
    let request = || RuntimeHostRequest::Command {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        command: command.clone(),
    };

    let first = block_on(service.handle_request(request()));
    let retry = block_on(service.handle_request(request()));

    assert!(matches!(
        &first,
        RuntimeHostResponse::CommandStatus(HostCommandStatus {
            disposition: HostCommandDisposition::Recorded,
            ..
        })
    ));
    assert_eq!(first, retry);
    assert_eq!(commands.try_recv().unwrap(), command);
    assert!(commands.try_recv().is_err(), "结果未知的命令不得重投");
}

#[test]
fn same_message_id_with_changed_content_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, mut commands, state) = service(directory.path(), generation);
    let token = state.lock().unwrap().manifest.token;
    let client_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, client_id),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    let request = |text: &str| RuntimeHostRequest::Command {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        command: RuntimeCommand {
            generation,
            message_id,
            action: RuntimeAction::Submit {
                input: vec![InputContent::Text(text.to_owned())],
            },
        },
    };
    let first = block_on(service.handle_request(request("first")));
    let changed = block_on(service.handle_request(request("changed")));

    assert!(matches!(first, RuntimeHostResponse::CommandStatus(_)));
    assert_eq!(
        changed,
        RuntimeHostResponse::Rejected {
            code: RuntimeHostRejection::CommandIdentityChanged
        }
    );
    assert_eq!(
        commands.try_recv().unwrap().action,
        RuntimeAction::Submit {
            input: vec![InputContent::Text("first".to_owned())]
        }
    );
    assert!(commands.try_recv().is_err());
}

#[test]
fn reconnect_pulls_only_events_after_the_durable_ack() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, _, state) = service(directory.path(), generation);
    let token = state.lock().unwrap().manifest.token;
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, first),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    {
        let mut state = state.lock().unwrap();
        state
            .record_event(RuntimeEvent {
                generation,
                native_session_id: Some("session-a".to_owned()),
                kind: RuntimeEventKind::Progress {
                    turn_id: "turn-a".to_owned(),
                    message: "one".to_owned(),
                },
            })
            .unwrap();
        state
            .record_event(RuntimeEvent {
                generation,
                native_session_id: Some("session-a".to_owned()),
                kind: RuntimeEventKind::Progress {
                    turn_id: "turn-a".to_owned(),
                    message: "two".to_owned(),
                },
            })
            .unwrap();
    }
    assert!(matches!(
        block_on(service.handle_request(RuntimeHostRequest::AckEvents {
            token,
            task_id: "task-a".to_owned(),
            runtime_generation: generation,
            owner_epoch: 1,
            client_id: first,
            through_sequence: 1,
        })),
        RuntimeHostResponse::EventsAcknowledged {
            through_sequence: 1
        }
    ));
    assert!(matches!(
        claim(&service, token, generation, 1, second),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 2, .. }
    ));

    let response = block_on(service.handle_request(RuntimeHostRequest::PullEvents {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 2,
        client_id: second,
        after_sequence: 1,
    }));

    let RuntimeHostResponse::Events(events) = response else {
        panic!("重关联必须返回事件");
    };
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sequence, 2);
    assert!(matches!(
        &events[0].event.kind,
        RuntimeEventKind::Progress { message, .. } if message == "two"
    ));
}

#[test]
fn inspection_recovers_only_acknowledged_pending_local_tools() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, _, state) = service(directory.path(), generation);
    let token = state.lock().unwrap().manifest.token;
    let client_id = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, client_id),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    let request = |call_id: &str| NativeLocalToolRequest {
        reply_target: LocalToolReplyTarget::Codex {
            request_id: serde_json::json!(call_id),
        },
        call_id: call_id.to_owned(),
        turn_id: "turn-tools".to_owned(),
        tool: "inspect_local_tasks".to_owned(),
        arguments: serde_json::json!({}),
    };
    {
        let mut state = state.lock().unwrap();
        for kind in [
            RuntimeEventKind::TurnStarted {
                turn_id: "turn-tools".to_owned(),
            },
            RuntimeEventKind::LocalToolRequested {
                request: request("acked-call"),
            },
            RuntimeEventKind::LocalToolRequested {
                request: request("unacked-call"),
            },
        ] {
            state
                .record_event(RuntimeEvent {
                    generation,
                    native_session_id: Some("session-tools".to_owned()),
                    kind,
                })
                .unwrap();
        }
    }
    assert!(matches!(
        block_on(service.handle_request(RuntimeHostRequest::AckEvents {
            token,
            task_id: "task-a".to_owned(),
            runtime_generation: generation,
            owner_epoch: 1,
            client_id,
            through_sequence: 2,
        })),
        RuntimeHostResponse::EventsAcknowledged {
            through_sequence: 2
        }
    ));
    let inspected = block_on(service.handle_request(RuntimeHostRequest::Inspect {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
    }));
    let RuntimeHostResponse::Inspection {
        acknowledged_sequence,
        snapshot,
        ..
    } = inspected
    else {
        panic!("检查必须返回已确认快照");
    };
    assert_eq!(acknowledged_sequence, 2);
    assert_eq!(snapshot.active_turn_id.as_deref(), Some("turn-tools"));
    assert_eq!(
        snapshot.local_tools.keys().cloned().collect::<Vec<_>>(),
        vec!["acked-call".to_owned()]
    );

    let pending = block_on(service.handle_request(RuntimeHostRequest::PullEvents {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 1,
        client_id,
        after_sequence: acknowledged_sequence,
    }));
    assert!(matches!(
        pending,
        RuntimeHostResponse::Events(events)
            if matches!(events.as_slice(), [HostedEvent {
                event: RuntimeEvent {
                    kind: RuntimeEventKind::LocalToolRequested { request },
                    ..
                },
                ..
            }] if request.call_id == "unacked-call")
    ));
}

#[test]
fn owner_claim_freezes_catch_up_tail_after_an_unacknowledged_tool_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let generation = Uuid::new_v4();
    let (service, _, state) = service(directory.path(), generation);
    let token = state.lock().unwrap().manifest.token;
    let first_client = Uuid::new_v4();
    let second_client = Uuid::new_v4();
    assert!(matches!(
        claim(&service, token, generation, 0, first_client),
        RuntimeHostResponse::OwnerClaimed { owner_epoch: 1, .. }
    ));
    let request = NativeLocalToolRequest {
        reply_target: LocalToolReplyTarget::Codex {
            request_id: serde_json::json!("request-catch-up-tail"),
        },
        call_id: "call-catch-up-tail".to_owned(),
        turn_id: "turn-catch-up-tail".to_owned(),
        tool: "inspect_local_tasks".to_owned(),
        arguments: serde_json::json!({}),
    };
    {
        let mut state = state.lock().unwrap();
        for kind in [
            RuntimeEventKind::TurnStarted {
                turn_id: request.turn_id.clone(),
            },
            RuntimeEventKind::LocalToolRequested {
                request: request.clone(),
            },
        ] {
            state
                .record_event(RuntimeEvent {
                    generation,
                    native_session_id: Some("session-catch-up-tail".to_owned()),
                    kind,
                })
                .unwrap();
        }
    }
    assert!(matches!(
        block_on(service.handle_request(RuntimeHostRequest::AckEvents {
            token,
            task_id: "task-a".to_owned(),
            runtime_generation: generation,
            owner_epoch: 1,
            client_id: first_client,
            through_sequence: 2,
        })),
        RuntimeHostResponse::EventsAcknowledged {
            through_sequence: 2
        }
    ));
    state
        .lock()
        .unwrap()
        .record_event(RuntimeEvent {
            generation,
            native_session_id: Some("session-catch-up-tail".to_owned()),
            kind: RuntimeEventKind::LocalToolCancelled {
                turn_id: request.turn_id.clone(),
                call_id: request.call_id.clone(),
            },
        })
        .unwrap();

    let claimed = claim(&service, token, generation, 1, second_client);
    let RuntimeHostResponse::OwnerClaimed {
        owner_epoch,
        last_event_sequence,
        acknowledged_sequence,
        snapshot,
    } = claimed
    else {
        panic!("新 owner 必须取得固定 catch-up 边界");
    };
    assert_eq!(owner_epoch, 2);
    assert_eq!(acknowledged_sequence, 2);
    assert_eq!(last_event_sequence, 3);
    assert!(snapshot.local_tools.contains_key(&request.call_id));

    state
        .lock()
        .unwrap()
        .record_event(RuntimeEvent {
            generation,
            native_session_id: Some("session-catch-up-tail".to_owned()),
            kind: RuntimeEventKind::Progress {
                turn_id: request.turn_id,
                message: "after-claim".to_owned(),
            },
        })
        .unwrap();
    let pending = block_on(service.handle_request(RuntimeHostRequest::PullEvents {
        token,
        task_id: "task-a".to_owned(),
        runtime_generation: generation,
        owner_epoch: 2,
        client_id: second_client,
        after_sequence: acknowledged_sequence,
    }));
    let RuntimeHostResponse::Events(events) = pending else {
        panic!("新 owner 必须能够读取固定 tail 后的新事件");
    };
    assert_eq!(
        events
            .iter()
            .map(|hosted| hosted.sequence)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_eq!(
        last_event_sequence, 3,
        "claim 后的新事件不能扩张本次 catch-up 屏障"
    );
}

#[cfg(unix)]
fn process_options(root: &Path, generation: Uuid) -> SessionOptions {
    SessionOptions {
        executable: std::env::current_exe().unwrap().canonicalize().unwrap(),
        cwd: root.to_owned(),
        state_dir: root.to_owned(),
        target: SessionTarget::New,
        generation,
        permission_policy: PermissionPolicy::ReadOnly,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    }
}

#[cfg(unix)]
fn helper_command(root: &Path, role: &str) -> BlockingCommand {
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let mut command = BlockingCommand::new(&executable);
    command
        .args([
            "--exact",
            PROCESS_HELPER_TEST,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(root)
        .env(PROCESS_ROOT_ENV, root)
        .env(PROCESS_ROLE_ENV, role)
        .env("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE", executable)
        .env(
            "INFINISHELL_CLI_PROCESS_SUPERVISOR_EXECUTABLE",
            root.join("managed-supervisor.sh"),
        )
        .stdin(Stdio::null());
    command
}

#[cfg(unix)]
fn prepare_process_wrappers(root: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    let source_binary = std::env::current_exe().unwrap().canonicalize().unwrap();
    #[cfg(target_os = "macos")]
    let binary = {
        // 外置 Cargo target 上的超大 libtest Mach-O 冷启动会耗尽生产握手期限；
        // 测试副本放入本地临时域，并随 TempDir 一起回收，不改变产品超时。
        let binary = root.join("process-helper");
        let mut source = File::open(&source_binary).unwrap();
        let mut destination = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&binary)
            .unwrap();
        io::copy(&mut source, &mut destination).unwrap();
        destination.sync_all().unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(binary.starts_with(root));
        assert_eq!(file_sha256(&source_binary), file_sha256(&binary));
        binary
    };
    #[cfg(not(target_os = "macos"))]
    let binary = source_binary;
    let supervisor = root.join("managed-supervisor.sh");
    fs::write(
        &supervisor,
        format!(
            "#!/bin/sh\nexport {PROCESS_ROOT_ENV}=\"{}\"\nexport INFINISHELL_CLI_PROCESS_SUPERVISOR_EXECUTABLE=\"$0\"\nif [ \"$3\" = \"--execute\" ]; then\n  export {PROCESS_ROLE_ENV}=managed_execute\nelse\n  export {PROCESS_ROLE_ENV}=managed_worker\nfi\nexport {PROCESS_MANIFEST_ENV}=\"$2\"\nexec \"{}\" --exact \"{PROCESS_HELPER_TEST}\" --ignored --nocapture --test-threads=1\n",
            root.display(),
            binary.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&supervisor, fs::Permissions::from_mode(0o700)).unwrap();
    let fake_cli = root.join("fake-cli.sh");
    fs::write(
        &fake_cli,
        format!(
            "#!/bin/sh\nexport {PROCESS_ROOT_ENV}=\"{}\"\nexport {PROCESS_ROLE_ENV}=fake_cli\nexec \"{}\" --exact \"{PROCESS_HELPER_TEST}\" --ignored --nocapture --test-threads=1\n",
            root.display(),
            binary.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&fake_cli, fs::Permissions::from_mode(0o700)).unwrap();
    let supervisor_contents = fs::read_to_string(&supervisor).unwrap();
    let binary_text = binary.to_string_lossy();
    assert!(supervisor_contents.contains(&*binary_text));
}

#[cfg(all(unix, target_os = "macos"))]
fn file_sha256(path: &Path) -> String {
    let mut file = File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let size = file.read(&mut buffer).unwrap();
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(unix)]
fn start_helper(root: &Path, role: &str) -> Child {
    helper_command(root, role)
        .stdout(File::create(root.join(format!("{role}.stdout"))).unwrap())
        .stderr(File::create(root.join(format!("{role}.stderr"))).unwrap())
        .spawn()
        .unwrap()
}

#[cfg(unix)]
fn process_diagnostics(root: &Path) -> String {
    fn visit(path: &Path, output: &mut String) {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, output);
            } else if matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some(
                    "macos-job.stdout"
                        | "macos-job.stderr"
                        | "supervisor.stderr"
                        | "launchctl-print.txt"
                )
            ) {
                output.push_str(&format!(
                    "\n{}:\n{}",
                    path.display(),
                    fs::read_to_string(&path).unwrap_or_default()
                ));
            }
        }
    }
    let mut output = String::new();
    visit(root, &mut output);
    output
}

#[cfg(unix)]
fn wait_child(child: &mut Child, root: &Path, role: &str) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "{role} 夹具失败\nstdout: {}\nstderr: {}",
                fs::read_to_string(root.join(format!("{role}.stdout"))).unwrap_or_default(),
                fs::read_to_string(root.join(format!("{role}.stderr"))).unwrap_or_default(),
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{role} 夹具等待超时\nstdout: {}\nstderr: {}\nhost stdout: {}\nhost stderr: {}\n文件: {:?}\n内层进程: {}",
            fs::read_to_string(root.join(format!("{role}.stdout"))).unwrap_or_default(),
            fs::read_to_string(root.join(format!("{role}.stderr"))).unwrap_or_default(),
            fs::read_to_string(root.join("host.stdout")).unwrap_or_default(),
            fs::read_to_string(root.join("host.stderr")).unwrap_or_default(),
            fs::read_dir(root)
                .map(|entries| entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name())
                    .collect::<Vec<_>>())
                .unwrap_or_default(),
            process_diagnostics(root),
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "等待夹具记录超时：{}",
            path.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn identity_alive(process_id: u32, start_time: u64, executable: &Path) -> bool {
    process_identity(process_id)
        .is_ok_and(|identity| identity.0 == start_time && identity.1 == executable)
}

#[cfg(unix)]
fn wait_identity_exit(process_id: u32, start_time: u64, executable: &Path) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    while identity_alive(process_id, start_time, executable) {
        assert!(Instant::now() < deadline, "夹具进程未退出：{process_id}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn terminate_identity(process_id: u32, start_time: u64, executable: &Path) {
    if identity_alive(process_id, start_time, executable) {
        // SAFETY: 发送信号前已用 PID、启动时间和可执行文件三元组核对本测试拥有的进程。
        unsafe {
            libc::kill(process_id as i32, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
struct ProcessCleanup {
    root: PathBuf,
}

#[cfg(unix)]
impl Drop for ProcessCleanup {
    fn drop(&mut self) {
        if let Ok(bytes) = fs::read(self.root.join("fake-ready.json"))
            && let Ok(ready) = serde_json::from_slice::<FakeProcessReady>(&bytes)
        {
            terminate_identity(
                ready.process_id,
                ready.process_start_time,
                &ready.executable,
            );
        }
        if let Ok(handoff) = serde_json::from_slice::<ProcessHandoff>(
            &fs::read(self.root.join("handoff.json")).unwrap_or_default(),
        ) && let Ok(Some(record)) = load_record(&self.root, handoff.generation)
            && let Ok(bytes) = read_private_record(&record.directory.join("ready.json"))
            && let Ok(ready) = serde_json::from_slice::<RuntimeHostReady>(&bytes)
        {
            terminate_identity(
                ready.process_id,
                ready.process_start_time,
                &ready.executable,
            );
        }
    }
}

#[cfg(unix)]
async fn fake_connection(
    root: PathBuf,
    native_state_dir: PathBuf,
    generation: Uuid,
) -> super::super::RuntimeConnection {
    let (controller, mut commands, sender, events) = super::super::channels(generation);
    let task = Box::pin(async move {
        let mut child = super::super::managed_process::spawn(
            &native_state_dir,
            generation,
            &root.join("fake-cli.sh"),
            &[],
            &root,
        )
        .await
        .map_err(|error| RuntimeError::Protocol(format!("fake managed spawn failed: {error}")))?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| RuntimeError::Protocol("fake cli missing stdin".to_owned()))?;
        let mut output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| RuntimeError::Protocol("fake cli missing stdout".to_owned()))?,
        );
        sender
            .send(RuntimeEvent {
                generation,
                native_session_id: Some("fake-native-session".to_owned()),
                kind: RuntimeEventKind::SessionReady {
                    verified_cli_version: Some("test-v2".to_owned()),
                    effective_permissions: serde_json::json!({"test": true}),
                },
            })
            .await
            .map_err(|_| RuntimeError::EventBackpressure)?;
        let mut submitted = 0_u8;
        while let Some(command) = commands.recv().await {
            let bytes = serde_json::to_vec(&command).map_err(|error| {
                RuntimeError::Protocol(format!("fake command encode failed: {error}"))
            })?;
            input.write_all(&bytes).await.map_err(|error| {
                RuntimeError::Protocol(format!("fake managed stdin write failed: {error}"))
            })?;
            input.write_all(b"\n").await.map_err(|error| {
                RuntimeError::Protocol(format!("fake managed stdin newline failed: {error}"))
            })?;
            input.flush().await.map_err(|error| {
                RuntimeError::Protocol(format!("fake managed stdin flush failed: {error}"))
            })?;
            let mut reply = String::new();
            loop {
                reply.clear();
                if output.read_line(&mut reply).await.map_err(|error| {
                    RuntimeError::Protocol(format!("fake managed stdout read failed: {error}"))
                })? == 0
                {
                    return Err(RuntimeError::Protocol(
                        "fake cli exited before ack".to_owned(),
                    ));
                }
                if let Some(acknowledged) = parse_fake_ack(&reply)? {
                    if acknowledged != command.message_id {
                        return Err(RuntimeError::Protocol(
                            "fake ack identity changed".to_owned(),
                        ));
                    }
                    break;
                }
            }
            let message_id = command.message_id;
            match command.action {
                RuntimeAction::Submit { .. } | RuntimeAction::Steer { .. } => {
                    submitted = submitted.saturating_add(1);
                    let turn_id = format!("turn-{message_id}");
                    sender
                        .send(RuntimeEvent {
                            generation,
                            native_session_id: Some("fake-native-session".to_owned()),
                            kind: RuntimeEventKind::MessageAccepted {
                                message_id,
                                turn_id: Some(turn_id.clone()),
                            },
                        })
                        .await
                        .map_err(|_| RuntimeError::EventBackpressure)?;
                    sender
                        .send(RuntimeEvent {
                            generation,
                            native_session_id: Some("fake-native-session".to_owned()),
                            kind: RuntimeEventKind::Progress {
                                turn_id: turn_id.clone(),
                                message: message_id.to_string(),
                            },
                        })
                        .await
                        .map_err(|_| RuntimeError::EventBackpressure)?;
                    if submitted == 2 && root.join("complete-after-second").is_file() {
                        sender
                            .send(RuntimeEvent {
                                generation,
                                native_session_id: Some("fake-native-session".to_owned()),
                                kind: RuntimeEventKind::TurnStarted {
                                    turn_id: turn_id.clone(),
                                },
                            })
                            .await
                            .map_err(|_| RuntimeError::EventBackpressure)?;
                        sender
                            .send(RuntimeEvent {
                                generation,
                                native_session_id: Some("fake-native-session".to_owned()),
                                kind: RuntimeEventKind::TextDelta {
                                    turn_id: turn_id.clone(),
                                    item_id: "offline-item".to_owned(),
                                    text: "offline-delta".to_owned(),
                                },
                            })
                            .await
                            .map_err(|_| RuntimeError::EventBackpressure)?;
                        sender
                            .send(RuntimeEvent {
                                generation,
                                native_session_id: Some("fake-native-session".to_owned()),
                                kind: RuntimeEventKind::TurnFinished {
                                    turn_id,
                                    outcome: TurnOutcome::Completed,
                                    output: "offline-final-result".to_owned(),
                                },
                            })
                            .await
                            .map_err(|_| RuntimeError::EventBackpressure)?;
                        break;
                    }
                }
                RuntimeAction::Interrupt { .. }
                | RuntimeAction::RespondApproval { .. }
                | RuntimeAction::RespondLocalTool { .. } => {
                    sender
                        .send(RuntimeEvent {
                            generation,
                            native_session_id: Some("fake-native-session".to_owned()),
                            kind: RuntimeEventKind::MessageAccepted {
                                message_id,
                                turn_id: None,
                            },
                        })
                        .await
                        .map_err(|_| RuntimeError::EventBackpressure)?;
                }
                RuntimeAction::Shutdown => break,
            }
        }
        drop(input);
        child.finish_after_stdin_close().await.map_err(|error| {
            RuntimeError::Protocol(format!("fake managed cleanup failed: {error}"))
        })?;
        Ok(())
    });
    super::super::RuntimeConnection {
        controller,
        events,
        task,
    }
}

#[cfg(unix)]
fn parse_fake_ack(line: &str) -> Result<Option<Uuid>, RuntimeError> {
    let mut markers = line.match_indices(FAKE_ACK_MARKER);
    let Some((offset, _)) = markers.next() else {
        return Ok(None);
    };
    if markers.next().is_some() {
        return Err(RuntimeError::Protocol(
            "fake ack marker repeated".to_owned(),
        ));
    }
    let value = line[offset + FAKE_ACK_MARKER.len()..].trim_end();
    let acknowledged = value
        .parse()
        .map_err(|error| RuntimeError::Protocol(format!("fake ack invalid: {error}")))?;
    Ok(Some(acknowledged))
}

#[cfg(unix)]
#[test]
fn fake_ack_parser_accepts_one_prefixed_marker_and_rejects_ambiguous_or_polluted_values() {
    let message_id = Uuid::new_v4();
    assert_eq!(
        parse_fake_ack(&format!(
            "test runtime_host_process_helper ... {FAKE_ACK_MARKER}{message_id}\n"
        ))
        .unwrap(),
        Some(message_id)
    );
    assert_eq!(parse_fake_ack("test helper output\n").unwrap(), None);
    assert!(
        parse_fake_ack(&format!(
            "{FAKE_ACK_MARKER}{message_id}{FAKE_ACK_MARKER}{message_id}\n"
        ))
        .is_err()
    );
    assert!(parse_fake_ack(&format!("{FAKE_ACK_MARKER}{message_id} polluted\n")).is_err());
}

#[cfg(unix)]
fn wait_for_events(
    client: &RuntimeHostClient,
    after_sequence: u64,
    message_id: Uuid,
) -> Vec<HostedEvent> {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let expected_turn = format!("turn-{message_id}");
    let expected_progress = message_id.to_string();
    loop {
        let events = block_on(client.pull_events(after_sequence)).unwrap();
        let accepted = events.iter().any(|event| {
            matches!(
                &event.event.kind,
                RuntimeEventKind::MessageAccepted {
                    message_id: actual,
                    turn_id: Some(turn_id),
                } if *actual == message_id && turn_id == &expected_turn
            )
        });
        let progressed = events.iter().any(|event| {
            matches!(
                &event.event.kind,
                RuntimeEventKind::Progress { turn_id, message }
                    if turn_id == &expected_turn && message == &expected_progress
            )
        });
        // 假适配器的 ACK 与 Progress 分别落盘；收齐本次结果后才能设置旧 GUI 的交接水位。
        if accepted && progressed {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "等待假 CLI 的 ACK 与 Progress 超时：message_id={message_id}，after_sequence={after_sequence}，accepted={accepted}，progressed={progressed}，events={events:#?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn non_failure_disposition_rank(disposition: HostCommandDisposition) -> Option<u8> {
    match disposition {
        HostCommandDisposition::Recorded => Some(0),
        HostCommandDisposition::DeliveredToAdapter => Some(1),
        HostCommandDisposition::NativeAccepted => Some(2),
        HostCommandDisposition::Failed => None,
    }
}

#[cfg(unix)]
fn run_old_gui(root: &Path) {
    let generation = Uuid::new_v4();
    let record = create_record(
        "process-task".to_owned(),
        1,
        Harness::Codex,
        process_options(root, generation),
    )
    .unwrap();
    let mut host = helper_command(root, "host");
    host.env(PROCESS_MANIFEST_ENV, record.directory.join("manifest.json"))
        .stdout(File::create(root.join("host.stdout")).unwrap())
        .stderr(File::create(root.join("host.stderr")).unwrap());
    drop(host.spawn().unwrap());
    wait_for_file(&record.directory.join("ready.json"));
    wait_for_file(&root.join("fake-ready.json"));

    let mut client = block_on(connect_verified(record)).unwrap();
    assert_eq!(block_on(client.claim_owner(0)).unwrap(), 1);
    let first_client_id = client.client_id;
    let first_message = Uuid::new_v4();
    let first = RuntimeCommand {
        generation,
        message_id: first_message,
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("first".to_owned())],
        },
    };
    block_on(client.send_command(first.clone())).unwrap();
    let first_events = wait_for_events(&client, 0, first_message);
    let acknowledged_sequence = first_events.last().unwrap().sequence;
    block_on(client.acknowledge_events(acknowledged_sequence)).unwrap();

    let second_message = Uuid::new_v4();
    let second = RuntimeCommand {
        generation,
        message_id: second_message,
        action: RuntimeAction::Submit {
            input: vec![InputContent::Text("second".to_owned())],
        },
    };
    let original = block_on(client.send_command(second.clone())).unwrap();
    let retry = block_on(client.send_command(second)).unwrap();
    assert_eq!(original.message_id, retry.message_id);
    assert_eq!(original.digest, retry.digest);
    let original_rank =
        non_failure_disposition_rank(original.disposition).expect("假 CLI 的首次命令状态不得失败");
    let retry_rank =
        non_failure_disposition_rank(retry.disposition).expect("假 CLI 的重试命令状态不得失败");
    assert!(
        retry_rank >= original_rank,
        "网络重试只能返回相同或更强的持久命令状态"
    );
    wait_for_events(&client, acknowledged_sequence, second_message);
    if root.join("complete-after-second").is_file() {
        write_new_record(&root.join("second-accepted-observed"), b"observed").unwrap();
    }
    let commands = fs::read_to_string(root.join("commands.log")).unwrap();
    let commands: Vec<_> = commands.lines().collect();
    assert_eq!(
        commands
            .iter()
            .filter(|message| **message == first_message.to_string())
            .count(),
        1,
        "首个输入必须只投递一次"
    );
    assert_eq!(
        commands
            .iter()
            .filter(|message| **message == second_message.to_string())
            .count(),
        1,
        "同一 message_id 的网络重试不得再次投递"
    );
    write_new_record(
        &root.join("handoff.json"),
        &serde_json::to_vec(&ProcessHandoff {
            generation,
            first_client_id,
            first_message,
            second_message,
            acknowledged_sequence,
        })
        .unwrap(),
    )
    .unwrap();
}

#[cfg(unix)]
fn run_new_gui(root: &Path) {
    let handoff: ProcessHandoff =
        serde_json::from_slice(&fs::read(root.join("handoff.json")).unwrap()).unwrap();
    let state = block_on(classify_startup(root, handoff.generation)).unwrap();
    let RuntimeHostStartupState::LiveReattachable(mut client) = state else {
        panic!("旧 GUI 退出后宿主必须可重关联");
    };
    let inspection = block_on(client.inspect()).unwrap();
    let RuntimeHostResponse::Inspection {
        owner_epoch,
        acknowledged_sequence,
        ..
    } = inspection
    else {
        panic!("重关联前必须读取宿主所有者与 ACK 水位");
    };
    assert_eq!(owner_epoch, 1);
    assert_eq!(acknowledged_sequence, handoff.acknowledged_sequence);
    assert_eq!(block_on(client.claim_owner(owner_epoch)).unwrap(), 2);
    let stale = block_on(client.call(RuntimeHostRequest::PullEvents {
        token: client.record.manifest.token,
        task_id: client.record.manifest.task_id.clone(),
        runtime_generation: client.record.manifest.runtime_generation,
        owner_epoch: 1,
        client_id: handoff.first_client_id,
        after_sequence: 0,
    }))
    .unwrap();
    assert_eq!(
        stale,
        RuntimeHostResponse::Rejected {
            code: RuntimeHostRejection::StaleOwner
        }
    );

    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        let statuses =
            block_on(client.inspect_commands(vec![handoff.first_message, handoff.second_message]))
                .unwrap();
        if statuses.len() == 2
            && statuses
                .iter()
                .all(|status| status.disposition == HostCommandDisposition::NativeAccepted)
        {
            break;
        }
        assert!(Instant::now() < deadline, "等待命令原生 ACK 账本超时");
        thread::sleep(Duration::from_millis(20));
    }
    let events = block_on(client.pull_events(acknowledged_sequence)).unwrap();
    assert!(!events.is_empty());
    assert!(
        events.iter().all(|event| match &event.event.kind {
            RuntimeEventKind::MessageAccepted { message_id, .. } => {
                *message_id == handoff.second_message
            }
            RuntimeEventKind::Progress { message, .. } =>
                message == &handoff.second_message.to_string(),
            RuntimeEventKind::SessionReady { .. }
            | RuntimeEventKind::CommandDispatched { .. }
            | RuntimeEventKind::TurnStarted { .. }
            | RuntimeEventKind::InputJoined { .. }
            | RuntimeEventKind::TextDelta { .. }
            | RuntimeEventKind::ApprovalRequested { .. }
            | RuntimeEventKind::ApprovalResolved { .. }
            | RuntimeEventKind::ApprovalCancelled { .. }
            | RuntimeEventKind::LocalToolCancelled { .. }
            | RuntimeEventKind::LocalToolRequested { .. }
            | RuntimeEventKind::TurnFinished { .. }
            | RuntimeEventKind::RequestFailed { .. }
            | RuntimeEventKind::Disconnected { .. } => false,
        }),
        "新 GUI 只能收到第二条输入的未确认事件：handoff={handoff:?}，after_sequence={acknowledged_sequence}，events={events:#?}"
    );
    block_on(client.send_command(RuntimeCommand {
        generation: handoff.generation,
        message_id: Uuid::new_v4(),
        action: RuntimeAction::Shutdown,
    }))
    .unwrap();
}

#[cfg(unix)]
fn run_offline_gui(root: &Path) {
    let handoff: ProcessHandoff =
        serde_json::from_slice(&fs::read(root.join("handoff.json")).unwrap()).unwrap();
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let (receipt, pending_events, mut snapshot) = loop {
        match block_on(classify_startup(root, handoff.generation)).unwrap() {
            RuntimeHostStartupState::ExitedResumable {
                receipt,
                pending_events,
                snapshot,
            } => break (receipt, pending_events, snapshot),
            RuntimeHostStartupState::LiveReattachable(_) => {
                assert!(Instant::now() < deadline, "等待宿主真实退出回执超时");
                thread::sleep(Duration::from_millis(20));
            }
            RuntimeHostStartupState::Unconfirmed(reason) => {
                panic!("宿主正常完成后不得降级为未确认：{reason:?}")
            }
        }
    };
    assert_eq!(receipt.native_process, NativeProcessCompletion::Exited);
    assert_eq!(receipt.acknowledged_sequence, handoff.acknowledged_sequence);
    let offline_turn = format!("turn-{}", handoff.second_message);
    assert!(snapshot.active_turn_id.is_none());
    assert!(!snapshot.finished_turn_ids.contains(&offline_turn));
    assert_ne!(snapshot.output, "offline-final-result");
    assert!(pending_events.iter().any(|event| matches!(
        &event.event.kind,
        RuntimeEventKind::TurnStarted { turn_id } if turn_id == &offline_turn
    )));
    assert!(pending_events.iter().any(|event| matches!(
        &event.event.kind,
        RuntimeEventKind::TextDelta { turn_id, text, .. }
            if turn_id == &offline_turn && text == "offline-delta"
    )));
    assert!(pending_events.iter().any(|event| matches!(
        &event.event.kind,
        RuntimeEventKind::TurnFinished {
            outcome: TurnOutcome::Completed,
            output,
            ..
        } if output == "offline-final-result"
    )));
    for hosted in &pending_events {
        snapshot.apply_recovered_event(&hosted.event);
    }
    assert_eq!(snapshot.output, "offline-final-result");
    assert!(snapshot.active_turn_id.is_none());
    assert_eq!(
        snapshot.finished_turn_ids,
        std::collections::BTreeSet::from([offline_turn])
    );
    write_new_record(
        &root.join("offline-recovery.json"),
        &serde_json::to_vec(&OfflineRecovery {
            recovered_through: pending_events
                .last()
                .map_or(receipt.acknowledged_sequence, |event| event.sequence),
            output: snapshot.output,
            finished_turn_ids: snapshot.finished_turn_ids,
        })
        .unwrap(),
    )
    .unwrap();
}

#[cfg(unix)]
fn run_fake_cli(root: &Path) {
    let (process_start_time, executable) = process_identity(std::process::id()).unwrap();
    write_new_record(
        &root.join("fake-ready.json"),
        &serde_json::to_vec(&FakeProcessReady {
            process_id: std::process::id(),
            process_start_time,
            executable,
        })
        .unwrap(),
    )
    .unwrap();
    let mut submitted = 0_u8;
    for line in std::io::stdin().lock().lines() {
        let command: RuntimeCommand = serde_json::from_str(&line.unwrap()).unwrap();
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("commands.log"))
            .unwrap();
        writeln!(log, "{}", command.message_id).unwrap();
        log.sync_all().unwrap();
        println!("INFINISHELL_FAKE_ACK:{}", command.message_id);
        std::io::stdout().flush().unwrap();
        if matches!(
            &command.action,
            RuntimeAction::Submit { .. } | RuntimeAction::Steer { .. }
        ) {
            submitted = submitted.saturating_add(1);
        }
        if command.action == RuntimeAction::Shutdown {
            break;
        }
        if submitted == 2 && root.join("complete-after-second").is_file() {
            wait_for_file(&root.join("second-accepted-observed"));
            break;
        }
    }
}

#[cfg(unix)]
fn run_host(root: &Path) {
    let manifest = PathBuf::from(std::env::var_os(PROCESS_MANIFEST_ENV).unwrap());
    let record = record_from_manifest_path(&manifest).unwrap();
    let state = Arc::new(Mutex::new(
        RuntimeHostState::new(
            &record.directory,
            record.manifest.clone(),
            record.manifest_sha256.clone(),
        )
        .unwrap(),
    ));
    let native_state_dir = record.directory.join("native");
    create_private_directory(&native_state_dir).unwrap();
    let connection = block_on(fake_connection(
        root.to_owned(),
        native_state_dir,
        record.manifest.runtime_generation,
    ));
    let outcome = run_connection(&record, state.clone(), connection).unwrap();
    let exit = write_exit_receipt(
        &record,
        &state,
        true,
        outcome.adapter.is_ok(),
        outcome.journal.is_ok(),
    );
    outcome.journal.unwrap();
    exit.unwrap();
    outcome.adapter.unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "仅由真实多进程宿主测试派生"]
fn runtime_host_process_helper() {
    let root = PathBuf::from(std::env::var_os(PROCESS_ROOT_ENV).unwrap())
        .canonicalize()
        .unwrap();
    match std::env::var(PROCESS_ROLE_ENV).unwrap().as_str() {
        "old_gui" => run_old_gui(&root),
        "new_gui" => run_new_gui(&root),
        "offline_gui" => run_offline_gui(&root),
        "host" => run_host(&root),
        "startup_stall" => {
            write_new_record(&root.join("startup-stall-ready"), b"ready").unwrap();
            loop {
                thread::sleep(Duration::from_secs(1));
            }
        }
        "fake_cli" => run_fake_cli(&root),
        "managed_worker" => super::super::managed_process::run_worker(
            &PathBuf::from(std::env::var_os(PROCESS_MANIFEST_ENV).unwrap()),
            false,
        )
        .unwrap(),
        "managed_execute" => super::super::managed_process::run_worker(
            &PathBuf::from(std::env::var_os(PROCESS_MANIFEST_ENV).unwrap()),
            true,
        )
        .unwrap(),
        role => panic!("未知进程夹具角色：{role}"),
    }
}

#[cfg(unix)]
#[test]
fn real_process_host_survives_gui_exit_and_reconnects_without_replay() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    prepare_process_wrappers(&root);
    let _cleanup = ProcessCleanup { root: root.clone() };
    let mut old_gui = start_helper(&root, "old_gui");
    wait_child(&mut old_gui, &root, "old_gui");
    let handoff: ProcessHandoff =
        serde_json::from_slice(&fs::read(root.join("handoff.json")).unwrap()).unwrap();
    let record = load_record(&root, handoff.generation).unwrap().unwrap();
    let ready: RuntimeHostReady =
        serde_json::from_slice(&read_private_record(&record.directory.join("ready.json")).unwrap())
            .unwrap();
    let fake: FakeProcessReady =
        serde_json::from_slice(&fs::read(root.join("fake-ready.json")).unwrap()).unwrap();
    assert!(identity_alive(
        ready.process_id,
        ready.process_start_time,
        &ready.executable
    ));
    assert!(identity_alive(
        fake.process_id,
        fake.process_start_time,
        &fake.executable
    ));

    let mut new_gui = start_helper(&root, "new_gui");
    wait_child(&mut new_gui, &root, "new_gui");
    wait_identity_exit(
        ready.process_id,
        ready.process_start_time,
        &ready.executable,
    );
    wait_identity_exit(fake.process_id, fake.process_start_time, &fake.executable);
    let commands = fs::read_to_string(root.join("commands.log")).unwrap();
    let commands: Vec<_> = commands.lines().collect();
    assert_eq!(commands.len(), 3, "两个输入和一次关闭必须各投递一次");
    assert_eq!(
        commands
            .iter()
            .filter(|message| **message == handoff.first_message.to_string())
            .count(),
        1
    );
    assert_eq!(
        commands
            .iter()
            .filter(|message| **message == handoff.second_message.to_string())
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn host_crash_is_unconfirmed_and_never_resumes_history() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    prepare_process_wrappers(&root);
    let _cleanup = ProcessCleanup { root: root.clone() };
    let mut old_gui = start_helper(&root, "old_gui");
    wait_child(&mut old_gui, &root, "old_gui");
    let handoff: ProcessHandoff =
        serde_json::from_slice(&fs::read(root.join("handoff.json")).unwrap()).unwrap();
    let record = load_record(&root, handoff.generation).unwrap().unwrap();
    let ready: RuntimeHostReady =
        serde_json::from_slice(&read_private_record(&record.directory.join("ready.json")).unwrap())
            .unwrap();
    let fake: FakeProcessReady =
        serde_json::from_slice(&fs::read(root.join("fake-ready.json")).unwrap()).unwrap();
    terminate_identity(
        ready.process_id,
        ready.process_start_time,
        &ready.executable,
    );
    wait_identity_exit(
        ready.process_id,
        ready.process_start_time,
        &ready.executable,
    );
    wait_identity_exit(fake.process_id, fake.process_start_time, &fake.executable);
    assert!(matches!(
        block_on(classify_startup(&root, handoff.generation)).unwrap(),
        RuntimeHostStartupState::Unconfirmed(RuntimeHostUnconfirmed::HostUnavailable)
    ));
}

#[cfg(unix)]
#[test]
fn completed_host_is_recovered_offline_without_reexecuting_commands() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    prepare_process_wrappers(&root);
    fs::write(root.join("complete-after-second"), b"1").unwrap();
    let _cleanup = ProcessCleanup { root: root.clone() };
    let mut old_gui = start_helper(&root, "old_gui");
    wait_child(&mut old_gui, &root, "old_gui");
    let handoff: ProcessHandoff =
        serde_json::from_slice(&fs::read(root.join("handoff.json")).unwrap()).unwrap();
    let record = load_record(&root, handoff.generation).unwrap().unwrap();
    let ready: RuntimeHostReady =
        serde_json::from_slice(&read_private_record(&record.directory.join("ready.json")).unwrap())
            .unwrap();
    wait_identity_exit(
        ready.process_id,
        ready.process_start_time,
        &ready.executable,
    );

    let mut offline_gui = start_helper(&root, "offline_gui");
    wait_child(&mut offline_gui, &root, "offline_gui");
    let recovered: OfflineRecovery =
        serde_json::from_slice(&fs::read(root.join("offline-recovery.json")).unwrap()).unwrap();
    assert_eq!(recovered.output, "offline-final-result");
    assert_eq!(
        recovered.finished_turn_ids,
        std::collections::BTreeSet::from([format!("turn-{}", handoff.second_message)])
    );
    let commands = fs::read_to_string(root.join("commands.log")).unwrap();
    let commands: Vec<_> = commands.lines().collect();
    assert_eq!(commands.len(), 2, "离线回收不得重投任何旧命令");
    assert_eq!(
        commands
            .iter()
            .filter(|message| **message == handoff.first_message.to_string())
            .count(),
        1
    );
    assert_eq!(
        commands
            .iter()
            .filter(|message| **message == handoff.second_message.to_string())
            .count(),
        1
    );
}
