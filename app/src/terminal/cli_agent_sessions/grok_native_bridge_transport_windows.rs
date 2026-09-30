//! 普通 Windows Grok 桥的只读发现与一次请求传输，不枚举进程或重投正文。

#[path = "grok_owned_windows_files.rs"]
mod files;

use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::time::Instant;

use command::managed::WindowsProcessLease;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use tokio::runtime::{Builder, Handle, Runtime};
use uuid::Uuid;

use super::super::grok_native_bridge_identity::{
    ARTIFACT_SHA256, Artifact, NativeBridgeProcess, verify_terminal,
};
use super::super::grok_native_bridge_root::pin_ancestors;
use super::{
    Envelope, IO_TIMEOUT, MAX_FRAME_BYTES, Request, canonical_uuid, invalid, lower_hex,
    valid_instance_name,
};
use crate::terminal::model::local_pty_identity::LocalPtyIdentity;

const MAX_MANIFEST_BYTES: u64 = 4096;
const MAX_DISCOVERY_ENTRIES: usize = 1024;

/// Windows 身份单独存储，不将登录会话或 ConPTY 代际伪装成 Unix 设备字段。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowsNativeBridgeBinding {
    pub instance_id: Uuid,
    pub native_process: NativeBridgeProcess,
    pub shell: NativeBridgeProcess,
    pub artifact_sha256: String,
    pub terminal_generation: Uuid,
}

pub(crate) use WindowsNativeBridgeBinding as NativeBridgeBinding;

pub(super) struct Transport {
    locator: Locator,
    pty: LocalPtyIdentity,
    peer: WindowsProcessLease,
    artifact: Artifact,
    binding: WindowsNativeBridgeBinding,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    protocol_version: u32,
    native_pid: u32,
    instance_id: String,
    socket: PathBuf,
    token: String,
}

// 含 token 的对象不实现 Debug；错误也不携带清单或响应原文。
struct Locator {
    root: PathBuf,
    root_file: File,
    root_identity: files::FileIdentity,
    directory: PathBuf,
    directory_file: File,
    directory_identity: files::FileIdentity,
    manifest_file: File,
    manifest_identity: files::FileIdentity,
    manifest_bytes: Vec<u8>,
    manifest: Manifest,
    _ancestors: Vec<File>,
}

impl Transport {
    pub(super) fn discover(root: &Path, pty: LocalPtyIdentity) -> io::Result<Option<Self>> {
        require_blocking_thread()?;
        match fs::symlink_metadata(root) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
        let _ancestors = pin_ancestors(root)?;
        let root_file = files::directory(root)?;
        let root_identity = files::file_identity(&root_file)?;
        pty.current_shell()?;
        let runtime = runtime()?;
        let mut found = None;
        for (index, entry) in fs::read_dir(root)?.enumerate() {
            if index >= MAX_DISCOVERY_ENTRIES {
                return Err(invalid());
            }
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_instance_name(&name) {
                continue;
            }
            // 别的实例可能尚未发布清单；不完整记录不成为候选，也不取得写入授权。
            let locator = match Locator::read(root, root_identity, entry.path()) {
                Ok(locator) => locator,
                Err(_) => continue,
            };
            let connection = match connect(&runtime, &locator.manifest.socket) {
                Ok(connection) => connection,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let peer = WindowsProcessLease::from_named_pipe_peer(&connection)?;
            let shell = match verify_terminal(&peer, &pty) {
                Ok(shell) => shell,
                Err(_) => continue,
            };
            if found.is_some() || peer.identity().pid != locator.manifest.native_pid {
                return Err(invalid());
            }
            let artifact = Artifact::capture(&peer)?;
            let binding = WindowsNativeBridgeBinding {
                instance_id: canonical_uuid(&locator.manifest.instance_id)?,
                native_process: peer.identity().into(),
                shell: shell.into(),
                artifact_sha256: ARTIFACT_SHA256.to_owned(),
                terminal_generation: pty.generation(),
            };
            let transport = Self {
                locator,
                pty: pty.clone(),
                peer,
                artifact,
                binding,
            };
            transport.verify(&transport.peer)?;
            found = Some(transport);
        }
        if files::file_identity(&root_file)? != root_identity
            || files::file_identity(&files::directory(root)?)? != root_identity
        {
            return Err(invalid());
        }
        pty.current_shell()?;
        Ok(found)
    }

    pub(super) fn binding(&self) -> &WindowsNativeBridgeBinding {
        &self.binding
    }

    pub(super) fn exchange(
        &self,
        request: Request<'_>,
        lease_deadline: Option<Instant>,
        admit: impl FnOnce() -> bool,
    ) -> io::Result<Vec<u8>> {
        require_blocking_thread()?;
        let bytes = serde_json::to_vec(&Envelope {
            token: &self.locator.manifest.token,
            request,
        })
        .map_err(|_| invalid())?;
        if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
            return Err(invalid());
        }
        let deadline = lease_deadline
            .map(|lease| lease.min(Instant::now() + IO_TIMEOUT))
            .unwrap_or_else(|| Instant::now() + IO_TIMEOUT);
        time_left(deadline)?;
        self.verify(&self.peer)?;
        let runtime = runtime()?;
        let mut connection = connect(&runtime, &self.locator.manifest.socket)?;
        let connected_peer = WindowsProcessLease::from_named_pipe_peer(&connection)?;
        self.verify(&connected_peer)?;
        connected_peer.validate_named_pipe_peer(&connection)?;
        let response = exchange_frame(&runtime, &mut connection, &bytes, deadline, admit)?;
        // 对端可在完整回包后关闭管道；继续验证已保留的真实进程句柄，不能另信同名端点。
        self.verify(&connected_peer)?;
        time_left(deadline)?;
        Ok(response)
    }

    fn verify(&self, peer: &WindowsProcessLease) -> io::Result<()> {
        self.locator.validate()?;
        self.peer.validate()?;
        if peer.identity() != self.peer.identity()
            || peer.identity().pid != self.locator.manifest.native_pid
            || self.pty.generation() != self.binding.terminal_generation
        {
            return Err(invalid());
        }
        if NativeBridgeProcess::from(verify_terminal(peer, &self.pty)?) != self.binding.shell {
            return Err(invalid());
        }
        self.artifact.verify(peer)?;
        if NativeBridgeProcess::from(verify_terminal(peer, &self.pty)?) != self.binding.shell {
            return Err(invalid());
        }
        self.locator.validate()
    }
}

impl Locator {
    fn read(
        root: &Path,
        expected_root: files::FileIdentity,
        directory: PathBuf,
    ) -> io::Result<Self> {
        if directory.parent() != Some(root) {
            return Err(invalid());
        }
        let ancestors = pin_ancestors(&directory)?;
        let root_file = files::directory(root)?;
        let root_identity = files::file_identity(&root_file)?;
        if root_identity != expected_root {
            return Err(invalid());
        }
        let directory_file = files::directory(&directory)?;
        let directory_identity = files::file_identity(&directory_file)?;
        let mut manifest_file = files::open_private(&directory.join("manifest.json"))?;
        let manifest_identity = files::file_identity(&manifest_file)?;
        let mut manifest_bytes = Vec::new();
        (&mut manifest_file)
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut manifest_bytes)?;
        let manifest = decode_manifest(&manifest_bytes, &directory)?;
        let locator = Self {
            root: root.to_owned(),
            root_file,
            root_identity,
            directory,
            directory_file,
            directory_identity,
            manifest_file,
            manifest_identity,
            manifest_bytes,
            manifest,
            _ancestors: ancestors,
        };
        locator.validate()?;
        Ok(locator)
    }

    fn validate(&self) -> io::Result<()> {
        files::plain_path(&self.directory)?;
        if files::file_identity(&self.root_file)? != self.root_identity
            || files::file_identity(&self.directory_file)? != self.directory_identity
            || files::file_identity(&files::directory(&self.root)?)? != self.root_identity
            || files::file_identity(&files::directory(&self.directory)?)? != self.directory_identity
            || files::file_identity(&self.manifest_file)? != self.manifest_identity
        {
            return Err(invalid());
        }
        let mut file = files::open_private(&self.directory.join("manifest.json"))?;
        if files::file_identity(&file)? != self.manifest_identity {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes != self.manifest_bytes {
            return Err(invalid());
        }
        Ok(())
    }
}

fn decode_manifest(bytes: &[u8], directory: &Path) -> io::Result<Manifest> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid());
    }
    let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let instance = canonical_uuid(&manifest.instance_id)?;
    if manifest.protocol_version != 1
        || manifest.native_pid == 0
        || directory.file_name() != Some(format!("t-{}", instance.simple()).as_ref())
        || manifest.socket != pipe_name(instance)
        || manifest.token.len() != 64
        || !manifest.token.bytes().all(lower_hex)
    {
        return Err(invalid());
    }
    Ok(manifest)
}

fn pipe_name(instance: Uuid) -> PathBuf {
    PathBuf::from(format!(
        r"\\.\pipe\infinishell-grok-terminal-bridge-v1-{}",
        instance.simple()
    ))
}

fn require_blocking_thread() -> io::Result<()> {
    if Handle::try_current().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "grok_native_bridge.background",
        ));
    }
    Ok(())
}

fn runtime() -> io::Result<Runtime> {
    Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
}

fn connect(runtime: &Runtime, name: &Path) -> io::Result<NamedPipeClient> {
    let _entered = runtime.enter();
    // Tokio 默认 SECURITY_IDENTIFICATION；服务端能核验登录身份，但不能模拟宿主权限。
    ClientOptions::new().open(name)
}

fn exchange_frame(
    runtime: &Runtime,
    connection: &mut NamedPipeClient,
    bytes: &[u8],
    deadline: Instant,
    admit: impl FnOnce() -> bool,
) -> io::Result<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err(invalid());
    }
    time_left(deadline)?;
    if !admit() {
        return Err(invalid());
    }
    time_left(deadline)?;
    runtime.block_on(async {
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
            connection
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .await?;
            connection.write_all(bytes).await?;
            let mut prefix = [0u8; 4];
            connection.read_exact(&mut prefix).await?;
            let length = u32::from_be_bytes(prefix) as usize;
            if length == 0 || length > MAX_FRAME_BYTES {
                return Err(invalid());
            }
            let mut response = vec![0u8; length];
            connection.read_exact(&mut response).await?;
            // 只确认传输帧已读完，让服务端安全释放管道；此字节不是原生输入接收回执。
            connection.write_all(&[1]).await?;
            Ok(response)
        })
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
    })
}

fn time_left(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::Error::from(io::ErrorKind::TimedOut));
    }
    Ok(())
}

#[cfg(test)]
#[path = "grok_native_bridge_transport_windows_tests.rs"]
mod tests;
