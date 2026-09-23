use std::collections::HashMap;
use std::sync::Arc;

use warpui::platform::WindowStyle;
use warpui::{App, EntityId, ViewHandle};

use super::*;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::features::FeatureFlag;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn mock_pane_group(app: &mut App) -> ViewHandle<PaneGroup> {
    let tips_model = app.add_model(|_| TipsCompleted::default());
    let (_, pane_group) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let banner_model = ctx.add_model(|_| BannerState::default());
        PaneGroup::new_with_panes_layout(
            tips_model,
            banner_model,
            Default::default(),
            Arc::new(HashMap::new()),
            None,
            ctx,
        )
    });
    pane_group
}

fn start_parent_conversation(
    panes: &PaneGroup,
    parent_pane_id: PaneId,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let parent_terminal_view_id = panes
        .terminal_view_from_pane_id(parent_pane_id, ctx)
        .expect("parent pane should have a terminal view")
        .id();
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
        history_model.start_new_conversation(parent_terminal_view_id, false, false, false, ctx)
    })
}

fn restore_conversation_for_terminal_view(
    terminal_view_id: EntityId,
    conversation: AIConversation,
    ctx: &mut ViewContext<PaneGroup>,
) -> AIConversationId {
    let conversation_id = conversation.id();
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
        history_model.restore_conversations(terminal_view_id, vec![conversation], ctx);
    });
    conversation_id
}

#[test]
fn test_ensure_hidden_child_agent_pane_restores_child_from_detached_group() {
    let _agent_view = FeatureFlag::AgentView.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let closed_pane_group = mock_pane_group(&mut app);
        let reopened_pane_group = mock_pane_group(&mut app);

        let (parent_conversation, child_conversation_id, previous_owner) = closed_pane_group
            .update(&mut app, |panes, ctx| {
                let parent_pane_id = panes.focused_pane_id(ctx);
                let parent_conversation_id = start_parent_conversation(panes, parent_pane_id, ctx);
                let mut child = AIConversation::new(false, false);
                child.set_parent_conversation_id(parent_conversation_id);
                child.set_agent_name("architect".to_string());
                let child_conversation_id = child.id();
                panes.create_hidden_child_agent_pane(child, parent_pane_id, ctx);

                let previous_owner = panes
                    .terminal_view_from_pane_id(
                        panes.child_agent_panes[&child_conversation_id],
                        ctx,
                    )
                    .unwrap();
                let parent_conversation = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&parent_conversation_id)
                    .unwrap()
                    .clone();
                panes.swap_active_pane_to_conversation(parent_pane_id, child_conversation_id, ctx);
                panes.detach_panes(ctx);

                (parent_conversation, child_conversation_id, previous_owner)
            });

        let restored_child = reopened_pane_group.update(&mut app, |panes, ctx| {
            let parent_pane_id = panes.focused_pane_id(ctx);
            let parent_view = panes
                .terminal_view_from_pane_id(parent_pane_id, ctx)
                .unwrap();
            let parent_conversation_id =
                restore_conversation_for_terminal_view(parent_view.id(), parent_conversation, ctx);
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.set_active_conversation_id(parent_conversation_id, parent_view.id(), ctx);
            });

            assert!(
                panes.ensure_hidden_child_agent_pane_for_conversation(child_conversation_id, ctx)
            );
            let child_pane_id = *panes
                .child_agent_panes
                .get(&child_conversation_id)
                .expect("an undo-retained owner must not prevent restoring the child");
            panes.swap_active_pane_to_conversation(parent_pane_id, child_conversation_id, ctx);

            assert_eq!(panes.focused_pane_id(ctx), child_pane_id);
            let child_view = panes
                .terminal_view_from_pane_id(child_pane_id, ctx)
                .unwrap();
            assert_eq!(
                child_view.as_ref(ctx).active_conversation_id(ctx),
                Some(child_conversation_id)
            );
            child_view
        });

        assert_ne!(restored_child.id(), previous_owner.id());
        closed_pane_group.update(&mut app, |panes, ctx| {
            panes.reattach_panes(ctx);
            assert!(!panes.child_agent_panes.contains_key(&child_conversation_id));
            assert_eq!(
                panes.find_pane_id_for_terminal_view(previous_owner.id(), ctx),
                None
            );
            assert_eq!(panes.visible_pane_count(), 1);
            assert!(
                panes.ensure_hidden_child_agent_pane_for_conversation(child_conversation_id, ctx)
            );
        });
        closed_pane_group.update(&mut app, |panes, ctx| {
            panes.clean_up_panes(ctx);
        });
        app.read(|ctx| {
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .terminal_surface_id_for_conversation(&child_conversation_id),
                Some(restored_child.id())
            );
            assert_eq!(
                restored_child.as_ref(ctx).active_conversation_id(ctx),
                Some(child_conversation_id)
            );
        });
    });
}
