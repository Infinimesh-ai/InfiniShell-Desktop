//! 小文件与持久收据夹具覆盖完整恢复分支；不派生 CLI，也不作为原生执行证据。

use std::ffi::OsStr;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

use sha2::Digest as _;

use super::*;

fn linked_fixture() -> (Fixture, ClaudeHardlink) {
    let mut fixture = fixture();
    let platform = target().unwrap();
    let package = &fixture.journal.owner.package_root;
    let native = package.join(format!(
        "node_modules/@anthropic-ai/claude-code-{platform}/claude"
    ));
    fs::create_dir_all(native.parent().unwrap()).unwrap();
    fs::hard_link(package.join("bin/claude.exe"), &native).unwrap();
    let link =
        ClaudeHardlink::new(&platform, 17, Sha256::digest(b"old public binary").into()).unwrap();
    fixture.journal.original = Directory::open(package)
        .unwrap()
        .claude_snapshot(&link)
        .unwrap();
    (fixture, link)
}

fn unit_probe_receipts(fixture: &mut Fixture) {
    let journal = &mut fixture.journal;
    let program = journal
        .owner
        .package_root
        .parent()
        .unwrap()
        .join(&journal.stage_name)
        .join("bin/claude.exe");
    let generation = Uuid::new_v4();
    let cwd = fixture
        .root
        .join(format!("claude-npm-version-{generation}"));
    fs::create_dir(&cwd).unwrap();
    fs::set_permissions(&cwd, fs::Permissions::from_mode(0o700)).unwrap();
    for name in ["home", "config", "cache", "data", "tmp"] {
        fs::create_dir(cwd.join(name)).unwrap();
        fs::set_permissions(cwd.join(name), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let cwd_metadata = fs::metadata(&cwd).unwrap();
    let digest = "1".repeat(64);
    let manifest = serde_json::json!({
        "version":1,"generation":generation,"launch_allowed":true,
        "token":Uuid::new_v4(),"parent_control":"127.0.0.1:0",
        "executable":program,"arguments":vec![OsString::from("--version")],"cwd":cwd,
        "expected_files":[managed_process::ExpectedFileIdentity::capture(&program).unwrap()],
        "atomic_launch_kind":"claude_npm_version_probe_v1",
        "atomic_cwd":{"path":cwd,"canonical_path":cwd,
            "file_id":{"volume":cwd_metadata.dev(),"index":cwd_metadata.ino()}}
    });
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let manifest_sha = format!("{:x}", Sha256::digest(&manifest_bytes));
    // 沿用可读的历史进程组退出收据，夹具只验证持久绑定和文件事务，不声称真实启动。
    let exit = serde_json::json!({"version":1,"generation":generation,
        "cleanup_confirmed":true,"containment":"unix_process_group",
        "exit_reason":"native_exit","exit_code":0,"manifest_sha256":manifest_sha});
    let exit_bytes = serde_json::to_vec(&exit).unwrap();
    let directory = fixture
        .root
        .join("cli-agent-processes")
        .join(generation.to_string());
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(directory.join("manifest.json"), &manifest_bytes).unwrap();
    fs::write(directory.join("exit.json"), &exit_bytes).unwrap();
    fs::write(
        directory.join("launch-binding.json"),
        serde_json::to_vec(&serde_json::json!({
            "version":1,"generation":generation,"manifest_sha256":manifest_sha,
            "binding_digest":digest,"kind":"claude_npm_version_probe_v1"
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        directory.join("exit-binding.json"),
        serde_json::to_vec(&serde_json::json!({
            "version":1,"generation":generation,"manifest_sha256":manifest_sha,
            "binding_digest":digest,"receipt_sha256":format!("{:x}",Sha256::digest(&exit_bytes))
        }))
        .unwrap(),
    )
    .unwrap();
    for name in [
        "manifest.json",
        "exit.json",
        "launch-binding.json",
        "exit-binding.json",
    ] {
        fs::set_permissions(directory.join(name), fs::Permissions::from_mode(0o600)).unwrap();
    }
    journal.probe = Some(Probe {
        generation,
        program: program.clone(),
        program_stamp: stamp(&program).unwrap(),
        binding_digest: digest,
        observed_version: Some(journal.target_version.clone()),
        codex_closure: None,
    });
}

fn committed_fixture() -> (Fixture, ClaudeHardlink) {
    let (mut fixture, link) = linked_fixture();
    unit_probe_receipts(&mut fixture);
    fixture.journal.phase = Phase::Committed;
    let journal = &fixture.journal;
    save(&journal_path(&fixture.root, CLIAgent::Claude), journal).unwrap();
    journal
        .owner
        .verify_external()
        .unwrap()
        .exchange(OsStr::new("claude-code"), &journal.stage_name)
        .unwrap();
    (fixture, link)
}

fn recover_fixture(fixture: &Fixture, link: &ClaudeHardlink) -> Result<Option<String>, Error> {
    let path = journal_path(&fixture.root, CLIAgent::Claude);
    let mut restored: Journal = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    validate(CLIAgent::Claude, &fixture.journal.owner.entry, &restored)?;
    recover_validated(
        CLIAgent::Claude,
        &fixture.root,
        &path,
        &mut restored,
        Some(link),
    )
}

#[test]
fn linked_original_survives_serialized_journal_exchange_rollback() {
    let (fixture, link) = linked_fixture();
    let journal = &fixture.journal;
    save(&journal_path(&fixture.root, CLIAgent::Claude), journal).unwrap();
    journal
        .owner
        .verify_external()
        .unwrap()
        .exchange(OsStr::new("claude-code"), &journal.stage_name)
        .unwrap();

    assert_eq!(recover_fixture(&fixture, &link).unwrap(), None);
    assert_eq!(
        Directory::open(&journal.owner.package_root)
            .unwrap()
            .claude_snapshot(&link)
            .unwrap(),
        journal.original
    );
    assert_eq!(fs::metadata(&journal.owner.entry).unwrap().nlink(), 2);
    assert!(!journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn committed_linked_backup_cleans_through_full_recovery() {
    let (fixture, link) = committed_fixture();

    assert_eq!(
        recover_fixture(&fixture, &link).unwrap(),
        Some("2.1.280".into())
    );
    assert_eq!(
        fs::read(&fixture.journal.owner.entry).unwrap(),
        b"new public binary"
    );
    assert!(
        !fixture
            .journal
            .owner
            .package_root
            .parent()
            .unwrap()
            .join(&fixture.journal.stage_name)
            .exists()
    );
    assert!(!journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn committed_linked_backup_resumes_after_first_name_unlink() {
    let (fixture, link) = committed_fixture();
    let stage = fixture
        .journal
        .owner
        .package_root
        .parent()
        .unwrap()
        .join(&fixture.journal.stage_name);
    fs::remove_file(stage.join("bin/claude.exe")).unwrap();
    fs::remove_dir(stage.join("bin")).unwrap();

    assert_eq!(
        recover_fixture(&fixture, &link).unwrap(),
        Some("2.1.280".into())
    );
    assert_eq!(
        fs::read(&fixture.journal.owner.entry).unwrap(),
        b"new public binary"
    );
    assert!(!stage.exists());
    assert!(!journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn partial_commit_cleanup_preserves_an_external_second_link() {
    let (fixture, link) = committed_fixture();
    let stage = fixture
        .journal
        .owner
        .package_root
        .parent()
        .unwrap()
        .join(&fixture.journal.stage_name);
    let external = fixture.root.join("external");
    fs::rename(stage.join("bin/claude.exe"), &external).unwrap();

    assert!(recover_fixture(&fixture, &link).is_err());
    assert_eq!(fs::read(&external).unwrap(), b"old public binary");
    assert_eq!(
        fs::read(&fixture.journal.owner.entry).unwrap(),
        b"new public binary"
    );
    assert!(stage.exists());
    assert!(journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn public_recovery_does_not_accept_the_small_unit_image_as_a_fixed_release() {
    let (fixture, _) = committed_fixture();

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
        b"new public binary"
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
    assert!(journal_path(&fixture.root, CLIAgent::Claude).exists());
}

#[test]
fn legacy_unlinked_snapshot_roundtrip_keeps_its_existing_shape() {
    let fixture = fixture();
    let value = serde_json::to_value(&fixture.journal).unwrap();
    assert!(value.get("claude_hardlink").is_none());
    assert!(value.get("claude_platform").is_none());
    assert_eq!(value["schema"], 1);
    assert!(
        !fixture
            .journal
            .owner
            .package_root
            .join("node_modules")
            .exists()
    );
    assert_eq!(fixture.journal.original.file_manifest().len(), 1);
    let restored: Journal = serde_json::from_value(value).unwrap();
    save(&journal_path(&fixture.root, CLIAgent::Claude), &restored).unwrap();
    restored
        .owner
        .verify_external()
        .unwrap()
        .exchange("claude-code".as_ref(), &restored.stage_name)
        .unwrap();

    assert_eq!(
        recover(CLIAgent::Claude, &restored.owner.entry, &fixture.root).unwrap(),
        None
    );
    assert_eq!(
        fs::read(&restored.owner.entry).unwrap(),
        b"old public binary"
    );
}

#[test]
fn public_recovery_rejects_an_unknown_original_version_without_removing_the_backup() {
    let (mut fixture, _) = committed_fixture();
    fixture.journal.old_version = "2.1.281".into();
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
        b"new public binary"
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
    assert!(journal_path(&fixture.root, CLIAgent::Claude).exists());
}
