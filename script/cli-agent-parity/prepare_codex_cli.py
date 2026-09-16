#!/usr/bin/env python3
"""在 RUNNER_TEMP 中准备固定 Codex 0.147.0，只向标准输出返回绝对可执行文件路径。"""

import argparse
import hashlib
import os
from pathlib import Path
import platform
import stat
import subprocess
import sys
import tarfile
import tempfile

sys.dont_write_bytecode = True
from codex_windows_hook_inputs import (CODEX_VERSION, RELEASE_ASSETS, fetch_file, regular_file,
                                       require, sha256, verify_codex)


LINUX_NAME = "codex-x86_64-unknown-linux-musl.tar.gz"
LINUX_MEMBER = "codex-x86_64-unknown-linux-musl"
LINUX_SIZE = 98970270
LINUX_SHA256 = "0246e2e773834e07f0fb5249ed6ebad12e4591e608f8c7bb97dd6a9690544c36"
MAX_LINUX_EXECUTABLE_SIZE = 512 * 1024 * 1024


def extract_linux_cli(archive, destination):
    regular_file(archive)
    with tarfile.open(archive, mode="r:gz") as source:
        members = [entry for entry in source.getmembers() if entry.name == LINUX_MEMBER]
        require(len(members) == 1, "固定 tar 包必须恰好包含一个目标可执行文件")
        member = members[0]
        require(member.isfile() and 0 < member.size <= MAX_LINUX_EXECUTABLE_SIZE,
                "目标必须是大小受限的普通文件，不能是链接或设备")
        # 不调用 extract/extractall；其它条目与成员路径都不能决定写入位置。
        descriptor, name = tempfile.mkstemp(prefix=".codex-executable-", dir=destination.parent)
        temporary = Path(name)
        try:
            with os.fdopen(descriptor, "wb") as output, source.extractfile(member) as data:
                digest = hashlib.sha256()
                count = 0
                while chunk := data.read(1024 * 1024):
                    count += len(chunk)
                    require(count <= member.size, "目标解包大小超过固定成员声明")
                    output.write(chunk)
                    digest.update(chunk)
                output.flush()
                os.fsync(output.fileno())
            require(count == member.size, "目标文件解包不完整")
            temporary.chmod(0o700)
            if destination.exists() or destination.is_symlink():
                require(sha256(destination) == digest.hexdigest(), "已有 Codex 与已验证归档内容不同，拒绝覆盖")
                # Windows 辅助测试只核对提取字节；真正 Linux 准备还必须保留执行位。
                require(os.name == "nt" or destination.stat().st_mode & stat.S_IXUSR,
                        "已有 Codex 不可执行，拒绝悄悄修改权限")
            else:
                # 原子创建且拒绝覆盖并发生成的路径；最终文件只来自已核验的固定归档。
                os.link(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)
    return destination


def verified_version(executable, runner_temp):
    allowed = {"PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "LANG", "LC_ALL"}
    environment = {key: value for key, value in os.environ.items() if key.upper() in allowed}
    with tempfile.TemporaryDirectory(prefix="codex-version-", dir=runner_temp) as temporary:
        root = Path(temporary)
        for key, relative in {"HOME": "home", "USERPROFILE": "home", "CODEX_HOME": "codex",
                              "APPDATA": "home/AppData/Roaming", "LOCALAPPDATA": "home/AppData/Local",
                              "TMPDIR": "tmp", "TEMP": "tmp", "TMP": "tmp"}.items():
            directory = root / relative
            directory.mkdir(parents=True, exist_ok=True)
            environment[key] = str(directory)
        (root / "codex/config.toml").write_text('cli_auth_credentials_store = "file"\n', encoding="utf-8")
        result = subprocess.run([str(executable), "--version"], cwd=root, env=environment,
                                capture_output=True, text=True, timeout=10, check=True)
    require(result.stdout.strip() == f"codex-cli {CODEX_VERSION}", "固定文件报告的 CLI 版本不匹配")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--download-dir", type=Path, required=True)
    args = parser.parse_args()
    machine = platform.machine().lower()
    architecture = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(machine)
    if sys.platform == "linux":
        require(architecture == "x86_64", "当前仅核实 Linux x86_64 官方归档")
    elif sys.platform == "win32":
        require(architecture in RELEASE_ASSETS, "当前 Windows 架构没有固定官方摘要")
    else:
        parser.error("此准备器只用于 Linux / Windows；macOS 使用已有受测 CLI 路径")
    require("RUNNER_TEMP" in os.environ, "必须显式提供 RUNNER_TEMP")
    runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    repository = Path(__file__).resolve().parents[2]
    directory = args.download_dir.resolve()
    require(directory.is_relative_to(runner_temp) and not directory.is_relative_to(repository),
            "下载目录必须位于 RUNNER_TEMP 且不能写入仓库")
    directory.mkdir(parents=True, exist_ok=True)
    if sys.platform == "linux":
        archive = directory / LINUX_NAME
        fetch_file(f"https://github.com/openai/codex/releases/download/rust-v{CODEX_VERSION}/{LINUX_NAME}",
                   archive, LINUX_SHA256, LINUX_SIZE)
        executable = extract_linux_cli(archive, directory / LINUX_MEMBER)
    else:
        name, size, digest = RELEASE_ASSETS[architecture]
        executable = directory / name
        fetch_file(f"https://github.com/openai/codex/releases/download/rust-v{CODEX_VERSION}/{name}",
                   executable, digest, size)
        verify_codex(executable, architecture)
    verified_version(executable, runner_temp)
    print(executable.resolve())


if __name__ == "__main__":
    main()
