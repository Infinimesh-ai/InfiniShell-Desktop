#!/usr/bin/env python3
"""以内置盘真实 PTY 验证固定 Grok owned 启动和两轮侧车输入；不代替 GUI 验收。"""

import argparse
import ctypes
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import termios
import time

from run_grok_official_adapter_live import copy_private_auth

GROK_SHA = "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d"
TEST = "terminal::cli_agent_sessions::grok_owned_launch::native::live_tests::grok_owned_native_two_turns_and_duplicate_guard"


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def internal(path):
    path = path.resolve(strict=True)
    if str(path).startswith("/Volumes/") or path.stat().st_dev != Path("/Users").stat().st_dev:
        raise RuntimeError("验收输入必须位于内置盘")
    return path


def private_json(path, value):
    with path.open("x", encoding="utf-8") as output:
        os.chmod(path, 0o600)
        json.dump(value, output, ensure_ascii=False, indent=2)
        output.write("\n")


class UniqueIdentity(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_ubyte * 16), ("unique_id", ctypes.c_uint64),
                ("parent_unique_id", ctypes.c_uint64), ("pid_version", ctypes.c_int32),
                ("original_parent_pid_version", ctypes.c_int32), ("reserved", ctypes.c_uint64 * 2)]


class CoalitionIdentity(ctypes.Structure):
    _fields_ = [("ids", ctypes.c_uint64 * 2), ("reserved", ctypes.c_uint64 * 3)]


def process_identity(pid):
    library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    library.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
    unique, coalition = UniqueIdentity(), CoalitionIdentity()
    for flavor, value in ((17, unique), (20, coalition)):
        ctypes.set_errno(0)
        if library.proc_pidinfo(pid, flavor, 0, ctypes.byref(value), ctypes.sizeof(value)) != ctypes.sizeof(value):
            if ctypes.get_errno() == errno.ESRCH:
                return None
            raise RuntimeError("process_identity_unavailable")
    return {"pid": pid, "unique_id": unique.unique_id, "pid_version": unique.pid_version,
            "resource_cid": coalition.ids[0]}


def same_lifetime(expected, actual):
    return actual is not None and all(expected[key] == actual[key] for key in ("pid", "unique_id", "resource_cid"))


def stop_owned(expected):
    for action, timeout in ((signal.SIGTERM, 4), (signal.SIGKILL, 3)):
        current = process_identity(expected["pid"])
        if not same_lifetime(expected, current):
            return True
        os.kill(expected["pid"], action)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if not same_lifetime(expected, process_identity(expected["pid"])):
                return True
            time.sleep(0.05)
    return False


def close_pty_and_reap_shell(master, shell):
    # macOS 可在退出时等待 PTY 主端关闭；所有 wait 均有界，不能把 close 放在无界 wait 之后。
    if master is not None:
        os.close(master)
    if shell is None:
        return False
    try:
        shell.wait(timeout=4)
        return True
    except subprocess.TimeoutExpired:
        shell.kill()
        try:
            shell.wait(timeout=4)
            return True
        except subprocess.TimeoutExpired:
            return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--app-binary", type=Path, required=True)
    parser.add_argument("--grok", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    root = args.output
    root.mkdir(parents=True, exist_ok=False, mode=0o700)
    root = internal(root)
    for name in ("home/.grok/hooks", "project", "tmp", "bin"):
        (root / name).mkdir(parents=True, mode=0o700)
    (root / ".owned-live").write_text("grok-owned-native-v1\n")
    test_source, app_source, grok = map(internal, (args.test_binary, args.app_binary, args.grok))
    assert digest(grok) == GROK_SHA
    binaries = {}
    for name, source in (("test", test_source), ("infinishell", app_source)):
        target = root / "bin" / name
        shutil.copy2(source, target)
        assert digest(target) == digest(source)
        subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(target)], check=True, capture_output=True)
        binaries[name] = {"source": str(source), "path": str(target), "sha256": digest(target)}
    env = {key: value for key, value in os.environ.items() if key in ("PATH", "USER", "LOGNAME", "LANG")}
    env.update(HOME=str(root / "home"), GROK_HOME=str(root / "home/.grok"),
               TERM="xterm-256color", TMPDIR=str(root / "tmp"), SHELL="/bin/zsh",
               PS1="OWNED_SHELL_READY> ", PS2="OWNED_MORE> ",
               XDG_CONFIG_HOME=str(root / "home/.config"), XDG_DATA_HOME=str(root / "home/.local/share"),
               XDG_CACHE_HOME=str(root / "home/.cache"), GROK_CLAUDE_HOOKS_ENABLED="0",
               GROK_CLAUDE_MCPS_ENABLED="0", GROK_CODEX_HOOKS_ENABLED="0", GROK_CODEX_MCPS_ENABLED="0",
               GROK_DISABLE_API_KEY_AUTH="1", GROK_DISABLE_AUTOUPDATER="1",
               WARP_CLI_AGENT_PROTOCOL_VERSION="1", GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null")
    version = subprocess.run([str(grok), "--version"], env=env, capture_output=True, check=True)
    assert version.stdout.decode().strip() == "grok 1.0.41 (4220f3b224a6)"
    (root / "version.stdout").write_bytes(version.stdout)
    (root / "home/.grok/config.toml").write_text('[cli]\nauto_update=false\n[models]\ndefault="grok-4.7"\n[features]\nturn_summary=false\ntitle_refresh=false\n')
    mapper = Path(__file__).resolve().parents[2] / "app/assets/bundled/cli-agent-plugins/grok/hooks/notify.cjs"
    mapper = internal(mapper)
    node = internal(Path(shutil.which("node")))
    observer = root / "observe.cjs"
    observer.write_text('''"use strict";
const fs = require("node:fs");
const mapper = require(process.argv[2]);
const raw = fs.readFileSync(0, "utf8");
if (Buffer.byteLength(raw) > 1048576) process.exit(1);
const value = JSON.parse(raw);
const notification = mapper.makeNotification(mapper.normalize(value, process.env));
if (notification) fs.appendFileSync(process.argv[3], JSON.stringify(notification) + "\\n", { mode: 0o600 });
''')
    hook_command = shlex.join([str(node), str(observer), str(mapper), str(root / "hooks.ndjson")])
    hooks = {name: [{"hooks": [{"type": "command", "command": hook_command, "timeout": 3}]}]
             for name in ("SessionStart", "UserPromptSubmit", "Stop", "StopCancelled", "PermissionDenied")}
    private_json(root / "home/.grok/hooks/owned-probe.json", {"hooks": hooks})
    private_json(root / "source.safe.json", {"binaries": binaries, "grok_sha256": GROK_SHA,
        "mapper_sha256": digest(mapper), "runner_sha256": digest(Path(__file__)), "test": TEST,
        "scope": "真实原生 PTY 加生产启动、侧车、SQLite；hook 使用生产 mapper，未经过 GUI OSC 接收", "gui_verified": False})
    auth = None
    shell = None
    test = None
    master = None
    child_pid = None
    leader_pid = None
    result = {"passed": False, "model_request_count_expected": 2, "pty_prompt_submit_count": 0}
    started = time.monotonic()
    raw_output = root / "pty.raw.bin"
    launch = None
    try:
        auth = copy_private_auth(Path.home() / ".grok", root / "home/.grok")
        master, slave = pty.openpty()
        device = os.fstat(slave).st_rdev
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 46, 160, 0, 0))

        def setup_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        shell = subprocess.Popen(["/bin/zsh", "-dfi"], cwd=root / "project", env=env,
            stdin=slave, stdout=slave, stderr=slave, preexec_fn=setup_terminal, close_fds=True)
        os.close(slave)
        ready = False
        received = b""
        with raw_output.open("xb") as output:
            while time.monotonic() - started < 320:
                readable, _, _ = select.select([master], [], [], 0.05)
                if readable:
                    try:
                        data = os.read(master, 65536)
                    except OSError:
                        data = b""
                    if not data:
                        raise RuntimeError("pty_eof")
                    # 认证失效只报告分类，禁止把授权材料落入原始收据。
                    if re.search(rb"Bearer\s+[A-Za-z0-9._-]+|(?:access_token|refresh_token|device_code|user_code)\s*[=:]", data):
                        raise RuntimeError("authentication_material_in_output")
                    output.write(data)
                    output.flush()
                    received = (received + data)[-262144:]
                    if b"\x1b[6n" in data:
                        os.write(master, b"\x1b[1;1R")
                    if not ready and b"OWNED_SHELL_READY> " in received:
                        ready = True
                        private_json(root / "pty.json", {"shell_pid": shell.pid, "slave_device": device, "master_fd": master})
                        test_env = env.copy()
                        test_env.update(INFINISHELL_GROK_OWNED_LIVE_ROOT=str(root),
                            INFINISHELL_GROK_LIVE_EXECUTABLE=str(grok),
                            INFINISHELL_GROK_OWNED_APP_EXECUTABLE=str(root / "bin/infinishell"))
                        with (root / "test.log").open("xb") as log:
                            test = subprocess.Popen([str(root / "bin/test"), TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                env=test_env, cwd=root, stdout=log, stderr=subprocess.STDOUT, start_new_session=True, pass_fds=(master,))
                command_path = root / "launch-command.json"
                if launch is None and command_path.exists():
                    launch = json.loads(command_path.read_text())
                    assert launch["argv"][0] == str(root / "bin/infinishell")
                    assert launch["argv"][1] == "--infinishell-owned-grok-tui"
                    assert len(launch["argv"]) == 3
                    os.write(master, (shlex.join(launch["argv"]) + "\r").encode())
                    result["pty_launch_command_count"] = 1
                if launch:
                    directory = Path(launch["manifest_path"]).parent
                    receipt = directory / "exec.json"
                    if receipt.exists():
                        child_pid = json.loads(receipt.read_text())["process"]["pid"]
                    bound = directory / "bound.json"
                    if bound.exists():
                        leader_pid = json.loads(bound.read_text())["leader"]["pid"]
                if test and test.poll() is not None:
                    result["test_exit_code"] = test.returncode
                    result["passed"] = test.returncode == 0 and (root / "native-result.json").exists()
                    break
            else:
                raise TimeoutError("native_probe_timeout")
    except BaseException as error:
        result["failure_class"] = type(error).__name__
        result["failure"] = str(error) if isinstance(error, (TimeoutError, RuntimeError)) else "probe_setup_or_execution_failed"
    finally:
        if test and test.poll() is None:
            os.killpg(test.pid, signal.SIGTERM)
            try:
                test.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(test.pid, signal.SIGKILL)
                test.wait()
        # 原生进程以本次内核生存期清理；PID、路径或 socket 名单独不能授予终止资格。
        cleanup = {"tui_exited": False, "leader_exited": False}
        if launch:
            directory = Path(launch["manifest_path"]).parent
            socket_path = directory / "leader.sock"
            try:
                receipt_path, bound_path = directory / "exec.json", directory / "bound.json"
                receipt = json.loads(receipt_path.read_text()) if receipt_path.exists() else None
                bound = json.loads(bound_path.read_text()) if bound_path.exists() else None
                leader = bound["leader"] if bound else None
                if leader is None and socket_path.exists():
                    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as peer:
                        peer.settimeout(1)
                        peer.connect(str(socket_path))
                        token = struct.unpack("=8I", peer.getsockopt(0, 6, 32))
                        assert token[1] == os.getuid()
                        leader = process_identity(token[5])
                        assert leader and leader["pid_version"] == token[7]
                        command = subprocess.run(["ps", "-p", str(leader["pid"]), "-o", "command="], capture_output=True, text=True)
                        assert str(grok) + " agent leader" in command.stdout and str(socket_path) in command.stdout
                if receipt:
                    cleanup["tui_exited"] = stop_owned(receipt["process"])
                if leader:
                    leader_pid = leader["pid"]
                    cleanup["leader_exited"] = stop_owned(leader)
            except BaseException as error:
                cleanup["failure_class"] = type(error).__name__
            for name in ("launch.json", "dispatched", "exec.json", "exec-failed.json", "bound.json"):
                source = directory / name
                if source.exists():
                    shutil.copy2(source, root / ("launch-" + name))
        result["cleanup"] = cleanup
        result["passed"] = result["passed"] and cleanup["tui_exited"] and cleanup["leader_exited"]
        result["shell_reaped"] = close_pty_and_reap_shell(master, shell)
        master = None
        if auth:
            auth.unlink(missing_ok=True)
            result["private_auth_removed"] = not auth.exists()
        result["passed"] = result["passed"] and result["shell_reaped"]
        result["seconds"] = round(time.monotonic() - started, 2)
        result["leader_pid"] = leader_pid
        private_json(root / "runner-result.json", result)
        print(json.dumps({"root": str(root), **result}, ensure_ascii=False), flush=True)
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
