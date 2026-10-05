"""核对已安装 Grok hook 的真实 Unix 控制终端输出；不运行模型。"""

import hashlib
import json
import math
import os
from pathlib import Path
import selectors
import stat
import struct
import subprocess
import sys
import tempfile
import time


DIAGNOSTIC_LIMIT = 4096
NATIVE_ERRORS = (
    "protocol_mismatch", "invalid_payload", "frame_too_large", "input_unavailable", "input_timeout",
    "terminal_unavailable", "tmux_unavailable", "lock_unavailable", "lock_timeout",
    "write_failed", "write_timeout",
    "send_timeout_prepare", "send_timeout_terminal", "send_timeout_cache",
    "send_timeout_lock_open", "send_timeout_lock_wait", "send_timeout_frame_write",
    "send_timeout_unknown", "send_unavailable",
)
DIAGNOSTIC_ERRORS = {"unknown", "ETIMEDOUT", "ENOENT", "EACCES", "ENOEXEC", "EAGAIN",
                     "ENOMEM", "ENOBUFS", *("cli_agent_notify_" + value for value in NATIVE_ERRORS)}
DIAGNOSTIC_SIGNALS = {"other", "SIGHUP", "SIGINT", "SIGQUIT", "SIGILL", "SIGABRT", "SIGFPE",
                      "SIGKILL", "SIGSEGV", "SIGPIPE", "SIGTERM", "SIGTTIN", "SIGTTOU", "SIGBUS"}
NATIVE_TRACE_RECORD = struct.Struct("<4sB3xIIQ16s8x")
NATIVE_TRACE_LIMIT = 32 * NATIVE_TRACE_RECORD.size
NATIVE_TRACE_STAGES = ("main", "fluent_begin", "fluent_end", "lease_begin", "lease_end",
                       "worker", "input_begin", "input_received", "prepare", "terminal",
                       "cache", "lock_open", "lock_wait", "frame_write", "frame_written",
                       "reply_begin", "reply_end", "main_ok", "main_error")


def native_worker_diagnostics(raw, nonce):
    # 仅输出定长字段；无首标不能区分 exec/加载/调度/授权失败，不推测 Fluent 阻塞。
    identity = {"wire_bytes": len(raw), "wire_sha256": hashlib.sha256(raw).hexdigest()}
    if not raw:
        return {"status": "missing", **identity}
    if len(raw) > NATIVE_TRACE_LIMIT:
        return {"status": "over_budget", **identity}
    if len(raw) % NATIVE_TRACE_RECORD.size:
        return {"status": "invalid", **identity}
    rows = []
    for index in range(0, len(raw), NATIVE_TRACE_RECORD.size):
        record = raw[index:index + NATIVE_TRACE_RECORD.size]
        magic, stage, sequence, pid, timestamp, actual_nonce = NATIVE_TRACE_RECORD.unpack(record)
        if (magic != b"INW1" or not 1 <= stage <= len(NATIVE_TRACE_STAGES)
                or record[5:8] != bytes(3) or record[40:] != bytes(8)
                or actual_nonce != nonce or sequence != len(rows) or pid == 0
                or (rows and (pid != rows[0]["pid"] or timestamp < rows[-1]["monotonic_ns"]))):
            return {"status": "invalid", **identity}
        rows.append({"stage": NATIVE_TRACE_STAGES[stage - 1], "sequence": sequence,
                     "pid": pid, "monotonic_ns": timestamp})
    return {"status": "captured", **identity, "events": rows, "invocation_nonce": nonce.hex(),
            "missing_stages_are_unknown": True}


class NativeWorkerTrace:
    # 每次原调用各一对匿名管道；读端保持至原调用结束，不等待 EOF、不增加发送次数。
    def __init__(self, *, enabled=None):
        self.enabled = sys.platform == "linux" if enabled is None else enabled
        self.reader = self.writer = None
        self.nonce = None
        self.authorization = None
        self.raw = bytearray()
        self.failed = False

    def __enter__(self):
        if self.enabled:
            try:
                self.nonce = os.urandom(16)
                self.reader, self.writer = os.pipe()
                for descriptor in (self.reader, self.writer):
                    os.set_blocking(descriptor, False)
                    os.set_inheritable(descriptor, False)
                info = os.fstat(self.writer)
                self.authorization = f"v1:{info.st_dev}:{info.st_ino}:{self.nonce.hex()}"
            except OSError:
                self.failed = True
                self.close()
        return self

    @property
    def pass_fds(self):
        return () if self.writer is None else (self.writer,)

    @property
    def binding(self):
        return None if self.writer is None else {"fd": self.writer, "authorization": self.authorization}

    def summary(self):
        if not self.enabled:
            return {"status": "not_enabled"}
        if self.failed:
            return {"status": "unavailable"}
        try:
            # 总记录小于 PIPE_BUF；一次有界非阻塞读取即可，无读线程或 join。
            remaining = NATIVE_TRACE_LIMIT + 1 - len(self.raw)
            if remaining > 0:
                self.raw.extend(os.read(self.reader, remaining))
        except BlockingIOError:
            pass
        except OSError:
            self.failed = True
            return {"status": "unavailable"}
        return native_worker_diagnostics(bytes(self.raw), self.nonce)

    def close(self):
        for name in ("reader", "writer"):
            descriptor = getattr(self, name)
            if descriptor is not None:
                setattr(self, name, None)
                try:
                    os.close(descriptor)
                except OSError:
                    # 诊断释放错误不能替换原调用的返回或异常。
                    self.failed = True

    def __exit__(self, exc_type, exc_value, traceback):
        self.close()


def diagnostic_preload(worker, descriptor, native_trace=None):
    # 只在测试 Node 外层加载；除显式诊断授权外，原参数/环境/期限与重抛保持。
    # 成功的 execFileSync 不暴露 stderr，明确记 null，不能把未知字节数写成零。
    source = r'''"use strict";
const fs = require("node:fs");
const child = require("node:child_process");
const execute = child.execFileSync;
const expected = __WORKER__;
const nativeTrace = __NATIVE_TRACE__;
const allowedErrors = new Set(__ERRORS__);
const allowedSignals = new Set(__SIGNALS__);
function emit(row) {
  try { fs.writeSync(__FD__, JSON.stringify(row) + "\n"); } catch (_) {}
}
function byteLength(value) {
  return typeof value === "string" || Buffer.isBuffer(value) ? Buffer.byteLength(value) : null;
}
emit({stage: "preload", elapsed_ms: 0, exit_code: null, signal: null,
  stdout_bytes: 0, stderr_bytes: 0, error_code: null});
child.execFileSync = function(file, args, options) {
  const stage = file === expected && Array.isArray(args) && args[0] === "cli-agent-notify"
    ? args.length === 1
      || args.length === 3 && args[1] === "--require-protocol" && args[2] === "1" ? "send"
      : args.length === 2 && args[1] === "--protocol-version" ? "protocol" : null : null;
  if (!stage) return execute.apply(this, arguments);
  const started = process.hrtime.bigint();
  const elapsed = () => Math.min(60000, Number(process.hrtime.bigint() - started) / 1e6);
  // 捕获原本丢弃的 stderr；超过既有 maxBuffer 会额外触发 ENOBUFS，诊断结果须保留此边界。
  // 原 argv/timeout/maxBuffer 和同步返回/重抛仍保持；环境只可追加下方测试授权。
  const observed = {...options, stdio: [...options.stdio]};
  observed.stdio[2] = "pipe";
  // 只给精确守卫发送增加独立 FD3；不改原标准流、argv、payload 或任何期限。
  if (process.platform === "linux" && nativeTrace && stage === "send"
      && args.length === 3 && args[1] === "--require-protocol" && args[2] === "1"
      && observed.stdio.length === 3
      && !Object.prototype.hasOwnProperty.call(options.env || process.env, "INFINISHELL_TEST_NOTIFY_TRACE")) {
    observed.stdio.push(nativeTrace.fd);
    observed.env = {...(options.env || process.env), INFINISHELL_TEST_NOTIFY_TRACE: nativeTrace.authorization};
  }
  try {
    const result = execute.call(this, file, args, observed);
    emit({stage, elapsed_ms: elapsed(), exit_code: 0, signal: null,
      stdout_bytes: byteLength(result), stderr_bytes: null, error_code: null});
    return result;
  } catch (error) {
    const stderr = typeof error.stderr === "string" || Buffer.isBuffer(error.stderr)
      ? error.stderr.toString() : "";
    const native = /^Error: (cli_agent_notify_[a-z_]+)\r?\n?$/.exec(stderr);
    const code = native && allowedErrors.has(native[1]) ? native[1]
      : allowedErrors.has(error.code) ? error.code : "unknown";
    emit({stage, elapsed_ms: elapsed(),
      exit_code: Number.isInteger(error.status) ? error.status : null,
      signal: error.signal ? allowedSignals.has(error.signal) ? error.signal : "other" : null,
      stdout_bytes: byteLength(error.stdout), stderr_bytes: byteLength(error.stderr), error_code: code});
    throw error;
  }
};
'''
    return (source.replace("__FD__", str(descriptor))
            .replace("__NATIVE_TRACE__", json.dumps(native_trace))
            .replace("__ERRORS__", json.dumps(sorted(DIAGNOSTIC_ERRORS)))
            .replace("__SIGNALS__", json.dumps(sorted(DIAGNOSTIC_SIGNALS)))
            .replace("__WORKER__", json.dumps(str(worker))))


def worker_diagnostics(raw):
    # 独立诊断 FD 也是不可信输入；拒绝额外字段、任意文本与异常数量，绝不回显原字节。
    if not raw:
        return {"status": "missing"}
    if len(raw) > DIAGNOSTIC_LIMIT:
        return {"status": "over_budget"}
    try:
        rows = [json.loads(line) for line in raw.decode("utf-8").splitlines()]
        if not 1 <= len(rows) <= 3 or not raw.endswith(b"\n"):
            raise ValueError
        fields = {"stage", "elapsed_ms", "exit_code", "signal", "stdout_bytes", "stderr_bytes", "error_code"}
        for row in rows:
            if (not isinstance(row, dict) or set(row) != fields
                    or row["stage"] not in ("preload", "protocol", "send")
                    or type(row["elapsed_ms"]) not in (int, float)
                    or not math.isfinite(row["elapsed_ms"]) or not 0 <= row["elapsed_ms"] <= 60000
                    or row["signal"] not in DIAGNOSTIC_SIGNALS | {None}
                    or row["error_code"] not in DIAGNOSTIC_ERRORS | {None}):
                raise ValueError
            for name, maximum in (("exit_code", 2147483647), ("stdout_bytes", 1048576), ("stderr_bytes", 1048576)):
                value = row[name]
                if value is not None and (type(value) is not int or not 0 <= value <= maximum):
                    raise ValueError
            if row["exit_code"] == 0 and (row["signal"] is not None or row["error_code"] is not None):
                raise ValueError
        # 新发送在同一进程内守卫协议；不能补造一次未发生的独立查询。
        # 保留旧 hook 的分阶段诊断，但仍拒绝重试、乱序和混合序列。
        if tuple(row["stage"] for row in rows) not in (
                ("preload",), ("preload", "send"), ("preload", "protocol"),
                ("preload", "protocol", "send")):
            raise ValueError
        if len(rows) == 3 and (rows[1]["exit_code"] != 0 or rows[1]["error_code"] is not None):
            raise ValueError
        return {"status": "captured", "events": rows}
    except (ValueError, TypeError, UnicodeError, OverflowError):
        return {"status": "invalid"}


def hook_failure(reason, diagnostics, native_trace=None):
    safe = json.dumps(worker_diagnostics(diagnostics), ensure_ascii=True, separators=(",", ":"))
    native = "" if native_trace is None else "; native_worker_diagnostics=" + json.dumps(
        native_trace.summary(), ensure_ascii=True, separators=(",", ":"))
    return ValueError(f"{reason}; worker_diagnostics={safe}{native}")


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
    diagnostic_file = None
    preload_name = None
    native_trace = NativeWorkerTrace()

    def observed_failure(reason):
        return hook_failure(reason, os.pread(diagnostic_file.fileno(), DIAGNOSTIC_LIMIT + 1, 0), native_trace)

    def controlling_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    try:
        native_trace.__enter__()
        # 匿名文件 FD 避免诊断管道背压扰动原生调用；父进程只读固定字节预算。
        diagnostic_file = tempfile.TemporaryFile(mode="w+b", dir=root)
        preload_fd, preload_name = tempfile.mkstemp(prefix="hook-diagnostic-", suffix=".cjs", dir=root)
        with os.fdopen(preload_fd, "w") as preload:
            preload.write(diagnostic_preload(worker, diagnostic_file.fileno(), native_trace.binding))
        # 保留控制终端；stdout 单独捕获，不能把它误判为原生通知通道。
        process = subprocess.Popen([str(node), "--require", preload_name, str(hook)], cwd=root, env=env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE,
                                   pass_fds=(slave, diagnostic_file.fileno(), *native_trace.pass_fds),
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
                raise observed_failure("hook_main_timeout")
            for key, _ in selector.select(timeout=min(.05, deadline - now)):
                descriptor = key.fileobj if isinstance(key.fileobj, int) else key.fileobj.fileno()
                chunk = os.read(descriptor, 16385)
                if chunk:
                    chunks[key.data].extend(chunk)
                    if len(chunks[key.data]) > 16384:
                        raise observed_failure("hook_output_budget_exceeded")
                else:
                    selector.unregister(key.fileobj)
            if process.poll() is not None:
                if exited_at is None:
                    exited_at = time.monotonic()
                if time.monotonic() - exited_at >= .1:
                    break
        if process.returncode != 0 or chunks["stdout"] or chunks["stderr"]:
            raise observed_failure("hook_main_channel_invalid")
        prefix = b"\x1b]777;notify;warp://cli-agent;"
        raw = bytes(chunks["tty"])
        if not raw.startswith(prefix) or not raw.endswith(b"\x07"):
            raise observed_failure(f"hook_main_notification_missing:tty_bytes={len(raw)}")
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
                "model_input_submitted": False,
                "native_worker_diagnostics": native_trace.summary()}
    finally:
        try:
            if process is not None:
                if process.poll() is None:
                    os.killpg(process.pid, 9)
                process.wait(timeout=5)
                for stream in (process.stdout, process.stderr):
                    stream.close()
        finally:
            native_trace.close()
        selector.close()
        if diagnostic_file is not None:
            diagnostic_file.close()
        if preload_name is not None:
            Path(preload_name).unlink()
        os.close(master)
        os.close(slave)
