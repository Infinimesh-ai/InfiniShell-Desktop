//! npm 单包目录的 Unix 原子交换；所有相对操作锚定已打开的目录描述符。

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::Error;

const MAX_FILES: usize = 2048;
const MAX_TREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}

impl Identity {
    fn read(metadata: &Metadata) -> Result<Self, Error> {
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o7022 != 0
            || !(metadata.is_dir() || metadata.is_file())
        {
            return Err(Error::UnsupportedSource);
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Node {
    identity: Identity,
    length: u64,
    sha256: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    pub(super) root: Identity,
    nodes: BTreeMap<PathBuf, Node>,
}

impl Snapshot {
    pub(super) fn verify_release_files(
        &self,
        expected: &BTreeMap<PathBuf, (u64, [u8; 32])>,
    ) -> Result<(), Error> {
        let mut directories = std::collections::BTreeSet::new();
        for path in expected.keys() {
            for ancestor in path
                .ancestors()
                .skip(1)
                .filter(|path| !path.as_os_str().is_empty())
            {
                directories.insert(ancestor.to_owned());
            }
        }
        if self.nodes.len() != expected.len() + directories.len() {
            return Err(Error::InvalidRelease);
        }
        for (path, node) in &self.nodes {
            match node.sha256 {
                Some(digest) if expected.get(path) == Some(&(node.length, digest)) => {}
                None if directories.contains(path) => {}
                Some(_) | None => return Err(Error::InvalidRelease),
            }
        }
        Ok(())
    }
}

pub(super) struct Directory {
    file: File,
}

fn name(value: &OsStr) -> Result<CString, Error> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(Error::SourceChanged);
    }
    CString::new(bytes).map_err(|_| Error::SourceChanged)
}

fn file(fd: libc::c_int) -> Result<File, Error> {
    if fd < 0 {
        return Err(Error::SourceChanged);
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

// 本版只保留 POSIX mode/uid/gid；有额外 ACL 或安全属性时拒绝，不能静默丢失权限。
#[cfg(target_os = "macos")]
fn reject_extra_permissions(file: &File) -> Result<(), Error> {
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_int) -> *mut libc::c_void;
        fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
    }
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
    if !acl.is_null() {
        unsafe { acl_free(acl) };
        return Err(Error::UnsupportedSource);
    }
    if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT) {
        return Err(Error::UnsupportedSource);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn reject_extra_permissions(file: &File) -> Result<(), Error> {
    let size = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
    if size < 0 || size > 64 * 1024 {
        return Err(Error::UnsupportedSource);
    }
    let mut names = vec![0_u8; size as usize];
    if size != 0 {
        let actual =
            unsafe { libc::flistxattr(file.as_raw_fd(), names.as_mut_ptr().cast(), names.len()) };
        if actual != size {
            return Err(Error::SourceChanged);
        }
    }
    if names.split(|byte| *byte == 0).any(|name| {
        name.starts_with(b"system.posix_acl_")
            || name.starts_with(b"security.")
            || name.starts_with(b"trusted.")
    }) {
        return Err(Error::UnsupportedSource);
    }
    Ok(())
}

impl Directory {
    pub(super) fn open(path: &Path) -> Result<Self, Error> {
        if !path.is_absolute() {
            return Err(Error::SourceChanged);
        }
        let mut current = Self {
            file: file(unsafe {
                libc::open(
                    c"/".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            })?,
        };
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(part) => current = current.child(part)?,
                Component::Prefix(_) | Component::CurDir | Component::ParentDir => {
                    return Err(Error::SourceChanged);
                }
            }
        }
        current.identity()?;
        Ok(current)
    }

    pub(super) fn identity(&self) -> Result<Identity, Error> {
        reject_extra_permissions(&self.file)?;
        Identity::read(&self.file.metadata().map_err(|_| Error::SourceChanged)?)
    }

    pub(super) fn child(&self, leaf: &OsStr) -> Result<Self, Error> {
        let leaf = name(leaf)?;
        Ok(Self {
            file: file(unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    leaf.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            })?,
        })
    }

    pub(super) fn has_child(&self, leaf: &OsStr) -> Result<bool, Error> {
        let leaf = name(leaf)?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.file.as_raw_fd(),
                leaf.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            return Ok(true);
        }
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(Error::SourceChanged)
        }
    }

    pub(super) fn create(&self, leaf: &OsStr) -> Result<Self, Error> {
        let leaf_c = name(leaf)?;
        if unsafe { libc::mkdirat(self.file.as_raw_fd(), leaf_c.as_ptr(), 0o700) } != 0 {
            return Err(Error::PersistenceFailed);
        }
        self.sync()?;
        let directory = self.child(leaf)?;
        directory.identity()?;
        Ok(directory)
    }

    fn names(&self) -> Result<Vec<OsString>, Error> {
        // fdopendir 拥有一个独立描述符，关闭列表游标不影响调用方持有的目录。
        let duplicate = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if duplicate < 0 {
            return Err(Error::SourceChanged);
        }
        let directory = unsafe { libc::fdopendir(duplicate) };
        if directory.is_null() {
            unsafe { libc::close(duplicate) };
            return Err(Error::SourceChanged);
        }
        let result = (|| {
            let mut result = Vec::new();
            loop {
                #[cfg(target_os = "macos")]
                let errno = unsafe { libc::__error() };
                #[cfg(target_os = "linux")]
                let errno = unsafe { libc::__errno_location() };
                unsafe { *errno = 0 };
                let entry = unsafe { libc::readdir(directory) };
                if entry.is_null() {
                    if unsafe { *errno } != 0 {
                        return Err(Error::SourceChanged);
                    }
                    break;
                }
                let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if bytes == b"." || bytes == b".." {
                    continue;
                }
                if result.len() >= MAX_FILES {
                    return Err(Error::UnsupportedSource);
                }
                result.push(OsStr::from_bytes(bytes).to_owned());
            }
            result.sort();
            Ok(result)
        })();
        unsafe { libc::closedir(directory) };
        result
    }

    fn read_file(&self, leaf: &OsStr) -> Result<File, Error> {
        let leaf = name(leaf)?;
        file(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        })
    }

    pub(super) fn snapshot(&self) -> Result<Snapshot, Error> {
        let root = self.identity()?;
        let mut nodes = BTreeMap::new();
        let mut bytes = 0;
        self.snapshot_into(Path::new(""), &mut nodes, &mut bytes)?;
        if self.identity()? != root {
            return Err(Error::SourceChanged);
        }
        Ok(Snapshot { root, nodes })
    }

    fn snapshot_into(
        &self,
        relative: &Path,
        nodes: &mut BTreeMap<PathBuf, Node>,
        bytes: &mut u64,
    ) -> Result<(), Error> {
        if relative.components().count() > 32 {
            return Err(Error::UnsupportedSource);
        }
        let names = self.names()?;
        for leaf in &names {
            if nodes.len() >= MAX_FILES {
                return Err(Error::UnsupportedSource);
            }
            let path = relative.join(leaf);
            let mut opened = self.read_file(leaf)?;
            reject_extra_permissions(&opened)?;
            let before = opened.metadata().map_err(|_| Error::SourceChanged)?;
            let identity = Identity::read(&before)?;
            let node = if before.is_dir() {
                let child = Self { file: opened };
                child.snapshot_into(&path, nodes, bytes)?;
                Node {
                    identity,
                    length: 0,
                    sha256: None,
                }
            } else {
                if before.nlink() != 1 {
                    return Err(Error::UnsupportedSource);
                }
                *bytes = bytes
                    .checked_add(before.len())
                    .ok_or(Error::UnsupportedSource)?;
                if *bytes > MAX_TREE_BYTES {
                    return Err(Error::UnsupportedSource);
                }
                let mut digest = Sha256::new();
                let mut length = 0;
                let mut buffer = [0; 64 * 1024];
                loop {
                    let count = opened.read(&mut buffer).map_err(|_| Error::SourceChanged)?;
                    if count == 0 {
                        break;
                    }
                    length += count as u64;
                    if length > before.len() {
                        return Err(Error::SourceChanged);
                    }
                    digest.update(&buffer[..count]);
                }
                reject_extra_permissions(&opened)?;
                let after = opened.metadata().map_err(|_| Error::SourceChanged)?;
                if length != before.len()
                    || Identity::read(&after)? != identity
                    || before.len() != after.len()
                    || before.mtime() != after.mtime()
                    || before.mtime_nsec() != after.mtime_nsec()
                    || before.ctime() != after.ctime()
                    || before.ctime_nsec() != after.ctime_nsec()
                {
                    return Err(Error::SourceChanged);
                }
                Node {
                    identity,
                    length,
                    sha256: Some(digest.finalize().into()),
                }
            };
            nodes.insert(path, node);
        }
        reject_extra_permissions(&self.file)?;
        if self.names()? != names {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }

    fn relative_parent(&self, path: &Path, create: bool) -> Result<(Self, OsString), Error> {
        let parts = path
            .components()
            .map(|part| match part {
                Component::Normal(value) => Ok(value.to_owned()),
                Component::Prefix(_)
                | Component::RootDir
                | Component::CurDir
                | Component::ParentDir => Err(Error::InvalidRelease),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (leaf, ancestors) = parts.split_last().ok_or(Error::InvalidRelease)?;
        let mut directory = Self {
            file: self.file.try_clone().map_err(|_| Error::SourceChanged)?,
        };
        for part in ancestors {
            directory = if create && !directory.has_child(part)? {
                directory.create(part)?
            } else {
                directory.child(part)?
            };
        }
        Ok((directory, leaf.clone()))
    }

    pub(super) fn write_new(
        &self,
        path: &Path,
        input: &mut dyn Read,
        length: u64,
        digest: [u8; 32],
    ) -> Result<(), Error> {
        let (parent, leaf) = self.relative_parent(path, true)?;
        let leaf = name(&leaf)?;
        let mut output = file(unsafe {
            libc::openat(
                parent.file.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        let mut hash = Sha256::new();
        let mut remaining = length;
        let mut buffer = [0; 64 * 1024];
        while remaining != 0 {
            let limit = remaining.min(buffer.len() as u64) as usize;
            let size = input
                .read(&mut buffer[..limit])
                .map_err(|_| Error::InvalidRelease)?;
            if size == 0 {
                return Err(Error::InvalidRelease);
            }
            hash.update(&buffer[..size]);
            output
                .write_all(&buffer[..size])
                .map_err(|_| Error::PersistenceFailed)?;
            remaining -= size as u64;
        }
        if <[u8; 32]>::from(hash.finalize()) != digest {
            return Err(Error::InvalidRelease);
        }
        output.sync_all().map_err(|_| Error::PersistenceFailed)?;
        parent.sync()
    }

    pub(super) fn replace_private_file(
        &self,
        path: &Path,
        source: &Path,
        length: u64,
        digest: [u8; 32],
    ) -> Result<(), Error> {
        let (parent, leaf) = self.relative_parent(path, false)?;
        let leaf_c = name(&leaf)?;
        // 只在尚未发布的私有新树内替换官方占位文件。
        if unsafe { libc::unlinkat(parent.file.as_raw_fd(), leaf_c.as_ptr(), 0) } != 0 {
            return Err(Error::PersistenceFailed);
        }
        let (source_parent, source_leaf) = self.relative_parent(source, false)?;
        let mut input = source_parent.read_file(&source_leaf)?;
        self.write_new(path, &mut input, length, digest)
    }

    pub(super) fn apply_permissions(
        &self,
        old: &Snapshot,
        executables: &BTreeMap<PathBuf, bool>,
    ) -> Result<(), Error> {
        let new = self.snapshot()?;
        for (path, node) in &new.nodes {
            let (parent, leaf) = self.relative_parent(path, false)?;
            let opened = parent.read_file(&leaf)?;
            let default_mode = if node.sha256.is_none() || executables.get(path) == Some(&true) {
                0o755
            } else {
                0o644
            };
            let (mode, gid) = old
                .nodes
                .get(path)
                .map_or((default_mode, old.root.gid), |old| {
                    (old.identity.mode & 0o777, old.identity.gid)
                });
            if unsafe { libc::fchown(opened.as_raw_fd(), old.root.uid, gid) } != 0
                || unsafe { libc::fchmod(opened.as_raw_fd(), mode as libc::mode_t) } != 0
            {
                return Err(Error::PersistenceFailed);
            }
            opened.sync_all().map_err(|_| Error::PersistenceFailed)?;
        }
        if unsafe { libc::fchown(self.file.as_raw_fd(), old.root.uid, old.root.gid) } != 0
            || unsafe {
                libc::fchmod(
                    self.file.as_raw_fd(),
                    (old.root.mode & 0o777) as libc::mode_t,
                )
            } != 0
        {
            return Err(Error::PersistenceFailed);
        }
        self.sync()
    }

    pub(super) fn exchange(&self, left: &OsStr, right: &OsStr) -> Result<(), Error> {
        let left = name(left)?;
        let right = name(right)?;
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                self.file.as_raw_fd(),
                left.as_ptr(),
                self.file.as_raw_fd(),
                right.as_ptr(),
                libc::RENAME_SWAP,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.file.as_raw_fd(),
                left.as_ptr(),
                self.file.as_raw_fd(),
                right.as_ptr(),
                libc::RENAME_EXCHANGE,
            )
        };
        if result != 0 {
            return Err(Error::PersistenceFailed);
        }
        self.sync()
    }

    pub(super) fn remove_matching(&self, leaf: &OsStr, expected: &Snapshot) -> Result<(), Error> {
        let directory = self.child(leaf)?;
        let current = directory.snapshot()?;
        // 清理中断后仅接受原清单的未变子集；新增或改写的文件不得继续删除。
        if current.root != expected.root
            || current
                .nodes
                .iter()
                .any(|(path, node)| expected.nodes.get(path) != Some(node))
        {
            return Err(Error::RecoveryRequired);
        }
        directory.remove_contents(expected, Path::new(""))?;
        if self.child(leaf)?.identity()? != expected.root {
            return Err(Error::RecoveryRequired);
        }
        let leaf = name(leaf)?;
        if unsafe { libc::unlinkat(self.file.as_raw_fd(), leaf.as_ptr(), libc::AT_REMOVEDIR) } != 0
        {
            return Err(Error::RecoveryRequired);
        }
        self.sync()
    }

    fn remove_contents(&self, expected: &Snapshot, relative: &Path) -> Result<(), Error> {
        let identity = if relative.as_os_str().is_empty() {
            &expected.root
        } else {
            &expected
                .nodes
                .get(relative)
                .ok_or(Error::RecoveryRequired)?
                .identity
        };
        if &self.identity()? != identity {
            return Err(Error::RecoveryRequired);
        }
        let names = self.names()?;
        // 整树检查与清理之间可能出现 npm 写入；只能删除原清单中仍然匹配的成员。
        if names
            .iter()
            .any(|leaf| !expected.nodes.contains_key(&relative.join(leaf)))
        {
            return Err(Error::RecoveryRequired);
        }
        for leaf in names {
            if &self.identity()? != identity {
                return Err(Error::RecoveryRequired);
            }
            let path = relative.join(&leaf);
            let node = expected.nodes.get(&path).ok_or(Error::RecoveryRequired)?;
            let mut opened = self.read_file(&leaf)?;
            reject_extra_permissions(&opened)?;
            let before = opened.metadata().map_err(|_| Error::RecoveryRequired)?;
            if Identity::read(&before)? != node.identity {
                return Err(Error::RecoveryRequired);
            }
            let flags = if before.is_dir() {
                Self {
                    file: opened.try_clone().map_err(|_| Error::RecoveryRequired)?,
                }
                .remove_contents(expected, &path)?;
                libc::AT_REMOVEDIR
            } else {
                if before.nlink() != 1 || before.len() != node.length {
                    return Err(Error::RecoveryRequired);
                }
                let mut digest = Sha256::new();
                let mut length = 0;
                let mut buffer = [0; 64 * 1024];
                loop {
                    let count = opened
                        .read(&mut buffer)
                        .map_err(|_| Error::RecoveryRequired)?;
                    if count == 0 {
                        break;
                    }
                    length += count as u64;
                    if length > node.length {
                        return Err(Error::RecoveryRequired);
                    }
                    digest.update(&buffer[..count]);
                }
                if length != node.length || Some(digest.finalize().into()) != node.sha256 {
                    return Err(Error::RecoveryRequired);
                }
                0
            };
            // 重新通过父目录打开成员，拒绝扫描后替换的 inode 或读取期间改写的文件。
            let current = self.read_file(&leaf)?;
            reject_extra_permissions(&current)?;
            let after = current.metadata().map_err(|_| Error::RecoveryRequired)?;
            if Identity::read(&after)? != node.identity
                || (!before.is_dir()
                    && (after.nlink() != 1
                        || after.len() != before.len()
                        || after.mtime() != before.mtime()
                        || after.mtime_nsec() != before.mtime_nsec()
                        || after.ctime() != before.ctime()
                        || after.ctime_nsec() != before.ctime_nsec()))
            {
                return Err(Error::RecoveryRequired);
            }
            let leaf = name(&leaf)?;
            if unsafe { libc::unlinkat(self.file.as_raw_fd(), leaf.as_ptr(), flags) } != 0 {
                return Err(Error::RecoveryRequired);
            }
        }
        if !self.names()?.is_empty() || &self.identity()? != identity {
            return Err(Error::RecoveryRequired);
        }
        self.sync()
    }

    pub(super) fn sync(&self) -> Result<(), Error> {
        self.file.sync_all().map_err(|_| Error::PersistenceFailed)
    }
}

#[cfg(test)]
#[path = "sources_npm_tree_unix_tests.rs"]
mod tests;
