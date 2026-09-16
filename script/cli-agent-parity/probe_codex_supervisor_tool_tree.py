#!/usr/bin/env python3
"""真实 Codex 工具运行时强杀隔离宿主，核对监督回执与心跳；不修改产品源码。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import time
import uuid

from probe_protocol import Recorder
from probe_supervisor_startup import receive


HEARTBEAT = '''import json,os,pathlib,subprocess,time
root=pathlib.Path(__file__).resolve().parent
child=subprocess.Popen(["/bin/sleep","60"],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
identity={"pid":os.getpid(),"ppid":os.getppid(),"pgid":os.getpgid(0),"sid":os.getsid(0),"sleep_pid":child.pid}
(root/"tool-identity.json").write_text(json.dumps(identity))
deadline=time.monotonic()+60
try:
    with (root/"heartbeat").open("a") as output:
        while time.monotonic()<deadline and not (root/"fixture-stop").exists():
            output.write(str(time.monotonic_ns())+"\\n"); output.flush(); os.fsync(output.fileno()); time.sleep(.1)
finally:
    if child.poll() is None: child.terminate()
    child.wait()
    (root/"fixture-finished").write_text("finished")
'''


def write(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    temporary.replace(path)


def safe_approval(details, expected, project):
    command = details.get("command", "")
    try:
        argv = shlex.split(command)
    except ValueError:
        return False
    if len(argv) == 3 and argv[0] in ("/bin/zsh", "/bin/bash", "/bin/sh") and argv[1] in ("-lc", "-c"):
        argv = shlex.split(argv[2])
    actions = details.get("commandActions", [])
    return argv == expected and details.get("cwd") == str(project) and len(actions) == 1 and shlex.split(actions[0].get("command", "")) == expected


def host(configuration):
    options = json.loads(configuration.read_text())
    root, project = Path(options["root"]), Path(options["root"]) / "project"
    generation, token = uuid.UUID(options["generation"]), uuid.uuid4()
    state = root / "cli-agent-processes" / str(generation)
    state.mkdir(parents=True, mode=0o700)
    with socket.socket() as listener, (root / "protocol.ndjson").open("w") as evidence:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        listener.settimeout(10)
        args = ["app-server", "--stdio", "--disable", "shell_snapshot"]
        manifest = {"version": 1, "launch_allowed": True, "generation": str(generation), "token": str(token), "parent_control": "127.0.0.1:" + str(listener.getsockname()[1]), "executable": options["codex"], "arguments": [{"Unix": list(os.fsencode(value))} for value in args], "cwd": str(project)}
        manifest_path = state / "manifest.json"
        write(manifest_path, manifest)
        manifest_path.chmod(0o600)
        recorder = Recorder([options["supervisor"], "cli-agent-supervisor", str(manifest_path)], os.environ.copy(), project, evidence)
        write(root / "worker.json", {"pid": recorder.process.pid, "host_pid": os.getpid()})
        control, _ = listener.accept()
        with control:
            control.settimeout(10)
            if receive(control, 32) != generation.bytes + token.bytes:
                raise RuntimeError("监督握手身份不匹配")
            control.sendall(b"\x01")
            if receive(control, 1) != b"\x01":
                raise RuntimeError("监督进程没有就绪")
            response = recorder.rpc("initialize", {"clientInfo": {"name": "infinishell_supervised_tool_probe", "version": "0.1.0"}}, 1)
            if not response or "result" not in response:
                raise RuntimeError("真实 Codex 初始化失败")
            recorder.send({"method": "initialized"})
            response = recorder.rpc("thread/start", {"cwd": str(project), "approvalPolicy": "untrusted", "sandbox": "workspace-write", "ephemeral": True}, 2)
            if not response or "result" not in response:
                raise RuntimeError("真实 Codex 新建会话失败")
            thread_id = response["result"]["thread"]["id"]
            expected = [options["python"], "heartbeat.py"]
            command = shlex.join(expected)
            prompt = "这是隔离临时项目里的进程清理测试。必须只调用 exec_command 一次，原样执行以下固定命令，不改文件、不检查其他目录、不用其他工具；获准后保持进程运行，不主动停止或重试。拒绝则结束。命令：\n" + command
            response = recorder.rpc("turn/start", {"threadId": thread_id, "input": [{"type": "text", "text": prompt}]}, 3)
            if not response or "result" not in response:
                raise RuntimeError("真实 Codex 输入失败")
            write(root / "native.json", {"thread_id": thread_id, "turn_id": response["result"]["turn"]["id"]})
            approved = 0
            deadline = time.monotonic() + 100
            while time.monotonic() < deadline:
                try:
                    message = recorder.messages.get(timeout=.1)
                except queue.Empty:
                    if recorder.process.poll() is not None:
                        break
                    continue
                if isinstance(message, dict) and "id" in message and message.get("method", "").endswith("/requestApproval"):
                    safe = message["method"] == "item/commandExecution/requestApproval" and approved == 0 and safe_approval(message.get("params", {}), expected, project)
                    recorder.send({"id": message["id"], "result": {"decision": "accept" if safe else "decline"}})
                    approved += int(safe)
                    write(root / "approval.json", {"accepted": approved, "last_request_matched_exact_fixture": safe})
            # CLI 崩溃分支保留宿主，避免把随后宿主退出混入原生崩溃的清理证据。
            write(root / "worker-exited.json", {"exit_code": recorder.process.poll()})
            while time.monotonic() < deadline:
                time.sleep(.1)


def process_rows(pids):
    rows = []
    for pid in sorted(set(pids)):
        result = subprocess.run(["ps", "-p", str(pid), "-o", "pid=,ppid=,pgid=,stat=,lstart=,comm="], capture_output=True, text=True)
        if not result.stdout.strip():
            rows.append({"pid": pid, "present": False})
            continue
        parts = result.stdout.strip().split(None, 9)
        try:
            sid = os.getsid(pid)
        except ProcessLookupError:
            sid = None
        rows.append({"pid": pid, "ppid": int(parts[1]), "pgid": int(parts[2]), "sid": sid, "state": parts[3], "start": " ".join(parts[4:9]), "command": parts[9] if len(parts) > 9 else "", "present": True})
    return rows


def ancestry(pids, boundary):
    found = set(pids)
    pending = list(pids)
    while pending:
        pid = pending.pop()
        row = process_rows([pid])[0]
        parent = row.get("ppid", 1)
        if pid != boundary and parent > 1 and parent not in found:
            found.add(parent)
            pending.append(parent)
    return process_rows(found)


def sample(path):
    data = path.read_bytes() if path.exists() else b""
    return {"bytes": len(data), "lines": data.count(b"\n"), "sha256": hashlib.sha256(data).hexdigest()}


def classify_cleanup(report):
    receipt = report.get("receipt")
    identity = report.get("tool_identity", {})
    expected = {identity.get("pid"), identity.get("sleep_pid")}
    rows = [row for row in report.get("after_observation", []) if row["pid"] in expected]
    stopped = None not in expected and len(rows) == 2 and all(not row["present"] or row.get("state", "").startswith("Z") for row in rows)
    stopped = stopped and report.get("heartbeat_continued_after_receipt") is False
    associated = bool(receipt and report.get("manifest_digest_matches") and receipt.get("generation") == report.get("generation"))
    confirmed = associated and receipt.get("cleanup_confirmed") is True
    return {"known_tool_processes_stopped": stopped, "cleanup_failed": not (confirmed and stopped),
            "unsafe_recovery_prevented": associated and receipt.get("cleanup_confirmed") is False,
            "recovery_verification_scope": "receipt_field_only; Rust confirmed_exit gate tested separately",
            "passed": confirmed and stopped}


def run(args):
    report = {"platform": sys.platform, "scope": "real_codex_tool_via_supervisor_custom_host", "crash_target": args.crash_target, "rust_adapter_tested": False, "model_requested": True, "credentials_isolated": True, "supervisor_sha256": hashlib.sha256(args.supervisor.read_bytes()).hexdigest()}
    version = subprocess.run([str(args.codex), "--version"], capture_output=True, text=True, check=True, timeout=5).stdout.strip()
    if version != "codex-cli 0.147.0":
        raise RuntimeError("只接受固定 Codex 0.147.0")
    report["cli_version"] = version
    with tempfile.TemporaryDirectory(prefix="infinishell-codex-tree-") as temporary:
        root = Path(temporary).resolve()
        project = root / "project"
        project.mkdir()
        home = root / "home"
        home.mkdir()
        configuration = root / "codex"
        configuration.mkdir(mode=0o700)
        credentials = configuration / "auth.json"
        with credentials.open("xb") as target:
            credentials.chmod(0o600)
            target.write(args.credential_source.read_bytes())
        environment = {key: value for key, value in os.environ.items() if key in ("PATH", "TMPDIR", "LANG", "LC_ALL")}
        environment.update(HOME=str(home), CODEX_HOME=str(configuration))
        (project / "heartbeat.py").write_text(HEARTBEAT)
        generation = str(uuid.uuid4())
        options = root / "host-config.json"
        write(options, {"root": str(root), "generation": generation, "codex": str(args.codex), "supervisor": str(args.supervisor), "python": str(Path(sys.executable).resolve())})
        state = root / "cli-agent-processes" / generation
        identity = {}
        with (root / "host-stderr").open("wb") as errors:
            process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--host", str(options)], env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=errors)
            try:
                deadline = time.monotonic() + 90
                while time.monotonic() < deadline and process.poll() is None:
                    if (project / "tool-identity.json").exists() and sample(project / "heartbeat")["lines"] >= 3:
                        break
                    time.sleep(.1)
                identity = json.loads((project / "tool-identity.json").read_text())
                worker = json.loads((root / "worker.json").read_text())
                report.update(native=json.loads((root / "native.json").read_text()), generation=generation, tool_identity=identity, worker=worker)
                report["approval"] = json.loads((root / "approval.json").read_text())
                report["before"] = ancestry([identity["pid"], identity["sleep_pid"], worker["pid"]], process.pid)
                report["heartbeat_before_kill"] = sample(project / "heartbeat")
                start = time.monotonic()
                if args.crash_target == "host":
                    process.kill()
                    report["host_exit"] = process.wait(timeout=5)
                    report["host_sigkill_sent"] = True
                else:
                    candidates = [row for row in report["before"] if row.get("ppid") == worker["pid"] and row.get("command") == str(args.codex)]
                    if len(candidates) != 1:
                        raise RuntimeError("无法无歧义定位本次监督者拥有的真实 Codex 根进程")
                    cli = candidates[0]
                    if process_rows([cli["pid"]])[0] != cli:
                        raise RuntimeError("强制终止前 Codex 进程身份已变化")
                    report["host_alive_at_cli_kill"] = process.poll() is None
                    os.kill(cli["pid"], signal.SIGKILL)
                    report["cli_sigkill_sent"] = True
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline and not (state / "exit.json").exists():
                    time.sleep(.02)
                receipt_path = state / "exit.json"
                report["receipt_observed_after_sec"] = round(time.monotonic() - start, 3)
                report["receipt"] = json.loads(receipt_path.read_text()) if receipt_path.exists() else None
                report["heartbeat_at_receipt"] = sample(project / "heartbeat")
                report["after_receipt"] = process_rows([row["pid"] for row in report["before"]])
                report["host_alive_after_receipt"] = process.poll() is None
                time.sleep(1.2)
                report["heartbeat_later"] = sample(project / "heartbeat")
                report["after_observation"] = process_rows([row["pid"] for row in report["before"]])
                report["heartbeat_continued_after_receipt"] = report["heartbeat_later"]["lines"] > report["heartbeat_at_receipt"]["lines"]
                report["manifest_digest_matches"] = bool(report["receipt"] and report["receipt"]["manifest_sha256"] == hashlib.sha256((state / "manifest.json").read_bytes()).hexdigest())
                report["tool_has_distinct_group_from_cli"] = identity["pgid"] not in [row["pgid"] for row in report["before"] if row.get("ppid") == worker["pid"]]
                report.update(classify_cleanup(report))
            except Exception as error:
                report.update(error=str(error), passed=False)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
                # 只停止本次已知夹具；正常停止标记避免按可能复用的 PID/PGID 发全局 kill。
                (project / "fixture-stop").write_text("stop")
                if identity:
                    deadline = time.monotonic() + 5
                    while time.monotonic() < deadline:
                        rows = process_rows([identity["pid"], identity["sleep_pid"]])
                        if all(not row["present"] or row.get("state", "").startswith("Z") for row in rows):
                            break
                        time.sleep(.05)
                    report["fixture_cleanup"] = rows
        report["host_stderr"] = (root / "host-stderr").read_text(errors="replace")
        report["supervisor_unchanged_during_probe"] = hashlib.sha256(args.supervisor.read_bytes()).hexdigest() == report["supervisor_sha256"]
        cleaner = Recorder.__new__(Recorder)
        cleaner.directory = str(root)
        protocol = root / "protocol.ndjson"
        events = []
        if protocol.exists():
            for line in protocol.read_text().splitlines():
                item = json.loads(line)
                message = item.get("message")
                if isinstance(message, dict) and (message.get("method") in ("item/started", "item/completed", "item/commandExecution/requestApproval", "turn/started", "turn/completed", "error") or "result" in message and "decision" in message["result"]):
                    events.append(item)
        report["protocol_events"] = events
        report = cleaner.clean(report)
    report["passed"] = report.get("passed", False) and report["supervisor_unchanged_during_probe"]
    write(args.output, report)
    print(json.dumps({key: report.get(key) for key in ("passed", "cleanup_failed", "unsafe_recovery_prevented", "tool_has_distinct_group_from_cli", "heartbeat_continued_after_receipt", "receipt", "error")}))
    return report["passed"]


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--host":
        host(Path(sys.argv[2]))
        return
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("codex", "supervisor", "credential-source", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--crash-target", choices=("host", "codex"), default="host")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("本次定向 probe 仅验证 macOS；其他平台须独立执行对应验收")
    for name in ("codex", "supervisor", "credential_source", "output"):
        setattr(args, name, getattr(args, name).resolve())
    if not run(args):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
