//! Grok portable 只绑定已经安装的 VC Runtime；不安装依赖、不修改系统登记。

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

use super::Error;
use super::managed_process::ExpectedFileIdentity;
use super::winget_codex_dependencies::{Values, text, values};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Runtime {
    registration: Values,
    pub(super) files: Vec<ExpectedFileIdentity>,
}
impl Runtime {
    pub(super) fn capture() -> Result<Self, Error> {
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
            registration: values(&vc)?,
            files: runtime,
        })
    }
    pub(super) fn verify(&self) -> Result<(), Error> {
        if Self::capture()? != *self {
            return Err(Error::SourceChanged);
        }
        Ok(())
    }
}
