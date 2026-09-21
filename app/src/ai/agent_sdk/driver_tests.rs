use std::time::Duration;

use futures::channel::oneshot;

use super::IdleTimeoutSender;

#[test]
fn idle_timeout_sender_complete_with_zero_idle_sends_immediately() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_after(Duration::from_millis(50), 1);

    idle_timeout.complete_with_optional_idle(Some(Duration::ZERO), 7);

    assert_eq!(rx.try_recv().unwrap(), Some(7));
}
