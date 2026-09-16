#!/usr/bin/env python3
"""诊断真实监督二进制的握手与空闲退出；无模型、无用户配置变更。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import uuid


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def host_case(test_binary, supervisor):
    with tempfile.TemporaryDirectory(prefix="infinishell-supervisor-host-") as temporary:
        directory = Path(temporary).resolve()
        env = os.environ.copy()
        env.update(INFINISHELL_MANAGED_PROCESS_FIXTURE="1", INFINISHELL_MANAGED_PROCESS_GENERATION=str(uuid.uuid4()), INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(supervisor))
        process = subprocess.Popen([str(test_binary), "--ignored", "--exact", "ai::cli_agent_runtime::managed_process::live_tests::fixture_host", "--nocapture"], cwd=directory, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 15
            ready = False
            while process.poll() is None and time.monotonic() < deadline:
                ready = (directory / "host-ready").exists() and (directory / "root-heartbeat").exists()
                if ready:
                    (directory / "finish-request").write_text("finish")
                    break
                time.sleep(0.02)
            output, errors = process.communicate(timeout=15)
            receipts = list(directory.rglob("exit.json"))
            receipt = json.loads(receipts[0].read_text()) if len(receipts) == 1 else None
            return {"ready": ready, "exit_code": process.returncode, "stdout": output.decode(errors="replace"), "stderr": errors.decode(errors="replace"), "receipt": receipt, "passed": ready and process.returncode == 0 and bool(receipt and receipt.get("cleanup_confirmed"))}
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


def receive(stream, count):
    data = bytearray()
    while len(data) < count:
        piece = stream.recv(count - len(data))
        if not piece:
            raise RuntimeError("监督控制连接提前关闭")
        data.extend(piece)
    return bytes(data)


def idle_case(supervisor):
    with tempfile.TemporaryDirectory(prefix="infinishell-supervisor-idle-") as temporary:
        directory = Path(temporary).resolve()
        generation, token = uuid.uuid4(), uuid.uuid4()
        state = directory / "cli-agent-processes" / str(generation)
        state.mkdir(parents=True, mode=0o700)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            listener.settimeout(10)
            def argument(value):
                if os.name == "nt":
                    encoded = value.encode("utf-16-le")
                    return {"Windows": [int.from_bytes(encoded[index:index + 2], "little") for index in range(0, len(encoded), 2)]}
                return {"Unix": list(os.fsencode(value))}
            manifest = {"version": 1, "launch_allowed": True, "generation": str(generation), "token": str(token), "parent_control": "127.0.0.1:" + str(listener.getsockname()[1]), "executable": str(Path(sys.executable).resolve()), "arguments": [argument("-c"), argument("pass")], "cwd": str(directory)}
            path = state / "manifest.json"
            path.write_text(json.dumps(manifest))
            path.chmod(0o600)
            process = subprocess.Popen([str(supervisor), "cli-agent-supervisor", str(path)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                control, _ = listener.accept()
                with control:
                    control.settimeout(10)
                    assert receive(control, 32) == generation.bytes + token.bytes
                    control.sendall(b"\x01")
                    started = time.monotonic()
                    ready = receive(control, 1) == b"\x01"
                    delay = time.monotonic() - started
                    process.wait(timeout=15)
                    output, errors = process.communicate()
                receipt = json.loads((state / "exit.json").read_text()) if (state / "exit.json").exists() else None
                return {"ready": ready, "readiness_delay_sec": round(delay, 3), "exit_code": process.returncode, "stdout_bytes": len(output), "stderr": errors.decode(errors="replace"), "receipt": receipt, "passed": ready and process.returncode == 0 and bool(receipt and receipt.get("cleanup_confirmed"))}
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary, supervisor = args.test_binary.resolve(), args.supervisor.resolve()
    cases = []
    for name, callback in (("host_handshake", lambda: host_case(binary, supervisor)), ("blocking_control_idle_exit", lambda: idle_case(supervisor))):
        try:
            cases.append({"case": name, **callback()})
        except Exception as error:
            cases.append({"case": name, "passed": False, "error": str(error)})
    result = {"platform": sys.platform, "test_binary_sha256": digest(binary), "supervisor_sha256": digest(supervisor), "models_requested": False, "cases": cases, "passed": all(case["passed"] for case in cases)}
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": result["passed"], "cases": len(cases)}))
    if not result["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
