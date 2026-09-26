//! 无网络 capability 的一次性版本探针；所有派生留在严格 Job，句柄继承仅限标准流。

use std::ffi::{OsString, c_void};
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, HLOCAL, LocalFree,
};
use windows::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetSecurityInfo, SE_FILE_OBJECT, SetEntriesInAclW,
    SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows::Win32::Security::Isolation::{CreateAppContainerProfile, DeleteAppContainerProfile};
use windows::Win32::Security::{
    ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, FreeSid, GetTokenInformation,
    IsValidAcl, NO_INHERITANCE, OBJECT_INHERIT_ACE, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_CAPABILITIES, TOKEN_APPCONTAINER_INFORMATION, TOKEN_QUERY, TokenAppContainerSid,
    TokenCapabilities, TokenIsAppContainer,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    READ_CONTROL, WRITE_DAC,
};
use windows::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DEBUG_PROCESS,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    OpenProcessToken, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ResumeThread,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

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

struct Attributes {
    storage: Vec<usize>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl Attributes {
    fn new() -> io::Result<Self> {
        let mut length = 0;
        let _ = unsafe { InitializeProcThreadAttributeList(None, 2, None, &mut length) };
        if length == 0 || length > 65536 {
            return Err(io::Error::other("版本探针启动属性大小无效"));
        }
        let mut storage = vec![0_usize; length.div_ceil(size_of::<usize>())];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        unsafe { InitializeProcThreadAttributeList(Some(list), 2, None, &mut length) }
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

/// 除固定 --version 外不接受其他参数；进程在 token 与严格 Job 核对前始终挂起。
pub struct AppContainerProbe {
    profile_name: Vec<u16>,
    sid: PSID,
    grants: Vec<Grant>,
    job: OwnedHandle,
    process: Option<OwnedHandle>,
    thread: Option<OwnedHandle>,
    process_id: u32,
    cleaned: bool,
}

impl AppContainerProbe {
    pub fn spawn_suspended(
        program: &Path,
        cwd: &Path,
        environment: &[(OsString, OsString)],
        name: &str,
    ) -> io::Result<Self> {
        Self::spawn_internal(program, None, cwd, environment, name, None)
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
        if readonly.is_empty()
            || readonly.len() > 128
            || readonly
                .iter()
                .any(|path| !path.starts_with(cwd) || path == cwd)
        {
            return Err(io::Error::other("包探针的只读对象范围无效"));
        }
        Self::spawn_internal(
            program,
            Some(arguments),
            cwd,
            environment,
            name,
            Some(readonly),
        )
    }

    fn spawn_internal(
        program: &Path,
        arguments: Option<&std::ffi::OsStr>,
        cwd: &Path,
        environment: &[(OsString, OsString)],
        name: &str,
        readonly: Option<&[std::path::PathBuf]>,
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
            thread: None,
            process_id: 0,
            cleaned: false,
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
        let attributes = Attributes::new()?;
        unsafe {
            UpdateProcThreadAttribute(
                attributes.list,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                Some(&mut capabilities as *mut _ as *const c_void),
                size_of_val(&capabilities),
                None,
                None,
            )
        }
        .map_err(io::Error::other)?;
        let mut streams = Vec::new();
        for stream in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let original = unsafe { GetStdHandle(stream) }.map_err(io::Error::other)?;
            let mut duplicate = HANDLE::default();
            unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    original,
                    GetCurrentProcess(),
                    &mut duplicate,
                    0,
                    true,
                    DUPLICATE_SAME_ACCESS,
                )
            }
            .map_err(io::Error::other)?;
            streams.push(owned(duplicate));
        }
        let mut handles: Vec<_> = streams.iter().map(handle).collect();
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
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
        startup.lpAttributeList = attributes.list;
        let program_wide = wide(program.as_os_str())?;
        if program_wide.contains(&(b'"' as u16)) {
            return Err(io::Error::other("版本探针路径含引号"));
        }
        let mut command = vec![b'"' as u16];
        command.extend_from_slice(&program_wide[..program_wide.len() - 1]);
        command.extend("\" ".encode_utf16());
        let arguments = wide(arguments.unwrap_or_else(|| std::ffi::OsStr::new("--version")))?;
        command.extend_from_slice(&arguments);
        let cwd_wide = wide(cwd.as_os_str())?;
        let mut environment = environment.to_vec();
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
        let mut process = PROCESS_INFORMATION::default();
        unsafe {
            CreateProcessW(
                PCWSTR(program_wide.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                true,
                CREATE_SUSPENDED
                    | CREATE_NO_WINDOW
                    | CREATE_UNICODE_ENVIRONMENT
                    | EXTENDED_STARTUPINFO_PRESENT
                    | DEBUG_PROCESS,
                Some(block.as_ptr().cast()),
                PCWSTR(cwd_wide.as_ptr()),
                &startup.StartupInfo,
                &mut process,
            )
        }
        .map_err(io::Error::other)?;
        result.process = Some(owned(process.hProcess));
        result.thread = Some(owned(process.hThread));
        result.process_id = process.dwProcessId;
        // 必须已继承外层托管 Job；随后加入无 breakaway 的本次探针子 Job。
        let mut inherited = BOOL::default();
        unsafe { IsProcessInJob(process.hProcess, None, &mut inherited) }
            .map_err(io::Error::other)?;
        if !inherited.as_bool() {
            return Err(io::Error::other("版本探针未继承托管 Job"));
        }
        unsafe { AssignProcessToJobObject(handle(&result.job), process.hProcess) }
            .map_err(io::Error::other)?;
        result.verify_token()?;
        Ok(result)
    }

    fn verify_token(&self) -> io::Result<()> {
        let process = handle(
            self.process
                .as_ref()
                .ok_or_else(|| io::Error::other("版本探针进程缺失"))?,
        );
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

    pub fn id(&self) -> u32 {
        self.process_id
    }

    pub fn resume(&mut self) -> io::Result<()> {
        self.verify_token()?;
        let thread = self
            .thread
            .take()
            .ok_or_else(|| io::Error::other("版本探针已恢复"))?;
        if unsafe { ResumeThread(handle(&thread)) } != 1 {
            return Err(io::Error::other("版本探针挂起计数不匹配"));
        }
        Ok(())
    }

    pub fn exit_code(&self) -> io::Result<u32> {
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

    pub fn cleanup(&mut self) -> io::Result<()> {
        if self.cleaned {
            return Ok(());
        }
        if self.process.is_some() {
            self.exit_code()?;
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
        if state.ActiveProcesses != 0 {
            return Err(io::Error::other("版本探针 Job 尚有进程"));
        }
        for grant in self.grants.iter_mut().rev() {
            grant.restore()?;
        }
        self.grants.clear();
        unsafe { DeleteAppContainerProfile(PCWSTR(self.profile_name.as_ptr())) }
            .map_err(io::Error::other)?;
        self.cleaned = true;
        Ok(())
    }

    pub fn write_cleanup_receipt(&mut self, path: &Path) -> io::Result<()> {
        self.cleanup()?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n")?;
        file.sync_all()
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
            while self.cleanup().is_err() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
        }
        unsafe { FreeSid(self.sid) };
    }
}
