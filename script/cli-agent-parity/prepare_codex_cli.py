#!/usr/bin/env python3
"""在 RUNNER_TEMP 中准备固定 Codex 0.147.0 完整运行包，只输出绝对入口路径。"""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

sys.dont_write_bytecode = True
from codex_windows_hook_inputs import CODEX_VERSION, fetch_file, regular_file, require, sha256


# 来自固定官方完整 package 的实际成员；路径、大小、模式和摘要都属于输入契约。
PACKAGES = {
    "linux-x64": {
        "archive": "codex-package-x86_64-unknown-linux-musl.tar.gz",
        "bytes": 119166922,
        "sha256": "bd758d53d56e41dc65e045f4589df79a038ed197a011adcb52a258e6ad64cfda",
        "target": "x86_64-unknown-linux-musl",
        "entrypoint": "bin/codex",
        "directories": {
            "bin": 0o755,
            "codex-path": 0o755,
            "codex-resources": 0o755,
            "codex-resources/zsh": 0o755,
            "codex-resources/zsh/bin": 0o755,
        },
        "files": {
            "bin/codex": (258278208, "cb0a15567e9a60a5820d54b0f6ae86d504dc3805c1eab21a47f70e3eb7b73a40", 0o755),
            "bin/codex-code-mode-host": (49682360, "00ecf5d040865b97884c488883abd342581c2a432debe7a54e4646bceee3d2d6", 0o755),
            "codex-package.json": (205, "00f66f11cc7d5c4133d500b4aae6ed4975608d6b040eefd56dc1ff343566e8cf", 0o644),
            "codex-path/rg": (5408904, "e62198eb19b136b88c330af83647b5a962cb99b6b1f066758568f12de1974849", 0o755),
            "codex-resources/bwrap": (529776, "77360cb751ccedc5971391444ac86a8a33c15b04d6b4a6fe45f5d25496e62c4c", 0o755),
            "codex-resources/zsh/bin/zsh": (898480, "67faaaa89242c4a332e16e508a1977cffc24bf7fca31d4411cdfd101f3831ef3", 0o755),
        },
    },
    "windows-x64": {
        "archive": "codex-package-x86_64-pc-windows-msvc.tar.gz",
        "bytes": 126777040,
        "sha256": "c156c8feb8cb20197bf74d2c6daffed1fec0a8c21a03bc2ca90d7ff81927b0c5",
        "target": "x86_64-pc-windows-msvc",
        "entrypoint": "bin/codex.exe",
        "directories": {
            "bin": 0o777,
            "codex-path": 0o777,
            "codex-resources": 0o777,
        },
        "files": {
            "bin/codex-code-mode-host.exe": (57450288, "37c23a542037e1bcfd0fa7eb4a150c697229d7ff31bf675c519d5bff7226b191", 0o777),
            "bin/codex.exe": (298668336, "935a1911ed2556e4ffcec995f4886ac2ac425863ba26fed264df62e30272ad9d", 0o777),
            "codex-package.json": (215, "ff573dd4c010f62fb40e3ed64c710622cf990b7b0a2be4bf20ff1d1f7b7c59b5", 0o666),
            "codex-path/rg.exe": (4218880, "14231169855ec5205cf5a1b6f1db358ff4aed4247c86b69ce8aae647c77f6680", 0o777),
            "codex-resources/codex-command-runner.exe": (1300272, "3a70491d8d588afa459a42816f05b8c2fdd6bddb0ef318f3dfccc963a30b420a", 0o777),
            "codex-resources/codex-windows-sandbox-setup.exe": (8804144, "a4df86996dfbb218d96d73a80606d89b742dfa4ddd3470614e90dde89e3250a3", 0o777),
        },
    },
    "windows-arm64": {
        "archive": "codex-package-aarch64-pc-windows-msvc.tar.gz",
        "bytes": 117548230,
        "sha256": "4533928d72ac4d7c19f16e8c4acdfd02dc255d2aeeb2f6d7dfd45493ec4c0806",
        "target": "aarch64-pc-windows-msvc",
        "entrypoint": "bin/codex.exe",
        "directories": {
            "bin": 0o777,
            "codex-path": 0o777,
            "codex-resources": 0o777,
        },
        "files": {
            "bin/codex-code-mode-host.exe": (54304560, "d322d6d721cf7f7ae523bfe31a504875611ec21bbf9b2bffca4b9fd30bdb1675", 0o777),
            "bin/codex.exe": (250102064, "1f0e8c2dd3c6b471e985fac76908366c1cf31155094fde606fb2d3052cf00584", 0o777),
            "codex-package.json": (216, "dc87fbce339d1f15852ea86c656ee0e45b543621405d4e24be7271279c3705d8", 0o666),
            "codex-path/rg.exe": (3859456, "d33a29a9ef03c9f4c03be9e8d88498e6e2d2e566d64cdbdef97f9afc8f13120c", 0o777),
            "codex-resources/codex-command-runner.exe": (1131312, "d7084b87834789ecf039423f83d855d2b9186287f89f19008dc793a0e9c91568", 0o777),
            "codex-resources/codex-windows-sandbox-setup.exe": (7758128, "45026d032a3fed97efe55fbc90c43848ce9434131e6e8c73a0167d7578ada897", 0o777),
        },
    },
}
MAX_MEMBERS = 32
MAX_FILE_BYTES = 512 * 1024 * 1024
MAX_TOTAL_BYTES = 512 * 1024 * 1024


def directory_without_links(path, *, private=False):
    information = path.lstat()
    # 仅认可 macOS 的 root 自有系统临时目录别名，不放宽任意缓存或运行包链接。
    if (sys.platform == "darwin" and path == Path("/tmp") and
            stat.S_ISLNK(information.st_mode) and information.st_uid == 0 and
            path.resolve(strict=True) == Path("/private/tmp")):
        path = Path("/private/tmp")
        information = path.lstat()
    require(stat.S_ISDIR(information.st_mode) and not path.is_symlink() and
            not (getattr(information, "st_file_attributes", 0) & 0x400),
            "运行时目录不能是链接、重解析点或其它文件类型")
    if os.name != "nt":
        owner = os.geteuid()
        mode = stat.S_IMODE(information.st_mode)
        if private:
            require(information.st_uid == owner and mode == 0o700,
                    "运行时和下载目录必须由当前用户拥有且权限为0700")
        else:
            # 系统祖先可属于 root；共享临时根必须有 sticky 位，保护已有的自有子目录。
            require(information.st_uid in (0, owner) and
                    (mode & 0o022 == 0 or mode & stat.S_ISVTX != 0),
                    "父目录不能由其他用户拥有或允许其他用户替换已有子目录")
    # Windows 的 mode 不能证明 ACL；此处只校验目录类型和重解析点。


def check_parent_chain(path):
    directory_without_links(path, private=True)
    for parent in path.parents:
        directory_without_links(parent)


def expected_metadata(package):
    return {"layoutVersion": 1, "version": CODEX_VERSION, "target": package["target"],
            "variant": "codex", "entrypoint": package["entrypoint"],
            "resourcesDir": "codex-resources", "pathDir": "codex-path"}


def verify_runtime_tree(directory, package):
    directory_without_links(directory, private=True)
    observed_files, observed_directories = set(), set()
    for root, directories, files in os.walk(directory, followlinks=False):
        for name in directories:
            path = Path(root) / name
            directory_without_links(path, private=True)
            observed_directories.add(path.relative_to(directory).as_posix())
        for name in files:
            path = Path(root) / name
            relative = path.relative_to(directory).as_posix()
            require(relative in package["files"], "已有运行时包含未知文件，拒绝覆盖")
            size, digest, mode = package["files"][relative]
            regular_file(path)
            require(path.stat().st_size == size and sha256(path) == digest,
                    f"运行时文件大小或摘要不匹配: {relative}")
            require(os.name == "nt" or stat.S_IMODE(path.stat().st_mode) == (mode & 0o700),
                    f"运行时文件权限不匹配: {relative}")
            observed_files.add(relative)
    require(observed_files == set(package["files"]) and
            observed_directories == set(package["directories"]), "运行时完整树存在缺失或额外条目")
    metadata = json.loads((directory / "codex-package.json").read_text(encoding="utf-8"))
    require(metadata == expected_metadata(package), "官方运行时布局元数据不匹配")
    return directory / package["entrypoint"]


def extract_runtime_package(archive, destination, target):
    package = PACKAGES[target]
    require(regular_file(archive).stat().st_size == package["bytes"] and
            sha256(archive) == package["sha256"], "完整官方归档大小或摘要不匹配")
    check_parent_chain(destination.parent)
    if destination.exists() or destination.is_symlink():
        return verify_runtime_tree(destination, package)
    # 同一个下载目录只允许一个准备进程发布，原目录不被清空或逐文件覆盖。
    lock = destination.with_name(destination.name + ".prepare-lock")
    lock.mkdir(mode=0o700)
    staging = None
    try:
        if destination.exists() or destination.is_symlink():
            return verify_runtime_tree(destination, package)
        staging = Path(tempfile.mkdtemp(prefix=".codex-package-", dir=destination.parent))
        seen, total = set(), 0
        with tarfile.open(archive, mode="r:gz") as source:
            for member in source:
                name = member.name
                parts = PurePosixPath(name).parts
                require(name and "\\" not in name and ":" not in name and
                        not name.startswith("/") and all(part not in ("", ".", "..") for part in name.split("/")) and
                        PurePosixPath(name).as_posix() == name,
                        "归档包含非规范或越界成员路径")
                require(name not in seen and len(seen) < MAX_MEMBERS, "归档成员重复或过多")
                seen.add(name)
                require(not member.linkname and not member.sparse, "归档不能包含链接或稀疏文件")
                path = staging.joinpath(*parts)
                if member.isdir():
                    require(name in package["directories"] and member.size == 0 and
                            member.mode == package["directories"][name], "归档目录不匹配固定布局")
                    path.mkdir(mode=0o700, parents=True, exist_ok=True)
                    continue
                require(member.isfile() and name in package["files"], "归档含未知文件、链接或设备")
                size, digest, mode = package["files"][name]
                total += member.size
                require(0 < member.size == size <= MAX_FILE_BYTES and total <= MAX_TOTAL_BYTES and
                        member.mode == mode, "归档文件大小或模式不匹配固定输入")
                path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                with source.extractfile(member) as data, path.open("xb") as output:
                    count, checksum = 0, hashlib.sha256()
                    while chunk := data.read(1024 * 1024):
                        count += len(chunk)
                        require(count <= size, "归档解包超过声明大小")
                        output.write(chunk)
                        checksum.update(chunk)
                    output.flush()
                    os.fsync(output.fileno())
                require(count == size and checksum.hexdigest() == digest, "归档成员内容不匹配固定摘要")
                # 保留所需读写/执行语义，只给当前用户权限，不复制官方包的组/其他写权限。
                path.chmod(mode & 0o700)
        require(seen == set(package["files"]) | set(package["directories"]), "完整归档有缺失成员")
        verify_runtime_tree(staging, package)
        require(not destination.exists() and not destination.is_symlink(), "发布目标在解包期间发生变化")
        staging.rename(destination)
        staging = None
        return verify_runtime_tree(destination, package)
    finally:
        original_error = sys.exc_info()[1]
        cleanup_errors = []
        if staging is not None:
            try:
                shutil.rmtree(staging)
            except OSError as error:
                cleanup_errors.append(error)
        try:
            lock.rmdir()
        except OSError as error:
            cleanup_errors.append(error)
        if cleanup_errors:
            # 清理失败须保留，但不能把原来的校验或文件事务错误替换成次生异常。
            if original_error is not None:
                for error in cleanup_errors:
                    original_error.add_note(f"运行包准备清理失败: {error}")
            else:
                raise cleanup_errors[0]


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
        target = "linux-x64"
    elif sys.platform == "win32":
        require(architecture in ("x86_64", "aarch64"), "当前 Windows 架构没有固定官方摘要")
        target = "windows-x64" if architecture == "x86_64" else "windows-arm64"
    else:
        parser.error("此准备器只用于 Linux / Windows；macOS 使用已有受测 CLI 路径")
    require("RUNNER_TEMP" in os.environ, "必须显式提供 RUNNER_TEMP")
    runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    repository = Path(__file__).resolve().parents[2]
    unresolved = args.download_dir.absolute()
    for parent in [unresolved, *unresolved.parents]:
        if parent.exists() or parent.is_symlink():
            directory_without_links(parent)
    directory = unresolved.resolve()
    require(directory.is_relative_to(runner_temp) and not directory.is_relative_to(repository),
            "下载目录必须位于 RUNNER_TEMP 且不能写入仓库")
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    check_parent_chain(directory)
    package = PACKAGES[target]
    name = package["archive"]
    archive = directory / name
    fetch_file(f"https://github.com/openai/codex/releases/download/rust-v{CODEX_VERSION}/{name}",
               archive, package["sha256"], package["bytes"])
    executable = extract_runtime_package(archive, directory / f"runtime-{CODEX_VERSION}-{target}", target)
    verified_version(executable, runner_temp)
    print(executable.resolve())


if __name__ == "__main__":
    main()
