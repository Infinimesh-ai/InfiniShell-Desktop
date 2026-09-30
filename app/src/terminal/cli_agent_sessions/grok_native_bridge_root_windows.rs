//! Windows 普通桥的发现目录来自系统账号配置，不读取 HOME 或 USERPROFILE 环境值。

#[path = "grok_owned_windows_files.rs"]
mod files;

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStringExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::{Component, Path, PathBuf, Prefix};

use sha2::{Digest as _, Sha256};
use warp_core::channel::{Channel, ChannelState};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::Security::TOKEN_QUERY;
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::GetUserProfileDirectoryW;
use windows::core::{HRESULT, PWSTR};

pub(crate) fn root() -> io::Result<PathBuf> {
    prepare_root(
        &profile_directory()?,
        &directory_name(
            ChannelState::channel(),
            ChannelState::data_profile().as_deref(),
        ),
    )
}

fn profile_directory() -> io::Result<PathBuf> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(io::Error::from)?;
    let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
    let mut length = 0;
    let _ = unsafe { GetUserProfileDirectoryW(HANDLE(token.as_raw_handle()), None, &mut length) };
    if length == 0 || length > 32768 {
        return Err(invalid());
    }
    let mut buffer = vec![0u16; length as usize];
    unsafe {
        GetUserProfileDirectoryW(
            HANDLE(token.as_raw_handle()),
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )
    }
    .map_err(io::Error::from)?;
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(invalid)?;
    if end == 0 || buffer[end..].iter().any(|unit| *unit != 0) {
        return Err(invalid());
    }
    let path = PathBuf::from(OsString::from_wide(&buffer[..end]));
    // 先拒绝链接，再规范驱动器前缀；不得把重解析点规范化后冒充原系统目录。
    let _ancestors = pin_ancestors(&path)?;
    dunce::canonicalize(path)
}

fn directory_name(channel: Channel, profile: Option<&str>) -> String {
    let mut digest = Sha256::new();
    digest.update(channel.to_string().as_bytes());
    digest.update([0]);
    match profile {
        Some(profile) => {
            digest.update([1]);
            digest.update(profile.as_bytes());
        }
        None => digest.update([0]),
    }
    let digest = format!("{:x}", digest.finalize());
    format!(".isp-grok-{}", &digest[..24])
}

fn prepare_root(home: &Path, name: &str) -> io::Result<PathBuf> {
    let _ancestors = pin_ancestors(home)?;
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(invalid());
    }
    let root = home.join(name);
    if root.parent() != Some(home) {
        return Err(invalid());
    }
    let directory = match files::create_directory(&root) {
        Ok(directory) => directory,
        // Windows 绑定保留 HRESULT；只识别精确的“已存在”，随后仍验证私有权限和目录身份。
        Err(error)
            if error.kind() == io::ErrorKind::AlreadyExists
                || error.raw_os_error() == Some(HRESULT::from_win32(ERROR_ALREADY_EXISTS.0).0) =>
        {
            files::directory(&root)?
        }
        Err(error) => return Err(error),
    };
    let original = files::file_identity(&directory)?;
    if original != files::file_identity(&files::directory(&root)?)? {
        return Err(invalid());
    }
    Ok(root)
}

/// 保留整条祖先链的目录句柄，禁止发现和请求期间通过重命名替换命名空间。
pub(crate) fn pin_ancestors(path: &Path) -> io::Result<Vec<File>> {
    let mut components = path.components();
    if !path.is_absolute()
        || !matches!(
            components.next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        )
        || !components
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(invalid());
    }
    files::plain_path(path)?;
    let mut pinned = Vec::new();
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        // 仅查询属性的句柄不参与共享访问检查；必须请求目录读取才能阻止 DELETE/重命名。
        let file = File::options()
            .access_mode(FILE_LIST_DIRECTORY.0 | FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(ancestor)?;
        if !file.metadata()?.is_dir() {
            return Err(invalid());
        }
        files::file_identity(&file)?;
        pinned.push(file);
    }
    files::plain_path(path)?;
    Ok(pinned)
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "grok_native_bridge.root")
}

#[cfg(test)]
#[path = "grok_native_bridge_root_windows_tests.rs"]
mod tests;
