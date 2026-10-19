use std::collections::HashSet;

use settings::Setting;
use warp_multi_agent_api::{ResponseEvent, response_event};
use warpui::{App, ViewHandle};

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionId, AIAgentActionResult, AIAgentActionResultType, SuggestPromptResult,
};
use crate::ai::agent_providers::llm_id;
use crate::ai::blocklist::controller::{RequestInput, ResponseStreamId};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::llms::LLMPreferences;
use crate::settings::{AISettings, AgentProvider, AgentProviderApiType, AgentProviderModel};
use crate::terminal::view::TerminalView;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

struct RoutingFixture {
    terminal: ViewHandle<TerminalView>,
    conversation_id: AIConversationId,
    root_task_id: TaskId,
    cli_task_id: TaskId,
}

impl RoutingFixture {
    fn request(&self, task_id: &TaskId, app: &App) -> RequestInput {
        self.terminal.read(app, |terminal, ctx| {
            RequestInput::for_task(
                vec![AIAgentInput::UserQuery {
                    query: "检查当前终端".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: Default::default(),
                    running_command: None,
                    intended_agent: None,
                }],
                task_id.clone(),
                terminal.active_session(),
                None,
                self.conversation_id,
                terminal.id(),
                ctx,
            )
        })
    }

    fn params(&self, request: &RequestInput, app: &mut App) -> api::RequestParams {
        self.terminal.update(app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                let snapshot = controller
                    .conversation_snapshot_for_request(request, ctx)
                    .unwrap();
                controller.build_request_params_for_input(
                    request,
                    snapshot.conversation_data,
                    None,
                    snapshot.parent_agent_id,
                    snapshot.agent_name,
                    ctx,
                )
            })
        })
    }

    fn dispatch(&self, params: &api::RequestParams, app: &App) -> ByopDispatch {
        app.read(|ctx| {
            byop_dispatch_info(
                params,
                &AIIdentifiers {
                    client_conversation_id: Some(self.conversation_id),
                    ..Default::default()
                },
                ctx,
            )
            .expect("有效提供商应产生实际发送参数")
        })
    }

    fn set_cli_model(&self, model: Option<ai::LLMId>, app: &mut App) {
        AIExecutionProfilesModel::handle(app).update(app, |profiles, ctx| {
            let profile_id = profiles
                .active_profile(Some(self.terminal.id()), ctx)
                .id()
                .clone();
            profiles.set_cli_agent_model(&profile_id, model, ctx);
        });
    }
}

fn with_routing_app(f: impl FnOnce(RoutingFixture, &mut App) + 'static) {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .agent_providers
                .set_value(
                    vec![
                        routing_provider("cli-default", AgentProviderApiType::OpenAi),
                        routing_provider("base", AgentProviderApiType::OpenAiResp),
                        routing_provider("cli-selected", AgentProviderApiType::OpenAi),
                    ],
                    ctx,
                )
                .unwrap();
        });
        LLMPreferences::handle(&app).update(&mut app, |preferences, ctx| {
            preferences.refresh_byop_models(ctx);
            preferences.update_preferred_agent_mode_llm(
                &llm_id::encode("base", "model"),
                terminal.id(),
                ctx,
            );
        });
        let (conversation_id, root_task_id, cli_task_id) =
            terminal.update(&mut app, |terminal, ctx| {
                let block_id = {
                    let mut model = terminal.model.lock();
                    model.simulate_long_running_block("fixture-terminal", "");
                    let block = model.block_list_mut().active_block_mut();
                    block.set_is_agent_tagged_in(true);
                    block.id().clone()
                };
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    let id =
                        history.start_new_conversation(terminal.id(), false, false, false, ctx);
                    let conversation = history.conversation_mut(&id).unwrap();
                    let cli = conversation.create_optimistic_cli_subagent_task_for_test(&block_id);
                    (id, conversation.get_root_task_id().clone(), cli)
                })
            });
        let fixture = RoutingFixture {
            terminal,
            conversation_id,
            root_task_id,
            cli_task_id,
        };
        fixture.set_cli_model(Some(llm_id::encode("cli-selected", "model")), &mut app);
        f(fixture, &mut app);
    });
}

fn routing_provider(id: &str, api_type: AgentProviderApiType) -> AgentProvider {
    AgentProvider {
        id: id.to_owned(),
        name: id.to_owned(),
        kind: Default::default(),
        api_type,
        base_url: format!("https://{id}.invalid/v1/"),
        models: vec![AgentProviderModel::from_id("model".to_owned())],
        extra_headers: vec![],
        responses: Default::default(),
    }
}

#[test]
fn 共享会话初始化保留base模型且不循环借用历史模型() {
    with_routing_app(|fixture, app| {
        BlocklistAIHistoryModel::handle(app).update(app, |history, _ctx| {
            history.set_server_conversation_token_for_conversation(
                fixture.conversation_id,
                "shared-routing-conversation".to_owned(),
            );
        });
        fixture.terminal.update(app, |terminal, ctx| {
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.handle_shared_session_response_event(
                    ResponseEvent {
                        r#type: Some(response_event::Type::Init(response_event::StreamInit {
                            request_id: "shared-routing-request".to_owned(),
                            conversation_id: "shared-routing-conversation".to_owned(),
                            run_id: String::new(),
                        })),
                    },
                    ctx,
                );
                assert_eq!(
                    controller.get_current_shared_session_conversation_id(ctx),
                    Some(fixture.conversation_id)
                );
            });
        });
        BlocklistAIHistoryModel::handle(app).read(app, |history, _ctx| {
            let conversation = history.conversation(&fixture.conversation_id).unwrap();
            assert_eq!(conversation.root_task_exchanges().count(), 1);
            let exchange = conversation.root_task_exchanges().next().unwrap();
            assert_eq!(exchange.model_id, llm_id::encode("base", "model"));
            assert_eq!(
                exchange.cli_agent_model_id,
                llm_id::encode("cli-selected", "model")
            );
        });
    });
}

#[test]
fn cli首次请求使用界面选择的模型和提供商() {
    with_routing_app(|fixture, app| {
        let request = fixture.request(&fixture.cli_task_id, app);
        assert_eq!(request.model_id, llm_id::encode("cli-selected", "model"));
        let params = fixture.params(&request, app);
        assert!(params.lrc_is_user_tag_in);
        assert!(params.lrc_should_spawn_subagent);
        assert_eq!(params.model, request.model_id);
        let dispatch = fixture.dispatch(&params, app);
        assert_eq!(dispatch.base_url, "https://cli-selected.invalid/v1/");
        assert_eq!(dispatch.api_type, AgentProviderApiType::OpenAi);
        assert_eq!(dispatch.model_id, "model");
    });
}

#[test]
fn 接管终端不改变普通base请求模型() {
    with_routing_app(|fixture, app| {
        let request = fixture.request(&fixture.root_task_id, app);
        assert_eq!(request.model_id, llm_id::encode("base", "model"));
        let params = fixture.params(&request, app);
        let dispatch = fixture.dispatch(&params, app);
        assert_eq!(dispatch.base_url, "https://base.invalid/v1/");
        assert_eq!(dispatch.api_type, AgentProviderApiType::OpenAiResp);
    });
}

#[test]
fn 未设cli模型时请求使用与界面相同的回退() {
    with_routing_app(|fixture, app| {
        fixture.set_cli_model(None, app);
        let displayed = app.read(|ctx| {
            LLMPreferences::as_ref(ctx)
                .get_active_cli_agent_model(ctx, Some(fixture.terminal.id()))
                .id
                .clone()
        });
        assert_eq!(displayed, llm_id::encode("cli-default", "model"));
        let request = fixture.request(&fixture.cli_task_id, app);
        assert_eq!(request.model_id, displayed);
        let params = fixture.params(&request, app);
        assert_eq!(
            fixture.dispatch(&params, app).base_url,
            "https://cli-default.invalid/v1/"
        );
    });
}

#[test]
fn cli重试和压缩保留原请求模型快照() {
    with_routing_app(|fixture, app| {
        let mut request = fixture.request(&fixture.cli_task_id, app);
        request.input_messages.push((
            fixture.cli_task_id.clone(),
            vec![AIAgentInput::ResumeConversation {
                context: Arc::from([]),
            }],
        ));
        let params = fixture.params(&request, app);
        let stream =
            ResponseStream::new_for_test_with_params(ResponseStreamId::new_local(), params);
        fixture.set_cli_model(Some(llm_id::encode("base", "model")), app);

        // retry 的分流与压缩都使用流的请求快照，不能因稍后改设置而切换提供商。
        assert_eq!(
            fixture.dispatch(&stream.params, app).base_url,
            "https://cli-selected.invalid/v1/"
        );
        let mut resumed = fixture.request(&fixture.root_task_id, app);
        assert_eq!(resumed.model_id, llm_id::encode("base", "model"));
        resumed.input_messages = request.input_messages.clone();
        stream.preserve_request_models(&mut resumed);
        let resumed_params = fixture.params(&resumed, app);
        assert_eq!(
            resumed_params.byop_target_task_id,
            Some(fixture.cli_task_id.to_string())
        );
        assert_eq!(resumed_params.input, stream.params.input);
        assert_eq!(
            fixture.dispatch(&resumed_params, app).base_url,
            "https://cli-selected.invalid/v1/"
        );
    });
}

#[test]
fn cli工具结果续轮使用cli模型但混合目标保持base() {
    with_routing_app(|fixture, app| {
        let cli_result = AIAgentActionResult {
            id: AIAgentActionId::from("cli-action".to_owned()),
            task_id: fixture.cli_task_id.clone(),
            result: AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
        };
        let request = fixture.terminal.read(app, |terminal, ctx| {
            RequestInput::for_actions_results(
                vec![cli_result.clone()],
                Arc::from([]),
                terminal.active_session(),
                None,
                fixture.conversation_id,
                terminal.id(),
                ctx,
            )
        });
        assert_eq!(request.model_id, llm_id::encode("cli-selected", "model"));
        let root_result = AIAgentActionResult {
            id: AIAgentActionId::from("root-action".to_owned()),
            task_id: fixture.root_task_id.clone(),
            result: AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
        };
        let mixed = fixture.terminal.read(app, |terminal, ctx| {
            RequestInput::for_actions_results(
                vec![cli_result, root_result],
                Arc::from([]),
                terminal.active_session(),
                None,
                fixture.conversation_id,
                terminal.id(),
                ctx,
            )
        });
        assert_eq!(mixed.model_id, llm_id::encode("base", "model"));
    });
}

#[test]
fn ordered_cli_result_batches_keep_selected_model_and_byop_target() {
    with_routing_app(|fixture, app| {
        let first = AIAgentActionResult {
            id: AIAgentActionId::from("cli-first".to_owned()),
            task_id: fixture.cli_task_id.clone(),
            result: AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
        };
        let second = AIAgentActionResult {
            id: AIAgentActionId::from("cli-second".to_owned()),
            task_id: fixture.cli_task_id.clone(),
            result: AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
        };
        let request = fixture.terminal.read(app, |terminal, ctx| {
            RequestInput::for_actions_results(
                vec![first, second],
                Arc::from([]),
                terminal.active_session(),
                None,
                fixture.conversation_id,
                terminal.id(),
                ctx,
            )
        });

        let params = fixture.params(&request, app);

        assert_eq!(request.input_messages.len(), 2);
        assert_eq!(request.model_id, llm_id::encode("cli-selected", "model"));
        assert_eq!(
            params.byop_target_task_id,
            Some(fixture.cli_task_id.to_string())
        );
        assert!(matches!(
            params.input.as_slice(),
            [AIAgentInput::ActionResult { result: first, .. }, AIAgentInput::ActionResult { result: second, .. }]
                if first.id.to_string() == "cli-first" && second.id.to_string() == "cli-second"
        ));
        assert_eq!(
            fixture.dispatch(&params, app).base_url,
            "https://cli-selected.invalid/v1/"
        );
    });
}

#[test]
fn ordered_interleaved_results_keep_transport_order_and_base_model() {
    with_routing_app(|fixture, app| {
        let first = AIAgentActionResult {
            id: AIAgentActionId::from("root-first".to_owned()),
            task_id: fixture.root_task_id.clone(),
            result: AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
        };
        let middle = AIAgentActionResult {
            id: AIAgentActionId::from("cli-middle".to_owned()),
            task_id: fixture.cli_task_id.clone(),
            result: AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
        };
        let last = AIAgentActionResult {
            id: AIAgentActionId::from("root-last".to_owned()),
            task_id: fixture.root_task_id.clone(),
            result: AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
        };
        let request = fixture.terminal.read(app, |terminal, ctx| {
            RequestInput::for_actions_results(
                vec![first, middle, last],
                Arc::from([]),
                terminal.active_session(),
                None,
                fixture.conversation_id,
                terminal.id(),
                ctx,
            )
        });

        let params = fixture.params(&request, app);

        assert_eq!(request.model_id, llm_id::encode("base", "model"));
        assert_eq!(params.byop_target_task_id, None);
        assert!(matches!(
            params.input.as_slice(),
            [
                AIAgentInput::ActionResult { result: first, .. },
                AIAgentInput::ActionResult { result: middle, .. },
                AIAgentInput::ActionResult { result: last, .. },
            ] if first.id.to_string() == "root-first"
                && middle.id.to_string() == "cli-middle"
                && last.id.to_string() == "root-last"
        ));
    });
}

#[test]
fn ordered_result_dedup_preserves_surviving_batches_and_empty_history_tasks() {
    with_routing_app(|fixture, app| {
        let result = AIAgentInput::ActionResult {
            result: AIAgentActionResult {
                id: AIAgentActionId::from("persisted-result".to_owned()),
                task_id: fixture.cli_task_id.clone(),
                result: AIAgentActionResultType::SuggestPrompt(SuggestPromptResult::Cancelled),
            },
            context: Arc::from([]),
        };
        let mut request = fixture.request(&fixture.root_task_id, app);
        let query = request.input_messages[0].1[0].clone();
        request.input_messages = vec![
            (fixture.cli_task_id.clone(), vec![result.clone()]),
            (fixture.root_task_id.clone(), vec![query]),
            (fixture.cli_task_id.clone(), vec![result]),
            (
                fixture.root_task_id.clone(),
                vec![AIAgentInput::ResumeConversation {
                    context: Arc::from([]),
                }],
            ),
        ];

        let removed = request.remove_action_results_by_tool_call_id(&HashSet::from([(
            fixture.cli_task_id.to_string(),
            "persisted-result".to_owned(),
        )]));
        let params = fixture.params(&request, app);

        assert_eq!(removed, 2);
        assert_eq!(request.input_messages.len(), 4);
        assert!(request.input_messages[0].1.is_empty());
        assert!(request.input_messages[2].1.is_empty());
        assert_eq!(params.byop_target_task_id, None);
        assert!(matches!(
            params.input.as_slice(),
            [AIAgentInput::UserQuery { query, .. }, AIAgentInput::ResumeConversation { .. }]
                if query == "检查当前终端"
        ));
    });
}
