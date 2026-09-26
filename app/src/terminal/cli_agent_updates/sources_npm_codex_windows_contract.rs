//! Codex 0.156.1 Windows x64 npm 与 cmd-shim 6.0.1 / 8 的固定字节合同。
//! 此模块不读取安装环境，来源事务与进程监督者分别用相同合同核验输入。

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use sha2::{Digest as _, Sha256};

pub(crate) const VERSION: &str = "0.156.1";
pub(crate) const WRAPPER_INTEGRITY: &str = "sha512-nI1iVl/n2SO2lSvlwEsJx63zdSI4C4Me2gR7AG0OWMJiGSakz2tY2hx43E39Zq5aEoeB5bZjJXzp5Sqhog6vyA==";
pub(crate) const PLATFORM_INTEGRITY: &str = "sha512-MJyLxbBs2zzp5kbaR/99Zwe7SmbrwUkveTcT+ayYlO48V0nYh0eU+h2lalBwvC7VJ/ya/bXnUtISJfJKhGCD/g==";
pub(crate) const DEPENDENCY: &str = "node_modules/@openai/codex-win32-x64";
pub(crate) const VENDOR: &str = "vendor/x86_64-pc-windows-msvc";
const MANIFEST: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/codex_0156_package_manifest.json");

pub(crate) fn files() -> io::Result<BTreeMap<PathBuf, (u64, String, bool)>> {
    let value: serde_json::Value = serde_json::from_slice(MANIFEST).map_err(io::Error::other)?;
    let package = &value["packages"]["windows-x64"];
    if value["version"] != VERSION || package["target"] != "x86_64-pc-windows-msvc" {
        return Err(io::Error::other("Codex Windows npm 固定清单不匹配"));
    }
    let native: BTreeMap<PathBuf, (u64, String, u32)> =
        serde_json::from_value(package["files"].clone()).map_err(io::Error::other)?;
    let mut files: BTreeMap<_, _> = native
        .into_iter()
        .map(|(path, (length, digest, mode))| {
            (
                PathBuf::from(DEPENDENCY).join(VENDOR).join(path),
                (length, digest, mode & 0o111 != 0),
            )
        })
        .collect();
    for (name, length, digest, executable) in [
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
        (
            "node_modules/@openai/codex-win32-x64/package.json",
            511,
            "8b95460d97552a63fcc53e13f216b51f3a0a59700b65dbafb0adac4ba8c03628",
            false,
        ),
        (
            "node_modules/@openai/codex-win32-x64/README.md",
            3334,
            "ba4e1f69ff48386e72a9c5e1edaf76aad64a475c2d51af79ccba6d1128261ba7",
            false,
        ),
    ] {
        files.insert(name.into(), (length, digest.into(), executable));
    }
    Ok(files)
}

pub(crate) fn cmd_shim() -> &'static str {
    concat!(
        "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n",
        "\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\n",
        "endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*\r\n"
    )
}

pub(crate) fn powershell_shim() -> &'static str {
    // 原样模板是 npm 协议字节；其中英文注释也参与完整匹配，不作本地化替换。
    concat!(
        "#!/usr/bin/env pwsh\n$basedir=Split-Path $MyInvocation.MyCommand.Definition -Parent\n\n",
        "$exe=\"\"\nif ($PSVersionTable.PSVersion -lt \"6.0\" -or $IsWindows) {\n  # Fix case when both the Windows and Linux builds of Node\n  # are installed in the same directory\n  $exe=\".exe\"\n}\n",
        "$ret=0\nif (Test-Path \"$basedir/node$exe\") {\n  # Support pipeline input\n  if ($MyInvocation.ExpectingInput) {\n    $input | & \"$basedir/node$exe\"  \"$basedir/node_modules/@openai/codex/bin/codex.js\" $args\n  } else {\n    & \"$basedir/node$exe\"  \"$basedir/node_modules/@openai/codex/bin/codex.js\" $args\n  }\n  $ret=$LASTEXITCODE\n} else {\n  # Support pipeline input\n  if ($MyInvocation.ExpectingInput) {\n    $input | & \"node$exe\"  \"$basedir/node_modules/@openai/codex/bin/codex.js\" $args\n  } else {\n    & \"node$exe\"  \"$basedir/node_modules/@openai/codex/bin/codex.js\" $args\n  }\n  $ret=$LASTEXITCODE\n}\nexit $ret\n"
    )
}

fn shell_shim_8() -> &'static str {
    concat!(
        "#!/bin/sh\nbasedir=$(dirname \"$(echo \"$0\" | sed -e 's,\\\\,/,g')\")\n\ncase `uname` in\n    *CYGWIN*|*MINGW*|*MSYS*)\n        if command -v cygpath > /dev/null 2>&1; then\n            basedir=`cygpath -w \"$basedir\"`\n        fi\n    ;;\nesac\n\n",
        "if [ -x \"$basedir/node\" ]; then\n  exec \"$basedir/node\"  \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nelse \n  exec node  \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nfi\n"
    )
}

fn shell_shim_601() -> &'static str {
    concat!(
        "#!/bin/sh\nbasedir=$(dirname \"$(echo \"$0\" | sed -e 's,\\\\,/,g')\")\n\ncase `uname` in\n    *CYGWIN*|*MINGW*|*MSYS*) basedir=`cygpath -w \"$basedir\"`;;\nesac\n\n",
        "if [ -x \"$basedir/node\" ]; then\n  exec \"$basedir/node\"  \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nelse \n  exec node  \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nfi\n"
    )
}

pub(crate) const SHIM_NAMES: [&str; 3] = ["codex.cmd", "codex.ps1", "codex"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShimTemplate {
    CmdShim601,
    CmdShim8,
}

impl ShimTemplate {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::CmdShim601 => "cmd-shim-6.0.1",
            Self::CmdShim8 => "cmd-shim-8",
        }
    }

    pub(crate) fn shims(self) -> [(&'static str, &'static str); 3] {
        let shell = match self {
            Self::CmdShim601 => shell_shim_601(),
            Self::CmdShim8 => shell_shim_8(),
        };
        [
            ("codex.cmd", cmd_shim()),
            ("codex.ps1", powershell_shim()),
            ("codex", shell),
        ]
    }
}

/// 从已持久化的三个摘要选择完整合同；不归一化字节，也不逐文件混用模板。
pub(crate) fn identify_shims(digests: &BTreeMap<String, String>) -> Option<ShimTemplate> {
    if digests.len() != SHIM_NAMES.len() {
        return None;
    }
    [ShimTemplate::CmdShim601, ShimTemplate::CmdShim8]
        .into_iter()
        .find(|template| {
            template.shims().into_iter().all(|(name, contents)| {
                digests.get(name) == Some(&format!("{:x}", Sha256::digest(contents.as_bytes())))
            })
        })
}

#[cfg(test)]
#[path = "sources_npm_codex_windows_contract_tests.rs"]
mod tests;
