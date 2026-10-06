use super::*;

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
