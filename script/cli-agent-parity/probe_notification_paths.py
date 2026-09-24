#!/usr/bin/env python3
"""隔离执行 Claude 原生初始化 hook；只核对路径参数，不请求模型或读取登录凭据。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from probe_local_tools import environment


ASSETS = Path(__file__).resolve().parents[2] / "app/assets/bundled/cli-agent-plugins"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claude-executable", required=True)
    parser.add_argument("--claude-version", choices=("2.1.273", "2.1.280"), default="2.1.273")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    version = subprocess.run([args.claude_executable, "--version"], text=True, capture_output=True, check=True, timeout=5).stdout.strip()
    if version != f"{args.claude_version} (Claude Code)" or (args.claude_version == "2.1.280" and sys.platform != "darwin"):
        raise SystemExit("CLI 版本或平台不符合显式选择的固定通知契约")
    hooks_path = ASSETS / "claude/hooks/hooks.json"
    hooks = json.loads(hooks_path.read_text(encoding="utf-8"))
    names = ["插件 空 格", "插件 ' $(touch INJECTED) `touch INJECTED`"]
    if os.name == "posix":
        names.append('插件 " $(touch INJECTED) `touch INJECTED`')
    results = []
    for name in names:
        with tempfile.TemporaryDirectory(prefix="infinishell-native-hook-path-") as temporary:
            directory = Path(temporary)
            plugin = directory / name
            for child in (".claude-plugin", "hooks", "scripts"):
                (plugin / child).mkdir(parents=True)
            (plugin / ".claude-plugin/plugin.json").write_text(json.dumps({"name": "infinishell-path-probe", "version": "0.1.0"}), encoding="utf-8")
            (plugin / "hooks/hooks.json").write_text(json.dumps({"hooks": {"SessionStart": hooks["hooks"]["SessionStart"]}}), encoding="utf-8")
            # 使用随附的真实入口配置，仅将脚本内容换成无模型、无终端输出的观测器。
            (plugin / "scripts/on-session-start.sh").write_text(
                '#!/bin/bash\njq -c --arg root "$CLAUDE_PLUGIN_ROOT" '\
                "'{hook_event_name, plugin_root:$root}' > \"$INFINISHELL_NOTIFICATION_PATH_PROBE/marker.json\"\n",
                encoding="utf-8", newline="\n",
            )
            isolated = environment(directory)
            (directory / "home").mkdir()
            isolated.update(HOME=str(directory / "home"), INFINISHELL_NOTIFICATION_PATH_PROBE=str(directory))
            command = [args.claude_executable, "--init-only", "--setting-sources", "", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}', "--plugin-dir", str(plugin)]
            result = subprocess.run(command, env=isolated, cwd=directory, text=True, capture_output=True, timeout=20)
            marker_path = directory / "marker.json"
            marker = json.loads(marker_path.read_text(encoding="utf-8")) if marker_path.is_file() else {}
            record = {
                "directory_name": name,
                "exit_code": result.returncode,
                "native_session_start": marker.get("hook_event_name") == "SessionStart",
                "plugin_root_preserved": marker.get("plugin_root") == str(plugin),
                "shell_injection_marker_absent": not (directory / "INJECTED").exists(),
            }
            results.append(record)
    passed = all(item["exit_code"] == 0 and item["native_session_start"] and item["plugin_root_preserved"] and item["shell_injection_marker_absent"] for item in results)
    report = {
        "cli": version, "host_os": sys.platform,
        "hook_manifest_sha256": hashlib.sha256(hooks_path.read_bytes()).hexdigest(),
        "mode": "native_init_only_instrumented_session_start",
        "model_requested": False, "full_notification_lifecycle_verified": False,
        "cases": results, "passed": passed,
    }
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"passed": passed, "cases": len(results)}))
    if not passed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
