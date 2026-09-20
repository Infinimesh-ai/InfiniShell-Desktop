use super::*;
use crate::view_components::DismissibleToastStack;
use crate::workspace::ToastStackEvent;
use warpui::platform::WindowStyle;

fn native_event(
    view: &mut TerminalView,
    event: CLIAgentEventType,
    ctx: &mut ViewContext<TerminalView>,
) {
    let agent = CLIAgentSessionsModel::as_ref(ctx)
        .session(view.view_id)
        .unwrap()
        .agent;
    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
        sessions.update_from_event(
            view.view_id,
            &CLIAgentEvent {
                v: 1,
                agent,
                event,
                session_id: Some("approval-guard-native-session".to_owned()),
                cwd: None,
                project: None,
                payload: CLIAgentEventPayload {
                    query: Some("existing native prompt".to_owned()),
                    turn_id: Some("approval-guard-native-turn".to_owned()),
                    prompt_id: Some("approval-guard-native-turn".to_owned()),
                    ..Default::default()
                },
                source: CLIAgentEventSource::RichPlugin,
            },
            ctx,
        );
    });
}

fn prepare_guard_test(app: &mut App, agent: CLIAgent) -> ViewHandle<TerminalView> {
    let terminal = prepare_rich_cli_test(app, agent);
    // 模拟用户手动保持编辑器打开，避免自动切换把发送守卫测试变成关闭状态测试。
    AISettings::handle(app).update(app, |settings, ctx| {
        settings
            .auto_toggle_rich_input
            .set_value(false, ctx)
            .unwrap();
    });
    terminal
}

fn collect_toasts(app: &mut App) -> Rc<RefCell<Vec<DismissibleToast<WorkspaceAction>>>> {
    let toasts = Rc::new(RefCell::new(Vec::new()));
    let collected = toasts.clone();
    let stack = ToastStack::handle(app);
    app.update(|ctx| {
        ctx.subscribe_to_model(&stack, move |_, event, _| {
            if let ToastStackEvent::AddEphemeralToast { toast, .. } = event {
                collected.borrow_mut().push(toast.clone());
            }
        });
    });
    toasts
}

#[test]
fn native_permission_waiting_keeps_draft_images_and_existing_query_for_all_three_agents() {
    for agent in [CLIAgent::Claude, CLIAgent::Codex, CLIAgent::Grok] {
        App::test((), move |mut app| async move {
            let terminal = prepare_guard_test(&mut app, agent);
            let writes = collect_cli_test_writes(&mut app, &terminal);
            terminal.update(&mut app, |view, ctx| {
                native_event(view, CLIAgentEventType::PromptSubmit, ctx);
                native_event(view, CLIAgentEventType::PermissionRequest, ctx);
                view.ai_context_model.update(ctx, |model, ctx| {
                    model.append_pending_images(vec![test_image("aGVsbG8=", "keep.png")], ctx);
                });
            });
            submit_cli_test_input(&terminal, &mut app, "不要批准\n保留这段追加内容");
            Timer::after(Duration::from_millis(100)).await;
            assert!(writes.borrow().is_empty());
            terminal.read(&app, |view, ctx| {
                let session = CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap();
                assert!(matches!(
                    session.status,
                    CLIAgentSessionStatus::Blocked { .. }
                ));
                assert_eq!(
                    session.session_context.query.as_deref(),
                    Some("existing native prompt")
                );
                assert_eq!(
                    view.input.as_ref(ctx).buffer_text(ctx),
                    "不要批准\n保留这段追加内容"
                );
                assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 1);
            });
        });
    }
}

#[test]
fn permission_arriving_after_text_prevents_delayed_enter_without_changing_generation() {
    App::test((), |mut app| async move {
        let terminal = prepare_guard_test(&mut app, CLIAgent::Claude);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            native_event(view, CLIAgentEventType::PromptSubmit, ctx)
        });
        submit_cli_test_input(&terminal, &mut app, "待发送文本");
        terminal.update(&mut app, |view, ctx| {
            let generation = CLIAgentSessionsModel::as_ref(ctx).input_generation(view.view_id);
            native_event(view, CLIAgentEventType::PermissionRequest, ctx);
            assert_eq!(
                CLIAgentSessionsModel::as_ref(ctx).input_generation(view.view_id),
                generation
            );
        });
        Timer::after(Duration::from_millis(100)).await;
        assert_eq!(*writes.borrow(), vec!["待发送文本".as_bytes().to_vec()]);
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "待发送文本");
            assert_eq!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .session_context
                    .query
                    .as_deref(),
                Some("existing native prompt")
            );
        });
    });
}

#[test]
fn permission_arriving_after_mode_prefix_prevents_remaining_text_and_enter() {
    App::test((), |mut app| async move {
        let terminal = prepare_guard_test(&mut app, CLIAgent::Claude);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            native_event(view, CLIAgentEventType::PromptSubmit, ctx)
        });
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("echo guarded", ctx);
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
        terminal.update(&mut app, |view, ctx| {
            native_event(view, CLIAgentEventType::PermissionRequest, ctx)
        });
        Timer::after(Duration::from_millis(100)).await;
        assert_eq!(*writes.borrow(), vec![b"!".to_vec()]);
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "echo guarded")
        });
    });
}

#[test]
fn permission_reply_allows_an_explicit_later_submit_without_replaying_the_rejected_draft() {
    App::test((), |mut app| async move {
        let terminal = prepare_guard_test(&mut app, CLIAgent::Codex);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            native_event(view, CLIAgentEventType::PromptSubmit, ctx);
            native_event(view, CLIAgentEventType::PermissionRequest, ctx);
        });
        submit_cli_test_input(&terminal, &mut app, "rejected draft");
        terminal.update(&mut app, |view, ctx| {
            native_event(view, CLIAgentEventType::PermissionReplied, ctx)
        });
        assert!(writes.borrow().is_empty());
        submit_cli_test_input(&terminal, &mut app, "explicit later draft");
        assert_eq!(
            *writes.borrow(),
            vec![
                b"\x1b[200~explicit later draft\x1b[201~".to_vec(),
                b"\r".to_vec()
            ]
        );
    });
}

#[test]
fn blocked_file_review_and_direct_followup_routes_never_write_to_the_pty() {
    App::test((), |mut app| async move {
        let terminal = prepare_guard_test(&mut app, CLIAgent::Codex);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            native_event(view, CLIAgentEventType::PromptSubmit, ctx);
            native_event(view, CLIAgentEventType::PermissionRequest, ctx);
            view.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::Manual, ctx);
            assert!(
                view.try_send_text_to_cli_agent_or_rich_input("中文路径/file.rs".to_owned(), ctx)
                    .is_none()
            );
            let review = crate::ai::agent::AgentReviewCommentBatch {
                comments: Vec::new(),
                diff_set: Default::default(),
            };
            assert!(
                view.send_review_to_cli_agent_or_rich_input(&review, ctx)
                    .is_err()
            );
            assert!(
                view.send_diff_hunk_to_cli_agent_or_rich_input("a.rs", 1, 2, 1, 0, ctx)
                    .is_none()
            );
            #[cfg(feature = "local_tty")]
            view.submit_text_to_cli_agent_pty("external followup".to_owned(), ctx);
        });
        assert!(writes.borrow().is_empty());
    });
}

#[test]
fn grok_copy_is_explicit_and_preserves_exact_unsent_utf8_with_shell_prefix() {
    App::test((), |mut app| async move {
        let terminal = prepare_guard_test(&mut app, CLIAgent::Grok);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        let toasts = collect_toasts(&mut app);
        app.update(|ctx| {
            ctx.clipboard().write(ClipboardContent::plain_text(
                "existing clipboard".to_owned(),
            ))
        });
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("echo 中文\n/skill path with spaces", ctx);
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
        assert!(writes.borrow().is_empty());
        app.update(|ctx| assert_eq!(ctx.clipboard().read().plain_text, "existing clipboard"));
        let callback = toasts
            .borrow()
            .last()
            .unwrap()
            .on_body_click
            .as_ref()
            .unwrap()
            .clone();
        let (_, stack) = app.add_window(WindowStyle::NotStealFocus, |_| {
            DismissibleToastStack::<WorkspaceAction>::new(Duration::from_secs(30))
        });
        stack.update(&mut app, |_, ctx| callback(ctx));
        app.update(|ctx| {
            assert_eq!(
                ctx.clipboard().read().plain_text,
                "!echo 中文\n/skill path with spaces"
            )
        });
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "echo 中文\n/skill path with spaces"
            );
            assert!(
                view.input
                    .as_ref(ctx)
                    .ai_input_model()
                    .as_ref(ctx)
                    .is_input_type_locked()
            );
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
fn grok_unknown_stop_and_idle_notification_do_not_authorize_automatic_enter() {
    App::test((), |mut app| async move {
        let terminal = prepare_guard_test(&mut app, CLIAgent::Grok);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            native_event(view, CLIAgentEventType::PromptSubmit, ctx);
            native_event(view, CLIAgentEventType::Stop, ctx);
            native_event(view, CLIAgentEventType::IdlePrompt, ctx);
            assert_eq!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .unwrap()
                    .status,
                CLIAgentSessionStatus::Unknown
            );
        });
        submit_cli_test_input(&terminal, &mut app, "unknown must remain a draft");
        assert!(writes.borrow().is_empty());
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "unknown must remain a draft"
            )
        });
    });
}
