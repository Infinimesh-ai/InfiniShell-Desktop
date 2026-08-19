#!/usr/bin/env python3
"""创建、校验或复制安装包内置的 remote-server 资源。"""

import argparse
import hashlib
import json
import shutil
from pathlib import Path


ARTIFACTS = (
    ("linux", "x86_64", "infinishell-linux-x86_64.tar.gz"),
    ("linux", "aarch64", "infinishell-linux-aarch64.tar.gz"),
    ("macos", "x86_64", "infinishell-macos-x86_64.tar.gz"),
    ("macos", "aarch64", "infinishell-macos-aarch64.tar.gz"),
    ("windows", "x86_64", "infinishell-windows-x86_64.zip"),
    ("windows", "aarch64", "infinishell-windows-aarch64.zip"),
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def expected_records(
    root: Path,
    artifacts: tuple[tuple[str, str, str], ...] = ARTIFACTS,
) -> list[dict[str, str]]:
    records = []
    for os_name, arch_name, file_name in artifacts:
        artifact_path = root / file_name
        if not artifact_path.is_file() or artifact_path.stat().st_size == 0:
            raise ValueError(f"缺少 remote-server 产物或文件为空: {artifact_path}")
        records.append(
            {
                "os": os_name,
                "arch": arch_name,
                "file": file_name,
                "sha256": sha256(artifact_path),
            }
        )
    return records


def create(
    source: Path,
    destination: Path,
    version: str,
    allow_partial: bool = False,
) -> None:
    artifacts = ARTIFACTS
    if allow_partial:
        artifacts = tuple(
            artifact for artifact in ARTIFACTS if (source / artifact[2]).is_file()
        )
        if not artifacts:
            raise ValueError(f"没有可打包的 remote-server 产物: {source}")
    records = expected_records(source, artifacts)
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    for record in records:
        shutil.copy2(source / record["file"], destination / record["file"])
    manifest = {"version": version, "artifacts": records}
    (destination / "manifest.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def verify(source: Path, version: str, allow_partial: bool = False) -> None:
    manifest_path = source / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"无法读取 remote-server 清单 {manifest_path}: {error}") from error

    if manifest.get("version") != version:
        raise ValueError(
            f"remote-server 清单版本不匹配: 期望 {version}, 实际 {manifest.get('version')}"
        )

    artifacts = ARTIFACTS
    if allow_partial:
        manifest_artifacts = manifest.get("artifacts")
        if not isinstance(manifest_artifacts, list) or not manifest_artifacts:
            raise ValueError("partial remote-server 清单必须至少包含一个产物")
        allowed = {
            (os_name, arch_name): (os_name, arch_name, file_name)
            for os_name, arch_name, file_name in ARTIFACTS
        }
        selected = []
        seen = set()
        for record in manifest_artifacts:
            if not isinstance(record, dict):
                raise ValueError("partial remote-server 清单包含无效产物记录")
            key = (record.get("os"), record.get("arch"))
            if key in seen:
                raise ValueError(f"partial remote-server 清单包含重复平台: {key}")
            seen.add(key)
            artifact = allowed.get(key)
            if artifact is None or record.get("file") != artifact[2]:
                raise ValueError(f"partial remote-server 清单包含未知产物: {record}")
            selected.append(artifact)
        artifacts = tuple(selected)
    expected = expected_records(source, artifacts)
    if manifest.get("artifacts") != expected:
        raise ValueError("remote-server 清单内容或 SHA-256 校验值不匹配")


def copy(
    source: Path,
    destination: Path,
    version: str,
    allow_partial: bool = False,
) -> None:
    verify(source, version, allow_partial)
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(source, destination)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("create", "create-partial", "verify", "copy"))
    parser.add_argument("--allow-partial", action="store_true")
    parser.add_argument("source", type=Path)
    parser.add_argument("version")
    parser.add_argument("destination", type=Path, nargs="?")
    args = parser.parse_args()

    allow_partial = args.allow_partial or args.mode == "create-partial"
    if args.mode == "verify":
        verify(args.source, args.version, allow_partial)
        return
    if args.destination is None:
        parser.error(f"{args.mode} 模式需要 destination")
    if args.mode in ("create", "create-partial"):
        create(args.source, args.destination, args.version, allow_partial)
    else:
        copy(args.source, args.destination, args.version, allow_partial)


if __name__ == "__main__":
    main()
