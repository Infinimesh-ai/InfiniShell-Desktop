use super::*;
use windows::Win32::System::Memory::{
    MEM_RELEASE, MEM_RESERVE, VirtualAlloc, VirtualFree, VirtualProtect,
};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

#[test]
fn exit_confirmation_preserves_original_wait_failure_code() {
    let denied = require_signaled(Err(io::Error::from_raw_os_error(5))).unwrap_err();
    assert_eq!(denied.raw_os_error(), Some(5));
    let invalid_handle = require_signaled(Err(io::Error::from_raw_os_error(6))).unwrap_err();
    assert_eq!(invalid_handle.raw_os_error(), Some(6));
}

#[test]
fn nonsignaled_object_is_not_an_exit_or_an_os_failure() {
    let error = require_signaled(Ok(WAIT_TIMEOUT)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(error.raw_os_error(), None);
    assert!(require_signaled(Ok(WAIT_OBJECT_0)).is_ok());
}

#[test]
fn unexpected_wait_status_cannot_confirm_process_or_thread_exit() {
    let error = require_signaled(Ok(WAIT_EVENT(128))).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "原对象等待返回意外状态 128");
}

fn empty_registers() -> Registers {
    Registers {
        address: [0; 4],
        status: 0xffff_0ff0,
        control: 1 << 10,
    }
}

#[test]
fn managed_slot_survives_classification_entry_return_and_post_budget_completion() {
    let original = empty_registers();
    let entry = original.combined(Some((0x12000, false)), Some(0x24000));
    assert_eq!(entry.address, [0x12000, 0x24000, 0, 0]);
    assert_eq!(entry.control, 0x405);
    let returning = original.combined(Some((0x36000, true)), Some(0x24000));
    assert_eq!(returning.address, [0, 0x24000, 0, 0x36000]);
    assert_eq!(returning.control, 0x444);
    let continuation_only = original.combined(None, Some(0x24000));
    assert_eq!(continuation_only.address, [0, 0x24000, 0, 0]);
    assert_eq!(continuation_only.control, 0x404);
    assert_eq!(
        original.combined(Some((0x12000, false)), None),
        original.armed(0x12000, false)
    );
    assert_eq!(original.combined(None, None), original);
}

#[test]
fn managed_slot_requires_a_single_owned_hit_and_exact_combined_configuration() {
    let expected = empty_registers().combined(Some((0x12000, false)), Some(0x24000));
    let mut current = CONTEXT {
        Rip: 0x24000,
        EFlags: 0x202,
        ..CONTEXT::default()
    };
    expected.write(&mut current);
    current.Dr6 |= 2;
    assert_eq!(owned_slot(expected, &current, 0x24000), Some(1));
    assert!(restorable(
        Registers::read(&current),
        empty_registers(),
        Some(expected),
        None
    ));
    current.Dr6 |= 1;
    assert_eq!(owned_slot(expected, &current, 0x24000), None);
    current.Dr6 &= !1;
    current.Dr1 += 1;
    assert_eq!(owned_slot(expected, &current, 0x24000), None);
    current.Dr1 -= 1;
    current.Dr7 |= 8;
    assert_eq!(owned_slot(expected, &current, 0x24000), None);
}

fn continuation_test_target() -> ManagedContinuationTarget {
    let spec = super::super::clr_reader::ClrManagedMethodSpec {
        module_mvid: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee".into(),
        method_token: 0x06000001,
        approved_il_offsets: vec![20],
        bool_local_index: 0,
        start_info_local_index: None,
    };
    ManagedContinuationTarget {
        process_id: 1,
        thread_id: 2,
        process_birth: 3,
        thread_birth: 4,
        pre_sequence: 4,
        pre_nonce: [1; 16],
        module_mvid: spec.module_mvid.clone(),
        method_token: spec.method_token,
        il_offset: 20,
        enc_version: 1,
        frame_rsp: 0x40000,
        extent_start: 0x20000,
        extent_end: 0x20100,
        address: 0x20020,
        map_count: 3,
        map_sha256: [2; 32],
        code_sha256: [3; 32],
        spec,
    }
}

fn continuation_test_pair() -> ManagedContinuationPair {
    let continuation = continuation_test_target();
    let mut initial = continuation.clone();
    initial.il_offset = 10;
    initial.address -= 8;
    initial.spec.approved_il_offsets = vec![10];
    ManagedContinuationPair {
        initial,
        continuation,
    }
}

#[test]
fn pair_lease_rejects_mixed_frames_requests_or_code_maps() {
    let pair = continuation_test_pair();
    let identity = continuation_identity(&pair.initial);
    assert!(continuation_pair_matches(&pair, identity, 4));
    assert!(!continuation_pair_matches(&pair, identity, 5));
    let mut variants = Vec::new();
    let mut changed = pair.clone();
    changed.initial.frame_rsp += 8;
    variants.push(changed);
    let mut changed = pair.clone();
    changed.initial.pre_nonce[0] += 1;
    variants.push(changed);
    let mut changed = pair.clone();
    changed.initial.enc_version += 1;
    variants.push(changed);
    let mut changed = pair.clone();
    changed.initial.map_sha256[0] += 1;
    variants.push(changed);
    let mut changed = pair.clone();
    changed.initial.code_sha256[0] += 1;
    variants.push(changed);
    let mut changed = pair.clone();
    changed.initial.thread_birth += 1;
    variants.push(changed);
    for changed in variants {
        assert!(!continuation_pair_matches(&changed, identity, 4));
    }
}

#[test]
fn pair_lease_cannot_alias_initial_and_later_points() {
    let mut pair = continuation_test_pair();
    let identity = continuation_identity(&pair.initial);
    pair.initial.address = pair.continuation.address;
    assert!(!continuation_pair_matches(&pair, identity, 4));
    let mut pair = continuation_test_pair();
    pair.initial
        .spec
        .approved_il_offsets
        .push(pair.continuation.il_offset);
    assert!(!continuation_pair_matches(&pair, identity, 4));
    pair.initial = pair.continuation.clone();
    assert!(!continuation_pair_matches(&pair, identity, 4));
}

#[test]
fn managed_target_rejects_changed_pre_identity_sequence_and_unapproved_mapping() {
    let target = continuation_test_target();
    let identity = continuation_identity(&target);
    assert!(continuation_binding_matches(&target, identity, 4));
    assert!(!continuation_binding_matches(&target, identity, 5));
    assert!(!continuation_binding_matches(
        &target,
        ObjectIdentity {
            thread_birth: 5,
            ..identity
        },
        4
    ));
    for changed in [
        ManagedContinuationTarget {
            pre_nonce: [0; 16],
            ..target.clone()
        },
        ManagedContinuationTarget {
            module_mvid: "other".into(),
            ..target.clone()
        },
        ManagedContinuationTarget {
            method_token: 0x06000002,
            ..target.clone()
        },
        ManagedContinuationTarget {
            il_offset: 21,
            ..target.clone()
        },
        ManagedContinuationTarget {
            map_count: 257,
            ..target.clone()
        },
        ManagedContinuationTarget {
            map_sha256: [0; 32],
            ..target.clone()
        },
        ManagedContinuationTarget {
            code_sha256: [0; 32],
            ..target.clone()
        },
        ManagedContinuationTarget {
            extent_end: target.extent_start,
            ..target.clone()
        },
        ManagedContinuationTarget {
            extent_end: target.extent_start + 65537,
            ..target.clone()
        },
        ManagedContinuationTarget {
            address: target.extent_end,
            ..target.clone()
        },
        ManagedContinuationTarget {
            frame_rsp: 0x40001,
            ..target.clone()
        },
    ] {
        assert!(!continuation_binding_matches(&changed, identity, 4));
    }
}

#[test]
fn managed_hit_requires_node_order_and_the_same_original_frame() {
    let target = continuation_test_target();
    let identity = continuation_identity(&target);
    let current = CONTEXT {
        Rip: target.address,
        Rsp: target.frame_rsp,
        ..CONTEXT::default()
    };
    assert!(continuation_hit_matches(
        &target,
        identity,
        Some((5, 6, 7)),
        8,
        &current
    ));
    for node in [
        None,
        Some((0, 6, 7)),
        Some((4, 6, 7)),
        Some((8, 6, 7)),
        Some((5, 0, 7)),
        Some((5, 6, 0)),
    ] {
        assert!(!continuation_hit_matches(
            &target, identity, node, 8, &current
        ));
    }
    let other_frame = CONTEXT {
        Rsp: current.Rsp + 8,
        ..current
    };
    assert!(!continuation_hit_matches(
        &target,
        identity,
        Some((5, 6, 7)),
        8,
        &other_frame
    ));
    let other_ip = CONTEXT {
        Rip: current.Rip + 1,
        ..current
    };
    assert!(!continuation_hit_matches(
        &target,
        identity,
        Some((5, 6, 7)),
        8,
        &other_ip
    ));
}

#[inline(never)]
fn continuation_code_fixture() -> u64 {
    0x1234
}

#[test]
fn managed_extent_is_rechecked_against_live_executable_pages_and_full_code_hash() {
    let process = unsafe { GetCurrentProcess() };
    let address = continuation_code_fixture as *const () as u64;
    let mut target = ManagedContinuationTarget {
        extent_start: address,
        extent_end: address + 16,
        address,
        ..continuation_test_target()
    };
    target.code_sha256 = Sha256::digest(memory(process, address, 16).unwrap()).into();
    let regions = continuation_regions(process, &target).unwrap();
    let mut lease = ManagedContinuationLease { target, regions };
    assert!(lease.verify(process).is_ok());
    lease.target.code_sha256[31] ^= 1;
    assert!(lease.verify(process).is_err());
    lease.target.code_sha256[31] ^= 1;
    lease.regions[0].protect ^= PAGE_GUARD.0;
    assert!(lease.verify(process).is_err());
}

#[test]
fn managed_extent_ignores_only_growth_beyond_the_method_and_rejects_internal_changes() {
    let mut info = SYSTEM_INFO::default();
    unsafe {
        GetSystemInfo(&mut info);
    }
    let page = info.dwPageSize as usize;
    let allocation = unsafe { VirtualAlloc(None, page * 2, MEM_RESERVE, PAGE_EXECUTE_READWRITE) };
    assert!(!allocation.is_null());
    let result = (|| -> io::Result<()> {
        let first =
            unsafe { VirtualAlloc(Some(allocation), page, MEM_COMMIT, PAGE_EXECUTE_READWRITE) };
        require(first == allocation, "测试首个执行页提交失败")?;
        let process = unsafe { GetCurrentProcess() };
        // 方法停在首个页末尾前八字节；其后内容不应进入样本或映射合同。
        let start = allocation as u64 + page as u64 - 16;
        let mut target = ManagedContinuationTarget {
            extent_start: start,
            extent_end: start + 8,
            address: start,
            ..continuation_test_target()
        };
        target.code_sha256 = Sha256::digest(memory(process, start, 8)?).into();
        let before = return_region(process, start)?;
        let regions = continuation_regions(process, &target)?;
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].base, start);
        assert_eq!(regions[0].length, 8);
        assert_eq!(regions[0].bytes.len(), 8);
        let lease = ManagedContinuationLease { target, regions };
        let second = unsafe {
            VirtualAlloc(
                Some((allocation as usize + page) as *const c_void),
                page,
                MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        };
        require(!second.is_null(), "测试相邻执行页提交失败")?;
        let after = return_region(process, start)?;
        assert_eq!(before.length + page as u64, after.length);
        assert!(lease.verify(process).is_ok());
        unsafe {
            *((start + 8) as *mut u8) = 1;
        }
        assert!(lease.verify(process).is_ok());
        unsafe {
            *(start as *mut u8) = 1;
        }
        assert!(lease.verify(process).is_err());
        unsafe {
            *(start as *mut u8) = 0;
        }
        let mut old = PAGE_EXECUTE_READWRITE;
        unsafe { VirtualProtect(allocation, page, PAGE_EXECUTE_READ, &mut old) }
            .map_err(io::Error::from)?;
        assert!(lease.verify(process).is_err());
        Ok(())
    })();
    unsafe { VirtualFree(allocation, 0, MEM_RELEASE) }.unwrap();
    result.unwrap();
}

#[test]
fn initially_zero_debug_view_arms_the_architectural_inactive_status() {
    let original = Registers {
        address: [0; 4],
        status: 0,
        control: 0,
    };
    let armed = original.armed(0x12340, false);
    assert_eq!(armed.status, 0xffff_0ff0);
    assert_eq!(armed.control, 0x401);
    let mut hit = CONTEXT {
        Rip: 0x12340,
        EFlags: 0x202,
        ..CONTEXT::default()
    };
    armed.write(&mut hit);
    hit.Dr6 = 0xffff_0ff1;
    assert_eq!(owned_slot(armed, &hit, 0x12340), Some(0));
    assert!(restorable(
        Registers::read(&hit),
        original,
        Some(armed),
        None
    ));
    original.write(&mut hit);
    assert_eq!(Registers::read(&hit), original);
}

#[test]
fn architectural_status_does_not_hide_another_debug_cause_or_configuration() {
    let armed = empty_registers().armed(0x12340, false);
    let mut hit = CONTEXT {
        Rip: 0x12340,
        EFlags: 0x202,
        ..CONTEXT::default()
    };
    armed.write(&mut hit);
    hit.Dr6 = 0xffff_2ff1;
    assert_eq!(owned_slot(armed, &hit, 0x12340), None);
    hit.Dr6 = 0xffff_07f1;
    assert_eq!(owned_slot(armed, &hit, 0x12340), None);
    hit.Dr6 = 0xfffe_0ff1;
    assert_eq!(owned_slot(armed, &hit, 0x12340), None);
    hit.Dr6 = 0xffff_0ff1;
    hit.Dr7 |= 2;
    assert_eq!(owned_slot(armed, &hit, 0x12340), None);
}

#[test]
fn inactive_api_view_accepts_only_the_complete_known_encoding() {
    let armed = empty_registers().armed(0x12340, false);
    let api = Registers {
        address: [0x12340, 0, 0, 0],
        status: 0,
        control: 1,
    };
    assert_eq!(armed.inactive_api_view(), api);
    assert!(restorable(api, empty_registers(), Some(armed), None));
    let active_bus_lock = Registers {
        status: 0xffff_07f0,
        ..empty_registers()
    };
    let active_transaction = Registers {
        status: 0xfffe_0ff0,
        ..empty_registers()
    };
    assert!(!active_bus_lock.vacant(0));
    assert!(!active_transaction.vacant(0));
    assert_eq!(active_bus_lock.inactive_api_view(), active_bus_lock);
    assert_eq!(active_transaction.inactive_api_view(), active_transaction);
    assert!(!restorable(
        Registers {
            control: 0x401,
            ..api
        },
        empty_registers(),
        Some(armed),
        None
    ));
}

#[test]
fn exact_utf16_node_requires_case_units_and_single_terminator() {
    let expected: Vec<_> = "Q:\\runtime\\node.exe".encode_utf16().collect();
    let mut actual = expected.clone();
    actual.push(0);
    assert!(exact_node(&expected, &actual));
    actual[0] = u16::from(b'q');
    assert!(!exact_node(&expected, &actual));
    actual[0] = expected[0];
    actual.push(0);
    assert!(!exact_node(&expected, &actual));
    assert!(!exact_node(&expected, &expected));
}

#[test]
fn path_comparison_distinguishes_unreadable_memory_from_observed_mismatch() {
    let expected = [81_u16, 58, 92];
    let mismatched = [113_u16, 58, 92, 0];
    let process = unsafe { GetCurrentProcess() };
    assert_eq!(
        compare_node_path(process, 0, &expected),
        PathComparison::Unreadable
    );
    assert_eq!(
        compare_node_path(process, u64::MAX, &expected),
        PathComparison::Unreadable
    );
    assert_eq!(
        compare_node_path(process, mismatched.as_ptr() as u64, &expected),
        PathComparison::Mismatched
    );
}

#[test]
fn path_comparison_requires_exact_units_and_reads_only_through_expected_terminator() {
    let expected = [81_u16, 58, 92];
    let terminated = [81_u16, 58, 92, 0, 1234];
    let extended = [81_u16, 58, 92, 100, 0];
    let shortened = [81_u16, 58, 0, 0];
    let process = unsafe { GetCurrentProcess() };
    assert_eq!(
        compare_node_path(process, terminated.as_ptr() as u64, &expected),
        PathComparison::Matched
    );
    assert_eq!(
        compare_node_path(process, extended.as_ptr() as u64, &expected),
        PathComparison::Mismatched
    );
    assert_eq!(
        compare_node_path(process, shortened.as_ptr() as u64, &expected),
        PathComparison::Mismatched
    );
}

#[test]
fn skipped_receipt_serialization_keeps_unknown_states_without_path_or_address() {
    let receipt = SkippedEntryReceipt {
        generation: [1; 16],
        identity: ObjectIdentity {
            process_id: 1,
            thread_id: 2,
            process_birth: 3,
            thread_birth: 4,
        },
        sequence: 7,
        node_create_sequence: None,
        flags: 0x2000,
        path_comparison: PathComparison::NotChecked,
    };
    let mut value = serde_json::to_value(&receipt).unwrap();
    assert_eq!(value["node_create_sequence"], serde_json::Value::Null);
    assert_eq!(value["path_comparison"], "not_checked");
    for field in [
        "generation",
        "identity",
        "sequence",
        "node_create_sequence",
        "flags",
        "path_comparison",
    ] {
        assert!(value.as_object_mut().unwrap().remove(field).is_some());
    }
    assert_eq!(value, serde_json::json!({}));
    let unreadable = SkippedEntryReceipt {
        node_create_sequence: Some(5),
        path_comparison: PathComparison::Unreadable,
        ..receipt
    };
    let value = serde_json::to_value(unreadable).unwrap();
    assert_eq!(value["node_create_sequence"], 5);
    assert_eq!(value["path_comparison"], "unreadable");
}

#[test]
fn existing_dr_or_single_step_configuration_is_never_vacant() {
    let original = empty_registers();
    assert!(original.vacant(0));
    assert!(!original.vacant(TF));
    for slot in 0..4 {
        let mut value = original;
        value.address[slot] = 0x10000;
        assert!(!value.vacant(0));
        value = original;
        value.control |= 1 << (slot * 2);
        assert!(!value.vacant(0));
        value = original;
        value.status |= 1 << slot;
        assert!(!value.vacant(0));
    }
}

#[test]
fn owned_hit_requires_exact_configuration_address_and_single_status_bit() {
    let armed = empty_registers().armed(0x12340, false);
    let mut value = CONTEXT {
        Rip: 0x12340,
        ..CONTEXT::default()
    };
    armed.write(&mut value);
    value.Dr6 |= 1;
    assert_eq!(owned_slot(armed, &value, value.Rip), Some(0));
    assert_eq!(owned_slot(armed, &value, value.Rip + 1), None);
    value.Dr6 |= 2;
    assert_eq!(owned_slot(armed, &value, value.Rip), None);
    value.Dr6 &= !2;
    value.Dr7 |= 1 << 16;
    assert_eq!(owned_slot(armed, &value, value.Rip), None);
}

#[test]
fn restoration_accepts_only_original_or_complete_owned_configurations() {
    let original = empty_registers();
    let entry = original.armed(0x10000, false);
    let returned = original.armed(0x20000, true);
    assert!(restorable(original, original, Some(returned), Some(entry)));
    assert!(restorable(entry, original, Some(returned), Some(entry)));
    assert!(restorable(returned, original, Some(returned), Some(entry)));
    let mut foreign = returned;
    foreign.address[1] = 0x30000;
    assert!(!restorable(foreign, original, Some(returned), Some(entry)));
    foreign = returned;
    foreign.status |= 1 << 14;
    assert!(!restorable(foreign, original, Some(returned), Some(entry)));
}

#[test]
fn return_pair_requires_exact_stack_and_preserves_register_scalar() {
    let pending = Pending {
        stack: 0x100008,
        returned_to: 0x200000,
        region: ReturnRegion {
            allocation: 0x200000,
            base: 0x200000,
            length: 4096,
            kind: MEM_PRIVATE.0,
            protect: PAGE_EXECUTE_READ.0,
            bytes: vec![0x90],
        },
        sequence: 1,
        nonce: [1; 16],
        phase: CallPhase::PreNode,
    };
    let mut value = CONTEXT {
        Rip: pending.returned_to,
        Rsp: pending.stack + 8,
        Rax: 0x100001234,
        ..CONTEXT::default()
    };
    assert!(matching_return(&pending, &value));
    assert!(return_phase_matches(&pending, None, 2));
    assert!(!return_phase_matches(&pending, None, 1));
    assert!(!return_phase_matches(&pending, Some((2, 5, 6)), 3));
    let original = value;
    empty_registers().write(&mut value);
    assert!(execution_equal(&original, &value));
    value.Rax += 1;
    assert!(!execution_equal(&original, &value));
    value.Rsp += 8;
    assert!(!matching_return(&pending, &value));
    let post = Pending {
        sequence: 4,
        phase: CallPhase::PostNode {
            node_create_sequence: 3,
        },
        ..pending
    };
    assert!(return_phase_matches(&post, Some((3, 5, 6)), 5));
    assert!(!return_phase_matches(&post, None, 5));
    assert!(!return_phase_matches(&post, Some((2, 5, 6)), 5));
    assert!(!return_phase_matches(&post, Some((3, 5, 6)), 4));
}

#[test]
fn pre_node_budget_cannot_consume_or_relax_the_post_node_budget() {
    assert_eq!(
        select_phase(None, 1, 0x2000, 0, 0),
        Some(CallPhase::PreNode)
    );
    assert_eq!(select_phase(None, 2, 0x2000, 1, 0), None);
    assert_eq!(
        select_phase(Some((3, 5, 6)), 4, 0x2000, 1, 0),
        Some(CallPhase::PostNode {
            node_create_sequence: 3,
        })
    );
    assert_eq!(select_phase(Some((3, 5, 6)), 4, 0x2000, 0, 1), None);
    for sequence in [0, 2, 3] {
        assert_eq!(select_phase(Some((3, 5, 6)), sequence, 0x2000, 0, 0), None);
    }
    assert_eq!(select_phase(None, 0, 0x2000, 0, 0), None);
    assert_eq!(select_phase(Some((0, 5, 6)), 1, 0x2000, 0, 0), None);
    assert_eq!(select_phase(None, 1, 0, 0, 0), None);
    assert_eq!(select_phase(Some((3, 5, 6)), 4, 0x2001, 0, 0), None);
}

#[test]
fn pre_node_receipt_preserves_full_return_without_inventing_a_node_identity() {
    let receipt = PreNodeReturnReceipt {
        generation: [1; 16],
        identity: ObjectIdentity {
            process_id: 1,
            thread_id: 2,
            process_birth: 3,
            thread_birth: 4,
        },
        pair_nonce: nonce([1; 16], 5),
        entry_sequence: 5,
        return_sequence: 6,
        expected_node_matched: true,
        flags: 0x2000,
        raw_return_u64: 0x1234_5678_0000_4550,
        raw_return_low32: 0x4550,
        return_region_kind: "private",
        registers_restored: true,
        execution_context_unchanged: true,
        pre_node_clr_stack_required: true,
    };
    let output = serde_json::to_value(receipt).unwrap();
    assert_eq!(
        output["raw_return_u64"].as_u64(),
        Some(0x1234_5678_0000_4550)
    );
    assert_eq!(output["raw_return_low32"], 0x4550);
    assert_eq!(output["pre_node_clr_stack_required"], true);
    for field in [
        "node_create_sequence",
        "node_process_id",
        "node_process_birth",
        "post_start_clr_stack_required",
        "return_address",
    ] {
        assert!(output.get(field).is_none());
    }
}

fn resume_test_event() -> (DEBUG_EVENT, PreNodeResume) {
    let mut event = DEBUG_EVENT {
        dwDebugEventCode: EXCEPTION_DEBUG_EVENT,
        dwProcessId: 1,
        dwThreadId: 2,
        ..DEBUG_EVENT::default()
    };
    event.u.Exception.dwFirstChance = 1;
    event.u.Exception.ExceptionRecord.ExceptionCode = EXCEPTION_SINGLE_STEP;
    event.u.Exception.ExceptionRecord.ExceptionAddress = 0x20000 as *mut c_void;
    let ticket = PreNodeResume {
        sequence: 4,
        identity: ObjectIdentity {
            process_id: 1,
            thread_id: 2,
            process_birth: 3,
            thread_birth: 4,
        },
        context: CONTEXT {
            Rip: 0x20000,
            ..CONTEXT::default()
        },
    };
    (event, ticket)
}

#[test]
fn pre_node_resume_rejects_another_thread_event_sequence_or_exception() {
    let (event, ticket) = resume_test_event();
    assert!(resume_event_matches(&event, 4, &ticket));
    assert!(!resume_event_matches(&event, 5, &ticket));
    let mut changed = event;
    changed.dwProcessId += 1;
    assert!(!resume_event_matches(&changed, 4, &ticket));
    changed = event;
    changed.dwThreadId += 1;
    assert!(!resume_event_matches(&changed, 4, &ticket));
    changed = event;
    changed.dwDebugEventCode = EXIT_THREAD_DEBUG_EVENT;
    assert!(!resume_event_matches(&changed, 4, &ticket));
    changed = event;
    changed.u.Exception.dwFirstChance = 0;
    assert!(!resume_event_matches(&changed, 4, &ticket));
    changed = event;
    changed.u.Exception.ExceptionRecord.ExceptionCode = windows::Win32::Foundation::NTSTATUS(1);
    assert!(!resume_event_matches(&changed, 4, &ticket));
    changed = event;
    changed.u.Exception.ExceptionRecord.ExceptionAddress = 0x20001 as *mut c_void;
    assert!(!resume_event_matches(&changed, 4, &ticket));
}

fn stopped_resume_witness() -> (ShellClassificationWitness, DEBUG_EVENT) {
    let process = duplicate(
        unsafe { GetCurrentProcess() },
        (PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE).0,
    )
    .unwrap();
    let process_birth = birth(raw(&process), false).unwrap();
    let root_pid = unsafe { GetCurrentProcessId() };
    let (mut event, mut ticket) = resume_test_event();
    event.dwProcessId = root_pid;
    ticket.identity.process_id = root_pid;
    ticket.identity.process_birth = process_birth;
    (
        ShellClassificationWitness {
            process,
            process_birth,
            root_pid,
            debugger_tid: unsafe { GetCurrentThreadId() },
            generation: [1; 16],
            expected_node: vec![1],
            threads: BTreeMap::new(),
            image: None,
            node: None,
            last_sequence: 4,
            selected: 0,
            returned: 0,
            pre_selected: 1,
            pre_returned: 1,
            pre_resume: Some(ticket),
            pre_return_binding: None,
            post_exception_candidate: None,
            post_exception_resume: None,
            post_exception_selected: 0,
            post_exception_resumed: 0,
            post_exception: None,
            pre_exception_resume: None,
            pre_exception_selected: 0,
            pre_exception_resumed: 0,
            pre_exception: None,
            initial: None,
            initial_bound: false,
            initial_selected: 0,
            initial_resumed: 0,
            initial_receipt: None,
            initial_resume: None,
            managed: None,
            managed_bound: false,
            managed_selected: 0,
            managed_resumed: 0,
            managed_invalidated: false,
            managed_receipt: None,
            managed_resume: None,
            post_return_resume: None,
            managed_deferred: None,
            post_node_sample_attempted: false,
            post_node_root_stop: None,
            skipped: 0,
            first_skipped_entry: None,
            restored: 1,
            exited_dirty: 0,
            exited: false,
            stopped: true,
            first_unowned_single_step: None,
            completed_inactive_api_readbacks: 0,
            completed_resume_flag_writes: 0,
            completed_fixed_eflags_api_readbacks: 0,
        },
        event,
    )
}

fn managed_resume_witness() -> (ShellClassificationWitness, DEBUG_EVENT) {
    let (mut witness, mut event) = stopped_resume_witness();
    let pre = witness.pre_resume.take().unwrap();
    let target = ManagedContinuationTarget {
        process_id: pre.identity.process_id,
        process_birth: pre.identity.process_birth,
        ..continuation_test_target()
    };
    event.u.Exception.ExceptionRecord.ExceptionAddress = target.address as *mut c_void;
    witness.pre_return_binding = Some((pre.identity, 4));
    witness.node = Some((5, 6, 7));
    witness.last_sequence = 8;
    witness.managed_bound = true;
    witness.managed_selected = 1;
    witness.managed_receipt = Some(ManagedContinuationReceipt {
        generation: witness.generation,
        identity: pre.identity,
        pre_return_sequence: 4,
        node_create_sequence: 5,
        node_process_id: 6,
        node_process_birth: 7,
        event_sequence: 8,
        first_delivery_sequence: 8,
        deferred_for_node: false,
        registers_restored: true,
        execution_context_unchanged: true,
        managed_continuation_clr_stack_required: true,
        mapping: target.safe_evidence(),
    });
    witness.managed = Some(ManagedContinuationLease {
        target,
        regions: Vec::new(),
    });
    witness.managed_resume = Some(PostNodeExceptionResume {
        stop: OriginalExceptionStop { event, sequence: 8 },
        identity: pre.identity,
        context: pre.context,
    });
    (witness, event)
}

fn initial_resume_witness() -> (ShellClassificationWitness, DEBUG_EVENT) {
    let (mut witness, event) = managed_resume_witness();
    let late = witness.managed_receipt.take().unwrap();
    witness.node = None;
    witness.initial_bound = true;
    witness.initial_selected = 1;
    witness.initial = Some(ManagedContinuationLease {
        target: witness.managed.as_ref().unwrap().target.clone(),
        regions: Vec::new(),
    });
    witness.initial_resume = witness.managed_resume.take();
    witness.managed_selected = 0;
    witness.initial_receipt = Some(ManagedInitialReceipt {
        generation: late.generation,
        identity: late.identity,
        pre_return_sequence: late.pre_return_sequence,
        event_sequence: late.event_sequence,
        registers_restored: true,
        execution_context_unchanged: true,
        managed_initial_clr_stack_required: true,
        mapping: late.mapping,
    });
    (witness, event)
}

#[test]
fn initial_stop_is_readable_without_node_and_never_uses_late_resume_ticket() {
    let (mut witness, event) = initial_resume_witness();
    assert!(witness.managed_initial_target().is_ok());
    assert!(witness.managed_continuation_target().is_err());
    assert!(
        witness
            .resume_after_managed_continuation(&event, 8)
            .is_err()
    );
    assert!(witness.managed_initial_target().is_err());
    assert!(witness.initial.is_none());
    assert!(witness.managed.is_none());
    assert_eq!(witness.initial_resumed, 0);
    assert!(witness.managed_deferred.is_none());
}

#[test]
fn initial_resume_rejects_replayed_event_and_retires_both_points() {
    let (mut witness, event) = initial_resume_witness();
    assert!(witness.resume_after_managed_initial(&event, 9).is_err());
    assert!(witness.resume_after_managed_initial(&event, 8).is_err());
    assert!(witness.managed_invalidated);
    assert_eq!(witness.initial_selected, 1);
    assert_eq!(witness.initial_resumed, 0);
    assert_eq!(witness.managed_selected, 0);
    assert!(witness.managed_deferred.is_none());
}

#[test]
fn initial_lease_is_not_resurrected_by_an_intervening_observation() {
    let (mut witness, mut event) = initial_resume_witness();
    event.dwProcessId += 1;
    assert!(matches!(
        witness.observe(&event, 9).unwrap(),
        Observation::NotOwned
    ));
    assert!(witness.managed_initial_target().is_err());
    assert!(witness.initial.is_none());
    assert!(witness.managed.is_none());
    assert!(witness.managed_invalidated);
}

#[test]
fn initial_pending_stage_does_not_arm_later_point_or_wait_for_node() {
    let (mut witness, _) = initial_resume_witness();
    witness.initial_selected = 0;
    assert!(witness.active_continuation().is_some());
    witness.initial_selected = 1;
    assert!(witness.active_continuation().is_none());
    witness.initial_resumed = 1;
    assert!(witness.active_continuation().is_some());
    witness.managed_invalidated = true;
    assert!(witness.active_continuation().is_none());
    assert!(witness.managed_deferred.is_none());
}

fn deferred_test_stop() -> DeferredManagedContinuation {
    let (event, pre) = resume_test_event();
    DeferredManagedContinuation {
        stop: OriginalExceptionStop { event, sequence: 8 },
        context: pre.context,
        receipt: ManagedContinuationDeferredReceipt {
            generation: [1; 16],
            identity: pre.identity,
            pre_return_sequence: 4,
            first_delivery_sequence: 8,
            phase: "suspended_before_continue",
            suspend_previous_count: 0,
            reply_later_confirmed: false,
            node_create_sequence: None,
            resume_previous_count: None,
            replay_sequence: None,
            suspend_owned: true,
            original_thread_exit_confirmed: false,
        },
    }
}

#[test]
fn deferred_continue_ack_requires_original_event_sequence_and_actual_reply_later() {
    let mut deferred = deferred_test_stop();
    let event = deferred.stop.event;
    assert!(deferred_ack_matches(&event, 8, DBG_REPLY_LATER, &deferred));
    assert!(!deferred_ack_matches(&event, 9, DBG_REPLY_LATER, &deferred));
    assert!(!deferred_ack_matches(
        &event,
        8,
        NTSTATUS(0x10002),
        &deferred
    ));
    let mut changed = event;
    changed.dwThreadId += 1;
    assert!(!deferred_ack_matches(
        &changed,
        8,
        DBG_REPLY_LATER,
        &deferred
    ));
    changed = event;
    changed.u.Exception.ExceptionRecord.NumberParameters = 1;
    assert!(!deferred_ack_matches(
        &changed,
        8,
        DBG_REPLY_LATER,
        &deferred
    ));
    deferred.receipt.suspend_previous_count = 1;
    assert!(!deferred_ack_matches(&event, 8, DBG_REPLY_LATER, &deferred));
    deferred.receipt.suspend_previous_count = 0;
    deferred.receipt.reply_later_confirmed = true;
    assert!(!deferred_ack_matches(&event, 8, DBG_REPLY_LATER, &deferred));
}

#[test]
fn deferred_replay_requires_original_context_and_node_between_both_deliveries() {
    let mut deferred = deferred_test_stop();
    deferred.receipt.phase = "awaiting_replay";
    deferred.receipt.reply_later_confirmed = true;
    deferred.receipt.node_create_sequence = Some(9);
    record_deferred_resume(&mut deferred.receipt, 1).unwrap();
    let event = deferred.stop.event;
    let current = deferred.context;
    assert!(deferred_replay_matches(&event, 10, 9, &current, &deferred));
    for (sequence, node) in [(8, 9), (9, 9), (10, 0), (10, 8), (10, 10)] {
        assert!(!deferred_replay_matches(
            &event, sequence, node, &current, &deferred
        ));
    }
    let changed = CONTEXT {
        Rsp: current.Rsp + 8,
        ..current
    };
    assert!(!deferred_replay_matches(&event, 10, 9, &changed, &deferred));
    let changed = CONTEXT {
        Dr6: current.Dr6 ^ 2,
        ..current
    };
    assert!(!deferred_replay_matches(&event, 10, 9, &changed, &deferred));
    deferred.receipt.suspend_owned = true;
    assert!(!deferred_replay_matches(&event, 10, 9, &current, &deferred));
    deferred.receipt.suspend_owned = false;
    deferred.receipt.replay_sequence = Some(10);
    assert!(!deferred_replay_matches(&event, 10, 9, &current, &deferred));
}

#[test]
fn a_bound_frame_before_child_delivery_is_deferable_but_not_a_post_node_read() {
    let target = continuation_test_target();
    let identity = continuation_identity(&target);
    let current = CONTEXT {
        Rip: target.address,
        Rsp: target.frame_rsp,
        ..CONTEXT::default()
    };
    assert!(continuation_frame_matches(&target, identity, &current));
    assert!(!continuation_hit_matches(
        &target, identity, None, 8, &current
    ));
    assert!(continuation_hit_matches(
        &target,
        identity,
        Some((9, 10, 11)),
        12,
        &current
    ));
}

#[test]
fn any_completed_resume_consumes_only_our_increment_even_if_the_count_is_unexpected() {
    for previous in [0_u32, 1, 2, 3] {
        let mut deferred = deferred_test_stop();
        deferred.receipt.suspend_previous_count = previous.saturating_sub(1);
        let original = deferred.receipt.suspend_previous_count;
        let result = record_deferred_resume(&mut deferred.receipt, previous);
        assert_eq!(result.is_ok(), previous == 1);
        assert!(!deferred.receipt.suspend_owned);
        assert_eq!(deferred.receipt.resume_previous_count, Some(previous));
        assert_eq!(deferred.receipt.suspend_previous_count, original);
    }
    let mut deferred = deferred_test_stop();
    assert!(record_deferred_resume(&mut deferred.receipt, u32::MAX).is_err());
    assert!(deferred.receipt.suspend_owned);
    assert!(deferred.receipt.resume_previous_count.is_none());
}

#[test]
fn wrong_deferred_continue_status_keeps_the_suspend_owned_for_cleanup() {
    let (mut witness, event) = managed_resume_witness();
    let mut deferred = deferred_test_stop();
    deferred.stop.event = event;
    deferred.receipt.identity = witness.pre_return_binding.unwrap().0;
    witness.managed_resume = None;
    witness.managed_selected = 0;
    witness.managed_deferred = Some(deferred);
    assert!(
        witness
            .on_event_continued(&event, 8, NTSTATUS(0x10002))
            .is_err()
    );
    assert!(witness.stopped);
    assert!(witness.managed_invalidated);
    assert!(
        witness
            .managed_deferred
            .as_ref()
            .unwrap()
            .receipt
            .suspend_owned
    );
    assert!(witness.requires_restoration());
}

#[test]
fn root_sampling_preserves_the_independent_awaiting_replay_ticket() {
    let (mut witness, event) = managed_resume_witness();
    let mut deferred = deferred_test_stop();
    deferred.receipt.phase = "awaiting_replay";
    deferred.receipt.suspend_owned = false;
    witness.managed_resume = None;
    witness.managed_selected = 0;
    witness.managed_deferred = Some(deferred);
    assert!(witness.sample_post_node_root_stop(&event, 8).is_err());
    assert_eq!(
        witness.managed_deferred.as_ref().unwrap().receipt.phase,
        "awaiting_replay"
    );
    assert!(witness.managed.is_some());
}

#[test]
fn cancellation_after_suspend_release_is_idempotent_and_preserves_a_completed_replay() {
    let (mut witness, _) = managed_resume_witness();
    let mut deferred = deferred_test_stop();
    deferred.receipt.phase = "replayed";
    deferred.receipt.replay_sequence = Some(10);
    record_deferred_resume(&mut deferred.receipt, 1).unwrap();
    witness.managed_deferred = Some(deferred);
    witness.cancel_deferred_after_target_termination().unwrap();
    witness.cancel_deferred_after_target_termination().unwrap();
    let receipt = witness.summary().managed_continuation_deferred.unwrap();
    assert_eq!(receipt.phase, "replayed");
    assert_eq!(receipt.resume_previous_count, Some(1));
    assert_eq!(receipt.replay_sequence, Some(10));
    assert!(!receipt.suspend_owned);
}

#[test]
fn managed_resume_rejects_replay_without_promoting_any_classification_budget() {
    let (mut witness, event) = managed_resume_witness();
    assert!(witness.managed_continuation_target().is_ok());
    assert!(
        witness
            .resume_after_managed_continuation(&event, 9)
            .is_err()
    );
    assert!(
        witness
            .resume_after_managed_continuation(&event, 8)
            .is_err()
    );
    assert!(witness.managed_continuation_target().is_err());
    let summary = witness.summary();
    assert!(summary.stopped && summary.managed_continuation_invalidated);
    assert_eq!(summary.managed_continuation_selected_calls, 1);
    assert_eq!(summary.managed_continuation_resumed_calls, 0);
    assert_eq!(summary.pre_node_returned_calls, 1);
    assert_eq!(summary.selected_calls, 0);
    assert_eq!(summary.returned_calls, 0);
    assert_eq!(summary.post_node_exception_selected_calls, 0);
    assert_eq!(summary.result, "unknown");
}

#[test]
fn an_intervening_operation_retires_the_managed_stop_lease() {
    let (mut witness, event) = managed_resume_witness();
    assert!(witness.sample_post_node_root_stop(&event, 8).is_err());
    assert!(witness.managed_continuation_target().is_err());
    assert!(
        witness
            .resume_after_managed_continuation(&event, 8)
            .is_err()
    );
    assert!(witness.summary().managed_continuation_invalidated);
    assert!(witness.stopped);
}

#[test]
fn post_return_cannot_revive_a_spent_managed_lease() {
    let (mut witness, event) = managed_resume_witness();
    witness.selected = 1;
    witness.returned = 1;
    assert!(
        !witness
            .resume_after_post_node_return_for_continuation(&event, 8)
            .unwrap()
    );
    assert!(witness.stopped);
    assert!(witness.managed.is_none());
    assert_eq!(witness.summary().managed_continuation_resumed_calls, 0);
    assert_eq!(witness.summary().returned_calls, 1);
}

#[test]
fn post_return_resume_requires_its_own_exact_event_ticket() {
    let (mut witness, mut event) = managed_resume_witness();
    witness.selected = 1;
    witness.returned = 1;
    witness.managed_selected = 0;
    witness.managed_receipt = None;
    witness.post_return_resume = witness.managed_resume.take();
    event.u.Exception.ExceptionRecord.NumberParameters = 1;
    assert!(
        witness
            .resume_after_post_node_return_for_continuation(&event, 8)
            .is_err()
    );
    assert!(witness.stopped);
    assert!(witness.summary().managed_continuation_invalidated);
    assert!(
        !witness
            .resume_after_post_node_return_for_continuation(&event, 8)
            .unwrap()
    );
    assert_eq!(witness.summary().returned_calls, 1);
}

#[test]
fn failed_resume_consumes_the_ticket_and_cannot_clear_a_stop() {
    let (mut witness, event) = stopped_resume_witness();
    assert!(witness.resume_after_pre_node_return(&event, 5).is_err());
    assert!(witness.stopped);
    assert!(witness.pre_resume.is_none());
    assert!(witness.resume_after_pre_node_return(&event, 4).is_err());
    assert!(witness.stopped);
    assert_eq!(witness.summary().result, "unknown");
}

#[test]
fn changed_process_identity_invalidates_resume_without_clearing_the_stop() {
    let (mut witness, event) = stopped_resume_witness();
    witness.process_birth += 1;
    assert!(witness.resume_after_pre_node_return(&event, 4).is_err());
    assert!(witness.pre_resume.is_none());
    assert!(witness.stopped);
    witness.process_birth -= 1;
    assert!(witness.resume_after_pre_node_return(&event, 4).is_err());
    assert!(witness.stopped);
}

#[test]
fn explicit_finish_or_intervening_event_invalidates_pre_node_resume() {
    let (mut witness, event) = stopped_resume_witness();
    witness.finish_observation(&event).unwrap();
    assert!(witness.resume_after_pre_node_return(&event, 4).is_err());
    assert!(witness.stopped);
    let (mut witness, event) = stopped_resume_witness();
    assert!(matches!(
        witness.observe(&event, 5).unwrap(),
        Observation::NotOwned
    ));
    assert!(witness.resume_after_pre_node_return(&event, 4).is_err());
    assert!(witness.stopped);
}

#[test]
fn post_node_sample_rejects_child_create_and_exited_events_without_claiming_coverage() {
    let (mut witness, mut event) = stopped_resume_witness();
    witness.node = Some((3, 5, 6));
    let original_sequence = witness.last_sequence;
    event.dwDebugEventCode = CREATE_PROCESS_DEBUG_EVENT;
    assert!(witness.sample_post_node_root_stop(&event, 5).is_err());
    event.dwProcessId += 1;
    event.dwDebugEventCode = EXCEPTION_DEBUG_EVENT;
    assert!(witness.sample_post_node_root_stop(&event, 5).is_err());
    event.dwProcessId = witness.root_pid;
    event.dwDebugEventCode = EXIT_THREAD_DEBUG_EVENT;
    assert!(witness.sample_post_node_root_stop(&event, 5).is_err());
    event.dwDebugEventCode = windows::Win32::System::Diagnostics::Debug::EXIT_PROCESS_DEBUG_EVENT;
    assert!(witness.sample_post_node_root_stop(&event, 5).is_err());
    assert!(witness.post_node_root_stop.is_none());
    assert!(!witness.post_node_sample_attempted);
    assert_eq!(witness.last_sequence, original_sequence);
}

#[test]
fn failed_post_node_sample_has_one_attempt_and_does_not_advance_observation() {
    let (mut witness, event) = stopped_resume_witness();
    witness.node = Some((3, 5, 6));
    assert!(witness.sample_post_node_root_stop(&event, 5).is_err());
    assert!(witness.post_node_sample_attempted);
    assert!(witness.post_node_root_stop.is_none());
    assert!(!witness.sample_post_node_root_stop(&event, 6).unwrap());
    assert_eq!(witness.last_sequence, 4);
}

#[test]
fn confirmed_root_exit_cannot_be_sampled_as_a_live_stop() {
    let (mut witness, event) = stopped_resume_witness();
    witness.node = Some((3, 5, 6));
    witness.exited = true;
    assert!(witness.sample_post_node_root_stop(&event, 5).is_err());
    assert!(witness.post_node_sample_attempted);
    assert!(witness.post_node_root_stop.is_none());
    assert_eq!(witness.last_sequence, 4);
}

#[test]
fn exception_metadata_preserves_unsupported_events_without_promoting_them_to_clr() {
    let (mut event, _) = resume_test_event();
    event.u.Exception.ExceptionRecord.ExceptionCode =
        windows::Win32::Foundation::NTSTATUS(CLR_EXCEPTION as i32);
    event.u.Exception.ExceptionRecord.NumberParameters = 1;
    unsafe {
        event.u.Exception.ExceptionRecord.ExceptionInformation[0] = 0xffff_ffff_8007_0005;
    }
    assert_eq!(
        exception_metadata(&event),
        (CLR_EXCEPTION, 1, 1, Some(0x80070005), true)
    );
    event.u.Exception.dwFirstChance = 0;
    assert_eq!(
        exception_metadata(&event),
        (CLR_EXCEPTION, 0, 1, Some(0x80070005), false)
    );
    event.u.Exception.dwFirstChance = 1;
    event.u.Exception.ExceptionRecord.ExceptionCode = EXCEPTION_SINGLE_STEP;
    assert_eq!(
        exception_metadata(&event),
        (
            EXCEPTION_SINGLE_STEP.0 as u32,
            1,
            1,
            Some(0x80070005),
            false
        )
    );
    event.u.Exception.ExceptionRecord.ExceptionCode =
        windows::Win32::Foundation::NTSTATUS(CLR_EXCEPTION as i32);
    for count in [0, 16, u32::MAX] {
        event.u.Exception.ExceptionRecord.NumberParameters = count;
        assert_eq!(
            exception_metadata(&event),
            (CLR_EXCEPTION, 1, count, None, false)
        );
    }
}

#[test]
fn exception_stop_ticket_rejects_changed_original_exception_fields() {
    let (mut event, _) = resume_test_event();
    event.u.Exception.ExceptionRecord.NumberParameters = 1;
    unsafe {
        event.u.Exception.ExceptionRecord.ExceptionInformation[0] = 0x80070005;
    }
    let stop = OriginalExceptionStop { event, sequence: 7 };
    assert!(same_exception_stop(&event, 7, &stop));
    assert!(!same_exception_stop(&event, 8, &stop));
    let mut changed = event;
    changed.dwThreadId += 1;
    assert!(!same_exception_stop(&changed, 7, &stop));
    changed = event;
    changed.u.Exception.dwFirstChance = 0;
    assert!(!same_exception_stop(&changed, 7, &stop));
    changed = event;
    changed.u.Exception.ExceptionRecord.ExceptionFlags = 1;
    assert!(!same_exception_stop(&changed, 7, &stop));
    changed = event;
    changed.u.Exception.ExceptionRecord.ExceptionAddress = 0x30000 as *mut c_void;
    assert!(!same_exception_stop(&changed, 7, &stop));
    changed = event;
    changed.u.Exception.ExceptionRecord.NumberParameters = 2;
    assert!(!same_exception_stop(&changed, 7, &stop));
    changed = event;
    unsafe {
        changed.u.Exception.ExceptionRecord.ExceptionInformation[0] = 0x80070006;
    }
    assert!(!same_exception_stop(&changed, 7, &stop));
}

fn post_exception_witness() -> (ShellClassificationWitness, DEBUG_EVENT) {
    let (mut witness, mut event) = stopped_resume_witness();
    witness.invalidate_stop_tickets();
    witness.stopped = false;
    let tid = unsafe { GetCurrentThreadId() };
    witness
        .insert_thread(tid, unsafe {
            windows::Win32::System::Threading::GetCurrentThread()
        })
        .unwrap();
    event.dwThreadId = tid;
    event.u.Exception.ExceptionRecord.NumberParameters = 1;
    unsafe {
        event.u.Exception.ExceptionRecord.ExceptionInformation[0] = 0x80070005;
    }
    witness.pre_return_binding = Some((witness.threads[&tid].identity, 4));
    witness.node = Some((5, witness.root_pid + 1, 6));
    witness.last_sequence = 6;
    witness.post_exception_candidate = Some(OriginalExceptionStop { event, sequence: 6 });
    (witness, event)
}

fn pre_exception_witness() -> (ShellClassificationWitness, DEBUG_EVENT) {
    let (mut witness, mut event) = post_exception_witness();
    let (identity, pre_return_sequence) = witness.pre_return_binding.unwrap();
    witness.node = None;
    witness.initial_bound = true;
    witness.initial_selected = 1;
    witness.initial_resumed = 1;
    witness.initial_receipt = Some(ManagedInitialReceipt {
        generation: witness.generation,
        identity,
        pre_return_sequence,
        event_sequence: 5,
        registers_restored: true,
        execution_context_unchanged: true,
        managed_initial_clr_stack_required: true,
        mapping: serde_json::json!({}),
    });
    event.u.Exception.ExceptionRecord.ExceptionCode = NTSTATUS(CLR_EXCEPTION as i32);
    event.u.Exception.dwFirstChance = 0;
    witness.post_exception_candidate = Some(OriginalExceptionStop { event, sequence: 6 });
    (witness, event)
}

#[test]
fn first_pre_start_clr_consumes_only_its_budget_even_when_unsupported() {
    let (mut witness, event) = pre_exception_witness();
    let receipt = witness
        .observe_pre_start_exception(&event, 6)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.initial_sequence, 5);
    assert_eq!(receipt.event_sequence, 6);
    assert!(!receipt.reader_eligible && !receipt.registers_restored);
    assert_eq!(witness.pre_exception_selected, 1);
    assert_eq!(witness.post_exception_selected, 0);
    assert_eq!(witness.managed_selected, 0);
    assert!(!witness.stopped);
    witness.last_sequence = 7;
    witness.post_exception_candidate = Some(OriginalExceptionStop { event, sequence: 7 });
    assert!(
        witness
            .observe_pre_start_exception(&event, 7)
            .unwrap()
            .is_none()
    );
    assert_eq!(witness.pre_exception.as_ref().unwrap().event_sequence, 6);
}

#[test]
fn pre_start_clr_window_requires_initial_resume_original_worker_and_no_node() {
    let (mut witness, event) = pre_exception_witness();
    witness.initial_resumed = 0;
    assert!(
        witness
            .observe_pre_start_exception(&event, 6)
            .unwrap()
            .is_none()
    );
    assert!(witness.post_exception_candidate.is_some());
    let (mut witness, event) = pre_exception_witness();
    witness.node = Some((5, witness.root_pid + 1, 6));
    assert!(
        witness
            .observe_pre_start_exception(&event, 6)
            .unwrap()
            .is_none()
    );
    assert!(witness.post_exception_candidate.is_some());
    assert!(
        witness
            .observe_post_node_exception(&event, 6)
            .unwrap()
            .is_some()
    );
    let (mut witness, mut event) = pre_exception_witness();
    event.dwThreadId += 1;
    assert!(
        witness
            .observe_pre_start_exception(&event, 6)
            .unwrap()
            .is_none()
    );
    assert_eq!(witness.pre_exception_selected, 0);
}

#[test]
fn native_exception_does_not_consume_pre_start_clr_budget() {
    let (mut witness, mut event) = pre_exception_witness();
    event.u.Exception.ExceptionRecord.ExceptionCode = EXCEPTION_SINGLE_STEP;
    assert!(
        witness
            .observe_pre_start_exception(&event, 6)
            .unwrap()
            .is_none()
    );
    assert_eq!(witness.pre_exception_selected, 0);
    assert!(witness.post_exception_candidate.is_some());
}

#[test]
fn pre_start_clr_requires_exact_observed_exception_and_initial_order() {
    let (mut witness, event) = pre_exception_witness();
    witness.post_exception_candidate = None;
    assert!(witness.observe_pre_start_exception(&event, 6).is_err());
    assert!(witness.stopped);
    let (mut witness, mut event) = pre_exception_witness();
    unsafe {
        event.u.Exception.ExceptionRecord.ExceptionInformation[0] += 1;
    }
    assert!(witness.observe_pre_start_exception(&event, 6).is_err());
    let (mut witness, event) = pre_exception_witness();
    witness.initial_receipt.as_mut().unwrap().event_sequence = 6;
    assert!(witness.observe_pre_start_exception(&event, 6).is_err());
    assert_eq!(witness.pre_exception_selected, 0);
}

#[test]
fn unsupported_first_post_node_exception_consumes_budget_without_restoring_or_stopping() {
    let (mut witness, event) = post_exception_witness();
    let restored = witness.restored;
    let receipt = witness
        .observe_post_node_exception(&event, 6)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.exception_code, EXCEPTION_SINGLE_STEP.0 as u32);
    assert_eq!(receipt.event_hresult, Some(0x80070005));
    assert!(!receipt.reader_eligible);
    assert!(!receipt.registers_restored);
    assert!(!receipt.execution_context_unchanged);
    assert!(!witness.stopped);
    assert_eq!(witness.restored, restored);
    assert_eq!(witness.post_exception_selected, 1);
    assert_eq!(witness.selected, 0);
    assert_eq!(witness.returned, 0);
    assert_eq!(witness.summary().result, "unknown");
    assert!(witness.resume_after_post_node_exception(&event, 6).is_err());
    let mut next = event;
    next.u.Exception.ExceptionRecord.ExceptionCode =
        windows::Win32::Foundation::NTSTATUS(CLR_EXCEPTION as i32);
    witness.last_sequence = 7;
    witness.post_exception_candidate = Some(OriginalExceptionStop {
        event: next,
        sequence: 7,
    });
    assert!(
        witness
            .observe_post_node_exception(&next, 7)
            .unwrap()
            .is_none()
    );
    assert_eq!(witness.post_exception.as_ref().unwrap().event_sequence, 6);
    assert_eq!(witness.post_exception_resumed, 0);
}

#[test]
fn post_node_exception_requires_the_exact_prior_unowned_event_and_order() {
    let (mut witness, event) = post_exception_witness();
    witness.post_exception_candidate = None;
    assert!(witness.observe_post_node_exception(&event, 6).is_err());
    assert!(witness.stopped);
    let (mut witness, mut event) = post_exception_witness();
    unsafe {
        event.u.Exception.ExceptionRecord.ExceptionInformation[0] = 0x80070006;
    }
    assert!(witness.observe_post_node_exception(&event, 6).is_err());
    assert!(witness.stopped);
    let (mut witness, event) = post_exception_witness();
    witness.node.as_mut().unwrap().0 = 6;
    assert!(witness.observe_post_node_exception(&event, 6).is_err());
    assert_eq!(witness.post_exception_selected, 0);
}

#[test]
fn post_node_exception_requires_pre_return_original_worker_and_unused_post_budget() {
    let (mut witness, mut event) = post_exception_witness();
    event.dwThreadId += 1;
    assert!(
        witness
            .observe_post_node_exception(&event, 6)
            .unwrap()
            .is_none()
    );
    assert_eq!(witness.post_exception_selected, 0);
    let (mut witness, event) = post_exception_witness();
    witness.pre_return_binding = None;
    assert!(
        witness
            .observe_post_node_exception(&event, 6)
            .unwrap()
            .is_none()
    );
    let (mut witness, event) = post_exception_witness();
    witness.selected = 1;
    assert!(
        witness
            .observe_post_node_exception(&event, 6)
            .unwrap()
            .is_none()
    );
    assert_eq!(witness.post_exception_selected, 0);
    let (mut witness, event) = post_exception_witness();
    witness.pre_return_binding.as_mut().unwrap().0.thread_birth += 1;
    assert!(witness.observe_post_node_exception(&event, 6).is_err());
    assert!(witness.stopped);
    assert_eq!(witness.post_exception_selected, 1);
    assert!(witness.post_exception_resume.is_none());
}

#[test]
fn pending_classification_unwind_cannot_create_an_exception_resume_ticket() {
    let (mut witness, mut event) = post_exception_witness();
    event.u.Exception.ExceptionRecord.ExceptionCode =
        windows::Win32::Foundation::NTSTATUS(CLR_EXCEPTION as i32);
    witness.threads.get_mut(&event.dwThreadId).unwrap().pending = Some(Pending {
        stack: 0x100008,
        returned_to: 0x200000,
        region: ReturnRegion {
            allocation: 0x200000,
            base: 0x200000,
            length: 4096,
            kind: MEM_PRIVATE.0,
            protect: PAGE_EXECUTE_READ.0,
            bytes: vec![0x90],
        },
        sequence: 6,
        nonce: [1; 16],
        phase: CallPhase::PostNode {
            node_create_sequence: 5,
        },
    });
    assert!(
        witness
            .observe_post_node_exception(&event, 6)
            .unwrap()
            .is_none()
    );
    assert_eq!(witness.post_exception_selected, 0);
    assert!(matches!(
        witness.observe(&event, 7).unwrap(),
        Observation::NotOwned
    ));
    assert!(witness.stopped);
    assert!(
        witness
            .observe_post_node_exception(&event, 7)
            .unwrap()
            .is_none()
    );
    assert!(witness.resume_after_post_node_exception(&event, 7).is_err());
    assert_eq!(witness.post_exception_selected, 0);
    assert_eq!(witness.post_exception_resumed, 0);
}

fn exception_resume_witness() -> (ShellClassificationWitness, DEBUG_EVENT) {
    let (mut witness, mut event) = post_exception_witness();
    event.u.Exception.ExceptionRecord.ExceptionCode =
        windows::Win32::Foundation::NTSTATUS(CLR_EXCEPTION as i32);
    let (identity, pre_return_sequence) = witness.pre_return_binding.unwrap();
    let node = witness.node.unwrap();
    witness.stopped = true;
    witness.post_exception_selected = 1;
    witness.post_exception_candidate = None;
    witness.post_exception = Some(PostNodeExceptionReceipt {
        generation: witness.generation,
        identity,
        pre_return_sequence,
        node_create_sequence: node.0,
        node_process_id: node.1,
        node_process_birth: node.2,
        event_sequence: 6,
        exception_code: CLR_EXCEPTION,
        first_chance: 1,
        parameter_count: 1,
        event_hresult: Some(0x80070005),
        reader_eligible: true,
        registers_restored: true,
        execution_context_unchanged: true,
    });
    witness.post_exception_resume = Some(PostNodeExceptionResume {
        stop: OriginalExceptionStop { event, sequence: 6 },
        identity,
        context: CONTEXT::default(),
    });
    (witness, event)
}

#[test]
fn post_node_exception_resume_rejects_changed_event_and_consumes_the_ticket() {
    let (mut witness, event) = exception_resume_witness();
    let mut changed = event;
    unsafe {
        changed.u.Exception.ExceptionRecord.ExceptionInformation[0] = 0x80070006;
    }
    assert!(
        witness
            .resume_after_post_node_exception(&changed, 6)
            .is_err()
    );
    assert!(witness.post_exception_resume.is_none());
    assert!(witness.stopped);
    assert!(witness.resume_after_post_node_exception(&event, 6).is_err());
    assert_eq!(witness.post_exception_resumed, 0);
}

#[test]
fn post_node_exception_resume_cannot_reuse_pre_ticket_or_survive_finish() {
    let (mut witness, event) = exception_resume_witness();
    assert!(witness.resume_after_pre_node_return(&event, 6).is_err());
    assert!(witness.post_exception_resume.is_none());
    assert!(witness.resume_after_post_node_exception(&event, 6).is_err());
    assert!(witness.stopped);
    let (mut witness, event) = exception_resume_witness();
    witness.finish_observation(&event).unwrap();
    assert!(witness.resume_after_post_node_exception(&event, 6).is_err());
    assert!(witness.stopped);
}

#[test]
fn post_node_exception_resume_rejects_changed_identity_or_node_generation() {
    let (mut witness, event) = exception_resume_witness();
    witness.process_birth += 1;
    assert!(witness.resume_after_post_node_exception(&event, 6).is_err());
    assert!(witness.post_exception_resume.is_none());
    assert!(witness.stopped);
    let (mut witness, event) = exception_resume_witness();
    witness.node.as_mut().unwrap().2 += 1;
    assert!(witness.resume_after_post_node_exception(&event, 6).is_err());
    assert!(witness.post_exception_resume.is_none());
    assert!(witness.stopped);
}

#[test]
fn pair_nonce_is_unique_per_original_event_and_contains_no_path() {
    assert_ne!(nonce([1; 16], 10), nonce([1; 16], 11));
    assert_ne!(nonce([1; 16], 10), nonce([2; 16], 10));
    let receipt = ReturnReceipt {
        generation: [1; 16],
        identity: ObjectIdentity {
            process_id: 1,
            thread_id: 2,
            process_birth: 3,
            thread_birth: 4,
        },
        pair_nonce: nonce([1; 16], 10),
        entry_sequence: 10,
        return_sequence: 11,
        node_create_sequence: 9,
        node_process_id: 5,
        node_process_birth: 6,
        expected_node_matched: true,
        flags: 0x2000,
        raw_return_u64: 0x4550,
        raw_return_low32: 0x4550,
        return_region_kind: "private",
        registers_restored: true,
        execution_context_unchanged: true,
        post_start_clr_stack_required: true,
    };
    let output = serde_json::to_value(receipt).unwrap();
    assert_eq!(output["expected_node_matched"], true);
    assert!(output.get("path").is_none());
    assert!(output.get("return_address").is_none());
    assert_eq!(output["post_start_clr_stack_required"], true);
}

#[test]
fn malformed_pe_and_address_overflow_are_rejected_without_indexing_panics() {
    assert!(export(&[]).is_err());
    assert!(export(&vec![0; 1024]).is_err());
    assert!(!user_range(u64::MAX, 8));
    assert!(!user_range(0, 1));
    assert!(!user_range(0x10000, 0));
}

#[test]
fn resume_flag_restoration_never_changes_another_instruction_or_original_flag() {
    let mut current = CONTEXT {
        Rip: 0x20000,
        EFlags: RF | 0x202,
        ..CONTEXT::default()
    };
    restore_own_resume_flag(&mut current, Some((0x20001, false)));
    assert_eq!(current.EFlags, RF | 0x202);
    restore_own_resume_flag(&mut current, Some((0x20000, true)));
    assert_eq!(current.EFlags, RF | 0x202);
    restore_own_resume_flag(&mut current, Some((0x20000, false)));
    assert_eq!(current.EFlags, 0x202);
}

#[test]
fn fixed_eflags_api_view_requires_an_owned_control_write_and_one_to_zero_direction() {
    let requested = CONTEXT {
        EFlags: 0x10202,
        ..CONTEXT::default()
    };
    let actual = CONTEXT {
        EFlags: 0x10200,
        ..requested
    };
    assert!(!execution_equal(&requested, &actual));
    assert!(fixed_eflags_api_readback(&requested, &actual, true));
    assert!(!fixed_eflags_api_readback(&requested, &actual, false));
    assert!(!fixed_eflags_api_readback(&actual, &requested, true));
    assert!(execution_equal(&requested, &requested));
    assert!(!fixed_eflags_api_readback(&requested, &requested, true));
}

#[test]
fn fixed_eflags_api_view_rejects_every_other_flag_bit_change() {
    let requested = CONTEXT {
        EFlags: 0x10202,
        ..CONTEXT::default()
    };
    for bit in 0..32 {
        if bit == 1 {
            continue;
        }
        let actual = CONTEXT {
            EFlags: 0x10200 ^ (1 << bit),
            ..requested
        };
        assert!(
            !fixed_eflags_api_readback(&requested, &actual, true),
            "EFLAGS bit {bit}"
        );
    }
}

#[test]
fn fixed_eflags_api_view_rejects_every_changed_execution_register() {
    let requested = CONTEXT {
        EFlags: 0x10202,
        ..CONTEXT::default()
    };
    let mutations: [fn(&mut CONTEXT); 17] = [
        |value| value.Rip ^= 1,
        |value| value.Rsp ^= 1,
        |value| value.Rax ^= 1,
        |value| value.Rcx ^= 1,
        |value| value.Rdx ^= 1,
        |value| value.Rbx ^= 1,
        |value| value.Rbp ^= 1,
        |value| value.Rsi ^= 1,
        |value| value.Rdi ^= 1,
        |value| value.R8 ^= 1,
        |value| value.R9 ^= 1,
        |value| value.R10 ^= 1,
        |value| value.R11 ^= 1,
        |value| value.R12 ^= 1,
        |value| value.R13 ^= 1,
        |value| value.R14 ^= 1,
        |value| value.R15 ^= 1,
    ];
    for mutate in mutations {
        let mut actual = CONTEXT {
            EFlags: 0x10200,
            ..requested
        };
        mutate(&mut actual);
        assert!(!fixed_eflags_api_readback(&requested, &actual, true));
    }
}

fn export_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x800];
    for (at, value) in [
        (0, 0x5a4du16),
        (0x84, 0x8664),
        (0x86, 1),
        (0x94, 0xf0),
        (0x96, 0x2000),
        (0x98, 0x20b),
    ] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [
        (0x3c, 0x80u32),
        (0x80, 0x4550),
        (0xd0, 0x2000),
        (0xd4, 0x200),
        (0x104, 16),
        (0x108, 0x1100),
        (0x10c, 0x100),
        (0x190, 0x600),
        (0x194, 0x1000),
        (0x198, 0x600),
        (0x19c, 0x200),
        (0x1ac, 0x6000_0020),
        (0x314, 1),
        (0x318, 1),
        (0x31c, 0x1140),
        (0x320, 0x1148),
        (0x324, 0x1150),
        (0x340, 0x1020),
        (0x348, 0x1160),
    ] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[0x220..0x230].fill(0x90);
    let name = b"SHGetFileInfoW\0";
    bytes[0x360..0x360 + name.len()].copy_from_slice(name);
    bytes
}

#[test]
fn exact_x64_export_is_bound_to_executable_file_bytes() {
    let bytes = export_fixture();
    let (size, rva, code) = export(&bytes).unwrap();
    assert_eq!(size, 0x2000);
    assert_eq!(rva, 0x1020);
    assert_eq!(code, vec![0x90; 16]);
}

#[test]
fn forwarded_or_duplicate_export_is_not_guessed() {
    let mut forwarded = export_fixture();
    forwarded[0x340..0x344].copy_from_slice(&0x1160u32.to_le_bytes());
    assert!(export(&forwarded).is_err());
    let mut duplicate = export_fixture();
    duplicate[0x318..0x31c].copy_from_slice(&2u32.to_le_bytes());
    duplicate[0x34c..0x350].copy_from_slice(&0x1160u32.to_le_bytes());
    assert!(export(&duplicate).is_err());
}
