#!/usr/bin/env python3
"""对完整受控插件只读执行原生 hooks/list，不创建回合、不写入信任配置。"""

import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import sys
import tempfile

from apply_notification_patch import bundle_data, default_bundle
from codex_persistent_source import materialize_plugin_fixture, source_bundle
from probe_codex_notification_paths import start_server
from probe_local_tools import environment


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-executable", required=True)
    parser.add_argument("--codex-version", choices=("0.147.0", "0.156.1"), default="0.147.0")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    executable = str(Path(args.codex_executable).resolve())
    version = subprocess.run([executable, "--version"], capture_output=True, text=True, check=True, timeout=5).stdout.strip()
    if version != f"codex-cli {args.codex_version}" or sys.platform not in ("darwin", "linux", "win32"):
        raise SystemExit("CLI 版本或平台不符合显式选择的固定通知契约")
    with Path(executable).open("rb") as binary:
        executable_sha256 = hashlib.file_digest(binary, "sha256").hexdigest()
    metadata, _ = bundle_data(default_bundle(), "codex")
    _, source_bytes, _ = source_bundle(default_bundle())
    with tempfile.TemporaryDirectory(prefix="infinishell-hook-trust-") as temporary:
        directory = Path(temporary).resolve()
        env = environment(directory)
        env["HOME"] = str(directory / "home")
        Path(env["HOME"]).mkdir()
        marketplace = directory / "marketplace"
        plugin = marketplace / "plugins/warp"
        # 从完整受控来源装配，新增原生脚本不能走只替换旧文件的路径。
        materialize_plugin_fixture(default_bundle(), plugin)
        index = marketplace / ".agents/plugins/marketplace.json"
        index.parent.mkdir(parents=True)
        index.write_text(json.dumps({"name": "codex-warp", "plugins": [{"name": "warp", "source": "./plugins/warp", "version": "0.4.0", "policy": {"installation": "AVAILABLE", "authentication": "ON_INSTALL"}}]}), encoding="utf-8")
        for command in (["plugin", "marketplace", "add", str(marketplace), "--json"], ["plugin", "add", "warp@codex-warp", "--json"]):
            subprocess.run([executable, *command], cwd=directory, env=env, capture_output=True, check=True, timeout=30)
        config = Path(env["CODEX_HOME"]) / "config.toml"
        before = config.read_bytes()
        recorder = start_server(executable, env, directory)
        try:
            listing = recorder.rpc("hooks/list", {"cwds": [str(directory)]}, 2)
        finally:
            recorder.close()
        hooks = [item for group in listing.get("result", {}).get("data", []) for item in group.get("hooks", []) if item.get("pluginId") == "warp@codex-warp"]
        fields = ("key", "eventName", "handlerType", "command", "timeoutSec", "enabled", "source", "pluginId", "currentHash", "trustStatus")
        hooks = [{key: item.get(key) for key in fields} for item in hooks]
        registration_shape_valid = len(hooks) == 5 and all(item["trustStatus"] == "untrusted" for item in hooks)
        contract_name = "NATIVE_HOOK_TRUST_WINDOWS.json" if sys.platform == "win32" else "NATIVE_HOOK_TRUST.json"
        contract = json.loads((default_bundle() / "codex" / contract_name).read_bytes())
        contract_matches = {item["key"]: item["currentHash"] for item in hooks} == {
            item["key"]: item["currentHash"] for item in contract["hooks"]
        }
        native_config_unchanged = config.read_bytes() == before
        report = {"cli": version, "plugin": "warp@codex-warp", "plugin_version": "0.4.0", "patch_revision": metadata["patch_revision"], "hooks_file_sha256": hashlib.sha256((plugin / "hooks/hooks.json").read_bytes()).hexdigest(), "model_request_attempted": False, "credentials_provided": False, "native_config_unchanged": native_config_unchanged, "source_metadata_sha256": hashlib.sha256(source_bytes).hexdigest(), "verified_platform": {"darwin": "macos", "linux": "linux", "win32": "windows"}[sys.platform], "verified_architecture": platform.machine().lower(), "executable_sha256": executable_sha256, "hooks": hooks, "passed": True}
    passed = registration_shape_valid and contract_matches and native_config_unchanged
    report.update(registration_shape_valid=registration_shape_valid, product_contract_matches=contract_matches, passed=passed)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if not passed:
        raise SystemExit("原生 Hook 注册、摘要或配置保持检查失败；实际收据已保留")
    print(json.dumps({"passed": True, "hooks": len(hooks)}))


if __name__ == "__main__":
    main()
