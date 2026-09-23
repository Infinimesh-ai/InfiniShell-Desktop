//! 应用托管进程的独立监督与退出回执；普通终端不经过此模块。

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{self, Read as _, Seek as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
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

pub(super) const WORKER_COMMAND: &str = "cli-agent-supervisor";
const EXEC_CONTROL_ENV: &str = "INFINISHELL_CLI_EXEC_CONTROL";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const GRACEFUL_EXIT_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const SPAWN_ATTEMPT_RECORD: &str = "spawn-attempt.json";
const SPAWN_REJECTED_RECORD: &str = "spawn-rejected.json";
const LAUNCH_BINDING_RECORD: &str = "launch-binding.json";
const EXIT_BINDING_RECORD: &str = "exit-binding.json";

fn copy_output_with_flush(
    reader: &mut impl io::Read,
    writer: &mut impl io::Write,
) -> io::Result<u64> {
    let mut buffer = [0_u8; 8 * 1024];
    let mut copied = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(copied);
        }
        writer.write_all(&buffer[..read])?;
        // CLI 的原生 ACK、审批和进度必须在进程仍运行时立即到达 adapter。
        writer.flush()?;
        copied = copied.saturating_add(read as u64);
    }
}

/// 更新器仅可覆盖发行选择与安装路径，不能借监督入口更改审批或注入认证。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedEnvironment {
    pub values: Vec<(OsString, OsString)>,
    pub remove: Vec<OsString>,
}

/// 调用方在准备更新事务时冻结的文件身份；最终 worker 在派生前再次核对。
///
/// 这是 opt-in 契约。旧调用方传空列表时维持原有行为；非空时必须包含实际程序，
/// 还可同时包含包管理器、辅助程序和注册文件等更新事务依赖。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectedFileIdentity {
    path: PathBuf,
    canonical_path: PathBuf,
    sha256: String,
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file_id: Option<ExpectedFileId>,
}

/// 来源层完成结构化参数和角色解析后生成的不可变绑定摘要。
///
/// 该类型只建立事务数据合同；各平台在实现真正的 fd/snapshot/lease 执行原语前，
/// `validate_update_execution` 必须保持 fail closed，不能退回 pathname 派生。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AtomicLaunchKind {
    NativeFile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedLaunchBinding {
    digest: String,
    kind: Option<AtomicLaunchKind>,
}

impl PreparedLaunchBinding {
    pub(crate) fn new(digest: String) -> io::Result<Self> {
        Self::with_kind(digest, None)
    }

    pub(crate) fn native_file(digest: String) -> io::Result<Self> {
        Self::with_kind(digest, Some(AtomicLaunchKind::NativeFile))
    }

    pub(crate) fn from_persisted(digest: String, kind: Option<&str>) -> io::Result<Self> {
        let kind = match kind {
            None => None,
            Some("native_file") => Some(AtomicLaunchKind::NativeFile),
            Some(_) => {
                return Err(io::Error::other(
                    "managed_process.launch_binding_kind_invalid",
                ));
            }
        };
        Self::with_kind(digest, kind)
    }

    fn with_kind(digest: String, kind: Option<AtomicLaunchKind>) -> io::Result<Self> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(io::Error::other(
                "managed_process.launch_binding_digest_invalid",
            ));
        }
        Ok(Self { digest, kind })
    }

    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }

    pub(crate) fn kind_name(&self) -> Option<&'static str> {
        match self.kind {
            Some(AtomicLaunchKind::NativeFile) => Some("native_file"),
            None => None,
        }
    }

    pub(crate) fn is_native_file(&self) -> bool {
        self.kind == Some(AtomicLaunchKind::NativeFile)
    }

    pub(crate) fn validate_update_execution(&self) -> io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if self.kind == Some(AtomicLaunchKind::NativeFile) {
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "managed_process.atomic_update_launch_not_implemented",
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFileId {
    volume: u64,
    index: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OpenedFileMetadata {
    size: u64,
    file_id: Option<ExpectedFileId>,
}

impl ExpectedFileIdentity {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn capture(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "managed_process.expected_file_path_not_absolute",
            ));
        }
        let canonical_path = path.canonicalize()?;
        let mut file = open_expected_file(&canonical_path)?;
        Self::capture_opened(path, canonical_path, &mut file)
    }

    fn capture_opened(path: &Path, canonical_path: PathBuf, file: &mut File) -> io::Result<Self> {
        let metadata = opened_file_metadata(file)?;
        let sha256 = sha256_file(file)?;
        if opened_file_metadata(file)? != metadata {
            return Err(io::Error::other(
                "managed_process.expected_file_changed_while_reading",
            ));
        }
        Ok(Self {
            path: path.to_owned(),
            canonical_path,
            sha256,
            size: metadata.size,
            file_id: metadata.file_id,
        })
    }
}

fn open_expected_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    options.open(path)
}

fn opened_file_metadata(file: &File) -> io::Result<OpenedFileMetadata> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other(
            "managed_process.expected_file_not_regular",
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;

        return Ok(OpenedFileMetadata {
            size: metadata.len(),
            file_id: Some(ExpectedFileId {
                volume: metadata.dev(),
                index: metadata.ino(),
            }),
        });
    }

    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle as _;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };

        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
            .map_err(io::Error::other)?;
        if !windows_file_attributes_are_plain(information.dwFileAttributes) {
            return Err(io::Error::other(
                "managed_process.expected_file_not_regular",
            ));
        }
        return Ok(OpenedFileMetadata {
            size: metadata.len(),
            file_id: Some(ExpectedFileId {
                volume: u64::from(information.dwVolumeSerialNumber),
                index: (u64::from(information.nFileIndexHigh) << 32)
                    | u64::from(information.nFileIndexLow),
            }),
        });
    }

    #[cfg(not(any(unix, windows)))]
    Ok(OpenedFileMetadata {
        size: metadata.len(),
        file_id: None,
    })
}

#[cfg(any(windows, test))]
fn windows_file_attributes_are_plain(attributes: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

impl ManagedEnvironment {
    fn validate(&self) -> io::Result<()> {
        let mut names = std::collections::HashSet::new();
        for (name, value) in &self.values {
            if !matches!(
                name.to_str(),
                Some(
                    "PATH"
                        | "CODEX_INSTALL_DIR"
                        | "CODEX_HOME"
                        | "CODEX_NON_INTERACTIVE"
                        | "CODEX_RELEASE"
                        | "HOMEBREW_NO_AUTO_UPDATE"
                        | "HOMEBREW_NO_INSTALL_CLEANUP"
                        | "CLAUDE_CONFIG_DIR"
                )
            ) || !names.insert(name)
                || value.as_encoded_bytes().contains(&0)
                || value.as_encoded_bytes().len() > 32 * 1024
                || name == "CLAUDE_CONFIG_DIR" && !Path::new(value).is_absolute()
            {
                return Err(io::Error::other("更新环境覆盖不符合启动契约"));
            }
        }
        for name in &self.remove {
            if !matches!(
                name.to_str(),
                Some(
                    "CODEX_MANAGED_BY_NPM"
                        | "CODEX_MANAGED_BY_BUN"
                        | "CODEX_MANAGED_BY_PNPM"
                        | "CODEX_MANAGED_BY_VITE_PLUS"
                )
            ) || !names.insert(name)
            {
                return Err(io::Error::other("更新环境移除不符合启动契约"));
            }
        }
        Ok(())
    }

    fn validate_for_generation(
        &self,
        state_dir: &Path,
        generation: Uuid,
        arguments: &[OsString],
    ) -> io::Result<()> {
        self.validate()?;
        for (name, value) in &self.values {
            if name != "CLAUDE_CONFIG_DIR" {
                continue;
            }
            let expected = state_dir
                .canonicalize()?
                .join(format!("claude-update-{generation}"))
                .join("config");
            if Path::new(value) != expected
                || arguments.len() != 3
                || arguments[0] != "--settings"
                || arguments[2] != "update"
                || !matches!(
                    arguments[1].to_str(),
                    Some(
                        "{\"autoUpdatesChannel\":\"latest\"}"
                            | "{\"autoUpdatesChannel\":\"stable\"}"
                    )
                )
            {
                return Err(io::Error::other("Claude 更新配置未绑定本次代次与更新命令"));
            }
            // 清理后目录可缺失；尚存在的任何一层不能把监督者重定向到用户目录。
            for ancestor in expected.ancestors() {
                match fs::symlink_metadata(ancestor) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(io::Error::other("Claude 更新配置路径不能经过链接"));
                    }
                    Ok(metadata) => {
                        #[cfg(windows)]
                        {
                            use std::os::windows::fs::MetadataExt as _;
                            use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
                            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                                return Err(io::Error::other(
                                    "Claude 更新配置路径不能经过重解析点",
                                ));
                            }
                        }
                        #[cfg(not(windows))]
                        let _ = metadata;
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    isolated_home: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    environment: Option<ManagedEnvironment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    expected_files: Vec<ExpectedFileIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    atomic_launch_kind: Option<AtomicLaunchKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    atomic_cwd: Option<AtomicDirectoryIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AtomicDirectoryIdentity {
    path: PathBuf,
    canonical_path: PathBuf,
    file_id: ExpectedFileId,
}

impl AtomicDirectoryIdentity {
    fn capture(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::other("managed_process.atomic_cwd_not_absolute"));
        }
        let canonical_path = path.canonicalize()?;
        #[cfg(unix)]
        let file_id = {
            let directory = open_atomic_directory(&canonical_path)?;
            let metadata = directory.metadata()?;
            if !metadata.is_dir() {
                return Err(io::Error::other("managed_process.atomic_cwd_not_directory"));
            }
            use std::os::unix::fs::MetadataExt as _;
            ExpectedFileId {
                volume: metadata.dev(),
                index: metadata.ino(),
            }
        };
        #[cfg(windows)]
        let file_id = atomic_windows::capture_directory_id(&canonical_path)?;
        #[cfg(not(any(unix, windows)))]
        let file_id = return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "managed_process.atomic_cwd_binding_not_implemented",
        ));
        Ok(Self {
            path: path.to_owned(),
            canonical_path,
            file_id,
        })
    }

    #[cfg(unix)]
    fn enter(&self) -> io::Result<()> {
        let directory = self.open_verified()?;
        if unsafe { libc::fchdir(std::os::fd::AsRawFd::as_raw_fd(&directory)) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(unix)]
    fn open_verified(&self) -> io::Result<File> {
        let directory = open_atomic_directory(&self.canonical_path)?;
        let metadata = directory.metadata()?;
        use std::os::unix::fs::MetadataExt as _;
        if !metadata.is_dir()
            || metadata.dev() != self.file_id.volume
            || metadata.ino() != self.file_id.index
        {
            return Err(io::Error::other("managed_process.atomic_cwd_changed"));
        }
        Ok(directory)
    }
}

#[cfg(unix)]
fn open_atomic_directory(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

/// 在调用 OS spawn 前持久化的不可变见证。
///
/// 仅有此记录不能证明进程已经或尚未启动；它只保证从这一刻开始，崩溃恢复必须保持
/// `Unconfirmed`，直到监督者退出回执或同一次 spawn 的明确拒绝记录出现。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnAttempt {
    version: u32,
    generation: Uuid,
    attempt: Uuid,
    manifest_sha256: String,
}

/// `Command::spawn` 明确返回错误后写入的不可变见证。
///
/// 写入前没有任何 await 点；因此只有该记录与 attempt、manifest 三者完全绑定时，恢复
/// 才能断言监督者没有启动。缺失、截断或身份不一致都不能解锁自动重投。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnRejected {
    version: u32,
    generation: Uuid,
    attempt: Uuid,
    manifest_sha256: String,
}

/// 将来源层摘要绑定到不可变 manifest；不改变普通托管任务的旧 manifest 格式。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchBindingRecord {
    version: u32,
    generation: Uuid,
    binding_digest: String,
    manifest_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<AtomicLaunchKind>,
}

/// 将退出回执再次绑定到相同摘要和 manifest，恢复时三者必须同时匹配。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitBindingRecord {
    version: u32,
    generation: Uuid,
    binding_digest: String,
    manifest_sha256: String,
    receipt_sha256: String,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ProcessCompletion {
    NotStarted,
    Exited(ExitReceipt),
    Unconfirmed,
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
        self.wait_for_exit().await
    }

    /// 调用方先释放已经取出的 stdin；保留控制连接，让监督者据真实 stdin EOF 或原生退出收尾。
    pub async fn finish_after_stdin_close(mut self) -> io::Result<ExitReceipt> {
        self.stdin.take();
        self.wait_for_exit().await
    }

    async fn wait_for_exit(mut self) -> io::Result<ExitReceipt> {
        let mut child = self
            .process
            .take()
            .ok_or_else(|| io::Error::other("监督进程已被回收"))?;
        self.wait_for_confirmed_exit(child.status(), CLEANUP_TIMEOUT + HANDSHAKE_TIMEOUT)
            .await
    }

    async fn wait_for_confirmed_exit(
        self,
        status: impl Future<Output = io::Result<ExitStatus>>,
        timeout: Duration,
    ) -> io::Result<ExitReceipt> {
        // 正常关闭时控制连接随 self 保留至退出与回执核验完成；超时或取消仍由 Drop 通知清理。
        let status = status
            .with_timeout(timeout)
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
    spawn_with_isolated_home(state_dir, generation, executable, arguments, cwd, None).await
}

pub(crate) async fn spawn_with_isolated_home(
    state_dir: &Path,
    generation: Uuid,
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
    isolated_home: Option<&Path>,
) -> io::Result<ManagedChild> {
    spawn_configured(
        state_dir,
        generation,
        executable,
        arguments,
        cwd,
        isolated_home,
        None,
        Vec::new(),
        None,
    )
    .await
}

pub(crate) async fn spawn_with_environment(
    state_dir: &Path,
    generation: Uuid,
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
    environment: ManagedEnvironment,
) -> io::Result<ManagedChild> {
    spawn_with_environment_and_expected_files(
        state_dir,
        generation,
        executable,
        arguments,
        cwd,
        environment,
        Vec::new(),
    )
    .await
}

/// 使用调用方预先冻结的文件身份启动更新命令。
///
/// 非空身份列表必须包含 `executable`；最终 worker 会在真实派生前核对列表中的每个文件。
pub(crate) async fn spawn_with_environment_and_expected_files(
    state_dir: &Path,
    generation: Uuid,
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
    environment: ManagedEnvironment,
    expected_files: Vec<ExpectedFileIdentity>,
) -> io::Result<ManagedChild> {
    environment.validate_for_generation(state_dir, generation, arguments)?;
    validate_expected_files_contract(executable, &expected_files)?;
    spawn_configured(
        state_dir,
        generation,
        executable,
        arguments,
        cwd,
        None,
        Some(environment),
        expected_files,
        None,
    )
    .await
}

/// 更新专用入口必须携带结构化绑定；未实现的平台在创建 generation 前拒绝。
pub(crate) async fn spawn_bound_update(
    state_dir: &Path,
    generation: Uuid,
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
    environment: ManagedEnvironment,
    expected_files: Vec<ExpectedFileIdentity>,
    binding: &PreparedLaunchBinding,
) -> io::Result<ManagedChild> {
    environment.validate_for_generation(state_dir, generation, arguments)?;
    validate_expected_files_contract(executable, &expected_files)?;
    if !cwd.is_absolute() {
        return Err(io::Error::other("managed_process.update_cwd_not_absolute"));
    }
    binding.validate_update_execution()?;
    spawn_configured(
        state_dir,
        generation,
        executable,
        arguments,
        cwd,
        None,
        Some(environment),
        expected_files,
        Some(binding),
    )
    .await
}

async fn spawn_configured(
    state_dir: &Path,
    generation: Uuid,
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
    isolated_home: Option<&Path>,
    environment: Option<ManagedEnvironment>,
    expected_files: Vec<ExpectedFileIdentity>,
    binding: Option<&PreparedLaunchBinding>,
) -> io::Result<ManagedChild> {
    let worker = supervisor_executable()?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let atomic_launch_kind = binding.and_then(|binding| binding.kind);
    let atomic_cwd = atomic_launch_kind
        .map(|_| AtomicDirectoryIdentity::capture(cwd))
        .transpose()?;
    let manifest = Manifest {
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: listener.local_addr()?,
        executable: executable.to_owned(),
        arguments: arguments.to_owned(),
        cwd: cwd.to_owned(),
        isolated_home: isolated_home.map(Path::to_owned),
        environment,
        expected_files,
        atomic_launch_kind,
        atomic_cwd,
    };
    let (directory, manifest_bytes) = create_launch_manifest(state_dir, &manifest)?;
    if let Some(binding) = binding {
        write_launch_binding(&directory, &manifest, &manifest_bytes, binding)?;
    }
    let manifest_path = directory.join("manifest.json");
    let mut command = Command::new(worker);
    command
        .arg(WORKER_COMMAND)
        .arg(&manifest_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    #[cfg(test)]
    command.stderr(Stdio::from(fs::File::create(
        directory.join("supervisor.stderr"),
    )?));
    #[cfg(not(test))]
    command.stderr(Stdio::null());
    // attempt 必须先于 OS spawn 落盘。此后任何进程崩溃都不能再把本代视为尚未启动。
    let attempt = write_spawn_attempt(&directory, generation, &manifest_bytes)?;
    let mut process = match command.spawn() {
        Ok(process) => process,
        Err(error) => {
            // spawn 返回错误可证明监督者没有创建；先持久化与本次 attempt 绑定的拒绝证据。
            // 若应用在此之前崩溃，只留下 attempt，恢复必须保持 Unconfirmed。
            write_spawn_rejected(&directory, &attempt)?;
            let receipt = not_started_receipt(generation, &manifest_bytes);
            write_receipt(&directory, &receipt)?;
            return Err(error);
        }
    };
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
    let manifest = Manifest {
        version: 1,
        launch_allowed: false,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:0".parse().expect("固定本机地址有效"),
        executable: executable.to_owned(),
        arguments: arguments.to_owned(),
        cwd: cwd.to_owned(),
        isolated_home: None,
        environment: None,
        expected_files: Vec::new(),
        atomic_launch_kind: None,
        atomic_cwd: None,
    };
    let (directory, bytes) = create_launch_manifest(state_dir, &manifest)?;
    let receipt = not_started_receipt(generation, &bytes);
    write_receipt(&directory, &receipt)?;
    Ok(receipt)
}

/// 为更新事务永久声明“未启动”，并把 manifest 与退出回执绑定到同一来源摘要。
pub(crate) fn record_not_started_with_binding(
    state_dir: &Path,
    generation: Uuid,
    executable: &Path,
    arguments: &[OsString],
    cwd: &Path,
    binding: &PreparedLaunchBinding,
) -> io::Result<ExitReceipt> {
    let manifest = Manifest {
        version: 1,
        launch_allowed: false,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:0".parse().expect("固定本机地址有效"),
        executable: executable.to_owned(),
        arguments: arguments.to_owned(),
        cwd: cwd.to_owned(),
        isolated_home: None,
        environment: None,
        expected_files: Vec::new(),
        atomic_launch_kind: binding.kind,
        atomic_cwd: None,
    };
    let (directory, bytes) = create_launch_manifest(state_dir, &manifest)?;
    write_launch_binding(&directory, &manifest, &bytes, binding)?;
    let receipt = not_started_receipt(generation, &bytes);
    write_receipt(&directory, &receipt)?;
    Ok(receipt)
}

fn not_started_receipt(generation: Uuid, manifest_bytes: &[u8]) -> ExitReceipt {
    ExitReceipt {
        version: 1,
        generation,
        cleanup_confirmed: true,
        containment: "not_started".to_owned(),
        exit_reason: ExitReason::StopRequested,
        exit_code: None,
        manifest_sha256: sha256(manifest_bytes),
    }
}

pub(super) fn supervisor_executable() -> io::Result<PathBuf> {
    #[cfg(test)]
    {
        let path = std::env::var_os("INFINISHELL_CLI_PROCESS_SUPERVISOR_EXECUTABLE")
            .or_else(|| std::env::var_os("INFINISHELL_CLI_SUPERVISOR_EXECUTABLE"))
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

fn execute_worker_executable() -> io::Result<PathBuf> {
    #[cfg(test)]
    return supervisor_executable();
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

// 在声明代次前检查记录，避免确定未启动的更新因准备失败永久锁住。
fn create_launch_manifest(state_dir: &Path, manifest: &Manifest) -> io::Result<(PathBuf, Vec<u8>)> {
    let bytes = serde_json::to_vec(manifest).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(io::Error::other("托管进程记录过大"));
    }
    let directory = create_generation_directory(state_dir, manifest.generation)?;
    let result = (|| {
        let mut record = NamedTempFile::new_in(&directory)?;
        record.write_all(&bytes)?;
        record.as_file().sync_all()?;
        record
            .persist_noclobber(directory.join("manifest.json"))
            .map_err(|error| error.error)?;
        sync_directory(&directory)?;
        if let Some(parent) = directory.parent() {
            sync_directory(parent)?;
        }
        Ok::<_, io::Error>(())
    })();
    if let Err(error) = result {
        // 这里只删除本次独占创建的空目录；从未派生进程，也不删除已有账本。
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    Ok((directory, bytes))
}

fn write_launch_binding(
    directory: &Path,
    manifest: &Manifest,
    manifest_bytes: &[u8],
    binding: &PreparedLaunchBinding,
) -> io::Result<LaunchBindingRecord> {
    if manifest.atomic_launch_kind != binding.kind {
        return Err(io::Error::other(
            "managed_process.launch_binding_manifest_mismatch",
        ));
    }
    let record = LaunchBindingRecord {
        version: 1,
        generation: manifest.generation,
        binding_digest: binding.digest().to_owned(),
        manifest_sha256: sha256(manifest_bytes),
        kind: binding.kind,
    };
    let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
    write_new_record(&directory.join(LAUNCH_BINDING_RECORD), &bytes)?;
    Ok(record)
}

fn read_bound_launch_binding(
    directory: &Path,
    manifest: &Manifest,
    manifest_bytes: &[u8],
    binding: &PreparedLaunchBinding,
) -> io::Result<LaunchBindingRecord> {
    let bytes = read_record(&directory.join(LAUNCH_BINDING_RECORD))?;
    let record: LaunchBindingRecord = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if record.version != 1
        || record.generation != manifest.generation
        || record.binding_digest != binding.digest()
        || record.manifest_sha256 != sha256(manifest_bytes)
        || record.kind != binding.kind
        || record.kind != manifest.atomic_launch_kind
    {
        return Err(io::Error::other(
            "managed_process.launch_binding_record_mismatch",
        ));
    }
    Ok(record)
}

fn read_worker_launch_binding(
    directory: &Path,
    manifest: &Manifest,
    manifest_bytes: &[u8],
) -> io::Result<Option<PreparedLaunchBinding>> {
    let bytes = match read_record(&directory.join(LAUNCH_BINDING_RECORD)) {
        Ok(bytes) => bytes,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound && manifest.atomic_launch_kind.is_none() =>
        {
            return Ok(None);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::other(
                "managed_process.required_launch_binding_missing",
            ));
        }
        Err(error) => return Err(error),
    };
    let record: LaunchBindingRecord = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if record.version != 1
        || record.generation != manifest.generation
        || record.manifest_sha256 != sha256(manifest_bytes)
        || record.kind != manifest.atomic_launch_kind
    {
        return Err(io::Error::other(
            "managed_process.launch_binding_record_mismatch",
        ));
    }
    let binding = PreparedLaunchBinding::with_kind(record.binding_digest, record.kind)?;
    binding.validate_update_execution()?;
    Ok(Some(binding))
}

fn write_exit_binding(
    directory: &Path,
    receipt: &ExitReceipt,
    receipt_bytes: &[u8],
) -> io::Result<()> {
    let binding_bytes = match read_record(&directory.join(LAUNCH_BINDING_RECORD)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let launch: LaunchBindingRecord =
        serde_json::from_slice(&binding_bytes).map_err(io::Error::other)?;
    if launch.version != 1
        || launch.generation != receipt.generation
        || launch.manifest_sha256 != receipt.manifest_sha256
    {
        return Err(io::Error::other(
            "managed_process.exit_binding_launch_mismatch",
        ));
    }
    let record = ExitBindingRecord {
        version: 1,
        generation: receipt.generation,
        binding_digest: launch.binding_digest,
        manifest_sha256: receipt.manifest_sha256.clone(),
        receipt_sha256: sha256(receipt_bytes),
    };
    let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
    write_new_record(&directory.join(EXIT_BINDING_RECORD), &bytes)
}

fn write_spawn_attempt(
    directory: &Path,
    generation: Uuid,
    manifest_bytes: &[u8],
) -> io::Result<SpawnAttempt> {
    let attempt = SpawnAttempt {
        version: 1,
        generation,
        attempt: Uuid::new_v4(),
        manifest_sha256: sha256(manifest_bytes),
    };
    let bytes = serde_json::to_vec(&attempt).map_err(io::Error::other)?;
    write_new_record(&directory.join(SPAWN_ATTEMPT_RECORD), &bytes)?;
    Ok(attempt)
}

fn write_spawn_rejected(directory: &Path, attempt: &SpawnAttempt) -> io::Result<SpawnRejected> {
    let (manifest, manifest_bytes) = read_manifest(&directory.join("manifest.json"))?;
    if read_bound_spawn_attempt(directory, &manifest, &manifest_bytes)? != *attempt {
        return Err(io::Error::other("拒绝记录不能绑定其他 spawn attempt"));
    }
    // rejection 一旦可见，恢复即可安全宣告未启动；因此隔离认证副本必须先清除。
    cleanup_isolated_auth(&manifest)?;
    let rejected = SpawnRejected {
        version: 1,
        generation: attempt.generation,
        attempt: attempt.attempt,
        manifest_sha256: attempt.manifest_sha256.clone(),
    };
    let bytes = serde_json::to_vec(&rejected).map_err(io::Error::other)?;
    write_new_record(&directory.join(SPAWN_REJECTED_RECORD), &bytes)?;
    Ok(rejected)
}

fn read_bound_spawn_attempt(
    directory: &Path,
    manifest: &Manifest,
    manifest_bytes: &[u8],
) -> io::Result<SpawnAttempt> {
    let bytes = read_record(&directory.join(SPAWN_ATTEMPT_RECORD))?;
    let attempt: SpawnAttempt = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if !manifest.launch_allowed
        || attempt.version != 1
        || attempt.generation != manifest.generation
        || attempt.manifest_sha256 != sha256(manifest_bytes)
    {
        return Err(io::Error::other(
            "托管进程 spawn attempt 与本代 manifest 不匹配",
        ));
    }
    Ok(attempt)
}

fn confirmed_spawn_rejection(
    directory: &Path,
    manifest: &Manifest,
    manifest_bytes: &[u8],
) -> io::Result<Option<SpawnRejected>> {
    let bytes = match read_record(&directory.join(SPAWN_REJECTED_RECORD)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let rejected: SpawnRejected = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let attempt = read_bound_spawn_attempt(directory, manifest, manifest_bytes)?;
    if !manifest.launch_allowed
        || rejected.version != 1
        || rejected.generation != manifest.generation
        || rejected.attempt != attempt.attempt
        || rejected.manifest_sha256 != attempt.manifest_sha256
    {
        return Err(io::Error::other(
            "托管进程 spawn 拒绝证据与本次 attempt 不匹配",
        ));
    }
    Ok(Some(rejected))
}

fn sync_directory(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        return File::open(directory)?.sync_all();
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(())
    }
}

fn write_new_record(path: &Path, contents: &[u8]) -> io::Result<()> {
    if contents.len() as u64 > MAX_RECORD_BYTES {
        return Err(io::Error::other("托管进程记录过大"));
    }
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("托管进程记录缺少父目录"))?;
    let mut temporary = NamedTempFile::new_in(directory)?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    sync_directory(directory)?;
    Ok(())
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
    validate_expected_files_contract(&manifest.executable, &manifest.expected_files)?;
    if let Some(environment) = &manifest.environment {
        if manifest.isolated_home.is_some() {
            return Err(io::Error::other("隔离任务不接受额外环境覆盖"));
        }
        let state_dir = directory
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| io::Error::other("更新环境缺少状态域"))?;
        environment.validate_for_generation(state_dir, manifest.generation, &manifest.arguments)?;
    }
    if let Some(home) = &manifest.isolated_home {
        let state = directory
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| io::Error::other("隔离启动缺少状态域"))?
            .canonicalize()?;
        if home.parent() != Some(state.join("grok-managed").as_path())
            || home
                .file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| Uuid::parse_str(name).is_err())
            || home.canonicalize()? != *home
        {
            return Err(io::Error::other("隔离启动目录不属于托管状态域"));
        }
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
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // spawn 明确拒绝后，rejection 本身就是“未启动”的持久证明；即使应用在
            // 写 exit.json 前崩溃，也可安全恢复。只有 attempt、manifest 和 rejection
            // 三者完全绑定才允许合成回执。
            match fs::symlink_metadata(directory.join(SPAWN_REJECTED_RECORD)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
                Ok(_) => {}
            }
            let (manifest, manifest_bytes) = read_manifest(&directory.join("manifest.json"))?;
            if manifest.generation != generation
                || confirmed_spawn_rejection(&directory, &manifest, &manifest_bytes)?.is_none()
            {
                return Err(io::Error::other("spawn 拒绝证据与运行代次不匹配"));
            }
            return Ok(Some(not_started_receipt(generation, &manifest_bytes)));
        }
        Err(error) => return Err(error),
    };
    let receipt: ExitReceipt = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let (manifest, manifest_bytes) = read_manifest(&directory.join("manifest.json"))?;
    let spawn_rejected = if manifest.launch_allowed {
        confirmed_spawn_rejection(&directory, &manifest, &manifest_bytes)?.is_some()
    } else {
        false
    };
    if receipt.version != 1
        || receipt.generation != generation
        || manifest.generation != generation
        || !receipt.cleanup_confirmed
        // 旧版 macOS 回执只证明原进程组；CLI 异常退出时另组工具可能仍在运行。
        || (receipt.containment == "unix_process_group" && receipt.exit_code != Some(0))
        || (spawn_rejected && receipt.containment != "not_started")
        || receipt.manifest_sha256 != sha256(&manifest_bytes)
        || match (manifest.launch_allowed, receipt.containment.as_str()) {
            (true, "linux_subtree" | "windows_job" | "unix_process_group")
            | (false, "not_started") => false,
            (true, "not_started") if spawn_rejected => false,
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

/// 更新恢复只能接受与 journal 中摘要一致的 manifest 和退出回执。
pub(crate) fn confirmed_exit_with_binding(
    state_dir: &Path,
    generation: Uuid,
    binding: &PreparedLaunchBinding,
) -> io::Result<Option<ExitReceipt>> {
    let Some(receipt) = confirmed_exit(state_dir, generation)? else {
        return Ok(None);
    };
    let directory = generation_directory(state_dir, generation);
    let (manifest, manifest_bytes) = read_manifest(&directory.join("manifest.json"))?;
    let launch = read_bound_launch_binding(&directory, &manifest, &manifest_bytes, binding)?;
    let receipt_bytes = read_record(&directory.join("exit.json"))?;
    let exit_bytes = read_record(&directory.join(EXIT_BINDING_RECORD))?;
    let exit: ExitBindingRecord = serde_json::from_slice(&exit_bytes).map_err(io::Error::other)?;
    if exit.version != 1
        || exit.generation != generation
        || exit.binding_digest != binding.digest()
        || exit.binding_digest != launch.binding_digest
        || exit.manifest_sha256 != launch.manifest_sha256
        || exit.receipt_sha256 != sha256(&receipt_bytes)
    {
        return Err(io::Error::other(
            "managed_process.exit_binding_record_mismatch",
        ));
    }
    Ok(Some(receipt))
}

/// 仅在 adapter 任务已经返回后调用；目录缺失此时可证明真实 CLI 从未进入监督启动。
pub(super) fn process_completion(
    state_dir: &Path,
    generation: Uuid,
) -> io::Result<ProcessCompletion> {
    let directory = generation_directory(state_dir, generation);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(io::Error::other("托管进程代次不是受控目录")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ProcessCompletion::NotStarted);
        }
        Err(error) => return Err(error),
    }
    let Some(receipt) = confirmed_exit(state_dir, generation)? else {
        return Ok(ProcessCompletion::Unconfirmed);
    };
    if receipt.containment == "not_started" {
        Ok(ProcessCompletion::NotStarted)
    } else {
        Ok(ProcessCompletion::Exited(receipt))
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(file: &mut File) -> io::Result<String> {
    file.rewind()?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_expected_files_contract(
    executable: &Path,
    expected_files: &[ExpectedFileIdentity],
) -> io::Result<()> {
    if expected_files.is_empty() {
        return Ok(());
    }
    let mut paths = std::collections::HashSet::new();
    let mut canonical_paths = std::collections::HashSet::new();
    for expected in expected_files {
        if !expected.path.is_absolute()
            || !expected.canonical_path.is_absolute()
            || expected.sha256.len() != 64
            || !expected.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !paths.insert(expected.path.clone())
            || !canonical_paths.insert(expected.canonical_path.clone())
        {
            return Err(io::Error::other(
                "managed_process.expected_file_identity_invalid",
            ));
        }
    }
    if !expected_files
        .iter()
        .any(|expected| expected.path == executable)
    {
        return Err(io::Error::other(
            "managed_process.expected_program_identity_missing",
        ));
    }
    Ok(())
}

fn verify_expected_files(expected_files: &[ExpectedFileIdentity]) -> io::Result<()> {
    for expected in expected_files {
        let actual = ExpectedFileIdentity::capture(&expected.path)?;
        if actual.path != expected.path
            || actual.canonical_path != expected.canonical_path
            || actual.sha256 != expected.sha256
            || actual.size != expected.size
            || expected.file_id.is_some() && actual.file_id != expected.file_id
        {
            return Err(io::Error::other(
                "managed_process.expected_file_identity_changed",
            ));
        }
    }
    Ok(())
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
    if !execute && super::runtime_host::is_runtime_host_manifest(path)? {
        return super::runtime_host::run_worker(path);
    }
    let (manifest, bytes) = read_manifest(path)?;
    if !manifest.launch_allowed {
        return Err(io::Error::other("此运行代次已永久声明为未启动"));
    }
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("缺少托管记录目录"))?;
    read_bound_spawn_attempt(directory, &manifest, &bytes)?;
    if confirmed_spawn_rejection(directory, &manifest, &bytes)?.is_some() {
        return Err(io::Error::other("OS 已拒绝本次监督进程启动"));
    }
    if execute {
        #[cfg(target_os = "macos")]
        if std::env::var_os(EXEC_CONTROL_ENV)
            .is_some_and(|value| value.as_encoded_bytes().starts_with(b"unix:"))
        {
            return macos::run_execute(path, &manifest);
        }
        return run_exec_worker(path, &manifest, &bytes);
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
        command::blocking::Command::new_with_managed_process_group(execute_worker_executable()?);
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
        let result = copy_output_with_flush(&mut child_output, &mut io::stdout().lock());
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
    // 给 stdin EOF 一个有界的原生收尾窗口；不以该窗口或空闲状态代替最终组/Job 验证。
    let graceful_deadline = Instant::now() + GRACEFUL_EXIT_TIMEOUT;
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

#[cfg(target_os = "macos")]
#[path = "managed_process_atomic_macos.rs"]
mod atomic_macos;

#[cfg(target_os = "linux")]
#[path = "managed_process_atomic_linux.rs"]
mod atomic_linux;

#[cfg(windows)]
#[path = "managed_process_atomic_windows.rs"]
mod atomic_windows;

fn write_receipt(directory: &Path, receipt: &ExitReceipt) -> io::Result<()> {
    // 只有原生进程树收尾后才删除本代认证副本；失败不能生成成功清理回执。
    if receipt.cleanup_confirmed {
        let (manifest, _) = read_manifest(&directory.join("manifest.json"))?;
        cleanup_isolated_auth(&manifest)?;
    }
    let bytes = serde_json::to_vec(receipt).map_err(io::Error::other)?;
    let mut temporary = NamedTempFile::new_in(directory)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(directory.join("exit.json"))
        .map_err(|error| error.error)?;
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    write_exit_binding(directory, receipt, &bytes)?;
    Ok(())
}

fn cleanup_isolated_auth(manifest: &Manifest) -> io::Result<()> {
    let Some(home) = &manifest.isolated_home else {
        return Ok(());
    };
    let auth = home.join("grok/auth.json");
    match fs::remove_file(&auth) {
        Ok(()) => {
            if let Some(directory) = auth.parent() {
                sync_directory(directory)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(super) fn isolated_environment(home: &Path) -> Vec<(OsString, OsString)> {
    // 只继承系统运行与网络路由变量；不继承 CLI 配置、动态加载器或自动批准开关。
    let mut values: Vec<_> = [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "LANG",
        "LC_ALL",
        "TZ",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ]
    .into_iter()
    .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
    .collect();
    values.extend(
        [
            ("HOME", "home"),
            ("USERPROFILE", "home"),
            ("GROK_HOME", "grok"),
            ("APPDATA", "home/AppData/Roaming"),
            ("LOCALAPPDATA", "home/AppData/Local"),
            ("XDG_CONFIG_HOME", "home/.config"),
            ("XDG_DATA_HOME", "home/.local/share"),
            ("XDG_CACHE_HOME", "home/.cache"),
            ("CLAUDE_CONFIG_DIR", "home/.claude"),
            ("CODEX_HOME", "home/.codex"),
            ("TMPDIR", "tmp"),
            ("TMP", "tmp"),
            ("TEMP", "tmp"),
        ]
        .into_iter()
        .map(|(name, path)| (OsString::from(name), home.join(path).into_os_string())),
    );
    values
}

fn run_exec_worker(path: &Path, manifest: &Manifest, manifest_bytes: &[u8]) -> io::Result<()> {
    let address = std::env::var(EXEC_CONTROL_ENV)
        .map_err(io::Error::other)?
        .parse::<SocketAddr>()
        .map_err(io::Error::other)?;
    drop(connect_authorized(address, manifest)?);
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("缺少托管记录目录"))?;
    if let Some(binding) = read_worker_launch_binding(directory, manifest, manifest_bytes)? {
        match binding.kind {
            Some(AtomicLaunchKind::NativeFile) => {
                #[cfg(target_os = "linux")]
                {
                    if manifest.expected_files.len() != 1
                        || manifest.expected_files[0].path != manifest.executable
                    {
                        return Err(io::Error::other(
                            "managed_process.linux_atomic_dependency_closure_invalid",
                        ));
                    }
                    let executable = atomic_linux::prepare(&manifest.expected_files[0])?;
                    if executable.sha256() != manifest.expected_files[0].sha256
                        || executable.size() != manifest.expected_files[0].size
                    {
                        return Err(io::Error::other(
                            "managed_process.linux_atomic_snapshot_changed",
                        ));
                    }
                    enter_atomic_cwd(manifest)?;
                    let environment = resolved_atomic_environment(manifest)?;
                    return Err(atomic_linux::execute(
                        &executable,
                        &manifest.executable,
                        &manifest.arguments,
                        &environment,
                    ));
                }
                #[cfg(target_os = "macos")]
                {
                    if manifest.expected_files.len() != 1
                        || manifest.expected_files[0].path != manifest.executable
                    {
                        return Err(io::Error::other(
                            "managed_process.atomic_macos_dependency_closure_invalid",
                        ));
                    }
                    let state_dir = directory.parent().and_then(Path::parent).ok_or_else(|| {
                        io::Error::other("managed_process.atomic_macos_state_dir_invalid")
                    })?;
                    let executable = atomic_macos::prepare_native_executable(
                        state_dir,
                        manifest.generation,
                        binding.digest(),
                        &manifest.expected_files[0],
                    )?;
                    executable.verify_for_execution()?;
                    return run_path_exec_worker(manifest, executable.execution_path(), false);
                }
                #[cfg(windows)]
                {
                    if manifest.expected_files.len() != 1
                        || manifest.expected_files[0].path != manifest.executable
                    {
                        return Err(io::Error::other(
                            "managed_process.atomic_windows_dependency_closure_invalid",
                        ));
                    }
                    let mut executable = atomic_windows::prepare(&manifest.expected_files[0])?;
                    let cwd_identity = manifest.atomic_cwd.as_ref().ok_or_else(|| {
                        io::Error::other("managed_process.atomic_cwd_binding_missing")
                    })?;
                    if cwd_identity.path != manifest.cwd {
                        return Err(io::Error::other(
                            "managed_process.atomic_cwd_binding_mismatch",
                        ));
                    }
                    let cwd = atomic_windows::prepare_directory(cwd_identity)?;
                    let mut command = command::blocking::Command::new(executable.execution_path());
                    command
                        .args(&manifest.arguments)
                        .current_dir(cwd.execution_path())
                        .env_clear()
                        .envs(resolved_atomic_environment(manifest)?)
                        .env_remove(EXEC_CONTROL_ENV)
                        .stdin(Stdio::inherit())
                        .stdout(Stdio::inherit())
                        .stderr(Stdio::inherit())
                        .inherit_managed_job();
                    command.creation_flags(windows::Win32::System::Threading::DEBUG_PROCESS.0);
                    executable.verify_command_for_suspended_spawn(&command)?;
                    cwd.verify_for_spawn()?;
                    let suspended = command.spawn_suspended()?;
                    let root_process_id = suspended.id();
                    let mut child = match suspended.resume_with_child_on_error() {
                        Ok(child) => child,
                        Err((failure, mut child)) => {
                            let _ = child.kill();
                            // 调试事件尚未继续，不能在此等待；外层严格 Job 确认整树退出。
                            return Err(failure);
                        }
                    };
                    let mut image_debug =
                        match executable.begin_image_debug_session(root_process_id) {
                            Ok(session) => session,
                            Err(failure) => {
                                let _ = child.kill();
                                // 外层严格 Job 负责整树终止及零残留确认，然后才写退出回执。
                                return Err(failure);
                            }
                        };
                    drop(cwd);
                    let status = match image_debug.wait_for_exit(&mut child) {
                        Ok(status) => status,
                        Err(failure) => {
                            let _ = child.kill();
                            return Err(failure);
                        }
                    };
                    if status.success() {
                        return Ok(());
                    }
                    let code = status.code().ok_or_else(|| {
                        io::Error::other("managed_process.atomic_windows_exit_code_missing")
                    })?;
                    std::process::exit(code);
                }
                #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "managed_process.atomic_update_launch_not_implemented",
                ));
            }
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "managed_process.atomic_update_launch_not_implemented",
                ));
            }
        }
    }
    run_path_exec_worker(manifest, &manifest.executable, true)
}

fn run_path_exec_worker(
    manifest: &Manifest,
    executable: &Path,
    verify_original_files: bool,
) -> io::Result<()> {
    let mut command = command::blocking::Command::new(executable);
    command
        .args(&manifest.arguments)
        .env_remove(EXEC_CONTROL_ENV)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if verify_original_files {
        command.current_dir(&manifest.cwd);
        if let Some(home) = &manifest.isolated_home {
            let environment = isolated_environment(home);
            command.env_clear().envs(environment);
        }
        if let Some(environment) = &manifest.environment {
            environment.validate()?;
            for name in &environment.remove {
                command.env_remove(name);
            }
            command.envs(environment.values.iter().map(|(name, value)| (name, value)));
        }
    } else {
        #[cfg(unix)]
        enter_atomic_cwd(manifest)?;
        command
            .env_clear()
            .envs(resolved_atomic_environment(manifest)?);
    }
    // 此核对必须尽量贴近最终 exec/status，不能只信 GUI 写 manifest 前的哈希。
    if verify_original_files {
        verify_expected_files(&manifest.expected_files)?;
    }
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

#[cfg(unix)]
fn enter_atomic_cwd(manifest: &Manifest) -> io::Result<()> {
    let cwd = manifest
        .atomic_cwd
        .as_ref()
        .ok_or_else(|| io::Error::other("managed_process.atomic_cwd_binding_missing"))?;
    if cwd.path != manifest.cwd {
        return Err(io::Error::other(
            "managed_process.atomic_cwd_binding_mismatch",
        ));
    }
    cwd.enter()
}

#[cfg(any(unix, windows))]
fn resolved_atomic_environment(manifest: &Manifest) -> io::Result<Vec<(OsString, OsString)>> {
    let mut values = if let Some(home) = &manifest.isolated_home {
        isolated_environment(home)
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>()
    } else {
        std::env::vars_os().collect::<std::collections::BTreeMap<_, _>>()
    };
    values.remove(&OsString::from(EXEC_CONTROL_ENV));
    if let Some(environment) = &manifest.environment {
        environment.validate()?;
        for name in &environment.remove {
            values.remove(name);
        }
        for (name, value) in &environment.values {
            values.insert(name.clone(), value.clone());
        }
    }
    values.retain(|name, _| !unsafe_dynamic_loader_environment(name));
    Ok(values.into_iter().collect())
}

#[cfg(unix)]
fn unsafe_dynamic_loader_environment(name: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt as _;

    let name = name.as_bytes();
    name.starts_with(b"DYLD_")
        || matches!(
            name,
            b"LD_AUDIT" | b"LD_DEBUG" | b"LD_LIBRARY_PATH" | b"LD_PRELOAD" | b"LD_PROFILE"
        )
}

#[cfg(windows)]
fn unsafe_dynamic_loader_environment(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.eq_ignore_ascii_case("__COMPAT_LAYER"))
}

#[cfg(test)]
#[path = "managed_process_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "managed_process_live_tests.rs"]
mod live_tests;
