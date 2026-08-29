use super::{append_password_submit_byte, bytes_look_like_password_prompt};

fn matches(input: &str) -> bool {
    bytes_look_like_password_prompt(input.as_bytes())
}

#[test]
fn matches_typical_password_prompt() {
    assert!(matches("user@host's password: "));
    assert!(matches("Password:"));
    assert!(matches("password: \r\n"));
}

#[test]
fn password_submission_uses_carriage_return() {
    let mut bytes = b"secret".to_vec();

    append_password_submit_byte(&mut bytes);

    assert_eq!(bytes, b"secret\r");
}

#[test]
fn matches_sudo_password_prompt() {
    assert!(matches("[sudo] password for alice: "));
}

#[test]
fn matches_passphrase_prompt() {
    assert!(matches("Enter passphrase for key '/home/u/.ssh/id_rsa': "));
}

#[test]
fn does_not_match_motd_with_password_word() {
    assert!(!matches("Welcome! Please change your password soon.\n# "));
    assert!(!matches(
        "Last login: Mon Jan 1 password rotated yesterday\n"
    ));
}

#[test]
fn does_not_match_no_colon() {
    assert!(!matches("password\n"));
    assert!(!matches("Enter password please\n"));
}
