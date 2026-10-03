//! 在调用方保留的真实 HPCON 内派生只读成员查询；不改变 GUI 的全局控制台。

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;

use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Console::{GetConsoleProcessList, HPCON};
use windows::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use crate::managed::{WindowsProcessIdentity, WindowsProcessLease};

const ENTRYPOINT: &str = "--grok-console-membership";
const WAIT_MS: u32 = 5000;

/// 仅拥有刚创建的查询 helper；从未拥有 shell 或 CLI 的终止权限。
pub struct ConsoleProbe {
    process: OwnedHandle,
    finished: bool,
}

impl ConsoleProbe {
    pub fn wait(mut self) -> io::Result<()> {
        let process = HANDLE(self.process.as_raw_handle());
        if unsafe { WaitForSingleObject(process, WAIT_MS) } != WAIT_OBJECT_0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "ConPTY 成员查询超时",
            ));
        }
        self.finished = true;
        let mut code = 0;
        unsafe { GetExitCodeProcess(process, &mut code) }.map_err(io::Error::from)?;
        if code != 0 {
            return Err(invalid());
        }
        Ok(())
    }
}

impl Drop for ConsoleProbe {
    fn drop(&mut self) {
        if !self.finished {
            let process = HANDLE(self.process.as_raw_handle());
            if unsafe { WaitForSingleObject(process, 0) } != WAIT_OBJECT_0 {
                // 原句柄只能指向本模块的私有查询 helper，绝不按 PID 结束其它进程。
                let _ = unsafe { TerminateProcess(process, 1) };
                let _ = unsafe { WaitForSingleObject(process, 1000) };
            }
        }
    }
}

/// # 安全约定
/// 调用方必须持有原 HPCON 生存期锁，直到此函数返回；不能传入缓存或恢复的整数句柄。
/// 释放锁后须再次校验两份原 lease，再等待 helper；错误回收也必须在锁外进行。
/// executable 必须是提供本模块隐藏入口的可信应用。
pub unsafe fn spawn_probe(
    console: HPCON,
    executable: &Path,
    shell: &WindowsProcessLease,
    native: &WindowsProcessLease,
) -> io::Result<ConsoleProbe> {
    if console.0 == 0 || !executable.is_absolute() || shell.identity() == native.identity() {
        return Err(invalid());
    }
    shell.validate()?;
    native.validate()?;
    spawn_in_console(
        console,
        executable,
        &arguments(shell.identity(), native.identity()),
    )
}

fn spawn_in_console(
    console: HPCON,
    executable: &Path,
    arguments: &[String],
) -> io::Result<ConsoleProbe> {
    if console.0 == 0 || !executable.is_absolute() {
        return Err(invalid());
    }
    let mut attributes = ConsoleAttributes::new(console)?;
    let mut program: Vec<u16> = executable.as_os_str().encode_wide().collect();
    if program.contains(&0) {
        return Err(invalid());
    }
    let mut command = Vec::new();
    quote(&program, &mut command);
    for argument in arguments {
        command.push(b' ' as u16);
        quote(&argument.encode_utf16().collect::<Vec<_>>(), &mut command);
    }
    program.push(0);
    command.push(0);
    if command.len() > 32767 {
        return Err(invalid());
    }
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    // 与真实 shell 相同，由 HPCON 建立标准句柄，不继承 GUI 的重定向句柄。
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = attributes.pointer();
    let mut information = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(program.as_ptr()),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            false,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            None,
            PCWSTR::null(),
            &startup.StartupInfo,
            &mut information,
        )
    }
    .map_err(io::Error::from)?;
    let probe = ConsoleProbe {
        process: unsafe { OwnedHandle::from_raw_handle(information.hProcess.0) },
        finished: false,
    };
    drop(unsafe { OwnedHandle::from_raw_handle(information.hThread.0) });
    Ok(probe)
}

/// 主程序必须在初始化 GUI 前调用；Some 的成败仅映射 exit(0/1)，不得打印到用户终端。
pub fn run_from_args() -> Option<io::Result<()>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.first().map(|value| value.as_os_str()) != Some(OsStr::new(ENTRYPOINT)) {
        return None;
    }
    Some((|| {
        let arguments = arguments
            .into_iter()
            .map(|value| value.into_string().map_err(|_| invalid()))
            .collect::<io::Result<Vec<_>>>()?;
        let (shell, native) = parse_arguments(&arguments)?;
        verify_console_membership(shell, native)
    })())
}

fn verify_console_membership(
    expected_shell: WindowsProcessIdentity,
    expected_native: WindowsProcessIdentity,
) -> io::Result<()> {
    let shell = capture_expected(expected_shell)?;
    let native = capture_expected(expected_native)?;
    if expected_shell == expected_native
        || expected_shell.pid == std::process::id()
        || expected_native.pid == std::process::id()
    {
        return Err(invalid());
    }
    console_contains(expected_shell.pid, expected_native.pid)?;
    shell.validate()?;
    native.validate()?;
    console_contains(expected_shell.pid, expected_native.pid)?;
    shell.validate()?;
    native.validate()
}

fn capture_expected(expected: WindowsProcessIdentity) -> io::Result<WindowsProcessLease> {
    let held = WindowsProcessLease::capture(expected.pid)?;
    if held.identity() != expected {
        return Err(invalid());
    }
    Ok(held)
}

fn console_contains(shell: u32, native: u32) -> io::Result<()> {
    let mut clients = vec![0u32; 16];
    for _ in 0..3 {
        let count = unsafe { GetConsoleProcessList(&mut clients) } as usize;
        if count == 0 || count > 4096 {
            return Err(invalid());
        }
        if count > clients.len() {
            clients.resize(count, 0);
            continue;
        }
        let clients = &clients[..count];
        if clients.contains(&shell)
            && clients.contains(&native)
            && clients.contains(&std::process::id())
        {
            return Ok(());
        }
        return Err(invalid());
    }
    Err(invalid())
}

fn arguments(shell: WindowsProcessIdentity, native: WindowsProcessIdentity) -> Vec<String> {
    let mut result = vec![ENTRYPOINT.to_owned(), "1".to_owned()];
    for identity in [shell, native] {
        result.extend([
            identity.pid.to_string(),
            identity.created_at.to_string(),
            identity.logon_low.to_string(),
            identity.logon_high.to_string(),
            identity.session_id.to_string(),
        ]);
    }
    result
}

fn parse_arguments(
    arguments: &[String],
) -> io::Result<(WindowsProcessIdentity, WindowsProcessIdentity)> {
    if arguments.len() != 12 || arguments[0] != ENTRYPOINT || arguments[1] != "1" {
        return Err(invalid());
    }
    fn parse(parts: &[String]) -> io::Result<WindowsProcessIdentity> {
        let identity = WindowsProcessIdentity {
            pid: parts[0].parse().map_err(|_| invalid())?,
            created_at: parts[1].parse().map_err(|_| invalid())?,
            logon_low: parts[2].parse().map_err(|_| invalid())?,
            logon_high: parts[3].parse().map_err(|_| invalid())?,
            session_id: parts[4].parse().map_err(|_| invalid())?,
        };
        if identity.pid == 0 || identity.created_at == 0 {
            return Err(invalid());
        }
        Ok(identity)
    }
    let shell = parse(&arguments[2..7])?;
    let native = parse(&arguments[7..12])?;
    if self::arguments(shell, native) != arguments || shell == native {
        return Err(invalid());
    }
    Ok((shell, native))
}

struct ConsoleAttributes {
    storage: Vec<usize>,
}

impl ConsoleAttributes {
    fn new(console: HPCON) -> io::Result<Self> {
        let mut bytes = 0;
        let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut bytes) };
        if bytes == 0 || bytes > 65536 {
            return Err(invalid());
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let pointer = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        unsafe { InitializeProcThreadAttributeList(Some(pointer), 1, None, &mut bytes) }
            .map_err(io::Error::from)?;
        let mut attributes = Self { storage };
        unsafe {
            UpdateProcThreadAttribute(
                attributes.pointer(),
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                Some(console.0 as *const std::ffi::c_void),
                size_of::<HPCON>(),
                None,
                None,
            )
        }
        .map_err(io::Error::from)?;
        Ok(attributes)
    }

    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.storage.as_mut_ptr().cast())
    }
}

impl Drop for ConsoleAttributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.pointer()) };
    }
}

fn quote(argument: &[u16], output: &mut Vec<u16>) {
    output.push(b'"' as u16);
    let mut backslashes = 0;
    for &unit in argument {
        if unit == b'\\' as u16 {
            backslashes += 1;
        } else {
            let count = if unit == b'"' as u16 {
                backslashes * 2 + 1
            } else {
                backslashes
            };
            output.extend(std::iter::repeat_n(b'\\' as u16, count));
            output.push(unit);
            backslashes = 0;
        }
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2));
    output.push(b'"' as u16);
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "ConPTY 成员身份不匹配")
}

#[cfg(test)]
#[path = "managed_windows_console_probe_tests.rs"]
mod tests;
