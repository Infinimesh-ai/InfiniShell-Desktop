#!/usr/bin/env python3
"""在官方隔离环境观察一次原生 SDK 请求；不派发任务，不预设来源字段存在。"""

import argparse
from contextlib import contextmanager
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import threading
import time

import run_grok_official_adapter_live as official

shared = official.shared
SCOPE = "grok_native_sdk_origin_probe"
TEST_NAME = "ai::cli_agent_runtime::grok::sdk_origin_live_tests::native_sdk_registration_and_origin_probe"
MAX_NATIVE_INPUTS = 1
MAX_TLS_CONNECTIONS = 32
REQUESTED_MODEL_BUDGET = 2
MAX_DEADLINE = 450
MAX_EVIDENCE_BYTES = 16 * 1024 * 1024
MAX_PRIVATE_ERROR_BYTES = 64 * 1024
ORIGIN_STATES = {"unknown", "missing_native_origin", "candidate_mapping_only", "verified_native_fields"}
EVENTS = {"probe_started", "probe_initialize_observed", "registration_received", "sdk_request_received",
    "native_tool_update_received", "native_mcp_status_observed", "probe_approval_observed", "probe_finished",
    "outbound_transaction_observed"}
SDK_WIRE_METHODS = {"x.ai/mcp/sdk_call", "_x.ai/mcp/sdk_call"}
JSON_TYPES = {"absent", "null", "absent_or_null", "boolean", "number", "string", "array", "object"}
MCP_STATUS_METHODS = {"_x.ai/mcp/init_progress", "_x.ai/mcp_initialized", "_x.ai/mcp/server_status"}
MCP_STATUSES = {"ready", "initializing", "unavailable", "needsauth"}
CALIBRATED_TOOL_KINDS = {"inspect", "search_discovery", "other"}
MCP_VERSIONS = {"2025-11-25", "2026-07-28"}
INITIALIZATION_MODES = {"legacy_initialize", "modern_discover"}
VERSION_CARRIERS = {"params.protocolVersion", "params._meta"}
NATIVE_TOOL_KINDS = {"ToolSearch", "McpToolSearch", "SearchTools", "ToolLookup", "DiscoverTools",
    "ListTools", "Read", "Write", "Bash", "inspect", "probe_inspect", "other"}
TRANSPORT_TASK_STATES = {"ok", "runtime_error", "join_error", "join_timeout"}
FAILURE_SOURCES = {"production_runtime", "fixture_loop", "join_error", "join_timeout", "acceptance"}
RUNTIME_ERROR_KINDS = {"stale_generation", "controller_closed", "unsupported_version", "invalid_configuration",
    "permission_ceiling_rejected", "protocol", "io", "request_timed_out", "event_backpressure"}
PROTOCOL_FAILURE_KINDS = {"invalid_response_id", "invalid_jsonrpc_version", "unsolicited_response",
    "conflicting_response", "stdout_closed", "identity_ledger_limit", "unknown", "not_protocol"}
NATIVE_ERROR_CATEGORIES = {"http_401", "http_402", "http_403", "http_429", "http_4xx", "http_5xx", "unknown"}
FAILURE_DIAGNOSTIC_KEYS = {"transport_task_status", "transport_error", "transport_join_error",
    "last_native_response_diagnostic", "failure_source", "private_error_capture_status",
    "private_error_response_bytes", "private_response_envelope_status", "private_response_envelope_bytes",
    "private_response_envelope_sha256"}
PRIVATE_ERROR_CAPTURE_STATES = {"not_observed", "captured", "budget_rejected", "encoding_failed", "write_failed"}
PRIVATE_ENVELOPE_STATES = PRIVATE_ERROR_CAPTURE_STATES | {"scope_rejected"}
TRANSACTION_METHODS = {"initialize", "authenticate", "session/new", "session/load", "session/prompt",
    "session/cancel", "x.ai/session/info", "x.ai/session/close"}
PENDING_KINDS = {"initialize", "authenticate", "new_session", "load_session", "prompt", "final_output", "close_session"}
DIAGNOSTIC_SUMMARY_KEYS = {"error_message", "error_data_message", "runtime_error_message",
    "result_stop_reason", "response_id", "transport_join_error"}
MCP_REASONS = {"transport_closed", "handshake_failed", "config_added", "config_removed", "config_changed",
    "disabled", "auth_expired", "initialized", "restart_succeeded", "restart_failed", "managed_token_refreshed"}
TYPE_KEYS = {"sdk_capability_type", "params_type", "session_id_type", "total_type", "connected_type",
    "mcp_tool_count_type", "elapsed_ms_type", "name_type", "detail_type", "native_tool_name_type",
    "error_type", "error_code_type", "result_type"}
COUNTER_KEYS = {"total", "connected", "mcp_tool_count", "elapsed_ms", "bytes", "response_id_number",
    "private_error_response_bytes", "private_response_envelope_bytes", "id_number"}
FIXED_VALUES = EVENTS | ORIGIN_STATES | {SCOPE, "sdk_mcp", "initialize", "tools/list", "tools/call", "ping", "server/discover",
    "unknown", "other", "absent", "null", "boolean", "number", "string", "array", "object",
    "absent_or_null", "unknown_or_absent", "outer_meta", "sdk_params", "sdk_params_meta",
    "mcp_envelope", "mcp_envelope_meta", "mcp_params_meta", "pending", "in_progress",
    "completed", "failed", "cancelled", "inspect_local_tasks"}
KEYS = {"event", "scope", "max_native_inputs", "origin_verification", "credential_files_read_by_probe",
    "skills_reload_closed_success_shape",
    "public_product_gate_open", "framework", "protocol_version", "sequence", "outer_id", "inner_id",
    "mcp_method", "outer_keys", "params_keys", "inner_keys", "tool_params_keys", "metadata_keys",
    "origin_fields", "origin_field_types", "origin", "origin_present", "origin_field_presence",
    "session_id", "prompt_id", "event_id", "tool_call_id", "status", "probe_tool", "raw_input_present",
    "raw_input_sha256", "passed", "native_inputs", "sdk_request_count", "inspect_call_count",
    "full_native_origin_fields_observed", "native_origin_ledger_relation_verified", "candidate_mapping_only",
    "no_side_effects", "approval_all_denied", "permission_request_count", "unexpected_tool_count",
    "cleanup_confirmed", "jsonrpc", "id", "method", "params", "result", "error", "serverId", "message",
    "_meta", "name", "arguments", "sessionId", "promptId", "turnId", "toolCallId", "eventId",
    "turn_id", "present", "type", "kind", "field", "native_verified", "outer", "sdk_params",
    "mcp_envelope", "mcp_params", "carrier", "fields", "sha256", "initialization_count",
    "tools_list_count", "duplicate_request_count", "unsafe_key_name_count", "origin_observations",
    "submitted_input_count",
    "session_id_matches", "prompt_id_matches", "tool_call_id_in_native_ledger", "native_session_id",
    "native_prompt_id", "no_project_files", "transport_closed", "cleanup_receipt_read",
    "parent_permission_ceiling_verified", "failure_reason", "requested_protocol_version", "wire_method",
    "sdk_capability_present", "sdk_capability_type", "sdk_capability_enabled", "notification_method",
    "params_type", "session_known", "session_id_present", "session_id_type", "total", "total_type",
    "connected", "connected_type", "mcp_tool_count", "mcp_tool_count_type", "elapsed_ms", "elapsed_ms_type",
    "name_type", "probe_server_name_matches", "mcp_status", "mcp_reason", "detail_present", "detail_type",
    "detail_sha256", "mcp_status_observation_count", "native_tool_name_type", "native_tool_name_sha256",
    "native_tool_kind", "request_id", "decision", "duplicate", "native_contract_verified",
    "approval_allow_count", "approval_deny_count", "approval_duplicate_count",
    "request_payload_sha256", "selected_option_id", "search_allow_count", "inspect_allow_count",
    "discovery_native_contract_verified", "approval_contract_verified", "calibrated_tool_kind", "discovery_tool",
    "discovery_count", "negotiatedProtocolVersion", "servedToolNames", "initialization_mode",
    "protocol_version_carrier", "metadata_schema_valid", "requested_protocol_version_sha256",
    "io.modelcontextprotocol/protocolVersion", "io.modelcontextprotocol/clientInfo",
    "io.modelcontextprotocol/clientCapabilities", "io.modelcontextprotocol/serverInfo",
    "response_diagnostic", "jsonrpc_is_2_0", "response_id", "response_id_number", "method_present",
    "error_present", "error_type", "error_code", "error_code_type", "error_message", "error_data_message",
    "result_present", "result_type", "result_stop_reason", "result_error_conflict", "bytes",
    "runtime_error_kind", "protocol_failure_kind", "runtime_error_message",
    "native_error_http_status", "native_error_category", "transaction_context", "generation", "next_request_id",
    "pending_id", "pending_kind", "id_number", "inner_response_id"} | FAILURE_DIAGNOSTIC_KEYS
ID_KEYS = {"outer_id", "inner_id", "session_id", "prompt_id", "event_id", "tool_call_id",
    "native_session_id", "native_prompt_id", "request_id"}
KEY_SET_KEYS = {"outer_keys", "params_keys", "inner_keys", "tool_params_keys", "metadata_keys"}
_budget_lock = threading.Lock()


def sha(value):
    return hashlib.sha256(value).hexdigest()


@contextmanager
def bounded_tunnel(timeout):
    # 原模块在每个 CONNECT 前读取预算；本独立进程临时设置连接预算，退出时恢复。
    with _budget_lock:
        previous = official.MAX_TUNNELS
        official.MAX_TUNNELS = MAX_TLS_CONNECTIONS
        tunnel = None
        try:
            tunnel = official.OfficialTunnel(timeout)
            yield tunnel
        finally:
            try:
                if tunnel is not None and not tunnel.closing:
                    if tunnel.thread.is_alive():
                        tunnel.close()
                    else:
                        tunnel.server.server_close()
            finally:
                official.MAX_TUNNELS = previous


def project_deny(root):
    project = str(root / "project")
    paths = {project, project.replace("/private/tmp/", "/tmp/")}
    return "".join(f"(deny file-write* (subpath {json.dumps(path)}))" for path in sorted(paths))


def prepare_probe_native(root, native, source_home, port):
    wrapper, settings = official.prepare_native(root, native, source_home, port)
    code = wrapper.read_text(encoding="utf-8")
    boundary = "if args==['--version']:"
    leader = """elif len(args)==4 and args[:3]==['agent','stdio','--leader-socket']:
 endpoint=Path(args[3]);resolved=endpoint.resolve()
 if not endpoint.is_absolute() or endpoint!=resolved or not resolved.is_relative_to(socket_root) or resolved.name!='leader.sock' or resolved.exists():raise SystemExit(92)
 parent=resolved.parent
 if parent.stat().st_uid!=os.getuid() or parent.stat().st_mode & 0o077:raise SystemExit(93)
 quoted=json.dumps(str(resolved))
 profile+='(allow file-write* (subpath '+json.dumps(str(socket_root))+'))(allow network-bind network-inbound (literal '+quoted+'))(allow network-outbound (remote unix-socket (path-literal '+quoted+')))'
 kind='private_leader'
"""
    if code.count(boundary) != 1 or code.count(leader) != 1:
        raise ValueError("原生包装器边界改变；禁止探针启动")
    # 仅 SDK 专用探针接受独立 stdio；删去额外 socket 权限，原样传递已校验的 argv。
    code = code.replace(leader, "elif args==['agent','--no-leader','stdio']:kind='direct_agent';endpoint=None\n", 1)
    code = code.replace(boundary, f"profile+={project_deny(root)!r}\n{boundary}", 1)
    compile(code, str(wrapper), "exec")
    wrapper.write_text(code, encoding="utf-8")
    return wrapper, settings


def project_write_canary(root, source_home, port, timeout):
    target = root / "project/.sdk-origin-write-canary"
    profile = shared.sandbox_profile(root, source_home, port) + project_deny(root)
    code = ("import json\ntry:\n with open(" + repr(str(target))
        + ",'x') as f:f.write('canary')\n result=True\nexcept OSError as e:result=e.errno\nprint(json.dumps({'project_write':result}))\n")
    result = subprocess.run(["/usr/bin/sandbox-exec", "-p", profile, sys.executable, "-c", code],
        env=official.official_environment(root, port), capture_output=True, text=True,
        timeout=min(15, timeout), check=False)
    observed = json.loads(result.stdout) if result.stdout else None
    if (result.returncode != 0 or json.dumps(observed, sort_keys=True) != '{"project_write": 1}'
            or target.exists()):
        raise ValueError("OS 沙箱未证明项目写入拒绝；禁止模型请求")
    return observed


def safe_key(value):
    return value if value in KEYS else "sha256:" + sha(value.encode("utf-8", errors="replace"))


def safe_id(value):
    if value is None or type(value) is int and -(2 ** 63) <= value < 2 ** 63:
        return value
    if isinstance(value, str):
        # 不把任意原生 ID 当作可公开文本；散列仍可比较同一请求和回调。
        return {"type": "string", "sha256": sha(value.encode("utf-8", errors="replace"))}
    if isinstance(value, dict) and set(value) == {"type", "sha256"} and value.get("type") == "string":
        digest = value.get("sha256")
        if isinstance(digest, str) and re.fullmatch(r"[0-9a-f]{64}", digest):
            return value
    if (isinstance(value, dict) and set(value) == {"type"} and isinstance(value["type"], str)
            and value["type"] in FIXED_VALUES):
        return value
    return {"type": type(value).__name__, "value_omitted": True}


def diagnostic_summary(value):
    if not isinstance(value, dict):
        return False
    if value == {"type": "absent"}:
        return True
    return (set(value) == {"type", "bytes", "sha256"} and isinstance(value["type"], str)
        and value["type"] in JSON_TYPES - {"absent", "absent_or_null"}
        and type(value["bytes"]) is int and 0 <= value["bytes"] <= MAX_EVIDENCE_BYTES
        and isinstance(value["sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", value["sha256"]) is not None)


def transaction_context_valid(value):
    if not isinstance(value, dict) or set(value) != {"generation", "next_request_id", "pending_id", "pending_kind"}:
        return False
    generation = value["generation"]
    if not isinstance(generation, str) or re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", generation) is None:
        return False
    numeric = lambda item: type(item) is int and 0 <= item < 2 ** 63
    return (numeric(value["next_request_id"]) and (value["pending_id"] is None or numeric(value["pending_id"]))
        and (value["pending_kind"] is None or isinstance(value["pending_kind"], str) and value["pending_kind"] in PENDING_KINDS)
        and (value["pending_id"] is None) == (value["pending_kind"] is None))


def outbound_transaction_valid(value):
    fields = {"event", "sequence", "transaction_context", "method", "id", "id_number", "inner_response_id",
        "method_present", "result_present", "error_present"}
    return (isinstance(value, dict) and set(value) == fields and value["event"] == "outbound_transaction_observed"
        and type(value["sequence"]) is int and 0 < value["sequence"] < 2 ** 63
        and transaction_context_valid(value["transaction_context"])
        and (isinstance(value["method"], str) and value["method"] in TRANSACTION_METHODS or diagnostic_summary(value["method"]))
        and diagnostic_summary(value["id"]) and diagnostic_summary(value["inner_response_id"])
        and (value["id_number"] is None or type(value["id_number"]) is int and 0 <= value["id_number"] < 2 ** 63)
        and all(type(value[key]) is bool for key in ("method_present", "result_present", "error_present"))
        and (value["id_number"] is None or value["id"]["type"] == "number")
        and (value["method_present"] == (value["method"] != {"type": "absent"}))
        and (value["result_present"] or value["inner_response_id"] == {"type": "absent"}))


def projection(value, key=None, depth=0):
    if depth > 5:
        return {"value_omitted": True}
    if key in ID_KEYS:
        return safe_id(value)
    if key == "skills_reload_closed_success_shape":
        return value if type(value) is bool else None
    if key == "transaction_context":
        return value if transaction_context_valid(value) else {"type": type(value).__name__, "sha256": sha(json.dumps(value, sort_keys=True).encode())}
    if key in {"method", "id", "inner_response_id"}:
        if key == "method" and isinstance(value, str) and value in TRANSACTION_METHODS | {"tools/list", "tools/call", "server/discover", "ping"} or diagnostic_summary(value):
            return value
        return {"type": type(value).__name__, "sha256": sha(json.dumps(value, sort_keys=True).encode())}
    if key in DIAGNOSTIC_SUMMARY_KEYS:
        if value is None:
            return None
        if (isinstance(value, dict) and set(value) <= {"type", "bytes", "sha256"}
                and isinstance(value.get("type"), str) and value["type"] in JSON_TYPES
                and ("bytes" not in value or type(value["bytes"]) is int and 0 <= value["bytes"] < 2 ** 63)
                and ("sha256" not in value or isinstance(value["sha256"], str)
                    and re.fullmatch(r"[0-9a-f]{64}", value["sha256"]))):
            return value
        # 正文即使恰好等于某个协议枚举，也不能作为错误摘要原文公开。
        return {"type": type(value).__name__, "sha256": sha(json.dumps(value,
            sort_keys=True, ensure_ascii=False).encode("utf-8"))}
    if key in {"transport_error", "last_native_response_diagnostic"} and isinstance(value, dict):
        allowed = ({"runtime_error_kind", "protocol_failure_kind", "runtime_error_message"}
            if key == "transport_error" else {"jsonrpc", "jsonrpc_is_2_0", "response_id", "response_id_number",
                "method_present", "error_present", "error_type", "error_code", "error_code_type",
                "error_message", "error_data_message", "result_present", "result_type",
                "result_stop_reason", "result_error_conflict", "native_error_http_status", "native_error_category",
                "transaction_context"})
        return {safe_key(str(name)): (projection(item, "runtime_error_message" if name == "jsonrpc" else name,
            depth + 1) if name in allowed else {"type": type(item).__name__, "sha256": sha(json.dumps(item,
                sort_keys=True, ensure_ascii=False).encode("utf-8"))}) for name, item in value.items()}
    if key in COUNTER_KEYS:
        return value if type(value) is int and 0 <= value < 2 ** 63 else None
    if key == "error_code":
        return value if type(value) is int and -(2 ** 63) <= value < 2 ** 63 else None
    if key == "native_error_http_status":
        return value if type(value) is int and 100 <= value <= 599 else None
    if key == "private_response_envelope_sha256":
        return value if value is None or isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) else None
    if key in {"transport_task_status", "failure_source", "runtime_error_kind", "protocol_failure_kind",
            "private_error_capture_status", "private_response_envelope_status", "native_error_category"}:
        if key == "failure_source" and value is None:
            return None
        allowed = {"transport_task_status": TRANSPORT_TASK_STATES, "failure_source": FAILURE_SOURCES,
            "runtime_error_kind": RUNTIME_ERROR_KINDS, "protocol_failure_kind": PROTOCOL_FAILURE_KINDS,
            "private_error_capture_status": PRIVATE_ERROR_CAPTURE_STATES,
            "private_response_envelope_status": PRIVATE_ENVELOPE_STATES,
            "native_error_category": NATIVE_ERROR_CATEGORIES}[key]
        return value if isinstance(value, str) and value in allowed else {"type": type(value).__name__,
            "sha256": sha(json.dumps(value, sort_keys=True, ensure_ascii=False).encode("utf-8"))}
    if key == "servedToolNames":
        if isinstance(value, list) and all(item == "inspect" for item in value) and len(value) <= 1:
            return value
        return {"type": type(value).__name__, "sha256": sha(json.dumps(value,
            sort_keys=True, ensure_ascii=False).encode("utf-8"))}
    if key in {"protocol_version", "requested_protocol_version", "negotiatedProtocolVersion",
            "initialization_mode", "protocol_version_carrier"}:
        allowed = (INITIALIZATION_MODES if key == "initialization_mode" else
            VERSION_CARRIERS if key == "protocol_version_carrier" else MCP_VERSIONS)
        if value is None or isinstance(value, str) and value in allowed:
            return value
        return {"type": "string" if isinstance(value, str) else type(value).__name__, "sha256": sha(json.dumps(value,
            sort_keys=True, ensure_ascii=False).encode("utf-8"))}
    # 已校准的 always-allow 仅可出现在原生选项列表，公开选中项只认 allow-once。
    if key == "selected_option_id":
        if value is None or value == "allow-once":
            return value
        return {"type": type(value).__name__, "sha256": sha(json.dumps(value,
            sort_keys=True, ensure_ascii=False).encode("utf-8"))}
    if key == "decision":
        if isinstance(value, str) and value in {"allow_once", "denied"}:
            return value
        return {"type": type(value).__name__, "sha256": sha(json.dumps(value,
            sort_keys=True, ensure_ascii=False).encode("utf-8"))}
    if key == "calibrated_tool_kind":
        if isinstance(value, str) and value in CALIBRATED_TOOL_KINDS:
            return value
        return {"type": type(value).__name__, "sha256": sha(json.dumps(value,
            sort_keys=True, ensure_ascii=False).encode("utf-8"))}
    if key == "native_tool_kind":
        # 分类仅来自封闭枚举，不能公开任意原生名称或推断工具许可。
        if isinstance(value, str) and value in NATIVE_TOOL_KINDS:
            return value
        return {"type": type(value).__name__, "sha256": sha(json.dumps(value,
            sort_keys=True, ensure_ascii=False).encode("utf-8"))}
    if key in KEY_SET_KEYS and isinstance(value, list):
        return sorted({safe_key(item) for item in value if isinstance(item, str)})
    if key == "metadata_keys" and isinstance(value, dict):
        return {safe_key(str(name)): projection(item, "params_keys", depth + 1) for name, item in value.items()}
    if value is None or type(value) is bool:
        return value
    if type(value) is int:
        return value if -(2 ** 63) <= value < 2 ** 63 else {"type": "integer", "value_omitted": True}
    if isinstance(value, str):
        if key in {"raw_input_sha256", "detail_sha256", "native_tool_name_sha256", "request_payload_sha256", "requested_protocol_version_sha256", "sha256"}:
            return value if re.fullmatch(r"[0-9a-f]{64}", value) else {"type": "string", "sha256": sha(value.encode("utf-8", errors="replace"))}
        if key in {"wire_method", "notification_method", "mcp_status", "mcp_reason"} or key in TYPE_KEYS:
            allowed = {"wire_method": SDK_WIRE_METHODS, "notification_method": MCP_STATUS_METHODS,
                "mcp_status": MCP_STATUSES, "mcp_reason": MCP_REASONS}.get(key, JSON_TYPES)
            return value if value in allowed else {"type": "string", "sha256": sha(value.encode("utf-8", errors="replace"))}
        if value in FIXED_VALUES:
            return value
        return {"type": "string", "sha256": sha(value.encode("utf-8", errors="replace"))}
    if isinstance(value, dict):
        return {safe_key(str(name)): (projection(item, name, depth + 1) if name in KEYS else
            {"type": type(item).__name__, "sha256": sha(json.dumps(item, sort_keys=True,
                ensure_ascii=False).encode("utf-8"))}) for name, item in value.items()}
    if isinstance(value, list):
        return [projection(item, depth=depth + 1) for item in value[:256]]
    return {"type": type(value).__name__, "value_omitted": True}


def public_events(events):
    result = []
    for event in events:
        if (not isinstance(event.get("event"), str) or event["event"] not in EVENTS
                or event["event"] == "outbound_transaction_observed" and not outbound_transaction_valid(event)):
            result.append({"event": "unrecognized_private_event", "sha256": sha(json.dumps(event,
                sort_keys=True, ensure_ascii=False).encode("utf-8"))})
            continue
        result.append(projection(event))
    return result


def private_bytes(path, max_bytes=MAX_EVIDENCE_BYTES):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        attributes = os.fstat(descriptor)
        if (not stat.S_ISREG(attributes.st_mode) or attributes.st_uid != os.getuid()
                or attributes.st_mode & 0o077 or attributes.st_nlink != 1
                or attributes.st_size > max_bytes):
            raise ValueError("私有证据必须是有限大小的独占普通文件")
        with os.fdopen(os.dup(descriptor), "rb") as file:
            content = file.read(max_bytes + 1)
        if len(content) > max_bytes:
            raise ValueError("私有证据在读取期间超过大小预算")
        return content
    finally:
        os.close(descriptor)


def private_events(path):
    events = [json.loads(line) for line in private_bytes(path).decode("utf-8").splitlines() if line.strip()]
    if not all(isinstance(event, dict) for event in events):
        raise ValueError("私有证据事件必须是对象")
    return events


def private_error_audit(path):
    raw = private_bytes(path, MAX_PRIVATE_ERROR_BYTES)
    records = [json.loads(line) for line in raw.splitlines() if line.strip()]
    if len(records) > 1:
        raise ValueError("私有原生错误只允许一个响应")
    for response in records:
        if (not isinstance(response, dict) or not set(response) <= {"jsonrpc", "id", "error"}
                or not isinstance(response.get("error"), dict)
                or not set(response["error"]) <= {"code", "message", "data"}):
            raise ValueError("私有原生错误出现未经允许的字段")
        scalar = lambda value: value is None or type(value) in (str, int, float)
        if (any(not scalar(value) for key, value in response.items() if key != "error")
                or any(not scalar(value) for key, value in response["error"].items() if key != "data")):
            raise ValueError("私有原生错误字段必须为标量")
        if "data" in response["error"]:
            data = response["error"]["data"]
            if (not isinstance(data, dict) or not set(data) <= {"message", "http_status", "code", "error_type"}
                    or any(not scalar(value) for value in data.values())):
                raise ValueError("私有原生错误 data 出现未经允许的字段")
    # 原文仅留在独占侧文件；公开 metadata 只有文件长度、数量与摘要。
    return {"private_native_error_response_bytes": len(raw), "private_native_error_response_count": len(records),
        "private_native_error_response_sha256": sha(raw), "private_native_error_response_scope_verified": True}


def private_response_envelope_audit(path):
    # 精确ID仅在当前用户独占目录/文件中校验，公开元数据只保存文件摘要。
    if os.name != "posix":
        raise ValueError("私有信封独占权限尚未验证此平台")
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        attributes = os.fstat(directory)
        if attributes.st_uid != os.getuid() or stat.S_IMODE(attributes.st_mode) != 0o700:
            raise ValueError("私有信封目录权限不匹配")
        descriptor = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory)
        try:
            attributes = os.fstat(descriptor)
            if (not stat.S_ISREG(attributes.st_mode) or attributes.st_uid != os.getuid()
                    or stat.S_IMODE(attributes.st_mode) != 0o600 or attributes.st_nlink != 1
                    or attributes.st_size > MAX_PRIVATE_ERROR_BYTES):
                raise ValueError("私有信封文件范围不匹配")
            with os.fdopen(os.dup(descriptor), "rb") as file:
                raw = file.read(MAX_PRIVATE_ERROR_BYTES + 1)
        finally:
            os.close(descriptor)
    finally:
        os.close(directory)
    if len(raw) > MAX_PRIVATE_ERROR_BYTES:
        raise ValueError("私有信封超过预算")
    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("私有信封重复字段")
            result[key] = value
        return result
    def reject_constant(value):
        raise ValueError("私有信封非有限数字")
    records = [json.loads(line, object_pairs_hook=unique_object, parse_constant=reject_constant)
        for line in raw.splitlines() if line.strip()]
    if len(records) > 1:
        raise ValueError("私有信封只允许一个响应")
    for envelope in records:
        fields = {"jsonrpc", "id", "result_type", "result_field_count", "result_fields", "error_present",
            "skills_reload_closed_success_shape"}
        if not isinstance(envelope, dict) or set(envelope) != fields:
            raise ValueError("私有信封出现额外字段")
        if (envelope["id"] is not None and type(envelope["id"]) not in (str, int, float)
                or type(envelope["id"]) is float and not math.isfinite(envelope["id"])
                or envelope["jsonrpc"] != "2.0" and not diagnostic_summary(envelope["jsonrpc"])
                or not isinstance(envelope["result_type"], str) or envelope["result_type"] not in JSON_TYPES - {"absent_or_null"}
                or type(envelope["result_field_count"]) is not int or not 0 <= envelope["result_field_count"] < 2 ** 63
                or type(envelope["error_present"]) is not bool
                or type(envelope["skills_reload_closed_success_shape"]) is not bool):
            raise ValueError("私有信封字段形状无效")
        rows = envelope["result_fields"]
        if envelope["result_type"] != "object":
            if rows is not None or envelope["result_field_count"] != 0:
                raise ValueError("私有信封结果类型不匹配")
        elif (not isinstance(rows, list) or len(rows) != min(envelope["result_field_count"], 128)
                or any(not isinstance(row, dict) or set(row) != {"key", "value_type"}
                    or not diagnostic_summary(row["key"]) or row["key"]["type"] != "string"
                    or not isinstance(row["value_type"], str) or row["value_type"] not in JSON_TYPES - {"absent", "absent_or_null"} for row in rows)):
            raise ValueError("私有信封结果字段范围无效")
        # 原始内层值已在 Rust 端丢弃；这里只核对布尔诊断与允许的外层指纹一致。
        if envelope["skills_reload_closed_success_shape"] and (
                envelope["jsonrpc"] != "2.0" or envelope["id"] != "skills-reload"
                or envelope["error_present"] or envelope["result_type"] != "object"
                or envelope["result_field_count"] != 1
                or rows != [{"key": {"type": "string", "bytes": 6, "sha256": sha(b"result")},
                    "value_type": "object"}]):
            raise ValueError("私有维护形状诊断与信封不匹配")
    audit = {"private_native_response_envelope_bytes": len(raw), "private_native_response_envelope_count": len(records),
        "private_native_response_envelope_sha256": sha(raw), "private_native_response_envelope_scope_verified": True}
    if records:
        audit["skills_reload_closed_success_shape"] = records[0]["skills_reload_closed_success_shape"]
    return audit


def response_envelope_ledger_matches(audit, events):
    finishes = [event for event in events if event.get("event") == "probe_finished"]
    if len(finishes) != 1:
        return False
    finish = finishes[0]
    status = finish.get("private_response_envelope_status")
    size = finish.get("private_response_envelope_bytes")
    digest = finish.get("private_response_envelope_sha256")
    return (isinstance(status, str) and status in {"not_observed", "captured"} and type(size) is int
        and audit["private_native_response_envelope_bytes"] == size
        and audit["private_native_response_envelope_count"] == int(status == "captured")
        and (digest == audit["private_native_response_envelope_sha256"] if status == "captured" else size == 0 and digest is None))


def exact_approval_observation(finish, events):
    # 两类只读许可分别有界，搜索账本不能成为 SDK inspect 来源。
    names = ("permission_request_count", "approval_allow_count", "approval_deny_count", "approval_duplicate_count",
        "search_allow_count", "inspect_allow_count")
    counters = {key: finish.get(key) for key in names}
    if not all(type(value) is int and value >= 0 for value in counters.values()):
        return False
    requests, allow, deny, duplicates, search, inspect = (counters[key] for key in names)
    approvals = [event for event in events if event.get("event") == "probe_approval_observed"]
    if (allow > 2 or search > 1 or inspect > 1 or allow != search + inspect or deny != 0
            or requests != len(approvals) or requests != allow + duplicates
            or duplicates > 0 and allow == 0
            or finish.get("approval_all_denied") is not (allow == 0)
            or finish.get("native_contract_verified") is not True
            or type(finish.get("discovery_native_contract_verified")) is not bool):
        return False
    ledger = [event for event in events if event.get("event") == "native_tool_update_received"]
    if finish["discovery_native_contract_verified"] is True and not any(
            tool.get("discovery_tool") is True and tool.get("calibrated_tool_kind") == "search_discovery"
            and tool.get("session_id") == finish.get("native_session_id")
            and tool.get("prompt_id") == finish.get("native_prompt_id") for tool in ledger):
        return False
    first_by_kind = {}
    duplicate_count = 0
    for event in approvals:
        kind = event.get("calibrated_tool_kind")
        if not isinstance(kind, str) or kind not in {"inspect", "search_discovery"}:
            return False
        first = first_by_kind.get(kind)
        repeated = first is not None
        if first is None:
            first = event
            first_by_kind[kind] = event
        if (event.get("decision") != "allow_once" or event.get("selected_option_id") != "allow-once"
                or event.get("duplicate") is not repeated
                or event.get("request_id") is None or event.get("request_id") != first.get("request_id")
                or event.get("session_id") is None or event.get("session_id") != finish.get("native_session_id")
                or event.get("prompt_id") is None or event.get("prompt_id") != finish.get("native_prompt_id")
                or event.get("tool_call_id") is None or event.get("tool_call_id") != first.get("tool_call_id")
                or not all(event.get(key) is True for key in ("approval_contract_verified", "session_id_matches",
                    "prompt_id_matches", "tool_call_id_in_native_ledger"))
                or event.get("native_contract_verified") is not (kind == "inspect")
                or event.get("discovery_native_contract_verified") is not (kind == "search_discovery")
                or kind == "search_discovery" and finish["discovery_native_contract_verified"] is not True
                or not isinstance(event.get("raw_input_sha256"), str)
                or not re.fullmatch(r"[0-9a-f]{64}", event["raw_input_sha256"])
                or event["raw_input_sha256"] != first.get("raw_input_sha256")
                or not isinstance(event.get("request_payload_sha256"), str)
                or not re.fullmatch(r"[0-9a-f]{64}", event["request_payload_sha256"])
                or event["request_payload_sha256"] != first.get("request_payload_sha256")
                or not any(tool.get("tool_call_id") == event["tool_call_id"]
                    and tool.get("session_id") == event["session_id"] and tool.get("prompt_id") == event["prompt_id"]
                    and tool.get("calibrated_tool_kind") == kind
                    and tool.get("probe_tool") is (kind == "inspect")
                    and tool.get("discovery_tool") is (kind == "search_discovery")
                    and tool.get("raw_input_present") is True
                    and tool.get("raw_input_sha256") == event["raw_input_sha256"] for tool in ledger)):
            return False
        duplicate_count += int(repeated)
    if len(first_by_kind) != allow or int("inspect" in first_by_kind) != inspect or int("search_discovery" in first_by_kind) != search:
        return False
    if len(first_by_kind) == 2 and (first_by_kind["inspect"]["request_id"] == first_by_kind["search_discovery"]["request_id"]
            or first_by_kind["inspect"]["tool_call_id"] == first_by_kind["search_discovery"]["tool_call_id"]):
        return False
    return duplicate_count == duplicates


def probe_observation(exit_code, output, events):
    starts = [event for event in events if event.get("event") == "probe_started"]
    finishes = [event for event in events if event.get("event") == "probe_finished"]
    result = {"probe_passed": False, "origin_verification": "unknown", "native_origin_verified": False}
    if len(starts) != 1 or len(finishes) != 1:
        return result
    start, finish = starts[0], finishes[0]
    # 生产错误的安全投影与能力证明分开；失败摘要不能提升来源或产品门禁。
    diagnostics = {key: finish[key] for key in FAILURE_DIAGNOSTIC_KEYS if key in finish}
    if diagnostics:
        result["failure_diagnostics"] = projection(diagnostics)
    state = finish.get("origin_verification")
    if isinstance(state, str) and state in ORIGIN_STATES:
        result["origin_verification"] = state
    counters = {key: finish.get(key) for key in ("native_inputs", "sdk_request_count", "inspect_call_count",
        "permission_request_count", "unexpected_tool_count", "submitted_input_count", "initialization_count",
        "tools_list_count", "approval_allow_count", "approval_deny_count", "approval_duplicate_count",
        "search_allow_count", "inspect_allow_count", "discovery_count")}
    result["native_counters"] = {key: value for key, value in counters.items() if type(value) is int and value >= 0}
    counts_valid = all(type(value) is int and value >= 0 for value in counters.values())
    registrations = [event for event in events if event.get("event") == "registration_received"]
    requests = [event for event in events if event.get("event") == "sdk_request_received"]
    legacy = (len(registrations) == 1 and registrations[0].get("initialization_mode") == "legacy_initialize"
        and registrations[0].get("protocol_version") == finish.get("negotiatedProtocolVersion") == "2025-11-25"
        and counters["initialization_count"] == 1 and counters["discovery_count"] == 0)
    modern = (len(registrations) == 1 and registrations[0].get("initialization_mode") == "modern_discover"
        and registrations[0].get("protocol_version") == finish.get("negotiatedProtocolVersion") == "2026-07-28"
        and counters["initialization_count"] == 0 and counters["discovery_count"] == 1
        and counters["tools_list_count"] == 1 and finish.get("servedToolNames") == ["inspect"]
        and requests and all(request.get("mcp_method") in {"server/discover", "tools/list", "tools/call"}
            and request.get("requested_protocol_version") == "2026-07-28"
            and request.get("protocol_version_carrier") == "params._meta"
            and request.get("metadata_schema_valid") is True for request in requests)
        and all(len({json.dumps(request.get("inner_id"), sort_keys=True) for request in requests
            if request.get("mcp_method") == method}) == 1
            for method in ("server/discover", "tools/list", "tools/call")))
    safe = (start.get("scope") == finish.get("scope") == SCOPE
        and type(start.get("max_native_inputs")) is int and start["max_native_inputs"] == MAX_NATIVE_INPUTS
        and start.get("origin_verification") == "unknown"
        and start.get("credential_files_read_by_probe") is False
        and start.get("public_product_gate_open") is False and finish.get("public_product_gate_open") is False
        and counts_valid and counters["native_inputs"] == MAX_NATIVE_INPUTS
        and counters["submitted_input_count"] == MAX_NATIVE_INPUTS
        and (legacy or modern) and counters["tools_list_count"] > 0
        and counters["inspect_call_count"] == 1 and counters["sdk_request_count"] == len(requests) > 0
        and counters["unexpected_tool_count"] == 0 and exact_approval_observation(finish, events)
        and len(registrations) == 1 and registrations[0].get("framework") == "sdk_mcp"
        and all(finish.get(key) is True for key in ("no_side_effects", "cleanup_confirmed",
            "cleanup_receipt_read", "transport_closed", "no_project_files"))
        and finish.get("parent_permission_ceiling_verified") is False
        and isinstance(state, str) and state in ORIGIN_STATES and finish.get("passed") is True
        and finish.get("transport_task_status", "ok") == "ok"
        and finish.get("transport_error") is None and finish.get("transport_join_error") is None
        and all(outbound_transaction_valid(event) for event in events if event.get("event") == "outbound_transaction_observed")
        and exit_code == 0 and "1 passed; 0 failed" in output and "test result: FAILED" not in output)
    verified = (state == "verified_native_fields" and finish.get("full_native_origin_fields_observed") is True
        and finish.get("native_origin_ledger_relation_verified") is True
        and finish.get("candidate_mapping_only") is False)
    result["probe_passed"] = safe and verified
    result["native_origin_verified"] = safe and verified
    return result


def artifacts(output):
    return [output, output.with_suffix(".metadata.json"), output.with_suffix(".network.json")]


def reserve_artifacts(output):
    created = []
    try:
        for path in artifacts(output):
            descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            os.close(descriptor)
            created.append(path)
    except OSError:
        for path in created:
            path.unlink()
        raise


def validate_paths(args, expected_sha256=shared.BINARY_SHA256):
    if sys.platform != "darwin":
        raise ValueError("官方原生 SDK 隔离探针目前只验证 macOS")
    for name in ("test_binary", "grok", "supervisor"):
        path = getattr(args, name)
        if path.is_symlink() or not path.is_file():
            raise ValueError("可执行输入必须是现有非符号链接文件")
        setattr(args, name, path.resolve(strict=True))
    if args.test_binary == args.supervisor or shared.digest(args.grok) != expected_sha256:
        raise ValueError("需要独立监督入口与固定 Grok 二进制")
    home = args.official_grok_home
    if home.is_symlink() or not home.is_dir() or home.stat().st_uid != os.getuid() or home.stat().st_mode & 0o077:
        raise ValueError("需要当前用户独占的专用 GROK_HOME")
    args.official_grok_home = home.resolve(strict=True)
    if args.max_native_inputs != MAX_NATIVE_INPUTS or not 30 <= args.timeout <= MAX_DEADLINE:
        raise ValueError("固定探针只允许 1 个原生输入与 30–450 秒期限")
    if args.output.is_symlink():
        raise ValueError("证据路径不能是符号链接")
    args.output = args.output.resolve()
    if args.output.suffix != ".ndjson":
        raise ValueError("证据需要 .ndjson 扩展名")
    for path in artifacts(args.output):
        if (path.exists() or path.is_symlink() or path.is_relative_to(args.official_grok_home)
                or path in (args.test_binary, args.grok, args.supervisor)):
            raise ValueError("不得覆盖既有文件或写入原始认证目录")


def run(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    reserve_artifacts(args.output)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-sdk-origin-", dir="/private/tmp")).resolve()
    root.chmod(0o700)
    for relative in ("home", "home/.grok", "project", "tmp", "state"):
        (root / relative).mkdir(parents=True, exist_ok=True, mode=0o700)
    (root / ".infinishell-grok-live-probe").write_text(shared.MARKER, encoding="utf-8")
    raw_path = root / "private-evidence.ndjson"
    raw_path.touch(mode=0o600)
    (root / "wrapper-audit.ndjson").touch(mode=0o600)
    metadata = {"scope": SCOPE, "probe_passed": False, "origin_verification": "unknown",
        "native_origin_verified": False, "public_product_gate_open": False,
        "full_cli_parity_acceptance_passed": False, "private_workspace": str(root),
        "auth_copy_method": "opaque_auth_json_only", "public_credential_values_recorded": False,
        "credential_files_read_by_probe": False, "native_received_real_credential": False,
        "max_native_inputs": MAX_NATIVE_INPUTS, "max_tls_connections": MAX_TLS_CONNECTIONS,
        "requested_model_request_budget": REQUESTED_MODEL_BUDGET, "http_model_call_budget_enforced": False,
        "http_model_calls_observable": False, "cost_budget_enforced": False,
        "budget_boundary": f"单次原生 prompt 与最多 {MAX_TLS_CONNECTIONS} 次 opaque TLS CONNECT；无法计数其中 HTTP 模型请求",
        "deadline_seconds": args.timeout, "max_tls_bytes": official.MAX_BYTES,
        "allowed_https_hosts": sorted(official.OFFICIAL_HOSTS), "tls_decrypted": False,
        "requested_model": official.MODEL, "grok_sha256": shared.digest(args.grok),
        "test_binary_sha256": shared.digest(args.test_binary), "supervisor_sha256": shared.digest(args.supervisor),
        "test_name": TEST_NAME, "same_commit_verified_by_runner": False}
    events, output, settings, settings_before = [], "", None, None
    with bounded_tunnel(args.timeout) as tunnel:
        try:
            port = tunnel.start()
            official.copy_private_auth(args.official_grok_home, root / "home/.grok")
            metadata["native_received_real_credential"] = True
            metadata["sandbox_canary"] = shared.network_canary(root, args.official_grok_home / "auth.json", port)
            remaining = tunnel.deadline - time.monotonic()
            if remaining <= 0 or tunnel.forwarded != 0:
                raise ValueError("探针前置校验已耗尽预算")
            metadata["project_write_canary"] = project_write_canary(root, args.official_grok_home, port, remaining)
            wrapper, settings = prepare_probe_native(root, args.grok, args.official_grok_home, port)
            settings_before = settings.read_bytes()
            environment = official.official_environment(root, port)
            environment.update({"INFINISHELL_GROK_LIVE_ROOT": str(root),
                "INFINISHELL_GROK_LIVE_EXECUTABLE": str(wrapper), "INFINISHELL_GROK_LIVE_ARTIFACT": str(raw_path),
                "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor)})
            metadata["native_environment_names"] = sorted(environment)
            remaining = tunnel.deadline - time.monotonic()
            if remaining <= 0:
                raise ValueError("原生版本校验前已到期限")
            version = subprocess.run([str(wrapper), "--version"], cwd=root / "project", env=environment,
                capture_output=True, text=True, timeout=min(10, remaining), check=True)
            if version.stdout.strip() != shared.VERSION or tunnel.forwarded != 0:
                raise ValueError("固定原生版本或请求预算前置条件不匹配")
            metadata["grok_version"] = shared.VERSION
            remaining = tunnel.deadline - time.monotonic()
            if remaining <= 0:
                raise ValueError("原生探针前已到期限")
            command = [str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
            process = subprocess.Popen(command, cwd=Path(__file__).resolve().parents[2], env=environment,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
            try:
                output, _ = process.communicate(timeout=remaining)
            except subprocess.TimeoutExpired:
                metadata["timed_out"] = True
                process.kill()
                output, _ = process.communicate(timeout=20)
            except BaseException:
                process.kill()
                process.wait(timeout=20)
                raise
            metadata["test_exit_code"] = process.returncode
            events = private_events(raw_path)
            metadata.update(probe_observation(process.returncode, output, events))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            metadata["runner_error_type"] = type(error).__name__
            metadata["probe_passed"] = False
            metadata["native_origin_verified"] = False
        finally:
            metadata["tunnels_stopped"] = tunnel.close()
            auth = root / "home/.grok/auth.json"
            try:
                auth.unlink(missing_ok=True)
                metadata["private_auth_copy_removed"] = not auth.exists()
            except OSError as error:
                metadata["private_auth_copy_removed"] = False
                metadata["auth_cleanup_error_type"] = type(error).__name__
            diagnostic = root / "private-test-output.txt"
            descriptor = os.open(diagnostic, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            with os.fdopen(descriptor, "w", encoding="utf-8") as file:
                file.write(output)
            native_error = root / "private-native-error-response.ndjson"
            try:
                metadata.update(private_error_audit(native_error))
                metadata["private_native_error_response_present"] = True
                finishes = [event for event in events if event.get("event") == "probe_finished"]
                if len(finishes) == 1:
                    finish = finishes[0]
                    status = finish.get("private_error_capture_status", "not_observed")
                    consistent = (status in {"not_observed", "captured"}
                        and metadata["private_native_error_response_count"] == int(status == "captured")
                        and metadata["private_native_error_response_bytes"] == finish.get("private_error_response_bytes", 0))
                    metadata["private_native_error_response_ledger_matches"] = consistent
                    if not consistent:
                        metadata["probe_passed"] = False
                        metadata["native_origin_verified"] = False
            except FileNotFoundError:
                metadata["private_native_error_response_present"] = False
                metadata["probe_passed"] = False
                metadata["native_origin_verified"] = False
            except (OSError, ValueError) as error:
                metadata["private_native_error_response_audit_error_type"] = type(error).__name__
                metadata["probe_passed"] = False
                metadata["native_origin_verified"] = False
            try:
                metadata.update(private_response_envelope_audit(root / "private-native-response-envelope.ndjson"))
                metadata["private_native_response_envelope_ledger_matches"] = response_envelope_ledger_matches(metadata, events)
                if not metadata["private_native_response_envelope_ledger_matches"]:
                    metadata["probe_passed"] = False
                    metadata["native_origin_verified"] = False
            except (OSError, ValueError, TypeError) as error:
                metadata["private_native_response_envelope_audit_error_type"] = type(error).__name__
                metadata["private_native_response_envelope_ledger_matches"] = False
                metadata["probe_passed"] = False
                metadata["native_origin_verified"] = False
            if not events:
                try:
                    events = private_events(raw_path)
                    metadata.update({key: value for key, value in probe_observation(None, "", events).items()
                        if key not in {"probe_passed", "native_origin_verified"}})
                except (OSError, ValueError):
                    metadata["private_evidence_parse_failed"] = True
            if settings is not None and settings_before is not None:
                try:
                    metadata["private_settings_audit"] = official.audit_private_settings(settings_before, settings.read_bytes())
                    metadata["private_settings_unchanged"] = metadata["private_settings_audit"]["bytes_unchanged"]
                except OSError as error:
                    metadata["settings_audit_error_type"] = type(error).__name__
            metadata["project_entry_count"] = sum(1 for _ in (root / "project").rglob("*"))
            launches = []
            try:
                launches = private_events(root / "wrapper-audit.ndjson")
            except (OSError, ValueError):
                metadata["wrapper_audit_parse_failed"] = True
            metadata["native_launch_counters"] = {kind: sum(item.get("kind") == kind for item in launches)
                for kind in ("version", "direct_agent")}
            metadata["native_launch_arguments_unchanged"] = all(item.get("arguments_unchanged") is True for item in launches)
            metadata["network_budget_exhausted"] = any(item.get("event") == "official_connect_budget_rejected"
                for item in tunnel.events)
            if metadata["network_budget_exhausted"]:
                metadata["origin_verification_limit"] = "network_budget_insufficient"
            model_tunnel = any(item == {"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"}
                for item in tunnel.events)
            boundary_ok = (metadata["tunnels_stopped"] and metadata.get("private_auth_copy_removed") is True
                and metadata.get("private_settings_audit", {}).get("settings_scope_verified") is True
                and metadata["project_entry_count"] == 0 and metadata["native_launch_arguments_unchanged"]
                and len(launches) == 3
                and metadata["native_launch_counters"] == {"version": 2, "direct_agent": 1}
                and tunnel.forwarded <= MAX_TLS_CONNECTIONS and model_tunnel
                and not metadata["network_budget_exhausted"] and not metadata.get("timed_out", False))
            metadata["probe_passed"] &= boundary_ok
            metadata["native_origin_verified"] &= metadata["probe_passed"]
            for key, path in (("private_evidence_sha256", raw_path), ("private_test_output_sha256", diagnostic),
                    ("wrapper_audit_sha256", root / "wrapper-audit.ndjson")):
                try:
                    metadata[key] = sha(private_bytes(path))
                except (OSError, ValueError) as error:
                    metadata[key + "_error_type"] = type(error).__name__
                    metadata["probe_passed"] = False
                    metadata["native_origin_verified"] = False
            args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in public_events(events)), encoding="utf-8")
            args.output.with_suffix(".network.json").write_text(json.dumps({"events": tunnel.events,
                "tls_connections_attempted": tunnel.forwarded, "tls_bytes": tunnel.bytes,
                "http_model_calls_observable": False}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            metadata["public_evidence_sha256"] = shared.digest(args.output)
            metadata["public_network_sha256"] = shared.digest(args.output.with_suffix(".network.json"))
            args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("官方 Grok SDK 来源探针" + ("完成" if metadata["probe_passed"] else "未通过"))
    print(f"脱敏证据：{args.output}")
    print(f"私有诊断目录：{root}")
    return 0 if metadata["probe_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--grok", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--official-grok-home", type=Path, required=True)
    parser.add_argument("--max-native-inputs", type=int, default=MAX_NATIVE_INPUTS)
    parser.add_argument("--timeout", type=int, default=MAX_DEADLINE)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        validate_paths(args)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"探针启动失败：{type(error).__name__}；请检查专用登录目录与固定输入。\n")


if __name__ == "__main__":
    raise SystemExit(main())
