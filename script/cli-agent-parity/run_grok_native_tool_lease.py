#!/usr/bin/env python3
"""通过生产 Grok SDK 租约链路执行一次只读 inspect；不证明子任务权限上限。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import time

import run_grok_sdk_origin_probe as isolation

# 只复用已验证的隔离及进程基础设施，不调用旧来源探针或修改它的验收条件。
official = isolation.official
shared = official.shared
SCOPE = "authenticated_process_capability_and_native_tool_lease"
TEST_NAME = "ai::cli_agent_runtime::grok::native_tool_lease_live_tests::" + SCOPE
MAX_NATIVE_INPUTS = 1
MAX_TLS_CONNECTIONS = 32
MAX_TLS_BYTES = 32 * 1024 * 1024
MAX_DEADLINE = 450
BOOL_AUDIT = {"owned_process_confirmed", "capability_confirmed", "retired"}
COUNT_AUDIT = {"registration_requests", "native_tool_frames", "native_initial_inputs", "native_complete_inputs",
    "permission_writes_allow", "permission_writes_deny", "business_dispatches", "reply_writes",
    "native_completions", "protocol_errors", "sdk_origin_observations"}
TRUE_FINISH = {"production_connect_path", "production_sdk_registration", "final_history_verified",
    "authenticated_native_tool_lease_verified", "cleanup_confirmed", "cleanup_receipt_read", "transport_closed",
    "no_project_files"}
FALSE_FINISH = {"test_only_sdk_hook", "full_native_origin_fields_observed", "native_origin_verified",
    "parent_permission_ceiling_verified", "spawn_verified", "coordinator_dispatch_verified", "full_cli_parity_acceptance_passed"}
COUNTERS = {"submitted_input_count", "accepted_input_count", "inspect_call_count", "inspect_approval_count",
    "search_approval_count", "unexpected_tool_count"}
FAILURES = {None, "connection", "fixture_loop", "transport", "cleanup", "acceptance"}
START = {"event": "lease_started", "scope": SCOPE, "max_native_inputs": 1,
    "production_connect_path": True, "test_only_sdk_hook": False, "credential_values_recorded": False}


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("证据含重复字段")
        result[key] = value
    return result


def read_events(path):
    raw = isolation.private_bytes(path, 128 * 1024)
    events = [json.loads(line, object_pairs_hook=_pairs,
        parse_constant=lambda value: (_ for _ in ()).throw(ValueError("非有限 JSON")))
        for line in raw.decode("utf-8").splitlines() if line.strip()]
    if len(events) > 2 or any(type(event) is not dict for event in events):
        raise ValueError("租约证据超出固定事件合同")
    return events


def validate_event(event):
    if event.get("event") == "lease_started":
        return event == START and type(event.get("max_native_inputs")) is int
    keys = {"event", "scope", "passed", "audit_before_shutdown", "failure_source", "final_response_sha256"} | TRUE_FINISH | FALSE_FINISH | COUNTERS
    if set(event) != keys or event.get("event") != "lease_finished" or event.get("scope") != SCOPE:
        return False
    if type(event["passed"]) is not bool or type(event["failure_source"]) not in (str, type(None)) or event["failure_source"] not in FAILURES:
        return False
    if any(type(event[key]) is not bool for key in (TRUE_FINISH | FALSE_FINISH) - {"full_native_origin_fields_observed"}):
        return False
    if any(type(event[key]) is not int or not 0 <= event[key] <= 256 for key in COUNTERS):
        return False
    audit = event["audit_before_shutdown"]
    if type(audit) is not dict or set(audit) != BOOL_AUDIT | COUNT_AUDIT | {"full_native_sdk_origin_fields_observed"}:
        return False
    if any(type(audit[key]) is not bool for key in BOOL_AUDIT):
        return False
    if any(type(audit[key]) is not int or not 0 <= audit[key] <= 16_384 for key in COUNT_AUDIT):
        return False
    for value in (event["full_native_origin_fields_observed"], audit["full_native_sdk_origin_fields_observed"]):
        if value is not None and type(value) is not bool:
            return False
    digest = event["final_response_sha256"]
    return digest is None or (type(digest) is str and len(digest) == 64 and all(c in "0123456789abcdef" for c in digest))


def observation(exit_code, events):
    result = {"lease_passed": False, "authenticated_native_tool_lease_verified": False,
        "native_origin_verified": False, "parent_permission_ceiling_verified": False,
        "spawn_verified": False, "full_cli_parity_acceptance_passed": False}
    if len(events) != 2 or not all(validate_event(event) for event in events) or events[0] != START:
        return result
    end = events[1]
    audit = end["audit_before_shutdown"]
    passed = (type(exit_code) is int and exit_code == 0 and end["passed"] is True
        and end["failure_source"] is None and all(end[key] is True for key in TRUE_FINISH)
        and all(end[key] is False for key in FALSE_FINISH)
        and end["submitted_input_count"] == end["accepted_input_count"] == end["inspect_call_count"] == end["inspect_approval_count"] == 1
        and end["search_approval_count"] <= 1 and end["unexpected_tool_count"] == 0
        and audit["owned_process_confirmed"] is True and audit["capability_confirmed"] is True
        and audit["registration_requests"] >= 2 and audit["native_tool_frames"] >= 3
        and audit["native_initial_inputs"] >= 1 and audit["native_complete_inputs"] >= 1
        and audit["permission_writes_allow"] == audit["business_dispatches"] == audit["reply_writes"] == audit["native_completions"] == 1
        and audit["permission_writes_deny"] == audit["protocol_errors"] == 0
        and audit["retired"] is False and audit["sdk_origin_observations"] == 1
        and audit["full_native_sdk_origin_fields_observed"] is False
        and end["final_response_sha256"] == hashlib.sha256(b"INFINISHELL_NATIVE_LEASE_OK").hexdigest())
    result.update(lease_passed=passed, authenticated_native_tool_lease_verified=passed,
        full_native_origin_fields_observed=end["full_native_origin_fields_observed"])
    return result


def auth_identity(home):
    attributes = (home / "auth.json").lstat()
    if (not stat.S_ISREG(attributes.st_mode) or attributes.st_uid != os.getuid()
            or attributes.st_mode & 0o077 or attributes.st_nlink != 1):
        raise ValueError("认证来源不满足独占文件要求")
    # 不解析、散列或记录认证文件正文。
    return (attributes.st_dev, attributes.st_ino, attributes.st_uid, attributes.st_mode,
        attributes.st_nlink, attributes.st_size, attributes.st_mtime_ns)


def boundary_passed(metadata, launches, tunnel):
    return (metadata.get("test_exit_code") == 0 and metadata.get("tunnels_stopped") is True
        and metadata.get("private_auth_copy_removed") is True
        and metadata.get("original_auth_stat_unchanged") is True
        and metadata.get("private_settings_audit", {}).get("settings_scope_verified") is True
        and metadata.get("project_entry_count") == 0 and metadata.get("timed_out") is not True
        and len(launches) == 3 and all(item.get("arguments_unchanged") is True for item in launches)
        and sorted(item.get("kind", "") for item in launches) == ["direct_agent", "version", "version"]
        and tunnel.forwarded <= MAX_TLS_CONNECTIONS and tunnel.bytes <= MAX_TLS_BYTES
        and any(item == {"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"} for item in tunnel.events)
        and not any(item.get("event") == "official_connect_budget_rejected" for item in tunnel.events))


def run(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    isolation.reserve_artifacts(args.output)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-native-lease-", dir="/private/tmp")).resolve()
    root.chmod(0o700)
    for relative in ("home/.grok", "project", "tmp", "state"):
        (root / relative).mkdir(parents=True, mode=0o700)
    (root / ".infinishell-grok-live-probe").write_text(shared.MARKER, encoding="utf-8")
    raw = root / "private-evidence.ndjson"
    raw.touch(mode=0o600)
    (root / "wrapper-audit.ndjson").touch(mode=0o600)
    log = root / "private-test-output.txt"
    metadata = dict(observation(None, []), scope=SCOPE, test_name=TEST_NAME,
        private_workspace=str(root), max_native_inputs=1, max_tls_connections=MAX_TLS_CONNECTIONS,
        max_tls_bytes=MAX_TLS_BYTES, deadline_seconds=args.timeout, http_model_calls_observable=False,
        http_model_call_budget_enforced=False, cost_budget_enforced=False, tls_decrypted=False,
        budget_boundary="最多一次原生输入、32 次 TLS CONNECT、32 MiB 与 450 秒；不能计数加密 HTTP 内模型请求",
        same_commit_verified_by_runner=False, public_credential_values_recorded=False,
        requested_model=official.MODEL, allowed_https_hosts=sorted(official.OFFICIAL_HOSTS),
        auth_copy_method="opaque_auth_json_only", grok_sha256=shared.digest(args.grok),
        test_binary_sha256=shared.digest(args.test_binary), supervisor_sha256=shared.digest(args.supervisor))
    events, launches, settings, before_settings = [], [], None, None
    before_auth = auth_identity(args.official_grok_home)
    with isolation.bounded_tunnel(args.timeout) as tunnel:
        try:
            port = tunnel.start()
            official.copy_private_auth(args.official_grok_home, root / "home/.grok")
            metadata["sandbox_canary"] = shared.network_canary(root, args.official_grok_home / "auth.json", port)
            metadata["project_write_canary"] = isolation.project_write_canary(root, args.official_grok_home, port,
                tunnel.deadline - time.monotonic())
            wrapper, settings = isolation.prepare_probe_native(root, args.grok, args.official_grok_home, port)
            before_settings = settings.read_bytes()
            environment = official.official_environment(root, port)
            environment.update(INFINISHELL_GROK_LIVE_ROOT=str(root), INFINISHELL_GROK_LIVE_EXECUTABLE=str(wrapper),
                INFINISHELL_GROK_LIVE_ARTIFACT=str(raw), INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(args.supervisor))
            remaining = tunnel.deadline - time.monotonic()
            if remaining <= 0 or tunnel.forwarded != 0:
                raise ValueError("前置校验已耗尽预算")
            version = subprocess.run([str(wrapper), "--version"], env=environment, cwd=root / "project",
                capture_output=True, text=True, timeout=min(10, remaining), check=True)
            if version.stdout.strip() != shared.VERSION or tunnel.forwarded != 0:
                raise ValueError("CLI 版本或网络预算不匹配")
            command = [str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
            remaining = tunnel.deadline - time.monotonic()
            if remaining <= 0:
                raise ValueError("真实输入前已到期限")
            with os.fdopen(os.open(log, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600), "wb") as output:
                process = subprocess.Popen(command, cwd=Path(__file__).resolve().parents[2], env=environment,
                    stdout=output, stderr=subprocess.STDOUT)
                try:
                    process.wait(timeout=remaining)
                except subprocess.TimeoutExpired:
                    metadata["timed_out"] = True
                    process.kill()
                    process.wait(timeout=20)
                except BaseException:
                    process.kill()
                    process.wait(timeout=20)
                    raise
            metadata["test_exit_code"] = process.returncode
            events = read_events(raw)
            metadata.update(observation(process.returncode, events))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            metadata["lease_passed"] = False
            metadata["authenticated_native_tool_lease_verified"] = False
            metadata["runner_error_type"] = type(error).__name__
        finally:
            metadata["tunnels_stopped"] = tunnel.close()
            try:
                auth = root / "home/.grok/auth.json"
                auth.unlink(missing_ok=True)
                metadata["private_auth_copy_removed"] = not auth.exists()
                metadata["original_auth_stat_unchanged"] = auth_identity(args.official_grok_home) == before_auth
                if settings is not None and before_settings is not None:
                    metadata["private_settings_audit"] = official.audit_private_settings(before_settings, settings.read_bytes())
                metadata["project_entry_count"] = sum(1 for _ in (root / "project").rglob("*"))
                launches = isolation.private_events(root / "wrapper-audit.ndjson")
            except (OSError, ValueError) as error:
                metadata["cleanup_error_type"] = type(error).__name__
            metadata["boundary_passed"] = boundary_passed(metadata, launches, tunnel)
            metadata["lease_passed"] &= metadata["boundary_passed"]
            metadata["authenticated_native_tool_lease_verified"] &= metadata["lease_passed"]
            # 原生输出日志仅留在私有目录；公开事件必须完整匹配固定标量合同。
            safe = events if all(validate_event(event) for event in events) else []
            args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in safe), encoding="utf-8")
            network = args.output.with_suffix(".network.json")
            network.write_text(json.dumps({"events": tunnel.events, "tls_connections_attempted": tunnel.forwarded,
                "tls_bytes": tunnel.bytes, "http_model_calls_observable": False}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            metadata.update(public_evidence_sha256=shared.digest(args.output), public_network_sha256=shared.digest(network))
            args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("生产 Grok 工具租约验收" + ("通过" if metadata["lease_passed"] else "未通过"))
    print(f"脱敏证据：{args.output}")
    return 0 if metadata["lease_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("test-binary", "grok", "supervisor", "official-grok-home", "output"):
        parser.add_argument("--" + option, type=Path, required=True)
    parser.add_argument("--max-native-inputs", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=MAX_DEADLINE)
    args = parser.parse_args()
    try:
        isolation.validate_paths(args)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"租约运行器启动失败：{type(error).__name__}\n")


if __name__ == "__main__":
    raise SystemExit(main())
