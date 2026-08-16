use std::cell::RefCell;
use std::rc::Rc;

use warp::tui_export::{
    AIConversationId, BlocklistAIHistoryModel, Harness, register_tui_session_view_test_singletons,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, ModelHandle, ReadModel, SingletonEntity as _};
use warpui_core::{App, EntityId, WindowId};

use super::{TuiOrchestrationEvent, TuiOrchestrationModel};
use crate::root_view::RootTuiView;
use crate::session_registry::{TuiSessionId, TuiSessionView, TuiSessions};
use crate::test_fixtures::{add_test_semantic_selection, add_test_terminal_session};

struct OrchestrationFixture {
    sessions: ModelHandle<TuiSessions>,
    window_id: WindowId,
}

fn orchestration_fixture(app: &mut App) -> OrchestrationFixture {
    register_tui_session_view_test_singletons(app);
    add_test_semantic_selection(app);
    app.update(crate::autoupdate::TuiAutoupdater::register);
    let (window_id, root) = app.update(|ctx| {
        ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| RootTuiView::new(),
        )
    });
    let sessions = app.add_singleton_model(|_| TuiSessions::new_for_test());
    root.update(app, |_, ctx| {
        ctx.subscribe_to_model(&sessions, |_, _, _, ctx| ctx.notify());
    });
    app.update(TuiOrchestrationModel::register);
    OrchestrationFixture {
        sessions,
        window_id,
    }
}

fn add_parent_session(app: &mut App, fixture: &OrchestrationFixture) -> TuiSessionId {
    let (session, manager) = add_test_terminal_session(app, fixture.window_id);
    let session_id = app.update(|ctx| {
        TuiSessions::register_session(&fixture.sessions, session, manager, true, ctx)
    });
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id =
                history.start_new_conversation(session_id.surface_id(), false, false, false, ctx);
            history.set_active_conversation_id(conversation_id, session_id.surface_id(), ctx);
        });
    });
    session_id
}

fn active_conversation_id(app: &App, session_id: TuiSessionId) -> AIConversationId {
    app.read(|ctx| {
        BlocklistAIHistoryModel::as_ref(ctx)
            .active_conversation(session_id.surface_id())
            .expect("父会话应处于活动状态")
            .id()
    })
}

fn seed_child(
    app: &mut App,
    parent_conversation_id: AIConversationId,
    name: &str,
    harness: Harness,
) -> AIConversationId {
    app.update(|ctx| {
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.start_new_child_conversation(
                EntityId::new(),
                name.to_owned(),
                parent_conversation_id,
                Some(harness),
                ctx,
            )
        })
    })
}

fn restore_requests(app: &mut App) -> Rc<RefCell<Vec<AIConversationId>>> {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let requests_for_events = requests.clone();
    app.update(|ctx| {
        ctx.subscribe_to_model(
            &TuiOrchestrationModel::handle(ctx),
            move |_, event, _| match event {
                TuiOrchestrationEvent::RestoreLocalChildSession { conversation, .. } => {
                    requests_for_events.borrow_mut().push(conversation.id());
                }
                TuiOrchestrationEvent::CreateLocalChildSession { .. }
                | TuiOrchestrationEvent::KillLocalChildSession { .. }
                | TuiOrchestrationEvent::RemoveChildSession(_) => {}
            },
        );
    });
    requests
}

fn restore_descendants(
    app: &mut App,
    parent_conversation_id: AIConversationId,
    root_session_id: TuiSessionId,
) {
    app.update(|ctx| {
        TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
            model.restore_descendant_sessions(parent_conversation_id, root_session_id, ctx);
        });
    });
}

fn materialize_local_child(
    app: &mut App,
    fixture: &OrchestrationFixture,
    conversation_id: AIConversationId,
) -> TuiSessionId {
    let conversation = app.read(|ctx| {
        BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .cloned()
            .expect("本地子会话应已载入")
    });
    let (view, manager) = add_test_terminal_session(app, fixture.window_id);
    let session_id = app.update(|ctx| {
        TuiSessions::register_session(&fixture.sessions, view.clone(), manager, false, ctx)
    });
    view.update(app, |view, ctx| {
        view.restore_orchestrated_child_conversation(conversation, ctx);
    });
    app.update(|ctx| {
        TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
            model.register_restored_local_oz_child_session(session_id, conversation_id, ctx);
        });
    });
    session_id
}

#[test]
fn restoring_parent_requests_local_oz_descendants_in_spawn_order() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_parent_session(&mut app, &fixture);
        let parent_id = active_conversation_id(&app, parent_session_id);
        let child_id = seed_child(&mut app, parent_id, "child", Harness::Oz);
        let grandchild_id = seed_child(&mut app, child_id, "grandchild", Harness::Oz);
        let requests = restore_requests(&mut app);

        restore_descendants(&mut app, parent_id, parent_session_id);

        assert_eq!(requests.borrow().as_slice(), &[child_id, grandchild_id]);
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            1,
            "请求恢复本身不应重新启动或提前创建本地进程"
        );
    });
}

#[test]
fn restoring_parent_twice_does_not_duplicate_local_child_sessions() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_parent_session(&mut app, &fixture);
        let parent_id = active_conversation_id(&app, parent_session_id);
        let child_id = seed_child(&mut app, parent_id, "child", Harness::Oz);
        let requests = restore_requests(&mut app);

        restore_descendants(&mut app, parent_id, parent_session_id);
        let child_session_id = materialize_local_child(&mut app, &fixture, child_id);
        restore_descendants(&mut app, parent_id, parent_session_id);

        assert_eq!(requests.borrow().as_slice(), &[child_id]);
        assert_eq!(
            app.read_model(&fixture.sessions, |sessions, _| sessions.len()),
            2
        );
        app.read(|ctx| {
            let session = TuiSessions::as_ref(ctx)
                .session(child_session_id)
                .expect("恢复后的本地子会话应保持注册");
            assert!(matches!(session.view(), TuiSessionView::Terminal(_)));
        });
    });
}

#[test]
fn restore_skips_remote_and_non_oz_children() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_parent_session(&mut app, &fixture);
        let parent_id = active_conversation_id(&app, parent_session_id);
        let local_id = seed_child(&mut app, parent_id, "local", Harness::Oz);
        let _non_oz_id = seed_child(&mut app, parent_id, "codex", Harness::Codex);
        let remote_id = seed_child(&mut app, parent_id, "remote", Harness::Oz);
        app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _| {
                history
                    .conversation_mut(&remote_id)
                    .expect("远端测试会话应存在")
                    .mark_as_remote_child();
            });
        });
        let requests = restore_requests(&mut app);

        restore_descendants(&mut app, parent_id, parent_session_id);

        assert_eq!(requests.borrow().as_slice(), &[local_id]);
    });
}

#[test]
fn discard_restored_descendants_removes_projections_without_deleting_history() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_parent_session(&mut app, &fixture);
        let parent_id = active_conversation_id(&app, parent_session_id);
        let child_id = seed_child(&mut app, parent_id, "child", Harness::Oz);
        let child_session_id = materialize_local_child(&mut app, &fixture, child_id);
        let sessions_for_events = fixture.sessions.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&TuiOrchestrationModel::handle(ctx), move |_, event, ctx| {
                if let TuiOrchestrationEvent::RemoveChildSession(session_id) = event {
                    sessions_for_events.update(ctx, |sessions, ctx| {
                        sessions.remove_session(*session_id, ctx);
                    });
                }
            });
        });

        app.update(|ctx| {
            TuiOrchestrationModel::handle(ctx).update(ctx, |model, ctx| {
                model.discard_restored_descendant_sessions(parent_id, parent_session_id, ctx);
            });
        });

        app.read(|ctx| {
            assert!(TuiSessions::as_ref(ctx).session(child_session_id).is_none());
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&child_id)
                    .is_some(),
                "移除 TUI 投影时必须保留历史记录"
            );
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(parent_session_id)
            );
        });
    });
}

#[test]
fn restored_local_oz_child_materializes_terminal_session_without_relaunch() {
    App::test((), |mut app| async move {
        let fixture = orchestration_fixture(&mut app);
        let parent_session_id = add_parent_session(&mut app, &fixture);
        let parent_id = active_conversation_id(&app, parent_session_id);
        let child_id = seed_child(&mut app, parent_id, "local-child", Harness::Oz);

        let child_session_id = materialize_local_child(&mut app, &fixture, child_id);

        app.read(|ctx| {
            let session = TuiSessions::as_ref(ctx)
                .session(child_session_id)
                .expect("恢复后的子会话应已注册");
            assert!(matches!(session.view(), TuiSessionView::Terminal(_)));
            assert_eq!(
                TuiSessions::as_ref(ctx).focused_session_id(),
                Some(parent_session_id),
                "恢复子会话不应抢占父会话焦点"
            );
            let snapshot = TuiOrchestrationModel::as_ref(ctx)
                .snapshot(parent_id, ctx)
                .expect("恢复后的子会话应出现在编排快照中");
            let child = snapshot
                .children
                .iter()
                .find(|child| child.conversation_id == child_id)
                .expect("恢复后的子会话应可导航");
            assert_eq!(child.label, "local-child");
        });
    });
}
