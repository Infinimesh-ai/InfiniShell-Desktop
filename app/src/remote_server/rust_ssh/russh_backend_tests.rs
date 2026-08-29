use std::io;
use std::path::PathBuf;
use std::time::Duration;

use russh::keys::{Algorithm, HashAlg};

use super::*;

fn config() -> OpenSshConfig {
    OpenSshConfig {
        hostname: "example.test".to_string(),
        user: "user".to_string(),
        port: 22,
        strict_host_key_checking: "yes".to_string(),
        host_key_alias: None,
        user_known_hosts_files: Vec::new(),
        global_known_hosts_files: Vec::new(),
        identity_files: Vec::new(),
        identities_only: false,
        use_agent: false,
        identity_agent_path: None,
        pubkey_authentication: true,
        keyboard_interactive_authentication: true,
        password_authentication: true,
        batch_mode: false,
        number_of_password_prompts: 3,
        preferred_authentications: Vec::new(),
        kex_algorithms: None,
        host_key_algorithms: None,
        pubkey_accepted_algorithms: None,
        ciphers: None,
        macs: None,
        compression: false,
        proxy_command: None,
        proxy_jump: None,
        address_family: "any".to_string(),
        interactive_ip_qos: None,
        connect_timeout: Some(Duration::from_secs(5)),
        connection_attempts: 1,
        tcp_keep_alive: true,
        server_alive_interval: Some(Duration::from_secs(10)),
        server_alive_count_max: 4,
        escape_char: Some(b'~'),
        send_env: Vec::new(),
        set_env: Vec::new(),
    }
}

#[test]
fn key_algorithm_mapping_covers_regular_and_security_keys() {
    assert_eq!(parse_key_algorithm("ssh-ed25519"), Some(Algorithm::Ed25519));
    assert_eq!(
        parse_key_algorithm("rsa-sha2-512"),
        Some(Algorithm::Rsa {
            hash: Some(HashAlg::Sha512)
        })
    );
    assert_eq!(
        parse_key_algorithm("sk-ssh-ed25519@openssh.com"),
        Some(Algorithm::SkEd25519)
    );
    assert_eq!(parse_key_algorithm("ssh-dss"), None);
}

#[test]
fn russh_preferences_preserve_supported_openssh_order() {
    let mut config = config();
    config.kex_algorithms =
        Some("sntrup761x25519-sha512,mlkem768x25519-sha256,curve25519-sha256".to_string());
    config.ciphers = Some("aes128-ctr,chacha20-poly1305@openssh.com".to_string());
    config.macs = Some("umac-64-etm@openssh.com,hmac-sha2-256".to_string());

    let built = build_russh_config(&config).unwrap();

    assert_eq!(built.preferred.kex[0].as_ref(), "mlkem768x25519-sha256");
    assert_eq!(built.preferred.kex[1].as_ref(), "curve25519-sha256");
    assert_eq!(built.preferred.cipher[0].as_ref(), "aes128-ctr");
    assert_eq!(
        built.preferred.cipher[1].as_ref(),
        "chacha20-poly1305@openssh.com"
    );
    assert_eq!(built.preferred.mac.len(), 1);
    assert_eq!(built.preferred.mac[0].as_ref(), "hmac-sha2-256");
    assert_eq!(built.keepalive_interval, Some(Duration::from_secs(10)));
    assert_eq!(built.keepalive_max, 4);
    assert_eq!(built.preferred.compression.len(), 1);
    assert_eq!(built.preferred.compression[0].as_ref(), "none");
}

#[test]
fn russh_preferences_reject_algorithm_lists_without_an_intersection() {
    let mut config = config();
    config.kex_algorithms = Some("sntrup761x25519-sha512".to_string());

    assert!(build_russh_config(&config).is_err());
}

#[test]
fn known_hosts_markers_force_the_existing_fallback_before_connect() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known_hosts");
    std::fs::write(
        &path,
        "@cert-authority *.example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA==\n",
    )
    .unwrap();
    let mut config = config();
    config.user_known_hosts_files = vec![path];

    assert!(ensure_known_hosts_supported(&config).is_err());
}

#[test]
fn ordinary_known_hosts_files_remain_on_the_russh_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known_hosts");
    std::fs::write(&path, "# empty test file\n").unwrap();
    let mut config = config();
    config.user_known_hosts_files = vec![PathBuf::from(path)];

    ensure_known_hosts_supported(&config).unwrap();
}

#[test]
fn public_key_authentication_honors_the_resolved_algorithm_allowlist() {
    let mut config = config();
    config.pubkey_accepted_algorithms = Some("ssh-ed25519,rsa-sha2-256".to_string());

    assert!(pubkey_algorithm_allowed(
        &config,
        Algorithm::Ed25519,
        None,
        false
    ));
    assert!(pubkey_algorithm_allowed(
        &config,
        Algorithm::Rsa { hash: None },
        Some(HashAlg::Sha256),
        false
    ));
    assert!(!pubkey_algorithm_allowed(
        &config,
        Algorithm::Rsa { hash: None },
        Some(HashAlg::Sha512),
        false
    ));
    assert!(!pubkey_algorithm_allowed(
        &config,
        Algorithm::Ed25519,
        None,
        true
    ));
}

#[test]
fn compression_yes_prefers_delayed_openssh_compression() {
    let mut config = config();
    config.compression = true;

    let built = build_russh_config(&config).unwrap();

    assert_eq!(built.preferred.compression[0].as_ref(), "zlib@openssh.com");
    assert_eq!(built.preferred.compression[1].as_ref(), "zlib");
    assert_eq!(built.preferred.compression[2].as_ref(), "none");
}

struct InterruptedOnce<R> {
    reader: R,
    interrupted: bool,
}

impl<R: io::Read> io::Read for InterruptedOnce<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        self.reader.read(buffer)
    }
}

#[test]
fn stdin_reader_retries_interrupted_reads_and_reports_eof() {
    let mut reader = InterruptedOnce {
        reader: io::Cursor::new(b"input"),
        interrupted: false,
    };
    let mut buffer = [0_u8; 8];

    assert_eq!(
        read_stdin_chunk(&mut reader, &mut buffer).unwrap(),
        Some(b"input".to_vec())
    );
    assert_eq!(read_stdin_chunk(&mut reader, &mut buffer).unwrap(), None);
}

struct FailingReader;

impl io::Read for FailingReader {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

#[test]
fn stdin_reader_preserves_non_interrupted_errors() {
    let error = read_stdin_chunk(&mut FailingReader, &mut [0_u8; 8]).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}
