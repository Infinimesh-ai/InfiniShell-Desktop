//! Windows 启动记录使用明确的当前用户/SYSTEM DACL，禁止重解析点和路径替换。

use std::ffi::c_void;
use std::fs::{self, File};
use std::io::{self, Read as _, Write as _};
use std::mem;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::MetadataExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _};
use std::path::{Path, PathBuf};
use std::ptr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, EqualSid, GetAce,
    GetSecurityDescriptorControl, GetTokenInformation, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser, WinLocalSystemSid,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ALL_ACCESS,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFileInformationByHandle, OPEN_EXISTING, READ_CONTROL,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{PCWSTR, PWSTR};

fn private_error() -> io::Error {
    io::Error::other("Windows 启动记录的私有权限或身份无效")
}

struct LocalMemory(*mut c_void);

impl Drop for LocalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // Windows 安全描述符和字符串均由 LocalAlloc 分配。
            unsafe {
                LocalFree(Some(HLOCAL(self.0)));
            }
        }
    }
}

struct PrivateSecurity {
    memory: LocalMemory,
    user: Vec<usize>,
    ace_flags: u8,
}

impl PrivateSecurity {
    fn new(inherit: bool) -> io::Result<Self> {
        let mut token = HANDLE::default();
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
            .map_err(|_| private_error())?;
        let mut length = 0;
        unsafe {
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut length);
        }
        if length == 0 || length > 65536 {
            unsafe {
                let _ = CloseHandle(token);
            }
            return Err(private_error());
        }
        let mut user = vec![0usize; (length as usize).div_ceil(mem::size_of::<usize>())];
        let result = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(user.as_mut_ptr().cast()),
                length,
                &mut length,
            )
        };
        unsafe {
            let _ = CloseHandle(token);
        }
        result.map_err(|_| private_error())?;
        let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        let mut text = PWSTR::null();
        unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(|_| private_error())?;
        let text_memory = LocalMemory(text.0.cast());
        let user_sid = unsafe { text.to_string() }.map_err(|_| private_error())?;
        drop(text_memory);
        // 目录权限向其子项继承；Windows 会从普通文件的显式 ACE 去掉 OI/CI 标记。
        // 两类对象仍都只允许当前用户和 SYSTEM 完全访问，并保护 DACL 不继承父目录授权。
        let (inheritance, ace_flags) = if inherit { ("OICI", 3) } else { ("", 0) };
        let sddl =
            format!("O:{user_sid}D:P(A;{inheritance};FA;;;{user_sid})(A;{inheritance};FA;;;SY)");
        let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|_| private_error())?;
        Ok(Self {
            memory: LocalMemory(descriptor.0),
            user,
            ace_flags,
        })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.memory.0,
            bInheritHandle: false.into(),
        }
    }

    fn verify(&self, file: &File) -> io::Result<()> {
        let mut owner = PSID::default();
        let mut dacl: *mut ACL = ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let error = unsafe {
            GetSecurityInfo(
                HANDLE(file.as_raw_handle()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                Some(&mut owner),
                None,
                Some(&mut dacl),
                None,
                Some(&mut descriptor),
            )
        };
        let memory = LocalMemory(descriptor.0);
        if error.0 != 0 || dacl.is_null() || owner.0.is_null() {
            return Err(private_error());
        }
        let user_sid = unsafe { (*(self.user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        unsafe { EqualSid(owner, user_sid) }.map_err(|_| private_error())?;
        let mut control = 0;
        let mut revision = 0;
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) }
            .map_err(|_| private_error())?;
        if control & SE_DACL_PROTECTED.0 == 0 || unsafe { (*dacl).AceCount } != 2 {
            return Err(private_error());
        }
        let mut user_seen = false;
        let mut system_seen = false;
        for index in 0..2 {
            let mut raw = ptr::null_mut();
            unsafe { GetAce(dacl, index, &mut raw) }.map_err(|_| private_error())?;
            if raw.is_null() {
                return Err(private_error());
            }
            // ACCESS_ALLOWED_ACE 的类型为 0；标记必须与目录或普通文件契约一致。
            let header = unsafe { &*(raw.cast::<ACE_HEADER>()) };
            if header.AceType != 0 || header.AceFlags != self.ace_flags || header.AceSize < 16 {
                return Err(private_error());
            }
            let ace = unsafe { &*(raw.cast::<ACCESS_ALLOWED_ACE>()) };
            if ace.Mask != FILE_ALL_ACCESS.0 {
                return Err(private_error());
            }
            let sid = PSID(ptr::addr_of!(ace.SidStart).cast_mut().cast());
            if unsafe { EqualSid(sid, user_sid) }.is_ok() && !user_seen {
                user_seen = true;
            } else if unsafe { IsWellKnownSid(sid, WinLocalSystemSid) }.as_bool() && !system_seen {
                system_seen = true;
            } else {
                return Err(private_error());
            }
        }
        drop(memory);
        if !user_seen || !system_seen {
            return Err(private_error());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileIdentity {
    volume: u32,
    index: u64,
}

pub(super) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(io::Error::from)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(private_error());
    }
    Ok(FileIdentity {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

pub(super) fn plain_path(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(private_error());
    }
    for component in path.ancestors() {
        if fs::symlink_metadata(component)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        {
            return Err(private_error());
        }
    }
    Ok(())
}

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if units.contains(&0) {
        return Err(private_error());
    }
    units.push(0);
    Ok(units)
}

pub(super) fn directory(path: &Path) -> io::Result<File> {
    plain_path(path)?;
    let wide = wide_path(path)?;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            READ_CONTROL.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(io::Error::from)?;
    let file = unsafe { File::from_raw_handle(handle.0) };
    if !file.metadata()?.is_dir() {
        return Err(private_error());
    }
    file_identity(&file)?;
    PrivateSecurity::new(true)?.verify(&file)?;
    Ok(file)
}

fn create_directory(path: &Path) -> io::Result<File> {
    plain_path(path.parent().ok_or_else(private_error)?)?;
    let security = PrivateSecurity::new(true)?;
    let attributes = security.attributes();
    let wide = wide_path(path)?;
    unsafe { CreateDirectoryW(PCWSTR(wide.as_ptr()), Some(&attributes)) }
        .map_err(io::Error::from)?;
    directory(path)
}

pub(super) fn create_launch_directory(state: &Path) -> io::Result<(PathBuf, FileIdentity)> {
    let state = state.canonicalize()?;
    plain_path(&state)?;
    let parent = state.join("grok-owned-terminal");
    let _parent = if parent.try_exists()? {
        directory(&parent)?
    } else {
        create_directory(&parent)?
    };
    let path = parent.join(format!("launch-{}", Uuid::new_v4()));
    let file = create_directory(&path)?;
    Ok((path, file_identity(&file)?))
}

pub(super) fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > 65536 {
        return Err(private_error());
    }
    let _parent = directory(path.parent().ok_or_else(private_error)?)?;
    let security = PrivateSecurity::new(false)?;
    let attributes = security.attributes();
    let wide = wide_path(path)?;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ,
            Some(&attributes),
            CREATE_NEW,
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(io::Error::from)?;
    let mut file = unsafe { File::from_raw_handle(handle.0) };
    security.verify(&file)?;
    file_identity(&file)?;
    file.write_all(bytes)?;
    file.sync_all()
}

pub(super) fn read_private(path: &Path) -> io::Result<Vec<u8>> {
    let _parent = directory(path.parent().ok_or_else(private_error)?)?;
    let wide = wide_path(path)?;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_READ.0 | READ_CONTROL.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(io::Error::from)?;
    let file = unsafe { File::from_raw_handle(handle.0) };
    PrivateSecurity::new(false)?.verify(&file)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(io::Error::from)?;
    file_identity(&file)?;
    if !file.metadata()?.is_file() || info.nNumberOfLinks != 1 || file.metadata()?.len() > 65536 {
        return Err(private_error());
    }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(private_error());
    }
    Ok(bytes)
}

pub(super) fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_private(path).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
