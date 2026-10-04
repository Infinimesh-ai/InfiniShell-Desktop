//! npm 单包目录的 Unix 原子交换；所有相对操作锚定已打开的目录描述符。

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(target_os = "macos")]
use std::os::macos::fs::MetadataExt as _;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::Error;

#[path = "sources_npm_acl_unix.rs"]
mod acl;

#[path = "sources_npm_grok_mirror.rs"]
pub(super) mod grok_mirror;

const MAX_FILES: usize = 2048;
const MAX_TREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_TREE_ACL_BYTES: usize = 1024 * 1024;

#[cfg(test)]
fn claude_acl_fixture_files(platform: &str) -> Result<[PathBuf; 3], Error> {
    if !matches!(
        platform,
        "darwin-arm64" | "darwin-x64" | "linux-x64" | "linux-arm64" | "linux-x64-musl"
    ) {
        return Err(Error::UnsupportedPlatform);
    }
    Ok([
        PathBuf::from("bin/claude.exe"),
        PathBuf::from(format!(
            "node_modules/@anthropic-ai/claude-code-{platform}/claude"
        )),
        PathBuf::from("README.md"),
    ])
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acl: Option<acl::Acl>,
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
            acl: None,
        })
    }

    fn read_file(file: &File) -> Result<Self, Error> {
        reject_security_attributes(file)?;
        let before = file.metadata().map_err(|_| Error::SourceChanged)?;
        let mut identity = Self::read(&before)?;
        let captured = acl::capture(file).map_err(|_| Error::UnsupportedSource)?;
        let after = file.metadata().map_err(|_| Error::SourceChanged)?;
        if Self::read(&after)? != identity
            || before.len() != after.len()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
        {
            return Err(Error::SourceChanged);
        }
        identity.acl = captured.acl.into_optional();
        reject_security_attributes(file)?;
        Ok(identity)
    }

    pub(super) fn has_acl(&self) -> bool {
        self.acl.is_some()
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
    #[cfg(test)]
    pub(super) fn verify_claude_readonly_acl_fixture(&self, platform: &str) -> Result<(), Error> {
        if self.root.acl != acl::readonly_test_acl(true).into_optional() {
            return Err(Error::SourceChanged);
        }
        for path in claude_acl_fixture_files(platform)? {
            let node = self.nodes.get(&path).ok_or(Error::SourceChanged)?;
            let expected = acl::readonly_file_test_acl(node.identity.mode & 0o777)
                .map_err(|_| Error::UnsupportedSource)?;
            if node.sha256.is_none() || node.identity.acl != expected.into_optional() {
                return Err(Error::SourceChanged);
            }
        }
        Ok(())
    }

    pub(super) fn has_acl(&self) -> bool {
        self.root.has_acl() || self.nodes.values().any(|node| node.identity.has_acl())
    }

    fn verify_acl_budget(&self) -> Result<(), Error> {
        let mut bytes = 0_usize;
        for identity in
            std::iter::once(&self.root).chain(self.nodes.values().map(|node| &node.identity))
        {
            if let Some(acl) = &identity.acl {
                bytes += serde_json::to_vec(acl)
                    .map_err(|_| Error::UnsupportedSource)?
                    .len();
                if bytes > MAX_TREE_ACL_BYTES {
                    return Err(Error::UnsupportedSource);
                }
            }
        }
        Ok(())
    }

    pub(super) fn file_manifest(&self) -> BTreeMap<PathBuf, (u64, [u8; 32])> {
        self.nodes
            .iter()
            .filter_map(|(path, node)| {
                node.sha256
                    .map(|digest| (path.clone(), (node.length, digest)))
            })
            .collect()
    }

    pub(super) fn verify_file(
        &self,
        path: &Path,
        length: u64,
        digest: [u8; 32],
    ) -> Result<(), Error> {
        if self
            .nodes
            .get(path)
            .is_none_or(|node| node.length != length || node.sha256 != Some(digest))
        {
            return Err(Error::InvalidRelease);
        }
        Ok(())
    }

    pub(super) fn contains_claude_musl(&self) -> bool {
        self.nodes
            .keys()
            .any(|path| path.starts_with("node_modules/@anthropic-ai/claude-code-linux-x64-musl"))
    }

    /// 只从保存的完整树识别平台，不读取交换后的当前安装或宿主 libc。
    pub(super) fn claude_platform(&self) -> Result<String, Error> {
        let mut selected = None;
        for platform in [
            "darwin-arm64",
            "darwin-x64",
            "linux-arm64",
            "linux-x64",
            "linux-x64-musl",
        ] {
            let root = PathBuf::from(format!("node_modules/@anthropic-ai/claude-code-{platform}"));
            if self.nodes.contains_key(&root) {
                if selected.is_some()
                    || !self
                        .nodes
                        .get(&root.join("package.json"))
                        .is_some_and(|node| node.sha256.is_some())
                    || !self
                        .nodes
                        .get(&root.join("claude"))
                        .is_some_and(|node| node.sha256.is_some())
                {
                    return Err(Error::RecoveryRequired);
                }
                selected = Some(platform.to_owned());
            }
        }
        selected.ok_or(Error::RecoveryRequired)
    }

    pub(super) fn verify_remaining(&self, expected: &Self) -> Result<(), Error> {
        if self.root != expected.root
            || self
                .nodes
                .iter()
                .any(|(path, node)| expected.nodes.get(path) != Some(node))
        {
            return Err(Error::RecoveryRequired);
        }
        Ok(())
    }

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

/// 只为已审核 Claude 原生包接受官方 postinstall 的两名包内硬链接。
#[derive(Clone, Debug)]
pub(super) struct ClaudeHardlink {
    paths: [PathBuf; 2],
    length: u64,
    sha256: [u8; 32],
}

impl ClaudeHardlink {
    pub(super) fn new(platform: &str, length: u64, sha256: [u8; 32]) -> Result<Self, Error> {
        if !matches!(platform, "darwin-arm64" | "linux-x64" | "linux-x64-musl") {
            return Err(Error::UnsupportedPlatform);
        }
        Ok(Self {
            paths: [
                "bin/claude.exe".into(),
                format!("node_modules/@anthropic-ai/claude-code-{platform}/claude").into(),
            ],
            length,
            sha256,
        })
    }

    fn contains(&self, path: &Path) -> bool {
        self.paths.iter().any(|member| member == path)
    }

    fn linked_node<'a>(&self, snapshot: &'a Snapshot) -> Option<&'a Node> {
        let first = snapshot.nodes.get(&self.paths[0])?;
        let second = snapshot.nodes.get(&self.paths[1])?;
        (first.sha256.is_some() && first == second).then_some(first)
    }

    fn verify_image(&self, node: &Node) -> Result<(), Error> {
        if node.length != self.length || node.sha256 != Some(self.sha256) {
            return Err(Error::UnsupportedSource);
        }
        Ok(())
    }
}

pub(super) struct Directory {
    file: File,
}

/// Grok 官方 postinstall 唯一包内链接；记录链接自身，不跟随它读取或删除。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GrokLink {
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GrokSnapshot {
    pub(super) tree: Snapshot,
    pub(super) link: Option<GrokLink>,
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

// 未保存 ACL 的独立消费者继续拒绝额外权限，不能仅改检查就静默丢失它们。
#[cfg(target_os = "macos")]
pub(super) fn reject_extra_permissions(file: &File) -> Result<(), Error> {
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
pub(super) fn reject_extra_permissions(file: &File) -> Result<(), Error> {
    reject_untracked_xattrs(file, false)
}

#[cfg(target_os = "macos")]
fn reject_security_attributes(file: &File) -> Result<(), Error> {
    // 文件标志会改变替换和清理权限；此增量只支持 ACL，不清除现有标志。
    if file
        .metadata()
        .map_err(|_| Error::SourceChanged)?
        .st_flags()
        != 0
    {
        return Err(Error::UnsupportedSource);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn reject_security_attributes(file: &File) -> Result<(), Error> {
    reject_untracked_xattrs(file, true)
}

#[cfg(target_os = "linux")]
fn reject_untracked_xattrs(file: &File, allow_acl: bool) -> Result<(), Error> {
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
        (name.starts_with(b"system.posix_acl_")
            && (!allow_acl
                || (name != b"system.posix_acl_access" && name != b"system.posix_acl_default")))
            || name.starts_with(b"security.")
            || name.starts_with(b"trusted.")
    }) {
        return Err(Error::UnsupportedSource);
    }
    Ok(())
}

impl Directory {
    /// 仅验收入口在校验本轮新建私有 fixture 后调用；恢复分支不得重设 ACL。
    #[cfg(test)]
    pub(super) fn install_claude_readonly_acl_fixture(&self, platform: &str) -> Result<(), Error> {
        let before = self.snapshot()?;
        if before.has_acl() {
            return Err(Error::SourceChanged);
        }
        let mut files = Vec::new();
        let mut inodes = std::collections::BTreeSet::new();
        for path in claude_acl_fixture_files(platform)? {
            let node = before.nodes.get(&path).ok_or(Error::SourceChanged)?;
            let (parent, leaf) = self.relative_parent(&path, false)?;
            let opened = parent.read_file(&leaf)?;
            if node.sha256.is_none()
                || Identity::read_file(&opened)? != node.identity
                || opened.metadata().map_err(|_| Error::SourceChanged)?.nlink() != 1
                || !inodes.insert((node.identity.device, node.identity.inode))
            {
                return Err(Error::SourceChanged);
            }
            files.push((opened, node));
        }
        // Python write_members/copytree 生成独立 inode；不能为硬链接放宽生产 apply。
        for (file, node) in files {
            let mode = node.identity.mode & 0o777;
            let desired =
                acl::readonly_file_test_acl(mode).map_err(|_| Error::UnsupportedSource)?;
            Self::apply_new_permissions(
                &file,
                &node.identity,
                node.identity.uid,
                node.identity.gid,
                mode,
                &desired,
            )?;
        }
        Self::apply_new_permissions(
            &self.file,
            &before.root,
            before.root.uid,
            before.root.gid,
            before.root.mode & 0o777,
            &acl::readonly_test_acl(true),
        )?;
        self.snapshot()?
            .verify_claude_readonly_acl_fixture(platform)
    }

    #[cfg(target_os = "macos")]
    fn reject_link_acl(&self, leaf: &CStr, device: u64, inode: u64) -> Result<(), Error> {
        // O_SYMLINK 单独保留链接 vnode；并用 O_NOFOLLOW 会被 XNU 以 ELOOP 拒绝。
        let opened = file(unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_RDONLY | libc::O_SYMLINK | libc::O_CLOEXEC,
            )
        })?;
        let metadata = opened.metadata().map_err(|_| Error::SourceChanged)?;
        if !metadata.file_type().is_symlink() || metadata.dev() != device || metadata.ino() != inode
        {
            return Err(Error::SourceChanged);
        }
        // 链接 ACL 的原生写入支持尚未验收；非空和无法读取都拒绝，绝不丢弃。
        reject_extra_permissions(&opened)
    }

    fn grok_link(&self, leaf: &OsStr) -> Result<GrokLink, Error> {
        let leaf = name(leaf)?;
        let read = || {
            let mut value = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    self.file.as_raw_fd(),
                    leaf.as_ptr(),
                    value.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(Error::SourceChanged);
            }
            let value = unsafe { value.assume_init() };
            if value.st_mode & libc::S_IFMT != libc::S_IFLNK
                || value.st_uid != unsafe { libc::geteuid() }
                || value.st_nlink != 1
            {
                return Err(Error::UnsupportedSource);
            }
            Ok(GrokLink {
                device: value.st_dev as u64,
                inode: value.st_ino as u64,
                uid: value.st_uid,
                gid: value.st_gid,
                mode: value.st_mode as u32,
            })
        };
        let before = read()?;
        #[cfg(target_os = "macos")]
        self.reject_link_acl(&leaf, before.device, before.inode)?;
        let mut target = [0_u8; 64];
        let length = unsafe {
            libc::readlinkat(
                self.file.as_raw_fd(),
                leaf.as_ptr(),
                target.as_mut_ptr().cast(),
                target.len(),
            )
        };
        if length < 0
            || target.get(..length as usize) != Some(b"./grok-native".as_slice())
            || read()? != before
        {
            return Err(Error::SourceChanged);
        }
        Ok(before)
    }

    pub(super) fn grok_snapshot(&self) -> Result<GrokSnapshot, Error> {
        let root = self.identity()?;
        let mut nodes = BTreeMap::new();
        let mut link = None;
        self.snapshot_into_grok(Path::new(""), &mut nodes, &mut 0, &mut link, true, None)?;
        if self.identity()? != root {
            return Err(Error::SourceChanged);
        }
        if let Some(expected) = &link
            && self
                .child(OsStr::new("bin"))?
                .grok_link(OsStr::new("grok"))?
                != *expected
        {
            return Err(Error::SourceChanged);
        }
        let tree = Snapshot { root, nodes };
        tree.verify_acl_budget()?;
        Ok(GrokSnapshot { tree, link })
    }

    pub(super) fn create_grok_link(&self) -> Result<(), Error> {
        let bin = self.child(OsStr::new("bin"))?;
        let native = bin.read_file(OsStr::new("grok-native"))?;
        Identity::read_file(&native)?;
        if unsafe {
            libc::symlinkat(
                c"./grok-native".as_ptr(),
                bin.file.as_raw_fd(),
                c"grok".as_ptr(),
            )
        } != 0
        {
            return Err(Error::PersistenceFailed);
        }
        bin.grok_link(OsStr::new("grok"))?;
        bin.sync()
    }

    pub(super) fn apply_grok_permissions(
        &self,
        old: &Snapshot,
        executables: &BTreeMap<PathBuf, bool>,
    ) -> Result<(), Error> {
        self.apply_snapshot_permissions(old, executables, self.grok_snapshot()?.tree)
    }

    pub(super) fn remove_grok_matching(
        &self,
        leaf: &OsStr,
        expected: &GrokSnapshot,
    ) -> Result<(), Error> {
        let directory = self.child(leaf)?;
        let current = directory.grok_snapshot()?;
        if current.tree.root != expected.tree.root
            || current
                .tree
                .nodes
                .iter()
                .any(|(path, node)| expected.tree.nodes.get(path) != Some(node))
            || current
                .link
                .as_ref()
                .is_some_and(|link| Some(link) != expected.link.as_ref())
        {
            return Err(Error::RecoveryRequired);
        }
        if let Some(link) = current.link {
            let bin = directory.child(OsStr::new("bin"))?;
            if bin.grok_link(OsStr::new("grok"))? != link
                || unsafe { libc::unlinkat(bin.file.as_raw_fd(), c"grok".as_ptr(), 0) } != 0
            {
                return Err(Error::RecoveryRequired);
            }
            bin.sync()?;
        }
        self.remove_matching(leaf, &expected.tree)
    }

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
        Identity::read_file(&self.file)
    }

    /// Homebrew 会创建 0775 的 Caskroom；仅私有前缀隔离下接纳该固定父目录。
    /// 包目录和文件仍使用严格的 identity/snapshot，不继承父目录的例外。
    pub(super) fn homebrew_caskroom(&self) -> Result<(Self, Identity), Error> {
        let prefix = self.identity()?;
        let directory = self.child(OsStr::new("Caskroom"))?;
        reject_extra_permissions(&directory.file)?;
        let metadata = directory
            .file
            .metadata()
            .map_err(|_| Error::SourceChanged)?;
        let identity = match Identity::read(&metadata) {
            Ok(identity) => identity,
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            Err(Error::UnsupportedSource)
                if prefix.mode & 0o7777 == 0o700
                    && metadata.is_dir()
                    && metadata.uid() == prefix.uid
                    && metadata.dev() == prefix.device
                    && metadata.mode() & 0o7777 == 0o775 =>
            {
                Identity {
                    device: metadata.dev(),
                    inode: metadata.ino(),
                    uid: metadata.uid(),
                    gid: metadata.gid(),
                    mode: metadata.mode(),
                    acl: None,
                }
            }
            Err(error) => return Err(error),
        };
        if self.identity()? != prefix {
            return Err(Error::SourceChanged);
        }
        Ok((directory, identity))
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

    /// 清单沿原目录描述符读取；不追随中间目录或文件链接。
    pub(super) fn read_manifest(&self, relative: &Path) -> Result<Vec<u8>, Error> {
        let (parent, leaf) = self.relative_parent(relative, false)?;
        let mut opened = parent.read_file(&leaf)?;
        let before = opened.metadata().map_err(|_| Error::SourceChanged)?;
        let identity = Identity::read_file(&opened)?;
        if !before.is_file() || before.nlink() != 1 || before.len() > 64 * 1024 {
            return Err(Error::UnsupportedSource);
        }
        let mut bytes = Vec::new();
        (&mut opened)
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::SourceChanged)?;
        let after = opened.metadata().map_err(|_| Error::SourceChanged)?;
        if bytes.len() as u64 != before.len()
            || after.len() != before.len()
            || Identity::read_file(&opened)? != identity
        {
            return Err(Error::SourceChanged);
        }
        Ok(bytes)
    }

    pub(super) fn snapshot(&self) -> Result<Snapshot, Error> {
        let root = self.identity()?;
        let mut nodes = BTreeMap::new();
        let mut bytes = 0;
        self.snapshot_into(Path::new(""), &mut nodes, &mut bytes)?;
        if self.identity()? != root {
            return Err(Error::SourceChanged);
        }
        let snapshot = Snapshot { root, nodes };
        snapshot.verify_acl_budget()?;
        Ok(snapshot)
    }

    pub(super) fn claude_snapshot(&self, link: &ClaudeHardlink) -> Result<Snapshot, Error> {
        self.claude_snapshot_remaining(link, None)
    }

    fn claude_snapshot_remaining(
        &self,
        link: &ClaudeHardlink,
        expected: Option<&Snapshot>,
    ) -> Result<Snapshot, Error> {
        let root = self.identity()?;
        let mut nodes = BTreeMap::new();
        self.snapshot_into_grok(
            Path::new(""),
            &mut nodes,
            &mut 0,
            &mut None,
            false,
            Some(link),
        )?;
        let snapshot = Snapshot { root, nodes };
        snapshot.verify_acl_budget()?;
        let linked = expected
            .and_then(|old| link.linked_node(old))
            .or_else(|| link.linked_node(&snapshot));
        if let Some(node) = linked {
            link.verify_image(node)?;
            if node.identity.device != snapshot.root.device {
                return Err(Error::UnsupportedSource);
            }
        }
        // 用原根描述符重新打开两名；nlink 必须恰等于本包仍持有的已知名称数。
        for path in &link.paths {
            if let Some(node) = snapshot.nodes.get(path) {
                let count = if linked.is_some_and(|old| old.identity == node.identity) {
                    link.paths
                        .iter()
                        .filter(|member| snapshot.nodes.get(*member) == Some(node))
                        .count() as u64
                } else {
                    1
                };
                let (parent, leaf) = self.relative_parent(path, false)?;
                let opened = parent.read_file(&leaf)?;
                let metadata = opened.metadata().map_err(|_| Error::SourceChanged)?;
                if Identity::read_file(&opened)? != node.identity
                    || metadata.len() != node.length
                    || metadata.nlink() != count
                {
                    return Err(Error::SourceChanged);
                }
            }
        }
        if self.identity()? != snapshot.root {
            return Err(Error::SourceChanged);
        }
        Ok(snapshot)
    }

    fn snapshot_into(
        &self,
        relative: &Path,
        nodes: &mut BTreeMap<PathBuf, Node>,
        bytes: &mut u64,
    ) -> Result<(), Error> {
        self.snapshot_into_grok(relative, nodes, bytes, &mut None, false, None)
    }

    fn snapshot_into_grok(
        &self,
        relative: &Path,
        nodes: &mut BTreeMap<PathBuf, Node>,
        bytes: &mut u64,
        link: &mut Option<GrokLink>,
        grok: bool,
        claude: Option<&ClaudeHardlink>,
    ) -> Result<(), Error> {
        if relative.components().count() > 32 {
            return Err(Error::UnsupportedSource);
        }
        let parent_identity = self.identity()?;
        let names = self.names()?;
        for leaf in &names {
            if nodes.len() >= MAX_FILES {
                return Err(Error::UnsupportedSource);
            }
            let path = relative.join(leaf);
            if grok && path == Path::new("bin/grok") {
                *link = Some(self.grok_link(leaf)?);
                continue;
            }
            let mut opened = self.read_file(leaf)?;
            let before = opened.metadata().map_err(|_| Error::SourceChanged)?;
            let identity = Identity::read_file(&opened)?;
            let node = if before.is_dir() {
                let child = Self { file: opened };
                child.snapshot_into_grok(&path, nodes, bytes, link, grok, claude)?;
                if child.identity()? != identity {
                    return Err(Error::SourceChanged);
                }
                Node {
                    identity,
                    length: 0,
                    sha256: None,
                }
            } else {
                if before.nlink() != 1
                    && !(before.nlink() == 2 && claude.is_some_and(|link| link.contains(&path)))
                {
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
                let after = opened.metadata().map_err(|_| Error::SourceChanged)?;
                if length != before.len()
                    || Identity::read_file(&opened)? != identity
                    || before.len() != after.len()
                    || before.mtime() != after.mtime()
                    || before.mtime_nsec() != after.mtime_nsec()
                    || before.ctime() != after.ctime()
                    || before.ctime_nsec() != after.ctime_nsec()
                    || claude.is_some() && before.nlink() != after.nlink()
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
        if self.identity()? != parent_identity || self.names()? != names {
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
        let created = Identity::read_file(&output)?;
        if output.metadata().map_err(|_| Error::SourceChanged)?.nlink() != 1 {
            return Err(Error::SourceChanged);
        }
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
        if Identity::read_file(&output)? != created
            || output.metadata().map_err(|_| Error::SourceChanged)?.nlink() != 1
        {
            return Err(Error::SourceChanged);
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
        self.apply_snapshot_permissions(old, executables, new)
    }

    fn apply_snapshot_permissions(
        &self,
        old: &Snapshot,
        executables: &BTreeMap<PathBuf, bool>,
        new: Snapshot,
    ) -> Result<(), Error> {
        // 只修改本轮新树；旧对象即使内容相同也不能借此重设权限。
        let old_inodes = std::iter::once(&old.root)
            .chain(old.nodes.values().map(|node| &node.identity))
            .map(|identity| (identity.device, identity.inode))
            .collect::<std::collections::BTreeSet<_>>();
        if std::iter::once(&new.root)
            .chain(new.nodes.values().map(|node| &node.identity))
            .any(|identity| old_inodes.contains(&(identity.device, identity.inode)))
        {
            return Err(Error::SourceChanged);
        }
        let mut permissions = BTreeMap::new();
        permissions.insert(
            PathBuf::new(),
            (
                old.root.mode & 0o777,
                old.root.gid,
                old.root.acl.clone().unwrap_or_else(acl::Acl::absent),
            ),
        );
        // BTreeMap 的路径顺序先父后子；新节点按最终父 ACL 继承，不能沿用临时 stage 的继承。
        for (path, node) in &new.nodes {
            let default_mode = if node.sha256.is_none() || executables.get(path) == Some(&true) {
                0o755
            } else {
                0o644
            };
            let desired = if let Some(previous) = old.nodes.get(path) {
                if previous.identity.has_acl() && previous.sha256.is_none() != node.sha256.is_none()
                {
                    return Err(Error::UnsupportedSource);
                }
                (
                    previous.identity.mode & 0o777,
                    previous.identity.gid,
                    previous
                        .identity
                        .acl
                        .clone()
                        .unwrap_or_else(acl::Acl::absent),
                )
            } else {
                let parent = path.parent().ok_or(Error::InvalidRelease)?;
                let (_, _, parent_acl) = permissions.get(parent).ok_or(Error::InvalidRelease)?;
                let (inherited, mode) = parent_acl
                    .inherit(node.sha256.is_none(), default_mode)
                    .map_err(|_| Error::UnsupportedSource)?;
                (mode, old.root.gid, inherited)
            };
            permissions.insert(path.clone(), desired);
        }
        permissions
            .values()
            .try_fold(0_usize, |total, (_, _, acl)| {
                let bytes = serde_json::to_vec(acl).map_err(|_| Error::UnsupportedSource)?;
                let total = total + bytes.len();
                if total > MAX_TREE_ACL_BYTES {
                    Err(Error::UnsupportedSource)
                } else {
                    Ok(total)
                }
            })?;
        let mut paths = new.nodes.keys().collect::<Vec<_>>();
        paths.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        // 先完成孩子，再应用父目录的 deny；不为遍历或清理临时移除旧 ACL。
        for path in paths {
            let (parent, leaf) = self.relative_parent(path, false)?;
            let opened = parent.read_file(&leaf)?;
            let (mode, gid, desired) = permissions.get(path).ok_or(Error::InvalidRelease)?;
            Self::apply_new_permissions(
                &opened,
                &new.nodes[path].identity,
                old.root.uid,
                *gid,
                *mode,
                desired,
            )?;
        }
        let (mode, gid, desired) = permissions
            .get(Path::new(""))
            .ok_or(Error::InvalidRelease)?;
        Self::apply_new_permissions(&self.file, &new.root, old.root.uid, *gid, *mode, desired)?;
        self.sync()
    }

    fn apply_new_permissions(
        file: &File,
        expected: &Identity,
        uid: u32,
        gid: u32,
        mode: u32,
        desired: &acl::Acl,
    ) -> Result<(), Error> {
        if &Identity::read_file(file)? != expected {
            return Err(Error::SourceChanged);
        }
        // 快照之后新增的硬链接也必须在 chmod/chown 之前拒绝，避免改到树外别名。
        let metadata = file.metadata().map_err(|_| Error::SourceChanged)?;
        if metadata.is_file() && metadata.nlink() != 1 {
            return Err(Error::SourceChanged);
        }
        if unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } != 0
            || unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } != 0
        {
            return Err(Error::PersistenceFailed);
        }
        let captured = acl::capture(file).map_err(|_| Error::UnsupportedSource)?;
        acl::apply_to_new(file, &captured, desired).map_err(|_| Error::PersistenceFailed)?;
        let actual = Identity::read_file(file)?;
        if actual.device != expected.device
            || actual.inode != expected.inode
            || actual.uid != uid
            || actual.gid != gid
            || actual.mode & 0o777 != mode
            || actual.acl != desired.clone().into_optional()
        {
            return Err(Error::SourceChanged);
        }
        file.sync_all().map_err(|_| Error::PersistenceFailed)
    }

    /// 跨 cask 改名只锚定本目录 fd，目标已存在时绝不覆盖。
    #[cfg(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64")
    ))]
    pub(super) fn rename_noreplace(&self, from: &OsStr, to: &OsStr) -> Result<(), Error> {
        let from = name(from)?;
        let to = name(to)?;
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                self.file.as_raw_fd(),
                from.as_ptr(),
                self.file.as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.file.as_raw_fd(),
                from.as_ptr(),
                self.file.as_raw_fd(),
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(Error::SourceChanged);
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
        self.remove_matching_inner(leaf, expected, None)
    }

    pub(super) fn remove_claude_matching(
        &self,
        leaf: &OsStr,
        expected: &Snapshot,
        link: &ClaudeHardlink,
    ) -> Result<(), Error> {
        self.remove_matching_inner(leaf, expected, Some(link))
    }

    fn remove_matching_inner(
        &self,
        leaf: &OsStr,
        expected: &Snapshot,
        link: Option<&ClaudeHardlink>,
    ) -> Result<(), Error> {
        let directory = self.child(leaf)?;
        let current = match link {
            Some(link) => directory.claude_snapshot_remaining(link, Some(expected))?,
            None => directory.snapshot()?,
        };
        // 清理中断后仅接受原清单的未变子集；新增或改写的文件不得继续删除。
        current.verify_remaining(expected)?;
        let mut remaining = current.nodes;
        directory.remove_contents_inner(expected, Path::new(""), link, &mut remaining)?;
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

    #[cfg(test)]
    fn remove_contents(&self, expected: &Snapshot, relative: &Path) -> Result<(), Error> {
        self.remove_contents_inner(expected, relative, None, &mut expected.nodes.clone())
    }

    fn remove_contents_inner(
        &self,
        expected: &Snapshot,
        relative: &Path,
        link: Option<&ClaudeHardlink>,
        remaining: &mut BTreeMap<PathBuf, Node>,
    ) -> Result<(), Error> {
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
            let before = opened.metadata().map_err(|_| Error::RecoveryRequired)?;
            if Identity::read_file(&opened)? != node.identity {
                return Err(Error::RecoveryRequired);
            }
            let flags = if before.is_dir() {
                Self {
                    file: opened.try_clone().map_err(|_| Error::RecoveryRequired)?,
                }
                .remove_contents_inner(expected, &path, link, remaining)?;
                libc::AT_REMOVEDIR
            } else {
                let links = Self::remaining_claude_links(link, expected, remaining, &path, node)?;
                if before.nlink() != links || before.len() != node.length {
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
            let after = current.metadata().map_err(|_| Error::RecoveryRequired)?;
            if Identity::read_file(&current)? != node.identity
                || (!before.is_dir()
                    && (after.nlink()
                        != Self::remaining_claude_links(link, expected, remaining, &path, node)?
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
            remaining.remove(&path);
        }
        if !self.names()?.is_empty() || &self.identity()? != identity {
            return Err(Error::RecoveryRequired);
        }
        self.sync()
    }

    fn remaining_claude_links(
        link: Option<&ClaudeHardlink>,
        expected: &Snapshot,
        remaining: &BTreeMap<PathBuf, Node>,
        path: &Path,
        node: &Node,
    ) -> Result<u64, Error> {
        let Some(link) = link else { return Ok(1) };
        let Some(linked) = link.linked_node(expected) else {
            return Ok(1);
        };
        link.verify_image(linked)?;
        if !link.contains(path) || linked.identity != node.identity {
            return Ok(1);
        }
        Ok(link
            .paths
            .iter()
            .filter(|member| remaining.get(*member) == Some(node))
            .count() as u64)
    }

    pub(super) fn sync(&self) -> Result<(), Error> {
        self.file.sync_all().map_err(|_| Error::PersistenceFailed)
    }
}

#[cfg(test)]
#[path = "sources_npm_tree_unix_tests.rs"]
mod tests;
