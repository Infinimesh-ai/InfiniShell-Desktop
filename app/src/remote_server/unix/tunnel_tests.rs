use std::sync::atomic::{AtomicUsize, Ordering};

use remote_server::proto::{OpenSshStream, SshStreamPurpose};
use remote_server::protocol::INITIAL_TUNNEL_WINDOW;

use super::{MAX_IDENTITY_KEY_BYTES, return_output_credit, ssh_stream_command};

fn open(purpose: SshStreamPurpose) -> OpenSshStream {
    OpenSshStream {
        control_id: "control".to_string(),
        purpose: purpose.into(),
        stdout_window_bytes: 1,
        stderr_window_bytes: 1,
        identity_key: String::new(),
    }
}

#[test]
fn stream_purpose_maps_to_a_daemon_owned_command() {
    let (command, stdin, accepts_client_stdin) = ssh_stream_command(
        &open(SshStreamPurpose::PreinstallCheck),
        "~/.warp/staged.tar.gz",
    )
    .unwrap();
    assert_eq!(command, "bash -s");
    assert!(stdin.is_some());
    assert!(!accepts_client_stdin);

    let mut proxy = open(SshStreamPurpose::RemoteServerProxy);
    proxy.identity_key = "identity".to_string();
    let (command, stdin, accepts_client_stdin) =
        ssh_stream_command(&proxy, "~/.warp/staged.tar.gz").unwrap();
    assert!(command.contains("remote-server-proxy"));
    assert!(stdin.is_none());
    assert!(accepts_client_stdin);
}

#[test]
fn identity_key_is_rejected_outside_proxy_and_when_oversized() {
    let mut check = open(SshStreamPurpose::CheckBinary);
    check.identity_key = "unexpected".to_string();
    assert!(ssh_stream_command(&check, "~/.warp/staged.tar.gz").is_err());

    let mut proxy = open(SshStreamPurpose::RemoteServerProxy);
    proxy.identity_key = "x".repeat(MAX_IDENTITY_KEY_BYTES + 1);
    assert!(ssh_stream_command(&proxy, "~/.warp/staged.tar.gz").is_err());

    let (command, stdin, accepts_client_stdin) = ssh_stream_command(
        &open(SshStreamPurpose::StageBinary),
        "~/.warp/staged.tar.gz",
    )
    .unwrap();
    assert!(command.contains("cat >"));
    assert!(stdin.is_none());
    assert!(accepts_client_stdin);

    let (_, stdin, accepts_client_stdin) = ssh_stream_command(
        &open(SshStreamPurpose::InstallStagedBinary),
        "~/.warp/staged.tar.gz",
    )
    .unwrap();
    assert!(stdin.is_some());
    assert!(!accepts_client_stdin);
}

#[test]
fn returned_output_credit_coalesces_many_small_updates() {
    let in_flight = AtomicUsize::new(64);
    let returned_credit = AtomicUsize::new(0);

    for _ in 0..64 {
        assert!(return_output_credit(&in_flight, &returned_credit, 1));
    }

    assert_eq!(in_flight.load(Ordering::Acquire), 0);
    assert_eq!(returned_credit.load(Ordering::Acquire), 64);
    assert!(!return_output_credit(
        &AtomicUsize::new(1),
        &AtomicUsize::new(INITIAL_TUNNEL_WINDOW),
        1,
    ));
}
