use std::path::Path;
use std::sync::mpsc::SyncSender;
use std::time::Duration;

use ::settings::Setting;
use futures::FutureExt;
use futures::channel::oneshot;
use uuid::Uuid;
use warpui::r#async::Timer;
use warpui::{AddSingletonModel, App};

use super::*;
use crate::ai::agent::{AIAgentExchange, AIAgentExchangeId, AIAgentOutputStatus};
use crate::ai::llms::LLMId;
use crate::global_resource_handles::GlobalResourceHandlesProvider;
use crate::persistence::local_cli_tasks::{enqueue_task_result, load_messages};
use crate::persistence::model::LocalCliReceiptKind;
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

fn explicit_parent_exchange() -> AIAgentExchange {
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

async fn committed<T>(receiver: oneshot::Receiver<Result<T, String>>) -> T {
    futures::select! {
        result = receiver.fuse() => result.expect("私有 SQLite 提交通道已关闭").unwrap(),
        _ = Timer::after(Duration::from_secs(5)).fuse() => panic!("私有 SQLite 提交超时"),
    }
}

async fn prepare_oz_result(
    app: &mut App,
    database: &Path,
    sender: &SyncSender<ModelEvent>,
) -> (
    AIConversationId,
    LocalCliTask,
    LocalCliTask,
    LocalCliMessage,
) {
    initialize_app_for_terminal_view(app);
    GlobalResourceHandlesProvider::handle(app).update(app, |resources, _| {
        resources.set_model_event_sender_for_test(sender.clone());
    });
    GeneralSettings::handle(app).update(app, |settings, ctx| {
        settings.persist_conversations.set_value(true, ctx).unwrap();
    });
    app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(sender.clone())));
    app.add_singleton_model(LocalCLIConversationBridge::new);
    let terminal = add_window_with_terminal(app, None);
    let parent_id = Uuid::new_v4().to_string();
    let root_task_id = Uuid::new_v4().to_string();
    let (conversation_id, identity, checkpoint) = terminal.update(app, |terminal, ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let id = history.start_new_conversation(terminal.id(), false, false, false, ctx);
            let conversation = history.conversation_mut(&id).unwrap();
            conversation.upgrade_optimistic_root_to_server_task_for_test(create_api_task(
                &root_task_id,
                Vec::new(),
            ));
            conversation.set_orchestration_harness(Harness::Oz);
            conversation.append_root_exchange_for_test(explicit_parent_exchange());
            // 使用生产登记入口，让结果接收方可以通过父运行 ID 找到真实会话。
            history.assign_run_id_for_conversation(id, parent_id.clone(), None, terminal.id(), ctx);
            assert_eq!(history.conversation_id_for_agent_id(&parent_id), Some(id));
            let identity =
                local_parent_history_identity(history.conversation(&id).unwrap()).unwrap();
            let checkpoint = history
                .conversation(&id)
                .unwrap()
                .checkpoint_compaction_recovery_state(ctx)
                .unwrap()
                .unwrap();
            (id, identity, checkpoint)
        })
    });
    committed(checkpoint).await;
    let parent = LocalCliTask {
        version: 1,
        task_id: parent_id,
        parent_task_id: None,
        parent_generation: None,
        harness: "oz".into(),
        working_directory: database.parent().unwrap().to_string_lossy().into_owned(),
        config_json: serde_json::json!({
            "execution_kind": "local_parent", "history_identity": identity,
        })
        .to_string(),
        native_session_id: None,
        generation: 1,
        revision: 0,
        state: LocalCliTaskState::Queued,
        result: None,
        terminal_evidence: None,
    };
    committed(checkpoint_task(sender, parent.clone(), None).unwrap()).await;
    let mut child = LocalCliTask {
        task_id: Uuid::new_v4().to_string(),
        parent_task_id: Some(parent.task_id.clone()),
        parent_generation: Some(parent.generation),
        harness: "codex".into(),
        config_json: "{}".into(),
        native_session_id: Some(Uuid::new_v4().to_string()),
        ..parent.clone()
    };
    committed(checkpoint_task(sender, child.clone(), None).unwrap()).await;
    // 这里是已提交终态的事件夹具，不派生 CLI 或请求模型。
    child.state = LocalCliTaskState::Completed;
    child.revision += 1;
    child.result = Some("父历史中的中文结果\nEnglish result 🧪".into());
    child.terminal_evidence = Some("fixture:turn/completed".into());
    committed(checkpoint_task(sender, child.clone(), Some(1)).unwrap()).await;
    let message =
        committed(enqueue_task_result(sender, child.task_id.clone(), child.generation).unwrap())
            .await
            .unwrap();
    assert_eq!(message.state, LocalCliMessageState::Queued);
    assert_eq!(message.receipt_kind, None);
    (conversation_id, parent, child, message)
}

fn emit_result_ready(app: &mut App, child: &LocalCliTask, message: &LocalCliMessage) {
    LocalCLITaskCoordinator::handle(app).update(app, |_, ctx| {
        // 走模型订阅，不直接调用桥或历史写入函数。
        ctx.emit(LocalCLITaskCoordinatorEvent::ResultReady {
            task: child.clone(),
            message: message.clone(),
        });
    });
}

async fn wait_for_result_bridge(app: &App, message_id: &str) -> Option<String> {
    for _ in 0..250 {
        Timer::after(Duration::from_millis(10)).await;
        let (pending, error) = LocalCLIConversationBridge::handle(app).read(app, |bridge, _| {
            (
                bridge.pending_results.contains_key(message_id),
                bridge.delivery_error(message_id).map(str::to_owned),
            )
        });
        if !pending {
            return error;
        }
    }
    panic!("父结果桥未在期限内完成：{message_id}");
}

fn saved_root_messages(database: &Path, conversation_id: AIConversationId) -> Vec<api::Message> {
    let saved = read_test_agent_conversation(database, &conversation_id.to_string())
        .unwrap()
        .unwrap();
    saved
        .tasks
        .into_iter()
        .filter(|task| task.dependencies.is_none())
        .flat_map(|task| task.messages)
        .collect()
}

#[test]
fn oz_result_ready_commits_history_once_and_records_application_receipt() {
    App::test((), |mut app| async move {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("oz-result.sqlite");
        let writer = start_test_writer(&database).unwrap();
        let (conversation_id, parent, child, message) =
            prepare_oz_result(&mut app, &database, &writer.sender).await;
        assert!(saved_root_messages(&database, conversation_id).is_empty());
        emit_result_ready(&mut app, &child, &message);
        emit_result_ready(&mut app, &child, &message);
        assert_eq!(
            wait_for_result_bridge(&app, &message.message_id).await,
            None
        );
        let stored = committed(
            load_messages(&writer.sender, parent.task_id.clone(), parent.generation).unwrap(),
        )
        .await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].state, LocalCliMessageState::Acknowledged);
        assert_eq!(
            stored[0].receipt_kind,
            Some(LocalCliReceiptKind::ApplicationHistory)
        );
        let root_id = recorded_history_identity(&parent).unwrap().root_task_id;
        let expected = history_result_message(&root_id, &message);
        // 回执来自生产桥；另开只读连接确认真实历史已经落盘。
        assert_eq!(
            saved_root_messages(&database, conversation_id),
            [expected.clone()]
        );

        emit_result_ready(&mut app, &child, &message);
        emit_result_ready(&mut app, &child, &message);
        assert_eq!(
            wait_for_result_bridge(&app, &message.message_id).await,
            None
        );
        assert_eq!(saved_root_messages(&database, conversation_id), [expected]);
        assert_eq!(
            committed(load_messages(&writer.sender, parent.task_id, parent.generation).unwrap())
                .await,
            stored
        );
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    });
}

#[test]
fn oz_result_ready_rejects_previous_parent_generation_or_user_exchange() {
    for change_generation in [true, false] {
        App::test((), move |mut app| async move {
            let directory = tempfile::tempdir().unwrap();
            let database = directory.path().join("oz-stale-result.sqlite");
            let writer = start_test_writer(&database).unwrap();
            let (conversation_id, mut parent, child, message) =
                prepare_oz_result(&mut app, &database, &writer.sender).await;
            if change_generation {
                parent.state = LocalCliTaskState::Disconnected;
                parent.revision += 1;
                committed(checkpoint_task(&writer.sender, parent.clone(), Some(1)).unwrap()).await;
                parent.generation += 1;
                parent.revision = 0;
                parent.state = LocalCliTaskState::Queued;
                committed(checkpoint_task(&writer.sender, parent.clone(), Some(1)).unwrap()).await;
            } else {
                BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, _| {
                    // 模拟用户轮已经改变、父记录尚未提交换代的窗口。
                    history
                        .conversation_mut(&conversation_id)
                        .unwrap()
                        .append_root_exchange_for_test(explicit_parent_exchange());
                    assert_ne!(
                        local_parent_history_identity(
                            history.conversation(&conversation_id).unwrap()
                        ),
                        recorded_history_identity(&parent),
                    );
                });
            }
            let before = committed(
                load_messages(
                    &writer.sender,
                    parent.task_id.clone(),
                    message.recipient_generation,
                )
                .unwrap(),
            )
            .await;
            assert_eq!(before.len(), 1);
            assert_eq!(before[0].receipt_kind, None);
            emit_result_ready(&mut app, &child, &message);
            assert!(
                wait_for_result_bridge(&app, &message.message_id)
                    .await
                    .is_some()
            );
            assert!(saved_root_messages(&database, conversation_id).is_empty());
            assert_eq!(
                committed(
                    load_messages(&writer.sender, parent.task_id, message.recipient_generation)
                        .unwrap()
                )
                .await,
                before,
                "过期事件不能领取、确认或重投原结果"
            );
            writer.sender.send(ModelEvent::Terminate).unwrap();
            writer.handle.join().unwrap();
        });
    }
}

#[test]
fn oz_result_ready_without_history_persistence_keeps_claim_unconfirmed() {
    App::test((), |mut app| async move {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("oz-unconfirmed-result.sqlite");
        let writer = start_test_writer(&database).unwrap();
        let (conversation_id, parent, child, message) =
            prepare_oz_result(&mut app, &database, &writer.sender).await;
        GeneralSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .persist_conversations
                .set_value(false, ctx)
                .unwrap();
        });
        emit_result_ready(&mut app, &child, &message);
        assert!(
            wait_for_result_bridge(&app, &message.message_id)
                .await
                .is_some()
        );
        let stored = committed(
            load_messages(&writer.sender, parent.task_id.clone(), parent.generation).unwrap(),
        )
        .await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].message_id, message.message_id);
        assert_eq!(stored[0].state, LocalCliMessageState::Sent);
        assert_eq!(stored[0].receipt_kind, None);
        assert!(saved_root_messages(&database, conversation_id).is_empty());
        assert!(
            !BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                conversation_has_message(
                    history.conversation(&conversation_id).unwrap(),
                    &message.message_id,
                )
            })
        );
        // 再次收到事件也不能把没有入历史的已领取结果自动重投或伪造确认。
        emit_result_ready(&mut app, &child, &stored[0]);
        emit_result_ready(&mut app, &child, &message);
        wait_for_result_bridge(&app, &message.message_id).await;
        assert!(saved_root_messages(&database, conversation_id).is_empty());
        assert_eq!(
            committed(load_messages(&writer.sender, parent.task_id, parent.generation).unwrap())
                .await,
            stored
        );
        writer.sender.send(ModelEvent::Terminate).unwrap();
        writer.handle.join().unwrap();
    });
}
