use std::cell::Cell;
use std::rc::Rc;

use onboarding::callout::{FinalState, OnboardingCalloutViewEvent};
use onboarding::{OnboardingCalloutView, OnboardingIntention};
use warpui::App;

use crate::terminal::view::Event;
use crate::workspace::view::tests::{initialize_app, mock_workspace};

#[test]
fn cancelled_pending_setup_does_not_start_tutorial_on_later_completion() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(onboarding::init);
        let workspace = mock_workspace(&mut app);
        let completions = Rc::new(Cell::new(0));
        let recorded = completions.clone();
        let (terminal, window_id) = workspace.update(&mut app, |workspace, ctx| {
            let terminal = workspace.active_session_view(ctx).expect("应有活动终端");
            terminal.update(ctx, |terminal, ctx| {
                terminal.set_pending_command("echo setup", ctx);
            });
            workspace.dispatch_tutorial_after_setup_commands(OnboardingIntention::Terminal, ctx);
            ctx.subscribe_to_view(&terminal, move |_, _, event, _| {
                if let Event::PendingCommandCompleted = event {
                    recorded.set(recorded.get() + 1);
                }
            });
            (terminal, ctx.window_id())
        });

        terminal.update(&mut app, |terminal, ctx| {
            terminal.write_to_pty(vec![0x03], ctx);
        });
        terminal.update(&mut app, |_, ctx| {
            ctx.emit(Event::PendingCommandCompleted);
        });

        assert!(app.read(|ctx| {
            ctx.views_of_type::<OnboardingCalloutView>(window_id)
                .unwrap()
                .is_empty()
        }));
        assert_eq!(completions.get(), 1, "同一 Workspace 的其他订阅必须保留");
    });
}

#[test]
fn completed_pending_setup_starts_tutorial_once_and_preserves_other_subscriptions() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        app.update(onboarding::init);
        let workspace = mock_workspace(&mut app);
        let completions = Rc::new(Cell::new(0));
        let recorded = completions.clone();
        let (terminal, window_id) = workspace.update(&mut app, |workspace, ctx| {
            let terminal = workspace.active_session_view(ctx).expect("应有活动终端");
            terminal.update(ctx, |terminal, ctx| {
                terminal.set_pending_command("echo setup", ctx);
            });
            workspace.dispatch_tutorial_after_setup_commands(OnboardingIntention::Terminal, ctx);
            ctx.subscribe_to_view(&terminal, move |_, _, event, _| {
                if let Event::PendingCommandCompleted = event {
                    recorded.set(recorded.get() + 1);
                }
            });
            (terminal, ctx.window_id())
        });

        terminal.update(&mut app, |_, ctx| {
            ctx.emit(Event::PendingCommandCompleted);
        });
        let callouts = app.read(|ctx| {
            ctx.views_of_type::<OnboardingCalloutView>(window_id)
                .unwrap()
        });
        assert_eq!(callouts.len(), 1, "正常完成必须启动教程");
        callouts[0].update(&mut app, |_, ctx| {
            ctx.emit(OnboardingCalloutViewEvent::Completed {
                final_state: FinalState::Skip,
            });
        });
        drop(callouts);
        app.update(|_| {});

        // 先关闭教程再发送下一次完成，避免被“教程已存在”的守卫掩盖重复派发。
        terminal.update(&mut app, |_, ctx| {
            ctx.emit(Event::PendingCommandCompleted);
        });
        assert!(app.read(|ctx| {
            ctx.views_of_type::<OnboardingCalloutView>(window_id)
                .unwrap()
                .is_empty()
        }));
        assert_eq!(completions.get(), 2, "同一 Workspace 的其他订阅必须保留");
    });
}
