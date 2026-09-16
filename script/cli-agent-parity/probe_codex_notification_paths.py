#!/usr/bin/env python3
"""隔离验证 Codex 原生 SessionStart 路径与信任状态；无登录的首次输入仅用于触发回调。"""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from probe_local_tools import environment
from probe_protocol import Recorder


ASSETS = Path(__file__).resolve().parents[2] / "app/assets/bundled/cli-agent-plugins/codex"


def start_server(executable, env, directory):
    recorder = Recorder([executable, "app-server", "--stdio", "--disable", "shell_snapshot"], env, directory, io.StringIO())
    response = recorder.rpc("initialize", {"clientInfo": {"name": "infinishell_hook_path_probe", "version": "0.1.0"}, "capabilities": {"experimentalApi": True}}, 1)
    if not response or "result" not in response:
        recorder.close()
        raise RuntimeError("原生初始化失败")
    recorder.send({"method": "initialized"})
    return recorder


def case(executable, name):
    with tempfile.TemporaryDirectory(prefix="infinishell-codex-hook-path-") as temporary:
        directory = Path(temporary).resolve()
        env = environment(directory)
        cli_home = directory / name
        cli_home.mkdir()
        env.update(CODEX_HOME=str(cli_home), HOME=str(directory / "home"), INFINISHELL_NOTIFICATION_PATH_PROBE=str(directory))
        (directory / "home").mkdir()
        marketplace = directory / "marketplace"
        plugin = marketplace / "plugins/path-probe"
        for child in (".codex-plugin", "hooks", "scripts"):
            (plugin / child).mkdir(parents=True)
        index = marketplace / ".agents/plugins/marketplace.json"
        index.parent.mkdir(parents=True)
        index.write_text(json.dumps({"name": "infinishell-path-probe", "plugins": [{"name": "path-probe", "source": "./plugins/path-probe", "version": "0.1.0", "policy": {"installation": "AVAILABLE", "authentication": "ON_INSTALL"}}]}), encoding="utf-8")
        (plugin / ".codex-plugin/plugin.json").write_text(json.dumps({"name": "path-probe", "description": "Isolated hook path fixture", "version": "0.1.0"}), encoding="utf-8")
        hooks = json.loads((ASSETS / "hooks/hooks.json").read_text(encoding="utf-8"))
        (plugin / "hooks/hooks.json").write_text(json.dumps({"hooks": {"SessionStart": hooks["hooks"]["SessionStart"]}}), encoding="utf-8")
        # 入口使用随附清单原文，脚本仅记录允许的字段，便于独立检验路径保真。
        script = '#!/bin/bash\njq -c --arg root "$PLUGIN_ROOT" \'{hook_event_name, plugin_root:$root}\' > "$INFINISHELL_NOTIFICATION_PATH_PROBE/marker.json"\n'
        (plugin / "scripts/on-session-start.sh").write_text(script, encoding="utf-8", newline="\n")
        for command in (["plugin", "marketplace", "add", str(marketplace), "--json"], ["plugin", "add", "path-probe@infinishell-path-probe", "--json"]):
            installed = subprocess.run([executable, *command], env=env, cwd=directory, capture_output=True, text=True, timeout=30)
            if installed.returncode:
                raise RuntimeError(f"原生安装失败：{installed.stderr[:1000]}")
        recorder = start_server(executable, env, directory)
        try:
            listing = recorder.rpc("hooks/list", {"cwds": [str(directory)]}, 2)
            groups = listing.get("result", {}).get("data", []) if listing else []
            candidates = [hook for group in groups for hook in group.get("hooks", []) if hook.get("eventName") == "sessionStart" and hook.get("source") == "plugin"]
            if len(candidates) != 1:
                raise RuntimeError(f"未发现唯一测试 hook：{recorder.clean(listing)}")
            hook = candidates[0]
        finally:
            recorder.close()
        trust = hook["trustStatus"]
        if trust not in ("trusted", "managed"):
            # 只信任本测试刚生成并核对的静态脚本；此操作不会应用到用户插件或配置。
            if Path(hook["sourcePath"]).parent.parent.joinpath("scripts/on-session-start.sh").read_text(encoding="utf-8") != script:
                raise RuntimeError("待授权脚本与隔离测试内容不一致")
            with (cli_home / "config.toml").open("a", encoding="utf-8") as config:
                config.write(f'\n[hooks.state.{json.dumps(hook["key"])}]\nenabled = true\ntrusted_hash = {json.dumps(hook["currentHash"])}\n')
        recorder = start_server(executable, env, directory)
        try:
            trusted = recorder.rpc("hooks/list", {"cwds": [str(directory)]}, 4)
            final_hooks = trusted.get("result", {}).get("data", [{}])[0].get("hooks", []) if trusted else []
            result = recorder.rpc("thread/start", {"cwd": str(directory), "sessionStartSource": "startup", "ephemeral": False, "sandbox": "read-only", "approvalPolicy": "on-request"}, 3)
            if result and "result" in result:
                recorder.rpc("turn/start", {"threadId": result["result"]["thread"]["id"], "input": [{"type": "text", "text": "Reply exactly HOOK_PROBE.", "text_elements": []}]}, 5)
            marker_path = directory / "marker.json"
            deadline = time.monotonic() + 3
            while not marker_path.is_file() and time.monotonic() < deadline:
                time.sleep(0.02)
            marker = json.loads(marker_path.read_text(encoding="utf-8")) if marker_path.is_file() else {}
            return {
                "directory_name": name,
                "initial_trust_status": trust,
                "explicit_fixture_trust_added": trust not in ("trusted", "managed"),
                "verified_trust_status": next((item["trustStatus"] for item in final_hooks if item["key"] == hook["key"]), None),
                "native_thread_started": bool(result and "result" in result),
                "native_session_start": marker.get("hook_event_name") == "SessionStart",
                "plugin_root_preserved": marker.get("plugin_root") == str(Path(hook["sourcePath"]).parent.parent),
                "shell_injection_marker_absent": not (directory / "INJECTED").exists(),
                "native_trace": recorder.output.getvalue()[-6000:] if not marker else None,
            }
        finally:
            recorder.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-executable", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    version = subprocess.run([args.codex_executable, "--version"], capture_output=True, text=True, check=True, timeout=5).stdout.strip()
    if version != "codex-cli 0.147.0":
        raise SystemExit("只接受已核实的 Codex CLI 0.147.0")
    names = ["插件 空 格", "插件 ' $(touch INJECTED) `touch INJECTED`"]
    if os.name == "posix":
        names.append('插件 " $(touch INJECTED) `touch INJECTED`')
    cases = []
    for name in names:
        try:
            cases.append(case(args.codex_executable, name))
        except Exception as error:
            cases.append({"directory_name": name, "error": str(error)})
    checks = ("native_thread_started", "native_session_start", "plugin_root_preserved", "shell_injection_marker_absent")
    passed = all(all(item.get(key) is True for key in checks) for item in cases)
    report = {"cli": version, "host_os": sys.platform, "hook_manifest_sha256": hashlib.sha256((ASSETS / "hooks/hooks.json").read_bytes()).hexdigest(), "mode": "native_first_turn_instrumented_session_start", "model_request_attempted": True, "credentials_provided": False, "model_generation_verified": False, "shell_snapshot_enabled": False, "full_notification_lifecycle_verified": False, "cases": cases, "passed": passed}
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"passed": passed, "cases": len(cases)}))
    if not passed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
