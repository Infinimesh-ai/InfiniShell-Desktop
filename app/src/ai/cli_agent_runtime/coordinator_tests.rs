use futures::executor::block_on;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::ai::cli_agent_runtime::{InputContent, channels};
use crate::persistence::local_cli_tasks::load_task_generations;

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
