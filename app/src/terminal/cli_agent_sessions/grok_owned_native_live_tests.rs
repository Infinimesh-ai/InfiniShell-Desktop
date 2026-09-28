//! 显式隔离运行器提供真实 shell PTY；验证生产启动、侧车与 SQLite，不代替 GUI 验收。

use std::env;
use std::os::unix::io::{AsRawFd as _, FromRawFd as _};
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::{Duration, Instant};

use futures::executor::block_on;
use serde_json::{Value, json};
use warpui::App;

use super::*;
use crate::persistence::ModelEvent;
use crate::persistence::local_cli_tasks::grok_terminal::{
    GrokTerminalDeliveryStatus, GrokTerminalOutcome, GrokTerminalOwner,
};
use crate::persistence::local_cli_tasks::{
    checkpoint_task, checkpoint_task_with_message, load_task_generations, load_task_messages,
    load_tasks,
};
use crate::persistence::model::{
    LocalCliMessage, LocalCliMessageState, LocalCliTask, LocalCliTaskState,
};
use crate::terminal::cli_agent_sessions::GrokPermissionEvidence;
use crate::terminal::cli_agent_sessions::event::{CLI_AGENT_NOTIFICATION_SENTINEL, parse_event};
use crate::terminal::cli_agent_sessions::grok_owned_worker::{
    GrokOwnedInputLease, GrokOwnedWorker, GrokOwnedWorkerEvent, GrokOwnedWorkerStop,
    recover_historical_results,
};

fn write_evidence(root: &Path, name: &str, value: Value) {
    write_new(
        &root.join(name),
        &serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
}

fn observation(root: &Path, session_id: Uuid, cwd: &Path) -> GrokPermissionObservation {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Ok(bytes) = fs::read_to_string(root.join("hooks.ndjson")) {
            for line in bytes.lines() {
                let Some(event) = parse_event(Some(CLI_AGENT_NOTIFICATION_SENTINEL), line) else {
                    continue;
                };
                if event.event
                    != crate::terminal::cli_agent_sessions::event::CLIAgentEventType::SessionStart
                {
                    continue;
                }
                let mut evidence = GrokPermissionEvidence::default();
                evidence.observe(&event, Some(&session_id.to_string()), cwd.to_str());
                if let GrokPermissionEvidence::Observed(observed) = evidence {
                    assert_eq!(observed.mode, "default");
                    return observed;
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("未收到本次原生 SessionStart/default 权限证据");
}

fn receive_event(
    events: &async_channel::Receiver<GrokOwnedWorkerEvent>,
    timeout: Duration,
) -> Result<GrokOwnedWorkerEvent, async_channel::TryRecvError> {
    // 仅原生验收线程使用有界等待；产品 GUI 接收器必须使用 recv().await。
    let deadline = Instant::now() + timeout;
    loop {
        match events.try_recv() {
            Err(async_channel::TryRecvError::Empty) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
}

#[test]
#[ignore = "由隔离运行器启动真实本地 PTY；两轮已授权模型调用，不能作为 GUI 或跨平台验收"]
fn grok_owned_native_two_turns_and_duplicate_guard() {
    App::test((), |mut app| async move {
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let root = PathBuf::from(env::var_os("INFINISHELL_GROK_OWNED_LIVE_ROOT").unwrap())
            .canonicalize()
            .unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".owned-live")).unwrap(),
            "grok-owned-native-v1\n"
        );
        assert!(!root.starts_with("/Volumes"));
        let cold = env::var_os("INFINISHELL_GROK_OWNED_PAUSE_AFTER_ACK").is_some();
        let setup: Value =
            serde_json::from_slice(&read_private(&root.join("pty.json")).unwrap()).unwrap();
        let master_fd = i32::try_from(setup["master_fd"].as_i64().unwrap()).unwrap();
        assert!(master_fd > 2);
        let master = unsafe { File::from_raw_fd(master_fd) };
        assert_eq!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
        let pty = LocalPtyIdentity::capture(
            u32::try_from(setup["shell_pid"].as_u64().unwrap()).unwrap(),
            &master,
        )
        .unwrap();
        let executable = PathBuf::from(env::var_os("INFINISHELL_GROK_LIVE_EXECUTABLE").unwrap());
        let app_executable =
            PathBuf::from(env::var_os("INFINISHELL_GROK_OWNED_APP_EXECUTABLE").unwrap());
        let cwd = root.join("project").canonicalize().unwrap();
        let mut launch = GrokOwnedLaunch::prepare(
            &executable,
            &app_executable,
            &cwd,
            &root,
            GrokOwnedPty::from_local_pty(pty).unwrap(),
        )
        .unwrap();
        let binding_id = Uuid::new_v4();
        let permission_revision = Uuid::new_v4();
        let mut owner = GrokTerminalOwner {
            version: 1,
            launch_id: launch.launch_id(),
            binding_id,
            session_id: launch.session_id(),
            input_revision: Uuid::new_v4(),
            permission_revision,
            cli_version: "1.0.41".into(),
            model_id: "grok-4.7".into(),
            permission_mode: "default".into(),
        };
        let writer = crate::persistence::start_test_writer(&root.join("tasks.sqlite")).unwrap();
        let mut task = LocalCliTask {
            version: 1,
            task_id: Uuid::new_v4().to_string(),
            parent_task_id: None,
            parent_generation: None,
            harness: "grok".into(),
            working_directory: cwd.to_string_lossy().into_owned(),
            config_json: json!({"execution_kind":"grok_owned_terminal", "grok_terminal":owner,
                "launch_manifest":launch.manifest_path(),"launch_sha256":launch.manifest_sha256()})
            .to_string(),
            native_session_id: Some(launch.session_id().to_string()),
            generation: 1,
            revision: 0,
            state: LocalCliTaskState::Queued,
            result: None,
            terminal_evidence: None,
        };
        block_on(checkpoint_task(&writer.sender, task.clone(), None).unwrap())
            .unwrap()
            .unwrap();
        app.update(|ctx| launch.reserve_launch(ctx).unwrap());
        let argv: Vec<_> = launch
            .take_launch_argv()
            .unwrap()
            .into_iter()
            .map(|value| value.into_string().unwrap())
            .collect();
        write_evidence(
            &root,
            "launch-command.json",
            json!({"argv":argv,"launch_id":launch.launch_id(),
            "session_id":launch.session_id(),"manifest_path":launch.manifest_path(),
            "manifest_sha256":launch.manifest_sha256()}),
        );
        let observed = observation(&root, launch.session_id(), &cwd);
        let mut native_prompt_ids = Vec::new();
        for turn in 1..=2 {
            owner.input_revision = Uuid::new_v4();
            task.revision += 1;
            task.state = LocalCliTaskState::Running;
            let mut config: Value = serde_json::from_str(&task.config_json).unwrap();
            config["grok_terminal"] = json!(owner);
            task.config_json = config.to_string();
            let message = LocalCliMessage {
                version: 1,
                message_id: Uuid::new_v4().to_string(),
                sender_task_id: task.task_id.clone(),
                recipient_task_id: task.task_id.clone(),
                sender_generation: task.generation,
                recipient_generation: task.generation,
                subject: crate::persistence::local_cli_tasks::grok_terminal::INPUT_SUBJECT.into(),
                body: format!(
                    "不要使用任何工具，不要修改文件。这是第 {turn} 轮中文输入。只回复 OWNED_{turn}_{}。",
                    launch.launch_id()
                ),
                state: LocalCliMessageState::Queued,
                receipt_kind: None,
            };
            block_on(
                checkpoint_task_with_message(
                    &writer.sender,
                    task.clone(),
                    Some(task.generation),
                    message.clone(),
                )
                .unwrap(),
            )
            .unwrap()
            .unwrap();
            let binding = launch
                .bind(binding_id, permission_revision, &observed)
                .unwrap();
            let lease =
                GrokOwnedInputLease::new(binding_id, owner.input_revision, permission_revision)
                    .unwrap();
            let (worker, events) = GrokOwnedWorker::start(
                binding,
                task.clone(),
                message.clone(),
                lease,
                writer.sender.clone(),
            )
            .unwrap();
            if cold {
                let acknowledged = loop {
                    match receive_event(&events, Duration::from_secs(100)) {
                        Ok(GrokOwnedWorkerEvent::Claimed(record)) => {
                            assert_eq!(record.message.state, LocalCliMessageState::Sent);
                        }
                        Ok(GrokOwnedWorkerEvent::Acknowledged(record)) => break record,
                        Ok(GrokOwnedWorkerEvent::NativePermissionPending { .. }) => {
                            panic!("冷恢复验收不得代答原生审批")
                        }
                        Ok(GrokOwnedWorkerEvent::ResultSaved(_))
                        | Ok(GrokOwnedWorkerEvent::ResultUnverified)
                        | Ok(GrokOwnedWorkerEvent::Finished(_))
                        | Ok(GrokOwnedWorkerEvent::Stopped { .. })
                        | Err(_) => panic!("冷恢复断点前未收到精确原生 ACK"),
                    }
                };
                assert_eq!(
                    acknowledged.message.state,
                    LocalCliMessageState::Acknowledged
                );
                let GrokTerminalDeliveryStatus::Finished {
                    native_prompt_id,
                    outcome: GrokTerminalOutcome::EndTurn,
                } = acknowledged.grok_terminal_delivery.as_ref().unwrap().status
                else {
                    panic!("冷恢复断点缺少 EndTurn 回执");
                };
                native_prompt_ids.push(native_prompt_id);
                drop(worker);
                assert!(matches!(
                    receive_event(&events, Duration::from_secs(5)).unwrap(),
                    GrokOwnedWorkerEvent::Stopped {
                        reason: GrokOwnedWorkerStop::Revoked,
                        ..
                    }
                ));
                let input = block_on(
                    crate::persistence::local_cli_tasks::grok_terminal::load_input(
                        &writer.sender,
                        Uuid::parse_str(&message.message_id).unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap()
                .unwrap()
                .unwrap();
                assert_eq!(input.message.state, LocalCliMessageState::Acknowledged);
                assert!(input.grok_terminal_result.is_none());
                let persisted = block_on(load_tasks(&writer.sender, false).unwrap())
                    .unwrap()
                    .unwrap();
                assert!(
                    persisted
                        .iter()
                        .any(|saved| { saved.task_id == task.task_id && saved.result.is_none() })
                );
                if turn == 1 {
                    task.revision += 1;
                    task.state = LocalCliTaskState::Disconnected;
                    block_on(checkpoint_task(&writer.sender, task.clone(), Some(1)).unwrap())
                        .unwrap()
                        .unwrap();
                    task.generation = 2;
                    task.revision = 0;
                    task.state = LocalCliTaskState::Queued;
                    block_on(checkpoint_task(&writer.sender, task.clone(), Some(1)).unwrap())
                        .unwrap()
                        .unwrap();
                }
                write_evidence(
                    &root,
                    &format!("cold-turn-{turn}-ack.safe.json"),
                    json!({"generation":turn,"acknowledged":true,"result_missing":true,
                        "native_outcome":"end_turn"}),
                );
                continue;
            }
            let deadline = Instant::now() + Duration::from_secs(100);
            let mut claimed = None;
            let mut result_saved = false;
            let mut acknowledged_at = None;
            let mut result_delay_ms = None;
            let finished = loop {
                assert!(Instant::now() < deadline, "原生输入尚未完成");
                match receive_event(&events, Duration::from_secs(1)) {
                    Ok(GrokOwnedWorkerEvent::Claimed(record)) => {
                        assert!(claimed.is_none());
                        assert_eq!(record.message.state, LocalCliMessageState::Sent);
                        assert_eq!(
                            record.grok_terminal_delivery.as_ref().unwrap().status,
                            GrokTerminalDeliveryStatus::Unknown
                        );
                        write_evidence(&root, &format!("turn-{turn}-claimed.json"), json!(record));
                        claimed = Some(record);
                    }
                    Ok(GrokOwnedWorkerEvent::Acknowledged(record)) => {
                        assert!(acknowledged_at.is_none());
                        assert_eq!(record.message.state, LocalCliMessageState::Acknowledged);
                        acknowledged_at = Some(Instant::now());
                    }
                    Ok(GrokOwnedWorkerEvent::ResultSaved(saved)) => {
                        assert!(!result_saved);
                        assert_eq!(saved.generation, task.generation);
                        let expected = format!("OWNED_{turn}_{}", launch.launch_id());
                        assert_eq!(
                            saved.result.as_deref().map(str::trim),
                            Some(expected.as_str())
                        );
                        result_delay_ms = Some(
                            u64::try_from(
                                Instant::now()
                                    .duration_since(acknowledged_at.unwrap())
                                    .as_millis(),
                            )
                            .unwrap(),
                        );
                        task = saved;
                        result_saved = true;
                    }
                    Ok(GrokOwnedWorkerEvent::Finished(record)) => break record,
                    Ok(GrokOwnedWorkerEvent::ResultUnverified) => {
                        panic!("原生回合已 ACK，但结果未核验保存")
                    }
                    Ok(GrokOwnedWorkerEvent::NativePermissionPending { .. }) => {
                        panic!("出现未授权的工具审批；验收不会代答")
                    }
                    Ok(GrokOwnedWorkerEvent::Stopped { reason, record }) => {
                        write_evidence(
                            &root,
                            &format!("turn-{turn}-stopped.json"),
                            json!({"reason":format!("{reason:?}"),"record":record}),
                        );
                        panic!("侧车未完成: {reason:?}");
                    }
                    Err(async_channel::TryRecvError::Empty) => {}
                    Err(async_channel::TryRecvError::Closed) => {
                        panic!("侧车事件通道关闭")
                    }
                }
            };
            assert!(result_saved, "完整原生历史结果未保存到任务");
            write_evidence(
                &root,
                &format!("turn-{turn}-result.safe.json"),
                json!({
                    "ack_to_result_saved_ms":result_delay_ms,
                    "result_bytes":task.result.as_deref().map(str::len),
                    "result_matches_expected":true
                }),
            );
            let loaded = block_on(load_tasks(&writer.sender, false).unwrap())
                .unwrap()
                .unwrap();
            assert_eq!(
                loaded.iter().find(|saved| saved.task_id == task.task_id),
                Some(&task)
            );
            drop(worker);
            let claimed = claimed.unwrap();
            let delivery = finished.grok_terminal_delivery.as_ref().unwrap();
            assert_eq!(
                delivery.rpc_id,
                claimed.grok_terminal_delivery.unwrap().rpc_id
            );
            assert_eq!(finished.message.state, LocalCliMessageState::Acknowledged);
            let GrokTerminalDeliveryStatus::Finished {
                native_prompt_id,
                outcome: GrokTerminalOutcome::EndTurn,
            } = delivery.status
            else {
                panic!("没有收到精确原生终态");
            };
            assert!(!native_prompt_ids.contains(&native_prompt_id));
            native_prompt_ids.push(native_prompt_id);
            write_evidence(
                &root,
                &format!("turn-{turn}-finished.json"),
                json!(finished),
            );
            let binding = launch
                .bind(binding_id, permission_revision, &observed)
                .unwrap();
            let lease =
                GrokOwnedInputLease::new(binding_id, owner.input_revision, permission_revision)
                    .unwrap();
            let (duplicate, events) = GrokOwnedWorker::start(
                binding,
                task.clone(),
                message,
                lease,
                writer.sender.clone(),
            )
            .unwrap();
            assert!(matches!(
                receive_event(&events, Duration::from_secs(5)).unwrap(),
                GrokOwnedWorkerEvent::Stopped {
                    reason: GrokOwnedWorkerStop::AlreadyClaimed,
                    ..
                }
            ));
            drop(duplicate);
        }
        if cold {
            assert_eq!(task.generation, 2);
            assert!(task.result.is_none());
            write_evidence(
                &root,
                "cold-ack.safe.json",
                json!({"version":"1.0.41","model":"grok-4.7",
                    "task_id":task.task_id,"native_prompt_ids":native_prompt_ids,
                    "two_acknowledged_turns_without_results":true,"application_process_may_exit":true}),
            );
            writer.sender.send(ModelEvent::Terminate).unwrap();
            writer.handle.join().unwrap();
            return;
        }
        let mut restored =
            GrokOwnedLaunch::restore(&launch.manifest_path(), launch.manifest_sha256()).unwrap();
        assert!(restored.take_launch_argv().is_err());
        write_evidence(
            &root,
            "native-result.json",
            json!({"version":"1.0.41","model":"grok-4.7",
            "native_prompt_ids":native_prompt_ids,"task_id":task.task_id,"duplicate_rejected":true,"restored_launch_cannot_dispatch":true,
            "gui_verified":false,"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH}),
        );
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    });
}

#[test]
#[ignore = "需由隔离原生运行器确认 TUI 与 leader 已退出；不提交模型输入"]
fn grok_owned_native_recovery_after_exit() {
    App::test((), |mut app| async move {
        let root = PathBuf::from(env::var_os("INFINISHELL_GROK_OWNED_LIVE_ROOT").unwrap())
            .canonicalize()
            .unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".owned-live")).unwrap(),
            "grok-owned-native-v1\n"
        );
        let command: Value =
            serde_json::from_slice(&read_private(&root.join("launch-command.json")).unwrap())
                .unwrap();
        let path = PathBuf::from(command["manifest_path"].as_str().unwrap());
        assert!(path.starts_with(root.join("grok-owned-terminal")));
        let sha = command["manifest_sha256"].as_str().unwrap();
        let mut launch = GrokOwnedLaunch::restore(&path, sha).unwrap();
        let socket_root = launch.manifest.socket_path.parent().unwrap().to_owned();
        assert!(launch.take_launch_argv().is_err());
        app.update(|ctx| launch.release_after_exit(ctx).unwrap());
        assert!(path.exists());
        assert!(!socket_root.exists());
        let mut restored = GrokOwnedLaunch::restore(&path, sha).unwrap();
        assert!(restored.is_retired());
        assert!(restored.take_launch_argv().is_err());
        let receipt: Value =
            serde_json::from_slice(&read_private(&root.join("native-result.json")).unwrap())
                .unwrap();
        let writer = crate::persistence::start_test_writer(&root.join("tasks.sqlite")).unwrap();
        let loaded = block_on(load_tasks(&writer.sender, true).unwrap())
            .unwrap()
            .unwrap();
        let task_id = receipt["task_id"].as_str().unwrap();
        let task = loaded.iter().find(|task| task.task_id == task_id).unwrap();
        let expected = format!("OWNED_2_{}", launch.launch_id());
        assert_eq!(
            task.result.as_deref().map(str::trim),
            Some(expected.as_str())
        );
        assert_eq!(task.state, LocalCliTaskState::Disconnected);
        let generations =
            block_on(load_task_generations(&writer.sender, task.task_id.clone()).unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(
            generations.iter().find(|saved| saved.generation == 1),
            Some(task)
        );
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        write_evidence(
            &root,
            "native-recovery-result.json",
            json!({
                "passed": true,
                "manifest_retained": true,
                "socket_directory_removed": true,
                "retired_launch_not_replayed": true,
                "model_inputs": 0,
                "application_restart_verified": true,
                "persisted_result_reloaded": true
            }),
        );
    });
}

#[test]
#[ignore = "须由隔离运行器保持同一原生 TUI/leader 存活；仅对已 ACK 的两代历史执行只读补查"]
fn grok_owned_native_readonly_cold_recovery() {
    App::test((), |_| async move {
        let root = PathBuf::from(env::var_os("INFINISHELL_GROK_OWNED_LIVE_ROOT").unwrap())
            .canonicalize()
            .unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".owned-live")).unwrap(),
            "grok-owned-native-v1\n"
        );
        let receipt: Value =
            serde_json::from_slice(&read_private(&root.join("cold-ack.safe.json")).unwrap())
                .unwrap();
        assert_eq!(receipt["two_acknowledged_turns_without_results"], true);
        let command: Value =
            serde_json::from_slice(&read_private(&root.join("launch-command.json")).unwrap())
                .unwrap();
        let path = PathBuf::from(command["manifest_path"].as_str().unwrap());
        let sha = command["manifest_sha256"].as_str().unwrap();
        let writer = crate::persistence::start_test_writer(&root.join("tasks.sqlite")).unwrap();
        let loaded = block_on(load_tasks(&writer.sender, true).unwrap())
            .unwrap()
            .unwrap();
        let task = loaded
            .iter()
            .find(|task| task.task_id == receipt["task_id"].as_str().unwrap())
            .unwrap()
            .clone();
        assert_eq!(task.generation, 2);
        assert_eq!(task.state, LocalCliTaskState::Disconnected);
        assert!(task.result.is_none());
        let generations =
            block_on(load_task_generations(&writer.sender, task.task_id.clone()).unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(generations.len(), 2);
        assert!(generations.iter().all(|saved| saved.result.is_none()));
        let messages = block_on(load_task_messages(&writer.sender, task.task_id.clone()).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert!(
            messages
                .iter()
                .all(|message| message.state == LocalCliMessageState::Acknowledged)
        );
        let config: Value = serde_json::from_str(&task.config_json).unwrap();
        let owner: GrokTerminalOwner =
            serde_json::from_value(config["grok_terminal"].clone()).unwrap();
        let observed = observation(&root, owner.session_id, Path::new(&task.working_directory));
        let mut launch = GrokOwnedLaunch::restore(&path, sha).unwrap();
        let binding = launch
            .bind(owner.binding_id, owner.permission_revision, &observed)
            .unwrap();
        let blocked = AtomicBool::new(true);
        let refreshed = recover_historical_results(
            binding,
            task.clone(),
            owner.clone(),
            writer.sender.clone(),
            &blocked,
        )
        .unwrap()
        .unwrap();
        let expected_current = format!("OWNED_2_{}", launch.launch_id());
        assert_eq!(refreshed.generation, 2);
        assert_eq!(refreshed.state, LocalCliTaskState::Disconnected);
        assert_eq!(
            refreshed.result.as_deref().map(str::trim),
            Some(expected_current.as_str())
        );
        let current = block_on(load_tasks(&writer.sender, false).unwrap())
            .unwrap()
            .unwrap()
            .into_iter()
            .find(|saved| saved.task_id == task.task_id)
            .unwrap();
        assert_eq!(current, refreshed);
        let generations =
            block_on(load_task_generations(&writer.sender, task.task_id.clone()).unwrap())
                .unwrap()
                .unwrap();
        let old = generations
            .iter()
            .find(|saved| saved.generation == 1)
            .unwrap();
        let expected_old = format!("OWNED_1_{}", launch.launch_id());
        assert_eq!(old.state, LocalCliTaskState::Disconnected);
        assert_eq!(
            old.result.as_deref().map(str::trim),
            Some(expected_old.as_str())
        );
        assert_eq!(
            generations.iter().find(|saved| saved.generation == 2),
            Some(&current)
        );
        let messages_after =
            block_on(load_task_messages(&writer.sender, task.task_id.clone()).unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(messages_after, messages);
        for message in &messages_after {
            let record = block_on(
                crate::persistence::local_cli_tasks::grok_terminal::load_input(
                    &writer.sender,
                    Uuid::parse_str(&message.message_id).unwrap(),
                )
                .unwrap(),
            )
            .unwrap()
            .unwrap()
            .unwrap();
            assert_eq!(record.message.state, LocalCliMessageState::Acknowledged);
            let verified = record.grok_terminal_result.unwrap();
            let expected = format!(
                "OWNED_{}_{}",
                message.recipient_generation,
                launch.launch_id()
            );
            assert_eq!(verified.output.trim(), expected);
        }
        let binding = launch
            .bind(owner.binding_id, owner.permission_revision, &observed)
            .unwrap();
        assert!(
            recover_historical_results(
                binding,
                current.clone(),
                owner,
                writer.sender.clone(),
                &blocked,
            )
            .unwrap()
            .is_none()
        );
        write_evidence(
            &root,
            "cold-recovery.safe.json",
            json!({"old_generation_result_bytes":old.result.as_deref().map(str::len),
                "current_generation_result_bytes":current.result.as_deref().map(str::len),
                "state":"disconnected","messages_before":messages.len(),
                "messages_after":messages_after.len(),"readonly_recovery_idempotent":true,
                "old_and_current_result_match_native_turns":true}),
        );
        write_evidence(
            &root,
            "cold-native-result.json",
            json!({"task_id":task.task_id,"launch_id":launch.launch_id(),
                "generations":2,"readonly_recovery_verified":true}),
        );
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    });
}

#[test]
#[ignore = "由隔离运行器确认同一原生 TUI/leader 已退出后，重开 SQLite 验证冷补写结果"]
fn grok_owned_native_cold_after_exit() {
    App::test((), |mut app| async move {
        let root = PathBuf::from(env::var_os("INFINISHELL_GROK_OWNED_LIVE_ROOT").unwrap())
            .canonicalize()
            .unwrap();
        let command: Value =
            serde_json::from_slice(&read_private(&root.join("launch-command.json")).unwrap())
                .unwrap();
        let receipt: Value =
            serde_json::from_slice(&read_private(&root.join("cold-native-result.json")).unwrap())
                .unwrap();
        let path = PathBuf::from(command["manifest_path"].as_str().unwrap());
        let sha = command["manifest_sha256"].as_str().unwrap();
        let mut launch = GrokOwnedLaunch::restore(&path, sha).unwrap();
        let socket_root = launch.manifest.socket_path.parent().unwrap().to_owned();
        app.update(|ctx| launch.release_after_exit(ctx).unwrap());
        assert!(!socket_root.exists());
        let mut retired = GrokOwnedLaunch::restore(&path, sha).unwrap();
        assert!(retired.is_retired());
        assert!(retired.take_launch_argv().is_err());
        let writer = crate::persistence::start_test_writer(&root.join("tasks.sqlite")).unwrap();
        let loaded = block_on(load_tasks(&writer.sender, true).unwrap())
            .unwrap()
            .unwrap();
        let task = loaded
            .iter()
            .find(|task| task.task_id == receipt["task_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(task.generation, 2);
        assert_eq!(task.state, LocalCliTaskState::Disconnected);
        let expected = format!("OWNED_2_{}", launch.launch_id());
        assert_eq!(
            task.result.as_deref().map(str::trim),
            Some(expected.as_str())
        );
        let generations =
            block_on(load_task_generations(&writer.sender, task.task_id.clone()).unwrap())
                .unwrap()
                .unwrap();
        let old = generations
            .iter()
            .find(|saved| saved.generation == 1)
            .unwrap();
        let expected_old = format!("OWNED_1_{}", launch.launch_id());
        assert_eq!(
            old.result.as_deref().map(str::trim),
            Some(expected_old.as_str())
        );
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
        write_evidence(
            &root,
            "cold-after-exit.safe.json",
            json!({"passed":true,"native_exited_before_read":true,
                "persisted_old_and_current_results_reloaded":true,
                "retired_launch_not_replayed":true,"model_inputs":0}),
        );
    });
}
