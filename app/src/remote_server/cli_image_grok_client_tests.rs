use super::*;

fn private_directory() -> tempfile::TempDir {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    builder.tempdir().unwrap()
}

fn journal(directory: &Path) -> Journal {
    check_private(directory, true).unwrap();
    let lock = options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("journal.lock"))
        .unwrap();
    Journal {
        directory: directory.to_owned(),
        lock,
    }
}

fn launch() -> Launch {
    Launch {
        tmux_source: None,
        tmux_terminal_session: None,
        host: "remote-fixture".into(),
        terminal_session: 7,
        block_id: "block-fixture".into(),
        cwd: "/fixture/project".into(),
        ticket: Ticket {
            id: Uuid::new_v4(),
            key: Uuid::new_v4(),
        },
    }
}

fn input() -> Input {
    Input {
        message_id: Uuid::new_v4(),
        text: "保留原草稿".into(),
        images: Vec::new(),
    }
}

#[test]
fn not_dispatched_input_allows_new_submission_after_journal_reopen() {
    let directory = private_directory();
    let store = journal(directory.path());
    let launch = launch();
    let original = input();
    let subject = store.claim_input(&launch, &original).unwrap();
    let claim_path = directory.path().join(format!(
        "{}-{}-unknown.json",
        launch.ticket.id, original.message_id
    ));
    let original_bytes = fs::read(&claim_path).unwrap();

    store
        .record_not_dispatched(&launch, original.message_id, &subject)
        .unwrap();
    let marker_path = directory.path().join(format!(
        "{}-{}-not-dispatched.json",
        launch.ticket.id, original.message_id
    ));
    let marker_bytes = fs::read(&marker_path).unwrap();
    drop(store);
    let restored = journal(directory.path());
    // 重复收尾只认可相同终态，原领取和终态字节均不得覆盖。
    restored
        .record_not_dispatched(&launch, original.message_id, &subject)
        .unwrap();
    assert_eq!(fs::read(claim_path).unwrap(), original_bytes);
    assert_eq!(fs::read(marker_path).unwrap(), marker_bytes);
    assert!(restored.unknown_inputs(&launch).unwrap().is_empty());
    assert!(restored.claim_input(&launch, &original).is_err());

    let next = input();
    assert_eq!(restored.claim_input(&launch, &next).unwrap(), subject);
    assert_eq!(
        restored.unknown_inputs(&launch).unwrap(),
        vec![next.message_id]
    );
}

#[test]
fn unknown_rpc_result_still_blocks_same_draft_after_reopen() {
    let directory = private_directory();
    let store = journal(directory.path());
    let launch = launch();
    let original = input();
    let subject = store.claim_input(&launch, &original).unwrap();
    store
        .record_reply(
            &launch,
            &Reply::Input {
                ticket: launch.ticket.clone(),
                message_id: original.message_id,
                subject_sha256: subject,
                state: "unknown".into(),
                native_prompt_id: None,
                native_ack_sha256: None,
            },
        )
        .unwrap();
    // 远端查询缺失或失败不能证明此前没有执行。
    store
        .record_reply(
            &launch,
            &Reply::Failed {
                code: "unavailable".into(),
            },
        )
        .unwrap();
    drop(store);

    let restored = journal(directory.path());
    assert!(restored.claim_input(&launch, &input()).is_err());
    assert_eq!(
        restored.unknown_inputs(&launch).unwrap(),
        vec![original.message_id]
    );
}

#[test]
fn not_dispatched_receipt_requires_exact_ticket_message_and_subject() {
    let directory = private_directory();
    let store = journal(directory.path());
    let original_launch = launch();
    let original = input();
    let subject = store.claim_input(&original_launch, &original).unwrap();

    assert!(
        store
            .record_not_dispatched(&launch(), original.message_id, &subject)
            .is_err()
    );
    assert!(
        store
            .record_not_dispatched(&original_launch, Uuid::new_v4(), &subject)
            .is_err()
    );
    assert!(
        store
            .record_not_dispatched(&original_launch, original.message_id, "不同内容摘要")
            .is_err()
    );
    assert!(store.claim_input(&original_launch, &input()).is_err());
    assert_eq!(
        store.unknown_inputs(&original_launch).unwrap(),
        vec![original.message_id]
    );
}

#[test]
fn mismatched_not_dispatched_receipt_does_not_unlock_unknown_input() {
    let directory = private_directory();
    let store = journal(directory.path());
    let launch = launch();
    let original = input();
    let subject = store.claim_input(&launch, &original).unwrap();
    let other_ticket = Ticket {
        id: launch.ticket.id,
        key: Uuid::new_v4(),
    };
    let marker = directory.path().join(format!(
        "{}-{}-not-dispatched.json",
        launch.ticket.id, original.message_id
    ));
    store
        .write(&marker, &(other_ticket, original.message_id, &subject))
        .unwrap();

    assert!(
        store
            .record_not_dispatched(&launch, original.message_id, &subject)
            .is_err()
    );
    assert!(store.claim_input(&launch, &input()).is_err());
    assert!(store.unknown_inputs(&launch).is_err());
}

#[test]
fn tmux_grok_launch_keeps_original_identity_and_does_not_restore_runtime_scope() {
    let directory = private_directory();
    let store = journal(directory.path());
    let mut current = launch();
    current.tmux_source = Some(current.ticket.id);
    current.tmux_terminal_session = Some(91);
    store.remember_tmux(&current).unwrap();
    let path = directory.path().join(format!("tmux-{}-launch.json", current.ticket.id));
    let persisted: Launch = store.read(&path).unwrap();
    assert_eq!(persisted.host, current.host);
    assert_eq!(persisted.terminal_session, 7);
    assert_eq!(persisted.tmux_source, current.tmux_source);
    assert!(persisted.tmux_terminal_session.is_none());
    assert!(store.known_launches(&current.host, SessionId::from(7), &current.cwd).unwrap().is_empty());
    let mut mapped_again = current.clone();
    mapped_again.tmux_terminal_session = Some(92);
    store.remember_tmux(&mapped_again).unwrap();
    mapped_again.cwd = "/other".into();
    assert!(store.remember_tmux(&mapped_again).is_err());
    let old: Launch = serde_json::from_slice(&encode(&launch()).unwrap()).unwrap();
    assert!(old.tmux_source.is_none());
    assert!(old.tmux_terminal_session.is_none());
}

#[test]
fn owned_identity_keeps_matching_native_prompt_but_revokes_changed_permission() {
    let native = Uuid::new_v4();
    let identity = ActiveIdentity::Owned {
        native_session: native,
        cwd: "/fixture/project".into(),
    };
    let body = serde_json::json!({"v":1,"agent":"grok","event":"prompt_submit","session_id":native,
        "cwd":"/fixture/project","event_id":"actual-prompt","permission_mode":"default"})
    .to_string();
    let event =
        crate::terminal::cli_agent_sessions::event::parse_event(Some("warp://cli-agent"), &body)
            .unwrap();
    assert!(!identity.invalidated_by(&event));
    for mode in [None, Some("auto"), Some("plan"), Some("bypassPermissions")] {
        let mut changed = event.clone();
        changed.payload.permission_mode = mode.map(str::to_owned);
        assert!(identity.invalidated_by(&changed));
    }
    let mut changed = event.clone();
    changed.event = CLIAgentEventType::SessionStart;
    assert!(identity.invalidated_by(&changed));
    changed = event.clone();
    changed.session_id = Some(Uuid::new_v4().to_string());
    assert!(identity.invalidated_by(&changed));
    changed = event;
    changed.cwd = Some("/another/project".into());
    assert!(identity.invalidated_by(&changed));
}

#[test]
fn native_pre_enqueue_rejection_is_terminal_but_never_allows_same_message_replay() {
    let directory = private_directory();
    let store = journal(directory.path());
    let launch = launch();
    let original = input();
    let subject = store.claim_input(&launch, &original).unwrap();
    let reply = Reply::Input {
        ticket: launch.ticket.clone(),
        message_id: original.message_id,
        subject_sha256: subject.clone(),
        state: "rejected_before_enqueue".into(),
        native_prompt_id: None,
        native_ack_sha256: Some("b".repeat(64)),
    };
    store.record_reply(&launch, &reply).unwrap();
    assert!(store.unknown_inputs(&launch).unwrap().is_empty());
    assert!(store.claim_input(&launch, &original).is_err());
    assert_eq!(store.claim_input(&launch, &input()).unwrap(), subject);
}

#[test]
fn native_pre_enqueue_rejection_requires_exact_claim_and_ack() {
    let directory = private_directory();
    let store = journal(directory.path());
    let launch = launch();
    let original = input();
    let subject = store.claim_input(&launch, &original).unwrap();
    for (prompt, ack, hash) in [
        (Some(Uuid::new_v4()), Some("b".repeat(64)), subject.clone()),
        (None, None, subject.clone()),
        (None, Some("not-a-digest".into()), subject.clone()),
        (None, Some("b".repeat(64)), "different-subject".into()),
    ] {
        assert!(
            store
                .record_reply(
                    &launch,
                    &Reply::Input {
                        ticket: launch.ticket.clone(),
                        message_id: original.message_id,
                        subject_sha256: hash,
                        state: "rejected_before_enqueue".into(),
                        native_prompt_id: prompt,
                        native_ack_sha256: ack,
                    }
                )
                .is_err()
        );
    }
    assert_eq!(
        store.unknown_inputs(&launch).unwrap(),
        vec![original.message_id]
    );
}
