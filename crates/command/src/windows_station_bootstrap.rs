//! 两段新登录会话引导；只有第二段拥有新站，调用方的窗口站与登录身份不变。

use std::ffi::{OsStr, OsString, c_void};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{
    AsHandle as _, AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle,
};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use windows::Win32::Foundation::{
    DUPLICATE_HANDLE_OPTIONS, DuplicateHandle, ERROR_FILE_NOT_FOUND, ERROR_NO_DATA,
    ERROR_NO_MORE_FILES, ERROR_NO_TOKEN, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, FILETIME,
    GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, LUID, LocalFree, MAX_PATH, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows::Win32::Security::Authentication::Identity::{
    LsaFreeReturnBuffer, LsaGetLogonSessionData,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_ELEVATION,
    TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER,
    TokenElevation, TokenIntegrityLevel, TokenSessionId, TokenStatistics, TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    GetFullPathNameW, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, READ_CONTROL, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeClientSessionId,
    GetNamedPipeServerProcessId, GetNamedPipeServerSessionId, PIPE_NOWAIT, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, SetNamedPipeHandleState,
};
use windows::Win32::System::StationsAndDesktops::{
    CloseWindowStation, GetProcessWindowStation, GetUserObjectInformationW, OpenWindowStationW,
    UOI_NAME,
};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    CreateProcessWithLogonW, GetCurrentProcess, GetCurrentThread, GetExitCodeProcess, GetProcessId,
    GetProcessTimes, LOGON_NETCREDENTIALS_ONLY, OpenProcess, OpenProcessToken, OpenThreadToken,
    PROCESS_CREATE_PROCESS, PROCESS_DUP_HANDLE, PROCESS_INFORMATION, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW,
    ResumeThread, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};
use windows::core::{BOOL, HRESULT, PCWSTR, PWSTR};

use super::appcontainer::desktop::NewLogonDesktop;
use crate::managed::ProbeJobOperation;

#[path = "windows_station_debugger.rs"]
mod debugger;
#[path = "windows_station_device_map.rs"]
mod device_map;
pub(super) use debugger::Spawn as StationSpawn;
pub use debugger::StationDebugger;

#[cfg(feature = "native-probe-witness")]
use windows::Win32::Foundation::DUPLICATE_SAME_ACCESS;
#[cfg(feature = "native-probe-witness")]
use windows::Win32::System::StationsAndDesktops::{CloseDesktop, HDESK};

#[cfg(feature = "native-probe-witness")]
struct WitnessDesktop(Option<usize>);

#[cfg(feature = "native-probe-witness")]
impl WitnessDesktop {
    fn close(&mut self) -> io::Result<()> {
        if let Some(value) = self.0 {
            unsafe { CloseDesktop(HDESK(value as *mut c_void)) }.map_err(io::Error::other)?;
            self.0 = None;
        }
        Ok(())
    }
}

#[cfg(feature = "native-probe-witness")]
impl Drop for WitnessDesktop {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(feature = "native-probe-witness")]
fn witness_disabled(value: &bool) -> bool {
    !value
}

const MODE: &str = "--infinishell-station-bootstrap";
const START_TIMEOUT: Duration = Duration::from_secs(15);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
// 上限只限制辅助进程的生存期；调用方仍保留原任务期限和取消机制。
const LIFETIME: Duration = Duration::from_secs(600);

fn invalid(stage: &'static str) -> io::Error {
    io::Error::other(stage)
}
fn require(value: bool, stage: &'static str) -> io::Result<()> {
    if value { Ok(()) } else { Err(invalid(stage)) }
}
fn raw(value: &OwnedHandle) -> HANDLE {
    HANDLE(value.as_raw_handle())
}
fn owned(value: HANDLE) -> OwnedHandle {
    unsafe { OwnedHandle::from_raw_handle(value.0) }
}
fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut result: Vec<_> = value.encode_wide().collect();
    require(!result.contains(&0), "窗口站引导参数含 NUL")?;
    result.push(0);
    Ok(result)
}
fn sid_text(sid: PSID) -> io::Result<String> {
    let mut value = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut value) }.map_err(io::Error::other)?;
    let result = unsafe { value.to_string() }.map_err(io::Error::other);
    unsafe { LocalFree(Some(HLOCAL(value.0.cast()))) };
    result
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    pid: u32,
    created: u64,
    auth_low: u32,
    auth_high: i32,
    session: u32,
    user: String,
    integrity: String,
    elevated: u32,
    token_type: i32,
}
impl Snapshot {
    fn capture(process: HANDLE) -> io::Result<Self> {
        let pid = unsafe { GetProcessId(process) };
        require(pid != 0, "窗口站引导 PID 无效")?;
        let (mut created, mut exit, mut kernel, mut user_time) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        unsafe {
            GetProcessTimes(
                process,
                &mut created,
                &mut exit,
                &mut kernel,
                &mut user_time,
            )
        }
        .map_err(io::Error::other)?;
        let mut token = HANDLE::default();
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(io::Error::other)?;
        let token = owned(token);
        let stats: TOKEN_STATISTICS = token_field(raw(&token), TokenStatistics)?;
        let elevation: TOKEN_ELEVATION = token_field(raw(&token), TokenElevation)?;
        let mut buffer = [0usize; 512];
        token_buffer(raw(&token), TokenUser, &mut buffer)?;
        let user = sid_text(unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid })?;
        token_buffer(raw(&token), TokenIntegrityLevel, &mut buffer)?;
        let integrity =
            sid_text(unsafe { (*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid })?;
        Ok(Self {
            pid,
            created: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
            auth_low: stats.AuthenticationId.LowPart,
            auth_high: stats.AuthenticationId.HighPart,
            session: token_field(raw(&token), TokenSessionId)?,
            user,
            integrity,
            elevated: elevation.TokenIsElevated,
            token_type: stats.TokenType.0,
        })
    }
    fn same_local(&self, other: &Self) -> bool {
        self.user == other.user
            && self.integrity == other.integrity
            && self.elevated == other.elevated
            && self.token_type == other.token_type
            && self.session == other.session
    }
    fn same_logon(&self, other: &Self) -> bool {
        (self.auth_low, self.auth_high) == (other.auth_low, other.auth_high)
    }
    fn same_package_logon(&self, other: &Self) -> bool {
        self.same_logon(other) && self.user == other.user && self.session == other.session
    }
    fn station_name(&self) -> String {
        format!("Service-0x{:x}-{:x}$", self.auth_high as u32, self.auth_low)
    }
}
fn token_field<T: Default>(token: HANDLE, field: TOKEN_INFORMATION_CLASS) -> io::Result<T> {
    let mut value = T::default();
    let mut used = 0;
    unsafe {
        GetTokenInformation(
            token,
            field,
            Some((&mut value as *mut T).cast()),
            size_of::<T>() as u32,
            &mut used,
        )
    }
    .map_err(io::Error::other)?;
    require(
        used as usize == size_of::<T>(),
        "窗口站 token 字段长度不匹配",
    )?;
    Ok(value)
}
fn token_buffer(
    token: HANDLE,
    field: TOKEN_INFORMATION_CLASS,
    storage: &mut [usize],
) -> io::Result<()> {
    let mut used = 0;
    unsafe {
        GetTokenInformation(
            token,
            field,
            Some(storage.as_mut_ptr().cast()),
            size_of_val(storage) as u32,
            &mut used,
        )
    }
    .map_err(io::Error::other)?;
    require(
        used as usize <= size_of_val(storage) && used as usize >= size_of::<TOKEN_USER>(),
        "窗口站 token 缓冲长度不匹配",
    )
}
fn no_impersonation() -> io::Result<()> {
    let mut token = HANDLE::default();
    match unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) } {
        Ok(()) => {
            drop(owned(token));
            Err(invalid("窗口站引导拒绝线程模拟身份"))
        }
        Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_TOKEN.0) => Ok(()),
        Err(error) => Err(io::Error::other(error)),
    }
}
fn process_image(process: HANDLE) -> io::Result<PathBuf> {
    let mut storage = vec![0u16; 32768];
    let mut used = storage.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(storage.as_mut_ptr()),
            &mut used,
        )
    }
    .map_err(io::Error::other)?;
    require(
        used > 0 && (used as usize) < storage.len(),
        "窗口站映像路径长度无效",
    )?;
    PathBuf::from(OsString::from_wide(&storage[..used as usize])).canonicalize()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    volume: u32,
    high: u32,
    low: u32,
}
fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let mut value = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut value) }
        .map_err(io::Error::other)?;
    require(
        value.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "窗口站引导拒绝重解析对象",
    )?;
    Ok(FileIdentity {
        volume: value.dwVolumeSerialNumber,
        high: value.nFileIndexHigh,
        low: value.nFileIndexLow,
    })
}
fn open_path_locked(path: &Path) -> io::Result<File> {
    let directory = fs::symlink_metadata(path)?.is_dir();
    let share = if directory {
        FILE_SHARE_READ | FILE_SHARE_WRITE
    } else {
        FILE_SHARE_READ
    };
    let file = OpenOptions::new()
        .read(true)
        .share_mode(share.0)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
        .open(path)?;
    require(
        file.metadata()?.is_dir() == directory,
        "窗口站引导路径类型变化",
    )?;
    Ok(file)
}
pub(super) struct PathLease {
    entries: Vec<(PathBuf, File, FileIdentity)>,
}
impl PathLease {
    fn capture(path: &Path) -> io::Result<Self> {
        require(path.is_absolute(), "窗口站引导需要绝对路径")?;
        // 在规范化前锁住调用方给出的每一级，不能先跟随链接再检查目标。
        let mut original = Vec::new();
        for ancestor in path.ancestors() {
            let file = open_path_locked(ancestor)?;
            file_identity(&file)?;
            original.push(file);
        }
        let canonical = path.canonicalize()?;
        require(
            canonical.to_string_lossy().starts_with(r"\\?\")
                && !canonical.to_string_lossy().starts_with(r"\\?\UNC\"),
            "窗口站引导只接受本地卷",
        )?;
        let mut entries = Vec::new();
        for path in canonical.ancestors() {
            let file = open_path_locked(path)?;
            let identity = file_identity(&file)?;
            entries.push((path.to_owned(), file, identity));
        }
        Ok(Self { entries })
    }
    fn path(&self) -> &Path {
        &self.entries[0].0
    }

    fn application_path(&self) -> io::Result<Vec<u16>> {
        self.verify()?;
        let original = wide(self.path().as_os_str())?;
        // 对齐 Rust 1.92 的 to_user_path：仅在 Win32 解析不改变短路径时去掉前缀。
        // 请求、argv0 和原映像租约仍使用 canonical 路径，不改变被审核的文件身份。
        if original.len() > MAX_PATH as usize
            || !original.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16])
            || original.get(5) != Some(&(b':' as u16))
            || original.get(6) != Some(&(b'\\' as u16))
        {
            return Ok(original);
        }
        let candidate = &original[4..];
        let mut normalized = [0; MAX_PATH as usize];
        let count =
            unsafe { GetFullPathNameW(PCWSTR(candidate.as_ptr()), Some(&mut normalized), None) }
                as usize;
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        if count >= normalized.len() || normalized[..count] != candidate[..candidate.len() - 1] {
            return Ok(original);
        }
        // 直接按这串 Win32 名称打开，避免标准库再次添加 verbatim 前缀后才比较身份。
        let handle = unsafe {
            CreateFileW(
                PCWSTR(normalized.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .map_err(io::Error::other)?;
        let file = unsafe { File::from_raw_handle(handle.0) };
        require(
            file_identity(&file)? == self.entries[0].2,
            "窗口站引导执行路径身份变化",
        )?;
        Ok(normalized[..=count].to_vec())
    }

    pub(super) fn verify(&self) -> io::Result<()> {
        for (path, file, identity) in &self.entries {
            require(
                file_identity(file)? == *identity
                    && file_identity(&open_path_locked(path)?)? == *identity,
                "窗口站引导路径身份变化",
            )?;
        }
        Ok(())
    }
}

/// 调用方必须给出同构建部署的独立引导程序及其受绑定摘要，不能退回 PATH。
pub struct StationBootstrapImage {
    lease: PathLease,
    size: u64,
    sha256: String,
}
impl StationBootstrapImage {
    pub fn capture(path: &Path, expected_size: u64, sha256: &str) -> io::Result<Self> {
        require(
            sha256.len() == 64
                && sha256
                    .bytes()
                    .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value)),
            "窗口站引导摘要无效",
        )?;
        let lease = PathLease::capture(path)?;
        let file = &lease.entries[0].1;
        let metadata = file.metadata()?;
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
            .map_err(io::Error::other)?;
        require(
            metadata.is_file()
                && information.nNumberOfLinks == 1
                && metadata.len() == expected_size
                && expected_size > 0
                && expected_size <= 64 * 1024 * 1024,
            "窗口站引导映像不是受绑定的有界文件",
        )?;
        let mut digest = Sha256::new();
        std::io::copy(&mut file.take(64 * 1024 * 1024 + 1), &mut digest)?;
        require(
            format!("{:x}", digest.finalize()) == sha256,
            "窗口站引导映像摘要不匹配",
        )?;
        lease.verify()?;
        Ok(Self {
            lease,
            size: expected_size,
            sha256: sha256.to_owned(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    nonce: String,
    profile: String,
    container_sid: String,
    caller: Snapshot,
    caller_station: String,
    image: PathBuf,
    image_size: u64,
    image_sha256: String,
    device_target: device_map::Target,
    #[cfg(feature = "native-probe-witness")]
    #[serde(default, skip_serializing_if = "witness_disabled")]
    witness_desktop: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Binding {
    nonce: String,
    first: Snapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Second {
    nonce: String,
    process: Snapshot,
    source_handle: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Setup {
    request: Request,
    binding: Binding,
    second: Option<Second>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Closed {
    nonce: String,
    process: Snapshot,
    desktop_verified_and_closed: bool,
    device_map: device_map::Binding,
    device_map_verified_and_removed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Ready {
    nonce: String,
    process: Snapshot,
    station: String,
    station_created_explicitly: bool,
    desktop: String,
    device_map: device_map::Binding,
    device_map_verified_and_created: bool,
    #[cfg(feature = "native-probe-witness")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    witness_desktop_handle: Option<u64>,
}

fn save<T: Serialize>(root: &Path, name: &str, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    require(bytes.len() <= 32768, "窗口站控制记录过大")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(root.join(name))?;
    file.write_all(&bytes)?;
    file.sync_all()
}
fn quoted(value: &OsStr) -> io::Result<Vec<u16>> {
    let value = wide(value)?;
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for &unit in &value[..value.len() - 1] {
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            b'\\' as u16,
            if unit == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        result.push(unit);
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    Ok(result)
}
fn command_line(image: &Path, stage: &str, root: &Path) -> io::Result<Vec<u16>> {
    let mut value = quoted(image.as_os_str())?;
    for argument in [OsStr::new(MODE), OsStr::new(stage), root.as_os_str()] {
        value.push(b' ' as u16);
        value.extend(quoted(argument)?);
    }
    value.push(0);
    Ok(value)
}
fn environment(root: &Path) -> io::Result<Vec<u16>> {
    // 系统目录直接向内核查询，不信任继承的 PATH、SystemRoot 或用户 hook。
    let mut storage = [0u16; 32768];
    let used = unsafe { GetWindowsDirectoryW(Some(&mut storage)) } as usize;
    require(
        used > 0 && used < storage.len(),
        "窗口站引导系统目录查询失败",
    )?;
    let system = OsString::from_wide(&storage[..used]);
    let mut result = Vec::new();
    for (key, value) in [
        ("SystemRoot", system.as_os_str()),
        ("TEMP", root.as_os_str()),
        ("TMP", root.as_os_str()),
        ("WINDIR", system.as_os_str()),
    ] {
        let mut item = OsString::from(key);
        item.push("=");
        item.push(value);
        result.extend(wide(&item)?);
    }
    result.push(0);
    Ok(result)
}
fn create_private_directory(path: &Path, owner: &str) -> io::Result<()> {
    let text = wide(format!("O:{owner}D:P(A;;FA;;;{owner})").as_ref())?;
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(text.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(io::Error::other)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let path = wide(path.as_os_str())?;
    let result = unsafe { CreateDirectoryW(PCWSTR(path.as_ptr()), Some(&attributes)) }
        .map_err(io::Error::other);
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    result
}
fn current_station() -> io::Result<String> {
    let station = unsafe { GetProcessWindowStation() }.map_err(io::Error::other)?;
    let mut storage = [0u16; 256];
    let mut used = 0;
    unsafe {
        GetUserObjectInformationW(
            HANDLE(station.0),
            UOI_NAME,
            Some(storage.as_mut_ptr().cast()),
            size_of_val(&storage) as u32,
            Some(&mut used),
        )
    }
    .map_err(io::Error::other)?;
    require(
        used >= 2 && used as usize <= size_of_val(&storage) && used % 2 == 0,
        "窗口站名称长度无效",
    )?;
    let length = used as usize / 2;
    require(
        storage[length - 1] == 0 && !storage[..length - 1].contains(&0),
        "窗口站名称无效",
    )?;
    String::from_utf16(&storage[..length - 1]).map_err(io::Error::other)
}
fn station_gone(name: &str) -> io::Result<bool> {
    let name = wide(name.as_ref())?;
    match unsafe { OpenWindowStationW(PCWSTR(name.as_ptr()), false, READ_CONTROL.0 | 2) } {
        Ok(station) => {
            unsafe { CloseWindowStation(station) }.map_err(io::Error::other)?;
            Ok(false)
        }
        Err(error) if error.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => Ok(true),
        Err(error) => Err(io::Error::other(error)),
    }
}
fn logon_gone(snapshot: &Snapshot) -> io::Result<bool> {
    let luid = LUID {
        LowPart: snapshot.auth_low,
        HighPart: snapshot.auth_high,
    };
    let mut data = std::ptr::null_mut();
    let status = unsafe { LsaGetLogonSessionData(&luid, &mut data) };
    if status.0 as u32 == 0xc000005f {
        return Ok(true);
    }
    require(status.0 == 0 && !data.is_null(), "窗口站登录会话消失未确认")?;
    let status = unsafe { LsaFreeReturnBuffer(data.cast()) };
    require(status.0 == 0, "窗口站登录会话信息释放失败")?;
    Ok(false)
}
fn release_observation(station: &io::Result<bool>, logon: &io::Result<bool>) -> serde_json::Value {
    let error_fields = |result: &io::Result<bool>| {
        result.as_ref().err().map(|error| {
            let native = error
                .get_ref()
                .and_then(|error| error.downcast_ref::<windows::core::Error>())
                .map(|error| error.code().0);
            serde_json::json!({
                "kind": format!("{:?}", error.kind()),
                "os_error": error.raw_os_error(),
                "hresult": native,
            })
        })
    };
    // 查询失败必须保留未知；只归档固定错误类型与原生码，不保存错误正文。
    serde_json::json!({
        "schema": 1,
        "station_absent": station.as_ref().ok(),
        "logon_absent": logon.as_ref().ok(),
        "station_query_error": error_fields(station),
        "logon_query_error": error_fields(logon),
    })
}
fn resume(thread: HANDLE) -> io::Result<()> {
    require(
        unsafe { ResumeThread(thread) } == 1,
        "窗口站引导挂起计数不匹配",
    )
}

// 授权只通过本机消息管道；磁盘 JSON 仅用于归档，绝不用于恢复、释放或句柄移交。
struct Pipe {
    handle: OwnedHandle,
}
impl Pipe {
    fn name(nonce: &str, stage: &str) -> io::Result<Vec<u16>> {
        wide(format!(r"\\.\pipe\InfiniShell.Station.{nonce}.{stage}").as_ref())
    }
    fn server(nonce: &str, stage: &str, owner: &str) -> io::Result<Self> {
        let name = Self::name(nonce, stage)?;
        let text = wide(format!("O:{owner}D:P(A;;GA;;;{owner})").as_ref())?;
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(text.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(io::Error::other)?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE
                    | PIPE_READMODE_MESSAGE
                    | PIPE_NOWAIT
                    | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                65536,
                0,
                Some(&attributes),
            )
        };
        let error = if handle.is_invalid() {
            Some(windows::core::Error::from_thread())
        } else {
            None
        };
        unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        if let Some(error) = error {
            return Err(io::Error::other(error));
        }
        Ok(Self {
            handle: owned(handle),
        })
    }
    fn connect(nonce: &str, stage: &str) -> io::Result<(Self, OwnedHandle, Snapshot)> {
        let name = Self::name(nonce, stage)?;
        // 服务端在派生 helper 前已建立；缺失或已被占用立即拒绝，不连接其他实例。
        let pipe = Self {
            handle: owned(
                unsafe {
                    CreateFileW(
                        PCWSTR(name.as_ptr()),
                        (GENERIC_READ | GENERIC_WRITE).0,
                        FILE_SHARE_MODE(0),
                        None,
                        OPEN_EXISTING,
                        SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                        None,
                    )
                }
                .map_err(io::Error::other)?,
            ),
        };
        unsafe {
            SetNamedPipeHandleState(
                raw(&pipe.handle),
                Some(&(PIPE_READMODE_MESSAGE | PIPE_NOWAIT)),
                None,
                None,
            )
        }
        .map_err(io::Error::other)?;
        let mut pid = 0;
        let mut session = 0;
        unsafe { GetNamedPipeServerProcessId(raw(&pipe.handle), &mut pid) }
            .map_err(io::Error::other)?;
        unsafe { GetNamedPipeServerSessionId(raw(&pipe.handle), &mut session) }
            .map_err(io::Error::other)?;
        let process = owned(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    false,
                    pid,
                )
            }
            .map_err(io::Error::other)?,
        );
        let identity = Snapshot::capture(raw(&process))?;
        require(
            identity.pid == pid && identity.session == session && alive(raw(&process))?,
            "窗口站管道服务端身份无效",
        )?;
        Ok((pipe, process, identity))
    }
    fn accept(&self, process: HANDLE, expected: &Snapshot) -> io::Result<()> {
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            match unsafe { ConnectNamedPipe(raw(&self.handle), None) } {
                Err(error) if error.code() == HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) => break,
                Ok(()) => {}
                Err(error) if error.code() == HRESULT::from_win32(ERROR_PIPE_LISTENING.0) => {}
                Err(error) => return Err(io::Error::other(error)),
            }
            wait_step(deadline, process)?;
        }
        let mut pid = 0;
        let mut session = 0;
        unsafe { GetNamedPipeClientProcessId(raw(&self.handle), &mut pid) }
            .map_err(io::Error::other)?;
        unsafe { GetNamedPipeClientSessionId(raw(&self.handle), &mut session) }
            .map_err(io::Error::other)?;
        // 不按报文 PID 重开；对端必须等于调用方始终持有的原创建进程。
        require(
            pid == expected.pid
                && session == expected.session
                && Snapshot::capture(process)? == *expected
                && alive(process)?,
            "窗口站管道对端不是本轮原进程",
        )
    }
    fn send<T: Serialize>(&self, value: &T) -> io::Result<()> {
        let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        require(
            !bytes.is_empty() && bytes.len() <= 32768,
            "窗口站握手消息长度无效",
        )?;
        let mut used = 0;
        unsafe { WriteFile(raw(&self.handle), Some(&bytes), Some(&mut used), None) }
            .map_err(io::Error::other)?;
        require(used as usize == bytes.len(), "窗口站握手消息未完整写入")
    }
    fn try_receive<T: DeserializeOwned>(&self) -> io::Result<Option<T>> {
        let mut bytes = [0u8; 32768];
        let mut used = 0;
        match unsafe { ReadFile(raw(&self.handle), Some(&mut bytes), Some(&mut used), None) } {
            Ok(()) => {
                require(
                    used > 0 && used as usize <= bytes.len(),
                    "窗口站握手消息长度无效",
                )?;
                serde_json::from_slice(&bytes[..used as usize])
                    .map(Some)
                    .map_err(io::Error::other)
            }
            Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_DATA.0) => Ok(None),
            Err(error) => Err(io::Error::other(error)),
        }
    }
    fn receive<T: DeserializeOwned>(&self, process: HANDLE, deadline: Instant) -> io::Result<T> {
        loop {
            if let Some(value) = self.try_receive()? {
                return Ok(value);
            }
            wait_step(deadline, process)?;
        }
    }
}
fn alive(process: HANDLE) -> io::Result<bool> {
    match unsafe { WaitForSingleObject(process, 0) } {
        WAIT_TIMEOUT => Ok(true),
        WAIT_OBJECT_0 => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}
fn wait_step(deadline: Instant, process: HANDLE) -> io::Result<()> {
    require(alive(process)?, "窗口站握手对端提前退出")?;
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "窗口站有界等待超时",
        ));
    }
    thread::sleep(Duration::from_millis(10));
    Ok(())
}
fn exit_code(process: HANDLE) -> io::Result<u32> {
    require(!alive(process)?, "窗口站进程尚未退出")?;
    let mut code = 0;
    unsafe { GetExitCodeProcess(process, &mut code) }.map_err(io::Error::other)?;
    Ok(code)
}
fn verify_parent(expected: &Snapshot, image: &Path) -> io::Result<OwnedHandle> {
    let processes = owned(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.map_err(io::Error::other)?,
    );
    let own_pid = unsafe { GetProcessId(GetCurrentProcess()) };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    unsafe { Process32FirstW(raw(&processes), &mut entry) }.map_err(io::Error::other)?;
    loop {
        if entry.th32ProcessID == own_pid {
            require(
                entry.th32ParentProcessID == expected.pid,
                "窗口站第二段原父进程不匹配",
            )?;
            break;
        }
        match unsafe { Process32NextW(raw(&processes), &mut entry) } {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_FILES.0) => {
                return Err(invalid("窗口站第二段不在进程快照中"));
            }
            Err(error) => return Err(io::Error::other(error)),
        }
    }
    let process = owned(
        unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                expected.pid,
            )
        }
        .map_err(io::Error::other)?,
    );
    require(
        Snapshot::capture(raw(&process))? == *expected
            && process_image(raw(&process))? == image
            && alive(raw(&process))?,
        "窗口站第二段原父身份变化",
    )?;
    Ok(process)
}
fn diagnostic<T>(
    root: &Path,
    name: &str,
    phase: &'static str,
    result: &io::Result<T>,
    first_exit: Option<u32>,
    second_exit: Option<u32>,
) {
    let error = result.as_ref().err();
    let native = error
        .and_then(|error| error.get_ref())
        .and_then(|error| error.downcast_ref::<windows::core::Error>())
        .map(|error| error.code().0);
    let desktop = error.and_then(super::appcontainer::desktop::diagnostic_code);
    let check = error.and_then(super::appcontainer::desktop::diagnostic_check);
    let caller_station_matched =
        error.and_then(super::appcontainer::desktop::diagnostic_caller_station_matched);
    // 正文、路径和请求参数均不进入失败收据；不存在的 code 保留 null。
    let _ = save(
        root,
        name,
        &serde_json::json!({"schema":1,"phase":phase,"ok":result.is_ok(),"error_kind":error.map(|error| format!("{:?}", error.kind())),"check_stage":check,"caller_station_matched":caller_station_matched,"os_error":error.and_then(io::Error::raw_os_error),"hresult":native.or(desktop.map(|value| value.1)),"native_phase":desktop.map(|value| value.0),"first_exit_code":first_exit,"second_exit_code":second_exit}),
    );
}

/// 只持有本次精确 helper 原句柄与严格 Job；不复用普通同登录会话 lease。
pub(super) struct PrivateStation {
    root: PathLease,
    package_root: PathLease,
    image: StationBootstrapImage,
    job: OwnedHandle,
    first: Option<OwnedHandle>,
    first_identity: Option<Snapshot>,
    second: Option<OwnedHandle>,
    second_identity: Option<Snapshot>,
    first_pipe: Option<Pipe>,
    second_pipe: Option<Pipe>,
    debugger: Option<StationDebugger>,
    nonce: String,
    phase: &'static str,
    startup: Vec<u16>,
    device_map: Option<device_map::Binding>,
    closed: bool,
    reaped: bool,
    abort_attempted: bool,
    close_attempted: bool,
    #[cfg(feature = "native-probe-witness")]
    witness_desktop: Option<WitnessDesktop>,
    #[cfg(feature = "native-probe-witness")]
    witness_desktop_error: Option<i32>,
}
impl PrivateStation {
    pub(super) fn create(
        image: &StationBootstrapImage,
        cwd: &Path,
        profile: &str,
        sid: PSID,
        job_operation: &mut dyn FnMut(ProbeJobOperation, BorrowedHandle<'_>) -> io::Result<()>,
    ) -> io::Result<(Self, io::Result<()>)> {
        no_impersonation()?;
        image.lease.verify()?;
        let image = StationBootstrapImage::capture(image.lease.path(), image.size, &image.sha256)?;
        let caller = Snapshot::capture(unsafe { GetCurrentProcess() })?;
        require(
            caller.user != "S-1-5-18",
            "局部盘映射拒绝 LocalSystem 调用方",
        )?;
        let caller_station = current_station()?;
        let package_root = PathLease::capture(cwd)?;
        let device_target = device_map::Target::capture(&package_root)?;
        let nonce = Uuid::new_v4().simple().to_string();
        let root = cwd.join(format!("station-{nonce}"));
        create_private_directory(&root, &caller.user)?;
        let root = PathLease::capture(&root)?;
        let request = Request {
            schema: 1,
            nonce: nonce.clone(),
            profile: profile.to_owned(),
            container_sid: sid_text(sid)?,
            caller,
            caller_station,
            image: image.lease.path().to_owned(),
            image_size: image.size,
            image_sha256: image.sha256.clone(),
            device_target,
            #[cfg(feature = "native-probe-witness")]
            witness_desktop: std::env::var("INFINISHELL_WINDOWS_NATIVE_WITNESS_GENERATION")
                .ok()
                .is_some_and(|generation| profile == format!("InfiniShell.Version.{generation}")),
        };
        let job = owned(unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::other)?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                raw(&job),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        }
        .map_err(io::Error::other)?;
        let mut result = Self {
            root,
            package_root,
            image,
            job,
            first: None,
            first_identity: None,
            second: None,
            second_identity: None,
            first_pipe: None,
            second_pipe: None,
            debugger: None,
            nonce,
            phase: "prepare",
            startup: Vec::new(),
            device_map: None,
            closed: false,
            reaped: false,
            abort_attempted: false,
            close_attempted: false,
            #[cfg(feature = "native-probe-witness")]
            witness_desktop: None,
            #[cfg(feature = "native-probe-witness")]
            witness_desktop_error: None,
        };
        let started = result.start(&request, job_operation);
        diagnostic(
            result.root.path(),
            "start-result.json",
            result.phase,
            &started,
            result
                .first
                .as_ref()
                .and_then(|process| exit_code(raw(process)).ok()),
            result
                .second
                .as_ref()
                .and_then(|process| exit_code(raw(process)).ok()),
        );
        // 启动失败也先移交持有对象，让调用方按原回收结果决定是否清理 profile。
        Ok((result, started))
    }
    fn start(
        &mut self,
        request: &Request,
        job_operation: &mut dyn FnMut(ProbeJobOperation, BorrowedHandle<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.phase = "create_pipes";
        self.first_pipe = Some(Pipe::server(&request.nonce, "first", &request.caller.user)?);
        self.second_pipe = Some(Pipe::server(
            &request.nonce,
            "second",
            &request.caller.user,
        )?);
        let program = wide(self.image.lease.path().as_os_str())?;
        let mut command = command_line(self.image.lease.path(), "first", self.root.path())?;
        let cwd = wide(self.root.path().as_os_str())?;
        let environment = environment(self.root.path())?;
        let user = wide(format!("ISP_{}", &request.nonce[..12]).as_ref())?;
        let domain = wide(".".as_ref())?;
        let mut password = wide(format!("UnusedOnly-{}-!a9", request.nonce).as_ref())?;
        let startup = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut created = PROCESS_INFORMATION::default();
        self.image.lease.verify()?;
        self.root.verify()?;
        self.phase = "create_process_with_logon";
        let spawned = unsafe {
            CreateProcessWithLogonW(
                PCWSTR(user.as_ptr()),
                PCWSTR(domain.as_ptr()),
                PCWSTR(password.as_ptr()),
                LOGON_NETCREDENTIALS_ONLY,
                PCWSTR(program.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                Some(environment.as_ptr().cast()),
                PCWSTR(cwd.as_ptr()),
                &startup,
                &mut created,
            )
        };
        for unit in &mut password {
            unsafe { std::ptr::write_volatile(unit, 0) };
        }
        spawned.map_err(io::Error::other)?;
        self.first = Some(owned(created.hProcess));
        let thread = owned(created.hThread);
        let first = raw(self.first.as_ref().unwrap());
        self.phase = "first_identity_and_job";
        let identity = Snapshot::capture(first)?;
        self.first_identity = Some(identity.clone());
        require(
            identity.pid == created.dwProcessId
                && identity.same_local(&request.caller)
                && !identity.same_logon(&request.caller),
            "窗口站引导首段身份不匹配",
        )?;
        require(
            process_image(first)? == request.image,
            "窗口站引导首段映像不匹配",
        )?;
        // 首段原句柄仍挂起；先由监督者接入原外层 Job，再建立本轮严格子 Job。
        job_operation(
            ProbeJobOperation::ClaimBootstrap,
            self.first.as_ref().unwrap().as_handle(),
        )?;
        unsafe { AssignProcessToJobObject(raw(&self.job), first) }.map_err(io::Error::other)?;
        self.verify_member(first)?;
        require(
            station_gone(&identity.station_name())?,
            "新登录窗口站在本轮开始前已存在",
        )?;
        let setup = Setup {
            request: request.clone(),
            binding: Binding {
                nonce: request.nonce.clone(),
                first: identity.clone(),
            },
            second: None,
        };
        save(self.root.path(), "binding.json", &setup.binding)?;
        require(
            Snapshot::capture(first)? == identity,
            "窗口站首段恢复前身份变化",
        )?;
        resume(raw(&thread))?;
        self.phase = "first_pipe_identity";
        let pipe = self.first_pipe.as_ref().unwrap();
        pipe.accept(first, &identity)?;
        pipe.send(&setup)?;
        self.phase = "second_original_handle";
        let second: Second = pipe.receive(first, Instant::now() + START_TIMEOUT)?;
        require(
            second.nonce == request.nonce
                && second.source_handle > 0
                && second.source_handle < isize::MAX as u64,
            "窗口站第二段句柄移交无效",
        )?;
        // 报文只给首段地址空间中的源值；仅内核 DuplicateHandle 新生成的本地句柄可接管。
        let mut duplicated = HANDLE::default();
        let access = PROCESS_QUERY_LIMITED_INFORMATION
            | PROCESS_SYNCHRONIZE
            | PROCESS_CREATE_PROCESS
            | PROCESS_DUP_HANDLE;
        unsafe {
            DuplicateHandle(
                first,
                HANDLE(second.source_handle as usize as *mut c_void),
                GetCurrentProcess(),
                &mut duplicated,
                access.0,
                false,
                DUPLICATE_HANDLE_OPTIONS(0),
            )
        }
        .map_err(io::Error::other)?;
        self.second = Some(owned(duplicated));
        let process = raw(self.second.as_ref().unwrap());
        require(
            Snapshot::capture(process)? == second.process
                && second.process.same_local(&identity)
                && second.process.same_logon(&identity)
                && second.process.pid != identity.pid
                && process_image(process)? == request.image,
            "窗口站第二段原句柄身份不匹配",
        )?;
        self.second_identity = Some(second.process.clone());
        self.verify_member(process)?;
        self.verify_member(first)?;
        self.root.verify()?;
        self.image.lease.verify()?;
        save(self.root.path(), "second.json", &second)?;
        pipe.send(&second)?;
        self.phase = "second_pipe_identity";
        let pipe = self.second_pipe.as_ref().unwrap();
        pipe.accept(process, &second.process)?;
        pipe.send(&Setup {
            second: Some(second.clone()),
            ..setup
        })?;
        self.phase = "station_authorization";
        let ready: Ready = pipe.receive(process, Instant::now() + START_TIMEOUT)?;
        validate_ready(request, &identity, &second.process, &ready)?;
        ready.device_map.verify_hidden_from_caller()?;
        require(
            Snapshot::capture(process)? == second.process && alive(process)?,
            "窗口站授权持有者已退出",
        )?;
        #[cfg(feature = "native-probe-witness")]
        if request.witness_desktop {
            // 只有原 helper 报出的实际持有对象参与比较，不按桌面名称重新打开。
            if let Some(source) = ready
                .witness_desktop_handle
                .filter(|value| *value > 0 && *value < isize::MAX as u64)
            {
                let mut desktop = HANDLE::default();
                let result = unsafe {
                    DuplicateHandle(
                        process,
                        HANDLE(source as usize as *mut c_void),
                        GetCurrentProcess(),
                        &mut desktop,
                        0,
                        false,
                        DUPLICATE_SAME_ACCESS,
                    )
                };
                match result {
                    Ok(()) => self.witness_desktop = Some(WitnessDesktop(Some(desktop.0 as usize))),
                    Err(error) => self.witness_desktop_error = Some(error.code().0),
                }
            }
        }
        save(self.root.path(), "ready.json", &ready)?;
        self.startup = wide(format!("{}\\{}", ready.station, ready.desktop).as_ref())?;
        self.device_map = Some(ready.device_map);
        self.debugger = Some(StationDebugger::new(
            self.second_pipe
                .take()
                .ok_or_else(|| invalid("窗口站调试管道缺失"))?,
            process,
            second.process.clone(),
            self.nonce.clone(),
        )?);
        self.phase = "ready";
        Ok(())
    }
    pub(super) fn debugger_handle(&self) -> Option<StationDebugger> {
        self.debugger.clone()
    }
    pub(super) fn resume_package(&self) -> io::Result<()> {
        self.debugger()?.resume()
    }
    pub(super) fn debugger(&self) -> io::Result<StationDebugger> {
        self.parent_process()?;
        self.debugger
            .clone()
            .ok_or_else(|| invalid("窗口站调试控制缺失"))
    }
    pub(super) fn spawn_package(&self, request: StationSpawn) -> io::Result<PROCESS_INFORMATION> {
        self.debugger()?.spawn(request)
    }
    fn verify_member(&self, process: HANDLE) -> io::Result<()> {
        let mut contained = BOOL::default();
        unsafe { IsProcessInJob(process, Some(raw(&self.job)), &mut contained) }
            .map_err(io::Error::other)?;
        require(contained.as_bool(), "窗口站引导进程不在本次严格 Job")
    }
    #[cfg(feature = "native-probe-witness")]
    pub(super) fn native_witness_desktop(&self) -> io::Result<HDESK> {
        require(!self.closed && !self.reaped, "取证桌面所有者已清理")?;
        let process = self
            .second
            .as_ref()
            .ok_or_else(|| invalid("取证桌面原所有者缺失"))?;
        let expected = self
            .second_identity
            .as_ref()
            .ok_or_else(|| invalid("取证桌面原身份缺失"))?;
        self.verify_member(raw(process))?;
        require(
            Snapshot::capture(raw(process))? == *expected && alive(raw(process))?,
            "取证桌面原所有者身份变化",
        )?;
        self.root.verify()?;
        self.image.lease.verify()?;
        if let Some(code) = self.witness_desktop_error {
            return Err(io::Error::other(windows::core::Error::from_hresult(
                HRESULT(code),
            )));
        }
        let handle = self
            .witness_desktop
            .as_ref()
            .and_then(|desktop| desktop.0)
            .ok_or_else(|| invalid("取证桌面对象未取得"))?;
        Ok(HDESK(handle as *mut c_void))
    }
    pub(super) fn configure_startup(&mut self, startup: &mut STARTUPINFOW) -> io::Result<()> {
        self.parent_process()?;
        startup.lpDesktop = PWSTR(self.startup.as_mut_ptr());
        Ok(())
    }
    pub(super) fn parent_process(&self) -> io::Result<BorrowedHandle<'_>> {
        require(
            !self.closed && !self.reaped && !self.startup.is_empty(),
            "窗口站所有权尚未建立或已释放",
        )?;
        self.root.verify()?;
        self.package_root.verify()?;
        self.image.lease.verify()?;
        for (process, expected) in [
            (&self.first, &self.first_identity),
            (&self.second, &self.second_identity),
        ] {
            let process = raw(process
                .as_ref()
                .ok_or_else(|| invalid("窗口站持有者缺失"))?);
            let expected = expected
                .as_ref()
                .ok_or_else(|| invalid("窗口站持有者身份缺失"))?;
            self.verify_member(process)?;
            require(
                alive(process)? && Snapshot::capture(process)? == *expected,
                "窗口站持有者退出或身份变化",
            )?;
        }
        let second = self
            .second
            .as_ref()
            .ok_or_else(|| invalid("窗口站父进程缺失"))?;
        Ok(second.as_handle())
    }
    pub(super) fn mapped_root(&self) -> io::Result<&Path> {
        self.parent_process()?;
        let binding = self
            .device_map
            .as_ref()
            .ok_or_else(|| invalid("局部盘映射授权缺失"))?;
        binding.validate(
            &device_map::Target::capture(&self.package_root)?,
            self.second_identity
                .as_ref()
                .ok_or_else(|| invalid("局部盘映射原身份缺失"))?,
        )?;
        Ok(&binding.mapped_root)
    }
    pub(super) fn verify_package_identity(&self, process: BorrowedHandle<'_>) -> io::Result<()> {
        self.parent_process()?;
        let process = HANDLE(process.as_raw_handle());
        self.verify_member(process)?;
        let expected = self
            .second_identity
            .as_ref()
            .ok_or_else(|| invalid("局部盘映射原身份缺失"))?;
        require(
            Snapshot::capture(process)?.same_package_logon(expected),
            "候选进程没有继承局部盘映射的登录身份",
        )
    }
    fn empty(&self) -> io::Result<bool> {
        let mut state = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(raw(&self.job)),
                JobObjectBasicAccountingInformation,
                (&mut state as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of_val(&state) as u32,
                None,
            )
        }
        .map_err(io::Error::other)?;
        if state.ActiveProcesses != 0 {
            return Ok(false);
        }
        for process in [&self.first, &self.second].into_iter().flatten() {
            if alive(raw(process))? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn wait_empty(&self, deadline: Instant) -> io::Result<()> {
        while !self.empty()? {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "窗口站辅助进程尚未退出",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
    fn release_and_verify(&mut self, deadline: Instant, receipt: &str) -> io::Result<()> {
        #[cfg(feature = "native-probe-witness")]
        if let Some(desktop) = &mut self.witness_desktop {
            desktop.close()?;
        }
        // 进程对象本身可保留主 token；原句柄确认退出后先释放，再核 LSA 会话消失。
        if let Some(debugger) = self.debugger.take() {
            debugger.invalidate()?;
        }
        self.first.take();
        self.second.take();
        self.first_pipe.take();
        self.second_pipe.take();
        if let Some(identity) = &self.first_identity {
            loop {
                // 两侧均实际查询；窗口站存在时也保留本轮 LSA 状态，避免短路掩盖剩余资源。
                let station = station_gone(&identity.station_name());
                let logon = logon_gone(identity);
                let complete = matches!((&station, &logon), (Ok(true), Ok(true)));
                let failed = station.is_err() || (matches!(station, Ok(true)) && logon.is_err());
                if complete || failed || Instant::now() >= deadline {
                    let _ = save(
                        self.root.path(),
                        receipt,
                        &release_observation(&station, &logon),
                    );
                    // 保留原错误优先级：站未消失时，新增 LSA 观察不改变原等待/超时结果。
                    if station? && logon? {
                        break;
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "窗口站或新登录会话尚未消失",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    }
    pub(super) fn close(&mut self) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        require(!self.close_attempted, "窗口站正常回收已失败")?;
        self.close_attempted = true;
        let result = self.close_inner();
        diagnostic(
            self.root.path(),
            "close-result.json",
            self.phase,
            &result,
            self.first
                .as_ref()
                .and_then(|process| exit_code(raw(process)).ok()),
            self.second
                .as_ref()
                .and_then(|process| exit_code(raw(process)).ok()),
        );
        result
    }
    fn close_inner(&mut self) -> io::Result<()> {
        require(!self.startup.is_empty(), "窗口站尚未授权完成")?;
        #[cfg(feature = "native-probe-witness")]
        if let Some(desktop) = &mut self.witness_desktop {
            desktop.close()?;
        }
        self.phase = "request_desktop_close";
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        let process = raw(self
            .second
            .as_ref()
            .ok_or_else(|| invalid("窗口站第二段缺失"))?);
        let closed = self
            .debugger
            .as_ref()
            .ok_or_else(|| invalid("窗口站调试控制缺失"))?
            .close(deadline)?;
        require(
            closed.nonce == self.nonce
                && Some(&closed.process) == self.second_identity.as_ref()
                && closed.desktop_verified_and_closed
                && Some(&closed.device_map) == self.device_map.as_ref()
                && closed.device_map_verified_and_removed,
            "窗口站原桌面关闭证明不匹配",
        )?;
        save(self.root.path(), "closed.json", &closed)?;
        self.phase = "helper_exit_codes";
        self.wait_empty(deadline)?;
        let first = exit_code(raw(self.first.as_ref().unwrap()))?;
        let second = exit_code(process)?;
        save(
            self.root.path(),
            "helper-exit.json",
            &serde_json::json!({"first_exit_code":first,"second_exit_code":second,"desktop_verified_and_closed":true,"device_map_verified_and_removed":true,"job_empty":true}),
        )?;
        require(first == 0 && second == 0, "窗口站辅助进程返回失败")?;
        self.phase = "station_and_logon_released";
        self.release_and_verify(deadline, "release-state.json")?;
        save(
            self.root.path(),
            "cleanup.json",
            &serde_json::json!({"graceful":true,"helpers_exit_zero":true,"desktop_verified_and_closed":true,"device_map_verified_and_removed":true,"job_empty":true,"station_absent":true,"logon_absent":true}),
        )?;
        self.closed = true;
        Ok(())
    }
}
impl PrivateStation {
    pub(super) fn abort_and_reap(&mut self) -> io::Result<()> {
        if self.closed || self.reaped {
            return Ok(());
        }
        require(!self.abort_attempted, "窗口站异常回收已失败")?;
        // 失败后的字段析构不能暗中再开启一轮等待，或把未确认回收当作成功。
        self.abort_attempted = true;
        let mut first_exit = self
            .first
            .as_ref()
            .and_then(|process| exit_code(raw(process)).ok());
        let mut second_exit = self
            .second
            .as_ref()
            .and_then(|process| exit_code(raw(process)).ok());
        // 首段可能在入 Job 前失败；只终止自己仍持有的原句柄，不按 PID 查找。
        if let Some(first) = &self.first {
            let _ = unsafe { TerminateProcess(raw(first), 1) };
        }
        let termination =
            unsafe { TerminateJobObject(raw(&self.job), 1) }.map_err(io::Error::other);
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        let result = termination
            .and_then(|()| self.wait_empty(deadline))
            .and_then(|()| {
                first_exit = self
                    .first
                    .as_ref()
                    .and_then(|process| exit_code(raw(process)).ok());
                second_exit = self
                    .second
                    .as_ref()
                    .and_then(|process| exit_code(raw(process)).ok());
                self.release_and_verify(deadline, "aborted-release-state.json")
            });
        diagnostic(
            self.root.path(),
            "aborted-cleanup.json",
            "aborted_cleanup_only",
            &result,
            first_exit,
            second_exit,
        );
        if result.is_ok() {
            self.reaped = true;
        }
        // 只证明失败资源已回收，不改变 close 的失败，也不写正常 cleanup.json。
        result
    }
}
impl Drop for PrivateStation {
    fn drop(&mut self) {
        if !self.abort_attempted {
            let _ = self.abort_and_reap();
        }
    }
}

fn validate_ready(
    request: &Request,
    first: &Snapshot,
    second: &Snapshot,
    ready: &Ready,
) -> io::Result<()> {
    ready.device_map.validate(&request.device_target, second)?;
    require(
        ready.nonce == request.nonce
            && ready.process == *second
            && ready.station.eq_ignore_ascii_case(&first.station_name())
            && !ready.station.eq_ignore_ascii_case(&request.caller_station)
            && ready.desktop == request.profile
            && ready.device_map_verified_and_created,
        "窗口站授权收据不匹配",
    )
}
struct ChildGuard {
    process: OwnedHandle,
    thread: OwnedHandle,
    complete: bool,
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.complete {
            let _ = unsafe { TerminateProcess(raw(&self.process), 1) };
            unsafe { WaitForSingleObject(raw(&self.process), CLEANUP_TIMEOUT.as_millis() as u32) };
        }
    }
}
fn helper_first(
    root: &Path,
    setup: &Setup,
    pipe: &Pipe,
    caller: HANDLE,
    phase: &mut &'static str,
) -> io::Result<()> {
    let request = &setup.request;
    let binding = &setup.binding;
    require(setup.second.is_none(), "窗口站首段不接受第二段授权")?;
    require(
        current_station()?.eq_ignore_ascii_case(&request.caller_station),
        "窗口站首段没有保留调用方窗口站",
    )?;
    // 再次由原创建父进程确认新站不存在，不能靠调用方声明证明新对象所有权。
    *phase = "verify_station_absent";
    require(
        station_gone(&binding.first.station_name())?,
        "窗口站首段拒绝已有新登录站",
    )?;
    let image = wide(request.image.as_os_str())?;
    let mut command = command_line(&request.image, "second", root)?;
    let cwd = wide(root.as_os_str())?;
    let environment = environment(root)?;
    let mut empty = [0u16];
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        lpDesktop: PWSTR(empty.as_mut_ptr()),
        ..Default::default()
    };
    let mut created = PROCESS_INFORMATION::default();
    *phase = "create_second_suspended";
    unsafe {
        CreateProcessW(
            PCWSTR(image.as_ptr()),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            Some(environment.as_ptr().cast()),
            PCWSTR(cwd.as_ptr()),
            &startup,
            &mut created,
        )
    }
    .map_err(io::Error::other)?;
    let mut child = ChildGuard {
        process: owned(created.hProcess),
        thread: owned(created.hThread),
        complete: false,
    };
    let identity = Snapshot::capture(raw(&child.process))?;
    require(
        identity.pid == created.dwProcessId
            && identity.same_local(&binding.first)
            && identity.same_logon(&binding.first)
            && process_image(raw(&child.process))? == request.image,
        "窗口站第二段创建身份不匹配",
    )?;
    let second = Second {
        nonce: request.nonce.clone(),
        process: identity.clone(),
        source_handle: raw(&child.process).0 as usize as u64,
    };
    pipe.send(&second)?;
    *phase = "second_resume_authorization";
    let authorization: Second = pipe.receive(caller, Instant::now() + START_TIMEOUT)?;
    require(
        authorization == second && Snapshot::capture(raw(&child.process))? == identity,
        "窗口站第二段恢复授权不匹配",
    )?;
    resume(raw(&child.thread))?;
    *phase = "second_lifetime";
    let deadline = Instant::now() + LIFETIME;
    while alive(raw(&child.process))? {
        wait_step(deadline, caller)?;
    }
    let code = exit_code(raw(&child.process))?;
    child.complete = true;
    save(
        root,
        "first-child-exit.json",
        &serde_json::json!({"second_exit_code":code}),
    )?;
    require(code == 0, "窗口站第二段授权或回收失败")
}
fn helper_second(
    setup: &Setup,
    pipe: &Pipe,
    caller: HANDLE,
    phase: &mut &'static str,
) -> io::Result<()> {
    let request = &setup.request;
    let binding = &setup.binding;
    let identity = Snapshot::capture(unsafe { GetCurrentProcess() })?;
    let authorization = setup
        .second
        .as_ref()
        .ok_or_else(|| invalid("窗口站第二段缺少恢复授权"))?;
    require(
        authorization.process == identity
            && authorization.nonce == request.nonce
            && identity.same_local(&binding.first)
            && identity.same_logon(&binding.first),
        "窗口站第二段自身身份不匹配",
    )?;
    let parent = verify_parent(&binding.first, &request.image)?;
    *phase = "local_device_map";
    let mut device_map = device_map::LocalDeviceMap::create(&request.device_target, &identity)?;
    *phase = "station_acl_and_desktop";
    let mut desktop = NewLogonDesktop::create(
        &binding.first.station_name(),
        &request.profile,
        &request.container_sid,
        &request.caller_station,
    )?;
    let ready = Ready {
        nonce: request.nonce.clone(),
        process: identity.clone(),
        station: binding.first.station_name(),
        station_created_explicitly: desktop.station_owned(),
        desktop: request.profile.clone(),
        device_map: device_map.binding().clone(),
        device_map_verified_and_created: true,
        #[cfg(feature = "native-probe-witness")]
        witness_desktop_handle: request
            .witness_desktop
            .then(|| {
                desktop
                    .witness_handle()
                    .map(|handle| handle.0 as usize as u64)
            })
            .transpose()?,
    };
    *phase = "station_ready_send";
    pipe.send(&ready)?;
    *phase = "desktop_lifetime";
    let deadline = Instant::now() + LIFETIME;
    let mut debugger = debugger::Server::new()?;
    let close_sequence = loop {
        require(alive(raw(&parent))?, "窗口站原父进程已退出")?;
        desktop.verify()?;
        if let Some(sequence) = debugger.poll(pipe, request)? {
            break sequence;
        }
        wait_step(deadline, caller)?;
    };
    *phase = "desktop_verify_and_close";
    desktop.verify()?;
    desktop.close()?;
    *phase = "device_map_verify_and_remove";
    let device_binding = device_map.binding().clone();
    device_map.close()?;
    debugger::Server::closed(
        pipe,
        close_sequence,
        Closed {
            nonce: request.nonce.clone(),
            process: identity,
            desktop_verified_and_closed: true,
            device_map: device_binding,
            device_map_verified_and_removed: true,
        },
    )
}

/// 只供明确部署的轻量 binary 调用；严格的两个阶段不提供任意命令执行接口。
pub fn run_station_bootstrap() -> io::Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    require(args.len() == 4 && args[1] == MODE, "窗口站引导参数不匹配")?;
    let stage = args[2]
        .to_str()
        .ok_or_else(|| invalid("窗口站引导阶段无效"))?;
    require(matches!(stage, "first" | "second"), "窗口站引导阶段未知")?;
    no_impersonation()?;
    let root = PathLease::capture(Path::new(&args[3]))?;
    let nonce = root
        .path()
        .file_name()
        .and_then(OsStr::to_str)
        .and_then(|value| value.strip_prefix("station-"))
        .filter(|value| {
            value.len() == 32
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| invalid("窗口站引导目录名无效"))?;
    let mut phase = "connect_control_pipe";
    let result = (|| {
        let (pipe, caller, caller_identity) = Pipe::connect(nonce, stage)?;
        phase = "receive_setup";
        let setup: Setup = pipe.receive(raw(&caller), Instant::now() + START_TIMEOUT)?;
        let request = &setup.request;
        require(
            request.schema == 1
                && request.nonce == nonce
                && request.caller == caller_identity
                && request.profile.starts_with("InfiniShell.Version.")
                && Uuid::parse_str(&request.profile["InfiniShell.Version.".len()..]).is_ok(),
            "窗口站引导请求合同无效",
        )?;
        let image = StationBootstrapImage::capture(
            &request.image,
            request.image_size,
            &request.image_sha256,
        )?;
        require(
            std::env::current_exe()?.canonicalize()? == image.lease.path(),
            "窗口站引导自身映像不匹配",
        )?;
        let binding = &setup.binding;
        require(
            binding.nonce == request.nonce
                && binding.first.same_local(&request.caller)
                && !binding.first.same_logon(&request.caller),
            "窗口站引导登录绑定不匹配",
        )?;
        require(
            root.path().parent() == Some(request.device_target.path.as_path()),
            "局部盘映射目标不是本轮候选树",
        )?;
        root.verify()?;
        image.lease.verify()?;
        match stage {
            "first" => {
                require(
                    Snapshot::capture(unsafe { GetCurrentProcess() })? == binding.first,
                    "窗口站首段自身身份不匹配",
                )?;
                helper_first(root.path(), &setup, &pipe, raw(&caller), &mut phase)
            }
            "second" => helper_second(&setup, &pipe, raw(&caller), &mut phase),
            _ => Err(invalid("窗口站引导阶段未知")),
        }
    })();
    diagnostic(
        root.path(),
        &format!("{stage}-result.json"),
        phase,
        &result,
        None,
        None,
    );
    result
}

#[cfg(test)]
#[path = "windows_station_bootstrap_tests.rs"]
pub(super) mod tests;
