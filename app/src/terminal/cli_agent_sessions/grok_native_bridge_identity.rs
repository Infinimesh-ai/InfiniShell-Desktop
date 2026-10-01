//! 普通 PTY 桥的运行映像与内核身份核验；不授予 owned 进程权限。

use std::ffi::OsString;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read as _};
use std::os::unix::ffi::OsStringExt as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::PathBuf;
use std::ptr;

use command::managed::{
    MacosPeerHandle, MacosProcessIdentity, MacosTerminalSnapshot, macos_process_identity,
    macos_process_terminal,
};
use core_foundation::base::{CFType, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use security_framework_sys::code_signing::{
    SecCodeCheckValidity, SecCodeCopyGuestWithAttributes, SecRequirementCreateWithString,
    kSecGuestAttributeAudit,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::terminal::model::local_pty_identity::LocalPtyIdentity;

// 旧文本桥与受验 PNG 桥分别绑定完整映像和签名，不以版本字符串授权。
const ARTIFACTS: &[(&str, &str)] = &[
    (
        "edcdc3d8729cc657080e6a266e26a6590ec2b275f4545a5b93c1dd5e26bf08f1",
        "356fe77c29f339fcaa90e48094fc7af7e0571ec5",
    ),
    (
        "b5432ea1a6b4fec7d898de5f3d289ec55a838b70cb7797ecacaeb982f79ce444",
        "0bd055aaf74487889b084c04be1b73a31a1e85df",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBridgeProcess {
    pub pid: i32,
    pub pid_version: u32,
    pub unique_id: u64,
    pub resource_cid: u64,
}

impl From<MacosProcessIdentity> for NativeBridgeProcess {
    fn from(value: MacosProcessIdentity) -> Self {
        Self {
            pid: value.pid,
            pid_version: value.pid_version,
            unique_id: value.unique_id,
            resource_cid: value.resource_cid,
        }
    }
}

/// 摘要校验后保留 FD；每次使用仍核对路径与 FD，不能只信首次发现的路径。
pub(super) struct Artifact {
    path: PathBuf,
    file: File,
    stamp: FileStamp,
    sha256: &'static str,
    cdhash: &'static str,
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
    pub(super) fn capture(peer: &MacosPeerHandle) -> io::Result<Self> {
        verify_code(peer)?;
        let path = process_path(peer.identity())?;
        let before = fs::symlink_metadata(&path)?;
        validate_artifact(&before)?;
        let stamp = FileStamp::of(&before);
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&path)?;
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
            if count > before.len() {
                return Err(invalid());
            }
            digest.update(&buffer[..length]);
        }
        if count != before.len() {
            return Err(invalid());
        }
        let digest = format!("{:x}", digest.finalize());
        let &(sha256, cdhash) = ARTIFACTS
            .iter()
            .find(|(expected, _)| *expected == digest)
            .ok_or_else(invalid)?;
        let artifact = Self {
            path,
            file,
            stamp,
            sha256,
            cdhash,
        };
        artifact.verify(peer)?;
        Ok(artifact)
    }

    pub(super) fn sha256(&self) -> &'static str {
        self.sha256
    }

    pub(super) fn verify(&self, peer: &MacosPeerHandle) -> io::Result<()> {
        if process_path(peer.identity())? != self.path
            || !self.stamp.matches(&self.file.metadata()?)
            || !self.stamp.matches(&fs::symlink_metadata(&self.path)?)
        {
            return Err(invalid());
        }
        // 映像摘要和运行中签名必须来自同一受验工件，不能混搭白名单的两个版本。
        verify_code_requirement(peer, &format!("cdhash H\"{}\"", self.cdhash))?;
        if !self.stamp.matches(&self.file.metadata()?)
            || !self.stamp.matches(&fs::symlink_metadata(&self.path)?)
            || macos_process_identity(peer.identity().pid)? != peer.identity()
        {
            return Err(invalid());
        }
        Ok(())
    }
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

fn process_path(expected: MacosProcessIdentity) -> io::Result<PathBuf> {
    if macos_process_identity(expected.pid)? != expected {
        return Err(invalid());
    }
    let mut bytes = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let count =
        unsafe { libc::proc_pidpath(expected.pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    if count <= 0 || count as usize >= bytes.len() {
        return Err(invalid());
    }
    bytes.truncate(count as usize);
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    let path = PathBuf::from(OsString::from_vec(bytes));
    if !path.is_absolute()
        || fs::canonicalize(&path)? != path
        || macos_process_identity(expected.pid)? != expected
    {
        return Err(invalid());
    }
    Ok(path)
}

/// audit token 必须来自连接的内核凭据；PID 或 manifest 不能构造动态 SecCode 身份。
fn verify_code(peer: &MacosPeerHandle) -> io::Result<()> {
    let requirement = ARTIFACTS
        .iter()
        .map(|(_, cdhash)| format!("cdhash H\"{cdhash}\""))
        .collect::<Vec<_>>()
        .join(" or ");
    verify_code_requirement(peer, &requirement)
}

fn verify_code_requirement(peer: &MacosPeerHandle, requirement: &str) -> io::Result<()> {
    let bytes: Vec<u8> = peer
        .audit_token()
        .iter()
        .flat_map(|part| part.to_ne_bytes())
        .collect();
    let audit = CFData::from_buffer(&bytes);
    let key = unsafe { CFString::wrap_under_get_rule(kSecGuestAttributeAudit) };
    let attributes = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), audit.as_CFType())]);
    let mut code = ptr::null_mut();
    if unsafe {
        SecCodeCopyGuestWithAttributes(
            ptr::null_mut(),
            attributes.as_concrete_TypeRef(),
            0,
            &mut code,
        )
    } != 0
        || code.is_null()
    {
        return Err(invalid());
    }
    let code_guard = unsafe { CFType::wrap_under_create_rule(code.cast()) };
    let text = CFString::new(requirement);
    let mut requirement = ptr::null_mut();
    if unsafe { SecRequirementCreateWithString(text.as_concrete_TypeRef(), 0, &mut requirement) }
        != 0
        || requirement.is_null()
    {
        return Err(invalid());
    }
    let requirement_guard = unsafe { CFType::wrap_under_create_rule(requirement.cast()) };
    let status = unsafe { SecCodeCheckValidity(code, 0, requirement) };
    drop(requirement_guard);
    drop(code_guard);
    if status != 0 || macos_process_identity(peer.identity().pid)? != peer.identity() {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn verify_terminal(
    peer: MacosProcessIdentity,
    pty: LocalPtyIdentity,
) -> io::Result<MacosProcessIdentity> {
    let shell = pty.current_shell()?;
    let shell_terminal = macos_process_terminal(shell)?;
    let terminal = macos_process_terminal(peer)?;
    if !terminal_matches(&terminal, &shell_terminal, pty.slave_device())
        || pty.current_shell()? != shell
        || macos_process_identity(peer.pid)? != peer
    {
        return Err(invalid());
    }
    Ok(shell)
}

fn terminal_matches(
    peer: &MacosTerminalSnapshot,
    shell: &MacosTerminalSnapshot,
    slave: u64,
) -> bool {
    let owner = unsafe { libc::geteuid() };
    slave != 0
        && peer.uid == owner
        && shell.uid == owner
        && peer.tty_device == slave
        && shell.tty_device == slave
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
#[path = "grok_native_bridge_identity_tests.rs"]
mod tests;
