//! Grok 1.0.41 Windows npm 的固定完整包与公开 Node shim 合同。
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

pub(crate) const VERSION: &str = "1.0.41";
pub(crate) const DEPENDENCY: &str = "node_modules/@xai-official/grok-win32-x64";
pub(crate) const COMPRESSED: &str = "node_modules/@xai-official/grok-win32-x64/bin/grok.exe.br";
const MANIFEST: &[u8] =
    include_bytes!("../../../../script/cli-agent-parity/grok_1041_npm_manifest.json");
pub(crate) fn files(version: &str) -> io::Result<BTreeMap<PathBuf, (u64, String, bool)>> {
    let value: serde_json::Value = serde_json::from_slice(MANIFEST).map_err(io::Error::other)?;
    if value["schema"] != 1 || !matches!(version, "1.0.40" | VERSION) {
        return Err(io::Error::other("Grok npm 固定版本不匹配"));
    }
    let mut result = BTreeMap::new();
    for (key, prefix) in [
        (format!("@xai-official/grok@{version}"), ""),
        (
            format!("@xai-official/grok-win32-x64@{version}"),
            DEPENDENCY,
        ),
        ("@iarna/toml@3.0.0".to_owned(), "node_modules/@iarna/toml"),
    ] {
        let files = value["packages"][&key]["files"]
            .as_object()
            .ok_or_else(|| io::Error::other("Grok npm 包清单缺失"))?;
        for (name, file) in files {
            result.insert(
                PathBuf::from(prefix).join(name),
                (
                    file["length"]
                        .as_u64()
                        .ok_or_else(|| io::Error::other("Grok npm 长度缺失"))?,
                    file["sha256"]
                        .as_str()
                        .ok_or_else(|| io::Error::other("Grok npm 摘要缺失"))?
                        .to_owned(),
                    file["executable"]
                        .as_bool()
                        .ok_or_else(|| io::Error::other("Grok npm 类型缺失"))?,
                ),
            );
        }
    }
    Ok(result)
}
pub(crate) fn native(version: &str) -> io::Result<(u64, String)> {
    let value: serde_json::Value = serde_json::from_slice(MANIFEST).map_err(io::Error::other)?;
    if !matches!(version, "1.0.40" | VERSION) {
        return Err(io::Error::other("Grok npm 版本不匹配"));
    }
    let image = &value["native"][version]["win32-x64"];
    Ok((
        image["length"]
            .as_u64()
            .ok_or_else(|| io::Error::other("Grok npm 映像长度缺失"))?,
        image["sha256"]
            .as_str()
            .ok_or_else(|| io::Error::other("Grok npm 映像摘要缺失"))?
            .to_owned(),
    ))
}

pub(crate) fn cmd_shim() -> &'static str {
    concat!(
        "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n",
        "\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\n",
        "endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@xai-official\\grok\\bin\\grok\" %*\r\n"
    )
}

pub(crate) fn powershell_shim() -> &'static str {
    // 原样模板是 npm 协议字节；其中英文注释也参与完整匹配，不作本地化替换。
    concat!(
        "#!/usr/bin/env pwsh\n$basedir=Split-Path $MyInvocation.MyCommand.Definition -Parent\n\n",
        "$exe=\"\"\nif ($PSVersionTable.PSVersion -lt \"6.0\" -or $IsWindows) {\n  # Fix case when both the Windows and Linux builds of Node\n  # are installed in the same directory\n  $exe=\".exe\"\n}\n",
        "$ret=0\nif (Test-Path \"$basedir/node$exe\") {\n  # Support pipeline input\n  if ($MyInvocation.ExpectingInput) {\n    $input | & \"$basedir/node$exe\"  \"$basedir/node_modules/@xai-official/grok/bin/grok\" $args\n  } else {\n    & \"$basedir/node$exe\"  \"$basedir/node_modules/@xai-official/grok/bin/grok\" $args\n  }\n  $ret=$LASTEXITCODE\n} else {\n  # Support pipeline input\n  if ($MyInvocation.ExpectingInput) {\n    $input | & \"node$exe\"  \"$basedir/node_modules/@xai-official/grok/bin/grok\" $args\n  } else {\n    & \"node$exe\"  \"$basedir/node_modules/@xai-official/grok/bin/grok\" $args\n  }\n  $ret=$LASTEXITCODE\n}\nexit $ret\n"
    )
}

pub(crate) fn shell_shim() -> &'static str {
    concat!(
        "#!/bin/sh\nbasedir=$(dirname \"$(echo \"$0\" | sed -e 's,\\\\,/,g')\")\n\ncase `uname` in\n    *CYGWIN*|*MINGW*|*MSYS*)\n        if command -v cygpath > /dev/null 2>&1; then\n            basedir=`cygpath -w \"$basedir\"`\n        fi\n    ;;\nesac\n\n",
        "if [ -x \"$basedir/node\" ]; then\n  exec \"$basedir/node\"  \"$basedir/node_modules/@xai-official/grok/bin/grok\" \"$@\"\nelse \n  exec node  \"$basedir/node_modules/@xai-official/grok/bin/grok\" \"$@\"\nfi\n"
    )
}

pub(crate) fn shims() -> [(&'static str, &'static str); 3] {
    [
        ("grok.cmd", cmd_shim()),
        ("grok.ps1", powershell_shim()),
        ("grok", shell_shim()),
    ]
}
