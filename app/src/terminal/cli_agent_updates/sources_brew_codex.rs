//! Codex 固定 cask 与完整资源归档；各版本、平台分别绑定已审核的逐文件发行清单。

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Seek as _, SeekFrom};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::package_tree::Directory;
use super::{Error, brew};

const FIXED_PACKAGES: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/codex_0156_package_manifest.json");
const MACOS_0160_PACKAGE: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/codex_0160_macos_package_manifest.json");

struct Contract {
    packages: &'static [u8],
    metadata_url: &'static str,
    cask_sha256: &'static str,
    tap_git_head: &'static str,
}

fn contract(version: &str) -> Result<Contract, Error> {
    match version {
        "0.156.1" => Ok(Contract {
            packages: FIXED_PACKAGES,
            metadata_url: "https://raw.githubusercontent.com/Homebrew/homebrew-cask/8fb173ad115785e9dd050af916e14b3d3af26d65/Casks/c/codex.rb",
            cask_sha256: "65252c5f63b2c39fe18622bea057df0b5bbf1d1c42777507b925c4e6e057c20b",
            tap_git_head: "8fb173ad115785e9dd050af916e14b3d3af26d65",
        }),
        "0.160.0" if cfg!(all(target_os = "macos", target_arch = "aarch64")) => Ok(Contract {
            packages: MACOS_0160_PACKAGE,
            metadata_url: "https://raw.githubusercontent.com/Homebrew/homebrew-cask/b5244981746375f8dee94b80c7f7738ff105256e/Casks/c/codex.rb",
            cask_sha256: "163cad3efc91c2489633d78346d12c8c37fc45b77aa06ec9ee9e324e6ce2efd6",
            tap_git_head: "b5244981746375f8dee94b80c7f7738ff105256e",
        }),
        _ => Err(Error::InvalidRelease),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    archive: String,
    bytes: u64,
    sha256: String,
    target: String,
    entrypoint: String,
    directories: BTreeMap<PathBuf, u32>,
    files: BTreeMap<PathBuf, (u64, String, u32)>,
    metadata: Value,
}

fn package(version: &str) -> Result<Package, Error> {
    let contract = contract(version)?;
    let (key, target, archive, digest) = if cfg!(all(target_os = "macos", target_arch = "aarch64"))
    {
        (
            "macos-arm64",
            "aarch64-apple-darwin",
            "codex-package-aarch64-apple-darwin.tar.gz",
            if version == "0.160.0" {
                "007df41b607dbbc8d204b9746ce7fed2d4ce6c813f44c32ceee54175ca796525"
            } else {
                "fea42f9625091f011e38f059da974d52e57ba31831648bb1c7f0b1a385fde547"
            },
        )
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        (
            "linux-x64",
            "x86_64-unknown-linux-musl",
            "codex-package-x86_64-unknown-linux-musl.tar.gz",
            "8b711520beddf385467b8da4d2c93736637c6ba1e46811cf0d8606b7c490b6f6",
        )
    } else {
        return Err(Error::UnsupportedPlatform);
    };
    let root: Value =
        serde_json::from_slice(contract.packages).map_err(|_| Error::InvalidRelease)?;
    if root["version"] != version || root["release_tag"] != format!("rust-v{version}") {
        return Err(Error::InvalidRelease);
    }
    let package: Package =
        serde_json::from_value(root["packages"][key].clone()).map_err(|_| Error::InvalidRelease)?;
    if package.target != target
        || package.entrypoint != "bin/codex"
        || package.archive != archive
        || package.sha256 != digest
        || package.metadata
            != json!({"layoutVersion":1,"version":version,"target":target,
            "variant":"codex","entrypoint":"bin/codex","resourcesDir":"codex-resources","pathDir":"codex-path"})
    {
        return Err(Error::InvalidRelease);
    }
    Ok(package)
}

pub(super) fn supports(version: &str) -> Result<(), Error> {
    package(version).map(|_| ())
}

pub(super) fn supports_installed(version: &str) -> Result<(), Error> {
    // 0.155.1 是既有认领的旧安装；后续已提交版本也必须具有本平台发行合同。
    supports(if version == "0.155.1" {
        "0.156.1"
    } else {
        version
    })
}

pub(super) fn metadata_url(version: &str) -> Result<&'static str, Error> {
    supports(version)?;
    Ok(contract(version)?.metadata_url)
}

fn archive_url(version: &str, package: &Package) -> String {
    format!(
        "https://github.com/openai/codex/releases/download/rust-v{version}/{}",
        package.archive
    )
}

pub(super) fn is_archive_url(url: &str) -> bool {
    ["0.156.1", "0.160.0"]
        .into_iter()
        .any(|version| package(version).is_ok_and(|package| archive_url(version, &package) == url))
}

pub(super) fn release(version: &str, source: &[u8]) -> Result<(Value, String, [u8; 32]), Error> {
    supports(version)?;
    let contract = contract(version)?;
    if <[u8; 32]>::from(Sha256::digest(source)) != brew::decode_sha256(contract.cask_sha256)? {
        return Err(Error::InvalidRelease);
    }
    let package = package(version)?;
    let url = archive_url(version, &package);
    // 这不是 Ruby 求值：仅把已固定完整源码的三个已审核 artifact 转成 Homebrew tab 结构。
    let metadata = json!({"token":"codex","tap":"homebrew/cask","version":version,
        "tap_git_head":contract.tap_git_head,
        "artifacts":[{"binary":["bin/codex"]},
            {"generate_completions_from_executable":["bin/codex","completion",{"base_name":null,"shell_parameter_format":null,"shells":["bash","zsh","fish"]}]},
            {"zap":[{"rmdir":"~/.codex"}]}]});
    Ok((metadata, url, brew::decode_sha256(&package.sha256)?))
}

pub(super) fn unpack(
    input: &mut File,
    stage: &Directory,
    version: &str,
) -> Result<(BTreeMap<PathBuf, (u64, [u8; 32])>, BTreeMap<PathBuf, bool>), Error> {
    supports(version)?;
    let package = package(version)?;
    if input.metadata().map_err(|_| Error::InvalidRelease)?.len() != package.bytes {
        return Err(Error::InvalidRelease);
    }
    input
        .seek(SeekFrom::Start(0))
        .map_err(|_| Error::InvalidRelease)?;
    let mut archive = tar::Archive::new(GzDecoder::new(input));
    let mut seen = BTreeSet::new();
    let mut files = BTreeMap::new();
    let mut executables = BTreeMap::new();
    for entry in archive.entries().map_err(|_| Error::InvalidRelease)? {
        let mut entry = entry.map_err(|_| Error::InvalidRelease)?;
        let path = entry
            .path()
            .map_err(|_| Error::InvalidRelease)?
            .into_owned();
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !seen.insert(path.clone())
            || seen.len() > package.files.len() + package.directories.len()
            || entry
                .link_name()
                .map_err(|_| Error::InvalidRelease)?
                .is_some()
        {
            return Err(Error::InvalidRelease);
        }
        let header = entry.header();
        let length = header.size().map_err(|_| Error::InvalidRelease)?;
        let mode = header.mode().map_err(|_| Error::InvalidRelease)?;
        if header.entry_type().is_dir() {
            if length != 0 || package.directories.get(&path) != Some(&mode) {
                return Err(Error::InvalidRelease);
            }
            continue;
        }
        let (expected_length, digest, expected_mode) =
            package.files.get(&path).ok_or(Error::InvalidRelease)?;
        if !header.entry_type().is_file() || length != *expected_length || mode != *expected_mode {
            return Err(Error::InvalidRelease);
        }
        let digest = brew::decode_sha256(digest)?;
        let output = Path::new(version).join(&path);
        stage.write_new(&output, &mut entry, length, digest)?;
        files.insert(output.clone(), (length, digest));
        executables.insert(output, mode & 0o111 != 0);
    }
    let expected: BTreeSet<_> = package
        .files
        .keys()
        .chain(package.directories.keys())
        .cloned()
        .collect();
    if seen != expected {
        return Err(Error::InvalidRelease);
    }
    Ok((files, executables))
}

pub(super) fn completion_paths(prefix: &Path) -> [(&'static str, PathBuf); 3] {
    [
        ("bash", prefix.join("etc/bash_completion.d/codex")),
        ("zsh", prefix.join("share/zsh/site-functions/_codex")),
        (
            "fish",
            prefix.join("share/fish/vendor_completions.d/codex.fish"),
        ),
    ]
}

#[cfg(all(
    test,
    any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64")
    )
))]
#[path = "sources_brew_codex_tests.rs"]
mod tests;
