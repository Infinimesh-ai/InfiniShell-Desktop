#!/usr/bin/env python3
"""真实 Grok 固定创建策略父子派发、排队消息、结果回收与冷继续验收。"""

import argparse
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import uuid

import run_grok_fixed_policy as fixed

lease = fixed.lease
isolation, official, shared = lease.isolation, lease.official, lease.shared
SCOPE = "real_grok_fixed_policy_parent_child"
TEST_NAME = "ai::cli_agent_runtime::coordinator::grok_child_live_tests::" + SCOPE
MAX_DEADLINE = 600
CLI_VERSION = "1.0.41"
CLI_VERSION_OUTPUT = "grok 1.0.41 (4220f3b224a6)"
CLI_SHA256 = "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d"
START = {"event": "acceptance_started", "scope": SCOPE, "max_native_inputs": 6,
    "cli_version": CLI_VERSION,
    "production_prepare": True, "test_argv_override": False, "real_gui_verified": False,
    "app_restart_verified": False, "native_effective_policy_verified": False, "filesystem_sandbox_verified": False}
GATE = {"event": "queued_before_read_allow", "scope": SCOPE,
    "sent": True, "native_ack": False, "child_waiting_approval": True}
RESUME = {"event": "cold_resume_verified", "scope": SCOPE,
    "same_native_session": True, "no_new_input": True, "messages_unchanged": True, "app_restart_verified": False}
END = {"event": "acceptance_passed", "scope": SCOPE, "native_inputs": 6, "cleanup_count": 3,
    "native_ack_both_directions": True, "automatic_result_ack": True, "final_result_via_inspect": True,
    "cold_resume_ready": True, "app_restart_verified": False, "real_gui_verified": False,
    "native_effective_policy_verified": False, "filesystem_sandbox_verified": False,
    "full_cli_parity_acceptance_passed": False}
CHAIN = {"event": "chain_verified", "scope": SCOPE, "tasks": 2, "parent_generations": 4,
    "child_generations": 2, "native_inputs": 6, "sdk_tool_calls": 4,
    "native_ack_both_directions": True, "automatic_result_ack": True, "final_result_via_inspect": True,
    "parent_binding_verified": True, "child_creation_policy_verified": True}
CHAIN_HASHES = {"parent_native_sha256", "child_native_sha256", "parent_result_sha256", "child_result_sha256"}


def typed_equal(left, right):
    return (type(left) is dict and set(left) == set(right)
        and all(type(left[key]) is type(value) and left[key] == value for key, value in right.items()))


def valid_cleanup_receipt(event):
    if set(event) != {"event", "scope", "runtime_generation", "receipt"}:
        return False
    receipt = event["receipt"]
    keys = {"version", "runtime_generation", "host_instance_id", "manifest_sha256",
        "journal_sha256", "last_event_sequence", "acknowledged_sequence", "native_process",
        "native_cleanup_sha256", "adapter_task_terminated", "adapter_succeeded", "event_journal_completed"}
    if type(receipt) is not dict or set(receipt) != keys:
        return False
    try:
        if (str(uuid.UUID(event["runtime_generation"])) != event["runtime_generation"]
                or str(uuid.UUID(receipt["host_instance_id"])) != receipt["host_instance_id"]):
            return False
    except (TypeError, ValueError, AttributeError):
        return False
    return (type(receipt["version"]) is int and receipt["version"] == 2
        and receipt["runtime_generation"] == event["runtime_generation"]
        and receipt["native_process"] == "exited"
        and all(type(receipt[key]) is bool for key in ("adapter_task_terminated", "adapter_succeeded", "event_journal_completed"))
        and all(fixed.is_hash(receipt[key]) for key in ("manifest_sha256", "journal_sha256", "native_cleanup_sha256"))
        and type(receipt["last_event_sequence"]) is int
        and type(receipt["acknowledged_sequence"]) is int
        and 0 <= receipt["acknowledged_sequence"] <= receipt["last_event_sequence"])


def validate_event(event):
    if type(event) is not dict or event.get("scope") != SCOPE:
        return False
    if any(typed_equal(event, expected) for expected in (START, GATE, RESUME, END)):
        return True
    if event.get("event") == "runtime_host_cleanup_receipt":
        return valid_cleanup_receipt(event)
    if event.get("event") == "chain_verified":
        return (set(event) == set(CHAIN) | CHAIN_HASHES
            and typed_equal({key: event[key] for key in CHAIN}, CHAIN)
            and all(fixed.is_hash(event[key]) for key in CHAIN_HASHES))
    if event.get("event") == "cleanup_verified":
        return (set(event) == {"event", "scope", "runtime_sha256", "cleanup_confirmed"}
            and fixed.is_hash(event["runtime_sha256"]) and event["cleanup_confirmed"] is True)
    return (event.get("event") == "acceptance_failed"
        and set(event) == {"event", "scope", "reason_bytes", "reason_sha256"}
        and type(event["reason_bytes"]) is int and 0 < event["reason_bytes"] <= 1048576
        and fixed.is_hash(event["reason_sha256"]))


def read_events(path):
    # 八条业务证据与三条真实宿主退出回执必须成对保留。
    raw = isolation.private_bytes(path, 128 * 1024)
    events = [json.loads(line, object_pairs_hook=lease._pairs,
        parse_constant=lambda value: (_ for _ in ()).throw(ValueError("非有限 JSON")))
        for line in raw.decode("utf-8").splitlines() if line.strip()]
    if len(events) > 11 or any(type(event) is not dict for event in events):
        raise ValueError("父子协调器证据超过十一条固定事件")
    return events


def observation(code, events):
    result = {"parent_child_passed": False, "cold_resume_ready_verified": False,
        "native_ack_both_directions": False, "automatic_result_ack_verified": False,
        "final_result_via_inspect_verified": False, "native_effective_policy_verified": False,
        "filesystem_sandbox_verified": False, "app_restart_verified": False,
        "real_gui_verified": False, "full_cli_parity_acceptance_passed": False}
    if (type(code) is not int or code != 0 or len(events) != 11
            or not all(validate_event(event) for event in events)):
        return result
    if ([event["event"] for event in events] != ["acceptance_started", "queued_before_read_allow", "chain_verified",
            "runtime_host_cleanup_receipt", "cleanup_verified", "runtime_host_cleanup_receipt", "cleanup_verified",
            "cold_resume_verified", "runtime_host_cleanup_receipt", "cleanup_verified", "acceptance_passed"]):
        return result
    chain = events[2]
    if (chain["parent_native_sha256"] == chain["child_native_sha256"]
            or not all(events[index]["receipt"][key] is True
                for index in (3, 5, 8)
                for key in ("adapter_task_terminated", "adapter_succeeded", "event_journal_completed"))
            or len({events[index]["runtime_generation"] for index in (3, 5, 8)}) != 3
            or any(events[index + 1]["runtime_sha256"] != fixed.sha(events[index]["runtime_generation"].encode())
                for index in (3, 5, 8))):
        return result
    result.update(parent_child_passed=True, cold_resume_ready_verified=True,
        native_ack_both_directions=True, automatic_result_ack_verified=True, final_result_via_inspect_verified=True)
    return result


def run(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    isolation.reserve_artifacts(args.output)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-child-coordinator-", dir="/private/tmp")).resolve()
    root.chmod(0o700)
    for relative in ("home/.grok", "tmp", "state"):
        (root / relative).mkdir(parents=True, mode=0o700)
    (root / ".infinishell-grok-live-probe").write_text(shared.MARKER)
    snapshot = fixed.setup_project_sentinel(root)
    (root / ".infinishell-grok-child-coordinator-probe").write_text(SCOPE)
    read_file = root / "project/child-read.txt"
    read_file.write_text("INFINISHELL_CHILD_READ_" + secrets.token_hex(32) + "\n")
    snapshot["child-read.txt"] = shared.digest(read_file)
    raw = root / "private-evidence.ndjson"
    raw.touch(mode=0o600)
    before_auth = lease.auth_identity(args.official_grok_home)
    binary_hash = shared.digest(args.grok)
    metadata = dict(observation(None, []), scope=SCOPE, test_name=TEST_NAME,
        cli_version=CLI_VERSION, cli_version_verified=False,
        private_workspace=str(root), max_native_inputs=6, max_tls_connections=lease.MAX_TLS_CONNECTIONS,
        max_tls_bytes=lease.MAX_TLS_BYTES, deadline_seconds=args.timeout, http_model_calls_observable=False,
        cost_budget_enforced=False, tls_decrypted=False, system_managed_policies_apply=True,
        native_executable_is_wrapper=False,
        same_commit_verified_by_runner=False, public_credential_values_recorded=False,
        grok_sha256=binary_hash, test_binary_sha256=shared.digest(args.test_binary), supervisor_sha256=shared.digest(args.supervisor))
    metadata.update(fixed.SANDBOX_SCOPE_FIELDS)
    events = []
    with isolation.bounded_tunnel(args.timeout) as tunnel:
        try:
            port = tunnel.start()
            environment = official.official_environment(root, port)
            version = subprocess.run([str(args.grok), "--version"], cwd=root / "project",
                env=environment, capture_output=True, text=True, check=True, timeout=30)
            if version.stdout.strip() != CLI_VERSION_OUTPUT or tunnel.forwarded != 0:
                raise ValueError("固定 Grok 版本探测不匹配")
            metadata["cli_version_verified"] = True
            official.copy_private_auth(args.official_grok_home, root / "home/.grok")
            environment.update(INFINISHELL_GROK_LIVE_ROOT=str(root), INFINISHELL_GROK_LIVE_EXECUTABLE=str(args.grok),
                INFINISHELL_GROK_LIVE_ARTIFACT=str(raw), INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(args.supervisor),
                INFINISHELL_GROK_LIVE_VERSION=CLI_VERSION)
            remaining = tunnel.deadline - time.monotonic()
            if remaining <= 0 or tunnel.forwarded != 0:
                raise ValueError("输入之前预算已耗尽")
            # 外层 sandbox-exec 会拦截监督器的短 socket 与 launchctl bootstrap，导致 CLI 尚未启动即失败。
            command = [str(args.test_binary), TEST_NAME,
                "--exact", "--ignored", "--nocapture", "--test-threads=1"]
            with os.fdopen(os.open(root / "private-test-output.txt", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600), "wb") as log:
                process = subprocess.Popen(command, cwd=Path(__file__).resolve().parents[2], env=environment, stdout=log, stderr=subprocess.STDOUT)
                try:
                    process.wait(timeout=remaining)
                except subprocess.TimeoutExpired:
                    metadata["timed_out"] = True
                    process.kill(); process.wait(timeout=20)
                except BaseException:
                    process.kill(); process.wait(timeout=20)
                    raise
            metadata["test_exit_code"] = process.returncode
            events = read_events(raw)
            metadata.update(observation(process.returncode, events))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            metadata["runner_error_type"] = type(error).__name__
        finally:
            try:
                metadata["tunnels_stopped"] = tunnel.close()
            except Exception as error:
                # 关闭错误不得跳过认证清理；仅公开异常类型，整次验收继续保持失败。
                metadata["tunnels_stopped"] = False
                metadata["tunnel_cleanup_error_type"] = type(error).__name__
            try:
                metadata["private_auth_copy_removed"] = fixed.cleanup_auth(root)
                metadata["original_auth_stat_unchanged"] = lease.auth_identity(args.official_grok_home) == before_auth
                metadata["project_snapshot_unchanged"] = fixed.project_snapshot_matches(root, snapshot)
                metadata["binary_unchanged"] = shared.digest(args.grok) == binary_hash
            except (OSError, ValueError) as error:
                metadata["cleanup_error_type"] = type(error).__name__
            metadata["boundary_passed"] = fixed.boundary_passed(metadata, tunnel)
            metadata["parent_child_passed"] &= metadata["boundary_passed"] and metadata["cli_version_verified"]
            for key in ("cold_resume_ready_verified", "native_ack_both_directions", "automatic_result_ack_verified", "final_result_via_inspect_verified"):
                metadata[key] &= metadata["parent_child_passed"]
            safe = events if all(validate_event(event) for event in events) else []
            args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in safe), encoding="utf-8")
            network = args.output.with_suffix(".network.json")
            network.write_text(json.dumps({"events": tunnel.events, "tls_connections_attempted": tunnel.forwarded,
                "tls_bytes": tunnel.bytes, "http_model_calls_observable": False,
                "tls_budget_scope": fixed.SANDBOX_SCOPE_FIELDS["tls_budget_scope"], "native_network_budget_enforced": False}, ensure_ascii=False, indent=2) + "\n")
            metadata.update(public_evidence_sha256=shared.digest(args.output), public_network_sha256=shared.digest(network))
            args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n")
    print("Grok固定策略父子协调器验收" + ("通过" if metadata["parent_child_passed"] else "未通过"))
    print(f"脱敏证据：{args.output}")
    return 0 if metadata["parent_child_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("test-binary", "grok", "supervisor", "official-grok-home", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--max-native-inputs", type=int, default=6)
    parser.add_argument("--timeout", type=int, default=MAX_DEADLINE)
    args = parser.parse_args()
    try:
        fixed.validate_paths(args, max_native_inputs=6, max_deadline=MAX_DEADLINE,
            expected_sha256=CLI_SHA256)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"父子协调器运行器失败：{type(error).__name__}\n")


if __name__ == "__main__":
    raise SystemExit(main())
