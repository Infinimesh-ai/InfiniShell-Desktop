use ::settings::Setting as _;
use instant::Instant;
use warpui::App;

use super::*;
use crate::ai::agent::AIAgentInput;
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent_providers::{AgentProviderSecrets, llm_id};
use crate::ai::blocklist::ResponseStreamId;
use crate::ai::llms::LLMPreferences;
use crate::settings::{AgentProvider, AgentProviderApiType, AgentProviderModel};
use crate::terminal::general_settings::GeneralSettings;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

#[test]
fn accepted_submission_consumes_only_the_submitted_draft() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            input.replace_buffer_content("已经接收的问题", ctx);
            input.ai_context_model.update(ctx, |context, ctx| {
                context.set_pending_context_selected_text(Some("原上下文".into()), true, ctx);
            });
            let revision = input.editor.as_ref(ctx).buffer_revision(ctx);
            let context_snapshot = input
                .ai_context_model
                .as_ref(ctx)
                .pending_context_snapshot();
            input.acknowledge_ai_input_submission(&revision, &context_snapshot, ctx);
            assert!(input.buffer_text(ctx).is_empty());
            assert!(
                input
                    .ai_context_model
                    .as_ref(ctx)
                    .pending_context_selected_text()
                    .is_none()
            );

            input.replace_buffer_content("停止后新输入的问题", ctx);
            input.acknowledge_ai_input_submission(&revision, &context_snapshot, ctx);
            assert_eq!(input.buffer_text(ctx), "停止后新输入的问题");
        });
    });
}

#[test]
fn acknowledgement_preserves_retyped_identical_text_and_new_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            input.replace_buffer_content("同一句问题", ctx);
            let revision = input.editor.as_ref(ctx).buffer_revision(ctx);
            let context_snapshot = input
                .ai_context_model
                .as_ref(ctx)
                .pending_context_snapshot();
            input.replace_buffer_content("中途修改", ctx);
            input.replace_buffer_content("同一句问题", ctx);
            input.ai_context_model.update(ctx, |context, ctx| {
                context.set_pending_context_selected_text(Some("新增上下文".into()), true, ctx);
            });
            input.acknowledge_ai_input_submission(&revision, &context_snapshot, ctx);
            assert_eq!(input.buffer_text(ctx), "同一句问题");
            assert_eq!(
                input
                    .ai_context_model
                    .as_ref(ctx)
                    .pending_context_selected_text()
                    .map(String::as_str),
                Some("新增上下文")
            );
        });
    });
}

#[test]
fn acknowledgement_preserves_context_added_after_submission() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            input.replace_buffer_content("已经接收的问题", ctx);
            let revision = input.editor.as_ref(ctx).buffer_revision(ctx);
            let context_snapshot = input
                .ai_context_model
                .as_ref(ctx)
                .pending_context_snapshot();
            input.ai_context_model.update(ctx, |context, ctx| {
                context.set_pending_context_selected_text(Some("新增上下文".into()), true, ctx);
            });
            input.acknowledge_ai_input_submission(&revision, &context_snapshot, ctx);
            assert!(input.buffer_text(ctx).is_empty());
            assert_eq!(
                input
                    .ai_context_model
                    .as_ref(ctx)
                    .pending_context_selected_text()
                    .map(String::as_str),
                Some("新增上下文")
            );
        });
    });
}

#[test]
fn delayed_request_event_cannot_clear_follow_up_draft() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let (input, controller) = terminal.read(&app, |terminal, _| {
            (terminal.input().clone(), terminal.ai_controller().clone())
        });
        input.update(&mut app, |input, ctx| {
            input.replace_buffer_content("停止后的新问题", ctx)
        });
        controller.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIControllerEvent::SentRequest {
                contains_user_query: true,
                is_queued_prompt: false,
                model_id: "test-model".into(),
                stream_id: ResponseStreamId::new_local(),
            });
        });
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "停止后的新问题")
        });
    });
}

#[test]
fn empty_enter_never_creates_a_request() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            assert!(!input.submit_user_query_now(String::new(), false, ctx));
            assert!(!input.submit_user_query_now(" \n\t".into(), false, ctx));
            assert!(
                input
                    .ai_context_model
                    .as_ref(ctx)
                    .selected_conversation_id(ctx)
                    .is_none()
            );
        });
    });
}

#[test]
fn accepted_query_clears_immediately_and_second_enter_does_not_resend() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let mut server = mockito::Server::new();
        let title_response = mock_submission_title(&mut server, "只发送一次的问题");
        let response = server.mock("POST", "/v1/chat/completions")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({"stream": true})))
            .with_header("content-type", "text/event-stream")
            .with_body(concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"测试回答\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ))
            .expect(1)
            .create();
        let mut provider = AgentProvider::new_empty();
        provider.id = "input-submission-test".into();
        provider.api_type = AgentProviderApiType::OpenAi;
        provider.base_url = format!("{}/v1", server.url());
        provider
            .models
            .push(AgentProviderModel::from_id("test-model".into()));
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .agent_providers
                .set_value(vec![provider], ctx)
                .unwrap();
        });
        GeneralSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .persist_conversations
                .set_value(false, ctx)
                .unwrap();
        });
        AgentProviderSecrets::handle(&app).update(&mut app, |secrets, ctx| {
            secrets.set("input-submission-test", "synthetic-test-key".into(), ctx);
        });
        let terminal = add_window_with_terminal(&mut app, None);
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        LLMPreferences::handle(&app).update(&mut app, |preferences, ctx| {
            preferences.update_preferred_agent_mode_llm(
                &llm_id::encode("input-submission-test", "test-model"),
                terminal.id(),
                ctx,
            );
        });
        input.update(&mut app, |input, ctx| {
            input.replace_buffer_content("只发送一次的问题", ctx);
            assert!(input.submit_user_query_now(input.buffer_text(ctx), false, ctx));
            assert!(input.buffer_text(ctx).is_empty());
            assert!(!input.submit_user_query_now(input.buffer_text(ctx), false, ctx));
            input.replace_buffer_content("请求完成前输入的新草稿", ctx);
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let finished = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                history
                    .all_live_conversations()
                    .iter()
                    .any(|(_, conversation)| conversation.status() == &ConversationStatus::Success)
            });
            if finished && title_response.matched() {
                break;
            }
            assert!(Instant::now() < deadline, "本地模拟响应未完成");
            warpui::r#async::Timer::after(Duration::from_millis(10)).await;
        }
        response.assert();
        title_response.assert();
        assert_single_submitted_query("只发送一次的问题", &app);
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "请求完成前输入的新草稿")
        });
    });
}

fn mock_submission_title(server: &mut mockito::Server, query: &str) -> mockito::Mock {
    // 首轮回答结束后会另发非流式标题请求，不能把它计作重复提交用户问题。
    server
        .mock("POST", "/v1/chat/completions")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "stream": false,
            "messages": [
                {"role": "system"},
                {
                    "role": "user",
                    "content": format!("Generate a title for this conversation:\n<user>{query}</user>"),
                },
            ],
        })))
        .with_header("content-type", "application/json")
        .with_body(r#"{"choices":[{"message":{"role":"assistant","content":"本地测试会话"},"finish_reason":"stop"}]}"#)
        .expect(1)
        .create()
}

fn assert_single_submitted_query(expected_query: &str, app: &App) {
    BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
        let queries = history
            .all_live_conversations()
            .into_iter()
            .flat_map(|(_, conversation)| conversation.all_exchanges())
            .flat_map(|exchange| &exchange.input)
            .filter_map(|input| {
                if let AIAgentInput::UserQuery { query, .. } = input {
                    Some(query.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(queries, vec![expected_query.to_owned()]);
    });
}

fn submission_image_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::Image(ImageContext {
        data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=".into(),
        mime_type: "image/png".into(),
        file_name: file_name.into(),
        is_figma: false,
    })
}

#[test]
fn accepted_queue_command_consumes_original_draft_and_preserves_follow_up() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let mut server = mockito::Server::new();
        let title_response = mock_submission_title(&mut server, "队列发送的问题");
        let response = server.mock("POST", "/v1/chat/completions")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({"stream": true})))
            .with_header("content-type", "text/event-stream")
            .with_body(concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"队列测试回答\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ))
            .expect(1)
            .create();
        let mut provider = AgentProvider::new_empty();
        provider.id = "queue-submission-test".into();
        provider.api_type = AgentProviderApiType::OpenAi;
        provider.base_url = format!("{}/v1", server.url());
        let mut model = AgentProviderModel::from_id("test-model".into());
        model.context_window = 200_000;
        model.image = Some(true);
        provider.models.push(model);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .agent_providers
                .set_value(vec![provider], ctx)
                .unwrap();
        });
        GeneralSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .persist_conversations
                .set_value(false, ctx)
                .unwrap();
        });
        AgentProviderSecrets::handle(&app).update(&mut app, |secrets, ctx| {
            secrets.set("queue-submission-test", "synthetic-test-key".into(), ctx);
        });
        let terminal = add_window_with_terminal(&mut app, None);
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        LLMPreferences::handle(&app).update(&mut app, |preferences, ctx| {
            preferences.update_preferred_agent_mode_llm(
                &llm_id::encode("queue-submission-test", "test-model"),
                terminal.id(),
                ctx,
            );
        });
        let follow_up_attachment = submission_image_attachment("follow-up.png");
        let conversation_id = input.update(&mut app, |input, ctx| {
            let conversation_id = input.agent_view_controller.update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::SlashCommand {
                            trigger: SlashCommandTrigger::input(),
                        },
                        ctx,
                    )
                    .unwrap()
            });
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                history
                    .conversation_mut(&conversation_id)
                    .unwrap()
                    .set_status_for_test(ConversationStatus::Success);
            });
            input.replace_buffer_content("/queue 队列发送的问题", ctx);
            input.ai_context_model.update(ctx, |context, ctx| {
                context.append_pending_attachments(
                    vec![submission_image_attachment("submitted.png")],
                    ctx,
                );
            });
            let argument = "队列发送的问题".to_owned();
            assert!(input.execute_slash_command(
                &commands::QUEUE,
                Some(&argument),
                SlashCommandTrigger::input(),
                false,
                ctx,
            ));
            assert!(input.buffer_text(ctx).is_empty());
            assert!(
                input
                    .ai_context_model
                    .as_ref(ctx)
                    .pending_attachments()
                    .is_empty()
            );
            input.replace_buffer_content("队列请求完成前的新草稿", ctx);
            input.ai_context_model.update(ctx, |context, ctx| {
                context.append_pending_attachments(vec![follow_up_attachment.clone()], ctx);
            });
            conversation_id
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let finished = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                history
                    .conversation(&conversation_id)
                    .is_some_and(|conversation| {
                        conversation.status() == &ConversationStatus::Success
                    })
            });
            if finished && title_response.matched() {
                break;
            }
            assert!(Instant::now() < deadline, "本地模拟队列响应未完成");
            warpui::r#async::Timer::after(Duration::from_millis(10)).await;
        }
        response.assert();
        title_response.assert();
        assert_single_submitted_query("队列发送的问题", &app);
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "队列请求完成前的新草稿");
            assert_eq!(
                input.ai_context_model.as_ref(ctx).pending_attachments(),
                &[follow_up_attachment],
            );
        });
    });
}

#[test]
fn rejected_queue_command_preserves_original_draft_and_attachments() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        input.update(&mut app, |input, ctx| {
            let conversation_id = input.agent_view_controller.update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::SlashCommand {
                            trigger: SlashCommandTrigger::input(),
                        },
                        ctx,
                    )
                    .unwrap()
            });
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                history
                    .conversation_mut(&conversation_id)
                    .unwrap()
                    .set_status_for_test(ConversationStatus::Success);
            });
            // 保持真实回退守卫存活，让控制器同步拒绝请求，不触发网络。
            let rewind_guard = input.ai_controller.update(ctx, |controller, _| {
                controller
                    .begin_conversation_rewind(conversation_id)
                    .unwrap()
            });
            let attachment = submission_image_attachment("rejected.png");
            input.replace_buffer_content("/queue 暂时不能发送的问题", ctx);
            input.ai_context_model.update(ctx, |context, ctx| {
                context.append_pending_attachments(vec![attachment.clone()], ctx);
            });
            let argument = "暂时不能发送的问题".to_owned();
            assert!(input.execute_slash_command(
                &commands::QUEUE,
                Some(&argument),
                SlashCommandTrigger::input(),
                false,
                ctx,
            ));
            assert_eq!(input.buffer_text(ctx), "/queue 暂时不能发送的问题");
            assert_eq!(
                input.ai_context_model.as_ref(ctx).pending_attachments(),
                &[attachment],
            );
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .unwrap()
                    .exchange_count(),
                0,
            );
            drop(rewind_guard);
        });
    });
}

#[test]
fn accepted_queued_submission_preserves_identical_fresh_draft_and_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let mut server = mockito::Server::new();
        let title_response = mock_submission_title(&mut server, "与排队问题相同的新草稿");
        let response = server.mock("POST", "/v1/chat/completions")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({"stream": true})))
            .with_header("content-type", "text/event-stream")
            .with_body(concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"后台队列回答\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ))
            .expect(1)
            .create();
        let mut provider = AgentProvider::new_empty();
        provider.id = "background-queue-test".into();
        provider.api_type = AgentProviderApiType::OpenAi;
        provider.base_url = format!("{}/v1", server.url());
        let mut model = AgentProviderModel::from_id("test-model".into());
        model.context_window = 200_000;
        model.image = Some(true);
        provider.models.push(model);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .agent_providers
                .set_value(vec![provider], ctx)
                .unwrap();
        });
        GeneralSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .persist_conversations
                .set_value(false, ctx)
                .unwrap();
        });
        AgentProviderSecrets::handle(&app).update(&mut app, |secrets, ctx| {
            secrets.set("background-queue-test", "synthetic-test-key".into(), ctx);
        });
        let terminal = add_window_with_terminal(&mut app, None);
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        LLMPreferences::handle(&app).update(&mut app, |preferences, ctx| {
            preferences.update_preferred_agent_mode_llm(
                &llm_id::encode("background-queue-test", "test-model"),
                terminal.id(),
                ctx,
            );
        });
        let fresh_attachment = submission_image_attachment("fresh-draft.png");
        let conversation_id = input.update(&mut app, |input, ctx| {
            let conversation_id = input.agent_view_controller.update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::SlashCommand {
                            trigger: SlashCommandTrigger::input(),
                        },
                        ctx,
                    )
                    .unwrap()
            });
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                history
                    .conversation_mut(&conversation_id)
                    .unwrap()
                    .set_status_for_test(ConversationStatus::Success);
            });
            // 新草稿即使与旧排队问题同文，也不属于后台提交。
            input.replace_buffer_content("与排队问题相同的新草稿", ctx);
            input.ai_context_model.update(ctx, |context, ctx| {
                context.set_pending_context_selected_text(Some("新草稿的上下文".into()), true, ctx);
                context.append_pending_attachments(vec![fresh_attachment.clone()], ctx);
            });
            assert!(input.submit_user_query_now("与排队问题相同的新草稿".into(), true, ctx));
            assert_eq!(input.buffer_text(ctx), "与排队问题相同的新草稿");
            assert_eq!(
                input.ai_context_model.as_ref(ctx).pending_attachments(),
                &[fresh_attachment.clone()],
            );
            assert_eq!(
                input
                    .ai_context_model
                    .as_ref(ctx)
                    .pending_context_selected_text()
                    .map(String::as_str),
                Some("新草稿的上下文"),
            );
            conversation_id
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let finished = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                history
                    .conversation(&conversation_id)
                    .is_some_and(|conversation| {
                        conversation.status() == &ConversationStatus::Success
                    })
            });
            if finished && title_response.matched() {
                break;
            }
            assert!(Instant::now() < deadline, "本地模拟后台队列响应未完成");
            warpui::r#async::Timer::after(Duration::from_millis(10)).await;
        }
        response.assert();
        title_response.assert();
        assert_single_submitted_query("与排队问题相同的新草稿", &app);
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "与排队问题相同的新草稿");
            assert_eq!(
                input.ai_context_model.as_ref(ctx).pending_attachments(),
                &[fresh_attachment],
            );
            assert_eq!(
                input
                    .ai_context_model
                    .as_ref(ctx)
                    .pending_context_selected_text()
                    .map(String::as_str),
                Some("新草稿的上下文"),
            );
        });
    });
}
