//! 普通本地 shell 的 Grok 桥接发现目录；只提供位置，不授予连接或提交权限。

use std::fs::{self, DirBuilder, Metadata};
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
use std::path::{Component, Path, PathBuf};

use nix::unistd::{Uid, User};
use sha2::{Digest as _, Sha256};
use warp_core::channel::{Channel, ChannelState};

pub(crate) fn root() -> io::Result<PathBuf> {
    let owner = unsafe { libc::geteuid() };
    // getpwuid_r 返回系统账号目录；环境 HOME 与调用者环境覆盖都不能选定发现位置。
    let user = User::from_uid(Uid::from_raw(owner))
        .map_err(io::Error::other)?
        .ok_or_else(|| invalid("系统账号主目录不可用"))?;
    prepare_root(
        &user.dir,
        &directory_name(
            ChannelState::channel(),
            ChannelState::data_profile().as_deref(),
        ),
        owner,
    )
}

fn directory_name(channel: Channel, profile: Option<&str>) -> String {
    // 渠道和 profile 使用固定长度摘要，既保持隔离，也不让长 profile 撑满 socket 路径。
    let mut digest = Sha256::new();
    digest.update(channel.to_string().as_bytes());
    digest.update([0]);
    match profile {
        Some(profile) => {
            digest.update([1]);
            digest.update(profile.as_bytes());
        }
        None => digest.update([0]),
    }
    let digest = format!("{:x}", digest.finalize());
    format!(".isp-grok-{}", &digest[..24])
}

fn prepare_root(home: &Path, name: &str, owner: u32) -> io::Result<PathBuf> {
    validate_ancestors(home, owner)?;
    let home_before = fs::symlink_metadata(home)?;
    if home_before.uid() != owner {
        return Err(invalid("系统账号主目录不属于当前用户"));
    }
    let root = home.join(name);
    if root.parent() != Some(home)
        || root
            .join("t-00000000000000000000000000000000/control.sock")
            .as_os_str()
            .as_bytes()
            .len()
            >= 104
    {
        return Err(invalid("Grok 桥接发现路径超过原生 socket 限制"));
    }
    match DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    validate_ancestors(&root, owner)?;
    let metadata = fs::symlink_metadata(&root)?;
    let home_after = fs::symlink_metadata(home)?;
    if metadata.uid() != owner
        || metadata.mode() & 0o7777 != 0o700
        || metadata.dev() != home_before.dev()
        || identity(&home_before) != identity(&home_after)
    {
        return Err(invalid("Grok 桥接发现目录权限或设备身份无效"));
    }
    Ok(root)
}

fn validate_ancestors(path: &Path, owner: u32) -> io::Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
        || fs::canonicalize(path)? != path
    {
        return Err(invalid("Grok 桥接发现目录必须是无链接的绝对路径"));
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)?;
        if !metadata.is_dir()
            || (metadata.uid() != owner && metadata.uid() != 0)
            || metadata.mode() & 0o022 != 0
        {
            return Err(invalid("Grok 桥接发现目录包含不可信祖先"));
        }
    }
    Ok(())
}

fn identity(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(test)]
#[path = "grok_native_bridge_root_tests.rs"]
mod tests;
