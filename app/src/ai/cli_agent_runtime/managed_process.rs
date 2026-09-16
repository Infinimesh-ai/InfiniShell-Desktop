//! 应用托管进程的独立监督与退出回执；普通终端不经过此模块。

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
#[cfg(not(target_os = "macos"))]
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use command::r#async::{Child, ChildStdin, ChildStdout, Command};
#[cfg(not(target_os = "macos"))]
use command::managed::{Containment, ManagedTree, prepare_supervisor};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;

const WORKER_COMMAND: &str = "cli-agent-supervisor";
const EXEC_CONTROL_ENV: &str = "INFINISHELL_CLI_EXEC_CONTROL";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RECORD_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    launch_allowed: bool,
    generation: Uuid,
    token: Uuid,
    parent_control: SocketAddr,
    executable: PathBuf,
    arguments: Vec<OsString>,
    cwd: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExitReason {
    NativeExit,
    StopRequested,
    HostDisconnected,
    StdioClosed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExitReceipt {
    pub version: u32,
    pub generation: Uuid,
    pub cleanup_confirmed: bool,
    pub containment: String,
    pub exit_reason: ExitReason,
    pub exit_code: Option<i32>,
    manifest_sha256: String,
}

pub(crate) struct ManagedChild {
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    process: Option<Child>,
    control: Option<TcpStream>,
    state_dir: PathBuf,
    generation: Uuid,
}

impl ManagedChild {
    /// 调用方先释放已经取出的 stdin；即使未释放，控制连接仍会触发限时强制清理。
    pub async fn finish(mut self) -> io::Result<ExitReceipt> {
        self.stdin.take();
        if let Some(control) = self.control.as_mut() {
            // CLI 已自然退出时控制连接可以先关闭；最终以实际监督进程与回执核对。
            let _ = control.write_all(&[2]);
            let _ = control.shutdown(Shutdown::Both);
        }
        self.control.take();
        let mut child = self
            .process
            .take()
            .ok_or_else(|| io::Error::other("监督进程已被回收"))?;
        let status = child
            .status()
            .with_timeout(CLEANUP_TIMEOUT + HANDSHAKE_TIMEOUT)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "监督进程退出未确认"))??;
        if !status.success() {
            return Err(io::Error::other("托管监督进程退出失败"));
        }
        confirmed_exit(&self.state_dir, self.generation)?
            .ok_or_else(|| io::Error::other("缺少托管进程退出回执"))
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        // 不杀监督者：应用崩溃和异步任务取消都由控制管道 EOF 通知其清理。
        if let Some(control) = self.control.take() {
            let _ = control.shutdown(Shutdown::Both);
        }
    }
}

pub(crate) async fn spawn(
    state_dir: &Path,
    generation: Uuid,
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
) -> io::Result<ManagedChild> {
    let worker = supervisor_executable()?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let manifest = Manifest {
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: listener.local_addr()?,
        executable: executable.to_owned(),
        arguments: arguments.to_owned(),
        cwd: cwd.to_owned(),
    };
    let directory = create_generation_directory(state_dir, generation)?;
    let manifest_path = directory.join("manifest.json");
    write_new_record(
        &manifest_path,
        &serde_json::to_vec(&manifest).map_err(io::Error::other)?,
    )?;
    let mut command = Command::new(worker);
    command
        .arg(WORKER_COMMAND)
        .arg(&manifest_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut process = command.spawn()?;
    let expected = manifest.clone();
    let control = blocking::unblock(move || {
        let mut stream = accept_authorized(&listener, &expected)?;
        let mut ready = [0];
        stream.read_exact(&mut ready)?;
        if ready != [1] {
            return Err(io::Error::other("托管执行未获得进程所有权"));
        }
        Ok::<_, io::Error>(stream)
    })
    .await?;
    Ok(ManagedChild {
        stdin: process.stdin.take(),
        stdout: process.stdout.take(),
        process: Some(process),
        control: Some(control),
        state_dir: state_dir.to_owned(),
        generation,
    })
}

/// 独占并永久关闭尚未领取的代次；已有启动账本绝不被“未启动”结论覆盖。
pub(crate) fn record_not_started(
    state_dir: &Path,
    generation: Uuid,
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
) -> io::Result<ExitReceipt> {
    let directory = create_generation_directory(state_dir, generation)?;
    let manifest = Manifest {
        version: 1,
        launch_allowed: false,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:0".parse().expect("固定本机地址有效"),
        executable: executable.to_owned(),
        arguments: arguments.to_owned(),
        cwd: cwd.to_owned(),
    };
    let bytes = serde_json::to_vec(&manifest).map_err(io::Error::other)?;
    write_new_record(&directory.join("manifest.json"), &bytes)?;
    let receipt = ExitReceipt {
        version: 1,
        generation,
        cleanup_confirmed: true,
        containment: "not_started".to_owned(),
        exit_reason: ExitReason::StopRequested,
        exit_code: None,
        manifest_sha256: sha256(&bytes),
    };
    write_receipt(&directory, &receipt)?;
    Ok(receipt)
}

fn supervisor_executable() -> io::Result<PathBuf> {
    #[cfg(test)]
    {
        let path = std::env::var_os("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("真实 adapter 测试需指定已构建的监督 worker 二进制"))?;
        if !path.is_absolute() || !path.is_file() {
            return Err(io::Error::other("监督 worker 二进制路径无效"));
        }
        Ok(path)
    }
    #[cfg(not(test))]
    std::env::current_exe()
}

fn generation_directory(state_dir: &Path, generation: Uuid) -> PathBuf {
    state_dir
        .join("cli-agent-processes")
        .join(generation.to_string())
}

fn create_generation_directory(state_dir: &Path, generation: Uuid) -> io::Result<PathBuf> {
    let parent = state_dir.join("cli-agent-processes");
    fs::create_dir_all(&parent)?;
    let directory = parent.canonicalize()?.join(generation.to_string());
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    // 目录原子创建即唯一 generation 声明；不复用任何旧 manifest 或回执。
    builder.create(&directory)?;
    Ok(directory)
}

fn write_new_record(path: &Path, contents: &[u8]) -> io::Result<()> {
    if contents.len() as u64 > MAX_RECORD_BYTES {
        return Err(io::Error::other("托管进程记录过大"));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

fn read_record(path: &Path) -> io::Result<Vec<u8>> {
    let information = fs::symlink_metadata(path)?;
    if !information.is_file() || information.len() > MAX_RECORD_BYTES {
        return Err(io::Error::other("托管进程记录不是受控普通文件"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if information.nlink() != 1 || information.mode() & 0o077 != 0 {
            return Err(io::Error::other("托管进程记录权限不正确"));
        }
    }
    fs::read(path)
}

fn read_manifest(path: &Path) -> io::Result<(Manifest, Vec<u8>)> {
    let bytes = read_record(path)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("缺少托管记录目录"))?;
    if manifest.version != 1
        || !manifest.parent_control.ip().is_loopback()
        || directory.file_name() != Some(std::ffi::OsStr::new(&manifest.generation.to_string()))
        || !manifest.executable.is_absolute()
        || !manifest.cwd.is_absolute()
    {
        return Err(io::Error::other("托管进程启动契约不匹配"));
    }
    Ok((manifest, bytes))
}

pub(crate) fn confirmed_exit(
    state_dir: &Path,
    generation: Uuid,
) -> io::Result<Option<ExitReceipt>> {
    let directory = generation_directory(state_dir, generation);
    let path = directory.join("exit.json");
    let bytes = match read_record(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let receipt: ExitReceipt = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let (manifest, manifest_bytes) = read_manifest(&directory.join("manifest.json"))?;
    if receipt.version != 1
        || receipt.generation != generation
        || manifest.generation != generation
        || !receipt.cleanup_confirmed
        // 旧版 macOS 回执只证明原进程组；CLI 异常退出时另组工具可能仍在运行。
        || (receipt.containment == "unix_process_group" && receipt.exit_code != Some(0))
        || receipt.manifest_sha256 != sha256(&manifest_bytes)
        || match (manifest.launch_allowed, receipt.containment.as_str()) {
            (true, "linux_subtree" | "windows_job" | "unix_process_group")
            | (false, "not_started") => false,
            #[cfg(target_os = "macos")]
            (true, "macos_resource_coalition") => false,
            (true, _) | (false, _) => true,
        }
    {
        return Err(io::Error::other("退出回执与此运行代次不匹配"));
    }
    #[cfg(target_os = "macos")]
    if receipt.containment == macos::CONTAINMENT {
        macos::verify_cleanup(&directory, &manifest, &manifest_bytes, &receipt)?;
    }
    Ok(Some(receipt))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn configure_stream(stream: &TcpStream) -> io::Result<()> {
    // macOS 的 accept 会继承 listener 的非阻塞属性，后续握手必须按超时等待完整字节。
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))
}

fn connect_authorized(address: SocketAddr, manifest: &Manifest) -> io::Result<TcpStream> {
    if !address.ip().is_loopback() {
        return Err(io::Error::other("托管控制连接必须是本机地址"));
    }
    let mut stream = TcpStream::connect_timeout(&address, HANDSHAKE_TIMEOUT)?;
    configure_stream(&stream)?;
    stream.write_all(manifest.generation.as_bytes())?;
    stream.write_all(manifest.token.as_bytes())?;
    let mut response = [0];
    stream.read_exact(&mut response)?;
    if response != [1] {
        return Err(io::Error::other("托管控制连接未获授权"));
    }
    Ok(stream)
}

fn accept_authorized(listener: &TcpListener, manifest: &Manifest) -> io::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => break stream,
            Ok(_) => return Err(io::Error::other("拒绝非本机托管连接")),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    };
    configure_stream(&stream)?;
    let mut identity = [0; 32];
    stream.read_exact(&mut identity)?;
    if identity[..16] != *manifest.generation.as_bytes()
        || identity[16..] != *manifest.token.as_bytes()
    {
        return Err(io::Error::other("托管控制连接代次或授权不匹配"));
    }
    stream.write_all(&[1])?;
    Ok(stream)
}

/// 隐藏 worker 入口；GUI 与 TUI 均在初始化应用界面之前分派到此处。
pub(crate) fn run_worker(path: &Path, execute: bool) -> io::Result<()> {
    let (manifest, bytes) = read_manifest(path)?;
    if !manifest.launch_allowed {
        return Err(io::Error::other("此运行代次已永久声明为未启动"));
    }
    if execute {
        #[cfg(target_os = "macos")]
        if std::env::var_os(EXEC_CONTROL_ENV)
            .is_some_and(|value| value.as_encoded_bytes().starts_with(b"unix:"))
        {
            return macos::run_execute(path, &manifest);
        }
        return run_exec_worker(&manifest);
    }
    #[cfg(target_os = "macos")]
    return macos::run_supervisor(path, &manifest, &bytes);
    #[cfg(not(target_os = "macos"))]
    run_process_tree_worker(path, &manifest, &bytes)
}

#[cfg(not(target_os = "macos"))]
fn run_process_tree_worker(path: &Path, manifest: &Manifest, bytes: &[u8]) -> io::Result<()> {
    prepare_supervisor()?;
    let mut control = connect_authorized(manifest.parent_control, manifest)?;
    control.set_read_timeout(None)?;
    let child_listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let mut command =
        command::blocking::Command::new_with_managed_process_group(std::env::current_exe()?);
    command
        .arg(WORKER_COMMAND)
        .arg(path)
        .arg("--execute")
        .env(EXEC_CONTROL_ENV, child_listener.local_addr()?.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut tree = ManagedTree::claim(command.spawn()?)?;
    // 严格 Job/进程组已经就绪后才发送授权，内部 worker 此前不能派生真实 CLI。
    let execution_control = accept_authorized(&child_listener, manifest)?;
    drop(execution_control);
    let mut child_input = tree
        .child_mut()
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("缺少托管 stdin"))?;
    let mut child_output = tree
        .child_mut()
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("缺少托管 stdout"))?;
    let (events, receiver) = mpsc::channel();
    let input_events = events.clone();
    thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut child_input);
        drop(child_input);
        let _ = input_events.send(ExitReason::StdioClosed);
    });
    let output_events = events.clone();
    let (output_done, output_completed) = mpsc::channel();
    thread::spawn(move || {
        let result = io::copy(&mut child_output, &mut io::stdout().lock());
        let _ = output_events.send(ExitReason::StdioClosed);
        let _ = output_done.send(result.map(|_| ()));
    });
    control.write_all(&[1])?;
    thread::spawn(move || {
        let mut request = [0];
        let reason = match control.read(&mut request) {
            Ok(1) if request == [2] => ExitReason::StopRequested,
            Ok(_) | Err(_) => ExitReason::HostDisconnected,
        };
        let _ = events.send(reason);
    });
    let reason = loop {
        if tree.root_exited()? {
            break ExitReason::NativeExit;
        }
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok(reason) => break reason,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break ExitReason::HostDisconnected,
        }
    };
    // 给 stdin EOF 一个短暂的原生收尾窗口；不以该窗口或空闲状态代替最终组/Job 验证。
    let graceful_deadline = Instant::now() + Duration::from_millis(500);
    while !tree.root_exited()? && Instant::now() < graceful_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let containment = match tree.containment() {
        Containment::LinuxSubtree => "linux_subtree",
        Containment::WindowsJob => "windows_job",
        Containment::UnixProcessGroup => "unix_process_group",
    };
    let status = tree.terminate_and_confirm(CLEANUP_TIMEOUT)?;
    let receipt = ExitReceipt {
        version: 1,
        generation: manifest.generation,
        // CLI 正常派生的工具也会另建进程组，异常退出不能作为整个任务已停止的证明。
        cleanup_confirmed: containment != "unix_process_group" || status.success(),
        containment: containment.to_owned(),
        exit_reason: reason,
        exit_code: status.code(),
        manifest_sha256: sha256(bytes),
    };
    write_receipt(
        path.parent()
            .ok_or_else(|| io::Error::other("缺少托管目录"))?,
        &receipt,
    )?;
    if reason == ExitReason::NativeExit {
        output_completed
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "托管输出尚未转发完成"))??;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "managed_process_macos.rs"]
mod macos;

fn write_receipt(directory: &Path, receipt: &ExitReceipt) -> io::Result<()> {
    let mut temporary = NamedTempFile::new_in(directory)?;
    let bytes = serde_json::to_vec(receipt).map_err(io::Error::other)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(directory.join("exit.json"))
        .map_err(|error| error.error)?;
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

fn run_exec_worker(manifest: &Manifest) -> io::Result<()> {
    let address = std::env::var(EXEC_CONTROL_ENV)
        .map_err(io::Error::other)?
        .parse::<SocketAddr>()
        .map_err(io::Error::other)?;
    drop(connect_authorized(address, manifest)?);
    let mut command = command::blocking::Command::new(&manifest.executable);
    command
        .args(&manifest.arguments)
        .current_dir(&manifest.cwd)
        .env_remove(EXEC_CONTROL_ENV)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use command::unix::CommandExt as _;
        Err(command.exec())
    }
    #[cfg(windows)]
    {
        command.inherit_managed_job();
        let status = command.status()?;
        if status.success() {
            Ok(())
        } else {
            let code = status
                .code()
                .ok_or_else(|| io::Error::other("缺少原生 CLI 退出码"))?;
            // 这里只退出已等到原生命令结束的内部 worker；专属 Job 仍由外层监督者持有。
            // 返回普通 Err 会让可执行入口把所有原生非零码折叠为 1。
            std::process::exit(code)
        }
    }
}

#[cfg(test)]
#[path = "managed_process_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "managed_process_live_tests.rs"]
mod live_tests;
