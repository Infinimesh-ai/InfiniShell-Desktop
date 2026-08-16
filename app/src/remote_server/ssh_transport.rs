//! SSH-specific implementation of [`RemoteTransport`].
//!
//! [`SshTransport`] uses either an existing OpenSSH ControlMaster or the
//! single-session Rust SSH broker to check/install the remote server binary
//! and launch the `remote-server-proxy` process whose stdin/stdout become the
//! protocol channel.
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use base64::Engine as _;
use remote_server::auth::RemoteServerAuthContext;
use remote_server::client::RemoteServerClient;
use remote_server::manager::RemoteServerExitStatus;
use remote_server::setup::{
    PreinstallCheckResult, RemoteOs, RemotePlatform, RemoteSetupCommand, RemoteShellDialect,
    parse_platform_output, parse_uname_output, remote_server_daemon_dir,
};
use remote_server::ssh::ssh_args;
use remote_server::transport::{
    Connection, ControlPath, Error, InstallOutcome, InstallSource, LocalProcessConnection,
    RemoteTransport,
};
use warpui::r#async::{FutureExt as _, executor};

#[path = "ssh_transport/installation.rs"]
pub(crate) mod installation;

/// SSH transport whose backend multiplexes setup and proxy commands without
/// re-authenticating: Unix platforms use a ControlMaster socket, while the
/// Windows wrapper uses channels on its existing Rust SSH session.
#[derive(Clone)]
pub struct SshTransport {
    backend: SshTransportBackend,
    auth_context: Arc<RemoteServerAuthContext>,
    remote_os: RemoteOs,
    detected_platform: Arc<Mutex<Option<RemotePlatform>>>,
}

#[derive(Clone)]
enum SshTransportBackend {
    ControlMaster {
        socket_path: PathBuf,
        warp_owns_control_master: bool,
    },
    RustBroker {
        endpoint: String,
        capability: String,
    },
}

impl fmt::Debug for SshTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let backend = match &self.backend {
            SshTransportBackend::ControlMaster { .. } => "control_master",
            SshTransportBackend::RustBroker { .. } => "rust_broker",
        };
        f.debug_struct("SshTransport")
            .field("backend", &backend)
            .field("remote_os", &self.remote_os)
            .finish_non_exhaustive()
    }
}

impl SshTransport {
    pub fn new(
        socket_path: PathBuf,
        auth_context: Arc<RemoteServerAuthContext>,
        warp_owns_control_master: bool,
    ) -> Self {
        Self {
            backend: SshTransportBackend::ControlMaster {
                socket_path,
                warp_owns_control_master,
            },
            auth_context,
            remote_os: RemoteOs::Linux,
            detected_platform: Arc::new(Mutex::new(None)),
        }
    }

    pub fn new_control_master(
        socket_path: PathBuf,
        auth_context: Arc<RemoteServerAuthContext>,
        warp_owns_control_master: bool,
        remote_os: RemoteOs,
    ) -> Self {
        Self {
            backend: SshTransportBackend::ControlMaster {
                socket_path,
                warp_owns_control_master,
            },
            auth_context,
            remote_os,
            detected_platform: Arc::new(Mutex::new(None)),
        }
    }

    pub fn new_rust_broker(
        endpoint: String,
        capability: String,
        auth_context: Arc<RemoteServerAuthContext>,
        remote_os: RemoteOs,
    ) -> Self {
        Self {
            backend: SshTransportBackend::RustBroker {
                endpoint,
                capability,
            },
            auth_context,
            remote_os,
            detected_platform: Arc::new(Mutex::new(None)),
        }
    }

    pub fn remote_daemon_socket_path(&self) -> String {
        format!(
            "{}/{}",
            remote_server_daemon_dir(&self.auth_context.remote_server_identity_key()),
            remote_server::setup::daemon_socket_name(),
        )
    }

    pub fn remote_daemon_pid_path(&self) -> String {
        format!(
            "{}/{}",
            remote_server_daemon_dir(&self.auth_context.remote_server_identity_key()),
            remote_server::setup::daemon_pid_name(),
        )
    }

    fn remote_proxy_command(&self) -> RemoteSetupCommand {
        let identity_key = self.auth_context.remote_server_identity_key();
        remote_server::setup::remote_server_proxy_command(&self.remote_os, &identity_key)
    }

    fn control_master(&self) -> Option<(&Path, bool)> {
        match &self.backend {
            SshTransportBackend::ControlMaster {
                socket_path,
                warp_owns_control_master,
            } => Some((socket_path, *warp_owns_control_master)),
            SshTransportBackend::RustBroker { .. } => None,
        }
    }

    async fn run_setup_command(
        &self,
        command: RemoteSetupCommand,
        timeout: std::time::Duration,
    ) -> Result<std::process::Output, Error> {
        let command = setup_command_line(&command);
        let child = match &self.backend {
            SshTransportBackend::ControlMaster { socket_path, .. } => {
                let mut args = ssh_args(socket_path);
                args.push(command);
                command::r#async::Command::new("ssh")
                    .args(&args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|error| Error::Other(error.into()))?
            }
            SshTransportBackend::RustBroker {
                endpoint,
                capability,
            } => {
                let executable = crate::remote_server::rust_ssh::worker_executable()
                    .map_err(|error| Error::Other(error.into()))?;
                command::r#async::Command::new(executable)
                    .arg("rust-ssh-broker-command")
                    .arg("--endpoint")
                    .arg(endpoint)
                    .arg("--command")
                    .arg(command)
                    .env(
                        crate::remote_server::rust_ssh::BROKER_CAPABILITY_ENV,
                        capability,
                    )
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|error| Error::Other(error.into()))?
            }
        };
        child
            .output()
            .with_timeout(timeout)
            .await
            .map_err(|_| Error::TimedOut)?
            .map_err(|error| Error::Other(error.into()))
    }

    async fn upload_file(
        &self,
        local_path: &Path,
        remote_path: &str,
        timeout: std::time::Duration,
    ) -> Result<(), Error> {
        let command = setup_command_line(&upload_command(&self.remote_os, remote_path));
        let mut child = match &self.backend {
            SshTransportBackend::ControlMaster { socket_path, .. } => {
                let mut args = ssh_args(socket_path);
                args.push(command);
                command::r#async::Command::new("ssh")
                    .args(&args)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|error| Error::Other(error.into()))?
            }
            SshTransportBackend::RustBroker {
                endpoint,
                capability,
            } => {
                let executable = crate::remote_server::rust_ssh::worker_executable()
                    .map_err(|error| Error::Other(error.into()))?;
                command::r#async::Command::new(executable)
                    .arg("rust-ssh-broker-command")
                    .arg("--endpoint")
                    .arg(endpoint)
                    .arg("--command")
                    .arg(command)
                    .env(
                        crate::remote_server::rust_ssh::BROKER_CAPABILITY_ENV,
                        capability,
                    )
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|error| Error::Other(error.into()))?
            }
        };
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Other(anyhow!("SSH upload process has no stdin")))?;
        let mut file = async_fs::File::open(local_path)
            .await
            .map_err(|error| Error::Other(error.into()))?;
        let output = async move {
            futures_lite::io::copy(&mut file, &mut stdin).await?;
            drop(stdin);
            child.output().await
        }
        .with_timeout(timeout)
        .await
        .map_err(|_| Error::TimedOut)?
        .map_err(|error| Error::Other(error.into()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::ScriptFailed {
                exit_code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }

    async fn install_for_platform(
        &self,
        platform: &RemotePlatform,
        staging_archive: Option<&str>,
    ) -> Result<(), Error> {
        let output = self
            .run_setup_command(
                remote_server::setup::install_command(platform, staging_archive),
                remote_server::setup::INSTALL_TIMEOUT,
            )
            .await?;
        if !output.status.success() {
            return Err(Error::ScriptFailed {
                exit_code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
        let verification = self
            .run_setup_command(
                remote_server::setup::binary_check_command_for(&self.remote_os),
                remote_server::setup::CHECK_TIMEOUT,
            )
            .await?;
        if verification.status.success() {
            Ok(())
        } else {
            Err(Error::ScriptFailed {
                exit_code: verification.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&verification.stderr).to_string(),
            })
        }
    }

    async fn install_client_archive(
        &self,
        platform: &RemotePlatform,
        local_archive: &Path,
    ) -> Result<(), Error> {
        let archive_extension = match platform.os {
            RemoteOs::Linux | RemoteOs::MacOs => "tar.gz",
            RemoteOs::Windows => "zip",
        };
        let remote_path = format!(
            "{}/infinishell-upload-{}.{}",
            remote_server::setup::remote_server_dir(),
            uuid::Uuid::new_v4(),
            archive_extension,
        );
        self.upload_file(
            local_archive,
            &remote_path,
            remote_server::setup::SCP_INSTALL_TIMEOUT,
        )
        .await?;
        self.install_for_platform(platform, Some(&remote_path))
            .await
    }
}

fn setup_command_line(command: &RemoteSetupCommand) -> String {
    match command.dialect {
        RemoteShellDialect::Posix => command.script.clone(),
        RemoteShellDialect::PowerShell => {
            let bytes = command
                .script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}")
        }
    }
}

fn upload_command(remote_os: &RemoteOs, remote_path: &str) -> RemoteSetupCommand {
    let relative_path = remote_path
        .trim_start_matches("~/")
        .trim_start_matches("~\\");
    match remote_os {
        RemoteOs::Linux | RemoteOs::MacOs => {
            let relative_path = relative_path.replace('\'', "'\\''");
            RemoteSetupCommand {
                dialect: RemoteShellDialect::Posix,
                script: format!(
                    "umask 077; path=\"$HOME/{}\"; mkdir -p \"$(dirname \"$path\")\" && cat > \"$path\"",
                    relative_path.replace('"', "\\\"")
                ),
            }
        }
        RemoteOs::Windows => {
            let relative_path = relative_path.replace('/', "\\").replace('\'', "''");
            RemoteSetupCommand {
                dialect: RemoteShellDialect::PowerShell,
                script: format!(
                    "$path = Join-Path $HOME '{relative_path}'; New-Item -ItemType Directory -Force -Path (Split-Path -Parent $path) | Out-Null; $source = [Console]::OpenStandardInput(); $destination = [IO.File]::Open($path, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::None); try {{ $source.CopyTo($destination) }} finally {{ $destination.Dispose() }}"
                ),
            }
        }
    }
}

/// Runs `uname -sm` on the remote host via the ControlMaster socket and
/// parses the output into a [`RemotePlatform`].
async fn detect_remote_platform(socket_path: &Path) -> Result<RemotePlatform, Error> {
    let output = remote_server::ssh::run_ssh_command(
        socket_path,
        "uname -sm",
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_uname_output(&stdout)
    } else {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::Other(anyhow::anyhow!(
            "uname -sm exited with code {code}: {stderr}"
        )))
    }
}

/// Zap fork:开发模式安装路径上传完二进制后的可运行性校验。
///
/// 常规安装路径的校验在 `ssh_transport/installation.rs` 里,dev 路径绕过
/// 了那套流程,所以这里单独保留一份。
async fn verify_installed_binary(transport: &SshTransport) -> Result<()> {
    let output = transport
        .run_setup_command(
            remote_server::setup::binary_check_command_for(&transport.remote_os),
            remote_server::setup::CHECK_TIMEOUT,
        )
        .await
        .map_err(anyhow::Error::from)?;

    if output.status.success() {
        return Ok(());
    }

    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "installed binary check failed with code {code}: {stderr}"
    ))
}

// ===========================================================================
// Zap fork:开发模式 remote-server 安装路径
//
// 上游 / release 构建会让远端安装脚本从 GitHub releases 下载预编译的
// remote-server 二进制。但在本地源码构建(`cargo run`)时,这会下载到
// 「最新已发布」的陈旧二进制,而不是开发者刚改过的代码,导致根本无法
// 调试 remote-server 的改动。
//
// 因此在 DEBUG 且无 release tag 的源码构建下(见
// `remote_server::setup::is_dev_source_build()`),`install_binary()` 改为:
//   1. 本地把 `warp` 二进制交叉编译到 x86_64 musl(profile/features 与
//      `script/deploy_remote_server` 完全一致);
//   2. 通过已有的 SSH ControlMaster socket,用 `scp_upload` 把产物上传到
//      `remote_server::setup::remote_server_binary()` 解析出的远端路径;
//   3. 完全跳过 GitHub 下载安装脚本。
//
// 如果交叉编译前置条件缺失(没装 musl target、没有 musl 链接器),不会
// 硬失败,而是打印清晰告警并回退到原有下载安装流程,保证 dev 仍可用。
// ===========================================================================

/// 开发模式交叉编译可能用到的 musl 链接器候选(按优先级)。
/// macOS 上一般是 `x86_64-linux-musl-gcc`(filosottile/musl-cross),
/// Linux 上常见为 `musl-gcc`。
const DEV_MUSL_LINKER_CANDIDATES: &[&str] = &["x86_64-linux-musl-gcc", "musl-gcc"];

/// 返回当前 workspace 根目录。
///
/// `ssh_transport.rs` 属于 `app` crate,`CARGO_MANIFEST_DIR` 指向
/// `<workspace>/app`,其父目录即 workspace 根。
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        // 理论上 `app` 一定有父目录;万一没有就退回 manifest 目录本身。
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

/// 返回追加了 `~/.cargo/bin`(及 `$CARGO_HOME/bin`)的 PATH。
///
/// warp 进程常由桌面环境或系统 `cargo` 拉起,其 PATH 可能只含 `/usr/bin`
/// 而不含 `~/.cargo/bin`。这会导致:
///   - `cargo zigbuild` 找不到 `cargo-zigbuild` 子命令 → 回退到 musl-gcc;
///   - cargo-zigbuild 自身找不到 `cargo` / `rustc`。
/// 交叉编译相关的子进程统一用这里返回的 PATH,保证两者都能解析到。
/// 若无需调整(无 HOME / 无法拼接)返回 `None`,调用方沿用继承的 PATH。
fn dev_build_path_env() -> Option<std::ffi::OsString> {
    let mut extra: Vec<PathBuf> = Vec::new();
    if let Some(cargo_home) = std::env::var_os("CARGO_HOME") {
        extra.push(PathBuf::from(cargo_home).join("bin"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        extra.push(PathBuf::from(home).join(".cargo").join("bin"));
    }
    if extra.is_empty() {
        return None;
    }
    let current = std::env::var_os("PATH").unwrap_or_default();
    extra.extend(std::env::split_paths(&current));
    std::env::join_paths(extra).ok()
}

/// 在 `PATH` 中查找首个可用的 musl 链接器,找不到返回 `None`。
fn find_musl_linker() -> Option<&'static str> {
    DEV_MUSL_LINKER_CANDIDATES.iter().copied().find(|linker| {
        command::blocking::Command::new(linker)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

/// dev 交叉编译使用的构建后端。
enum DevBuildBackend {
    /// `cargo zigbuild`:zig 充当完整的 C/C++ musl 交叉工具链,无需单独安装
    /// `*-musl-gcc` / `*-musl-g++`,能正确编译 `freetype-sys` 等带 C/C++ 源码
    /// 的依赖。这是首选后端。
    Zigbuild,
    /// 原生 `cargo build` + musl 链接器。仅当系统装有完整的 musl C/C++ 交叉
    /// 工具链时才可靠 —— 只有 `*-musl-gcc`、缺 `*-musl-g++` 时,`freetype-sys`
    /// 之类的 C++ 依赖会编译失败。
    MuslGcc(&'static str),
}

/// 检测 `cargo-zigbuild` 是否可用。
///
/// 直接探测 `cargo-zigbuild --version`(二进制本身),而不是
/// `cargo zigbuild --version` —— 后者会被 `zigbuild` 子命令解析为未知参数
/// 而失败。探测用的 PATH 与实际构建一致(注入 `~/.cargo/bin`)。
fn cargo_zigbuild_available() -> bool {
    let mut cmd = command::blocking::Command::new("cargo-zigbuild");
    cmd.arg("--version");
    if let Some(path) = dev_build_path_env() {
        cmd.env("PATH", path);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// 选择 dev 交叉编译后端:优先 `cargo zigbuild`,回退到原生 `cargo build`
/// + musl 链接器。两者都不可用时返回 `None`,由调用方回退到下载安装。
fn select_dev_build_backend() -> Option<DevBuildBackend> {
    if cargo_zigbuild_available() {
        return Some(DevBuildBackend::Zigbuild);
    }
    find_musl_linker().map(DevBuildBackend::MuslGcc)
}

/// 检查 `x86_64-unknown-linux-musl` target 是否已通过 rustup 安装。
async fn musl_target_installed() -> bool {
    let output = command::r#async::Command::new("rustup")
        .arg("target")
        .arg("list")
        .arg("--installed")
        .kill_on_drop(true)
        .output()
        .await;
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim() == remote_server::setup::DEV_MUSL_TARGET),
        // 拿不到 rustup 输出时保守地认为未安装,从而触发回退。
        _ => false,
    }
}

/// 交叉编译本地 `warp` 二进制到 musl,返回产物路径。
///
/// profile / features 与 `script/deploy_remote_server` 对齐。
async fn cross_compile_remote_server(backend: &DevBuildBackend) -> Result<PathBuf> {
    let root = workspace_root();
    // 当前 channel 对应的 `[[bin]]` 名 —— OSS fork 是 `infinishell`(见 app/Cargo.toml)。
    // 不能写死 `warp`:`warp` 那个 bin 走 `load_config!("local")`,需要私有的
    // `warp-channel-config` 才能生成 `local_config.json`,OSS fork 没有它会编译失败;
    // `infinishell`(src/bin/infinishell.rs)内联 `ChannelConfig`,无此依赖。
    let bin_name = remote_server::setup::binary_name();
    let backend_desc = match backend {
        DevBuildBackend::Zigbuild => "cargo-zigbuild".to_string(),
        DevBuildBackend::MuslGcc(linker) => format!("cargo-build/{linker}"),
    };
    log::info!(
        "dev remote-server: 交叉编译 {bin_name} -> {} (profile={}, backend={backend_desc})",
        remote_server::setup::DEV_MUSL_TARGET,
        remote_server::setup::DEV_REMOTE_PROFILE,
    );
    // 首次会编译整个 warp,耗时通常数分钟。stdout/stderr 直接 inherit 到运行
    // InfiniShell 的终端,这样开发者能看到 cargo 的实时编译进度(否则全程静默,
    // 容易误以为卡死)。
    log::info!(
        "dev remote-server: 正在交叉编译,首次通常需数分钟 —— cargo 进度会打印到\
         运行 InfiniShell 的终端"
    );

    let status = async {
        let mut cmd = command::r#async::Command::new("cargo");
        cmd.current_dir(&root);
        // 注入 `~/.cargo/bin`,确保 `cargo zigbuild` 能解析 `cargo-zigbuild`
        // 子命令,且 cargo-zigbuild 能找到 `cargo` / `rustc`。
        if let Some(path) = dev_build_path_env() {
            cmd.env("PATH", path);
        }
        match backend {
            // zigbuild 是 cargo 子命令,自带 zig 链接器与 C/C++ 交叉编译器,
            // 无需再设 LINKER env。
            DevBuildBackend::Zigbuild => {
                cmd.arg("zigbuild");
            }
            // 原生 cargo build:通过 env 指定 musl 链接器并覆盖 rustflags,
            // 避免 .cargo/config.toml 里 macOS 专用 flag 污染交叉编译。
            DevBuildBackend::MuslGcc(linker) => {
                cmd.arg("build")
                    .env("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER", *linker)
                    .env(
                        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS",
                        "-C symbol-mangling-version=v0",
                    );
            }
        }
        cmd.arg("-p")
            .arg("warp")
            .arg("--bin")
            .arg(bin_name)
            .arg("--target")
            .arg(remote_server::setup::DEV_MUSL_TARGET)
            .arg("--profile")
            .arg(remote_server::setup::DEV_REMOTE_PROFILE)
            .arg("--features")
            .arg(remote_server::setup::DEV_REMOTE_FEATURES)
            // inherit:把 cargo 实时进度透到终端,而不是全程静默缓冲。
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .status()
            .await
    }
    .with_timeout(remote_server::setup::DEV_CROSS_COMPILE_TIMEOUT)
    .await
    .map_err(|_| {
        anyhow!(
            "dev remote-server 交叉编译超时(>{:?})",
            remote_server::setup::DEV_CROSS_COMPILE_TIMEOUT
        )
    })?
    .map_err(|e| anyhow!("无法启动 cargo 构建: {e}"))?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(anyhow!(
            "cargo 交叉编译失败(exit {code}),详见运行 InfiniShell 的终端的 cargo 输出"
        ));
    }

    // 产物位置:`<target_dir>/<triple>/<profile>/<bin_name>`。
    // 优先读 `CARGO_TARGET_DIR`,否则回退到 `<workspace>/target`。仓库未在
    // `.cargo/config.toml` 里设 `[build] target-dir`,故只需考虑 env。
    let target_root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let binary = target_root
        .join(remote_server::setup::DEV_MUSL_TARGET)
        .join(remote_server::setup::DEV_REMOTE_PROFILE)
        .join(bin_name);
    if !binary.is_file() {
        return Err(anyhow!(
            "交叉编译完成但未在 {} 找到产物(若设置了 CARGO_TARGET_DIR 请确认路径)",
            binary.display()
        ));
    }
    Ok(binary)
}

/// 开发模式安装:交叉编译本地 `warp` 并上传到远端 remote-server 路径。
///
/// 上传目标与 `remote_server_binary()` 完全一致,确保随后的
/// `check_binary()` / proxy 启动能找到它。
async fn dev_install_local_binary(transport: &SshTransport) -> Result<()> {
    // 前置条件检查:缺任意一项都返回错误,由调用方回退到下载安装。
    if !musl_target_installed().await {
        return Err(anyhow!(
            "未安装 rust target {};可执行 `rustup target add {}`",
            remote_server::setup::DEV_MUSL_TARGET,
            remote_server::setup::DEV_MUSL_TARGET,
        ));
    }
    // 选择交叉编译后端:优先 `cargo zigbuild`(zig 自带完整 C/C++ musl 工具链,
    // 能编译 freetype-sys 等 C++ 依赖),否则回退到 musl-gcc。两者皆无则报错。
    let backend = select_dev_build_backend().ok_or_else(|| {
        anyhow!(
            "未找到可用的 musl 交叉编译后端。建议安装 cargo-zigbuild + zig\
             (`cargo install cargo-zigbuild`,并用包管理器安装 `zig`),\
             或安装完整的 musl C/C++ 交叉工具链({})",
            DEV_MUSL_LINKER_CANDIDATES.join(" / ")
        )
    })?;

    let local_binary = cross_compile_remote_server(&backend).await?;

    // 上传到 `remote_server_binary()` 解析出的精确路径,先建好父目录。
    let remote_binary = remote_server::setup::remote_server_binary();
    let remote_dir = remote_server::setup::remote_server_dir();
    let mkdir_output = transport
        .run_setup_command(
            RemoteSetupCommand {
                dialect: RemoteShellDialect::Posix,
                script: format!("mkdir -p {remote_dir}"),
            },
            remote_server::setup::CHECK_TIMEOUT,
        )
        .await
        .map_err(anyhow::Error::from)?;
    if !mkdir_output.status.success() {
        let code = mkdir_output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr);
        return Err(anyhow!(
            "远端 remote-server 目录创建失败(exit {code}): {stderr}"
        ));
    }

    if let Some((socket_path, _)) = transport.control_master() {
        log::info!(
            "dev remote-server: 上传本地交叉编译产物到 {remote_binary}(scp -C 压缩,数百 MB 可能需数分钟)"
        );
        remote_server::ssh::scp_upload(
            socket_path,
            &local_binary,
            &remote_binary,
            remote_server::setup::DEV_UPLOAD_TIMEOUT,
        )
        .await?;
    } else {
        log::info!(
            "dev remote-server: 通过单连接 Rust SSH transport 上传本地交叉编译产物到 {remote_binary}"
        );
        transport
            .upload_file(
                &local_binary,
                &remote_binary,
                remote_server::setup::DEV_UPLOAD_TIMEOUT,
            )
            .await
            .map_err(anyhow::Error::from)?;
    }

    // 赋予可执行权限。
    let chmod_output = transport
        .run_setup_command(
            RemoteSetupCommand {
                dialect: RemoteShellDialect::Posix,
                script: format!("chmod 755 {remote_binary}"),
            },
            remote_server::setup::CHECK_TIMEOUT,
        )
        .await
        .map_err(anyhow::Error::from)?;
    if !chmod_output.status.success() {
        let code = chmod_output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&chmod_output.stderr);
        return Err(anyhow!("远端 chmod 失败(exit {code}): {stderr}"));
    }

    // 复用既有校验逻辑确认上传的二进制可运行。
    verify_installed_binary(transport).await
}

impl RemoteTransport for SshTransport {
    fn detect_platform(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemotePlatform, Error>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let output = transport
                .run_setup_command(
                    remote_server::setup::platform_probe_command(&transport.remote_os),
                    remote_server::setup::CHECK_TIMEOUT,
                )
                .await?;
            if output.status.success() {
                let platform = parse_platform_output(&String::from_utf8_lossy(&output.stdout))?;
                *transport
                    .detected_platform
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(platform.clone());
                Ok(platform)
            } else {
                let code = output.status.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(Error::Other(anyhow::anyhow!(
                    "platform probe exited with code {code}: {stderr}"
                )))
            }
        })
    }

    fn run_preinstall_check(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<PreinstallCheckResult, Error>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let Some(command) =
                remote_server::setup::preinstall_check_command(&transport.remote_os)
            else {
                return Ok(PreinstallCheckResult {
                    status: remote_server::setup::PreinstallStatus::Supported,
                    libc: remote_server::setup::RemoteLibc::Unknown,
                    raw: String::new(),
                });
            };
            let output = transport
                .run_setup_command(command, remote_server::setup::CHECK_TIMEOUT)
                .await?;
            if output.status.success() {
                Ok(PreinstallCheckResult::parse(&String::from_utf8_lossy(
                    &output.stdout,
                )))
            } else {
                let exit_code = output.status.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                Err(Error::ScriptFailed { exit_code, stderr })
            }
        })
    }

    fn check_binary(&self) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            log::info!("Running remote-server binary check");
            let output = transport
                .run_setup_command(
                    remote_server::setup::binary_check_command_for(&transport.remote_os),
                    remote_server::setup::CHECK_TIMEOUT,
                )
                .await?;
            // `<binary> --version` exits 0 when present, executable, and
            // functional. Exit 127 means the binary was not found, and 126
            // means it exists but is not executable. Any other non-zero
            // exit (e.g. SSH exit 255 for a dead connection, or signal
            // termination) is treated as a transport-level failure.
            let code = output.status.code();
            log::info!("Binary check result: exit={code:?}");
            match code {
                Some(0) => Ok(true),
                Some(1) if matches!(transport.remote_os, RemoteOs::Windows) => Ok(false),
                Some(126) | Some(127) => Ok(false),
                Some(code) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(Error::Other(anyhow::anyhow!(
                        "binary check exited with code {code}: {stderr}"
                    )))
                }
                None => Err(Error::Other(anyhow::anyhow!(
                    "binary check terminated by signal"
                ))),
            }
        })
    }

    fn check_has_old_binary(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            // Treat the existence of the remote-server install directory
            // itself as evidence of a prior install. If `~/.warp-XX/remote-server`
            // exists, something was installed there before, so any mismatch
            // with the client's expected binary path should be auto-updated
            // rather than surfaced as a first-time install prompt.
            let command = match transport.remote_os {
                RemoteOs::Linux | RemoteOs::MacOs => RemoteSetupCommand {
                    dialect: RemoteShellDialect::Posix,
                    script: format!("test -d {}", remote_server::setup::remote_server_dir()),
                },
                RemoteOs::Windows => {
                    let relative_dir = remote_server::setup::remote_server_dir()
                        .trim_start_matches("~/")
                        .replace('/', "\\");
                    RemoteSetupCommand {
                        dialect: RemoteShellDialect::PowerShell,
                        script: format!(
                            "$dir = Join-Path $HOME '{}'; if (Test-Path -LiteralPath $dir -PathType Container) {{ exit 0 }} else {{ exit 1 }}",
                            relative_dir.replace('\'', "''")
                        ),
                    }
                }
            };
            let output = transport
                .run_setup_command(command, remote_server::setup::CHECK_TIMEOUT)
                .await
                .map_err(anyhow::Error::from)?;
            // `test -d` exits 0 when present, 1 when missing.
            // Anything else is treated as a check failure.
            match output.status.code() {
                Some(0) => Ok(true),
                Some(1) => Ok(false),
                Some(code) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(anyhow::anyhow!(
                        "remote-server dir check exited with code {code}: {stderr}"
                    ))
                }
                None => Err(anyhow::anyhow!(
                    "remote-server dir check terminated by signal"
                )),
            }
        })
    }

    fn install_binary(&self) -> Pin<Box<dyn Future<Output = InstallOutcome> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            // Zap fork:DEBUG 源码构建(无 release tag)走开发模式,
            // 交叉编译本地 `warp` 并上传,而不是下载陈旧的 GitHub release。
            // 失败时(交叉编译前置条件缺失等)打印告警并回退到常规下载安装,
            // 保证 dev 体验不被破坏。release 构建跳过整段逻辑,行为不变。
            if remote_server::setup::is_dev_source_build()
                && matches!(transport.remote_os, RemoteOs::Linux)
            {
                log::info!("dev remote-server: 检测到 DEBUG 源码构建,改用本地交叉编译安装");
                match dev_install_local_binary(&transport).await {
                    Ok(()) => {
                        return InstallOutcome {
                            // 二进制由本机交叉编译后上传,与 SCP fallback 同属
                            // 客户端侧安装。
                            source: Some(InstallSource::Client),
                            result: Ok(()),
                        };
                    }
                    Err(error) => {
                        log::warn!(
                            "dev remote-server: 本地交叉编译安装不可用,回退到下载安装: {error:#}"
                        );
                        // 落空,继续走下方常规下载安装流程。
                    }
                }
            }

            if matches!(transport.remote_os, RemoteOs::Linux | RemoteOs::MacOs)
                && let Some((socket_path, _)) = transport.control_master()
            {
                return installation::install_binary(socket_path).await;
            }

            let platform = match transport
                .detected_platform
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                Some(platform) => platform,
                None => {
                    return InstallOutcome {
                        source: None,
                        result: Err(Error::Other(anyhow::anyhow!(
                            "remote platform is unavailable for installation"
                        ))),
                    };
                }
            };

            match installation::scp_fallback::bundled_remote_server_tarball(&platform).await {
                Ok(Some(archive)) => {
                    match transport.install_client_archive(&platform, &archive).await {
                        Ok(()) => {
                            return InstallOutcome {
                                source: Some(InstallSource::Client),
                                result: Ok(()),
                            };
                        }
                        Err(error) if !installation::scp_fallback::should_try_install(&error) => {
                            return InstallOutcome {
                                source: Some(InstallSource::Client),
                                result: Err(error),
                            };
                        }
                        Err(error) => {
                            log::warn!(
                                "bundled remote-server upload failed; falling back to remote download: {error:#}"
                            );
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    log::warn!(
                        "bundled remote-server archive is unavailable; falling back to remote download: {error:#}"
                    );
                }
            }

            match transport.install_for_platform(&platform, None).await {
                Ok(()) => InstallOutcome {
                    source: Some(InstallSource::Server),
                    result: Ok(()),
                },
                Err(server_error)
                    if installation::scp_fallback::should_try_install(&server_error) =>
                {
                    let result = async {
                        let archive =
                            installation::scp_fallback::cached_remote_server_tarball(&platform)
                                .await
                                .map_err(Error::Other)?;
                        transport.install_client_archive(&platform, &archive).await
                    }
                    .await;
                    InstallOutcome {
                        source: Some(InstallSource::Client),
                        result,
                    }
                }
                Err(server_error) => InstallOutcome {
                    source: Some(InstallSource::Server),
                    result: Err(server_error),
                },
            }
        })
    }

    fn connect(
        &self,
        executor: Arc<executor::Background>,
    ) -> Pin<Box<dyn Future<Output = Result<Connection>> + Send>> {
        let transport = self.clone();
        let remote_proxy_command = setup_command_line(&self.remote_proxy_command());
        Box::pin(async move {
            // `kill_on_drop(true)` pairs with ownership of the `Child` being
            // returned in the [`Connection`] below: the
            // [`RemoteServerManager`] holds the `Child` on its per-session
            // state, and dropping that state (on explicit teardown or
            // spontaneous disconnect) sends SIGKILL to this ssh process.
            let (mut child, control_path) = match &transport.backend {
                SshTransportBackend::ControlMaster {
                    socket_path,
                    warp_owns_control_master,
                } => {
                    let mut args = ssh_args(socket_path);
                    args.push(remote_proxy_command);
                    let child = command::r#async::Command::new("ssh")
                        .args(&args)
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .kill_on_drop(true)
                        .spawn()?;
                    let control_path = if *warp_owns_control_master {
                        ControlPath::WarpManaged(socket_path.clone())
                    } else {
                        ControlPath::UserOwned(socket_path.clone())
                    };
                    (child, control_path)
                }
                SshTransportBackend::RustBroker {
                    endpoint,
                    capability,
                } => {
                    let executable = crate::remote_server::rust_ssh::worker_executable()?;
                    let child = command::r#async::Command::new(executable)
                        .arg("rust-ssh-broker-command")
                        .arg("--endpoint")
                        .arg(endpoint)
                        .arg("--command")
                        .arg(remote_proxy_command)
                        .env(
                            crate::remote_server::rust_ssh::BROKER_CAPABILITY_ENV,
                            capability,
                        )
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .kill_on_drop(true)
                        .spawn()?;
                    (child, ControlPath::None)
                }
            };

            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture child stdin"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture child stdout"))?;
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture child stderr"))?;

            let (client, event_rx, failure_rx, host_response_rx, stderr_tail) =
                RemoteServerClient::from_child_streams(stdin, stdout, stderr, &executor);
            Ok(Connection {
                client,
                event_rx,
                failure_rx,
                host_response_rx,
                resource: Box::new(LocalProcessConnection::new(child, stderr_tail)),
                control_path,
            })
        })
    }

    fn remove_remote_server_binary(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            log::info!("Removing stale remote server binary");
            let output = transport
                .run_setup_command(
                    remote_server::setup::remote_server_removal_command_for(&transport.remote_os),
                    remote_server::setup::CHECK_TIMEOUT,
                )
                .await
                .map_err(anyhow::Error::from)?;
            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("Failed to remove binary: {stderr}"))
            }
        })
    }

    fn is_reconnectable(&self, exit_status: Option<&RemoteServerExitStatus>) -> bool {
        match exit_status {
            Some(status) if status.signal_killed => false,
            Some(status) => match &self.backend {
                // OpenSSH reserves 255 for a connection-level failure, which
                // means the ControlMaster can no longer open another channel.
                SshTransportBackend::ControlMaster { .. } => status.code != Some(255),
                // The broker worker also uses 255 for a single command/protocol
                // failure. The parent Rust SSH session can still be healthy, so
                // this status must not inherit ControlMaster-only semantics.
                SshTransportBackend::RustBroker { .. } => true,
            },
            // No exit status available — optimistically allow reconnect.
            None => true,
        }
    }
}

#[cfg(test)]
#[path = "ssh_transport_tests.rs"]
mod tests;
