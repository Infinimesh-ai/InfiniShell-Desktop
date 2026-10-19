use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use warp_errors::{ErrorExt, register_error};

#[derive(thiserror::Error, Debug)]
pub enum FileSaveError {
    #[error("No file path associated with file when saving file {0:?}")]
    NoFilePath(FileId),
    #[error("IO error when saving file.")]
    IOError {
        #[source]
        error: io::Error,
        path: PathBuf,
    },
    #[error("Remote file operation failed: {0}")]
    RemoteError(String),
    /// 内容已写入源路径，但原生移动失败；撤销必须恢复源内容。
    #[error("{0}")]
    RenameFailedAfterSave(String),
    /// A non-IO failure with a self-describing message (e.g. content could
    /// not be derived for the write).
    #[error("{0}")]
    Other(String),
}

impl ErrorExt for FileSaveError {
    fn is_actionable(&self) -> bool {
        match self {
            FileSaveError::NoFilePath(_) | FileSaveError::Other(_) => true,
            FileSaveError::IOError { .. }
            | FileSaveError::RemoteError(_)
            | FileSaveError::RenameFailedAfterSave(_) => false,
        }
    }
}
register_error!(FileSaveError);

#[derive(thiserror::Error, Debug)]
pub enum FileLoadError {
    #[error("File does not exist")]
    DoesNotExist,
    #[error("IO error when loading file.")]
    IOError(#[from] io::Error),
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileId(usize);

impl FileId {
    /// Constructs a new globally-unique file ID.
    #[allow(clippy::new_without_default)]
    pub fn new() -> FileId {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        FileId(raw)
    }
}

/// 删除与覆盖移动撤销需要的最小权限快照；不包含 ACL 或扩展属性。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilePermissions {
    pub unix_mode: Option<u32>,
    pub readonly: bool,
}

/// Agent 的破坏性差异暂不支持恢复链接身份，必须在修改内容前拒绝这些目录项。
pub fn restorable_file_permissions(path: &Path) -> io::Result<FilePermissions> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Agent delete/move requires a regular file entry",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Agent delete/move cannot restore a hard-linked entry",
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
            GetFileInformationByHandle,
        };
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // 文件句柄在调用期间有效，输出结构由当前栈持有。
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(io::Error::from)?;
        if info.nNumberOfLinks != 1 || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Agent delete/move cannot restore a linked or reparse entry",
            ));
        }
    }
    #[cfg(not(any(unix, windows)))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "File entry identity validation is unavailable",
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(FilePermissions {
            unix_mode: Some(metadata.permissions().mode() & 0o7777),
            readonly: metadata.permissions().readonly(),
        })
    }
    #[cfg(windows)]
    Ok(FilePermissions {
        unix_mode: None,
        readonly: metadata.permissions().readonly(),
    })
}

/// 撤销移动不得覆盖用户在原路径新建的目录项；不支持时失败，不降级为覆盖或复制。
pub fn rename_file_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let from = CString::new(from.as_os_str().as_bytes())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let to = CString::new(to.as_os_str().as_bytes())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        // 两个 C 字符串在系统调用期间有效；EXCL/NOREPLACE 由内核禁止覆盖目标。
        #[cfg(target_os = "macos")]
        let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
        use windows::core::HSTRING;
        let from = HSTRING::from(from.as_os_str());
        let to = HSTRING::from(to.as_os_str());
        // 不设置 REPLACE_EXISTING 或 COPY_ALLOWED，保留原生不覆盖移动语义。
        unsafe { MoveFileExW(&from, &to, MOVE_FILE_FLAGS(0)) }.map_err(io::Error::from)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("Non-replacing rename from {from:?} to {to:?} is unavailable"),
    ))
}
