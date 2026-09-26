//! Codex 0.156.1 npm 的公共 Node launcher、平台别名包和完整资源闭包。

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::managed_process::ExpectedFileIdentity;
use super::npm_release::{NpmRelease, VerifiedNpmArchive};
use super::{Error, Stamp, managed_process, stamp};

const PACKAGE_MANIFEST: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/codex_0156_package_manifest.json");
const WRAPPER_INTEGRITY: &str = "sha512-nI1iVl/n2SO2lSvlwEsJx63zdSI4C4Me2gR7AG0OWMJiGSakz2tY2hx43E39Zq5aEoeB5bZjJXzp5Sqhog6vyA==";
const WRAPPER_FILES: [(&str, u64, &str, bool); 3] = [
    (
        "bin/codex.js",
        8790,
        "61b0194f3bb6534439c8d26a3ed57d0805f84b884588b761795323eeb92fcf70",
        true,
    ),
    (
        "package.json",
        1082,
        "c3f16464dca0fe1269b17d02fe0997d1ca3a241c3a61da3d89ec13def0a66c6e",
        false,
    ),
    (
        "README.md",
        3334,
        "ba4e1f69ff48386e72a9c5e1edaf76aad64a475c2d51af79ccba6d1128261ba7",
        false,
    ),
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProbeClosure {
    pub(super) arguments: Vec<OsString>,
    pub(super) expected_files: Vec<ExpectedFileIdentity>,
}

fn target() -> Result<(&'static str, &'static str, &'static str), Error> {
    // 其他 Unix 架构尚无逐文件固定清单，不能从同版本另一架构推导闭包。
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok(("darwin-arm64", "macos-arm64", "aarch64-apple-darwin"))
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok(("linux-x64", "linux-x64", "x86_64-unknown-linux-musl"))
    } else {
        Err(Error::UnsupportedPlatform)
    }
}

pub(super) fn supports(version: &str) -> Result<(), Error> {
    if version != "0.156.1" {
        return Err(Error::InvalidRelease);
    }
    target().map(|_| ())
}

pub(super) fn verify_metadata(wrapper: &[u8], platform: &[u8]) -> Result<(), Error> {
    let (platform_name, _, _) = target()?;
    let wrapper: Value = serde_json::from_slice(wrapper).map_err(|_| Error::InvalidRelease)?;
    let platform: Value = serde_json::from_slice(platform).map_err(|_| Error::InvalidRelease)?;
    let platform_integrity = match platform_name {
        "darwin-arm64" => {
            "sha512-Jg6wbdV+wmMZczhwE74GSxOYEZlViKXn6KyCw/yfrz3PAKFD14xljuPopmdhWC1+8IKU2WdN5fdmXNPt2q4HPA=="
        }
        "linux-x64" => {
            "sha512-2ePo0wgOcnONKsuzp8vBjOmNY+IdsKaouaDDIdiKq9HOWNuV/GI22OeXft2A/1GaoL71ictO0/pLAsCEnQ6wew=="
        }
        _ => return Err(Error::UnsupportedPlatform),
    };
    if wrapper["dist"]["integrity"] != WRAPPER_INTEGRITY
        || platform["dist"]["integrity"] != platform_integrity
    {
        return Err(Error::InvalidRelease);
    }
    Ok(())
}

pub(super) fn verify_archives(
    release: &NpmRelease,
    wrapper: &VerifiedNpmArchive,
    platform: &VerifiedNpmArchive,
) -> Result<(), Error> {
    let (target, package, triple) = target()?;
    if release.public_entry != Path::new("bin/codex.js")
        || release.dependency_directory
            != PathBuf::from(format!("node_modules/@openai/codex-{target}"))
        || release.native_entry != PathBuf::from(format!("vendor/{triple}/bin/codex"))
        || release.materialize_native_entry
        || wrapper.files.len() != WRAPPER_FILES.len()
    {
        return Err(Error::InvalidRelease);
    }
    for (path, size, digest, executable) in WRAPPER_FILES {
        let file = wrapper
            .files
            .get(Path::new(path))
            .ok_or(Error::InvalidRelease)?;
        if file.length != size
            || file.sha256 != digest_bytes(digest)?
            || file.executable != executable
        {
            return Err(Error::InvalidRelease);
        }
    }
    let metadata: Value =
        serde_json::from_slice(PACKAGE_MANIFEST).map_err(|_| Error::InvalidRelease)?;
    let expected = &metadata["packages"][package];
    let files: BTreeMap<PathBuf, (u64, String, u32)> =
        serde_json::from_value(expected["files"].clone()).map_err(|_| Error::InvalidRelease)?;
    if metadata["version"] != "0.156.1"
        || expected["target"] != triple
        || platform.files.len() != files.len() + 2
    {
        return Err(Error::InvalidRelease);
    }
    // npm 只在已经封存的完整发行树外增加 package.json 和 README；两者仍受固定 SRI 绑定。
    for name in ["package.json", "README.md"] {
        let entry = platform
            .files
            .get(Path::new(name))
            .ok_or(Error::InvalidRelease)?;
        if entry.executable || entry.length == 0 || entry.length > 64 * 1024 {
            return Err(Error::InvalidRelease);
        }
    }
    for (path, (length, digest, mode)) in files {
        let path = PathBuf::from("vendor").join(triple).join(path);
        let file = platform.files.get(&path).ok_or(Error::InvalidRelease)?;
        if file.length != length
            || file.sha256 != digest_bytes(&digest)?
            || file.executable != (mode & 0o111 != 0)
        {
            return Err(Error::InvalidRelease);
        }
    }
    Ok(())
}

fn digest_bytes(value: &str) -> Result<[u8; 32], Error> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::InvalidRelease);
    }
    let mut result = [0; 32];
    for (index, slot) in result.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| Error::InvalidRelease)?;
    }
    Ok(result)
}

pub(super) fn capture_probe(
    node: &Stamp,
    stage: &Path,
    files: &BTreeMap<PathBuf, (u64, [u8; 32])>,
) -> Result<ProbeClosure, Error> {
    if stamp(&node.canonical)? != *node
        || node.canonical.file_name().is_none_or(|name| name != "node")
    {
        return Err(Error::SourceChanged);
    }
    let node_identity =
        ExpectedFileIdentity::capture(&node.canonical).map_err(|_| Error::SourceChanged)?;
    let mut expected_files = managed_process::capture_codex_npm_node_closure(&node_identity)
        .map_err(|_| Error::UnsupportedSource)?;
    for (path, (length, digest)) in files {
        expected_files.push(
            ExpectedFileIdentity::capture_release_image(&stage.join(path), *length, *digest)
                .map_err(|_| Error::SourceChanged)?,
        );
    }
    Ok(ProbeClosure {
        arguments: vec![
            stage.join("bin/codex.js").into_os_string(),
            "--version".into(),
        ],
        expected_files,
    })
}
