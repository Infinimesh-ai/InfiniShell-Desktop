//! Claude 的进程、控制终端、原生注册表与 socket 必须属于同一个仍存活的会话。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::cli_image_claude_queue::ClaudeInboxTarget;
use super::cli_image_native_process::{Configuration, Process, peer_pid, process};
use super::proto::CliImageClaudeBinding;

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Registry {
    messaging_socket_path: PathBuf,
    cwd: PathBuf,
    session_id: Uuid,
    proc_start: String,
    pid_domain: Option<String>,
    kind: String,
}

pub(super) struct Binding {
    target: ClaudeInboxTarget,
    process: Process,
    tty: File,
    tty_path: PathBuf,
    tty_identity: (u64, u64, u64),
    image: File,
    image_identity: (u64, u64, u64, i64, i64, i64, i64),
    socket_identity: (u64, u64),
    registry: File,
    registry_identity: (u64, u64),
    registry_fields: Registry,
}

impl Binding {
    pub(super) fn connect(
        candidate: &CliImageClaudeBinding,
        session: Uuid,
    ) -> io::Result<(Self, UnixStream)> {
        if session.is_nil() || !(2..=i32::MAX as u32).contains(&candidate.process_id_candidate) {
            return Err(invalid());
        }
        let process = process(candidate.process_id_candidate as i32, Configuration::Claude)?;
        let config = fs::canonicalize(&candidate.claude_config_directory)?;
        let tty_path = PathBuf::from(&candidate.tty_path);
        let metadata = fs::symlink_metadata(&tty_path)?;
        if !tty_path.is_absolute()
            || !metadata.file_type().is_char_device()
            || metadata.uid() != unsafe { libc::geteuid() }
            || process.uid != metadata.uid()
            || process.tty != metadata.rdev()
            || process.group <= 0
            || process.foreground_group != process.group
            || process.config_home != config
        {
            return Err(invalid());
        }
        let tty = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NOCTTY | libc::O_NONBLOCK)
            .open(&tty_path)?;
        let tty_identity = (metadata.dev(), metadata.ino(), metadata.rdev());
        let image = fixed_image(&process)?;
        let image_identity = identity(&image.metadata()?);
        let registry_directory = config.join("sessions");
        private_directory(&registry_directory)?;
        let registry_path = registry_directory.join(format!("{}.json", process.pid));
        let mut registry = open_private_file(&registry_path)?;
        let registry_metadata = registry.metadata()?;
        let registry_identity = (registry_metadata.dev(), registry_metadata.ino());
        let mut bytes = Vec::new();
        (&mut registry)
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 {
            return Err(invalid());
        }
        let record: Registry = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if record.session_id != session
            || record.cwd != Path::new(&candidate.working_directory)
            || !record.cwd.is_absolute()
            || record.kind != "interactive"
            || process
                .arguments
                .iter()
                .skip(1)
                .any(|arg| arg == b"-p" || arg == b"--print" || arg.starts_with(b"--print="))
            || registry.metadata()?.ino() != registry_identity.1
        {
            return Err(invalid());
        }
        let socket = socket_metadata(&record.messaging_socket_path)?;
        let socket_identity = (socket.dev(), socket.ino());
        let transcript = PathBuf::from(&candidate.transcript_path);
        // 历史必须位于当前配置的 projects 下，且每一层均无链接；消费者再验证原生会话链。
        let relative = transcript
            .strip_prefix(config.join("projects"))
            .map_err(|_| invalid())?;
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
            || transcript.file_name().and_then(|name| name.to_str())
                != Some(format!("{session}.jsonl").as_str())
        {
            return Err(invalid());
        }
        let mut checked = config.join("projects");
        for component in relative.components() {
            checked.push(component);
            let entry = fs::symlink_metadata(&checked)?;
            if entry.file_type().is_symlink()
                || entry.uid() != process.uid
                || entry.mode() & 0o022 != 0
            {
                return Err(invalid());
            }
        }
        let target = ClaudeInboxTarget {
            session_id: session,
            cwd: record.cwd.clone(),
            process_id: process.pid as u32,
            socket_path: record.messaging_socket_path.clone(),
            registry_directory,
            transcript_path: transcript,
        };
        let stream = UnixStream::connect(&target.socket_path)?;
        let binding = Self {
            target,
            process,
            tty,
            tty_path,
            tty_identity,
            image,
            image_identity,
            socket_identity,
            registry,
            registry_identity,
            registry_fields: record,
        };
        binding.validate(&stream)?;
        Ok((binding, stream))
    }

    pub(super) fn target(&self) -> ClaudeInboxTarget {
        self.target.clone()
    }

    pub(super) fn validate(&self, stream: &UnixStream) -> io::Result<()> {
        if peer_pid(stream)? != self.process.pid {
            return Err(invalid());
        }
        let now = process(self.process.pid, Configuration::Claude)?;
        let tty = fs::symlink_metadata(&self.tty_path)?;
        let opened = self.tty.metadata()?;
        let socket = socket_metadata(&self.target.socket_path)?;
        let registry_path = self
            .target
            .registry_directory
            .join(format!("{}.json", now.pid));
        let metadata = self.registry.metadata()?;
        let mut current_registry = open_private_file(&registry_path)?;
        let reopened = current_registry.metadata()?;
        let mut bytes = Vec::new();
        (&mut current_registry)
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 {
            return Err(invalid());
        }
        let fields: Registry = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if now.identity != self.process.identity
            || now.uid != self.process.uid
            || now.config_home != self.process.config_home
            || now.executable_identity != self.process.executable_identity
            || now.tty != self.tty_identity.2
            || now.group != self.process.group
            || now.foreground_group != now.group
            || (tty.dev(), tty.ino(), tty.rdev()) != self.tty_identity
            || (opened.dev(), opened.ino(), opened.rdev()) != self.tty_identity
            || (socket.dev(), socket.ino()) != self.socket_identity
            || identity(&self.image.metadata()?) != self.image_identity
            || identity(&fs::symlink_metadata(&now.executable)?) != self.image_identity
            || (metadata.dev(), metadata.ino()) != self.registry_identity
            || (reopened.dev(), reopened.ino()) != self.registry_identity
            || fields != self.registry_fields
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn fixed_image(process: &Process) -> io::Result<File> {
    let mut image = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&process.executable)?;
    let metadata = image.metadata()?;
    if !metadata.is_file()
        || metadata.mode() & 0o022 != 0
        || (metadata.dev(), metadata.ino()) != process.executable_identity
    {
        return Err(invalid());
    }
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../specs/cli-agent-parity/fixtures/claude-2.1.280-release-manifest.json"
    ))
    .map_err(|_| invalid())?;
    let key = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("linux", "x86_64") => "linux-x64",
        _ => return Err(invalid()),
    };
    let record = &manifest["platforms"][key];
    if manifest["version"] != "2.1.280" || record["size"].as_u64() != Some(metadata.len()) {
        return Err(invalid());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = image.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if record["checksum"].as_str() != Some(format!("{:x}", digest.finalize()).as_str())
        || identity(&image.metadata()?) != identity(&metadata)
    {
        return Err(invalid());
    }
    Ok(image)
}

fn identity(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
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

fn private_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(invalid());
    }
    Ok(())
}

fn open_private_file(path: &Path) -> io::Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
        || identity(&metadata) != identity(&file.metadata()?)
    {
        return Err(invalid());
    }
    Ok(file)
}

fn socket_metadata(path: &Path) -> io::Result<fs::Metadata> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(invalid());
    }
    private_directory(path.parent().ok_or_else(invalid)?)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(invalid());
    }
    Ok(metadata)
}

fn invalid() -> io::Error {
    io::Error::other("remote Claude process binding is unavailable")
}
