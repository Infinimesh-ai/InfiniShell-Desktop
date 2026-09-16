#!/usr/bin/env python3
"""在隔离配置中探测真实 CLI；仅保存经过脱敏的协议证据。"""

import argparse
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import tempfile
import threading
import time
import uuid


class Recorder:
    def __init__(self, command, environment, directory, output):
        self.directory = str(directory)
        self.output = output
        self.output_lock = threading.Lock()
        self.readers = []
        self.messages = queue.Queue()
        self.process = subprocess.Popen(
            command, cwd=directory, env=environment, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1,
        )
        for name in ("stdout", "stderr"):
            reader = threading.Thread(target=self.read, args=(name,), daemon=True)
            reader.start()
            self.readers.append(reader)

    def clean(self, value):
        if isinstance(value, dict):
            return {
                key: "<redacted>" if re.search(
                    r"^(access_token|refresh_token|id_token|api_key|authorization|email|hostname|serverName|installationId|team_id|team_name|team_role|subscription_tier|agentId|agentInstanceId|workflowPath)$", key,
                    re.IGNORECASE,
                ) else self.clean(item) for key, item in value.items()
            }
        if isinstance(value, list):
            return [self.clean(item) for item in value]
        if isinstance(value, str):
            value = value.replace(str(Path(self.directory).resolve()), "<probe-project>")
            value = value.replace(self.directory, "<probe-project>")
            value = value.replace(str(Path.home()), "<user-home>")
            return re.sub(r"(?:sk-[A-Za-z0-9_-]{16,}|Bearer [A-Za-z0-9_.-]+)", "<redacted>", value)
        return value

    def record(self, direction, value):
        with self.output_lock:
            self.output.write(json.dumps({"direction": direction, "message": self.clean(value)}, ensure_ascii=False) + "\n")
            self.output.flush()

    def read(self, name):
        for line in getattr(self.process, name):
            try:
                value = json.loads(line)
            except json.JSONDecodeError:
                value = line.rstrip()
            self.record(name, value)
            if name == "stdout":
                self.messages.put(value)

    def send(self, value):
        self.record("stdin", value)
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()

    def until(self, predicate, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                message = self.messages.get(timeout=min(0.2, deadline - time.monotonic()))
            except queue.Empty:
                if self.process.poll() is not None:
                    break
                continue
            if predicate(message):
                return message
        return None

    def rpc(self, method, params, request_id, timeout=20, jsonrpc=False):
        request = {"id": request_id, "method": method, "params": params}
        if jsonrpc:
            request["jsonrpc"] = "2.0"
        self.send(request)
        return self.until(lambda item: isinstance(item, dict) and item.get("id") == request_id, timeout)

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        for reader in self.readers:
            reader.join(timeout=2)
        self.record("process", {"exit_code": self.process.returncode})


def probe(cli, executable, directory, output):
    environment = os.environ.copy()
    # 子进程只使用专用配置根；默认探测不继承模型或登录凭据。
    for key in list(environment):
        if any(name in key for name in ("TOKEN", "API_KEY", "AUTH", "SECRET")):
            environment.pop(key)
    environment.update({
        "CODEX_HOME": str(directory / "codex"),
        "GROK_HOME": str(directory / "grok"),
        "CLAUDE_CONFIG_DIR": str(directory / "claude"),
        "GROK_AUTO_UPDATE": "0", "GROK_DISABLE_AUTOUPDATER": "1",
        "GROK_CLAUDE_HOOKS_ENABLED": "0", "GROK_CLAUDE_MCPS_ENABLED": "0",
        "GROK_CODEX_HOOKS_ENABLED": "0", "GROK_CODEX_MCPS_ENABLED": "0",
    })
    for name in ("codex", "grok", "claude"):
        (directory / name).mkdir(exist_ok=True)
    if cli == "codex":
        command = [executable, "app-server", "--stdio"]
    elif cli == "grok":
        command = [executable, "agent", "stdio", "--leader-socket", str(directory / "grok.sock")]
    else:
        command = [executable, "--print", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--replay-user-messages", "--permission-mode", "dontAsk", "--permission-prompts", "none", "--strict-mcp-config", "--setting-sources", ""]
    recorder = Recorder(command, environment, directory, output)
    try:
        if cli == "codex":
            recorder.rpc("initialize", {"clientInfo": {"name": "infinishell_protocol_probe", "version": "0.1.0"}}, 1)
            recorder.send({"method": "initialized"})
            response = recorder.rpc("thread/start", {"cwd": str(directory), "approvalPolicy": "never", "sandbox": "read-only"}, 2)
            if response and "result" in response:
                thread_id = response["result"]["thread"]["id"]
                recorder.rpc("turn/steer", {"threadId": thread_id, "expectedTurnId": "missing-turn", "input": [{"type": "text", "text": "Hello"}]}, 3)
                recorder.rpc("thread/resume", {"threadId": "00000000-0000-4000-8000-000000000000"}, 4)
        elif cli == "grok":
            recorder.rpc("initialize", {"protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False}}, 1, jsonrpc=True)
            recorder.rpc("session/new", {"cwd": str(directory), "mcpServers": []}, 2, jsonrpc=True)
            recorder.rpc("session/load", {"sessionId": "00000000-0000-4000-8000-000000000000", "cwd": str(directory), "mcpServers": []}, 3, jsonrpc=True)
            recorder.rpc("session/resume", {"sessionId": "00000000-0000-4000-8000-000000000000", "cwd": str(directory), "mcpServers": []}, 4, jsonrpc=True)
        else:
            recorder.send({"type": "control_request", "request_id": "init-probe", "request": {"subtype": "initialize"}})
            recorder.until(lambda item: isinstance(item, dict) and item.get("type") == "control_response")
            recorder.send({"type": "user", "message": {"role": "user", "content": "只回复 PROBE_OK，不调用工具。"}, "parent_tool_use_id": None, "session_id": "", "uuid": str(uuid.uuid4())})
            recorder.until(lambda item: isinstance(item, dict) and item.get("type") == "result", 30)
            recorder.send({"type": "control_request", "request_id": "interrupt-probe", "request": {"subtype": "interrupt"}})
            recorder.until(lambda item: isinstance(item, dict) and item.get("type") == "control_response")
    finally:
        recorder.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", choices=("codex", "claude", "grok"), required=True)
    parser.add_argument("--executable", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="infinishell-parity-") as temporary:
        with args.output.open("w") as output:
            probe(args.cli, args.executable, Path(temporary), output)


if __name__ == "__main__":
    main()
