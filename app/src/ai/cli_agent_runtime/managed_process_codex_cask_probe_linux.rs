//! Linuxbrew Codex 的窄探针合同；固定 musl 映像经密封 memfd 执行，不读取用户配置。

use std::io;
use std::path::Path;

use serde_json::Value;
use uuid::Uuid;

use super::{AtomicLaunchKind, Manifest};

const OLD_PACKAGE: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/codex_0155_package_manifest.json");
const PACKAGE: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/codex_0156_package_manifest.json");
const CASKROOM: &str = "/home/linuxbrew/.linuxbrew/Caskroom";

pub(super) fn valid_entry(manifest: &Manifest) -> bool {
    if manifest.atomic_launch_kind != Some(AtomicLaunchKind::CodexHomebrewVersionProbeV1)
        || !manifest.executable.ends_with("bin/codex")
    {
        return false;
    }
    let version = manifest.executable.parent().and_then(Path::parent);
    let package = version.and_then(Path::parent);
    let name = package
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    let version = version
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    let installed = name == Some("codex") && matches!(version, Some("0.155.1" | "0.156.1"));
    let candidate = version == Some("0.156.1")
        && name
            .and_then(|name| name.strip_prefix(".infinishell-brew-"))
            .and_then(|value| Uuid::parse_str(value).ok())
            .is_some_and(|id| !id.is_nil());
    (installed || candidate) && package.and_then(Path::parent) == Some(Path::new(CASKROOM))
}

pub(super) fn validate(manifest: &Manifest) -> io::Result<()> {
    if !valid_entry(manifest) || manifest.expected_files.len() != 1 {
        return Err(invalid());
    }
    let version = manifest
        .executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .ok_or_else(invalid)?;
    let bytes = match version {
        "0.155.1" => OLD_PACKAGE,
        "0.156.1" => PACKAGE,
        _ => return Err(invalid()),
    };
    let release: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let package = &release["packages"]["linux-x64"];
    let native = &package["files"]["bin/codex"];
    let expected = &manifest.expected_files[0];
    if release["version"] != version
        || release["release_tag"] != format!("rust-v{version}")
        || package["target"] != "x86_64-unknown-linux-musl"
        || package["entrypoint"] != "bin/codex"
        || expected.path != manifest.executable
        || expected.canonical_path != manifest.executable
        || native[0].as_u64() != Some(expected.size)
        || native[1].as_str() != Some(expected.sha256.as_str())
    {
        return Err(invalid());
    }
    Ok(())
}

fn invalid() -> io::Error {
    io::Error::other("Linuxbrew Codex 探针与固定 musl 发行合同不匹配")
}
