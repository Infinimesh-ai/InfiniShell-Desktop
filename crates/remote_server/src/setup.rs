mod glibc;
mod windows;

use std::time::Duration;

use anyhow::anyhow;
pub use glibc::{GlibcVersion, RemoteLibc};
use warp_core::channel::{Channel, ChannelState};
use warp_core::paths::OSS_CONFIG_DIR;
pub const REMOTE_SERVER_ARTIFACT_VERSION_UNPINNED: &str = "unversioned";

/// State machine for the remote server install → launch → initialize flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteServerSetupState {
    /// Checking if the binary exists on remote.
    Checking,
    /// Downloading and installing the binary for the first time on this host.
    Installing { progress_percent: Option<u8> },
    /// Replacing an existing install with a differently-versioned binary.
    /// Rendered as "Updating..." in the UI so the user understands this
    /// isn't a fresh install.
    Updating,
    /// Binary is launched, waiting for InitializeResponse.
    Initializing,
    /// Handshake complete. Ready.
    Ready,
    /// Something failed. Fall back to ControlMaster.
    Failed { error: String },
    /// Preinstall check classified the host as incompatible with the
    /// prebuilt remote-server binary. The controller treats this as a
    /// clean fall-back to the legacy ControlMaster-backed SSH flow,
    /// distinct from `Failed` (which is rendered as a real error).
    Unsupported { reason: UnsupportedReason },
}

impl RemoteServerSetupState {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }

    pub fn is_terminal(&self) -> bool {
        self.is_ready() || self.is_failed() || self.is_unsupported()
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Installing { .. } | Self::Updating | Self::Initializing
        )
    }

    pub fn is_connecting(&self) -> bool {
        matches!(
            self,
            Self::Installing { .. } | Self::Updating | Self::Initializing
        )
    }
}

impl From<&crate::transport::Error> for RemoteServerSetupState {
    fn from(error: &crate::transport::Error) -> Self {
        if let Some(reason) = UnsupportedReason::from_transport_error(error) {
            Self::Unsupported { reason }
        } else {
            Self::Failed {
                error: error.to_string(),
            }
        }
    }
}

/// Outcome of [`crate::transport::RemoteTransport::run_preinstall_check`].
///
/// The script runs over the existing SSH socket before any install UI
/// surfaces and reports whether the host can run the prebuilt
/// remote-server binary. The Rust side is intentionally a thin parser
/// over the script's structured stdout (see `preinstall_check.sh`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreinstallCheckResult {
    pub status: PreinstallStatus,
    pub libc: RemoteLibc,
    /// Verbatim, trimmed script stdout. Forwarded to telemetry for
    /// diagnosing `Unknown` outcomes on exotic distros.
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreinstallStatus {
    Supported,
    Unsupported {
        reason: UnsupportedReason,
    },
    /// Probe ran but couldn't classify the host. Treated as supported
    /// (fail open) by [`PreinstallCheckResult::is_supported`] so we keep
    /// today's install-and-try behavior on hosts where the probe is
    /// unreliable.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsupportedReason {
    GlibcTooOld {
        detected: GlibcVersion,
        required: GlibcVersion,
    },
    NonGlibc {
        name: String,
    },
    UnsupportedOs {
        os: String,
    },
    UnsupportedArch {
        arch: String,
    },
}

impl UnsupportedReason {
    pub fn from_transport_error(error: &crate::transport::Error) -> Option<Self> {
        match error {
            crate::transport::Error::UnsupportedOs { os } => {
                Some(Self::UnsupportedOs { os: os.clone() })
            }
            crate::transport::Error::UnsupportedArch { arch } => {
                Some(Self::UnsupportedArch { arch: arch.clone() })
            }
            crate::transport::Error::TimedOut
            | crate::transport::Error::ScriptFailed { .. }
            | crate::transport::Error::Other(_) => None,
        }
    }

    pub fn as_telemetry_reason(&self) -> &'static str {
        match self {
            Self::GlibcTooOld { .. } => "glibc_too_old",
            Self::NonGlibc { .. } => "non_glibc",
            Self::UnsupportedOs { .. } => "unsupported_os",
            Self::UnsupportedArch { .. } => "unsupported_arch",
        }
    }
}

impl PreinstallCheckResult {
    pub fn unsupported(reason: UnsupportedReason) -> Self {
        Self {
            status: PreinstallStatus::Unsupported { reason },
            libc: RemoteLibc::Unknown,
            raw: String::new(),
        }
    }
    /// Whether the host is supported. Both `Supported` and `Unknown`
    /// return true — only positive detection of an incompatible libc
    /// triggers the silent fall-back.
    pub fn is_supported(&self) -> bool {
        match self.status {
            PreinstallStatus::Supported | PreinstallStatus::Unknown => true,
            PreinstallStatus::Unsupported { .. } => false,
        }
    }

    /// Parses the structured `key=value` stdout emitted by
    /// `preinstall_check.sh`. Tolerates unknown keys and lines without
    /// `=` (forward-compatibility): future versions of the script can
    /// add new keys without coordinating a client release.
    pub fn parse(stdout: &str) -> Self {
        let mut status_str: Option<&str> = None;
        let mut reason_str: Option<&str> = None;
        let mut libc_family: Option<&str> = None;
        let mut libc_version: Option<&str> = None;
        let mut required_glibc: Option<&str> = None;

        for line in stdout.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "status" => status_str = Some(value.trim()),
                "reason" => reason_str = Some(value.trim()),
                "libc_family" => libc_family = Some(value.trim()),
                "libc_version" => libc_version = Some(value.trim()),
                "required_glibc" => required_glibc = Some(value.trim()),
                _ => {} // ignore unknown keys
            }
        }

        let libc = glibc::parse_libc(libc_family, libc_version);
        let status = parse_status(status_str, reason_str, &libc, required_glibc);

        Self {
            status,
            libc,
            raw: stdout.trim().to_string(),
        }
    }
}

fn parse_status(
    status: Option<&str>,
    reason: Option<&str>,
    _libc: &RemoteLibc,
    _required_glibc: Option<&str>,
) -> PreinstallStatus {
    // remote-server 现在是静态 musl 二进制(见 `preinstall_check.sh` 顶部
    // 注释),不链接宿主的动态 libc。因此 `glibc_too_old` / `non_glibc`
    // 已不再是「不支持」的理由 —— 任意 glibc 版本与 musl/uclibc 宿主都能
    // 运行该二进制。新版脚本不会再发出这两个 reason;但旧版 remote 端可能
    // 仍缓存着老脚本,所以这里把这些 libc 门禁理由一并当作 `Supported`,
    // 而不是 `Unsupported`,保持新旧脚本的判定一致。
    match status {
        Some("supported") => PreinstallStatus::Supported,
        Some("unsupported") => match reason {
            // 旧脚本残留的 libc 门禁理由:静态二进制下已失效,视为支持。
            Some("glibc_too_old") | Some("non_glibc") => PreinstallStatus::Supported,
            // 其他无法识别的 unsupported 理由:保守起见 fail open。
            _ => PreinstallStatus::Unknown,
        },
        // status=unknown, missing, or anything else → fail open.
        _ => PreinstallStatus::Unknown,
    }
}

/// The bundled preinstall check script. Loaded as a string so the SSH
/// transport can pipe it through the existing ControlMaster socket via
/// [`crate::ssh::run_ssh_script`].
///
/// The script is intentionally self-contained — the supported-glibc
/// floor is hardcoded inside the script (see `preinstall_check.sh`)
/// rather than templated from Rust.
pub const PREINSTALL_CHECK_SCRIPT: &str = include_str!("preinstall_check.sh");

/// 探测到的远端平台。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePlatform {
    pub os: RemoteOs,
    pub arch: RemoteArch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteOs {
    Linux,
    MacOs,
    Windows,
}

impl RemoteOs {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::MacOs => "macos",
            Self::Windows => "windows",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteArch {
    X86_64,
    Aarch64,
}

impl RemoteArch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

/// 远端命令应由哪种 shell 解释。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteShellDialect {
    Posix,
    PowerShell,
    WindowsCmd,
}

/// transport 可执行的远端命令或脚本。
///
/// `PowerShell` 脚本应由 transport 编码成 UTF-16LE Base64，并通过
/// `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand` 执行，
/// 避免把脚本直接拼进远端默认 shell。
/// `WindowsCmd` 只用于必须保留原始 stdio 字节的原生命令；PowerShell 会把
/// 非交互命令的 stdout 缓冲到进程退出，不能承载 remote-server 协议。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSetupCommand {
    pub dialect: RemoteShellDialect,
    pub script: String,
}

impl RemoteSetupCommand {
    pub fn posix(script: impl Into<String>) -> Self {
        Self {
            dialect: RemoteShellDialect::Posix,
            script: script.into(),
        }
    }

    pub fn powershell(script: impl Into<String>) -> Self {
        Self {
            dialect: RemoteShellDialect::PowerShell,
            script: script.into(),
        }
    }

    fn windows_cmd(script: String) -> Self {
        Self {
            dialect: RemoteShellDialect::WindowsCmd,
            script,
        }
    }
}

/// Parse platform probe output into a [`RemotePlatform`].
///
/// 格式为 `<os> <arch>`，例如 `Linux x86_64`、`Darwin arm64` 或
/// `Windows AMD64`。读取最后一行以跳过 shell 初始化输出。
pub fn parse_platform_output(
    output: &str,
) -> std::result::Result<RemotePlatform, crate::transport::Error> {
    use crate::transport::Error;

    let line = output
        .lines()
        .last()
        .ok_or_else(|| Error::Other(anyhow!("empty platform probe output")))
        .map(str::trim)?;

    let mut parts = line.split_whitespace();
    let os_str = parts
        .next()
        .ok_or_else(|| Error::Other(anyhow!("missing OS in platform probe output: {line}")))?;
    let arch_str = parts
        .next()
        .ok_or_else(|| Error::Other(anyhow!("missing arch in platform probe output: {line}")))?;

    let os = if os_str.eq_ignore_ascii_case("linux") {
        RemoteOs::Linux
    } else if os_str.eq_ignore_ascii_case("darwin") {
        RemoteOs::MacOs
    } else if os_str.eq_ignore_ascii_case("windows") || os_str.eq_ignore_ascii_case("windows_nt") {
        RemoteOs::Windows
    } else {
        return Err(Error::UnsupportedOs {
            os: os_str.to_string(),
        });
    };

    let arch = if arch_str.eq_ignore_ascii_case("x86_64") || arch_str.eq_ignore_ascii_case("amd64")
    {
        RemoteArch::X86_64
    } else if arch_str.eq_ignore_ascii_case("aarch64") || arch_str.eq_ignore_ascii_case("arm64") {
        RemoteArch::Aarch64
    } else {
        return Err(Error::UnsupportedArch {
            arch: arch_str.to_string(),
        });
    };

    Ok(RemotePlatform { os, arch })
}

/// 保留原 API；Unix transport 的 `uname -sm` 输出同样走通用平台解析器。
pub fn parse_uname_output(
    output: &str,
) -> std::result::Result<RemotePlatform, crate::transport::Error> {
    parse_platform_output(output)
}

/// 返回已知平台对应的平台探测命令。
pub fn platform_probe_command(os: &RemoteOs) -> RemoteSetupCommand {
    match os {
        RemoteOs::Linux | RemoteOs::MacOs => RemoteSetupCommand::posix("uname -sm".to_string()),
        RemoteOs::Windows => RemoteSetupCommand::powershell(windows::platform_probe_script()),
    }
}

/// 返回预安装检查；Windows 不需要 Linux libc 检查。
pub fn preinstall_check_command(os: &RemoteOs) -> Option<RemoteSetupCommand> {
    match os {
        RemoteOs::Linux | RemoteOs::MacOs => Some(RemoteSetupCommand::posix(
            PREINSTALL_CHECK_SCRIPT.to_string(),
        )),
        RemoteOs::Windows => None,
    }
}

/// 返回远端二进制安装目录,按 channel 隔离。
///
/// - stable:      `~/.warp/remote-server`
/// - preview:     `~/.warp-preview/remote-server`
/// - dev:         `~/.warp-dev/remote-server`
/// - local:       `~/.warp-local/remote-server`
/// - integration: `~/.warp-dev/remote-server`
/// - oss:         `~/.infinishell/remote-server`
pub fn remote_server_dir() -> String {
    format!("~/{}/remote-server", remote_server_config_dir_name())
}

fn remote_server_config_dir_name() -> &'static str {
    match ChannelState::channel() {
        Channel::Stable => ".warp",
        Channel::Preview => ".warp-preview",
        Channel::Dev | Channel::Integration => ".warp-dev",
        Channel::Local => ".warp-local",
        Channel::Oss => OSS_CONFIG_DIR,
    }
}

fn remote_server_relative_dir() -> String {
    format!("{}/remote-server", remote_server_config_dir_name())
}

/// Returns a short, deterministic directory name for a remote-server
/// identity key, used for the daemon socket and PID file paths.
///
/// Hashes the key to 8 hex chars so the socket path stays within the
/// `sun_path` limit across all channels.
pub fn remote_server_identity_dir_name(identity_key: &str) -> String {
    use std::hash::{Hash, Hasher};

    if identity_key.is_empty() {
        return "empty".to_string();
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    identity_key.hash(&mut hasher);
    format!("{:016x}", hasher.finish())[..8].to_string()
}

/// Percent-encodes an identity key for use in filesystem paths.
///
/// Keeps ASCII alphanumeric characters plus `-` and `_`; percent-encodes
/// all other bytes.  Used by [`remote_server_daemon_data_dir`] for
/// persistent data that must not collide across identities.
fn percent_encode_identity_key(identity_key: &str) -> String {
    if identity_key.is_empty() {
        return "empty".to_string();
    }

    let mut encoded = String::with_capacity(identity_key.len());
    for byte in identity_key.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Returns the identity-scoped remote directory used for the daemon socket
/// and PID file.  Uses the hashed identity dir name so the full socket
/// path fits within `sun_path`.
pub fn remote_server_daemon_dir(identity_key: &str) -> String {
    format!(
        "{}/{}",
        remote_server_dir(),
        remote_server_identity_dir_name(identity_key)
    )
}

/// Returns the identity-scoped remote directory used for daemon-owned
/// per-user data files (e.g. SQLite databases).
///
/// Uses the full percent-encoded identity key (not the hash) so that
/// persistent data is never shared between distinct identities due to
/// a hash collision.  The `sun_path` limit does not apply here because
/// this path is only used for regular file I/O, not Unix sockets.
pub fn remote_server_daemon_data_dir(identity_key: &str) -> String {
    format!(
        "{}/{}/data",
        remote_server_dir(),
        percent_encode_identity_key(identity_key)
    )
}

/// Returns a short, deterministic 8-hex-char hash of the app version string.
///
/// Used to version-discriminate daemon socket and PID files without
/// embedding the full version string in the filename, which would push
/// the Unix domain socket path over the `sun_path` limit (107 bytes on
/// Linux, 103 on macOS) for users with moderately long identity keys or
/// home directory paths.
pub fn version_hash() -> Option<String> {
    use std::hash::{Hash, Hasher};

    let version = ChannelState::app_version()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    version.hash(&mut hasher);
    Some(format!("{:016x}", hasher.finish())[..8].to_string())
}

/// Returns the daemon socket filename, versioned with a short hash when
/// a release tag is baked in.
///
/// - With `GIT_RELEASE_TAG`:    `server-{hash8}.sock`  (e.g. `server-a1b2c3d4.sock`)
/// - Without (plain cargo run): `server.sock`
pub fn daemon_socket_name() -> String {
    match version_hash() {
        Some(hash) => format!("server-{hash}.sock"),
        None => "server.sock".to_string(),
    }
}

/// Returns the daemon PID filename, versioned with a short hash when a
/// release tag is baked in.
///
/// - With `GIT_RELEASE_TAG`:    `server-{hash8}.pid`
/// - Without (plain cargo run): `server.pid`
pub fn daemon_pid_name() -> String {
    match version_hash() {
        Some(hash) => format!("server-{hash}.pid"),
        None => "server.pid".to_string(),
    }
}

/// 返回远端 remote-server 二进制文件名。
pub fn binary_name() -> &'static str {
    ChannelState::channel().cli_command_name()
}

/// 返回平台对应的版本化 remote-server 文件名。
pub fn remote_server_binary_name(os: &RemoteOs) -> String {
    let extension = match os {
        RemoteOs::Linux | RemoteOs::MacOs => "",
        RemoteOs::Windows => ".exe",
    };
    format!("{}{}{extension}", binary_name(), version_suffix())
}

/// 返回当前 channel 和客户端版本对应的远端二进制完整路径。
///
/// Local 构建保留无版本后缀路径,以便 `script/deploy_remote_server`
/// 覆盖同一个开发 slot。InfiniShell release 构建带 `GIT_RELEASE_TAG`
/// 时使用版本后缀,这样新版本会自然触发重新安装;源码本地构建没有
/// release tag,仍使用无后缀路径。
pub fn remote_server_binary() -> String {
    remote_server_binary_for(&RemoteOs::Linux)
}

/// 返回适用于目标平台 shell 的 remote-server 路径表达式。
pub fn remote_server_binary_for(os: &RemoteOs) -> String {
    let binary_name = remote_server_binary_name(os);
    match os {
        RemoteOs::Linux | RemoteOs::MacOs => format!("{}/{binary_name}", remote_server_dir()),
        RemoteOs::Windows => format!("$HOME/{}/{binary_name}", remote_server_relative_dir()),
    }
}

/// 返回检查远端 remote-server 二进制存在且可执行的 shell 命令。
///
/// 与上游一致,这里实际运行 `--version`,而不只是 `test -x`;
/// 这样可以把损坏或无法解析参数的二进制提前识别出来。
pub fn binary_check_command() -> String {
    format!("{} --version", remote_server_binary())
}

/// 返回目标平台对应的二进制检查命令。
pub fn binary_check_command_for(os: &RemoteOs) -> RemoteSetupCommand {
    match os {
        RemoteOs::Linux | RemoteOs::MacOs => RemoteSetupCommand::posix(binary_check_command()),
        RemoteOs::Windows => RemoteSetupCommand::powershell(windows::binary_check_script(
            &remote_server_relative_binary_path(os),
        )),
    }
}

/// 返回检查远端是否存在任一旧版 remote-server 二进制的命令。
///
/// 不能只检查安装目录：上传中的临时压缩包也会创建该目录，把它当成旧安装会让
/// 同时打开的第二个 SSH 会话错误显示为“正在升级”。
pub fn old_binary_check_command_for(os: &RemoteOs) -> RemoteSetupCommand {
    match os {
        RemoteOs::Linux | RemoteOs::MacOs => RemoteSetupCommand::posix(format!(
            "for path in {}/{}*; do [ -f \"$path\" ] && exit 0; done; exit 1",
            remote_server_dir(),
            binary_name(),
        )),
        RemoteOs::Windows => {
            let relative_dir = remote_server_relative_dir().replace('/', "\\");
            let binary_pattern = format!("{}*.exe", binary_name());
            RemoteSetupCommand::powershell(format!(
                "$dir = Join-Path $HOME '{}'; $binary = Get-ChildItem -LiteralPath $dir -File -Filter '{}' -ErrorAction SilentlyContinue | Select-Object -First 1; if ($null -ne $binary) {{ exit 0 }} else {{ exit 1 }}",
                relative_dir.replace('\'', "''"),
                binary_pattern.replace('\'', "''"),
            ))
        }
    }
}

/// Returns the shell command to remove the current remote-server binary.
///
/// The global bundled resources directory is deliberately left in place:
/// the next install overwrites it, and an older daemon that is still
/// running parsed its skills at startup.
pub fn remote_server_removal_command() -> String {
    format!("rm -f {}", remote_server_binary())
}

/// 返回目标平台对应的清理命令。全局 bundled resources 保持不变。
pub fn remote_server_removal_command_for(os: &RemoteOs) -> RemoteSetupCommand {
    match os {
        RemoteOs::Linux | RemoteOs::MacOs => {
            RemoteSetupCommand::posix(remote_server_removal_command())
        }
        RemoteOs::Windows => RemoteSetupCommand::powershell(windows::removal_script(
            &remote_server_relative_binary_path(os),
        )),
    }
}

/// 返回通过 SSH 启动 remote-server proxy 的平台命令。
pub fn remote_server_proxy_command(os: &RemoteOs, identity_key: &str) -> RemoteSetupCommand {
    match os {
        RemoteOs::Linux | RemoteOs::MacOs => RemoteSetupCommand::posix(format!(
            "{} remote-server-proxy --identity-key {}",
            remote_server_binary_for(os),
            posix_single_quoted(identity_key),
        )),
        RemoteOs::Windows => RemoteSetupCommand::windows_cmd(windows::proxy_command(
            &remote_server_relative_binary_path(os),
            identity_key,
        )),
    }
}

pub fn remote_server_relative_binary_path(os: &RemoteOs) -> String {
    format!(
        "{}/{}",
        remote_server_relative_dir(),
        remote_server_binary_name(os)
    )
}

fn posix_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// 返回用于版本化安装路径的版本号。优先使用编译时注入的
/// `GIT_RELEASE_TAG`;没有 release tag 时回退到 `CARGO_PKG_VERSION`,
/// 让需要版本化路径的 channel 保持确定性,并在缺少对应 release 资产时
/// 清晰失败,而不是误用无版本路径。
fn pinned_version() -> &'static str {
    ChannelState::app_version().unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// Returns the version key used to identify remote-server download artifacts.
///
/// This must match the versioning used by [`download_tarball_url`] and
/// [`install_script`], so versioned download URLs do not reuse stale tarballs
/// from a previous client version.
pub fn remote_server_artifact_version() -> &'static str {
    match ChannelState::channel() {
        Channel::Local => REMOTE_SERVER_ARTIFACT_VERSION_UNPINNED,
        Channel::Oss if ChannelState::app_version().is_none() => {
            REMOTE_SERVER_ARTIFACT_VERSION_UNPINNED
        }
        Channel::Oss | Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            pinned_version()
        }
    }
}

/// Name of the global, version-independent resources directory inside
/// [`remote_server_dir`], populated by the install script from the
/// artifact's `resources/` tree (bundled skills, settings schema).
pub const BUNDLED_RESOURCES_DIR_NAME: &str = "bundled_resources";

/// Returns the global, version-independent directory where the install
/// script places the artifact's `resources/` tree. Shell-form path
/// (`~/...`); the daemon expands it against its own home directory.
///
/// Deliberately not version-scoped: the last install wins, and slight
/// version skew between the resources and a running daemon is accepted
/// (the daemon parses its skills once at startup).
pub fn remote_server_bundled_resources_dir() -> String {
    format!("{}/{}", remote_server_dir(), BUNDLED_RESOURCES_DIR_NAME)
}

/// 安装脚本模板独立放在 `.sh` 文件里方便维护。
/// `{download_base_url}` 等占位符由 [`install_script`] 替换。
const INSTALL_SCRIPT_TEMPLATE: &str = include_str!("install_remote_server.sh");

/// 返回安装脚本。`staging_tarball_path` 非空时,脚本跳过远端下载,
/// 改为解压客户端通过 SCP 预上传的 tarball。
pub fn install_script(staging_tarball_path: Option<&str>) -> String {
    let version_suffix = version_suffix();
    INSTALL_SCRIPT_TEMPLATE
        .replace("{download_base_url}", &download_url())
        .replace("{install_dir}", &remote_server_dir())
        .replace("{binary_name}", binary_name())
        .replace("{version_suffix}", &version_suffix)
        .replace("{bundled_resources_dir_name}", BUNDLED_RESOURCES_DIR_NAME)
        .replace(
            "{no_http_client_exit_code}",
            &NO_HTTP_CLIENT_EXIT_CODE.to_string(),
        )
        .replace("{staging_tarball_path}", staging_tarball_path.unwrap_or(""))
}

/// 返回目标平台对应的安装脚本。
///
/// POSIX 平台保持现有 Bash/tar.gz 行为；Windows 返回 PowerShell/zip 脚本。
pub fn install_command(
    platform: &RemotePlatform,
    staging_archive_path: Option<&str>,
) -> RemoteSetupCommand {
    match platform.os {
        RemoteOs::Linux | RemoteOs::MacOs => {
            RemoteSetupCommand::posix(install_script(staging_archive_path))
        }
        RemoteOs::Windows => RemoteSetupCommand::powershell(windows::install_script(
            &remote_server_relative_dir(),
            &remote_server_binary_name(&platform.os),
            &download_archive_url(platform),
            staging_archive_path,
            BUNDLED_RESOURCES_DIR_NAME,
        )),
    }
}

/// 返回从已上传归档安装 remote-server 的命令。此路径不需要再次选择下载资产，
/// 因而只需要目标操作系统，不依赖远端架构。
pub fn install_staged_command(os: &RemoteOs, staging_archive_path: &str) -> RemoteSetupCommand {
    match os {
        RemoteOs::Linux | RemoteOs::MacOs => {
            RemoteSetupCommand::posix(install_script(Some(staging_archive_path)))
        }
        RemoteOs::Windows => RemoteSetupCommand::powershell(windows::install_script(
            &remote_server_relative_dir(),
            &remote_server_binary_name(os),
            "",
            Some(staging_archive_path),
            BUNDLED_RESOURCES_DIR_NAME,
        )),
    }
}

/// 构造 InfiniShell CLI release 资产下载基址。
fn download_url() -> String {
    let release_path = match ChannelState::app_version() {
        Some(tag) => format!("download/{tag}"),
        None => "latest/download".to_string(),
    };
    format!("https://github.com/Infinimesh-ai/InfiniShell-Desktop/releases/{release_path}")
}

fn version_suffix() -> String {
    match ChannelState::channel() {
        Channel::Local => String::new(),
        Channel::Oss if ChannelState::app_version().is_none() => String::new(),
        Channel::Oss | Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            format!("-{}", pinned_version())
        }
    }
}

/// 返回指定远端平台对应的 InfiniShell CLI tarball URL。
pub fn download_tarball_url(platform: &RemotePlatform) -> String {
    download_archive_url(platform)
}

/// 返回指定平台的 release archive URL。
pub fn download_archive_url(platform: &RemotePlatform) -> String {
    format!(
        "{}/{}",
        download_url(),
        remote_server_archive_name(platform)
    )
}

/// 返回指定远端平台对应的 InfiniShell CLI archive 文件名。
pub fn remote_server_archive_name(platform: &RemotePlatform) -> String {
    let extension = match platform.os {
        RemoteOs::Linux | RemoteOs::MacOs => "tar.gz",
        RemoteOs::Windows => "zip",
    };
    format!(
        "infinishell-{}-{}.{}",
        platform.os.as_str(),
        platform.arch.as_str(),
        extension,
    )
}

/// 保留原 API 名称；Windows 平台返回 `.zip` 文件名。
pub fn remote_server_tarball_name(platform: &RemotePlatform) -> String {
    remote_server_archive_name(platform)
}

/// Exit code the install script uses when neither curl nor wget is
/// available on the remote host. The Rust side matches on this to
/// trigger the SCP upload fallback.
pub const NO_HTTP_CLIENT_EXIT_CODE: i32 = 3;

/// Zap fork:开发模式(DEBUG 源码构建,无 release tag)下,
/// SSH transport 不再从 GitHub 下载陈旧的发行版,而是本地交叉编译
/// 当前 `warp` 二进制并上传。下面这些常量集中描述该交叉编译产物,
/// 与 `script/deploy_remote_server` 保持一致(同 profile / 同 features /
/// 同 target),避免两处分叉。
///
/// 交叉编译目标三元组。
pub const DEV_MUSL_TARGET: &str = "x86_64-unknown-linux-musl";

/// 交叉编译使用的 cargo profile。对应 `Cargo.toml` 的 `[profile.dev-remote]`,
/// 它继承 `dev` 并 strip 符号以减小体积、加快上传。
pub const DEV_REMOTE_PROFILE: &str = "dev-remote";

/// 交叉编译启用的 features,与 `script/deploy_remote_server` 一致。
pub const DEV_REMOTE_FEATURES: &str = "release_bundle,crash_reporting,standalone,agent_mode_debug";

/// 判断当前是否处于「开发模式 remote-server 安装」路径。
///
/// 默认条件:DEBUG 构建(`debug_assertions`)且没有注入 `GIT_RELEASE_TAG`
/// (`app_version().is_none()`,即源码本地构建,非发行版)。这与
/// `remote_server_binary()` / `download_url()` 中对「无 release tag」的
/// 判定保持同一标准。release 构建恒为 `false`,行为完全不变。
///
/// 显式覆盖:设置 `WARP_REMOTE_SERVER_FROM_LOCAL=1` 强制走本地交叉编译路径
/// (`0`/未设视为关闭)。用于 release 构建里临时联调本地 remote-server。
pub fn is_dev_source_build() -> bool {
    if let Some(raw) = std::env::var_os("WARP_REMOTE_SERVER_FROM_LOCAL") {
        let lossy = raw.to_string_lossy();
        let trimmed = lossy.trim();
        let disabled =
            trimmed.is_empty() || trimmed == "0" || trimmed.eq_ignore_ascii_case("false");
        if !disabled {
            return true;
        }
    }
    cfg!(debug_assertions) && ChannelState::app_version().is_none()
}

/// 检查二进制是否存在的超时。
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// 常规远端安装脚本超时。
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);

/// SCP fallback 包含本地下载、上传和远端解压,给它更宽松的超时。
pub const SCP_INSTALL_TIMEOUT: Duration = Duration::from_secs(120);

/// 开发模式交叉编译可能要从头编译整个 crate 图,给它一个很宽松的超时。
pub const DEV_CROSS_COMPILE_TIMEOUT: Duration = Duration::from_secs(900);

/// 开发模式上传本地交叉编译产物的超时。dev 二进制(未优化 + 调试信息)有
/// 数百 MB,即便 scp 开了 `-C` 压缩,跨公网上传也可能要数分钟,因此给一个
/// 远超 `SCP_INSTALL_TIMEOUT` 的宽松上限。
pub const DEV_UPLOAD_TIMEOUT: Duration = Duration::from_secs(1800);

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
