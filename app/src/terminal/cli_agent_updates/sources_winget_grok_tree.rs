//! Grok WinGet 整树身份；固定别名硬链接和 Zone.Identifier 均计入原始身份。

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Seek as _, Write as _};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, EqualSid, GetLengthSid, IsValidAcl, IsValidSid,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_NAME_NORMALIZED, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_STREAM_INFO, FileDispositionInfo, FileRenameInfo, FileStreamInfo,
    GetFileInformationByHandle, GetFileInformationByHandleEx, GetFinalPathNameByHandleW,
    READ_CONTROL, SetFileInformationByHandle, WRITE_DAC,
};

use super::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub(super) volume: u32,
    pub(super) index: u64,
    pub(super) directory: bool,
    pub(super) attributes: u32,
    pub(super) length: u64,
    pub(super) digest: [u8; 32],
    pub(super) security: [u8; 32],
    pub(super) zone: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    pub(super) members: BTreeMap<PathBuf, Identity>,
}

struct Security {
    allocation: PSECURITY_DESCRIPTOR,
    owner: PSID,
    acl: *mut ACL,
}

impl Security {
    fn read(file: &File) -> Result<Self, Error> {
        let mut value = Self {
            allocation: PSECURITY_DESCRIPTOR::default(),
            owner: PSID::default(),
            acl: std::ptr::null_mut(),
        };
        let status = unsafe {
            GetSecurityInfo(
                HANDLE(file.as_raw_handle()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                Some(&mut value.owner),
                None,
                Some(&mut value.acl),
                None,
                Some(&mut value.allocation),
            )
        };
        if status.0 != 0
            || value.owner.0.is_null()
            || value.acl.is_null()
            || !unsafe { IsValidSid(value.owner) }.as_bool()
            || !unsafe { IsValidAcl(value.acl) }.as_bool()
        {
            return Err(Error::PermissionDenied);
        }
        Ok(value)
    }

    fn digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(unsafe {
            std::slice::from_raw_parts(self.owner.0.cast::<u8>(), GetLengthSid(self.owner) as usize)
        });
        digest.update(unsafe {
            std::slice::from_raw_parts(self.acl.cast::<u8>(), usize::from((*self.acl).AclSize))
        });
        digest.finalize().into()
    }
}

impl Drop for Security {
    fn drop(&mut self) {
        if !self.allocation.0.is_null() {
            unsafe { LocalFree(Some(HLOCAL(self.allocation.0))) };
        }
    }
}

pub(super) fn copy_security(original: &Path, candidate: &Path) -> Result<(), Error> {
    let source = open(original, false)?;
    let candidate = OpenOptions::new()
        .access_mode((READ_CONTROL | WRITE_DAC).0)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
        .open(candidate)
        .map_err(|_| Error::PermissionDenied)?;
    let source = Security::read(&source)?;
    let before = Security::read(&candidate)?;
    unsafe { EqualSid(source.owner, before.owner) }.map_err(|_| Error::PermissionDenied)?;
    let status = unsafe {
        SetSecurityInfo(
            HANDLE(candidate.as_raw_handle()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(source.acl),
            None,
        )
    };
    if status.0 != 0 || Security::read(&candidate)?.digest() != source.digest() {
        return Err(Error::PermissionDenied);
    }
    Ok(())
}

fn open(path: &Path, mutation: bool) -> Result<File, Error> {
    OpenOptions::new()
        .access_mode(0x8000_0000 | READ_CONTROL.0 | if mutation { DELETE.0 } else { 0 })
        .share_mode(if mutation {
            FILE_SHARE_READ.0
        } else {
            (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0
        })
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
        .open(path)
        .map_err(|_| Error::SourceChanged)
}

fn information(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, Error> {
    let mut value = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut value) }
        .map_err(|_| Error::SourceChanged)?;
    if value.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        || !(1..=2).contains(&value.nNumberOfLinks)
    {
        return Err(Error::UnsupportedSource);
    }
    Ok(value)
}

fn final_path(file: &File) -> Result<PathBuf, Error> {
    let mut buffer = [0u16; 32768];
    let count = unsafe {
        GetFinalPathNameByHandleW(
            HANDLE(file.as_raw_handle()),
            &mut buffer,
            FILE_NAME_NORMALIZED,
        )
    } as usize;
    if count == 0 || count >= buffer.len() {
        return Err(Error::SourceChanged);
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..count])))
}

fn zone_path(path: &Path) -> OsString {
    let mut value = path.as_os_str().to_owned();
    value.push(":Zone.Identifier");
    value
}

fn read_zone(file: &File) -> Result<Option<Vec<u8>>, Error> {
    let mut buffer = [0u64; 1024];
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileStreamInfo,
            buffer.as_mut_ptr().cast(),
            std::mem::size_of_val(&buffer) as u32,
        )
    }
    .map_err(|_| Error::UnsupportedSource)?;
    let mut offset = 0usize;
    let mut names = BTreeSet::new();
    loop {
        let base = offset
            .checked_add(std::mem::offset_of!(FILE_STREAM_INFO, StreamName))
            .ok_or(Error::SourceChanged)?;
        if base > std::mem::size_of_val(&buffer) || offset % 8 != 0 {
            return Err(Error::SourceChanged);
        }
        let stream = unsafe {
            buffer
                .as_ptr()
                .cast::<u8>()
                .add(offset)
                .cast::<FILE_STREAM_INFO>()
        };
        let length = unsafe { (*stream).StreamNameLength } as usize;
        if length % 2 != 0 || length > 1024 || base + length > std::mem::size_of_val(&buffer) {
            return Err(Error::SourceChanged);
        }
        let name = String::from_utf16(unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!((*stream).StreamName).cast::<u16>(),
                length / 2,
            )
        })
        .map_err(|_| Error::UnsupportedSource)?;
        if !matches!(name.as_str(), "::$DATA" | ":Zone.Identifier:$DATA")
            || !names.insert(name.clone())
        {
            return Err(Error::UnsupportedSource);
        }
        if name == ":Zone.Identifier:$DATA"
            && !(1..=4096).contains(&unsafe { (*stream).StreamSize })
        {
            return Err(Error::UnsupportedSource);
        }
        let next = unsafe { (*stream).NextEntryOffset } as usize;
        if next == 0 {
            break;
        }
        if next < std::mem::offset_of!(FILE_STREAM_INFO, StreamName) + length {
            return Err(Error::SourceChanged);
        }
        offset = offset.checked_add(next).ok_or(Error::SourceChanged)?;
    }
    if !names.contains("::$DATA") {
        return Err(Error::UnsupportedSource);
    }
    if !names.contains(":Zone.Identifier:$DATA") {
        return Ok(None);
    }
    let mut input = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(zone_path(&final_path(file)?))
        .map_err(|_| Error::SourceChanged)?;
    let mut bytes = Vec::new();
    (&mut input)
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::SourceChanged)?;
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(Error::UnsupportedSource);
    }
    Ok(Some(bytes))
}

pub(super) fn copy_zone(expected: &Identity, target: &Path) -> Result<(), Error> {
    let Some(bytes) = &expected.zone else {
        return Ok(());
    };
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(Error::SourceChanged);
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(zone_path(target))
        .map_err(|_| Error::PersistenceFailed)?;
    file.write_all(bytes)
        .map_err(|_| Error::PersistenceFailed)?;
    file.sync_all().map_err(|_| Error::PersistenceFailed)
}

fn identity(file: &File) -> Result<Identity, Error> {
    let before = information(file)?;
    let directory = before.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    let mut length = 0;
    let mut digest = [0; 32];
    let mut zone = None;
    if !directory {
        // WinGet 官方下载保留 MotW；只接受已命名的安全标记，未知 ADS 仍拒绝。
        zone = read_zone(file)?;
        length = (u64::from(before.nFileSizeHigh) << 32) | u64::from(before.nFileSizeLow);
        if length > 512 * 1024 * 1024 {
            return Err(Error::UnsupportedSource);
        }
        let mut input = file.try_clone().map_err(|_| Error::SourceChanged)?;
        input.rewind().map_err(|_| Error::SourceChanged)?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        let mut count = 0u64;
        loop {
            let size = input.read(&mut buffer).map_err(|_| Error::SourceChanged)?;
            if size == 0 {
                break;
            }
            count += size as u64;
            if count > length {
                return Err(Error::SourceChanged);
            }
            hash.update(&buffer[..size]);
        }
        if count != length {
            return Err(Error::SourceChanged);
        }
        digest = hash.finalize().into();
    }
    if !directory && read_zone(file)? != zone {
        return Err(Error::SourceChanged);
    }
    let after = information(file)?;
    if before.dwVolumeSerialNumber != after.dwVolumeSerialNumber
        || before.nFileIndexHigh != after.nFileIndexHigh
        || before.nFileIndexLow != after.nFileIndexLow
        || before.dwFileAttributes != after.dwFileAttributes
        || !directory
            && (before.ftLastWriteTime.dwHighDateTime != after.ftLastWriteTime.dwHighDateTime
                || before.ftLastWriteTime.dwLowDateTime != after.ftLastWriteTime.dwLowDateTime)
    {
        return Err(Error::SourceChanged);
    }
    Ok(Identity {
        volume: before.dwVolumeSerialNumber,
        index: (u64::from(before.nFileIndexHigh) << 32) | u64::from(before.nFileIndexLow),
        directory,
        attributes: before.dwFileAttributes,
        length,
        digest,
        security: Security::read(file)?.digest(),
        zone,
    })
}

pub(super) fn path_identity(path: &Path) -> Result<Identity, Error> {
    super::plain_ancestors(path)?;
    identity(&open(path, false)?)
}

pub(super) fn safe_relative(path: &Path) -> Result<(), Error> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || path
            .to_string_lossy()
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '"' | '%' | '!' | '&' | '|' | '<' | '>'))
    {
        return Err(Error::UnsupportedSource);
    }
    Ok(())
}

pub(super) fn snapshot(root: &Path) -> Result<Snapshot, Error> {
    super::plain_ancestors(root)?;
    let mut members = BTreeMap::new();
    let mut aliases = BTreeSet::new();
    let mut links: BTreeMap<(u32, u64), Vec<(PathBuf, u32)>> = BTreeMap::new();
    let mut pending = vec![PathBuf::new()];
    let mut bytes = 0u64;
    while let Some(relative) = pending.pop() {
        if members.len() >= 256 {
            return Err(Error::UnsupportedSource);
        }
        let path = root.join(&relative);
        let opened = open(&path, false)?;
        let before = identity(&opened)?;
        bytes = bytes
            .checked_add(before.length)
            .ok_or(Error::UnsupportedSource)?;
        if bytes > 1024 * 1024 * 1024 {
            return Err(Error::UnsupportedSource);
        }
        if before.directory {
            for entry in fs::read_dir(&path).map_err(|_| Error::SourceChanged)? {
                let child = relative.join(entry.map_err(|_| Error::SourceChanged)?.file_name());
                safe_relative(&child)?;
                if !aliases.insert(child.to_string_lossy().to_ascii_lowercase()) {
                    return Err(Error::SourceChanged);
                }
                pending.push(child);
            }
        }
        if identity(&opened)? != before || path_identity(&path)? != before {
            return Err(Error::SourceChanged);
        }
        if !before.directory {
            links
                .entry((before.volume, before.index))
                .or_default()
                .push((relative.clone(), information(&opened)?.nNumberOfLinks));
        }
        members.insert(relative, before);
    }
    for group in links.values() {
        if group
            .iter()
            .any(|(_, count)| *count as usize != group.len())
        {
            return Err(Error::UnsupportedSource);
        }
        if group.len() > 1 {
            let members = group
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<BTreeSet<_>>();
            if !["1.0.40", "1.0.41"].into_iter().any(|version| {
                members
                    == BTreeSet::from([
                        PathBuf::from("grok.exe"),
                        PathBuf::from(format!("grok-{version}-windows-x86_64.exe")),
                    ])
            }) {
                return Err(Error::UnsupportedSource);
            }
        }
    }
    Ok(Snapshot { members })
}

pub(super) fn parents(path: &Path) -> Result<Vec<File>, Error> {
    path.parent()
        .ok_or(Error::SourceChanged)?
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|path| {
            let file = OpenOptions::new()
                .read(true)
                .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
                .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
                .open(path)
                .map_err(|_| Error::SourceChanged)?;
            if !identity(&file)?.directory {
                return Err(Error::SourceChanged);
            }
            Ok(file)
        })
        .collect()
}

pub(super) fn rename(root: &Path, destination: &Path, expected: &Snapshot) -> Result<(), Error> {
    if root.parent() != destination.parent()
        || destination.try_exists().map_err(|_| Error::SourceChanged)?
    {
        return Err(Error::SourceChanged);
    }
    let _parents = parents(root)?;
    let file = open(root, true)?;
    if snapshot(root)? != *expected
        || identity(&file)?
            != *expected
                .members
                .get(Path::new(""))
                .ok_or(Error::RecoveryRequired)?
    {
        return Err(Error::SourceChanged);
    }
    let name: Vec<_> = destination.as_os_str().encode_wide().collect();
    let length = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * 2;
    let mut buffer = vec![0usize; length.div_ceil(std::mem::size_of::<usize>())];
    let rename = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*rename).Anonymous.ReplaceIfExists = false;
        (*rename).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*rename).FileName).cast(),
            name.len(),
        );
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileRenameInfo,
            rename.cast(),
            length as u32,
        )
    }
    .map_err(|_| Error::PersistenceFailed)?;
    if snapshot(destination)? != *expected {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

pub(super) fn remove_matching(root: &Path, expected: &Snapshot) -> Result<(), Error> {
    if !root.try_exists().map_err(|_| Error::RecoveryRequired)? {
        return Ok(());
    }
    let _parents = parents(root)?;
    let remaining = snapshot(root)?;
    // 清理中断后只可继续删除原清单的剩余子集；任何新增或改变的成员都保留。
    if remaining
        .members
        .iter()
        .any(|(path, identity)| expected.members.get(path) != Some(identity))
    {
        return Err(Error::RecoveryRequired);
    }
    for (relative, expected) in remaining.members.iter().rev() {
        let file = open(&root.join(relative), true)?;
        if identity(&file)? != *expected {
            return Err(Error::RecoveryRequired);
        }
        let deletion = FILE_DISPOSITION_INFO { DeleteFile: true };
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                &deletion as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&deletion) as u32,
            )
        }
        .map_err(|_| Error::RecoveryRequired)?;
    }
    Ok(())
}

/// 写入访问只用于排除已经映射或即将映射的 PE，不向任何文件写数据。
/// 与应用启动预约同时持有，直至两步目录切换结束；占用时禁止目录切换。
pub(super) fn freeze_images(root: &Path, expected: &Snapshot) -> Result<Vec<File>, Error> {
    let mut held = Vec::new();
    let mut identities = BTreeSet::new();
    for (relative, before) in &expected.members {
        if before.directory
            || !relative.extension().is_some_and(|suffix| {
                suffix.as_encoded_bytes().eq_ignore_ascii_case(b"exe")
                    || suffix.as_encoded_bytes().eq_ignore_ascii_case(b"dll")
            })
        {
            continue;
        }
        if !identities.insert((before.volume, before.index)) {
            continue;
        }
        let path = root.join(relative);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_DELETE).0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
            .map_err(|_| Error::SourceChanged)?;
        if identity(&file)? != *before {
            return Err(Error::SourceChanged);
        }
        held.push(file);
    }
    if snapshot(root)? != *expected {
        return Err(Error::SourceChanged);
    }
    Ok(held)
}

/// 公共链接本身的对象身份与目标文本；目标目录暂时移开时也能核验。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Link {
    pub(super) path: PathBuf,
    pub(super) target: PathBuf,
    volume: u32,
    index: u64,
    security: [u8; 32],
}

impl Link {
    pub(super) fn capture(path: &Path) -> Result<Self, Error> {
        super::plain_ancestors(path.parent().ok_or(Error::SourceChanged)?)?;
        let file = open(path, false)?;
        let mut value = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut value) }
            .map_err(|_| Error::SourceChanged)?;
        if value.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
            || value.nNumberOfLinks != 1
            || !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(Error::UnsupportedSource);
        }
        let target = fs::read_link(path).map_err(|_| Error::SourceChanged)?;
        if !target.is_absolute() {
            return Err(Error::UnsupportedSource);
        }
        Ok(Self {
            path: path.to_owned(),
            target,
            volume: value.dwVolumeSerialNumber,
            index: (u64::from(value.nFileIndexHigh) << 32) | u64::from(value.nFileIndexLow),
            security: Security::read(&file)?.digest(),
        })
    }

    pub(super) fn hold(&self) -> Result<File, Error> {
        // 禁止重建或改写链接，但不打开、执行或锁住链接目标。
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
            .open(&self.path)
            .map_err(|_| Error::SourceChanged)?;
        if Self::capture(&self.path)? != *self {
            return Err(Error::SourceChanged);
        }
        Ok(file)
    }
}
