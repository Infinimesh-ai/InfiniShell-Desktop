#[cfg(unix)]
use std::fs;

#[cfg(unix)]
use command::blocking::Command;

use super::*;

#[test]
fn scan_parser_only_accepts_managed_version_files() {
    let output = "startup noise\n\
I\tinfinishell-v0.2026.08.19\t120\t0\n\
I\tinfinishell-v0.2026.08.18\t240\t1\n\
I\t../infinishell-v0.1\t1\t0\n\
I\tinfinishell-vbad\t1\t0\n";

    let versions = parse_scan_output(output, "infinishell-v0.2026.08.19");

    assert_eq!(
        versions,
        vec![
            InstalledVersion {
                file_name: "infinishell-v0.2026.08.19".to_string(),
                size_bytes: 120,
                is_current: true,
                is_running: false,
            },
            InstalledVersion {
                file_name: "infinishell-v0.2026.08.18".to_string(),
                size_bytes: 240,
                is_current: false,
                is_running: true,
            },
        ]
    );
}

#[test]
fn cleanup_script_rejects_paths_and_shell_syntax() {
    let invalid_path = "infinishell-v0.1/../../secret".to_string();
    let invalid_shell = "infinishell-v0.1;touch-pwned".to_string();

    assert!(cleanup_script(&[invalid_path], "infinishell-v0.2").is_err());
    assert!(cleanup_script(&[invalid_shell], "infinishell-v0.2").is_err());
}

#[test]
fn cleanup_script_uses_only_the_fixed_directory_and_requested_names() {
    let script = cleanup_script(
        &["infinishell-v0.2026.08.18".to_string()],
        "infinishell-v0.2026.08.19",
    )
    .unwrap();

    assert!(script.contains("install_dir=\"$HOME/.infinishell/remote-server\""));
    assert!(script.contains("for file_name in 'infinishell-v0.2026.08.18'"));
    assert!(script.contains("rm -f -- \"$path\""));
    assert!(!script.contains("rm -rf"));
}

#[test]
fn cleanup_result_preserves_protected_and_failed_versions() {
    let result = parse_cleanup_output(
        "R\tinfinishell-v0.5\n\
C\tinfinishell-v0.4\n\
U\tinfinishell-v0.3\n\
M\tinfinishell-v0.2\n\
F\tinfinishell-v0.1\n",
    );

    assert_eq!(result.removed, vec!["infinishell-v0.5"]);
    assert_eq!(result.skipped_current, vec!["infinishell-v0.4"]);
    assert_eq!(result.skipped_running, vec!["infinishell-v0.3"]);
    assert_eq!(result.missing, vec!["infinishell-v0.2"]);
    assert_eq!(result.failed, vec!["infinishell-v0.1"]);
}

#[cfg(unix)]
#[test]
fn posix_scripts_scan_and_remove_only_the_requested_version() {
    let test_root = std::env::temp_dir().join(format!(
        "infinishell-storage-cleanup-{}",
        uuid::Uuid::new_v4()
    ));
    let install_dir = test_root.join(".infinishell/remote-server");
    fs::create_dir_all(&install_dir).unwrap();
    let removed_name = "infinishell-v0.2026.08.18";
    let retained_name = "infinishell-v0.2026.08.17";
    let removed_path = install_dir.join(removed_name);
    let retained_path = install_dir.join(retained_name);
    fs::write(&removed_path, "old").unwrap();
    fs::write(&retained_path, "older").unwrap();

    let scan_output = Command::new("bash")
        .arg("-c")
        .arg(scan_script())
        .env("HOME", &test_root)
        .output()
        .unwrap();
    assert!(scan_output.status.success());
    assert_eq!(
        parse_scan_output(
            &String::from_utf8_lossy(&scan_output.stdout),
            "infinishell-v0.2026.08.19"
        )
        .len(),
        2
    );

    let cleanup_output = Command::new("bash")
        .arg("-c")
        .arg(cleanup_script(&[removed_name.to_string()], "infinishell-v0.2026.08.19").unwrap())
        .env("HOME", &test_root)
        .output()
        .unwrap();
    assert!(cleanup_output.status.success());
    assert!(!removed_path.exists());
    assert!(retained_path.exists());
    assert_eq!(
        parse_cleanup_output(&String::from_utf8_lossy(&cleanup_output.stdout)).removed,
        vec![removed_name]
    );

    fs::remove_dir_all(test_root).unwrap();
}
