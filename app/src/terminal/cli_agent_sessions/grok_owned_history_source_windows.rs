//! Windows 官方 npm 公开入口到固定 native 的读取合同；不运行 Node、bootstrap 或 postinstall。

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use crate::ai::cli_agent_runtime::managed_process::ExpectedFileIdentity;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

#[path = "../cli_agent_updates/sources_npm_grok_windows_contract.rs"]
mod contract;
#[path = "grok_owned_windows_files.rs"]
mod files;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceFile {
    path: PathBuf,
    identity: ExpectedFileIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NpmGrokSource {
    pub(crate) executable: PathBuf,
    /// 只标识实际 npm 安装来源；受限托管 profile 不能用它覆盖自己的隔离 GROK_HOME。
    pub(crate) grok_home: PathBuf,
    prefix: PathBuf,
    files: Vec<SourceFile>,
}

impl NpmGrokSource {
    pub(crate) fn capture(entry: &Path) -> io::Result<Option<Self>> {
        // 先核公开入口本身，不能让规范化隐藏链接或重解析点跳转。
        files::plain_path(entry)?;
        let entry = entry.canonicalize()?;
        let name = entry
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if !["grok.exe", "grok.cmd", "grok.ps1"]
            .iter()
            .any(|expected| name.eq_ignore_ascii_case(expected))
        {
            return Ok(None);
        }
        if name.eq_ignore_ascii_case("grok.exe") {
            let observed = hash(&entry)?;
            if observed.0 == 154082120
                && observed.1 == "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25"
            {
                return Ok(None);
            }
        }
        let home = if let Some(home) = std::env::var_os("GROK_HOME") {
            PathBuf::from(home)
        } else {
            PathBuf::from(std::env::var_os("USERPROFILE").ok_or_else(invalid)?)
                .canonicalize()?
                .join(".grok")
        };
        if !home.is_absolute() {
            return Err(invalid());
        }
        files::plain_path(&home)?;
        let home = home.canonicalize()?;
        let native = home.join("bin/grok.exe");
        files::plain_path(&native)?;
        let native = native.canonicalize()?;
        if name.eq_ignore_ascii_case("grok.exe") {
            let digest = hash(&entry)?;
            if entry != native || digest != contract::native(contract::VERSION)? {
                return Err(invalid());
            }
        }
        validate_config(&home)?;
        let mut prefixes = BTreeSet::new();
        if !name.eq_ignore_ascii_case("grok.exe") {
            prefixes.insert(entry.parent().ok_or_else(invalid)?.to_owned());
        } else {
            for prefix in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .filter(|path| path.is_absolute())
            {
                if files::plain_path(&prefix).is_err() {
                    continue;
                }
                if let Ok(prefix) = prefix.canonicalize() {
                    if prefix.join("grok.cmd").is_file() {
                        prefixes.insert(prefix);
                    }
                }
            }
        }
        let mut matches = Vec::new();
        for prefix in prefixes {
            if let Ok(value) = Self::capture_prefix(&prefix, &home, &native) {
                matches.push(value);
            }
        }
        if matches.len() != 1 {
            return Err(invalid());
        }
        Ok(matches.pop())
    }

    fn capture_prefix(prefix: &Path, home: &Path, native: &Path) -> io::Result<Self> {
        let required = required_files(prefix, home)?;
        let mut recorded = Vec::new();
        let mut held = Vec::new();
        for (path, expected) in required {
            held.extend(parent_handles(&path)?);
            let file = open(&path)?;
            if hash_opened(&file)? != expected {
                return Err(invalid());
            }
            recorded.push(SourceFile {
                path: path.clone(),
                identity: ExpectedFileIdentity::capture(&path)?,
            });
            held.push(file);
        }
        let value = Self {
            executable: native.to_owned(),
            grok_home: home.to_owned(),
            prefix: prefix.to_owned(),
            files: recorded,
        };
        value.validate_and_hold()?;
        Ok(value)
    }

    /// 真正派生时再次验证且持句柄；调用方保留到原生进程已进入可信监督范围。
    pub(crate) fn validate_and_hold(&self) -> io::Result<Vec<File>> {
        if self.executable != self.grok_home.join("bin/grok.exe") || !self.prefix.is_absolute() {
            return Err(invalid());
        }
        validate_config(&self.grok_home)?;
        let mut required = required_files(&self.prefix, &self.grok_home)?;
        let mut held = Vec::new();
        for proof in &self.files {
            let expected = required.remove(&proof.path).ok_or_else(invalid)?;
            held.extend(parent_handles(&proof.path)?);
            let file = open(&proof.path)?;
            if hash_opened(&file)? != expected
                || ExpectedFileIdentity::capture(&proof.path)? != proof.identity
            {
                return Err(invalid());
            }
            held.push(file);
        }
        if !required.is_empty() {
            return Err(invalid());
        }
        Ok(held)
    }
}

fn required_files(prefix: &Path, home: &Path) -> io::Result<BTreeMap<PathBuf, (u64, String)>> {
    let package = prefix.join("node_modules/@xai-official/grok");
    let mut result: BTreeMap<_, _> = contract::files(contract::VERSION)?
        .into_iter()
        .map(|(path, (size, digest, _))| (package.join(path), (size, digest)))
        .collect();
    for (name, content) in contract::shims() {
        result.insert(
            prefix.join(name),
            (
                content.len() as u64,
                format!("{:x}", Sha256::digest(content.as_bytes())),
            ),
        );
    }
    let native = contract::native(contract::VERSION)?;
    result.insert(home.join("bin/grok.exe"), native.clone());
    result.insert(
        home.join(format!("bin/grok-{}.exe", contract::VERSION)),
        native,
    );
    Ok(result)
}
fn validate_config(home: &Path) -> io::Result<()> {
    let path = home.join("config.toml");
    files::plain_path(&path)?;
    let mut input = fs::File::open(&path)?;
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid());
    }
    let value: toml::Value = std::str::from_utf8(&bytes)
        .map_err(|_| invalid())?
        .parse()
        .map_err(|_| invalid())?;
    if value
        .get("cli")
        .and_then(|cli| cli.get("installer"))
        .and_then(toml::Value::as_str)
        != Some("npm")
    {
        return Err(invalid());
    }
    Ok(())
}
fn parent_handles(path: &Path) -> io::Result<Vec<File>> {
    files::plain_path(path)?;
    path.parent()
        .ok_or_else(invalid)?
        .ancestors()
        .map(|parent| {
            let file = OpenOptions::new()
                .read(true)
                .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
                .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
                .open(parent)?;
            files::file_identity(&file)?;
            if !file.metadata()?.is_dir() {
                return Err(invalid());
            }
            Ok(file)
        })
        .collect()
}
fn open(path: &Path) -> io::Result<File> {
    files::plain_path(path)?;
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)?;
    files::file_identity(&file)?;
    if !file.metadata()?.is_file() {
        return Err(invalid());
    }
    Ok(file)
}
fn hash(path: &Path) -> io::Result<(u64, String)> {
    hash_opened(&open(path)?)
}
fn hash_opened(file: &File) -> io::Result<(u64, String)> {
    let mut file = file.try_clone()?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > 512 * 1024 * 1024 {
            return Err(invalid());
        }
        hash.update(&buffer[..count]);
    }
    if bytes != file.metadata()?.len() {
        return Err(invalid());
    }
    Ok((bytes, format!("{:x}", hash.finalize())))
}
fn invalid() -> io::Error {
    io::Error::other("Grok Windows npm 公开入口或固定资源身份不匹配")
}
