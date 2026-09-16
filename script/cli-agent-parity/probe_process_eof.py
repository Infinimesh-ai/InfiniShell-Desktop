#!/usr/bin/env python3
"""验证原生空闲 stdio EOF；结果不证明父进程崩溃或运行中工具树已安全退出。"""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from probe_local_tools import ToolRecorder, environment


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex-executable", required=True)
    parser.add_argument("--claude-executable", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    cases = []
    for agent, executable, expected in (
        ("codex", args.codex_executable, "codex-cli 0.147.0"),
        ("claude", args.claude_executable, "2.1.273 (Claude Code)"),
    ):
        version = subprocess.run([executable, "--version"], capture_output=True, text=True, check=True, timeout=5).stdout.strip()
        if version != expected:
            raise SystemExit("CLI 版本不属于此探测的固定契约")
        with tempfile.TemporaryDirectory(prefix="infinishell-native-eof-") as temporary:
            directory = Path(temporary)
            isolated = environment(directory)
            (directory / "home").mkdir()
            isolated["HOME"] = str(directory / "home")
            command = [executable, "app-server", "--stdio"] if agent == "codex" else [executable, "--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--setting-sources", "", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}']
            with (directory / "raw.ndjson").open("w", encoding="utf-8") as output:
                recorder = ToolRecorder(command, isolated, directory, output)
                try:
                    if agent == "codex":
                        response = recorder.rpc("initialize", {"clientInfo": {"name": "infinishell_eof_probe", "version": "0.1.0"}}, 1)
                        recorder.send({"method": "initialized"})
                    else:
                        recorder.send({"type": "control_request", "request_id": "init", "request": {"subtype": "initialize"}})
                        response = recorder.until(lambda item: isinstance(item, dict) and item.get("type") == "control_response")
                    before = time.monotonic()
                    recorder.process.stdin.close()
                    try:
                        recorder.process.wait(timeout=5)
                        exited = True
                    except subprocess.TimeoutExpired:
                        exited = False
                    cases.append({"agent": agent, "cli_version": version, "handshake_received": response is not None,
                        "stdin_eof_exited_within_5s": exited, "exit_code_before_cleanup": recorder.process.returncode,
                        "elapsed_ms": round((time.monotonic() - before) * 1000), "active_turn_tested": False, "tool_descendant_tested": False})
                finally:
                    # 超时后的强制清理不计入 EOF 自行退出结果。
                    recorder.close()
    report = {"mode": "native_idle_stdio_eof", "host_os": sys.platform, "model_requested": False,
        "parent_sigkill_tested": False, "grok_tested": False, "cases": cases}
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")
    passed = all(case["handshake_received"] and case["stdin_eof_exited_within_5s"] and case["exit_code_before_cleanup"] == 0 for case in cases)
    print(json.dumps({"idle_eof_passed": passed, "cases": len(cases)}))
    if not passed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
