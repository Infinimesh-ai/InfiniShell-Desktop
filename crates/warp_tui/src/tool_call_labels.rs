//! Per-tool, per-state labels for tool-call rows in the TUI transcript,
//! modeled on the GUI's inline action text.

use std::path::Path;

use ai::agent::action_result::RunAgentsAgentOutcome;
use warp::tui_export::{
    AIActionStatus, AIAgentAction, AIAgentActionResultType, AIAgentActionType,
    AskUserQuestionResult, FileGlobV2Result, GrepResult, RequestCommandOutputResult,
    RunAgentsAgentOutcomeKind, RunAgentsResult, SuggestNewConversationResult,
    mcp_server_name_for_id,
};
use warp_core::command::ExitCode;
use warpui_core::AppContext;
use warpui_core::elements::tui::{Modifier, TuiStyle};

use self::ToolCallDisplayState as State;
use crate::tui_builder::TuiUiBuilder;

/// Ground-truth state of the terminal block backing a shell-command tool
/// call, resolved by the caller. When a block exists, its state supersedes
/// the stored action status/result for execution states (mirroring the GUI's
/// `RequestedCommandView`, which derives icon and expandability from the
/// block whenever one exists). Notably, an agent-monitored command's stored
/// result stays a `LongRunningCommandSnapshot` forever, so without the block
/// its row could never leave the "still running" state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandBlockState {
    Running,
    Finished { exit_code: ExitCode },
}

/// A shell-command tool call's terminal block as resolved by the caller: its
/// execution state plus the command it actually ran. The block's command
/// supersedes the streamed one, which the user may have edited before
/// accepting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedCommandBlock {
    /// The block's command, when it has one; `None` while the block's
    /// command grid is still empty.
    pub(crate) command: Option<String>,
    pub(crate) state: CommandBlockState,
}

/// Longest rendered length for compact interpolated values such as queries and
/// paths. Shell commands are preserved in full and wrap in their collapsible
/// header instead.
const MAX_INLINE_LEN: usize = 80;

/// Coarse presentation state for a tool call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolCallDisplayState {
    /// The tool call's arguments are still streaming and may be incomplete.
    Constructing,
    /// The tool call is waiting to begin execution.
    Pending,
    /// The tool call is blocked on user confirmation.
    Blocked,
    /// The tool call is executing asynchronously.
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl ToolCallDisplayState {
    /// The compact leading glyph for this state.
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::Constructing | Self::Pending => "○",
            Self::Blocked | Self::Cancelled => "■",
            Self::Running => "●",
            Self::Succeeded => "✓",
            Self::Failed => "×",
        }
    }

    /// The semantic theme style for this state's glyph.
    pub(crate) fn glyph_style(self, builder: &TuiUiBuilder) -> TuiStyle {
        match self {
            Self::Constructing | Self::Pending => builder.dim_text_style(),
            Self::Blocked | Self::Running => builder.attention_glyph_style(),
            Self::Succeeded => builder.success_glyph_style(),
            Self::Failed => builder.error_text_style(),
            Self::Cancelled => builder.muted_text_style(),
        }
    }

    /// The semantic text style paired with this state.
    pub(crate) fn label_style(self, builder: &TuiUiBuilder) -> TuiStyle {
        match self {
            Self::Constructing | Self::Pending => builder.dim_text_style(),
            Self::Blocked | Self::Running | Self::Succeeded | Self::Failed | Self::Cancelled => {
                builder.primary_text_style()
            }
        }
    }
}

/// Styles the first word of a tool-call label as the action and the rest as details.
pub(crate) fn styled_tool_call_label_spans(
    label: &str,
    builder: &TuiUiBuilder,
) -> Vec<(String, TuiStyle)> {
    let action_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
    let details_style = builder.neutral_7_text_style();
    match label.find(char::is_whitespace) {
        Some(first_word_end) => vec![
            (label[..first_word_end].to_owned(), action_style),
            (label[first_word_end..].to_owned(), details_style),
        ],
        None => vec![(label.to_owned(), action_style)],
    }
}

/// Collapses an optional action status into the coarse display state.
/// `output_streaming` is whether the exchange output is still streaming;
/// a status-less action in a streaming output is still being constructed
/// (mirroring the GUI's `status.is_none() && is_streaming()` gating).
/// A resolved `block_state` supersedes the status for execution states
/// (see [`CommandBlockState`]).
pub(crate) fn tool_call_display_state(
    status: Option<&AIActionStatus>,
    output_streaming: bool,
    block_state: Option<CommandBlockState>,
) -> ToolCallDisplayState {
    // A block existing means the command actually started executing, so its
    // state is authoritative over the action status/result.
    match block_state {
        Some(CommandBlockState::Running) => return State::Running,
        Some(CommandBlockState::Finished { exit_code }) => {
            return if exit_code.is_sigint() {
                State::Cancelled
            } else if exit_code.was_successful() {
                State::Succeeded
            } else {
                State::Failed
            };
        }
        None => {}
    }
    match status {
        None if output_streaming => State::Constructing,
        None | Some(AIActionStatus::Preprocessing | AIActionStatus::Queued) => State::Pending,
        Some(AIActionStatus::Blocked) => State::Blocked,
        Some(AIActionStatus::RunningAsync) => State::Running,
        Some(finished @ AIActionStatus::Finished(_)) => {
            if finished.is_cancelled() {
                State::Cancelled
            } else if finished.is_failed() {
                State::Failed
            } else {
                State::Succeeded
            }
        }
    }
}

/// Returns the transcript label for a tool call in its current state.
///
/// Equivalent to [`tool_call_label_with_server`] with no MCP server name; use
/// that variant when rendering an MCP tool call whose originating server is
/// known so the label surfaces both the tool name and the server.
pub(crate) fn tool_call_label(
    action: &AIAgentAction,
    status: Option<&AIActionStatus>,
    output_streaming: bool,
    block: Option<&ResolvedCommandBlock>,
) -> String {
    tool_call_label_with_server(action, status, output_streaming, block, None)
}

/// Like [`tool_call_label`], but interpolates the MCP tool's originating server
/// name (when known) into the per-state label so MCP tool calls surface both
/// their tool name and server identity across the transcript lifecycle.
pub(crate) fn tool_call_label_with_server(
    action: &AIAgentAction,
    status: Option<&AIActionStatus>,
    output_streaming: bool,
    block: Option<&ResolvedCommandBlock>,
    server_name: Option<&str>,
) -> String {
    let state = tool_call_display_state(status, output_streaming, block.map(|block| block.state));
    let result = status
        .and_then(AIActionStatus::finished_result)
        .map(|result| &result.result);
    let label = label_for_action(&action.action, state, result, block, server_name);
    match state {
        State::Blocked => warp::t!("tui-tool-awaiting-approval", label = label),
        State::Constructing
        | State::Pending
        | State::Running
        | State::Succeeded
        | State::Failed
        | State::Cancelled => label,
    }
}

/// Resolves the user-facing name of the originating MCP server for an MCP
/// tool-call action, for use in transcript labels. Returns `None` for non-
/// MCP-tool actions, legacy/flat calls with no server id, or unknown servers.
pub(crate) fn mcp_server_name_for_action(
    action: &AIAgentActionType,
    app: &AppContext,
) -> Option<String> {
    match action {
        AIAgentActionType::CallMCPTool { server_id, .. } => server_id
            .as_ref()
            .and_then(|id| mcp_server_name_for_id(id, app)),
        _ => None,
    }
}

/// Builds the per-tool label body; the awaiting-approval suffix is applied by
/// [`tool_call_label`]. `result` is the finished result, when there is one.
///
/// `Constructing` arms never interpolate argument fields (they may be empty
/// or partial while streaming); their copy is indexed on the GUI's loading
/// messages (`common.rs` `LOAD_OUTPUT_MESSAGE_*` and the requested-command
/// view's "Generating command...").
fn label_for_action(
    action: &AIAgentActionType,
    state: State,
    result: Option<&AIAgentActionResultType>,
    block: Option<&ResolvedCommandBlock>,
    server_name: Option<&str>,
) -> String {
    let block_state = block.map(|block| block.state);
    match action {
        AIAgentActionType::RequestCommandOutput { command, .. } => {
            // The streamed command can be edited before acceptance, so
            // prefer the executed command from the finished result or the
            // resolved block over the original suggestion.
            let executed = result
                .and_then(AIAgentActionResultType::command_str)
                .or_else(|| block.and_then(|block| block.command.as_deref()));
            // Shell-command headers wrap in `TuiShellCommandView`, so retain
            // the complete command instead of capping it at MAX_INLINE_LEN.
            let cmd = executed.unwrap_or(command).trim_end();
            match state {
                State::Constructing => warp::t!("tui-tool-command-generating"),
                State::Pending | State::Blocked => warp::t!("tui-tool-command-run", command = cmd),
                State::Running => warp::t!("tui-tool-command-running", command = cmd),
                State::Succeeded => match block_state {
                    Some(CommandBlockState::Finished { .. }) => {
                        warp::t!("tui-tool-command-ran", command = cmd)
                    }
                    // No local block: fall back to the stored result. A
                    // snapshot result means the command was still running at
                    // the last point we could observe it.
                    Some(CommandBlockState::Running) | None => match result {
                        Some(AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::LongRunningCommandSnapshot { .. },
                        )) => warp::t!("tui-tool-command-still-running", command = cmd),
                        _ => warp::t!("tui-tool-command-ran", command = cmd),
                    },
                },
                State::Failed => match block_state {
                    Some(CommandBlockState::Finished { exit_code }) => {
                        warp::t!(
                            "tui-tool-command-exited",
                            command = cmd,
                            code = exit_code.value()
                        )
                    }
                    Some(CommandBlockState::Running) | None => match result {
                        Some(AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::Completed { exit_code, .. },
                        )) => warp::t!(
                            "tui-tool-command-exited",
                            command = cmd,
                            code = exit_code.value()
                        ),
                        Some(AIAgentActionResultType::RequestCommandOutput(
                            RequestCommandOutputResult::Denylisted { .. },
                        )) => warp::t!("tui-tool-command-denied", command = cmd),
                        _ => warp::t!("tui-tool-command-failed", command = cmd),
                    },
                },
                State::Cancelled => warp::t!("tui-tool-command-cancelled", command = cmd),
            }
        }
        AIAgentActionType::WriteToLongRunningShellCommand { .. } => match state {
            State::Constructing => warp::t!("tui-tool-command-input-writing"),
            State::Pending | State::Blocked => warp::t!("tui-tool-command-input-write"),
            State::Running => warp::t!("tui-tool-command-input-writing-running"),
            State::Succeeded => warp::t!("tui-tool-command-input-wrote"),
            State::Failed => warp::t!("tui-tool-command-input-failed"),
            State::Cancelled => warp::t!("tui-tool-command-input-cancelled"),
        },
        AIAgentActionType::ReadFiles(request) => {
            let files = files_summary(request.locations.iter().map(|location| &location.name));
            match state {
                State::Constructing => warp::t!("tui-tool-files-reading"),
                State::Pending | State::Blocked | State::Succeeded => {
                    warp::t!("tui-tool-files-read", files = files)
                }
                State::Running => warp::t!("tui-tool-files-reading-named", files = files),
                State::Failed => warp::t!("tui-tool-files-failed", files = files),
                State::Cancelled => warp::t!("tui-tool-files-cancelled", files = files),
            }
        }
        // Rendered by its own stateful child view (`TuiFileEditsView`); the
        // label path should never be reached for it.
        AIAgentActionType::RequestFileEdits { .. } => {
            log::warn!("tool_call_label called for RequestFileEdits, which has custom rendering");
            String::new()
        }
        AIAgentActionType::Grep { queries, path } => {
            let queries = single_line(&queries.join(", "));
            let path = display_path(path);
            match state {
                State::Constructing => warp::t!("tui-tool-grep-starting"),
                State::Pending | State::Blocked => {
                    warp::t!("tui-tool-grep", queries = queries, path = path)
                }
                State::Running => warp::t!("tui-tool-grep-running", queries = queries, path = path),
                State::Succeeded => match result {
                    Some(AIAgentActionResultType::Grep(GrepResult::Success { matched_files })) => {
                        warp::t!(
                            "tui-tool-grep-succeeded-with-count",
                            queries = queries,
                            path = path,
                            count = matched_files.len()
                        )
                    }
                    _ => warp::t!("tui-tool-grep-succeeded", queries = queries, path = path),
                },
                State::Failed => warp::t!("tui-tool-grep-failed", queries = queries),
                State::Cancelled => warp::t!("tui-tool-grep-cancelled", queries = queries),
            }
        }
        AIAgentActionType::FileGlob { patterns, path } => {
            file_glob_label(patterns, path.as_deref(), state, None)
        }
        AIAgentActionType::FileGlobV2 {
            patterns,
            search_dir,
        } => {
            let matched_count = match result {
                Some(AIAgentActionResultType::FileGlobV2(FileGlobV2Result::Success {
                    matched_files,
                    ..
                })) => Some(matched_files.len()),
                _ => None,
            };
            file_glob_label(patterns, search_dir.as_deref(), state, matched_count)
        }
        AIAgentActionType::ReadMCPResource { name, uri, .. } => {
            let resource = single_line(uri.as_deref().unwrap_or(name));
            match state {
                // The resource name arrives with the tool-call header (not
                // the streamed args), so include it when present, like the
                // GUI's "Reading \"{name}\" MCP resource..." loading text.
                State::Constructing if name.is_empty() => warp::t!("tui-tool-mcp-resource-reading"),
                State::Constructing => warp::t!("tui-tool-mcp-resource-reading-name", name = name),
                State::Pending | State::Blocked | State::Succeeded => {
                    warp::t!("tui-tool-mcp-resource-read", resource = resource)
                }
                State::Running => {
                    warp::t!("tui-tool-mcp-resource-reading-uri", resource = resource)
                }
                State::Failed => warp::t!("tui-tool-mcp-resource-failed", resource = resource),
                State::Cancelled => {
                    warp::t!("tui-tool-mcp-resource-cancelled", resource = resource)
                }
            }
        }
        AIAgentActionType::CallMCPTool { name, .. } => {
            let name = single_line(name);
            // Append the originating server when known so MCP tool calls
            // surface both identities, with a deterministic no-server fallback.
            let suffix = server_name
                .map(|server| warp::t!("tui-tool-mcp-server-suffix", server = server))
                .unwrap_or_default();
            match state {
                // Like the GUI's "Calling \"{name}\" MCP tool..." loading
                // text; the tool name is available before its args finish.
                State::Constructing if name.is_empty() => {
                    warp::t!("tui-tool-mcp-calling", suffix = suffix)
                }
                State::Constructing => {
                    warp::t!("tui-tool-mcp-calling-name", name = name, suffix = suffix)
                }
                State::Pending | State::Blocked => {
                    warp::t!("tui-tool-mcp-call", name = name, suffix = suffix)
                }
                State::Running => warp::t!(
                    "tui-tool-mcp-calling-name-plain",
                    name = name,
                    suffix = suffix
                ),
                State::Succeeded => warp::t!("tui-tool-mcp-called", name = name, suffix = suffix),
                State::Failed => warp::t!("tui-tool-mcp-failed", name = name, suffix = suffix),
                State::Cancelled => {
                    warp::t!("tui-tool-mcp-cancelled", name = name, suffix = suffix)
                }
            }
        }
        AIAgentActionType::SuggestNewConversation { .. } => match state {
            State::Constructing => warp::t!("tui-tool-new-conversation-suggesting"),
            State::Pending | State::Blocked | State::Running | State::Failed => {
                warp::t!("tui-tool-new-conversation-suggested")
            }
            State::Succeeded => match result {
                Some(AIAgentActionResultType::SuggestNewConversation(
                    SuggestNewConversationResult::Rejected,
                )) => warp::t!("tui-tool-current-conversation-continuing"),
                _ => warp::t!("tui-tool-new-conversation-started"),
            },
            State::Cancelled => warp::t!("tui-tool-new-conversation-cancelled"),
        },
        AIAgentActionType::SuggestPrompt(_) | AIAgentActionType::OpenCodeReview => {
            fallback_label(action, state)
        }
        AIAgentActionType::ReadDocuments(request) => {
            let documents = count_documents(request.document_ids.len());
            match state {
                State::Constructing => warp::t!("tui-tool-documents-reading"),
                State::Pending | State::Blocked | State::Succeeded => {
                    warp::t!("tui-tool-documents-read", documents = documents)
                }
                State::Running => {
                    warp::t!("tui-tool-documents-reading-count", documents = documents)
                }
                State::Failed => warp::t!("tui-tool-documents-failed"),
                State::Cancelled => warp::t!("tui-tool-documents-cancelled"),
            }
        }
        AIAgentActionType::EditDocuments(request) => match state {
            State::Pending | State::Blocked => warp::t!("tui-tool-plan-update"),
            State::Constructing | State::Running => warp::t!("tui-tool-plan-updating"),
            State::Succeeded => warp::t!("tui-tool-plan-updated", count = request.diffs.len()),
            State::Failed => warp::t!("tui-tool-plan-update-failed"),
            State::Cancelled => warp::t!("tui-tool-plan-update-cancelled"),
        },
        AIAgentActionType::CreateDocuments(request) => match state {
            State::Pending | State::Blocked => warp::t!("tui-tool-plan-create"),
            State::Constructing | State::Running => warp::t!("tui-tool-plan-generating"),
            State::Succeeded => {
                let count = request.documents.len();
                if count > 1 {
                    warp::t!("tui-tool-documents-created", count = count)
                } else {
                    warp::t!("tui-tool-plan-created")
                }
            }
            State::Failed => warp::t!("tui-tool-plan-create-failed"),
            State::Cancelled => warp::t!("tui-tool-plan-create-cancelled"),
        },
        AIAgentActionType::ReadShellCommandOutput { .. } => match state {
            State::Pending | State::Blocked | State::Succeeded => {
                warp::t!("tui-tool-command-output-read")
            }
            State::Constructing | State::Running => warp::t!("tui-tool-command-output-reading"),
            State::Failed => warp::t!("tui-tool-command-output-failed"),
            State::Cancelled => warp::t!("tui-tool-command-output-cancelled"),
        },
        AIAgentActionType::InsertCodeReviewComments { comments, .. } => {
            let comments = count_review_comments(comments.len());
            match state {
                State::Constructing => warp::t!("tui-tool-review-comments-preparing"),
                State::Pending | State::Blocked => {
                    warp::t!("tui-tool-review-comments-insert", comments = comments)
                }
                State::Running => {
                    warp::t!("tui-tool-review-comments-inserting", comments = comments)
                }
                State::Succeeded => {
                    warp::t!("tui-tool-review-comments-inserted", comments = comments)
                }
                State::Failed => warp::t!("tui-tool-review-comments-failed"),
                State::Cancelled => warp::t!("tui-tool-review-comments-cancelled"),
            }
        }
        AIAgentActionType::ReadSkill(request) => {
            let skill = single_line(&request.skill.display_label());
            match state {
                State::Constructing => warp::t!("tui-tool-skill-reading"),
                State::Pending | State::Blocked | State::Succeeded => {
                    warp::t!("tui-tool-skill-read", skill = skill)
                }
                State::Running => warp::t!("tui-tool-skill-reading-name", skill = skill),
                State::Failed => warp::t!("tui-tool-skill-failed", skill = skill),
                State::Cancelled => warp::t!("tui-tool-skill-cancelled", skill = skill),
            }
        }
        AIAgentActionType::TransferShellCommandControlToUser { reason } => match state {
            State::Constructing => warp::t!("tui-tool-control-transferring"),
            State::Pending | State::Blocked | State::Running => {
                warp::t!(
                    "tui-tool-control-transferring-reason",
                    reason = single_line(reason)
                )
            }
            State::Succeeded => warp::t!("tui-tool-control-transferred"),
            State::Failed => warp::t!("tui-tool-control-transfer-failed"),
            State::Cancelled => warp::t!("tui-tool-control-transfer-cancelled"),
        },
        AIAgentActionType::AskUserQuestion { questions } => match state {
            State::Constructing => warp::t!("tui-tool-question-preparing"),
            State::Pending | State::Blocked | State::Running => {
                warp::t!("tui-tool-questions-asking", count = questions.len())
            }
            State::Succeeded => match result {
                Some(AIAgentActionResultType::AskUserQuestion(
                    AskUserQuestionResult::Success { answers },
                )) => {
                    let total = answers.len();
                    let answered = answers.iter().filter(|answer| !answer.is_skipped()).count();
                    if answered == 0 {
                        warp::t!("tui-questions-skipped")
                    } else if answered == total && total == 1 {
                        warp::t!("tui-answered-question")
                    } else if answered == total {
                        warp::t!("tui-answered-all-questions", total = total)
                    } else {
                        warp::t!(
                            "tui-answered-some-questions",
                            answered = answered,
                            total = total
                        )
                    }
                }
                Some(AIAgentActionResultType::AskUserQuestion(
                    AskUserQuestionResult::SkippedByAutoApprove { .. },
                )) => warp::t!("tui-questions-skipped"),
                _ => warp::t!("tui-answered-questions"),
            },
            State::Failed => warp::t!("tui-tool-questions-failed"),
            State::Cancelled => warp::t!("tui-tool-questions-cancelled"),
        },
        AIAgentActionType::RunAgents(request) => {
            let total = request.agent_run_configs.len();
            match state {
                State::Constructing | State::Pending | State::Blocked => {
                    warp::t!("tui-tool-agents-configuring")
                }
                State::Running => {
                    warp::t!("tui-tool-agents-spawning", count = total)
                }
                State::Succeeded => match result {
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
                        agents,
                        ..
                    })) => launched_agents_label(agents),
                    _ => warp::t!("tui-tool-agents-spawned", count = total),
                },
                State::Failed => match result {
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Launched {
                        agents,
                        ..
                    })) => launched_agents_label(agents),
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Denied {
                        ..
                    })) => warp::t!("tui-tool-orchestration-disabled"),
                    Some(AIAgentActionResultType::RunAgents(RunAgentsResult::Failure {
                        error,
                    })) if !error.is_empty() => {
                        warp::t!(
                            "tui-tool-orchestration-failed-error",
                            error = single_line(error)
                        )
                    }
                    _ => warp::t!("tui-tool-orchestration-failed"),
                },
                State::Cancelled => warp::t!("tui-tool-agents-cancelled"),
            }
        }
        AIAgentActionType::WaitForEvents { .. } => match state {
            State::Constructing | State::Pending | State::Blocked | State::Running => {
                warp::t!("tui-tool-events-waiting")
            }
            State::Succeeded => warp::t!("tui-tool-events-done"),
            State::Failed => warp::t!("tui-tool-events-failed"),
            State::Cancelled => warp::t!("tui-tool-events-cancelled"),
        },
    }
}

fn launched_agents_label(agents: &[RunAgentsAgentOutcome]) -> String {
    let launched = agents
        .iter()
        .filter(|agent| matches!(agent.kind, RunAgentsAgentOutcomeKind::Launched { .. }))
        .count();
    let total = agents.len();
    if launched == total {
        warp::t!("tui-tool-agents-spawned", count = total)
    } else if launched == 0 {
        warp::t!("tui-tool-agents-spawn-failed", count = total)
    } else {
        warp::t!(
            "tui-tool-agents-spawned-some",
            launched = launched,
            total = total
        )
    }
}
/// Shared label body for both file-glob action versions; only V2 results
/// carry a match count.
fn file_glob_label(
    patterns: &[String],
    path: Option<&str>,
    state: State,
    matched_count: Option<usize>,
) -> String {
    let patterns = single_line(&patterns.join(", "));
    let path = display_path(path.unwrap_or("."));
    match state {
        State::Constructing => warp::t!("tui-tool-files-finding"),
        State::Pending | State::Blocked => {
            warp::t!("tui-tool-files-find", patterns = patterns, path = path)
        }
        State::Running => warp::t!(
            "tui-tool-files-finding-pattern",
            patterns = patterns,
            path = path
        ),
        State::Succeeded => match matched_count {
            Some(count) => warp::t!(
                "tui-tool-files-found-count",
                count = count,
                patterns = patterns
            ),
            None => warp::t!("tui-tool-files-found", patterns = patterns),
        },
        State::Failed => warp::t!("tui-tool-files-search-failed", patterns = patterns),
        State::Cancelled => warp::t!("tui-tool-files-search-cancelled", patterns = patterns),
    }
}

/// Generic label for action types without bespoke text, derived from the
/// action's user-friendly name.
fn fallback_label(action: &AIAgentActionType, state: State) -> String {
    let name = action.user_friendly_name();
    match state {
        State::Pending | State::Blocked => name,
        State::Constructing | State::Running => warp::t!("tui-tool-generic-running", name = name),
        State::Succeeded => warp::t!("tui-tool-generic-done", name = name),
        State::Failed => warp::t!("tui-tool-generic-failed", name = name),
        State::Cancelled => warp::t!("tui-tool-generic-cancelled", name = name),
    }
}

/// Collapses text to its first line, capped at [`MAX_INLINE_LEN`] chars, with
/// a trailing `…` when anything was trimmed.
fn single_line(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default().trim_end();
    let mut out: String = first_line.chars().take(MAX_INLINE_LEN).collect();
    if first_line.chars().count() > MAX_INLINE_LEN || text.lines().count() > 1 {
        out.push('…');
    }
    out
}

/// Renders a search path for display, mirroring the GUI's treatment of `.`.
fn display_path(path: &str) -> String {
    if path == "." {
        warp::t!("tui-current-directory")
    } else {
        single_line(path)
    }
}

/// Returns the final path component, falling back to the input when there is none.
fn base_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Summarizes file paths as comma-joined base names for up to 3 files, else a count.
fn files_summary<'a>(paths: impl ExactSizeIterator<Item = &'a String>) -> String {
    if paths.len() > 3 {
        return count_files(paths.len());
    }
    let names: Vec<String> = paths.map(|path| base_name(path)).collect();
    if names.is_empty() {
        warp::t!("tui-files")
    } else {
        names.join(", ")
    }
}

fn count_files(count: usize) -> String {
    warp::t!("tui-count-files", count = count)
}

fn count_documents(count: usize) -> String {
    warp::t!("tui-count-documents", count = count)
}

fn count_review_comments(count: usize) -> String {
    warp::t!("tui-count-review-comments", count = count)
}

#[cfg(test)]
#[path = "tool_call_labels_tests.rs"]
mod tests;
