use super::{FileStamp, NativeBridgeProcess, validate_digest, verify_digest};
use command::managed::WindowsProcessIdentity;
use std::fs::{File, OpenOptions};
use std::io::{Seek as _, SeekFrom, Write as _};
use std::path::Path;

const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

fn fixture(directory: &Path) -> File {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(directory.join("image.exe"))
        .unwrap();
    file.write_all(b"hello").unwrap();
    file.sync_all().unwrap();
    file
}

#[test]
fn windows_process_snapshot_has_strict_platform_and_lifetime_fields() {
    let process = NativeBridgeProcess::from(WindowsProcessIdentity {
        pid: 41,
        created_at: 12345,
        logon_low: 9,
        logon_high: -1,
        session_id: 2,
    });
    let expected = serde_json::json!({"platform":"windows","pid":41,"created_at":12345,"logon_low":9,"logon_high":-1,"session_id":2});

    assert_eq!(serde_json::to_value(process).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<NativeBridgeProcess>(expected).unwrap(),
        process
    );
    assert!(serde_json::from_value::<NativeBridgeProcess>(serde_json::json!({"platform":"linux","pid":41,"created_at":12345,"logon_low":9,"logon_high":-1,"session_id":2})).is_err());
    assert!(serde_json::from_value::<NativeBridgeProcess>(serde_json::json!({"platform":"windows","pid":41,"created_at":12345,"logon_low":9,"logon_high":-1,"session_id":2,"token":"unexpected"})).is_err());
}

#[test]
fn missing_or_noncanonical_digest_cannot_authorize_an_artifact() {
    assert!(validate_digest("").is_err());
    assert!(validate_digest(&HELLO_SHA256.to_uppercase()).is_err());
    assert!(validate_digest(&HELLO_SHA256[..63]).is_err());
    validate_digest(HELLO_SHA256).unwrap();
}

#[test]
fn digest_reads_the_actual_file_from_zero_regardless_of_shared_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let file = fixture(directory.path());
    let stamp = FileStamp::read(&file).unwrap();
    let mut shared = file.try_clone().unwrap();
    shared.seek(SeekFrom::End(0)).unwrap();

    assert_eq!(
        verify_digest(
            &file,
            stamp,
            &[
                "0000000000000000000000000000000000000000000000000000000000000000",
                HELLO_SHA256,
            ],
        )
        .unwrap(),
        HELLO_SHA256,
    );
}

#[test]
fn changed_image_content_or_file_identity_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let mut file = fixture(directory.path());
    let stamp = FileStamp::read(&file).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"HELLO").unwrap();
    file.sync_all().unwrap();

    assert!(verify_digest(&file, stamp, &[HELLO_SHA256]).is_err());
    let other_directory = tempfile::tempdir().unwrap();
    let other = fixture(other_directory.path());
    assert!(verify_digest(&other, stamp, &[HELLO_SHA256]).is_err());
}
