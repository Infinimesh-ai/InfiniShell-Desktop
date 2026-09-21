#!/usr/bin/env python3
"""准备显式选择的官方固定 Claude 文件；默认最新正式版，不安装或改写已有 CLI。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import stat
import subprocess
import sys
import tempfile
import urllib.request


VERSION = "2.1.273"
DEFAULT_VERSION = "2.1.278"
RELEASE_COMMIT = "d48ecfd7a41c16c42e0564f7a94947d6e4c50db1"
BASE_URL = f"https://downloads.claude.ai/claude-code-releases/{VERSION}"
MANIFEST_SHA256 = "02aa2311fd5d9a4cc9a5aea017b89f5067eec78013bd340f62450719a5393fca"
MANIFEST_SIZE = 2161
MARKER = ".infinishell-claude-fixed-inputs"
MARKER_CONTENTS = f"isolated Claude Code {VERSION} verification inputs\n".encode()
# 摘要来自已验证官方签名的固定清单；macOS 只用于核对已有原生文件。
RELEASES = {
    "linux-x64": ("claude", 228663608, "6c752e2cc7c110c9df15f26d8d134d438c5ae95dbd610efc1a308bf7f9c5f6c1"),
    "win32-x64": ("claude.exe", 231776416, "19654006672b6da7c945115eea99ca10051796016df563a65b3f0c7d72720ef0"),
    "darwin-arm64": ("claude", 212228880, "953e9880dbcb0b70f31c1f508de6a3fd389753d131688557fd992da9184693fb"),
    "darwin-x64": ("claude", 221023456, "2030ecf911e301e778b3c5a49068d6751384c830e48ea14f11eb61dd23622cee"),
}
RELEASE_CATALOG = {
    VERSION: {"commit": RELEASE_COMMIT, "manifest_sha256": MANIFEST_SHA256,
              "manifest_size": MANIFEST_SIZE, "platforms": RELEASES},
    "2.1.278": {
        "commit": "809c980662e3525645594dc8b74f78c38a348db1",
        "manifest_sha256": "d1bf63d94621d6aa6fb84297b235ddb8f5aaadc9252cef8020d170ef661b2f28",
        "manifest_size": 2161,
        "platforms": {
            "linux-x64": ("claude", 234119480, "5c4735937844e84f8a93306e841a5b0e12252909b07870f789b190468da147ab"),
            "win32-x64": ("claude.exe", 237232800, "006ea5c8638f67f10a5ae66bb232fd267c9f6af294e3f03f4cfcf1fd3f2cced8"),
            "darwin-arm64": ("claude", 217695408, "bd245662fb8a0e321b3bf133e930371d6563c387527885f30b2613aef3ba14d6"),
            "darwin-x64": ("claude", 226521952, "c522425e3d42275d2ac2238757ef8ba7f80d165a934044ec5a7a5fd7d7b9950b"),
        },
    },
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def release_contract(version=VERSION):
    require(isinstance(version, str) and version in RELEASE_CATALOG, "只允许已绑定官方摘要的 Claude 版本")
    return RELEASE_CATALOG[version]


def current_platform():
    architecture = {"amd64": "x64", "x86_64": "x64", "arm64": "arm64", "aarch64": "arm64"}.get(platform.machine().lower())
    target = f"{sys.platform}-{architecture}"
    require(target in RELEASES, "当前平台没有固定的原生 Claude 摘要")
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


def verify_binary(path, target, version=VERSION):
    contract = release_contract(version)
    require(target in contract["platforms"], "当前平台没有该固定 Claude 版本的摘要")
    name, size, checksum = contract["platforms"][target]
    require(regular_file(path).st_size == size and digest(path) == checksum,
            "原生 Claude 文件不匹配固定官方版本摘要")
    return {"platform": target, "binary": name, "bytes": size, "sha256": checksum,
            "url": f"https://downloads.claude.ai/claude-code-releases/{version}/{target}/{name}"}


def isolated_environment(root):
    allowed = {"PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "LANG", "LC_ALL"}
    env = {key: value for key, value in os.environ.items() if key.upper() in allowed}
    directories = {
        "HOME": "home", "USERPROFILE": "home", "APPDATA": "home/AppData/Roaming",
        "LOCALAPPDATA": "home/AppData/Local", "XDG_CONFIG_HOME": "home/.config",
        "XDG_DATA_HOME": "home/.local/share", "XDG_CACHE_HOME": "home/.cache",
        "CLAUDE_CONFIG_DIR": "claude", "TMPDIR": "tmp", "TMP": "tmp", "TEMP": "tmp",
    }
    for name, relative in directories.items():
        path = root / relative
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
        env[name] = str(path)
    env.update(CLAUDE_CODE_ENTRYPOINT="sdk-py", DISABLE_AUTOUPDATER="1", DISABLE_UPDATES="1",
               CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1", PYTHONUTF8="1")
    (root / "claude/settings.json").write_text("{}\n", encoding="utf-8", newline="\n")
    return env


def verify_version(executable, directory, version=VERSION):
    release_contract(version)
    with tempfile.TemporaryDirectory(prefix="version-", dir=directory) as temporary:
        root = Path(temporary).resolve()
        completed = subprocess.run([str(executable), "--version"], env=isolated_environment(root),
                                   cwd=root, capture_output=True, text=True, encoding="utf-8",
                                   errors="strict", timeout=10, check=True)
    require(completed.stdout.strip() == f"{version} (Claude Code)", "原生 Claude 报告的版本不匹配")
    return completed.stdout.strip()


def owned_directory(directory, explicit_private=False, version=VERSION):
    release_contract(version)
    marker_contents = f"isolated Claude Code {version} verification inputs\n".encode()
    require(not directory.is_symlink(), "测试目录不能是符号链接")
    if directory.exists():
        require(not getattr(directory.lstat(), "st_file_attributes", 0) & 0x400, "测试目录不能是 Windows 重解析点")
    directory = directory.resolve()
    repository = Path(__file__).resolve().parents[2]
    require(not directory.is_relative_to(repository), "测试文件必须位于源树外")
    if not explicit_private:
        require("RUNNER_TEMP" in os.environ, "必须设置 RUNNER_TEMP，或显式指定新的私有目录")
        runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
        require(directory != runner_temp and directory.is_relative_to(runner_temp), "下载目录必须位于 RUNNER_TEMP 的子目录")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = directory.lstat()
    require(stat.S_ISDIR(info.st_mode) and not getattr(info, "st_file_attributes", 0) & 0x400,
            "测试目录不能是 Windows 重解析点")
    require(os.name == "nt" or not stat.S_IMODE(info.st_mode) & 0o077, "已有测试目录必须只对当前用户开放")
    marker = directory / MARKER
    if marker.exists():
        regular_file(marker)
        require(marker.read_bytes() == marker_contents, "目录不属于此固定版本准备器")
    else:
        require(not any(directory.iterdir()), "不接管已有内容的目录")
        with marker.open("xb") as output:
            output.write(marker_contents)
    return directory


def fetch(url, destination, expected_size, expected_sha256, executable=False, version=VERSION):
    release_contract(version)
    base_url = f"https://downloads.claude.ai/claude-code-releases/{version}"
    require(url.startswith(base_url + "/"), "只允许固定官方版本的下载地址")
    if destination.exists() or destination.is_symlink():
        require(regular_file(destination).st_size == expected_size and digest(destination) == expected_sha256,
                "已有文件不匹配固定发布，拒绝覆盖")
        require(not executable or os.name == "nt" or destination.stat().st_mode & stat.S_IXUSR,
                "已有文件不可执行，拒绝悄悄修改权限")
        return
    descriptor, name = tempfile.mkstemp(prefix=".claude-download-", dir=destination.parent)
    temporary = Path(name)
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "InfiniShell-fixed-Claude-verification"})
        with os.fdopen(descriptor, "wb") as output, urllib.request.urlopen(request, timeout=60) as response:
            checksum = hashlib.sha256()
            count = 0
            while chunk := response.read(1024 * 1024):
                count += len(chunk)
                require(count <= expected_size, "下载超过固定发布大小")
                checksum.update(chunk)
                output.write(chunk)
            output.flush()
            os.fsync(output.fileno())
        require(count == expected_size and checksum.hexdigest() == expected_sha256, "下载大小或摘要不匹配")
        if executable and os.name != "nt":
            temporary.chmod(0o700)
        # 原子创建并拒绝覆盖；临时名称释放后目标仍然只有一个硬链接。
        os.link(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claude-version", choices=tuple(RELEASE_CATALOG), default=DEFAULT_VERSION,
                        help=f"精确官方版本；缺省使用最新正式版 {DEFAULT_VERSION}")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--download-dir", type=Path, help="RUNNER_TEMP 内的专用下载目录")
    group.add_argument("--private-directory", type=Path, help="显式指定源树外的新私有目录；已有目录必须带本准备器标记")
    args = parser.parse_args()
    version = args.claude_version
    contract = release_contract(version)
    base_url = f"https://downloads.claude.ai/claude-code-releases/{version}"
    target = current_platform()
    require(target in ("linux-x64", "win32-x64"), "准备器仅下载 Linux/Windows x64；macOS 探测使用已有受测原生文件")
    directory = owned_directory(args.private_directory or args.download_dir, args.private_directory is not None,
                                version=version)
    manifest = directory / "manifest.json"
    fetch(f"{base_url}/manifest.json", manifest, contract["manifest_size"], contract["manifest_sha256"], version=version)
    metadata = json.loads(manifest.read_text(encoding="utf-8"))
    name, size, checksum = contract["platforms"][target]
    require(metadata["version"] == version and metadata["commit"] == contract["commit"] and
            metadata["platforms"][target] == {"binary": name, "size": size, "checksum": checksum},
            "固定签名清单与内置契约不一致")
    executable = directory / name
    fetch(f"{base_url}/{target}/{name}", executable, size, checksum, executable=True, version=version)
    evidence = verify_binary(executable, target, version)
    evidence.update(version=verify_version(executable, directory, version), release_commit=contract["commit"],
                    manifest_sha256=digest(manifest), installed=False, credentials_provided=False)
    verify_binary(executable, target, version)
    report = directory / "claude-fixed-inputs.json"
    if report.exists() or report.is_symlink():
        regular_file(report)
    report.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(executable.resolve())


if __name__ == "__main__":
    main()
