//! 本机 Codex 私有启动意图；重连仅查询同一票据，不重新启动。

use super::cli_image_codex_owned_protocol::{
    Action, MAX_BODY_BYTES, Reply, Request, Scope, Ticket, encode,
};
use super::client::RemoteServerClient;
use super::proto::CliImageStagingScope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;
use warp_core::SessionId;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Launch {
    pub host: String,
    pub terminal_session: u64,
    pub block_id: String,
    pub cwd: String,
    pub ticket: Ticket,
}

pub(crate) struct Journal {
    directory: PathBuf,
    lock: File,
}

struct Guard<'a>(&'a File);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl Journal {
    pub(crate) fn open() -> io::Result<Self> {
        let database = crate::persistence::database_file_path_for_current_scope();
        let parent = database.parent().ok_or_else(invalid)?;
        let directory = parent.join("remote-codex-owned-v1");
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        check_private(&directory, true)?;
        let lock_path = directory.join("journal.lock");
        let lock = options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        check_private(&lock_path, false)?;
        Ok(Self { directory, lock })
    }

    fn acquire(&self) -> io::Result<Guard<'_>> {
        self.lock.lock()?;
        Ok(Guard(&self.lock))
    }

    /// 首次意图先落盘；已有票据无论远端状态如何都不会分配新 UUID。
    pub(crate) fn reserve(
        &self,
        host: String,
        session: SessionId,
        block_id: String,
        cwd: String,
    ) -> io::Result<(Launch, bool)> {
        let _guard = self.acquire()?;
        let key = format!(
            "{:x}",
            Sha256::digest(encode(&(&host, session.as_u64(), &block_id)).map_err(|_| invalid())?)
        );
        let path = self.directory.join(format!("{key}-launch.json"));
        if path.exists() {
            let previous: Launch = self.read(&path)?;
            if previous.host != host
                || previous.terminal_session != session.as_u64()
                || previous.block_id != block_id
                || previous.cwd != cwd
            {
                return Err(invalid());
            }
            return Ok((previous, false));
        }
        let launch = Launch {
            host,
            terminal_session: session.as_u64(),
            block_id,
            cwd,
            ticket: Ticket {
                id: Uuid::new_v4(),
                key: Uuid::new_v4(),
            },
        };
        self.write(&path, &launch)?;
        Ok((launch, true))
    }

    pub(crate) fn claim_launch(&self, launch: &Launch) -> io::Result<()> {
        let _guard = self.acquire()?;
        self.write(
            &self
                .directory
                .join(format!("{}-launch-unknown.json", launch.ticket.id)),
            launch,
        )
    }

    pub(crate) fn known_launches(
        &self,
        host: &str,
        session: SessionId,
        cwd: &str,
    ) -> io::Result<Vec<Launch>> {
        let mut found = Vec::new();
        for entry in fs::read_dir(&self.directory)?.take(4097) {
            let entry = entry?;
            if !entry
                .file_name()
                .to_string_lossy()
                .ends_with("-launch.json")
            {
                continue;
            }
            let launch: Launch = self.read(&entry.path())?;
            if launch.host == host
                && launch.terminal_session == session.as_u64()
                && launch.cwd == cwd
            {
                found.push(launch);
                if found.len() > 128 {
                    return Err(invalid());
                }
            }
        }
        Ok(found)
    }

    fn read<T: serde::de::DeserializeOwned>(&self, path: &Path) -> io::Result<T> {
        check_private(path, false)?;
        let file = options().read(true).open(path)?;
        let mut bytes = Vec::new();
        file.take(MAX_BODY_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BODY_BYTES {
            return Err(invalid());
        }
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }
    fn write<T: Serialize>(&self, path: &Path, value: &T) -> io::Result<()> {
        check_private(&self.directory, true)?;
        let bytes = encode(value).map_err(|_| invalid())?;
        let mut file = options().write(true).create_new(true).open(path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

pub(crate) fn scope(
    client: &RemoteServerClient,
    launch: &Launch,
    generation: Uuid,
) -> io::Result<(CliImageStagingScope, u64)> {
    let (scope, revision) = client
        .allocate_cli_image_scope(
            SessionId::from(launch.terminal_session),
            &launch.ticket.id.to_string(),
            generation,
            Uuid::new_v4(),
        )
        .map_err(|_| invalid())?;
    if scope.host_id != launch.host {
        return Err(invalid());
    }
    Ok((scope, revision))
}

pub(crate) async fn request(
    client: &Arc<RemoteServerClient>,
    scope: CliImageStagingScope,
    revision: u64,
    action: Action,
) -> io::Result<Reply> {
    let body = request_body(&scope, revision, action)?;
    let response = client
        .cli_codex_owned_request(scope, body)
        .await
        .map_err(|_| invalid())?;
    if response.len() > MAX_BODY_BYTES {
        return Err(invalid());
    }
    serde_json::from_slice(&response).map_err(io::Error::other)
}

pub(crate) fn request_body(
    scope: &CliImageStagingScope,
    revision: u64,
    action: Action,
) -> io::Result<Vec<u8>> {
    encode(&Request {
        version: 1,
        revision,
        scope: Scope {
            host: scope.host_id.clone(),
            terminal_session: scope.terminal_session_id,
            terminal_epoch: Uuid::parse_str(&scope.terminal_epoch).map_err(|_| invalid())?,
            generation: Uuid::parse_str(&scope.input_generation).map_err(|_| invalid())?,
        },
        action,
    })
    .map_err(|_| invalid())
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    options
}

fn check_private(path: &Path, directory: bool) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(invalid());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid());
        }
    }
    Ok(())
}
fn invalid() -> io::Error {
    io::Error::other("远端 Codex 意图或连接不可用")
}
