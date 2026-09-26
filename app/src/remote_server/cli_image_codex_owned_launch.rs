//! 显式启动的每终端私有 Codex app-server 与 TUI；继承用户配置，不修改审批策略。
//! 子进程在 exec 前保存生存期，监督者退出后仍可精确识别并回收自身服务端。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use command::blocking::Command;
use command::unix::CommandExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use super::cli_image_codex_owned_protocol::{
    MAX_BODY_BYTES, Owner, REMOTE_CODEX_COMMAND, Reply, Scope, Ticket,
};
use super::cli_image_codex_owned_socket::{SocketLease, SocketStamp};
use super::cli_image_native_process::{Configuration, peer_pid, process};
use crate::terminal::{CLIAgent, cli_agent::discover_cli_agent_executable};

#[path = "cli_image_codex_owned_process.rs"]
mod process_tokens;
use process_tokens::Token;

const CHILD_COMMAND: &str = "--infinishell-remote-owned-codex-child";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservation {
    version: u32,
    id: Uuid,
    key_sha256: String,
    host: String,
    terminal_session: u64,
    cwd: PathBuf,
    app: PathBuf,
    app_sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    ticket: Uuid,
    reservation_sha256: String,
    executable: PathBuf,
    cwd: PathBuf,
    codex_home: PathBuf,
    app: PathBuf,
    wrapper: Token,
    shell: Token,
    group: i32,
    tty_device: u64,
    socket: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Birth {
    manifest_sha256: String,
    process: Token,
}

pub(super) struct TicketStore {
    root: PathBuf,
    identity: (u64, u64),
}
pub(super) struct Guard {
    file: File,
    directory: PathBuf,
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl TicketStore {
    pub(super) fn new(parent: &Path) -> io::Result<Self> {
        private_metadata(parent, true)?;
        let root = parent.join("codex-owned-remote-v1");
        match fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => File::open(parent)?.sync_all()?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = private_metadata(&root, true)?;
        Ok(Self {
            root,
            identity: (metadata.dev(), metadata.ino()),
        })
    }
    fn directory(&self, id: Uuid) -> io::Result<PathBuf> {
        let metadata = private_metadata(&self.root, true)?;
        if id.is_nil() || (metadata.dev(), metadata.ino()) != self.identity {
            return Err(invalid());
        }
        Ok(self.root.join(id.to_string()))
    }
    pub(super) fn reserve(&self, scope: &Scope, ticket: &Ticket, cwd: &str) -> io::Result<Reply> {
        if ticket.key.is_nil() {
            return Err(invalid());
        }
        let cwd = PathBuf::from(cwd);
        if !cwd.is_absolute() || !cwd.is_dir() || cwd.canonicalize()? != cwd {
            return Err(invalid());
        }
        let directory = self.directory(ticket.id)?;
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        File::open(&self.root)?.sync_all()?;
        let app = std::env::current_exe()?.canonicalize()?;
        let reservation = Reservation {
            version: 1,
            id: ticket.id,
            key_sha256: digest(ticket.key.as_bytes()),
            host: scope.host.clone(),
            terminal_session: scope.terminal_session,
            cwd,
            app_sha256: executable_digest(&app)?,
            app,
        };
        let path = directory.join("ticket.json");
        write_new(&path, &reservation)?;
        Ok(Reply::Reserved {
            ticket: ticket.clone(),
            argv: vec![
                reservation.app.to_str().ok_or_else(invalid)?.into(),
                REMOTE_CODEX_COMMAND.into(),
                path.to_str().ok_or_else(invalid)?.into(),
                digest(&read_private(&path)?),
            ],
        })
    }
    pub(super) fn lock(&self, scope: &Scope, ticket: &Ticket) -> io::Result<Guard> {
        let guard = lock_directory(&self.directory(ticket.id)?)?;
        let reservation: Reservation = read_json(&guard.directory.join("ticket.json"))?;
        if reservation.version != 1
            || reservation.id != ticket.id
            || ticket.key.is_nil()
            || reservation.key_sha256 != digest(ticket.key.as_bytes())
            || reservation.host != scope.host
            || reservation.terminal_session != scope.terminal_session
        {
            return Err(invalid());
        }
        Ok(guard)
    }
    pub(super) fn image_owner(
        &self,
        scope: &Scope,
        id: Uuid,
        hash: &str,
    ) -> io::Result<OwnedImageLease> {
        let guard = lock_directory(&self.directory(id)?)?;
        let reservation: Reservation = read_json(&guard.directory.join("ticket.json"))?;
        if reservation.id != id
            || reservation.host != scope.host
            || reservation.terminal_session != scope.terminal_session
        {
            return Err(invalid());
        }
        OwnedImageLease::capture(guard, hash)
    }
    pub(super) fn reap(&self) {
        if self.directory(Uuid::new_v4()).is_err() {
            return;
        }
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.take(4096).flatten() {
            if Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err() {
                continue;
            }
            if let Ok(guard) = try_lock_directory(&entry.path()) {
                let _ = guard.reap();
            }
        }
    }
}

impl Guard {
    pub(super) fn status(&self, ticket: &Ticket) -> io::Result<Reply> {
        let _ = self.reap();
        let phase = if self.directory.join("released.json").exists() {
            "released"
        } else if self.directory.join("cancelled.json").exists() {
            "cancelled"
        } else if self.directory.join("failed.json").exists() {
            "failed"
        } else if self.directory.join("manifest.json").exists() {
            "dispatched_unknown"
        } else {
            "reserved"
        };
        let owner = self.owner().ok();
        Ok(Reply::Launch {
            ticket: ticket.clone(),
            phase: if owner.is_some() { "active" } else { phase }.into(),
            owner,
        })
    }
    fn owner(&self) -> io::Result<Owner> {
        let bytes = read_private(&self.directory.join("manifest.json"))?;
        let manifest = bound_manifest(&self.directory, &bytes)?;
        let hash = digest(&bytes);
        let tui = child(&self.directory, &manifest, &hash, "tui")?;
        let server = child(&self.directory, &manifest, &hash, "server")?;
        let current = process(tui.pid, Configuration::Codex)?;
        if current.tty != manifest.tty_device
            || current.group != manifest.group
            || current.foreground_group != manifest.group
        {
            return Err(invalid());
        }
        bound_socket(&self.directory, &manifest, &server)?;
        Ok(Owner {
            ticket_id: manifest.ticket,
            manifest_sha256: hash,
            socket_path: manifest.socket.to_str().ok_or_else(invalid)?.into(),
            tui_pid: tui.pid,
            server_pid: server.pid,
            tty_device: manifest.tty_device,
        })
    }
    pub(super) fn cancel(&self, ticket: &Ticket) -> io::Result<Reply> {
        if !self.directory.join("entered.json").exists()
            && !self.directory.join("cancelled.json").exists()
        {
            write_new(&self.directory.join("cancelled.json"), &true)?;
        }
        // 已启动的 TUI 由用户原生退出；取消富输入不杀死当前对话。
        self.status(ticket)
    }
    fn reap(&self) -> io::Result<()> {
        if self.directory.join("released.json").exists() {
            return Ok(());
        }
        let bytes = read_private(&self.directory.join("manifest.json"))?;
        let manifest = bound_manifest(&self.directory, &bytes)?;
        let hash = digest(&bytes);
        let tui = optional_json::<Birth>(&self.directory.join("tui-birth.json"))?;
        let tui_exited = match tui {
            Some(birth) if birth.manifest_sha256 == hash => birth.process.exited()?,
            Some(_) => return Err(invalid()),
            None => {
                let never_started =
                    optional_json::<bool>(&self.directory.join("tui-never-executed.json"))?
                        == Some(true);
                // helper 的 birth 在 exec 前同步落盘；wrapper 真正退出且没有 birth 才证明没有 TUI。
                if !never_started && !manifest.wrapper.exited()? {
                    return Ok(());
                }
                // 同一排他锁保证：尚未写 birth 的 helper 之后会看到 wrapper 已退出，不能再 exec。
                true
            }
        };
        if !tui_exited {
            return Ok(());
        }
        let Some(server_birth) = optional_json::<Birth>(&self.directory.join("server-birth.json"))?
        else {
            return Ok(());
        };
        if server_birth.manifest_sha256 != hash {
            return Err(invalid());
        }
        if !server_birth.process.exited()? {
            let server = child(&self.directory, &manifest, &hash, "server")?;
            // 原生退出会自行删除物理 socket；路径被替换时不触发该清理。
            bound_socket(&self.directory, &manifest, &server)?.validate()?;
            server.terminate()?;
            return Ok(());
        }
        write_new(&self.directory.join("released.json"), &true)
    }
}

pub(super) struct OwnedImageLease {
    guard: Guard,
    manifest: Manifest,
    hash: String,
    tui: Token,
    server: Token,
    socket: SocketLease,
    executable: File,
    executable_identity: (u64, u64, u64, i64, i64),
}
impl OwnedImageLease {
    fn capture(guard: Guard, expected_hash: &str) -> io::Result<Self> {
        let bytes = read_private(&guard.directory.join("manifest.json"))?;
        if digest(&bytes) != expected_hash || guard.directory.join("released.json").exists() {
            return Err(invalid());
        }
        let manifest = bound_manifest(&guard.directory, &bytes)?;
        let tui = child(&guard.directory, &manifest, expected_hash, "tui")?;
        let server = child(&guard.directory, &manifest, expected_hash, "server")?;
        let executable = fixed_executable(&manifest.executable)?;
        let executable_identity = image_identity(&executable.metadata()?);
        let socket = bound_socket(&guard.directory, &manifest, &server)?;
        Ok(Self {
            guard,
            manifest,
            hash: expected_hash.into(),
            tui,
            server,
            socket,
            executable,
            executable_identity,
        })
    }
    pub(super) fn socket_path(&self) -> &Path {
        self.socket.requested()
    }
    pub(super) fn cwd(&self) -> &Path {
        &self.manifest.cwd
    }
    pub(super) fn codex_home(&self) -> &Path {
        &self.manifest.codex_home
    }
    pub(super) fn tui_pid(&self) -> i32 {
        self.tui.pid
    }
    pub(super) fn server_pid(&self) -> i32 {
        self.server.pid
    }
    pub(super) fn tty_device(&self) -> u64 {
        self.manifest.tty_device
    }
    pub(super) fn validate(&self, stream: &UnixStream) -> io::Result<()> {
        self.socket.validate()?;
        self.tui.validate()?;
        self.server.validate()?;
        if peer_pid(stream)? != self.server.pid
            || digest(&read_private(&self.guard.directory.join("manifest.json"))?) != self.hash
            || image_identity(&self.executable.metadata()?) != self.executable_identity
            || image_identity(&fs::metadata(&self.manifest.executable)?) != self.executable_identity
        {
            return Err(invalid());
        }
        let tui = process(self.tui.pid, Configuration::Codex)?;
        let server = process(self.server.pid, Configuration::Codex)?;
        if tui.tty != self.manifest.tty_device
            || tui.group != self.manifest.group
            || tui.foreground_group != tui.group
            || tui.config_home != self.manifest.codex_home
            || server.config_home != self.manifest.codex_home
            || !arguments_match(&tui.arguments, &self.manifest, "tui")
            || !arguments_match(&server.arguments, &self.manifest, "server")
        {
            return Err(invalid());
        }
        Ok(())
    }
}

/// 主入口需在 GUI、线程池与信号处理器初始化之前调用。
pub(crate) fn run_from_args() -> Option<io::Result<()>> {
    let mut args = std::env::args_os().skip(1);
    let command = args.next()?;
    if command != REMOTE_CODEX_COMMAND && command != CHILD_COMMAND {
        return None;
    }
    Some((|| {
        let path = PathBuf::from(args.next().ok_or_else(invalid)?);
        let hash = args
            .next()
            .and_then(|value| value.into_string().ok())
            .ok_or_else(invalid)?;
        if command == CHILD_COMMAND {
            let role = args
                .next()
                .and_then(|value| value.into_string().ok())
                .ok_or_else(invalid)?;
            if args.next().is_some() {
                return Err(invalid());
            }
            return exec_child(&path, &hash, &role);
        }
        if args.next().is_some() {
            return Err(invalid());
        }
        supervise(&path, &hash)
    })())
}

fn supervise(path: &Path, expected_hash: &str) -> io::Result<()> {
    let directory = path.parent().ok_or_else(invalid)?;
    let guard = lock_directory(directory)?;
    let bytes = read_private(path)?;
    if digest(&bytes) != expected_hash
        || path.file_name().and_then(|name| name.to_str()) != Some("ticket.json")
        || directory.join("entered.json").exists()
        || directory.join("cancelled.json").exists()
    {
        return Err(invalid());
    }
    let reservation: Reservation = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if reservation.version != 1
        || std::env::current_exe()?.canonicalize()? != reservation.app
        || executable_digest(&reservation.app)? != reservation.app_sha256
    {
        return Err(invalid());
    }
    let (shell, tty_device, group) = current_terminal()?;
    let executable = resolve_executable()?;
    let binary = fixed_executable(&executable)?;
    let wrapper = Token::capture(unsafe { libc::getpid() })?;
    let codex_home = process(wrapper.pid, Configuration::Codex)?.config_home;
    let manifest = Manifest {
        version: 1,
        ticket: reservation.id,
        reservation_sha256: expected_hash.into(),
        executable,
        cwd: reservation.cwd,
        codex_home,
        app: reservation.app,
        wrapper,
        shell,
        group,
        tty_device,
        socket: directory.join("control.sock"),
    };
    write_new(&directory.join("entered.json"), &manifest.wrapper)?;
    let path = directory.join("manifest.json");
    write_new(&path, &manifest)?;
    let hash = digest(&read_private(&path)?);
    drop(guard);
    // Ctrl-C/退出键仍交给同一前台组内的原生 TUI，不能先结束监督者并把 shell 提前放回前台。
    for signal in [libc::SIGINT, libc::SIGQUIT] {
        if unsafe { libc::signal(signal, libc::SIG_IGN) } == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
    }
    // helper 在 exec 前持久化自身生存期；即使监督者崩溃，也不存在未登记的原生 daemon。
    let mut server_command = helper_command(&manifest, &path, &hash, "server");
    server_command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        server_command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut server = server_command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let ready = loop {
        if let Ok(socket) = SocketLease::capture(&manifest.socket) {
            if let Ok(stream) = UnixStream::connect(socket.requested()) {
                if peer_pid(&stream).ok() == Some(server.id() as i32)
                    && child(directory, &manifest, &hash, "server").is_ok()
                {
                    break Ok(());
                }
            }
        }
        if server.try_wait()?.is_some() || Instant::now() >= deadline {
            break Err(invalid());
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    if ready.is_err() {
        let guard = lock_directory(directory)?;
        let _ = write_new(&directory.join("tui-never-executed.json"), &true);
        let _ = write_new(&directory.join("failed.json"), &true);
        let _ = guard.reap();
        return ready;
    }
    let server_identity = child(directory, &manifest, &hash, "server")?;
    bound_socket(directory, &manifest, &server_identity)?;
    write_new(&directory.join("tui-spawn-unknown.json"), &true)?;
    let mut tui = match helper_command(&manifest, &path, &hash, "tui").spawn() {
        Ok(child) => child,
        Err(error) => {
            let guard = lock_directory(directory)?;
            let _ = write_new(&directory.join("tui-never-executed.json"), &true);
            let _ = guard.reap();
            return Err(error);
        }
    };
    let result = tui.wait();
    // 直接子进程 wait 是真实退出证据；退出后只终止已固定身份的本次 app-server。
    if result.is_ok() {
        let guard = lock_directory(directory)?;
        guard.reap()?;
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until && server.try_wait()?.is_none() {
            std::thread::sleep(Duration::from_millis(30));
        }
        let _ = guard.reap();
    }
    drop(binary);
    result.map(|_| ())
}

fn helper_command(manifest: &Manifest, path: &Path, hash: &str, role: &str) -> Command {
    let mut command = Command::new(&manifest.app);
    command
        .arg(CHILD_COMMAND)
        .arg(path)
        .arg(hash)
        .arg(role)
        .current_dir(&manifest.cwd)
        .env("CODEX_HOME", &manifest.codex_home);
    unsafe {
        command.pre_exec(|| {
            for signal in [libc::SIGINT, libc::SIGQUIT] {
                libc::signal(signal, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    command
}

fn exec_child(path: &Path, hash: &str, role: &str) -> io::Result<()> {
    if role != "server" && role != "tui" {
        return Err(invalid());
    }
    let directory = path.parent().ok_or_else(invalid)?;
    let guard = lock_directory(directory)?;
    let bytes = read_private(path)?;
    if digest(&bytes) != hash
        || path.file_name().and_then(|name| name.to_str()) != Some("manifest.json")
    {
        return Err(invalid());
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let reservation: Reservation = read_json(&directory.join("ticket.json"))?;
    if manifest.version != 1
        || manifest.reservation_sha256 != digest(&read_private(&directory.join("ticket.json"))?)
        || manifest.ticket != reservation.id
        || std::env::current_exe()?.canonicalize()? != reservation.app
        || executable_digest(&reservation.app)? != reservation.app_sha256
        || unsafe { libc::getppid() } != manifest.wrapper.pid
    {
        return Err(invalid());
    }
    manifest.wrapper.validate()?;
    let binary = fixed_executable(&manifest.executable)?;
    if role == "tui" {
        if directory.join("tui-never-executed.json").exists() {
            return Err(invalid());
        }
        let (_, device, group) = current_terminal()?;
        if device != manifest.tty_device || group != manifest.group {
            return Err(invalid());
        }
    }
    let birth = Birth {
        manifest_sha256: hash.into(),
        process: Token::capture(unsafe { libc::getpid() })?,
    };
    write_new(&directory.join(format!("{role}-birth.json")), &birth)?;
    drop(guard);
    let mut command = Command::new(&manifest.executable);
    command
        .args(arguments(&manifest, role))
        .current_dir(&manifest.cwd)
        .env("CODEX_HOME", &manifest.codex_home);
    // 仅禁止此私有进程注册额外远程控制入口；queue、认证与审批配置仍由原生读取。
    if role == "server" {
        command.env("CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED", "1");
    }
    let error = command.exec();
    drop(binary);
    let _ = write_new(&directory.join(format!("{role}-exec-failed.json")), &true);
    Err(error)
}

fn arguments(manifest: &Manifest, role: &str) -> Vec<String> {
    let endpoint = format!("unix://{}", manifest.socket.display());
    if role == "server" {
        vec!["app-server".into(), "--listen".into(), endpoint]
    } else {
        vec![
            "--remote".into(),
            endpoint,
            "--cd".into(),
            manifest.cwd.to_string_lossy().into_owned(),
        ]
    }
}
fn arguments_match(actual: &[Vec<u8>], manifest: &Manifest, role: &str) -> bool {
    let expected = arguments(manifest, role);
    actual.len() == expected.len() + 1
        && actual[1..]
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual == expected.as_bytes())
}
fn child(directory: &Path, manifest: &Manifest, hash: &str, role: &str) -> io::Result<Token> {
    let birth: Birth = read_json(&directory.join(format!("{role}-birth.json")))?;
    let token = Token::capture(birth.process.pid)?;
    let current = process(token.pid, Configuration::Codex)?;
    let image = fs::metadata(&manifest.executable)?;
    if birth.manifest_sha256 != hash
        || !birth.process.same_lifetime(&token)
        || current.executable_identity != (image.dev(), image.ino())
        || current.config_home != manifest.codex_home
        || !arguments_match(&current.arguments, manifest, role)
    {
        return Err(invalid());
    }
    Ok(token)
}

fn bound_manifest(directory: &Path, bytes: &[u8]) -> io::Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let reserved = read_private(&directory.join("ticket.json"))?;
    let reservation: Reservation = serde_json::from_slice(&reserved).map_err(|_| invalid())?;
    if manifest.version != 1
        || reservation.version != 1
        || manifest.ticket != reservation.id
        || manifest.reservation_sha256 != digest(&reserved)
        || manifest.cwd != reservation.cwd
        || manifest.app != reservation.app
        || manifest.socket != directory.join("control.sock")
    {
        return Err(invalid());
    }
    Ok(manifest)
}

fn bound_socket(directory: &Path, manifest: &Manifest, server: &Token) -> io::Result<SocketLease> {
    let socket = SocketLease::capture(&manifest.socket)?;
    let path = directory.join("socket.json");
    if let Some(stamp) = optional_json::<SocketStamp>(&path)? {
        socket.validate_stamp(&stamp)?;
    } else {
        // 首次记录必须连接本次已登记的原生子进程；之后不以现存替换对象刷新身份。
        let stream = UnixStream::connect(socket.requested())?;
        server.validate()?;
        if peer_pid(&stream)? != server.pid {
            return Err(invalid());
        }
        write_new(&path, &socket.stamp()?)?;
    }
    Ok(socket)
}

fn current_terminal() -> io::Result<(Token, u64, i32)> {
    let parent = unsafe { libc::getppid() };
    let group = unsafe { libc::getpgrp() };
    let session = unsafe { libc::getsid(0) };
    if parent <= 1
        || group <= 0
        || session <= 0
        || unsafe { libc::getsid(parent) } != session
        || unsafe { libc::tcgetpgrp(0) } != group
    {
        return Err(invalid());
    }
    let mut device = None;
    for descriptor in [0, 1, 2] {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::isatty(descriptor) } != 1
            || unsafe { libc::fstat(descriptor, stat.as_mut_ptr()) } != 0
        {
            return Err(invalid());
        }
        let stat = unsafe { stat.assume_init() };
        let current = stat.st_rdev as u64;
        if stat.st_mode & libc::S_IFMT != libc::S_IFCHR
            || current == 0
            || device.is_some_and(|value| value != current)
        {
            return Err(invalid());
        }
        device = Some(current);
    }
    let parent_token = Token::capture(parent)?;
    let device = device.ok_or_else(invalid)?;
    #[cfg(target_os = "linux")]
    let (parent_uid, parent_tty) = {
        let shell = command::managed::LinuxProcessHandle::capture(parent)?.snapshot()?;
        (shell.identity.uid, shell.tty_device)
    };
    #[cfg(target_os = "macos")]
    let (parent_uid, parent_tty) = {
        let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
        if unsafe {
            libc::proc_pidinfo(
                parent,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                std::mem::size_of_val(&info) as i32,
            )
        } != std::mem::size_of_val(&info) as i32
        {
            return Err(invalid());
        }
        (info.pbi_uid, info.e_tdev as u64)
    };
    if parent_uid != unsafe { libc::geteuid() } || parent_tty != device {
        return Err(invalid());
    }
    parent_token.validate()?;
    Ok((parent_token, device, group))
}

fn resolve_executable() -> io::Result<PathBuf> {
    let public = discover_cli_agent_executable(CLIAgent::Codex)
        .ok_or_else(invalid)?
        .canonicalize()?;
    if public.file_name().is_none_or(|name| name != "codex.js") {
        return Ok(public);
    }
    // 只解析固定 npm launcher 的公开布局，不执行 JavaScript，也不从可变配置猜原生入口。
    let package = public.parent().and_then(Path::parent).ok_or_else(invalid)?;
    if public != package.join("bin/codex.js")
        || executable_digest(&public)?
            != "61b0194f3bb6534439c8d26a3ed57d0805f84b884588b761795323eeb92fcf70"
        || executable_digest(&package.join("package.json"))?
            != "c3f16464dca0fe1269b17d02fe0997d1ca3a241c3a61da3d89ec13def0a66c6e"
    {
        return Err(invalid());
    }
    #[cfg(target_os = "macos")]
    let (platform, triple) = ("darwin-arm64", "aarch64-apple-darwin");
    #[cfg(target_os = "linux")]
    let (platform, triple) = ("linux-x64", "x86_64-unknown-linux-musl");
    let relative = PathBuf::from(format!("vendor/{triple}/bin/codex"));
    let roots = [
        package.join(format!("node_modules/@openai/codex-{platform}")),
        package
            .parent()
            .ok_or_else(invalid)?
            .join(format!("codex-{platform}")),
    ];
    let mut found = Vec::new();
    for root in roots {
        let Ok(metadata) = fs::read(root.join("package.json")) else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_slice(&metadata).map_err(|_| invalid())?;
        if value["name"] != "@openai/codex" || value["version"] != format!("0.156.1-{platform}") {
            return Err(invalid());
        }
        let native = root.join(&relative).canonicalize()?;
        if !found.contains(&native) {
            found.push(native);
        }
    }
    if found.len() != 1 {
        return Err(invalid());
    }
    Ok(found.remove(0))
}

fn fixed_executable(path: &Path) -> io::Result<File> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.mode() & 0o022 != 0 {
        return Err(invalid());
    }
    let packages: serde_json::Value = serde_json::from_str(include_str!(
        "../../../script/cli-agent-parity/codex_0156_package_manifest.json"
    ))
    .map_err(|_| invalid())?;
    #[cfg(target_os = "macos")]
    let platform = "macos-arm64";
    #[cfg(target_os = "linux")]
    let platform = "linux-x64";
    let expected = &packages["packages"][platform]["files"]["bin/codex"];
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = format!("{:x}", digest.finalize());
    if packages["version"] != "0.156.1"
        || expected[0].as_u64() != Some(metadata.len())
        || expected[1].as_str() != Some(actual.as_str())
        || image_identity(&metadata) != image_identity(&file.metadata()?)
        || image_identity(&metadata) != image_identity(&fs::metadata(path)?)
    {
        return Err(invalid());
    }
    Ok(file)
}
fn image_identity(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
    )
}

fn lock_directory(directory: &Path) -> io::Result<Guard> {
    open_lock(directory, false)
}
fn try_lock_directory(directory: &Path) -> io::Result<Guard> {
    open_lock(directory, true)
}
fn open_lock(directory: &Path, nonblocking: bool) -> io::Result<Guard> {
    private_metadata(directory, true)?;
    let path = directory.join("ticket.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    let metadata = private_metadata(&path, false)?;
    let opened = file.metadata()?;
    if (metadata.dev(), metadata.ino()) != (opened.dev(), opened.ino()) {
        return Err(invalid());
    }
    if nonblocking {
        file.try_lock().map_err(io::Error::other)?;
    } else {
        file.lock()?;
    }
    Ok(Guard {
        file,
        directory: directory.into(),
    })
}
pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn executable_digest(path: &Path) -> io::Result<String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
pub(super) fn private_metadata(path: &Path, directory: bool) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != if directory { 0o700 } else { 0o600 }
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file() || metadata.nlink() != 1
        }
    {
        return Err(invalid());
    }
    Ok(metadata)
}
pub(super) fn read_private(path: &Path) -> io::Result<Vec<u8>> {
    let before = private_metadata(path, false)?;
    if before.len() > MAX_BODY_BYTES as u64 {
        return Err(invalid());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let opened = file.metadata()?;
    if (before.dev(), before.ino()) != (opened.dev(), opened.ino()) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}
pub(super) fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&read_private(path)?).map_err(|_| invalid())
}
fn optional_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match read_json(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
pub(super) fn write_new<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(invalid());
    }
    private_metadata(path.parent().ok_or_else(invalid)?, true)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    File::open(path.parent().ok_or_else(invalid)?)?.sync_all()
}
pub(super) fn invalid() -> io::Error {
    io::Error::other("远端 Codex 私有启动身份或状态无效")
}
