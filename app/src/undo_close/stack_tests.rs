use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, TimeZone};
use warpui::r#async::FutureExt as _;
use warpui::{App, SingletonEntity};

use super::ClosedItem;
use crate::ai::agent::{
    AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus, FinishedAIAgentOutput,
    Shared,
};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::llms::LLMId;
use crate::banner::BannerState;
use crate::pane_group::PaneGroup;
use crate::resource_center::TipsCompleted;
use crate::workspace::view::tests::{initialize_app, mock_workspace};

#[test]
fn undo_close_cleanup_retains_the_last_pane_handle_until_cleanup_finishes() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        let pane_group = app.update(|ctx| {
            let tips = ctx.add_model(|_| TipsCompleted::default());
            let banner = ctx.add_model(|_| BannerState::default());
            // 不放入 workspace.tabs，使测试能够释放调用方持有的最后一个强句柄。
            ctx.add_view(workspace.window_id(ctx), |ctx| {
                PaneGroup::new_with_panes_layout(
                    tips,
                    banner,
                    Default::default(),
                    Arc::new(HashMap::new()),
                    None,
                    ctx,
                )
            })
        });
        let terminal_view_id = pane_group.read(&app, |pane_group, ctx| {
            pane_group
                .focused_session_view(ctx)
                .expect("测试 pane 应包含终端")
                .id()
        });
        let conversation_id = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let conversation_id =
                    history.start_new_conversation(terminal_view_id, false, false, false, ctx);
                // 空会话不会进入历史；使用可见提问验证延迟清理前确实完成归档。
                history
                    .conversation_mut(&conversation_id)
                    .expect("测试会话应存在")
                    .append_root_exchange_for_test(AIAgentExchange {
                        id: AIAgentExchangeId::new(),
                        input: vec![AIAgentInput::UserQuery {
                            query: "保留关闭后的会话历史".to_owned(),
                            context: Default::default(),
                            static_query_type: None,
                            referenced_attachments: Default::default(),
                            user_query_mode: Default::default(),
                            running_command: None,
                            intended_agent: None,
                        }],
                        output_status: AIAgentOutputStatus::Finished {
                            finished_output: FinishedAIAgentOutput::Success {
                                output: Shared::new(Default::default()),
                            },
                        },
                        added_message_ids: Default::default(),
                        start_time: Local.timestamp_opt(0, 0).unwrap(),
                        finish_time: None,
                        time_to_first_token_ms: None,
                        working_directory: None,
                        model_id: LLMId::from("test-model"),
                        request_cost: None,
                        coding_model_id: LLMId::from("test-model"),
                        cli_agent_model_id: LLMId::from("test-model"),
                        computer_use_model_id: LLMId::from("test-model"),
                        response_initiator: None,
                    });
                conversation_id
            })
        });

        pane_group.update(&mut app, |_, ctx| {
            assert!(ctx.is_window_open(pane_group.window_id(ctx)));
            let history = BlocklistAIHistoryModel::handle(ctx);
            ClosedItem::mark_conversations_historical_for_pane_group(&pane_group, &history, ctx);
            ClosedItem::clean_up_pane_group(&pane_group, ctx);
            // 清理必须等待当前 pane 借用结束，不能在这里重入。
            assert!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .all_live_conversations_for_terminal_surface(terminal_view_id)
                    .next()
                    .is_some()
            );
            assert!(
                history
                    .as_ref(ctx)
                    .get_conversation_metadata(&conversation_id)
                    .is_none()
            );
        });
        let weak_pane_group = pane_group.downgrade();
        drop(pane_group);
        app.update(|ctx| {
            assert!(weak_pane_group.upgrade(ctx).is_some());
        });

        async {
            while app.read(|ctx| {
                BlocklistAIHistoryModel::as_ref(ctx)
                    .all_live_conversations_for_terminal_surface(terminal_view_id)
                    .next()
                    .is_some()
            }) {
                futures_lite::future::yield_now().await;
            }
        }
        .with_timeout(Duration::from_secs(5))
        .await
        .expect("借用结束后应自动清理终端会话");
        app.read(|ctx| {
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .all_live_conversations_for_terminal_surface(terminal_view_id)
                    .count(),
                0
            );
            let metadata = BlocklistAIHistoryModel::as_ref(ctx)
                .get_conversation_metadata(&conversation_id)
                .expect("已清理的可见会话仍应可从历史导航访问");
            assert_eq!(metadata.initial_query, "保留关闭后的会话历史");
            assert!(weak_pane_group.upgrade(ctx).is_none());
        });
    });
}
