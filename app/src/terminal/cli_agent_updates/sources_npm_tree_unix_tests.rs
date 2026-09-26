use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};

use super::*;

fn directory(root: &Path, leaf: &str, bytes: &[u8]) -> Directory {
    fs::create_dir(root.join(leaf)).unwrap();
    fs::write(root.join(leaf).join("payload"), bytes).unwrap();
    Directory::open(&root.join(leaf)).unwrap()
}

#[test]
fn exchange_preserves_two_complete_tree_identities() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let old = directory(&root, "package", b"old").snapshot().unwrap();
    let new = directory(&root, "stage", b"new").snapshot().unwrap();
    let parent = Directory::open(&root).unwrap();

    parent
        .exchange(OsStr::new("package"), OsStr::new("stage"))
        .unwrap();

    assert_eq!(
        parent
            .child(OsStr::new("package"))
            .unwrap()
            .snapshot()
            .unwrap(),
        new
    );
    assert_eq!(
        parent
            .child(OsStr::new("stage"))
            .unwrap()
            .snapshot()
            .unwrap(),
        old
    );
    assert_eq!(fs::read(root.join("package/payload")).unwrap(), b"new");
    assert_eq!(fs::read(root.join("stage/payload")).unwrap(), b"old");
}

#[test]
fn symlink_member_cannot_escape_package_snapshot() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = directory(&root, "package", b"old");
    fs::write(root.join("unrelated"), b"preserved").unwrap();
    symlink(root.join("unrelated"), root.join("package/link")).unwrap();

    assert!(package.snapshot().is_err());
    assert_eq!(fs::read(root.join("unrelated")).unwrap(), b"preserved");
}

#[test]
fn linked_regular_member_is_not_an_owned_single_package_file() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = directory(&root, "package", b"old");
    fs::hard_link(root.join("package/payload"), root.join("shared")).unwrap();

    assert!(package.snapshot().is_err());
    assert_eq!(fs::read(root.join("shared")).unwrap(), b"old");
}

#[test]
fn executable_and_private_file_modes_survive_replacement() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let old = directory(&root, "old", b"old");
    fs::set_permissions(root.join("old/payload"), fs::Permissions::from_mode(0o500)).unwrap();
    let old = old.snapshot().unwrap();
    let new = directory(&root, "new", b"new");

    new.apply_permissions(&old, &BTreeMap::new()).unwrap();

    assert_eq!(
        fs::metadata(root.join("new/payload"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o500
    );
}

#[test]
fn changed_backup_is_retained_instead_of_deleted() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let expected = directory(&root, "backup", b"old").snapshot().unwrap();
    fs::write(root.join("backup/payload"), b"external npm change").unwrap();

    assert!(
        Directory::open(&root)
            .unwrap()
            .remove_matching(OsStr::new("backup"), &expected)
            .is_err()
    );
    assert_eq!(
        fs::read(root.join("backup/payload")).unwrap(),
        b"external npm change"
    );
}

#[test]
fn interrupted_cleanup_accepts_only_unchanged_remaining_members() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let backup = directory(&root, "backup", b"old");
    fs::write(root.join("backup/second"), b"keep").unwrap();
    let expected = backup.snapshot().unwrap();
    fs::remove_file(root.join("backup/payload")).unwrap();

    Directory::open(&root)
        .unwrap()
        .remove_matching(OsStr::new("backup"), &expected)
        .unwrap();

    assert!(!root.join("backup").exists());
}

#[test]
fn cleanup_preserves_members_added_after_the_initial_snapshot() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let backup = directory(&root, "backup", b"old");
    let expected = backup.snapshot().unwrap();
    fs::write(root.join("backup/new-member"), b"external change").unwrap();

    assert!(backup.remove_contents(&expected, Path::new("")).is_err());
    assert_eq!(
        fs::read(root.join("backup/new-member")).unwrap(),
        b"external change"
    );
    assert_eq!(fs::read(root.join("backup/payload")).unwrap(), b"old");
}

#[test]
fn cleanup_preserves_nested_same_length_rewrites_after_the_initial_snapshot() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let backup = directory(&root, "backup", b"old");
    fs::create_dir(root.join("backup/nested")).unwrap();
    fs::write(root.join("backup/nested/member"), b"old").unwrap();
    let expected = backup.snapshot().unwrap();
    fs::write(root.join("backup/nested/member"), b"new").unwrap();

    assert!(backup.remove_contents(&expected, Path::new("")).is_err());
    assert_eq!(fs::read(root.join("backup/nested/member")).unwrap(), b"new");
}

#[test]
fn cleanup_preserves_replacement_directory_with_identical_contents() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let backup = directory(&root, "backup", b"old");
    fs::create_dir(root.join("backup/nested")).unwrap();
    fs::write(root.join("backup/nested/member"), b"same").unwrap();
    let expected = backup.snapshot().unwrap();
    fs::rename(root.join("backup/nested"), root.join("original-nested")).unwrap();
    fs::create_dir(root.join("backup/nested")).unwrap();
    fs::write(root.join("backup/nested/member"), b"same").unwrap();

    assert!(backup.remove_contents(&expected, Path::new("")).is_err());
    assert_eq!(
        fs::read(root.join("backup/nested/member")).unwrap(),
        b"same"
    );
    assert_eq!(
        fs::read(root.join("original-nested/member")).unwrap(),
        b"same"
    );
}

#[test]
fn ancestor_symlink_is_rejected_before_writing() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    fs::create_dir(root.join("actual")).unwrap();
    symlink(root.join("actual"), root.join("alias")).unwrap();

    assert!(Directory::open(&root.join("alias")).is_err());
    assert_eq!(fs::read_dir(root.join("actual")).unwrap().count(), 0);
}

#[cfg(target_os = "macos")]
#[test]
fn acl_is_rejected_instead_of_silently_losing_access_rules() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = directory(&root, "package", b"original");
    let status = command::blocking::Command::new("/bin/chmod")
        .args([
            std::ffi::OsStr::new("+a"),
            std::ffi::OsStr::new("everyone allow read"),
            root.join("package/payload").as_os_str(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(package.snapshot().is_err());
    assert_eq!(fs::read(root.join("package/payload")).unwrap(), b"original");
}

#[test]
fn prepared_tree_must_match_every_official_archive_member() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = directory(&root, "package", b"official");
    let expected = BTreeMap::from([(
        PathBuf::from("payload"),
        (8, Sha256::digest(b"official").into()),
    )]);
    package
        .snapshot()
        .unwrap()
        .verify_release_files(&expected)
        .unwrap();
    fs::write(root.join("package/payload"), b"modified").unwrap();
    assert!(
        package
            .snapshot()
            .unwrap()
            .verify_release_files(&expected)
            .is_err()
    );
    fs::write(root.join("package/payload"), b"official").unwrap();
    fs::write(root.join("package/extra-script"), b"not in archive").unwrap();
    assert!(
        package
            .snapshot()
            .unwrap()
            .verify_release_files(&expected)
            .is_err()
    );
}
