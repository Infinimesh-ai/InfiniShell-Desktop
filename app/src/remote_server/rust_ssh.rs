//! Windows OpenSSH 不提供 ControlMaster 时使用的单连接 Rust SSH transport。
//!
//! session worker 持有唯一的 libssh2 session：一个 channel 承载交互 shell，
//! 其余 channel 由仅监听 loopback 的本地 broker 按需创建。broker capability
//! 只经环境变量和终端私有 hook 传递，不写日志，也不进入子进程命令行。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::fd::{AsRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, RawSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::{Duration, Instant};
#[cfg(windows)]
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use ssh2::{
    CheckResult, ErrorCode, HashType, HostKeyType, KeyboardInteractivePrompt, KnownHostFileKind,
    MethodType, Prompt, Session,
};
use warp_cli::{RustSshBrokerCommandArgs, RustSshSessionArgs};
#[cfg(windows)]
use warp_core::channel::ChannelState;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0};
#[cfg(windows)]
use windows::Win32::Networking::WinSock::{
    IP_TOS, IPPROTO_IP, IPPROTO_IPV6, IPV6_TCLASS, SOCKET, SOCKET_ERROR, WSAGetLastError,
    setsockopt,
};
#[cfg(windows)]
use windows::Win32::System::Console::{
    CONSOLE_MODE, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
    ENABLE_VIRTUAL_TERMINAL_INPUT, GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE, SetConsoleMode,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    CreateEventW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent, WaitForMultipleObjects,
};
#[cfg(windows)]
use windows::core::HSTRING;
use zeroize::Zeroizing;

pub const BROKER_CAPABILITY_ENV: &str = "WARP_SSH_BROKER_CAPABILITY";

const MAX_BROKER_HEADER_BYTES: usize = 1024 * 1024;
const FRAME_STDOUT: u8 = 1;
const FRAME_STDERR: u8 = 2;
const FRAME_EXIT: u8 = 3;
const POLL_INTERVAL: Duration = Duration::from_millis(4);
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(windows)]
const ENHANCED_SSH_OPT_IN_MAX_AGE: Duration = Duration::from_secs(10 * 60);

type SessionGate = Arc<Mutex<()>>;

#[derive(Debug)]
struct EnhancedSshOptInRequired {
    host: String,
    options: Vec<&'static str>,
}

impl std::fmt::Display for EnhancedSshOptInRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "OpenSSH options {} are not supported by enhanced SSH; enable enhanced SSH for this host or set UpdateHostKeys=no and ObscureKeystrokeTiming=no",
            self.options.join(", ")
        )
    }
}

impl std::error::Error for EnhancedSshOptInRequired {}

#[cfg(windows)]
#[derive(Debug, Serialize, Deserialize)]
struct EnhancedSshOptInRequest {
    host: String,
    created_at_unix_seconds: u64,
}

#[cfg(windows)]
#[derive(Debug, Serialize, Deserialize)]
struct EnhancedSshOptInResult {
    applied: bool,
    error: Option<String>,
}

#[cfg(windows)]
struct WindowsEventHandle(HANDLE);

#[cfg(windows)]
impl Drop for WindowsEventHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

mod russh_backend;

/// Windows 的 console stdin 默认是按行、回显并把 Ctrl+C 当本地信号处理。
/// SSH 交互 channel 必须临时切到 VT raw input，退出时再恢复 PowerShell 的
/// 原始模式；如果 stdin 是 pipe，`GetConsoleMode` 会失败，而 pipe 本身已是
/// 字节流，因此无需修改。
#[cfg(windows)]
struct WindowsConsoleInputMode {
    handle: HANDLE,
    original: CONSOLE_MODE,
}

#[cfg(windows)]
impl WindowsConsoleInputMode {
    fn enter() -> Result<Option<Self>> {
        let Ok(handle) = (unsafe { GetStdHandle(STD_INPUT_HANDLE) }) else {
            return Ok(None);
        };
        let mut original = CONSOLE_MODE::default();
        if unsafe { GetConsoleMode(handle, &mut original) }.is_err() {
            return Ok(None);
        }
        let raw = (original & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT))
            | ENABLE_VIRTUAL_TERMINAL_INPUT;
        unsafe { SetConsoleMode(handle, raw) }
            .context("failed to enable raw input for the Rust SSH session")?;
        Ok(Some(Self { handle, original }))
    }
}

#[cfg(windows)]
impl Drop for WindowsConsoleInputMode {
    fn drop(&mut self) {
        unsafe {
            let _ = SetConsoleMode(self.handle, self.original);
        }
    }
}

/// 正式 Windows 安装优先使用控制台 subsystem 的专用 worker。
/// 源码调试和其它平台没有该 sibling 时继续复用当前二进制。
pub fn worker_executable() -> io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    #[cfg(windows)]
    {
        let worker = executable.with_file_name("infinishell-ssh.exe");
        if worker.is_file() {
            return Ok(worker);
        }
        if !cfg!(debug_assertions) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "infinishell-ssh.exe is missing next to the application",
            ));
        }
    }
    Ok(executable)
}

struct PreparedSession {
    session: Session,
    interactive: ssh2::Channel,
    mode: PreparedSessionMode,
    channel_environment: Arc<Vec<(String, String)>>,
}

enum PreparedSessionMode {
    Enhanced {
        listener: TcpListener,
        endpoint: SocketAddr,
        capability: String,
        remote_shell: RemoteShell,
    },
    Plain,
}

#[derive(Debug)]
struct OpenSshConfig {
    hostname: String,
    user: String,
    port: u16,
    strict_host_key_checking: String,
    host_key_alias: Option<String>,
    user_known_hosts_files: Vec<PathBuf>,
    global_known_hosts_files: Vec<PathBuf>,
    identity_files: Vec<PathBuf>,
    identities_only: bool,
    use_agent: bool,
    identity_agent_path: Option<PathBuf>,
    pubkey_authentication: bool,
    keyboard_interactive_authentication: bool,
    password_authentication: bool,
    batch_mode: bool,
    number_of_password_prompts: u32,
    preferred_authentications: Vec<String>,
    kex_algorithms: Option<String>,
    host_key_algorithms: Option<String>,
    pubkey_accepted_algorithms: Option<String>,
    ciphers: Option<String>,
    macs: Option<String>,
    compression: bool,
    proxy_command: Option<String>,
    proxy_jump: Option<String>,
    address_family: String,
    interactive_ip_qos: Option<u8>,
    connect_timeout: Option<Duration>,
    connection_attempts: u32,
    tcp_keep_alive: bool,
    server_alive_interval: Option<Duration>,
    server_alive_count_max: u32,
    escape_char: Option<u8>,
    send_env: Vec<String>,
    set_env: Vec<(String, String)>,
}

/// `ssh -G` 输出中经过审计的字段。这里是能力门禁而不是解析便利列表：
/// OpenSSH 新增字段时必须先判断它是否影响当前会话，并为非中性值增加
/// 实现或显式回退；未知字段不能被静默忽略。
const RECOGNIZED_OPENSSH_CONFIG_KEYS: &[&str] = &[
    "addkeystoagent",
    "addressfamily",
    "applemultipath",
    "batchmode",
    "bindaddress",
    "bindinterface",
    "canonicaldomains",
    "canonicalizefallbacklocal",
    "canonicalizehostname",
    "canonicalizemaxdots",
    "canonicalizepermittedcnames",
    "casignaturealgorithms",
    "certificatefile",
    "channeltimeout",
    "checkhostip",
    "ciphers",
    "clearallforwardings",
    "compression",
    "connectionattempts",
    "connecttimeout",
    "controlmaster",
    "controlpath",
    "controlpersist",
    "dynamicforward",
    "enableescapecommandline",
    "enablesshkeysign",
    "escapechar",
    "exitonforwardfailure",
    "fingerprinthash",
    "forkafterauthentication",
    "forwardagent",
    "forwardx11",
    "forwardx11timeout",
    "forwardx11trusted",
    "gatewayports",
    "globalknownhostsfile",
    "gssapiauthentication",
    "gssapidelegatecredentials",
    "hashknownhosts",
    "host",
    "hostbasedacceptedalgorithms",
    "hostbasedauthentication",
    "hostkeyalias",
    "hostkeyalgorithms",
    "hostname",
    "identitiesonly",
    "identityagent",
    "identityfile",
    "ipqos",
    "kbdinteractiveauthentication",
    "kexalgorithms",
    "knownhostscommand",
    "localcommand",
    "localforward",
    "loglevel",
    "logverbose",
    "macs",
    "nohostauthenticationforlocalhost",
    "nohostauthenticationforproxycommand",
    "numberofpasswordprompts",
    "obscurekeystroketiming",
    "passwordauthentication",
    "permitlocalcommand",
    "permitremoteopen",
    "pkcs11provider",
    "port",
    "preferredauthentications",
    "proxycommand",
    "proxyjump",
    "proxyusefdpass",
    "pubkeyacceptedalgorithms",
    "pubkeyauthentication",
    "rekeylimit",
    "remotecommand",
    "remoteforward",
    "requesttty",
    "requiredrsasize",
    "revokedhostkeys",
    "securitykeyprovider",
    "sendenv",
    "serveralivecountmax",
    "serveraliveinterval",
    "sessiontype",
    "setenv",
    "stdinnull",
    "streamlocalbindmask",
    "streamlocalbindunlink",
    "stricthostkeychecking",
    "syslogfacility",
    "tcpkeepalive",
    "tunnel",
    "tunneldevice",
    "updatehostkeys",
    "user",
    "userknownhostsfile",
    "verifyhostkeydns",
    "visualhostkey",
    "warnweakcrypto",
    "xauthlocation",
];

#[derive(Debug, PartialEq, Eq)]
struct ProxyProcessSpec {
    program: PathBuf,
    args: Vec<OsString>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SshEscapeOutput {
    bytes: Vec<u8>,
    disconnect: bool,
    show_help: bool,
}

struct SshEscapeFilter {
    escape_char: Option<u8>,
    at_line_start: bool,
    pending_escape: bool,
}

impl SshEscapeFilter {
    fn new(escape_char: Option<u8>) -> Self {
        Self {
            escape_char,
            at_line_start: true,
            pending_escape: false,
        }
    }

    fn push(&mut self, input: &[u8]) -> SshEscapeOutput {
        let mut output = SshEscapeOutput::default();
        let Some(escape_char) = self.escape_char else {
            output.bytes.extend_from_slice(input);
            return output;
        };
        for &byte in input {
            if self.pending_escape {
                self.pending_escape = false;
                match byte {
                    b'.' => {
                        output.disconnect = true;
                        break;
                    }
                    b'?' => {
                        output.show_help = true;
                        self.at_line_start = true;
                    }
                    byte if byte == escape_char => {
                        output.bytes.push(byte);
                        self.at_line_start = false;
                    }
                    byte => {
                        output.bytes.push(escape_char);
                        output.bytes.push(byte);
                        self.at_line_start = matches!(byte, b'\r' | b'\n');
                    }
                }
                continue;
            }
            if self.at_line_start && byte == escape_char {
                self.pending_escape = true;
                continue;
            }
            output.bytes.push(byte);
            self.at_line_start = matches!(byte, b'\r' | b'\n');
        }
        output
    }

    fn finish(&mut self) -> Vec<u8> {
        if self.pending_escape {
            self.pending_escape = false;
            self.at_line_start = false;
            self.escape_char.into_iter().collect()
        } else {
            Vec::new()
        }
    }
}

/// libssh2 需要一个双向 socket。ProxyCommand 使用 stdin/stdout 传输 SSH 字节流，
/// 因此用 loopback socketpair 等价物把两者桥接起来，并让 Session 持有其生命周期。
struct ProxyCommandSocket {
    socket: TcpStream,
    child: Child,
}

#[cfg(unix)]
impl AsRawFd for ProxyCommandSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

#[cfg(windows)]
impl AsRawSocket for ProxyCommandSocket {
    fn as_raw_socket(&self) -> RawSocket {
        self.socket.as_raw_socket()
    }
}

impl Drop for ProxyCommandSocket {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteShell {
    Bash,
    Zsh,
    PowerShell,
}

impl RemoteShell {
    fn hook_name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::PowerShell => "pwsh",
        }
    }
}

#[derive(Serialize)]
struct SshHook<'a> {
    hook: &'static str,
    value: SshHookValue<'a>,
}

#[derive(Serialize)]
struct SshHookValue<'a> {
    transport: SshHookTransport<'a>,
    remote_shell: &'static str,
    session_id: u64,
    remote_session_id: u64,
    parent_session_id: u64,
    control_scope: &'a str,
    hop_depth: u32,
    external_control_master: bool,
}

#[derive(Serialize)]
struct SshHookTransport<'a> {
    version: u64,
    #[serde(rename = "type")]
    transport_type: &'static str,
    endpoint: String,
    capability: &'a str,
}

#[derive(Serialize, Deserialize)]
struct BrokerRequest {
    capability: String,
    operation: BrokerOperation,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BrokerOperation {
    Exec {
        command: String,
    },
    Upload {
        remote_path: String,
        size: u64,
        windows: bool,
    },
}

/// 建立单个 SSH session，启动本地 broker，并把交互 channel 映射到 stdio。
pub fn run_session_worker(args: &RustSshSessionArgs) -> Result<i32> {
    let args = resolve_session_worker_args(args)?;
    russh_backend::run_session_worker(&args)
}

fn resolve_session_worker_args(args: &RustSshSessionArgs) -> Result<RustSshSessionArgs> {
    let mut resolved = args.clone();
    if !resolved.commands_base64 {
        return Ok(resolved);
    }

    resolved.posix_command = decode_bootstrap_command(&resolved.posix_command, "POSIX")?;
    resolved.windows_command = decode_bootstrap_command(&resolved.windows_command, "Windows")?;
    resolved.commands_base64 = false;
    Ok(resolved)
}

fn decode_bootstrap_command(encoded: &str, command_name: &str) -> Result<String> {
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        encoded.as_bytes(),
    )
    .with_context(|| format!("invalid base64 {command_name} bootstrap command"))?;
    String::from_utf8(decoded)
        .with_context(|| format!("{command_name} bootstrap command is not valid UTF-8"))
}

fn run_ssh2_session_worker(args: &RustSshSessionArgs) -> Result<i32> {
    let config = match resolve_openssh_config(&args.ssh_executable, &args.ssh_args) {
        Ok(config) => config,
        Err(error) => {
            #[cfg(windows)]
            {
                if let Some(opt_in) = error.downcast_ref::<EnhancedSshOptInRequired>() {
                    match request_enhanced_ssh_opt_in(opt_in) {
                        Ok(true) => {
                            match resolve_openssh_config(&args.ssh_executable, &args.ssh_args) {
                                Ok(config) => config,
                                Err(error) => {
                                    return run_native_ssh_fallback(
                                        args,
                                        "updated OpenSSH configuration",
                                        &error,
                                    );
                                }
                            }
                        }
                        Ok(false) => {
                            return run_native_ssh_fallback(args, "OpenSSH configuration", &error);
                        }
                        Err(opt_in_error) => {
                            eprintln!(
                                "InfiniShell could not offer one-click enhanced SSH setup: {opt_in_error:#}"
                            );
                            return run_native_ssh_fallback(args, "OpenSSH configuration", &error);
                        }
                    }
                } else {
                    return run_native_ssh_fallback(args, "OpenSSH configuration", &error);
                }
            }
            #[cfg(not(windows))]
            {
                return run_native_ssh_fallback(args, "OpenSSH configuration", &error);
            }
        }
    };
    let proxy = match proxy_process_spec(&config, &args.ssh_executable, &args.ssh_args) {
        Ok(proxy) => proxy,
        Err(error) => {
            return run_native_ssh_fallback(args, "proxy configuration", &error);
        }
    };
    #[cfg(windows)]
    if let Err(error) = WindowsConsoleInputMode::enter() {
        return run_native_ssh_fallback(args, "console input", &error);
    }
    let mut prompted = false;
    let session = match connect_session(&config, proxy, &mut prompted) {
        Ok(session) => session,
        Err(error) if !prompted => {
            return run_native_ssh_fallback(args, "connection setup", &error);
        }
        Err(_) => return Ok(255),
    };
    if let Err(error) = authenticate_session(&session, &config, &mut prompted) {
        if !prompted {
            return run_native_ssh_fallback(args, "authentication", &error);
        }
        return Ok(255);
    }
    let prepared = prepare_authenticated_session(args, session, &config)?;
    let PreparedSession {
        session,
        interactive,
        mode,
        channel_environment,
    } = prepared;

    if let Some(interval) = config.server_alive_interval {
        session.set_keepalive(
            config.server_alive_count_max > 0,
            interval.as_secs().min(u64::from(u32::MAX)) as u32,
        );
    }
    #[cfg(windows)]
    let _console_input_mode = WindowsConsoleInputMode::enter()?;
    session.set_blocking(false);
    let session_gate = Arc::new(Mutex::new(()));
    let running = Arc::new(AtomicBool::new(true));
    let broker_thread = match mode {
        PreparedSessionMode::Enhanced {
            listener,
            endpoint,
            capability,
            remote_shell,
        } => {
            emit_ssh_hook(
                endpoint,
                &capability,
                remote_shell,
                args.session_id,
                args.remote_session_id,
                &args.control_scope,
                args.hop_depth,
            )?;
            Some(spawn_broker(
                listener,
                session.clone(),
                session_gate.clone(),
                capability,
                running.clone(),
                channel_environment,
            ))
        }
        PreparedSessionMode::Plain => None,
    };
    let result = bridge_interactive_channel(
        interactive,
        &session,
        &session_gate,
        config.server_alive_interval,
        config.server_alive_count_max,
        config.escape_char,
    );
    running.store(false, Ordering::Release);
    if let Some(broker_thread) = broker_thread {
        let _ = broker_thread.join();
    }
    match result {
        Ok(exit_code) => Ok(exit_code),
        Err(_) => {
            eprintln!("InfiniShell Rust SSH transport ended unexpectedly.");
            Ok(255)
        }
    }
}

fn prepare_authenticated_session(
    args: &RustSshSessionArgs,
    session: Session,
    config: &OpenSshConfig,
) -> Result<PreparedSession> {
    let channel_environment = Arc::new(resolve_channel_environment(&config));
    let enhanced = detect_remote_shell(&session).ok().and_then(|remote_shell| {
        let remote_command = match remote_shell {
            RemoteShell::Bash | RemoteShell::Zsh => &args.posix_command,
            RemoteShell::PowerShell => &args.windows_command,
        };
        open_interactive_channel(&session, &channel_environment, Some(remote_command))
            .ok()
            .map(|interactive| (remote_shell, interactive))
    });
    let (interactive, mode) = match enhanced {
        Some((remote_shell, interactive)) => {
            let mode = TcpListener::bind(("127.0.0.1", 0))
                .and_then(|listener| {
                    listener.set_nonblocking(true)?;
                    let endpoint = listener.local_addr()?;
                    Ok((listener, endpoint))
                })
                .map_or(PreparedSessionMode::Plain, |(listener, endpoint)| {
                    PreparedSessionMode::Enhanced {
                        listener,
                        endpoint,
                        capability: new_capability(),
                        remote_shell,
                    }
                });
            (interactive, mode)
        }
        None => (
            open_interactive_channel(&session, &channel_environment, None)?,
            PreparedSessionMode::Plain,
        ),
    };

    Ok(PreparedSession {
        session,
        interactive,
        mode,
        channel_environment,
    })
}

fn open_interactive_channel(
    session: &Session,
    channel_environment: &[(String, String)],
    command: Option<&str>,
) -> Result<ssh2::Channel> {
    let mut channel = session.channel_session()?;
    apply_channel_environment(&mut channel, channel_environment);
    let (columns, rows) = terminal_dimensions();
    channel.request_pty("xterm-256color", None, Some((columns, rows, 0, 0)))?;
    match command {
        Some(command) => channel.exec(command)?,
        None => channel.shell()?,
    }
    Ok(channel)
}

fn run_native_ssh(args: &RustSshSessionArgs) -> Result<i32> {
    let mut command = command::blocking::Command::new(&args.ssh_executable);
    #[cfg(windows)]
    command.inherit_console();
    let status = command
        .args(&args.ssh_args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to start native OpenSSH fallback")?;
    Ok(status.code().unwrap_or(255))
}

fn run_native_ssh_fallback(
    args: &RustSshSessionArgs,
    stage: &str,
    error: &anyhow::Error,
) -> Result<i32> {
    eprintln!("InfiniShell enhanced SSH is unavailable during {stage}: {error:#}");
    eprintln!(
        "Continuing with native OpenSSH; InfiniShell SSH extension features are unavailable for this session."
    );
    run_native_ssh(args)
}

#[cfg(windows)]
fn request_enhanced_ssh_opt_in(opt_in: &EnhancedSshOptInRequired) -> Result<bool> {
    validate_ssh_config_host(&opt_in.host)?;

    let stdin_handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) }
        .context("failed to access console input for enhanced SSH setup")?;
    let mut console_mode = CONSOLE_MODE::default();
    if unsafe { GetConsoleMode(stdin_handle, &mut console_mode) }.is_err() {
        return Ok(false);
    }

    let mut token_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut token_bytes);
    let token = token_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let event_name = enhanced_ssh_opt_in_event_name(&token);
    let event_name_wide = HSTRING::from(&event_name);
    let event = WindowsEventHandle(
        unsafe { CreateEventW(None, true, false, &event_name_wide) }
            .context("failed to create enhanced SSH setup event")?,
    );
    let request_path = enhanced_ssh_opt_in_request_path(&token);
    let result_path = enhanced_ssh_opt_in_result_path(&token);
    let request = EnhancedSshOptInRequest {
        host: opt_in.host.clone(),
        created_at_unix_seconds: current_unix_seconds()?,
    };
    let request_bytes = serde_json::to_vec(&request)?;
    let request_directory = request_path
        .parent()
        .context("enhanced SSH setup request path has no parent")?;
    std::fs::create_dir_all(request_directory)
        .context("failed to create enhanced SSH setup directory")?;
    let mut request_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&request_path)
        .context("failed to create enhanced SSH setup request")?;
    request_file
        .write_all(&request_bytes)
        .context("failed to write enhanced SSH setup request")?;
    drop(request_file);

    let uri = format!(
        "{}://ssh/enable-enhanced?request={token}",
        ChannelState::url_scheme()
    );
    let link = format!(
        "\x1b]8;;{uri}\x1b\\Enable enhanced SSH for {}\x1b]8;;\x1b\\",
        opt_in.host
    );
    eprintln!(
        "InfiniShell enhanced SSH requires per-host OpenSSH settings because {} are enabled.",
        opt_in.options.join(", ")
    );
    eprintln!(
        "This disables automatic host-key updates and keystroke-timing obfuscation for this host."
    );
    eprintln!(
        "{link}, or press Enter to continue with native OpenSSH without SSH extension features."
    );

    let wait_result = unsafe { WaitForMultipleObjects(&[event.0, stdin_handle], false, INFINITE) };
    let opted_in = if wait_result == WAIT_OBJECT_0 {
        let result_bytes =
            std::fs::read(&result_path).context("failed to read enhanced SSH setup result")?;
        let result: EnhancedSshOptInResult = serde_json::from_slice(&result_bytes)
            .context("failed to parse enhanced SSH setup result")?;
        if result.applied {
            eprintln!("InfiniShell enhanced SSH is enabled for this host; connecting now.");
            true
        } else {
            eprintln!(
                "InfiniShell could not update the OpenSSH configuration: {}",
                result.error.as_deref().unwrap_or("unknown error")
            );
            false
        }
    } else if wait_result.0 == WAIT_OBJECT_0.0 + 1 {
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .context("failed to read enhanced SSH setup choice")?;
        false
    } else if wait_result == WAIT_FAILED {
        bail!("failed to wait for enhanced SSH setup choice");
    } else {
        bail!("unexpected enhanced SSH setup wait result");
    };

    let _ = std::fs::remove_file(request_path);
    let _ = std::fs::remove_file(result_path);
    Ok(opted_in)
}

#[cfg(windows)]
fn enhanced_ssh_opt_in_directory() -> PathBuf {
    std::env::temp_dir()
        .join("infinishell-ssh")
        .join("enhanced-opt-in")
}

#[cfg(windows)]
fn enhanced_ssh_opt_in_request_path(token: &str) -> PathBuf {
    enhanced_ssh_opt_in_directory().join(format!("{token}.request.json"))
}

#[cfg(windows)]
fn enhanced_ssh_opt_in_result_path(token: &str) -> PathBuf {
    enhanced_ssh_opt_in_directory().join(format!("{token}.result.json"))
}

#[cfg(windows)]
fn enhanced_ssh_opt_in_event_name(token: &str) -> String {
    format!("Local\\InfiniShellSshOptIn-{token}")
}

#[cfg(windows)]
fn current_unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs())
}

/// 处理用户从终端点击的一次性增强 SSH 配置请求。
#[cfg(windows)]
pub fn approve_enhanced_ssh_opt_in(token: &str) -> Result<String> {
    let config_path = default_ssh_config_path()?;
    approve_enhanced_ssh_opt_in_at_path(token, &config_path)
}

#[cfg(windows)]
fn approve_enhanced_ssh_opt_in_at_path(token: &str, config_path: &Path) -> Result<String> {
    validate_enhanced_ssh_opt_in_token(token)?;
    let request_path = enhanced_ssh_opt_in_request_path(token);
    let request_bytes =
        std::fs::read(&request_path).context("enhanced SSH setup request is missing or expired")?;
    let request: EnhancedSshOptInRequest =
        serde_json::from_slice(&request_bytes).context("enhanced SSH setup request is invalid")?;
    validate_ssh_config_host(&request.host)?;
    let now = current_unix_seconds()?;
    if request.created_at_unix_seconds > now
        || now - request.created_at_unix_seconds > ENHANCED_SSH_OPT_IN_MAX_AGE.as_secs()
    {
        bail!("enhanced SSH setup request is expired");
    }

    let apply_result = upsert_enhanced_ssh_host_config(config_path, &request.host);
    let result = EnhancedSshOptInResult {
        applied: apply_result.is_ok(),
        error: apply_result.as_ref().err().map(ToString::to_string),
    };
    let result_bytes = serde_json::to_vec(&result)?;
    let write_result = std::fs::write(enhanced_ssh_opt_in_result_path(token), result_bytes)
        .context("failed to write enhanced SSH setup result");
    let signal_result = signal_enhanced_ssh_opt_in(token);

    write_result?;
    signal_result?;
    apply_result?;
    Ok(request.host)
}

#[cfg(windows)]
fn signal_enhanced_ssh_opt_in(token: &str) -> Result<()> {
    let event_name = enhanced_ssh_opt_in_event_name(token);
    let event_name_wide = HSTRING::from(&event_name);
    let event = WindowsEventHandle(
        unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &event_name_wide) }
            .context("enhanced SSH setup request is no longer active")?,
    );
    unsafe { SetEvent(event.0) }.context("failed to notify the SSH worker")
}

#[cfg(windows)]
fn validate_enhanced_ssh_opt_in_token(token: &str) -> Result<()> {
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid enhanced SSH setup request identifier");
    }
    Ok(())
}

#[cfg(windows)]
fn default_ssh_config_path() -> Result<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .context("home directory is unavailable")?;
    Ok(PathBuf::from(home).join(".ssh").join("config"))
}

#[cfg(any(windows, test))]
fn validate_ssh_config_host(host: &str) -> Result<()> {
    if host.is_empty()
        || host.len() > 1024
        || host.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '*' | '?' | '!' | '#')
        })
    {
        bail!("OpenSSH host is not safe for an exact per-host configuration block");
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn upsert_enhanced_ssh_host_config(config_path: &Path, host: &str) -> Result<()> {
    validate_ssh_config_host(host)?;
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).context("failed to read OpenSSH user configuration"),
    };
    let (bom, body) = contents
        .strip_prefix('\u{feff}')
        .map_or(("", contents.as_str()), |body| ("\u{feff}", body));
    let line_ending = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let begin_marker = format!("# >>> InfiniShell enhanced SSH for {host}");
    let end_marker = format!("# <<< InfiniShell enhanced SSH for {host}");
    let mut preserved = body.to_string();
    if let Some(start) = preserved.find(&begin_marker) {
        let marker_end = preserved[start..]
            .find(&end_marker)
            .map(|offset| start + offset + end_marker.len())
            .context("managed InfiniShell SSH configuration block is incomplete")?;
        let mut end = marker_end;
        for _ in 0..2 {
            if preserved[end..].starts_with("\r\n") {
                end += 2;
            } else if preserved[end..].starts_with('\n') {
                end += 1;
            }
        }
        preserved.replace_range(start..end, "");
    }
    let block = format!(
        "{begin_marker}{line_ending}Host {host}{line_ending}    UpdateHostKeys no{line_ending}    ObscureKeystrokeTiming no{line_ending}Host *{line_ending}{end_marker}{line_ending}{line_ending}"
    );
    let updated = format!("{bom}{block}{preserved}");

    let directory = config_path
        .parent()
        .context("OpenSSH user configuration path has no parent")?;
    std::fs::create_dir_all(directory)
        .context("failed to create the OpenSSH user configuration directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .context("failed to create a temporary OpenSSH user configuration")?;
    temporary
        .write_all(updated.as_bytes())
        .context("failed to write the OpenSSH user configuration")?;
    temporary
        .as_file()
        .sync_all()
        .context("failed to flush the OpenSSH user configuration")?;
    temporary
        .persist(config_path)
        .map_err(|error| error.error)
        .context("failed to replace the OpenSSH user configuration")?;
    Ok(())
}

/// 连接本机 broker，把一个远端 exec channel 映射为本进程 stdio。
pub fn run_broker_command(args: &RustSshBrokerCommandArgs) -> Result<i32> {
    if let (None, Some(control_path)) = (&args.endpoint, &args.control_path) {
        return run_control_master_upload(args, control_path);
    }
    if args.endpoint.is_none() || args.control_path.is_some() {
        bail!("SSH broker command requires exactly one transport");
    }
    let capability =
        std::env::var(BROKER_CAPABILITY_ENV).context("missing SSH broker capability")?;
    run_broker_command_with_capability(args, capability)
}

fn run_control_master_upload(args: &RustSshBrokerCommandArgs, control_path: &Path) -> Result<i32> {
    let BrokerOperation::Upload {
        remote_path,
        size,
        windows,
    } = broker_operation(args, None)?
    else {
        bail!("SSH ControlMaster worker only supports uploads");
    };
    let stdin = io::stdin();
    let archive = stage_control_upload(stdin.lock(), size)?;
    futures_lite::future::block_on(remote_server::ssh::scp_upload(
        control_path,
        archive.path(),
        control_master_upload_path(&remote_path, windows),
        remote_server::setup::SCP_INSTALL_TIMEOUT,
    ))?;
    Ok(0)
}

pub(crate) fn control_master_upload_path(remote_path: &str, windows: bool) -> &str {
    if windows {
        remote_path
            .strip_prefix("~/")
            .or_else(|| remote_path.strip_prefix("~\\"))
            .unwrap_or(remote_path)
    } else {
        remote_path
    }
}

fn stage_control_upload(mut reader: impl Read, size: u64) -> Result<tempfile::NamedTempFile> {
    let mut archive = tempfile::NamedTempFile::new()
        .context("failed to create SSH ControlMaster upload staging file")?;
    let copied = io::copy(&mut reader.by_ref().take(size), archive.as_file_mut())
        .context("failed to stage SSH ControlMaster upload")?;
    if copied != size {
        bail!("SSH ControlMaster upload ended before the declared file size");
    }
    archive
        .as_file_mut()
        .flush()
        .context("failed to flush SSH ControlMaster upload staging file")?;
    Ok(archive)
}

fn run_broker_command_with_capability(
    args: &RustSshBrokerCommandArgs,
    capability: String,
) -> Result<i32> {
    let mut stdin_file = args
        .stdin_file
        .as_ref()
        .map(std::fs::File::open)
        .transpose()
        .context("failed to open SSH broker stdin file")?;
    let endpoint: SocketAddr = args
        .endpoint
        .as_deref()
        .context("missing SSH broker endpoint")?
        .parse()
        .context("invalid SSH broker endpoint")?;
    if !endpoint.ip().is_loopback() {
        bail!("SSH broker endpoint is not loopback");
    }
    let operation = broker_operation(args, stdin_file.as_ref())?;
    let wait_for_input = matches!(&operation, BrokerOperation::Upload { .. });
    let mut stream = TcpStream::connect_timeout(&endpoint, CONNECT_TIMEOUT)?;
    stream.set_nodelay(true)?;
    write_header(
        &mut stream,
        &BrokerRequest {
            capability,
            operation,
        },
    )?;

    let mut ack = [0_u8; 1];
    stream.read_exact(&mut ack)?;
    if ack[0] != 0 {
        bail!("SSH broker rejected the command");
    }

    // 上传是有限输入，收到远端退出帧前必须确认声明的内容已完整发送。普通 exec
    // 继承的 stdin 可能仍阻塞等待用户输入；远端命令结束后不能 join 该线程，
    // 否则一个不读取 stdin 的命令会让 broker 永远无法退出。
    let mut input = stream.try_clone()?;
    let mut input_thread = Some(thread::spawn(move || {
        let result = match stdin_file.as_mut() {
            Some(file) => io::copy(file, &mut input),
            None => {
                let mut stdin = io::stdin().lock();
                io::copy(&mut stdin, &mut input)
            }
        };
        let _ = input.shutdown(Shutdown::Write);
        result
    }));

    loop {
        let mut kind = [0_u8; 1];
        stream.read_exact(&mut kind)?;
        let length = read_u32(&mut stream)? as usize;
        if length > MAX_BROKER_HEADER_BYTES {
            bail!("SSH broker frame is too large");
        }
        let mut payload = vec![0_u8; length];
        stream.read_exact(&mut payload)?;
        match kind[0] {
            FRAME_STDOUT => {
                let mut stdout = io::stdout().lock();
                stdout.write_all(&payload)?;
                stdout.flush()?;
            }
            FRAME_STDERR => {
                let mut stderr = io::stderr().lock();
                stderr.write_all(&payload)?;
                stderr.flush()?;
            }
            FRAME_EXIT if payload.len() == 4 => {
                finish_broker_input(&mut input_thread, wait_for_input)?;
                return Ok(i32::from_be_bytes(payload.try_into().unwrap()));
            }
            FRAME_EXIT => bail!("invalid SSH broker exit frame"),
            _ => bail!("unknown SSH broker frame"),
        }
    }
}

fn broker_operation(
    args: &RustSshBrokerCommandArgs,
    stdin_file: Option<&std::fs::File>,
) -> Result<BrokerOperation> {
    match (&args.command, &args.upload_path) {
        (Some(command), None) => Ok(BrokerOperation::Exec {
            command: command.clone(),
        }),
        (None, Some(remote_path)) => {
            let size = match (stdin_file, args.stdin_size) {
                (Some(file), None) => file
                    .metadata()
                    .context("failed to read SSH broker upload metadata")?
                    .len(),
                (None, Some(size @ 1..)) => size,
                (Some(_), Some(_)) | (None, None) | (None, Some(0)) => {
                    bail!("SSH broker upload requires exactly one non-empty input source")
                }
            };
            Ok(BrokerOperation::Upload {
                remote_path: remote_path.clone(),
                size,
                windows: args.upload_windows,
            })
        }
        (Some(_), Some(_)) | (None, None) => {
            bail!("SSH broker requires exactly one operation")
        }
    }
}

fn finish_broker_input(
    input_thread: &mut Option<thread::JoinHandle<io::Result<u64>>>,
    wait_for_input: bool,
) -> Result<()> {
    if !wait_for_input {
        return Ok(());
    }
    input_thread
        .take()
        .expect("SSH broker input thread should exist")
        .join()
        .map_err(|_| anyhow!("SSH broker input thread panicked"))?
        .context("failed to forward SSH broker input")?;
    Ok(())
}

fn resolve_openssh_config(ssh_executable: &Path, ssh_args: &[OsString]) -> Result<OpenSshConfig> {
    let output = command::blocking::Command::new(ssh_executable)
        .arg("-G")
        .args(ssh_args)
        .stdin(Stdio::null())
        .output()
        .context("failed to resolve OpenSSH configuration")?;
    if !output.status.success() {
        bail!("ssh -G failed");
    }

    let stdout = String::from_utf8(output.stdout).context("ssh -G returned non-UTF-8 data")?;
    parse_openssh_config(&stdout)
}

fn parse_openssh_config(stdout: &str) -> Result<OpenSshConfig> {
    let mut single = HashMap::<String, String>::new();
    let mut multiple = HashMap::<String, Vec<String>>::new();
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let key = key.to_ascii_lowercase();
        let value = value.trim().to_string();
        multiple.entry(key.clone()).or_default().push(value.clone());
        single.entry(key).or_insert(value);
    }

    for key in single.keys() {
        if !RECOGNIZED_OPENSSH_CONFIG_KEYS.contains(&key.as_str()) {
            bail!("OpenSSH option {key} is not supported by enhanced SSH");
        }
    }

    for unsupported in [
        "remotecommand",
        "localcommand",
        "knownhostscommand",
        "pkcs11provider",
        "revokedhostkeys",
    ] {
        if single
            .get(unsupported)
            .is_some_and(|value| !value.eq_ignore_ascii_case("none"))
        {
            bail!("OpenSSH option {unsupported} is not supported by enhanced SSH");
        }
    }
    for unsupported_boolean in [
        "gssapiauthentication",
        "gssapidelegatecredentials",
        "hostbasedauthentication",
        "checkhostip",
        "enablesshkeysign",
        "enableescapecommandline",
        "forwardagent",
        "forwardx11",
        "forwardx11trusted",
        "nohostauthenticationforlocalhost",
        "nohostauthenticationforproxycommand",
        "permitlocalcommand",
        "proxyusefdpass",
        "verifyhostkeydns",
        "visualhostkey",
    ] {
        if single
            .get(unsupported_boolean)
            .is_some_and(|value| value.eq_ignore_ascii_case("yes"))
        {
            bail!("OpenSSH option {unsupported_boolean}=yes is not supported by enhanced SSH");
        }
    }
    for unsupported in ["dynamicforward", "localforward", "remoteforward"] {
        if multiple
            .get(unsupported)
            .is_some_and(|values| !values.is_empty())
        {
            bail!("OpenSSH option {unsupported} is not supported by enhanced SSH");
        }
    }
    if multiple.get("certificatefile").is_some_and(|values| {
        values
            .iter()
            .any(|value| !value.eq_ignore_ascii_case("none"))
    }) {
        bail!("OpenSSH certificates require the native SSH fallback");
    }
    if single
        .get("stricthostkeychecking")
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "no" | "off"))
    {
        // OpenSSH 对变更主机密钥的 `no/off` 处理还会禁用密码认证和转发；
        // Rust transport 不能只忽略 mismatch，否则会改变安全语义。
        bail!("StrictHostKeyChecking=no requires the native SSH fallback");
    }
    if single
        .get("addkeystoagent")
        .is_some_and(|value| !matches!(value.to_ascii_lowercase().as_str(), "no" | "false"))
    {
        bail!("AddKeysToAgent requires the native SSH fallback");
    }
    if single
        .get("fingerprinthash")
        .is_some_and(|value| !value.eq_ignore_ascii_case("sha256"))
    {
        bail!("FingerprintHash requires the native SSH fallback");
    }
    if single
        .get("hashknownhosts")
        .is_some_and(|value| value.eq_ignore_ascii_case("yes"))
    {
        // 不能把用户要求散列保存的主机名降级成明文 known_hosts 条目。
        bail!("HashKnownHosts requires the native SSH fallback");
    }
    if single
        .get("requiredrsasize")
        .is_some_and(|value| value != "1024")
    {
        bail!("RequiredRSASize requires the native SSH fallback");
    }
    let identity_files = config_single_paths(&multiple, "identityfile");
    if let Some(provider) = single.get("securitykeyprovider") {
        let provider_is_neutral = provider == "$SSH_SK_PROVIDER"
            || provider.eq_ignore_ascii_case("none")
            || (provider.eq_ignore_ascii_case("internal")
                && !identity_files
                    .iter()
                    .any(|path| security_key_identity(path)));
        if !provider_is_neutral {
            bail!("OpenSSH option securitykeyprovider is not supported by enhanced SSH");
        }
    }
    for (key, allowed) in [
        ("bindaddress", &["*"][..]),
        ("bindinterface", &["none"][..]),
        ("controlmaster", &["false", "no"][..]),
        ("requesttty", &["auto", "yes", "force"][..]),
        ("sessiontype", &["default"][..]),
        ("stdinnull", &["no"][..]),
        ("forkafterauthentication", &["no"][..]),
        ("tunnel", &["false", "no"][..]),
        ("channeltimeout", &["none"][..]),
        ("loglevel", &["INFO"][..]),
        ("logverbose", &["none"][..]),
        ("rekeylimit", &["0 0"][..]),
        ("canonicalizehostname", &["false", "no"][..]),
        ("controlpath", &["none"][..]),
        ("controlpersist", &["false", "no", "0"][..]),
        ("warnweakcrypto", &["false", "no"][..]),
    ] {
        if single.get(key).is_some_and(|value| {
            !allowed
                .iter()
                .any(|allowed| value.eq_ignore_ascii_case(allowed))
        }) {
            bail!("OpenSSH option {key} is not supported by enhanced SSH");
        }
    }

    let hostname = required_config(&single, "hostname")?.to_string();
    let user = required_config(&single, "user")?.to_string();
    let port = required_config(&single, "port")?
        .parse::<u16>()
        .context("invalid SSH port")?;
    let strict_host_key_checking = single
        .get("stricthostkeychecking")
        .cloned()
        .unwrap_or_else(|| "ask".to_string());
    let host_key_alias = single
        .get("hostkeyalias")
        .filter(|value| !value.eq_ignore_ascii_case("none"))
        .cloned();
    let (use_agent, identity_agent_path) = match single.get("identityagent") {
        None => (true, None),
        Some(value) if value.eq_ignore_ascii_case("none") => (false, None),
        Some(value)
            if value == "$SSH_AUTH_SOCK"
                || std::env::var("SSH_AUTH_SOCK").ok().as_deref() == Some(value.as_str()) =>
        {
            (true, None)
        }
        Some(value) => (true, Some(expand_home(value))),
    };

    let connect_timeout = match single.get("connecttimeout").map(String::as_str) {
        None | Some("none") | Some("0") => None,
        Some(value) => Some(Duration::from_secs(
            value.parse::<u64>().context("invalid ConnectTimeout")?,
        )),
    };
    let connection_attempts = single
        .get("connectionattempts")
        .map(String::as_str)
        .unwrap_or("1")
        .parse::<u32>()
        .context("invalid ConnectionAttempts")?
        .max(1);
    let server_alive_interval = match single.get("serveraliveinterval").map(String::as_str) {
        None | Some("0") => None,
        Some(value) => Some(Duration::from_secs(
            value
                .parse::<u64>()
                .context("invalid ServerAliveInterval")?,
        )),
    };
    let server_alive_count_max = single
        .get("serveralivecountmax")
        .map(String::as_str)
        .unwrap_or("3")
        .parse::<u32>()
        .context("invalid ServerAliveCountMax")?;
    let preferred_authentications = single
        .get("preferredauthentications")
        .map(|value| {
            value
                .split(',')
                .map(|method| method.trim().to_ascii_lowercase())
                .filter(|method| !method.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if preferred_authentications.iter().any(|method| {
        !matches!(
            method.as_str(),
            "publickey" | "keyboard-interactive" | "password"
        )
    }) {
        bail!("PreferredAuthentications requires the native SSH fallback");
    }
    let set_env = multiple
        .get("setenv")
        .into_iter()
        .flatten()
        .filter_map(|value| value.split_once('='))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    let interactive_ip_qos = single
        .get("ipqos")
        .map(|value| parse_ip_qos(value))
        .transpose()?
        .flatten();
    let escape_char = single
        .get("escapechar")
        .map(|value| parse_escape_char(value))
        .transpose()?
        .unwrap_or(Some(b'~'));
    let opt_in_options = ["updatehostkeys", "obscurekeystroketiming"]
        .into_iter()
        .filter(|key| {
            single
                .get(*key)
                .is_some_and(|value| !matches!(value.to_ascii_lowercase().as_str(), "false" | "no"))
        })
        .collect::<Vec<_>>();
    if !opt_in_options.is_empty() {
        return Err(EnhancedSshOptInRequired {
            host: required_config(&single, "host")?.to_string(),
            options: opt_in_options,
        }
        .into());
    }

    Ok(OpenSshConfig {
        hostname,
        user,
        port,
        strict_host_key_checking,
        host_key_alias,
        user_known_hosts_files: config_paths(&multiple, "userknownhostsfile"),
        global_known_hosts_files: config_paths(&multiple, "globalknownhostsfile"),
        identity_files,
        identities_only: single
            .get("identitiesonly")
            .is_some_and(|value| value.eq_ignore_ascii_case("yes")),
        use_agent,
        identity_agent_path,
        pubkey_authentication: single.get("pubkeyauthentication").is_none_or(|value| {
            !value.eq_ignore_ascii_case("no") && !value.eq_ignore_ascii_case("false")
        }),
        keyboard_interactive_authentication: single
            .get("kbdinteractiveauthentication")
            .is_none_or(|value| value.eq_ignore_ascii_case("yes")),
        password_authentication: single
            .get("passwordauthentication")
            .is_none_or(|value| value.eq_ignore_ascii_case("yes")),
        batch_mode: single
            .get("batchmode")
            .is_some_and(|value| value.eq_ignore_ascii_case("yes")),
        number_of_password_prompts: single
            .get("numberofpasswordprompts")
            .map(String::as_str)
            .unwrap_or("3")
            .parse::<u32>()
            .context("invalid NumberOfPasswordPrompts")?,
        preferred_authentications,
        kex_algorithms: single.get("kexalgorithms").cloned(),
        host_key_algorithms: single.get("hostkeyalgorithms").cloned(),
        pubkey_accepted_algorithms: single.get("pubkeyacceptedalgorithms").cloned(),
        ciphers: single.get("ciphers").cloned(),
        macs: single.get("macs").cloned(),
        compression: single
            .get("compression")
            .is_some_and(|value| value.eq_ignore_ascii_case("yes")),
        proxy_command: optional_config(&single, "proxycommand"),
        proxy_jump: optional_config(&single, "proxyjump"),
        address_family: single
            .get("addressfamily")
            .cloned()
            .unwrap_or_else(|| "any".to_string()),
        interactive_ip_qos,
        connect_timeout,
        connection_attempts,
        tcp_keep_alive: single
            .get("tcpkeepalive")
            .is_none_or(|value| value.eq_ignore_ascii_case("yes")),
        server_alive_interval,
        server_alive_count_max,
        escape_char,
        send_env: multiple
            .get("sendenv")
            .into_iter()
            .flatten()
            .flat_map(|value| value.split_whitespace())
            .map(str::to_string)
            .collect(),
        set_env,
    })
}

fn parse_escape_char(value: &str) -> Result<Option<u8>> {
    if value.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let bytes = value.as_bytes();
    match bytes {
        [byte] if byte.is_ascii() => Ok(Some(*byte)),
        [b'^', b'?'] => Ok(Some(0x7f)),
        [b'^', control] if control.is_ascii() => {
            let control = control.to_ascii_uppercase();
            if (b'@'..=b'_').contains(&control) {
                Ok(Some(control & 0x1f))
            } else {
                bail!("EscapeChar requires the native SSH fallback")
            }
        }
        _ => bail!("EscapeChar requires the native SSH fallback"),
    }
}

fn parse_ip_qos(value: &str) -> Result<Option<u8>> {
    let values = value.split_whitespace().collect::<Vec<_>>();
    if !(1..=2).contains(&values.len()) {
        bail!("IPQoS requires the native SSH fallback");
    }
    let interactive = parse_ip_qos_value(values[0])?;
    if let Some(non_interactive) = values.get(1) {
        parse_ip_qos_value(non_interactive)?;
    }
    Ok(interactive)
}

fn parse_ip_qos_value(value: &str) -> Result<Option<u8>> {
    let value = value.to_ascii_lowercase();
    if value == "none" {
        return Ok(None);
    }
    let dscp = match value.as_str() {
        "af11" => 10,
        "af12" => 12,
        "af13" => 14,
        "af21" => 18,
        "af22" => 20,
        "af23" => 22,
        "af31" => 26,
        "af32" => 28,
        "af33" => 30,
        "af41" => 34,
        "af42" => 36,
        "af43" => 38,
        "cs0" => 0,
        "cs1" => 8,
        "cs2" => 16,
        "cs3" => 24,
        "cs4" => 32,
        "cs5" => 40,
        "cs6" => 48,
        "cs7" => 56,
        "ef" => 46,
        "le" => 1,
        // 旧版 OpenSSH 接受的 IPv4 ToS 名称；换算成等价 DSCP。
        "lowdelay" => 4,
        "throughput" => 2,
        "reliability" => 1,
        numeric => numeric
            .parse::<u8>()
            .ok()
            .filter(|value| *value <= 63)
            .context("IPQoS requires the native SSH fallback")?,
    };
    Ok(Some(dscp << 2))
}

fn optional_config(config: &HashMap<String, String>, key: &str) -> Option<String> {
    config
        .get(key)
        .filter(|value| !value.eq_ignore_ascii_case("none"))
        .cloned()
}

fn required_config<'a>(config: &'a HashMap<String, String>, key: &str) -> Result<&'a str> {
    config
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| anyhow!("ssh -G omitted {key}"))
}

fn config_paths(config: &HashMap<String, Vec<String>>, key: &str) -> Vec<PathBuf> {
    config
        .get(key)
        .into_iter()
        .flatten()
        .flat_map(|value| value.split_whitespace())
        .filter(|value| !value.eq_ignore_ascii_case("none"))
        .map(expand_home)
        .collect()
}

fn config_single_paths(config: &HashMap<String, Vec<String>>, key: &str) -> Vec<PathBuf> {
    config
        .get(key)
        .into_iter()
        .flatten()
        .filter(|value| !value.eq_ignore_ascii_case("none"))
        .map(|value| expand_home(value))
        .collect()
}

/// `SecurityKeyProvider=internal` 是 Windows OpenSSH 9.5 的默认值；只有当前主机
/// 确实配置了本地 SK/FIDO identity 时，它才会影响认证语义。默认但不存在的
/// `id_ecdsa_sk` / `id_ed25519_sk` 路径不能阻止普通密钥、agent 或密码认证。
fn security_key_identity(path: &Path) -> bool {
    let mut public_key_path = path.as_os_str().to_owned();
    public_key_path.push(".pub");
    if file_uses_security_key_algorithm(Path::new(&public_key_path))
        || file_uses_security_key_algorithm(path)
    {
        return true;
    }
    path.is_file()
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.to_ascii_lowercase().ends_with("_sk"))
}

fn file_uses_security_key_algorithm(path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    contents.lines().any(|line| {
        matches!(
            line.trim_start().split_whitespace().next(),
            Some("sk-ssh-ed25519@openssh.com" | "sk-ecdsa-sha2-nistp256@openssh.com")
        )
    })
}

fn expand_home(value: &str) -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    match (
        value
            .strip_prefix("~/")
            .or_else(|| value.strip_prefix("~\\")),
        home,
    ) {
        (Some(relative), Some(home)) => PathBuf::from(home).join(relative),
        _ => PathBuf::from(value),
    }
}

fn resolve_channel_environment(config: &OpenSshConfig) -> Vec<(String, String)> {
    let mut environment = BTreeMap::new();
    for (name, value) in std::env::vars() {
        if config
            .send_env
            .iter()
            .any(|pattern| wildcard_matches(pattern.as_bytes(), name.as_bytes()))
        {
            environment.insert(name, value);
        }
    }
    for (name, value) in &config.set_env {
        environment.insert(name.clone(), value.clone());
    }
    environment.into_iter().collect()
}

fn wildcard_matches(pattern: &[u8], value: &[u8]) -> bool {
    match pattern.split_first() {
        None => value.is_empty(),
        Some((&b'*', rest)) => {
            wildcard_matches(rest, value)
                || value
                    .split_first()
                    .is_some_and(|(_, value_rest)| wildcard_matches(pattern, value_rest))
        }
        Some((&b'?', rest)) => value
            .split_first()
            .is_some_and(|(_, value_rest)| wildcard_matches(rest, value_rest)),
        Some((&expected, rest)) => value.split_first().is_some_and(|(&actual, value_rest)| {
            expected == actual && wildcard_matches(rest, value_rest)
        }),
    }
}

fn apply_channel_environment(channel: &mut ssh2::Channel, environment: &[(String, String)]) {
    for (name, value) in environment {
        let _ = channel.setenv(name, value);
    }
}

fn connect_session(
    config: &OpenSshConfig,
    proxy: Option<ProxyProcessSpec>,
    prompted: &mut bool,
) -> Result<Session> {
    let mut session = Session::new()?;
    apply_session_preferences(&session, config)?;
    if let Some(connect_timeout) = config.connect_timeout {
        session.set_timeout(connect_timeout.as_millis().min(u128::from(u32::MAX)) as u32);
    }
    if let Some(spec) = proxy {
        session.set_tcp_stream(connect_proxy_command(&spec)?);
    } else {
        let tcp = connect_tcp(config)?;
        tcp.set_nodelay(true)?;
        session.set_tcp_stream(tcp);
    }
    session.handshake()?;
    session.set_timeout(0);
    verify_host_key(&session, config, prompted)?;
    Ok(session)
}

fn authenticate_session(
    session: &Session,
    config: &OpenSshConfig,
    prompted: &mut bool,
) -> Result<()> {
    let authentication_order = if config.preferred_authentications.is_empty() {
        vec!["publickey", "keyboard-interactive", "password"]
    } else {
        config
            .preferred_authentications
            .iter()
            .map(String::as_str)
            .collect()
    };
    for method in authentication_order {
        if session.authenticated() {
            return Ok(());
        }
        let server_supports_method = session
            .auth_methods(&config.user)
            .unwrap_or_default()
            .split(',')
            .any(|supported| supported == method);
        if !server_supports_method {
            continue;
        }

        match method {
            "publickey" if config.pubkey_authentication => {
                authenticate_with_public_keys(session, config, prompted)?;
            }
            "keyboard-interactive"
                if config.keyboard_interactive_authentication && !config.batch_mode =>
            {
                let mut prompter = ConsoleKeyboardInteractive { prompted };
                let _ = session.userauth_keyboard_interactive(&config.user, &mut prompter);
            }
            "password" if config.password_authentication && !config.batch_mode => {
                for _ in 0..config.number_of_password_prompts {
                    *prompted = true;
                    let terminal = console::Term::stderr();
                    terminal.write_str("SSH password: ")?;
                    let password = Zeroizing::new(terminal.read_secure_line()?);
                    if session.userauth_password(&config.user, &password).is_ok() {
                        break;
                    }
                }
            }
            "publickey" | "keyboard-interactive" | "password" => {}
            unsupported => {
                bail!("SSH authentication method {unsupported} requires the native SSH fallback")
            }
        }
    }
    if session.authenticated() {
        return Ok(());
    }
    bail!("configured SSH credentials require the native SSH fallback")
}

struct ConsoleKeyboardInteractive<'a> {
    prompted: &'a mut bool,
}

impl KeyboardInteractivePrompt for ConsoleKeyboardInteractive<'_> {
    fn prompt(
        &mut self,
        username: &str,
        instructions: &str,
        prompts: &[Prompt<'_>],
    ) -> Vec<String> {
        if !prompts.is_empty() {
            *self.prompted = true;
        }
        let terminal = console::Term::stderr();
        if !username.is_empty() {
            let _ = terminal.write_line(username);
        }
        if !instructions.is_empty() {
            let _ = terminal.write_line(instructions);
        }
        prompts
            .iter()
            .map(|prompt| {
                let _ = terminal.write_str(&prompt.text);
                if prompt.echo {
                    terminal.read_line().unwrap_or_default()
                } else {
                    terminal.read_secure_line().unwrap_or_default()
                }
            })
            .collect()
    }
}

fn authenticate_with_public_keys(
    session: &Session,
    config: &OpenSshConfig,
    prompted: &mut bool,
) -> Result<()> {
    if config.use_agent {
        authenticate_with_agent(session, config);
    }
    if session.authenticated() {
        return Ok(());
    }

    for identity_file in &config.identity_files {
        if !identity_file.is_file() {
            continue;
        }
        if session
            .userauth_pubkey_file(&config.user, None, identity_file, None)
            .is_ok()
        {
            return Ok(());
        }
        if !config.batch_mode && private_key_requires_passphrase(identity_file) {
            for _ in 0..config.number_of_password_prompts {
                *prompted = true;
                let terminal = console::Term::stderr();
                terminal.write_str(&format!(
                    "Enter passphrase for key '{}': ",
                    identity_file.display()
                ))?;
                let passphrase = Zeroizing::new(terminal.read_secure_line()?);
                if session
                    .userauth_pubkey_file(
                        &config.user,
                        None,
                        identity_file,
                        Some(passphrase.as_str()),
                    )
                    .is_ok()
                {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn private_key_requires_passphrase(path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    if contents.contains("ENCRYPTED PRIVATE KEY") || contents.contains("Proc-Type: 4,ENCRYPTED") {
        return true;
    }
    if !contents.contains("BEGIN OPENSSH PRIVATE KEY") {
        return false;
    }
    let encoded = contents
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<String>();
    let Ok(decoded) = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        encoded.as_bytes(),
    ) else {
        return false;
    };
    let Some(after_magic) = decoded.strip_prefix(b"openssh-key-v1\0") else {
        return false;
    };
    let Some(length) = after_magic.get(..4) else {
        return false;
    };
    let cipher_length = u32::from_be_bytes(length.try_into().unwrap()) as usize;
    after_magic.get(4..4 + cipher_length) != Some(b"none")
}

fn proxy_process_spec(
    config: &OpenSshConfig,
    ssh_executable: &Path,
    ssh_args: &[OsString],
) -> Result<Option<ProxyProcessSpec>> {
    if let Some(command) = &config.proxy_command {
        if proxy_command_requires_shell(command) {
            bail!("shell-based ProxyCommand requires the native SSH fallback");
        }
        let mut words = shell_words::split(command).context("invalid ProxyCommand")?;
        if words.first().is_some_and(|word| word == "exec") {
            words.remove(0);
        }
        if words.is_empty() {
            bail!("ProxyCommand is empty");
        }
        for word in &mut words {
            *word = expand_proxy_tokens(word, config)?;
        }
        let program = PathBuf::from(words.remove(0));
        return Ok(Some(ProxyProcessSpec {
            program,
            args: words.into_iter().map(OsString::from).collect(),
        }));
    }

    let Some(proxy_jump) = &config.proxy_jump else {
        return Ok(None);
    };
    let mut hops = proxy_jump
        .split(',')
        .map(str::trim)
        .filter(|hop| !hop.is_empty())
        .collect::<Vec<_>>();
    let final_hop = hops.pop().context("ProxyJump is empty")?;
    let mut args = proxy_jump_config_args(ssh_args);
    args.push(OsString::from("-q"));
    if !hops.is_empty() {
        args.push(OsString::from("-J"));
        args.push(OsString::from(hops.join(",")));
    }
    args.push(OsString::from("-W"));
    args.push(OsString::from(proxy_destination(config)));
    args.push(OsString::from(final_hop));
    Ok(Some(ProxyProcessSpec {
        program: ssh_executable.to_path_buf(),
        args,
    }))
}

/// 直接派生只覆盖不依赖 shell 的 ProxyCommand。管道、重定向、变量展开等
/// 必须在建立任何连接前交还 OpenSSH，不能把 shell 语义误当成 argv。
fn proxy_command_requires_shell(command: &str) -> bool {
    if command.contains('\\') {
        // `shell_words` 使用 POSIX 转义规则，不能无损解析 Windows 路径。
        return true;
    }
    let mut quote = None;
    let mut escaped = false;
    for character in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            continue;
        }
        if quote != Some('\'') && matches!(character, '$' | '`') {
            return true;
        }
        if quote.is_none() && matches!(character, '~' | '#' | '^') {
            // `~` 与 `#` 依赖 POSIX shell 的单词位置语义，`^` 可能是
            // Windows cmd.exe 的转义符；直接 argv 派生无法无损复现。
            return true;
        }
        if quote.is_none()
            && (character.is_control()
                || matches!(
                    character,
                    '|' | '&' | ';' | '<' | '>' | '(' | ')' | '*' | '?'
                ))
        {
            return true;
        }
    }
    quote.is_some() || escaped
}

/// `ProxyJump` 由一个本机 OpenSSH 子进程承载。`-F` 必须继续传给该子进程，
/// 否则只存在于自定义配置文件里的 jump host 别名会在这里失效。
fn proxy_jump_config_args(ssh_args: &[OsString]) -> Vec<OsString> {
    let mut config_args = Vec::new();
    let mut index = 0;
    while index < ssh_args.len() {
        let argument = ssh_args[index].to_string_lossy();
        if argument == "-F" {
            if let Some(path) = ssh_args.get(index + 1) {
                config_args.push(OsString::from("-F"));
                config_args.push(path.clone());
                index += 2;
                continue;
            }
        } else if argument.starts_with("-F") && argument.len() > 2 {
            config_args.push(ssh_args[index].clone());
        }
        index += 1;
    }
    config_args
}

fn expand_proxy_tokens(value: &str, config: &OpenSshConfig) -> Result<String> {
    let mut expanded = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '%' {
            expanded.push(character);
            continue;
        }
        match chars
            .next()
            .context("ProxyCommand ends with an incomplete token")?
        {
            '%' => expanded.push('%'),
            'h' => expanded.push_str(&config.hostname),
            'p' => expanded.push_str(&config.port.to_string()),
            'r' => expanded.push_str(&config.user),
            'k' => expanded.push_str(config.host_key_alias.as_deref().unwrap_or(&config.hostname)),
            token => bail!("ProxyCommand token %{token} is not supported"),
        }
    }
    Ok(expanded)
}

fn proxy_destination(config: &OpenSshConfig) -> String {
    if config.hostname.contains(':') {
        format!("[{}]:{}", config.hostname, config.port)
    } else {
        format!("{}:{}", config.hostname, config.port)
    }
}

fn copy_proxy_stream(reader: &mut impl Read, writer: &mut impl Write) -> io::Result<()> {
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => writer.write_all(&buffer[..read])?,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn connect_proxy_command(spec: &ProxyProcessSpec) -> Result<ProxyCommandSocket> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let socket = TcpStream::connect(listener.local_addr()?)?;
    let (relay, _) = listener.accept()?;
    socket.set_nodelay(true)?;
    relay.set_nodelay(true)?;

    let mut child = command::blocking::Command::new(&spec.program)
        .args(&spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to start SSH ProxyCommand")?;
    let mut child_stdin = child.stdin.take().context("ProxyCommand has no stdin")?;
    let mut child_stdout = child.stdout.take().context("ProxyCommand has no stdout")?;
    let mut upload = relay.try_clone()?;
    thread::spawn(move || {
        let _ = copy_proxy_stream(&mut upload, &mut child_stdin);
    });
    thread::spawn(move || {
        let mut relay = relay;
        let _ = copy_proxy_stream(&mut child_stdout, &mut relay);
        let _ = relay.shutdown(Shutdown::Write);
    });

    Ok(ProxyCommandSocket { socket, child })
}

fn apply_session_preferences(session: &Session, config: &OpenSshConfig) -> Result<()> {
    for (method, preferences) in [
        (MethodType::Kex, config.kex_algorithms.as_deref()),
        (MethodType::HostKey, config.host_key_algorithms.as_deref()),
        (
            MethodType::SignAlgo,
            config.pubkey_accepted_algorithms.as_deref(),
        ),
        (MethodType::CryptCs, config.ciphers.as_deref()),
        (MethodType::CryptSc, config.ciphers.as_deref()),
        (MethodType::MacCs, config.macs.as_deref()),
        (MethodType::MacSc, config.macs.as_deref()),
    ] {
        if let Some(preferences) = preferences {
            session.method_pref(method, preferences)?;
        }
    }
    session.set_compress(config.compression);
    Ok(())
}

fn authentication_method_enabled(config: &OpenSshConfig, method: &str) -> bool {
    config.preferred_authentications.is_empty()
        || config
            .preferred_authentications
            .iter()
            .any(|configured| configured == method)
}

fn authenticate_with_agent(session: &Session, config: &OpenSshConfig) {
    let Ok(mut agent) = session.agent() else {
        return;
    };
    if let Some(path) = &config.identity_agent_path
        && agent.set_identity_path(path).is_err()
    {
        return;
    }
    if agent.connect().is_err() || agent.list_identities().is_err() {
        return;
    }
    let identities = agent.identities().unwrap_or_default();
    let configured_blobs = configured_public_key_blobs(&config.identity_files);

    // OpenSSH 先按 IdentityFile 配置顺序尝试 agent 中匹配的 key；
    // IdentitiesOnly=no 时再尝试 agent 的其余 key，最后才读取私钥文件。
    // 保持这个顺序可避免不必要的 passphrase prompt 和 MaxAuthTries 耗尽。
    for configured_blob in &configured_blobs {
        if let Some(identity) = identities
            .iter()
            .find(|identity| identity.blob() == configured_blob)
            && agent.userauth(&config.user, identity).is_ok()
        {
            let _ = agent.disconnect();
            return;
        }
    }
    if !config.identities_only {
        for identity in &identities {
            if configured_blobs
                .iter()
                .any(|configured_blob| configured_blob == identity.blob())
            {
                continue;
            }
            if agent.userauth(&config.user, identity).is_ok() {
                break;
            }
        }
    }
    let _ = agent.disconnect();
}

fn configured_public_key_blobs(identity_files: &[PathBuf]) -> Vec<Vec<u8>> {
    identity_files
        .iter()
        .filter_map(|identity_file| {
            let mut public_key_file = identity_file.as_os_str().to_owned();
            public_key_file.push(".pub");
            std::fs::read_to_string(PathBuf::from(public_key_file))
                .ok()
                .or_else(|| std::fs::read_to_string(identity_file).ok())
        })
        .filter_map(|contents| {
            contents
                .lines()
                .find(|line| !line.trim_start().starts_with('#'))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|encoded| {
                    base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        encoded.as_bytes(),
                    )
                    .ok()
                })
        })
        .collect()
}

fn connect_tcp(config: &OpenSshConfig) -> Result<TcpStream> {
    let addresses = (config.hostname.as_str(), config.port)
        .to_socket_addrs()?
        .filter(|address| match config.address_family.as_str() {
            "any" => true,
            "inet" => address.is_ipv4(),
            "inet6" => address.is_ipv6(),
            _ => false,
        })
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        bail!("SSH hostname did not resolve for AddressFamily");
    }

    let mut last_error = None;
    for _ in 0..config.connection_attempts {
        for address in &addresses {
            let result = connect_tcp_address(config, *address);
            match result {
                Ok(stream) => return Ok(stream),
                Err(error) => last_error = Some(error),
            }
        }
    }
    Err(last_error
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow!("failed to connect to SSH host")))
}

fn connect_tcp_address(config: &OpenSshConfig, address: SocketAddr) -> io::Result<TcpStream> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    socket.set_keepalive(config.tcp_keep_alive)?;
    if let Some(tos) = config.interactive_ip_qos {
        // OpenSSH 将 IPQoS 作为 best-effort socket hint；平台拒绝 DSCP 时
        // 连接仍应继续，不能把普通 SSH 变成不可用。
        let _ = set_socket_ip_qos(&socket, address, tos);
    }
    match config.connect_timeout {
        Some(timeout) => socket.connect_timeout(&SockAddr::from(address), timeout)?,
        None => socket.connect(&SockAddr::from(address))?,
    }
    Ok(socket.into())
}

#[cfg(unix)]
fn set_socket_ip_qos(socket: &Socket, address: SocketAddr, tos: u8) -> io::Result<()> {
    let (level, option) = if address.is_ipv4() {
        (libc::IPPROTO_IP, libc::IP_TOS)
    } else {
        (libc::IPPROTO_IPV6, libc::IPV6_TCLASS)
    };
    let value = i32::from(tos);
    let result = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            level,
            option,
            std::ptr::from_ref(&value).cast(),
            std::mem::size_of_val(&value) as libc::socklen_t,
        )
    };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn set_socket_ip_qos(socket: &Socket, address: SocketAddr, tos: u8) -> io::Result<()> {
    let (level, option) = if address.is_ipv4() {
        (IPPROTO_IP.0, IP_TOS)
    } else {
        (IPPROTO_IPV6.0, IPV6_TCLASS)
    };
    let value = i32::from(tos).to_ne_bytes();
    let result = unsafe {
        setsockopt(
            SOCKET(socket.as_raw_socket() as usize),
            level,
            option,
            Some(&value),
        )
    };
    if result == SOCKET_ERROR {
        Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }.0))
    } else {
        Ok(())
    }
}

fn verify_host_key(session: &Session, config: &OpenSshConfig, prompted: &mut bool) -> Result<()> {
    let (host_key, host_key_type) = session.host_key().context("SSH server sent no host key")?;
    let mut known_hosts = session.known_hosts()?;
    for path in config
        .global_known_hosts_files
        .iter()
        .chain(config.user_known_hosts_files.iter())
    {
        if path.is_file() {
            known_hosts.read_file(path, KnownHostFileKind::OpenSSH)?;
        }
    }

    let host = config.host_key_alias.as_deref().unwrap_or(&config.hostname);
    match known_hosts.check_port(host, config.port, host_key) {
        CheckResult::Match => Ok(()),
        CheckResult::NotFound
            if matches!(
                config
                    .strict_host_key_checking
                    .to_ascii_lowercase()
                    .as_str(),
                "no" | "off"
            ) =>
        {
            Ok(())
        }
        CheckResult::NotFound
            if config
                .strict_host_key_checking
                .eq_ignore_ascii_case("accept-new") =>
        {
            append_known_host(config, host, host_key, host_key_type)
        }
        CheckResult::NotFound
            if config.strict_host_key_checking.eq_ignore_ascii_case("ask")
                && !config.batch_mode =>
        {
            let fingerprint = session
                .host_key_hash(HashType::Sha256)
                .map(|hash| {
                    format!(
                        "SHA256:{}",
                        base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD_NO_PAD,
                            hash
                        )
                    )
                })
                .unwrap_or_else(|| "unavailable".to_string());
            let terminal = console::Term::stderr();
            terminal.write_line(&format!(
                "The authenticity of host '{host}' can't be established."
            ))?;
            terminal.write_line(&format!("Host key fingerprint is {fingerprint}."))?;
            terminal.write_str("Are you sure you want to continue connecting (yes/no)? ")?;
            *prompted = true;
            let answer = terminal.read_line()?;
            if answer.eq_ignore_ascii_case("yes") || answer == fingerprint {
                append_known_host(config, host, host_key, host_key_type)
            } else {
                bail!("SSH host key was not accepted")
            }
        }
        CheckResult::NotFound => bail!("SSH host key is not trusted"),
        CheckResult::Mismatch => bail!("SSH host key mismatch"),
        CheckResult::Failure => bail!("failed to verify SSH host key"),
    }
}

fn append_known_host(
    config: &OpenSshConfig,
    host: &str,
    host_key: &[u8],
    host_key_type: HostKeyType,
) -> Result<()> {
    let path = config
        .user_known_hosts_files
        .iter()
        .find(|path| !is_null_device(path))
        .context("no writable UserKnownHostsFile is configured")?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    let needs_newline = std::fs::read(path)
        .ok()
        .is_some_and(|contents| !contents.is_empty() && !contents.ends_with(b"\n"));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    if needs_newline {
        writeln!(file)?;
    }
    let destination = if config.port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{}", config.port)
    };
    let algorithm = match host_key_type {
        HostKeyType::Unknown => bail!("unsupported SSH host key type"),
        HostKeyType::Rsa => "ssh-rsa",
        HostKeyType::Dss => "ssh-dss",
        HostKeyType::Ecdsa256 => "ecdsa-sha2-nistp256",
        HostKeyType::Ecdsa384 => "ecdsa-sha2-nistp384",
        HostKeyType::Ecdsa521 => "ecdsa-sha2-nistp521",
        HostKeyType::Ed25519 => "ssh-ed25519",
    };
    writeln!(
        file,
        "{destination} {algorithm} {}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, host_key)
    )?;
    Ok(())
}

fn is_null_device(path: &Path) -> bool {
    path == Path::new("/dev/null") || path.to_string_lossy().eq_ignore_ascii_case("nul")
}

fn detect_remote_shell(session: &Session) -> Result<RemoteShell> {
    let posix_probe = exec_capture(session, "echo __WARP_REMOTE_SHELL__$SHELL")?;
    for line in posix_probe.lines() {
        let Some(shell) = line.trim().strip_prefix("__WARP_REMOTE_SHELL__") else {
            continue;
        };
        if shell.ends_with("/bash") || shell == "bash" {
            return Ok(RemoteShell::Bash);
        }
        if shell.ends_with("/zsh") || shell == "zsh" {
            return Ok(RemoteShell::Zsh);
        }
    }

    let probe = "$os=if($PSVersionTable.PSVersion.Major -le 5 -or $IsWindows -or $env:OS -eq 'Windows_NT'){'windows'}else{'unknown'};[Console]::Out.WriteLine('__WARP_REMOTE_CAPS__v=1;os={0};shell=powershell' -f $os)";
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        probe
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let powershell_probe = exec_capture(
        session,
        &format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}"),
    )?;
    if is_windows_powershell_capability(&powershell_probe) {
        return Ok(RemoteShell::PowerShell);
    }
    bail!("remote shell is not supported by the enhanced SSH path")
}

fn is_windows_powershell_capability(output: &str) -> bool {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    lines.next().is_some_and(|line| {
        line.trim_end_matches('\r') == "__WARP_REMOTE_CAPS__v=1;os=windows;shell=powershell"
    }) && lines.next().is_none()
}

fn exec_capture(session: &Session, command: &str) -> Result<String> {
    let mut channel = session.channel_session()?;
    channel.exec(command)?;
    let mut stdout = String::new();
    channel.read_to_string(&mut stdout)?;
    let mut stderr = String::new();
    channel.stderr().read_to_string(&mut stderr)?;
    channel.wait_close()?;
    if channel.exit_status()? != 0 {
        bail!("remote SSH probe failed");
    }
    Ok(stdout)
}

fn new_capability() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn emit_ssh_hook(
    endpoint: SocketAddr,
    capability: &str,
    remote_shell: RemoteShell,
    session_id: u64,
    remote_session_id: u64,
    control_scope: &str,
    hop_depth: u32,
) -> Result<()> {
    let payload = ssh_hook_payload(
        endpoint,
        capability,
        remote_shell,
        session_id,
        remote_session_id,
        control_scope,
        hop_depth,
    )?;
    let mut stdout = io::stdout().lock();
    write!(stdout, "\x1b]9278;d;{}\x07", hex::encode(payload))?;
    stdout.flush()?;
    Ok(())
}

fn ssh_hook_payload(
    endpoint: SocketAddr,
    capability: &str,
    remote_shell: RemoteShell,
    session_id: u64,
    remote_session_id: u64,
    control_scope: &str,
    hop_depth: u32,
) -> Result<Vec<u8>> {
    if !matches!(control_scope, "local" | "remote") || !(1..=8).contains(&hop_depth) {
        bail!("invalid SSH transport scope or hop depth");
    }
    serde_json::to_vec(&SshHook {
        hook: "SSH",
        value: SshHookValue {
            transport: SshHookTransport {
                version: 1,
                transport_type: "rust_broker",
                endpoint: endpoint.to_string(),
                capability,
            },
            remote_shell: remote_shell.hook_name(),
            session_id,
            remote_session_id,
            parent_session_id: session_id,
            control_scope,
            hop_depth,
            external_control_master: false,
        },
    })
    .map_err(Into::into)
}

fn spawn_broker(
    listener: TcpListener,
    session: Session,
    session_gate: SessionGate,
    capability: String,
    running: Arc<AtomicBool>,
    channel_environment: Arc<Vec<(String, String)>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while running.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let session = session.clone();
                    let session_gate = session_gate.clone();
                    let capability = capability.clone();
                    let channel_environment = channel_environment.clone();
                    thread::spawn(move || {
                        let _ = handle_broker_connection(
                            stream,
                            session,
                            &session_gate,
                            &capability,
                            &channel_environment,
                        );
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(POLL_INTERVAL);
                }
                Err(_) => break,
            }
        }
    })
}

fn handle_broker_connection(
    mut stream: TcpStream,
    session: Session,
    session_gate: &SessionGate,
    expected_capability: &str,
    channel_environment: &[(String, String)],
) -> Result<()> {
    stream.set_nodelay(true)?;
    let request: BrokerRequest = read_header(&mut stream)?;
    if !capabilities_match(&request.capability, expected_capability) {
        stream.write_all(&[1])?;
        bail!("invalid SSH broker capability");
    }

    match request.operation {
        BrokerOperation::Exec { command } => {
            let mut channel = retry_ssh(session_gate, || session.channel_session())?;
            apply_channel_environment(&mut channel, channel_environment);
            retry_ssh(session_gate, || channel.exec(&command))?;
            stream.write_all(&[0])?;
            stream.flush()?;
            bridge_broker_channel(stream, channel, session_gate)
        }
        BrokerOperation::Upload {
            remote_path,
            size,
            windows,
        } => handle_broker_upload(stream, session, session_gate, &remote_path, size, windows),
    }
}

fn handle_broker_upload(
    mut stream: TcpStream,
    session: Session,
    session_gate: &SessionGate,
    remote_path: &str,
    size: u64,
    windows: bool,
) -> Result<()> {
    let upload = prepare_scp_upload(&session, session_gate, remote_path, size, windows);
    let (mut channel, _sftp) = match upload {
        Ok(upload) => upload,
        Err(error) => {
            stream.write_all(&[1])?;
            return Err(error);
        }
    };
    stream.write_all(&[0])?;
    stream.flush()?;
    stream.set_read_timeout(Some(remote_server::setup::SCP_INSTALL_TIMEOUT))?;

    let mut remaining = size;
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        let limit = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
        let read = stream.read(&mut buffer[..limit])?;
        if read == 0 {
            bail!("SSH broker upload ended before the declared file size");
        }
        write_channel_all(&mut channel, &buffer[..read], session_gate)?;
        remaining -= read as u64;
    }
    write_channel_all(&mut channel, &[0], session_gate)?;
    read_scp_ack(&mut channel, session_gate)?;
    retry_ssh(session_gate, || channel.send_eof())?;
    retry_ssh(session_gate, || channel.wait_eof())?;
    retry_ssh(session_gate, || channel.close())?;
    retry_ssh(session_gate, || channel.wait_close())?;
    let exit_code = retry_ssh(session_gate, || channel.exit_status())?;
    write_frame(&mut stream, FRAME_EXIT, &exit_code.to_be_bytes())?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

fn prepare_scp_upload(
    session: &Session,
    session_gate: &SessionGate,
    remote_path: &str,
    size: u64,
    windows: bool,
) -> Result<(ssh2::Channel, ssh2::Sftp)> {
    let relative_path = remote_path
        .trim_start_matches("~/")
        .trim_start_matches("~\\")
        .replace('\\', "/");
    let components = relative_path
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    if components.is_empty()
        || components.iter().any(|component| {
            matches!(*component, "." | "..")
                || !component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        })
    {
        bail!("invalid relative SSH upload path");
    }

    let sftp = retry_ssh(session_gate, || session.sftp())?;
    let mut path = retry_ssh(session_gate, || sftp.realpath(Path::new(".")))?;
    for component in &components[..components.len() - 1] {
        path.push(component);
        let stat = retry_ssh(session_gate, || sftp.stat(&path));
        if stat.is_err() {
            retry_ssh(session_gate, || sftp.mkdir(&path, 0o700))?;
        }
    }
    path.push(components.last().unwrap());
    let path = path
        .to_str()
        .context("SSH upload target is not valid UTF-8")?;
    if path.contains(['\r', '\n', '\0']) {
        bail!("SSH upload target contains an invalid character");
    }
    let quoted_path = if windows {
        path.replace('`', "``")
            .replace('$', "`$")
            .replace('"', "`\"")
    } else {
        path.replace('\\', "\\\\")
            .replace('$', "\\$")
            .replace('`', "\\`")
            .replace('"', "\\\"")
    };
    let mut channel = retry_ssh(session_gate, || session.channel_session())?;
    retry_ssh(session_gate, || {
        channel.exec(&format!("scp -t \"{quoted_path}\""))
    })?;
    read_scp_ack(&mut channel, session_gate)?;
    let file_name = components.last().unwrap();
    write_channel_all(
        &mut channel,
        format!("C0600 {size} {file_name}\n").as_bytes(),
        session_gate,
    )?;
    read_scp_ack(&mut channel, session_gate)?;
    Ok((channel, sftp))
}

fn write_channel_all(
    channel: &mut ssh2::Channel,
    mut bytes: &[u8],
    session_gate: &SessionGate,
) -> Result<()> {
    let deadline = Instant::now() + remote_server::setup::SCP_INSTALL_TIMEOUT;
    while !bytes.is_empty() {
        let result = {
            let _guard = lock_session_gate(session_gate);
            channel.write(bytes)
        };
        match result {
            Ok(0) => bail!("SSH upload channel made no progress"),
            Ok(written) => bytes = &bytes[written..],
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(POLL_INTERVAL)
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                bail!("SSH upload channel timed out")
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn read_scp_ack(channel: &mut ssh2::Channel, session_gate: &SessionGate) -> Result<()> {
    let mut ack = [0_u8; 1];
    let deadline = Instant::now() + remote_server::setup::SCP_INSTALL_TIMEOUT;
    loop {
        let result = {
            let _guard = lock_session_gate(session_gate);
            channel.read(&mut ack)
        };
        match result {
            Ok(1) if ack[0] == 0 => return Ok(()),
            Ok(1) => bail!("remote SCP process rejected the upload"),
            Ok(0) => bail!("remote SCP process closed before acknowledging the upload"),
            Ok(_) => unreachable!(),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(POLL_INTERVAL)
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                bail!("remote SCP process acknowledgement timed out")
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn bridge_broker_channel(
    mut stream: TcpStream,
    mut channel: ssh2::Channel,
    session_gate: &SessionGate,
) -> Result<()> {
    let (input_tx, input_rx) = mpsc::channel::<Option<Vec<u8>>>();
    let mut input_stream = stream.try_clone()?;
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match input_stream.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    let _ = input_tx.send(None);
                    break;
                }
                Ok(read) => {
                    if input_tx.send(Some(buffer[..read].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut pending = VecDeque::<Vec<u8>>::new();
    let mut pending_offset = 0;
    let mut local_eof = false;
    let mut sent_eof = false;
    let mut stdout_buffer = [0_u8; 8192];
    let mut stderr_buffer = [0_u8; 8192];
    loop {
        let mut progressed = false;
        while let Ok(input) = input_rx.try_recv() {
            match input {
                Some(bytes) => pending.push_back(bytes),
                None => local_eof = true,
            }
            progressed = true;
        }
        progressed |= write_pending(
            &mut channel,
            &mut pending,
            &mut pending_offset,
            session_gate,
        )?;
        if local_eof && pending.is_empty() && !sent_eof {
            let _guard = lock_session_gate(session_gate);
            match channel.send_eof() {
                Ok(()) => {
                    sent_eof = true;
                    progressed = true;
                }
                Err(error) if is_ssh_would_block(&error) => {}
                Err(error) => return Err(error.into()),
            }
        }

        progressed |= read_channel_frame(
            &mut channel,
            &mut stdout_buffer,
            FRAME_STDOUT,
            &mut stream,
            session_gate,
        )?;
        progressed |= read_channel_frame(
            &mut channel.stderr(),
            &mut stderr_buffer,
            FRAME_STDERR,
            &mut stream,
            session_gate,
        )?;
        let channel_eof = {
            let _guard = lock_session_gate(session_gate);
            channel.eof()
        };
        if channel_eof && !progressed {
            let exit_code = retry_ssh(session_gate, || channel.exit_status())?;
            write_frame(&mut stream, FRAME_EXIT, &exit_code.to_be_bytes())?;
            let _ = stream.shutdown(Shutdown::Both);
            return Ok(());
        }
        if !progressed {
            thread::sleep(POLL_INTERVAL);
        }
    }
}

fn bridge_interactive_channel(
    mut channel: ssh2::Channel,
    session: &Session,
    session_gate: &SessionGate,
    server_alive_interval: Option<Duration>,
    server_alive_count_max: u32,
    escape_char: Option<u8>,
) -> Result<i32> {
    let (input_tx, input_rx) = mpsc::channel::<Option<Vec<u8>>>();
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 8192];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    let _ = input_tx.send(None);
                    break;
                }
                Ok(read) => {
                    if input_tx.send(Some(buffer[..read].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut pending = VecDeque::<Vec<u8>>::new();
    let mut pending_offset = 0;
    let mut stdin_eof = false;
    let mut sent_eof = false;
    let mut stdout_buffer = [0_u8; 8192];
    let mut stderr_buffer = [0_u8; 8192];
    let mut dimensions = terminal_dimensions();
    let mut next_resize_poll = Instant::now() + RESIZE_POLL_INTERVAL;
    let mut next_keepalive = server_alive_interval.map(|interval| Instant::now() + interval);
    let mut keepalive_failures = 0_u32;
    let mut escape_filter = SshEscapeFilter::new(escape_char);
    let mut local_disconnect = false;
    loop {
        let mut progressed = false;
        while let Ok(input) = input_rx.try_recv() {
            match input {
                Some(bytes) => {
                    let output = escape_filter.push(&bytes);
                    if !output.bytes.is_empty() {
                        pending.push_back(output.bytes);
                    }
                    if output.show_help {
                        print_ssh_escape_help()?;
                    }
                    if output.disconnect {
                        local_disconnect = true;
                        break;
                    }
                }
                None => {
                    let bytes = escape_filter.finish();
                    if !bytes.is_empty() {
                        pending.push_back(bytes);
                    }
                    stdin_eof = true;
                }
            }
            progressed = true;
        }
        progressed |= write_pending(
            &mut channel,
            &mut pending,
            &mut pending_offset,
            session_gate,
        )?;
        if local_disconnect && pending.is_empty() {
            return Ok(0);
        }
        if stdin_eof && pending.is_empty() && !sent_eof {
            let _guard = lock_session_gate(session_gate);
            match channel.send_eof() {
                Ok(()) => {
                    sent_eof = true;
                    progressed = true;
                }
                Err(error) if is_ssh_would_block(&error) => {}
                Err(error) => return Err(error.into()),
            }
        }

        progressed |= read_channel_output(&mut channel, &mut stdout_buffer, false, session_gate)?;
        progressed |= read_channel_output(
            &mut channel.stderr(),
            &mut stderr_buffer,
            true,
            session_gate,
        )?;
        if Instant::now() >= next_resize_poll {
            let next_dimensions = terminal_dimensions();
            if next_dimensions != dimensions {
                let resize_result = {
                    let _guard = lock_session_gate(session_gate);
                    channel.request_pty_size(next_dimensions.0, next_dimensions.1, None, None)
                };
                match resize_result {
                    Ok(()) => {
                        dimensions = next_dimensions;
                        progressed = true;
                    }
                    Err(error) if is_ssh_would_block(&error) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            next_resize_poll = Instant::now() + RESIZE_POLL_INTERVAL;
        }
        if next_keepalive.is_some_and(|deadline| Instant::now() >= deadline) {
            let keepalive_result = {
                let _guard = lock_session_gate(session_gate);
                session.keepalive_send()
            };
            match keepalive_result {
                Ok(seconds_until_next) => {
                    keepalive_failures = 0;
                    next_keepalive = Some(
                        Instant::now() + Duration::from_secs(u64::from(seconds_until_next.max(1))),
                    );
                    progressed = true;
                }
                Err(error) if is_ssh_would_block(&error) => {
                    next_keepalive = Some(Instant::now() + POLL_INTERVAL);
                }
                Err(error) => {
                    keepalive_failures = keepalive_failures.saturating_add(1);
                    if server_alive_count_max > 0 && keepalive_failures >= server_alive_count_max {
                        return Err(error.into());
                    }
                    next_keepalive =
                        server_alive_interval.map(|interval| Instant::now() + interval);
                }
            }
        }
        let channel_eof = {
            let _guard = lock_session_gate(session_gate);
            channel.eof()
        };
        if channel_eof && !progressed {
            return retry_ssh(session_gate, || channel.exit_status());
        }
        if !progressed {
            thread::sleep(POLL_INTERVAL);
        }
    }
}

fn print_ssh_escape_help() -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    stderr.write_all(
        b"\r\nSupported InfiniShell SSH escape sequences:\r\n  ~.  disconnect\r\n  ~~  send the escape character\r\n  ~?  show this help\r\n",
    )?;
    stderr.flush()
}

fn terminal_dimensions() -> (u32, u32) {
    let (rows, columns) = console::Term::stdout().size();
    (u32::from(columns.max(1)), u32::from(rows.max(1)))
}

fn write_pending(
    writer: &mut ssh2::Channel,
    pending: &mut VecDeque<Vec<u8>>,
    pending_offset: &mut usize,
    session_gate: &SessionGate,
) -> Result<bool> {
    let Some(front) = pending.front() else {
        return Ok(false);
    };
    let _guard = lock_session_gate(session_gate);
    match writer.write(&front[*pending_offset..]) {
        Ok(0) => Ok(false),
        Ok(written) => {
            *pending_offset += written;
            if *pending_offset == front.len() {
                pending.pop_front();
                *pending_offset = 0;
            }
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn read_channel_frame<R: Read>(
    reader: &mut R,
    buffer: &mut [u8],
    kind: u8,
    stream: &mut TcpStream,
    session_gate: &SessionGate,
) -> Result<bool> {
    let read_result = {
        let _guard = lock_session_gate(session_gate);
        reader.read(buffer)
    };
    match read_result {
        Ok(0) => Ok(false),
        Ok(read) => {
            write_frame(stream, kind, &buffer[..read])?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn read_channel_output<R: Read>(
    reader: &mut R,
    buffer: &mut [u8],
    stderr: bool,
    session_gate: &SessionGate,
) -> Result<bool> {
    let read_result = {
        let _guard = lock_session_gate(session_gate);
        reader.read(buffer)
    };
    match read_result {
        Ok(0) => Ok(false),
        Ok(read) => {
            if stderr {
                let mut output = io::stderr().lock();
                output.write_all(&buffer[..read])?;
                output.flush()?;
            } else {
                let mut output = io::stdout().lock();
                output.write_all(&buffer[..read])?;
                output.flush()?;
            }
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn retry_ssh<T>(
    session_gate: &SessionGate,
    mut operation: impl FnMut() -> std::result::Result<T, ssh2::Error>,
) -> Result<T> {
    // libssh2 的 channel-open/exec 等操作在 EAGAIN 后保存 session 级状态。
    // 整个重试期间持有外层 gate，避免其它 channel 插入调用破坏该状态机。
    let _guard = lock_session_gate(session_gate);
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if is_ssh_would_block(&error) && Instant::now() < deadline => {
                thread::sleep(POLL_INTERVAL);
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn lock_session_gate(session_gate: &SessionGate) -> MutexGuard<'_, ()> {
    session_gate
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn capabilities_match(actual: &str, expected: &str) -> bool {
    let mut difference = actual.len() ^ expected.len();
    for (actual, expected) in actual.bytes().zip(expected.bytes()) {
        difference |= usize::from(actual ^ expected);
    }
    difference == 0
}

fn is_ssh_would_block(error: &ssh2::Error) -> bool {
    error.code() == ErrorCode::Session(-37)
}

fn write_header(stream: &mut TcpStream, request: &BrokerRequest) -> Result<()> {
    let payload = serde_json::to_vec(request)?;
    if payload.len() > MAX_BROKER_HEADER_BYTES {
        bail!("SSH broker request is too large");
    }
    stream.write_all(&(payload.len() as u32).to_be_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()?;
    Ok(())
}

fn read_header(stream: &mut TcpStream) -> Result<BrokerRequest> {
    let length = read_u32(stream)? as usize;
    if length > MAX_BROKER_HEADER_BYTES {
        bail!("SSH broker request is too large");
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload)?;
    serde_json::from_slice(&payload).context("invalid SSH broker request")
}

fn write_frame(stream: &mut TcpStream, kind: u8, payload: &[u8]) -> Result<()> {
    stream.write_all(&[kind])?;
    stream.write_all(&(payload.len() as u32).to_be_bytes())?;
    stream.write_all(payload)?;
    stream.flush()?;
    Ok(())
}

fn read_u32(reader: &mut impl Read) -> Result<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

#[cfg(test)]
#[path = "rust_ssh_tests.rs"]
mod tests;
