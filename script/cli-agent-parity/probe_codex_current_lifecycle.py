#!/usr/bin/env python3
"""以既有登录态验证当前 Codex app-server 生命周期；只输出安全汇总。"""

import argparse
import datetime
import errno
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shlex
import stat
import subprocess
import sys
import tempfile
import time

from probe_protocol import Recorder


MARKERS = {
    "first": "INFINISHELL_CODEX_CURRENT_FIRST",
    "second": "INFINISHELL_CODEX_CURRENT_SECOND",
    "allow": "INFINISHELL_CODEX_CURRENT_ALLOW",
    "deny": "INFINISHELL_CODEX_CURRENT_DENY",
    "steer": "INFINISHELL_CODEX_CURRENT_STEER",
    "resume": "INFINISHELL_CODEX_CURRENT_RESUME",
}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha256_bytes(value):
    return hashlib.sha256(value).hexdigest()


def sha256_file(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def private_auth_copy(source, destination):
    information = os.lstat(source)
    require(stat.S_ISREG(information.st_mode), "Codex 认证入口不是普通文件")
    require(information.st_nlink == 1, "Codex 认证入口存在额外硬链接")
    require(information.st_mode & 0o077 == 0, "Codex 认证入口权限过宽")
    require(0 < information.st_size <= 128 * 1024, "Codex 认证入口大小异常")
    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        opened = os.fstat(descriptor)
        require(
            (opened.st_dev, opened.st_ino, opened.st_size)
            == (information.st_dev, information.st_ino, information.st_size),
            "Codex 认证入口在打开期间发生替换",
        )
        value = bytearray()
        while len(value) <= 128 * 1024:
            chunk = os.read(descriptor, 8192)
            if not chunk:
                break
            value.extend(chunk)
        require(len(value) == information.st_size, "Codex 认证入口读取不完整")
    finally:
        os.close(descriptor)
    target = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        offset = 0
        while offset < len(value):
            offset += os.write(target, value[offset:])
        os.fsync(target)
    finally:
        os.close(target)
    value[:] = b"\0" * len(value)
    return information.st_size


def clean_environment(codex_home):
    environment = os.environ.copy()
    for name in list(environment):
        upper = name.upper()
        if any(fragment in upper for fragment in ("TOKEN", "API_KEY", "AUTH", "SECRET")):
            environment.pop(name)
    environment.update(
        {
            "CODEX_HOME": str(codex_home),
            "CODEX_NON_INTERACTIVE": "1",
        }
    )
    return environment


class LifecycleRecorder(Recorder):
    def __init__(self, command, environment, directory, output):
        self.observed = []
        super().__init__(command, environment, directory, output)

    def next_message(self, timeout):
        try:
            message = self.messages.get(timeout=timeout)
        except queue.Empty:
            return None
        self.observed.append(message)
        return message

    def until(self, predicate, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            message = self.next_message(min(0.2, deadline - time.monotonic()))
            if message is None:
                if self.process.poll() is not None:
                    break
                continue
            if predicate(message):
                return message
        return None


def rpc_result(recorder, method, params, request_id, timeout=30):
    response = recorder.rpc(method, params, request_id, timeout=timeout)
    require(isinstance(response, dict), f"{method} 未返回响应")
    require("error" not in response, f"{method} 返回错误")
    require("result" in response, f"{method} 缺少 result")
    return response["result"]


def initialize(recorder, request_id):
    result = rpc_result(
        recorder,
        "initialize",
        {
            "clientInfo": {
                "name": "infinishell_codex_current_lifecycle",
                "version": "0.1.0",
            }
        },
        request_id,
    )
    recorder.send({"method": "initialized"})
    return result


def server_cli_version(initialized):
    user_agent = initialized.get("userAgent") if isinstance(initialized, dict) else None
    if not isinstance(user_agent, str):
        return None
    matched = re.search(r"(?<!\d)(\d+\.\d+\.\d+)(?!\d)", user_agent)
    return matched.group(1) if matched else None


def item_from(message):
    if not isinstance(message, dict):
        return None
    if message.get("method") not in ("item/started", "item/completed"):
        return None
    params = message.get("params")
    if not isinstance(params, dict):
        return None
    return params.get("item") if isinstance(params.get("item"), dict) else None


def completed_turn(message, thread_id, turn_id):
    if not isinstance(message, dict) or message.get("method") != "turn/completed":
        return None
    params = message.get("params")
    if not isinstance(params, dict) or params.get("threadId") != thread_id:
        return None
    turn = params.get("turn")
    if not isinstance(turn, dict) or turn.get("id") != turn_id:
        return None
    return turn


def exact_command(details, expected, project):
    if not isinstance(details, dict):
        return False
    command = details.get("command")
    if not isinstance(command, str) or details.get("cwd") != str(project):
        return False
    try:
        parsed = shlex.split(command)
        if (
            len(parsed) == 3
            and parsed[0] in ("/bin/zsh", "/bin/bash", "/bin/sh")
            and parsed[1] in ("-lc", "-c")
        ):
            parsed = shlex.split(parsed[2])
        return parsed == expected
    except ValueError:
        return False


def agent_texts(messages, thread_id, turn_id):
    values = []
    for message in messages:
        if not isinstance(message, dict) or message.get("method") != "item/completed":
            continue
        params = message.get("params")
        if not isinstance(params, dict):
            continue
        if params.get("threadId") != thread_id or params.get("turnId") != turn_id:
            continue
        item = params.get("item")
        if isinstance(item, dict) and item.get("type") == "agentMessage":
            text = item.get("text")
            if isinstance(text, str):
                values.append(text)
    return values


def process_exists(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError as error:
        if error.errno == errno.ESRCH:
            return False
        raise


def wait_for_process_exit(pid, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not process_exists(pid):
            return True
        time.sleep(0.05)
    return not process_exists(pid)


def start_turn(recorder, thread_id, prompt, request_id):
    result = rpc_result(
        recorder,
        "turn/start",
        {"threadId": thread_id, "input": [{"type": "text", "text": prompt}]},
        request_id,
        timeout=30,
    )
    turn = result.get("turn") if isinstance(result, dict) else None
    require(isinstance(turn, dict) and isinstance(turn.get("id"), str), "turn/start 未返回 turn id")
    return turn["id"]


def wait_turn(
    recorder,
    thread_id,
    turn_id,
    project,
    *,
    expected_command=None,
    decision=None,
    action=None,
    started_path=None,
    request_id=None,
    timeout=180,
):
    start_index = len(recorder.observed)
    approval = None
    action_result = None
    command_statuses = []
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if action_result is None and action is not None and started_path.exists():
            if action == "steer":
                action_result = rpc_result(
                    recorder,
                    "turn/steer",
                    {
                        "threadId": thread_id,
                        "expectedTurnId": turn_id,
                        "input": [
                            {
                                "type": "text",
                                "text": f"追加要求：最终回复必须包含 {MARKERS['steer']}。",
                            }
                        ],
                    },
                    request_id,
                    timeout=30,
                )
                require(action_result.get("turnId") == turn_id, "turn/steer 未确认当前 turn")
            elif action == "interrupt":
                action_result = rpc_result(
                    recorder,
                    "turn/interrupt",
                    {"threadId": thread_id, "turnId": turn_id},
                    request_id,
                    timeout=30,
                )
            else:
                raise RuntimeError("未知运行中动作")

        message = recorder.next_message(min(0.2, max(0.0, deadline - time.monotonic())))
        if message is None:
            if recorder.process.poll() is not None:
                break
            for observed in recorder.observed[start_index:]:
                turn = completed_turn(observed, thread_id, turn_id)
                if turn is not None:
                    return turn, approval, action_result, command_statuses
            continue

        if (
            isinstance(message, dict)
            and "id" in message
            and message.get("method") == "item/commandExecution/requestApproval"
        ):
            require(approval is None, "同一阶段出现重复命令审批")
            details = message.get("params")
            matched = (
                expected_command is not None
                and exact_command(details, expected_command, project)
                and details.get("threadId") == thread_id
                and details.get("turnId") == turn_id
            )
            require(matched, "命令审批不匹配固定夹具")
            require(decision in ("accept", "decline"), "阶段未声明审批决定")
            recorder.send({"id": message["id"], "result": {"decision": decision}})
            approval = {
                "request_id_sha256": sha256_bytes(str(message["id"]).encode()),
                "item_id_sha256": sha256_bytes(str(details.get("itemId", "")).encode()),
                "decision": decision,
                "exact_fixture": True,
            }

        item = item_from(message)
        if item is not None and item.get("type") == "commandExecution":
            command_statuses.append(
                {
                    "event": message.get("method"),
                    "status": item.get("status"),
                    "exit_code": item.get("exitCode"),
                }
            )

        turn = completed_turn(message, thread_id, turn_id)
        if turn is not None:
            return turn, approval, action_result, command_statuses

    raise RuntimeError("等待 turn/completed 超时或 app-server 提前退出")


def text_phase(recorder, thread_id, project, name, marker, request_id):
    turn_id = start_turn(
        recorder,
        thread_id,
        f"不要调用任何工具，只回复精确文本 {marker}",
        request_id,
    )
    turn, approval, action, commands = wait_turn(recorder, thread_id, turn_id, project)
    texts = agent_texts(recorder.observed, thread_id, turn_id)
    passed = turn.get("status") == "completed" and approval is None and not commands and any(
        marker in text for text in texts
    )
    return {
        "phase": name,
        "turn_id_sha256": sha256_bytes(turn_id.encode()),
        "status": turn.get("status"),
        "marker_observed": any(marker in text for text in texts),
        "approval_count": int(approval is not None),
        "command_event_count": len(commands),
        "passed": passed,
    }


def command_phase(
    recorder,
    thread_id,
    project,
    helper,
    name,
    mode,
    decision,
    request_id,
    *,
    action=None,
    started_path=None,
):
    expected = [sys.executable, str(helper), mode]
    marker = MARKERS.get(name)
    prompt = (
        "这是隔离临时目录的固定生命周期夹具。必须只调用 exec_command 一次，原样执行以下命令，"
        "不要改写命令，不调用其他工具。审批拒绝时不要重试。命令：\n"
        f"{shlex.join(expected)}"
    )
    if marker:
        prompt += f"\n命令结束后最终回复必须包含 {marker}。"
    turn_id = start_turn(recorder, thread_id, prompt, request_id)
    turn, approval, action_result, commands = wait_turn(
        recorder,
        thread_id,
        turn_id,
        project,
        expected_command=expected,
        decision=decision,
        action=action,
        started_path=started_path or Path("/__unused__"),
        request_id=request_id + 1000,
    )
    texts = agent_texts(recorder.observed, thread_id, turn_id)
    return turn_id, turn, approval, action_result, commands, texts


def run(args):
    codex = args.codex.resolve(strict=True)
    auth = args.auth_file.resolve(strict=True)
    source_commit = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=args.repository, text=True
    ).strip()
    source_dirty = bool(
        subprocess.check_output(
            ["git", "status", "--porcelain"], cwd=args.repository, text=True
        ).strip()
    )
    codex_version = subprocess.check_output([str(codex), "--version"], text=True).strip()
    report = {
        "schema_version": 1,
        "captured_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "scope": "Codex 0.155.1 native app-server authenticated lifecycle",
        "source": {
            "commit": source_commit,
            "tree_dirty": source_dirty,
            "probe_sha256": sha256_file(Path(__file__)),
            "codex_version": codex_version,
            "codex_sha256": sha256_file(codex),
        },
        "authentication": {
            "existing_login_reused": True,
            "credential_content_recorded": False,
        },
        "phases": [],
        "result": {
            "native_lifecycle_passed": False,
            "product_adapter_passed": False,
            "application_restart_passed": False,
            "full_goal_passed": False,
        },
        "limitations": [
            "This native probe does not execute the InfiniShell adapter, GUI, managed task persistence, or application restart.",
            "The source working tree is dirty and not a frozen cross-platform candidate.",
        ],
    }
    error = None
    with tempfile.TemporaryDirectory(prefix="infinishell-codex-current-") as temporary:
        root = Path(temporary)
        codex_home = root / "codex-home"
        project = root / "project"
        codex_home.mkdir(mode=0o700)
        project.mkdir(mode=0o700)
        report["authentication"]["credential_bytes_copied_to_temporary_home"] = private_auth_copy(
            auth, codex_home / "auth.json"
        )
        helper = project / "fixture.py"
        helper.write_text(
            """import os\nfrom pathlib import Path\nimport sys\nimport time\n\nmode = sys.argv[1]\nif mode == 'allow':\n    Path('approval-allow.txt').write_text('INFINISHELL_CODEX_CURRENT_ALLOW')\nelif mode == 'deny':\n    Path('approval-deny.txt').write_text('INFINISHELL_CODEX_CURRENT_DENY')\nelif mode == 'steer':\n    Path('steer-started').write_text(str(os.getpid()))\n    time.sleep(8)\nelif mode == 'cancel':\n    Path('cancel-started').write_text(str(os.getpid()))\n    time.sleep(60)\nelse:\n    raise SystemExit(2)\n""",
            encoding="utf-8",
        )
        helper.chmod(0o700)
        transcript = root / "transcript.safe.ndjson"
        environment = clean_environment(codex_home)
        command = [str(codex), "app-server", "--stdio", "--disable", "shell_snapshot"]
        thread_id = None
        first_exit = None
        second_exit = None
        try:
            with transcript.open("w", encoding="utf-8") as evidence:
                recorder = LifecycleRecorder(command, environment, project, evidence)
                try:
                    initialized = initialize(recorder, 1)
                    report["source"]["server_cli_version"] = server_cli_version(initialized)
                    started = rpc_result(
                        recorder,
                        "thread/start",
                        {
                            "cwd": str(project),
                            "approvalPolicy": "untrusted",
                            "sandbox": "workspace-write",
                            "ephemeral": False,
                            "developerInstructions": (
                                "For this isolated lifecycle fixture, do not use tools unless the user gives one exact command. "
                                "When an exact command is given, call exec_command exactly once without rewriting it."
                            ),
                        },
                        2,
                    )
                    thread = started.get("thread") if isinstance(started, dict) else None
                    require(isinstance(thread, dict) and isinstance(thread.get("id"), str), "thread/start 未返回 thread id")
                    thread_id = thread["id"]
                    report["thread_id_sha256"] = sha256_bytes(thread_id.encode())
                    report["phases"].append(
                        text_phase(recorder, thread_id, project, "first", MARKERS["first"], 10)
                    )
                    report["phases"].append(
                        text_phase(recorder, thread_id, project, "second", MARKERS["second"], 11)
                    )

                    turn_id, turn, approval, _, commands, texts = command_phase(
                        recorder, thread_id, project, helper, "allow", "allow", "accept", 12
                    )
                    allow_file = project / "approval-allow.txt"
                    report["phases"].append(
                        {
                            "phase": "approval_allow",
                            "turn_id_sha256": sha256_bytes(turn_id.encode()),
                            "status": turn.get("status"),
                            "approval": approval,
                            "command_events": commands,
                            "file_effect_verified": allow_file.is_file()
                            and allow_file.read_text(encoding="utf-8") == MARKERS["allow"],
                            "marker_observed": any(MARKERS["allow"] in text for text in texts),
                        }
                    )

                    turn_id, turn, approval, _, commands, _ = command_phase(
                        recorder, thread_id, project, helper, "deny", "deny", "decline", 13
                    )
                    report["phases"].append(
                        {
                            "phase": "approval_deny",
                            "turn_id_sha256": sha256_bytes(turn_id.encode()),
                            "status": turn.get("status"),
                            "approval": approval,
                            "command_events": commands,
                            "file_absence_verified": not (project / "approval-deny.txt").exists(),
                        }
                    )

                    turn_id, turn, approval, steer, commands, texts = command_phase(
                        recorder,
                        thread_id,
                        project,
                        helper,
                        "steer",
                        "steer",
                        "accept",
                        14,
                        action="steer",
                        started_path=project / "steer-started",
                    )
                    report["phases"].append(
                        {
                            "phase": "running_steer",
                            "turn_id_sha256": sha256_bytes(turn_id.encode()),
                            "status": turn.get("status"),
                            "approval": approval,
                            "steer_native_ack": isinstance(steer, dict)
                            and steer.get("turnId") == turn_id,
                            "command_events": commands,
                            "marker_observed": any(MARKERS["steer"] in text for text in texts),
                        }
                    )

                    turn_id, turn, approval, interrupted, commands, _ = command_phase(
                        recorder,
                        thread_id,
                        project,
                        helper,
                        "cancel",
                        "cancel",
                        "accept",
                        15,
                        action="interrupt",
                        started_path=project / "cancel-started",
                    )
                    cancel_pid = int((project / "cancel-started").read_text(encoding="utf-8"))
                    report["phases"].append(
                        {
                            "phase": "running_cancel",
                            "turn_id_sha256": sha256_bytes(turn_id.encode()),
                            "status": turn.get("status"),
                            "approval": approval,
                            "interrupt_native_ack": isinstance(interrupted, dict),
                            "command_events": commands,
                            "command_terminal_event_observed": any(
                                event.get("event") == "item/completed" for event in commands
                            ),
                            "fixture_process_exited": wait_for_process_exit(cancel_pid),
                        }
                    )
                finally:
                    recorder.close()
                    first_exit = recorder.process.returncode

                recorder = LifecycleRecorder(command, environment, project, evidence)
                try:
                    initialize(recorder, 101)
                    resumed = rpc_result(
                        recorder,
                        "thread/resume",
                        {
                            "threadId": thread_id,
                            "cwd": str(project),
                            "approvalPolicy": "untrusted",
                            "sandbox": "workspace-write",
                        },
                        102,
                    )
                    resumed_thread = resumed.get("thread") if isinstance(resumed, dict) else None
                    same_thread = isinstance(resumed_thread, dict) and resumed_thread.get("id") == thread_id
                    phase = text_phase(
                        recorder, thread_id, project, "resume", MARKERS["resume"], 103
                    )
                    phase["same_thread_after_server_restart"] = same_thread
                    phase["passed"] = phase["passed"] and same_thread
                    report["phases"].append(phase)
                finally:
                    recorder.close()
                    second_exit = recorder.process.returncode
        except Exception as failure:
            error = f"{type(failure).__name__}: {failure}"

        report["transcript"] = {
            "bytes": transcript.stat().st_size,
            "lines": transcript.read_bytes().count(b"\n"),
            "sha256": sha256_file(transcript),
            "persisted": False,
        }
        report["process_exit_codes"] = [first_exit, second_exit]
        if error is not None:
            report["error"] = error

    phases = {phase["phase"]: phase for phase in report["phases"]}
    native_passed = (
        error is None
        and report.get("source", {}).get("server_cli_version") == "0.155.1"
        and len(phases) == 7
        and phases["first"].get("passed") is True
        and phases["second"].get("passed") is True
        and phases["approval_allow"].get("status") == "completed"
        and phases["approval_allow"].get("approval", {}).get("decision") == "accept"
        and phases["approval_allow"].get("file_effect_verified") is True
        and phases["approval_deny"].get("approval", {}).get("decision") == "decline"
        and phases["approval_deny"].get("file_absence_verified") is True
        and phases["running_steer"].get("status") == "completed"
        and phases["running_steer"].get("steer_native_ack") is True
        and phases["running_steer"].get("marker_observed") is True
        and phases["running_cancel"].get("status") == "interrupted"
        and phases["running_cancel"].get("interrupt_native_ack") is True
        and phases["running_cancel"].get("command_terminal_event_observed") is True
        and phases["running_cancel"].get("fixture_process_exited") is True
        and phases["resume"].get("passed") is True
        and report["process_exit_codes"] == [0, 0]
    )
    report["result"]["native_lifecycle_passed"] = native_passed
    return report


def write_new(path, value):
    encoded = json.dumps(value, ensure_ascii=False, indent=2).encode() + b"\n"
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        offset = 0
        while offset < len(encoded):
            offset += os.write(descriptor, encoded[offset:])
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", type=Path, required=True)
    parser.add_argument("--auth-file", type=Path, required=True)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = run(args)
    write_new(args.output, report)
    print(json.dumps(report["result"], ensure_ascii=False))
    raise SystemExit(0 if report["result"]["native_lifecycle_passed"] else 1)


if __name__ == "__main__":
    main()
