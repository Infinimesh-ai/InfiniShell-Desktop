//! agent 是独立符号链接 artifact；不能复用只认领普通文件的补全账本。

use std::fs;
use std::os::unix::fs::{MetadataExt as _, symlink};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Error;
use super::package_tree::{Directory, Identity};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    target: PathBuf,
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
}

fn link(path: &Path) -> Result<Link, Error> {
    let before = fs::symlink_metadata(path).map_err(|_| Error::SourceChanged)?;
    if !before.file_type().is_symlink() || before.uid() != unsafe { libc::geteuid() } {
        return Err(Error::UnsupportedSource);
    }
    let target = fs::read_link(path).map_err(|_| Error::SourceChanged)?;
    let after = fs::symlink_metadata(path).map_err(|_| Error::SourceChanged)?;
    if before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.mode() != after.mode()
        || before.uid() != after.uid()
        || before.gid() != after.gid()
    {
        return Err(Error::SourceChanged);
    }
    Ok(Link {
        target,
        device: before.dev(),
        inode: before.ino(),
        uid: before.uid(),
        gid: before.gid(),
        mode: before.mode(),
    })
}

fn target(path: &Path, value: &Link) -> Result<PathBuf, Error> {
    let joined = path
        .parent()
        .ok_or(Error::SourceChanged)?
        .join(&value.target);
    let mut resolved = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::RootDir | Component::Normal(_) => resolved.push(component.as_os_str()),
            Component::ParentDir if resolved.parent().is_some() => {
                resolved.pop();
            }
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return Err(Error::SourceChanged),
        }
    }
    Ok(resolved)
}

pub(super) fn verify_registered(path: &Path, native: &Path) -> Result<(), Error> {
    super::plain_ancestors(path.parent().ok_or(Error::SourceChanged)?)?;
    let observed = link(path)?;
    if target(path, &observed)? != native
        || path.canonicalize().ok().as_deref() != Some(native)
        || link(path)? != observed
    {
        return Err(Error::SourceChanged);
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Alias {
    path: PathBuf,
    stage: PathBuf,
    parent: Identity,
    original: Link,
    prepared: Option<Link>,
}

impl Alias {
    pub(super) fn capture(prefix: &Path, id: Uuid, old_native: &Path) -> Result<Self, Error> {
        let path = prefix.join("bin/agent");
        verify_registered(&path, old_native)?;
        Ok(Self {
            parent: Directory::open(&prefix.join("bin"))?.identity()?,
            original: link(&path)?,
            path,
            stage: prefix
                .join("bin")
                .join(format!(".infinishell-brew-agent-{id}")),
            prepared: None,
        })
    }

    pub(super) fn validate(
        &self,
        prefix: &Path,
        id: Uuid,
        old_native: &Path,
        new_native: &Path,
    ) -> Result<(), Error> {
        if prefix != super::brew_grok::prefix()?
            || id.is_nil()
            || self.path != prefix.join("bin/agent")
            || self.stage
                != prefix
                    .join("bin")
                    .join(format!(".infinishell-brew-agent-{id}"))
            || target(&self.path, &self.original)? != old_native
            || self
                .prepared
                .as_ref()
                .is_some_and(|value| target(&self.path, value).ok().as_deref() != Some(new_native))
        {
            return Err(Error::RecoveryRequired);
        }
        self.verify_parent()
    }

    fn verify_parent(&self) -> Result<(), Error> {
        let parent = self.path.parent().ok_or(Error::SourceChanged)?;
        if Directory::open(parent)?.identity()? != self.parent {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }

    pub(super) fn prepare(&mut self, native: &Path) -> Result<(), Error> {
        self.verify_original()?;
        if self.prepared.is_some() {
            return Err(Error::RecoveryRequired);
        }
        symlink(native, &self.stage).map_err(|_| Error::SourceChanged)?;
        self.prepared = Some(link(&self.stage)?);
        super::sync_config_directory(self.path.parent().ok_or(Error::PersistenceFailed)?)
    }

    pub(super) fn verify_original(&self) -> Result<(), Error> {
        self.verify_parent()?;
        if link(&self.path)? != self.original {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }

    pub(super) fn verify_published(&self) -> Result<(), Error> {
        self.verify_parent()?;
        if Some(&link(&self.path)?) != self.prepared.as_ref() {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }

    fn exchange(&self, left: &Link, right: &Link) -> Result<(), Error> {
        self.verify_parent()?;
        if link(&self.path)? != *left || link(&self.stage)? != *right {
            return Err(Error::SourceChanged);
        }
        super::brew_transaction::exchange_paths(&self.path, &self.stage)?;
        super::sync_config_directory(self.path.parent().ok_or(Error::PersistenceFailed)?)?;
        // 交换后再次检查；并发改动保留在原处或 stage 中，绝不按路径盲删。
        if link(&self.path)? != *right || link(&self.stage)? != *left {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }

    pub(super) fn publish(&self) -> Result<(), Error> {
        self.exchange(
            &self.original,
            self.prepared.as_ref().ok_or(Error::RecoveryRequired)?,
        )
    }

    pub(super) fn rollback(&self) -> Result<(), Error> {
        self.verify_parent()?;
        let current = link(&self.path)?;
        if current == self.original {
            return Ok(());
        }
        if Some(&current) != self.prepared.as_ref() {
            return Err(Error::SourceChanged);
        }
        self.exchange(&current, &self.original)
    }

    pub(super) fn cleanup(&self, committed: bool) -> Result<(), Error> {
        if committed {
            self.verify_published()?;
        } else {
            self.verify_original()?;
        }
        let expected = if committed {
            Some(&self.original)
        } else {
            self.prepared.as_ref()
        };
        match fs::symlink_metadata(&self.stage) {
            Ok(_) => {
                if Some(&link(&self.stage)?) != expected {
                    return Err(Error::SourceChanged);
                }
                fs::remove_file(&self.stage).map_err(|_| Error::RecoveryRequired)?;
                super::sync_config_directory(self.path.parent().ok_or(Error::PersistenceFailed)?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(Error::RecoveryRequired),
        }
    }
}
