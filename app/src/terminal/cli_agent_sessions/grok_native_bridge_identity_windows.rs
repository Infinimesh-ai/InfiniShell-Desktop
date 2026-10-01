//! 普通 Windows Grok 的实际主映像和原 ConPTY 身份；不授予用户进程终止权限。

use std::fs::File;
use std::io;
use std::os::windows::fs::FileExt as _;
use std::os::windows::io::AsRawHandle as _;

use command::blocking::Command;
use command::managed::{WindowsProcessIdentity, WindowsProcessLease};
use command::managed_windows_image_observer::observe_process_image;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_ID_INFO,
    FILE_STANDARD_INFO, FileBasicInfo, FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx,
};

use crate::terminal::model::local_pty_identity::LocalPtyIdentity;

// 仅接受已核验的 Windows x64 定制 .11 工件，并通过实际主映像句柄核对摘要。
const ARTIFACT_SHA256S: &[&str] =
    &["acb9a34e9371285e1d5ce5f932dc4697e54927aea366048d361675075d4dbefc"];
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum NativeBridgePlatform {
    #[serde(rename = "windows")]
    Windows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBridgeProcess {
    pub platform: NativeBridgePlatform,
    pub pid: u32,
    pub created_at: u64,
    pub logon_low: u32,
    pub logon_high: i32,
    pub session_id: u32,
}

impl From<WindowsProcessIdentity> for NativeBridgeProcess {
    fn from(identity: WindowsProcessIdentity) -> Self {
        Self {
            platform: NativeBridgePlatform::Windows,
            pid: identity.pid,
            created_at: identity.created_at,
            logon_low: identity.logon_low,
            logon_high: identity.logon_high,
            session_id: identity.session_id,
        }
    }
}

pub(super) struct Artifact {
    process: WindowsProcessLease,
    file: File,
    stamp: FileStamp,
    sha256: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileStamp {
    volume: u64,
    id: [u8; 16],
    size: u64,
    links: u32,
    attributes: u32,
    created: i64,
    modified: i64,
    changed: i64,
}

impl FileStamp {
    fn read(file: &File) -> io::Result<Self> {
        let handle = HANDLE(file.as_raw_handle());
        let mut id = FILE_ID_INFO::default();
        let mut basic = FILE_BASIC_INFO::default();
        let mut standard = FILE_STANDARD_INFO::default();
        unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                (&mut id as *mut FILE_ID_INFO).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )?;
            GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )?;
            GetFileInformationByHandleEx(
                handle,
                FileStandardInfo,
                (&mut standard as *mut FILE_STANDARD_INFO).cast(),
                size_of::<FILE_STANDARD_INFO>() as u32,
            )?;
        }
        if standard.Directory
            || standard.DeletePending
            || standard.NumberOfLinks != 1
            || standard.EndOfFile <= 0
            || standard.EndOfFile as u64 > MAX_ARTIFACT_BYTES
            || basic.FileAttributes & (FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0)
                != 0
        {
            return Err(invalid());
        }
        Ok(Self {
            volume: id.VolumeSerialNumber,
            id: id.FileId.Identifier,
            size: standard.EndOfFile as u64,
            links: standard.NumberOfLinks,
            attributes: basic.FileAttributes,
            created: basic.CreationTime,
            modified: basic.LastWriteTime,
            changed: basic.ChangeTime,
        })
    }
}

impl Artifact {
    pub(super) fn capture(peer: &WindowsProcessLease) -> io::Result<Self> {
        peer.validate()?;
        let process = WindowsProcessLease::from_process_handle(peer)?;
        let mut bootstrap = Command::new(std::env::current_exe()?);
        bootstrap.arg("--grok-image-observer-bootstrap");
        let file = observe_process_image(peer, bootstrap)?;
        let stamp = FileStamp::read(&file)?;
        let sha256 = verify_digest(&file, stamp, ARTIFACT_SHA256S)?;
        process.validate()?;
        peer.validate()?;
        Ok(Self {
            process,
            file,
            stamp,
            sha256,
        })
    }

    pub(super) fn sha256(&self) -> &'static str {
        self.sha256
    }

    pub(super) fn verify(&self, peer: &WindowsProcessLease) -> io::Result<()> {
        self.process.validate()?;
        peer.validate()?;
        if self.process.identity() != peer.identity() {
            return Err(invalid());
        }
        verify_digest(&self.file, self.stamp, &[self.sha256])?;
        self.process.validate()?;
        peer.validate()
    }
}

fn validate_digest(expected: &str) -> io::Result<()> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid());
    }
    Ok(())
}

fn verify_digest<'a>(file: &File, stamp: FileStamp, expected: &[&'a str]) -> io::Result<&'a str> {
    for digest in expected {
        validate_digest(digest)?;
    }
    if FileStamp::read(file)? != stamp {
        return Err(invalid());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut offset = 0;
    while offset < stamp.size {
        let length = usize::try_from((stamp.size - offset).min(buffer.len() as u64))
            .map_err(|_| invalid())?;
        // 每次使用绝对偏移，不能让并发 verify 的 try_clone 共用文件游标。
        let count = file.seek_read(&mut buffer[..length], offset)?;
        if count == 0 {
            return Err(invalid());
        }
        digest.update(&buffer[..count]);
        offset += count as u64;
    }
    if FileStamp::read(file)? != stamp {
        return Err(invalid());
    }
    let digest = format!("{:x}", digest.finalize());
    expected
        .iter()
        .copied()
        .find(|expected| *expected == digest)
        .ok_or_else(invalid)
}

pub(super) fn verify_terminal(
    peer: &WindowsProcessLease,
    pty: &LocalPtyIdentity,
) -> io::Result<WindowsProcessIdentity> {
    let shell = pty.current_shell()?;
    peer.validate()?;
    // helper 从原 HPCON 创建；它查询自己的成员列表，不按 shell 当前 console 猜测归属。
    pty.spawn_console_probe(peer)?.wait()?;
    peer.validate()?;
    if pty.current_shell()? != shell {
        return Err(invalid());
    }
    Ok(shell)
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "grok_native_bridge.identity",
    )
}

#[cfg(test)]
#[path = "grok_native_bridge_identity_windows_tests.rs"]
mod tests;
