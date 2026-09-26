//! 每条宿主命令独立监督；只有真实清理收据才能释放当前命令与延迟回合事件。

use std::time::{Duration, Instant};

use super::reviewed_project_commands::{ReviewedCommand, reject};
use super::{RuntimeError, RuntimeEventKind};

#[derive(Default)]
pub(super) struct ReviewedCommandLifecycle {
    active: Option<(String, String, Instant)>,
    retiring: bool,
    deferred: Vec<RuntimeEventKind>,
}

impl ReviewedCommandLifecycle {
    pub(super) fn can_approve(&self) -> bool {
        self.active.is_none() && !self.retiring
    }

    pub(super) fn start_host(
        &mut self,
        turn: String,
        tool_call: String,
        command: &ReviewedCommand,
    ) -> Result<(), RuntimeError> {
        if !self.can_approve() || tool_call.is_empty() || turn.is_empty() {
            return Err(reject("reviewed_commands_previous_cleanup_pending"));
        }
        // 执行器管理命令期限；额外预算仅用于派生和收集完整清理回执。
        self.active = Some((
            turn,
            tool_call,
            Instant::now() + Duration::from_millis(command.timeout_ms) + Duration::from_secs(60),
        ));
        Ok(())
    }

    pub(super) fn has_command(&self) -> bool {
        self.active.is_some()
    }

    pub(super) fn turn(&self) -> Option<&str> {
        self.active.as_ref().map(|(turn, _, _)| turn.as_str())
    }

    pub(super) fn timed_out(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(_, _, deadline)| Instant::now() >= *deadline)
    }

    pub(super) fn owns_tool(&self, tool_call: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(_, expected, _)| expected == tool_call)
    }

    pub(super) fn retire(&mut self) {
        self.retiring = true;
    }
    pub(super) fn retiring(&self) -> bool {
        self.retiring
    }

    pub(super) fn before_publish(&mut self, event: RuntimeEventKind) -> Option<RuntimeEventKind> {
        if let RuntimeEventKind::TurnFinished { turn_id, .. } = &event {
            if self.turn() == Some(turn_id.as_str()) {
                self.deferred.push(event);
                return None;
            }
        }
        Some(event)
    }

    pub(super) fn host_cleanup_confirmed(
        &mut self,
        turn: &str,
        call: &str,
    ) -> Result<Vec<RuntimeEventKind>, RuntimeError> {
        if !self
            .active
            .as_ref()
            .is_some_and(|(active_turn, active_call, _)| active_turn == turn && active_call == call)
        {
            return Err(reject("reviewed_commands_cleanup_identity_mismatch"));
        }
        // 仅宿主执行 future 在核验自身独立进程树的真实退出后调用；工具结果不可触发。
        self.active = None;
        Ok(std::mem::take(&mut self.deferred))
    }

    pub(super) fn cleanup_confirmed(&mut self) -> Vec<RuntimeEventKind> {
        // 父 CLI 和全部独立子命令均已确认清理后，才发布仍被保留的终态。
        self.active = None;
        std::mem::take(&mut self.deferred)
    }
}
