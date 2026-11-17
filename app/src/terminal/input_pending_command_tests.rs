use std::cell::RefCell;
#[cfg(unix)]
use std::path::Path;
use std::rc::Rc;

use futures::FutureExt as _;
use warpui::{App, TypedActionView, ViewHandle};

use super::tests::{add_window_with_bootstrapped_terminal, initialize_app};
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::ai::agent_sdk::driver::terminal::TerminalDriver;
use crate::editor::EditorAction;
use crate::terminal::TerminalView;
use crate::terminal::cli_agent::CLIAgent;
#[cfg(unix)]
use crate::terminal::cli_agent::{CLIAgentLaunchSnapshot, cli_agent_launch_fallback};
use crate::terminal::model::ansi::{
    CommandFinishedValue, CompletionMetadata, Handler, PrecmdValue, PromptMetadata,
};
use crate::terminal::model_events::ModelEvent;
#[cfg(unix)]
use crate::terminal::shell::ShellType;
use crate::terminal::view::Event;

fn finish_bootstrap(terminal: &ViewHandle<TerminalView>, app: &mut App) {
    terminal.update(app, |terminal, ctx| {
        {
            let mut model = terminal.model.lock();
            let session_id = model.block_list().active_block().session_id().unwrap();
            let completion_metadata = CompletionMetadata::default();
            model.command_finished(CommandFinishedValue {
                completion_metadata: completion_metadata.clone(),
                session_id: Some(session_id.as_u64()),
            });
            model.precmd_with_completion_metadata(PrecmdValue {
                completion_metadata,
                prompt_metadata: PromptMetadata {
                    session_id: Some(session_id.as_u64()),
                    ..Default::default()
                },
            });
            assert!(model.block_list().is_bootstrapping_precmd_done());
        }
        terminal.model_event_dispatcher().update(ctx, |_, ctx| {
            ctx.emit(ModelEvent::BootstrapPrecmdDone);
        });
    });
}

fn executed_commands(
    terminal: &ViewHandle<TerminalView>,
    app: &mut App,
) -> Rc<RefCell<Vec<String>>> {
    let commands = Rc::new(RefCell::new(Vec::new()));
    let recorded = commands.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(terminal, move |_, event, _| {
            if let Event::ExecuteCommand(event) = event {
                recorded.borrow_mut().push(event.command.clone());
            }
        });
    });
    commands
}

#[test]
fn pending_command_replaces_leftover_input() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        finish_bootstrap(&terminal, &mut app);
        let commands = executed_commands(&terminal, &mut app);

        terminal.update(&mut app, |terminal, ctx| {
            terminal.input().update(ctx, |input, ctx| {
                input.user_replace_editor_text("echo leftover", ctx);
            });
            terminal.execute_command_or_set_pending("echo exact", ctx);
        });

        assert_eq!(commands.borrow().as_slice(), ["echo exact"]);
    });
}

#[test]
fn pending_command_keeps_exact_text_while_bootstrapping() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let commands = executed_commands(&terminal, &mut app);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.execute_command_or_set_pending("echo exact", ctx);
            terminal.input().update(ctx, |input, ctx| {
                input.user_replace_editor_text("echo changed while waiting", ctx);
            });
        });
        assert!(commands.borrow().is_empty());

        finish_bootstrap(&terminal, &mut app);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.execute_pending_command((), ctx);
        });

        assert_eq!(commands.borrow().as_slice(), ["echo exact"]);
    });
}

#[test]
fn pending_command_synced_input_uses_current_editor_text() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        finish_bootstrap(&terminal, &mut app);
        let commands = executed_commands(&terminal, &mut app);

        terminal.update(&mut app, |terminal, ctx| {
            terminal.input().update(ctx, |input, ctx| {
                input.set_pending_command("echo programmatic", ctx);
                input.user_replace_editor_text("echo synchronized", ctx);
                input.run_command_in_synced_terminal_input(ctx);
            });
        });

        assert_eq!(commands.borrow().as_slice(), ["echo synchronized"]);
    });
}

#[test]
fn pending_command_specific_cli_launch_keeps_edited_draft() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let commands = executed_commands(&terminal, &mut app);
        terminal.update(&mut app, |terminal, ctx| {
            terminal.execute_specific_cli_agent_or_set_pending(CLIAgent::Grok, None, ctx);
            terminal.input().update(ctx, |input, ctx| {
                input.user_replace_editor_text("echo user draft", ctx);
            });
        });

        finish_bootstrap(&terminal, &mut app);

        assert_eq!(commands.borrow().as_slice(), ["echo user draft"]);
    });
}

#[cfg(unix)]
#[test]
fn pending_command_specific_cli_launch_preserves_quoted_fallback() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let commands = executed_commands(&terminal, &mut app);
        let installed = Path::new("/synthetic/install dir/grok");
        let replacement = cli_agent_launch_fallback(
            CLIAgent::Grok,
            Some(installed),
            CLIAgentLaunchSnapshot {
                host_namespace_verified: true,
                command_snapshot_complete: true,
                shell_type: ShellType::Zsh,
                path: Some("/shell/bin"),
                cwd: Some(Path::new("/project")),
                command_known: false,
            },
            |path| path == installed,
        )
        .expect("完整快照应选用带引号的安装路径");

        terminal.update(&mut app, |terminal, ctx| {
            terminal.execute_specific_cli_agent_or_set_pending(
                CLIAgent::Grok,
                Some(installed.to_owned()),
                ctx,
            );
            // 验证解析结果到待执行输入的连接，不模拟真实 PTY 的环境判定。
            terminal.input().update(ctx, |input, ctx| {
                input.replace_buffer_content(&replacement, ctx);
            });
        });
        finish_bootstrap(&terminal, &mut app);

        assert_eq!(
            commands.borrow().as_slice(),
            ["'/synthetic/install dir/grok'"]
        );
    });
}

#[test]
fn pending_command_ctrl_c_cancels_dispatch_and_driver_wait() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let commands = executed_commands(&terminal, &mut app);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let start = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo cancelled", ctx)
            })
            .expect("应接受待执行命令");

        terminal.update(&mut app, |terminal, ctx| {
            terminal.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.handle_action(&EditorAction::CtrlC, ctx);
                });
            });
        });

        assert!(matches!(
            start.now_or_never(),
            Some(Err(AgentDriverError::InvalidRuntimeState))
        ));
        finish_bootstrap(&terminal, &mut app);
        assert!(commands.borrow().is_empty());
    });
}

#[test]
fn pending_command_raw_ctrl_c_cancels_once_and_preserves_new_draft() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let commands = executed_commands(&terminal, &mut app);
        let cancellations = Rc::new(RefCell::new(0));
        let recorded = cancellations.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::PendingCommandCancelled = event {
                    *recorded.borrow_mut() += 1;
                }
            });
        });
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let start = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo cancelled", ctx)
            })
            .expect("应接受待执行命令");

        terminal.update(&mut app, |terminal, ctx| {
            terminal.write_to_pty(vec![0x03], ctx);
            terminal.write_to_pty(vec![0x03], ctx);
            terminal.input().update(ctx, |input, ctx| {
                input.user_replace_editor_text("echo new draft", ctx);
            });
        });

        assert!(matches!(
            start.now_or_never(),
            Some(Err(AgentDriverError::InvalidRuntimeState))
        ));
        finish_bootstrap(&terminal, &mut app);
        assert!(commands.borrow().is_empty());
        assert_eq!(*cancellations.borrow(), 1);
        terminal.read(&app, |terminal, ctx| {
            assert_eq!(
                terminal.input().as_ref(ctx).buffer_text(ctx),
                "echo new draft"
            );
        });
    });
}

#[cfg(all(
    feature = "local_fs",
    feature = "local_tty",
    any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(windows, target_arch = "x86_64")
    )
))]
#[test]
fn owned_cli_pending_command_dispatches_supplied_command_once() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        finish_bootstrap(&terminal, &mut app);
        let commands = executed_commands(&terminal, &mut app);

        terminal.update(&mut app, |terminal, ctx| {
            terminal.input().update(ctx, |input, ctx| {
                input.set_pending_command("grok", ctx);
                assert!(input.execute_owned_cli_command_once("echo owned", ctx));
                assert!(!input.execute_owned_cli_command_once("echo duplicate", ctx));
                input.execute_pending_command(ctx);
            });
        });

        assert_eq!(commands.borrow().as_slice(), ["echo owned"]);
    });
}

#[cfg(all(
    feature = "local_fs",
    feature = "local_tty",
    any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(windows, target_arch = "x86_64")
    )
))]
#[test]
fn owned_cli_pending_command_cancel_prevents_later_dispatch() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let commands = executed_commands(&terminal, &mut app);

        terminal.update(&mut app, |terminal, ctx| {
            terminal.input().update(ctx, |input, ctx| {
                input.set_pending_command("grok", ctx);
                input.cancel_owned_cli_pending_command();
                assert!(!input.execute_owned_cli_command_once("echo cancelled", ctx));
            });
        });
        finish_bootstrap(&terminal, &mut app);

        assert!(commands.borrow().is_empty());
    });
}

#[cfg(all(
    feature = "local_fs",
    feature = "local_tty",
    any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(windows, target_arch = "x86_64")
    )
))]
#[test]
fn owned_cli_pending_command_failed_dispatch_is_not_retried() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        finish_bootstrap(&terminal, &mut app);
        let commands = executed_commands(&terminal, &mut app);

        terminal.update(&mut app, |terminal, ctx| {
            terminal
                .model
                .lock()
                .simulate_long_running_block("sleep 10", "");
            terminal.input().update(ctx, |input, ctx| {
                input.set_pending_command("grok", ctx);
                assert!(!input.execute_owned_cli_command_once("echo owned", ctx));
            });
            terminal.model.lock().finish_block();
            terminal.execute_pending_command((), ctx);
            terminal.input().update(ctx, |input, ctx| {
                assert!(!input.execute_owned_cli_command_once("echo duplicate", ctx));
            });
        });

        assert!(commands.borrow().is_empty());
    });
}
