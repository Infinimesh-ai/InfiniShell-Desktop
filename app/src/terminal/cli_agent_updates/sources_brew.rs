//! Homebrew 来源以实际入口、cask 收据和版本目录共同认领，不执行任意 cask Ruby。

use std::path::{Path, PathBuf};

use futures::StreamExt as _;
use serde_json::Value;

use super::{CLIAgent, Error, Installation, MAX_CONFIG, Stamp, read_limited, stamp};

/// 前缀只从已经选中的公共入口推导，不能从 cask 原生映像或恢复账本另选安装。
pub(super) fn prefix_from_entry(agent: CLIAgent, entry: &Path) -> Result<PathBuf, Error> {
    if !matches!(agent, CLIAgent::Claude | CLIAgent::Codex | CLIAgent::Grok)
        || entry.file_name().and_then(|name| name.to_str()) != Some(agent.command_prefix())
    {
        return Err(Error::UnsupportedSource);
    }
    let bin = entry.parent().ok_or(Error::UnsupportedSource)?;
    if bin.file_name().and_then(|name| name.to_str()) != Some("bin") {
        return Err(Error::UnsupportedSource);
    }
    let prefix = bin.parent().ok_or(Error::UnsupportedSource)?;
    validate_prefix(prefix)?;
    Ok(prefix.to_owned())
}

pub(super) fn validate_prefix(prefix: &Path) -> Result<(), Error> {
    #[cfg(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64"))]
    {
        validate_macos_prefix(prefix)
    }
    #[cfg(not(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64")))]
    {
        if cfg!(all(target_os = "linux", target_arch = "x86_64"))
            && prefix == Path::new("/home/linuxbrew/.linuxbrew")
        {
            Ok(())
        } else {
            Err(Error::UnsupportedPlatform)
        }
    }
}

/// 更新不运行 brew，但管理器公共入口也必须仍指向该前缀内同一份文件。
pub(super) fn manager_stamp(prefix: &Path) -> Result<Stamp, Error> {
    validate_prefix(prefix)?;
    let manager = stamp(&prefix.join("bin/brew"))?;
    if !manager.canonical.starts_with(prefix) {
        return Err(Error::UnsupportedSource);
    }
    #[cfg(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64"))]
    {
        use std::os::unix::fs::MetadataExt as _;

        validate_macos_prefix(manager.canonical.parent().ok_or(Error::UnsupportedSource)?)?;
        let file = std::fs::File::open(&manager.canonical).map_err(|_| Error::SourceChanged)?;
        let metadata = file.metadata().map_err(|_| Error::SourceChanged)?;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o7022 != 0
            || metadata.nlink() != 1
        {
            return Err(Error::UnsupportedSource);
        }
        super::package_tree::reject_extra_permissions(&file)?;
    }
    Ok(manager)
}

#[cfg(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64"))]
fn validate_macos_prefix(prefix: &Path) -> Result<(), Error> {
    use std::os::unix::fs::MetadataExt as _;

    if !prefix.is_absolute() || prefix.canonicalize().ok().as_deref() != Some(prefix) {
        return Err(Error::UnsupportedSource);
    }
    let uid = unsafe { libc::geteuid() };
    for ancestor in prefix.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| Error::SourceChanged)?;
        if !metadata.is_dir()
            || metadata.uid() != 0 && metadata.uid() != uid
            || metadata.mode() & 0o022 != 0
        {
            return Err(Error::UnsupportedSource);
        }
        reject_writable_ancestor_acl(ancestor)?;
    }
    // 前缀本身继续使用事务目录的当前用户、mode 和无额外 ACL 约束。
    super::package_tree::Directory::open(prefix)?.identity()?;
    Ok(())
}

#[cfg(all(feature = "local_fs", target_os = "macos", target_arch = "aarch64"))]
fn reject_writable_ancestor_acl(path: &Path) -> Result<(), Error> {
    use std::ffi::{CString, c_void};
    use std::os::unix::ffi::OsStrExt as _;

    unsafe extern "C" {
        fn acl_get_file(path: *const libc::c_char, kind: libc::c_int) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, id: libc::c_int, entry: *mut *mut c_void)
        -> libc::c_int;
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut libc::c_int) -> libc::c_int;
        fn acl_get_permset(entry: *mut c_void, permissions: *mut *mut c_void) -> libc::c_int;
        fn acl_get_perm_np(permissions: *mut c_void, permission: libc::c_int) -> libc::c_int;
        fn acl_free(acl: *mut c_void) -> libc::c_int;
    }
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| Error::UnsupportedSource)?;
    // sys/acl.h：ACL_TYPE_EXTENDED；只读和 deny 条目不增加目录写权限。
    let acl = unsafe { acl_get_file(path.as_ptr(), 0x100) };
    if acl.is_null() {
        return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            Ok(())
        } else {
            Err(Error::UnsupportedSource)
        };
    }
    let result = (|| {
        let mut id = 0; // ACL_FIRST_ENTRY；后续为 ACL_NEXT_ENTRY。
        loop {
            let mut entry = std::ptr::null_mut();
            let status = unsafe { acl_get_entry(acl, id, &mut entry) };
            if status != 0 {
                return if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL) {
                    Ok(())
                } else {
                    Err(Error::UnsupportedSource)
                };
            }
            id = -1;
            let mut tag = 0;
            if unsafe { acl_get_tag_type(entry, &mut tag) } != 0 || !matches!(tag, 1 | 2) {
                return Err(Error::UnsupportedSource);
            }
            if tag == 2 {
                continue;
            }
            let mut permissions = std::ptr::null_mut();
            if unsafe { acl_get_permset(entry, &mut permissions) } != 0 {
                return Err(Error::UnsupportedSource);
            }
            // 写入、删除、追加、删除子项、属性、扩展属性、安全描述和所有者。
            for bit in [2, 4, 5, 6, 8, 10, 12, 13] {
                if unsafe { acl_get_perm_np(permissions, 1 << bit) } != 0 {
                    return Err(Error::UnsupportedSource);
                }
            }
        }
    })();
    unsafe { acl_free(acl) };
    result
}

pub(super) struct Registration {
    pub(super) root: PathBuf,
    pub(super) receipt: PathBuf,
    pub(super) receipt_stamp: Stamp,
    pub(super) entry_relative: PathBuf,
}

pub(super) async fn metadata(token: &str) -> Result<Value, Error> {
    if !matches!(
        token,
        "codex" | "claude-code" | "claude-code@latest" | "grok-build"
    ) {
        return Err(Error::UnsupportedSource);
    }
    // 来源检查只读取官方 JSON 与本地 tab，不为 info 查询执行用户可改写的 Ruby/cask。
    let url = format!("https://formulae.brew.sh/api/cask/{token}.json");
    let response = http_client::Client::new()
        .get(&url)
        .timeout(super::PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() || response.url().as_str() != url {
        return Err(Error::Network);
    }
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Network)?;
        if bytes.len() + chunk.len() > MAX_CONFIG as usize {
            return Err(Error::InvalidRelease);
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidRelease)?;
    if value["token"] != token || value["tap"] != "homebrew/cask" {
        return Err(Error::UnsupportedSource);
    }
    Ok(value)
}

pub(super) fn registration(
    agent: CLIAgent,
    installation: &Installation,
    prefix: &Path,
    token: &str,
    version: &str,
) -> Result<Registration, Error> {
    let command = match (agent, token) {
        (CLIAgent::Claude, "claude-code" | "claude-code@latest") => "claude",
        (CLIAgent::Codex, "codex") => "codex",
        (CLIAgent::Grok, "grok-build") => "grok",
        _ => return Err(Error::UnsupportedSource),
    };
    super::parse_version(version)?;
    let root = prefix.join("Caskroom").join(token);
    if prefix_from_entry(agent, &installation.entry)?.as_path() != prefix
        || prefix.canonicalize().ok().as_deref() != Some(prefix)
        || root.canonicalize().ok().as_deref() != Some(root.as_path())
        || installation.entry != prefix.join("bin").join(command)
        || !std::fs::symlink_metadata(&installation.entry)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::UnsupportedSource);
    }
    let manager = manager_stamp(prefix)?;
    if installation
        .manager
        .as_ref()
        .is_some_and(|expected| *expected != manager)
    {
        return Err(Error::SourceChanged);
    }
    let entry_relative = installation
        .stamp
        .canonical
        .strip_prefix(root.join(version))
        .map_err(|_| Error::UnsupportedSource)?
        .to_owned();
    if entry_relative.as_os_str().is_empty() {
        return Err(Error::UnsupportedSource);
    }
    let receipt = root.join(".metadata/INSTALL_RECEIPT.json");
    let value: Value = serde_json::from_slice(&read_limited(&receipt, MAX_CONFIG)?)
        .map_err(|_| Error::UnsupportedSource)?;
    if value.pointer("/source/tap").and_then(Value::as_str) != Some("homebrew/cask")
        || value.pointer("/source/version").and_then(Value::as_str) != Some(version)
        || value
            .get("uninstall_flight_blocks")
            .and_then(Value::as_bool)
            != Some(false)
    {
        return Err(Error::UnsupportedSource);
    }
    let registered = Registration {
        root,
        receipt_stamp: stamp(&receipt)?,
        receipt,
        entry_relative,
    };
    if agent == CLIAgent::Grok {
        #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
        super::brew_grok::verify_registration(installation, prefix, version, &registered)?;
        #[cfg(not(all(feature = "local_fs", any(target_os = "macos", target_os = "linux"))))]
        return Err(Error::UnsupportedPlatform);
    }
    #[cfg(all(feature = "local_fs", target_os = "linux"))]
    if agent == CLIAgent::Claude {
        super::brew_claude_linux::verify_registration(
            installation,
            prefix,
            token,
            version,
            &registered,
        )?;
    }
    Ok(registered)
}

pub(super) fn supports(agent: CLIAgent, version: &str) -> Result<(), Error> {
    if !cfg!(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64")
    )) {
        return Err(Error::UnsupportedPlatform);
    }
    if agent == CLIAgent::Grok {
        #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
        return super::brew_grok::supports(version);
        #[cfg(not(all(feature = "local_fs", any(target_os = "macos", target_os = "linux"))))]
        return Err(Error::UnsupportedPlatform);
    }
    if agent == CLIAgent::Codex {
        #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
        return super::brew_codex::supports(version);
        #[cfg(not(all(feature = "local_fs", any(target_os = "macos", target_os = "linux"))))]
        return Err(Error::UnsupportedPlatform);
    }
    if agent != CLIAgent::Claude {
        return Err(Error::UnsupportedSource);
    }
    if cfg!(target_os = "linux") {
        #[cfg(all(feature = "local_fs", target_os = "linux"))]
        return super::brew_claude_linux::supports(version);
        #[cfg(not(all(feature = "local_fs", target_os = "linux")))]
        return Err(Error::UnsupportedPlatform);
    }
    if version != "2.1.280"
        && !(cfg!(all(target_os = "macos", target_arch = "aarch64"))
            && super::claude_current_release::supports(version, "darwin-arm64").is_ok())
    {
        return Err(Error::InvalidRelease);
    }
    Ok(())
}

/// 只有无依赖、无安装脚本的官方单二进制 cask 可由宿主构造；zap 只保存，不执行。
pub(super) fn release(
    token: &str,
    version: &str,
    bytes: &[u8],
) -> Result<(Value, String, [u8; 32]), Error> {
    if token == "grok-build" {
        #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
        return super::brew_grok::release(version, bytes);
        #[cfg(not(all(feature = "local_fs", any(target_os = "macos", target_os = "linux"))))]
        return Err(Error::UnsupportedPlatform);
    }
    if token == "codex" {
        #[cfg(all(feature = "local_fs", any(target_os = "macos", target_os = "linux")))]
        return super::brew_codex::release(version, bytes);
        #[cfg(not(all(feature = "local_fs", any(target_os = "macos", target_os = "linux"))))]
        return Err(Error::UnsupportedPlatform);
    }
    if cfg!(target_os = "linux") {
        #[cfg(all(feature = "local_fs", target_os = "linux"))]
        return super::brew_claude_linux::release(token, version, bytes);
        #[cfg(not(all(feature = "local_fs", target_os = "linux")))]
        return Err(Error::UnsupportedPlatform);
    }
    supports(CLIAgent::Claude, version)?;
    if !matches!(token, "claude-code" | "claude-code@latest") || bytes.len() > MAX_CONFIG as usize {
        return Err(Error::InvalidRelease);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| Error::InvalidRelease)?;
    if value.get("token").and_then(Value::as_str) != Some(token)
        || value.get("tap").and_then(Value::as_str) != Some("homebrew/cask")
        || value.get("version").and_then(Value::as_str) != Some(version)
        || value.get("disabled").and_then(Value::as_bool) == Some(true)
        || !value
            .get("depends_on")
            .is_some_and(|value| value.as_object().is_some_and(|value| value.is_empty()))
        || value.get("container").is_some_and(|value| !value.is_null())
        || value
            .get("rename")
            .is_some_and(|value| value.as_array().is_none_or(|value| !value.is_empty()))
    {
        return Err(Error::InvalidRelease);
    }
    let artifacts = value
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidRelease)?;
    let mut binary = false;
    for artifact in artifacts {
        let fields = artifact.as_object().ok_or(Error::InvalidRelease)?;
        if fields.get("binary") == Some(&serde_json::json!(["claude"]))
            && fields.len() == 2
            && fields.get("target").and_then(Value::as_str) == Some("$HOMEBREW_PREFIX/bin/claude")
            && !binary
        {
            binary = true;
        } else if fields.len() != 1 || !fields.contains_key("zap") {
            return Err(Error::UnsupportedSource);
        }
    }
    if !binary {
        return Err(Error::InvalidRelease);
    }
    let cpu = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        _ => return Err(Error::UnsupportedPlatform),
    };
    let url =
        format!("https://downloads.claude.ai/claude-code-releases/{version}/darwin-{cpu}/claude");
    // 当前 API 的默认记录为 arm64；其他架构只接受明确同 URL 的变体。
    let selected = std::iter::once(&value)
        .chain(
            value
                .get("variations")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|map| map.values()),
        )
        .find(|item| item.get("url").and_then(Value::as_str) == Some(url.as_str()))
        .ok_or(Error::UnsupportedPlatform)?;
    let sha = selected
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or(Error::InvalidRelease)?;
    let digest = decode_sha256(sha)?;
    if version != "2.1.280"
        && digest != super::claude_current_release::native(version, "darwin-arm64")?.1
    {
        return Err(Error::InvalidRelease);
    }
    Ok((value, url, digest))
}

pub(super) fn decode_sha256(value: &str) -> Result<[u8; 32], Error> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::InvalidRelease);
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| Error::InvalidRelease)?;
    }
    Ok(digest)
}

#[cfg(all(
    test,
    feature = "local_fs",
    target_os = "macos",
    target_arch = "aarch64"
))]
#[path = "sources_brew_tests.rs"]
mod tests;
