use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::future::AbortHandle;
use remote_server::proto::{OpenSshStream, SshStreamPurpose, TunnelChannel, TunnelData};
use remote_server::protocol::INITIAL_TUNNEL_WINDOW;
use remote_server::setup::RemoteOs;
use warpui::r#async::executor;

use super::{
    MAX_IDENTITY_KEY_BYTES, SshStreamOperation, TunnelBroker, TunnelBrokerInner, TunnelProcess,
    next_stdin_offset, return_output_credit, ssh_stream_command, ssh_stream_operation,
    stdin_is_complete,
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
fn old_binary_check_uses_the_legacy_binary_probe() {
    let (current_binary_check, _, _) = ssh_stream_command(
        &open(SshStreamPurpose::CheckBinary),
        "~/.infinishell/remote-server/staged.tar.gz",
        &RemoteOs::Linux,
    )
    .unwrap();
    let (old_binary_check, _, _) = ssh_stream_command(
        &open(SshStreamPurpose::CheckOldBinary),
        "~/.infinishell/remote-server/staged.tar.gz",
        &RemoteOs::Linux,
    )
    .unwrap();
    let expected = crate::remote_server::ssh_transport::setup_command_line(
        &remote_server::setup::old_binary_check_command_for(&RemoteOs::Linux),
    );

    assert_ne!(old_binary_check, current_binary_check);
    assert_eq!(old_binary_check, expected);
}

#[tokio::test]
async fn stdin_byte_window_accepts_more_than_eight_small_frames_before_pump_progress() {
    let (broker, stdin_rx) = broker_with_pending_stdin();

    send_stdin_frame(&broker, 0).await;
    send_stdin_frame(&broker, 8192).await;
    send_stdin_frame(&broker, 16384).await;
    send_stdin_frame(&broker, 24576).await;
    send_stdin_frame(&broker, 32768).await;
    send_stdin_frame(&broker, 40960).await;
    send_stdin_frame(&broker, 49152).await;
    send_stdin_frame(&broker, 57344).await;
    send_stdin_frame(&broker, 65536).await;

    assert!(broker.stream("stream").is_some());
    assert_eq!(stdin_rx.len(), 9);
}

fn broker_with_pending_stdin() -> (TunnelBroker, async_channel::Receiver<Vec<u8>>) {
    let (control_outbound_tx, _control_outbound_rx) = async_channel::unbounded();
    let (outbound_tx, _outbound_rx) = async_channel::unbounded();
    let (stdin_tx, stdin_rx) = async_channel::unbounded();
    let (stdout_credit_tx, _stdout_credit_rx) = async_channel::unbounded();
    let (stderr_credit_tx, _stderr_credit_rx) = async_channel::unbounded();
    let (abort_handle, _) = AbortHandle::new_pair();
    let process = Arc::new(TunnelProcess {
        stdin_tx,
        stdout_credit_tx,
        stderr_credit_tx,
        expected_stdin_offset: AtomicU64::new(0),
        expected_stdin_size: Some(INITIAL_TUNNEL_WINDOW as u64),
        stdin_credit: AtomicUsize::new(INITIAL_TUNNEL_WINDOW),
        stdout_in_flight: AtomicUsize::new(0),
        stderr_in_flight: AtomicUsize::new(0),
        stdout_returned_credit: AtomicUsize::new(0),
        stderr_returned_credit: AtomicUsize::new(0),
        stderr_tail: Mutex::new(VecDeque::new()),
        abort_handle,
        stdin_closed: AtomicBool::new(false),
        finished: AtomicBool::new(false),
    });
    let broker = TunnelBroker {
        inner: Arc::new(TunnelBrokerInner {
            controls: Mutex::new(HashMap::new()),
            streams: Mutex::new(HashMap::from([("stream".to_string(), process)])),
            control_outbound_tx,
            outbound_tx,
            executor: Arc::new(executor::Background::default()),
        }),
    };
    (broker, stdin_rx)
}

async fn send_stdin_frame(broker: &TunnelBroker, offset: u64) {
    broker
        .handle_data(
            "stream",
            TunnelData {
                channel: TunnelChannel::Stdin.into(),
                offset,
                data: vec![0; 8192],
            },
        )
        .await;
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
