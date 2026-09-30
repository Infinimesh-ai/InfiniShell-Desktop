use super::*;
use command::managed::linux_peer_handle;
use std::fs::{self, OpenOptions};
use std::io::{Seek as _, SeekFrom, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _, symlink};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn snapshot() -> LinuxProcessSnapshot {
    LinuxProcessSnapshot {
        identity: LinuxProcessIdentity {
            pid: 17,
            start_time_ticks: 19,
            proc_inode: 21,
            pid_namespace_device: 4,
            pid_namespace_inode: 23,
            uid: unsafe { libc::geteuid() },
            executable_device: 5,
            executable_inode: 25,
        },
        parent_pid: 9,
        process_group: 17,
        session: 9,
        tty_device: 123,
        foreground_group: 17,
        executable: PathBuf::new(),
        arguments: Vec::new(),
    }
}

#[test]
fn terminal_bridge_requires_the_shell_session_and_foreground_process_group() {
    let mut peer = snapshot();
    let mut shell = snapshot();
    shell.process_group = 9;
    assert!(terminal_matches(&peer, &shell, 123));

    peer.session = 11;
    assert!(!terminal_matches(&peer, &shell, 123));
    peer.session = 0;
    shell.session = 0;
    assert!(!terminal_matches(&peer, &shell, 123));
    peer.session = 9;
    shell.session = 9;
    peer.process_group = 16;
    assert!(!terminal_matches(&peer, &shell, 123));
    peer.process_group = 17;
    shell.foreground_group = 16;
    assert!(!terminal_matches(&peer, &shell, 123));
    shell.foreground_group = 17;
    peer.process_group = 0;
    peer.foreground_group = 0;
    shell.foreground_group = 0;
    assert!(!terminal_matches(&peer, &shell, 123));
}

#[test]
fn terminal_bridge_requires_both_owners_and_the_real_slave() {
    let mut peer = snapshot();
    let mut shell = snapshot();
    assert!(!terminal_matches(&peer, &shell, 0));
    peer.tty_device = 124;
    assert!(!terminal_matches(&peer, &shell, 123));
    peer.tty_device = 123;
    shell.tty_device = 124;
    assert!(!terminal_matches(&peer, &shell, 123));
    shell.tty_device = 123;
    peer.identity.uid = unsafe { libc::geteuid() }.wrapping_add(1);
    assert!(!terminal_matches(&peer, &shell, 123));
    peer.identity.uid = unsafe { libc::geteuid() };
    shell.identity.uid = unsafe { libc::geteuid() }.wrapping_add(1);
    assert!(!terminal_matches(&peer, &shell, 123));
}

#[test]
fn terminal_bridge_linux_snapshot_cannot_impersonate_a_macos_snapshot() {
    let process = NativeBridgeProcess::from(snapshot().identity);
    let mut value = serde_json::to_value(process).unwrap();
    assert_eq!(value["platform"], "linux");
    assert!(value.get("unique_id").is_none());
    assert_eq!(
        serde_json::from_value::<NativeBridgeProcess>(value.clone()).unwrap(),
        process
    );

    value["platform"] = "macos".into();
    assert!(serde_json::from_value::<NativeBridgeProcess>(value.clone()).is_err());
    value["platform"] = "linux".into();
    value["unique_id"] = 19.into();
    assert!(serde_json::from_value::<NativeBridgeProcess>(value).is_err());
}

#[test]
fn terminal_bridge_same_owner_socket_does_not_authorize_an_unlisted_image() {
    let (stream, _server) = UnixStream::pair().unwrap();
    let peer = linux_peer_handle(&stream).unwrap();
    assert_eq!(peer.identity().pid as u32, std::process::id());
    assert!(
        matches!(Artifact::capture(&peer), Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
    );
}

#[test]
fn terminal_bridge_digest_requires_the_complete_fixed_content() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("image");
    fs::write(&path, b"abc").unwrap();
    let mut file = File::open(&path).unwrap();
    let stamp = FileStamp::of(&file.metadata().unwrap());
    verify_digest(
        &mut file,
        stamp,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    )
    .unwrap();

    file.seek(SeekFrom::Start(1)).unwrap();
    assert!(
        verify_digest(
            &mut file,
            stamp,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        )
        .is_err()
    );
    file.seek(SeekFrom::Start(0)).unwrap();
    assert!(
        verify_digest(
            &mut file,
            stamp,
            "0000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err()
    );
    assert!(verify_digest(&mut file, stamp, "").is_err());
}

#[test]
fn terminal_bridge_digest_rejects_mutation_since_the_pre_hash_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("image");
    let mut writer = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o700)
        .open(&path)
        .unwrap();
    writer.write_all(b"abc").unwrap();
    let mut file = File::open(&path).unwrap();
    let stamp = FileStamp::of(&file.metadata().unwrap());
    writer.write_all(b"changed").unwrap();

    assert!(
        verify_digest(
            &mut file,
            stamp,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        )
        .is_err()
    );
}

#[test]
fn terminal_bridge_artifact_rejects_aliases_and_writable_images() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("image");
    fs::write(&path, b"fixture").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    validate_artifact(&fs::symlink_metadata(&path).unwrap()).unwrap();
    let alias = root.path().join("alias");
    symlink(&path, &alias).unwrap();
    assert!(validate_artifact(&fs::symlink_metadata(&alias).unwrap()).is_err());
    fs::remove_file(&alias).unwrap();
    fs::hard_link(&path, &alias).unwrap();
    assert!(validate_artifact(&fs::symlink_metadata(&path).unwrap()).is_err());
    fs::remove_file(&alias).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o720)).unwrap();
    assert!(validate_artifact(&fs::symlink_metadata(&path).unwrap()).is_err());
}

#[test]
fn terminal_bridge_retained_image_rejects_unlink_and_path_replacement() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("image");
    fs::write(&path, b"fixture").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let file = File::open(&path).unwrap();
    let stamp = FileStamp::of(&file.metadata().unwrap());
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"fixture").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();

    assert!(!stamp.matches(&fs::metadata(&path).unwrap()));
    assert!(!stamp.matches(&file.metadata().unwrap()));
    assert!(validate_artifact(&file.metadata().unwrap()).is_err());
}
