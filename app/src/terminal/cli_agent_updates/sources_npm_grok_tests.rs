//! 小文件只覆盖真实回滚发布块；固定发行身份、原生退出与 CLI 验收不由此替代。

use std::os::unix::fs::symlink;

use super::super::ConfigKind;
use super::super::package_tree::grok_mirror::tests::{change_new_readonly_acl, published_fixture};
use super::*;

struct Fixture {
    _temporary: tempfile::TempDir,
    journal: Journal,
    parent: Directory,
}

fn fixture(existing_new: bool) -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let prefix = root.join("prefix");
    let parent_path = prefix.join("lib/node_modules/@xai-official");
    let package = parent_path.join("grok");
    let home = root.join("home");
    let id = Uuid::new_v4();
    let stage = OsString::from(format!(".infinishell-grok-npm-{id}"));
    fs::create_dir_all(&package).unwrap();
    fs::create_dir(parent_path.join(&stage)).unwrap();
    fs::write(package.join("payload"), b"old package").unwrap();
    fs::write(parent_path.join(&stage).join("payload"), b"new package").unwrap();
    let parent = Directory::open(&parent_path).unwrap();
    let original = parent
        .child(OsStr::new("grok"))
        .unwrap()
        .grok_snapshot()
        .unwrap();
    let prepared = parent.child(&stage).unwrap().grok_snapshot().unwrap();
    let mirror = published_fixture(&home.join("bin"), id, existing_new);
    let journal = Journal {
        schema: 1,
        id,
        old_version: "1.0.40".into(),
        target_version: "1.0.41".into(),
        intent: "fixture".into(),
        owner: Owner {
            prefix_identity: Directory::open(&prefix).unwrap().identity().unwrap(),
            parent_identity: parent.identity().unwrap(),
            prefix,
            package,
            entry: home.join("bin/grok"),
            external_link: None,
            external_files: Vec::new(),
            home: home.clone(),
        },
        stage,
        phase: Phase::SwapIntent,
        original,
        prepared: Some(prepared),
        mirror,
        config: ConfigBackup {
            kind: ConfigKind::Grok,
            path: home.join("config.toml"),
            before: None,
            after: None,
            desired: None,
            before_mode: None,
            restore_stage: None,
        },
        probe: None,
        archives: Vec::new(),
    };
    parent.exchange(OsStr::new("grok"), &journal.stage).unwrap();
    Fixture {
        _temporary: temporary,
        journal,
        parent,
    }
}

fn link_identity(path: &Path) -> (u64, u64, PathBuf) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino(), fs::read_link(path).unwrap())
}

#[test]
fn changed_mirror_acl_rejects_rollback_before_package_or_link_exchange() {
    for existing_new in [false, true] {
        let fixture = fixture(existing_new);
        let journal = &fixture.journal;
        let bin = journal.owner.home.join("bin");
        let staged_link = bin.join(format!(".infinishell-grok-npm-link-{}", journal.id));
        let public_before = link_identity(&bin.join("grok"));
        let stage_before = link_identity(&staged_link);
        change_new_readonly_acl(&journal.mirror);
        let changed = fs::metadata(bin.join("grok-1.0.41")).unwrap();

        assert!(rollback_publication(journal, &fixture.parent).is_err());
        assert_eq!(
            fixture
                .parent
                .child(OsStr::new("grok"))
                .unwrap()
                .grok_snapshot()
                .unwrap(),
            *journal.prepared.as_ref().unwrap()
        );
        assert_eq!(
            fixture
                .parent
                .child(&journal.stage)
                .unwrap()
                .grok_snapshot()
                .unwrap(),
            journal.original
        );
        assert_eq!(link_identity(&bin.join("grok")), public_before);
        assert_eq!(link_identity(&staged_link), stage_before);
        let after = fs::metadata(bin.join("grok-1.0.41")).unwrap();
        assert_eq!(
            (after.ctime(), after.ctime_nsec()),
            (changed.ctime(), changed.ctime_nsec())
        );
    }
}

#[test]
fn rollback_accepts_restored_links_and_partially_cleaned_candidate() {
    for existing_new in [false, true] {
        let fixture = fixture(existing_new);
        let journal = &fixture.journal;
        let bin = journal.owner.home.join("bin");
        if existing_new {
            // 另一可恢复阶段：镜像链接已回滚，stage 仍是原登记的新链接。
            Directory::open(&bin)
                .unwrap()
                .exchange(
                    OsStr::new("grok"),
                    &OsString::from(format!(".infinishell-grok-npm-link-{}", journal.id)),
                )
                .unwrap();
        }
        assert_eq!(rollback_publication(journal, &fixture.parent), Ok(()));
        fs::remove_file(
            journal
                .owner
                .package
                .parent()
                .unwrap()
                .join(&journal.stage)
                .join("payload"),
        )
        .unwrap();
        assert_eq!(rollback_publication(journal, &fixture.parent), Ok(()));
        assert_eq!(
            fs::read(journal.owner.package.join("payload")).unwrap(),
            b"old package"
        );
        assert_eq!(
            fs::read_link(bin.join("grok")).unwrap(),
            Path::new("grok-1.0.40")
        );
        assert_eq!(bin.join("grok-1.0.41").exists(), existing_new);
    }
}

#[test]
fn published_mirror_requires_the_original_stage_link_before_package_exchange() {
    for replace_with_new_target in [false, true] {
        let fixture = fixture(false);
        let journal = &fixture.journal;
        let bin = journal.owner.home.join("bin");
        let staged_link = bin.join(format!(".infinishell-grok-npm-link-{}", journal.id));
        let public_before = link_identity(&bin.join("grok"));
        fs::remove_file(&staged_link).unwrap();
        if replace_with_new_target {
            symlink("grok-1.0.41", &staged_link).unwrap();
        }
        assert!(rollback_publication(journal, &fixture.parent).is_err());
        assert_eq!(link_identity(&bin.join("grok")), public_before);
        assert_eq!(
            fixture
                .parent
                .child(OsStr::new("grok"))
                .unwrap()
                .grok_snapshot()
                .unwrap(),
            *journal.prepared.as_ref().unwrap()
        );
    }
}
