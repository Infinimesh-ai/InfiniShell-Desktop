#!/usr/bin/env python3
"""对完整受控插件只读执行原生 hooks/list，不创建回合、不写入信任配置。"""

import argparse
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from apply_notification_patch import apply_files, bundle_data, default_bundle, validate_tree
from probe_codex_notification_paths import start_server
from probe_local_tools import environment


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-executable", required=True)
    parser.add_argument("--codex-version", choices=("0.147.0", "0.156.1"), default="0.147.0")
    parser.add_argument("--upstream-plugin", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    executable = str(Path(args.codex_executable).resolve())
    version = subprocess.run([executable, "--version"], capture_output=True, text=True, check=True, timeout=5).stdout.strip()
    if version != f"codex-cli {args.codex_version}" or (args.codex_version == "0.156.1" and sys.platform != "darwin"):
        raise SystemExit("CLI 版本或平台不符合显式选择的固定通知契约")
    metadata, replacements = bundle_data(default_bundle(), "codex")
    with tempfile.TemporaryDirectory(prefix="infinishell-hook-trust-") as temporary:
        directory = Path(temporary).resolve()
        env = environment(directory)
        env["HOME"] = str(directory / "home")
        Path(env["HOME"]).mkdir()
        marketplace = directory / "marketplace"
        plugin = marketplace / "plugins/warp"
        shutil.copytree(args.upstream_plugin, plugin)
        validate_tree(plugin, "0.4.0", metadata)
        apply_files(plugin, metadata, replacements)
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
        hooks = [{key: item[key] for key in fields} for item in hooks]
        assert len(hooks) == 5 and all(item["trustStatus"] == "untrusted" for item in hooks)
        assert config.read_bytes() == before
        report = {"cli": version, "plugin": "warp@codex-warp", "plugin_version": "0.4.0", "patch_revision": metadata["patch_revision"], "hooks_file_sha256": hashlib.sha256((plugin / "hooks/hooks.json").read_bytes()).hexdigest(), "model_request_attempted": False, "credentials_provided": False, "native_config_unchanged": True, "hooks": hooks, "passed": True}
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "hooks": len(hooks)}))


if __name__ == "__main__":
    main()
