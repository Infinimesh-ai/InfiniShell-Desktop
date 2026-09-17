use std::time::Duration;

use futures::executor::block_on;
use serde_json::json;
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
