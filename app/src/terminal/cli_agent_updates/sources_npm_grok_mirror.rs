//! Grok npm 同时维护用户 bin 中的版本映像和当前链接；不清理其他历史版本。

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::super::npm_grok_contract as contract;
use super::{Directory, Error, Identity, Node, name, reject_extra_permissions};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    target: OsString,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct Mirror {
    bin: PathBuf,
    directory: Identity,
    old_name: OsString,
    old_file: Node,
    new_name: OsString,
    existing_new: Option<Node>,
    file_stage: OsString,
    link_stage: OsString,
    old_link: Link,
    new_file: Option<Node>,
    new_link: Option<Link>,
}

fn file_node(directory: &Directory, leaf: &OsStr) -> Result<Node, Error> {
    let mut file = directory.read_file(leaf)?;
    reject_extra_permissions(&file)?;
    let before = file.metadata().map_err(|_| Error::SourceChanged)?;
    let identity = Identity::read(&before)?;
    if !before.is_file() || before.nlink() != 1 || before.len() > 1024 * 1024 * 1024 {
        return Err(Error::UnsupportedSource);
    }
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut bytes = [0; 64 * 1024];
    loop {
        let length = file.read(&mut bytes).map_err(|_| Error::SourceChanged)?;
        if length == 0 {
            break;
        }
        total += length as u64;
        if total > before.len() {
            return Err(Error::SourceChanged);
        }
        digest.update(&bytes[..length]);
    }
    let after = file.metadata().map_err(|_| Error::SourceChanged)?;
    if total != before.len()
        || Identity::read(&after)? != identity
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(Error::SourceChanged);
    }
    Ok(Node {
        identity,
        length: total,
        sha256: Some(digest.finalize().into()),
    })
}

fn link(directory: &Directory, leaf: &OsStr) -> Result<Link, Error> {
    let leaf = name(leaf)?;
    let read = || {
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                directory.file.as_raw_fd(),
                leaf.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(Error::SourceChanged);
        }
        let metadata = unsafe { metadata.assume_init() };
        if metadata.st_mode & libc::S_IFMT != libc::S_IFLNK
            || metadata.st_uid != unsafe { libc::geteuid() }
            || metadata.st_nlink != 1
        {
            return Err(Error::UnsupportedSource);
        }
        Ok((
            metadata.st_dev as u64,
            metadata.st_ino as u64,
            metadata.st_uid,
            metadata.st_gid,
            metadata.st_mode as u32,
        ))
    };
    let before = read()?;
    let mut target = [0; 128];
    let length = unsafe {
        libc::readlinkat(
            directory.file.as_raw_fd(),
            leaf.as_ptr(),
            target.as_mut_ptr().cast(),
            target.len(),
        )
    };
    if length <= 0 || length as usize >= target.len() || read()? != before {
        return Err(Error::SourceChanged);
    }
    let target = OsStr::from_bytes(&target[..length as usize]).to_owned();
    if !matches!(target.to_str(), Some("grok-1.0.40" | "grok-1.0.41")) {
        return Err(Error::UnsupportedSource);
    }
    Ok(Link {
        device: before.0,
        inode: before.1,
        uid: before.2,
        gid: before.3,
        mode: before.4,
        target,
    })
}

fn matches_native(node: &Node, version: &str) -> Result<(), Error> {
    let expected = contract::native(version)?;
    if node.length != expected.length
        || node.sha256 != Some(expected.digest()?)
        || node.identity.mode & 0o111 == 0
    {
        return Err(Error::SourceChanged);
    }
    Ok(())
}

impl Mirror {
    pub(in super::super) fn capture(home: &Path, old: &str, id: Uuid) -> Result<Self, Error> {
        contract::native(old)?;
        let bin = home.join("bin");
        let directory = Directory::open(&bin)?;
        let old_name: OsString = format!("grok-{old}").into();
        let new_name: OsString = format!("grok-{}", contract::VERSION).into();
        let old_link = link(&directory, OsStr::new("grok"))?;
        if old_link.target != old_name {
            return Err(Error::UnsupportedSource);
        }
        let old_file = file_node(&directory, &old_name)?;
        matches_native(&old_file, old)?;
        let existing_new = if directory.has_child(&new_name)? {
            let file = file_node(&directory, &new_name)?;
            matches_native(&file, contract::VERSION)?;
            Some(file)
        } else {
            None
        };
        Ok(Self {
            bin,
            directory: directory.identity()?,
            old_name,
            old_file,
            new_name,
            existing_new,
            file_stage: format!(".infinishell-grok-npm-image-{id}").into(),
            link_stage: format!(".infinishell-grok-npm-link-{id}").into(),
            old_link,
            new_file: None,
            new_link: None,
        })
    }

    pub(in super::super) fn validate(&self, home: &Path, old: &str, id: Uuid) -> Result<(), Error> {
        if self.bin != home.join("bin")
            || self.old_name != OsString::from(format!("grok-{old}"))
            || self.new_name != OsString::from(format!("grok-{}", contract::VERSION))
            || self.file_stage != OsString::from(format!(".infinishell-grok-npm-image-{id}"))
            || self.link_stage != OsString::from(format!(".infinishell-grok-npm-link-{id}"))
            || self.old_link.target != self.old_name
            || self
                .new_link
                .as_ref()
                .is_some_and(|link| link.target != self.new_name)
        {
            return Err(Error::RecoveryRequired);
        }
        matches_native(&self.old_file, old)?;
        for node in self.existing_new.iter().chain(self.new_file.iter()) {
            matches_native(node, contract::VERSION)?;
        }
        Ok(())
    }

    fn directory(&self) -> Result<Directory, Error> {
        let directory = Directory::open(&self.bin)?;
        if directory.identity()? != self.directory
            || file_node(&directory, &self.old_name)? != self.old_file
        {
            return Err(Error::SourceChanged);
        }
        Ok(directory)
    }

    pub(in super::super) fn prepare(&mut self, native: &mut File) -> Result<(), Error> {
        let directory = self.directory()?;
        if link(&directory, OsStr::new("grok"))? != self.old_link {
            return Err(Error::SourceChanged);
        }
        if let Some(expected) = &self.existing_new {
            if file_node(&directory, &self.new_name)? != *expected {
                return Err(Error::SourceChanged);
            }
            self.new_file = Some(expected.clone());
        } else {
            let expected = contract::native(contract::VERSION)?;
            directory.write_new(
                Path::new(&self.file_stage),
                native,
                expected.length,
                expected.digest()?,
            )?;
            let file = directory.read_file(&self.file_stage)?;
            // 新版本沿用原镜像的 POSIX 权限，不能因升级扩大组或其他用户的访问范围。
            let old = &self.old_file.identity;
            if unsafe { libc::fchown(file.as_raw_fd(), old.uid, old.gid) } != 0
                || unsafe { libc::fchmod(file.as_raw_fd(), (old.mode & 0o777) as libc::mode_t) }
                    != 0
            {
                return Err(Error::PersistenceFailed);
            }
            file.sync_all().map_err(|_| Error::PersistenceFailed)?;
            self.new_file = Some(file_node(&directory, &self.file_stage)?);
        }
        let target = name(&self.new_name)?;
        let stage = name(&self.link_stage)?;
        if unsafe { libc::symlinkat(target.as_ptr(), directory.file.as_raw_fd(), stage.as_ptr()) }
            != 0
        {
            return Err(Error::PersistenceFailed);
        }
        directory.sync()?;
        self.new_link = Some(link(&directory, &self.link_stage)?);
        Ok(())
    }

    pub(in super::super) fn publish(&self) -> Result<(), Error> {
        let directory = self.directory()?;
        let expected = self.new_file.as_ref().ok_or(Error::RecoveryRequired)?;
        if link(&directory, OsStr::new("grok"))? != self.old_link
            || Some(&link(&directory, &self.link_stage)?) != self.new_link.as_ref()
        {
            return Err(Error::SourceChanged);
        }
        if self.existing_new.is_none() {
            if file_node(&directory, &self.file_stage)? != *expected
                || directory.has_child(&self.new_name)?
            {
                return Err(Error::SourceChanged);
            }
            let stage = name(&self.file_stage)?;
            let target = name(&self.new_name)?;
            #[cfg(target_os = "macos")]
            let result = unsafe {
                libc::renameatx_np(
                    directory.file.as_raw_fd(),
                    stage.as_ptr(),
                    directory.file.as_raw_fd(),
                    target.as_ptr(),
                    libc::RENAME_EXCL,
                )
            };
            #[cfg(target_os = "linux")]
            let result = unsafe {
                libc::syscall(
                    libc::SYS_renameat2,
                    directory.file.as_raw_fd(),
                    stage.as_ptr(),
                    directory.file.as_raw_fd(),
                    target.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            };
            if result != 0 {
                return Err(Error::PersistenceFailed);
            }
            directory.sync()?;
        }
        if file_node(&directory, &self.new_name)? != *expected {
            return Err(Error::SourceChanged);
        }
        directory.exchange(OsStr::new("grok"), &self.link_stage)?;
        self.verify_published()
    }

    pub(in super::super) fn verify_published(&self) -> Result<(), Error> {
        let directory = self.directory()?;
        if Some(&link(&directory, OsStr::new("grok"))?) != self.new_link.as_ref()
            || Some(&file_node(&directory, &self.new_name)?) != self.new_file.as_ref()
        {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }

    pub(in super::super) fn finish(&self, committed: bool) -> Result<(), Error> {
        let directory = self.directory()?;
        let actual = link(&directory, OsStr::new("grok"))?;
        if committed {
            self.verify_published()?;
        } else if Some(&actual) == self.new_link.as_ref() {
            if link(&directory, &self.link_stage)? != self.old_link {
                return Err(Error::RecoveryRequired);
            }
            directory.exchange(OsStr::new("grok"), &self.link_stage)?;
        } else if actual != self.old_link {
            return Err(Error::RecoveryRequired);
        }
        if directory.has_child(&self.link_stage)? {
            // 尚未记录 inode 的私有链接保留现场，绝不凭路径删除。
            if let Some(new_link) = &self.new_link {
                let expected = if committed { &self.old_link } else { new_link };
                if link(&directory, &self.link_stage)? != *expected {
                    return Err(Error::RecoveryRequired);
                }
                unlink(&directory, &self.link_stage)?;
            }
        }
        if let Some(expected) = &self.new_file {
            for leaf in [&self.file_stage, &self.new_name] {
                if leaf == &self.new_name && (committed || self.existing_new.is_some()) {
                    continue;
                }
                if directory.has_child(leaf)? {
                    if file_node(&directory, leaf)? != *expected {
                        return Err(Error::RecoveryRequired);
                    }
                    unlink(&directory, leaf)?;
                }
            }
        }
        directory.sync()
    }
}

fn unlink(directory: &Directory, leaf: &OsStr) -> Result<(), Error> {
    let leaf = name(leaf)?;
    if unsafe { libc::unlinkat(directory.file.as_raw_fd(), leaf.as_ptr(), 0) } != 0 {
        return Err(Error::RecoveryRequired);
    }
    directory.sync()
}
