#!/usr/bin/env python3
"""以专用 Grok 官方缓存登录验收生产监督链；不接受 API 凭据或自定义模型地址。"""

import argparse
import hashlib
import http.server
import ipaddress
import json
import os
from pathlib import Path
import re
import select
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
import tomllib

import run_grok_adapter_live as shared

MODEL = "grok-4.6-build"
MODEL_PATH = "Grok Build + 官方缓存登录"
# 固定官方源码的生产采样地址与 OAuth 发行方；不开放通配域名、任意 HTTPS 或资产下载。
OFFICIAL_HOSTS = frozenset({"cli-chat-proxy.grok.com", "auth.x.ai"})
ACP_INPUTS = 8
MAX_TUNNELS = 32
MAX_BYTES = 32 * 1024 * 1024
NATIVE_MARKETPLACE_INITIALIZATION = {
    "default_skills_installs_purged": True,
    "official_marketplace_auto_installed": True,
    "sources": [{"name": "xAI Official", "git": "https://github.com/xai-org/plugin-marketplace.git"}],
}


def rejected_origin_event(authority):
    # 只记录固定类别与完整 authority 的散列，不公开任意域名、用户信息或查询文本。
    host = authority[:-4] if authority.endswith(":443") else ""
    valid_host = bool(re.fullmatch(r"[a-z0-9-]+(?:\.[a-z0-9-]+)*", host))
    if host == "api.x.ai":
        kind = "xai_api"
    elif host in {"github.com", "api.github.com", "raw.githubusercontent.com"}:
        kind = "github_asset_or_marketplace"
    elif valid_host and host.endswith((".ingest.sentry.io", ".ingest.us.sentry.io", ".ingest.de.sentry.io")):
        kind = "sentry_ingest"
    else:
        kind = "unknown"
    return {"event": "rejected_nonofficial_origin", "origin_kind": kind,
        "authority_sha256": hashlib.sha256(authority.encode("utf-8", errors="replace")).hexdigest()}


def audit_private_settings(before, after):
    # 首次启动会写入固定官方市场元数据；权限及其余配置必须保持原始语义。
    audit = {"bytes_unchanged": before == after,
        "before_sha256": hashlib.sha256(before).hexdigest(),
        "after_sha256": hashlib.sha256(after).hexdigest(),
        "toml_parse_succeeded": False, "permission_section_unchanged": False,
        "native_marketplace_initialization_only": False, "settings_scope_verified": False}
    try:
        initial = tomllib.loads(before.decode("utf-8"))
        current = tomllib.loads(after.decode("utf-8"))
        # JSON 比较保留 bool/int 等类型区别；不允许以 True == 1 接受配置类型改变。
        canonical_initial = json.dumps(initial, sort_keys=True)
        canonical_current = json.dumps(current, sort_keys=True)
        unchanged = canonical_initial == canonical_current
        audit["toml_parse_succeeded"] = True
        audit["permission_section_unchanged"] = (json.dumps(initial.get("permission"), sort_keys=True)
            == json.dumps(current.get("permission"), sort_keys=True))
        keys = set(initial) | set(current)
        changed = [key for key in keys if json.dumps(initial.get(key), sort_keys=True)
            != json.dumps(current.get(key), sort_keys=True)]
        known = {"cli", "models", "features", "permission", "marketplace"}
        audit["changed_top_level_tables"] = sorted({key if key in known else "<unexpected>" for key in changed})
        marketplace = current.get("marketplace")
        fixed_initialization = ("marketplace" not in initial
            and json.dumps(marketplace, sort_keys=True) == json.dumps(NATIVE_MARKETPLACE_INITIALIZATION, sort_keys=True))
        without_marketplace = {key: value for key, value in current.items() if key != "marketplace"}
        initialization_only = fixed_initialization and json.dumps(without_marketplace, sort_keys=True) == canonical_initial
        audit["native_marketplace_initialization_only"] = initialization_only
        audit["settings_scope_verified"] = unchanged or initialization_only
    except (UnicodeError, ValueError, TypeError) as error:
        audit["parse_error_type"] = type(error).__name__
    return audit


def official_environment(root, port):
    environment = shared.native_environment(root)
    environment.pop("INFINISHELL_GROK_BYOK_KEY")
    environment.update({"HTTPS_PROXY": f"http://127.0.0.1:{port}", "NO_PROXY": "",
        "GROK_DISABLE_API_KEY_AUTH": "1", "GROK_CLAUDE_HOOKS_ENABLED": "0",
        "GROK_CLAUDE_MCPS_ENABLED": "0", "GROK_CODEX_HOOKS_ENABLED": "0",
        "GROK_CODEX_MCPS_ENABLED": "0", "INFINISHELL_GROK_LIVE_AUTH_MODE": "official-cached-token"})
    return environment


def copy_private_auth(source_home, target_home):
    # 不解析登录资料，仅把显式授权的单个原生文件复制到私有目录；不复制配置或插件。
    source = source_home / "auth.json"
    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        attributes = os.fstat(descriptor)
        if (not stat.S_ISREG(attributes.st_mode) or attributes.st_uid != os.getuid()
                or attributes.st_mode & 0o077 or attributes.st_nlink != 1
                or not 0 < attributes.st_size <= 1024 * 1024):
            raise ValueError("认证来源必须是当前用户独占的普通私有文件")
        destination = target_home / "auth.json"
        target = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(target, "wb") as output, os.fdopen(os.dup(descriptor), "rb") as input_file:
            shutil.copyfileobj(input_file, output)
        return destination
    finally:
        os.close(descriptor)


class OfficialTunnel:
    """只转发白名单官方 origin 的端到端 TLS；不解密、不记录令牌或 HTTP 内容。"""

    def __init__(self, timeout):
        self.deadline = time.monotonic() + timeout
        self.lock = threading.Condition()
        self.connections = set()
        self.forwarded = 0
        self.bytes = 0
        self.events = []
        self.closing = False
        owner = self

        class Server(http.server.ThreadingHTTPServer):
            daemon_threads = True

            def handle_error(self, request, address):
                owner.record({"event": "tunnel_socket_error"})

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_CONNECT(self):
                if self.path not in {host + ":443" for host in OFFICIAL_HOSTS}:
                    owner.record(rejected_origin_event(self.path))
                    self.send_error(403)
                    return
                host = self.path[:-4]
                with owner.lock:
                    if owner.closing or owner.forwarded >= MAX_TUNNELS or time.monotonic() >= owner.deadline:
                        owner.events.append({"event": "official_connect_budget_rejected", "host": host})
                        self.send_error(403)
                        return
                    owner.forwarded += 1
                    owner.events.append({"event": "official_connect_attempt", "host": host})
                    owner.connections.add(self.connection)
                upstream = None
                try:
                    upstream = owner.connect(host)
                    with owner.lock:
                        if owner.closing:
                            return
                        owner.connections.add(upstream)
                        owner.events.append({"event": "official_tunnel_opened", "host": host})
                    self.send_response(200, "Connection Established")
                    self.end_headers()
                    owner.relay(self.connection, upstream)
                except OSError:
                    owner.record({"event": "official_tunnel_io_failed", "host": host})
                finally:
                    if upstream:
                        upstream.close()
                    with owner.lock:
                        owner.connections.discard(self.connection)
                        owner.connections.discard(upstream)
                        owner.lock.notify_all()
                    self.close_connection = True

        self.server = Server(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def record(self, event):
        with self.lock:
            self.events.append(event)

    @staticmethod
    def connect(host):
        addresses = socket.getaddrinfo(host, 443, type=socket.SOCK_STREAM)
        for family, kind, protocol, _, address in addresses:
            if not ipaddress.ip_address(address[0]).is_global:
                raise OSError("官方域名解析到非公网地址")
            connection = socket.socket(family, kind, protocol)
            connection.settimeout(10)
            try:
                connection.connect(address)
                return connection
            except OSError:
                connection.close()
        raise OSError("官方地址不可连接")

    def relay(self, client, upstream):
        while time.monotonic() < self.deadline:
            with self.lock:
                if self.closing or self.bytes >= MAX_BYTES:
                    return
            ready, _, _ = select.select([client, upstream], [], [], 0.2)
            for source in ready:
                data = source.recv(65536)
                if not data:
                    return
                with self.lock:
                    if self.bytes + len(data) > MAX_BYTES:
                        self.events.append({"event": "tunnel_byte_budget_exhausted"})
                        return
                    self.bytes += len(data)
                destination = upstream if source is client else client
                destination.sendall(data)

    def start(self):
        self.thread.start()
        return self.server.server_port

    def close(self):
        with self.lock:
            self.closing = True
            connections = list(self.connections)
        for connection in connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            connection.close()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        with self.lock:
            drained = self.lock.wait_for(lambda: not self.connections, timeout=12)
        return drained and not self.thread.is_alive()


def prepare_native(root, native, source_home, port):
    wrapper, settings = shared.prepare_native(root, native, source_home, port)
    # 保留原生模型定义与官方认证解析，仅固定模型选择；不能把缓存令牌送往自定义后端。
    settings.write_text(f'''[cli]
use_leader = true
auto_update = false
[models]
default = "{MODEL}"
session_summary = "{MODEL}"
web_search = "{MODEL}"
image_description = "{MODEL}"
[features]
turn_summary = false
title_refresh = false
support_permission = true
[[permission.rules]]
action = "ask"
tool = "any"
''', encoding="utf-8")
    settings.chmod(0o600)
    return wrapper, settings


def public_events(events):
    # 官方令牌不被运行器解析，故公开证据采用字段值白名单，不能依赖已知密钥替换。
    fixed = {shared.SCOPE, MODEL_PATH, "Completed", "Cancelled", "Failed", "AllowOnce", "DenyOnce",
        shared.RECEIPT_SOURCE,"PARITY_ONE", "PARITY_TWO", "APPROVED", "READY", "QUEUE_PARENT_DONE"}
    identifiers = {"event", "phase", "exit_reason"}
    result = []
    for event in events:
        cleaned = {}
        for key, value in event.items():
            if key in {"reason", "details", "error", "stderr"}:
                cleaned[key] = "<仅保留在私有诊断目录>"
            elif value is None or isinstance(value, (bool, int, float)):
                cleaned[key] = value
            elif isinstance(value, str):
                safe = (value in fixed or re.fullmatch(r"(?:QUEUED_APPLIED_)?[0-9a-f-]{32,36}", value)
                    or (key in {"full_output_sha256","final_response_sha256"} and re.fullmatch(r"[0-9a-f]{64}",value))
                    or (key == "history_completion_watermark" and re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}-(?:0|[1-9][0-9]{0,19})",value))
                    or (key in identifiers and re.fullmatch(r"[a-z][a-z_]{0,80}", value)))
                cleaned[key] = value if safe else "<省略非固定文本>"
            else:
                cleaned[key] = "<省略复杂字段>"
        result.append(cleaned)
    return result


def validate_paths(args):
    if sys.platform != "darwin":
        raise ValueError("官方隔离运行器目前只验证 macOS")
    for name in ("test_binary", "grok", "supervisor"):
        path = getattr(args, name)
        if path.is_symlink() or not path.is_file():
            raise ValueError("可执行输入必须是现有非符号链接文件")
        setattr(args, name, path.resolve(strict=True))
    if args.test_binary == args.supervisor or shared.digest(args.grok) != shared.BINARY_SHA256:
        raise ValueError("需要同提交监督入口与固定 Grok 二进制")
    home = args.official_grok_home
    if home.is_symlink() or not home.is_dir():
        raise ValueError("需要显式已登录的专用 GROK_HOME")
    args.official_grok_home = home.resolve(strict=True)
    if home.stat().st_uid != os.getuid() or home.stat().st_mode & 0o077:
        raise ValueError("认证目录必须由当前用户独占")
    if args.max_acp_inputs != ACP_INPUTS or not 30 <= args.timeout <= 900:
        raise ValueError("固定生命周期需要 8 个 ACP 输入，期限必须在 30–900 秒")
    if args.output.is_symlink():
        raise ValueError("证据路径不能是符号链接")
    args.output = args.output.resolve()
    if args.output.suffix != ".ndjson":
        raise ValueError("证据需要 .ndjson 扩展名")
    for path in artifacts(args.output):
        if (path.exists() or path.is_symlink() or path.is_relative_to(args.official_grok_home)
                or path in (args.test_binary, args.grok, args.supervisor)):
            raise ValueError("不得覆盖既有文件或写入原始认证目录")


def artifacts(output):
    return [output, output.with_suffix(".metadata.json"), output.with_suffix(".network.json")]


def run(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for path in artifacts(args.output):
        descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        os.close(descriptor)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-official-adapter-", dir="/private/tmp")).resolve()
    for relative in ("home", "home/.grok", "project", "tmp", "state"):
        (root / relative).mkdir(parents=True, exist_ok=True, mode=0o700)
    (root / ".infinishell-grok-live-probe").write_text(shared.MARKER, encoding="utf-8")
    raw_path = root / "private-evidence.ndjson"
    raw_path.touch(mode=0o600)
    metadata = {"scope": shared.SCOPE, "model_path": MODEL_PATH, "requested_model": MODEL,
        "official_grok_model_tested": False, "acceptance_passed": False,
        "public_product_gate_open": False, "app_restart_and_ui_verified": False,
        "test_only_internal_command_switch": False, "production_runtime_commands": True,
        "native_received_real_credential": True, "public_credential_values_recorded": False,
        "auth_copy_method": "opaque_auth_json_only", "private_workspace": str(root),
        "max_acp_inputs": ACP_INPUTS, "acp_budget_source": "固定 libtest 的 8 个输入",
        "http_model_call_budget_enforced": False, "cost_budget_enforced": False,
        "deadline_seconds": args.timeout, "max_tls_connections": MAX_TUNNELS,
        "max_tls_bytes": MAX_BYTES, "allowed_https_hosts": sorted(OFFICIAL_HOSTS),
        "tls_decrypted": False, "product_network_isolation_verified": False,
        "grok_sha256": shared.digest(args.grok), "test_binary_sha256": shared.digest(args.test_binary),
        "supervisor_sha256": shared.digest(args.supervisor)}
    tunnel = OfficialTunnel(args.timeout)
    port = tunnel.start()
    events = []
    output = ""
    settings = None
    try:
        copy_private_auth(args.official_grok_home, root / "home/.grok")
        metadata["sandbox_canary"] = shared.network_canary(root, args.official_grok_home / "auth.json", port)
        wrapper, settings = prepare_native(root, args.grok, args.official_grok_home, port)
        settings_before = settings.read_bytes()
        environment = official_environment(root, port)
        environment.update({"INFINISHELL_GROK_LIVE_ROOT": str(root),
            "INFINISHELL_GROK_LIVE_EXECUTABLE": str(wrapper), "INFINISHELL_GROK_LIVE_ARTIFACT": str(raw_path),
            "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor)})
        metadata["native_environment_names"] = sorted(environment)
        version = subprocess.run([str(wrapper), "--version"], cwd=root / "project", env=environment,
            capture_output=True, text=True, timeout=10, check=True)
        if version.stdout.strip() != shared.VERSION:
            raise ValueError("固定原生版本不匹配")
        metadata["grok_version"] = shared.VERSION
        repository = Path(__file__).resolve().parents[2]
        metadata["repository_commit"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repository, text=True).strip()
        metadata["worktree_dirty"] = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=repository, text=True).strip())
        command = [str(args.test_binary), shared.TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
        process = subprocess.Popen(command, cwd=repository, env=environment, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
        try:
            output, _ = process.communicate(timeout=max(1, tunnel.deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            metadata["timed_out"] = True
            process.kill()
            output, _ = process.communicate(timeout=20)
        except BaseException:
            process.kill()
            process.wait(timeout=20)
            raise
        metadata["test_exit_code"] = process.returncode
        events = [json.loads(line) for line in raw_path.read_text(encoding="utf-8").splitlines()]
        launches = [json.loads(line) for line in (root / "wrapper-audit.ndjson").read_text().splitlines()]
        leaders = [item for item in launches if item.get("kind") == "private_leader"]
        metadata["native_launches"] = launches
        metadata["private_settings_audit"] = audit_private_settings(settings_before, settings.read_bytes())
        metadata["private_settings_unchanged"] = metadata["private_settings_audit"]["bytes_unchanged"]
        metadata["project_files"] = sorted(str(path.relative_to(root / "project")) for path in (root / "project").rglob("*") if path.is_file())
        model_tunnel = any(item == {"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"} for item in tunnel.events)
        passed = shared.verified_acceptance(process.returncode, output, events, official=True)
        metadata["acceptance_passed"] = (passed and model_tunnel and not metadata.get("timed_out", False)
            and metadata["private_settings_audit"]["settings_scope_verified"]
            and metadata["project_files"] == ["approval-allow.txt"]
            and len(leaders) == 2 and len({item["private_socket"] for item in leaders}) == 2)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        # 异常可能包含原生响应，公开报告仅记录类型，原生输出保存在 0700 工作目录。
        metadata["runner_error_type"] = type(error).__name__
    finally:
        metadata["tunnels_stopped"] = tunnel.close()
        metadata["acceptance_passed"] &= metadata["tunnels_stopped"]
        metadata["official_grok_model_tested"] = metadata["acceptance_passed"]
        auth = root / "home/.grok/auth.json"
        try:
            auth.unlink(missing_ok=True)
            metadata["private_auth_copy_removed"] = not auth.exists()
        except OSError as error:
            metadata["private_auth_copy_removed"] = False
            metadata["auth_cleanup_error_type"] = type(error).__name__
            metadata["acceptance_passed"] = False
            metadata["official_grok_model_tested"] = False
        diagnostic = root / "private-test-output.txt"
        descriptor = os.open(diagnostic, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as file:
            file.write(output)
        if not events:
            try:
                events = [json.loads(line) for line in raw_path.read_text(encoding="utf-8").splitlines()]
            except (OSError, ValueError):
                metadata["private_evidence_parse_failed"] = True
        args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in public_events(events)), encoding="utf-8")
        args.output.with_suffix(".network.json").write_text(json.dumps({"events": tunnel.events,
            "tls_connections_attempted": tunnel.forwarded, "tls_bytes": tunnel.bytes}, ensure_ascii=False, indent=2) + "\n")
        args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n")
    print("官方 Grok 生产监督链验收" + ("通过" if metadata["acceptance_passed"] else "未通过"))
    print(f"脱敏证据：{args.output}")
    print(f"私有诊断目录：{root}")
    return 0 if metadata["acceptance_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--grok", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--official-grok-home", type=Path, required=True)
    parser.add_argument("--max-acp-inputs", type=int, default=ACP_INPUTS)
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        validate_paths(args)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"运行器启动失败：{type(error).__name__}；请检查专用登录目录与固定输入。\n")


if __name__ == "__main__":
    raise SystemExit(main())
