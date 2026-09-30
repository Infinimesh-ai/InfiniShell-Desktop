use super::*;
use std::fs;

#[test]
fn native_bridge_root_windows_is_private_and_stable() {
    let home = tempfile::tempdir().unwrap();
    let first = prepare_root(home.path(), "bridge").unwrap();
    let original = files::file_identity(&files::directory(&first).unwrap()).unwrap();
    let second = prepare_root(home.path(), "bridge").unwrap();

    assert_eq!(first, second);
    assert_eq!(
        files::file_identity(&files::directory(&second).unwrap()).unwrap(),
        original
    );
}

#[test]
fn native_bridge_root_windows_does_not_replace_existing_objects() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("bridge");
    fs::write(&path, b"keep").unwrap();

    assert!(prepare_root(home.path(), "bridge").is_err());
    assert_eq!(fs::read(path).unwrap(), b"keep");
}

#[test]
fn native_bridge_root_windows_rejects_escape_and_remote_paths() {
    let home = tempfile::tempdir().unwrap();
    assert!(prepare_root(home.path(), "../outside").is_err());
    assert!(prepare_root(home.path(), "nested/bridge").is_err());
    assert!(pin_ancestors(Path::new(r"\\server\share\bridge")).is_err());
    assert!(pin_ancestors(Path::new(r"\\?\UNC\server\share\bridge")).is_err());
    assert!(pin_ancestors(Path::new("relative")).is_err());
    assert!(fs::read_dir(home.path()).unwrap().next().is_none());
}

#[test]
fn native_bridge_root_windows_pins_ancestor_directories_against_rename() {
    let home = tempfile::tempdir().unwrap();
    let parent = home.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let root = prepare_root(&parent, "bridge").unwrap();
    let handles = pin_ancestors(&root).unwrap();

    assert!(fs::rename(&parent, home.path().join("replacement")).is_err());
    assert!(fs::rename(&root, parent.join("replacement")).is_err());
    drop(handles);
    fs::rename(&parent, home.path().join("replacement")).unwrap();
}

#[test]
fn native_bridge_root_windows_separates_channels_and_profiles() {
    assert_ne!(
        directory_name(Channel::Stable, None),
        directory_name(Channel::Preview, None)
    );
    assert_ne!(
        directory_name(Channel::Oss, None),
        directory_name(Channel::Oss, Some(""))
    );
    assert_ne!(
        directory_name(Channel::Oss, Some("a")),
        directory_name(Channel::Oss, Some("b"))
    );
    assert_eq!(
        directory_name(Channel::Oss, Some(&"p".repeat(1024))).len(),
        34
    );
}
