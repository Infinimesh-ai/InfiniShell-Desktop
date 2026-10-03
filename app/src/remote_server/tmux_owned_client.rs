//! tmux 新窗格的本机持久意图；恢复只查询原 ID，不重新派发启动。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::proto::TerminalBindingOwnedAgent;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Launch {
    pub version: u32,
    pub host: String,
    pub terminal_session: u64,
    pub block_id: String,
    pub agent: i32,
    pub id: Uuid,
    pub key: Uuid,
}

pub(crate) struct Journal {
    directory: PathBuf,
}

impl Journal {
    pub(crate) fn open() -> io::Result<Self> {
        let database = crate::persistence::database_file_path_for_current_scope();
        Self::at(database.parent().ok_or_else(invalid)?)
    }

    fn at(parent: &Path) -> io::Result<Self> {
        let directory = parent.join("remote-tmux-owned-v1");
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
        Ok(Self { directory })
    }

    /// 只能由用户显式的新建操作调用；同 outer block 的不同新意图有不同文件。
    pub(crate) fn reserve(
        &self,
        host: String,
        terminal_session: u64,
        block_id: String,
        agent: TerminalBindingOwnedAgent,
    ) -> io::Result<Launch> {
        let launch = Launch {
            version: 1,
            host,
            terminal_session,
            block_id,
            agent: agent as i32,
            id: Uuid::new_v4(),
            key: Uuid::new_v4(),
        };
        validate(&launch)?;
        self.write(&format!("{}.json", launch.id), &launch)?;
        Ok(launch)
    }

    /// 在 Start RPC 前一次领取；失败、超时和进程重启都不能再次领取。
    pub(crate) fn claim_start(&self, launch: &Launch) -> io::Result<()> {
        if self.read(launch.id)? != *launch {
            return Err(invalid());
        }
        self.write(&format!("{}-start-unknown.json", launch.id), launch)
    }

    pub(crate) fn read(&self, id: Uuid) -> io::Result<Launch> {
        if id.is_nil() {
            return Err(invalid());
        }
        check_private(&self.directory, true)?;
        let path = self.directory.join(format!("{id}.json"));
        check_private(&path, false)?;
        let mut bytes = Vec::new();
        options()
            .read(true)
            .open(path)?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 {
            return Err(invalid());
        }
        let launch: Launch = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        validate(&launch)?;
        if launch.id != id {
            return Err(invalid());
        }
        Ok(launch)
    }

    /// 冷恢复只允许唯一原意图；不能在同一 outer block 的多个 pane 间猜测。
    pub(crate) fn unique_for_block(
        &self,
        host: Option<&str>,
        terminal_session: u64,
        block_id: &str,
    ) -> io::Result<Launch> {
        check_private(&self.directory, true)?;
        let mut found = None;
        for (index, entry) in fs::read_dir(&self.directory)?.enumerate() {
            if index >= 4096 {
                return Err(invalid());
            }
            let entry = entry?;
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
                .and_then(|name| Uuid::parse_str(name).ok())
            else {
                continue;
            };
            let launch = self.read(id)?;
            if host.is_none_or(|host| launch.host == host)
                && launch.terminal_session == terminal_session
                && launch.block_id == block_id
            {
                if found.is_some() {
                    return Err(invalid());
                }
                found = Some(launch);
            }
        }
        found.ok_or_else(invalid)
    }

    fn write(&self, name: &str, launch: &Launch) -> io::Result<()> {
        check_private(&self.directory, true)?;
        let bytes = serde_json::to_vec(launch).map_err(io::Error::other)?;
        let mut file = options()
            .write(true)
            .create_new(true)
            .open(self.directory.join(name))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

fn validate(launch: &Launch) -> io::Result<()> {
    if launch.version != 1
        || launch.id.is_nil()
        || launch.key.is_nil()
        || launch.host.is_empty()
        || launch.host.len() > 4096
        || launch.terminal_session == 0
        || launch.block_id.is_empty()
        || launch.block_id.len() > 4096
        || !matches!(
            TerminalBindingOwnedAgent::try_from(launch.agent),
            Ok(TerminalBindingOwnedAgent::Claude
                | TerminalBindingOwnedAgent::Codex
                | TerminalBindingOwnedAgent::Grok)
        )
    {
        return Err(invalid());
    }
    Ok(())
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
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || !directory && metadata.nlink() != 1
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

fn invalid() -> io::Error {
    io::Error::other("tmux 启动意图不可用")
}

#[cfg(test)]
#[path = "tmux_owned_client_tests.rs"]
mod tests;
