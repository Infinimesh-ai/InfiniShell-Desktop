//! 私有 npm 调试 worker 的一次性、取消前 x64 用户态快照。
//!
//! 调用方必须已用原 Job、进程创建身份、token 和映像租约核验所有输入，并在调用期间
//! 持有它们；本模块不按 PID 获取权限，也不改变任何授权。所有结果仅供诊断。
//! 栈展开只用已持映像的 PE 异常表及受限内存读取，不初始化符号或搜索符号服务器。

#![cfg(all(windows, target_arch = "x86_64"))]

use std::cell::Cell;
use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::fs::FileExt as _;
use std::os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _};
use std::ptr;
use std::sync::Mutex;

use serde::Serialize;
use windows::Win32::Foundation::{
    CompareObjectHandles, ERROR_NOT_SAME_OBJECT, FILETIME, GENERIC_READ, GetLastError, HANDLE,
    SetLastError, WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetFileInformationByHandle, ReOpenFile,
};
use windows::Win32::System::Diagnostics::Debug::{
    ADDRESS64, AddrModeFlat, CONTEXT, CONTEXT_FULL_AMD64, STACKFRAME64, StackWalk64,
};
use windows::Win32::System::StationsAndDesktops::HDESK;
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_UNKNOWN,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThreadId, GetProcessId, GetProcessIdOfThread, GetProcessTimes,
    GetThreadId, GetThreadTimes, IsWow64Process2, ResumeThread, SuspendThread,
};
use windows::core::{BOOL, Error as WindowsError};

const MAX_MODULES: usize = 64;
const MAX_IMAGE_SIZE: u32 = 1024 * 1024 * 1024;
const MAX_EXCEPTION_BYTES: usize = 768 * 1024;
const MAX_TOTAL_EXCEPTION_BYTES: usize = 4 * 1024 * 1024;
const MAX_FRAMES: usize = 32;
const MAX_STACK_SPAN: u64 = 1024 * 1024;
const MAX_READ_BYTES: usize = 256 * 1024;
const MAX_READ_ONCE: usize = 16 * 1024;
const MAX_USER_ADDRESS: u64 = 0x0000_7fff_ffff_ffff;

// 不使用符号引擎或改变其全局选项；本模块内的 DbgHelp 调用仍须串行。
static STACK_WALK_LOCK: Mutex<()> = Mutex::new(());
thread_local! {
    static ACTIVE_WALK: Cell<*mut WalkState<'static>> = const { Cell::new(ptr::null_mut()) };
}

// 保留 API 返回后的即时 Win32 错误，避免 Result 包装后再查询线程错误槽。
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetThreadContext"]
    fn get_thread_context_raw(thread: HANDLE, context: *mut CONTEXT) -> BOOL;
    #[link_name = "ReadProcessMemory"]
    fn read_process_memory_raw(
        process: HANDLE,
        address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        read: *mut usize,
    ) -> BOOL;
}

#[link(name = "user32")]
unsafe extern "system" {
    #[link_name = "GetThreadDesktop"]
    fn get_thread_desktop_raw(thread_id: u32) -> HDESK;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct Failure {
    operation: &'static str,
    reason: &'static str,
    win32: Option<u32>,
    hresult: Option<i32>,
}

impl Failure {
    fn rejected(reason: &'static str) -> Self {
        Self {
            operation: "snapshot",
            reason,
            win32: None,
            hresult: None,
        }
    }

    fn last(operation: &'static str) -> Self {
        let code = unsafe { GetLastError() }.0;
        Self {
            operation,
            reason: "api_failed",
            win32: Some(code),
            hresult: None,
        }
    }

    fn windows(operation: &'static str, error: WindowsError) -> Self {
        let code = error.code().0;
        let bits = code as u32;
        Self {
            operation,
            reason: "api_failed",
            win32: (bits & 0xffff_0000 == 0x8007_0000).then_some(bits & 0xffff),
            hresult: Some(code),
        }
    }

    fn io(error: io::Error) -> Self {
        Self {
            operation: "read_leased_image",
            reason: "read_failed",
            win32: error.raw_os_error().map(|code| code as u32),
            hresult: None,
        }
    }
}

/// 文件及 SHA 必须来自仍持有的已核租约；base/size 来自同一进程的已核加载记录。
pub(super) struct VerifiedModule<'a> {
    pub(super) file: &'a File,
    pub(super) base: u64,
    pub(super) size: u32,
    pub(super) sha256: &'a str,
}

#[derive(Clone, Debug, Serialize)]
struct ModuleIdentity {
    base: u64,
    size: u32,
    volume_serial: u32,
    file_index: u64,
    sha256: String,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimeFunction {
    begin: u32,
    end: u32,
    unwind: u32,
}

struct Module {
    identity: ModuleIdentity,
    functions: Vec<RuntimeFunction>,
}

pub(super) struct PreparedModules {
    modules: Vec<Module>,
}

#[derive(Debug, Serialize)]
struct Identity {
    process_id: u32,
    process_created: u64,
    thread_id: u32,
    thread_created: u64,
}

#[derive(Debug, Serialize)]
struct ContextAddresses {
    instruction: u64,
    stack: u64,
    frame: Option<u64>,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct Frame {
    instruction: u64,
    stack: u64,
    module_index: Option<usize>,
    module_offset: Option<u64>,
}

#[derive(Debug, Serialize)]
struct DesktopComparison {
    same_object: Option<bool>,
    failure: Option<Failure>,
}

#[derive(Debug, Serialize)]
struct Suspension {
    previous_count: u32,
    resume_previous_count: Option<u32>,
    balanced: bool,
    failure: Option<Failure>,
}

#[derive(Debug, Serialize)]
pub(super) struct Snapshot {
    identity: Option<Identity>,
    modules: Vec<ModuleIdentity>,
    context: Option<ContextAddresses>,
    frames: Vec<Frame>,
    stack_stop: Option<Failure>,
    desktop: Option<DesktopComparison>,
    suspension: Option<Suspension>,
    failure: Option<Failure>,
}

impl Snapshot {
    /// 只报告本次增加的暂停计数是否已平衡；不代表其他诊断字段成功。
    pub(super) fn suspension_balanced(&self) -> bool {
        self.suspension
            .as_ref()
            .is_none_or(|suspension| suspension.balanced)
    }
}

// Windows x64 的 CONTEXT 要求 16 字节对齐，不依赖生成类型的默认 Rust 对齐。
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn contained(base: u64, size: u64, address: u64, length: u64) -> bool {
    length != 0
        && base <= address
        && address
            .checked_add(length)
            .zip(base.checked_add(size))
            .is_some_and(|(end, bound)| end <= bound && bound <= MAX_USER_ADDRESS)
}

fn read_exact_at(file: &File, offset: u64, length: usize) -> Result<Vec<u8>, Failure> {
    let mut bytes = vec![0; length];
    let mut done = 0;
    while done < length {
        let read = file
            .seek_read(&mut bytes[done..], offset + done as u64)
            .map_err(Failure::io)?;
        if read == 0 {
            return Err(Failure::rejected("truncated_leased_image"));
        }
        done += read;
    }
    Ok(bytes)
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn parse_functions(bytes: &[u8], image_size: u32) -> Result<Vec<RuntimeFunction>, Failure> {
    if bytes.len() > MAX_EXCEPTION_BYTES || bytes.len() % 12 != 0 {
        return Err(Failure::rejected("invalid_exception_table_size"));
    }
    let mut functions = Vec::with_capacity(bytes.len() / 12);
    let mut previous_end = 0;
    for row in bytes.chunks_exact(12) {
        let function = RuntimeFunction {
            begin: u32_at(row, 0),
            end: u32_at(row, 4),
            unwind: u32_at(row, 8),
        };
        if function.begin < previous_end
            || function.begin >= function.end
            || function.end > image_size
            || function.unwind == 0
            || function.unwind >= image_size
        {
            return Err(Failure::rejected("invalid_exception_table_entry"));
        }
        previous_end = function.end;
        functions.push(function);
    }
    Ok(functions)
}

fn load_functions(file: &File, image_size: u32) -> Result<Vec<RuntimeFunction>, Failure> {
    // Windows seek_read 会修改文件游标，DuplicateHandle 也共享该游标。
    // 只从仍持有的原文件对象建立独立读取句柄，避免改变生产租约的读位置。
    let reader = unsafe {
        ReOpenFile(
            HANDLE(file.as_raw_handle()),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAGS_AND_ATTRIBUTES(0),
        )
    }
    .map_err(|error| Failure::windows("ReOpenFile", error))?;
    let reader = unsafe { File::from_raw_handle(reader.0) };
    let file = &reader;
    let dos = read_exact_at(file, 0, 64)?;
    let pe_offset = u32_at(&dos, 60);
    if &dos[..2] != b"MZ" || !(64..=1024 * 1024).contains(&pe_offset) {
        return Err(Failure::rejected("invalid_dos_header"));
    }
    let header = read_exact_at(file, u64::from(pe_offset), 24)?;
    let sections = usize::from(u16_at(&header, 6));
    let optional_size = usize::from(u16_at(&header, 20));
    if &header[..4] != b"PE\0\0"
        || u16_at(&header, 4) != 0x8664
        || !(1..=96).contains(&sections)
        || !(144..=512).contains(&optional_size)
    {
        return Err(Failure::rejected("invalid_x64_pe_header"));
    }
    let optional = read_exact_at(file, u64::from(pe_offset) + 24, optional_size)?;
    if u16_at(&optional, 0) != 0x20b
        || u32_at(&optional, 56) != image_size
        || u32_at(&optional, 108) < 4
    {
        return Err(Failure::rejected("image_layout_mismatch"));
    }
    let rva = u32_at(&optional, 136);
    let length = u32_at(&optional, 140);
    if rva == 0 && length == 0 {
        return Ok(Vec::new());
    }
    if rva == 0
        || length == 0
        || length as usize > MAX_EXCEPTION_BYTES
        || rva.checked_add(length).is_none_or(|end| end > image_size)
    {
        return Err(Failure::rejected("invalid_exception_directory"));
    }
    let table = read_exact_at(
        file,
        u64::from(pe_offset) + 24 + optional_size as u64,
        sections * 40,
    )?;
    let mut offset = None;
    for section in table.chunks_exact(40) {
        let start = u32_at(section, 12);
        let raw_size = u32_at(section, 16);
        if rva >= start
            && rva
                .checked_sub(start)
                .and_then(|delta| delta.checked_add(length))
                .is_some_and(|end| end <= raw_size)
        {
            if offset.is_some() {
                return Err(Failure::rejected("ambiguous_exception_section"));
            }
            offset = Some(u64::from(u32_at(section, 20)) + u64::from(rva - start));
        }
    }
    let bytes = read_exact_at(
        file,
        offset.ok_or_else(|| Failure::rejected("exception_section_missing"))?,
        length as usize,
    )?;
    parse_functions(&bytes, image_size)
}

/// 在暂停线程之前准备；不访问目标进程内存、不改变文件游标、不按路径重新打开映像。
pub(super) fn prepare_modules(modules: &[VerifiedModule<'_>]) -> Result<PreparedModules, Failure> {
    if modules.len() > MAX_MODULES {
        return Err(Failure::rejected("module_count_out_of_bounds"));
    }
    let mut prepared = Vec::<Module>::with_capacity(modules.len());
    let mut total = 0;
    for module in modules {
        if module.size == 0
            || module.size > MAX_IMAGE_SIZE
            || !contained(module.base, u64::from(module.size), module.base, 1)
            || module.sha256.len() != 64
            || !module.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || prepared.iter().any(|other| {
                module.base < other.identity.base + u64::from(other.identity.size)
                    && other.identity.base < module.base + u64::from(module.size)
            })
        {
            return Err(Failure::rejected("invalid_module_binding"));
        }
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(module.file.as_raw_handle()), &mut info) }
            .map_err(|error| Failure::windows("GetFileInformationByHandle", error))?;
        let functions = load_functions(module.file, module.size)?;
        total += functions.len() * 12;
        if total > MAX_TOTAL_EXCEPTION_BYTES {
            return Err(Failure::rejected("exception_tables_limit"));
        }
        prepared.push(Module {
            identity: ModuleIdentity {
                base: module.base,
                size: module.size,
                volume_serial: info.dwVolumeSerialNumber,
                file_index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
                sha256: module.sha256.to_owned(),
            },
            functions,
        });
    }
    Ok(PreparedModules { modules: prepared })
}

struct SuspendedThread {
    thread: Option<HANDLE>,
    previous_count: u32,
}

impl SuspendedThread {
    fn acquire(thread: HANDLE) -> Result<Self, Failure> {
        unsafe { SetLastError(WIN32_ERROR(0)) };
        let previous_count = unsafe { SuspendThread(thread) };
        if previous_count == u32::MAX {
            return Err(Failure::last("SuspendThread"));
        }
        Ok(Self {
            thread: Some(thread),
            previous_count,
        })
    }

    fn finish(mut self) -> Suspension {
        let thread = self.thread.take().expect("暂停句柄仍由 guard 持有");
        unsafe { SetLastError(WIN32_ERROR(0)) };
        let count = unsafe { ResumeThread(thread) };
        let failure = (count == u32::MAX).then(|| Failure::last("ResumeThread"));
        Suspension {
            previous_count: self.previous_count,
            resume_previous_count: (count != u32::MAX).then_some(count),
            balanced: count != u32::MAX && self.previous_count.checked_add(1) == Some(count),
            failure,
        }
    }
}

impl Drop for SuspendedThread {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            // 仅异常退出兜底；不循环降低其他持有者的暂停计数。
            unsafe { ResumeThread(thread) };
        }
    }
}

fn desktop(thread_id: u32, expected: Option<HDESK>) -> DesktopComparison {
    let Some(expected) = expected.filter(|handle| !handle.is_invalid()) else {
        return DesktopComparison {
            same_object: None,
            failure: Some(Failure::rejected("expected_desktop_unavailable")),
        };
    };
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let actual = unsafe { get_thread_desktop_raw(thread_id) };
    if actual.is_invalid() {
        return DesktopComparison {
            same_object: None,
            failure: Some(Failure::last("GetThreadDesktop")),
        };
    }
    // GetThreadDesktop 返回借用对象；不得 CloseDesktop，名称也不代替对象比较。
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let same = unsafe { CompareObjectHandles(HANDLE(actual.0), HANDLE(expected.0)) }.as_bool();
    let error = unsafe { GetLastError() }.0;
    let failure = (!same && error != 0 && error != ERROR_NOT_SAME_OBJECT.0).then_some(Failure {
        operation: "CompareObjectHandles",
        reason: "api_failed",
        win32: Some(error),
        hresult: None,
    });
    DesktopComparison {
        same_object: failure.is_none().then_some(same),
        failure,
    }
}

fn identity(process: HANDLE, thread: HANDLE, expected_creation: u64) -> Result<Identity, Failure> {
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let process_id = unsafe { GetProcessId(process) };
    if process_id == 0 {
        return Err(Failure::last("GetProcessId"));
    }
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let thread_id = unsafe { GetThreadId(thread) };
    if thread_id == 0 {
        return Err(Failure::last("GetThreadId"));
    }
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let owner = unsafe { GetProcessIdOfThread(thread) };
    if owner == 0 {
        return Err(Failure::last("GetProcessIdOfThread"));
    }
    if owner != process_id
        || process_id == unsafe { GetCurrentProcessId() }
        || thread_id == unsafe { GetCurrentThreadId() }
    {
        return Err(Failure::rejected("thread_process_identity_mismatch"));
    }
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|error| Failure::windows("GetProcessTimes", error))?;
    let process_created = ticks(created);
    if process_created != expected_creation || ticks(exited) != 0 {
        return Err(Failure::rejected("process_creation_or_liveness_mismatch"));
    }
    unsafe { GetThreadTimes(thread, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|error| Failure::windows("GetThreadTimes", error))?;
    if ticks(exited) != 0 {
        return Err(Failure::rejected("thread_exited"));
    }
    let mut machine = IMAGE_FILE_MACHINE_UNKNOWN;
    let mut native = IMAGE_FILE_MACHINE_UNKNOWN;
    unsafe { IsWow64Process2(process, &mut machine, Some(&mut native)) }
        .map_err(|error| Failure::windows("IsWow64Process2", error))?;
    if machine != IMAGE_FILE_MACHINE_UNKNOWN || native != IMAGE_FILE_MACHINE_AMD64 {
        return Err(Failure::rejected("not_native_amd64"));
    }
    Ok(Identity {
        process_id,
        process_created,
        thread_id,
        thread_created: ticks(created),
    })
}

struct WalkState<'a> {
    process: HANDLE,
    modules: &'a PreparedModules,
    stack: u64,
    remaining: usize,
    failure: Option<Failure>,
}

impl PreparedModules {
    fn module(&self, address: u64) -> Option<(usize, &Module)> {
        self.modules.iter().enumerate().find(|(_, module)| {
            contained(
                module.identity.base,
                u64::from(module.identity.size),
                address,
                1,
            )
        })
    }

    fn frame(&self, instruction: u64, stack: u64) -> Frame {
        let bound = self.module(instruction);
        Frame {
            instruction,
            stack,
            module_index: bound.map(|(index, _)| index),
            module_offset: bound.map(|(_, module)| instruction - module.identity.base),
        }
    }
}

unsafe extern "system" fn read_memory(
    process: HANDLE,
    address: u64,
    buffer: *mut c_void,
    size: u32,
    read: *mut u32,
) -> BOOL {
    ACTIVE_WALK.with(|active| {
        let state = active.get();
        if state.is_null() || buffer.is_null() || read.is_null() {
            return BOOL(0);
        }
        let state = unsafe { &mut *state };
        unsafe { *read = 0 };
        let size = size as usize;
        let allowed = contained(state.stack, MAX_STACK_SPAN, address, size as u64)
            || state.modules.modules.iter().any(|module| {
                contained(
                    module.identity.base,
                    u64::from(module.identity.size),
                    address,
                    size as u64,
                )
            });
        if process != state.process || !allowed || size > MAX_READ_ONCE || size > state.remaining {
            state
                .failure
                .get_or_insert_with(|| Failure::rejected("stack_read_out_of_bounds"));
            return BOOL(0);
        }
        state.remaining -= size;
        let mut actual = 0;
        unsafe { SetLastError(WIN32_ERROR(0)) };
        let result = unsafe {
            read_process_memory_raw(process, address as *const c_void, buffer, size, &mut actual)
        };
        let failure = (!result.as_bool()).then(|| Failure::last("ReadProcessMemory"));
        if let Some(failure) = failure {
            state.failure.get_or_insert(failure);
        }
        if actual != size {
            state
                .failure
                .get_or_insert_with(|| Failure::rejected("partial_stack_read"));
            return BOOL(0);
        }
        unsafe { *read = actual as u32 };
        result
    })
}

unsafe extern "system" fn module_base(process: HANDLE, address: u64) -> u64 {
    ACTIVE_WALK.with(|active| {
        let state = active.get();
        if state.is_null() {
            return 0;
        }
        let state = unsafe { &*state };
        if process != state.process {
            return 0;
        }
        state
            .modules
            .module(address)
            .map_or(0, |(_, module)| module.identity.base)
    })
}

unsafe extern "system" fn function_table(process: HANDLE, address: u64) -> *mut c_void {
    ACTIVE_WALK.with(|active| {
        let state = active.get();
        if state.is_null() {
            return ptr::null_mut();
        }
        let state = unsafe { &*state };
        if process != state.process {
            return ptr::null_mut();
        }
        let Some((_, module)) = state.modules.module(address) else {
            return ptr::null_mut();
        };
        let rva = (address - module.identity.base) as u32;
        let index = module
            .functions
            .partition_point(|function| function.end <= rva);
        module
            .functions
            .get(index)
            .filter(|function| function.begin <= rva)
            .map_or(ptr::null_mut(), |function| {
                ptr::from_ref(function).cast_mut().cast()
            })
    })
}

struct ActiveWalk;
impl Drop for ActiveWalk {
    fn drop(&mut self) {
        ACTIVE_WALK.with(|active| active.set(ptr::null_mut()));
    }
}

fn walk(
    process: HANDLE,
    thread: HANDLE,
    context: &mut CONTEXT,
    modules: &PreparedModules,
    snapshot: &mut Snapshot,
) {
    let mut state = WalkState {
        process,
        modules,
        stack: context.Rsp,
        remaining: MAX_READ_BYTES,
        failure: None,
    };
    ACTIVE_WALK.with(|active| active.set(ptr::from_mut(&mut state).cast()));
    let active = ActiveWalk;
    let mut frame = STACKFRAME64 {
        AddrPC: ADDRESS64 {
            Offset: context.Rip,
            Mode: AddrModeFlat,
            ..Default::default()
        },
        AddrStack: ADDRESS64 {
            Offset: context.Rsp,
            Mode: AddrModeFlat,
            ..Default::default()
        },
        AddrFrame: ADDRESS64 {
            Offset: context.Rbp,
            Mode: AddrModeFlat,
            ..Default::default()
        },
        ..Default::default()
    };
    snapshot
        .frames
        .push(modules.frame(context.Rip, context.Rsp));
    for step in 0..MAX_FRAMES {
        if modules.module(frame.AddrPC.Offset).is_none() {
            snapshot.stack_stop = Some(Failure::rejected("instruction_outside_verified_modules"));
            break;
        }
        unsafe { SetLastError(WIN32_ERROR(0)) };
        let ok = unsafe {
            StackWalk64(
                0x8664,
                process,
                thread,
                &mut frame,
                ptr::from_mut(context).cast(),
                Some(read_memory),
                Some(function_table),
                Some(module_base),
                None,
            )
        };
        if !ok.as_bool() {
            // StackWalk64 通常不设置 LastError；不能把遗留错误槽归因于本次展开。
            // ReadProcessMemory 的即时错误由下方 state.failure 单独保留。
            snapshot.stack_stop = Some(Failure {
                operation: "StackWalk64",
                reason: "unwind_stopped",
                win32: None,
                hresult: None,
            });
            break;
        }
        if frame.AddrPC.Offset == 0 {
            break;
        }
        let next = modules.frame(frame.AddrPC.Offset, frame.AddrStack.Offset);
        if snapshot.frames.last() == Some(&next) {
            // StackWalk64 首次可返回初始帧；后续重复则停止，不能制造重复栈。
            if step == 0 {
                continue;
            }
            snapshot.stack_stop = Some(Failure::rejected("repeated_stack_frame"));
            break;
        }
        if !contained(state.stack, MAX_STACK_SPAN, next.stack, 1) {
            snapshot.stack_stop = Some(Failure::rejected("stack_pointer_out_of_bounds"));
            break;
        }
        snapshot.frames.push(next);
        if snapshot.frames.len() == MAX_FRAMES {
            snapshot.stack_stop = Some(Failure::rejected("frame_limit"));
            break;
        }
    }
    drop(active);
    if let Some(failure) = state.failure {
        snapshot.stack_stop = Some(failure);
    }
}

/// 仅在原调试 worker 无待 Continue 事件时调用；输入句柄和映像租约须保持至返回。
/// expected_desktop 须为经原 helper 进程句柄复制并仍持有的对象，不能按名称重新认领。
/// 返回后调用方必须检查 suspension_balanced()；失败不改变原取消/严格 Job 清理路径。
pub(super) fn capture(
    process: BorrowedHandle<'_>,
    process_creation_filetime: u64,
    thread: BorrowedHandle<'_>,
    expected_desktop: Option<HDESK>,
    modules: &PreparedModules,
) -> Snapshot {
    let mut snapshot = Snapshot {
        identity: None,
        modules: modules
            .modules
            .iter()
            .map(|module| module.identity.clone())
            .collect(),
        context: None,
        frames: Vec::new(),
        stack_stop: None,
        desktop: None,
        suspension: None,
        failure: None,
    };
    let process = HANDLE(process.as_raw_handle());
    let thread = HANDLE(thread.as_raw_handle());
    let identity = match identity(process, thread, process_creation_filetime) {
        Ok(identity) => identity,
        Err(failure) => {
            snapshot.failure = Some(failure);
            return snapshot;
        }
    };
    let thread_id = identity.thread_id;
    snapshot.identity = Some(identity);
    let Ok(_walk_lock) = STACK_WALK_LOCK.try_lock() else {
        snapshot.failure = Some(Failure::rejected("stack_walker_busy"));
        return snapshot;
    };
    if ACTIVE_WALK.with(|active| !active.get().is_null()) {
        snapshot.failure = Some(Failure::rejected("nested_stack_walk"));
        return snapshot;
    }
    let suspended = match SuspendedThread::acquire(thread) {
        Ok(guard) => guard,
        Err(failure) => {
            snapshot.failure = Some(failure);
            return snapshot;
        }
    };
    let mut context = AlignedContext(CONTEXT {
        ContextFlags: CONTEXT_FULL_AMD64,
        ..Default::default()
    });
    unsafe { SetLastError(WIN32_ERROR(0)) };
    if unsafe { get_thread_context_raw(thread, &mut context.0) }.as_bool() {
        snapshot.context = Some(ContextAddresses {
            instruction: context.0.Rip,
            stack: context.0.Rsp,
            // 优化代码可能将 RBP 用作普通寄存器；只输出栈范围内的地址。
            frame: contained(context.0.Rsp, MAX_STACK_SPAN, context.0.Rbp, 1)
                .then_some(context.0.Rbp),
        });
        walk(process, thread, &mut context.0, modules, &mut snapshot);
    } else {
        snapshot.failure = Some(Failure::last("GetThreadContext"));
    }
    snapshot.desktop = Some(desktop(thread_id, expected_desktop));
    snapshot.suspension = Some(suspended.finish());
    snapshot
}

#[cfg(test)]
#[path = "managed_process_atomic_windows_snapshot_tests.rs"]
mod tests;
