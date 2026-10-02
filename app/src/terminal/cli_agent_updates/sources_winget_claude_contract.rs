//! 仅绑定官方 Windows x64 Claude portable 286 来源与 Stable 285 目标。
//! 完整官方 YAML 中的 arm64 条目不构成对该平台原生映像的授权。

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::{Error, brew};

pub(super) const FROM: &str = "2.1.286";
pub(super) const TO: &str = "2.1.285";

#[derive(Deserialize)]
struct Release {
    winget_installer_url: String,
    winget_installer_sha256: String,
    winget_installer_bytes: u64,
    expected_complete_manifest: Value,
    native_url: String,
    native_sha256: String,
    native_bytes: u64,
}

fn contract(version: &str) -> Result<Release, Error> {
    let mut contracts: BTreeMap<String, Release> =
        serde_json::from_slice(include_bytes!("sources_winget_claude_contract.json"))
            .map_err(|_| Error::InvalidRelease)?;
    contracts.remove(version).ok_or(Error::InvalidRelease)
}

pub(super) fn native(version: &str) -> Result<(u64, [u8; 32]), Error> {
    let release = contract(version)?;
    Ok((
        release.native_bytes,
        brew::decode_sha256(&release.native_sha256)?,
    ))
}

pub(super) fn metadata_url(version: &str) -> Result<String, Error> {
    if version != TO {
        return Err(Error::InvalidRelease);
    }
    Ok(contract(version)?.winget_installer_url)
}

pub(super) fn manifest_sha256(version: &str) -> Result<[u8; 32], Error> {
    brew::decode_sha256(&contract(version)?.winget_installer_sha256)
}

pub(super) fn release(version: &str, bytes: &[u8]) -> Result<(String, [u8; 32]), Error> {
    if version != TO {
        return Err(Error::InvalidRelease);
    }
    let release = contract(version)?;
    if bytes.len() as u64 != release.winget_installer_bytes
        || <[u8; 32]>::from(Sha256::digest(bytes)) != manifest_sha256(version)?
        || serde_yaml::from_slice::<Value>(bytes).map_err(|_| Error::InvalidRelease)?
            != release.expected_complete_manifest
    {
        return Err(Error::InvalidRelease);
    }
    Ok((
        release.native_url,
        brew::decode_sha256(&release.native_sha256)?,
    ))
}

#[cfg(test)]
#[path = "sources_winget_claude_contract_tests.rs"]
mod tests;
