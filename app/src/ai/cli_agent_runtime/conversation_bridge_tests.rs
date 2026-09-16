use ::settings::Setting;
use warpui::App;

use super::*;
use crate::global_resource_handles::GlobalResourceHandlesProvider;
use crate::persistence::{ModelEvent, read_test_agent_conversation, start_test_writer};
use crate::terminal::general_settings::GeneralSettings;
use crate::test_util::ai_agent_tasks::create_api_task;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

#[test]
fn restored_completion_requires_native_identity_and_evidence_for_success() {
    let mut task = LocalCliTask {
        version: 1,
        task_id: "task".to_owned(),
        parent_task_id: None,
        parent_generation: None,
        harness: "codex".to_owned(),
        working_directory: "/project".to_owned(),
        config_json: "{}".to_owned(),
        native_session_id: None,
        generation: 1,
        revision: 2,
        state: LocalCliTaskState::Completed,
        result: Some("看起来已经完成".to_owned()),
        terminal_evidence: None,
    };
    assert!(matches!(
        status_for_task(&task).0,
        ConversationStatus::Blocked { .. }
    ));
    task.native_session_id = Some("native".to_owned());
    assert!(matches!(
        status_for_task(&task).0,
        ConversationStatus::Blocked { .. }
    ));
    task.terminal_evidence = Some("native:turn/completed".to_owned());
    assert_eq!(status_for_task(&task).0, ConversationStatus::Success);
    task.state = LocalCliTaskState::Disconnected;
    assert!(matches!(
        status_for_task(&task).0,
        ConversationStatus::Blocked { .. }
    ));
    // 即使记录还保留旧结果证据，待确认状态也不能显示成功或冒称连接中断。
    task.state = LocalCliTaskState::Unconfirmed;
    assert_eq!(
        status_for_task(&task),
        (
            ConversationStatus::Blocked {
                blocked_action: crate::t!("cli-agent-task-outcome-unconfirmed"),
            },
            None,
        )
    );
    task.state = LocalCliTaskState::Cancelled;
    assert_eq!(status_for_task(&task).0, ConversationStatus::Cancelled);
}

#[test]
fn local_parent_turn_identity_excludes_provider_results_and_internal_summaries() {
    assert!(input_starts_user_turn(&AIAgentInput::ResumeConversation {
        context: Vec::new().into()
    }));
    assert!(!input_starts_user_turn(
        &AIAgentInput::MessagesReceivedFromAgents {
            messages: Vec::new()
        }
    ));
    assert!(!input_starts_user_turn(&AIAgentInput::EventsFromAgents {
        events: Vec::new()
    }));
    assert!(!input_starts_user_turn(
        &AIAgentInput::SummarizeConversation {
            prompt: None,
            overflow: true,
            context: Vec::new().into(),
        }
    ));
    assert!(history_can_receive(&ConversationStatus::InProgress));
    assert!(history_can_receive(&ConversationStatus::WaitingForEvents));
    assert!(!history_can_receive(&ConversationStatus::Success));
    assert!(!history_can_receive(&ConversationStatus::Cancelled));
    assert!(!history_can_receive(&ConversationStatus::TransientError));
}

#[test]
fn local_result_history_message_keeps_original_identity_and_content() {
    let result = LocalCliMessage {
        version: 1,
        message_id: "stable-result-id".to_owned(),
        sender_task_id: "child".to_owned(),
        recipient_task_id: "parent".to_owned(),
        sender_generation: 2,
        recipient_generation: 3,
        subject: "local_task_result".to_owned(),
        body: "中文结果\nEnglish result".to_owned(),
        state: LocalCliMessageState::Sent,
        receipt_kind: None,
    };
    let saved = history_result_message("root-task", &result);
    assert_eq!(saved.id, result.message_id);
    assert_eq!(saved.task_id, "root-task");
    let Some(Message::MessagesReceivedFromAgents(received)) = saved.message else {
        panic!("结果必须使用已有父会话消息类型");
    };
    assert_eq!(received.messages.len(), 1);
    assert_eq!(received.messages[0].message_id, result.message_id);
    assert_eq!(received.messages[0].sender_agent_id, "child");
    assert_eq!(received.messages[0].addresses, ["parent"]);
    assert_eq!(received.messages[0].message_body, result.body);
}

#[test]
fn local_result_source_history_checkpoint_persists_the_actual_message_before_receipt() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("result-history.sqlite");
        let writer = start_test_writer(&database).unwrap();
        GlobalResourceHandlesProvider::handle(&app).update(&mut app, |resources, _| {
            resources.set_model_event_sender_for_test(writer.sender.clone());
        });
        GeneralSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.persist_conversations.set_value(true, ctx).unwrap();
        });
        let terminal = add_window_with_terminal(&mut app, None);
        let expected = LocalCliMessage {
            version: 1,
            message_id: "durable-result-id".to_owned(),
            sender_task_id: "child".to_owned(),
            recipient_task_id: "parent".to_owned(),
            sender_generation: 1,
            recipient_generation: 1,
            subject: "local_task_result".to_owned(),
            body: "必须存在于真实父历史中的中文结果".to_owned(),
            state: LocalCliMessageState::Sent,
            receipt_kind: None,
        };
        let expected_history = history_result_message("root-task", &expected);
        let (conversation_id, checkpoint) = terminal.update(&mut app, |terminal, ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let id = history.start_new_conversation(terminal.id(), false, false, false, ctx);
                history
                    .conversation_mut(&id)
                    .unwrap()
                    .upgrade_optimistic_root_to_server_task_for_test(create_api_task(
                        "root-task",
                        Vec::new(),
                    ));
                let task_id = history
                    .conversation(&id)
                    .unwrap()
                    .get_root_task_id()
                    .clone();
                assert_eq!(
                    history
                        .append_byop_preflight_messages_to_task(
                            id,
                            task_id,
                            vec![expected_history.clone()],
                            ctx
                        )
                        .unwrap(),
                    1
                );
                let checkpoint = history
                    .conversation(&id)
                    .unwrap()
                    .checkpoint_compaction_recovery_state(ctx)
                    .unwrap()
                    .unwrap();
                (id, checkpoint)
            })
        });
        assert_eq!(checkpoint.await.unwrap(), Ok(()));
        // 使用另一条只读 SQLite 连接，不能用内存 task store 充当落盘证明。
        let stored = read_test_agent_conversation(&database, &conversation_id.to_string())
            .unwrap()
            .unwrap();
        let root = stored
            .tasks
            .iter()
            .find(|task| task.id == "root-task")
            .unwrap();
        assert_eq!(root.messages, [expected_history]);
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    });
}
