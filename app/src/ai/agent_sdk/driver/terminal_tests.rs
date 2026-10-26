use std::sync::Arc;

use futures::FutureExt as _;
use warpui::App;

use super::TerminalDriver;
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::terminal::input::CommandExecutionSource;
use crate::terminal::model::ansi::{Handler, PreexecValue};
use crate::terminal::model::block::{BlockId, BlockState, SerializedBlock};
use crate::terminal::view::{Event, ExecuteCommandEvent};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn command_started_event(command: &str) -> Event {
    Event::ExecuteCommand(ExecuteCommandEvent {
        command: command.to_owned(),
        session_id: 123.into(),
        workflow_id: None,
        workflow_command: None,
        should_add_command_to_history: false,
        source: CommandExecutionSource::User,
    })
}

fn block_completed_event(id: BlockId, exit_code: i32) -> Event {
    Event::BlockCompleted {
        block: Arc::new(SerializedBlock {
            id,
            exit_code: exit_code.into(),
            did_execute: false,
            is_background: false,
            ..Default::default()
        }),
        is_local: true,
    }
}

#[test]
fn block_completion_before_command_start_does_not_resolve_exit() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let command_future = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo ready", ctx)
            })
            .expect("应接受命令");

        terminal.update(&mut app, |view, ctx| {
            assert!(
                view.model
                    .lock()
                    .block_list()
                    .is_bootstrapping_precmd_done(),
                "回归前提：旧逻辑会接受引导完成后的非后台块完成事件"
            );
            ctx.emit(block_completed_event(BlockId::new(), 0));
        });
        terminal.update(&mut app, |_, ctx| {
            ctx.emit(command_started_event("echo ready"));
        });
        let mut handle = command_future.await.expect("应确认命令开始");

        assert!((&mut handle).now_or_never().is_none());
        let block_id = handle.block_id().clone();
        terminal.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(block_id, 7));
        });
        assert_eq!(handle.await.expect("应确认命令完成").value(), 7);
    });
}

#[test]
fn matching_block_completion_resolves_nonzero_exit_without_preexec() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let active_block_id =
            terminal.read(&app, |view, _| view.model.lock().active_block_id().clone());
        let command_future = driver
            .update(&mut app, |driver, ctx| driver.execute_command("false", ctx))
            .expect("应接受命令");

        terminal.update(&mut app, |_, ctx| {
            ctx.emit(command_started_event("false"));
        });
        let handle = command_future.await.expect("应确认命令开始");
        assert_eq!(handle.block_id(), &active_block_id);

        terminal.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(active_block_id, 17));
        });
        assert_eq!(handle.await.expect("应确认命令完成").value(), 17);
    });
}

#[test]
fn unrelated_block_completion_after_command_start_does_not_resolve_exit() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let command_future = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo ready", ctx)
            })
            .expect("应接受命令");
        terminal.update(&mut app, |_, ctx| {
            ctx.emit(command_started_event("echo ready"));
        });
        let mut handle = command_future.await.expect("应确认命令开始");

        terminal.update(&mut app, |view, ctx| {
            assert!(
                view.model
                    .lock()
                    .block_list()
                    .is_bootstrapping_precmd_done(),
                "回归前提：旧逻辑会接受引导完成后的非后台块完成事件"
            );
            ctx.emit(block_completed_event(BlockId::new(), 0));
        });
        assert!((&mut handle).now_or_never().is_none());

        let block_id = handle.block_id().clone();
        terminal.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(block_id, 0));
        });
        assert_eq!(handle.await.expect("应确认命令完成").value(), 0);
    });
}

#[test]
fn different_command_text_does_not_record_started_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let command_future = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo ready", ctx)
            })
            .expect("应接受命令");
        futures::pin_mut!(command_future);

        terminal.update(&mut app, |_, ctx| {
            ctx.emit(command_started_event("echo unrelated"));
        });
        assert!(command_future.as_mut().now_or_never().is_none());

        let active_block_id =
            terminal.read(&app, |view, _| view.model.lock().active_block_id().clone());
        terminal.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(active_block_id.clone(), 0));
            ctx.emit(command_started_event("echo ready"));
        });
        let mut handle = command_future.await.expect("应确认预期命令开始");
        assert!((&mut handle).now_or_never().is_none());

        terminal.update(&mut app, |_, ctx| {
            ctx.emit(block_completed_event(active_block_id, 9));
        });
        assert_eq!(handle.await.expect("应确认预期命令完成").value(), 9);
    });
}

#[test]
fn shell_exit_before_command_start_returns_error() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let command_future = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo ready", ctx)
            })
            .expect("应接受命令");

        terminal.update(&mut app, |_, ctx| ctx.emit(Event::Exited));

        assert!(matches!(
            command_future.now_or_never(),
            Some(Err(AgentDriverError::InvalidRuntimeState))
        ));
    });
}

#[test]
fn shell_exit_after_command_start_returns_error() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let command_future = driver
            .update(&mut app, |driver, ctx| {
                driver.execute_command("echo ready", ctx)
            })
            .expect("应接受命令");
        terminal.update(&mut app, |_, ctx| {
            ctx.emit(command_started_event("echo ready"));
        });
        let handle = command_future.await.expect("应确认命令开始");

        terminal.update(&mut app, |_, ctx| ctx.emit(Event::Exited));

        assert!(matches!(
            handle.now_or_never(),
            Some(Err(AgentDriverError::InvalidRuntimeState))
        ));
    });
}

#[test]
fn block_plaintext_exposes_continuation_prompt_until_preexec() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let driver =
            app.update(|ctx| TerminalDriver::create_from_existing_view(terminal.clone(), ctx));
        let block_id = terminal.update(&mut app, |view, _| {
            let mut model = view.model.lock();
            model.block_list_mut().active_block_mut().start();
            model.process_bytes("echo \"unterminated\r\ndquote> ");
            assert_eq!(
                model.block_list().active_block().state(),
                BlockState::BeforeExecution
            );
            model.active_block_id().clone()
        });

        assert_eq!(
            app.read(|ctx| driver.as_ref(ctx).block_output_plaintext(&block_id, ctx)),
            Some("echo \"unterminated\ndquote> ".to_owned())
        );

        terminal.update(&mut app, |view, _| {
            let mut model = view.model.lock();
            model.preexec(PreexecValue {
                command: "echo \"unterminated\"".to_owned(),
                ..Default::default()
            });
            model.process_bytes("executed");
        });

        assert_eq!(
            app.read(|ctx| driver.as_ref(ctx).block_output_plaintext(&block_id, ctx)),
            Some("executed".to_owned())
        );
    });
}
