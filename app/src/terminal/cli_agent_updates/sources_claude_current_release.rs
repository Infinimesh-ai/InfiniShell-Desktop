//! 已审核的 Claude 消费者发行原件；不授予托管协议或原生历史兼容能力。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use super::Error;

pub(super) const V285: &str = "2.1.285";
pub(super) const V287: &str = "2.1.287";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Release {
    wrapper: Artifact,
    platform: Artifact,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    integrity: String,
    sha256: String,
    files: BTreeMap<PathBuf, File>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    length: u64,
    sha256: String,
    mode: u32,
}

pub(super) fn supports(version: &str, platform: &str) -> Result<(), Error> {
    if platform == "linux-x64-musl" {
        return if matches!(version, "2.1.278" | "2.1.280" | V285 | V287) {
            Ok(())
        } else {
            Err(Error::InvalidRelease)
        };
    }
    if !matches!(version, V285 | V287) {
        return Err(Error::InvalidRelease);
    }
    if platform != "darwin-arm64" {
        return Err(Error::UnsupportedPlatform);
    }
    Ok(())
}

fn release(version: &str, platform: &str) -> Result<Release, Error> {
    supports(version, platform)?;
    let bytes: &[u8] = if platform == "linux-x64-musl" {
        include_bytes!("sources_claude_musl_release.json")
    } else {
        include_bytes!("sources_claude_current_release.json")
    };
    let mut releases: BTreeMap<String, Release> =
        serde_json::from_slice(bytes).map_err(|_| Error::InvalidRelease)?;
    releases.remove(version).ok_or(Error::InvalidRelease)
}

/// Homebrew 的独立原件与 npm 平台包采用同一份已审核原生映像。
pub(super) fn native(version: &str, platform: &str) -> Result<(u64, [u8; 32]), Error> {
    let release = release(version, platform)?;
    let file = release
        .platform
        .files
        .get(&PathBuf::from("claude"))
        .ok_or(Error::InvalidRelease)?;
    Ok((file.length, super::brew::decode_sha256(&file.sha256)?))
}

pub(super) fn verify_metadata(
    version: &str,
    platform: &str,
    wrapper: &[u8],
    native: &[u8],
) -> Result<(), Error> {
    let release = release(version, platform)?;
    let platform_name = format!("@anthropic-ai/claude-code-{platform}");
    for (bytes, name, expected) in [
        (wrapper, "@anthropic-ai/claude-code", &release.wrapper),
        (native, platform_name.as_str(), &release.platform),
    ] {
        let value: Value = serde_json::from_slice(bytes).map_err(|_| Error::InvalidRelease)?;
        let short = name
            .strip_prefix("@anthropic-ai/")
            .ok_or(Error::InvalidRelease)?;
        if value["name"] != name
            || value["version"] != version
            || value["dist"]["integrity"] != expected.integrity
            || value["dist"]["tarball"]
                != format!("https://registry.npmjs.org/{name}/-/{short}-{version}.tgz")
            || value["dist"]["fileCount"].as_u64() != Some(expected.files.len() as u64)
            || value["dist"]["unpackedSize"].as_u64()
                != Some(expected.files.values().map(|file| file.length).sum())
        {
            return Err(Error::InvalidRelease);
        }
    }
    Ok(())
}

/// 已安装布局把 wrapper 占位入口替换为同版原生映像；恢复只接受这份完整清单。
pub(super) fn installed_files(
    version: &str,
    platform: &str,
) -> Result<BTreeMap<PathBuf, (u64, [u8; 32])>, Error> {
    let release = release(version, platform)?;
    let native = native(version, platform)?;
    let dependency = PathBuf::from(format!("node_modules/@anthropic-ai/claude-code-{platform}"));
    let mut files = BTreeMap::new();
    for (path, file) in release.wrapper.files {
        files.insert(
            path,
            (file.length, super::brew::decode_sha256(&file.sha256)?),
        );
    }
    for (path, file) in release.platform.files {
        files.insert(
            dependency.join(path),
            (file.length, super::brew::decode_sha256(&file.sha256)?),
        );
    }
    files.insert("bin/claude.exe".into(), native);
    Ok(files)
}

pub(super) fn verify_archive_digests(
    version: &str,
    platform: &str,
    wrapper: [u8; 32],
    native: [u8; 32],
) -> Result<(), Error> {
    let release = release(version, platform)?;
    if hex::encode(wrapper) != release.wrapper.sha256
        || hex::encode(native) != release.platform.sha256
    {
        return Err(Error::InvalidRelease);
    }
    Ok(())
}

#[cfg(all(
    feature = "local_fs",
    any(target_os = "macos", target_os = "linux", windows)
))]
pub(super) fn verify_archives(
    version: &str,
    platform: &str,
    wrapper: &super::npm_release::VerifiedNpmArchive,
    native: &super::npm_release::VerifiedNpmArchive,
) -> Result<(), Error> {
    let release = release(version, platform)?;
    for (archive, expected) in [(wrapper, release.wrapper), (native, release.platform)] {
        if hex::encode(archive.compressed_sha256) != expected.sha256
            || archive.files.len() != expected.files.len()
        {
            return Err(Error::InvalidRelease);
        }
        for (path, expected) in expected.files {
            let file = archive.files.get(&path).ok_or(Error::InvalidRelease)?;
            if file.length != expected.length
                || hex::encode(file.sha256) != expected.sha256
                || file.mode != expected.mode
                || file.executable != (expected.mode & 0o111 != 0)
            {
                return Err(Error::InvalidRelease);
            }
        }
    }
    Ok(())
}

#[cfg(all(
    test,
    feature = "local_fs",
    any(target_os = "macos", target_os = "linux", windows)
))]
#[path = "sources_claude_current_release_tests.rs"]
mod tests;

#[cfg(all(
    test,
    feature = "local_fs",
    any(target_os = "macos", target_os = "linux", windows)
))]
#[path = "sources_claude_musl_release_tests.rs"]
mod musl_tests;
