use super::*;

fn output(exit_code: Option<i32>) -> RoutedCommandOutput {
    RoutedCommandOutput {
        stdout: Vec::new(),
        stderr: "binary check failed".to_string(),
        exit_code,
    }
}

#[test]
fn windows_exit_one_means_binary_is_missing() {
    assert!(!binary_check_result(&RemoteOs::Windows, output(Some(1))).unwrap());
}

#[test]
fn non_windows_exit_one_remains_an_error() {
    assert!(binary_check_result(&RemoteOs::Linux, output(Some(1))).is_err());
    assert!(binary_check_result(&RemoteOs::MacOs, output(Some(1))).is_err());
}

#[test]
fn standard_binary_check_exit_codes_keep_their_meaning() {
    assert!(binary_check_result(&RemoteOs::Windows, output(Some(0))).unwrap());
    assert!(!binary_check_result(&RemoteOs::Linux, output(Some(126))).unwrap());
    assert!(!binary_check_result(&RemoteOs::Linux, output(Some(127))).unwrap());
    assert!(binary_check_result(&RemoteOs::Linux, output(None)).is_err());
}
