//! Tracks the `&` prefix mode drafting state in the local input while the user
//! writes a cloud handoff prompt, before a cloud pane/model exists.

use warpui::{Entity, ModelContext};

use crate::server::ids::SyncId;

/// The entry point through which a handoff compose session was initiated.
///
/// Zap:上游把这个类型定义在 `ai::ambient_agents::telemetry` 里,该遥测模块随云端
/// 上报链路一起删除。入口来源本身仍被输入区的 `&` 模式/底栏 chip/斜杠命令共用
/// (决定退出行为与提示文案),因此把类型下沉到它唯一的状态持有者这里。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub enum HandoffEntryPoint {
    /// User typed `&` in the input to enter handoff compose mode.
    #[default]
    Ampersand,
    /// User used the `/handoff` slash command.
    SlashCommand,
    /// User clicked the "Hand off to cloud" chip in the footer toolbar.
    FooterChip,
    /// The client automatically initiated handoff for an eligible local agent.
    Automatic,
}

#[derive(Clone)]
pub enum HandoffComposeStateEvent {
    ActiveChanged,
    EnvironmentSelected,
}

/// Transient state owned by the local input while drafting a cloud handoff
/// prompt (the `&` prefix mode), before a cloud pane exists.
#[derive(Default)]
pub struct HandoffComposeState {
    active: bool,
    selected_environment_id: Option<SyncId>,
    has_explicit_environment_selection: bool,
    entry_point: HandoffEntryPoint,
}

impl HandoffComposeState {
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn activate(
        &mut self,
        entry_point: HandoffEntryPoint,
        ctx: &mut ModelContext<Self>,
    ) {
        self.active = true;
        self.has_explicit_environment_selection = false;
        self.entry_point = entry_point;
        ctx.emit(HandoffComposeStateEvent::ActiveChanged);
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn entry_point(&self) -> HandoffEntryPoint {
        self.entry_point
    }

    pub(crate) fn exit(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.active && !self.has_explicit_environment_selection {
            return;
        }

        self.active = false;
        self.has_explicit_environment_selection = false;
        ctx.emit(HandoffComposeStateEvent::ActiveChanged);
    }

    pub(crate) fn selected_environment_id(&self) -> Option<&SyncId> {
        self.selected_environment_id.as_ref()
    }

    pub(crate) fn set_environment_id(
        &mut self,
        environment_id: Option<SyncId>,
        is_explicit: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // Async/implicit updates (e.g. pwd-based overlap resolution) must not
        // overwrite an environment the user already picked explicitly.
        if !is_explicit && self.has_explicit_environment_selection {
            return;
        }

        // No-op when the value is unchanged, unless this is the first explicit
        // selection (which needs to promote `has_explicit_environment_selection`).
        if self.selected_environment_id == environment_id
            && (!is_explicit || self.has_explicit_environment_selection)
        {
            return;
        }

        self.selected_environment_id = environment_id;
        if is_explicit {
            self.has_explicit_environment_selection = true;
        }
        ctx.emit(HandoffComposeStateEvent::EnvironmentSelected);
    }

    pub(crate) fn ensure_default_environment_id(
        &mut self,
        environment_id: SyncId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.selected_environment_id.is_none() {
            self.set_environment_id(Some(environment_id), false, ctx);
        }
    }
}

impl Entity for HandoffComposeState {
    type Event = HandoffComposeStateEvent;
}

#[cfg(test)]
#[path = "handoff_compose_tests.rs"]
mod tests;
