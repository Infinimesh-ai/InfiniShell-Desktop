//! 官方 npm 单包树的版本化读取合同；不运行 npm、Node 或包生命周期脚本。
//!
//! 此模块只证明下载内容与已登记布局相符。入口归属、忙碌延期、发布与恢复由事务层负责。

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use flate2::read::GzDecoder;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256, Sha512};
use tar::Archive;

use super::{CLIAgent, Error};

pub(super) const LAYOUT_VERSION: u32 = 1;
const MAX_METADATA: usize = 1024 * 1024;
const MAX_COMPRESSED: u64 = 512 * 1024 * 1024;
const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;
const MAX_FILES: u64 = 256;
const MAX_PATH: usize = 240;

const CODEX_TARGETS: [&str; 6] = [
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64",
    "linux-x64",
    "win32-arm64",
    "win32-x64",
];
const CLAUDE_TARGETS: [&str; 8] = [
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64",
    "linux-x64",
    "linux-arm64-musl",
    "linux-x64-musl",
    "win32-arm64",
    "win32-x64",
];
const CLAUDE_PREPARE: &str = "node -e \"if (!process.env.AUTHORIZED) { console.error('ERROR: Direct publishing is not allowed.\\nPlease see the release workflow documentation to publish this package.'); process.exit(1); }\"";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Codex01561,
    Claude21280,
}

/// 两个 tarball 各自有独立 SRI；平台目录名不等同于平台包清单中的 name。
#[derive(Debug)]
pub(super) struct NpmRelease {
    pub(super) wrapper: NpmArtifact,
    pub(super) platform: NpmArtifact,
    pub(super) dependency_directory: PathBuf,
    pub(super) native_entry: PathBuf,
    pub(super) public_entry: PathBuf,
    pub(super) materialize_native_entry: bool,
}

#[derive(Debug)]
pub(super) struct NpmArtifact {
    pub(super) tarball_url: String,
    pub(super) metadata_sha256: [u8; 32],
    integrity: [u8; 64],
    manifest_contract: Value,
    unpacked_size: u64,
    file_count: u64,
}

/// 只包含已完整读取并校验的普通文件；事务层不能据此放宽目标目录的本地身份检查。
#[derive(Debug)]
pub(super) struct VerifiedNpmArchive {
    pub(super) files: BTreeMap<PathBuf, NpmArchiveFile>,
    pub(super) compressed_sha256: [u8; 32],
}

#[derive(Debug)]
pub(super) struct NpmArchiveFile {
    pub(super) length: u64,
    pub(super) sha256: [u8; 32],
    pub(super) executable: bool,
}

struct ArchiveRead<'a, R> {
    reader: &'a mut R,
    digest: Sha512,
    bytes: u64,
}

impl<R: Read> Read for ArchiveRead<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let length = self.reader.read(buffer)?;
        self.bytes += length as u64;
        if self.bytes > MAX_COMPRESSED {
            return Err(std::io::Error::other("npm_archive_compressed_limit"));
        }
        self.digest.update(&buffer[..length]);
        Ok(length)
    }
}

impl NpmRelease {
    pub(super) fn from_metadata(
        agent: CLIAgent,
        version: &str,
        target: &str,
        wrapper_metadata: &[u8],
        platform_metadata: &[u8],
    ) -> Result<Self, Error> {
        // 消费者渠道仍由外层选择；布局合同不会把本次验收外推到未知版本。
        let layout = if agent == CLIAgent::Codex && version == "0.156.1" {
            Layout::Codex01561
        } else if agent == CLIAgent::Claude && version == "2.1.280" {
            Layout::Claude21280
        } else {
            return Err(Error::InvalidRelease);
        };
        let (package, command, entry, targets) = match layout {
            Layout::Codex01561 => (
                "@openai/codex",
                "codex",
                "bin/codex.js",
                CODEX_TARGETS.as_slice(),
            ),
            Layout::Claude21280 => (
                "@anthropic-ai/claude-code",
                "claude",
                "bin/claude.exe",
                CLAUDE_TARGETS.as_slice(),
            ),
        };
        if !targets.contains(&target) {
            return Err(Error::UnsupportedPlatform);
        }
        let optional: BTreeMap<_, _> = targets
            .iter()
            .map(|target| {
                let requirement = match layout {
                    Layout::Codex01561 => format!("npm:{package}@{version}-{target}"),
                    Layout::Claude21280 => version.to_owned(),
                };
                (format!("{package}-{target}"), requirement)
            })
            .collect();
        let scripts = match layout {
            Layout::Codex01561 => json!({}),
            Layout::Claude21280 => {
                json!({"prepare":CLAUDE_PREPARE,"postinstall":"node install.cjs"})
            }
        };
        let wrapper_contract = json!({
            "name":package,"version":version,"bin":{command:entry},
            "dependencies":{},"optionalDependencies":optional,"scripts":scripts,
            "os":null,"cpu":null,"libc":null,
        });
        let platform_package = format!("{package}-{target}");
        let (platform_name, platform_version) = match layout {
            Layout::Codex01561 => (package.to_owned(), format!("{version}-{target}")),
            Layout::Claude21280 => (platform_package.clone(), version.to_owned()),
        };
        let mut components = target.split('-');
        let os = components.next().ok_or(Error::InvalidRelease)?;
        let cpu = components.next().ok_or(Error::InvalidRelease)?;
        let libc = if layout == Layout::Claude21280 && os == "linux" {
            json!([if target.ends_with("-musl") {
                "musl"
            } else {
                "glibc"
            }])
        } else {
            Value::Null
        };
        let platform_contract = json!({
            "name":platform_name,"version":platform_version,"bin":{},
            "dependencies":{},"optionalDependencies":{},"scripts":{},
            "os":[os],"cpu":[cpu],"libc":libc,
        });
        let wrapper = NpmArtifact::from_metadata(wrapper_metadata, wrapper_contract)?;
        let platform = NpmArtifact::from_metadata(platform_metadata, platform_contract)?;
        let native_entry = match layout {
            Layout::Codex01561 => {
                let triple = match target {
                    "darwin-arm64" => "aarch64-apple-darwin",
                    "darwin-x64" => "x86_64-apple-darwin",
                    "linux-arm64" => "aarch64-unknown-linux-musl",
                    "linux-x64" => "x86_64-unknown-linux-musl",
                    "win32-arm64" => "aarch64-pc-windows-msvc",
                    "win32-x64" => "x86_64-pc-windows-msvc",
                    // 前面的精确白名单拒绝其他目标，不能猜测原生包路径。
                    _ => return Err(Error::UnsupportedPlatform),
                };
                let binary = if os == "win32" { "codex.exe" } else { "codex" };
                PathBuf::from(format!("vendor/{triple}/bin/{binary}"))
            }
            Layout::Claude21280 => PathBuf::from(if os == "win32" {
                "claude.exe"
            } else {
                "claude"
            }),
        };
        Ok(Self {
            wrapper,
            platform,
            dependency_directory: PathBuf::from("node_modules").join(platform_package),
            native_entry,
            public_entry: PathBuf::from(entry),
            // Claude 的官方 postinstall 仅由宿主已审查的文件复制替代，永远不执行 install.cjs。
            materialize_native_entry: layout == Layout::Claude21280,
        })
    }

    pub(super) fn verify_entries(
        &self,
        wrapper: &VerifiedNpmArchive,
        platform: &VerifiedNpmArchive,
    ) -> Result<(), Error> {
        if !wrapper.files.contains_key(&self.public_entry)
            || !platform
                .files
                .get(&self.native_entry)
                .is_some_and(|file| file.executable && file.length > 0)
        {
            return Err(Error::InvalidRelease);
        }
        Ok(())
    }
}

fn contract(manifest: &Value) -> Result<Value, Error> {
    if !manifest.is_object() {
        return Err(Error::InvalidRelease);
    }
    // 未登记的运行时依赖不能因为 npm 的可选安装行为而被静默遗漏。
    for key in [
        "peerDependencies",
        "bundledDependencies",
        "bundleDependencies",
    ] {
        if manifest
            .get(key)
            .is_some_and(|value| !value.is_null() && value != &json!({}) && value != &json!([]))
        {
            return Err(Error::InvalidRelease);
        }
    }
    let mut result = json!({});
    for key in [
        "name",
        "version",
        "bin",
        "dependencies",
        "optionalDependencies",
        "scripts",
        "os",
        "cpu",
        "libc",
    ] {
        result[key] = manifest.get(key).cloned().unwrap_or_else(|| {
            if matches!(
                key,
                "bin" | "dependencies" | "optionalDependencies" | "scripts"
            ) {
                json!({})
            } else {
                Value::Null
            }
        });
    }
    Ok(result)
}

impl NpmArtifact {
    fn from_metadata(bytes: &[u8], expected: Value) -> Result<Self, Error> {
        if bytes.len() > MAX_METADATA {
            return Err(Error::InvalidRelease);
        }
        let manifest: Value = serde_json::from_slice(bytes).map_err(|_| Error::InvalidRelease)?;
        if contract(&manifest)? != expected {
            return Err(Error::InvalidRelease);
        }
        let name = expected["name"].as_str().ok_or(Error::InvalidRelease)?;
        let version = expected["version"].as_str().ok_or(Error::InvalidRelease)?;
        let leaf = name.rsplit('/').next().ok_or(Error::InvalidRelease)?;
        let tarball_url = format!("https://registry.npmjs.org/{name}/-/{leaf}-{version}.tgz");
        if manifest["dist"]["tarball"] != tarball_url {
            return Err(Error::InvalidRelease);
        }
        let encoded = manifest["dist"]["integrity"]
            .as_str()
            .and_then(|value| value.strip_prefix("sha512-"))
            .ok_or(Error::InvalidRelease)?;
        let decoded = STANDARD
            .decode(encoded)
            .map_err(|_| Error::InvalidRelease)?;
        if STANDARD.encode(&decoded) != encoded {
            return Err(Error::InvalidRelease);
        }
        let integrity = decoded.try_into().map_err(|_| Error::InvalidRelease)?;
        let unpacked_size = manifest["dist"]["unpackedSize"]
            .as_u64()
            .ok_or(Error::InvalidRelease)?;
        let file_count = manifest["dist"]["fileCount"]
            .as_u64()
            .ok_or(Error::InvalidRelease)?;
        if !(1..=MAX_UNPACKED).contains(&unpacked_size) || !(1..=MAX_FILES).contains(&file_count) {
            return Err(Error::InvalidRelease);
        }
        Ok(Self {
            tarball_url,
            metadata_sha256: Sha256::digest(bytes).into(),
            integrity,
            manifest_contract: expected,
            unpacked_size,
            file_count,
        })
    }

    /// 先校验完整归档，再逐成员交给锚定目录描述符的写入方；不调用 tar::unpack。
    pub(super) fn extract_verified(
        &self,
        reader: &mut (impl Read + Seek),
        mut write: impl FnMut(&std::path::Path, &NpmArchiveFile, &mut dyn Read) -> Result<(), Error>,
    ) -> Result<VerifiedNpmArchive, Error> {
        let verified = self.verify_archive(reader)?;
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|_| Error::InvalidRelease)?;
        let mut archive =
            Archive::new(GzDecoder::new(reader).take(MAX_UNPACKED + 1024 * MAX_FILES));
        let mut seen = BTreeMap::new();
        for entry in archive
            .entries()
            .map_err(|_| Error::InvalidRelease)?
            .raw(true)
        {
            let mut entry = entry.map_err(|_| Error::InvalidRelease)?;
            if !entry.header().entry_type().is_file() {
                return Err(Error::InvalidRelease);
            }
            let path = safe_archive_path(entry.path_bytes().as_ref())?;
            let expected = verified.files.get(&path).ok_or(Error::InvalidRelease)?;
            if seen.insert(path.clone(), ()).is_some() || entry.size() != expected.length {
                return Err(Error::InvalidRelease);
            }
            write(&path, expected, &mut entry)?;
            let mut tail = [0];
            if entry.read(&mut tail).map_err(|_| Error::InvalidRelease)? != 0 {
                return Err(Error::InvalidRelease);
            }
        }
        if seen.len() != verified.files.len() {
            return Err(Error::InvalidRelease);
        }
        Ok(verified)
    }

    /// 先完整核对压缩字节 SRI，再只读扫描归档；此入口不创建任何目录或文件。
    pub(super) fn verify_archive(
        &self,
        reader: &mut (impl Read + Seek),
    ) -> Result<VerifiedNpmArchive, Error> {
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|_| Error::InvalidRelease)?;
        let mut sha512 = Sha512::new();
        let mut sha256 = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let mut compressed_size = 0_u64;
        loop {
            let size = reader
                .read(&mut buffer)
                .map_err(|_| Error::InvalidRelease)?;
            if size == 0 {
                break;
            }
            compressed_size += size as u64;
            if compressed_size > MAX_COMPRESSED {
                return Err(Error::InvalidRelease);
            }
            sha512.update(&buffer[..size]);
            sha256.update(&buffer[..size]);
        }
        if <[u8; 64]>::from(sha512.finalize()) != self.integrity {
            return Err(Error::InvalidRelease);
        }
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|_| Error::InvalidRelease)?;
        // tar 头/填充也计入展开上限，避免大量零块或扩展头构造压缩炸弹。
        let tar_limit = self.unpacked_size + (self.file_count + 4) * 1024;
        let checked_reader = ArchiveRead {
            reader,
            digest: Sha512::new(),
            bytes: 0,
        };
        let mut archive = Archive::new(GzDecoder::new(checked_reader).take(tar_limit + 1));
        let mut files = BTreeMap::new();
        let mut aliases = BTreeMap::new();
        let mut directories = BTreeMap::new();
        let mut total = 0_u64;
        let mut manifest_seen = false;
        for entry in archive
            .entries()
            .map_err(|_| Error::InvalidRelease)?
            .raw(true)
        {
            let mut entry = entry.map_err(|_| Error::InvalidRelease)?;
            // 固定版本官方包只有普通文件；拒绝链接、设备、FIFO、稀疏与 PAX/GNU 改名头。
            if !entry.header().entry_type().is_file() {
                return Err(Error::InvalidRelease);
            }
            let path = safe_archive_path(entry.path_bytes().as_ref())?;
            let alias = path.to_string_lossy().to_ascii_lowercase();
            if aliases.insert(alias, ()).is_some() || files.len() as u64 >= self.file_count {
                return Err(Error::InvalidRelease);
            }
            for parent in path
                .ancestors()
                .skip(1)
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                let spelling = parent.to_string_lossy().into_owned();
                if directories
                    .insert(spelling.to_ascii_lowercase(), spelling.clone())
                    .is_some_and(|previous| previous != spelling)
                {
                    return Err(Error::InvalidRelease);
                }
            }
            let length = entry.size();
            total = total.checked_add(length).ok_or(Error::InvalidRelease)?;
            if total > self.unpacked_size {
                return Err(Error::InvalidRelease);
            }
            let mode = entry.header().mode().map_err(|_| Error::InvalidRelease)?;
            if mode & !0o777 != 0 {
                return Err(Error::InvalidRelease);
            }
            let mut digest = Sha256::new();
            let mut package_json = Vec::new();
            let mut actual_length = 0_u64;
            loop {
                let size = entry.read(&mut buffer).map_err(|_| Error::InvalidRelease)?;
                if size == 0 {
                    break;
                }
                actual_length += size as u64;
                digest.update(&buffer[..size]);
                if path == PathBuf::from("package.json") {
                    if package_json.len() + size > MAX_METADATA {
                        return Err(Error::InvalidRelease);
                    }
                    package_json.extend_from_slice(&buffer[..size]);
                }
            }
            if actual_length != length {
                return Err(Error::InvalidRelease);
            }
            if path == PathBuf::from("package.json") {
                let manifest: Value =
                    serde_json::from_slice(&package_json).map_err(|_| Error::InvalidRelease)?;
                if contract(&manifest)? != self.manifest_contract {
                    return Err(Error::InvalidRelease);
                }
                manifest_seen = true;
            }
            files.insert(
                path,
                NpmArchiveFile {
                    length,
                    sha256: digest.finalize().into(),
                    executable: mode & 0o111 != 0,
                },
            );
        }
        let mut decoded = archive.into_inner();
        // 必须读到 gzip 校验尾部；tar 的结束块后只能剩下有界零填充。
        loop {
            let size = decoded
                .read(&mut buffer)
                .map_err(|_| Error::InvalidRelease)?;
            if size == 0 {
                break;
            }
            if buffer[..size].iter().any(|byte| *byte != 0) {
                return Err(Error::InvalidRelease);
            }
        }
        if decoded.limit() == 0
            || !manifest_seen
            || total != self.unpacked_size
            || files.len() as u64 != self.file_count
        {
            return Err(Error::InvalidRelease);
        }
        // 第二遍实际解析到的压缩字节也必须匹配，拒绝验证与解析之间替换了内容的读取器。
        let mut checked_reader = decoded.into_inner().into_inner();
        while checked_reader
            .read(&mut buffer)
            .map_err(|_| Error::InvalidRelease)?
            != 0
        {}
        if checked_reader.bytes != compressed_size
            || <[u8; 64]>::from(checked_reader.digest.finalize()) != self.integrity
        {
            return Err(Error::InvalidRelease);
        }
        // 文件路径不能兼作另一个成员的目录，即使大小写在当前 OS 上不同。
        for path in files.keys() {
            for parent in path.ancestors().skip(1) {
                if aliases.contains_key(&parent.to_string_lossy().to_ascii_lowercase()) {
                    return Err(Error::InvalidRelease);
                }
            }
        }
        Ok(VerifiedNpmArchive {
            files,
            compressed_sha256: sha256.finalize().into(),
        })
    }
}

fn safe_archive_path(bytes: &[u8]) -> Result<PathBuf, Error> {
    let value = std::str::from_utf8(bytes).map_err(|_| Error::InvalidRelease)?;
    let value = value
        .strip_prefix("package/")
        .ok_or(Error::InvalidRelease)?;
    if value.len() > MAX_PATH || value.is_empty() || !value.is_ascii() {
        return Err(Error::InvalidRelease);
    }
    for part in value.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.bytes().any(|byte| {
                byte.is_ascii_control()
                    || matches!(
                        byte,
                        b'\\' | b':' | b'<' | b'>' | b'"' | b'|' | b'?' | b'*' | b'~'
                    )
            })
            || matches!(
                stem.as_str(),
                "con"
                    | "prn"
                    | "aux"
                    | "nul"
                    | "conin$"
                    | "conout$"
                    | "clock$"
                    | "com1"
                    | "com2"
                    | "com3"
                    | "com4"
                    | "com5"
                    | "com6"
                    | "com7"
                    | "com8"
                    | "com9"
                    | "lpt1"
                    | "lpt2"
                    | "lpt3"
                    | "lpt4"
                    | "lpt5"
                    | "lpt6"
                    | "lpt7"
                    | "lpt8"
                    | "lpt9"
            )
        {
            return Err(Error::InvalidRelease);
        }
    }
    Ok(PathBuf::from(value))
}

#[cfg(test)]
#[path = "sources_npm_release_tests.rs"]
mod tests;
