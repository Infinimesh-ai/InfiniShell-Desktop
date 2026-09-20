#!/usr/bin/env python3
"""在 macOS 私有代理和 OS 沙箱内验证生产 Grok 进程/传输；产品模型门禁保持关闭。"""

import argparse
import hashlib
import http.client
import http.server
import json
import os
from pathlib import Path
import re
import secrets
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
from urllib.parse import urlsplit

TEST_NAME = "ai::cli_agent_runtime::grok::live_tests::real_grok_managed_lifecycle"
SCOPE = "production_process_transport_with_runtime_commands"
RECEIPT_SOURCE = "verified_native_final_history"
VERSION = "grok 1.0.30 (04b7ffed98c6)"
BINARY_SHA256 = "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb"
MODEL = "claude-sonnet-4-6"
FIXED_PROMPT = "INFINISHELL_GROK_ADAPTER"
FAKE_KEY = "infinishell-loopback-placeholder-not-a-real-key"
MARKER = "isolated Grok Rust adapter verification\n"


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def load_credentials(path):
    if path.stat().st_size > 65536:
        raise ValueError("显式凭据文件过大")
    value = json.loads(path.read_text(encoding="utf-8"))
    allowed = {"ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "ANTHROPIC_MODEL"}
    if not isinstance(value, dict) or not set(value).issubset(allowed):
        raise ValueError("凭据文件含不支持的键")
    if any(not isinstance(item, str) or not item or any(c in item for c in "\x00\r\n") for item in value.values()):
        raise ValueError("凭据值必须是非空单行字符串")
    if ("ANTHROPIC_API_KEY" in value) == ("ANTHROPIC_AUTH_TOKEN" in value):
        raise ValueError("必须且只能显式提供一种服务凭据")
    if value.get("ANTHROPIC_MODEL", MODEL) != MODEL:
        raise ValueError("此验收只支持已验证的固定模型")
    base = value.get("ANTHROPIC_BASE_URL", "")
    target = urlsplit(base)
    if (target.scheme != "https" or not target.hostname or target.username or target.password
            or target.query or target.fragment or target.path.rstrip("/") not in ("", "/v1")):
        raise ValueError("只允许显式指定的单一 HTTPS origin 与可选 /v1 前缀")
    return value, target


def sanitizer(credentials, root, fake_key=FAKE_KEY):
    secrets = [credentials.get(key, "") for key in ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL")]
    base = credentials.get("ANTHROPIC_BASE_URL", "")
    if base:
        secrets.extend([urlsplit(base).netloc, urlsplit(base).hostname or ""])
    def clean(text):
        for secret in secrets:
            if secret:
                text = text.replace(json.dumps(secret, ensure_ascii=False)[1:-1], "<redacted>").replace(secret, "<redacted>")
        return text.replace(str(root), "<probe-root>").replace(fake_key, "<dummy-key>")
    return clean


def native_environment(root, fake_key=FAKE_KEY):
    # 真密钥与用户登录环境不传入 libtest、监督者、包装器或原生 Grok。
    keep = {"PATH", "LANG", "LC_ALL"}
    environment = {key: value for key, value in os.environ.items() if key in keep}
    environment.update({"HOME": str(root / "home"), "GROK_HOME": str(root / "home/.grok"),
        "XDG_CONFIG_HOME": str(root / "home/.config"), "XDG_DATA_HOME": str(root / "home/.local/share"),
        "XDG_CACHE_HOME": str(root / "home/.cache"), "TMPDIR": str(root / "tmp"),
        "GROK_DISABLE_AUTOUPDATER": "1", "INFINISHELL_GROK_BYOK_KEY": fake_key,
        "PYTHONUTF8": "1"})
    return environment


def sandbox_profile(root, credential_directory, port, socket_path=None):
    literal = lambda value: json.dumps(str(value))
    # network-bind/inbound 的精确 Unix 路径形式沿用系统 com.apple.rpcbind.sb；canary 必须实际验证。
    profile = ('(version 1)(allow default)(deny network*)'
        f'(allow network-outbound (remote ip "localhost:{port}"))'
        f'(deny file-read* (subpath {literal(credential_directory)}))'
        f'(deny file-write*)(allow file-write* (subpath {literal(root)}) (literal "/dev/null"))')
    alternate = str(credential_directory).replace("/private/tmp/", "/tmp/")
    if alternate != str(credential_directory):
        profile += f'(deny file-read* (subpath {literal(alternate)}))'
    if socket_path:
        profile += (f'(allow network-bind network-inbound (literal {literal(socket_path)}))'
            f'(allow network-outbound (remote unix-socket (path-literal {literal(socket_path)})))')
    return profile


def network_canary(root, credential_path, port):
    endpoint = root / "tmp/canary.sock"
    blocked = root / "tmp/blocked.sock"
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(str(blocked)); listener.listen(1)
    profile = sandbox_profile(root, credential_path.parent, port, endpoint)
    source = '''import json,socket
r={}
for label,address in [('allowed_proxy',('127.0.0.1',PORT)),('other_loopback',('127.0.0.1',9)),('external',('1.1.1.1',443))]:
 s=socket.socket();s.settimeout(2)
 try:s.connect(address);r[label]=True
 except OSError as e:r[label]=e.errno
 finally:s.close()
a=socket.socket(socket.AF_UNIX);b=socket.socket(socket.AF_UNIX);b.settimeout(2)
try:
 a.bind(ENDPOINT);a.listen(1);b.connect(ENDPOINT);c,_=a.accept();c.close();r['exact_unix']=True
except OSError as e:r['exact_unix']=e.errno
finally:a.close();b.close()
s=socket.socket(socket.AF_UNIX);s.settimeout(2)
try:s.connect(BLOCKED);r['other_unix']=True
except OSError as e:r['other_unix']=e.errno
finally:s.close()
try:
 with open(CREDENTIAL,'rb') as f:f.read(1)
 r['credential_read']=True
except OSError as e:r['credential_read']=e.errno
print(json.dumps(r))
'''.replace("PORT", str(port)).replace("ENDPOINT", repr(str(endpoint))).replace("BLOCKED", repr(str(blocked))).replace("CREDENTIAL", repr(str(credential_path)))
    try:
        result = subprocess.run(["/usr/bin/sandbox-exec", "-p", profile, sys.executable, "-c", source],
            capture_output=True, text=True, timeout=15, env=native_environment(root))
        observed = json.loads(result.stdout) if result.stdout else None
        expected = {"allowed_proxy": True, "other_loopback": 1, "external": 1,
            "exact_unix": True, "other_unix": 1, "credential_read": 1}
        # bool 与 int 的 Python 相等规则不能把成功 True 当 errno=1。
        if result.returncode != 0 or json.dumps(observed, sort_keys=True) != json.dumps(expected, sort_keys=True):
            raise ValueError("OS 沙箱 canary 未证明精确网络与凭据边界；禁止模型请求")
        return observed
    finally:
        listener.close()
        endpoint.unlink(missing_ok=True); blocked.unlink(missing_ok=True)


def redacted_response_chunks(response, secrets):
    needles = [value.encode() for secret in secrets if secret for value in (secret, json.dumps(secret, ensure_ascii=False)[1:-1])]
    retain = max((len(value) for value in needles), default=1) - 1
    pending = b""
    while chunk := response.read1(65536):
        pending += chunk
        for value in needles:
            pending = pending.replace(value, b"<redacted>")
        if len(pending) > retain:
            boundary = len(pending) - retain
            yield pending[:boundary]
            pending = pending[boundary:]
    if pending:
        yield pending


class MessagesProxy:
    def __init__(self, credentials, target, maximum, fake_key=FAKE_KEY):
        self.credentials = credentials; self.target = target; self.maximum = maximum
        self.forwarded = 0; self.events = []; self.lock = threading.Lock()
        self.fake_key = fake_key; self.closing = False; self.connections = set()
        self.drained = threading.Condition(self.lock)
        owner = self
        class Server(http.server.ThreadingHTTPServer):
            daemon_threads = True
            def handle_error(self, request, address):
                owner.record({"event": "proxy_socket_error"})
        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"
            def log_message(self, *args):
                pass
            def reject(self, status, reason):
                owner.record({"event": "rejected", "reason": reason, "status": status})
                data = json.dumps({"type": "error", "error": {"type": "permission_error", "message": reason}}).encode()
                self.send_response(status); self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data))); self.send_header("Connection", "close")
                self.end_headers(); self.wfile.write(data); self.close_connection = True
            def do_GET(self):
                self.reject(405, "仅允许 Messages POST")
            def do_CONNECT(self):
                self.reject(405, "不提供通用代理")
            def do_POST(self):
                if self.path != "/v1/messages" or self.headers.get("Transfer-Encoding"):
                    return self.reject(403, "路径或传输方式不在范围内")
                if (self.headers.get("x-api-key") != owner.fake_key
                        and self.headers.get("Authorization") != "Bearer " + owner.fake_key):
                    return self.reject(403, "缺少本次私有代理凭据")
                try:
                    size = int(self.headers.get("Content-Length", "0"))
                    if not 0 < size < 8 * 1024 * 1024:
                        return self.reject(400, "请求长度超出范围")
                    raw = self.rfile.read(size); body = json.loads(raw)
                except (ValueError, UnicodeError):
                    return self.reject(400, "请求必须是定长 JSON")
                if not owner.allowed_body(body, raw):
                    return self.reject(403, "模型、固定提示或输出预算不在范围内")
                with owner.lock:
                    if owner.closing or owner.forwarded >= owner.maximum:
                        number = None
                    else:
                        owner.forwarded += 1; number = owner.forwarded
                        owner.events.append({"event": "forwarding", "number": number, "model": MODEL,
                            "max_tokens": body["max_tokens"], "message_count": len(body.get("messages", [])),
                            "request_bytes": len(raw), "destination": "<用户指定 HTTPS origin>/v1/messages"})
                if number is None:
                    return self.reject(403, "本次总请求预算已耗尽")
                key = owner.credentials.get("ANTHROPIC_API_KEY") or owner.credentials["ANTHROPIC_AUTH_TOKEN"]
                connection = http.client.HTTPSConnection(owner.target.hostname, owner.target.port or 443,
                    timeout=60, context=ssl.create_default_context())
                with owner.lock:
                    allowed = not owner.closing
                    if allowed:
                        owner.connections.add(connection)
                if not allowed:
                    connection.close()
                    return self.reject(403, "代理正在关闭")
                try:
                    headers = {"Content-Type": "application/json", "Accept": "text/event-stream",
                        "Authorization": "Bearer " + key, "anthropic-version": "2023-06-01"}
                    if "ANTHROPIC_API_KEY" in owner.credentials:
                        headers["x-api-key"] = key
                    connection.request("POST", "/v1/messages", body=raw, headers=headers)
                    response = connection.getresponse()
                    owner.record({"event": "upstream_response", "number": number, "status": response.status,
                        "credential_destination": "<用户指定 HTTPS origin>", "redirects_followed": False})
                    if 300 <= response.status < 400:
                        return self.reject(502, "禁止重定向")
                    if response.status >= 400:
                        return self.reject(response.status, "上游服务拒绝请求，未记录服务错误正文")
                    self.send_response(response.status)
                    self.send_header("Content-Type", response.getheader("Content-Type", "application/json"))
                    self.send_header("Connection", "close"); self.end_headers()
                    # 连跨网络块的凭据回显也先脱敏，避免进入原生 stdout 或 Rust 证据。
                    sensitive = [key, owner.credentials.get("ANTHROPIC_BASE_URL", ""), owner.target.netloc, owner.target.hostname]
                    for chunk in redacted_response_chunks(response, sensitive):
                        self.wfile.write(chunk); self.wfile.flush()
                    self.close_connection = True
                except Exception as error:
                    owner.record({"event": "upstream_error", "type": type(error).__name__})
                    self.close_connection = True
                finally:
                    connection.close()
                    with owner.drained:
                        owner.connections.discard(connection)
                        owner.drained.notify_all()
        self.server = Server(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @staticmethod
    def allowed_body(body, raw):
        return (isinstance(body, dict) and body.get("model") == MODEL
            and type(body.get("max_tokens")) is int and 0 < body["max_tokens"] <= 2048
            and FIXED_PROMPT.encode() in raw)

    def record(self, event):
        with self.lock:
            self.events.append(event)

    def start(self):
        self.thread.start()
        return self.server.server_address[1]

    def close(self):
        with self.lock:
            self.closing = True
            active = list(self.connections)
        self.server.shutdown(); self.server.server_close(); self.thread.join(timeout=3)
        for connection in active:
            try:
                if connection.sock:
                    connection.sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            connection.close()
        with self.drained:
            drained = self.drained.wait_for(lambda: not self.connections, timeout=5)
        return not self.thread.is_alive() and drained


def prepare_native(root, real_cli, credential_directory, port, binary_sha256=None):
    binary_sha256 = BINARY_SHA256 if binary_sha256 is None else binary_sha256
    configuration = f'''[cli]
use_leader = true
auto_update = false
[models]
default = "parity-claude"
session_summary = "parity-claude"
web_search = "parity-claude"
image_description = "parity-claude"
[model.parity-claude]
model = "{MODEL}"
name = "Grok Build - custom Claude backend"
base_url = "http://127.0.0.1:{port}/v1"
env_key = "INFINISHELL_GROK_BYOK_KEY"
api_backend = "messages"
context_window = 200000
max_completion_tokens = 2048
max_retries = 0
supports_backend_search = false
supports_reasoning_effort = false
stream_tool_calls = false
extra_headers = {{ "anthropic-version" = "2023-06-01" }}
[features]
turn_summary = false
title_refresh = false
support_permission = true
[[permission.rules]]
action = "ask"
tool = "any"
'''
    settings = root / "home/.grok/config.toml"
    settings.write_text(configuration, encoding="utf-8")
    # 包装器不改 CLI 参数，只为生产传入的独立 socket 增加精确沙箱白名单。
    wrapper = root / "grok-sandbox"
    code = f'''#!{Path(sys.executable).resolve()}
import hashlib,json,os,sys
from pathlib import Path
root=Path({str(root)!r}); native=Path({str(real_cli)!r}); args=sys.argv[1:]
if hashlib.sha256(native.read_bytes()).hexdigest()!={binary_sha256!r}:raise SystemExit(91)
profile={sandbox_profile(root, credential_directory, port)!r}
if args==['--version']:kind='version';endpoint=None
elif len(args)==4 and args[:3]==['agent','stdio','--leader-socket']:
 endpoint=Path(args[3]);resolved=endpoint.resolve()
 if not endpoint.is_absolute() or endpoint!=resolved or not resolved.is_relative_to(root/'tmp') or resolved.name!='leader.sock' or resolved.exists():raise SystemExit(92)
 parent=resolved.parent
 if parent.stat().st_uid!=os.getuid() or parent.stat().st_mode & 0o077:raise SystemExit(93)
 quoted=json.dumps(str(resolved))
 profile+='(allow network-bind network-inbound (literal '+quoted+'))(allow network-outbound (remote unix-socket (path-literal '+quoted+')))'
 kind='private_leader'
else:raise SystemExit(94)
with (root/'wrapper-audit.ndjson').open('a') as evidence:
 evidence.write(json.dumps({{'event':'native_launch','kind':kind,'arguments_unchanged':True,'private_socket':str(endpoint.relative_to(root)) if endpoint else None}})+'\\n')
os.execv('/usr/bin/sandbox-exec',['sandbox-exec','-p',profile,str(native),*args])
'''
    wrapper.write_text(code, encoding="utf-8"); wrapper.chmod(0o700)
    return wrapper, settings


def verified_final_response(event, native_session_id, expected=None):
    # 回执只能来自同会话、同回合、生产确认完成水位之前的最后响应流。
    watermark = event.get("history_completion_watermark")
    prefix = native_session_id + "-"
    if (event.get("receipt_source") != RECEIPT_SOURCE
            or event.get("history_verified") is not True
            or event.get("product_full_output_preserved") is not True
            or event.get("history_native_session_id") != native_session_id
            or event.get("history_native_turn_id") != event.get("turn_id")
            or not isinstance(watermark, str) or not watermark.startswith(prefix)):
        return False
    sequence = watermark[len(prefix):]
    if (not re.fullmatch(r"(?:0|[1-9][0-9]{0,19})", sequence)
            or int(sequence) > (1 << 64) - 1):
        return False
    stream = event.get("final_response_stream_start_ms")
    if ((event.get("outcome") == "Completed" and type(stream) is not int)
            or (stream is not None and (type(stream) is not int or not -(1 << 63) <= stream < (1 << 63)))):
        return False
    reply = event.get("final_response")
    full_bytes = event.get("full_output_bytes")
    full_hash = event.get("full_output_sha256")
    if (not isinstance(reply, str) or not isinstance(full_hash, str)
            or (event.get("outcome") == "Completed" and not reply)):
        return False
    encoded = reply.encode("utf-8")
    if (type(event.get("final_response_bytes")) is not int
            or event["final_response_bytes"] != len(encoded)
            or event.get("final_response_sha256") != hashlib.sha256(encoded).hexdigest()
            or type(full_bytes) is not int or full_bytes < len(encoded)
            or not re.fullmatch(r"[0-9a-f]{64}", full_hash)):
        return False
    archived = event.get("output")
    if not isinstance(archived, str) or type(event.get("output_truncated")) is not bool:
        return False
    if not event["output_truncated"]:
        if len(archived.encode("utf-8")) != full_bytes or hashlib.sha256(archived.encode("utf-8")).hexdigest() != full_hash:
            return False
    elif len(archived) != 4096 or len(archived.encode("utf-8")) >= full_bytes:
        return False
    return expected is None or reply.strip() == expected


def verified_acceptance(exit_code, output, events, *, official=False):
    if exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return False
    endings = [event for event in events if event.get("event") == "acceptance_passed"]
    if len(endings) != 1 or any(event.get("event") == "acceptance_failed" for event in events):
        return False
    ending = endings[0]; native = ending.get("native_session_id")
    if (not isinstance(native, str) or not native or ending.get("scope") != SCOPE
            or ending.get("official_grok_model_tested") is not official
            or ending.get("queued_input_verified") is not True
            or any(ending.get(key) is not False for key in ("same_turn_steering_supported",
                "public_product_gate_open", "app_restart_and_ui_verified", "parent_permission_ceiling_verified"))):
        return False
    results = [event for event in events if event.get("event") == "turn_finished"]
    if len(results) != 8 or len({event.get("turn_id") for event in results}) != 8:
        return False
    if any(event.get("native_session_id") != native for event in results):
        return False
    if any(not verified_final_response(event, native) for event in results):
        return False
    for phase, outcome, reply in (("first_turn", "Completed", "PARITY_ONE"),
            ("second_turn", "Completed", "PARITY_TWO"), ("approval_allow", "Completed", "APPROVED"),
            ("approval_deny", "Cancelled", None), ("cancel", "Cancelled", None)):
        matching = [event for event in results if event.get("phase") == phase]
        if len(matching) != 1 or matching[0].get("outcome") != outcome:
            return False
        if reply is not None and not verified_final_response(matching[0], native, reply):
            return False
    for result in results:
        matching = [event for event in events if event.get("event") == "message_accepted"
            and event.get("turn_id") == result.get("turn_id") and event.get("native_receipt") is True]
        if len(matching) != 1 or not matching[0].get("message_id"):
            return False
        if sum(event.get("event") == "turn_started" and event.get("turn_id") == result.get("turn_id") for event in events) != 1:
            return False
    for phase, decision, allowed in (("approval_allow", "AllowOnce", True), ("approval_deny", "DenyOnce", False)):
        approvals = [event for event in events if event.get("event") == "approval_requested" and event.get("phase") == phase]
        if len(approvals) != 1 or approvals[0].get("decision") != decision or approvals[0].get("exact_write_fixture") is not True:
            return False
        if not any(event.get("event") == "file_effect_verified" and event.get("phase") == phase and event.get("allowed") is allowed for event in events):
            return False
    queued = [event for event in events if event.get("event") == "queued_input_result_verified"]
    resumed = [event for event in results if event.get("phase") == "resume_result"]
    queue_results = [event for event in results if event.get("phase") == "queued_input"]
    if (len(queued) != 1 or queued[0].get("native_acknowledgement_verified") is not True
            or len(resumed) != 1 or resumed[0].get("outcome") != "Completed"
            or not queued[0].get("marker") or not verified_final_response(resumed[0], native, queued[0]["marker"])
            or len(queue_results) != 2 or any(event.get("outcome") != "Completed" for event in queue_results)):
        return False
    if not any(event.get("event") == "cancel_submitted" and event.get("submitted_after_real_text") is True for event in events):
        return False
    if not any(event.get("event") == "queued_input_submitted" and event.get("submitted_after_real_text") is True for event in events):
        return False
    shutdowns = [event for event in events if event.get("event") == "connection_shutdown"]
    return (len(shutdowns) == 2 and [event.get("queued_submissions_observed_inside_adapter") for event in shutdowns] == [1, 0]
        and all(event.get("cleanup_confirmed") is True and event.get("native_session_id") == native for event in shutdowns))


def validate_paths(args):
    if sys.platform != "darwin":
        raise ValueError("此运行器只验证 macOS 的精确 OS 沙箱，其他平台尚需独立运行器")
    for name in ("test_binary", "grok", "supervisor", "api_environment_file"):
        path = getattr(args, name)
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"{name} 必须是现有非符号链接文件")
        setattr(args, name, path.resolve(strict=True))
    if args.test_binary == args.supervisor or digest(args.grok) != BINARY_SHA256:
        raise ValueError("必须提供同提交监督入口与固定已验证 Grok 二进制")
    if not 1 <= args.max_messages <= 16:
        raise ValueError("总 Messages 预算必须在 1–16 之间")
    if args.output.is_symlink():
        raise ValueError("证据路径不能是符号链接")
    args.output = args.output.resolve()
    if args.output.suffix != ".ndjson":
        raise ValueError("证据路径必须使用 .ndjson 扩展名")
    for path in (args.output, args.output.with_suffix(".metadata.json"), args.output.with_suffix(".test-output.txt"), args.output.with_suffix(".proxy.json")):
        if (path.exists() or path.is_symlink() or path.is_relative_to(args.api_environment_file.parent)
                or path in (args.test_binary, args.supervisor, args.grok, args.api_environment_file)):
            raise ValueError("不得覆盖既有证据或输入文件")


def run(args):
    # 先独占所有产物路径，避免并发运行用失败元数据覆盖另一轮证据。
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for path in (args.output, args.output.with_suffix(".metadata.json"), args.output.with_suffix(".test-output.txt"), args.output.with_suffix(".proxy.json")):
        with path.open("x", encoding="utf-8"):
            pass
    credentials, target = load_credentials(args.api_environment_file)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-adapter-", dir="/private/tmp")).resolve()
    for relative in ("home/.grok", "project", "tmp", "state"):
        (root / relative).mkdir(parents=True, mode=0o700)
    (root / ".infinishell-grok-live-probe").write_text(MARKER, encoding="utf-8")
    fake_key = "infinishell-loopback-" + secrets.token_urlsafe(24)
    clean = sanitizer(credentials, root, fake_key)
    metadata = {"test": TEST_NAME, "scope": SCOPE, "platform": sys.platform,
        "public_product_gate_open": False, "test_only_internal_command_switch": False,
        "production_runtime_commands": True,
        "official_grok_model_tested": False, "model_path": "Grok Build + 自定义 Claude 后端",
        "max_messages_requests": args.max_messages, "max_tokens_per_request": 2048,
        "native_received_real_credential": False, "credential_values_recorded": False,
        "product_network_isolation_verified": False, "acceptance_passed": False,
        "private_workspace_preserved": True, "private_workspace": str(root),
        "test_binary_sha256": digest(args.test_binary), "supervisor_sha256": digest(args.supervisor),
        "grok_sha256": digest(args.grok), "native_launch_arguments_modified": False}
    proxy = MessagesProxy(credentials, target, args.max_messages, fake_key); port = proxy.start()
    output = ""; owned_output = True; events = []
    try:
        metadata["sandbox_canary"] = network_canary(root, args.api_environment_file, port)
        if proxy.forwarded != 0:
            raise ValueError("canary 阶段不得产生模型请求")
        wrapper, settings = prepare_native(root, args.grok, args.api_environment_file.parent, port)
        metadata["private_settings_sha256"] = digest(settings)
        environment = native_environment(root, fake_key)
        environment.update({"INFINISHELL_GROK_LIVE_ROOT": str(root),
            "INFINISHELL_GROK_LIVE_EXECUTABLE": str(wrapper), "INFINISHELL_GROK_LIVE_ARTIFACT": str(args.output),
            "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor)})
        metadata["native_environment_names"] = sorted(environment)
        version = subprocess.run([str(wrapper), "--version"], env=environment, cwd=root / "project",
            capture_output=True, text=True, timeout=10, check=True)
        if version.stdout.strip() != VERSION:
            raise ValueError("原生 Grok 版本与固定契约不匹配")
        metadata["grok_version"] = version.stdout.strip()
        repository = Path(__file__).resolve().parents[2]
        for command, key in ((["git", "rev-parse", "HEAD"], "repository_commit"), (["git", "status", "--porcelain"], "worktree_dirty")):
            result = subprocess.run(command, cwd=repository, capture_output=True, text=True, check=True)
            metadata[key] = bool(result.stdout.strip()) if key == "worktree_dirty" else result.stdout.strip()
        command = [str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
        process = subprocess.Popen(command, cwd=repository, env=environment,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
        try:
            output, _ = process.communicate(timeout=900)
        except subprocess.TimeoutExpired:
            metadata["timed_out"] = True; process.kill(); output, _ = process.communicate(timeout=20)
        except BaseException:
            process.kill(); process.wait(timeout=20); raise
        metadata["test_exit_code"] = process.returncode
        raw = clean(args.output.read_text(encoding="utf-8"))
        args.output.write_text(raw, encoding="utf-8")
        events = [json.loads(line) for line in raw.splitlines()]
        launches = [json.loads(line) for line in (root / "wrapper-audit.ndjson").read_text().splitlines()]
        leader_launches = [event for event in launches if event.get("kind") == "private_leader"]
        metadata["native_launches"] = launches
        metadata["private_settings_unchanged"] = digest(settings) == metadata["private_settings_sha256"]
        metadata["project_files"] = sorted(str(path.relative_to(root / "project")) for path in (root / "project").rglob("*") if path.is_file())
        metadata["acceptance_passed"] = (not metadata.get("timed_out", False)
            and metadata["private_settings_unchanged"] and len(leader_launches) == 2
            and len({event["private_socket"] for event in leader_launches}) == 2
            and metadata["project_files"] == ["approval-allow.txt"]
            and verified_acceptance(process.returncode, output, events))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        metadata["runner_error"] = clean(f"{type(error).__name__}: {error}")
    finally:
        metadata["proxy_stopped"] = proxy.close(); metadata["requests_forwarded"] = proxy.forwarded
        metadata["acceptance_passed"] &= metadata["proxy_stopped"]
        if owned_output and args.output.exists():
            args.output.write_text(clean(args.output.read_text(encoding="utf-8", errors="replace")), encoding="utf-8")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.with_suffix(".test-output.txt").write_text(clean(output), encoding="utf-8")
        args.output.with_suffix(".proxy.json").write_text(json.dumps(proxy.events, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
        args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    print("真实 Grok Rust 进程/传输验收" + ("通过" if metadata["acceptance_passed"] else "未通过"))
    print(f"证据：{args.output}")
    print(f"私有工作目录保留：{root}")
    return 0 if metadata["acceptance_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--grok", type=Path, required=True, help="固定 macOS aarch64 Grok 1.0.30")
    parser.add_argument("--supervisor", type=Path, required=True, help="同提交主程序或 TUI 监督入口")
    parser.add_argument("--api-environment-file", type=Path, required=True, help="仅转发器读取的显式用户服务 JSON")
    parser.add_argument("--max-messages", type=int, default=16)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        validate_paths(args)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"运行器启动失败：{type(error).__name__}；请检查显式输入与隔离条件。\n")


if __name__ == "__main__":
    raise SystemExit(main())
