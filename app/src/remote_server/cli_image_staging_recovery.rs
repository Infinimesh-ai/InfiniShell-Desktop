//! 已发布图片的持久引用；恢复凭据只允许查询/释放，永不恢复输入授权。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const RECORD_LIMIT: u64 = 256 * 1024;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Reference {
    pub(super) version: u32,
    pub(super) host: String,
    pub(super) native_session: String,
    pub(super) transfer_id: Uuid,
    pub(super) key_hash: [u8; 32],
    pub(super) directory: String,
    pub(super) directory_identity: (u64, u64),
    pub(super) filename: String,
    pub(super) file_identity: (u64, u64),
    pub(super) byte_len: u64,
    pub(super) sha256: [u8; 32],
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Release {
    version: u32,
    key_hash: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(in crate::remote_server) struct QueueClaim {
    pub(in crate::remote_server) version: u32,
    pub(in crate::remote_server) host: String,
    pub(in crate::remote_server) native_session: String,
    pub(in crate::remote_server) submission: Uuid,
    pub(in crate::remote_server) key_hash: [u8; 32],
    pub(in crate::remote_server) subject: [u8; 32],
    pub(in crate::remote_server) native_request_sha256: [u8; 32],
    pub(in crate::remote_server) references: Vec<(Uuid, Uuid)>,
    /// 仅原生历史恢复所需路径、偏移和原图摘要；不含 socket 认证材料。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::remote_server) claude_recovery: Option<serde_json::Value>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(in crate::remote_server) struct QueueResult {
    pub(in crate::remote_server) claim: QueueClaim,
    pub(in crate::remote_server) status: String,
    pub(in crate::remote_server) native_queue_id: String,
    pub(in crate::remote_server) native_ack_sha256: Option<[u8; 32]>,
}

pub(super) struct ReferenceStore {
    parent: PathBuf,
    parent_identity: (u64, u64),
    root: PathBuf,
    root_identity: (u64, u64),
    lock: File,
}

struct StoreLock<'a>(&'a File);
impl Drop for StoreLock<'_> {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl ReferenceStore {
    pub(super) fn new(parent: &Path) -> io::Result<Self> {
        let parent_identity = identity(&private_metadata(parent, true)?);
        let root = parent.join("cli-image-references-v1");
        match fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => File::open(parent)?.sync_all()?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let root_identity = identity(&private_metadata(&root, true)?);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join("lock"))?;
        let lock_metadata = private_metadata(&root.join("lock"), false)?;
        if identity(&lock_metadata) != identity(&lock.metadata()?) {
            return Err(invalid());
        }
        Ok(Self {
            parent: parent.to_owned(),
            parent_identity,
            root,
            root_identity,
            lock,
        })
    }

    fn acquire(&self) -> io::Result<StoreLock<'_>> {
        self.lock.lock()?;
        let guard = StoreLock(&self.lock);
        if identity(&private_metadata(&self.parent, true)?) != self.parent_identity
            || identity(&private_metadata(&self.root, true)?) != self.root_identity
            || identity(&private_metadata(&self.root.join("lock"), false)?)
                != identity(&self.lock.metadata()?)
        {
            return Err(invalid());
        }
        Ok(guard)
    }

    pub(super) fn persist(&self, reference: &Reference) -> io::Result<()> {
        let _guard = self.acquire()?;
        self.validate_reference(reference)?;
        if self.release_record(reference.transfer_id)?.is_some() {
            return Err(invalid());
        }
        let path = self.record_path(reference.transfer_id);
        if let Some(previous) = read_record::<Reference>(&path)? {
            return if previous == *reference {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        write_record(&self.root, &path, reference)
    }

    pub(super) fn identifier_available(&self, id: Uuid) -> io::Result<bool> {
        let _guard = self.acquire()?;
        Ok(read_record::<Reference>(&self.record_path(id))?.is_none()
            && self.release_record(id)?.is_none())
    }

    /// 回收仅依据已经落盘的显式释放请求；无记录或未释放的图片不会按年龄删除。
    pub(super) fn retained_bytes(&self, host: &str) -> io::Result<u64> {
        let _guard = self.acquire()?;
        let mut bytes = 0u64;
        for item in fs::read_dir(&self.root)? {
            let path = item?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(id) = name
                .strip_prefix("reference-")
                .and_then(|name| name.strip_suffix(".json"))
                .and_then(|name| Uuid::parse_str(name).ok())
            else {
                continue;
            };
            let reference = read_record::<Reference>(&path)?.ok_or_else(invalid)?;
            self.validate_reference(&reference)?;
            if reference.transfer_id != id || reference.host != host {
                return Err(invalid());
            }
            if let Some(release) = self.release_record(id)? {
                if release.key_hash != reference.key_hash {
                    return Err(invalid());
                }
                self.remove_reference_file(&reference)?;
                continue;
            }
            // 发布未返回时发生崩溃也保留这份引用，等待客户端的持久意图决定。
            bytes = bytes.checked_add(reference.byte_len).ok_or_else(invalid)?;
        }
        Ok(bytes)
    }

    pub(super) fn recover(
        &self,
        host: &str,
        native_session: &str,
        id: Uuid,
        key: Uuid,
    ) -> io::Result<(Option<Reference>, bool)> {
        let _guard = self.acquire()?;
        let key_hash = key_hash(key)?;
        if let Some(release) = self.release_record(id)? {
            if release.key_hash != key_hash {
                return Err(invalid());
            }
            if let Some(reference) = read_record::<Reference>(&self.record_path(id))? {
                self.require_owner(&reference, host, native_session, key_hash)?;
                self.remove_reference_file(&reference)?;
            }
            return Ok((None, true));
        }
        let Some(reference) = read_record::<Reference>(&self.record_path(id))? else {
            return Ok((None, false));
        };
        self.require_owner(&reference, host, native_session, key_hash)?;
        self.open_reference_file(&reference)?;
        Ok((Some(reference), false))
    }

    /// 先持久撤销再删除。即使释放先于发布到达，同一编号也不能复活。
    pub(super) fn release(
        &self,
        host: &str,
        native_session: &str,
        id: Uuid,
        key: Uuid,
    ) -> io::Result<u64> {
        let _guard = self.acquire()?;
        let key_hash = key_hash(key)?;
        let reference = read_record::<Reference>(&self.record_path(id))?;
        if let Some(reference) = &reference {
            self.require_owner(reference, host, native_session, key_hash)?;
        }
        let release = Release {
            version: 1,
            key_hash,
        };
        let path = self.release_path(id);
        match self.release_record(id)? {
            Some(previous) if previous != release => return Err(invalid()),
            Some(_) => {}
            None => write_record(&self.root, &path, &release)?,
        }
        match reference {
            Some(reference) => self.remove_reference_file(&reference),
            None => Ok(0),
        }
    }

    pub(super) fn path(&self, reference: &Reference) -> PathBuf {
        self.parent
            .join(&reference.directory)
            .join(&reference.filename)
    }

    /// 原生写入前落盘未知状态。已经存在的编号只可查询，不能再次领取发送。
    pub(super) fn claim_queue(&self, claim: &QueueClaim) -> io::Result<()> {
        let _guard = self.acquire()?;
        let path = self.root.join(format!("queue-{}.json", claim.submission));
        if claim.version != 1
            || claim.references.is_empty()
            || claim.references.len() > 20
            || claim.submission.is_nil()
            || read_record::<QueueClaim>(&path)?.is_some()
        {
            return Err(invalid());
        }
        for (id, key) in &claim.references {
            let reference =
                read_record::<Reference>(&self.record_path(*id))?.ok_or_else(invalid)?;
            self.require_owner(
                &reference,
                &claim.host,
                &claim.native_session,
                key_hash(*key)?,
            )?;
            if self.release_record(*id)?.is_some() {
                return Err(invalid());
            }
            self.open_reference_file(&reference)?;
        }
        write_record(&self.root, &path, claim)
    }

    pub(super) fn finish_queue(&self, result: &QueueResult) -> io::Result<()> {
        let _guard = self.acquire()?;
        let claim = read_record::<QueueClaim>(
            &self
                .root
                .join(format!("queue-{}.json", result.claim.submission)),
        )?
        .ok_or_else(invalid)?;
        if claim != result.claim
            || !matches!(result.status.as_str(), "confirmed" | "rejected")
            || (result.status == "confirmed" && result.native_queue_id.is_empty())
        {
            return Err(invalid());
        }
        let path = self
            .root
            .join(format!("queue-result-{}.json", claim.submission));
        if let Some(previous) = read_record::<QueueResult>(&path)? {
            return if previous == *result {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        write_record(&self.root, &path, result)
    }

    pub(super) fn queue_status(
        &self,
        host: &str,
        native_session: &str,
        submission: Uuid,
        key: Uuid,
    ) -> io::Result<Option<QueueResult>> {
        let _guard = self.acquire()?;
        let Some(claim) =
            read_record::<QueueClaim>(&self.root.join(format!("queue-{submission}.json")))?
        else {
            return Ok(None);
        };
        if claim.version != 1
            || claim.host != host
            || claim.native_session != native_session
            || claim.submission != submission
            || claim.key_hash != key_hash(key)?
        {
            return Err(invalid());
        }
        match read_record::<QueueResult>(
            &self.root.join(format!("queue-result-{submission}.json")),
        )? {
            Some(result)
                if result.claim == claim
                    && matches!(result.status.as_str(), "confirmed" | "rejected") =>
            {
                Ok(Some(result))
            }
            Some(_) => Err(invalid()),
            None => Ok(Some(QueueResult {
                claim,
                status: "unknown".to_owned(),
                native_queue_id: String::new(),
                native_ack_sha256: None,
            })),
        }
    }

    fn record_path(&self, id: Uuid) -> PathBuf {
        self.root.join(format!("reference-{id}.json"))
    }
    fn release_path(&self, id: Uuid) -> PathBuf {
        self.root.join(format!("released-{id}.json"))
    }
    fn release_record(&self, id: Uuid) -> io::Result<Option<Release>> {
        let release = read_record::<Release>(&self.release_path(id))?;
        if release.as_ref().is_some_and(|release| release.version != 1) {
            return Err(invalid());
        }
        Ok(release)
    }

    fn validate_reference(&self, reference: &Reference) -> io::Result<()> {
        if reference.version != 1
            || reference.transfer_id.is_nil()
            || reference.host.is_empty()
            || reference.native_session.is_empty()
            || reference.byte_len == 0
            || !safe_component(&reference.directory, "cli-image-unpublished-", "")
            || !safe_component(&reference.filename, "attachment-", ".png")
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn require_owner(
        &self,
        reference: &Reference,
        host: &str,
        native_session: &str,
        key: [u8; 32],
    ) -> io::Result<()> {
        self.validate_reference(reference)?;
        if reference.host != host
            || reference.native_session != native_session
            || reference.key_hash != key
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn open_reference_file(&self, reference: &Reference) -> io::Result<File> {
        let directory = self.parent.join(&reference.directory);
        if identity(&private_metadata(&directory, true)?) != reference.directory_identity {
            return Err(invalid());
        }
        let path = self.path(reference);
        if identity(&private_metadata(&path, false)?) != reference.file_identity {
            return Err(invalid());
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        let metadata = file.metadata()?;
        if identity(&metadata) != reference.file_identity || metadata.len() != reference.byte_len {
            return Err(invalid());
        }
        let mut digest = Sha256::new();
        let mut read = 0u64;
        let mut bounded = (&mut file).take(reference.byte_len.saturating_add(1));
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = bounded.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read += count as u64;
            digest.update(&buffer[..count]);
        }
        if read != reference.byte_len || <[u8; 32]>::from(digest.finalize()) != reference.sha256 {
            return Err(invalid());
        }
        Ok(file)
    }

    fn remove_reference_file(&self, reference: &Reference) -> io::Result<u64> {
        let path = self.path(reference);
        match self.open_reference_file(reference) {
            Ok(file) => {
                // 使用已打开句柄再次比对，目录/文件被替换后不能按旧清单删除。
                if identity(&private_metadata(path.parent().ok_or_else(invalid)?, true)?)
                    != reference.directory_identity
                    || identity(&private_metadata(&path, false)?) != identity(&file.metadata()?)
                {
                    return Err(invalid());
                }
                fs::remove_file(&path)?;
                File::open(path.parent().ok_or_else(invalid)?)?.sync_all()?;
                Ok(reference.byte_len)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(error),
        }
    }
}

pub(super) fn key_hash(key: Uuid) -> io::Result<[u8; 32]> {
    if key.is_nil() {
        return Err(invalid());
    }
    Ok(Sha256::digest(key.as_bytes()).into())
}

fn safe_component(value: &str, prefix: &str, suffix: &str) -> bool {
    value.starts_with(prefix)
        && value.ends_with(suffix)
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && !value.contains("..")
}

fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}
fn invalid() -> io::Error {
    io::Error::other("invalid durable image reference")
}
fn private_metadata(path: &Path, directory: bool) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    // 安全性：geteuid 只读取当前进程身份，不接触外部内存。
    let uid = unsafe { libc::geteuid() };
    if metadata.file_type().is_symlink()
        || metadata.uid() != uid
        || metadata.permissions().mode() & 0o077 != 0
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(invalid());
    }
    Ok(metadata)
}

fn read_record<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    let metadata = match private_metadata(path, false) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.len() > RECORD_LIMIT {
        return Err(invalid());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if identity(&metadata) != identity(&file.metadata()?) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(RECORD_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err(invalid());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| invalid())
}

fn write_record<T: Serialize>(directory: &Path, path: &Path, record: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(record).map_err(|_| invalid())?;
    if bytes.len() as u64 > RECORD_LIMIT {
        return Err(invalid());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.disable_cleanup(true);
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?
        .sync_all()?;
    File::open(directory)?.sync_all()
}
