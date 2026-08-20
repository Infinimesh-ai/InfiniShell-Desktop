//! Windows `remote-server-proxy`:通过 named pipe 连接常驻 daemon。

use std::hash::{Hash, Hasher};
use std::io::{Read as _, Write as _};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
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
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()?;
    let runtime_guard = runtime.enter();
    let startup_mutex = StartupMutex::acquire(identity_key)?;
    let pipe_name = pipe_name(identity_key);
    let stream = match connect_pipe(&pipe_name) {
        Ok(stream) => stream,
        Err(_) => start_daemon_and_connect(identity_key, &pipe_name)?,
    };
    drop(startup_mutex);
    drop(runtime_guard);
    runtime.block_on(bridge_stdio_to_pipe(stream))
}

fn start_daemon_and_connect(
    identity_key: &str,
    pipe_name: &str,
) -> anyhow::Result<NamedPipeClient> {
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
        match connect_pipe(pipe_name) {
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

pub(super) fn connect_pipe(pipe_name: &str) -> std::io::Result<NamedPipeClient> {
    ClientOptions::new().open(format!(r"\\.\pipe\{pipe_name}"))
}

async fn bridge_stdio_to_pipe(stream: NamedPipeClient) -> anyhow::Result<()> {
    let (mut pipe_reader, mut pipe_writer) = tokio::io::split(stream);
    let (stdin_tx, stdin_rx) = async_channel::bounded::<std::io::Result<Vec<u8>>>(8);
    let _stdin_thread = std::thread::Builder::new()
        .name("proxy-stdin-fwd".into())
        .spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buffer = [0u8; 8192];
            loop {
                let result = stdin.read(&mut buffer).map(|read| buffer[..read].to_vec());
                let should_finish = !matches!(result, Ok(ref bytes) if !bytes.is_empty());
                if stdin_tx.send_blocking(result).is_err() || should_finish {
                    break;
                }
            }
        })?;

    let (stdout_tx, stdout_rx) = async_channel::bounded::<Vec<u8>>(8);
    let (stdout_result_tx, stdout_result_rx) = async_channel::bounded(1);
    let _stdout_thread = std::thread::Builder::new()
        .name("proxy-stdout-fwd".into())
        .spawn(move || {
            let result = (|| {
                let mut stdout = std::io::stdout().lock();
                while let Ok(bytes) = stdout_rx.recv_blocking() {
                    stdout.write_all(&bytes)?;
                    stdout.flush()?;
                }
                Ok::<_, std::io::Error>(())
            })();
            let _ = stdout_result_tx.send_blocking(result);
        })?;

    let mut pipe_buffer = [0u8; 8192];
    let result = loop {
        tokio::select! {
            input = stdin_rx.recv() => {
                match input {
                    Ok(Ok(bytes)) if bytes.is_empty() => break Ok(()),
                    Ok(Ok(bytes)) => {
                        if let Err(error) = pipe_writer.write_all(&bytes).await {
                            break Err(error);
                        }
                        if let Err(error) = pipe_writer.flush().await {
                            break Err(error);
                        }
                    }
                    Ok(Err(error)) => break Err(error),
                    Err(_) => break Ok(()),
                }
            }
            read = pipe_reader.read(&mut pipe_buffer) => {
                match read {
                    Ok(0) => {
                        stdout_tx.close();
                        break stdout_result_rx
                            .recv()
                            .await
                            .unwrap_or_else(|_| Ok(()));
                    }
                    Ok(read) => {
                        if stdout_tx.send(pipe_buffer[..read].to_vec()).await.is_err() {
                            break Ok(());
                        }
                    }
                    Err(error) => break Err(error),
                }
            }
            stdout_result = stdout_result_rx.recv() => {
                break stdout_result.unwrap_or_else(|_| Ok(()));
            }
        }
    };
    stdout_tx.close();
    result.map_err(anyhow::Error::new)
}
