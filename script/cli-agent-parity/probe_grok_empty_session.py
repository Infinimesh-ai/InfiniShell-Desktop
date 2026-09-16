#!/usr/bin/env python3
"""在临时配置中验证 Grok 空会话恢复；不提交模型请求。"""

import argparse
import os
from pathlib import Path
import tempfile

from probe_protocol import Recorder


def run(executable, credentials, output):
    with tempfile.TemporaryDirectory(prefix="infinishell-grok-empty-") as temporary:
        directory = Path(temporary)
        configuration = directory / "grok"
        configuration.mkdir(mode=0o700)
        credential_copy = configuration / "auth.json"
        # 只在自动清理的私有目录短暂复制，原始登录文件保持只读。
        with credential_copy.open("xb") as target:
            credential_copy.chmod(0o600)
            target.write(credentials.read_bytes())
        environment = os.environ.copy()
        for key in list(environment):
            if any(part in key for part in ("TOKEN", "API_KEY", "AUTH", "SECRET")):
                environment.pop(key)
        environment.update({
            "GROK_HOME": str(configuration), "GROK_AUTO_UPDATE": "0",
            "GROK_DISABLE_AUTOUPDATER": "1", "GROK_CLAUDE_HOOKS_ENABLED": "0",
            "GROK_CLAUDE_MCPS_ENABLED": "0", "GROK_CODEX_HOOKS_ENABLED": "0",
            "GROK_CODEX_MCPS_ENABLED": "0",
        })
        session_id = None
        for index, method in enumerate(("session/new", "session/load", "session/resume")):
            recorder = Recorder([
                str(executable), "agent", "stdio", "--leader-socket",
                str(directory / f"leader-{index}.sock"),
            ], environment, directory, output)
            try:
                recorder.record("probe", {"phase": method, "model_prompt_submitted": False})
                initialized = recorder.rpc("initialize", {
                    "protocolVersion": 1,
                    "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False},
                }, 1, jsonrpc=True)
                assert initialized and "result" in initialized, "ACP 初始化失败"
                methods = initialized["result"].get("authMethods", [])
                assert any(item["id"] == "cached_token" for item in methods), "未提供已有登录认证方式"
                authenticated = recorder.rpc("authenticate", {
                    "methodId": "cached_token", "_meta": {"headless": True},
                }, 2, jsonrpc=True)
                assert authenticated and "result" in authenticated, "已有登录认证失败"
                params = {"cwd": str(directory), "mcpServers": []}
                if session_id:
                    params["sessionId"] = session_id
                response = recorder.rpc(method, params, 3, jsonrpc=True)
                assert response is not None, "会话请求超时"
                if method == "session/new":
                    assert "result" in response, "空会话新建失败"
                    session_id = response["result"]["sessionId"]
                if "result" in response:
                    # ACP cancel 是无响应通知；只验证该空闲连接随后仍能响应 close。
                    recorder.send({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session_id}})
                    recorder.rpc("session/close", {"sessionId": session_id}, 4, jsonrpc=True)
                print(f"{method}: {'result' if 'result' in response else 'error'}", flush=True)
            finally:
                recorder.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--credential-source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output = args.output.resolve()
    if args.output in {args.executable.resolve(), args.credential_source.resolve()}:
        parser.error("证据输出不得覆盖可执行文件或只读认证来源")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w") as output:
        run(args.executable.resolve(), args.credential_source.resolve(), output)


if __name__ == "__main__":
    main()
