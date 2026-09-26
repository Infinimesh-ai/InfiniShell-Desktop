//! 客户端引用意图保存在当前数据库 scope；重启只恢复记录，绝不重发原生输入。

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::client::RemoteServerClient;
use super::proto::{
    CliImageCodexQueueResult, CliImageCodexQueueStatus, CliImageStagingRecover,
    CliImageStagingRequest, CliImageStagingScope, CliImageStagingSpec,
    cli_image_staging_request::Action, cli_image_staging_response::Result as Response,
};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    version: u32,
    pub(super) host: String,
    pub(super) native_session: String,
    pub(super) submission_id: Uuid,
    pub(super) transfer_id: Uuid,
    // 恢复凭据仅写私有运行状态和 SSH 帧，禁止写日志、遥测或验收收据。
    pub(super) recovery_key: Uuid,
    byte_len: u64,
    sha256: Vec<u8>,
}

impl Intent {
    pub(super) fn new(
        scope: &CliImageStagingScope,
        transfer_id: Uuid,
        spec: &CliImageStagingSpec,
    ) -> io::Result<Self> {
        Ok(Self {
            version: 1,
            host: scope.host_id.clone(),
            native_session: scope.cli_session_id.clone(),
            submission_id: Uuid::parse_str(&scope.submission_id).map_err(|_| invalid())?,
            transfer_id,
            recovery_key: Uuid::new_v4(),
            byte_len: spec.byte_len,
            sha256: spec.sha256.clone(),
        })
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Claim {
    version: u32,
    subject: [u8; 32],
    process_instance: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueueIntent {
    version: u32,
    pub(super) submission: Uuid,
    pub(super) recovery_key: Uuid,
    subject: [u8; 32],
    #[serde(default)]
    claude: bool,
}

pub(super) struct Journal {
    directory: PathBuf,
    lock: File,
}
pub(super) struct PendingLease(File);
impl Drop for PendingLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
struct Guard<'a>(&'a File);
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

type RecoveryWork = (Intent, bool, Option<Arc<PendingLease>>);

impl Journal {
    pub(super) fn open() -> io::Result<Self> {
        #[cfg(not(feature = "local_fs"))]
        return Err(invalid());
        #[cfg(feature = "local_fs")]
        {
            let database = crate::persistence::database_file_path_for_current_scope();
            let parent = database.parent().ok_or_else(invalid)?;
            if !parent.is_absolute() || !fs::symlink_metadata(parent)?.is_dir() {
                return Err(invalid());
            }
            let directory = parent.join("remote-cli-image-references-v1");
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => sync_directory(parent)?,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            private_path(&directory, true)?;
            let mut options = safe_options();
            let lock = options
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(directory.join("lock"))?;
            private_path(&directory.join("lock"), false)?;
            Ok(Self { directory, lock })
        }
    }

    fn acquire(&self) -> io::Result<Guard<'_>> {
        self.lock.lock()?;
        let guard = Guard(&self.lock);
        private_path(&self.directory, true)?;
        private_path(&self.directory.join("lock"), false)?;
        Ok(guard)
    }

    fn path(&self, kind: &str, id: Uuid) -> PathBuf {
        self.directory.join(format!("{kind}-{id}.json"))
    }

    pub(super) fn lease(&self, submission: Uuid) -> io::Result<PendingLease> {
        self.try_lease(submission)?.ok_or_else(invalid)
    }

    fn try_lease(&self, submission: Uuid) -> io::Result<Option<PendingLease>> {
        let path = self.path("lease", submission);
        let file = safe_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        private_path(&path, false)?;
        // 活跃应用持有此锁；另一窗口或恢复进程不能把其尚未领取的上传当成孤儿。
        if file.try_lock().is_err() {
            return Ok(None);
        }
        Ok(Some(PendingLease(file)))
    }

    pub(super) fn persist_intent(&self, intent: &Intent) -> io::Result<()> {
        let _guard = self.acquire()?;
        self.write(&self.path("intent", intent.transfer_id), intent)
    }

    pub(super) fn prepare_queue(
        &self,
        scope: &CliImageStagingScope,
        subject: [u8; 32],
        claude: bool,
    ) -> io::Result<QueueIntent> {
        let submission = Uuid::parse_str(&scope.submission_id).map_err(|_| invalid())?;
        let consumer = if claude { "claude-read" } else { "codex-queue" };
        self.claim(scope, subject, &format!("{consumer}:{submission}"))?;
        let _guard = self.acquire()?;
        let intent = QueueIntent {
            version: 1,
            submission,
            recovery_key: Uuid::new_v4(),
            subject,
            claude,
        };
        self.write(&self.path("queue", submission), &intent)?;
        Ok(intent)
    }

    /// 精确 ACK 和显式未派发结果允许释放；Unknown/Absent 永远不被当成未执行。
    pub(super) fn record_queue_result(
        &self,
        intent: &QueueIntent,
        result: &CliImageCodexQueueResult,
    ) -> io::Result<bool> {
        if result.submission_id != intent.submission.to_string()
            || result.subject_sha256.as_slice() != intent.subject
            || !matches!(result.status.as_str(), "confirmed" | "rejected" | "unknown")
            || (result.status == "confirmed" && result.native_queue_id.is_empty())
        {
            return Err(invalid());
        }
        if result.status == "unknown" {
            return Ok(false);
        }
        let _guard = self.acquire()?;
        self.write(
            &self.path("queue-result", intent.submission),
            &(result.status.clone(), result.native_queue_id.clone()),
        )?;
        self.write(&self.path("release", intent.submission), &true)?;
        Ok(result.status == "confirmed")
    }

    /// 此记录必须先于第一笔原生写入提交；进程在领取后崩溃一律视为未知。
    pub(super) fn claim(
        &self,
        scope: &CliImageStagingScope,
        subject: [u8; 32],
        process_instance: &str,
    ) -> io::Result<()> {
        let _guard = self.acquire()?;
        let submission = Uuid::parse_str(&scope.submission_id).map_err(|_| invalid())?;
        for intent in self.intents()? {
            if intent.host != scope.host_id
                || intent.native_session != scope.cli_session_id
                || intent.submission_id == submission
            {
                continue;
            }
            if self
                .read::<bool>(&self.path("done", intent.transfer_id))?
                .is_some()
            {
                continue;
            }
            if self
                .read::<Claim>(&self.path("claim", intent.submission_id))?
                .is_some_and(|claim| claim.subject == subject)
            {
                return Err(invalid());
            }
        }
        self.write(
            &self.path("claim", submission),
            &Claim {
                version: 1,
                subject,
                process_instance: process_instance.to_owned(),
            },
        )
    }

    /// 已验证原生进程结束后先记录释放意图，即使 SSH 此刻断开也不会丢失回收工作。
    pub(super) fn request_release(
        &self,
        host: &str,
        native_session: &str,
        process_instance: &str,
    ) -> io::Result<()> {
        let _guard = self.acquire()?;
        for intent in self.intents()? {
            if intent.host != host || intent.native_session != native_session {
                continue;
            }
            if self
                .read::<Claim>(&self.path("claim", intent.submission_id))?
                .is_some_and(|claim| {
                    claim.version == 1 && claim.process_instance == process_instance
                })
            {
                self.write(&self.path("release", intent.submission_id), &true)?;
            }
        }
        Ok(())
    }

    /// 每次新 SSH 实例恢复同主机记录；只读恢复不会重建上传/输入租约。
    pub(super) async fn recover(
        &self,
        client: &RemoteServerClient,
        scope: &CliImageStagingScope,
        revision: u64,
    ) -> io::Result<()> {
        self.recover_matching(client, scope, revision, false)
            .await
            .map(|_| ())
    }

    /// 只检查当前 Claude 会话的旧意图；返回是否仍有未确认引用，供有限后续查询使用。
    pub(super) async fn recover_claude(
        &self,
        client: &RemoteServerClient,
        scope: &CliImageStagingScope,
        revision: u64,
    ) -> io::Result<bool> {
        self.recover_matching(client, scope, revision, true).await
    }

    fn recovery_work(
        &self,
        scope: &CliImageStagingScope,
        claude_only: bool,
    ) -> io::Result<Vec<RecoveryWork>> {
        let _guard = self.acquire()?;
        let mut work = Vec::new();
        let mut leases: HashMap<Uuid, Arc<PendingLease>> = HashMap::new();
        for intent in self.intents()? {
            if intent.host != scope.host_id
                || self
                    .read::<bool>(&self.path("done", intent.transfer_id))?
                    .is_some()
            {
                continue;
            }
            if claude_only
                && (intent.native_session != scope.cli_session_id
                    || !self
                        .read::<QueueIntent>(&self.path("queue", intent.submission_id))?
                        .is_some_and(|queue| queue.claude))
            {
                continue;
            }
            let unclaimed = self
                .read::<Claim>(&self.path("claim", intent.submission_id))?
                .is_none();
            let explicit_release =
                self.read::<bool>(&self.path("release", intent.submission_id))? == Some(true);
            let lease = if unclaimed && !explicit_release {
                if let Some(lease) = leases.get(&intent.submission_id) {
                    Some(lease.clone())
                } else {
                    let Some(lease) = self.try_lease(intent.submission_id)? else {
                        continue;
                    };
                    let lease = Arc::new(lease);
                    leases.insert(intent.submission_id, lease.clone());
                    Some(lease)
                }
            } else {
                None
            };
            work.push((intent, unclaimed || explicit_release, lease));
        }
        Ok(work)
    }

    async fn recover_matching(
        &self,
        client: &RemoteServerClient,
        scope: &CliImageStagingScope,
        revision: u64,
        claude_only: bool,
    ) -> io::Result<bool> {
        let work = self.recovery_work(scope, claude_only)?;
        let mut recovered_queues = HashMap::new();
        let mut unconfirmed = false;
        for (intent, mut release, lease) in work {
            let mut request_scope = scope.clone();
            request_scope.cli_session_id = intent.native_session.clone();
            if !release {
                if let Some(recovered) = recovered_queues.get(&intent.submission_id) {
                    release = *recovered;
                } else if let Some(queue) =
                    self.read::<QueueIntent>(&self.path("queue", intent.submission_id))?
                {
                    let status = CliImageCodexQueueStatus {
                        submission_id: intent.submission_id.to_string(),
                        recovery_key: queue.recovery_key.to_string(),
                    };
                    let action = if queue.claude {
                        Action::ClaudeQueueStatus(status)
                    } else {
                        Action::CodexQueueStatus(status)
                    };
                    let result = client
                        .stage_unpublished_cli_image(CliImageStagingRequest {
                            scope: Some(request_scope.clone()),
                            revision,
                            action: Some(action),
                        })
                        .await
                        .map_err(|_| invalid())?;
                    let result = match (queue.claude, result.result) {
                        (false, Some(Response::CodexQueue(result)))
                        | (true, Some(Response::ClaudeQueue(result))) => result,
                        _ => return Err(invalid()),
                    };
                    if result.status != "absent" {
                        self.record_queue_result(&queue, &result)?;
                        release = matches!(result.status.as_str(), "confirmed" | "rejected");
                    }
                    recovered_queues.insert(intent.submission_id, release);
                }
            }
            let response = client
                .stage_unpublished_cli_image(CliImageStagingRequest {
                    scope: Some(request_scope),
                    revision,
                    action: Some(Action::Recover(CliImageStagingRecover {
                        transfer_id: intent.transfer_id.to_string(),
                        recovery_key: intent.recovery_key.to_string(),
                        release,
                    })),
                })
                .await
                .map_err(|_| invalid())?;
            let Some(Response::Recovered(recovered)) = response.result else {
                return Err(invalid());
            };
            if let Some(reference) = recovered.reference {
                if reference.spec
                    != Some(CliImageStagingSpec {
                        byte_len: intent.byte_len,
                        sha256: intent.sha256,
                    })
                {
                    return Err(invalid());
                }
            } else if !recovered.released {
                // 领取后的清单缺失不是消费完成，也不能作为重新发送依据。
                return Err(invalid());
            }
            if recovered.released {
                let _guard = self.acquire()?;
                self.write(&self.path("done", intent.transfer_id), &true)?;
            } else {
                unconfirmed = true;
            }
            drop(lease);
        }
        Ok(unconfirmed)
    }

    fn intents(&self) -> io::Result<Vec<Intent>> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let path = entry?.path();
            let Some(id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("intent-"))
                .and_then(|name| name.strip_suffix(".json"))
                .and_then(|name| Uuid::parse_str(name).ok())
            else {
                continue;
            };
            let intent = self.read::<Intent>(&path)?.ok_or_else(invalid)?;
            if intent.version != 1
                || intent.transfer_id != id
                || intent.recovery_key.is_nil()
                || intent.submission_id.is_nil()
                || intent.byte_len == 0
                || intent.sha256.len() != 32
            {
                return Err(invalid());
            }
            records.push(intent);
        }
        Ok(records)
    }

    fn read<T: serde::de::DeserializeOwned>(&self, path: &Path) -> io::Result<Option<T>> {
        match private_path(path, false) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
        let mut bytes = Vec::new();
        safe_options()
            .read(true)
            .open(path)?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 {
            return Err(invalid());
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| invalid())
    }

    fn write<T: Serialize + serde::de::DeserializeOwned + PartialEq>(
        &self,
        path: &Path,
        value: &T,
    ) -> io::Result<()> {
        if let Some(previous) = self.read::<T>(path)? {
            return if previous == *value {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        let bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
        if bytes.len() > 16 * 1024 {
            return Err(invalid());
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        temporary.disable_cleanup(true);
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(path)
            .map_err(|error| error.error)?
            .sync_all()?;
        sync_directory(&self.directory)
    }
}

fn safe_options() -> OpenOptions {
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

fn private_path(path: &Path, directory: bool) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // 安全性：geteuid 只取得当前进程的有效用户编号。
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
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

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}
fn invalid() -> io::Error {
    io::Error::other("remote image reference journal unavailable")
}

#[cfg(test)]
#[path = "cli_image_submission_journal_tests.rs"]
mod tests;
