#!/usr/bin/env python3
"""准备 Grok 1.0.34 原始 ACP P0 候选；默认不执行，真实运行需要双重显式确认。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import secrets
import signal
import stat
import subprocess
import sys
import threading
import time
import tomllib
import uuid

sys.dont_write_bytecode = True
import prepare_grok_cli as fixed
import run_grok_official_adapter_live as official

SCOPE = "grok_1_0_34_raw_acp_native_p0_candidate"
CONFIRM = "grok-1.0.34-raw-p0-four-inputs"
VERSION = "1.0.34"
PHASES = ("new_allow_read", "deny_read", "cancel_after_real_text", "cold_load_recall")
MAX_INPUTS, MAX_REQUESTS, MAX_PROCESSES = 4, 27, 2
MAX_SECONDS, MAX_TLS_CONNECTIONS, MAX_TLS_BYTES = 540, 32, 32 * 1024 * 1024
MAX_LINE, MAX_BYTES, MAX_FRAMES = 1024 * 1024, 8 * 1024 * 1024, 2048
RUN_ENV = "INFINISHELL_GROK_NATIVE_P0_RUN"
PRIVATE_DIRECTORIES = ("home", "home/.grok", "home/.claude", "home/.codex", "home/.config",
    "home/.local", "home/.local/share", "home/.local/state", "home/.cache", "home/AppData",
    "home/AppData/Roaming", "home/AppData/Local", "project", "tmp", "cases")


class Rejected(ValueError):
    """异常只包含固定原因代码，不携带原生正文、路径或认证。"""


def require(value, reason):
    if not value:
        raise Rejected(reason)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, "duplicate_json_key")
        result[key] = value
    return result


def parse(data):
    return json.loads(data, object_pairs_hook=pairs,
                      parse_constant=lambda value: (_ for _ in ()).throw(Rejected("nonfinite_json")))


def encoded(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


def private_file(path, limit=MAX_BYTES):
    info = fixed.regular_file(path)
    require(info.st_uid == os.getuid() and info.st_mode & 0o077 == 0
            and 0 <= info.st_size <= limit, "private_file_invalid")
    with path.open("rb") as source:
        data = source.read(limit + 1)
    require(len(data) <= limit, "private_file_too_large")
    return data


def private_directory(path):
    require(path.is_absolute() and not path.is_symlink() and path.resolve(strict=True) == path,
            "private_directory_identity_invalid")
    info = path.stat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid()
            and info.st_mode & 0o077 == 0, "private_directory_permissions_invalid")


def create_file(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())


def helper_hashes():
    return {Path(module.__file__).name: fixed.digest(Path(module.__file__).resolve())
            for module in (sys.modules[__name__], fixed, official, official.shared)}


def snapshots(root):
    names = ("project/allow.txt", "project/deny.txt", "home/.claude/settings.json",
             "home/.claude.json", "home/.codex/config.toml", "home/.zshrc")
    return {name: sha(private_file(root / name)) for name in names}


def validate_binary(path):
    require(path.is_absolute() and path.resolve(strict=True) == path and not path.is_symlink(),
            "binary_path_invalid")
    return fixed.verify_binary(path, "darwin-arm64", VERSION)


def configuration():
    return (f'[cli]\nuse_leader = false\nauto_update = false\n'
            f'[models]\ndefault = "{official.MODEL}"\nsession_summary = "{official.MODEL}"\n'
            f'web_search = "{official.MODEL}"\nimage_description = "{official.MODEL}"\n'
            '[features]\nturn_summary = false\ntitle_refresh = false\nsupport_permission = true\n'
            '[[permission.rules]]\naction = "ask"\ntool = "any"\n').encode()


def prepare(root, binary, source_home):
    require(root.is_absolute() and root.parent.resolve(strict=True) == root.parent
            and not root.exists() and not root.is_symlink(), "fixture_must_be_new")
    require(not root.is_relative_to(Path(__file__).resolve().parents[2]), "fixture_inside_repository")
    validate_binary(binary)
    private_directory(source_home)
    require(not root.is_relative_to(source_home) and not source_home.is_relative_to(root), "auth_scope_overlap")
    root.mkdir(mode=0o700)
    for relative in PRIVATE_DIRECTORIES:
        (root / relative).mkdir(mode=0o700)
    for name in ("allow", "deny"):
        create_file(root / f"project/{name}.txt", secrets.token_hex(32).encode())
    for relative, content in (("home/.claude/settings.json", b'{"permissions":{"defaultMode":"default"}}\n'),
                               ("home/.claude.json", b'{"synthetic_guard":true}\n'),
                               ("home/.codex/config.toml", b'approval_policy = "on-request"\n'),
                               ("home/.zshrc", '# 本次隔离原生 P0 哨兵\n'.encode())):
        create_file(root / relative, content)
    create_file(root / "home/.grok/config.toml", configuration())
    manifest = {"schema": 1, "scope": SCOPE, "candidate_unverified": True, "root": str(root),
        "binary": str(binary), "binary_evidence": validate_binary(binary), "version": VERSION,
        "version_output": fixed.VERSION_OUTPUTS[VERSION], "source_home": str(source_home),
        "helpers": helper_hashes(), "snapshots": snapshots(root), "config_sha256": sha(configuration()),
        "phases": list(PHASES), "max_native_inputs": MAX_INPUTS, "max_protocol_requests": MAX_REQUESTS,
        "max_native_processes": MAX_PROCESSES, "deadline_seconds": MAX_SECONDS,
        "max_tls_connections": MAX_TLS_CONNECTIONS, "max_tls_bytes": MAX_TLS_BYTES,
        "auth_copied": False, "model_inputs_sent": 0, "default_executes": False}
    create_file(root / "manifest.json", encoded(manifest))
    return {"scope": SCOPE, "prepared_only": True, "candidate_unverified": True,
            "manifest_sha256": sha(encoded(manifest)), "model_inputs_sent": 0}


def validate_manifest(root, binary, source_home):
    private_directory(root)
    require(not root.is_relative_to(Path(__file__).resolve().parents[2]) and not root.is_relative_to(source_home)
            and not source_home.is_relative_to(root), "fixture_scope_overlap")
    manifest = parse(private_file(root / "manifest.json", 65536))
    expected = {"schema": 1, "scope": SCOPE, "candidate_unverified": True, "root": str(root),
        "binary": str(binary), "binary_evidence": validate_binary(binary), "version": VERSION,
        "version_output": fixed.VERSION_OUTPUTS[VERSION], "source_home": str(source_home),
        "helpers": helper_hashes(), "snapshots": snapshots(root), "config_sha256": sha(configuration()),
        "phases": list(PHASES), "max_native_inputs": MAX_INPUTS, "max_protocol_requests": MAX_REQUESTS,
        "max_native_processes": MAX_PROCESSES, "deadline_seconds": MAX_SECONDS,
        "max_tls_connections": MAX_TLS_CONNECTIONS, "max_tls_bytes": MAX_TLS_BYTES,
        "auth_copied": False, "model_inputs_sent": 0, "default_executes": False}
    require(encoded(manifest) == encoded(expected), "manifest_binding_changed")
    require(private_file(root / "home/.grok/config.toml") == configuration(), "config_changed_before_run")
    private_directory(source_home)
    for relative in PRIVATE_DIRECTORIES:
        private_directory(root / relative)
    return manifest


def summary(value):
    names = {type(None): "null", bool: "boolean", int: "integer", float: "number",
             str: "string", list: "array", dict: "object"}
    data = encoded(value)
    return {"type": names.get(type(value), "invalid"), "bytes": len(data), "sha256": sha(data)}


def internal_skills_reload_success(value):
    # 与生产适配器相同的封闭内部成功结构；不能消费用户 RPC 或证明技能调用成功。
    if (type(value) is not dict or set(value) != {"jsonrpc", "id", "result"}
            or value.get("jsonrpc") != "2.0" or value.get("id") != "skills-reload"):
        return False
    result = value["result"]
    if type(result) is not dict or set(result) != {"result"}:
        return False
    inner = result["result"]
    return (type(inner) is dict and set(inner) == {"reloaded"} and type(inner["reloaded"]) is int
            and 0 <= inner["reloaded"] <= 2**64 - 1)


def diagnostic_shape(value):
    # 只展开固定协议字段名；未知键名也可能夹带正文，因此仅计数，不直接归档。
    keys = {"jsonrpc", "id", "method", "params", "result", "error", "code", "data", "_meta",
        "sessionId", "promptId", "eventId", "protocolVersion", "agentVersion", "agentCapabilities",
        "promptCapabilities", "image", "loadSession", "authMethods", "update", "updates", "sessionUpdate",
        "stopReason", "stop_reason", "cancellationCategory", "cancellationContext", "tool_name", "prompt_id",
        "runningPromptId", "entries", "kind", "status", "toolCall", "toolCallId", "rawInput", "rawOutput",
        "x.ai/tool", "name", "namespace", "version", "read_only", "variant", "target_file", "offset", "limit",
        "pages", "format", "options", "optionId", "outcome", "content", "text", "type", "streamStartMs",
        "chunkId", "hasMore", "totalCount", "lastEventId", "isReplay", "availableCommands", "mcpServers"}
    values = {"initialize", "authenticate", "session/new", "session/load", "session/prompt", "session/cancel",
        "session/update", "session/request_permission", "_x.ai/session/updates", "_x.ai/queue/changed",
        "_x.ai/session_notification", "_x.ai/session/update", "_x.ai/session/prompt_complete", "_x.ai/models/update",
        "_x.ai/mcp/servers_updated", "agent_message_chunk", "agent_thought_chunk", "available_commands_update",
        "tool_call", "tool_call_update", "turn_completed", "turn_started", "turn_created", "session_summary_generated", "end_turn", "cancelled",
        "error", "MidTurnAbort", "PermissionRejected", "read", "ReadFile", "pending", "in_progress", "completed", "failed"}
    enum_keys = {"method", "sessionUpdate", "stopReason", "stop_reason", "cancellationCategory", "kind", "status", "variant"}
    result = {**summary(value), "known_paths": [], "known_enums": {}, "unknown_keys": 0, "truncated": False}
    method = value.get("method") if type(value) is dict else None
    # 仅记录顶层、固定命名空间的有界方法 token；诊断识别不代表协议获准执行。
    if (type(method) is str and len(method) <= 96
            and re.fullmatch(r"(?:_x\.ai|session)/(?:[a-z][a-z_]{0,31}/){0,3}[a-z][a-z_]{0,31}", method)):
        result["safe_method_name"] = method
    def visit(item, path, depth):
        if depth > 5 or len(result["known_paths"]) >= 64:
            result["truncated"] = True
            return
        if type(item) is dict:
            for key, child in list(item.items())[:64]:
                if key not in keys:
                    result["unknown_keys"] += 1
                    continue
                current = path + "." + key if path else key
                result["known_paths"].append({"path": current, "type": summary(child)["type"]})
                if key in enum_keys:
                    result["known_enums"][current] = child if type(child) is str and child in values else summary(child)
                visit(child, current, depth + 1)
                if len(result["known_paths"]) >= 64:
                    result["truncated"] = True
                    break
            result["truncated"] |= len(item) > 64
        elif type(item) is list:
            for child in item[:2]:
                visit(child, path + "[]", depth + 1)
            result["truncated"] |= len(item) > 2
    visit(value, "", 0)
    return result


def identifier(value):
    return type(value) is str and bool(re.fullmatch(r"[0-9a-zA-Z_-]{1,256}", value))


def session_identifier(value):
    try:
        return type(value) is str and str(uuid.UUID(value)) == value
    except (ValueError, AttributeError):
        return False


def exact_permission(message, session, turn, call, target, project):
    try:
        params = message["params"]
        tool = params["toolCall"]
        meta = tool["_meta"]["x.ai/tool"]
        raw = tool["rawInput"]
        options = params["options"]
        require(message["jsonrpc"] == "2.0" and (type(message["id"]) is int and message["id"] >= 0 or type(message["id"]) is str and identifier(message["id"]))
                and params["sessionId"] == session and call is not None
                and tool["toolCallId"] == call and turn is not None, "permission_identity")
        require(tool["kind"] == "read" and meta["name"] == "read_file"
                and meta["namespace"] == "grok_build" and type(meta["version"]) is int
                and meta["version"] == 1 and meta["read_only"] is True, "permission_tool")
        require(type(raw) is dict and set(raw) <= {"variant", "target_file", "offset", "limit", "pages", "format"}
                and raw.get("variant") == "ReadFile" and type(raw.get("target_file")) is str,
                "permission_input")
        require((project / raw["target_file"]).resolve(strict=True) == target
                and not target.is_symlink(), "permission_path")
        require(all(raw.get(key) is None or type(raw[key]) is int and raw[key] == 1 for key in ("offset", "limit"))
                and all(raw.get(key) is None for key in ("pages", "format")), "permission_range")
        require(type(options) is list and len(options) <= 8 and all(type(option) is dict for option in options),
                "permission_options")
        ids = [option.get("optionId") for option in options]
        require(all(identifier(value) for value in ids) and len(set(ids)) == len(ids)
                and any(option.get("optionId") == "allow-once" and option.get("kind") == "allow_once" for option in options)
                and any(option.get("optionId") == "reject-once" and option.get("kind") == "reject_once" for option in options),
                "permission_options")
        return True
    except (Rejected, KeyError, TypeError, OSError, ValueError):
        return False


def permission_summary(message, session, turn, call, target, project):
    params = message.get("params") if type(message) is dict else None
    tool = params.get("toolCall") if type(params) is dict else None
    tool = tool if type(tool) is dict else {}
    meta = tool.get("_meta", {})
    meta = meta.get("x.ai/tool", {}) if type(meta) is dict else {}
    meta = meta if type(meta) is dict else {}
    raw = tool.get("rawInput")
    known = {"variant", "target_file", "offset", "limit", "pages", "format"}
    return {"exact_matches": exact_permission(message, session, turn, call, target, project),
        "name_matches": meta.get("name") == "read_file", "namespace_matches": meta.get("namespace") == "grok_build",
        "version_matches": type(meta.get("version")) is int and meta.get("version") == 1,
        "kind_matches": tool.get("kind") == "read", "raw_type": summary(raw)["type"],
        "missing_required_keys": len({"variant", "target_file"} - set(raw)) if type(raw) is dict else 2,
        "extra_key_count": len(set(raw) - known) if type(raw) is dict else 0,
        "variant_matches": type(raw) is dict and raw.get("variant") == "ReadFile"}


def network_guard(tunnel):
    with tunnel.lock:
        require(tunnel.forwarded <= MAX_TLS_CONNECTIONS and tunnel.bytes <= MAX_TLS_BYTES
                and time.monotonic() < tunnel.deadline
                and not any(event.get("event") in {"official_connect_budget_rejected", "tunnel_byte_budget_exhausted"}
                            for event in tunnel.events), "network_budget_exhausted")


def history_snapshot(value, session, turn, reason):
    require(type(value) is dict and type(value.get("updates")) is list, "history_shape")
    updates = value["updates"]
    require(len(updates) <= 4096 and value.get("hasMore") is False
            and type(value.get("totalCount")) is int and value["totalCount"] == len(updates), "history_truncated")
    previous, last, watermark, category, cancelled_tool = -1, None, None, None, None
    text, tools, terminal, raw_outputs = [], {}, {}, set()
    text_after_cancelled_tool = False
    for record in updates:
        require(type(record) is dict and type(record.get("params")) is dict, "history_record")
        params = record["params"]
        update, meta = params.get("update"), params.get("_meta")
        require(params.get("sessionId") == session and type(update) is dict and type(meta) is dict, "history_identity")
        event = meta.get("eventId")
        require(type(event) is str and event.startswith(session + "-"), "history_event_id")
        sequence = event[len(session) + 1:]
        require(bool(re.fullmatch(r"0|[1-9][0-9]{0,19}", sequence))
                and int(sequence) <= (1 << 64) - 1 and int(sequence) > previous, "history_event_order")
        previous, last = int(sequence), event
        ours = meta.get("promptId") == turn
        if record.get("method") == "session/update" and ours:
            kind = update.get("sessionUpdate")
            if kind == "agent_message_chunk":
                require(watermark is None and type(update.get("content")) is dict
                        and update["content"].get("type") == "text"
                        and type(update["content"].get("text")) is str, "history_text")
                text_after_cancelled_tool |= any(status in {"failed", "cancelled"} for status in terminal.values())
                text.append(update["content"]["text"])
            if kind == "tool_call":
                call = update.get("toolCallId")
                require(identifier(call) and call not in tools and watermark is None, "history_tool_identity")
                tools[call] = update
            if kind in ("tool_call", "tool_call_update"):
                require(watermark is None, "history_tool_after_terminal")
                call = update.get("toolCallId")
                require(identifier(call), "history_tool_identity")
                if update.get("status") in ("completed", "failed", "cancelled"):
                    require(call not in terminal, "history_duplicate_tool_terminal")
                    terminal[call] = update["status"]
                if "rawOutput" in update:
                    raw_outputs.add(call)
        if record.get("method") == "_x.ai/session/update" and update.get("sessionUpdate") == "turn_completed" and update.get("prompt_id") == turn:
            require(watermark is None and update.get("stop_reason") == reason, "history_terminal_conflict")
            watermark = event
            category = meta.get("cancellationCategory")
            cancelled_tool = meta.get("cancellationContext", {}).get("tool_name")
    require(value.get("lastEventId") == last, "history_last_event")
    require(set(terminal) <= set(tools) and raw_outputs <= set(tools), "history_orphan_tool_update")
    output = "".join(text)
    require(len(output.encode()) <= MAX_LINE, "history_text_budget")
    return {"output": output, "watermark": watermark, "tools": tools, "terminal": terminal, "raw_outputs": raw_outputs, "category": category, "cancelled_tool": cancelled_tool,
            "text_after_cancelled_tool": text_after_cancelled_tool}


def contains_unique_read_marker(output, secret):
    # 证明文件内容已返回，不把模型额外的排版文字误当成协议或工具失败。
    # 限制总长度且拒绝多个／更长标记，标记本身从不出现在提交给模型的输入中。
    return (len(output.encode()) <= 2048
            and re.findall(r"(?<![A-Za-z0-9_])[0-9a-f]{64}(?![A-Za-z0-9_])", output) == [secret])


class Wire:
    """仅持有本次子进程组；二进制读取有总字节和帧数上限。"""

    def __init__(self, argv, environment, directory, guard=lambda: None):
        self.guard = guard
        self.process = subprocess.Popen(argv, cwd=directory, env=environment, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        self.messages, self.errors = queue.Queue(), []
        self.bytes, self.frames = 0, 0
        self.lock = threading.Lock()
        self.readers = [threading.Thread(target=self.read, args=(name,), daemon=True) for name in ("stdout", "stderr")]
        for reader in self.readers:
            reader.start()

    def read(self, name):
        try:
            channel = getattr(self.process, name)
            while line := channel.readline(MAX_LINE + 1):
                with self.lock:
                    self.bytes += len(line)
                    self.frames += 1
                    require(len(line) <= MAX_LINE and self.bytes <= MAX_BYTES and self.frames <= MAX_FRAMES,
                            "native_output_budget")
                if name == "stdout":
                    value = parse(line)
                    require(type(value) is dict, "native_frame_not_object")
                    self.messages.put(value)
                else:
                    # stderr 仅累计字节；原文不进入异常、终端或安全报告。
                    line.decode("utf-8", errors="strict")
        except Exception as error:
            self.errors.append(type(error).__name__)

    def send(self, value):
        self.guard()
        data = encoded(value) + b"\n"
        require(len(data) <= MAX_LINE and self.process.poll() is None, "native_send_closed")
        self.process.stdin.write(data)
        self.process.stdin.flush()

    def receive(self, deadline):
        while time.monotonic() < deadline:
            self.guard()
            require(not self.errors, "native_reader_failed")
            try:
                return self.messages.get(timeout=min(0.1, max(0.001, deadline - time.monotonic())))
            except queue.Empty:
                require(self.process.poll() is None, "native_exit_before_response")
        raise Rejected("native_response_timeout")

    def group_alive(self):
        try:
            os.killpg(self.process.pid, 0)
            return True
        except ProcessLookupError:
            return False

    def close(self, graceful):
        forced = False
        try:
            self.process.stdin.close()
        except (OSError, BrokenPipeError):
            pass
        try:
            self.process.wait(timeout=5 if graceful else 0.1)
        except subprocess.TimeoutExpired:
            forced = True
        if self.group_alive():
            forced = True
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            return False
        for reader in self.readers:
            reader.join(timeout=2)
        clean = not self.group_alive() and not any(reader.is_alive() for reader in self.readers) and not self.errors
        for name, reader in zip(("stdout", "stderr"), self.readers):
            if not reader.is_alive():
                getattr(self.process, name).close()
        return clean and (not graceful or not forced and self.process.returncode == 0)


class Ledger:
    def __init__(self, root, evidence, deadline):
        self.root, self.evidence, self.deadline = root, evidence, deadline
        self.requests, self.inputs, self.processes, self.written_inputs = 0, 0, 0, 0
        self.config_guard = lambda: None
        self.opening_sessions = set()
        self.next_id, self.pending, self.responses = 1, {}, set()
        self.permission_ids, self.event_ids, self.turns = set(), set(), set()
        self.session, self.turn, self.call, self.phase = None, None, None, None
        self.allowed, self.cancel_sent, self.started = None, False, False
        self.partial, self.tool_count, self.permission_count = "", 0, 0
        self.finished = False

    def request(self, wire, method, params):
        require(method in {"initialize", "authenticate", "session/new", "session/load", "session/prompt", "_x.ai/session/updates"},
                "request_method_not_allowed")
        require(self.requests < MAX_REQUESTS and time.monotonic() < self.deadline, "request_budget")
        if method == "session/prompt":
            self.config_guard()
            require(self.inputs < MAX_INPUTS and not self.pending, "input_budget_or_pending")
            create_file(self.root / "cases" / f"{self.phase}.json", encoded({"phase": self.phase, "input": self.inputs + 1}))
            self.inputs += 1
        identifier = self.next_id
        self.next_id += 1
        self.requests += 1
        self.pending[identifier] = method
        wire.send({"jsonrpc": "2.0", "id": identifier, "method": method, "params": params})
        if method == "session/prompt":
            self.written_inputs += 1
        return identifier

    def receive(self, wire, expected, seconds=120):
        deadline = min(self.deadline, time.monotonic() + seconds)
        while True:
            value = wire.receive(deadline)
            try:
                require(value.get("jsonrpc") == "2.0", "jsonrpc_version")
                if "method" in value:
                    self.notification(wire, value)
                    continue
                if internal_skills_reload_success(value):
                    self.evidence["internal_reload_responses"] = self.evidence.get("internal_reload_responses", 0) + 1
                    continue
                identifier = value.get("id")
                require(type(identifier) is int and identifier == expected and identifier in self.pending
                        and identifier not in self.responses and "error" not in value
                        and type(value.get("result")) is dict, "rpc_response_unowned_or_error")
                self.responses.add(identifier)
                del self.pending[identifier]
                return value["result"]
            except (Rejected, KeyError, TypeError):
                self.evidence["last_frame_shape"] = diagnostic_shape(value)
                raise

    def rpc(self, wire, method, params, seconds=20):
        return self.receive(wire, self.request(wire, method, params), seconds)

    def notification(self, wire, value):
        method, params = value.get("method"), value.get("params")
        require(type(method) is str and type(params) is dict and "result" not in value and "error" not in value,
                "notification_shape")
        if "id" in value:
            if method != "session/request_permission":
                wire.send({"jsonrpc": "2.0", "id": value["id"], "error": {"code": -32601, "message": "Unsupported client method"}})
                raise Rejected("unexpected_reverse_request")
            require(type(value["id"]) in (str, int) and identifier(str(value["id"])), "reverse_request_id_invalid")
            key = encoded(value["id"])
            target = self.root / "project" / ("allow.txt" if self.phase == PHASES[0] else "deny.txt")
            safe = (self.phase in PHASES[:2] and key not in self.permission_ids and self.permission_count == 0
                    and exact_permission(value, self.session, self.turn, self.call, target, self.root / "project"))
            if not safe:
                self.evidence["permission_comparison"] = permission_summary(value, self.session, self.turn, self.call, target, self.root / "project")
                wire.send({"jsonrpc": "2.0", "id": value["id"], "result": {"outcome": {"outcome": "cancelled"}}})
                raise Rejected("unexpected_permission_rejected")
            self.permission_ids.add(key)
            self.permission_count += 1
            option = "allow-once" if self.phase == PHASES[0] else "reject-once"
            self.allowed = self.phase == PHASES[0]
            wire.send({"jsonrpc": "2.0", "id": value["id"], "result": {"outcome": {"outcome": "selected", "optionId": option}}})
            return
        if method == "_x.ai/sessions/changed":
            # 官方会话目录广播没有当前回合语义；只计数/摘要，不读取其中会话或完成字段。
            self.evidence["session_roster_notifications"] = self.evidence.get("session_roster_notifications", 0) + 1
            self.evidence["last_session_roster_params"] = summary(params)
            return
        if method == "_x.ai/settings/update":
            # 原生远端设置广播只供界面显示；不能改探针权限、绑定回合或替代终态。
            self.evidence["settings_notifications"] = self.evidence.get("settings_notifications", 0) + 1
            self.evidence["last_settings_params"] = summary(params)
            return
        if method == "_x.ai/announcements/update":
            # 公告广播没有回合语义；正文只留摘要，不展示或执行其中内容。
            self.evidence["announcement_notifications"] = self.evidence.get("announcement_notifications", 0) + 1
            self.evidence["last_announcement_params"] = summary(params)
            return
        if method in {"_x.ai/mcp/servers_updated", "_x.ai/models/update", "_x.ai/mcp_initialized", "_x.ai/mcp/init_progress", "_x.ai/mcp/server_status"}:
            if method == "_x.ai/mcp/servers_updated":
                require(params.get("mcpServers") == [], "unexpected_mcp_server")
            return
        require(method in {"session/update", "_x.ai/session/update", "_x.ai/queue/changed", "_x.ai/session_notification", "_x.ai/session/prompt_complete"},
                "unknown_native_notification")
        session = params.get("sessionId")
        require(session_identifier(session) and (self.session is None or session == self.session), "notification_session")
        if self.session is None:
            self.opening_sessions.add(session)
        meta = params.get("_meta", {})
        require(type(meta) is dict, "notification_meta")
        if method == "_x.ai/session/update":
            # 原生 Load 使用这个名字回放旧结束事件；不能将它当成当前回合完成。
            require(meta.get("isReplay") is True, "history_alias_without_replay")
        if meta.get("isReplay") is True:
            require(self.phase is None, "replay_during_new_input")
            return
        event = meta.get("eventId")
        if event is not None:
            require(type(event) is str and event not in self.event_ids, "duplicate_native_event")
            self.event_ids.add(event)
        if method == "_x.ai/queue/changed":
            entries = params.get("entries")
            require(type(entries) is list and len(entries) <= 1 and all(type(item) is dict for item in entries), "unexpected_queue")
            if self.phase is not None and self.turn is None and len(entries) == 1 and entries[0].get("kind") == "prompt" and params.get("runningPromptId") is None:
                turn = entries[0].get("id")
                require(identifier(turn) and turn not in self.turns, "old_or_invalid_prompt")
                self.turn = turn
                self.turns.add(turn)
            running = params.get("runningPromptId")
            if self.phase is not None and running is not None:
                require(running == self.turn, "running_prompt_mismatch")
                self.started = True
            if self.phase is None:
                self.turns.update(item["id"] for item in entries if type(item) is dict and identifier(item.get("id")))
            return
        update = params.get("update", {})
        require(type(update) is dict, "update_shape")
        if method == "session/update":
            kind = update.get("sessionUpdate")
            if kind in {"available_commands_update", "current_mode_update", "config_option_update", "session_info_update", "plan", "usage_update"}:
                return
            require(not self.finished and self.phase is not None and self.turn is not None and meta.get("promptId") == self.turn,
                    "update_turn_mismatch")
            if kind in {"agent_message_chunk", "agent_thought_chunk"}:
                content = update.get("content")
                require(type(content) is dict and content.get("type") == "text" and type(content.get("text")) is str,
                        "text_shape")
                self.started = True
                if kind == "agent_message_chunk":
                    self.partial += content["text"]
                    require(len(self.partial.encode()) <= MAX_LINE, "turn_text_budget")
                    if self.phase == PHASES[2] and content["text"] and not self.cancel_sent:
                        self.cancel_sent = True
                        wire.send({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": self.session}})
                return
            require(kind in {"tool_call", "tool_call_update"} and self.phase in PHASES[:2], "unexpected_native_tool")
            call = update.get("toolCallId")
            require(identifier(call), "tool_id_invalid")
            if kind == "tool_call":
                require(self.tool_count == 0, "extra_tool_call")
                self.call, self.tool_count = call, 1
            require(call == self.call, "tool_call_mismatch")
            if update.get("status") == "completed" or "rawOutput" in update:
                require(self.allowed is True, "tool_executed_without_allow")
            return
        # 终态通知不能替代本回合 session/prompt 的 RPC 及持久回放。
        if method == "_x.ai/session/prompt_complete":
            require(self.phase is not None and params.get("promptId") == self.turn, "completion_turn_mismatch")
        elif update.get("sessionUpdate") == "turn_completed":
            require(self.phase is not None and update.get("prompt_id") == self.turn, "completion_turn_mismatch")
        elif update.get("sessionUpdate") not in {"model_changed", "session_info_update", "session_title_update", "session_summary_generated", "turn_started", "turn_created"}:
            # 与生产适配器一致：非终态扩展只作诊断。标准工具事件、审批、RPC 和历史仍须独立验证。
            self.evidence["uninterpreted_session_metadata"] = self.evidence.get("uninterpreted_session_metadata", 0) + 1
            self.evidence["last_session_metadata_shape"] = summary(update)

    def open(self, wire, previous=None):
        self.phase, self.turn, self.call = None, None, None
        self.finished = False
        self.session = previous
        self.opening_sessions.clear()
        initialized = self.rpc(wire, "initialize", {"protocolVersion": 1, "clientCapabilities": {
            "fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False}})
        require(initialized.get("protocolVersion") == 1 and type(initialized.get("protocolVersion")) is int
                and initialized.get("_meta", {}).get("agentVersion") == VERSION
                and initialized.get("agentCapabilities", {}).get("loadSession") is True
                and initialized.get("agentCapabilities", {}).get("promptCapabilities", {}).get("image") is False,
                "native_identity_or_capability")
        require(any(type(item) is dict and item.get("id") == "cached_token" for item in initialized.get("authMethods", [])),
                "cached_auth_not_advertised")
        self.rpc(wire, "authenticate", {"methodId": "cached_token", "_meta": {"headless": True}})
        params = {"cwd": str(self.root / "project"), "mcpServers": [], "_meta": {"yoloMode": False, "autoMode": False}}
        if previous is not None:
            params["sessionId"] = previous
        result = self.rpc(wire, "session/new" if previous is None else "session/load", params)
        if previous is None:
            require(session_identifier(result.get("sessionId")), "native_new_session_invalid")
            self.session = result["sessionId"]
            require(self.opening_sessions <= {self.session}, "early_opening_session_conflict")
        else:
            for candidate in (result.get("sessionId"), result.get("_meta", {}).get("sessionId")):
                require(candidate is None or candidate == previous, "native_load_changed_session")
        return self.session

    def final_history(self, wire, reason):
        for attempt in range(4):
            value = self.rpc(wire, "_x.ai/session/updates", {"sessionId": self.session,
                "cwd": str(self.root / "project"), "offset": 0, "limit": 4096})
            self.evidence["last_history_shape"] = diagnostic_shape(value)
            snapshot = history_snapshot(value, self.session, self.turn, reason)
            if snapshot["watermark"] is not None:
                require(snapshot["output"].startswith(self.partial), "history_output_stream_mismatch")
                return snapshot
            if attempt < 3:
                time.sleep(0.25)
        raise Rejected("history_completion_missing")

    def run_phase(self, wire, phase, prompt, secret):
        require(self.inputs < MAX_INPUTS and phase == PHASES[self.inputs] and secret not in prompt
                and private_file(self.root / "project/deny.txt").decode() not in prompt, "phase_order_or_secret_in_prompt")
        self.phase, self.turn, self.call = phase, None, None
        self.partial, self.tool_count, self.permission_count = "", 0, 0
        self.allowed, self.cancel_sent, self.started, self.finished = None, False, False, False
        result = self.receive(wire, self.request(wire, "session/prompt", {
            "sessionId": self.session, "prompt": [{"type": "text", "text": prompt}]}))
        require(self.turn is not None and self.started and result.get("_meta", {}).get("promptId") == self.turn,
                "prompt_receipt_identity")
        require(result.get("_meta", {}).get("sessionId", self.session) == self.session, "prompt_session_changed")
        reason = "end_turn" if phase in (PHASES[0], PHASES[3]) else "cancelled"
        require(result.get("stopReason") == reason, "prompt_stop_reason")
        if phase in (PHASES[1], PHASES[2]):
            category = "PermissionRejected" if phase == PHASES[1] else "MidTurnAbort"
            require(result.get("_meta", {}).get("cancellationCategory") == category, "cancel_category")
        snapshot = self.final_history(wire, reason)
        self.evidence["last_turn_result"] = {
            "output_bytes": len(snapshot["output"].encode()),
            "output_sha256": sha(snapshot["output"].encode()),
            "exact_read_marker": snapshot["output"] == secret,
            "unique_read_marker": contains_unique_read_marker(snapshot["output"], secret),
            "tool_completed": snapshot["terminal"].get(self.call) == "completed",
        }
        if phase in (PHASES[1], PHASES[2]):
            require(snapshot["category"] == category, "history_cancel_category")
            if phase == PHASES[1]:
                require(snapshot["cancelled_tool"] == "read_file", "history_cancel_tool")
        require(private_file(self.root / "project/deny.txt").decode() not in encoded(result).decode()
                and private_file(self.root / "project/deny.txt").decode() not in snapshot["output"], "denied_secret_disclosed")
        if phase in PHASES[:2]:
            require(self.permission_count == self.tool_count == 1 and set(snapshot["tools"]) == {self.call}, "approval_not_observed")
            if phase == PHASES[0]:
                require(snapshot["terminal"].get(self.call) == "completed", "allowed_read_tool_not_completed")
                require(contains_unique_read_marker(snapshot["output"], secret), "allowed_read_output_mismatch")
            else:
                require(snapshot["terminal"].get(self.call) in {"failed", "cancelled"}
                        and self.call not in snapshot["raw_outputs"] and not snapshot["text_after_cancelled_tool"], "denied_read_executed")
        else:
            require(self.tool_count == self.permission_count == 0 and not snapshot["tools"], "unexpected_tools")
            if phase == PHASES[2]:
                require(self.cancel_sent and bool(self.partial), "cancel_not_sent_after_text")
            else:
                require(contains_unique_read_marker(snapshot["output"], secret), "cold_recall_mismatch")
        self.evidence["cases"].append({"phase": phase, "passed": True, "input": self.inputs,
            "native_session_sha256": sha(self.session.encode()), "native_turn_sha256": sha(self.turn.encode()),
            "permissions": self.permission_count, "tools": self.tool_count, "cancel_sent": self.cancel_sent,
            "stop_reason": reason, "history_verified": True, "output_bytes": len(snapshot["output"].encode()),
            "output_sha256": sha(snapshot["output"].encode()), "completion_watermark_sha256": sha(snapshot["watermark"].encode())})
        self.config_guard()
        self.finished = True
        return self.turn

    def drain(self, wire):
        while not wire.messages.empty():
            value = wire.messages.get_nowait()
            if internal_skills_reload_success(value):
                self.evidence["internal_reload_responses"] = self.evidence.get("internal_reload_responses", 0) + 1
                continue
            require("method" in value, "late_or_duplicate_rpc_after_eof")
            self.notification(wire, value)


def environment(root, port):
    for relative in PRIVATE_DIRECTORIES:
        private_directory(root / relative)
    env = {key: value for key, value in os.environ.items() if key in {"LANG", "LC_ALL"}}
    directories = {"HOME": "home", "USERPROFILE": "home", "GROK_HOME": "home/.grok",
        "CODEX_HOME": "home/.codex", "CLAUDE_CONFIG_DIR": "home/.claude",
        "XDG_CONFIG_HOME": "home/.config", "XDG_DATA_HOME": "home/.local/share",
        "XDG_STATE_HOME": "home/.local/state", "XDG_CACHE_HOME": "home/.cache",
        "APPDATA": "home/AppData/Roaming", "LOCALAPPDATA": "home/AppData/Local",
        "TMPDIR": "tmp", "TMP": "tmp", "TEMP": "tmp"}
    env.update({key: str(root / relative) for key, relative in directories.items()})
    env.update(PATH="/usr/bin:/bin:/usr/sbin:/sbin", HTTPS_PROXY=f"http://127.0.0.1:{port}",
        HTTP_PROXY=f"http://127.0.0.1:{port}", NO_PROXY="", GROK_DISABLE_API_KEY_AUTH="1",
        GROK_AUTO_UPDATE="0", GROK_DISABLE_AUTOUPDATER="1", GROK_CLAUDE_HOOKS_ENABLED="0",
        GROK_CLAUDE_MCPS_ENABLED="0", GROK_CODEX_HOOKS_ENABLED="0", GROK_CODEX_MCPS_ENABLED="0")
    return env


def sandbox_canary(root, source_home, profile, env):
    # 只打开来源目录自身；不枚举或读取认证内容。外部写入目标是本次预建的合成哨兵。
    outside = root.parent / (root.name + "-outside-canary")
    create_file(outside, b"preserve")
    code = '''import errno,json,os,socket,sys
r={}
try:fd=os.open(sys.argv[1],os.O_RDONLY);os.close(fd);r['auth_source_read_blocked']=False
except OSError as e:r['auth_source_read_blocked']=e.errno==errno.EPERM
try:fd=os.open(sys.argv[2],os.O_WRONLY);os.close(fd);r['outside_write_blocked']=False
except OSError as e:r['outside_write_blocked']=e.errno==errno.EPERM
s=socket.socket();s.settimeout(1)
try:s.connect(('1.1.1.1',443));r['direct_network_blocked']=False
except OSError as e:r['direct_network_blocked']=e.errno==errno.EPERM
finally:s.close()
print(json.dumps(r))
'''
    try:
        completed = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(profile), sys.executable, "-c", code,
            str(source_home), str(outside)], cwd=root, env=env, capture_output=True, timeout=10, check=True)
        require(parse(completed.stdout) == {"auth_source_read_blocked": True, "outside_write_blocked": True,
                "direct_network_blocked": True} and outside.read_bytes() == b"preserve", "sandbox_canary_failed")
    finally:
        outside.unlink(missing_ok=True)


def cleanup(wire, tunnel, auth, evidence):
    clean = True
    try:
        if wire is not None:
            clean = wire.close(False) and clean
    except Exception:
        clean = False
    finally:
        try:
            if tunnel is not None:
                clean = tunnel.close() is True and clean
        except Exception:
            clean = False
        finally:
            try:
                auth.unlink(missing_ok=True)
                evidence["auth_copy_removed"] = not auth.exists()
            except OSError:
                evidence["auth_copy_removed"] = False
            clean = clean and evidence["auth_copy_removed"]
    evidence["cleanup_confirmed"] = clean
    return clean


def audit_native_settings(after):
    before = configuration()
    audit = official.audit_private_settings(before, after)
    purge_only = False
    if audit["toml_parse_succeeded"]:
        initial = tomllib.loads(before.decode("utf-8"))
        current = tomllib.loads(after.decode("utf-8"))
        # 仅固定 34 已观察到的单字段初始化；不接受完整市场初始化的任意子集。
        purge_only = ("marketplace" not in initial
            and encoded(current.get("marketplace")) == encoded({"default_skills_installs_purged": True})
            and encoded({key: value for key, value in current.items() if key != "marketplace"}) == encoded(initial))
    audit["native_marketplace_purge_only"] = purge_only
    audit["native_initialization_state"] = (
        "purge_only_1_0_34" if purge_only else
        "full_marketplace" if audit["native_marketplace_initialization_only"] else
        "unchanged" if audit["settings_scope_verified"] else "unverified")
    audit["settings_scope_verified"] = audit["settings_scope_verified"] or purge_only
    return audit


def verify_fixture(root, manifest, evidence=None):
    require(snapshots(root) == manifest["snapshots"], "sentinel_changed")
    require({path.name for path in (root / "project").iterdir()} == {"allow.txt", "deny.txt"}, "unexpected_project_write")
    audit = audit_native_settings(private_file(root / "home/.grok/config.toml"))
    if evidence is not None:
        evidence["config_audit"] = audit
    require(audit["settings_scope_verified"] and audit["permission_section_unchanged"], "native_config_changed")
    return audit


def run(root, binary, source_home, confirmation):
    require(confirmation == CONFIRM and os.environ.get(RUN_ENV) == "1", "explicit_run_authorization_missing")
    require(sys.platform == "darwin", "native_platform_unprepared")
    manifest = validate_manifest(root, binary, source_home)
    require(official.MAX_TUNNELS == MAX_TLS_CONNECTIONS and official.MAX_BYTES == MAX_TLS_BYTES, "tunnel_contract_changed")
    create_file(root / "invocation.json", encoded({"scope": SCOPE, "manifest_sha256": sha(encoded(manifest)), "started_at": time.time()}))
    result_path = root / "result.safe.json"
    result_descriptor = os.open(result_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
    evidence = {"scope": SCOPE, "candidate_unverified": True, "passed": False, "prepared_only": False,
        "production_runtime_verified": False, "full_cli_parity_acceptance_passed": False,
        "model_http_calls_observed": False, "model_inputs_sent": 0, "cases": [],
        "auth_copy_removed": False, "cleanup_confirmed": False, "normal_eof_count": 0,
        "max_native_inputs": MAX_INPUTS, "raw_acp_only": True, "binary": manifest["binary_evidence"],
        "version_output": manifest["version_output"], "manifest_sha256": sha(encoded(manifest)), "helpers": manifest["helpers"]}
    wire, tunnel = None, None
    auth = root / "home/.grok/auth.json"
    ledger = Ledger(root, evidence, time.monotonic() + MAX_SECONDS)
    ledger.config_guard = lambda: verify_fixture(root, manifest, evidence)
    try:
        require(fixed.verify_version(binary, root, VERSION) == fixed.VERSION_OUTPUTS[VERSION], "version_probe_failed")
        tunnel = official.OfficialTunnel(MAX_SECONDS)
        port = tunnel.start()
        env = environment(root, port)
        profile = root / "native.sb"
        sandbox = official.shared.sandbox_profile(root, source_home, port)
        # 来源在 /tmp 时仍须额外禁止读取真实用户 HOME；固定原生文件必须在用户 HOME 外。
        user_home = Path.home().resolve()
        require(not binary.is_relative_to(user_home) and not root.is_relative_to(user_home), "fixture_overlaps_user_home")
        sandbox += '(deny file-read* (subpath ' + json.dumps(str(user_home)) + '))'
        create_file(profile, sandbox.encode())
        sandbox_canary(root, source_home, profile, env)
        official.copy_private_auth(source_home, root / "home/.grok")
        secret = private_file(root / "project/allow.txt").decode("ascii")
        require(bool(re.fullmatch(r"[0-9a-f]{64}", secret)), "fixture_secret_invalid")
        original_session, first_turn = None, None
        for process_index in range(MAX_PROCESSES):
            require(wire is None and time.monotonic() < ledger.deadline, "previous_process_not_exited")
            ledger.processes += 1
            wire = Wire(["/usr/bin/sandbox-exec", "-f", str(profile), str(binary), "agent", "--no-leader", "stdio"],
                        env, root / "project", guard=lambda: network_guard(tunnel))
            session = ledger.open(wire, original_session)
            if process_index == 0:
                original_session = session
                first_turn = ledger.run_phase(wire, PHASES[0], "只用 read_file 读取 allow.txt 一次，参数只含 target_file=allow.txt。不要其他工具。成功后只输出文件唯一一行的原文，不解释。", secret)
                ledger.run_phase(wire, PHASES[1], "只用 read_file 读取 deny.txt 一次，参数只含 target_file=deny.txt。等待审批；拒绝后不要重试或使用其他工具。", secret)
                ledger.run_phase(wire, PHASES[2], "不要调用工具。先输出 READY，然后逐个列出从 1 到 100000 的整数。", secret)
            else:
                require(session == original_session, "cold_session_changed")
                history = ledger.rpc(wire, "_x.ai/session/updates", {"sessionId": session, "cwd": str(root / "project"), "offset": 0, "limit": 4096})
                restored = history_snapshot(history, session, first_turn, "end_turn")
                require(restored["watermark"] is not None
                        and sha(restored["output"].encode()) == evidence["cases"][0]["output_sha256"],
                        "cold_history_not_restored")
                ledger.run_phase(wire, PHASES[3], "不要调用任何工具，只回复第一轮已经读取的文件唯一一行原文。", secret)
            require(wire.close(True), "native_eof_or_process_cleanup_failed")
            ledger.drain(wire)
            wire = None
            evidence["normal_eof_count"] += 1
            evidence["config_audit"] = verify_fixture(root, manifest)
            require(validate_binary(binary) == manifest["binary_evidence"], "binary_changed")
        require(ledger.inputs == ledger.written_inputs == MAX_INPUTS and ledger.processes == MAX_PROCESSES
                and len(evidence["cases"]) == 4 and not ledger.pending, "incomplete_p0_chain")
        evidence["passed"] = True
    except Exception as error:
        evidence["failure_code"] = str(error) if isinstance(error, Rejected) else type(error).__name__
        evidence["failed_phase"] = ledger.phase
        evidence["active_phase_shape"] = {"claimed": ledger.inputs, "written": ledger.written_inputs,
            "native_turn_bound": ledger.turn is not None, "started": ledger.started, "permissions": ledger.permission_count,
            "tools": ledger.tool_count, "cancel_sent": ledger.cancel_sent}
    finally:
        evidence.update(model_inputs_sent=ledger.written_inputs, model_inputs_claimed=ledger.inputs, protocol_requests=ledger.requests,
                        native_session_processes=ledger.processes)
        if tunnel is not None:
            evidence["network"] = {"connections": tunnel.forwarded, "tls_bytes": tunnel.bytes,
                "tls_decrypted": False, "max_connections": MAX_TLS_CONNECTIONS, "max_tls_bytes": MAX_TLS_BYTES}
            evidence["passed"] &= (tunnel.forwarded <= MAX_TLS_CONNECTIONS and tunnel.bytes <= MAX_TLS_BYTES
                and not any(event.get("event") in {"official_connect_budget_rejected", "tunnel_byte_budget_exhausted"} for event in tunnel.events))
        evidence["passed"] &= cleanup(wire, tunnel, auth, evidence)
        if tunnel is not None:
            evidence["network"]["connections"] = tunnel.forwarded
            evidence["network"]["tls_bytes"] = tunnel.bytes
            evidence["network"]["rejected_origins"] = sum(event.get("event") == "rejected_nonofficial_origin" for event in tunnel.events)
            try:
                network_guard(tunnel)
            except Rejected:
                evidence["passed"] = False
        with os.fdopen(result_descriptor, "wb") as result_file:
            current = result_path.lstat()
            owned = os.fstat(result_file.fileno())
            evidence["passed"] &= (stat.S_ISREG(current.st_mode) and current.st_nlink == 1
                and (current.st_dev, current.st_ino) == (owned.st_dev, owned.st_ino) and current.st_size == 0)
            result_file.write(encoded(evidence))
            result_file.flush()
            os.fsync(result_file.fileno())
    return evidence


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture-root", type=Path, required=True)
    parser.add_argument("--grok", type=Path, required=True)
    parser.add_argument("--official-grok-home", type=Path, required=True)
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--confirm")
    args = parser.parse_args(argv)
    try:
        if args.run:
            report = run(args.fixture_root, args.grok, args.official_grok_home, args.confirm)
        else:
            require(args.confirm is None, "confirmation_without_run")
            report = prepare(args.fixture_root, args.grok, args.official_grok_home)
        print(json.dumps(report, ensure_ascii=False))
        return 0 if report.get("prepared_only") or report.get("passed") else 1
    except (OSError, ValueError, TypeError, subprocess.SubprocessError):
        sys.stderr.write("Grok 原始 ACP P0 候选未运行或未通过严格检查；没有开放产品能力。\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
