#!/usr/bin/env python3
"""独立 Grok 1.0.41 原生 ACP 零输入预检；只公开有界安全摘要。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time
import uuid

sys.dont_write_bytecode = True
import run_grok_official_adapter_live as official
import run_grok_policy_preflight as policy
import run_grok_sdk_origin_probe as isolation
import run_grok_selected_skill as selected_skill

VERSION = "grok 1.0.41 (4220f3b224a6)"
BINARY_BYTES = 145657952
BINARY_SHA256 = "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d"
SCOPE = "grok_1_0_41_zero_input_p0"
MAX_SECONDS = 120
MAX_LINE = 1024 * 1024
MAX_TOTAL = 4 * 1024 * 1024
MAX_FRAMES = 256
REQUESTS = ("initialize", "authenticate", "session/new")
NOTIFICATIONS = {"_x.ai/mcp/servers_updated", "_x.ai/models/update", "_x.ai/session/setup",
    "session/update", "_x.ai/session_notification", "_x.ai/mcp_initialized", "_x.ai/mcp/init_progress",
    "_x.ai/mcp/server_status"}


class ProbeRejected(ValueError):
    """异常仅承载固定原因代码，避免原生正文进入公开输出。"""


def require(condition, reason):
    if not condition:
        raise ProbeRejected(reason)


def digest(value):
    return hashlib.sha256(value).hexdigest()


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, "duplicate_json_key")
        result[key] = value
    return result


def decode_frame(raw):
    require(len(raw) <= MAX_LINE and raw.endswith(b"\n"), "frame_limit")
    try:
        value = json.loads(raw, object_pairs_hook=pairs,
            parse_constant=lambda _: (_ for _ in ()).throw(ProbeRejected("nonfinite_json")))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ProbeRejected("invalid_json") from error
    require(type(value) is dict and value.get("jsonrpc") == "2.0", "invalid_frame")
    return value


def binary_identity(path):
    require(path.is_absolute() and path.is_file(), "binary_missing")
    target = path.resolve(strict=True)
    attributes = target.stat()
    require(stat.S_ISREG(attributes.st_mode) and attributes.st_size == BINARY_BYTES,
        "binary_size_mismatch")
    require(official.shared.digest(target) == BINARY_SHA256, "binary_digest_mismatch")
    return target


def prepare_wrapper(root, binary, source_home, port):
    wrapper, settings = official.prepare_native(root, binary, source_home, port,
        binary_sha256=BINARY_SHA256, model="grok-4.7")
    code = wrapper.read_text(encoding="utf-8")
    leader = """elif len(args)==4 and args[:3]==['agent','stdio','--leader-socket']:
 endpoint=Path(args[3]);resolved=endpoint.resolve()
 if not endpoint.is_absolute() or endpoint!=resolved or not resolved.is_relative_to(socket_root) or resolved.name!='leader.sock' or resolved.exists():raise SystemExit(92)
 parent=resolved.parent
 if parent.stat().st_uid!=os.getuid() or parent.stat().st_mode & 0o077:raise SystemExit(93)
 quoted=json.dumps(str(resolved))
 profile+='(allow file-write* (subpath '+json.dumps(str(socket_root))+'))(allow network-bind network-inbound (literal '+quoted+'))(allow network-outbound (remote unix-socket (path-literal '+quoted+')))'
 kind='private_leader'
"""
    require(code.count(leader) == 1 and code.count("if args==['--version']:") == 1,
        "wrapper_contract_changed")
    code = code.replace(leader,
        "elif args==['agent','--no-leader','stdio']:kind='direct_agent';endpoint=None\n", 1)
    code = code.replace("if args==['--version']:",
        f"profile+={isolation.project_deny(root)!r}\nif args==['--version']:", 1)
    compile(code, str(wrapper), "exec")
    wrapper.write_text(code, encoding="utf-8")
    settings_bytes = settings.read_bytes()
    require(b"auto_update = false" in settings_bytes and b'action = "ask"' in settings_bytes,
        "private_config_invalid")
    return wrapper, settings, settings_bytes


def request(identifier, method, params):
    require(method in REQUESTS, "request_not_allowed")
    return {"jsonrpc": "2.0", "id": identifier, "method": method, "params": params}


def response_summary(method, frame):
    require("error" not in frame and type(frame.get("result")) is dict, "native_rpc_error")
    result = frame["result"]
    if method == "initialize":
        metadata = result.get("_meta")
        require(result.get("protocolVersion") == 1 and type(metadata) is dict and
            metadata.get("agentVersion") == "1.0.41", "initialize_identity_changed")
        methods = result.get("authMethods")
        require(type(methods) is list and any(type(item) is dict and
            item.get("id") == "cached_token" for item in methods), "cached_auth_unavailable")
        capabilities = result.get("agentCapabilities")
        return {"protocol_v1": True, "native_version_matched": True,
            "cached_auth_advertised": True,
            "load_session_advertised": type(capabilities) is dict and
                capabilities.get("loadSession") is True}
    if method == "authenticate":
        return {"cached_auth_accepted": True}
    require(method == "session/new", "unexpected_response")
    native_id = result.get("sessionId")
    try:
        valid = type(native_id) is str and str(uuid.UUID(native_id)) == native_id
    except ValueError:
        valid = False
    require(valid, "native_session_id_invalid")
    return {"new_session_accepted": True, "native_session_id_sha256": digest(native_id.encode())}


def exchange(process, messages, observed, method, params, deadline):
    identifier = len(observed) + 1
    outgoing = request(identifier, method, params)
    raw = json.dumps(outgoing, separators=(",", ":")).encode() + b"\n"
    require(process.poll() is None and time.monotonic() < deadline, "native_process_unavailable")
    process.stdin.write(raw)
    process.stdin.flush()
    observed.append(method)
    while time.monotonic() < deadline:
        try:
            item = messages.get(timeout=min(0.2, max(0.001, deadline - time.monotonic())))
        except queue.Empty:
            require(process.poll() is None, "native_exited_before_response")
            continue
        require(item is not None, "native_stdout_closed")
        frame = decode_frame(item)
        if "id" not in frame:
            native_method = frame.get("method")
            require(type(native_method) is str and native_method in NOTIFICATIONS,
                "unexpected_native_request_or_notification")
            continue
        require(type(frame["id"]) is int and frame["id"] == identifier
            and "method" not in frame, "unmatched_native_response")
        return response_summary(method, frame)
    raise ProbeRejected("native_response_timeout")


def run_native(wrapper, root, environment, timeout):
    version = subprocess.run([str(wrapper), "--version"], cwd=root / "project",
        env=environment, capture_output=True, timeout=10, check=False)
    require(version.returncode == 0 and version.stdout.strip() == VERSION.encode()
        and len(version.stderr) <= MAX_LINE, "version_probe_failed")
    stderr_path = root / "native-stderr.private"
    messages = queue.Queue(maxsize=MAX_FRAMES)
    observed, responses = [], {}
    total = [0]
    reader_error = [False]
    with stderr_path.open("xb") as stderr:
        process = subprocess.Popen([str(wrapper), "agent", "--no-leader", "stdio"],
            cwd=root / "project", env=environment, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=stderr, start_new_session=True)

        def read_frames():
            try:
                for _ in range(MAX_FRAMES + 1):
                    line = process.stdout.readline(MAX_LINE + 1)
                    total[0] += len(line)
                    if total[0] > MAX_TOTAL or len(line) > MAX_LINE:
                        reader_error[0] = True
                        break
                    messages.put(line if line else None, timeout=1)
                    if not line:
                        break
                else:
                    reader_error[0] = True
            except (OSError, queue.Full):
                reader_error[0] = True

        reader = threading.Thread(target=read_frames, daemon=True)
        reader.start()
        deadline = time.monotonic() + timeout
        try:
            requests = (
                ("initialize", {"protocolVersion": 1, "clientCapabilities": {
                    "fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False}}),
                ("authenticate", {"methodId": "cached_token", "_meta": {"headless": True}}),
                ("session/new", {"cwd": str(root / "project"), "mcpServers": []}),
            )
            for method, params in requests:
                responses.update(exchange(process, messages, observed, method, params, deadline))
            require(observed == list(REQUESTS) and not reader_error[0], "request_guard_failed")
            process.stdin.close()
            process.wait(timeout=min(10, max(0.001, deadline - time.monotonic())))
            reader.join(timeout=2)
            require(process.returncode == 0 and not reader.is_alive() and not reader_error[0],
                "native_eof_exit_failed")
            while not messages.empty():
                item = messages.get_nowait()
                if item is None:
                    continue
                frame = decode_frame(item)
                require("id" not in frame and frame.get("method") in NOTIFICATIONS,
                    "unexpected_native_activity_after_session")
            return {**responses, "native_exit_code": 0, "native_eof_exit": True,
                "protocol_request_methods": list(REQUESTS), "model_inputs_sent": 0,
                "business_tool_dispatches": 0, "native_stdout_bytes": total[0]}
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
            if process.stdin and not process.stdin.closed:
                process.stdin.close()
            reader.join(timeout=2)
            if process.stdout:
                process.stdout.close()


def run(args):
    require(sys.platform == "darwin", "platform_unverified")
    require(30 <= args.timeout <= MAX_SECONDS, "deadline_invalid")
    binary = binary_identity(args.grok)
    source_home = args.official_grok_home
    require(source_home.is_absolute() and not source_home.is_symlink() and
        source_home.is_dir() and source_home.stat().st_uid == os.getuid() and
        source_home.stat().st_mode & 0o022 == 0, "auth_home_invalid")
    source_home = source_home.resolve(strict=True)
    before_auth = official.shared.digest(binary)
    import run_grok_native_tool_lease as lease
    auth_before = lease.auth_identity(source_home)
    require(args.output.is_absolute() and args.output.suffix == ".json" and
        not args.output.exists() and not args.output.is_symlink() and
        not args.output.is_relative_to(source_home), "output_invalid")
    result = {"schema": 1, "scope": SCOPE, "platform": "macos-arm64",
        "binary_version": VERSION, "binary_bytes": BINARY_BYTES, "binary_sha256": BINARY_SHA256,
        "zero_input_guard": True, "model_inputs_sent": 0, "passed": False,
        "product_gate_open": False, "full_goal_passed": False,
        "http_model_call_count_measured": False, "tls_decrypted": False}
    root = None
    try:
        with tempfile.TemporaryDirectory(prefix="infinishell-grok-1041-p0-", dir="/private/tmp") as temporary:
            root = Path(temporary).resolve()
            root.chmod(0o700)
            for relative in ("home/.grok", "home/.config", "home/.local/share", "home/.cache",
                    "project", "tmp"):
                (root / relative).mkdir(parents=True, mode=0o700, exist_ok=True)
            (root / "wrapper-audit.ndjson").touch(mode=0o600)
            private_binary = root / "grok-native"
            shutil.copyfile(binary, private_binary)
            private_binary.chmod(0o500)
            require(binary_identity(private_binary) == private_binary, "private_binary_invalid")
            auth_copy = root / "home/.grok/auth.json"
            with policy.bounded_tunnel(official, args.timeout, 16, 8 * 1024 * 1024) as tunnel:
                try:
                    port = tunnel.start()
                    official.copy_private_auth(source_home, root / "home/.grok")
                    result["private_auth_copy_created"] = auth_copy.is_file()
                    result["sandbox_network_canary"] = official.shared.network_canary(
                        root, source_home / "auth.json", port) == {
                        "allowed_proxy": True, "other_loopback": 1, "external": 1,
                        "exact_unix": True, "other_unix": 1, "credential_read": 1}
                    require(result["sandbox_network_canary"], "sandbox_canary_failed")
                    wrapper, settings, settings_before = prepare_wrapper(root, private_binary, source_home, port)
                    environment = official.official_environment(root, port)
                    result.update(run_native(wrapper, root, environment, args.timeout))
                    launches = isolation.private_events(root / "wrapper-audit.ndjson")
                    result["wrapper_launches_verified"] = (
                        len(launches) == 2 and
                        [item.get("kind") for item in launches] == ["version", "direct_agent"] and
                        all(item.get("event") == "native_launch" and
                            item.get("arguments_unchanged") is True and
                            item.get("private_socket") is None for item in launches))
                    require(result["wrapper_launches_verified"], "wrapper_launches_changed")
                    settings_audit = selected_skill.audit_current_settings(
                        settings_before, settings.read_bytes())
                    result["private_settings_audit"] = {
                        key: settings_audit[key] for key in (
                            "bytes_unchanged", "permission_section_unchanged",
                            "native_marketplace_initialization_only", "native_marketplace_purge_only",
                            "settings_scope_verified")}
                    result["private_binary_unchanged"] = (
                        official.shared.digest(private_binary) == BINARY_SHA256)
                    require(settings_audit["settings_scope_verified"] and
                        settings_audit["permission_section_unchanged"] and
                        result["private_binary_unchanged"],
                        "private_inputs_changed")
                finally:
                    auth_copy.unlink(missing_ok=True)
                    result["private_auth_copy_removed"] = not auth_copy.exists()
                    result["original_auth_stat_unchanged"] = lease.auth_identity(source_home) == auth_before
            result["tunnel_closed"] = tunnel.preflight_cleanup_confirmed is True
            result["tls_connections_attempted"] = tunnel.forwarded
            result["tls_bytes"] = tunnel.bytes
            require(tunnel.forwarded <= 16 and tunnel.bytes <= 8 * 1024 * 1024,
                "network_budget_exceeded")
            require(result["private_auth_copy_removed"] and result["original_auth_stat_unchanged"]
                and result["tunnel_closed"], "isolation_cleanup_failed")
        result["private_root_removed"] = not root.exists()
        require(result["private_root_removed"] and official.shared.digest(binary) == before_auth,
            "root_or_binary_changed")
        result["passed"] = True
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        result["failure_code"] = str(error) if type(error) is ProbeRejected else type(error).__name__
        result["private_root_removed"] = root is not None and not root.exists()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(result, output, ensure_ascii=False, indent=2)
        output.write("\n")
    return 0 if result["passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--grok", type=Path, required=True)
    parser.add_argument("--official-grok-home", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=MAX_SECONDS)
    args = parser.parse_args()
    raise SystemExit(run(args))


if __name__ == "__main__":
    main()
