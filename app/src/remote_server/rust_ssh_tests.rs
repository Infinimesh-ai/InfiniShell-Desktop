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
            command: "first\nsecond".to_string(),
        },
    )
    .unwrap();

    let request = server.join().unwrap();
    assert_eq!(request.capability, "secret");
    assert_eq!(request.command, "first\nsecond");
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
        .set_read_timeout(Some(Duration::from_secs(1)))
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
        ssh_executable: "/bin/sh".into(),
        posix_command: String::new(),
        windows_command: String::new(),
        ssh_args: ["-c", "exit 125"].into_iter().map(OsString::from).collect(),
    };

    assert_eq!(run_native_ssh(&args).unwrap(), 125);
}
