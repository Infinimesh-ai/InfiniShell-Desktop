use std::fs::{self, OpenOptions};
use std::io::{Seek as _, SeekFrom};
use std::os::windows::fs::OpenOptionsExt as _;

use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
use windows::Win32::System::Threading::GetCurrentProcess;

use super::*;

fn cpu_sample() -> CpuSample {
    CpuSample {
        identity: Identity {
            process_id: 10,
            process_created: 20,
            thread_id: 30,
            thread_created: 40,
        },
        sampled_at_ms: 15_000,
        cumulative: CpuTimes {
            kernel_100ns: 100,
            user_100ns: 200,
        },
    }
}

#[test]
fn cpu_difference_uses_exact_identity_and_cumulative_100ns_units() {
    let earlier = cpu_sample();
    let later = CpuSample {
        sampled_at_ms: 240_000,
        cumulative: CpuTimes {
            kernel_100ns: 150,
            user_100ns: 500,
        },
        ..earlier
    };
    assert_eq!(
        later.difference_from(&earlier).unwrap(),
        CpuTimes {
            kernel_100ns: 50,
            user_100ns: 300
        }
    );
}

#[test]
fn cpu_difference_rejects_reused_process_or_thread_identity() {
    let earlier = cpu_sample();
    let later = CpuSample {
        sampled_at_ms: 240_000,
        ..earlier
    };
    assert_eq!(
        CpuSample {
            identity: Identity {
                process_created: 21,
                ..later.identity
            },
            ..later
        }
        .difference_from(&earlier)
        .unwrap_err()
        .reason,
        "cpu_identity_changed"
    );
    assert_eq!(
        CpuSample {
            identity: Identity {
                thread_created: 41,
                ..later.identity
            },
            ..later
        }
        .difference_from(&earlier)
        .unwrap_err()
        .reason,
        "cpu_identity_changed"
    );
    assert_eq!(
        CpuSample {
            identity: Identity {
                process_id: 11,
                ..later.identity
            },
            ..later
        }
        .difference_from(&earlier)
        .unwrap_err()
        .reason,
        "cpu_identity_changed"
    );
    assert_eq!(
        CpuSample {
            identity: Identity {
                thread_id: 31,
                ..later.identity
            },
            ..later
        }
        .difference_from(&earlier)
        .unwrap_err()
        .reason,
        "cpu_identity_changed"
    );
}

#[test]
fn cpu_difference_rejects_equal_or_reversed_sample_time() {
    let earlier = cpu_sample();
    assert_eq!(
        earlier.difference_from(&earlier).unwrap_err().reason,
        "cpu_sample_time_not_increasing"
    );
    let reversed = CpuSample {
        sampled_at_ms: 14_999,
        ..earlier
    };
    assert_eq!(
        reversed.difference_from(&earlier).unwrap_err().reason,
        "cpu_sample_time_not_increasing"
    );
}

#[test]
fn cpu_difference_rejects_decreased_cumulative_time() {
    let earlier = cpu_sample();
    let later = CpuSample {
        sampled_at_ms: 240_000,
        cumulative: CpuTimes {
            kernel_100ns: 99,
            user_100ns: 300,
        },
        ..earlier
    };
    assert_eq!(
        later.difference_from(&earlier).unwrap_err().reason,
        "cpu_cumulative_time_decreased"
    );
    let later = CpuSample {
        cumulative: CpuTimes {
            kernel_100ns: 200,
            user_100ns: 199,
        },
        ..later
    };
    assert_eq!(
        later.difference_from(&earlier).unwrap_err().reason,
        "cpu_cumulative_time_decreased"
    );
}

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
fn partial_preparation_keeps_valid_modules_and_excludes_bad_unwind_tables() {
    let (_good_directory, good) = leased_pe(&pe_fixture());
    let mut bytes = pe_fixture();
    bytes[0x124..0x128].fill(0);
    let (_bad_directory, bad) = leased_pe(&bytes);
    let sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let result = prepare_partial_modules(&[
        VerifiedModule {
            file: &bad,
            base: 0x10000,
            size: 0x3000,
            sha256: sha,
        },
        VerifiedModule {
            file: &good,
            base: 0x20000,
            size: 0x3000,
            sha256: sha,
        },
    ])
    .unwrap();

    assert_eq!(result.prepared.modules.len(), 1);
    assert_eq!(result.omitted.len(), 1);
    assert_eq!(
        result.omitted[0].failure.reason,
        "exception_directory_zero_mismatch"
    );
    assert_eq!(result.omitted[0].identity.base, 0x10000);
    assert_eq!(result.omitted[0].identity.sha256, sha);
    assert_eq!(result.omitted[0].directory.as_ref().unwrap().rva, 0x1000);
    assert_eq!(result.omitted[0].directory.as_ref().unwrap().length, 0);
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(bad.as_raw_handle()), &mut info) }.unwrap();
    assert_eq!(
        result.omitted[0].identity.volume_serial,
        info.dwVolumeSerialNumber
    );
    assert_eq!(
        result.omitted[0].identity.file_index,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)
    );
    // walk 在同一 module 查询为 None 时立即停止，不能为坏表生成叶函数或地址归属。
    assert!(result.prepared.module(0x11100).is_none());
    assert_eq!(result.prepared.frame(0x11100, 0x80000).module_index, None);
    assert_eq!(
        result.prepared.frame(0x21100, 0x80000).module_index,
        Some(0)
    );
}

#[test]
fn omitted_bad_module_still_participates_in_global_overlap_rejection() {
    let mut bytes = pe_fixture();
    bytes[0x124..0x128].fill(0);
    let (_bad_directory, bad) = leased_pe(&bytes);
    let (_good_directory, good) = leased_pe(&pe_fixture());
    let sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let result = prepare_partial_modules(&[
        VerifiedModule {
            file: &bad,
            base: 0x10000,
            size: 0x3000,
            sha256: sha,
        },
        VerifiedModule {
            file: &good,
            base: 0x12000,
            size: 0x3000,
            sha256: sha,
        },
    ]);

    assert_eq!(result.err().unwrap().reason, "invalid_module_binding");
}

#[test]
fn exception_directory_budget_and_address_failures_are_distinct() {
    let mut oversized = pe_fixture();
    oversized[0x124..0x128].copy_from_slice(&0xc0001u32.to_le_bytes());
    let (_large_directory, large) = leased_pe(&oversized);
    assert_eq!(
        load_functions(&large, 0x3000).unwrap_err().reason,
        "exception_directory_per_module_budget"
    );

    let mut outside = pe_fixture();
    outside[0x120..0x124].copy_from_slice(&0x2fffu32.to_le_bytes());
    let (_outside_directory, outside) = leased_pe(&outside);
    assert_eq!(
        load_functions(&outside, 0x3000).unwrap_err().reason,
        "exception_directory_outside_image"
    );
}

#[test]
fn failed_exception_table_reads_do_not_refund_the_shared_budget() {
    let (_directory, file) = leased_pe(&pe_fixture()[..0x210]);
    let mut remaining = 24;
    let mut directory = None;

    let failure =
        load_functions_budgeted(&file, 0x3000, &mut remaining, &mut directory).unwrap_err();

    assert_eq!(failure.reason, "truncated_leased_image");
    assert_eq!(remaining, 0);
    assert_eq!(directory.unwrap().length, 24);
    let mut later_directory = None;
    let later =
        load_functions_budgeted(&file, 0x3000, &mut remaining, &mut later_directory).unwrap_err();
    assert_eq!(later.reason, "exception_total_budget_exhausted");
    assert!(later_directory.is_none());
}

fn bad_large_exception_fixture(length: u32) -> Vec<u8> {
    let mut bytes = pe_fixture();
    bytes.resize(0x200 + length as usize, 0);
    bytes[0x200..].fill(0);
    bytes[0xd0..0xd4].copy_from_slice(&0x100000u32.to_le_bytes());
    bytes[0x124..0x128].copy_from_slice(&length.to_le_bytes());
    bytes[0x198..0x19c].copy_from_slice(&length.to_le_bytes());
    bytes
}

#[test]
fn omitted_tables_cannot_multiply_the_total_exception_read_budget() {
    let (_large_directory, large) = leased_pe(&bad_large_exception_fixture(0xc0000));
    let (_tail_directory, tail) = leased_pe(&bad_large_exception_fixture(0x40000));
    let sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let result = prepare_partial_modules(&[
        VerifiedModule {
            file: &large,
            base: 0x100000,
            size: 0x100000,
            sha256: sha,
        },
        VerifiedModule {
            file: &large,
            base: 0x200000,
            size: 0x100000,
            sha256: sha,
        },
        VerifiedModule {
            file: &large,
            base: 0x300000,
            size: 0x100000,
            sha256: sha,
        },
        VerifiedModule {
            file: &large,
            base: 0x400000,
            size: 0x100000,
            sha256: sha,
        },
        VerifiedModule {
            file: &large,
            base: 0x500000,
            size: 0x100000,
            sha256: sha,
        },
        VerifiedModule {
            file: &tail,
            base: 0x600000,
            size: 0x100000,
            sha256: sha,
        },
        VerifiedModule {
            file: &large,
            base: 0x700000,
            size: 0x100000,
            sha256: sha,
        },
    ])
    .unwrap();

    assert!(result.prepared.modules.is_empty());
    assert_eq!(result.exception_bytes_attempted, 4_194_304);
    assert_eq!(result.omitted.len(), 7);
    assert_eq!(
        result.omitted[0].failure.reason,
        "invalid_exception_table_entry"
    );
    assert_eq!(
        result.omitted[5].failure.reason,
        "invalid_exception_table_size"
    );
    assert_eq!(
        result.omitted[6].failure.reason,
        "exception_total_budget_exhausted"
    );
    assert!(result.omitted[6].directory.is_none());
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
        stopped: &|| false,
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
        stopped: &|| false,
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
fn cancelled_read_keeps_the_buffer_and_shared_budget_untouched() {
    let modules = PreparedModules {
        modules: Vec::new(),
    };
    let mut state = WalkState {
        process: HANDLE(1usize as *mut c_void),
        modules: &modules,
        stack: 0x80000,
        remaining: 8,
        failure: None,
        stopped: &|| true,
    };
    let mut bytes = [0xa5; 8];
    assert_eq!(
        state.read_bytes(0x80000, &mut bytes).unwrap_err().reason,
        "cancelled_or_expired_before_read"
    );
    assert_eq!(bytes, [0xa5; 8]);
    assert_eq!(state.remaining, 8);
}

#[test]
fn direct_and_stack_reads_share_one_budget() {
    let source = [1u8, 2, 3, 4];
    let address = source.as_ptr() as u64;
    let modules = PreparedModules {
        modules: Vec::new(),
    };
    let process = unsafe { GetCurrentProcess() };
    let mut state = WalkState {
        process,
        modules: &modules,
        stack: address,
        remaining: 8,
        failure: None,
        stopped: &|| false,
    };
    let mut bytes = [0u8; 4];
    state.read_bytes(address, &mut bytes).unwrap();
    assert_eq!(bytes, [1, 2, 3, 4]);
    assert_eq!(state.remaining, 4);
    ACTIVE_WALK.with(|active| active.set(ptr::from_mut(&mut state).cast()));
    let active = ActiveWalk;
    let mut read = 0;
    assert!(
        unsafe { read_memory(process, address, bytes.as_mut_ptr().cast(), 4, &mut read) }.as_bool()
    );
    assert_eq!(read, 4);
    assert_eq!(state.remaining, 0);
    drop(active);
    assert_eq!(
        state.read_bytes(address, &mut bytes).unwrap_err().reason,
        "snapshot_read_out_of_bounds"
    );
    assert_eq!(state.remaining, 0);
}

#[test]
fn empty_module_map_never_authorizes_instruction_or_read() {
    let prepared = prepare_modules(&[]).unwrap();
    assert!(prepared.module(0x12340000).is_none());
    let frame = prepared.frame(0x12340000, 0x56780000);
    assert!(frame.module_index.is_none());
    assert!(frame.module_offset.is_none());
}
