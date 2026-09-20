use warpui::App;

use super::*;
use crate::terminal::cli_agent_updates::{CliAgentUpdatePhase, CliAgentUpdatesModel};
use crate::terminal::model::ansi::{BootstrappedValue, InitShellValue};
use crate::terminal::model::session::{BootstrapSessionType, SessionInfo};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

#[test]
fn updating_cli_refuses_launch_without_replacing_existing_draft() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| ToastStack);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Codex, CliAgentUpdatePhase::Updating);
        });
        let terminal = add_window_with_terminal(&mut app, None);
        register_test_session(&terminal, BootstrapSessionType::Local, &mut app);

        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("保留用户草稿", ctx);
            });
            view.execute_specific_cli_agent_or_set_pending(CLIAgent::Codex, None, ctx);

            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "保留用户草稿");
            assert!(!view.input.as_ref(ctx).has_pending_command());
            assert!(view.pending_specific_cli_agent_launch.is_none());
            assert!(!view.awaiting_pending_command_completion);
        });
    });
}

#[test]
fn verifying_cli_keeps_pending_launch_after_shell_becomes_ready() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| ToastStack);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let terminal = add_window_with_terminal(&mut app, None);
        register_test_session(&terminal, BootstrapSessionType::Local, &mut app);
        terminal.update(&mut app, |view, ctx| {
            view.execute_specific_cli_agent_or_set_pending(CLIAgent::Claude, None, ctx);
        });
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Claude, CliAgentUpdatePhase::Verifying);
        });

        terminal.update(&mut app, |view, ctx| {
            let revision = view
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .buffer_revision(ctx);
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
                model.simulate_block("pwd", "合成目录");
            }
            view.execute_pending_command((), ctx);

            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "claude");
            assert_eq!(
                view.input
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .buffer_revision(ctx),
                revision
            );
            assert!(view.input.as_ref(ctx).has_pending_command());
            assert_eq!(
                view.pending_specific_cli_agent_launch
                    .as_ref()
                    .unwrap()
                    .agent,
                CLIAgent::Claude
            );
            assert!(!view.awaiting_pending_command_completion);
            assert!(!view.prepare_specific_cli_agent_pending_command(ctx));
        });
    });
}

#[test]
fn checking_cli_still_accepts_launch() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Grok, CliAgentUpdatePhase::Checking);
        });
        let terminal = add_window_with_terminal(&mut app, None);
        register_test_session(&terminal, BootstrapSessionType::Local, &mut app);

        terminal.update(&mut app, |view, ctx| {
            view.execute_specific_cli_agent_or_set_pending(CLIAgent::Grok, None, ctx);

            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "grok");
            assert!(view.input.as_ref(ctx).has_pending_command());
            assert!(view.pending_specific_cli_agent_launch.is_some());
            assert!(view.prepare_specific_cli_agent_pending_command(ctx));
        });
    });
}

#[test]
fn another_cli_update_does_not_block_launch() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Codex, CliAgentUpdatePhase::Updating);
        });
        let terminal = add_window_with_terminal(&mut app, None);
        register_test_session(&terminal, BootstrapSessionType::Local, &mut app);

        terminal.update(&mut app, |view, ctx| {
            view.execute_specific_cli_agent_or_set_pending(CLIAgent::Grok, None, ctx);

            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "grok");
            assert!(view.input.as_ref(ctx).has_pending_command());
            assert!(view.pending_specific_cli_agent_launch.is_some());
        });
    });
}

fn register_test_session(
    terminal: &ViewHandle<TerminalView>,
    session_type: BootstrapSessionType,
    app: &mut App,
) {
    terminal.update(app, |view, ctx| {
        CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, ctx| {
            for agent in [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok] {
                updates.set_busy(agent, false, ctx);
            }
        });
        view.sessions.update(ctx, |sessions, _| {
            sessions.register_session_for_test(
                SessionInfo::new_for_test().with_session_type(session_type),
            );
        });
        view.input.update(ctx, |input, ctx| {
            input.set_active_block_metadata(BlockMetadata::new(Some(0.into()), None), false, ctx);
        });
    });
}

#[test]
fn remote_cli_launch_ignores_local_update() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Codex, CliAgentUpdatePhase::Updating);
        });
        let terminal = add_window_with_terminal(&mut app, None);
        register_test_session(&terminal, BootstrapSessionType::WarpifiedRemote, &mut app);
        terminal.update(&mut app, |view, ctx| {
            view.execute_specific_cli_agent_or_set_pending(CLIAgent::Codex, None, ctx);
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "codex");
            assert!(view.input.as_ref(ctx).has_pending_command());
            assert!(view.prepare_specific_cli_agent_pending_command(ctx));
        });
    });
}

#[test]
fn stale_block_completion_cannot_release_current_cli_launch() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let terminal = add_window_with_terminal(&mut app, None);
        register_test_session(&terminal, BootstrapSessionType::Local, &mut app);
        terminal.update(&mut app, |view, ctx| {
            assert!(
                view.input
                    .update(ctx, |input, ctx| input.try_reserve_cli_launch("codex", ctx))
                    .is_ok()
            );
            let current: BlockId = "current-cli-launch".to_owned().into();
            let old: BlockId = "old-cli-launch".to_owned().into();
            view.cli_agent_launch_reservation_block_id = Some(current.clone());
            view.release_cli_launch_reservation(Some(&old), ctx);
            assert!(
                CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Codex)
                    .unwrap()
                    .busy
            );
            view.release_cli_launch_reservation(Some(&current), ctx);
            assert!(
                !CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Codex)
                    .unwrap()
                    .busy
            );
            view.release_cli_launch_reservation(Some(&current), ctx);
            assert!(
                !CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Codex)
                    .unwrap()
                    .busy
            );
        });
    });
}

#[test]
fn update_finished_does_not_replay_an_edited_cli_draft() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let terminal = add_window_with_terminal(&mut app, None);
        register_test_session(&terminal, BootstrapSessionType::Local, &mut app);
        terminal.update(&mut app, |view, ctx| {
            view.execute_specific_cli_agent_or_set_pending(CLIAgent::Claude, None, ctx);
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("echo edited", ctx)
            });
            CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, _| {
                updates.set_phase_for_test(CLIAgent::Claude, CliAgentUpdatePhase::UpToDate);
            });
            view.retry_cli_launch_after_update(CLIAgent::Claude, ctx);
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "echo edited");
            assert!(view.input.as_ref(ctx).has_pending_command());
            assert!(!view.awaiting_pending_command_completion);
        });
    });
}

#[test]
fn background_cli_reservation_survives_block_and_shell_exit() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let terminal = add_window_with_terminal(&mut app, None);
        register_test_session(&terminal, BootstrapSessionType::Local, &mut app);
        terminal.update(&mut app, |view, ctx| {
            assert!(
                view.input
                    .update(ctx, |input, ctx| input
                        .try_reserve_cli_launch("grok &", ctx))
                    .is_ok()
            );
            let block: BlockId = "background-cli-launch".to_owned().into();
            view.cli_agent_launch_reservation_block_id = Some(block.clone());
            view.release_cli_launch_reservation(Some(&block), ctx);
            assert!(
                CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Grok)
                    .unwrap()
                    .busy
            );
            view.release_cli_launch_reservation(None, ctx);
            assert!(
                CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Grok)
                    .unwrap()
                    .busy
            );
        });
    });
}

#[test]
fn project_command_refuses_local_cli_during_update() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let terminal = add_window_with_terminal(&mut app, None);
        register_test_session(&terminal, BootstrapSessionType::Local, &mut app);
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Claude, CliAgentUpdatePhase::Updating);
        });
        terminal.update(&mut app, |view, ctx| {
            assert!(!view.execute_project_agent_command(
                "claude update",
                AIAgentActionId::from("project-update-guard".to_owned()),
                AIConversationId::new(),
                ctx,
            ));
            assert!(view.cli_agent_launch_reservation_block_id.is_none());
            assert!(
                !CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Claude)
                    .unwrap()
                    .busy
            );
        });
    });
}
