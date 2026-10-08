use super::super::{ConfigDesired, ConfigKind};
use super::*;

fn historical_journal(root: &Path) -> Journal {
    let prefix = root.join("prefix");
    let package = prefix.join("node_modules/@anthropic-ai/claude-code");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("package.json"), b"old registered package").unwrap();
    let mut shims = BTreeMap::new();
    for (name, contents) in contract::shims() {
        let path = prefix.join(name);
        fs::write(&path, contents).unwrap();
        shims.insert(name.to_owned(), stamp(&path).unwrap());
    }
    let mut dependencies = Vec::new();
    for name in ["node.exe", "npm-cli.js", "npm-package.json"] {
        let path = prefix.join(name);
        fs::write(&path, name).unwrap();
        dependencies.push(stamp(&path).unwrap());
    }
    let config_path = root.join("settings.json");
    let before = br#"{"autoUpdatesChannel":"latest"}"#.to_vec();
    fs::write(&config_path, &before).unwrap();
    let config = ConfigBackup {
        kind: ConfigKind::Claude,
        path: config_path,
        before: Some(before.clone()),
        after: None,
        desired: Some(ConfigDesired {
            bytes: Some(before),
        }),
        before_mode: None,
        restore_stage: None,
    };
    let id = Uuid::new_v4();
    Journal {
        schema: 1,
        id,
        owner: Owner {
            entry: prefix.join("claude.cmd"),
            prefix_identity: tree::path_identity(&prefix).unwrap(),
            parent_identity: tree::path_identity(package.parent().unwrap()).unwrap(),
            prefix,
            package: package.clone(),
            dependencies,
            shims,
        },
        old_version: "2.1.278".into(),
        target_version: contract::VERSION.into(),
        stage: package
            .parent()
            .unwrap()
            .join(format!(".infinishell-npm-{id}")),
        backup: package
            .parent()
            .unwrap()
            .join(format!(".infinishell-npm-old-{id}")),
        original: tree::snapshot(&package).unwrap(),
        prepared: None,
        probes: Vec::new(),
        claude_policy: super::super::snapshot_claude_scope(
            &config,
            id,
            root.join(".config.json"),
            contract::VERSION,
        )
        .unwrap(),
        config: Some(config),
        phase: Phase::Allocating,
        intent: "old-latest".into(),
        downgrade: None,
    }
}

// 只构造持久清单准入；实际NTFS树由 snapshot/rename/原件验收另行核实。
fn fixed_inventory(version: &str) -> tree::Snapshot {
    let mut members = BTreeMap::new();
    for (path, (length, digest, _)) in contract::files_for(version).unwrap() {
        for ancestor in path.ancestors().skip(1) {
            members
                .entry(ancestor.to_owned())
                .or_insert(tree::Identity {
                    volume: 1,
                    index: 1,
                    directory: true,
                    length: 0,
                    digest: [0; 32],
                    security: [0; 32],
                });
        }
        members.insert(
            path,
            tree::Identity {
                volume: 1,
                index: 2,
                directory: false,
                length,
                digest: hex::decode(digest).unwrap().try_into().unwrap(),
                security: [0; 32],
            },
        );
    }
    tree::Snapshot { members }
}

#[test]
fn old_journal_without_downgrade_field_recovers_without_changing_original_tree() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let journal = historical_journal(&root);
    let bytes = serde_json::to_vec(&journal).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value.get("downgrade").is_none());
    fs::write(journal_path(&root), &bytes).unwrap();
    assert_eq!(
        recover(CLIAgent::Claude, &journal.owner.entry, &root).unwrap(),
        None
    );
    assert_eq!(
        tree::snapshot(&journal.owner.package).unwrap(),
        journal.original
    );
    assert!(!journal_path(&root).exists());
}

#[test]
fn new_journal_binds_intent_source_and_candidate_versions_before_recovery_mutates_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let mut journal = historical_journal(&root);
    let actual = journal.original.clone();
    journal.old_version = "2.1.287".into();
    journal.target_version = "2.1.285".into();
    journal.downgrade =
        Some(super::super::claude_downgrade::Intent::ClaudeNpmWindowsStable21287To21285);
    journal.config.as_mut().unwrap().desired = Some(ConfigDesired {
        bytes: Some(br#"{"autoUpdatesChannel":"stable"}"#.to_vec()),
    });
    journal.original = fixed_inventory("2.1.287");
    journal.prepared = Some(fixed_inventory("2.1.285"));
    verify_version_contract(&journal).unwrap();
    journal.prepared = Some(fixed_inventory(contract::VERSION));
    assert!(verify_version_contract(&journal).is_err());
    journal.prepared = Some(fixed_inventory("2.1.285"));
    journal.original = fixed_inventory("2.1.285");
    assert!(verify_version_contract(&journal).is_err());
    journal.original = fixed_inventory("2.1.287");
    journal.downgrade = None;
    save(&root, &journal).unwrap();
    assert!(recover(CLIAgent::Claude, &journal.owner.entry, &root).is_err());
    assert_eq!(tree::snapshot(&journal.owner.package).unwrap(), actual);
    assert!(journal_path(&root).exists());
}
