//! 手动原生布局验收的静态长回复会话，不访问模型或执行工具。

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::vec2f;
use warp_multi_agent_api as api;
use warpui::integration::TestStep;
use warpui::{App, TypedActionView, ViewHandle, WindowId, async_assert};

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::agent::{
    AIAgentOutputStatus, FinishedAIAgentOutput, MessageId, RenderableAIError, Shared,
};
use crate::ai::api_error::AIApiError;
use crate::ai::blocklist::block::{AIBlock, AIBlockAction};
use crate::features::FeatureFlag;
use crate::i18n;
use crate::integration_testing::step::new_step_with_default_assertions;
use crate::integration_testing::view_getters::single_terminal_view_for_tab;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};

const LONG_RESPONSE_VISUAL_MESSAGE_ID: &str = "long-response-visual-output";

fn long_response_visual_text() -> String {
    (0..48)
        .map(|index| format!(
            "Paragraph {index:02}: The complete response stays available for reading and copying. 段落 {index:02}：这是一段用于确认滚动区域、折叠操作与完整复制的固定测试内容。\n\n"
        ))
        .collect()
}

fn long_response_error_visual_conversation() -> AIConversation {
    let task_id = "long-response-visual-task";
    let request_id = "long-response-visual-request";
    let text = long_response_visual_text();
    assert!(text.chars().count() > 4_000);
    let mut conversation = AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: task_id.to_owned(),
            messages: vec![
                api::Message {
                    id: "long-response-visual-query".to_owned(),
                    task_id: task_id.to_owned(),
                    request_id: request_id.to_owned(),
                    message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                        query: "Long response preview / 较长回复预览".to_owned(),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                api::Message {
                    id: LONG_RESPONSE_VISUAL_MESSAGE_ID.to_owned(),
                    task_id: task_id.to_owned(),
                    request_id: request_id.to_owned(),
                    message: Some(api::message::Message::AgentOutput(
                        api::message::AgentOutput { text },
                    )),
                    ..Default::default()
                },
                api::Message {
                    id: "long-response-visual-tool".to_owned(),
                    task_id: task_id.to_owned(),
                    request_id: request_id.to_owned(),
                    message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                        tool_call_id: "separate-tool-card".to_owned(),
                        tool: Some(api::message::tool_call::Tool::RunShellCommand(
                            api::message::tool_call::RunShellCommand {
                                command: "printf 'separate tool card\\n'".to_owned(),
                                is_read_only: true,
                                ..Default::default()
                            },
                        )),
                    })),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        None,
    )
    .expect("静态长回复会话应能恢复");
    let exchange_id = conversation.latest_exchange().expect("应有回复").id;
    let exchange = conversation
        .get_exchange_to_update(exchange_id)
        .expect("应有回复");
    let output = exchange.output_status.output().map(Shared::get_owned);
    let error = Arc::new(AIApiError::ProviderProtocol(crate::t!(
        "ai-error-repeated-response"
    )));
    exchange.output_status = AIAgentOutputStatus::Finished {
        finished_output: FinishedAIAgentOutput::Error {
            output,
            error: RenderableAIError::from(&error),
        },
    };
    conversation
}

fn long_response_visual_block(
    app: &App,
    window_id: WindowId,
    conversation_id: Option<AIConversationId>,
) -> Option<ViewHandle<AIBlock>> {
    let conversation_id = conversation_id?;
    // 切换会话时旧会话的入口卡片可能追加到末尾，不能把最后一张富内容卡片当作目标回复。
    app.views_of_type::<AIBlock>(window_id)?
        .into_iter()
        .rev()
        .find(|block| block.read(app, |block, _| block.conversation_id() == conversation_id))
}

/// 手动原生截图：同一生产长回复组件和错误卡片覆盖双语、折叠、展开与完整复制。
pub fn long_response_error_visual_steps() -> Vec<TestStep> {
    FeatureFlag::AgentView.set_enabled(true);
    let mut steps = Vec::new();
    for locale in ["en", "zh-CN"] {
        let target_conversation = Rc::new(Cell::new(None));
        let restored_target = target_conversation.clone();
        let visible_target = target_conversation.clone();
        steps.push(
            new_step_with_default_assertions("恢复长回复和重复停止错误").with_action(
                move |app, window_id, _| {
                    i18n::init(Some(locale));
                    i18n::set_locale(locale);
                    let bounds = app.window_bounds(&window_id).expect("应有真实窗口");
                    app.update(|ctx| {
                        ctx.set_and_cache_window_bounds(
                            window_id,
                            RectF::new(bounds.origin(), vec2f(1000., 900.)),
                        )
                    });
                    single_terminal_view_for_tab(app, window_id, 0).update(app, |view, ctx| {
                        let conversation = long_response_error_visual_conversation();
                        restored_target.set(Some(conversation.id()));
                        view.restore_conversation_after_view_creation(
                            RestoredAIConversation::new(conversation),
                            true,
                            RestoreConversationEntryBehavior::EnterRestoredConversation,
                            ctx,
                        );
                    });
                },
            ).add_named_assertion("目标会话已切换且回复区块已完成布局", move |app, window_id| {
                let target_id = visible_target.get();
                let Some(block) = long_response_visual_block(app, window_id, target_id) else {
                    return async_assert!(false, "等待目标会话的长回复区块：{target_id:?}");
                };
                let active_id = single_terminal_view_for_tab(app, window_id, 0).read(app, |view, ctx| {
                    view.agent_view_controller().as_ref(ctx).agent_view_state().active_conversation_id()
                });
                let position_id = block.read(app, |block, _| block.saved_position_id());
                let presenter = app.presenter(window_id).expect("应有窗口渲染器");
                let has_layout = presenter.borrow().position_cache().get_position(&position_id).is_some();
                async_assert!(active_id == target_id && has_layout,
                    "等待目标会话可见布局：target={target_id:?}, active={active_id:?}, has_layout={has_layout}")
            }),
        );
        let expanded_height = Rc::new(Cell::new(0.0));
        for (stage, toggle, expanded) in [
            ("expanded", false, true),
            ("collapsed", true, false),
            ("expanded-again", true, true),
        ] {
            let expanded_height = expanded_height.clone();
            let action_target = target_conversation.clone();
            let content_target = target_conversation.clone();
            let layout_target = target_conversation.clone();
            steps.push(
                TestStep::new(&format!("核对长回复布局 {locale} {stage}"))
                    .with_action(move |app, window_id, _| {
                        let block = long_response_visual_block(app, window_id, action_target.get())
                            .expect("已完成布局的目标回复区块应继续存在");
                        block.update(app, |block, ctx| {
                            if toggle {
                                block.handle_action(
                                    &AIBlockAction::ToggleCollapsibleBlockExpanded(MessageId::new(
                                        LONG_RESPONSE_VISUAL_MESSAGE_ID.to_owned(),
                                    )),
                                    ctx,
                                );
                            }
                            block.handle_action(&AIBlockAction::CopyOutput, ctx);
                            ctx.notify();
                        });
                    })
                    .add_named_assertion(
                        "正文完整、错误可见且工具卡片独立",
                        move |app, window_id| {
                            let Some(block) =
                                long_response_visual_block(app, window_id, content_target.get())
                            else {
                                return async_assert!(false, "目标回复区块应继续存在");
                            };
                            block.read(app, |block, ctx| {
                                async_assert!(
                                    block
                                        .get_output_text(ctx)
                                        .contains(long_response_visual_text().trim_end())
                                        && block.status(ctx).error().is_some()
                                        && block.num_requested_commands() == 1,
                                    "展开状态不能删除正文、错误或独立工具卡片"
                                )
                            })
                        },
                    )
                    .add_named_assertion("折叠确实改变可视高度", move |app, window_id| {
                        let Some(block) =
                            long_response_visual_block(app, window_id, layout_target.get())
                        else {
                            return async_assert!(false, "目标回复区块应继续存在");
                        };
                        let position_id = block.read(app, |block, _| block.saved_position_id());
                        let presenter = app.presenter(window_id).expect("应有窗口渲染器");
                        let bounds = presenter
                            .borrow()
                            .position_cache()
                            .get_position(&position_id);
                        let Some(bounds) = bounds else {
                            return async_assert!(false, "等待回复区块完成布局");
                        };
                        let height = bounds.height();
                        if !toggle {
                            expanded_height.set(height);
                            async_assert!(height > 360., "默认展开应显示有界正文区域")
                        } else if expanded {
                            async_assert!(
                                height >= expanded_height.get() - 2.,
                                "重新展开应恢复可视高度"
                            )
                        } else {
                            async_assert!(
                                height < expanded_height.get() - 100.,
                                "收起应隐藏正文区域"
                            )
                        }
                    })
                    .add_named_assertion("复制包含完整正文", |app, _| {
                        let clipboard = app.update(|ctx| ctx.clipboard().read());
                        async_assert!(
                            clipboard
                                .plain_text
                                .contains(long_response_visual_text().trim_end()),
                            "复制输出应保留滚动区域以外的全部正文"
                        )
                    })
                    .with_take_screenshot(format!("long-response-{locale}-{stage}.png")),
            );
        }
    }
    steps
}
