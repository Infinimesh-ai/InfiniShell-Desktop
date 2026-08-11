//! [`TerminalView`]-specific implementation for shared sessions.

use chrono::{DateTime, Local};
use itertools::Itertools;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::semantic_selection::SemanticSelection;
use warp_errors::report_error;
use warpui::r#async::Timer;
use warpui::units::IntoLines;
use warpui::{AppContext, ModelHandle, SingletonEntity, ViewContext};

use super::adapter::{Adapter, Kind, Participant};
use super::cloud_conversation_continuation::{
    CloudConversationContinuationUiState, TombstoneCta, conversation_failed_before_task_creation,
    resolve_cloud_conversation_continuation_ui_state,
};
use super::sharer::Sharer;
use super::sharer::inactivity_modal::InactivityModalEvent;
use super::viewer::Viewer;
use super::{ConversationEndedTombstoneEvent, ConversationEndedTombstoneView};
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::auth::UserUid;
use crate::context_chips::ContextChipKind;
use crate::editor::{InteractionState, ReplicaId};
use crate::menu::{MenuItem, MenuItemFields};
use crate::settings::InputModeSettings;
use crate::terminal::TerminalModel;
use crate::terminal::block_list_viewport::ScrollPositionUpdate;
use crate::terminal::model::blocks::BlockListPoint;
use crate::terminal::model::index::Point;
use crate::terminal::model::terminal_model::WithinBlock;
use crate::terminal::shared_session::participant_avatar_view::{
    ParticipantAvatarEvent, ParticipantAvatarView,
};
use crate::terminal::shared_session::presence_manager::{
    Event as PresenceManagerEvent, PresenceManager,
};
// Zap:session-sharing-protocol crate 已剥离,协议类型改用本地 protocol 模块
use crate::terminal::shared_session::protocol::{
    ParticipantId, ParticipantList, ParticipantPresenceUpdate, Role, RoleUpdateReason,
    RoleUpdatedReason, SessionEndedReason, SessionId, SessionSourceType, WindowSize,
};
use crate::terminal::shared_session::selections::point_to_session_sharing;
use crate::terminal::shared_session::settings::SharedSessionSettings;
use crate::terminal::shared_session::{
    SharedSessionActionSource, SharedSessionScrollbackType, SharedSessionSource,
    SharedSessionStatus,
};
use crate::terminal::view::{
    ContextMenuAction, Event, InlineBannerItem, InlineBannerType, PendingUserQueryKind,
    RichContentInsertionPosition, SharedSessionBanners, SizeUpdateBuilder, TerminalAction,
    TerminalView,
};
use crate::view_components::ToastFlavor;

impl TerminalView {
    pub fn sharer_session_kind(&self) -> Option<&Kind> {
        self.shared_session.as_ref().map(|s| s.kind())
    }

    pub fn sharer_session_kind_mut(&mut self) -> Option<&mut Kind> {
        self.shared_session.as_mut().map(|s| s.kind_mut())
    }

    pub fn shared_session_sharer(&self) -> Option<&Sharer> {
        self.sharer_session_kind().and_then(|k| k.as_sharer())
    }

    pub fn shared_session_sharer_mut(&mut self) -> Option<&mut Sharer> {
        self.sharer_session_kind_mut()
            .and_then(|k| k.as_sharer_mut())
    }

    pub fn shared_session_viewer(&self) -> Option<&Viewer> {
        self.sharer_session_kind().and_then(|k| k.as_viewer())
    }

    pub fn shared_session_viewer_mut(&mut self) -> Option<&mut Viewer> {
        self.sharer_session_kind_mut()
            .and_then(|k| k.as_viewer_mut())
    }

    // TODO (suraj): do we actually need to expose this? It's a bit of a smell.
    pub fn shared_session_presence_manager(&self) -> Option<ModelHandle<PresenceManager>> {
        Some(self.shared_session.as_ref()?.presence_manager().clone())
    }

    pub fn shared_session_id(&self) -> Option<&SessionId> {
        Some(self.shared_session.as_ref()?.session_id())
    }

    fn shared_session_source_type(&self) -> Option<&SessionSourceType> {
        Some(self.shared_session.as_ref()?.source_type())
    }

    pub(crate) fn is_shared_session_for_ambient_agent(&self) -> bool {
        matches!(
            self.shared_session_source_type(),
            Some(SessionSourceType::AmbientAgent { .. })
        )
    }

    pub(in crate::terminal::view) fn cloud_conversation_continuation_ui_state(
        &self,
        ctx: &AppContext,
    ) -> Option<CloudConversationContinuationUiState> {
        let task_id = {
            let model = self.model.lock();
            if !FeatureFlag::CloudModeSetupV2.is_enabled()
                || !FeatureFlag::HandoffCloudCloud.is_enabled()
                || model.is_receiving_agent_conversation_replay()
            {
                return None;
            }

            let is_cloud_conversation_selection = model.is_shared_ambient_agent_session()
                || model.is_conversation_transcript_viewer()
                || self
                    .ambient_agent_view_model
                    .as_ref()
                    .is_some_and(|model| model.as_ref(ctx).is_ambient_agent());
            if !is_cloud_conversation_selection {
                return None;
            }

            self.ambient_agent_task_id_for_details_panel_from_model(&model, ctx)
        };
        let Some(task_id) = task_id else {
            return conversation_failed_before_task_creation(
                self.id(),
                BlocklistAIHistoryModel::as_ref(ctx),
            )
            .then_some(CloudConversationContinuationUiState::Tombstone { cta: None });
        };
        match resolve_cloud_conversation_continuation_ui_state(self.id(), task_id, ctx) {
            Ok(state) => Some(state),
            Err(error) => error
                .should_fallback_to_tombstone()
                .then_some(CloudConversationContinuationUiState::Tombstone { cta: None }),
        }
    }

    pub(in crate::terminal::view) fn blocks_cloud_followups_for_ambient_agent_session_from_model(
        &self,
        model: &TerminalModel,
        ctx: &AppContext,
    ) -> bool {
        // Zap:`AmbientAgentViewModel::blocks_cloud_followups` 未随上游引入
        //(本地 ambient 模型为精简版),仅按任务数据判断。
        let Some(task_id) = self.ambient_agent_task_id_for_details_panel_from_model(model, ctx)
        else {
            return false;
        };

        AgentConversationsModel::as_ref(ctx)
            .get_task_data(&task_id)
            .is_some_and(|task| task.blocks_cloud_followups())
    }

    pub(crate) fn owned_ambient_agent_task_id(
        &self,
        ctx: &AppContext,
    ) -> Option<AmbientAgentTaskId> {
        let task_id = self.ambient_agent_task_id_for_details_panel(ctx)?;
        self.is_current_user_creator_of_ambient_task(task_id, ctx)
            .then_some(task_id)
    }

    fn is_current_user_creator_of_ambient_task(
        &self,
        task_id: AmbientAgentTaskId,
        ctx: &AppContext,
    ) -> bool {
        let Some(current_user_uid) = self.auth_state.user_id().map(|uid| uid.as_string()) else {
            return false;
        };

        AgentConversationsModel::as_ref(ctx)
            .get_task_data(&task_id)
            .and_then(|task| task.creator.map(|creator| creator.uid))
            .is_some_and(|creator_uid| creator_uid == current_user_uid)
    }

    pub(in crate::terminal::view) fn enable_cloud_followup_input(
        &mut self,
        task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.pending_cloud_followup_task_id = Some(task_id);
        self.input.update(ctx, |input, ctx| {
            input.reset_after_cloud_followup_submission(ctx);
            input.set_input_mode_agent(true, ctx);
            input.editor().update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Editable, ctx);
            });
        });
        self.update_pane_configuration(ctx);
        ctx.notify();
    }

    /// Clears the finished/read-only state a pane accumulates when its shared session ends, so it
    /// can host a live session again. Idempotent.
    ///
    /// A failed run whose environment is retained for debugging leaves the pane read-only with an
    /// ended-conversation tombstone even though its session is still reachable; reattaching must
    /// produce a writable terminal rather than that ended-run view.
    pub(crate) fn prepare_for_live_session_reattach(&mut self, ctx: &mut ViewContext<Self>) {
        self.remove_conversation_ended_tombstone(ctx);

        {
            let mut model = self.model.lock();
            if model.shared_session_status().is_finished_viewer() {
                // The join performed by the caller moves this to `ViewPending` and then
                // `ActiveViewer`; clearing it here just lifts `TerminalModel::is_read_only`.
                model.set_shared_session_status(SharedSessionStatus::NotShared);
            }
        }

        self.input().update(ctx, |input, ctx| {
            input.editor().update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Editable, ctx);
            });
        });
        self.update_pane_configuration(ctx);
        ctx.notify();
    }

    fn enable_cloud_followup_input_after_conversation_end(
        &mut self,
        task_id: AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.remove_conversation_ended_tombstone(ctx);

        {
            let mut model = self.model.lock();
            if model.shared_session_status().is_finished_viewer() {
                model.set_shared_session_status(SharedSessionStatus::NotShared);
            }
        }

        self.enable_cloud_followup_input(task_id, ctx);
    }

    // Zap:viewer 角色切换菜单已随云端 role-change 流程移除,
    // 不引入上游的 handle_viewer_role_change_menu_event / close_viewer_role_change_menu。
    fn handle_participant_avatar_event(
        &mut self,
        event: &ParticipantAvatarEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ParticipantAvatarEvent::ScrollToSharedSessionParticipant { participant_id } => {
                self.scroll_to_shared_session_participant_selection(participant_id, ctx);
            }
            ParticipantAvatarEvent::MenuOpened { participant_id } => {
                // Ensure only one context menu is open at a time
                if let Some(shared_session) = &self.shared_session {
                    for (avatar_participant_id, participant) in shared_session.viewers() {
                        if participant_id != avatar_participant_id {
                            participant.avatar.update(ctx, |avatar, ctx| {
                                avatar.close_context_menu(ctx);
                            });
                        }
                    }
                }
            }
            // ParticipantAvatarEvent::MenuClosed is not handled in the match statement
            // since it only needs to trigger a pane header re-render which is called for every event.
            _ => {}
        }

        self.update_shared_session_pane_header(ctx);
    }

    pub fn update_role(
        &mut self,
        participant_id: ParticipantId,
        role: Role,
        ctx: &mut ViewContext<Self>,
    ) {
        self.on_participant_role_changed(&participant_id, role, ctx);
        ctx.emit(Event::UpdateRole {
            participant_id,
            role,
        });
    }

    fn refresh_input_data_for_participants(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(shared_session) = &self.shared_session else {
            return;
        };
        let presence_manager = shared_session.presence_manager().clone();
        for participant in presence_manager
            .as_ref(ctx)
            .all_present_participants()
            .cloned()
            .collect_vec()
        {
            let (input_replica_id, cursor_data) = presence_manager
                .as_ref(ctx)
                .input_data_for_participant(&participant);
            let replica_id = ReplicaId::from(input_replica_id);
            self.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_remote_peer_selection_data(&replica_id, cursor_data, ctx);
                });
            });
        }
        ctx.notify();
    }

    fn update_shared_session_pane_header(&mut self, _ctx: &mut ViewContext<Self>) {
        // Zap Phase 2a: pane-header sharing UI is gone, so the pane no
        // longer tracks `ShareableObject::Session`. The shared-session itself
        // still runs; it just doesn't surface a "share" button in the header.
    }

    // Zap:Share Session 路径已切断,下面两个方法保留签名但 no-op,
    // 不再 emit `Event::OpenShareSessionModal{,DeniedModal}`,也不再触达云端协同会话服务。
    pub fn open_share_session_modal(
        &mut self,
        _open_source: SharedSessionActionSource,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    pub fn open_share_session_denied_modal(&mut self, _ctx: &mut ViewContext<Self>) {}

    /// Focuses the view by telling the parent view to focus this session.
    /// For example, in the common case, the parent pane group would consume
    /// this event and focus the pane that this session lives in.
    pub fn focus_shared_session(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.windows().show_window_and_focus_app(ctx.window_id());
        ctx.emit(Event::FocusSession);
    }

    /// The entrypoint to start a shared session: all attempts to start a shared session must
    /// go through this API! This is important to guarantee that the right session is being shared.
    /// The TerminalView is responsible for decorating the terminal to reflect its shared status and for
    /// emitting the appropriate events for its terminal manager to setup the appropriate facilities for
    /// sharing to work.
    ///
    /// Specifically, this is the data flow to start a shared session:
    /// 1. User attempts to start a shared session (i.e. this API)
    /// 2. We emit an event that the `shared_session::sharer::Network` model (configured by TerminalManager) picks up.
    /// 3. The `Network` model attempts to establish a shared session connection
    ///    with the server. Once established, it emits an event back.
    /// 4. The TerminalManager handles this event by
    ///    a. Updating the shared session status in the TerminalModel
    ///    b. Registering the shared session with the [`shared_session::manager::Manager`]
    ///    c. Calling into [`TerminalView::on_session_share_started`]
    /// 5. Once the session is registered with [`shared_session::manager::Manager`], it
    ///    will emit an event for relevant subscribers (e.g. the Workspace will need to
    ///    re-render when a share starts for tab indicator, share button, etc.)
    // Zap:Shared Session 网络入口已切断,attempt_to_share_session 整体 no-op,
    // 不再 set SharePending 状态、不再 emit StartSharingCurrentSession、不再触发遥测。
    pub fn attempt_to_share_session(
        &mut self,
        _scrollback_type: SharedSessionScrollbackType,
        _action_source: Option<SharedSessionActionSource>,
        _source: SharedSessionSource,
        _bypass_conversation_guard: bool,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    /// Sets the PresenceManager and decorates the view accordingly when a shared session has been started.
    #[allow(clippy::too_many_arguments)]
    pub fn on_session_share_started(
        &mut self,
        sharer_id: ParticipantId,
        user_uid: UserUid,
        scrollback_type: SharedSessionScrollbackType,
        session_id: SessionId,
        source_type: SessionSourceType,
        ctx: &mut ViewContext<Self>,
    ) {
        let started_at = Local::now();
        // TODO(zap-cloud-removal Phase 5): `self_handle` 原本喂给 ShareableObject::Session
        // 用于 sharing UI 反查 pane;sharing UI 已删但 shared_session 整条链路仍在,
        // 完整退役 shared_session 时再删这个 ctx.handle() 调用。
        let _self_handle = ctx.handle();
        let adapter = Adapter::new_for_sharer(
            sharer_id,
            user_uid,
            session_id,
            started_at,
            source_type,
            ctx,
        );
        let presence_manager = adapter.presence_manager().clone();

        self.shared_session = Some(adapter);
        self.reset_sharer_inactivity_timer(ctx);
        self.input.update(ctx, |input, _| {
            input.set_shared_session_presence_manager(presence_manager);
        });
        let share_source = self.pending_share_source.take();
        let is_remote_control = matches!(share_source, Some(SharedSessionActionSource::FooterChip));
        self.insert_shared_session_started_banner(
            scrollback_type,
            is_remote_control,
            started_at,
            ctx,
        );

        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.refresh_pane_header_overflow_menu_items(ctx);
            // Zap Phase 2a: sharing dialog + pane-header `ShareableObject`
            // bookkeeping removed; the shared session continues without a UI
            // entry point.
            pane_config.notify_header_content_changed(ctx);
        });
    }

    /// The entrypoint to stop a shared session: all attempts to stop a shared session must
    /// go through this API! This is important to guarantee that we correctly stop the share.
    pub fn stop_sharing_session(&mut self, ctx: &mut ViewContext<Self>) {
        self.stop_sharing_session_for_reason(SessionEndedReason::EndedBySharer, ctx);
    }

    fn stop_sharing_session_for_reason(
        &mut self,
        reason: SessionEndedReason,
        ctx: &mut ViewContext<Self>,
    ) {
        let session_id = self.shared_session_id().cloned();
        let source_task_id = self
            .model
            .lock()
            .shared_session_source()
            .and_then(|share_source| share_source.orchestrator_task_id().map(str::to_owned));
        // Zap:上游这里还打印 `action_source={source:?}`。`source: SharedSessionActionSource`
        // 参数随停止分享的遥测事件一同删除,函数签名里已经没有这个变量,故日志里去掉该字段。
        log::info!(
            "Shared session view stop requested: session_id={session_id:?} source_task_id={source_task_id:?} reason={reason:?}"
        );
        ctx.emit(Event::StopSharingCurrentSession { reason });
    }

    // TODO: why do we need to pass through input replica ID as a separate argument?
    // It should be in `participant_list`.
    #[allow(clippy::too_many_arguments)]
    pub fn on_session_share_joined(
        &mut self,
        viewer_id: ParticipantId,
        user_uid: UserUid,
        input_replica_id: ReplicaId,
        participant_list: Box<ParticipantList>,
        session_id: SessionId,
        source_type: SessionSourceType,
        ctx: &mut ViewContext<Self>,
    ) {
        let started_at = Local::now();
        // TODO(zap-cloud-removal Phase 5): `self_handle` 原本喂给 ShareableObject::Session
        // 用于 sharing UI 反查 pane;sharing UI 已删但 shared_session 整条链路仍在,
        // 完整退役 shared_session 时再删这个 ctx.handle() 调用。
        let _self_handle = ctx.handle();
        let adapter = Adapter::new_for_viewer(
            viewer_id.clone(),
            user_uid,
            participant_list,
            session_id,
            started_at,
            source_type.clone(),
            ctx,
        );
        let presence_manager = adapter.presence_manager().clone();
        let role = presence_manager.as_ref(ctx).role();
        self.shared_session = Some(adapter);

        self.insert_shared_session_started_banner(
            SharedSessionScrollbackType::All,
            false,
            started_at,
            ctx,
        );

        self.input.update(ctx, |input, ctx| {
            input.on_session_share_joined(input_replica_id, presence_manager, ctx);
        });

        // Mark this terminal as a viewer for chips and AI context menu once on join
        let is_ambient = self.is_ambient_agent_session(ctx);
        self.input().update(ctx, |input, ctx| {
            input
                .prompt_render_helper
                .prompt_view()
                .update(ctx, |prompt_display, ctx| {
                    prompt_display.update_shared_session_viewer_status(true, ctx);
                });

            input.editor().update(ctx, |editor, ctx| {
                if let Some(ai_context_menu) = editor.ai_context_menu() {
                    ai_context_menu.update(ctx, |menu, ctx| {
                        menu.set_is_shared_session_viewer(true, ctx);
                        menu.set_is_in_ambient_agent(is_ambient, ctx);
                    });
                }
            });
        });

        // If viewer joined as an executor, make sure the view state is updated.
        if let Some(role) = role {
            self.on_self_role_updated(role, ctx);
        }

        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.refresh_pane_header_overflow_menu_items(ctx);
            // Zap Phase 2a: removed `set_shareable_object` (cloud sharing UI gone).
            pane_config.notify_header_content_changed(ctx);
        });

        // When we join a shared session, we get a snapshot of the sharer's chip states,
        // including the working directory chip. We can use this chip value to set the terminal title
        // with the correct pwd on-join (even if there is no active block yet to populate the TerminalView's pwd).
        if let Some(pwd) = self
            .current_prompt
            .as_ref(ctx)
            .latest_chip_value(&ContextChipKind::WorkingDirectory, ctx)
        {
            self.terminal_title = pwd.to_string();
        }

        // Update the pane title, which will show either the conversation title/status
        // if there's an active conversation, or fall back to the terminal_title (pwd).
        self.update_pane_configuration(ctx);

        self.update_shared_session_pane_header(ctx);
        // Zap:上游在这里按 `FeatureFlag::CloudMode` 自动展开会话详情面板并上报
        // `TelemetryEvent::JoinedSharedSession`;云端模式与遥测均已剥离,这里不做任何事。
    }

    /// Clear the presence manager and handle any UI necessary on shared session end.
    /// Applies to both sharer and viewer when the session sharing ends.
    pub fn on_session_share_ended(&mut self, ctx: &mut ViewContext<Self>) {
        let viewed_ambient_task_id = self.ambient_agent_task_id_for_details_panel(ctx);
        let handoff_continuation_state = self.cloud_conversation_continuation_ui_state(ctx);
        let should_insert_legacy_tombstone = {
            let model = self.model.lock();
            !FeatureFlag::CloudModeSetupV2.is_enabled()
                && model.is_shared_ambient_agent_session()
                && self.conversation_ended_tombstone_view_id.is_none()
                && !model.is_receiving_agent_conversation_replay()
        };
        if let Some(state) = handoff_continuation_state {
            match state {
                CloudConversationContinuationUiState::Tombstone { cta } => {
                    self.insert_conversation_ended_tombstone_with_cta(cta, ctx);
                }
                CloudConversationContinuationUiState::FollowupInput => {
                    self.remove_conversation_ended_tombstone(ctx);
                }
            }
        } else if should_insert_legacy_tombstone {
            self.insert_conversation_ended_tombstone_with_cta(None, ctx);
        }
        // Ensure inactivity timer is aborted for sharer
        if let Some(sharer) = self.shared_session_sharer_mut()
            && let Some(old_abort_handle) = sharer.inactivity_timer_abort_handle.take()
        {
            old_abort_handle.abort();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.active_viewer_driven_size.is_some() && !self.is_shared_session_for_ambient_agent() {
            self.restore_pty_to_sharer_size(ctx);
        }

        // Zap Phase 2a: 上游在这里保留 `ShareableObject::Session` 以便 ambient agent
        // 的分享弹窗继续可见;分享 UI 已随云端剥离,无需保留。
        self.shared_session = None;
        self.insert_shared_session_ended_banner(ctx);
        self.on_shared_session_reconnection_status_changed(false, ctx);

        self.input().update(ctx, |input, ctx| {
            input.editor().update(ctx, |editor, ctx| {
                editor.unregister_all_remote_peers(ctx);
            });
        });

        if self.pending_cloud_followup_task_id.is_none() {
            if matches!(
                handoff_continuation_state,
                Some(CloudConversationContinuationUiState::FollowupInput)
            ) {
                if let Some(task_id) = viewed_ambient_task_id {
                    self.enable_cloud_followup_input(task_id, ctx);
                }
            } else if self.model.lock().shared_session_status().is_viewer() {
                // When the session is ended, the input should be uneditable iff this is a viewer.
                self.input().update(ctx, |input, ctx| {
                    input.editor().update(ctx, |editor, ctx| {
                        editor.set_interaction_state(InteractionState::Selectable, ctx);
                    });
                });
            }
        }

        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.refresh_pane_header_overflow_menu_items(ctx);
            pane_config.notify_header_content_changed(ctx);
            ctx.notify();
        });
    }

    pub fn on_ambient_agent_execution_ended(&mut self, ctx: &mut ViewContext<Self>) {
        self.handle_non_running_ambient_agent_task(ctx);
    }

    fn handle_non_running_ambient_agent_task(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(task_id) = self.ambient_agent_task_id_for_details_panel(ctx) {
            AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
                model.mark_task_execution_ended(task_id, ctx);
            });
        }
        // Zap:会话详情面板(conversation_details_panel)已随云端 ambient agent 链路剥离,
        // 这里无需再刷新。
        let has_live_shared_session = {
            let status = self.model.lock().shared_session_status().clone();
            status.is_active_viewer() || status.is_active_sharer()
        };
        if has_live_shared_session {
            return;
        }
        let has_pending_cloud_followup = self.pending_cloud_followup_task_id.is_some();
        if !FeatureFlag::CloudModeSetupV2.is_enabled() || has_pending_cloud_followup {
            return;
        }
        if !FeatureFlag::HandoffCloudCloud.is_enabled() {
            self.insert_conversation_ended_tombstone_with_cta(None, ctx);
            return;
        }
        let Some(state) = self.cloud_conversation_continuation_ui_state(ctx) else {
            return;
        };
        match state {
            CloudConversationContinuationUiState::Tombstone { cta } => {
                self.insert_conversation_ended_tombstone_with_cta(cta, ctx);
            }
            CloudConversationContinuationUiState::FollowupInput => {
                if let Some(task_id) = self.ambient_agent_task_id_for_details_panel(ctx) {
                    self.enable_cloud_followup_input_after_conversation_end(task_id, ctx);
                }
            }
        }
    }

    fn start_cloud_followup_from_tombstone(
        &mut self,
        task_id: crate::ai::ambient_agents::AmbientAgentTaskId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::HandoffCloudCloud.is_enabled() {
            return;
        }

        let Some(ambient_agent_view_model) = self.ambient_agent_view_model.as_ref() else {
            self.show_error_toast("Couldn't continue this cloud task.".to_string(), ctx);
            return;
        };

        if ambient_agent_view_model.as_ref(ctx).task_id() != Some(task_id) {
            self.show_error_toast("Couldn't continue this cloud task.".to_string(), ctx);
            return;
        }
        self.enable_cloud_followup_input_after_conversation_end(task_id, ctx);
        self.focus_input_box(ctx);
        ctx.notify();
    }

    pub fn handle_inactivity_modal_event(
        &mut self,
        event: &InactivityModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(sharer) = self.shared_session_sharer_mut() else {
            return;
        };
        sharer.close_inactivity_warning_modal();
        ctx.notify();

        match event {
            InactivityModalEvent::TimedOut => self.end_session_on_inactivity_period_expired(ctx),
            InactivityModalEvent::StopSharing => self.stop_sharing_session(ctx),
            InactivityModalEvent::ContinueSharing => self.reset_sharer_inactivity_timer(ctx),
        }
    }

    fn end_session_on_inactivity_period_expired(&mut self, ctx: &mut ViewContext<Self>) {
        self.stop_sharing_session_for_reason(SessionEndedReason::InactivityLimitReached, ctx);
        self.show_persistent_toast(
            "Sharing ended due to inactivity".to_owned(),
            ToastFlavor::Error,
            ctx,
        );
    }

    fn show_warning_on_inactivity_period_expired(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(sharer) = self.shared_session_sharer_mut() else {
            return;
        };
        // Ensure warning modal isn't already open
        if !sharer.is_inactivity_warning_modal_open {
            sharer.open_inactivity_warning_modal(ctx);
            ctx.notify();
        }
    }

    fn set_inactivity_timer_to_show_warning(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(sharer) = self.shared_session_sharer_mut() else {
            return;
        };

        // After the second interval of inactivity, we display a warning modal
        let inactivity_period = SharedSessionSettings::as_ref(ctx)
            .inactivity_period_between_revoking_roles_and_warning();
        let timer_handler = ctx.spawn_abortable(
            Timer::after(inactivity_period),
            move |me, _, ctx| me.show_warning_on_inactivity_period_expired(ctx),
            |_, _| {},
        );
        sharer.inactivity_timer_abort_handle = Some(timer_handler);
    }

    fn revoke_roles_on_inactivity_period_expired(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(shared_session) = self.shared_session.as_mut() else {
            return;
        };

        // Ensure executors exist
        let num_executors = shared_session.presence_manager().read(ctx, |manager, _| {
            manager
                .get_present_viewers()
                .filter(|viewer| viewer.role.is_some_and(|r| r.can_execute()))
                .count()
        });
        if num_executors > 0 {
            self.make_all_shared_session_participants_readers(
                RoleUpdateReason::InactivityLimitReached,
                ctx,
            );
            self.show_persistent_toast(
                "Shared editing permissions were revoked due to inactivity".to_owned(),
                ToastFlavor::Error,
                ctx,
            );
        }

        // Set timer for second interval
        self.set_inactivity_timer_to_show_warning(ctx);
    }

    /// Resets sharer's inactivity timer
    /// (1) After the first interval, we revoke all executor permissions
    /// (2) After the second interval, we show a warning modal
    /// (3) After the third interval, we end the session
    pub fn reset_sharer_inactivity_timer(&mut self, ctx: &mut ViewContext<Self>) {
        // For ambient agent shared sessions, we do not auto-revoke roles or end the
        // session due to inactivity. Clear any existing timer and return early so
        // the session stays open until explicitly closed.
        if self.model.lock().is_shared_ambient_agent_session() {
            if let Some(sharer) = self.shared_session_sharer_mut()
                && let Some(old_abort_handle) = sharer.inactivity_timer_abort_handle.take()
            {
                old_abort_handle.abort();
            }
            return;
        }

        let Some(sharer) = self.shared_session_sharer_mut() else {
            return;
        };

        // Ignore timer resets from throttled activity when warning modal is open.
        // User must explicitly close modal to continue the session.
        if sharer.is_inactivity_warning_modal_open {
            return;
        }

        if let Some(old_abort_handle) = sharer.inactivity_timer_abort_handle.take() {
            old_abort_handle.abort();
        }

        // After the first interval of inactivity, we revoke all executor permissions
        let inactivity_period = SharedSessionSettings::as_ref(ctx)
            .inactivity_period_before_revoking_roles
            .value();
        let timer_handler = ctx.spawn_abortable(
            Timer::after(*inactivity_period),
            move |me, _, ctx| me.revoke_roles_on_inactivity_period_expired(ctx),
            |_, _| {},
        );
        sharer.inactivity_timer_abort_handle = Some(timer_handler);
    }

    pub fn get_shared_session_presence_selection(
        &self,
        ctx: &AppContext,
    ) -> crate::terminal::shared_session::protocol::Selection {
        let model_lock = self.model.lock();
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        let semantic_selection = SemanticSelection::as_ref(ctx);

        // First check if we have any selected blocks.
        let selected_block_ids = self
            .selected_blocks
            .to_block_ids(model_lock.block_list())
            .map(|id| id.to_string().into())
            .collect_vec();
        if !selected_block_ids.is_empty() {
            return crate::terminal::shared_session::protocol::Selection::Blocks {
                block_ids: selected_block_ids,
            };
        }

        // Then check if we have selected text in the alt screen or block list.
        if model_lock.is_alt_screen_active() {
            if let Some(selection_range) =
                model_lock.alt_screen().selection_range(semantic_selection)
            {
                return crate::terminal::shared_session::protocol::Selection::AltScreenText {
                    start: point_to_session_sharing(*selection_range.start()),
                    end: point_to_session_sharing(*selection_range.end()),
                    is_reversed: selection_range.is_reversed(),
                };
            }
        } else if let Some((start, end, is_reversed)) = model_lock
            .block_list()
            .text_selection_range(semantic_selection, input_mode.is_inverted_blocklist())
        {
            let Some(start) = start.to_session_sharing_block_point(model_lock.block_list()) else {
                report_error!("Failed convert start of selection range to BlockPoint");
                return crate::terminal::shared_session::protocol::Selection::None;
            };
            let Some(end) = end.to_session_sharing_block_point(model_lock.block_list()) else {
                report_error!("Failed convert end of selection range to BlockPoint");
                return crate::terminal::shared_session::protocol::Selection::None;
            };
            return crate::terminal::shared_session::protocol::Selection::BlockText {
                start,
                end,
                is_reversed,
            };
        }
        crate::terminal::shared_session::protocol::Selection::None
    }

    pub fn handle_presence_manager_event(
        &mut self,
        event: &PresenceManagerEvent,
        presence_manager: ModelHandle<PresenceManager>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(shared_session) = self.shared_session.as_mut() else {
            log::warn!("Received presence manager event for a session that isn't shared");
            return;
        };

        match event {
            // TODO(suraj): improve the diff approach.
            PresenceManagerEvent::ParticipantListUpdated => {
                // Make sure all the absent viewers have been removed.
                for viewer in presence_manager
                    .as_ref(ctx)
                    .absent_viewers()
                    .cloned()
                    .collect_vec()
                {
                    if !shared_session.viewers().contains_key(viewer.id()) {
                        continue;
                    }

                    shared_session.remove_viewer(viewer.id());
                    self.input.update(ctx, |input, ctx| {
                        input.editor().update(ctx, |editor, ctx| {
                            let replica_id = (viewer.input_replica_id().clone()).into();
                            editor.unregister_remote_peer(&replica_id, ctx);
                        });
                    });
                }

                // Make sure all the active viewers are added.
                let active_viewers = presence_manager
                    .as_ref(ctx)
                    .get_present_viewers()
                    .cloned()
                    .collect_vec();
                let is_self_sharer = shared_session.kind().is_sharer();
                let is_reconnecting = presence_manager.as_ref(ctx).is_reconnecting();
                for viewer in active_viewers {
                    if let Some(existing_viewer) = shared_session.viewers().get(viewer.id()) {
                        // A change to the viewer's ACL may have originated from
                        // warp-server, so we need to update the avatar's role.
                        existing_viewer.avatar.update(ctx, |avatar, ctx| {
                            if avatar.role() != viewer.role {
                                avatar.set_role(viewer.role);
                                ctx.notify();
                            }
                        });
                        continue;
                    }

                    let pane_header_avatar = ctx.add_typed_action_view(|ctx| {
                        ParticipantAvatarView::new(
                            is_self_sharer,
                            viewer.info.clone(),
                            viewer.color,
                            is_reconnecting,
                            viewer.role,
                            ctx,
                        )
                    });
                    ctx.subscribe_to_view(&pane_header_avatar, |me, _, event, ctx| {
                        me.handle_participant_avatar_event(event, ctx);
                    });
                    shared_session.add_viewer(viewer.id().to_owned(), pane_header_avatar);

                    let (input_replica_id, cursor_data) = presence_manager
                        .as_ref(ctx)
                        .input_data_for_participant(&viewer);
                    self.input.update(ctx, |input, ctx| {
                        input.editor().update(ctx, |editor, ctx| {
                            editor.register_remote_peer(input_replica_id.into(), cursor_data, ctx);
                        });
                    });
                }

                if let Some(sharer) = presence_manager.as_ref(ctx).get_sharer().cloned() {
                    if let Kind::Viewer(v) = shared_session.kind_mut() {
                        let pane_header_avatar = ctx.add_typed_action_view(|ctx| {
                            ParticipantAvatarView::new(
                                is_self_sharer,
                                sharer.info.clone(),
                                sharer.color,
                                is_reconnecting,
                                None,
                                ctx,
                            )
                        });
                        ctx.subscribe_to_view(&pane_header_avatar, |me, _, event, ctx| {
                            me.handle_participant_avatar_event(event, ctx);
                        });
                        v.sharer = Some(Participant::new(pane_header_avatar));
                    }

                    let (input_replica_id, cursor_data) = presence_manager
                        .as_ref(ctx)
                        .input_data_for_participant(&sharer);
                    self.input.update(ctx, |input, ctx| {
                        input.editor().update(ctx, |editor, ctx| {
                            editor.register_remote_peer(input_replica_id.into(), cursor_data, ctx);
                        });
                    });
                }
            }
        }

        self.update_shared_session_pane_header(ctx);

        // Notify the pane header that its content has changed and needs to re-render.
        self.pane_configuration.update(ctx, |config, ctx| {
            config.notify_header_content_changed(ctx);
        });
    }

    fn scroll_to_shared_session_participant_selection(
        &mut self,
        participant_id: &ParticipantId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(participant) = self
            .shared_session_presence_manager()
            .as_ref()
            .and_then(|pm| pm.as_ref(ctx).get_participant(participant_id))
        else {
            return;
        };

        // If we the participant has block(s) selected, scroll to the block where the avatar is.
        // Otherwise, if the participant has block text selected, scroll so the cursor is in view.
        if let Some(block_index) =
            { participant.get_selected_block_index_for_avatar(self.model.lock().block_list()) }
        {
            self.update_scroll_position_locking(
                ScrollPositionUpdate::ScrollToTopOfBlockWithBuffer {
                    block_index,
                    buffer_lines: 2.into_lines(),
                },
                ctx,
            );
        } else if let crate::terminal::shared_session::protocol::Selection::BlockText {
            start,
            end,
            is_reversed,
        } = &participant.info.selection
        {
            let cursor_point = if *is_reversed { start } else { end };
            let Some(within_block_point) = WithinBlock::<Point>::from_session_sharing_block_point(
                cursor_point.clone(),
                self.model.lock().block_list(),
            ) else {
                return;
            };
            let block_list_point = BlockListPoint::from_within_block_point(
                &within_block_point,
                self.model.lock().block_list(),
            );
            self.update_scroll_position_locking(
                ScrollPositionUpdate::ScrollToBlocklistRowIfNotVisible {
                    row: block_list_point.row.into_lines(),
                },
                ctx,
            );
        }
    }

    // If open, ensure that participant avatar context menu is not triggered
    pub fn pane_header_overflow_menu_toggled(
        &mut self,
        is_open: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(shared_session) = self.shared_session.as_mut() {
            for viewer in shared_session.viewers().values() {
                viewer.avatar.update(ctx, |avatar, _| {
                    avatar.set_is_pane_header_overflow_menu_open(is_open);
                });
            }
        }
    }

    pub fn make_all_shared_session_participants_readers(
        &mut self,
        reason: RoleUpdateReason,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(shared_session) = self.shared_session.as_mut() {
            if !shared_session.kind().is_sharer() {
                return;
            }

            shared_session
                .presence_manager()
                .update(ctx, |manager, ctx| {
                    manager.make_all_participants_readers(ctx);
                });

            for viewer in shared_session.viewers().values() {
                viewer.avatar.update(ctx, |avatar, ctx| {
                    avatar.set_role(Some(Role::Reader));
                    ctx.notify();
                });
            }
        }

        self.update_shared_session_pane_header(ctx);
        log::warn!("Ignoring removed shared session revoke-all network update: {reason:?}");
    }

    // Called when viewer receives acknowledgment from server
    // on role request status (in flight, or failed)
    /// Updates view state when our own role was changed.
    fn on_self_role_updated(&mut self, role: Role, ctx: &mut ViewContext<Self>) {
        // Update shared session status only if we are an active viewer.
        // This avoids a race condition if a viewer receives a role change
        // before catching up, by ensuring the view is still pending.
        if self.model.lock().shared_session_status().is_active_viewer() {
            // If not an active viewer now, role and status will be updated
            // in the call `process_ordered_terminal_event`.
            self.model
                .lock()
                .set_shared_session_status(SharedSessionStatus::ActiveViewer { role });
        }

        // Enable/disable the editor based on the new role
        self.input().update(ctx, |input, ctx| {
            input.editor().update(ctx, |editor, ctx| {
                let role = &role;
                editor.set_interaction_state(role.into(), ctx);
            });
            // Role gates whether prompts can be sent, so the queued prompts panel's
            // send-now buttons and enter hint must re-sync.
            if let Some(panel) = input.queued_prompts_panel().cloned() {
                panel.update(ctx, |panel, ctx| {
                    panel.set_can_send_prompt(role.can_execute(), ctx);
                });
            }
        });
    }

    // Zap:上游在这里定义了 `on_shared_session_role_request_response`(依赖已删除的
    // `shared_session::role_change_modal`)、`copy_shared_session_link`(依赖已删除的
    // `shared_session::manager::Manager` 与 `join_link`)以及
    // `open_shared_session_qr_code`(依赖已删除的分享对话框 UI)。三者随云端分享链路一并移除。

    fn insert_shared_session_started_banner(
        &mut self,
        scrollback_type: SharedSessionScrollbackType,
        is_remote_control: bool,
        started_at: DateTime<Local>,
        ctx: &mut ViewContext<Self>,
    ) {
        let banner_id = self.inline_banners_state.next_banner_id();

        let mut model = self.model.lock();

        // TODO: technically the first block index could change between the time we insert
        // the banner and the time we actually compute the scrollback.
        let block_index = scrollback_type.first_block_index(&model);

        // Remove any existing banners if any.
        if let SharedSessionBanners::LastShared {
            started_banner_id,
            ended_banner_id,
            ..
        } = self.inline_banners_state.shared_session_banner_state
        {
            model
                .block_list_mut()
                .remove_inline_banner(started_banner_id);
            model.block_list_mut().remove_inline_banner(ended_banner_id);
        }

        self.inline_banners_state.shared_session_banner_state = SharedSessionBanners::ActiveShare {
            started_banner_id: banner_id,
            started_at,
            is_remote_control,
        };

        model.block_list_mut().insert_inline_banner_before_block(
            block_index,
            InlineBannerItem::new(banner_id, InlineBannerType::SharedSessionStart),
            None,
        );

        ctx.notify();
    }

    pub fn on_participant_presence_updated(
        &mut self,
        update: &ParticipantPresenceUpdate,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(presence_manager) = &self.shared_session_presence_manager() else {
            return;
        };
        let input_data = presence_manager.update(ctx, |manager, ctx| {
            manager.update_participant_presence(update.to_owned(), ctx);
            manager
                .get_participant(&update.participant_id)
                .map(|participant| manager.input_data_for_participant(participant))
        });

        if let Some((input_replica_id, cursor_data)) = input_data {
            let replica_id = ReplicaId::from(input_replica_id);
            self.input.update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_remote_peer_selection_data(&replica_id, cursor_data, ctx);
                });
            });
        }
        ctx.notify();
    }

    /// Only show toast if role is new and reason is valid.
    pub fn maybe_show_role_changed_toast(
        &mut self,
        participant_id: &ParticipantId,
        reason: RoleUpdatedReason,
        new_role: Role,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(presence_manager) = self.shared_session_presence_manager() else {
            return;
        };
        let is_self_role_updated = participant_id == &presence_manager.as_ref(ctx).id();
        let is_new_role_reader = match presence_manager.as_ref(ctx).role() {
            Some(old_role) => old_role.can_execute() && matches!(new_role, Role::Reader),
            None => false,
        };

        if is_self_role_updated
            && is_new_role_reader
            && matches!(reason, RoleUpdatedReason::InactivityLimitReached)
        {
            self.show_persistent_toast(
                "Editing permissions were revoked because the sharer is idle".to_owned(),
                ToastFlavor::Error,
                ctx,
            );
        }
    }

    // Called by both sharer and viewer when a participant's role has changed.
    pub fn on_participant_role_changed(
        &mut self,
        participant_id: &ParticipantId,
        new_role: Role,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(shared_session) = self.shared_session.as_mut() {
            shared_session.update_participant_role(participant_id, new_role, ctx);

            let is_self = *participant_id == shared_session.presence_manager().as_ref(ctx).id();
            if is_self {
                self.on_self_role_updated(new_role, ctx);
            }
        }
        self.update_shared_session_pane_header(ctx);
    }

    pub fn on_self_role_maybe_changed(
        &mut self,
        participant_list: &ParticipantList,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(shared_session) = self.shared_session.as_ref() else {
            return;
        };
        let presence_manager = shared_session.presence_manager().as_ref(ctx);
        let self_id = presence_manager.id();
        let Some(existing_role) = presence_manager.role() else {
            return;
        };

        let Some(new_role) = participant_list
            .present_viewers
            .iter()
            .find(|v| v.info.id == self_id)
            .map(|v| v.max_acl)
        else {
            log::warn!("Could not find new role for viewer {self_id:?} in participant list");
            return;
        };

        if existing_role != new_role {
            self.on_self_role_updated(new_role, ctx);
        }
    }

    pub fn insert_shared_session_ended_banner(&mut self, ctx: &mut ViewContext<Self>) {
        let banner_id = self.inline_banners_state.next_banner_id();
        let banner = InlineBannerItem::new(banner_id, InlineBannerType::SharedSessionEnd);

        if let SharedSessionBanners::ActiveShare {
            started_banner_id,
            started_at,
            is_remote_control,
        } = self.inline_banners_state.shared_session_banner_state
        {
            self.inline_banners_state.shared_session_banner_state =
                SharedSessionBanners::LastShared {
                    started_banner_id,
                    started_at,
                    is_remote_control,
                    ended_at: Local::now(),
                    ended_banner_id: banner_id,
                };
        }

        let mut model = self.model.lock();
        if model.shared_session_status().is_active_viewer() {
            // For viewers, the banner goes after the long running block so no content appears after the banner.
            model
                .block_list_mut()
                .append_inline_banner_after_long_running(banner);
        } else {
            // For sharers, it goes before the long running block so the banner doesn't end up pinned at the bottom while the block above changes.
            model.block_list_mut().append_inline_banner(banner);
        }

        ctx.notify();
    }

    pub(crate) fn insert_conversation_ended_tombstone_with_cta(
        &mut self,
        tombstone_cta: Option<TombstoneCta>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.conversation_ended_tombstone_view_id.is_some() {
            self.remove_conversation_ended_tombstone(ctx);
        }
        let task_id = self.ambient_agent_task_id_for_details_panel(ctx);
        let terminal_view_id = self.id();

        let tombstone_view_handle = ctx.add_typed_action_view(|ctx| {
            ConversationEndedTombstoneView::new(ctx, terminal_view_id, task_id, tombstone_cta)
        });
        ctx.subscribe_to_view(&tombstone_view_handle, |me, _, event, ctx| match event {
            ConversationEndedTombstoneEvent::ContinueInCloud { task_id } => {
                me.start_cloud_followup_from_tombstone(*task_id, ctx);
            }
        });
        let tombstone_view_id = tombstone_view_handle.id();
        // The cloud-mode queued-prompt block is pinned to the bottom so it stays below any
        // streaming agent output. When inserting the conversation-ended tombstone we want the
        // tombstone below the queued prompt instead, so unpin the queued prompt first.
        if self.pending_user_query_kind == Some(PendingUserQueryKind::CloudMode)
            && let Some(pending_query_view_id) = self.pending_user_query_view_id
        {
            self.model
                .lock()
                .block_list_mut()
                .unpin_rich_content_from_bottom(pending_query_view_id);
        }
        let insertion_position = self
            .pending_user_query_view_id
            .map(RichContentInsertionPosition::AfterRichContent)
            .unwrap_or(RichContentInsertionPosition::Append {
                insert_below_long_running_block: true,
            });
        self.insert_rich_content(None, tombstone_view_handle, None, insertion_position, ctx);
        self.conversation_ended_tombstone_view_id = Some(tombstone_view_id);
    }

    pub(crate) fn insert_conversation_ended_tombstone_with_resolved_cta(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::HandoffCloudCloud.is_enabled() {
            self.insert_conversation_ended_tombstone_with_cta(None, ctx);
            return;
        }

        match self.cloud_conversation_continuation_ui_state(ctx) {
            Some(CloudConversationContinuationUiState::Tombstone { cta }) => {
                self.insert_conversation_ended_tombstone_with_cta(cta, ctx);
            }
            Some(CloudConversationContinuationUiState::FollowupInput) => {
                if let Some(task_id) = self.ambient_agent_task_id_for_details_panel(ctx) {
                    self.enable_cloud_followup_input_after_conversation_end(task_id, ctx);
                } else {
                    self.insert_conversation_ended_tombstone_with_cta(None, ctx);
                }
            }
            None => {
                self.insert_conversation_ended_tombstone_with_cta(None, ctx);
            }
        }
    }

    pub(in crate::terminal::view) fn remove_conversation_ended_tombstone(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(view_id) = self.conversation_ended_tombstone_view_id.take() else {
            return;
        };
        self.model
            .lock()
            .block_list_mut()
            .remove_rich_content(view_id);
        self.rich_content_views.retain(|rc| rc.view_id() != view_id);
        ctx.notify();
    }

    /// Updates shared session reconnection banner, participant avatars and
    /// input interaction state depending on the reconnection state.
    pub fn on_shared_session_reconnection_status_changed(
        &mut self,
        is_reconnecting: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if is_reconnecting
            && !self
                .model
                .lock()
                .shared_session_status()
                .is_sharer_or_viewer()
        {
            log::warn!(
                "Tried to open shared session reconnecting banner for a session that isn't shared"
            );
            return;
        }

        if let Some(shared_session) = self.shared_session.as_mut() {
            shared_session.on_reconnection_status_changed(is_reconnecting, ctx);
        }

        // Input is disabled for an offline executor and re-enabled when back online.
        if self.model.lock().shared_session_status().is_executor() {
            let interaction_state = if is_reconnecting {
                InteractionState::Selectable
            } else {
                InteractionState::Editable
            };
            self.input().update(ctx, |input, ctx| {
                input.editor().update(ctx, |editor, ctx| {
                    editor.set_interaction_state(interaction_state, ctx);
                });
            });
        }

        self.refresh_input_data_for_participants(ctx);
        self.update_shared_session_pane_header(ctx);
        ctx.notify();
    }

    pub fn session_sharing_context_menu_items(
        &self,
        model: &TerminalModel,
        is_share_session_disabled: bool,
        has_session_link: bool,
    ) -> Vec<MenuItem<TerminalAction>> {
        let mut items = Vec::new();

        // Zap:分享 UI 已随云端剥离,这两个入参只保留签名兼容。
        let _ = is_share_session_disabled;
        let _ = has_session_link;
        if model.shared_session_status().is_active_sharer() {
            items.push(
                MenuItemFields::new(crate::t!("terminal-stop-sharing"))
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::StopSharing,
                    ))
                    .into_item(),
            );
        }

        // Zap:上游在这里加入 "Copy session sharing link" 菜单项
        // (`TerminalAction::CopySharedSessionLink`),分享链接链路已删除。
        items
    }

    /// Resizes the terminal from when the sharer updates size.
    pub fn resize_from_sharer_update(
        &mut self,
        new_sharer_size: WindowSize,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(viewer) = self.shared_session_viewer_mut() {
            viewer.sharer_size = Some(new_sharer_size);

            let size_update = SizeUpdateBuilder::for_shared_session_update(
                *self.size_info,
                new_sharer_size.num_rows,
                new_sharer_size.num_cols,
            )
            .build(self, ctx);
            self.resize_internal(size_update, ctx);
        }
    }

    /// Returns true if viewer-driven sizing should be active.
    /// For ambient-agent sessions, the same-user identity check is skipped.
    /// Otherwise, conditions: exactly 1 viewer, and that viewer is the same user as the sharer.
    pub(crate) fn is_viewer_driven_sizing_eligible(
        &self,
        is_sharer: bool,
        ctx: &ViewContext<Self>,
    ) -> bool {
        let skip_uid_check = self.is_shared_session_for_ambient_agent();
        self.shared_session_presence_manager()
            .map(|manager| {
                let manager = manager.as_ref(ctx);
                if is_sharer {
                    manager
                        .single_distinct_present_viewer_uid()
                        .is_some_and(|viewer_uid| {
                            skip_uid_check || viewer_uid == manager.user_uid().as_str()
                        })
                } else {
                    // No other distinct user should be viewing.
                    // Stale copies of our own connection share our UID.
                    let no_other_user = manager
                        .get_present_viewers()
                        .all(|v| v.info.profile_data.user_uid == manager.user_uid().as_string());
                    no_other_user
                        && (skip_uid_check
                            || manager.get_sharer().is_some_and(|s| {
                                s.info.profile_data.user_uid == manager.user_uid().as_string()
                            }))
                }
            })
            .unwrap_or(false)
    }

    /// Restores the PTY to the sharer's own terminal size by refreshing
    /// through the normal resize pipeline.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn restore_pty_to_sharer_size(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_viewer_driven_size = None;
        self.refresh_size(ctx);
    }

    /// Forces a fresh viewer-size report to the sharer by clearing the dedup cache and
    /// refreshing size. No-op when not an active viewer or when viewer-driven sizing is
    /// not eligible. Used when a new process (e.g. the harness CLI starting for a non-oz
    /// ambient-agent run) needs the sharer to resize its PTY so the new process picks up
    /// correct terminal dimensions at startup.
    pub(in crate::terminal::view) fn force_report_viewer_terminal_size(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(viewer) = self.shared_session_viewer_mut() {
            viewer.last_reported_natural_size = None;
        }
        self.refresh_size(ctx);
    }

    /// Resizes the sharer's terminal to match the viewer's reported size,
    /// going through the normal view/model/PTY resize pipeline.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn resize_from_viewer_report(
        &mut self,
        viewer_size: WindowSize,
        ctx: &mut ViewContext<Self>,
    ) {
        self.active_viewer_driven_size = Some((viewer_size.num_rows, viewer_size.num_cols));
        let size_update = SizeUpdateBuilder::for_viewer_size_report(
            *self.size_info,
            viewer_size.num_rows,
            viewer_size.num_cols,
        )
        .build(self, ctx);
        self.resize_internal(size_update, ctx);
    }
}

#[cfg(test)]
#[path = "view_impl_tests.rs"]
mod tests;
