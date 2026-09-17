#!/usr/bin/env python3
"""准备官方 Grok 普通 PTY 输入探针；未验证全资源域时拒绝真实启动。"""

import argparse
import base64
import codecs
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import sys
import tempfile
import time
import unicodedata
import uuid

import run_grok_official_adapter_live as official

SCOPE = "official_grok_ordinary_pty_input_preparation"
HELP_SHA256 = "cda6873e2f90a7d77de94c2e3026794671fac04b2e40ac74e4d91f429f829403"
PASTE_BEGIN, PASTE_END = b"\x1b[200~", b"\x1b[201~"
MAX_RAW_BYTES = 8 * 1024 * 1024
MAX_FRAME_FILE_BYTES = 16 * 1024 * 1024
MAX_INPUTS = 3
LIVE_BLOCK = "pty_containment_not_verified"


def require(value):
    if not value:
        raise ValueError("PTY 输入探针缺少固定隔离或渲染证据")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical_uuid(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", value)


def build_plan(session_id):
    require(canonical_uuid(session_id))
    token = session_id.replace("-", "")
    stages = []
    for phase, prefix, text in (
            ("english_multiline", "EN", f"INPUT_EN_{token}\nDo not call tools. Reply only with the concatenation of these five pieces: PTY, underscore, EN, underscore, {token}."),
            ("chinese_multiline", "ZH", f"输入中文_{token}\n第一行：不要调用任何工具。\n第二行：只回复这五段拼接的结果：PTY、下划线、ZH、下划线、{token}。")):
        expected = f"PTY_{prefix}_{token}"
        payload = text.encode("utf-8")
        require(expected.encode() not in payload)
        stages.append({"phase": phase, "payload": text, "expected": expected,
            "render_tokens": text.splitlines()[:2] if prefix == "ZH" else [text.splitlines()[0]],
            "payload_bytes": len(payload), "payload_sha256": digest(payload),
            "expected_response_bytes": len(expected.encode()), "expected_response_sha256": digest(expected.encode())})
    text = f"INPUT_CANCEL_{token}\nDo not call tools. Immediately print every integer from 1 through 1000000 on its own line, without explanation."
    stages.append({"phase": "cancel_signal_probe", "payload": text, "expected": None,
        "render_tokens": [text.splitlines()[0]], "payload_bytes": len(text.encode()),
        "payload_sha256": digest(text.encode()), "expected_response_bytes": None,
        "expected_response_sha256": None})
    return stages


def tui_arguments(root, session_id):
    require(canonical_uuid(session_id) and root.is_absolute())
    socket = root / "tmp/pty-leader/leader.sock"
    return ["--minimal", "--no-alt-screen", "--cwd", str(root / "project"), "--leader-socket", str(socket),
        "--session-id", session_id, "--no-subagents", "--disable-web-search"]


def pty_sandbox_profile(root, source_home, port, user_home):
    # 只允许精确私有 socket；宿主 SSH 与兼容配置不可读，终端写入只给终端设备。
    require(root.is_absolute() and source_home.is_absolute() and user_home.is_absolute()
        and type(port) is int and 1 <= port <= 65535)
    endpoint = root / "tmp/pty-leader/leader.sock"
    profile = official.shared.sandbox_profile(root, source_home, port, endpoint)
    for relative in (".ssh", ".claude", ".codex", ".grok/auth.json", ".grok/config.toml"):
        profile += f"(deny file-read* (subpath {json.dumps(str(user_home / relative))}))"
    profile += '(allow file-write* (regex #"^/dev/(tty|ttys[0-9]+|ptmx)$"))'
    return profile


def isolated_environment(root, port):
    require(root.is_absolute() and type(port) is int and 1 <= port <= 65535)
    environment = official.official_environment(root, port)
    environment.update(TERM="xterm-256color", COLUMNS="120", LINES="40", LANG="en_US.UTF-8",
        LC_ALL="en_US.UTF-8", GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null",
        GIT_TERMINAL_PROMPT="0", GIT_SSH_COMMAND="ssh -F " + shlex.quote(str(root / "home/.ssh/config")))
    return environment


def private_file(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(data)


class PrivateFrames:
    """完整 TUI 与输入只写私有帧文件，公开报告只引用累计摘要。"""

    def __init__(self, path):
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
        self.file = os.fdopen(descriptor, "wb")
        self.bytes = 0
        self.encoded_bytes = 0
        self.hash = hashlib.sha256()

    def record(self, direction, data):
        require(direction in ("stdin", "stdout") and isinstance(data, bytes))
        require(self.bytes + len(data) <= MAX_RAW_BYTES)
        frame = (json.dumps({"direction": direction, "bytes_base64": base64.b64encode(data).decode("ascii")},
            separators=(",", ":")) + "\n").encode()
        require(self.encoded_bytes + len(frame) <= MAX_FRAME_FILE_BYTES)
        self.file.write(frame)
        self.file.flush()
        self.bytes += len(data)
        self.encoded_bytes += len(frame)
        self.hash.update(frame)

    def close(self):
        self.file.close()
        return {"raw_terminal_bytes": self.bytes, "private_frame_file_bytes": self.encoded_bytes,
            "private_frame_file_sha256": self.hash.hexdigest()}


class Screen:
    """探针专用有界 VT 视口；未知控制序列使响应判据失效，不能猜测实际布局。"""

    def __init__(self, columns=120, rows=40):
        require(type(columns) is int and 40 <= columns <= 240 and type(rows) is int and 10 <= rows <= 100)
        self.columns, self.rows = columns, rows
        self.cells = [[" "] * columns for _ in range(rows)]
        self.x = self.y = 0
        self.saved = (0, 0)
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("strict")
        self.supported = True

    def newline(self):
        self.y += 1
        if self.y >= self.rows:
            self.cells.pop(0)
            self.cells.append([" "] * self.columns)
            self.y = self.rows - 1

    def csi(self, parameters, final):
        if parameters.startswith("?"):
            if final not in "hl" or parameters[1:] not in ("25", "2004", "1049", "1000", "1002", "1003", "1006"):
                self.supported = False
            return
        if not re.fullmatch(r"[0-9;]*", parameters):
            self.supported = False
            return
        values = [int(value or "0") for value in parameters.split(";")]
        first = values[0]
        amount = first or 1
        if final == "m":
            return
        if final in "Hf":
            self.y = min(self.rows - 1, max(0, (first or 1) - 1))
            self.x = min(self.columns - 1, max(0, ((values[1] if len(values) > 1 else 1) or 1) - 1))
        elif final == "A":
            self.y = max(0, self.y - amount)
        elif final == "B":
            self.y = min(self.rows - 1, self.y + amount)
        elif final == "C":
            self.x = min(self.columns - 1, self.x + amount)
        elif final == "D":
            self.x = max(0, self.x - amount)
        elif final == "G":
            self.x = min(self.columns - 1, amount - 1)
        elif final == "d":
            self.y = min(self.rows - 1, amount - 1)
        elif final == "K" and first in (0, 1, 2):
            start, end = (self.x, self.columns) if first == 0 else ((0, self.x + 1) if first == 1 else (0, self.columns))
            self.cells[self.y][start:end] = [" "] * (end - start)
        elif final == "J" and first in (0, 1, 2, 3):
            if first in (2, 3):
                self.cells = [[" "] * self.columns for _ in range(self.rows)]
            else:
                selected = range(self.y + 1, self.rows) if first == 0 else range(self.y)
                for row in selected:
                    self.cells[row] = [" "] * self.columns
                self.csi("0" if first == 0 else "1", "K")
        elif final == "s":
            self.saved = (self.x, self.y)
        elif final == "u":
            self.x, self.y = self.saved
        else:
            self.supported = False

    def feed(self, data):
        try:
            self.pending += self.decoder.decode(data)
        except UnicodeError:
            self.supported = False
            raise ValueError("PTY 渲染字节无法按 UTF-8 解码") from None
        while self.pending:
            if self.pending.startswith("\x1b["):
                sequence = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", self.pending)
                if sequence is None:
                    if len(self.pending) > 256:
                        self.supported = False
                        self.pending = ""
                    break
                if sequence[2]:
                    self.supported = False
                else:
                    self.csi(sequence[1], sequence[3])
                self.pending = self.pending[len(sequence[0]):]
                continue
            if self.pending.startswith("\x1b]"):
                sequence = re.match(r"\x1b\].*?(?:\x07|\x1b\\)", self.pending, re.DOTALL)
                if sequence is None:
                    if len(self.pending) > 4096:
                        self.supported = False
                        self.pending = ""
                    break
                self.pending = self.pending[len(sequence[0]):]
                continue
            character = self.pending[0]
            if character == "\x1b":
                if len(self.pending) == 1:
                    break
                self.supported = False
                self.pending = self.pending[2:]
                continue
            self.pending = self.pending[1:]
            if character == "\r":
                self.x = 0
            elif character == "\n":
                self.newline()
            elif character == "\b":
                self.x = max(0, self.x - 1)
            elif character == "\t":
                self.x = min(self.columns - 1, (self.x // 8 + 1) * 8)
            elif character == "\x07":
                continue
            elif ord(character) < 32 or character == "\x7f":
                self.supported = False
            elif unicodedata.combining(character):
                if self.x:
                    self.cells[self.y][self.x - 1] += character
            else:
                width = 2 if unicodedata.east_asian_width(character) in ("W", "F") else 1
                if self.x + width > self.columns:
                    self.x = 0
                    self.newline()
                self.cells[self.y][self.x] = character
                if width == 2:
                    self.cells[self.y][self.x + 1] = ""
                self.x += width

    def lines(self):
        return ["".join(row).rstrip() for row in self.cells]


def observe_stage(channel, stage, frames, screen, timeout=30, now=time.monotonic):
    # 通道必须由未来已验证的资源域所有者提供；本文件不派生普通 PTY 或原生进程。
    require(1 <= timeout <= 60 and stage["phase"] in ("english_multiline", "chinese_multiline", "cancel_signal_probe"))
    payload = stage["payload"].encode()
    expected = stage["expected"]
    require(expected is None or expected.encode() not in payload)
    events = []

    def send(data):
        require(channel.write(data) == len(data))
        frames.record("stdin", data)

    def until(predicate):
        deadline = now() + timeout
        while now() < deadline:
            data = channel.read(min(0.2, max(0, deadline - now())))
            if data is None:
                raise ValueError("PTY 已断开，不能计为响应或取消成功")
            if data:
                frames.record("stdout", data)
                screen.feed(data)
            if screen.supported and not screen.pending and predicate(screen.lines()):
                return
        raise ValueError("PTY 输入或文本响应超过有界期限")

    send(PASTE_BEGIN + payload + PASTE_END)
    events.append({"event": "bracketed_paste_written", "phase": stage["phase"],
        "payload_bytes": len(payload), "payload_sha256": digest(payload), "submitted_to_model_verified": False})
    until(lambda lines: all(any(token in line for line in lines) for token in stage["render_tokens"]))
    events.append({"event": "paste_render_observed", "phase": stage["phase"], "gui_verified": False})
    require(expected is None or expected not in [line.strip() for line in screen.lines()])
    send(b"\r")
    events.append({"event": "enter_written", "phase": stage["phase"], "native_ack_verified": False})
    if expected is not None:
        until(lambda lines: expected in [line.strip() for line in lines])
        events.append({"event": "response_text_observed", "phase": stage["phase"],
            "response_marker_bytes": len(expected.encode()), "response_marker_sha256": digest(expected.encode()),
            "task_completed_verified": False, "native_ack_verified": False})
    else:
        until(lambda lines: any([line.strip() for line in lines][index:index + 3] == ["1", "2", "3"]
            for index in range(len(lines) - 2)))
        send(b"\x03")
        events.append({"event": "cancel_signal_written", "phase": stage["phase"],
            "partial_text_observed": True, "task_cancelled_verified": False, "all_processes_exited_verified": False})
    return events


def live_preflight():
    # 当前监督入口使用管道；没有核验控制终端及私有 leader 的完整资源域，不设置绕过开关。
    if sys.platform != "darwin":
        raise ValueError("本运行器不在 Linux 或 Windows 启动原生 CLI")
    raise ValueError(LIVE_BLOCK)


def prepare_only(args):
    require(sys.platform == "darwin")
    native = args.grok
    require(not native.is_symlink() and native.is_file() and os.access(native, os.X_OK)
        and official.shared.digest(native) == official.shared.BINARY_SHA256)
    require(30 <= args.timeout <= 300 and args.output.suffix == ".ndjson" and not args.output.is_symlink())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-pty-preparation-", dir="/private/tmp")).resolve()
    for name in ("home/.grok", "home/.ssh", "tmp/pty-leader", "project", "unread-auth-source"):
        (root / name).mkdir(parents=True, exist_ok=True, mode=0o700)
    session_id = str(uuid.uuid4())
    plan = build_plan(session_id)
    wrapper, settings = official.prepare_native(root, native.resolve(), root / "unread-auth-source", 1)
    wrapper.unlink()
    private_file(root / "home/.ssh/known_hosts", b"")
    private_file(root / "home/.ssh/config", b"Host *\n    StrictHostKeyChecking yes\n    UserKnownHostsFile ~/.ssh/known_hosts\n    GlobalKnownHostsFile /dev/null\n    BatchMode yes\n    IdentityAgent none\n")
    private_file(root / "private-input-plan.json", (json.dumps(plan, ensure_ascii=False, indent=2) + "\n").encode())
    private_file(root / "private-tui-arguments.json", (json.dumps(tui_arguments(root, session_id)) + "\n").encode())
    events = [{"event": "preparation_only", "scope": SCOPE, "session_id": session_id,
        "native_launched": False, "official_auth_copied": False, "model_request_sent": False,
        "pty_containment_verified": False, "normal_cleanup_verified": False,
        "task_completed_verified": False, "task_cancelled_verified": False}]
    events.extend({"event": "planned_input", **{key: stage[key] for key in ("phase", "payload_bytes",
        "payload_sha256", "expected_response_bytes", "expected_response_sha256")}} for stage in plan)
    metadata = {"scope": SCOPE, "preparation_only": True, "live_block": LIVE_BLOCK,
        "private_workspace": str(root), "native_sha256": official.shared.digest(native),
        "cli_version_expected": official.shared.VERSION, "help_sha256": HELP_SHA256,
        "help_calibrated_without_auth_or_network": True, "normal_cleanup_verified": False,
        "help_calibration_is_prior_observation": True, "version_actually_detected": False,
        "pty_containment_verified": False, "real_input_verified": False, "gui_verified": False,
        "ssh_tmux_verified": False, "claude_hooks_compatibility_verified": False,
        "claude_hooks_enabled": False, "private_settings_sha256": official.shared.digest(settings),
        "max_native_inputs": MAX_INPUTS, "http_model_call_budget_enforced": False,
        "planned_total_timeout_seconds": args.timeout,
        "cost_budget_enforced": False, "tls_decrypted": False}
    network = {"events": [], "tls_connections_attempted": 0, "tls_bytes": 0,
        "allowed_hosts": sorted(official.OFFICIAL_HOSTS), "max_tunnels": official.MAX_TUNNELS,
        "max_tls_bytes": official.MAX_BYTES, "network_canary_verified": False,
        "auth_copy_removed": None}
    targets = (args.output, args.output.with_suffix(".metadata.json"), args.output.with_suffix(".network.json"))
    require(all(not path.exists() and not path.is_symlink() for path in targets))
    private_file(targets[0], "".join(json.dumps(event, ensure_ascii=False) + "\n" for event in events).encode())
    private_file(targets[1], (json.dumps(metadata, ensure_ascii=False, indent=2) + "\n").encode())
    private_file(targets[2], (json.dumps(network, indent=2) + "\n").encode())
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--grok", type=Path)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.live:
            live_preflight()
        require(args.prepare_only and args.grok is not None and args.output is not None)
        return prepare_only(args)
    except (OSError, ValueError, UnicodeError) as error:
        refusal = {"event": "live_execution_refused" if args.live else "preparation_failed",
            "scope": SCOPE, "reason": LIVE_BLOCK if args.live and sys.platform == "darwin" else "unsupported_or_invalid_preparation",
            "error_type": type(error).__name__, "native_launched": False, "auth_copied": False,
            "model_request_sent": False, "normal_cleanup_verified": False}
        print(json.dumps(refusal, ensure_ascii=False))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
