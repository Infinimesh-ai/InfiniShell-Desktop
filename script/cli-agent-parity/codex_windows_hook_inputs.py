"""固定 Windows Codex 和原始插件的获取与摘要核验；不会安装插件或写入用户配置。"""

import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
import urllib.request


CODEX_VERSION = "0.147.0"
CODEX_COMMIT = "be6e8eac029b183056b7e4402879f15d2c85f61b"
PLUGIN_COMMIT = "31ce59d9011cfb1d78f265649a228dac5de58d76"
# 摘要来自固定 release 的官方 API digest；不查询 latest，也不接受调用方覆盖摘要。
RELEASE_ASSETS = {
    "x86_64": ("codex-x86_64-pc-windows-msvc.exe", 298668336,
               "935a1911ed2556e4ffcec995f4886ac2ac425863ba26fed264df62e30272ad9d"),
    "aarch64": ("codex-aarch64-pc-windows-msvc.exe", 250102064,
                "1f0e8c2dd3c6b471e985fac76908366c1cf31155094fde606fb2d3052cf00584"),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def regular_file(path):
    information = path.lstat()
    require(not path.is_symlink() and not (getattr(information, "st_file_attributes", 0) & 0x400),
            "拒绝符号链接或 Windows 重解析点")
    require(stat.S_ISREG(information.st_mode) and information.st_nlink == 1,
            "只接受无硬链接的普通文件")
    return path


def sha256(path):
    with regular_file(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def plugin_base(repo):
    metadata = json.loads((repo / "app/assets/bundled/cli-agent-plugins/codex/PATCH_METADATA.json")
                          .read_text(encoding="utf-8"))
    require(metadata["upstream_repository"] == "https://github.com/warpdotdev/codex-warp",
            "上游仓库与固定探测契约不匹配")
    matches = [base for base in metadata["compatible_bases"]
               if base["commit"] == PLUGIN_COMMIT and base["version"] == "0.4.0"]
    require(len(matches) == 1, "缺少固定的原始插件摘要清单")
    return matches[0]


def verify_plugin(root, base):
    require(root.is_dir() and not root.is_symlink(), "原始插件目录不存在或为链接")
    observed = {}
    for directory, directories, files in os.walk(root, followlinks=False):
        for name in [".", *directories]:
            path = Path(directory) / name
            information = path.lstat()
            require(not path.is_symlink() and not (getattr(information, "st_file_attributes", 0) & 0x400),
                    "原始插件目录包含链接或重解析点")
        for name in files:
            path = Path(directory) / name
            observed[path.relative_to(root).as_posix()] = sha256(path)
    require(observed == base["tree_sha256"], "原始插件全树摘要不匹配；不接受已修补、自定义或缺失文件")
    return observed


def verify_codex(path, architecture):
    name, size, digest = RELEASE_ASSETS[architecture]
    require(regular_file(path).stat().st_size == size and sha256(path) == digest,
            "Codex 可执行文件不匹配固定官方 release 摘要")
    return {"asset": name, "bytes": size, "sha256": digest,
            "url": f"https://github.com/openai/codex/releases/download/rust-v{CODEX_VERSION}/{name}"}


def fetch_file(url, destination, digest, size=None):
    require(url.startswith(("https://github.com/openai/codex/releases/download/rust-v0.147.0/",
                            f"https://raw.githubusercontent.com/warpdotdev/codex-warp/{PLUGIN_COMMIT}/")),
            "只允许固定官方发布或固定上游提交 URL")
    if destination.exists():
        require(sha256(destination) == digest and (size is None or destination.stat().st_size == size),
                "已存在的下载文件摘要不匹配，拒绝覆盖")
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(prefix=".download-", dir=destination.parent)
    temporary = Path(name)
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "InfiniShell-fixed-hook-verification"})
        with os.fdopen(descriptor, "wb") as output, urllib.request.urlopen(request, timeout=60) as response:
            count = 0
            while chunk := response.read(1024 * 1024):
                count += len(chunk)
                require(count <= (size if size is not None else 1024 * 1024), "下载内容超过固定大小限制")
                output.write(chunk)
        require(sha256(temporary) == digest and (size is None or count == size), "下载摘要或大小不匹配")
        # 链接创建拒绝覆盖并发生成的文件；下载缓存放在同一临时文件系统。
        os.link(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def obtain_inputs(repo, directory, architecture, executable=None, plugin=None):
    base = plugin_base(repo)
    if executable is None:
        require(directory is not None, "没有提供 Codex 文件或固定下载目录")
        name, size, digest = RELEASE_ASSETS[architecture]
        executable = directory / name
        fetch_file(f"https://github.com/openai/codex/releases/download/rust-v{CODEX_VERSION}/{name}",
                   executable, digest, size)
    cli_evidence = verify_codex(executable, architecture)
    if plugin is None:
        require(directory is not None, "没有提供原始插件或固定下载目录")
        plugin = directory / ("codex-warp-" + PLUGIN_COMMIT) / "plugins/warp"
        for name, digest in base["tree_sha256"].items():
            require(not Path(name).is_absolute() and ".." not in Path(name).parts, "固定插件路径越界")
            fetch_file(f"https://raw.githubusercontent.com/warpdotdev/codex-warp/{PLUGIN_COMMIT}/plugins/warp/{name}",
                       plugin / name, digest)
    plugin_evidence = verify_plugin(plugin, base)
    return executable, plugin, {"codex": cli_evidence, "plugin_commit": PLUGIN_COMMIT,
                                "plugin_tree_sha256": plugin_evidence}
