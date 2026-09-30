use super::{
    ConsoleProbe, ENTRYPOINT, arguments, capture_expected, parse_arguments, quote,
    spawn_in_console, spawn_probe,
};
use crate::managed::{WindowsProcessIdentity, WindowsProcessLease};
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateEventW, EVENT_MODIFY_STATE, GetExitCodeProcess, OpenEventW, SetEvent, WaitForSingleObject,
};
use windows::core::HSTRING;

const CONSOLE_FIXTURE: &str = "managed_windows_console_probe::tests::console_member_fixture";

fn shell() -> WindowsProcessIdentity {
    WindowsProcessIdentity {
        pid: 41,
        created_at: 12345,
        logon_low: 9,
        logon_high: -1,
        session_id: 2,
    }
}

fn native() -> WindowsProcessIdentity {
    WindowsProcessIdentity {
        pid: 42,
        created_at: 12346,
        logon_low: 9,
        logon_high: -1,
        session_id: 2,
    }
}

#[test]
fn identity_arguments_preserve_all_lifetime_and_logon_fields() {
    let values = [
        ENTRYPOINT, "1", "41", "12345", "9", "-1", "2", "42", "12346", "9", "-1", "2",
    ]
    .map(str::to_owned);

    assert_eq!(parse_arguments(&values).unwrap(), (shell(), native()));
    assert_eq!(arguments(shell(), native()), values);
}

#[test]
fn ambiguous_numeric_identity_and_extra_arguments_are_rejected() {
    let mut leading_zero = arguments(shell(), native());
    leading_zero[2] = "041".to_owned();
    let mut extra = arguments(shell(), native());
    extra.push("ignored".to_owned());
    let mut version = arguments(shell(), native());
    version[1] = "2".to_owned();

    assert!(parse_arguments(&leading_zero).is_err());
    assert!(parse_arguments(&extra).is_err());
    assert!(parse_arguments(&version).is_err());
}

#[test]
fn same_process_cannot_stand_in_for_both_shell_and_native() {
    assert!(parse_arguments(&arguments(shell(), shell())).is_err());
}

#[test]
fn changed_creation_time_cannot_authorize_a_current_pid() {
    let current = WindowsProcessLease::capture(std::process::id()).unwrap();
    let mut stale = current.identity();
    stale.created_at -= 1;

    assert!(capture_expected(stale).is_err());
    assert!(!current.exited().unwrap());
}

#[test]
fn absent_hpcon_is_rejected_before_spawning_a_helper() {
    let current = WindowsProcessLease::capture(std::process::id()).unwrap();

    let result = unsafe {
        spawn_probe(
            HPCON::default(),
            &std::env::current_exe().unwrap(),
            &current,
            &current,
        )
    };

    assert!(result.is_err());
    assert!(!current.exited().unwrap());
}

#[test]
fn program_quoting_preserves_spaces_quotes_and_trailing_slashes() {
    let mut output = Vec::new();
    quote(
        &r#"C:\a b\quoted"name\"#.encode_utf16().collect::<Vec<_>>(),
        &mut output,
    );

    assert_eq!(
        String::from_utf16(&output).unwrap(),
        "\"C:\\a b\\quoted\\\"name\\\\\""
    );
}

struct TestConsole {
    console: Option<HPCON>,
    input: Option<OwnedHandle>,
    drain: Option<JoinHandle<io::Result<u64>>>,
}

fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    unsafe { CreatePipe(&mut read, &mut write, None, 0) }.map_err(io::Error::from)?;
    Ok((unsafe { OwnedHandle::from_raw_handle(read.0) }, unsafe {
        OwnedHandle::from_raw_handle(write.0)
    }))
}

impl TestConsole {
    fn create() -> io::Result<Self> {
        let (input_read, input_write) = pipe()?;
        let (output_read, output_write) = pipe()?;
        let console = unsafe {
            CreatePseudoConsole(
                COORD { X: 80, Y: 24 },
                HANDLE(input_read.as_raw_handle()),
                HANDLE(output_write.as_raw_handle()),
                0,
            )
        }
        .map_err(io::Error::from)?;
        let mut result = Self {
            console: Some(console),
            input: Some(input_write),
            drain: None,
        };
        // ConPTY 已复制服务端句柄；父进程不能保留写端而阻止 drain 观察 EOF。
        drop(input_read);
        drop(output_write);
        let mut output = File::from(output_read);
        result.drain = Some(thread::spawn(move || {
            match io::copy(&mut output, &mut io::sink()) {
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(0),
                result => result,
            }
        }));
        Ok(result)
    }

    fn raw(&self) -> HPCON {
        self.console.unwrap()
    }

    fn close(&mut self) -> io::Result<()> {
        drop(self.input.take());
        if let Some(console) = self.console.take() {
            // ClosePseudoConsole 可等待输出排空；始终让独立 reader 在关闭期间继续读取。
            unsafe { ClosePseudoConsole(console) };
        }
        if let Some(drain) = self.drain.take() {
            drain
                .join()
                .map_err(|_| io::Error::other("ConPTY 输出排空线程异常退出"))??;
        }
        Ok(())
    }
}

impl Drop for TestConsole {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

struct Fixture {
    process: Option<ConsoleProbe>,
    lease: WindowsProcessLease,
    finish: OwnedHandle,
}

fn fixture_event(identity: WindowsProcessIdentity) -> HSTRING {
    HSTRING::from(format!(
        "Local\\InfiniShellConsoleProbe-{}-{}",
        identity.pid, identity.created_at
    ))
}

impl Fixture {
    fn start(console: &TestConsole) -> io::Result<Self> {
        let process = spawn_in_console(
            console.raw(),
            &std::env::current_exe()?,
            &[
                "--ignored".to_owned(),
                "--exact".to_owned(),
                CONSOLE_FIXTURE.to_owned(),
                "--nocapture".to_owned(),
            ],
        )?;
        let lease = WindowsProcessLease::from_process_handle(&process.process)?;
        let name = fixture_event(lease.identity());
        let deadline = Instant::now() + Duration::from_secs(10);
        let event = loop {
            lease.validate()?;
            if let Ok(event) = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &name) } {
                break event;
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "ConPTY 替身未就绪"));
            }
            thread::sleep(Duration::from_millis(10));
        };
        Ok(Self {
            process: Some(process),
            lease,
            finish: unsafe { OwnedHandle::from_raw_handle(event.0) },
        })
    }

    fn finish(mut self) -> io::Result<()> {
        unsafe { SetEvent(HANDLE(self.finish.as_raw_handle())) }.map_err(io::Error::from)?;
        self.process.take().unwrap().wait()?;
        if !self.lease.exited()? {
            return Err(io::Error::other("ConPTY 替身退出未确认"));
        }
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = unsafe { SetEvent(HANDLE(self.finish.as_raw_handle())) };
        if let Some(process) = &self.process {
            // 失败回收先让本测试替身自然退出，超时才由其 ConsoleProbe 原句柄收尾。
            let _ = unsafe { WaitForSingleObject(HANDLE(process.process.as_raw_handle()), 1000) };
        }
    }
}

fn assert_probe_denied(probe: ConsoleProbe) {
    let process = HANDLE(probe.process.as_raw_handle());
    assert_eq!(unsafe { WaitForSingleObject(process, 5000) }, WAIT_OBJECT_0);
    let mut exit = 0;
    unsafe { GetExitCodeProcess(process, &mut exit) }.unwrap();
    assert_eq!(
        exit, 1,
        "必须由真实 helper 明确拒绝，不能把超时或崩溃当作拒绝"
    );
    assert_eq!(
        probe.wait().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
#[ignore = "需要 workflow 先构建真实 infinishell-ssh.exe 并提供固定 helper 路径"]
fn real_conpty_probe_binds_original_console() {
    let executable = PathBuf::from(
        std::env::var_os("INFINISHELL_TEST_CONSOLE_PROBE_EXECUTABLE")
            .expect("缺少真实 SSH worker 路径"),
    );
    assert!(executable.is_absolute() && executable.is_file());
    let mut first = TestConsole::create().unwrap();
    let mut second = TestConsole::create().unwrap();
    let first_shell = Fixture::start(&first).unwrap();
    let first_native = Fixture::start(&first).unwrap();
    let second_shell = Fixture::start(&second).unwrap();
    let second_native = Fixture::start(&second).unwrap();

    unsafe {
        spawn_probe(
            first.raw(),
            &executable,
            &first_shell.lease,
            &first_native.lease,
        )
    }
    .unwrap()
    .wait()
    .unwrap();
    unsafe {
        spawn_probe(
            second.raw(),
            &executable,
            &second_shell.lease,
            &second_native.lease,
        )
    }
    .unwrap()
    .wait()
    .unwrap();
    let different_console = unsafe {
        spawn_probe(
            first.raw(),
            &executable,
            &first_shell.lease,
            &second_native.lease,
        )
    }
    .unwrap();
    assert_probe_denied(different_console);
    let mut stale_native = first_native.lease.identity();
    stale_native.created_at -= 1;
    let stale = spawn_in_console(
        first.raw(),
        &executable,
        &arguments(first_shell.lease.identity(), stale_native),
    )
    .unwrap();
    assert_probe_denied(stale);

    assert!(!first_shell.lease.exited().unwrap());
    assert!(!first_native.lease.exited().unwrap());
    assert!(!second_shell.lease.exited().unwrap());
    assert!(!second_native.lease.exited().unwrap());
    first_shell.finish().unwrap();
    first_native.finish().unwrap();
    second_shell.finish().unwrap();
    second_native.finish().unwrap();
    first.close().unwrap();
    second.close().unwrap();
}

#[test]
#[ignore = "仅由真实 HPCON 成员测试派生，事件解除或 90 秒后自然退出"]
fn console_member_fixture() {
    let current = WindowsProcessLease::capture(std::process::id()).unwrap();
    let name = fixture_event(current.identity());
    let event = unsafe { CreateEventW(None, true, false, &name) }.unwrap();
    let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
    let result = unsafe { WaitForSingleObject(HANDLE(event.as_raw_handle()), 90_000) };
    assert!(result == WAIT_OBJECT_0 || result == WAIT_TIMEOUT);
}
