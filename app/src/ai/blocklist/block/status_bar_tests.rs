use std::cell::RefCell;
use std::rc::Rc;

use warp_core::features::FeatureFlag;
use warpui::{App, EntityId, SingletonEntity, ViewHandle};

use super::{BlocklistAIStatusBar, BlocklistAIStatusBarEvent};
use crate::ai::agent::AIAgentExchangeId;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::test_util::ai_agent_tasks::{create_api_task, create_message};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn restored_status_bar(
    ctx: &mut App,
) -> (
    ViewHandle<BlocklistAIStatusBar>,
    AIConversationId,
    [AIAgentExchangeId; 2],
    EntityId,
) {
    initialize_app_for_terminal_view(ctx);
    let terminal = add_window_with_terminal(ctx, None);
    let terminal_surface_id = terminal.id();
    let status_bar = terminal.read(ctx, |terminal, ctx| {
        terminal.input().as_ref(ctx).agent_status_bar().clone()
    });
    let mut first_message = create_message("first-message", "root");
    first_message.request_id = "first-request".to_owned();
    let mut second_message = create_message("second-message", "root");
    second_message.request_id = "second-request".to_owned();
    let conversation = AIConversation::new_restored(
        AIConversationId::new(),
        vec![create_api_task("root", vec![first_message, second_message])],
        None,
    )
    .unwrap();
    let conversation_id = conversation.id();
    let exchanges = conversation.all_exchanges();
    assert_eq!(exchanges.len(), 2);
    let exchange_ids = [exchanges[0].id, exchanges[1].id];
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
        history.restore_conversations(terminal_surface_id, vec![conversation], ctx);
    });
    (
        status_bar,
        conversation_id,
        exchange_ids,
        terminal_surface_id,
    )
}

fn observe_dialog_events(
    status_bar: &ViewHandle<BlocklistAIStatusBar>,
    ctx: &mut App,
) -> Rc<RefCell<Vec<bool>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed_events = events.clone();
    ctx.update(|ctx| {
        ctx.subscribe_to_view(status_bar, move |_, event, _| {
            if let BlocklistAIStatusBarEvent::SummarizationCancelDialogToggled { is_open } = event {
                observed_events.borrow_mut().push(*is_open);
            }
        });
    });
    events
}

#[test]
fn inactive_exchange_updates_preserve_dialog_after_repeated_switches() {
    App::test((), |mut app| async move {
        let _tips = FeatureFlag::AgentTips.override_enabled(false);
        let (status_bar, conversation_id, [first, second], terminal_surface_id) =
            restored_status_bar(&mut app);
        let history = BlocklistAIHistoryModel::handle(&app);
        let task_id = history.read(&app, |history, _| {
            history
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone()
        });
        for exchange_id in [first, second, first, second] {
            history.update(&mut app, |_, ctx| {
                ctx.emit(BlocklistAIHistoryEvent::AppendedExchange {
                    exchange_id,
                    conversation_id,
                    task_id: task_id.clone(),
                    terminal_surface_id,
                    is_hidden: false,
                    response_stream_id: None,
                });
            });
        }
        let dialog_events = observe_dialog_events(&status_bar, &mut app);
        status_bar.update(&mut app, |bar, ctx| {
            bar.open_summarization_cancel_dialog(ctx);
        });

        history.update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                exchange_id: first,
                conversation_id,
                terminal_surface_id,
                is_hidden: false,
            });
        });

        assert_eq!(*dialog_events.borrow(), vec![true]);
        status_bar.read(&app, |bar, _| {
            assert!(bar.is_summarization_cancel_dialog_open);
        });
    });
}

#[test]
fn current_exchange_output_closes_dialog_across_terminal_surfaces() {
    App::test((), |mut app| async move {
        let _tips = FeatureFlag::AgentTips.override_enabled(false);
        let (status_bar, conversation_id, [_, exchange_id], terminal_surface_id) =
            restored_status_bar(&mut app);
        status_bar.update(&mut app, |bar, ctx| {
            bar.reset_model_for_exchange(exchange_id, conversation_id, ctx);
        });
        let dialog_events = observe_dialog_events(&status_bar, &mut app);
        let other_surface_id = EntityId::new();
        assert_ne!(other_surface_id, terminal_surface_id);

        for event_surface_id in [terminal_surface_id, other_surface_id] {
            status_bar.update(&mut app, |bar, ctx| {
                bar.open_summarization_cancel_dialog(ctx);
            });
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |_, ctx| {
                ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                    exchange_id,
                    conversation_id,
                    terminal_surface_id: event_surface_id,
                    is_hidden: false,
                });
            });
        }

        assert_eq!(*dialog_events.borrow(), vec![true, false, true, false]);
    });
}

#[test]
fn repeated_exchange_activation_does_not_reprocess_the_same_output_event() {
    App::test((), |mut app| async move {
        let _tips = FeatureFlag::AgentTips.override_enabled(false);
        let (status_bar, conversation_id, [first, second], terminal_surface_id) =
            restored_status_bar(&mut app);
        status_bar.update(&mut app, |bar, ctx| {
            bar.reset_model_for_exchange(second, conversation_id, ctx);
        });
        let dialog_events = observe_dialog_events(&status_bar, &mut app);
        let reopened_status_bar = status_bar.clone();
        app.update(|ctx| {
            // 此监听器注册在两次激活之间；第一次处理输出后重新打开的对话框，
            // 不应再被同一事件触发的残留回调关闭。
            ctx.subscribe_to_model(
                &BlocklistAIHistoryModel::handle(ctx),
                move |_, event, ctx| {
                    if let BlocklistAIHistoryEvent::UpdatedStreamingExchange { exchange_id, .. } =
                        event
                        && *exchange_id == second
                    {
                        reopened_status_bar.update(ctx, |bar, ctx| {
                            bar.open_summarization_cancel_dialog(ctx);
                        });
                    }
                },
            );
        });
        // 在事件派发外切换，使上述监听器的注册顺序保持明确。
        status_bar.update(&mut app, |bar, ctx| {
            bar.reset_model_for_exchange(first, conversation_id, ctx);
            bar.reset_model_for_exchange(second, conversation_id, ctx);
            bar.open_summarization_cancel_dialog(ctx);
        });

        BlocklistAIHistoryModel::handle(&app).update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                exchange_id: second,
                conversation_id,
                terminal_surface_id,
                is_hidden: false,
            });
        });

        assert_eq!(*dialog_events.borrow(), vec![true, false, true]);
        status_bar.read(&app, |bar, _| {
            assert!(bar.is_summarization_cancel_dialog_open);
        });
    });
}

#[test]
fn global_usage_update_preserves_current_dialog() {
    App::test((), |mut app| async move {
        let _tips = FeatureFlag::AgentTips.override_enabled(false);
        let (status_bar, conversation_id, [_, exchange_id], _) = restored_status_bar(&mut app);
        status_bar.update(&mut app, |bar, ctx| {
            bar.reset_model_for_exchange(exchange_id, conversation_id, ctx);
        });
        let dialog_events = observe_dialog_events(&status_bar, &mut app);
        status_bar.update(&mut app, |bar, ctx| {
            bar.open_summarization_cancel_dialog(ctx);
        });
        let other_conversation_id = AIConversationId::new();
        assert_ne!(other_conversation_id, conversation_id);

        BlocklistAIHistoryModel::handle(&app).update(&mut app, |_, ctx| {
            ctx.emit(BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated {
                conversation_id: other_conversation_id,
            });
        });

        assert_eq!(*dialog_events.borrow(), vec![true]);
    });
}
