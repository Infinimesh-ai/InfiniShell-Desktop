use super::*;

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// 只有两个不重叠段、一个名称导入；不会打开文件或调用 Windows API。
fn root_bytes() -> Vec<u8> {
    let mut bytes = vec![0; 0xa00];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 60, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put16(&mut bytes, 0x84, 0x8664);
    put16(&mut bytes, 0x86, 2);
    put16(&mut bytes, 0x94, 0xf0);
    put16(&mut bytes, 0x96, 2);
    put16(&mut bytes, 0x98, 0x20b);
    put32(&mut bytes, 0xd0, 0x3000);
    put32(&mut bytes, 0xd4, 0x400);
    put32(&mut bytes, 0x104, 16);
    put32(&mut bytes, 0x110, 0x2000);
    put32(&mut bytes, 0x114, 40);
    put32(&mut bytes, 0x190, 0x200);
    put32(&mut bytes, 0x194, 0x1000);
    put32(&mut bytes, 0x198, 0x200);
    put32(&mut bytes, 0x19c, 0x400);
    put32(&mut bytes, 0x1ac, 0x6000_0020);
    put32(&mut bytes, 0x1b8, 0x400);
    put32(&mut bytes, 0x1bc, 0x2000);
    put32(&mut bytes, 0x1c0, 0x400);
    put32(&mut bytes, 0x1c4, 0x600);
    put32(&mut bytes, 0x1d4, 0xc000_0040);
    put32(&mut bytes, 0x600, 0x2040);
    put32(&mut bytes, 0x60c, 0x2080);
    put32(&mut bytes, 0x610, 0x2060);
    put64(&mut bytes, 0x640, 0x20a0);
    put64(&mut bytes, 0x660, 0x20a0);
    bytes[0x680..0x68d].copy_from_slice(b"KERNEL32.dll\0");
    bytes[0x6a2..0x6b1].copy_from_slice(b"CreateProcessW\0");
    bytes
}

fn root(bytes: Vec<u8>) -> Image {
    Image::parse(bytes, 0x1_4000_0000, 0x3000, false).expect("有效的固定 PE 夹具")
}
fn assert_reason<T>(result: Result<T>, reason: &'static str) {
    assert_eq!(result.err().expect("必须拒绝").reason, reason);
}
fn vacant_registers() -> DebugRegisters {
    DebugRegisters {
        address: [0; 4],
        status: 0xffff_0ff0,
        control: 0x400,
    }
}
fn creator_entry() -> Entry {
    Entry {
        api: Api::CreateProcessW,
        address: 0x7ff0_1234_1000,
        iat_rvas: vec![0x2060],
    }
}

#[test]
fn named_import_binds_the_exact_root_iat_slot() {
    let image = root(root_bytes());

    assert_eq!(
        image.imports().unwrap(),
        vec![Import {
            api: Api::CreateProcessW,
            iat_rva: 0x2060
        }]
    );
    assert!(checked_iat_slot(&image, &creator_entry(), 0x1_4000_2060).is_ok());
    assert_reason(
        checked_iat_slot(&image, &creator_entry(), 0x1_4000_2068),
        "return_call_uses_another_iat_slot",
    );
}

#[test]
fn root_pe_rejects_changed_machine_size_and_header_aliases() {
    let mut machine = root_bytes();
    put16(&mut machine, 0x84, 0x14c);
    assert_reason(
        Image::parse(machine, 0x1_4000_0000, 0x3000, false),
        "not_x64_pe",
    );
    assert_reason(
        Image::parse(root_bytes(), 0x1_4000_0000, 0x4000, false),
        "image_layout_mismatch",
    );
    let mut headers = root_bytes();
    put32(&mut headers, 0x19c, 0x200);
    assert_reason(
        Image::parse(headers, 0x1_4000_0000, 0x3000, false),
        "ambiguous_pe_sections",
    );
}

#[test]
fn root_pe_rejects_overlapping_virtual_or_file_sections() {
    let mut virtual_overlap = root_bytes();
    put32(&mut virtual_overlap, 0x1bc, 0x1100);
    assert_reason(
        Image::parse(virtual_overlap, 0x1_4000_0000, 0x3000, false),
        "ambiguous_pe_sections",
    );
    let mut file_overlap = root_bytes();
    put32(&mut file_overlap, 0x1c4, 0x500);
    assert_reason(
        Image::parse(file_overlap, 0x1_4000_0000, 0x3000, false),
        "ambiguous_pe_sections",
    );
}

#[test]
fn incomplete_import_names_never_claim_creator_coverage() {
    let mut ordinal = root_bytes();
    put64(&mut ordinal, 0x640, 0x8000_0000_0000_0001);
    assert_reason(
        root(ordinal).imports(),
        "ordinal_or_large_import_unsupported",
    );
    let mut bound = root_bytes();
    put32(&mut bound, 0x600, 0);
    assert_reason(root(bound).imports(), "unbound_import_names_required");
    let mut delayed = root_bytes();
    put32(&mut delayed, 0x170, 0x2100);
    put32(&mut delayed, 0x174, 32);
    assert_reason(root(delayed).imports(), "delay_import_coverage_unsupported");
}

#[test]
fn unterminated_import_directory_is_not_a_complete_binding() {
    let mut bytes = root_bytes();
    put32(&mut bytes, 0x114, 20);
    assert_reason(root(bytes).imports(), "unterminated_import_directory");
}

#[test]
fn instruction_ranges_exclude_writable_code_and_virtual_padding() {
    let mut bytes = root_bytes();
    put32(&mut bytes, 0x190, 0x300);
    let image = root(bytes);
    assert_reason(image.bytes_at(0x1200, 1), "rva_outside_file_section");
    assert!(!image.executable(0x1_4000_2000, 1));
    let mut writable = root_bytes();
    put32(&mut writable, 0x1ac, 0xe000_0020);
    assert!(!root(writable).executable(0x1_4000_1000, 1));
}

#[test]
fn repeated_named_import_requires_one_resolved_entry() {
    let mut entries = Vec::new();
    combine_entry(
        &mut entries,
        Import {
            api: Api::CreateProcessW,
            iat_rva: 0x2060,
        },
        0x1000,
    )
    .unwrap();
    combine_entry(
        &mut entries,
        Import {
            api: Api::CreateProcessW,
            iat_rva: 0x2070,
        },
        0x1000,
    )
    .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].iat_rvas, vec![0x2060, 0x2070]);
    assert_reason(
        combine_entry(
            &mut entries,
            Import {
                api: Api::CreateProcessW,
                iat_rva: 0x2080,
            },
            0x2000,
        ),
        "same_api_multiple_addresses",
    );
    assert_reason(
        combine_entry(
            &mut entries,
            Import {
                api: Api::CreateProcessA,
                iat_rva: 0x2080,
            },
            0x1000,
        ),
        "ambiguous_api_address",
    );
}

#[test]
fn fourth_distinct_entry_cannot_steal_the_return_slot() {
    let mut entries = Vec::new();
    combine_entry(
        &mut entries,
        Import {
            api: Api::CreateProcessW,
            iat_rva: 0x2060,
        },
        0x1000,
    )
    .unwrap();
    combine_entry(
        &mut entries,
        Import {
            api: Api::CreateProcessA,
            iat_rva: 0x2068,
        },
        0x2000,
    )
    .unwrap();
    combine_entry(
        &mut entries,
        Import {
            api: Api::CreateProcessAsUserW,
            iat_rva: 0x2070,
        },
        0x3000,
    )
    .unwrap();
    assert_reason(
        combine_entry(
            &mut entries,
            Import {
                api: Api::CreateProcessAsUserA,
                iat_rva: 0x2078,
            },
            0x4000,
        ),
        "more_than_three_creator_entries",
    );
}

#[test]
fn vacant_debug_state_rejects_disabled_addresses_and_foreign_traps() {
    let original = vacant_registers();
    assert!(original.vacant(0x202));
    assert!(
        !DebugRegisters {
            address: [1, 0, 0, 0],
            ..original
        }
        .vacant(0x202)
    );
    assert!(
        !DebugRegisters {
            control: 0x2400,
            ..original
        }
        .vacant(0x202)
    );
    assert!(
        !DebugRegisters {
            status: 0xffff_4ff0,
            ..original
        }
        .vacant(0x202)
    );
    assert!(!original.vacant(0x302));
}

#[test]
fn owned_step_requires_exact_slot_address_and_no_other_debug_cause() {
    let expected = vacant_registers().entries(&[creator_entry()]);
    let mut context = CONTEXT {
        Rip: 0x7ff0_1234_1000,
        EFlags: 0x202,
        ..Default::default()
    };
    expected.write(&mut context);
    context.Dr6 = 0xffff_0ff1;
    assert_eq!(owned_slot(expected, &context, 0x7ff0_1234_1000), Some(0));
    assert_eq!(owned_slot(expected, &context, 0x7ff0_1234_1001), None);
    context.Dr6 = 0xffff_0ff3;
    assert_eq!(owned_slot(expected, &context, 0x7ff0_1234_1000), None);
    context.Dr6 = 0xffff_4ff1;
    assert_eq!(owned_slot(expected, &context, 0x7ff0_1234_1000), None);
    context.Dr6 = 0xffff_0ff1;
    context.Dr7 |= 2;
    assert_eq!(owned_slot(expected, &context, 0x7ff0_1234_1000), None);
}

#[test]
fn return_pair_requires_original_stack_frame_after_ret() {
    let pending = PendingCall {
        api: Api::CreateProcessW,
        stack: 0x2008,
        return_address: 0x1_4000_1006,
        information: 0x3000,
    };
    let mut context = CONTEXT {
        Rip: 0x1_4000_1006,
        Rsp: 0x2010,
        ..Default::default()
    };
    assert!(matching_return(&pending, &context));
    context.Rsp = 0x2008;
    assert!(!matching_return(&pending, &context));
    context.Rsp = 0x2010;
    context.Rip = 0x1_4000_1007;
    assert!(!matching_return(&pending, &context));
}

#[test]
fn uncertain_set_only_allows_known_complete_register_configurations() {
    let original = vacant_registers();
    let entry = original.entries(&[creator_entry()]);
    let returned = original.returned(0x1_4000_1006);
    assert!(restorable(entry, original, Some(returned), Some(entry)));
    assert!(restorable(returned, original, Some(returned), Some(entry)));
    assert!(!restorable(
        DebugRegisters {
            address: [0x7ff0_1234_1000, 0, 0, 0x1_4000_1006],
            ..returned
        },
        original,
        Some(returned),
        Some(entry)
    ));
    assert!(!restorable(
        DebugRegisters {
            status: 0xffff_4ff0,
            ..entry
        },
        original,
        Some(returned),
        Some(entry)
    ));
}

#[test]
fn call_displacement_is_signed_and_cannot_wrap_into_another_range() {
    assert_eq!(
        relative_target(0x1006, &[0xfa, 0x0f, 0, 0]).unwrap(),
        0x2000
    );
    assert_eq!(
        relative_target(0x2006, &[0xfa, 0xef, 0xff, 0xff]).unwrap(),
        0x1000
    );
    assert_reason(
        relative_target(1, &[0xfe, 0xff, 0xff, 0xff]),
        "relative_target_overflow",
    );
    assert_reason(
        relative_target(MAX_USER, &[1, 0, 0, 0]),
        "relative_target_overflow",
    );
}

#[test]
fn only_pre_mutation_coverage_limits_can_be_unavailable() {
    for reason in [
        "delay_import_coverage_unsupported",
        "no_supported_creator_import",
        "more_than_three_creator_entries",
        "creator_module_limit",
        "existing_debug_configuration_rejected",
    ] {
        assert!(Failure::invalid(reason).coverage_unavailable());
    }
    for reason in [
        "ordinal_or_large_import_unsupported",
        "unbound_import_names_required",
        "original_object_identity_mismatch",
        "iat_target_not_in_verified_module",
        "debug_register_ownership_lost",
        "indirect_call_shape_unsupported",
        "api_failed",
    ] {
        assert!(!Failure::invalid(reason).coverage_unavailable());
    }
    let mut wrapped = Failure::invalid("no_supported_creator_import");
    wrapped.hresult = Some(-2147024891);
    assert!(!wrapped.coverage_unavailable());
}
