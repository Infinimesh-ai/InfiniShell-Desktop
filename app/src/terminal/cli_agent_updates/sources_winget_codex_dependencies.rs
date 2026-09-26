//! Codex WinGet 的既有 ripgrep 与 VC Runtime 来源绑定；不负责安装、替换或提权。

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

use super::managed_process::ExpectedFileIdentity;
use super::winget::UNINSTALL;
use super::winget_codex_contract as contract;
use super::winget_codex_index as index;
use super::winget_codex_tree as tree;
use super::{Error, absolute_env};

pub(super) type Values = BTreeMap<String, (u32, Vec<u8>)>;

pub(super) fn values(key: &RegKey) -> Result<Values, Error> {
    if key.enum_keys().next().is_some() {
        return Err(Error::UnsupportedSource);
    }
    let mut result = BTreeMap::new();
    for item in key.enum_values() {
        let (name, value) = item.map_err(|_| Error::SourceChanged)?;
        if result.len() >= 64 || name.len() > 256 || value.bytes.len() > 16384 {
            return Err(Error::UnsupportedSource);
        }
        result.insert(name, (value.vtype as u32, value.bytes));
    }
    Ok(result)
}

pub(super) fn registry(product: &str) -> Result<RegKey, Error> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            format!(r"{UNINSTALL}\{product}"),
            KEY_READ | KEY_WOW64_64KEY,
        )
        .map_err(|_| Error::SourceChanged)
}

pub(super) fn text(key: &RegKey, name: &str) -> Result<String, Error> {
    key.get_value::<String, _>(name)
        .ok()
        .filter(|value| !value.contains('\0') && value.len() < 32768)
        .ok_or(Error::SourceChanged)
}

/// 仅统一 Windows 绝对 DOS 路径的大小写和内核前缀，不解析任意相对路径。
pub(super) fn path_key(path: &Path) -> Result<String, Error> {
    let text = path.to_str().ok_or(Error::SourceChanged)?;
    let text = text
        .strip_prefix(r"\\?\")
        .unwrap_or(text)
        .replace('/', "\\");
    if text.as_bytes().get(1) != Some(&b':')
        || text.as_bytes().get(2) != Some(&b'\\')
        || text[2..].contains(':')
        || text.contains('\0')
        || text.split('\\').any(|part| matches!(part, "." | ".."))
    {
        return Err(Error::SourceChanged);
    }
    Ok(text.trim_end_matches('\\').to_ascii_lowercase())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Dependencies {
    rg_root: PathBuf,
    rg_registration: Values,
    rg_tree: tree::Snapshot,
    rg_link: tree::Link,
    pub(super) rg: ExpectedFileIdentity,
    vc_registration: Values,
    pub(super) runtime: Vec<ExpectedFileIdentity>,
}

impl Dependencies {
    pub(super) fn capture() -> Result<Self, Error> {
        let prefix = absolute_env("LOCALAPPDATA")
            .ok_or(Error::UnsupportedSource)?
            .join("Microsoft/WinGet");
        let product = format!("BurntSushi.ripgrep.MSVC_{}", contract::SOURCE);
        let key = registry(&product)?;
        let rg_root = PathBuf::from(text(&key, "InstallLocation")?);
        if text(&key, "WinGetPackageIdentifier")? != "BurntSushi.ripgrep.MSVC"
            || text(&key, "WinGetSourceIdentifier")? != contract::SOURCE
            || text(&key, "WinGetInstallerType")? != "portable"
            || path_key(&rg_root)? != path_key(&prefix.join("Packages").join(&product))?
        {
            return Err(Error::UnsupportedSource);
        }
        super::parse_version(&text(&key, "DisplayVersion")?)?;
        let rg_link = tree::Link::capture(&prefix.join("Links/rg.exe"))?;
        let target = rg_link
            .target
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?;
        let canonical_root = rg_root.canonicalize().map_err(|_| Error::SourceChanged)?;
        if !target.starts_with(&canonical_root)
            || target.file_name().is_none_or(|name| name != "rg.exe")
        {
            return Err(Error::SourceChanged);
        }
        let entries = index::read(&rg_root.join(format!("{product}.db")))?;
        let mut linked = false;
        let mut owned = false;
        for entry in entries {
            match entry.kind {
                1 => {
                    if path_key(&entry.path)? == path_key(&target)? {
                        let digest = super::stamp(&target)?.digest;
                        if entry.sha
                            != digest
                                .iter()
                                .map(|byte| format!("{byte:02x}"))
                                .collect::<String>()
                        {
                            return Err(Error::SourceChanged);
                        }
                        owned = true;
                    }
                }
                2 => {
                    let directory = entry
                        .path
                        .canonicalize()
                        .map_err(|_| Error::SourceChanged)?;
                    if directory.starts_with(&canonical_root) && target.starts_with(directory) {
                        owned = true;
                    }
                }
                3 => {
                    if path_key(&entry.path)? == path_key(&rg_link.path)?
                        && path_key(&entry.target)? == path_key(&target)?
                    {
                        linked = true;
                    }
                }
                4 => {}
                _ => return Err(Error::UnsupportedSource),
            }
        }
        if !owned || !linked {
            return Err(Error::SourceChanged);
        }
        let vc = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(
                r"SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64",
                KEY_READ | KEY_WOW64_64KEY,
            )
            .map_err(|_| Error::UnsupportedSource)?;
        let version = text(&vc, "Version")?;
        let parts: Vec<u32> = ["Major", "Minor", "Bld", "Rbld"]
            .into_iter()
            .map(|name| vc.get_value(name).map_err(|_| Error::SourceChanged))
            .collect::<Result<_, _>>()?;
        if vc.get_value::<u32, _>("Installed").ok() != Some(1)
            || parts[0] != 14
            || version
                .strip_prefix('v')
                .and_then(|value| {
                    value
                        .split('.')
                        .map(str::parse::<u32>)
                        .collect::<Result<Vec<_>, _>>()
                        .ok()
                })
                .as_ref()
                != Some(&parts)
        {
            return Err(Error::UnsupportedSource);
        }
        let mut buffer = [0u16; 32768];
        let count = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
        if count == 0 || count >= buffer.len() {
            return Err(Error::UnsupportedPlatform);
        }
        let system = PathBuf::from(OsString::from_wide(&buffer[..count]))
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?;
        let runtime = ["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"]
            .into_iter()
            .map(|name| {
                let path = system
                    .join(name)
                    .canonicalize()
                    .map_err(|_| Error::SourceChanged)?;
                ExpectedFileIdentity::capture(&path).map_err(|_| Error::SourceChanged)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            rg_tree: tree::snapshot(&rg_root)?,
            rg_root,
            rg_registration: values(&key)?,
            rg_link,
            rg: ExpectedFileIdentity::capture(&target).map_err(|_| Error::SourceChanged)?,
            vc_registration: values(&vc)?,
            runtime,
        })
    }

    pub(super) fn verify(&self) -> Result<(), Error> {
        if Self::capture()? != *self {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }
}
