use super::*;

const ATTEMPT: &[u8] = b"10000000-0000-4000-8000-000000000001";
const NONCE: &[u8] = b"20000000-0000-4000-8000-000000000001";

fn fields() -> Vec<&'static [u8]> {
    vec![b"9278", b"t", b"1", b"17", ATTEMPT, NONCE]
}

#[test]
fn challenge_is_bounded_typed_and_redacted() {
    let challenge = parse_terminal_binding_challenge(&fields()).unwrap();
    assert_eq!(challenge.session_id, SessionId::from(17));
    assert_eq!(challenge.attempt.to_string().as_bytes(), ATTEMPT);
    assert_eq!(challenge.nonce.to_string().as_bytes(), NONCE);
    let debug = format!("{challenge:?}");
    assert!(!debug.contains(str::from_utf8(NONCE).unwrap()));
    assert_eq!(debug, "TerminalBindingChallenge(<redacted>)");
}

#[test]
fn challenge_rejects_extra_missing_empty_or_noncanonical_fields() {
    for (index, invalid) in [
        (0, b"777".as_slice()),
        (1, b"tt"),
        (2, b"2"),
        (2, b""),
        (3, b""),
        (3, b"0"),
        (3, b"017"),
        (3, b"+17"),
        (3, b"-1"),
        (3, b"18446744073709551616"),
        (4, b""),
        (4, b"null"),
        (4, b"00000000-0000-0000-0000-000000000000"),
        (4, b"10000000000040008000000000000001"),
        (5, b"null"),
        (5, b"00000000-0000-0000-0000-000000000000"),
        (5, b"\xff"),
    ] {
        let mut params = fields();
        params[index] = invalid;
        assert!(parse_terminal_binding_challenge(&params).is_none());
    }
    let mut params = fields();
    params.push(NONCE);
    assert!(parse_terminal_binding_challenge(&params).is_none());
    params.pop();
    params.pop();
    assert!(parse_terminal_binding_challenge(&params).is_none());
    let oversized = vec![b'a'; 257];
    let mut params = fields();
    params[5] = &oversized;
    assert!(parse_terminal_binding_challenge(&params).is_none());
}
