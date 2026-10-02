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

fn downgrade_fixture(old: &str, target: &str, intent: claude_downgrade::Intent) -> Fixture {
    use super::super::{ConfigDesired, ConfigKind};

    let mut fixture = fixture();
    let settings = fixture.root.join("settings.json");
    let before = br#"{"autoUpdatesChannel":"latest","unrelated":"preserved"}"#.to_vec();
    fs::write(&settings, &before).unwrap();
    fixture.journal.old_version = old.into();
    fixture.journal.target_version = target.into();
    fixture.journal.downgrade = Some(intent);
    fixture.journal.config = Some(ConfigBackup {
        kind: ConfigKind::Claude,
        path: settings,
        before: Some(before),
        after: None,
        desired: Some(ConfigDesired {
            bytes: Some(br#"{"autoUpdatesChannel":"stable","unrelated":"preserved"}"#.to_vec()),
        }),
        before_mode: None,
        restore_stage: None,
    });
    fixture
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn consumer_downgrade_crash_restores_original_without_publishing_stable() {
    let fixture = downgrade_fixture(
        "2.1.287",
        "2.1.285",
        claude_downgrade::Intent::ClaudeNpmStable21287To21285,
    );
    let journal = &fixture.journal;
    save(&journal_path(&fixture.root, CLIAgent::Claude), journal).unwrap();
    let parent = journal.owner.verify_external().unwrap();
    parent
        .exchange(std::ffi::OsStr::new("claude-code"), &journal.stage_name)
        .unwrap();

    assert_eq!(
        recover(CLIAgent::Claude, &journal.owner.entry, &fixture.root).unwrap(),
        None
    );
    assert_eq!(
        fs::read(&journal.owner.entry).unwrap(),
        b"old public binary"
    );
    assert_eq!(
        fs::read(&journal.config.as_ref().unwrap().path).unwrap(),
        br#"{"autoUpdatesChannel":"latest","unrelated":"preserved"}"#
    );
    assert!(!journal_path(&fixture.root, CLIAgent::Claude).exists());
    assert!(!parent.has_child(&journal.stage_name).unwrap());
}

#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64")
))]
#[test]
fn historical_downgrade_journal_still_recovers_after_new_intent_is_added() {
    let fixture = downgrade_fixture(
        "2.1.280",
        "2.1.278",
        claude_downgrade::Intent::ClaudeNpmStable21280To21278,
    );
    let journal = &fixture.journal;
    let serialized = serde_json::to_value(journal).unwrap();
    assert_eq!(serialized["downgrade"], "claude_npm_stable21280_to21278");
    save(&journal_path(&fixture.root, CLIAgent::Claude), journal).unwrap();

    assert_eq!(
        recover(CLIAgent::Claude, &journal.owner.entry, &fixture.root).unwrap(),
        None
    );
    assert_eq!(
        fs::read(&journal.owner.entry).unwrap(),
        b"old public binary"
    );
    assert_eq!(
        fs::read(&journal.config.as_ref().unwrap().path).unwrap(),
        br#"{"autoUpdatesChannel":"latest","unrelated":"preserved"}"#
    );
    assert!(!journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn consumer_downgrade_journal_with_removed_intent_cannot_mutate_installation() {
    let fixture = downgrade_fixture(
        "2.1.287",
        "2.1.285",
        claude_downgrade::Intent::ClaudeNpmStable21287To21285,
    );
    let journal = &fixture.journal;
    let mut serialized = serde_json::to_value(journal).unwrap();
    serialized.as_object_mut().unwrap().remove("downgrade");
    fs::write(
        journal_path(&fixture.root, CLIAgent::Claude),
        serde_json::to_vec(&serialized).unwrap(),
    )
    .unwrap();

    assert_eq!(
        recover(CLIAgent::Claude, &journal.owner.entry, &fixture.root),
        Err(Error::RecoveryRequired)
    );
    assert_eq!(
        fs::read(&journal.owner.entry).unwrap(),
        b"old public binary"
    );
    assert!(journal_path(&fixture.root, CLIAgent::Claude).exists());
    assert!(
        journal
            .owner
            .verify_external()
            .unwrap()
            .has_child(&journal.stage_name)
            .unwrap()
    );
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

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn codex_probe_fixture(target: &str) -> Fixture {
    let mut fixture = fixture();
    let journal = &mut fixture.journal;
    journal.agent = "codex".into();
    journal.old_version = "0.155.1".into();
    journal.target_version = target.into();
    journal.owner.package_root = journal.owner.prefix.join("lib/node_modules/@openai/codex");
    journal.owner.public_relative = "bin/codex.js".into();
    journal.owner.entry = journal.owner.prefix.join("bin/codex");
    let stage = journal
        .owner
        .package_root
        .parent()
        .unwrap()
        .join(&journal.stage_name);
    let (wrapper_digest, mut inventory) = match target {
        "0.156.1" => {
            let manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
                "../../../../script/cli-agent-parity/codex_0156_package_manifest.json"
            ))
            .unwrap();
            let native: BTreeMap<PathBuf, (u64, String, u32)> =
                serde_json::from_value(manifest["packages"]["macos-arm64"]["files"].clone())
                    .unwrap();
            let mut files = native
                .into_iter()
                .map(|(path, spec)| (Path::new("vendor/aarch64-apple-darwin").join(path), spec))
                .collect::<BTreeMap<_, _>>();
            // 旧合同的两份平台元数据由归档 SRI 绑定，worker 保留原有有界检查。
            files.insert("package.json".into(), (517, "1".repeat(64), 0o644));
            files.insert("README.md".into(), (3334, "2".repeat(64), 0o644));
            (
                "c3f16464dca0fe1269b17d02fe0997d1ca3a241c3a61da3d89ec13def0a66c6e",
                files,
            )
        }
        "0.160.0" => (
            "29c350dfcd8d33749852c16e2f5dcde528409d7e1d3f5d916fe3c576f3914820",
            serde_json::from_slice(include_bytes!("fixtures/codex-0160/platform-files.json"))
                .unwrap(),
        ),
        _ => panic!("测试仅覆盖两份已审核发行闭包"),
    };
    inventory = inventory
        .into_iter()
        .map(|(path, spec)| {
            (
                Path::new("node_modules/@openai/codex-darwin-arm64").join(path),
                spec,
            )
        })
        .collect();
    inventory.insert(
        "bin/codex.js".into(),
        (
            8790,
            "61b0194f3bb6534439c8d26a3ed57d0805f84b884588b761795323eeb92fcf70".into(),
            0o755,
        ),
    );
    inventory.insert("package.json".into(), (1082, wrapper_digest.into(), 0o644));
    inventory.insert(
        "README.md".into(),
        (
            3334,
            "ba4e1f69ff48386e72a9c5e1edaf76aad64a475c2d51af79ccba6d1128261ba7".into(),
            0o644,
        ),
    );
    let node = fixture.root.join("node");
    fs::write(&node, "仅供账本校验的 Node 身份夹具".as_bytes()).unwrap();
    let node_stamp = stamp(&node).unwrap();
    journal.owner.external_files = vec![node_stamp.clone()];
    let mut expected_files = vec![managed_process::ExpectedFileIdentity::capture(&node).unwrap()];
    // 不执行原生程序；序列化库存仅覆盖恢复 validate 的发行版本关联。
    expected_files.extend(inventory.into_iter().map(|(path, (size, sha256, _mode))| {
        let path = stage.join(path);
        serde_json::from_value(serde_json::json!({
            "path":path, "canonical_path":path, "size":size, "sha256":sha256,
            "file_id":{"volume":1,"index":2}
        }))
        .unwrap()
    }));
    let codex_closure = Some(npm_codex::ProbeClosure {
        arguments: vec![
            stage.join("bin/codex.js").into_os_string(),
            "--version".into(),
        ],
        expected_files,
    });
    let binding_digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&node_stamp, "--version", journal.id, &codex_closure)).unwrap()
        )
    );
    journal.probe = Some(Probe {
        generation: Uuid::new_v4(),
        program: node,
        program_stamp: node_stamp,
        binding_digest,
        observed_version: None,
        codex_closure,
    });
    fixture
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn historical_codex_probe_cannot_be_relabelled_as_current_target() {
    let fixture = codex_probe_fixture("0.156.1");
    let mut journal: Journal =
        serde_json::from_slice(&serde_json::to_vec(&fixture.journal).unwrap()).unwrap();
    assert_eq!(
        validate(CLIAgent::Codex, &journal.owner.entry, &journal),
        Ok(())
    );
    journal.target_version = "0.160.0".into();
    assert_eq!(
        validate(CLIAgent::Codex, &journal.owner.entry, &journal),
        Err(Error::RecoveryRequired)
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_codex_probe_cannot_be_relabelled_as_historical_target() {
    let fixture = codex_probe_fixture("0.160.0");
    let mut journal: Journal =
        serde_json::from_slice(&serde_json::to_vec(&fixture.journal).unwrap()).unwrap();
    assert_eq!(
        validate(CLIAgent::Codex, &journal.owner.entry, &journal),
        Ok(())
    );
    journal.target_version = "0.156.1".into();
    assert_eq!(
        validate(CLIAgent::Codex, &journal.owner.entry, &journal),
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
