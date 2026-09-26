//! 固定 Grok cask 的公开源码与原生文件合同；不求值 Ruby，也不执行 zap。

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::{Error, Installation, MAX_CONFIG, brew, read_limited, stamp};

pub(super) const METADATA_URL: &str = "https://raw.githubusercontent.com/Homebrew/homebrew-cask/52a97ac96ae1af1a764eaefe0bdcad4296a12541/Casks/g/grok-build.rb";
const CASK_SHA256: &str = "623f5dedb31afda33d630de3e023748142e5465edbfdc0594f50ddf1c4552ec1";

pub(super) fn prefix() -> Result<&'static Path, Error> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok(Path::new("/opt/homebrew")),
        ("linux", "x86_64") => Ok(Path::new("/home/linuxbrew/.linuxbrew")),
        _ => Err(Error::UnsupportedPlatform),
    }
}

pub(super) fn supports(version: &str) -> Result<(), Error> {
    prefix()?;
    if version != "1.0.41" {
        return Err(Error::InvalidRelease);
    }
    Ok(())
}

pub(super) fn native(version: &str) -> Result<(u64, [u8; 32]), Error> {
    supports("1.0.41")?;
    let (length, digest) = match (std::env::consts::OS, version) {
        // 旧版摘要来自既有官方 URL 完整下载记录；不是发布签名。
        ("macos", "1.0.40") => (
            145_308_720,
            "3f2aef9618191a2c60d18a5044fa462c9c77bdc4187b02ed716b0394e8d4fef2",
        ),
        ("macos", "1.0.41") => (
            145_657_952,
            "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d",
        ),
        ("linux", "1.0.40") => (
            165_587_968,
            "92c997dfd109c0672d40d5ae6fbd15835d53ffaf12cf9ea124d22aaef3ff23fc",
        ),
        ("linux", "1.0.41") => (
            165_967_424,
            "9ce03ed23e16ea01072b4496263d6213a27899e1e3e107f008d36edf82e70407",
        ),
        _ => return Err(Error::InvalidRelease),
    };
    Ok((length, brew::decode_sha256(digest)?))
}

pub(super) fn entry(version: &str) -> Result<PathBuf, Error> {
    native(version)?;
    let target = if cfg!(target_os = "linux") {
        "linux-x86_64"
    } else {
        "macos-aarch64"
    };
    Ok(format!("grok-{version}-{target}").into())
}

fn artifacts(version: &str) -> Result<Value, Error> {
    let binary = entry(version)?;
    // Homebrew 7.0.4 的 uninstall tab 使用原始参数，不含 API 的独立 target 字段。
    Ok(json!([
        {"binary":[binary,{"target":"grok"}]},
        {"binary":[binary,{"target":"agent"}]},
        {"generate_completions_from_executable":[binary,"completions",{"base_name":"grok","shell_parameter_format":null,"shells":["bash","zsh","fish"]}]},
        {"zap":[{"rmdir":"~/.grok"}]}
    ]))
}

pub(super) fn release(version: &str, bytes: &[u8]) -> Result<(Value, String, [u8; 32]), Error> {
    supports(version)?;
    if <[u8; 32]>::from(Sha256::digest(bytes)) != brew::decode_sha256(CASK_SHA256)? {
        return Err(Error::InvalidRelease);
    }
    Ok((
        json!({"token":"grok-build","tap":"homebrew/cask","version":version,
        "tap_git_head":"52a97ac96ae1af1a764eaefe0bdcad4296a12541","artifacts":artifacts(version)?}),
        format!(
            "https://x.ai/cli/{}",
            entry(version)?.to_str().ok_or(Error::InvalidRelease)?
        ),
        native(version)?.1,
    ))
}

/// 必须同时有真实 tab、固定原生映像与两个归属一致的公开别名；用户自有 agent 不被认领。
pub(super) fn verify_registration(
    installation: &Installation,
    prefix: &Path,
    version: &str,
    registered: &brew::Registration,
) -> Result<(), Error> {
    let relative = entry(version)?;
    let (length, digest) = native(version)?;
    let native = registered.root.join(version).join(&relative);
    if prefix != self::prefix()?
        || registered.root != prefix.join("Caskroom/grok-build")
        || registered.entry_relative != relative
        || installation.stamp.canonical != native
        || installation.stamp.digest != digest
        || std::fs::metadata(&native)
            .map_err(|_| Error::SourceChanged)?
            .len()
            != length
    {
        return Err(Error::UnsupportedSource);
    }
    let receipt: Value = serde_json::from_slice(&read_limited(&registered.receipt, MAX_CONFIG)?)
        .map_err(|_| Error::UnsupportedSource)?;
    let expected_artifacts = artifacts(version)?;
    let registered_artifacts = receipt["uninstall_artifacts"]
        .as_array()
        .ok_or(Error::UnsupportedSource)?;
    let expected_artifacts = expected_artifacts.as_array().ok_or(Error::InvalidRelease)?;
    let arch = if cfg!(target_os = "linux") {
        "x86_64"
    } else {
        "arm64"
    };
    if receipt["arch"] != arch
        || receipt["homebrew_version"] != "7.0.4"
        || receipt["runtime_dependencies"] != json!({})
        || registered_artifacts.len() != expected_artifacts.len()
        || expected_artifacts.iter().any(|expected| {
            registered_artifacts
                .iter()
                .filter(|actual| *actual == expected)
                .count()
                != 1
        })
    {
        return Err(Error::UnsupportedSource);
    }
    for command in ["grok", "agent"] {
        let public = prefix.join("bin").join(command);
        super::brew_grok_aliases::verify_registered(&public, &native)?;
        if stamp(&public)? != installation.stamp {
            return Err(Error::SourceChanged);
        }
    }
    Ok(())
}

pub(super) fn completion_paths(prefix: &Path) -> [(&'static str, PathBuf); 3] {
    [
        ("bash", prefix.join("etc/bash_completion.d/grok")),
        ("zsh", prefix.join("share/zsh/site-functions/_grok")),
        (
            "fish",
            prefix.join("share/fish/vendor_completions.d/grok.fish"),
        ),
    ]
}
