//! 远端 CLI 图片字节暂存；原生投递与文件引用发布由上层显式控制。
//!
//! 仅服务端已连接会话可登记作用域。ConnectionId 必须来自 RPC 连接上下文，
//! 不能采用客户端正文里的值。校验收据不包含路径，不能充当图片已投递证明。
//! 已发布附件跨断连保留；只有明确的原生消费/退出合同才能调用显式释放。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::{NamedTempFile, TempDir};
use uuid::Uuid;
use warp_core::{HostId, SessionId};

#[path = "cli_image_staging_recovery.rs"]
mod recovery;
pub(super) use recovery::{QueueClaim, QueueResult};

const MAX_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const MAX_SCOPE_TRANSFERS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RemoteImageScope {
    pub host_id: HostId,
    pub connection_id: Uuid,
    pub terminal_session_id: SessionId,
    pub terminal_epoch: Uuid,
    pub cli_session_id: String,
    pub input_generation: Uuid,
    // 一次用户发送尝试的稳定编号；重试复用，新草稿发送必须换号。
    pub submission_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteImageSpec {
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedStagedImage {
    pub scope: RemoteImageScope,
    pub transfer_id: Uuid,
    pub spec: RemoteImageSpec,
}

struct StagedImage {
    scope: RemoteImageScope,
    spec: RemoteImageSpec,
    file: NamedTempFile,
    written: u64,
    published: bool,
}

pub(crate) struct RemoteImageStaging {
    host_id: HostId,
    root: TempDir,
    root_identity: (u64, u64),
    max_reserved_bytes: u64,
    reserved_bytes: u64,
    scopes: HashMap<(Uuid, SessionId), RemoteImageScope>,
    entries: HashMap<Uuid, StagedImage>,
    // 已取消的编号在同一作用域内不能重新创建，防止迟到的 begin 恢复旧上传。
    used_ids: HashMap<(Uuid, SessionId), HashSet<Uuid>>,
    references: recovery::ReferenceStore,
}

impl RemoteImageStaging {
    pub(super) fn queue_status(&self, scope: &RemoteImageScope, submission: Uuid, key: Uuid) -> io::Result<Option<QueueResult>> {
        self.references.queue_status(scope.host_id.as_str(), &scope.cli_session_id, submission, key)
    }

    pub(super) fn claim_queue(&self, claim: &QueueClaim) -> io::Result<()> {
        self.references.claim_queue(claim)
    }

    pub(super) fn finish_queue(&mut self, scope: &RemoteImageScope, result: &QueueResult) -> io::Result<()> {
        self.references.finish_queue(result)?;
        // 先保存原生快照确认，再逐个释放。清理中断可从同一持久结果恢复。
        for (id, key) in &result.claim.references {
            self.release_recovered(scope, *id, *key)?;
        }
        Ok(())
    }
    /// parent 必须是服务端已有的当前用户私有目录；不创建/放宽上层权限。
    /// max_reserved_bytes 由未来产品层从附件总预算传入，不在这里放宽原生大小限制。
    pub(crate) fn new(host_id: HostId, parent: &Path, max_reserved_bytes: u64) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(parent)?;
        // 安全性：geteuid 没有参数或内存访问，仅取得当前进程的有效用户编号。
        let uid = unsafe { libc::geteuid() };
        if host_id.as_str().is_empty()
            || max_reserved_bytes == 0
            || !parent.is_absolute()
            || !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != uid
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(rejected("invalid private staging parent"));
        }
        let mut root = tempfile::Builder::new()
            .prefix("cli-image-unpublished-")
            .tempdir_in(parent)?;
        // 禁止 TempDir 析构递归删除；身份不明的磁盘项必须留给显式恢复。
        root.disable_cleanup(true);
        let root_metadata = fs::symlink_metadata(root.path())?;
        if root_metadata.permissions().mode() & 0o077 != 0 {
            return Err(rejected("staging directory is not private"));
        }
        let references = recovery::ReferenceStore::new(parent)?;
        let reserved_bytes = references.retained_bytes(host_id.as_str())?;
        Ok(Self {
            host_id,
            root,
            root_identity: (root_metadata.dev(), root_metadata.ino()),
            max_reserved_bytes,
            reserved_bytes,
            scopes: HashMap::new(),
            entries: HashMap::new(),
            used_ids: HashMap::new(),
            references,
        })
    }

    /// 由可信终端/连接生命周期调用；不能把每个上传请求当作重新登记机会。
    pub(crate) fn activate_scope(&mut self, scope: RemoteImageScope) -> io::Result<()> {
        self.require_root_identity()?;
        if scope.host_id != self.host_id
            || scope.connection_id.is_nil()
            || scope.terminal_epoch.is_nil()
            || scope.input_generation.is_nil()
            || scope.submission_id.is_nil()
            || scope.cli_session_id.is_empty()
            || scope.cli_session_id.len() > 512
            || scope.cli_session_id.chars().any(char::is_control)
        {
            return Err(rejected("invalid image scope"));
        }
        if self
            .scopes
            .get(&(scope.connection_id, scope.terminal_session_id))
            == Some(&scope)
        {
            return Ok(());
        }
        let key = (scope.connection_id, scope.terminal_session_id);
        self.remove_terminal(key)?;
        self.scopes.insert(key, scope);
        Ok(())
    }

    pub(crate) fn begin(
        &mut self,
        scope: &RemoteImageScope,
        transfer_id: Uuid,
        spec: RemoteImageSpec,
    ) -> io::Result<u64> {
        self.require_current(scope)?;
        if transfer_id.is_nil() || spec.byte_len == 0 {
            return Err(rejected("invalid image transfer"));
        }
        if let Some(entry) = self.entries.get(&transfer_id) {
            return if entry.scope == *scope && entry.spec == spec {
                Ok(entry.written)
            } else {
                Err(rejected("image transfer identity changed"))
            };
        }
        if !self.references.identifier_available(transfer_id)? {
            return Err(rejected("image reference identifier is retired"));
        }
        let used = self
            .used_ids
            .entry((scope.connection_id, scope.terminal_session_id))
            .or_default();
        if used.contains(&transfer_id) || used.len() >= MAX_SCOPE_TRANSFERS {
            return Err(rejected("image transfer identifier unavailable"));
        }
        let reserved = self
            .reserved_bytes
            .checked_add(spec.byte_len)
            .filter(|reserved| *reserved <= self.max_reserved_bytes)
            .ok_or_else(|| rejected("image staging budget exceeded"))?;
        // NamedTempFile 使用独占创建与 0600；文件名和路径不采用用户输入。
        let mut file = tempfile::Builder::new()
            .prefix("attachment-")
            .suffix(".png")
            .tempfile_in(self.root.path())?;
        // 路径被替换后，NamedTempFile 的无条件析构删除会绕过身份核验。
        file.disable_cleanup(true);
        if file.as_file().metadata()?.permissions().mode() & 0o077 != 0 {
            return Err(rejected("image staging file is not private"));
        }
        used.insert(transfer_id);
        self.reserved_bytes = reserved;
        self.entries.insert(
            transfer_id,
            StagedImage {
                scope: scope.clone(),
                spec,
                file,
                written: 0,
                published: false,
            },
        );
        Ok(0)
    }

    /// 精确重投返回当前水位；空块、空洞、部分重叠和不同字节均拒绝。
    pub(crate) fn write_chunk(
        &mut self,
        scope: &RemoteImageScope,
        transfer_id: Uuid,
        offset: u64,
        bytes: &[u8],
    ) -> io::Result<u64> {
        self.require_current(scope)?;
        let entry = self.entries.get_mut(&transfer_id).ok_or_else(missing)?;
        require_entry_scope(entry, scope)?;
        if entry.published {
            return Err(rejected("published image is immutable"));
        }
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| rejected("image offset overflow"))?;
        if bytes.is_empty() || bytes.len() > MAX_CHUNK_BYTES || end > entry.spec.byte_len {
            return Err(rejected("invalid image chunk size"));
        }
        let file = entry.file.as_file_mut();
        if offset < entry.written && end <= entry.written {
            file.seek(SeekFrom::Start(offset))?;
            let mut previous = vec![0; bytes.len()];
            file.read_exact(&mut previous)?;
            return if previous == bytes {
                Ok(entry.written)
            } else {
                Err(rejected("duplicate image chunk changed"))
            };
        }
        if offset != entry.written {
            return Err(rejected("image chunk is not contiguous"));
        }
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(bytes)?;
        entry.written = end;
        Ok(end)
    }

    /// 只证明当前暂存文件与预期原始字节摘要相同；不证明格式或 CLI 消费。
    pub(crate) fn verify(
        &mut self,
        scope: &RemoteImageScope,
        transfer_id: Uuid,
    ) -> io::Result<VerifiedStagedImage> {
        self.require_current(scope)?;
        let entry = self.entries.get_mut(&transfer_id).ok_or_else(missing)?;
        require_entry_scope(entry, scope)?;
        let file = entry.file.as_file_mut();
        if entry.written != entry.spec.byte_len || file.metadata()?.len() != entry.spec.byte_len {
            return Err(rejected("image transfer is incomplete"));
        }
        file.flush()?;
        file.seek(SeekFrom::Start(0))?;
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let mut read_bytes = 0u64;
        let mut bounded = file.take(entry.spec.byte_len.saturating_add(1));
        loop {
            let count = bounded.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read_bytes += count as u64;
            digest.update(&buffer[..count]);
        }
        if read_bytes != entry.spec.byte_len {
            return Err(rejected("image staging file length changed"));
        }
        let actual: [u8; 32] = digest.finalize().into();
        if actual != entry.spec.sha256 {
            return Err(rejected("image transfer digest mismatch"));
        }
        Ok(VerifiedStagedImage {
            scope: entry.scope.clone(),
            transfer_id,
            spec: entry.spec.clone(),
        })
    }

    /// 仅撤销未投递上传。重复撤销成功，但其他作用域不能删除这份文件。
    pub(crate) fn cancel(&mut self, scope: &RemoteImageScope, transfer_id: Uuid) -> io::Result<()> {
        self.require_current(scope)?;
        if transfer_id.is_nil() {
            return Err(rejected("invalid image transfer"));
        }
        if let Some(entry) = self.entries.get(&transfer_id) {
            require_entry_scope(entry, scope)?;
            if entry.published {
                return Err(rejected("published image requires explicit release"));
            }
            remove_owned_file(entry)?;
        }
        // cancel 可能先于 begin 到达。返回成功前也要为未知编号留下墓碑，
        // 并沿用作用域上限，避免大量取消请求造成无界内存增长。
        let used = self
            .used_ids
            .entry((scope.connection_id, scope.terminal_session_id))
            .or_default();
        if !used.contains(&transfer_id) && used.len() >= MAX_SCOPE_TRANSFERS {
            return Err(rejected("image transfer identifier unavailable"));
        }
        used.insert(transfer_id);
        if let Some(entry) = self.entries.remove(&transfer_id) {
            self.reserved_bytes -= entry.spec.byte_len;
        }
        Ok(())
    }

    /// 发布返回核验后的原始文件引用；不能据此宣称原生 CLI 已消费图片。
    pub(crate) fn publish(
        &mut self,
        scope: &RemoteImageScope,
        transfer_id: Uuid,
        recovery_key: Uuid,
    ) -> io::Result<(PathBuf, RemoteImageSpec)> {
        let verified = self.verify(scope, transfer_id)?;
        let entry = self.entries.get_mut(&transfer_id).ok_or_else(missing)?;
        require_owned_file(entry)?;
        // 当前消费者统一转成 PNG；后缀必须与原生按路径识别的格式一致。
        let mut signature = [0; 8];
        entry.file.as_file_mut().seek(SeekFrom::Start(0))?;
        entry.file.as_file_mut().read_exact(&mut signature)?;
        if signature != *b"\x89PNG\r\n\x1a\n" {
            return Err(rejected("published image is not PNG"));
        }
        entry.file.as_file().sync_all()?;
        let metadata = entry.file.as_file().metadata()?;
        let reference = recovery::Reference {
            version: 1,
            host: scope.host_id.as_str().to_owned(),
            native_session: scope.cli_session_id.clone(),
            transfer_id,
            key_hash: recovery::key_hash(recovery_key)?,
            directory: self
                .root
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| rejected("invalid image directory"))?
                .to_owned(),
            directory_identity: self.root_identity,
            filename: entry
                .file
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| rejected("invalid image filename"))?
                .to_owned(),
            file_identity: (metadata.dev(), metadata.ino()),
            byte_len: verified.spec.byte_len,
            sha256: verified.spec.sha256,
        };
        // 响应和原生写入之前必须提交恢复清单；进程崩溃不能丢失已发布归属。
        self.references.persist(&reference)?;
        entry.published = true;
        Ok((entry.file.path().to_owned(), verified.spec))
    }

    /// 原生消费或会话结束已得到上层明确确认后，才允许释放已发布引用。
    pub(crate) fn release_published(
        &mut self,
        scope: &RemoteImageScope,
        transfer_id: Uuid,
        recovery_key: Uuid,
    ) -> io::Result<()> {
        self.require_root_identity()?;
        if let Some(entry) = self.entries.get(&transfer_id) {
            require_entry_scope(entry, scope)?;
            if !entry.published {
                return Err(rejected("image reference was not published"));
            }
        }
        self.release_recovered(scope, transfer_id, recovery_key)?;
        Ok(())
    }

    pub(crate) fn recover_reference(
        &mut self,
        scope: &RemoteImageScope,
        transfer_id: Uuid,
        key: Uuid,
        release: bool,
    ) -> io::Result<(Option<(PathBuf, RemoteImageSpec)>, bool)> {
        if release {
            self.release_recovered(scope, transfer_id, key)?;
        }
        let (reference, released) = self.references.recover(
            scope.host_id.as_str(),
            &scope.cli_session_id,
            transfer_id,
            key,
        )?;
        Ok((
            reference.map(|reference| {
                (
                    self.references.path(&reference),
                    RemoteImageSpec {
                        byte_len: reference.byte_len,
                        sha256: reference.sha256,
                    },
                )
            }),
            released,
        ))
    }

    fn release_recovered(
        &mut self,
        scope: &RemoteImageScope,
        transfer_id: Uuid,
        key: Uuid,
    ) -> io::Result<()> {
        let released = self.references.release(
            scope.host_id.as_str(),
            &scope.cli_session_id,
            transfer_id,
            key,
        )?;
        // 同一内核的句柄只解除自有引用；磁盘删除已经经过持久清单身份核验。
        if self
            .entries
            .get(&transfer_id)
            .is_some_and(|entry| entry.published)
        {
            self.entries.remove(&transfer_id);
        }
        self.reserved_bytes = self.reserved_bytes.saturating_sub(released);
        Ok(())
    }

    /// 精确撤销旧批次，不触及同终端较新的作用域或已发布引用。
    pub(crate) fn revoke_scope(&mut self, scope: &RemoteImageScope) -> io::Result<()> {
        let key = (scope.connection_id, scope.terminal_session_id);
        if self.scopes.get(&key) == Some(scope) {
            self.scopes.remove(&key);
        }
        self.require_root_identity()?;
        let transfers: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.scope == *scope && !entry.published)
            .map(|(id, _)| *id)
            .collect();
        let mut failure = None;
        for id in transfers {
            let entry = self.entries.get(&id).ok_or_else(missing)?;
            if let Err(error) = remove_owned_file(entry) {
                if failure.is_none() {
                    failure = Some(error);
                }
                continue;
            }
            if let Some(entry) = self.entries.remove(&id) {
                self.reserved_bytes -= entry.spec.byte_len;
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// 连接断开仅回收未发布字节；已发布引用仍等待显式释放。

    pub(crate) fn disconnect(&mut self, connection_id: Uuid) -> io::Result<()> {
        let sessions: HashSet<_> = self
            .scopes
            .keys()
            .copied()
            .chain(
                self.entries
                    .values()
                    .map(|entry| (entry.scope.connection_id, entry.scope.terminal_session_id)),
            )
            .filter(|(connection, _)| *connection == connection_id)
            .collect();
        let mut failure = None;
        for key in sessions {
            if let Err(error) = self.remove_terminal(key) {
                // 单个文件清理失败不能阻止同连接其余作用域撤销。
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn require_root_identity(&self) -> io::Result<()> {
        let named = fs::symlink_metadata(self.root.path())?;
        if !named.is_dir()
            || named.file_type().is_symlink()
            || (named.dev(), named.ino()) != self.root_identity
            || named.permissions().mode() & 0o077 != 0
        {
            return Err(rejected("image staging directory identity changed"));
        }
        Ok(())
    }

    fn require_current(&self, scope: &RemoteImageScope) -> io::Result<()> {
        self.require_root_identity()?;
        if self
            .scopes
            .get(&(scope.connection_id, scope.terminal_session_id))
            != Some(scope)
        {
            return Err(rejected("image scope is stale"));
        }
        Ok(())
    }

    fn remove_terminal(&mut self, key: (Uuid, SessionId)) -> io::Result<()> {
        // 即使文件系统拒绝清理，也先撤销输入租约，禁止旧回调继续写入。
        self.scopes.remove(&key);
        self.require_root_identity()?;
        let transfers: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                (entry.scope.connection_id, entry.scope.terminal_session_id) == key
                    && !entry.published
            })
            .map(|(transfer_id, _)| *transfer_id)
            .collect();
        for transfer_id in transfers {
            let entry = self.entries.get(&transfer_id).ok_or_else(missing)?;
            remove_owned_file(entry)?;
            if let Some(entry) = self.entries.remove(&transfer_id) {
                self.reserved_bytes -= entry.spec.byte_len;
            }
        }
        self.used_ids.remove(&key);
        Ok(())
    }
}

impl Drop for RemoteImageStaging {
    fn drop(&mut self) {
        // 析构只作尽力回收，不能作为清理成功收据。身份不符时保留原路径；
        // 逐文件核验后只删除空目录，绝不递归删除外来替换项或新增文件。
        if self.require_root_identity().is_err() {
            return;
        }
        for entry in self.entries.values().filter(|entry| !entry.published) {
            let _ = remove_owned_file(entry);
        }
        if self.require_root_identity().is_ok() {
            let _ = fs::remove_dir(self.root.path());
        }
    }
}

fn require_owned_file(entry: &StagedImage) -> io::Result<()> {
    let named = fs::symlink_metadata(entry.file.path())?;
    let opened = entry.file.as_file().metadata()?;
    if !named.is_file()
        || named.file_type().is_symlink()
        || named.dev() != opened.dev()
        || named.ino() != opened.ino()
    {
        return Err(rejected("image staging file identity changed"));
    }
    Ok(())
}

fn remove_owned_file(entry: &StagedImage) -> io::Result<()> {
    require_owned_file(entry)?;
    fs::remove_file(entry.file.path())
}

fn require_entry_scope(entry: &StagedImage, scope: &RemoteImageScope) -> io::Result<()> {
    if entry.scope != *scope {
        return Err(rejected("image transfer belongs to another scope"));
    }
    Ok(())
}

fn rejected(message: &'static str) -> io::Error {
    // 稳定内部诊断，不能直接作为 GUI 文案；接线时映射中英文错误键。
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn missing() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "unknown image transfer")
}

#[cfg(test)]
#[path = "cli_image_staging_tests.rs"]
mod tests;
