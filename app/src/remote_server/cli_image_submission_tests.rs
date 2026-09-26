use super::*;

fn recovery_key() -> ClaudeRecoveryKey {
    ClaudeRecoveryKey {
        client: 1,
        terminal_session: SessionId::from(7),
        terminal_epoch: Uuid::new_v4().to_string(),
        native_session: Uuid::new_v4().to_string(),
    }
}

#[test]
fn delayed_transcript_recovery_is_bounded_and_stops_on_a_terminal_receipt() {
    assert_eq!(claude_recovery_delay(0, true), Some(Duration::ZERO));
    assert_eq!(claude_recovery_delay(1, true), Some(Duration::from_secs(1)));
    assert_eq!(claude_recovery_delay(2, true), Some(Duration::from_secs(3)));
    assert_eq!(claude_recovery_delay(3, true), None);
    assert_eq!(claude_recovery_delay(usize::MAX, true), None);
    for attempt in 0..3 {
        assert_eq!(claude_recovery_delay(attempt, false), None);
    }
}

#[test]
fn native_events_during_recovery_coalesce_without_losing_the_last_query() {
    let mut registry = Registry::default();
    let key = recovery_key();
    assert!(registry.request_claude_recovery(key.clone()));
    for _ in 0..20 {
        assert!(!registry.request_claude_recovery(key.clone()));
    }
    assert_eq!(registry.claude_recoveries.len(), 1);
    assert!(registry.finish_claude_recovery(&key));
    // 没有新的原生事件时结束；Unknown 不能产生永久查询循环。
    assert!(!registry.finish_claude_recovery(&key));
    assert!(registry.claude_recoveries.is_empty());
    // 后续 Read/Stop 事件可以重新查询旧提交，不依赖 SSH 重连或新的图片操作。
    assert!(registry.request_claude_recovery(key.clone()));
    assert!(!registry.finish_claude_recovery(&key));
}

#[test]
fn recovery_coalescing_never_crosses_connection_epoch_or_native_session() {
    let mut registry = Registry::default();
    let original = recovery_key();
    let mut connection = original.clone();
    connection.client += 1;
    let mut epoch = original.clone();
    epoch.terminal_epoch = Uuid::new_v4().to_string();
    let mut native = original.clone();
    native.native_session = Uuid::new_v4().to_string();
    let mut terminal = original.clone();
    terminal.terminal_session = SessionId::from(8);
    for key in [&original, &connection, &epoch, &native, &terminal] {
        assert!(registry.request_claude_recovery(key.clone()));
    }
    assert!(!registry.request_claude_recovery(original.clone()));
    assert!(!registry.finish_claude_recovery(&connection));
    assert!(!registry.finish_claude_recovery(&epoch));
    assert!(!registry.finish_claude_recovery(&native));
    assert!(!registry.finish_claude_recovery(&terminal));
    assert!(registry.finish_claude_recovery(&original));
    assert!(!registry.finish_claude_recovery(&original));
    assert!(registry.claude_recoveries.is_empty());
}
