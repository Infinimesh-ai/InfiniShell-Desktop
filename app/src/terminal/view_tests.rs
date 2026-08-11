use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::pin::pin;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Local;
use parking_lot::FairMutex;
use warp_cli::agent::Harness;
use warp_multi_agent_api as api;
use warp_terminal::model::escape_sequences::{BRACKETED_PASTE_END, BRACKETED_PASTE_START, C0};
use warpui::notification::UserNotification;
use warpui::platform::WindowStyle;
use warpui::{App, EntityIdSet, Presenter, WindowInvalidation};

use super::*;
use crate::ai::active_agent_views_model::ActiveAgentViewsModel;
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionId, AIAgentExchange, AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus,
    AgentReviewCommentBatch, UserQueryMode,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::agent_view::agent_input_footer::toolbar_item::AgentToolbarItemKind;
use crate::ai::blocklist::agent_view::{
    AgentViewEntryBlock, AgentViewEntryOrigin, AgentViewState, EnterAgentBlockAction,
    ExitAgentViewError,
};
use crate::ai::blocklist::block::cli_controller::{
    LongRunningCommandControlState, UserTakeOverReason,
};
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, InputConfig, InputType, ResponseStream,
    ResponseStreamId, SerializedBlockListItem,
};
use crate::ai::llms::LLMId;
use crate::code_review::comments::{
    AttachedReviewComment, AttachedReviewCommentTarget, CommentOrigin,
};
use crate::context_chips::prompt::Prompt;
use crate::editor::{AutosuggestionLocation, AutosuggestionType};
use crate::features::FeatureFlag;
use crate::pane_group::focus_state::PaneGroupFocusState;
use crate::pane_group::{BackingView, TerminalPaneId};
use crate::persistence::model::AgentConversationData;
use crate::settings::import::model::ImportedConfigModel;
use crate::settings::{AISettings, AppEditorSettings, WarpPromptSeparator};
use crate::terminal::alt_screen::should_intercept_mouse;
use crate::terminal::block_list_element::{SnackbarPoint, SnackbarTranslationMode};
use crate::terminal::block_list_viewport::{ClampingMode, ScrollLines};
use crate::terminal::cli_agent_sessions::event::{
    CLI_AGENT_NOTIFICATION_SENTINEL, CLIAgentEvent, CLIAgentEventPayload, CLIAgentEventSource,
    CLIAgentEventType,
};
use crate::terminal::cli_agent_sessions::listener::CLIAgentSessionListener;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentRichInputCloseReason, CLIAgentSession,
    CLIAgentSessionContext, CLIAgentSessionStatus, CLIAgentSessionsModel,
};
use crate::terminal::model::ansi::{self, BootstrappedValue, InitShellValue, PreexecValue};
use crate::terminal::model::block::{
    AgentInteractionMetadata, AgentViewVisibility, BlockId, SerializedAIMetadata, SerializedBlock,
};
use crate::terminal::model::blocks::{TotalIndex, insert_block};
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::terminal_model::WithinBlock;
use crate::terminal::session_settings::AgentToolbarChipSelection;
use crate::terminal::shared_session::{SharedSessionSource, SharedSessionStatus};
use crate::terminal::view::ambient_agent::AmbientAgentViewModelEvent;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};
use crate::terminal::{CLIAgent, MockTerminalManager, TerminalManager, TerminalModel};
use crate::test_util::ai_agent_tasks::{
    create_api_subtask, create_api_task, create_message, create_subagent_tool_call_message,
};
use crate::test_util::terminal::{
    add_window_with_id_and_terminal, initialize_app_for_terminal_view,
};
use crate::test_util::{add_window_with_terminal, assert_eventually};
use crate::view_components::find::FindWithinBlockState;
use crate::workspace::ToastStack;

fn add_window_with_cloud_mode_terminal(app: &mut App) -> ViewHandle<TerminalView> {
    let tips_model = app.add_model(|_| Default::default());
    let (_, terminal) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        TerminalView::new_for_test_with_cloud_mode(tips_model, None, true, ctx)
    });
    terminal.update(app, |view, _| {
        view.model.lock().set_is_dummy_cloud_mode_session(true);
    });
    terminal
}

// Zap:`owned_resumable_oz_task` 只服务于已删除的云端 follow-up 测试,随之移除。

/// The AI blocks currently flagged to render the transcript-navigation ring.
fn navigation_ring_targets(view: &TerminalView, app: &AppContext) -> Vec<EntityId> {
    view.rich_content_views
        .iter()
        .filter_map(|rich_content| {
            let ai_metadata = rich_content.ai_block_metadata()?;
            ai_metadata
                .ai_block_handle
                .as_ref(app)
                .is_agent_transcript_navigation_target()
                .then(|| ai_metadata.ai_block_handle.id())
        })
        .collect()
}

fn has_pending_user_query_block(view: &TerminalView) -> bool {
    let Some(view_id) = view.pending_user_query_view_id else {
        return false;
    };
    view.rich_content_views.iter().any(|rich_content| {
        rich_content.view_id() == view_id && rich_content.is_pending_user_query()
    })
}

fn tool_call_message_with_tool_for_test(
    id: &str,
    call_id: &str,
    tool: api::message::tool_call::Tool,
) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: "root-task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        // proto b0886a95 新增字段;本地无记忆检索结果。
        fetched_memories: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: call_id.to_string(),
            tool: Some(tool),
        })),
        request_id: "request-1".to_string(),
        timestamp: None,
    }
}

fn tool_call_result_message_with_result_for_test(
    id: &str,
    call_id: &str,
    result: api::message::tool_call_result::Result,
) -> api::Message {
    api::Message {
        id: id.to_string(),
        task_id: "root-task".to_string(),
        server_message_data: String::new(),
        citations: vec![],
        // proto b0886a95 新增字段;本地无记忆检索结果。
        fetched_memories: vec![],
        message: Some(api::message::Message::ToolCallResult(
            api::message::ToolCallResult {
                tool_call_id: call_id.to_string(),
                result: Some(result),
                context: None,
            },
        )),
        request_id: "request-1".to_string(),
        timestamp: None,
    }
}

fn run_shell_command_tool_for_test(command: &str) -> api::message::tool_call::Tool {
    use api::message::tool_call::run_shell_command::WaitUntilCompleteValue;

    // CLI subagent 会挂在这条 shell command block 上；字段只保留序列化所需的最小值。
    api::message::tool_call::Tool::RunShellCommand(api::message::tool_call::RunShellCommand {
        command: command.to_string(),
        is_read_only: true,
        uses_pager: false,
        is_risky: false,
        citations: vec![],
        wait_until_complete_value: Some(WaitUntilCompleteValue::WaitUntilComplete(true)),
        risk_category: 0,
    })
}

#[allow(deprecated)]
fn build_restored_conversation_with_cli_subagent_for_test(
    block_id: BlockId,
    task_id: TaskId,
) -> AIConversation {
    // 构造包含 CLI subagent tool call 的历史 conversation，模拟普通 /agent 历史恢复数据。
    let root_task_id = "root-task";
    let block_id_string = String::from(block_id);
    let task_id_string = String::from(task_id);
    let root_task = create_api_task(
        root_task_id,
        vec![
            tool_call_message_with_tool_for_test(
                "run-shell-call",
                "run-shell-call-1",
                run_shell_command_tool_for_test("ssh jump"),
            ),
            tool_call_result_message_with_result_for_test(
                "run-shell-result",
                "run-shell-call-1",
                api::message::tool_call_result::Result::RunShellCommand(
                    api::RunShellCommandResult {
                        command: "ssh jump".to_string(),
                        output: String::new(),
                        exit_code: 0,
                        result: Some(api::run_shell_command_result::Result::CommandFinished(
                            api::ShellCommandFinished {
                                command_id: block_id_string.clone(),
                                output: "jump output".to_string(),
                                exit_code: 0,
                                // proto b0886a95 新增的起止时间戳;测试数据不关心时序。
                                start_ts: None,
                                finish_ts: None,
                            },
                        )),
                    },
                ),
            ),
            create_subagent_tool_call_message(
                "cli-subagent-call",
                root_task_id,
                &task_id_string,
                Some(api::message::tool_call::subagent::Metadata::Cli(
                    api::message::tool_call::subagent::CliSubagent {
                        command_id: block_id_string,
                    },
                )),
            ),
        ],
    );
    let subtask = create_api_subtask(
        &task_id_string,
        root_task_id,
        vec![create_message("cli-subagent-output", &task_id_string)],
    );

    AIConversation::new_restored(AIConversationId::new(), vec![root_task, subtask], None)
        .expect("restored CLI subagent conversation should build")
}

fn empty_agent_conversation_data_for_test() -> AgentConversationData {
    AgentConversationData {
        server_conversation_token: None,
        conversation_usage_metadata: None,
        reverted_action_ids: None,
        forked_from_server_conversation_token: None,
        artifacts_json: None,
        parent_agent_id: None,
        agent_name: None,
        orchestration_harness_type: None,
        parent_conversation_id: None,
        is_remote_child: false,
        root_task_is_optimistic: None,
        run_id: None,
        autoexecute_override: None,
        last_event_sequence: None,
        pinned: false,
        compaction_state_json: None,
        byop_repair_state_json: None,
        cli_subagent_block_snapshots_json: None,
    }
}

fn cli_subagent_snapshot_json_for_test(
    conversation_id: AIConversationId,
    task_id: &TaskId,
    block_id: &BlockId,
    output: &[u8],
) -> String {
    // 模拟关闭标签后随 conversation_data 留存的完整终端 block 快照。
    let mut block = SerializedBlock::new_for_test(b"ssh jump".to_vec(), output.to_vec());
    block.id = block_id.clone();
    block.ai_metadata = serde_json::to_string(&Some(Into::<SerializedAIMetadata>::into(
        AgentInteractionMetadata::new(
            None,
            conversation_id,
            Some(task_id.clone()),
            Some(LongRunningCommandControlState::Agent {
                is_blocked: false,
                should_hide_responses: false,
            }),
            false,
            false,
        ),
    )))
    .ok();
    block.agent_view_visibility =
        Some(AgentViewVisibility::new_from_conversation(conversation_id).into());

    serde_json::to_string(&vec![serde_json::json!({
        "task_id": String::from(task_id.clone()),
        "block_id": block_id.as_str(),
        "block": block,
    })])
    .expect("CLI subagent snapshot JSON should serialize")
}

#[allow(deprecated)]
fn build_restored_conversation_with_cli_subagent_snapshot_for_test(
    conversation_id: AIConversationId,
    block_id: BlockId,
    task_id: TaskId,
    snapshot_output: &[u8],
) -> AIConversation {
    let mut conversation_data = empty_agent_conversation_data_for_test();
    conversation_data.cli_subagent_block_snapshots_json = Some(
        cli_subagent_snapshot_json_for_test(conversation_id, &task_id, &block_id, snapshot_output),
    );

    // task result 故意保留短输出，确保测试验证的是 snapshot 恢复而不是 task fallback。
    let root_task_id = "root-task";
    let block_id_string = String::from(block_id);
    let task_id_string = String::from(task_id);
    let root_task = create_api_task(
        root_task_id,
        vec![
            tool_call_message_with_tool_for_test(
                "run-shell-call",
                "run-shell-call-1",
                run_shell_command_tool_for_test("ssh jump"),
            ),
            tool_call_result_message_with_result_for_test(
                "run-shell-result",
                "run-shell-call-1",
                api::message::tool_call_result::Result::RunShellCommand(
                    api::RunShellCommandResult {
                        command: "ssh jump".to_string(),
                        output: String::new(),
                        exit_code: 0,
                        result: Some(api::run_shell_command_result::Result::CommandFinished(
                            api::ShellCommandFinished {
                                command_id: block_id_string.clone(),
                                output: "short task output".to_string(),
                                exit_code: 0,
                                // proto b0886a95 新增的起止时间戳;测试数据不关心时序。
                                start_ts: None,
                                finish_ts: None,
                            },
                        )),
                    },
                ),
            ),
            create_subagent_tool_call_message(
                "cli-subagent-call",
                root_task_id,
                &task_id_string,
                Some(api::message::tool_call::subagent::Metadata::Cli(
                    api::message::tool_call::subagent::CliSubagent {
                        command_id: block_id_string,
                    },
                )),
            ),
        ],
    );
    let subtask = create_api_subtask(
        &task_id_string,
        root_task_id,
        vec![create_message("cli-subagent-output", &task_id_string)],
    );

    AIConversation::new_restored(
        conversation_id,
        vec![root_task, subtask],
        Some(conversation_data),
    )
    .expect("restored CLI subagent conversation with snapshot should build")
}

fn build_restored_conversation_without_cli_subagent_for_test() -> AIConversation {
    // 构造普通 /agent 历史 conversation，用于确认没有 CLI subagent 时不会误建浮窗。
    let root_task_id = "root-task";
    let root_task = create_api_task(
        root_task_id,
        vec![create_message("ordinary-agent-output", root_task_id)],
    );

    AIConversation::new_restored(AIConversationId::new(), vec![root_task], None)
        .expect("ordinary restored conversation should build")
}

fn serialized_blocks_for_restored_cli_subagent_for_test(
    conversation: &AIConversation,
) -> Vec<SerializedBlockListItem> {
    // 走真实持久化序列化路径，避免测试只验证内存中的临时 view 状态。
    let serialized_blocks = conversation.to_serialized_blocklist_items();
    assert_eq!(
        serialized_blocks.len(),
        1,
        "CLI subagent conversation should serialize one command block"
    );
    serialized_blocks
}

fn clear_ai_metadata_for_serialized_blocks_for_test(blocks: &mut [SerializedBlockListItem]) {
    // 模拟旧历史记录或损坏记录缺少 ai_metadata 的情况，恢复逻辑应安全跳过。
    for item in blocks {
        match item {
            SerializedBlockListItem::Command { block } => block.ai_metadata = None,
        }
    }
}

#[test]
fn restores_cli_subagent_view_from_serialized_history_blocks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let block_id = BlockId::from("cli-block-1".to_string());
        let task_id = TaskId::new("cli-task-1".to_string());
        let conversation = build_restored_conversation_with_cli_subagent_for_test(
            block_id.clone(),
            task_id.clone(),
        );
        let serialized_blocks = serialized_blocks_for_restored_cli_subagent_for_test(&conversation);

        let terminal = add_window_with_terminal(&mut app, Some(&serialized_blocks));
        terminal.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(
                view.cli_subagent_views.contains_key(&block_id),
                "restored CLI subagent view should be keyed by its command block id"
            );
        });
    });
}

#[test]
fn restores_cli_subagent_terminal_output_from_persisted_snapshot() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let conversation_id = AIConversationId::new();
        let block_id = BlockId::from("cli-block-snapshot".to_string());
        let task_id = TaskId::new("cli-task-snapshot".to_string());
        let snapshot_output =
            b"* Documentation: https://help.ubuntu.com\r\nCONTAINER ID   IMAGE\r\nanalyzer-runtime\r\n";
        let conversation = build_restored_conversation_with_cli_subagent_snapshot_for_test(
            conversation_id,
            block_id.clone(),
            task_id,
            snapshot_output,
        );
        let serialized_blocks = serialized_blocks_for_restored_cli_subagent_for_test(&conversation);

        let terminal = add_window_with_terminal(&mut app, Some(&serialized_blocks));
        terminal.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(
                view.cli_subagent_views.contains_key(&block_id),
                "snapshot-restored CLI subagent view should still be expandable"
            );
            let model = view.model.lock();
            let block = model
                .block_list()
                .block_with_id(&block_id)
                .expect("snapshot-restored command block should exist");
            let serialized_block = SerializedBlock::from(block);
            let output = String::from_utf8_lossy(&serialized_block.stylized_output);
            assert!(
                output.contains("analyzer-runtime"),
                "restored terminal block should contain persisted SSH output: {output}"
            );
            assert!(
                !output.contains("short task output"),
                "task fallback output should not replace persisted terminal snapshot"
            );
        });
    });
}

#[test]
fn restores_cli_subagent_snapshot_when_history_blocks_are_not_preloaded() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let conversation_id = AIConversationId::new();
        let block_id = BlockId::from("cli-block-missing-preload".to_string());
        let task_id = TaskId::new("cli-task-missing-preload".to_string());
        let conversation = build_restored_conversation_with_cli_subagent_snapshot_for_test(
            conversation_id,
            block_id.clone(),
            task_id,
            b"ssh jump\r\ndocker ps -a\r\nanalyzer-runtime\r\n",
        );

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            // 历史列表进入已有 terminal 时不会像新 pane 初始化那样预置 restored blocks；
            // restore_conversation_after_view_creation 必须自己补回 CLI subagent 的终端快照。
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(
                view.cli_subagent_views.contains_key(&block_id),
                "未预置 restored blocks 时也应恢复 CLI subagent 只读卡片"
            );
            let model = view.model.lock();
            let block = model
                .block_list()
                .block_with_id(&block_id)
                .expect("CLI subagent snapshot block should be restored into the terminal model");
            let serialized_block = SerializedBlock::from(block);
            let output = String::from_utf8_lossy(&serialized_block.stylized_output);
            assert!(
                output.contains("analyzer-runtime"),
                "历史入口恢复时应显示持久化的 SSH 终端输出: {output}"
            );
        });
    });
}

#[test]
fn exiting_restored_cli_subagent_agent_view_inserts_entry_card() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let conversation_id = AIConversationId::new();
        let block_id = BlockId::from("cli-block-agent-view-exit".to_string());
        let task_id = TaskId::new("cli-task-agent-view-exit".to_string());
        let conversation = build_restored_conversation_with_cli_subagent_snapshot_for_test(
            conversation_id,
            block_id.clone(),
            task_id,
            b"ssh jump\r\ndocker ps -a\r\nanalyzer-runtime\r\n",
        );
        let serialized_blocks = serialized_blocks_for_restored_cli_subagent_for_test(&conversation);

        let terminal = add_window_with_terminal(&mut app, Some(&serialized_blocks));
        terminal.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            view.enter_agent_view_for_conversation(
                None,
                AgentViewEntryOrigin::AgentViewBlock,
                conversation_id,
                ctx,
            );
            view.handle_action(&TerminalAction::ExitAgentView, ctx);
        });

        terminal.read(&app, |view, _| {
            assert!(
                view.last_visible_item_is_agent_view_block_for_conversation(conversation_id),
                "ESC 返回终端后应保留 restored CLI subagent 的折叠入口卡片"
            );
            assert!(
                view.cli_subagent_views.contains_key(&block_id),
                "CLI subagent 只读卡片仍应可展开"
            );
            let model = view.model.lock();
            let block = model
                .block_list()
                .block_with_id(&block_id)
                .expect("snapshot-restored command block should exist after exiting agent view");
            let serialized_block = SerializedBlock::from(block);
            let output = String::from_utf8_lossy(&serialized_block.stylized_output);
            assert!(
                output.contains("analyzer-runtime"),
                "退出 AgentView 后仍应保留 SSH snapshot 输出: {output}"
            );
        });
    });
}

fn assert_exiting_restored_ordinary_agent_view_inserts_entry_card(origin: AgentViewEntryOrigin) {
    App::test((), move |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let conversation = build_restored_conversation_without_cli_subagent_for_test();
        let conversation_id = conversation.id();
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            // 普通 restored 会话只是在 AgentView 中查看，没有新增 exchange；
            // ESC 回到 terminal 后仍需要保留一个可再次进入的折叠入口。
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            view.enter_agent_view_for_conversation(None, origin, conversation_id, ctx);
            view.handle_action(&TerminalAction::ExitAgentView, ctx);
        });

        terminal.read(&app, |view, _| {
            assert!(
                view.last_visible_item_is_agent_view_block_for_conversation(conversation_id),
                "ESC 返回 terminal 后应保留普通 restored /agent 会话的折叠入口卡片"
            );
            assert!(
                view.cli_subagent_views.is_empty(),
                "普通 restored /agent 会话不应创建 CLI subagent view"
            );
        });
    });
}

#[test]
fn exiting_restored_ordinary_agent_view_inserts_entry_card_from_history() {
    assert_exiting_restored_ordinary_agent_view_inserts_entry_card(
        AgentViewEntryOrigin::ConversationListView,
    );
}

#[test]
fn exiting_restored_ordinary_agent_view_inserts_entry_card_from_entry_block() {
    assert_exiting_restored_ordinary_agent_view_inserts_entry_card(
        AgentViewEntryOrigin::AgentViewBlock,
    );
}

#[test]
fn skips_cli_subagent_view_restore_without_matching_ai_metadata() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let block_id = BlockId::from("cli-block-1".to_string());
        let task_id = TaskId::new("cli-task-1".to_string());
        let conversation =
            build_restored_conversation_with_cli_subagent_for_test(block_id.clone(), task_id);
        let mut serialized_blocks =
            serialized_blocks_for_restored_cli_subagent_for_test(&conversation);
        clear_ai_metadata_for_serialized_blocks_for_test(&mut serialized_blocks);

        let terminal = add_window_with_terminal(&mut app, Some(&serialized_blocks));
        terminal.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(
                !view.cli_subagent_views.contains_key(&block_id),
                "missing metadata should not restore a CLI subagent view"
            );
            assert!(
                !view.rich_content_views.is_empty(),
                "normal /agent blocks should still restore without CLI metadata"
            );
        });
    });
}

#[test]
fn ordinary_agent_restore_does_not_create_cli_subagent_views() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let conversation = build_restored_conversation_without_cli_subagent_for_test();
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(
                view.cli_subagent_views.is_empty(),
                "ordinary /agent restore should not create CLI subagent views"
            );
            assert!(
                !view.rich_content_views.is_empty(),
                "ordinary /agent block should still restore"
            );
        });
    });
}

#[test]
fn finished_cli_subagent_keeps_read_only_card_when_metadata_matches() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        // FinishedSubagent 会触发 sidecar 持久化，需要 GlobalResourceHandlesProvider——
        // 它已由 `initialize_app_for_terminal_view` →
        // `initialize_history_persistence_for_tests` 注册,这里不再重复注册。

        // 先恢复一个带 CLI subagent 的历史会话，建立带匹配 metadata 的 command block
        // 和一个 RestoredReadOnly 视图，模拟 SSH 会话在 agent view 里展开后的状态。
        // 用带 snapshot 的构造函数，确保 conversation_id 可控且与 block metadata 一致。
        let block_id = BlockId::from("cli-block-finished".to_string());
        let task_id = TaskId::new("cli-task-finished".to_string());
        let conversation_id = AIConversationId::new();
        let conversation = build_restored_conversation_with_cli_subagent_snapshot_for_test(
            conversation_id,
            block_id.clone(),
            task_id.clone(),
            b"ssh session output",
        );
        let serialized_blocks = serialized_blocks_for_restored_cli_subagent_for_test(&conversation);

        let terminal = add_window_with_terminal(&mut app, Some(&serialized_blocks));
        terminal.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        // 恢复后应存在 CLI subagent 视图。
        terminal.read(&app, |view, _| {
            assert!(
                view.cli_subagent_views.contains_key(&block_id),
                "restored CLI subagent view should exist before FinishedSubagent"
            );
        });

        // 模拟 SSH 会话结束触发的 FinishedSubagent 事件。
        // 修复前：live 视图被 remove 后不再重建，卡片消失；
        // 修复后：在 block 仍持有匹配 metadata 时以 RestoredReadOnly 重建折叠卡片。
        terminal.update(&mut app, |view, ctx| {
            view.handle_cli_subagent_controller_event(
                view.cli_subagent_controller.clone(),
                &CLISubagentEvent::FinishedSubagent {
                    block_id: block_id.clone(),
                    task_id: task_id.clone(),
                    conversation_id: Some(conversation_id),
                    initial_requested_command_action_id: None,
                },
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(
                view.cli_subagent_views.contains_key(&block_id),
                "FinishedSubagent should keep a read-only CLI subagent card when metadata matches"
            );
        });
    });
}

#[test]
fn finished_cli_subagent_skips_rebuild_when_block_missing() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        // 用一个不存在于 block_list 的 block_id 触发 FinishedSubagent，
        // 模拟 block 已被回收 / metadata 无法匹配的情况，重建前置检查应不通过。
        let block_id = BlockId::from("cli-block-gone".to_string());
        let task_id = TaskId::new("cli-task-gone".to_string());
        let conversation_id = AIConversationId::new();

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            view.handle_cli_subagent_controller_event(
                view.cli_subagent_controller.clone(),
                &CLISubagentEvent::FinishedSubagent {
                    block_id: block_id.clone(),
                    task_id: task_id.clone(),
                    conversation_id: Some(conversation_id),
                    initial_requested_command_action_id: None,
                },
                ctx,
            );
        });

        terminal.read(&app, |view, _| {
            assert!(
                !view.cli_subagent_views.contains_key(&block_id),
                "FinishedSubagent should not rebuild a card when the block no longer exists"
            );
        });
    });
}

#[cfg(windows)]
#[test]
fn restored_cli_subagent_windows_ctrl_c_does_not_write_to_pty() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            // 让 Ctrl-C 的 live 路径具备可观察副作用：如果被转发，会写 ETX 到 PTY。
            view.model
                .lock()
                .simulate_long_running_block("sleep 10", "running");

            view.handle_cli_subagent_view_event(
                view.view_id,
                &CLISubagentViewEvent::WindowsCtrlC,
                false,
                ctx,
            );
        });
        assert!(
            pty_writes.borrow().is_empty(),
            "restored CLI subagent Ctrl-C should not reach the live terminal"
        );

        terminal.update(&mut app, |view, ctx| {
            view.handle_cli_subagent_view_event(
                view.view_id,
                &CLISubagentViewEvent::WindowsCtrlC,
                true,
                ctx,
            );
        });
        assert_eq!(
            pty_writes.borrow().as_slice(),
            &[vec![warp_terminal::model::escape_sequences::C0::ETX]],
            "live CLI subagent Ctrl-C should still forward to the active terminal"
        );
    });
}

#[test]
fn agent_view_lifecycle_updates_input_mode() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        // Zap:上游这个用例隐含依赖「测试用户未登录 ⇒ `is_any_ai_enabled` 为 false ⇒ 终端里
        // NLD 自动识别关闭」。我方 `is_any_ai_enabled` 恒为 true(去掉了账号门禁),NLD 默认开着,
        // 于是退出 agent view 后空 buffer 会被 `PendingQueryStateUpdated` 重新标成 AI
        // (未锁定的“未分类”态,见 `terminal::input` 里那段注释)——那是 NLD 的行为,不是
        // agent view 生命周期的行为。这里显式关掉 NLD,断言才落在被测的那条转换上。
        AISettings::handle(&app).update(&mut app, |ai_settings, ctx| {
            let _ = ai_settings
                .nld_in_terminal_enabled_internal
                .set_value(false, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.ai_input_model().as_ref(ctx).input_type(),
                InputType::Shell
            );
        });
        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("agent view entry should succeed");
            });
        });
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.ai_input_model().as_ref(ctx).input_type(),
                InputType::AI
            );
        });
        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller.exit_agent_view_without_confirmation(ctx)
            });
        });
        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.ai_input_model().as_ref(ctx).input_type(),
                InputType::Shell
            );
        });
    });
}

#[test]
fn cmd_up_in_agent_view_navigates_prompts_and_user_shell_blocks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        let (prompt_view_ids, user_shell_index) = terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_agent_view_for_navigation(view, ctx);

            // prompt 1 (user-query AI block — navigable)
            append_inputs_to_conversation_and_handle_event(
                view,
                conversation_id,
                vec![agent_view_user_query_input("prompt 1")],
                ctx,
            );

            // Agent-requested run-shell (unhidden) and agent-monitored command: not navigable.
            {
                let mut model = view.model.lock();
                model.simulate_block("tool-call", "tool-result");
                let tool_index = model.block_list().blocks().len() - 2;
                let action_id = AIAgentActionId::from("tool-action".to_owned());
                {
                    let block = &mut model.block_list_mut().blocks_mut()[tool_index];
                    block.set_conversation_id(conversation_id);
                    block.set_agent_interaction_mode(
                        crate::terminal::model::block::AgentInteractionMetadata::new_hidden(
                            action_id.clone(),
                            conversation_id,
                        ),
                    );
                }
                model
                    .block_list_mut()
                    .set_visibility_of_block_for_ai_action(&action_id, true);

                model.simulate_block("agent-monitored", "output");
                let monitored_index = model.block_list().blocks().len() - 2;
                let block = &mut model.block_list_mut().blocks_mut()[monitored_index];
                block.set_conversation_id(conversation_id);
                block.set_agent_interaction_mode(
                    crate::terminal::model::block::AgentInteractionMetadata::new(
                        None,
                        conversation_id,
                        None,
                        None,
                        false,
                        false,
                    ),
                );
            }

            // Post-tool-call agent-reply AI segment: production mounts this as a second
            // AIBlock after run-shell. ResumeConversation has no displayable user query,
            // so has_user_input is false and Cmd-Up must skip it.
            append_inputs_to_conversation_and_handle_event(
                view,
                conversation_id,
                vec![AIAgentInput::ResumeConversation {
                    context: Default::default(),
                }],
                ctx,
            );

            // user-executed shell command (navigable)
            let user_shell_index =
                simulate_user_shell_block_in_conversation(view, conversation_id, "user-shell");

            // prompt 2 (user-query AI block — navigable)
            append_inputs_to_conversation_and_handle_event(
                view,
                conversation_id,
                vec![agent_view_user_query_input("prompt 2")],
                ctx,
            );

            // Collect AI rich-content blocks and classify via production navigable list.
            let ai_view_ids: Vec<_> = view
                .rich_content_views
                .iter()
                .filter_map(|rc| rc.ai_block_metadata().map(|meta| meta.ai_block_handle.id()))
                .collect();
            assert!(
                ai_view_ids.len() >= 3,
                "expected query + agent-reply + query AI blocks, got {}",
                ai_view_ids.len()
            );

            let navigable = view
                .model
                .lock()
                .block_list()
                .agent_transcript_navigable_items();
            let navigable_ai: Vec<_> = navigable
                .iter()
                .filter_map(|item| match item {
                    crate::terminal::model::blocks::AgentTranscriptNavigableItem::AiBlock {
                        view_id,
                    } => Some(*view_id),
                    _ => None,
                })
                .collect();
            assert_eq!(
                navigable_ai.len(),
                2,
                "only user-query AI segments should be navigable, got {navigable:?}"
            );
            // Agent-reply segment must not appear in navigable AI stops.
            for ai_id in &ai_view_ids {
                if !navigable_ai.contains(ai_id) {
                    // Non-navigable AI block present — good (the post-tool reply).
                    continue;
                }
            }
            assert!(
                ai_view_ids.iter().any(|id| !navigable_ai.contains(id)),
                "expected at least one non-navigable agent-reply AI block among {ai_view_ids:?}"
            );

            (navigable_ai, user_shell_index)
        });

        let prompt_1 = *prompt_view_ids.first().expect("prompt 1");
        let prompt_2 = *prompt_view_ids.last().expect("prompt 2");

        terminal.update(&mut app, |view, ctx| {
            // From the bottom: first Cmd-Up lands on latest prompt.
            view.select_less_recent_block(false /* is_shift_down */, ctx);
            assert_eq!(
                view.agent_transcript_selection_for_test(),
                Some(
                    crate::terminal::model::blocks::AgentTranscriptNavigableItem::AiBlock {
                        view_id: prompt_2
                    }
                )
            );
            assert_eq!(view.selected_blocks.tail(), None);

            // Next Cmd-Up lands on the user shell command.
            view.select_less_recent_block(false, ctx);
            assert_eq!(
                view.agent_transcript_selection_for_test(),
                Some(
                    crate::terminal::model::blocks::AgentTranscriptNavigableItem::ShellBlock(
                        user_shell_index
                    )
                )
            );
            assert_eq!(view.selected_blocks.tail(), Some(user_shell_index));

            // Next Cmd-Up lands on the first prompt, skipping the tool call.
            view.select_less_recent_block(false, ctx);
            assert_eq!(
                view.agent_transcript_selection_for_test(),
                Some(
                    crate::terminal::model::blocks::AgentTranscriptNavigableItem::AiBlock {
                        view_id: prompt_1
                    }
                )
            );
            assert_eq!(view.selected_blocks.tail(), None);

            // Another Cmd-Up at the oldest item stays put.
            view.select_less_recent_block(false, ctx);
            assert_eq!(
                view.agent_transcript_selection_for_test(),
                Some(
                    crate::terminal::model::blocks::AgentTranscriptNavigableItem::AiBlock {
                        view_id: prompt_1
                    }
                )
            );
            assert_eq!(view.selected_blocks.tail(), None);

            // Cmd-Down symmetry walks back toward the latest prompt.
            view.select_more_recent_block(true, false, ctx);
            assert_eq!(
                view.agent_transcript_selection_for_test(),
                Some(
                    crate::terminal::model::blocks::AgentTranscriptNavigableItem::ShellBlock(
                        user_shell_index
                    )
                )
            );
            view.select_more_recent_block(true, false, ctx);
            assert_eq!(
                view.agent_transcript_selection_for_test(),
                Some(
                    crate::terminal::model::blocks::AgentTranscriptNavigableItem::AiBlock {
                        view_id: prompt_2
                    }
                )
            );
        });
    })
}

#[test]
fn cmd_down_past_newest_transcript_item_scrolls_to_end() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_agent_view_for_navigation(view, ctx);
            append_inputs_to_conversation_and_handle_event(
                view,
                conversation_id,
                vec![agent_view_user_query_input("prompt 1")],
                ctx,
            );
            // Enough conversation blocks that the transcript is taller than the viewport.
            for _ in 0..100 {
                simulate_user_shell_block_in_conversation(view, conversation_id, "user-shell");
            }
            assert!(view.is_vertically_scrollable(ctx));

            // Select the newest navigable stop, then scroll the viewport to the top.
            view.select_less_recent_block(false /* is_shift_down */, ctx);
            assert!(view.agent_transcript_selection_for_test().is_some());
            view.update_scroll_position_locking(ScrollPositionUpdate::AfterHome, ctx);
            assert!(matches!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition { .. }
            ));

            // Cmd-Down past the newest stop must land the viewport on the true end of
            // the blocklist and clear the navigation selection.
            view.select_more_recent_block(true, false, ctx);
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
            assert_eq!(view.agent_transcript_selection_for_test(), None);
            assert_eq!(view.selected_blocks.tail(), None);
        });
    })
}

#[test]
fn cmd_down_past_newest_preserves_viewport_when_already_at_end() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_agent_view_for_navigation(view, ctx);
            append_inputs_to_conversation_and_handle_event(
                view,
                conversation_id,
                vec![agent_view_user_query_input("prompt 1")],
                ctx,
            );
            simulate_user_shell_block_in_conversation(view, conversation_id, "user-shell");
            assert!(!view.is_vertically_scrollable(ctx));

            view.select_less_recent_block(false /* is_shift_down */, ctx);
            assert!(view.agent_transcript_selection_for_test().is_some());
            view.update_scroll_position_locking(ScrollPositionUpdate::AfterHome, ctx);
            assert!(matches!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition { .. }
            ));

            // The whole transcript fits in the viewport, so it is already at the end:
            // Cmd-Down past the newest stop must not touch the scroll position.
            view.select_more_recent_block(true, false, ctx);
            assert!(matches!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition { .. }
            ));
            assert_eq!(view.agent_transcript_selection_for_test(), None);
        });
    })
}

#[test]
fn cmd_down_without_navigation_cursor_is_a_no_op() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_agent_view_for_navigation(view, ctx);
            append_inputs_to_conversation_and_handle_event(
                view,
                conversation_id,
                vec![agent_view_user_query_input("prompt 1")],
                ctx,
            );
            simulate_user_shell_block_in_conversation(view, conversation_id, "user-shell");

            // At the end of the transcript with no navigation cursor there is nothing to
            // move toward, so Cmd-Down must leave the cursor, the selection, the ring and
            // the viewport untouched.
            let scroll_position = view.scroll_position();
            view.select_more_recent_block(
                true,  /* is_cmd_down */
                false, /* is_shift_down */
                ctx,
            );
            assert_eq!(view.agent_transcript_selection_for_test(), None);
            assert_eq!(view.selected_blocks.tail(), None);
            assert!(navigation_ring_targets(view, ctx).is_empty());
            assert_eq!(view.scroll_position(), scroll_position);

            // Cmd-Up takes the newest stop and Cmd-Down past it clears the cursor; a
            // further Cmd-Down must not re-select that stop (no oscillation).
            view.select_less_recent_block(false /* is_shift_down */, ctx);
            assert!(view.agent_transcript_selection_for_test().is_some());
            view.select_more_recent_block(true, false, ctx);
            assert_eq!(view.agent_transcript_selection_for_test(), None);

            view.select_more_recent_block(true, false, ctx);
            assert_eq!(view.agent_transcript_selection_for_test(), None);
            assert_eq!(view.selected_blocks.tail(), None);
            assert!(navigation_ring_targets(view, ctx).is_empty());
        });
    })
}

#[test]
fn agent_transcript_navigation_marks_target_user_query() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        let (prompt_view_ids, user_shell_index) = terminal.update(&mut app, |view, ctx| {
            let conversation_id = enter_agent_view_for_navigation(view, ctx);
            append_inputs_to_conversation_and_handle_event(
                view,
                conversation_id,
                vec![agent_view_user_query_input("prompt 1")],
                ctx,
            );
            let user_shell_index =
                simulate_user_shell_block_in_conversation(view, conversation_id, "user-shell");
            append_inputs_to_conversation_and_handle_event(
                view,
                conversation_id,
                vec![agent_view_user_query_input("prompt 2")],
                ctx,
            );

            let navigable = view
                .model
                .lock()
                .block_list()
                .agent_transcript_navigable_items();
            let navigable_ai: Vec<_> = navigable
                .iter()
                .filter_map(|item| match item {
                    crate::terminal::model::blocks::AgentTranscriptNavigableItem::AiBlock {
                        view_id,
                    } => Some(*view_id),
                    crate::terminal::model::blocks::AgentTranscriptNavigableItem::ShellBlock(_) => {
                        None
                    }
                })
                .collect();
            assert_eq!(navigable_ai.len(), 2);
            (navigable_ai, user_shell_index)
        });

        let prompt_1 = *prompt_view_ids.first().expect("prompt 1");
        let prompt_2 = *prompt_view_ids.last().expect("prompt 2");

        terminal.update(&mut app, |view, ctx| {
            // No navigation yet: nothing is marked.
            assert_eq!(view.agent_transcript_navigated_ai_block(ctx), None);
            assert!(navigation_ring_targets(view, ctx).is_empty());

            // Every stop on a user query marks exactly that query.
            view.select_less_recent_block(false /* is_shift_down */, ctx);
            assert_eq!(
                view.agent_transcript_navigated_ai_block(ctx),
                Some(prompt_2)
            );
            assert_eq!(navigation_ring_targets(view, ctx), vec![prompt_2]);

            // Stopping on a shell block moves the mark off the query.
            view.select_less_recent_block(false, ctx);
            assert_eq!(view.agent_transcript_navigated_ai_block(ctx), None);
            assert_eq!(view.selected_blocks.tail(), Some(user_shell_index));
            assert!(navigation_ring_targets(view, ctx).is_empty());

            // The mark follows the cursor to the other query.
            view.select_less_recent_block(false, ctx);
            assert_eq!(
                view.agent_transcript_navigated_ai_block(ctx),
                Some(prompt_1)
            );
            assert_eq!(navigation_ring_targets(view, ctx), vec![prompt_1]);

            // Clamped at the oldest stop: the target stays identifiable even though
            // neither the cursor nor the viewport moves.
            view.select_less_recent_block(false, ctx);
            assert_eq!(
                view.agent_transcript_navigated_ai_block(ctx),
                Some(prompt_1)
            );
            assert_eq!(navigation_ring_targets(view, ctx), vec![prompt_1]);

            // Cmd-Down back through the stops, then past the newest one, which clears
            // the mark together with the navigation selection.
            view.select_more_recent_block(true, false, ctx);
            assert_eq!(view.agent_transcript_navigated_ai_block(ctx), None);
            assert!(navigation_ring_targets(view, ctx).is_empty());
            view.select_more_recent_block(true, false, ctx);
            assert_eq!(
                view.agent_transcript_navigated_ai_block(ctx),
                Some(prompt_2)
            );
            assert_eq!(navigation_ring_targets(view, ctx), vec![prompt_2]);
            view.select_more_recent_block(true, false, ctx);
            assert_eq!(view.agent_transcript_navigated_ai_block(ctx), None);
            assert_eq!(view.agent_transcript_selection_for_test(), None);
            assert!(navigation_ring_targets(view, ctx).is_empty());

            // Re-mark a query, then exit the agent view below.
            view.select_less_recent_block(false, ctx);
            assert_eq!(
                view.agent_transcript_navigated_ai_block(ctx),
                Some(prompt_2)
            );
            assert_eq!(navigation_ring_targets(view, ctx), vec![prompt_2]);
        });

        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller.exit_agent_view_without_confirmation(ctx)
            });
        });
        terminal.read(&app, |view, ctx| {
            // Leaving the agent view drops the navigation cursor, its mark, and the ring.
            assert_eq!(view.agent_transcript_selection_for_test(), None);
            assert_eq!(view.agent_transcript_navigated_ai_block(ctx), None);
            assert!(navigation_ring_targets(view, ctx).is_empty());
        });
    })
}

#[test]
fn ordinary_block_selection_unchanged_outside_agent_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.simulate_block("ls", "foo");
                model.simulate_block("pwd", "bar");
            }

            // Outside the agent view, Cmd-Up walks ordinary block selection and never
            // produces a query mark.
            view.select_less_recent_block(false /* is_shift_down */, ctx);
            let first = view.selected_blocks.tail().expect("a block gets selected");
            assert_eq!(view.agent_transcript_navigated_ai_block(ctx), None);
            assert!(navigation_ring_targets(view, ctx).is_empty());

            view.select_less_recent_block(false, ctx);
            let second = view.selected_blocks.tail().expect("a block stays selected");
            assert_eq!(second.0 + 1, first.0);
            assert_eq!(view.agent_transcript_navigated_ai_block(ctx), None);
            assert!(navigation_ring_targets(view, ctx).is_empty());
        });
    })
}

#[test]
fn focus_reporting_writes_focus_events_in_normal_screen() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();

        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.simulate_long_running_block("python3 /tmp/warp_focus_test.py", "");
            assert!(!model.is_alt_screen_active());
            ansi::Handler::set_mode(&mut *model, ansi::Mode::ReportFocusInOut);
            assert!(model.is_term_mode_set(TermMode::FOCUS_IN_OUT));
            drop(model);
            assert!(view.should_report_focus(ctx));

            view.maybe_report_focus_out(ctx);
            view.maybe_report_focus_in(ctx);
        });

        assert_eq!(
            *pty_writes.borrow(),
            vec![
                escape_sequences::EscCodes::FOCUS_OUT.to_vec(),
                escape_sequences::EscCodes::FOCUS_IN.to_vec(),
            ]
        );
    })
}

// Zap:`input_operations_for_buffer_content` 只服务于已删除的 Cloud Mode Setup V2
// 输入同步测试,随之移除。

fn exchange_with_inputs(inputs: Vec<AIAgentInput>) -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: inputs,
        output_status: AIAgentOutputStatus::Streaming { output: None },
        added_message_ids: HashSet::new(),
        start_time: Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-agent-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

fn append_exchange_and_handle_event(
    view: &mut TerminalView,
    input: AIAgentInput,
    ctx: &mut ViewContext<TerminalView>,
) -> (
    AIConversationId,
    TaskId,
    AIAgentExchangeId,
    ResponseStreamId,
) {
    append_exchange_with_inputs_and_handle_event(view, vec![input], ctx)
}

fn append_exchange_with_inputs_and_handle_event(
    view: &mut TerminalView,
    inputs: Vec<AIAgentInput>,
    ctx: &mut ViewContext<TerminalView>,
) -> (
    AIConversationId,
    TaskId,
    AIAgentExchangeId,
    ResponseStreamId,
) {
    let history_model = BlocklistAIHistoryModel::handle(ctx);
    let (conversation_id, task_id, exchange_id, response_stream_id) =
        history_model.update(ctx, |history_model, ctx| {
            let conversation_id =
                history_model.start_new_conversation(view.view_id, false, false, false, ctx);
            let task_id = history_model
                .conversation(&conversation_id)
                .expect("conversation should exist")
                .get_root_task_id()
                .clone();
            let response_stream_id = ResponseStreamId::new_for_test();
            let exchange = exchange_with_inputs(inputs);
            let exchange_id = exchange.id;
            history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should exist")
                .append_reassigned_exchange(&response_stream_id, exchange, view.view_id, ctx)
                .expect("exchange should append");
            (conversation_id, task_id, exchange_id, response_stream_id)
        });

    view.handle_ai_history_model_event(
        history_model,
        &BlocklistAIHistoryEvent::AppendedExchange {
            exchange_id,
            task_id: task_id.clone(),
            terminal_surface_id: view.view_id,
            conversation_id,
            is_hidden: false,
            response_stream_id: Some(response_stream_id.clone()),
        },
        ctx,
    );
    (conversation_id, task_id, exchange_id, response_stream_id)
}

fn update_exchange_input_and_handle_event(
    view: &mut TerminalView,
    conversation_id: AIConversationId,
    exchange_id: AIAgentExchangeId,
    response_stream_id: ResponseStreamId,
    inputs: Vec<AIAgentInput>,
    ctx: &mut ViewContext<TerminalView>,
) {
    let history_model = BlocklistAIHistoryModel::handle(ctx);
    history_model.update(ctx, |history_model, ctx| {
        let conversation = history_model
            .conversation_mut(&conversation_id)
            .expect("conversation should exist");
        let mut exchange = conversation
            .remove_exchange(exchange_id)
            .expect("exchange should exist");
        exchange.input = inputs;
        conversation
            .append_reassigned_exchange(&response_stream_id, exchange, view.view_id, ctx)
            .expect("exchange should append");
    });

    view.handle_ai_history_model_event(
        history_model,
        &BlocklistAIHistoryEvent::UpdatedStreamingExchange {
            exchange_id,
            terminal_surface_id: view.view_id,
            conversation_id,
            is_hidden: false,
        },
        ctx,
    );
}

fn enter_agent_view_for_navigation(
    view: &mut TerminalView,
    ctx: &mut ViewContext<TerminalView>,
) -> AIConversationId {
    view.agent_view_controller().update(ctx, |controller, ctx| {
        controller
            .try_enter_agent_view(
                None,
                AgentViewEntryOrigin::Input {
                    was_prompt_autodetected: false,
                },
                ctx,
            )
            .expect("agent view entry should succeed")
    })
}

fn append_inputs_to_conversation_and_handle_event(
    view: &mut TerminalView,
    conversation_id: AIConversationId,
    inputs: Vec<AIAgentInput>,
    ctx: &mut ViewContext<TerminalView>,
) {
    let history_model = BlocklistAIHistoryModel::handle(ctx);
    let (exchange_id, task_id, response_stream_id) =
        history_model.update(ctx, |history_model, ctx| {
            let response_stream_id = ResponseStreamId::new_for_test();
            let exchange = exchange_with_inputs(inputs);
            let exchange_id = exchange.id;
            let task_id = history_model
                .conversation(&conversation_id)
                .expect("conversation should exist")
                .get_root_task_id()
                .clone();
            history_model
                .conversation_mut(&conversation_id)
                .expect("conversation should exist")
                .append_reassigned_exchange(&response_stream_id, exchange, view.view_id, ctx)
                .expect("exchange should append");
            (exchange_id, task_id, response_stream_id)
        });
    view.handle_ai_history_model_event(
        history_model,
        &BlocklistAIHistoryEvent::AppendedExchange {
            exchange_id,
            task_id,
            terminal_surface_id: view.view_id,
            conversation_id,
            is_hidden: false,
            response_stream_id: Some(response_stream_id),
        },
        ctx,
    );
}

fn agent_view_user_query_input(query: &str) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_owned(),
        context: Default::default(),
        static_query_type: None,
        referenced_attachments: Default::default(),
        user_query_mode: UserQueryMode::Normal,
        running_command: None,
        intended_agent: None,
    }
}

fn simulate_user_shell_block_in_conversation(
    view: &mut TerminalView,
    conversation_id: AIConversationId,
    command: &str,
) -> BlockIndex {
    let mut model = view.model.lock();
    model.simulate_block(command, "shell-output");
    let index = model.block_list().blocks().len() - 2;
    model.block_list_mut().blocks_mut()[index].set_conversation_id(conversation_id);
    index.into()
}

fn ai_block_count(view: &TerminalView) -> usize {
    view.rich_content_views
        .iter()
        .filter(|rich_content| {
            matches!(
                rich_content.metadata(),
                Some(RichContentMetadata::AIBlock(_))
            )
        })
        .count()
}

fn agent_view_entry_count_for_conversation(
    view: &TerminalView,
    conversation_id: AIConversationId,
) -> usize {
    view.rich_content_views
        .iter()
        .filter(|rich_content| {
            matches!(
                rich_content.metadata(),
                Some(RichContentMetadata::AgentViewEntry(params))
                    if params.conversation_id == conversation_id
            )
        })
        .count()
}

fn command_block_count_for_conversation(
    view: &TerminalView,
    conversation_id: AIConversationId,
) -> usize {
    view.model
        .lock()
        .block_list()
        .blocks()
        .iter()
        .filter(|block| {
            matches!(
                block.agent_view_visibility(),
                AgentViewVisibility::Agent {
                    origin_conversation_id,
                    ..
                } if *origin_conversation_id == conversation_id
            )
        })
        .count()
}

/// Bootstraps the terminal model with one completed block and one active long-running block.
fn bootstrap_with_long_running_block(view: &mut TerminalView) {
    let mut model = view.model.lock();
    model.init_shell(InitShellValue {
        session_id: 0.into(),
        shell: "zsh".to_owned(),
        ..Default::default()
    });
    model.bootstrapped(BootstrappedValue {
        shell: "zsh".to_owned(),
        ..Default::default()
    });
    model.simulate_block("ls", "file.txt");
    model.simulate_long_running_block("long-command", "output");
}

/// Places the active block in agent-driving-but-not-monitoring state:
/// `requested_command_action_id` is set but `long_running_control_state` is None.
/// This simulates the window between when the agent writes the command to the
/// PTY and when `BlocklistAIHistoryEvent::CreatedSubtask` fires.
fn set_active_block_agent_driving(view: &mut TerminalView, conversation_id: AIConversationId) {
    let action_id = AIAgentActionId::from("test-action".to_owned());
    view.model
        .lock()
        .block_list_mut()
        .active_block_mut()
        .set_agent_interaction_mode_for_requested_command(action_id, None, conversation_id);
}

#[test]
fn updated_conversation_metadata_refreshes_selected_conversation_pane_title() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(false);
        let terminal = add_window_with_terminal(&mut app, None);
        let conversation_id = AIConversationId::new();

        terminal.update(&mut app, |view, ctx| {
            let conversation = AIConversation::new_restored(
                conversation_id,
                vec![warp_multi_agent_api::Task {
                    id: "root-task".to_string(),
                    messages: vec![],
                    dependencies: None,
                    description: "Original title".to_string(),
                    summary: String::new(),
                    server_data: String::new(),
                }],
                None,
            )
            .expect("conversation should restore");

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.restore_conversations(view.view_id, vec![conversation], ctx);
            });
            view.ai_context_model.update(ctx, |context_model, ctx| {
                context_model.set_pending_query_state_for_existing_conversation(
                    conversation_id,
                    AgentViewEntryOrigin::AgentViewBlock,
                    ctx,
                );
            });
            view.update_pane_configuration(ctx);
            assert_eq!(
                view.pane_configuration.as_ref(ctx).title(),
                "Original title"
            );

            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.apply_conversation_title(conversation_id, "Renamed title".to_string(), ctx)
            });
            view.handle_ai_history_model_event(
                BlocklistAIHistoryModel::handle(ctx),
                &BlocklistAIHistoryEvent::UpdatedConversationTitle {
                    terminal_surface_id: Some(view.view_id),
                    conversation_id,
                    title: "Renamed title".to_string(),
                },
                ctx,
            );

            assert_eq!(view.pane_configuration.as_ref(ctx).title(), "Renamed title");
        });
    })
}
struct TestTerminalManager {
    model: Arc<FairMutex<TerminalModel>>,
    _view: ViewHandle<TerminalView>,
}

impl TerminalManager for TestTerminalManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Test to verify that blocks created through normal execution
/// have the correct local status set
#[test]
fn test_create_new_block_with_local_status() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // Set up a terminal with a local session
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();

            // Initialize a local session
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "bash".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                ..Default::default()
            });
        });

        assert_eventually!(
            terminal.read(&app, |view, ctx| !view
                .active_block_is_considered_remote(ctx)),
            "Block should be local"
        );

        // No remote blocks should exist
        assert_eventually!(
            terminal.read(&app, |view, _ctx| !view.contains_restored_remote_blocks()),
            "No remote blocks should exist"
        );

        // Update the view's flags
        // view.update_focused_terminal_info(ctx);
        assert_eventually!(
            terminal.read(&app, |view, _ctx| !view.any_session_contains_remote_blocks),
            "No remote blocks should exist"
        );

        // Now test with a remote session
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();

            // Create a new block with a remote session ID and remote_shell
            model.init_shell(InitShellValue {
                session_id: 1.into(),
                shell: "bash".to_owned(),
                user: "user".to_owned(),
                hostname: "remote".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                ..Default::default()
            });

            // Create a block in the remote session
            model.simulate_block("echo remote", "remote output");
        });

        // Verify block is non-local (remote)
        assert_eventually!(
            terminal.read(&app, |view, ctx| view
                .active_block_is_considered_remote(ctx)),
            "Block should be non-local (remote)"
        );

        // Remote blocks should be detected
        assert_eventually!(
            terminal.read(&app, |view, _ctx| view.any_session_contains_remote_blocks),
            "Remote blocks should be detected"
        );
    })
}

#[test]
fn submit_cli_agent_rich_input_restores_unlocked_input_config() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_agent_rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .auto_dismiss_rich_input_after_submit
                .set_value(true, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.ai_input_model().update(ctx, |ai_input, ctx| {
                    ai_input.set_input_config(
                        InputConfig {
                            input_type: InputType::Shell,
                            is_locked: false,
                        },
                        true,
                        None,
                        ctx,
                    );
                });
            });

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Droid,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            view.submit_cli_agent_rich_input("hello!".to_owned(), ctx);
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            let input = view.input.as_ref(ctx);
            let ai_input_model = input.ai_input_model().as_ref(ctx);

            assert_eq!(
                ai_input_model.input_config(),
                InputConfig {
                    input_type: InputType::Shell,
                    is_locked: false,
                }
            );
            assert!(input.editor().as_ref(ctx).buffer_text(ctx).is_empty());
        });
    })
}

#[test]
fn unregister_cli_agent_session_restores_unlocked_input_config() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_agent_rich_input = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.ai_input_model().update(ctx, |ai_input, ctx| {
                    ai_input.set_input_config(
                        InputConfig {
                            input_type: InputType::Shell,
                            is_locked: false,
                        },
                        true,
                        None,
                        ctx,
                    );
                });
            });

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.remove_session(view.view_id, ctx);
            });
            assert!(!view.has_active_cli_agent_input_session(ctx));
            assert!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.view_id)
                    .is_none()
            );
        });

        terminal.read(&app, |view, ctx| {
            let input = view.input.as_ref(ctx);
            let ai_input_model = input.ai_input_model().as_ref(ctx);

            assert_eq!(
                ai_input_model.input_config(),
                InputConfig {
                    input_type: InputType::Shell,
                    is_locked: false,
                }
            );
            assert!(input.editor().as_ref(ctx).buffer_text(ctx).is_empty());
        });
    })
}

#[test]
fn clear_buffer_action_in_fullscreen_agent_view_starts_new_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let original_conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        terminal.update(&mut app, |view, ctx| {
            view.handle_action(&TerminalAction::ClearBuffer, ctx);
        });

        terminal.update(&mut app, |view, ctx| {
            let new_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should still be active");
            assert_ne!(new_conversation_id, original_conversation_id);
        });
    })
}

fn agent_jump_user_query(query: &str) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_owned(),
        context: Default::default(),
        static_query_type: None,
        referenced_attachments: Default::default(),
        user_query_mode: UserQueryMode::Normal,
        running_command: None,
        intended_agent: None,
    }
}

#[test]
fn jump_to_latest_agent_message_no_ops_when_agent_view_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // Create a conversation while the agent view feature is enabled...
        let agent_view = FeatureFlag::AgentView.override_enabled(true);
        terminal.update(&mut app, |view, ctx| {
            append_exchange_and_handle_event(view, agent_jump_user_query("hi"), ctx);
        });
        drop(agent_view);

        // ...then turn the feature off: the action must be inert even though a
        // conversation with a visible exchange exists.
        let _agent_view_off = FeatureFlag::AgentView.override_enabled(false);
        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
            assert_eq!(view.pending_agent_scroll_target, None);
        });
    })
}

#[test]
fn jump_to_latest_agent_message_no_ops_without_conversations() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        // No conversations exist, so there is nothing to jump to.
        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
            assert_eq!(view.pending_agent_scroll_target, None);
        });
    })
}

#[test]
fn jump_to_latest_agent_message_enters_agent_view_and_records_pending_scroll() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let (conversation_id, _task_id, exchange_id, _stream_id) = terminal
            .update(&mut app, |view, ctx| {
                append_exchange_and_handle_event(view, agent_jump_user_query("hi"), ctx)
            });

        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let controller = view.agent_view_controller().as_ref(ctx);
            match controller.agent_view_state() {
                AgentViewState::Active { origin, .. } => {
                    assert_eq!(
                        controller.agent_view_state().active_conversation_id(),
                        Some(conversation_id)
                    );
                    // 我方没有上游的 `JumpToLatestAgentMessage` 变体,
                    // `jump_to_latest_agent_message` 以 `Keybinding` 为入口 origin。
                    assert_eq!(*origin, AgentViewEntryOrigin::Keybinding);
                }
                state => panic!("expected an active agent view, got {state:?}"),
            }
            // Entering from the terminal mounts the target block on a later frame,
            // so the scroll target is recorded for `after_terminal_view_layout`.
            assert_eq!(view.pending_agent_scroll_target, Some(exchange_id));
        });
    })
}

#[test]
fn jump_to_latest_agent_message_targets_latest_visible_exchange() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let (conversation_id, _task_id, _first_exchange_id, _stream_id) = terminal
            .update(&mut app, |view, ctx| {
                append_exchange_and_handle_event(view, agent_jump_user_query("first"), ctx)
            });

        // Append a second, newer exchange to the same conversation.
        let second_exchange_id = terminal.update(&mut app, |view, ctx| {
            let history_model = BlocklistAIHistoryModel::handle(ctx);
            history_model.update(ctx, |history_model, ctx| {
                let response_stream_id = ResponseStreamId::new_for_test();
                let exchange = exchange_with_inputs(vec![agent_jump_user_query("second")]);
                let exchange_id = exchange.id;
                history_model
                    .conversation_mut(&conversation_id)
                    .expect("conversation should exist")
                    .append_reassigned_exchange(&response_stream_id, exchange, view.view_id, ctx)
                    .expect("exchange should append");
                exchange_id
            })
        });

        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, _ctx| {
            // The jump targets the latest visible exchange, not the first one.
            assert_eq!(view.pending_agent_scroll_target, Some(second_exchange_id));
        });
    })
}

#[test]
fn jump_to_latest_agent_message_scrolls_without_re_entering_when_already_in_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let (conversation_id, _task_id, _exchange_id, _stream_id) = terminal
            .update(&mut app, |view, ctx| {
                append_exchange_and_handle_event(view, agent_jump_user_query("hi"), ctx)
            });

        // First jump enters the agent view from the terminal and records a pending
        // scroll target; simulate the layout pass consuming it.
        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
            view.pending_agent_scroll_target = None;
        });

        // Second jump: already in this conversation's agent view, so it scrolls
        // directly without re-entering or recording a new pending target.
        terminal.update(&mut app, |view, ctx| {
            view.jump_to_latest_agent_message(ctx);
        });

        terminal.read(&app, |view, ctx| {
            let active_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should be active");
            assert_eq!(active_conversation_id, conversation_id);
            assert_eq!(view.pending_agent_scroll_target, None);
        });
    })
}

#[test]
fn restoring_conversation_to_new_pane_transfers_blocks_from_previous_terminal_surface() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let original_view = add_window_with_terminal(&mut app, None);
        let restored_view = add_window_with_terminal(&mut app, None);

        let original_view_id = original_view.read(&app, |view, _| view.view_id);
        let restored_view_id = restored_view.read(&app, |view, _| view.view_id);

        let conversation_id = original_view.update(&mut app, |view, ctx| {
            let (conversation_id, _, _, _) = append_exchange_with_inputs_and_handle_event(
                view,
                vec![AIAgentInput::UserQuery {
                    query: "first query".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }],
                ctx,
            );
            view.insert_agent_view_entry_block(
                AgentViewEntryBlockParams {
                    conversation_id,
                    is_new: false,
                    is_restored: false,
                    origin: AgentViewEntryOrigin::AgentViewBlock,
                    agent_view_controller: view.agent_view_controller().clone(),
                },
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: false,
                },
                ctx,
            );
            {
                let mut model = view.model.lock();
                model.simulate_block("agent command", "agent output");
                let command_block_index = model.block_list().blocks().len() - 2;
                model.block_list_mut().blocks_mut()[command_block_index]
                    .set_conversation_id(conversation_id);
            }
            conversation_id
        });

        original_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 1);
            assert_eq!(
                agent_view_entry_count_for_conversation(view, conversation_id),
                1
            );
            assert_eq!(
                command_block_count_for_conversation(view, conversation_id),
                1
            );
        });

        let restored_conversation =
            BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                history
                    .conversation(&conversation_id)
                    .cloned()
                    .expect("conversation should exist")
            });

        restored_view.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(restored_conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            let original_view_live_conversation_ids = history
                .all_live_conversations_for_terminal_surface(original_view_id)
                .map(|conversation| conversation.id())
                .collect::<Vec<_>>();
            let restored_view_live_conversation_ids = history
                .all_live_conversations_for_terminal_surface(restored_view_id)
                .map(|conversation| conversation.id())
                .collect::<Vec<_>>();
            assert_eq!(
                history.terminal_surface_id_for_conversation(&conversation_id),
                Some(restored_view_id)
            );
            assert!(original_view_live_conversation_ids.is_empty());
            assert_eq!(restored_view_live_conversation_ids, vec![conversation_id]);
        });

        original_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 0);
            assert_eq!(
                agent_view_entry_count_for_conversation(view, conversation_id),
                1
            );
            assert_eq!(
                command_block_count_for_conversation(view, conversation_id),
                0
            );
        });
        restored_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 1);
        });
    })
}

#[test]
fn clicking_old_banner_for_open_conversation_focuses_current_terminal_surface_without_transferring_blocks()
 {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let original_view = add_window_with_terminal(&mut app, None);
        let restored_view = add_window_with_terminal(&mut app, None);

        let original_window_id = app.read(|ctx| original_view.window_id(ctx));
        let original_view_id = original_view.read(&app, |view, _| view.view_id);
        let restored_view_id = restored_view.read(&app, |view, _| view.view_id);

        let conversation_id = original_view.update(&mut app, |view, ctx| {
            let (conversation_id, _, _, _) = append_exchange_with_inputs_and_handle_event(
                view,
                vec![AIAgentInput::UserQuery {
                    query: "first query".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }],
                ctx,
            );
            view.insert_agent_view_entry_block(
                AgentViewEntryBlockParams {
                    conversation_id,
                    is_new: false,
                    is_restored: false,
                    origin: AgentViewEntryOrigin::AgentViewBlock,
                    agent_view_controller: view.agent_view_controller().clone(),
                },
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: false,
                },
                ctx,
            );
            conversation_id
        });

        let restored_conversation =
            BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                history
                    .conversation(&conversation_id)
                    .cloned()
                    .expect("conversation should exist")
            });

        restored_view.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(restored_conversation),
                true,
                RestoreConversationEntryBehavior::PreserveAgentViewState,
                ctx,
            );
            assert_eq!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id(),
                None
            );
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        Some(conversation_id),
                        AgentViewEntryOrigin::AgentViewBlock,
                        ctx,
                    )
                    .expect("restored view should enter agent view");
            });
        });
        let restored_agent_view_controller =
            restored_view.read(&app, |view, _| view.agent_view_controller().clone());
        let restored_active_session =
            restored_view.read(&app, |view, _| view.active_session().clone());
        ActiveAgentViewsModel::handle(&app).update(&mut app, |active_views, ctx| {
            active_views.register_agent_view_controller(
                &restored_agent_view_controller,
                &restored_active_session,
                restored_view_id,
                ctx,
            );
        });

        ActiveAgentViewsModel::handle(&app).read(&app, |active_views, ctx| {
            assert_eq!(
                active_views.terminal_view_id_for_conversation(conversation_id, ctx),
                Some(restored_view_id)
            );
        });
        original_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 0);
            assert_eq!(
                agent_view_entry_count_for_conversation(view, conversation_id),
                1
            );
        });
        restored_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 1);
        });

        let entry_blocks = app
            .views_of_type::<AgentViewEntryBlock>(original_window_id)
            .expect("original window should contain agent entry block");
        assert_eq!(entry_blocks.len(), 1);
        entry_blocks[0].update(&mut app, |block, ctx| {
            block.handle_action(
                &EnterAgentBlockAction::EnterAgentMode { conversation_id },
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.terminal_surface_id_for_conversation(&conversation_id),
                Some(restored_view_id)
            );
            assert!(
                history
                    .all_live_conversations_for_terminal_surface(original_view_id)
                    .next()
                    .is_none()
            );
        });
        original_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 0);
            assert_eq!(
                agent_view_entry_count_for_conversation(view, conversation_id),
                1
            );
        });
        restored_view.read(&app, |view, _| {
            assert_eq!(ai_block_count(view), 1);
        });
    })
}

#[test]
fn appended_exchange_renders_in_current_terminal_surface_after_conversation_transfer() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let original_view = add_window_with_terminal(&mut app, None);
        let transferred_view = add_window_with_terminal(&mut app, None);

        let original_view_id = original_view.read(&app, |view, _| view.view_id);
        let transferred_view_id = transferred_view.read(&app, |view, _| view.view_id);

        let conversation_id = original_view.update(&mut app, |view, ctx| {
            let (conversation_id, _, _, _) = append_exchange_with_inputs_and_handle_event(
                view,
                vec![AIAgentInput::UserQuery {
                    query: "first query".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }],
                ctx,
            );
            conversation_id
        });

        let restored_conversation =
            BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                history
                    .conversation(&conversation_id)
                    .cloned()
                    .expect("conversation should exist")
            });

        transferred_view.update(&mut app, |view, ctx| {
            view.restore_conversation_after_view_creation(
                RestoredAIConversation::new(restored_conversation),
                true,
                RestoreConversationEntryBehavior::EnterRestoredConversation,
                ctx,
            );
        });

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.terminal_surface_id_for_conversation(&conversation_id),
                Some(transferred_view_id)
            );
        });

        let original_view_block_count_after_restore =
            original_view.read(&app, |view, _| ai_block_count(view));
        let transferred_view_block_count_after_restore =
            transferred_view.read(&app, |view, _| ai_block_count(view));
        assert_eq!(transferred_view_block_count_after_restore, 1);

        let (task_id, exchange_id, response_stream_id) = BlocklistAIHistoryModel::handle(&app)
            .update(&mut app, |history, ctx| {
                let conversation = history
                    .conversation_mut(&conversation_id)
                    .expect("conversation should exist");
                let task_id = conversation.get_root_task_id().clone();
                let response_stream_id = ResponseStreamId::new_for_test();
                let exchange = exchange_with_inputs(vec![AIAgentInput::UserQuery {
                    query: "follow up".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }]);
                let exchange_id = exchange.id;
                conversation
                    .append_reassigned_exchange(
                        &response_stream_id,
                        exchange,
                        original_view_id,
                        ctx,
                    )
                    .expect("exchange should append");
                (task_id, exchange_id, response_stream_id)
            });

        original_view.update(&mut app, |view, ctx| {
            view.handle_ai_history_model_event(
                BlocklistAIHistoryModel::handle(ctx),
                &BlocklistAIHistoryEvent::AppendedExchange {
                    exchange_id,
                    task_id: task_id.clone(),
                    terminal_surface_id: original_view_id,
                    conversation_id,
                    is_hidden: false,
                    response_stream_id: Some(response_stream_id.clone()),
                },
                ctx,
            );
        });

        transferred_view.update(&mut app, |view, ctx| {
            view.handle_ai_history_model_event(
                BlocklistAIHistoryModel::handle(ctx),
                &BlocklistAIHistoryEvent::AppendedExchange {
                    exchange_id,
                    task_id,
                    terminal_surface_id: original_view_id,
                    conversation_id,
                    is_hidden: false,
                    response_stream_id: Some(response_stream_id),
                },
                ctx,
            );
        });

        original_view.read(&app, |view, _| {
            assert_eq!(
                ai_block_count(view),
                original_view_block_count_after_restore
            );
        });
        transferred_view.read(&app, |view, _| {
            assert_eq!(
                ai_block_count(view),
                transferred_view_block_count_after_restore + 1
            );
        });
    })
}

#[test]
fn command_first_word_and_suffix_preserves_leading_whitespace() {
    assert_eq!(
        command_first_word_and_suffix("  myssh arg"),
        Some(("myssh", " arg"))
    );
}

#[test]
fn command_first_word_and_suffix_handles_alias_without_args() {
    assert_eq!(
        command_first_word_and_suffix("  myssh"),
        Some(("myssh", ""))
    );
}

// 此处原有 5 个上游云端能力测试(cloud mode pane / cloud agent 派发),随该能力剥离一并删除。

#[test]
fn escape_does_not_exit_root_cloud_agent_view_with_long_running_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cloud_mode = FeatureFlag::CloudMode.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            // 我方把上游的 `CloudAgent` 变体叫作 `AmbientAgent`。
            view.enter_agent_view_for_new_conversation(
                None,
                AgentViewEntryOrigin::AmbientAgent,
                ctx,
            );
            view.model
                .lock()
                .simulate_long_running_block("claude", "running");

            view.handle_input_event(&InputEvent::Escape, ctx);

            // Root cloud-mode pane has no parent terminal to return to,
            // so Escape is a no-op and agent view stays active.
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

#[test]
fn escape_does_not_exit_local_agent_view_with_long_running_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(
                None,
                AgentViewEntryOrigin::Input {
                    was_prompt_autodetected: false,
                },
                ctx,
            );
            view.model
                .lock()
                .simulate_long_running_block("sleep 10", "running");

            assert!(matches!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .can_exit_agent_view(ctx),
                Err(ExitAgentViewError::LongRunningCommand)
            ));

            view.handle_input_event(&InputEvent::Escape, ctx);

            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

#[test]
fn set_input_mode_agent_does_not_enter_local_agent_from_root_cloud_mode_pane() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);
        FeatureFlag::CloudMode.set_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            view.ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .update(ctx, |model, ctx| {
                    model.enter_setup(ctx);
                });
            view.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::FinishedViewer);
        });

        terminal.update(&mut app, |view, ctx| {
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
            view.handle_action(&TerminalAction::SetInputModeAgent, ctx);
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
        });
    });
}

// Zap:上游的 `fresh_cloud_mode_setup_enters_agent_view_when_view_pending` 覆盖的是
// `TerminalView::enter_ambient_agent_setup` —— 一个 CloudMode 专属包装(进入 agent view +
// 把初始 prompt 塞进输入框)。该包装未在 Zap 保留(见 `terminal/view.rs`),
// 其内部两步都由已有测试分别覆盖,故删除本测试。

#[test]
fn shared_third_party_viewer_sync_enters_agent_view_and_retags_existing_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _agent_harness = FeatureFlag::AgentHarness.override_enabled(true);

        let terminal = add_window_with_cloud_mode_terminal(&mut app);

        terminal.update(&mut app, |view, ctx| {
            let harness_block_id = {
                let mut model = view.model.lock();
                model.set_shared_session_source(SharedSessionSource::ambient_agent(None));
                model.set_shared_session_status(SharedSessionStatus::ActiveViewer {
                    role: Default::default(),
                });
                model.simulate_block("claude", "running");
                model
                    .block_list()
                    .blocks()
                    .iter()
                    .find(|block| block.command_to_string() == "claude")
                    .expect("harness block should exist")
                    .id()
                    .clone()
            };

            view.ambient_agent_view_model()
                .expect("cloud mode terminal should have ambient model")
                .update(ctx, |model, ctx| {
                    model.set_harness(Harness::Claude, ctx);
                });

            // 我方没有上游那个返回 `Option<AIConversationId>` 的
            // `sync_agent_view_for_shared_third_party_viewer`;等价逻辑在
            // `maybe_enter_agent_view_for_shared_third_party_viewer`,由
            // `HarnessSelected` 事件驱动。重复投递同一事件即可验证幂等
            // (函数开头有 `agent_view_state().is_active()` 短路)。
            view.handle_ambient_agent_event(&AmbientAgentViewModelEvent::HarnessSelected, ctx);
            let conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("shared third-party viewer should sync");
            view.handle_ambient_agent_event(&AmbientAgentViewModelEvent::HarnessSelected, ctx);
            let idempotent_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("sync should be idempotent");
            assert_eq!(conversation_id, idempotent_conversation_id);

            let controller = view.agent_view_controller().as_ref(ctx);
            match controller.agent_view_state() {
                AgentViewState::Active { origin, .. } => {
                    assert_eq!(
                        controller.agent_view_state().active_conversation_id(),
                        Some(conversation_id)
                    );
                    // 上游 `ThirdPartyCloudAgent` 在我方叫 `ExternalAmbientAgent`。
                    assert_eq!(*origin, AgentViewEntryOrigin::ExternalAmbientAgent);
                }
                state => panic!("expected active agent view, got {state:?}"),
            }

            let model = view.model.lock();
            let block = model
                .block_list()
                .block_with_id(&harness_block_id)
                .expect("harness block should still exist");
            assert!(!block.should_hide_block(model.block_list().transcript_scope()));
            match block.agent_view_visibility() {
                AgentViewVisibility::Terminal {
                    conversation_ids,
                    pending_conversation_ids,
                } => {
                    assert!(pending_conversation_ids.is_empty());
                    assert!(conversation_ids.contains(&conversation_id));
                }
                visibility => panic!("expected terminal block visibility, got {visibility:?}"),
            }
        });
    });
}

// Zap:上游的 `shared_third_party_viewer_syncs_from_viewer_harness_updated_when_harness_unchanged`
// 依赖 `AmbientAgentViewModelEvent::ViewerHarnessResolved`(harness 未变化时的 viewer 侧兜底事件),
// 我方 `AmbientAgentViewModelEvent` 没有该变体 —— viewer 侧同步统一由 `HarnessSelected` 驱动,
// 覆盖已并入上面的 `shared_third_party_viewer_sync_enters_agent_view_and_retags_existing_block`。

// Zap:上游的 `cloud_mode_followup_input_uses_explicit_submit_event_even_when_view_pending`
// 断言 follow-up 被暂存到 `AmbientAgentViewModel::pending_followup_prompt()`。整条云端
// follow-up 提交链路(`submit_cloud_followup` / `pending_followup_prompt`)在 Zap 已剥离
// (见 `terminal/view.rs::try_submit_pending_cloud_followup` 的中文注释),故删除本测试;
// 失败回退路径由下面的 `pending_cloud_followup_without_ambient_model_restores_prompt` 覆盖。

#[test]
fn pending_cloud_followup_without_ambient_model_restores_prompt() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| ToastStack);
        let _flag = FeatureFlag::HandoffCloudCloud.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        let task_id = AmbientAgentTaskId::from_str("123e4567-e89b-12d3-a456-426614174000")
            .expect("valid task id");

        terminal.update(&mut app, |view, ctx| {
            view.pending_cloud_followup_task_id = Some(task_id);

            assert!(view.try_submit_pending_cloud_followup("follow up".to_string(), ctx));
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(view.pending_cloud_followup_task_id, None);
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "follow up");
        });
    });
}

#[test]
fn cmd_enter_from_terminal_without_selected_block_enters_agent_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            assert!(
                view.ai_context_model
                    .as_ref(ctx)
                    .pending_context_block_ids()
                    .is_empty()
            );
            view.focus_terminal(ctx);
        });

        let keystroke = if cfg!(target_os = "macos") {
            "cmd-enter"
        } else {
            "ctrl-shift-enter"
        };
        let handled = app
            .dispatch_keystroke(
                window_id,
                &[terminal.id()],
                &warpui::keymap::Keystroke::parse(keystroke).expect("valid keystroke"),
                false,
            )
            .expect("dispatch should succeed");
        assert!(
            handled,
            "{keystroke} should be handled from terminal context"
        );

        terminal.read(&app, |view, ctx| {
            assert!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .is_fullscreen()
            );
            assert!(
                view.ai_context_model
                    .as_ref(ctx)
                    .pending_context_block_ids()
                    .is_empty()
            );
        });
    });
}

#[test]
fn cmd_enter_from_terminal_with_selected_block_enters_agent_view_with_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);

        let selected_block_id = terminal.update(&mut app, |view, ctx| {
            let (selected_block_index, selected_block_id) = {
                let mut model = view.model.lock();
                model.simulate_block("echo selected", "selected");
                let block = model
                    .block_list()
                    .blocks()
                    .iter()
                    .find(|block| block.command_to_string() == "echo selected")
                    .expect("simulated block should exist");
                (block.index(), block.id().clone())
            };

            view.integration_test_change_block_selection_to_single(selected_block_index, ctx);
            assert!(
                view.ai_context_model
                    .as_ref(ctx)
                    .pending_context_block_ids()
                    .contains(&selected_block_id)
            );
            view.focus_terminal(ctx);
            selected_block_id
        });

        let keystroke = if cfg!(target_os = "macos") {
            "cmd-enter"
        } else {
            "ctrl-shift-enter"
        };
        let handled = app
            .dispatch_keystroke(
                window_id,
                &[terminal.id()],
                &warpui::keymap::Keystroke::parse(keystroke).expect("valid keystroke"),
                false,
            )
            .expect("dispatch should succeed");
        assert!(
            handled,
            "{keystroke} should be handled from terminal context"
        );

        terminal.read(&app, |view, ctx| {
            let conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state().active_conversation_id()
                .expect("agent view should be active");

            let model = view.model.lock();
            let block = model
                .block_list()
                .block_with_id(&selected_block_id)
                .expect("selected block should still exist");
            assert!(
                !block.should_hide_block(model.block_list().transcript_scope()),
                "selected block should remain visible in the new agent conversation"
            );
            match block.agent_view_visibility() {
                AgentViewVisibility::Terminal {
                    pending_conversation_ids,
                    ..
                } => {
                    assert!(
                        pending_conversation_ids.contains(&conversation_id),
                        "selected block should be attached as pending context for the new conversation"
                    );
                }
                visibility => panic!("expected terminal block visibility, got {visibility:?}"),
            }
        });
    });
}

#[test]
fn cmd_enter_from_active_non_empty_agent_view_requires_confirmation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);

        let original_conversation_id = terminal.update(&mut app, |view, ctx| {
            let (conversation_id, _, _, _) =
                append_exchange_and_handle_event(view, agent_jump_user_query("first"), ctx);
            view.enter_agent_view_for_conversation(
                None,
                AgentViewEntryOrigin::ConversationSelector,
                conversation_id,
                ctx,
            );
            view.focus_terminal(ctx);
            conversation_id
        });

        let keystroke = if cfg!(target_os = "macos") {
            "cmd-enter"
        } else {
            "ctrl-shift-enter"
        };
        let keystroke = warpui::keymap::Keystroke::parse(keystroke).expect("valid keystroke");

        let handled = app
            .dispatch_keystroke(window_id, &[terminal.id()], &keystroke, false)
            .expect("dispatch should succeed");
        assert!(
            handled,
            "new conversation keybinding should be handled from terminal context"
        );

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id(),
                Some(original_conversation_id),
                "first keybinding press should keep the current conversation active"
            );
        });

        let handled = app
            .dispatch_keystroke(window_id, &[terminal.id()], &keystroke, false)
            .expect("dispatch should succeed");
        assert!(
            handled,
            "new conversation keybinding should be handled from terminal context"
        );

        terminal.read(&app, |view, ctx| {
            let new_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should be active");
            assert_ne!(
                new_conversation_id, original_conversation_id,
                "second keybinding press should start a new conversation"
            );
        });
    });
}

// Zap:以下两个上游测试随云端 follow-up / Cloud Mode Setup V2 一起删除。
// - `cloud_mode_followup_dispatched_inserts_queued_user_query` 依赖
//   `AmbientAgentViewModel::submit_cloud_followup` 与 `AmbientAgentViewModelEvent::FollowupDispatched`,
//   两者都不存在(云端 follow-up 链路已剥离)。
// - `cloud_mode_setup_v2_suppresses_sharer_input_updates_while_followup_setup_commands_run` 依赖
//   `AmbientAgentViewModel::setup_command_state{,_mut}()`(纯上游 Cloud Mode Setup V2 面板状态);
//   我方 `TerminalView::should_suppress_ambient_setup_input_sync` 已恒返回 false,断言不再成立。

#[test]
fn pending_cloud_mode_query_waits_for_renderable_user_query_exchange() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.insert_cloud_mode_queued_user_query_block("queued prompt".to_string(), ctx);
            assert!(has_pending_user_query_block(view));

            append_exchange_and_handle_event(
                view,
                AIAgentInput::ResumeConversation {
                    context: Default::default(),
                },
                ctx,
            );
            assert!(has_pending_user_query_block(view));

            append_exchange_and_handle_event(
                view,
                AIAgentInput::UserQuery {
                    query: "real prompt".to_string(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::default(),
                    running_command: None,
                    intended_agent: None,
                },
                ctx,
            );
            assert!(!has_pending_user_query_block(view));
        });
    });
}

#[test]
fn pending_cloud_mode_query_clears_when_streaming_exchange_becomes_renderable() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.insert_cloud_mode_queued_user_query_block(
                "write a poem about rocks".to_string(),
                ctx,
            );
            assert!(has_pending_user_query_block(view));

            let (conversation_id, _, exchange_id, response_stream_id) =
                append_exchange_with_inputs_and_handle_event(view, vec![], ctx);
            assert!(has_pending_user_query_block(view));

            update_exchange_input_and_handle_event(
                view,
                conversation_id,
                exchange_id,
                response_stream_id,
                vec![AIAgentInput::UserQuery {
                    query: "write an ode about stones".to_string(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                }],
                ctx,
            );
            assert!(!has_pending_user_query_block(view));

            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("conversation should exist");
            let initial_user_query = conversation.initial_user_query();
            let exchange = conversation
                .exchange_with_id(exchange_id)
                .expect("exchange should exist");
            assert_eq!(
                exchange.input[0]
                    .display_user_query(initial_user_query.as_ref())
                    .as_deref(),
                Some("/agent write an ode about stones")
            );
        });
    });
}

/// Test clearing of session flag state when terminal is cleared
#[test]
fn test_clear_session_flag_state() {
    use warp_terminal::shell::ShellType;

    use crate::ai::blocklist::SerializedBlockListItem;
    use crate::terminal::ShellHost;
    use crate::terminal::model::block::SerializedBlock;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        // Create a remote restored block
        let mut remote_block =
            SerializedBlock::new_for_test("echo remote".into(), "remote output".into());
        remote_block.is_local = Some(false); // Mark it as a remote block
        remote_block.shell_host = Some(ShellHost {
            shell_type: ShellType::Bash,
            user: "user".to_string(),
            hostname: "remote".to_string(), // Remote hostname indicates a remote session
        });

        // Convert to SerializedBlockListItem
        let restored_blocks = [SerializedBlockListItem::Command {
            block: Box::new(remote_block),
        }];

        // Create terminal with the restored remote block
        let terminal = add_window_with_terminal(&mut app, Some(&restored_blocks));

        terminal.update(&mut app, |view, ctx| {
            // Verify initial state - block was created as remote and restored
            assert!(
                !view.any_session_contains_remote_blocks,
                "Terminal should not have remote blocks"
            );
            assert!(
                view.any_session_contains_restored_remote_blocks,
                "Terminal should have restored remote blocks"
            );

            {
                // Verify the block was properly created with correct properties
                let model = view.model.lock();
                let blocks = model.block_list().blocks();

                // The first block should be our restored remote block
                assert!(!blocks.is_empty(), "At least one block should exist");
                if let Some(first_block) = blocks.first() {
                    assert_eq!(
                        first_block.restored_block_was_local(),
                        Some(false),
                        "First block should be marked as a remote restored block"
                    );
                }
            }

            // Now clear the terminal
            view.clear_buffer_for_testing(ctx);

            // Flags should be reset
            assert!(
                !view.any_session_contains_remote_blocks,
                "Terminal should not have remote blocks after clearing"
            );
            assert!(
                !view.any_session_contains_restored_remote_blocks,
                "Terminal should not have restored remote blocks after clearing"
            );
        });
    })
}

fn assert_block_has_find_match(find_model: &TerminalFindModel, block_index: BlockIndex) {
    assert!(
        find_model
            .block_list_find_run()
            .is_some_and(|run| run.matches_for_block(block_index).next().is_some())
    );
}

impl TerminalView {
    fn is_top_of_active_block_in_viewport(
        &self,
        model: &TerminalModel,
        input_mode: InputMode,
        app: &AppContext,
    ) -> bool {
        let active_block_index = model.block_list().active_block_index();
        let viewport = self.viewport_state(model.block_list(), input_mode, app);
        viewport.is_block_in_view(active_block_index, BlockVisibilityMode::TopOfBlockVisible)
    }

    fn scroll_top_in_lines(
        &self,
        model: &TerminalModel,
        input_mode: InputMode,
        app: &AppContext,
    ) -> Lines {
        let viewport = self.viewport_state(model.block_list(), input_mode, app);
        viewport.scroll_top_in_lines()
    }

    fn is_vertically_scrollable(&self, app: &AppContext) -> bool {
        let total_block_heights = self
            .model
            .lock()
            .block_list()
            .block_heights()
            .summary()
            .height;
        let visible_rows = self.content_element_height_lines(app);
        heights_approx_gt(total_block_heights, visible_rows)
    }
}

fn read_from_clipboard(ctx: &mut ViewContext<TerminalView>) -> String {
    TerminalView::read_from_clipboard(Some(ShellFamily::Posix), ctx)
}

#[test]
fn test_insert() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let select_text = |view: &mut TerminalView, ctx: &mut ViewContext<TerminalView>| {
            {
                let mut model = view.model.lock();
                model.start_command_execution();
                let blocks = model.block_list_mut();
                blocks.input('f');
                blocks.linefeed();
                blocks.preexec(PreexecValue::default());
                blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
            }
            view.begin_block_text_selection(
                BlockListPoint::new(1.0, 1),
                Side::Right,
                SelectionType::Semantic,
                Vector2F::zero(),
                ctx,
            );
            view.end_text_selection(ctx);
        };
        let assert_input_text_eq = |app: &mut App, expected_text: &str| {
            terminal.read(app, |view, _ctx| {
                view.input.read(app, |view, ctx| {
                    assert_eq!(view.buffer_text(ctx), String::from(expected_text));
                });
            });
        };
        let assert_selected_blocks_cardinality_eq =
            |app: &mut App, expected_cardinality: BlockSelectionCardinality| {
                terminal.read(app, |view, _ctx| {
                    assert_eq!(
                        view.selected_blocks.cardinality().as_keymap_context_value(),
                        expected_cardinality.as_keymap_context_value()
                    );
                });
            };
        let assert_selected_text_eq = |app: &mut App, expected_text: Option<String>| {
            terminal.update(app, |view, ctx| {
                let semantic_selection = SemanticSelection::as_ref(ctx);
                let model = view.model.lock();
                let context_selected_text =
                    model.selection_to_string(semantic_selection, false, ctx);
                assert_eq!(context_selected_text, expected_text);
            });
        };

        // Shell Mode: Nothing selected
        terminal.update(&mut app, |view, ctx| {
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("hello", ctx);
        });
        assert_input_text_eq(&mut app, "hello");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, None);

        // Shell Mode: Block selected
        terminal.update(&mut app, |view, ctx| {
            view.selected_blocks.reset_to_single(BlockIndex::zero());
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_this", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, None);

        // Shell Mode: Text selected
        terminal.update(&mut app, |view, ctx| {
            select_text(view, ctx);
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_is", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this_is");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, None);

        // Activate Agent Mode, which should no longer allow text insertion to clear the selected block(s) or text
        terminal.update(&mut app, |view, ctx| {
            view.set_ai_input_mode_with_query(None, ctx);
        });

        // Agent Mode: Nothing selected
        terminal.update(&mut app, |view, ctx| {
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_your", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this_is_your");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, None);

        // Agent Mode: Block selected
        terminal.update(&mut app, |view, ctx| {
            view.selected_blocks.reset_to_single(BlockIndex::zero());
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_captain", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this_is_your_captain");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::One);
        assert_selected_text_eq(&mut app, None);

        // Agent Mode: Text selected
        terminal.update(&mut app, |view, ctx| {
            select_text(view, ctx);
            view.focus_terminal(ctx);
            view.typed_characters_on_terminal("_speaking", ctx);
        });
        assert_input_text_eq(&mut app, "hello_this_is_your_captain_speaking");
        assert_selected_blocks_cardinality_eq(&mut app, BlockSelectionCardinality::None);
        assert_selected_text_eq(&mut app, Some("f".to_owned()));
    })
}

const BODY_PREFIX: &str = "Latest output: ";

/// Regression test for CORE-1654. Tests the "Insert into Input" functionality from the context menu.
#[test]
fn test_insert_into_input() {
    // Note that this is defined as a unit test rather than an integration test since it requires precise selections
    // (where we don't want UI updates making the test brittle, due to hardcoded mouse positions).
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // TODO: Potentially explore if we can re-use helpers from `input_test.rs` (`select_first_command_line_of_block` and `insert_dummy_block`).
        terminal.update(&mut app, |terminal_view, ctx| {
            {
                let mut terminal_model = terminal_view.model.lock();
                let blocks = terminal_model.block_list_mut();
                // Add two lines to the command grid and output grid in a new block.
                let block_index = insert_block(blocks, "cmd_a\ncmd_b\n", "output_a\noutput_b\n");
                let block = blocks.block_at(block_index).expect("block should exist");
                // Selections are inclusive of endpoint, hence we need to identify the last column to select the first command.
                let block_command_columns =
                    block.prompt_and_command_grid().grid_handler().columns();
                let command_grid_offset = block.command_grid_offset();
                // Create a selection that just spans the first line of the command grid in the block.
                blocks.start_selection(
                    BlockListPoint::new(command_grid_offset, 0),
                    SelectionType::Simple,
                    Side::Left,
                );
                blocks.update_selection(
                    BlockListPoint::new(command_grid_offset, block_command_columns),
                    Side::Right,
                );
                let selection = blocks.selection();
                assert!(selection.is_some());
            }

            terminal_view.context_menu_insert_selected_text(ctx);
        });

        // Confirm that the blocklist selection is cleared upon inserting into the input box.
        terminal.read(&app, |terminal_view, _ctx| {
            let terminal_model = terminal_view.model.lock();
            let blocks = terminal_model.block_list();
            let selection = blocks.selection();
            assert!(
                selection.is_none(),
                "Expected no selections in the blocklist but got {selection:?}"
            );
        });
        let input = terminal.read(&app, |terminal, _ctx| terminal.input().clone());
        // Confirm that the input box has the correct text (the first line of the command grid was selected above).
        input.read(&app, |input, ctx| {
            assert_eq!(input.buffer_text(ctx), "cmd_a");
        });
    });
}

#[test]
fn test_copy_on_select() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        // Add some text and make sure we update the selection
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.start_command_execution();
                let blocks = model.block_list_mut();

                blocks.input('f');
                blocks.input('o');
                blocks.input('o');

                blocks.linefeed();

                blocks.preexec(PreexecValue::default());

                blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
            }

            view.begin_block_text_selection(
                BlockListPoint::new(1.0, 1),
                Side::Right,
                SelectionType::Semantic,
                Vector2F::zero(),
                ctx,
            );

            let selection_settings = SelectionSettings::as_ref(ctx);
            assert!(selection_settings.copy_on_select_enabled());
            assert_eq!("", &read_from_clipboard(ctx));
            view.end_text_selection(ctx);
            assert_eq!("foo", &read_from_clipboard(ctx));
        });
    })
}

#[test]
fn test_alt_screen_copy_on_select() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                // Enter alt screen and add text
                let mut model = view.model.lock();
                model.set_mode(ansi::Mode::SwapScreen {
                    save_cursor_and_clear_screen: true,
                });
                assert!(model.is_alt_screen_active());

                model.alt_screen_mut().input('h');
            }
            // Ensure copy on select is enabled
            let selection_settings = SelectionSettings::as_ref(ctx);
            assert!(selection_settings.copy_on_select_enabled());

            // Select input
            view.begin_alt_selection(Point::new(0, 0), Side::Left, SelectionType::Simple, ctx);
            assert_eq!("", &read_from_clipboard(ctx));
            view.update_alt_selection(Point::new(0, 2), Side::Left, &Lines::zero(), ctx);
            view.end_alt_selection(ctx);
            // Ensure selection is copied
            assert_eq!("h", &read_from_clipboard(ctx));
        });
    })
}

#[test]
fn test_alt_screen_select_with_sgr_mouse() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let (window_id, terminal) = add_window_with_id_and_terminal(&mut app, None);

        let mut updated = EntityIdSet::default();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));

        let semantic_selection = SemanticSelection::mock(true, "");

        let size_info = terminal.update(&mut app, |view, ctx| {
            {
                // Enter alt screen and enable SGR Mouse
                let mut model = view.model.lock();
                model.set_mode(ansi::Mode::SwapScreen {
                    save_cursor_and_clear_screen: true,
                });
                model.set_mode(ansi::Mode::SgrMouse);
                assert!(model.is_alt_screen_active());
                assert!(!should_intercept_mouse(&model, false, ctx));
                assert!(should_intercept_mouse(&model, true, ctx));

                // Write a bunch of characters into the alt screen.
                // ABCDEFG
                // HIJKLMN
                // OPQRSTU
                // VWXYZ[\
                // ]^_`abc
                // defghij
                // klmnopq
                // rstuvwx
                // yz{|}~
                // € ‚ƒ„…†
                // ‡ˆ‰Š‹Œ
                let mut ascii: u8 = 65;
                for _ in 0..view.size_info.rows {
                    for _ in 0..view.size_info.columns {
                        model.alt_screen_mut().input(ascii as char);
                        ascii += 1;
                    }
                }

                *view.size_info
            }
        });

        // We need to manually trigger re-renders to ensure the AltScreenElement is recreated, e.g.
        // so its `is_terminal_selecting` property will be up-to-date.
        macro_rules! rerender {
            ($app:ident, $presenter:expr_2021, $invalidation:expr_2021, $size_info:expr_2021) => {
                app.update(enclose!((presenter, invalidation) move |ctx| {
                    presenter
                        .borrow_mut()
                        .invalidate(invalidation, ctx);
                    presenter.borrow_mut().build_scene(
                        vec2f(size_info.pane_width_px, size_info.pane_height_px),
                        1.,
                        None,
                        ctx,
                    );
                }));
            }
        }

        // The start and end positions corresponds to 'J'
        // and 'a' in the grid, respectively.
        //
        // We adjust the vertical coordinates to account for padding
        // in the alt-screen.
        let start_position = vec2f(
            2. * size_info.cell_width_px.as_f32(),
            2. * size_info.cell_height_px.as_f32() - 1.,
        );
        let end_position = vec2f(
            5. * size_info.cell_width_px.as_f32(),
            5. * size_info.cell_height_px.as_f32() - 1.,
        );

        // Simulate a mouse drag from the "J" to the "a" cell.
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseDown {
                    position: start_position,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
        }));
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseDragged {
                    position: end_position,
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
        }));
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseUp {
                    position: end_position,
                    modifiers: Default::default(),
                },
                window_id,
                presenter.clone(),
            );
        }));

        // No selection should've occurred as we aren't intercepting mouse events.
        terminal.read(&app, |view, ctx| {
            let selected_text =
                view.model
                    .lock()
                    .selection_to_string(&semantic_selection, false, ctx);
            assert_eq!(selected_text, None);
        });

        // This time, hold Shift key for all mouse events.
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseDown {
                    position: start_position,
                    modifiers: ModifiersState {
                        shift: true,
                        ..Default::default()
                    },
                    click_count: 1,
                    is_first_mouse: false,
                },
                window_id,
                presenter.clone(),
            );
        }));
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseDragged {
                    position: end_position,
                    modifiers: ModifiersState {
                        shift: true,
                        ..Default::default()
                    },
                },
                window_id,
                presenter.clone(),
            );
        }));
        rerender!(app, presenter, invalidation, size_info);
        app.update(enclose!((presenter) move |ctx| {
            ctx.simulate_window_event(
                warpui::Event::LeftMouseUp {
                    position: end_position,
                    modifiers: ModifiersState {
                        shift: true,
                        ..Default::default()
                    },
                },
                window_id,
                presenter.clone(),
            );
        }));

        // This time we expect a selection since the Shift key had been held for this mouse drag.
        terminal.read(&app, |view, ctx| {
            let selected_text =
                view.model
                    .lock()
                    .selection_to_string(&semantic_selection, false, ctx);
            assert_eq!(selected_text.as_ref().unwrap(), "JKLMNOPQRSTUVWXYZ[\\]^_`a");
        });
    })
}

// Regression test for WAR-3433 on find bar selection crash.
#[test]
fn test_find_bar_select() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        // Add some text and make sure we update the selection
        terminal.update(&mut app, |view, ctx| {
            // Mock a block with content 'foo g'.
            {
                let mut model = view.model.lock();
                model.start_command_execution();
                let blocks = model.block_list_mut();

                blocks.input('f');
                blocks.input('o');
                blocks.input('o');

                blocks.input(' ');
                blocks.input('g');

                blocks.linefeed();

                blocks.preexec(PreexecValue::default());

                blocks.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
            }

            // Select 'foo'.
            view.begin_block_text_selection(
                BlockListPoint::new(1.0, 1),
                Side::Right,
                SelectionType::Semantic,
                Vector2F::zero(),
                ctx,
            );

            let selection_settings = SelectionSettings::as_ref(ctx);
            assert!(selection_settings.copy_on_select_enabled());
            assert_eq!("", &read_from_clipboard(ctx));
            view.end_text_selection(ctx);
            assert_eq!("foo", &read_from_clipboard(ctx));

            // Show find bar. The find bar should have selected text 'foo' in its editor.
            view.show_find_bar(ctx);
            view.find_bar.read(ctx, |find, ctx| {
                find.editor().read(ctx, |editor, ctx| {
                    assert_eq!("foo".to_string(), editor.selected_text(ctx));
                })
            });

            // Now select 'foo g'.
            view.begin_block_text_selection(
                BlockListPoint::new(1.0, 1),
                Side::Right,
                SelectionType::Lines,
                Vector2F::zero(),
                ctx,
            );

            let selection_settings = SelectionSettings::as_ref(ctx);
            assert!(selection_settings.copy_on_select_enabled());
            assert_eq!("foo", &read_from_clipboard(ctx));
            view.end_text_selection(ctx);
            assert_eq!("foo g", &read_from_clipboard(ctx));

            // Show find bar. The find bar should have selected text 'foo g' in its editor.
            view.show_find_bar(ctx);
            view.find_bar.read(ctx, |find, ctx| {
                find.editor().read(ctx, |editor, ctx| {
                    assert_eq!("foo g".to_string(), editor.selected_text(ctx));
                })
            });
        });
    })
}

#[test]
fn test_viewport_iter_most_recent_at_bottom() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.simulate_block("ls", "foo");
            model.simulate_block("echo multiline", "bar\nhey");
            let viewport = view.viewport_state(model.block_list(), InputMode::PinnedToBottom, ctx);
            let mut iter = viewport.iter();
            let first_block = iter.next().expect("item 1");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(1)),
                first_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(1),
                first_block.entry_index
            );
            assert!(first_block.block_height_item.height().into_lines() > Lines::zero());
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(1)),
                viewport.topmost_visible_block()
            );

            let second_block = iter.next().expect("item 2");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(2)),
                second_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(2),
                second_block.entry_index
            );
            assert!(
                second_block.block_height_item.height() > first_block.block_height_item.height()
            );
            assert!(viewport.is_block_in_view(
                std::convert::Into::<BlockIndex>::into(2),
                BlockVisibilityMode::TopOfBlockVisible
            ));

            let third_block = iter.next().expect("item 3");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(3)),
                third_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(3),
                third_block.entry_index
            );
            assert_eq!(0., third_block.block_height_item.height().as_f64());
        });
    })
}

#[test]
fn test_viewport_iter_most_recent_at_top() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx: &mut ViewContext<'_, TerminalView>| {
            let mut model = view.model.lock();
            model.simulate_block("ls", "foo");
            model.simulate_block("echo multiline", "bar\nhey");
            let viewport = view.viewport_state(model.block_list(), InputMode::PinnedToTop, ctx);
            let mut iter = viewport.iter();
            let echo_block = iter.next().expect("item 2");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(2)),
                echo_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(2),
                echo_block.entry_index
            );
            assert!(echo_block.block_height_item.height().into_lines() > Lines::zero());
            assert_eq!(Pixels::zero(), viewport.offset_to_top_of_first_block(ctx));
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(2)),
                viewport.topmost_visible_block()
            );
            assert!(viewport.is_block_in_view(
                std::convert::Into::<BlockIndex>::into(2),
                BlockVisibilityMode::TopOfBlockVisible
            ));

            let ls_block = iter.next().expect("item 1");
            assert_eq!(
                Some(std::convert::Into::<BlockIndex>::into(1)),
                ls_block.block_index
            );
            assert_eq!(
                std::convert::Into::<TotalIndex>::into(1),
                ls_block.entry_index
            );
            assert!(
                echo_block.block_height_item.height().as_f64()
                    > ls_block.block_height_item.height().as_f64()
            );
        });
    })
}

#[test]
fn test_viewport_most_recent_at_top() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let mut model = view.model.lock();
            model.simulate_block("ls", "foo");
            model.simulate_block("echo multiline", "bar\nhey");
            let viewport = view.viewport_state(model.block_list(), InputMode::PinnedToTop, ctx);
            // Most recent block should be visible.
            let topmost_visible_block = viewport.topmost_visible_block().unwrap();
            assert!(viewport.is_block_in_view(
                topmost_visible_block,
                BlockVisibilityMode::TopOfBlockVisible
            ));
            assert_eq!(Pixels::zero(), viewport.offset_to_top_of_first_block(ctx));
            assert_eq!(0., viewport.scroll_top_in_lines().as_f64());
            assert!(matches!(
                viewport.next_scroll_position(
                    ScrollPositionUpdate::AfterScrollEvent {
                        scroll_delta: 1.0.into_lines()
                    },
                    ctx
                ),
                ScrollPosition::FixedAtPosition { .. }
            ));
            assert_eq!(
                Lines::zero(),
                viewport.top_of_block_in_lines(topmost_visible_block)
            );
            assert!(matches!(
                viewport.scroll_position_at_bottom_of_block(topmost_visible_block),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            ));
            let block_list_point = viewport
                .screen_coord_to_blocklist_point(
                    vec2f(0., 0.),
                    SnackbarPoint {
                        coord: vec2f(0., 0.),
                        translation_mode: SnackbarTranslationMode::WithinSnackbar,
                    },
                    ClampingMode::ClampToGrid,
                )
                .unwrap();
            assert_eq!(
                Some(2.into()),
                viewport.block_index_from_point(block_list_point)
            );
        });
    })
}

#[test]
fn test_scroll_fixed_to_bottom() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.read(&app, |view, _| {
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
        });
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..100 {
                    model.simulate_block("ls", "foo");
                }
            }
            assert!(view.is_vertically_scrollable(ctx));
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
            view.scroll(1.0.into_lines(), ctx);

            let expected_scroll_top = {
                let model = view.model.lock();
                model.block_list().block_heights().summary().height
                    - view.content_element_height_lines(ctx)
                    - 1.0.into_lines()
            };
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(expected_scroll_top)
                },
            );
            // Now add to the active block and make sure we don't scroll
            {
                let mut model = view.model.lock();
                model.simulate_cmd("test");
            }
            {
                let mut model = view.model.lock();
                for _ in 0..100 {
                    model.linefeed();
                }
            }
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(expected_scroll_top)
                },
            );
        });
    })
}

#[test]
fn test_scroll_to_row() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..50 {
                    model.simulate_block("ls", "foo\nfie\nfay\nfoe\nfum");
                }
            }

            assert!(view.is_vertically_scrollable(ctx));
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );

            // Scroll upwards (no snackbar)
            let a = BlockListPoint::new(30.0, 0);
            view.scroll_to_row_if_not_visible(a.row.into_lines(), ctx);
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(30.0.into_lines())
                }
            );

            // Don't scroll at all
            let b = BlockListPoint::new(38.0, 0);
            view.scroll_to_row_if_not_visible(b.row.into_lines(), ctx);
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(30.0.into_lines())
                }
            );

            // Scroll downwards
            let c = BlockListPoint::new(100.0, 0);
            view.scroll_to_row_if_not_visible(c.row.into_lines(), ctx);
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition {
                    scroll_lines: ScrollLines::ScrollTop(90.5.into_lines())
                }
            );
        });
    })
}

#[test]
fn test_stable_scrolling_during_grid_truncation() {
    App::test((), |mut app| async move {
        const MAX_GRID_SIZE: usize = 50;
        const INPUT_MODE: InputMode = InputMode::PinnedToBottom;

        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // Note: this test is done in a single `update` to prevent
        // any changes in the presenter's position cache throughout.
        terminal.update(&mut app, |view, ctx| {
            // Set up the block list by creating a long-running
            // block that spans the entire viewport.
            {
                let mut model = view.model.lock();
                model.update_max_grid_size(MAX_GRID_SIZE);

                // Create a dummy, finished block and a long-running block.
                model.simulate_block("ls", "foo");
                model.simulate_long_running_block("cat", "");
                assert!(
                    model
                        .block_list()
                        .active_block()
                        .is_active_and_long_running()
                );

                // Add enough newlines so that the long-running block spans at
                // least the viewport and surely exceeds the grid size.
                let mut i = 0;
                while view.is_top_of_active_block_in_viewport(&model, INPUT_MODE, ctx)
                    || i < MAX_GRID_SIZE * 2
                {
                    model.process_bytes("\n");
                    i += 1;
                }
            }

            // Scroll up one line.
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
            view.scroll(1.into_lines(), ctx);
            assert!(matches!(
                view.scroll_position(),
                ScrollPosition::FixedWithinLongRunningBlock { .. }
            ));

            // Introduce new lines and make sure the scroll-top is adjusted as expected.
            {
                let mut model = view.model.lock();
                let active_block_index = model.block_list().active_block_index();
                let scroll_top_before_scrolling = view.scroll_top_in_lines(&model, INPUT_MODE, ctx);

                // To get to the top of the block, we need 50 lines for output grid and
                // then one line for command grid.
                for i in 1..=(MAX_GRID_SIZE + 1) {
                    model.process_bytes("\n");

                    let actual_scroll_top = view.scroll_top_in_lines(&model, INPUT_MODE, ctx);
                    let expected_scroll_top = scroll_top_before_scrolling - i.into_lines();
                    assert_eq!(actual_scroll_top, expected_scroll_top);
                }

                // Flush one full line in case the top of the block doesn't perfectly
                // line up with full lines (e.g. due to padding).
                model.process_bytes("\n");

                // Any remaining newlines should not move the scroll-top;
                // it should be "locked" at the top of the block.
                for _ in 0..MAX_GRID_SIZE {
                    model.process_bytes("\n");

                    let viewport = view.viewport_state(model.block_list(), INPUT_MODE, ctx);
                    let actual_scroll_top = viewport.scroll_top_in_lines();
                    let expected_scroll_top = viewport.top_of_block_in_lines(active_block_index);
                    assert_eq!(actual_scroll_top, expected_scroll_top);
                }
            }

            // Scroll up one line, bringing the previous block into the viewport.
            view.scroll(1.into_lines(), ctx);
            assert!(matches!(
                view.scroll_position(),
                ScrollPosition::FixedAtPosition { .. }
            ));

            // Introduce newlines and make sure the scroll-top does _not_ change anymore.
            {
                let mut model = view.model.lock();
                let scroll_top_before_newlines = view.scroll_top_in_lines(&model, INPUT_MODE, ctx);

                for _ in 0..MAX_GRID_SIZE {
                    model.process_bytes("\n");

                    let new_scroll_top = view.scroll_top_in_lines(&model, INPUT_MODE, ctx);
                    assert_eq!(scroll_top_before_newlines, new_scroll_top);
                }
            }
        });
    })
}

#[test]
fn test_clear_buffer() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                for _ in 0..10 {
                    model.simulate_block("ls", "foo");
                }

                assert!(!model.block_list().blocks().is_empty());
            }

            view.bookmark_block(&BlockIndex::zero(), ctx);
            view.clear_buffer(ctx);

            {
                let model = view.model.lock();

                // There should be only one precmd block.
                assert_eq!(model.block_list().blocks().len(), 1);
                assert_eq!(view.bookmarked_blocks.len(), 0);
            }
        });
    })
}

#[test]
fn test_context_menu_includes_clear_when_block_list_non_empty() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.simulate_block("ls", "foo");
                assert!(!model.is_block_list_empty());
            }

            let menu_source = BlockListMenuSource::OutsideBlockRightClick {
                position_in_terminal_view: Vector2F::zero(),
            };
            let items = view.context_menu_items(&menu_source, ctx);
            let labels: Vec<&str> = items
                .iter()
                .filter_map(|item| item.fields().map(|fields| fields.label()))
                .collect();
            assert!(
                labels.contains(&"Clear Blocks"),
                "Expected `Clear Blocks` menu item, got {labels:?}"
            );
        });
    })
}

#[test]
fn test_context_menu_omits_clear_when_block_list_empty() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let model = view.model.lock();
                assert!(model.is_block_list_empty());
            }

            let menu_source = BlockListMenuSource::OutsideBlockRightClick {
                position_in_terminal_view: Vector2F::zero(),
            };
            let items = view.context_menu_items(&menu_source, ctx);
            let labels: Vec<&str> = items
                .iter()
                .filter_map(|item| item.fields().map(|fields| fields.label()))
                .collect();
            assert!(
                !labels.contains(&"Clear Blocks"),
                "Did not expect `Clear Blocks` menu item when block list is empty, got {labels:?}"
            );
        });
    })
}

#[test]
fn test_context_menu_omits_clear_for_text_right_click() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.simulate_block("ls", "foo");
                assert!(!model.is_block_list_empty());
            }

            let menu_source = BlockListMenuSource::RegularTextRightClick {
                position_in_terminal_view: Vector2F::zero(),
            };
            let items = view.context_menu_items(&menu_source, ctx);
            let labels: Vec<&str> = items
                .iter()
                .filter_map(|item| item.fields().map(|fields| fields.label()))
                .collect();
            assert!(
                !labels.contains(&"Clear Blocks"),
                "Did not expect `Clear Blocks` in text-selection right-click menu, got {labels:?}"
            );
        });
    })
}

#[test]
fn test_clear_buffer_clears_autosuggestion() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            // Set a next command suggestion (empty input)
            view.input.update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_autosuggestion(
                        "git status",
                        AutosuggestionLocation::EndOfBuffer,
                        AutosuggestionType::Command {
                            was_intelligent_autosuggestion: true,
                        },
                        ctx,
                    );
                });
            });

            // Verify autosuggestion is present
            view.input.read(ctx, |input, ctx| {
                input.editor().read(ctx, |editor, _ctx| {
                    assert!(
                        editor.active_autosuggestion(),
                        "Autosuggestion should be active before clear_buffer"
                    );
                });
            });

            // Clear the buffer
            view.clear_buffer(ctx);

            // Verify autosuggestion is cleared
            view.input.read(ctx, |input, ctx| {
                input.editor().read(ctx, |editor, _ctx| {
                    assert!(
                        !editor.active_autosuggestion(),
                        "Autosuggestion should be cleared after clear_buffer"
                    );
                });
            });
        });
    })
}

#[test]
fn test_bookmark_blocks_navigation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                for _ in 0..10 {
                    model.simulate_block("ls", "foo");
                }

                assert!(!model.block_list().blocks().is_empty());
            }

            view.bookmark_block(&BlockIndex::zero(), ctx);
            view.bookmark_block(&BlockIndex::from(1), ctx);
            view.bookmark_block(&BlockIndex::from(4), ctx);

            view.bookmark_up(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(4.into()));
            view.bookmark_down(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(0.into()));
            view.bookmark_up(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(4.into()));
            view.bookmark_up(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(1.into()));
            view.bookmark_up(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(0.into()));
            view.bookmark_down(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(1.into()));
            view.bookmark_down(ctx);
            assert_eq!(view.selected_blocks.tail(), Some(4.into()));
        });
    })
}

fn run_navigation_test(input_mode: InputMode) {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.read(&app, |view, _ctx| {
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
        });
        terminal.update(&mut app, |view, ctx| {
            InputModeSettings::handle(ctx).update(ctx, |input_mode_settings, ctx| {
                let _ = input_mode_settings.input_mode.set_value(input_mode, ctx);
            });

            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..100 {
                    model.simulate_block("ls", "foo");
                }

                // Put in one block that is larger than the viewport height.
                model.simulate_block("ls", "foo\n".repeat(100).as_str())
            }

            assert!(view.is_vertically_scrollable(ctx));
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );

            view.select_most_recent_blocks(1, ctx);
            assert_eq!(view.selected_blocks.tail(), Some(101.into()));

            view.select_less_recent_block(false /* is_shift_down */, ctx);
            assert_eq!(view.selected_blocks.tail(), Some(100.into()));

            view.select_more_recent_block(
                true,  /* is_cmd_down */
                false, /* is_shift_down */
                ctx,
            );
            assert_ne!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );
            assert_eq!(view.selected_blocks.tail(), Some(101.into()));

            view.select_more_recent_block(
                true,  /* is_cmd_down */
                false, /* is_shift_down */
                ctx,
            );
            if input_mode.is_inverted_blocklist() {
                // In the inverted case, we intentionally align to the
                // top of the most recent block here, not to its bottom
                assert!(matches!(
                    view.scroll_position(),
                    ScrollPosition::FixedAtPosition { .. }
                ));
            } else {
                assert_eq!(
                    view.scroll_position(),
                    ScrollPosition::FollowsBottomOfMostRecentBlock
                );
            }
            assert_eq!(view.selected_blocks.tail(), Some(101.into()));

            view.select_more_recent_block(
                true,  /* is_cmd_down */
                false, /* is_shift_down */
                ctx,
            );
            assert_eq!(view.selected_blocks.tail(), None);
        });
    });
}

#[test]
fn test_navigate_blocks() {
    run_navigation_test(InputMode::PinnedToBottom);
}

// #[test]
// fn test_navigate_blocks_inverted_blocklist() {
//     run_navigation_test(InputMode::PinnedToTop);
// }

#[test]
fn test_not_bootstrapped() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let model = view.model.lock();
            assert!(view.is_input_box_visible(&model, ctx));
            drop(model);

            assert_eq!(view.active_session_path_if_local(ctx), None);
        });
    })
}

#[test]
fn test_block_select() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.selected_blocks
                .toggle(10.into(), Some(11.into()), Some(9.into()));

            let single_mouse_down = BlockSelectAction::MouseDown(Some(1.into()));
            // On Mac, we use cmd-click to toggle block selections, but
            // we use ctrl-click on non-Mac platforms.
            let single_mouse_up = if cfg!(target_os = "macos") {
                BlockSelectAction::MouseUp {
                    block_index: 1.into(),
                    is_ctrl_down: false,
                    is_cmd_down: true,
                    is_shift_down: false,
                }
            } else {
                BlockSelectAction::MouseUp {
                    block_index: 1.into(),
                    is_ctrl_down: true,
                    is_cmd_down: false,
                    is_shift_down: false,
                }
            };
            view.block_select(&single_mouse_down, true, ctx);
            view.block_select(&single_mouse_up, true, ctx);
            assert!(view.selected_blocks.is_selected(1.into()));
            assert!(view.selected_blocks.is_selected(10.into()));

            let range_mouse_down = BlockSelectAction::MouseDown(Some(5.into()));
            let range_mouse_up = BlockSelectAction::MouseUp {
                block_index: 5.into(),
                is_ctrl_down: false,
                is_cmd_down: false,
                is_shift_down: true,
            };
            view.block_select(&range_mouse_down, true, ctx);
            view.block_select(&range_mouse_up, true, ctx);
            assert!(!view.selected_blocks.is_selected(10.into()));
            assert_eq!(view.selected_blocks_pivot_index(), Some(1.into()));
            assert_eq!(view.selected_blocks_tail_index(), Some(5.into()));
        });
    })
}

#[test]
fn test_select_all_blocks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..100 {
                    model.simulate_block("ls", "foo");
                }
            }
            assert!(view.is_vertically_scrollable(ctx));

            view.select_all_blocks(ctx);
            assert_eq!(view.selected_blocks_pivot_index().unwrap(), 1.into());
            assert_eq!(view.selected_blocks_tail_index().unwrap(), 100.into());
            for i in 1..100 {
                assert!(view.selected_blocks.is_selected(i.into()));
            }
        });
    })
}

#[test]
fn test_expand_selection_above_and_below() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                // Put in enough blocks so that the view should be scrollable
                for _ in 0..100 {
                    model.simulate_block("ls", "foo");
                }
            }
            assert!(view.is_vertically_scrollable(ctx));

            // helper to ensure indices are all selected
            fn assert_all_selected(selected_blocks: &SelectedBlocks, indices: Vec<BlockIndex>) {
                for &idx in indices.iter() {
                    assert!(selected_blocks.is_selected(idx));
                }
            }

            view.selected_blocks
                .toggle(5.into(), Some(6.into()), Some(4.into()));
            assert_all_selected(&view.selected_blocks, vec![5.into()]);

            view.select_more_recent_block(
                false, /* is_cmd_down */
                true,  /* is_shift_down */
                ctx,
            );
            assert_all_selected(&view.selected_blocks, vec![5.into(), 6.into()]);

            view.select_more_recent_block(
                false, /* is_cmd_down */
                true,  /* is_shift_down */
                ctx,
            );
            assert_all_selected(&view.selected_blocks, vec![5.into(), 6.into(), 7.into()]);

            view.select_less_recent_block(true /* is_shift_down */, ctx);
            assert_all_selected(&view.selected_blocks, vec![5.into(), 6.into()]);

            view.select_less_recent_block(true /* is_shift_down */, ctx);
            assert_all_selected(&view.selected_blocks, vec![5.into()]);

            view.select_less_recent_block(true /* is_shift_down */, ctx);
            assert_all_selected(&view.selected_blocks, vec![5.into(), 4.into()]);

            view.select_more_recent_block(
                false, /* is_cmd_down */
                true,  /* is_shift_down */
                ctx,
            );
            assert_all_selected(&view.selected_blocks, vec![5.into()]);
        });
    })
}

#[test]
fn test_copy_blocks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let (first_command, first_output) = ("ls", "foo");
            let (second_command, second_output) = ("pwd", "bar");

            {
                let mut model = view.model.lock();
                model.simulate_block(first_command, first_output);
                model.simulate_block(second_command, second_output);
            }

            // select a single block
            view.selected_blocks.toggle(2.into(), None, Some(1.into()));

            // test copy for a single block
            view.copy_blocks(BlockEntity::Command, ctx);
            assert_eq!(read_from_clipboard(ctx), second_command.to_string());

            view.copy_blocks(BlockEntity::Output, ctx);
            assert_eq!(read_from_clipboard(ctx), second_output.to_string());

            view.copy_blocks(BlockEntity::CommandAndOutput, ctx);
            assert_eq!(
                read_from_clipboard(ctx),
                format!("{second_command}\n{second_output}")
            );

            // select another block (in reverse)
            view.selected_blocks.toggle(1.into(), Some(2.into()), None);

            // test copy semantics for multiple blocks
            view.copy_blocks(BlockEntity::Command, ctx);
            let expected_commands_str = format!("{first_command}\n{second_command}");
            assert_eq!(read_from_clipboard(ctx), expected_commands_str);

            view.copy_blocks(BlockEntity::Output, ctx);
            let expected_outputs_str = format!("{first_output}\n{second_output}");
            assert_eq!(read_from_clipboard(ctx), expected_outputs_str);

            view.copy_blocks(BlockEntity::CommandAndOutput, ctx);
            let expected_both_str =
                format!("{first_command}\n{first_output}\n{second_command}\n{second_output}");
            assert_eq!(read_from_clipboard(ctx), expected_both_str);
        });
    })
}

#[test]
fn test_reinput_blocks() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let (first_command, first_output) = ("ls", "foo");
            let (second_command, second_output) = ("pwd", "bar");

            {
                let mut model = view.model.lock();
                model.simulate_block(first_command, first_output);
                model.simulate_block(second_command, second_output);
            }

            // test reinput command for single block
            view.selected_blocks.toggle(2.into(), None, Some(1.into()));
            view.reinput_commands(false /* as_root */, ctx);
            assert_eq!(view.input().as_ref(ctx).buffer_text(ctx), second_command);

            view.selected_blocks.toggle(2.into(), None, Some(1.into()));
            view.reinput_commands(true /* as_root */, ctx);
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                format!("sudo {second_command}")
            );

            // test reinput commands for multiple blocks (selected in reverse)
            view.selected_blocks.toggle(2.into(), None, Some(1.into()));
            view.selected_blocks.toggle(1.into(), Some(2.into()), None);
            view.reinput_commands(false /* as_root */, ctx);
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                format!("{first_command}\n{second_command}")
            );

            view.selected_blocks.toggle(2.into(), None, Some(1.into()));
            view.selected_blocks.toggle(1.into(), Some(2.into()), None);
            view.reinput_commands(true /* as_root */, ctx);
            assert_eq!(
                view.input().as_ref(ctx).buffer_text(ctx),
                format!("sudo {first_command}\nsudo {second_command}")
            );
        });
    })
}

fn run_find_test(input_mode: InputMode) {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            InputModeSettings::handle(ctx).update(ctx, |input_mode_settings, ctx| {
                let _ = input_mode_settings.input_mode.set_value(input_mode, ctx);
            });

            let (first_command, first_output) = ("ls", "foo");
            let (second_command, second_output) = ("pwd", "foobar foo beans");
            let (third_command, third_output) = ("fools", "baz");

            {
                let mut model = view.model.lock();
                model.simulate_block(first_command, first_output);
                model.simulate_block(second_command, second_output);
                model.simulate_block(third_command, third_output);
            }

            view.show_find_bar(ctx);

            // Test without find_in_block enabled (results should be selection-agnostic)
            view.find_bar.update(ctx, |view, _ctx| {
                view.display_find_within_block = FindWithinBlockState::Disabled;
            });

            // find when no block is selected
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("foo".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                4
            );
            assert_eq!(
                view.find_model
                    .as_ref(ctx)
                    .block_list_find_run()
                    .expect("BlockListFindRun exists.")
                    .focused_match_block_index()
                    .expect("Focused match exists."),
                3.into()
            );
            view.handle_find_event(
                &FindEvent::NextMatch {
                    direction: FindDirection::Down,
                },
                ctx,
            );
            if input_mode.is_inverted_blocklist() {
                // should go "down" to middle block
                assert_eq!(
                    view.find_model
                        .as_ref(ctx)
                        .block_list_find_run()
                        .expect("BlockListFindRun exists.")
                        .focused_match_block_index()
                        .expect("Focused match exists."),
                    2.into()
                );
            } else {
                // should loop to earliest block
                assert_eq!(
                    view.find_model
                        .as_ref(ctx)
                        .block_list_find_run()
                        .expect("BlockListFindRun exists.")
                        .focused_match_block_index()
                        .expect("Focused match exists."),
                    1.into()
                );
            }

            // find when a single block is selected
            view.selected_blocks.reset_to_single(2.into());
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("ls".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                2
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 1.into());
            assert_block_has_find_match(view.find_model.as_ref(ctx), 3.into());

            // Test with find_in_block enabled
            view.find_bar.update(ctx, |view, _ctx| {
                view.display_find_within_block = FindWithinBlockState::Enabled;
            });

            // find when no block is selected
            view.selected_blocks.reset();
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("foo".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                0
            );

            // find when a single block is selected
            view.selected_blocks.reset_to_single(2.into());
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("pwd".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 2.into());

            // find when multiple blocks are selected, and find in block is enabled
            view.selected_blocks.toggle(3.into(), Some(2.into()), None);
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("foo".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                3
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 2.into());
            assert_block_has_find_match(view.find_model.as_ref(ctx), 3.into());
        });
    })
}

#[test]
fn test_find_in_blocks() {
    run_find_test(InputMode::PinnedToBottom);
}

#[test]
fn test_find_in_blocks_inverted_blocklist() {
    run_find_test(InputMode::PinnedToTop);
}

#[test]
fn test_case_sensitive_find() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let (first_command, first_output) = ("ls", "foo");
            let (second_command, second_output) = ("pwd", "fOObar");
            let (third_command, third_output) = ("FoOls", "baz");

            {
                let mut model = view.model.lock();
                model.simulate_block(first_command, first_output);
                model.simulate_block(second_command, second_output);
                model.simulate_block(third_command, third_output);
            }

            view.show_find_bar(ctx);

            // Test without case sensitivity enabled (no blocks enabled)
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("fOO".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                3
            );

            // Test without case sensitivity enabled, but with find in block
            view.find_bar.update(ctx, |view, _ctx| {
                view.display_find_within_block = FindWithinBlockState::Enabled;
            });
            view.selected_blocks.reset_to_single(1.into());
            view.update_find_selection(ctx);
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 1.into());

            // Test with case sensitivity enabled (one block enabled)
            view.handle_find_event(
                &FindEvent::ToggleCaseSensitivity {
                    is_case_sensitive: true,
                },
                ctx,
            );
            view.selected_blocks.reset_to_single(1.into());
            view.update_find_selection(ctx);
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                0
            );

            view.selected_blocks.reset_to_single(2.into());
            view.update_find_selection(ctx);
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 2.into());

            // Test with case sensitivity enabled (no blocks enabled)
            view.selected_blocks.reset();
            view.find_bar.update(ctx, |view, _ctx| {
                view.display_find_within_block = FindWithinBlockState::Disabled;
            });
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
            assert_block_has_find_match(view.find_model.as_ref(ctx), 2.into());

            // Change regex to mismatch case sensitivity across all blocks
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("FOO".to_string()),
                },
                ctx,
            );
            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                0
            );
        });
    })
}

#[test]
fn test_find_bar_prefix_search() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let (command_1, output_1) = ("echo foo", "foo");
            let (command_2, output_2) = ("echo bar foo", "bar foo");

            {
                let mut model = view.model.lock();
                model.simulate_block(command_1, output_1);
                model.simulate_block(command_2, output_2);
            }

            view.show_find_bar(ctx);

            // Test without regex enabled
            view.handle_find_event(
                &FindEvent::Update {
                    query: Some("^foo".to_string()),
                },
                ctx,
            );

            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                0
            );

            view.handle_find_event(
                &FindEvent::ToggleRegexSearch {
                    is_regex_enabled: true,
                },
                ctx,
            );

            assert_eq!(
                view.find_model.as_ref(ctx).visible_block_list_match_count(),
                1
            );
        });
    });
}

#[test]
fn test_create_notification_shorter_than_max() {
    let command = "cargo run";
    let output = "error: failed to find directory";
    let command_succeeded = false;
    let block_duration = Duration::new(4, 2);

    let trigger = NotificationsTrigger::LongRunningCommand(command_succeeded, block_duration);

    let actual_content =
        trigger.create_notification_content(command.to_string(), output.to_string());

    let expected_title = format!("'{command}' failed after 4s");
    let expected_body = format!("{BODY_PREFIX}{output}");

    assert_eq!(actual_content.title, expected_title);
    assert_eq!(actual_content.body, expected_body);
}

#[test]
fn test_create_notification_as_long_as_max() {
    let expected_title_suffix = " finished after 4s";
    let max_command_len = UserNotification::MAX_TITLE_LENGTH - expected_title_suffix.len() - 2;
    let command = "a".repeat(max_command_len);

    let max_output_len = UserNotification::MAX_BODY_LENGTH - BODY_PREFIX.len();
    let output = "a".repeat(max_output_len);

    let command_succeeded = true;
    let block_duration = Duration::new(4, 2);

    let trigger = NotificationsTrigger::LongRunningCommand(command_succeeded, block_duration);

    let actual_content =
        trigger.create_notification_content(command.to_string(), output.to_string());

    let expected_title = format!("'{command}'{expected_title_suffix}");
    let expected_body = format!("{BODY_PREFIX}{output}");

    assert_eq!(actual_content.title, expected_title);
    assert_eq!(actual_content.body, expected_body);
}

#[test]
fn test_create_notification_longer_than_max() {
    let expected_title_suffix = " finished after 4s";
    let max_command_len = UserNotification::MAX_TITLE_LENGTH - expected_title_suffix.len() - 2;
    let command = "a".repeat(max_command_len + 1);

    let max_output_len = UserNotification::MAX_BODY_LENGTH - BODY_PREFIX.len();
    let output = "a".repeat(max_output_len + 1);

    let command_succeeded = true;
    let block_duration = Duration::new(4, 2);

    let trigger = NotificationsTrigger::LongRunningCommand(command_succeeded, block_duration);

    let actual_content =
        trigger.create_notification_content(command.to_string(), output.to_string());

    let expected_title = format!(
        "'{}...'{expected_title_suffix}",
        &command[..max_command_len - 3]
    );
    let expected_body = format!("{BODY_PREFIX}...{}", &output[..max_output_len - 3]);

    assert_eq!(actual_content.title, expected_title);
    assert_eq!(actual_content.body, expected_body);
}

#[test]
fn test_create_notification_char_boundaries_respected() {
    let expected_title_suffix = " finished after 4s";
    let max_command_len = UserNotification::MAX_TITLE_LENGTH - expected_title_suffix.len() - 2;
    let command = "😊".repeat(max_command_len + 1);

    let output = "error: failed to find directory";
    let command_succeeded = true;
    let block_duration = Duration::new(4, 2);

    let trigger = NotificationsTrigger::LongRunningCommand(command_succeeded, block_duration);

    let actual_content = trigger.create_notification_content(command, output.to_string());

    let expected_command_prefix = "😊".repeat(max_command_len - 3);
    let expected_title = format!("'{expected_command_prefix}...'{expected_title_suffix}",);
    assert_eq!(actual_content.title, expected_title);
}

#[test]
fn test_banner_for_incompatible_plugins() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        SessionSettings::handle(&app).update(&mut app, |session_settings, ctx| {
            let _ = session_settings.honor_ps1.set_value(true, ctx);
        });

        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "zsh".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "zsh".to_owned(),
                shell_plugins: Some(HashSet::from(["p10k_unsupported".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            200 => terminal.read(&app, |view, _ctx| view
                .is_incompatible_configuration_banner_open),
            "Banner did not open in time"
        );
    })
}

/// Regression test for #9011: the slow-bootstrap banner used to persist
/// indefinitely when shell integration never sent the bootstrap signal
/// (e.g. the user's shell `exec`s into `expect` before Warp's integration
/// runs). The auto-dismiss timer scheduled when the banner opens must
/// eventually close it.
#[test]
fn test_slow_bootstrap_banner_auto_dismisses() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Open the banner directly and schedule a short-duration auto-dismiss
        // timer. We bypass `on_bootstrap_failed_timer_complete` (which itself
        // waits the 7-second bootstrap timeout) to keep this test fast — the
        // important behavior under test is the auto-dismiss path.
        terminal.update(&mut app, |view, ctx| {
            view.is_slow_bootstrap_banner_open = true;
            view.slow_bootstrap_banner_auto_dismiss_handle = Some(
                view.start_slow_bootstrap_banner_auto_dismiss_timer(Duration::from_millis(50), ctx),
            );
        });

        assert!(terminal.read(&app, |view, _ctx| view.is_slow_bootstrap_banner_open));

        assert_eventually!(
            200 => terminal.read(&app, |view, _ctx| !view.is_slow_bootstrap_banner_open
                && view.slow_bootstrap_banner_auto_dismiss_handle.is_none()),
            "Slow bootstrap banner did not auto-dismiss"
        );
    })
}

/// Regression test for #9011: when the banner is dismissed by another path
/// (manual user dismissal or a successful bootstrap event), any pending
/// auto-dismiss timer should be aborted so it can't fire after the fact.
#[test]
fn test_hide_slow_bootstrap_banner_aborts_pending_auto_dismiss() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.is_slow_bootstrap_banner_open = true;
            view.slow_bootstrap_banner_auto_dismiss_handle =
                Some(view.start_slow_bootstrap_banner_auto_dismiss_timer(
                    // Long enough that the timer can't fire before we hide.
                    Duration::from_secs(60),
                    ctx,
                ));
            view.hide_slow_bootstrap_banner(ctx);
        });

        terminal.read(&app, |view, _ctx| {
            assert!(!view.is_slow_bootstrap_banner_open);
            assert!(view.slow_bootstrap_banner_auto_dismiss_handle.is_none());
        });
    })
}

// Regression test for GH#3548 / GH#6093: the "Seems like your completions are not
// working" banner must offer a permanent "Don't show me again" dismissal that is
// persisted, while the "x" close button keeps its existing per-session behavior.
#[test]
fn test_control_master_banner_permanent_dismissal_persists() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            // Temporary dismissal (the "x" button) clears the banner for this session
            // but must not persist a "don't show again" preference.
            view.control_master_error_banner_state.is_open = true;
            view.handle_controlmaster_error_banner_event(
                &BannerEvent::Dismiss(DismissalType::Temporary),
                ctx,
            );
            assert!(!view.control_master_error_banner_state.is_open);
            assert!(!view.control_master_error_banner_suppressed);
            assert_eq!(
                ctx.private_user_preferences()
                    .read_value(CONTROL_MASTER_BANNER_SUPPRESSED_KEY)
                    .unwrap(),
                None,
                "temporary dismissal should not persist a preference"
            );

            // Permanent dismissal ("Don't show me again") clears the banner and persists
            // the choice so it never reopens.
            view.control_master_error_banner_state.is_open = true;
            view.handle_controlmaster_error_banner_event(
                &BannerEvent::Dismiss(DismissalType::Permanent),
                ctx,
            );
            assert!(!view.control_master_error_banner_state.is_open);
            assert!(view.control_master_error_banner_suppressed);
            assert_eq!(
                ctx.private_user_preferences()
                    .read_value(CONTROL_MASTER_BANNER_SUPPRESSED_KEY)
                    .unwrap(),
                Some("true".to_owned()),
                "permanent dismissal should persist a preference"
            );
        });
    })
}

// Regression test for GH#3548 / GH#6093: once the banner has been permanently
// dismissed it must not reopen on subsequent sessions.
#[test]
fn test_control_master_banner_suppressed_does_not_reopen() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        terminal.update(&mut app, |view, _ctx| {
            // With no remote server and no prior dismissal, the banner may open.
            view.control_master_error_banner_suppressed = false;
            assert!(view.should_open_control_master_banner(/* has_remote_server */ false));
            // A remote server makes the CTA irrelevant, so it stays closed.
            assert!(!view.should_open_control_master_banner(/* has_remote_server */ true));

            // Once permanently dismissed it must never reopen, even without a remote server.
            view.control_master_error_banner_suppressed = true;
            assert!(!view.should_open_control_master_banner(false));
        });
    })
}

#[test]
fn test_bash_vim_banner_already_shown() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // The banner has already been shown and dismissed.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::Dismissed, ctx);
        });

        // Ensure Warp's vim keybindings are off.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(false, ctx);
        });

        // Bootstrap a bash session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "bash".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // Since the user already dismissed the banner, it should not
            // be shown again.
            terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_none()
            }),
            "Banner should not have opened"
        );
    })
}

#[test]
fn test_bash_vim_banner_on() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure the banner has never been shown.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::NotDismissed, ctx);
        });

        // Ensure Warp's vim keybindings are off.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(false, ctx);
        });

        // Bootstrap a bash session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "bash".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should display.
            200 => terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_some()
            }),
            "Banner did not open in time"
        );
    })
}

#[test]
fn test_bash_vim_banner_off() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure the banner has never been shown.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::NotDismissed, ctx);
        });

        // Ensure Warp's vim keybindings are on.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(true, ctx);
        });

        // Bootstrap a bash session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "bash".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "bash".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should NOT display
            // because the user already has vim keybindings turned on.
            terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_none()
            }),
            "Banner should not have opened"
        );
    })
}

#[test]
fn test_zsh_vim_banner_on() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure the banner has never been shown.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::NotDismissed, ctx);
        });

        // Ensure Warp's vim keybindings are off.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(false, ctx);
        });

        // Bootstrap a zsh session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "zsh".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "zsh".to_owned(),
                shell_plugins: Some(HashSet::from(["vi".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should display.
            200 => terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_some()
            }),
            "Banner did not open in time"
        );
    })
}

#[test]
fn test_zsh_vim_banner_off() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure the banner has never been shown.
        VimBannerSettings::handle(&app).update(&mut app, |banner_settings, ctx| {
            let _ = banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::NotDismissed, ctx);
        });

        // Ensure Warp's vim keybindings are on.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(true, ctx);
        });

        // Bootstrap a zsh session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "zsh".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "zsh".to_owned(),
                shell_plugins: Some(HashSet::from(["vi".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should NOT display
            // because the user already has vim keybindings turned on.
            terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_none()
            }),
            "Banner should not have opened"
        );
    })
}

#[test]
fn test_fish_vim_banner_on() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure Warp's vim keybindings are off.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(false, ctx);
        });

        // Bootstrap a fish session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "fish".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "fish".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should display.
            200 => terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_some()
            }),
            "Banner did not open in time"
        );
    })
}

#[test]
fn test_fish_vim_banner_off() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal =
            MockTerminalManager::create_new_terminal_view_window_for_test(&mut app, None);

        // Ensure the terminal is the active session.
        terminal.update(&mut app, |view, ctx| {
            let terminal_pane_id = TerminalPaneId::dummy_terminal_pane_id();
            let focus_state = ctx.add_model(|_| {
                PaneGroupFocusState::new(terminal_pane_id.into(), Some(terminal_pane_id), false)
            });
            let focus_handle = PaneFocusHandle::new(terminal_pane_id.into(), focus_state);
            view.set_focus_handle(focus_handle, ctx);
        });

        // Ensure Warp's vim keybindings are on.
        AppEditorSettings::handle(&app).update(&mut app, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(true, ctx);
        });

        // Bootstrap a fish session with vi mode enabled.
        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "fish".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "fish".to_owned(),
                shell_options: Some(HashSet::from(["vi_mode".to_string()])),
                ..Default::default()
            });
        });

        // This is asynchronous because we're waiting for the bootstrap event
        // to be sent from the terminal model to the terminal view.
        assert_eventually!(
            // The vim keybinding banner should NOT display
            // because the user already has vim keybindings turned on.
            terminal.read(&app, |terminal, _terminal_ctx| {
                terminal.inline_banners_state.vim_banner_state.is_none()
            }),
            "Banner should not have opened"
        );
    })
}

#[test]
fn test_prompt_context_menu_items_for_ps1() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        SessionSettings::handle(&app).update(&mut app, |session_settings, ctx| {
            let _ = session_settings.honor_ps1.set_value(true, ctx);
        });

        terminal.read(&app, |view, ctx| {
            let items = view.prompt_context_menu_items(ctx);
            let len = items.len();
            assert_eq!(len, 3);
            assert_eq!(items[0].fields().unwrap().label(), "Copy prompt");
            assert!(items[1].is_separator());
            assert_eq!(items[2].fields().unwrap().label(), "Edit prompt");
            assert!(!items[2].fields().unwrap().is_disabled());
        });
    })
}

#[test]
fn test_prompt_context_menu_items_for_context_chips() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let model = view.model.lock();
            view.current_prompt.update(ctx, |prompt, ctx| {
                let PromptType::Dynamic { prompt } = prompt else {
                    return;
                };
                prompt.update(ctx, |prompt, ctx| {
                    prompt.update_context(model.block_list().active_block(), ctx)
                });
            })
        });

        // Set the prompt to something we can actually read for.
        let prompt = Prompt::handle(&app);
        prompt.update(&mut app, |prompt, ctx| {
            prompt
                .update(
                    [ContextChipKind::Time12],
                    false,
                    WarpPromptSeparator::None,
                    ctx,
                )
                .expect("updating prompt to time chip failed");
        });

        let session_settings = SessionSettings::handle(&app);
        session_settings.update(&mut app, |settings, ctx| {
            // Force a toggle so the change event fires.
            let _ = settings.honor_ps1.set_value(true, ctx);
            let _ = settings.honor_ps1.set_value(false, ctx);
        });

        terminal.read(&app, |view, ctx| {
            let items: Vec<MenuItem<TerminalAction>> = view.prompt_context_menu_items(ctx);
            assert_eq!(items.len(), 5);

            // We expect the prompt menu items to be something like the following when context chips are used:
            // Copy prompt
            // ------------
            // <context chip specific actions>
            // ------------
            // Edit prompt
            assert_eq!(items[0].fields().unwrap().label(), "Copy prompt");
            assert!(items[1].is_separator());
            assert_eq!(
                items[2].fields().unwrap().label(),
                "Copy Time (12-hour format)"
            );
            assert!(items[3].is_separator());
            assert_eq!(items[4].fields().unwrap().label(), "Edit prompt");
            assert!(!items[4].fields().unwrap().is_disabled());
        });
    })
}

#[test]
fn test_prompt_context_menu_items_for_no_context_chips() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let model = view.model.lock();
            view.current_prompt.update(ctx, |prompt, ctx| {
                let PromptType::Dynamic { prompt } = prompt else {
                    return;
                };
                prompt.update(ctx, |prompt, ctx| {
                    prompt.update_context(model.block_list().active_block(), ctx)
                });
            })
        });

        let session_settings = SessionSettings::handle(&app);
        session_settings.update(&mut app, |settings, ctx| {
            let _ = settings.honor_ps1.set_value(false, ctx);
        });

        terminal.read(&app, |view, ctx| {
            let items: Vec<MenuItem<TerminalAction>> = view.prompt_context_menu_items(ctx);
            assert_eq!(items.len(), 3);

            // We expect the prompt menu items to be something like the following when no context chips exist:
            // Copy prompt
            // ------------
            // Edit prompt
            assert_eq!(items[0].fields().unwrap().label(), "Copy prompt");
            assert!(items[1].is_separator());
            assert_eq!(items[2].fields().unwrap().label(), "Edit prompt");
            assert!(!items[2].fields().unwrap().is_disabled());
        });
    })
}

#[test]
fn test_prompt_context_menu_items_for_agent_toolbelt_flag() {
    let _agent_view_guard = FeatureFlag::AgentView.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view");
            });
        });

        {
            let _agent_footer_guard = FeatureFlag::AgentToolbarEditor.override_enabled(false);
            terminal.read(&app, |view, ctx| {
                let items = view.prompt_context_menu_items(ctx);
                let labels = items
                    .iter()
                    .filter_map(|item| item.fields().map(|fields| fields.label()))
                    .collect::<Vec<_>>();

                assert!(!labels.contains(&"Edit prompt"));
                assert!(!labels.contains(&"Edit agent toolbelt"));
            });
        }

        {
            let _agent_footer_guard = FeatureFlag::AgentToolbarEditor.override_enabled(true);
            terminal.read(&app, |view, ctx| {
                let items = view.prompt_context_menu_items(ctx);
                let labels = items
                    .iter()
                    .filter_map(|item| item.fields().map(|fields| fields.label()))
                    .collect::<Vec<_>>();
                assert!(!labels.contains(&"Edit prompt"));
                assert!(labels.contains(&"Edit agent toolbelt"));
            });
        }
    })
}

#[test]
fn agent_footer_updates_chip_groups_when_side_assignment_changes() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let model = view.model.lock();
            view.current_prompt.update(ctx, |prompt, ctx| {
                let PromptType::Dynamic { prompt } = prompt else {
                    return;
                };
                prompt.update(ctx, |prompt, ctx| {
                    prompt.update_context(model.block_list().active_block(), ctx);
                });
            });
        });

        SessionSettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings.agent_footer_chip_selection.set_value(
                AgentToolbarChipSelection::Custom {
                    left: vec![AgentToolbarItemKind::ContextChip(ContextChipKind::Time12)],
                    right: vec![AgentToolbarItemKind::ContextChip(ContextChipKind::Time24)],
                },
                ctx,
            );
        });

        assert_eventually!(
            terminal.read(&app, |view, ctx| {
                view.input().as_ref(ctx).agent_footer_chip_kinds(ctx)
                    == (vec![ContextChipKind::Time12], vec![ContextChipKind::Time24])
            }),
            "Agent footer should render separate left and right chip groups"
        );

        SessionSettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings.agent_footer_chip_selection.set_value(
                AgentToolbarChipSelection::Custom {
                    left: vec![
                        AgentToolbarItemKind::ContextChip(ContextChipKind::Time12),
                        AgentToolbarItemKind::ContextChip(ContextChipKind::Time24),
                    ],
                    right: vec![],
                },
                ctx,
            );
        });

        assert_eventually!(
            terminal.read(&app, |view, ctx| {
                view.input().as_ref(ctx).agent_footer_chip_kinds(ctx)
                    == (
                        vec![ContextChipKind::Time12, ContextChipKind::Time24],
                        vec![],
                    )
            }),
            "Agent footer should update when a chip moves between sides without changing overall chip order"
        );
    })
}

#[test]
fn test_link_at_range_trims_zero_width_spaces() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // NOTE: this has two zero-width spaces, one after the '(', and one before the ')'
        let input_url = "(\u{200b}https://warp.dev\u{200b})";
        // NOTE: the final character in this string is a zero-width space
        let non_escaped_url = "https://warp.dev\u{200b}";
        let escaped_url = "https://warp.dev";

        terminal.update(&mut app, |view, _ctx| {
            view.model.lock().simulate_block(
                r"printf '(%bhttps://warp.dev%b)\n' '\U200b' '\U200b'",
                input_url,
            );
        });

        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();

            let block = view
                .viewport_state(model.block_list(), InputMode::PinnedToBottom, ctx)
                .iter()
                .next()
                .expect("blocklist should have at least one item");

            let point = WithinModel::BlockList(WithinBlock::new(
                // I picked the point 0, 4 b/c it seemed to work. It's not clear to me
                // why 4 works when numbers like 9 do not. Either way, this is just to
                // get the actual url out (passing 9 fails on url_at_point), and does
                // not matter for testing link_at_range.
                Point::new(0, 4),
                block.block_index.expect("block index should exist"),
                crate::terminal::GridType::Output,
            ));

            let url = model
                .url_at_point(&point)
                .expect("url at the designated point should exist");

            // Assert that string_at_range preserves the ZW Space
            assert_eq!(
                model.string_at_range(&url, RespectObfuscatedSecrets::No),
                non_escaped_url
            );

            // Assert that link_at_range removes the ZW Space
            assert_eq!(
                model.link_at_range(&url, RespectObfuscatedSecrets::No),
                escaped_url
            );
        });
    })
}

#[test]
fn test_scroll_position_doesnt_change_when_block_finished() {
    use futures_lite::StreamExt;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let (tx, rx) = async_channel::bounded(1);
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::BlockCompleted { block, .. } = event {
                    let output = std::str::from_utf8(&block.stylized_output).unwrap();
                    if output.trim() == "lr" {
                        tx.try_send(()).expect("Can send over channel");
                    }
                }
            });
        });

        let scroll_position_before_finished = terminal.update(&mut app, |view, ctx| {
            // Finish a lengthy block.
            view.model.lock().simulate_block("ls", &"\n".repeat(1000));
            assert!(view.is_vertically_scrollable(ctx));
            assert_eq!(
                view.scroll_position(),
                ScrollPosition::FollowsBottomOfMostRecentBlock
            );

            // Start long-running block.
            view.model.lock().simulate_long_running_block("", "lr");

            // Before the block is finished, scroll up.
            view.scroll(1.0.into_lines(), ctx);
            let scroll_position_before_finished = view.scroll_position();
            assert!(matches!(
                scroll_position_before_finished,
                ScrollPosition::FixedAtPosition { .. }
            ));

            // Finish the block.
            view.model.lock().finish_block();

            scroll_position_before_finished
        });

        // Wait until the terminal view acknowledges the block as completed.
        assert!(pin!(rx).next().await.is_some());

        // Make sure the scroll position is unchanged when the block finishes.
        terminal.read(&app, |view, _| {
            let scroll_position_after_finished = view.scroll_position();
            assert_eq!(
                scroll_position_before_finished,
                scroll_position_after_finished
            );
        });
    })
}

#[test]
fn inline_agent_view_exits_when_tagged_in_long_running_command_is_tagged_out() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.init_shell(InitShellValue {
                    session_id: 0.into(),
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.bootstrapped(BootstrappedValue {
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.simulate_long_running_block("sleep 10", "running");
            }

            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_inline_agent_view(
                        None,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                    .expect("should enter inline agent view for a tagged-in command");
            });
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);

            assert!(view.agent_view_controller().as_ref(ctx).is_inline());
            assert!(
                view.model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_tagged_in()
            );

            let model = view.model.lock();
            assert!(view.is_input_box_visible(&model, ctx));
            drop(model);

            view.handle_action(&TerminalAction::SetInputModeTerminal, ctx);

            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
            let model = view.model.lock();
            let active_block = model.block_list().active_block();
            assert!(!active_block.is_agent_tagged_in());
            assert!(!view.is_input_box_visible(&model, ctx));
        });
    })
}

#[test]
fn ctrl_c_after_stop_takeover_cancels_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(view.view_id, false, false, false, ctx)
                });

            view.model
                .lock()
                .simulate_long_running_block("sleep 20", "running");
            let task_id = TaskId::new("test-cli-subagent".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("command should become agent monitored");

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(
                    UserTakeOverReason::Stop {
                        should_auto_resume: true,
                    },
                    ctx,
                );
            });

            conversation_id
        });

        terminal.update(&mut app, |view, ctx| {
            view.handle_action(&TerminalAction::CtrlC, ctx);
        });

        assert_eq!(*pty_writes.borrow(), vec![vec![C0::ETX]]);
        terminal.read(&app, |_, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::Cancelled);
        });
    })
}

#[test]
fn ctrl_c_after_transfer_takeover_does_not_cancel_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(view.view_id, false, false, false, ctx)
                });

            view.model
                .lock()
                .simulate_long_running_block("ssh localhost", "Password:");
            let task_id = TaskId::new("test-cli-subagent".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("command should become agent monitored");

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(
                    UserTakeOverReason::TransferFromAgent {
                        reason: "Enter your password".to_owned(),
                    },
                    ctx,
                );
            });

            conversation_id
        });

        terminal.update(&mut app, |view, ctx| {
            view.handle_action(&TerminalAction::CtrlC, ctx);
        });

        assert_eq!(*pty_writes.borrow(), vec![vec![C0::ETX]]);
        terminal.read(&app, |_, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&conversation_id)
                .expect("conversation should exist");
            // Transfer takeover is still waiting to report control handback or command
            // completion to the agent, so Ctrl-C should interrupt the command without
            // cancelling the conversation.
            assert_eq!(conversation.status(), &ConversationStatus::InProgress);
        });
    })
}

#[test]
fn completed_user_controlled_lrc_resumes_when_not_suppressed() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(view.view_id, false, false, false, ctx)
                });

            view.model
                .lock()
                .simulate_long_running_block("sleep 20", "running");
            let task_id = TaskId::new("test-cli-subagent".to_owned());
            let block_id = {
                let mut model = view.model.lock();
                let active_block = model.block_list_mut().active_block_mut();
                active_block.set_agent_interaction_mode_for_requested_command(
                    AIAgentActionId::from("requested-command".to_owned()),
                    Some(task_id.clone()),
                    conversation_id,
                );
                active_block
                    .set_agent_interaction_mode_for_agent_monitored_command(
                        &task_id,
                        conversation_id,
                    )
                    .expect("command should become agent monitored");
                active_block
                    .take_over_control_for_user(UserTakeOverReason::Stop {
                        should_auto_resume: true,
                    })
                    .expect("user takeover should succeed");
                active_block.id().clone()
            };

            assert!(
                !view
                    .ai_controller
                    .as_ref(ctx)
                    .has_active_stream_for_conversation(conversation_id, ctx)
            );

            view.on_user_block_completed(&block_id, ctx);

            // A Ctrl-C takeover (Stop) without an explicit teardown should resume the
            // conversation once the command completes, just like a manual takeover.
            assert!(
                view.ai_controller
                    .as_ref(ctx)
                    .has_active_stream_for_conversation(conversation_id, ctx)
            );
        });
    })
}

#[test]
fn completed_user_controlled_lrc_skips_resume_when_suppressed() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(view.view_id, false, false, false, ctx)
                });

            view.model
                .lock()
                .simulate_long_running_block("sleep 20", "running");
            let task_id = TaskId::new("test-cli-subagent".to_owned());
            let block_id = {
                let mut model = view.model.lock();
                let active_block = model.block_list_mut().active_block_mut();
                active_block.set_agent_interaction_mode_for_requested_command(
                    AIAgentActionId::from("requested-command".to_owned()),
                    Some(task_id.clone()),
                    conversation_id,
                );
                active_block
                    .set_agent_interaction_mode_for_agent_monitored_command(
                        &task_id,
                        conversation_id,
                    )
                    .expect("command should become agent monitored");
                // Mirrors rewind / stop_local_agent_conversation tearing down the conversation.
                active_block.set_user_control_for_teardown();
                active_block.id().clone()
            };

            view.on_user_block_completed(&block_id, ctx);

            assert!(
                !view
                    .ai_controller
                    .as_ref(ctx)
                    .has_active_stream_for_conversation(conversation_id, ctx)
            );
        });
    })
}

#[test]
fn inline_agent_view_persists_across_transfer_takeover_for_monitored_long_running_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.init_shell(InitShellValue {
                    session_id: 0.into(),
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.bootstrapped(BootstrappedValue {
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.simulate_long_running_block("ssh localhost", "Password:");
            }

            let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_inline_agent_view(
                        None,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                    .expect("inline agent view should create a conversation")
            });
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);

            let task_id = TaskId::new("test-task".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("tagged-in command should transition to agent-monitored");

            assert!(view.agent_view_controller().as_ref(ctx).is_inline());

            let model = view.model.lock();
            assert!(model.block_list().active_block().is_agent_in_control());
            assert!(view.is_input_box_visible(&model, ctx));
            drop(model);

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(
                    UserTakeOverReason::TransferFromAgent {
                        reason: "Enter your password".to_owned(),
                    },
                    ctx,
                );
            });

            assert!(view.agent_view_controller().as_ref(ctx).is_inline());
            let model = view.model.lock();
            let active_block = model.block_list().active_block();
            assert!(active_block.is_eligible_for_agent_handoff());
            assert!(!view.is_input_box_visible(&model, ctx));
            drop(model);

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.handoff_active_command_control_to_agent(ctx);
            });

            assert!(view.agent_view_controller().as_ref(ctx).is_inline());
            let model = view.model.lock();
            let active_block = model.block_list().active_block();
            assert!(active_block.is_agent_in_control());
            assert!(view.is_input_box_visible(&model, ctx));
        });
    })
}

#[test]
fn use_agent_footer_renders_for_transfer_handoff_even_when_user_command_footer_setting_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .should_render_use_agent_footer_for_user_commands
                .set_value(false, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.init_shell(InitShellValue {
                    session_id: 0.into(),
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.bootstrapped(BootstrappedValue {
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.simulate_long_running_block("ssh localhost", "Password:");
            }

            view.maybe_show_use_agent_footer_in_blocklist(ctx);
            {
                let model = view.model.lock();
                assert!(!view.should_render_use_agent_footer(&model, ctx));
                let active_block_index = model.block_list().active_block_index();
                assert!(
                    model
                        .block_list()
                        .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                        .is_none()
                );
            }

            let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_inline_agent_view(
                        None,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                    .expect("inline agent view should create a conversation")
            });
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);

            let task_id = TaskId::new("test-task".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("tagged-in command should transition to agent-monitored");

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(
                    UserTakeOverReason::TransferFromAgent {
                        reason: "Enter your password".to_owned(),
                    },
                    ctx,
                );
            });

            view.maybe_show_use_agent_footer_in_blocklist(ctx);
            let model = view.model.lock();
            assert!(view.should_render_use_agent_footer(&model, ctx));
            let active_block_index = model.block_list().active_block_index();
            let rendered_footer_view_id = model
                .block_list()
                .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                .map(|(_, item)| item.view_id);
            assert_eq!(rendered_footer_view_id, Some(view.use_agent_footer.id()));
        });
    })
}

#[test]
fn exiting_agent_view_removes_empty_conversations() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        // Enter agent view (creates new conversation)
        let conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        // Entering agent view without specifying a conversation creates a new conversation.
        let exists_before_exit = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            history
                .conversation(&conversation_id)
                .is_some_and(|c| c.exchange_count() == 0)
        });
        assert!(exists_before_exit);

        // Sanity: conversation exists but has no exchanges.
        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller()
                .update(ctx, |controller, ctx| controller.exit_agent_view(ctx))
        });

        // Exiting agent view should remove the empty conversation.
        let exists_after_exit = BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            history.conversation(&conversation_id).is_some()
        });
        assert!(!exists_after_exit);
        // Zap:空会话被清理后也不能留下 restored 入口卡片。
        let has_entry_card_after_exit = terminal.read(&app, |view, _| {
            view.has_agent_view_entry_block_for_conversation(conversation_id)
        });
        assert!(
            !has_entry_card_after_exit,
            "新建但未修改的空会话退出后不应留下 restored 入口卡片"
        );
    })
}

#[test]
fn ctrl_c_exit_agent_view_requires_confirmation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        // Enter agent view (creates new conversation)
        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        // First ctrl-c should arm confirmation but not exit.
        terminal.update(&mut app, |view, ctx| {
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 0,
                },
                ctx,
            );
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });

        // Second ctrl-c should confirm and exit.
        terminal.update(&mut app, |view, ctx| {
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 0,
                },
                ctx,
            );
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

#[test]
fn ctrl_c_buffer_clear_then_exit_requires_three_presses_in_agent_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        // 1st ctrl-c clears input buffer (simulated) and should not trigger cancel/exit.
        terminal.update(&mut app, |view, ctx| {
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 5,
                },
                ctx,
            );
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });

        // 2nd ctrl-c arms exit confirmation.
        terminal.update(&mut app, |view, ctx| {
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 0,
                },
                ctx,
            );
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });

        // 3rd ctrl-c confirms and exits.
        terminal.update(&mut app, |view, ctx| {
            view.handle_input_event(
                &InputEvent::CtrlC {
                    cleared_buffer_len: 0,
                },
                ctx,
            );
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

#[test]
fn terminal_action_ctrl_c_exit_agent_view_requires_confirmation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        terminal.update(&mut app, |view, ctx| {
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
            view.handle_action(&TerminalAction::CtrlC, ctx);
            assert!(view.agent_view_controller().as_ref(ctx).is_active());
        });

        terminal.update(&mut app, |view, ctx| {
            view.handle_action(&TerminalAction::CtrlC, ctx);
            assert!(!view.agent_view_controller().as_ref(ctx).is_active());
        });
    })
}

/// Sets up a CLI agent session, opens rich input, submits `text`, and returns
/// the terminal handle and the collected PTY writes.
#[allow(clippy::type_complexity)]
fn submit_rich_input_and_collect_pty_writes(
    app: &mut App,
    agent: CLIAgent,
    text: &str,
) -> (ViewHandle<TerminalView>, Rc<RefCell<Vec<Vec<u8>>>>) {
    let terminal = add_window_with_terminal(app, None);
    let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
    let writes = pty_writes.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(&terminal, move |_, event, _| {
            if let Event::WriteBytesToPty { bytes } = event {
                writes.borrow_mut().push(bytes.to_vec());
            }
        });
    });

    terminal.update(app, |view, ctx| {
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.set_session(
                view.view_id,
                CLIAgentSession {
                    agent,
                    status: CLIAgentSessionStatus::InProgress,
                    session_context: CLIAgentSessionContext::default(),
                    input_state: CLIAgentInputState::Closed,
                    should_auto_toggle_input: false,
                    listener: None,
                    remote_host: None,
                    plugin_version: None,
                    draft_text: None,
                    custom_command_prefix: None,
                    received_rich_notification: false,
                },
                ctx,
            );
        });

        view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        assert!(view.has_active_cli_agent_input_session(ctx));

        view.submit_cli_agent_rich_input(text.to_owned(), ctx);
    });

    (terminal, pty_writes)
}

fn open_cli_agent_rich_input_for_agent(app: &mut App, agent: CLIAgent) -> ViewHandle<TerminalView> {
    open_cli_agent_rich_input_for_agent_with_window_id(app, agent).1
}

fn open_cli_agent_rich_input_for_agent_with_window_id(
    app: &mut App,
    agent: CLIAgent,
) -> (WindowId, ViewHandle<TerminalView>) {
    let (window_id, terminal) = add_window_with_id_and_terminal(app, None);
    terminal.update(app, |view, ctx| {
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.set_session(
                view.view_id,
                CLIAgentSession {
                    agent,
                    status: CLIAgentSessionStatus::InProgress,
                    session_context: CLIAgentSessionContext::default(),
                    input_state: CLIAgentInputState::Closed,
                    should_auto_toggle_input: false,
                    listener: None,
                    remote_host: None,
                    plugin_version: None,
                    draft_text: None,
                    custom_command_prefix: None,
                    received_rich_notification: false,
                },
                ctx,
            );
        });

        view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
        assert!(view.has_active_cli_agent_input_session(ctx));
    });
    (window_id, terminal)
}

/// Verifies that Ctrl-G closes CLI agent rich input when dispatched from the
/// focused editor context. This is a regression test for #9286 where the
/// keybinding only matched the terminal context, not the embedded editor.
#[test]
fn ctrl_g_closes_cli_agent_rich_input_when_editor_is_focused() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        // Register keybindings so keystroke dispatch can match the Ctrl-G binding.
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let (window_id, terminal) =
            open_cli_agent_rich_input_for_agent_with_window_id(&mut app, CLIAgent::OpenCode);

        // Dispatch Ctrl-G through the focused editor's responder chain.
        let (input_id, editor_id) = terminal.read(&app, |view, ctx| {
            let input = view.input.clone();
            let editor = input.as_ref(ctx).editor().clone();
            (input.id(), editor.id())
        });
        let handled = app
            .dispatch_keystroke(
                window_id,
                &[terminal.id(), input_id, editor_id],
                &warpui::keymap::Keystroke::parse("ctrl-g").expect("valid keystroke"),
                false,
            )
            .expect("dispatch should succeed");

        assert!(handled, "ctrl-g should be handled from the focused editor");
        terminal.read(&app, |view, ctx| {
            assert!(
                !view.has_active_cli_agent_input_session(ctx),
                "rich input should be closed after Ctrl-G"
            );
        });
    })
}

/// Verifies that Ctrl-G closes CLI agent rich input when dispatched from the
/// terminal context alone (no editor in the responder chain). Regression test
/// for #9916 where the keybinding only opened rich input but did not close it
/// in scenarios where focus was outside the embedded editor and the active
/// block had transitioned out of `LongRunningCommand` — for example, when the
/// CLI agent has paused waiting for user input.
#[test]
fn ctrl_g_closes_cli_agent_rich_input_from_terminal_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        // Register keybindings so keystroke dispatch can match the Ctrl-G binding.
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let (window_id, terminal) =
            open_cli_agent_rich_input_for_agent_with_window_id(&mut app, CLIAgent::OpenCode);

        // Dispatch Ctrl-G with only the terminal view in the responder chain.
        // This simulates the case where focus is not on the embedded editor
        // (e.g., on the block list) and the previous Case 1 / Case 2 predicates
        // would both fail to match.
        let handled = app
            .dispatch_keystroke(
                window_id,
                &[terminal.id()],
                &warpui::keymap::Keystroke::parse("ctrl-g").expect("valid keystroke"),
                false,
            )
            .expect("dispatch should succeed");

        assert!(
            handled,
            "ctrl-g should be handled from the terminal context when rich input is open"
        );
        terminal.read(&app, |view, ctx| {
            assert!(
                !view.has_active_cli_agent_input_session(ctx),
                "rich input should be closed after Ctrl-G from terminal context"
            );
        });
    })
}

/// Verifies that Ctrl-G is a true toggle: opens then closes rich input from
/// the terminal context. Regression test for #9916.
#[test]
fn ctrl_g_toggles_cli_agent_rich_input_from_terminal_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        app.update(|ctx| {
            crate::terminal::init(ctx);
            crate::editor::init(ctx);
        });
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        // Start with rich input open, then close via Ctrl-G, then re-open via
        // direct call (Ctrl-G open path requires LongRunningCommand which is
        // tricky to simulate in a unit test), then close via Ctrl-G again.
        let (window_id, terminal) =
            open_cli_agent_rich_input_for_agent_with_window_id(&mut app, CLIAgent::OpenCode);

        let keystroke = warpui::keymap::Keystroke::parse("ctrl-g").expect("valid keystroke");

        // First close: rich input is open → Ctrl-G should close.
        let handled = app
            .dispatch_keystroke(window_id, &[terminal.id()], &keystroke, false)
            .expect("dispatch should succeed");
        assert!(handled, "first ctrl-g should be handled (close)");
        terminal.read(&app, |view, ctx| {
            assert!(
                !view.has_active_cli_agent_input_session(ctx),
                "rich input should be closed after first Ctrl-G"
            );
        });

        // Re-open programmatically (mirrors the user re-triggering open via
        // Ctrl-G in a long-running context).
        terminal.update(&mut app, |view, ctx| {
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::CtrlG, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));
        });

        // Second close: rich input is open again → Ctrl-G should close again.
        let handled = app
            .dispatch_keystroke(window_id, &[terminal.id()], &keystroke, false)
            .expect("dispatch should succeed");
        assert!(handled, "second ctrl-g should be handled (close again)");
        terminal.read(&app, |view, ctx| {
            assert!(
                !view.has_active_cli_agent_input_session(ctx),
                "rich input should be closed after second Ctrl-G"
            );
        });
    })
}

#[test]
fn cli_agent_rich_input_hint_text_mentions_active_cli_agent() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        for (agent, expected_hint_text) in [
            (CLIAgent::Claude, "Enter prompt for Claude Code..."),
            (CLIAgent::Gemini, "Enter prompt for Gemini..."),
            (CLIAgent::Codex, "Enter prompt for Codex..."),
            (CLIAgent::Unknown, "Tell the agent what to build..."),
        ] {
            let terminal = open_cli_agent_rich_input_for_agent(&mut app, agent);
            terminal.read(&app, |view, ctx| {
                let placeholder_text = view
                    .input
                    .as_ref(ctx)
                    .editor()
                    .as_ref(ctx)
                    .placeholder_text("");
                assert_eq!(placeholder_text, Some(expected_hint_text));
            });
        }
    })
}

#[test]
fn cli_agent_rich_input_shell_mode_uses_run_commands_hint_text() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.ai_input_model().update(ctx, |ai_input, ctx| {
                    ai_input.set_input_config(
                        InputConfig {
                            input_type: InputType::Shell,
                            is_locked: true,
                        },
                        true,
                        None,
                        ctx,
                    );
                });
                input.set_zero_state_hint_text(ctx);
            });
        });
        terminal.read(&app, |view, ctx| {
            let placeholder_text = view
                .input
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .placeholder_text("");
            assert_eq!(placeholder_text, Some("Run commands"));
        });
    })
}

#[test]
fn submit_cli_agent_rich_input_codex_uses_bracketed_paste() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let (_terminal, pty_writes) =
            submit_rich_input_and_collect_pty_writes(&mut app, CLIAgent::Codex, "hello");

        let writes = pty_writes.borrow();
        // BracketedPaste: first write is ESC[200~ + text + ESC[201~, second is \r.
        assert_eq!(
            writes.len(),
            2,
            "expected 2 PTY writes, got {}",
            writes.len()
        );

        let mut expected_paste =
            Vec::with_capacity(BRACKETED_PASTE_START.len() + 5 + BRACKETED_PASTE_END.len());
        expected_paste.extend_from_slice(BRACKETED_PASTE_START);
        expected_paste.extend_from_slice(b"hello");
        expected_paste.extend_from_slice(BRACKETED_PASTE_END);
        assert_eq!(writes[0], expected_paste);
        assert_eq!(writes[1], b"\r");
    })
}

/// Verifies that multi-line Hermes rich input is delivered as a single bracketed
/// paste payload with a standalone \r submit. Embedded newlines must remain
/// inside the paste instead of triggering separate submissions.
#[test]
fn submit_cli_agent_rich_input_hermes_multiline_uses_bracketed_paste() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let (_terminal, pty_writes) =
            submit_rich_input_and_collect_pty_writes(&mut app, CLIAgent::Hermes, "line1\nline2");

        let writes = pty_writes.borrow();
        // BracketedPaste: first write is ESC[200~ + both lines + ESC[201~, second is \r.
        // The embedded \n between lines must NOT split into a separate write or trigger
        // a second submission — that was the voice-input auto-submit regression.
        assert_eq!(
            writes.len(),
            2,
            "expected 2 PTY writes (paste payload + submit \r), got {}: {:?}",
            writes.len(),
            writes
        );

        let mut expected_paste =
            Vec::with_capacity(BRACKETED_PASTE_START.len() + 11 + BRACKETED_PASTE_END.len());
        expected_paste.extend_from_slice(BRACKETED_PASTE_START);
        expected_paste.extend_from_slice(b"line1\nline2");
        expected_paste.extend_from_slice(BRACKETED_PASTE_END);
        assert_eq!(
            writes[0], expected_paste,
            "first write should be the full bracketed paste payload"
        );
        assert_eq!(
            writes[1], b"\r",
            "second write should be the standalone submit \r"
        );
    })
}

#[test]
fn submit_cli_agent_rich_input_opencode_defers_enter_and_close() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let (_terminal, pty_writes) =
            submit_rich_input_and_collect_pty_writes(&mut app, CLIAgent::OpenCode, "hello");

        // Immediately after submit, only the text should have been written;
        // the \r is sent after a short delay.
        assert_eq!(pty_writes.borrow().len(), 1);
        assert_eq!(pty_writes.borrow()[0], b"hello");

        // Wait for the delayed \r to arrive.
        assert_eventually!(
            pty_writes.borrow().len() == 2,
            "carriage return should be written after delay"
        );
        assert_eq!(pty_writes.borrow()[1], b"\r");
    })
}

#[test]
fn attach_path_as_context_routes_to_open_cli_agent_rich_input() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        let _hoa_code_review = FeatureFlag::HoaCodeReview.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            view.attach_path_as_context(std::path::Path::new("src/main.rs"), ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(view.input.as_ref(ctx).buffer_text(ctx), "src/main.rs");
        });
        assert!(
            pty_writes.borrow().is_empty(),
            "context should be inserted into rich input instead of written to PTY"
        );
    })
}
#[test]
fn drag_drop_image_in_cli_agent_long_running_command_pastes_via_clipboard() {
    // Regression test: dropping an image file into a tab where a CLI agent
    // (e.g. Claude Code) is the foreground long-running process should
    // mirror the Cmd+V image-paste path — write the image to the system
    // clipboard and send the agent's paste keystroke to the PTY — instead
    // of shell-escaping the path and typing it into the agent's prompt.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        // The new path actually reads the file off disk, so we need a real
        // file. Bytes don't have to be a valid PNG.
        let mut image_path = std::env::temp_dir();
        image_path.push(format!(
            "warp-test-cli-agent-drop-{}.png",
            std::process::id()
        ));
        std::fs::write(&image_path, b"fake-png-bytes").expect("write tmp image");
        let image_path_str = image_path.to_string_lossy().into_owned();

        let terminal = add_window_with_terminal(&mut app, None);

        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            // The CLI-agent paste branch is gated on the active block being
            // long-running (the agent's TUI). Without a long-running block
            // we'd fall through to the regular image-attach flow.
            {
                let mut model = view.model.lock();
                model.simulate_long_running_block("claude", "");
                assert!(
                    model
                        .block_list()
                        .active_block()
                        .is_active_and_long_running()
                );
            }

            view.drag_and_drop_files(&[image_path_str], ctx);
        });

        // The paste flow is async (off-thread file read, then hop back to
        // the view to write the clipboard + paste keystroke). Wait for the
        // single PTY write of the platform-appropriate paste byte: 0x16
        // (Ctrl+V) on macOS/Linux, or `ESC v` on Windows. Without the fix
        // a shell-escaped path string is written here instead.
        let expected_paste_bytes: Vec<u8> = if cfg!(windows) {
            vec![0x1b, b'v']
        } else {
            vec![0x16]
        };
        assert_eventually!(
            pty_writes.borrow().len() == 1 && pty_writes.borrow()[0] == expected_paste_bytes,
            "expected single paste-keystroke PTY write {:?}; got {:?}",
            expected_paste_bytes,
            pty_writes.borrow()
        );

        std::fs::remove_file(&image_path).ok();
    })
}

#[test]
fn paste_raw_image_clipboard_in_cli_agent_sends_correct_bytes() {
    fn run_for_agent(agent: CLIAgent) {
        App::test((), move |mut app| async move {
            initialize_app_for_terminal_view(&mut app);
            let _agent_view = FeatureFlag::AgentView.override_enabled(true);

            let terminal = add_window_with_terminal(&mut app, None);

            let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
            let writes = pty_writes.clone();
            app.update(|ctx| {
                ctx.subscribe_to_view(&terminal, move |_, event, _| {
                    if let Event::WriteBytesToPty { bytes } = event {
                        writes.borrow_mut().push(bytes.to_vec());
                    }
                });
            });

            terminal.update(&mut app, |view, ctx| {
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    sessions.set_session(
                        view.view_id,
                        CLIAgentSession {
                            agent,
                            status: CLIAgentSessionStatus::InProgress,
                            session_context: CLIAgentSessionContext::default(),
                            input_state: CLIAgentInputState::Closed,
                            should_auto_toggle_input: false,
                            listener: None,
                            remote_host: None,
                            plugin_version: None,
                            draft_text: None,
                            custom_command_prefix: None,
                            received_rich_notification: false,
                        },
                        ctx,
                    );
                });

                {
                    let mut model = view.model.lock();
                    model.simulate_long_running_block(agent.command_prefix(), "");
                    model.set_mode(ansi::Mode::BracketedPaste);
                }

                // Write image-only data to the clipboard (no text, no paths).
                ctx.clipboard().write(ClipboardContent {
                    images: Some(vec![warpui::clipboard::ImageData {
                        data: vec![0x89, 0x50, 0x4E, 0x47], // PNG magic bytes
                        mime_type: "image/png".to_string(),
                        filename: None,
                    }]),
                    ..Default::default()
                });

                view.handle_action(&TerminalAction::Paste, ctx);
            });

            let writes = pty_writes.borrow();
            assert_eq!(
                writes.len(),
                1,
                "expected 1 PTY write, got {}",
                writes.len()
            );

            if cfg!(windows) {
                if agent == CLIAgent::Claude {
                    assert_eq!(writes[0], vec![C0::ESC, b'v']);
                } else {
                    let mut expected = Vec::new();
                    expected.extend_from_slice(BRACKETED_PASTE_START);
                    expected.extend_from_slice(BRACKETED_PASTE_END);
                    assert_eq!(writes[0], expected);
                }
            } else {
                assert_eq!(writes[0], vec![C0::SYN]);
            }
        })
    }

    run_for_agent(CLIAgent::Claude);
    run_for_agent(CLIAgent::OpenCode);
    run_for_agent(CLIAgent::Codex);
}

#[test]
fn submit_without_auto_dismiss_keeps_rich_input_open() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        // auto_dismiss defaults to false — leave it off.

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            view.submit_cli_agent_rich_input("hello".to_owned(), ctx);

            // Rich input stays open because auto_dismiss is off.
            assert!(view.has_active_cli_agent_input_session(ctx));
        });

        // Buffer should still be cleared even though rich input is open.
        terminal.read(&app, |view, ctx| {
            let input = view.input.as_ref(ctx);
            assert!(input.editor().as_ref(ctx).buffer_text(ctx).is_empty());
        });
    })
}

#[test]
fn submit_with_plugin_and_auto_toggle_keeps_rich_input_open() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        // auto_toggle_rich_input defaults to true.
        // Turn on auto_dismiss too — it should be overridden by auto_toggle.
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .auto_dismiss_rich_input_after_submit
                .set_value(true, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            // Create a session with a plugin listener and should_auto_toggle_input.
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: true,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            view.submit_cli_agent_rich_input("hello".to_owned(), ctx);

            // Rich input stays open because auto_toggle + plugin takes precedence.
            assert!(view.has_active_cli_agent_input_session(ctx));
        });
    })
}

#[test]
fn submit_with_plugin_but_auto_toggle_off_respects_auto_dismiss() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings.auto_toggle_rich_input.set_value(false, ctx);
            let _ = settings
                .auto_dismiss_rich_input_after_submit
                .set_value(true, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            view.submit_cli_agent_rich_input("hello".to_owned(), ctx);
        });

        // auto_toggle is off, so auto_dismiss closes rich input.
        // Claude uses DelayedEnter, so the close happens after a timer.
        assert_eventually!(
            terminal.read(&app, |view, ctx| !view
                .has_active_cli_agent_input_session(ctx)),
            "Rich input should be closed after submit with auto_dismiss"
        );
    })
}

#[test]
fn exiting_lrc_user_takeover_does_not_insert_agent_view_entry_card() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            {
                let mut model = view.model.lock();
                model.init_shell(InitShellValue {
                    session_id: 0.into(),
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.bootstrapped(BootstrappedValue {
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.simulate_long_running_block("ssh localhost", "Password:");
            }

            let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_inline_agent_view(
                        None,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                    .expect("inline agent view should create a conversation")
            });
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);

            let task_id = TaskId::new("test-task".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("tagged-in command should transition to agent-monitored");

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(
                    UserTakeOverReason::TransferFromAgent {
                        reason: "Enter your password".to_owned(),
                    },
                    ctx,
                );
            });

            view.agent_view_controller()
                .update(ctx, |controller, ctx| controller.exit_agent_view(ctx));

            assert!(
                !view.has_agent_view_entry_block_for_conversation(conversation_id),
                "LRC 用户接管退出时不应重复插入 AgentView 入口卡片"
            );
        });
    })
}

#[test]
fn status_blocked_auto_closes_rich_input() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        // auto_toggle_rich_input defaults to true.

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            // Simulate a PermissionRequest event → status transitions to Blocked.
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionRequest,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            summary: Some("Approve?".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        // The StatusChanged event is delivered to the terminal view, which
        // auto-closes rich input because the agent is blocked.
        terminal.read(&app, |view, ctx| {
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        // should_auto_toggle_input is preserved so auto-open can fire later.
        terminal.read(&app, |_view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx).session(_view.view_id);
            assert!(session.unwrap().should_auto_toggle_input);
        });
    })
}

#[test]
fn status_in_progress_auto_opens_rich_input_after_blocked() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            // Open rich input, then simulate blocked → closed automatically.
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionRequest,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            summary: Some("Approve?".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        // Rich input should be auto-closed from the blocked status.
        terminal.read(&app, |view, ctx| {
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        // Simulate permission replied → status transitions back to InProgress.
        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionReplied,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload::default(),
                    },
                    ctx,
                );
            });
        });

        // Rich input should auto-open because should_auto_toggle_input was preserved.
        terminal.read(&app, |view, ctx| {
            assert!(view.has_active_cli_agent_input_session(ctx));
        });
    })
}

// Regression test for https://github.com/warpdotdev/warp/issues/9059.
// Codex's listener doesn't emit Blocked-state events (it only forwards opaque
// OSC 9 notifications as Stop), so auto-toggling rich input would trap arrow
// keys when Codex shows interactive option menus. Auto-toggle must not fire
// for agents whose handlers report `supports_rich_status() == false`.
#[test]
fn codex_status_change_does_not_auto_open_rich_input() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        // auto_toggle_rich_input defaults to true.

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Codex,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Codex,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            // Rich input starts closed. Simulating a Stop event (the only
            // status Codex's handler ever emits) must not re-open it,
            // because the user may be navigating Codex's option menus.
            assert!(!view.has_active_cli_agent_input_session(ctx));
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::CodexOsc9Fallback,
                        v: 1,
                        agent: CLIAgent::Codex,
                        event: CLIAgentEventType::Stop,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            query: Some("Agent turn complete".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |view, ctx| {
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });
    })
}

#[test]
fn cli_session_status_updates_active_child_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let child_conversation_id = terminal.update(&mut app, |view, ctx| {
            let parent_conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(view.view_id, false, false, false, ctx)
                });
            let child_conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_child_conversation(
                        view.view_id,
                        "Agent 2".to_string(),
                        parent_conversation_id,
                        None,
                        ctx,
                    )
                });

            view.enter_agent_view(
                None,
                Some(child_conversation_id),
                AgentViewEntryOrigin::ChildAgent,
                ctx,
            );

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            child_conversation_id
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::InProgress);
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionRequest,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            summary: Some("Approve?".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(
                conversation.status(),
                &ConversationStatus::Blocked {
                    blocked_action: "Approve?".to_string(),
                }
            );
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionReplied,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload::default(),
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::InProgress);
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::Stop,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            response: Some("Done".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::Success);
        });
    })
}

#[test]
fn cli_session_status_updates_single_child_conversation_without_agent_view() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let child_conversation_id = terminal.update(&mut app, |view, ctx| {
            let parent_conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(view.view_id, false, false, false, ctx)
                });
            let child_conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_child_conversation(
                        view.view_id,
                        "Agent 2".to_string(),
                        parent_conversation_id,
                        None,
                        ctx,
                    )
                });

            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        remote_host: None,
                        plugin_version: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            child_conversation_id
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::InProgress);
        });

        terminal.update(&mut app, |view, ctx| {
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::Stop,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            response: Some("Done".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
        });

        terminal.read(&app, |_view, ctx| {
            let conversation = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&child_conversation_id)
                .expect("child conversation should exist");
            assert_eq!(conversation.status(), &ConversationStatus::Success);
        });
    })
}

#[test]
fn manual_dismiss_disables_auto_toggle_for_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            let listener = ctx.add_model(|ctx| {
                CLIAgentSessionListener::new(
                    view.view_id,
                    CLIAgent::Claude,
                    &view.model_events_handle,
                    ctx,
                )
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view.view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: true,
                        listener: Some(listener),
                        remote_host: None,
                        plugin_version: Some("1.0.0".to_owned()),
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                    },
                    ctx,
                );
            });

            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));

            // Manual dismiss via the "disable auto-toggle" path (Escape / Ctrl-G / footer).
            view.close_cli_agent_rich_input_and_disable_auto_toggle(ctx);
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        // should_auto_toggle_input should now be false.
        terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx).session(view.view_id);
            assert!(!session.unwrap().should_auto_toggle_input);
        });

        // A status change to InProgress should NOT auto-open rich input.
        terminal.update(&mut app, |view, ctx| {
            // First move to Blocked so we can transition back.
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionRequest,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload {
                            summary: Some("Approve?".to_owned()),
                            ..Default::default()
                        },
                    },
                    ctx,
                );
            });
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.update_from_event(
                    view.view_id,
                    &CLIAgentEvent {
                        source: CLIAgentEventSource::RichPlugin,
                        v: 1,
                        agent: CLIAgent::Claude,
                        event: CLIAgentEventType::PermissionReplied,
                        session_id: None,
                        cwd: None,
                        project: None,
                        payload: CLIAgentEventPayload::default(),
                    },
                    ctx,
                );
            });
        });

        // Rich input should remain closed.
        terminal.read(&app, |view, ctx| {
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });
    })
}

#[test]
fn cli_agent_rich_input_open_sets_terminal_keymap_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);
        terminal.read(&app, |view, ctx| {
            let keymap_context = <TerminalView as warpui::View>::keymap_context(view, ctx);
            assert!(
                keymap_context
                    .set
                    .contains(crate::settings_view::flags::CLI_AGENT_RICH_INPUT_OPEN),
                "terminal keymap context should expose open CLI agent rich input"
            );
        });
    })
}

// 行为测试：当 rich input 已打开时，再次触发 ToggleCLIAgentRichInput action
// （即 Ctrl-G 实际命中绑定后被 dispatch 的 handler）应当关闭 rich input。
// 这覆盖了本次修复的核心 toggle 路径。
#[test]
fn ctrl_g_action_closes_open_cli_agent_rich_input() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(ImportedConfigModel::new);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);
        terminal.update(&mut app, |view, ctx| {
            assert!(
                view.is_cli_agent_rich_input_open(ctx),
                "rich input should be open before toggling"
            );
            view.handle_action(&TerminalAction::ToggleCLIAgentRichInput, ctx);
            assert!(
                !view.is_cli_agent_rich_input_open(ctx),
                "ToggleCLIAgentRichInput should close rich input when already open"
            );
        });
    })
}

#[test]
fn close_cli_agent_rich_input_saves_draft_and_reopen_restores_it() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);

        // Type some text into the composer.
        terminal.update(&mut app, |view, ctx| {
            view.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("work in progress", ctx);
            });
        });

        // Close the composer — the buffer text should be saved as a draft.
        terminal.update(&mut app, |view, ctx| {
            view.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::Manual, ctx);
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .expect("session should exist");
            assert_eq!(
                session.draft_text.as_deref(),
                Some("work in progress"),
                "draft should be saved on close"
            );
        });

        // Reopen — draft should be restored into the buffer and consumed.
        terminal.update(&mut app, |view, ctx| {
            view.open_cli_agent_rich_input(CLIAgentInputEntrypoint::FooterButton, ctx);
            assert!(view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "work in progress",
                "draft should be restored on reopen"
            );
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .expect("session should exist");
            assert_eq!(
                session.draft_text, None,
                "draft should be consumed after restore"
            );
        });
    })
}

#[test]
fn submit_cli_agent_rich_input_clears_draft() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            // Keep the input open after submit so we can inspect the buffer.
            let _ = settings
                .auto_dismiss_rich_input_after_submit
                .set_value(false, ctx);
        });

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);

        terminal.update(&mut app, |view, ctx| {
            view.submit_cli_agent_rich_input("hello agent".to_owned(), ctx);
            // Input stays open because auto-dismiss is off.
            assert!(view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .expect("session should exist");
            assert_eq!(
                session.draft_text, None,
                "draft should be cleared after submit"
            );
            assert!(
                view.input.as_ref(ctx).buffer_text(ctx).is_empty(),
                "buffer should be empty after submit"
            );
        });
    })
}

#[test]
fn close_cli_agent_rich_input_with_empty_buffer_stores_no_draft() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _cli_rich = FeatureFlag::CLIAgentRichInput.override_enabled(true);

        let terminal = open_cli_agent_rich_input_for_agent(&mut app, CLIAgent::Claude);

        // Close immediately without typing anything.
        terminal.update(&mut app, |view, ctx| {
            view.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::Manual, ctx);
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });

        terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .expect("session should exist");
            assert_eq!(
                session.draft_text, None,
                "no draft should be stored for empty buffer"
            );
        });
    })
}

#[test]
fn ctrl_c_does_not_accept_prompt_suggestion_banner() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let block_id = terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.simulate_block("ls", "output");
            let last_completed_block_index = BlockIndex(model.block_list().blocks().len() - 2);
            model
                .block_list()
                .block_at(last_completed_block_index)
                .unwrap()
                .id()
                .clone()
        });

        terminal.update(&mut app, |view, ctx| {
            view.on_legacy_prompt_suggestion_generated(
                AgentModePromptSuggestion::Success(PromptSuggestion {
                    id: "suggestion".to_owned(),
                    label: Some("Do something".to_owned()),
                    prompt: "Do something".to_owned(),
                    coding_query_context: None,
                    static_prompt_suggestion_name: None,
                    should_start_new_conversation: false,
                }),
                block_id.clone(),
                "ls".to_owned(),
                0,
                ctx,
            );

            assert!(
                view.inline_banners_state
                    .prompt_suggestions_banner
                    .is_some()
            );

            // Ctrl-C should not accept the prompt suggestion.
            view.handle_action(&TerminalAction::CtrlC, ctx);

            assert!(
                view.inline_banners_state
                    .prompt_suggestions_banner
                    .is_some()
            );
        });
    })
}

/// Regression test for GH703: a Linear deeplink prompt must never be auto-submitted
/// to the LLM. Because `LinearDeepLink` returns `AutoTriggerBehavior::Never`, the
/// prompt must land in the input buffer as a draft and the "press enter again to
/// send" ephemeral message must be shown so the user can inspect and explicitly
/// send it.
#[test]
fn linear_deeplink_populates_input_as_draft_when_not_in_agent_view() {
    use super::agent_view::ENTER_AGAIN_TO_SEND_MESSAGE_ID;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(
                Some("attacker prompt".to_owned()),
                AgentViewEntryOrigin::LinearDeepLink,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            assert!(
                view.agent_view_controller().as_ref(ctx).is_active(),
                "Linear deeplink should enter agent view"
            );
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "attacker prompt",
                "Linear deeplink prompt should be placed in the input buffer as a draft"
            );
            let ephemeral_message_id = view
                .ephemeral_message_model
                .as_ref(ctx)
                .current_message()
                .and_then(|msg| msg.id().map(|id| id.to_owned()));
            assert_eq!(
                ephemeral_message_id.as_deref(),
                Some(ENTER_AGAIN_TO_SEND_MESSAGE_ID),
                "the 'enter again to send' affordance should be shown"
            );
        });
    })
}

/// The critical regression guard for GH703: even when the user is already in
/// fullscreen agent view, a Linear deeplink prompt must not be auto-submitted to
/// the LLM. `LinearDeepLink` returns `AutoTriggerBehavior::Never`, so even the
/// `was_in_agent_view_already` shortcut cannot promote it to auto-submit.
#[test]
fn linear_deeplink_does_not_auto_submit_when_already_in_agent_view() {
    use super::agent_view::ENTER_AGAIN_TO_SEND_MESSAGE_ID;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        // First enter fullscreen agent view with no initial prompt. This matches the
        // pre-condition in the issue: the focused terminal is already in fullscreen
        // agent view when the `warp://linear/work?prompt=...` URI is dispatched.
        let original_conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            })
        });

        terminal.read(&app, |view, ctx| {
            assert!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .is_fullscreen()
            );
        });

        // Now dispatch the Linear deeplink while already in fullscreen agent view.
        terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(
                Some("attacker prompt".to_owned()),
                AgentViewEntryOrigin::LinearDeepLink,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            // A new conversation should have been created for the Linear deeplink.
            let new_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should still be active after Linear deeplink entry");
            assert_ne!(
                new_conversation_id, original_conversation_id,
                "Linear deeplink should open a new conversation"
            );

            // The prompt must be a draft, not auto-submitted.
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "attacker prompt",
                "Linear deeplink prompt must stay as a draft in the input buffer"
            );
            let ephemeral_message_id = view
                .ephemeral_message_model
                .as_ref(ctx)
                .current_message()
                .and_then(|msg| msg.id().map(|id| id.to_owned()));
            assert_eq!(
                ephemeral_message_id.as_deref(),
                Some(ENTER_AGAIN_TO_SEND_MESSAGE_ID),
                "the 'enter again to send' affordance must be shown so the user can \
                 consciously send the Linear-originated prompt"
            );
        });
    })
}

/// `LinearDeepLink` returns `AutoTriggerBehavior::Never`, so it must not
/// auto-submit regardless of prior agent-view state.
#[test]
fn linear_deeplink_via_default_entrypoint_does_not_auto_submit_in_fullscreen() {
    use super::agent_view::ENTER_AGAIN_TO_SEND_MESSAGE_ID;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("Should be able to enter agent view")
            });
        });

        terminal.update(&mut app, |view, ctx| {
            view.enter_agent_view_for_new_conversation(
                Some("attacker prompt".to_owned()),
                AgentViewEntryOrigin::LinearDeepLink,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.input.as_ref(ctx).buffer_text(ctx),
                "attacker prompt",
                "Linear deeplink prompt must not be auto-submitted"
            );
            let ephemeral_message_id = view
                .ephemeral_message_model
                .as_ref(ctx)
                .current_message()
                .and_then(|msg| msg.id().map(|id| id.to_owned()));
            assert_eq!(
                ephemeral_message_id.as_deref(),
                Some(ENTER_AGAIN_TO_SEND_MESSAGE_ID),
            );
        });
    })
}

/// Regression test for https://github.com/warpdotdev/warp/issues/11212.
///
/// Closing the find bar must immediately clear find highlights on AI blocks.
/// AI blocks are separate child views, so unless `close_find_bar` clears find
/// state they keep stale highlights until the pane is refocused.
#[test]
fn close_find_bar_clears_ai_block_find_highlights() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        // Create an AI block whose user query contains a searchable term.
        terminal.update(&mut app, |view, ctx| {
            append_exchange_and_handle_event(
                view,
                AIAgentInput::UserQuery {
                    query: "highlight the needle here".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                },
                ctx,
            );
        });

        let ai_block = terminal.read(&app, |view, _| {
            view.rich_content_views
                .iter()
                .find_map(|rich_content| {
                    rich_content
                        .ai_block_metadata()
                        .map(|metadata| metadata.ai_block_handle.clone())
                })
                .expect("an AI block should have been inserted")
        });

        // Open the find bar and run find for a term that matches the AI block.
        // The blocklist find pipeline only visits rich-content views that have
        // been laid out, which does not happen in this headless test, so also
        // drive the AI block's `run_find` directly to put it in the
        // highlighted state.
        let find_options = || FindOptions {
            query: Some("needle".to_owned().into()),
            ..Default::default()
        };
        terminal.update(&mut app, |view, ctx| {
            view.show_find_bar(ctx);
            view.run_find(find_options(), ctx);
        });
        ai_block.update(&mut app, |block, ctx| {
            crate::terminal::find::FindableRichContentView::run_find(block, &find_options(), ctx);
        });

        assert_eq!(
            ai_block.read(&app, |block, _| block.find_match_count()),
            1,
            "running find should highlight the match inside the AI block"
        );

        // Dismissing the find bar must clear the AI block's find highlights.
        terminal.update(&mut app, |view, ctx| {
            view.close_find_bar(ctx);
        });

        assert_eq!(
            ai_block.read(&app, |block, _| block.find_match_count()),
            0,
            "closing the find bar should immediately clear AI block find highlights"
        );
    })
}

/// Regression test for the async-find branch of #11212.
///
/// Closing the find bar must clear stale AI block highlights without dropping
/// the saved query options on the async-find path. `open_find_bar` reads
/// `active_find_options` to restore the previous query; if `close_find_bar`
/// routes through `clear_matches → AsyncFindController::clear_results`, that
/// helper resets `current_find_options` and reopening the find bar starts
/// from a blank query instead of the previous one.
#[test]
fn close_find_bar_preserves_options_on_async_find_path() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _async_find = FeatureFlag::AsyncFind.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        let needle_options = || FindOptions {
            query: Some("needle".to_owned().into()),
            ..Default::default()
        };

        terminal.update(&mut app, |view, ctx| {
            view.show_find_bar(ctx);
            view.run_find(needle_options(), ctx);
        });

        // The async controller should have saved the active query.
        assert_eq!(
            terminal.read(&app, |view, ctx| view
                .find_model
                .as_ref(ctx)
                .active_find_options()
                .map(|o| o.query.clone())),
            Some(needle_options().query),
            "running find on the async path must save the active query"
        );

        // Closing the find bar must NOT drop the saved query — otherwise
        // the next `open_find_bar` would start blank instead of restoring
        // the previous search.
        terminal.update(&mut app, |view, ctx| {
            view.close_find_bar(ctx);
        });

        assert_eq!(
            terminal.read(&app, |view, ctx| view
                .find_model
                .as_ref(ctx)
                .active_find_options()
                .map(|o| o.query.clone())),
            Some(needle_options().query),
            "closing the find bar must preserve the saved query on the async path"
        );
    })
}

/// Regression test: selecting text inside an AI (rich content) block and copying
/// must place the selected text on the clipboard.
///
/// AI blocks own their text selection independently of the point-based model
/// selection, so the model must be told (via `AIBlockEvent::SelectionChanged`)
/// which rich content block has an active selection. Otherwise
/// `selection_to_string` returns nothing and the copy paths produce an empty
/// clipboard (the regression introduced by the `mouse_down` `if !handled` guard
/// in #12079).
#[test]
fn copy_selected_text_from_ai_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let terminal = add_window_with_terminal(&mut app, None);

        // Insert an AI block with a user query.
        terminal.update(&mut app, |view, ctx| {
            append_exchange_and_handle_event(
                view,
                AIAgentInput::UserQuery {
                    query: "the quick brown fox".to_owned(),
                    context: Default::default(),
                    static_query_type: None,
                    referenced_attachments: Default::default(),
                    user_query_mode: UserQueryMode::Normal,
                    running_command: None,
                    intended_agent: None,
                },
                ctx,
            );
        });

        let ai_block = terminal.read(&app, |view, _| {
            view.rich_content_views
                .iter()
                .find_map(|rich_content| {
                    rich_content
                        .ai_block_metadata()
                        .map(|metadata| metadata.ai_block_handle.clone())
                })
                .expect("an AI block should have been inserted")
        });

        // Simulate a block-level text selection within the AI block and notify the
        // terminal view (mirrors the `SelectableArea` selection callback plus the
        // `AIBlockAction::SelectText` dispatch that happens on a real drag).
        ai_block.update(&mut app, |block, ctx| {
            block.set_block_level_selected_text_for_test(Some("quick brown".to_owned()));
            block.handle_action(&AIBlockAction::SelectText, ctx);
        });

        // The model must now record that the AI block has an active text
        // selection, which is what lets the copy/insert paths (via
        // `selection_to_string`) find the selected text. This is the part the
        // #12079 regression broke. We assert on the model record rather than the
        // clipboard string because reading the selected text cross-view requires
        // an active window, which the headless test harness does not provide (the
        // end-to-end clipboard behavior is covered by manual/computer-use
        // verification).
        terminal.read(&app, |view, ctx| {
            let semantic_selection = SemanticSelection::as_ref(ctx);
            let model = view.model.lock();
            assert!(
                model
                    .block_list()
                    .has_renderable_selection(semantic_selection, false),
                "the model must record the AI block's text selection so copy/insert can find it"
            );
        });

        // Clearing the AI block's selection must clear the tracked model selection
        // so stale text isn't returned by later copy/insert operations.
        ai_block.update(&mut app, |block, ctx| {
            block.set_block_level_selected_text_for_test(None);
            block.handle_action(&AIBlockAction::SelectText, ctx);
        });
        terminal.read(&app, |view, ctx| {
            let semantic_selection = SemanticSelection::as_ref(ctx);
            let model = view.model.lock();
            assert!(
                !model
                    .block_list()
                    .has_renderable_selection(semantic_selection, false),
                "clearing the AI block selection should clear the model's recorded selection"
            );
        });
    })
}

#[test]
fn cmd_k_does_not_clear_buffer_when_agent_is_driving_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            bootstrap_with_long_running_block(view);

            let conversation_id =
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                    history.start_new_conversation(view.view_id, false, false, false, ctx)
                });
            set_active_block_agent_driving(view, conversation_id);

            assert!(
                view.model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_driving_command()
            );
            assert!(
                !view
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_monitoring()
            );

            let block_count_before = view.model.lock().block_list().blocks().len();

            view.clear_buffer_for_testing(ctx);

            assert_eq!(
                view.model.lock().block_list().blocks().len(),
                block_count_before,
                "cmd-k must not wipe blocks while the agent is driving a command"
            );
        });
    })
}

#[test]
fn cmd_k_in_agent_view_clears_active_block_not_full_buffer_when_agent_driving_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let conversation_id = terminal.update(&mut app, |view, ctx| {
            let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("should enter agent view")
            });

            bootstrap_with_long_running_block(view);
            set_active_block_agent_driving(view, conversation_id);

            assert!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .is_fullscreen()
            );
            assert!(
                view.model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_driving_command()
            );

            conversation_id
        });

        let block_count_before = terminal.read(&app, |view, _| {
            view.model.lock().block_list().blocks().len()
        });

        terminal.update(&mut app, |view, ctx| {
            view.clear_buffer_for_testing(ctx);
        });

        terminal.read(&app, |view, ctx| {
            // Same conversation still active: no new conversation was started.
            assert_eq!(
                view.agent_view_controller()
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id(),
                Some(conversation_id),
                "cmd-k must not start a new conversation while agent is driving a command"
            );
            // Block count unchanged: only the active block output was cleared.
            assert_eq!(
                view.model.lock().block_list().blocks().len(),
                block_count_before,
                "cmd-k must not remove blocks while agent is driving a command"
            );
        });
    })
}

#[test]
fn cmd_k_in_agent_view_cancels_in_progress_conversation_and_starts_new_one() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        let old_conversation_id = terminal.update(&mut app, |view, ctx| {
            view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_agent_view(
                        None,
                        AgentViewEntryOrigin::Input {
                            was_prompt_autodetected: false,
                        },
                        ctx,
                    )
                    .expect("should enter agent view")
            })
        });

        // New conversations always start InProgress.
        terminal.read(&app, |_, ctx| {
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&old_conversation_id)
                    .map(|c| c.status().clone()),
                Some(ConversationStatus::InProgress)
            );
        });

        // Attach a mock in-flight response stream so cancel_conversation_progress
        // can actually cancel it and flip the status to Cancelled.
        let stream_id = ResponseStreamId::new_for_test();
        terminal.update(&mut app, |view, ctx| {
            // Associate the stream_id with the conversation in the history model so
            // is_processing_response_stream returns true when the controller looks it up.
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                let exchange = exchange_with_inputs(vec![]);
                history
                    .conversation_mut(&old_conversation_id)
                    .expect("conversation should exist")
                    .append_reassigned_exchange(&stream_id, exchange, view.view_id, ctx)
                    .expect("exchange should append");
            });
            let stream = ctx.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
            view.ai_controller.update(ctx, |controller, ctx| {
                controller.register_mock_stream_for_test(
                    stream_id.clone(),
                    old_conversation_id,
                    stream,
                    ctx,
                );
            });
        });

        // Cmd+K with no long-running command: cancels the old in-progress conversation
        // (stream is cancelled → AfterStreamFinished → Cancelled status) and starts a new one.
        terminal.update(&mut app, |view, ctx| {
            view.clear_buffer_for_testing(ctx);
        });

        terminal.read(&app, |view, ctx| {
            // A new conversation must now be active.
            let new_conversation_id = view
                .agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
                .expect("agent view should still be active after cmd-k");
            assert_ne!(
                new_conversation_id, old_conversation_id,
                "cmd-k must start a new conversation when an in-progress one is active"
            );

            // The old conversation must be Cancelled — the stream was actually cancelled.
            assert_eq!(
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&old_conversation_id)
                    .map(|c| c.status().clone()),
                Some(ConversationStatus::Cancelled),
                "the old in-progress conversation must be Cancelled after cmd-k"
            );
        });
    })
}

#[test]
#[cfg(target_os = "linux")]
fn copy_forwards_etx_to_pty_on_linux_alt_screen_without_warp_selection() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            // Enter the alt screen (a fullscreen TUI is in control) and add no
            // Warp-visible selection of any kind.
            {
                let mut model = view.model.lock();
                model.set_mode(ansi::Mode::SwapScreen {
                    save_cursor_and_clear_screen: true,
                });
                assert!(model.is_alt_screen_active());
            }
            assert_eq!("", &read_from_clipboard(ctx));

            // No CLI-subagent / error-screen / grid / input-editor / block
            // selection exists, so `copy()` reaches the new fallback. The clipboard
            // is written synchronously, but `WriteBytesToPty` events are dispatched
            // after the update closure returns, so the PTY-write assertion is made
            // outside the closure (mirroring `ctrl_c_after_stop_takeover_cancels_conversation`).
            view.handle_action(&TerminalAction::Copy, ctx);

            assert_eq!(
                read_from_clipboard(ctx),
                "",
                "Copy must not write anything to the clipboard when Warp has no selection"
            );
        });

        assert_eq!(
            *pty_writes.borrow(),
            vec![vec![C0::ETX]],
            "Copy on Linux alt screen with no Warp selection must forward exactly one ETX byte to the PTY"
        );
    })
}

#[test]
fn copy_does_not_forward_when_alt_screen_has_warp_selection() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            {
                // Enter the alt screen and add text the user will select in Warp.
                let mut model = view.model.lock();
                model.set_mode(ansi::Mode::SwapScreen {
                    save_cursor_and_clear_screen: true,
                });
                assert!(model.is_alt_screen_active());

                model.alt_screen_mut().input('h');
            }

            // Make a Warp-owned alt-screen selection (the path that copies today).
            view.begin_alt_selection(Point::new(0, 0), Side::Left, SelectionType::Simple, ctx);
            view.update_alt_selection(Point::new(0, 2), Side::Left, &Lines::zero(), ctx);
            view.end_alt_selection(ctx);
            // `end_alt_selection` copies via copy-on-select, so the clipboard now
            // holds the selected text. Reset the PTY-write recorder so the only
            // writes observed below come from the explicit Copy dispatch.
            pty_writes.borrow_mut().clear();
            assert_eq!("h", &read_from_clipboard(ctx));

            view.handle_action(&TerminalAction::Copy, ctx);

            assert_eq!(
                read_from_clipboard(ctx),
                "h",
                "Copy must still copy the Warp alt-screen selection to the clipboard"
            );
        });

        assert!(
            pty_writes.borrow().is_empty(),
            "Copy must not forward ETX to the PTY when a Warp alt-screen selection was copied"
        );
    })
}

#[test]
fn copy_does_not_forward_on_normal_screen() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            // Normal screen: no alt screen, no selection of any kind.
            assert!(!view.model.lock().is_alt_screen_active());
            assert_eq!("", &read_from_clipboard(ctx));

            view.handle_action(&TerminalAction::Copy, ctx);

            assert_eq!(
                read_from_clipboard(ctx),
                "",
                "Copy must not write anything to the clipboard on the normal screen with no selection"
            );
        });

        assert!(
            pty_writes.borrow().is_empty(),
            "Copy must not write anything to the PTY on the normal screen with no selection"
        );
    })
}

/// Builds a minimal review batch with a single non-outdated general comment.
fn single_general_review_comment(content: &str) -> AgentReviewCommentBatch {
    AgentReviewCommentBatch {
        comments: vec![AttachedReviewComment {
            id: Default::default(),
            content: content.to_string(),
            target: AttachedReviewCommentTarget::General,
            last_update_time: Local::now(),
            base: None,
            head: None,
            outdated: false,
            origin: CommentOrigin::Native,
        }],
        diff_set: HashMap::new(),
    }
}

fn set_warp_tui_session(view: &mut TerminalView, ctx: &mut ViewContext<TerminalView>) {
    view.model.lock().simulate_long_running_block("warp", "");
    assert_eq!(
        CLIAgent::detect("warp", None, None, ctx),
        Some(CLIAgent::WarpTui)
    );

    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
        sessions.set_session(
            view.view_id,
            CLIAgentSession {
                agent: CLIAgent::WarpTui,
                status: CLIAgentSessionStatus::InProgress,
                session_context: CLIAgentSessionContext::default(),
                input_state: CLIAgentInputState::Closed,
                should_auto_toggle_input: false,
                listener: None,
                remote_host: None,
                plugin_version: None,
                draft_text: None,
                custom_command_prefix: None,
                received_rich_notification: false,
            },
            ctx,
        );
    });
}

#[test]
fn warp_tui_listener_does_not_auto_open_rich_input() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .auto_open_rich_input_on_cli_agent_start
                .set_value(true, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            view.handle_cli_agent_notification(
                Some(CLI_AGENT_NOTIFICATION_SENTINEL),
                r#"{"v":1,"agent":"warp-tui","event":"session_start"}"#,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            let session = CLIAgentSessionsModel::as_ref(ctx)
                .session(view.view_id)
                .expect("Warp TUI session should be registered");
            assert!(session.listener.is_some());
            assert!(!session.should_auto_toggle_input);
            assert!(!view.has_active_cli_agent_input_session(ctx));
        });
    });
}
#[test]
fn active_cli_agent_recognizes_detected_warp_tui_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _hoa_code_review = FeatureFlag::HoaCodeReview.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            set_warp_tui_session(view, ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.active_cli_agent(ctx),
                Some(CLIAgent::WarpTui),
                "Warp TUI should be recognized as a code-review destination while running"
            );
        });
    });
}

/// `active_cli_agent` must return `None` for the Warp TUI when `HoaCodeReview`
/// is disabled, preserving the pre-feature behavior (no review destination).
#[test]
fn active_cli_agent_ignores_warp_tui_when_hoa_code_review_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _hoa_code_review = FeatureFlag::HoaCodeReview.override_enabled(false);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, ctx| {
            set_warp_tui_session(view, ctx);
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(
                view.active_cli_agent(ctx),
                None,
                "Warp TUI should not be a review destination when HoaCodeReview is disabled"
            );
        });
    });
}

#[test]
fn active_cli_agent_ignores_non_tui_long_running_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _hoa_code_review = FeatureFlag::HoaCodeReview.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |view, _| {
            view.model.lock().simulate_long_running_block("vim", "");
        });

        terminal.read(&app, |view, ctx| {
            assert_eq!(CLIAgent::detect("vim", None, None, ctx), None);
            assert_eq!(
                view.active_cli_agent(ctx),
                None,
                "a non-TUI long-running command must not be a review destination"
            );
        });
    });
}

/// Sending review comments while the Warp TUI is running writes the built prompt
/// directly to the TUI's PTY rather than the outer rich input.
#[test]
fn send_review_comments_to_warp_tui_writes_prompt_to_pty() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _hoa_code_review = FeatureFlag::HoaCodeReview.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        let pty_writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
        let writes = pty_writes.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&terminal, move |_, event, _| {
                if let Event::WriteBytesToPty { bytes } = event {
                    writes.borrow_mut().push(bytes.to_vec());
                }
            });
        });

        terminal.update(&mut app, |view, ctx| {
            set_warp_tui_session(view, ctx);
            assert_eq!(view.active_cli_agent(ctx), Some(CLIAgent::WarpTui));
            assert!(!view.is_cli_agent_rich_input_open(ctx));

            let review = single_general_review_comment("please fix the off-by-one");
            view.send_review_to_cli_agent_or_rich_input(&review, ctx)
                .expect("send should succeed");
        });

        // The review prompt is written to the PTY in a single write because
        // Warp TUI sessions do not open the outer rich input.
        let writes = pty_writes.borrow();
        assert_eq!(
            writes.len(),
            1,
            "expected a single PTY write, got {writes:?}"
        );
        let prompt = std::str::from_utf8(&writes[0]).expect("prompt is valid UTF-8");
        assert!(
            prompt.contains("please fix the off-by-one"),
            "PTY write should contain the review prompt, got: {prompt}"
        );
    });
}

#[test]
fn back_button_label_names_the_direct_parent_at_depth() {
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let (root_id, mid_id, grandchild_id) = history_model.update(&mut app, |history, ctx| {
            let root_id =
                history.start_new_conversation(terminal_view_id, false, false, false, ctx);
            let mid_id = history.start_new_child_conversation(
                terminal_view_id,
                "api-refactor".to_string(),
                root_id,
                None,
                ctx,
            );
            let grandchild_id = history.start_new_child_conversation(
                terminal_view_id,
                "grandchild".to_string(),
                mid_id,
                None,
                ctx,
            );
            (root_id, mid_id, grandchild_id)
        });

        // A nested parent whose agent name is empty falls back to the
        // generic wording instead of rendering "for ".
        let (unnamed_mid_id, nested_under_unnamed_id) =
            history_model.update(&mut app, |history, ctx| {
                let unnamed_mid_id = history.start_new_child_conversation(
                    terminal_view_id,
                    String::new(),
                    root_id,
                    None,
                    ctx,
                );
                let nested_id = history.start_new_child_conversation(
                    terminal_view_id,
                    "nested".to_string(),
                    unnamed_mid_id,
                    None,
                    ctx,
                );
                (unnamed_mid_id, nested_id)
            });

        history_model.read(&app, |history, _| {
            assert_eq!(agent_view_back_button_label(history, None), "for terminal");
            assert_eq!(
                agent_view_back_button_label(history, Some(root_id)),
                "for terminal",
            );
            assert_eq!(
                agent_view_back_button_label(history, Some(mid_id)),
                "for Orchestrator",
            );
            assert_eq!(
                agent_view_back_button_label(history, Some(grandchild_id)),
                "for api-refactor",
            );
            assert_eq!(
                agent_view_back_button_label(history, Some(unnamed_mid_id)),
                "for Orchestrator",
            );
            assert_eq!(
                agent_view_back_button_label(history, Some(nested_under_unnamed_id)),
                "for parent agent",
            );
        });
    });
}

/// A child linked to its parent only via a legacy server conversation token
/// in `parent_agent_id` (no explicit parent conversation id, no run id) must
/// still resolve the parent through the history model's canonical
/// resolution, yielding the same back-button label as an id-linked child.
#[test]
fn back_button_label_resolves_token_only_parent_linkage() {
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_history_persistence_for_tests(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let child_id = history_model.update(&mut app, |history, ctx| {
            let root_id =
                history.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history
                .set_server_conversation_token_for_conversation(root_id, "root-token".to_string());
            let child_id =
                history.start_new_conversation(terminal_view_id, false, false, false, ctx);
            history
                .conversation_mut(&child_id)
                .expect("child conversation exists")
                .set_parent_agent_id("root-token".to_string());
            child_id
        });

        history_model.read(&app, |history, _| {
            assert_eq!(
                agent_view_back_button_label(history, Some(child_id)),
                "for Orchestrator",
            );
        });
    });
}

// ---- OneKey 搜索过滤 pure 函数测试 ----
//
// 这些测试只针对 `filter_and_sort_onekey_candidates` 这条 pure 路径,
// 不需要构造 TerminalView / ctx。skim 算法的 Unicode 处理由 fuzzy_match
// crate 负责,这里只验证我们在 view.rs 中对它的使用是否符合预期。

use super::OnekeyMenuRows;
use super::filter_and_sort_onekey_candidates;

fn rows_indices(rows: OnekeyMenuRows) -> Vec<usize> {
    match rows {
        OnekeyMenuRows::Ordered(v) => v,
        OnekeyMenuRows::NoMatches => panic!("expected Ordered, got NoMatches"),
    }
}

#[test]
fn onekey_empty_query_returns_full_set_in_original_order() {
    let candidates = vec![
        ("prod-db", "deploy@10.0.0.5:22"),
        ("staging-web", "ubuntu@stage.foo.com:22"),
        ("生产数据库", "ops@db.example.com:22"),
    ];
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "");
    assert_eq!(rows_indices(result), vec![0, 1, 2]);
}

#[test]
fn onekey_empty_query_after_trim_returns_full_set() {
    let candidates = vec![("a", "1"), ("b", "2")];
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "   ");
    assert_eq!(rows_indices(result), vec![0, 1]);
}

#[test]
fn onekey_query_filters_out_non_matches() {
    let candidates = vec![("apple", "x"), ("banana", "x"), ("apricot", "x")];
    // "ap" 应该命中 apple / apricot,banana 不命中。
    // 这里不断言 apple / apricot 之间的相对顺序——具体打分由 skim 决定,
    // 我们只保证 banana 被过滤掉。
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "ap");
    let indices = rows_indices(result);
    assert!(indices.contains(&0)); // apple
    assert!(indices.contains(&2)); // apricot
    assert!(!indices.contains(&1)); // banana
}

#[test]
fn onekey_query_matches_subtitle() {
    let candidates = vec![
        ("server-1", "alice@prod.example.com:22"),
        ("server-2", "bob@stage.example.com:22"),
    ];
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "prod");
    let indices = rows_indices(result);
    // 只有 server-1 的 subtitle 含 prod。
    assert_eq!(indices, vec![0]);
}

#[test]
fn onekey_query_no_match_returns_no_matches() {
    let candidates = vec![("a", "1"), ("b", "2")];
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "zzz-not-present");
    assert_eq!(result, OnekeyMenuRows::NoMatches);
}

#[test]
fn onekey_query_matches_chinese_characters() {
    // 中文字符序列匹配:skim 算法按 Unicode char 处理。
    let candidates = vec![
        ("生产数据库", "ops@db.example.com:22"),
        ("测试服务器", "qa@test.example.com:22"),
    ];
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "生产");
    let indices = rows_indices(result);
    assert_eq!(indices, vec![0]);
}

#[test]
fn onekey_query_matches_japanese_characters() {
    let candidates = vec![
        ("本番データベース", "ops@db.example.com:22"),
        ("ステージング", "stage@example.com:22"),
    ];
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "データ");
    let indices = rows_indices(result);
    assert_eq!(indices, vec![0]);
}

#[test]
fn onekey_query_case_insensitive() {
    let candidates = vec![("ProductionDB", "ops@example.com:22")];
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "production");
    assert_eq!(rows_indices(result), vec![0]);
}

#[test]
fn onekey_empty_candidates_with_query_returns_no_matches() {
    let candidates: Vec<(&str, &str)> = vec![];
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "anything");
    assert_eq!(result, OnekeyMenuRows::NoMatches);
}

#[test]
fn onekey_empty_candidates_with_empty_query_returns_empty_ordered() {
    let candidates: Vec<(&str, &str)> = vec![];
    let result = filter_and_sort_onekey_candidates(candidates.iter().copied(), "");
    assert_eq!(rows_indices(result), Vec::<usize>::new());
}
