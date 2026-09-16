#!/usr/bin/env python3
"""只读挂载并验收 macOS Intel DMG；取证文件留在新的隔离目录。"""

import argparse
import hashlib
import json
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(*arguments):
    result = subprocess.run(list(map(str, arguments)), capture_output=True, check=True)
    return result.stdout


def sha256(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validate_resources(resources, ci_resources, tag, manifest_sha256):
    manifest_path = resources / "manifest.json"
    ci_manifest_path = ci_resources / "manifest.json"
    require(sha256(manifest_path) == manifest_sha256, "内嵌 manifest 不匹配 CI 原始摘要")
    require(sha256(ci_manifest_path) == manifest_sha256, "传入的 CI 原始资源清单摘要错误")
    data = json.loads(manifest_path.read_text())
    require(data["version"] == tag and len(data["artifacts"]) == 6, "manifest 版本或件数错误")
    expected_pairs = {
        (os_name, arch)
        for os_name in ("linux", "macos", "windows")
        for arch in ("x86_64", "aarch64")
    }
    require(
        {(entry["os"], entry["arch"]) for entry in data["artifacts"]} == expected_pairs,
        "manifest 六架构集合错误",
    )
    for entry in data["artifacts"]:
        suffix = "zip" if entry["os"] == "windows" else "tar.gz"
        expected_file = f'infinishell-{entry["os"]}-{entry["arch"]}.{suffix}'
        require(entry["file"] == expected_file, "manifest 文件名错误")
        embedded = resources / expected_file
        original = ci_resources / expected_file
        require(sha256(embedded) == entry["sha256"], f"内嵌归档摘要错误：{expected_file}")
        require(sha256(embedded) == sha256(original), f"归档与 CI 原始资源不一致：{expected_file}")
    return data["artifacts"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--dmg", type=Path, required=True)
    parser.add_argument("--ci-resources", type=Path, required=True)
    parser.add_argument("--isolation", type=Path, required=True)
    parser.add_argument(
        "--manifest-sha256", required=True, help="CI 原始 manifest 摘要，不使用本机重签后的清单"
    )
    parser.add_argument("--expected-signature", choices=("adhoc", "developer-id"), default="adhoc")
    args = parser.parse_args()

    tag_match = re.fullmatch(
        r"v(0)\.(\d{4})\.(\d{2})\.(\d{2})\.(\d{2})\.(\d{2})\.oss_(\d{2})",
        args.tag,
    )
    require(tag_match is not None, "Tag 格式错误")
    require(re.fullmatch(r"[a-fA-F0-9]{64}", args.manifest_sha256), "manifest 摘要格式错误")
    require(platform.system() == "Darwin", "需要 macOS 原生环境")
    for tool in ("hdiutil", "lipo", "codesign", "arch"):
        require(shutil.which(tool), f"缺少工具：{tool}")
    dmg = args.dmg.resolve(strict=True)
    ci_resources = args.ci_resources.resolve(strict=True)
    require(dmg.is_file() and ci_resources.is_dir(), "DMG 或 CI 原始资源目录不存在")
    isolation = args.isolation.absolute()
    require(not isolation.exists(), "隔离目录必须尚不存在")
    manifest_sha256 = args.manifest_sha256.lower()
    require(sha256(ci_resources / "manifest.json") == manifest_sha256, "CI 原始 manifest 摘要错误")
    expected_version = ".".join(tag_match.groups())

    isolation.mkdir(parents=True)
    mount = isolation / "mount"
    mount.mkdir()
    device = None
    attached = False
    result = None
    run("hdiutil", "verify", dmg)
    try:
        attached_plist = run(
            "hdiutil", "attach", "-readonly", "-nobrowse", "-mountpoint", mount, "-plist", dmg
        )
        attached = True
        (isolation / "mount.plist").write_bytes(attached_plist)
        entities = plistlib.loads(attached_plist)["system-entities"]
        device = next(entry["dev-entry"] for entry in entities if "mount-point" in entry)
        app = mount / "InfiniShell.app"
        info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
        require(info["WarpVersion"] == args.tag, "WarpVersion 与 Tag 不一致")
        require(info["CFBundleIdentifier"] == "dev.infinishell.InfiniShell", "Bundle ID 错误")
        require(
            info["CFBundleVersion"] == info["CFBundleShortVersionString"] == expected_version,
            "Bundle 版本错误",
        )
        binary = app / "Contents/MacOS" / info["CFBundleExecutable"]
        require(run("lipo", "-archs", binary).decode().strip() == "x86_64", "主程序架构错误")
        run("codesign", "--verify", "--deep", "--strict", "--verbose=4", app)
        signature = subprocess.run(
            ["codesign", "-dv", "--verbose=4", str(app)], capture_output=True, check=True
        ).stderr.decode(errors="replace")
        if args.expected_signature == "adhoc":
            require("Signature=adhoc" in signature, "Intel App 未采用预期的 ad-hoc 签名")
        else:
            require("Authority=Developer ID Application:" in signature, "缺少 Developer ID 签名")
        require("runtime" in signature, "签名缺少 hardened runtime")
        version = run("arch", "-x86_64", binary, "--version").decode().strip()
        require(args.tag in version.split(), f"实际版本错误：{version}")
        resources = app / "Contents/Resources/remote-server"
        artifacts = validate_resources(resources, ci_resources, args.tag, manifest_sha256)
        result = {
            "tag": args.tag,
            "dmg_sha256": sha256(dmg),
            "binary_sha256": sha256(binary),
            "version": version,
            "architecture": "x86_64",
            "signature": signature,
            "manifest_sha256": sha256(resources / "manifest.json"),
            "embedded_artifacts": artifacts,
            "plist": info,
        }
    finally:
        if attached:
            run("hdiutil", "detach", device or mount)
        mount.rmdir()
    require(result is not None, "验收结果缺失")
    (isolation / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print("Intel DMG 验收通过", result["dmg_sha256"], flush=True)


if __name__ == "__main__":
    main()
