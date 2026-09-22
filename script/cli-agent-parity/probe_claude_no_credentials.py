#!/usr/bin/env python3
"""无凭据验证固定 Claude 的 initialize 与空闲 stdin EOF；不创建模型回合或验证产品适配器。"""

import argparse
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import sys
import tempfile
import threading
import time
import uuid

sys.dont_write_bytecode = True
from prepare_claude_cli import (DEFAULT_VERSION, RELEASE_CATALOG, current_platform,
                                isolated_environment, regular_file, require, verify_binary,
                                verify_version)


INITIALIZE_TIMEOUT = 30
EOF_TIMEOUT = 5
MAX_LINE_CHARACTERS = 1024 * 1024
MAX_RECORDS = 256


def command(executable):
    return [str(executable), "--print", "--input-format", "stream-json", "--output-format",
            "stream-json", "--verbose", "--permission-prompt-tool", "stdio", "--permission-prompts",
            "host", "--setting-sources", "", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}']


def validate_initialize(message, request_id, pid):
    require(isinstance(message, dict) and message.get("type") == "control_response",
            "只允许 initialize 控制响应；拒绝 user、模型、回合或未知输出")
    response = message.get("response")
    require(isinstance(response, dict) and response.get("request_id") == request_id,
            "initialize 响应未关联本次请求")
    require(response.get("subtype") == "success", "原生 initialize 没有成功")
    value = response.get("response")
    require(isinstance(value, dict) and value.get("pid") == pid and type(value.get("pid")) is int,
            "initialize 没有确认本次原生进程 PID")
    account = value.get("account")
    require(isinstance(account, dict) and account.get("tokenSource") == "none"
            and account.get("apiProvider") == "firstParty", "原生响应没有证明无账号的默认提供方")
    require(value.get("session_state") == "idle" and value.get("current_permission_mode") == "default",
            "原生进程没有确认默认权限的空闲状态")
    require(response.get("pending_permission_requests") == [] and response.get("pending_user_dialog_requests") == [],
            "空闲初始化出现了待审批或待用户输入")
    require(not message.get("session_id") and not value.get("session_id"), "控制握手意外变成原生会话关联")


def clean(value, directory):
    if isinstance(value, dict):
        return {key: {"catalog_omitted": True, "count": len(item)} if key in ("commands", "agents", "models") and isinstance(item, list)
                else "<redacted>" if re.search(r"token$|api.?key|authorization|email|secret", key, re.IGNORECASE)
                else clean(item, directory) for key, item in value.items()}
    if isinstance(value, list):
        return [clean(item, directory) for item in value]
    if isinstance(value, str):
        for root in (str(directory), str(directory.resolve())):
            value = value.replace(root, "<isolated-probe>")
        return re.sub(r"(?:sk-[A-Za-z0-9_-]{16,}|Bearer [A-Za-z0-9_.-]+)", "<redacted>", value)
    return value


class Recorder:
    def __init__(self, arguments, env, directory):
        self.process = subprocess.Popen(arguments, env=env, cwd=directory, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                        encoding="utf-8", errors="strict", bufsize=1)
        self.messages = queue.Queue()
        self.records = []
        self.reader_errors = []
        self.lock = threading.Lock()
        self.readers = []
        for channel in ("stdout", "stderr"):
            reader = threading.Thread(target=self.read, args=(channel,), daemon=True)
            reader.start()
            self.readers.append(reader)

    def read(self, channel):
        stream = getattr(self.process, channel)
        try:
            while line := stream.readline(MAX_LINE_CHARACTERS + 1):
                require(len(line) <= MAX_LINE_CHARACTERS, "原生输出行超过探测上限")
                try:
                    value = json.loads(line)
                except json.JSONDecodeError:
                    value = line.rstrip("\r\n")
                with self.lock:
                    require(len(self.records) < MAX_RECORDS, "原生输出超过探测记录上限")
                    self.records.append({"direction": channel, "message": value})
                if channel == "stdout":
                    self.messages.put(value)
        except Exception as error:
            self.reader_errors.append(str(error))

    def initialize(self, request_id):
        request = {"type": "control_request", "request_id": request_id, "request": {"subtype": "initialize"}}
        with self.lock:
            self.records.append({"direction": "stdin", "message": request})
        self.process.stdin.write(json.dumps(request) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + INITIALIZE_TIMEOUT
        while time.monotonic() < deadline:
            require(not self.reader_errors, "读取原生输出失败：" + repr(self.reader_errors))
            try:
                return self.messages.get(timeout=0.1)
            except queue.Empty:
                if self.process.poll() is not None:
                    break
        raise ValueError("initialize 没有在期限内返回原生响应")

    def finish_eof(self):
        require(self.process.poll() is None, "发送 stdin EOF 前原生进程已经退出")
        before = time.monotonic()
        self.process.stdin.close()
        try:
            self.process.wait(timeout=EOF_TIMEOUT)
            exited = True
        except subprocess.TimeoutExpired:
            exited = False
        return {"alive_before_stdin_eof": True, "stdin_eof_exited_within_5s": exited, "exit_code_before_cleanup": self.process.returncode,
                "eof_elapsed_ms": round((time.monotonic() - before) * 1000)}

    def close(self):
        forced = False
        try:
            self.process.stdin.close()
        except (BrokenPipeError, OSError):
            pass
        if self.process.poll() is None:
            forced = True
            # 仅清理本探测持有的原生子进程句柄；强制退出不计入 EOF 自行退出结果。
            self.process.kill()
        try:
            self.process.wait(timeout=5)
            for reader in self.readers:
                reader.join(timeout=2)
            require(not self.reader_errors and all(not reader.is_alive() for reader in self.readers),
                    "原生输出没有完整读取：" + repr(self.reader_errors))
        finally:
            # 读取线程未结束时不抢占其 TextIO 锁；该情况已明确失败，不能在 close 中无限等待。
            for channel, reader in zip(("stdout", "stderr"), self.readers):
                if not reader.is_alive():
                    getattr(self.process, channel).close()
        return forced


def validate_transcript(records, request_id, pid):
    require(all(not isinstance(row["message"], dict) or row["message"].get("type")
                not in ("user", "assistant", "result", "stream_event") for row in records),
            "任意通道都不能出现 user、模型输出或模型结果")
    requests = [row["message"] for row in records if row["direction"] == "stdin"]
    require(requests == [{"type": "control_request", "request_id": request_id, "request": {"subtype": "initialize"}}],
            "探测只能发送一次 initialize，不能发送 user 或模型请求")
    outputs = [row["message"] for row in records if row["direction"] == "stdout"]
    require(len(outputs) == 1, "只能收到一次 initialize 响应；额外输出或重复响应不计成功")
    validate_initialize(outputs[0], request_id, pid)


def repository_identity(env):
    repository = Path(__file__).resolve().parents[2]
    result = subprocess.run(["git", "rev-parse", "HEAD"], cwd=repository, env=env,
                            capture_output=True, text=True, encoding="utf-8", timeout=10, check=True)
    status = subprocess.run(["git", "status", "--porcelain"], cwd=repository, env=env,
                            capture_output=True, text=True, encoding="utf-8", timeout=10, check=True)
    return {"repository_commit": result.stdout.strip(), "worktree_dirty": bool(status.stdout.strip())}


def run(executable, root, report, version):
    env = isolated_environment(root)
    report.update(repository_identity(env))
    report["binary"] = verify_binary(executable, current_platform(), version)
    report["cli_version"] = verify_version(executable, root, version)
    project = root / "project"
    project.mkdir()
    recorder = Recorder(command(executable), env, project)
    request_id = f"infinishell-initialize-{uuid.uuid4()}"
    report.update(native_pid=recorder.process.pid, request_id=request_id)
    try:
        response = recorder.initialize(request_id)
        validate_initialize(response, request_id, recorder.process.pid)
        report["initialize_confirmed"] = True
        report.update(recorder.finish_eof())
    finally:
        try:
            report["forced_cleanup_used"] = recorder.close()
        finally:
            report["events"] = clean(recorder.records, root)
    validate_transcript(recorder.records, request_id, recorder.process.pid)
    require(report["stdin_eof_exited_within_5s"] and report["exit_code_before_cleanup"] == 0
            and not report["forced_cleanup_used"], "原生空闲 EOF 没有自行正常退出")
    require(not (root / "claude/.credentials.json").exists(), "无凭据探测意外生成了账号凭据文件")
    require(verify_binary(executable, current_platform(), version) == report["binary"], "探测期间原生文件改变")
    report["binary_unchanged_after_probe"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claude-version", choices=tuple(RELEASE_CATALOG), default=DEFAULT_VERSION,
                        help=f"与准备器相同的精确官方版本；缺省为 {DEFAULT_VERSION}")
    parser.add_argument("--executable", type=Path, required=True, help="固定摘要的原生 Claude 绝对路径，不接受安装器或脚本包装")
    parser.add_argument("--output", type=Path, required=True, help="源树外的 JSON 验证记录")
    args = parser.parse_args()
    require(args.executable.is_absolute() and args.output.is_absolute(), "可执行文件和输出必须使用绝对路径")
    regular_file(args.executable)
    executable = args.executable.resolve(strict=True)
    output = args.output.resolve()
    repository = Path(__file__).resolve().parents[2]
    require(output != executable and not output.is_relative_to(repository) and not args.output.is_symlink(),
            "输出不能覆盖可执行文件、符号链接或仓库文件")
    if output.exists():
        regular_file(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    report = {"passed": False, "mode": "claude_initialize_and_idle_eof", "host_os": sys.platform,
              "expected_version": args.claude_version, "credentials_provided": False, "model_commands_sent": 0,
              "native_session_association_confirmed": False, "production_rust_adapter_verified": False,
              "model_lifecycle_verified": False, "active_turn_tested": False, "tool_descendant_tested": False,
              "parent_sigkill_tested": False, "app_restart_and_ui_verified": False, "events": []}
    try:
        with tempfile.TemporaryDirectory(prefix="infinishell-claude-no-credentials-", dir=os.environ.get("RUNNER_TEMP")) as temporary:
            root = Path(temporary).resolve()
            require(not root.is_relative_to(repository), "临时配置必须位于源树外")
            run(executable, root, report, args.claude_version)
        report["passed"] = True
    except Exception as error:
        report["failure"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"claude_initialize_idle_eof_passed": report["passed"], "evidence": str(output)}, ensure_ascii=False))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
