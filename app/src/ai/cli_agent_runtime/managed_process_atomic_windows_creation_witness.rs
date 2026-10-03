//! 固定私有 CMD 原始主线程的一次创建调用观察；不是授权或通用调试器。
//!
//! 调用方持续持有原 Job/token/映像租约，先验证 loader 事件再调用本模块，且独占
//! WaitForDebugEvent/ContinueDebugEvent。本模块不等待事件、不继续事件、不写映像、
//! 不按 PID 打开对象。所有修改只发生在调用方仍持有的 root 调试停点。
//! 未命中仅表示本次原始主线程观察没有结果，不能推断其他线程或动态解析 API 未调用。

#![cfg(all(
    windows,
    target_arch = "x86_64",
    any(test, feature = "cli-agent-native-witness")
))]

use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Read as _};
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use serde::Serialize;
use sha2::{Digest as _, Sha256};
use windows::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, EXCEPTION_BREAKPOINT, EXCEPTION_SINGLE_STEP, FILETIME,
    GENERIC_READ, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFileInformationByHandle, ReOpenFile,
};
use windows::Win32::System::Diagnostics::Debug::{
    CONTEXT, CONTEXT_CONTROL_AMD64, CONTEXT_DEBUG_REGISTERS_AMD64, CONTEXT_INTEGER_AMD64,
    CREATE_PROCESS_DEBUG_EVENT, DEBUG_EVENT, EXCEPTION_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT,
    EXIT_THREAD_DEBUG_EVENT,
};
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_IMAGE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, VirtualQueryEx,
};
use windows::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_UNKNOWN,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetProcessId, GetProcessIdOfThread,
    GetProcessTimes, GetThreadId, GetThreadTimes, IsWow64Process2, WaitForSingleObject,
};
use windows::core::{BOOL, Error as WindowsError};

const MAX_FILE: usize = 32 * 1024 * 1024;
const MAX_IMAGE: u32 = 1024 * 1024 * 1024;
const MAX_MODULES: usize = 64;
const MAX_IMPORTS: usize = 4096;
const MAX_USER: u64 = 0x0000_7fff_ffff_ffff;
const OWN_STATUS: u64 = 0xf;
const OTHER_DEBUG_STATUS: u64 = (1 << 13) | (1 << 14) | (1 << 15);

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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct Failure {
    operation: &'static str,
    reason: &'static str,
    win32: Option<u32>,
    hresult: Option<i32>,
}
impl Failure {
    /// 这些覆盖限制只在本次 SetThreadContext 之前产生；不包含身份、API 或布局失败。
    pub(super) fn coverage_unavailable(&self) -> bool {
        self.operation == "creation_witness"
            && self.win32.is_none()
            && self.hresult.is_none()
            && matches!(
                self.reason,
                "delay_import_coverage_unsupported"
                    | "no_supported_creator_import"
                    | "more_than_three_creator_entries"
                    | "creator_module_limit"
                    | "existing_debug_configuration_rejected"
            )
    }

    fn invalid(reason: &'static str) -> Self {
        Self {
            operation: "creation_witness",
            reason,
            win32: None,
            hresult: None,
        }
    }
    fn last(operation: &'static str) -> Self {
        Self {
            operation,
            reason: "api_failed",
            win32: Some(unsafe { GetLastError() }.0),
            hresult: None,
        }
    }
    fn windows(operation: &'static str, error: WindowsError) -> Self {
        let bits = error.code().0 as u32;
        Self {
            operation,
            reason: "api_failed",
            win32: (bits & 0xffff_0000 == 0x8007_0000).then_some(bits & 0xffff),
            hresult: Some(error.code().0),
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
type Result<T> = std::result::Result<T, Failure>;

/// 必须来自同一 root 进程的已验证 loader 事件及持续持有的租约，不能按名称猜映像。
pub(super) struct VerifiedImage<'a> {
    pub(super) file: &'a File,
    pub(super) base: u64,
    pub(super) size: u32,
    pub(super) sha256: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) enum Api {
    CreateProcessW,
    CreateProcessA,
    CreateProcessAsUserW,
    CreateProcessAsUserA,
}
impl Api {
    fn named(value: &[u8]) -> Option<Self> {
        match value {
            b"CreateProcessW" => Some(Self::CreateProcessW),
            b"CreateProcessA" => Some(Self::CreateProcessA),
            b"CreateProcessAsUserW" => Some(Self::CreateProcessAsUserW),
            b"CreateProcessAsUserA" => Some(Self::CreateProcessAsUserA),
            _ => None,
        }
    }
    fn information_offset(self) -> u64 {
        match self {
            Self::CreateProcessW | Self::CreateProcessA => 0x50,
            Self::CreateProcessAsUserW | Self::CreateProcessAsUserA => 0x58,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) enum Phase {
    Prepared,
    Armed,
    Entered,
    Returned,
    Withdrawn,
    Incomplete,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) enum Restoration {
    NotModified,
    Required,
    ReadbackVerified,
    ProcessExitedBeforeRestore,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Disposition {
    NotOwned,
    OwnedEntry,
    OwnedReturn,
}

/// 刻意没有地址、参数、模块路径、原始寄存器或内存字段。
#[derive(Debug, Serialize)]
pub(super) struct Summary {
    coverage: &'static str,
    absence_is_not_process_wide_evidence: bool,
    root_pid: u32,
    root_thread_id: u32,
    phase: Phase,
    supported_imports: Vec<Api>,
    armed_at_ms: Option<u128>,
    entered_at_ms: Option<u128>,
    returned_at_ms: Option<u128>,
    api: Option<Api>,
    returned_bool: Option<bool>,
    output_pid: Option<u32>,
    snapshot_started_ms: Option<u128>,
    snapshot_finished_ms: Option<u128>,
    snapshot_suspension_balanced: Option<bool>,
    cancellation_at_ms: Option<u128>,
    restoration: Restoration,
    restoration_at_ms: Option<u128>,
    original_process_exit_confirmed: bool,
    failure: Option<Failure>,
}
impl Summary {
    fn new(pid: u32, tid: u32) -> Self {
        Self {
            coverage: "original_root_thread_one_direct_import_call_only",
            absence_is_not_process_wide_evidence: true,
            root_pid: pid,
            root_thread_id: tid,
            phase: Phase::Prepared,
            supported_imports: Vec::new(),
            armed_at_ms: None,
            entered_at_ms: None,
            returned_at_ms: None,
            api: None,
            returned_bool: None,
            output_pid: None,
            snapshot_started_ms: None,
            snapshot_finished_ms: None,
            snapshot_suspension_balanced: None,
            cancellation_at_ms: None,
            restoration: Restoration::NotModified,
            restoration_at_ms: None,
            original_process_exit_confirmed: false,
            failure: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Section {
    rva: u32,
    span: u32,
    raw: u32,
    raw_size: u32,
    executable: bool,
}
struct Image {
    bytes: Vec<u8>,
    base: u64,
    size: u32,
    sections: Vec<Section>,
    directories: Vec<(u32, u32)>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Import {
    api: Api,
    iat_rva: u32,
}
#[derive(Clone)]
struct Entry {
    api: Api,
    address: u64,
    iat_rvas: Vec<u32>,
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| Failure::invalid("truncated_pe"))?
            .try_into()
            .expect("已检查两个字节"),
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| Failure::invalid("truncated_pe"))?
            .try_into()
            .expect("已检查四个字节"),
    ))
}
fn u64_at(bytes: &[u8], offset: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or_else(|| Failure::invalid("truncated_value"))?
            .try_into()
            .expect("已检查八个字节"),
    ))
}
fn user_range(address: u64, length: usize) -> bool {
    address != 0
        && length != 0
        && address
            .checked_add(length as u64)
            .is_some_and(|end| end <= MAX_USER)
}
fn file_identity(file: &File) -> Result<(u32, u64, u64, u32)> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|error| Failure::windows("GetFileInformationByHandle", error))?;
    if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0) != 0 {
        return Err(Failure::invalid("image_not_plain"));
    }
    Ok((
        info.dwVolumeSerialNumber,
        u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow),
        u64::from(info.nFileSizeHigh) << 32 | u64::from(info.nFileSizeLow),
        info.dwFileAttributes,
    ))
}
impl Image {
    fn load(image: &VerifiedImage<'_>, dll: bool) -> Result<Self> {
        if image.sha256.len() != 64
            || !image
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Failure::invalid("invalid_image_digest"));
        }
        let before = file_identity(image.file)?;
        if before.2 == 0 || before.2 > MAX_FILE as u64 {
            return Err(Failure::invalid("image_file_limit"));
        }
        // ReOpenFile 不改变租约文件的共享游标；仍从原对象读取，不按路径重开。
        let handle = unsafe {
            ReOpenFile(
                HANDLE(image.file.as_raw_handle()),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_FLAGS_AND_ATTRIBUTES(0),
            )
        }
        .map_err(|error| Failure::windows("ReOpenFile", error))?;
        let reader = unsafe { File::from_raw_handle(handle.0) };
        if file_identity(&reader)? != before {
            return Err(Failure::invalid("reopened_image_changed"));
        }
        let mut bytes = Vec::new();
        (&reader)
            .take(MAX_FILE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(Failure::io)?;
        if bytes.len() as u64 != before.2
            || file_identity(image.file)? != before
            || file_identity(&reader)? != before
            || format!("{:x}", Sha256::digest(&bytes)) != image.sha256
        {
            return Err(Failure::invalid("leased_image_digest_changed"));
        }
        Self::parse(bytes, image.base, image.size, dll)
    }
    fn parse(bytes: Vec<u8>, base: u64, size: u32, dll: bool) -> Result<Self> {
        if size == 0
            || size > MAX_IMAGE
            || !user_range(base, size as usize)
            || bytes.get(..2) != Some(b"MZ")
        {
            return Err(Failure::invalid("invalid_image_range"));
        }
        let pe = u32_at(&bytes, 60)? as usize;
        if !(64..=1024 * 1024).contains(&pe)
            || bytes.get(pe..pe + 4) != Some(b"PE\0\0")
            || u16_at(&bytes, pe + 4)? != 0x8664
        {
            return Err(Failure::invalid("not_x64_pe"));
        }
        let count = u16_at(&bytes, pe + 6)? as usize;
        let optional_size = u16_at(&bytes, pe + 20)? as usize;
        let attributes = u16_at(&bytes, pe + 22)?;
        let optional = pe + 24;
        if !(1..=96).contains(&count)
            || !(112..=512).contains(&optional_size)
            || attributes & 2 == 0
            || (attributes & 0x2000 != 0) != dll
            || u16_at(&bytes, optional)? != 0x20b
            || u32_at(&bytes, optional + 56)? != size
        {
            return Err(Failure::invalid("image_layout_mismatch"));
        }
        let directory_count = u32_at(&bytes, optional + 108)? as usize;
        if directory_count > 16 || 112 + directory_count * 8 > optional_size {
            return Err(Failure::invalid("invalid_data_directories"));
        }
        let headers = u32_at(&bytes, optional + 60)?;
        if headers as usize > bytes.len()
            || headers > size
            || (headers as usize) < optional + optional_size + count * 40
        {
            return Err(Failure::invalid("invalid_image_headers"));
        }
        let mut directories = Vec::new();
        for index in 0..directory_count {
            directories.push((
                u32_at(&bytes, optional + 112 + index * 8)?,
                u32_at(&bytes, optional + 116 + index * 8)?,
            ));
        }
        let mut sections: Vec<Section> = Vec::new();
        for index in 0..count {
            let offset = optional + optional_size + index * 40;
            let raw_size = u32_at(&bytes, offset + 16)?;
            let section = Section {
                rva: u32_at(&bytes, offset + 12)?,
                span: u32_at(&bytes, offset + 8)?.max(raw_size),
                raw: u32_at(&bytes, offset + 20)?,
                raw_size,
                executable: u32_at(&bytes, offset + 36)? & 0xa000_0000 == 0x2000_0000,
            };
            if section.span == 0
                || section.rva < headers
                || section.raw_size != 0 && section.raw < headers
                || section
                    .rva
                    .checked_add(section.span)
                    .is_none_or(|end| end > size)
                || (section.raw as usize)
                    .checked_add(raw_size as usize)
                    .is_none_or(|end| end > bytes.len())
                || sections.iter().any(|other| {
                    section.rva < other.rva + other.span && other.rva < section.rva + section.span
                        || section.raw_size != 0
                            && other.raw_size != 0
                            && u64::from(section.raw)
                                < u64::from(other.raw) + u64::from(other.raw_size)
                            && u64::from(other.raw)
                                < u64::from(section.raw) + u64::from(section.raw_size)
                })
            {
                return Err(Failure::invalid("ambiguous_pe_sections"));
            }
            sections.push(section);
        }
        Ok(Self {
            bytes,
            base,
            size,
            sections,
            directories,
        })
    }
    fn bytes_at(&self, rva: u32, length: usize) -> Result<&[u8]> {
        let section = self
            .sections
            .iter()
            .find(|section| rva >= section.rva && rva - section.rva < section.span)
            .ok_or_else(|| Failure::invalid("unmapped_rva"))?;
        let delta = (rva - section.rva) as usize;
        if delta
            .checked_add(length)
            .is_none_or(|end| end > section.raw_size as usize || end > section.span as usize)
        {
            return Err(Failure::invalid("rva_outside_file_section"));
        }
        let start = section.raw as usize + delta;
        self.bytes
            .get(start..start + length)
            .ok_or_else(|| Failure::invalid("rva_outside_file"))
    }
    fn executable(&self, address: u64, length: usize) -> bool {
        address
            .checked_sub(self.base)
            .and_then(|rva| u32::try_from(rva).ok())
            .is_some_and(|rva| {
                self.sections.iter().any(|section| {
                    section.executable
                        && rva >= section.rva
                        && u64::from(rva - section.rva) + length as u64 <= u64::from(section.span)
                })
            })
    }
    fn name(&self, rva: u32) -> Result<Vec<u8>> {
        let mut name = Vec::new();
        for offset in 0..256 {
            let at = rva
                .checked_add(offset)
                .ok_or_else(|| Failure::invalid("name_overflow"))?;
            let byte = self.bytes_at(at, 1)?[0];
            if byte == 0 {
                return if name.is_empty() {
                    Err(Failure::invalid("empty_import_name"))
                } else {
                    Ok(name)
                };
            }
            if !byte.is_ascii() {
                return Err(Failure::invalid("non_ascii_import_name"));
            }
            name.push(byte);
        }
        Err(Failure::invalid("unterminated_import_name"))
    }
    fn imports(&self) -> Result<Vec<Import>> {
        if self.directories.get(13).copied().unwrap_or_default() != (0, 0) {
            return Err(Failure::invalid("delay_import_coverage_unsupported"));
        }
        let (start, length) = self.directories.get(1).copied().unwrap_or_default();
        if start == 0 || length < 20 || length > 64 * 1024 {
            return Err(Failure::invalid("invalid_import_directory"));
        }
        self.bytes_at(start, length as usize)?;
        let mut imports = Vec::new();
        let mut total = 0;
        for index in 0..(length / 20).min(128) {
            let at = start
                .checked_add(index * 20)
                .ok_or_else(|| Failure::invalid("import_overflow"))?;
            let descriptor = self.bytes_at(at, 20)?;
            if descriptor.iter().all(|byte| *byte == 0) {
                return if imports.is_empty() {
                    Err(Failure::invalid("no_supported_creator_import"))
                } else {
                    Ok(imports)
                };
            }
            let lookup = u32_at(descriptor, 0)?;
            let iat = u32_at(descriptor, 16)?;
            let library = self.name(u32_at(descriptor, 12)?)?;
            if lookup == 0
                || iat == 0
                || library.contains(&b'/')
                || library.contains(&b'\\')
                || library.contains(&b':')
                || !library
                    .get(library.len().saturating_sub(4)..)
                    .is_some_and(|end| end.eq_ignore_ascii_case(b".dll"))
            {
                return Err(Failure::invalid("unbound_import_names_required"));
            }
            let mut terminated = false;
            for slot in 0..MAX_IMPORTS {
                total += 1;
                if total > MAX_IMPORTS {
                    return Err(Failure::invalid("import_count_limit"));
                }
                let name_rva = u64_at(
                    self.bytes_at(
                        lookup
                            .checked_add((slot * 8) as u32)
                            .ok_or_else(|| Failure::invalid("lookup_overflow"))?,
                        8,
                    )?,
                    0,
                )?;
                if name_rva == 0 {
                    terminated = true;
                    break;
                }
                // 不靠 ordinal 猜它是否为创建 API；这种映像的覆盖范围不能证明。
                if name_rva >> 32 != 0 {
                    return Err(Failure::invalid("ordinal_or_large_import_unsupported"));
                }
                let name = self.name(
                    (name_rva as u32)
                        .checked_add(2)
                        .ok_or_else(|| Failure::invalid("hint_name_overflow"))?,
                )?;
                if let Some(api) = Api::named(&name) {
                    let iat_rva = iat
                        .checked_add((slot * 8) as u32)
                        .ok_or_else(|| Failure::invalid("iat_overflow"))?;
                    self.bytes_at(iat_rva, 8)?;
                    if imports.len() >= 24
                        || imports.iter().any(|item: &Import| item.iat_rva == iat_rva)
                    {
                        return Err(Failure::invalid("ambiguous_creator_import"));
                    }
                    imports.push(Import { api, iat_rva });
                }
            }
            if !terminated {
                return Err(Failure::invalid("unterminated_lookup_table"));
            }
        }
        Err(Failure::invalid("unterminated_import_directory"))
    }
}

fn combine_entry(entries: &mut Vec<Entry>, import: Import, address: u64) -> Result<()> {
    if entries
        .iter()
        .any(|entry| entry.address == address && entry.api != import.api)
    {
        return Err(Failure::invalid("ambiguous_api_address"));
    }
    if let Some(entry) = entries.iter_mut().find(|entry| entry.api == import.api) {
        if entry.address != address {
            return Err(Failure::invalid("same_api_multiple_addresses"));
        }
        entry.iat_rvas.push(import.iat_rva);
    } else {
        if entries.len() == 3 {
            return Err(Failure::invalid("more_than_three_creator_entries"));
        }
        entries.push(Entry {
            api: import.api,
            address,
            iat_rvas: vec![import.iat_rva],
        });
    }
    Ok(())
}

fn read_memory(process: HANDLE, address: u64, length: usize) -> Result<Vec<u8>> {
    if !user_range(address, length) || length > 256 {
        return Err(Failure::invalid("memory_read_out_of_bounds"));
    }
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
        return Err(Failure::last("ReadProcessMemory"));
    }
    if read != length {
        return Err(Failure::invalid("partial_memory_read"));
    }
    Ok(bytes)
}
fn image_region(
    process: HANDLE,
    base: u64,
    address: u64,
    length: usize,
    executable: bool,
) -> Result<()> {
    let mut info = MEMORY_BASIC_INFORMATION::default();
    let size = unsafe {
        VirtualQueryEx(
            process,
            Some(address as *const c_void),
            &mut info,
            std::mem::size_of_val(&info),
        )
    };
    if size == 0 {
        return Err(Failure::last("VirtualQueryEx"));
    }
    if size != std::mem::size_of_val(&info)
        || !user_range(address, length)
        || info.AllocationBase as u64 != base
        || info.State != MEM_COMMIT
        || info.Type != MEM_IMAGE
        || info.Protect.0 & (PAGE_GUARD.0 | PAGE_NOACCESS.0) != 0
        || address < info.BaseAddress as u64
        || address.checked_add(length as u64).is_none_or(|end| {
            (info.BaseAddress as u64)
                .checked_add(info.RegionSize as u64)
                .is_none_or(|region_end| end > region_end)
        })
        || executable
            && ![PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_WRITECOPY].contains(&info.Protect)
    {
        return Err(Failure::invalid("remote_image_region_mismatch"));
    }
    Ok(())
}
fn instruction(process: HANDLE, image: &Image, address: u64, length: usize) -> Result<Vec<u8>> {
    if !image.executable(address, length) {
        return Err(Failure::invalid("instruction_outside_verified_code"));
    }
    image_region(process, image.base, address, length, true)?;
    let rva = u32::try_from(address - image.base)
        .map_err(|_| Failure::invalid("call_site_rva_overflow"))?;
    let actual = read_memory(process, address, length)?;
    if actual != image.bytes_at(rva, length)? {
        return Err(Failure::invalid("verified_instruction_changed"));
    }
    Ok(actual)
}
fn relative_target(end: u64, displacement: &[u8]) -> Result<u64> {
    let value = i32::from_le_bytes(
        displacement
            .try_into()
            .map_err(|_| Failure::invalid("invalid_relative_instruction"))?,
    ) as i64;
    end.checked_add_signed(value)
        .filter(|address| user_range(*address, 1))
        .ok_or_else(|| Failure::invalid("relative_target_overflow"))
}
fn checked_iat_slot(root: &Image, entry: &Entry, slot: u64) -> Result<()> {
    if !entry
        .iat_rvas
        .iter()
        .any(|rva| root.base + u64::from(*rva) == slot)
    {
        return Err(Failure::invalid("return_call_uses_another_iat_slot"));
    }
    Ok(())
}
fn direct_iat_call(process: HANDLE, root: &Image, returned_to: u64, entry: &Entry) -> Result<()> {
    // 只识别两种固定 CALL 形态，不作启发式栈扫描或通用反汇编。
    // 先按已核文件范围判断 6 字节形态是否可能；真实远程读取失败必须原样返回。
    let direct = match returned_to
        .checked_sub(6)
        .filter(|address| root.executable(*address, 6))
    {
        Some(address) => Some(instruction(process, root, address, 6)?),
        None => None,
    };
    let slot = if let Some(bytes) = direct.filter(|bytes| bytes[..2] == [0xff, 0x15]) {
        relative_target(returned_to, &bytes[2..])?
    } else {
        let call = instruction(
            process,
            root,
            returned_to
                .checked_sub(5)
                .ok_or_else(|| Failure::invalid("invalid_return_address"))?,
            5,
        )?;
        if call[0] != 0xe8 {
            return Err(Failure::invalid("indirect_call_shape_unsupported"));
        }
        let thunk = relative_target(returned_to, &call[1..])?;
        let jump = instruction(process, root, thunk, 6)?;
        if jump[..2] != [0xff, 0x25] {
            return Err(Failure::invalid("import_thunk_shape_unsupported"));
        }
        relative_target(thunk + 6, &jump[2..])?
    };
    checked_iat_slot(root, entry, slot)?;
    image_region(process, root.base, slot, 8, false)?;
    if u64_at(&read_memory(process, slot, 8)?, 0)? != entry.address {
        return Err(Failure::invalid("iat_changed_since_arm"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DebugRegisters {
    address: [u64; 4],
    status: u64,
    control: u64,
}
impl DebugRegisters {
    fn read(context: &CONTEXT) -> Self {
        Self {
            address: [context.Dr0, context.Dr1, context.Dr2, context.Dr3],
            status: context.Dr6,
            control: context.Dr7,
        }
    }
    fn vacant(self, eflags: u32) -> bool {
        self.address == [0; 4]
            && self.control & !(1 << 10) == 0
            && self.status & (OWN_STATUS | OTHER_DEBUG_STATUS) == 0
            && eflags & 0x100 == 0
    }
    fn same_configuration(self, other: Self) -> bool {
        self.address == other.address
            && self.control == other.control
            && self.status & !OWN_STATUS == other.status & !OWN_STATUS
    }
    fn entries(self, entries: &[Entry]) -> Self {
        let mut registers = self;
        registers.status &= !OWN_STATUS;
        for (slot, entry) in entries.iter().enumerate() {
            registers.address[slot] = entry.address;
            registers.control |= 1 << (slot * 2);
        }
        registers
    }
    fn returned(self, address: u64) -> Self {
        Self {
            address: [0, 0, 0, address],
            status: self.status & !OWN_STATUS,
            control: self.control | (1 << 6),
        }
    }
    fn write(self, context: &mut CONTEXT) {
        [context.Dr0, context.Dr1, context.Dr2, context.Dr3] = self.address;
        context.Dr6 = self.status;
        context.Dr7 = self.control;
    }
}
fn restorable(
    current: DebugRegisters,
    original: DebugRegisters,
    expected: Option<DebugRegisters>,
    previous: Option<DebugRegisters>,
) -> bool {
    // Set 结果不明时仅允许之前或目标的完整自有配置；部分写入、外部配置均不覆盖。
    current == original
        || expected.is_some_and(|owned| current.same_configuration(owned))
        || previous.is_some_and(|owned| current.same_configuration(owned))
}
fn owned_slot(
    expected: DebugRegisters,
    context: &CONTEXT,
    exception_address: u64,
) -> Option<usize> {
    let actual = DebugRegisters::read(context);
    let hit = actual.status & OWN_STATUS;
    if !actual.same_configuration(expected)
        || !hit.is_power_of_two()
        || context.EFlags & 0x100 != 0
        || exception_address != context.Rip
    {
        return None;
    }
    let slot = hit.trailing_zeros() as usize;
    (expected.control & (1 << (slot * 2)) != 0 && expected.address[slot] == context.Rip)
        .then_some(slot)
}
#[repr(align(16))]
struct AlignedContext(CONTEXT);
struct PendingCall {
    api: Api,
    stack: u64,
    return_address: u64,
    information: u64,
}
fn matching_return(pending: &PendingCall, context: &CONTEXT) -> bool {
    context.Rip == pending.return_address && pending.stack.checked_add(8) == Some(context.Rsp)
}
fn duplicate(handle: HANDLE) -> Result<OwnedHandle> {
    let current = unsafe { GetCurrentProcess() };
    let mut result = HANDLE::default();
    unsafe {
        DuplicateHandle(
            current,
            handle,
            current,
            &mut result,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .map_err(|error| Failure::windows("DuplicateHandle", error))?;
    Ok(unsafe { OwnedHandle::from_raw_handle(result.0) })
}
fn process_time(handle: HANDLE) -> Result<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|error| Failure::windows("GetProcessTimes", error))?;
    Ok(u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime))
}
fn thread_time(handle: HANDLE) -> Result<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetThreadTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|error| Failure::windows("GetThreadTimes", error))?;
    Ok(u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime))
}

/// 不能在仍有修改的活线程上直接丢弃；必须显式恢复读回，或由原 Job 清理后核原句柄退出。
#[must_use]
pub(super) struct CreationWitness {
    process: OwnedHandle,
    thread: OwnedHandle,
    process_created: u64,
    thread_created: u64,
    debugger_thread: u32,
    root: Image,
    imports: Vec<Import>,
    entries: Vec<Entry>,
    targets: Vec<Image>,
    original: Option<DebugRegisters>,
    expected: Option<DebugRegisters>,
    previous_expected: Option<DebugRegisters>,
    pending: Option<PendingCall>,
    dirty: bool,
    last_ms: u128,
    summary: Summary,
}
impl std::fmt::Debug for CreationWitness {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.summary.fmt(formatter)
    }
}
impl CreationWitness {
    /// root.file 必须是已核 CMD 原映像，事件必须仍由调用方拥有且未继续。
    pub(super) fn new(
        event: &DEBUG_EVENT,
        root: VerifiedImage<'_>,
        expected_process_creation_filetime: u64,
        at_ms: u128,
    ) -> Result<Self> {
        if event.dwDebugEventCode != CREATE_PROCESS_DEBUG_EVENT
            || event.dwProcessId == 0
            || event.dwProcessId == unsafe { GetCurrentProcessId() }
        {
            return Err(Failure::invalid("original_root_create_event_required"));
        }
        let info = unsafe { event.u.CreateProcessInfo };
        if info.lpBaseOfImage as u64 != root.base {
            return Err(Failure::invalid("root_image_base_mismatch"));
        }
        let process = duplicate(info.hProcess)?;
        let thread = duplicate(info.hThread)?;
        let process_handle = HANDLE(process.as_raw_handle());
        let thread_handle = HANDLE(thread.as_raw_handle());
        if unsafe { GetProcessId(process_handle) } != event.dwProcessId
            || unsafe { GetThreadId(thread_handle) } != event.dwThreadId
            || unsafe { GetProcessIdOfThread(thread_handle) } != event.dwProcessId
            || expected_process_creation_filetime == 0
            || process_time(process_handle)? != expected_process_creation_filetime
        {
            return Err(Failure::invalid("original_object_identity_mismatch"));
        }
        let mut emulated = IMAGE_FILE_MACHINE_UNKNOWN;
        let mut native = IMAGE_FILE_MACHINE_UNKNOWN;
        unsafe { IsWow64Process2(process_handle, &mut emulated, Some(&mut native)) }
            .map_err(|error| Failure::windows("IsWow64Process2", error))?;
        if emulated != IMAGE_FILE_MACHINE_UNKNOWN || native != IMAGE_FILE_MACHINE_AMD64 {
            return Err(Failure::invalid("native_x64_required"));
        }
        let root = Image::load(&root, false)?;
        let imports = root.imports()?;
        let thread_created = thread_time(thread_handle)?;
        Ok(Self {
            process,
            thread,
            process_created: expected_process_creation_filetime,
            thread_created,
            debugger_thread: unsafe { GetCurrentThreadId() },
            root,
            imports,
            entries: Vec::new(),
            targets: Vec::new(),
            original: None,
            expected: None,
            previous_expected: None,
            pending: None,
            dirty: false,
            last_ms: at_ms,
            summary: Summary::new(event.dwProcessId, event.dwThreadId),
        })
    }
    fn time(&mut self, at_ms: u128) -> Result<()> {
        if at_ms < self.last_ms {
            return Err(Failure::invalid("timeline_moved_backwards"));
        }
        self.last_ms = at_ms;
        Ok(())
    }
    fn stopped(&mut self, event: &DEBUG_EVENT, at_ms: u128) -> Result<()> {
        self.time(at_ms)?;
        if unsafe { GetCurrentThreadId() } != self.debugger_thread
            || event.dwProcessId != self.summary.root_pid
            || event.dwDebugEventCode == EXIT_PROCESS_DEBUG_EVENT
            || event.dwDebugEventCode == EXIT_THREAD_DEBUG_EVENT
                && event.dwThreadId == self.summary.root_thread_id
        {
            return Err(Failure::invalid("live_owned_root_debug_stop_required"));
        }
        Ok(())
    }
    fn context(&self) -> Result<AlignedContext> {
        let process = HANDLE(self.process.as_raw_handle());
        let thread = HANDLE(self.thread.as_raw_handle());
        if process_time(process)? != self.process_created
            || thread_time(thread)? != self.thread_created
            || unsafe { GetProcessIdOfThread(thread) } != self.summary.root_pid
        {
            return Err(Failure::invalid("original_thread_identity_changed"));
        }
        let mut context = AlignedContext(CONTEXT {
            ContextFlags: CONTEXT_CONTROL_AMD64
                | CONTEXT_INTEGER_AMD64
                | CONTEXT_DEBUG_REGISTERS_AMD64,
            ..Default::default()
        });
        if !unsafe { get_context_raw(thread, &mut context.0) }.as_bool() {
            return Err(Failure::last("GetThreadContext"));
        }
        Ok(context)
    }
    fn set_registers(&mut self, desired: DebugRegisters) -> Result<()> {
        // 先标为需要恢复；Set 失败也不能假设没有部分修改。
        self.dirty = true;
        self.summary.restoration = Restoration::Required;
        self.previous_expected = self.expected.or(self.original);
        self.expected = Some(desired);
        let mut context = AlignedContext(CONTEXT {
            ContextFlags: CONTEXT_DEBUG_REGISTERS_AMD64,
            ..Default::default()
        });
        desired.write(&mut context.0);
        if !unsafe { set_context_raw(HANDLE(self.thread.as_raw_handle()), &context.0) }.as_bool() {
            return Err(Failure::last("SetThreadContext"));
        }
        if DebugRegisters::read(&self.context()?.0) != desired {
            return Err(Failure::invalid("debug_register_readback_mismatch"));
        }
        self.previous_expected = None;
        Ok(())
    }
    fn restore(&mut self, at_ms: u128) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let original = self
            .original
            .ok_or_else(|| Failure::invalid("original_debug_context_missing"))?;
        let current = DebugRegisters::read(&self.context()?.0);
        if !restorable(current, original, self.expected, self.previous_expected) {
            return Err(Failure::invalid("debug_register_ownership_lost"));
        }
        self.set_registers(original)?;
        self.dirty = false;
        self.summary.restoration = Restoration::ReadbackVerified;
        self.summary.restoration_at_ms = Some(at_ms);
        Ok(())
    }
    fn failed<T>(&mut self, result: Result<T>) -> Result<T> {
        if let Err(failure) = &result {
            self.summary.phase = Phase::Incomplete;
            if self.summary.failure.is_none() {
                self.summary.failure = Some(failure.clone());
            }
        }
        result
    }
    /// modules 仅含同一 root 已核加载的 DLL，不含已单独绑定的 root 映像。
    pub(super) fn arm_on_initial_breakpoint(
        &mut self,
        event: &DEBUG_EVENT,
        modules: &[VerifiedImage<'_>],
        at_ms: u128,
    ) -> Result<()> {
        let result = self.arm_inner(event, modules, at_ms);
        self.failed(result)
    }
    fn arm_inner(
        &mut self,
        event: &DEBUG_EVENT,
        modules: &[VerifiedImage<'_>],
        at_ms: u128,
    ) -> Result<()> {
        self.stopped(event, at_ms)?;
        if self.summary.phase != Phase::Prepared
            || self.summary.cancellation_at_ms.is_some()
            || event.dwThreadId != self.summary.root_thread_id
            || event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT
        {
            return Err(Failure::invalid("initial_root_breakpoint_required"));
        }
        let exception = unsafe { event.u.Exception };
        if exception.dwFirstChance != 1
            || exception.ExceptionRecord.ExceptionCode != EXCEPTION_BREAKPOINT
            || modules.is_empty()
            || modules.len() > MAX_MODULES
        {
            return Err(Failure::invalid("initial_breakpoint_or_module_set_invalid"));
        }
        for (index, image) in modules.iter().enumerate() {
            if image.size == 0
                || image.size > MAX_IMAGE
                || !user_range(image.base, image.size as usize)
                || image.base < self.root.base + u64::from(self.root.size)
                    && self.root.base < image.base + u64::from(image.size)
                || modules[..index].iter().any(|other| {
                    image.base < other.base + u64::from(other.size)
                        && other.base < image.base + u64::from(image.size)
                })
            {
                return Err(Failure::invalid("overlapping_loaded_images"));
            }
        }
        let process = HANDLE(self.process.as_raw_handle());
        let mut entries = Vec::new();
        let mut targets: Vec<Image> = Vec::new();
        for import in &self.imports {
            let slot = self.root.base + u64::from(import.iat_rva);
            image_region(process, self.root.base, slot, 8, false)?;
            let address = u64_at(&read_memory(process, slot, 8)?, 0)?;
            let image = modules
                .iter()
                .find(|image| address >= image.base && address - image.base < u64::from(image.size))
                .ok_or_else(|| Failure::invalid("iat_target_not_in_verified_module"))?;
            if !targets.iter().any(|target| target.base == image.base) {
                if targets.len() >= 3 {
                    return Err(Failure::invalid("creator_module_limit"));
                }
                targets.push(Image::load(image, true)?);
            }
            let target = targets
                .iter()
                .find(|target| target.base == image.base)
                .expect("刚检查或添加的映像");
            instruction(process, target, address, 1)?;
            combine_entry(&mut entries, *import, address)?;
        }
        let context = self.context()?;
        let original = DebugRegisters::read(&context.0);
        if !original.vacant(context.0.EFlags) {
            return Err(Failure::invalid("existing_debug_configuration_rejected"));
        }
        self.original = Some(original);
        self.summary.supported_imports = entries.iter().map(|entry| entry.api).collect();
        self.set_registers(original.entries(&entries))?;
        self.entries = entries;
        self.targets = targets;
        self.summary.phase = Phase::Armed;
        self.summary.armed_at_ms = Some(at_ms);
        Ok(())
    }
    /// 仅 Owned* 可以由原事件拥有者按 DBG_CONTINUE 消费；Err 仍保留 pending 事件供原清理路径处理。
    /// Err 后不能继续业务事件循环，必须在原停点恢复或走精确 Job 终止及原句柄退出核验。
    pub(super) fn observe_exception(
        &mut self,
        event: &DEBUG_EVENT,
        at_ms: u128,
    ) -> Result<Disposition> {
        let result = self.observe_inner(event, at_ms);
        self.failed(result)
    }
    fn observe_inner(&mut self, event: &DEBUG_EVENT, at_ms: u128) -> Result<Disposition> {
        if self.summary.cancellation_at_ms.is_some()
            || event.dwProcessId != self.summary.root_pid
            || event.dwThreadId != self.summary.root_thread_id
            || event.dwDebugEventCode != EXCEPTION_DEBUG_EVENT
        {
            return Ok(Disposition::NotOwned);
        }
        self.stopped(event, at_ms)?;
        if !self.dirty || !matches!(self.summary.phase, Phase::Armed | Phase::Entered) {
            return Ok(Disposition::NotOwned);
        }
        let exception = unsafe { event.u.Exception };
        let context = self.context()?;
        let slot = if exception.dwFirstChance == 1
            && exception.ExceptionRecord.ExceptionCode == EXCEPTION_SINGLE_STEP
        {
            self.expected.and_then(|expected| {
                owned_slot(
                    expected,
                    &context.0,
                    exception.ExceptionRecord.ExceptionAddress as u64,
                )
            })
        } else {
            None
        };
        let Some(slot) = slot else {
            // 不吞外部异常；停止本次配对，避免异常展开后把旧返回点误当正常返回。
            self.restore(at_ms)?;
            self.pending = None;
            self.summary.phase = Phase::Withdrawn;
            self.summary.failure = Some(Failure::invalid("unowned_exception_outside_coverage"));
            return Ok(Disposition::NotOwned);
        };
        if self.summary.phase == Phase::Armed {
            let entry = self
                .entries
                .get(slot)
                .cloned()
                .ok_or_else(|| Failure::invalid("unexpected_entry_slot"))?;
            let target = self
                .targets
                .iter()
                .find(|image| {
                    entry.address >= image.base
                        && entry.address - image.base < u64::from(image.size)
                })
                .ok_or_else(|| Failure::invalid("owned_entry_module_missing"))?;
            instruction(
                HANDLE(self.process.as_raw_handle()),
                target,
                entry.address,
                1,
            )?;
            if context.0.Rsp & 15 != 8 {
                return Err(Failure::invalid("nonstandard_entry_stack"));
            }
            let returned_to = u64_at(
                &read_memory(HANDLE(self.process.as_raw_handle()), context.0.Rsp, 8)?,
                0,
            )?;
            direct_iat_call(
                HANDLE(self.process.as_raw_handle()),
                &self.root,
                returned_to,
                &entry,
            )?;
            let information_slot = context
                .0
                .Rsp
                .checked_add(entry.api.information_offset())
                .ok_or_else(|| Failure::invalid("argument_slot_overflow"))?;
            let information = u64_at(
                &read_memory(HANDLE(self.process.as_raw_handle()), information_slot, 8)?,
                0,
            )?;
            if !user_range(information, 24) {
                return Err(Failure::invalid("invalid_process_information_pointer"));
            }
            self.pending = Some(PendingCall {
                api: entry.api,
                stack: context.0.Rsp,
                return_address: returned_to,
                information,
            });
            self.summary.api = Some(entry.api);
            self.summary.entered_at_ms = Some(at_ms);
            self.set_registers(
                self.original
                    .ok_or_else(|| Failure::invalid("original_debug_context_missing"))?
                    .returned(returned_to),
            )?;
            self.summary.phase = Phase::Entered;
            Ok(Disposition::OwnedEntry)
        } else {
            let pending = self
                .pending
                .as_ref()
                .ok_or_else(|| Failure::invalid("pending_creation_missing"))?;
            if slot != 3
                || !matching_return(pending, &context.0)
                || self.summary.api != Some(pending.api)
            {
                return Err(Failure::invalid("return_stack_or_call_identity_mismatch"));
            }
            instruction(
                HANDLE(self.process.as_raw_handle()),
                &self.root,
                pending.return_address,
                1,
            )?;
            let succeeded = context.0.Rax as u32 != 0;
            self.summary.returned_bool = Some(succeeded);
            self.summary.returned_at_ms = Some(at_ms);
            if succeeded {
                let output = read_memory(
                    HANDLE(self.process.as_raw_handle()),
                    pending.information,
                    24,
                )?;
                let pid = u32_at(&output, 16)?;
                if pid == 0 || pid == self.summary.root_pid {
                    return Err(Failure::invalid("invalid_returned_process_id"));
                }
                self.summary.output_pid = Some(pid);
            }
            self.restore(at_ms)?;
            self.pending = None;
            self.summary.phase = Phase::Returned;
            Ok(Disposition::OwnedReturn)
        }
    }
    pub(super) fn withdraw_while_stopped(
        &mut self,
        event: &DEBUG_EVENT,
        at_ms: u128,
    ) -> Result<()> {
        let result = (|| {
            self.stopped(event, at_ms)?;
            self.restore(at_ms)?;
            self.pending = None;
            if self.summary.phase != Phase::Returned {
                self.summary.phase = Phase::Withdrawn;
            }
            Ok(())
        })();
        self.failed(result)
    }
    /// 时间与 balanced 来自调用方实际快照收据；本模块不自行生成或扩大快照证据。
    pub(super) fn note_pre_cancel_snapshot(
        &mut self,
        started_ms: u128,
        finished_ms: u128,
        suspension_balanced: bool,
    ) -> Result<()> {
        if self.summary.snapshot_started_ms.is_some()
            || self.summary.cancellation_at_ms.is_some()
            || started_ms < self.last_ms
            || finished_ms < started_ms
        {
            return Err(Failure::invalid("snapshot_timeline_invalid"));
        }
        self.time(finished_ms)?;
        self.summary.snapshot_started_ms = Some(started_ms);
        self.summary.snapshot_finished_ms = Some(finished_ms);
        self.summary.snapshot_suspension_balanced = Some(suspension_balanced);
        Ok(())
    }
    pub(super) fn note_cancel(&mut self, at_ms: u128) -> Result<()> {
        if self.summary.cancellation_at_ms.is_some() {
            return Err(Failure::invalid("duplicate_cancellation_time"));
        }
        self.time(at_ms)?;
        self.summary.cancellation_at_ms = Some(at_ms);
        if self.summary.phase != Phase::Returned {
            self.summary.phase = Phase::Incomplete;
        }
        Ok(())
    }
    /// 仅原 Job 清理和 EXIT 继续后调用；不把退出当成寄存器恢复成功。
    pub(super) fn confirm_process_exit(&mut self, at_ms: u128) -> Result<()> {
        self.time(at_ms)?;
        for handle in [
            HANDLE(self.process.as_raw_handle()),
            HANDLE(self.thread.as_raw_handle()),
        ] {
            let state = unsafe { WaitForSingleObject(handle, 0) };
            if state == WAIT_FAILED {
                return Err(Failure::last("WaitForSingleObject"));
            }
            if state != WAIT_OBJECT_0 {
                return Err(Failure::invalid("original_objects_exit_unconfirmed"));
            }
        }
        self.summary.original_process_exit_confirmed = true;
        if self.dirty {
            self.summary.restoration = Restoration::ProcessExitedBeforeRestore;
            self.dirty = false;
        }
        if self.summary.phase != Phase::Returned {
            self.summary.phase = Phase::Incomplete;
        }
        Ok(())
    }
    pub(super) fn requires_restore_or_original_exit(&self) -> bool {
        self.dirty
    }
    pub(super) fn summary(&self) -> &Summary {
        &self.summary
    }
}

#[cfg(test)]
#[path = "managed_process_atomic_windows_creation_witness_tests.rs"]
mod tests;
