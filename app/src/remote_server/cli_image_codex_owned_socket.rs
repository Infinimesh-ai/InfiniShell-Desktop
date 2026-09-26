//! Codex 0.156.1 的 Unix rendezvous 链接与受保护物理 socket，不通用放行符号链接。

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SocketStamp {
    requested: PathBuf,
    physical: PathBuf,
    alias: (u64, u64),
    socket: (u64, u64),
    requested_parent: (u64, u64),
    physical_parent: (u64, u64),
}

pub(super) struct SocketLease {
    requested: PathBuf,
    physical: PathBuf,
    alias_identity: (u64, u64),
    socket_identity: (u64, u64),
    requested_parent: File,
    physical_parent: File,
}

impl SocketLease {
    /// 官方 protected_socket_path 对 canonicalize(parent)+filename 做 SHA256。
    /// uds::daemon_directory 使用固定 /tmp/codex-daemon-UID，不受 TMPDIR/HOME 影响。
    pub(super) fn capture(requested: &Path) -> io::Result<Self> {
        if !requested.is_absolute() {
            return Err(invalid());
        }
        let parent = requested.parent().ok_or_else(invalid)?;
        let requested = parent
            .canonicalize()?
            .join(requested.file_name().ok_or_else(invalid)?);
        let requested_parent = directory(requested.parent().ok_or_else(invalid)?)?;
        let root =
            fs::canonicalize("/tmp")?.join(format!("codex-daemon-{}", unsafe { libc::geteuid() }));
        let physical_parent = directory(&root)?;
        let physical = root.join(format!(
            "{:x}",
            Sha256::digest(requested.as_os_str().as_bytes())
        ));
        let alias = fs::symlink_metadata(&requested)?;
        let socket = fs::symlink_metadata(&physical)?;
        if !alias.file_type().is_symlink()
            || alias.uid() != unsafe { libc::geteuid() }
            || fs::read_link(&requested)? != physical
            || !socket.file_type().is_socket()
            || socket.uid() != alias.uid()
            || socket.mode() & 0o777 != 0o600
        {
            return Err(invalid());
        }
        Ok(Self {
            requested,
            physical,
            alias_identity: (alias.dev(), alias.ino()),
            socket_identity: (socket.dev(), socket.ino()),
            requested_parent,
            physical_parent,
        })
    }

    pub(super) fn requested(&self) -> &Path {
        &self.requested
    }
    pub(super) fn physical(&self) -> &Path {
        &self.physical
    }

    pub(super) fn stamp(&self) -> io::Result<SocketStamp> {
        self.validate()?;
        let requested = self.requested_parent.metadata()?;
        let physical = self.physical_parent.metadata()?;
        Ok(SocketStamp {
            requested: self.requested.clone(),
            physical: self.physical.clone(),
            alias: self.alias_identity,
            socket: self.socket_identity,
            requested_parent: (requested.dev(), requested.ino()),
            physical_parent: (physical.dev(), physical.ino()),
        })
    }

    pub(super) fn validate_stamp(&self, stamp: &SocketStamp) -> io::Result<()> {
        if self.stamp()? != *stamp {
            return Err(invalid());
        }
        Ok(())
    }

    pub(super) fn validate(&self) -> io::Result<()> {
        let alias = fs::symlink_metadata(&self.requested)?;
        let socket = fs::symlink_metadata(&self.physical)?;
        for (file, path) in [
            (&self.requested_parent, self.requested.parent()),
            (&self.physical_parent, self.physical.parent()),
        ] {
            let current = directory(path.ok_or_else(invalid)?)?.metadata()?;
            let pinned = file.metadata()?;
            if (current.dev(), current.ino()) != (pinned.dev(), pinned.ino()) {
                return Err(invalid());
            }
        }
        if !alias.file_type().is_symlink()
            || alias.uid() != unsafe { libc::geteuid() }
            || (alias.dev(), alias.ino()) != self.alias_identity
            || fs::read_link(&self.requested)? != self.physical
            || !socket.file_type().is_socket()
            || socket.uid() != alias.uid()
            || socket.mode() & 0o777 != 0o600
            || (socket.dev(), socket.ino()) != self.socket_identity
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn directory(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let current = fs::symlink_metadata(path)?;
    let pinned = file.metadata()?;
    if !current.is_dir()
        || current.uid() != unsafe { libc::geteuid() }
        || current.mode() & 0o777 != 0o700
        || (current.dev(), current.ino()) != (pinned.dev(), pinned.ino())
    {
        return Err(invalid());
    }
    Ok(file)
}

fn invalid() -> io::Error {
    io::Error::other("Codex 原生 socket 身份无效")
}
