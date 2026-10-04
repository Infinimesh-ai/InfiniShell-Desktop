use std::io::Write as _;

use super::*;

#[test]
fn temp_environment_binds_only_the_three_case_insensitive_keys() {
    let environment = [
        ("tmp".into(), "t".into()),
        ("TEMP".into(), "t".into()),
        ("UserProfile".into(), "h".into()),
        ("TMPDIR".into(), "ignored".into()),
    ];
    assert_eq!(
        expected_temp_environment(&environment),
        Some([vec![116], vec![116], vec![104]])
    );
}

#[test]
fn temp_environment_rejects_missing_duplicate_or_embedded_nul_values() {
    assert!(expected_temp_environment(&[]).is_none());
    let mut environment = vec![
        ("TMP".into(), "t".into()),
        ("TEMP".into(), "t".into()),
        ("USERPROFILE".into(), "h".into()),
    ];
    environment.push(("tmp".into(), "other".into()));
    assert!(expected_temp_environment(&environment).is_none());
    environment.pop();
    environment[0].1 = "t\0".into();
    assert!(expected_temp_environment(&environment).is_none());
}

#[test]
fn snapshots_are_due_once_at_each_fixed_boundary() {
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(14), false, false, true),
        SnapshotAction::Wait
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(15), false, false, true),
        SnapshotAction::Early
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(239), true, false, true),
        SnapshotAction::Wait
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(240), true, false, true),
        SnapshotAction::Late
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(241), true, true, true),
        SnapshotAction::Wait
    );
}

#[test]
fn cancellation_or_deadline_precedes_due_snapshot() {
    assert_eq!(
        snapshot_action(true, false, Duration::from_secs(15), false, false, true),
        SnapshotAction::Cancelled
    );
    assert_eq!(
        snapshot_action(true, true, Duration::from_secs(240), true, false, true),
        SnapshotAction::Cancelled
    );
    assert_eq!(
        snapshot_action(false, true, Duration::from_secs(240), true, false, true),
        SnapshotAction::Expired
    );
}

#[test]
fn snapshots_require_the_original_root_binding() {
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(15), false, false, false),
        SnapshotAction::Wait
    );
    assert_eq!(
        snapshot_action(false, false, Duration::from_secs(240), true, false, false),
        SnapshotAction::Wait
    );
}

#[test]
fn witness_requires_exact_generation_and_authorized_mode() {
    let generation = Uuid::new_v4();
    let value = generation.to_string();
    assert_eq!(
        enabled(generation, "cmd", Some(&value), Some("cmd")),
        Some(WitnessMode::Cmd)
    );
    assert_eq!(
        enabled(generation, "powershell", Some(&value), Some("powershell")),
        Some(WitnessMode::PowerShell)
    );
    assert_eq!(
        enabled(generation, "powershell", Some(&value), Some("cmd")),
        None
    );
    assert_eq!(
        enabled(generation, "cmd", Some(&value), Some("powershell")),
        None
    );
    assert_eq!(
        enabled(generation, "pwsh", Some(&value), Some("pwsh")),
        None
    );
    assert_eq!(enabled(generation, "cmd", Some(&value), None), None);
    assert_eq!(enabled(generation, "cmd", None, Some("cmd")), None);
    assert_eq!(enabled(generation, "cmd", Some("1"), Some("cmd")), None);
    assert_eq!(
        enabled(
            generation,
            "cmd",
            Some(&Uuid::new_v4().to_string()),
            Some("cmd")
        ),
        None
    );
    assert_eq!(
        enabled(generation, "cmd", Some(&format!("{value} ")), Some("cmd")),
        None
    );
}

#[test]
fn powershell_never_binds_creation_or_debug_registers() {
    let file = tempfile::tempfile().unwrap();
    let event = DEBUG_EVENT::default();
    let root = || VerifiedImage {
        file: &file,
        base: 0,
        size: 0,
        sha256: "",
    };
    assert!(
        WitnessMode::PowerShell
            .bind_creation(&event, root(), 0, 0)
            .unwrap()
            .is_none()
    );
    assert!(
        WitnessMode::Cmd
            .bind_creation(&event, root(), 0, 0)
            .is_err()
    );
}

#[test]
fn powershell_module_overflow_is_partial_but_cmd_still_rejects() {
    assert!(WitnessMode::PowerShell.retain_module(63, false).unwrap());
    assert!(!WitnessMode::PowerShell.retain_module(64, false).unwrap());
    assert!(!WitnessMode::PowerShell.retain_module(65, false).unwrap());
    assert!(WitnessMode::Cmd.retain_module(63, false).unwrap());
    assert!(WitnessMode::Cmd.retain_module(64, false).is_err());
}

#[test]
fn partial_module_coverage_never_admits_a_duplicate_binding() {
    assert!(WitnessMode::PowerShell.retain_module(64, true).is_err());
    assert!(WitnessMode::PowerShell.retain_module(1, true).is_err());
    assert!(WitnessMode::Cmd.retain_module(1, true).is_err());
}

#[test]
fn mapped_size_uses_pe_optional_header_not_file_length() {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&0x80u32.to_le_bytes());
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    bytes[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[0xd0..0xd4].copy_from_slice(&0x5000u32.to_le_bytes());
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(&bytes).unwrap();
    assert_eq!(mapped_image_size(&file).unwrap(), 0x5000);
    assert_ne!(
        mapped_image_size(&file).unwrap() as u64,
        file.metadata().unwrap().len()
    );
    file.set_len(0xd2).unwrap();
    assert!(mapped_image_size(&file).is_err());
}

#[test]
fn safe_failure_preserves_wrapped_native_error_without_error_text() {
    let hresult = HRESULT::from_win32(5);
    let failure = io::Error::other(WindowsError::from_hresult(hresult));
    let observed = safe_failure("thread_open", &failure);
    assert_eq!(observed["hresult"], hresult.0);
    assert_eq!(observed["win32_error"], 5);
    let direct = safe_failure("thread_open", &io::Error::from_raw_os_error(6));
    assert_eq!(direct["win32_error"], 6);
    let private = safe_failure(
        "module_binding",
        &io::Error::other("不得记录此路径或控制材料"),
    );
    assert!(!private.to_string().contains("不得记录"));
}
