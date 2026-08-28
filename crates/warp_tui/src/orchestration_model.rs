//! TUI 的本地子代理编排与导航状态。

use std::collections::HashMap;
use std::path::PathBuf;

use warp::tui_export::{
    AIConversation, AIConversationId, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
    ConversationStatus, Harness, LoadedConversationData, RenderableAIError,
    StartAgentExecutionMode, StartAgentRequest, apply_child_agent_model_override,
    descendant_conversation_ids_in_spawn_order, descendant_conversations_in_pill_order,
    inherit_child_agent_settings, orchestration_root_conversation_id,
    prepare_local_oz_child_launch,
};
use warpui::SingletonEntity;
use warpui_core::{AppContext, Entity, EntityId, ModelContext, ModelHandle, ViewHandle};

use crate::session_registry::{TuiSessionId, TuiSessions};
use crate::tab_bar::TuiTabBarPagingState;
use crate::terminal_session_view::TuiTerminalSessionView;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiOrchestrationChild {
    pub(crate) conversation_id: AIConversationId,
    pub(crate) label: String,
    pub(crate) spawn_index: usize,
    pub(crate) status: ConversationStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TuiOrchestrationSnapshot {
    pub(crate) root_conversation_id: AIConversationId,
    pub(crate) selected_conversation_id: AIConversationId,
    pub(crate) children: Vec<TuiOrchestrationChild>,
    pub(crate) page_anchor: Option<AIConversationId>,
    pub(crate) reveal_selected: bool,
}

pub(crate) struct TuiOrchestrationModel {
    child_session_by_conversation: HashMap<AIConversationId, TuiSessionId>,
    tab_bar_paging: TuiTabBarPagingState<AIConversationId>,
}

#[allow(clippy::enum_variant_names)]
pub(crate) enum TuiOrchestrationEvent {
    CreateLocalChildSession {
        parent_session_id: TuiSessionId,
        request: Box<StartAgentRequest>,
        model_id: Option<String>,
        working_directory: Option<PathBuf>,
        task_id: warp::tui_export::AmbientAgentTaskId,
        conversation_name: String,
    },
    KillLocalChildSession {
        session_id: TuiSessionId,
        conversation_id: AIConversationId,
    },
    RemoveChildSession(TuiSessionId),
    RestoreLocalChildSession {
        root_session_id: TuiSessionId,
        conversation: Box<AIConversation>,
    },
}

pub(crate) struct MaterializedLocalOzChildSession {
    pub(crate) parent_session_id: TuiSessionId,
    pub(crate) session_id: TuiSessionId,
    pub(crate) session_view: ViewHandle<TuiTerminalSessionView>,
    pub(crate) request: StartAgentRequest,
    pub(crate) model_id: Option<String>,
    pub(crate) task_id: warp::tui_export::AmbientAgentTaskId,
    pub(crate) conversation_name: String,
}

impl Entity for TuiOrchestrationModel {
    type Event = TuiOrchestrationEvent;
}

impl SingletonEntity for TuiOrchestrationModel {}

impl TuiOrchestrationModel {
    pub(crate) fn register(ctx: &mut AppContext) -> ModelHandle<Self> {
        let history = BlocklistAIHistoryModel::handle(ctx);
        let model = ctx.add_singleton_model(|_| Self {
            child_session_by_conversation: HashMap::new(),
            tab_bar_paging: TuiTabBarPagingState::default(),
        });
        let model_for_history = model.clone();
        ctx.subscribe_to_model(&history, move |_, event, ctx| {
            let topology_changed = match event {
                BlocklistAIHistoryEvent::StartedNewConversation { .. }
                | BlocklistAIHistoryEvent::AppendedExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
                | BlocklistAIHistoryEvent::ClearedConversationsForTerminalSurface { .. }
                | BlocklistAIHistoryEvent::SplitConversation { .. }
                | BlocklistAIHistoryEvent::RemoveConversation { .. }
                | BlocklistAIHistoryEvent::DeletedConversation { .. }
                | BlocklistAIHistoryEvent::RestoredConversations { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
                | BlocklistAIHistoryEvent::ConversationTransferredBetweenTerminalSurfaces {
                    ..
                } => true,
                BlocklistAIHistoryEvent::CreatedSubtask { .. }
                | BlocklistAIHistoryEvent::UpgradedTask { .. }
                | BlocklistAIHistoryEvent::ReassignedExchange { .. }
                | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
                | BlocklistAIHistoryEvent::SetActiveConversation { .. }
                | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
                | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
                | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationTitle { .. }
                | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
                | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
                | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
                | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
                | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
                | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => false,
            };
            if topology_changed {
                model_for_history.update(ctx, |model, ctx| model.topology_changed(ctx));
            }
        });
        model
    }

    pub(crate) fn snapshot(
        &self,
        selected_conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> Option<TuiOrchestrationSnapshot> {
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let root_conversation_id =
            orchestration_root_conversation_id(history, selected_conversation_id)?;
        let session_ids_by_conversation =
            TuiSessions::as_ref(ctx).session_ids_by_conversation(history);
        session_ids_by_conversation.get(&root_conversation_id)?;

        let children = descendant_conversations_in_pill_order(history, root_conversation_id)
            .into_iter()
            .filter_map(|descendant| {
                let conversation_id = descendant.conversation_id;
                session_ids_by_conversation.get(&conversation_id)?;
                let conversation = history.conversation(&conversation_id)?;
                Some(TuiOrchestrationChild {
                    conversation_id,
                    label: conversation
                        .agent_name()
                        .filter(|name| !name.is_empty())
                        .unwrap_or(warp::t_static!("tui-agent"))
                        .to_owned(),
                    spawn_index: descendant.spawn_index,
                    status: conversation.status().clone(),
                })
            })
            .collect::<Vec<_>>();
        if children.is_empty() {
            return None;
        }

        let resolved_page = self.tab_bar_paging.resolve(
            children.first().map(|child| child.conversation_id),
            |anchor| {
                children
                    .iter()
                    .any(|child| child.conversation_id == *anchor)
            },
        );
        Some(TuiOrchestrationSnapshot {
            root_conversation_id,
            selected_conversation_id,
            children,
            page_anchor: resolved_page.page_anchor,
            reveal_selected: resolved_page.reveal_selected,
        })
    }

    pub(crate) fn set_explicit_page(
        &mut self,
        page_anchor: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.tab_bar_paging.set_explicit_anchor(page_anchor);
        ctx.notify();
    }

    pub(crate) fn focus_conversation_session(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiSessionId> {
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        orchestration_root_conversation_id(history, conversation_id)?;
        let session_id = *TuiSessions::as_ref(ctx)
            .session_ids_by_conversation(history)
            .get(&conversation_id)?;
        self.tab_bar_paging.clear_explicit_anchor();
        TuiSessions::handle(ctx).update(ctx, |sessions, ctx| {
            sessions.focus_session(session_id, ctx);
        });
        Some(session_id)
    }

    fn topology_changed(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.notify();
    }

    pub(crate) fn dispatch_create_agent(
        &mut self,
        parent_session_id: TuiSessionId,
        request: StartAgentRequest,
        working_directory: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        match request.execution_mode.clone() {
            StartAgentExecutionMode::Local {
                harness_type: None,
                model_id,
            } => self.begin_local_oz_child_launch(
                parent_session_id,
                request,
                model_id,
                working_directory,
                ctx,
            ),
            StartAgentExecutionMode::Local {
                harness_type: Some(harness_type),
                ..
            } => self.fail_child_request(
                &request,
                warp::t!(
                    "tui-local-harness-child-unsupported",
                    harness = harness_type
                ),
                ctx,
            ),
            StartAgentExecutionMode::Remote { .. } => {
                self.fail_child_request(&request, warp::t!("tui-remote-child-unavailable"), ctx)
            }
        }
    }

    fn begin_local_oz_child_launch(
        &mut self,
        parent_session_id: TuiSessionId,
        request: StartAgentRequest,
        model_id: Option<String>,
        working_directory: Option<PathBuf>,
        ctx: &mut ModelContext<Self>,
    ) {
        let launch = prepare_local_oz_child_launch(
            &request.name,
            &request.prompt,
            request.parent_run_id.as_deref(),
            ctx,
        );
        ctx.spawn(launch, move |model, result, ctx| match result {
            Ok(prepared) => ctx.emit(TuiOrchestrationEvent::CreateLocalChildSession {
                parent_session_id,
                request: Box::new(request),
                model_id,
                working_directory,
                task_id: prepared.task_id,
                conversation_name: prepared.conversation_name,
            }),
            Err(error) => model.fail_child_request(
                &request,
                warp::t!("tui-local-child-create-failed", error = error.to_string()),
                ctx,
            ),
        });
    }

    pub(crate) fn register_local_oz_child_session(
        &mut self,
        child: MaterializedLocalOzChildSession,
        ctx: &mut ModelContext<Self>,
    ) {
        let MaterializedLocalOzChildSession {
            parent_session_id,
            session_id,
            session_view,
            request,
            model_id,
            task_id,
            conversation_name,
        } = child;
        let child_surface_id = session_id.surface_id();
        inherit_child_agent_settings(parent_session_id.surface_id(), child_surface_id, ctx);
        apply_child_agent_model_override(child_surface_id, model_id.as_deref(), ctx);

        let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_child_conversation(
                child_surface_id,
                conversation_name,
                request.parent_conversation_id,
                Some(Harness::Oz),
                ctx,
            );
            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                conversation.set_task_id(task_id);
            }
            history.set_active_conversation_id(conversation_id, child_surface_id, ctx);
            history.record_new_conversation_request_complete(request.id, conversation_id, ctx);
            conversation_id
        });

        session_view.update(ctx, |view, ctx| {
            view.initialize_orchestrated_child_conversation(conversation_id, ctx);
        });
        session_view.update(ctx, |view, ctx| {
            view.start_orchestrated_child(task_id, request.prompt, conversation_id, ctx);
        });
        self.child_session_by_conversation
            .insert(conversation_id, session_id);
        ctx.notify();
    }

    pub(crate) fn restore_descendant_sessions(
        &mut self,
        parent_conversation_id: AIConversationId,
        root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let descendant_ids = descendant_conversation_ids_in_spawn_order(
            BlocklistAIHistoryModel::as_ref(ctx),
            parent_conversation_id,
        );
        for descendant_id in descendant_ids {
            self.restore_descendant_child(
                parent_conversation_id,
                descendant_id,
                root_session_id,
                ctx,
            );
        }
        ctx.notify();
    }

    pub(crate) fn discard_restored_descendant_sessions(
        &mut self,
        previous_parent_conversation_id: AIConversationId,
        _root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let descendant_ids = descendant_conversation_ids_in_spawn_order(
            BlocklistAIHistoryModel::as_ref(ctx),
            previous_parent_conversation_id,
        );
        for descendant_id in descendant_ids {
            if let Some(session_id) = self.child_session_by_conversation.remove(&descendant_id) {
                ctx.emit(TuiOrchestrationEvent::RemoveChildSession(session_id));
            }
        }
        ctx.notify();
    }

    fn restore_descendant_child(
        &mut self,
        root_parent_conversation_id: AIConversationId,
        conversation_id: AIConversationId,
        root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.is_child_already_materialized(conversation_id, ctx) {
            return;
        }
        match BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .cloned()
        {
            Some(conversation) => {
                self.emit_restore_child_session(conversation, root_session_id, ctx);
            }
            None => self.load_and_restore_descendant_child(
                root_parent_conversation_id,
                conversation_id,
                root_session_id,
                ctx,
            ),
        }
    }

    fn is_child_already_materialized(
        &self,
        conversation_id: AIConversationId,
        ctx: &AppContext,
    ) -> bool {
        self.child_session_by_conversation
            .contains_key(&conversation_id)
            || TuiSessions::as_ref(ctx)
                .session_ids_by_conversation(BlocklistAIHistoryModel::as_ref(ctx))
                .contains_key(&conversation_id)
    }

    fn load_and_restore_descendant_child(
        &mut self,
        root_parent_conversation_id: AIConversationId,
        conversation_id: AIConversationId,
        root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let future = BlocklistAIHistoryModel::as_ref(ctx).load_conversation_data(conversation_id);
        ctx.spawn(future, move |model, result, ctx| {
            if TuiSessions::as_ref(ctx).session(root_session_id).is_none()
                || model.is_child_already_materialized(conversation_id, ctx)
                || !descendant_conversation_ids_in_spawn_order(
                    BlocklistAIHistoryModel::as_ref(ctx),
                    root_parent_conversation_id,
                )
                .contains(&conversation_id)
            {
                return;
            }
            match result {
                Some(LoadedConversationData::Oz(conversation)) => {
                    model.emit_restore_child_session(*conversation, root_session_id, ctx);
                }
                Some(LoadedConversationData::CLIAgent(_)) | None => {
                    log::warn!(
                        "TUI restore: could not load local descendant conversation {conversation_id:?}."
                    );
                }
            }
        });
    }

    fn emit_restore_child_session(
        &mut self,
        conversation: AIConversation,
        root_session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        let conversation_id = conversation.id();
        if conversation.is_viewing_shared_session() || conversation.is_remote_child() {
            log::debug!("TUI restore: skipping unsupported child {conversation_id:?}");
            return;
        }
        match conversation.orchestration_harness() {
            None | Some(Harness::Oz) => {
                ctx.emit(TuiOrchestrationEvent::RestoreLocalChildSession {
                    root_session_id,
                    conversation: Box::new(conversation),
                });
            }
            Some(
                Harness::Claude
                | Harness::OpenCode
                | Harness::Gemini
                | Harness::Codex
                | Harness::Unknown,
            ) => {
                log::debug!("TUI restore: skipping local non-Oz child {conversation_id:?}.");
            }
        }
    }

    pub(crate) fn register_restored_local_oz_child_session(
        &mut self,
        session_id: TuiSessionId,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.child_session_by_conversation
            .insert(conversation_id, session_id);
        ctx.notify();
    }

    pub(crate) fn cleanup_child(
        &mut self,
        conversation_id: &AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_surface_id = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_surface_id_for_conversation(conversation_id);
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.delete_conversation(*conversation_id, terminal_surface_id, ctx);
        });
        if let Some(session_id) = self.child_session_by_conversation.remove(conversation_id) {
            ctx.emit(TuiOrchestrationEvent::RemoveChildSession(session_id));
        }
        ctx.notify();
    }

    pub(crate) fn kill_child_agent(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let is_in_progress = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&conversation_id)
            .is_some_and(|conversation| {
                conversation.status().is_in_progress() || conversation.status().is_blocked()
            });
        if is_in_progress
            && let Some(session_id) = self
                .child_session_by_conversation
                .get(&conversation_id)
                .copied()
        {
            ctx.emit(TuiOrchestrationEvent::KillLocalChildSession {
                session_id,
                conversation_id,
            });
            return;
        }
        self.cleanup_child(&conversation_id, ctx);
    }

    pub(crate) fn kill_descendant_agents(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ModelContext<Self>,
    ) {
        let descendant_ids = descendant_conversation_ids_in_spawn_order(
            BlocklistAIHistoryModel::as_ref(ctx),
            conversation_id,
        );
        for descendant_id in descendant_ids.into_iter().rev() {
            self.kill_child_agent(descendant_id, ctx);
        }
    }

    fn fail_child_request(
        &mut self,
        request: &StartAgentRequest,
        message: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let surface_id = EntityId::new();
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            let conversation_id = history.start_new_child_conversation(
                surface_id,
                request.name.trim().to_owned(),
                request.parent_conversation_id,
                None,
                ctx,
            );
            history.update_conversation_status_with_error(
                surface_id,
                conversation_id,
                ConversationStatus::Error,
                Some(RenderableAIError::other(message, false)),
                ctx,
            );
            history.record_new_conversation_request_complete(request.id, conversation_id, ctx);
        });
    }

    pub(crate) fn handle_session_removed(
        &mut self,
        session_id: TuiSessionId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.child_session_by_conversation
            .retain(|_, child_session_id| *child_session_id != session_id);
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "orchestration_model_tests.rs"]
mod tests;
