use super::*;
use std::io::Write as _;
use std::os::unix::fs::FileExt as _;

fn elf(
    interpreter: Option<&[u8]>,
    needed: &[&str],
    extra_tags: &[(u64, u64)],
) -> tempfile::NamedTempFile {
    let mut strings = vec![0];
    let mut entries = vec![(5_u64, 512_u64), (10, 0)];
    for name in needed {
        entries.push((1, strings.len() as u64));
        strings.extend_from_slice(name.as_bytes());
        strings.push(0);
    }
    entries[1].1 = strings.len() as u64;
    entries.extend_from_slice(extra_tags);
    entries.push((0, 0));
    let mut bytes = vec![0_u8; 1024];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&3_u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&62_u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
    bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
    bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&(if interpreter.is_some() { 3_u16 } else { 2 }).to_le_bytes());
    bytes[64..68].copy_from_slice(&1_u32.to_le_bytes());
    bytes[96..104].copy_from_slice(&1024_u64.to_le_bytes());
    bytes[104..112].copy_from_slice(&1024_u64.to_le_bytes());
    bytes[120..124].copy_from_slice(&2_u32.to_le_bytes());
    bytes[128..136].copy_from_slice(&256_u64.to_le_bytes());
    bytes[136..144].copy_from_slice(&256_u64.to_le_bytes());
    bytes[152..160].copy_from_slice(&(entries.len() as u64 * 16).to_le_bytes());
    if let Some(interpreter) = interpreter {
        bytes[176..180].copy_from_slice(&3_u32.to_le_bytes());
        bytes[184..192].copy_from_slice(&448_u64.to_le_bytes());
        bytes[192..200].copy_from_slice(&448_u64.to_le_bytes());
        bytes[208..216].copy_from_slice(&(interpreter.len() as u64).to_le_bytes());
        bytes[448..448 + interpreter.len()].copy_from_slice(interpreter);
    }
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        let at = 256 + index * 16;
        bytes[at..at + 8].copy_from_slice(&tag.to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&value.to_le_bytes());
    }
    bytes[512..512 + strings.len()].copy_from_slice(&strings);
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&bytes).unwrap();
    file
}

#[test]
fn exact_musl_interpreter_and_libc_self_dependency_are_accepted() {
    let file = elf(Some(INTERPRETER.to_bytes_with_nul()), &[LIBC], &[]);

    verify_main(file.as_file(), 1024).unwrap();
}

#[test]
fn musl_rejects_missing_duplicate_or_additional_dependencies() {
    for needed in [
        vec![],
        vec![LIBC, LIBC],
        vec![LIBC, "libother.so"],
        vec!["libc.so.6"],
    ] {
        let file = elf(Some(INTERPRETER.to_bytes_with_nul()), &needed, &[]);

        assert_eq!(
            verify_main(file.as_file(), 1024).unwrap_err().to_string(),
            "managed_process.linux_musl_dependency_unbound"
        );
    }
}

#[test]
fn musl_rejects_changed_or_unterminated_interpreter() {
    for interpreter in [
        b"/lib64/ld-linux-x86-64.so.2\0".as_slice(),
        b"/tmp/ld-musl-x86_64.so.1\0",
        b"/lib/ld-musl-x86_64.so.1",
        b"/lib/ld-musl-x86_64.so.1\0extra\0",
    ] {
        let file = elf(Some(interpreter), &[LIBC], &[]);

        assert!(verify_main(file.as_file(), 1024).is_err());
    }
}

#[test]
fn musl_rejects_search_audit_filter_and_path_dependencies() {
    for tag in [
        15,
        29,
        0x6fff_fefa,
        0x6fff_fefb,
        0x6fff_fefc,
        0x7fff_fffd,
        0x7fff_ffff,
    ] {
        let file = elf(Some(INTERPRETER.to_bytes_with_nul()), &[LIBC], &[(tag, 1)]);

        assert!(verify_main(file.as_file(), 1024).is_err());
    }
    let file = elf(
        Some(INTERPRETER.to_bytes_with_nul()),
        &["/lib/libc.musl-x86_64.so.1"],
        &[],
    );
    assert!(verify_main(file.as_file(), 1024).is_err());
}

#[test]
fn musl_loader_accepts_the_self_contained_shared_object_without_soname() {
    let file = elf(None, &[], &[]);

    verify_loader(file.as_file(), 1024).unwrap();
}

#[test]
fn musl_loader_rejects_a_second_interpreter_dependency_or_search_path() {
    for file in [
        elf(Some(INTERPRETER.to_bytes_with_nul()), &[], &[]),
        elf(None, &[LIBC], &[]),
        elf(None, &[], &[(29, 0)]),
    ] {
        assert!(verify_loader(file.as_file(), 1024).is_err());
    }
}

#[test]
fn musl_loader_rejects_an_executable_in_place_of_the_shared_object() {
    let file = elf(None, &[], &[]);
    file.as_file()
        .write_all_at(&2_u16.to_le_bytes(), 16)
        .unwrap();

    assert!(verify_loader(file.as_file(), 1024).is_err());
}

#[test]
fn musl_loader_rejects_a_foreign_soname() {
    let file = elf(None, &["ld-linux-x86-64.so.2"], &[(14, 1)]);
    // 把夹具的 DT_NEEDED 改为无路径影响的 DT_DEBUG，只留下待核 SONAME。
    file.as_file()
        .write_all_at(&21_u64.to_le_bytes(), 288)
        .unwrap();

    assert!(verify_loader(file.as_file(), 1024).is_err());
}

#[test]
fn musl_rejects_dynamic_tables_outside_the_bound_mapping() {
    let file = elf(Some(INTERPRETER.to_bytes_with_nul()), &[LIBC], &[]);
    file.as_file()
        .write_all_at(&1024_u64.to_le_bytes(), 136)
        .unwrap();

    assert!(verify_main(file.as_file(), 1024).is_err());
}

#[test]
fn musl_closure_rejects_a_changed_bound_system_file_identity() {
    // 只读已存在的系统文件；这里核共用身份复核，不声称 shell 是合法 musl loader。
    let mut loader = BoundFile::capture(Path::new("/bin/sh"), MAX_NATIVE_EXECUTABLE_BYTES).unwrap();
    loader.identity.1 ^= 1;
    let closure = SystemClosure { loader };

    assert_eq!(
        closure.verify().unwrap_err().to_string(),
        "managed_process.linux_musl_binding_changed"
    );
}

#[test]
fn musl_rejects_duplicate_interpreter_before_system_loader_lookup() {
    let file = elf(Some(INTERPRETER.to_bytes_with_nul()), &[LIBC], &[]);
    file.as_file()
        .write_all_at(&4_u16.to_le_bytes(), 56)
        .unwrap();
    let mut header = [0_u8; 56];
    file.as_file().read_exact_at(&mut header, 176).unwrap();
    file.as_file().write_all_at(&header, 232).unwrap();

    assert_eq!(
        super::super::verify_elf(file.as_file(), 1024)
            .unwrap_err()
            .to_string(),
        "managed_process.linux_atomic_interpreter_invalid"
    );
}
