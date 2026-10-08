use super::*;
use std::io::{Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;

fn connected_control() -> (Control, UnixStream) {
    let (client, server) = UnixStream::pair().unwrap();
    nonblocking(client.as_raw_fd()).unwrap();
    let process = platform::Process::capture(std::process::id() as i32).unwrap();
    let endpoint = process
        .sockets(&Budget::new())
        .unwrap()
        .into_iter()
        .find(|endpoint| endpoint.descriptor == client.as_raw_fd())
        .unwrap();
    let input = ChildStdin::from(OwnedFd::from(client.try_clone().unwrap()));
    let output = ChildStdout::from(OwnedFd::from(client));
    (
        Control {
            child: None,
            input: Some(input),
            output,
            process,
            endpoint,
            buffer: Vec::new(),
            last_number: 0,
            usable: true,
        },
        server,
    )
}

#[test]
fn tmux_argv_encoding_preserves_literals_without_command_separators() {
    let frame = encode(&[
        OsStr::new("split-window"),
        OsStr::new(""),
        OsStr::new("a'b; run-shell x"),
        OsStr::new("$HOME #{pane_id} \\ 中文"),
    ])
    .unwrap();
    assert_eq!(
        frame,
        "'split-window' '' 'a'\\''b; run-shell x' '$HOME #{pane_id} \\ 中文'\n".as_bytes()
    );
}

#[test]
fn tmux_argv_encoding_rejects_frame_injection_and_oversize() {
    assert!(encode(&[OsStr::new("split-window"), OsStr::new("a\nkill-server")]).is_err());
    assert!(encode(&[OsStr::new("split-window"), OsStr::new("a\0b")]).is_err());
    assert!(encode(&[OsStr::new(&"x".repeat(MAX_FRAME))]).is_err());
}

#[test]
fn control_guards_require_all_three_canonical_numeric_fields() {
    assert_eq!(
        guard(b"%begin 123 456 1", b"%begin ").unwrap(),
        (123, 456, 1)
    );
    assert_eq!(guard(b"%end 123 456 1", b"%end ").unwrap(), (123, 456, 1));
    assert!(guard(b"%end 123 456 1 extra", b"%end ").is_err());
    assert!(guard(b"%end 123 -456 1", b"%end ").is_err());
    assert!(guard(b"%end 123 456 18446744073709551616", b"%end ").is_err());
}

#[test]
fn no_output_control_rejects_pane_data_and_unpaired_receipts() {
    assert!(validate_notification(b"%session-changed $0 private-name").is_ok());
    assert!(validate_notification(b"%output %1 secret").is_err());
    assert!(validate_notification(b"%extended-output %1 1 : secret").is_err());
    assert!(validate_notification(b"%end 123 456 1").is_err());
    assert!(validate_notification(b"%exit").is_err());
}

#[test]
fn mismatched_control_receipt_poisoning_prevents_a_second_send() {
    let (mut control, mut peer) = connected_control();
    peer.write_all(b"%begin 123 9 1\n%2\t200\t/dev/ttys002\n%end 123 10 1\n")
        .unwrap();
    assert!(
        control
            .command(&[OsStr::new("list-clients")], &Budget::new())
            .is_err()
    );
    assert!(
        control
            .command(&[OsStr::new("split-window")], &Budget::new())
            .is_err()
    );
    peer.set_nonblocking(true).unwrap();
    let mut observed = [0u8; 128];
    let count = peer.read(&mut observed).unwrap();
    assert_eq!(&observed[..count], b"'list-clients'\n");
    assert_eq!(
        peer.read(&mut observed).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[test]
fn control_accepts_global_number_gaps_but_not_a_replayed_guard() {
    let (mut control, mut peer) = connected_control();
    peer.write_all(b"%begin 123 9 1\nfirst\n%end 123 9 1\n%begin 124 27 1\nsecond\n%end 124 27 1\n%begin 124 27 1\nreplay\n%end 124 27 1\n").unwrap();
    assert_eq!(
        control
            .command(&[OsStr::new("list-clients")], &Budget::new())
            .unwrap(),
        [b"first".to_vec()]
    );
    assert_eq!(
        control
            .command(&[OsStr::new("list-clients")], &Budget::new())
            .unwrap(),
        [b"second".to_vec()]
    );
    assert!(
        control
            .command(&[OsStr::new("list-clients")], &Budget::new())
            .is_err()
    );
}

#[test]
fn guarded_split_requires_both_condition_and_action_receipts() {
    let (mut control, mut peer) = connected_control();
    peer.write_all(
        b"%begin 123 9 1\n%end 123 9 1\n%begin 123 10 1\n%2\t200\t/dev/ttys002\n%end 123 10 1\n",
    )
    .unwrap();
    assert_eq!(
        control
            .guarded_split(&[OsStr::new("if-shell")], &Budget::new())
            .unwrap(),
        [b"%2\t200\t/dev/ttys002".to_vec()]
    );
}

#[test]
fn missing_action_receipt_is_unknown_and_never_resends_the_action() {
    let (mut control, mut peer) = connected_control();
    peer.write_all(b"%begin 123 9 1\n%end 123 9 1\n").unwrap();
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    assert!(
        control
            .guarded_split(&[OsStr::new("if-shell")], &Budget::new())
            .is_err()
    );
    assert!(
        control
            .guarded_split(&[OsStr::new("if-shell")], &Budget::new())
            .is_err()
    );
    peer.set_nonblocking(true).unwrap();
    let mut observed = [0u8; 128];
    let count = peer.read(&mut observed).unwrap();
    assert_eq!(&observed[..count], b"'if-shell'\n");
}
