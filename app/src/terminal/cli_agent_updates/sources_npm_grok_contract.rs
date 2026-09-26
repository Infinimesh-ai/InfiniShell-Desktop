//! Grok npm 的固定 wrapper、平台包、TOML 依赖与解压后原生映像合同。

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;

use super::npm_release::{NpmArtifact, VerifiedNpmArchive};
use super::{
    Channel, Error, Installation, MAX_CONFIG, UPDATE_TIMEOUT, absolute_env, plain_ancestors,
    read_limited, stamp, user_home,
};

const MANIFEST: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/grok_1041_npm_manifest.json");
pub(super) const VERSION: &str = "1.0.41";
pub(super) const PACKAGE: &str = "@xai-official/grok";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileSpec {
    pub(super) length: u64,
    pub(super) sha256: String,
    pub(super) executable: bool,
}

impl FileSpec {
    pub(super) fn digest(&self) -> Result<[u8; 32], Error> {
        digest(&self.sha256)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeImage {
    pub(super) length: u64,
    pub(super) sha256: String,
}

impl NativeImage {
    pub(super) fn digest(&self) -> Result<[u8; 32], Error> {
        digest(&self.sha256)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    integrity: String,
    manifest: Value,
    files: BTreeMap<PathBuf, FileSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    schema: u32,
    versions: Vec<String>,
    targets: Vec<String>,
    packages: BTreeMap<String, Package>,
    native: BTreeMap<String, BTreeMap<String, NativeImage>>,
}

pub(super) fn target() -> Result<&'static str, Error> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok("darwin-arm64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok("linux-x64")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Ok("win32-x64")
    } else {
        Err(Error::UnsupportedPlatform)
    }
}

pub(super) fn supports(version: &str) -> Result<(), Error> {
    target()?;
    if version != VERSION {
        return Err(Error::InvalidRelease);
    }
    Ok(())
}

pub(super) fn grok_home() -> Result<PathBuf, Error> {
    let home = if std::env::var_os("GROK_HOME").is_some() {
        absolute_env("GROK_HOME").ok_or(Error::UnsupportedSource)?
    } else {
        user_home()
            .ok_or(Error::UnsupportedSource)?
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?
            .join(".grok")
    };
    plain_ancestors(&home.join("config.toml"))?;
    let canonical = home.canonicalize().map_err(|_| Error::SourceChanged)?;
    if cfg!(unix) && canonical != home {
        return Err(Error::UnsupportedSource);
    }
    Ok(canonical)
}

pub(super) fn verify_installer_config(home: &Path) -> Result<Channel, Error> {
    let bytes = read_limited(&home.join("config.toml"), MAX_CONFIG)?;
    let value: toml::Value = std::str::from_utf8(&bytes)
        .map_err(|_| Error::UnsupportedSource)?
        .parse()
        .map_err(|_| Error::UnsupportedSource)?;
    let cli = value
        .get("cli")
        .and_then(toml::Value::as_table)
        .ok_or(Error::UnsupportedSource)?;
    if cli.get("installer").and_then(toml::Value::as_str) != Some("npm") {
        return Err(Error::UnsupportedSource);
    }
    // 注册来源在升级期间保持原字节；不把用户配置的 registry 作为可执行脚本来源。
    if cli
        .get("npm_registry")
        .is_some_and(|value| value.as_str().is_none())
    {
        return Err(Error::UnsupportedSource);
    }
    match cli.get("channel").and_then(toml::Value::as_str) {
        None | Some("stable") => Ok(Channel::Stable),
        Some("alpha") => Ok(Channel::Alpha),
        Some(_) => Err(Error::ChannelMismatch),
    }
}

/// npm 自带的 canonical bin 也可能排在 PATH 首位；只有包和用户 bin 同版本时才认领。
pub(super) fn registered_mirror(
    installation: &Installation,
    package_root: &Path,
    version: &str,
) -> Result<bool, Error> {
    let Ok(home) = grok_home() else {
        return Ok(false);
    };
    let entry = home.join(if cfg!(windows) {
        "bin/grok.exe"
    } else {
        "bin/grok"
    });
    if if cfg!(windows) {
        installation.stamp.canonical != entry
    } else {
        installation.entry != entry
    } {
        return Ok(false);
    }
    verify_installer_config(&home)?;
    let expected = native(version)?;
    if installation.stamp.digest != expected.digest()? {
        return Ok(false);
    }
    let native = if cfg!(windows) {
        home.join(format!("bin/grok-{version}.exe"))
    } else {
        let target = format!("grok-{version}");
        if std::fs::read_link(&entry).ok().as_deref() != Some(Path::new(&target)) {
            return Ok(false);
        }
        package_root.join("bin/grok-native")
    };
    Ok(stamp(&native)?.digest == expected.digest()?)
}

pub(super) async fn latest(client: &http_client::Client) -> Result<String, Error> {
    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    if let Some(version) = super::npm_grok::live_tests::fixed_release() {
        return Ok(version);
    }
    let url = "https://registry.npmjs.org/@xai-official/grok/latest";
    let response = client
        .get(url)
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
    if value["name"] != PACKAGE {
        return Err(Error::InvalidRelease);
    }
    let version = value["version"].as_str().ok_or(Error::InvalidRelease)?;
    super::parse_version(version)?;
    Ok(version.to_owned())
}

fn contract(version: &str) -> Result<Contract, Error> {
    let value: Contract = serde_json::from_slice(MANIFEST).map_err(|_| Error::InvalidRelease)?;
    if value.schema != 1
        || !matches!(version, "1.0.40" | VERSION)
        || !value.versions.iter().any(|item| item == version)
        || !value
            .targets
            .iter()
            .any(|item| item == target().unwrap_or_default())
    {
        return Err(Error::InvalidRelease);
    }
    Ok(value)
}

pub(super) fn native(version: &str) -> Result<NativeImage, Error> {
    contract(version)?
        .native
        .get(version)
        .and_then(|targets| targets.get(target().ok()?))
        .cloned()
        .ok_or(Error::InvalidRelease)
}

fn packages(version: &str) -> Result<[(String, PathBuf); 3], Error> {
    let platform = format!("{PACKAGE}-{}", target()?);
    Ok([
        (format!("{PACKAGE}@{version}"), PathBuf::new()),
        (
            format!("{platform}@{version}"),
            PathBuf::from("node_modules").join(platform),
        ),
        (
            "@iarna/toml@3.0.0".to_owned(),
            PathBuf::from("node_modules/@iarna/toml"),
        ),
    ])
}

/// Unix postinstall 把唯一公共包入口换成 ./grok-native；Windows 保留原 Node wrapper。
pub(super) fn installed_files(version: &str) -> Result<BTreeMap<PathBuf, FileSpec>, Error> {
    let contract = contract(version)?;
    let mut files = BTreeMap::new();
    for (key, prefix) in packages(version)? {
        for (path, expected) in &contract
            .packages
            .get(&key)
            .ok_or(Error::InvalidRelease)?
            .files
        {
            files.insert(prefix.join(path), expected.clone());
        }
    }
    if !cfg!(windows) {
        files.remove(Path::new("bin/grok"));
        let native = native(version)?;
        files.insert(
            PathBuf::from("bin/grok-native"),
            FileSpec {
                length: native.length,
                sha256: native.sha256,
                executable: true,
            },
        );
    }
    Ok(files)
}

pub(super) fn compressed_entry() -> PathBuf {
    PathBuf::from(if cfg!(windows) {
        "bin/grok.exe.br"
    } else {
        "bin/grok.br"
    })
}

pub(super) struct DownloadedPackage {
    pub(super) prefix: PathBuf,
    pub(super) artifact: NpmArtifact,
    pub(super) archive: NamedTempFile,
    pub(super) verified: VerifiedNpmArchive,
}

pub(super) async fn download_release(root: &Path) -> Result<Vec<DownloadedPackage>, Error> {
    supports(VERSION)?;
    let contract = contract(VERSION)?;
    let mut output = Vec::new();
    for (key, prefix) in packages(VERSION)? {
        let expected = contract.packages.get(&key).ok_or(Error::InvalidRelease)?;
        let name = expected.manifest["name"]
            .as_str()
            .ok_or(Error::InvalidRelease)?;
        let version = expected.manifest["version"]
            .as_str()
            .ok_or(Error::InvalidRelease)?;
        let mut metadata = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
        download(
            &format!("https://registry.npmjs.org/{name}/{version}"),
            metadata.as_file_mut(),
            1024 * 1024,
        )
        .await?;
        let bytes = std::fs::read(metadata.path()).map_err(|_| Error::PersistenceFailed)?;
        let artifact = NpmArtifact::from_pinned_metadata(
            &bytes,
            expected.manifest.clone(),
            &expected.integrity,
        )?;
        let mut archive = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
        download(
            &artifact.tarball_url,
            archive.as_file_mut(),
            512 * 1024 * 1024,
        )
        .await?;
        let verified = artifact.verify_archive(archive.as_file_mut())?;
        if verified.files.len() != expected.files.len() {
            return Err(Error::InvalidRelease);
        }
        for (path, file) in &verified.files {
            let expected = expected.files.get(path).ok_or(Error::InvalidRelease)?;
            if file.length != expected.length
                || file.sha256 != expected.digest()?
                || file.executable != expected.executable
            {
                return Err(Error::InvalidRelease);
            }
        }
        output.push(DownloadedPackage {
            prefix,
            artifact,
            archive,
            verified,
        });
    }
    Ok(output)
}

async fn download(url: &str, output: &mut File, limit: u64) -> Result<(), Error> {
    #[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
    if let Some(result) = super::npm_grok::live_tests::copy_fixed_download(url, output, limit) {
        result?;
        return output.sync_all().map_err(|_| Error::PersistenceFailed);
    }
    let response = http_client::Client::new()
        .get(url)
        .timeout(UPDATE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() || response.url().as_str() != url {
        return Err(Error::Network);
    }
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    let mut length = 0_u64;
    while let Some(bytes) = stream.next().await {
        let bytes = bytes.map_err(|_| Error::Network)?;
        length = length
            .checked_add(bytes.len() as u64)
            .ok_or(Error::InvalidRelease)?;
        if length > limit {
            return Err(Error::InvalidRelease);
        }
        output
            .write_all(&bytes)
            .map_err(|_| Error::PersistenceFailed)?;
    }
    if length == 0 {
        return Err(Error::InvalidRelease);
    }
    output.sync_all().map_err(|_| Error::PersistenceFailed)
}

/// 只对已经验证的固定平台 .br 解压；不启动 Node 或官方 postinstall。
pub(super) fn decompress(input: &mut dyn std::io::Read, output: &mut File) -> Result<(), Error> {
    let expected = native(VERSION)?;
    let mut reader = brotli::Decompressor::new(input, 64 * 1024);
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let size = reader
            .read(&mut buffer)
            .map_err(|_| Error::InvalidRelease)?;
        if size == 0 {
            break;
        }
        length = length
            .checked_add(size as u64)
            .ok_or(Error::InvalidRelease)?;
        if length > expected.length {
            return Err(Error::InvalidRelease);
        }
        digest.update(&buffer[..size]);
        output
            .write_all(&buffer[..size])
            .map_err(|_| Error::PersistenceFailed)?;
    }
    if length != expected.length || <[u8; 32]>::from(digest.finalize()) != expected.digest()? {
        return Err(Error::InvalidRelease);
    }
    output.sync_all().map_err(|_| Error::PersistenceFailed)
}

pub(super) fn digest(value: &str) -> Result<[u8; 32], Error> {
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
