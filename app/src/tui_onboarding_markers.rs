use warpui::{Entity, ModelContext, SingletonEntity};

/// TUI 中只显示一次的本地引导标记。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiOnboardingMarker {
    FirstZeroState,
    FirstCreditGate,
}

/// 引导标记状态变化事件。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiOnboardingMarkersEvent {
    Loading,
    Ready,
}

/// 本地进程内的 TUI 引导标记。
///
/// Zap 没有账号云同步；标记只在当前进程内保持单调消费，避免重新引入已经删除的
/// cloud preference 与 Server API 依赖。
pub struct TuiOnboardingMarkers {
    ready: bool,
    first_zero_state_available: bool,
    first_credit_gate_available: bool,
}

impl TuiOnboardingMarkers {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self::new_ready(true, true)
    }

    fn new_ready(first_zero_state_available: bool, first_credit_gate_available: bool) -> Self {
        Self {
            ready: true,
            first_zero_state_available,
            first_credit_gate_available,
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn new_ready_for_test(
        first_zero_state_available: bool,
        first_credit_gate_available: bool,
    ) -> Self {
        Self::new_ready(first_zero_state_available, first_credit_gate_available)
    }

    pub fn load_current_account(&mut self, ctx: &mut ModelContext<Self>) {
        self.ready = true;
        ctx.emit(TuiOnboardingMarkersEvent::Ready);
        ctx.notify();
    }

    pub fn reset_for_account_transition(&mut self, ctx: &mut ModelContext<Self>) {
        self.ready = false;
        self.first_zero_state_available = false;
        self.first_credit_gate_available = false;
        ctx.emit(TuiOnboardingMarkersEvent::Loading);
        ctx.notify();
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn set_ready_for_test(
        &mut self,
        first_zero_state_available: bool,
        first_credit_gate_available: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.ready = true;
        self.first_zero_state_available = first_zero_state_available;
        self.first_credit_gate_available = first_credit_gate_available;
        ctx.emit(TuiOnboardingMarkersEvent::Ready);
        ctx.notify();
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn consume(&mut self, marker: TuiOnboardingMarker, ctx: &mut ModelContext<Self>) -> bool {
        if !self.ready {
            return false;
        }
        let available = match marker {
            TuiOnboardingMarker::FirstZeroState => &mut self.first_zero_state_available,
            TuiOnboardingMarker::FirstCreditGate => &mut self.first_credit_gate_available,
        };
        let consumed = std::mem::take(available);
        if consumed {
            ctx.notify();
        }
        consumed
    }
}

impl Entity for TuiOnboardingMarkers {
    type Event = TuiOnboardingMarkersEvent;
}

impl SingletonEntity for TuiOnboardingMarkers {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_consumed_once() {
        let mut markers = TuiOnboardingMarkers::new_ready_for_test(true, true);

        assert!(markers.first_zero_state_available);
        assert!(markers.first_credit_gate_available);
        assert!(std::mem::take(&mut markers.first_zero_state_available));
        assert!(!std::mem::take(&mut markers.first_zero_state_available));
        assert!(std::mem::take(&mut markers.first_credit_gate_available));
        assert!(!std::mem::take(&mut markers.first_credit_gate_available));
    }
}
