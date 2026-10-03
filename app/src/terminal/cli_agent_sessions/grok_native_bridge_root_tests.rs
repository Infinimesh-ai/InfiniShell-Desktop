use std::os::unix::fs::{PermissionsExt as _, symlink};

use super::*;

fn home() -> tempfile::TempDir {
    // 验证命令须使用仓库约定的短、私有 TMPDIR；不能绕过产品祖先检查。
    tempfile::Builder::new()
        .prefix("g")
        .rand_bytes(2)
        .tempdir()
        .unwrap()
}

#[test]
fn native_bridge_root_is_private_stable_and_on_the_home_device() {
    let home = home();
    let owner = unsafe { libc::geteuid() };
    let first = prepare_root(home.path(), "b", owner).unwrap();
    let original = fs::symlink_metadata(&first).unwrap();
    let second = prepare_root(home.path(), "b", owner).unwrap();

    assert_eq!(first, second);
    assert_eq!(original.mode() & 0o7777, 0o700);
    assert_eq!(original.uid(), owner);
    assert_eq!(original.dev(), fs::metadata(home.path()).unwrap().dev());
    assert_eq!(
        identity(&original),
        identity(&fs::metadata(second).unwrap())
    );
}

#[test]
fn native_bridge_root_rejects_existing_permissions_without_repairing_them() {
    let home = home();
    let root = home.path().join("b");
    DirBuilder::new().mode(0o700).create(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();

    assert!(prepare_root(home.path(), "b", unsafe { libc::geteuid() }).is_err());
    assert_eq!(fs::metadata(root).unwrap().mode() & 0o7777, 0o750);
}

#[test]
fn native_bridge_root_rejects_symlinks_to_private_directories() {
    let home = home();
    let target = home.path().join("target");
    DirBuilder::new().mode(0o700).create(&target).unwrap();
    symlink(&target, home.path().join("b")).unwrap();

    assert!(prepare_root(home.path(), "b", unsafe { libc::geteuid() }).is_err());
    assert!(fs::read_dir(target).unwrap().next().is_none());
}

#[test]
fn native_bridge_root_rejects_writable_ancestors_before_creating_a_directory() {
    let home = home();
    fs::set_permissions(home.path(), fs::Permissions::from_mode(0o770)).unwrap();

    assert!(prepare_root(home.path(), "b", unsafe { libc::geteuid() }).is_err());
    assert!(!home.path().join("b").exists());
    assert_eq!(fs::metadata(home.path()).unwrap().mode() & 0o7777, 0o770);
}

#[test]
fn native_bridge_root_reserves_space_for_the_complete_native_socket_path() {
    let home = home();
    let name = "x".repeat(104);

    assert!(prepare_root(home.path(), &name, unsafe { libc::geteuid() }).is_err());
    assert!(!home.path().join(name).exists());
}

#[test]
fn native_bridge_root_separates_channels_and_profiles_in_short_names() {
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
