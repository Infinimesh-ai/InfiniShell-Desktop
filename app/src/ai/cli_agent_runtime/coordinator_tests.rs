use std::time::Duration;

#[cfg(unix)]
use command::blocking::Command;
use diesel::{Connection, ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection};
use futures::executor::block_on;
use serde_json::{Value, json};
use uuid::Uuid;
use warpui::App;
use warpui::r#async::Timer;

use super::*;
use crate::ai::agent::{AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::cli_agent_runtime::conversation_bridge::local_parent_history_identity;
use crate::ai::cli_agent_runtime::{InputContent, channels};
use crate::ai::llms::LLMId;
use crate::ai::local_cli_mailbox::send_local_message_if_current;
use crate::persistence::local_cli_tasks::load_task_generations;
use crate::persistence::model::LocalCliReceiptKind;
use crate::persistence::schema::{local_cli_messages, local_cli_task_generations};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

#[test]
fn parent_message_validation_after_sent_and_capacity_wait_cannot_enqueue_an_old_action() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let history = BlocklistAIHistoryModel::handle(&app);
        let (conversation_id, identity, spawner) = terminal.update(&mut app, |terminal, ctx| {
            history.update(ctx, |history, ctx| {
                let conversation_id =
                    history.start_new_conversation(terminal.id(), false, false, false, ctx);
                let conversation = history.conversation_mut(&conversation_id).unwrap();
                conversation.append_root_exchange_for_test(parent_message_user_exchange());
                (
                    conversation_id,
                    local_parent_history_identity(conversation).unwrap(),
                    ctx.spawner(),
                )
            })
        });
        let directory = tempfile::tempdir().unwrap();
        let writer =
            crate::persistence::start_test_writer(&directory.path().join("messages.sqlite"))
                .unwrap();
        let mut parent = snapshot().task;
        parent.task_id = "parent".into();
        parent.parent_task_id = None;
        parent.parent_generation = None;
        parent.harness = "oz".into();
        parent.native_session_id = None;
        let child = LocalCliTask {
            task_id: "child".into(),
            parent_task_id: Some("parent".into()),
            parent_generation: Some(1),
            harness: "codex".into(),
            native_session_id: Some("child-native-session".into()),
            ..parent.clone()
        };
        for task in [parent, child] {
            checkpoint_task(&writer.sender, task, None)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
        }
        let token = Uuid::new_v4();
        let (commands, mut receiver) = mpsc::channel(1);
        let (reply, _reply) = oneshot::channel();
        commands
            .try_send(ManagedRequest {
                expected_generation: 1,
                from_mailbox: false,
                prepared_result: None,
                message_id: Uuid::new_v4(),
                action: RuntimeAction::Shutdown,
                reply,
            })
            .unwrap();
        let endpoint = ManagedTaskEndpoint {
            task_id: "child".into(),
            generation: 1,
            harness: Harness::Codex,
            runtime_generation: token,
            active_turn_id: None,
            commands,
        };
        let message = LocalCliMessage {
            version: 1,
            message_id: Uuid::new_v4().to_string(),
            sender_task_id: "parent".into(),
            recipient_task_id: "child".into(),
            sender_generation: 1,
            recipient_generation: 1,
            subject: "follow-up".into(),
            body: "旧轮指令不得入队".into(),
            state: LocalCliMessageState::Queued,
            receipt_kind: None,
        };
        let attempted = send_local_message_if_current(
            &writer.sender,
            endpoint.clone(),
            message.clone(),
            |prepared| async {
                spawner
                    .spawn(move |history, _| {
                        if history
                            .conversation(&conversation_id)
                            .and_then(local_parent_history_identity)
                            .as_ref()
                            != Some(&identity)
                        {
                            return Err("父用户轮已变化".into());
                        }
                        // 与生产路径一致，校验与提交之间没有 await。
                        Ok(prepared.commit())
                    })
                    .await
                    .map_err(|_| "父模型已关闭".to_owned())?
            },
        );
        let change_while_waiting = async {
            let mut sent = false;
            for _ in 0..250 {
                let saved = load_messages(&writer.sender, "child".into(), 1)
                    .unwrap()
                    .await
                    .unwrap()
                    .unwrap();
                if saved
                    .first()
                    .is_some_and(|message| message.state == LocalCliMessageState::Sent)
                {
                    sent = true;
                    break;
                }
                Timer::after(Duration::from_millis(10)).await;
            }
            assert!(sent, "消息必须先提交 Sent");
            history.update(&mut app, |history, _| {
                history
                    .conversation_mut(&conversation_id)
                    .unwrap()
                    .append_root_exchange_for_test(parent_message_user_exchange());
            });
            // 队列仍由先前请求占满，释放后才允许新消息取得容量。
            let previous = receiver.recv().await.unwrap();
            assert!(matches!(previous.action, RuntimeAction::Shutdown));
        };
        let (result, ()) = futures::join!(attempted, change_while_waiting);
        assert!(result.is_err());
        assert!(receiver.try_recv().is_err(), "旧用户轮的消息不能进入队列");
        let saved = load_messages(&writer.sender, "child".into(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].state, LocalCliMessageState::Sent);
        assert_eq!(saved[0].receipt_kind, None);
        let duplicate =
            send_local_message_if_current(&writer.sender, endpoint, message, |_| async {
                panic!("Sent 消息不得再次保留容量或调用派发闭包")
            })
            .await
            .unwrap();
        assert_eq!(duplicate, LocalCliMessageState::Sent);
        assert!(receiver.try_recv().is_err());
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    });
}

fn parent_message_user_exchange() -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![AIAgentInput::ResumeConversation {
            context: Vec::new().into(),
        }],
        output_status: AIAgentOutputStatus::Streaming { output: None },
        added_message_ids: Default::default(),
        start_time: chrono::Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-model"),
        cli_agent_model_id: LLMId::from("test-model"),
        computer_use_model_id: LLMId::from("test-model"),
        response_initiator: None,
    }
}

#[test]
fn resume_claims_unstarted_generation_but_rejects_incomplete_process_records() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    let runtime_generation = Uuid::new_v4();
    let mut prior = snapshot().task;
    prior.config_json = json!({"runtime_generation": runtime_generation}).to_string();
    let options = SessionOptions {
        executable: std::env::current_exe().unwrap(),
        cwd: directory.path().to_owned(),
        state_dir: directory.path().to_owned(),
        target: SessionTarget::Resume {
            native_session_id: prior.native_session_id.clone().unwrap(),
        },
        generation: Uuid::new_v4(),
        permission_policy: super::super::PermissionPolicy::Inherit,
        permission_ceiling: None,
        claude_profile: None,
        grok_profile: None,
        model: None,
        local_tools: None,
        selected_skills: Vec::new(),
    };
    block_on(async {
        checkpoint_task(&writer.sender, prior.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        prior.state = LocalCliTaskState::Disconnected;
        prior.revision += 1;
        checkpoint_task(&writer.sender, prior.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let next = next_task_generation(&prior).unwrap();
        verify_previous_process_exit(&writer.sender, &next, Some(1), &options)
            .await
            .unwrap();
        let receipt =
            super::super::managed_process::confirmed_exit(directory.path(), runtime_generation)
                .unwrap()
                .unwrap();
        assert_eq!(receipt.containment, "not_started");
        assert!(
            verify_previous_process_exit(&writer.sender, &next, Some(2), &options)
                .await
                .is_err()
        );
        let incomplete = Uuid::new_v4();
        std::fs::create_dir(
            directory
                .path()
                .join("cli-agent-processes")
                .join(incomplete.to_string()),
        )
        .unwrap();
        prior.config_json = json!({"runtime_generation": incomplete}).to_string();
        prior.revision += 1;
        checkpoint_task(&writer.sender, prior.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert!(
            verify_previous_process_exit(&writer.sender, &next, Some(1), &options)
                .await
                .is_err()
        );
        let stored = load_tasks(&writer.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored, [prior]);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

fn snapshot() -> ManagedTaskSnapshot {
    ManagedTaskSnapshot {
        task: LocalCliTask {
            version: 1,
            task_id: Uuid::new_v4().to_string(),
            parent_task_id: None,
            parent_generation: None,
            harness: "codex".into(),
            working_directory: std::env::temp_dir().to_string_lossy().into(),
            config_json: "{}".into(),
            native_session_id: Some(Uuid::new_v4().to_string()),
            generation: 1,
            revision: 0,
            state: LocalCliTaskState::Queued,
            result: None,
            terminal_evidence: None,
        },
        ready: true,
        connected: true,
        active_turn_id: None,
        approvals: Vec::new(),
        output: String::new(),
        error: None,
    }
}

#[test]
fn internal_runtime_messages_are_never_treated_as_ordinary_parent_child_inputs() {
    for subject in [
        "local_task_result",
        LOCAL_TOOL_LEASE_SUBJECT,
        "native_tool_call",
        "native_tool_result",
    ] {
        assert!(internal_runtime_message_subject(subject));
    }
    assert!(!internal_runtime_message_subject("progress"));
}

async fn persist_running_task(
    sender: &std::sync::mpsc::SyncSender<ModelEvent>,
    task: &mut LocalCliTask,
) {
    let mut queued = task.clone();
    queued.revision = 0;
    queued.state = LocalCliTaskState::Queued;
    queued.result = None;
    queued.terminal_evidence = None;
    checkpoint_task(sender, queued, None)
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    task.revision = 1;
    task.state = LocalCliTaskState::Running;
    checkpoint_task(sender, task.clone(), Some(task.generation))
        .unwrap()
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn repeated_refresh_skips_every_runtime_already_owned_by_this_coordinator() {
    let generation = Uuid::new_v4();
    let owned = HashMap::from([("task-owned".to_owned(), generation)]);

    assert!(runtime_host_is_owned(&owned, "task-owned"));
    assert!(!runtime_host_is_owned(&owned, "task-other"));
}

fn recovery_options(state_dir: &std::path::Path, generation: Uuid) -> SessionOptions {
    SessionOptions {
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
    }
}

#[cfg(unix)]
#[test]
fn restart_reattaches_completed_live_host_without_replaying_input() {
    const CHILD_ENV: &str = "INFINISHELL_COMPLETED_HOST_RECOVERY_TEST";
    if std::env::var_os(CHILD_ENV).is_none() {
        // 子进程隔离宿主可执行文件设置，不修改并行测试共享的进程环境。
        let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
        let output = Command::new(&executable)
            .args([
                "--exact",
                "ai::cli_agent_runtime::coordinator::tests::restart_reattaches_completed_live_host_without_replaying_input",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .env("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE", &executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "宿主恢复子进程失败：{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let writer =
        crate::persistence::start_test_writer(&state_dir.join("completed-live.sqlite")).unwrap();
    block_on(async {
        let generation = Uuid::new_v4();
        let mut task = snapshot().task;
        task.working_directory = state_dir.to_string_lossy().into_owned();
        task.config_json = json!({"runtime_generation": generation}).to_string();
        persist_running_task(&writer.sender, &mut task).await;
        task.revision += 1;
        task.state = LocalCliTaskState::Completed;
        task.result = Some("已完成的结果".into());
        task.terminal_evidence = Some("已确认的终态".into());
        checkpoint_task(&writer.sender, task.clone(), Some(task.generation))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let first_turn = task.clone();
        task = next_task_generation(&task).unwrap();
        checkpoint_task(&writer.sender, task.clone(), Some(first_turn.generation))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        task.revision += 1;
        task.state = LocalCliTaskState::Running;
        checkpoint_task(&writer.sender, task.clone(), Some(task.generation))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        task.revision += 1;
        task.state = LocalCliTaskState::Completed;
        task.result = Some("第二轮已完成的结果".into());
        task.terminal_evidence = Some("第二轮已确认的终态".into());
        checkpoint_task(&writer.sender, task.clone(), Some(task.generation))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let events = vec![
            RuntimeEvent {
                generation,
                native_session_id: task.native_session_id.clone(),
                kind: RuntimeEventKind::SessionReady {
                    verified_cli_version: None,
                    effective_permissions: Value::Null,
                },
            },
            RuntimeEvent {
                generation,
                native_session_id: task.native_session_id.clone(),
                kind: RuntimeEventKind::TurnFinished {
                    turn_id: "completed-turn".into(),
                    outcome: TurnOutcome::Completed,
                    output: task.result.clone().unwrap(),
                },
            },
        ];
        let (server, mut commands, record) =
            super::super::runtime_host::start_acknowledged_host_for_test(
                task.task_id.clone(),
                first_turn.generation,
                recovery_options(&state_dir, generation),
                events,
            )
            .await
            .unwrap();
        let mut batch = recover_runtime_hosts_in_state_dir(
            &writer.sender,
            vec![task.clone()],
            &HashMap::new(),
            &state_dir,
        )
        .await
        .unwrap();
        assert_eq!(batch.records, vec![task.clone()]);
        assert_eq!(batch.hosts.len(), 1);
        assert!(batch.unconfirmed_hosts.is_empty());
        assert_eq!(
            load_task_generations(&writer.sender, task.task_id.clone())
                .unwrap()
                .await
                .unwrap()
                .unwrap(),
            vec![first_turn, task.clone()]
        );
        let recovered = batch.hosts.pop().unwrap();
        assert_eq!(recovered.start_mode, ManagedStartMode::Reattached);
        assert_eq!(recovered.snapshot.task, task);
        assert!(recovered.snapshot.ready && recovered.snapshot.connected);
        assert_eq!(recovered.snapshot.active_turn_id, None);
        assert_eq!(recovered.snapshot.output, task.result.clone().unwrap());
        assert!(recovered.initial_input.is_none());
        assert!(recovered.pending_local_tools.is_empty());
        assert!(
            recovered
                .protocol_state
                .finished_turns
                .contains("completed-turn")
        );
        assert_eq!(commands.try_recv(), Err(mpsc::error::TryRecvError::Empty));
        assert!(
            load_messages(&writer.sender, task.task_id.clone(), task.generation)
                .unwrap()
                .await
                .unwrap()
                .unwrap()
                .is_empty()
        );
        let client = super::super::runtime_host::connect_existing(&state_dir, generation)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            client.inspect().await.unwrap(),
            super::super::runtime_host::RuntimeHostResponse::Inspection {
                owner_epoch: 2,
                acknowledged_sequence: 2,
                ..
            }
        ));
        drop((recovered, client, server));
        // IPC 服务不拥有端点文件；测试结束后移除其私有临时 socket。
        #[cfg(target_os = "macos")]
        let endpoint = std::path::Path::new("/tmp")
            .join(format!("is-cli-host-{}.sock", record.host_instance_id()));
        #[cfg(not(target_os = "macos"))]
        let endpoint = std::env::temp_dir().join(format!(
            "infinishell-cli-host-{}.sock",
            record.host_instance_id()
        ));
        let _ = std::fs::remove_file(endpoint);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn advanced_host_generation_requires_contiguous_unchanged_session_history() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let generation = Uuid::new_v4();
    let mut first = snapshot().task;
    first.working_directory = state_dir.to_string_lossy().into_owned();
    first.config_json = json!({
        "runtime_generation": generation,
        "permission_policy": "read_only",
        "selected_skills": []
    })
    .to_string();
    super::super::runtime_host::write_cancelled_startup_marker_for_test(
        first.task_id.clone(),
        first.generation,
        Harness::Codex,
        recovery_options(&state_dir, generation),
    )
    .unwrap();
    let record = super::super::runtime_host::load_record(&state_dir, generation)
        .unwrap()
        .unwrap();
    let second = next_task_generation(&first).unwrap();
    let current = next_task_generation(&second).unwrap();
    let history = vec![first, second, current.clone()];
    assert!(runtime_host_history_matches(&record, &current, &history));
    assert!(!runtime_host_history_matches(
        &record,
        &current,
        &history[1..]
    ));
    assert!(!runtime_host_history_matches(
        &record,
        &current,
        &[history[0].clone(), current.clone()]
    ));
    let mut duplicate = history.clone();
    duplicate.insert(1, history[0].clone());
    assert!(!runtime_host_history_matches(&record, &current, &duplicate));
    for field in [
        "native_session",
        "runtime",
        "permission",
        "task",
        "parent",
        "cwd",
        "skill",
    ] {
        let mut changed = history.clone();
        match field {
            "native_session" => changed[1].native_session_id = Some("another-session".into()),
            "runtime" => changed[1].config_json = json!({"runtime_generation": Uuid::new_v4()}).to_string(),
            "permission" => changed[1].config_json = json!({"runtime_generation": generation, "permission_policy": "full_access", "selected_skills": []}).to_string(),
            "task" => changed[1].task_id = "another-task".into(),
            "parent" => changed[1].parent_task_id = Some("another-parent".into()),
            "cwd" => changed[1].working_directory = state_dir.join("another-directory").to_string_lossy().into_owned(),
            "skill" => changed[1].config_json = json!({"runtime_generation": generation, "permission_policy": "read_only", "selected_skills": ["unexpected"]}).to_string(),
            _ => unreachable!(),
        }
        assert!(
            !runtime_host_history_matches(&record, &current, &changed),
            "{field}"
        );
    }
    let mut unpersisted = current.clone();
    unpersisted.revision += 1;
    assert!(!runtime_host_history_matches(
        &record,
        &unpersisted,
        &history
    ));
}

#[test]
fn restart_preserves_terminal_records_without_a_confirmed_live_host() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let writer =
        crate::persistence::start_test_writer(&state_dir.join("terminal-history.sqlite")).unwrap();
    block_on(async {
        for state in [
            LocalCliTaskState::Completed,
            LocalCliTaskState::Failed,
            LocalCliTaskState::Cancelled,
        ] {
            for evidence in ["missing", "not_started", "unconfirmed"] {
                let generation = Uuid::new_v4();
                let mut task = snapshot().task;
                task.config_json = json!({"runtime_generation": generation}).to_string();
                persist_running_task(&writer.sender, &mut task).await;
                task.revision += 1;
                task.state = state;
                task.result = Some("保留历史结果".into());
                task.terminal_evidence = Some("保留历史终态".into());
                checkpoint_task(&writer.sender, task.clone(), Some(task.generation))
                    .unwrap()
                    .await
                    .unwrap()
                    .unwrap();
                if evidence == "not_started" {
                    super::super::runtime_host::write_manifest_failure_receipt_for_test(
                        task.task_id.clone(),
                        task.generation,
                        Harness::Codex,
                        recovery_options(&state_dir, generation),
                    )
                    .unwrap();
                } else if evidence == "unconfirmed" {
                    super::super::runtime_host::write_cancelled_startup_marker_for_test(
                        task.task_id.clone(),
                        task.generation,
                        Harness::Codex,
                        recovery_options(&state_dir, generation),
                    )
                    .unwrap();
                }
                let batch = recover_runtime_hosts_in_state_dir(
                    &writer.sender,
                    vec![task.clone()],
                    &HashMap::new(),
                    &state_dir,
                )
                .await
                .unwrap();
                assert!(batch.hosts.is_empty());
                assert_eq!(batch.records, vec![task.clone()]);
                let mut coordinator = LocalCLITaskCoordinator::new(None);
                coordinator.unconfirmed_hosts = batch.unconfirmed_hosts;
                assert_eq!(
                    coordinator.cli_update_busy("codex"),
                    evidence != "not_started"
                );
                assert!(!coordinator.cli_update_busy("claude"));
                assert_eq!(
                    load_tasks(&writer.sender, false)
                        .unwrap()
                        .await
                        .unwrap()
                        .unwrap()
                        .into_iter()
                        .find(|stored| stored.task_id == task.task_id),
                    Some(task)
                );
            }
        }
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn recovery_marks_manifest_failure_without_launch_intent_unconfirmed_and_does_not_retry() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let writer =
        crate::persistence::start_test_writer(&state_dir.join("startup-recovery.sqlite")).unwrap();
    let generation = Uuid::new_v4();
    let mut task = snapshot().task;
    task.native_session_id = None;
    task.config_json = json!({"runtime_generation": generation}).to_string();
    block_on(async {
        checkpoint_task(&writer.sender, task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        super::super::runtime_host::write_manifest_failure_receipt_for_test(
            task.task_id.clone(),
            task.generation,
            Harness::Codex,
            recovery_options(&state_dir, generation),
        )
        .unwrap();

        let batch = recover_runtime_hosts_in_state_dir(
            &writer.sender,
            vec![task],
            &HashMap::new(),
            &state_dir,
        )
        .await
        .unwrap();

        assert!(
            batch.hosts.is_empty(),
            "缺少 launch intent 时不得构造重试连接"
        );
        assert_eq!(batch.records[0].state, LocalCliTaskState::Unconfirmed);
        let config: Value = serde_json::from_str(&batch.records[0].config_json).unwrap();
        assert_eq!(
            config["runtime_host_start_state"],
            "not_started_launch_intent_missing"
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn recovery_rejects_manifest_failure_receipt_for_a_different_database_task() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let writer =
        crate::persistence::start_test_writer(&state_dir.join("startup-identity.sqlite")).unwrap();
    let generation = Uuid::new_v4();
    let mut task = snapshot().task;
    task.native_session_id = None;
    task.config_json = json!({"runtime_generation": generation}).to_string();
    block_on(async {
        checkpoint_task(&writer.sender, task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        super::super::runtime_host::write_manifest_failure_receipt_for_test(
            "different-task".to_owned(),
            task.generation,
            Harness::Codex,
            recovery_options(&state_dir, generation),
        )
        .unwrap();

        let batch = recover_runtime_hosts_in_state_dir(
            &writer.sender,
            vec![task],
            &HashMap::new(),
            &state_dir,
        )
        .await
        .unwrap();

        assert!(batch.hosts.is_empty());
        assert_eq!(batch.records[0].state, LocalCliTaskState::Unconfirmed);
        let config: Value = serde_json::from_str(&batch.records[0].config_json).unwrap();
        assert_eq!(
            config["runtime_host_start_state"],
            "startup_evidence_identity_mismatch"
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn recovery_does_not_retry_a_spawn_cancelled_before_exit_was_confirmed() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let writer =
        crate::persistence::start_test_writer(&state_dir.join("startup-cancelled.sqlite")).unwrap();
    let generation = Uuid::new_v4();
    let mut task = snapshot().task;
    task.native_session_id = None;
    task.config_json = json!({"runtime_generation": generation}).to_string();
    block_on(async {
        checkpoint_task(&writer.sender, task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        super::super::runtime_host::write_cancelled_startup_marker_for_test(
            task.task_id.clone(),
            task.generation,
            Harness::Codex,
            recovery_options(&state_dir, generation),
        )
        .unwrap();

        let batch = recover_runtime_hosts_in_state_dir(
            &writer.sender,
            vec![task],
            &HashMap::new(),
            &state_dir,
        )
        .await
        .unwrap();

        assert!(batch.hosts.is_empty(), "未确认退出不得自动重试");
        assert_eq!(batch.records[0].state, LocalCliTaskState::Unconfirmed);
        let config: Value = serde_json::from_str(&batch.records[0].config_json).unwrap();
        assert_eq!(
            config["runtime_host_start_state"],
            "native_exit_unconfirmed"
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn native_session_mismatch_is_detected_before_runtime_owner_claim() {
    let mut task = snapshot().task;
    task.native_session_id = Some("saved-session".to_owned());
    let host = super::super::runtime_host::RuntimeHostSnapshot {
        native_session_id: Some("different-session".to_owned()),
        ..Default::default()
    };

    assert!(native_session_mismatches(&task, &host));
    assert!(!native_session_mismatches(
        &task,
        &super::super::runtime_host::RuntimeHostSnapshot {
            native_session_id: None,
            ..Default::default()
        }
    ));
}

#[test]
fn committed_duplicate_is_acked_without_reapplying_state_and_next_event_continues() {
    let token = Uuid::new_v4();
    let (mut controller, _commands, _events_sender, _events) = channels(token);
    let (host_acks, mut committed_events) = tokio::sync::mpsc::channel(2);
    controller.host_event_acks = Some(host_acks);
    let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
    let turn_id = "turn-committed-before-crash".to_owned();
    let mut state = snapshot();
    state.task.state = LocalCliTaskState::Completed;
    state.task.result = Some("saved-result".to_owned());
    state.output = "saved-result".to_owned();
    let mut accepted = HashSet::new();
    let mut finished = HashSet::from([turn_id.clone()]);
    let duplicate = RuntimeEvent {
        generation: token,
        native_session_id: state.task.native_session_id.clone(),
        kind: RuntimeEventKind::TurnFinished {
            turn_id: turn_id.clone(),
            outcome: TurnOutcome::Completed,
            output: "saved-result".to_owned(),
        },
    };
    let next = RuntimeEvent {
        generation: token,
        native_session_id: state.task.native_session_id.clone(),
        kind: RuntimeEventKind::Progress {
            turn_id,
            message: "next-event".to_owned(),
        },
    };

    block_on(async {
        assert!(
            !commit_and_ack_runtime_event(
                &sender,
                &mut state,
                &controller,
                &duplicate,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap()
            .committed,
            "已提交的 TurnFinished 不得再次改写任务状态"
        );
        assert_eq!(committed_events.try_recv(), Ok(()));
        assert!(
            commit_and_ack_runtime_event(
                &sender,
                &mut state,
                &controller,
                &next,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap()
            .committed,
            "重复事件 ACK 后必须继续处理后续事件"
        );
        assert_eq!(committed_events.try_recv(), Ok(()));
    });
    assert_eq!(state.task.result.as_deref(), Some("saved-result"));
    assert_eq!(state.output, "saved-result");
}

#[test]
fn duplicate_terminal_event_repairs_parent_result_before_advancing_host_ack() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("result-replay.sqlite"))
            .unwrap();
    block_on(async {
        let mut parent = snapshot().task;
        parent.task_id = "result-parent".into();
        parent.native_session_id = None;
        persist_running_task(&writer.sender, &mut parent).await;
        let mut state = snapshot();
        state.task.task_id = "result-child".into();
        state.task.parent_task_id = Some(parent.task_id.clone());
        state.task.parent_generation = Some(parent.generation);
        persist_running_task(&writer.sender, &mut state.task).await;
        state.active_turn_id = Some("turn-result-replay".into());
        let token = Uuid::new_v4();
        let finished = RuntimeEvent {
            generation: token,
            native_session_id: state.task.native_session_id.clone(),
            kind: RuntimeEventKind::TurnFinished {
                turn_id: "turn-result-replay".into(),
                outcome: TurnOutcome::Completed,
                output: "已提交但尚未生成父结果".into(),
            },
        };
        let mut accepted = HashSet::new();
        let mut finished_turns = HashSet::new();
        assert!(
            commit_runtime_event(
                &writer.sender,
                &mut state,
                &finished,
                &mut accepted,
                &mut finished_turns,
            )
            .await
            .unwrap()
        );
        let before = load_task_messages(&writer.sender, state.task.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert!(
            !before
                .iter()
                .any(|message| message.subject == "local_task_result")
        );

        let (mut controller, _commands, _events_sender, _events) = channels(token);
        let (host_acks, mut committed_events) = tokio::sync::mpsc::channel(2);
        controller.host_event_acks = Some(host_acks);
        let repaired = commit_and_ack_runtime_event(
            &writer.sender,
            &mut state,
            &controller,
            &finished,
            &mut accepted,
            &mut finished_turns,
        )
        .await
        .unwrap();
        assert!(!repaired.committed);
        let message = repaired.result.expect("重复终态必须补齐父结果");
        assert_eq!(committed_events.try_recv(), Ok(()));
        let saved = load_task_messages(&writer.sender, state.task.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved
                .iter()
                .filter(|stored| stored.subject == "local_task_result")
                .count(),
            1
        );
        assert!(saved.iter().any(|stored| stored == &message));

        let repeated = commit_and_ack_runtime_event(
            &writer.sender,
            &mut state,
            &controller,
            &finished,
            &mut accepted,
            &mut finished_turns,
        )
        .await
        .unwrap();
        assert_eq!(repeated.result.unwrap().message_id, message.message_id);
        assert_eq!(committed_events.try_recv(), Ok(()));
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn restart_repairs_a_committed_child_result_without_restarting_the_terminal_task() {
    let directory = tempfile::tempdir().unwrap();
    let state_dir = directory.path().canonicalize().unwrap();
    let writer =
        crate::persistence::start_test_writer(&state_dir.join("terminal-result-repair.sqlite"))
            .unwrap();
    block_on(async {
        let mut parent = snapshot().task;
        parent.task_id = "terminal-result-parent".into();
        parent.native_session_id = None;
        persist_running_task(&writer.sender, &mut parent).await;

        let mut child = snapshot().task;
        child.task_id = "terminal-result-child".into();
        child.parent_task_id = Some(parent.task_id.clone());
        child.parent_generation = Some(parent.generation);
        persist_running_task(&writer.sender, &mut child).await;
        child.revision += 1;
        child.state = LocalCliTaskState::Completed;
        child.result = Some("崩溃前已经提交的真实结果".into());
        child.terminal_evidence = Some("原生终态收据".into());
        checkpoint_task(&writer.sender, child.clone(), Some(child.generation))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert!(
            load_task_messages(&writer.sender, child.task_id.clone())
                .unwrap()
                .await
                .unwrap()
                .unwrap()
                .is_empty(),
            "模拟任务终态已提交而结果消息尚未生成的崩溃窗口"
        );

        for _ in 0..2 {
            let batch = recover_runtime_hosts_in_state_dir(
                &writer.sender,
                vec![parent.clone(), child.clone()],
                &HashMap::new(),
                &state_dir,
            )
            .await
            .unwrap();
            assert!(batch.hosts.is_empty(), "终态子任务不能被重新启动");
            assert_eq!(batch.results.len(), 1);
            assert_eq!(batch.results[0].0, child);
            assert_eq!(batch.results[0].1.state, LocalCliMessageState::Queued);
            assert_eq!(batch.results[0].1.receipt_kind, None);
        }

        let messages = load_task_messages(&writer.sender, child.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.subject == "local_task_result")
                .count(),
            1,
            "确定性结果 ID 必须让重复恢复保持单条记录"
        );
        assert_eq!(messages[0].recipient_task_id, parent.task_id);
        assert_eq!(messages[0].recipient_generation, parent.generation);
        assert_eq!(messages[0].sender_generation, child.generation);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn restart_never_replays_a_child_result_after_delivery_became_unconfirmed() {
    let directory = tempfile::tempdir().unwrap();
    let writer = crate::persistence::start_test_writer(
        &directory.path().join("terminal-result-unconfirmed.sqlite"),
    )
    .unwrap();
    block_on(async {
        let mut parent = snapshot().task;
        parent.task_id = "unconfirmed-result-parent".into();
        parent.native_session_id = None;
        persist_running_task(&writer.sender, &mut parent).await;

        let mut child = snapshot().task;
        child.task_id = "unconfirmed-result-child".into();
        child.parent_task_id = Some(parent.task_id.clone());
        child.parent_generation = Some(parent.generation);
        persist_running_task(&writer.sender, &mut child).await;
        child.revision += 1;
        child.state = LocalCliTaskState::Completed;
        child.result = Some("只允许投递一次的结果".into());
        child.terminal_evidence = Some("原生完成收据".into());
        checkpoint_task(&writer.sender, child.clone(), Some(child.generation))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let message = enqueue_task_result(&writer.sender, child.task_id.clone(), child.generation)
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        acknowledge_message(
            &writer.sender,
            message.message_id.clone(),
            parent.task_id.clone(),
            parent.generation,
            LocalCliMessageState::Sent,
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();

        assert!(
            recover_terminal_task_result(&writer.sender, &child)
                .await
                .unwrap()
                .is_none(),
            "Sent 表示交付不确定，恢复不得把它重新排队"
        );
        let messages = load_task_messages(&writer.sender, child.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].state, LocalCliMessageState::Sent);
        assert_eq!(messages[0].receipt_kind, None);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn local_tool_replay_repairs_missing_lease_before_acknowledging_event() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tool-lease-repair.sqlite"))
            .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let mut state = snapshot();
        state.task.task_id = "tool-lease-repair".into();
        state.task.state = LocalCliTaskState::Running;
        state.active_turn_id = Some("turn-tool-lease-repair".into());
        persist_running_task(&writer.sender, &mut state.task).await;
        let request = NativeLocalToolRequest {
            reply_target: super::super::local_tools::LocalToolReplyTarget::Codex {
                request_id: json!("request-tool-lease-repair"),
            },
            call_id: "call-tool-lease-repair".into(),
            turn_id: "turn-tool-lease-repair".into(),
            tool: "inspect_local_tasks".into(),
            arguments: json!({}),
        };
        let event = RuntimeEvent {
            generation: token,
            native_session_id: state.task.native_session_id.clone(),
            kind: RuntimeEventKind::LocalToolRequested { request },
        };
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();

        assert!(
            commit_runtime_event(
                &writer.sender,
                &mut state,
                &event,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap(),
            "LocalToolRequested 当前不在任务快照中记录去重标记"
        );
        let stored = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored
                .iter()
                .filter(|message| message.subject == LOCAL_TOOL_LEASE_SUBJECT)
                .count(),
            0,
            "模拟业务事件已提交、租约尚未写入的崩溃窗口"
        );

        let (mut controller, _commands, _events_sender, _events) = channels(token);
        let (host_acks, mut committed_events) = tokio::sync::mpsc::channel(2);
        controller.host_event_acks = Some(host_acks);
        let replayed = commit_and_ack_runtime_event(
            &writer.sender,
            &mut state,
            &controller,
            &event,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert!(
            replayed.committed,
            "LocalToolRequested 重放当前仍返回已提交，租约修复不能依赖该布尔值"
        );
        assert_eq!(committed_events.try_recv(), Ok(()));
        let stored = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored
                .iter()
                .filter(|message| message.subject == LOCAL_TOOL_LEASE_SUBJECT)
                .count(),
            1
        );

        let repeated = commit_and_ack_runtime_event(
            &writer.sender,
            &mut state,
            &controller,
            &event,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert!(repeated.committed);
        assert_eq!(committed_events.try_recv(), Ok(()));
        let stored = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored
                .iter()
                .filter(|message| message.subject == LOCAL_TOOL_LEASE_SUBJECT)
                .count(),
            1
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn live_reattach_catch_up_applies_cancellation_before_selecting_recovered_tools() {
    let directory = tempfile::tempdir().unwrap();
    let writer = crate::persistence::start_test_writer(
        &directory
            .path()
            .join("live-tool-cancellation-catch-up.sqlite"),
    )
    .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let mut task = snapshot().task;
        task.task_id = "live-tool-cancellation-catch-up".into();
        task.state = LocalCliTaskState::Running;
        let turn_id = "turn-live-tool-cancellation".to_owned();
        let request = NativeLocalToolRequest {
            reply_target: super::super::local_tools::LocalToolReplyTarget::Codex {
                request_id: json!("request-live-tool-cancellation"),
            },
            call_id: "call-live-tool-cancellation".into(),
            turn_id: turn_id.clone(),
            tool: "inspect_local_tasks".into(),
            arguments: json!({}),
        };
        let requested = RuntimeEvent {
            generation: token,
            native_session_id: task.native_session_id.clone(),
            kind: RuntimeEventKind::LocalToolRequested {
                request: request.clone(),
            },
        };
        let cancelled = RuntimeEvent {
            generation: token,
            native_session_id: task.native_session_id.clone(),
            kind: RuntimeEventKind::LocalToolCancelled {
                turn_id: turn_id.clone(),
                call_id: request.call_id.clone(),
            },
        };
        let mut host_snapshot = super::super::runtime_host::RuntimeHostSnapshot {
            ready: true,
            connected: true,
            native_session_id: task.native_session_id.clone(),
            active_turn_id: Some(turn_id),
            ..Default::default()
        };
        host_snapshot.apply_recovered_event(&requested);
        persist_running_task(&writer.sender, &mut task).await;
        ensure_local_tool_lease(&writer.sender, &task, token, &request)
            .await
            .unwrap();
        let mut protocol_state = protocol_state_from_host(&host_snapshot);
        let mut state = snapshot_from_host(task, host_snapshot.clone());

        let committed = commit_catch_up_runtime_event(
            &writer.sender,
            &mut state,
            &mut host_snapshot,
            &cancelled,
            &mut protocol_state,
        )
        .await
        .unwrap();

        assert!(committed.committed);
        assert!(host_snapshot.local_tools.is_empty());
        let recovered_tools: Vec<_> = host_snapshot.local_tools.values().cloned().collect();
        assert!(
            recovered_tools.is_empty(),
            "未 ACK 的取消必须在选择恢复工具前生效"
        );
        let stored = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert!(
            stored
                .iter()
                .all(|message| message.subject != "native_tool_call"),
            "catch-up 屏障内不得启动本地工具副作用"
        );
        assert_eq!(
            stored
                .iter()
                .filter(|message| message.subject == LOCAL_TOOL_LEASE_SUBJECT)
                .count(),
            1,
            "已 ACK 的工具请求租约必须保留，但取消后不得执行"
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn local_tool_lease_is_idempotent_and_started_without_result_recovers_as_uncertain() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tool-replay.sqlite"))
            .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let mut state = snapshot();
        state.task.task_id = "tool-recovery".into();
        state.task.state = LocalCliTaskState::Running;
        state.active_turn_id = Some("turn-tool-recovery".into());
        persist_running_task(&writer.sender, &mut state.task).await;
        let request = NativeLocalToolRequest {
            reply_target: super::super::local_tools::LocalToolReplyTarget::Codex {
                request_id: json!("request-tool-recovery"),
            },
            call_id: "call-tool-recovery".into(),
            turn_id: "turn-tool-recovery".into(),
            tool: "inspect_local_tasks".into(),
            arguments: json!({}),
        };
        let event = RuntimeEvent {
            generation: token,
            native_session_id: state.task.native_session_id.clone(),
            kind: RuntimeEventKind::LocalToolRequested {
                request: request.clone(),
            },
        };
        let (mut controller, _commands, _events_sender, _events) = channels(token);
        let (host_acks, mut committed_events) = tokio::sync::mpsc::channel(2);
        controller.host_event_acks = Some(host_acks);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        for _ in 0..2 {
            assert!(
                commit_and_ack_runtime_event(
                    &writer.sender,
                    &mut state,
                    &controller,
                    &event,
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap()
                .committed
            );
            assert_eq!(committed_events.try_recv(), Ok(()));
        }
        let stored = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored
                .iter()
                .filter(|message| message.subject == LOCAL_TOOL_LEASE_SUBJECT)
                .count(),
            1
        );
        assert!(
            recovered_local_tool_completion(&writer.sender, &state.task, token, &request)
                .await
                .unwrap()
                .is_none(),
            "只有租约而没有调用记录时可安全开始"
        );

        let call_id = Uuid::new_v4();
        let call = LocalCliMessage {
            version: 1,
            message_id: call_id.to_string(),
            sender_task_id: state.task.task_id.clone(),
            recipient_task_id: state.task.task_id.clone(),
            sender_generation: 1,
            recipient_generation: 1,
            subject: "native_tool_call".into(),
            body: serde_json::to_string(&request).unwrap(),
            state: LocalCliMessageState::Queued,
            receipt_kind: None,
        };
        enqueue_message(&writer.sender, call.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        acknowledge_message(
            &writer.sender,
            call.message_id,
            state.task.task_id.clone(),
            1,
            LocalCliMessageState::Sent,
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
        let uncertain =
            recovered_local_tool_completion(&writer.sender, &state.task, token, &request)
                .await
                .unwrap()
                .expect("已有调用但缺少结果必须收敛为不确定失败");
        assert!(uncertain.receipt_id.is_none());
        assert_eq!(
            uncertain.result.as_ref().unwrap_err(),
            "runtime_host.local_tool_execution_unconfirmed"
        );
        tools::prepare_tool_reply(&writer.sender, &state.task, &uncertain)
            .await
            .unwrap();
        let recovered =
            recovered_local_tool_completion(&writer.sender, &state.task, token, &request)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(recovered.result, uncertain.result);

        let completed_request = NativeLocalToolRequest {
            call_id: "call-tool-completed".into(),
            ..request
        };
        ensure_local_tool_lease(&writer.sender, &state.task, token, &completed_request)
            .await
            .unwrap();
        let completed_call_id = Uuid::new_v4();
        let completed_call = LocalCliMessage {
            version: 1,
            message_id: completed_call_id.to_string(),
            sender_task_id: state.task.task_id.clone(),
            recipient_task_id: state.task.task_id.clone(),
            sender_generation: 1,
            recipient_generation: 1,
            subject: "native_tool_call".into(),
            body: serde_json::to_string(&completed_request).unwrap(),
            state: LocalCliMessageState::Queued,
            receipt_kind: None,
        };
        enqueue_message(&writer.sender, completed_call.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        acknowledge_message(
            &writer.sender,
            completed_call.message_id,
            state.task.task_id.clone(),
            1,
            LocalCliMessageState::Sent,
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
        let completed_result_id = local_tool_result_id(completed_call_id);
        let completed_result: Result<Value, String> = Ok(json!({"tasks": []}));
        enqueue_message(
            &writer.sender,
            LocalCliMessage {
                version: 1,
                message_id: completed_result_id.to_string(),
                sender_task_id: state.task.task_id.clone(),
                recipient_task_id: state.task.task_id.clone(),
                sender_generation: 1,
                recipient_generation: 1,
                subject: "native_tool_result".into(),
                body: serde_json::to_string(&completed_result).unwrap(),
                state: LocalCliMessageState::Queued,
                receipt_kind: None,
            },
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
        let completed =
            recovered_local_tool_completion(&writer.sender, &state.task, token, &completed_request)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(completed.receipt_id, Some(completed_result_id));
        assert_eq!(completed.result, completed_result);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn cross_generation_host_event_is_rejected_instead_of_waiting_for_an_ack() {
    let state = snapshot();
    let event = RuntimeEvent {
        generation: Uuid::new_v4(),
        native_session_id: state.task.native_session_id,
        kind: RuntimeEventKind::Progress {
            turn_id: "wrong-generation".to_owned(),
            message: "must-fail-closed".to_owned(),
        },
    };

    assert!(verify_runtime_event_generation(&event, Uuid::new_v4()).is_err());
}

fn event(snapshot: &ManagedTaskSnapshot, kind: RuntimeEventKind) -> RuntimeEvent {
    RuntimeEvent {
        generation: Uuid::new_v4(),
        native_session_id: snapshot.task.native_session_id.clone(),
        kind,
    }
}

#[test]
fn disconnect_cannot_imply_success_and_cannot_overwrite_a_committed_result() {
    let mut state = snapshot();
    let disconnected = event(
        &state,
        RuntimeEventKind::Disconnected {
            reason: "EOF".into(),
        },
    );
    apply_runtime_event(&mut state, &disconnected).unwrap();
    assert_eq!(state.task.state, LocalCliTaskState::Disconnected);
    state.task.state = LocalCliTaskState::Completed;
    state.task.result = Some("真实结果".into());
    apply_runtime_event(&mut state, &disconnected).unwrap();
    assert_eq!(state.task.state, LocalCliTaskState::Completed);
    assert_eq!(state.task.result.as_deref(), Some("真实结果"));
}

#[test]
fn old_turn_cannot_finish_the_current_task() {
    let mut state = snapshot();
    state.active_turn_id = Some("current".into());
    state.task.state = LocalCliTaskState::Running;
    let finished = event(
        &state,
        RuntimeEventKind::TurnFinished {
            turn_id: "old".into(),
            outcome: TurnOutcome::Completed,
            output: "旧结果".into(),
        },
    );
    apply_runtime_event(&mut state, &finished).unwrap();
    assert_eq!(state.task.state, LocalCliTaskState::Running);
    assert_eq!(state.task.result, None);
}

#[test]
fn duplicate_turn_start_keeps_current_output_and_state() {
    let mut state = snapshot();
    state.active_turn_id = Some("current".into());
    state.task.state = LocalCliTaskState::WaitingForUser;
    state.output = "已经接收的中文输出".into();
    let duplicate = event(
        &state,
        RuntimeEventKind::TurnStarted {
            turn_id: "current".into(),
        },
    );
    apply_runtime_event(&mut state, &duplicate).unwrap();
    assert_eq!(state.output, "已经接收的中文输出");
    assert_eq!(state.task.state, LocalCliTaskState::WaitingForUser);
}

#[test]
fn approval_cancellation_is_not_a_user_denial_or_task_cancellation() {
    let mut state = snapshot();
    state.active_turn_id = Some("turn".into());
    let requested = event(
        &state,
        RuntimeEventKind::ApprovalRequested {
            approval_id: "permission".into(),
            turn_id: "turn".into(),
            method: "exec".into(),
            details: json!({}),
        },
    );
    apply_runtime_event(&mut state, &requested).unwrap();
    apply_runtime_event(&mut state, &requested).unwrap();
    assert_eq!(state.approvals.len(), 1);
    assert_eq!(state.task.state, LocalCliTaskState::WaitingForUser);
    let cancelled = event(
        &state,
        RuntimeEventKind::ApprovalCancelled {
            approval_id: "permission".into(),
        },
    );
    apply_runtime_event(&mut state, &cancelled).unwrap();
    assert!(state.approvals.is_empty());
    assert_eq!(state.task.state, LocalCliTaskState::Running);
}

#[test]
fn a_success_without_native_session_identity_is_rejected() {
    let mut state = snapshot();
    state.task.native_session_id = None;
    state.active_turn_id = Some("turn".into());
    let finished = event(
        &state,
        RuntimeEventKind::TurnFinished {
            turn_id: "turn".into(),
            outcome: TurnOutcome::Completed,
            output: "not verified".into(),
        },
    );
    assert!(apply_runtime_event(&mut state, &finished).is_err());
}

#[test]
fn followup_keeps_prior_result_and_duplicate_message_does_not_dispatch_twice() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    let sender = writer.sender.clone();
    block_on(async {
        let mut state = snapshot();
        checkpoint_task(&sender, state.task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let previous = state.task.clone();
        state.task.state = LocalCliTaskState::Completed;
        state.task.result = Some("第一轮结果".into());
        state.task.terminal_evidence = Some("native turn/completed".into());
        commit_transition(&sender, &mut state.task, &previous)
            .await
            .unwrap();
        let token = Uuid::new_v4();
        let (controller, mut commands, _sender, _events) = channels(token);
        let message_id = Uuid::new_v4();
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text("第二轮\n保留中文 🦀".into())],
        };
        send_user_request(
            &sender,
            &mut state,
            &controller,
            token,
            message_id,
            action.clone(),
            false,
        )
        .await
        .unwrap();
        send_user_request(
            &sender,
            &mut state,
            &controller,
            token,
            message_id,
            action.clone(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(commands.try_recv().unwrap().action, action);
        assert!(commands.try_recv().is_err());
        let generations = load_task_generations(&sender, state.task.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(generations.len(), 2);
        assert_eq!(generations[0].result.as_deref(), Some("第一轮结果"));
        assert_eq!(generations[1].state, LocalCliTaskState::Queued);
        let messages = load_messages(&sender, state.task.task_id.clone(), 2)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(messages[0].state, LocalCliMessageState::Sent);
        let accepted = RuntimeEventKind::MessageAccepted {
            message_id,
            turn_id: Some("turn".into()),
        };
        acknowledge_runtime_message(&sender, &state.task.task_id, 2, &accepted)
            .await
            .unwrap();
        let messages = load_messages(&sender, state.task.task_id.clone(), 2)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(messages[0].state, LocalCliMessageState::Acknowledged);
    });
    sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn closed_controller_preserves_input_with_failed_delivery() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("tasks.sqlite")).unwrap();
    let sender = writer.sender.clone();
    block_on(async {
        let mut state = snapshot();
        checkpoint_task(&sender, state.task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let token = Uuid::new_v4();
        let (controller, commands, _sender, _events) = channels(token);
        drop(commands);
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text("保留此输入".into())],
        };
        assert!(
            send_user_request(
                &sender,
                &mut state,
                &controller,
                token,
                Uuid::new_v4(),
                action,
                false
            )
            .await
            .is_err()
        );
        let messages = load_messages(&sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(messages[0].state, LocalCliMessageState::Failed);
        assert!(messages[0].body.contains("保留此输入"));
    });
    sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn permission_failure_preserves_sqlite_evidence_and_leaves_initial_input_undispatched() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("permissions.sqlite"))
            .unwrap();
    block_on(async {
        let mut state = snapshot();
        state.task.state = LocalCliTaskState::Queued;
        state.task.revision = 0;
        state.task.native_session_id = None;
        checkpoint_task(&writer.sender, state.task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let message_id = Uuid::new_v4();
        let input = input_message(
            &state.task,
            message_id,
            &RuntimeAction::Submit {
                input: vec![InputContent::Text("中文输入必须保留且不派发".into())],
            },
        )
        .unwrap();
        enqueue_message(&writer.sender, input.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let error = super::super::permissions::rejected(
            None,
            &json!({"sandbox":{"type":"workspaceWrite"}}),
            "effective_permissions_changed",
            true,
        );
        persist_permission_failure(
            &writer.sender,
            &mut state.task,
            error.to_string(),
            error.permission_ceiling_evidence().unwrap().clone(),
        )
        .await
        .unwrap();
        let saved = load_tasks(&writer.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].state, LocalCliTaskState::Failed);
        assert!(!saved[0].result.as_deref().unwrap().is_empty());
        let evidence: serde_json::Value =
            serde_json::from_str(saved[0].terminal_evidence.as_ref().unwrap()).unwrap();
        assert_eq!(evidence["source"], "parent_permission_ceiling");
        assert_eq!(evidence["task_input_sent"], false);
        let messages = load_messages(
            &writer.sender,
            state.task.task_id.clone(),
            state.task.generation,
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, input.body);
        assert_eq!(messages[0].state, LocalCliMessageState::Cancelled);
        assert!(messages[0].receipt_kind.is_none());
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn sqlite_parent_ceiling_uses_the_creation_generation_after_parent_resume() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("parent-permissions.sqlite"))
            .unwrap();
    block_on(async {
        let capture = include_str!(
            "../../../../specs/cli-agent-parity/fixtures/codex-0.147.0-local-tools-registration.ndjson"
        );
        let result = capture
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|record| record.pointer("/message/result/sandbox").is_some())
            .unwrap()["message"]["result"]
            .clone();
        let mut parent = snapshot().task;
        parent.config_json = json!({"cli_version":"0.147.0","effective_permissions":{
            "approvalPolicy":result["approvalPolicy"],"sandbox":result["sandbox"],"approvalsReviewer":result["approvalsReviewer"],
        }}).to_string();
        checkpoint_task(&writer.sender, parent.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        parent.state = LocalCliTaskState::Running;
        parent.revision = 1;
        checkpoint_task(&writer.sender, parent.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let ceiling = super::super::permissions::ceiling_from_parent(&parent, "codex").unwrap();
        let mut child = snapshot().task;
        child.parent_task_id = Some(parent.task_id.clone());
        child.parent_generation = Some(1);
        child.config_json = json!({"permission_ceiling":ceiling}).to_string();
        checkpoint_task(&writer.sender, child.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let mut options = SessionOptions {
            executable: std::env::current_exe().unwrap(),
            cwd: parent.working_directory.clone().into(),
            state_dir: directory.path().to_owned(),
            target: SessionTarget::Resume {
                native_session_id: child.native_session_id.clone().unwrap(),
            },
            generation: Uuid::new_v4(),
            permission_policy: super::super::PermissionPolicy::Inherit,
            permission_ceiling: serde_json::from_str::<serde_json::Value>(&child.config_json)
                .unwrap()
                .get("permission_ceiling")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .unwrap(),
            claude_profile: None,
            grok_profile: None,
            model: None,
            local_tools: None,
            selected_skills: Vec::new(),
        };
        verify_saved_parent_ceiling(&writer.sender, &child, &options)
            .await
            .unwrap();
        parent.state = LocalCliTaskState::Disconnected;
        parent.revision = 2;
        checkpoint_task(&writer.sender, parent.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let mut current_parent = next_task_generation(&parent).unwrap();
        current_parent.config_json = json!({"cli_version":"0.147.0","effective_permissions":{"sandbox":{"type":"dangerFullAccess"}}}).to_string();
        checkpoint_task(&writer.sender, current_parent, Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        verify_saved_parent_ceiling(&writer.sender, &child, &options)
            .await
            .unwrap();
        let mut rebound = child.clone();
        rebound.parent_generation = Some(2);
        assert!(
            verify_saved_parent_ceiling(&writer.sender, &rebound, &options)
                .await
                .is_err()
        );
        options.permission_ceiling = None;
        assert!(
            verify_saved_parent_ceiling(&writer.sender, &child, &options)
                .await
                .is_err()
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

async fn persisted_running_claude(sender: &SyncSender<ModelEvent>) -> ManagedTaskSnapshot {
    let mut state = snapshot();
    state.task.harness = "claude".into();
    checkpoint_task(sender, state.task.clone(), None)
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    let previous = state.task.clone();
    state.task.state = LocalCliTaskState::Running;
    state.active_turn_id = Some(Uuid::from_u128(10).to_string());
    state.output = "当前回合输出".into();
    commit_transition(sender, &mut state.task, &previous)
        .await
        .unwrap();
    state
}

#[test]
fn result_blocked_by_a_native_pending_input_stays_queued_and_is_written_once_after_join() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("result.sqlite")).unwrap();
    block_on(async {
        let mut state = persisted_running_claude(&writer.sender).await;
        let mut child = snapshot().task;
        child.parent_task_id = Some(state.task.task_id.clone());
        child.parent_generation = Some(1);
        checkpoint_task(&writer.sender, child.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        child.revision = 1;
        child.state = LocalCliTaskState::Completed;
        child.result = Some("子任务结果".into());
        child.terminal_evidence = Some("turn/completed:child-1".into());
        checkpoint_task(&writer.sender, child.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let result = enqueue_task_result(&writer.sender, child.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let token = Uuid::new_v4();
        let (controller, mut commands, _sender, _events) = channels(token);
        let user_id = Uuid::from_u128(11);
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            user_id,
            RuntimeAction::Submit {
                input: vec![InputContent::Text("先入队的用户输入".into())],
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(commands.try_recv().unwrap().message_id, user_id);
        let result_id = Uuid::parse_str(&result.message_id).unwrap();
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text("自动结果".into())],
        };
        send_mailbox_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            result_id,
            action.clone(),
            Some(result.clone()),
        )
        .await
        .unwrap();
        assert!(commands.try_recv().is_err());
        let stored = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored
                .iter()
                .find(|message| message.message_id == result.message_id)
                .unwrap()
                .state,
            LocalCliMessageState::Queued
        );
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        for id in [user_id, result_id] {
            if id == result_id {
                send_mailbox_request(
                    &writer.sender,
                    &mut state,
                    &controller,
                    token,
                    result_id,
                    action.clone(),
                    Some(result.clone()),
                )
                .await
                .unwrap();
                assert_eq!(commands.try_recv().unwrap().message_id, result_id);
                let pending = claude_pending_input(&state.task).unwrap();
                send_mailbox_request(
                    &writer.sender,
                    &mut state,
                    &controller,
                    token,
                    result_id,
                    action.clone(),
                    Some(result.clone()),
                )
                .await
                .unwrap();
                assert!(commands.try_recv().is_err());
                assert_eq!(claude_pending_input(&state.task).unwrap(), pending);
            }
            for kind in [
                RuntimeEventKind::MessageAccepted {
                    message_id: id,
                    turn_id: Some(id.to_string()),
                },
                RuntimeEventKind::InputJoined {
                    message_id: id,
                    turn_id: Uuid::from_u128(10).to_string(),
                },
            ] {
                let native = event(&state, kind);
                commit_runtime_event(
                    &writer.sender,
                    &mut state,
                    &native,
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
            }
        }
        send_mailbox_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            result_id,
            action,
            Some(result.clone()),
        )
        .await
        .unwrap();
        assert!(commands.try_recv().is_err(), "已领取的结果不能再次写入协议");
        assert!(!claude_input_pending(&state.task));
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn offline_recovery_replays_a_committed_event_before_advancing_its_durable_watermark() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("runtime-host-recovery.sqlite");
    let token = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    let identity = RuntimeHostRecoveryIdentity {
        runtime_generation: token,
        host_instance_id: Uuid::new_v4(),
        manifest_sha256: "manifest-a".to_owned(),
    };
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    let (task_id, acknowledged) = block_on(async {
        let mut state = snapshot();
        state.task.harness = "claude".into();
        state.task.config_json = json!({"runtime_generation":token}).to_string();
        checkpoint_task(&writer.sender, state.task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let (controller, mut commands, _sender, _events) = channels(token);
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            message_id,
            RuntimeAction::Submit {
                input: vec![InputContent::Text("跨崩溃恢复".into())],
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(commands.try_recv().unwrap().message_id, message_id);
        let acknowledged = RuntimeEvent {
            generation: token,
            native_session_id: state.task.native_session_id.clone(),
            kind: RuntimeEventKind::MessageAccepted {
                message_id,
                turn_id: Some(message_id.to_string()),
            },
        };
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &acknowledged,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        // 模拟 SQLite 事件/消息已提交，而 recovered-through 尚未推进即崩溃。
        assert!(recovery_checkpoint(&state.task).unwrap().is_none());
        (state.task.task_id, acknowledged)
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();

    let restarted = crate::persistence::start_test_writer(&database).unwrap();
    block_on(async {
        let task = load_tasks(&restarted.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .into_iter()
            .find(|task| task.task_id == task_id)
            .unwrap();
        let messages = load_messages(&restarted.sender, task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(messages[0].state, LocalCliMessageState::Acknowledged);
        let mut state = ManagedTaskSnapshot {
            task,
            ready: true,
            connected: true,
            active_turn_id: None,
            approvals: Vec::new(),
            output: String::new(),
            error: None,
        };
        let mut protocol = RecoveryProtocolState::default();
        // 重放已提交的 ACK 必须保持幂等，同时重建下一事件所需的 accepted turn。
        commit_runtime_event(
            &restarted.sender,
            &mut state,
            &acknowledged,
            &mut protocol.accepted_turns,
            &mut protocol.finished_turns,
        )
        .await
        .unwrap();
        advance_recovery_protocol_state(&mut protocol, &acknowledged);
        persist_recovery_checkpoint(&restarted.sender, &mut state.task, &identity, 1)
            .await
            .unwrap();
        let started = RuntimeEvent {
            generation: token,
            native_session_id: state.task.native_session_id.clone(),
            kind: RuntimeEventKind::TurnStarted {
                turn_id: message_id.to_string(),
            },
        };
        commit_runtime_event(
            &restarted.sender,
            &mut state,
            &started,
            &mut protocol.accepted_turns,
            &mut protocol.finished_turns,
        )
        .await
        .unwrap();
        persist_recovery_checkpoint(&restarted.sender, &mut state.task, &identity, 2)
            .await
            .unwrap();
        assert_eq!(state.task.state, LocalCliTaskState::Running);
        let expected_turn = message_id.to_string();
        assert_eq!(
            state.active_turn_id.as_deref(),
            Some(expected_turn.as_str())
        );
        assert_eq!(
            recovery_checkpoint(&state.task)
                .unwrap()
                .unwrap()
                .recovered_through,
            2
        );
    });
    restarted.sender.send(ModelEvent::Terminate).unwrap();
    restarted.handle.join().unwrap();
}

#[test]
fn claude_queued_input_commits_receipt_before_advancing_the_execution_generation() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("queue.sqlite")).unwrap();
    block_on(async {
        let mut state = persisted_running_claude(&writer.sender).await;
        let token = Uuid::new_v4();
        let (controller, mut commands, _sender, _events) = channels(token);
        let id = Uuid::from_u128(11);
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text("下一轮中文\nNext turn".into())],
        };
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            id,
            action.clone(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(commands.try_recv().unwrap().action, action);
        assert_eq!(state.task.generation, 1);
        assert_eq!(
            state.active_turn_id.as_deref(),
            Some(Uuid::from_u128(10).to_string().as_str())
        );
        assert_eq!(state.output, "当前回合输出");
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].state, LocalCliMessageState::Sent);
        assert!(claude_input_pending(&state.task));

        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            id,
            action.clone(),
            false,
        )
        .await
        .unwrap();
        assert!(commands.try_recv().is_err());
        assert!(
            send_user_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                Uuid::from_u128(12),
                action,
                false
            )
            .await
            .is_err()
        );
        assert!(commands.try_recv().is_err());
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        let ack = event(
            &state,
            RuntimeEventKind::MessageAccepted {
                message_id: id,
                turn_id: Some(id.to_string()),
            },
        );
        assert!(
            commit_runtime_event(
                &writer.sender,
                &mut state,
                &ack,
                &mut accepted,
                &mut finished
            )
            .await
            .unwrap()
        );
        assert!(claude_input_pending(&state.task));
        assert_eq!(state.task.generation, 1);
        assert_eq!(state.output, "当前回合输出");
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].state, LocalCliMessageState::Acknowledged);
        assert_eq!(
            saved[0].receipt_kind,
            Some(LocalCliReceiptKind::NativeProtocol)
        );

        let first_finished = event(
            &state,
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(10).to_string(),
                outcome: TurnOutcome::Completed,
                output: "第一轮最终结果".into(),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &first_finished,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.generation, 1);
        assert_eq!(state.task.result.as_deref(), Some("第一轮最终结果"));
        assert!(claude_input_pending(&state.task));
        let old_start = event(
            &state,
            RuntimeEventKind::TurnStarted {
                turn_id: Uuid::from_u128(10).to_string(),
            },
        );
        assert!(
            !commit_runtime_event(
                &writer.sender,
                &mut state,
                &old_start,
                &mut accepted,
                &mut finished
            )
            .await
            .unwrap()
        );
        let next_start = event(
            &state,
            RuntimeEventKind::TurnStarted {
                turn_id: id.to_string(),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &next_start,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.generation, 2);
        assert_eq!(state.task.state, LocalCliTaskState::Running);
        assert!(!claude_input_pending(&state.task));
        assert_eq!(state.output, "");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&state.task.config_json).unwrap()["claude_current_input"],
            json!({"turn_id":id,"submission_generation":1})
        );
        assert!(
            !commit_runtime_event(
                &writer.sender,
                &mut state,
                &next_start,
                &mut accepted,
                &mut finished
            )
            .await
            .unwrap()
        );
        let next_finished = event(
            &state,
            RuntimeEventKind::TurnFinished {
                turn_id: id.to_string(),
                outcome: TurnOutcome::Completed,
                output: "下一轮最终结果".into(),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &next_finished,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let generations = load_task_generations(&writer.sender, state.task.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(generations.len(), 2);
        assert_eq!(generations[0].result.as_deref(), Some("第一轮最终结果"));
        assert_eq!(generations[1].result.as_deref(), Some("下一轮最终结果"));
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].state, LocalCliMessageState::Acknowledged);
        assert_eq!(saved[0].recipient_generation, 1);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn native_joined_input_keeps_the_execution_generation_and_requires_its_result() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("joined.sqlite")).unwrap();
    block_on(async {
        let mut state = persisted_running_claude(&writer.sender).await;
        let token = Uuid::new_v4();
        let (controller, mut commands, _sender, _events) = channels(token);
        let id = Uuid::from_u128(11);
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            id,
            RuntimeAction::Submit {
                input: vec![InputContent::Text("追加输入".into())],
            },
            false,
        )
        .await
        .unwrap();
        commands.try_recv().unwrap();
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        let joined = event(
            &state,
            RuntimeEventKind::InputJoined {
                message_id: id,
                turn_id: Uuid::from_u128(10).to_string(),
            },
        );
        assert!(
            commit_runtime_event(
                &writer.sender,
                &mut state,
                &joined,
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        let ack = event(
            &state,
            RuntimeEventKind::MessageAccepted {
                message_id: id,
                turn_id: Some(id.to_string()),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &ack,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &joined,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert!(!claude_input_pending(&state.task));
        assert_eq!(state.task.generation, 1);
        assert_eq!(state.output, "当前回合输出");
        assert_eq!(state.active_turn_id, Some(Uuid::from_u128(10).to_string()));
        // 模拟 InputJoined 的业务提交完成、recovered-through 尚未推进即崩溃；
        // 重启从前一 ACK 快照重建 accepted 后，必须只收敛水位而不再次要求 pending。
        accepted.insert(id.to_string());
        assert!(
            !commit_runtime_event(
                &writer.sender,
                &mut state,
                &joined,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap()
        );
        assert!(!accepted.contains(&id.to_string()));
        let execution_result = event(
            &state,
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(10).to_string(),
                outcome: TurnOutcome::Completed,
                output: "共同结果".into(),
            },
        );
        assert!(
            commit_runtime_event(
                &writer.sender,
                &mut state,
                &execution_result,
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        assert_eq!(state.task.state, LocalCliTaskState::Running);
        let input_result = event(
            &state,
            RuntimeEventKind::TurnFinished {
                turn_id: id.to_string(),
                outcome: TurnOutcome::Completed,
                output: "共同结果".into(),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &input_result,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.state, LocalCliTaskState::Running);
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &execution_result,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.state, LocalCliTaskState::Completed);
        assert_eq!(state.task.generation, 1);
        let saved = load_task_generations(&writer.sender, state.task.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.len(), 1);
        let config: serde_json::Value = serde_json::from_str(&saved[0].config_json).unwrap();
        assert_eq!(
            config["claude_joined_inputs"],
            json!([{"message_id":id,"turn_id":Uuid::from_u128(10),"submission_generation":1,"outcome":"Completed"}])
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn cancelling_current_claude_turn_does_not_claim_queued_input_was_cancelled() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("queue.sqlite")).unwrap();
    block_on(async {
        let mut state = persisted_running_claude(&writer.sender).await;
        let token = Uuid::new_v4();
        let (controller, mut commands, _sender, _events) = channels(token);
        let id = Uuid::from_u128(11);
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text("已排队输入".into())],
        };
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            id,
            action,
            false,
        )
        .await
        .unwrap();
        assert_eq!(commands.try_recv().unwrap().message_id, id);
        let interrupt = RuntimeAction::Interrupt {
            turn_id: Uuid::from_u128(10).to_string(),
        };
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            Uuid::from_u128(12),
            interrupt.clone(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(commands.try_recv().unwrap().action, interrupt);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        let cancelled = event(
            &state,
            RuntimeEventKind::TurnFinished {
                turn_id: Uuid::from_u128(10).to_string(),
                outcome: TurnOutcome::Cancelled,
                output: "".into(),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &cancelled,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.state, LocalCliTaskState::Cancelled);
        assert!(claude_input_pending(&state.task));
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].state, LocalCliMessageState::Sent);
        assert_eq!(saved[0].receipt_kind, None);
        let late_ack = event(
            &state,
            RuntimeEventKind::MessageAccepted {
                message_id: id,
                turn_id: Some(id.to_string()),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &late_ack,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let started = event(
            &state,
            RuntimeEventKind::TurnStarted {
                turn_id: id.to_string(),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &started,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let generations = load_task_generations(&writer.sender, state.task.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(generations[0].state, LocalCliTaskState::Cancelled);
        assert_eq!(generations[1].state, LocalCliTaskState::Running);
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].state, LocalCliMessageState::Acknowledged);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn disconnected_claude_queue_keeps_unconfirmed_delivery_without_replay() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("queue.sqlite")).unwrap();
    block_on(async {
        let mut state = persisted_running_claude(&writer.sender).await;
        let token = Uuid::new_v4();
        let (controller, mut commands, _sender, _events) = channels(token);
        let id = Uuid::from_u128(11);
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            id,
            RuntimeAction::Submit {
                input: vec![InputContent::Text("不能自动重放".into())],
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(commands.try_recv().unwrap().message_id, id);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        let disconnected = event(
            &state,
            RuntimeEventKind::Disconnected {
                reason: "EOF".into(),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &disconnected,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let recovered = load_tasks(&writer.sender, true)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered[0].state, LocalCliTaskState::Disconnected);
        assert_eq!(recovered[0].generation, 1);
        assert!(claude_input_pending(&recovered[0]));
        assert_eq!(recovered[0].result, None);
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].state, LocalCliMessageState::Sent);
        assert_eq!(saved[0].receipt_kind, None);
        assert!(saved[0].body.contains("不能自动重放"));
        assert!(commands.try_recv().is_err());
        assert!(
            send_user_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                id,
                RuntimeAction::Submit {
                    input: vec![InputContent::Text("不能自动重放".into())]
                },
                false
            )
            .await
            .is_err()
        );
        assert!(commands.try_recv().is_err());
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn premature_claude_queue_terminal_or_overlapping_start_cannot_replace_the_active_turn() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("queue.sqlite")).unwrap();
    block_on(async {
        let mut state = persisted_running_claude(&writer.sender).await;
        let token = Uuid::new_v4();
        let (controller, mut commands, _sender, _events) = channels(token);
        let id = Uuid::from_u128(11);
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            id,
            RuntimeAction::Submit {
                input: vec![InputContent::Text("等待当前轮结束".into())],
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(commands.try_recv().unwrap().message_id, id);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        let ack = event(
            &state,
            RuntimeEventKind::MessageAccepted {
                message_id: id,
                turn_id: Some(id.to_string()),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &ack,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let before = state.task.clone();
        let premature = event(
            &state,
            RuntimeEventKind::TurnFinished {
                turn_id: id.to_string(),
                outcome: TurnOutcome::Cancelled,
                output: "".into(),
            },
        );
        assert!(
            commit_runtime_event(
                &writer.sender,
                &mut state,
                &premature,
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, before);
        let overlap = event(
            &state,
            RuntimeEventKind::TurnStarted {
                turn_id: id.to_string(),
            },
        );
        assert!(
            commit_runtime_event(
                &writer.sender,
                &mut state,
                &overlap,
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, before);
        assert_eq!(state.output, "当前回合输出");
        assert_eq!(
            state.active_turn_id.as_deref(),
            Some(Uuid::from_u128(10).to_string().as_str())
        );
        assert!(claude_input_pending(&state.task));
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn claude_fixed_profile_is_committed_before_ready_and_cannot_change_on_repeated_ready() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("profile.sqlite")).unwrap();
    block_on(async {
        let mut state = snapshot();
        state.ready = false;
        state.task.harness = "claude".into();
        state.task.working_directory = directory.path().to_string_lossy().into();
        state.task.config_json = json!({"permission_policy":"ClaudeRestrictedFilesV1"}).to_string();
        checkpoint_task(&writer.sender, state.task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let profile = json!({
            "version":1,
            "workingDirectory":state.task.working_directory,
            "canonicalWorkingDirectory":std::fs::canonicalize(&state.task.working_directory).unwrap(),
            "executableSha256":"953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb",
            "denyRules":[], "sourceRules":[], "localTools":null,
        });
        let permissions = json!({
            "permissionMode":"default",
            "fixedProfileVerified":true,
            "claudeRestrictedFilesV1":profile,
        });
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        let ready = event(
            &state,
            RuntimeEventKind::SessionReady {
                verified_cli_version: Some("2.1.273".into()),
                effective_permissions: permissions.clone(),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &ready,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert!(state.ready);
        let stored = load_tasks(&writer.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored, [state.task.clone()]);
        let config: serde_json::Value = serde_json::from_str(&stored[0].config_json).unwrap();
        assert_eq!(config["claude_profile"], profile);
        assert!(
            load_messages(&writer.sender, state.task.task_id.clone(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap()
                .is_empty()
        );

        for replacement in [
            json!({"fixedProfileVerified":false,"claudeRestrictedFilesV1":profile}),
            json!({"fixedProfileVerified":true}),
            {
                let mut changed = permissions.clone();
                changed["permissionMode"] = json!("plan");
                changed
            },
            {
                let mut changed = permissions.clone();
                changed["permissionMode"] = json!("bypassPermissions");
                changed
            },
            {
                let mut changed = permissions.clone();
                changed["claudeRestrictedFilesV1"]["localTools"] =
                    json!({"allow_spawn":true,"allow_message":true});
                changed
            },
            {
                let mut changed = permissions.clone();
                changed["claudeRestrictedFilesV1"]["workingDirectory"] = json!("relative/path");
                changed
            },
        ] {
            let before = state.task.clone();
            let invalid = event(
                &state,
                RuntimeEventKind::SessionReady {
                    verified_cli_version: Some("2.1.273".into()),
                    effective_permissions: replacement,
                },
            );
            assert!(
                commit_runtime_event(
                    &writer.sender,
                    &mut state,
                    &invalid,
                    &mut accepted,
                    &mut finished
                )
                .await
                .is_err()
            );
            assert_eq!(state.task, before);
            assert_eq!(
                load_tasks(&writer.sender, false)
                    .unwrap()
                    .await
                    .unwrap()
                    .unwrap(),
                [before]
            );
        }
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_ready_commits_verified_version_without_claiming_fixed_permissions_or_local_tools() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("grok-ready.sqlite")).unwrap();
    block_on(async {
        let mut state = snapshot();
        state.task.harness = "grok".into();
        state.ready = false;
        state.task.config_json =
            json!({"permission_policy":"Inherit", "local_tools":null}).to_string();
        checkpoint_task(&writer.sender, state.task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let permissions = json!({"requestedPolicy":"inherit", "effectiveNativePolicy":null,
            "permissionEnforcementVerified":false, "verifiedCapabilities":{"submit":true,
                "queuedSubmit":true, "approval":true, "cancel":true, "resume":true,
                "steer":false, "localTools":false, "childTasks":false}});
        let ready = event(
            &state,
            RuntimeEventKind::SessionReady {
                verified_cli_version: Some("1.0.30".into()),
                effective_permissions: permissions.clone(),
            },
        );
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &ready,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let stored = load_tasks(&writer.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert!(state.ready);
        assert_eq!(stored, [state.task.clone()]);
        let config: Value = serde_json::from_str(&stored[0].config_json).unwrap();
        assert_eq!(config["cli_version"], "1.0.30");
        assert_eq!(config["effective_permissions"], permissions);
        assert!(config["permission_ceiling"].is_null());
        assert!(config["local_tools"].is_null());
        assert!(accepted.is_empty());
        assert!(finished.is_empty());
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

async fn persisted_grok_runtime(
    sender: &SyncSender<ModelEvent>,
    token: Uuid,
) -> ManagedTaskSnapshot {
    let mut state = snapshot();
    state.task.harness = "grok".into();
    state.task.config_json = json!({"runtime_generation":token}).to_string();
    checkpoint_task(sender, state.task.clone(), None)
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    state
}

async fn commit_grok_kind(
    sender: &SyncSender<ModelEvent>,
    state: &mut ManagedTaskSnapshot,
    token: Uuid,
    kind: RuntimeEventKind,
    accepted: &mut HashSet<String>,
    finished: &mut HashSet<String>,
) -> Result<bool, String> {
    let mut native_event = event(state, kind);
    native_event.generation = token;
    commit_runtime_event(sender, state, &native_event, accepted, finished).await
}

async fn submit_grok_input(
    sender: &SyncSender<ModelEvent>,
    state: &mut ManagedTaskSnapshot,
    controller: &RuntimeController,
    token: Uuid,
    message_id: Uuid,
) {
    send_user_request(
        sender,
        state,
        controller,
        token,
        message_id,
        RuntimeAction::Submit {
            input: vec![InputContent::Text("相同正文不用于关联输入".into())],
        },
        false,
    )
    .await
    .unwrap();
}

#[test]
fn grok_queued_inputs_keep_original_receipts_and_native_links_across_generations_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("grok-queue.sqlite");
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    let token = Uuid::new_v4();
    let a = Uuid::from_u128(101);
    let b = Uuid::from_u128(102);
    let c = Uuid::from_u128(103);
    let task_id = block_on(async {
        let mut state = persisted_grok_runtime(&writer.sender, token).await;
        let (controller, mut commands, _sender, _events) = channels(token);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        submit_grok_input(&writer.sender, &mut state, &controller, token, a).await;
        commands.try_recv().unwrap();
        for kind in [
            RuntimeEventKind::MessageAccepted {
                message_id: a,
                turn_id: Some("native-a".into()),
            },
            RuntimeEventKind::TurnStarted {
                turn_id: "native-a".into(),
            },
        ] {
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                kind,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
        }
        submit_grok_input(&writer.sender, &mut state, &controller, token, b).await;
        commands.try_recv().unwrap();
        submit_grok_input(&writer.sender, &mut state, &controller, token, b).await;
        assert!(commands.try_recv().is_err());
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::TurnFinished {
                turn_id: "native-a".into(),
                outcome: TurnOutcome::Completed,
                output: "第一轮完整结果".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::MessageAccepted {
                message_id: b,
                turn_id: Some("native-b".into()),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        // 终态提交 C 创建 Queued 代 2；FIFO 中 B 先绑定执行 2，C 原提交代仍为 2。
        submit_grok_input(&writer.sender, &mut state, &controller, token, c).await;
        commands.try_recv().unwrap();
        assert_eq!(state.task.generation, 2);
        assert_eq!(state.task.state, LocalCliTaskState::Queued);
        assert!(grok_input_links(&state.task).unwrap().current.is_none());
        let queued = grok_input_links(&state.task).unwrap().pending;
        assert_eq!(queued[0].message_id, b);
        assert_eq!(queued[0].submission_generation, 1);
        assert_eq!(queued[1].message_id, c);
        assert_eq!(queued[1].submission_generation, 2);
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::TurnStarted {
                turn_id: "native-b".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.generation, 2);
        let link = grok_input_links(&state.task).unwrap().current.unwrap();
        assert_eq!(link.message_id, b);
        assert_eq!(link.submission_generation, 1);
        assert_eq!(link.native_turn_id.as_deref(), Some("native-b"));
        submit_grok_input(&writer.sender, &mut state, &controller, token, b).await;
        assert!(commands.try_recv().is_err(), "跨代重投不能再写原生协议");
        for kind in [
            RuntimeEventKind::MessageAccepted {
                message_id: a,
                turn_id: Some("native-a".into()),
            },
            RuntimeEventKind::TurnStarted {
                turn_id: "native-a".into(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: "native-a".into(),
                outcome: TurnOutcome::Completed,
                output: "过时结果".into(),
            },
        ] {
            assert!(
                !commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    kind,
                    &mut accepted,
                    &mut finished
                )
                .await
                .unwrap()
            );
        }
        for kind in [
            RuntimeEventKind::TurnFinished {
                turn_id: "native-b".into(),
                outcome: TurnOutcome::Cancelled,
                output: "原生确认取消 B".into(),
            },
            RuntimeEventKind::MessageAccepted {
                message_id: c,
                turn_id: Some("native-c".into()),
            },
        ] {
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                kind,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
        }
        assert!(
            !commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::MessageAccepted {
                    message_id: c,
                    turn_id: Some("native-c".into())
                },
                &mut accepted,
                &mut finished
            )
            .await
            .unwrap()
        );
        for kind in [
            RuntimeEventKind::TurnStarted {
                turn_id: "native-c".into(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: "native-c".into(),
                outcome: TurnOutcome::Completed,
                output: "取消后等待的 C 完成".into(),
            },
        ] {
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                kind,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
        }
        assert_eq!(state.task.generation, 3);
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.len(), 2);
        assert!(saved.iter().all(|message| message.recipient_generation == 1
            && message.sender_generation == 1
            && message.state == LocalCliMessageState::Acknowledged
            && message.receipt_kind == Some(LocalCliReceiptKind::NativeProtocol)));
        let saved_c = load_messages(&writer.sender, state.task.task_id.clone(), 2)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved_c.len(), 1);
        assert_eq!(saved_c[0].message_id, c.to_string());
        assert_eq!(saved_c[0].recipient_generation, 2);
        assert_eq!(saved_c[0].state, LocalCliMessageState::Acknowledged);
        state.task.task_id
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
    let restarted = crate::persistence::start_test_writer(&database).unwrap();
    block_on(async {
        let history = load_task_generations(&restarted.sender, task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.len(), 3);
        for (record, id, turn, original_generation) in [
            (&history[0], a, "native-a", 1),
            (&history[1], b, "native-b", 1),
            (&history[2], c, "native-c", 2),
        ] {
            let links = grok_input_links(record).unwrap();
            let input = links.current.unwrap();
            assert_eq!(input.message_id, id);
            assert_eq!(input.submission_generation, original_generation);
            assert_eq!(input.runtime_generation, token);
            assert_eq!(input.native_turn_id.as_deref(), Some(turn));
            assert!(record.terminal_evidence.is_some());
        }
        assert_eq!(history[0].result.as_deref(), Some("第一轮完整结果"));
        assert_eq!(history[1].state, LocalCliTaskState::Cancelled);
        assert_eq!(history[2].result.as_deref(), Some("取消后等待的 C 完成"));
        let current = load_tasks(&restarted.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current, [history[2].clone()]);
        assert!(grok_input_links(&current[0]).unwrap().pending.is_empty());
    });
    restarted.sender.send(ModelEvent::Terminate).unwrap();
    restarted.handle.join().unwrap();
}

#[test]
fn grok_prepared_input_is_linked_once_and_unknown_or_stale_receipts_cannot_start_it() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("grok-proof.sqlite")).unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let mut state = persisted_grok_runtime(&writer.sender, token).await;
        let (controller, mut commands, _sender, _events) = channels(token);
        let id = Uuid::new_v4();
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text("初始准备输入".into())],
        };
        enqueue_message(
            &writer.sender,
            input_message(&state.task, id, &action).unwrap(),
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            id,
            action.clone(),
            true,
        )
        .await
        .unwrap();
        assert_eq!(commands.try_recv().unwrap().action, action);
        send_user_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            id,
            action,
            true,
        )
        .await
        .unwrap();
        assert!(commands.try_recv().is_err());
        assert_eq!(grok_input_links(&state.task).unwrap().pending.len(), 1);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        let before = state.task.clone();
        assert!(
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::TurnStarted {
                    turn_id: "unacknowledged".into()
                },
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        assert!(
            !commit_grok_kind(
                &writer.sender,
                &mut state,
                Uuid::new_v4(),
                RuntimeEventKind::MessageAccepted {
                    message_id: id,
                    turn_id: Some("native-proof".into())
                },
                &mut accepted,
                &mut finished
            )
            .await
            .unwrap()
        );
        assert!(
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::MessageAccepted {
                    message_id: Uuid::new_v4(),
                    turn_id: Some("native-proof".into())
                },
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, before);
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::MessageAccepted {
                message_id: id,
                turn_id: Some("native-proof".into()),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let after_ack = state.task.clone();
        assert!(
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::MessageAccepted {
                    message_id: id,
                    turn_id: Some("conflicting-native-id".into())
                },
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, after_ack);
        // 清空内存关联后，原生 started 仍需 SQLite 中实际 ACK 与持久回合映射共同证明。
        accepted.clear();
        state.task = load_tasks(&writer.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .remove(0);
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::TurnStarted {
                turn_id: "native-proof".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.generation, 1);
        assert_eq!(state.task.state, LocalCliTaskState::Running);
        assert_eq!(
            grok_input_links(&state.task)
                .unwrap()
                .current
                .unwrap()
                .message_id,
            id
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_pending_queue_is_bounded_and_closed_delivery_does_not_leave_a_live_link() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("grok-limit.sqlite")).unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let mut state = persisted_grok_runtime(&writer.sender, token).await;
        let (controller, mut commands, _sender, _events) = channels(token);
        for _ in 0..MAX_GROK_PENDING_INPUTS {
            submit_grok_input(
                &writer.sender,
                &mut state,
                &controller,
                token,
                Uuid::new_v4(),
            )
            .await;
            commands.try_recv().unwrap();
        }
        let before = state.task.clone();
        assert!(
            send_user_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                Uuid::new_v4(),
                RuntimeAction::Submit {
                    input: vec![InputContent::Text("超出等待容量".into())]
                },
                false
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, before);
        assert!(commands.try_recv().is_err());
        let mut rejected = persisted_grok_runtime(&writer.sender, token).await;
        let id = Uuid::new_v4();
        drop(commands);
        assert!(
            send_user_request(
                &writer.sender,
                &mut rejected,
                &controller,
                token,
                id,
                RuntimeAction::Submit {
                    input: vec![InputContent::Text("保留失败输入".into())]
                },
                false
            )
            .await
            .is_err()
        );
        assert!(grok_input_links(&rejected.task).unwrap().pending.is_empty());
        let stored = load_messages(&writer.sender, rejected.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored[0].state, LocalCliMessageState::Failed);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_old_submission_failure_commits_before_clearing_only_its_pending_link() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("grok-failure.sqlite"))
            .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let mut state = persisted_grok_runtime(&writer.sender, token).await;
        let (controller, mut commands, _sender, _events) = channels(token);
        let a = Uuid::from_u128(201);
        let b = Uuid::from_u128(202);
        let c = Uuid::from_u128(203);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        submit_grok_input(&writer.sender, &mut state, &controller, token, a).await;
        commands.try_recv().unwrap();
        for kind in [
            RuntimeEventKind::MessageAccepted {
                message_id: a,
                turn_id: Some("failure-a".into()),
            },
            RuntimeEventKind::TurnStarted {
                turn_id: "failure-a".into(),
            },
        ] {
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                kind,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
        }
        for id in [b, c] {
            submit_grok_input(&writer.sender, &mut state, &controller, token, id).await;
            commands.try_recv().unwrap();
        }
        for kind in [
            RuntimeEventKind::TurnFinished {
                turn_id: "failure-a".into(),
                outcome: TurnOutcome::Completed,
                output: "A 结果".into(),
            },
            RuntimeEventKind::MessageAccepted {
                message_id: b,
                turn_id: Some("failure-b".into()),
            },
            RuntimeEventKind::TurnStarted {
                turn_id: "failure-b".into(),
            },
        ] {
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                kind,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
        }
        assert_eq!(state.task.generation, 2);
        let stale_failure = RuntimeEventKind::RequestFailed {
            message_id: c,
            message: "旧运行失败".into(),
        };
        let before = state.task.clone();
        assert!(
            !commit_grok_kind(
                &writer.sender,
                &mut state,
                Uuid::new_v4(),
                stale_failure,
                &mut accepted,
                &mut finished
            )
            .await
            .unwrap()
        );
        assert_eq!(state.task, before);
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::RequestFailed {
                message_id: c,
                message: "原生明确拒绝 C".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert!(grok_input_links(&state.task).unwrap().pending.is_empty());
        assert_eq!(
            grok_input_links(&state.task)
                .unwrap()
                .current
                .unwrap()
                .message_id,
            b
        );
        assert_eq!(state.task.generation, 2);
        assert_eq!(state.task.state, LocalCliTaskState::Running);
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let failed = saved
            .iter()
            .find(|message| message.message_id == c.to_string())
            .unwrap();
        assert_eq!(failed.recipient_generation, 1);
        assert_eq!(failed.state, LocalCliMessageState::Failed);
        assert_eq!(failed.receipt_kind, None);
        assert!(
            !commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::TurnFinished {
                    turn_id: "failure-c".into(),
                    outcome: TurnOutcome::Completed,
                    output: "未启动回合不能收回成功".into()
                },
                &mut accepted,
                &mut finished
            )
            .await
            .unwrap()
        );
        assert_eq!(state.task.state, LocalCliTaskState::Running);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_native_ack_requires_matching_session_fifo_and_committed_delivery_proof() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("grok-identity.sqlite"))
            .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let mut state = persisted_grok_runtime(&writer.sender, token).await;
        let (controller, mut commands, _sender, _events) = channels(token);
        let a = Uuid::from_u128(301);
        let b = Uuid::from_u128(302);
        for id in [a, b] {
            submit_grok_input(&writer.sender, &mut state, &controller, token, id).await;
            commands.try_recv().unwrap();
        }
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        let before = state.task.clone();
        assert!(
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::MessageAccepted {
                    message_id: b,
                    turn_id: Some("wrong-first".into())
                },
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        let mut wrong_session = event(
            &state,
            RuntimeEventKind::MessageAccepted {
                message_id: a,
                turn_id: Some("native-identity-a".into()),
            },
        );
        wrong_session.generation = token;
        wrong_session.native_session_id = Some("other-session".into());
        assert!(
            commit_runtime_event(
                &writer.sender,
                &mut state,
                &wrong_session,
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, before);
        // 只伪造持久 native turn 不足以 started；原消息仍 Sent，必须拒绝而保持 Queued。
        let previous = state.task.clone();
        let mut links = grok_input_links(&state.task).unwrap();
        links.pending[0].native_turn_id = Some("native-identity-a".into());
        set_grok_input_links(&mut state.task, &links).unwrap();
        commit_transition(&writer.sender, &mut state.task, &previous)
            .await
            .unwrap();
        let before = state.task.clone();
        accepted.insert("native-identity-a".into());
        assert!(
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::TurnStarted {
                    turn_id: "native-identity-a".into()
                },
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, before);
        assert_eq!(state.task.state, LocalCliTaskState::Queued);
        let mut invalid = state.task.clone();
        let mut links = grok_input_links(&invalid).unwrap();
        links.pending.push(links.pending[0].clone());
        set_grok_input_links(&mut invalid, &links).unwrap();
        assert!(grok_input_links(&invalid).is_err());
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_terminal_gap_preserves_multiple_inputs_with_or_without_head_ack() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("grok-terminal-gap.sqlite");
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    let task_ids = block_on(async {
        let mut ids = Vec::new();
        for ack_before_gap in [false, true] {
            for outcome in [
                TurnOutcome::Completed,
                TurnOutcome::Cancelled,
                TurnOutcome::Failed {
                    message: "原生 A 失败".into(),
                },
            ] {
                let token = Uuid::new_v4();
                let mut state = persisted_grok_runtime(&writer.sender, token).await;
                let (controller, mut commands, _sender, _events) = channels(token);
                let inputs = [
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                ];
                let mut accepted = HashSet::new();
                let mut finished = HashSet::new();
                submit_grok_input(&writer.sender, &mut state, &controller, token, inputs[0]).await;
                commands.try_recv().unwrap();
                for kind in [
                    RuntimeEventKind::MessageAccepted {
                        message_id: inputs[0],
                        turn_id: Some("gap-a".into()),
                    },
                    RuntimeEventKind::TurnStarted {
                        turn_id: "gap-a".into(),
                    },
                ] {
                    commit_grok_kind(
                        &writer.sender,
                        &mut state,
                        token,
                        kind,
                        &mut accepted,
                        &mut finished,
                    )
                    .await
                    .unwrap();
                }
                submit_grok_input(&writer.sender, &mut state, &controller, token, inputs[1]).await;
                commands.try_recv().unwrap();
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    RuntimeEventKind::TurnFinished {
                        turn_id: "gap-a".into(),
                        outcome,
                        output: "A 原生结果".into(),
                    },
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
                let b_ack = RuntimeEventKind::MessageAccepted {
                    message_id: inputs[1],
                    turn_id: Some("gap-b".into()),
                };
                if ack_before_gap {
                    commit_grok_kind(
                        &writer.sender,
                        &mut state,
                        token,
                        b_ack.clone(),
                        &mut accepted,
                        &mut finished,
                    )
                    .await
                    .unwrap();
                }
                for id in [inputs[2], inputs[3]] {
                    submit_grok_input(&writer.sender, &mut state, &controller, token, id).await;
                    commands.try_recv().unwrap();
                }
                assert_eq!(state.task.generation, 2);
                assert_eq!(state.task.state, LocalCliTaskState::Queued);
                let links = grok_input_links(&state.task).unwrap();
                assert!(links.current.is_none());
                assert_eq!(
                    links
                        .pending
                        .iter()
                        .map(|input| (input.message_id, input.submission_generation))
                        .collect::<Vec<_>>(),
                    [(inputs[1], 1), (inputs[2], 2), (inputs[3], 2)]
                );
                if !ack_before_gap {
                    commit_grok_kind(
                        &writer.sender,
                        &mut state,
                        token,
                        b_ack,
                        &mut accepted,
                        &mut finished,
                    )
                    .await
                    .unwrap();
                }
                for (id, turn, execution_generation) in [
                    (inputs[1], "gap-b", 2),
                    (inputs[2], "gap-c", 3),
                    (inputs[3], "gap-d", 4),
                ] {
                    if id != inputs[1] {
                        commit_grok_kind(
                            &writer.sender,
                            &mut state,
                            token,
                            RuntimeEventKind::MessageAccepted {
                                message_id: id,
                                turn_id: Some(turn.into()),
                            },
                            &mut accepted,
                            &mut finished,
                        )
                        .await
                        .unwrap();
                    }
                    commit_grok_kind(
                        &writer.sender,
                        &mut state,
                        token,
                        RuntimeEventKind::TurnStarted {
                            turn_id: turn.into(),
                        },
                        &mut accepted,
                        &mut finished,
                    )
                    .await
                    .unwrap();
                    assert_eq!(state.task.generation, execution_generation);
                    assert_eq!(
                        grok_input_links(&state.task)
                            .unwrap()
                            .current
                            .unwrap()
                            .message_id,
                        id
                    );
                    submit_grok_input(&writer.sender, &mut state, &controller, token, id).await;
                    assert!(commands.try_recv().is_err());
                    commit_grok_kind(
                        &writer.sender,
                        &mut state,
                        token,
                        RuntimeEventKind::TurnFinished {
                            turn_id: turn.into(),
                            outcome: TurnOutcome::Completed,
                            output: format!("真实结果 {turn}"),
                        },
                        &mut accepted,
                        &mut finished,
                    )
                    .await
                    .unwrap();
                }
                for (generation, expected) in [
                    (1, vec![inputs[0], inputs[1]]),
                    (2, vec![inputs[2], inputs[3]]),
                ] {
                    let messages =
                        load_messages(&writer.sender, state.task.task_id.clone(), generation)
                            .unwrap()
                            .await
                            .unwrap()
                            .unwrap();
                    assert_eq!(messages.len(), expected.len());
                    assert!(messages.iter().all(|message| {
                        message.recipient_generation == generation
                            && message.state == LocalCliMessageState::Acknowledged
                            && expected
                                .iter()
                                .any(|id| id.to_string() == message.message_id)
                    }));
                }
                ids.push(state.task.task_id);
            }
        }
        ids
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
    let reopened = crate::persistence::start_test_writer(&database).unwrap();
    block_on(async {
        for id in task_ids {
            let history = load_task_generations(&reopened.sender, id)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(history.len(), 4);
            for (record, turn, original_generation) in [
                (&history[1], "gap-b", 1),
                (&history[2], "gap-c", 2),
                (&history[3], "gap-d", 2),
            ] {
                let link = grok_input_links(record).unwrap().current.unwrap();
                assert_eq!(link.native_turn_id.as_deref(), Some(turn));
                assert_eq!(link.submission_generation, original_generation);
                assert!(record.result.is_some());
            }
        }
    });
    reopened.sender.send(ModelEvent::Terminate).unwrap();
    reopened.handle.join().unwrap();
}

#[test]
fn grok_acknowledged_head_failure_before_started_is_persisted_without_rewriting_ack() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("grok-prestart-failure.sqlite");
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    let task_ids = block_on(async {
        let mut ids = Vec::new();
        for open_gap_generation in [false, true] {
            let token = Uuid::new_v4();
            let mut state = persisted_grok_runtime(&writer.sender, token).await;
            let (controller, mut commands, _sender, _events) = channels(token);
            let a = Uuid::new_v4();
            let b = Uuid::new_v4();
            let c = Uuid::new_v4();
            let mut accepted = HashSet::new();
            let mut finished = HashSet::new();
            submit_grok_input(&writer.sender, &mut state, &controller, token, a).await;
            commands.try_recv().unwrap();
            for kind in [
                RuntimeEventKind::MessageAccepted {
                    message_id: a,
                    turn_id: Some("prestart-a".into()),
                },
                RuntimeEventKind::TurnStarted {
                    turn_id: "prestart-a".into(),
                },
            ] {
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    kind,
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
            }
            submit_grok_input(&writer.sender, &mut state, &controller, token, b).await;
            commands.try_recv().unwrap();
            if !open_gap_generation {
                submit_grok_input(&writer.sender, &mut state, &controller, token, c).await;
                commands.try_recv().unwrap();
            }
            for kind in [
                RuntimeEventKind::TurnFinished {
                    turn_id: "prestart-a".into(),
                    outcome: TurnOutcome::Completed,
                    output: "A 完成".into(),
                },
                RuntimeEventKind::MessageAccepted {
                    message_id: b,
                    turn_id: Some("prestart-b".into()),
                },
            ] {
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    kind,
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
            }
            if open_gap_generation {
                submit_grok_input(&writer.sender, &mut state, &controller, token, c).await;
                commands.try_recv().unwrap();
            }
            assert!(state.active_turn_id.is_none());
            let failed = RuntimeEventKind::TurnFinished {
                turn_id: "prestart-b".into(),
                outcome: TurnOutcome::Failed {
                    message: "原生 RPC 失败".into(),
                },
                output: "B 原生失败结果".into(),
            };
            assert!(
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    failed.clone(),
                    &mut accepted,
                    &mut finished
                )
                .await
                .unwrap()
            );
            assert_eq!(state.task.generation, 2);
            assert_eq!(state.task.state, LocalCliTaskState::Failed);
            assert!(state.active_turn_id.is_none());
            assert_eq!(state.task.result.as_deref(), Some("B 原生失败结果"));
            let links = grok_input_links(&state.task).unwrap();
            assert_eq!(links.current.unwrap().message_id, b);
            assert_eq!(links.pending.len(), 1);
            assert_eq!(links.pending[0].message_id, c);
            let original_c = if open_gap_generation { 2 } else { 1 };
            assert_eq!(links.pending[0].submission_generation, original_c);
            let messages = load_messages(&writer.sender, state.task.task_id.clone(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            let b_message = messages
                .iter()
                .find(|message| message.message_id == b.to_string())
                .unwrap();
            assert_eq!(b_message.state, LocalCliMessageState::Acknowledged);
            assert_eq!(
                b_message.receipt_kind,
                Some(LocalCliReceiptKind::NativeProtocol)
            );
            assert_eq!(b_message.recipient_generation, 1);
            assert!(
                !commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    failed,
                    &mut accepted,
                    &mut finished
                )
                .await
                .unwrap()
            );
            for kind in [
                RuntimeEventKind::MessageAccepted {
                    message_id: c,
                    turn_id: Some("prestart-c".into()),
                },
                RuntimeEventKind::TurnStarted {
                    turn_id: "prestart-c".into(),
                },
                RuntimeEventKind::TurnFinished {
                    turn_id: "prestart-c".into(),
                    outcome: TurnOutcome::Completed,
                    output: "C 后续真实完成".into(),
                },
            ] {
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    kind,
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
            }
            assert_eq!(state.task.generation, 3);
            assert_eq!(
                grok_input_links(&state.task)
                    .unwrap()
                    .current
                    .unwrap()
                    .submission_generation,
                original_c
            );
            ids.push(state.task.task_id);
        }
        ids
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
    let reopened = crate::persistence::start_test_writer(&database).unwrap();
    block_on(async {
        for id in task_ids {
            let history = load_task_generations(&reopened.sender, id)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(history.len(), 3);
            assert_eq!(history[1].state, LocalCliTaskState::Failed);
            assert_eq!(
                grok_input_links(&history[1])
                    .unwrap()
                    .current
                    .unwrap()
                    .native_turn_id
                    .as_deref(),
                Some("prestart-b")
            );
            let evidence: Value =
                serde_json::from_str(history[1].terminal_evidence.as_deref().unwrap()).unwrap();
            assert!(matches!(
                serde_json::from_value::<RuntimeEventKind>(evidence["event"].clone()).unwrap(),
                RuntimeEventKind::TurnFinished {
                    outcome: TurnOutcome::Failed { .. },
                    ..
                }
            ));
            assert_eq!(history[2].result.as_deref(), Some("C 后续真实完成"));
        }
    });
    reopened.sender.send(ModelEvent::Terminate).unwrap();
    reopened.handle.join().unwrap();
}

#[test]
fn grok_failure_before_started_requires_real_ack_fifo_and_current_connection() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("grok-prestart-reject.sqlite");
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let mut state = persisted_grok_runtime(&writer.sender, token).await;
        let (controller, mut commands, _sender, _events) = channels(token);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        submit_grok_input(&writer.sender, &mut state, &controller, token, a).await;
        commands.try_recv().unwrap();
        for kind in [
            RuntimeEventKind::MessageAccepted {
                message_id: a,
                turn_id: Some("reject-a".into()),
            },
            RuntimeEventKind::TurnStarted {
                turn_id: "reject-a".into(),
            },
        ] {
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                kind,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
        }
        submit_grok_input(&writer.sender, &mut state, &controller, token, b).await;
        commands.try_recv().unwrap();
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::TurnFinished {
                turn_id: "reject-a".into(),
                outcome: TurnOutcome::Completed,
                output: "A 已提交结果".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let b_failure = RuntimeEventKind::TurnFinished {
            turn_id: "reject-b".into(),
            outcome: TurnOutcome::Failed {
                message: "未启动的原生失败".into(),
            },
            output: "拒绝此结果".into(),
        };
        let before = state.task.clone();
        assert!(
            !commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                b_failure.clone(),
                &mut accepted,
                &mut finished
            )
            .await
            .unwrap()
        );
        assert_eq!(state.task, before);
        // 映射和内存 ACK 都不能代替 SQLite 中实际的 NativeProtocol 回执。
        let mut links = grok_input_links(&state.task).unwrap();
        links.pending[0].native_turn_id = Some("reject-b".into());
        set_grok_input_links(&mut state.task, &links).unwrap();
        commit_transition(&writer.sender, &mut state.task, &before)
            .await
            .unwrap();
        accepted.insert("reject-b".into());
        let before = state.task.clone();
        assert!(
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                b_failure.clone(),
                &mut accepted,
                &mut finished
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, before);
        links.pending[0].native_turn_id = None;
        set_grok_input_links(&mut state.task, &links).unwrap();
        commit_transition(&writer.sender, &mut state.task, &before)
            .await
            .unwrap();
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::MessageAccepted {
                message_id: b,
                turn_id: Some("reject-b".into()),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        // 保留原生 ACK 和完整关联，仅损坏消息版本；任何结果或换代均不得信任它。
        let mut database_connection =
            SqliteConnection::establish(database.to_str().unwrap()).unwrap();
        let stored_ack: String = local_cli_messages::table
            .filter(local_cli_messages::message_id.eq(b.to_string()))
            .select(local_cli_messages::data)
            .first(&mut database_connection)
            .unwrap();
        let mut incompatible_ack: LocalCliMessage = serde_json::from_str(&stored_ack).unwrap();
        incompatible_ack.version = 2;
        let damaged_ack = serde_json::to_string(&incompatible_ack).unwrap();
        diesel::update(
            local_cli_messages::table.filter(local_cli_messages::message_id.eq(b.to_string())),
        )
        .set(local_cli_messages::data.eq(&damaged_ack))
        .execute(&mut database_connection)
        .unwrap();
        let before_incompatible = state.task.clone();
        for kind in [
            RuntimeEventKind::TurnStarted {
                turn_id: "reject-b".into(),
            },
            b_failure.clone(),
        ] {
            assert!(
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    kind,
                    &mut accepted,
                    &mut finished
                )
                .await
                .is_err()
            );
            assert_eq!(state.task, before_incompatible);
        }
        assert!(
            send_user_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                Uuid::new_v4(),
                RuntimeAction::Submit {
                    input: vec![InputContent::Text("损坏回执禁止换代".into())]
                },
                false
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, before_incompatible);
        assert!(state.active_turn_id.is_none());
        assert!(commands.try_recv().is_err());
        let unchanged_ack: String = local_cli_messages::table
            .filter(local_cli_messages::message_id.eq(b.to_string()))
            .select(local_cli_messages::data)
            .first(&mut database_connection)
            .unwrap();
        assert_eq!(unchanged_ack, damaged_ack, "拒绝事件不能回写损坏原消息");
        assert_eq!(
            load_task_generations(&writer.sender, state.task.task_id.clone())
                .unwrap()
                .await
                .unwrap()
                .unwrap()
                .len(),
            1
        );
        diesel::update(
            local_cli_messages::table.filter(local_cli_messages::message_id.eq(b.to_string())),
        )
        .set(local_cli_messages::data.eq(stored_ack))
        .execute(&mut database_connection)
        .unwrap();
        let mut incompatible_task = state.clone();
        incompatible_task.task.version = 2;
        let incompatible_task_before = incompatible_task.task.clone();
        for kind in [
            RuntimeEventKind::TurnStarted {
                turn_id: "reject-b".into(),
            },
            b_failure.clone(),
        ] {
            assert!(
                commit_grok_kind(
                    &writer.sender,
                    &mut incompatible_task,
                    token,
                    kind,
                    &mut accepted,
                    &mut finished
                )
                .await
                .is_err()
            );
            assert_eq!(incompatible_task.task, incompatible_task_before);
        }
        assert!(
            send_user_request(
                &writer.sender,
                &mut incompatible_task,
                &controller,
                token,
                Uuid::new_v4(),
                RuntimeAction::Submit {
                    input: vec![InputContent::Text("未来任务版本禁止换代".into())]
                },
                false
            )
            .await
            .is_err()
        );
        assert_eq!(incompatible_task.task, incompatible_task_before);
        assert!(commands.try_recv().is_err());
        submit_grok_input(&writer.sender, &mut state, &controller, token, c).await;
        commands.try_recv().unwrap();
        // 跨代关联还须读取原任务版本；不能因当前任务为版本 1 而信任未来历史格式。
        let stored_history: String = local_cli_task_generations::table
            .filter(local_cli_task_generations::task_id.eq(&state.task.task_id))
            .filter(local_cli_task_generations::generation.eq(1_i64))
            .select(local_cli_task_generations::data)
            .first(&mut database_connection)
            .unwrap();
        let mut incompatible_history: LocalCliTask = serde_json::from_str(&stored_history).unwrap();
        incompatible_history.version = 2;
        let damaged_history = serde_json::to_string(&incompatible_history).unwrap();
        diesel::update(
            local_cli_task_generations::table
                .filter(local_cli_task_generations::task_id.eq(&state.task.task_id))
                .filter(local_cli_task_generations::generation.eq(1_i64)),
        )
        .set(local_cli_task_generations::data.eq(&damaged_history))
        .execute(&mut database_connection)
        .unwrap();
        let before_incompatible_history = state.task.clone();
        for kind in [
            RuntimeEventKind::TurnStarted {
                turn_id: "reject-b".into(),
            },
            b_failure.clone(),
        ] {
            assert!(
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    kind,
                    &mut accepted,
                    &mut finished
                )
                .await
                .is_err()
            );
            assert_eq!(state.task, before_incompatible_history);
        }
        assert!(
            verify_grok_pending_inputs(
                &writer.sender,
                &state.task,
                &grok_input_links(&state.task).unwrap(),
                token
            )
            .await
            .is_err()
        );
        assert!(state.active_turn_id.is_none());
        assert!(commands.try_recv().is_err());
        let unchanged_history: String = local_cli_task_generations::table
            .filter(local_cli_task_generations::task_id.eq(&state.task.task_id))
            .filter(local_cli_task_generations::generation.eq(1_i64))
            .select(local_cli_task_generations::data)
            .first(&mut database_connection)
            .unwrap();
        assert_eq!(unchanged_history, damaged_history);
        assert_eq!(
            load_task_generations(&writer.sender, state.task.task_id.clone())
                .unwrap()
                .await
                .unwrap()
                .unwrap()
                .len(),
            2
        );
        diesel::update(
            local_cli_task_generations::table
                .filter(local_cli_task_generations::task_id.eq(&state.task.task_id))
                .filter(local_cli_task_generations::generation.eq(1_i64)),
        )
        .set(local_cli_task_generations::data.eq(stored_history))
        .execute(&mut database_connection)
        .unwrap();
        // 给非队首映射一个候选 turn；其失败仍不得越过真实已确认的队首。
        let previous = state.task.clone();
        let mut links = grok_input_links(&state.task).unwrap();
        links.pending[1].native_turn_id = Some("reject-c".into());
        set_grok_input_links(&mut state.task, &links).unwrap();
        commit_transition(&writer.sender, &mut state.task, &previous)
            .await
            .unwrap();
        let before = state.task.clone();
        for kind in [
            RuntimeEventKind::TurnFinished {
                turn_id: "reject-b".into(),
                outcome: TurnOutcome::Completed,
                output: "尚未开始不能报告完成".into(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: "reject-b".into(),
                outcome: TurnOutcome::Cancelled,
                output: "尚未开始不能报告取消".into(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: "reject-c".into(),
                outcome: TurnOutcome::Failed {
                    message: "非队首失败".into(),
                },
                output: "不能越过 B".into(),
            },
            RuntimeEventKind::TurnFinished {
                turn_id: "reject-a".into(),
                outcome: TurnOutcome::Failed {
                    message: "旧 A 回调".into(),
                },
                output: "不能覆盖结果".into(),
            },
        ] {
            assert!(
                !commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    kind,
                    &mut accepted,
                    &mut finished
                )
                .await
                .unwrap()
            );
            assert_eq!(state.task, before);
        }
        let mut stale_runtime = event(&state, b_failure.clone());
        stale_runtime.generation = Uuid::new_v4();
        let mut stale_session = event(&state, b_failure.clone());
        stale_session.generation = token;
        stale_session.native_session_id = Some("其他会话".into());
        let mut missing_session = event(&state, b_failure.clone());
        missing_session.generation = token;
        missing_session.native_session_id = None;
        for callback in [stale_runtime, stale_session, missing_session] {
            assert!(
                !commit_runtime_event(
                    &writer.sender,
                    &mut state,
                    &callback,
                    &mut accepted,
                    &mut finished
                )
                .await
                .unwrap()
            );
            assert_eq!(state.task, before);
        }
        for (connected, ready, task_state) in [
            (false, true, LocalCliTaskState::Queued),
            (true, false, LocalCliTaskState::Queued),
            (true, true, LocalCliTaskState::Disconnected),
            (true, true, LocalCliTaskState::Unknown),
            (true, true, LocalCliTaskState::Unconfirmed),
        ] {
            let mut unavailable = state.clone();
            unavailable.connected = connected;
            unavailable.ready = ready;
            unavailable.task.state = task_state;
            let unavailable_task = unavailable.task.clone();
            assert!(
                !commit_grok_kind(
                    &writer.sender,
                    &mut unavailable,
                    token,
                    b_failure.clone(),
                    &mut accepted,
                    &mut finished
                )
                .await
                .unwrap()
            );
            assert_eq!(unavailable.task, unavailable_task);
        }
        finished.insert("reject-b".into());
        assert!(
            !commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                b_failure,
                &mut accepted,
                &mut finished
            )
            .await
            .unwrap()
        );
        assert_eq!(state.task, before);
        assert!(state.active_turn_id.is_none());
        let messages = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let b_message = messages
            .iter()
            .find(|message| message.message_id == b.to_string())
            .unwrap();
        assert_eq!(b_message.state, LocalCliMessageState::Acknowledged);
        assert_eq!(
            b_message.receipt_kind,
            Some(LocalCliReceiptKind::NativeProtocol)
        );
        let history = load_task_generations(&writer.sender, state.task.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].result.as_deref(), Some("A 已提交结果"));
        assert!(history[1].result.is_none());
        assert!(commands.try_recv().is_err());
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_terminal_gap_request_failure_preserves_other_original_submissions_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("grok-gap-request-failure.sqlite");
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    let d = Uuid::new_v4();
    let task_id = block_on(async {
        let token = Uuid::new_v4();
        let mut state = persisted_grok_runtime(&writer.sender, token).await;
        let (controller, mut commands, _sender, _events) = channels(token);
        let a = Uuid::new_v4();
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        submit_grok_input(&writer.sender, &mut state, &controller, token, a).await;
        commands.try_recv().unwrap();
        for kind in [
            RuntimeEventKind::MessageAccepted {
                message_id: a,
                turn_id: Some("request-a".into()),
            },
            RuntimeEventKind::TurnStarted {
                turn_id: "request-a".into(),
            },
        ] {
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                kind,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
        }
        submit_grok_input(&writer.sender, &mut state, &controller, token, b).await;
        commands.try_recv().unwrap();
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::TurnFinished {
                turn_id: "request-a".into(),
                outcome: TurnOutcome::Completed,
                output: "A 完成".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        for id in [c, d] {
            submit_grok_input(&writer.sender, &mut state, &controller, token, id).await;
            commands.try_recv().unwrap();
        }
        assert_eq!(state.task.generation, 2);
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::RequestFailed {
                message_id: b,
                message: "B 未被原生接收".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.state, LocalCliTaskState::Queued);
        let links = grok_input_links(&state.task).unwrap();
        assert!(links.current.is_none());
        assert_eq!(
            links
                .pending
                .iter()
                .map(|input| (input.message_id, input.submission_generation))
                .collect::<Vec<_>>(),
            [(c, 2), (d, 2)]
        );
        let b_messages = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let b_message = b_messages
            .iter()
            .find(|message| message.message_id == b.to_string())
            .unwrap();
        assert_eq!(b_message.recipient_generation, 1);
        assert_eq!(b_message.state, LocalCliMessageState::Failed);
        assert_eq!(b_message.receipt_kind, None);
        for (id, turn, generation) in [(c, "request-c", 2), (d, "request-d", 3)] {
            for kind in [
                RuntimeEventKind::MessageAccepted {
                    message_id: id,
                    turn_id: Some(turn.into()),
                },
                RuntimeEventKind::TurnStarted {
                    turn_id: turn.into(),
                },
                RuntimeEventKind::TurnFinished {
                    turn_id: turn.into(),
                    outcome: TurnOutcome::Completed,
                    output: format!("已验证结果 {turn}"),
                },
            ] {
                assert!(
                    commit_grok_kind(
                        &writer.sender,
                        &mut state,
                        token,
                        kind,
                        &mut accepted,
                        &mut finished
                    )
                    .await
                    .unwrap()
                );
            }
            assert_eq!(state.task.generation, generation);
            assert_eq!(
                grok_input_links(&state.task)
                    .unwrap()
                    .current
                    .unwrap()
                    .message_id,
                id
            );
            submit_grok_input(&writer.sender, &mut state, &controller, token, id).await;
            assert!(commands.try_recv().is_err());
        }
        state.task.task_id
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
    let reopened = crate::persistence::start_test_writer(&database).unwrap();
    block_on(async {
        let history = load_task_generations(&reopened.sender, task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.len(), 3);
        for (record, id, turn) in [(&history[1], c, "request-c"), (&history[2], d, "request-d")] {
            let link = grok_input_links(record).unwrap().current.unwrap();
            assert_eq!(link.message_id, id);
            assert_eq!(link.submission_generation, 2);
            assert_eq!(link.native_turn_id.as_deref(), Some(turn));
            assert!(record.result.is_some());
        }
        let messages = load_messages(&reopened.sender, task_id, 2)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert!(
            messages
                .iter()
                .all(|message| message.recipient_generation == 2
                    && message.state == LocalCliMessageState::Acknowledged
                    && message.receipt_kind == Some(LocalCliReceiptKind::NativeProtocol))
        );
    });
    reopened.sender.send(ModelEvent::Terminate).unwrap();
    reopened.handle.join().unwrap();
}

#[test]
fn grok_terminal_submit_rejects_any_pending_input_without_committed_delivery() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("grok-gap-prepared.sqlite"))
            .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let mut state = persisted_grok_runtime(&writer.sender, token).await;
        let (controller, mut commands, _sender, _events) = channels(token);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let prepared = Uuid::new_v4();
        let new_input = Uuid::new_v4();
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        submit_grok_input(&writer.sender, &mut state, &controller, token, a).await;
        commands.try_recv().unwrap();
        for kind in [
            RuntimeEventKind::MessageAccepted {
                message_id: a,
                turn_id: Some("prepared-a".into()),
            },
            RuntimeEventKind::TurnStarted {
                turn_id: "prepared-a".into(),
            },
        ] {
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                kind,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
        }
        submit_grok_input(&writer.sender, &mut state, &controller, token, b).await;
        commands.try_recv().unwrap();
        // 模拟准备输入已落盘、尚未交付的中断；后面的异常也必须挡住整个换代。
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text("准备但未派发".into())],
        };
        let previous = state.task.clone();
        let mut links = grok_input_links(&state.task).unwrap();
        links.pending.push(GrokInputLink {
            message_id: prepared,
            submission_generation: 1,
            runtime_generation: token,
            native_turn_id: None,
            mailbox_sha256: None,
        });
        set_grok_input_links(&mut state.task, &links).unwrap();
        let message = input_message(&state.task, prepared, &action).unwrap();
        state.task.revision = previous.revision + 1;
        checkpoint_task_with_message(&writer.sender, state.task.clone(), Some(1), message)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::TurnFinished {
                turn_id: "prepared-a".into(),
                outcome: TurnOutcome::Completed,
                output: "A 原生完成".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let before = state.task.clone();
        assert!(
            send_user_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                new_input,
                RuntimeAction::Submit {
                    input: vec![InputContent::Text("不得覆盖异常关联".into())]
                },
                false
            )
            .await
            .is_err()
        );
        assert_eq!(state.task, before);
        assert!(commands.try_recv().is_err());
        let history = load_task_generations(&writer.sender, state.task.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.len(), 1);
        let messages = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages
                .iter()
                .find(|message| message.message_id == b.to_string())
                .unwrap()
                .state,
            LocalCliMessageState::Sent
        );
        assert_eq!(
            messages
                .iter()
                .find(|message| message.message_id == prepared.to_string())
                .unwrap()
                .state,
            LocalCliMessageState::Cancelled
        );
        assert!(
            messages
                .iter()
                .all(|message| message.message_id != new_input.to_string())
        );
        assert_eq!(grok_input_links(&state.task).unwrap().pending.len(), 2);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_terminal_gap_closed_delivery_preserves_sent_fifo_without_retransmission() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("grok-gap-closed.sqlite");
    let writer = crate::persistence::start_test_writer(&database).unwrap();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    let task_id = block_on(async {
        let token = Uuid::new_v4();
        let mut state = persisted_grok_runtime(&writer.sender, token).await;
        let (controller, mut commands, _sender, _events) = channels(token);
        let a = Uuid::new_v4();
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        submit_grok_input(&writer.sender, &mut state, &controller, token, a).await;
        commands.try_recv().unwrap();
        for kind in [
            RuntimeEventKind::MessageAccepted {
                message_id: a,
                turn_id: Some("closed-a".into()),
            },
            RuntimeEventKind::TurnStarted {
                turn_id: "closed-a".into(),
            },
        ] {
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                kind,
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
        }
        submit_grok_input(&writer.sender, &mut state, &controller, token, b).await;
        commands.try_recv().unwrap();
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::TurnFinished {
                turn_id: "closed-a".into(),
                outcome: TurnOutcome::Cancelled,
                output: "A 原生取消".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        drop(commands);
        assert!(
            send_user_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                c,
                RuntimeAction::Submit {
                    input: vec![InputContent::Text("连接关闭后的输入".into())]
                },
                false
            )
            .await
            .is_err()
        );
        assert_eq!(state.task.generation, 2);
        let links = grok_input_links(&state.task).unwrap();
        assert!(links.current.is_none());
        assert_eq!(links.pending.len(), 1);
        assert_eq!(links.pending[0].message_id, b);
        assert_eq!(links.pending[0].submission_generation, 1);
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::Disconnected {
                reason: "原生连接关闭".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.state, LocalCliTaskState::Disconnected);
        assert!(!state.connected);
        assert!(!state.ready);
        state.task.task_id
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
    let reopened = crate::persistence::start_test_writer(&database).unwrap();
    block_on(async {
        let history = load_task_generations(&reopened.sender, task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].state, LocalCliTaskState::Cancelled);
        assert_eq!(history[0].result.as_deref(), Some("A 原生取消"));
        assert_eq!(history[1].state, LocalCliTaskState::Disconnected);
        let links = grok_input_links(&history[1]).unwrap();
        assert_eq!(links.pending[0].message_id, b);
        assert!(links.pending[0].native_turn_id.is_none());
        for (generation, id, expected) in [
            (1, b, LocalCliMessageState::Sent),
            (2, c, LocalCliMessageState::Failed),
        ] {
            let messages = load_messages(&reopened.sender, task_id.clone(), generation)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            let message = messages
                .iter()
                .find(|message| message.message_id == id.to_string())
                .unwrap();
            assert_eq!(message.recipient_generation, generation);
            assert_eq!(message.state, expected);
            assert_eq!(message.receipt_kind, None);
        }
    });
    reopened.sender.send(ModelEvent::Terminate).unwrap();
    reopened.handle.join().unwrap();
}

#[test]
fn grok_ready_persists_file_mode_and_rejects_a_different_saved_policy() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let profile = super::super::permissions::GrokCreationPolicyV1::compile(
        &cwd,
        "1.0.30",
        "a".repeat(64),
        "b".repeat(64),
        None,
        PermissionPolicy::GrokRestrictedFilesV1,
    )
    .unwrap();
    let mut state = snapshot();
    state.task.harness = "grok".into();
    state.task.working_directory = cwd.to_string_lossy().into_owned();
    state.task.config_json = json!({"permission_policy":"GrokRestrictedFilesV1"}).to_string();
    let ready = event(
        &state,
        RuntimeEventKind::SessionReady {
            verified_cli_version: Some("1.0.30".into()),
            effective_permissions: json!({
                "requestedPolicy":"GrokRestrictedFilesV1","appCreationPolicyApplied":true,
                "permissionEnforcementVerified":false,"grokCreationPolicyV1":profile,
            }),
        },
    );
    apply_runtime_event(&mut state, &ready).unwrap();
    let saved: Value = serde_json::from_str(&state.task.config_json).unwrap();
    assert_eq!(saved["permission_policy"], "GrokRestrictedFilesV1");
    assert_eq!(saved["grok_profile"]["toolSet"], "files");
    assert_eq!(saved["cli_version"], "1.0.30");
    let mut mismatched = snapshot();
    mismatched.task.harness = "grok".into();
    mismatched.task.working_directory = state.task.working_directory;
    mismatched.task.config_json = json!({"permission_policy":"GrokRestrictedReadV1"}).to_string();
    let mut wrong = ready;
    wrong.native_session_id = mismatched.task.native_session_id.clone();
    assert!(apply_runtime_event(&mut mismatched, &wrong).is_err());
    assert_eq!(
        mismatched.task.config_json,
        json!({"permission_policy":"GrokRestrictedReadV1"}).to_string()
    );
}

async fn grok_mailbox_family(
    sender: &SyncSender<ModelEvent>,
    token: Uuid,
    recipient_is_child: bool,
) -> (ManagedTaskSnapshot, LocalCliTask) {
    let mut recipient = snapshot();
    recipient.task.harness = "grok".into();
    recipient.task.config_json = json!({"runtime_generation":token}).to_string();
    let mut source = snapshot().task;
    if recipient_is_child {
        recipient.task.parent_task_id = Some(source.task_id.clone());
        recipient.task.parent_generation = Some(1);
        checkpoint_task(sender, source.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        checkpoint_task(sender, recipient.task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
    } else {
        source.parent_task_id = Some(recipient.task.task_id.clone());
        source.parent_generation = Some(1);
        checkpoint_task(sender, recipient.task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        checkpoint_task(sender, source.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
    }
    (recipient, source)
}

fn grok_mailbox_message(source: &LocalCliTask, recipient: &LocalCliTask) -> LocalCliMessage {
    LocalCliMessage {
        version: 1,
        message_id: Uuid::new_v4().to_string(),
        sender_task_id: source.task_id.clone(),
        recipient_task_id: recipient.task_id.clone(),
        sender_generation: source.generation,
        recipient_generation: recipient.generation,
        subject: "追加指令".into(),
        body: "保留 English 与中文\n不要执行 `$()` 中的文本。".into(),
        state: LocalCliMessageState::Queued,
        receipt_kind: None,
    }
}

async fn dispatch_grok_mailbox(
    sender: &SyncSender<ModelEvent>,
    state: &mut ManagedTaskSnapshot,
    controller: &RuntimeController,
    token: Uuid,
    message: LocalCliMessage,
) -> LocalCliMessageState {
    let (commands, mut receiver) = mpsc::channel(1);
    let endpoint = ManagedTaskEndpoint {
        task_id: state.task.task_id.clone(),
        generation: state.task.generation,
        harness: Harness::Grok,
        runtime_generation: token,
        active_turn_id: state.active_turn_id.clone(),
        commands,
    };
    let send = send_local_message_if_current(sender, endpoint, message, |prepared| async {
        Ok(prepared.commit())
    });
    let dispatch = async {
        let request = receiver.recv().await.unwrap();
        assert!(
            matches!(request.action, RuntimeAction::Submit { .. }),
            "Grok 活跃邮箱必须排队，不能发送 Steer"
        );
        let result = send_mailbox_request(
            sender,
            state,
            controller,
            token,
            request.message_id,
            request.action,
            request.prepared_result,
        )
        .await;
        request.reply.send(result).unwrap();
    };
    let (result, ()) = futures::join!(send, dispatch);
    result.unwrap()
}

#[test]
fn grok_mailbox_parent_and_child_followups_keep_native_receipts_in_the_original_generation() {
    for recipient_is_child in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let writer =
            crate::persistence::start_test_writer(&directory.path().join("mailbox.sqlite"))
                .unwrap();
        block_on(async {
            let token = Uuid::new_v4();
            let (mut state, source) =
                grok_mailbox_family(&writer.sender, token, recipient_is_child).await;
            let (controller, mut commands, _sender, _events) = channels(token);
            let mut accepted = HashSet::new();
            let mut finished = HashSet::new();
            let first = Uuid::new_v4();
            submit_grok_input(&writer.sender, &mut state, &controller, token, first).await;
            commands.try_recv().unwrap();
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::MessageAccepted {
                    message_id: first,
                    turn_id: Some("initial".into()),
                },
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::TurnStarted {
                    turn_id: "initial".into(),
                },
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
            let b = grok_mailbox_message(&source, &state.task);
            let c = grok_mailbox_message(&source, &state.task);
            assert_eq!(
                dispatch_grok_mailbox(&writer.sender, &mut state, &controller, token, b.clone())
                    .await,
                LocalCliMessageState::Sent
            );
            commands.try_recv().unwrap();
            assert_eq!(
                dispatch_grok_mailbox(&writer.sender, &mut state, &controller, token, c.clone())
                    .await,
                LocalCliMessageState::Sent
            );
            commands.try_recv().unwrap();
            let stored = load_tasks(&writer.sender, false)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            let persisted = stored
                .iter()
                .find(|task| task.task_id == state.task.task_id)
                .unwrap();
            assert_eq!(grok_input_links(persisted).unwrap().pending.len(), 2);
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::TurnFinished {
                    turn_id: "initial".into(),
                    outcome: TurnOutcome::Completed,
                    output: "首轮完成".into(),
                },
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
            for (message, turn) in [(&b, "mailbox-b"), (&c, "mailbox-c")] {
                let id = message.message_id.parse().unwrap();
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    RuntimeEventKind::MessageAccepted {
                        message_id: id,
                        turn_id: Some(turn.into()),
                    },
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    RuntimeEventKind::TurnStarted {
                        turn_id: turn.into(),
                    },
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    RuntimeEventKind::TurnFinished {
                        turn_id: turn.into(),
                        outcome: TurnOutcome::Completed,
                        output: "邮箱回合完成".into(),
                    },
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
            }
            assert_eq!(state.task.generation, 3);
            let original = load_messages(&writer.sender, state.task.task_id.clone(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            for expected in [b, c] {
                let saved = original
                    .iter()
                    .find(|message| message.message_id == expected.message_id)
                    .unwrap();
                assert_eq!(saved.recipient_generation, 1);
                assert_eq!(saved.sender_task_id, source.task_id);
                assert_eq!(saved.body, expected.body);
                assert_eq!(saved.state, LocalCliMessageState::Acknowledged);
                assert_eq!(
                    saved.receipt_kind,
                    Some(LocalCliReceiptKind::NativeProtocol)
                );
            }
            assert!(commands.try_recv().is_err());
        });
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    }
}

#[test]
fn grok_mailbox_rejects_changed_payload_old_runtime_and_closed_delivery_without_replay() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("mailbox-failed.sqlite"))
            .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let (mut state, source) = grok_mailbox_family(&writer.sender, token, true).await;
        let message = grok_mailbox_message(&source, &state.task);
        let id = message.message_id.parse().unwrap();
        enqueue_message(&writer.sender, message.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        acknowledge_message(
            &writer.sender,
            message.message_id.clone(),
            state.task.task_id.clone(),
            1,
            LocalCliMessageState::Sent,
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
        let action = RuntimeAction::Submit {
            input: vec![InputContent::Text(format!(
                "Subject: {}\n\n{}",
                message.subject, message.body
            ))],
        };
        let (controller, mut commands, _sender, _events) = channels(token);
        assert!(
            send_mailbox_action(
                &writer.sender,
                &mut state,
                &controller,
                token,
                id,
                RuntimeAction::Submit {
                    input: vec![InputContent::Text("篡改正文".into())]
                }
            )
            .await
            .is_err()
        );
        assert!(
            send_mailbox_action(
                &writer.sender,
                &mut state,
                &controller,
                Uuid::new_v4(),
                id,
                action.clone()
            )
            .await
            .is_err()
        );
        assert!(commands.try_recv().is_err());
        assert!(grok_input_links(&state.task).unwrap().pending.is_empty());
        drop(commands);
        assert!(
            send_mailbox_action(
                &writer.sender,
                &mut state,
                &controller,
                token,
                id,
                action.clone()
            )
            .await
            .is_err()
        );
        assert!(grok_input_links(&state.task).unwrap().pending.is_empty());
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].state, LocalCliMessageState::Failed);
        assert_eq!(saved[0].receipt_kind, None);
        let (replacement, mut commands, _sender, _events) = channels(token);
        assert!(
            send_mailbox_action(&writer.sender, &mut state, &replacement, token, id, action)
                .await
                .is_err()
        );
        assert!(commands.try_recv().is_err());
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

async fn grok_mailbox_result_fixture(
    sender: &SyncSender<ModelEvent>,
    token: Uuid,
    parent_state: LocalCliTaskState,
) -> (ManagedTaskSnapshot, LocalCliMessage) {
    let (mut parent, mut child) = grok_mailbox_family(sender, token, false).await;
    parent.task.state = parent_state;
    parent.task.revision = 1;
    if parent_state.is_terminal() {
        parent.task.result = Some("父任务原回合结果".into());
        parent.task.terminal_evidence = Some("父任务原生终态".into());
    }
    checkpoint_task(sender, parent.task.clone(), Some(1))
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    child.state = LocalCliTaskState::Completed;
    child.revision = 1;
    child.result = Some("子任务正式结果".into());
    child.terminal_evidence = Some("子任务原生完成".into());
    checkpoint_task(sender, child.clone(), Some(1))
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    let message = enqueue_task_result(sender, child.task_id, 1)
        .unwrap()
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    (parent, message)
}

#[test]
fn grok_mailbox_completed_parent_continues_only_after_native_result_ack_and_start() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("mailbox-result.sqlite"))
            .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let (mut state, message) =
            grok_mailbox_result_fixture(&writer.sender, token, LocalCliTaskState::Completed).await;
        let (controller, mut wire, _sender, _events) = channels(token);
        let (commands, mut requests) = mpsc::channel(1);
        let mut coordinator = LocalCLITaskCoordinator::new(None);
        coordinator.entries.insert(
            state.task.task_id.clone(),
            ManagedTaskEntry {
                token,
                snapshot: state.clone(),
                commands,
                pending_tool_calls: HashSet::new(),
            },
        );
        assert!(coordinator.endpoint(&state.task.task_id).is_none());
        let endpoint = coordinator.result_endpoint(&state.task.task_id).unwrap();
        let send = send_prepared_result(endpoint, message.clone());
        let receive = async {
            let request = requests.recv().await.unwrap();
            assert_eq!(request.expected_generation, 1);
            assert!(matches!(request.action, RuntimeAction::Submit { .. }));
            let result = send_mailbox_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                request.message_id,
                request.action,
                request.prepared_result,
            )
            .await;
            request.reply.send(result).unwrap();
        };
        let (result, ()) = futures::join!(send, receive);
        result.unwrap();
        let dispatched = wire.try_recv().unwrap();
        assert_eq!(dispatched.message_id.to_string(), message.message_id);
        assert_eq!(state.task.generation, 1);
        assert_eq!(state.task.state, LocalCliTaskState::Completed);
        // 已领取结果的重复回调不会二次派发，也不占另一个队列槽。
        send_mailbox_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            dispatched.message_id,
            dispatched.action.clone(),
            Some(message.clone()),
        )
        .await
        .unwrap();
        assert!(wire.try_recv().is_err());
        assert_eq!(grok_input_links(&state.task).unwrap().pending.len(), 1);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::MessageAccepted {
                message_id: dispatched.message_id,
                turn_id: Some("automatic-result".into()),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.generation, 1);
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::TurnStarted {
                turn_id: "automatic-result".into(),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.generation, 2);
        assert_eq!(state.task.state, LocalCliTaskState::Running);
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].body, message.body);
        assert_eq!(saved[0].sender_generation, 1);
        assert_eq!(saved[0].recipient_generation, 1);
        assert_eq!(
            saved[0].receipt_kind,
            Some(LocalCliReceiptKind::NativeProtocol)
        );
        send_mailbox_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            dispatched.message_id,
            dispatched.action,
            Some(message),
        )
        .await
        .unwrap();
        assert!(wire.try_recv().is_err());
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_mailbox_cancelled_or_failed_parent_keeps_result_queued_without_native_send() {
    for parent_state in [LocalCliTaskState::Cancelled, LocalCliTaskState::Failed] {
        let directory = tempfile::tempdir().unwrap();
        let writer =
            crate::persistence::start_test_writer(&directory.path().join("mailbox-stopped.sqlite"))
                .unwrap();
        block_on(async {
            let token = Uuid::new_v4();
            let (mut state, message) =
                grok_mailbox_result_fixture(&writer.sender, token, parent_state).await;
            let (controller, mut wire, _sender, _events) = channels(token);
            let (commands, _requests) = mpsc::channel(1);
            let mut coordinator = LocalCLITaskCoordinator::new(None);
            coordinator.entries.insert(
                state.task.task_id.clone(),
                ManagedTaskEntry {
                    token,
                    snapshot: state.clone(),
                    commands,
                    pending_tool_calls: HashSet::new(),
                },
            );
            assert!(coordinator.result_endpoint(&state.task.task_id).is_none());
            let action = RuntimeAction::Submit {
                input: vec![InputContent::Text(format!(
                    "Subject: {}\n\n{}",
                    message.subject, message.body
                ))],
            };
            send_mailbox_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                message.message_id.parse().unwrap(),
                action,
                Some(message.clone()),
            )
            .await
            .unwrap();
            assert!(wire.try_recv().is_err());
            let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved, vec![message]);
            assert_eq!(state.task.state, parent_state);
            assert!(grok_input_links(&state.task).unwrap().pending.is_empty());
        });
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    }
}

#[test]
fn grok_mailbox_later_parent_generation_can_address_its_original_child() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("mailbox-parent-next.sqlite"))
            .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let (mut state, mut source) = grok_mailbox_family(&writer.sender, token, true).await;
        source.state = LocalCliTaskState::Completed;
        source.revision = 1;
        source.result = Some("父首轮完成".into());
        source.terminal_evidence = Some("原生完成".into());
        checkpoint_task(&writer.sender, source.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        source = next_task_generation(&source).unwrap();
        checkpoint_task(&writer.sender, source.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let message = grok_mailbox_message(&source, &state.task);
        let (controller, mut wire, _sender, _events) = channels(token);
        assert_eq!(
            dispatch_grok_mailbox(
                &writer.sender,
                &mut state,
                &controller,
                token,
                message.clone()
            )
            .await,
            LocalCliMessageState::Sent
        );
        let dispatched = wire.try_recv().unwrap();
        assert_eq!(state.task.parent_generation, Some(1));
        assert_eq!(message.sender_generation, 2);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::MessageAccepted {
                message_id: dispatched.message_id,
                turn_id: Some("later-parent-input".into()),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved[0].sender_generation, 2);
        assert_eq!(saved[0].recipient_generation, 1);
        assert_eq!(
            saved[0].receipt_kind,
            Some(LocalCliReceiptKind::NativeProtocol)
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_mailbox_active_parent_keeps_automatic_result_queued_through_failure_or_cancel() {
    for outcome in [
        TurnOutcome::Failed {
            message: "原生失败".into(),
        },
        TurnOutcome::Cancelled,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let writer = crate::persistence::start_test_writer(
            &directory.path().join("mailbox-active-result.sqlite"),
        )
        .unwrap();
        block_on(async {
            let token = Uuid::new_v4();
            let (mut state, message) =
                grok_mailbox_result_fixture(&writer.sender, token, LocalCliTaskState::Running)
                    .await;
            state.active_turn_id = Some("parent-work".into());
            let (controller, mut wire, _sender, _events) = channels(token);
            let action = RuntimeAction::Submit {
                input: vec![InputContent::Text(format!(
                    "Subject: {}\n\n{}",
                    message.subject, message.body
                ))],
            };
            send_mailbox_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                message.message_id.parse().unwrap(),
                action.clone(),
                Some(message.clone()),
            )
            .await
            .unwrap();
            assert!(
                wire.try_recv().is_err(),
                "运行中的自动结果不能预先进入原生队列"
            );
            let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved, vec![message.clone()]);
            let mut accepted = HashSet::new();
            let mut finished = HashSet::new();
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::TurnFinished {
                    turn_id: "parent-work".into(),
                    outcome,
                    output: "原生非成功终态".into(),
                },
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
            send_mailbox_request(
                &writer.sender,
                &mut state,
                &controller,
                token,
                message.message_id.parse().unwrap(),
                action,
                Some(message.clone()),
            )
            .await
            .unwrap();
            assert!(wire.try_recv().is_err());
            let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved, vec![message]);
            assert!(grok_input_links(&state.task).unwrap().pending.is_empty());
        });
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    }
}

#[test]
fn grok_mailbox_multiple_results_never_enter_native_queue_together_before_start() {
    let directory = tempfile::tempdir().unwrap();
    let writer = crate::persistence::start_test_writer(
        &directory.path().join("mailbox-single-result.sqlite"),
    )
    .unwrap();
    block_on(async {
        let token = Uuid::new_v4();
        let (mut state, first) =
            grok_mailbox_result_fixture(&writer.sender, token, LocalCliTaskState::Completed).await;
        let source = load_tasks(&writer.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .into_iter()
            .find(|task| task.task_id == first.sender_task_id)
            .unwrap();
        let mut next = next_task_generation(&source).unwrap();
        checkpoint_task(&writer.sender, next.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        next.state = LocalCliTaskState::Completed;
        next.revision = 1;
        next.result = Some("第二份子任务结果".into());
        next.terminal_evidence = Some("第二份原生完成".into());
        checkpoint_task(&writer.sender, next.clone(), Some(2))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let second = enqueue_task_result(&writer.sender, next.task_id, 2)
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let (controller, mut wire, _sender, _events) = channels(token);
        let first_action = RuntimeAction::Submit {
            input: vec![InputContent::Text(format!(
                "Subject: {}\n\n{}",
                first.subject, first.body
            ))],
        };
        let second_action = RuntimeAction::Submit {
            input: vec![InputContent::Text(format!(
                "Subject: {}\n\n{}",
                second.subject, second.body
            ))],
        };
        send_mailbox_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            first.message_id.parse().unwrap(),
            first_action,
            Some(first.clone()),
        )
        .await
        .unwrap();
        let dispatched = wire.try_recv().unwrap();
        send_mailbox_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            second.message_id.parse().unwrap(),
            second_action.clone(),
            Some(second.clone()),
        )
        .await
        .unwrap();
        assert!(wire.try_recv().is_err());
        assert_eq!(grok_input_links(&state.task).unwrap().pending.len(), 1);
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        commit_grok_kind(
            &writer.sender,
            &mut state,
            token,
            RuntimeEventKind::MessageAccepted {
                message_id: dispatched.message_id,
                turn_id: Some("only-automatic-result".into()),
            },
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        assert_eq!(state.task.generation, 1);
        send_mailbox_request(
            &writer.sender,
            &mut state,
            &controller,
            token,
            second.message_id.parse().unwrap(),
            second_action,
            Some(second.clone()),
        )
        .await
        .unwrap();
        assert!(
            wire.try_recv().is_err(),
            "ACK 不代表下一轮已开始，更不能领取第二个结果"
        );
        let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved
                .iter()
                .find(|message| message.message_id == second.message_id),
            Some(&second)
        );
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn grok_mailbox_original_results_follow_progress_across_completed_generations_only() {
    for result_outcome in [
        TurnOutcome::Completed,
        TurnOutcome::Cancelled,
        TurnOutcome::Failed {
            message: "结果回合失败".into(),
        },
    ] {
        let directory = tempfile::tempdir().unwrap();
        let writer = crate::persistence::start_test_writer(
            &directory.path().join("mailbox-progress-results.sqlite"),
        )
        .unwrap();
        block_on(async {
            let token = Uuid::new_v4();
            let (mut state, first) =
                grok_mailbox_result_fixture(&writer.sender, token, LocalCliTaskState::Running)
                    .await;
            state.active_turn_id = Some("parent-initial".into());
            let child = load_tasks(&writer.sender, false)
                .unwrap()
                .await
                .unwrap()
                .unwrap()
                .into_iter()
                .find(|task| task.task_id == first.sender_task_id)
                .unwrap();
            let mut child = next_task_generation(&child).unwrap();
            checkpoint_task(&writer.sender, child.clone(), Some(1))
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            child.state = LocalCliTaskState::Completed;
            child.revision = 1;
            child.result = Some("子任务第二代完整结果".into());
            child.terminal_evidence = Some("子任务第二代原生完成".into());
            checkpoint_task(&writer.sender, child.clone(), Some(2))
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            let second = enqueue_task_result(&writer.sender, child.task_id.clone(), 2)
                .unwrap()
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let (controller, mut wire, _sender, _events) = channels(token);
            let progress = grok_mailbox_message(&child, &state.task);
            dispatch_grok_mailbox(
                &writer.sender,
                &mut state,
                &controller,
                token,
                progress.clone(),
            )
            .await;
            wire.try_recv().unwrap();
            for result in [&first, &second] {
                let action = RuntimeAction::Submit {
                    input: vec![InputContent::Text(format!(
                        "Subject: {}\n\n{}",
                        result.subject, result.body
                    ))],
                };
                send_mailbox_request(
                    &writer.sender,
                    &mut state,
                    &controller,
                    token,
                    result.message_id.parse().unwrap(),
                    action,
                    Some(result.clone()),
                )
                .await
                .unwrap();
                assert!(wire.try_recv().is_err());
            }
            let mut accepted = HashSet::new();
            let mut finished = HashSet::new();
            commit_grok_kind(
                &writer.sender,
                &mut state,
                token,
                RuntimeEventKind::TurnFinished {
                    turn_id: "parent-initial".into(),
                    outcome: TurnOutcome::Completed,
                    output: "父首轮完成".into(),
                },
                &mut accepted,
                &mut finished,
            )
            .await
            .unwrap();
            assert!(
                !grok_result_slot_available(&state).unwrap(),
                "普通进度输入占槽时不能领取结果"
            );
            for kind in [
                RuntimeEventKind::MessageAccepted {
                    message_id: progress.message_id.parse().unwrap(),
                    turn_id: Some("progress".into()),
                },
                RuntimeEventKind::TurnStarted {
                    turn_id: "progress".into(),
                },
                RuntimeEventKind::TurnFinished {
                    turn_id: "progress".into(),
                    outcome: TurnOutcome::Completed,
                    output: "进度回合完成".into(),
                },
            ] {
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    kind,
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
            }
            assert_eq!(state.task.generation, 2);
            let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
                .unwrap()
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                saved
                    .iter()
                    .find(|message| message.message_id == first.message_id),
                Some(&first)
            );
            assert_eq!(
                saved
                    .iter()
                    .find(|message| message.message_id == second.message_id),
                Some(&second)
            );
            for (index, result) in [&first, &second].into_iter().enumerate() {
                let (commands, mut requests) = mpsc::channel(1);
                let endpoint = ManagedTaskEndpoint {
                    task_id: state.task.task_id.clone(),
                    generation: state.task.generation,
                    harness: Harness::Grok,
                    runtime_generation: token,
                    active_turn_id: None,
                    commands,
                };
                assert!(
                    send_local_message_if_current(
                        &writer.sender,
                        endpoint.clone(),
                        result.clone(),
                        |prepared| async { Ok(prepared.commit()) }
                    )
                    .await
                    .is_err(),
                    "普通邮箱路径仍拒绝跨代结果"
                );
                let send = send_prepared_result(endpoint, result.clone());
                let dispatch = async {
                    let request = requests.recv().await.unwrap();
                    assert_eq!(request.expected_generation, state.task.generation);
                    let delivered = send_mailbox_request(
                        &writer.sender,
                        &mut state,
                        &controller,
                        token,
                        request.message_id,
                        request.action,
                        request.prepared_result,
                    )
                    .await;
                    request.reply.send(delivered).unwrap();
                };
                let (delivered, ()) = futures::join!(send, dispatch);
                delivered.unwrap();
                if index == 1 && result_outcome != TurnOutcome::Completed {
                    assert!(
                        wire.try_recv().is_err(),
                        "失败或取消后的剩余结果不能自动启动"
                    );
                    let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
                        .unwrap()
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        saved
                            .iter()
                            .find(|message| message.message_id == second.message_id),
                        Some(&second)
                    );
                    break;
                }
                let command = wire.try_recv().unwrap();
                assert_eq!(command.message_id.to_string(), result.message_id);
                assert_eq!(
                    grok_input_links(&state.task).unwrap().pending[0].submission_generation,
                    1
                );
                let generation = state.task.generation;
                let turn = format!("automatic-result-{index}");
                commit_grok_kind(
                    &writer.sender,
                    &mut state,
                    token,
                    RuntimeEventKind::MessageAccepted {
                        message_id: command.message_id,
                        turn_id: Some(turn.clone()),
                    },
                    &mut accepted,
                    &mut finished,
                )
                .await
                .unwrap();
                assert_eq!(
                    state.task.generation, generation,
                    "只有真实 started 才能增代"
                );
                let saved = load_messages(&writer.sender, state.task.task_id.clone(), 1)
                    .unwrap()
                    .await
                    .unwrap()
                    .unwrap();
                let receipt = saved
                    .iter()
                    .find(|message| message.message_id == result.message_id)
                    .unwrap();
                assert_eq!(receipt.recipient_generation, 1);
                assert_eq!(receipt.sender_generation, result.sender_generation);
                assert_eq!(receipt.body, result.body);
                assert_eq!(
                    receipt.receipt_kind,
                    Some(LocalCliReceiptKind::NativeProtocol)
                );
                for kind in [
                    RuntimeEventKind::TurnStarted {
                        turn_id: turn.clone(),
                    },
                    RuntimeEventKind::TurnFinished {
                        turn_id: turn,
                        outcome: result_outcome.clone(),
                        output: "结果回合真实终态".into(),
                    },
                ] {
                    commit_grok_kind(
                        &writer.sender,
                        &mut state,
                        token,
                        kind,
                        &mut accepted,
                        &mut finished,
                    )
                    .await
                    .unwrap();
                }
            }
            assert_eq!(
                state.task.generation,
                if result_outcome == TurnOutcome::Completed {
                    4
                } else {
                    3
                }
            );
        });
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    }
}

fn assert_ready_version_is_persisted(harness: &str, version: &str) {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("version.sqlite")).unwrap();
    block_on(async {
        let mut state = snapshot();
        state.ready = false;
        state.task.harness = harness.into();
        checkpoint_task(&writer.sender, state.task.clone(), None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let ready = event(
            &state,
            RuntimeEventKind::SessionReady {
                verified_cli_version: Some(version.into()),
                effective_permissions: json!({}),
            },
        );
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &ready,
            &mut HashSet::new(),
            &mut HashSet::new(),
        )
        .await
        .unwrap();
        let saved = load_tasks(&writer.sender, false)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved, [state.task.clone()]);
        let config: Value = serde_json::from_str(&saved[0].config_json).unwrap();
        assert_eq!(config["cli_version"], version);
        assert_eq!(
            config["cli_version_runtime_generation"],
            json!(ready.generation)
        );
        assert_eq!(saved[0].native_session_id, ready.native_session_id);
        assert!(state.ready);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}

#[test]
fn codex_latest_paired_version_is_persisted() {
    assert_ready_version_is_persisted("codex", "0.155.1");
}

#[test]
fn claude_latest_paired_version_is_persisted() {
    assert_ready_version_is_persisted("claude", "2.1.278");
}

#[test]
fn grok_latest_paired_version_is_persisted() {
    assert_ready_version_is_persisted("grok", "1.0.34");
}

#[test]
fn associated_ready_without_version_cannot_become_ready() {
    for harness in ["codex", "claude", "grok"] {
        let mut state = snapshot();
        state.task.harness = harness.into();
        state.ready = false;
        let before = state.task.clone();
        let ready = event(
            &state,
            RuntimeEventKind::SessionReady {
                verified_cli_version: None,
                effective_permissions: json!({}),
            },
        );
        assert!(apply_runtime_event(&mut state, &ready).is_err());
        assert!(!state.ready);
        assert_eq!(state.task, before);
    }
}

#[test]
fn empty_paired_version_cannot_become_ready() {
    let mut state = snapshot();
    state.ready = false;
    let ready = event(
        &state,
        RuntimeEventKind::SessionReady {
            verified_cli_version: Some(String::new()),
            effective_permissions: json!({}),
        },
    );
    assert!(apply_runtime_event(&mut state, &ready).is_err());
    assert!(!state.ready);
    assert_eq!(state.task.config_json, "{}");
}

#[test]
fn same_runtime_cannot_replace_its_paired_version() {
    let mut state = snapshot();
    let mut ready = event(
        &state,
        RuntimeEventKind::SessionReady {
            verified_cli_version: Some("0.147.0".into()),
            effective_permissions: json!({}),
        },
    );
    apply_runtime_event(&mut state, &ready).unwrap();
    let before = state.task.clone();
    ready.kind = RuntimeEventKind::SessionReady {
        verified_cli_version: Some("0.155.1".into()),
        effective_permissions: json!({}),
    };
    assert!(apply_runtime_event(&mut state, &ready).is_err());
    assert_eq!(state.task, before);
}

#[test]
fn claude_initial_control_ready_does_not_invent_a_version() {
    let mut state = snapshot();
    state.task.harness = "claude".into();
    state.task.native_session_id = None;
    state.ready = false;
    let ready = event(
        &state,
        RuntimeEventKind::SessionReady {
            verified_cli_version: None,
            effective_permissions: json!({}),
        },
    );
    apply_runtime_event(&mut state, &ready).unwrap();
    let config: Value = serde_json::from_str(&state.task.config_json).unwrap();
    assert!(state.ready);
    assert!(config.get("cli_version").is_none());
    assert!(config.get("cli_version_runtime_generation").is_none());
}

#[test]
fn claude_paired_version_does_not_restart_a_running_or_terminal_turn() {
    for phase in [
        LocalCliTaskState::Running,
        LocalCliTaskState::Completed,
        LocalCliTaskState::Cancelled,
        LocalCliTaskState::Failed,
    ] {
        let mut state = snapshot();
        state.task.harness = "claude".into();
        state.task.state = phase;
        state.task.result = Some("保留原回合结果".into());
        state.task.terminal_evidence = Some("保留原生证据".into());
        state.active_turn_id = Some("original-turn".into());
        state.output = "保留输出".into();
        let before = state.task.clone();
        let ready = event(
            &state,
            RuntimeEventKind::SessionReady {
                verified_cli_version: Some("2.1.278".into()),
                effective_permissions: json!({}),
            },
        );
        apply_runtime_event(&mut state, &ready).unwrap();
        apply_runtime_event(&mut state, &ready).unwrap();
        assert_eq!(state.task.state, before.state);
        assert_eq!(state.task.result, before.result);
        assert_eq!(state.task.terminal_evidence, before.terminal_evidence);
        assert_eq!(state.task.generation, before.generation);
        assert_eq!(state.active_turn_id.as_deref(), Some("original-turn"));
        assert_eq!(state.output, "保留输出");
    }
}

#[test]
fn resumed_claude_keeps_historical_version_until_the_new_runtime_pairs() {
    let directory = tempfile::tempdir().unwrap();
    let writer =
        crate::persistence::start_test_writer(&directory.path().join("resume-version.sqlite"))
            .unwrap();
    block_on(async {
        let old_runtime = Uuid::from_u128(42);
        let new_runtime = Uuid::from_u128(43);
        let mut previous = snapshot().task;
        previous.harness = "claude".into();
        previous.state = LocalCliTaskState::Completed;
        previous.result = Some("历史结果".into());
        previous.terminal_evidence = Some("历史原生完成证据".into());
        previous.config_json = json!({"cli_version":"2.1.273",
            "cli_version_runtime_generation":old_runtime,"runtime_generation":old_runtime})
        .to_string();
        let mut queued = previous.clone();
        queued.state = LocalCliTaskState::Queued;
        queued.result = None;
        queued.terminal_evidence = None;
        checkpoint_task(&writer.sender, queued, None)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        previous.revision = 1;
        checkpoint_task(&writer.sender, previous.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let mut state = snapshot();
        state.task = next_task_generation(&previous).unwrap();
        state.ready = false;
        let mut config: Value = serde_json::from_str(&state.task.config_json).unwrap();
        config["runtime_generation"] = json!(new_runtime);
        state.task.config_json = config.to_string();
        checkpoint_task(&writer.sender, state.task.clone(), Some(1))
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let mut ready = RuntimeEvent {
            generation: new_runtime,
            native_session_id: None,
            kind: RuntimeEventKind::SessionReady {
                verified_cli_version: None,
                effective_permissions: json!({}),
            },
        };
        let mut accepted = HashSet::new();
        let mut finished = HashSet::new();
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &ready,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let interim: Value = serde_json::from_str(&state.task.config_json).unwrap();
        assert_eq!(interim["cli_version"], "2.1.273");
        assert_eq!(
            interim["cli_version_runtime_generation"],
            json!(old_runtime)
        );
        ready.native_session_id = previous.native_session_id.clone();
        ready.kind = RuntimeEventKind::SessionReady {
            verified_cli_version: Some("2.1.278".into()),
            effective_permissions: json!({}),
        };
        commit_runtime_event(
            &writer.sender,
            &mut state,
            &ready,
            &mut accepted,
            &mut finished,
        )
        .await
        .unwrap();
        let history = load_task_generations(&writer.sender, previous.task_id.clone())
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0], previous);
        assert_eq!(history[1], state.task);
        let current: Value = serde_json::from_str(&history[1].config_json).unwrap();
        assert_eq!(current["cli_version"], "2.1.278");
        assert_eq!(
            current["cli_version_runtime_generation"],
            json!(new_runtime)
        );
        assert_eq!(history[0].native_session_id, history[1].native_session_id);
    });
    writer.sender.send(ModelEvent::Terminate).unwrap();
    writer.handle.join().unwrap();
}
