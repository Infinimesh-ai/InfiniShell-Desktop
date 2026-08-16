use super::*;

#[test]
fn enabled_support_uses_remote_server_when_feature_and_transport_are_available() {
    assert!(SshRemoteServerSupport::Enabled.should_use_remote_server(true, true,));
}

#[test]
fn disabled_support_skips_remote_server_when_transport_is_available() {
    assert!(!SshRemoteServerSupport::Disabled.should_use_remote_server(true, true,));
}

#[test]
fn enabled_support_skips_remote_server_when_feature_is_disabled() {
    assert!(!SshRemoteServerSupport::Enabled.should_use_remote_server(false, true,));
}

#[test]
fn enabled_support_skips_remote_server_without_transport() {
    assert!(!SshRemoteServerSupport::Enabled.should_use_remote_server(true, false,));
}
