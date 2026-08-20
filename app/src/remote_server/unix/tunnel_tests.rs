use std::sync::atomic::{AtomicUsize, Ordering};

use remote_server::proto::{OpenSshStream, SshStreamPurpose};
use remote_server::protocol::INITIAL_TUNNEL_WINDOW;
use remote_server::setup::RemoteOs;

use super::{
    MAX_IDENTITY_KEY_BYTES, SshStreamOperation, next_stdin_offset, return_output_credit,
    ssh_stream_command, ssh_stream_operation, stdin_is_complete,
};

fn open(purpose: SshStreamPurpose) -> OpenSshStream {
    OpenSshStream {
        control_id: "control".to_string(),
        purpose: purpose.into(),
        stdout_window_bytes: 1,
        stderr_window_bytes: 1,
        identity_key: String::new(),
        stdin_size_bytes: 0,
    }
}

#[test]
fn stream_purpose_maps_to_a_daemon_owned_command() {
    let (command, stdin, accepts_client_stdin) = ssh_stream_command(
        &open(SshStreamPurpose::PreinstallCheck),
        "~/.warp/staged.tar.gz",
        &RemoteOs::Linux,
    )
    .unwrap();
    assert_eq!(command, "bash -s");
    assert!(stdin.is_some());
    assert!(!accepts_client_stdin);

    let mut proxy = open(SshStreamPurpose::RemoteServerProxy);
    proxy.identity_key = "identity".to_string();
    let (command, stdin, accepts_client_stdin) =
        ssh_stream_command(&proxy, "~/.warp/staged.tar.gz", &RemoteOs::Linux).unwrap();
    assert!(command.contains("remote-server-proxy"));
    assert!(stdin.is_none());
    assert!(accepts_client_stdin);
}

#[test]
fn identity_key_is_rejected_outside_proxy_and_when_oversized() {
    let mut check = open(SshStreamPurpose::CheckBinary);
    check.identity_key = "unexpected".to_string();
    assert!(ssh_stream_command(&check, "~/.warp/staged.tar.gz", &RemoteOs::Linux).is_err());

    let mut proxy = open(SshStreamPurpose::RemoteServerProxy);
    proxy.identity_key = "x".repeat(MAX_IDENTITY_KEY_BYTES + 1);
    assert!(ssh_stream_command(&proxy, "~/.warp/staged.tar.gz", &RemoteOs::Linux).is_err());

    let (command, stdin, accepts_client_stdin) = ssh_stream_command(
        &open(SshStreamPurpose::StageBinary),
        "~/.warp/staged.tar.gz",
        &RemoteOs::Linux,
    )
    .unwrap();
    assert!(command.contains("cat >"));
    assert!(stdin.is_none());
    assert!(accepts_client_stdin);

    let (_, stdin, accepts_client_stdin) = ssh_stream_command(
        &open(SshStreamPurpose::InstallStagedBinary),
        "~/.warp/staged.tar.gz",
        &RemoteOs::Linux,
    )
    .unwrap();
    assert!(stdin.is_some());
    assert!(!accepts_client_stdin);
}

#[test]
fn windows_stream_purposes_use_powershell_and_client_staging() {
    let (detect, stdin, _) = ssh_stream_command(
        &open(SshStreamPurpose::DetectPlatform),
        "~/.warp/staged.zip",
        &RemoteOs::Windows,
    )
    .unwrap();
    assert!(detect.starts_with("powershell.exe "));
    assert!(stdin.is_none());

    let (install, stdin, accepts_client_stdin) = ssh_stream_command(
        &open(SshStreamPurpose::InstallStagedBinary),
        "~/.warp/staged.zip",
        &RemoteOs::Windows,
    )
    .unwrap();
    assert!(install.starts_with("powershell.exe "));
    assert!(stdin.is_none());
    assert!(!accepts_client_stdin);
}

#[test]
fn declared_stage_size_selects_the_transport_upload_operation() {
    let mut stage = open(SshStreamPurpose::StageBinary);
    stage.stdin_size_bytes = 4096;

    let operation = ssh_stream_operation(
        &stage,
        "legacy upload command".to_string(),
        "~/.infinishell/remote-server/staged.zip",
        &RemoteOs::Windows,
    );

    assert_eq!(
        operation,
        SshStreamOperation::Upload {
            remote_path: "~/.infinishell/remote-server/staged.zip".to_string(),
            size: 4096,
            windows: true,
        }
    );
}

#[test]
fn missing_stage_size_preserves_the_legacy_exec_operation() {
    let operation = ssh_stream_operation(
        &open(SshStreamPurpose::StageBinary),
        "legacy upload command".to_string(),
        "~/.infinishell/remote-server/staged.tar.gz",
        &RemoteOs::Linux,
    );

    assert_eq!(
        operation,
        SshStreamOperation::Exec("legacy upload command".to_string())
    );
}

#[test]
fn declared_stage_size_rejects_an_overflowing_frame() {
    assert_eq!(next_stdin_offset(Some(8), 0, 8), Some(8));
    assert_eq!(next_stdin_offset(Some(8), 0, 9), None);
    assert_eq!(next_stdin_offset(Some(u64::MAX), u64::MAX, 1), None);
}

#[test]
fn declared_stage_size_requires_a_complete_half_close() {
    assert!(stdin_is_complete(Some(8), 8));
    assert!(!stdin_is_complete(Some(8), 7));
    assert!(stdin_is_complete(None, 7));
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
