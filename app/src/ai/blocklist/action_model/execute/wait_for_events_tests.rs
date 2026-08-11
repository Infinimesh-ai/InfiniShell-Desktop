//! Unit tests for the pure helpers in `wait_for_events`, plus an App-based
//! test of the executor's conversation-status wiring.

use std::time::Duration;

use warpui::{App, EntityId};

use super::{
    AnyActionExecution, CLIENT_WATCHDOG_SAFETY_MARGIN, DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS,
    ExecuteActionInput, HARD_FLOOR, WaitForEventsExecutor, watchdog_timeout_for_stamped_seconds,
};
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};
use crate::ai::blocklist::BlocklistAIHistoryModel;

#[test]
fn watchdog_timeout_constants_match_documented_values() {
    // The behavioural tests below assert the contract; this trips if
    // someone moves a constant without updating the documented intent.
    assert_eq!(DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS, 30 * 60);
    assert_eq!(CLIENT_WATCHDOG_SAFETY_MARGIN, Duration::from_secs(30));
    assert_eq!(HARD_FLOOR, Duration::from_secs(5));
}

#[test]
fn watchdog_timeout_subtracts_margin_for_stamped_minute() {
    // A 60s stamped timeout has 30s of headroom after subtracting the
    // safety margin — that's the canonical "happy path" the safety
    // margin is designed for.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(60),
        Duration::from_secs(30)
    );
}

#[test]
fn watchdog_timeout_clamps_to_hard_floor_when_stamped_value_is_too_small() {
    // A 10s stamped timeout would become negative after subtracting the
    // 30s safety margin — the hard floor kicks in so the watchdog still
    // fires after a finite delay.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(10),
        HARD_FLOOR,
        "stamped 10s should clamp to HARD_FLOOR after subtracting the safety margin"
    );
}

#[test]
fn watchdog_timeout_falls_back_to_default_minus_margin_when_unset() {
    // Prost flattens scalars, so the proto's "unset" looks like `0` on
    // the Rust side; treat that as "use the default minus margin".
    let expected = Duration::from_secs(DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS as u64)
        - CLIENT_WATCHDOG_SAFETY_MARGIN;
    assert_eq!(watchdog_timeout_for_stamped_seconds(0), expected);
}

#[test]
fn watchdog_timeout_clamps_negative_value_to_default_minus_margin() {
    // Defense against a buggy or malicious payload. `Duration::from_secs`
    // takes a `u64`; a negative value would underflow without the clamp.
    let expected = Duration::from_secs(DEFAULT_ORCHESTRATED_IDLE_TIMEOUT_SECONDS as u64)
        - CLIENT_WATCHDOG_SAFETY_MARGIN;
    assert_eq!(watchdog_timeout_for_stamped_seconds(-42), expected);
}

#[test]
fn watchdog_timeout_preserves_large_stamped_value() {
    // Server-supplied values well above the margin pass through as
    // (stamped - margin). 15 minutes stays at 14m30s after the
    // subtraction.
    assert_eq!(
        watchdog_timeout_for_stamped_seconds(900),
        Duration::from_secs(900) - CLIENT_WATCHDOG_SAFETY_MARGIN
    );
}

#[test]
fn execute_flips_conversation_into_waiting_for_events() {
    // Zap:上游这里还断言 `OrchestrationEventStreamer` 的 parent 注册会发出
    // `get_ambient_agent_task` 拉取;该 streamer 依赖云端 server_api,未挂载,
    // 因此只保留本地可验证的部分 —— 异步执行 + 会话状态翻转。
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], vec![], &[]));

        let executor = app.add_model(|ctx| WaitForEventsExecutor::new(terminal_view_id, ctx));

        // Child conversation: own run_id plus a parent_agent_id.
        let mut conversation = AIConversation::new(false, false);
        conversation.set_run_id("550e8400-e29b-41d4-a716-446655440530".to_string());
        conversation.set_parent_agent_id("550e8400-e29b-41d4-a716-4466554405fc".to_string());
        let conversation_id = conversation.id();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model.update_conversation_status(
                terminal_view_id,
                conversation_id,
                ConversationStatus::InProgress,
                ctx,
            );
        });

        let action = AIAgentAction {
            id: AIAgentActionId::from("wait-action".to_string()),
            action: AIAgentActionType::WaitForEvents {
                tool_call_id: "tool-call-1".to_string(),
                idle_timeout_seconds: 600,
            },
            task_id: TaskId::new("wait-task".to_string()),
            requires_result: false,
        };

        let execution = executor.update(&mut app, |executor, ctx| {
            let input = ExecuteActionInput {
                action: &action,
                conversation_id,
            };
            let result: AnyActionExecution = executor.execute(input, ctx).into();
            result
        });
        assert!(
            matches!(execution, AnyActionExecution::Async { .. }),
            "WaitForEvents should yield an async execution"
        );

        for _ in 0..3 {
            futures_lite::future::yield_now().await;
        }

        history_model.read(&app, |model, _| {
            assert!(
                matches!(
                    model.conversation(&conversation_id).map(|c| c.status()),
                    Some(ConversationStatus::WaitingForEvents)
                ),
                "execute() must flip the conversation into WaitingForEvents"
            );
        });
    });
}
