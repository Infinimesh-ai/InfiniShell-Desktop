//! 只冻结会话通知资源；终端路由由原生 pager 的当前连接独立验证和更新。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MANIFEST: &[u8] =
    include_bytes!("../../../assets/bundled/cli-agent-plugins/grok/.grok-plugin/plugin.json");
const HOOKS: &[u8] =
    include_bytes!("../../../assets/bundled/cli-agent-plugins/grok/hooks/hooks.json");
const NOTIFY: &[u8] =
    include_bytes!("../../../assets/bundled/cli-agent-plugins/grok/hooks/notify.cjs");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanos: i64,
    changed_seconds: i64,
    changed_nanos: i64,
    mode: u32,
    uid: u32,
}

impl Identity {
    fn capture(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanos: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanos: metadata.ctime_nsec(),
            mode: metadata.mode(),
            uid: metadata.uid(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenFile {
    identity: Identity,
    sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NotificationPlan {
    version: u32,
    pub(crate) session_id: Uuid,
    root: PathBuf,
    cwd: PathBuf,
    worker: PathBuf,
    worker_identity: Identity,
    directories: BTreeMap<String, Identity>,
    files: BTreeMap<String, FrozenFile>,
}

#[derive(Serialize)]
struct Descriptor<'a> {
    version: u32,
    session_id: Uuid,
    cwd: &'a Path,
    plugin_dir: PathBuf,
    files: BTreeMap<&'static str, String>,
}

impl NotificationPlan {
    pub(crate) fn create(
        parent: &Path,
        session_id: Uuid,
        cwd: &Path,
        worker: &Path,
    ) -> io::Result<Self> {
        private_directory(parent)?;
        if session_id.is_nil()
            || cwd.canonicalize()? != cwd
            || !cwd.is_dir()
            || worker.canonicalize()? != worker
        {
            return Err(invalid());
        }
        let root = parent.join("notifications");
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        for relative in ["plugin", "plugin/.grok-plugin", "plugin/hooks"] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(root.join(relative))?;
        }
        let resources = resources(worker)?;
        let descriptor = Descriptor {
            version: 1,
            session_id,
            cwd,
            plugin_dir: root.join("plugin"),
            files: resources
                .iter()
                .map(|(name, bytes)| (*name, digest(bytes)))
                .collect(),
        };
        for (relative, bytes) in &resources {
            write_new(&root.join("plugin").join(relative), bytes)?;
        }
        write_new(
            &root.join("binding.json"),
            &serde_json::to_vec(&descriptor).map_err(io::Error::other)?,
        )?;
        let mut files = BTreeMap::new();
        for relative in [
            "binding.json",
            "plugin/.grok-plugin/plugin.json",
            "plugin/hooks/hooks.json",
            "plugin/hooks/notify.cjs",
        ] {
            files.insert(relative.into(), frozen_file(&root.join(relative))?);
        }
        let mut directories = BTreeMap::new();
        for relative in ["", "plugin", "plugin/.grok-plugin", "plugin/hooks"] {
            File::open(root.join(relative))?.sync_all()?;
            directories.insert(
                relative.into(),
                Identity::capture(&private_directory(&root.join(relative))?),
            );
        }
        File::open(parent)?.sync_all()?;
        let plan = Self {
            version: 1,
            session_id,
            root,
            cwd: cwd.to_owned(),
            worker: worker.to_owned(),
            worker_identity: worker_identity(worker)?,
            directories,
            files,
        };
        plan.verify(session_id, cwd, worker)?;
        Ok(plan)
    }

    pub(crate) fn verify_parent(&self, parent: &Path) -> io::Result<()> {
        if self.root != parent.join("notifications") {
            return Err(invalid());
        }
        self.verify_current()
    }

    pub(crate) fn descriptor_path(&self) -> PathBuf {
        self.root.join("binding.json")
    }

    pub(crate) fn verify(&self, session_id: Uuid, cwd: &Path, worker: &Path) -> io::Result<()> {
        self.validate_binding(session_id, cwd, worker)?;
        if cwd.canonicalize()? != cwd
            || !cwd.is_dir()
            || worker.canonicalize()? != worker
            || worker_identity(worker)? != self.worker_identity
        {
            return Err(invalid());
        }
        for (relative, entries) in [
            ("", vec!["binding.json", "plugin"]),
            ("plugin", vec![".grok-plugin", "hooks"]),
            ("plugin/.grok-plugin", vec!["plugin.json"]),
            ("plugin/hooks", vec!["hooks.json", "notify.cjs"]),
        ] {
            let path = self.root.join(relative);
            let identity = Identity::capture(&private_directory(&path)?);
            if self.directories.get(relative) != Some(&identity) {
                return Err(invalid());
            }
            let mut actual = fs::read_dir(&path)?
                .take(4)
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<Vec<_>>>()?;
            actual.sort();
            if actual
                != entries
                    .iter()
                    .map(|value| std::ffi::OsString::from(*value))
                    .collect::<Vec<_>>()
            {
                return Err(invalid());
            }
        }
        let resources = resources(worker)?;
        let descriptor = Descriptor {
            version: 1,
            session_id,
            cwd,
            plugin_dir: self.root.join("plugin"),
            files: resources
                .iter()
                .map(|(name, bytes)| (*name, digest(bytes)))
                .collect(),
        };
        let mut expected = resources
            .into_iter()
            .map(|(name, bytes)| (format!("plugin/{name}"), bytes))
            .collect::<BTreeMap<_, _>>();
        expected.insert(
            "binding.json".into(),
            serde_json::to_vec(&descriptor).map_err(io::Error::other)?,
        );
        for (relative, bytes) in expected {
            let actual = frozen_file(&self.root.join(&relative))?;
            if self.files.get(&relative) != Some(&actual) || actual.sha256 != digest(&bytes) {
                return Err(invalid());
            }
        }
        // 读取期间发生目录或 worker 替换也必须使本次验证失败。
        for (relative, expected) in &self.directories {
            if Identity::capture(&private_directory(&self.root.join(relative))?) != *expected {
                return Err(invalid());
            }
        }
        if worker_identity(worker)? != self.worker_identity {
            return Err(invalid());
        }
        Ok(())
    }

    /// 恢复查询与退出回收不执行 hook；资源丢失不能阻止已捕获原生进程的生命周期核对。
    pub(crate) fn validate_binding(
        &self,
        session_id: Uuid,
        cwd: &Path,
        worker: &Path,
    ) -> io::Result<()> {
        if self.version != 1
            || session_id.is_nil()
            || self.session_id != session_id
            || self.cwd != cwd
            || !cwd.is_absolute()
            || self.worker != worker
            || !worker.is_absolute()
            || !self.root.is_absolute()
            || self.root.file_name().and_then(|value| value.to_str()) != Some("notifications")
            || self.directories.len() != 4
            || self.files.len() != 4
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub(crate) fn verify_session(&self, session_id: Uuid, cwd: &Path) -> io::Result<()> {
        self.verify(session_id, cwd, &self.worker)
    }

    pub(crate) fn verify_current(&self) -> io::Result<()> {
        self.verify(self.session_id, &self.cwd, &self.worker)
    }

    /// 原生只读证明与输入前置使用同一完整描述符；对象字段顺序不参与身份判断。
    pub(crate) fn descriptor(&self) -> io::Result<serde_json::Value> {
        self.verify_current()?;
        let resources = resources(&self.worker)?;
        let descriptor = Descriptor {
            version: 1,
            session_id: self.session_id,
            cwd: &self.cwd,
            plugin_dir: self.root.join("plugin"),
            files: resources
                .iter()
                .map(|(name, bytes)| (*name, digest(bytes)))
                .collect(),
        };
        let value = serde_json::to_value(descriptor).map_err(io::Error::other)?;
        self.verify_current()?;
        Ok(value)
    }
}

fn resources(worker: &Path) -> io::Result<BTreeMap<&'static str, Vec<u8>>> {
    let worker = worker.to_str().ok_or_else(invalid)?;
    let mut hooks: serde_json::Value = serde_json::from_slice(HOOKS).map_err(io::Error::other)?;
    for rules in hooks["hooks"]
        .as_object_mut()
        .ok_or_else(invalid)?
        .values_mut()
    {
        let handler = rules[0]["hooks"][0].as_object_mut().ok_or_else(invalid)?;
        handler.insert(
            "env".into(),
            json!({
                "WARP_CLI_AGENT_PROTOCOL_VERSION": "1",
                "WARP_CLI_AGENT_NOTIFY_EXECUTABLE": worker,
                "WARP_CLI_AGENT_SESSION_ROUTE": "1",
            }),
        );
    }
    Ok(BTreeMap::from([
        (".grok-plugin/plugin.json", MANIFEST.to_vec()),
        (
            "hooks/hooks.json",
            serde_json::to_vec(&hooks).map_err(io::Error::other)?,
        ),
        ("hooks/notify.cjs", NOTIFY.to_vec()),
    ]))
}

fn private_directory(path: &Path) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if path.canonicalize()? != path
        || !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(invalid());
    }
    Ok(metadata)
}

fn worker_identity(path: &Path) -> io::Result<Identity> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    let identity = Identity::capture(&metadata);
    if !metadata.is_file()
        || ![0, unsafe { libc::geteuid() }].contains(&metadata.uid())
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
        || Identity::capture(&fs::symlink_metadata(path)?) != identity
    {
        return Err(invalid());
    }
    Ok(identity)
}

fn frozen_file(path: &Path) -> io::Result<FrozenFile> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.len() > 256 * 1024
    {
        return Err(invalid());
    }
    let identity = Identity::capture(&metadata);
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    if Identity::capture(&file.metadata()?) != identity {
        return Err(invalid());
    }
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    let mut total = 0;
    loop {
        let length = file.read(&mut bytes)?;
        if length == 0 {
            break;
        }
        total += length;
        if total > 256 * 1024 {
            return Err(invalid());
        }
        hash.update(&bytes[..length]);
    }
    if Identity::capture(&file.metadata()?) != identity
        || Identity::capture(&fs::symlink_metadata(path)?) != identity
    {
        return Err(invalid());
    }
    Ok(FrozenFile {
        identity,
        sha256: format!("{:x}", hash.finalize()),
    })
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn invalid() -> io::Error {
    io::Error::other("Grok 会话通知冻结资源不匹配")
}

#[cfg(test)]
#[path = "grok_notification_plan_tests.rs"]
mod tests;
