use super::{
    connection_label_from_session_hosts, connection_label_from_ssh_host,
    connection_label_from_user_and_host, powershell_bootstrap_paths,
};

#[test]
fn powershell_bootstrap_path_is_relative_to_remote_home() {
    assert_eq!(
        powershell_bootstrap_paths(Some("v0.1.2+dev/test")),
        Some((
            "~/.infinishell/remote-server/pwsh-bootstrap-v0.1.2_dev_test.ps1".to_string(),
            ".infinishell/remote-server/pwsh-bootstrap-v0.1.2_dev_test.ps1".to_string(),
        ))
    );
}

#[test]
fn connection_label_prefers_ssh_host_over_reported_hostname() {
    assert_eq!(
        connection_label_from_session_hosts(
            "moira",
            "remote-reported-hostname",
            Some("ssh-user@devbox.namespace"),
        ),
        "moira@devbox.namespace"
    );
    assert_eq!(
        connection_label_from_session_hosts("moira", "remote-reported-hostname", None),
        "moira@remote-reported-hostname"
    );
}

#[test]
fn connection_label_from_ssh_host_strips_user_prefix() {
    assert_eq!(
        connection_label_from_ssh_host("moira@moira.devbox.namespace"),
        "moira.devbox.namespace"
    );
    assert_eq!(
        connection_label_from_ssh_host("moira.devbox.namespace"),
        "moira.devbox.namespace"
    );
}

#[test]
fn connection_label_from_user_and_host_matches_udi_format() {
    assert_eq!(
        connection_label_from_user_and_host("kevinyang", Some("ssh-testing")),
        "kevinyang@ssh-testing"
    );
    assert_eq!(
        connection_label_from_user_and_host("kevinyang", None),
        "kevinyang"
    );
    assert_eq!(
        connection_label_from_user_and_host("", Some("ssh-testing")),
        "ssh-testing"
    );
    assert_eq!(connection_label_from_user_and_host("", None), "Remote host");
}
