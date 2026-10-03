use std::io::Write as _;

use super::*;

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
