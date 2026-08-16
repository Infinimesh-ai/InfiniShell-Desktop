//! Windows `remote-server-proxy`:通过 named pipe 连接常驻 daemon。

use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::process::Stdio;
use std::time::{Duration, Instant};

use interprocess::local_socket::LocalSocketStream;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::core::PCWSTR;

use super::super::setup;

const DAEMON_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn pipe_name(identity_key: &str) -> String {
    let logical_name = format!(
        "{}:{}",
        setup::remote_server_daemon_dir(identity_key),
        setup::daemon_socket_name()
    );
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    logical_name.hash(&mut hasher);
    format!("@infinishell-remote-server-{:016x}", hasher.finish())
}

fn mutex_name(identity_key: &str) -> Vec<u16> {
    format!(
        "Local\\InfiniShellRemoteServer-{}",
        pipe_name(identity_key).trim_start_matches('@')
    )
    .encode_utf16()
    .chain(std::iter::once(0))
    .collect()
}

struct StartupMutex(HANDLE);

impl StartupMutex {
    fn acquire(identity_key: &str) -> anyhow::Result<Self> {
        let name = mutex_name(identity_key);
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }?;
        let wait_result = unsafe {
            WaitForSingleObject(
                handle,
                DAEMON_STARTUP_TIMEOUT.as_millis().min(u128::from(u32::MAX)) as u32,
            )
        };
        if wait_result == WAIT_OBJECT_0 || wait_result == WAIT_ABANDONED {
            Ok(Self(handle))
        } else if wait_result == WAIT_TIMEOUT {
            unsafe {
                let _ = CloseHandle(handle);
            }
            anyhow::bail!("timed out waiting for remote-server startup mutex")
        } else {
            unsafe {
                let _ = CloseHandle(handle);
            }
            anyhow::bail!("failed to acquire remote-server startup mutex: {wait_result:?}")
        }
    }
}

impl Drop for StartupMutex {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn run(identity_key: &str) -> anyhow::Result<()> {
    let startup_mutex = StartupMutex::acquire(identity_key)?;
    let pipe_name = pipe_name(identity_key);
    let stream = match LocalSocketStream::connect(pipe_name.as_str()) {
        Ok(stream) => stream,
        Err(_) => start_daemon_and_connect(identity_key, &pipe_name)?,
    };
    drop(startup_mutex);
    bridge_stdio_to_pipe(stream)
}

fn start_daemon_and_connect(
    identity_key: &str,
    pipe_name: &str,
) -> anyhow::Result<LocalSocketStream> {
    let exe = std::env::current_exe()?;
    let mut command = command::blocking::Command::new(exe);
    command
        .arg("remote-server-daemon")
        .arg("--identity-key")
        .arg(identity_key)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // `command::blocking::Command` 在 Windows 上默认带
    // CREATE_BREAKAWAY_FROM_JOB,daemon 不会随 sshd/proxy 的 Job Object 退出。
    let mut child = command
        .spawn()
        .map_err(|error| anyhow::anyhow!("failed to spawn Windows daemon: {error}"))?;

    let start = Instant::now();
    loop {
        match LocalSocketStream::connect(pipe_name) {
            Ok(stream) => return Ok(stream),
            Err(error) if start.elapsed() >= DAEMON_STARTUP_TIMEOUT => {
                return Err(anyhow::Error::new(error)
                    .context("timed out waiting for Windows daemon named pipe"));
            }
            Err(_) => {}
        }
        if let Some(status) = child.try_wait()?
            && !status.success()
        {
            anyhow::bail!("Windows remote-server daemon exited early: {status}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[derive(Clone, Copy)]
struct ShareableHandle(usize);

impl ShareableHandle {
    fn cancel_io(self) {
        unsafe {
            let _ = CancelIoEx(HANDLE(self.0 as *mut std::ffi::c_void), None);
        }
    }
}

fn bridge_stdio_to_pipe(mut write_stream: LocalSocketStream) -> anyhow::Result<()> {
    let mut read_stream = duplicate_stream(&write_stream)?;
    let read_handle = ShareableHandle(read_stream.as_raw_handle() as usize);
    let write_handle = ShareableHandle(write_stream.as_raw_handle() as usize);
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();

    let finished_tx_stdin = finished_tx.clone();
    let _stdin_thread = std::thread::Builder::new()
        .name("proxy-stdin-fwd".into())
        .spawn(move || {
            let result = std::io::copy(&mut std::io::stdin(), &mut write_stream).map(|_| ());
            read_handle.cancel_io();
            let _ = finished_tx_stdin.send(result);
        })?;
    let _stdout_thread = std::thread::Builder::new()
        .name("proxy-stdout-fwd".into())
        .spawn(move || {
            let mut stdout = std::io::stdout().lock();
            let mut buffer = [0u8; 8192];
            let result = loop {
                let read = match read_stream.read(&mut buffer) {
                    Ok(0) => break Ok(()),
                    Ok(read) => read,
                    Err(error) => break Err(error),
                };
                if let Err(error) = stdout.write_all(&buffer[..read]) {
                    break Err(error);
                }
                if let Err(error) = stdout.flush() {
                    break Err(error);
                }
            };
            write_handle.cancel_io();
            let _ = finished_tx.send(result);
        })?;

    // 任一方向结束都意味着 SSH 通道应关闭。返回 worker 主线程即可结束进程，
    // 不等待另一个可能仍阻塞在同步 stdin 读取上的线程。
    finished_rx
        .recv()
        .map_err(|error| anyhow::anyhow!("Windows proxy bridge threads exited: {error}"))??;
    Ok(())
}

fn duplicate_stream(stream: &LocalSocketStream) -> std::io::Result<LocalSocketStream> {
    use windows::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let process = unsafe { GetCurrentProcess() };
    let source = HANDLE(stream.as_raw_handle());
    let mut target = HANDLE::default();
    unsafe {
        DuplicateHandle(
            process,
            source,
            process,
            &mut target,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(std::io::Error::other)?;
        Ok(LocalSocketStream::from_raw_handle(target.0))
    }
}
