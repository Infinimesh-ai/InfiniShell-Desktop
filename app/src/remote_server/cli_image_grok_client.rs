//! 本机持久意图与远端 Grok 请求；恢复只能查旧票据与回执，不能重发启动或输入。

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use warp_core::SessionId;
use warpui::EntityId;

use crate::terminal::cli_agent_sessions::GrokPermissionObservation;

use super::cli_image_grok_protocol::{
    Action, Input, MAX_BODY_BYTES, Reply, Request, Scope, Ticket, encode,
};
use super::client::RemoteServerClient;
use super::proto::CliImageStagingScope;

struct Active {
    client: Arc<RemoteServerClient>,
    scope: CliImageStagingScope,
    revision: u64,
    ticket: Ticket,
    observation: GrokPermissionObservation,
}
static ACTIVE: LazyLock<Mutex<HashMap<EntityId, Active>>> = LazyLock::new(Mutex::default);

pub(crate) fn register(
    view: EntityId,
    client: Arc<RemoteServerClient>,
    scope: CliImageStagingScope,
    revision: u64,
    ticket: Ticket,
    observation: GrokPermissionObservation,
) {
    revoke(view);
    ACTIVE.lock().expect("远端 Grok 输入锁").insert(
        view,
        Active {
            client,
            scope,
            revision,
            ticket,
            observation,
        },
    );
}

pub(crate) fn revoke(view: EntityId) {
    if let Some(active) = ACTIVE.lock().expect("远端 Grok 输入锁").remove(&view) {
        if let Ok(generation) = Uuid::parse_str(&active.scope.input_generation) {
            if let Ok(body) = request_body(
                &active.scope,
                active.revision,
                Action::Revoke {
                    ticket: active.ticket,
                    generation,
                },
            ) {
                active.client.revoke_cli_grok_owned(active.scope, body);
            }
        }
    }
}

pub(crate) fn revoke_unless(view: EntityId, observed: Option<&GrokPermissionObservation>) {
    let stale = ACTIVE
        .lock()
        .expect("远端 Grok 输入锁")
        .get(&view)
        .is_some_and(|active| Some(&active.observation) != observed);
    if stale {
        revoke(view);
    }
}

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
        let directory = parent.join("remote-grok-owned-v1");
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

    pub(crate) fn claim_input(&self, launch: &Launch, input: &Input) -> io::Result<String> {
        let _guard = self.acquire()?;
        let subject = subject(input)?;
        // 内容摘要不含 message_id；改变 UUID 不能重新发送结果未知的同一草稿。
        let prefix = format!("{}-", launch.ticket.id);
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix)
                && name.ends_with("-unknown.json")
                && !name.ends_with("-launch-unknown.json")
            {
                let (id, previous): (Uuid, String) = self.read(&entry.path())?;
                if previous == subject
                    && !self.directory.join(format!("{id}-finished.json")).exists()
                    && !self.input_not_dispatched(launch, id, &previous)?
                {
                    return Err(invalid());
                }
            }
        }
        let path = self.directory.join(format!(
            "{}-{}-unknown.json",
            launch.ticket.id, input.message_id
        ));
        self.write(&path, &(input.message_id, &subject))?;
        Ok(subject)
    }

    /// 仅用于尚未调用 Submit 的本地取消；已开始 RPC 的未知结果不能使用此终态。
    pub(crate) fn record_not_dispatched(
        &self,
        launch: &Launch,
        message_id: Uuid,
        subject: &str,
    ) -> io::Result<()> {
        let _guard = self.acquire()?;
        let claim = self
            .directory
            .join(format!("{}-{message_id}-unknown.json", launch.ticket.id));
        let (expected, hash): (Uuid, String) = self.read(&claim)?;
        if expected != message_id
            || hash != subject
            || self
                .directory
                .join(format!("{message_id}-finished.json"))
                .exists()
        {
            return Err(invalid());
        }
        if !self.input_not_dispatched(launch, message_id, subject)? {
            // 保留原领取字节，另写不可变终态；写入失败仍保持 Unknown。
            self.write(
                &self.directory.join(format!(
                    "{}-{message_id}-not-dispatched.json",
                    launch.ticket.id
                )),
                &(&launch.ticket, message_id, subject),
            )?;
        }
        Ok(())
    }

    fn input_not_dispatched(
        &self,
        launch: &Launch,
        message_id: Uuid,
        subject: &str,
    ) -> io::Result<bool> {
        let path = self.directory.join(format!(
            "{}-{message_id}-not-dispatched.json",
            launch.ticket.id
        ));
        match self.read::<(Ticket, Uuid, String)>(&path) {
            Ok((ticket, expected, hash))
                if ticket == launch.ticket && expected == message_id && hash == subject =>
            {
                Ok(true)
            }
            Ok(_) => Err(invalid()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn record_reply(&self, launch: &Launch, reply: &Reply) -> io::Result<()> {
        let _guard = self.acquire()?;
        let Reply::Input {
            ticket,
            message_id,
            subject_sha256,
            state,
            native_prompt_id,
            native_ack_sha256,
        } = reply
        else {
            return Ok(());
        };
        if ticket != &launch.ticket {
            return Err(invalid());
        }
        if !matches!(state.as_str(), "finished" | "cancelled") {
            return Ok(());
        }
        if native_prompt_id.is_none_or(|id| id.is_nil())
            || native_ack_sha256.as_ref().is_none_or(|value| {
                value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(invalid());
        }
        let claim_path = self
            .directory
            .join(format!("{}-{message_id}-unknown.json", ticket.id));
        let (expected, hash): (Uuid, String) = self.read(&claim_path)?;
        if expected != *message_id || hash != *subject_sha256 {
            return Err(invalid());
        }
        let finished = self.directory.join(format!("{message_id}-finished.json"));
        if !finished.exists() {
            self.write(&finished, reply)?;
        }
        // 已确认仍保留摘要记录；明确再次发送同样内容由新的编辑/发送操作另行处理。
        Ok(())
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

    fn unknown_inputs(&self, launch: &Launch) -> io::Result<Vec<Uuid>> {
        let prefix = format!("{}-", launch.ticket.id);
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.directory)?.take(4097) {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(&prefix)
                || !name.ends_with("-unknown.json")
                || name.ends_with("-launch-unknown.json")
            {
                continue;
            }
            let (id, subject): (Uuid, String) = self.read(&entry.path())?;
            if !self.directory.join(format!("{id}-finished.json")).exists()
                && !self.input_not_dispatched(launch, id, &subject)?
            {
                ids.push(id);
            }
        }
        Ok(ids)
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
        .cli_grok_owned_request(scope, body)
        .await
        .map_err(|_| invalid())?;
    if response.len() > MAX_BODY_BYTES {
        return Err(invalid());
    }
    serde_json::from_slice(&response).map_err(io::Error::other)
}

pub(crate) async fn recover_replies(
    client: &Arc<RemoteServerClient>,
    launch: &Launch,
) -> io::Result<()> {
    let journal = Journal::open()?;
    for message_id in journal.unknown_inputs(launch)? {
        let (scope, revision) = scope(client, launch, Uuid::new_v4())?;
        let reply = request(
            client,
            scope,
            revision,
            Action::InputStatus {
                ticket: launch.ticket.clone(),
                message_id,
            },
        )
        .await?;
        journal.record_reply(launch, &reply)?;
    }
    Ok(())
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

pub(crate) fn subject(input: &Input) -> io::Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(encode(&(&input.text, &input.images)).map_err(|_| invalid())?)
    ))
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
    io::Error::other("远端 Grok 意图或连接不可用")
}

#[cfg(test)]
#[path = "cli_image_grok_client_tests.rs"]
mod tests;
