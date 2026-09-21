//! 域外监督者与 launchd 内 wrapper 的执行授权；普通终端和其他平台保持原有路径。

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, TcpListener};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use command::blocking::Command;
use command::managed::{
    MacosCoalition, MacosProcessIdentity, macos_boot_session, macos_peer_identity,
    macos_process_identity,
};
use serde::{Deserialize, Serialize};
use tempfile::{NamedTempFile, TempDir};
use uuid::Uuid;

use super::{
    CLEANUP_TIMEOUT, EXEC_CONTROL_ENV, ExitReason, ExitReceipt, GRACEFUL_EXIT_TIMEOUT,
    HANDSHAKE_TIMEOUT, MAX_RECORD_BYTES, Manifest, WORKER_COMMAND, accept_authorized,
    connect_authorized, copy_output_with_flush, execute_worker_executable, read_record, sha256,
    write_receipt,
};

pub(super) const CONTAINMENT: &str = "macos_resource_coalition";
const CLAIM_FILE: &str = "macos-coalition.json";
const NATIVE_FILE: &str = "macos-native.json";
const PROOF_FILE: &str = "macos-cleanup.json";
const MAX_FRAME_BYTES: usize = 1024 * 1024;
const LAUNCHCTL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SavedIdentity {
    pid: i32,
    pid_version: u32,
    unique_id: u64,
    resource_cid: u64,
}

impl From<MacosProcessIdentity> for SavedIdentity {
    fn from(identity: MacosProcessIdentity) -> Self {
        Self {
            pid: identity.pid,
            pid_version: identity.pid_version,
            unique_id: identity.unique_id,
            resource_cid: identity.resource_cid,
        }
    }
}

impl From<SavedIdentity> for MacosProcessIdentity {
    fn from(identity: SavedIdentity) -> Self {
        Self {
            pid: identity.pid,
            pid_version: identity.pid_version,
            unique_id: identity.unique_id,
            resource_cid: identity.resource_cid,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Claim {
    version: u32,
    generation: Uuid,
    manifest_sha256: String,
    label: String,
    boot_session: String,
    wrapper: SavedIdentity,
}

#[derive(Serialize, Deserialize)]
struct NativeClaim {
    generation: Uuid,
    claim_sha256: String,
    identity: SavedIdentity,
}

#[derive(Serialize, Deserialize)]
struct CleanupProof {
    version: u32,
    generation: Uuid,
    claim_sha256: String,
    native_sha256: Option<String>,
    job_removed: bool,
    resource_cid_destroyed: bool,
    native_wait_status: Option<i32>,
    execution_failed: bool,
}

// 环境只经过有界内存帧；不派生 Debug、不写日志和持久记录。
#[derive(Serialize, Deserialize)]
struct Environment {
    values: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Serialize, Deserialize)]
struct Prepared {
    generation: Uuid,
    identity: SavedIdentity,
}

#[derive(Serialize, Deserialize)]
struct NativeStatus {
    generation: Uuid,
    identity: SavedIdentity,
    wait_status: i32,
}

fn label(generation: Uuid) -> String {
    format!("dev.infinishell.cli-agent.{generation}")
}

fn encode<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(io::Error::other)
}

fn persist_record(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(io::Error::other("托管资源域记录过大"));
    }
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("缺少托管资源域记录目录"))?;
    let mut temporary = NamedTempFile::new_in(directory)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    // 先完整刷盘再发布，既有完整或不完整证明都不能被后续写入覆盖。
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    fs::File::open(directory)?.sync_all()
}

fn frame_write<T: Serialize>(stream: &mut UnixStream, value: &T) -> io::Result<()> {
    let bytes = encode(value)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::other("托管 IPC 帧超过容量"));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)
}

fn frame_read<T: for<'a> Deserialize<'a>>(stream: &mut UnixStream) -> io::Result<T> {
    let mut size = [0u8; 4];
    stream.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size > MAX_FRAME_BYTES {
        return Err(io::Error::other("托管 IPC 帧超过容量"));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn configure(stream: &UnixStream) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))
}

fn environment() -> Environment {
    Environment {
        values: std::env::vars_os()
            .filter(|(key, _)| key != EXEC_CONTROL_ENV)
            .map(|(key, value)| (key.into_vec(), value.into_vec()))
            .collect(),
    }
}

fn decode_environment(environment: Environment) -> io::Result<Vec<(OsString, OsString)>> {
    environment
        .values
        .into_iter()
        .map(|(key, value)| {
            if key.is_empty()
                || key.contains(&0)
                || key.contains(&b'=')
                || value.contains(&0)
                || key == EXEC_CONTROL_ENV.as_bytes()
            {
                return Err(io::Error::other("托管执行环境格式无效"));
            }
            Ok((OsString::from_vec(key), OsString::from_vec(value)))
        })
        .collect()
}

fn launchctl(arguments: &[&OsStr]) -> io::Result<ExitStatus> {
    let mut command = Command::new("/bin/launchctl");
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    let deadline = Instant::now() + LAUNCHCTL_TIMEOUT;
    let error = loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => break error,
        }
        if Instant::now() >= deadline {
            break io::Error::new(io::ErrorKind::TimedOut, "托管用户服务操作超时");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let kill = child.kill();
    match child.wait() {
        Ok(_) => Err(error),
        Err(wait_error) => Err(io::Error::other(format!(
            "{error}；服务控制进程回收失败：{wait_error}；终止结果：{kill:?}"
        ))),
    }
}

fn cleanup_operations(
    remove_job: impl FnOnce() -> io::Result<()>,
    clear_domain: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let removal = remove_job();
    // 服务移除失败仍必须尝试清理已领取的域；两个结果相互独立，不能被问号提前截断。
    let domain = clear_domain();
    match (removal, domain) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(removal), Err(domain)) => Err(io::Error::new(
            removal.kind(),
            format!("{removal}；资源域清理失败：{domain}"),
        )),
    }
}

struct Job {
    directory: PathBuf,
    service: String,
    registered: bool,
    bootstrap_pending: bool,
    cleanup_attempted: bool,
    coalition: Option<MacosCoalition>,
    output_completed: Option<mpsc::Receiver<io::Result<()>>>,
    errors_completed: Option<mpsc::Receiver<io::Result<()>>>,
    _socket_directory: TempDir,
    listener: UnixListener,
}

fn job_program_arguments(executable: &Path, path: &Path) -> io::Result<Vec<String>> {
    let text_path = |value: &Path| {
        value
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| io::Error::other("托管服务路径不是 UTF-8"))
    };
    let mut arguments = vec![text_path(executable)?];
    #[cfg(test)]
    if executable.extension() == Some(OsStr::new("sh")) {
        // launchd 测试夹具显式经系统 shell 执行；产品 worker 是 Mach-O，不经过此分支。
        arguments.insert(0, "/bin/sh".to_owned());
    }
    arguments.extend([
        WORKER_COMMAND.to_owned(),
        text_path(path)?,
        "--execute".to_owned(),
    ]);
    Ok(arguments)
}

impl Job {
    #[cfg(any(test, debug_assertions))]
    fn capture_launchctl_state(&self) {
        let output = Command::new("/bin/launchctl")
            .args([OsStr::new("print"), OsStr::new(&self.service)])
            .output();
        if let Ok(output) = output {
            let mut bytes = output.stdout;
            bytes.extend_from_slice(&output.stderr);
            let _ = fs::write(self.directory.join("launchctl-print.txt"), bytes);
        }
    }

    fn create(path: &Path, manifest: &Manifest, executable: &Path) -> io::Result<Self> {
        macos_boot_session()?;
        let directory = path
            .parent()
            .ok_or_else(|| io::Error::other("缺少托管目录"))?
            .to_owned();
        let socket_directory = tempfile::Builder::new()
            .prefix("is-cli-")
            .tempdir_in("/tmp")?;
        fs::set_permissions(socket_directory.path(), fs::Permissions::from_mode(0o700))?;
        let socket = socket_directory.path().join("control");
        if socket.as_os_str().as_bytes().len() >= 104 {
            return Err(io::Error::other("托管 IPC 路径过长"));
        }
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let job_label = label(manifest.generation);
        let domain = format!("gui/{}", unsafe { libc::geteuid() });
        let service = format!("{domain}/{job_label}");
        let text_path = |value: &Path| {
            value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| io::Error::other("托管服务路径不是 UTF-8"))
        };
        let mut config = plist::Dictionary::new();
        config.insert("Label".into(), plist::Value::String(job_label));
        let program_arguments = job_program_arguments(executable, path)?;
        config.insert(
            "ProgramArguments".into(),
            plist::Value::Array(
                program_arguments
                    .into_iter()
                    .map(plist::Value::String)
                    .collect(),
            ),
        );
        config.insert("KeepAlive".into(), plist::Value::Boolean(false));
        config.insert("AbandonProcessGroup".into(), plist::Value::Boolean(false));
        config.insert("ExitTimeOut".into(), plist::Value::Integer(2.into()));
        let mut routing = plist::Dictionary::new();
        routing.insert(
            EXEC_CONTROL_ENV.into(),
            plist::Value::String(format!("unix:{}", text_path(&socket)?)),
        );
        config.insert(
            "EnvironmentVariables".into(),
            plist::Value::Dictionary(routing),
        );
        #[cfg(any(test, debug_assertions))]
        let (standard_output, standard_error) = (
            text_path(&directory.join("macos-job.stdout"))?,
            text_path(&directory.join("macos-job.stderr"))?,
        );
        #[cfg(not(any(test, debug_assertions)))]
        let (standard_output, standard_error) = ("/dev/null".to_owned(), "/dev/null".to_owned());
        config.insert(
            "StandardOutPath".into(),
            plist::Value::String(standard_output),
        );
        config.insert(
            "StandardErrorPath".into(),
            plist::Value::String(standard_error),
        );
        let mut bytes = Vec::new();
        plist::Value::Dictionary(config)
            .to_writer_xml(&mut bytes)
            .map_err(io::Error::other)?;
        let plist_path = directory.join("macos-job.plist");
        persist_record(&plist_path, &bytes)?;
        let mut job = Self {
            directory,
            service,
            registered: false,
            bootstrap_pending: false,
            cleanup_attempted: false,
            coalition: None,
            output_completed: None,
            errors_completed: None,
            _socket_directory: socket_directory,
            listener,
        };
        // 只检查本代次随机 label；已有同名 job 不属于本次调用，不能在失败时删除它。
        let existing = launchctl(&[OsStr::new("print"), OsStr::new(&job.service)])?;
        if existing.code() != Some(113) {
            return Err(io::Error::other("无法确认本代次托管服务尚不存在"));
        }
        job.bootstrap_pending = true;
        let status = match launchctl(&[
            OsStr::new("bootstrap"),
            OsStr::new(&domain),
            plist_path.as_os_str(),
        ]) {
            Ok(status) => status,
            Err(error) => {
                // launchd 可能已经受理；本次独占 label 的结果未知时也必须尝试撤销。
                let cleanup = job.cleanup();
                return Err(match cleanup {
                    Ok(()) => error,
                    Err(cleanup_error) => io::Error::other(format!(
                        "{error}；启动未确认的服务清理失败：{cleanup_error}"
                    )),
                });
            }
        };
        job.bootstrap_pending = false;
        if !status.success() {
            return Err(io::Error::other("无法建立专属托管用户服务"));
        }
        job.registered = true;
        if !launchctl(&[OsStr::new("kickstart"), OsStr::new(&job.service)])?.success() {
            return Err(io::Error::other("无法启动等待授权的托管服务"));
        }
        Ok(job)
    }

    fn accept(
        &self,
        manifest: &Manifest,
        role: u8,
    ) -> io::Result<(UnixStream, MacosProcessIdentity)> {
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let mut stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) => {
                    #[cfg(any(test, debug_assertions))]
                    self.capture_launchctl_state();
                    return Err(error);
                }
            }
        };
        configure(&stream)?;
        let identity = macos_peer_identity(&stream)?;
        let mut header = [0u8; 33];
        stream.read_exact(&mut header)?;
        if header[..16] != *manifest.generation.as_bytes()
            || header[16..32] != *manifest.token.as_bytes()
            || header[32] != role
        {
            return Err(io::Error::other("托管 IPC 代次或通道授权不匹配"));
        }
        stream.write_all(&[1])?;
        Ok((stream, identity))
    }

    fn cleanup(&mut self) -> io::Result<()> {
        self.cleanup_attempted = true;
        let Self {
            registered,
            bootstrap_pending,
            service,
            coalition,
            ..
        } = self;
        cleanup_operations(
            || {
                if *registered || *bootstrap_pending {
                    let status = launchctl(&[OsStr::new("bootout"), OsStr::new(service)])?;
                    if !status.success() {
                        return Err(io::Error::other(format!(
                            "托管用户服务尚未确认移除：{status}"
                        )));
                    }
                    *registered = false;
                    *bootstrap_pending = false;
                }
                Ok(())
            },
            || {
                if let Some(coalition) = coalition {
                    coalition.terminate_and_confirm(CLEANUP_TIMEOUT)?;
                }
                Ok(())
            },
        )
    }

    fn finish_output(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut failures = Vec::new();
        let mut timed_out = false;
        for (name, completion) in [
            ("stdout", self.output_completed.take()),
            ("stderr", self.errors_completed.take()),
        ] {
            let Some(completion) = completion else {
                continue;
            };
            match completion.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("{name} 转发失败：{error}")),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    timed_out = true;
                    failures.push(format!("{name} 转发尚未确认完成"));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    failures.push(format!("{name} 转发确认通道已关闭"))
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(io::Error::new(
                if timed_out {
                    io::ErrorKind::TimedOut
                } else {
                    io::ErrorKind::Other
                },
                failures.join("；"),
            ))
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // 仅清理本次随机代次的已注册或启动结果未知 job；明确拒绝注册的同名 job 不归本对象。
        if !self.cleanup_attempted {
            let _ = self.cleanup();
        }
    }
}

fn connect(path: &Path, manifest: &Manifest, role: u8) -> io::Result<UnixStream> {
    let mut stream = UnixStream::connect(path)?;
    configure(&stream)?;
    stream.write_all(manifest.generation.as_bytes())?;
    stream.write_all(manifest.token.as_bytes())?;
    stream.write_all(&[role])?;
    let mut response = [0];
    stream.read_exact(&mut response)?;
    if response != [1] {
        return Err(io::Error::other("托管 IPC 连接未获授权"));
    }
    Ok(stream)
}

pub(super) fn run_execute(path: &Path, manifest: &Manifest) -> io::Result<()> {
    let route =
        std::env::var_os(EXEC_CONTROL_ENV).ok_or_else(|| io::Error::other("缺少托管 IPC 地址"))?;
    let socket = PathBuf::from(OsString::from_vec(
        route
            .as_bytes()
            .strip_prefix(b"unix:")
            .ok_or_else(|| io::Error::other("托管 IPC 地址格式无效"))?
            .to_vec(),
    ));
    let mut control = connect(&socket, manifest, b'c')?;
    let data = connect(&socket, manifest, b'i')?;
    let errors = connect(&socket, manifest, b'e')?;
    for stream in [&data, &errors] {
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(None)?;
    }
    let environment = decode_environment(frame_read(&mut control)?)?;
    let mut authorization = [0];
    control.read_exact(&mut authorization)?;
    if authorization != [1] {
        return Err(io::Error::other("托管 wrapper 未获执行授权"));
    }
    // 子 worker 在自己的 TCP 授权前不会 exec；因此能先固定其 PID version 再允许真实 CLI。
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let mut command = Command::new(execute_worker_executable()?);
    command
        .arg(WORKER_COMMAND)
        .arg(path)
        .arg("--execute")
        .env_clear()
        .envs(environment)
        .env(EXEC_CONTROL_ENV, listener.local_addr()?.to_string())
        .stdin(Stdio::from(OwnedFd::from(data.try_clone()?)))
        .stdout(Stdio::from(OwnedFd::from(data)))
        .stderr(Stdio::from(OwnedFd::from(errors)));
    let mut child = command.spawn()?;
    let result = (|| {
        let native_identity = macos_process_identity(child.id() as i32)?;
        let wrapper_identity = macos_process_identity(std::process::id() as i32)?;
        if native_identity.resource_cid != wrapper_identity.resource_cid {
            return Err(io::Error::other("托管子 worker 未继承资源域"));
        }
        let identity = SavedIdentity::from(native_identity);
        frame_write(
            &mut control,
            &Prepared {
                generation: manifest.generation,
                identity: identity.clone(),
            },
        )?;
        control.read_exact(&mut authorization)?;
        if authorization != [1] {
            return Err(io::Error::other("原生 CLI 未获执行授权"));
        }
        drop(accept_authorized(&listener, manifest)?);
        let status = child.wait()?;
        frame_write(
            &mut control,
            &NativeStatus {
                generation: manifest.generation,
                identity,
                wait_status: status.into_raw(),
            },
        )?;
        control.read_exact(&mut authorization)?;
        if authorization != [2] {
            return Err(io::Error::other("原生退出结果尚未确认接收"));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

enum Event {
    Stop(ExitReason),
    Native(io::Result<NativeStatus>),
}

struct Outcome {
    reason: ExitReason,
    status: ExitStatus,
    claim_bytes: Vec<u8>,
    native_bytes: Vec<u8>,
}

fn validate_native_status(
    status: NativeStatus,
    generation: Uuid,
    identity: &SavedIdentity,
) -> io::Result<ExitStatus> {
    if status.generation != generation
        || &status.identity != identity
        || (!libc::WIFEXITED(status.wait_status) && !libc::WIFSIGNALED(status.wait_status))
    {
        return Err(io::Error::other("原生退出状态与受管理进程不匹配"));
    }
    Ok(ExitStatus::from_raw(status.wait_status))
}

fn validate_after_exec(
    current: MacosProcessIdentity,
    original: &SavedIdentity,
) -> io::Result<MacosProcessIdentity> {
    if current.pid != original.pid
        || current.unique_id != original.unique_id
        || current.resource_cid != original.resource_cid
    {
        return Err(io::Error::other("原生进程 PID 已被复用"));
    }
    Ok(current)
}

fn execute(
    job: &mut Job,
    manifest: &Manifest,
    bytes: &[u8],
    mut parent_control: std::net::TcpStream,
) -> io::Result<Outcome> {
    let (events, receiver) = mpsc::channel();
    let parent_stopped = Arc::new(AtomicBool::new(false));
    let stop_flag = parent_stopped.clone();
    let parent_events = events.clone();
    let mut parent_reader = parent_control.try_clone()?;
    thread::spawn(move || {
        let mut request = [0];
        let reason = match parent_reader.read(&mut request) {
            Ok(1) if request == [2] => ExitReason::StopRequested,
            Ok(_) | Err(_) => ExitReason::HostDisconnected,
        };
        stop_flag.store(true, Ordering::Release);
        let _ = parent_events.send(Event::Stop(reason));
    });
    let (mut control, wrapper) = job
        .accept(manifest, b'c')
        .map_err(|error| io::Error::new(error.kind(), format!("控制通道未连接：{error}")))?;
    let coalition = MacosCoalition::claim(wrapper)?;
    job.coalition = Some(coalition);
    let (data, input_identity) = job
        .accept(manifest, b'i')
        .map_err(|error| io::Error::new(error.kind(), format!("数据通道未连接：{error}")))?;
    let (errors, error_identity) = job
        .accept(manifest, b'e')
        .map_err(|error| io::Error::new(error.kind(), format!("错误通道未连接：{error}")))?;
    if input_identity != wrapper || error_identity != wrapper {
        return Err(io::Error::other("托管 stdio 与控制通道身份不同"));
    }
    for stream in [&data, &errors] {
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(None)?;
    }
    let coalition = job
        .coalition
        .as_ref()
        .ok_or_else(|| io::Error::other("缺少托管资源域"))?;
    let claim = Claim {
        version: 1,
        generation: manifest.generation,
        manifest_sha256: sha256(bytes),
        label: label(manifest.generation),
        boot_session: coalition.boot_session().to_owned(),
        wrapper: wrapper.into(),
    };
    let claim_bytes = encode(&claim)?;
    persist_record(&job.directory.join(CLAIM_FILE), &claim_bytes)?;
    if parent_stopped.load(Ordering::Acquire) {
        return Err(io::Error::other("宿主已结束，拒绝启动托管进程"));
    }
    frame_write(&mut control, &environment())?;
    control.write_all(&[1])?;
    let prepared: Prepared = frame_read(&mut control)?;
    if prepared.generation != manifest.generation
        || coalition.identity(prepared.identity.pid)?
            != MacosProcessIdentity::from(prepared.identity.clone())
    {
        return Err(io::Error::other("授权前的原生 worker 身份不匹配"));
    }
    let native = NativeClaim {
        generation: manifest.generation,
        claim_sha256: sha256(&claim_bytes),
        identity: prepared.identity,
    };
    let native_bytes = encode(&native)?;
    persist_record(&job.directory.join(NATIVE_FILE), &native_bytes)?;

    let mut child_input = data.try_clone()?;
    let input_events = events.clone();
    thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut child_input);
        // 同一 socket 的读半部还需接收 stdout，不能用 drop 代替 stdin 的 EOF。
        let _ = child_input.shutdown(Shutdown::Write);
        let _ = input_events.send(Event::Stop(ExitReason::StdioClosed));
    });
    let (output_done, output_completed) = mpsc::channel();
    job.output_completed = Some(output_completed);
    let output_events = events.clone();
    let mut child_output = data;
    thread::spawn(move || {
        let mut output = io::stdout().lock();
        let result = copy_output_with_flush(&mut child_output, &mut output).map(|_| ());
        let _ = output_events.send(Event::Stop(ExitReason::StdioClosed));
        let _ = output_done.send(result);
    });
    let mut child_errors = errors;
    let (errors_done, errors_completed) = mpsc::channel();
    job.errors_completed = Some(errors_completed);
    thread::spawn(move || {
        let mut output = io::stderr().lock();
        let result = io::copy(&mut child_errors, &mut output).and_then(|_| output.flush());
        let _ = errors_done.send(result);
    });
    let mut native_reader = control.try_clone()?;
    native_reader.set_read_timeout(None)?;
    let native_events = events.clone();
    thread::spawn(move || {
        let _ = native_events.send(Event::Native(frame_read(&mut native_reader)));
    });
    // 首次 CID 和子 worker 身份已经落盘，这个字节才允许其 exec 成真实 CLI。
    if parent_stopped.load(Ordering::Acquire) {
        return Err(io::Error::other("宿主已结束，拒绝启动原生 CLI"));
    }
    control.write_all(&[1])?;
    parent_control.write_all(&[1])?;

    let first = receiver.recv().map_err(io::Error::other)?;
    let (reason, mut status) = match first {
        Event::Stop(reason) => (reason, None),
        Event::Native(result) => (
            ExitReason::NativeExit,
            Some(validate_native_status(
                result?,
                manifest.generation,
                &native.identity,
            )?),
        ),
    };
    let graceful_deadline = Instant::now() + GRACEFUL_EXIT_TIMEOUT;
    while status.is_none() && Instant::now() < graceful_deadline {
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(Event::Native(result)) => {
                status = Some(validate_native_status(
                    result?,
                    manifest.generation,
                    &native.identity,
                )?)
            }
            Ok(Event::Stop(_)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if status.is_none() {
        // exec 会改变 PID version；只在 PID、unique ID 与 CID 不变时刷新本次信号版本。
        if let Ok(current) = coalition.identity(native.identity.pid) {
            let current = validate_after_exec(current, &native.identity)?;
            if let Err(error) = coalition.signal_member(current, libc::SIGKILL) {
                if coalition
                    .identity(native.identity.pid)
                    .is_ok_and(|identity| identity == current)
                {
                    return Err(error);
                }
            }
        }
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        while status.is_none() && Instant::now() < deadline {
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(Event::Native(result)) => {
                    status = Some(validate_native_status(
                        result?,
                        manifest.generation,
                        &native.identity,
                    )?)
                }
                Ok(Event::Stop(_)) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }
    let status = status
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "原生 CLI 的准确退出状态未确认"))?;
    control.write_all(&[2])?;
    Ok(Outcome {
        reason,
        status,
        claim_bytes,
        native_bytes,
    })
}

pub(super) fn run_supervisor(path: &Path, manifest: &Manifest, bytes: &[u8]) -> io::Result<()> {
    let control = connect_authorized(manifest.parent_control, manifest)?;
    control.set_read_timeout(None)?;
    let mut job = Job::create(path, manifest, &execute_worker_executable()?)?;
    let result = execute(&mut job, manifest, bytes, control);
    let cleanup = job.cleanup();
    match (result, cleanup) {
        (Ok(outcome), Ok(())) => {
            if job.coalition.is_none() {
                return Err(io::Error::other("缺少首次验证的资源域身份"));
            }
            let proof = CleanupProof {
                version: 1,
                generation: manifest.generation,
                claim_sha256: sha256(&outcome.claim_bytes),
                native_sha256: Some(sha256(&outcome.native_bytes)),
                job_removed: true,
                resource_cid_destroyed: true,
                native_wait_status: Some(outcome.status.into_raw()),
                execution_failed: false,
            };
            persist_record(&job.directory.join(PROOF_FILE), &encode(&proof)?)?;
            write_receipt(
                &job.directory,
                &ExitReceipt {
                    version: 1,
                    generation: manifest.generation,
                    cleanup_confirmed: true,
                    containment: CONTAINMENT.to_owned(),
                    exit_reason: outcome.reason,
                    exit_code: outcome.status.code(),
                    manifest_sha256: sha256(bytes),
                },
            )?;
            // stdin EOF 通常先于 NativeExit；任何退出原因都要确认两条尾部已完整转发。
            job.finish_output()
        }
        (Err(error), Ok(())) => {
            // 原生结果未知仍返回失败；已证明销毁的独占域可以独立允许后续恢复。
            let proof = failed_execution_proof(&job, manifest);
            let confirmed = proof.is_ok();
            write_receipt(
                &job.directory,
                &ExitReceipt {
                    version: 1,
                    generation: manifest.generation,
                    cleanup_confirmed: confirmed,
                    containment: CONTAINMENT.to_owned(),
                    exit_reason: ExitReason::HostDisconnected,
                    exit_code: None,
                    manifest_sha256: sha256(bytes),
                },
            )?;
            let error = match job.finish_output() {
                Ok(()) => error,
                Err(output_error) => io::Error::other(format!("{error}；{output_error}")),
            };
            match proof {
                Ok(()) => Err(error),
                Err(proof_error) => Err(io::Error::other(format!(
                    "{error}；退出证明未保存：{proof_error}"
                ))),
            }
        }
        (result, Err(cleanup_error)) => {
            let (reason, code, error) = match result {
                Ok(outcome) => (outcome.reason, outcome.status.code(), cleanup_error),
                Err(error) => (
                    ExitReason::HostDisconnected,
                    None,
                    io::Error::other(format!("{error}；资源域清理失败：{cleanup_error}")),
                ),
            };
            write_receipt(
                &job.directory,
                &ExitReceipt {
                    version: 1,
                    generation: manifest.generation,
                    cleanup_confirmed: false,
                    containment: CONTAINMENT.to_owned(),
                    exit_reason: reason,
                    exit_code: code,
                    manifest_sha256: sha256(bytes),
                },
            )?;
            match job.finish_output() {
                Ok(()) => Err(error),
                Err(output_error) => Err(io::Error::other(format!("{error}；{output_error}"))),
            }
        }
    }
}

fn failed_execution_proof(job: &Job, manifest: &Manifest) -> io::Result<()> {
    let coalition = job
        .coalition
        .as_ref()
        .ok_or_else(|| io::Error::other("缺少首次验证的资源域"))?;
    let claim_bytes = read_record(&job.directory.join(CLAIM_FILE))?;
    let claim: Claim = serde_json::from_slice(&claim_bytes).map_err(io::Error::other)?;
    if claim.generation != manifest.generation
        || claim.wrapper.resource_cid != coalition.id()
        || claim.boot_session != coalition.boot_session()
        || !MacosCoalition::is_destroyed(coalition.id(), coalition.boot_session())?
    {
        return Err(io::Error::other("失败运行的资源域销毁未确认"));
    }
    let native_sha256 = match read_record(&job.directory.join(NATIVE_FILE)) {
        Ok(bytes) => Some(sha256(&bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let proof = CleanupProof {
        version: 1,
        generation: manifest.generation,
        claim_sha256: sha256(&claim_bytes),
        native_sha256,
        job_removed: !job.registered && !job.bootstrap_pending,
        resource_cid_destroyed: true,
        native_wait_status: None,
        execution_failed: true,
    };
    persist_record(&job.directory.join(PROOF_FILE), &encode(&proof)?)
}

pub(super) fn verify_cleanup(
    directory: &Path,
    manifest: &Manifest,
    bytes: &[u8],
    receipt: &ExitReceipt,
) -> io::Result<()> {
    let claim_bytes = read_record(&directory.join(CLAIM_FILE))?;
    let proof_bytes = read_record(&directory.join(PROOF_FILE))?;
    let claim: Claim = serde_json::from_slice(&claim_bytes).map_err(io::Error::other)?;
    let proof: CleanupProof = serde_json::from_slice(&proof_bytes).map_err(io::Error::other)?;
    let native = match &proof.native_sha256 {
        Some(expected) => {
            let bytes = read_record(&directory.join(NATIVE_FILE))?;
            if sha256(&bytes) != *expected {
                return Err(io::Error::other("原生身份记录摘要不匹配"));
            }
            let native: NativeClaim = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if native.generation != manifest.generation
                || native.identity.pid <= 0
                || native.identity.unique_id == 0
                || native.identity.resource_cid != claim.wrapper.resource_cid
                || native.claim_sha256 != sha256(&claim_bytes)
            {
                return Err(io::Error::other("原生进程不属于首次验证的资源域"));
            }
            Some(native)
        }
        None => None,
    };
    let status_matches = match (proof.execution_failed, proof.native_wait_status, native) {
        (false, Some(wait_status), Some(native)) => {
            validate_native_status(
                NativeStatus {
                    generation: proof.generation,
                    identity: native.identity.clone(),
                    wait_status,
                },
                manifest.generation,
                &native.identity,
            )?
            .code()
                == receipt.exit_code
        }
        (true, None, _) => receipt.exit_code.is_none(),
        (false, None, _) | (false, Some(_), None) | (true, Some(_), _) => false,
    };
    if claim.version != 1
        || proof.version != 1
        || claim.generation != manifest.generation
        || proof.generation != manifest.generation
        || claim.manifest_sha256 != sha256(bytes)
        || claim.label != label(manifest.generation)
        || Uuid::parse_str(&claim.boot_session).is_err()
        || claim.wrapper.resource_cid == 0
        || claim.wrapper.pid <= 0
        || claim.wrapper.unique_id == 0
        || proof.claim_sha256 != sha256(&claim_bytes)
        || !proof.job_removed
        || !proof.resource_cid_destroyed
        || !status_matches
    {
        return Err(io::Error::other("macOS 资源域退出证明与此运行代次不匹配"));
    }
    // 这是已落盘的历史销毁证明，不向旧 CID 查询或发信号；之后系统重启不否定已完成历史。
    Ok(())
}

#[cfg(test)]
#[path = "managed_process_macos_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "managed_process_macos_live_tests.rs"]
mod live_tests;
