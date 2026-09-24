//! CLI agent detection and configuration.
//!
//! This module provides types for detecting and working with CLI-based AI agents
//! like Claude Code, Gemini CLI, Codex, Amp, and Droid.

use std::borrow::Cow;
use std::collections::HashMap;
#[cfg(unix)]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

use ai::skills::{SkillProvider, SkillScope};
#[cfg(not(target_family = "wasm"))]
use command::Stdio;
#[cfg(not(target_family = "wasm"))]
use command::r#async::Command;
use enum_iterator::Sequence;
#[cfg(not(target_family = "wasm"))]
use futures::future::join_all;
use markdown_parser::parse_markdown;
use pathfinder_color::ColorU;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use warp_cli::agent::Harness;
use warp_completer::parsers::simple::{all_parsed_commands, decompose_command, top_level_command};
use warp_editor::content::buffer::Buffer;
use warp_editor::content::markdown::MarkdownStyle;
use warp_util::path::EscapeChar;
#[cfg(not(target_family = "wasm"))]
use warpui::r#async::FutureExt as _;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::ai::agent::{AgentReviewCommentBatch, DiffSetHunk};
use crate::ai::blocklist::CLAUDE_ORANGE;
#[cfg(all(feature = "local_fs", not(target_family = "wasm")))]
use crate::ai::cli_agent_runtime::coordinator::LocalCLITaskCoordinator;
use crate::code::editor::line::EditorLineLocation;
use crate::code_review::comments::AttachedReviewCommentTarget;
use crate::server::telemetry::CLIAgentType;
#[cfg(not(target_family = "wasm"))]
use crate::settings::{AISettings, CLIUpdateChannel};
#[cfg(not(target_family = "wasm"))]
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::cli_agent_updates::{
    CliAgentUpdateChannel, CliAgentUpdateEvent, CliAgentUpdatesModel,
};
use crate::terminal::model::session::command_executor::shell_quote_arg;
use crate::terminal::shell::ShellType;
use crate::ui_components::icons::Icon;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// UID for the Uber team.
/// See https://warp.metabaseapp.com/dashboard/1454?team_id=46347
const UBER_TEAM_UID: &str = "BdVbYjy9LRZcZrYBemSfAF";

/// Gemini brand blue color
pub(crate) const GEMINI_BLUE: ColorU = ColorU {
    r: 66,
    g: 133,
    b: 244,
    a: 255,
};

/// OpenAI brand color (dark gray/black)
pub(crate) const OPENAI_COLOR: ColorU = ColorU {
    r: 0,
    g: 0,
    b: 0,
    a: 255,
};

/// Amp brand color (#F34E3F)
const AMP_COLOR: ColorU = ColorU {
    r: 243,
    g: 78,
    b: 63,
    a: 255,
};

/// Droid brand color (white)
const DROID_COLOR: ColorU = ColorU {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// OpenCode brand color (gray, used for contrast calculation only)
pub(crate) const OPENCODE_COLOR: ColorU = ColorU {
    r: 128,
    g: 128,
    b: 128,
    a: 255,
};

/// Copilot brand color (Copilot purple selected from https://brand.github.com/brand-identity/copilot)
const COPILOT_COLOR: ColorU = ColorU {
    r: 133,
    g: 52,
    b: 243,
    a: 255,
};

/// Pi brand color (white, monochrome logo)
const PI_COLOR: ColorU = ColorU {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// Auggie brand color (white, monochrome logo)
const AUGGIE_COLOR: ColorU = ColorU {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// Cursor brand color (#26251E, from official brand assets)
const CURSOR_COLOR: ColorU = ColorU {
    r: 38,
    g: 37,
    b: 30,
    a: 255,
};

/// Antigravity brand color (#7C3AED, purple from official banner accent)
const ANTIGRAVITY_PURPLE: ColorU = ColorU {
    r: 0x7C,
    g: 0x3A,
    b: 0xED,
    a: 255,
};

/// DeepSeek brand color (#3578E5)
const DEEPSEEK_COLOR: ColorU = ColorU {
    r: 53,
    g: 120,
    b: 229,
    a: 255,
};

/// Goose brand color (#101010, from Block's official Goose logo)
const GOOSE_COLOR: ColorU = ColorU {
    r: 16,
    g: 16,
    b: 16,
    a: 255,
};

/// omp (oh-my-pi) brand color (#9b4dff, midpoint purple of the official pink→purple→blue gradient π logo)
const OMP_COLOR: ColorU = ColorU {
    r: 0x9b,
    g: 0x4d,
    b: 0xff,
    a: 255,
};

/// Hermes brand color (Nous Research purple #7C3AED)
const HERMES_PURPLE: ColorU = ColorU {
    r: 124,
    g: 58,
    b: 237,
    a: 255,
};

/// Mistral brand orange (#FA520F)
const MISTRAL_ORANGE: ColorU = ColorU {
    r: 250,
    g: 82,
    b: 15,
    a: 255,
};

/// Represents a CLI agent (e.g., Claude Code, Gemini CLI, Codex, Amp, Droid, OpenCode, Copilot, Pi, Auggie, Cursor, Goose, Hermes, Mistral Vibe)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Sequence, Serialize, Deserialize)]
pub enum CLIAgent {
    Claude,
    Gemini,
    Codex,
    Grok,
    Amp,
    Droid,
    OpenCode,
    Copilot,
    Pi,
    OhMyPi,
    Auggie,
    CursorCli,
    Goose,
    DeepSeek,
    Hermes,
    Vibe,
    Antigravity,
    Omp,
    /// Warp's own headless TUI.
    WarpTui,
    /// Represents an unknown/custom CLI agent matched by user-configured regex patterns.
    Unknown,
}

impl CLIAgent {
    /// Command prefixes that identify this CLI agent.
    pub(crate) fn command_prefixes(&self) -> &'static [&'static str] {
        match self {
            CLIAgent::Claude => &["claude"],
            CLIAgent::Gemini => &["gemini"],
            CLIAgent::Codex => &["codex"],
            CLIAgent::Grok => &["grok"],
            CLIAgent::Amp => &["amp"],
            CLIAgent::Droid => &["droid"],
            CLIAgent::OpenCode => &["opencode"],
            CLIAgent::Copilot => &["copilot"],
            CLIAgent::Pi => &["pi"],
            CLIAgent::OhMyPi => &["omp"],
            CLIAgent::Auggie => &["auggie"],
            CLIAgent::CursorCli => &["agent"],
            CLIAgent::Goose => &["goose"],
            CLIAgent::DeepSeek => &["deepseek", "deepseek-tui"],
            CLIAgent::Hermes => &["hermes"],
            CLIAgent::Vibe => &["vibe", "vibe-acp"],
            CLIAgent::Antigravity => &["agy"],
            CLIAgent::Omp => &["omp"],
            CLIAgent::WarpTui => &[
                "infinishell-tui",
                "infinishell-tui-local",
                "infinishell-tui-dev",
                "infinishell-tui-preview",
                "infinishell-tui-stable",
                "warp",
                "warp-preview",
                "warp-dev",
                "warp-tui",
                "warp-tui-oss",
                "run-tui",
            ],
            CLIAgent::Unknown => &[],
        }
    }

    /// The canonical command prefix used to identify this CLI agent in places
    /// that require one stable value.
    pub fn command_prefix(&self) -> &'static str {
        self.command_prefixes().first().copied().unwrap_or_default()
    }

    /// Serialized version of the CLIAgent name (e.g. "Claude", "Gemini"). Used for the
    /// session-sharing protocol's opaque `cli_agent` string field.
    pub fn to_serialized_name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    /// Inverse of `to_serialized_name`. Falls back to `Unknown`.
    pub fn from_serialized_name(name: &str) -> CLIAgent {
        serde_json::from_value(name.into()).unwrap_or(CLIAgent::Unknown)
    }

    /// Returns the [`CLIAgent`] corresponding to a cloud-agent [`Harness`] when it represents a
    /// third-party agent. Returns `None` for [`Harness::Oz`] (Warp's built-in harness has no
    /// distinct CLI agent identity).
    pub fn from_harness(harness: Harness) -> Option<Self> {
        match harness {
            Harness::Oz => None,
            Harness::Claude => Some(CLIAgent::Claude),
            Harness::Gemini => Some(CLIAgent::Gemini),
            Harness::OpenCode => Some(CLIAgent::OpenCode),
            Harness::Codex => Some(CLIAgent::Codex),
            Harness::Grok => Some(CLIAgent::Grok),
            Harness::Unknown => Some(CLIAgent::Unknown),
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            CLIAgent::Claude => "Claude Code".to_string(),
            CLIAgent::Gemini => "Gemini".to_string(),
            CLIAgent::Codex => "Codex".to_string(),
            CLIAgent::Grok => "Grok Build".to_string(),
            CLIAgent::Amp => "Amp".to_string(),
            CLIAgent::Droid => "Droid".to_string(),
            CLIAgent::OpenCode => "OpenCode".to_string(),
            CLIAgent::Copilot => "Copilot".to_string(),
            CLIAgent::Pi => "Pi".to_string(),
            CLIAgent::OhMyPi => "oh-my-pi".to_string(),
            CLIAgent::Auggie => "Auggie".to_string(),
            CLIAgent::CursorCli => "Cursor".to_string(),
            CLIAgent::Goose => "Goose".to_string(),
            CLIAgent::DeepSeek => "DeepSeek".to_string(),
            CLIAgent::Hermes => "Hermes".to_string(),
            CLIAgent::Vibe => "Mistral Vibe".to_string(),
            CLIAgent::Antigravity => "Antigravity".to_string(),
            CLIAgent::Omp => "Omp".to_string(),
            CLIAgent::WarpTui => "InfiniShell TUI".to_string(),
            CLIAgent::Unknown => crate::t!("cli-agent-generic-name"),
        }
    }

    /// Returns the Icon for this CLI agent, or `None` for unknown/custom agents.
    pub fn icon(&self) -> Option<Icon> {
        match self {
            CLIAgent::Claude => Some(Icon::ClaudeLogo),
            CLIAgent::Gemini => Some(Icon::GeminiLogo),
            CLIAgent::Codex => Some(Icon::OpenAILogo),
            CLIAgent::Grok => Some(Icon::GrokLogo),
            CLIAgent::Amp => Some(Icon::AmpLogo),
            CLIAgent::Droid => Some(Icon::DroidLogo),
            CLIAgent::OpenCode => Some(Icon::OpenCodeLogo),
            CLIAgent::Copilot => Some(Icon::CopilotLogo),
            CLIAgent::Pi => Some(Icon::PiLogo),
            CLIAgent::OhMyPi => Some(Icon::OhMyPiLogo),
            CLIAgent::Auggie => Some(Icon::AuggieLogo),
            CLIAgent::CursorCli => Some(Icon::CursorLogo),
            CLIAgent::Goose => Some(Icon::GooseLogo),
            CLIAgent::DeepSeek => Some(Icon::DeepSeekLogo),
            CLIAgent::Hermes => None,
            // Vibe is recognized but ships without a brand asset. The brand color
            // still drives the toolbar tile; an `Icon::MistralLogo` can be wired
            // up in a follow-up once an officially licensed SVG is available.
            CLIAgent::Vibe => None,
            CLIAgent::Antigravity => Some(Icon::AntigravityLogo),
            CLIAgent::Omp => Some(Icon::OmpLogo),
            CLIAgent::WarpTui => Some(Icon::InfiniShell),
            CLIAgent::Unknown => None,
        }
    }

    /// Returns the skill providers whose skills this CLI agent can natively interpret.
    /// When the CLI agent rich input is open, only skills from these providers are shown
    /// in the slash menu. Returns an empty slice for agents with no known skills support.
    pub fn supported_skill_providers(&self) -> &'static [SkillProvider] {
        match self {
            CLIAgent::Claude => &[SkillProvider::Claude],
            CLIAgent::Grok => &[SkillProvider::Grok, SkillProvider::Claude],
            CLIAgent::Codex => &[
                SkillProvider::Agents,
                SkillProvider::Claude,
                SkillProvider::Codex,
            ],
            CLIAgent::OpenCode => &[
                SkillProvider::OpenCode,
                SkillProvider::Agents,
                SkillProvider::Claude,
            ],
            CLIAgent::Gemini => &[SkillProvider::Agents, SkillProvider::Gemini],
            CLIAgent::Amp => &[SkillProvider::Agents],
            CLIAgent::Copilot => &[SkillProvider::Agents, SkillProvider::Copilot],
            CLIAgent::Droid => &[SkillProvider::Droid, SkillProvider::Agents],
            CLIAgent::Pi => &[SkillProvider::Agents],
            CLIAgent::OhMyPi => &[SkillProvider::Agents],
            CLIAgent::Auggie => &[SkillProvider::Agents],
            CLIAgent::CursorCli => &[SkillProvider::Agents],
            CLIAgent::Goose => &[SkillProvider::Agents],
            CLIAgent::DeepSeek => &[SkillProvider::Agents],
            CLIAgent::Hermes => &[SkillProvider::Agents],
            CLIAgent::Vibe => &[SkillProvider::Agents],
            CLIAgent::Antigravity => &[SkillProvider::Agents],
            CLIAgent::Omp => &[SkillProvider::Agents],
            CLIAgent::WarpTui => &[],
            CLIAgent::Unknown => &[],
        }
    }

    /// Grok 只声明兼容用户级 Agents 技能，不能把项目级目录误呈现为原生技能。
    pub fn supports_skill(&self, provider: SkillProvider, scope: SkillScope) -> bool {
        self.supported_skill_providers_for_scope(scope)
            .contains(&provider)
    }

    pub fn supported_skill_providers_for_scope(
        &self,
        scope: SkillScope,
    ) -> &'static [SkillProvider] {
        if *self == Self::Grok && scope == SkillScope::Home {
            &[
                SkillProvider::Grok,
                SkillProvider::Claude,
                SkillProvider::Agents,
            ]
        } else {
            self.supported_skill_providers()
        }
    }

    /// Returns the prefix character used for skill invocations by this CLI agent.
    /// Most agents use `/` (e.g. `/skill-name`), but Codex uses `$` (e.g. `$skill-name`).
    pub fn skill_command_prefix(&self) -> &'static str {
        match self {
            CLIAgent::Codex => "$",
            _ => "/",
        }
    }

    /// Whether this CLI agent supports the `!` bash mode prefix in the rich input.
    /// When `true`, typing `!` in the CLI agent rich input activates shell mode with
    /// decorations, completions, and error underlining.
    ///
    /// TODO(advait): Check whether Gemini, Amp, Droid, and Copilot support `!` bash
    /// mode and enable them here if so.
    pub fn supports_bash_mode(&self) -> bool {
        matches!(
            self,
            CLIAgent::Claude
                | CLIAgent::Codex
                | CLIAgent::OpenCode
                | CLIAgent::DeepSeek
                | CLIAgent::OhMyPi
        )
    }

    /// Whether Warp should show its CLI-agent footer for this agent.
    pub(super) fn supports_cli_agent_footer(&self) -> bool {
        !matches!(self, CLIAgent::WarpTui)
    }

    /// Returns the brand color for this CLI agent, or `None` for unknown/custom agents.
    pub fn brand_color(&self) -> Option<ColorU> {
        match self {
            CLIAgent::Claude => Some(CLAUDE_ORANGE),
            CLIAgent::Gemini => Some(GEMINI_BLUE),
            CLIAgent::Codex => Some(OPENAI_COLOR),
            CLIAgent::Grok => Some(ColorU::black()),
            CLIAgent::Amp => Some(AMP_COLOR),
            CLIAgent::Droid => Some(DROID_COLOR),
            CLIAgent::OpenCode => Some(OPENCODE_COLOR),
            CLIAgent::Copilot => Some(COPILOT_COLOR),
            CLIAgent::Pi => Some(PI_COLOR),
            CLIAgent::OhMyPi => Some(PI_COLOR),
            CLIAgent::Auggie => Some(AUGGIE_COLOR),
            CLIAgent::CursorCli => Some(CURSOR_COLOR),
            CLIAgent::Goose => Some(GOOSE_COLOR),
            CLIAgent::DeepSeek => Some(DEEPSEEK_COLOR),
            CLIAgent::Hermes => Some(HERMES_PURPLE),
            CLIAgent::Vibe => Some(MISTRAL_ORANGE),
            CLIAgent::Antigravity => Some(ANTIGRAVITY_PURPLE),
            CLIAgent::Omp => Some(OMP_COLOR),
            CLIAgent::WarpTui => Some(ColorU::black()),
            CLIAgent::Unknown => None,
        }
    }

    /// Returns the icon color to use when rendered on the brand-colored circle background.
    /// Agents with light brand colors use a dark icon for contrast.
    pub fn brand_icon_color(&self) -> ColorU {
        match self {
            CLIAgent::Pi | CLIAgent::OhMyPi | CLIAgent::Auggie | CLIAgent::Droid => {
                ColorU::new(0, 0, 0, 255)
            }
            _ => ColorU::white(),
        }
    }

    /// Extracts the first meaningful command token from a command string.
    ///
    /// When `escape_char` is provided, uses shell parsing to skip leading
    /// env-var assignments (e.g. `FOO=1 claude` → `claude`).
    /// Otherwise falls back to a simple whitespace split.
    fn extract_first_command(command: &str, escape_char: Option<EscapeChar>) -> Option<String> {
        match escape_char {
            Some(esc) => top_level_command(command, esc),
            None => top_level_command(command, EscapeChar::Backslash),
        }
    }

    /// Returns whether the command's executable name identifies this CLI agent.
    pub(super) fn matches_command(&self, command: &str, escape_char: Option<EscapeChar>) -> bool {
        let Some(first_word) = Self::extract_first_command(command.trim_start(), escape_char)
        else {
            return false;
        };
        let basename = first_word.rsplit(['/', '\\']).next().unwrap_or(&first_word);
        if matches!(self, Self::Claude | Self::Codex | Self::Grok) {
            let basename = basename
                .strip_suffix(".exe")
                .or_else(|| basename.strip_suffix(".cmd"))
                .or_else(|| basename.strip_suffix(".bat"))
                .unwrap_or(basename);
            return self.command_prefixes().contains(&basename)
                && self.is_interactive_command(command, escape_char);
        }
        self.command_prefixes().contains(&basename)
    }

    /// 只给交互会话提供终端工具栏；管理命令和结构化服务不接收富输入。
    fn is_interactive_command(&self, command: &str, escape_char: Option<EscapeChar>) -> bool {
        let Some(parsed) =
            all_parsed_commands(command, escape_char.unwrap_or(EscapeChar::Backslash)).next()
        else {
            return false;
        };
        let mut arguments = parsed.parts.iter().skip(1).peekable();
        let mut saw_positional = false;
        while let Some(argument) = arguments.next() {
            let argument = argument.item.as_str();
            if argument == "--" {
                break;
            }
            let flag = argument.split('=').next().unwrap_or(argument);
            if matches!(flag, "--help" | "-h" | "--version" | "-V" | "-v")
                || (matches!(self, Self::Claude) && matches!(flag, "--print" | "-p"))
                || (matches!(self, Self::Grok)
                    && matches!(flag, "--single" | "-p" | "--prompt-file" | "--prompt-json"))
            {
                return false;
            }
            if argument.starts_with('-') {
                if !argument.contains('=') && self.option_takes_value(flag) {
                    if arguments
                        .peek()
                        .is_some_and(|next| !next.item.starts_with('-'))
                    {
                        arguments.next();
                    } else if !matches!(flag, "-r" | "--resume" | "-w" | "--worktree") {
                        return false;
                    }
                }
                continue;
            }
            if !saw_positional {
                if self.non_interactive_commands().contains(&argument) {
                    return false;
                }
                saw_positional = true;
            }
        }
        true
    }

    /// 依据受测版本帮助枚举参数，避免把模型名、目录或配置值当成管理子命令。
    fn option_takes_value(&self, flag: &str) -> bool {
        match self {
            Self::Codex => matches!(
                flag,
                "-c" | "--config"
                    | "--enable"
                    | "--disable"
                    | "--remote"
                    | "--remote-auth-token-env"
                    | "-i"
                    | "--image"
                    | "-m"
                    | "--model"
                    | "--local-provider"
                    | "-p"
                    | "--profile"
                    | "-s"
                    | "--sandbox"
                    | "-C"
                    | "--cd"
                    | "--add-dir"
                    | "-a"
                    | "--ask-for-approval"
            ),
            Self::Claude => matches!(
                flag,
                "--agent"
                    | "--agents"
                    | "--allowedTools"
                    | "--allowed-tools"
                    | "--append-system-prompt"
                    | "--betas"
                    | "--disallowedTools"
                    | "--disallowed-tools"
                    | "--fallback-model"
                    | "--input-format"
                    | "--json-schema"
                    | "--max-budget-usd"
                    | "--mcp-config"
                    | "--model"
                    | "--output-format"
                    | "--permission-mode"
                    | "--permission-prompt-tool"
                    | "--plugin-dir"
                    | "-r"
                    | "--resume"
                    | "--session-id"
                    | "--settings"
                    | "--setting-sources"
                    | "--system-prompt"
                    | "--tools"
                    | "--add-dir"
            ),
            Self::Grok => matches!(
                flag,
                "--agent"
                    | "--agents"
                    | "--allow"
                    | "--allowedTools"
                    | "--cwd"
                    | "--debug-file"
                    | "--deny"
                    | "--disallowedTools"
                    | "--disallowed-tools"
                    | "--json-schema"
                    | "--leader-socket"
                    | "-m"
                    | "--model"
                    | "--max-turns"
                    | "--output-format"
                    | "--permission-mode"
                    | "-r"
                    | "--resume"
                    | "--reasoning-effort"
                    | "--effort"
                    | "--rules"
                    | "-s"
                    | "--session-id"
                    | "--sandbox"
                    | "--system-prompt-override"
                    | "--system-prompt"
                    | "--tools"
                    | "-w"
                    | "--worktree"
                    | "--worktree-ref"
                    | "--ref"
            ),
            Self::Gemini
            | Self::Amp
            | Self::Droid
            | Self::OpenCode
            | Self::Copilot
            | Self::Pi
            | Self::OhMyPi
            | Self::Auggie
            | Self::CursorCli
            | Self::Goose
            | Self::DeepSeek
            | Self::Hermes
            | Self::Vibe
            | Self::Antigravity
            | Self::Omp
            | Self::WarpTui
            | Self::Unknown => false,
        }
    }

    fn non_interactive_commands(&self) -> &'static [&'static str] {
        match self {
            Self::Codex => &[
                "exec",
                "e",
                "review",
                "login",
                "logout",
                "mcp",
                "plugin",
                "mcp-server",
                "app-server",
                "remote-control",
                "app",
                "completion",
                "update",
                "doctor",
                "sandbox",
                "debug",
                "apply",
                "a",
                "archive",
                "delete",
                "unarchive",
                "cloud",
                "exec-server",
                "features",
                "help",
            ],
            Self::Claude => &[
                "auth",
                "doctor",
                "install",
                "mcp",
                "plugin",
                "plugins",
                "setup-token",
                "update",
                "upgrade",
                "help",
            ],
            Self::Grok => &[
                "agent",
                "clone",
                "completions",
                "cursor-worker",
                "doctor",
                "du",
                "disk-usage",
                "export",
                "help",
                "inspect",
                "leader",
                "login",
                "logout",
                "mcp",
                "memory",
                "models",
                "plugin",
                "sessions",
                "setup",
                "trace",
                "update",
                "usage",
                "version",
                "v",
                "worktree",
                "wrap",
            ],
            Self::Gemini
            | Self::Amp
            | Self::Droid
            | Self::OpenCode
            | Self::Copilot
            | Self::Pi
            | Self::OhMyPi
            | Self::Auggie
            | Self::CursorCli
            | Self::Goose
            | Self::DeepSeek
            | Self::Hermes
            | Self::Vibe
            | Self::Antigravity
            | Self::Omp
            | Self::WarpTui
            | Self::Unknown => &[],
        }
    }

    /// 简单解析器不保留分隔符语义；用原始范围仅接受完整的字面参数。
    /// 变量、命令替换、复杂引号拼接和注释保守拒绝，不尝试执行或补全 shell 语法。
    fn is_single_literal_command(command: &str, escape_char: EscapeChar) -> bool {
        if command.contains(['$', '`', '\n', '\r', '#', '!']) {
            return false;
        }
        let (decomposed, contains_redirection) = decompose_command(command, escape_char);
        if contains_redirection || decomposed.len() != 1 {
            return false;
        }
        let mut commands = all_parsed_commands(command, escape_char);
        let Some(parsed) = commands.next() else {
            return false;
        };
        let Some((first, last)) = parsed.parts.first().zip(parsed.parts.last()) else {
            return false;
        };
        if commands.next().is_some()
            || !command
                .get(..first.span.start())
                .is_some_and(|prefix| prefix.trim().is_empty())
            || !command
                .get(last.span.end()..)
                .is_some_and(|suffix| suffix.trim().is_empty())
        {
            return false;
        }
        parsed.parts.iter().all(|part| {
            let Some(raw) = command.get(part.span.start()..part.span.end()) else {
                return false;
            };
            let value = part.item.as_str();
            let quoted = raw
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .or_else(|| {
                    raw.strip_prefix('"')
                        .and_then(|value| value.strip_suffix('"'))
                });
            quoted == Some(value) || (raw == value && !raw.contains(['*', '?', '[', ']']))
        })
    }

    /// 仅识别一个普通 cd 后以 && 启动单个 CLI，不猜测任意复合命令当前执行到了哪一段。
    fn command_after_directory_change(command: &str, escape_char: EscapeChar) -> Option<&str> {
        if command.contains(['\n', '\r']) {
            return None;
        }
        let mut commands = all_parsed_commands(command, escape_char);
        let directory_change = commands.next()?;
        let cli = commands.next()?;
        if commands.next().is_some() {
            return None;
        }
        let [cd, path] = directory_change.parts.as_slice() else {
            return None;
        };
        let cli_start = cli.parts.first()?.span.start();
        let path_end = path.span.end();
        if cd.span.start() != 0
            || command.get(..cd.span.end()) != Some("cd")
            || path.item.is_empty()
            || path.item.starts_with('-')
            || command.get(path_end..cli_start)?.trim() != "&&"
            || !Self::is_single_literal_command(command.get(..path_end)?, escape_char)
            || !Self::is_single_literal_command(command.get(cli_start..)?, escape_char)
        {
            return None;
        }
        command.get(cli_start..)
    }

    /// Detects the CLI agent from a command string.
    ///
    /// When `escape_char` is provided, full shell parsing is used to skip leading
    /// env-var assignments (e.g. `FOO=1 claude`). Otherwise falls back to a simple
    /// whitespace split.
    ///
    /// If `aliases` is provided, the first word of the command will be looked up
    /// in the alias map. If found, the alias value replaces the first word to
    /// produce the resolved command used for detection.
    ///
    /// Returns `Some(CLIAgent)` if the command matches a known CLI agent, `None` otherwise.
    pub fn detect(
        command: &str,
        escape_char: Option<EscapeChar>,
        aliases: Option<&HashMap<SmolStr, String>>,
        ctx: &AppContext,
    ) -> Option<CLIAgent> {
        let trimmed = command.trim_start();
        let first_word = Self::extract_first_command(trimmed, escape_char)?;
        let shell_escape = escape_char.unwrap_or(EscapeChar::Backslash);
        let has_directory_change =
            first_word == "cd" && all_parsed_commands(trimmed, shell_escape).nth(1).is_some();
        let trimmed = if has_directory_change {
            // cd 被别名重定义时，不能假设它只切换目录。
            if aliases.is_some_and(|aliases| aliases.contains_key("cd")) {
                return None;
            }
            Self::command_after_directory_change(trimmed, shell_escape)?
        } else {
            trimmed
        };
        let first_word = Self::extract_first_command(trimmed, escape_char)?;

        // Resolve the full command through aliases. If the first word matches an
        // alias, replace it with the alias value to produce the resolved command.
        let resolved_command: Cow<'_, str> = aliases
            .and_then(|a| a.get(first_word.as_str()))
            .map(|alias_value| {
                let rest = trimmed
                    .find(first_word.as_str())
                    .map(|pos| &trimmed[pos + first_word.len()..])
                    .unwrap_or("");
                Cow::Owned(format!("{}{}", alias_value.trim(), rest))
            })
            .unwrap_or(Cow::Borrowed(trimmed));

        // 末段别名也必须仍是单个命令；版本、管理及非交互参数继续交给原有过滤。
        if has_directory_change && !Self::is_single_literal_command(&resolved_command, shell_escape)
        {
            return None;
        }

        // Check if resolved command matches any known CLI agent.
        // Also matches `aifx agent run claude` as Claude for Uber employees.
        enum_iterator::all::<CLIAgent>()
            .filter(|agent| {
                !matches!(agent, CLIAgent::Unknown)
                    && (!has_directory_change
                        || matches!(agent, CLIAgent::Claude | CLIAgent::Codex | CLIAgent::Grok))
            })
            .find(|agent| {
                agent.matches_command(&resolved_command, escape_char)
                    || (matches!(agent, CLIAgent::Claude)
                        && Self::is_aifx_agent_run_claude(&resolved_command, ctx))
            })
    }

    /// Returns true if the resolved command is `aifx agent run claude` (Uber's
    /// internal wrapper around Claude) and the user is on the Uber team.
    /// We special-case this so Uber employees get the toolbar without needing
    /// to configure anything.
    fn is_aifx_agent_run_claude(resolved_command: &str, ctx: &AppContext) -> bool {
        resolved_command.starts_with("aifx agent run claude")
            && Self::is_on_uber_team(UserWorkspaces::as_ref(ctx))
    }

    fn is_on_uber_team(user_workspaces: &UserWorkspaces) -> bool {
        user_workspaces
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.teams.iter())
            .any(|team| team.uid.uid() == UBER_TEAM_UID)
    }
}

/// Builds a prompt string from a batch of code review comments suitable for
/// writing to a CLI agent's PTY.
///
/// # Location format
/// Locations use `L<line>` notation (1-indexed).
/// Line ranges are written `L<start>-L<end>` where both ends are **inclusive**.
/// Instructs the agent to run `git diff` for deleted-line context rather than
/// inlining the full diff.
pub fn build_review_prompt(review: &AgentReviewCommentBatch) -> String {
    let mut text = String::from(
        "Please address the following code review comments. \
         Run `git diff` (or `git diff HEAD`) to see the full context of any changes, \
         especially for deleted lines.\n",
    );

    for comment in &review.comments {
        if comment.outdated {
            continue;
        }
        let body = export_review_comment_for_cli_prompt(&comment.content);
        let location = match &comment.target {
            AttachedReviewCommentTarget::Line {
                absolute_file_path,
                line,
                ..
            } => {
                let path = absolute_file_path.display_path();
                match line {
                    EditorLineLocation::Current { line_number, .. } => {
                        let n = line_number.as_usize() + 1;
                        format!("{path} L{n}")
                    }
                    EditorLineLocation::Removed { line_number, .. } => {
                        let n = line_number.as_usize() + 1;
                        format!("{path} (deleted, was L{n} — see `git diff`)")
                    }
                    EditorLineLocation::Collapsed { line_range } => {
                        // line_range is [start, end) 0-indexed; convert to L<start>-L<end>
                        // where both start and end are 1-indexed inclusive.
                        let start = line_range.start.as_usize() + 1;
                        let end = line_range.end.as_usize();
                        format!("{path} (collapsed hunk, L{start}-L{end} — see `git diff`)")
                    }
                }
            }
            AttachedReviewCommentTarget::File { absolute_file_path } => {
                let path = absolute_file_path.display_path();
                let is_deleted = review.diff_set.iter().any(|(file_key, hunks)| {
                    path.ends_with(file_key.as_str())
                        && !hunks.is_empty()
                        && hunks
                            .iter()
                            .all(|h| h.lines_added == 0 && h.lines_removed > 0)
                });
                if is_deleted {
                    format!("{path} (deleted file — see `git diff`)")
                } else {
                    path
                }
            }
            AttachedReviewCommentTarget::General => "General".to_string(),
        };
        text.push_str(&format!("\n- {location}: {body}"));
    }

    text
}

fn export_review_comment_for_cli_prompt(comment: &str) -> String {
    let mut result = parse_markdown(comment)
        .map(|parsed| {
            Buffer::export_to_markdown(
                parsed,
                None,
                MarkdownStyle::Export {
                    app_context: None,
                    should_not_escape_markdown_punctuation: true,
                },
            )
        })
        .unwrap_or_else(|_| comment.to_string());
    result.truncate(result.trim_end().len());
    result
}

/// Builds a prompt string for a single diff hunk location suitable for writing
/// to a CLI agent's PTY. Includes change stats (+N -N) and instructs the agent
/// to run `git diff` for full context.
///
/// # Location format
/// `<path> L<start>-L<end>` where `start` and `end` are 1-indexed and both
/// ends are **inclusive**.
pub fn build_diff_hunk_prompt(
    file_path: &str,
    start_line: usize,
    end_line: usize,
    lines_added: u32,
    lines_removed: u32,
) -> String {
    format!(
        "{file_path} L{start_line}-L{end_line} (+{lines_added} -{lines_removed}) \
         -- run `git diff` to see the full context."
    )
}

/// Builds a prompt string for a set of diff file context hunks suitable for
/// writing to a CLI agent's PTY.
///
/// # Location format
/// Each line is `<path> L<start>-L<end> (+N -N)` where `start` and `end` are
/// 1-indexed and both ends are **inclusive**.
pub fn build_diff_context_prompt(file_diffs: &HashMap<String, Vec<DiffSetHunk>>) -> String {
    let mut text = String::new();
    let mut sorted_keys: Vec<&String> = file_diffs.keys().collect();
    sorted_keys.sort();
    for file_key in sorted_keys {
        let hunks = &file_diffs[file_key];
        for hunk in hunks {
            // hunk.line_range is [start, end) 0-indexed; convert to L<start>-L<end>
            // where both start and end are 1-indexed inclusive.
            let start = hunk.line_range.start.as_usize() + 1;
            let end = hunk.line_range.end.as_usize();
            text.push_str(&format!(
                "{file_key} L{start}-L{end} (+{} -{})",
                hunk.lines_added, hunk.lines_removed,
            ));
            text.push('\n');
        }
    }
    // Remove trailing newline.
    text.truncate(text.trim_end().len());
    text
}

/// Builds a prompt for a single-line text selection suitable for writing to a CLI agent's PTY.
/// Prefixes the literal text with its file path and line number for context.
///
/// # Format
/// `<path> L<line>: <text>` where `line` is 1-indexed.
pub fn build_selection_substring_prompt(file_path: &str, line: usize, text: &str) -> String {
    format!("{file_path} L{line}: {text}")
}

/// Builds a prompt for a multi-line selection suitable for writing to a CLI agent's PTY.
/// For single-line selections, use [`build_selection_substring_prompt`] instead.
///
/// # Location format
/// `<path> L<start>-L<end>` where line numbers are 1-indexed and both ends are inclusive.
pub fn build_selection_line_range_prompt(
    file_path: &str,
    start_line: usize,
    end_line: usize,
) -> String {
    format!("{file_path} L{start_line}-L{end_line}")
}

impl From<CLIAgent> for CLIAgentType {
    fn from(agent: CLIAgent) -> Self {
        match agent {
            CLIAgent::Claude => CLIAgentType::Claude,
            CLIAgent::Gemini => CLIAgentType::Gemini,
            CLIAgent::Codex => CLIAgentType::Codex,
            CLIAgent::Grok => CLIAgentType::Grok,
            CLIAgent::Amp => CLIAgentType::Amp,
            CLIAgent::Droid => CLIAgentType::Droid,
            CLIAgent::OpenCode => CLIAgentType::OpenCode,
            CLIAgent::Copilot => CLIAgentType::Copilot,
            CLIAgent::Pi => CLIAgentType::Pi,
            CLIAgent::OhMyPi => CLIAgentType::OhMyPi,
            CLIAgent::Auggie => CLIAgentType::Auggie,
            CLIAgent::CursorCli => CLIAgentType::Cursor,
            CLIAgent::Goose => CLIAgentType::Goose,
            CLIAgent::DeepSeek => CLIAgentType::DeepSeek,
            CLIAgent::Hermes => CLIAgentType::Hermes,
            CLIAgent::Vibe => CLIAgentType::Vibe,
            CLIAgent::Antigravity => CLIAgentType::Antigravity,
            CLIAgent::Omp => CLIAgentType::Omp,
            CLIAgent::WarpTui => CLIAgentType::WarpTui,
            CLIAgent::Unknown => CLIAgentType::Unknown,
        }
    }
}

// ── CLI Agent 安装状态 singleton model ──
// 对齐 AntivirusInfo 模式:ctx.spawn 异步扫描 → 回调 emit 事件 → 订阅者自动刷新 UI

/// CLI agent 安装扫描完成事件。
pub enum CLIAgentInstallEvent {
    /// 后台扫描完成，安装状态缓存已就绪。
    ScanComplete,
}

/// 安装发现与版本探测的结果；PATH 命中不代表已登录或托管协议可用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CLIAgentInstallation {
    pub executable: Option<PathBuf>,
    pub version: CLIAgentVersionStatus,
}

/// 仅使用当前终端的完整命令快照；安装扫描本身不能证明 shell 找不到命令。
pub(crate) struct CLIAgentLaunchSnapshot<'a> {
    pub host_namespace_verified: bool,
    pub command_snapshot_complete: bool,
    pub shell_type: ShellType,
    pub path: Option<&'a str>,
    pub cwd: Option<&'a Path>,
    pub command_known: bool,
}

/// 在可靠的本地主 shell 中保留 alias/function/PATH 优先级，再考虑已安装路径。
pub(crate) fn cli_agent_launch_fallback(
    agent: CLIAgent,
    discovered: Option<&Path>,
    snapshot: CLIAgentLaunchSnapshot<'_>,
    is_executable: impl Fn(&Path) -> bool,
) -> Option<String> {
    if !matches!(agent, CLIAgent::Claude | CLIAgent::Codex | CLIAgent::Grok)
        || !snapshot.host_namespace_verified
        || !snapshot.command_snapshot_complete
        || snapshot.command_known
        // Fish 的现有 PATH 快照可能把数组用空格连接；PowerShell 缺少完整命令快照。
        || !matches!(snapshot.shell_type, ShellType::Bash | ShellType::Zsh)
    {
        return None;
    }
    let path = snapshot.path?;
    for directory in std::env::split_paths(path) {
        let directory = if directory.is_absolute() {
            directory
        } else {
            let cwd = snapshot.cwd.filter(|cwd| cwd.is_absolute())?;
            cwd.join(directory)
        };
        if is_executable(&directory.join(agent.command_prefix())) {
            return None;
        }
    }
    let discovered = discovered.filter(|path| path.is_absolute())?;
    // 不将另一个 CLI 的安装路径错误套用到当前启动意图。
    let basename = discovered.file_name()?.to_str()?;
    if !agent.command_prefixes().contains(&basename) || !is_executable(discovered) {
        return None;
    }
    Some(shell_quote_arg(discovered.to_str()?, snapshot.shell_type))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CLIAgentVersionStatus {
    NotInstalled,
    NotProbed,
    Detected(String),
    /// 程序存在，但执行失败、超时或输出不是该 CLI 的版本。
    Unknown,
}

/// 缓存本机安装信息；远端会话不能将该缓存视为远端 CLI 的能力证明。
pub struct CLIAgentInstallModel {
    cache: Option<HashMap<CLIAgent, CLIAgentInstallation>>,
    scan_generation: u64,
}

impl CLIAgentInstallModel {
    #[cfg(test)]
    pub(crate) fn with_installation_for_test(
        agent: CLIAgent,
        installation: CLIAgentInstallation,
    ) -> Self {
        Self {
            cache: Some(HashMap::from([(agent, installation)])),
            scan_generation: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self {
            cache: Some(HashMap::new()),
            scan_generation: 0,
        }
    }

    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut model = Self {
            cache: None,
            scan_generation: 0,
        };
        model.refresh(ctx);
        model
    }

    /// 安装或升级之后重新扫描；旧扫描结果不会覆盖较新的请求。
    pub fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        self.scan_generation += 1;
        let generation = self.scan_generation;
        ctx.spawn(
            async move { (generation, scan_cli_agent_installations().await) },
            Self::on_scan_complete,
        );
    }

    fn on_scan_complete(
        &mut self,
        (generation, results): (u64, HashMap<CLIAgent, CLIAgentInstallation>),
        ctx: &mut ModelContext<Self>,
    ) {
        if generation != self.scan_generation {
            return;
        }
        self.cache = Some(results);
        if let Some(snapshot) = self.snapshot() {
            crate::settings::AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.sync_per_agent_from_scan(&snapshot, ctx);
            });
        }
        ctx.emit(CLIAgentInstallEvent::ScanComplete);
    }

    /// 查询安装路径和版本；扫描尚未完成时返回 None。
    pub fn installation(&self, agent: CLIAgent) -> Option<&CLIAgentInstallation> {
        self.cache.as_ref()?.get(&agent)
    }

    /// 复用安装扫描实际探测的程序路径，不因派发时 PATH 不同而换用另一份 CLI。
    pub(crate) fn executable(&self, agent: CLIAgent) -> Option<&Path> {
        self.installation(agent)?.executable.as_deref()
    }

    pub fn is_cli_agent_installed(&self, agent: CLIAgent) -> bool {
        self.installation(agent)
            .is_some_and(|installation| installation.executable.is_some())
    }

    pub fn is_scan_complete(&self) -> bool {
        self.cache.is_some()
    }

    /// 保持既有设置同步使用的安装状态接口。
    pub fn snapshot(&self) -> Option<HashMap<CLIAgent, bool>> {
        self.cache.as_ref().map(|cache| {
            cache
                .iter()
                .map(|(agent, installation)| (*agent, installation.executable.is_some()))
                .collect()
        })
    }
}

impl Entity for CLIAgentInstallModel {
    type Event = CLIAgentInstallEvent;
}

impl SingletonEntity for CLIAgentInstallModel {}

/// 复用安装、会话及设置事件同步升级条件，不在终端模型锁内执行更新操作。
#[cfg(not(target_family = "wasm"))]
pub(crate) fn init_cli_agent_updates(ctx: &mut AppContext) {
    ctx.add_singleton_model(CliAgentUpdatesModel::new);
    sync_cli_agent_update_conditions(ctx);
    ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, ctx| {
        sync_cli_agent_update_conditions(ctx);
    });
    ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |_, _, ctx| {
        sync_cli_agent_update_conditions(ctx);
    });
    #[cfg(feature = "local_fs")]
    ctx.subscribe_to_model(&LocalCLITaskCoordinator::handle(ctx), |_, _, ctx| {
        sync_cli_agent_update_conditions(ctx);
    });
    ctx.subscribe_to_model(&CLIAgentInstallModel::handle(ctx), |_, _, ctx| {
        sync_cli_agent_update_conditions(ctx);
        CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, ctx| {
            for agent in [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok] {
                updates.check_now(agent, ctx);
            }
        });
    });
    ctx.subscribe_to_model(
        &CliAgentUpdatesModel::handle(ctx),
        |_, event, ctx| match event {
            CliAgentUpdateEvent::Changed { .. } => {}
            CliAgentUpdateEvent::InstallationChanged { .. } => {
                CLIAgentInstallModel::handle(ctx).update(ctx, |model, ctx| model.refresh(ctx));
            }
        },
    );
}

#[cfg(not(target_family = "wasm"))]
fn sync_cli_agent_update_conditions(ctx: &mut AppContext) {
    let conditions = [CLIAgent::Codex, CLIAgent::Claude, CLIAgent::Grok].map(|agent| {
        let automatic = AISettings::as_ref(ctx).is_cli_agent_auto_update_enabled(agent);
        let mut busy = CLIAgentSessionsModel::as_ref(ctx).has_local_session(agent);
        #[cfg(feature = "local_fs")]
        {
            busy |= LocalCLITaskCoordinator::as_ref(ctx).cli_update_busy(agent.command_prefix());
        }
        let channel = match AISettings::as_ref(ctx).cli_agent_update_channel(agent) {
            CLIUpdateChannel::FollowInstallation => CliAgentUpdateChannel::FollowInstallation,
            CLIUpdateChannel::Latest => CliAgentUpdateChannel::Latest,
            CLIUpdateChannel::Stable => CliAgentUpdateChannel::Stable,
            CLIUpdateChannel::Alpha => CliAgentUpdateChannel::Alpha,
        };
        (agent, automatic, busy, channel)
    });
    CliAgentUpdatesModel::handle(ctx).update(ctx, |updates, ctx| {
        for (agent, automatic, busy, channel) in conditions {
            updates.configure(agent, automatic, busy, channel, ctx);
        }
    });
}

/// 更新期间阻止应用启动同款 CLI；尚未初始化升级模型的测试和入口保持原行为。
pub(crate) fn cli_agent_update_in_progress(agent: CLIAgent, ctx: &AppContext) -> bool {
    #[cfg(not(target_family = "wasm"))]
    {
        ctx.has_singleton_model::<CliAgentUpdatesModel>()
            && CliAgentUpdatesModel::as_ref(ctx).is_updating(agent)
    }
    #[cfg(target_family = "wasm")]
    {
        // Web 入口不运行本地 CLI 更新器，保持调用方相同的启动检查接口。
        let _ = (agent, ctx);
        false
    }
}

#[cfg(not(target_family = "wasm"))]
async fn scan_cli_agent_installations() -> HashMap<CLIAgent, CLIAgentInstallation> {
    let search_dirs = cli_agent_search_dirs().collect::<Vec<_>>();
    join_all(
        enum_iterator::all::<CLIAgent>()
            .filter(|agent| *agent != CLIAgent::Unknown)
            .map(|agent| {
                let executable = find_cli_agent_executable(agent, &search_dirs);
                async move {
                    let version = match executable.as_deref() {
                        None => CLIAgentVersionStatus::NotInstalled,
                        Some(path)
                            if matches!(
                                agent,
                                CLIAgent::Claude | CLIAgent::Codex | CLIAgent::Grok
                            ) =>
                        {
                            probe_cli_agent_version(agent, path).await
                        }
                        Some(_) => CLIAgentVersionStatus::NotProbed,
                    };
                    (
                        agent,
                        CLIAgentInstallation {
                            executable,
                            version,
                        },
                    )
                }
            }),
    )
    .await
    .into_iter()
    .collect()
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn discover_cli_agent_executable(agent: CLIAgent) -> Option<PathBuf> {
    find_cli_agent_executable(agent, &cli_agent_search_dirs().collect::<Vec<_>>())
}

#[cfg(not(target_family = "wasm"))]
fn find_cli_agent_executable(agent: CLIAgent, search_dirs: &[PathBuf]) -> Option<PathBuf> {
    let commands = match agent {
        CLIAgent::CursorCli => &["cursor-agent"][..],
        other => other.command_prefixes(),
    };
    let executable = search_dirs.iter().find_map(|directory| {
        commands
            .iter()
            .find_map(|command| find_executable_in_dir(directory, command))
    })?;
    if executable.is_absolute() {
        Some(executable)
    } else {
        // PATH 可含相对目录；后台派发仍须绑定扫描时的工作目录。
        Some(std::env::current_dir().ok()?.join(executable))
    }
}

#[cfg(unix)]
fn find_executable_in_dir(directory: &Path, command: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let path = directory.join(command);
    let metadata = std::fs::metadata(&path).ok()?;
    (metadata.is_file() && metadata.permissions().mode() & 0o111 != 0).then_some(path)
}

#[cfg(windows)]
fn find_executable_in_dir(directory: &Path, command: &str) -> Option<PathBuf> {
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
    extensions
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| directory.join(format!("{command}{extension}")))
        .find(|path| path.is_file())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) async fn probe_cli_agent_version(
    agent: CLIAgent,
    executable: &Path,
) -> CLIAgentVersionStatus {
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(Ok(output)) = command.output().with_timeout(Duration::from_secs(3)).await else {
        return CLIAgentVersionStatus::Unknown;
    };
    if !output.status.success() {
        return CLIAgentVersionStatus::Unknown;
    }
    String::from_utf8(output.stdout)
        .ok()
        .and_then(|output| parse_cli_agent_version(agent, &output))
        .map(CLIAgentVersionStatus::Detected)
        .unwrap_or(CLIAgentVersionStatus::Unknown)
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn parse_cli_agent_version(agent: CLIAgent, output: &str) -> Option<String> {
    let output = output.trim();
    let version = match agent {
        CLIAgent::Codex => output
            .strip_prefix("codex-cli ")?
            .split_whitespace()
            .next()?,
        CLIAgent::Grok => output.strip_prefix("grok ")?.split_whitespace().next()?,
        CLIAgent::Claude => output.strip_suffix(" (Claude Code)")?,
        CLIAgent::Gemini
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::OpenCode
        | CLIAgent::Copilot
        | CLIAgent::Pi
        | CLIAgent::OhMyPi
        | CLIAgent::Auggie
        | CLIAgent::CursorCli
        | CLIAgent::Goose
        | CLIAgent::DeepSeek
        | CLIAgent::Hermes
        | CLIAgent::Vibe
        | CLIAgent::Antigravity
        | CLIAgent::Omp
        | CLIAgent::WarpTui
        | CLIAgent::Unknown => return None,
    };
    let core_version = version.split(['-', '+']).next()?;
    let components = core_version.split('.').collect::<Vec<_>>();
    (components.len() == 3
        && components.iter().all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        }))
    .then(|| version.to_string())
}

#[cfg(unix)]
fn cli_agent_search_dirs() -> impl Iterator<Item = PathBuf> {
    let mut dirs = Vec::new();

    if let Some(path_var) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path_var));
    }

    extend_common_cli_dirs(&mut dirs);
    dedupe_paths(dirs).into_iter()
}

#[cfg(unix)]
fn extend_common_cli_dirs(dirs: &mut Vec<PathBuf>) {
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/local/sbin"),
    ]);

    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };

    dirs.extend([
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
        home.join(".local/bin"),
        home.join(".grok/bin"),
    ]);

    if let Ok(node_versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        dirs.extend(
            node_versions
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("bin")),
        );
    }
}

#[cfg(unix)]
fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::with_capacity(paths.len());
    let mut deduped = Vec::with_capacity(paths.len());
    for path in paths {
        if seen.insert(path.clone()) {
            deduped.push(path);
        }
    }
    deduped
}

#[cfg(target_family = "wasm")]
async fn scan_cli_agent_installations() -> HashMap<CLIAgent, CLIAgentInstallation> {
    // 浏览器不能探测本机进程，也不能将本机缓存当成远端安装状态。
    HashMap::new()
}

#[cfg(windows)]
fn cli_agent_search_dirs() -> impl Iterator<Item = PathBuf> {
    let mut directories = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        directories.push(home.join(".grok").join("bin"));
        directories.push(home.join(".local").join("bin"));
    }
    directories.into_iter()
}

#[cfg(test)]
#[path = "cli_agent_tests.rs"]
mod tests;
