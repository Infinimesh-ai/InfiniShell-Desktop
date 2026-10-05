//! 原 PowerShell 工作线程的一次 Shell32 分类返回观察，不参与映像授权或候选成功判定。
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
    DUPLICATE_HANDLE_OPTIONS, DuplicateHandle, EXCEPTION_SINGLE_STEP, FILETIME, GENERIC_READ,
    HANDLE, WAIT_EVENT, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, GetFileInformationByHandle, ReOpenFile,
};
use windows::Win32::System::Diagnostics::Debug::{
    CONTEXT, CONTEXT_CONTROL_AMD64, CONTEXT_DEBUG_REGISTERS_AMD64, CONTEXT_INTEGER_AMD64,
    CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT, DEBUG_EVENT, EXCEPTION_DEBUG_EVENT,
    EXIT_THREAD_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT, UNLOAD_DLL_DEBUG_EVENT,
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
    PROCESS_SYNCHRONIZE, PROCESS_VM_READ, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION,
    THREAD_SET_CONTEXT, THREAD_SYNCHRONIZE, WaitForSingleObject,
};
use windows::core::BOOL;

const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_THREADS: usize = 64;
const MAX_PATH_UNITS: usize = 1024;
const MAX_CALLS: u32 = 1;
const MAX_USER: u64 = 0x0000_7fff_ffff_ffff;
const OWN_STATUS: u64 = 15;
// x64 DR6 非活动状态及 DR7 固定置一位；不把内核初始零视图作为硬件命中后的期望值。
const DR6_INACTIVE: u64 = 0xffff_0ff0;
const DR7_FIXED_ONE: u64 = 1 << 10;
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
    pub raw_return_low32: u32,
    pub return_region_kind: &'static str,
    pub registers_restored: bool,
    pub execution_context_unchanged: bool,
    pub post_start_clr_stack_required: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub generation: [u8; 16],
    pub root_process_id: u32,
    pub selected_calls: u32,
    pub returned_calls: u32,
    pub skipped_entries: u64,
    pub dirty_threads: usize,
    pub restored_threads: u64,
    pub inactive_api_readbacks: u64,
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
    OwnedEntry,
    OwnedReturn(ReturnReceipt),
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
    let bytes = memory(process, address, (end - address).min(32) as usize)?;
    Ok(ReturnRegion {
        allocation: info.AllocationBase as u64,
        base,
        length: info.RegionSize as u64,
        kind: info.Type.0,
        protect: info.Protect.0,
        bytes,
    })
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

struct Pending {
    stack: u64,
    returned_to: u64,
    region: ReturnRegion,
    sequence: u64,
    nonce: [u8; 16],
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
        if (actual != registers && actual != registers.inactive_api_view())
            || !execution_equal(&current.0, &after.0)
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
    skipped: u64,
    restored: u64,
    exited_dirty: u64,
    exited: bool,
    stopped: bool,
    first_unowned_single_step: Option<serde_json::Value>,
    completed_inactive_api_readbacks: u64,
}
impl std::fmt::Debug for ShellClassificationWitness {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.summary().fmt(formatter)
    }
}
impl ShellClassificationWitness {
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
            skipped: 0,
            restored: 0,
            exited_dirty: 0,
            exited: false,
            stopped: false,
            first_unowned_single_step: None,
            completed_inactive_api_readbacks: 0,
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
        };
        thread.verify(raw(&self.process))?;
        self.threads.insert(tid, thread);
        Ok(())
    }
    pub fn retain_thread(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
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
        self.stopped(event)?;
        if self.stopped || self.selected >= MAX_CALLS {
            return Ok(());
        }
        let Some(image) = self.image.as_ref() else {
            return Ok(());
        };
        image.verify_entry(raw(&self.process))?;
        for thread in self.threads.values_mut() {
            thread.verify(raw(&self.process))?;
            if thread.dirty {
                continue;
            }
            let current = context(raw(&thread.handle))?;
            let original = Registers::read(&current.0);
            require(
                original.vacant(current.0.EFlags),
                "拒绝覆盖已有调试寄存器配置",
            )?;
            thread.original = Some(original);
            thread.change(current, original.armed(image.entry, false), false)?;
        }
        Ok(())
    }
    pub fn bound_base(&self) -> Option<u64> {
        self.image.as_ref().map(|image| image.base)
    }
    /// 必须在原 UNLOAD 停点撤销后停止，不能对同基址的新映像自动重新安装。
    pub fn unload(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
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
        require(
            !self.exited
                && unsafe { GetCurrentThreadId() } == self.debugger_tid
                && self.node.is_none()
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
        Ok(())
    }
    fn path_matches(&self, address: u64) -> bool {
        let mut actual = Vec::new();
        for (index, wanted) in self
            .expected_node
            .iter()
            .copied()
            .chain(std::iter::once(0))
            .enumerate()
        {
            let Some(at) = address.checked_add((index * 2) as u64) else {
                return false;
            };
            let Ok(bytes) = memory(raw(&self.process), at, 2) else {
                return false;
            };
            let unit = u16::from_le_bytes([bytes[0], bytes[1]]);
            if unit != wanted {
                return false;
            }
            actual.push(unit);
        }
        exact_node(&self.expected_node, &actual)
    }
    pub fn observe(&mut self, event: &DEBUG_EVENT, sequence: u64) -> io::Result<Observation> {
        if event.dwProcessId != self.root_pid || event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT {
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
            let selected = self.node.is_some_and(|node| sequence > node.0)
                && flags == 0x2000
                && self.selected < MAX_CALLS
                && self.path_matches(current.0.Rcx);
            let thread = self.threads.get_mut(&event.dwThreadId).unwrap();
            require(thread.pending.is_none(), "分类调用意外重入")?;
            if !selected {
                let mut resumed = current;
                let change_resume_flag = resumed.0.EFlags & RF == 0;
                thread.resume_flag = Some((resumed.0.Rip, resumed.0.EFlags & RF != 0));
                resumed.0.EFlags |= RF;
                let mut expected = thread.expected.unwrap();
                expected.status &= !OWN_STATUS;
                thread.change(resumed, expected, change_resume_flag)?;
                self.skipped += 1;
                return Ok(Observation::OwnedSkipped);
            }
            let returned_to = qword(&memory(raw(&self.process), current.0.Rsp, 8)?, 0)?;
            let region = return_region(raw(&self.process), returned_to)?;
            let pending = Pending {
                stack: current.0.Rsp,
                returned_to,
                region,
                sequence,
                nonce: nonce(self.generation, sequence),
            };
            let original = thread.original.ok_or_else(|| invalid("原 DR 配置缺失"))?;
            thread.pending = Some(pending);
            self.selected += 1;
            thread.change(current, original.armed(returned_to, true), false)?;
            return Ok(Observation::OwnedEntry);
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
        let node = self.node.ok_or_else(|| invalid("Node 绑定缺失"))?;
        let receipt = ReturnReceipt {
            generation: self.generation,
            identity: thread.identity,
            pair_nonce: pending.nonce,
            entry_sequence: pending.sequence,
            return_sequence: sequence,
            node_create_sequence: node.0,
            node_process_id: node.1,
            node_process_birth: node.2,
            expected_node_matched: true,
            flags: 0x2000,
            raw_return_low32: current.0.Rax as u32,
            return_region_kind: if pending.region.kind == MEM_PRIVATE.0 {
                "private"
            } else {
                "image"
            },
            registers_restored: true,
            execution_context_unchanged: true,
            post_start_clr_stack_required: true,
        };
        self.withdraw_all(event)?;
        let after = context(raw(&self.threads[&event.dwThreadId].handle))?;
        require(
            execution_equal(&current.0, &after.0),
            "恢复 DR 后原返回执行上下文改变",
        )?;
        self.returned += 1;
        self.stopped = true;
        Ok(Observation::OwnedReturn(receipt))
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
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    /// EXIT_THREAD 已由原事件拥有者继续后调用；退出不冒充恢复读回。
    pub fn thread_exited(&mut self, event: &DEBUG_EVENT) -> io::Result<()> {
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
        self.threads.remove(&event.dwThreadId);
        Ok(())
    }
    pub fn confirm_process_exit(&mut self) -> io::Result<()> {
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
    }
    pub fn summary(&self) -> Summary {
        Summary {
            generation: self.generation,
            root_process_id: self.root_pid,
            selected_calls: self.selected,
            returned_calls: self.returned,
            skipped_entries: self.skipped,
            dirty_threads: self.threads.values().filter(|thread| thread.dirty).count(),
            restored_threads: self.restored,
            inactive_api_readbacks: self.completed_inactive_api_readbacks
                + self
                    .threads
                    .values()
                    .map(|thread| thread.inactive_api_readbacks)
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
