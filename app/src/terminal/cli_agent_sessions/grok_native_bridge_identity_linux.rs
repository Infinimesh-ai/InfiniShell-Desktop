//! 普通 Linux PTY 桥只绑定 socket 的 pidfd 与实际运行映像，不取得进程控制权。

use std::fs::{File, Metadata};
use std::io::{self, Read as _};
use std::os::unix::fs::MetadataExt as _;

use command::managed::{LinuxProcessHandle, LinuxProcessIdentity, LinuxProcessSnapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::terminal::model::local_pty_identity::LocalPtyIdentity;

// 只能填入唯一构建工作流产出的定制 Linux ELF 摘要；空值拒绝所有映像。
pub(super) const ARTIFACT_SHA256: &str = "";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeBridgePlatform {
    Linux,
}

/// 此快照仅用于精确比对；反序列化不能取得 pidfd 或恢复输入租约。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBridgeProcess {
    pub platform: NativeBridgePlatform,
    pub pid: i32,
    pub start_time_ticks: u64,
    pub proc_inode: u64,
    pub pid_namespace_device: u64,
    pub pid_namespace_inode: u64,
    pub uid: u32,
    pub executable_device: u64,
    pub executable_inode: u64,
}

impl From<LinuxProcessIdentity> for NativeBridgeProcess {
    fn from(value: LinuxProcessIdentity) -> Self {
        Self {
            platform: NativeBridgePlatform::Linux,
            pid: value.pid,
            start_time_ticks: value.start_time_ticks,
            proc_inode: value.proc_inode,
            pid_namespace_device: value.pid_namespace_device,
            pid_namespace_inode: value.pid_namespace_inode,
            uid: value.uid,
            executable_device: value.executable_device,
            executable_inode: value.executable_inode,
        }
    }
}

/// 同时保留连接的原 pidfd 和经摘要验证的映像 FD，路径替换不能改变绑定对象。
pub(super) struct Artifact {
    process: LinuxProcessHandle,
    file: File,
    stamp: FileStamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FileStamp {
    device: u64,
    inode: u64,
    size: u64,
    uid: u32,
    mode: u32,
    links: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl FileStamp {
    pub(super) fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            uid: metadata.uid(),
            mode: metadata.mode(),
            links: metadata.nlink(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }

    pub(super) fn matches(self, metadata: &Metadata) -> bool {
        self == Self::of(metadata)
    }
}

impl Artifact {
    pub(super) fn capture(peer: &LinuxProcessHandle) -> io::Result<Self> {
        // 工件未绑定时先拒绝，不能将 manifest、运行路径或当前进程的摘要当白名单。
        validate_digest(ARTIFACT_SHA256)?;
        let process = peer.try_clone()?;
        let mut file = process.executable_file()?;
        let metadata = file.metadata()?;
        validate_artifact(&metadata)?;
        let stamp = FileStamp::of(&metadata);
        verify_digest(&mut file, stamp, ARTIFACT_SHA256)?;
        let artifact = Self {
            process,
            file,
            stamp,
        };
        artifact.verify(peer)?;
        Ok(artifact)
    }

    pub(super) fn verify(&self, peer: &LinuxProcessHandle) -> io::Result<()> {
        if peer.identity() != self.process.identity() {
            return Err(invalid());
        }
        peer.snapshot()?;
        // 原 pidfd 必须仍存活；再次打开实际 exe，排除 exec 与原映像已删除等变化。
        let current = self.process.executable_file()?;
        let metadata = current.metadata()?;
        validate_artifact(&metadata)?;
        if !self.stamp.matches(&metadata) || !self.stamp.matches(&self.file.metadata()?) {
            return Err(invalid());
        }
        peer.snapshot()?;
        self.process.snapshot()?;
        if !self.stamp.matches(&current.metadata()?) || !self.stamp.matches(&self.file.metadata()?)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn validate_digest(digest: &str) -> io::Result<()> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid());
    }
    Ok(())
}

fn verify_digest(file: &mut File, stamp: FileStamp, expected: &str) -> io::Result<()> {
    validate_digest(expected)?;
    if !stamp.matches(&file.metadata()?) {
        return Err(invalid());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut count = 0u64;
    loop {
        let length = file.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        count = count.checked_add(length as u64).ok_or_else(invalid)?;
        if count > stamp.size {
            return Err(invalid());
        }
        digest.update(&buffer[..length]);
    }
    if count != stamp.size
        || format!("{:x}", digest.finalize()) != expected
        || !stamp.matches(&file.metadata()?)
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_artifact(metadata: &Metadata) -> io::Result<()> {
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() == 0
        || (metadata.uid() != unsafe { libc::geteuid() } && metadata.uid() != 0)
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn verify_terminal(
    peer: &LinuxProcessHandle,
    pty: LocalPtyIdentity,
) -> io::Result<LinuxProcessIdentity> {
    let shell = pty.current_shell()?;
    let shell_handle = LinuxProcessHandle::capture(shell.pid)?;
    if shell_handle.identity() != shell {
        return Err(invalid());
    }
    let shell_terminal = shell_handle.snapshot()?;
    let terminal = peer.snapshot()?;
    if !terminal_matches(&terminal, &shell_terminal, pty.slave_device()) {
        return Err(invalid());
    }
    let shell_after = shell_handle.snapshot()?;
    let terminal_after = peer.snapshot()?;
    if !terminal_matches(&terminal_after, &shell_after, pty.slave_device())
        || terminal_after.session != terminal.session
        || terminal_after.process_group != terminal.process_group
        || pty.current_shell()? != shell
    {
        return Err(invalid());
    }
    Ok(shell)
}

fn terminal_matches(peer: &LinuxProcessSnapshot, shell: &LinuxProcessSnapshot, slave: u64) -> bool {
    let owner = unsafe { libc::geteuid() };
    slave != 0
        && peer.identity.uid == owner
        && shell.identity.uid == owner
        && peer.tty_device == slave
        && shell.tty_device == slave
        && peer.session > 0
        && peer.session == shell.session
        && peer.process_group > 0
        && peer.process_group == peer.foreground_group
        && shell.foreground_group == peer.foreground_group
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "grok_native_bridge.identity",
    )
}

#[cfg(test)]
#[path = "grok_native_bridge_identity_linux_tests.rs"]
mod tests;
