//! 从当前控制终端和内核进程发现既有 Codex daemon；候选 PID/路径本身不授予输入权限。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::cli_image_codex_owned_socket::SocketLease;
use super::cli_image_native_process::{Configuration, Process, foreground_pids, peer_pid};
use super::proto::CliImageCodexBinding;

pub(super) struct Binding {
    tty: File,
    tty_path: PathBuf,
    tty_identity: (u64, u64, u64),
    foreground: i32,
    tui: Process,
    peer: Process,
    codex_home: PathBuf,
    socket: SocketLease,
    tui_image: ImageLease,
    peer_image: ImageLease,
}

struct ImageLease {
    file: File,
    metadata: (u64, u64, u64, i64, i64, i64, i64),
}

impl ImageLease {
    fn validate(&self, process: &Process) -> io::Result<()> {
        let current = fs::symlink_metadata(&process.executable)?;
        if image_metadata(&current) != self.metadata
            || image_metadata(&self.file.metadata()?) != self.metadata
            || (current.dev(), current.ino()) != process.executable_identity
        {
            return Err(invalid());
        }
        Ok(())
    }
}

impl Binding {
    pub(super) fn connect(candidate: &CliImageCodexBinding) -> io::Result<(Self, UnixStream)> {
        let tty_path = PathBuf::from(&candidate.tty_path);
        let metadata = fs::symlink_metadata(&tty_path)?;
        if !tty_path.is_absolute()
            || !metadata.file_type().is_char_device()
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(invalid());
        }
        let tty = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NOCTTY | libc::O_NONBLOCK)
            .open(&tty_path)?;
        let tty_identity = (metadata.dev(), metadata.ino(), metadata.rdev());
        let opened = tty.metadata()?;
        if tty_identity != (opened.dev(), opened.ino(), opened.rdev()) {
            return Err(invalid());
        }
        let codex_home = fs::canonicalize(&candidate.codex_home)?;
        let tui = unique_terminal_client(&codex_home)?;
        if tui.tty != metadata.rdev() || !is_tui(&tui) {
            return Err(invalid());
        }
        let foreground = tui.group;
        let socket_path = codex_home.join("app-server-control/app-server-control.sock");
        let socket = SocketLease::capture(&socket_path)?;
        let stream = UnixStream::connect(&socket_path)?;
        let peer = process(peer_pid(&stream)?)?;
        if peer.uid != tui.uid
            || peer.config_home != codex_home
            || candidate.daemon_pid_candidate != peer.pid as u32
            || !peer.arguments.iter().any(|arg| arg == b"app-server")
        {
            return Err(invalid());
        }
        let tui_image = capture_image(&tui)?;
        let peer_image = capture_image(&peer)?;
        let binding = Self {
            tty,
            tty_path,
            tty_identity,
            foreground,
            tui,
            peer,
            codex_home,
            socket,
            tui_image,
            peer_image,
        };
        binding.validate(&stream)?;
        Ok((binding, stream))
    }

    /// 每次原生请求前重查进程生存期、TTY 前台组、映像与 socket 对端；不向 PID 发信号。
    pub(super) fn validate(&self, stream: &UnixStream) -> io::Result<()> {
        let metadata = fs::symlink_metadata(&self.tty_path)?;
        if (metadata.dev(), metadata.ino(), metadata.rdev()) != self.tty_identity
            || {
                let opened = self.tty.metadata()?;
                (opened.dev(), opened.ino(), opened.rdev()) != self.tty_identity
            }
            || peer_pid(stream)? != self.peer.pid
        {
            return Err(invalid());
        }
        self.socket.validate()?;
        let tui = process(self.tui.pid)?;
        let peer = process(self.peer.pid)?;
        if unique_terminal_client(&self.codex_home)?.identity != self.tui.identity
            || tui.identity != self.tui.identity
            || peer.identity != self.peer.identity
            || tui.executable_identity != self.tui.executable_identity
            || peer.executable_identity != self.peer.executable_identity
            || tui.group != self.foreground
            || tui.foreground_group != self.foreground
            || tui.tty != self.tty_identity.2
            || tui.config_home != self.codex_home
            || peer.config_home != self.codex_home
            || !is_tui(&tui)
        {
            return Err(invalid());
        }
        self.tui_image.validate(&tui)?;
        self.peer_image.validate(&peer)?;
        Ok(())
    }
}

/// 与原生唯一 loaded thread 配对；只找到候选 TTY 上的一个进程仍不足以排除其他 pane。
fn unique_terminal_client(codex_home: &Path) -> io::Result<Process> {
    let mut candidate = None;
    for pid in foreground_pids()? {
        let Ok(process) = process(pid) else {
            continue;
        };
        if process.uid == unsafe { libc::geteuid() }
            && process.tty != 0
            && process.group > 0
            && process.group == process.foreground_group
            && process.config_home == codex_home
            && process
                .executable
                .file_name()
                .is_some_and(|name| name == "codex")
        {
            // 带未受测参数的另一个 Codex 也计入歧义，不能因不识别它而假装独占。
            if candidate.is_some() {
                return Err(invalid());
            }
            candidate = Some(process);
        }
    }
    candidate.ok_or_else(invalid)
}

fn is_tui(process: &Process) -> bool {
    process
        .executable
        .file_name()
        .is_some_and(|name| name == "codex")
        && !process.arguments.iter().skip(1).any(|arg| {
            // 这些入口可能改用另一个 daemon，不能用默认 socket 猜测实际接收端。
            arg.starts_with(b"--remote=")
                || arg.starts_with(b"--config=")
                || arg.starts_with(b"-c")
                || matches!(
                    arg.as_slice(),
                    b"--remote"
                        | b"--no-daemon"
                        | b"--config"
                        | b"--enable"
                        | b"--disable"
                        | b"app-server"
                        | b"exec"
                        | b"e"
                        | b"mcp"
                        | b"mcp-server"
                        | b"login"
                        | b"logout"
                        | b"features"
                        | b"completion"
                        | b"debug"
                        | b"sandbox"
                        | b"cloud"
                        | b"review"
                )
        })
}

fn image_metadata(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

fn capture_image(process: &Process) -> io::Result<ImageLease> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&process.executable)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || (metadata.dev(), metadata.ino()) != process.executable_identity
        || metadata.mode() & 0o022 != 0
        || metadata.len() > 1024 * 1024 * 1024
    {
        return Err(invalid());
    }
    let mut digest = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../script/cli-agent-parity/codex_0156_package_manifest.json"
    ))
    .map_err(|_| invalid())?;
    let package = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("macos", "aarch64") => "macos-arm64",
        _ => return Err(invalid()),
    };
    let record = &manifest["packages"][package]["files"]["bin/codex"];
    let sha256 = format!("{:x}", digest.finalize());
    if manifest["version"].as_str() != Some("0.156.1")
        || record[0].as_u64() != Some(metadata.len())
        || record[1].as_str() != Some(sha256.as_str())
        || image_metadata(&file.metadata()?) != image_metadata(&metadata)
    {
        return Err(invalid());
    }
    Ok(ImageLease {
        file,
        metadata: image_metadata(&metadata),
    })
}

fn process(pid: i32) -> io::Result<Process> {
    super::cli_image_native_process::process(pid, Configuration::Codex)
}

fn invalid() -> io::Error {
    io::Error::other("remote Codex process binding is unavailable")
}
