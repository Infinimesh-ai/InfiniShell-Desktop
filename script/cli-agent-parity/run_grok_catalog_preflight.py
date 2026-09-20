#!/usr/bin/env python3
"""固定 Grok 正式 SDK 桥目录零输入预检；不提交用户输入或调用业务工具。"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import run_grok_fixed_policy as fixed

lease = fixed.lease
isolation, official, shared = lease.isolation, lease.official, lease.shared
SCOPE = "grok_sdk_catalog_zero_input"
TEST_NAME = "ai::cli_agent_runtime::grok::catalog_preflight_live_tests::" + SCOPE
MAX_DEADLINE = 180
NAMES = {"read_file", "search_tool", "use_tool", "infinishell-local-tasks__inspect_local_tasks",
    "infinishell-local-tasks__run_agents", "infinishell-local-tasks__send_message_to_agent"}
START = {"event": "catalog_preflight_started", "scope": SCOPE, "max_native_inputs": 0,
    "production_prepare": True, "test_argv_override": False}
OBS_BOOLS = {"builtin_seen", "union_seen", "served_names_exact", "pull_verified", "acu_union_seen"}
OBS_COUNTS = {"catalogs", "duplicate_count", "unknown_count", "non_string_count",
    "outbound_request_attempts", "outbound_response_attempts", "guard_rejections"}
END_BOOLS = {"passed", "ready", "transport_closed", "cleanup_confirmed", "managed_auth_removed", "full_cli_parity_acceptance_passed"}


def validate_event(event):
    if type(event) is not dict or event.get("scope") != SCOPE:
        return False
    if event.get("event") == "catalog_preflight_started":
        return (set(event) == set(START) and all(type(event[k]) is type(v) and event[k] == v for k, v in START.items()))
    if event.get("event") == "catalog_observed":
        matches = event.get("name_matches")
        return (set(event) == OBS_BOOLS | OBS_COUNTS | {"event", "scope", "name_matches"}
            and all(type(event[k]) is bool for k in OBS_BOOLS)
            and all(type(event[k]) is int and 0 <= event[k] <= 16384 for k in OBS_COUNTS)
            and type(matches) is dict and set(matches) == NAMES and all(type(v) is bool for v in matches.values()))
    return (event.get("event") == "catalog_preflight_finished"
        and set(event) == END_BOOLS | {"event", "scope", "native_inputs", "business_tool_dispatches"}
        and all(type(event[k]) is bool for k in END_BOOLS)
        and all(type(event[k]) is int and event[k] == 0 for k in ("native_inputs", "business_tool_dispatches")))


def read_events(path):
    raw = isolation.private_bytes(path, 128 * 1024)
    events = [json.loads(line, object_pairs_hook=lease._pairs,
        parse_constant=lambda value: (_ for _ in ()).throw(ValueError("非有限 JSON")))
        for line in raw.decode("utf-8").splitlines() if line.strip()]
    if len(events) > 3 or any(type(event) is not dict for event in events):
        raise ValueError("目录预检证据超过三条固定事件")
    return events


def observation(code, events):
    result = {"catalog_union_passed": False, "full_cli_parity_acceptance_passed": False}
    if (type(code) is not int or code != 0 or len(events) != 3
            or not all(validate_event(event) for event in events) or events[0] != START):
        return result
    observed, end = events[1:]
    if observed.get("event") != "catalog_observed" or end.get("event") != "catalog_preflight_finished":
        return result
    result["catalog_union_passed"] = (observed["union_seen"] and observed["served_names_exact"]
        and (observed["pull_verified"] or observed["acu_union_seen"])
        and all(observed["name_matches"].values()) and 0 < observed["catalogs"] <= 32
        and 0 < observed["outbound_request_attempts"] <= 8 and 2 <= observed["outbound_response_attempts"] <= 8
        and all(observed[k] == 0 for k in ("duplicate_count", "unknown_count", "non_string_count", "guard_rejections"))
        and all(end[k] for k in END_BOOLS - {"full_cli_parity_acceptance_passed"})
        and not end["full_cli_parity_acceptance_passed"])
    return result


def boundary_passed(metadata, tunnel):
    # 无输入握手不要求触达模型代理；只核对实际网络预算和本次所有权清理。
    return (metadata.get("test_exit_code") == 0 and metadata.get("timed_out") is not True
        and all(type(metadata.get(k)) is type(v) and metadata[k] == v for k, v in fixed.SANDBOX_SCOPE_FIELDS.items())
        and all(metadata.get(k) is True for k in ("tunnels_stopped", "private_auth_copy_removed",
            "original_auth_stat_unchanged", "project_snapshot_unchanged", "binary_unchanged"))
        and tunnel.forwarded <= lease.MAX_TLS_CONNECTIONS and tunnel.bytes <= lease.MAX_TLS_BYTES
        and not any(item.get("event") == "official_connect_budget_rejected" for item in tunnel.events))


def run(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    isolation.reserve_artifacts(args.output)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-catalog-preflight-", dir="/private/tmp")).resolve()
    root.chmod(0o700)
    for relative in ("home/.grok", "tmp", "state"):
        (root / relative).mkdir(parents=True, mode=0o700)
    (root / ".infinishell-grok-live-probe").write_text(shared.MARKER, encoding="utf-8")
    snapshot = fixed.setup_project_sentinel(root)
    (root / ".infinishell-grok-catalog-probe").write_text(SCOPE, encoding="utf-8")
    raw = root / "private-evidence.ndjson"
    raw.touch(mode=0o600)
    before_auth = lease.auth_identity(args.official_grok_home)
    binary_hash = shared.digest(args.grok)
    metadata = dict(observation(None, []), scope=SCOPE, test_name=TEST_NAME,
        private_workspace=str(root), max_native_inputs=0, max_tls_connections=lease.MAX_TLS_CONNECTIONS,
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
            official.copy_private_auth(args.official_grok_home, root / "home/.grok")
            environment = official.official_environment(root, port)
            environment.update(INFINISHELL_GROK_LIVE_ROOT=str(root), INFINISHELL_GROK_LIVE_EXECUTABLE=str(args.grok),
                INFINISHELL_GROK_LIVE_ARTIFACT=str(raw), INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(args.supervisor))
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
            metadata["boundary_passed"] = boundary_passed(metadata, tunnel)
            metadata["catalog_union_passed"] &= metadata["boundary_passed"]
            safe = events if all(validate_event(event) for event in events) else []
            args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in safe), encoding="utf-8")
            network = args.output.with_suffix(".network.json")
            network.write_text(json.dumps({"events": tunnel.events, "tls_connections_attempted": tunnel.forwarded,
                "tls_bytes": tunnel.bytes, "http_model_calls_observable": False,
                "tls_budget_scope": fixed.SANDBOX_SCOPE_FIELDS["tls_budget_scope"], "native_network_budget_enforced": False}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            metadata.update(public_evidence_sha256=shared.digest(args.output), public_network_sha256=shared.digest(network))
            args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("Grok SDK 目录零输入预检" + ("通过" if metadata["catalog_union_passed"] else "未通过"))
    print(f"脱敏证据：{args.output}")
    return 0 if metadata["catalog_union_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("test-binary", "grok", "supervisor", "official-grok-home", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--max-native-inputs", type=int, default=0)
    parser.add_argument("--timeout", type=int, default=MAX_DEADLINE)
    args = parser.parse_args()
    try:
        fixed.validate_paths(args, max_native_inputs=0, max_deadline=MAX_DEADLINE)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"目录零输入运行器失败：{type(error).__name__}\n")


if __name__ == "__main__":
    raise SystemExit(main())
