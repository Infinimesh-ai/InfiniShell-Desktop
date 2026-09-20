use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;

use super::fixture_environment_entry_allowed;

#[test]
fn known_driver_environment_keys_remain_allowed() {
    assert!(fixture_environment_entry_allowed(
        OsStr::new("HOME"),
        OsStr::new("synthetic-home"),
        501,
    ));
    assert!(fixture_environment_entry_allowed(
        OsStr::new("LANG"),
        OsStr::new("en_US.UTF-8"),
        501,
    ));
}

#[test]
fn default_core_foundation_encoding_is_allowed_only_on_macos() {
    assert_eq!(
        fixture_environment_entry_allowed(
            OsStr::new("__CF_USER_TEXT_ENCODING"),
            OsStr::new("0x1F5:0x0:0x0"),
            501,
        ),
        cfg!(target_os = "macos")
    );
}

#[test]
fn encoding_for_another_user_is_rejected() {
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("__CF_USER_TEXT_ENCODING"),
        OsStr::new("0x1F6:0x0:0x0"),
        501,
    ));
}

#[test]
fn nondefault_or_extended_encoding_is_rejected() {
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("__CF_USER_TEXT_ENCODING"),
        OsStr::new("0x1F5:0x1:0x0"),
        501,
    ));
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("__CF_USER_TEXT_ENCODING"),
        OsStr::new("0x1F5:0x0:0x1"),
        501,
    ));
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("__CF_USER_TEXT_ENCODING"),
        OsStr::new("0x1F5:0x0:0x0 extra"),
        501,
    ));
}

#[test]
fn credentials_and_proxy_keys_are_rejected() {
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("OPENAI_API_KEY"),
        OsStr::new("synthetic-value"),
        501,
    ));
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("ANTHROPIC_API_KEY"),
        OsStr::new("synthetic-value"),
        501,
    ));
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("XAI_API_KEY"),
        OsStr::new("synthetic-value"),
        501,
    ));
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("HTTPS_PROXY"),
        OsStr::new("synthetic-value"),
        501,
    ));
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("ALL_PROXY"),
        OsStr::new("synthetic-value"),
        501,
    ));
}

#[test]
fn injection_and_other_core_foundation_keys_are_rejected() {
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("DYLD_INSERT_LIBRARIES"),
        OsStr::new("synthetic-library"),
        501,
    ));
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("CFFIXED_USER_HOME"),
        OsStr::new("synthetic-home"),
        501,
    ));
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("__CF_OTHER_VARIABLE"),
        OsStr::new("synthetic-value"),
        501,
    ));
}

#[test]
fn non_utf8_key_or_encoding_value_is_rejected() {
    assert!(!fixture_environment_entry_allowed(
        OsStr::from_bytes(b"INVALID_\xff"),
        OsStr::new("synthetic-value"),
        501,
    ));
    assert!(!fixture_environment_entry_allowed(
        OsStr::new("__CF_USER_TEXT_ENCODING"),
        OsStr::from_bytes(b"0x1F5:0x0:0x0\xff"),
        501,
    ));
}
