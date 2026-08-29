use super::{SshConfigCandidate, parse_ssh_config};

#[test]
fn merges_managed_enhanced_ssh_block_with_original_host_block() {
    let input = "\
# >>> InfiniShell enhanced SSH for devbox
Host devbox
    UpdateHostKeys no
    ObscureKeystrokeTiming no
Host *
# <<< InfiniShell enhanced SSH for devbox

Host devbox
    HostName 192.168.20.117
    User dev
    Port 2222
";

    let candidates = parse_ssh_config(input);

    assert_eq!(
        candidates,
        vec![SshConfigCandidate {
            alias: "devbox".into(),
            hostname: Some("192.168.20.117".into()),
            user: Some("dev".into()),
            port: Some(2222),
            identity_file: None,
        }]
    );
}
