use super::*;

fn fixture() -> tempfile::TempDir {
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    builder.tempdir().unwrap()
}

#[test]
fn tmux_owned_intent_survives_reopen_and_start_can_only_be_claimed_once() {
    let root = fixture();
    let journal = Journal::at(root.path()).unwrap();
    let launch = journal
        .reserve(
            "host".into(),
            17,
            "block".into(),
            TerminalBindingOwnedAgent::Claude,
        )
        .unwrap();
    journal.claim_start(&launch).unwrap();
    let reopened = Journal::at(root.path()).unwrap();
    assert!(reopened.read(launch.id).unwrap() == launch);
    assert!(reopened.claim_start(&launch).is_err());
    let mut changed = launch.clone();
    changed.key = Uuid::new_v4();
    assert!(reopened.claim_start(&changed).is_err());
}

#[test]
fn tmux_owned_same_outer_block_keeps_distinct_intents_and_rejects_ambiguous_restore() {
    let root = fixture();
    let journal = Journal::at(root.path()).unwrap();
    let first = journal
        .reserve(
            "host".into(),
            17,
            "block".into(),
            TerminalBindingOwnedAgent::Grok,
        )
        .unwrap();
    assert!(journal.unique_for_block(Some("host"), 17, "block").unwrap() == first);
    let second = journal
        .reserve(
            "host".into(),
            17,
            "block".into(),
            TerminalBindingOwnedAgent::Grok,
        )
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_ne!(first.key, second.key);
    assert!(journal.unique_for_block(Some("host"), 17, "block").is_err());
    assert!(
        journal
            .unique_for_block(Some("other"), 17, "block")
            .is_err()
    );
    assert!(journal.read(first.id).unwrap() == first);
    assert!(journal.read(second.id).unwrap() == second);
}

#[cfg(unix)]
#[test]
fn tmux_owned_journal_rejects_replaced_file_and_does_not_change_existing_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = fixture();
    let journal = Journal::at(root.path()).unwrap();
    let launch = journal
        .reserve(
            "host".into(),
            17,
            "block".into(),
            TerminalBindingOwnedAgent::Codex,
        )
        .unwrap();
    let original = journal.directory.join(format!("{}.json", launch.id));
    let moved = root.path().join("retained.json");
    fs::rename(&original, &moved).unwrap();
    symlink(&moved, &original).unwrap();
    assert!(journal.read(launch.id).is_err());
    assert!(moved.is_file());
    let unsafe_root = fixture();
    let directory = unsafe_root.path().join("remote-tmux-owned-v1");
    let mut builder = fs::DirBuilder::new();
    use std::os::unix::fs::DirBuilderExt;
    builder.mode(0o755).create(&directory).unwrap();
    let before = fs::metadata(&directory).unwrap().permissions().mode();
    assert!(Journal::at(unsafe_root.path()).is_err());
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode(),
        before
    );
}

#[test]
fn tmux_codex_launch_json_preserves_origin_but_never_restores_runtime_alias() {
    use crate::remote_server::cli_image_codex_owned_client::Launch as CodexLaunch;
    use crate::remote_server::cli_image_codex_owned_protocol::Ticket;
    let id = Uuid::new_v4();
    let launch = CodexLaunch {
        host: "original-daemon".into(),
        terminal_session: 7,
        block_id: "block".into(),
        cwd: "/fixture".into(),
        ticket: Ticket {
            id,
            key: Uuid::new_v4(),
        },
        tmux_source: Some(id),
        tmux_terminal_session: Some(99),
    };
    let restored: CodexLaunch =
        serde_json::from_slice(&serde_json::to_vec(&launch).unwrap()).unwrap();
    assert_eq!(restored.host, "original-daemon");
    assert_eq!(restored.terminal_session, 7);
    assert_eq!(restored.tmux_source, Some(id));
    assert!(restored.tmux_terminal_session.is_none());
    let mut value = serde_json::to_value(&launch).unwrap();
    value.as_object_mut().unwrap().remove("tmux_source");
    let old: CodexLaunch = serde_json::from_value(value).unwrap();
    assert!(old.tmux_source.is_none());
    assert!(old.tmux_terminal_session.is_none());
}
