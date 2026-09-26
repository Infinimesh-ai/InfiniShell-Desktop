//! 固定 Claude 2.1.280 的原生收件箱与图片读取回执。
//!
//! cc-socks 只接收文本；真实图片由当前会话的 Read 工具读取，保留原生审批。
//! 写出文本不代表图片已消费；只有最终会话历史中的内联原图可产生回执。

use std::collections::HashSet;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::Shutdown;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

#[path = "cli_image_claude_transcript.rs"]
mod transcript;
pub(crate) use transcript::ClaudeTranscript;

const MAX_WIRE_BYTES: usize = 1024 * 1024;
const MAX_PRIVATE_JSON_BYTES: u64 = 16 * 1024;
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
// 原生历史可能同时保存 tool_result 与 toolUseResult 两份 base64，给恢复预算留余量。
const MAX_TOTAL_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_IMAGES: usize = 20;

/// 所有字段由服务端当前终端绑定产生，客户端不能直接指定任意路径或进程。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaudeInboxTarget {
    pub(crate) session_id: Uuid,
    pub(crate) cwd: PathBuf,
    pub(crate) process_id: u32,
    pub(crate) socket_path: PathBuf,
    pub(crate) registry_directory: PathBuf,
    pub(crate) transcript_path: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaudeQueueImage {
    pub(crate) path: PathBuf,
    pub(crate) byte_len: u64,
    pub(crate) sha256: [u8; 32],
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaudeTranscriptIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

/// 必须在任何原生输入写出之前持久化；该 UUID 不是原生去重承诺。
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaudeImageAttempt {
    pub(crate) client_message_id: Uuid,
    pub(crate) session_id: Uuid,
    pub(crate) request_sha256: [u8; 32],
    pub(crate) transcript_path: PathBuf,
    pub(crate) transcript_identity: ClaudeTranscriptIdentity,
    pub(crate) transcript_offset: u64,
    pub(crate) images: Vec<ClaudeQueueImage>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaudeImageReadReceipt {
    pub(crate) tool_use_id: String,
    pub(crate) assistant_message_uuid: Uuid,
    pub(crate) result_message_uuid: Uuid,
    pub(crate) image_sha256: [u8; 32],
    pub(crate) raw_result_sha256: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaudeImageReceipt {
    pub(crate) attempt: ClaudeImageAttempt,
    pub(crate) reads: Vec<ClaudeImageReadReceipt>,
}

// 密钥只存在于会清零的内存，不能派生 Debug 或 Serialize，也不能进入错误文本。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PeerKey {
    #[serde(deserialize_with = "deserialize_peer_token")]
    peer_token: Zeroizing<String>,
    proc_start: Option<String>,
    pid_domain: Option<String>,
}

fn deserialize_peer_token<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Zeroizing<String>, D::Error> {
    String::deserialize(deserializer).map(Zeroizing::new)
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct NativeRegistry {
    pid: u32,
    messaging_socket_path: PathBuf,
    cwd: PathBuf,
    session_id: Uuid,
    proc_start: String,
    pid_domain: Option<String>,
    kind: String,
}

pub(crate) struct ClaudeImageInbox {
    stream: UnixStream,
    target: ClaudeInboxTarget,
    registry_file: File,
    registry: NativeRegistry,
    key_file: File,
    key_path: PathBuf,
    key_bytes: Zeroizing<Vec<u8>>,
    key: PeerKey,
}

impl ClaudeImageInbox {
    /// validate 负责核验同一内核 peer、固定映像、当前 TUI/TTY 及实际 registry 归属。
    /// 只能在后台调用；不会启动 CLI、恢复会话或变更审批设置。
    pub(crate) fn connect(
        stream: UnixStream,
        target: ClaudeInboxTarget,
        validate: &impl Fn(&UnixStream) -> io::Result<()>,
    ) -> io::Result<Self> {
        validate_target(&target)?;
        validate(&stream)?;
        let registry_path = target
            .registry_directory
            .join(format!("{}.json", target.process_id));
        let mut registry_file = open_registry_file(&registry_path)?;
        let registry_bytes = read_small(&mut registry_file)?;
        let registry: NativeRegistry = serde_json::from_slice(&registry_bytes)
            .map_err(|_| rejected("invalid_native_registry"))?;
        if registry.pid != target.process_id
            || registry.session_id != target.session_id
            || registry.cwd != target.cwd
            || registry.messaging_socket_path != target.socket_path
            || registry.proc_start.is_empty()
            || registry.proc_start.len() > 128
            || registry.kind != "interactive"
        {
            return Err(rejected("native_registry_mismatch"));
        }
        // 固定版用 path.resolve(socket) 的 UTF-8 字节摘要命名 key，不跟随 socket 符号链接。
        let socket = target
            .socket_path
            .to_str()
            .ok_or_else(|| rejected("invalid_socket_path"))?;
        let socket_sha = Sha256::digest(socket.as_bytes());
        let key_path = target
            .registry_directory
            .join(format!("{}.{socket_sha:x}.key", target.process_id));
        let mut key_file = open_private_file(&key_path)?;
        let key_bytes = read_secret(&mut key_file)?;
        let key: PeerKey =
            serde_json::from_slice(&key_bytes).map_err(|_| rejected("invalid_peer_key"))?;
        if key.peer_token.len() != 32
            || !key
                .peer_token
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || key.proc_start.as_deref() != Some(registry.proc_start.as_str())
            || key.pid_domain != registry.pid_domain
        {
            return Err(rejected("peer_key_registry_mismatch"));
        }
        stream.set_nonblocking(false)?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        validate(&stream)?;
        Ok(Self {
            stream,
            target,
            registry_file,
            registry,
            key_file,
            key_path,
            key_bytes,
            key,
        })
    }

    /// 返回值仍为 Unknown，只说明一次写出已完成；调用者随后查询 transcript。
    /// before_write 必须先持久化 Unknown，再领取有效 lease；任何错误均不得重投。
    pub(crate) fn submit_once(
        mut self,
        client_message_id: Uuid,
        text: &str,
        images: &[ClaudeQueueImage],
        before_write: impl FnOnce(&ClaudeImageAttempt) -> io::Result<()>,
        validate: &impl Fn(&UnixStream) -> io::Result<()>,
    ) -> io::Result<ClaudeImageAttempt> {
        if client_message_id.is_nil() || text.len() > MAX_WIRE_BYTES / 2 || text.contains('\0') {
            return Err(rejected("invalid_request"));
        }
        let image_leases = prepare_images(images)?;
        let mut content = String::with_capacity(text.len() + images.len() * 256);
        content.push_str(text);
        // 这是发给原生模型的图片工具输入协议，不模拟附件成功，也不提供审批答复。
        content.push_str("\n\nRead each attached PNG with the Read tool before answering this message. Keep the native permission checks. Attached PNG paths (JSON strings):\n");
        for image in images {
            let path = image
                .path
                .to_str()
                .ok_or_else(|| rejected("invalid_image_path"))?;
            let encoded =
                serde_json::to_string(path).map_err(|_| rejected("invalid_image_path"))?;
            content.push_str(&encoded);
            content.push('\n');
        }
        let request = json!({
            "type": "user",
            "session_id": self.target.session_id,
            "uuid": client_message_id,
            "msg_id": client_message_id,
            "priority": "next",
            "message": { "role": "user", "content": content },
        });
        let mut encoded = serde_json::to_vec(&request).map_err(|_| rejected("request_encoding"))?;
        encoded.push(b'\n');
        let auth = Zeroizing::new(
            format!(
                "{{\"type\":\"auth\",\"token\":\"{}\"}}\n",
                self.key.peer_token.as_str()
            )
            .into_bytes(),
        );
        if encoded.len().saturating_add(auth.len()) > MAX_WIRE_BYTES {
            return Err(rejected("request_too_large"));
        }
        self.verify_private_binding()?;
        validate(&self.stream)?;
        let mut transcript = open_private_file(&self.target.transcript_path)?;
        let metadata = transcript.metadata()?;
        let offset = metadata.len();
        if offset != 0 {
            transcript.seek(SeekFrom::End(-1))?;
            let mut last = [0_u8; 1];
            transcript.read_exact(&mut last)?;
            if last[0] != b'\n' {
                return Err(rejected("transcript_append_in_progress"));
            }
        }
        let attempt = ClaudeImageAttempt {
            client_message_id,
            session_id: self.target.session_id,
            request_sha256: Sha256::digest(&encoded).into(),
            transcript_path: self.target.transcript_path.clone(),
            transcript_identity: identity(&metadata),
            transcript_offset: offset,
            images: images.to_vec(),
        };
        before_write(&attempt)?;
        self.verify_private_binding()?;
        validate(&self.stream)?;
        for (image, file) in images.iter().zip(&image_leases) {
            verify_file_path(file, &image.path)?;
        }
        verify_file_path(&transcript, &self.target.transcript_path)?;
        // auth 与 user 均为 JSONL。没有输入 ACK；写入、关闭或之后的错误都保留 Unknown。
        self.stream
            .write_all(&auth)
            .and_then(|()| self.stream.write_all(&encoded))
            .and_then(|()| self.stream.shutdown(Shutdown::Write))
            .map_err(|_| rejected("inbox_write_outcome_unknown"))?;
        Ok(attempt)
    }

    fn verify_private_binding(&mut self) -> io::Result<()> {
        let path = self
            .target
            .registry_directory
            .join(format!("{}.json", self.target.process_id));
        verify_registry_path(&self.registry_file, &path)?;
        verify_file_path(&self.key_file, &self.key_path)?;
        self.registry_file.seek(SeekFrom::Start(0))?;
        self.key_file.seek(SeekFrom::Start(0))?;
        let registry: NativeRegistry =
            serde_json::from_slice(&read_small(&mut self.registry_file)?)
                .map_err(|_| rejected("invalid_native_registry"))?;
        // 固定版原地更新 status/name/updatedAt；这些字段不改变当前会话绑定。
        if registry != self.registry {
            return Err(rejected("native_registry_changed"));
        }
        let key = read_secret(&mut self.key_file)?;
        if key.as_slice() != self.key_bytes.as_slice() {
            return Err(rejected("peer_key_changed"));
        }
        Ok(())
    }
}

fn validate_target(target: &ClaudeInboxTarget) -> io::Result<()> {
    if target.session_id.is_nil()
        || target.process_id <= 1
        || [
            &target.cwd,
            &target.socket_path,
            &target.registry_directory,
            &target.transcript_path,
        ]
        .into_iter()
        .any(|path| !is_absolute_normal_path(path))
        || target
            .registry_directory
            .file_name()
            .and_then(|name| name.to_str())
            != Some("sessions")
    {
        return Err(rejected("invalid_binding"));
    }
    let directory = fs::symlink_metadata(&target.registry_directory)?;
    if !directory.is_dir()
        || directory.uid() != unsafe { libc::geteuid() }
        || directory.mode() & 0o077 != 0
    {
        return Err(rejected("registry_directory_not_private"));
    }
    Ok(())
}

pub(super) fn is_absolute_normal_path(path: &Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some_and(|path| !path.contains('\0'))
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && path.components().collect::<PathBuf>().as_os_str() == path.as_os_str()
}

fn prepare_images(images: &[ClaudeQueueImage]) -> io::Result<Vec<File>> {
    if images.is_empty() || images.len() > MAX_IMAGES {
        return Err(rejected("invalid_image_count"));
    }
    let mut paths = HashSet::new();
    let mut leases = Vec::with_capacity(images.len());
    let mut total_bytes = 0_u64;
    for image in images {
        if !is_absolute_normal_path(&image.path)
            || !paths.insert(image.path.clone())
            || !(8..=MAX_IMAGE_BYTES).contains(&image.byte_len)
        {
            return Err(rejected("invalid_image"));
        }
        total_bytes = total_bytes
            .checked_add(image.byte_len)
            .filter(|total| *total <= MAX_TOTAL_IMAGE_BYTES)
            .ok_or_else(|| rejected("total_image_budget_exceeded"))?;
        let mut file = open_private_file(&image.path)?;
        if file.metadata()?.len() != image.byte_len {
            return Err(rejected("image_length_mismatch"));
        }
        let mut signature = [0_u8; 8];
        file.read_exact(&mut signature)?;
        if signature != *b"\x89PNG\r\n\x1a\n" {
            return Err(rejected("image_not_png"));
        }
        let mut digest = Sha256::new();
        digest.update(signature);
        let copied = io::copy(
            &mut (&mut file).take(image.byte_len - 8),
            &mut DigestWriter(&mut digest),
        )?;
        let sha: [u8; 32] = digest.finalize().into();
        if copied != image.byte_len - 8 || sha != image.sha256 {
            return Err(rejected("image_digest_mismatch"));
        }
        verify_file_path(&file, &image.path)?;
        leases.push(file);
    }
    Ok(leases)
}

pub(super) fn open_private_file(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    verify_file_path(&file, path)?;
    Ok(file)
}

fn open_registry_file(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    verify_registry_path(&file, path)?;
    Ok(file)
}

fn verify_registry_path(file: &File, path: &Path) -> io::Result<()> {
    let metadata = file.metadata()?;
    let current = fs::symlink_metadata(path)?;
    // 原生 PID JSON 使用默认文件 mode；0700 的 sessions 目录负责隔离读取。
    if !metadata.is_file()
        || !current.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
        || metadata.nlink() != 1
        || identity(&metadata) != identity(&current)
    {
        return Err(rejected("native_registry_identity_changed"));
    }
    Ok(())
}

pub(super) fn verify_file_path(file: &File, path: &Path) -> io::Result<()> {
    let metadata = file.metadata()?;
    let current = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || !current.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || identity(&metadata) != identity(&current)
    {
        return Err(rejected("private_file_identity_changed"));
    }
    Ok(())
}

pub(super) fn identity(metadata: &Metadata) -> ClaudeTranscriptIdentity {
    ClaudeTranscriptIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

fn read_small(file: &mut File) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(MAX_PRIVATE_JSON_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PRIVATE_JSON_BYTES {
        return Err(rejected("private_record_too_large"));
    }
    Ok(bytes)
}

fn read_secret(file: &mut File) -> io::Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_PRIVATE_JSON_BYTES as usize + 1));
    file.take(MAX_PRIVATE_JSON_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PRIVATE_JSON_BYTES {
        return Err(rejected("private_record_too_large"));
    }
    Ok(bytes)
}

pub(super) struct DigestWriter<'a>(pub(super) &'a mut Sha256);

impl Write for DigestWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn rejected(code: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, code)
}
