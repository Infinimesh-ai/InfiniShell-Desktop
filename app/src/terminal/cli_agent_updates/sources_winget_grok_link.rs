//! 版本化 native 名称改变时，以对象句柄交换 WinGet 公共链接，未知占位不覆盖。

use std::fs::{self, File, OpenOptions};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::{OpenOptionsExt as _, symlink_file};
use std::os::windows::io::AsRawHandle as _;
use std::path::Path;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    DELETE, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_RENAME_INFO, FILE_SHARE_READ, FileDispositionInfo, FileRenameInfo,
    SetFileInformationByHandle,
};

use super::Error;
use super::winget_grok_tree::{self as tree, Link};

pub(super) fn at(expected: &Link, path: &Path) -> Link {
    let mut value = expected.clone();
    value.path = path.to_owned();
    value
}

pub(super) fn absent(path: &Path) -> Result<bool, Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err(Error::SourceChanged),
    }
}

pub(super) fn matches(expected: &Link, path: &Path) -> Result<bool, Error> {
    if absent(path)? {
        return Ok(false);
    }
    Ok(Link::capture(path)? == at(expected, path))
}

pub(super) fn create(original: &Link, path: &Path, target: &Path) -> Result<Link, Error> {
    if original.path.parent() != path.parent() || !absent(path)? {
        return Err(Error::SourceChanged);
    }
    let _parents = tree::parents(path)?;
    let _original = original.hold()?;
    symlink_file(target, path).map_err(|_| Error::PermissionDenied)?;
    tree::copy_security(&original.path, path)?;
    let value = Link::capture(path)?;
    if value.target != target {
        return Err(Error::SourceChanged);
    }
    Ok(value)
}

fn hold(expected: &Link) -> Result<File, Error> {
    let file = OpenOptions::new()
        .access_mode(0x8000_0000 | DELETE.0)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
        .open(&expected.path)
        .map_err(|_| Error::SourceChanged)?;
    if Link::capture(&expected.path)? != *expected {
        return Err(Error::SourceChanged);
    }
    Ok(file)
}

pub(super) fn rename(expected: &Link, destination: &Path) -> Result<(), Error> {
    if expected.path.parent() != destination.parent() || !absent(destination)? {
        return Err(Error::SourceChanged);
    }
    let _parents = tree::parents(&expected.path)?;
    let file = hold(expected)?;
    let name: Vec<_> = destination.as_os_str().encode_wide().collect();
    if name.is_empty() || name.len() > 32767 || name.contains(&0) || !destination.is_absolute() {
        return Err(Error::SourceChanged);
    }
    let length = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * 2;
    let mut storage = vec![0_usize; length.div_ceil(std::mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast(),
            name.len(),
        );
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileRenameInfo,
            info.cast(),
            length as u32,
        )
    }
    .map_err(|_| Error::PersistenceFailed)?;
    if !matches(expected, destination)? {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

pub(super) fn remove(expected: &Link, path: &Path) -> Result<(), Error> {
    if absent(path)? {
        return Ok(());
    }
    let _parents = tree::parents(path)?;
    let file = hold(&at(expected, path))?;
    let deletion = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            &deletion as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&deletion) as u32,
        )
    }
    .map_err(|_| Error::PersistenceFailed)
}
