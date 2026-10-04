//! 无网络 capability 的一次性版本探针；所有派生留在严格 Job，句柄继承仅限标准流。

use std::borrow::Cow;
use std::ffi::{OsStr, OsString, c_void};
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle};
use std::path::{Component, Path, PathBuf, Prefix};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_NO_TOKEN, HANDLE, HLOCAL,
    LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetSecurityInfo, SE_FILE_OBJECT,
    SetEntriesInAclW, SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows::Win32::Security::Isolation::{CreateAppContainerProfile, DeleteAppContainerProfile};
use windows::Win32::Security::{
    ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, FreeSid, GetTokenInformation,
    IsValidAcl, NO_INHERITANCE, OBJECT_INHERIT_ACE, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_CAPABILITIES, TOKEN_APPCONTAINER_INFORMATION, TOKEN_IMPERSONATE, TOKEN_QUERY,
    TokenAppContainerSid, TokenCapabilities, TokenIsAppContainer,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, READ_CONTROL, WRITE_DAC,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    CreateProcessW, DEBUG_PROCESS, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetCurrentProcess, GetCurrentThread, GetExitCodeProcess, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken, OpenThreadToken,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, ResumeThread, STARTF_USESHOWWINDOW,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, KF_FLAG_DONT_VERIFY, KF_FLAG_NO_PACKAGE_REDIRECTION,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{BOOL, GUID, HRESULT, PCWSTR, PWSTR};

use super::station_bootstrap::{
    PrivateStation, StationBootstrapImage, StationDebugger, StationSpawn,
};
use crate::managed::ProbeJobOperation;

#[path = "windows_appcontainer_output.rs"]
mod output;
use output::CapturedOutput;

fn wide(value: &std::ffi::OsStr) -> io::Result<Vec<u16>> {
    let mut bytes: Vec<_> = value.encode_wide().collect();
    if bytes.contains(&0) {
        return Err(io::Error::other("版本探针路径含 NUL"));
    }
    bytes.push(0);
    Ok(bytes)
}

fn owned(handle: HANDLE) -> OwnedHandle {
    unsafe { OwnedHandle::from_raw_handle(handle.0) }
}
fn handle(value: &OwnedHandle) -> HANDLE {
    HANDLE(value.as_raw_handle())
}

// 保留失败时的原始 out 指针，满足 Known Folder API 在失败分支也释放缓冲区的合同。
#[link(name = "shell32")]
unsafe extern "system" {
    #[link_name = "SHGetKnownFolderPath"]
    fn profile_known_folder_path(
        folder: *const GUID,
        flags: u32,
        token: HANDLE,
        path: *mut PWSTR,
    ) -> HRESULT;
}

#[link(name = "userenv")]
unsafe extern "system" {
    #[link_name = "GetAppContainerFolderPath"]
    fn profile_container_folder_path(sid: PCWSTR, path: *mut PWSTR) -> HRESULT;
}

fn profile_without_impersonation() -> io::Result<()> {
    let mut token = HANDLE::default();
    match unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) } {
        Ok(()) => {
            drop(owned(token));
            Err(io::Error::other("版本探针 profile 查询拒绝线程模拟身份"))
        }
        Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_TOKEN.0) => Ok(()),
        Err(error) => Err(io::Error::other(error)),
    }
}

fn local_profile_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(
        components.next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    ) && matches!(components.next(), Some(Component::RootDir))
        && components.all(|part| matches!(part, Component::Normal(_)))
}

fn take_profile_path(status: HRESULT, pointer: PWSTR) -> io::Result<PathBuf> {
    let result = (|| {
        status.ok().map_err(io::Error::other)?;
        if pointer.is_null() {
            return Err(io::Error::other("版本探针原生 profile 路径缺失"));
        }
        // 指针来自固定 Windows API 的 NUL 结尾字符串；只复制路径，不读取目录内容。
        let path = PathBuf::from(unsafe { OsString::from_wide(pointer.as_wide()) });
        if !local_profile_path(&path) {
            return Err(io::Error::other("版本探针原生 profile 需要本地绝对目录"));
        }
        Ok(path)
    })();
    unsafe { CoTaskMemFree(Some(pointer.0.cast())) };
    result
}

fn native_profile_paths(sid: PSID) -> io::Result<(PathBuf, PathBuf)> {
    profile_without_impersonation()?;
    let mut token = HANDLE::default();
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_IMPERSONATE,
            &mut token,
        )
    }
    .map_err(io::Error::other)?;
    let token = owned(token);
    let mut base = PWSTR::null();
    let status = unsafe {
        profile_known_folder_path(
            &FOLDERID_LocalAppData,
            (KF_FLAG_NO_PACKAGE_REDIRECTION | KF_FLAG_DONT_VERIFY).0 as u32,
            handle(&token),
            &mut base,
        )
    };
    // 显式传入原进程用户 token 查询基址，不退回私有环境值或设置线程 token。
    // 不触发 Known Folder 的创建/初始化或网络验证，随后由只读本地目录句柄核实。
    let base = take_profile_path(status, base)?;
    let mut sid_text = PWSTR::null();
    let converted = unsafe { ConvertSidToStringSidW(sid, &mut sid_text) };
    let result = (|| {
        converted.map_err(io::Error::other)?;
        if sid_text.is_null() {
            return Err(io::Error::other("版本探针 profile SID 字符串缺失"));
        }
        let mut profile = PWSTR::null();
        let status = unsafe { profile_container_folder_path(PCWSTR(sid_text.0), &mut profile) };
        let profile = take_profile_path(status, profile)?;
        profile_without_impersonation()?;
        Ok((base, profile))
    })();
    unsafe { LocalFree(Some(HLOCAL(sid_text.0.cast()))) };
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProfileDirectoryIdentity {
    volume: u32,
    high: u32,
    low: u32,
}

fn profile_directory_identity(file: &File) -> io::Result<ProfileDirectoryIdentity> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
        .map_err(io::Error::other)?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0
    {
        return Err(io::Error::other(
            "版本探针 profile 目录类型或重解析属性无效",
        ));
    }
    Ok(ProfileDirectoryIdentity {
        volume: information.dwVolumeSerialNumber,
        high: information.nFileIndexHigh,
        low: information.nFileIndexLow,
    })
}

fn open_profile_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
        .open(path)
}

fn profile_is_below_base(
    base: &[ProfileDirectoryIdentity],
    profile: &[ProfileDirectoryIdentity],
) -> bool {
    !base.is_empty() && profile.len() > base.len() && profile.starts_with(base)
}

struct NativeProfileDirectories {
    base: PathBuf,
    profile: PathBuf,
    directories: Vec<(PathBuf, File, ProfileDirectoryIdentity)>,
}

impl NativeProfileDirectories {
    fn capture(sid: PSID) -> io::Result<Self> {
        let (base, profile) = native_profile_paths(sid)?;
        let mut directories = Vec::new();
        let mut identities = Vec::new();
        for path in [&base, &profile] {
            let mut chain = Vec::new();
            // 从卷根逐级锁住原路径，不先 canonicalize 后丢失重解析祖先的证据。
            for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
                let file = open_profile_directory(ancestor)?;
                let identity = profile_directory_identity(&file)?;
                chain.push(identity);
                directories.push((ancestor.to_owned(), file, identity));
            }
            identities.push(chain);
        }
        if !profile_is_below_base(&identities[0], &identities[1]) {
            return Err(io::Error::other(
                "版本探针 SID profile 不属于原生 LocalAppData",
            ));
        }
        Ok(Self {
            base,
            profile,
            directories,
        })
    }

    fn verify(&self, sid: PSID) -> io::Result<()> {
        let (base, profile) = native_profile_paths(sid)?;
        if base != self.base || profile != self.profile {
            return Err(io::Error::other("版本探针原生 profile 映射变化"));
        }
        for (path, file, identity) in &self.directories {
            if profile_directory_identity(file)? != *identity
                || profile_directory_identity(&open_profile_directory(path)?)? != *identity
            {
                return Err(io::Error::other("版本探针原生 profile 目录身份变化"));
            }
        }
        Ok(())
    }
}

fn profile_environment(
    environment: &[(OsString, OsString)],
    base: &OsStr,
) -> io::Result<Vec<(OsString, OsString)>> {
    let mut matching = environment.iter().enumerate().filter(|(_, (key, _))| {
        key.to_str()
            .is_some_and(|key| key.eq_ignore_ascii_case("LOCALAPPDATA"))
    });
    let Some((index, _)) = matching.next() else {
        return Err(io::Error::other("版本探针环境缺少 LOCALAPPDATA"));
    };
    if matching.next().is_some() {
        return Err(io::Error::other("版本探针环境 LOCALAPPDATA 重复"));
    }
    let mut result = environment.to_vec();
    // 仅候选副本替换唯一值；保留键名、顺序以及其他所有变量的原字节。
    result[index].1 = base.to_owned();
    Ok(result)
}

struct Grant {
    file: File,
    descriptor: PSECURITY_DESCRIPTOR,
    original: *mut ACL,
    granted: *mut ACL,
    applied: Vec<u8>,
    restored: bool,
}

impl Grant {
    fn new(path: &Path, sid: PSID, writable_directory: bool) -> io::Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink()
            || path.canonicalize()? != path
            || writable_directory && !metadata.is_dir()
        {
            return Err(io::Error::other("版本探针授权对象不匹配"));
        }
        let file = OpenOptions::new()
            .access_mode(READ_CONTROL.0 | WRITE_DAC.0)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
            .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
            .open(path)?;
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let mut original = std::ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                HANDLE(file.as_raw_handle()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut original),
                None,
                Some(&mut descriptor),
            )
        };
        if status.0 != 0 {
            return Err(io::Error::from_raw_os_error(status.0 as i32));
        }
        if original.is_null() {
            unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
            return Err(io::Error::other("版本探针拒绝空 DACL"));
        }
        let mut entry = EXPLICIT_ACCESS_W::default();
        // candidate 仅授予读取/执行；私有目录可写，但不授予改 DACL 或 owner 的权限。
        entry.grfAccessPermissions = if writable_directory {
            0x001301ff
        } else {
            0x001200a9
        };
        entry.grfAccessMode = GRANT_ACCESS;
        entry.grfInheritance = if writable_directory {
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
        } else {
            NO_INHERITANCE
        };
        entry.Trustee = TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: PWSTR(sid.0.cast()),
            ..Default::default()
        };
        let mut granted = std::ptr::null_mut();
        let status = unsafe { SetEntriesInAclW(Some(&[entry]), Some(original), &mut granted) };
        if status.0 != 0 {
            unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
            return Err(io::Error::from_raw_os_error(status.0 as i32));
        }
        let mut grant = Self {
            file,
            descriptor,
            original,
            granted,
            applied: Vec::new(),
            restored: false,
        };
        let status = unsafe {
            SetSecurityInfo(
                HANDLE(grant.file.as_raw_handle()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(granted),
                None,
            )
        };
        if status.0 != 0 {
            return Err(io::Error::from_raw_os_error(status.0 as i32));
        }
        grant.applied = current_acl(&grant.file)?;
        Ok(grant)
    }

    fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        // 探针期间出现外部 ACL 变更时保留现场，不能把旧描述符盲写回用户文件。
        if current_acl(&self.file)? != self.applied {
            return Err(io::Error::other("版本探针 ACL 已被外部修改"));
        }
        let status = unsafe {
            SetSecurityInfo(
                HANDLE(self.file.as_raw_handle()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(self.original),
                None,
            )
        };
        if status.0 != 0 {
            return Err(io::Error::from_raw_os_error(status.0 as i32));
        }
        self.restored = true;
        Ok(())
    }
}

fn current_acl(file: &File) -> io::Result<Vec<u8>> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let mut acl = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut acl),
            None,
            Some(&mut descriptor),
        )
    };
    if status.0 != 0 {
        return Err(io::Error::from_raw_os_error(status.0 as i32));
    }
    let result = if acl.is_null() || !unsafe { IsValidAcl(acl) }.as_bool() {
        Err(io::Error::other("版本探针 ACL 无效"))
    } else {
        Ok(
            unsafe { std::slice::from_raw_parts(acl.cast::<u8>(), usize::from((*acl).AclSize)) }
                .to_vec(),
        )
    };
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    result
}

impl Drop for Grant {
    fn drop(&mut self) {
        // 析构不猜测进程是否退出；恢复失败必须由拥有者作为清理失败返回。
        unsafe { LocalFree(Some(HLOCAL(self.granted.cast()))) };
        unsafe { LocalFree(Some(HLOCAL(self.descriptor.0))) };
    }
}

pub(super) struct Attributes {
    storage: Vec<usize>,
    pub(super) list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl Attributes {
    pub(super) fn new(count: u32) -> io::Result<Self> {
        let mut length = 0;
        let _ = unsafe { InitializeProcThreadAttributeList(None, count, None, &mut length) };
        if length == 0 || length > 65536 {
            return Err(io::Error::other("版本探针启动属性大小无效"));
        }
        let mut storage = vec![0_usize; length.div_ceil(size_of::<usize>())];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        unsafe { InitializeProcThreadAttributeList(Some(list), count, None, &mut length) }
            .map_err(io::Error::other)?;
        Ok(Self { storage, list })
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.list) };
        std::hint::black_box(&self.storage);
    }
}

// 第二段负责真正创建候选；只把三个标准流副本送入其句柄表。
struct InheritedStreams<'a> {
    parent: Option<BorrowedHandle<'a>>,
    handles: Vec<HANDLE>,
}

impl<'a> InheritedStreams<'a> {
    fn new(
        parent: Option<BorrowedHandle<'a>>,
        output: Option<&CapturedOutput>,
    ) -> io::Result<Self> {
        let original =
            [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(|stream| {
                match (stream, output) {
                    (STD_OUTPUT_HANDLE, Some(output)) => Ok(output.stdout_handle()),
                    (STD_ERROR_HANDLE, Some(output)) => Ok(output.stderr_handle()),
                    _ => unsafe { GetStdHandle(stream) }.map_err(io::Error::other),
                }
            });
        let [input, output, error] = original;
        Self::from_handles(parent, [input?, output?, error?])
    }

    fn from_handles(
        parent: Option<BorrowedHandle<'a>>,
        mut original: [HANDLE; 3],
    ) -> io::Result<Self> {
        // 私有站版本候选没有输入；独立 EOF 不能关闭监督者用于控制生命周期的 stdin。
        // 原管道两端均不可继承，唯一写端在复制读端之前释放。
        let input = if parent.is_some() {
            let mut read = HANDLE::default();
            let mut write = HANDLE::default();
            unsafe { CreatePipe(&mut read, &mut write, None, 0) }.map_err(io::Error::other)?;
            let read = owned(read);
            drop(owned(write));
            original[0] = handle(&read);
            Some(read)
        } else {
            None
        };
        let mut result = Self {
            parent,
            handles: Vec::new(),
        };
        for original in original {
            let mut duplicate = HANDLE::default();
            unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    original,
                    result.parent_handle(),
                    &mut duplicate,
                    0,
                    true,
                    DUPLICATE_SAME_ACCESS,
                )
            }
            .map_err(io::Error::other)?;
            result.handles.push(duplicate);
        }
        drop(input);
        Ok(result)
    }

    fn parent_handle(&self) -> HANDLE {
        self.parent
            .map(|parent| HANDLE(parent.as_raw_handle()))
            .unwrap_or_else(|| unsafe { GetCurrentProcess() })
    }

    fn close(&mut self) -> io::Result<()> {
        let parent = self.parent_handle();
        let mut failure = None;
        for source in self.handles.drain(..) {
            let mut local = HANDLE::default();
            // CLOSE_SOURCE 即使返回失败也会关闭源值；每个远端值只能消费一次。
            let result = unsafe {
                DuplicateHandle(
                    parent,
                    source,
                    GetCurrentProcess(),
                    &mut local,
                    0,
                    false,
                    DUPLICATE_CLOSE_SOURCE | DUPLICATE_SAME_ACCESS,
                )
            };
            match result {
                Ok(()) => drop(owned(local)),
                Err(error) => {
                    failure.get_or_insert_with(|| io::Error::other(error));
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for InheritedStreams<'_> {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

enum ProbeCommand<'a> {
    Fixed {
        arguments: Option<&'a OsStr>,
        environment: &'a [(OsString, OsString)],
    },
    Mapped(Box<dyn FnOnce(&Path) -> io::Result<(OsString, Vec<(OsString, OsString)>)> + 'a>),
}

#[path = "windows_appcontainer_desktop.rs"]
pub(super) mod desktop;

#[cfg(feature = "native-probe-witness")]
#[path = "windows_appcontainer_witness.rs"]
mod witness;
#[cfg(feature = "native-probe-witness")]
pub use witness::NativeWitnessProcess;

/// 除固定 --version 外不接受其他参数；进程在 token 与严格 Job 核对前始终挂起。
pub struct AppContainerProbe {
    profile_name: Vec<u16>,
    sid: PSID,
    grants: Vec<Grant>,
    job: OwnedHandle,
    process: Option<OwnedHandle>,
    confirmed_exit_code: Option<u32>,
    startup_failure: Option<io::Error>,
    thread: Option<OwnedHandle>,
    process_id: u32,
    cleaned: bool,
    private_station: Option<PrivateStation>,
    profile_directories: Option<NativeProfileDirectories>,
    captured_output: Option<CapturedOutput>,
    #[cfg(any(test, feature = "test-util"))]
    private_desktop: Option<desktop::ProbeDesktop>,
}

enum ProbeDesktopMode<'a> {
    Inherited,
    NewLogon(
        &'a StationBootstrapImage,
        &'a mut dyn FnMut(ProbeJobOperation, BorrowedHandle<'_>) -> io::Result<()>,
        &'a Path,
    ),
    #[cfg(any(test, feature = "test-util"))]
    Private,
    #[cfg(any(test, feature = "test-util"))]
    ExistingStationPrivate,
}

enum ProbeConsoleMode {
    NoWindow,
    HiddenNewConsole,
}

impl ProbeConsoleMode {
    fn configure_startup(self, startup: &mut STARTUPINFOW) -> PROCESS_CREATION_FLAGS {
        match self {
            Self::NoWindow => CREATE_NO_WINDOW,
            Self::HiddenNewConsole => {
                // 隐藏独立控制台，不替换调用方绑定的三个标准流句柄。
                startup.dwFlags |= STARTF_USESHOWWINDOW;
                startup.wShowWindow = SW_HIDE.0 as u16;
                CREATE_NEW_CONSOLE
            }
        }
    }
}

impl AppContainerProbe {
    pub fn spawn_suspended(
        program: &Path,
        cwd: &Path,
        environment: &[(OsString, OsString)],
        name: &str,
    ) -> io::Result<Self> {
        Self::spawn_internal(
            program,
            ProbeCommand::Fixed {
                arguments: None,
                environment,
            },
            cwd,
            cwd,
            name,
            None,
            ProbeConsoleMode::NoWindow,
            ProbeDesktopMode::Inherited,
        )
    }

    /// 调用方绑定固定公共包装入口；候选树只读，仍先核对零 capability 与严格 Job。
    pub fn spawn_package_suspended(
        program: &Path,
        arguments: &std::ffi::OsStr,
        cwd: &Path,
        environment: &[(OsString, OsString)],
        name: &str,
        readonly: &[std::path::PathBuf],
    ) -> io::Result<Self> {
        Self::spawn_package_suspended_with_execution_cwd(
            program,
            arguments,
            cwd,
            cwd,
            environment,
            name,
            readonly,
        )
    }

    /// ACL 仍绑定原规范目录；仅允许给进程传入指向同一目录的另一种路径表示。
    pub fn spawn_package_suspended_with_execution_cwd(
        program: &Path,
        arguments: &std::ffi::OsStr,
        cwd: &Path,
        execution_cwd: &Path,
        environment: &[(OsString, OsString)],
        name: &str,
        readonly: &[std::path::PathBuf],
    ) -> io::Result<Self> {
        Self::spawn_package_internal(
            program,
            arguments,
            cwd,
            execution_cwd,
            environment,
            name,
            readonly,
            ProbeConsoleMode::NoWindow,
            ProbeDesktopMode::Inherited,
        )
    }

    /// 显式选择隐藏独立控制台；调用方仍须绑定精确控制台宿主并核验完整调试进程树。
    pub fn spawn_package_suspended_with_hidden_console(
        program: &Path,
        arguments: &std::ffi::OsStr,
        cwd: &Path,
        execution_cwd: &Path,
        environment: &[(OsString, OsString)],
        name: &str,
        readonly: &[std::path::PathBuf],
    ) -> io::Result<Self> {
        Self::spawn_package_internal(
            program,
            arguments,
            cwd,
            execution_cwd,
            environment,
            name,
            readonly,
            ProbeConsoleMode::HiddenNewConsole,
            ProbeDesktopMode::Inherited,
        )
    }

    /// 新站由精确绑定的两段 helper 持有；无输入版本候选从独立管道立即获得 EOF。
    /// 仅向本轮 AppContainer SID 授权，监督者的标准输入与取消生命周期保持不变。
    pub fn spawn_package_suspended_with_station(
        program: &Path,
        cwd: &Path,
        execution_cwd: &Path,
        name: &str,
        readonly: &[std::path::PathBuf],
        bootstrap: &StationBootstrapImage,
        output_directory: &Path,
        mut authorize: impl FnMut(ProbeJobOperation, BorrowedHandle<'_>) -> io::Result<()>,
        prepare_command: impl FnOnce(&Path) -> io::Result<(OsString, Vec<(OsString, OsString)>)>,
    ) -> io::Result<Self> {
        Self::verify_package_paths(cwd, execution_cwd, readonly)?;
        Self::spawn_internal(
            program,
            ProbeCommand::Mapped(Box::new(prepare_command)),
            cwd,
            execution_cwd,
            name,
            Some(readonly),
            ProbeConsoleMode::NoWindow,
            ProbeDesktopMode::NewLogon(bootstrap, &mut authorize, output_directory),
        )
    }

    /// 仅供独立测试 driver 的固定 Node 根 --version 对照；不接受任意桌面名称。
    #[cfg(any(test, feature = "test-util"))]
    pub fn spawn_package_suspended_with_private_desktop(
        program: &Path,
        cwd: &Path,
        execution_cwd: &Path,
        environment: &[(OsString, OsString)],
        name: &str,
        readonly: &[std::path::PathBuf],
    ) -> io::Result<Self> {
        Self::spawn_package_internal(
            program,
            std::ffi::OsStr::new("--version"),
            cwd,
            execution_cwd,
            environment,
            name,
            readonly,
            ProbeConsoleMode::NoWindow,
            ProbeDesktopMode::Private,
        )
    }

    /// 测试专用：只在调用方已有的非交互窗口站中创建本轮桌面，不修改该窗口站。
    #[cfg(any(test, feature = "test-util"))]
    pub fn spawn_package_suspended_with_existing_station_desktop(
        program: &Path,
        cwd: &Path,
        execution_cwd: &Path,
        environment: &[(OsString, OsString)],
        name: &str,
        readonly: &[std::path::PathBuf],
    ) -> io::Result<Self> {
        Self::spawn_package_internal(
            program,
            std::ffi::OsStr::new("--version"),
            cwd,
            execution_cwd,
            environment,
            name,
            readonly,
            ProbeConsoleMode::NoWindow,
            ProbeDesktopMode::ExistingStationPrivate,
        )
    }

    /// 只读复核本次持有的私有对象；不重新打开名称，也不修改既有对象权限。
    #[cfg(any(test, feature = "test-util"))]
    pub fn verify_private_desktop(&self) -> io::Result<()> {
        self.private_desktop
            .as_ref()
            .ok_or_else(|| io::Error::other("版本探针未选择私有桌面"))?
            .verify()
    }

    fn spawn_package_internal(
        program: &Path,
        arguments: &std::ffi::OsStr,
        cwd: &Path,
        execution_cwd: &Path,
        environment: &[(OsString, OsString)],
        name: &str,
        readonly: &[std::path::PathBuf],
        console_mode: ProbeConsoleMode,
        desktop_mode: ProbeDesktopMode<'_>,
    ) -> io::Result<Self> {
        Self::verify_package_paths(cwd, execution_cwd, readonly)?;
        Self::spawn_internal(
            program,
            ProbeCommand::Fixed {
                arguments: Some(arguments),
                environment,
            },
            cwd,
            execution_cwd,
            name,
            Some(readonly),
            console_mode,
            desktop_mode,
        )
    }

    fn verify_package_paths(
        cwd: &Path,
        execution_cwd: &Path,
        readonly: &[PathBuf],
    ) -> io::Result<()> {
        if !execution_cwd.is_absolute() || execution_cwd.canonicalize()? != cwd {
            return Err(io::Error::other("版本探针执行目录不匹配"));
        }
        if readonly.is_empty()
            || readonly.len() > 128
            || readonly
                .iter()
                .any(|path| !path.starts_with(cwd) || path == cwd)
        {
            return Err(io::Error::other("包探针的只读对象范围无效"));
        }
        Ok(())
    }

    fn spawn_internal(
        program: &Path,
        command: ProbeCommand<'_>,
        cwd: &Path,
        execution_cwd: &Path,
        name: &str,
        readonly: Option<&[std::path::PathBuf]>,
        console_mode: ProbeConsoleMode,
        desktop_mode: ProbeDesktopMode<'_>,
    ) -> io::Result<Self> {
        if !name.starts_with("InfiniShell.Version.")
            || name.len() != "InfiniShell.Version.".len() + 36
            || !name["InfiniShell.Version.".len()..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        {
            return Err(io::Error::other("版本探针 profile 名无效"));
        }
        let profile_name = wide(name.as_ref())?;
        profile_without_impersonation()?;
        let sid = unsafe {
            CreateAppContainerProfile(
                PCWSTR(profile_name.as_ptr()),
                PCWSTR(profile_name.as_ptr()),
                PCWSTR(profile_name.as_ptr()),
                None,
            )
        }
        .map_err(io::Error::other)?;
        let job_result = unsafe { CreateJobObjectW(None, None) }.map_err(io::Error::other);
        let job = match job_result {
            Ok(job) => owned(job),
            Err(error) => {
                unsafe { FreeSid(sid) };
                let _ = unsafe { DeleteAppContainerProfile(PCWSTR(profile_name.as_ptr())) };
                return Err(error);
            }
        };
        let mut result = Self {
            profile_name,
            sid,
            grants: Vec::new(),
            job,
            process: None,
            confirmed_exit_code: None,
            startup_failure: None,
            thread: None,
            process_id: 0,
            cleaned: false,
            private_station: None,
            profile_directories: None,
            captured_output: None,
            #[cfg(any(test, feature = "test-util"))]
            private_desktop: None,
        };
        // 只在 fresh profile 创建成功后查询本轮 SID，失败仍由 result 的原清理路径回收。
        result.profile_directories = Some(NativeProfileDirectories::capture(result.sid)?);
        let mut authorize = None;
        match desktop_mode {
            ProbeDesktopMode::Inherited => {}
            ProbeDesktopMode::NewLogon(image, bind_job, output_directory) => {
                result.captured_output = Some(CapturedOutput::create(output_directory)?);
                result.private_station = Some(PrivateStation::create(
                    image, cwd, name, result.sid, bind_job,
                )?);
                authorize = Some(bind_job);
            }
            #[cfg(any(test, feature = "test-util"))]
            ProbeDesktopMode::Private => {
                result.private_desktop = Some(desktop::ProbeDesktop::NewStation(
                    desktop::PrivateDesktop::create(name, result.sid)?,
                ));
            }
            #[cfg(any(test, feature = "test-util"))]
            ProbeDesktopMode::ExistingStationPrivate => {
                match desktop::ExistingStationDesktop::create(name, result.sid) {
                    Ok(private) => {
                        result.private_desktop =
                            Some(desktop::ProbeDesktop::ExistingStation(private));
                    }
                    Err(failure) => {
                        // 根进程尚未创建；显式删除本轮 profile 后才报告环境建立失败。
                        let cleanup = result.cleanup();
                        eprintln!(
                            "atomic_windows_existing_station_setup={{\"profile_cleanup_confirmed\":{}}}",
                            cleanup.is_ok()
                        );
                        return Err(match cleanup {
                            Ok(()) => failure,
                            Err(_) => io::Error::other(format!(
                                "{failure}；AppContainer profile 清理未确认"
                            )),
                        });
                    }
                }
            }
        }
        let (arguments, environment) = match command {
            ProbeCommand::Fixed {
                arguments,
                environment,
            } => (
                Cow::Borrowed(arguments.unwrap_or_else(|| OsStr::new("--version"))),
                Cow::Borrowed(environment),
            ),
            ProbeCommand::Mapped(prepare) => {
                let station = result
                    .private_station
                    .as_ref()
                    .ok_or_else(|| io::Error::other("版本探针缺少私有路径映射"))?;
                let (arguments, environment) = prepare(station.mapped_root()?)?;
                (Cow::Owned(arguments), Cow::Owned(environment))
            }
        };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                handle(&result.job),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const c_void,
                size_of_val(&limits) as u32,
            )
        }
        .map_err(io::Error::other)?;
        if let Some(paths) = readonly {
            // 系统 shell 使用系统原有 ACL，绝不写 System32；私有树逐对象授予读权限。
            result.grants.push(Grant::new(cwd, result.sid, false)?);
            for path in paths {
                result.grants.push(Grant::new(path, result.sid, false)?);
            }
        } else {
            result.grants.push(Grant::new(program, result.sid, false)?);
            result.grants.push(Grant::new(cwd, result.sid, true)?);
        }
        for relative in ["home", "config", "cache", "data", "tmp"] {
            result
                .grants
                .push(Grant::new(&cwd.join(relative), result.sid, true)?);
        }
        let mut capabilities = SECURITY_CAPABILITIES {
            AppContainerSid: result.sid,
            Capabilities: std::ptr::null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        };
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        if let Some(station) = &mut result.private_station {
            station.configure_startup(&mut startup.StartupInfo)?;
        }
        #[cfg(any(test, feature = "test-util"))]
        if let Some(desktop) = &mut result.private_desktop {
            desktop.configure_startup(&mut startup.StartupInfo)?;
        }
        let parent = result
            .private_station
            .as_ref()
            .map(PrivateStation::parent_process)
            .transpose()?;
        let mut streams = InheritedStreams::new(parent, result.captured_output.as_ref())?;
        let mut handles = streams.handles.clone();
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
        let console_flags = console_mode.configure_startup(&mut startup.StartupInfo);
        let program_wide = wide(program.as_os_str())?;
        if program_wide.contains(&(b'"' as u16)) {
            return Err(io::Error::other("版本探针路径含引号"));
        }
        let mut command = vec![b'"' as u16];
        command.extend_from_slice(&program_wide[..program_wide.len() - 1]);
        command.extend("\" ".encode_utf16());
        let arguments = wide(&arguments)?;
        command.extend_from_slice(&arguments);
        // Grant 已持有规范目录的句柄；重验别名后才交给 CreateProcessW，禁止切换目录。
        if !execution_cwd.is_absolute() || execution_cwd.canonicalize()? != cwd {
            return Err(io::Error::other("版本探针执行目录不匹配"));
        }
        let cwd_wide = wide(execution_cwd.as_os_str())?;
        let profile = result
            .profile_directories
            .as_ref()
            .ok_or_else(|| io::Error::other("版本探针原生 profile 绑定缺失"))?;
        profile.verify(result.sid)?;
        let mut environment = profile_environment(&environment, profile.base.as_os_str())?;
        environment.sort_by_key(|(key, _)| key.to_string_lossy().to_ascii_uppercase());
        let mut block = Vec::new();
        for (key, value) in environment {
            if key.as_encoded_bytes().contains(&b'=') {
                return Err(io::Error::other("版本探针环境名无效"));
            }
            let key = wide(&key)?;
            block.extend_from_slice(&key[..key.len() - 1]);
            block.push(b'=' as u16);
            block.extend(wide(&value)?);
        }
        block.push(0);
        let spawned = if let Some(station) = &result.private_station {
            let (request, image) = StationSpawn::new(
                program,
                command,
                block,
                execution_cwd,
                [handles[0], handles[1], handles[2]],
                console_flags == CREATE_NEW_CONSOLE,
            )?;
            image.verify()?;
            station.spawn_package(request)
        } else {
            let attributes = Attributes::new(2)?;
            unsafe {
                UpdateProcThreadAttribute(
                    attributes.list,
                    0,
                    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                    Some((&mut capabilities as *mut SECURITY_CAPABILITIES).cast()),
                    size_of_val(&capabilities),
                    None,
                    None,
                )
            }
            .map_err(io::Error::other)?;
            unsafe {
                UpdateProcThreadAttribute(
                    attributes.list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    Some(handles.as_mut_ptr().cast()),
                    handles.len() * size_of::<HANDLE>(),
                    None,
                    None,
                )
            }
            .map_err(io::Error::other)?;
            startup.lpAttributeList = attributes.list;
            let mut process = PROCESS_INFORMATION::default();
            unsafe {
                CreateProcessW(
                    PCWSTR(program_wide.as_ptr()),
                    Some(PWSTR(command.as_mut_ptr())),
                    None,
                    None,
                    true,
                    CREATE_SUSPENDED
                        | console_flags
                        | CREATE_UNICODE_ENVIRONMENT
                        | EXTENDED_STARTUPINFO_PRESENT
                        | DEBUG_PROCESS,
                    Some(block.as_ptr().cast()),
                    PCWSTR(cwd_wide.as_ptr()),
                    &startup.StartupInfo,
                    &mut process,
                )
            }
            .map_err(io::Error::other)
            .map(|()| process)
        };
        // 创建调用返回后才关闭远端标准流副本，候选仍只继承三个白名单句柄。
        let streams_closed = streams.close();
        drop(streams);
        let process = spawned?;
        result.process = Some(owned(process.hProcess));
        result.thread = Some(owned(process.hThread));
        result.process_id = process.dwProcessId;
        let binding = (|| {
            streams_closed?;
            if let Some(station) = &result.private_station {
                station.verify_package_identity(unsafe {
                    BorrowedHandle::borrow_raw(process.hProcess.0)
                })?;
            } else {
                let mut inherited = BOOL::default();
                unsafe { IsProcessInJob(process.hProcess, None, &mut inherited) }
                    .map_err(io::Error::other)?;
                if !inherited.as_bool() {
                    return Err(io::Error::other("版本探针未继承托管 Job"));
                }
            }
            // 挂起期间加入候选子 Job，并由唯一持原外层 Job 的监督者核验真实成员关系。
            unsafe { AssignProcessToJobObject(handle(&result.job), process.hProcess) }
                .map_err(io::Error::other)?;
            if let Some(authorize) = &mut authorize {
                authorize(ProbeJobOperation::VerifyCandidate, unsafe {
                    BorrowedHandle::borrow_raw(process.hProcess.0)
                })?;
            }
            result.verify_token()
        })();
        if let Err(failure) = binding {
            if result.private_station.is_none() {
                return Err(failure);
            }
            // 原调试线程必须拿到进程后才能终止并排空 CREATE/EXIT；拒绝恢复但保留所有权。
            result.startup_failure = Some(failure);
        }
        Ok(result)
    }

    /// 只接受本次精确 Job 中、具有同一零 capability AppContainer 身份的调试进程。
    pub fn verify_package_process(&self, process: BorrowedHandle<'_>) -> io::Result<()> {
        if !self.contains_package_process(process)? {
            return Err(io::Error::other("版本探针子进程不在本次 Job"));
        }
        self.verify_token_for(HANDLE(process.as_raw_handle()))
    }

    /// 调试器只借用实际事件句柄；不得通过 PID 重开而引入身份复用窗口。
    pub fn contains_package_process(&self, process: BorrowedHandle<'_>) -> io::Result<bool> {
        let mut contained = BOOL::default();
        unsafe {
            IsProcessInJob(
                HANDLE(process.as_raw_handle()),
                Some(handle(&self.job)),
                &mut contained,
            )
        }
        .map_err(io::Error::other)?;
        Ok(contained.as_bool())
    }

    fn verify_token(&self) -> io::Result<()> {
        let process = handle(
            self.process
                .as_ref()
                .ok_or_else(|| io::Error::other("版本探针进程缺失"))?,
        );
        self.verify_token_for(process)
    }

    fn verify_token_for(&self, process: HANDLE) -> io::Result<()> {
        if let Some(station) = &self.private_station {
            station.verify_package_identity(unsafe { BorrowedHandle::borrow_raw(process.0) })?;
        }
        let mut token = HANDLE::default();
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(io::Error::other)?;
        let token = owned(token);
        let mut length = 0_u32;
        let mut contained = 0_u32;
        unsafe {
            GetTokenInformation(
                handle(&token),
                TokenIsAppContainer,
                Some(&mut contained as *mut _ as *mut c_void),
                4,
                &mut length,
            )
        }
        .map_err(io::Error::other)?;
        if contained != 1 {
            return Err(io::Error::other("版本探针不是 AppContainer"));
        }
        let mut buffer = vec![0_usize; 1024];
        unsafe {
            GetTokenInformation(
                handle(&token),
                TokenCapabilities,
                Some(buffer.as_mut_ptr().cast()),
                (buffer.len() * size_of::<usize>()) as u32,
                &mut length,
            )
        }
        .map_err(io::Error::other)?;
        if unsafe { *buffer.as_ptr().cast::<u32>() } != 0 {
            return Err(io::Error::other("版本探针携带 capability"));
        }
        unsafe {
            GetTokenInformation(
                handle(&token),
                TokenAppContainerSid,
                Some(buffer.as_mut_ptr().cast()),
                (buffer.len() * size_of::<usize>()) as u32,
                &mut length,
            )
        }
        .map_err(io::Error::other)?;
        let actual = unsafe { &*buffer.as_ptr().cast::<TOKEN_APPCONTAINER_INFORMATION>() };
        unsafe { EqualSid(actual.TokenAppContainer, self.sid) }.map_err(io::Error::other)?;
        Ok(())
    }

    pub fn station_debugger(&self) -> Option<StationDebugger> {
        self.private_station
            .as_ref()
            .and_then(PrivateStation::debugger_handle)
    }

    pub fn id(&self) -> u32 {
        self.process_id
    }

    pub fn resume(&mut self) -> io::Result<()> {
        if let Some(failure) = &self.startup_failure {
            return Err(match failure.raw_os_error() {
                Some(code) => io::Error::from_raw_os_error(code),
                None => io::Error::new(failure.kind(), failure.to_string()),
            });
        }
        self.verify_token()?;
        let thread = self
            .thread
            .take()
            .ok_or_else(|| io::Error::other("版本探针已恢复"))?;
        if let Some(station) = &self.private_station {
            return station.resume_package();
        }
        if unsafe { ResumeThread(handle(&thread)) } != 1 {
            return Err(io::Error::other("版本探针挂起计数不匹配"));
        }
        Ok(())
    }

    pub fn exit_code(&self) -> io::Result<u32> {
        if let Some(code) = self.confirmed_exit_code {
            return Ok(code);
        }
        let mut code = 0;
        unsafe {
            GetExitCodeProcess(
                handle(
                    self.process
                        .as_ref()
                        .ok_or_else(|| io::Error::other("版本探针进程缺失"))?,
                ),
                &mut code,
            )
        }
        .map_err(io::Error::other)?;
        if code == 259 {
            return Err(io::Error::other("版本探针仍在运行"));
        }
        Ok(code)
    }

    /// 原根可能在身份核验失败前尚未入子 Job；原句柄和严格 Job 都必须终止。
    pub fn terminate_job(&self) -> io::Result<()> {
        let root = if let Some(process) = &self.process {
            match unsafe { WaitForSingleObject(handle(process), 0) } {
                WAIT_OBJECT_0 => Ok(()),
                WAIT_TIMEOUT => {
                    unsafe { TerminateProcess(handle(process), 1) }.map_err(io::Error::other)
                }
                _ => Err(io::Error::last_os_error()),
            }
        } else {
            Ok(())
        };
        let job = unsafe { TerminateJobObject(handle(&self.job), 1) }.map_err(io::Error::other);
        root.and(job)
    }

    fn processes_terminated(&self) -> io::Result<bool> {
        if let Some(process) = &self.process {
            // EXIT 调试事件尚未继续时也可能已有退出码；必须另证原句柄已发出退出信号。
            let state = unsafe { WaitForSingleObject(handle(process), 0) };
            if state == WAIT_TIMEOUT {
                return Ok(false);
            }
            if state != WAIT_OBJECT_0 {
                return Err(io::Error::last_os_error());
            }
        }
        let mut state = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(handle(&self.job)),
                JobObjectBasicAccountingInformation,
                &mut state as *mut _ as *mut c_void,
                size_of_val(&state) as u32,
                None,
            )
        }
        .map_err(io::Error::other)?;
        Ok(state.ActiveProcesses == 0)
    }

    pub fn cleanup(&mut self) -> io::Result<()> {
        if self.cleaned {
            return Ok(());
        }
        if !self.processes_terminated()? {
            return Err(io::Error::other("版本探针进程或 Job 尚未确认退出"));
        }
        if self.process.is_some() {
            self.confirmed_exit_code = Some(self.exit_code()?);
        }
        // 新 LUID 的所有候选对象句柄先释放，之后才允许建站 helper 等待登录会话消失。
        self.thread = None;
        self.process = None;
        // 异常收尾也只在原进程和 Job 已退出后释放候选的普通文件输出对象。
        self.captured_output = None;
        if let Some(station) = &mut self.private_station {
            // CLI 与其严格 Job 已退出，才允许 helper 释放本轮站与新登录会话。
            station.close()?;
            self.private_station = None;
        }
        #[cfg(any(test, feature = "test-util"))]
        if let Some(desktop) = &mut self.private_desktop {
            // 只有进程与严格 Job 均已确认退出，才关闭本次私有对象。
            desktop.close()?;
            self.private_desktop = None;
        }
        for grant in self.grants.iter_mut().rev() {
            grant.restore()?;
        }
        self.grants.clear();
        // 存储目录句柄会阻止完整删除；仅在原进程、Job、站和 ACL 清理完成后释放。
        self.profile_directories = None;
        unsafe { DeleteAppContainerProfile(PCWSTR(self.profile_name.as_ptr())) }
            .map_err(io::Error::other)?;
        self.cleaned = true;
        Ok(())
    }

    /// 固定版本候选的输出先转存为仅 worker 持有的文件，完成原生清理后才写回原流。
    pub fn write_cleanup_receipt_with_output(
        &mut self,
        path: &Path,
        stdout: &mut impl io::Write,
        stderr: &mut impl io::Write,
        mut check_cancelled: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.wait_for_processes_terminated()?;
        let output = self
            .captured_output
            .take()
            .ok_or_else(|| io::Error::other("版本探针私有输出缺失"))
            .and_then(|output| output.seal(&mut check_cancelled));
        // 转存/取消失败不得跳过原生清理；对外写入即使阻塞也已不再持有新 LUID 的输出对象。
        let cleanup = self.write_cleanup_receipt(path);
        let replay = output.and_then(|output| output.replay(stdout, stderr, &mut check_cancelled));
        cleanup.and(replay)
    }

    pub fn write_cleanup_receipt(&mut self, path: &Path) -> io::Result<()> {
        self.wait_for_processes_terminated()?;
        self.cleanup()?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n")?;
        file.sync_all()
    }

    fn wait_for_processes_terminated(&self) -> io::Result<()> {
        // ContinueDebugEvent 返回后，内核退出信号可能稍后到达。只等待进程/Job 暂态；
        // ACL 外部修改、profile 删除和文件写入失败均保持原错误，不在这里重试。
        let deadline = Instant::now() + Duration::from_secs(3);
        while !self.processes_terminated()? {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "版本探针退出确认超时",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

impl Drop for AppContainerProbe {
    fn drop(&mut self) {
        if !self.cleaned {
            // 创建后加入 Job 失败时，仍通过已持有的进程句柄终止本次根进程。
            if let Some(process) = &self.process {
                let _ = unsafe { TerminateProcess(handle(process), 1) };
            }
            let _ = unsafe { TerminateJobObject(handle(&self.job), 1) };
            let deadline = Instant::now() + Duration::from_secs(3);
            while matches!(self.processes_terminated(), Ok(false)) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            // 正常 close 的错误仍返回调用方；析构只在业务进程确证退出后收敛辅助资源。
            if self.processes_terminated().unwrap_or(false) {
                self.confirmed_exit_code = self.exit_code().ok();
                self.thread = None;
                self.process = None;
                self.captured_output = None;
                let station_reaped = match &mut self.private_station {
                    Some(station) => station.abort_and_reap().is_ok(),
                    None => true,
                };
                if station_reaped {
                    self.private_station = None;
                    // helper Job、站和 LSA 均已释放，才恢复文件 ACL 并删除本轮 profile。
                    // 此路径不调用 write_cleanup_receipt，不能将失败事务标为成功。
                    while self.cleanup().is_err() && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        }
        unsafe { FreeSid(self.sid) };
    }
}

#[cfg(test)]
#[path = "windows_appcontainer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "windows_appcontainer_console_tests.rs"]
mod console_tests;
