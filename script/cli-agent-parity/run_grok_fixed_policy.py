#!/usr/bin/env python3
"""固定 Grok 原生路径与生产 prepare 的两次读取审批、冷恢复和项目 hook 验收。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shlex
import subprocess
import sys
import tempfile
import time

import run_grok_native_tool_lease as lease

isolation, official, shared = lease.isolation, lease.official, lease.shared
CURRENT_FIXED_VERSION = "1.0.41"
CURRENT_FIXED_SHA256 = "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d"
SCOPE = "authenticated_fixed_policy_read_approval_and_cold_restore"
TEST_NAME = "ai::cli_agent_runtime::grok::fixed_policy_live_tests::" + SCOPE
MAX_DEADLINE = 540
# launchd 监督器需要正常宿主上下文；这些验收不施加操作系统沙箱，只限制已观测的代理流量。
SANDBOX_SCOPE_FIELDS = {"test_process_sandbox": False, "runtime_os_sandbox": False, "sandbox_scope": "none",
    "launchd_native_sandbox_verified": False, "native_direct_network_blocked": False,
    "native_network_budget_enforced": False, "tls_budget_scope": "observed_configured_proxy_connections"}
PHASE_BOOLS = {"passed", "ready", "same_saved_profile", "same_native_session", "no_replay_before_input",
    "hook_absent_at_ready", "hook_absent_after_shutdown", "final_history_verified", "native_tool_terminal",
    "cleanup_confirmed", "transport_closed", "managed_auth_removed", "allow", "denied_read_not_executed"}
COUNTERS = {"submitted", "accepted", "approvals", "approval_resolved"}
END_BOOLS = {"passed", "system_managed_policies_apply", "native_effective_policy_verified", "filesystem_sandbox_verified",
    "app_restart_verified", "coordinator_verified", "spawn_verified", "full_cli_parity_acceptance_passed"}
START = {"event": "fixed_policy_started", "scope": SCOPE, "max_native_inputs": 2,
    "production_prepare": True, "test_argv_override": False, "credential_values_recorded": False}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def is_hash(value):
    return type(value) is str and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def read_events(path):
    # 本夹具有开始、两阶段及结束四条事件，不能套用单输入租约的两条限制。
    raw = isolation.private_bytes(path, 128 * 1024)
    events = [json.loads(line, object_pairs_hook=lease._pairs,
        parse_constant=lambda value: (_ for _ in ()).throw(ValueError("非有限 JSON")))
        for line in raw.decode("utf-8").splitlines() if line.strip()]
    if len(events) > 4 or any(type(event) is not dict for event in events):
        raise ValueError("固定策略证据超出四条事件合同")
    return events


def validate_event(event):
    if type(event) is not dict or event.get("scope") != SCOPE:
        return False
    if event.get("event") == "fixed_policy_started":
        return event == START and type(event["max_native_inputs"]) is int
    if event.get("event") == "fixed_policy_phase":
        return (set(event) == PHASE_BOOLS | COUNTERS | {"event", "scope", "phase", "final_sha256", "native_session_sha256", "native_outcome", "failure_stage"}
            and event["phase"] in {"new_allow", "load_deny"}
            and (event["native_outcome"] is None or type(event["native_outcome"]) is str
                and event["native_outcome"] in {"Completed", "Cancelled", "Failed"})
            and (event["failure_stage"] is None or type(event["failure_stage"]) is str
                and event["failure_stage"] in {"connect", "ready", "submitted", "approval_requested", "approval_resolved", "turn_finished", "verified"})
            and all(type(event[key]) is bool for key in PHASE_BOOLS)
            and all(type(event[key]) is int and 0 <= event[key] <= 16384 for key in COUNTERS)
            and all(event[key] is None or is_hash(event[key]) for key in ("final_sha256", "native_session_sha256")))
    return (event.get("event") == "fixed_policy_finished" and set(event) == END_BOOLS | {"event", "scope"}
        and all(type(event[key]) is bool for key in END_BOOLS))


def observation(code, events, allowed_hash):
    result = {"fixed_policy_passed": False, "cold_native_restore_verified": False,
        "project_hook_suppression_observed": False, "native_effective_policy_verified": False,
        "filesystem_sandbox_verified": False, "app_restart_verified": False,
        "coordinator_verified": False, "spawn_verified": False, "full_cli_parity_acceptance_passed": False}
    if (type(code) is not int or code != 0 or len(events) != 4 or not is_hash(allowed_hash)
            or not all(validate_event(event) for event in events) or events[0] != START):
        return result
    first, second, end = events[1:]
    if first.get("phase") != "new_allow" or second.get("phase") != "load_deny" or end.get("event") != "fixed_policy_finished":
        return result
    passed = (all(first[key] and second[key] for key in PHASE_BOOLS - {"allow", "denied_read_not_executed"})
        and first["allow"] is True and second["allow"] is False
        and first["native_outcome"] == "Completed" and second["native_outcome"] == "Cancelled"
        and first["denied_read_not_executed"] is False and second["denied_read_not_executed"] is True
        and first["failure_stage"] is None and second["failure_stage"] is None
        and all(first[key] == second[key] == 1 for key in COUNTERS)
        and first["final_sha256"] == allowed_hash and is_hash(second["final_sha256"])
        and is_hash(first["native_session_sha256"]) and first["native_session_sha256"] == second["native_session_sha256"]
        and end["passed"] and end["system_managed_policies_apply"]
        and not any(end[key] for key in END_BOOLS - {"passed", "system_managed_policies_apply"}))
    result.update(fixed_policy_passed=passed, cold_native_restore_verified=passed,
        project_hook_suppression_observed=passed)
    return result


def runtime_sandbox(root, source_home, port):
    # 监督器使用随机本机 TCP 控制端口；此测试沙箱明确允许 loopback，不冒充产品沙箱。
    profile = shared.sandbox_profile(root, source_home, port)
    profile += ('(allow network-bind network-inbound (local ip "localhost:*"))'
        '(allow network-outbound (remote ip "localhost:*"))')
    path = root / "runtime.sb"
    path.write_text(profile, encoding="utf-8")
    path.chmod(0o600)
    return path


def setup_project_sentinel(root):
    project = root / "project"
    project.mkdir(parents=True, exist_ok=True, mode=0o700)
    subprocess.run(["/usr/bin/git", "-c", "init.templateDir=", "init", "-q", str(project)],
        env=shared.native_environment(root), capture_output=True, check=True, timeout=10)
    target = project / ".project-hook-ran"
    command = "/usr/bin/touch " + shlex.quote(str(target))
    hooks = {"hooks": {event: [{"hooks": [{"type": "command", "command": command}]}]
        for event in ("SessionStart", "UserPromptSubmit", "SessionEnd")}}
    snapshot = {}
    # 同时放置原生项目 hook 和 Claude 兼容配置；两者都只能写同一个可观察哨兵。
    for relative in (".grok/hooks/managed-policy-canary.json", ".claude/settings.json"):
        path = project / relative
        path.parent.mkdir(parents=True, mode=0o700)
        path.write_text(json.dumps(hooks) + "\n", encoding="utf-8")
        snapshot[relative] = shared.digest(path)
    return snapshot


def project_snapshot_matches(root, snapshot):
    project = root / "project"
    return (not (project / ".project-hook-ran").exists()
        and all((project / path).is_file() and not (project / path).is_symlink()
            and shared.digest(project / path) == expected for path, expected in snapshot.items()))


def sandbox_canary(root, source_home, profile, environment):
    # 只尝试打开来源目录本身，不读取认证正文；哨兵写成功后立即删除。
    source = '''import json,os,socket,subprocess
r={}
s=socket.socket();s.settimeout(1)
try:s.connect(('1.1.1.1',443));r['external_blocked']=False
except OSError as e:r['external_blocked']=e.errno==1
finally:s.close()
a=socket.socket();a.bind(('127.0.0.1',0));a.listen(1);b=socket.socket();b.connect(a.getsockname());c,_=a.accept();r['loopback_control_allowed']=True;c.close();a.close();b.close()
try:fd=os.open(SOURCE,os.O_RDONLY);os.close(fd);r['source_directory_blocked']=False
except OSError as e:r['source_directory_blocked']=e.errno==1
p=subprocess.run(['/usr/bin/touch',TARGET]);r['sentinel_write_allowed']=p.returncode==0 and os.path.isfile(TARGET)
if os.path.exists(TARGET):os.unlink(TARGET)
print(json.dumps(r))
'''.replace("SOURCE", repr(str(source_home))).replace("TARGET", repr(str(root / "project/.project-hook-ran")))
    process = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(profile), sys.executable, "-c", source],
        env=environment, capture_output=True, text=True, timeout=10, check=True)
    result = json.loads(process.stdout)
    expected = {"external_blocked": True, "loopback_control_allowed": True,
        "source_directory_blocked": True, "sentinel_write_allowed": True}
    if result != expected or any(type(value) is not bool for value in result.values()):
        raise ValueError("固定策略测试沙箱边界未通过")
    return result


def validate_paths(args, *, max_native_inputs=2, max_deadline=MAX_DEADLINE,
        expected_sha256=shared.BINARY_SHA256):
    # 沿用官方入口的路径/摘要/凭据元数据规则，只替换本夹具的预算。
    if (type(args.max_native_inputs) is not int or args.max_native_inputs != max_native_inputs
            or type(args.timeout) is not int or not 30 <= args.timeout <= max_deadline):
        raise ValueError("固定策略验收超出调用方的输入或期限预算")
    check = argparse.Namespace(**vars(args))
    check.max_native_inputs, check.timeout = 1, min(args.timeout, lease.MAX_DEADLINE)
    isolation.validate_paths(check, expected_sha256=expected_sha256)
    for name in ("test_binary", "grok", "supervisor", "official_grok_home", "output"):
        setattr(args, name, getattr(check, name))


def cleanup_auth(root):
    root = root.resolve(strict=True)
    # 适配器使用 root/state，协调器使用私有 HOME 的平台数据目录；两者都必须清理。
    paths = {root / "home/.grok/auth.json"}
    for base in (root / "state", root / "home"):
        paths.update(base.rglob("grok-managed/*/grok/auth.json"))
    # 整批预检后再删除；任何符号链接都不能把清理引向来源认证或其他用户文件。
    for path in paths:
        if not path.is_relative_to(root):
            raise ValueError("认证清理路径越出私有工作区")
        current = path
        while current != root:
            if current.is_symlink():
                raise ValueError("认证清理路径含符号链接")
            current = current.parent
    for path in paths:
        path.unlink(missing_ok=True)
    return not any(path.exists() for path in paths)


def boundary_passed(metadata, tunnel, *, connection_budget=lease.MAX_TLS_CONNECTIONS):
    return (type(connection_budget) is int and 1 <= connection_budget <= 64
        and metadata.get("test_exit_code") == 0 and metadata.get("timed_out") is not True
        and all(type(metadata.get(key)) is type(value) and metadata[key] == value
            for key, value in SANDBOX_SCOPE_FIELDS.items())
        and all(metadata.get(key) is True for key in ("tunnels_stopped", "private_auth_copy_removed",
            "original_auth_stat_unchanged", "project_snapshot_unchanged", "binary_unchanged"))
        and tunnel.forwarded <= connection_budget and tunnel.bytes <= lease.MAX_TLS_BYTES
        and any(item == {"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"} for item in tunnel.events)
        and not any(item.get("event") == "official_connect_budget_rejected" for item in tunnel.events))


def run(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    isolation.reserve_artifacts(args.output)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-fixed-policy-", dir="/private/tmp")).resolve()
    root.chmod(0o700)
    for relative in ("home/.grok", "tmp", "state"):
        (root / relative).mkdir(parents=True, mode=0o700)
    (root / ".infinishell-grok-live-probe").write_text(shared.MARKER)
    snapshot = setup_project_sentinel(root)
    allowed = "INFINISHELL_ALLOWED_" + secrets.token_hex(32)
    for name, token in (("allow.txt", allowed), ("deny.txt", "INFINISHELL_DENIED_" + secrets.token_hex(32))):
        path = root / "project" / name
        path.write_text(token + "\n")
        snapshot[name] = shared.digest(path)
    raw = root / "private-evidence.ndjson"
    raw.touch(mode=0o600)
    before_auth = lease.auth_identity(args.official_grok_home)
    binary_hash = shared.digest(args.grok)
    metadata = dict(observation(None, [], sha(allowed.encode())), scope=SCOPE, test_name=TEST_NAME,
        private_workspace=str(root), max_native_inputs=2, max_tls_connections=lease.MAX_TLS_CONNECTIONS,
        max_tls_bytes=lease.MAX_TLS_BYTES, deadline_seconds=args.timeout, http_model_calls_observable=False,
        cost_budget_enforced=False, tls_decrypted=False, system_managed_policies_apply=True,
        native_executable_is_wrapper=False,
        same_commit_verified_by_runner=False, public_credential_values_recorded=False,
        grok_sha256=binary_hash, test_binary_sha256=shared.digest(args.test_binary), supervisor_sha256=shared.digest(args.supervisor))
    metadata.update(SANDBOX_SCOPE_FIELDS)
    events = []
    with isolation.bounded_tunnel(args.timeout) as tunnel:
        try:
            port = tunnel.start()
            official.copy_private_auth(args.official_grok_home, root / "home/.grok")
            environment = official.official_environment(root, port)
            environment.update(INFINISHELL_GROK_LIVE_ROOT=str(root), INFINISHELL_GROK_LIVE_EXECUTABLE=str(args.grok),
                INFINISHELL_GROK_LIVE_ARTIFACT=str(raw), INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(args.supervisor),
                INFINISHELL_GROK_FIXED_VERSION=CURRENT_FIXED_VERSION)
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
            metadata.update(observation(process.returncode, events, sha(allowed.encode())))
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
                metadata["private_auth_copy_removed"] = cleanup_auth(root)
                metadata["original_auth_stat_unchanged"] = lease.auth_identity(args.official_grok_home) == before_auth
                metadata["project_snapshot_unchanged"] = project_snapshot_matches(root, snapshot)
                metadata["binary_unchanged"] = shared.digest(args.grok) == binary_hash
            except (OSError, ValueError) as error:
                metadata["cleanup_error_type"] = type(error).__name__
            metadata["boundary_passed"] = boundary_passed(metadata, tunnel)
            metadata["fixed_policy_passed"] &= metadata["boundary_passed"]
            for key in ("cold_native_restore_verified", "project_hook_suppression_observed"):
                metadata[key] &= metadata["fixed_policy_passed"]
            safe = events if all(validate_event(event) for event in events) else []
            args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in safe), encoding="utf-8")
            network = args.output.with_suffix(".network.json")
            network.write_text(json.dumps({"events": tunnel.events, "tls_connections_attempted": tunnel.forwarded,
                "tls_bytes": tunnel.bytes, "http_model_calls_observable": False,
                "tls_budget_scope": SANDBOX_SCOPE_FIELDS["tls_budget_scope"], "native_network_budget_enforced": False}, ensure_ascii=False, indent=2) + "\n")
            metadata.update(public_evidence_sha256=shared.digest(args.output), public_network_sha256=shared.digest(network))
            args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n")
    print("Grok固定策略根任务验收" + ("通过" if metadata["fixed_policy_passed"] else "未通过"))
    print(f"脱敏证据：{args.output}")
    return 0 if metadata["fixed_policy_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("test-binary", "grok", "supervisor", "official-grok-home", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--max-native-inputs", type=int, default=2)
    parser.add_argument("--timeout", type=int, default=MAX_DEADLINE)
    args = parser.parse_args()
    try:
        validate_paths(args, expected_sha256=CURRENT_FIXED_SHA256)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"固定策略运行器失败：{type(error).__name__}\n")


if __name__ == "__main__":
    raise SystemExit(main())
