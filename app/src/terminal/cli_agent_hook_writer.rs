//! 一次性 Grok 通知发送入口；只串行写完整 OSC，不保存回合或投递状态。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use fs4::fs_std::FileExt as _;
use serde::{Deserialize, Serialize};
use warp_core::cli_agent_protocol::{CLI_AGENT_NOTIFICATION_SENTINEL, WARP_CLI_AGENT_TTY_ENV};

const MAX_FRAME_BYTES: usize = 4096;
const MAX_ID_BYTES: usize = 256;
const IO_TIMEOUT: Duration = Duration::from_millis(500);
const SEND_TIMEOUT: Duration = Duration::from_millis(1600);
const RETRY_DELAY: Duration = Duration::from_millis(5);
const PROTOCOL_REPLY: &[u8] = b"{\"protocol\":1,\"maxFrameBytes\":4096}\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum HookWriteError {
    #[error("cli_agent_notify_invalid_payload")]
    InvalidPayload,
    #[error("cli_agent_notify_frame_too_large")]
    FrameTooLarge,
    #[error("cli_agent_notify_input_unavailable")]
    InputUnavailable,
    #[error("cli_agent_notify_input_timeout")]
    InputTimeout,
    #[error("cli_agent_notify_terminal_unavailable")]
    TerminalUnavailable,
    #[error("cli_agent_notify_tmux_unavailable")]
    TmuxUnavailable,
    #[error("cli_agent_notify_lock_unavailable")]
    LockUnavailable,
    #[error("cli_agent_notify_lock_timeout")]
    LockTimeout,
    #[error("cli_agent_notify_write_failed")]
    WriteFailed,
    #[error("cli_agent_notify_write_timeout")]
    WriteTimeout,
}

type Result<T> = std::result::Result<T, HookWriteError>;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Notification {
    v: u32,
    agent: String,
    event: String,
    session_id: String,
    event_id: String,
    plugin_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal_unverified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcript_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_type: Option<String>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn parse_notification(bytes: &[u8]) -> Result<Notification> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(HookWriteError::FrameTooLarge);
    }
    let notification: Notification =
        serde_json::from_slice(bytes).map_err(|_| HookWriteError::InvalidPayload)?;
    let known_event = matches!(
        notification.event.as_str(),
        "session_start"
            | "prompt_submit"
            | "tool_complete"
            | "stop"
            | "stop_failure"
            | "cancelled"
            | "notification"
    );
    let event_id = notification.event_id.strip_prefix("grok:");
    if notification.v != 1
        || notification.agent != "grok"
        || !known_event
        || !valid_id(&notification.session_id)
        || notification
            .prompt_id
            .as_deref()
            .is_some_and(|id| !valid_id(id))
        || !event_id.is_some_and(|id| {
            id.len() == 64
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        || notification.plugin_version.is_empty()
        || notification.plugin_version.len() > 32
        || !notification
            .plugin_version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return Err(HookWriteError::InvalidPayload);
    }
    Ok(notification)
}

fn encode_frame(notification: &Notification, tmux: bool) -> Result<Vec<u8>> {
    let json = serde_json::to_string(notification).map_err(|_| HookWriteError::InvalidPayload)?;
    let mut body = String::with_capacity(json.len());
    for character in json.chars() {
        // serde 转义 C0；额外隔离 DEL/C1 与 OSC 参数分号，避免 VT 注入或参数截断。
        if character == ';' || ('\u{7f}'..='\u{9f}').contains(&character) {
            use std::fmt::Write as _;
            let codepoint = character as u32;
            write!(body, "\\u{codepoint:04x}").map_err(|_| HookWriteError::InvalidPayload)?;
        } else {
            body.push(character);
        }
    }
    let sequence = format!("\x1b]777;notify;{CLI_AGENT_NOTIFICATION_SENTINEL};{body}\x07");
    let frame = if tmux {
        format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"))
    } else {
        sequence
    };
    if frame.len() > MAX_FRAME_BYTES {
        return Err(HookWriteError::FrameTooLarge);
    }
    Ok(frame.into_bytes())
}

fn read_input(mut input: impl io::Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| HookWriteError::InputUnavailable)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(HookWriteError::FrameTooLarge);
    }
    Ok(bytes)
}

/// 只能从一次性 worker 入口调用：超时后入口退出，内核关闭所有句柄并释放锁。
/// 成功仅说明终端写入完成，不能视为应用已接受通知或允许重投。
pub(crate) fn run_worker(protocol_version: bool) -> Result<()> {
    if protocol_version {
        return io::stdout()
            .write_all(PROTOCOL_REPLY)
            .map_err(|_| HookWriteError::WriteFailed);
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(read_input(io::stdin().lock()));
    });
    let bytes = receiver
        .recv_timeout(IO_TIMEOUT)
        .map_err(|_| HookWriteError::InputTimeout)??;
    let notification = parse_notification(&bytes)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(send_notification(notification));
    });
    // Windows 同步控制台写在严重背压下可阻塞；入口总期限仍保证进程退出。
    receiver
        .recv_timeout(SEND_TIMEOUT)
        .map_err(|_| HookWriteError::WriteTimeout)?
}

fn send_notification(notification: Notification) -> Result<()> {
    let tmux = std::env::var_os("TMUX").is_some_and(|value| !value.is_empty());
    let bytes = encode_frame(&notification, tmux)?;
    let mut terminal = open_terminal(tmux)?;
    let cache = dirs::cache_dir().ok_or(HookWriteError::LockUnavailable)?;
    let directory = cache.join("infinishell-cli-agent-notifications-v1");
    send_frame(&directory, &mut terminal, &bytes)
}

fn send_frame(directory: &Path, terminal: &mut impl io::Write, bytes: &[u8]) -> Result<()> {
    let lock = open_lock(directory)?;
    acquire_lock(&lock, IO_TIMEOUT)?;
    // 锁句柄覆盖全部短写；保留空锁文件，进程死亡时仅由内核解除占用。
    write_frame(terminal, bytes, IO_TIMEOUT)
}

fn acquire_lock(lock: &File, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        match lock.try_lock_exclusive() {
            Ok(true) => return Ok(()),
            Ok(false) => {
                if Instant::now() >= deadline {
                    return Err(HookWriteError::LockTimeout);
                }
                thread::sleep(RETRY_DELAY.min(deadline.saturating_duration_since(Instant::now())));
            }
            Err(_) => return Err(HookWriteError::LockUnavailable),
        }
    }
}

fn write_frame(output: &mut impl io::Write, bytes: &[u8], timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut offset = 0;
    while offset < bytes.len() {
        if Instant::now() >= deadline {
            return Err(HookWriteError::WriteTimeout);
        }
        match output.write(&bytes[offset..]) {
            Ok(0) => return Err(HookWriteError::WriteFailed),
            Ok(count) => offset += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(RETRY_DELAY.min(deadline.saturating_duration_since(Instant::now())));
            }
            Err(_) => return Err(HookWriteError::WriteFailed),
        }
    }
    Ok(())
}

fn verify_path_components(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(HookWriteError::LockUnavailable);
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(HookWriteError::LockUnavailable);
        }
        prefix.push(component);
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(HookWriteError::LockUnavailable);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt as _;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(HookWriteError::LockUnavailable);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(HookWriteError::LockUnavailable),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn open_lock(directory: &Path) -> Result<File> {
    use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
    verify_path_components(directory)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|_| HookWriteError::LockUnavailable)?;
    verify_path_components(directory)?;
    let metadata = fs::symlink_metadata(directory).map_err(|_| HookWriteError::LockUnavailable)?;
    // 只核对和创建本模块目录，不收紧用户原有缓存父目录的权限。
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(HookWriteError::LockUnavailable);
    }
    let path = directory.join("transport.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|_| HookWriteError::LockUnavailable)?;
    let metadata = lock
        .metadata()
        .map_err(|_| HookWriteError::LockUnavailable)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() != 0
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(HookWriteError::LockUnavailable);
    }
    let named = fs::symlink_metadata(path).map_err(|_| HookWriteError::LockUnavailable)?;
    if named.dev() != metadata.dev()
        || named.ino() != metadata.ino()
        || named.file_type().is_symlink()
    {
        return Err(HookWriteError::LockUnavailable);
    }
    Ok(lock)
}

#[cfg(unix)]
fn open_unix_terminal(path: &Path) -> io::Result<File> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::{FileTypeExt as _, OpenOptionsExt as _};
    verify_path_components(path).map_err(|_| io::Error::from(io::ErrorKind::PermissionDenied))?;
    if !path.is_absolute()
        || !path.starts_with("/dev")
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        || !fs::symlink_metadata(path)?.file_type().is_char_device()
    {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    let terminal = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    if unsafe { libc::isatty(terminal.as_raw_fd()) } != 1 {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    Ok(terminal)
}

#[cfg(unix)]
fn tmux_terminal() -> Result<PathBuf> {
    use command::blocking::Command;
    use std::process::Stdio;
    let pane = std::env::var("TMUX_PANE").map_err(|_| HookWriteError::TmuxUnavailable)?;
    if !pane.strip_prefix('%').is_some_and(|id| {
        !id.is_empty() && id.len() <= 16 && id.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return Err(HookWriteError::TmuxUnavailable);
    }
    let mut command = Command::new("tmux");
    command
        .args(["display-message", "-p", "-t", &pane, "#{pane_tty}"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| HookWriteError::TmuxUnavailable)?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(HookWriteError::TmuxUnavailable);
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(read_input(stdout));
    });
    let deadline = Instant::now() + IO_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(RETRY_DELAY),
            Ok(None) | Err(_) => break None,
        }
    };
    if !status.is_some_and(|status| status.success()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(HookWriteError::TmuxUnavailable);
    }
    let bytes = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| HookWriteError::TmuxUnavailable)?
        .map_err(|_| HookWriteError::TmuxUnavailable)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| HookWriteError::TmuxUnavailable)?;
    let path = text.strip_suffix('\n').unwrap_or(text);
    if path.is_empty() || path.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(HookWriteError::TmuxUnavailable);
    }
    Ok(PathBuf::from(path))
}

#[cfg(unix)]
fn open_terminal(tmux: bool) -> Result<File> {
    // tmux 必须写当前 pane，不能因为继承了外层控制终端就绕过精确解析。
    if tmux {
        return open_unix_terminal(&tmux_terminal()?)
            .map_err(|_| HookWriteError::TerminalUnavailable);
    }
    match open_unix_terminal(Path::new("/dev/tty")) {
        Ok(terminal) => return Ok(terminal),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ENXIO | libc::ENODEV | libc::ENOENT)
            ) => {}
        Err(_) => return Err(HookWriteError::TerminalUnavailable),
    }
    let path = std::env::var_os(WARP_CLI_AGENT_TTY_ENV)
        .or_else(|| std::env::var_os("SSH_TTY"))
        .ok_or(HookWriteError::TerminalUnavailable)?;
    open_unix_terminal(Path::new(&path)).map_err(|_| HookWriteError::TerminalUnavailable)
}

#[cfg(windows)]
mod windows_platform {
    use std::ffi::c_void;
    use std::mem;
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _};
    use std::ptr;

    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, EqualSid, GetAce,
        GetSecurityDescriptorControl, GetTokenInformation, IsWellKnownSid,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser, WinLocalSystemSid,
    };
    use windows::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, CreateFileW, FILE_ALL_ACCESS,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_CHAR,
        GetFileInformationByHandle, GetFileType, OPEN_ALWAYS, OPEN_EXISTING, READ_CONTROL,
    };
    use windows::Win32::System::Console::{
        CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::core::{PCWSTR, PWSTR};

    use super::*;

    struct LocalMemory(*mut c_void);

    impl Drop for LocalMemory {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // Windows 安全描述符和字符串均由 LocalAlloc 分配。
                unsafe {
                    LocalFree(Some(HLOCAL(self.0)));
                }
            }
        }
    }

    struct PrivateSecurity {
        memory: LocalMemory,
        user: Vec<usize>,
    }

    impl PrivateSecurity {
        fn new() -> Result<Self> {
            let mut token = HANDLE::default();
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
                .map_err(|_| HookWriteError::LockUnavailable)?;
            let mut length = 0;
            unsafe {
                let _ = GetTokenInformation(token, TokenUser, None, 0, &mut length);
            }
            if length == 0 || length > 65536 {
                unsafe {
                    let _ = CloseHandle(token);
                }
                return Err(HookWriteError::LockUnavailable);
            }
            let mut user = vec![0usize; (length as usize).div_ceil(mem::size_of::<usize>())];
            let result = unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    Some(user.as_mut_ptr().cast()),
                    length,
                    &mut length,
                )
            };
            unsafe {
                let _ = CloseHandle(token);
            }
            result.map_err(|_| HookWriteError::LockUnavailable)?;
            let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
            let mut text = PWSTR::null();
            unsafe { ConvertSidToStringSidW(sid, &mut text) }
                .map_err(|_| HookWriteError::LockUnavailable)?;
            let text_memory = LocalMemory(text.0.cast());
            let user_sid =
                unsafe { text.to_string() }.map_err(|_| HookWriteError::LockUnavailable)?;
            drop(text_memory);
            // 仅当前用户和 SYSTEM 完全访问；保护 DACL，不继承缓存父目录的额外授权。
            let sddl = format!("O:{user_sid}D:P(A;OICI;FA;;;{user_sid})(A;OICI;FA;;;SY)");
            let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(wide.as_ptr()),
                    1,
                    &mut descriptor,
                    None,
                )
            }
            .map_err(|_| HookWriteError::LockUnavailable)?;
            Ok(Self {
                memory: LocalMemory(descriptor.0),
                user,
            })
        }

        fn attributes(&self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: self.memory.0,
                bInheritHandle: false.into(),
            }
        }

        fn verify(&self, file: &File) -> Result<()> {
            let mut owner = PSID::default();
            let mut dacl: *mut ACL = ptr::null_mut();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            let error = unsafe {
                GetSecurityInfo(
                    HANDLE(file.as_raw_handle()),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    Some(&mut owner),
                    None,
                    Some(&mut dacl),
                    None,
                    Some(&mut descriptor),
                )
            };
            let memory = LocalMemory(descriptor.0);
            if error.0 != 0 || dacl.is_null() || owner.0.is_null() {
                return Err(HookWriteError::LockUnavailable);
            }
            let user_sid = unsafe { (*(self.user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
            unsafe { EqualSid(owner, user_sid) }.map_err(|_| HookWriteError::LockUnavailable)?;
            let mut control = 0;
            let mut revision = 0;
            unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) }
                .map_err(|_| HookWriteError::LockUnavailable)?;
            if control & SE_DACL_PROTECTED.0 == 0 || unsafe { (*dacl).AceCount } != 2 {
                return Err(HookWriteError::LockUnavailable);
            }
            let mut user_seen = false;
            let mut system_seen = false;
            for index in 0..2 {
                let mut raw = ptr::null_mut();
                unsafe { GetAce(dacl, index, &mut raw) }
                    .map_err(|_| HookWriteError::LockUnavailable)?;
                if raw.is_null() {
                    return Err(HookWriteError::LockUnavailable);
                }
                // ACCESS_ALLOWED_ACE 的类型为 0；仅接纳本模块写出的 OI/CI 两个标记。
                let header = unsafe { &*(raw.cast::<ACE_HEADER>()) };
                if header.AceType != 0 || header.AceFlags != 3 || header.AceSize < 16 {
                    return Err(HookWriteError::LockUnavailable);
                }
                let ace = unsafe { &*(raw.cast::<ACCESS_ALLOWED_ACE>()) };
                if ace.Mask != FILE_ALL_ACCESS.0 {
                    return Err(HookWriteError::LockUnavailable);
                }
                let sid = PSID(ptr::addr_of!(ace.SidStart).cast_mut().cast());
                if unsafe { EqualSid(sid, user_sid) }.is_ok() && !user_seen {
                    user_seen = true;
                } else if unsafe { IsWellKnownSid(sid, WinLocalSystemSid) }.as_bool()
                    && !system_seen
                {
                    system_seen = true;
                } else {
                    return Err(HookWriteError::LockUnavailable);
                }
            }
            drop(memory);
            if !user_seen || !system_seen {
                return Err(HookWriteError::LockUnavailable);
            }
            Ok(())
        }
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn open_lock(directory: &Path) -> Result<File> {
        verify_path_components(directory)?;
        let parent = directory.parent().ok_or(HookWriteError::LockUnavailable)?;
        fs::create_dir_all(parent).map_err(|_| HookWriteError::LockUnavailable)?;
        verify_path_components(directory)?;
        let security = PrivateSecurity::new()?;
        let attributes = security.attributes();
        let wide = wide_path(directory);
        let created = unsafe { CreateDirectoryW(PCWSTR(wide.as_ptr()), Some(&attributes)) };
        if created.is_err() && !directory.is_dir() {
            return Err(HookWriteError::LockUnavailable);
        }
        verify_path_components(directory)?;
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                READ_CONTROL.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .map_err(|_| HookWriteError::LockUnavailable)?;
        let directory_handle = unsafe { File::from_raw_handle(handle.0) };
        security.verify(&directory_handle)?;
        let path = directory.join("transport.lock");
        verify_path_components(&path)?;
        let wide = wide_path(&path);
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                Some(&attributes),
                OPEN_ALWAYS,
                FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
        }
        .map_err(|_| HookWriteError::LockUnavailable)?;
        let lock = unsafe { File::from_raw_handle(handle.0) };
        security.verify(&lock)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(handle, &mut info) }
            .map_err(|_| HookWriteError::LockUnavailable)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
            || info.nNumberOfLinks != 1
            || info.nFileSizeHigh != 0
            || info.nFileSizeLow != 0
            || !lock
                .metadata()
                .map_err(|_| HookWriteError::LockUnavailable)?
                .is_file()
        {
            return Err(HookWriteError::LockUnavailable);
        }
        Ok(lock)
    }

    pub(super) fn open_terminal(tmux: bool) -> Result<File> {
        if tmux {
            return Err(HookWriteError::TmuxUnavailable);
        }
        let terminal = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
            .open("CONOUT$")
            .map_err(|_| HookWriteError::TerminalUnavailable)?;
        let handle = HANDLE(terminal.as_raw_handle());
        let mut mode = CONSOLE_MODE::default();
        if unsafe { GetFileType(handle) } != FILE_TYPE_CHAR
            || unsafe { GetConsoleMode(handle, &mut mode) }.is_err()
            || mode.0 & ENABLE_VIRTUAL_TERMINAL_PROCESSING.0 == 0
        {
            return Err(HookWriteError::TerminalUnavailable);
        }
        Ok(terminal)
    }
}

#[cfg(windows)]
use windows_platform::{open_lock, open_terminal};

#[cfg(test)]
#[path = "cli_agent_hook_writer_tests.rs"]
mod tests;
