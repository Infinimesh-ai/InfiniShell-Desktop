#!/usr/bin/env python3
"""核对固定 Grok 已认证 ACP 的图片合同；不把协议接收当作图片理解。"""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

from probe_protocol import Recorder
from run_grok_official_adapter_live import copy_private_auth
from prepare_grok_cli import verify_binary


class ImageRecorder(Recorder):
    def __init__(self, *args):
        self.responses = []
        self.denied_tools = 0
        super().__init__(*args)

    def record(self, direction, value):
        # stderr 可能含账户诊断，只存字节摘要；协议中的认证字段沿用脱敏器。
        if direction == "stderr":
            value = {"bytes_sha256": hashlib.sha256(str(value).encode()).hexdigest()}
        elif isinstance(value, dict):
            self.responses.append((direction, value))
        super().record(direction, value)

    def until(self, predicate, timeout=20):
        def handle(value):
            if isinstance(value, dict) and "id" in value and "method" in value:
                if value["method"] == "session/request_permission":
                    self.denied_tools += 1
                    self.send({"jsonrpc": "2.0", "id": value["id"],
                               "result": {"outcome": {"outcome": "cancelled"}}})
                else:
                    self.send({"jsonrpc": "2.0", "id": value["id"],
                               "error": {"code": -32601, "message": "Probe client method unavailable"}})
            return predicate(value)
        return super().until(handle, timeout)


def run_session(recorder, project, body, session=None, pure_image=False):
    summary = {"resumed": session is not None, "pure_image": pure_image}
    initialized = recorder.rpc("initialize", {"protocolVersion": 1,
        "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False},
                               "terminal": False}}, 1, jsonrpc=True)
    if not initialized or "result" not in initialized:
        return None, dict(summary, initialize_received=False)
    summary["capabilities"] = initialized["result"].get("agentCapabilities")
    methods = {item["id"] for item in initialized["result"].get("authMethods", [])}
    if "cached_token" not in methods:
        return None, dict(summary, authenticated=False)
    authenticated = recorder.rpc("authenticate", {"methodId": "cached_token",
        "_meta": {"headless": True}}, 2, jsonrpc=True)
    summary["authenticated"] = bool(authenticated and "result" in authenticated)
    if not summary["authenticated"]:
        return None, summary
    params = {"cwd": str(project), "mcpServers": []}
    if session:
        params["sessionId"] = session
    created = recorder.rpc("session/load" if session else "session/new", params,
                           3, timeout=45, jsonrpc=True)
    summary["session_ready"] = bool(created and "result" in created)
    if not summary["session_ready"]:
        return None, summary
    native_id = created["result"].get("sessionId", session)
    summary["same_session"] = not session or native_id == session
    session = native_id
    summary["session_sha256"] = hashlib.sha256(session.encode()).hexdigest()
    content = [] if pure_image else [{"type": "text", "text": "只根据随附图片回答左半和右半分别是什么颜色，只输出两个中文颜色名称。不要使用工具，也不要猜测。"}]
    content.append({"type": "image", "mimeType": "image/png", "data": base64.b64encode(body).decode()})
    start = len(recorder.responses)
    result = recorder.rpc("session/prompt", {"sessionId": session, "prompt": content},
                          4, timeout=90, jsonrpc=True)
    summary["image_prompt_response"] = recorder.clean(result)
    summary["model_answer"] = recorder.clean("".join(message.get("params", {}).get("update", {}).get("content", {}).get("text", "")
        for direction, message in recorder.responses[start:] if direction == "stdout"
        and message.get("params", {}).get("update", {}).get("sessionUpdate") == "agent_message_chunk"))
    summary["permission_requests_denied"] = recorder.denied_tools
    recorder.rpc("session/close", {"sessionId": session}, 5, jsonrpc=True)
    return session, summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--credential-home", type=Path, required=True)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cold-resume", action="store_true")
    parser.add_argument("--pure-image", action="store_true")
    args = parser.parse_args()
    executable = args.executable.resolve(strict=True)
    identity = verify_binary(executable, "darwin-arm64", "1.0.41")
    version = subprocess.check_output([str(executable), "--version"], text=True).strip()
    if version != "grok 1.0.41 (4220f3b224a6)":
        raise ValueError("固定版本不匹配")
    body = args.image.read_bytes()
    if not body.startswith(b"\x89PNG\r\n\x1a\n") or len(body) > 1024 * 1024:
        raise ValueError("仅接受小于 1 MiB 的专用 PNG 夹具")
    args.output.mkdir(parents=True, exist_ok=False)
    summary = {"source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
               "version": version, "binary": identity, "platform": "macos-arm64",
               "mode": "native_acp_no_leader", "model_requested": "grok-4.7",
               "image_sha256": hashlib.sha256(body).hexdigest(), "image_bytes": len(body),
               "probe_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
               "runs": [], "product_adapter_tested": False,
               "image_understanding_passed": False, "full_goal_passed": False}
    with tempfile.TemporaryDirectory(prefix="isp-grok-image-", dir="/private/tmp") as temporary:
        root = Path(temporary)
        home, project = root / "home", root / "project"
        configuration = home / ".grok"
        configuration.mkdir(parents=True, mode=0o700)
        project.mkdir(mode=0o700)
        auth = copy_private_auth(args.credential_home, configuration)
        environment = {key: value for key, value in os.environ.items()
                       if not re.search(r"TOKEN|API_KEY|AUTH|SECRET|^GROK_|^XAI_", key)}
        environment.update({"HOME": str(home), "GROK_HOME": str(configuration),
                            "XDG_CONFIG_HOME": str(home / ".config"),
                            "XDG_DATA_HOME": str(home / ".local/share"),
                            "XDG_CACHE_HOME": str(home / ".cache"),
                            "GROK_DISABLE_API_KEY_AUTH": "1", "GROK_AUTO_UPDATE": "0",
                            "GROK_DISABLE_AUTOUPDATER": "1", "GROK_CLAUDE_HOOKS_ENABLED": "0",
                            "GROK_CLAUDE_MCPS_ENABLED": "0", "GROK_CODEX_HOOKS_ENABLED": "0",
                            "GROK_CODEX_MCPS_ENABLED": "0"})
        (configuration / "config.toml").write_text('[cli]\nauto_update = false\n[models]\ndefault = "grok-4.7"\n')
        session = None
        try:
            for index in range(2 if args.cold_resume else 1):
                with (args.output / f"protocol-{index}.safe.ndjson").open("x") as output:
                    recorder = ImageRecorder([str(executable), "--no-auto-update", "--trust", "agent",
                                              "--no-leader", "--model", "grok-4.7", "stdio"],
                                             environment, project, output)
                    observed = {}
                    try:
                        session, observed = run_session(recorder, project, body, session, args.pure_image)
                    finally:
                        recorder.close()
                        observed["exit_code"] = recorder.process.returncode
                        observed["readers_finished"] = all(not reader.is_alive() for reader in recorder.readers)
                        summary["runs"].append(observed)
                    if session is None:
                        break
        finally:
            auth.unlink(missing_ok=True)
            summary["private_auth_removed"] = not auth.exists()
            (args.output / "summary.safe.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"runs": [{key: row.get(key) for key in
        ("authenticated", "session_ready", "resumed", "same_session", "model_answer", "exit_code")}
        for row in summary["runs"]]}, ensure_ascii=False))


if __name__ == "__main__":
    main()
