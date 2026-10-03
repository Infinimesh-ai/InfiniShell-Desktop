//! Linux x64 的固定 Claude cask 合同；仅认领官方数字版本，不求值 Ruby 或执行 zap。

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use futures::StreamExt as _;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::package_tree::Directory;
use super::{Error, Installation, MAX_CONFIG, brew, read_limited, stamp};

pub(super) const TOKEN: &str = "claude-code@latest";
pub(super) const METADATA_URL: &str = "https://raw.githubusercontent.com/Homebrew/homebrew-cask/ef1bb0b080bda4075f4a8992bb6b10a0f0bc5bf7/Casks/c/claude-code@latest.rb";
const CASK_SHA256: &str = "c4173c146d619ef3902640d853af310b4ea8748add84e8ec518de61eb5781555";
const MIGRATION_TAP: &str = "8d9df9ae501d586458789ecbf01115d91fe838c0";

fn token(version: &str) -> Result<&'static str, Error> {
    match version {
        "2.1.278" | "2.1.280" | "2.1.287" => Ok(TOKEN),
        "2.1.285" => Ok("claude-code"),
        _ => Err(Error::InvalidRelease),
    }
}

pub(super) fn metadata_url(token: &str, version: &str) -> Result<String, Error> {
    prefix()?;
    let (tap, _) = source_contract(token, version)?;
    Ok(format!(
        "https://raw.githubusercontent.com/Homebrew/homebrew-cask/{tap}/Casks/c/{token}.rb"
    ))
}

fn source_contract(token: &str, version: &str) -> Result<(&'static str, &'static str), Error> {
    match (token, version) {
        (TOKEN, "2.1.280") => Ok(("ef1bb0b080bda4075f4a8992bb6b10a0f0bc5bf7", CASK_SHA256)),
        ("claude-code", "2.1.285") => Ok((
            MIGRATION_TAP,
            "afd076e38f356a677ed9abfafca8305299f91454b36b30e50e635a4eb546dd12",
        )),
        (TOKEN, "2.1.287") => Ok((
            MIGRATION_TAP,
            "30834127bab1cf3bda09f66ca9243b10b8a864ef3b3e8cdd7af68a68f47a4d5b",
        )),
        _ => Err(Error::InvalidRelease),
    }
}

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

pub(super) fn supports_transition(installed: &str, target: &str) -> Result<(), Error> {
    prefix()?;
    if matches!(
        (installed, target),
        ("2.1.278" | "2.1.280", "2.1.280")
            | ("2.1.285", "2.1.285")
            | ("2.1.287", "2.1.287" | "2.1.285")
    ) {
        Ok(())
    } else {
        Err(Error::InvalidRelease)
    }
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
        "2.1.285" => (
            240_327_864,
            "33dad1ec615a2e08cc78b494f05c110e49916de2c79d78ec8799ebf46b233d29",
        ),
        "2.1.287" => (
            244_317_368,
            "3920489a5109cff5786a1a392c25277408ff22bc796d5edb9c16a60e5a1718f0",
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
    prefix()?;
    let (tap, checksum) = source_contract(token, version)?;
    if source.len() > MAX_CONFIG as usize
        || <[u8; 32]>::from(Sha256::digest(source)) != brew::decode_sha256(checksum)?
    {
        return Err(Error::InvalidRelease);
    }
    let url =
        format!("https://downloads.claude.ai/claude-code-releases/{version}/linux-x64/claude");
    let digest = native(version)?.1;
    let mut metadata = json!({"token":token,"tap":"homebrew/cask","version":version,
        "tap_git_head":tap,"artifacts":artifacts()});
    if matches!(version, "2.1.285" | "2.1.287") {
        // 固定原 Ruby 的 Linux 分支：只投影已审核字段，不求值 Ruby，也不使用 Mac 摘要。
        metadata["artifacts"][0]["target"] = json!("$HOMEBREW_PREFIX/bin/claude");
        metadata["url"] = json!(url);
        metadata["sha256"] = json!(hex::encode(digest));
        metadata["url_specs"] = json!({});
        metadata["depends_on"] = json!({});
        metadata["ruby_source_path"] = json!(format!("Casks/c/{token}.rb"));
        metadata["ruby_source_checksum"] = json!({"sha256":checksum});
        metadata["conflicts_with"] = json!({"cask":[if token == TOKEN {
            "claude-code"
        } else {
            TOKEN
        }]});
    }
    Ok((metadata, url, digest))
}

pub(super) async fn migration_metadata() -> Result<(Value, Value), Error> {
    prefix()?;
    // 已安装的 Latest 固定为 287；仅目标 Stable 必须仍是官方当前指针。
    if brew::metadata("claude-code").await?["version"] != "2.1.285" {
        return Err(Error::InvalidRelease);
    }
    let mut values = Vec::new();
    for (token, version) in [(TOKEN, "2.1.287"), ("claude-code", "2.1.285")] {
        let url = metadata_url(token, version)?;
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
        values.push(release(token, version, &bytes)?.0);
    }
    let target = values.pop().ok_or(Error::InvalidRelease)?;
    let old = values.pop().ok_or(Error::InvalidRelease)?;
    Ok((old, target))
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
    if matches!(version, "2.1.285" | "2.1.287") {
        let source_path = receipt["source"]["path"]
            .as_str()
            .map(Path::new)
            .ok_or(Error::UnsupportedSource)?;
        if !source_path.is_absolute()
            || source_path.file_name().and_then(|name| name.to_str())
                != Some(format!("{}.rb", token(version)?).as_str())
            || !receipt["source"]["tap_git_head"]
                .as_str()
                .is_some_and(|head| {
                    head.len() == 40 && head.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        {
            return Err(Error::UnsupportedSource);
        }
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
        .join(format!("{}.json", token(version)?));
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
    // version 是数字；两个 token 的 installer 均不生成 LATEST_DOWNLOAD_SHA256。
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
        || token != self::token(version)?
        || registered.root != prefix.join("Caskroom").join(token)
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
    if prefix != self::prefix()? || token != TOKEN || !matches!(old_version, "2.1.278" | "2.1.280")
    {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

#[cfg(test)]
#[path = "sources_brew_claude_linux_tests.rs"]
mod tests;
