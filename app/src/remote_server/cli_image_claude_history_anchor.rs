//! 未落盘历史只记录受保护目录锚；创建和追加均由原生 Claude 完成。

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    ClaudeTranscriptIdentity, identity, is_absolute_normal_path, rejected, verify_file_path,
};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TranscriptAnchor {
    root: PathBuf,
    // 从配置根到当时最深既存父目录，后缀只能由精确 transcript 路径确定。
    directories: Vec<ClaudeTranscriptIdentity>,
}

pub(crate) struct TranscriptPathGuard {
    pub(crate) anchor: TranscriptAnchor,
    directories: Vec<File>,
    path: PathBuf,
    session: Uuid,
}

impl TranscriptPathGuard {
    pub(crate) fn capture(config: &Path, path: &Path, session: Uuid) -> io::Result<Self> {
        let initial = TranscriptAnchor {
            root: config.to_owned(),
            directories: Vec::new(),
        };
        let (anchor, directories, _) = initial.walk(path, session)?;
        Ok(Self {
            anchor,
            directories,
            path: path.to_owned(),
            session,
        })
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        let (current, _, _) = self.anchor.walk(&self.path, self.session)?;
        for (file, expected) in self.directories.iter().zip(&self.anchor.directories) {
            if identity(&file.metadata()?) != *expected {
                return Err(rejected("transcript_directory_changed"));
            }
        }
        if !current.directories.starts_with(&self.anchor.directories) {
            return Err(rejected("transcript_directory_changed"));
        }
        Ok(())
    }
}

impl TranscriptAnchor {
    pub(crate) fn extends(&self, earlier: &Self) -> bool {
        self.root == earlier.root
            && !earlier.directories.is_empty()
            && self.directories.starts_with(&earlier.directories)
    }
    /// 返回新文件及完整父目录身份；调用方必须先持久绑定，再读取任何历史内容。
    pub(crate) fn open(&self, path: &Path, session: Uuid) -> io::Result<(File, Self)> {
        let (complete, directories, ready) = self.walk(path, session)?;
        if !ready {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let parent = directories
            .last()
            .ok_or_else(|| rejected("missing_transcript_anchor"))?;
        let name = path
            .file_name()
            .ok_or_else(|| rejected("invalid_transcript_path"))?;
        let file = open_at(parent, name, false)?;
        verify_file_path(&file, path)?;
        complete.verify(path, session)?;
        Ok((file, complete))
    }

    pub(crate) fn verify(&self, path: &Path, session: Uuid) -> io::Result<()> {
        if self.directories.is_empty() {
            return Err(rejected("missing_transcript_anchor"));
        }
        self.walk(path, session).map(|_| ())
    }

    fn walk(&self, path: &Path, session: Uuid) -> io::Result<(Self, Vec<File>, bool)> {
        if session.is_nil()
            || !is_absolute_normal_path(&self.root)
            || !is_absolute_normal_path(path)
            || fs::canonicalize(&self.root)? != self.root
            || path.file_name().and_then(|name| name.to_str())
                != Some(format!("{session}.jsonl").as_str())
        {
            return Err(rejected("invalid_transcript_path"));
        }
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| rejected("invalid_transcript_path"))?;
        let components = relative.components().collect::<Vec<_>>();
        if components.len() < 3
            || components[0] != Component::Normal("projects".as_ref())
            || components
                .iter()
                .any(|part| !matches!(part, Component::Normal(_)))
            || self.directories.len() > components.len()
        {
            return Err(rejected("invalid_transcript_path"));
        }
        let root = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.root)?;
        let mut files = vec![root];
        let mut identities = Vec::new();
        let mut current = self.root.clone();
        let mut ready = true;
        for index in 0..components.len() {
            if index > 0 {
                let name = components[index - 1].as_os_str();
                current.push(name);
                match open_at(files.last().unwrap(), name, true) {
                    Ok(file) => files.push(file),
                    Err(error)
                        if error.kind() == io::ErrorKind::NotFound
                            && index >= self.directories.len() =>
                    {
                        ready = false;
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            let metadata = files.last().unwrap().metadata()?;
            let entry = fs::symlink_metadata(&current)?;
            // 配置目录兼容原生的可读权限，但不能由其他用户写入或经过链接。
            if !metadata.is_dir()
                || !entry.is_dir()
                || entry.file_type().is_symlink()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o022 != 0
                || identity(&entry) != identity(&metadata)
                || self
                    .directories
                    .get(index)
                    .is_some_and(|expected| *expected != identity(&metadata))
            {
                return Err(rejected("transcript_directory_changed"));
            }
            identities.push(identity(&metadata));
        }
        Ok((
            Self {
                root: self.root.clone(),
                directories: identities,
            },
            files,
            ready,
        ))
    }
}

fn open_at(parent: &File, name: &std::ffi::OsStr, directory: bool) -> io::Result<File> {
    let name = CString::new(name.as_bytes()).map_err(|_| rejected("invalid_transcript_path"))?;
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | if directory { libc::O_DIRECTORY } else { 0 };
    // 安全性：目录句柄在调用期间存活，名称是无 NUL 的单一路径分量。
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // 安全性：openat 成功产生的新句柄在此处唯一转交给 File。
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(test)]
#[path = "cli_image_claude_history_anchor_tests.rs"]
mod tests;
