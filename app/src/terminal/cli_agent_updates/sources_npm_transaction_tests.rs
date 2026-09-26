use std::os::unix::fs::symlink;

use super::*;

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    journal: Journal,
}

fn fixture() -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().canonicalize().unwrap();
    let root = base.join("journal");
    let prefix = base.join("prefix");
    let package_root = prefix.join("lib/node_modules/@anthropic-ai/claude-code");
    fs::create_dir_all(package_root.join("bin")).unwrap();
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::create_dir(&root).unwrap();
    fs::write(package_root.join("bin/claude.exe"), b"old public binary").unwrap();
    fs::write(base.join("manager"), b"same manager").unwrap();
    let entry = prefix.join("bin/claude");
    symlink(
        "../lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe",
        &entry,
    )
    .unwrap();
    let parent = Directory::open(package_root.parent().unwrap()).unwrap();
    let original = parent
        .child(std::ffi::OsStr::new("claude-code"))
        .unwrap()
        .snapshot()
        .unwrap();
    let id = Uuid::new_v4();
    let stage_name = OsString::from(format!(".infinishell-npm-{id}"));
    let stage = parent.create(&stage_name).unwrap();
    let stage_root = package_root.parent().unwrap().join(&stage_name);
    fs::create_dir(stage_root.join("bin")).unwrap();
    fs::write(stage_root.join("bin/claude.exe"), b"new public binary").unwrap();
    let journal = Journal {
        schema: 1,
        layout_version: LAYOUT_VERSION,
        agent: "claude".into(),
        id,
        owner: Owner {
            prefix: prefix.clone(),
            prefix_identity: Directory::open(&prefix).unwrap().identity().unwrap(),
            parent_identity: parent.identity().unwrap(),
            package_root,
            public_relative: "bin/claude.exe".into(),
            entry: entry.clone(),
            entry_link: Some(entry_link(&entry).unwrap()),
            external_files: vec![stamp(&base.join("manager")).unwrap()],
        },
        old_version: "2.1.278".into(),
        target_version: "2.1.280".into(),
        intent: "fixed-test-intent".into(),
        downgrade: None,
        stage_name,
        phase: Phase::SwapIntent,
        original,
        prepared: Some(stage.snapshot().unwrap()),
        stage_identity: Some(stage.identity().unwrap()),
        probe: None,
        config: None,
        protected_config: None,
        claude_policy: None,
        wrapper_archive_sha256: [1; 32],
        platform_archive_sha256: [2; 32],
    };
    Fixture {
        _temporary: temporary,
        root,
        journal,
    }
}

#[test]
fn crash_before_exchange_preserves_original_and_removes_only_owned_stage() {
    let fixture = fixture();
    let journal = &fixture.journal;
    save(&journal_path(&fixture.root, CLIAgent::Claude), journal).unwrap();

    assert_eq!(
        recover(CLIAgent::Claude, &journal.owner.entry, &fixture.root).unwrap(),
        None
    );

    assert_eq!(
        fs::read(&journal.owner.entry).unwrap(),
        b"old public binary"
    );
    assert!(
        !journal
            .owner
            .package_root
            .parent()
            .unwrap()
            .join(&journal.stage_name)
            .exists()
    );
    assert!(super::super::failure_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn exchange_without_post_exchange_receipt_rolls_back_by_tree_identity() {
    let fixture = fixture();
    let journal = &fixture.journal;
    save(&journal_path(&fixture.root, CLIAgent::Claude), journal).unwrap();
    let parent = journal.owner.verify_external().unwrap();
    parent
        .exchange(std::ffi::OsStr::new("claude-code"), &journal.stage_name)
        .unwrap();

    recover(CLIAgent::Claude, &journal.owner.entry, &fixture.root).unwrap();

    assert_eq!(
        fs::read(&journal.owner.entry).unwrap(),
        b"old public binary"
    );
    assert_eq!(
        parent
            .child(std::ffi::OsStr::new("claude-code"))
            .unwrap()
            .snapshot()
            .unwrap(),
        journal.original
    );
}

#[test]
fn external_npm_change_after_exchange_blocks_rollback_and_preserves_both_trees() {
    let fixture = fixture();
    let journal = &fixture.journal;
    save(&journal_path(&fixture.root, CLIAgent::Claude), journal).unwrap();
    journal
        .owner
        .verify_external()
        .unwrap()
        .exchange(std::ffi::OsStr::new("claude-code"), &journal.stage_name)
        .unwrap();
    fs::write(
        journal.owner.package_root.join("bin/claude.exe"),
        b"later npm installation",
    )
    .unwrap();

    assert!(recover(CLIAgent::Claude, &journal.owner.entry, &fixture.root).is_err());

    assert_eq!(
        fs::read(&journal.owner.entry).unwrap(),
        b"later npm installation"
    );
    assert_eq!(
        fs::read(
            journal
                .owner
                .package_root
                .parent()
                .unwrap()
                .join(&journal.stage_name)
                .join("bin/claude.exe")
        )
        .unwrap(),
        b"old public binary"
    );
    assert!(journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn manager_replacement_blocks_transaction_recovery() {
    let fixture = fixture();
    let journal = &fixture.journal;
    save(&journal_path(&fixture.root, CLIAgent::Claude), journal).unwrap();
    fs::write(
        &journal.owner.external_files[0].canonical,
        b"changed manager",
    )
    .unwrap();

    assert!(recover(CLIAgent::Claude, &journal.owner.entry, &fixture.root).is_err());
    assert_eq!(
        fs::read(&journal.owner.entry).unwrap(),
        b"old public binary"
    );
    assert!(journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn modified_backup_is_not_swapped_over_current_installation() {
    let fixture = fixture();
    let journal = &fixture.journal;
    save(&journal_path(&fixture.root, CLIAgent::Claude), journal).unwrap();
    journal
        .owner
        .verify_external()
        .unwrap()
        .exchange(std::ffi::OsStr::new("claude-code"), &journal.stage_name)
        .unwrap();
    fs::write(
        journal
            .owner
            .package_root
            .parent()
            .unwrap()
            .join(&journal.stage_name)
            .join("bin/claude.exe"),
        b"modified backup",
    )
    .unwrap();

    assert!(recover(CLIAgent::Claude, &journal.owner.entry, &fixture.root).is_err());
    assert_eq!(
        fs::read(&journal.owner.entry).unwrap(),
        b"new public binary"
    );
}

#[test]
fn incomplete_unpublished_tree_is_retained_with_its_record() {
    let mut fixture = fixture();
    fixture.journal.phase = Phase::Preparing;
    fixture.journal.prepared = None;
    save(
        &journal_path(&fixture.root, CLIAgent::Claude),
        &fixture.journal,
    )
    .unwrap();

    recover(
        CLIAgent::Claude,
        &fixture.journal.owner.entry,
        &fixture.root,
    )
    .unwrap();

    assert_eq!(
        fs::read(&fixture.journal.owner.entry).unwrap(),
        b"old public binary"
    );
    assert!(
        fixture
            .journal
            .owner
            .package_root
            .parent()
            .unwrap()
            .join(&fixture.journal.stage_name)
            .exists()
    );
    assert!(
        fixture
            .root
            .join(format!("claude-npm-retained-{}.json", fixture.journal.id))
            .exists()
    );
    assert!(!journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn claimed_commit_without_observed_version_cannot_publish_success() {
    let mut fixture = fixture();
    fixture.journal.phase = Phase::Committed;
    save(
        &journal_path(&fixture.root, CLIAgent::Claude),
        &fixture.journal,
    )
    .unwrap();

    assert!(
        recover(
            CLIAgent::Claude,
            &fixture.journal.owner.entry,
            &fixture.root
        )
        .is_err()
    );
    assert_eq!(
        fs::read(&fixture.journal.owner.entry).unwrap(),
        b"old public binary"
    );
}

#[test]
fn journal_cannot_redirect_package_recovery_to_another_prefix() {
    let mut fixture = fixture();
    fixture.journal.owner.package_root = fixture.root.clone();
    save(
        &journal_path(&fixture.root, CLIAgent::Claude),
        &fixture.journal,
    )
    .unwrap();

    assert!(
        recover(
            CLIAgent::Claude,
            &fixture.journal.owner.entry,
            &fixture.root
        )
        .is_err()
    );
    assert!(journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn future_release_does_not_inherit_fixed_layout_acceptance() {
    assert_eq!(supports(CLIAgent::Claude, "2.1.280"), Ok(()));
    assert_eq!(
        supports(CLIAgent::Claude, "2.1.281"),
        Err(Error::InvalidRelease)
    );
}

#[test]
fn codex_npm_release_support_does_not_extend_to_an_unknown_version() {
    assert_eq!(supports(CLIAgent::Codex, "0.156.1"), Ok(()));
    assert_eq!(
        supports(CLIAgent::Codex, "0.156.2"),
        Err(Error::InvalidRelease)
    );
}

#[test]
fn codex_public_launcher_requires_more_than_a_native_binary_probe() {
    let mut fixture = fixture();
    let journal = &mut fixture.journal;
    journal.agent = "codex".into();
    journal.old_version = "0.155.1".into();
    journal.target_version = "0.156.1".into();
    journal.owner.package_root = journal.owner.prefix.join("lib/node_modules/@openai/codex");
    journal.owner.public_relative = "bin/codex.js".into();
    journal.owner.entry = journal.owner.prefix.join("bin/codex");
    let stage = journal
        .owner
        .package_root
        .parent()
        .unwrap()
        .join(&journal.stage_name);
    fs::create_dir_all(&stage).unwrap();
    let native = stage.join("codex");
    fs::write(&native, b"native probe without the Node launcher").unwrap();
    assert_eq!(
        validate(CLIAgent::Codex, &journal.owner.entry, journal),
        Ok(())
    );
    journal.probe = Some(Probe {
        generation: Uuid::new_v4(),
        program: native.clone(),
        program_stamp: stamp(&native).unwrap(),
        binding_digest: "0".repeat(64),
        observed_version: Some("0.156.1".into()),
        codex_closure: None,
    });

    assert_eq!(
        validate(CLIAgent::Codex, &journal.owner.entry, journal),
        Err(Error::RecoveryRequired)
    );
}

#[test]
fn platform_native_probe_cannot_substitute_for_public_entry() {
    let mut fixture = fixture();
    let stage = fixture
        .journal
        .owner
        .package_root
        .parent()
        .unwrap()
        .join(&fixture.journal.stage_name);
    let native = stage.join("dependency-native");
    fs::write(&native, b"different native path").unwrap();
    fixture.journal.probe = Some(Probe {
        generation: Uuid::new_v4(),
        program: native.clone(),
        program_stamp: stamp(&native).unwrap(),
        binding_digest: "0".repeat(64),
        observed_version: Some("2.1.280".into()),
        codex_closure: None,
    });
    assert_eq!(
        validate(
            CLIAgent::Claude,
            &fixture.journal.owner.entry,
            &fixture.journal
        ),
        Err(Error::RecoveryRequired)
    );
}
