use super::*;
use crate::remote_server::proto::{
    TerminalBindingAck, TerminalBindingBegin, TerminalBindingCancel,
};

fn fixture() -> (Connection, SessionBootstrapped, TerminalBindingRequest) {
    let connection = Connection::new("daemon".into());
    connection.initialize();
    let epoch = Uuid::new_v4().to_string();
    let bootstrap = SessionBootstrapped {
        session_id: 42,
        shell_type: "bash".into(),
        shell_path: Some("/bin/bash".into()),
        image_staging_epoch: epoch.clone(),
        image_staging_epoch_revision: 1,
        shell_pid: Some(1),
        shell_tty: Some("/not-a-terminal".into()),
    };
    connection.bootstrap(&bootstrap);
    let request = TerminalBindingRequest {
        scope: Some(TerminalBindingScope {
            host_id: "daemon".into(),
            terminal_session_id: 42,
            terminal_epoch: epoch,
        }),
        attempt_id: Uuid::new_v4().to_string(),
        action: Some(terminal_binding_request::Action::Begin(
            TerminalBindingBegin {},
        )),
    };
    (connection, bootstrap, request)
}

fn begin_attempt(work: &Work) -> Arc<Attempt> {
    match &work.operation {
        Operation::Begin { attempt, .. } => attempt.clone(),
        Operation::Ack { .. } | Operation::Cancel | Operation::Failed => panic!("预期仅准备挑战"),
        #[cfg(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64")
        ))]
        Operation::Owned { .. } => panic!("预期仅准备挑战"),
    }
}

fn assert_failed(response: TerminalBindingResponse) {
    assert!(matches!(
        response.result,
        Some(terminal_binding_response::Result::Failed(_))
    ));
}

#[test]
fn repeated_begin_cannot_replace_the_original_attempt() {
    let (connection, _, request) = fixture();
    let original = connection.prepare(request.clone());
    let attempt = begin_attempt(&original);
    assert_failed(connection.prepare(request).execute());
    assert!(attempt.pending());
    assert!(original.retired.is_none());
}

#[test]
fn replacement_retires_pending_work_before_any_terminal_write() {
    let (connection, _, mut request) = fixture();
    let original = connection.prepare(request.clone());
    let first = begin_attempt(&original);
    request.attempt_id = Uuid::new_v4().to_string();
    let replacement = connection.prepare(request);
    assert!(!first.pending());
    assert!(begin_attempt(&replacement).pending());
    // 原候选是不存在的 TTY；撤销后执行只能给出固定失败，不接触或补写原线路。
    assert_failed(original.execute());
}

#[test]
fn stale_cancel_cannot_cancel_a_new_attempt() {
    let (connection, _, mut old) = fixture();
    let original = connection.prepare(old.clone());
    let mut new = old.clone();
    new.attempt_id = Uuid::new_v4().to_string();
    let replacement = connection.prepare(new);
    old.action = Some(terminal_binding_request::Action::Cancel(
        TerminalBindingCancel {},
    ));
    assert_failed(connection.prepare(old).execute());
    assert!(begin_attempt(&replacement).pending());
    assert!(!begin_attempt(&original).pending());
}

#[test]
fn current_cancel_revokes_before_background_cleanup() {
    let (connection, _, mut request) = fixture();
    let original = connection.prepare(request.clone());
    let attempt = begin_attempt(&original);
    request.action = Some(terminal_binding_request::Action::Cancel(
        TerminalBindingCancel {},
    ));
    let cancel = connection.prepare(request);
    assert!(!attempt.pending());
    assert_failed(original.execute());
    assert!(matches!(
        cancel.execute().result,
        Some(terminal_binding_response::Result::Cancelled(_))
    ));
}

#[test]
fn stale_bootstrap_cannot_replace_candidates_or_revoke_current_attempt() {
    let (connection, mut bootstrap, request) = fixture();
    let original = connection.prepare(request);
    bootstrap.shell_pid = Some(2);
    bootstrap.image_staging_epoch = Uuid::new_v4().to_string();
    assert!(connection.bootstrap(&bootstrap).is_none());
    assert!(begin_attempt(&original).pending());
}

#[test]
fn new_bootstrap_without_candidates_revokes_and_does_not_inherit_old_tty() {
    let (connection, mut bootstrap, mut request) = fixture();
    let original = connection.prepare(request.clone());
    bootstrap.image_staging_epoch_revision += 1;
    bootstrap.image_staging_epoch = Uuid::new_v4().to_string();
    bootstrap.shell_pid = None;
    bootstrap.shell_tty = None;
    let retired = connection.bootstrap(&bootstrap).unwrap();
    assert!(!retired.pending());
    request.scope.as_mut().unwrap().terminal_epoch = bootstrap.image_staging_epoch;
    request.attempt_id = Uuid::new_v4().to_string();
    assert_failed(connection.prepare(request).execute());
    assert_failed(original.execute());
}

#[test]
fn another_host_epoch_or_session_cannot_begin_or_cancel() {
    let (connection, _, request) = fixture();
    let original = connection.prepare(request.clone());
    for kind in 0..3 {
        let mut other = request.clone();
        let scope = other.scope.as_mut().unwrap();
        match kind {
            0 => scope.host_id = "another-daemon".into(),
            1 => scope.terminal_epoch = Uuid::new_v4().to_string(),
            2 => scope.terminal_session_id += 1,
            _ => unreachable!(),
        }
        other.action = Some(terminal_binding_request::Action::Cancel(
            TerminalBindingCancel {},
        ));
        assert_failed(connection.prepare(other.clone()).execute());
        other.attempt_id = Uuid::new_v4().to_string();
        other.action = Some(terminal_binding_request::Action::Begin(
            TerminalBindingBegin {},
        ));
        assert_failed(connection.prepare(other).execute());
        assert!(begin_attempt(&original).pending());
    }
}

#[test]
fn ack_cannot_claim_a_challenge_before_a_successful_terminal_write() {
    let (connection, _, mut request) = fixture();
    let original = connection.prepare(request.clone());
    let attempt = begin_attempt(&original);
    request.action = Some(terminal_binding_request::Action::Ack(TerminalBindingAck {
        nonce: attempt.nonce.to_string(),
    }));
    assert_failed(connection.prepare(request).execute());
    assert!(attempt.pending());
    assert!(matches!(*attempt.state.lock().unwrap(), State::Writing));
}

#[test]
fn disconnect_revokes_pending_write_and_rebootstrap_cannot_restore_connection() {
    let (connection, mut bootstrap, mut request) = fixture();
    let original = connection.prepare(request.clone());
    let retired = connection.disconnect();
    assert_eq!(retired.len(), 1);
    assert_failed(original.execute());
    bootstrap.image_staging_epoch_revision += 1;
    bootstrap.image_staging_epoch = Uuid::new_v4().to_string();
    connection.bootstrap(&bootstrap);
    connection.initialize();
    request.scope.as_mut().unwrap().terminal_epoch = bootstrap.image_staging_epoch;
    request.attempt_id = Uuid::new_v4().to_string();
    assert_failed(connection.prepare(request).execute());
}

#[test]
fn disconnect_revokes_launch_even_when_a_consumer_keeps_the_attempt() {
    let (connection, _, request) = fixture();
    let work = connection.prepare(request);
    let consumer = begin_attempt(&work);
    connection.disconnect();
    assert!(!consumer.claim_launch());
    assert_eq!(consumer.launch.load(Ordering::SeqCst), 2);
}

#[test]
fn replacement_cannot_lend_its_launch_permission_to_the_previous_consumer() {
    let (connection, _, mut request) = fixture();
    let previous = begin_attempt(&connection.prepare(request.clone()));
    request.attempt_id = Uuid::new_v4().to_string();
    let current = begin_attempt(&connection.prepare(request));
    assert!(!previous.claim_launch());
    assert!(current.claim_launch());
    assert!(!current.claim_launch());
}

#[test]
fn claimed_launch_is_never_reset_by_disconnect_or_reinitialize() {
    let (connection, _, request) = fixture();
    let consumer = begin_attempt(&connection.prepare(request));
    assert!(consumer.claim_launch());
    connection.disconnect();
    connection.initialize();
    assert_eq!(consumer.launch.load(Ordering::SeqCst), 1);
    assert!(!consumer.claim_launch());
}

#[test]
fn dropping_connection_revokes_a_retained_consumer() {
    let (connection, _, request) = fixture();
    let consumer = begin_attempt(&connection.prepare(request));
    drop(connection);
    assert!(!consumer.claim_launch());
}

#[test]
fn owned_start_cannot_consume_a_challenge_before_native_binding() {
    let (connection, _, mut request) = fixture();
    let original = connection.prepare(request.clone());
    let attempt = begin_attempt(&original);
    request.action = Some(terminal_binding_request::Action::StartOwned(
        crate::remote_server::proto::TerminalBindingStartOwned {
            opaque_binding_id: attempt.binding_id.to_string(),
            launch_id: Uuid::new_v4().to_string(),
            launch_key: Uuid::new_v4().to_string(),
            agent: crate::remote_server::proto::TerminalBindingOwnedAgent::Codex as i32,
        },
    ));
    assert_failed(connection.prepare(request).execute());
    assert_eq!(attempt.launch.load(Ordering::SeqCst), 0);
    assert!(attempt.pending());
}

/// 只构造已完成回执的身份字段，用于检查发送前的所属代次，不伪造原生状态。
fn completed_responses(
    request: &TerminalBindingRequest,
    attempt: &Attempt,
) -> [TerminalBindingResponse; 3] {
    [
        terminal_binding_response::Result::ChallengeWritten(TerminalBindingChallengeWritten {}),
        terminal_binding_response::Result::Bound(TerminalBindingBound {
            opaque_binding_id: attempt.binding_id.to_string(),
            pane_cwd: "/fixture".into(),
        }),
        terminal_binding_response::Result::Owned(
            crate::remote_server::proto::TerminalBindingOwned {
                launch_id: Uuid::new_v4().to_string(),
                agent: crate::remote_server::proto::TerminalBindingOwnedAgent::Claude as i32,
                phase: "unknown".into(),
                owned_reply_json: Vec::new(),
                native_session_id: None,
            },
        ),
    ]
    .map(|result| TerminalBindingResponse {
        scope: request.scope.clone(),
        attempt_id: request.attempt_id.clone(),
        result: Some(result),
    })
}

#[test]
fn dropping_connection_revokes_work_without_waiting_for_its_state_lock() {
    let (connection, _, request) = fixture();
    let connection = Arc::new(connection);
    let callback_origin = Arc::downgrade(&connection);
    let work = connection.prepare(request);
    let attempt = begin_attempt(&work);
    // Drop 不应等待 Work 的状态锁，更不能同步等待其原生控制客户端。
    let state = attempt.state.lock().unwrap();
    drop(connection);
    assert!(callback_origin.upgrade().is_none());
    assert!(!attempt.pending());
    drop(state);
    assert_failed(work.execute());
}

#[test]
fn completed_reply_requires_exact_scope_attempt_and_binding_id() {
    let (connection, _, request) = fixture();
    let work = connection.prepare(request.clone());
    let attempt = begin_attempt(&work);
    let responses = completed_responses(&request, &attempt);
    for response in &responses {
        assert!(connection.completed_response_is_current(response));
        for changed in 0..4 {
            let mut stale = response.clone();
            match changed {
                0 => stale.scope.as_mut().unwrap().host_id = "another-daemon".into(),
                1 => stale.scope.as_mut().unwrap().terminal_session_id += 1,
                2 => stale.scope.as_mut().unwrap().terminal_epoch = Uuid::new_v4().to_string(),
                3 => stale.attempt_id = Uuid::new_v4().to_string(),
                _ => unreachable!(),
            }
            assert!(!connection.completed_response_is_current(&stale));
        }
    }
    let mut wrong_binding = responses[1].clone();
    let Some(terminal_binding_response::Result::Bound(bound)) = &mut wrong_binding.result else {
        unreachable!()
    };
    bound.opaque_binding_id = Uuid::new_v4().to_string();
    assert!(!connection.completed_response_is_current(&wrong_binding));
    // 回调身份检查不是原生绑定生成接口，不能把此测试回执当成可消费能力。
    assert!(
        connection
            .bound(
                request.scope.as_ref().unwrap(),
                &attempt.binding_id.to_string()
            )
            .is_none()
    );
}

#[test]
fn cancel_between_completion_and_callback_rejects_all_completed_replies() {
    let (connection, _, mut request) = fixture();
    let work = connection.prepare(request.clone());
    let responses = completed_responses(&request, &begin_attempt(&work));
    request.action = Some(terminal_binding_request::Action::Cancel(
        TerminalBindingCancel {},
    ));
    let cancel = connection.prepare(request);
    for response in responses {
        assert!(!connection.completed_response_is_current(&response));
    }
    assert!(matches!(
        cancel.execute().result,
        Some(terminal_binding_response::Result::Cancelled(_))
    ));
}

#[test]
fn replacement_between_completion_and_callback_cannot_publish_an_old_success() {
    let (connection, _, mut request) = fixture();
    let original = connection.prepare(request.clone());
    let old = completed_responses(&request, &begin_attempt(&original));
    request.attempt_id = Uuid::new_v4().to_string();
    let current = connection.prepare(request.clone());
    for response in old {
        assert!(!connection.completed_response_is_current(&response));
    }
    for response in completed_responses(&request, &begin_attempt(&current)) {
        assert!(connection.completed_response_is_current(&response));
    }
}

#[test]
fn bootstrap_or_disconnect_between_completion_and_callback_rejects_success() {
    for disconnect in [false, true] {
        let (connection, mut bootstrap, request) = fixture();
        let original = connection.prepare(request.clone());
        let responses = completed_responses(&request, &begin_attempt(&original));
        if disconnect {
            connection.disconnect();
        } else {
            bootstrap.image_staging_epoch_revision += 1;
            bootstrap.image_staging_epoch = Uuid::new_v4().to_string();
            connection.bootstrap(&bootstrap);
        }
        for response in responses {
            assert!(!connection.completed_response_is_current(&response));
        }
    }
}
