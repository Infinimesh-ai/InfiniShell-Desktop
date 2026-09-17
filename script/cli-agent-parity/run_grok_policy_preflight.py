#!/usr/bin/env python3
"""准备 Grok 固定策略的无模型接口调查；Rust 入口未接线时前置拒绝。"""

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import sys
import tempfile
import threading
import time
import uuid


SCOPE = "grok_fixed_policy_interface_preflight"
TEST_NAME = "ai::cli_agent_runtime::grok::policy_preflight_live_tests::native_fixed_policy_interfaces"
# 根代理接线、编译并核对真实无模型写入守卫后才能修改；没有环境变量或参数旁路。
RUST_ENTRYPOINT_PREPARED = True
MAX_NATIVE_INPUTS = 0
MAX_REQUESTS = 12
MAX_PROCESSES = 2
MAX_DEADLINE = 360
MAX_EVIDENCE_BYTES = 4 * 1024 * 1024
MAX_STDOUT_BYTES = 4 * 1024 * 1024
MAX_TLS_BYTES = 8 * 1024 * 1024
MAX_TLS_CONNECTIONS = 16
VERSION = "grok 1.0.30 (04b7ffed98c6)"
BINARY_SHA256 = "d53b6e543e482716236748914331db50145c696ac7af91f1ebdedcf5654cfecb"
BINARY_BYTES = 141869568
SOURCE_SNAPSHOT = "482711333c7195dc16a272777f86086d615e2afb"
SOURCE_REVISION = "be7ce6e8cffe46d20bef9834b211616082ee866b"
DIAGNOSTIC_METHODS = (
    "x.ai/session/info", "x.ai/session/state", "x.ai/mcp/list", "x.ai/debug/agent",
)
ALLOWED_METHODS = ("initialize", "session/new", "session/load", *DIAGNOSTIC_METHODS)
RESPONSE_STATES = {"ok", "method_not_found", "auth_required", "rpc_error", "timeout"}
LIMITATIONS = (
    "candidate_source_differs_from_binary", "no_effective_builtin_catalog_interface_verified",
    "profile_loading_unknown", "effective_mode_unknown", "configuration_sources_unknown",
    "wrapper_target_closure_unknown", "parent_ceiling_unverified", "filesystem_sandbox_unverified",
    "opaque_tls_model_http_count_unknown", "cross_platform_unverified", "ssh_tmux_unverified",
)
_tunnel_lock = threading.Lock()


class PreflightRejected(ValueError):
    """只传播固定原因代码，避免异常带出配置、响应或路径。"""


def require(condition, code):
    if not condition:
        raise PreflightRejected(code)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


def digest(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def valid_uuid(value):
    try:
        return isinstance(value, str) and str(uuid.UUID(value)) == value
    except (ValueError, AttributeError):
        return False


def integer(value, maximum=MAX_EVIDENCE_BYTES):
    return type(value) is int and 0 <= value <= maximum


def make_plan(profile_sha256, config_sha256, request_budget=MAX_REQUESTS):
    require(digest(profile_sha256) and digest(config_sha256), "snapshot_invalid")
    require(type(request_budget) is int and 0 < request_budget <= MAX_REQUESTS, "request_budget_invalid")
    return {"schema_version": 1, "scope": SCOPE, "max_native_inputs": 0,
        "max_protocol_requests": request_budget, "max_session_processes": MAX_PROCESSES,
        "allowed_methods": list(ALLOWED_METHODS), "diagnostic_methods": list(DIAGNOSTIC_METHODS),
        "profile_sha256": profile_sha256, "config_sha256": config_sha256,
        "always_approve_requested": False, "auto_mode_requested": False,
        "protocol_guard_required_before_write": True, "reject_reverse_tool_requests": True,
        "model_http_count_measured": False, "candidate_source_is_exact_binary": False}


def unknown_policy():
    # 本端快照、原生响应及清理都不能填补尚不存在的有效策略证明。
    return {"profile_loading": "unknown", "effective_mode": "unknown",
        "configuration_sources": "unknown", "builtin_catalog": "unknown",
        "wrapper_closure": "unknown", "parent_permission_ceiling_verified": False,
        "filesystem_sandbox_verified": False, "ready_for_policy_implementation": False}


def observe_mode(value):
    return {"state": "unknown", "value_sha256": None if value is None else sha(canonical(value)),
        "reason": "no_verified_effective_mode_interface"}


def catalog_observation(requested_names, observed_names):
    require(isinstance(requested_names, list) and isinstance(observed_names, list)
        and len(requested_names) <= 64 and len(observed_names) <= 256
        and all(isinstance(name, str) and 0 < len(name.encode()) <= 256
            for name in requested_names + observed_names), "catalog_invalid")
    missing = set(requested_names) - set(observed_names)
    extra = set(observed_names) - set(requested_names)
    reason = ("unknown_name_or_native_fallback" if missing else
        "unexpected_tools" if extra else "names_only_not_effective_catalog")
    return {"state": "unknown", "reason": reason,
        "requested_names_sha256": sha(canonical(sorted(requested_names))),
        "observed_names_sha256": sha(canonical(sorted(observed_names))),
        "missing_name_count": len(missing), "extra_tool_count": len(extra),
        "fixed_tool_closure_verified": False}


def source_observation(expected_hashes, observed_hashes):
    require(isinstance(expected_hashes, list) and isinstance(observed_hashes, list)
        and len(expected_hashes) <= 64 and len(observed_hashes) <= 64
        and all(digest(value) for value in expected_hashes + observed_hashes), "sources_invalid")
    return {"state": "unknown", "extra_source_count": len(set(observed_hashes) - set(expected_hashes)),
        "missing_source_count": len(set(expected_hashes) - set(observed_hashes)),
        "source_sets_match": sorted(expected_hashes) == sorted(observed_hashes),
        "all_configuration_sources_verified": False}


def resume_profile_matches(original, current):
    require(digest(original) and digest(current), "snapshot_invalid")
    return original == current


def stdout_summary(raw):
    require(isinstance(raw, bytes) and len(raw) <= MAX_STDOUT_BYTES, "stdout_budget_exceeded")
    # 沿用图片运行器的六类定向模式；仅导出计数和完整散列，不保存正文或匹配文本。
    patterns = {
        "api_key": rb"(?<![A-Za-z0-9_-])(?:sk-[A-Za-z0-9_-]{16,}|xai-[A-Za-z0-9_-]{16,})",
        "jwt": rb"eyJ[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,}",
        "bearer": rb"(?i)Bearer\s+[A-Za-z0-9._~+/=-]{16,}",
        "private_key": rb"-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----",
        "credential_assignment": rb"(?i)(?:api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|password)\s*[=:]\s*[\"']?[A-Za-z0-9._~+/=-]{16,}",
        "email": rb"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
    }
    return {"raw_stdout_archived": False, "stdout_bytes": len(raw), "stdout_sha256": sha(raw),
        "credential_shape_counts": {name: len(re.findall(pattern, raw)) for name, pattern in patterns.items()},
        "test_success_summary_count": len(re.findall(rb"test result: ok\. 1 passed; 0 failed; 0 ignored;", raw))}


def event_schemas():
    flag = lambda value: type(value) is bool
    count = integer
    zero = lambda value: type(value) is int and value == 0
    nullable_digest = lambda value: value is None or digest(value)
    choice = lambda *values: lambda value: isinstance(value, str) and value in values
    return {
        "probe_started": {"scope": choice(SCOPE), "max_native_inputs": zero,
            "request_budget": lambda value: type(value) is int and 0 < value <= MAX_REQUESTS,
            "allowed_methods_sha256": digest, "guard_before_native_write": flag,
            "production_supervision": flag, "candidate_source_is_exact_binary": flag},
        "launch_snapshot": {"generation": valid_uuid, "phase": choice("new", "resume"),
            "cli_version": choice(VERSION), "cli_sha256": digest, "profile_sha256": digest,
            "config_sha256": digest, "always_approve_requested": flag, "auto_mode_requested": flag},
        "rpc_sent": {"generation": valid_uuid, "sequence": count,
            "method": choice(*ALLOWED_METHODS), "rpc_id": count, "request_bytes": count,
            "request_sha256": digest, "guard_checked_before_write": flag},
        "rpc_response": {"generation": valid_uuid, "sequence": count, "rpc_id": count,
            "status": choice(*RESPONSE_STATES), "response_bytes": count, "response_sha256": nullable_digest},
        "diagnostic_observed": {"generation": valid_uuid, "method": choice(*DIAGNOSTIC_METHODS),
            "status": choice(*RESPONSE_STATES), "top_level_key_sha256s": lambda value:
                isinstance(value, list) and len(value) <= 128 and all(digest(item) for item in value)},
        "mode_observation": {"generation": valid_uuid, "state": choice("unknown"),
            "value_sha256": nullable_digest},
        "catalog_observation": {"generation": valid_uuid, "state": choice("unknown"),
            "coverage": choice("mcp_only", "unavailable"), "tool_count": count,
            "catalog_sha256": nullable_digest, "fixed_tool_closure_verified": flag},
        "source_observation": {"generation": valid_uuid, "state": choice("unknown"),
            "extra_source_count": count, "all_configuration_sources_verified": flag},
        "resume_checked": {"generation": valid_uuid, "native_session_id_sha256": digest,
            "original_profile_sha256": digest, "current_profile_sha256": digest,
            "same_profile": flag, "inputs_replayed": zero},
        "process_cleanup": {"generation": valid_uuid, "exit_code": lambda value: type(value) is int and -255 <= value <= 255,
            "exit_reason": choice("stdio_closed", "native_exit", "stop_requested", "host_disconnected"),
            "cleanup_confirmed": flag, "receipt_sha256": digest},
        "probe_failed": {"reason_sha256": digest, "reason_bytes": count},
        "probe_finished": {"scope": choice(SCOPE), "native_inputs": zero, "tool_exec_count": zero,
            "unexpected_native_activity": count, "protocol_request_count": count,
            "guard_before_native_write": flag, "transport_closed": flag,
            "parent_permission_ceiling_verified": flag, "filesystem_sandbox_verified": flag,
            "profile_loading": choice("unknown"), "effective_mode": choice("unknown"),
            "configuration_sources": choice("unknown"), "builtin_catalog": choice("unknown"),
            "wrapper_closure": choice("unknown")},
    }


def project_events(events):
    require(isinstance(events, list) and len(events) <= 256, "event_budget_exceeded")
    schemas = event_schemas()
    projected = []
    failures = []
    for item in events:
        try:
            require(isinstance(item, dict) and isinstance(item.get("event"), str)
                and item["event"] in schemas, "event_schema_invalid")
            schema = schemas[item["event"]]
            require(set(item) == {"event", *schema}, "event_schema_invalid")
            require(all(check(item[name]) for name, check in schema.items()), "event_value_invalid")
            # 深拷贝只通过封闭模式的内容；未知字段的值不进入任何公开产物。
            projected.append(json.loads(canonical(item)))
        except PreflightRejected:
            failures.append({"event": "projection_rejected", "record_sha256": sha(canonical(item)),
                "record_bytes": len(canonical(item))})
    return projected, failures


def audit_events(exit_code, stdout, events):
    public, failures = project_events(events)
    result = {"execution_boundary_passed": False, "interface_investigation_completed": False,
        "policy": unknown_policy(), "events": public + failures, "stdout": stdout_summary(stdout)}
    try:
        require(not failures and exit_code == 0 and result["stdout"]["test_success_summary_count"] == 1
            and not any(result["stdout"]["credential_shape_counts"].values()), "test_or_projection_failed")
        one = lambda name: [row for row in public if row["event"] == name]
        require(len(one("probe_started")) == 1 and len(one("probe_finished")) == 1
            and not one("probe_failed"), "probe_incomplete")
        start, finish = one("probe_started")[0], one("probe_finished")[0]
        require(start["allowed_methods_sha256"] == sha(canonical(list(ALLOWED_METHODS)))
            and start["guard_before_native_write"] and start["production_supervision"]
            and start["candidate_source_is_exact_binary"] is False, "guard_unproved")
        launches = one("launch_snapshot")
        require(len(launches) == MAX_PROCESSES and [row["phase"] for row in launches] == ["new", "resume"], "launch_count_invalid")
        generations = [row["generation"] for row in launches]
        require(len(set(generations)) == MAX_PROCESSES, "generation_reused")
        require(public[0] is start and public[-1] is finish
            and all(row.get("generation") in generations for row in public if "generation" in row),
            "event_generation_invalid")
        first, second = launches
        require(all(row["cli_sha256"] == BINARY_SHA256 and row["always_approve_requested"] is False
            and row["auto_mode_requested"] is False for row in launches), "requested_mode_invalid")
        require(first["profile_sha256"] == second["profile_sha256"]
            and first["config_sha256"] == second["config_sha256"], "resume_snapshot_changed")
        sent, responses = one("rpc_sent"), one("rpc_response")
        require(0 < len(sent) <= start["request_budget"] and [row["sequence"] for row in sent] == list(range(1, len(sent) + 1)), "request_ledger_invalid")
        identities = [(row["generation"], row["rpc_id"]) for row in sent]
        require(len(set(identities)) == len(identities) and all(row["generation"] in generations
            and row["request_bytes"] > 0 and row["guard_checked_before_write"] for row in sent), "request_identity_invalid")
        require(len(responses) == len(sent), "response_missing")
        for request, response in zip(sent, responses):
            require((response["generation"], response["rpc_id"], response["sequence"])
                == (request["generation"], request["rpc_id"], request["sequence"]), "response_uncorrelated")
            require(response["status"] == "ok" and response["response_bytes"] > 0
                and digest(response["response_sha256"]), "interface_unavailable")
            require(public.index(request) < public.index(response), "response_before_request")
        for generation, opening in zip(generations, ("session/new", "session/load")):
            methods = [row["method"] for row in sent if row["generation"] == generation]
            require(methods == ["initialize", opening, *DIAGNOSTIC_METHODS], "method_sequence_invalid")
            diagnostics = [row for row in one("diagnostic_observed") if row["generation"] == generation]
            require([row["method"] for row in diagnostics] == list(DIAGNOSTIC_METHODS)
                and all(row["status"] == "ok" for row in diagnostics), "diagnostic_missing")
            for name in ("mode_observation", "catalog_observation", "source_observation"):
                require(len([row for row in one(name) if row["generation"] == generation]) == 1, "unknown_boundary_missing")
        require(all(row["fixed_tool_closure_verified"] is False for row in one("catalog_observation")), "catalog_overclaimed")
        require(all(row["extra_source_count"] == 0 and row["all_configuration_sources_verified"] is False
            for row in one("source_observation")), "extra_source_observed")
        resumes = one("resume_checked")
        require(len(resumes) == 1 and resumes[0]["generation"] == generations[1]
            and resumes[0]["same_profile"] and resumes[0]["original_profile_sha256"] == first["profile_sha256"]
            and resumes[0]["current_profile_sha256"] == first["profile_sha256"], "resume_profile_changed")
        cleanups = one("process_cleanup")
        require(len(cleanups) == MAX_PROCESSES and {row["generation"] for row in cleanups} == set(generations)
            and all(row["exit_code"] == 0 and row["exit_reason"] == "stdio_closed"
                and row["cleanup_confirmed"] for row in cleanups), "cleanup_unconfirmed")
        for launch in launches:
            generation = launch["generation"]
            cleanup = next(row for row in cleanups if row["generation"] == generation)
            require(all(public.index(launch) < public.index(row) < public.index(cleanup)
                for row in public if row.get("generation") == generation
                and row["event"] not in ("launch_snapshot", "process_cleanup")), "event_outside_process")
        require(public.index(cleanups[0]) < public.index(second), "processes_overlap")
        require(finish["guard_before_native_write"] and finish["transport_closed"]
            and finish["protocol_request_count"] == len(sent) and finish["unexpected_native_activity"] == 0
            and finish["parent_permission_ceiling_verified"] is False
            and finish["filesystem_sandbox_verified"] is False, "finish_overclaimed")
        result["execution_boundary_passed"] = True
        result["interface_investigation_completed"] = True
    except PreflightRejected as error:
        result["failure_code"] = str(error)
    return result


@contextmanager
def bounded_tunnel(official, timeout, connection_budget, byte_budget):
    require(type(connection_budget) is int and 0 <= connection_budget <= MAX_TLS_CONNECTIONS
        and type(byte_budget) is int and 0 <= byte_budget <= MAX_TLS_BYTES, "network_budget_invalid")
    with _tunnel_lock:
        previous = official.MAX_TUNNELS, official.MAX_BYTES
        official.MAX_TUNNELS = connection_budget if byte_budget else 0
        official.MAX_BYTES = byte_budget
        tunnel = None
        try:
            tunnel = official.OfficialTunnel(timeout)
            tunnel.preflight_cleanup_confirmed = False
            yield tunnel
        finally:
            try:
                if tunnel is not None:
                    tunnel.preflight_cleanup_confirmed = tunnel.close() is True
            finally:
                official.MAX_TUNNELS, official.MAX_BYTES = previous


def capture_process(command, environment, directory, timeout, byte_budget):
    require(type(byte_budget) is int and 0 < byte_budget <= MAX_STDOUT_BYTES, "stdout_budget_invalid")
    require(0 < timeout <= MAX_DEADLINE, "deadline_invalid")
    process = subprocess.Popen(command, cwd=directory, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    chunks = queue.Queue(maxsize=4)
    stopping = threading.Event()

    def reader():
        try:
            while not stopping.is_set():
                data = process.stdout.read1(65536)
                while not stopping.is_set():
                    try:
                        chunks.put(data, timeout=0.1)
                        break
                    except queue.Full:
                        continue
                if not data:
                    break
        finally:
            process.stdout.close()

    thread = threading.Thread(target=reader, daemon=True)
    thread.start()
    deadline, output = time.monotonic() + timeout, bytearray()
    try:
        while True:
            require(time.monotonic() < deadline, "process_deadline_exceeded")
            try:
                data = chunks.get(timeout=min(0.1, max(0.001, deadline - time.monotonic())))
            except queue.Empty:
                require(thread.is_alive(), "stdout_reader_failed")
                continue
            if not data:
                break
            require(len(output) + len(data) <= byte_budget, "stdout_budget_exceeded")
            output.extend(data)
        process.wait(timeout=max(0.001, deadline - time.monotonic()))
        return process.returncode, bytes(output)
    finally:
        stopping.set()
        if process.poll() is None:
            process.kill()
            process.wait(timeout=15)
        thread.join(timeout=2)


def rewrite_wrapper(code, profile_path, profile_sha256):
    require(digest(profile_sha256), "snapshot_invalid")
    old = """elif len(args)==4 and args[:3]==['agent','stdio','--leader-socket']:
 endpoint=Path(args[3]);resolved=endpoint.resolve()
 if not endpoint.is_absolute() or endpoint!=resolved or not resolved.is_relative_to(root/'tmp') or resolved.name!='leader.sock' or resolved.exists():raise SystemExit(92)
 parent=resolved.parent
 if parent.stat().st_uid!=os.getuid() or parent.stat().st_mode & 0o077:raise SystemExit(93)
 quoted=json.dumps(str(resolved))
 profile+='(allow network-bind network-inbound (literal '+quoted+'))(allow network-outbound (remote unix-socket (path-literal '+quoted+')))'
 kind='private_leader'
"""
    require(code.count(old) == 1 and code.count("if args==['--version']:") == 1, "wrapper_source_changed")
    replacement = (f"elif args==['agent','--no-leader','--agent-profile',{str(profile_path)!r},'stdio']:\n"
        f" if hashlib.sha256(Path({str(profile_path)!r}).read_bytes()).hexdigest()!={profile_sha256!r}:raise SystemExit(95)\n"
        " kind='policy_preflight';endpoint=None\n")
    updated = code.replace(old, replacement, 1)
    compile(updated, "grok-policy-preflight-wrapper", "exec")
    return updated


def ensure_entrypoint():
    require(RUST_ENTRYPOINT_PREPARED, "rust_entrypoint_not_prepared")


def validate_paths(args):
    # 必须先拒绝未接线入口，然后才可读取显式输入、保留产物或复制 opaque auth。
    ensure_entrypoint()
    require(sys.platform == "darwin", "platform_not_prepared")
    import prepare_grok_cli as fixed
    for name in ("test_binary", "supervisor", "grok", "profile", "config"):
        path = getattr(args, name)
        require(path is not None and path.is_absolute(), "input_path_invalid")
        fixed.regular_file(path)
        setattr(args, name, path.resolve(strict=True))
    require(args.test_binary != args.supervisor and args.grok.stat().st_size == BINARY_BYTES
        and fixed.digest(args.grok) == BINARY_SHA256, "fixed_cli_invalid")
    require(args.max_native_inputs == 0 and 30 <= args.timeout <= MAX_DEADLINE, "input_budget_invalid")
    require(0 <= args.max_tls_connections <= MAX_TLS_CONNECTIONS and 0 <= args.max_tls_bytes <= MAX_TLS_BYTES, "network_budget_invalid")
    require(args.output is not None and args.output.is_absolute() and args.output.suffix == ".ndjson", "output_path_invalid")
    for path in (args.output, args.output.with_suffix(".metadata.json"), args.output.with_suffix(".network.json")):
        require(not path.exists() and not path.is_symlink(), "output_already_exists")
    home = args.official_grok_home
    if home is not None:
        require(home.is_absolute() and not home.is_symlink() and home.is_dir()
            and home.stat().st_uid == os.getuid() and home.stat().st_mode & 0o077 == 0, "auth_directory_invalid")
        require(not args.output.resolve().is_relative_to(home.resolve()), "output_in_auth_directory")


def run(args):
    validate_paths(args)
    import run_grok_official_adapter_live as official
    import prepare_grok_cli as fixed
    profile, config = args.profile.read_bytes(), args.config.read_bytes()
    require(0 < len(profile) <= 65536 and 0 < len(config) <= 65536, "snapshot_budget_invalid")
    profile.decode("utf-8")
    config.decode("utf-8")
    plan = make_plan(sha(profile), sha(config))
    # 先核对 libtest 确实存在唯一精确 ignored 入口；列表操作不启动原生 Grok。
    listing_env = {"PATH": "/usr/bin:/bin", "HOME": "/nonexistent/infinishell-grok-policy-list",
        "GROK_HOME": "/nonexistent/infinishell-grok-policy-list/grok", "CODEX_HOME": "/nonexistent/infinishell-grok-policy-list/codex",
        "CLAUDE_CONFIG_DIR": "/nonexistent/infinishell-grok-policy-list/claude"}
    code, listing = capture_process([str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--list"],
        listing_env, Path(__file__).resolve().parents[2], 30, 65536)
    require(code == 0 and listing.splitlines().count((TEST_NAME + ": test").encode()) == 1, "rust_test_not_listed")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for path in official.artifacts(args.output):
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.close(descriptor)
    metadata = {"scope": SCOPE, "test_name": TEST_NAME, "prepared_only": False,
        "native_inputs": 0, "policy": unknown_policy(), "limitations": list(LIMITATIONS),
        "execution_boundary_passed": False, "interface_investigation_completed": False,
        "same_commit_verified_by_runner": False, "cross_platform_verified": False,
        "model_http_count_measured": False, "tls_decrypted": False,
        "auth_copy_method": "none" if args.official_grok_home is None else "opaque_auth_json_only",
        "credential_values_parsed_by_runner": False, "candidate_source_snapshot": SOURCE_SNAPSHOT,
        "candidate_source_revision": SOURCE_REVISION, "candidate_source_is_exact_binary": False,
        "input_hashes": {name: fixed.digest(getattr(args, name)) for name in ("test_binary", "supervisor", "grok")}}
    public, network, raw_stdout = [], {}, b""
    root = None
    metadata.update({"opaque_auth_copy_removed": False, "tunnels_stopped": False,
        "private_workspace_removed": False, "runner_cleanup_confirmed": False})
    try:
        with tempfile.TemporaryDirectory(prefix="infinishell-grok-policy-preflight-", dir="/private/tmp") as temporary:
            root = Path(temporary).resolve()
            for name in ("home/.grok", "home/.claude", "home/.codex", "project", "tmp", "state", "policy"):
                (root / name).mkdir(parents=True, mode=0o700, exist_ok=True)
            profile_path = root / "policy/profile.md"
            profile_path.write_bytes(profile)
            profile_path.chmod(0o400)
            raw_evidence = root / "evidence.ndjson"
            raw_evidence.touch(mode=0o600)
            auth_copy = root / "home/.grok/auth.json"
            with bounded_tunnel(official, args.timeout, args.max_tls_connections, args.max_tls_bytes) as tunnel:
                port = tunnel.start()
                source_home = args.official_grok_home or root / "no-auth-source"
                try:
                    if args.official_grok_home is not None:
                        official.copy_private_auth(source_home, root / "home/.grok")
                    wrapper, settings = official.prepare_native(root, args.grok, source_home, port)
                    settings.write_bytes(config)
                    settings.chmod(0o400)
                    wrapper.write_text(rewrite_wrapper(wrapper.read_text(), profile_path, sha(profile)), encoding="utf-8")
                    wrapper.chmod(0o700)
                    plan_path = root / "policy/plan.json"
                    plan_path.write_bytes(canonical(plan))
                    plan_path.chmod(0o400)
                    environment = official.official_environment(root, port)
                    environment.update({"CODEX_HOME": str(root / "home/.codex"), "CLAUDE_CONFIG_DIR": str(root / "home/.claude"),
                        "INFINISHELL_GROK_POLICY_PREFLIGHT_ROOT": str(root),
                        "INFINISHELL_GROK_POLICY_PREFLIGHT_PLAN": str(plan_path),
                        "INFINISHELL_GROK_POLICY_PREFLIGHT_EXECUTABLE": str(wrapper),
                        "INFINISHELL_GROK_POLICY_PREFLIGHT_ARTIFACT": str(raw_evidence),
                        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor)})
                    command = [str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
                    exit_code, raw_stdout = capture_process(command, environment, root / "project", args.timeout, MAX_STDOUT_BYTES)
                    require(raw_evidence.stat().st_size <= MAX_EVIDENCE_BYTES, "evidence_budget_exceeded")
                    events = [json.loads(line) for line in raw_evidence.read_bytes().splitlines() if line.strip()]
                    audited = audit_events(exit_code, raw_stdout, events)
                    public = audited.pop("events")
                    metadata.update(audited)
                    snapshots = [row for row in public if row["event"] == "launch_snapshot"]
                    metadata["snapshots_match_explicit_inputs"] = len(snapshots) == MAX_PROCESSES and all(
                        row["profile_sha256"] == sha(profile) and row["config_sha256"] == sha(config) for row in snapshots)
                    metadata["test_exit_code"] = exit_code
                    metadata["private_config_bytes_unchanged"] = settings.read_bytes() == config
                    metadata["private_profile_bytes_unchanged"] = profile_path.read_bytes() == profile
                    metadata["execution_boundary_passed"] &= (metadata["snapshots_match_explicit_inputs"]
                        and metadata["private_config_bytes_unchanged"] and metadata["private_profile_bytes_unchanged"])
                except (OSError, ValueError, UnicodeError, subprocess.SubprocessError) as error:
                    metadata["runner_error_type"] = type(error).__name__
                    metadata["execution_boundary_passed"] = False
                    # 尽量保存已经完成的严格投影；失败证据不得由成功收尾覆盖。
                    try:
                        require(raw_evidence.stat().st_size <= MAX_EVIDENCE_BYTES, "evidence_budget_exceeded")
                        partial = [json.loads(line) for line in raw_evidence.read_bytes().splitlines() if line.strip()]
                        rows, faults = project_events(partial)
                        public = rows + faults
                    except (OSError, ValueError):
                        metadata["partial_projection_unavailable"] = True
                finally:
                    try:
                        auth_copy.unlink(missing_ok=True)
                        metadata["opaque_auth_copy_removed"] = not auth_copy.exists()
                    except OSError:
                        metadata["opaque_auth_copy_removed"] = False
                    metadata["stdout"] = stdout_summary(raw_stdout)
                    network = {"max_tls_connections": args.max_tls_connections, "max_tls_bytes": args.max_tls_bytes,
                        "connections_attempted": tunnel.forwarded, "tls_bytes": tunnel.bytes,
                        "model_http_count_measured": False, "tls_decrypted": False}
            metadata["tunnels_stopped"] = (tunnel.preflight_cleanup_confirmed is True
                and tunnel.closing and not tunnel.thread.is_alive())
            metadata["execution_boundary_passed"] &= metadata["opaque_auth_copy_removed"] and metadata["tunnels_stopped"]
    except OSError as error:
        # 外层资源准备或收尾失败仍保存已有严格投影；异常正文和路径不得归档。
        metadata["runner_error_type"] = type(error).__name__
        metadata["outer_resource_error_type"] = type(error).__name__
        metadata["execution_boundary_passed"] = False
        reason = b"outer_resource_failed"
        public.append({"event": "probe_failed", "reason_sha256": sha(reason), "reason_bytes": len(reason)})
    metadata["private_workspace_removed"] = root is not None and not root.exists()
    metadata["runner_cleanup_confirmed"] = (metadata["opaque_auth_copy_removed"]
        and metadata["tunnels_stopped"] and metadata["private_workspace_removed"]
        and "outer_resource_error_type" not in metadata)
    metadata["execution_boundary_passed"] &= metadata["runner_cleanup_confirmed"]
    metadata["interface_investigation_completed"] &= metadata["execution_boundary_passed"]
    metadata["policy"] = unknown_policy()
    args.output.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in public), encoding="utf-8")
    args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    args.output.with_suffix(".network.json").write_text(json.dumps(network, indent=2) + "\n", encoding="utf-8")
    return 0 if metadata["execution_boundary_passed"] else 1


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("test-binary", "supervisor", "grok", "profile", "config", "official-grok-home", "output"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--max-native-inputs", type=int, default=0)
    parser.add_argument("--timeout", type=int, default=MAX_DEADLINE)
    parser.add_argument("--max-tls-connections", type=int, default=MAX_TLS_CONNECTIONS)
    parser.add_argument("--max-tls-bytes", type=int, default=MAX_TLS_BYTES)
    args = parser.parse_args(argv)
    try:
        ensure_entrypoint()
        return run(args)
    except (OSError, ValueError, UnicodeError, subprocess.SubprocessError):
        # 不打印异常、输入路径或原生输出；未准备入口也不读取任何显式认证输入。
        sys.stderr.write("Grok 固定策略调查入口未准备或输入未通过前置检查；没有授予生产权限能力。\n")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
