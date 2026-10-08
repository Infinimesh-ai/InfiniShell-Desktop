use std::os::unix::fs::PermissionsExt as _;

use super::*;

#[test]
fn legacy_acl_absent_migration_journal_keeps_schema_and_cold_rollback() {
    let mut fixture = fixture();
    let bytes = serde_json::to_vec(&fixture.journal).unwrap();
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains("\"acl\"")
    );
    let loaded: Migration = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(loaded.schema, 1);
    assert_eq!(loaded.transaction.schema, 1);
    assert_eq!(serde_json::to_vec(&loaded).unwrap(), bytes);
    assert_eq!(save_and_recover(&mut fixture).unwrap(), None);
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"old binary"
    );
    assert_eq!(
        fs::read(&fixture.journal.config.path).unwrap(),
        fixture.journal.config.before.unwrap()
    );
}

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    parent: Directory,
    journal: Migration,
}

fn fixture() -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let prefix = root.join("prefix");
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::create_dir_all(prefix.join("var/homebrew/locks")).unwrap();
    fs::create_dir(prefix.join("Caskroom")).unwrap();
    fs::set_permissions(&prefix, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(prefix.join("bin/brew"), b"fixed manager").unwrap();
    let prefix_directory = Directory::open(&prefix).unwrap();
    let (parent, parent_identity) = prefix_directory.homebrew_caskroom().unwrap();
    let source = parent.create(OsStr::new(OLD_TOKEN)).unwrap();
    source
        .write_new(
            Path::new("2.1.287/claude"),
            &mut &b"old binary"[..],
            10,
            Sha256::digest(b"old binary").into(),
        )
        .unwrap();
    let original = source.snapshot().unwrap();
    let id = Uuid::new_v4();
    let stage_name = format!(".infinishell-brew-{id}");
    let stage = parent.create(OsStr::new(&stage_name)).unwrap();
    stage
        .write_new(
            Path::new("2.1.285/claude"),
            &mut &b"new binary"[..],
            10,
            Sha256::digest(b"new binary").into(),
        )
        .unwrap();
    let entry = prefix.join("bin/claude");
    symlink(
        prefix
            .join("Caskroom")
            .join(OLD_TOKEN)
            .join("2.1.287/claude"),
        &entry,
    )
    .unwrap();
    let link_stage = prefix
        .join("bin")
        .join(format!(".infinishell-brew-link-{id}"));
    symlink(
        prefix
            .join("Caskroom")
            .join(NEW_TOKEN)
            .join("2.1.285/claude"),
        &link_stage,
    )
    .unwrap();
    let config_path = root.join("settings.json");
    let before = br#"{"autoUpdatesChannel":"latest","other":true}"#.to_vec();
    fs::write(&config_path, &before).unwrap();
    fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600)).unwrap();
    let config = ConfigBackup {
        kind: ConfigKind::Claude,
        path: config_path,
        before: Some(before.clone()),
        after: None,
        desired: Some(sources::ConfigDesired {
            bytes: sources::selected_channel_config(
                ConfigKind::Claude,
                Some(&before),
                Some(Channel::Stable),
            )
            .unwrap(),
        }),
        before_mode: Some(0o600),
        restore_stage: None,
    };
    let policy =
        sources::snapshot_claude_scope(&config, id, root.join(".config.json"), NEW_VERSION)
            .unwrap();
    let journal = Migration {
        schema: 1,
        phase: MigrationPhase::Prepared,
        downgrade: claude_downgrade::Intent::ClaudeHomebrewStable21287To21285,
        config,
        policy,
        transaction: Journal {
            schema: 1,
            id,
            agent: "claude".into(),
            prefix: prefix.clone(),
            prefix_identity: prefix_directory.identity().unwrap(),
            parent_identity,
            bin_identity: Directory::open(&prefix.join("bin"))
                .unwrap()
                .identity()
                .unwrap(),
            token: OLD_TOKEN.into(),
            entry: entry.clone(),
            manager: sources::stamp(&prefix.join("bin/brew")).unwrap(),
            old_version: OLD_VERSION.into(),
            target_version: NEW_VERSION.into(),
            intent: "user-stable".into(),
            original,
            prepared: Some(stage.snapshot().unwrap()),
            original_link: link(&entry).unwrap(),
            prepared_link: Some(link(&link_stage).unwrap()),
            phase: Phase::Prepared,
            probe: None,
            extra_probes: Vec::new(),
            completions: Vec::new(),
            aliases: Vec::new(),
            entry_probes: Vec::new(),
        },
    };
    Fixture {
        _temporary: temporary,
        root,
        parent,
        journal,
    }
}

fn save_and_recover(fixture: &mut Fixture) -> Result<Option<String>, Error> {
    let path = path(&fixture.root);
    save_migration(&path, &fixture.journal)?;
    // 真实序列化重读；文件状态机测试的小映像不进入生产固定发行合同。
    let mut loaded = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    recover_validated(&fixture.root, &path, &mut loaded)
}

fn publish_target(fixture: &mut Fixture) {
    fixture
        .parent
        .rename_noreplace(
            &fixture.journal.transaction.stage_name(),
            OsStr::new(NEW_TOKEN),
        )
        .unwrap();
    fixture.journal.phase = MigrationPhase::PublishingTarget;
}

fn publish_link(fixture: &mut Fixture) {
    publish_target(fixture);
    exchange_links(&fixture.journal.transaction).unwrap();
    fixture.journal.phase = MigrationPhase::PublishingLink;
}

fn retire_source(fixture: &mut Fixture) {
    publish_link(fixture);
    fixture
        .parent
        .rename_noreplace(OsStr::new(OLD_TOKEN), &fixture.journal.backup_name())
        .unwrap();
    fixture.journal.phase = MigrationPhase::RetiringSource;
}

#[test]
fn prepared_restart_keeps_original_cask_and_configuration() {
    let mut fixture = fixture();
    assert_eq!(save_and_recover(&mut fixture).unwrap(), None);
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"old binary"
    );
    assert_eq!(
        fs::read(&fixture.journal.config.path).unwrap(),
        fixture.journal.config.before.unwrap()
    );
    assert!(
        !fixture
            .parent
            .has_child(&fixture.journal.transaction.stage_name())
            .unwrap()
    );
    assert!(!path(&fixture.root).exists());
}

#[test]
fn target_publication_restart_restores_the_only_original_registration() {
    let mut fixture = fixture();
    publish_target(&mut fixture);
    assert_eq!(save_and_recover(&mut fixture).unwrap(), None);
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"old binary"
    );
    assert!(!fixture.parent.has_child(OsStr::new(NEW_TOKEN)).unwrap());
    assert_eq!(
        fixture
            .parent
            .child(OsStr::new(OLD_TOKEN))
            .unwrap()
            .snapshot()
            .unwrap(),
        fixture.journal.transaction.original
    );
}

#[test]
fn link_publication_restart_rolls_back_actual_new_public_entry() {
    let mut fixture = fixture();
    publish_link(&mut fixture);
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"new binary"
    );
    assert_eq!(save_and_recover(&mut fixture).unwrap(), None);
    assert_eq!(
        link(&fixture.journal.transaction.entry).unwrap(),
        fixture.journal.transaction.original_link
    );
    assert!(!fixture.parent.has_child(OsStr::new(NEW_TOKEN)).unwrap());
}

#[test]
fn retired_source_restart_restores_complete_old_tree_and_original_link() {
    let mut fixture = fixture();
    retire_source(&mut fixture);
    assert_eq!(save_and_recover(&mut fixture).unwrap(), None);
    assert_eq!(
        fixture
            .parent
            .child(OsStr::new(OLD_TOKEN))
            .unwrap()
            .snapshot()
            .unwrap(),
        fixture.journal.transaction.original
    );
    assert_eq!(
        link(&fixture.journal.transaction.entry).unwrap(),
        fixture.journal.transaction.original_link
    );
    assert!(
        !fixture
            .parent
            .has_child(&fixture.journal.backup_name())
            .unwrap()
    );
}

#[test]
fn changed_candidate_prevents_any_rollback_publication() {
    let mut fixture = fixture();
    retire_source(&mut fixture);
    fs::write(fixture.journal.target(), b"external edit").unwrap();
    assert!(save_and_recover(&mut fixture).is_err());
    assert_eq!(
        link(&fixture.journal.transaction.entry).unwrap(),
        fixture.journal.transaction.prepared_link.clone().unwrap()
    );
    assert!(!fixture.parent.has_child(OsStr::new(OLD_TOKEN)).unwrap());
    assert!(
        fixture
            .parent
            .has_child(&fixture.journal.backup_name())
            .unwrap()
    );
    assert!(path(&fixture.root).exists());
}

#[test]
fn changed_backup_prevents_public_link_rollback() {
    let mut fixture = fixture();
    retire_source(&mut fixture);
    fs::write(
        fixture
            .journal
            .transaction
            .parent()
            .join(fixture.journal.backup_name())
            .join("2.1.287/claude"),
        b"external old edit",
    )
    .unwrap();
    assert!(save_and_recover(&mut fixture).is_err());
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"new binary"
    );
    assert!(!fixture.parent.has_child(OsStr::new(OLD_TOKEN)).unwrap());
}

#[test]
fn changed_configuration_prevents_tree_and_link_rollback() {
    let mut fixture = fixture();
    retire_source(&mut fixture);
    fs::write(&fixture.journal.config.path, b"user edit").unwrap();
    assert!(save_and_recover(&mut fixture).is_err());
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"new binary"
    );
    assert_eq!(
        fs::read(&fixture.journal.config.path).unwrap(),
        b"user edit"
    );
}

#[test]
fn rollback_cleanup_restart_accepts_only_unchanged_remaining_members() {
    let mut fixture = fixture();
    fixture.journal.phase = MigrationPhase::RollingBack;
    fs::remove_file(fixture.journal.candidate()).unwrap();
    assert_eq!(save_and_recover(&mut fixture).unwrap(), None);
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"old binary"
    );
    assert!(
        !fixture
            .parent
            .has_child(&fixture.journal.transaction.stage_name())
            .unwrap()
    );
}

#[test]
fn rollback_cleanup_rejects_an_added_member_before_touching_original() {
    let mut fixture = fixture();
    fixture.journal.phase = MigrationPhase::RollingBack;
    fs::write(
        fixture
            .journal
            .candidate()
            .parent()
            .unwrap()
            .join("foreign"),
        b"foreign",
    )
    .unwrap();
    assert!(save_and_recover(&mut fixture).is_err());
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"old binary"
    );
    assert!(path(&fixture.root).exists());
}

#[test]
fn production_recovery_never_accepts_small_test_images_as_official_releases() {
    let fixture = fixture();
    save_migration(&path(&fixture.root), &fixture.journal).unwrap();
    assert!(
        recover_if_present(
            CLIAgent::Claude,
            &fixture.journal.transaction.entry,
            &fixture.root
        )
        .is_err()
    );
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"old binary"
    );
    assert!(path(&fixture.root).exists());
}

#[test]
fn legacy_same_token_journal_is_not_deserialized_as_migration() {
    let fixture = fixture();
    let legacy = serde_json::to_vec(&fixture.journal.transaction).unwrap();
    assert!(serde_json::from_slice::<Migration>(&legacy).is_err());
    assert!(
        serde_json::from_slice::<Journal>(&serde_json::to_vec(&fixture.journal).unwrap()).is_err()
    );
}

fn committed_fixture() -> Fixture {
    let mut fixture = fixture();
    retire_source(&mut fixture);
    fixture.journal.config.after = fixture.journal.config.before.clone();
    sources::plan_config_publish(&mut fixture.journal.config, true).unwrap();
    sources::publish_config(&fixture.journal.config, true).unwrap();
    fixture.journal.phase = MigrationPhase::Committed;
    fixture
}

#[test]
fn committed_cleanup_keeps_new_registration_and_stable_configuration() {
    let fixture = committed_fixture();
    save_migration(&path(&fixture.root), &fixture.journal).unwrap();
    finish_files(&fixture.root, &path(&fixture.root), &fixture.journal).unwrap();
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"new binary"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&fixture.journal.config.path).unwrap()).unwrap(),
        json!({"autoUpdatesChannel":"stable", "other":true})
    );
    assert!(!fixture.parent.has_child(OsStr::new(OLD_TOKEN)).unwrap());
    assert!(
        !fixture
            .parent
            .has_child(&fixture.journal.backup_name())
            .unwrap()
    );
    assert!(!path(&fixture.root).exists());
}

#[test]
fn committed_cleanup_restart_accepts_only_old_tree_remaining_members() {
    let fixture = committed_fixture();
    fs::remove_file(
        fixture
            .journal
            .transaction
            .parent()
            .join(fixture.journal.backup_name())
            .join("2.1.287/claude"),
    )
    .unwrap();
    save_migration(&path(&fixture.root), &fixture.journal).unwrap();
    let loaded: Migration =
        serde_json::from_slice(&fs::read(path(&fixture.root)).unwrap()).unwrap();
    finish_files(&fixture.root, &path(&fixture.root), &loaded).unwrap();
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"new binary"
    );
    assert!(
        !fixture
            .parent
            .has_child(&fixture.journal.backup_name())
            .unwrap()
    );
}

#[test]
fn committed_cleanup_rejects_foreign_backup_before_removing_any_file() {
    let fixture = committed_fixture();
    let backup = fixture
        .journal
        .transaction
        .parent()
        .join(fixture.journal.backup_name());
    fs::write(backup.join("foreign"), b"user file").unwrap();
    save_migration(&path(&fixture.root), &fixture.journal).unwrap();
    assert!(finish_files(&fixture.root, &path(&fixture.root), &fixture.journal).is_err());
    assert_eq!(
        fs::read(backup.join("2.1.287/claude")).unwrap(),
        b"old binary"
    );
    assert!(path(&fixture.root).exists());
}

#[test]
fn preparing_interruption_retires_active_journal_and_preserves_unknown_partial_stage() {
    let mut fixture = fixture();
    fixture.journal.phase = MigrationPhase::Preparing;
    fixture.journal.transaction.prepared = None;
    fixture.journal.transaction.prepared_link = None;
    fs::write(fixture.journal.candidate(), b"partial download").unwrap();
    assert_eq!(save_and_recover(&mut fixture).unwrap(), None);
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"old binary"
    );
    assert_eq!(
        fs::read(fixture.journal.candidate()).unwrap(),
        b"partial download"
    );
    assert!(!path(&fixture.root).exists());
    assert!(
        fixture
            .root
            .join(format!(
                "claude-homebrew-migration-{}.retained.json",
                fixture.journal.transaction.id
            ))
            .exists()
    );
}

#[test]
fn preparing_before_stage_creation_keeps_original_cli_available() {
    let mut fixture = fixture();
    fixture
        .parent
        .remove_matching(
            &fixture.journal.transaction.stage_name(),
            fixture.journal.transaction.prepared.as_ref().unwrap(),
        )
        .unwrap();
    fs::remove_file(fixture.journal.transaction.link_stage()).unwrap();
    fixture.journal.phase = MigrationPhase::Preparing;
    fixture.journal.transaction.prepared = None;
    fixture.journal.transaction.prepared_link = None;
    assert_eq!(save_and_recover(&mut fixture).unwrap(), None);
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"old binary"
    );
    assert!(!path(&fixture.root).exists());
}

#[test]
fn preparing_recovery_preserves_external_source_changes_and_active_journal() {
    let mut fixture = fixture();
    fixture.journal.phase = MigrationPhase::Preparing;
    fixture.journal.transaction.prepared = None;
    fixture.journal.transaction.prepared_link = None;
    fs::write(fixture.journal.original_target(), b"external source").unwrap();
    assert!(save_and_recover(&mut fixture).is_err());
    assert_eq!(
        fs::read(&fixture.journal.transaction.entry).unwrap(),
        b"external source"
    );
    assert!(path(&fixture.root).exists());
}
