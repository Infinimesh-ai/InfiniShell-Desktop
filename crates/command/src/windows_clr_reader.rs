//! 固定 CLR 停点的独立只读 reader 控制器；不持有或继续目标调试事件。

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read as _, Seek as _, SeekFrom, Write as _};
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_HANDLE_OPTIONS, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, GetFileVersionInfoSizeW, GetFileVersionInfoW,
    GetFinalPathNameByHandleW, VerQueryValueW, BY_HANDLE_FILE_INFORMATION,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
    FILE_SHARE_READ, VS_FIXEDFILEINFO,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::SystemInformation::{GetTickCount64, GetWindowsDirectoryW};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessId, GetProcessIdOfThread, GetProcessTimes, GetThreadId,
    GetThreadTimes, WaitForSingleObject, PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_VM_READ, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION,
};

use crate::blocking::Command;

pub(super) const MAX_FILE: u64 = 64 * 1024 * 1024;
pub(super) const MAX_OUTPUT: u64 = 32768;
const REQUEST_SIZE: usize = 168;
const MANAGED_EXTENSION_SIZE: usize = 200;
const MAX_POLL: Duration = Duration::from_millis(100);
const FIXTURE_READ_BYTES: u32 = 16 * 1024 * 1024;
const POWERSHELL_READ_BYTES: u32 = 24 * 1024 * 1024;
const CLR_EXCEPTION: u32 = 0xe0434352;
const NATIVE_RETURN_SINGLE_STEP: u32 = 0x80000004;

/// 只允许原停点实际方法中的固定 IL 与局部变量索引，不触发目标求值。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClrManagedMethodSpec {
    pub module_mvid: String,
    pub method_token: u32,
    pub approved_il_offsets: Vec<u32>,
    pub bool_local_index: u32,
    pub start_info_local_index: Option<u32>,
}

impl ClrManagedMethodSpec {
    fn validate(&self) -> io::Result<()> {
        let mvid = uuid::Uuid::parse_str(&self.module_mvid).map_err(io::Error::other)?;
        require(
            !mvid.is_nil()
                && mvid.to_string() == self.module_mvid
                && self.method_token & 0xff00_0000 == 0x0600_0000
                && self.method_token & 0x00ff_ffff != 0
                && !self.approved_il_offsets.is_empty()
                && self.approved_il_offsets.len() <= 4
                && self
                    .approved_il_offsets
                    .iter()
                    .enumerate()
                    .all(|(index, il)| {
                        *il < 0x10000 && !self.approved_il_offsets[..index].contains(il)
                    })
                && self.bool_local_index < 256
                && self
                    .start_info_local_index
                    .is_none_or(|index| index < 256 && index != self.bool_local_index),
            "CLR 托管方法规格无效",
        )
    }
}

/// 只由已回收 reader 的私有回复构造；应用层不能制造地址票据。
#[derive(Clone)]
pub struct ManagedContinuationTarget {
    pub(crate) process_id: u32,
    pub(crate) thread_id: u32,
    pub(crate) process_birth: u64,
    pub(crate) thread_birth: u64,
    pub(crate) pre_sequence: u64,
    pub(crate) pre_nonce: [u8; 16],
    pub(crate) module_mvid: String,
    pub(crate) method_token: u32,
    pub(crate) il_offset: u32,
    pub(crate) enc_version: u32,
    pub(crate) frame_rsp: u64,
    pub(crate) extent_start: u64,
    pub(crate) extent_end: u64,
    pub(crate) address: u64,
    pub(crate) map_count: u32,
    pub(crate) map_sha256: [u8; 32],
    pub(crate) code_sha256: [u8; 32],
    pub(crate) spec: ClrManagedMethodSpec,
}

impl std::fmt::Debug for ManagedContinuationTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagedContinuationTarget { 私有地址已隐藏 }")
    }
}

impl ManagedContinuationTarget {
    pub fn safe_evidence(&self) -> Value {
        json!({"module_mvid":self.module_mvid,"method_token":self.method_token,
            "il_offset":self.il_offset,"enc_version":self.enc_version,"map_count":self.map_count,
            "map_sha256":hex(&self.map_sha256),"code_sha256":hex(&self.code_sha256),
            "extent_length":self.extent_end-self.extent_start,"pre_sequence":self.pre_sequence})
    }
}

fn fixed_hex<const N: usize>(value: &Value) -> io::Result<[u8; N]> {
    let text = value
        .as_str()
        .ok_or_else(|| io::Error::other("CLR 私有摘要缺失"))?;
    require(
        text.len() == N * 2
            && text
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "CLR 私有摘要格式无效",
    )?;
    let mut bytes = [0; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte =
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).map_err(io::Error::other)?;
    }
    require(bytes != [0; N], "CLR 私有摘要不能为空")?;
    Ok(bytes)
}

fn managed_target(
    value: &Value,
    binding: &Value,
    spec: &ClrManagedMethodSpec,
) -> io::Result<ManagedContinuationTarget> {
    spec.validate()?;
    let number = |key: &str| {
        value[key]
            .as_u64()
            .ok_or_else(|| io::Error::other("CLR 私有票据整数缺失"))
    };
    let small = |key: &str| u32::try_from(number(key)?).map_err(io::Error::other);
    let target = ManagedContinuationTarget {
        process_id: small("process_id")?,
        thread_id: small("thread_id")?,
        process_birth: number("process_birth")?,
        thread_birth: number("thread_birth")?,
        pre_sequence: number("pre_sequence")?,
        pre_nonce: fixed_hex(&value["pre_nonce"])?,
        module_mvid: value["module_mvid"]
            .as_str()
            .ok_or_else(|| io::Error::other("CLR 私有模块缺失"))?
            .into(),
        method_token: small("method_token")?,
        il_offset: small("il_offset")?,
        enc_version: small("enc_version")?,
        frame_rsp: number("frame_rsp")?,
        extent_start: number("extent_start")?,
        extent_end: number("extent_end")?,
        address: number("address")?,
        map_count: small("map_count")?,
        map_sha256: fixed_hex(&value["map_sha256"])?,
        code_sha256: fixed_hex(&value["code_sha256"])?,
        spec: spec.clone(),
    };
    require(
        binding["operation"] == 3
            && binding["event_sequence"] == target.pre_sequence
            && binding["nonce"] == hex(&target.pre_nonce)
            && binding["pid"] == target.process_id
            && binding["tid"] == target.thread_id
            && binding["process_birth"] == target.process_birth
            && binding["thread_birth"] == target.thread_birth
            && target.pre_sequence != 0
            && target.process_id != 0
            && target.thread_id != 0
            && target.process_birth != 0
            && target.thread_birth != 0
            && target.module_mvid == spec.module_mvid
            && target.method_token == spec.method_token
            && spec.approved_il_offsets.contains(&target.il_offset)
            && (1..=256).contains(&target.map_count)
            && target.frame_rsp >= 0x10000
            && target.frame_rsp <= 0x0000_7fff_ffff_ffff
            && target.frame_rsp % 8 == 0
            && target.extent_start >= 0x10000
            && target.extent_end > target.extent_start
            && target.extent_end - target.extent_start <= 65536
            && target.extent_end <= 0x0000_8000_0000_0000
            && target.address >= target.extent_start
            && target.address < target.extent_end,
        "CLR 私有续点票据未绑定原停点或完整方法",
    )?;
    Ok(target)
}

fn managed_extension(
    spec: &ClrManagedMethodSpec,
    target: Option<&ManagedContinuationTarget>,
) -> io::Result<Vec<u8>> {
    spec.validate()?;
    let mut bytes = vec![0; MANAGED_EXTENSION_SIZE];
    bytes[..36].copy_from_slice(spec.module_mvid.as_bytes());
    put32(&mut bytes, 36, spec.method_token);
    put32(&mut bytes, 40, spec.approved_il_offsets.len() as u32);
    for (index, il) in spec.approved_il_offsets.iter().enumerate() {
        put32(&mut bytes, 44 + index * 4, *il);
    }
    put32(&mut bytes, 60, spec.bool_local_index);
    put32(
        &mut bytes,
        64,
        spec.start_info_local_index.unwrap_or(u32::MAX),
    );
    if let Some(target) = target {
        put64(&mut bytes, 68, target.pre_sequence);
        bytes[76..92].copy_from_slice(&target.pre_nonce);
        put32(&mut bytes, 92, target.il_offset);
        put32(&mut bytes, 96, target.enc_version);
        put64(&mut bytes, 100, target.frame_rsp);
        put64(&mut bytes, 108, target.extent_start);
        put64(&mut bytes, 116, target.extent_end);
        put64(&mut bytes, 124, target.address);
        put32(&mut bytes, 132, target.map_count);
        bytes[136..168].copy_from_slice(&target.map_sha256);
        bytes[168..200].copy_from_slice(&target.code_sha256);
    }
    Ok(bytes)
}

fn validate_managed_observation(
    value: &Value,
    target: &ManagedContinuationTarget,
) -> io::Result<()> {
    require(
        value["bound"].is_boolean()
            && value["api_hresult"]
                .as_u64()
                .is_some_and(|n| n <= u32::MAX as u64),
        "CLR 托管续点结果缺失",
    )?;
    if value["bound"] == false {
        return require(
            value["mapping"].is_null()
                && value["boolean_local"].is_null()
                && value["start_info_field"].is_null()
                && value["api_hresult"] != 0,
            "CLR 未绑定续点不能报告局部值",
        );
    }
    require(
        value["api_hresult"] == 0 && value["mapping"] == target.safe_evidence(),
        "CLR 托管续点与原映射不符",
    )?;
    let boolean = &value["boolean_local"];
    require(
        boolean["index"] == target.spec.bool_local_index,
        "CLR 布尔局部索引不符",
    )?;
    validate_local_value(boolean, "", 1, 1, boolean["value"].is_boolean())?;
    require(
        boolean["value"].is_boolean() || boolean["value"].is_null(),
        "CLR 布尔局部值格式无效",
    )?;
    if boolean["value"].is_boolean() {
        require(
            boolean["type_name"] == "System.Boolean",
            "CLR 局部不是真实 Boolean",
        )?;
    } else {
        require(boolean["api_hresult"] != 0, "CLR 布尔成功结果缺少真实值")?;
    }
    let field = &value["start_info_field"];
    let requested = target.spec.start_info_local_index.is_some();
    require(
        field["requested"] == requested
            && field["index"] == json!(target.spec.start_info_local_index)
            && (field["value"].is_boolean() || field["value"].is_null()),
        "CLR PSI 请求或值不符",
    )?;
    if !requested {
        return require(field["value"].is_null(), "CLR 未请求 PSI 不应报告字段值");
    }
    validate_local_value(field, "local_", 8, 0x10, field["value"].is_boolean())?;
    require(
        field["api_hresult"]
            .as_u64()
            .is_some_and(|n| n <= u32::MAX as u64),
        "CLR PSI 结果码缺失",
    )?;
    if field["value"].is_boolean() {
        let mvid = field["module_mvid"]
            .as_str()
            .and_then(|text| uuid::Uuid::parse_str(text).ok());
        require(
            field["api_hresult"] == 0
                && field["field_read_hresult"] == 0
                && field["field_name"] == "useShellExecute"
                && field["field_type"] == 2
                && field["field_sig_type"] == 2
                && mvid.is_some_and(|id| !id.is_nil())
                && field["type_token"].as_u64().is_some_and(|n| {
                    n & 0xff00_0000 == 0x0200_0000 && n & 0x00ff_ffff != 0 && n <= u32::MAX as u64
                })
                && field["field_token"].as_u64().is_some_and(|n| {
                    n & 0xff00_0000 == 0x0400_0000 && n & 0x00ff_ffff != 0 && n <= u32::MAX as u64
                }),
            "CLR PSI 字段未由实际元数据绑定",
        )?;
    } else {
        require(field["api_hresult"] != 0, "CLR PSI 成功结果缺少字段值")?;
    }
    Ok(())
}

fn validate_local_value(
    value: &Value,
    prefix: &str,
    bytes: u64,
    kind: u64,
    read: bool,
) -> io::Result<()> {
    for field in [
        "api_hresult",
        "get_local_hresult",
        "locations_hresult",
        "type_hresult",
        "type_name_hresult",
        "flags_hresult",
        "size_hresult",
        "bytes_hresult",
    ] {
        let key = format!("{prefix}{field}");
        require(
            value[&key].as_u64().is_some_and(|n| n <= u32::MAX as u64),
            "CLR 局部读取结果码缺失",
        )?;
        if read {
            // 引用 Value 没有声明类型实例；只读引用字节后由实际对象 MT 校验类型。
            let expected = if kind == 0x10 && matches!(field, "type_hresult" | "type_name_hresult")
            {
                0x8000_000a_u32
            } else {
                0
            };
            require(value[&key] == expected, "CLR 局部读取或未请求类型状态不符")?;
        }
    }
    if read {
        require(
            value[format!("{prefix}locations")]
                .as_u64()
                .is_some_and(|n| (1..=2).contains(&n))
                && value[format!("{prefix}flags")]
                    .as_u64()
                    .is_some_and(|n| n & 0x7f == kind)
                && value[format!("{prefix}size")] == bytes
                && value[format!("{prefix}bytes_read")] == bytes,
            "CLR 局部位置、类型或字节数不完整",
        )?;
    }
    Ok(())
}

pub(super) fn require(value: bool, message: &'static str) -> io::Result<()> {
    if value {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}
pub(super) fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(super) fn remaining(deadline: Instant) -> io::Result<u32> {
    let value = deadline.saturating_duration_since(Instant::now());
    if value.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "CLR reader 原期限耗尽",
        ));
    }
    Ok(value.as_millis().min(u32::MAX as u128).max(1) as u32)
}

pub(super) fn information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(io::Error::from)?;
    require(
        info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "夹具文件不能是重解析点",
    )?;
    Ok(info)
}

pub(super) fn file_identity(info: &BY_HANDLE_FILE_INFORMATION) -> (u32, u32, u32, u32, u32) {
    (
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
        info.nFileSizeHigh,
        info.nFileSizeLow,
    )
}

#[derive(Debug)]
pub(super) struct BoundFile {
    pub(super) file: File,
    pub(super) path: PathBuf,
    pub(super) identity: (u32, u32, u32, u32, u32),
    pub(super) sha: [u8; 32],
    pub(super) size: u64,
}

impl BoundFile {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        require(path.is_absolute(), "夹具文件必须为绝对路径")?;
        let path = path.canonicalize()?;
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&path)?;
        let identity = file_identity(&information(&file)?);
        let size = file.metadata()?.len();
        require(size > 0 && size <= MAX_FILE, "夹具文件大小超过固定边界")?;
        let sha = Self::digest(&mut file)?;
        Ok(Self {
            file,
            path,
            identity,
            sha,
            size,
        })
    }

    pub(super) fn digest(file: &mut File) -> io::Result<[u8; 32]> {
        file.seek(SeekFrom::Start(0))?;
        let mut hash = Sha256::new();
        let mut bytes = [0; 65536];
        loop {
            let count = file.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            hash.update(&bytes[..count]);
        }
        Ok(hash.finalize().into())
    }

    pub(super) fn verify(&mut self) -> io::Result<()> {
        require(
            file_identity(&information(&self.file)?) == self.identity,
            "夹具原文件身份改变",
        )?;
        require(
            Self::digest(&mut self.file)? == self.sha,
            "夹具原文件摘要改变",
        )?;
        let file = File::open(&self.path)?;
        require(
            file_identity(&information(&file)?) == self.identity,
            "夹具路径已指向另一文件",
        )
    }

    pub(super) fn receipt(&self) -> Value {
        json!({"sha256":hex(&self.sha),"bytes":self.size,"file_identity":self.identity})
    }

    pub(super) fn version(&mut self) -> io::Result<(u32, u32)> {
        self.verify()?;
        let path: Vec<u16> = self.path.as_os_str().encode_wide().chain(Some(0)).collect();
        let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None) };
        require(
            size > 0 && size <= 1024 * 1024,
            "系统映像版本资源超过固定边界",
        )?;
        let mut bytes = vec![0_u8; size as usize];
        unsafe {
            GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, bytes.as_mut_ptr().cast())
        }
        .map_err(io::Error::from)?;
        let mut pointer = std::ptr::null_mut();
        let mut length = 0;
        require(
            unsafe { VerQueryValueW(bytes.as_ptr().cast(), w!("\\"), &mut pointer, &mut length) }
                .as_bool()
                && !pointer.is_null()
                && length as usize >= size_of::<VS_FIXEDFILEINFO>(),
            "系统映像缺少固定版本资源",
        )?;
        let start = bytes.as_ptr() as usize;
        let address = pointer as usize;
        require(
            address >= start
                && address
                    .checked_add(size_of::<VS_FIXEDFILEINFO>())
                    .is_some_and(|end| end <= start + bytes.len()),
            "系统版本资源指针越界",
        )?;
        let info = unsafe { std::ptr::read_unaligned(pointer.cast::<VS_FIXEDFILEINFO>()) };
        require(info.dwSignature == 0xfeef04bd, "系统映像版本签名无效")?;
        self.verify()?;
        Ok((info.dwFileVersionMS, info.dwFileVersionLS))
    }
}

pub(super) fn image_path(file: &File) -> io::Result<PathBuf> {
    let mut buffer = [0_u16; 2048];
    let count = unsafe {
        GetFinalPathNameByHandleW(
            HANDLE(file.as_raw_handle()),
            &mut buffer,
            FILE_NAME_NORMALIZED,
        )
    };
    require(
        count > 0 && count < buffer.len() as u32,
        "原映像路径超出固定边界",
    )?;
    Ok(PathBuf::from(OsString::from_wide(
        &buffer[..count as usize],
    )))
}

pub(super) fn birth(handle: HANDLE, thread: bool) -> io::Result<u64> {
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

pub(super) fn duplicate(source: HANDLE, target: HANDLE, access: u32) -> io::Result<HANDLE> {
    let mut result = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            target,
            &mut result,
            access,
            false,
            DUPLICATE_HANDLE_OPTIONS(0),
        )
    }
    .map_err(io::Error::from)?;
    Ok(result)
}

#[derive(Debug)]
pub(super) struct Job(OwnedHandle);

impl Job {
    pub(super) fn new() -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::from)?;
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle.0) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                raw(&job.0),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(io::Error::from)?;
        Ok(job)
    }

    pub(super) fn assign(&self, process: HANDLE) -> io::Result<()> {
        unsafe { AssignProcessToJobObject(raw(&self.0), process) }.map_err(io::Error::from)?;
        let mut member = BOOL::default();
        unsafe { IsProcessInJob(process, Some(raw(&self.0)), &mut member) }
            .map_err(io::Error::from)?;
        require(member.as_bool(), "夹具进程没有进入精确 Job")
    }

    pub(super) fn terminate(&self) -> io::Result<()> {
        unsafe { TerminateJobObject(raw(&self.0), 1) }.map_err(io::Error::from)
    }

    pub(super) fn empty(&self) -> io::Result<bool> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(raw(&self.0)),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
        }
        .map_err(io::Error::from)?;
        Ok(info.ActiveProcesses == 0)
    }
}

pub(super) fn empty_environment(command: &mut Command, root: &Path) -> io::Result<()> {
    let system = std::env::var_os("SystemRoot").ok_or_else(|| io::Error::other("缺少系统根"))?;
    command
        .env_clear()
        .env("SystemRoot", &system)
        .env("WINDIR", system)
        .env("TEMP", root)
        .env("TMP", root)
        .current_dir(root);
    Ok(())
}

pub(super) fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
pub(super) fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn request(operation: u32, nonce: [u8; 16], deadline: Instant) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; REQUEST_SIZE];
    bytes[..8].copy_from_slice(b"G09CLR1\0");
    put32(&mut bytes, 8, 1);
    put32(&mut bytes, 12, operation);
    bytes[16..32].copy_from_slice(&nonce);
    put64(
        &mut bytes,
        96,
        unsafe { GetTickCount64() } + u64::from(remaining(deadline)?),
    );
    Ok(bytes)
}

/// 已由验收驱动固定摘要的独立读取器映像。
#[derive(Debug)]
pub struct ClrReaderImage(BoundFile);

impl ClrReaderImage {
    pub fn bind(path: &Path, expected_sha256: [u8; 32]) -> io::Result<Self> {
        let image = BoundFile::open(path)?;
        require(image.sha == expected_sha256, "CLR reader 映像摘要不匹配")?;
        Ok(Self(image))
    }

    pub fn verify(&mut self) -> io::Result<()> {
        self.0.verify()
    }

    pub fn receipt(&self) -> Value {
        self.0.receipt()
    }
}

/// 仅在调用方完成普通 LOAD 授权后，从该原文件句柄绑定固定 Framework CLR/DAC。
#[derive(Debug)]
pub struct ClrRuntimeBinding {
    pub(super) image: BoundFile,
    pub(super) dac: BoundFile,
    base: u64,
    size: u32,
    timestamp: u32,
    pub(super) version: (u32, u32),
}

impl ClrRuntimeBinding {
    pub fn from_load(original_clr_file: &File, base: u64) -> io::Result<Self> {
        let path = image_path(original_clr_file)?;
        let mut image = BoundFile::open(&path)?;
        require(
            image.identity == file_identity(&information(original_clr_file)?),
            "CLR 路径与原 LOAD 映像不同",
        )?;
        let mut system = [0_u16; 1024];
        let length = unsafe { GetWindowsDirectoryW(Some(&mut system)) } as usize;
        require(
            length > 0 && length < system.len(),
            "固定 Windows 目录不可用",
        )?;
        let expected = PathBuf::from(OsString::from_wide(&system[..length]))
            .join("Microsoft.NET/Framework64/v4.0.30319/clr.dll")
            .canonicalize()?;
        require(
            path.to_string_lossy()
                .eq_ignore_ascii_case(&expected.to_string_lossy()),
            "未加载固定 Framework64 CLR",
        )?;
        let mut header = [0_u8; 4096];
        image.file.seek(SeekFrom::Start(0))?;
        image.file.read_exact(&mut header)?;
        let pe = u32::from_le_bytes(header[60..64].try_into().unwrap()) as usize;
        require(&header[..2] == b"MZ" && pe <= 3900, "CLR PE 头无效")?;
        require(
            &header[pe..pe + 4] == b"PE\0\0"
                && u16::from_le_bytes(header[pe + 4..pe + 6].try_into().unwrap()) == 0x8664
                && u16::from_le_bytes(header[pe + 24..pe + 26].try_into().unwrap()) == 0x20b,
            "CLR 不是固定 x64 PE32+",
        )?;
        let timestamp = u32::from_le_bytes(header[pe + 8..pe + 12].try_into().unwrap());
        let size = u32::from_le_bytes(header[pe + 80..pe + 84].try_into().unwrap());
        require(
            base != 0
                && size > 0
                && size <= MAX_FILE as u32
                && base.checked_add(u64::from(size)).is_some(),
            "CLR 映像范围越界",
        )?;
        let mut dac = BoundFile::open(&path.with_file_name("mscordacwks.dll"))?;
        let version = image.version()?;
        require(version.0 == 0x00040008, "不是固定 Framework 4.8")?;
        require(dac.version()? == version, "CLR 与 DAC 的实际文件版本不匹配")?;
        Ok(Self {
            image,
            dac,
            base,
            size,
            timestamp,
            version,
        })
    }

    pub fn verify(&mut self) -> io::Result<()> {
        self.image.verify()?;
        self.dac.verify()
    }

    pub fn receipt(&self) -> Value {
        json!({"clr_image":self.image.receipt(),"dac_image":self.dac.receipt(),"file_version":self.version})
    }
}

/// 句柄必须来自仍未 Continue 的原调试事件；调用方保存 generation 和 pending 所有权。
#[derive(Debug)]
pub struct ClrExceptionStop<'a> {
    pub process: BorrowedHandle<'a>,
    pub thread: BorrowedHandle<'a>,
    pub process_id: u32,
    pub thread_id: u32,
    pub event_sequence: u64,
    pub exception_code: u32,
    pub first_chance: u32,
    pub hresult: u32,
    pub read_budget_bytes: u32,
}

/// 仅限调用方自有 SHGetFileInfoW 返回单步；必须仍持有原 root 的 pending 事件。
#[derive(Debug)]
pub struct ClrNativeReturnStop<'a> {
    pub process: BorrowedHandle<'a>,
    pub thread: BorrowedHandle<'a>,
    pub process_id: u32,
    pub thread_id: u32,
    pub event_sequence: u64,
    pub exception_code: u32,
    pub first_chance: u32,
    pub hresult: u32,
    pub read_budget_bytes: u32,
}

/// 仅限 core 由私有方法票据布置并确认的自有托管续点 DR。
#[derive(Debug)]
pub struct ClrManagedContinuationStop<'a> {
    pub process: BorrowedHandle<'a>,
    pub thread: BorrowedHandle<'a>,
    pub process_id: u32,
    pub thread_id: u32,
    pub event_sequence: u64,
    pub exception_code: u32,
    pub first_chance: u32,
    pub hresult: u32,
    pub read_budget_bytes: u32,
}

impl<'a> From<ClrManagedContinuationStop<'a>> for ClrStop<'a> {
    fn from(stop: ClrManagedContinuationStop<'a>) -> Self {
        Self {
            process: stop.process,
            thread: stop.thread,
            process_id: stop.process_id,
            thread_id: stop.thread_id,
            event_sequence: stop.event_sequence,
            exception_code: stop.exception_code,
            first_chance: stop.first_chance,
            hresult: stop.hresult,
            read_budget_bytes: stop.read_budget_bytes,
        }
    }
}

struct ClrStop<'a> {
    process: BorrowedHandle<'a>,
    thread: BorrowedHandle<'a>,
    process_id: u32,
    thread_id: u32,
    event_sequence: u64,
    exception_code: u32,
    first_chance: u32,
    hresult: u32,
    read_budget_bytes: u32,
}

impl<'a> From<ClrExceptionStop<'a>> for ClrStop<'a> {
    fn from(stop: ClrExceptionStop<'a>) -> Self {
        Self {
            process: stop.process,
            thread: stop.thread,
            process_id: stop.process_id,
            thread_id: stop.thread_id,
            event_sequence: stop.event_sequence,
            exception_code: stop.exception_code,
            first_chance: stop.first_chance,
            hresult: stop.hresult,
            read_budget_bytes: stop.read_budget_bytes,
        }
    }
}

impl<'a> From<ClrNativeReturnStop<'a>> for ClrStop<'a> {
    fn from(stop: ClrNativeReturnStop<'a>) -> Self {
        Self {
            process: stop.process,
            thread: stop.thread,
            process_id: stop.process_id,
            thread_id: stop.thread_id,
            event_sequence: stop.event_sequence,
            exception_code: stop.exception_code,
            first_chance: stop.first_chance,
            hresult: stop.hresult,
            read_budget_bytes: stop.read_budget_bytes,
        }
    }
}

fn poll_wait(left: Duration) -> Option<u32> {
    (!left.is_zero()).then(|| left.min(MAX_POLL).as_millis().max(1) as u32)
}

fn valid_read_budget(bytes: u32) -> bool {
    matches!(bytes, FIXTURE_READ_BYTES | POWERSHELL_READ_BYTES)
}

fn valid_stop_kind(operation: u32, code: u32, first_chance: u32, hresult: u32) -> bool {
    first_chance == 1
        && ((operation == 1 && code == CLR_EXCEPTION)
            || (matches!(operation, 3 | 4) && code == NATIVE_RETURN_SINGLE_STEP && hresult == 0))
}

fn validate_stop(stop: &ClrStop<'_>, operation: u32, sequence: u64) -> io::Result<()> {
    require(
        stop.event_sequence == sequence
            && sequence != 0
            && stop.process_id != 0
            && stop.thread_id != 0
            && valid_stop_kind(
                operation,
                stop.exception_code,
                stop.first_chance,
                stop.hresult,
            )
            && valid_read_budget(stop.read_budget_bytes),
        "CLR reader 停点或固定额度不符",
    )?;
    require(
        unsafe { GetProcessId(HANDLE(stop.process.as_raw_handle())) } == stop.process_id
            && unsafe { GetThreadId(HANDLE(stop.thread.as_raw_handle())) } == stop.thread_id
            && unsafe { GetProcessIdOfThread(HANDLE(stop.thread.as_raw_handle())) }
                == stop.process_id,
        "CLR reader 原进程或线程身份不符",
    )
}

fn validate_reply(value: &Value, binding: &Value) -> io::Result<()> {
    require(
        value["schema"] == 1
            && matches!(binding["operation"].as_u64(), Some(1 | 3 | 4))
            && value["operation"] == binding["operation"],
        "CLR reader 回复协议不符",
    )?;
    require(
        matches!(
            value["status"].as_str(),
            Some("observed" | "partial" | "unavailable")
        ),
        "CLR reader 回复状态无效",
    )?;
    if binding["operation"] == 3 || binding["operation"] == 4 {
        require(
            value["event_hresult"] == 0
                && value["exception_source"] == "none"
                && value["exception_api_hresult"] == 0x8000000a_u32
                && value["exception_state_flags"] == 0
                && value["tracker_complete"] == false
                && value["object_chain_complete"] == false
                && value["chain"].as_array().is_some_and(Vec::is_empty)
                && value["frames"]
                    .as_array()
                    .is_some_and(|frames| frames.len() <= 32),
            "CLR 原生返回停点回复混入异常链",
        )?;
        require(
            value["status"] != "observed"
                || (value["stack_api_hresult"] == 0
                    && value["budget_exhausted"] == false
                    && value["frames"]
                        .as_array()
                        .is_some_and(|frames| !frames.is_empty())),
            "CLR 原生返回停点未完整读取栈",
        )?;
    }
    let budget = binding["read_budget_bytes"]
        .as_u64()
        .filter(|bytes| {
            *bytes == u64::from(FIXTURE_READ_BYTES) || *bytes == u64::from(POWERSHELL_READ_BYTES)
        })
        .ok_or_else(|| io::Error::other("CLR reader 原请求额度缺失"))?;
    require(
        value["read_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes <= budget)
            && value["read_calls"]
                .as_u64()
                .is_some_and(|calls| calls <= 8192),
        "CLR reader 回复读取额度无效",
    )?;
    for field in [
        "nonce",
        "event_sequence",
        "pid",
        "tid",
        "process_birth",
        "thread_birth",
        "event_hresult",
    ] {
        require(
            !binding[field].is_null() && value[field] == binding[field],
            "CLR reader 回复原停点不符",
        )?;
    }
    require(
        value["target_identity_verified"] == true
            && value["dac_sha256_verified"] == true
            && value["dac_loaded"] == true,
        "CLR reader 回复没有完成身份绑定",
    )
}

/// 单次 reader。任何初始化或轮询错误都保留本对象，调用方须先终止目标，再显式回收。
#[derive(Debug)]
pub struct ClrReader {
    child: Child,
    job: Job,
    output: File,
    image_guard: File,
    image_sha: [u8; 32],
    operation: u32,
    sequence: u64,
    deadline: Instant,
    binding: Option<Value>,
    initialization_attempted: bool,
    request_sent: bool,
    exit_status: Option<ExitStatus>,
    reaped: bool,
    delivered: bool,
    managed_spec: Option<ClrManagedMethodSpec>,
    managed_request_target: Option<ManagedContinuationTarget>,
    managed_response_target: Option<ManagedContinuationTarget>,
    managed_target_taken: bool,
}

impl ClrReader {
    pub fn start(
        image: &mut ClrReaderImage,
        private_root: &Path,
        sequence: u64,
        deadline: Instant,
    ) -> io::Result<Self> {
        require(sequence != 0, "CLR reader 事件序号无效")?;
        Self::start_named(
            image,
            private_root,
            &format!("reader-{sequence}"),
            1,
            sequence,
            deadline,
        )
    }

    pub fn start_native_return(
        image: &mut ClrReaderImage,
        private_root: &Path,
        sequence: u64,
        deadline: Instant,
    ) -> io::Result<Self> {
        require(sequence != 0, "CLR reader 事件序号无效")?;
        Self::start_named(
            image,
            private_root,
            &format!("native-return-reader-{sequence}"),
            3,
            sequence,
            deadline,
        )
    }

    pub fn start_managed_continuation(
        image: &mut ClrReaderImage,
        private_root: &Path,
        sequence: u64,
        deadline: Instant,
    ) -> io::Result<Self> {
        require(sequence != 0, "CLR 托管续点序号无效")?;
        Self::start_named(
            image,
            private_root,
            &format!("managed-continuation-reader-{sequence}"),
            4,
            sequence,
            deadline,
        )
    }

    fn start_named(
        image: &mut ClrReaderImage,
        root: &Path,
        name: &str,
        operation: u32,
        sequence: u64,
        deadline: Instant,
    ) -> io::Result<Self> {
        remaining(deadline)?;
        image.verify()?;
        let output = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(root.join(format!("{name}.stdout.json")))?;
        let stderr = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(format!("{name}.stderr")))?;
        let image_guard = image.0.file.try_clone()?;
        let mut command = Command::new_with_managed_process_group(&image.0.path);
        empty_environment(&mut command, root)?;
        command
            .stdin(Stdio::piped())
            .stdout(output.try_clone()?)
            .stderr(stderr);
        let job = Job::new()?;
        remaining(deadline)?;
        let child = command.spawn()?;
        // spawn 后不能再有可能返回 Err 的步骤；原子保存所有权后由 initialize 分配 Job。
        Ok(Self {
            child,
            job,
            output,
            image_guard,
            image_sha: image.0.sha,
            operation,
            sequence,
            deadline,
            binding: None,
            initialization_attempted: false,
            request_sent: false,
            exit_status: None,
            reaped: false,
            delivered: false,
            managed_spec: None,
            managed_request_target: None,
            managed_response_target: None,
            managed_target_taken: false,
        })
    }

    fn initialize(&mut self, image: &mut ClrReaderImage) -> io::Result<()> {
        require(!self.initialization_attempted, "CLR reader 初始化不能重放")?;
        self.initialization_attempted = true;
        remaining(self.deadline)?;
        self.job.assign(HANDLE(self.child.as_raw_handle()))?;
        require(
            image.0.sha == self.image_sha
                && file_identity(&information(&self.image_guard)?) == image.0.identity,
            "CLR reader 初始化映像变化",
        )?;
        image.verify()
    }

    pub fn bind_and_send(
        &mut self,
        image: &mut ClrReaderImage,
        runtime: &mut ClrRuntimeBinding,
        stop: ClrExceptionStop<'_>,
    ) -> io::Result<()> {
        require(self.operation == 1, "CLR reader 不是异常读取请求")?;
        self.bind_stop_and_send(image, runtime, stop.into())
    }

    pub fn bind_native_return_and_send(
        &mut self,
        image: &mut ClrReaderImage,
        runtime: &mut ClrRuntimeBinding,
        stop: ClrNativeReturnStop<'_>,
    ) -> io::Result<()> {
        require(self.operation == 3, "CLR reader 不是原生返回读取请求")?;
        self.bind_stop_and_send(image, runtime, stop.into())
    }

    pub fn bind_native_return_with_managed_spec_and_send(
        &mut self,
        image: &mut ClrReaderImage,
        runtime: &mut ClrRuntimeBinding,
        stop: ClrNativeReturnStop<'_>,
        spec: &ClrManagedMethodSpec,
    ) -> io::Result<()> {
        require(
            self.operation == 3 && self.managed_spec.is_none() && !self.initialization_attempted,
            "CLR 托管规格请求不能重放",
        )?;
        spec.validate()?;
        self.managed_spec = Some(spec.clone());
        self.bind_stop_and_send(image, runtime, stop.into())
    }

    pub fn bind_managed_continuation_and_send(
        &mut self,
        image: &mut ClrReaderImage,
        runtime: &mut ClrRuntimeBinding,
        stop: ClrManagedContinuationStop<'_>,
        target: &ManagedContinuationTarget,
    ) -> io::Result<()> {
        require(
            self.operation == 4
                && self.managed_spec.is_none()
                && !self.initialization_attempted
                && target.pre_sequence < stop.event_sequence
                && target.process_id == stop.process_id
                && target.thread_id == stop.thread_id
                && birth(HANDLE(stop.process.as_raw_handle()), false)? == target.process_birth
                && birth(HANDLE(stop.thread.as_raw_handle()), true)? == target.thread_birth,
            "CLR 托管续点没有原票据或原身份变化",
        )?;
        target.spec.validate()?;
        self.managed_spec = Some(target.spec.clone());
        self.managed_request_target = Some(target.clone());
        self.bind_stop_and_send(image, runtime, stop.into())
    }

    fn bind_stop_and_send(
        &mut self,
        image: &mut ClrReaderImage,
        runtime: &mut ClrRuntimeBinding,
        stop: ClrStop<'_>,
    ) -> io::Result<()> {
        require(
            self.binding.is_none() && !self.request_sent,
            "CLR reader 请求不能重放",
        )?;
        self.initialize(image)?;
        validate_stop(&stop, self.operation, self.sequence)?;
        runtime.verify()?;
        let process = HANDLE(stop.process.as_raw_handle());
        let thread = HANDLE(stop.thread.as_raw_handle());
        let process_birth = birth(process, false)?;
        let thread_birth = birth(thread, true)?;
        let nonce = *uuid::Uuid::new_v4().as_bytes();
        self.binding = Some(
            json!({"operation":self.operation,"event_sequence":stop.event_sequence,"pid":stop.process_id,"tid":stop.thread_id,
            "process_birth":process_birth,"thread_birth":thread_birth,"event_hresult":stop.hresult,"nonce":hex(&nonce),
            "read_budget_bytes":stop.read_budget_bytes}),
        );
        // 部分复制失败时远端句柄仍归本 reader；调用方保留本对象直至精确回收。
        let process = duplicate(
            process,
            HANDLE(self.child.as_raw_handle()),
            (PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE).0,
        )?;
        let thread = duplicate(
            thread,
            HANDLE(self.child.as_raw_handle()),
            (THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION).0,
        )?;
        let mut bytes = request(self.operation, nonce, self.deadline)?;
        put64(&mut bytes, 32, stop.event_sequence);
        put32(&mut bytes, 40, stop.process_id);
        put32(&mut bytes, 44, stop.thread_id);
        put64(&mut bytes, 48, process_birth);
        put64(&mut bytes, 56, thread_birth);
        put64(&mut bytes, 64, process.0 as u64);
        put64(&mut bytes, 72, thread.0 as u64);
        put64(&mut bytes, 80, runtime.base);
        put32(&mut bytes, 88, runtime.size);
        put32(&mut bytes, 92, runtime.timestamp);
        put32(&mut bytes, 104, stop.read_budget_bytes);
        put32(&mut bytes, 108, 8192);
        put64(&mut bytes, 112, runtime.dac.size);
        bytes[120..152].copy_from_slice(&runtime.dac.sha);
        let path: Vec<u16> = runtime.dac.path.as_os_str().encode_wide().collect();
        require(
            !path.is_empty() && !path.contains(&0) && path.len() <= 1024,
            "DAC 路径超过固定边界",
        )?;
        put32(&mut bytes, 152, path.len() as u32);
        put32(&mut bytes, 156, stop.hresult);
        put32(&mut bytes, 160, stop.first_chance);
        put32(&mut bytes, 164, stop.exception_code);
        if let Some(spec) = self.managed_spec.as_ref() {
            put32(&mut bytes, 8, 2);
            bytes.extend(managed_extension(
                spec,
                self.managed_request_target.as_ref(),
            )?);
        }
        bytes.extend(path.iter().flat_map(|unit| unit.to_le_bytes()));
        self.send(&bytes)
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        remaining(self.deadline)?;
        require(
            !self.request_sent && bytes.len() < 4096,
            "CLR reader 请求重放或超过固定边界",
        )?;
        // 固定请求小于空匿名管道的缓冲；任何部分发送后也不再重放。
        self.request_sent = true;
        let mut stdin = self
            .child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("CLR reader 请求已关闭"))?;
        stdin.write_all(bytes)
    }

    fn poll_exit(
        &mut self,
        deadline: Instant,
        mut cancelled: impl FnMut() -> bool,
    ) -> io::Result<Option<ExitStatus>> {
        let poll_deadline = deadline.min(Instant::now() + MAX_POLL);
        loop {
            if cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "CLR reader 原控制已取消",
                ));
            }
            remaining(deadline)?;
            if self.reaped {
                return Ok(self.exit_status);
            }
            if self.exit_status.is_none() {
                let status = unsafe {
                    WaitForSingleObject(
                        HANDLE(self.child.as_raw_handle()),
                        match poll_wait(poll_deadline.saturating_duration_since(Instant::now())) {
                            Some(wait_ms) => wait_ms,
                            None => return Ok(None),
                        },
                    )
                };
                if status == WAIT_TIMEOUT {
                    return Ok(None);
                }
                if status != WAIT_OBJECT_0 {
                    return Err(io::Error::last_os_error());
                }
                self.exit_status = Some(self.child.wait()?);
            }
            if cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "CLR reader 原控制已取消",
                ));
            }
            remaining(deadline)?;
            if self.job.empty()? {
                self.reaped = true;
                return Ok(self.exit_status);
            }
            let left = poll_deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            thread::sleep(left.min(Duration::from_millis(10)));
        }
    }

    fn read_output(&mut self) -> io::Result<Value> {
        require(self.reaped, "CLR reader 未回收，不能读取完成结果")?;
        require(
            self.output.metadata()?.len() <= MAX_OUTPUT,
            "CLR reader 输出超过固定边界",
        )?;
        self.output.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        (&mut self.output)
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)?;
        require(
            bytes.len() as u64 <= MAX_OUTPUT,
            "CLR reader 输出读取超过固定边界",
        )?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }

    pub fn poll(&mut self, cancelled: impl FnMut() -> bool) -> io::Result<Option<Value>> {
        require(
            self.request_sent && self.binding.is_some() && !self.delivered,
            "CLR reader 尚未发送或回复已交付",
        )?;
        let Some(exit) = self.poll_exit(self.deadline, cancelled)? else {
            return Ok(None);
        };
        let mut value = self.read_output()?;
        require(exit.success(), "CLR reader 原生读取失败，原件已保留")?;
        validate_reply(&value, self.binding.as_ref().expect("已核对原请求绑定"))?;
        let private = value
            .as_object_mut()
            .ok_or_else(|| io::Error::other("CLR 回复不是对象"))?
            .remove("private_target");
        if let Some(spec) = self.managed_spec.as_ref() {
            if self.operation == 3 {
                let mapping = &value["managed_mapping"];
                require(
                    matches!(mapping["status"].as_str(), Some("bound" | "unknown")),
                    "CLR 方法映射状态缺失",
                )?;
                if mapping["status"] == "bound" {
                    require(
                        mapping["api_hresult"] == 0
                            && mapping["matched_frames"] == 1
                            && value["budget_exhausted"] == false,
                        "CLR 方法映射未完整绑定",
                    )?;
                    let target = managed_target(
                        private
                            .as_ref()
                            .ok_or_else(|| io::Error::other("CLR 私有续点缺失"))?,
                        self.binding.as_ref().expect("已核对原请求绑定"),
                        spec,
                    )?;
                    require(
                        mapping["mapping"] == target.safe_evidence(),
                        "CLR 公开映射与私有票据不一致",
                    )?;
                    self.managed_response_target = Some(target);
                } else {
                    require(private.is_none(), "CLR 未知映射不能携带地址")?;
                }
            } else {
                require(private.is_none(), "CLR 续点回复不能发布新地址")?;
                let target = self
                    .managed_request_target
                    .as_ref()
                    .ok_or_else(|| io::Error::other("CLR 原续点票据缺失"))?;
                validate_managed_observation(&value["managed_continuation"], target)?;
            }
        } else {
            require(
                private.is_none()
                    && value.get("managed_mapping").is_none()
                    && value.get("managed_continuation").is_none(),
                "CLR 旧协议回复混入托管票据",
            )?;
        }
        self.delivered = true;
        Ok(Some(value))
    }

    pub fn take_managed_continuation_target(
        &mut self,
    ) -> io::Result<Option<ManagedContinuationTarget>> {
        require(
            self.operation == 3
                && self.managed_spec.is_some()
                && self.reaped
                && self.delivered
                && !self.managed_target_taken,
            "CLR 私有续点尚未回收交付或已取走",
        )?;
        self.managed_target_taken = true;
        Ok(self.managed_response_target.take())
    }

    pub fn abort_and_reap(&mut self, cleanup_deadline: Instant) -> io::Result<()> {
        if self.reaped {
            return Ok(());
        }
        drop(self.child.stdin.take());
        // 先尝试精确 Job，再覆盖尚未入 Job 的原 child；错误不能充当退出证明。
        let _ = self.job.terminate();
        let _ = self.child.kill();
        while self.poll_exit(cleanup_deadline, || false)?.is_none() {}
        Ok(())
    }

    pub fn is_reaped(&self) -> bool {
        self.reaped
    }
    pub fn binding(&self) -> Option<&Value> {
        self.binding.as_ref()
    }
}

#[cfg(test)]
impl ClrReader {
    pub(super) fn start_fixture(
        image: &mut ClrReaderImage,
        root: &Path,
        name: &str,
        deadline: Instant,
    ) -> io::Result<Self> {
        Self::start_named(image, root, name, 2, 0, deadline)
    }

    pub(super) fn send_fixture(
        &mut self,
        image: &mut ClrReaderImage,
        bytes: &[u8],
    ) -> io::Result<()> {
        self.initialize(image)?;
        self.send(bytes)
    }

    pub(super) fn wait_fixture(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.poll_exit(self.deadline, || false)? {
                return Ok(status);
            }
        }
    }

    pub(super) fn output_fixture(&mut self) -> io::Result<Value> {
        self.read_output()
    }

    pub(super) fn job_empty_fixture(&self) -> io::Result<bool> {
        self.job.empty()
    }

    pub(super) fn remains_running_fixture(&self) -> bool {
        (unsafe { WaitForSingleObject(HANDLE(self.child.as_raw_handle()), 500) }) == WAIT_TIMEOUT
    }
}

impl Drop for ClrReader {
    fn drop(&mut self) {
        if !self.reaped {
            // Drop 不等同回收成功，也不另开等待期限；调用方必须显式保存失败状态。
            drop(self.child.stdin.take());
            let _ = self.job.terminate();
            let _ = self.child.kill();
        }
    }
}

#[cfg(test)]
#[path = "windows_clr_reader_tests.rs"]
mod tests;
