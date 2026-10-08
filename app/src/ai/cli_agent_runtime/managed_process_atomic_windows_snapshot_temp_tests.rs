use super::super::ModuleIdentity;
use super::*;

fn utf16(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn bytes(value: &[u16]) -> Vec<u8> {
    value.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

fn expected() -> [Vec<u16>; 3] {
    [
        utf16("fixture-temp"),
        utf16("fixture-temp"),
        utf16("fixture-home"),
    ]
}

fn kernelbase() -> Module {
    Module {
        identity: ModuleIdentity {
            base: 0x180000000,
            size: 0x400000,
            volume_serial: 1,
            file_index: 2,
            sha256: KERNELBASE_SHA.to_owned(),
        },
        functions: Vec::new(),
    }
}

fn frame(pc: u64, stack: u64) -> Frame {
    Frame {
        instruction: 0x180000000 + pc,
        stack,
        module_index: Some(0),
        module_offset: Some(pc),
    }
}

#[test]
fn environment_read_stops_at_double_nul_before_adjacent_memory() {
    let value = bytes(&utf16(
        "TMP=fixture-temp\0TEMP=fixture-temp\0USERPROFILE=fixture-home\0\0",
    ));
    let mut read_end = 0;
    let result = read_environment(0x10000, &mut |address, output| {
        let offset = (address - 0x10000) as usize;
        read_end = offset + output.len();
        output.copy_from_slice(
            value
                .get(offset..read_end)
                .expect("不得读取终止符后的相邻内存"),
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(read_end, value.len());
    assert_eq!(
        inspect_variables(&result, &expected()).unwrap()[0].matches_expected,
        Some(true)
    );
}

#[test]
fn environment_read_preserves_cancellation_without_retrying() {
    let mut calls = 0;
    let failure = read_environment(0x10000, &mut |_, _| {
        calls += 1;
        Err(rejected("cancelled_or_deadline_reached"))
    })
    .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(failure.reason, "cancelled_or_deadline_reached");
}

#[test]
fn environment_read_rejects_unterminated_limit() {
    let mut read_end = 0;
    let failure = read_environment(0x10000, &mut |address, output| {
        read_end = address + output.len() as u64;
        output.copy_from_slice(&[b'A', 0]);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(read_end, 0x14000);
    assert_eq!(failure.reason, "environment_unterminated_or_over_limit");
}

#[test]
fn environment_read_rejects_partial_input_instead_of_reporting_missing_keys() {
    let failure = read_environment(0x10000, &mut |address, output| {
        if address == 0x10000 {
            output.copy_from_slice(&[b'T', 0]);
            Ok(())
        } else {
            Err(rejected("read_failed"))
        }
    })
    .unwrap_err();
    assert_eq!(failure.reason, "read_failed");
}

#[test]
fn variables_distinguish_missing_empty_and_changed_values() {
    let result = inspect_variables(&utf16("TMP=\0TEMP=changed\0\0"), &expected()).unwrap();
    assert_eq!(result[0].count, 1);
    assert_eq!(result[0].utf16_units, Some(0));
    assert_eq!(result[0].matches_expected, Some(false));
    assert_eq!(result[1].utf16_units, Some(7));
    assert_eq!(result[1].matches_expected, Some(false));
    assert_eq!(result[2].count, 0);
    assert_eq!(result[2].utf16_units, None);
    assert_eq!(result[2].matches_expected, None);
}

#[test]
fn variables_reject_case_insensitive_duplicates() {
    let failure = inspect_variables(
        &utf16("TMP=fixture-temp\0tMp=fixture-temp\0\0"),
        &expected(),
    )
    .unwrap_err();
    assert_eq!(failure.reason, "environment_target_key_duplicated");
}

#[test]
fn variables_count_utf16_units_without_serializing_values_or_other_keys() {
    let expected = [utf16("😺"), utf16("fixture-temp"), utf16("fixture-home")];
    let result = inspect_variables(
        &utf16("=C:=private-drive\0OTHER=private-secret\0tMp=😺\0\0"),
        &expected,
    )
    .unwrap();
    assert_eq!(result[0].utf16_units, Some(2));
    assert_eq!(result[0].matches_expected, Some(true));
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains("private"));
    assert!(!serialized.contains("OTHER"));
    assert!(!serialized.contains("😺"));
}

fn environment_memory() -> Vec<(u64, Vec<u8>)> {
    vec![
        (0x10020, 0x20000u64.to_le_bytes().to_vec()),
        (0x20080, 0x30000u64.to_le_bytes().to_vec()),
        (
            0x30000,
            bytes(&utf16(
                "TMP=fixture-temp\0TEMP=fixture-temp\0USERPROFILE=fixture-home\0\0",
            )),
        ),
    ]
}

fn read_fixture(memory: &[(u64, Vec<u8>)], address: u64, output: &mut [u8]) -> Result<(), Failure> {
    let (base, value) = memory
        .iter()
        .find(|(base, value)| {
            address >= *base && address + output.len() as u64 <= *base + value.len() as u64
        })
        .expect("只读两个指针槽和环境段");
    let offset = (address - base) as usize;
    output.copy_from_slice(&value[offset..offset + output.len()]);
    Ok(())
}

#[test]
fn environment_observation_requires_two_equal_reads_without_claiming_atomicity() {
    let memory = environment_memory();
    let result = inspect_environment(0x10000, &expected(), &mut |address, output| {
        read_fixture(&memory, address, output)
    })
    .unwrap();
    assert!(result.observed_stable);
    assert!(!result.process_atomicity_proven);
    assert!(
        result
            .variables
            .iter()
            .all(|variable| variable.matches_expected == Some(true))
    );
}

#[test]
fn environment_observation_rejects_changed_pointer_even_with_identical_values() {
    let memory = environment_memory();
    let mut parameter_reads = 0;
    let failure = inspect_environment(0x10000, &expected(), &mut |address, output| {
        if address == 0x20080 {
            parameter_reads += 1;
            if parameter_reads == 2 {
                output.copy_from_slice(&0x40000u64.to_le_bytes());
                return Ok(());
            }
        }
        read_fixture(&memory, address, output)
    })
    .unwrap_err();
    assert_eq!(failure.reason, "environment_changed_during_observation");
}

#[test]
fn environment_observation_rejects_changed_value_without_pointer_change() {
    let memory = environment_memory();
    let mut first_unit_reads = 0;
    let failure = inspect_environment(0x10000, &expected(), &mut |address, output| {
        read_fixture(&memory, address, output)?;
        if address == 0x30000 {
            first_unit_reads += 1;
            if first_unit_reads == 2 {
                output.copy_from_slice(&[b'X', 0]);
            }
        }
        Ok(())
    })
    .unwrap_err();
    assert_eq!(failure.reason, "environment_changed_during_observation");
}

#[test]
fn basic_information_requires_success_exact_length_and_pid() {
    let data = [0, 0x10000, 0, 0, 42, 0];
    assert_eq!(basic_information_peb(0, 48, &data, 42).unwrap(), 0x10000);
    assert_eq!(
        basic_information_peb(-1, 48, &data, 42).unwrap_err().reason,
        "NtQueryInformationProcess_failed"
    );
    assert_eq!(
        basic_information_peb(0, 40, &data, 42).unwrap_err().reason,
        "process_basic_information_mismatch"
    );
    assert_eq!(
        basic_information_peb(0, 48, &data, 43).unwrap_err().reason,
        "process_basic_information_mismatch"
    );
}

#[test]
fn retry_frame_keeps_raw_insufficient_lengths_without_normalizing() {
    let frames = [frame(0xef802, 0x10000)];
    let result = inspect_frame(&frames, 0, &kernelbase(), &mut |address, output| {
        match address {
            0x10024 => output.copy_from_slice(&0xc0000023u32.to_le_bytes()),
            0x10030 => output.copy_from_slice(&[0x58, 0x02, 0x0a, 0x02]),
            _ => panic!("不读取 Buffer 或其他栈位置"),
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(result.saved_query_status, Some(0xc0000023));
    assert_eq!(result.length_bytes, Some(600));
    assert_eq!(result.maximum_length_bytes, Some(522));
}

#[test]
fn query_return_without_initialized_saved_status_is_skipped() {
    let result = inspect_frame(&[frame(0xef65f, 0x10000)], 0, &kernelbase(), &mut |_, _| {
        panic!("不读未初始化槽")
    })
    .unwrap();
    assert_eq!(result.status, "skipped");
    assert_eq!(result.saved_query_status, None);
}

#[test]
fn retry_frame_outside_original_stack_is_rejected_before_read() {
    let frames = [frame(0xef65f, 0x10000), frame(0xef802, 0x110000)];
    let failure = inspect_frame(&frames, 0, &kernelbase(), &mut |_, _| {
        panic!("不读栈范围外地址")
    })
    .unwrap_err();
    assert_eq!(failure.reason, "retry_frame_outside_original_stack");
}

#[test]
fn ambiguous_fixed_module_stops_before_any_remote_read() {
    let modules = PreparedModules {
        modules: vec![kernelbase(), kernelbase()],
    };
    let result = observe(
        HANDLE(1usize as *mut c_void),
        42,
        &modules,
        &[],
        &expected(),
        &mut |_, _| panic!("未唯一绑定前不读远端"),
    );
    assert_eq!(result.status, "failed");
    assert_eq!(result.failure.unwrap().reason, "fixed_module_ambiguous");
    assert_eq!(result.nt_query_status, None);
}

#[test]
fn mismatched_code_records_all_fixed_checks_before_rejecting_environment() {
    let mut ntdll = kernelbase();
    ntdll.identity.sha256 = NTDLL_SHA.to_owned();
    ntdll.identity.base = 0x190000000;
    let modules = PreparedModules {
        modules: vec![kernelbase(), ntdll],
    };
    let mut calls = 0;
    let result = observe(
        HANDLE(1usize as *mut c_void),
        42,
        &modules,
        &[],
        &expected(),
        &mut |_, output| {
            calls += 1;
            output.fill(0);
            Ok(())
        },
    );
    assert_eq!(calls, 6);
    assert_eq!(result.code_matches, Some(false));
    assert_eq!(result.iat_matches, Some(false));
    assert_eq!(result.code_checks.len(), 3);
    assert_eq!(result.iat_checks.len(), 3);
    assert_eq!(result.iat_checks[0].target_kind, "unknown");
    assert_eq!(result.nt_query_status, None);
    assert_eq!(
        result.failure.unwrap().reason,
        "fixed_function_bytes_changed"
    );
}
