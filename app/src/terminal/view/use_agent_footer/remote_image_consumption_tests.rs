use futures::channel::oneshot;
use futures::future::{Either, select};

use super::*;

fn claimed_image(
    app: &mut App,
    terminal: &ViewHandle<TerminalView>,
) -> (Uuid, Uuid, Rc<CliInputSubmission>) {
    terminal.update(app, |view, ctx| {
        view.input.update(ctx, |input, ctx| {
            input.replace_buffer_content("原图片草稿\n第二行", ctx);
        });
        view.ai_context_model.update(ctx, |model, ctx| {
            model.append_pending_images(vec![test_image("aGVsbG8=", "original.png")], ctx);
        });
        let snapshot = Rc::new(CliInputSubmission {
            agent: CLIAgent::Claude,
            query: "原图片草稿\n第二行".to_owned(),
            editor_revision: view
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .buffer_revision(ctx),
            attachments_revision: view
                .ai_context_model
                .as_ref(ctx)
                .pending_attachments_revision(),
        });
        let submission_id = Uuid::new_v4();
        let generation = CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
            let generation = sessions.input_generation(view.view_id).unwrap();
            assert!(sessions.begin_input_submission(view.view_id, generation));
            assert!(sessions.register_remote_image_submission_owner(
                view.view_id,
                generation,
                submission_id,
                Some(snapshot.editor_revision.clone()),
            ));
            // 本组只模拟服务端已领取之后的 UI 凭证，不发送原生输入或制造原生 ACK。
            sessions.register_remote_image_consumption(view.view_id, generation, submission_id);
            generation
        });
        (generation, submission_id, snapshot)
    })
}

fn next_visibility_event(
    app: &mut App,
    terminal: &ViewHandle<TerminalView>,
) -> oneshot::Receiver<()> {
    let view_id = terminal.read(app, |view, _| view.view_id);
    let (sender, receiver) = oneshot::channel();
    let mut sender = Some(sender);
    app.update(|ctx| {
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), move |_, event, _| {
            if let CLIAgentSessionsModelEvent::InputSessionChanged {
                terminal_view_id, ..
            } = event
                && *terminal_view_id == view_id
                && let Some(sender) = sender.take()
            {
                sender.send(()).unwrap();
            }
        });
    });
    receiver
}

async fn visibility_delivered(receiver: oneshot::Receiver<()>) {
    match select(receiver, Box::pin(Timer::after(Duration::from_secs(5)))).await {
        Either::Left((result, _)) => result.unwrap(),
        Either::Right(_) => panic!("未收到真实输入可见性事件"),
    }
}

async fn auto_hide(app: &mut App, terminal: &ViewHandle<TerminalView>) {
    let event = next_visibility_event(app, terminal);
    terminal.update(app, |view, ctx| {
        view.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::AutoToggle, ctx);
    });
    visibility_delivered(event).await;
}

async fn reopen(app: &mut App, terminal: &ViewHandle<TerminalView>) {
    let event = next_visibility_event(app, terminal);
    terminal.update(app, |view, ctx| {
        view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
    });
    visibility_delivered(event).await;
}

#[test]
fn remote_image_consumption_clears_automatically_hidden_draft_without_reviving_write_lease() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let writes = collect_cli_test_writes(&mut app, &terminal);
        let (generation, id, snapshot) = claimed_image(&mut app, &terminal);
        auto_hide(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            assert_ne!(sessions.input_generation(view.view_id), Some(generation));
            assert!(!sessions.is_input_submission_current(view.view_id, generation));
            assert_eq!(
                sessions
                    .session(view.view_id)
                    .unwrap()
                    .draft_text
                    .as_deref(),
                Some("原图片草稿\n第二行")
            );
            assert!(view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .unwrap();
            assert!(session.draft_text.is_none());
            assert!(session.session_context.query.is_none());
            assert_eq!(session.status, CLIAgentSessionStatus::InProgress);
            assert!(
                view.ai_context_model
                    .as_ref(ctx)
                    .pending_images()
                    .is_empty()
            );
        });
        reopen(&mut app, &terminal).await;
        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "")
        });
        assert!(writes.borrow().is_empty());
    });
}

#[test]
fn remote_image_consumption_follows_real_hide_restore_buffer_revisions() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (_, id, snapshot) = claimed_image(&mut app, &terminal);
        auto_hide(&mut app, &terminal).await;
        reopen(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "原图片草稿\n第二行"
            );
            assert_ne!(
                view.input
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .buffer_revision(ctx),
                snapshot.editor_revision
            );
            assert!(view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "");
            assert!(
                view.ai_context_model
                    .as_ref(ctx)
                    .pending_images()
                    .is_empty()
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
fn remote_image_consumption_does_not_treat_unconfirmed_auto_restore_as_consumed() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let _ = claimed_image(&mut app, &terminal);
        auto_hide(&mut app, &terminal).await;
        reopen(&mut app, &terminal).await;
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "原图片草稿\n第二行"
            );
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 1);
        });
    });
}

#[test]
fn remote_image_consumption_rejects_editor_aba_before_automatic_hide() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (_, id, snapshot) = claimed_image(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("新草稿", ctx);
                input.replace_buffer_content("原图片草稿\n第二行", ctx);
            })
        });
        auto_hide(&mut app, &terminal).await;
        reopen(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            assert!(!view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "原图片草稿\n第二行"
            );
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 1);
        });
    });
}

#[test]
fn remote_image_consumption_rejects_edits_while_hidden_even_if_restore_text_matches() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (_, id, snapshot) = claimed_image(&mut app, &terminal);
        auto_hide(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("隐藏时的新编辑", ctx);
                input.replace_buffer_content("", ctx);
            })
        });
        reopen(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            assert!(!view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "原图片草稿\n第二行"
            );
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 1);
        });
    });
}

#[test]
fn remote_image_consumption_preserves_new_attachments_and_original_text_together() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (_, id, snapshot) = claimed_image(&mut app, &terminal);
        auto_hide(&mut app, &terminal).await;
        reopen(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            view.ai_context_model.update(ctx, |model, ctx| {
                model.append_pending_images(vec![test_image("aGVsbG8=", "new.png")], ctx);
            });
            assert!(!view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "原图片草稿\n第二行"
            );
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 2);
        });
    });
}

#[test]
fn remote_image_consumption_rejects_saved_draft_aba_while_hidden() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (_, id, snapshot) = claimed_image(&mut app, &terminal);
        auto_hide(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                sessions.set_draft(view.view_id, "新的保存草稿".to_owned());
                sessions.set_draft(view.view_id, "原图片草稿\n第二行".to_owned());
            });
        });
        reopen(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            assert!(!view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "原图片草稿\n第二行"
            );
            assert_eq!(view.ai_context_model.as_ref(ctx).pending_images().len(), 1);
        });
    });
}

#[test]
fn remote_image_consumption_is_revoked_by_manual_hide() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (_, id, snapshot) = claimed_image(&mut app, &terminal);
        let event = next_visibility_event(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            view.close_cli_agent_rich_input_and_disable_auto_toggle(ctx)
        });
        visibility_delivered(event).await;
        reopen(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            assert!(!view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "原图片草稿\n第二行"
            );
        });
    });
}

#[test]
fn remote_image_consumption_is_revoked_by_ctrl_c_after_auto_hide() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (_, id, snapshot) = claimed_image(&mut app, &terminal);
        auto_hide(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.observe_ctrl_c_write(view.view_id, ctx);
            })
        });
        reopen(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            assert!(!view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "原图片草稿\n第二行"
            );
        });
    });
}

#[test]
fn remote_image_consumption_old_callback_does_not_release_a_new_submission() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (_, old_id, old_snapshot) = claimed_image(&mut app, &terminal);
        auto_hide(&mut app, &terminal).await;
        reopen(&mut app, &terminal).await;
        let (generation, id, snapshot) = claimed_image(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            assert!(!view.complete_remote_cli_image_consumption(old_id, &old_snapshot, ctx));
            let sessions = CLIAgentSessionsModel::as_ref(ctx);
            assert!(sessions.is_input_submission_current(view.view_id, generation));
            assert!(
                sessions
                    .remote_image_consumption_revision(view.view_id, id)
                    .is_some()
            );
            assert!(view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
        });
    });
}

#[test]
fn remote_image_consumption_cannot_clear_a_replacement_session() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (_, id, snapshot) = claimed_image(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            register_cli_input_test_session(view, CLIAgent::Claude, ctx);
        });
        reopen(&mut app, &terminal).await;
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("新会话草稿", ctx)
            });
            assert!(!view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "新会话草稿");
        });
    });
}

#[test]
fn remote_image_write_lease_survives_consumption_draft_revocation() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (generation, id, snapshot) = claimed_image(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                sessions.set_draft(view.view_id, "用户保存的新草稿".to_owned());
                assert!(
                    sessions
                        .remote_image_consumption_revision(view.view_id, id)
                        .is_none()
                );
                assert!(sessions.is_input_submission_current(view.view_id, generation));
                assert!(sessions.finish_remote_image_submission(view.view_id, generation, id));
                assert!(!sessions.is_input_submission_current(view.view_id, generation));
            });
            assert!(!view.complete_remote_cli_image_consumption(id, &snapshot, ctx));
        });
    });
}

#[test]
fn remote_image_old_callback_cannot_release_new_unclaimed_submission_in_same_generation() {
    App::test((), |mut app| async move {
        let terminal = prepare_rich_cli_test(&mut app, CLIAgent::Claude);
        let (generation, old_id, old_snapshot) = claimed_image(&mut app, &terminal);
        terminal.update(&mut app, |view, ctx| {
            let new_id = Uuid::new_v4();
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                assert!(sessions.finish_remote_image_submission(view.view_id, generation, old_id));
                assert!(sessions.begin_input_submission(view.view_id, generation));
                assert!(sessions.register_remote_image_submission_owner(
                    view.view_id,
                    generation,
                    new_id,
                    None,
                ));
                assert!(!sessions.finish_remote_image_submission(view.view_id, generation, old_id));
                sessions.finish_remote_image_consumption(view.view_id, old_id);
                assert!(sessions.is_input_submission_current(view.view_id, generation));
                assert!(
                    sessions
                        .remote_image_consumption_revision(view.view_id, new_id)
                        .is_none()
                );
            });
            assert!(!view.complete_remote_cli_image_consumption(old_id, &old_snapshot, ctx));
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                assert!(sessions.is_input_submission_current(view.view_id, generation));
                assert!(sessions.finish_remote_image_submission(view.view_id, generation, new_id));
                assert!(!sessions.is_input_submission_current(view.view_id, generation));
            });
        });
    });
}
