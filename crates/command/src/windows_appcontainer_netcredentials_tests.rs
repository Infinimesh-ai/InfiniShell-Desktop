//! 非管理员服务身份的单次新登录会话候选；不代表普通交互用户产品或 G09 关闭。

use std::ffi::{OsStr, OsString, c_void};
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, IntoRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use windows::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_FILE_NOT_FOUND, ERROR_NO_TOKEN,
    FILETIME, HANDLE, LUID, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::Authentication::Identity::{
    LsaFreeReturnBuffer, LsaGetLogonSessionData,
};
use windows::Win32::Security::{
    ACL, ACL_SIZE_INFORMATION, AclSizeInformation, CheckTokenMembership, CreateWellKnownSid,
    DACL_SECURITY_INFORMATION, DuplicateToken, GetAce, GetAclInformation, GetLengthSid,
    GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, GetSidSubAuthority,
    GetSidSubAuthorityCount, GetTokenInformation, GetUserObjectSecurity, IsValidSecurityDescriptor,
    IsValidSid, LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    RevertToSelf, SECURITY_IMPERSONATION_LEVEL, SecurityIdentification, TOKEN_DUPLICATE,
    TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_STATISTICS,
    TOKEN_USER, TokenElevation, TokenImpersonationLevel, TokenIntegrityLevel, TokenSessionId,
    TokenStatistics, TokenUser, WinBuiltinAdministratorsSid,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_TYPE_PIPE, GetFileInformationByHandle,
    GetFileType, QueryDosDeviceW, READ_CONTROL,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Pipes::{ImpersonateNamedPipeClient, PeekNamedPipe};
use windows::Win32::System::StationsAndDesktops::{
    CloseWindowStation, GetProcessWindowStation, GetThreadDesktop, GetUserObjectInformationW,
    OpenWindowStationW, UOI_FLAGS, UOI_NAME, USEROBJECTFLAGS,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessWithLogonW,
    GetCurrentProcess, GetCurrentThread, GetCurrentThreadId, GetExitCodeProcess, GetProcessId,
    GetProcessTimes, LOGON_NETCREDENTIALS_ONLY, OpenProcess, OpenProcessToken, OpenThreadToken,
    PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, ResumeThread, STARTUPINFOW, TerminateProcess,
    WaitForSingleObject,
};
use windows::core::{BOOL, Error as WindowsError, HRESULT, PCWSTR, PWSTR};

type Result<T> = std::result::Result<T, Value>;
const PREFIX: &str = "INFINISHELL_WINDOWS_NETCREDENTIALS_";
const HEADER: usize = 44;
const OBJECT: usize = 16912;
const SD_LIMIT: usize = 16384;
const WAIT: Duration = Duration::from_secs(10);

fn failure(stage: &str, detail: &str) -> Value {
    json!({"stage":stage,"detail":detail})
}

fn api<T>(stage: &str, value: windows::core::Result<T>) -> Result<T> {
    value.map_err(|error| {
        let hresult = error.code().0 as u32;
        json!({"stage":stage,"hresult":hresult,
            "win32_from_hresult":if hresult & 0xffff0000 == 0x80070000 {Some(hresult & 0xffff)} else {None}})
    })
}

fn disk<T>(stage: &str, value: std::io::Result<T>) -> Result<T> {
    value.map_err(|error| json!({"stage":stage,"os_code":error.raw_os_error(),"kind":format!("{:?}",error.kind())}))
}

fn require(value: bool, stage: &str) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(failure(stage, "本轮精确合同不匹配"))
    }
}

fn raw(value: &OwnedHandle) -> HANDLE {
    HANDLE(value.as_raw_handle())
}

fn owned(value: HANDLE) -> OwnedHandle {
    unsafe { OwnedHandle::from_raw_handle(value.0) }
}

fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut result: Vec<_> = value.encode_wide().collect();
    require(!result.contains(&0), "wide_nul")?;
    result.push(0);
    Ok(result)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[repr(C)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct Process {
    pid: u32,
    created_low: u32,
    created_high: u32,
    auth_low: u32,
    auth_high: u32,
    session: u32,
    token_type: u32,
    elevated: u32,
    integrity: u32,
    sid_length: u32,
    sid: [u8; 68],
}

impl Process {
    fn json(&self) -> Value {
        json!({"pid":self.pid,"created_low":self.created_low,"created_high":self.created_high,
            "auth_low":self.auth_low,"auth_high":self.auth_high,"session":self.session,
            "token_type":self.token_type,"elevated":self.elevated,"integrity":self.integrity,
            "user_sid":hex(&self.sid[..self.sid_length as usize])})
    }

    fn same_local_token(&self, other: &Self) -> bool {
        self.sid == other.sid
            && self.sid_length == other.sid_length
            && self.session == other.session
            && self.token_type == other.token_type
            && self.elevated == other.elevated
            && self.integrity == other.integrity
    }

    fn same_logon(&self, other: &Self) -> bool {
        (self.auth_low, self.auth_high) == (other.auth_low, other.auth_high)
    }
}

fn token_field<T: Default>(token: HANDLE, kind: TOKEN_INFORMATION_CLASS) -> Result<T> {
    let mut value = T::default();
    let mut used = 0;
    api("token_field", unsafe {
        GetTokenInformation(
            token,
            kind,
            Some((&mut value as *mut T).cast()),
            size_of::<T>() as u32,
            &mut used,
        )
    })?;
    require(used as usize == size_of::<T>(), "token_field_size")?;
    Ok(value)
}

fn sid_bytes(sid: PSID) -> Result<Vec<u8>> {
    require(unsafe { IsValidSid(sid) }.as_bool(), "valid_sid")?;
    let length = unsafe { GetLengthSid(sid) } as usize;
    require((8..=68).contains(&length), "sid_size")?;
    Ok(unsafe { std::slice::from_raw_parts(sid.0.cast(), length) }.to_vec())
}

fn process_snapshot(process: HANDLE) -> Result<Process> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    api("process_times", unsafe {
        GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user)
    })?;
    let mut token = HANDLE::default();
    api("open_token", unsafe {
        OpenProcessToken(process, TOKEN_QUERY, &mut token)
    })?;
    let token = owned(token);
    let statistics: TOKEN_STATISTICS = token_field(raw(&token), TokenStatistics)?;
    let elevation: TOKEN_ELEVATION = token_field(raw(&token), TokenElevation)?;
    let mut result = Process {
        pid: unsafe { GetProcessId(process) },
        created_low: created.dwLowDateTime,
        created_high: created.dwHighDateTime,
        auth_low: statistics.AuthenticationId.LowPart,
        auth_high: statistics.AuthenticationId.HighPart as u32,
        session: token_field(raw(&token), TokenSessionId)?,
        token_type: statistics.TokenType.0 as u32,
        elevated: elevation.TokenIsElevated,
        integrity: 0,
        sid_length: 0,
        sid: [0; 68],
    };
    let mut buffer = [0usize; 128];
    let mut used = 0;
    api("token_user", unsafe {
        GetTokenInformation(
            raw(&token),
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            size_of_val(&buffer) as u32,
            &mut used,
        )
    })?;
    let sid = sid_bytes(unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid })?;
    result.sid_length = sid.len() as u32;
    result.sid[..sid.len()].copy_from_slice(&sid);
    api("token_integrity", unsafe {
        GetTokenInformation(
            raw(&token),
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr().cast()),
            size_of_val(&buffer) as u32,
            &mut used,
        )
    })?;
    let label = unsafe { (*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid };
    sid_bytes(label)?;
    let count = unsafe { *GetSidSubAuthorityCount(label) };
    require(count > 0, "integrity_sid")?;
    result.integrity = unsafe { *GetSidSubAuthority(label, u32::from(count - 1)) };
    require(result.pid > 0, "process_pid")?;
    Ok(result)
}

fn no_thread_token() -> Result<()> {
    let mut token = HANDLE::default();
    match unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) } {
        Ok(()) => {
            drop(owned(token));
            Err(failure("calling_thread", "不接受调用线程模拟令牌"))
        }
        Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_TOKEN.0) => Ok(()),
        Err(error) => api("calling_thread", Err(error)),
    }
}

fn administrator_memberships() -> Result<(bool, bool)> {
    let mut sid = [0u32; 17];
    let mut length = size_of_val(&sid) as u32;
    let sid = PSID(sid.as_mut_ptr().cast());
    api("administrator_sid", unsafe {
        CreateWellKnownSid(WinBuiltinAdministratorsSid, None, Some(sid), &mut length)
    })?;
    let mut calling_thread = BOOL::default();
    api("calling_thread_admin_membership", unsafe {
        CheckTokenMembership(None, sid, &mut calling_thread)
    })?;
    let mut primary = HANDLE::default();
    api("process_token_for_membership", unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE,
            &mut primary,
        )
    })?;
    let primary = owned(primary);
    let mut duplicate = HANDLE::default();
    api("identification_token", unsafe {
        DuplicateToken(raw(&primary), SecurityIdentification, &mut duplicate)
    })?;
    let duplicate = owned(duplicate);
    let mut process = BOOL::default();
    api("process_admin_membership", unsafe {
        CheckTokenMembership(Some(raw(&duplicate)), sid, &mut process)
    })?;
    Ok((calling_thread.as_bool(), process.as_bool()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Object {
    name: String,
    inherit: u32,
    flags: u32,
    sd: Vec<u8>,
}

impl Object {
    fn json(&self) -> Result<Value> {
        validate_descriptor_bounds(&self.sd)?;
        let mut words = vec![0u32; self.sd.len().div_ceil(4)];
        let storage = unsafe {
            std::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), words.len() * 4)
        };
        storage[..self.sd.len()].copy_from_slice(&self.sd);
        let descriptor = PSECURITY_DESCRIPTOR(words.as_mut_ptr().cast());
        require(
            unsafe { IsValidSecurityDescriptor(descriptor) }.as_bool(),
            "valid_descriptor",
        )?;
        let mut owner = PSID::default();
        let mut defaulted = BOOL::default();
        api("descriptor_owner", unsafe {
            GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted)
        })?;
        let owner = sid_bytes(owner)?;
        let mut present = BOOL::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        api("descriptor_dacl", unsafe {
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
        })?;
        let mut aces = Vec::new();
        if present.as_bool() && !dacl.is_null() {
            let mut info = ACL_SIZE_INFORMATION::default();
            api("acl_size", unsafe {
                GetAclInformation(
                    dacl,
                    (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                    size_of_val(&info) as u32,
                    AclSizeInformation,
                )
            })?;
            require(info.AceCount <= 1024, "acl_count")?;
            for index in 0..info.AceCount {
                let mut ace: *mut c_void = std::ptr::null_mut();
                api("acl_ace", unsafe { GetAce(dacl, index, &mut ace) })?;
                let start = ace as usize;
                let base = words.as_ptr() as usize;
                require(
                    start >= base
                        && start
                            .checked_add(4)
                            .is_some_and(|end| end <= base + self.sd.len()),
                    "ace_header_bounds",
                )?;
                let header = unsafe { std::slice::from_raw_parts(ace.cast::<u8>(), 4) };
                let length = usize::from(u16::from_le_bytes([header[2], header[3]]));
                require(
                    length >= 4
                        && start
                            .checked_add(length)
                            .is_some_and(|end| end <= base + self.sd.len()),
                    "ace_bounds",
                )?;
                aces.push(hex(unsafe {
                    std::slice::from_raw_parts(ace.cast::<u8>(), length)
                }));
            }
        }
        Ok(
            json!({"name":self.name,"inherit":self.inherit,"flags":self.flags,
            "owner_sid":hex(&owner),"dacl_present":present.as_bool(),"dacl_null":dacl.is_null(),
            "ace_bytes":aces,"descriptor_sha256":hex(&Sha256::digest(&self.sd)),"descriptor_bytes":self.sd.len()}),
        )
    }
}

fn validate_descriptor_bounds(bytes: &[u8]) -> Result<()> {
    require(
        bytes.len() >= 20
            && bytes[0] == 1
            && u16::from_le_bytes([bytes[2], bytes[3]]) & 0x8000 != 0,
        "relative_descriptor_header",
    )?;
    for offset in [4, 8] {
        let start = word(bytes, offset) as usize;
        if start == 0 {
            continue;
        }
        require(
            start >= 20 && start.checked_add(8).is_some_and(|end| end <= bytes.len()),
            "descriptor_sid_header",
        )?;
        let count = bytes[start + 1] as usize;
        require(
            count <= 15
                && start
                    .checked_add(8 + count * 4)
                    .is_some_and(|end| end <= bytes.len()),
            "descriptor_sid_bounds",
        )?;
    }
    require(word(bytes, 4) != 0, "descriptor_owner_present")?;
    for offset in [12, 16] {
        let start = word(bytes, offset) as usize;
        if start == 0 {
            continue;
        }
        require(
            start >= 20 && start.checked_add(8).is_some_and(|end| end <= bytes.len()),
            "descriptor_acl_header",
        )?;
        let length = usize::from(u16::from_le_bytes([bytes[start + 2], bytes[start + 3]]));
        require(
            length >= 8
                && start
                    .checked_add(length)
                    .is_some_and(|end| end <= bytes.len()),
            "descriptor_acl_bounds",
        )?;
        let count = usize::from(u16::from_le_bytes([bytes[start + 4], bytes[start + 5]]));
        require(count <= 1024, "descriptor_ace_count")?;
        let mut at = start + 8;
        for _ in 0..count {
            require(at + 4 <= start + length, "descriptor_ace_header")?;
            let size = usize::from(u16::from_le_bytes([bytes[at + 2], bytes[at + 3]]));
            require(
                size >= 4 && at + size <= start + length,
                "descriptor_ace_bounds",
            )?;
            at += size;
        }
    }
    Ok(())
}

fn object_snapshot(object: HANDLE) -> Result<Object> {
    let mut name = [0u16; 256];
    let mut used = 0;
    api("object_name", unsafe {
        GetUserObjectInformationW(
            object,
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            size_of_val(&name) as u32,
            Some(&mut used),
        )
    })?;
    require(
        (4..=512).contains(&used) && used % 2 == 0 && name[used as usize / 2 - 1] == 0,
        "object_name_size",
    )?;
    let name = String::from_utf16(&name[..used as usize / 2 - 1])
        .map_err(|_| failure("object_name", "UTF-16 无效"))?;
    let mut flags = USEROBJECTFLAGS::default();
    api("object_flags", unsafe {
        GetUserObjectInformationW(
            object,
            UOI_FLAGS,
            Some((&mut flags as *mut USEROBJECTFLAGS).cast()),
            size_of_val(&flags) as u32,
            Some(&mut used),
        )
    })?;
    require(used as usize == size_of_val(&flags), "object_flags_size")?;
    let mut words = vec![0u32; SD_LIMIT / 4];
    let requested =
        (OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION).0;
    api("object_sd", unsafe {
        GetUserObjectSecurity(
            object,
            &requested,
            Some(PSECURITY_DESCRIPTOR(words.as_mut_ptr().cast())),
            SD_LIMIT as u32,
            &mut used,
        )
    })?;
    require(used > 0 && used as usize <= SD_LIMIT, "object_sd_size")?;
    let sd =
        unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), used as usize) }.to_vec();
    let result = Object {
        name,
        inherit: flags.fInherit.0 as u32,
        flags: flags.dwFlags,
        sd,
    };
    result.json()?;
    Ok(result)
}

fn current_objects() -> Result<(Object, Object)> {
    let station = api("current_station", unsafe { GetProcessWindowStation() })?;
    let desktop = api("current_desktop", unsafe {
        GetThreadDesktop(GetCurrentThreadId())
    })?;
    Ok((
        object_snapshot(HANDLE(station.0))?,
        object_snapshot(HANDLE(desktop.0))?,
    ))
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn check_header(bytes: &[u8], nonce: &[u8], kind: u32, length: usize) -> Result<()> {
    require(
        bytes.len() == length
            && word(bytes, 0) == 0x4e435331
            && word(bytes, 4) == 1
            && word(bytes, 8) == kind
            && &bytes[12..HEADER] == nonce,
        "receipt_header",
    )
}

fn parse_process(bytes: &[u8]) -> Result<Process> {
    require(
        bytes.len() == size_of::<Process>() && size_of::<Process>() == 108,
        "process_receipt_size",
    )?;
    // 此结构只含整数和定长字节数组，所有位模式均有效；不读取任何原生指针。
    let result = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<Process>()) };
    require(
        result.pid > 0 && (8..=68).contains(&result.sid_length),
        "process_receipt_fields",
    )?;
    let sid = sid_bytes(PSID(result.sid.as_ptr().cast_mut().cast()))?;
    require(
        sid.len() == result.sid_length as usize,
        "process_receipt_sid",
    )?;
    Ok(result)
}

fn parse_object(bytes: &[u8]) -> Result<Object> {
    require(bytes.len() == OBJECT, "object_receipt_size")?;
    let name_bytes = word(bytes, 0) as usize;
    let sd_bytes = word(bytes, 524) as usize;
    require(
        (4..=512).contains(&name_bytes)
            && name_bytes % 2 == 0
            && (1..=SD_LIMIT).contains(&sd_bytes),
        "object_receipt_bounds",
    )?;
    let name: Vec<_> = bytes[4..4 + name_bytes]
        .chunks_exact(2)
        .map(|part| u16::from_le_bytes([part[0], part[1]]))
        .collect();
    require(
        name.last() == Some(&0) && !name[..name.len() - 1].contains(&0),
        "object_receipt_name",
    )?;
    let result = Object {
        name: String::from_utf16(&name[..name.len() - 1])
            .map_err(|_| failure("object_receipt_name", "UTF-16 无效"))?,
        inherit: word(bytes, 516),
        flags: word(bytes, 520),
        sd: bytes[528..528 + sd_bytes].to_vec(),
    };
    result.json()?;
    Ok(result)
}

fn observation(bytes: &[u8], nonce: &[u8], kind: u32) -> Result<(Process, Object, Object)> {
    check_header(bytes, nonce, kind, 33980)?;
    Ok((
        parse_process(&bytes[HEADER..152])?,
        parse_object(&bytes[156..156 + OBJECT])?,
        parse_object(&bytes[156 + OBJECT..])?,
    ))
}

fn read_receipt(path: &Path, maximum: usize) -> Result<Option<Vec<u8>>> {
    let opened = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path);
    let mut file = match opened {
        Ok(file) => file,
        Err(error) if matches!(error.raw_os_error(), Some(2 | 32)) => return Ok(None),
        Err(error) => return disk("open_receipt", Err(error)),
    };
    let metadata = disk("receipt_metadata", file.metadata())?;
    require(
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
            && metadata.is_file()
            && metadata.len() <= maximum as u64,
        "receipt_regular_file",
    )?;
    let mut bytes = Vec::new();
    disk(
        "read_receipt",
        (&mut file).take(maximum as u64 + 1).read_to_end(&mut bytes),
    )?;
    require(bytes.len() <= maximum, "receipt_limit")?;
    Ok(Some(bytes))
}

fn image_path(process: HANDLE) -> Result<PathBuf> {
    let mut name = [0u16; 1024];
    let mut length = name.len() as u32;
    api("process_image", unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(name.as_mut_ptr()),
            &mut length,
        )
    })?;
    require(
        length > 0 && (length as usize) < name.len(),
        "process_image_size",
    )?;
    disk(
        "process_image_canonical",
        PathBuf::from(OsString::from_wide(&name[..length as usize])).canonicalize(),
    )
}

fn resume(thread: HANDLE, stage: &str) -> Result<()> {
    match unsafe { ResumeThread(thread) } {
        1 => Ok(()),
        u32::MAX => api(stage, Err(WindowsError::from_thread())),
        count => Err(json!({"stage":stage,"unexpected_suspend_count":count})),
    }
}

struct HeldFile {
    path: PathBuf,
    file: File,
    identity: (u32, u32, u32),
}

fn file_identity(file: &File) -> Result<(u32, u32, u32)> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    api("file_identity", unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
    })?;
    require(
        info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "file_reparse",
    )?;
    Ok((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}

fn lock_path(path: &Path, held: &mut Vec<HeldFile>) -> Result<PathBuf> {
    require(path.is_absolute(), "absolute_path")?;
    let canonical = disk("canonical_path", path.canonicalize())?;
    require(
        canonical.to_string_lossy().starts_with(r"\\?\")
            && !canonical.to_string_lossy().starts_with(r"\\?\UNC\"),
        "local_path",
    )?;
    for part in canonical.ancestors() {
        let file = disk(
            "lock_path",
            OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ.0)
                .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
                .open(part),
        )?;
        let identity = file_identity(&file)?;
        held.push(HeldFile {
            path: part.to_owned(),
            file,
            identity,
        });
    }
    Ok(canonical)
}

fn revalidate(held: &[HeldFile]) -> Result<()> {
    for item in held {
        require(
            file_identity(&item.file)? == item.identity,
            "held_file_identity",
        )?;
        let current = disk(
            "reopen_bound_path",
            OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ.0)
                .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
                .open(&item.path),
        )?;
        require(file_identity(&current)? == item.identity, "path_identity")?;
    }
    Ok(())
}

fn mapping_target(path: &Path) -> Result<Vec<u16>> {
    let path: Vec<_> = path.as_os_str().encode_wide().collect();
    require(
        path.len() > 7
            && path[..4] == [92, 92, 63, 92]
            && path[4] <= 127
            && (path[4] as u8).is_ascii_alphabetic()
            && path[5..7] == [58, 92],
        "device_map_physical_path",
    )?;
    let name = [path[4], 58, 0];
    let mut target = [0u16; 1200];
    let used = unsafe { QueryDosDeviceW(PCWSTR(name.as_ptr()), Some(&mut target)) } as usize;
    if used == 0 {
        return api(
            "device_map_physical_volume",
            Err(WindowsError::from_thread()),
        );
    }
    require(used <= target.len(), "device_map_volume_limit")?;
    let length = target[..used]
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(|| failure("device_map_volume", "目标未终止"))?;
    let prefix: Vec<_> = r"\Device\HarddiskVolume".encode_utf16().collect();
    require(
        length > prefix.len()
            && target[..prefix.len()] == prefix
            && target[prefix.len()..length]
                .iter()
                .all(|unit| (48..=57).contains(unit))
            && target[length..used].iter().all(|unit| *unit == 0),
        "device_map_single_physical_volume",
    )?;
    let mut result = target[..length].to_vec();
    result.push(92);
    result.extend_from_slice(&path[7..]);
    require(result.len() < 1200, "device_map_target_limit")?;
    Ok(result)
}

fn mapping_receipt(
    bytes: &[u8],
    nonce: &[u8],
    process: &Process,
    target: &[u16],
    identity: (u32, u32, u32),
) -> Result<Value> {
    check_header(bytes, nonce, 7, 2612)?;
    let actual_process = parse_process(&bytes[HEADER..152]);
    let length = word(bytes, 184) as usize;
    let units: Vec<_> = bytes[212..]
        .chunks_exact(2)
        .map(|part| u16::from_le_bytes([part[0], part[1]]))
        .collect();
    let target_matches = length < units.len()
        && units[..length] == *target
        && units[length..].iter().all(|unit| *unit == 0);
    let target_identity = (word(bytes, 188), word(bytes, 192), word(bytes, 196));
    let mapped_identity = (word(bytes, 200), word(bytes, 204), word(bytes, 208));
    let alias = word(bytes, 180);
    let bound = actual_process.as_ref().is_ok_and(|value| value == process)
        && target_matches
        && target_identity == identity
        && mapped_identity == identity
        && (b'D' as u32..=b'Z' as u32).contains(&alias);
    Ok(
        json!({"stage":word(bytes,152),"win32_error":word(bytes,156),
        "hresult_from_win32":word(bytes,160),"cleanup_stage":word(bytes,164),"cleanup_error":word(bytes,168),
        "create_verified":word(bytes,172)==1,"remove_verified":word(bytes,176)==1,
        "alias_letter":alias,"target_utf16_units":length,"target_matches_bound_directory":target_matches,
        "target_file_id":target_identity,"mapped_file_id":mapped_identity,"expected_file_id":identity,
        "process":actual_process.as_ref().map(Process::json).unwrap_or(Value::Null),
        "process_error":actual_process.err(),"bound":bound,
        "confirmed":bound && word(bytes,152)==75 && word(bytes,156)==0 && word(bytes,160)==0
            && word(bytes,164)==76 && word(bytes,168)==0
            && word(bytes,172)==1 && word(bytes,176)==1}),
    )
}

struct Tree {
    job: Option<OwnedHandle>,
    first: Option<OwnedHandle>,
    thread: Option<OwnedHandle>,
    second: Option<OwnedHandle>,
    assigned: bool,
}

impl Tree {
    fn new() -> Result<Self> {
        let job = owned(api("create_job", unsafe {
            CreateJobObjectW(None, PCWSTR::null())
        })?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        api("strict_job_limits", unsafe {
            SetInformationJobObject(
                raw(&job),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        })?;
        Ok(Self {
            job: Some(job),
            first: None,
            thread: None,
            second: None,
            assigned: false,
        })
    }

    fn member(&self, process: HANDLE) -> Result<()> {
        let mut yes = BOOL::default();
        api("exact_job_membership", unsafe {
            IsProcessInJob(process, Some(raw(self.job.as_ref().unwrap())), &mut yes)
        })?;
        require(yes.as_bool(), "exact_job_membership")
    }

    fn active(&self) -> Result<u32> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        api("job_accounting", unsafe {
            QueryInformationJobObject(
                Some(raw(self.job.as_ref().unwrap())),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of_val(&info) as u32,
                None,
            )
        })?;
        Ok(info.ActiveProcesses)
    }

    fn cleanup(&mut self, retain_empty_job: bool) -> Value {
        let naturally_exited = self
            .first
            .iter()
            .chain(self.second.iter())
            .all(|p| unsafe { WaitForSingleObject(raw(p), 0) } == WAIT_OBJECT_0);
        let mut errors = Vec::new();
        if !naturally_exited || self.active().ok() != Some(0) {
            if let Err(error) = api("terminate_owned_job", unsafe {
                TerminateJobObject(raw(self.job.as_ref().unwrap()), 1)
            }) {
                errors.push(error);
            }
        }
        if !self.assigned {
            if let Some(first) = &self.first {
                if unsafe { WaitForSingleObject(raw(first), 0) } != WAIT_OBJECT_0 {
                    if let Err(error) = api("terminate_unassigned_first", unsafe {
                        TerminateProcess(raw(first), 1)
                    }) {
                        errors.push(error);
                    }
                }
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let empty = loop {
            let exited = self
                .first
                .iter()
                .chain(self.second.iter())
                .all(|p| unsafe { WaitForSingleObject(raw(p), 0) } == WAIT_OBJECT_0);
            match self.active() {
                Ok(0) if exited => break true,
                Err(error) => {
                    errors.push(error);
                    break false;
                }
                Ok(_) => {}
            }
            if Instant::now() >= deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(10));
        };
        for handle in [&mut self.thread, &mut self.second, &mut self.first] {
            if let Some(handle) = handle.take() {
                if let Err(error) = api("close_owned_handle", unsafe {
                    CloseHandle(HANDLE(handle.into_raw_handle()))
                }) {
                    errors.push(error);
                }
            }
        }
        // 只有原进程已退出、Job 已空且句柄关闭无错误，才允许延后关闭这个同一 Job。
        let retained = retain_empty_job && empty && errors.is_empty();
        if !retained {
            if let Some(job) = self.job.take() {
                if let Err(error) = api("close_owned_job", unsafe {
                    CloseHandle(HANDLE(job.into_raw_handle()))
                }) {
                    errors.push(error);
                }
            }
        }
        json!({"naturally_exited":naturally_exited,"job_empty_and_original_processes_signalled":empty,
            "handle_close_errors":errors,"confirmed":empty && errors.is_empty(),
            "job_retained_for_logon_observation":retained,"job_closed_before_logon_observation":!retained && errors.is_empty()})
    }

    fn close_retained_job(&mut self) -> Value {
        if self.job.is_none() {
            return json!({"retained":false,"confirmed":true});
        }
        let active = self.active();
        if active.as_ref().is_ok_and(|count| *count == 0) {
            let job = self.job.take().unwrap();
            let closed = api("close_retained_empty_job", unsafe {
                CloseHandle(HANDLE(job.into_raw_handle()))
            });
            return json!({"retained":true,"active_before_close":0,
                "confirmed":closed.is_ok(),"close_error":closed.err()});
        }
        // 不能把未知或非空 Job 当作空 Job 提前关闭；保留异常并走既有有界清理。
        let cleanup = self.cleanup(false);
        json!({"retained":true,"confirmed":false,"active_before_close":active.as_ref().ok(),
            "accounting_error":active.err(),"fallback_cleanup":cleanup})
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        if self.job.is_some() {
            self.cleanup(false);
        }
    }
}

// 只提供 Rust 1.92 Stdio::piped 的原管道，不能用另一种管道的阴性结果替代。
struct StdioKeeper {
    run: PathBuf,
    child: Option<Child>,
    reader: Option<ChildStdout>,
    tree: Tree,
    identity: Option<Process>,
    ready: Vec<u8>,
    closed: Option<Vec<u8>>,
}

impl StdioKeeper {
    fn start(
        helper: &Path,
        run: &Path,
        nonce: &[u8],
        environment: &[(String, OsString)],
        caller: &Process,
    ) -> Result<Self> {
        let mut keeper = Self {
            run: run.to_path_buf(),
            child: None,
            reader: None,
            tree: Tree::new()?,
            identity: None,
            ready: Vec::new(),
            closed: None,
        };
        let mut command = crate::blocking::Command::new_with_managed_process_group(helper);
        command
            .env_clear()
            .envs(environment.iter().map(|(key, value)| (key, value)))
            .env(format!("{PREFIX}STAGE"), "0")
            .current_dir(run)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        keeper.child = Some(disk("stdio_keeper_spawn", command.spawn())?);
        let child = keeper.child.as_mut().unwrap();
        let process = HANDLE(child.as_raw_handle());
        keeper.reader = child.stdout.take();
        let mut duplicate = HANDLE::default();
        api("stdio_keeper_original_process_duplicate", unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                process,
                GetCurrentProcess(),
                &mut duplicate,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        })?;
        keeper.tree.first = Some(owned(duplicate));
        api("stdio_keeper_assign_job", unsafe {
            AssignProcessToJobObject(raw(keeper.tree.job.as_ref().unwrap()), process)
        })?;
        keeper.tree.assigned = true;
        keeper.tree.member(process)?;
        let identity = process_snapshot(process)?;
        require(
            identity.pid == child.id()
                && identity.same_local_token(caller)
                && identity.same_logon(caller)
                && image_path(process)? == helper,
            "stdio_keeper_identity",
        )?;
        let ready = wait_file(&run.join("stdio-keeper-ready.bin"), 184, process)?;
        check_header(&ready, nonce, 8, 184)?;
        require(
            parse_process(&ready[HEADER..152])? == identity
                && ready[160..].iter().all(|byte| *byte == 0),
            "stdio_keeper_ready_identity",
        )?;
        require(keeper.reader.is_some(), "stdio_keeper_reader")?;
        keeper.identity = Some(identity);
        keeper.ready = ready;
        keeper.live()?;
        Ok(keeper)
    }

    fn live(&self) -> Result<()> {
        let process = HANDLE(
            self.child
                .as_ref()
                .ok_or_else(|| failure("stdio_keeper", "宿主缺失"))?
                .as_raw_handle(),
        );
        require(
            unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT,
            "stdio_keeper_alive",
        )?;
        require(
            self.identity.as_ref() == Some(&process_snapshot(process)?),
            "stdio_keeper_original_identity",
        )?;
        self.tree.member(process)
    }

    fn authorize(
        &self,
        first: HANDLE,
        identity: &Process,
        run: &Path,
        write_marker: bool,
    ) -> Result<()> {
        self.live()?;
        let source = HANDLE(
            ((u64::from(word(&self.ready, 156)) << 32) | u64::from(word(&self.ready, 152))) as usize
                as *mut c_void,
        );
        let mut local = HANDLE::default();
        api("stdio_keeper_pipe_check_duplicate", unsafe {
            DuplicateHandle(
                HANDLE(self.child.as_ref().unwrap().as_raw_handle()),
                source,
                GetCurrentProcess(),
                &mut local,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        })?;
        let local = owned(local);
        require(
            unsafe { GetFileType(raw(&local)) } == FILE_TYPE_PIPE,
            "stdio_keeper_pipe_type",
        )?;
        let mut transferred = HANDLE::default();
        api("stdio_transfer_to_suspended_first", unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                raw(&local),
                first,
                &mut transferred,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        })?;
        // 新副本只属于仍挂起且已绑定的 first；后续失败由该原进程的有界清理释放。
        let mut authorization = self.ready.clone();
        authorization[8..12].copy_from_slice(&9u32.to_le_bytes());
        let identity_bytes =
            unsafe { std::slice::from_raw_parts((identity as *const Process).cast(), 108) };
        authorization[HEADER..152].copy_from_slice(identity_bytes);
        authorization[152..160].copy_from_slice(&(transferred.0 as usize as u64).to_le_bytes());
        authorization[160..164].copy_from_slice(&u32::from(write_marker).to_le_bytes());
        save(&run.join("stdio-first-authorized.bin"), &authorization)
    }

    fn observe(
        &mut self,
        run: &Path,
        nonce: &[u8],
        identity: &Process,
        write_marker: bool,
    ) -> Result<Value> {
        self.live()?;
        let completion = read_receipt(&run.join("stdio-first-complete.bin"), 184)?
            .ok_or_else(|| failure("stdio_first_completion", "收据缺失"))?;
        check_header(&completion, nonce, 10, 184)?;
        require(
            parse_process(&completion[HEADER..152])? == *identity
                && word(&completion, 160) == u32::from(write_marker)
                && word(&completion, 164) == if write_marker { HEADER as u32 } else { 0 }
                && word(&completion, 168) == 1
                && word(&completion, 176) == 0
                && word(&completion, 180) == 0,
            "stdio_first_write_and_close",
        )?;
        let reader = self
            .reader
            .as_mut()
            .ok_or_else(|| failure("stdio_reader", "原读端缺失"))?;
        let pipe = HANDLE(reader.as_raw_handle());
        let mut available = 0;
        let deadline = Instant::now() + WAIT;
        loop {
            api("stdio_peek_marker", unsafe {
                PeekNamedPipe(pipe, None, 0, None, Some(&mut available), None)
            })?;
            // 不写病例绝不等待 marker；写病例只有完整定长数据可读后才进入 ReadFile。
            if !write_marker || available >= HEADER as u32 {
                break;
            }
            require(Instant::now() < deadline, "stdio_marker_deadline")?;
            thread::sleep(Duration::from_millis(10));
        }
        require(
            available == if write_marker { HEADER as u32 } else { 0 },
            "stdio_exact_available_bytes",
        )?;
        let context = if write_marker {
            let mut marker = [0; HEADER];
            disk("stdio_read_marker", reader.read_exact(&mut marker))?;
            check_header(&marker, nonce, 15, HEADER)?;
            // 原读端仍由本对象持有；独立线程 join 后才允许进入 LSA 查询。
            let pipe_value = pipe.0 as usize;
            let mut observation = thread::scope(|scope| scope.spawn(move || pipe_context(pipe_value)).join())
                .unwrap_or_else(|_| json!({"query_succeeded":false,"resources_released":false,"thread_panicked":true}));
            observation["thread_joined"] = json!(true);
            observation["cached_authentication_id_matches_writer"] =
                if observation["query_succeeded"] == true {
                    json!(
                        observation["token"]["auth_low"] == identity.auth_low
                            && observation["token"]["auth_high"] == identity.auth_high
                    )
                } else {
                    Value::Null
                };
            observation
        } else {
            json!({"attempted":false,"reason":"无写基线不模拟不存在的已读消息"})
        };
        Ok(
            json!({"write_marker":write_marker,"marker_bytes_read":if write_marker {HEADER} else {0},
            "no_write_did_not_wait_for_marker":!write_marker,"first_pipe_copy_closed":true,
            "context":context,"keeper_original_identity":self.identity.as_ref().map(Process::json),
            "keeper_exact_job":true,"old_reader_and_writer_still_held":true}),
        )
    }

    fn close_pipe(&mut self, run: &Path, nonce: &[u8]) -> Value {
        let closed = (|| -> Result<()> {
            self.live()?;
            let mut request = self.ready.clone();
            request[8..12].copy_from_slice(&11u32.to_le_bytes());
            save(&run.join("stdio-keeper-close.bin"), &request)?;
            let response = wait_file(
                &run.join("stdio-keeper-closed.bin"),
                184,
                HANDLE(self.child.as_ref().unwrap().as_raw_handle()),
            )?;
            check_header(&response, nonce, 12, 184)?;
            let mut expected = self.ready.clone();
            expected[8..12].copy_from_slice(&12u32.to_le_bytes());
            expected[168..172].copy_from_slice(&1u32.to_le_bytes());
            require(response == expected, "stdio_keeper_exact_close_ack")?;
            self.closed = Some(response);
            let reader = self
                .reader
                .take()
                .ok_or_else(|| failure("stdio_close_reader", "原读端缺失"))?;
            api("stdio_close_original_reader", unsafe {
                CloseHandle(HANDLE(reader.into_raw_handle()))
            })?;
            self.live()?;
            Ok(())
        })();
        json!({"confirmed":closed.is_ok(),"error":closed.err(),
            "keeper_stays_alive":self.live().is_ok(),"reader_released":self.reader.is_none()})
    }

    fn finish(&mut self, run: &Path, nonce: &[u8]) -> Value {
        let natural = (|| -> Result<()> {
            self.live()?;
            let mut request = self
                .closed
                .clone()
                .ok_or_else(|| failure("stdio_keeper_finish", "尚无关闭确认"))?;
            request[8..12].copy_from_slice(&13u32.to_le_bytes());
            save(&run.join("stdio-keeper-stop.bin"), &request)?;
            let process = HANDLE(self.child.as_ref().unwrap().as_raw_handle());
            require(
                unsafe { WaitForSingleObject(process, WAIT.as_millis() as u32) } == WAIT_OBJECT_0,
                "stdio_keeper_exit",
            )?;
            let mut exit = 0;
            api("stdio_keeper_exit_code", unsafe {
                GetExitCodeProcess(process, &mut exit)
            })?;
            require(exit == 0, "stdio_keeper_success")?;
            let response = read_receipt(&run.join("stdio-keeper-complete.bin"), 184)?
                .ok_or_else(|| failure("stdio_keeper_complete", "缺失"))?;
            check_header(&response, nonce, 14, 184)?;
            request[8..12].copy_from_slice(&14u32.to_le_bytes());
            require(response == request, "stdio_keeper_complete_identity")?;
            Ok(())
        })();
        let cleanup = self.cleanup();
        json!({"natural_exit_confirmed":natural.is_ok(),"natural_exit_error":natural.err(),"cleanup":cleanup})
    }

    fn cleanup(&mut self) -> Value {
        self.reader.take();
        let mut errors = Vec::new();
        if let Some(child) = self.child.as_mut() {
            if unsafe { WaitForSingleObject(HANDLE(child.as_raw_handle()), 0) } != WAIT_OBJECT_0 {
                // 派生后尚未能复制/认领时，也只通过原 Child 句柄回收。
                if let Err(error) = disk("stdio_keeper_terminate_original", child.kill()) {
                    errors.push(error);
                }
            }
            // 已有 Tree 原句柄时只沿同一五秒回收，不叠加第二个五秒等待。
            if self.tree.first.is_none()
                && unsafe { WaitForSingleObject(HANDLE(child.as_raw_handle()), 5000) }
                    != WAIT_OBJECT_0
            {
                errors.push(failure("stdio_keeper_unclaimed_exit", "原句柄退出未确认"));
            }
        }
        let result = self.tree.cleanup(false);
        let child_exited = self.child.as_ref().is_none_or(|child| unsafe {
            WaitForSingleObject(HANDLE(child.as_raw_handle()), 0) == WAIT_OBJECT_0
        });
        self.child.take();
        json!({"confirmed":child_exited && errors.is_empty() && result["confirmed"] == true,
            "original_child_signalled":child_exited,"errors":errors,"job_cleanup":result})
    }
}

impl Drop for StdioKeeper {
    fn drop(&mut self) {
        if self.child.is_some() {
            let cleanup = self.cleanup();
            // setup 的早退也保存实际清理结果，不让 RAII 隐去失败阶段。
            if let Ok(bytes) = serde_json::to_vec_pretty(&cleanup) {
                let _ = save(
                    &self.run.join("stdio-keeper-fallback-cleanup.safe.json"),
                    &bytes,
                );
            }
        }
    }
}

fn pipe_context(pipe_value: usize) -> Value {
    let mut report = json!({"attempted":true,"query_succeeded":false,"resources_released":false});
    if let Err(error) = no_thread_token() {
        report["precondition_error"] = error;
        let reverted = api("stdio_precondition_revert", unsafe { RevertToSelf() });
        report["revert_confirmed"] = json!(reverted.is_ok());
        report["revert_error"] = reverted.err().unwrap_or(Value::Null);
        return report;
    }
    let impersonated = api("stdio_impersonate", unsafe {
        ImpersonateNamedPipeClient(HANDLE(pipe_value as *mut c_void))
    });
    report["impersonate_error"] = impersonated.as_ref().err().cloned().unwrap_or(Value::Null);
    let mut token_closed = true;
    if impersonated.is_ok() {
        let query = (|| -> Result<Value> {
            let mut token = HANDLE::default();
            api("stdio_open_thread_token", unsafe {
                OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token)
            })?;
            let token = owned(token);
            let result = (|| -> Result<Value> {
                let statistics: TOKEN_STATISTICS = token_field(raw(&token), TokenStatistics)?;
                let level: SECURITY_IMPERSONATION_LEVEL =
                    token_field(raw(&token), TokenImpersonationLevel)?;
                Ok(json!({"auth_low":statistics.AuthenticationId.LowPart,
                    "auth_high":statistics.AuthenticationId.HighPart as u32,"impersonation_level":level.0}))
            })();
            let close = api("stdio_close_context_token", unsafe {
                CloseHandle(HANDLE(token.into_raw_handle()))
            });
            token_closed = close.is_ok();
            report["token_close_error"] = close.err().unwrap_or(Value::Null);
            result
        })();
        report["query_succeeded"] = json!(query.is_ok());
        match query {
            Ok(value) => report["token"] = value,
            Err(error) => report["query_error"] = error,
        }
    }
    // 即使模拟/查询失败也显式恢复；Revert 失败的线程立即结束，调用方 join 后拒绝该病例。
    let reverted = api("stdio_revert", unsafe { RevertToSelf() });
    report["revert_confirmed"] = json!(reverted.is_ok());
    report["revert_error"] = reverted.err().unwrap_or(Value::Null);
    report["token_closed"] = json!(token_closed);
    report["resources_released"] = json!(token_closed && report["revert_confirmed"] == true);
    report
}

fn logon_observation(process: &Process) -> Value {
    let luid = LUID {
        LowPart: process.auth_low,
        HighPart: process.auth_high as i32,
    };
    let mut data = std::ptr::null_mut();
    let status = unsafe { LsaGetLogonSessionData(&luid, &mut data) };
    if status.0 != 0 {
        return json!({"status":status.0 as u32,"gone":status.0 as u32 == 0xc000005f,"query_succeeded":false});
    }
    if data.is_null() {
        return json!({"status":0,"gone":false,"invalid_null":true});
    }
    let result = unsafe { &*data };
    let observation = json!({"status":0,"gone":false,"query_succeeded":true,
        "auth_low":result.LogonId.LowPart,"auth_high":result.LogonId.HighPart as u32,
        "logon_type":result.LogonType,"session":result.Session});
    let freed = unsafe { LsaFreeReturnBuffer(data.cast()) };
    json!({"data":observation,"free_status":freed.0 as u32,"gone":false})
}

fn station_observation(name: &str) -> Value {
    let encoded: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    match unsafe { OpenWindowStationW(PCWSTR(encoded.as_ptr()), false, READ_CONTROL.0 | 2) } {
        Ok(handle) => {
            json!({"gone":false,"open_succeeded":true,"close_confirmed":unsafe { CloseWindowStation(handle) }.is_ok()})
        }
        Err(error) => {
            json!({"gone":error.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0),"open_succeeded":false,"hresult":error.code().0 as u32})
        }
    }
}

fn wait_file(path: &Path, limit: usize, first: HANDLE) -> Result<Vec<u8>> {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(bytes) = read_receipt(path, limit)? {
            return Ok(bytes);
        }
        require(
            unsafe { WaitForSingleObject(first, 0) } == WAIT_TIMEOUT,
            "helper_exited_before_receipt",
        )?;
        if Instant::now() >= deadline {
            return Err(failure("wait_receipt", "有界等待超时"));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = disk(
        "create_receipt",
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path),
    )?;
    disk("write_receipt", file.write_all(bytes))?;
    disk("sync_receipt", file.sync_all())
}

fn execute(
    with_device_map: bool,
    retain_empty_job: bool,
    stdio_write: Option<bool>,
) -> Result<Value> {
    let environment = |suffix: &str| {
        std::env::var_os(format!("{PREFIX}{suffix}")).ok_or_else(|| failure("environment", suffix))
    };
    let helper_input = PathBuf::from(environment("HELPER")?);
    let root_input = PathBuf::from(environment("EVIDENCE_DIR")?);
    let nonce = environment("NONCE")?
        .into_string()
        .map_err(|_| failure("nonce", "编码无效"))?;
    let expected = environment("HELPER_SHA256")?
        .into_string()
        .map_err(|_| failure("helper_sha256", "编码无效"))?;
    require(
        nonce.len() == 32
            && nonce
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "nonce",
    )?;
    require(
        expected.len() == 64
            && expected
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "helper_digest",
    )?;
    let mut held = Vec::new();
    let helper = lock_path(&helper_input, &mut held)?;
    let root = lock_path(&root_input, &mut held)?;
    let source = disk("helper_read", File::open(&helper))?;
    let metadata = disk("helper_metadata", source.metadata())?;
    require(
        metadata.is_file() && metadata.len() <= 16 * 1024 * 1024,
        "helper_regular_and_bounded",
    )?;
    let mut hasher = Sha256::new();
    disk(
        "helper_digest_read",
        std::io::copy(&mut source.take(16 * 1024 * 1024 + 1), &mut hasher),
    )?;
    require(hex(&hasher.finalize()) == expected, "helper_digest_match")?;
    let run = root.join(format!("run-{nonce}"));
    disk("create_run_directory", fs::create_dir(&run))?;
    lock_path(&run, &mut held)?;
    let mapping = if with_device_map {
        let identity = held
            .iter()
            .find(|item| item.path == run)
            .ok_or_else(|| failure("device_map_bound_directory", "目录锁缺失"))?
            .identity;
        Some((mapping_target(&run)?, identity))
    } else {
        None
    };
    let mut tree = Tree::new()?;
    let mut report = json!({"schema":1,"candidate_only":true,"g09_closed":false,"nonce":nonce,"helper_sha256":expected,
        "validation_scope":"nonadministrator_service_candidate","interactive_user_validation_passed":false,
        "acl_modified":false,"network_called":false,"explicit_station_selection":false,
        "first_lpdesktop":"NULL","second_lpdesktop":"empty","second_inherit_handles":false,
        "with_device_map":with_device_map,"retain_empty_job":retain_empty_job,
        "appcontainer_console_debugger_combination_tested":false});
    let mut first_identity = None;
    let mut second_identity = None;
    let mut candidate_station = None;
    let mut parent_objects = None;
    let mut stdio_keeper = None;
    let scenario = (|| -> Result<()> {
        no_thread_token()?;
        let caller = process_snapshot(unsafe { GetCurrentProcess() })?;
        report["caller"] = caller.json();
        let (thread_admin, process_admin) = administrator_memberships()?;
        report["calling_thread_administrator_member"] = json!(thread_admin);
        report["process_administrator_member"] = json!(process_admin);
        report["ordinary_unelevated_caller"] = json!(
            caller.elevated == 0 && caller.integrity <= 0x2000 && !thread_admin && !process_admin
        );
        require(
            !thread_admin && !process_admin,
            "nonadministrator_calling_identity",
        )?;
        let original = current_objects()?;
        report["caller_station"] = original.0.json()?;
        report["caller_desktop"] = original.1.json()?;
        parent_objects = Some(original.clone());
        let program = wide(helper.as_os_str())?;
        require(
            !program.contains(&(b'"' as u16)) && program.len() < 900,
            "helper_command_path",
        )?;
        let mut command = wide(format!("\"{}\"", helper.display()).as_ref())?;
        let cwd = wide(run.as_os_str())?;
        let system_root =
            std::env::var_os("SystemRoot").ok_or_else(|| failure("system_root", "缺失"))?;
        require(
            Path::new(&system_root).is_absolute(),
            "system_root_absolute",
        )?;
        let mut values = vec![
            ("SystemRoot".to_owned(), system_root.clone()),
            ("WINDIR".to_owned(), system_root),
            ("TEMP".to_owned(), run.clone().into_os_string()),
            ("TMP".to_owned(), run.clone().into_os_string()),
            (format!("{PREFIX}RUN_DIR"), run.clone().into_os_string()),
            (format!("{PREFIX}NONCE"), nonce.clone().into()),
            (format!("{PREFIX}STAGE"), "1".into()),
        ];
        if with_device_map {
            values.push((format!("{PREFIX}DEVICE_MAP"), "1".into()));
        }
        if let Some(write_marker) = stdio_write {
            require(
                !with_device_map && !retain_empty_job,
                "stdio_separate_control",
            )?;
            // keeper 只用明确的私有环境；不读取或继承完整调用方环境。
            stdio_keeper = Some(StdioKeeper::start(
                &helper,
                &run,
                nonce.as_bytes(),
                &values,
                &caller,
            )?);
            values.push((
                format!("{PREFIX}STDIO"),
                if write_marker { "1" } else { "0" }.into(),
            ));
            report["stdio_control"] = json!({"write_marker":write_marker,"rust_stdio_piped":true});
        }
        values.sort_by_key(|(key, _)| key.to_ascii_uppercase());
        let mut block = Vec::new();
        for (key, value) in values {
            block.extend(wide(format!("{key}={}", value.to_string_lossy()).as_ref())?);
        }
        block.push(0);
        let username = wide(format!("ISP_{}", &nonce[..12]).as_ref())?;
        let domain = wide(".".as_ref())?;
        let mut password = wide(format!("FakeOnly-{nonce}-!a9").as_ref())?;
        let startup = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut created = PROCESS_INFORMATION::default();
        revalidate(&held)?;
        let spawned = unsafe {
            CreateProcessWithLogonW(
                PCWSTR(username.as_ptr()),
                PCWSTR(domain.as_ptr()),
                PCWSTR(password.as_ptr()),
                LOGON_NETCREDENTIALS_ONLY,
                PCWSTR(program.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                Some(block.as_ptr().cast()),
                PCWSTR(cwd.as_ptr()),
                &startup,
                &mut created,
            )
        };
        for unit in &mut password {
            unsafe { std::ptr::write_volatile(unit, 0) };
        }
        api("create_netcredentials_first", spawned)?;
        tree.first = Some(owned(created.hProcess));
        tree.thread = Some(owned(created.hThread));
        let first = raw(tree.first.as_ref().unwrap());
        let first_snapshot = process_snapshot(first)?;
        report["first_created"] = first_snapshot.json();
        first_identity = Some(first_snapshot.clone());
        require(
            first_snapshot.same_local_token(&caller) && first_snapshot.pid == created.dwProcessId,
            "first_local_identity",
        )?;
        require(image_path(first)? == helper, "first_image")?;
        api("assign_first_job", unsafe {
            AssignProcessToJobObject(raw(tree.job.as_ref().unwrap()), first)
        })?;
        tree.assigned = true;
        tree.member(first)?;
        report["first_exact_job_before_resume"] = json!(true);
        report["new_logon_observation"] = logon_observation(&first_snapshot);
        report["new_authentication_id"] = json!(!first_snapshot.same_logon(&caller));
        require(
            !first_snapshot.same_logon(&caller),
            "first_new_authentication_id",
        )?;
        // 这是按 AuthenticationId 作出的站名预测，不假定它总等于 token 的 logon SID。
        // 恢复前仅接受该名字确实不存在；第二段必须以实际对象名验证，不回退其他名字。
        let auth_high = first_snapshot.auth_high;
        let auth_low = first_snapshot.auth_low;
        let expected_station = format!("Service-0x{auth_high:x}-{auth_low:x}$");
        candidate_station = Some(expected_station.clone());
        report["station_name_prediction"] = json!(expected_station);
        report["station_before_resume"] = station_observation(&expected_station);
        let station_absent_before = report["station_before_resume"]["gone"] == true;
        report["station_absent_before"] = json!(station_absent_before);
        require(
            station_absent_before,
            "predicted_station_absent_before_resume",
        )?;
        require(
            process_snapshot(first)? == first_snapshot,
            "first_before_resume",
        )?;
        if let (Some(keeper), Some(write_marker)) = (&stdio_keeper, stdio_write) {
            keeper.authorize(first, &first_snapshot, &run, write_marker)?;
            // 移交句柄后再次核对原挂起进程与原 Job，之后才恢复。
            require(
                process_snapshot(first)? == first_snapshot,
                "stdio_first_before_resume",
            )?;
            tree.member(first)?;
        }
        resume(raw(tree.thread.as_ref().unwrap()), "resume_first")?;
        let suspended = wait_file(&run.join("suspended2.bin"), 152, first)?;
        check_header(&suspended, nonce.as_bytes(), 3, 152)?;
        let candidate = parse_process(&suspended[HEADER..])?;
        tree.second = Some(owned(api("open_suspended_second", unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                candidate.pid,
            )
        })?));
        let second = raw(tree.second.as_ref().unwrap());
        require(
            process_snapshot(second)? == candidate
                && candidate.same_local_token(&first_snapshot)
                && candidate.same_logon(&first_snapshot),
            "second_kernel_identity",
        )?;
        require(
            image_path(second)? == helper && candidate.pid != first_snapshot.pid,
            "second_image",
        )?;
        tree.member(second)?;
        tree.member(first)?;
        revalidate(&held)?;
        report["second_created"] = candidate.json();
        second_identity = Some(candidate.clone());
        report["second_exact_job_before_resume"] = json!(true);
        let mut authorization = suspended;
        authorization[8..12].copy_from_slice(&4u32.to_le_bytes());
        save(&run.join("resume2.bin"), &authorization)?;
        let completion = wait_file(&run.join("stage1-complete.bin"), 80, first)?;
        check_header(&completion, nonce.as_bytes(), 5, 80)?;
        report["first_completion"] = json!({"stage":word(&completion,44),"win32_error":word(&completion,48),
            "hresult_from_win32":word(&completion,52),"child_wait":word(&completion,56),"child_exit":word(&completion,60),
            "process_closed":word(&completion,64),"thread_closed":word(&completion,68),
            "cleanup_stage":word(&completion,72),"cleanup_error":word(&completion,76)});
        require(
            word(&completion, 48) == 0
                && word(&completion, 56) == WAIT_OBJECT_0.0
                && word(&completion, 60) == 0
                && word(&completion, 64) == 1
                && word(&completion, 68) == 1
                && word(&completion, 76) == 0,
            "helper_completion",
        )?;
        require(
            unsafe { WaitForSingleObject(first, 5000) } == WAIT_OBJECT_0,
            "first_exit_signal",
        )?;
        let mut exit = 0;
        api("first_exit_code", unsafe {
            GetExitCodeProcess(first, &mut exit)
        })?;
        require(
            exit == 0 && unsafe { WaitForSingleObject(second, 0) } == WAIT_OBJECT_0,
            "helper_exit",
        )?;
        let first_bytes = read_receipt(&run.join("stage1.bin"), 33980)?
            .ok_or_else(|| failure("stage1", "收据缺失"))?;
        let second_bytes = read_receipt(&run.join("stage2.bin"), 33980)?
            .ok_or_else(|| failure("stage2", "收据缺失"))?;
        let a = observation(&first_bytes, nonce.as_bytes(), 1)?;
        let b = observation(&second_bytes, nonce.as_bytes(), 2)?;
        report["first_station"] = a.1.json()?;
        report["first_desktop"] = a.2.json()?;
        report["second_station"] = b.1.json()?;
        report["second_desktop"] = b.2.json()?;
        require(
            a.0 == first_snapshot && b.0 == candidate,
            "self_and_original_handle_identity",
        )?;
        // fInherit 属于各进程中的句柄；同一对象的名称、标志和描述符仍须完全一致。
        let same_object = |actual: &Object, expected: &Object| {
            actual.name == expected.name
                && actual.flags == expected.flags
                && actual.sd == expected.sd
        };
        require(
            same_object(&a.1, &original.0) && same_object(&a.2, &original.1),
            "first_inherited_objects",
        )?;
        let station_name_matches = b.1.name.eq_ignore_ascii_case(&expected_station);
        report["station_name_prediction_matched"] = json!(station_name_matches);
        report["noninteractive_noninheritable"] =
            json!(b.1.flags & 1 == 0 && b.1.inherit == 0 && b.2.inherit == 0);
        require(
            station_absent_before
                && station_name_matches
                && !first_snapshot.same_logon(&caller)
                && !b.1.name.eq_ignore_ascii_case(&a.1.name)
                && b.1.flags & 1 == 0
                && b.1.inherit == 0
                && b.2.inherit == 0,
            "candidate_independent_station",
        )?;
        revalidate(&held)?;
        if let (Some(keeper), Some(write_marker)) = (&mut stdio_keeper, stdio_write) {
            report["stdio_control"] =
                keeper.observe(&run, nonce.as_bytes(), &first_snapshot, write_marker)?;
            require(
                !write_marker || report["stdio_control"]["context"]["resources_released"] == true,
                "stdio_context_resources_released",
            )?;
        }
        Ok(())
    })();
    // 无论原生调用还是合同失败，先处理本次确切 Job 与原创建句柄，之后才允许断言。
    report["cleanup"] = tree.cleanup(retain_empty_job);
    report["scenario_ok"] = json!(scenario.is_ok());
    report["scenario_error"] = scenario.err().unwrap_or(Value::Null);
    report["caller_objects_unchanged"] = match parent_objects {
        Some(original) => json!(current_objects().is_ok_and(|actual| actual == original)),
        None => Value::Null,
    };
    if let Some(keeper) = &stdio_keeper {
        let live = keeper.live();
        report["stdio_keeper_before_original_query"] =
            json!({"confirmed":live.is_ok(),"error":live.err()});
    }
    if let Some(identity) = &first_identity {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let logon = logon_observation(identity);
            let station = candidate_station
                .as_ref()
                .map(|name| station_observation(name));
            let finished = logon["gone"] == true
                && station.as_ref().is_some_and(|value| value["gone"] == true);
            report["logon_after_exit"] = logon;
            report["station_after_exit"] = station.unwrap_or(Value::Null);
            if finished || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    // 原三秒查询结果保持为判定依据；随后关闭已空 Job，只作一次新的观察，不等待或洗掉失败。
    report["retained_job_release"] = tree.close_retained_job();
    if retain_empty_job {
        report["logon_after_job_close"] = first_identity
            .as_ref()
            .map(logon_observation)
            .unwrap_or(Value::Null);
        report["station_after_job_close"] = candidate_station
            .as_ref()
            .map(|name| station_observation(name))
            .unwrap_or(Value::Null);
    }
    if let Some(keeper) = &mut stdio_keeper {
        let live = keeper.live();
        report["stdio_keeper_after_original_query"] =
            json!({"confirmed":live.is_ok(),"error":live.err()});
        report["stdio_pipe_release"] = keeper.close_pipe(&run, nonce.as_bytes());
        // 单次后观察只能说明资源释放之后的状态，绝不覆盖原三秒结果。
        report["logon_after_pipe_close"] = first_identity
            .as_ref()
            .map(logon_observation)
            .unwrap_or(Value::Null);
        report["station_after_pipe_close"] = candidate_station
            .as_ref()
            .map(|name| station_observation(name))
            .unwrap_or(Value::Null);
        let live = keeper.live();
        report["stdio_keeper_after_post_query"] =
            json!({"confirmed":live.is_ok(),"error":live.err()});
        report["stdio_keeper_finish"] = keeper.finish(&run, nonce.as_bytes());
    }
    if let Some((target, identity)) = mapping {
        report["device_map_expected_nt_target"] = json!(String::from_utf16(&target).ok());
        report["device_map"] = match (
            read_receipt(&run.join("device-map.bin"), 2612),
            second_identity.as_ref(),
        ) {
            (Ok(Some(bytes)), Some(process)) => {
                mapping_receipt(&bytes, nonce.as_bytes(), process, &target, identity)
                    .unwrap_or_else(|error| json!({"confirmed":false,"error":error}))
            }
            (Err(error), _) => json!({"confirmed":false,"error":error}),
            (Ok(_), _) => json!({"confirmed":false,"receipt_or_bound_process_missing":true}),
        };
    }
    let mut receipt_names = vec![
        "stage1.bin",
        "suspended2.bin",
        "resume2.bin",
        "stage2.bin",
        "stage1-complete.bin",
        "stage2-complete.bin",
        "device-map.bin",
    ];
    if stdio_write.is_some() {
        receipt_names.extend([
            "stdio-keeper-ready.bin",
            "stdio-first-authorized.bin",
            "stdio-first-complete.bin",
            "stdio-keeper-close.bin",
            "stdio-keeper-closed.bin",
            "stdio-keeper-stop.bin",
            "stdio-keeper-complete.bin",
            "stdio-keeper-fallback-cleanup.safe.json",
        ]);
        report["stdio_native_completions"] = json!(
            [
                ("stdio-first-complete.bin", 10),
                ("stdio-keeper-complete.bin", 14)
            ]
            .into_iter()
            .map(|(name, kind)| match read_receipt(&run.join(name), 184) {
                Ok(Some(bytes)) if check_header(&bytes, nonce.as_bytes(), kind, 184).is_ok() =>
                    json!({
                    "name":name,"stage":word(&bytes,172),"win32_error":word(&bytes,176),
                    "hresult_from_win32":word(&bytes,180),"write_marker":word(&bytes,160),
                    "written":word(&bytes,164),"pipe_closed":word(&bytes,168)}),
                Ok(Some(_)) => json!({"name":name,"valid":false}),
                Ok(None) => json!({"name":name,"present":false}),
                Err(error) => json!({"name":name,"error":error}),
            })
            .collect::<Vec<_>>()
        );
    }
    let receipts: Vec<_> = receipt_names
        .into_iter()
        .map(|name| match read_receipt(&run.join(name), 33980) {
            Ok(Some(bytes)) => {
                json!({"name":name,"bytes":bytes.len(),"sha256":hex(&Sha256::digest(bytes))})
            }
            Ok(None) => json!({"name":name,"present":false}),
            Err(error) => json!({"name":name,"error":error}),
        })
        .collect();
    report["raw_receipts"] = json!(receipts);
    // 子阶段在写 suspended2 之前失败时，也必须提取它保存的原始错误，不能只剩等待错误。
    report["native_completions"] = json!([("stage1-complete.bin",5),("stage2-complete.bin",6)]
        .into_iter().map(|(name,kind)| match read_receipt(&run.join(name),80) {
            Ok(Some(bytes)) if check_header(&bytes,nonce.as_bytes(),kind,80).is_ok() => json!({
                "name":name,"stage":word(&bytes,44),"win32_error":word(&bytes,48),
                "hresult_from_win32":word(&bytes,52),"child_wait":word(&bytes,56),"child_exit":word(&bytes,60),
                "process_closed":word(&bytes,64),"thread_closed":word(&bytes,68),
                "cleanup_stage":word(&bytes,72),"cleanup_error":word(&bytes,76)}),
            Ok(Some(_)) => json!({"name":name,"valid":false}),
            Ok(None) => json!({"name":name,"present":false}),
            Err(error) => json!({"name":name,"error":error}),
        }).collect::<Vec<_>>());
    let candidate_passed = report["scenario_ok"] == true
        && report["cleanup"]["confirmed"] == true
        && report["cleanup"]["job_retained_for_logon_observation"] == retain_empty_job
        && report["retained_job_release"]["confirmed"] == true
        && (!with_device_map || report["device_map"]["confirmed"] == true)
        && (stdio_write.is_none()
            || (report["stdio_keeper_before_original_query"]["confirmed"] == true
                && report["stdio_keeper_after_original_query"]["confirmed"] == true
                && report["stdio_pipe_release"]["confirmed"] == true
                && report["stdio_keeper_after_post_query"]["confirmed"] == true
                && report["stdio_keeper_finish"]["natural_exit_confirmed"] == true
                && report["stdio_keeper_finish"]["cleanup"]["confirmed"] == true))
        && report["caller_objects_unchanged"] == true
        && report["station_after_exit"]["gone"] == true
        && report["logon_after_exit"]["gone"] == true;
    report["candidate_contract_passed"] = json!(candidate_passed);
    report["ordinary_user_validation_passed"] =
        json!(candidate_passed && report["ordinary_unelevated_caller"] == true);
    report["passed"] = json!(candidate_passed);
    let bytes =
        serde_json::to_vec_pretty(&report).map_err(|_| failure("serialize", "审计序列化失败"))?;
    save(&run.join("netcredentials.safe.json"), &bytes)?;
    Ok(report)
}

pub(super) fn run(with_device_map: bool, retain_empty_job: bool) {
    let report = execute(with_device_map, retain_empty_job, None)
        .unwrap_or_else(|error| json!({"passed":false,"setup_error":error,"g09_closed":false}));
    eprintln!("windows_netcredentials_station_candidate={report}");
    assert_eq!(
        report["passed"], true,
        "非管理员服务身份的新原生路径未满足候选条件；保留本轮原件，不自动重跑"
    );
}

pub(super) fn run_stdio(write_marker: bool) {
    let report = execute(false, false, Some(write_marker))
        .unwrap_or_else(|error| json!({"passed":false,"setup_error":error,"g09_closed":false}));
    eprintln!("windows_netcredentials_stdio_candidate={report}");
    assert_eq!(
        report["passed"], true,
        "管道生命周期对照的原三秒条件未满足；后置观察不能覆盖原失败"
    );
}
