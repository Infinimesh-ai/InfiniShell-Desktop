pub mod bindings;
pub mod commands;

use bitflags::bitflags;
pub use commands::SlashCommandId;
use settings::SettingsMode;

bitflags! {
    /// Specifies the requirements for a slash command to be available.
    ///
    /// Each flag represents a requirement that the session context must satisfy. The command is
    /// available when the session supports *all* of the command's requirement flags.
    ///
    /// A few common cases:
    /// * If neither [`Self::AGENT_VIEW`] nor [`Self::TERMINAL_VIEW`] is set, the command is available in all modes.
    ///   A command should *not* set both flags to be available in both modes - this results in requirements that cannot be satisfied.
    /// * Most `/fork`-like slash commands require [`Self::NO_LRC_CONTROL`] and [`Self::ACTIVE_CONVERSATION`]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Availability: u16 {
        /// No requirements — always available.
        const ALWAYS = 0;
        /// Requires the agent view.
        const AGENT_VIEW = 1 << 0;
        /// Requires the terminal view.
        const TERMINAL_VIEW = 1 << 1;
        /// Requires a local session (not available in remote/cloud sessions).
        const LOCAL = 1 << 2;
        /// Requires a git repository.
        const REPOSITORY = 1 << 3;
        /// Requires that the agent is not currently in control of a long-running command.
        const NO_LRC_CONTROL = 1 << 4;
        /// Requires an active AI conversation.
        const ACTIVE_CONVERSATION = 1 << 5;
        /// 要求已启用代码库上下文。
        const CODEBASE_CONTEXT = 1 << 6;
        /// Requires AI to be globally enabled.
        const AI_ENABLED = 1 << 7;
        /// 要求当前不是云端 Agent 上下文。
        const NOT_CLOUD_AGENT = 1 << 8;
        /// 要求当前是云端 Agent 上下文。
        const CLOUD_AGENT = 1 << 9;
        /// 仅当斜杠命令数据源通过 `SlashCommandDataSource::for_cloud_mode_v2` 构造，
        /// 且启用了 `FeatureFlag::CloudModeInputV2` 时，才在会话上下文中设置。
        /// 依赖该位的命令只会出现在 V2 云模式的输入框中。
        const CLOUD_MODE_V2_COMPOSER = 1 << 10;
    }
}
/// 静态斜杠命令的稳定标识。
///
/// 前端根据该值分发命令，不再匹配命令名称字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlashCommandKind {
    Agent,
    CloudAgent,
    AddMcp,
    ApiKeys,
    ConnectGrok,
    Upgrade,
    AutoApprove,
    Statusline,
    ResetStatusline,
    Mcp,
    ViewLogs,
    Voice,
    NaturalLanguageDetection,
    Theme,
    Exit,
    CreateEnvironment,
    CreateDockerSandbox,
    CreateNewProject,
    EditSkill,
    InvokeSkill,
    AddPrompt,
    AddRule,
    Edit,
    RenameTab,
    RenameConversation,
    SetTabColor,
    Fork,
    MoveToCloud,
    OpenCodeReview,
    Index,
    Init,
    OpenProjectRules,
    OpenMcpServers,
    OpenSettingsFile,
    Changelog,
    Feedback,
    OpenRepo,
    OpenRules,
    New,
    Clear,
    Model,
    Host,
    Harness,
    Environment,
    Profile,
    Plan,
    Orchestrate,
    Compact,
    CompactAnd,
    Queue,
    ForkAndCompact,
    ForkFrom,
    ContinueLocally,
    Usage,
    RemoteControl,
    Cost,
    Conversations,
    Prompts,
    Rewind,
    ExportToClipboard,
    ExportToFile,
    VimMode,
    Status,
    CopyDebuggingId,
}

/// 静态斜杠命令所支持的应用界面。
///
/// 每个 [`StaticCommand`] 都必须显式声明仅支持 GUI、仅支持 TUI，或两者共享。
/// 支持 GUI 的变体还必须提供菜单渲染所需的图标路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommandSurfaces {
    GuiOnly { icon_path: &'static str },
    TuiOnly,
    GuiAndTui { icon_path: &'static str },
}

impl SlashCommandSurfaces {
    pub fn supports_gui(self) -> bool {
        matches!(self, Self::GuiOnly { .. } | Self::GuiAndTui { .. })
    }

    pub fn supports_tui(self) -> bool {
        matches!(self, Self::TuiOnly | Self::GuiAndTui { .. })
    }

    pub fn gui_icon_path(self) -> Option<&'static str> {
        match self {
            Self::GuiOnly { icon_path } | Self::GuiAndTui { icon_path } => Some(icon_path),
            Self::TuiOnly => None,
        }
    }
    pub fn includes(self, settings_mode: SettingsMode) -> bool {
        match settings_mode {
            SettingsMode::Gui => self.supports_gui(),
            SettingsMode::Tui => self.supports_tui(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Argument {
    pub hint_text: Option<&'static str>,
    pub is_optional: bool,
    /// If `true`, selecting the slash command from the menu (or via keybinding) will execute the
    /// slash command with no arguments.
    ///
    /// If `false`, selecting the slash command from the menu (or via keybinding) inserts the
    /// slash command into the input.
    ///
    /// Set this based on whether or not you want you think a user should always have the option to
    /// supply an argument.
    pub should_execute_on_selection: bool,
}

impl Argument {
    pub(super) fn optional() -> Self {
        Self {
            is_optional: true,
            ..Default::default()
        }
    }

    pub(super) fn required() -> Self {
        Self {
            is_optional: false,
            ..Default::default()
        }
    }

    pub(super) fn with_hint_text(mut self, text: &'static str) -> Self {
        self.hint_text = Some(text);
        self
    }

    pub(super) fn with_execute_on_selection(mut self) -> Self {
        self.should_execute_on_selection = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCommand {
    pub kind: SlashCommandKind,
    pub name: &'static str,
    pub description: &'static str,
    pub supported_surfaces: SlashCommandSurfaces,
    /// Specifies the requirements for this command to be available. See [`Availability`].
    pub availability: Availability,
    /// Whether this command requires AI mode when executed.
    /// If true, AI mode will be activated when the command is accepted.
    pub auto_enter_ai_mode: bool,
    pub argument: Option<Argument>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandArgumentHint {
    pub input_prefix: String,
    pub text: String,
}

impl StaticCommand {
    pub fn supports_gui(&self) -> bool {
        self.supported_surfaces.supports_gui()
    }

    pub fn supports_tui(&self) -> bool {
        self.supported_surfaces.supports_tui()
    }
    pub fn supports_surface(&self, settings_mode: SettingsMode) -> bool {
        self.supported_surfaces.includes(settings_mode)
    }

    pub fn matches_filter(&self, filter_text: &str) -> bool {
        if filter_text.is_empty() {
            return true;
        }

        let filter_lower = filter_text.to_lowercase();
        self.name
            .to_lowercase()
            .get(1..)
            .unwrap_or("")
            .starts_with(&filter_lower)
    }

    pub fn is_active(&self, session_context: Availability) -> bool {
        session_context.contains(self.availability)
    }

    /// 返回当前界面语言下的命令说明。
    ///
    /// `description` 字段仍保留英文原文，供静态注册和不初始化界面资源的底层测试使用；
    /// 所有面向用户的菜单都必须通过本方法取值。
    pub fn localized_description(&self) -> String {
        match self.name {
            "/agent" => crate::t!("slash-cmd-agent-desc"),
            "/cloud-agent" => crate::t!("slash-cmd-cloud-agent-desc"),
            "/add-mcp" => crate::t!("slash-cmd-add-mcp-desc"),
            "/reset-statusline" => crate::t!("slash-cmd-reset-statusline-desc"),
            "/statusline" => crate::t!("slash-cmd-statusline-desc"),
            "/auto-approve" => crate::t!("slash-cmd-auto-approve-desc"),
            "/mcp" => crate::t!("slash-cmd-mcp-desc"),
            "/view-logs" => crate::t!("slash-cmd-view-logs-desc"),
            "/voice" => crate::t!("slash-cmd-voice-desc"),
            "/natural-language-detection" => {
                crate::t!("slash-cmd-natural-language-detection-desc")
            }
            "/api-keys" => crate::t!("slash-cmd-api-keys-desc"),
            "/connect-grok" => crate::t!("slash-cmd-connect-grok-desc"),
            "/upgrade" => crate::t!("slash-cmd-upgrade-desc"),
            "/theme" => crate::t!("slash-cmd-theme-desc"),
            "/exit" => crate::t!("slash-cmd-exit-desc"),
            "/status" => crate::t!("slash-cmd-status-desc"),
            "/create-environment" => crate::t!("slash-cmd-create-environment-desc"),
            "/docker-sandbox" => crate::t!("slash-cmd-docker-sandbox-desc"),
            "/create-new-project" => crate::t!("slash-cmd-create-new-project-desc"),
            "/open-skill" => crate::t!("slash-cmd-open-skill-desc"),
            "/skills" => crate::t!("slash-cmd-skills-desc"),
            "/add-prompt" => crate::t!("slash-cmd-add-prompt-desc"),
            "/add-rule" => crate::t!("slash-cmd-add-rule-desc"),
            "/open-file" => crate::t!("slash-cmd-open-file-desc"),
            "/rename-tab" => crate::t!("slash-cmd-rename-tab-desc"),
            "/rename-conversation" => crate::t!("slash-cmd-rename-conversation-desc"),
            "/set-tab-color" => crate::t!("slash-cmd-set-tab-color-desc"),
            "/fork" => crate::t!("slash-cmd-fork-desc"),
            "/handoff" => crate::t!("slash-cmd-handoff-desc"),
            "/pr-comments" => crate::t!("slash-cmd-pr-comments-desc"),
            "/open-code-review" => crate::t!("slash-cmd-open-code-review-desc"),
            "/index" => crate::t!("slash-cmd-index-desc"),
            "/init" => crate::t!("slash-cmd-init-desc"),
            "/open-project-rules" => crate::t!("slash-cmd-open-project-rules-desc"),
            "/open-mcp-servers" => crate::t!("slash-cmd-open-mcp-servers-desc"),
            "/open-settings-file" => crate::t!("slash-cmd-open-settings-file-desc"),
            "/changelog" => crate::t!("slash-cmd-changelog-desc"),
            "/feedback" => crate::t!("slash-cmd-feedback-desc"),
            "/open-repo" => crate::t!("slash-cmd-open-repo-desc"),
            "/open-rules" => crate::t!("slash-cmd-open-rules-desc"),
            "/new" => crate::t!("slash-cmd-new-desc"),
            "/clear" => crate::t!("slash-cmd-clear-desc"),
            "/model" => crate::t!("slash-cmd-model-desc"),
            "/host" => crate::t!("slash-cmd-host-desc"),
            "/harness" => crate::t!("slash-cmd-harness-desc"),
            "/environment" => crate::t!("slash-cmd-environment-desc"),
            "/profile" => crate::t!("slash-cmd-profile-desc"),
            "/plan" => crate::t!("slash-cmd-plan-desc"),
            "/orchestrate" => crate::t!("slash-cmd-orchestrate-desc"),
            "/compact" => crate::t!("slash-cmd-compact-desc"),
            "/compact-and" => crate::t!("slash-cmd-compact-and-desc"),
            "/queue" => crate::t!("slash-cmd-queue-desc"),
            "/fork-and-compact" => crate::t!("slash-cmd-fork-and-compact-desc"),
            "/fork-from" => crate::t!("slash-cmd-fork-from-desc"),
            "/continue-locally" => crate::t!("slash-cmd-continue-locally-desc"),
            "/usage" => crate::t!("slash-cmd-usage-desc"),
            "/remote-control" => crate::t!("slash-cmd-remote-control-desc"),
            "/cost" => crate::t!("slash-cmd-cost-desc"),
            "/conversations" => crate::t!("slash-cmd-conversations-desc"),
            "/prompts" => crate::t!("slash-cmd-prompts-desc"),
            "/rewind" => crate::t!("slash-cmd-rewind-desc"),
            "/export-to-clipboard" => crate::t!("slash-cmd-export-to-clipboard-desc"),
            "/export-to-file" => crate::t!("slash-cmd-export-to-file-desc"),
            "/vim-mode" => crate::t!("slash-cmd-vim-mode-desc"),
            "/copy-debugging-id" => crate::t!("slash-cmd-copy-debugging-id-desc"),
            command_name => {
                log::warn!("未找到斜杠命令 {command_name:?} 的本地化说明，回退到注册文本");
                self.description.to_owned()
            }
        }
    }

    pub fn argument_hint(&self) -> Option<SlashCommandArgumentHint> {
        let fallback = self.argument.as_ref()?.hint_text?;
        let text = match self.name {
            "/theme" => crate::t!("slash-cmd-theme-hint"),
            "/create-environment" => crate::t!("slash-cmd-create-environment-hint"),
            "/create-new-project" => crate::t!("slash-cmd-create-new-project-hint"),
            "/open-file" => crate::t!("slash-cmd-open-file-hint"),
            "/rename-tab" => crate::t!("slash-cmd-rename-tab-hint"),
            "/rename-conversation" => crate::t!("slash-cmd-rename-conversation-hint"),
            "/fork" => crate::t!("slash-cmd-fork-hint"),
            "/handoff" => crate::t!("slash-cmd-handoff-hint"),
            "/plan" | "/orchestrate" => crate::t!("slash-cmd-plan-hint"),
            "/compact" => crate::t!("slash-cmd-compact-hint"),
            "/compact-and" => crate::t!("slash-cmd-compact-and-hint"),
            "/queue" => crate::t!("slash-cmd-queue-hint"),
            "/fork-and-compact" => crate::t!("slash-cmd-fork-and-compact-hint"),
            "/continue-locally" => crate::t!("slash-cmd-continue-locally-hint"),
            "/export-to-file" => crate::t!("slash-cmd-export-to-file-hint"),
            "/set-tab-color" => fallback.to_owned(),
            command_name => {
                log::warn!("未找到斜杠命令 {command_name:?} 的本地化参数提示，回退到注册文本");
                fallback.to_owned()
            }
        };
        Some(SlashCommandArgumentHint {
            input_prefix: format!("{} ", self.name),
            text,
        })
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
