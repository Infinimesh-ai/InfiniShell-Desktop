use std::io::Write as _;

use super::*;

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
fn witness_requires_exact_generation_and_first_cmd_mode() {
    let generation = Uuid::new_v4();
    let value = generation.to_string();
    assert!(enabled(generation, "cmd", Some(&value)));
    assert!(!enabled(generation, "powershell", Some(&value)));
    assert!(!enabled(generation, "cmd", None));
    assert!(!enabled(generation, "cmd", Some("1")));
    assert!(!enabled(
        generation,
        "cmd",
        Some(&Uuid::new_v4().to_string())
    ));
    assert!(!enabled(generation, "cmd", Some(&format!("{value} "))));
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
