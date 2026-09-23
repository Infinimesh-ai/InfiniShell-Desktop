"""核对已安装 Grok hook 的真实 Unix 控制终端输出；不运行模型。"""

import hashlib
import json
import os
from pathlib import Path
import selectors
import stat
import subprocess
import time


def plain_file(path, expected_sha256, root=None, executable=False):
    path = Path(path)
    info = path.lstat()
    # 固定摘要的系统 Node 可由 root 安装；私有 hook 仍只能属于当前用户。
    owner_valid = info.st_uid == os.getuid()
    if executable:
        owner_valid = (info.st_uid in (0, os.getuid())
                       and not stat.S_IMODE(info.st_mode) & 0o022
                       and bool(stat.S_IMODE(info.st_mode) & 0o111))
    if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
            or not owner_valid or path.resolve(strict=True) != path
            or (root is not None and not path.is_relative_to(root))
            or hashlib.sha256(path.read_bytes()).hexdigest() != expected_sha256):
        raise ValueError("hook_file_identity_invalid")
    return path


def verify_installed_hook(node, node_sha256, hook, hook_sha256, worker, worker_sha256,
                          private_root, expected_version, environment):
    if os.name != "posix":
        raise ValueError("unix_controlling_terminal_required")
    import fcntl
    import pty
    import termios
    import tty

    root = Path(private_root)
    info = root.lstat()
    if (not root.is_absolute() or root.resolve(strict=True) != root
            or not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o700
            or info.st_uid != os.getuid()):
        raise ValueError("private_hook_root_invalid")
    node = plain_file(node, node_sha256, executable=True)
    hook = plain_file(hook, hook_sha256, root)
    worker = plain_file(worker, worker_sha256, executable=True)
    # 精确白名单同时拒绝凭据与 NODE_OPTIONS 之类的运行时注入入口。
    if set(environment) != {"HOME", "GROK_HOME", "TMPDIR", "PATH"}:
        raise ValueError("hook_environment_not_isolated")
    for name in ("HOME", "GROK_HOME", "TMPDIR"):
        candidate = Path(environment[name])
        if candidate.resolve(strict=True) != candidate or not candidate.is_relative_to(root):
            raise ValueError("hook_environment_path_invalid")
    session = "isolated-installed-hook-main"
    env = {**environment, "WARP_CLI_AGENT_PROTOCOL_VERSION": "1",
           "GROK_HOOK_EVENT": "session_start", "GROK_SESSION_ID": session,
           "WARP_CLI_AGENT_NOTIFY_EXECUTABLE": str(worker)}
    env.pop("TMUX", None)
    payload = json.dumps({"hookEventName": "session_start", "sessionId": session}).encode()
    master, slave = pty.openpty()
    tty.setraw(slave)
    process = None
    selector = selectors.DefaultSelector()
    chunks = {"tty": bytearray(), "stdout": bytearray(), "stderr": bytearray()}

    def controlling_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    try:
        # 保留控制终端；stdout 单独捕获，不能把它误判为原生通知通道。
        process = subprocess.Popen([str(node), str(hook)], cwd=root, env=env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, pass_fds=(slave,),
                                   preexec_fn=controlling_terminal)
        process.stdin.write(payload)
        process.stdin.close()
        for stream, kind in [(master, "tty"), (process.stdout, "stdout"), (process.stderr, "stderr")]:
            os.set_blocking(stream if isinstance(stream, int) else stream.fileno(), False)
            selector.register(stream, selectors.EVENT_READ, kind)
        deadline = time.monotonic() + 5
        exited_at = None
        while True:
            now = time.monotonic()
            if now >= deadline:
                raise ValueError("hook_main_timeout")
            for key, _ in selector.select(timeout=min(.05, deadline - now)):
                descriptor = key.fileobj if isinstance(key.fileobj, int) else key.fileobj.fileno()
                chunk = os.read(descriptor, 16385)
                if chunk:
                    chunks[key.data].extend(chunk)
                    if len(chunks[key.data]) > 16384:
                        raise ValueError("hook_output_budget_exceeded")
                else:
                    selector.unregister(key.fileobj)
            if process.poll() is not None:
                if exited_at is None:
                    exited_at = time.monotonic()
                if time.monotonic() - exited_at >= .1:
                    break
        if process.returncode != 0 or chunks["stdout"] or chunks["stderr"]:
            raise ValueError("hook_main_channel_invalid")
        prefix = b"\x1b]777;notify;warp://cli-agent;"
        raw = bytes(chunks["tty"])
        if not raw.startswith(prefix) or not raw.endswith(b"\x07"):
            raise ValueError(f"hook_main_notification_missing:tty_bytes={len(raw)}")
        notification = json.loads(raw[len(prefix):-1])
        if (set(notification) != {"v", "agent", "event", "session_id", "plugin_version", "event_id"}
                or type(notification["v"]) is not int or notification["v"] != 1
                or notification["agent"] != "grok"
                or notification["event"] != "session_start" or notification["session_id"] != session
                or notification["plugin_version"] != expected_version
                or not isinstance(notification["event_id"], str)
                or not notification["event_id"].startswith("grok:")):
            raise ValueError("installed_hook_notification_invalid")
        plain_file(node, node_sha256, executable=True)
        plain_file(hook, hook_sha256, root)
        plain_file(worker, worker_sha256, executable=True)
        return {"main_entry_verified": True,
                "notification_channel": "native_worker_to_unix_controlling_tty",
                "event": "session_start", "agent": "grok", "plugin_version": expected_version,
                "hook_sha256": hook_sha256, "worker_sha256": worker_sha256,
                "session_id_matches": True, "node_exit_code": 0,
                "stdout_bytes": 0, "stderr_bytes": 0, "credentials_provided": False,
                "model_input_submitted": False}
    finally:
        if process is not None:
            if process.poll() is None:
                os.killpg(process.pid, 9)
            process.wait(timeout=5)
            for stream in (process.stdout, process.stderr):
                stream.close()
        selector.close()
        os.close(master)
        os.close(slave)
