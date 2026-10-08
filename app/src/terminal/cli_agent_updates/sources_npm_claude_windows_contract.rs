//! Claude Windows x64 官方 npm 归档与 cmd-shim 8 完整字节合同。
//! 安装脚本只作为受摘要保护的文件保留；宿主明确复制平台 native，不执行生命周期脚本。

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

pub(crate) const VERSION: &str = "2.1.280";
pub(crate) const WRAPPER_INTEGRITY: &str = "sha512-EZlX8jqNf+e7q9v+UoPbLYAbEGth7aDbcTytHzPYYohbP/fCfrjboCbcv85ZYGEq1Rq7Amm8hXLhuCKxLsabwA==";
pub(crate) const PLATFORM_INTEGRITY: &str = "sha512-/Cbb0f28a9iKVoZwgrGhNT7/5c8dhDbnKwaQvJFz8qjApgcfrWEtBmDQbOP0WbGH56NyytJGJ1oQfpfsCfIQcA==";
pub(crate) const DEPENDENCY: &str = "node_modules/@anthropic-ai/claude-code-win32-x64";
pub(crate) const NATIVE: &str = "node_modules/@anthropic-ai/claude-code-win32-x64/claude.exe";
pub(crate) const PUBLIC: &str = "bin/claude.exe";

#[derive(Deserialize)]
struct Release {
    wrapper: Artifact,
    platform: Artifact,
}

#[derive(Deserialize)]
struct Artifact {
    integrity: String,
    sha256: String,
    files: BTreeMap<PathBuf, File>,
}

#[derive(Deserialize)]
struct File {
    length: u64,
    sha256: String,
    mode: u32,
}

pub(crate) fn supports(version: &str) -> bool {
    matches!(version, "2.1.285" | "2.1.287")
}

fn release(version: &str) -> Option<Release> {
    if !supports(version) {
        return None;
    }
    let mut values: BTreeMap<String, Release> =
        serde_json::from_slice(include_bytes!("sources_npm_claude_windows_contract.json")).ok()?;
    values.remove(version)
}

pub(crate) fn integrities_for(version: &str) -> Option<(String, String)> {
    if version == VERSION {
        return Some((WRAPPER_INTEGRITY.into(), PLATFORM_INTEGRITY.into()));
    }
    let release = release(version)?;
    Some((release.wrapper.integrity, release.platform.integrity))
}

pub(crate) fn archive_sha256_for(version: &str) -> Option<(String, String)> {
    let release = release(version)?;
    Some((release.wrapper.sha256, release.platform.sha256))
}

pub(crate) fn archive_modes_for(version: &str) -> Option<BTreeMap<PathBuf, u32>> {
    let release = release(version)?;
    Some(
        release
            .wrapper
            .files
            .into_iter()
            .map(|(path, file)| (path, file.mode))
            .chain(
                release
                    .platform
                    .files
                    .into_iter()
                    .map(|(path, file)| (PathBuf::from(DEPENDENCY).join(path), file.mode)),
            )
            .collect(),
    )
}

pub(crate) fn verify_metadata(version: &str, wrapper: &[u8], platform: &[u8]) -> io::Result<()> {
    let invalid = || io::Error::other("Claude Windows npm 官方发行元数据不匹配");
    let release = release(version).ok_or_else(invalid)?;
    for (bytes, name, expected) in [
        (wrapper, "claude-code", release.wrapper),
        (platform, "claude-code-win32-x64", release.platform),
    ] {
        if bytes.len() > 1024 * 1024 {
            return Err(invalid());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        if value["name"] != format!("@anthropic-ai/{name}")
            || value["version"] != version
            || value["dist"]["integrity"] != expected.integrity
            || value["dist"]["tarball"]
                != format!("https://registry.npmjs.org/@anthropic-ai/{name}/-/{name}-{version}.tgz")
            || value["dist"]["fileCount"].as_u64() != Some(expected.files.len() as u64)
            || value["dist"]["unpackedSize"].as_u64()
                != Some(expected.files.values().map(|file| file.length).sum())
        {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(crate) fn archive_files_for(version: &str) -> Option<BTreeMap<PathBuf, (u64, String, bool)>> {
    if version == VERSION {
        return Some(archive_files());
    }
    let release = release(version)?;
    Some(
        release
            .wrapper
            .files
            .into_iter()
            .chain(
                release
                    .platform
                    .files
                    .into_iter()
                    .map(|(path, file)| (PathBuf::from(DEPENDENCY).join(path), file)),
            )
            .map(|(path, file)| (path, (file.length, file.sha256, file.mode & 0o111 != 0)))
            .collect(),
    )
}

pub(crate) fn files_for(version: &str) -> Option<BTreeMap<PathBuf, (u64, String, bool)>> {
    let mut files = archive_files_for(version)?;
    files.insert(PUBLIC.into(), files.get(&PathBuf::from(NATIVE))?.clone());
    Some(files)
}

pub(crate) fn archive_files() -> BTreeMap<PathBuf, (u64, String, bool)> {
    [
        (
            "cli-wrapper.cjs",
            4997,
            "61ad63033d9c8155d5e60a29f45dc4665afa07631c0b108e62cc83bf45ba490e",
            false,
        ),
        (
            "install.cjs",
            7196,
            "5cbab1670597f492cd4eeb946f3c344ebcb1fbd43c623ba192c9b33744461b85",
            false,
        ),
        (
            "bin/claude.exe",
            500,
            "6d7abae055d3b598281300a6c835086dec81bf3048f8a2294c5d3e50c8830d7b",
            false,
        ),
        (
            "package.json",
            1476,
            "11803205d31e7f72027ec4b8e0634644c722a921372b6046e1c39bf77d41d622",
            false,
        ),
        (
            "LICENSE.md",
            147,
            "8ce94b9478bb9868f9641f818e06cd722fbe55d4c22e2d2ed11971b20146173a",
            false,
        ),
        (
            "README.md",
            2037,
            "da7cf15ce4e35bad6a107acd6a36b4fe052083068bb9e564b1aa3f2078b5d0ce",
            false,
        ),
        (
            "sdk-tools.d.ts",
            167766,
            "4fdbd64f76e190800f48e93ddf4edcf6f3c87187396b586a75de808a78d8c1c4",
            false,
        ),
        (
            "node_modules/@anthropic-ai/claude-code-win32-x64/claude.exe",
            237100192,
            "0e4195524b73eb77efbdf3e2b36de5322a29f0ca575dfd2d9b4f946b1d425469",
            true,
        ),
        (
            "node_modules/@anthropic-ai/claude-code-win32-x64/package.json",
            272,
            "9663a715e25dede8f48202f6dd5f7017d23905f527b279757e8c6881be733186",
            false,
        ),
        (
            "node_modules/@anthropic-ai/claude-code-win32-x64/LICENSE.md",
            148,
            "fe8260e23b045e5d1ce9c31d2dfe06a39ec9227a12b1bde4c8c6bcb50e9310c8",
            false,
        ),
        (
            "node_modules/@anthropic-ai/claude-code-win32-x64/README.md",
            150,
            "88b447ebed5c162732eb18e9bd51aed947841a82272225a636b84bfbef06eb55",
            false,
        ),
    ]
    .into_iter()
    .map(|(path, length, digest, executable)| (path.into(), (length, digest.into(), executable)))
    .collect()
}

pub(crate) fn files() -> BTreeMap<PathBuf, (u64, String, bool)> {
    let mut files = archive_files();
    // install.cjs 的已审查语义：平台文件替换无 shebang 的占位文件。
    let native = files
        .get(&PathBuf::from(NATIVE))
        .expect("固定 native 合同")
        .clone();
    files.insert(PUBLIC.into(), native);
    files
}

// 模板中的官方英文注释参与完整摘要匹配，不作本地化替换。
pub(crate) fn cmd_shim() -> &'static str {
    "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n"
}
pub(crate) fn powershell_shim() -> &'static str {
    "#!/usr/bin/env pwsh\n$basedir=Split-Path $MyInvocation.MyCommand.Definition -Parent\n\n$exe=\"\"\nif ($PSVersionTable.PSVersion -lt \"6.0\" -or $IsWindows) {\n  # Fix case when both the Windows and Linux builds of Node\n  # are installed in the same directory\n  $exe=\".exe\"\n}\n# Support pipeline input\nif ($MyInvocation.ExpectingInput) {\n  $input | & \"$basedir/node_modules/@anthropic-ai/claude-code/bin/claude.exe\"   $args\n} else {\n  & \"$basedir/node_modules/@anthropic-ai/claude-code/bin/claude.exe\"   $args\n}\nexit $LASTEXITCODE\n"
}
pub(crate) fn shell_shim() -> &'static str {
    "#!/bin/sh\nbasedir=$(dirname \"$(echo \"$0\" | sed -e 's,\\\\,/,g')\")\n\ncase `uname` in\n    *CYGWIN*|*MINGW*|*MSYS*)\n        if command -v cygpath > /dev/null 2>&1; then\n            basedir=`cygpath -w \"$basedir\"`\n        fi\n    ;;\nesac\n\nexec \"$basedir/node_modules/@anthropic-ai/claude-code/bin/claude.exe\"   \"$@\"\n"
}
pub(crate) fn shims() -> [(&'static str, &'static str); 3] {
    [
        ("claude.cmd", cmd_shim()),
        ("claude.ps1", powershell_shim()),
        ("claude", shell_shim()),
    ]
}

#[cfg(test)]
#[path = "sources_npm_claude_windows_contract_tests.rs"]
mod tests;
