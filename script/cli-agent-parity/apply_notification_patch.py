#!/usr/bin/env python3
"""向本机或 SSH 目标已安装的受测插件应用随附修补；只使用 Python 3.11+ 标准库。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib


CONTRACTS = {
    "claude": ("2.1.273 (Claude Code)", "2.2.0", "claude-code-warp", ".claude-plugin/plugin.json", "CLAUDE_CONFIG_DIR"),
    "codex": ("codex-cli 0.147.0", "0.4.0", "codex-warp", ".codex-plugin/plugin.json", "CODEX_HOME"),
}
FILES = {
    "claude": ("scripts/build-payload.sh", "scripts/on-stop.sh", "scripts/should-use-structured.sh", "hooks/hooks.json", "scripts/warp-notify.sh"),
    "codex": ("scripts/build-payload.sh", "scripts/on-stop.sh", "hooks/hooks.json", "scripts/warp-notify.sh", "scripts/on-prompt-submit.sh"),
}


def digest(contents):
    return hashlib.sha256(contents).hexdigest()


def default_bundle():
    script = Path(__file__).resolve()
    if (script.parent / "claude/PATCH_METADATA.json").is_file():
        return script.parent
    return script.parents[2] / "app/assets/bundled/cli-agent-plugins"


def bundle_data(bundle, agent):
    root = bundle / agent
    metadata = json.loads((root / "PATCH_METADATA.json").read_text())
    if set(metadata["files"]) != set(FILES[agent]):
        raise ValueError("随附文件清单不符合此版本的受控范围")
    replacements = {name: (root / name).read_bytes() for name in FILES[agent]}
    for name, contents in replacements.items():
        if digest(contents) != metadata["files"][name]["replacement_sha256"]:
            raise ValueError("随附脚本摘要不匹配")
    return metadata, replacements


def checked_file(root, name):
    relative = Path(name)
    if relative.is_absolute() or any(part in (".", "..") for part in relative.parts):
        raise ValueError("插件文件路径越界")
    path = root
    for part in relative.parts:
        path = path / part
        if path.is_symlink():
            raise ValueError("拒绝替换符号链接")
    information = path.stat()
    if not stat.S_ISREG(information.st_mode) or information.st_nlink != 1:
        raise ValueError("拒绝替换非普通文件或硬链接")
    return path


def installation(home, agent, bundle=None):
    _, expected_version, marketplace, manifest_name, _ = CONTRACTS[agent]
    base = home / "plugins/cache" / marketplace / "warp"
    if agent == "claude":
        registry = json.loads((home / "plugins/installed_plugins.json").read_text())
        entries = registry.get("plugins", {}).get("warp@claude-code-warp", [])
        if len(entries) != 1 or entries[0].get("scope") != "user":
            raise ValueError("只支持唯一的用户级插件安装记录")
        root = Path(entries[0]["installPath"])
        version = entries[0]["version"]
        settings_path = home / "settings.json"
        settings = json.loads(settings_path.read_text()) if settings_path.exists() else {}
        if settings.get("enabledPlugins", {}).get("warp@claude-code-warp") is False:
            raise ValueError("插件已被用户禁用，保持禁用状态")
        source = settings.get("extraKnownMarketplaces", {}).get(marketplace, {}).get("source", {})
        if isinstance(source, str) or (isinstance(source, dict) and source.get("source") == "directory"):
            raise ValueError("自定义 marketplace 不自动替换")
    else:
        settings = tomllib.loads((home / "config.toml").read_text())
        if settings.get("plugins", {}).get("warp@codex-warp", {}).get("enabled") is not True:
            raise ValueError("插件未启用，保持原配置")
        source = settings.get("marketplaces", {}).get(marketplace, {})
        if source and source.get("source_type") != "git":
            from codex_persistent_source import validate_source
            validate_source(home.resolve(), settings, bundle or default_bundle())
        entries = list(base.iterdir())
        if len(entries) != 1:
            raise ValueError("存在零个或多个插件缓存，无法确认活跃安装")
        root = entries[0]
        version = json.loads(checked_file(root, manifest_name).read_text())["version"]
    if root.is_symlink() or root.parent.resolve() != base.resolve() or not root.resolve().is_relative_to(home.resolve()):
        raise ValueError("插件路径不属于此 CLI 的受控缓存")
    manifest = json.loads(checked_file(root, manifest_name).read_text())
    if root.name != version or version != expected_version or manifest.get("version") != version or manifest.get("name") != "warp":
        raise ValueError("插件版本不符合受测版本；请先通过原生 CLI 更新")
    return root, version


def validate_tree(root, version, metadata):
    base = next((base for base in metadata["compatible_bases"] if base["version"] == version), None)
    if base is None:
        raise ValueError("插件版本未经验证")
    expected = base["tree_sha256"]
    observed = set()
    for directory, directories, files in os.walk(root, followlinks=False):
        for name in directories:
            if (Path(directory) / name).is_symlink():
                raise ValueError("插件目录含符号链接")
        for name in files:
            path = Path(directory) / name
            relative = path.relative_to(root).as_posix()
            if relative not in expected:
                raise ValueError("插件存在自定义文件，未执行替换")
            actual = digest(checked_file(root, relative).read_bytes())
            replacement = metadata["files"].get(relative, {}).get("replacement_sha256")
            if actual not in (expected[relative], replacement):
                raise ValueError("插件文件存在自定义修改，未执行替换")
            observed.add(relative)
    if observed != set(expected):
        raise ValueError("插件文件不完整")


def atomic_write(path, contents, mode):
    descriptor, temporary_name = tempfile.mkstemp(prefix=".infinishell-patch-", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            if os.name == "posix":
                os.fchmod(output.fileno(), mode)
            else:
                # Windows 先关闭写句柄再替换目标，chmod 仅保留可读写属性。
                os.chmod(temporary, mode)
            output.write(contents)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def apply_files(root, metadata, replacements):
    originals = []
    for name, contents in replacements.items():
        path = checked_file(root, name)
        original = path.read_bytes()
        hashes = metadata["files"][name]
        if digest(original) not in (hashes["upstream_sha256"], hashes["replacement_sha256"]) or digest(contents) != hashes["replacement_sha256"]:
            raise ValueError("摘要不匹配，未执行替换")
        originals.append((name, path, original, stat.S_IMODE(path.stat().st_mode)))
    replaced = []
    try:
        for name, path, original, mode in originals:
            if checked_file(root, name).read_bytes() != original:
                raise ValueError("预检后检测到并发编辑")
            atomic_write(path, replacements[name], mode)
            replaced.append((name, path, original, mode))
        for name, path, _, _ in originals:
            if digest(checked_file(root, name).read_bytes()) != metadata["files"][name]["replacement_sha256"]:
                raise ValueError("写入后摘要校验失败")
    except Exception as original_error:
        failures = []
        for name, path, original, mode in reversed(replaced):
            try:
                if checked_file(root, name).read_bytes() != replacements[name]:
                    raise ValueError("文件被并发修改，拒绝覆盖")
                atomic_write(path, original, mode)
            except Exception:
                failures.append(name)
        if failures:
            raise ValueError("部分文件恢复失败，需重新校验：" + ", ".join(failures)) from original_error
        raise


def verify_runtime(agent, home, cli, files_only=False):
    if os.name != "posix" and not files_only:
        raise ValueError("尚未验证 Windows/Git Bash 通知运行环境，拒绝应用修补")
    environment = os.environ.copy()
    environment[CONTRACTS[agent][4]] = str(home)
    versions = {}
    executables = [(agent, cli)]
    if not files_only:
        executables.extend([("bash", "bash"), ("jq", "jq")])
    for name, executable in executables:
        result = subprocess.run([executable, "--version"], env=environment, capture_output=True, text=True, timeout=5, check=True)
        versions[name] = result.stdout.strip()
    if versions[agent] != CONTRACTS[agent][0]:
        raise ValueError("CLI 版本不是此修补的已验证版本")
    return versions[agent]


def export_bundle(source, destination):
    destination.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, destination / "apply_notification_patch.py")
    shutil.copy2(Path(__file__).with_name("codex_persistent_source.py"), destination / "codex_persistent_source.py")
    from codex_persistent_source import source_bundle
    source_bundle(source)
    shutil.copytree(source / "codex/source", destination / "codex/source")
    shutil.copy2(source / "codex/SOURCE_METADATA.json", destination / "codex/SOURCE_METADATA.json")
    shutil.copytree(source / "codex/revisions", destination / "codex/revisions")
    for agent in CONTRACTS:
        bundle_data(source, agent)
        for name in (*FILES[agent], "PATCH_METADATA.json", "LICENSE", "README.md", "upstream.patch"):
            target = destination / agent / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / agent / name, target)
    manifest = {path.relative_to(destination).as_posix(): digest(path.read_bytes()) for path in destination.rglob("*") if path.is_file()}
    (destination / "SHA256SUMS.json").write_text(json.dumps(manifest, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", choices=CONTRACTS)
    parser.add_argument("--cli-home", type=Path)
    parser.add_argument("--cli")
    parser.add_argument("--bundle-dir", type=Path, default=default_bundle())
    parser.add_argument("--check", action="store_true", help="只核对，不写入插件")
    parser.add_argument("--files-only", action="store_true", help="只验证 CLI 版本及文件事务；Windows 可用，但不代表原生 hooks 已可运行")
    parser.add_argument("--export", type=Path, help="输出可传输至 SSH 目标的独立目录")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    if args.export:
        export_bundle(args.bundle_dir, args.export)
        print("已导出修补包及逐文件 SHA-256 清单。")
        return
    if not args.agent:
        parser.error("必须指定 --agent 或 --export")
    home = args.cli_home or Path(os.environ.get(CONTRACTS[args.agent][4]) or Path.home() / ("." + args.agent))
    if args.report and args.report.resolve().is_relative_to(home.resolve()):
        parser.error("验证报告不能写入 CLI 的配置或插件目录")
    version = verify_runtime(args.agent, home, args.cli or args.agent, args.files_only)
    metadata, replacements = bundle_data(args.bundle_dir, args.agent)
    if args.agent == "codex":
        from codex_persistent_source import install_source
        install_source(home, args.cli or args.agent, args.bundle_dir, check=args.check)
    root, plugin_version = installation(home, args.agent, args.bundle_dir)
    validate_tree(root, plugin_version, metadata)
    if not args.check and args.agent != "codex":
        apply_files(root, metadata, replacements)
    hashes = {name: digest(checked_file(root, name).read_bytes()) for name in replacements}
    verified = all(value == metadata["files"][name]["replacement_sha256"] for name, value in hashes.items())
    report = {"agent": args.agent, "cli_version": version, "plugin_version": plugin_version, "mode": "check" if args.check else "apply", "patch_verified": verified, "sha256": hashes, "persistent_source_verified": args.agent == "codex", "native_hook_lifecycle_verified": False, "host_os": sys.platform, "files_only": args.files_only, "bash_jq_probed": not args.files_only, "native_notifications_verified": False}
    if args.report:
        args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report, ensure_ascii=False))
    if not verified:
        raise ValueError("尚未应用完整关联修补")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"通知修补失败：{error}", file=sys.stderr)
        sys.exit(1)
