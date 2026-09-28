use super::*;

struct Fixture {
    _temporary: tempfile::TempDir,
    parent: Directory,
    journal: Journal,
}

fn fixture() -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let prefix = temporary.path().canonicalize().unwrap().join("prefix");
    let original_root = prefix.join("Caskroom/codex");
    fs::create_dir_all(original_root.join("0.155.1/bin")).unwrap();
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::write(
        original_root.join("0.155.1/bin/codex"),
        b"old public binary",
    )
    .unwrap();
    fs::write(prefix.join("bin/brew"), b"unchanged manager").unwrap();
    let entry = prefix.join("bin/codex");
    symlink("../Caskroom/codex/0.155.1/bin/codex", &entry).unwrap();
    let parent = Directory::open(&prefix.join("Caskroom")).unwrap();
    let original = parent.child("codex".as_ref()).unwrap().snapshot().unwrap();
    let id = Uuid::new_v4();
    let stage_name = OsString::from(format!(".infinishell-brew-{id}"));
    let stage = parent.create(&stage_name).unwrap();
    let stage_root = prefix.join("Caskroom").join(&stage_name);
    fs::create_dir_all(stage_root.join("0.156.1/bin")).unwrap();
    fs::write(stage_root.join("0.156.1/bin/codex"), b"new public binary").unwrap();
    let link_stage = prefix
        .join("bin")
        .join(format!(".infinishell-brew-link-{id}"));
    symlink(original_root.join("0.156.1/bin/codex"), &link_stage).unwrap();
    let journal = Journal {
        schema: 1,
        id,
        agent: "codex".into(),
        prefix_identity: Directory::open(&prefix).unwrap().identity().unwrap(),
        parent_identity: parent.identity().unwrap(),
        bin_identity: Directory::open(&prefix.join("bin"))
            .unwrap()
            .identity()
            .unwrap(),
        token: "codex".into(),
        entry: entry.clone(),
        manager: stamp(&prefix.join("bin/brew")).unwrap(),
        old_version: "0.155.1".into(),
        target_version: "0.156.1".into(),
        intent: "fixed-test-intent".into(),
        original,
        prepared: Some(stage.snapshot().unwrap()),
        original_link: link(&entry).unwrap(),
        prepared_link: Some(link(&link_stage).unwrap()),
        phase: Phase::ExchangeIntent,
        probe: None,
        extra_probes: Vec::new(),
        completions: Vec::new(),
        aliases: Vec::new(),
        entry_probes: Vec::new(),
        prefix,
    };
    parent.exchange("codex".as_ref(), &stage_name).unwrap();
    exchange_links(&journal).unwrap();
    Fixture {
        _temporary: temporary,
        parent,
        journal,
    }
}

#[test]
fn changed_published_tree_preserves_public_link_and_backup() {
    let fixture = fixture();
    let journal = &fixture.journal;
    let current_root = journal.parent().join("codex");
    fs::write(current_root.join("external-marker"), b"external change").unwrap();
    let changed = fixture
        .parent
        .child("codex".as_ref())
        .unwrap()
        .snapshot()
        .unwrap();
    let published_link = link(&journal.entry).unwrap();
    let backup_link = link(&journal.link_stage()).unwrap();

    assert_eq!(
        rollback_publication(journal, &fixture.parent),
        Err(Error::RecoveryRequired)
    );

    assert_eq!(link(&journal.entry).unwrap(), published_link);
    assert_eq!(link(&journal.link_stage()).unwrap(), backup_link);
    assert_eq!(fs::read(&journal.entry).unwrap(), b"new public binary");
    assert_eq!(
        fs::read(current_root.join("external-marker")).unwrap(),
        b"external change"
    );
    assert_eq!(
        fixture
            .parent
            .child("codex".as_ref())
            .unwrap()
            .snapshot()
            .unwrap(),
        changed
    );
    assert_eq!(
        fixture
            .parent
            .child(&journal.stage_name())
            .unwrap()
            .snapshot()
            .unwrap(),
        journal.original
    );
}

#[test]
fn changed_backup_tree_preserves_public_link_and_current_tree() {
    let fixture = fixture();
    let journal = &fixture.journal;
    let backup_root = journal.parent().join(journal.stage_name());
    fs::write(
        backup_root.join("external-marker"),
        b"external backup change",
    )
    .unwrap();
    let changed = fixture
        .parent
        .child(&journal.stage_name())
        .unwrap()
        .snapshot()
        .unwrap();
    let published_link = link(&journal.entry).unwrap();
    let backup_link = link(&journal.link_stage()).unwrap();

    assert_eq!(
        rollback_publication(journal, &fixture.parent),
        Err(Error::RecoveryRequired)
    );

    assert_eq!(link(&journal.entry).unwrap(), published_link);
    assert_eq!(link(&journal.link_stage()).unwrap(), backup_link);
    assert_eq!(fs::read(&journal.entry).unwrap(), b"new public binary");
    assert_eq!(
        fs::read(backup_root.join("external-marker")).unwrap(),
        b"external backup change"
    );
    assert_eq!(
        fixture
            .parent
            .child("codex".as_ref())
            .unwrap()
            .snapshot()
            .unwrap(),
        *journal.prepared.as_ref().unwrap()
    );
    assert_eq!(
        fixture
            .parent
            .child(&journal.stage_name())
            .unwrap()
            .snapshot()
            .unwrap(),
        changed
    );
}

#[test]
fn unchanged_published_trees_restore_original_publication() {
    let fixture = fixture();
    let journal = &fixture.journal;

    rollback_publication(journal, &fixture.parent).unwrap();

    assert_eq!(fs::read(&journal.entry).unwrap(), b"old public binary");
    assert_eq!(link(&journal.entry).unwrap(), journal.original_link);
    assert_eq!(
        link(&journal.link_stage()).unwrap(),
        *journal.prepared_link.as_ref().unwrap()
    );
    assert_eq!(
        fixture
            .parent
            .child("codex".as_ref())
            .unwrap()
            .snapshot()
            .unwrap(),
        journal.original
    );
    assert_eq!(
        fixture
            .parent
            .child(&journal.stage_name())
            .unwrap()
            .snapshot()
            .unwrap(),
        *journal.prepared.as_ref().unwrap()
    );
}
