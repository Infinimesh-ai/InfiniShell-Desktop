//! npm 来源识别只接受包清单与实际入口的一一对应，不授予更新执行能力。

use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::{CLIAgent, Error, Installation, MAX_CONFIG, Stamp, read_limited, stamp};

pub(super) struct Registration {
    pub(super) prefix: PathBuf,
    pub(super) package_root: PathBuf,
    pub(super) manifest: PathBuf,
    pub(super) stamp: Stamp,
}

pub(super) fn package_name(agent: CLIAgent) -> Option<&'static str> {
    match agent {
        CLIAgent::Codex => Some("@openai/codex"),
        CLIAgent::Claude => Some("@anthropic-ai/claude-code"),
        CLIAgent::Grok => Some("@xai-official/grok"),
        CLIAgent::Gemini
        | CLIAgent::Amp
        | CLIAgent::Droid
        | CLIAgent::OpenCode
        | CLIAgent::Copilot
        | CLIAgent::Pi
        | CLIAgent::OhMyPi
        | CLIAgent::Auggie
        | CLIAgent::CursorCli
        | CLIAgent::Goose
        | CLIAgent::DeepSeek
        | CLIAgent::Hermes
        | CLIAgent::Vibe
        | CLIAgent::Antigravity
        | CLIAgent::Omp
        | CLIAgent::WarpTui
        | CLIAgent::Unknown => None,
    }
}

fn relative_bin(value: &Value, command: &str) -> Option<PathBuf> {
    let bin = value.get("bin")?;
    let bin = bin
        .as_str()
        .or_else(|| bin.get(command).and_then(Value::as_str))?;
    let bin = bin.strip_prefix("./").unwrap_or(bin);
    // 包路径采用 npm 的正斜杠形式；同时拒绝两种平台上的转义及 shell 元字符。
    if bin.is_empty()
        || bin
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || bin.chars().any(|character| {
            character.is_control()
                || matches!(character, '\\' | ':' | '"' | '%' | '&' | '|' | '<' | '>')
        })
    {
        return None;
    }
    Some(PathBuf::from(bin))
}

fn read_registration(path: &Path) -> Result<Option<(Value, Stamp)>, Error> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::ProbeFailed),
    };
    if !metadata.file_type().is_file() {
        return Ok(None);
    }
    let identity = stamp(path)?;
    let bytes = read_limited(path, MAX_CONFIG)?;
    if <[u8; 32]>::from(Sha256::digest(&bytes)) != identity.digest {
        return Err(Error::SourceChanged);
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| Error::ProbeFailed)?;
    Ok(Some((value, identity)))
}

pub(super) fn registered_manager(cli: &Path) -> Result<bool, Error> {
    let canonical = cli.canonicalize().map_err(|_| Error::SourceChanged)?;
    let Some(root) = canonical.parent().and_then(Path::parent) else {
        return Ok(false);
    };
    let Some((value, identity)) = read_registration(&root.join("package.json"))? else {
        return Ok(false);
    };
    if value.get("name").and_then(Value::as_str) != Some("npm")
        || value.get("version").and_then(Value::as_str).is_none()
        || relative_bin(&value, "npm").is_none_or(|bin| root.join(bin) != canonical)
    {
        return Ok(false);
    }
    if stamp(&identity.canonical)? != identity {
        return Err(Error::SourceChanged);
    }
    Ok(true)
}

pub(super) fn registered_installation(
    agent: CLIAgent,
    installation: &Installation,
    installed: &str,
    prefix: &Path,
    windows_layout: bool,
) -> Result<Option<Registration>, Error> {
    let Some(package) = package_name(agent) else {
        return Ok(None);
    };
    if !prefix.is_absolute() {
        return Ok(None);
    }
    let prefix = match prefix.canonicalize() {
        Ok(prefix) => prefix,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::SourceChanged),
    };
    let package_root = prefix
        .join(if windows_layout {
            "node_modules"
        } else {
            "lib/node_modules"
        })
        .join(package);
    // npm link 或越出前缀的目录链接不是此全局安装的拥有权证明。
    if package_root.canonicalize().ok().as_ref() != Some(&package_root) {
        return Ok(None);
    }
    let manifest = package_root.join("package.json");
    let Some((value, manifest_stamp)) = read_registration(&manifest)? else {
        return Ok(None);
    };
    if value.get("name").and_then(Value::as_str) != Some(package)
        || value.get("version").and_then(Value::as_str) != Some(installed)
    {
        return Ok(None);
    }
    let Some(bin) = relative_bin(&value, agent.command_prefix()) else {
        return Ok(None);
    };
    let bin_path = package_root.join(&bin);
    let Ok(canonical_bin) = bin_path.canonicalize() else {
        return Ok(None);
    };
    if !canonical_bin.starts_with(&package_root) {
        return Ok(None);
    }
    let matches_entry = if canonical_bin == installation.stamp.canonical {
        true
    } else if agent == CLIAgent::Grok && cfg!(feature = "local_fs") {
        #[cfg(feature = "local_fs")]
        {
            if super::npm_grok_contract::registered_mirror(installation, &package_root, installed)?
            {
                true
            } else if windows_layout {
                matches_windows_cmd_shim(installation, &prefix, package, &bin, &bin_path)?
            } else {
                false
            }
        }
        #[cfg(not(feature = "local_fs"))]
        {
            false
        }
    } else if windows_layout {
        matches_windows_cmd_shim(installation, &prefix, package, &bin, &bin_path)?
    } else {
        false
    };
    if !matches_entry {
        return Ok(None);
    }
    if stamp(&manifest)? != manifest_stamp || stamp(&installation.entry)? != installation.stamp {
        return Err(Error::SourceChanged);
    }
    Ok(Some(Registration {
        prefix,
        package_root,
        manifest,
        stamp: manifest_stamp,
    }))
}

fn matches_windows_cmd_shim(
    installation: &Installation,
    prefix: &Path,
    package: &str,
    bin: &Path,
    bin_path: &Path,
) -> Result<bool, Error> {
    // cmd-shim 的完整脚本必须匹配；只搜包含目标路径的行会误接受自定义附加命令。
    #[cfg(windows)]
    if package == "@openai/codex"
        && bin == Path::new("bin/codex.js")
        && installation.stamp.canonical == prefix.join("codex.ps1")
    {
        return Ok(read_limited(&installation.entry, 16 * 1024)?
            == super::npm_windows_contract::powershell_shim().as_bytes());
    }
    #[cfg(windows)]
    if package == "@anthropic-ai/claude-code"
        && bin == Path::new("bin/claude.exe")
        && installation.stamp.canonical == prefix.join("claude.ps1")
    {
        return Ok(read_limited(&installation.entry, 16 * 1024)?
            == super::npm_claude_windows_contract::powershell_shim().as_bytes());
    }
    #[cfg(windows)]
    if package == "@xai-official/grok" && bin == Path::new("bin/grok")
        && installation.stamp.canonical == prefix.join("grok.ps1")
    {
        return Ok(read_limited(&installation.entry, 16 * 1024)?
            == super::npm_grok_windows_contract::powershell_shim().as_bytes());
    }
    let expected_entry = prefix.join(format!(
        "{}.cmd",
        match package {
            "@openai/codex" => "codex",
            "@xai-official/grok" => "grok",
            "@anthropic-ai/claude-code" => "claude",
            _ => return Ok(false),
        }
    ));
    if installation.stamp.canonical != expected_entry {
        return Ok(false);
    }
    let target = format!("node_modules/{package}/{}", bin.to_string_lossy()).replace('/', "\\");
    let mut header = [0; 128];
    let length = File::open(bin_path)
        .and_then(|mut file| file.read(&mut header))
        .map_err(|_| Error::SourceChanged)?;
    let node_script = header[..length].starts_with(b"#!/usr/bin/env node\n")
        || header[..length].starts_with(b"#!/usr/bin/env node\r\n");
    if header[..length].starts_with(b"#!") && !node_script {
        return Ok(false);
    }
    let expected = windows_cmd_shim(&target, node_script);
    let bytes = read_limited(&installation.entry, 16 * 1024)?;
    Ok(bytes == expected.as_bytes())
}

fn windows_cmd_shim(target: &str, node_script: bool) -> String {
    // npm cmd-shim 8 的两个官方模板：Node 脚本与无 shebang 的原生可执行文件。
    let head = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n";
    if node_script {
        format!(
            "{head}\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\{target}\" %*\r\n"
        )
    } else {
        format!("{head}\"%dp0%\\{target}\"   %*\r\n")
    }
}

#[cfg(test)]
#[path = "sources_npm_tests.rs"]
mod tests;
