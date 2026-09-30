"""从固定公开基线和受审补丁构建普通 Grok 桥；仅由现有跨平台 workflow 调用。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time

REPOSITORY = Path(__file__).resolve().parents[2]
SOURCE_METADATA = REPOSITORY / "native/grok-build/source.json"
PATCH = REPOSITORY / "native/grok-build/terminal-bridge.patch"
UPSTREAM = "https://github.com/xai-org/grok-build"
BASE = "07e35a3dfeed2f200d319ef6c893b5ea286d9a51"
TOOLCHAIN = "1.94.0"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--target-dir", required=True, type=Path)
    args = parser.parse_args()
    system = platform.system()
    if system not in ("Linux", "Windows") or platform.machine().lower() not in ("x86_64", "amd64"):
        raise RuntimeError("只允许本机 Linux/Windows x64 工具链构建")
    metadata = json.loads(SOURCE_METADATA.read_text(encoding="utf-8"))
    build = metadata["cross_platform_build"]
    if metadata["upstream"] != UPSTREAM or metadata["base_commit"] != BASE:
        raise RuntimeError("公开源码基线不匹配")
    patch_digest = digest(PATCH)
    if patch_digest != build["patch_sha256"] or not build["native_tree"]:
        raise RuntimeError("补丁摘要或预期源码树未冻结")
    if build["custom_version"] != "1.0.41+infinishell.terminal-bridge.9":
        raise RuntimeError("定制构建版本不匹配")
    output = args.output.absolute()
    output.mkdir(parents=True, exist_ok=False)
    source = output / "source"
    source.mkdir()
    artifacts = output / "artifacts"
    artifacts.mkdir()
    target = args.target_dir.absolute()
    target.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ, CARGO_TARGET_DIR=str(target),
                       GROK_VERSION=build["custom_version"], CARGO_TERM_COLOR="never")
    receipt = {"schema_version": 1, "status": "running", "platform": system,
               "kernel_release": platform.release(),
               "host_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPOSITORY, text=True).strip(),
               "upstream": UPSTREAM, "base_commit": BASE, "native_tree": build["native_tree"],
               "patch_sha256": patch_digest, "metadata_sha256": digest(SOURCE_METADATA),
               "custom_version": build["custom_version"], "toolchain": TOOLCHAIN,
               "commands": [], "model_inputs": 0,
               "boundary": "公开源码构建及定向库回归；不代表模型或交互式桌面验收"}
    receipt_path = artifacts / "build.safe.json"

    def save():
        receipt_path.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def run(command, capture=False):
        index = len(receipt["commands"]) + 1
        log = artifacts / f"step-{index}.log"
        started = time.monotonic()
        with log.open("wb") as stream:
            result = subprocess.run(command, cwd=source, env=environment, stdout=stream, stderr=subprocess.STDOUT)
        receipt["commands"].append({"command": command, "exit_code": result.returncode,
                                    "elapsed_seconds": round(time.monotonic() - started, 3),
                                    "log": log.name, "log_sha256": digest(log)})
        save()
        print(json.dumps(receipt["commands"][-1]), flush=True)
        if result.returncode:
            raise RuntimeError(f"源码门禁第 {index} 步失败，见 {log.name}")
        return log.read_text(encoding="utf-8").strip() if capture else None

    save()
    try:
        run(["git", "init"])
        run(["git", "config", "core.autocrlf", "false"])
        run(["git", "remote", "add", "origin", UPSTREAM + ".git"])
        run(["git", "-c", "credential.helper=", "fetch", "--depth=1", "origin", BASE])
        run(["git", "checkout", "--detach", "FETCH_HEAD"])
        if run(["git", "rev-parse", "HEAD"], True) != BASE:
            raise RuntimeError("下载的公开基线不是固定提交")
        run(["git", "apply", "--index", str(PATCH)])
        if run(["git", "write-tree"], True) != build["native_tree"]:
            raise RuntimeError("补丁重建源码树不匹配")
        run(["rustup", "toolchain", "install", TOOLCHAIN, "--profile", "minimal", "--no-self-update"])
        run(["cargo", "+" + TOOLCHAIN, "test", "--locked", "-p", "xai-proto-build", "--lib"])
        run(["cargo", "+" + TOOLCHAIN, "test", "--locked", "-p", "xai-grok-tools", "--lib",
             "test_parse_login_env_capture", "--", "--test-threads=1"])
        asset_tests = ["cargo", "+" + TOOLCHAIN, "test", "--locked", "-p", "xai-grok-shell", "--lib",
                       "retain_session_asset_files_tests", "--", "--test-threads=1"]
        if system == "Windows":
            # 用户已将外平台实机验收后置；此额外资源测试缺少符号链接特权，保留失败而不重跑拒绝。
            fixture = "crates/codegen/xai-grok-shell/src/session/helpers/session_compact_retain_session_asset_files_tests.rs"
            fixture_sha = "abb12e06f3d233c794c054d6b80e434efe2f926fd46b29c8eb7b9070e728717e"
            if digest(source / fixture) != fixture_sha:
                raise RuntimeError("Windows 符号链接测试源码已变化，必须重新审计验收范围")
            deferred = "session::helpers::session_compact::retain_session_asset_files_tests::keeps_only_regular_files_inside_the_assets_dir"
            asset_tests += ["--skip", deferred]
            receipt["deferred_environment_checks"] = [{
                "test": deferred, "source_sha256": fixture_sha, "status": "deferred_not_passed",
                "prior_run_id": 36766846873, "win32_error": 1314,
                "prior_log_sha256": "e9f551f19e5a7f23ba20d0cd025281f314c8263c72d7302c52890f1e4b90f2b2",
                "scope": "额外资源整理测试；Mac三项已通过。Windows真实符号链接验证移交后续环境，不属于G01文本原子桥验收。",
            }]
            save()
        run(asset_tests)
        if system == "Windows":
            run(["cargo", "+" + TOOLCHAIN, "test", "--locked", "-p", "xai-grok-shell", "--lib",
                 "leader::transport::windows_impl::tests::pipe_name_is_bounded", "--", "--exact"])
        run(["cargo", "+" + TOOLCHAIN, "check", "--locked", "-p", "xai-grok-pager-bin"])
        run(["cargo", "+" + TOOLCHAIN, "test", "--locked", "-p", "xai-grok-pager", "-p", "xai-grok-shell",
             "-p", "xai-grok-tools", "-p", "xai-grok-shell-terminal", "--lib", "terminal_bridge", "--", "--test-threads=1"])
        run(["cargo", "+" + TOOLCHAIN, "build", "--locked", "-p", "xai-grok-pager-bin"])
        if run(["git", "write-tree"], True) != build["native_tree"]:
            raise RuntimeError("构建期间源码索引改变")
        run(["git", "diff", "--exit-code"])
        executable = target / "debug" / ("xai-grok-pager.exe" if system == "Windows" else "xai-grok-pager")
        destination = artifacts / executable.name
        shutil.copy2(executable, destination)
        receipt["binary"] = {"file": destination.name, "bytes": destination.stat().st_size,
                             "sha256": digest(destination)}
        # --version 不初始化用户会话；结果保留真实公开基线 stamp，补丁源码树另有独立核验。
        receipt["version_output"] = run([str(destination), "--version"], True)
        if build["custom_version"] not in receipt["version_output"]:
            raise RuntimeError("定制版本标记缺失")
        receipt["status"] = "passed"
    except Exception as error:
        receipt["status"] = "failed"
        receipt["failure"] = str(error)
        raise
    finally:
        save()


if __name__ == "__main__":
    main()
