//! 远端启动凭据先落盘；真正的控制终端身份只在当前 shell 执行的早期入口捕获。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::cli_image_grok_protocol::{REMOTE_GROK_COMMAND, Reply, Scope, Ticket};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent::discover_cli_agent_executable;
use crate::terminal::cli_agent_sessions::grok_owned_launch::{
    GrokOwnedLaunch, GrokOwnedPty, exec_owned_from_manifest,
};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Reservation {
    version: u32,
    id: Uuid,
    key_sha256: String,
    host: String,
    terminal_session: u64,
    cwd: PathBuf,
    app_executable: PathBuf,
    app_sha256: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct LaunchLink {
    pub manifest: PathBuf,
    pub sha256: String,
    pub native_session: Uuid,
}

#[derive(Clone)]
pub(super) struct TicketStore {
    root: PathBuf,
    identity: (u64, u64),
}

pub(super) struct TicketGuard {
    file: File,
    pub directory: PathBuf,
}

impl Drop for TicketGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl TicketStore {
    pub(super) fn new(parent: &Path) -> io::Result<Self> {
        private_metadata(parent, true)?;
        let root = parent.join("grok-owned-remote-v1");
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

    fn directory(&self, ticket: &Ticket) -> io::Result<PathBuf> {
        let metadata = private_metadata(&self.root, true)?;
        if (metadata.dev(), metadata.ino()) != self.identity
            || ticket.id.is_nil()
            || ticket.key.is_nil()
        {
            return Err(invalid());
        }
        Ok(self.root.join(ticket.id.to_string()))
    }

    pub(super) fn reserve(&self, scope: &Scope, ticket: &Ticket, cwd: &str) -> io::Result<Reply> {
        let cwd = PathBuf::from(cwd);
        if !cwd.is_absolute() || !cwd.is_dir() || cwd.canonicalize()? != cwd {
            return Err(invalid());
        }
        let directory = self.directory(ticket)?;
        // 重复 Reserve 绝不返还可再次执行的启动命令；丢失响应只允许查询原凭据。
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        File::open(&self.root)?.sync_all()?;
        let app_executable = std::env::current_exe()?.canonicalize()?;
        let reservation = Reservation {
            version: 1,
            id: ticket.id,
            key_sha256: digest(ticket.key.as_bytes()),
            host: scope.host.clone(),
            terminal_session: scope.terminal_session,
            cwd,
            app_sha256: executable_digest(&app_executable)?,
            app_executable,
        };
        write_new(&directory.join("ticket.json"), &reservation)?;
        let bytes = read_private(&directory.join("ticket.json"))?;
        Ok(Reply::Reserved {
            ticket: ticket.clone(),
            argv: vec![
                reservation
                    .app_executable
                    .to_str()
                    .ok_or_else(invalid)?
                    .to_owned(),
                REMOTE_GROK_COMMAND.into(),
                directory
                    .join("ticket.json")
                    .to_str()
                    .ok_or_else(invalid)?
                    .to_owned(),
                digest(&bytes),
            ],
        })
    }

    /// 应用或 daemon 重启后继续处理已明确请求的清理，不从计时或目录消失推断会话结束。
    pub(super) fn reap_requested(&self) -> io::Result<Vec<PathBuf>> {
        let metadata = private_metadata(&self.root, true)?;
        if (metadata.dev(), metadata.ino()) != self.identity {
            return Err(invalid());
        }
        let mut released = Vec::new();
        for entry in fs::read_dir(&self.root)?.take(4096) {
            let entry = entry?;
            let Ok(id) = Uuid::parse_str(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            let directory = entry.path();
            let Ok(guard) = lock_directory(&directory) else {
                continue;
            };
            let Ok(reservation) = read_json::<Reservation>(&directory.join("ticket.json")) else {
                continue;
            };
            if reservation.version != 1 || reservation.id != id {
                continue;
            }
            // 即使插件缺失也捕获私有进程；不由插件是否上线决定退出清理能力。
            if let Ok(mut launch) = guard.launch() {
                let _ = launch.capture_owned_processes();
            }
            if !directory.join("cleanup-requested.json").exists() {
                continue;
            }
            if guard.reap_exited().is_ok() && directory.join("released.json").exists() {
                released.push(directory);
            }
        }
        Ok(released)
    }

    pub(super) fn lock(&self, scope: &Scope, ticket: &Ticket) -> io::Result<TicketGuard> {
        let directory = self.directory(ticket)?;
        let guard = lock_directory(&directory)?;
        let reservation: Reservation = read_json(&directory.join("ticket.json"))?;
        if reservation.version != 1
            || reservation.id != ticket.id
            || reservation.key_sha256 != digest(ticket.key.as_bytes())
            || reservation.host != scope.host
            || reservation.terminal_session != scope.terminal_session
        {
            return Err(invalid());
        }
        Ok(guard)
    }
}

impl TicketGuard {
    pub(super) fn launch(&self) -> io::Result<GrokOwnedLaunch> {
        if self.directory.join("cancelled.json").exists()
            || self.directory.join("released.json").exists()
        {
            return Err(invalid());
        }
        let link: LaunchLink = read_json(&self.directory.join("launch-link.json"))?;
        let launch = GrokOwnedLaunch::restore(&link.manifest, &link.sha256)?;
        if !launch.was_dispatched() || launch.session_id() != link.native_session {
            return Err(invalid());
        }
        Ok(launch)
    }

    pub(super) fn status(&self, ticket: &Ticket) -> io::Result<Reply> {
        let link = optional_json::<LaunchLink>(&self.directory.join("launch-link.json"))?;
        if link.is_some() && !self.directory.join("released.json").exists() {
            // 捕获身份不发送输入；后续关闭时不能只剩无法安全处理的历史 PID。
            if let Ok(mut launch) = self.launch() {
                let _ = launch.capture_owned_processes();
            }
        }
        let phase = if self.directory.join("released.json").exists() {
            "released"
        } else if self.directory.join("cancelled.json").exists() {
            "cancelled"
        } else if self.directory.join("failed.json").exists() {
            "failed"
        } else if link.is_some() {
            "dispatched_unknown"
        } else if self.directory.join("entered.json").exists() {
            "entered_unknown"
        } else {
            "reserved"
        };
        Ok(Reply::Launch {
            ticket: ticket.clone(),
            native_session: link.as_ref().map(|value| value.native_session),
            manifest_sha256: link.map(|value| value.sha256),
            phase: phase.into(),
        })
    }

    pub(super) fn cancel(&self, ticket: &Ticket) -> io::Result<Reply> {
        if !self.directory.join("entered.json").exists() {
            if !self.directory.join("cancelled.json").exists() {
                write_new(&self.directory.join("cancelled.json"), &true)?;
            }
        } else {
            if !self.directory.join("cleanup-requested.json").exists() {
                write_new(&self.directory.join("cleanup-requested.json"), &true)?;
            }
            let _ = self.reap_exited();
        }
        self.status(ticket)
    }

    fn reap_exited(&self) -> io::Result<()> {
        if self.directory.join("released.json").exists() {
            return Ok(());
        }
        let link: LaunchLink = read_json(&self.directory.join("launch-link.json"))?;
        let mut launch = GrokOwnedLaunch::restore(&link.manifest, &link.sha256)?;
        if launch.session_id() != link.native_session
            || !(launch.was_dispatched() || launch.is_retired())
        {
            return Err(invalid());
        }
        launch.refresh_bound_processes()?;
        // 原生 TUI 仍活跃时不会发信号；只有本次已验证 TUI 退出才处理私有 leader。
        let _ = launch.capture_owned_processes();
        let _ = launch.stop_leader_after_tui_exit();
        launch.release_remote_after_exit()?;
        write_new(&self.directory.join("released.json"), &true)
    }
}

/// 在所有 GUI/线程初始化之前调用；不新建 shell，不从 daemon 猜测 tty，不执行 resume。
pub(crate) fn run_from_args() -> Option<io::Result<()>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(REMOTE_GROK_COMMAND)) {
        return None;
    }
    Some((|| {
        let path = PathBuf::from(args.next().ok_or_else(invalid)?);
        let hash = args
            .next()
            .and_then(|arg| arg.into_string().ok())
            .ok_or_else(invalid)?;
        if args.next().is_some() {
            return Err(invalid());
        }
        let directory = path.parent().ok_or_else(invalid)?;
        if path.file_name().and_then(|value| value.to_str()) != Some("ticket.json") {
            return Err(invalid());
        }
        let guard = lock_directory(directory)?;
        let bytes = read_private(&path)?;
        let ticket: Reservation = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if digest(&bytes) != hash
            || ticket.version != 1
            || ticket.id.is_nil()
            || directory.file_name().and_then(|value| value.to_str())
                != Some(ticket.id.to_string().as_str())
            || ticket.cwd != std::env::current_dir()?.canonicalize()?
            || ticket.app_executable != std::env::current_exe()?.canonicalize()?
            || ticket.app_sha256 != executable_digest(&ticket.app_executable)?
            || directory.join("cancelled.json").exists()
        {
            return Err(invalid());
        }
        // 该记录也覆盖 prepare 中途崩溃；再次执行同一命令永远不能重启原会话。
        write_new(&directory.join("entered.json"), &true)?;
        let prepared = (|| {
            let pty = GrokOwnedPty::from_current_terminal()?;
            let executable = discover_cli_agent_executable(CLIAgent::Grok).ok_or_else(invalid)?;
            let mut launch = GrokOwnedLaunch::prepare(
                &executable,
                &ticket.app_executable,
                &ticket.cwd,
                directory,
                pty,
            )?;
            let link = LaunchLink {
                manifest: launch.manifest_path(),
                sha256: launch.manifest_sha256().into(),
                native_session: launch.session_id(),
            };
            // 凭据与清单必须在不可逆派发前持久化；持票锁使 cancel 无法穿过此边界。
            write_new(&directory.join("launch-link.json"), &link)?;
            let manifest = launch.dispatch_remote_reserved()?;
            Ok::<_, io::Error>(manifest)
        })();
        let manifest = match prepared {
            Ok(manifest) => manifest,
            Err(error) => {
                let _ = write_new(&directory.join("failed.json"), &true);
                return Err(error);
            }
        };
        drop(guard);
        exec_owned_from_manifest(&manifest)
    })())
}

fn lock_directory(directory: &Path) -> io::Result<TicketGuard> {
    private_metadata(directory, true)?;
    let path = directory.join("ticket.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    let metadata = private_metadata(&path, false)?;
    let opened = file.metadata()?;
    if (metadata.dev(), metadata.ino()) != (opened.dev(), opened.ino()) {
        return Err(invalid());
    }
    file.lock()?;
    Ok(TicketGuard {
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
    if !file.metadata()?.is_file() {
        return Err(invalid());
    }
    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
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
    if before.len() > super::cli_image_grok_protocol::MAX_BODY_BYTES as u64 {
        return Err(invalid());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let opened = file.metadata()?;
    if (before.dev(), before.ino()) != (opened.dev(), opened.ino()) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(super::cli_image_grok_protocol::MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > super::cli_image_grok_protocol::MAX_BODY_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}

pub(super) fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&read_private(path)?).map_err(io::Error::other)
}

pub(super) fn optional_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match read_json(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn write_new<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > super::cli_image_grok_protocol::MAX_BODY_BYTES {
        return Err(invalid());
    }
    private_metadata(path.parent().ok_or_else(invalid)?, true)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    File::open(path.parent().ok_or_else(invalid)?)?.sync_all()
}

pub(super) fn invalid() -> io::Error {
    io::Error::other("远端 Grok 私有启动身份或状态无效")
}
