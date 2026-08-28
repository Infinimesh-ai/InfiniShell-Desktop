#[cfg(test)]
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::{error, fmt};

use parking_lot::FairMutex;
use warp::tui_export::{BlocklistAIInputModel, CLISubagentController, TerminalModel};
use warpui_core::keymap::Context;
use warpui_core::{
    AppContext, Entity, ModelContext, ModelHandle, ViewHandle, WeakModelHandle, WeakViewHandle,
};

use super::{
    AUTO_APPROVE_TOGGLE_BINDING_NAME, BlockingInputSource,
    DETACH_AGENT_FROM_RUNNING_COMMAND_BINDING_NAME,
};
use crate::input_mode_policy;
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::keybindings::{PLAN_TOGGLE_BINDING_NAME, binding_hint};
use crate::read_only_menu::TuiReadOnlyMenuKind;
use crate::tab_bar::TuiTabBarView;
use crate::terminal_use::{TuiInputTarget, inline_process_owns_input, tui_input_target};
use crate::transcript_view::TuiTranscriptView;
use crate::tui_cli_subagent_view::{HAND_BACK_KEY_BINDING, TAKE_CONTROL_KEY_BINDING};

const HINT_SEPARATOR: &str = " • ";
enum TuiTerminalSessionStateSource {
    Session {
        terminal_model: Weak<FairMutex<TerminalModel>>,
        cli_subagent_controller: WeakModelHandle<CLISubagentController>,
        transcript: WeakViewHandle<TuiTranscriptView>,
        input_mode: WeakModelHandle<BlocklistAIInputModel>,
        suggestions_mode: WeakModelHandle<TuiInputSuggestionsModeModel>,
        orchestration_tab_bar: WeakViewHandle<TuiTabBarView>,
    },
    #[cfg(test)]
    InputTest {
        input_mode: WeakModelHandle<BlocklistAIInputModel>,
        suggestions_mode: WeakModelHandle<TuiInputSuggestionsModeModel>,
        orchestration_tabs_available: Rc<dyn Fn(&AppContext) -> bool>,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiFirstZeroStateState {
    Pending,
    Visible,
    Dismissed,
}

/// Persistent session-owned model that resolves a live state snapshot.
///
/// The source entities remain authoritative and are held weakly to avoid
/// extending their lifetimes. Resolving on demand avoids a cached derivative
/// that could become stale while still giving the session, input, and other
/// presentation components one shared state source.
pub(crate) struct TuiTerminalSessionStateModel {
    source: TuiTerminalSessionStateSource,
    first_zero_state: TuiFirstZeroStateState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuiTerminalSessionStateResolveError {
    TerminalModel,
    CliSubagentController,
    Transcript,
    InputMode,
    SuggestionsMode,
    OrchestrationTabBar,
}

impl fmt::Display for TuiTerminalSessionStateResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::TerminalModel => warp::t!("tui-session-state-terminal-unavailable"),
            Self::CliSubagentController => {
                warp::t!("tui-session-state-cli-controller-unavailable")
            }
            Self::Transcript => warp::t!("tui-session-state-transcript-unavailable"),
            Self::InputMode => warp::t!("tui-session-state-input-mode-unavailable"),
            Self::SuggestionsMode => {
                warp::t!("tui-session-state-suggestions-mode-unavailable")
            }
            Self::OrchestrationTabBar => {
                warp::t!("tui-session-state-orchestration-tabs-unavailable")
            }
        };
        formatter.write_str(&message)
    }
}

impl error::Error for TuiTerminalSessionStateResolveError {}

fn upgrade_terminal_model(
    terminal_model: &Weak<FairMutex<TerminalModel>>,
) -> Result<Arc<FairMutex<TerminalModel>>, TuiTerminalSessionStateResolveError> {
    terminal_model
        .upgrade()
        .ok_or(TuiTerminalSessionStateResolveError::TerminalModel)
}

impl Entity for TuiTerminalSessionStateModel {
    type Event = ();
}

impl TuiTerminalSessionStateModel {
    pub(super) fn new(
        terminal_model: &Arc<FairMutex<TerminalModel>>,
        cli_subagent_controller: &ModelHandle<CLISubagentController>,
        transcript: &ViewHandle<TuiTranscriptView>,
        input_mode: &ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: &ModelHandle<TuiInputSuggestionsModeModel>,
        orchestration_tab_bar: &ViewHandle<TuiTabBarView>,
        first_zero_state: TuiFirstZeroStateState,
    ) -> Self {
        Self {
            source: TuiTerminalSessionStateSource::Session {
                terminal_model: Arc::downgrade(terminal_model),
                cli_subagent_controller: cli_subagent_controller.downgrade(),
                transcript: transcript.downgrade(),
                input_mode: input_mode.downgrade(),
                suggestions_mode: suggestions_mode.downgrade(),
                orchestration_tab_bar: orchestration_tab_bar.downgrade(),
            },
            first_zero_state,
        }
    }

    pub(crate) fn show_first_zero_state(&self) -> bool {
        self.first_zero_state == TuiFirstZeroStateState::Visible
    }

    pub(crate) fn set_first_zero_state_pending(&mut self, ctx: &mut ModelContext<Self>) {
        if self.first_zero_state != TuiFirstZeroStateState::Pending {
            self.first_zero_state = TuiFirstZeroStateState::Pending;
            ctx.notify();
        }
    }

    pub(crate) fn resolve_first_zero_state(&mut self, show: bool, ctx: &mut ModelContext<Self>) {
        if self.first_zero_state != TuiFirstZeroStateState::Pending {
            return;
        }
        self.first_zero_state = if show {
            TuiFirstZeroStateState::Visible
        } else {
            TuiFirstZeroStateState::Dismissed
        };
        ctx.notify();
    }

    pub(crate) fn dismiss_first_zero_state(&mut self, ctx: &mut ModelContext<Self>) {
        if self.first_zero_state != TuiFirstZeroStateState::Dismissed {
            self.first_zero_state = TuiFirstZeroStateState::Dismissed;
            ctx.notify();
        }
    }
    #[cfg(test)]
    pub(crate) fn new_for_input(
        input_mode: &ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: &ModelHandle<TuiInputSuggestionsModeModel>,
        orchestration_tabs_available: impl Fn(&AppContext) -> bool + 'static,
    ) -> Self {
        Self {
            source: TuiTerminalSessionStateSource::InputTest {
                input_mode: input_mode.downgrade(),
                suggestions_mode: suggestions_mode.downgrade(),
                orchestration_tabs_available: Rc::new(orchestration_tabs_available),
            },
            first_zero_state: TuiFirstZeroStateState::Dismissed,
        }
    }

    pub(crate) fn resolve(
        &self,
        ctx: &AppContext,
    ) -> Result<TuiTerminalSessionState, TuiTerminalSessionStateResolveError> {
        match &self.source {
            TuiTerminalSessionStateSource::Session {
                terminal_model,
                cli_subagent_controller,
                transcript,
                input_mode,
                suggestions_mode,
                orchestration_tab_bar,
            } => {
                let terminal_model = upgrade_terminal_model(terminal_model)?;
                let cli_subagent_controller = cli_subagent_controller
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::CliSubagentController)?;
                let transcript = transcript
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::Transcript)?;
                let input_mode = input_mode
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::InputMode)?;
                let suggestions_mode = suggestions_mode
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::SuggestionsMode)?;
                let orchestration_tab_bar = orchestration_tab_bar
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::OrchestrationTabBar)?;
                let (
                    alt_screen_active,
                    input_target,
                    user_owns_running_command,
                    can_attach_agent_to_running_command,
                    agent_is_tagged_in,
                ) = {
                    let terminal_model = terminal_model.lock();
                    let active_block = terminal_model.block_list().active_block();
                    (
                        terminal_model.is_alt_screen_active(),
                        tui_input_target(&terminal_model),
                        inline_process_owns_input(&terminal_model),
                        active_block.is_eligible_to_tag_in_agent(),
                        active_block.is_agent_tagged_in(),
                    )
                };
                let terminal_use_control = cli_subagent_controller
                    .as_ref(ctx)
                    .active_target()
                    .map(|target| target.control_state);
                let interaction = if let Some(source) =
                    transcript.as_ref(ctx).active_blocking_input_source(ctx)
                {
                    TuiInteractionState::Blocking(source)
                } else if terminal_use_control
                    .as_ref()
                    .is_some_and(|control| control.is_user_in_control())
                {
                    TuiInteractionState::Pty(TuiPtyState::UserControlledTerminalUse)
                } else if user_owns_running_command {
                    TuiInteractionState::Blocking(BlockingInputSource::LongRunningCommand)
                } else {
                    match input_target {
                        TuiInputTarget::Disabled => TuiInteractionState::StartingShell,
                        TuiInputTarget::Pty => TuiInteractionState::Pty(TuiPtyState::Process),
                        TuiInputTarget::AgentEditor => {
                            let mode = if terminal_use_control
                                .as_ref()
                                .is_some_and(|control| control.is_agent_in_control())
                            {
                                TuiComposerMode::Agent {
                                    agent_controlled_terminal_use: true,
                                }
                            } else if input_mode_policy::is_shell_mode(input_mode.as_ref(ctx)) {
                                TuiComposerMode::Shell
                            } else {
                                TuiComposerMode::Agent {
                                    agent_controlled_terminal_use: false,
                                }
                            };
                            TuiInteractionState::AgentEditor(TuiAgentEditorState {
                                mode,
                                suggestions_mode: suggestions_mode.as_ref(ctx).mode(),
                            })
                        }
                    }
                };
                let state = TuiBlockSessionState {
                    interaction,
                    transcript_is_empty: transcript.as_ref(ctx).is_empty(),
                    orchestration_available: orchestration_tab_bar.as_ref(ctx).has_tabs(),
                    plan_available: transcript.as_ref(ctx).has_toggleable_plan(ctx),
                    can_attach_agent_to_running_command,
                    agent_is_tagged_in,
                };
                Ok(if alt_screen_active {
                    TuiTerminalSessionState::AltScreen {
                        input_target,
                        state,
                    }
                } else {
                    TuiTerminalSessionState::Block(state)
                })
            }
            #[cfg(test)]
            TuiTerminalSessionStateSource::InputTest {
                input_mode,
                suggestions_mode,
                orchestration_tabs_available,
            } => {
                let input_mode = input_mode
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::InputMode)?;
                let suggestions_mode = suggestions_mode
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::SuggestionsMode)?;
                Ok(TuiTerminalSessionState::for_input(
                    input_mode_policy::is_shell_mode(input_mode.as_ref(ctx)),
                    suggestions_mode.as_ref(ctx).mode(),
                    true,
                    orchestration_tabs_available(ctx),
                ))
            }
        }
    }
}

/// The terminal surface plus its current interaction projection.
///
/// Alternate-screen commands can still expose an agent composer beneath the
/// terminal, so the surface and interaction state are represented separately.
#[derive(Clone)]
pub(crate) enum TuiTerminalSessionState {
    AltScreen {
        input_target: TuiInputTarget,
        state: TuiBlockSessionState,
    },
    Block(TuiBlockSessionState),
}

/// State available only while the block UI is the active surface.
///
/// `interaction` is exclusive, while orchestration and plan availability are
/// additive capabilities that may contribute shortcuts to a composer.
#[derive(Clone)]
pub(crate) struct TuiBlockSessionState {
    pub(super) interaction: TuiInteractionState,
    pub(super) transcript_is_empty: bool,
    pub(super) orchestration_available: bool,
    pub(super) plan_available: bool,
    pub(super) can_attach_agent_to_running_command: bool,
    pub(super) agent_is_tagged_in: bool,
}

/// The single interaction that currently owns the block UI's input area.
#[derive(Clone)]
pub(super) enum TuiInteractionState {
    Blocking(BlockingInputSource),
    StartingShell,
    AgentEditor(TuiAgentEditorState),
    Pty(TuiPtyState),
}

/// State for the agent-editor surface, which cannot coexist with blocking or PTY input.
///
/// The active inline menu separately resolves whether it or the composer owns
/// the shared editor's behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TuiAgentEditorState {
    pub(super) mode: TuiComposerMode,
    pub(super) suggestions_mode: TuiInputSuggestionsMode,
}

/// Mutually exclusive composer modes.
///
/// Agent-controlled terminal use retains the agent composer. Shell mode cannot
/// represent terminal use, and user-controlled terminal use moves to `Pty`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiComposerMode {
    Agent { agent_controlled_terminal_use: bool },
    Shell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiPtyState {
    Process,
    UserControlledTerminalUse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TuiShortcut {
    pub(crate) key: String,
    pub(crate) description: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TuiShortcutSection {
    pub(crate) title: &'static str,
    pub(crate) shortcuts: Vec<TuiShortcut>,
}

impl TuiTerminalSessionState {
    pub(super) fn with_blocking_input_source(mut self, source: BlockingInputSource) -> Self {
        let state = match &mut self {
            Self::AltScreen { state, .. } | Self::Block(state) => state,
        };
        state.interaction = TuiInteractionState::Blocking(source);
        self
    }
    fn state(&self) -> &TuiBlockSessionState {
        match self {
            Self::AltScreen { state, .. } | Self::Block(state) => state,
        }
    }

    fn interaction(&self) -> &TuiInteractionState {
        &self.state().interaction
    }

    pub(crate) fn is_alt_screen(&self) -> bool {
        matches!(self, Self::AltScreen { .. })
    }

    pub(crate) fn has_blocking_interaction(&self) -> bool {
        matches!(
            self.interaction(),
            TuiInteractionState::Blocking(source) if source.is_interactive()
        )
    }

    pub(super) fn blocking_input_source(&self) -> Option<&BlockingInputSource> {
        match self.interaction() {
            TuiInteractionState::Blocking(source) => Some(source),
            TuiInteractionState::StartingShell
            | TuiInteractionState::AgentEditor(_)
            | TuiInteractionState::Pty(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_input(
        input_is_shell: bool,
        suggestions_mode: TuiInputSuggestionsMode,
        transcript_is_empty: bool,
        orchestration_available: bool,
    ) -> Self {
        let mode = if input_is_shell {
            TuiComposerMode::Shell
        } else {
            TuiComposerMode::Agent {
                agent_controlled_terminal_use: false,
            }
        };
        Self::Block(TuiBlockSessionState {
            interaction: TuiInteractionState::AgentEditor(TuiAgentEditorState {
                mode,
                suggestions_mode,
            }),
            transcript_is_empty,
            orchestration_available,
            plan_available: false,
            can_attach_agent_to_running_command: false,
            agent_is_tagged_in: false,
        })
    }

    pub(crate) fn input_target(&self) -> TuiInputTarget {
        match self {
            Self::AltScreen { input_target, .. } => *input_target,
            Self::Block(state) => match &state.interaction {
                TuiInteractionState::Blocking(source) => match source {
                    BlockingInputSource::LongRunningCommand => TuiInputTarget::Pty,
                    BlockingInputSource::AskQuestion(_)
                    | BlockingInputSource::Permission(_)
                    | BlockingInputSource::Orchestration(_)
                    | BlockingInputSource::Handoff(_) => TuiInputTarget::Disabled,
                },
                TuiInteractionState::StartingShell => TuiInputTarget::Disabled,
                TuiInteractionState::AgentEditor(_) => TuiInputTarget::AgentEditor,
                TuiInteractionState::Pty(_) => TuiInputTarget::Pty,
            },
        }
    }

    pub(crate) fn user_owns_running_command(&self) -> bool {
        matches!(
            self.interaction(),
            TuiInteractionState::Blocking(BlockingInputSource::LongRunningCommand)
                | TuiInteractionState::Pty(TuiPtyState::UserControlledTerminalUse)
        )
    }

    pub(crate) fn orchestration_available(&self) -> bool {
        self.state().orchestration_available
    }

    pub(crate) fn plan_available(&self) -> bool {
        self.state().plan_available
    }

    pub(crate) fn can_hand_back_terminal_use(&self) -> bool {
        matches!(
            self.interaction(),
            TuiInteractionState::Pty(TuiPtyState::UserControlledTerminalUse)
        )
    }
    pub(crate) fn can_attach_agent_to_running_command(&self) -> bool {
        self.state().can_attach_agent_to_running_command
    }

    pub(crate) fn agent_is_tagged_in(&self) -> bool {
        self.state().agent_is_tagged_in
    }

    /// Returns whether composer shortcuts are active without a suggestions overlay.
    pub(crate) fn composer_shortcuts_active(&self) -> bool {
        matches!(
            self.interaction(),
            TuiInteractionState::AgentEditor(TuiAgentEditorState {
                suggestions_mode,
                ..
            }) if !suggestions_mode.is_visible()
        )
    }

    pub(crate) fn hint_text(&self) -> Option<String> {
        let state = self.state();
        let TuiInteractionState::AgentEditor(agent_editor) = &state.interaction else {
            return None;
        };
        if agent_editor.suggestions_mode.read_only_menu().is_some() {
            return None;
        }
        Some(match agent_editor.mode {
            TuiComposerMode::Shell => warp::t!("tui-shell-input-hint"),
            TuiComposerMode::Agent { .. } => {
                agent_input_hint(state.transcript_is_empty, state.orchestration_available)
            }
        })
    }

    pub(crate) fn read_only_menu(&self) -> Option<TuiReadOnlyMenuKind> {
        let TuiInteractionState::AgentEditor(agent_editor) = self.interaction() else {
            return None;
        };
        agent_editor.suggestions_mode.read_only_menu()
    }

    pub(crate) fn shortcut_sections(
        &self,
        context: &Context,
        ctx: &AppContext,
    ) -> Vec<TuiShortcutSection> {
        let state = self.state();
        let agent_editor = match &state.interaction {
            TuiInteractionState::Blocking(BlockingInputSource::LongRunningCommand) => {
                return vec![TuiShortcutSection {
                    title: warp::t_static!("tui-terminal"),
                    shortcuts: vec![TuiShortcut {
                        key: "ctrl-c".to_owned(),
                        description: warp::t_static!("tui-shortcut-interrupt-command"),
                    }],
                }];
            }
            TuiInteractionState::Blocking(
                BlockingInputSource::AskQuestion(_)
                | BlockingInputSource::Permission(_)
                | BlockingInputSource::Orchestration(_)
                | BlockingInputSource::Handoff(_),
            )
            | TuiInteractionState::StartingShell
            | TuiInteractionState::Pty(TuiPtyState::Process) => return Vec::new(),
            TuiInteractionState::Pty(TuiPtyState::UserControlledTerminalUse) => {
                return vec![TuiShortcutSection {
                    title: warp::t_static!("tui-terminal"),
                    shortcuts: vec![TuiShortcut {
                        key: HAND_BACK_KEY_BINDING.to_owned(),
                        description: warp::t_static!("tui-shortcut-hand-back-control"),
                    }],
                }];
            }
            TuiInteractionState::AgentEditor(agent_editor) => agent_editor,
        };

        let mut shortcuts = vec![TuiShortcut {
            key: "?".to_owned(),
            description: warp::t_static!("tui-shortcuts-lowercase"),
        }];
        match agent_editor.mode {
            TuiComposerMode::Agent { .. } => shortcuts.extend([
                TuiShortcut {
                    key: "/".to_owned(),
                    description: warp::t_static!("tui-commands-lowercase"),
                },
                TuiShortcut {
                    key: "!".to_owned(),
                    description: warp::t_static!("tui-shell-mode-lowercase"),
                },
                TuiShortcut {
                    key: "←".to_owned(),
                    description: warp::t_static!("tui-conversations-lowercase"),
                },
            ]),
            TuiComposerMode::Shell => shortcuts.push(TuiShortcut {
                key: "Esc".to_owned(),
                description: warp::t_static!("tui-agent-mode-lowercase"),
            }),
        }
        if matches!(agent_editor.mode, TuiComposerMode::Agent { .. }) {
            shortcuts.push(TuiShortcut {
                key: "↑".to_owned(),
                description: warp::t_static!("tui-input-history-lowercase"),
            });
        }
        if let Some(key) = binding_hint(AUTO_APPROVE_TOGGLE_BINDING_NAME, context, ctx) {
            shortcuts.push(TuiShortcut {
                key,
                description: warp::t_static!("tui-toggle-auto-approve-lowercase"),
            });
        }
        if state.plan_available
            && let Some(key) = binding_hint(PLAN_TOGGLE_BINDING_NAME, context, ctx)
        {
            shortcuts.push(TuiShortcut {
                key,
                description: warp::t_static!("tui-expand-collapse-plans-lowercase"),
            });
        }

        let mut sections = vec![TuiShortcutSection {
            title: warp::t_static!("tui-shortcuts"),
            shortcuts,
        }];
        if matches!(
            agent_editor.mode,
            TuiComposerMode::Agent {
                agent_controlled_terminal_use: true
            }
        ) {
            sections.push(TuiShortcutSection {
                title: warp::t_static!("tui-terminal-use"),
                shortcuts: vec![TuiShortcut {
                    key: TAKE_CONTROL_KEY_BINDING.to_owned(),
                    description: warp::t_static!("tui-take-control-lowercase"),
                }],
            });
        } else if state.agent_is_tagged_in
            && let Some(key) =
                binding_hint(DETACH_AGENT_FROM_RUNNING_COMMAND_BINDING_NAME, context, ctx)
        {
            sections.push(TuiShortcutSection {
                title: warp::t_static!("tui-terminal-use"),
                shortcuts: vec![TuiShortcut {
                    key,
                    description: warp::t_static!("tui-return-control-command-lowercase"),
                }],
            });
        }
        if state.orchestration_available {
            sections.push(TuiShortcutSection {
                title: warp::t_static!("tui-orchestration"),
                shortcuts: vec![TuiShortcut {
                    key: "Shift+↑".to_owned(),
                    description: warp::t_static!("tui-navigate-agents-lowercase"),
                }],
            });
        }
        sections
    }
}

fn agent_input_hint(transcript_is_empty: bool, orchestration_tabs_available: bool) -> String {
    let mut hints = Vec::with_capacity(5);
    if transcript_is_empty {
        hints.push(warp::t!("tui-hint-shortcuts"));
        if orchestration_tabs_available {
            hints.push(warp::t!("tui-hint-other-agents"));
        }
        hints.extend([
            warp::t!("tui-hint-commands"),
            warp::t!("tui-hint-conversations"),
        ]);
    } else {
        hints.push(warp::t!("tui-hint-ask-agent"));
        hints.push(warp::t!("tui-hint-shortcuts"));
        if orchestration_tabs_available {
            hints.push(warp::t!("tui-hint-other-agents"));
        }
        hints.extend([
            warp::t!("tui-hint-shell-mode"),
            warp::t!("tui-hint-commands"),
        ]);
    }
    hints.join(HINT_SEPARATOR)
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
