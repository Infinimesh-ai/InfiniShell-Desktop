//! 普通终端侧车保留 Windows 进程句柄；PID 或管道名称不能单独证明会话归属。

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt as _;
use std::os::windows::io::{AsRawHandle, FromRawHandle as _, OwnedHandle};
use std::path::PathBuf;

use windows::Win32::Foundation::{
    DUPLICATE_HANDLE_OPTIONS, DuplicateHandle, ERROR_INVALID_PARAMETER, FILETIME, HANDLE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Security::{
    GetLengthSid, GetTokenInformation, IsValidSid, TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER,
    TokenSessionId, TokenStatistics, TokenUser,
};
use windows::Win32::System::Pipes::{GetNamedPipeServerProcessId, GetNamedPipeServerSessionId};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessId, GetProcessTimes, OpenProcess, OpenProcessToken,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    QueryFullProcessImageNameW, WaitForSingleObject,
};
use windows::core::{HRESULT, PWSTR};

/// 适合写入应用私有启动清单；恢复时仍须重新打开句柄并核对全部字段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowsProcessIdentity {
    pub pid: u32,
    pub created_at: u64,
    pub logon_low: u32,
    pub logon_high: i32,
    pub session_id: u32,
}

/// 只持有查询和等待权限；侧车析构与断连都不能终止原生 TUI 或 leader。
pub struct WindowsProcessLease {
    handle: OwnedHandle,
    identity: WindowsProcessIdentity,
    owner: Vec<u8>,
    image: PathBuf,
}

impl AsRawHandle for WindowsProcessLease {
    fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        self.handle.as_raw_handle()
    }
}

impl WindowsProcessLease {
    pub fn capture(pid: u32) -> io::Result<Self> {
        if pid == 0 {
            return Err(io::Error::other("原生进程 PID 无效"));
        }
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        }
        .map_err(io::Error::from)?;
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let lease = Self::from_owned_handle(handle)?;
        if lease.identity.pid != pid {
            return Err(io::Error::other("原生进程身份已变化"));
        }
        Ok(lease)
    }

    /// ConPTY 创建者传入 CreateProcess 返回的原句柄，复制期间不通过 PID 重新寻找进程。
    pub fn from_process_handle(process: &impl AsRawHandle) -> io::Result<Self> {
        let current = unsafe { GetCurrentProcess() };
        let mut duplicated = HANDLE::default();
        unsafe {
            DuplicateHandle(
                current,
                HANDLE(process.as_raw_handle()),
                current,
                &mut duplicated,
                (PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE).0,
                false,
                DUPLICATE_HANDLE_OPTIONS(0),
            )
        }
        .map_err(io::Error::from)?;
        Self::from_owned_handle(unsafe { OwnedHandle::from_raw_handle(duplicated.0) })
    }

    /// 仅捕获已连接端点。调用者仍需与自己保存的启动记录、CLI 映像和会话 ID 绑定。
    pub fn from_named_pipe_peer(pipe: &impl AsRawHandle) -> io::Result<Self> {
        let mut pid = 0;
        unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }
            .map_err(io::Error::from)?;
        let lease = Self::capture(pid)?;
        lease.validate_named_pipe_peer(pipe)?;
        Ok(lease)
    }

    fn from_owned_handle(handle: OwnedHandle) -> io::Result<Self> {
        let raw = HANDLE(handle.as_raw_handle());
        let (identity, owner) = identity(raw)?;
        let current = token(unsafe { GetCurrentProcess() })?;
        let current_raw = HANDLE(current.as_raw_handle());
        let (logon_low, logon_high, session_id) = token_session(current_raw)?;
        if owner != token_owner(current_raw)?
            || (identity.logon_low, identity.logon_high, identity.session_id)
                != (logon_low, logon_high, session_id)
        {
            return Err(io::Error::other("原生进程不属于当前登录会话"));
        }
        let image = image_path(raw)?;
        let lease = Self {
            handle,
            identity,
            owner,
            image,
        };
        lease.validate()?;
        Ok(lease)
    }

    pub fn identity(&self) -> WindowsProcessIdentity {
        self.identity
    }

    pub fn image_path(&self) -> &std::path::Path {
        &self.image
    }

    pub fn validate(&self) -> io::Result<()> {
        let raw = HANDLE(self.handle.as_raw_handle());
        if unsafe { WaitForSingleObject(raw, 0) } != WAIT_TIMEOUT {
            return Err(io::Error::other("原生进程已经退出或无法核对"));
        }
        let (current, owner) = identity(raw)?;
        if current != self.identity || owner != self.owner || image_path(raw)? != self.image {
            return Err(io::Error::other("原生进程身份已变化"));
        }
        Ok(())
    }

    pub fn exited(&self) -> io::Result<bool> {
        match unsafe { WaitForSingleObject(HANDLE(self.handle.as_raw_handle()), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// 从已经连接的管道取内核服务端身份；不枚举或尝试接入其它 Grok leader。
    pub fn validate_named_pipe_peer(&self, pipe: &impl AsRawHandle) -> io::Result<()> {
        self.validate()?;
        let raw = HANDLE(pipe.as_raw_handle());
        let mut pid = 0;
        let mut session_id = 0;
        unsafe {
            GetNamedPipeServerProcessId(raw, &mut pid)?;
            GetNamedPipeServerSessionId(raw, &mut session_id)?;
        }
        if pid != self.identity.pid || session_id != self.identity.session_id {
            return Err(io::Error::other("原生管道服务端与启动会话不匹配"));
        }
        let peer = Self::capture(pid)?;
        if peer.identity != self.identity || peer.image != self.image || peer.owner != self.owner {
            return Err(io::Error::other("原生管道服务端身份已变化"));
        }
        // 保留的原句柄再次核对，禁止用此刻同名进程替代原服务端。
        self.validate()
    }
}

/// 恢复仅核对旧进程是否结束；拒绝访问或身份读取失败都不能当成进程已退出。
pub fn windows_process_exited(expected: WindowsProcessIdentity) -> io::Result<bool> {
    if expected.pid == 0 || expected.created_at == 0 {
        return Err(io::Error::other("原生退出记录身份无效"));
    }
    let raw = match unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            expected.pid,
        )
    } {
        Ok(handle) => handle,
        Err(error) if error.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => {
            return Ok(true);
        }
        Err(error) => return Err(io::Error::from(error)),
    };
    let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    let raw = HANDLE(handle.as_raw_handle());
    match unsafe { WaitForSingleObject(raw, 0) } {
        WAIT_OBJECT_0 => return Ok(true),
        WAIT_TIMEOUT => {}
        _ => return Err(io::Error::last_os_error()),
    }
    let (current, _) = identity(raw)?;
    if current.pid != expected.pid {
        return Err(io::Error::other("原生进程句柄与 PID 不匹配"));
    }
    if current.created_at != expected.created_at {
        return Ok(true);
    }
    if current != expected {
        // 同一生存期的令牌变化不是退出；不得据此释放升级占用或清理会话文件。
        return Err(io::Error::other("原生进程登录身份发生变化"));
    }
    match unsafe { WaitForSingleObject(raw, 0) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}

fn identity(process: HANDLE) -> io::Result<(WindowsProcessIdentity, Vec<u8>)> {
    let pid = unsafe { GetProcessId(process) };
    if pid == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
        .map_err(io::Error::from)?;
    let token = token(process)?;
    let raw = HANDLE(token.as_raw_handle());
    let (logon_low, logon_high, session_id) = token_session(raw)?;
    Ok((
        WindowsProcessIdentity {
            pid,
            created_at: (u64::from(creation.dwHighDateTime) << 32)
                | u64::from(creation.dwLowDateTime),
            logon_low,
            logon_high,
            session_id,
        },
        token_owner(raw)?,
    ))
}

fn token(process: HANDLE) -> io::Result<OwnedHandle> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(io::Error::from)?;
    Ok(unsafe { OwnedHandle::from_raw_handle(token.0) })
}

fn token_session(token: HANDLE) -> io::Result<(u32, i32, u32)> {
    let mut statistics = TOKEN_STATISTICS::default();
    let mut session_id = 0u32;
    let mut written = 0;
    unsafe {
        GetTokenInformation(
            token,
            TokenStatistics,
            Some((&mut statistics as *mut TOKEN_STATISTICS).cast()),
            size_of::<TOKEN_STATISTICS>() as u32,
            &mut written,
        )?;
    }
    if written as usize != size_of::<TOKEN_STATISTICS>() {
        return Err(io::Error::other("原生进程登录标识长度无效"));
    }
    unsafe {
        GetTokenInformation(
            token,
            TokenSessionId,
            Some((&mut session_id as *mut u32).cast()),
            size_of::<u32>() as u32,
            &mut written,
        )?;
    }
    if written as usize != size_of::<u32>() {
        return Err(io::Error::other("原生进程会话标识长度无效"));
    }
    Ok((
        statistics.AuthenticationId.LowPart,
        statistics.AuthenticationId.HighPart,
        session_id,
    ))
}

fn token_owner(token: HANDLE) -> io::Result<Vec<u8>> {
    let mut length = 0;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut length) };
    if !(size_of::<TOKEN_USER>()..=4096).contains(&(length as usize)) {
        return Err(io::Error::other("原生进程用户标识长度无效"));
    }
    // usize 缓冲保证 TOKEN_USER 与其 SID 指针的原生对齐。
    let mut storage = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    let capacity = storage.len() * size_of::<usize>();
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(storage.as_mut_ptr().cast()),
            capacity as u32,
            &mut length,
        )
    }
    .map_err(io::Error::from)?;
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let start = storage.as_ptr() as usize;
    let sid_start = user.User.Sid.0 as usize;
    let end = start + length as usize;
    if length as usize > capacity
        || sid_start < start
        || sid_start.checked_add(8).is_none_or(|v| v > end)
    {
        return Err(io::Error::other("原生进程用户标识边界无效"));
    }
    let sid_length = unsafe { GetLengthSid(user.User.Sid) } as usize;
    if sid_length == 0
        || sid_start.checked_add(sid_length).is_none_or(|v| v > end)
        || !unsafe { IsValidSid(user.User.Sid) }.as_bool()
    {
        return Err(io::Error::other("原生进程用户标识无效"));
    }
    Ok(unsafe { std::slice::from_raw_parts(user.User.Sid.0.cast::<u8>(), sid_length) }.to_vec())
}

fn image_path(process: HANDLE) -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(io::Error::from)?;
    if length == 0 || length as usize >= buffer.len() {
        return Err(io::Error::other("原生进程映像路径无效"));
    }
    Ok(PathBuf::from(OsString::from_wide(
        &buffer[..length as usize],
    )))
}
