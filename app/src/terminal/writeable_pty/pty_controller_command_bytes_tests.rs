use super::*;

#[test]
#[cfg(windows)]
fn bracketed_paste_command_execution_normalizes_crlf_to_lf_for_posix_shells_on_windows() {
    let command = "curl 'https://google.com' \\\r\n  -H 'accept: application/json'";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, true);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_START);
    expected.extend_from_slice(b"curl 'https://google.com' \\\n  -H 'accept: application/json'");
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_END);
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&b'\r'));
}

#[test]
#[cfg(not(windows))]
fn bracketed_paste_command_execution_preserves_crlf_for_posix_shells_off_windows() {
    let command = "curl 'https://google.com' \\\r\n  -H 'accept: application/json'";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, true);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_START);
    expected.extend_from_slice(b"curl 'https://google.com' \\\r\n  -H 'accept: application/json'");
    expected.extend_from_slice(escape_sequences::BRACKETED_PASTE_END);
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(bytes.contains(&b'\r'));
}

#[test]
fn unbracketed_paste_command_execution_preserves_lf_for_posix_shells() {
    let command = "printf 'hello'\nprintf 'world'";

    let bytes = bytes_to_execute_command(command, ShellType::Bash, false);

    let mut expected = ShellType::Bash.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(b"printf 'hello'\nprintf 'world'");
    expected.extend_from_slice(ShellType::Bash.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&b'\r'));
}

#[test]
fn powershell_command_execution_normalizes_linefeeds_to_carriage_returns() {
    let command = "Write-Output 'hello'\r\nWrite-Output 'world'\nWrite-Output 'again'";

    let bytes = bytes_to_execute_command(command, ShellType::PowerShell, false);

    let mut expected = ShellType::PowerShell.kill_buffer_bytes().to_vec();
    expected.extend_from_slice(b"Write-Output 'hello'\rWrite-Output 'world'\rWrite-Output 'again'");
    expected.extend_from_slice(ShellType::PowerShell.execute_command_bytes());

    assert_eq!(bytes, expected);
    assert!(!bytes.contains(&b'\n'));
}

#[test]
fn powershell_ssh_bootstrap_normalizes_linefeeds_to_carriage_returns() {
    let bootstrap = b"function Test-Warp {\r\n  Write-Output 'ready'\n}\n".to_vec();

    let bytes = normalize_powershell_pty_line_endings(bootstrap);

    assert_eq!(bytes, b"function Test-Warp {\r  Write-Output 'ready'\r}\r");
    assert!(!bytes.contains(&b'\n'));
}
