use warpui::r#async::BoxFuture;

use super::*;

fn static_auth_context() -> Arc<RemoteServerAuthContext> {
    Arc::new(RemoteServerAuthContext::new(
        || -> BoxFuture<'static, Option<String>> { Box::pin(async { None }) },
        || "user id/with spaces".to_string(),
    ))
}

#[test]
fn exit_255_only_invalidates_an_openssh_control_master() {
    let exit_status = RemoteServerExitStatus {
        code: Some(255),
        signal_killed: false,
    };
    let control_master = SshTransport::new(
        PathBuf::from("/tmp/control-master.sock"),
        static_auth_context(),
        true,
    );
    let rust_broker = SshTransport::new_rust_broker(
        "127.0.0.1:1".to_string(),
        "capability".to_string(),
        static_auth_context(),
        RemoteOs::Linux,
    );

    assert!(!control_master.is_reconnectable(Some(&exit_status)));
    assert!(rust_broker.is_reconnectable(Some(&exit_status)));
}

#[test]
fn a_signal_kill_invalidates_every_ssh_backend() {
    let exit_status = RemoteServerExitStatus {
        code: None,
        signal_killed: true,
    };
    let control_master = SshTransport::new(
        PathBuf::from("/tmp/control-master.sock"),
        static_auth_context(),
        true,
    );
    let rust_broker = SshTransport::new_rust_broker(
        "127.0.0.1:1".to_string(),
        "capability".to_string(),
        static_auth_context(),
        RemoteOs::Linux,
    );

    assert!(!control_master.is_reconnectable(Some(&exit_status)));
    assert!(!rust_broker.is_reconnectable(Some(&exit_status)));
}

#[test]
fn remote_proxy_command_quotes_identity_key() {
    let transport = SshTransport::new(
        PathBuf::from("/tmp/control-master.sock"),
        static_auth_context(),
        true,
    );

    let command = transport.remote_proxy_command();

    assert!(
        command
            .script
            .contains("remote-server-proxy --identity-key")
    );
    assert!(command.script.contains("'user id/with spaces'"));
}

#[test]
fn powershell_setup_commands_use_utf16_encoded_command() {
    let command = RemoteSetupCommand {
        dialect: RemoteShellDialect::PowerShell,
        script: "[Console]::Out.WriteLine('Windows')".to_owned(),
    };
    let command_line = setup_command_line(&command);
    let encoded = command_line.split_whitespace().last().unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    let utf16 = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    assert_eq!(
        String::from_utf16(&utf16).unwrap(),
        "[Console]::Out.WriteLine('Windows')"
    );
}

#[test]
fn posix_upload_command_streams_to_a_home_relative_file() {
    let command = upload_command(
        &RemoteOs::Linux,
        "~/.warp-test/remote-server/archive.tar.gz",
    );

    assert_eq!(command.dialect, RemoteShellDialect::Posix);
    assert!(
        command
            .script
            .contains("$HOME/.warp-test/remote-server/archive.tar.gz")
    );
    assert!(command.script.contains("cat > \"$path\""));
}

#[test]
fn windows_upload_command_uses_binary_standard_input() {
    let command = upload_command(&RemoteOs::Windows, "~/.warp-test/remote-server/archive.zip");

    assert_eq!(command.dialect, RemoteShellDialect::PowerShell);
    assert!(command.script.contains("[Console]::OpenStandardInput()"));
    assert!(command.script.contains("$source.CopyTo($destination)"));
    assert!(
        command
            .script
            .contains("'.warp-test\\remote-server\\archive.zip'")
    );
}
