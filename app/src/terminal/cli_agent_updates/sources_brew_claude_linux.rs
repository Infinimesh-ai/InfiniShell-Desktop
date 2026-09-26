//! Linux x64 的固定 Claude cask 合同；仅认领官方数字版本，不求值 Ruby 或执行 zap。

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::package_tree::Directory;
use super::{Error, Installation, MAX_CONFIG, brew, read_limited, stamp};

pub(super) const TOKEN: &str = "claude-code@latest";
pub(super) const METADATA_URL: &str = "https://raw.githubusercontent.com/Homebrew/homebrew-cask/ef1bb0b080bda4075f4a8992bb6b10a0f0bc5bf7/Casks/c/claude-code@latest.rb";
const CASK_SHA256: &str = "c4173c146d619ef3902640d853af310b4ea8748add84e8ec518de61eb5781555";

pub(super) fn prefix() -> Result<&'static Path, Error> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err(Error::UnsupportedPlatform);
    }
    Ok(Path::new("/home/linuxbrew/.linuxbrew"))
}

pub(super) fn supports(version: &str) -> Result<(), Error> {
    prefix()?;
    if version != "2.1.280" {
        return Err(Error::InvalidRelease);
    }
    Ok(())
}

pub(super) fn native(version: &str) -> Result<(u64, [u8; 32]), Error> {
    prefix()?;
    // 摘要同时见固定官方 cask 与既有官方 release manifest；不称为发布签名。
    let (length, digest) = match version {
        "2.1.278" => (
            234_119_480,
            "5c4735937844e84f8a93306e841a5b0e12252909b07870f789b190468da147ab",
        ),
        "2.1.280" => (
            233_709_640,
            "1e08503dbdf3c2cb0d706d32f3408277388d1c76ef108673e8fe42c1b322925b",
        ),
        _ => return Err(Error::InvalidRelease),
    };
    Ok((length, brew::decode_sha256(digest)?))
}

fn artifacts() -> Value {
    // 7.0.4 的卸载 tab 保存原始参数；仅展示的 API target 字段不属于此结构。
    json!([
        {"binary":["claude"]},
        {"zap":[{"trash":[
            "~/.cache/claude", "~/.claude.json*", "~/.config/claude",
            "~/.local/bin/claude", "~/.local/share/claude", "~/.local/state/claude",
            "~/Library/Caches/claude-cli-nodejs"
        ],"rmdir":"~/.claude"}]}
    ])
}

pub(super) fn release(
    token: &str,
    version: &str,
    source: &[u8],
) -> Result<(Value, String, [u8; 32]), Error> {
    supports(version)?;
    if token != TOKEN
        || <[u8; 32]>::from(Sha256::digest(source)) != brew::decode_sha256(CASK_SHA256)?
    {
        return Err(Error::InvalidRelease);
    }
    Ok((
        json!({"token":TOKEN,"tap":"homebrew/cask","version":version,
            "tap_git_head":"ef1bb0b080bda4075f4a8992bb6b10a0f0bc5bf7",
            "artifacts":artifacts()}),
        format!("https://downloads.claude.ai/claude-code-releases/{version}/linux-x64/claude"),
        native(version)?.1,
    ))
}

fn verify_receipt(root: &Path, version: &str) -> Result<(), Error> {
    let receipt: Value = serde_json::from_slice(&read_limited(
        &root.join(".metadata/INSTALL_RECEIPT.json"),
        MAX_CONFIG,
    )?)
    .map_err(|_| Error::UnsupportedSource)?;
    let expected = artifacts();
    let expected = expected.as_array().ok_or(Error::InvalidRelease)?;
    let registered = receipt["uninstall_artifacts"]
        .as_array()
        .ok_or(Error::UnsupportedSource)?;
    // AbstractTab.create 使用 Hardware::CPU.arch；x64 是 x86_64，不是下载 URL 的 x64。
    // DependsOn 的空 formula/cask 数组不会自动登记宿主 glibc 或 gcc。
    if receipt["homebrew_version"] != "7.0.4"
        || receipt["arch"] != "x86_64"
        || receipt["runtime_dependencies"] != json!({})
        || receipt["uninstall_flight_blocks"] != false
        || receipt["source"]["tap"] != "homebrew/cask"
        || receipt["source"]["version"] != version
        || registered.len() != expected.len()
        || expected.iter().any(|artifact| {
            registered
                .iter()
                .filter(|actual| *actual == artifact)
                .count()
                != 1
        })
    {
        return Err(Error::UnsupportedSource);
    }
    Ok(())
}

/// 包树必须与数字版本 cask 的实际持久化布局相符；未知文件一律保留现场，不参与交换。
pub(super) fn verify_tree(root: &Path, version: &str) -> Result<(), Error> {
    let (length, digest) = native(version)?;
    if root.canonicalize().ok().as_deref() != Some(root) {
        return Err(Error::SourceChanged);
    }
    verify_receipt(root, version)?;
    let config: Value = serde_json::from_slice(&read_limited(
        &root.join(".metadata/config.json"),
        MAX_CONFIG,
    )?)
    .map_err(|_| Error::UnsupportedSource)?;
    if config.as_object().is_none_or(|fields| {
        fields.len() != 3
            || ["default", "env", "explicit"]
                .iter()
                .any(|field| !fields.get(*field).is_some_and(Value::is_object))
    }) {
        return Err(Error::UnsupportedSource);
    }
    let metadata_version = root.join(".metadata").join(version);
    let mut children = fs::read_dir(&metadata_version).map_err(|_| Error::SourceChanged)?;
    let timestamp = children
        .next()
        .transpose()
        .map_err(|_| Error::SourceChanged)?
        .ok_or(Error::UnsupportedSource)?;
    if children.next().is_some() {
        return Err(Error::UnsupportedSource);
    }
    let timestamp = timestamp.file_name();
    let timestamp = timestamp.to_str().ok_or(Error::UnsupportedSource)?;
    if timestamp.len() != 18
        || timestamp.as_bytes()[14] != b'.'
        || !timestamp
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 14 || byte.is_ascii_digit())
    {
        return Err(Error::UnsupportedSource);
    }
    let cask_relative = PathBuf::from(".metadata")
        .join(version)
        .join(timestamp)
        .join("Casks")
        .join(format!("{TOKEN}.json"));
    let installed: Value =
        serde_json::from_slice(&read_limited(&root.join(&cask_relative), MAX_CONFIG)?)
            .map_err(|_| Error::UnsupportedSource)?;
    if installed != json!({}) {
        return Err(Error::UnsupportedSource);
    }
    let native_relative = PathBuf::from(version).join("claude");
    let mut files = BTreeMap::from([(native_relative, (length, digest))]);
    for relative in [
        PathBuf::from(".metadata/INSTALL_RECEIPT.json"),
        PathBuf::from(".metadata/config.json"),
        cask_relative,
    ] {
        let path = root.join(&relative);
        let recorded = stamp(&path)?;
        if recorded.canonical != path {
            return Err(Error::SourceChanged);
        }
        files.insert(
            relative,
            (
                fs::metadata(path).map_err(|_| Error::SourceChanged)?.len(),
                recorded.digest,
            ),
        );
    }
    // @latest 是 token 后缀；version 是数字，installer 不生成 LATEST_DOWNLOAD_SHA256。
    Directory::open(root)?
        .snapshot()?
        .verify_release_files(&files)
}

pub(super) fn verify_registration(
    installation: &Installation,
    prefix: &Path,
    token: &str,
    version: &str,
    registered: &brew::Registration,
) -> Result<(), Error> {
    let (length, digest) = native(version)?;
    let native = registered.root.join(version).join("claude");
    let public = prefix.join("bin/claude");
    let link = fs::symlink_metadata(&public).map_err(|_| Error::SourceChanged)?;
    if prefix != self::prefix()?
        || token != TOKEN
        || registered.root != prefix.join("Caskroom").join(TOKEN)
        || registered.entry_relative != Path::new("claude")
        || installation.entry != public
        || !link.file_type().is_symlink()
        || link.uid() != unsafe { libc::geteuid() }
        || installation.stamp.canonical != native
        || installation.stamp.digest != digest
        || fs::metadata(&native)
            .map_err(|_| Error::SourceChanged)?
            .len()
            != length
        || stamp(&public)? != installation.stamp
    {
        return Err(Error::UnsupportedSource);
    }
    verify_tree(&registered.root, version)
}

pub(super) fn verify_recovery(
    prefix: &Path,
    token: &str,
    old_version: &str,
    target_version: &str,
) -> Result<(), Error> {
    supports(target_version)?;
    native(old_version)?;
    if prefix != self::prefix()? || token != TOKEN {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}
