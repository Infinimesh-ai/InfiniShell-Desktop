//! WinGet portable 来源通过当前用户 ARP 记录、原生文件摘要及实际公共链接共同绑定。
//! 宿主按官方单文件清单更新 portable 文件与 ARP，不派生任意管理器或改变安装来源。

use std::path::{Path, PathBuf};

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

use serde::{Deserialize, Serialize};

use super::{
    ArtifactRole, BoundInvocation, CLIAgent, Channel, Error, Installation, Source, absolute_env,
    brew,
};

pub(super) const SOURCE: &str = "Microsoft.Winget.Source_8wekyb3d8bbwe";
pub(super) const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

pub(super) fn supports(agent: CLIAgent, target: &str) -> Result<(), Error> {
    #[cfg(feature = "local_fs")]
    if agent == CLIAgent::Codex {
        return super::winget_codex::supports(agent, target);
    }
    #[cfg(feature = "local_fs")]
    if agent == CLIAgent::Grok {
        return super::winget_grok::supports(agent, target);
    }
    if agent == CLIAgent::Claude
        && target == "2.1.280"
        && cfg!(any(target_arch = "x86_64", target_arch = "aarch64"))
    {
        Ok(())
    } else {
        Err(Error::UnsupportedSource)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Owner {
    pub(super) product: String,
    pub(super) root: PathBuf,
    pub(super) target: PathBuf,
    pub(super) public: PathBuf,
    pub(super) version: String,
    pub(super) sha256: String,
}

impl Owner {
    pub(super) fn read() -> Result<Option<Self>, Error> {
        let product = format!("Anthropic.ClaudeCode_{SOURCE}");
        let key = match RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(format!(r"{UNINSTALL}\{product}"), KEY_READ)
        {
            Ok(key) => key,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Error::PermissionDenied),
        };
        if string(&key, "WinGetPackageIdentifier").as_deref() != Some("Anthropic.ClaudeCode")
            || string(&key, "WinGetSourceIdentifier").as_deref() != Some(SOURCE)
            || string(&key, "WinGetInstallerType").as_deref() != Some("portable")
        {
            return Ok(None);
        }
        let required = |name| string(&key, name).ok_or(Error::SourceChanged);
        Ok(Some(Self {
            product,
            root: PathBuf::from(required("InstallLocation")?),
            target: PathBuf::from(required("TargetFullPath")?),
            public: PathBuf::from(required("SymlinkFullPath")?),
            version: required("DisplayVersion")?,
            sha256: required("SHA256")?,
        }))
    }

    pub(super) fn verify_layout(&self) -> Result<(), Error> {
        let prefix = absolute_env("LOCALAPPDATA")
            .ok_or(Error::UnsupportedSource)?
            .join("Microsoft/WinGet");
        if self.product != format!("Anthropic.ClaudeCode_{SOURCE}")
            || self.root != prefix.join("Packages").join(&self.product)
            || self.target != self.root.join("claude.exe")
            || self.public != prefix.join("Links/claude.exe")
            || !std::fs::symlink_metadata(&self.public)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(Error::SourceChanged);
        }
        let link = std::fs::read_link(&self.public).map_err(|_| Error::SourceChanged)?;
        let link = if link.is_absolute() {
            link
        } else {
            self.public.parent().ok_or(Error::SourceChanged)?.join(link)
        };
        if link != self.target
            && link
                != self
                    .root
                    .canonicalize()
                    .map_err(|_| Error::SourceChanged)?
                    .join("claude.exe")
        {
            return Err(Error::SourceChanged);
        }
        super::parse_version(&self.version)?;
        brew::decode_sha256(&self.sha256)?;
        super::plain_ancestors(&self.target)
    }
}

fn string(key: &RegKey, name: &str) -> Option<String> {
    key.get_value::<String, _>(name)
        .ok()
        .filter(|value| !value.contains('\0'))
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.canonicalize()
        .ok()
        .zip(right.canonicalize().ok())
        .is_some_and(|(left, right)| left == right)
}

pub(super) fn discover(
    agent: CLIAgent,
    installation: &Installation,
    installed: &str,
    requested: Channel,
) -> Result<Option<Installation>, Error> {
    #[cfg(feature = "local_fs")]
    if agent == CLIAgent::Codex {
        return super::winget_codex::discover(agent, installation, installed, requested);
    }
    #[cfg(feature = "local_fs")]
    if agent == CLIAgent::Grok {
        return super::winget_grok::discover(agent, installation, installed, requested);
    }
    let (package, command) = match agent {
        CLIAgent::Claude => ("Anthropic.ClaudeCode", "claude.exe"),
        // Codex 的包可带 zip/辅助资源；不将一个同名 exe 认领为完整 portable 包。
        _ => return Ok(None),
    };
    let Some(local) = absolute_env("LOCALAPPDATA") else {
        return Ok(None);
    };
    let prefix = local.join("Microsoft/WinGet");
    let product = format!("{package}_{SOURCE}");
    let expected_root = prefix.join("Packages").join(&product);
    let registered = match RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(format!(r"{UNINSTALL}\{product}"), KEY_READ)
    {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::PermissionDenied),
    };
    if string(&registered, "WinGetPackageIdentifier").as_deref() != Some(package)
        || string(&registered, "WinGetSourceIdentifier").as_deref() != Some(SOURCE)
        || string(&registered, "WinGetInstallerType").as_deref() != Some("portable")
        || string(&registered, "DisplayVersion").as_deref() != Some(installed)
    {
        return Ok(None);
    }
    let Some(root) = string(&registered, "InstallLocation").map(PathBuf::from) else {
        return Ok(None);
    };
    let Some(target) = string(&registered, "TargetFullPath").map(PathBuf::from) else {
        return Ok(None);
    };
    let Some(public) = string(&registered, "SymlinkFullPath").map(PathBuf::from) else {
        return Ok(None);
    };
    if !root.is_absolute()
        || !target.is_absolute()
        || !public.is_absolute()
        || !same_path(&root, &expected_root)
        || target.parent() != Some(root.as_path())
        || target.file_name() != Some(command.as_ref())
        || public != prefix.join("Links").join(command)
        || installation.entry != public && installation.entry != target
        || !same_path(&target, &installation.stamp.canonical)
        || !std::fs::symlink_metadata(&public)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        || !same_path(&public, &target)
    {
        return Ok(None);
    }
    // WinGet 单文件 portable 布局将 SHA256 写入 ARP；带 SQLite 索引的多文件布局另需合同。
    let Some(recorded_digest) = string(&registered, "SHA256") else {
        return Ok(None);
    };
    if brew::decode_sha256(&recorded_digest)? != installation.stamp.digest {
        return Err(Error::SourceChanged);
    }
    super::plain_ancestors(&target)?;
    let mut found = installation.clone();
    found.source = Source::WinGet;
    found.channel = Channel::Latest;
    found.invocation = Some(BoundInvocation::manual_only(
        [
            (ArtifactRole::Program, target),
            (ArtifactRole::InstallRoot, root),
        ],
        ArtifactRole::Program,
        Vec::new(),
        "WinGet portable 由宿主事务发布，不派生管理器",
    ));
    found.error = if matches!(requested, Channel::FollowInstallation | Channel::Latest) {
        None
    } else {
        Some(Error::ChannelMismatch)
    };
    Ok(Some(found))
}
