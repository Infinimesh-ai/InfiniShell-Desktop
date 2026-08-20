use super::*;

const BASIC_CONFIG: &str = r#"
host test
hostname 192.0.2.10
user alice
port 2222
stricthostkeychecking yes
userknownhostsfile ~/.ssh/known_hosts ~/.ssh/known_hosts2
globalknownhostsfile none
identityfile ~/.ssh/id_ed25519
identityfile ~/.ssh/id_rsa
identitiesonly no
passwordauthentication yes
kbdinteractiveauthentication yes
pubkeyauthentication true
preferredauthentications publickey,password
numberofpasswordprompts 3
batchmode no
compression yes
ciphers aes256-ctr,aes128-ctr
hostkeyalgorithms ssh-ed25519,rsa-sha2-256
pubkeyacceptedalgorithms ssh-ed25519,rsa-sha2-256
kexalgorithms curve25519-sha256,diffie-hellman-group14-sha256
macs hmac-sha2-256,hmac-sha2-512
proxycommand none
proxyjump none
proxyusefdpass no
addressfamily inet
connecttimeout 10
connectionattempts 2
tcpkeepalive yes
serveraliveinterval 30
serveralivecountmax 4
sendenv LANG
sendenv LC_*
setenv WARP_TEST=value
remotecommand none
localcommand none
knownhostscommand none
pkcs11provider none
gssapiauthentication no
hostbasedauthentication no
forwardagent no
forwardx11 no
forwardx11trusted no
verifyhostkeydns no
"#;

#[test]
fn parses_resolved_openssh_config() {
    let config = parse_openssh_config(BASIC_CONFIG).unwrap();
    assert_eq!(config.hostname, "192.0.2.10");
    assert_eq!(config.user, "alice");
    assert_eq!(config.port, 2222);
    assert_eq!(config.identity_files.len(), 2);
    assert_eq!(config.user_known_hosts_files.len(), 2);
    assert!(!config.identities_only);
    assert!(config.use_agent);
    assert!(config.pubkey_authentication);
    assert!(config.password_authentication);
    assert!(config.keyboard_interactive_authentication);
    assert!(!config.batch_mode);
    assert_eq!(config.preferred_authentications, ["publickey", "password"]);
    assert!(config.compression);
    assert_eq!(config.ciphers.as_deref(), Some("aes256-ctr,aes128-ctr"));
    assert_eq!(
        config.pubkey_accepted_algorithms.as_deref(),
        Some("ssh-ed25519,rsa-sha2-256")
    );
    assert_eq!(config.proxy_command, None);
    assert_eq!(config.proxy_jump, None);
    assert_eq!(config.address_family, "inet");
    assert_eq!(config.connect_timeout, Some(Duration::from_secs(10)));
    assert_eq!(config.connection_attempts, 2);
    assert!(config.tcp_keep_alive);
    assert_eq!(config.server_alive_interval, Some(Duration::from_secs(30)));
    assert_eq!(config.server_alive_count_max, 4);
    assert_eq!(config.send_env, ["LANG", "LC_*"]);
    assert_eq!(config.set_env, [("WARP_TEST".into(), "value".into())]);
}

#[test]
fn rejects_an_unrecognized_effective_config_field_before_connecting() {
    let config = format!("{BASIC_CONFIG}\nfuturetransportoption enabled\n");

    assert!(parse_openssh_config(&config).is_err());
}

#[test]
fn rejects_update_host_keys_until_the_rust_transport_can_persist_them() {
    let config = format!("{BASIC_CONFIG}\nupdatehostkeys yes\n");

    assert!(parse_openssh_config(&config).is_err());
}

#[test]
fn rejects_keystroke_timing_obfuscation_until_the_transport_can_preserve_it() {
    let config = format!("{BASIC_CONFIG}\nobscurekeystroketiming yes\n");

    assert!(parse_openssh_config(&config).is_err());
}

#[test]
fn rejects_an_invalid_ip_qos_value_before_connecting() {
    let config = format!("{BASIC_CONFIG}\nipqos invalid\n");

    assert!(parse_openssh_config(&config).is_err());
}

#[test]
fn parses_the_interactive_ip_qos_value() {
    for (value, expected) in [
        ("ef cs0", Some(46 << 2)),
        ("af21 none", Some(18 << 2)),
        ("42", Some(42 << 2)),
        ("none", None),
    ] {
        let config = parse_openssh_config(&format!("{BASIC_CONFIG}\nipqos {value}\n")).unwrap();
        assert_eq!(config.interactive_ip_qos, expected, "{value}");
    }
}

#[test]
fn parses_custom_and_disabled_escape_characters() {
    for (value, expected) in [
        ("~", Some(b'~')),
        ("none", None),
        ("^]", Some(0x1d)),
        ("^?", Some(0x7f)),
    ] {
        assert_eq!(parse_escape_char(value).unwrap(), expected, "{value}");
    }
    assert!(parse_escape_char("long").is_err());
}

#[test]
fn ssh_escape_filter_handles_chunk_boundaries_and_line_start() {
    let mut filter = SshEscapeFilter::new(Some(b'~'));

    assert_eq!(filter.push(b"echo ~.\r").bytes, b"echo ~.\r");
    assert_eq!(filter.push(b"~"), SshEscapeOutput::default());
    assert_eq!(
        filter.push(b"~literal\r"),
        SshEscapeOutput {
            bytes: b"~literal\r".to_vec(),
            ..SshEscapeOutput::default()
        }
    );
    assert_eq!(
        filter.push(b"~."),
        SshEscapeOutput {
            disconnect: true,
            ..SshEscapeOutput::default()
        }
    );
}

#[test]
fn ssh_escape_help_keeps_the_filter_at_line_start() {
    let mut filter = SshEscapeFilter::new(Some(b'~'));

    assert_eq!(
        filter.push(b"~?"),
        SshEscapeOutput {
            show_help: true,
            ..SshEscapeOutput::default()
        }
    );
    assert!(filter.push(b"~.").disconnect);
}

#[test]
fn ssh_escape_filter_flushes_a_pending_literal_at_eof() {
    let mut filter = SshEscapeFilter::new(Some(b'~'));
    assert_eq!(filter.push(b"~"), SshEscapeOutput::default());
    assert_eq!(filter.finish(), b"~");

    let mut disabled = SshEscapeFilter::new(None);
    assert_eq!(disabled.push(b"~.").bytes, b"~.");
}

#[test]
fn accepts_audited_neutral_values_for_modern_openssh_fields() {
    let config = format!(
        "{BASIC_CONFIG}\ncanonicalizehostname false\ncontrolpath none\ncontrolpersist no\nobscurekeystroketiming no\nupdatehostkeys no\nwarnweakcrypto no\nipqos none\n"
    );

    assert!(parse_openssh_config(&config).is_ok());
}

#[test]
fn applies_audited_algorithm_preferences_before_connecting() {
    let config = parse_openssh_config(BASIC_CONFIG).unwrap();
    let session = Session::new().unwrap();

    apply_session_preferences(&session, &config).unwrap();
}

#[test]
fn proxycommand_uses_the_resolved_target_through_the_jump_host() {
    let output = BASIC_CONFIG.replace("proxycommand none", "proxycommand ssh -q -W %h:%p cloud");
    let config = parse_openssh_config(&output).unwrap();

    let spec = proxy_process_spec(&config, Path::new("ignored-ssh"), &[]).unwrap();

    assert_eq!(
        spec,
        Some(ProxyProcessSpec {
            program: PathBuf::from("ssh"),
            args: ["-q", "-W", "192.0.2.10:2222", "cloud"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        })
    );
}

#[test]
fn proxycommand_with_shell_semantics_falls_back_before_connecting() {
    for command in [
        "nc %h %p 2>/tmp/proxy-errors",
        "~/bin/connect %h %p",
        "nc %h %p # proxy comment",
    ] {
        let output = BASIC_CONFIG.replace("proxycommand none", &format!("proxycommand {command}"));
        let config = parse_openssh_config(&output).unwrap();

        assert!(
            proxy_process_spec(&config, Path::new("ssh"), &[]).is_err(),
            "{command} 应在联网前交还原生 OpenSSH"
        );
    }
}

#[test]
fn proxycommand_accepts_an_exec_prefix_without_changing_the_process() {
    let output = BASIC_CONFIG.replace("proxycommand none", "proxycommand exec nc %h %p");
    let config = parse_openssh_config(&output).unwrap();

    let spec = proxy_process_spec(&config, Path::new("ssh"), &[]).unwrap();

    assert_eq!(
        spec,
        Some(ProxyProcessSpec {
            program: PathBuf::from("nc"),
            args: ["192.0.2.10", "2222"]
                .into_iter()
                .map(OsString::from)
                .collect(),
        })
    );
}

#[test]
fn proxyjump_uses_openssh_as_the_jump_transport() {
    let output = BASIC_CONFIG.replace("proxyjump none", "proxyjump bastion");
    let config = parse_openssh_config(&output).unwrap();

    let ssh_args = [OsString::from("-F"), OsString::from("C:\\ssh config")];
    let spec = proxy_process_spec(&config, Path::new("custom-ssh"), &ssh_args).unwrap();

    assert_eq!(
        spec,
        Some(ProxyProcessSpec {
            program: PathBuf::from("custom-ssh"),
            args: [
                "-F",
                "C:\\ssh config",
                "-q",
                "-W",
                "192.0.2.10:2222",
                "bastion"
            ]
            .into_iter()
            .map(OsString::from)
            .collect(),
        })
    );
}

#[test]
fn rejects_gssapi_authentication() {
    let config = BASIC_CONFIG.replace("gssapiauthentication no", "gssapiauthentication yes");
    assert!(parse_openssh_config(&config).is_err());
}

#[test]
fn preserves_a_custom_identity_agent_path() {
    let config = format!("{BASIC_CONFIG}\nidentityagent /tmp/custom-agent.sock\n");
    let config = parse_openssh_config(&config).unwrap();
    assert_eq!(
        config.identity_agent_path,
        Some(PathBuf::from("/tmp/custom-agent.sock"))
    );
}

#[test]
fn preserves_spaces_in_each_identity_file() {
    let config = BASIC_CONFIG.replace(
        "identityfile ~/.ssh/id_ed25519",
        "identityfile /tmp/key with spaces",
    );
    let config = parse_openssh_config(&config).unwrap();

    assert_eq!(
        config.identity_files[0],
        PathBuf::from("/tmp/key with spaces")
    );
}

#[test]
fn rejects_certificate_authentication_before_connecting() {
    let config = format!("{BASIC_CONFIG}\ncertificatefile ~/.ssh/id_ed25519-cert.pub\n");
    assert!(parse_openssh_config(&config).is_err());
}

#[test]
fn rejects_security_options_that_cannot_be_preserved() {
    for (key, config) in [
        (
            "stricthostkeychecking",
            BASIC_CONFIG.replace("stricthostkeychecking yes", "stricthostkeychecking no"),
        ),
        (
            "hashknownhosts",
            format!("{BASIC_CONFIG}\nhashknownhosts yes\n"),
        ),
        (
            "fingerprinthash",
            format!("{BASIC_CONFIG}\nfingerprinthash md5\n"),
        ),
        (
            "requiredrsasize",
            format!("{BASIC_CONFIG}\nrequiredrsasize 3072\n"),
        ),
        ("loglevel", format!("{BASIC_CONFIG}\nloglevel DEBUG\n")),
        (
            "channeltimeout",
            format!("{BASIC_CONFIG}\nchanneltimeout session=5m\n"),
        ),
    ] {
        assert!(
            parse_openssh_config(&config).is_err(),
            "{key} 应在联网前交还原生 OpenSSH"
        );
    }
}

#[test]
fn rejects_unsupported_authentication_order_before_connecting() {
    let config = BASIC_CONFIG.replace(
        "preferredauthentications publickey,password",
        "preferredauthentications publickey,gssapi-with-mic,password",
    );
    assert!(parse_openssh_config(&config).is_err());
}

#[test]
fn send_env_wildcards_match_openssh_patterns() {
    assert!(wildcard_matches(b"LC_*", b"LC_ALL"));
    assert!(wildcard_matches(b"L?NG", b"LANG"));
    assert!(!wildcard_matches(b"LC_*", b"LANG"));
}

#[test]
fn detects_encrypted_pem_private_keys_before_prompting() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("id_rsa");
    std::fs::write(
        &path,
        "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\n-----END RSA PRIVATE KEY-----\n",
    )
    .unwrap();

    assert!(private_key_requires_passphrase(&path));
}

#[test]
fn does_not_prompt_for_unencrypted_pem_private_keys() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("id_rsa");
    std::fs::write(
        &path,
        "-----BEGIN RSA PRIVATE KEY-----\nunencrypted\n-----END RSA PRIVATE KEY-----\n",
    )
    .unwrap();

    assert!(!private_key_requires_passphrase(&path));
}

#[test]
fn capability_comparison_requires_the_complete_value() {
    assert!(capabilities_match("abcdef", "abcdef"));
    assert!(!capabilities_match("abcdeg", "abcdef"));
    assert!(!capabilities_match("abc", "abcdef"));
}

#[test]
fn windows_capability_requires_one_exact_versioned_line() {
    assert!(is_windows_powershell_capability(
        "__WARP_REMOTE_CAPS__v=1;os=windows;shell=powershell\r\n"
    ));
    assert!(!is_windows_powershell_capability(
        "banner\n__WARP_REMOTE_CAPS__v=1;os=windows;shell=powershell\n"
    ));
    assert!(!is_windows_powershell_capability(
        "__WARP_REMOTE_CAPS__v=2;os=windows;shell=powershell\n"
    ));
}

#[test]
fn broker_header_round_trips_multiline_commands() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_header(&mut stream).unwrap()
    });

    let mut stream = TcpStream::connect(endpoint).unwrap();
    write_header(
        &mut stream,
        &BrokerRequest {
            capability: "secret".to_string(),
            operation: BrokerOperation::Exec {
                command: "first\nsecond".to_string(),
            },
        },
    )
    .unwrap();

    let request = server.join().unwrap();
    assert_eq!(request.capability, "secret");
    assert!(matches!(
        request.operation,
        BrokerOperation::Exec { command } if command == "first\nsecond"
    ));
}

#[test]
fn broker_command_streams_a_local_stdin_file() {
    let directory = tempfile::tempdir().unwrap();
    let stdin_file = directory.path().join("remote-server.zip");
    let payload = vec![42_u8; 1024 * 1024];
    std::fs::write(&stdin_file, &payload).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_header(&mut stream).unwrap();
        assert_eq!(request.capability, "secret");
        assert!(matches!(
            request.operation,
            BrokerOperation::Upload {
                remote_path,
                size,
                windows: true,
            }
                if remote_path == "~/.infinishell/remote-server/archive.zip"
                    && size == 1024 * 1024
        ));
        stream.write_all(&[0]).unwrap();

        let mut received = Vec::new();
        stream.read_to_end(&mut received).unwrap();
        write_frame(&mut stream, FRAME_EXIT, &7_i32.to_be_bytes()).unwrap();
        received
    });
    let args = RustSshBrokerCommandArgs {
        endpoint: endpoint.to_string(),
        command: None,
        upload_path: Some("~/.infinishell/remote-server/archive.zip".to_string()),
        upload_windows: true,
        stdin_file: Some(stdin_file),
    };

    assert_eq!(
        run_broker_command_with_capability(&args, "secret".to_string()).unwrap(),
        7
    );
    assert_eq!(server.join().unwrap(), payload);
}

#[test]
fn broker_exec_does_not_wait_for_inherited_stdin_after_remote_exit() {
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let mut input_thread = Some(thread::spawn(move || {
        release_rx.recv().unwrap();
        Ok(0)
    }));

    finish_broker_input(&mut input_thread, false).unwrap();
    assert!(input_thread.is_some());

    release_tx.send(()).unwrap();
    input_thread.take().unwrap().join().unwrap().unwrap();
}

/// 真实验证当前 Rust SSH session 上的 SCP broker 上传。
///
/// 运行前设置：
/// `WARP_WINDOWS_SSH_E2E_HOST=<ssh-host>`
/// `WARP_WINDOWS_SSH_E2E_LOCAL_ARCHIVE=<windows-zip>`
#[test]
#[ignore]
fn windows_broker_scp_uploads_the_complete_archive() {
    let host = std::env::var("WARP_WINDOWS_SSH_E2E_HOST").unwrap();
    let local_archive = PathBuf::from(std::env::var("WARP_WINDOWS_SSH_E2E_LOCAL_ARCHIVE").unwrap());
    let expected_size = std::fs::metadata(&local_archive).unwrap().len();
    let ssh_args = [
        "-o",
        "UpdateHostKeys=no",
        "-o",
        "WarnWeakCrypto=no",
        "-o",
        "ObscureKeystrokeTiming=no",
        &host,
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    let config = resolve_openssh_config(Path::new("ssh"), &ssh_args).unwrap();
    let proxy = proxy_process_spec(&config, Path::new("ssh"), &ssh_args).unwrap();
    let mut prompted = false;
    let session = connect_session(&config, proxy, &mut prompted).unwrap();
    authenticate_session(&session, &config, &mut prompted).unwrap();
    session.set_blocking(false);

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let session_gate = Arc::new(Mutex::new(()));
    let capability = "test-capability".to_string();
    let broker_session = session.clone();
    let broker_gate = session_gate.clone();
    let broker_capability = capability.clone();
    let broker = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_broker_connection(
            stream,
            broker_session,
            &broker_gate,
            &broker_capability,
            &[],
        )
    });
    let remote_relative = format!(
        ".infinishell/remote-server/broker-upload-{}.zip",
        uuid::Uuid::new_v4()
    );
    let args = RustSshBrokerCommandArgs {
        endpoint: endpoint.to_string(),
        command: None,
        upload_path: Some(format!("~/{remote_relative}")),
        upload_windows: true,
        stdin_file: Some(local_archive),
    };

    let upload = run_broker_command_with_capability(&args, capability);
    let broker_result = broker.join().unwrap();
    assert!(broker_result.is_ok(), "SCP broker 失败: {broker_result:?}");
    let exit_code = upload.unwrap();
    assert_eq!(exit_code, 0);

    session.set_blocking(true);
    let sftp = session.sftp().unwrap();
    let remote_path = sftp.realpath(Path::new(".")).unwrap().join(remote_relative);
    assert_eq!(sftp.stat(&remote_path).unwrap().size, Some(expected_size));
    sftp.unlink(&remote_path).unwrap();
}

#[test]
fn recursive_rust_broker_hook_carries_scope_and_hop() {
    let payload = ssh_hook_payload(
        "127.0.0.1:49152".parse().unwrap(),
        &"ab".repeat(32),
        RemoteShell::PowerShell,
        7,
        8,
        "remote",
        3,
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();

    assert_eq!(value["value"]["session_id"], 7);
    assert_eq!(value["value"]["remote_session_id"], 8);
    assert_eq!(value["value"]["parent_session_id"], 7);
    assert_eq!(value["value"]["control_scope"], "remote");
    assert_eq!(value["value"]["hop_depth"], 3);
    assert_eq!(value["value"]["transport"]["type"], "rust_broker");
    assert!(
        ssh_hook_payload(
            "127.0.0.1:49152".parse().unwrap(),
            &"ab".repeat(32),
            RemoteShell::PowerShell,
            7,
            8,
            "remote",
            9,
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn proxy_command_socket_carries_the_ssh_byte_stream_in_both_directions() {
    let spec = ProxyProcessSpec {
        program: PathBuf::from("/bin/cat"),
        args: Vec::new(),
    };
    let mut proxy = connect_proxy_command(&spec).unwrap();
    proxy
        .socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();

    proxy.socket.write_all(b"ssh bytes").unwrap();
    let mut echoed = [0_u8; 9];
    proxy.socket.read_exact(&mut echoed).unwrap();

    assert_eq!(&echoed, b"ssh bytes");
}

#[cfg(unix)]
#[test]
fn native_fallback_preserves_a_remote_status_that_was_previously_reserved() {
    let args = RustSshSessionArgs {
        session_id: 1,
        remote_session_id: 2,
        control_scope: "local".to_string(),
        hop_depth: 1,
        ssh_executable: "/bin/sh".into(),
        posix_command: String::new(),
        windows_command: String::new(),
        ssh_args: ["-c", "exit 125"].into_iter().map(OsString::from).collect(),
    };

    assert_eq!(run_native_ssh(&args).unwrap(), 125);
}

#[cfg(unix)]
#[test]
fn unknown_effective_config_falls_back_with_the_original_arguments_before_connecting() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("fake ssh");
    let recorded_arguments = directory.path().join("fallback arguments");
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"-G\" ]; then\n  printf '%s\\n' 'host test' 'hostname 192.0.2.10' 'user alice' 'port 22' 'futuretransportoption enabled'\n  exit 0\nfi\nprintf '%s\\n' \"$@\" > '{}'\nexit 73\n",
            recorded_arguments.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ssh_args = ["-F", "config with spaces", "alice@test"]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    let args = RustSshSessionArgs {
        session_id: 1,
        remote_session_id: 2,
        control_scope: "local".to_string(),
        hop_depth: 1,
        ssh_executable: executable,
        posix_command: String::new(),
        windows_command: String::new(),
        ssh_args,
    };

    assert_eq!(run_session_worker(&args).unwrap(), 73);
    assert_eq!(
        std::fs::read_to_string(recorded_arguments).unwrap(),
        "-F\nconfig with spaces\nalice@test\n"
    );
}
