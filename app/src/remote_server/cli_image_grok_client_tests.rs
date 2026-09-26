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
