#!/usr/bin/env python3
"""隔离凭据验证本地工具注册；不请求模型生成，也不调用任务工具。"""

import argparse
import json
import os
from pathlib import Path
import queue
import tempfile
import time

from probe_protocol import Recorder


SERVER = "infinishell-local-tasks"
TOOL = {
    "name": "inspect_local_tasks",
    "description": "Inspect only tasks related to this local agent connection.",
    "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
}


class ToolRecorder(Recorder):
    def clean(self, value):
        if isinstance(value, dict) and isinstance(value.get("commands"), list):
            value = {**value, "commands": "<unrelated command descriptions omitted>"}
        return super().clean(value)


def environment(directory):
    # 仅继承启动所需环境；既不读取用户认证文件，也不带入 API 或代理凭据。
    result = {key: os.environ[key] for key in ("PATH", "TMPDIR", "LANG", "LC_ALL", "SYSTEMROOT", "WINDIR") if key in os.environ}
    result.update({"CODEX_HOME": str(directory / "codex"), "CLAUDE_CONFIG_DIR": str(directory / "claude"), "CLAUDE_CODE_ENTRYPOINT": "sdk-py"})
    for name in ("codex", "claude"):
        (directory / name).mkdir()
    return result


def codex(executable, directory, output):
    recorder = ToolRecorder([executable, "app-server", "--stdio"], environment(directory), directory, output)
    try:
        recorder.rpc("initialize", {"clientInfo": {"name": "infinishell_local_tools_probe", "version": "0.1.0"}, "capabilities": {"experimentalApi": True}}, 1)
        recorder.send({"method": "initialized"})
        result = recorder.rpc("thread/start", {"cwd": str(directory), "ephemeral": True, "sandbox": "read-only", "approvalPolicy": "on-request", "dynamicTools": [{"type": "function", **TOOL}]}, 2)
        recorder.record("assertion", {"tool_registration_accepted": bool(result and "result" in result), "tool_invocation_tested": False})
    finally:
        recorder.close()


def claude(executable, directory, output):
    command = [executable, "--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--permission-prompt-tool", "stdio", "--permission-prompts", "host", "--setting-sources", "", "--strict-mcp-config", "--mcp-config", json.dumps({"mcpServers": {SERVER: {"type": "sdk", "name": SERVER}}})]
    recorder = ToolRecorder(command, environment(directory), directory, output)
    observed = set()
    try:
        recorder.send({"type": "control_request", "request_id": "init-local-tools", "request": {"subtype": "initialize"}})
        deadline = time.monotonic() + 20
        sent_status = False
        while time.monotonic() < deadline:
            try:
                item = recorder.messages.get(timeout=0.2)
            except queue.Empty:
                if recorder.process.poll() is not None:
                    break
                continue
            if not isinstance(item, dict):
                continue
            if item.get("type") == "control_response":
                request_id = item.get("response", {}).get("request_id")
                if request_id == "init-local-tools" and not sent_status:
                    recorder.send({"type": "control_request", "request_id": "status-local-tools", "request": {"subtype": "mcp_status"}})
                    sent_status = True
                elif request_id == "registered-local-tools":
                    break
                continue
            request = item.get("request", {})
            if item.get("type") != "control_request" or request.get("subtype") != "mcp_message":
                continue
            message = request.get("message", {})
            method = message.get("method")
            observed.add(method)
            if method == "initialize":
                result = {"protocolVersion": message["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": SERVER, "version": "0.1.0"}}
            elif method == "tools/list":
                result = {"tools": [TOOL]}
            else:
                result = {}
            response = {"jsonrpc": "2.0", "result": result}
            if "id" in message:
                response["id"] = message["id"]
            recorder.send({"type": "control_response", "response": {"subtype": "success", "request_id": item["request_id"], "response": {"mcp_response": response}}})
            if method == "tools/list":
                recorder.send({"type": "control_request", "request_id": "registered-local-tools", "request": {"subtype": "mcp_status"}})
        recorder.record("assertion", {"sdk_mcp_methods": sorted(observed), "tool_invocation_tested": False})
    finally:
        recorder.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", choices=("codex", "claude"), required=True)
    parser.add_argument("--executable", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="infinishell-local-tools-") as temporary:
        with args.output.open("w") as output:
            {"codex": codex, "claude": claude}[args.cli](args.executable, Path(temporary), output)


if __name__ == "__main__":
    main()
