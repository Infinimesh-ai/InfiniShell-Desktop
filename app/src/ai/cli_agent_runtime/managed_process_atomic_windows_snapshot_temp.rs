//! 固定 Windows 映像的临时路径观察，仅用于调用方已暂停并核验的私有候选根线程。
//! 读取器统一持有原进程句柄、取消检查及预算；本模块不暂停、重开句柄或修改远端。

use std::ffi::c_void;

use serde::Serialize;
use sha2::{Digest as _, Sha256};
use windows::Win32::Foundation::HANDLE;

use super::{Failure, Frame, Module, PreparedModules};

const KERNELBASE_SHA: &str = "e75b795bb6fc69711ed684832e0a73c4eeb1ab177bfc62d7415a26ada167ae1d";
const NTDLL_SHA: &str = "cfb1a39a15036ee71daa52e428523c9e7bf83d85bb1b2f0a268d05c3b5392bdd";
const MAX_ENV_BYTES: usize = 16 * 1024;
const MAX_USER_ADDRESS: u64 = 0x0000_7fff_ffff_ffff;
const KEYS: [&str; 3] = ["TMP", "TEMP", "USERPROFILE"];

// 三段来自同 SHA 原件的原始代码；已核整个范围均无 PE 基址重定位覆盖。
const FUNCTIONS: [(u64, usize, &str); 3] = [
    (
        0xef5e0,
        613,
        "16581b6421c7f599a37a77314d17ab443f96d4457b2d5c1545400f547d6544b5",
    ),
    (
        0xa9320,
        102,
        "e46270acb18be147ef275ee1616d8382455faaaeffa0f6949319e03e0ab8cfdc",
    ),
    (
        0xa95f0,
        2500,
        "7fc65ec0c7549ae36b34782c90ca466365f28b6cb93a6edee8dc1373db91d6e4",
    ),
];
// KERNELBASE IAT 槽及其固定 ntdll 导出 RVA；不追读任何未知目标。
const IAT: [(u64, u64); 3] = [
    (0x2906c0, 0x1c1d0),
    (0x28f808, 0x78ce0),
    (0x2904a0, 0xa9320),
];

type Reader<'a> = dyn FnMut(u64, &mut [u8]) -> Result<(), Failure> + 'a;
type NtQueryInformationProcess =
    unsafe extern "system" fn(HANDLE, u32, *mut c_void, u32, *mut u32) -> i32;

// 动态查找已有 ntdll，不加载额外模块；保留原始 NTSTATUS，不转换为 HRESULT。
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetModuleHandleW"]
    fn get_module_handle(name: *const u16) -> *mut c_void;
    #[link_name = "GetProcAddress"]
    fn get_proc_address(
        module: *mut c_void,
        name: *const u8,
    ) -> Option<unsafe extern "system" fn() -> isize>;
}

#[derive(Debug, Serialize)]
pub(super) struct Observation {
    status: &'static str,
    code_matches: Option<bool>,
    iat_matches: Option<bool>,
    code_checks: Vec<CodeCheck>,
    iat_checks: Vec<IatCheck>,
    frame: FrameObservation,
    nt_query_status: Option<i32>,
    environment: Option<EnvironmentObservation>,
    failure: Option<Failure>,
}

#[derive(Debug, Serialize)]
struct CodeCheck {
    module_sha256: &'static str,
    rva: u64,
    length: usize,
    expected_sha256: &'static str,
    actual_sha256: Option<String>,
    matches: Option<bool>,
}

#[derive(Debug, Serialize)]
struct IatCheck {
    module_sha256: &'static str,
    slot_rva: u64,
    expected_module_sha256: &'static str,
    expected_rva: u64,
    actual_module_sha256: Option<String>,
    actual_rva: Option<u64>,
    target_kind: &'static str,
    matches: Option<bool>,
}

#[derive(Debug, Serialize)]
struct FrameObservation {
    status: &'static str,
    reason: &'static str,
    saved_query_status: Option<u32>,
    length_bytes: Option<u16>,
    maximum_length_bytes: Option<u16>,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct VariableObservation {
    key: &'static str,
    count: usize,
    utf16_units: Option<usize>,
    matches_expected: Option<bool>,
}

#[derive(Debug, Serialize)]
struct EnvironmentObservation {
    variables: [VariableObservation; 3],
    observed_stable: bool,
    // 两次读取一致且指针未变，不代表其他线程被暂停或全进程环境原子性。
    process_atomicity_proven: bool,
}

fn rejected(reason: &'static str) -> Failure {
    Failure {
        operation: "temp_path_observation",
        reason,
        win32: None,
        hresult: None,
    }
}

fn address(base: u64, offset: u64, length: usize) -> Result<u64, Failure> {
    let start = base
        .checked_add(offset)
        .ok_or_else(|| rejected("address_overflow"))?;
    if start < 0x1_0000
        || length == 0
        || start
            .checked_add(length as u64 - 1)
            .is_none_or(|end| end > MAX_USER_ADDRESS)
    {
        return Err(rejected("address_out_of_user_range"));
    }
    Ok(start)
}

fn pointer(value: u64, alignment: u64) -> Result<u64, Failure> {
    address(value, 0, 1)?;
    if value % alignment != 0 {
        return Err(rejected("unaligned_pointer"));
    }
    Ok(value)
}

fn read_u64(base: u64, offset: u64, read: &mut Reader<'_>) -> Result<u64, Failure> {
    let mut bytes = [0; 8];
    read(address(base, offset, bytes.len())?, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn unique_module<'a>(
    modules: &'a PreparedModules,
    sha: &str,
) -> Result<(usize, &'a Module), Failure> {
    let mut found = modules
        .modules
        .iter()
        .enumerate()
        .filter(|(_, module)| module.identity.sha256 == sha);
    let result = found
        .next()
        .ok_or_else(|| rejected("fixed_module_missing"))?;
    if found.next().is_some() {
        return Err(rejected("fixed_module_ambiguous"));
    }
    Ok(result)
}

fn module_address(module: &Module, rva: u64, size: usize) -> Result<u64, Failure> {
    if rva
        .checked_add(size as u64)
        .is_none_or(|end| end > u64::from(module.identity.size))
    {
        return Err(rejected("fixed_module_range_invalid"));
    }
    address(module.identity.base, rva, size)
}

fn inspect_frame(
    frames: &[Frame],
    kernelbase_index: usize,
    kernelbase: &Module,
    read: &mut Reader<'_>,
) -> Result<FrameObservation, Failure> {
    // 仅此返回点证明保存的状态已写入且栈布局未进入尾声。首次 allocate 和 query
    // 返回点尚不能保证该状态槽已初始化；也不把失效/正在释放的 Buffer 当作环境值。
    let pc = module_address(kernelbase, 0xef802, 1)?;
    let mut matches = frames.iter().filter(|frame| {
        frame.instruction == pc
            && frame.module_index == Some(kernelbase_index)
            && frame.module_offset == Some(0xef802)
    });
    let Some(frame) = matches.next() else {
        return Ok(FrameObservation {
            status: "skipped",
            reason: "trusted_retry_frame_unavailable",
            saved_query_status: None,
            length_bytes: None,
            maximum_length_bytes: None,
        });
    };
    if matches.next().is_some() {
        return Err(rejected("retry_frame_ambiguous"));
    }
    pointer(frame.stack, 8)?;
    let original_stack = frames
        .first()
        .ok_or_else(|| rejected("original_stack_missing"))?
        .stack;
    let stack_end = original_stack
        .checked_add(1024 * 1024)
        .ok_or_else(|| rejected("stack_range_overflow"))?;
    if frame.stack < original_stack
        || frame
            .stack
            .checked_add(0x34)
            .is_none_or(|end| end > stack_end)
    {
        return Err(rejected("retry_frame_outside_original_stack"));
    }
    let mut status = [0; 4];
    let mut lengths = [0; 4];
    read(address(frame.stack, 0x24, 4)?, &mut status)?;
    read(address(frame.stack, 0x30, 4)?, &mut lengths)?;
    Ok(FrameObservation {
        status: "complete",
        reason: "retry_free_return_only_not_first_query",
        saved_query_status: Some(u32::from_le_bytes(status)),
        length_bytes: Some(u16::from_le_bytes([lengths[0], lengths[1]])),
        maximum_length_bytes: Some(u16::from_le_bytes([lengths[2], lengths[3]])),
    })
}

fn basic_information_peb(
    status: i32,
    returned: u32,
    data: &[u64; 6],
    process_id: u32,
) -> Result<u64, Failure> {
    if status != 0 {
        return Err(rejected("NtQueryInformationProcess_failed"));
    }
    // 固定 x64 PROCESS_BASIC_INFORMATION 为 48 字节，PEB 在 +8，PID 在 +32。
    if returned != 48 || process_id == 0 || data[4] != u64::from(process_id) {
        return Err(rejected("process_basic_information_mismatch"));
    }
    pointer(data[1], 8)
}

fn query_peb(
    process: HANDLE,
    process_id: u32,
    observation: &mut Observation,
) -> Result<u64, Failure> {
    let name: Vec<u16> = "ntdll.dll\0".encode_utf16().collect();
    let module = unsafe { get_module_handle(name.as_ptr()) };
    if module.is_null() {
        return Err(Failure::last("GetModuleHandleW"));
    }
    let api = unsafe { get_proc_address(module, c"NtQueryInformationProcess".as_ptr().cast()) }
        .ok_or_else(|| Failure::last("GetProcAddress"))?;
    let query = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, NtQueryInformationProcess>(api)
    };
    let mut data = [0u64; 6];
    let mut returned = 0;
    let status = unsafe { query(process, 0, data.as_mut_ptr().cast(), 48, &mut returned) };
    observation.nt_query_status = Some(status);
    basic_information_peb(status, returned, &data, process_id)
}

fn environment_pointers(peb: u64, read: &mut Reader<'_>) -> Result<(u64, u64), Failure> {
    // 只用于上述固定 ntdll 的 x64 布局，不作为通用 Windows ABI 承诺。
    let parameters = pointer(read_u64(peb, 0x20, read)?, 8)?;
    let environment = pointer(read_u64(parameters, 0x80, read)?, 2)?;
    Ok((parameters, environment))
}

fn read_environment(base: u64, read: &mut Reader<'_>) -> Result<Vec<u16>, Failure> {
    let mut units = Vec::new();
    for offset in (0..MAX_ENV_BYTES).step_by(2) {
        // 不跨过双 NUL 读取相邻堆内容；取消及共享预算由每次读取回调检查。
        let mut bytes = [0; 2];
        read(address(base, offset as u64, 2)?, &mut bytes)?;
        let unit = u16::from_le_bytes(bytes);
        let ended = unit == 0 && units.last() == Some(&0);
        units.push(unit);
        if ended {
            return Ok(units);
        }
    }
    Err(rejected("environment_unterminated_or_over_limit"))
}

fn inspect_variables(
    units: &[u16],
    expected: &[Vec<u16>; 3],
) -> Result<[VariableObservation; 3], Failure> {
    if !units.ends_with(&[0, 0]) {
        return Err(rejected("environment_unterminated_or_over_limit"));
    }
    let mut result = KEYS.map(|key| VariableObservation {
        key,
        count: 0,
        utf16_units: None,
        matches_expected: None,
    });
    for entry in units[..units.len() - 1]
        .split(|unit| *unit == 0)
        .filter(|entry| !entry.is_empty())
    {
        // Windows 允许 =C:=... 隐藏项；只识别三个 ASCII 键，不记录其他键和值。
        let separator = entry
            .iter()
            .enumerate()
            .find(|(index, unit)| *index > 0 && **unit == u16::from(b'='))
            .map(|(index, _)| index)
            .ok_or_else(|| rejected("environment_entry_malformed"))?;
        let (key, value) = (&entry[..separator], &entry[separator + 1..]);
        for (index, known) in KEYS.iter().enumerate() {
            if key.len() == known.len()
                && key.iter().zip(known.bytes()).all(|(actual, expected)| {
                    u8::try_from(*actual).is_ok_and(|actual| actual.eq_ignore_ascii_case(&expected))
                })
            {
                let item = &mut result[index];
                item.count += 1;
                if item.count > 1 {
                    return Err(rejected("environment_target_key_duplicated"));
                }
                item.utf16_units = Some(value.len());
                item.matches_expected = Some(value == expected[index]);
            }
        }
    }
    Ok(result)
}

fn inspect_environment(
    peb: u64,
    expected: &[Vec<u16>; 3],
    read: &mut Reader<'_>,
) -> Result<EnvironmentObservation, Failure> {
    let before = environment_pointers(peb, read)?;
    let first = read_environment(before.1, read)?;
    let variables = inspect_variables(&first, expected)?;
    let second = read_environment(before.1, read)?;
    let after = environment_pointers(peb, read)?;
    if before != after || first != second {
        return Err(rejected("environment_changed_during_observation"));
    }
    Ok(EnvironmentObservation {
        variables,
        observed_stable: true,
        process_atomicity_proven: false,
    })
}

pub(super) fn observe(
    process: HANDLE,
    process_id: u32,
    modules: &PreparedModules,
    frames: &[Frame],
    expected: &[Vec<u16>; 3],
    read: &mut dyn FnMut(u64, &mut [u8]) -> Result<(), Failure>,
) -> Observation {
    let mut observation = Observation {
        status: "failed",
        code_matches: None,
        iat_matches: None,
        code_checks: Vec::new(),
        iat_checks: Vec::new(),
        frame: FrameObservation {
            status: "skipped",
            reason: "prerequisite_unverified",
            saved_query_status: None,
            length_bytes: None,
            maximum_length_bytes: None,
        },
        nt_query_status: None,
        environment: None,
        failure: None,
    };
    let result = (|| {
        if process.is_invalid()
            || expected
                .iter()
                .any(|value| value.contains(&0) || value.len() > MAX_ENV_BYTES / 2)
        {
            return Err(rejected("invalid_observation_input"));
        }
        let (kernelbase_index, kernelbase) = unique_module(modules, KERNELBASE_SHA)?;
        let (_, ntdll) = unique_module(modules, NTDLL_SHA)?;
        for ((module, module_sha256), (rva, length, sha)) in [
            (kernelbase, KERNELBASE_SHA),
            (ntdll, NTDLL_SHA),
            (ntdll, NTDLL_SHA),
        ]
        .into_iter()
        .zip(FUNCTIONS)
        {
            observation.code_checks.push(CodeCheck {
                module_sha256,
                rva,
                length,
                expected_sha256: sha,
                actual_sha256: None,
                matches: None,
            });
            let mut bytes = vec![0; length];
            read(module_address(module, rva, length)?, &mut bytes)?;
            let actual = format!("{:x}", Sha256::digest(&bytes));
            let check = observation
                .code_checks
                .last_mut()
                .expect("已追加当前核验项");
            check.matches = Some(actual == sha);
            check.actual_sha256 = Some(actual);
        }
        observation.code_matches = Some(
            observation
                .code_checks
                .iter()
                .all(|check| check.matches == Some(true)),
        );
        for (slot, target) in IAT {
            observation.iat_checks.push(IatCheck {
                module_sha256: KERNELBASE_SHA,
                slot_rva: slot,
                expected_module_sha256: NTDLL_SHA,
                expected_rva: target,
                actual_module_sha256: None,
                actual_rva: None,
                target_kind: "unread",
                matches: None,
            });
            let slot = module_address(kernelbase, slot, 8)?;
            let actual = read_u64(slot, 0, read)?;
            let check = observation.iat_checks.last_mut().expect("已追加当前核验项");
            check.matches = Some(actual == module_address(ntdll, target, 1)?);
            let mut known = modules.modules.iter().filter(|module| {
                actual >= module.identity.base
                    && actual - module.identity.base < u64::from(module.identity.size)
            });
            match (known.next(), known.next()) {
                (Some(module), None) => {
                    check.target_kind = "verified_module";
                    check.actual_module_sha256 = Some(module.identity.sha256.clone());
                    check.actual_rva = Some(actual - module.identity.base);
                }
                (None, None) | (Some(_), Some(_)) | (None, Some(_)) => {
                    check.target_kind = "unknown"
                }
            }
        }
        observation.iat_matches = Some(
            observation
                .iat_checks
                .iter()
                .all(|check| check.matches == Some(true)),
        );
        if observation.code_matches != Some(true) {
            return Err(rejected("fixed_function_bytes_changed"));
        }
        if observation.iat_matches != Some(true) {
            return Err(rejected("fixed_iat_target_changed"));
        }
        observation.frame = inspect_frame(frames, kernelbase_index, kernelbase, read)?;
        let peb = query_peb(process, process_id, &mut observation)?;
        observation.environment = Some(inspect_environment(peb, expected, read)?);
        Ok(())
    })();
    match result {
        Ok(()) => observation.status = "complete",
        Err(failure) => observation.failure = Some(failure),
    }
    observation
}

#[cfg(test)]
#[path = "managed_process_atomic_windows_snapshot_temp_tests.rs"]
mod tests;
