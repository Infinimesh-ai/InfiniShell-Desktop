use super::*;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::os::unix::net::UnixListener;

#[test]
fn challenge_accepts_only_the_dedicated_complete_frame() {
    let frame = b"\x1b]9278;t;1;42;11111111-1111-4111-8111-111111111111;22222222-2222-4222-8222-222222222222\x07";
    assert!(validate_challenge(frame).is_ok());
    assert!(validate_challenge(b"\x1b]777;notify;SessionStart\x07").is_err());
    assert!(validate_challenge(&frame[..frame.len() - 1]).is_err());
    assert!(validate_challenge(b"\x1b]9278;t;1;42;11111111-1111-4111-8111-111111111111;00000000-0000-0000-0000-000000000000\x07").is_err());
    assert!(validate_challenge(b"\x1b]9278;t;1;42;11111111-1111-4111-8111-111111111111;22222222-2222-4222-8222-222222222222;extra\x07").is_err());
}

#[test]
fn client_metadata_refuses_ambiguous_or_independent_panes() {
    let ordinary = b"100\t/dev/ttys001\tattached,UTF-8\t$0\t@1\t%2\t200\t/dev/ttys002".to_vec();
    let row = parse_client(&[ordinary.clone()]).unwrap();
    assert_eq!(row.pane_id, "%2");
    assert_eq!(row.session_id, "$0");
    assert!(parse_client(&[ordinary.clone(), ordinary]).is_err());
    assert!(
        parse_client(&[
            b"100\t/dev/ttys001\tattached,active-pane\t$0\t@1\t%2\t200\t/dev/ttys002".to_vec()
        ])
        .is_err()
    );
    assert!(
        parse_client(&[
            b"100\t/dev/ttys001\tattached,read-only\t$0\t@1\t%2\t200\t/dev/ttys002".to_vec()
        ])
        .is_err()
    );
}

#[test]
fn pane_result_rejects_commands_disguised_as_ids_or_tty_paths() {
    assert!(parse_pane(&[b"%2;kill-server\t200\t/dev/ttys002".to_vec()]).is_err());
    assert!(parse_pane(&[b"%2\t4294967295\t/dev/ttys002".to_vec()]).is_err());
    assert!(parse_pane(&[b"%2\t200\t/dev/ttys002\n%3".to_vec()]).is_err());
    assert_eq!(
        parse_pane(&[b"%2\t200\t/dev/ttys002".to_vec()]).unwrap(),
        ("%2".to_owned(), 200, PathBuf::from("/dev/ttys002"))
    );
}

#[test]
fn private_socket_rejects_a_link_and_a_shared_parent() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let socket = directory.path().join("tmux.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    let identity = private_socket(&socket).unwrap();
    assert_ne!(identity.1, 0);
    let alias = directory.path().join("alias.sock");
    symlink(&socket, &alias).unwrap();
    assert!(private_socket(&alias).is_err());
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o750)).unwrap();
    assert!(private_socket(&socket).is_err());
}

#[test]
fn unix_reverse_endpoints_agree_with_real_kernel_identity() {
    let process = platform::Process::capture(std::process::id() as i32).unwrap();
    let (first, second) = UnixStream::pair().unwrap();
    let endpoints = process.sockets(&Budget::new()).unwrap();
    let first = endpoints
        .iter()
        .find(|endpoint| endpoint.descriptor == first.as_raw_fd())
        .unwrap();
    let second = endpoints
        .iter()
        .find(|endpoint| endpoint.descriptor == second.as_raw_fd())
        .unwrap();
    assert!(first.connects_to(second));
    assert!(!first.connects_to(first));
    let mut wrong_generation = second.clone();
    wrong_generation.protocol = wrong_generation.protocol.wrapping_add(1);
    assert!(!first.connects_to(&wrong_generation));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_stream_peer_is_bound_to_the_real_kernel_lifetime() {
    let process = platform::Process::capture(std::process::id() as i32).unwrap();
    let (stream, _peer) = UnixStream::pair().unwrap();
    platform::validate_peer(&stream, &process, &Budget::new()).unwrap();
}

#[test]
fn process_identity_snapshot_uses_the_current_executable_and_uid() {
    let process = platform::Process::capture(std::process::id() as i32).unwrap();
    let snapshot = process.snapshot().unwrap();
    assert_eq!(snapshot.uid, unsafe { libc::geteuid() });
    assert_eq!(snapshot.pid as u32, std::process::id());
    assert!(snapshot.executable.is_absolute());
    assert!(snapshot.cwd.is_absolute());
    assert_eq!(snapshot.executable_file, {
        let image = fs::metadata(&snapshot.executable).unwrap();
        (image.dev(), image.ino())
    });
}
