use std::collections::HashMap;
use std::path::PathBuf;

use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity};

use super::core::subscribe_to_shared_dependencies;
use super::{
    InlineItem, SlashCommandDataSource, SlashCommandDataSourceState, UpdatedActiveCommands,
};
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::ai::blocklist::block::cli_controller::CLISubagentController;
use crate::search::SyncDataSource;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::slash_command_menu::StaticCommand;
use crate::search::slash_command_menu::static_commands::Availability;
use crate::search::slash_command_menu::static_commands::commands::{self, COMMAND_REGISTRY};
use crate::settings::{InputSettings, InputSettingsChangedEvent};
use crate::terminal::input::slash_commands::AcceptSlashCommandOrSavedPrompt;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;

pub struct GuiDataSourceArgs {
    pub active_session: ModelHandle<ActiveSession>,
    pub agent_view_controller: ModelHandle<AgentViewController>,
    pub cli_subagent_controller: ModelHandle<CLISubagentController>,
    pub terminal_view_id: EntityId,
    pub ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
}

pub struct GuiSlashCommandDataSource {
    state: SlashCommandDataSourceState,
    agent_view_controller: ModelHandle<AgentViewController>,
    ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
    is_cloud_mode_v2: bool,
}

impl GuiSlashCommandDataSource {
    pub fn new(args: GuiDataSourceArgs, ctx: &mut ModelContext<Self>) -> Self {
        Self::build(args, false, ctx)
    }

    pub fn for_cloud_mode_v2(args: GuiDataSourceArgs, ctx: &mut ModelContext<Self>) -> Self {
        Self::build(args, true, ctx)
    }

    fn build(
        args: GuiDataSourceArgs,
        is_cloud_mode_v2: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let GuiDataSourceArgs {
            active_session,
            agent_view_controller,
            cli_subagent_controller,
            terminal_view_id,
            ambient_agent_view_model,
        } = args;

        subscribe_to_shared_dependencies(
            &active_session,
            &cli_subagent_controller,
            terminal_view_id,
            Self::recompute_active_commands,
            ctx,
        );
        ctx.subscribe_to_model(&agent_view_controller, |me, _, event, ctx| match event {
            AgentViewControllerEvent::EnteredAgentView { .. }
            | AgentViewControllerEvent::ExitedAgentView { .. } => {
                me.recompute_active_commands(ctx);
            }
            _ => (),
        });
        // Preserve the existing GUI subscriptions whose settings affect GUI-only command gates.
        //
        // Zap:上游这里还订阅了 `PrivacySettingsChangedEvent::UpdateIsCloudConversationStorageEnabled`
        // (云端会话存储开关)。该设置项在本地优先分支不存在,故剥离。
        ctx.subscribe_to_model(&InputSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(
                event,
                InputSettingsChangedEvent::EnableSlashCommandsInTerminal { .. }
            ) {
                me.recompute_active_commands(ctx);
            }
        });

        let mut me = Self {
            state: SlashCommandDataSourceState::new(
                active_session,
                cli_subagent_controller,
                terminal_view_id,
            ),
            agent_view_controller,
            ambient_agent_view_model: None,
            is_cloud_mode_v2,
        };
        // Route ambient wiring through the setter so construction and the lazy shared-session
        // viewer path share one implementation.
        if let Some(ambient_agent_view_model) = ambient_agent_view_model {
            me.set_ambient_agent_view_model(ambient_agent_view_model, ctx);
        } else {
            me.recompute_active_commands(ctx);
        }
        me
    }

    /// Attaches an ambient agent view model after construction. Used on the shared-session viewer
    /// path where the model is created lazily at `SessionJoined`, after the data source was built
    /// with `None`. Keeps cloud-mode command and skill gating correct for a link-join viewer.
    /// Idempotent: a no-op when a model is already set.
    pub fn set_ambient_agent_view_model(
        &mut self,
        ambient_agent_view_model: ModelHandle<AmbientAgentViewModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.ambient_agent_view_model.is_some() {
            return;
        }
        self.ambient_agent_view_model = Some(ambient_agent_view_model);
        self.recompute_active_commands(ctx);
    }

    pub(super) fn is_cloud_mode_v2(&self) -> bool {
        self.is_cloud_mode_v2
    }

    pub fn is_agent_view_active(&self, ctx: &AppContext) -> bool {
        self.agent_view_controller.as_ref(ctx).is_active()
    }

    pub fn set_active_repo_root(
        &mut self,
        repo_root: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.update_active_repo_root(repo_root) {
            self.recompute_active_commands(ctx);
        }
    }

    /// Zap:上游用 `StaticCommand::supports_gui()` 过滤掉 TUI-only 命令。我方命令注册表尚未
    /// 随上游拆出 `SlashCommandSurfaces`,注册表里全部是 GUI 命令,故省去该过滤。
    pub(crate) fn command_is_active(&self, command: &StaticCommand, ctx: &AppContext) -> bool {
        let availability = self.availability(ctx);
        let gates = self.common_command_gates(ctx);
        command.supports_gui()
            && self.command_passes_common_gates(command, availability, &gates)
            && self.command_passes_gui_gates(command)
    }

    fn recompute_active_commands(&mut self, ctx: &mut ModelContext<Self>) {
        let availability = self.availability(ctx);
        let gates = self.common_command_gates(ctx);
        let commands = HashMap::from_iter(
            COMMAND_REGISTRY
                .all_commands_by_id()
                .filter(|(_, command)| {
                    command.supports_gui()
                        && self.command_passes_common_gates(command, availability, &gates)
                        && self.command_passes_gui_gates(command)
                })
                .map(|(id, command)| (id, command.clone())),
        );
        if self.replace_active_commands(commands) {
            ctx.emit(UpdatedActiveCommands);
        }
    }

    fn availability(&self, ctx: &AppContext) -> Availability {
        let is_agent_view_active = self.is_agent_view_active(ctx);
        let mut availability =
            self.base_availability(ctx) | Self::view_availability(is_agent_view_active);

        if self.has_active_conversation(is_agent_view_active, ctx) {
            availability |= Availability::ACTIVE_CONVERSATION;
        }

        if self.is_cloud_mode_v2_composer() {
            availability |= Availability::CLOUD_MODE_V2_COMPOSER;
        }

        if self.is_cloud_mode(ctx) {
            availability |= Availability::CLOUD_AGENT;
        } else {
            availability |= Availability::NOT_CLOUD_AGENT;
        }

        availability
    }

    /// View-related availability bits for the GUI's legacy terminal-view and agent-view
    /// modalities. When the AgentView feature flag is disabled, both bits are set so either
    /// requirement is satisfied.
    fn view_availability(is_agent_view_active: bool) -> Availability {
        if !FeatureFlag::AgentView.is_enabled() {
            Availability::AGENT_VIEW | Availability::TERMINAL_VIEW
        } else if is_agent_view_active {
            Availability::AGENT_VIEW
        } else {
            Availability::TERMINAL_VIEW
        }
    }

    /// Zap:上游在这里还会隐藏 `/continue-locally`(仅对云端 Oz 会话可见)。cloud→local
    /// 交接链路在本地优先分支已删除,该命令与其 harness 判定一并剥离。
    fn command_passes_gui_gates(&self, command: &StaticCommand) -> bool {
        // `/fork` 在 V2 云端 composer 里不可用。
        if command.name == commands::FORK.name && self.is_cloud_mode_v2_composer() {
            return false;
        }
        true
    }

    /// Whether this data source backs the V2 cloud-mode composing input.
    fn is_cloud_mode_v2_composer(&self) -> bool {
        self.is_cloud_mode_v2 && FeatureFlag::CloudModeInputV2.is_enabled()
    }

    fn is_cloud_mode(&self, ctx: &AppContext) -> bool {
        self.is_cloud_mode_v2
            || (FeatureFlag::CloudMode.is_enabled()
                && self
                    .ambient_agent_view_model
                    .as_ref()
                    .is_some_and(|model| model.as_ref(ctx).is_ambient_agent()))
    }

    // Zap:上游在这里还有 `active_conversation_id` / `active_conversation_is_cloud_oz`,
    // 用于把 `/continue-locally` 限定到云端 Oz 会话。cloud→local 交接链路已删除,
    // 两个 helper 没有其它调用点,故一并删除。
}

impl SyncDataSource for GuiSlashCommandDataSource {
    type Action = AcceptSlashCommandOrSavedPrompt;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        if query.text.is_empty() {
            return Ok(vec![]);
        }

        let query_text = query.text.trim().to_lowercase();
        let mut results = self.match_active_commands(&query_text, app);
        // Skills invoke locally, so they're hidden on any cloud pane (live viewer,
        // disconnected follow-up, or read-only tombstone).
        if !self.is_cloud_mode(app) {
            results.extend(self.match_skills(&query_text, app));
        }

        Ok(results
            .into_iter()
            .map(|item: InlineItem| item.with_compact_layout(self.is_cloud_mode_v2).into())
            .collect())
    }
}

impl SlashCommandDataSource for GuiSlashCommandDataSource {
    fn state(&self) -> &SlashCommandDataSourceState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut SlashCommandDataSourceState {
        &mut self.state
    }
}
impl Entity for GuiSlashCommandDataSource {
    type Event = UpdatedActiveCommands;
}
