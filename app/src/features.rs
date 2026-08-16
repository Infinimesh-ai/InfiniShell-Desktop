use std::collections::HashSet;

use warp_core::channel::ChannelState;
pub use warp_core::features::*;

/// Mark all features which should be enabled on the current channel as enabled.
/// This sets global feature flag state and should never be called in a unit test.
pub fn init_feature_flags() {
    for flag in enabled_features() {
        flag.set_enabled(true);
    }
    mark_initialized();
}

/// Returns all feature flags which should be enabled in the current channel.
fn enabled_features() -> HashSet<FeatureFlag> {
    // Enable features overridden for the given channel.
    let mut flags = ChannelState::additional_features();

    // Enable flags for release builds, if appropriate.
    if ChannelState::is_release_bundle() {
        flags.extend(RELEASE_FLAGS);
    }

    // SSH remote-server:release bundle 走 RELEASE_FLAGS 启用,但 dev 源码构建
    // (`cargo run`)不是 release bundle,该 flag 会一直关闭 —— 于是 SSH 会话
    // 永远退回 legacy 路径,remote-server transport 不激活,dev 模式自动构建并
    // 上传二进制(见 ssh_transport.rs)也就没有机会触发。这里在 debug 构建里
    // 显式开启,保证所有本地平台都能联调远端文件打开 / buffer-sync。
    #[cfg(debug_assertions)]
    flags.insert(FeatureFlag::SshRemoteServer);
    #[cfg(debug_assertions)]
    flags.insert(FeatureFlag::RecursiveSshExtension);
    #[cfg(debug_assertions)]
    flags.insert(FeatureFlag::ServerFileBrowser);

    // Issue #72: HTTP 代理设置页面。不走 channel 判断,所有 channel 含 infinishell
    // 默认启用,作为企业 VPN / 公司代理场景的基本能力。
    flags.insert(FeatureFlag::HttpProxySettings);

    flags.extend([
        #[cfg(feature = "autoupdate")]
        FeatureFlag::Autoupdate,
        #[cfg(feature = "changelog")]
        FeatureFlag::Changelog,
        #[cfg(feature = "crash_reporting")]
        FeatureFlag::CrashReporting,
        #[cfg(feature = "record_app_active_events")]
        FeatureFlag::RecordAppActiveEvents,
        #[cfg(feature = "runtime_feature_flags")]
        FeatureFlag::RuntimeFeatureFlags,
        #[cfg(feature = "sequential_storage")]
        FeatureFlag::SequentialStorage,
        #[cfg(feature = "in_band_generators_ssh")]
        FeatureFlag::InBandGeneratorsForSSH,
        #[cfg(feature = "run_generators_with_cmd_exe")]
        FeatureFlag::RunGeneratorsWithCmdExe,
        #[cfg(feature = "ligatures")]
        FeatureFlag::Ligatures,
        #[cfg(feature = "selectable_prompt")]
        FeatureFlag::SelectablePrompt,
        #[cfg(feature = "agent_mode")]
        FeatureFlag::AgentMode,
        #[cfg(feature = "shared_session_long_running_commands")]
        FeatureFlag::SharedSessionWriteToLongRunningCommands,
        #[cfg(feature = "resize_fix")]
        FeatureFlag::ResizeFix,
        #[cfg(feature = "richtext_multiselect")]
        FeatureFlag::RichTextMultiselect,
        #[cfg(feature = "default_waterfall_mode")]
        FeatureFlag::DefaultWaterfallMode,
        #[cfg(feature = "settings_file")]
        FeatureFlag::SettingsFile,
        #[cfg(feature = "file_backed_execution_profiles")]
        FeatureFlag::FileBackedExecutionProfiles,
        #[cfg(feature = "rect_selection")]
        FeatureFlag::RectSelection,
        #[cfg(feature = "russh_transport")]
        FeatureFlag::RusshTransport,
        #[cfg(feature = "alacritty_settings_import")]
        FeatureFlag::AlacrittySettingsImport,
        #[cfg(feature = "dynamic_workflow_enums")]
        FeatureFlag::DynamicWorkflowEnums,
        #[cfg(feature = "shared_with_me")]
        FeatureFlag::SharedWithMe,
        #[cfg(feature = "am_workflows")]
        FeatureFlag::AgentModeWorkflows,
        #[cfg(feature = "ai_rules")]
        FeatureFlag::AIRules,
        #[cfg(feature = "shell_selector")]
        FeatureFlag::ShellSelector,
        #[cfg(feature = "cloud_runners")]
        FeatureFlag::CloudRunners,
        #[cfg(feature = "cloud_agent_runners")]
        FeatureFlag::CloudAgentRunners,
        #[cfg(feature = "account_first_onboarding")]
        FeatureFlag::AccountFirstOnboarding,
        #[cfg(all(feature = "simulate_github_unauthed", debug_assertions))]
        FeatureFlag::SimulateGithubUnauthed,
        #[cfg(feature = "session_sharing_acls")]
        FeatureFlag::SessionSharingAcls,
        #[cfg(feature = "full_screen_zen_mode")]
        FeatureFlag::FullScreenZenMode,
        #[cfg(feature = "minimalist_ui")]
        FeatureFlag::MinimalistUI,
        #[cfg(feature = "avatar_in_tab_bar")]
        FeatureFlag::AvatarInTabBar,
        #[cfg(feature = "workflow_aliases")]
        FeatureFlag::WorkflowAliases,
        #[cfg(feature = "ssh_drag_and_drop")]
        FeatureFlag::SshDragAndDrop,
        #[cfg(feature = "drag_tabs_to_windows")]
        FeatureFlag::DragTabsToWindows,
        #[cfg(feature = "cycle_next_command_suggestion")]
        FeatureFlag::CycleNextCommandSuggestion,
        #[cfg(feature = "multi_workspace")]
        FeatureFlag::MultiWorkspace,
        #[cfg(feature = "ime_marked_text")]
        FeatureFlag::ImeMarkedText,
        #[cfg(feature = "partial_next_command_suggestions")]
        FeatureFlag::PartialNextCommandSuggestions,
        #[cfg(feature = "iterm_images")]
        FeatureFlag::ITermImages,
        #[cfg(feature = "validate_autosuggestions")]
        FeatureFlag::ValidateAutosuggestions,
        #[cfg(feature = "prompt_suggestions_via_maa")]
        FeatureFlag::PromptSuggestionsViaMAA,
        #[cfg(feature = "clear_autosuggestion_on_escape")]
        FeatureFlag::ClearAutosuggestionOnEscape,
        #[cfg(feature = "autoupdate_ui_revamp")]
        FeatureFlag::AutoupdateUIRevamp,
        #[cfg(all(not(windows), feature = "kitty_images"))]
        FeatureFlag::KittyImages,
        #[cfg(feature = "warp_packs")]
        FeatureFlag::WarpPacks,
        #[cfg(feature = "default_adeberry_theme")]
        FeatureFlag::DefaultAdeberryTheme,
        #[cfg(feature = "agent_mode_primary_xml")]
        FeatureFlag::AgentModePrimaryXML,
        #[cfg(feature = "agent_mode_pre_plan_xml")]
        FeatureFlag::AgentModePrePlanXML,
        #[cfg(feature = "agent_onboarding")]
        FeatureFlag::AgentOnboarding,
        #[cfg(feature = "agent_shared_sessions")]
        FeatureFlag::AgentSharedSessions,
        #[cfg(feature = "suggested_rules")]
        FeatureFlag::SuggestedRules,
        #[cfg(feature = "suggested_agent_mode_workflows")]
        FeatureFlag::SuggestedAgentModeWorkflows,
        #[cfg(feature = "command_correction_key")]
        FeatureFlag::CommandCorrectionKey,
        #[cfg(feature = "predict_am_queries")]
        FeatureFlag::PredictAMQueries,
        #[cfg(feature = "full_source_code_embedding")]
        FeatureFlag::FullSourceCodeEmbedding,
        #[cfg(feature = "remote_codebase_indexing")]
        FeatureFlag::RemoteCodebaseIndexing,
        #[cfg(feature = "use_tantivy_search")]
        FeatureFlag::UseTantivySearch,
        #[cfg(feature = "grep_tool")]
        FeatureFlag::GrepTool,
        #[cfg(feature = "mcp_server")]
        FeatureFlag::McpServer,
        #[cfg(feature = "mcp_debugging_ids")]
        FeatureFlag::McpDebuggingIds,
        #[cfg(feature = "markdown_tables")]
        FeatureFlag::MarkdownTables,
        #[cfg(feature = "jupyter_notebook_rendering")]
        FeatureFlag::JupyterNotebookRendering,
        #[cfg(feature = "blocklist_markdown_table_rendering")]
        FeatureFlag::BlocklistMarkdownTableRendering,
        #[cfg(feature = "blocklist_markdown_images")]
        FeatureFlag::BlocklistMarkdownImages,
        #[cfg(feature = "markdown_mermaid")]
        FeatureFlag::MarkdownMermaid,
        #[cfg(feature = "editable_markdown_mermaid")]
        FeatureFlag::EditableMarkdownMermaid,
        #[cfg(feature = "image_as_context")]
        FeatureFlag::ImageAsContext,
        #[cfg(feature = "msys2_shells")]
        FeatureFlag::MSYS2Shells,
        #[cfg(feature = "file_retrieval_tools")]
        FeatureFlag::FileRetrievalTools,
        #[cfg(feature = "reload_stale_conversation_files")]
        FeatureFlag::ReloadStaleConversationFiles,
        #[cfg(feature = "retry_truncated_code_responses")]
        FeatureFlag::RetryTruncatedCodeResponses,
        #[cfg(feature = "read_image_files")]
        FeatureFlag::ReadImageFiles,
        #[cfg(feature = "ai_context_menu")]
        FeatureFlag::AIContextMenuEnabled,
        #[cfg(feature = "at_menu_outside_of_ai_mode")]
        FeatureFlag::AtMenuOutsideOfAIMode,
        #[cfg(feature = "ai_resume_button")]
        FeatureFlag::AIResumeButton,
        #[cfg(feature = "figma_detection")]
        FeatureFlag::FigmaDetection,
        #[cfg(feature = "agent_decides_command_execution")]
        FeatureFlag::AgentDecidesCommandExecution,
        #[cfg(feature = "context_line_review_comments")]
        FeatureFlag::ContextLineReviewComments,
        #[cfg(feature = "fast_forward_autoexecute_button")]
        FeatureFlag::FastForwardAutoexecuteButton,
        #[cfg(feature = "agent_approval_modes")]
        FeatureFlag::AgentApprovalModes,
        #[cfg(feature = "code_find_replace")]
        FeatureFlag::CodeFindReplace,
        #[cfg(feature = "command_palette_file_search")]
        FeatureFlag::CommandPaletteFileSearch,
        #[cfg(feature = "ai_context_menu_commands")]
        FeatureFlag::AIContextMenuCommands,
        #[cfg(feature = "ai_context_menu_code")]
        FeatureFlag::AIContextMenuCode,
        #[cfg(feature = "expand_edit_to_pane")]
        FeatureFlag::ExpandEditToPane,
        #[cfg(feature = "fallback_model_load_output_messaging")]
        FeatureFlag::FallbackModelLoadOutputMessaging,
        #[cfg(feature = "tab_close_button_on_left")]
        FeatureFlag::TabCloseButtonOnLeft,
        #[cfg(feature = "profiles_design_revamp")]
        FeatureFlag::ProfilesDesignRevamp,
        #[cfg(feature = "search_codebase_ui")]
        FeatureFlag::SearchCodebaseUI,
        #[cfg(feature = "linked_code_blocks")]
        FeatureFlag::LinkedCodeBlocks,
        #[cfg(feature = "tabbed_editor_view")]
        FeatureFlag::TabbedEditorView,
        #[cfg(feature = "undo_closed_panes")]
        FeatureFlag::UndoClosedPanes,
        #[cfg(feature = "multi_profile")]
        FeatureFlag::MultiProfile,
        #[cfg(feature = "conversation_artifacts")]
        FeatureFlag::ConversationArtifacts,
        #[cfg(feature = "sync_ambient_plans")]
        FeatureFlag::SyncAmbientPlans,
        #[cfg(feature = "get_started_tab")]
        FeatureFlag::GetStartedTab,
        #[cfg(feature = "projects")]
        FeatureFlag::Projects,
        #[cfg(feature = "drive_objects_as_context")]
        FeatureFlag::DriveObjectsAsContext,
        #[cfg(feature = "pr_comments_v2")]
        FeatureFlag::PRCommentsV2,
        #[cfg(feature = "pr_comments_skill")]
        FeatureFlag::PRCommentsSkill,
        #[cfg(feature = "selection_as_context")]
        FeatureFlag::SelectionAsContext,
        #[cfg(feature = "code_mode_chip")]
        FeatureFlag::CodeModeChip,
        #[cfg(feature = "github_pr_prompt_chip")]
        FeatureFlag::GithubPrPromptChip,
        #[cfg(feature = "create_project_flow")]
        FeatureFlag::CreateProjectFlow,
        #[cfg(feature = "vim_code_editor")]
        FeatureFlag::VimCodeEditor,
        #[cfg(feature = "allow_opening_file_links_using_editor_env")]
        FeatureFlag::AllowOpeningFileLinksUsingEditorEnv,
        #[cfg(feature = "revert_diff_hunk")]
        FeatureFlag::RevertDiffHunk,
        #[cfg(feature = "code_review_save_changes")]
        FeatureFlag::CodeReviewSaveChanges,
        #[cfg(feature = "file_tree")]
        FeatureFlag::FileTree,
        #[cfg(feature = "allow_ignoring_input_suggestions")]
        FeatureFlag::AllowIgnoringInputSuggestions,
        #[cfg(feature = "code_launch_modal")]
        FeatureFlag::CodeLaunchModal,
        #[cfg(feature = "api_key_management")]
        FeatureFlag::APIKeyManagement,
        #[cfg(feature = "mcp_oauth")]
        FeatureFlag::McpOauth,
        #[cfg(feature = "file_based_mcp")]
        FeatureFlag::FileBasedMcp,
        #[cfg(feature = "diff_set_as_context")]
        FeatureFlag::DiffSetAsContext,
        #[cfg(feature = "discard_per_file_and_all_changes")]
        FeatureFlag::DiscardPerFileAndAllChanges,
        #[cfg(feature = "summarization_cancellation_confirmation")]
        FeatureFlag::SummarizationCancellationConfirmation,
        #[cfg(feature = "code_review_find")]
        FeatureFlag::CodeReviewFind,
        #[cfg(feature = "ui_zoom")]
        FeatureFlag::UIZoom,
        #[cfg(feature = "auto_open_code_review_pane")]
        FeatureFlag::AutoOpenCodeReviewPane,
        #[cfg(feature = "inline_code_review")]
        FeatureFlag::InlineCodeReview,
        #[cfg(feature = "summarize_conversation_command")]
        FeatureFlag::SummarizationConversationCommand,
        #[cfg(feature = "mcp_grouped_server_context")]
        FeatureFlag::MCPGroupedServerContext,
        #[cfg(feature = "web_search_ui")]
        FeatureFlag::WebSearchUI,
        #[cfg(feature = "web_fetch_ui")]
        FeatureFlag::WebFetchUI,
        #[cfg(feature = "fork_from_command")]
        FeatureFlag::ForkFromCommand,
        #[cfg(feature = "context_window_usage_v2")]
        FeatureFlag::ContextWindowUsageV2,
        #[cfg(feature = "context_window_usage_breakdown")]
        FeatureFlag::ContextWindowUsageBreakdown,
        #[cfg(feature = "global_search")]
        FeatureFlag::GlobalSearch,
        #[cfg(feature = "embedded_code_review_comments")]
        FeatureFlag::EmbeddedCodeReviewComments,
        #[cfg(feature = "file_and_diff_set_comments")]
        FeatureFlag::FileAndDiffSetComments,
        #[cfg(feature = "revert_to_checkpoints")]
        FeatureFlag::RevertToCheckpoints,
        #[cfg(feature = "rewind_slash_command")]
        FeatureFlag::RewindSlashCommand,
        #[cfg(feature = "agent_view")]
        FeatureFlag::AgentView,
        #[cfg(feature = "agent_view_block_context")]
        FeatureFlag::AgentViewBlockContext,
        #[cfg(feature = "v4a_file_diffs")]
        FeatureFlag::V4AFileDiffs,
        #[cfg(feature = "interactive_conversation_management_view")]
        FeatureFlag::InteractiveConversationManagementView,
        #[cfg(feature = "agent_tips")]
        FeatureFlag::AgentTips,
        #[cfg(feature = "agent_mode_computer_use")]
        FeatureFlag::AgentModeComputerUse,
        #[cfg(feature = "local_computer_use")]
        FeatureFlag::LocalComputerUse,
        #[cfg(feature = "background_computer_use")]
        FeatureFlag::BackgroundComputerUse,
        #[cfg(feature = "local_claude_codex_child_harnesses")]
        FeatureFlag::LocalClaudeCodexChildHarnesses,
        #[cfg(feature = "team_api_keys")]
        FeatureFlag::TeamApiKeys,
        #[cfg(feature = "named_agents")]
        FeatureFlag::NamedAgents,
        #[cfg(feature = "cloud_conversations")]
        FeatureFlag::CloudConversations,
        #[cfg(feature = "agent_toolbar_editor")]
        FeatureFlag::AgentToolbarEditor,
        #[cfg(feature = "configurable_toolbar")]
        FeatureFlag::ConfigurableToolbar,
        #[cfg(feature = "agent_view_prompt_chip")]
        FeatureFlag::AgentViewPromptChip,
        #[cfg(feature = "classic_completions")]
        FeatureFlag::ClassicCompletions,
        #[cfg(feature = "force_classic_completions")]
        FeatureFlag::ForceClassicCompletions,
        #[cfg(feature = "agent_view_conversation_list_view")]
        FeatureFlag::AgentViewConversationListView,
        #[cfg(feature = "inline_history_menu")]
        FeatureFlag::InlineHistoryMenu,
        #[cfg(feature = "inline_repo_menu")]
        FeatureFlag::InlineRepoMenu,
        #[cfg(feature = "summarization_via_message_replacement")]
        FeatureFlag::SummarizationViaMessageReplacement,
        #[cfg(feature = "pluggable_notifications")]
        FeatureFlag::PluggableNotifications,
        #[cfg(feature = "async_find")]
        FeatureFlag::AsyncFind,
        #[cfg(feature = "list_skills")]
        FeatureFlag::ListSkills,
        #[cfg(feature = "ask_user_question")]
        FeatureFlag::AskUserQuestion,
        #[cfg(feature = "inline_profile_selector")]
        FeatureFlag::InlineProfileSelector,
        #[cfg(feature = "oz_platform_skills")]
        FeatureFlag::OzPlatformSkills,
        #[cfg(feature = "bundled_skills")]
        FeatureFlag::BundledSkills,
        #[cfg(feature = "project_manager")]
        FeatureFlag::ZapProjects,
        #[cfg(feature = "open_warp_launch_modal")]
        FeatureFlag::ZapLaunchModal,
        #[cfg(feature = "orchestration_launch_modal")]
        FeatureFlag::OrchestrationLaunchModal,
        #[cfg(feature = "agent_cli_launch_modal")]
        FeatureFlag::AgentCliLaunchModal,
        #[cfg(feature = "new_tab_styling")]
        FeatureFlag::NewTabStyling,
        #[cfg(feature = "skill_arguments")]
        FeatureFlag::SkillArguments,
        #[cfg(feature = "active_conversation_requires_interaction")]
        FeatureFlag::ActiveConversationRequiresInteraction,
        #[cfg(feature = "conversations_as_context")]
        FeatureFlag::ConversationsAsContext,
        #[cfg(feature = "incremental_auto_reload")]
        FeatureFlag::IncrementalAutoReload,
        #[cfg(feature = "wait_for_events_parent_registration")]
        FeatureFlag::WaitForEventsParentRegistration,
        #[cfg(feature = "pending_user_query_indicator")]
        FeatureFlag::PendingUserQueryIndicator,
        #[cfg(feature = "queue_slash_command")]
        FeatureFlag::QueueSlashCommand,
        #[cfg(feature = "queued_prompts_v2")]
        FeatureFlag::QueuedPromptsV2,
        #[cfg(feature = "kitty_keyboard_protocol")]
        FeatureFlag::KittyKeyboardProtocol,
        #[cfg(feature = "inline_menu_headers")]
        FeatureFlag::InlineMenuHeaders,
        #[cfg(feature = "restore_prompt_on_inline_model_selector_search")]
        FeatureFlag::RestorePromptOnInlineModelSelectorSearch,
        #[cfg(feature = "directory_tab_colors")]
        FeatureFlag::DirectoryTabColors,
        #[cfg(feature = "open_warp_new_settings_modes")]
        FeatureFlag::ZapNewSettingsModes,
        #[cfg(feature = "hoa_code_review")]
        FeatureFlag::HoaCodeReview,
        #[cfg(feature = "vertical_tabs")]
        FeatureFlag::VerticalTabs,
        #[cfg(feature = "vertical_tabs_summary_mode")]
        FeatureFlag::VerticalTabsSummaryMode,
        #[cfg(feature = "tab_configs")]
        FeatureFlag::TabConfigs,
        #[cfg(feature = "grouped_tabs")]
        FeatureFlag::GroupedTabs,
        #[cfg(feature = "pinned_tabs")]
        FeatureFlag::PinnedTabs,
        #[cfg(feature = "warp_control_cli")]
        FeatureFlag::WarpControlCli,
        #[cfg(feature = "agent_harness")]
        FeatureFlag::AgentHarness,
        #[cfg(feature = "handoff_local_cloud")]
        FeatureFlag::HandoffLocalCloud,
        #[cfg(feature = "hoa_notifications")]
        FeatureFlag::HOANotifications,
        #[cfg(feature = "open_code_notifications")]
        FeatureFlag::OpenCodeNotifications,
        #[cfg(feature = "cli_agent_rich_input")]
        FeatureFlag::CLIAgentRichInput,
        #[cfg(feature = "transfer_control_tool")]
        FeatureFlag::TransferControlTool,
        #[cfg(feature = "warpify_footer")]
        FeatureFlag::WarpifyFooter,
        #[cfg(feature = "solo_user_byok")]
        FeatureFlag::SoloUserByok,
        #[cfg(feature = "billing_and_usage_page_v2")]
        FeatureFlag::BillingAndUsagePageV2,
        #[cfg(feature = "gpt_configurable_context_window")]
        FeatureFlag::GPTConfigurableContextWindow,
        #[cfg(feature = "hoa_onboarding_flow")]
        FeatureFlag::HOAOnboardingFlow,
        #[cfg(feature = "git_operations_in_code_review")]
        FeatureFlag::GitOperationsInCodeReview,
        #[cfg(feature = "hoa_remote_control")]
        FeatureFlag::HOARemoteControl,
        #[cfg(feature = "codex_notifications")]
        FeatureFlag::CodexNotifications,
        #[cfg(feature = "codex_plugin")]
        FeatureFlag::CodexPlugin,
        #[cfg(feature = "trim_trailing_blank_lines")]
        FeatureFlag::TrimTrailingBlankLines,
        #[cfg(feature = "cloud_mode_input_v2")]
        FeatureFlag::CloudModeInputV2,
        #[cfg(feature = "handoff_cloud_cloud")]
        FeatureFlag::HandoffCloudCloud,
        #[cfg(feature = "git_credential_refresh")]
        FeatureFlag::GitCredentialRefresh,
        #[cfg(feature = "remote_code_review")]
        FeatureFlag::RemoteCodeReview,
        #[cfg(feature = "custom_model_routers")]
        FeatureFlag::CustomModelRouters,
        #[cfg(feature = "supergrok")]
        FeatureFlag::SuperGrok,
        #[cfg(feature = "gemini_enterprise")]
        FeatureFlag::GeminiEnterprise,
        #[cfg(feature = "nld_prompt_history_match")]
        FeatureFlag::NldPromptHistoryMatch,
        #[cfg(feature = "prompt_cache_expiry_warning")]
        FeatureFlag::PromptCacheExpiryWarning,
        #[cfg(feature = "osc_hyperlinks")]
        FeatureFlag::OscHyperlinks,
        #[cfg(feature = "terminal_lifecycle_recovery")]
        FeatureFlag::TerminalLifecycleRecovery,
        #[cfg(feature = "onekey_prompt")]
        FeatureFlag::OneKeyPrompt,
        #[cfg(feature = "recursive_ssh_extension")]
        FeatureFlag::RecursiveSshExtension,
        #[cfg(feature = "block_toolbelt_save_as_workflow")]
        FeatureFlag::BlockToolbeltSaveAsWorkflow,
        #[cfg(feature = "remove_alt_screen_padding")]
        FeatureFlag::RemoveAltScreenPadding,
        #[cfg(feature = "changed_lines_only_apply_diff_result")]
        FeatureFlag::ChangedLinesOnlyApplyDiffResult,
        #[cfg(feature = "server_file_browser")]
        FeatureFlag::ServerFileBrowser,
        #[cfg(feature = "windows_high_performance_gpu_default")]
        FeatureFlag::WindowsHighPerformanceGpuDefault,
        #[cfg(feature = "configurable_context_window")]
        FeatureFlag::ConfigurableContextWindow,
    ]);

    // 不稳定功能开关:统一通过 `ZAP_UNSTABLE_FEATURES` 环境变量在 release 构建里
    // 显式启用尚未正式发布的功能。值为逗号分隔的不稳定功能名(snake_case),
    // 或 `all` / `*` 表示一次性全开;dev 构建已经在 debug_assertions 路径上自动
    // 启用所有当前不稳定功能,因此此处主要服务于 release 用户。
    if let Ok(raw) = std::env::var("ZAP_UNSTABLE_FEATURES") {
        let normalized = raw.trim().to_ascii_lowercase();
        let enable_all = matches!(normalized.as_str(), "all" | "*");
        let requested: HashSet<&str> = normalized
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        for (name, flag) in UNSTABLE_FEATURES {
            if enable_all || requested.contains(name) {
                flags.insert(*flag);
            }
        }
    }

    flags
}

/// `ZAP_UNSTABLE_FEATURES` 接受的不稳定功能名 -> FeatureFlag 映射。
/// 这里登记的功能在 release 构建下默认隐藏,设置对应 token 后才会出现;
/// dev 构建走 debug_assertions 分支默认启用,无需该变量。
const UNSTABLE_FEATURES: &[(&str, FeatureFlag)] = &[
    (
        "recursive_ssh_extension",
        FeatureFlag::RecursiveSshExtension,
    ),
    ("server_file_browser", FeatureFlag::ServerFileBrowser),
    (
        "windows_high_performance_gpu_default",
        FeatureFlag::WindowsHighPerformanceGpuDefault,
    ),
];
