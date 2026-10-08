use super::*;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};

fn root() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = fs::canonicalize(directory.path()).unwrap();
    (directory, path)
}

#[test]
fn missing_project_parents_are_observed_without_creation() {
    let (_directory, root) = root();
    let session = Uuid::new_v4();
    let path = root
        .join("projects/new-project")
        .join(format!("{session}.jsonl"));
    let guard = TranscriptPathGuard::capture(&root, &path, session).unwrap();
    guard.verify().unwrap();
    assert!(!root.join("projects").exists());
    assert!(
        matches!(guard.anchor.open(&path, session), Err(error) if error.kind() == io::ErrorKind::NotFound)
    );
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path.parent().unwrap())
        .unwrap();
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    let (file, complete) = guard.anchor.open(&path, session).unwrap();
    assert_eq!(file.metadata().unwrap().len(), 0);
    assert!(complete.extends(&guard.anchor));
}

#[test]
fn existing_parent_replacement_is_rejected_even_if_permissions_match() {
    let (_directory, root) = root();
    let session = Uuid::new_v4();
    let path = root
        .join("projects/project")
        .join(format!("{session}.jsonl"));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path.parent().unwrap())
        .unwrap();
    let guard = TranscriptPathGuard::capture(&root, &path, session).unwrap();
    fs::rename(root.join("projects"), root.join("original-projects")).unwrap();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path.parent().unwrap())
        .unwrap();
    assert!(guard.verify().is_err());
}

#[test]
fn symlink_in_new_parent_or_leaf_cannot_satisfy_pending_anchor() {
    let (_directory, root) = root();
    let session = Uuid::new_v4();
    let path = root
        .join("projects/project")
        .join(format!("{session}.jsonl"));
    let guard = TranscriptPathGuard::capture(&root, &path, session).unwrap();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(root.join("other"))
        .unwrap();
    symlink(root.join("other"), root.join("projects")).unwrap();
    assert!(guard.anchor.open(&path, session).is_err());
    fs::remove_file(root.join("projects")).unwrap();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path.parent().unwrap())
        .unwrap();
    let other = root.join("other-file");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&other)
        .unwrap();
    symlink(&other, &path).unwrap();
    assert!(guard.anchor.open(&path, session).is_err());
}

#[test]
fn old_session_filename_and_writable_ancestor_are_rejected() {
    let (_directory, root) = root();
    let old_path = root
        .join("projects/project")
        .join(format!("{}.jsonl", Uuid::new_v4()));
    assert!(TranscriptPathGuard::capture(&root, &old_path, Uuid::new_v4()).is_err());
    let session = Uuid::new_v4();
    let path = root
        .join("projects/project")
        .join(format!("{session}.jsonl"));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(root.join("projects"))
        .unwrap();
    fs::set_permissions(root.join("projects"), fs::Permissions::from_mode(0o770)).unwrap();
    assert!(TranscriptPathGuard::capture(&root, &path, session).is_err());
}
