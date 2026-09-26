use std::cell::RefCell;
use std::rc::Rc;

use warp_core::settings::Setting as _;
use warp_terminal::model::escape_sequences::{BRACKETED_PASTE_END, BRACKETED_PASTE_START};
use warpui::{App, AppContext, SingletonEntity, ViewContext};

use super::super::{AIBlockMetadata, RichContentMetadata, RichContentType};
use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentInput, ServerOutputId, UserQueryMode};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::agent_view::agent_input_footer::AgentInputFooterAction;
use crate::ai::blocklist::block::cli_controller::UserTakeOverReason;
use crate::ai::blocklist::model::{
    AIBlockModel, AIBlockOutputStatus, AIRequestType, OutputStatusUpdateCallback,
};
use crate::ai::blocklist::{AIBlock, ClientIdentifiers, InputConfig, InputType};
use crate::ai::llms::LLMId;
use crate::features::FeatureFlag;
use crate::settings::AISettings;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::terminal::model::ansi::{BootstrappedValue, Handler as _, InitShellValue};
use crate::terminal::shared_session::SharedSessionSource;
use crate::terminal::{CLIAgent, Event};
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[test]
fn deepseek_uses_bracketed_paste_submission() {
    assert_eq!(
        rich_input_submit_strategy(CLIAgent::DeepSeek),
        RichInputSubmitStrategy::BracketedPaste
    );
}

struct PendingAIBlockModel {
    conversation_id: AIConversationId,
    input: Vec<AIAgentInput>,
    model_id: LLMId,
}

impl PendingAIBlockModel {
    fn new(conversation_id: AIConversationId, input: Vec<AIAgentInput>) -> Self {
        Self {
            conversation_id,
            input,
            model_id: LLMId::from("fake-llm"),
        }
    }
}

impl AIBlockModel for PendingAIBlockModel {
    type View = AIBlock;

    fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
        AIBlockOutputStatus::Pending
    }

    fn server_output_id(&self, _app: &AppContext) -> Option<ServerOutputId> {
        None
    }

    fn model_id(&self, _app: &AppContext) -> Option<LLMId> {
        None
    }

    fn base_model<'a>(&'a self, _app: &'a AppContext) -> Option<&'a LLMId> {
        Some(&self.model_id)
    }

    fn inputs_to_render<'a>(&'a self, _app: &'a AppContext) -> &'a [AIAgentInput] {
        &self.input
    }

    fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
        Some(self.conversation_id)
    }

    fn on_updated_output(
        &self,
        _callback: OutputStatusUpdateCallback<AIBlock>,
        _ctx: &mut ViewContext<AIBlock>,
    ) {
    }

    fn request_type(&self, _app: &AppContext) -> AIRequestType {
        AIRequestType::Active
    }
}

fn simulate_user_started_long_running_command(view: &mut TerminalView) {
    {
        let mut model = view.model.lock();
        model.init_shell(InitShellValue {
            session_id: 0.into(),
            shell: "zsh".to_owned(),
            ..Default::default()
        });
        model.bootstrapped(BootstrappedValue {
            shell: "zsh".to_owned(),
            ..Default::default()
        });
        model.simulate_long_running_block("ssh localhost", "Password:");
    }
}

fn transition_to_user_handoff_state(
    view: &mut TerminalView,
    reason: UserTakeOverReason,
    ctx: &mut ViewContext<TerminalView>,
) -> AIConversationId {
    let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
        controller
            .try_enter_inline_agent_view(None, AgentViewEntryOrigin::LongRunningCommand, ctx)
            .expect("inline agent view should create a conversation")
    });
    view.model
        .lock()
        .block_list_mut()
        .active_block_mut()
        .set_is_agent_tagged_in(true);

    let task_id = TaskId::new("test-task".to_owned());
    view.model
        .lock()
        .block_list_mut()
        .active_block_mut()
        .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
        .expect("tagged-in command should transition to agent-monitored");

    view.cli_subagent_controller.update(ctx, |controller, ctx| {
        controller.switch_control_to_user(reason, ctx);
    });

    conversation_id
}

fn insert_pending_ai_block(
    view: &mut TerminalView,
    conversation_id: AIConversationId,
    ctx: &mut ViewContext<TerminalView>,
) {
    let ai_block_model = Rc::new(PendingAIBlockModel::new(
        conversation_id,
        vec![AIAgentInput::UserQuery {
            query: "help with this running command".to_owned(),
            context: vec![].into(),
            static_query_type: None,
            referenced_attachments: Default::default(),
            user_query_mode: UserQueryMode::default(),
            running_command: None,
            intended_agent: None,
        }],
    ));
    let ai_block = ctx.add_typed_action_view(|ctx| {
        AIBlock::new(
            ai_block_model.clone(),
            view.model.clone(),
            ClientIdentifiers {
                client_exchange_id: Default::default(),
                conversation_id,
                response_stream_id: None,
            },
            view.ai_controller.clone(),
            None,
            None,
            view.ai_action_model.clone(),
            view.ai_context_model.clone(),
            view.find_model.clone(),
            view.active_session.clone(),
            &view.cli_subagent_controller,
            &view.model_events_handle,
            view.agent_view_controller.clone(),
            view.ambient_agent_view_model.clone(),
            view.view_handle.clone(),
            view.id(),
            ctx,
        )
    });

    view.insert_rich_content(
        Some(RichContentType::AIBlock),
        ai_block.clone(),
        Some(RichContentMetadata::AIBlock(AIBlockMetadata {
            exchange_id: Default::default(),
            conversation_id,
            ai_block_handle: ai_block,
        })),
        RichContentInsertionPosition::Append {
            insert_below_long_running_block: false,
        },
        ctx,
    );
}

#[test]
fn use_agent_footer_renders_for_manual_handoff_even_when_user_command_footer_setting_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .should_render_use_agent_footer_for_user_commands
                .set_value(false, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            simulate_user_started_long_running_command(view);

            view.maybe_show_use_agent_footer_in_blocklist(ctx);
            {
                let model = view.model.lock();
                assert!(!view.should_render_use_agent_footer(&model, ctx));
                let active_block_index = model.block_list().active_block_index();
                assert!(
                    model
                        .block_list()
                        .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                        .is_none()
                );
            }

            transition_to_user_handoff_state(view, UserTakeOverReason::Manual, ctx);

            view.maybe_show_use_agent_footer_in_blocklist(ctx);
            let model = view.model.lock();
            assert!(view.should_render_use_agent_footer(&model, ctx));
            let active_block_index = model.block_list().active_block_index();
            let rendered_footer_view_id = model
                .block_list()
                .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                .map(|(_, item)| item.view_id);
            assert_eq!(rendered_footer_view_id, Some(view.use_agent_footer.id()));
        });
    })
}

#[test]
fn use_agent_footer_renders_for_manual_handoff_when_unfinished_ai_block_remains() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            simulate_user_started_long_running_command(view);

            let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_inline_agent_view(
                        None,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                    .expect("inline agent view should create a conversation")
            });
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);
            let task_id = TaskId::new("test-task".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("tagged-in command should transition to agent-monitored");

            insert_pending_ai_block(view, conversation_id, ctx);
            assert!(view.active_ai_block(ctx).is_some());

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(UserTakeOverReason::Manual, ctx);
            });
        });

        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();
            assert!(view.should_render_use_agent_footer(&model, ctx));
            let active_block_index = model.block_list().active_block_index();
            let rendered_footer_view_id = model
                .block_list()
                .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                .map(|(_, item)| item.view_id);
            assert_eq!(rendered_footer_view_id, Some(view.use_agent_footer.id()));
        });
    })
}

/// During the setup phase of an ambient-agent shared session — LRCs
/// running before any CLI agent has started — the use-agent footer must stay
/// hidden.
#[test]
fn use_agent_footer_hidden_during_ambient_agent_setup_lrc() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            simulate_user_started_long_running_command(view);

            // Ambient-agent setup phase: ambient source type set, LRC running,
            // NO CLIAgentSession registered yet.
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(None));
            assert!(view.model.lock().is_shared_ambient_agent_session());
            assert!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.id())
                    .is_none(),
                "precondition: no CLI agent session yet",
            );

            view.maybe_show_use_agent_footer_in_blocklist(ctx);

            let model = view.model.lock();
            assert!(
                !view.should_render_use_agent_footer(&model, ctx),
                "footer should be hidden during ambient-agent setup LRCs",
            );
            let active_block_index = model.block_list().active_block_index();
            assert!(
                model
                    .block_list()
                    .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                    .is_none(),
                "footer rich content should not be in the blocklist during ambient setup",
            );
        });
    })
}

/// When viewing a shared ambient-agent session whose sharer is
/// running a CLI agent, the CLI agent footer should still render.
#[test]
fn cli_agent_footer_renders_for_viewer_of_shared_ambient_agent_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            simulate_user_started_long_running_command(view);

            // Mark the model as a shared ambient agent session, mirroring
            // what the viewer's terminal manager does on `JoinedSuccessfully`.
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(None));
            assert!(view.model.lock().is_shared_ambient_agent_session());

            // Inject the CLI agent session state that an old shared-session viewer
            // would have received from the sharer.
            let view_id = view.id();
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        listener: None,
                        plugin_version: None,
                        remote_host: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                        should_auto_toggle_input: false,
                    },
                    ctx,
                );
            });

            view.maybe_show_use_agent_footer_in_blocklist(ctx);

            let model = view.model.lock();
            assert!(
                view.should_render_use_agent_footer(&model, ctx),
                "footer should render for viewer of shared ambient agent session with CLI agent",
            );
            let active_block_index = model.block_list().active_block_index();
            let rendered_footer_view_id = model
                .block_list()
                .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                .map(|(_, item)| item.view_id);
            assert_eq!(rendered_footer_view_id, Some(view.use_agent_footer.id()));
        });
    })
}

#[test]
fn cli_agent_footer_does_not_render_for_warp_tui_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            simulate_user_started_long_running_command(view);

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.id(),
                    CLIAgentSession {
                        agent: CLIAgent::WarpTui,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        listener: None,
                        plugin_version: None,
                        remote_host: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                        should_auto_toggle_input: false,
                    },
                    ctx,
                );
            });

            view.maybe_show_use_agent_footer_in_blocklist(ctx);

            let model = view.model.lock();
            assert!(!view.should_render_use_agent_footer(&model, ctx));
            let active_block_index = model.block_list().active_block_index();
            assert!(
                model
                    .block_list()
                    .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                    .is_none()
            );
        });
    })
}
#[test]
fn test_rich_input_submit_strategy_for_oh_my_pi() {
    assert_eq!(
        rich_input_submit_strategy(CLIAgent::OhMyPi),
        RichInputSubmitStrategy::BracketedPaste
    );
}

/// Hermes interprets embedded newlines as submit actions when text is written
/// directly. Bracketed paste preserves them as part of one input payload.
#[test]
fn test_rich_input_submit_strategy_for_hermes_uses_bracketed_paste() {
    assert_eq!(
        rich_input_submit_strategy(CLIAgent::Hermes),
        RichInputSubmitStrategy::BracketedPaste
    );
}

fn register_cli_input_test_session(
    view: &mut TerminalView,
    agent: CLIAgent,
    ctx: &mut ViewContext<TerminalView>,
) -> Uuid {
    {
        let mut model = view.model.lock();
        // 更换 CLI 必须先结束前一命令，否则夹具会把两条命令拼进同一活动 block。
        if model
            .block_list()
            .active_block()
            .is_active_and_long_running()
        {
            model.finish_block();
        }
        model.simulate_long_running_block(agent.command_prefix(), "");
        assert!(
            agent.matches_command(
                &model
                    .block_list()
                    .active_block()
                    .command_with_secrets_obfuscated(false),
                None
            )
        );
    }
    let view_id = view.view_id;
    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
        sessions.set_session(
            view_id,
            CLIAgentSession {
                agent,
                status: CLIAgentSessionStatus::InProgress,
                session_context: CLIAgentSessionContext::default(),
                input_state: CLIAgentInputState::Closed,
                should_auto_toggle_input: false,
                listener: None,
                remote_host: None,
                plugin_version: None,
                draft_text: None,
                custom_command_prefix: None,
                received_rich_notification: false,
            },
            ctx,
        );
        sessions.input_generation(view_id).unwrap()
    })
}

fn prepare_rich_cli_test(app: &mut App, agent: CLIAgent) -> ViewHandle<TerminalView> {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| crate::workspace::ToastStack);
    FeatureFlag::CLIAgentRichInput.set_enabled(true);
    AISettings::handle(app).update(app, |settings, ctx| {
        settings
            .auto_dismiss_rich_input_after_submit
            .set_value(false, ctx)
            .unwrap();
        settings.submit_on_ctrl_enter.set_value(true, ctx).unwrap();
    });
    let terminal = add_window_with_terminal(app, None);
    terminal.update(app, |view, ctx| {
        register_cli_input_test_session(view, agent, ctx);
        view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
    });
    terminal
}

fn collect_cli_test_writes(
    app: &mut App,
    terminal: &ViewHandle<TerminalView>,
) -> Rc<RefCell<Vec<Vec<u8>>>> {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let collected = writes.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(terminal, move |_, event, _| {
            if let Event::WriteBytesToPty { bytes } = event {
                collected.borrow_mut().push(bytes.to_vec());
            }
        });
    });
    writes
}

fn prepare_hook_bound_ssh_input(app: &mut App) -> ViewHandle<TerminalView> {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| crate::workspace::ToastStack);
    FeatureFlag::CodexPlugin.set_enabled(true);
    let terminal = add_window_with_terminal(app, None);
    terminal.update(app, |view, ctx| {
        view.model
            .lock()
            .simulate_long_running_block("ssh -F /private/tmp/isolated-config -tt host direct", "");
        {
            let model = view.model.lock();
            assert!(view.detect_cli_agent_from_model(&model, ctx).is_none());
        }
        view.handle_cli_agent_notification(
            Some("warp://cli-agent"),
            r#"{"v":1,"agent":"codex","event":"session_start","session_id":"native-ssh-session","plugin_version":"0.4.0"}"#,
            ctx,
        );
    });
    terminal
}

#[test]
fn ssh_native_hook_allows_utf8_paste_without_submitting() {
    App::test((), |mut app| async move {
        let terminal = prepare_hook_bound_ssh_input(&mut app);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            let generation = CLIAgentSessionsModel::as_ref(ctx)
                .input_generation(view.view_id)
                .unwrap();
            assert!(view.insert_text_into_cli_agent_pty("中文验收\n第二行", generation, ctx));
        });
        assert_eq!(
            *writes.borrow(),
            vec!["\u{1b}[200~中文验收\n第二行\u{1b}[201~".as_bytes().to_vec()]
        );
    });
}

#[test]
fn ssh_native_hook_cannot_authorize_a_later_command_block() {
    App::test((), |mut app| async move {
        let terminal = prepare_hook_bound_ssh_input(&mut app);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            let generation = CLIAgentSessionsModel::as_ref(ctx)
                .input_generation(view.view_id)
                .unwrap();
            {
                let mut model = view.model.lock();
                model.finish_block();
                model.simulate_long_running_block("ssh -tt different-host sh", "");
            }
            assert!(!view.insert_text_into_cli_agent_pty("不得写入后续命令", generation, ctx));
        });
        assert!(writes.borrow().is_empty());
    });
}

#[test]
fn ssh_input_generation_rotation_rejects_stale_paste_but_accepts_fresh_input() {
    App::test((), |mut app| async move {
        let terminal = prepare_hook_bound_ssh_input(&mut app);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            let stale_generation = CLIAgentSessionsModel::as_ref(ctx)
                .input_generation(view.view_id)
                .unwrap();
            let generation = CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.observe_ctrl_c_write(view.view_id, ctx);
                sessions.input_generation(view.view_id).unwrap()
            });
            assert!(!view.insert_text_into_cli_agent_pty("过期输入", stale_generation, ctx));
            assert!(view.insert_text_into_cli_agent_pty("新的输入", generation, ctx));
        });
        assert_eq!(
            *writes.borrow(),
            vec!["\u{1b}[200~新的输入\u{1b}[201~".as_bytes().to_vec()]
        );
    });
}

#[test]
fn ssh_native_hook_cannot_authorize_a_replacement_native_session() {
    App::test((), |mut app| async move {
        let terminal = prepare_hook_bound_ssh_input(&mut app);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            let mut replacement = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .unwrap()
                .clone();
            replacement.session_context.session_id = Some("replacement-native-session".to_owned());
            let generation = CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(view.view_id, replacement, ctx);
                sessions.input_generation(view.view_id).unwrap()
            });
            assert!(!view.insert_text_into_cli_agent_pty("不得沿用旧会话绑定", generation, ctx));
        });
        assert!(writes.borrow().is_empty());
    });
}

#[test]
fn ssh_session_label_without_native_hook_cannot_authorize_paste() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Codex);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.finish_block();
                model.simulate_long_running_block(
                    "ssh -F /private/tmp/isolated-config -tt host direct",
                    "",
                );
            }
            let generation = CLIAgentSessionsModel::as_ref(ctx)
                .input_generation(view.view_id)
                .unwrap();
            assert!(!view.insert_text_into_cli_agent_pty("仅标签不可放行", generation, ctx));
        });
        assert!(writes.borrow().is_empty());
    });
}

fn submit_cli_test_input(terminal: &ViewHandle<TerminalView>, app: &mut App, text: &str) {
    terminal.update(app, |view, ctx| {
        view.input.update(ctx, |input, ctx| {
            input.replace_buffer_content(text, ctx);
        });
    });
    terminal.update(app, |view, ctx| {
        view.input.update(ctx, |input, ctx| {
            input.input_ctrl_enter(ctx);
        });
    });
}

fn test_image(data: &str, filename: &str) -> ImageContext {
    ImageContext {
        data: data.to_owned(),
        mime_type: "image/png".to_owned(),
        file_name: filename.to_owned(),
        is_figma: false,
    }
}

#[test]
fn grok_rich_input_keeps_multiline_utf8_without_injecting_paste_or_enter() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Grok);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        submit_cli_test_input(&terminal, &mut app, "第一行：修复 bug\n第二行：添加测试 🦀");
        assert!(writes.borrow().is_empty());
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "第一行：修复 bug\n第二行：添加测试 🦀"
            );
            assert!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .session_context
                    .query
                    .is_none()
            );
            assert!(view.has_active_cli_agent_input_session(ctx));
        });
    });
}

#[test]
fn malformed_image_keeps_rich_draft_and_entire_attachment_batch() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Codex);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            view.ai_context_model.update(ctx, |model, ctx| {
                model.append_pending_images(
                    vec![
                        test_image("aGVsbG8=", "first.png"),
                        test_image("%%%", "bad.png"),
                    ],
                    ctx,
                )
            });
        });
        submit_cli_test_input(&terminal, &mut app, "保留完整草稿");
        Timer::after(Duration::from_millis(100)).await;
        assert!(writes.borrow().is_empty());
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "保留完整草稿");
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 2);
            assert!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .session_context
                    .query
                    .is_none()
            );
        });
    });
}

#[test]
fn delayed_submit_does_not_clear_a_reedited_identical_draft() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        submit_cli_test_input(&terminal, &mut app, "same draft 中文");
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("new draft", ctx);
                input.replace_buffer_content("same draft 中文", ctx);
                input.input_ctrl_enter(ctx);
            });
        });
        Timer::after(Duration::from_millis(100)).await;
        assert_eq!(
            *writes.borrow(),
            vec!["same draft 中文".as_bytes().to_vec(), b"\r".to_vec()]
        );
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "same draft 中文")
        });
    });
}

#[test]
fn delayed_submit_keeps_new_attachments_and_new_text() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        submit_cli_test_input(&terminal, &mut app, "first");
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("second", ctx)
            });
            view.ai_context_model.update(ctx, |model, ctx| {
                model.append_pending_images(vec![test_image("aGVsbG8=", "second.png")], ctx)
            });
        });
        Timer::after(Duration::from_millis(100)).await;
        assert_eq!(*writes.borrow(), vec![b"first".to_vec(), b"\r".to_vec()]);
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "second");
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 1);
        });
    });
}

#[test]
fn cancellation_prevents_the_real_delayed_enter_and_keeps_draft() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        submit_cli_test_input(&terminal, &mut app, "取消后不得提交");
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.observe_ctrl_c_write(view.view_id, ctx)
            });
        });
        Timer::after(Duration::from_millis(100)).await;
        assert_eq!(*writes.borrow(), vec!["取消后不得提交".as_bytes().to_vec()]);
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "取消后不得提交")
        });
    });
}

#[test]
fn replacement_session_prevents_old_delayed_enter() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        submit_cli_test_input(&terminal, &mut app, "old");
        terminal.update(&mut app, |view, ctx| {
            register_cli_input_test_session(view, CLIAgent::Codex, ctx);
        });
        Timer::after(Duration::from_millis(100)).await;
        assert_eq!(*writes.borrow(), vec![b"old".to_vec()]);
    });
}

#[test]
fn pty_control_rejection_keeps_draft_and_does_not_emit_prompt_submit() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Codex);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, _| {
            view.model.lock().block_list_mut().active_block_mut().set_agent_interaction_mode(
                crate::terminal::model::block::AgentInteractionMetadata::new(None, AIConversationId::new(), None,
                    Some(crate::ai::blocklist::block::cli_controller::LongRunningCommandControlState::Agent { is_blocked: false, should_hide_responses: false }), false, false));
        });
        submit_cli_test_input(&terminal, &mut app, "must remain");
        assert!(writes.borrow().is_empty());
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "must remain");
            assert!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .session_context
                    .query
                    .is_none()
            );
        });
    });
}

#[test]
fn rejected_second_shell_submit_keeps_explicit_shell_mode() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        submit_cli_test_input(&terminal, &mut app, "first");
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("echo 中文", ctx);
                input.ai_input_model().update(ctx, |model, ctx| {
                    model.set_input_config(
                        crate::ai::blocklist::InputConfig {
                            input_type: crate::ai::blocklist::InputType::Shell,
                            is_locked: true,
                        },
                        false,
                        None,
                        ctx,
                    );
                });
                input.input_ctrl_enter(ctx);
            });
        });
        Timer::after(Duration::from_millis(100)).await;
        terminal.read(&app, |view, ctx| {
            let input = view.input.as_ref(ctx);
            assert_eq!(input.buffer_text(ctx), "echo 中文");
            assert!(!input.ai_input_model().as_ref(ctx).input_type().is_ai());
            assert!(input.ai_input_model().as_ref(ctx).is_input_type_locked());
        });
    });
}

#[test]
fn file_picker_producer_uses_original_generation_and_preserves_failed_insert() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        let terminal = add_window_with_terminal(&mut app, None);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        let original = terminal.update(&mut app, |view, ctx| {
            register_cli_input_test_session(view, CLIAgent::Codex, ctx)
        });
        terminal.update(&mut app, |view, ctx| {
            register_cli_input_test_session(view, CLIAgent::Claude, ctx);
            let footer = view.use_agent_footer.as_ref(ctx).agent_input_footer.clone();
            footer.update(ctx, |footer, ctx| footer.handle_action(&crate::ai::blocklist::agent_view::agent_input_footer::AgentInputFooterAction::InsertFilePath { path: "/tmp/old.txt".to_owned(), generation: original }, ctx));
        });
        assert!(writes.borrow().is_empty());
        terminal.update(&mut app, |view, ctx| {
            let generation = CLIAgentSessionsModel::as_ref(ctx).input_generation(view.view_id).unwrap();
            assert!(view.begin_cli_agent_text_submit(generation, ctx));
            let footer = view.use_agent_footer.as_ref(ctx).agent_input_footer.clone();
            footer.update(ctx, |footer, ctx| footer.handle_action(&crate::ai::blocklist::agent_view::agent_input_footer::AgentInputFooterAction::InsertFilePath { path: "/tmp/kept.txt".to_owned(), generation }, ctx));
        });
        assert!(writes.borrow().is_empty());
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .draft_text
                    .as_deref(),
                Some("/tmp/kept.txt ")
            )
        });
    });
}

#[cfg(feature = "voice_input")]
#[test]
fn voice_transcription_producer_preserves_multiline_and_drops_stale_results() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        let terminal = add_window_with_terminal(&mut app, None);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        let original = terminal.update(&mut app, |view, ctx| {
            register_cli_input_test_session(view, CLIAgent::Codex, ctx)
        });
        terminal.update(&mut app, |view, ctx| {
            let footer = view.use_agent_footer.as_ref(ctx).agent_input_footer.clone();
            footer.update(ctx, |footer, ctx| {
                footer.apply_cli_transcribed_voice_input(
                    Ok("中文第一行\nEnglish second line".to_owned()),
                    original,
                    ctx,
                )
            });
        });
        assert_eq!(
            *writes.borrow(),
            vec![
                "\x1b[200~中文第一行\nEnglish second line\x1b[201~"
                    .as_bytes()
                    .to_vec()
            ]
        );
        terminal.update(&mut app, |view, ctx| {
            register_cli_input_test_session(view, CLIAgent::Claude, ctx);
            let footer = view.use_agent_footer.as_ref(ctx).agent_input_footer.clone();
            footer.update(ctx, |footer, ctx| {
                footer.apply_cli_transcribed_voice_input(Ok("stale".to_owned()), original, ctx)
            });
        });
        assert_eq!(writes.borrow().len(), 1);
    });
}

#[test]
fn grok_clipboard_and_rich_images_are_gated_without_clearing_draft() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Grok);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            view.ai_context_model.update(ctx, |model, ctx| {
                model.append_pending_images(vec![test_image("aGVsbG8=", "grok.png")], ctx)
            });
            assert!(!view.paste_clipboard_image_to_cli_agent(ctx));
        });
        submit_cli_test_input(&terminal, &mut app, "keep Grok draft");
        assert!(writes.borrow().is_empty());
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "keep Grok draft");
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 1);
        });
    });
}

#[test]
fn windows_image_policy_distinguishes_claude_codex_and_grok() {
    assert_eq!(
        cli_agent_paste_keystroke_bytes(CLIAgent::Claude, true),
        Some(vec![0x1b, b'v'])
    );
    assert_eq!(
        cli_agent_paste_keystroke_bytes(CLIAgent::Codex, true),
        Some([BRACKETED_PASTE_START, BRACKETED_PASTE_END].concat())
    );
    assert_eq!(cli_agent_paste_keystroke_bytes(CLIAgent::Grok, true), None);
    assert_eq!(cli_agent_paste_keystroke_bytes(CLIAgent::Grok, false), None);
}

#[test]
fn input_submission_lease_survives_old_callbacks_without_blocking_a_new_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let first = register_cli_input_test_session(view, CLIAgent::Claude, ctx);
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                assert!(sessions.begin_input_submission(view.view_id, first));
                assert!(!sessions.begin_input_submission(view.view_id, first));
            });
            let second = register_cli_input_test_session(view, CLIAgent::Claude, ctx);
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                assert!(sessions.begin_input_submission(view.view_id, second));
                sessions.finish_input_submission(view.view_id, first);
                assert!(!sessions.begin_input_submission(view.view_id, second));
                sessions.finish_input_submission(view.view_id, second);
                assert!(sessions.begin_input_submission(view.view_id, second));
            });
        });
    });
}

#[test]
fn review_delivery_rejects_busy_or_unavailable_pty_and_preserves_batch() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        let _review = FeatureFlag::HoaCodeReview.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        let review = crate::ai::agent::AgentReviewCommentBatch {
            comments: Vec::new(),
            diff_set: Default::default(),
        };
        terminal.update(&mut app, |view, ctx| {
            assert!(
                view.send_review_to_cli_agent_or_rich_input(&review, ctx)
                    .is_err()
            );
            let generation = register_cli_input_test_session(view, CLIAgent::Codex, ctx);
            assert!(view.begin_cli_agent_text_submit(generation, ctx));
            assert!(
                view.send_review_to_cli_agent_or_rich_input(&review, ctx)
                    .is_err()
            );
            assert!(
                view.try_send_text_to_cli_agent_or_rich_input("file context\n中文".to_owned(), ctx)
                    .is_none()
            );
        });
        assert!(writes.borrow().is_empty());
    });
}

#[test]
fn closed_rich_input_file_context_is_one_literal_paste_without_enter() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        let _review = FeatureFlag::HoaCodeReview.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            register_cli_input_test_session(view, CLIAgent::Codex, ctx);
            assert!(
                view.try_send_text_to_cli_agent_or_rich_input(
                    "中文文件\n`rm -rf` is literal".to_owned(),
                    ctx
                )
                .is_some()
            );
        });
        assert_eq!(
            *writes.borrow(),
            vec![
                "\x1b[200~中文文件\n`rm -rf` is literal\x1b[201~"
                    .as_bytes()
                    .to_vec()
            ]
        );
    });
}

#[test]
fn closed_unowned_grok_context_does_not_open_composer_or_write_to_pty() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        let _review = FeatureFlag::HoaCodeReview.override_enabled(true);
        let _rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            register_cli_input_test_session(view, CLIAgent::Grok, ctx);
            CLIAgentSessionsModel::handle(ctx).update(ctx, |model, _| {
                model.set_draft(view.view_id, "保留普通会话草稿".into());
            });
            assert!(
                view.try_send_text_to_cli_agent_or_rich_input("中文文件上下文".into(), ctx)
                    .is_none()
            );
            assert!(!view.is_cli_agent_rich_input_open(ctx));
            assert_eq!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .draft_text
                    .as_deref(),
                Some("保留普通会话草稿")
            );
        });
        assert!(writes.borrow().is_empty());
    });
}

#[test]
fn slash_skill_selection_uses_the_active_cli_native_prefix() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Codex);
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.handle_slash_commands_menu_event(
                    &crate::terminal::input::slash_commands::SlashCommandsEvent::SelectedSkill {
                        name: "review-local".to_owned(),
                        reference: ai::skills::SkillReference::BundledSkillId(
                            "review-local".to_owned(),
                        ),
                    },
                    ctx,
                );
            });
        });
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "$review-local ")
        });
    });
}

#[test]
fn native_notification_authorization_opens_manual_steps_without_writing_to_cli() {
    use crate::ai::blocklist::agent_view::agent_input_footer::AgentInputFooterAction;
    use crate::terminal::cli_agent_sessions::plugin_manager::PluginModalKind;
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| crate::workspace::ToastStack);
        let terminal = add_window_with_terminal(&mut app, None);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        let opened = Rc::new(RefCell::new(Vec::new()));
        let captured = opened.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::OpenPluginInstructionsPane(agent, kind) = event {
                    captured.borrow_mut().push((*agent, *kind));
                }
            });
        });
        terminal.update(&mut app, |view, ctx| {
            register_cli_input_test_session(view, CLIAgent::Codex, ctx);
            let footer = view.use_agent_footer.as_ref(ctx).agent_input_footer.clone();
            footer.update(ctx, |footer, ctx| {
                footer.handle_action(
                    &AgentInputFooterAction::OpenNativeAuthorizationInstructions,
                    ctx,
                );
            });
        });
        assert_eq!(
            *opened.borrow(),
            vec![(CLIAgent::Codex, PluginModalKind::NativeAuthorization)]
        );
        assert!(writes.borrow().is_empty());
    });
}

#[path = "input_approval_guard_tests.rs"]
mod input_approval_guard_tests;

#[path = "file_submission_tests.rs"]
mod file_submission_tests;

#[test]
fn rich_cli_picker_without_plugin_uses_attachment_flow_and_preserves_shell_lock() {
    for agent in [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok] {
        App::test((), move |mut app| async move {
            let terminal = prepare_rich_cli_test(&mut app, agent);
            let writes = collect_cli_test_writes(&mut app, &terminal);
            let selected = Rc::new(RefCell::new(0));
            let captured = selected.clone();
            let footer = terminal.read(&app, |view, ctx| {
                view.use_agent_footer.as_ref(ctx).agent_input_footer.clone()
            });
            app.update(|ctx| {
                ctx.subscribe_to_view(&footer, move |_, event, _| {
                    if matches!(event, AgentInputFooterEvent::SelectFile) {
                        *captured.borrow_mut() += 1;
                    }
                });
            });
            terminal.update(&mut app, |view, ctx| {
                let session = CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap();
                assert!(session.listener.is_none());
                assert!(!session.received_rich_notification);
                view.input.update(ctx, |input, ctx| {
                    input.ai_input_model().update(ctx, |model, ctx| {
                        model.set_input_config(
                            InputConfig {
                                input_type: InputType::Shell,
                                is_locked: true,
                            },
                            false,
                            None,
                            ctx,
                        );
                    });
                    input.insert_into_cli_agent_rich_input("保留草稿", ctx);
                });
                footer.update(ctx, |footer, ctx| {
                    footer.handle_action(&AgentInputFooterAction::SelectFile, ctx)
                });
            });
            // 无头选择器返回取消；验证真实 footer 事件接到了附件入口且没有立即写入 PTY。
            Timer::after(Duration::from_millis(50)).await;
            assert_eq!(*selected.borrow(), 1);
            assert!(writes.borrow().is_empty());
            terminal.read(&app, |view, ctx| {
                let input = view.input.as_ref(ctx);
                assert_eq!(input.buffer_text(ctx), "保留草稿");
                assert!(!input.ai_input_model().as_ref(ctx).input_type().is_ai());
                assert!(input.ai_input_model().as_ref(ctx).is_input_type_locked());
                assert!(view.ai_context_model.as_ref(ctx).pending_files().is_empty());
            });
        });
    }
}
