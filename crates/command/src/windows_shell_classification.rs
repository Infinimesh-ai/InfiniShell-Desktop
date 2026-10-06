//! 原 PowerShell 工作线程启动前后各一次 Shell32 分类返回观察，不参与映像授权或候选成功判定。
//!
//! 调用方独占调试事件，持续持有原 Job 和映像租约。本模块不等待或继续事件，不按
//! PID/TID 重开对象，不写代码。每次返回先恢复所有自有 DR，再交调用方读取 CLR 栈。

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Read as _};
use std::os::windows::io::{
    AsHandle as _, AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle,
};

use serde::Serialize;
use sha2::{Digest as _, Sha256};
use windows::Win32::Foundation::{
    DBG_REPLY_LATER, DUPLICATE_HANDLE_OPTIONS, DuplicateHandle, EXCEPTION_SINGLE_STEP, FILETIME,
    GENERIC_READ, HANDLE, NTSTATUS, WAIT_EVENT, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, GetFileInformationByHandle, ReOpenFile,
};
use windows::Win32::System::Diagnostics::Debug::{
    CONTEXT, CONTEXT_CONTROL_AMD64, CONTEXT_DEBUG_REGISTERS_AMD64, CONTEXT_INTEGER_AMD64,
    CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, DEBUG_EVENT, EXCEPTION_DEBUG_EVENT,
    EXIT_THREAD_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT, OUTPUT_DEBUG_STRING_EVENT,
    UNLOAD_DLL_DEBUG_EVENT,
};
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_IMAGE, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, VirtualQueryEx,
};
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_UNKNOWN,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetProcessId, GetProcessIdOfThread,
    GetProcessTimes, GetThreadId, GetThreadTimes, IsWow64Process2, PROCESS_QUERY_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_VM_READ, ResumeThread, SuspendThread, THREAD_GET_CONTEXT,
    THREAD_QUERY_INFORMATION, THREAD_SET_CONTEXT, THREAD_SUSPEND_RESUME, THREAD_SYNCHRONIZE,
    WaitForSingleObject,
};
use windows::core::BOOL;

use super::clr_reader::{ManagedContinuationPair, ManagedContinuationTarget};

const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_THREADS: usize = 64;
const MAX_PATH_UNITS: usize = 1024;
const MAX_CALLS: u32 = 1;
const CLR_EXCEPTION: u32 = 0xe0434352;
const MAX_USER: u64 = 0x0000_7fff_ffff_ffff;
const OWN_STATUS: u64 = 15;
// x64 DR6 非活动状态及 DR7 固定置一位；不把内核初始零视图作为硬件命中后的期望值。
const DR6_INACTIVE: u64 = 0xffff_0ff0;
const DR7_FIXED_ONE: u64 = 1 << 10;
const EFLAGS_FIXED_ONE: u32 = 1 << 1;
const RF: u32 = 1 << 16;
const TF: u32 = 1 << 8;

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetThreadContext"]
    fn get_context_raw(thread: HANDLE, context: *mut CONTEXT) -> BOOL;
    #[link_name = "SetThreadContext"]
    fn set_context_raw(thread: HANDLE, context: *const CONTEXT) -> BOOL;
    #[link_name = "ReadProcessMemory"]
    fn read_memory_raw(
        process: HANDLE,
        address: *const c_void,
        buffer: *mut c_void,
        length: usize,
        read: *mut usize,
    ) -> BOOL;
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::other(message)
}
fn require(value: bool, message: &'static str) -> io::Result<()> {
    if value { Ok(()) } else { Err(invalid(message)) }
}
fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}
fn duplicate(handle: HANDLE, access: u32) -> io::Result<OwnedHandle> {
    let current = unsafe { GetCurrentProcess() };
    let mut copied = HANDLE::default();
    unsafe {
        DuplicateHandle(
            current,
            handle,
            current,
            &mut copied,
            access,
            false,
            DUPLICATE_HANDLE_OPTIONS(0),
        )
    }
    .map_err(io::Error::from)?;
    Ok(unsafe { OwnedHandle::from_raw_handle(copied.0) })
}
fn require_signaled(result: io::Result<WAIT_EVENT>) -> io::Result<()> {
    match result? {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(io::Error::new(io::ErrorKind::WouldBlock, "原对象尚未退出")),
        status => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("原对象等待返回意外状态 {}", status.0),
        )),
    }
}
fn confirm_exit(handle: HANDLE) -> io::Result<()> {
    let status = unsafe { WaitForSingleObject(handle, 0) };
    // 仅 WAIT_FAILED 读取原错误码；未退出不能被包装成同一个笼统错误。
    require_signaled(if status == WAIT_FAILED {
        Err(io::Error::last_os_error())
    } else {
        Ok(status)
    })
}
fn birth(handle: HANDLE, thread: bool) -> io::Result<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    if thread {
        unsafe { GetThreadTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
    } else {
        unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
    }
    .map_err(io::Error::from)?;
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}
fn user_range(address: u64, length: usize) -> bool {
    address >= 0x10000
        && length != 0
        && address
            .checked_add(length as u64 - 1)
            .is_some_and(|end| end <= MAX_USER)
}
fn memory(process: HANDLE, address: u64, length: usize) -> io::Result<Vec<u8>> {
    require(
        user_range(address, length) && length <= 65536,
        "分类观察读取范围无效",
    )?;
    let mut bytes = vec![0; length];
    let mut read = 0;
    if !unsafe {
        read_memory_raw(
            process,
            address as *const c_void,
            bytes.as_mut_ptr().cast(),
            length,
            &mut read,
        )
    }
    .as_bool()
    {
        return Err(io::Error::last_os_error());
    }
    require(read == length, "分类观察读取不完整")?;
    Ok(bytes)
}
fn word(bytes: &[u8], offset: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| invalid("PE 短整数越界"))?
            .try_into()
            .unwrap(),
    ))
}
fn dword(bytes: &[u8], offset: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid("PE 整数越界"))?
            .try_into()
            .unwrap(),
    ))
}
fn qword(bytes: &[u8], offset: usize) -> io::Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or_else(|| invalid("地址整数越界"))?
            .try_into()
            .unwrap(),
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ObjectIdentity {
    pub process_id: u32,
    pub thread_id: u32,
    pub process_birth: u64,
    pub thread_birth: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReturnReceipt {
    pub generation: [u8; 16],
    pub identity: ObjectIdentity,
    pub pair_nonce: [u8; 16],
    pub entry_sequence: u64,
    pub return_sequence: u64,
    pub node_create_sequence: u64,
    pub node_process_id: u32,
    pub node_process_birth: u64,
    pub expected_node_matched: bool,
    pub flags: u32,
    pub raw_return_u64: u64,
    pub raw_return_low32: u32,
    pub return_region_kind: &'static str,
    pub registers_restored: bool,
    pub execution_context_unchanged: bool,
    pub post_start_clr_stack_required: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreNodeReturnReceipt {
    pub generation: [u8; 16],
    pub identity: ObjectIdentity,
    pub pair_nonce: [u8; 16],
    pub entry_sequence: u64,
    pub return_sequence: u64,
    pub expected_node_matched: bool,
    pub flags: u32,
    pub raw_return_u64: u64,
    pub raw_return_low32: u32,
    pub return_region_kind: &'static str,
    pub registers_restored: bool,
    pub execution_context_unchanged: bool,
    pub pre_node_clr_stack_required: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PostNodeExceptionReceipt {
    pub generation: [u8; 16],
    pub identity: ObjectIdentity,
    pub pre_return_sequence: u64,
    pub node_create_sequence: u64,
    pub node_process_id: u32,
    pub node_process_birth: u64,
    pub event_sequence: u64,
    pub exception_code: u32,
    pub first_chance: u32,
    pub parameter_count: u32,
    pub event_hresult: Option<u32>,
    pub reader_eligible: bool,
    pub registers_restored: bool,
    pub execution_context_unchanged: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreStartExceptionReceipt {
    pub generation: [u8; 16],
    pub identity: ObjectIdentity,
    pub pre_return_sequence: u64,
    pub initial_sequence: u64,
    pub event_sequence: u64,
    pub exception_code: u32,
    pub first_chance: u32,
    pub parameter_count: u32,
    pub event_hresult: Option<u32>,
    pub reader_eligible: bool,
    pub registers_restored: bool,
    pub execution_context_unchanged: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ManagedInitialReceipt {
    pub generation: [u8; 16],
    pub identity: ObjectIdentity,
    pub pre_return_sequence: u64,
    pub event_sequence: u64,
    pub registers_restored: bool,
    pub execution_context_unchanged: bool,
    pub managed_initial_clr_stack_required: bool,
    pub mapping: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct ManagedContinuationReceipt {
    pub generation: [u8; 16],
    pub identity: ObjectIdentity,
    pub pre_return_sequence: u64,
    pub node_create_sequence: u64,
    pub node_process_id: u32,
    pub node_process_birth: u64,
    pub event_sequence: u64,
    pub first_delivery_sequence: u64,
    pub deferred_for_node: bool,
    pub registers_restored: bool,
    pub execution_context_unchanged: bool,
    pub managed_continuation_clr_stack_required: bool,
    pub mapping: serde_json::Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct ManagedContinuationDeferredReceipt {
    pub generation: [u8; 16],
    pub identity: ObjectIdentity,
    pub pre_return_sequence: u64,
    pub first_delivery_sequence: u64,
    pub phase: &'static str,
    pub suspend_previous_count: u32,
    pub reply_later_confirmed: bool,
    pub node_create_sequence: Option<u64>,
    pub resume_previous_count: Option<u32>,
    pub replay_sequence: Option<u64>,
    pub suspend_owned: bool,
    pub original_thread_exit_confirmed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PostNodeDrSample {
    pub generation: [u8; 16],
    pub identity: ObjectIdentity,
    pub sequence: u64,
    pub node_create_sequence: u64,
    pub event_code: u32,
    pub scope: &'static str,
    pub dirty: bool,
    pub addresses_equal: Option<bool>,
    pub configuration_equal: Option<bool>,
    pub dr6_expected: Option<u64>,
    pub dr6_actual: u64,
    pub dr7_expected: Option<u64>,
    pub dr7_actual: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PathComparison {
    NotChecked,
    Matched,
    Mismatched,
    Unreadable,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkippedEntryReceipt {
    pub generation: [u8; 16],
    pub identity: ObjectIdentity,
    pub sequence: u64,
    pub node_create_sequence: Option<u64>,
    pub flags: u32,
    pub path_comparison: PathComparison,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub generation: [u8; 16],
    pub root_process_id: u32,
    pub selected_calls: u32,
    pub returned_calls: u32,
    pub pre_node_selected_calls: u32,
    pub pre_node_returned_calls: u32,
    pub post_node_root_stop_attempted: bool,
    pub post_node_root_stop: Option<PostNodeDrSample>,
    pub post_node_exception_selected_calls: u32,
    pub post_node_exception_resumed_calls: u32,
    pub post_node_exception: Option<PostNodeExceptionReceipt>,
    pub pre_start_exception_selected_calls: u32,
    pub pre_start_exception_resumed_calls: u32,
    pub pre_start_exception: Option<PreStartExceptionReceipt>,
    pub managed_initial_bound: bool,
    pub managed_initial_selected_calls: u32,
    pub managed_initial_resumed_calls: u32,
    pub managed_initial: Option<ManagedInitialReceipt>,
    pub managed_continuation_bound: bool,
    pub managed_continuation_selected_calls: u32,
    pub managed_continuation_resumed_calls: u32,
    pub managed_continuation_invalidated: bool,
    pub managed_continuation: Option<ManagedContinuationReceipt>,
    pub managed_continuation_deferred: Option<ManagedContinuationDeferredReceipt>,
    pub skipped_entries: u64,
    pub first_skipped_entry: Option<SkippedEntryReceipt>,
    pub dirty_threads: usize,
    pub restored_threads: u64,
    pub inactive_api_readbacks: u64,
    pub resume_flag_writes: u64,
    pub fixed_eflags_api_readbacks: u64,
    pub threads_exited_before_restore: u64,
    pub original_process_exit_confirmed: bool,
    pub stopped: bool,
    pub restoration: &'static str,
    pub result: &'static str,
    pub first_unowned_single_step: Option<serde_json::Value>,
}

#[derive(Debug)]
pub enum Observation {
    NotOwned,
    OwnedSkipped,
    OwnedPreNodeEntry,
    OwnedPreNodeReturn(PreNodeReturnReceipt),
    OwnedEntry,
    OwnedReturn(ReturnReceipt),
    OwnedManagedInitial(ManagedInitialReceipt),
    OwnedManagedContinuation(ManagedContinuationReceipt),
    OwnedManagedContinuationDeferred(ManagedContinuationDeferredReceipt),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Registers {
    address: [u64; 4],
    status: u64,
    control: u64,
}
impl Registers {
    fn read(context: &CONTEXT) -> Self {
        Self {
            address: [context.Dr0, context.Dr1, context.Dr2, context.Dr3],
            status: context.Dr6,
            control: context.Dr7,
        }
    }
    fn write(self, context: &mut CONTEXT) {
        [context.Dr0, context.Dr1, context.Dr2, context.Dr3] = self.address;
        context.Dr6 = self.status;
        context.Dr7 = self.control;
    }
    fn vacant(self, flags: u32) -> bool {
        self.address == [0; 4]
            && self.control & !(1 << 10) == 0
            && matches!(self.status, 0 | DR6_INACTIVE)
            && flags & TF == 0
    }
    fn inactive_api_view(self) -> Self {
        // Windows 在 Set 后把完整非活动位组读回为零，硬件单步则读回架构值。
        // 仅转换请求中的完整非活动位组；不掩掉实际 BLD/RTM 或其他调试原因。
        if self.status & !OWN_STATUS == DR6_INACTIVE {
            Self {
                status: self.status & OWN_STATUS,
                control: self.control & !DR7_FIXED_ONE,
                ..self
            }
        } else {
            self
        }
    }
    fn same_configuration(self, other: Self) -> bool {
        self.address == other.address
            && self.control == other.control
            && self.status & !OWN_STATUS == other.status & !OWN_STATUS
    }
    fn armed(self, address: u64, returning: bool) -> Self {
        let mut value = self;
        let slot = if returning { 3 } else { 0 };
        value.address[slot] = address;
        value.control |= DR7_FIXED_ONE | (1 << (slot * 2));
        value.status = (value.status | DR6_INACTIVE) & !OWN_STATUS;
        value
    }
    fn combined(self, classification: Option<(u64, bool)>, continuation: Option<u64>) -> Self {
        let mut value = match classification {
            Some((address, returning)) => self.armed(address, returning),
            None => self,
        };
        if let Some(address) = continuation {
            value.address[1] = address;
            value.control |= DR7_FIXED_ONE | (1 << 2);
            value.status = (value.status | DR6_INACTIVE) & !OWN_STATUS;
        }
        value
    }
}
fn owned_slot(expected: Registers, context: &CONTEXT, exception_address: u64) -> Option<usize> {
    let actual = Registers::read(context);
    let hit = actual.status & OWN_STATUS;
    if !actual.same_configuration(expected)
        || !hit.is_power_of_two()
        || context.EFlags & TF != 0
        || context.Rip != exception_address
    {
        return None;
    }
    let slot = hit.trailing_zeros() as usize;
    (expected.control & (1 << (slot * 2)) != 0 && expected.address[slot] == context.Rip)
        .then_some(slot)
}
fn restorable(
    current: Registers,
    original: Registers,
    expected: Option<Registers>,
    previous: Option<Registers>,
) -> bool {
    current == original
        || expected.is_some_and(|value| current.same_configuration(value))
        || previous.is_some_and(|value| current.same_configuration(value))
        || expected.is_some_and(|value| current.same_configuration(value.inactive_api_view()))
        || previous.is_some_and(|value| current.same_configuration(value.inactive_api_view()))
}
fn execution_equal(left: &CONTEXT, right: &CONTEXT) -> bool {
    [
        left.Rip, left.Rsp, left.Rax, left.Rcx, left.Rdx, left.Rbx, left.Rbp, left.Rsi, left.Rdi,
        left.R8, left.R9, left.R10, left.R11, left.R12, left.R13, left.R14, left.R15,
    ] == [
        right.Rip, right.Rsp, right.Rax, right.Rcx, right.Rdx, right.Rbx, right.Rbp, right.Rsi,
        right.Rdi, right.R8, right.R9, right.R10, right.R11, right.R12, right.R13, right.R14,
        right.R15,
    ] && left.EFlags == right.EFlags
}
fn fixed_eflags_api_readback(
    requested: &CONTEXT,
    actual: &CONTEXT,
    change_resume_flag: bool,
) -> bool {
    if !change_resume_flag || requested.EFlags & EFLAGS_FIXED_ONE == 0 {
        return false;
    }
    // EFLAGS bit 1 是架构保留常一位；原 Windows CONTROL 写入后的 API 视图将其读为零。
    // 只允许此次自有 RF 写入的这一方向；全部可变标志、RIP/RSP 和通用寄存器仍逐项相等。
    let mut expected_api = *requested;
    expected_api.EFlags ^= EFLAGS_FIXED_ONE;
    execution_equal(&expected_api, actual)
}
fn restore_own_resume_flag(current: &mut CONTEXT, introduced: Option<(u64, bool)>) {
    if let Some((rip, was_set)) = introduced {
        // 只撤销尚未执行的那条指令上由本观察器补入的 RF。
        if current.Rip == rip && !was_set {
            current.EFlags &= !RF;
        }
    }
}
#[repr(align(16))]
struct AlignedContext(CONTEXT);
fn context(handle: HANDLE) -> io::Result<AlignedContext> {
    let mut value = AlignedContext(CONTEXT {
        ContextFlags: CONTEXT_CONTROL_AMD64 | CONTEXT_INTEGER_AMD64 | CONTEXT_DEBUG_REGISTERS_AMD64,
        ..CONTEXT::default()
    });
    if !unsafe { get_context_raw(handle, &mut value.0) }.as_bool() {
        return Err(io::Error::last_os_error());
    }
    Ok(value)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReturnRegion {
    allocation: u64,
    base: u64,
    length: u64,
    kind: u32,
    protect: u32,
    bytes: Vec<u8>,
}
fn return_region(process: HANDLE, address: u64) -> io::Result<ReturnRegion> {
    return_region_with_sample_limit(process, address, 32)
}
fn return_region_with_sample_limit(
    process: HANDLE,
    address: u64,
    sample_limit: usize,
) -> io::Result<ReturnRegion> {
    require(user_range(address, 1), "返回地址无效")?;
    let mut info = MEMORY_BASIC_INFORMATION::default();
    let size = unsafe {
        VirtualQueryEx(
            process,
            Some(address as *const c_void),
            &mut info,
            std::mem::size_of_val(&info),
        )
    };
    require(size == std::mem::size_of_val(&info), "返回地址映射查询失败")?;
    require(
        info.State == MEM_COMMIT
            && (info.Type == MEM_IMAGE || info.Type == MEM_PRIVATE)
            && info.Protect.0 & (PAGE_GUARD.0 | PAGE_NOACCESS.0) == 0
            && matches!(info.Protect.0 & 255, value if value == PAGE_EXECUTE.0 || value == PAGE_EXECUTE_READ.0 || value == PAGE_EXECUTE_READWRITE.0 || value == PAGE_EXECUTE_WRITECOPY.0),
        "返回地址不是已提交可执行范围",
    )?;
    let base = info.BaseAddress as u64;
    let end = base
        .checked_add(info.RegionSize as u64)
        .ok_or_else(|| invalid("返回地址映射溢出"))?;
    require(base <= address && address < end, "返回地址不在查询范围")?;
    let bytes = memory(
        process,
        address,
        (end - address).min(sample_limit as u64) as usize,
    )?;
    Ok(ReturnRegion {
        allocation: info.AllocationBase as u64,
        base,
        length: info.RegionSize as u64,
        kind: info.Type.0,
        protect: info.Protect.0,
        bytes,
    })
}

fn continuation_identity(target: &ManagedContinuationTarget) -> ObjectIdentity {
    ObjectIdentity {
        process_id: target.process_id,
        thread_id: target.thread_id,
        process_birth: target.process_birth,
        thread_birth: target.thread_birth,
    }
}
fn continuation_binding_matches(
    target: &ManagedContinuationTarget,
    identity: ObjectIdentity,
    pre_sequence: u64,
) -> bool {
    let length = target.extent_end.checked_sub(target.extent_start);
    continuation_identity(target) == identity
        && identity.process_id != 0
        && identity.thread_id != 0
        && identity.process_birth != 0
        && identity.thread_birth != 0
        && target.pre_sequence == pre_sequence
        && pre_sequence != 0
        && target.pre_nonce != [0; 16]
        && target.module_mvid == target.spec.module_mvid
        && target.method_token == target.spec.method_token
        && target.spec.approved_il_offsets.contains(&target.il_offset)
        && (1..=256).contains(&target.map_count)
        && target.map_sha256 != [0; 32]
        && target.code_sha256 != [0; 32]
        && user_range(target.frame_rsp, 8)
        && target.frame_rsp % 8 == 0
        && length.is_some_and(|length| {
            (1..=65536).contains(&length) && user_range(target.extent_start, length as usize)
        })
        && target.extent_start <= target.address
        && target.address < target.extent_end
}
fn continuation_pair_matches(
    pair: &ManagedContinuationPair,
    identity: ObjectIdentity,
    sequence: u64,
) -> bool {
    let initial = &pair.initial;
    let continuation = &pair.continuation;
    continuation_binding_matches(initial, identity, sequence)
        && continuation_binding_matches(continuation, identity, sequence)
        && initial.pre_nonce == continuation.pre_nonce
        && initial.module_mvid == continuation.module_mvid
        && initial.method_token == continuation.method_token
        && initial.enc_version == continuation.enc_version
        && initial.frame_rsp == continuation.frame_rsp
        && initial.extent_start == continuation.extent_start
        && initial.extent_end == continuation.extent_end
        && initial.map_count == continuation.map_count
        && initial.map_sha256 == continuation.map_sha256
        && initial.code_sha256 == continuation.code_sha256
        && initial.il_offset != continuation.il_offset
        && initial.address != continuation.address
        && initial
            .spec
            .approved_il_offsets
            .iter()
            .all(|offset| !continuation.spec.approved_il_offsets.contains(offset))
}
fn continuation_regions(
    process: HANDLE,
    target: &ManagedContinuationTarget,
) -> io::Result<Vec<ReturnRegion>> {
    require(
        continuation_binding_matches(target, continuation_identity(target), target.pre_sequence),
        "托管续点私有映射票据无效",
    )?;
    let mut regions = Vec::new();
    let mut cursor = target.extent_start;
    while cursor < target.extent_end {
        require(regions.len() < 256, "托管续点可执行页数量超限")?;
        let mut region = return_region_with_sample_limit(
            process,
            cursor,
            (target.extent_end - cursor).min(32) as usize,
        )?;
        let next = region
            .base
            .checked_add(region.length)
            .ok_or_else(|| invalid("托管续点可执行页范围溢出"))?
            .min(target.extent_end);
        // VirtualQueryEx 的相邻同属性页可能随 JIT 提交而增长；只绑定原方法的完整范围。
        // 分配身份、类型、全部保护位及完整代码摘要仍严格核对，不接受范围内变化。
        region.base = cursor;
        region.length = next - cursor;
        cursor = next;
        regions.push(region);
    }
    let bytes = memory(
        process,
        target.extent_start,
        (target.extent_end - target.extent_start) as usize,
    )?;
    let hash: [u8; 32] = Sha256::digest(&bytes).into();
    require(hash == target.code_sha256, "托管续点完整代码摘要改变")?;
    Ok(regions)
}
fn continuation_hit_matches(
    target: &ManagedContinuationTarget,
    identity: ObjectIdentity,
    node: Option<(u64, u32, u64)>,
    sequence: u64,
    current: &CONTEXT,
) -> bool {
    continuation_frame_matches(target, identity, current)
        && node.is_some_and(|node| {
            target.pre_sequence < node.0 && node.0 < sequence && node.1 != 0 && node.2 != 0
        })
}
fn continuation_frame_matches(
    target: &ManagedContinuationTarget,
    identity: ObjectIdentity,
    current: &CONTEXT,
) -> bool {
    continuation_binding_matches(target, identity, target.pre_sequence)
        && current.Rip == target.address
        && current.Rsp == target.frame_rsp
}
struct ManagedContinuationLease {
    target: ManagedContinuationTarget,
    regions: Vec<ReturnRegion>,
}
impl ManagedContinuationLease {
    fn verify(&self, process: HANDLE) -> io::Result<()> {
        let current = continuation_regions(process, &self.target)?;
        if current != self.regions {
            // 仅报告映射等价性与长度，不泄露进程地址或读取的指令字节。
            let differences = self.regions.iter().zip(&current).enumerate()
                .filter(|(_, (expected, actual))| expected != actual)
                .map(|(index, (expected, actual))| serde_json::json!({
                    "index":index,"allocation_equal":expected.allocation==actual.allocation,
                    "base_equal":expected.base==actual.base,"expected_length":expected.length,
                    "actual_length":actual.length,"kind_equal":expected.kind==actual.kind,
                    "protect_equal":expected.protect==actual.protect,"bytes_equal":expected.bytes==actual.bytes,
                })).collect::<Vec<_>>();
            let safe = serde_json::json!({"expected_regions":self.regions.len(),
                "actual_regions":current.len(),"differences":differences});
            return Err(io::Error::other(format!("托管续点可执行映射改变：{safe}")));
        }
        Ok(())
    }
}

struct Image {
    file: File,
    bytes: Vec<u8>,
    base: u64,
    size: u32,
    entry: u64,
    entry_bytes: Vec<u8>,
}
#[derive(Clone, Copy)]
struct Section {
    va: u32,
    virtual_size: u32,
    raw: u32,
    raw_size: u32,
    flags: u32,
}
fn export(bytes: &[u8]) -> io::Result<(u32, u32, Vec<u8>)> {
    require(word(bytes, 0)? == 0x5a4d, "Shell32 DOS 标头无效")?;
    let pe = dword(bytes, 0x3c)? as usize;
    require(
        dword(bytes, pe)? == 0x4550
            && word(bytes, pe + 4)? == 0x8664
            && word(bytes, pe + 22)? & 0x2000 != 0,
        "Shell32 必须是 x64 DLL",
    )?;
    let count = word(bytes, pe + 6)? as usize;
    let optional = pe + 24;
    let optional_size = word(bytes, pe + 20)? as usize;
    require(
        (1..=96).contains(&count) && optional_size >= 120 && word(bytes, optional)? == 0x20b,
        "Shell32 PE 布局无效",
    )?;
    let image_size = dword(bytes, optional + 56)?;
    let headers = dword(bytes, optional + 60)?;
    require(
        image_size != 0 && image_size <= 1024 * 1024 * 1024 && dword(bytes, optional + 108)? != 0,
        "Shell32 映像范围无效",
    )?;
    let export_rva = dword(bytes, optional + 112)?;
    let export_size = dword(bytes, optional + 116)?;
    let mut sections = Vec::new();
    for index in 0..count {
        let at = optional + optional_size + index * 40;
        let section = Section {
            virtual_size: dword(bytes, at + 8)?,
            va: dword(bytes, at + 12)?,
            raw_size: dword(bytes, at + 16)?,
            raw: dword(bytes, at + 20)?,
            flags: dword(bytes, at + 36)?,
        };
        require(
            section
                .va
                .checked_add(section.virtual_size.max(section.raw_size))
                .is_some_and(|end| end <= image_size),
            "Shell32 节范围无效",
        )?;
        sections.push(section);
    }
    let range = |rva: u32, length: usize| -> io::Result<&[u8]> {
        let end = u64::from(rva) + length as u64;
        if end <= u64::from(headers) {
            return bytes
                .get(rva as usize..end as usize)
                .ok_or_else(|| invalid("PE 标头读取越界"));
        }
        let found: Vec<_> = sections
            .iter()
            .filter(|s| rva >= s.va && end <= u64::from(s.va) + u64::from(s.raw_size))
            .collect();
        require(found.len() == 1, "PE RVA 缺失或重叠")?;
        let start = u64::from(found[0].raw) + u64::from(rva - found[0].va);
        bytes
            .get(start as usize..(start + length as u64) as usize)
            .ok_or_else(|| invalid("PE RVA 超出文件"))
    };
    require(
        export_size >= 40
            && export_rva
                .checked_add(export_size)
                .is_some_and(|end| end <= image_size),
        "导出目录范围无效",
    )?;
    let table = range(export_rva, 40)?;
    let functions = dword(table, 20)? as usize;
    let names = dword(table, 24)? as usize;
    require(
        functions > 0 && functions <= 65536 && names > 0 && names <= 65536,
        "导出数量超限",
    )?;
    let addresses = range(dword(table, 28)?, functions * 4)?;
    let name_table = range(dword(table, 32)?, names * 4)?;
    let ordinals = range(dword(table, 36)?, names * 2)?;
    let mut selected = None;
    for index in 0..names {
        let rva = dword(name_table, index * 4)?;
        let mut name = Vec::new();
        for offset in 0..128u32 {
            let byte = range(
                rva.checked_add(offset)
                    .ok_or_else(|| invalid("导出名称溢出"))?,
                1,
            )?[0];
            if byte == 0 {
                break;
            }
            name.push(byte);
        }
        if name != b"SHGetFileInfoW" {
            continue;
        }
        require(selected.is_none(), "SHGetFileInfoW 导出重复")?;
        let ordinal = word(ordinals, index * 2)? as usize;
        require(ordinal < functions, "导出序号越界")?;
        let entry = dword(addresses, ordinal * 4)?;
        require(
            !(export_rva..export_rva + export_size).contains(&entry),
            "不猜测转发导出",
        )?;
        require(
            sections.iter().any(|s| {
                s.flags & 0x2000_0000 != 0
                    && entry >= s.va
                    && u64::from(entry) + 16 <= u64::from(s.va) + u64::from(s.raw_size)
            }),
            "导出入口不在可执行文件节",
        )?;
        selected = Some((entry, range(entry, 16)?.to_vec()));
    }
    let (entry, entry_bytes) = selected.ok_or_else(|| invalid("SHGetFileInfoW 导出缺失"))?;
    Ok((image_size, entry, entry_bytes))
}
fn file_identity(handle: HANDLE) -> io::Result<(u32, u32, u32, u64)> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle, &mut info) }.map_err(io::Error::from)?;
    require(
        info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0) == 0,
        "Shell32 文件类型无效",
    )?;
    Ok((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
        (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
    ))
}
impl Image {
    fn bind(event: &DEBUG_EVENT, file: &File, expected: [u8; 32]) -> io::Result<Self> {
        let info = unsafe { event.u.LoadDll };
        let original = file_identity(info.hFile)?;
        require(
            file_identity(HANDLE(file.as_raw_handle()))? == original
                && original.3 > 0
                && original.3 <= MAX_FILE as u64,
            "Shell32 原事件文件身份不符",
        )?;
        let reopened = unsafe {
            ReOpenFile(
                HANDLE(file.as_raw_handle()),
                GENERIC_READ.0,
                FILE_SHARE_READ,
                FILE_FLAGS_AND_ATTRIBUTES(0),
            )
        }
        .map_err(io::Error::from)?;
        let mut held = unsafe { File::from_raw_handle(reopened.0) };
        require(
            file_identity(reopened)? == original,
            "Shell32 拒写租约身份不符",
        )?;
        let mut bytes = Vec::new();
        (&mut held)
            .take(MAX_FILE as u64 + 1)
            .read_to_end(&mut bytes)?;
        require(
            bytes.len() as u64 == original.3
                && <[u8; 32]>::from(Sha256::digest(&bytes)) == expected
                && file_identity(reopened)? == original,
            "Shell32 字节摘要不符",
        )?;
        let (size, rva, entry_bytes) = export(&bytes)?;
        let base = info.lpBaseOfDll as u64;
        require(user_range(base, size as usize), "Shell32 映射基址无效")?;
        Ok(Self {
            file: held,
            bytes,
            base,
            size,
            entry: base + u64::from(rva),
            entry_bytes,
        })
    }
    fn verify_entry(&self, process: HANDLE) -> io::Result<()> {
        let region = return_region(process, self.entry)?;
        require(
            region.kind == MEM_IMAGE.0
                && region.allocation == self.base
                && self.entry < self.base + u64::from(self.size)
                && memory(process, self.entry, self.entry_bytes.len())? == self.entry_bytes,
            "Shell32 已加载入口不符",
        )?;
        require(
            file_identity(HANDLE(self.file.as_raw_handle()))?.3 == self.bytes.len() as u64,
            "Shell32 租约改变",
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CallPhase {
    PreNode,
    PostNode { node_create_sequence: u64 },
}
fn select_phase(
    node: Option<(u64, u32, u64)>,
    sequence: u64,
    flags: u32,
    pre_selected: u32,
    post_selected: u32,
) -> Option<CallPhase> {
    if flags != 0x2000 || sequence == 0 {
        return None;
    }
    match node {
        None => (pre_selected < MAX_CALLS).then_some(CallPhase::PreNode),
        Some((created, _, _)) => (created != 0 && created < sequence && post_selected < MAX_CALLS)
            .then_some(CallPhase::PostNode {
                node_create_sequence: created,
            }),
    }
}
struct Pending {
    stack: u64,
    returned_to: u64,
    region: ReturnRegion,
    sequence: u64,
    nonce: [u8; 16],
    phase: CallPhase,
}
fn return_phase_matches(pending: &Pending, node: Option<(u64, u32, u64)>, sequence: u64) -> bool {
    pending.sequence < sequence
        && match (pending.phase, node) {
            (CallPhase::PreNode, None) => true,
            (
                CallPhase::PostNode {
                    node_create_sequence,
                },
                Some((created, _, _)),
            ) => created == node_create_sequence && created != 0 && created < pending.sequence,
            (CallPhase::PreNode, Some(_)) | (CallPhase::PostNode { .. }, None) => false,
        }
}
struct PreNodeResume {
    sequence: u64,
    identity: ObjectIdentity,
    context: CONTEXT,
}

struct OriginalExceptionStop {
    event: DEBUG_EVENT,
    sequence: u64,
}
struct PostNodeExceptionResume {
    stop: OriginalExceptionStop,
    identity: ObjectIdentity,
    context: CONTEXT,
}
struct DeferredManagedContinuation {
    stop: OriginalExceptionStop,
    context: CONTEXT,
    receipt: ManagedContinuationDeferredReceipt,
}
fn deferred_ack_matches(
    event: &DEBUG_EVENT,
    sequence: u64,
    status: NTSTATUS,
    deferred: &DeferredManagedContinuation,
) -> bool {
    status == DBG_REPLY_LATER
        && deferred.receipt.phase == "suspended_before_continue"
        && deferred.receipt.suspend_owned
        && !deferred.receipt.reply_later_confirmed
        && deferred.receipt.suspend_previous_count == 0
        && same_exception_stop(event, sequence, &deferred.stop)
}
fn deferred_replay_matches(
    event: &DEBUG_EVENT,
    sequence: u64,
    node_sequence: u64,
    current: &CONTEXT,
    deferred: &DeferredManagedContinuation,
) -> bool {
    deferred.receipt.phase == "awaiting_replay"
        && !deferred.receipt.suspend_owned
        && deferred.receipt.reply_later_confirmed
        && deferred.receipt.resume_previous_count == Some(1)
        && deferred.receipt.node_create_sequence == Some(node_sequence)
        && deferred.stop.sequence < node_sequence
        && node_sequence < sequence
        && deferred.receipt.replay_sequence.is_none()
        // 比较原异常原件；首次递送与本次重放序号独立保存，不能改写原发生顺序。
        && same_exception_stop(event, deferred.stop.sequence, &deferred.stop)
        && execution_equal(current, &deferred.context)
        && Registers::read(current) == Registers::read(&deferred.context)
}
fn record_deferred_resume(
    receipt: &mut ManagedContinuationDeferredReceipt,
    previous: u32,
) -> io::Result<()> {
    require(previous != u32::MAX, "失败的 Resume 不能消费自有暂停")?;
    receipt.resume_previous_count = Some(previous);
    receipt.suspend_owned = false;
    require(previous == 1, "延后续点恢复前暂停计数不是唯一自有的一次")
}
fn same_exception_stop(event: &DEBUG_EVENT, sequence: u64, stop: &OriginalExceptionStop) -> bool {
    if event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT
        || stop.event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT
        || event.dwProcessId != stop.event.dwProcessId
        || event.dwThreadId != stop.event.dwThreadId
        || sequence != stop.sequence
    {
        return false;
    }
    let actual = unsafe { event.u.Exception };
    let expected = unsafe { stop.event.u.Exception };
    actual.dwFirstChance == expected.dwFirstChance
        && actual.ExceptionRecord.ExceptionCode == expected.ExceptionRecord.ExceptionCode
        && actual.ExceptionRecord.ExceptionFlags == expected.ExceptionRecord.ExceptionFlags
        && actual.ExceptionRecord.ExceptionRecord == expected.ExceptionRecord.ExceptionRecord
        && actual.ExceptionRecord.ExceptionAddress == expected.ExceptionRecord.ExceptionAddress
        && actual.ExceptionRecord.NumberParameters == expected.ExceptionRecord.NumberParameters
        && actual.ExceptionRecord.ExceptionInformation
            == expected.ExceptionRecord.ExceptionInformation
}
fn exception_metadata(event: &DEBUG_EVENT) -> (u32, u32, u32, Option<u32>, bool) {
    let exception = unsafe { event.u.Exception };
    let record = exception.ExceptionRecord;
    let code = record.ExceptionCode.0 as u32;
    let parameters = record.NumberParameters;
    let hresult = (1..=record.ExceptionInformation.len() as u32)
        .contains(&parameters)
        .then_some(record.ExceptionInformation[0] as u32);
    (
        code,
        exception.dwFirstChance,
        parameters,
        hresult,
        code == CLR_EXCEPTION && exception.dwFirstChance == 1 && hresult.is_some(),
    )
}
fn resume_event_matches(event: &DEBUG_EVENT, sequence: u64, ticket: &PreNodeResume) -> bool {
    if event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT
        || event.dwProcessId != ticket.identity.process_id
        || event.dwThreadId != ticket.identity.thread_id
        || sequence != ticket.sequence
    {
        return false;
    }
    let exception = unsafe { event.u.Exception };
    exception.dwFirstChance == 1
        && exception.ExceptionRecord.ExceptionCode == EXCEPTION_SINGLE_STEP
        && exception.ExceptionRecord.ExceptionAddress as u64 == ticket.context.Rip
}
fn live_root_sample_event(event: &DEBUG_EVENT, root_pid: u32) -> bool {
    event.dwProcessId == root_pid
        && event.dwThreadId != 0
        && matches!(
            event.dwDebugEventCode,
            EXCEPTION_DEBUG_EVENT
                | CREATE_THREAD_DEBUG_EVENT
                | LOAD_DLL_DEBUG_EVENT
                | UNLOAD_DLL_DEBUG_EVENT
                | OUTPUT_DEBUG_STRING_EVENT
        )
}
fn confirm_live(handle: HANDLE) -> io::Result<()> {
    let status = unsafe { WaitForSingleObject(handle, 0) };
    if status == WAIT_FAILED {
        return Err(io::Error::last_os_error());
    }
    require(status == WAIT_TIMEOUT, "采样原对象已退出或等待状态无效")
}
fn matching_return(pending: &Pending, value: &CONTEXT) -> bool {
    value.Rip == pending.returned_to && pending.stack.checked_add(8) == Some(value.Rsp)
}
fn nonce(generation: [u8; 16], sequence: u64) -> [u8; 16] {
    let mut value = generation;
    for (destination, byte) in value[8..].iter_mut().zip(sequence.to_le_bytes()) {
        *destination ^= byte;
    }
    value
}
fn exact_node(expected: &[u16], actual: &[u16]) -> bool {
    actual.len() == expected.len() + 1
        && actual.last() == Some(&0)
        && &actual[..expected.len()] == expected
}
fn compare_node_path(process: HANDLE, address: u64, expected: &[u16]) -> PathComparison {
    let mut actual = Vec::new();
    for (index, wanted) in expected
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .enumerate()
    {
        let Some(at) = address.checked_add((index * 2) as u64) else {
            return PathComparison::Unreadable;
        };
        let Ok(bytes) = memory(process, at, 2) else {
            return PathComparison::Unreadable;
        };
        let unit = u16::from_le_bytes([bytes[0], bytes[1]]);
        if unit != wanted {
            return PathComparison::Mismatched;
        }
        actual.push(unit);
    }
    if exact_node(expected, &actual) {
        PathComparison::Matched
    } else {
        PathComparison::Mismatched
    }
}
struct Thread {
    handle: OwnedHandle,
    identity: ObjectIdentity,
    original: Option<Registers>,
    expected: Option<Registers>,
    previous: Option<Registers>,
    dirty: bool,
    resume_flag: Option<(u64, bool)>,
    pending: Option<Pending>,
    inactive_api_readbacks: u64,
    resume_flag_writes: u64,
    fixed_eflags_api_readbacks: u64,
}
impl Thread {
    fn verify(&self, process: HANDLE) -> io::Result<()> {
        require(
            unsafe { GetProcessId(process) } == self.identity.process_id
                && birth(process, false)? == self.identity.process_birth
                && unsafe { GetThreadId(raw(&self.handle)) } == self.identity.thread_id
                && unsafe { GetProcessIdOfThread(raw(&self.handle)) } == self.identity.process_id
                && birth(raw(&self.handle), true)? == self.identity.thread_birth,
            "原线程身份改变",
        )
    }
    fn change(
        &mut self,
        mut current: AlignedContext,
        registers: Registers,
        change_resume_flag: bool,
    ) -> io::Result<()> {
        self.previous = self.expected;
        self.expected = Some(registers);
        self.dirty = true;
        registers.write(&mut current.0);
        // 普通断点只写 DR；重写无关 CONTROL 会让内核规范化原 EFlags。
        // 仅显式调整自有 RF 时才写 CONTROL，执行上下文仍须严格读回。
        current.0.ContextFlags = if change_resume_flag {
            CONTEXT_DEBUG_REGISTERS_AMD64 | CONTEXT_CONTROL_AMD64
        } else {
            CONTEXT_DEBUG_REGISTERS_AMD64
        };
        if !unsafe { set_context_raw(raw(&self.handle), &current.0) }.as_bool() {
            return Err(io::Error::last_os_error());
        }
        let after = context(raw(&self.handle))?;
        let actual = Registers::read(&after.0);
        let fixed_flags_view = fixed_eflags_api_readback(&current.0, &after.0, change_resume_flag);
        if (actual != registers && actual != registers.inactive_api_view())
            || !(execution_equal(&current.0, &after.0) || fixed_flags_view)
        {
            // 只保留控制位及逐项相等性，不输出目标地址或通用寄存器内容。
            return Err(io::Error::other(format!(
                "调试寄存器写入读回不符：addresses_equal={} dr6_expected={:#x} dr6_actual={:#x} dr7_expected={:#x} dr7_actual={:#x} execution_equal={} eflags_expected={:#x} eflags_actual={:#x}",
                actual.address == registers.address,
                registers.status,
                actual.status,
                registers.control,
                actual.control,
                execution_equal(&current.0, &after.0),
                current.0.EFlags,
                after.0.EFlags,
            )));
        }
        self.inactive_api_readbacks += u64::from(actual != registers);
        self.resume_flag_writes += u64::from(change_resume_flag);
        self.fixed_eflags_api_readbacks += u64::from(fixed_flags_view);
        Ok(())
    }
    fn restore(&mut self) -> io::Result<bool> {
        if !self.dirty {
            return Ok(false);
        }
        let mut current = context(raw(&self.handle))?;
        let original = self.original.ok_or_else(|| invalid("原 DR 配置缺失"))?;
        require(
            restorable(
                Registers::read(&current.0),
                original,
                self.expected,
                self.previous,
            ),
            "拒绝覆盖外部调试配置",
        )?;
        let original_flags = current.0.EFlags;
        restore_own_resume_flag(&mut current.0, self.resume_flag);
        let change_resume_flag = original_flags != current.0.EFlags;
        self.change(current, original, change_resume_flag)?;
        self.dirty = false;
        self.pending = None;
        self.resume_flag = None;
        Ok(true)
    }
}

/// 活线程存在自有修改时不能丢弃；必须恢复读回或核原进程及线程句柄均已退出。
#[must_use]
pub struct ShellClassificationWitness {
    process: OwnedHandle,
    process_birth: u64,
    root_pid: u32,
    debugger_tid: u32,
    generation: [u8; 16],
    expected_node: Vec<u16>,
    threads: BTreeMap<u32, Thread>,
    image: Option<Image>,
    node: Option<(u64, u32, u64)>,
    last_sequence: u64,
    selected: u32,
    returned: u32,
    pre_selected: u32,
    pre_returned: u32,
    pre_resume: Option<PreNodeResume>,
    pre_return_binding: Option<(ObjectIdentity, u64)>,
    post_exception_candidate: Option<OriginalExceptionStop>,
    post_exception_resume: Option<PostNodeExceptionResume>,
    post_exception_selected: u32,
    post_exception_resumed: u32,
    post_exception: Option<PostNodeExceptionReceipt>,
    pre_exception_resume: Option<PostNodeExceptionResume>,
    pre_exception_selected: u32,
    pre_exception_resumed: u32,
    pre_exception: Option<PreStartExceptionReceipt>,
    initial: Option<ManagedContinuationLease>,
    initial_bound: bool,
    initial_selected: u32,
    initial_resumed: u32,
    initial_receipt: Option<ManagedInitialReceipt>,
    initial_resume: Option<PostNodeExceptionResume>,
    managed: Option<ManagedContinuationLease>,
    managed_bound: bool,
    managed_selected: u32,
    managed_resumed: u32,
    managed_invalidated: bool,
    managed_receipt: Option<ManagedContinuationReceipt>,
    managed_resume: Option<PostNodeExceptionResume>,
    post_return_resume: Option<PostNodeExceptionResume>,
    managed_deferred: Option<DeferredManagedContinuation>,
    post_node_sample_attempted: bool,
    post_node_root_stop: Option<PostNodeDrSample>,
    skipped: u64,
    first_skipped_entry: Option<SkippedEntryReceipt>,
    restored: u64,
    exited_dirty: u64,
    exited: bool,
    stopped: bool,
    first_unowned_single_step: Option<serde_json::Value>,
    completed_inactive_api_readbacks: u64,
    completed_resume_flag_writes: u64,
    completed_fixed_eflags_api_readbacks: u64,
}
impl std::fmt::Debug for ShellClassificationWitness {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.summary().fmt(formatter)
    }
}
impl ShellClassificationWitness {
    fn invalidate_stop_tickets(&mut self) {
        self.pre_resume = None;
        self.post_exception_candidate = None;
        self.post_exception_resume = None;
        // 读后恢复之外的操作越过了原停点，不能留下可在其他停点复活的租约。
        let abandoned_managed = self.managed_resume.take().is_some();
        let abandoned_initial = self.initial_resume.take().is_some();
        let abandoned_pre_exception = self.pre_exception_resume.take().is_some();
        let abandoned_post = self.post_return_resume.take().is_some();
        if abandoned_managed || abandoned_post || abandoned_initial || abandoned_pre_exception {
            self.retire_continuation();
        }
    }
    fn retire_continuation(&mut self) {
        let had_lease = self.managed.take().is_some();
        let had_initial = self.initial.take().is_some();
        if had_lease || had_initial || (self.managed_bound && self.managed_resumed == 0) {
            self.managed_invalidated = true;
        }
        self.managed_resume = None;
        self.initial_resume = None;
        self.pre_exception_resume = None;
        self.post_return_resume = None;
    }
    fn active_continuation(&self) -> Option<&ManagedContinuationLease> {
        if self.initial_bound && self.initial_resumed == 0 {
            return self
                .initial
                .as_ref()
                .filter(|_| self.initial_selected == 0 && !self.managed_invalidated);
        }
        self.managed
            .as_ref()
            .filter(|_| self.managed_selected == 0 && !self.managed_invalidated)
    }
    /// 仅在 Continue 成功后以实际返回状态确认；重试不能冒充第二次原生 Continue。
    pub fn on_event_continued(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
        status: NTSTATUS,
    ) -> io::Result<()> {
        let Some(deferred) = &self.managed_deferred else {
            return Ok(());
        };
        if deferred.receipt.phase != "suspended_before_continue" {
            return Ok(());
        }
        let matched = deferred_ack_matches(event, sequence, status, deferred)
            && sequence == self.last_sequence
            && unsafe { GetCurrentThreadId() } == self.debugger_tid;
        if !matched {
            self.stopped = true;
            self.retire_continuation();
            return Err(invalid("托管续点延后继续未确认同一原事件及实际状态"));
        }
        let deferred = self.managed_deferred.as_mut().unwrap();
        deferred.receipt.reply_later_confirmed = true;
        deferred.receipt.phase = "awaiting_node";
        Ok(())
    }
    fn release_deferred_suspend(&mut self) -> io::Result<()> {
        let Some(deferred) = &self.managed_deferred else {
            return Ok(());
        };
        if !deferred.receipt.suspend_owned {
            return Ok(());
        }
        let identity = deferred.receipt.identity;
        let thread = self
            .threads
            .get(&identity.thread_id)
            .ok_or_else(|| invalid("延后续点的原暂停线程缺失"))?;
        thread.verify(raw(&self.process))?;
        require(thread.identity == identity, "延后续点暂停线程身份改变")?;
        let previous = unsafe { ResumeThread(raw(&thread.handle)) };
        if previous == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        // 任一非失败返回都已执行一次 Resume；即使计数异常也绝不能再次减原计数。
        let receipt = &mut self.managed_deferred.as_mut().unwrap().receipt;
        record_deferred_resume(receipt, previous)
    }
    fn release_deferred_after_node(&mut self, node_sequence: u64) -> io::Result<()> {
        let Some(deferred) = &self.managed_deferred else {
            return Ok(());
        };
        if deferred.receipt.phase == "cancelled_original_exit_required"
            || deferred.receipt.original_thread_exit_confirmed
        {
            return Ok(());
        }
        let lease = self
            .active_continuation()
            .ok_or_else(|| invalid("Node 创建时延后续点租约已经失效"))?;
        require(
            deferred.receipt.phase == "awaiting_node"
                && deferred.receipt.suspend_owned
                && deferred.receipt.reply_later_confirmed
                && deferred.stop.sequence < node_sequence
                && self.pre_return_binding
                    == Some((deferred.receipt.identity, lease.target.pre_sequence))
                && continuation_binding_matches(
                    &lease.target,
                    deferred.receipt.identity,
                    deferred.receipt.pre_return_sequence,
                ),
            "Node 创建不能释放未确认的延后续点",
        )?;
        confirm_live(raw(&self.process))?;
        let thread = self
            .threads
            .get(&deferred.receipt.identity.thread_id)
            .ok_or_else(|| invalid("Node 创建时原暂停线程缺失"))?;
        thread.verify(raw(&self.process))?;
        confirm_live(raw(&thread.handle))?;
        let current = context(raw(&thread.handle))?;
        require(
            thread.identity == deferred.receipt.identity
                && execution_equal(&current.0, &deferred.context)
                && Registers::read(&current.0) == Registers::read(&deferred.context),
            "Node 创建时原暂停上下文改变",
        )?;
        lease.verify(raw(&self.process))?;
        self.managed_deferred
            .as_mut()
            .unwrap()
            .receipt
            .node_create_sequence = Some(node_sequence);
        // 此处仅操作已显式暂停的原 worker；child CREATE 不是原 root 全线程停点。
        self.release_deferred_suspend()?;
        self.managed_deferred.as_mut().unwrap().receipt.phase = "awaiting_replay";
        Ok(())
    }
    /// 调用方已请求精确原 Job 终止；仅平衡自有暂停，原退出证明仍由既有排空路径完成。
    pub fn cancel_deferred_after_target_termination(&mut self) -> io::Result<()> {
        if self.managed_deferred.is_none() {
            return Ok(());
        }
        require(
            unsafe { GetCurrentThreadId() } == self.debugger_tid,
            "延后续点取消必须由原调试线程完成",
        )?;
        self.stopped = true;
        self.retire_continuation();
        let deferred = self.managed_deferred.as_mut().unwrap();
        if deferred.receipt.phase != "replayed" {
            deferred.receipt.phase = "cancelled_original_exit_required";
        }
        if !deferred.receipt.suspend_owned {
            return Ok(());
        }
        let identity = deferred.receipt.identity;
        let thread = self
            .threads
            .get_mut(&identity.thread_id)
            .ok_or_else(|| invalid("取消延后续点时原暂停线程缺失"))?;
        thread.verify(raw(&self.process))?;
        require(thread.identity == identity, "取消延后续点时线程身份改变")?;
        match confirm_exit(raw(&thread.handle)) {
            Ok(()) => {
                let receipt = &mut self.managed_deferred.as_mut().unwrap().receipt;
                receipt.suspend_owned = false;
                receipt.original_thread_exit_confirmed = true;
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error),
        }
        // 原 worker 仍持唯一自有暂停，可恢复它的 DR；不写其他正在运行的 root 线程。
        if thread.restore()? {
            self.restored += 1;
        }
        self.release_deferred_suspend()
    }
    pub fn new(
        event: &DEBUG_EVENT,
        process_birth: u64,
        generation: [u8; 16],
        expected_node: Vec<u16>,
    ) -> io::Result<Self> {
        require(
            event.dwDebugEventCode == CREATE_PROCESS_DEBUG_EVENT
                && event.dwProcessId != 0
                && event.dwProcessId != unsafe { GetCurrentProcessId() }
                && process_birth != 0
                && generation != [0; 16]
                && !expected_node.is_empty()
                && expected_node.len() <= MAX_PATH_UNITS
                && !expected_node.contains(&0),
            "原 root CREATE 或固定 Node 路径无效",
        )?;
        let created = unsafe { event.u.CreateProcessInfo };
        // 原调试句柄不保证等待权限；只向同一原对象请求读取、身份查询及退出核验。
        let process = duplicate(
            created.hProcess,
            (PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE).0,
        )?;
        require(
            unsafe { GetProcessId(raw(&process)) } == event.dwProcessId
                && birth(raw(&process), false)? == process_birth,
            "原 root 进程身份不符",
        )?;
        let (mut emulated, mut native) = (IMAGE_FILE_MACHINE_UNKNOWN, IMAGE_FILE_MACHINE_UNKNOWN);
        unsafe { IsWow64Process2(raw(&process), &mut emulated, Some(&mut native)) }
            .map_err(io::Error::from)?;
        require(
            emulated == IMAGE_FILE_MACHINE_UNKNOWN && native == IMAGE_FILE_MACHINE_AMD64,
            "分类观察仅支持原生 x64",
        )?;
        let mut value = Self {
            process,
            process_birth,
            root_pid: event.dwProcessId,
            debugger_tid: unsafe { GetCurrentThreadId() },
            generation,
            expected_node,
            threads: BTreeMap::new(),
            image: None,
            node: None,
            last_sequence: 0,
            selected: 0,
            returned: 0,
            pre_selected: 0,
            pre_returned: 0,
            pre_resume: None,
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
            restored: 0,
            exited_dirty: 0,
            exited: false,
            stopped: false,
            first_unowned_single_step: None,
            completed_inactive_api_readbacks: 0,
            completed_resume_flag_writes: 0,
            completed_fixed_eflags_api_readbacks: 0,
        };
        value.insert_thread(event.dwThreadId, created.hThread)?;
        Ok(value)
    }
    fn stopped(&self, event: &DEBUG_EVENT) -> io::Result<()> {
        require(
            !self.exited
                && unsafe { GetCurrentThreadId() } == self.debugger_tid
                && event.dwProcessId == self.root_pid
                && event.dwThreadId != 0,
            "必须持有原 root 调试停点",
        )?;
        require(
            unsafe { GetProcessId(raw(&self.process)) } == self.root_pid
                && birth(raw(&self.process), false)? == self.process_birth,
            "root 进程身份改变",
        )
    }
    fn insert_thread(&mut self, tid: u32, original: HANDLE) -> io::Result<()> {
        require(
            tid != 0 && !self.threads.contains_key(&tid) && self.threads.len() < MAX_THREADS,
            "原线程重复或数量超限",
        )?;
        let handle = duplicate(
            original,
            (THREAD_GET_CONTEXT
                | THREAD_SET_CONTEXT
                | THREAD_QUERY_INFORMATION
                | THREAD_SUSPEND_RESUME
                | THREAD_SYNCHRONIZE)
                .0,
        )?;
        let identity = ObjectIdentity {
            process_id: self.root_pid,
            thread_id: tid,
            process_birth: self.process_birth,
            thread_birth: birth(raw(&handle), true)?,
        };
        let thread = Thread {
            handle,
            identity,
            original: None,
            expected: None,
            previous: None,
            dirty: false,
            resume_flag: None,
            pending: None,
            inactive_api_readbacks: 0,
            resume_flag_writes: 0,
            fixed_eflags_api_readbacks: 0,
        };
        thread.verify(raw(&self.process))?;
        self.threads.insert(tid, thread);
        Ok(())
    }
    pub fn retain_thread(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        self.invalidate_stop_tickets();
        self.stopped(event)?;
        require(
            event.dwDebugEventCode == CREATE_THREAD_DEBUG_EVENT,
            "必须使用原 CREATE_THREAD",
        )?;
        self.insert_thread(event.dwThreadId, unsafe { event.u.CreateThread.hThread })
    }
    pub fn bind_shell32(
        &mut self,
        event: &DEBUG_EVENT,
        file: &File,
        expected_sha256: [u8; 32],
    ) -> io::Result<()> {
        self.invalidate_stop_tickets();
        self.stopped(event)?;
        require(
            event.dwDebugEventCode == LOAD_DLL_DEBUG_EVENT && self.image.is_none() && !self.stopped,
            "Shell32 必须绑定一次原 LOAD",
        )?;
        let image = Image::bind(event, file, expected_sha256)?;
        image.verify_entry(raw(&self.process))?;
        self.image = Some(image);
        Ok(())
    }
    pub fn arm_all(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        self.invalidate_stop_tickets();
        let result = self.arm_all_at_stop(event);
        if result.is_err() {
            self.stopped = true;
            self.retire_continuation();
        }
        result
    }
    fn arm_all_at_stop(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        self.stopped(event)?;
        if self.stopped {
            return Ok(());
        }
        let Some(image) = self.image.as_ref() else {
            return Ok(());
        };
        image.verify_entry(raw(&self.process))?;
        let classification = (self.selected < MAX_CALLS).then_some((image.entry, false));
        let continuation = if let Some(lease) = self.active_continuation() {
            lease.verify(raw(&self.process))?;
            let identity = continuation_identity(&lease.target);
            let worker = self
                .threads
                .get(&identity.thread_id)
                .ok_or_else(|| invalid("托管续点原工作线程缺失"))?;
            worker.verify(raw(&self.process))?;
            confirm_live(raw(&worker.handle))?;
            require(worker.identity == identity, "托管续点原线程身份改变")?;
            Some((identity, lease.target.address))
        } else {
            None
        };
        for thread in self.threads.values_mut() {
            thread.verify(raw(&self.process))?;
            if thread.dirty {
                continue;
            }
            let continuation = continuation
                .filter(|(identity, _)| *identity == thread.identity)
                .map(|(_, address)| address);
            if classification.is_none() && continuation.is_none() {
                continue;
            }
            let current = context(raw(&thread.handle))?;
            let original = Registers::read(&current.0);
            require(
                original.vacant(current.0.EFlags),
                "拒绝覆盖已有调试寄存器配置",
            )?;
            thread.original = Some(original);
            thread.change(
                current,
                original.combined(classification, continuation),
                false,
            )?;
        }
        Ok(())
    }
    /// 调用方须先确认同停点 CLR reader 已退出且 Job 清空；本模块不负责继续事件。
    pub fn resume_after_pre_node_return(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<()> {
        self.resume_pre_node_return(event, sequence, None, None)
    }
    /// 私有地址票据只能在产生它的 pre 返回停点、reader 完全回收后移交。
    pub fn resume_after_pre_node_return_with_continuation(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
        target: ManagedContinuationTarget,
    ) -> io::Result<()> {
        self.resume_pre_node_return(event, sequence, Some(target), None)
    }
    /// 同一 pre reader 产生两张同帧票据；先安装首次启动前的点，不等待 Node。
    pub fn resume_after_pre_node_return_with_managed_pair(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
        pair: ManagedContinuationPair,
    ) -> io::Result<()> {
        let result = (|| {
            let ticket = self
                .pre_resume
                .as_ref()
                .ok_or_else(|| invalid("双点绑定缺少原 pre 停点"))?;
            require(
                continuation_pair_matches(&pair, ticket.identity, sequence),
                "双点票据必须来自同一原帧、代码映射及 pre 请求",
            )?;
            self.resume_pre_node_return(
                event,
                sequence,
                Some(pair.continuation),
                Some(pair.initial),
            )
        })();
        if result.is_err() {
            self.stopped = true;
            self.invalidate_stop_tickets();
            self.retire_continuation();
        }
        result
    }
    fn resume_pre_node_return(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
        target: Option<ManagedContinuationTarget>,
        initial: Option<ManagedContinuationTarget>,
    ) -> io::Result<()> {
        // 先消耗一次性票据；任一核验失败都不能通过重试清除停止状态。
        let ticket = self.pre_resume.take();
        self.invalidate_stop_tickets();
        let ticket = ticket.ok_or_else(|| invalid("没有可恢复的启动前返回停点"))?;
        self.stopped(event)?;
        require(
            self.stopped
                && self.node.is_none()
                && self.pre_selected == 1
                && self.pre_returned == 1
                && self.selected == 0
                && self.returned == 0
                && self.last_sequence == sequence
                && !self.requires_restoration()
                && resume_event_matches(event, sequence, &ticket),
            "启动前返回恢复事件或阶段不符",
        )?;
        confirm_live(raw(&self.process))?;
        let thread = self
            .threads
            .get(&event.dwThreadId)
            .ok_or_else(|| invalid("启动前返回原线程缺失"))?;
        thread.verify(raw(&self.process))?;
        confirm_live(raw(&thread.handle))?;
        let current = context(raw(&thread.handle))?;
        require(
            thread.identity == ticket.identity
                && execution_equal(&ticket.context, &current.0)
                && Registers::read(&ticket.context) == Registers::read(&current.0),
            "启动前返回恢复上下文改变",
        )?;
        require(self.image.is_some(), "启动前返回 Shell32 租约缺失")?;
        require(
            !self.initial_bound && self.initial.is_none(),
            "首次托管点不可重复绑定",
        )?;
        if let Some(target) = target {
            require(
                !self.managed_bound
                    && !self.managed_invalidated
                    && self.managed.is_none()
                    && self.managed_selected == 0
                    && continuation_binding_matches(&target, ticket.identity, sequence),
                "托管续点必须绑定同一 pre 原线程、序号和独立预算",
            )?;
            let regions = continuation_regions(raw(&self.process), &target)?;
            self.managed = Some(ManagedContinuationLease { target, regions });
            self.managed_bound = true;
        }
        if let Some(target) = initial {
            require(self.managed_bound, "首次托管点必须同时绑定后续点")?;
            let regions = continuation_regions(raw(&self.process), &target)?;
            self.initial = Some(ManagedContinuationLease { target, regions });
            self.initial_bound = true;
        }
        self.stopped = false;
        let result = self.arm_all(event);
        if result.is_err() {
            self.stopped = true;
        }
        result
    }
    /// 仅承接 observe 对同一原异常返回 NotOwned 的结果；不改变原异常继续语义。
    pub fn observe_post_node_exception(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<Option<PostNodeExceptionReceipt>> {
        let candidate = self.post_exception_candidate.take();
        self.invalidate_stop_tickets();
        let result = self.observe_post_node_exception_at_stop(event, sequence, candidate);
        if result.is_err() {
            self.stopped = true;
            self.invalidate_stop_tickets();
            self.retire_continuation();
        }
        result
    }
    fn observe_post_node_exception_at_stop(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
        candidate: Option<OriginalExceptionStop>,
    ) -> io::Result<Option<PostNodeExceptionReceipt>> {
        let Some((pre_identity, pre_sequence)) = self.pre_return_binding else {
            return Ok(None);
        };
        let Some(node) = self.node else {
            return Ok(None);
        };
        if self.stopped
            || self.post_exception_selected != 0
            || self.pre_returned != 1
            || self.selected != 0
            || self.threads.values().any(|thread| thread.pending.is_some())
            || event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT
            || event.dwProcessId != pre_identity.process_id
            || event.dwThreadId != pre_identity.thread_id
        {
            return Ok(None);
        }
        let stop = candidate.ok_or_else(|| invalid("启动后异常未经过同停点原观察"))?;
        require(
            pre_sequence < node.0
                && node.0 < sequence
                && sequence == self.last_sequence
                && same_exception_stop(event, sequence, &stop),
            "启动后异常原事件或顺序不符",
        )?;
        // 首次选中即消耗独立预算；类型不支持、身份失败和读取失败均不能追逐后续异常。
        self.post_exception_selected = 1;
        self.stopped(event)?;
        confirm_live(raw(&self.process))?;
        let thread = self
            .threads
            .get(&event.dwThreadId)
            .ok_or_else(|| invalid("启动后异常原工作线程缺失"))?;
        thread.verify(raw(&self.process))?;
        confirm_live(raw(&thread.handle))?;
        require(
            thread.identity == pre_identity,
            "启动后异常与启动前返回不是同一原工作线程",
        )?;
        let (exception_code, first_chance, parameter_count, event_hresult, reader_eligible) =
            exception_metadata(event);
        let mut receipt = PostNodeExceptionReceipt {
            generation: self.generation,
            identity: thread.identity,
            pre_return_sequence: pre_sequence,
            node_create_sequence: node.0,
            node_process_id: node.1,
            node_process_birth: node.2,
            event_sequence: sequence,
            exception_code,
            first_chance,
            parameter_count,
            event_hresult,
            reader_eligible,
            registers_restored: false,
            execution_context_unchanged: false,
        };
        self.post_exception = Some(receipt.clone());
        if !reader_eligible {
            return Ok(Some(receipt));
        }
        let current = context(raw(&thread.handle))?;
        self.stopped = true;
        self.withdraw_for_reader(event)?;
        let after = context(raw(&self.threads[&event.dwThreadId].handle))?;
        require(
            !self.requires_restoration() && execution_equal(&current.0, &after.0),
            "恢复 DR 后原异常执行上下文改变",
        )?;
        receipt.registers_restored = true;
        receipt.execution_context_unchanged = true;
        self.post_exception = Some(receipt.clone());
        self.post_exception_resume = Some(PostNodeExceptionResume {
            stop,
            identity: receipt.identity,
            context: after.0,
        });
        Ok(Some(receipt))
    }
    /// 调用方先回收本次异常 reader；仅此独立票据可在原异常停点重新布置入口 DR。
    pub fn resume_after_post_node_exception(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<()> {
        let result = self.resume_post_node_exception_at_stop(event, sequence);
        if result.is_err() {
            self.stopped = true;
            self.invalidate_stop_tickets();
            self.retire_continuation();
        }
        result
    }
    fn resume_post_node_exception_at_stop(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<()> {
        self.pre_resume = None;
        self.post_exception_candidate = None;
        let ticket = self
            .post_exception_resume
            .take()
            .ok_or_else(|| invalid("没有可恢复的启动后异常停点"))?;
        self.stopped(event)?;
        let receipt = self
            .post_exception
            .as_ref()
            .ok_or_else(|| invalid("启动后异常收据缺失"))?;
        require(
            self.stopped
                && self.post_exception_selected == 1
                && self.post_exception_resumed == 0
                && self.pre_returned == 1
                && self.selected == 0
                && self.returned == 0
                && self.last_sequence == sequence
                && !self.requires_restoration()
                && self.threads.values().all(|thread| thread.pending.is_none())
                && receipt.reader_eligible
                && receipt.registers_restored
                && receipt.execution_context_unchanged
                && receipt.event_sequence == sequence
                && receipt.identity == ticket.identity
                && self.pre_return_binding == Some((receipt.identity, receipt.pre_return_sequence))
                && self.node
                    == Some((
                        receipt.node_create_sequence,
                        receipt.node_process_id,
                        receipt.node_process_birth,
                    ))
                && same_exception_stop(event, sequence, &ticket.stop),
            "启动后异常恢复事件或阶段不符",
        )?;
        confirm_live(raw(&self.process))?;
        let thread = self
            .threads
            .get(&event.dwThreadId)
            .ok_or_else(|| invalid("启动后异常原线程缺失"))?;
        thread.verify(raw(&self.process))?;
        confirm_live(raw(&thread.handle))?;
        let current = context(raw(&thread.handle))?;
        require(
            thread.identity == ticket.identity
                && execution_equal(&ticket.context, &current.0)
                && Registers::read(&ticket.context) == Registers::read(&current.0),
            "启动后异常恢复上下文改变",
        )?;
        require(self.image.is_some(), "启动后异常 Shell32 租约缺失")?;
        self.stopped = false;
        let result = self.arm_all(event);
        if result.is_ok() {
            self.post_exception_resumed = 1;
        } else {
            self.stopped = true;
        }
        result
    }
    /// 首次点恢复全部 DR 后立即读；它没有 Node 身份，也不会触发延后重放。
    pub fn managed_initial_target(&self) -> io::Result<&ManagedContinuationTarget> {
        require(
            self.stopped
                && self.node.is_none()
                && self.initial_selected == 1
                && self.initial_resumed == 0
                && !self.managed_invalidated
                && !self.requires_restoration()
                && self
                    .initial_resume
                    .as_ref()
                    .is_some_and(|ticket| ticket.stop.sequence == self.last_sequence),
            "当前不是可读取的首次启动前托管停点",
        )?;
        self.initial
            .as_ref()
            .map(|lease| &lease.target)
            .ok_or_else(|| invalid("首次托管点私有租约缺失"))
    }
    /// 初始 op4 reader 完全回收后，撤销初始点并布置同帧后续点，正常继续原事件。
    pub fn resume_after_managed_initial(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<()> {
        let previously_resumed = self.initial_resumed;
        let ticket = self.initial_resume.take();
        self.invalidate_stop_tickets();
        let result = (|| {
            let ticket = ticket.ok_or_else(|| invalid("没有可恢复的首次托管停点"))?;
            self.verify_restored_stop(event, sequence, &ticket)?;
            let receipt = self
                .initial_receipt
                .as_ref()
                .ok_or_else(|| invalid("首次托管点收据缺失"))?;
            let initial = self
                .initial
                .as_ref()
                .ok_or_else(|| invalid("首次托管点租约缺失"))?;
            let continuation = self
                .managed
                .as_ref()
                .ok_or_else(|| invalid("同帧后续点租约缺失"))?;
            require(
                self.initial_selected == 1
                    && self.initial_resumed == 0
                    && self.managed_selected == 0
                    && self.managed_resumed == 0
                    && !self.managed_invalidated
                    && self.node.is_none()
                    && receipt.event_sequence == sequence
                    && receipt.identity == ticket.identity
                    && receipt.registers_restored
                    && receipt.execution_context_unchanged
                    && self.pre_return_binding
                        == Some((ticket.identity, receipt.pre_return_sequence))
                    && continuation_binding_matches(
                        &initial.target,
                        ticket.identity,
                        receipt.pre_return_sequence,
                    )
                    && continuation_binding_matches(
                        &continuation.target,
                        ticket.identity,
                        receipt.pre_return_sequence,
                    ),
                "首次托管点恢复顺序、身份或独立预算不符",
            )?;
            initial.verify(raw(&self.process))?;
            continuation.verify(raw(&self.process))?;
            require(self.image.is_some(), "首次托管点恢复 Shell32 租约缺失")?;
            self.initial = None;
            self.initial_resumed = 1;
            self.stopped = false;
            self.arm_all(event)
        })();
        if result.is_err() {
            self.initial_resumed = previously_resumed;
            self.stopped = true;
            self.retire_continuation();
        }
        result
    }
    /// 仅选择初始读取恢复后、Node 创建前同一原工作线程的首个 CLR 异常。
    pub fn observe_pre_start_exception(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<Option<PreStartExceptionReceipt>> {
        let result = (|| {
            let Some(initial) = self.initial_receipt.as_ref() else {
                return Ok(None);
            };
            if event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT {
                return Ok(None);
            }
            let (exception_code, first_chance, parameter_count, event_hresult, reader_eligible) =
                exception_metadata(event);
            if self.stopped
                || self.node.is_some()
                || self.initial_resumed != 1
                || self.pre_exception_selected != 0
                || self.pre_returned != 1
                || self.selected != 0
                || self.threads.values().any(|thread| thread.pending.is_some())
                || event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT
                || event.dwProcessId != initial.identity.process_id
                || event.dwThreadId != initial.identity.thread_id
                || exception_code != CLR_EXCEPTION
            {
                return Ok(None);
            }
            let stop = self
                .post_exception_candidate
                .take()
                .ok_or_else(|| invalid("启动前异常未经过同停点原观察"))?;
            require(
                initial.pre_return_sequence < initial.event_sequence
                    && initial.event_sequence < sequence
                    && sequence == self.last_sequence
                    && self.pre_return_binding
                        == Some((initial.identity, initial.pre_return_sequence))
                    && same_exception_stop(event, sequence, &stop),
                "启动前异常原事件或初始读取顺序不符",
            )?;
            // 首个 CLR 事件即消费预算；即使元数据不支持也不能追逐另一个异常。
            self.pre_exception_selected = 1;
            self.stopped(event)?;
            confirm_live(raw(&self.process))?;
            let thread = self
                .threads
                .get(&event.dwThreadId)
                .ok_or_else(|| invalid("启动前异常原线程缺失"))?;
            thread.verify(raw(&self.process))?;
            confirm_live(raw(&thread.handle))?;
            require(
                thread.identity == initial.identity,
                "启动前异常线程出生身份改变",
            )?;
            let mut receipt = PreStartExceptionReceipt {
                generation: self.generation,
                identity: thread.identity,
                pre_return_sequence: initial.pre_return_sequence,
                initial_sequence: initial.event_sequence,
                event_sequence: sequence,
                exception_code,
                first_chance,
                parameter_count,
                event_hresult,
                reader_eligible,
                registers_restored: false,
                execution_context_unchanged: false,
            };
            self.pre_exception = Some(receipt.clone());
            if !reader_eligible {
                return Ok(Some(receipt));
            }
            let current = context(raw(&thread.handle))?;
            self.stopped = true;
            self.withdraw_for_reader(event)?;
            let after = context(raw(&self.threads[&event.dwThreadId].handle))?;
            require(
                !self.requires_restoration() && execution_equal(&current.0, &after.0),
                "恢复 DR 后启动前原异常执行上下文改变",
            )?;
            receipt.registers_restored = true;
            receipt.execution_context_unchanged = true;
            self.pre_exception = Some(receipt.clone());
            self.pre_exception_resume = Some(PostNodeExceptionResume {
                stop,
                identity: receipt.identity,
                context: after.0,
            });
            Ok(Some(receipt))
        })();
        if result.is_err() {
            self.stopped = true;
            self.invalidate_stop_tickets();
            self.retire_continuation();
        }
        result
    }
    /// 回收异常 reader 后恢复原后续点；不吞掉或重新投递目标异常。
    pub fn resume_after_pre_start_exception(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<()> {
        let ticket = self.pre_exception_resume.take();
        self.invalidate_stop_tickets();
        let result = (|| {
            let ticket = ticket.ok_or_else(|| invalid("没有可恢复的启动前异常停点"))?;
            self.verify_restored_stop(event, sequence, &ticket)?;
            let receipt = self
                .pre_exception
                .as_ref()
                .ok_or_else(|| invalid("启动前异常收据缺失"))?;
            require(
                self.node.is_none()
                    && self.initial_resumed == 1
                    && self.pre_exception_selected == 1
                    && self.pre_exception_resumed == 0
                    && receipt.reader_eligible
                    && receipt.registers_restored
                    && receipt.execution_context_unchanged
                    && receipt.event_sequence == sequence
                    && receipt.identity == ticket.identity
                    && self.pre_return_binding
                        == Some((receipt.identity, receipt.pre_return_sequence))
                    && self.initial_receipt.as_ref().is_some_and(|initial| {
                        initial.identity == receipt.identity
                            && initial.event_sequence == receipt.initial_sequence
                    })
                    && !self.managed_invalidated
                    && self.managed_selected == 0,
                "启动前异常恢复身份、阶段或预算不符",
            )?;
            self.managed
                .as_ref()
                .ok_or_else(|| invalid("启动前异常后续点租约缺失"))?
                .verify(raw(&self.process))?;
            self.stopped = false;
            self.arm_all(event)?;
            self.pre_exception_resumed = 1;
            Ok(())
        })();
        if result.is_err() {
            self.stopped = true;
            self.retire_continuation();
        }
        result
    }
    /// 只在已撤销全部自有 DR 的独立续点停点提供 opaque 票据。
    pub fn managed_continuation_target(&self) -> io::Result<&ManagedContinuationTarget> {
        require(
            self.stopped
                && self.managed_selected == 1
                && self.managed_resumed == 0
                && (!self.initial_bound || self.initial_resumed == 1)
                && !self.managed_invalidated
                && !self.requires_restoration()
                && self
                    .managed_resume
                    .as_ref()
                    .is_some_and(|ticket| ticket.stop.sequence == self.last_sequence),
            "当前不是可读取的原托管续点停点",
        )?;
        self.managed
            .as_ref()
            .map(|lease| &lease.target)
            .ok_or_else(|| invalid("托管续点私有租约缺失"))
    }
    fn verify_restored_stop(
        &self,
        event: &DEBUG_EVENT,
        sequence: u64,
        ticket: &PostNodeExceptionResume,
    ) -> io::Result<()> {
        self.stopped(event)?;
        require(
            self.stopped
                && self.last_sequence == sequence
                && !self.requires_restoration()
                && self.threads.values().all(|thread| thread.pending.is_none())
                && same_exception_stop(event, sequence, &ticket.stop),
            "托管续点恢复必须保有同一原事件及已恢复配置",
        )?;
        confirm_live(raw(&self.process))?;
        let thread = self
            .threads
            .get(&ticket.identity.thread_id)
            .ok_or_else(|| invalid("托管续点恢复原线程缺失"))?;
        thread.verify(raw(&self.process))?;
        confirm_live(raw(&thread.handle))?;
        let current = context(raw(&thread.handle))?;
        require(
            thread.identity == ticket.identity
                && execution_equal(&ticket.context, &current.0)
                && Registers::read(&ticket.context) == Registers::read(&current.0),
            "托管续点恢复原上下文改变",
        )
    }
    /// 调用方必须先回收 op4 reader；本次命中永不重新安装 DR1。
    pub fn resume_after_managed_continuation(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<()> {
        let ticket = self.managed_resume.take();
        self.invalidate_stop_tickets();
        let result = (|| {
            let ticket = ticket.ok_or_else(|| invalid("没有可恢复的托管续点停点"))?;
            self.verify_restored_stop(event, sequence, &ticket)?;
            let receipt = self
                .managed_receipt
                .as_ref()
                .ok_or_else(|| invalid("托管续点收据缺失"))?;
            let lease = self
                .managed
                .as_ref()
                .ok_or_else(|| invalid("托管续点租约缺失"))?;
            require(
                self.managed_selected == 1
                    && self.managed_resumed == 0
                    && !self.managed_invalidated
                    && receipt.event_sequence == sequence
                    && receipt.identity == ticket.identity
                    && receipt.registers_restored
                    && receipt.execution_context_unchanged
                    && self.pre_return_binding
                        == Some((ticket.identity, receipt.pre_return_sequence))
                    && self.node
                        == Some((
                            receipt.node_create_sequence,
                            receipt.node_process_id,
                            receipt.node_process_birth,
                        ))
                    && continuation_binding_matches(
                        &lease.target,
                        ticket.identity,
                        receipt.pre_return_sequence,
                    ),
                "托管续点恢复阶段、身份或预算不符",
            )?;
            lease.verify(raw(&self.process))?;
            require(self.image.is_some(), "托管续点恢复 Shell32 租约缺失")?;
            self.managed = None;
            // post 分类尚未发生时才恢复其入口；已完成的分类预算不会被续点重置。
            if self.selected < MAX_CALLS {
                self.stopped = false;
                self.arm_all(event)?;
            }
            self.managed_resumed = 1;
            Ok(())
        })();
        if result.is_err() {
            self.stopped = true;
            self.retire_continuation();
        }
        result
    }
    /// post 分类已结束时，只有尚未命中的原续点租约可单独继续观察。
    pub fn resume_after_post_node_return_for_continuation(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<bool> {
        let ticket = self.post_return_resume.take();
        self.invalidate_stop_tickets();
        if self.active_continuation().is_none() {
            return Ok(false);
        }
        let result = (|| {
            let ticket = ticket.ok_or_else(|| invalid("没有可恢复的 post 分类返回停点"))?;
            self.verify_restored_stop(event, sequence, &ticket)?;
            let lease = self.active_continuation().unwrap();
            require(
                self.selected == 1
                    && self.returned == 1
                    && self.pre_return_binding
                        == Some((
                            continuation_identity(&lease.target),
                            lease.target.pre_sequence,
                        ))
                    && self.node.is_some_and(|node| {
                        lease.target.pre_sequence < node.0 && node.0 < sequence
                    }),
                "post 分类返回不能恢复已失效或不同阶段的续点",
            )?;
            lease.verify(raw(&self.process))?;
            require(self.image.is_some(), "post 分类返回 Shell32 租约缺失")?;
            self.stopped = false;
            self.arm_all(event)?;
            Ok(true)
        })();
        if result.is_err() {
            self.stopped = true;
            self.retire_continuation();
        }
        result
    }
    /// 仅覆盖此调试停点的原 root 事件线程，不代表其他线程或后续期间的 DR 状态。
    pub fn sample_post_node_root_stop(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
    ) -> io::Result<bool> {
        self.invalidate_stop_tickets();
        require(
            live_root_sample_event(event, self.root_pid),
            "DR 采样必须使用原 root 活线程停点",
        )?;
        let Some((node_sequence, _, _)) = self.node else {
            return Ok(false);
        };
        if self.post_node_sample_attempted {
            return Ok(false);
        }
        require(
            node_sequence < sequence && sequence >= self.last_sequence,
            "DR 采样必须晚于绑定 Node CREATE",
        )?;
        self.post_node_sample_attempted = true;
        self.stopped(event)?;
        confirm_live(raw(&self.process))?;
        let thread = self
            .threads
            .get(&event.dwThreadId)
            .ok_or_else(|| invalid("DR 采样原线程尚未绑定"))?;
        thread.verify(raw(&self.process))?;
        confirm_live(raw(&thread.handle))?;
        let current = context(raw(&thread.handle))?;
        let actual = Registers::read(&current.0);
        self.post_node_root_stop = Some(PostNodeDrSample {
            generation: self.generation,
            identity: thread.identity,
            sequence,
            node_create_sequence: node_sequence,
            event_code: event.dwDebugEventCode.0,
            scope: "original_root_event_thread_at_this_stop_only",
            dirty: thread.dirty,
            addresses_equal: thread
                .expected
                .map(|expected| expected.address == actual.address),
            configuration_equal: thread.expected.map(|expected| {
                actual.same_configuration(expected)
                    || actual.same_configuration(expected.inactive_api_view())
            }),
            dr6_expected: thread.expected.map(|expected| expected.status),
            dr6_actual: actual.status,
            dr7_expected: thread.expected.map(|expected| expected.control),
            dr7_actual: actual.control,
        });
        // 不推进观察序号，同一原事件还要交 observe 配对。
        Ok(true)
    }
    pub fn bound_base(&self) -> Option<u64> {
        self.image.as_ref().map(|image| image.base)
    }
    /// 必须在原 UNLOAD 停点撤销后停止，不能对同基址的新映像自动重新安装。
    pub fn unload(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        self.invalidate_stop_tickets();
        self.stopped(event)?;
        require(
            event.dwDebugEventCode == UNLOAD_DLL_DEBUG_EVENT
                && self.bound_base() == Some(unsafe { event.u.UnloadDll.lpBaseOfDll } as u64),
            "Shell32 原卸载事件不符",
        )?;
        self.stopped = true;
        self.withdraw_all(event)?;
        self.image = None;
        Ok(())
    }
    pub fn finish_observation(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        self.invalidate_stop_tickets();
        self.stopped(event)?;
        self.stopped = true;
        self.withdraw_all(event)
    }
    /// 此事件由调用方先核为本轮绑定 Node，不能用名称相似的任意后代代替。
    pub fn set_node_created(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
        node_birth: u64,
    ) -> io::Result<()> {
        self.invalidate_stop_tickets();
        require(
            !self.exited
                && unsafe { GetCurrentThreadId() } == self.debugger_tid
                && self.node.is_none()
                && (!self.initial_bound || self.initial_resumed == 1)
                && sequence > self.last_sequence
                && event.dwDebugEventCode == CREATE_PROCESS_DEBUG_EVENT
                && event.dwProcessId != self.root_pid
                && event.dwProcessId != 0
                && node_birth != 0,
            "绑定 Node CREATE 无效",
        )?;
        let handle = unsafe { event.u.CreateProcessInfo.hProcess };
        require(
            unsafe { GetProcessId(handle) } == event.dwProcessId
                && birth(handle, false)? == node_birth,
            "Node 原创建身份不符",
        )?;
        self.node = Some((sequence, event.dwProcessId, node_birth));
        self.last_sequence = sequence;
        let result = self.release_deferred_after_node(sequence);
        if result.is_err() {
            self.stopped = true;
            self.retire_continuation();
        }
        result
    }
    pub fn observe(&mut self, event: &DEBUG_EVENT, sequence: u64) -> io::Result<Observation> {
        self.invalidate_stop_tickets();
        let result = self.observe_event(event, sequence);
        if result.is_err() {
            self.stopped = true;
            self.invalidate_stop_tickets();
            self.retire_continuation();
        } else if matches!(&result, Ok(Observation::NotOwned))
            && !self.stopped
            && event.dwProcessId == self.root_pid
            && event.dwDebugEventCode == EXCEPTION_DEBUG_EVENT
            && sequence == self.last_sequence
        {
            self.post_exception_candidate = Some(OriginalExceptionStop {
                event: *event,
                sequence,
            });
        }
        result
    }
    fn observe_event(&mut self, event: &DEBUG_EVENT, sequence: u64) -> io::Result<Observation> {
        if event.dwProcessId != self.root_pid || event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT {
            return Ok(Observation::NotOwned);
        }
        require(
            !self.stopped || !self.requires_restoration(),
            "已停止分类观察仍有未恢复配置",
        )?;
        if self.stopped {
            return Ok(Observation::NotOwned);
        }
        self.stopped(event)?;
        require(sequence > self.last_sequence, "分类停点序号未递增")?;
        self.last_sequence = sequence;
        let exception = unsafe { event.u.Exception };
        if exception.dwFirstChance != 1
            || exception.ExceptionRecord.ExceptionCode != EXCEPTION_SINGLE_STEP
        {
            return self.unowned_exception(event);
        }
        let thread = self
            .threads
            .get(&event.dwThreadId)
            .ok_or_else(|| invalid("原异常线程未绑定"))?;
        thread.verify(raw(&self.process))?;
        let current = context(raw(&thread.handle))?;
        let Some(slot) = thread
            .expected
            .filter(|_| thread.dirty)
            .and_then(|expected| {
                owned_slot(
                    expected,
                    &current.0,
                    exception.ExceptionRecord.ExceptionAddress as u64,
                )
            })
        else {
            if self.first_unowned_single_step.is_none() {
                let actual = Registers::read(&current.0);
                // 只记录首次未归属停点的控制位与相等性，原异常继续语义不变。
                self.first_unowned_single_step = Some(serde_json::json!({
                    "sequence": sequence, "identity": thread.identity, "dirty": thread.dirty,
                    "addresses_equal": thread.expected.map(|value| value.address == actual.address),
                    "dr6_expected": thread.expected.map(|value| value.status),
                    "dr6_actual": actual.status,
                    "dr7_expected": thread.expected.map(|value| value.control),
                    "dr7_actual": actual.control,
                    "eflags": current.0.EFlags,
                    "ip_matches_exception": current.0.Rip == exception.ExceptionRecord.ExceptionAddress as u64,
                    "ip_matches_slots": actual.address.map(|address| address == current.0.Rip),
                }));
            }
            return self.unowned_exception(event);
        };
        if slot == 1 {
            if self.initial_bound && self.initial_resumed == 0 {
                return self.observe_managed_initial(event, sequence, current);
            }
            return self.observe_managed_continuation(event, sequence, current);
        }
        if slot == 0 {
            let image = self.image.as_ref().ok_or_else(|| invalid("API 映像缺失"))?;
            image.verify_entry(raw(&self.process))?;
            require(current.0.Rsp & 15 == 8, "API 入口栈未按 x64 ABI 对齐")?;
            let flags_address = current
                .0
                .Rsp
                .checked_add(0x28)
                .ok_or_else(|| invalid("API 参数槽溢出"))?;
            let flags = dword(&memory(raw(&self.process), flags_address, 4)?, 0)?;
            let phase = select_phase(self.node, sequence, flags, self.pre_selected, self.selected);
            // 启动前独立预算不消费 postNode 预算；首个跳过入口仍只额外比较一次路径。
            let path_comparison =
                if phase.is_some() || (self.first_skipped_entry.is_none() && flags == 0x2000) {
                    compare_node_path(raw(&self.process), current.0.Rcx, &self.expected_node)
                } else {
                    PathComparison::NotChecked
                };
            let phase = phase.filter(|_| path_comparison == PathComparison::Matched);
            let continuation = if let Some(lease) = self.active_continuation() {
                lease.verify(raw(&self.process))?;
                (continuation_identity(&lease.target) == thread.identity)
                    .then_some(lease.target.address)
            } else {
                None
            };
            let thread = self.threads.get_mut(&event.dwThreadId).unwrap();
            require(thread.pending.is_none(), "分类调用意外重入")?;
            let Some(phase) = phase else {
                let mut resumed = current;
                let change_resume_flag = resumed.0.EFlags & RF == 0;
                thread.resume_flag = Some((resumed.0.Rip, resumed.0.EFlags & RF != 0));
                resumed.0.EFlags |= RF;
                let mut expected = thread.expected.unwrap();
                expected.status &= !OWN_STATUS;
                thread.change(resumed, expected, change_resume_flag)?;
                self.skipped += 1;
                self.first_skipped_entry
                    .get_or_insert_with(|| SkippedEntryReceipt {
                        generation: self.generation,
                        identity: thread.identity,
                        sequence,
                        node_create_sequence: self.node.map(|node| node.0),
                        flags,
                        path_comparison,
                    });
                return Ok(Observation::OwnedSkipped);
            };
            let returned_to = qword(&memory(raw(&self.process), current.0.Rsp, 8)?, 0)?;
            let region = return_region(raw(&self.process), returned_to)?;
            let pending = Pending {
                stack: current.0.Rsp,
                returned_to,
                region,
                sequence,
                nonce: nonce(self.generation, sequence),
                phase,
            };
            let original = thread.original.ok_or_else(|| invalid("原 DR 配置缺失"))?;
            thread.pending = Some(pending);
            match phase {
                CallPhase::PreNode => self.pre_selected += 1,
                CallPhase::PostNode { .. } => self.selected += 1,
            }
            thread.change(
                current,
                original.combined(Some((returned_to, true)), continuation),
                false,
            )?;
            return Ok(match phase {
                CallPhase::PreNode => Observation::OwnedPreNodeEntry,
                CallPhase::PostNode { .. } => Observation::OwnedEntry,
            });
        }
        require(slot == 3, "意外的自有调试槽")?;
        let thread = self.threads.get(&event.dwThreadId).unwrap();
        let pending = thread
            .pending
            .as_ref()
            .ok_or_else(|| invalid("分类返回无入口配对"))?;
        require(
            matching_return(pending, &current.0)
                && return_region(raw(&self.process), pending.returned_to)? == pending.region,
            "分类返回栈或代码映射改变",
        )?;
        require(
            return_phase_matches(pending, self.node, sequence),
            "分类调用跨越 Node 创建或返回阶段不符",
        )?;
        let identity = thread.identity;
        let return_region_kind = if pending.region.kind == MEM_PRIVATE.0 {
            "private"
        } else {
            "image"
        };
        let phase = pending.phase;
        let observation = match phase {
            CallPhase::PreNode => Observation::OwnedPreNodeReturn(PreNodeReturnReceipt {
                generation: self.generation,
                identity,
                pair_nonce: pending.nonce,
                entry_sequence: pending.sequence,
                return_sequence: sequence,
                expected_node_matched: true,
                flags: 0x2000,
                raw_return_u64: current.0.Rax,
                raw_return_low32: current.0.Rax as u32,
                return_region_kind,
                registers_restored: true,
                execution_context_unchanged: true,
                pre_node_clr_stack_required: true,
            }),
            CallPhase::PostNode { .. } => {
                let node = self.node.ok_or_else(|| invalid("Node 绑定缺失"))?;
                Observation::OwnedReturn(ReturnReceipt {
                    generation: self.generation,
                    identity,
                    pair_nonce: pending.nonce,
                    entry_sequence: pending.sequence,
                    return_sequence: sequence,
                    node_create_sequence: node.0,
                    node_process_id: node.1,
                    node_process_birth: node.2,
                    expected_node_matched: true,
                    flags: 0x2000,
                    raw_return_u64: current.0.Rax,
                    raw_return_low32: current.0.Rax as u32,
                    return_region_kind,
                    registers_restored: true,
                    execution_context_unchanged: true,
                    post_start_clr_stack_required: true,
                })
            }
        };
        self.withdraw_for_reader(event)?;
        let after = context(raw(&self.threads[&event.dwThreadId].handle))?;
        require(
            execution_equal(&current.0, &after.0),
            "恢复 DR 后原返回执行上下文改变",
        )?;
        self.stopped = true;
        match phase {
            CallPhase::PreNode => {
                self.pre_returned += 1;
                self.pre_return_binding = Some((identity, sequence));
                self.pre_resume = Some(PreNodeResume {
                    sequence,
                    identity,
                    context: after.0,
                });
            }
            CallPhase::PostNode { .. } => {
                self.returned += 1;
                if self.active_continuation().is_some() {
                    self.post_return_resume = Some(PostNodeExceptionResume {
                        stop: OriginalExceptionStop {
                            event: *event,
                            sequence,
                        },
                        identity,
                        context: after.0,
                    });
                }
            }
        }
        Ok(observation)
    }
    fn observe_managed_initial(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
        current: AlignedContext,
    ) -> io::Result<Observation> {
        let lease = self
            .active_continuation()
            .ok_or_else(|| invalid("首次托管点命中缺少有效租约"))?;
        let identity = self.threads[&event.dwThreadId].identity;
        let target = &lease.target;
        require(
            self.node.is_none()
                && self.initial_selected == 0
                && self.initial_resumed == 0
                && self.pre_returned == 1
                && self.managed_selected == 0
                && self.pre_return_binding == Some((identity, target.pre_sequence))
                && target.pre_sequence < sequence
                && continuation_frame_matches(target, identity, &current.0)
                && self.threads.values().all(|thread| thread.pending.is_none()),
            "首次托管点必须位于原 pre 后、Node 创建前的同一帧",
        )?;
        lease.verify(raw(&self.process))?;
        let mut receipt = ManagedInitialReceipt {
            generation: self.generation,
            identity,
            pre_return_sequence: target.pre_sequence,
            event_sequence: sequence,
            registers_restored: false,
            execution_context_unchanged: false,
            managed_initial_clr_stack_required: true,
            mapping: target.safe_evidence(),
        };
        self.initial_selected = 1;
        self.stopped = true;
        self.withdraw_for_reader(event)?;
        let after = context(raw(&self.threads[&event.dwThreadId].handle))?;
        require(
            !self.requires_restoration() && execution_equal(&current.0, &after.0),
            "恢复 DR 后首次托管点执行上下文改变",
        )?;
        receipt.registers_restored = true;
        receipt.execution_context_unchanged = true;
        self.initial_receipt = Some(receipt.clone());
        self.initial_resume = Some(PostNodeExceptionResume {
            stop: OriginalExceptionStop {
                event: *event,
                sequence,
            },
            identity,
            context: after.0,
        });
        Ok(Observation::OwnedManagedInitial(receipt))
    }
    fn observe_managed_continuation(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
        current: AlignedContext,
    ) -> io::Result<Observation> {
        let lease = self
            .active_continuation()
            .ok_or_else(|| invalid("托管续点命中没有有效的独立租约"))?;
        let identity = self.threads[&event.dwThreadId].identity;
        let target = &lease.target;
        require(
            self.pre_returned == 1
                && self.managed_selected == 0
                && self.managed_resumed == 0
                && self.pre_return_binding == Some((identity, target.pre_sequence))
                && target.pre_sequence < sequence
                && continuation_frame_matches(target, identity, &current.0)
                && self.threads.values().all(|thread| thread.pending.is_none()),
            "托管续点的原帧、线程、Node 阶段或分类配对不符",
        )?;
        lease.verify(raw(&self.process))?;
        let Some(node) = self.node else {
            return self.defer_managed_continuation(event, sequence, current);
        };
        require(
            continuation_hit_matches(target, identity, self.node, sequence, &current.0),
            "托管续点读取必须晚于真实 Node CREATE 绑定",
        )?;
        let first_delivery_sequence = if let Some(deferred) = &self.managed_deferred {
            require(
                deferred_replay_matches(event, sequence, node.0, &current.0, deferred),
                "托管续点重放事件、原上下文或独立顺序不符",
            )?;
            deferred.stop.sequence
        } else {
            sequence
        };
        let mut receipt = ManagedContinuationReceipt {
            generation: self.generation,
            identity,
            pre_return_sequence: target.pre_sequence,
            node_create_sequence: node.0,
            node_process_id: node.1,
            node_process_birth: node.2,
            event_sequence: sequence,
            first_delivery_sequence,
            deferred_for_node: self.managed_deferred.is_some(),
            registers_restored: false,
            execution_context_unchanged: false,
            managed_continuation_clr_stack_required: true,
            mapping: target.safe_evidence(),
        };
        if let Some(deferred) = &mut self.managed_deferred {
            deferred.receipt.replay_sequence = Some(sequence);
            deferred.receipt.phase = "replayed";
        }
        self.managed_selected = 1;
        self.stopped = true;
        self.withdraw_for_reader(event)?;
        let after = context(raw(&self.threads[&event.dwThreadId].handle))?;
        require(
            !self.requires_restoration() && execution_equal(&current.0, &after.0),
            "恢复 DR 后原托管续点执行上下文改变",
        )?;
        receipt.registers_restored = true;
        receipt.execution_context_unchanged = true;
        self.managed_receipt = Some(receipt.clone());
        self.managed_resume = Some(PostNodeExceptionResume {
            stop: OriginalExceptionStop {
                event: *event,
                sequence,
            },
            identity,
            context: after.0,
        });
        Ok(Observation::OwnedManagedContinuation(receipt))
    }
    fn defer_managed_continuation(
        &mut self,
        event: &DEBUG_EVENT,
        sequence: u64,
        current: AlignedContext,
    ) -> io::Result<Observation> {
        require(
            self.managed_deferred.is_none() && self.node.is_none(),
            "托管续点延后只能使用第一次原命中",
        )?;
        let (identity, pre_sequence) = self
            .pre_return_binding
            .ok_or_else(|| invalid("延后续点的 pre 原绑定缺失"))?;
        let thread = &self.threads[&event.dwThreadId];
        confirm_live(raw(&thread.handle))?;
        let previous = unsafe { SuspendThread(raw(&thread.handle)) };
        if previous == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        // 先记住本次增加；包括非零原计数、读回失败，都必须平衡这一且仅这一增量。
        self.managed_deferred = Some(DeferredManagedContinuation {
            stop: OriginalExceptionStop {
                event: *event,
                sequence,
            },
            context: current.0,
            receipt: ManagedContinuationDeferredReceipt {
                generation: self.generation,
                identity,
                pre_return_sequence: pre_sequence,
                first_delivery_sequence: sequence,
                phase: "suspended_before_continue",
                suspend_previous_count: previous,
                reply_later_confirmed: false,
                node_create_sequence: None,
                resume_previous_count: None,
                replay_sequence: None,
                suspend_owned: true,
                original_thread_exit_confirmed: false,
            },
        });
        require(previous == 0, "延后续点原线程已有外部暂停计数")?;
        let after = context(raw(&thread.handle))?;
        require(
            execution_equal(&current.0, &after.0)
                && Registers::read(&current.0) == Registers::read(&after.0),
            "延后续点暂停后原上下文改变",
        )?;
        Ok(Observation::OwnedManagedContinuationDeferred(
            self.managed_deferred.as_ref().unwrap().receipt.clone(),
        ))
    }
    fn unowned_exception(&mut self, event: &DEBUG_EVENT) -> io::Result<Observation> {
        if self.threads.values().any(|thread| thread.pending.is_some()) {
            // 异常可能展开原调用帧；不能把将来同址命中误配为正常返回。
            self.stopped = true;
            self.withdraw_all(event)?;
        }
        Ok(Observation::NotOwned)
    }
    pub fn withdraw_all(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        self.retire_continuation();
        self.withdraw_for_reader(event)?;
        if let Some(deferred) = &mut self.managed_deferred {
            if deferred.receipt.phase != "replayed" {
                deferred.receipt.phase = "cancelled_original_exit_required";
            }
            self.release_deferred_suspend()?;
        }
        Ok(())
    }
    fn withdraw_for_reader(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        self.invalidate_stop_tickets();
        self.stopped(event)?;
        let mut failure = None;
        for thread in self.threads.values_mut() {
            let result = thread
                .verify(raw(&self.process))
                .and_then(|()| thread.restore());
            match result {
                Ok(true) => self.restored += 1,
                Ok(false) => (),
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
            }
        }
        match failure {
            Some(error) => {
                self.retire_continuation();
                Err(error)
            }
            None => Ok(()),
        }
    }
    /// EXIT_THREAD 已由原事件拥有者继续后调用；退出不冒充恢复读回。
    pub fn thread_exited(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
        self.invalidate_stop_tickets();
        require(
            event.dwProcessId == self.root_pid
                && event.dwDebugEventCode == EXIT_THREAD_DEBUG_EVENT
                && unsafe { GetCurrentThreadId() } == self.debugger_tid,
            "原线程退出事件无效",
        )?;
        let thread = self
            .threads
            .get(&event.dwThreadId)
            .ok_or_else(|| invalid("退出线程未绑定"))?;
        confirm_exit(raw(&thread.handle))?;
        self.exited_dirty += u64::from(thread.dirty);
        self.completed_inactive_api_readbacks += thread.inactive_api_readbacks;
        self.completed_resume_flag_writes += thread.resume_flag_writes;
        self.completed_fixed_eflags_api_readbacks += thread.fixed_eflags_api_readbacks;
        if let Some(deferred) = &mut self.managed_deferred
            && deferred.receipt.identity == thread.identity
        {
            deferred.receipt.original_thread_exit_confirmed = true;
            deferred.receipt.suspend_owned = false;
            if deferred.receipt.phase != "replayed" {
                deferred.receipt.phase = "cancelled_original_exit_required";
            }
        }
        if self
            .managed
            .as_ref()
            .is_some_and(|lease| continuation_identity(&lease.target) == thread.identity)
        {
            self.retire_continuation();
        }
        self.threads.remove(&event.dwThreadId);
        Ok(())
    }
    pub fn confirm_process_exit(&mut self) -> io::Result<()> {
        self.invalidate_stop_tickets();
        require(
            unsafe { GetCurrentThreadId() } == self.debugger_tid,
            "必须由原调试线程确认退出",
        )?;
        confirm_exit(raw(&self.process))?;
        for thread in self.threads.values_mut() {
            confirm_exit(raw(&thread.handle))?;
            self.exited_dirty += u64::from(thread.dirty);
            thread.dirty = false;
            thread.pending = None;
        }
        self.exited = true;
        self.stopped = true;
        if let Some(deferred) = &mut self.managed_deferred {
            deferred.receipt.original_thread_exit_confirmed = true;
            deferred.receipt.suspend_owned = false;
            if deferred.receipt.phase != "replayed" {
                deferred.receipt.phase = "cancelled_original_exit_required";
            }
        }
        self.retire_continuation();
        Ok(())
    }
    pub fn process_handle(&self) -> BorrowedHandle<'_> {
        self.process.as_handle()
    }
    pub fn thread_handle(&self, tid: u32) -> io::Result<BorrowedHandle<'_>> {
        self.threads
            .get(&tid)
            .map(|thread| thread.handle.as_handle())
            .ok_or_else(|| invalid("原线程句柄缺失"))
    }
    pub fn requires_restoration(&self) -> bool {
        self.threads.values().any(|thread| thread.dirty)
            || self
                .managed_deferred
                .as_ref()
                .is_some_and(|deferred| deferred.receipt.suspend_owned)
    }
    pub fn summary(&self) -> Summary {
        Summary {
            generation: self.generation,
            root_process_id: self.root_pid,
            selected_calls: self.selected,
            returned_calls: self.returned,
            pre_node_selected_calls: self.pre_selected,
            pre_node_returned_calls: self.pre_returned,
            post_node_root_stop_attempted: self.post_node_sample_attempted,
            post_node_root_stop: self.post_node_root_stop.clone(),
            post_node_exception_selected_calls: self.post_exception_selected,
            post_node_exception_resumed_calls: self.post_exception_resumed,
            post_node_exception: self.post_exception.clone(),
            pre_start_exception_selected_calls: self.pre_exception_selected,
            pre_start_exception_resumed_calls: self.pre_exception_resumed,
            pre_start_exception: self.pre_exception.clone(),
            managed_initial_bound: self.initial_bound,
            managed_initial_selected_calls: self.initial_selected,
            managed_initial_resumed_calls: self.initial_resumed,
            managed_initial: self.initial_receipt.clone(),
            managed_continuation_bound: self.managed_bound,
            managed_continuation_selected_calls: self.managed_selected,
            managed_continuation_resumed_calls: self.managed_resumed,
            managed_continuation_invalidated: self.managed_invalidated,
            managed_continuation: self.managed_receipt.clone(),
            managed_continuation_deferred: self
                .managed_deferred
                .as_ref()
                .map(|value| value.receipt.clone()),
            skipped_entries: self.skipped,
            first_skipped_entry: self.first_skipped_entry.clone(),
            dirty_threads: self.threads.values().filter(|thread| thread.dirty).count(),
            restored_threads: self.restored,
            inactive_api_readbacks: self.completed_inactive_api_readbacks
                + self
                    .threads
                    .values()
                    .map(|thread| thread.inactive_api_readbacks)
                    .sum::<u64>(),
            resume_flag_writes: self.completed_resume_flag_writes
                + self
                    .threads
                    .values()
                    .map(|thread| thread.resume_flag_writes)
                    .sum::<u64>(),
            fixed_eflags_api_readbacks: self.completed_fixed_eflags_api_readbacks
                + self
                    .threads
                    .values()
                    .map(|thread| thread.fixed_eflags_api_readbacks)
                    .sum::<u64>(),
            threads_exited_before_restore: self.exited_dirty,
            original_process_exit_confirmed: self.exited,
            stopped: self.stopped,
            first_unowned_single_step: self.first_unowned_single_step.clone(),
            restoration: if self.requires_restoration() {
                "required"
            } else if self.exited_dirty > 0 {
                "original_exit_confirmed"
            } else if self.restored > 0 {
                "readback_verified"
            } else {
                "not_modified"
            },
            result: if self.returned == 0 {
                "unknown"
            } else {
                "return_observed_clr_validation_required"
            },
        }
    }
}

#[cfg(test)]
#[path = "windows_shell_classification_tests.rs"]
mod tests;
