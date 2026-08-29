//! GUI(`TerminalView`)侧的本地 PTY 终端表面构造。
//!
//! Zap:上游这个文件里还有一整套 ~1700 行的 **sharer 侧会话共享网络接线**
//! (`wire_up_terminal_view_session_sharing` / `start_sharing_session` /
//! `wire_up_session_sharer_with_view` / `handle_network_status_events` 等),
//! 全部依赖已被物理删除的模块:`session_sharing_protocol` crate、
//! `terminal::shared_session::{manager, permissions_manager, replay_agent_conversations,
//! shared_handlers, sharer::network}`。
//!
//! Zap 自己的会话共享实现落在 `terminal::view::shared_session`(`Sharer` / `Adapter` /
//! `view_impl`),其中 `attempt_to_share_session` 已是 no-op、`TerminalManager` 也不再
//! 持有 `session_sharer` 字段(见 `terminal_manager.rs` 里的同名注释),
//! 因此这里的整段 sharer 接线没有任何可用的数据源与调用点,一并删除。

use std::any::Any;
use std::sync::Arc;
use std::sync::mpsc::SyncSender;

use parking_lot::FairMutex;
#[cfg(windows)]
use warpui::ModelHandle;
use warpui::{AppContext, ViewHandle, WindowId};

use super::terminal_manager::{TerminalManager, TerminalSurfaceInit, TerminalSurfaceResult};
use crate::ai::blocklist::{InputConfig, SerializedBlockListItem};
use crate::context_chips::current_prompt::CurrentPrompt;
use crate::context_chips::prompt_type::PromptType;
use crate::pane_group::TerminalViewResources;
use crate::persistence::ModelEvent;
#[cfg(test)]
use crate::terminal::shared_session::{
    SharedSessionScrollbackType, SharedSessionSource, SharedSessionStatus,
};
use crate::terminal::view::ConversationRestorationInNewPaneType;
use crate::terminal::writeable_pty::terminal_manager_util::wire_up_remote_server_controller_with_view;
use crate::terminal::{TerminalManager as TerminalManagerTrait, TerminalModel, TerminalView};

/// Configuration for constructing the GUI terminal surface.
pub(crate) struct TerminalViewSurfaceConfig {
    pub(crate) resources: TerminalViewResources,
    pub(crate) model_event_sender: Option<SyncSender<ModelEvent>>,
    pub(crate) window_id: WindowId,
    pub(crate) initial_input_config: Option<InputConfig>,
    pub(crate) conversation_restoration: Option<ConversationRestorationInNewPaneType>,
    pub(crate) has_conversation_restoration: bool,
    pub(crate) is_historical: bool,
    pub(crate) should_use_live_appearance: bool,
    pub(crate) has_restored_command_blocks: bool,
}

/// Resolves the block list used by the GUI `TerminalView` surface.
pub(crate) fn terminal_view_restored_blocks(
    restored_blocks: Option<&Vec<SerializedBlockListItem>>,
    conversation_restoration: &Option<ConversationRestorationInNewPaneType>,
) -> Option<Vec<SerializedBlockListItem>> {
    restored_blocks
        .filter(|blocks| !blocks.is_empty())
        .cloned()
        .or_else(|| match conversation_restoration {
            Some(ConversationRestorationInNewPaneType::Historical { conversation, .. })
            | Some(ConversationRestorationInNewPaneType::Forked { conversation, .. }) => {
                Some(conversation.to_serialized_blocklist_items())
            }
            Some(ConversationRestorationInNewPaneType::Startup { conversations, .. }) => {
                let mut items: Vec<_> = conversations
                    .iter()
                    .flat_map(|c| c.to_serialized_blocklist_items())
                    .collect();
                // Because there are multiple conversations that may have interleaved timestamps, we need to sort by start_ts
                items.sort_by_key(|item| item.start_ts());
                if items.is_empty() { None } else { Some(items) }
            }
            _ => None,
        })
}

/// Creates the GUI terminal surface and its manager-owned post-wiring closure.
pub(crate) fn create_terminal_view_surface(
    config: TerminalViewSurfaceConfig,
    surface_init: TerminalSurfaceInit,
    ctx: &mut AppContext,
) -> TerminalSurfaceResult<
    TerminalView,
    impl FnOnce(&mut TerminalManager<TerminalView>, &ViewHandle<TerminalView>, &mut AppContext) + use<>,
> {
    let TerminalSurfaceInit {
        wakeups_rx,
        model_events,
        model,
        sessions,
        size_info,
        colors,
        inactive_pty_reads_rx,
    } = surface_init;
    let TerminalViewSurfaceConfig {
        resources,
        model_event_sender,
        window_id,
        initial_input_config,
        conversation_restoration,
        has_conversation_restoration,
        is_historical,
        should_use_live_appearance,
        has_restored_command_blocks,
    } = config;
    let current_prompt = ctx.add_model(|ctx| {
        CurrentPrompt::new_with_model_events(sessions.clone(), Some(&model_events), ctx)
    });
    let prompt_type = ctx.add_model(|ctx| PromptType::new_dynamic(current_prompt.clone(), ctx));
    let view = ctx.add_typed_action_view(window_id, |ctx| {
        TerminalView::new(
            resources,
            wakeups_rx,
            model_events,
            model,
            sessions,
            size_info,
            colors,
            model_event_sender,
            prompt_type.clone(),
            initial_input_config,
            conversation_restoration,
            Some(inactive_pty_reads_rx),
            false,
            ctx,
        )
    });

    TerminalSurfaceResult {
        surface: view,
        post_wire: move |terminal_manager: &mut TerminalManager<TerminalView>,
                         view: &ViewHandle<TerminalView>,
                         ctx: &mut AppContext| {
            // Append the session restoration separator to the block list if there are any
            // restored blocks (command blocks or AI conversations) to show.
            let should_show_restoration_separator = (has_conversation_restoration
                || has_restored_command_blocks)
                && !should_use_live_appearance;

            if should_show_restoration_separator {
                terminal_manager
                    .model()
                    .lock()
                    .block_list_mut()
                    .append_session_restoration_separator_to_block_list(is_historical);
            }

            // In unit tests, we know we aren't going to bootstrap a shell
            // so if we're waiting on starting a shared session until bootstrapped,
            // just attempt to start it now.
            #[cfg(test)]
            if matches!(
                terminal_manager.model().lock().shared_session_status(),
                SharedSessionStatus::SharePendingPreBootstrap { .. }
            ) {
                view.update(ctx, |view, ctx| {
                    view.attempt_to_share_session(
                        SharedSessionScrollbackType::All,
                        None,
                        SharedSessionSource::user(None),
                        false,
                        ctx,
                    )
                });
            }

            wire_up_remote_server_controller_with_view(
                &terminal_manager.remote_server_controller(),
                view,
                ctx,
            );

            // Zap:上游在这里还调用 `wire_up_terminal_view_session_sharing(...)` 把
            // sharer 侧 `Network` 句柄写回 `terminal_manager.session_sharer`。
            // 该字段与整个 sharer 网络模块都已删除,故不再接线。
        },
    }
}

impl TerminalManager<TerminalView> {
    /// Returns the PTY process id, for integration tests.
    #[cfg(feature = "integration_tests")]
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }
}

impl TerminalManagerTrait for TerminalManager<TerminalView> {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn on_view_detached(
        &self,
        detach_type: crate::pane_group::pane::DetachType,
        app: &mut AppContext,
    ) {
        let should_shutdown_pty = match detach_type {
            crate::pane_group::pane::DetachType::Closed => true,
            crate::pane_group::pane::DetachType::HiddenForClose => {
                self.model.lock().should_shutdown_pty_on_reversible_close()
            }
            crate::pane_group::pane::DetachType::Moved => false,
        };
        if should_shutdown_pty {
            self.remote_server_controller()
                .update(app, |controller, _| {
                    controller.cancel_pending_setup_for_terminal_close();
                });
            self.view.update(app, |terminal_view, ctx| {
                terminal_view.prepare_for_pty_shutdown(ctx);
            });
            // 关闭窗口时 View 事件可能在订阅被移除后才刷新，因此必须直接通知 PTY controller。
            self.shutdown_pty(app);
        }

        // 共享会话即使只是可撤销隐藏也必须立即停止，避免不可见终端继续接受远端输入。
        let is_sharer = self.model.lock().shared_session_status().is_sharer();
        if !is_sharer {
            return;
        }
        // Zap：上游还会走 `end_shared_session` 向云端 `Network` 发送结束原因。
        // sharer 侧网络模块已删除，这里只保留本地停止入口：
        // `TerminalView::stop_sharing_session` 会 emit `StopSharingCurrentSession`，
        // 由 `terminal_pane` / `pane_group` 完成剩余清理。
        log::info!(
            "Shared session local lifecycle: event=view_detached detach_type={detach_type:?}"
        );
        self.view.update(app, |terminal_view, ctx| {
            terminal_view.stop_sharing_session(ctx)
        });
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Send a Shutdown event to each PTY's event loop and waits for the
/// event loop to terminate.
/// This is needed on Windows to ensure all OpenConsole processes are
/// cleaned up before the main thread exits.
#[cfg(windows)]
pub fn shutdown_all_pty_event_loops(ctx: &mut AppContext) {
    let terminal_managers: Vec<ModelHandle<Box<dyn TerminalManagerTrait>>> = ctx.models_of_type();
    terminal_managers.into_iter().for_each(|terminal_manager| {
        terminal_manager.update(ctx, |terminal_manager, _ctx| {
            if let Some(manager) = terminal_manager
                .as_any_mut()
                .downcast_mut::<TerminalManager<TerminalView>>()
            {
                manager.shutdown_event_loop();
            }
        })
    })
}
