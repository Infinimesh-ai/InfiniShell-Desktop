#!/usr/bin/env python3
"""准备官方固定 Grok 1.0.30 原生文件；只写私有测试目录，默认仅运行 --version。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import stat
import struct
import subprocess
import sys
import tempfile
import urllib.request


VERSION = "1.0.30"
VERSION_OUTPUT = "grok 1.0.30 (04b7ffed98c6)"
SOURCE_COMMIT = "482711333c7195dc16a272777f86086d615e2afb"
BASE_URL = "https://x.ai/cli"
MARKER = ".infinishell-grok-fixed-inputs"
MARKER_CONTENTS = f"isolated Grok {VERSION} verification inputs\n".encode()
# 摘要来自固定官方地址的完整下载；并非官方签名或官方发布的 SHA-256 清单。
RELEASES = {
    "linux-x64": ("grok-1.0.30-linux-x86_64", "grok", 161725088,
                  "504dd6546ab991b75d36698242875ce461489cd1f8cd84285873cb55bd5c7d54"),
    "win32-x64": ("grok-1.0.30-windows-x86_64.exe", "grok.exe", 150036808,
                  "ca24ea63272ba7881261f4a52498d1f5bd884b01da25845990422a10dd315266"),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def current_platform():
    architecture = {"amd64": "x64", "x86_64": "x64"}.get(platform.machine().lower())
    target = f"{sys.platform}-{architecture}"
    require(target in RELEASES, "当前平台没有本准备器固定的原生 Grok 文件")
    return target


def regular_file(path):
    info = path.lstat()
    require(not path.is_symlink() and not getattr(info, "st_file_attributes", 0) & 0x400,
            "拒绝符号链接或 Windows 重解析点")
    require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1, "只接受无硬链接的普通文件")
    return info


def digest(path):
    regular_file(path)
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def binary_format(path, target):
    regular_file(path)
    with path.open("rb") as source:
        header = source.read(64)
        require(len(header) == 64, "原生文件头不完整")
        if target == "linux-x64":
            require(header[:6] == b"\x7fELF\x02\x01" and
                    struct.unpack_from("<HH", header, 16) == (3, 62), "需要 x86-64 小端 ELF PIE")
            return "elf64-x86-64"
        require(target == "win32-x64" and header[:2] == b"MZ", "需要 Windows PE 文件")
        offset = struct.unpack_from("<I", header, 60)[0]
        require(64 <= offset <= 1024 * 1024, "PE 文件头偏移越界")
        source.seek(offset)
        pe = source.read(26)
        require(len(pe) == 26 and pe[:4] == b"PE\x00\x00" and
                struct.unpack_from("<H", pe, 4)[0] == 0x8664 and
                struct.unpack_from("<H", pe, 24)[0] == 0x20B, "需要 Windows x86-64 PE32+")
        return "pe32+-x86-64"


def verify_binary(path, target):
    artifact, name, size, checksum = RELEASES[target]
    require(regular_file(path).st_size == size and digest(path) == checksum,
            "原生 Grok 文件不匹配固定官方下载摘要")
    return {"platform": target, "artifact": artifact, "binary": name, "bytes": size,
            "sha256": checksum, "url": f"{BASE_URL}/{artifact}", "format": binary_format(path, target)}


def isolated_environment(root):
    allowed = {"PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "LANG", "LC_ALL"}
    env = {key: value for key, value in os.environ.items() if key.upper() in allowed}
    directories = {
        "HOME": "home", "USERPROFILE": "home", "APPDATA": "home/AppData/Roaming",
        "LOCALAPPDATA": "home/AppData/Local", "XDG_CONFIG_HOME": "home/.config",
        "XDG_DATA_HOME": "home/.local/share", "XDG_CACHE_HOME": "home/.cache",
        "GROK_HOME": "grok", "CODEX_HOME": "codex", "CLAUDE_CONFIG_DIR": "claude",
        "TMPDIR": "tmp", "TMP": "tmp", "TEMP": "tmp",
    }
    for name, relative in directories.items():
        path = root / relative
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
        env[name] = str(path)
    env.update(GROK_AUTO_UPDATE="0", GROK_DISABLE_AUTOUPDATER="1",
               GROK_CLAUDE_HOOKS_ENABLED="0", GROK_CLAUDE_MCPS_ENABLED="0",
               GROK_CODEX_HOOKS_ENABLED="0", GROK_CODEX_MCPS_ENABLED="0")
    return env


def verify_version(executable, directory):
    with tempfile.TemporaryDirectory(prefix="version-", dir=directory) as temporary:
        root = Path(temporary).resolve()
        completed = subprocess.run([str(executable), "--version"], env=isolated_environment(root),
                                   cwd=root, capture_output=True, text=True, encoding="utf-8",
                                   errors="strict", timeout=10, check=True)
    require(completed.stdout.strip() == VERSION_OUTPUT, "原生 Grok 报告的版本不匹配")
    return completed.stdout.strip()


def owned_directory(directory, explicit_private=False):
    require(not directory.is_symlink(), "测试目录不能是符号链接")
    if directory.exists():
        require(not getattr(directory.lstat(), "st_file_attributes", 0) & 0x400,
                "测试目录不能是 Windows 重解析点")
    directory = directory.resolve()
    repository = Path(__file__).resolve().parents[2]
    require(not directory.is_relative_to(repository), "测试文件必须位于源树外")
    if not explicit_private:
        require("RUNNER_TEMP" in os.environ, "必须设置 RUNNER_TEMP，或显式指定新的私有目录")
        runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
        require(directory != runner_temp and directory.is_relative_to(runner_temp),
                "下载目录必须位于 RUNNER_TEMP 的子目录")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = directory.lstat()
    require(stat.S_ISDIR(info.st_mode) and not getattr(info, "st_file_attributes", 0) & 0x400,
            "测试目录不能是 Windows 重解析点")
    require(os.name == "nt" or not stat.S_IMODE(info.st_mode) & 0o077,
            "已有测试目录必须只对当前用户开放")
    marker = directory / MARKER
    if marker.exists() or marker.is_symlink():
        regular_file(marker)
        require(marker.read_bytes() == MARKER_CONTENTS, "目录不属于此固定版本准备器")
    else:
        require(not any(directory.iterdir()), "不接管已有内容的目录")
        with marker.open("xb") as output:
            output.write(MARKER_CONTENTS)
    return directory


def fetch(url, destination, expected_size, expected_sha256):
    require(url in {f"{BASE_URL}/{release[0]}" for release in RELEASES.values()},
            "只允许内置的两个固定官方版本下载地址")
    if destination.exists() or destination.is_symlink():
        require(regular_file(destination).st_size == expected_size and digest(destination) == expected_sha256,
                "已有文件不匹配固定发布，拒绝覆盖")
        require(os.name == "nt" or destination.stat().st_mode & stat.S_IXUSR,
                "已有文件不可执行，拒绝悄悄修改权限")
        return
    descriptor, name = tempfile.mkstemp(prefix=".grok-download-", dir=destination.parent)
    temporary = Path(name)
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "InfiniShell-fixed-Grok-verification"})
        with os.fdopen(descriptor, "wb") as output, urllib.request.urlopen(request, timeout=60) as response:
            # 不跟随未核实的新发行位置；官方固定地址变化应先更新验证记录。
            require(response.geturl() == url, "固定官方地址发生重定向")
            checksum = hashlib.sha256()
            count = 0
            while chunk := response.read(1024 * 1024):
                count += len(chunk)
                require(count <= expected_size, "下载超过固定发布大小")
                checksum.update(chunk)
                output.write(chunk)
            output.flush()
            os.fsync(output.fileno())
        require(count == expected_size and checksum.hexdigest() == expected_sha256,
                "下载大小或摘要不匹配")
        if os.name != "nt":
            temporary.chmod(0o700)
        # 原子创建并拒绝覆盖；临时名称释放后目标仍然只有一个硬链接。
        os.link(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--download-dir", type=Path, help="RUNNER_TEMP 内的专用下载目录")
    group.add_argument("--private-directory", type=Path, help="显式指定源树外的新私有目录")
    parser.add_argument("--download-only", action="store_true", help="只验证完整下载字节，不执行原生文件")
    parser.add_argument("--target", choices=RELEASES, help="仅 --download-only 允许指定其他平台")
    args = parser.parse_args()
    require(args.target is None or args.download_only, "跨平台选择只允许下载，不能执行")
    target = args.target or current_platform()
    directory = owned_directory(args.private_directory or args.download_dir, args.private_directory is not None)
    artifact, name, size, checksum = RELEASES[target]
    executable = directory / name
    fetch(f"{BASE_URL}/{artifact}", executable, size, checksum)
    evidence = verify_binary(executable, target)
    evidence.update(version=VERSION, version_output=None, native_version_verified=False,
                    installed=False, credentials_provided=False, model_input_submitted=False,
                    checksum_origin="observed_complete_official_download", archive=False)
    if not args.download_only:
        evidence.update(version_output=verify_version(executable, directory), native_version_verified=True)
        verify_binary(executable, target)
    report = directory / "grok-fixed-inputs.json"
    if report.exists() or report.is_symlink():
        regular_file(report)
    report.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(executable.resolve())


if __name__ == "__main__":
    main()
