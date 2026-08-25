//! 基于 russh 的异步单连接后端。
//!
//! 该模块复用父模块的 OpenSSH 配置解析、ProxyCommand/ProxyJump 展开、
//! PowerShell bootstrap 和 loopback broker 协议。

use std::borrow::Cow;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
#[cfg(windows)]
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, bail};
use remote_server::setup::RemoteOs;
use russh::ChannelMsg;
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::agent::AgentIdentity;
use russh::keys::{Algorithm, HashAlg, PrivateKeyWithHashAlg, PublicKey};
use ssh2::{CheckResult, KnownHostFileKind};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener as TokioTcpListener, TcpStream as TokioTcpStream};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use warp_cli::RustSshSessionArgs;
use zeroize::Zeroizing;

use super::{
    BrokerOperation, BrokerRequest, CONNECT_TIMEOUT, FRAME_EXIT, FRAME_STDERR, FRAME_STDOUT,
    MAX_BROKER_HEADER_BYTES, OpenSshConfig, ProxyCommandSocket, ProxyProcessSpec,
    RESIZE_POLL_INTERVAL, RemoteShell, SshEscapeFilter, capabilities_match, connect_proxy_command,
    connect_tcp, emit_ssh_hook, is_windows_powershell_capability, new_capability,
    print_ssh_escape_help, proxy_process_spec, resolve_channel_environment, resolve_openssh_config,
    run_ssh2_session_worker, terminal_dimensions,
};
use crate::remote_server::ssh_transport::{setup_command_line, upload_command};

type RusshHandle = Handle<HostKeyHandler>;
type SharedHandle = Arc<AsyncMutex<RusshHandle>>;

/// 建立 russh session；在交互提示或认证成功前失败，仍可安全回到已经验证过的
/// ssh2/OpenSSH 链路。
pub(super) fn run_session_worker(args: &RustSshSessionArgs) -> Result<i32> {
    let config = match resolve_openssh_config(&args.ssh_executable, &args.ssh_args) {
        Ok(config) => config,
        Err(_) => return run_ssh2_session_worker(args),
    };
    let proxy = match proxy_process_spec(&config, &args.ssh_executable, &args.ssh_args) {
        Ok(proxy) => proxy,
        Err(_) => return run_ssh2_session_worker(args),
    };
    #[cfg(windows)]
    let _console_input_mode = match super::WindowsConsoleInputMode::enter() {
        Ok(mode) => mode,
        Err(_) => return run_ssh2_session_worker(args),
    };

    let prompted = Arc::new(AtomicBool::new(false));
    let committed = Arc::new(AtomicBool::new(false));
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return run_ssh2_session_worker(args),
    };
    match runtime.block_on(run_session(
        args,
        config,
        proxy,
        prompted.clone(),
        committed.clone(),
    )) {
        Ok(exit_code) => Ok(exit_code),
        Err(_) if !prompted.load(Ordering::Acquire) && !committed.load(Ordering::Acquire) => {
            eprintln!("InfiniShell russh transport is unavailable; falling back to ssh2.");
            run_ssh2_session_worker(args)
        }
        Err(_) => Ok(255),
    }
}

async fn run_session(
    args: &RustSshSessionArgs,
    config: OpenSshConfig,
    proxy: Option<ProxyProcessSpec>,
    prompted: Arc<AtomicBool>,
    committed: Arc<AtomicBool>,
) -> Result<i32> {
    ensure_known_hosts_supported(&config)?;
    let russh_config = Arc::new(build_russh_config(&config)?);
    let (stream, _proxy_guard) = connect_stream(&config, proxy)?;
    let handler = HostKeyHandler {
        config: HostKeyConfig::from(&config),
        prompted: prompted.clone(),
    };
    let connect = client::connect_stream(russh_config, stream, handler);
    let mut handle =
        tokio::time::timeout(config.connect_timeout.unwrap_or(CONNECT_TIMEOUT), connect)
            .await
            .context("russh handshake timed out")??;
    authenticate(&mut handle, &config, prompted).await?;
    // 从这里开始会执行远端探测并最终承载交互 shell。失败时不能再自动新建
    // ssh2/OpenSSH 连接，否则无密码 agent 会话退出后可能意外弹出第二个 shell。
    committed.store(true, Ordering::Release);

    let channel_environment = Arc::new(resolve_channel_environment(&config));
    let enhanced = detect_remote_shell(&handle)
        .await
        .ok()
        .and_then(|remote_shell| {
            let command = match remote_shell {
                RemoteShell::Bash | RemoteShell::Zsh => args.posix_command.clone(),
                RemoteShell::PowerShell => args.windows_command.clone(),
            };
            Some((remote_shell, command))
        });
    let (interactive, mode) = match enhanced {
        Some((remote_shell, command)) => {
            match open_interactive_channel(&handle, &channel_environment, Some(&command)).await {
                Ok(channel) => match TokioTcpListener::bind(("127.0.0.1", 0)).await {
                    Ok(listener) => {
                        let endpoint = listener.local_addr()?;
                        (
                            channel,
                            RusshPreparedMode::Enhanced {
                                listener,
                                endpoint,
                                capability: new_capability(),
                                remote_shell,
                            },
                        )
                    }
                    Err(_) => (channel, RusshPreparedMode::Plain),
                },
                Err(_) => (
                    open_interactive_channel(&handle, &channel_environment, None).await?,
                    RusshPreparedMode::Plain,
                ),
            }
        }
        None => (
            open_interactive_channel(&handle, &channel_environment, None).await?,
            RusshPreparedMode::Plain,
        ),
    };

    let handle = Arc::new(AsyncMutex::new(handle));
    let broker = match mode {
        RusshPreparedMode::Enhanced {
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
            let handle = handle.clone();
            let channel_environment = channel_environment.clone();
            Some(tokio::spawn(async move {
                serve_broker(listener, handle, capability, channel_environment).await;
            }))
        }
        RusshPreparedMode::Plain => None,
    };

    let exit_code = bridge_interactive_channel(interactive, config.escape_char).await?;
    if let Some(broker) = broker {
        broker.abort();
    }
    let handle = handle.lock().await;
    let _ = handle
        .disconnect(russh::Disconnect::ByApplication, "session closed", "")
        .await;
    Ok(exit_code)
}

enum RusshPreparedMode {
    Enhanced {
        listener: TokioTcpListener,
        endpoint: std::net::SocketAddr,
        capability: String,
        remote_shell: RemoteShell,
    },
    Plain,
}

fn build_russh_config(config: &OpenSshConfig) -> Result<client::Config> {
    let mut preferred = russh::Preferred::default();
    if let Some(value) = &config.kex_algorithms {
        let mut algorithms = value
            .split(',')
            .filter_map(|name| russh::kex::Name::try_from(name).ok())
            .collect::<Vec<_>>();
        if algorithms.is_empty() {
            bail!("configured KexAlgorithms are not supported by russh");
        }
        algorithms.push(russh::kex::EXTENSION_SUPPORT_AS_CLIENT);
        algorithms.push(russh::kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT);
        preferred.kex = Cow::Owned(algorithms);
    }
    if let Some(value) = &config.host_key_algorithms {
        let algorithms = value
            .split(',')
            .filter_map(parse_key_algorithm)
            .collect::<Vec<_>>();
        if algorithms.is_empty() {
            bail!("configured HostKeyAlgorithms are not supported by russh");
        }
        preferred.key = Cow::Owned(algorithms);
    }
    if let Some(value) = &config.ciphers {
        let algorithms = value
            .split(',')
            .filter_map(|name| russh::cipher::Name::try_from(name).ok())
            .collect::<Vec<_>>();
        if algorithms.is_empty() {
            bail!("configured Ciphers are not supported by russh");
        }
        preferred.cipher = Cow::Owned(algorithms);
    }
    if let Some(value) = &config.macs {
        let algorithms = value
            .split(',')
            .filter_map(|name| russh::mac::Name::try_from(name).ok())
            .collect::<Vec<_>>();
        if algorithms.is_empty() {
            bail!("configured MACs are not supported by russh");
        }
        preferred.mac = Cow::Owned(algorithms);
    }
    preferred.compression = if config.compression {
        Cow::Owned(vec![
            russh::compression::ZLIB_LEGACY,
            russh::compression::ZLIB,
            russh::compression::NONE,
        ])
    } else {
        Cow::Owned(vec![russh::compression::NONE])
    };
    let keepalive_max = usize::try_from(config.server_alive_count_max).unwrap_or(usize::MAX);
    Ok(client::Config {
        preferred,
        keepalive_interval: config.server_alive_interval,
        keepalive_max,
        nodelay: true,
        ..Default::default()
    })
}

fn parse_key_algorithm(name: &str) -> Option<Algorithm> {
    match name {
        "ssh-ed25519" => Some(Algorithm::Ed25519),
        "ecdsa-sha2-nistp256" => Some(Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP256,
        }),
        "ecdsa-sha2-nistp384" => Some(Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP384,
        }),
        "ecdsa-sha2-nistp521" => Some(Algorithm::Ecdsa {
            curve: russh::keys::EcdsaCurve::NistP521,
        }),
        "rsa-sha2-512" => Some(Algorithm::Rsa {
            hash: Some(HashAlg::Sha512),
        }),
        "rsa-sha2-256" => Some(Algorithm::Rsa {
            hash: Some(HashAlg::Sha256),
        }),
        "ssh-rsa" => Some(Algorithm::Rsa { hash: None }),
        "sk-ssh-ed25519@openssh.com" => Some(Algorithm::SkEd25519),
        "sk-ecdsa-sha2-nistp256@openssh.com" => Some(Algorithm::SkEcdsaSha2NistP256),
        _ => None,
    }
}

fn connect_stream(
    config: &OpenSshConfig,
    proxy: Option<ProxyProcessSpec>,
) -> Result<(TokioTcpStream, Option<ProxyCommandSocket>)> {
    let (stream, proxy_guard) = match proxy {
        Some(spec) => {
            let proxy = connect_proxy_command(&spec)?;
            (proxy.socket.try_clone()?, Some(proxy))
        }
        None => (connect_tcp(config)?, None),
    };
    stream.set_nonblocking(true)?;
    Ok((TokioTcpStream::from_std(stream)?, proxy_guard))
}

#[derive(Clone)]
struct HostKeyConfig {
    host: String,
    port: u16,
    strict_host_key_checking: String,
    batch_mode: bool,
    user_known_hosts_files: Vec<std::path::PathBuf>,
    global_known_hosts_files: Vec<std::path::PathBuf>,
}

impl From<&OpenSshConfig> for HostKeyConfig {
    fn from(config: &OpenSshConfig) -> Self {
        Self {
            host: config
                .host_key_alias
                .clone()
                .unwrap_or_else(|| config.hostname.clone()),
            port: config.port,
            strict_host_key_checking: config.strict_host_key_checking.clone(),
            batch_mode: config.batch_mode,
            user_known_hosts_files: config.user_known_hosts_files.clone(),
            global_known_hosts_files: config.global_known_hosts_files.clone(),
        }
    }
}

struct HostKeyHandler {
    config: HostKeyConfig,
    prompted: Arc<AtomicBool>,
}

impl client::Handler for HostKeyHandler {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool> {
        match check_known_host(&self.config, key)? {
            KnownHostState::Match => Ok(true),
            KnownHostState::Mismatch => bail!("SSH host key mismatch"),
            KnownHostState::NotFound
                if self
                    .config
                    .strict_host_key_checking
                    .eq_ignore_ascii_case("accept-new") =>
            {
                append_russh_known_host(&self.config, key)?;
                Ok(true)
            }
            KnownHostState::NotFound
                if self
                    .config
                    .strict_host_key_checking
                    .eq_ignore_ascii_case("ask")
                    && !self.config.batch_mode =>
            {
                let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
                let terminal = console::Term::stderr();
                terminal.write_line(&format!(
                    "The authenticity of host '{}' can't be established.",
                    self.config.host
                ))?;
                terminal.write_line(&format!("Host key fingerprint is {fingerprint}."))?;
                terminal.write_str("Are you sure you want to continue connecting (yes/no)? ")?;
                self.prompted.store(true, Ordering::Release);
                let answer = terminal.read_line()?;
                if answer.eq_ignore_ascii_case("yes") || answer == fingerprint {
                    append_russh_known_host(&self.config, key)?;
                    Ok(true)
                } else {
                    bail!("SSH host key was not accepted")
                }
            }
            KnownHostState::NotFound => bail!("SSH host key is not trusted"),
        }
    }

    async fn auth_banner(&mut self, banner: &str, _: &mut client::Session) -> Result<()> {
        let mut stderr = io::stderr().lock();
        stderr.write_all(banner.as_bytes())?;
        stderr.flush()?;
        Ok(())
    }
}

enum KnownHostState {
    Match,
    NotFound,
    Mismatch,
}

fn check_known_host(config: &HostKeyConfig, key: &PublicKey) -> Result<KnownHostState> {
    let session = ssh2::Session::new()?;
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
    let key = key.to_bytes()?;
    match known_hosts.check_port(&config.host, config.port, &key) {
        CheckResult::Match => Ok(KnownHostState::Match),
        CheckResult::NotFound => Ok(KnownHostState::NotFound),
        CheckResult::Mismatch => Ok(KnownHostState::Mismatch),
        CheckResult::Failure => bail!("failed to verify SSH host key"),
    }
}

fn ensure_known_hosts_supported(config: &OpenSshConfig) -> Result<()> {
    for path in config
        .global_known_hosts_files
        .iter()
        .chain(config.user_known_hosts_files.iter())
    {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        if contents.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("@cert-authority") || line.starts_with("@revoked")
        }) {
            bail!("OpenSSH known_hosts markers require the ssh2 fallback");
        }
    }
    Ok(())
}

fn append_russh_known_host(config: &HostKeyConfig, key: &PublicKey) -> Result<()> {
    let path = config
        .user_known_hosts_files
        .iter()
        .find(|path| !super::is_null_device(path))
        .context("no writable UserKnownHostsFile is configured")?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    russh::keys::known_hosts::learn_known_hosts_path(&config.host, config.port, key, path)?;
    Ok(())
}

async fn authenticate(
    handle: &mut RusshHandle,
    config: &OpenSshConfig,
    prompted: Arc<AtomicBool>,
) -> Result<()> {
    if handle.authenticate_none(&config.user).await?.success() {
        return Ok(());
    }
    let order = if config.preferred_authentications.is_empty() {
        vec!["publickey", "keyboard-interactive", "password"]
    } else {
        config
            .preferred_authentications
            .iter()
            .map(String::as_str)
            .collect()
    };
    for method in order {
        let authenticated = match method {
            "publickey" if config.pubkey_authentication => {
                authenticate_public_keys(handle, config, prompted.clone()).await?
            }
            "keyboard-interactive"
                if config.keyboard_interactive_authentication && !config.batch_mode =>
            {
                authenticate_keyboard_interactive(handle, config, prompted.clone()).await?
            }
            "password" if config.password_authentication && !config.batch_mode => {
                authenticate_password(handle, config, prompted.clone()).await?
            }
            "publickey" | "keyboard-interactive" | "password" => false,
            unsupported => bail!("unsupported SSH authentication method {unsupported}"),
        };
        if authenticated {
            return Ok(());
        }
    }
    bail!("configured SSH credentials were rejected")
}

async fn authenticate_public_keys(
    handle: &mut RusshHandle,
    config: &OpenSshConfig,
    prompted: Arc<AtomicBool>,
) -> Result<bool> {
    if config.use_agent && authenticate_agent(handle, config).await.unwrap_or(false) {
        return Ok(true);
    }
    for identity_file in &config.identity_files {
        if !identity_file.is_file() {
            continue;
        }
        let mut key = russh::keys::load_secret_key(identity_file, None);
        if matches!(key, Err(russh::keys::Error::KeyIsEncrypted)) && !config.batch_mode {
            for _ in 0..config.number_of_password_prompts {
                prompted.store(true, Ordering::Release);
                let terminal = console::Term::stderr();
                terminal.write_str(&format!(
                    "Enter passphrase for key '{}': ",
                    identity_file.display()
                ))?;
                let passphrase = Zeroizing::new(terminal.read_secure_line()?);
                key = russh::keys::load_secret_key(identity_file, Some(passphrase.as_str()));
                if key.is_ok() {
                    break;
                }
            }
        }
        let Ok(key) = key else {
            continue;
        };
        let hash = rsa_hash(handle, key.algorithm()).await?;
        if !pubkey_algorithm_allowed(config, key.algorithm(), hash, false) {
            continue;
        }
        let result = handle
            .authenticate_publickey(
                &config.user,
                PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            )
            .await?;
        if result.success() {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn rsa_hash(handle: &RusshHandle, algorithm: Algorithm) -> Result<Option<HashAlg>> {
    if !matches!(algorithm, Algorithm::Rsa { .. }) {
        return Ok(None);
    }
    Ok(handle
        .best_supported_rsa_hash()
        .await?
        .unwrap_or(Some(HashAlg::Sha512)))
}

async fn authenticate_agent(handle: &mut RusshHandle, config: &OpenSshConfig) -> Result<bool> {
    #[cfg(unix)]
    let mut agent = match &config.identity_agent_path {
        Some(path) => russh::keys::agent::client::AgentClient::connect_uds(path)
            .await?
            .dynamic(),
        None => russh::keys::agent::client::AgentClient::connect_env()
            .await?
            .dynamic(),
    };
    #[cfg(windows)]
    let mut agent = {
        match &config.identity_agent_path {
            Some(path) => russh::keys::agent::client::AgentClient::connect_named_pipe(path)
                .await?
                .dynamic(),
            None => match russh::keys::agent::client::AgentClient::connect_named_pipe(Path::new(
                r"\\.\pipe\openssh-ssh-agent",
            ))
            .await
            {
                Ok(agent) => agent.dynamic(),
                Err(_) => russh::keys::agent::client::AgentClient::connect_pageant()
                    .await?
                    .dynamic(),
            },
        }
    };
    authenticate_agent_identities(handle, config, &mut agent).await
}

async fn authenticate_agent_identities<S>(
    handle: &mut RusshHandle,
    config: &OpenSshConfig,
    agent: &mut russh::keys::agent::client::AgentClient<S>,
) -> Result<bool>
where
    S: russh::keys::agent::client::AgentStream + Send + Unpin,
{
    let identities = agent.request_identities().await?;
    let configured_keys = config
        .identity_files
        .iter()
        .filter_map(load_identity_public_key)
        .collect::<Vec<_>>();
    let mut order = Vec::new();
    for configured in &configured_keys {
        if let Some(index) = identities
            .iter()
            .position(|identity| identity.public_key().as_ref() == configured)
            && !order.contains(&index)
        {
            order.push(index);
        }
    }
    if !config.identities_only {
        for index in 0..identities.len() {
            if !order.contains(&index) {
                order.push(index);
            }
        }
    }
    for index in order {
        let identity = &identities[index];
        let hash = rsa_hash(handle, identity.public_key().algorithm()).await?;
        let certificate = matches!(identity, AgentIdentity::Certificate { .. });
        if !pubkey_algorithm_allowed(config, identity.public_key().algorithm(), hash, certificate) {
            continue;
        }
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(&config.user, key.clone(), hash, agent)
                    .await?
            }
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(&config.user, certificate.clone(), hash, agent)
                    .await?
            }
        };
        if result.success() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn pubkey_algorithm_allowed(
    config: &OpenSshConfig,
    algorithm: Algorithm,
    hash: Option<HashAlg>,
    certificate: bool,
) -> bool {
    let Some(accepted) = &config.pubkey_accepted_algorithms else {
        return true;
    };
    let algorithm = match algorithm {
        Algorithm::Rsa { .. } => Algorithm::Rsa { hash },
        algorithm => algorithm,
    };
    let name = if certificate {
        algorithm.to_certificate_type()
    } else {
        algorithm.as_str().to_string()
    };
    accepted.split(',').any(|accepted| accepted == name)
}

fn load_identity_public_key(path: &std::path::PathBuf) -> Option<PublicKey> {
    let mut public_path = path.as_os_str().to_owned();
    public_path.push(".pub");
    russh::keys::load_public_key(std::path::PathBuf::from(public_path))
        .or_else(|_| russh::keys::load_public_key(path))
        .ok()
}

async fn authenticate_keyboard_interactive(
    handle: &mut RusshHandle,
    config: &OpenSshConfig,
    prompted: Arc<AtomicBool>,
) -> Result<bool> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(&config.user, None)
        .await?;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                if !prompts.is_empty() {
                    prompted.store(true, Ordering::Release);
                }
                let terminal = console::Term::stderr();
                if !name.is_empty() {
                    terminal.write_line(&name)?;
                }
                if !instructions.is_empty() {
                    terminal.write_line(&instructions)?;
                }
                let mut answers = Vec::with_capacity(prompts.len());
                for prompt in prompts {
                    terminal.write_str(&prompt.prompt)?;
                    let answer = if prompt.echo {
                        terminal.read_line()?
                    } else {
                        terminal.read_secure_line()?
                    };
                    answers.push(answer);
                }
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await?;
            }
        }
    }
}

async fn authenticate_password(
    handle: &mut RusshHandle,
    config: &OpenSshConfig,
    prompted: Arc<AtomicBool>,
) -> Result<bool> {
    for _ in 0..config.number_of_password_prompts {
        prompted.store(true, Ordering::Release);
        let terminal = console::Term::stderr();
        terminal.write_str("SSH password: ")?;
        let password = Zeroizing::new(terminal.read_secure_line()?);
        if handle
            .authenticate_password(&config.user, password.as_str())
            .await?
            .success()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn detect_remote_shell(handle: &RusshHandle) -> Result<RemoteShell> {
    let posix = exec_capture(handle, "echo __WARP_REMOTE_SHELL__$SHELL").await?;
    for line in posix.lines() {
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
    let output = exec_capture(
        handle,
        &format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}"),
    )
    .await?;
    if is_windows_powershell_capability(&output) {
        Ok(RemoteShell::PowerShell)
    } else {
        bail!("remote shell is not supported")
    }
}

async fn exec_capture(handle: &RusshHandle, command: &str) -> Result<String> {
    let mut channel = handle.channel_open_session().await?;
    channel.exec(true, command).await?;
    let mut stdout = Vec::new();
    let mut exit_status = None;
    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => {
                if stdout.len().saturating_add(data.len()) > MAX_BROKER_HEADER_BYTES {
                    bail!("remote probe output is too large");
                }
                stdout.extend_from_slice(&data);
            }
            ChannelMsg::ExitStatus {
                exit_status: status,
            } => exit_status = Some(status),
            ChannelMsg::Close => break,
            ChannelMsg::Eof
            | ChannelMsg::ExtendedData { .. }
            | ChannelMsg::ExitSignal { .. }
            | ChannelMsg::WindowAdjusted { .. }
            | ChannelMsg::Success
            | ChannelMsg::Failure
            | ChannelMsg::XonXoff { .. }
            | ChannelMsg::Open { .. }
            | ChannelMsg::RequestPty { .. }
            | ChannelMsg::RequestShell { .. }
            | ChannelMsg::Exec { .. }
            | ChannelMsg::Signal { .. }
            | ChannelMsg::RequestSubsystem { .. }
            | ChannelMsg::RequestX11 { .. }
            | ChannelMsg::SetEnv { .. }
            | ChannelMsg::WindowChange { .. }
            | ChannelMsg::AgentForward { .. }
            | ChannelMsg::OpenFailure(_) => {}
            // ChannelMsg 是 non_exhaustive；未知协议消息不应改变 probe 结果。
            _ => {}
        }
    }
    if exit_status.unwrap_or(255) != 0 {
        bail!("remote SSH probe failed");
    }
    String::from_utf8(stdout).context("remote SSH probe returned non-UTF-8 output")
}

async fn open_interactive_channel(
    handle: &RusshHandle,
    environment: &[(String, String)],
    command: Option<&str>,
) -> Result<russh::Channel<client::Msg>> {
    let channel = handle.channel_open_session().await?;
    for (name, value) in environment {
        let _ = channel.set_env(false, name, value).await;
    }
    let (columns, rows) = terminal_dimensions();
    channel
        .request_pty(true, "xterm-256color", columns, rows, 0, 0, &[])
        .await?;
    match command {
        Some(command) => channel.exec(true, command).await?,
        None => channel.request_shell(true).await?,
    }
    Ok(channel)
}

async fn serve_broker(
    listener: TokioTcpListener,
    handle: SharedHandle,
    capability: String,
    environment: Arc<Vec<(String, String)>>,
) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let handle = handle.clone();
        let capability = capability.clone();
        let environment = environment.clone();
        tokio::spawn(async move {
            let _ = handle_broker_connection(stream, handle, &capability, &environment).await;
        });
    }
}

async fn handle_broker_connection(
    mut stream: TokioTcpStream,
    handle: SharedHandle,
    expected_capability: &str,
    environment: &[(String, String)],
) -> Result<()> {
    stream.set_nodelay(true)?;
    let request = read_broker_header(&mut stream).await?;
    if !capabilities_match(&request.capability, expected_capability) {
        stream.write_all(&[1]).await?;
        bail!("invalid SSH broker capability");
    }
    let (command, mut remaining_upload_bytes) = match request.operation {
        BrokerOperation::Exec { command } => (command, None),
        BrokerOperation::Upload {
            remote_path,
            size,
            windows,
        } => {
            let remote_os = if windows {
                RemoteOs::Windows
            } else {
                RemoteOs::Linux
            };
            (
                setup_command_line(&upload_command(&remote_os, &remote_path)),
                Some(size),
            )
        }
    };
    let mut channel = {
        let handle = handle.lock().await;
        handle.channel_open_session().await?
    };
    for (name, value) in environment {
        let _ = channel.set_env(false, name, value).await;
    }
    channel.exec(true, command).await?;
    stream.write_all(&[0]).await?;
    stream.flush().await?;

    let mut buffer = vec![0_u8; 8192];
    let mut exit_status = 255_i32;
    let mut local_eof = false;
    loop {
        tokio::select! {
            read = stream.read(&mut buffer), if !local_eof => {
                match read? {
                    0 => {
                        if remaining_upload_bytes.is_some_and(|remaining| remaining != 0) {
                            bail!("SSH broker upload ended before the declared file size");
                        }
                        local_eof = true;
                        channel.eof().await?;
                    }
                    read => {
                        if let Some(remaining) = remaining_upload_bytes.as_mut() {
                            let read = read as u64;
                            if read > *remaining {
                                bail!("SSH broker upload exceeded the declared file size");
                            }
                            *remaining -= read;
                        }
                        channel.data_bytes(buffer[..read].to_vec()).await?;
                    }
                }
            }
            message = channel.wait() => {
                match message {
                    Some(ChannelMsg::Data { data }) => {
                        write_broker_frame(&mut stream, FRAME_STDOUT, &data).await?;
                    }
                    Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                        write_broker_frame(&mut stream, FRAME_STDERR, &data).await?;
                    }
                    Some(ChannelMsg::ExitStatus { exit_status: status }) => {
                        exit_status = i32::try_from(status).unwrap_or(255);
                    }
                    Some(ChannelMsg::Close) | None => {
                        write_broker_frame(&mut stream, FRAME_EXIT, &exit_status.to_be_bytes()).await?;
                        return Ok(());
                    }
                    Some(ChannelMsg::Eof)
                    | Some(ChannelMsg::ExtendedData { .. })
                    | Some(ChannelMsg::ExitSignal { .. })
                    | Some(ChannelMsg::WindowAdjusted { .. })
                    | Some(ChannelMsg::Success)
                    | Some(ChannelMsg::Failure)
                    | Some(ChannelMsg::XonXoff { .. })
                    | Some(ChannelMsg::Open { .. })
                    | Some(ChannelMsg::RequestPty { .. })
                    | Some(ChannelMsg::RequestShell { .. })
                    | Some(ChannelMsg::Exec { .. })
                    | Some(ChannelMsg::Signal { .. })
                    | Some(ChannelMsg::RequestSubsystem { .. })
                    | Some(ChannelMsg::RequestX11 { .. })
                    | Some(ChannelMsg::SetEnv { .. })
                    | Some(ChannelMsg::WindowChange { .. })
                    | Some(ChannelMsg::AgentForward { .. })
                    | Some(ChannelMsg::OpenFailure(_)) => {}
                    // ChannelMsg 是 non_exhaustive；后续新增消息保持忽略语义。
                    Some(_) => {}
                }
            }
        }
    }
}

async fn bridge_interactive_channel(
    mut channel: russh::Channel<client::Msg>,
    escape_char: Option<u8>,
) -> Result<i32> {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Option<Vec<u8>>>();
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 8192];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    let _ = input_tx.send(None);
                    return;
                }
                Ok(read) => {
                    if input_tx.send(Some(buffer[..read].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
    });

    let mut escape_filter = SshEscapeFilter::new(escape_char);
    let mut dimensions = terminal_dimensions();
    let mut resize = tokio::time::interval(RESIZE_POLL_INTERVAL);
    let mut exit_status = 255_i32;
    let mut stdin_eof = false;
    loop {
        tokio::select! {
            input = input_rx.recv(), if !stdin_eof => {
                match input.flatten() {
                    Some(input) => {
                        let output = escape_filter.push(&input);
                        if !output.bytes.is_empty() {
                            channel.data_bytes(output.bytes).await?;
                        }
                        if output.show_help {
                            print_ssh_escape_help()?;
                        }
                        if output.disconnect {
                            channel.close().await?;
                            return Ok(0);
                        }
                    }
                    None => {
                        stdin_eof = true;
                        let remaining = escape_filter.finish();
                        if !remaining.is_empty() {
                            channel.data_bytes(remaining).await?;
                        }
                        channel.eof().await?;
                    }
                }
            }
            _ = resize.tick() => {
                let next = terminal_dimensions();
                if next != dimensions {
                    dimensions = next;
                    channel.window_change(next.0, next.1, 0, 0).await?;
                }
            }
            message = channel.wait() => {
                match message {
                    Some(ChannelMsg::Data { data }) => write_terminal_output(false, &data)?,
                    Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                        write_terminal_output(true, &data)?;
                    }
                    Some(ChannelMsg::ExitStatus { exit_status: status }) => {
                        exit_status = i32::try_from(status).unwrap_or(255);
                    }
                    Some(ChannelMsg::Close) | None => return Ok(exit_status),
                    Some(ChannelMsg::Eof)
                    | Some(ChannelMsg::ExtendedData { .. })
                    | Some(ChannelMsg::ExitSignal { .. })
                    | Some(ChannelMsg::WindowAdjusted { .. })
                    | Some(ChannelMsg::Success)
                    | Some(ChannelMsg::Failure)
                    | Some(ChannelMsg::XonXoff { .. })
                    | Some(ChannelMsg::Open { .. })
                    | Some(ChannelMsg::RequestPty { .. })
                    | Some(ChannelMsg::RequestShell { .. })
                    | Some(ChannelMsg::Exec { .. })
                    | Some(ChannelMsg::Signal { .. })
                    | Some(ChannelMsg::RequestSubsystem { .. })
                    | Some(ChannelMsg::RequestX11 { .. })
                    | Some(ChannelMsg::SetEnv { .. })
                    | Some(ChannelMsg::WindowChange { .. })
                    | Some(ChannelMsg::AgentForward { .. })
                    | Some(ChannelMsg::OpenFailure(_)) => {}
                    // ChannelMsg 是 non_exhaustive；后续新增消息保持忽略语义。
                    Some(_) => {}
                }
            }
        }
    }
}

fn write_terminal_output(stderr: bool, bytes: &[u8]) -> Result<()> {
    if stderr {
        let mut output = io::stderr().lock();
        output.write_all(bytes)?;
        output.flush()?;
    } else {
        let mut output = io::stdout().lock();
        output.write_all(bytes)?;
        output.flush()?;
    }
    Ok(())
}

async fn read_broker_header(stream: &mut TokioTcpStream) -> Result<BrokerRequest> {
    let length = stream.read_u32().await? as usize;
    if length > MAX_BROKER_HEADER_BYTES {
        bail!("SSH broker request is too large");
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload).context("invalid SSH broker request")
}

async fn write_broker_frame(stream: &mut TokioTcpStream, kind: u8, payload: &[u8]) -> Result<()> {
    stream.write_u8(kind).await?;
    stream.write_u32(payload.len() as u32).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
#[path = "russh_backend_tests.rs"]
mod tests;
