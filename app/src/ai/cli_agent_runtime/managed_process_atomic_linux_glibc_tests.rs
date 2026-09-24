use super::*;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;

fn cache(entries: &[(&str, &str, u64, u32)], hwcaps: &[&str]) -> Vec<u8> {
    let mut bytes = vec![0; 48 + entries.len() * 24];
    bytes[..20].copy_from_slice(b"glibc-ld.so.cache1.1");
    bytes[20..24].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    bytes[28] = 2;
    let string_start = bytes.len();
    for (index, (name, path, hwcap, minimum_kernel)) in entries.iter().enumerate() {
        let key = bytes.len() as u32;
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        let value = bytes.len() as u32;
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
        let at = 48 + index * 24;
        bytes[at..at + 4].copy_from_slice(&0x303_u32.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&key.to_le_bytes());
        bytes[at + 8..at + 12].copy_from_slice(&value.to_le_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&minimum_kernel.to_le_bytes());
        bytes[at + 16..at + 24].copy_from_slice(&hwcap.to_le_bytes());
    }
    let mut indices = Vec::new();
    for name in hwcaps {
        indices.push(bytes.len() as u32);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
    }
    let string_length = bytes.len() - string_start;
    bytes[24..28].copy_from_slice(&(string_length as u32).to_le_bytes());
    if !hwcaps.is_empty() {
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        let extension = bytes.len();
        bytes[32..36].copy_from_slice(&(extension as u32).to_le_bytes());
        bytes.extend_from_slice(&0xeaa4_2174_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&((extension + 24) as u32).to_le_bytes());
        bytes.extend_from_slice(&((indices.len() * 4) as u32).to_le_bytes());
        for index in indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
    }
    bytes
}

#[test]
fn cache_retains_all_hwcaps_candidates_and_requires_a_baseline() {
    let bytes = cache(
        &[
            (
                "libc.so.6",
                "/lib/glibc-hwcaps/x86-64-v3/libc.so.6",
                HWCAP_EXTENSION,
                0,
            ),
            ("libc.so.6", "/lib/libc.so.6", 0, 0),
        ],
        &["x86-64-v3"],
    );
    let parsed = parse_cache(&bytes).unwrap();
    let entries = dependency_candidates(&parsed, "libc.so.6", 0x06_08_00).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].path,
        Path::new("/lib/glibc-hwcaps/x86-64-v3/libc.so.6")
    );
    assert!(!entries[0].baseline);
    assert!(entries[1].baseline);

    let bytes = cache(
        &[(
            "libc.so.6",
            "/lib/glibc-hwcaps/x86-64-v3/libc.so.6",
            HWCAP_EXTENSION,
            0,
        )],
        &["x86-64-v3"],
    );
    let parsed = parse_cache(&bytes).unwrap();
    assert_eq!(
        dependency_candidates(&parsed, "libc.so.6", 0x06_08_00)
            .unwrap_err()
            .to_string(),
        "managed_process.linux_glibc_cache_baseline_missing"
    );
}

#[test]
fn older_cache_minimum_kernel_requires_a_usable_baseline_without_dropping_candidates() {
    let bytes = cache(
        &[
            ("libc.so.6", "/lib/libc.so.6", 0, 0x03_02_00),
            (
                "libc.so.6",
                "/lib/glibc-hwcaps/x86-64-v3/libc.so.6",
                HWCAP_EXTENSION,
                0x07_00_00,
            ),
        ],
        &["x86-64-v3"],
    );
    let parsed = parse_cache(&bytes).unwrap();
    assert_eq!(
        dependency_candidates(&parsed, "libc.so.6", 0x06_08_00)
            .unwrap()
            .len(),
        2
    );
    assert!(dependency_candidates(&parsed, "libc.so.6", 0x02_06_00).is_err());
}

#[test]
fn unrelated_local_cache_entries_do_not_expand_trusted_dependency_paths() {
    let bytes = cache(
        &[
            ("libz.so.1", "/usr/local/lib/libz.so.1", 0, 0),
            ("libc.so.6", "/lib/libc.so.6", 0, 0),
        ],
        &[],
    );
    let parsed = parse_cache(&bytes).unwrap();
    assert_eq!(
        dependency_candidates(&parsed, "libc.so.6", 0x06_08_00)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        dependency_candidates(&parsed, "libz.so.1", 0x06_08_00)
            .unwrap_err()
            .to_string(),
        "managed_process.linux_glibc_system_file_untrusted"
    );
    assert!(dependency_candidates(&parsed, "libmissing.so.1", 0x06_08_00).is_err());
}

#[test]
fn cache_rejects_truncated_offsets_wrong_endian_and_unknown_extensions() {
    let original = cache(&[("libc.so.6", "/lib/libc.so.6", 0, 0)], &["x86-64-v3"]);
    let mut bytes = original.clone();
    bytes[52..56].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(parse_cache(&bytes).is_err());
    let mut bytes = original.clone();
    bytes[28] = 3;
    assert!(parse_cache(&bytes).is_err());
    let mut bytes = original;
    let extension = u32_at(&bytes, 32).unwrap() as usize;
    bytes[extension + 8..extension + 12].copy_from_slice(&7_u32.to_le_bytes());
    assert!(parse_cache(&bytes).is_err());
}

#[test]
fn cache_rejects_numeric_aliases_bad_sort_order_and_unbound_hwcaps() {
    assert!(
        parse_cache(&cache(
            &[
                ("liba01.so", "/lib/liba01.so", 0, 0),
                ("liba1.so", "/lib/liba1.so", 0, 0),
            ],
            &[]
        ))
        .is_err()
    );
    assert!(
        parse_cache(&cache(
            &[
                ("liba.so", "/lib/liba.so", 0, 0),
                ("libz.so", "/lib/libz.so", 0, 0),
            ],
            &[]
        ))
        .is_err()
    );
    assert!(
        parse_cache(&cache(
            &[("libc.so.6", "/lib/libc.so.6", HWCAP_EXTENSION, 0)],
            &[]
        ))
        .is_err()
    );
    assert!(parse_cache(&cache(&[("libc.so.6", "/lib/libc.so.6", 1, 0)], &[])).is_err());
}

fn elf(extra_tag: u64, needed: &str) -> tempfile::NamedTempFile {
    let strings = format!("\0{needed}\0libfixture.so.1\0").into_bytes();
    let mut bytes = vec![0; 512 + strings.len()];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&3_u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&62_u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
    bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
    bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&2_u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&1_u32.to_le_bytes());
    let size = bytes.len() as u64;
    bytes[96..104].copy_from_slice(&size.to_le_bytes());
    bytes[104..112].copy_from_slice(&size.to_le_bytes());
    bytes[120..124].copy_from_slice(&2_u32.to_le_bytes());
    bytes[128..136].copy_from_slice(&256_u64.to_le_bytes());
    bytes[136..144].copy_from_slice(&256_u64.to_le_bytes());
    bytes[152..160].copy_from_slice(&96_u64.to_le_bytes());
    for (index, (tag, value)) in [
        (5_u64, 512_u64),
        (10, strings.len() as u64),
        (1, 1),
        (14, needed.len() as u64 + 2),
        (extra_tag, 0),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 256 + index * 16;
        bytes[at..at + 8].copy_from_slice(&tag.to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&value.to_le_bytes());
    }
    bytes[512..].copy_from_slice(&strings);
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&bytes).unwrap();
    file
}

#[test]
fn elf_reads_bounded_needed_and_soname_from_virtual_string_mapping() {
    let file = elf(0, "libc.so.6");
    let image = parse_elf(file.as_file(), file.as_file().metadata().unwrap().len()).unwrap();
    assert_eq!(image.needed, ["libc.so.6"]);
    assert_eq!(image.soname.as_deref(), Some("libfixture.so.1"));
}

#[test]
fn elf_rejects_external_search_audit_filter_and_path_dependencies() {
    for tag in [
        15,
        29,
        0x6fff_fefa,
        0x6fff_fefb,
        0x6fff_fefc,
        0x7fff_fffd,
        0x7fff_ffff,
    ] {
        let file = elf(tag, "libc.so.6");
        assert!(
            parse_elf(file.as_file(), file.as_file().metadata().unwrap().len()).is_err(),
            "未拒绝外部解析 tag：{tag}"
        );
    }
    let file = elf(0, "../libc.so.6");
    assert!(parse_elf(file.as_file(), file.as_file().metadata().unwrap().len()).is_err());
}

#[test]
fn system_file_rejects_writable_ancestor_and_writable_objects() {
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("libfixture.so");
    fs::write(&path, b"fixture").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(require_root(&fs::metadata(&path).unwrap(), false).is_err());
    assert!(open_system_file(&path).is_err());
}

#[test]
fn symlink_components_preserve_parent_traversal_until_after_link_resolution() {
    assert_eq!(
        path_components(Path::new("/usr/lib/alias/../loader")).unwrap(),
        VecDeque::from([
            std::ffi::OsString::from("usr"),
            std::ffi::OsString::from("lib"),
            std::ffi::OsString::from("alias"),
            std::ffi::OsString::from(".."),
            std::ffi::OsString::from("loader")
        ])
    );
}

#[test]
#[cfg(target_arch = "x86_64")]
fn real_system_cache_is_read_after_hashing_and_file_hash_is_rechecked() {
    let cache = BoundFile::capture(Path::new(CACHE), MAX_CACHE).unwrap();
    let bytes = read(&cache.file, 0, cache.identity.2 as usize, cache.identity.2).unwrap();
    let parsed = parse_cache(&bytes).unwrap();
    dependency_candidates(&parsed, "libc.so.6", kernel_version().unwrap()).unwrap();
    let mut loader =
        BoundFile::capture(Path::new(INTERPRETER), MAX_NATIVE_EXECUTABLE_BYTES).unwrap();
    loader.verify().unwrap();
    loader.sha256 = "0".repeat(64);
    assert!(loader.verify().is_err());
}

#[test]
fn elf_rejects_dynamic_table_with_a_different_mapped_address() {
    let file = elf(0, "libc.so.6");
    file.as_file()
        .write_all_at(&272_u64.to_le_bytes(), 136)
        .unwrap();
    assert!(parse_elf(file.as_file(), file.as_file().metadata().unwrap().len()).is_err());
}
