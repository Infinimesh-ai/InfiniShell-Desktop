use warpui::App;

use super::*;
use crate::terminal::cli_agent_updates::{CliAgentUpdatePhase, CliAgentUpdatesModel};
use crate::terminal::model::session::{BootstrapSessionType, SessionInfo};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn input_with_session(app: &mut App, session_info: SessionInfo) -> ViewHandle<Input> {
    let terminal = add_window_with_terminal(app, None);
    let input = terminal.read(app, |view, _| view.input().clone());
    input.update(app, |input, ctx| {
        CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, ctx| {
            for agent in [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok] {
                updates.set_busy(agent, false, ctx);
            }
        });
        input.sessions.update(ctx, |sessions, _| {
            sessions.register_session_for_test(session_info);
        });
        input.set_active_block_metadata(BlockMetadata::new(Some(0.into()), None), false, ctx);
    });
    // 保留终端根视图由 window 持有，输入的终端 ID 与租约保持一致。
    input
}

#[test]
fn management_and_version_commands_reserve_local_installation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let input = input_with_session(&mut app, SessionInfo::new_for_test());
        for (command, agent) in [
            ("codex update", CLIAgent::Codex),
            ("claude --version", CLIAgent::Claude),
            ("grok update --check --json", CLIAgent::Grok),
            ("SYNTHETIC=1 grok --version", CLIAgent::Grok),
            (r#""/synthetic folder/codex" --version"#, CLIAgent::Codex),
            ("claude.exe --version", CLIAgent::Claude),
        ] {
            input.update(&mut app, |input, ctx| {
                assert!(input.try_reserve_cli_launch(command, ctx).is_ok());
                assert!(
                    CliAgentUpdatesModel::as_ref(ctx)
                        .status(agent)
                        .unwrap()
                        .busy
                );
                CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, ctx| {
                    updates.release_launch(input.terminal_view_id, ctx);
                });
            });
        }
    });
}

#[test]
fn compound_and_alias_launches_reserve_each_cli_once() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let input = input_with_session(
            &mut app,
            SessionInfo::new_for_test().with_aliases(HashMap::from([(
                "assistant".into(),
                "claude --version".into(),
            )])),
        );
        input.update(&mut app, |input, ctx| {
            assert!(
                input
                    .try_reserve_cli_launch("cd project && codex; assistant; grok update", ctx)
                    .is_ok()
            );
            for agent in [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok] {
                assert!(
                    CliAgentUpdatesModel::as_ref(ctx)
                        .status(agent)
                        .unwrap()
                        .busy
                );
            }
            CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, ctx| {
                updates.release_launch(input.terminal_view_id, ctx);
            });
            for agent in [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok] {
                assert!(
                    !CliAgentUpdatesModel::as_ref(ctx)
                        .status(agent)
                        .unwrap()
                        .busy
                );
            }
        });
    });
}

#[test]
fn blocked_compound_keeps_draft_and_does_not_claim_partial_launches() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let input = input_with_session(&mut app, SessionInfo::new_for_test());
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Grok, CliAgentUpdatePhase::Verifying);
        });
        input.update(&mut app, |input, ctx| {
            input.set_pending_command("codex; grok", ctx);
            let revision = input.editor().as_ref(ctx).buffer_revision(ctx);
            assert!(input.try_reserve_cli_launch("codex; grok", ctx).is_err());
            assert!(input.has_pending_command());
            assert_eq!(input.buffer_text(ctx), "codex; grok");
            assert_eq!(input.editor().as_ref(ctx).buffer_revision(ctx), revision);
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
fn remote_commands_neither_block_nor_reserve_local_updates() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let input = input_with_session(
            &mut app,
            SessionInfo::new_for_test().with_session_type(BootstrapSessionType::WarpifiedRemote),
        );
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Codex, CliAgentUpdatePhase::Updating);
        });
        input.update(&mut app, |input, ctx| {
            assert!(
                input
                    .try_reserve_cli_launch("codex; claude --version", ctx)
                    .is_ok()
            );
            assert!(
                !CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Codex)
                    .unwrap()
                    .busy
            );
            assert!(
                !CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Claude)
                    .unwrap()
                    .busy
            );
        });
    });
}

#[test]
fn checking_phase_allows_reservation_and_generic_commands_do_not_reserve() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let input = input_with_session(&mut app, SessionInfo::new_for_test());
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Grok, CliAgentUpdatePhase::Checking);
        });
        input.update(&mut app, |input, ctx| {
            assert!(input.try_reserve_cli_launch("echo grok", ctx).is_ok());
            assert!(
                !CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Grok)
                    .unwrap()
                    .busy
            );
            assert!(input.try_reserve_cli_launch("grok --version", ctx).is_ok());
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
fn pending_launch_is_consumed_only_after_update_guard_accepts_dispatch() {
    App::test((), |mut app| async move {
        super::tests::initialize_app(&mut app);
        app.add_singleton_model(CliAgentUpdatesModel::new);
        let terminal =
            super::tests::add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |view, _| view.input().clone());
        super::tests::simulate_directory_for_completion(
            0.into(),
            &terminal,
            &mut app,
            "/synthetic",
        );
        CliAgentUpdatesModel::handle(&app).update(&mut app, |updates, _| {
            updates.set_phase_for_test(CLIAgent::Codex, CliAgentUpdatePhase::Updating);
        });
        input.update(&mut app, |input, ctx| {
            assert!(!input.can_execute_command(ctx).is_no());
            assert!(
                input
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .has_received_precmd()
            );
            input.set_pending_command("codex", ctx);
            let revision = input.editor().as_ref(ctx).buffer_revision(ctx);
            input.execute_pending_command(ctx);
            assert!(input.has_pending_command());
            assert_eq!(input.buffer_text(ctx), "codex");
            assert_eq!(input.editor().as_ref(ctx).buffer_revision(ctx), revision);
            CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, _| {
                updates.set_phase_for_test(CLIAgent::Codex, CliAgentUpdatePhase::UpToDate);
            });
            input.execute_pending_command(ctx);
            assert!(!input.has_pending_command());
            assert!(
                CliAgentUpdatesModel::as_ref(ctx)
                    .status(CLIAgent::Codex)
                    .unwrap()
                    .busy
            );
        });
    });
}

#[test]
fn background_separator_uses_parsed_argument_boundaries() {
    assert!(Input::has_background_separator(
        "grok &",
        EscapeChar::Backslash
    ));
    assert!(Input::has_background_separator(
        "grok | cat &",
        EscapeChar::Backslash
    ));
    assert!(Input::has_background_separator(
        "grok && sleep 1 &",
        EscapeChar::Backslash
    ));
    assert!(!Input::has_background_separator(
        "grok && echo done",
        EscapeChar::Backslash
    ));
    assert!(!Input::has_background_separator(
        "grok 'a & b'",
        EscapeChar::Backslash
    ));
    assert!(!Input::has_background_separator(
        r"grok \&",
        EscapeChar::Backslash
    ));
}
