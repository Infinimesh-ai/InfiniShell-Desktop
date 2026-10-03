use std::fs::{self, OpenOptions};
use std::io::{Seek as _, SeekFrom};
use std::os::windows::fs::OpenOptionsExt as _;

use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

use super::*;

#[test]
fn address_reads_reject_cross_boundary_zero_and_overflow() {
    assert!(contained(0x1000, 0x100, 0x10f8, 8));
    assert!(!contained(0x1000, 0x100, 0x10f8, 9));
    assert!(!contained(0x1000, 0x100, 0x0fff, 1));
    assert!(!contained(0x1000, 0x100, 0x1000, 0));
    assert!(!contained(u64::MAX - 3, 8, u64::MAX - 2, 2));
    assert!(!contained(MAX_USER_ADDRESS, 2, MAX_USER_ADDRESS, 1));
}

#[test]
fn exception_table_rejects_overlap_truncation_and_unwind_outside_image() {
    let valid = [
        0x00, 0x11, 0, 0, 0x20, 0x11, 0, 0, 0x00, 0x20, 0, 0, 0x20, 0x11, 0, 0, 0x40, 0x11, 0, 0,
        0x10, 0x20, 0, 0,
    ];
    assert_eq!(parse_functions(&valid, 0x3000).unwrap().len(), 2);
    assert_eq!(
        parse_functions(&valid[..23], 0x3000).unwrap_err().reason,
        "invalid_exception_table_size"
    );

    let mut overlap = valid;
    overlap[12] = 0x10;
    assert_eq!(
        parse_functions(&overlap, 0x3000).unwrap_err().reason,
        "invalid_exception_table_entry"
    );

    let mut outside = valid;
    outside[9] = 0x30;
    assert_eq!(
        parse_functions(&outside, 0x3000).unwrap_err().reason,
        "invalid_exception_table_entry"
    );
}

fn pe_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x218];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&0x80u32.to_le_bytes());
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    bytes[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
    bytes[0x94..0x96].copy_from_slice(&0xf0u16.to_le_bytes());
    bytes[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[0xd0..0xd4].copy_from_slice(&0x3000u32.to_le_bytes());
    bytes[0x104..0x108].copy_from_slice(&16u32.to_le_bytes());
    bytes[0x120..0x124].copy_from_slice(&0x1000u32.to_le_bytes());
    bytes[0x124..0x128].copy_from_slice(&24u32.to_le_bytes());
    bytes[0x194..0x198].copy_from_slice(&0x1000u32.to_le_bytes());
    bytes[0x198..0x19c].copy_from_slice(&24u32.to_le_bytes());
    bytes[0x19c..0x1a0].copy_from_slice(&0x200u32.to_le_bytes());
    bytes[0x200..].copy_from_slice(&[
        0x00, 0x11, 0, 0, 0x20, 0x11, 0, 0, 0x00, 0x20, 0, 0, 0x20, 0x11, 0, 0, 0x40, 0x11, 0, 0,
        0x10, 0x20, 0, 0,
    ]);
    bytes
}

/// 匿名 tempfile 在 Windows 使用 share_mode(0)；这里按生产映像租约只共享读取。
fn leased_pe(bytes: &[u8]) -> (tempfile::TempDir, File) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image.exe");
    fs::write(&path, bytes).unwrap();
    let file = OpenOptions::new()
        .access_mode(GENERIC_READ.0)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .unwrap();
    (directory, file)
}

#[test]
fn exception_directory_uses_file_offsets_and_preserves_the_file_cursor() {
    let (directory, mut file) = leased_pe(&pe_fixture());
    file.seek(SeekFrom::End(0)).unwrap();
    let functions = load_functions(&file, 0x3000).unwrap();
    assert_eq!(
        functions,
        [
            RuntimeFunction {
                begin: 0x1100,
                end: 0x1120,
                unwind: 0x2000
            },
            RuntimeFunction {
                begin: 0x1120,
                end: 0x1140,
                unwind: 0x2010
            },
        ]
    );
    assert_eq!(file.stream_position().unwrap(), 0x218);
    let path = directory.path().join("image.exe");
    assert!(OpenOptions::new().write(true).open(&path).is_err());
    assert!(fs::rename(&path, directory.path().join("replaced.exe")).is_err());
    assert_eq!(
        load_functions(&file, 0x4000).unwrap_err().reason,
        "image_layout_mismatch"
    );
}

#[test]
fn truncated_exception_data_is_not_treated_as_a_leaf_function() {
    let (_directory, file) = leased_pe(&pe_fixture()[..0x210]);
    assert_eq!(
        load_functions(&file, 0x3000).unwrap_err().reason,
        "truncated_leased_image"
    );
}

#[test]
fn module_overlap_is_rejected_and_unknown_pc_is_not_attributed() {
    let (_directory, file) = leased_pe(&pe_fixture());
    let sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let modules = prepare_modules(&[VerifiedModule {
        file: &file,
        base: 0x10000,
        size: 0x3000,
        sha256: sha,
    }])
    .unwrap();
    assert_eq!(
        modules.frame(0x11108, 0x80000),
        Frame {
            instruction: 0x11108,
            stack: 0x80000,
            module_index: Some(0),
            module_offset: Some(0x1108),
        }
    );
    assert_eq!(
        modules.frame(0x13000, 0x80000),
        Frame {
            instruction: 0x13000,
            stack: 0x80000,
            module_index: None,
            module_offset: None,
        }
    );
    let result = prepare_modules(&[
        VerifiedModule {
            file: &file,
            base: 0x10000,
            size: 0x3000,
            sha256: sha,
        },
        VerifiedModule {
            file: &file,
            base: 0x12000,
            size: 0x3000,
            sha256: sha,
        },
    ]);
    assert_eq!(result.err().unwrap().reason, "invalid_module_binding");
}

#[test]
fn callback_only_returns_the_exception_entry_containing_pc() {
    let (_directory, file) = leased_pe(&pe_fixture());
    let modules = prepare_modules(&[VerifiedModule {
        file: &file,
        base: 0x10000,
        size: 0x3000,
        sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    }])
    .unwrap();
    // 使用无效的合成句柄只测试本地查表；本测试不调用读取/暂停/派生 API。
    let process = HANDLE(1usize as *mut c_void);
    let mut state = WalkState {
        process,
        modules: &modules,
        stack: 0x80000,
        remaining: MAX_READ_BYTES,
        failure: None,
    };
    ACTIVE_WALK.with(|active| active.set(ptr::from_mut(&mut state).cast()));
    let active = ActiveWalk;
    let found = unsafe { function_table(process, 0x11120) };
    assert!(!found.is_null());
    assert_eq!(
        unsafe { *found.cast::<RuntimeFunction>() },
        RuntimeFunction {
            begin: 0x1120,
            end: 0x1140,
            unwind: 0x2010
        }
    );
    assert!(unsafe { function_table(process, 0x11140) }.is_null());
    assert!(unsafe { function_table(HANDLE(2usize as *mut c_void), 0x11120) }.is_null());
    drop(active);
    assert!(unsafe { function_table(process, 0x11120) }.is_null());
}

#[test]
fn rejected_memory_read_keeps_the_buffer_and_budget_untouched() {
    let process = HANDLE(1usize as *mut c_void);
    let modules = PreparedModules {
        modules: Vec::new(),
    };
    let mut state = WalkState {
        process,
        modules: &modules,
        stack: 0x80000,
        remaining: 8,
        failure: None,
    };
    ACTIVE_WALK.with(|active| active.set(ptr::from_mut(&mut state).cast()));
    let active = ActiveWalk;
    let mut buffer = [0xa5u8; 8];
    let mut read = 99;
    let result = unsafe { read_memory(process, 0x70000, buffer.as_mut_ptr().cast(), 8, &mut read) };
    assert!(!result.as_bool());
    assert_eq!(read, 0);
    assert_eq!(buffer, [0xa5; 8]);
    assert_eq!(state.remaining, 8);
    assert_eq!(
        state.failure.as_ref().unwrap().reason,
        "stack_read_out_of_bounds"
    );
    drop(active);
}

#[test]
fn empty_module_map_never_authorizes_instruction_or_read() {
    let prepared = prepare_modules(&[]).unwrap();
    assert!(prepared.module(0x12340000).is_none());
    let frame = prepared.frame(0x12340000, 0x56780000);
    assert!(frame.module_index.is_none());
    assert!(frame.module_offset.is_none());
}
