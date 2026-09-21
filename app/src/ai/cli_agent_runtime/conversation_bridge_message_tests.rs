use std::time::Duration;

use ::settings::Setting;
use futures::FutureExt;
use uuid::Uuid;
use warpui::App;
use warpui::r#async::Timer;

use super::*;
use crate::ai::agent::{AIAgentExchange, AIAgentExchangeId, AIAgentOutputStatus};
use crate::ai::llms::LLMId;
use crate::global_resource_handles::GlobalResourceHandlesProvider;
use crate::persistence::local_cli_tasks::{enqueue_message, load_messages};
use crate::persistence::model::LocalCliReceiptKind;
use crate::persistence::{ModelEvent, read_test_agent_conversation, start_test_writer};
use crate::terminal::general_settings::GeneralSettings;
use crate::test_util::ai_agent_tasks::create_api_task;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn user_exchange() -> AIAgentExchange {
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

async fn commit<T>(receiver: futures::channel::oneshot::Receiver<Result<T, String>>) -> T {
    futures::select! {
        result = receiver.fuse() => result.expect("私有数据库已关闭").unwrap(),
        _ = Timer::after(Duration::from_secs(5)).fuse() => panic!("私有数据库提交超时"),
    }
}

#[test]
fn ordinary_parent_message_event_commits_history_and_never_replays_sent_without_history() {
    // 分别验证成功、持久化关闭和父用户轮已经变化，全部走生产模型订阅。
    for scenario in ["current", "persistence-disabled", "new-exchange"] {
        App::test((), move |mut app| async move {
            initialize_app_for_terminal_view(&mut app);
            let directory = tempfile::tempdir().unwrap();
            let database = directory.path().join("ordinary-message.sqlite");
            let writer = start_test_writer(&database).unwrap();
            GlobalResourceHandlesProvider::handle(&app).update(&mut app, |resources, _| {
                resources.set_model_event_sender_for_test(writer.sender.clone());
            });
            GeneralSettings::handle(&app).update(&mut app, |settings, ctx| {
                settings.persist_conversations.set_value(true, ctx).unwrap();
            });
            app.add_singleton_model(|_| LocalCLITaskCoordinator::new(Some(writer.sender.clone())));
            app.add_singleton_model(LocalCLIConversationBridge::new);
            let terminal = add_window_with_terminal(&mut app, None);
            let parent_id = Uuid::new_v4().to_string();
            let (conversation_id, identity, checkpoint) =
                terminal.update(&mut app, |terminal, ctx| {
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        let id =
                            history.start_new_conversation(terminal.id(), false, false, false, ctx);
                        let conversation = history.conversation_mut(&id).unwrap();
                        conversation.upgrade_optimistic_root_to_server_task_for_test(
                            create_api_task("root", Vec::new()),
                        );
                        conversation.set_orchestration_harness(Harness::Oz);
                        conversation.append_root_exchange_for_test(user_exchange());
                        history.assign_run_id_for_conversation(
                            id,
                            parent_id.clone(),
                            None,
                            terminal.id(),
                            ctx,
                        );
                        let conversation = history.conversation(&id).unwrap();
                        (
                            id,
                            local_parent_history_identity(conversation).unwrap(),
                            conversation
                                .checkpoint_compaction_recovery_state(ctx)
                                .unwrap()
                                .unwrap(),
                        )
                    })
                });
            commit(checkpoint).await;
            let parent = LocalCliTask {
                version: 1,
                task_id: parent_id,
                parent_task_id: None,
                parent_generation: None,
                harness: "oz".into(),
                working_directory: directory.path().to_string_lossy().into_owned(),
                config_json:
                    serde_json::json!({"execution_kind":"local_parent","history_identity":identity})
                        .to_string(),
                native_session_id: None,
                generation: 1,
                revision: 0,
                state: LocalCliTaskState::Queued,
                result: None,
                terminal_evidence: None,
            };
            commit(checkpoint_task(&writer.sender, parent.clone(), None).unwrap()).await;
            let child = LocalCliTask {
                task_id: Uuid::new_v4().to_string(),
                parent_task_id: Some(parent.task_id.clone()),
                parent_generation: Some(1),
                harness: "codex".into(),
                config_json:
                    serde_json::json!({"local_tools":{"allow_spawn":false,"allow_message":true}})
                        .to_string(),
                ..parent.clone()
            };
            commit(checkpoint_task(&writer.sender, child.clone(), None).unwrap()).await;
            let message = LocalCliMessage {
                version: 1,
                message_id: Uuid::new_v4().to_string(),
                sender_task_id: child.task_id.clone(),
                recipient_task_id: parent.task_id.clone(),
                sender_generation: 1,
                recipient_generation: 1,
                subject: "进度 Progress".into(),
                body: "中文报告\nEnglish progress 🧪".into(),
                state: LocalCliMessageState::Queued,
                receipt_kind: None,
            };
            commit(enqueue_message(&writer.sender, message.clone()).unwrap()).await;
            if scenario == "persistence-disabled" {
                GeneralSettings::handle(&app).update(&mut app, |settings, ctx| {
                    settings
                        .persist_conversations
                        .set_value(false, ctx)
                        .unwrap();
                });
            } else if scenario == "new-exchange" {
                BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, _| {
                    history
                        .conversation_mut(&conversation_id)
                        .unwrap()
                        .append_root_exchange_for_test(user_exchange());
                });
            }
            for _ in 0..2 {
                LocalCLITaskCoordinator::handle(&app).update(&mut app, |_, ctx| {
                    ctx.emit(LocalCLITaskCoordinatorEvent::ParentMessageReady {
                        task: child.clone(),
                        message: message.clone(),
                    });
                });
                let mut finished = false;
                for _ in 0..250 {
                    Timer::after(Duration::from_millis(10)).await;
                    if LocalCLIConversationBridge::handle(&app).read(&app, |bridge, _| {
                        !bridge.pending_results.contains_key(&message.message_id)
                    }) {
                        finished = true;
                        break;
                    }
                }
                assert!(finished, "普通父消息桥超时");
            }
            let messages = commit(load_messages(&writer.sender, parent.task_id, 1).unwrap()).await;
            assert_eq!(messages.len(), 1);
            let history = read_test_agent_conversation(&database, &conversation_id.to_string())
                .unwrap()
                .unwrap();
            let saved: Vec<_> = history
                .tasks
                .into_iter()
                .flat_map(|task| task.messages)
                .collect();
            if scenario == "current" {
                assert_eq!(messages[0].state, LocalCliMessageState::Acknowledged);
                assert_eq!(
                    messages[0].receipt_kind,
                    Some(LocalCliReceiptKind::ApplicationHistory)
                );
                assert_eq!(saved, [history_result_message("root", &message)]);
                let Some(Message::MessagesReceivedFromAgents(received)) = &saved[0].message else {
                    panic!("缺少普通父消息");
                };
                assert_eq!(received.messages[0].subject, message.subject);
                assert_eq!(received.messages[0].message_body, message.body);
            } else {
                assert!(saved.is_empty());
                assert_eq!(messages[0].receipt_kind, None);
                assert_eq!(
                    messages[0].state,
                    if scenario == "persistence-disabled" {
                        LocalCliMessageState::Sent
                    } else {
                        LocalCliMessageState::Queued
                    }
                );
                assert!(
                    LocalCLIConversationBridge::handle(&app).read(&app, |bridge, _| bridge
                        .delivery_error(&message.message_id)
                        .is_some())
                );
            }
            writer.sender.send(ModelEvent::Terminate).unwrap();
            writer.handle.join().unwrap();
        });
    }
}
