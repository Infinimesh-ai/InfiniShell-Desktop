#!/usr/bin/env python3
"""固定文件工具策略：六次输入，逐次审批、文件字节、原生历史及同 ID 冷恢复验收。"""
import argparse
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time

import run_grok_fixed_policy as fixed

lease, isolation, official, shared = fixed.lease, fixed.isolation, fixed.official, fixed.shared
SCOPE = "authenticated_files_policy_six_input_lifecycle"
TEST_NAME = "ai::cli_agent_runtime::grok::files_policy_live_tests::" + SCOPE
MAX_DEADLINE = 900
PHASES = ("write_allow", "write_deny", "edit_allow", "edit_deny", "pending_cancel", "cold_read")
PHASE_BOOLS = {"passed", "ready", "same_saved_profile", "same_native_session", "no_replay_before_input",
    "hook_absent_at_ready", "hook_absent_after_shutdown", "final_history_verified", "native_tool_terminal",
    "native_catalog_verified", "file_bytes_match", "cleanup_confirmed", "transport_closed", "managed_auth_removed"}
COUNTERS = {"submitted", "accepted", "approvals", "approval_resolved", "approval_cancelled"}
HASH_FIELDS = {"file_before_sha256", "file_after_sha256", "final_sha256", "native_session_sha256"}
END_BOOLS = fixed.END_BOOLS | {"shell_build_tools_verified"}
START = {"event": "files_policy_started", "scope": SCOPE, "max_native_inputs": 6,
    "production_prepare": True, "test_argv_override": False, "credential_values_recorded": False}
STAGES = {"connect", "ready", "submitted", "approval_requested", "approval_resolved", "turn_finished", "verified"}


def read_events(path):
    raw = isolation.private_bytes(path, 128 * 1024)
    events = [json.loads(line, object_pairs_hook=lease._pairs,
        parse_constant=lambda value: (_ for _ in ()).throw(ValueError("非有限 JSON")))
        for line in raw.decode("utf-8").splitlines() if line.strip()]
    if len(events) > 8 or any(type(event) is not dict for event in events):
        raise ValueError("文件工具证据超出八条事件合同")
    return events


APPROVAL_FLAGS = {"expected_tool_present", "request_identity_matches", "session_id_matches", "name_matches",
    "namespace_matches", "version_matches", "kind_matches", "read_only_matches", "variant_matches", "parameters_match"}
APPROVAL_TYPES = {"raw_input_type", "name_type", "namespace_type", "version_type", "kind_type"}
PARAMETER_KEYS = {"variant", "target_file", "file_path", "content", "old_string", "new_string", "replace_all", "offset", "limit", "pages", "format"}
JSON_TYPES = {"absent", "null", "boolean", "number", "string", "array", "object"}


def approval_summary(value):
    if value is None:
        return True
    return (type(value) is dict and set(value) == APPROVAL_FLAGS | APPROVAL_TYPES
        | {"missing_parameter_count", "extra_parameter_count", "parameter_types"}
        and all(type(value[key]) is bool for key in APPROVAL_FLAGS)
        and all(type(value[key]) is str and value[key] in JSON_TYPES for key in APPROVAL_TYPES)
        and all(type(value[key]) is int and 0 <= value[key] <= 2 ** 32
            for key in ("missing_parameter_count", "extra_parameter_count"))
        and type(value["parameter_types"]) is dict and set(value["parameter_types"]) == PARAMETER_KEYS
        and all(type(kind) is str and kind in JSON_TYPES for kind in value["parameter_types"].values()))


def validate_event(event):
    if type(event) is not dict or event.get("scope") != SCOPE:
        return False
    if event.get("event") == "files_policy_started":
        return event == START and type(event["max_native_inputs"]) is int
    if event.get("event") == "files_policy_phase":
        return (set(event) == PHASE_BOOLS | COUNTERS | HASH_FIELDS | {"event", "scope", "phase", "native_outcome", "failure_stage", "approval_diagnostic"}
            and approval_summary(event["approval_diagnostic"])
            and type(event["phase"]) is str and event["phase"] in PHASES
            and all(type(event[key]) is bool for key in PHASE_BOOLS)
            and all(type(event[key]) is int and 0 <= event[key] <= 16384 for key in COUNTERS)
            and all(event[key] is None or fixed.is_hash(event[key]) for key in HASH_FIELDS)
            and (event["native_outcome"] is None or type(event["native_outcome"]) is str and event["native_outcome"] in {"Completed", "Cancelled", "Failed"})
            and (event["failure_stage"] is None or type(event["failure_stage"]) is str and event["failure_stage"] in STAGES))
    return (event.get("event") == "files_policy_finished" and set(event) == END_BOOLS | {"event", "scope"}
        and all(type(event[key]) is bool for key in END_BOOLS))


def fixture_cases(root):
    def token(label):
        return "INFINISHELL_" + label + "_" + secrets.token_hex(32) + "\n"
    cases = {}
    for phase, name in zip(PHASES[:5], ("write-allow.txt", "write-deny.txt", "edit-allow.txt", "edit-deny.txt", "cancel.txt")):
        before = token(phase.upper() + "_BEFORE") if phase in {"edit_allow", "edit_deny"} else None
        proposed = token(phase.upper() + "_AFTER")
        after = proposed if phase in {"write_allow", "edit_allow"} else before
        cases[phase] = {"name": name, "before": before, "proposed": proposed, "after": after}
        if before is not None:
            (root / "project" / name).write_bytes(before.encode("utf-8"))
    row = cases["write_allow"]
    cases["cold_read"] = {"name": row["name"], "before": row["after"], "proposed": row["after"], "after": row["after"]}
    path = root / "file-cases.json"
    with os.fdopen(os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY | getattr(os, "O_NOFOLLOW", 0), 0o600), "w") as output:
        json.dump(cases, output, ensure_ascii=False); output.write("\n")
    return cases


def expected_receipts(cases):
    return {phase: {"file_before_sha256": fixed.sha(cases[phase]["before"].encode()) if cases[phase]["before"] is not None else None,
        "file_after_sha256": fixed.sha(cases[phase]["after"].encode()) if cases[phase]["after"] is not None else None,
        # 取消前言由模型实际产生；仅要求原生回放验证后的真实摘要，不预造空结果。
        "final_sha256": fixed.sha((cases[phase]["after"].strip() if phase == "cold_read" else phase.upper()).encode())
            if phase in {"write_allow", "edit_allow", "cold_read"} else None}
        for phase in PHASES}


def files_match(root, cases):
    for phase, row in cases.items():
        if phase == "cold_read":
            continue
        path = root / "project" / row["name"]
        if path.is_symlink():
            return False
        if row["after"] is None:
            if path.exists():
                return False
        elif not path.is_file() or path.read_bytes() != row["after"].encode():
            return False
    return True


def observation(code, events, receipts):
    result = {"files_policy_passed": False, "cold_native_restore_verified": False,
        "project_hook_suppression_observed": False, "native_effective_policy_verified": False,
        "filesystem_sandbox_verified": False, "app_restart_verified": False, "coordinator_verified": False,
        "spawn_verified": False, "shell_build_tools_verified": False, "full_cli_parity_acceptance_passed": False}
    if (type(code) is not int or code != 0 or len(events) != 8 or not all(validate_event(event) for event in events)
            or events[0] != START or [event.get("phase") for event in events[1:7]] != list(PHASES)
            or events[-1].get("event") != "files_policy_finished" or set(receipts) != set(PHASES)):
        return result
    session_hash = events[1]["native_session_sha256"]
    for event in events[1:7]:
        phase = event["phase"]
        tools = 2 if phase in {"edit_allow", "edit_deny"} else 1
        cancelled = 1 if phase == "pending_cancel" else 0
        outcome = "Completed" if phase in {"write_allow", "edit_allow", "cold_read"} else "Cancelled"
        if (not all(event[key] for key in PHASE_BOOLS) or event["submitted"] != 1 or event["accepted"] != 1
                or event["approvals"] != tools or event["approval_resolved"] != tools - cancelled
                or event["approval_cancelled"] != cancelled or event["native_outcome"] != outcome or event["failure_stage"] is not None
                or event["approval_diagnostic"] is not None
                or not fixed.is_hash(session_hash) or event["native_session_sha256"] != session_hash
                or set(receipts[phase]) != {"file_before_sha256", "file_after_sha256", "final_sha256"}
                or any(
                    (value is not None or not fixed.is_hash(event[key]))
                        if key == "final_sha256" and outcome == "Cancelled"
                        else ((value is not None and not fixed.is_hash(value)) or event[key] != value
                            or (key == "final_sha256" and value is None))
                    for key, value in receipts[phase].items())):
            return result
    end = events[-1]
    passed = (end["passed"] and end["system_managed_policies_apply"]
        and not any(end[key] for key in END_BOOLS - {"passed", "system_managed_policies_apply"}))
    result.update(files_policy_passed=passed, cold_native_restore_verified=passed, project_hook_suppression_observed=passed)
    return result


def run(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    isolation.reserve_artifacts(args.output)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-files-policy-", dir="/private/tmp")).resolve()
    root.chmod(0o700)
    for relative in ("home/.grok", "tmp", "state"):
        (root / relative).mkdir(parents=True, mode=0o700)
    (root / ".infinishell-grok-live-probe").write_text(shared.MARKER)
    snapshot = fixed.setup_project_sentinel(root)
    cases = fixture_cases(root)
    receipts = expected_receipts(cases)
    contract_hash = shared.digest(root / "file-cases.json")
    raw = root / "private-evidence.ndjson"; raw.touch(mode=0o600)
    before_auth = lease.auth_identity(args.official_grok_home)
    binary_hash = shared.digest(args.grok)
    metadata = dict(observation(None, [], receipts), scope=SCOPE, test_name=TEST_NAME, private_workspace=str(root),
        max_native_inputs=6, max_tls_connections=lease.MAX_TLS_CONNECTIONS, max_tls_bytes=lease.MAX_TLS_BYTES,
        deadline_seconds=args.timeout, http_model_calls_observable=False, cost_budget_enforced=False, tls_decrypted=False,
        system_managed_policies_apply=True, native_executable_is_wrapper=False, same_commit_verified_by_runner=False,
        public_credential_values_recorded=False, grok_sha256=binary_hash,
        test_binary_sha256=shared.digest(args.test_binary), supervisor_sha256=shared.digest(args.supervisor))
    metadata.update(fixed.SANDBOX_SCOPE_FIELDS)
    events = []
    with isolation.bounded_tunnel(args.timeout) as tunnel:
        try:
            port = tunnel.start()
            official.copy_private_auth(args.official_grok_home, root / "home/.grok")
            environment = official.official_environment(root, port)
            environment.update(INFINISHELL_GROK_LIVE_ROOT=str(root), INFINISHELL_GROK_LIVE_EXECUTABLE=str(args.grok),
                INFINISHELL_GROK_LIVE_ARTIFACT=str(raw), INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(args.supervisor),
                INFINISHELL_GROK_FILES_INPUT_BUDGET="6")
            remaining = tunnel.deadline - time.monotonic()
            if remaining <= 0 or tunnel.forwarded != 0:
                raise ValueError("输入之前预算已耗尽")
            command = [str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
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
            metadata.update(observation(process.returncode, events, receipts))
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
                metadata["fixture_contract_unchanged"] = shared.digest(root / "file-cases.json") == contract_hash
                metadata["expected_file_bytes_verified"] = files_match(root, cases)
                metadata["binary_unchanged"] = shared.digest(args.grok) == binary_hash
            except (OSError, ValueError) as error:
                metadata["cleanup_error_type"] = type(error).__name__
            metadata["boundary_passed"] = (fixed.boundary_passed(metadata, tunnel)
                and metadata.get("fixture_contract_unchanged") is True and metadata.get("expected_file_bytes_verified") is True)
            metadata["files_policy_passed"] &= metadata["boundary_passed"]
            for key in ("cold_native_restore_verified", "project_hook_suppression_observed"):
                metadata[key] &= metadata["files_policy_passed"]
            safe = events if all(validate_event(event) for event in events) else []
            args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in safe), encoding="utf-8")
            network = args.output.with_suffix(".network.json")
            network.write_text(json.dumps({"events":tunnel.events,"tls_connections_attempted":tunnel.forwarded,"tls_bytes":tunnel.bytes,
                "http_model_calls_observable":False,"tls_budget_scope":fixed.SANDBOX_SCOPE_FIELDS["tls_budget_scope"],"native_network_budget_enforced":False},ensure_ascii=False,indent=2)+"\n")
            metadata.update(public_evidence_sha256=shared.digest(args.output),public_network_sha256=shared.digest(network))
            args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata,ensure_ascii=False,indent=2)+"\n")
    print("Grok 固定文件工具验收" + ("通过" if metadata["files_policy_passed"] else "未通过"))
    print(f"脱敏证据：{args.output}")
    return 0 if metadata["files_policy_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("test-binary", "grok", "supervisor", "official-grok-home", "output"):
        parser.add_argument("--"+name,type=Path,required=True)
    parser.add_argument("--max-native-inputs",type=int,default=6)
    parser.add_argument("--timeout",type=int,default=MAX_DEADLINE)
    args = parser.parse_args()
    try:
        fixed.validate_paths(args,max_native_inputs=6,max_deadline=MAX_DEADLINE)
        return run(args)
    except (OSError,ValueError,subprocess.SubprocessError) as error:
        parser.exit(2,f"文件工具运行器失败：{type(error).__name__}\n")


if __name__ == "__main__":
    raise SystemExit(main())
