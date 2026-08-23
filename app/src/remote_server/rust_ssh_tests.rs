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
fn reports_all_default_security_options_needed_for_enhanced_ssh_opt_in() {
    let config = format!(
        "{BASIC_CONFIG}\nupdatehostkeys true\nobscurekeystroketiming yes\nsecuritykeyprovider internal\n"
    );

    let error = parse_openssh_config(&config).unwrap_err();
    let opt_in = error
        .downcast_ref::<EnhancedSshOptInRequired>()
        .expect("应返回可由用户确认的一键配置请求");

    assert_eq!(opt_in.host, "test");
    assert_eq!(opt_in.options, ["updatehostkeys", "obscurekeystroketiming"]);
    let message = error.to_string();
    assert!(message.contains("updatehostkeys, obscurekeystroketiming"));
    assert!(message.contains("UpdateHostKeys=no and ObscureKeystrokeTiming=no"));
}

#[test]
fn accepts_windows_internal_security_key_provider_without_a_local_sk_identity() {
    let config = format!(
        "{BASIC_CONFIG}\nsecuritykeyprovider internal\nupdatehostkeys no\nobscurekeystroketiming no\n"
    );

    assert!(parse_openssh_config(&config).is_ok());
}

#[test]
fn rejects_windows_internal_security_key_provider_with_a_local_sk_identity() {
    let directory = tempfile::tempdir().unwrap();
    let identity = directory.path().join("hardware-key");
    let public_identity = directory.path().join("hardware-key.pub");
    std::fs::write(
        public_identity,
        "sk-ssh-ed25519@openssh.com AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29t test\n",
    )
    .unwrap();
    let config = format!(
        "{BASIC_CONFIG}\nidentityfile {}\nsecuritykeyprovider internal\nupdatehostkeys no\nobscurekeystroketiming no\n",
        identity.display()
    );

    let error = parse_openssh_config(&config).unwrap_err().to_string();

    assert!(error.contains("securitykeyprovider"));
}

#[test]
fn one_click_opt_in_prepends_an_exact_host_block_and_preserves_crlf_and_bom() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join(".ssh").join("config");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(
        &config_path,
        "\u{feff}Host *\r\n    ServerAliveInterval 30\r\n",
    )
    .unwrap();

    upsert_enhanced_ssh_host_config(&config_path, "example-host").unwrap();
    upsert_enhanced_ssh_host_config(&config_path, "example-host").unwrap();

    let updated = std::fs::read_to_string(config_path).unwrap();
    assert!(updated.starts_with(
        "\u{feff}# >>> InfiniShell enhanced SSH for example-host\r\nHost example-host\r\n    UpdateHostKeys no\r\n    ObscureKeystrokeTiming no\r\nHost *\r\n# <<< InfiniShell enhanced SSH for example-host\r\n\r\n"
    ));
    assert_eq!(
        updated
            .matches("# >>> InfiniShell enhanced SSH for example-host")
            .count(),
        1
    );
    assert!(updated.ends_with("Host *\r\n    ServerAliveInterval 30\r\n"));
    assert!(!updated.contains("\nHost *\n"));
}

#[test]
fn one_click_opt_in_rejects_a_host_pattern_without_touching_the_config() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config");
    std::fs::write(&config_path, "Host existing\n").unwrap();

    assert!(upsert_enhanced_ssh_host_config(&config_path, "*.example.com").is_err());

    assert_eq!(
        std::fs::read_to_string(config_path).unwrap(),
        "Host existing\n"
    );
}

#[cfg(windows)]
#[test]
fn one_click_opt_in_request_updates_the_config_and_signals_the_worker() {
    let token = format!("{:032x}", rand::random::<u128>());
    let event_name = HSTRING::from(enhanced_ssh_opt_in_event_name(&token));
    let event =
        WindowsEventHandle(unsafe { CreateEventW(None, true, false, &event_name) }.unwrap());
    let request_path = enhanced_ssh_opt_in_request_path(&token);
    let result_path = enhanced_ssh_opt_in_result_path(&token);
    std::fs::create_dir_all(request_path.parent().unwrap()).unwrap();
    std::fs::write(
        &request_path,
        serde_json::to_vec(&EnhancedSshOptInRequest {
            host: "one-click-test".to_string(),
            created_at_unix_seconds: current_unix_seconds().unwrap(),
        })
        .unwrap(),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join(".ssh").join("config");

    let host = approve_enhanced_ssh_opt_in_at_path(&token, &config_path).unwrap();

    assert_eq!(host, "one-click-test");
    assert!(
        std::fs::read_to_string(config_path)
            .unwrap()
            .contains("Host one-click-test\n    UpdateHostKeys no")
    );
    assert_eq!(
        unsafe { WaitForMultipleObjects(&[event.0], false, 0) },
        WAIT_OBJECT_0
    );
    let result: EnhancedSshOptInResult =
        serde_json::from_slice(&std::fs::read(&result_path).unwrap()).unwrap();
    assert!(result.applied);

    let _ = std::fs::remove_file(request_path);
    let _ = std::fs::remove_file(result_path);
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
        "{BASIC_CONFIG}\ncanonicalizehostname false\ncontrolpath none\ncontrolpersist no\nobscurekeystroketiming no\nupdatehostkeys no\nwarnweakcrypto no\nipqos none\nsecuritykeyprovider internal\n"
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
        endpoint: Some(endpoint.to_string()),
        control_path: None,
        command: None,
        upload_path: Some("~/.infinishell/remote-server/archive.zip".to_string()),
        upload_windows: true,
        stdin_file: Some(stdin_file),
        stdin_size: None,
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

#[test]
fn broker_stream_upload_uses_the_declared_stdin_size() {
    let args = RustSshBrokerCommandArgs {
        endpoint: Some("127.0.0.1:1".to_string()),
        control_path: None,
        command: None,
        upload_path: Some("~/.infinishell/remote-server/archive.zip".to_string()),
        upload_windows: true,
        stdin_file: None,
        stdin_size: Some(8192),
    };

    let operation = broker_operation(&args, None).unwrap();

    assert!(matches!(
        operation,
        BrokerOperation::Upload {
            remote_path,
            size: 8192,
            windows: true,
        } if remote_path == "~/.infinishell/remote-server/archive.zip"
    ));
}

#[test]
fn broker_stream_upload_rejects_a_missing_or_empty_size() {
    let mut args = RustSshBrokerCommandArgs {
        endpoint: Some("127.0.0.1:1".to_string()),
        control_path: None,
        command: None,
        upload_path: Some("~/.infinishell/remote-server/archive.zip".to_string()),
        upload_windows: true,
        stdin_file: None,
        stdin_size: None,
    };

    assert!(broker_operation(&args, None).is_err());
    args.stdin_size = Some(0);
    assert!(broker_operation(&args, None).is_err());
}

#[test]
fn control_master_upload_stages_exactly_the_declared_input() {
    let payload = b"recursive windows archive";

    let archive = stage_control_upload(&payload[..], payload.len() as u64).unwrap();

    assert_eq!(std::fs::read(archive.path()).unwrap(), payload);
}

#[test]
fn control_master_upload_rejects_truncated_input() {
    let payload = b"short";

    assert!(stage_control_upload(&payload[..], 10).is_err());
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
        endpoint: Some(endpoint.to_string()),
        control_path: None,
        command: None,
        upload_path: Some(format!("~/{remote_relative}")),
        upload_windows: true,
        stdin_file: Some(local_archive),
        stdin_size: None,
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
        commands_base64: false,
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
        commands_base64: false,
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

#[test]
fn base64_session_commands_are_decoded_before_use() {
    let args = RustSshSessionArgs {
        session_id: 1,
        remote_session_id: 2,
        control_scope: "local".to_string(),
        hop_depth: 1,
        commands_base64: true,
        ssh_executable: "ssh.exe".into(),
        posix_command: "cHJpbnRmICclc1xuJyAiJDEiIHwgY29tbWFuZCAtcCB4eGQgLXAgLXI=".into(),
        windows_command: "cG93ZXJzaGVsbC5leGUgLU5vTG9nbyAtTm9FeGl0".into(),
        ssh_args: Vec::new(),
    };

    let resolved = resolve_session_worker_args(&args).unwrap();

    assert!(!resolved.commands_base64);
    assert_eq!(
        resolved.posix_command,
        "printf '%s\\n' \"$1\" | command -p xxd -p -r"
    );
    assert_eq!(resolved.windows_command, "powershell.exe -NoLogo -NoExit");
}

#[test]
fn raw_session_commands_remain_supported_for_running_shells_during_update() {
    let args = RustSshSessionArgs {
        session_id: 1,
        remote_session_id: 2,
        control_scope: "local".to_string(),
        hop_depth: 1,
        commands_base64: false,
        ssh_executable: "ssh.exe".into(),
        posix_command: "printf '%s\\n' \"$1\" | command -p xxd -p -r".into(),
        windows_command: "powershell.exe -NoLogo -NoExit".into(),
        ssh_args: Vec::new(),
    };

    let resolved = resolve_session_worker_args(&args).unwrap();

    assert_eq!(resolved.posix_command, args.posix_command);
    assert_eq!(resolved.windows_command, args.windows_command);
}

#[test]
fn invalid_base64_session_command_is_rejected_before_connecting() {
    let args = RustSshSessionArgs {
        session_id: 1,
        remote_session_id: 2,
        control_scope: "local".to_string(),
        hop_depth: 1,
        commands_base64: true,
        ssh_executable: "ssh.exe".into(),
        posix_command: "not base64".into(),
        windows_command: "cG93ZXJzaGVsbA==".into(),
        ssh_args: Vec::new(),
    };

    let error = resolve_session_worker_args(&args).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("invalid base64 POSIX bootstrap command")
    );
}
