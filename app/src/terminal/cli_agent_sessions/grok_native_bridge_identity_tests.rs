use super::*;
use command::managed::macos_peer_handle;
use std::io::Write as _;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::os::unix::net::UnixStream;

fn snapshot() -> MacosTerminalSnapshot {
    MacosTerminalSnapshot {
        identity: MacosProcessIdentity {
            pid: 17,
            pid_version: 2,
            unique_id: 19,
            resource_cid: 3,
        },
        uid: unsafe { libc::geteuid() },
        tty_device: 123,
        process_group: 17,
        foreground_group: 17,
    }
}

#[test]
fn terminal_requires_the_real_slave_and_same_foreground_group() {
    let peer = snapshot();
    let mut shell = snapshot();
    shell.process_group = 9;
    assert!(terminal_matches(&peer, &shell, 123));
    for changed in [
        MacosTerminalSnapshot {
            uid: peer.uid.wrapping_add(1),
            ..peer
        },
        MacosTerminalSnapshot {
            tty_device: 124,
            ..peer
        },
        MacosTerminalSnapshot {
            process_group: 0,
            foreground_group: 0,
            ..peer
        },
        MacosTerminalSnapshot {
            process_group: 16,
            ..peer
        },
        MacosTerminalSnapshot {
            foreground_group: 16,
            ..peer
        },
    ] {
        assert!(!terminal_matches(&changed, &shell, 123));
    }
    shell.tty_device = 124;
    assert!(!terminal_matches(&peer, &shell, 123));
    assert!(!terminal_matches(&peer, &peer, 0));
}

#[test]
fn known_pid_and_same_owner_do_not_replace_the_dynamic_signature() {
    let (stream, _other) = UnixStream::pair().unwrap();
    let peer = macos_peer_handle(&stream).unwrap();
    assert_eq!(peer.identity().pid as u32, std::process::id());
    // 测试进程虽持有真实同 UID 凭据，也不是固定 Grok 主映像。
    assert_eq!(
        verify_code(&peer).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn artifact_stamp_rejects_replacement_and_in_place_mutation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("grok");
    let mut file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .mode(0o700)
        .open(&path)
        .unwrap();
    file.write_all(b"first").unwrap();
    let stamp = FileStamp::of(&file.metadata().unwrap());
    assert!(stamp.matches(&fs::symlink_metadata(&path).unwrap()));
    file.write_all(b" changed").unwrap();
    assert!(!stamp.matches(&file.metadata().unwrap()));
    let stamp = FileStamp::of(&file.metadata().unwrap());
    fs::rename(&path, root.path().join("original")).unwrap();
    fs::write(&path, b"first changed").unwrap();
    assert!(!stamp.matches(&fs::symlink_metadata(&path).unwrap()));
}

#[test]
fn artifact_metadata_rejects_symlinks_hardlinks_and_writable_images() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("grok");
    fs::write(&path, b"fixture").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    validate_artifact(&fs::symlink_metadata(&path).unwrap()).unwrap();
    let link = root.path().join("alias");
    symlink(&path, &link).unwrap();
    assert!(validate_artifact(&fs::symlink_metadata(&link).unwrap()).is_err());
    fs::remove_file(&link).unwrap();
    fs::hard_link(&path, &link).unwrap();
    assert!(validate_artifact(&fs::symlink_metadata(&path).unwrap()).is_err());
    fs::remove_file(&link).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o720)).unwrap();
    assert!(validate_artifact(&fs::symlink_metadata(&path).unwrap()).is_err());
}
