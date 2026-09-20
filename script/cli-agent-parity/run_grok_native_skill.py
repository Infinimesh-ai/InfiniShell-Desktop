#!/usr/bin/env python3
"""隔离 profile 的 ACP slash 技能黑盒对照；每次一个输入，不打开产品 selected_skills。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time

import run_grok_native_tool_lease as lease

isolation, official, shared = lease.isolation, lease.official, lease.shared
SCOPE = "authenticated_native_skill_black_box"
TEST_NAME = "ai::cli_agent_runtime::grok::native_skill_live_tests::" + SCOPE
MAX_DEADLINE = lease.MAX_DEADLINE
PROFILE_PATH = Path("specs/cli-agent-parity/fixtures/grok-native-skill-profile.md")
SKILL_PATH = Path("project/.grok/skills/infinishell-native-skill/SKILL.md")
PROMPT = "/infinishell-native-skill"
BOOL_KEYS = {"passed", "production_connect_path", "test_argv_override", "product_selected_skills_verified",
    "final_history_verified", "exact_skill_read_only_verified", "native_skill_expansion_verified", "hidden_control_verified", "sentinel_matched",
    "transport_closed", "cleanup_confirmed", "skill_snapshot_unchanged", "full_cli_parity_acceptance_passed"}
COUNTERS = {"submitted_input_count", "accepted_input_count", "approval_count", "native_tool_event_count", "native_skill_read_count"}


def skill_document(case, sentinel):
    if case not in {"visible", "hidden"} or not sentinel.startswith("INFINISHELL_SKILL_"):
        raise ValueError("技能夹具不合法")
    return ("---\nname: infinishell-native-skill\ndescription: 仅手动触发的技能验收。\n"
        f"user-invocable: {'true' if case == 'visible' else 'false'}\ndisable-model-invocation: true\n---\n"
        "已经读到本技能后不要继续调用工具、读取其它文件、搜索、执行命令或修改文件。\n最终只输出下一行，不解释：\n" + sentinel + "\n")


def started(case):
    return {"event": "skill_started", "scope": SCOPE, "case": case, "max_native_inputs": 1,
        "production_connect_path": True, "test_argv_override": True, "product_selected_skills_verified": False,
        "secret_in_submitted_prompt": False, "credential_values_recorded": False}


def is_hash(value):
    return type(value) is str and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


CATALOG_BOOL_KEYS = {"before_first_submit", "metadata_present", "path_present", "path_absolute",
    "path_matches_selected", "bare_name_matches", "qualified_name_matches"}
CATALOG_COUNTS = {"command_count", "selected_name_count"}
CATALOG_SCOPES = {"local", "repo", "user", "server", "bundled", "plugin", "other", "absent"}


def read_events(path):
    # 技能证据最多含开始、目录投影和结束三条；租约读取器仍保持原来的两条上限。
    raw = isolation.private_bytes(path, 128 * 1024)
    events = [json.loads(line, object_pairs_hook=lease._pairs,
        parse_constant=lambda value: (_ for _ in ()).throw(ValueError("非有限 JSON")))
        for line in raw.decode("utf-8").splitlines() if line.strip()]
    if len(events) > 3 or any(type(event) is not dict for event in events):
        raise ValueError("技能证据超出固定事件合同")
    return events


def validate_catalog_snapshot(row):
    return (type(row) is dict
        and set(row) == CATALOG_BOOL_KEYS | CATALOG_COUNTS | {"selected_name", "metadata_scope"}
        and row["selected_name"] == "infinishell-native-skill"
        and type(row["metadata_scope"]) is str and row["metadata_scope"] in CATALOG_SCOPES
        and all(type(row[key]) is bool for key in CATALOG_BOOL_KEYS)
        and all(type(row[key]) is int and 0 <= row[key] <= 16384 for key in CATALOG_COUNTS)
        and row["selected_name_count"] <= row["command_count"]
        and (not row["path_matches_selected"] or (row["path_present"] and row["path_absolute"]
            and row["metadata_present"] and row["selected_name_count"] == 1)))


def validate_event(event):
    if type(event) is not dict or event.get("case") not in {"visible", "hidden"}:
        return False
    if event.get("event") == "skill_started":
        return event == started(event["case"]) and type(event.get("max_native_inputs")) is int
    if event.get("event") == "skill_catalog_observed":
        return (set(event) == {"event", "scope", "case", "snapshots", "overflow", "unassociated_snapshot_count"}
            and event["scope"] == SCOPE and type(event["overflow"]) is bool
            and type(event["unassociated_snapshot_count"]) is int
            and 0 <= event["unassociated_snapshot_count"] <= 64
            and type(event["snapshots"]) is list and len(event["snapshots"]) <= 64
            and len(event["snapshots"]) + event["unassociated_snapshot_count"] <= 64
            and all(validate_catalog_snapshot(row) for row in event["snapshots"]))
    return (set(event) == BOOL_KEYS | COUNTERS | {"event", "scope", "case", "sentinel_sha256", "final_response_sha256"}
        and event["event"] == "skill_finished" and event["scope"] == SCOPE
        and all(type(event[key]) is bool for key in BOOL_KEYS)
        and all(type(event[key]) is int and 0 <= event[key] <= 16384 for key in COUNTERS)
        and is_hash(event["sentinel_sha256"])
        and (event["final_response_sha256"] is None or is_hash(event["final_response_sha256"])))


def observation(exit_code, events, case, expected_sha256):
    result = {"case_passed": False, "native_skill_expansion_verified": False, "hidden_control_verified": False,
        "product_selected_skills_verified": False, "full_cli_parity_acceptance_passed": False,
        "native_catalog_observed": False, "native_catalog_selected_path_verified": False,
        "native_catalog_selected_path_seen_before_input": False}
    if (len(events) not in (2, 3) or not all(validate_event(event) for event in events) or events[0] != started(case)
            or events[-1]["event"] != "skill_finished" or any(event["case"] != case for event in events)
            or not is_hash(expected_sha256)
            or (len(events) == 3 and events[1]["event"] != "skill_catalog_observed")):
        return result
    end = events[-1]
    passed = (type(exit_code) is int and exit_code == 0 and end["passed"]
        and all(end[key] for key in ("production_connect_path", "test_argv_override", "final_history_verified", "exact_skill_read_only_verified",
            "transport_closed", "cleanup_confirmed", "skill_snapshot_unchanged"))
        and not end["product_selected_skills_verified"] and not end["full_cli_parity_acceptance_passed"]
        and end["submitted_input_count"] == end["accepted_input_count"] == 1
        and end["native_skill_read_count"] in (0, 1)
        and end["approval_count"] <= end["native_skill_read_count"]
        and ((end["native_tool_event_count"] == 0) is (end["native_skill_read_count"] == 0))
        and (case == "visible" or end["native_skill_read_count"] == 0)
        and end["sentinel_sha256"] == expected_sha256 and is_hash(end["final_response_sha256"])
        and end["sentinel_matched"] is (case == "visible")
        and end["native_skill_expansion_verified"] is (case == "visible")
        and end["hidden_control_verified"] is (case == "hidden")
        and ((end["final_response_sha256"] == expected_sha256) is (case == "visible")))
    result.update(case_passed=passed, native_skill_expansion_verified=passed and case == "visible",
        hidden_control_verified=passed and case == "hidden")
    if len(events) == 3:
        catalog = events[1]
        rows = catalog["snapshots"]
        before = [row for row in rows if row["before_first_submit"]]
        result.update(native_catalog_observed=bool(rows),
            native_catalog_selected_path_verified=passed and not catalog["overflow"]
                and bool(rows) and rows[-1]["path_matches_selected"],
            native_catalog_selected_path_seen_before_input=passed and not catalog["overflow"]
                and bool(before) and before[-1]["path_matches_selected"])
    return result


def prepare_native(root, native, source_home, port, profile):
    wrapper, settings = isolation.prepare_probe_native(root, native, source_home, port)
    code = wrapper.read_text(encoding="utf-8")
    old = "elif args==['agent','--no-leader','stdio']:kind='direct_agent';endpoint=None\n"
    if code.count(old) != 1:
        raise ValueError("隔离包装器入口改变")
    replacement = (f"elif args==['agent','--no-leader','--agent-profile',{str(profile)!r},'stdio']:\n"
        f" if hashlib.sha256(Path({str(profile)!r}).read_bytes()).hexdigest()!={shared.digest(profile)!r}:raise SystemExit(95)\n"
        " kind='native_skill';endpoint=None\n"
        " if root.resolve()!=root or any((root/part).resolve(strict=True)!=root/part for part in ('home','home/.grok','project')):raise SystemExit(96)\n"
        " if Path.cwd()!=root/'project' or Path(os.environ.get('HOME',''))!=root/'home' or Path(os.environ.get('GROK_HOME',''))!=root/'home/.grok':raise SystemExit(96)\n")
    code = code.replace(old, replacement, 1)
    # 原生目录信任只授予这次合成项目；显式记录验收包装器添加的参数。
    audit = "'arguments_unchanged':True,"
    launch = "str(native),*args]"
    if code.count(audit) != 1 or code.count(launch) != 1:
        raise ValueError("隔离包装器审计或执行入口改变")
    code = code.replace(audit, "'arguments_unchanged':kind!='native_skill','synthetic_project_trust_requested':kind=='native_skill',", 1)
    code = code.replace(launch, "str(native),*(['--trust',*args] if kind=='native_skill' else args)]", 1)
    compile(code, str(wrapper), "exec")
    wrapper.write_text(code, encoding="utf-8")
    return wrapper, settings


def project_unchanged(root, before):
    expected = {str(path) for path in (SKILL_PATH.relative_to("project"), Path(".grok"),
        Path(".grok/skills"), Path(".grok/skills/infinishell-native-skill"))}
    entries = list((root / "project").rglob("*"))
    return ({str(path.relative_to(root / "project")) for path in entries} == expected
        and all(not path.is_symlink() for path in entries) and (root / SKILL_PATH).read_bytes() == before)


def boundary_passed(metadata, launches, tunnel):
    return (metadata.get("test_exit_code") == 0 and metadata.get("timed_out") is not True
        and all(metadata.get(key) is True for key in ("tunnels_stopped", "private_auth_copy_removed",
            "original_auth_stat_unchanged", "profile_snapshot_unchanged", "project_snapshot_unchanged"))
        and metadata.get("private_settings_audit", {}).get("settings_scope_verified") is True
        and metadata.get("synthetic_project_trust_requested") is True
        and metadata.get("project_trust_scope") == "synthetic_project_only"
        and metadata.get("project_trust_flag") == "--trust"
        and metadata.get("trust_store_scope") == "private_home_only"
        and len(launches) == 3
        and all(item.get("arguments_unchanged") is (item.get("kind") != "native_skill")
            and item.get("synthetic_project_trust_requested") is (item.get("kind") == "native_skill")
            for item in launches)
        and sorted(item.get("kind", "") for item in launches) == ["native_skill", "version", "version"]
        and tunnel.forwarded <= lease.MAX_TLS_CONNECTIONS and tunnel.bytes <= lease.MAX_TLS_BYTES
        and any(item == {"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"} for item in tunnel.events)
        and not any(item.get("event") == "official_connect_budget_rejected" for item in tunnel.events))


def run(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    isolation.reserve_artifacts(args.output)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-native-skill-", dir="/private/tmp")).resolve()
    root.chmod(0o700)
    for relative in ("home/.grok", "home/.claude", "home/.codex", str(SKILL_PATH.parent), "tmp", "state", "policy"):
        (root / relative).mkdir(parents=True, mode=0o700, exist_ok=True)
    (root / ".infinishell-grok-live-probe").write_text(shared.MARKER, encoding="utf-8")
    sentinel = "INFINISHELL_SKILL_" + secrets.token_hex(32)
    skill = skill_document(args.case, sentinel).encode("utf-8")
    (root / SKILL_PATH).write_bytes(skill)
    (root / SKILL_PATH).chmod(0o400)
    profile_bytes = (Path(__file__).resolve().parents[2] / PROFILE_PATH).read_bytes()
    if sentinel.encode() in profile_bytes or sentinel in PROMPT:
        raise ValueError("秘密标记泄漏到输入")
    profile = root / "policy/profile.md"
    profile.write_bytes(profile_bytes)
    profile.chmod(0o400)
    raw = root / "private-evidence.ndjson"
    raw.touch(mode=0o600)
    (root / "wrapper-audit.ndjson").touch(mode=0o600)
    expected_sha256 = hashlib.sha256(sentinel.encode()).hexdigest()
    metadata = dict(observation(None, [], args.case, expected_sha256), scope=SCOPE, case=args.case, test_name=TEST_NAME,
        private_workspace=str(root), max_native_inputs=1, max_tls_connections=lease.MAX_TLS_CONNECTIONS,
        max_tls_bytes=lease.MAX_TLS_BYTES, deadline_seconds=args.timeout, http_model_calls_observable=False,
        http_model_call_budget_enforced=False, tls_decrypted=False, same_commit_verified_by_runner=False,
        test_argv_override=True, discover_skills=True,
        synthetic_project_trust_requested=True, project_trust_scope="synthetic_project_only",
        project_trust_flag="--trust", trust_store_scope="private_home_only",
        declared_tool_count=1, max_exact_skill_reads=1 if args.case == "visible" else 0,
        read_scope="only_generated_skill_file",
        auth_copy_method="opaque_auth_json_only", requested_model=official.MODEL,
        allowed_https_hosts=sorted(official.OFFICIAL_HOSTS), sentinel_sha256=expected_sha256,
        grok_sha256=shared.digest(args.grok), test_binary_sha256=shared.digest(args.test_binary),
        supervisor_sha256=shared.digest(args.supervisor), profile_sha256=shared.digest(profile))
    events, launches, settings, before_settings = [], [], None, None
    before_auth = lease.auth_identity(args.official_grok_home)
    with isolation.bounded_tunnel(args.timeout) as tunnel:
        try:
            port = tunnel.start()
            official.copy_private_auth(args.official_grok_home, root / "home/.grok")
            metadata["sandbox_canary"] = shared.network_canary(root, args.official_grok_home / "auth.json", port)
            metadata["project_write_canary"] = isolation.project_write_canary(root, args.official_grok_home, port, tunnel.deadline - time.monotonic())
            wrapper, settings = prepare_native(root, args.grok, args.official_grok_home, port, profile)
            before_settings = settings.read_bytes()
            environment = official.official_environment(root, port)
            environment.update(INFINISHELL_GROK_LIVE_ROOT=str(root), INFINISHELL_GROK_LIVE_EXECUTABLE=str(wrapper),
                INFINISHELL_GROK_LIVE_ARTIFACT=str(raw), INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(args.supervisor),
                INFINISHELL_GROK_SKILL_CASE=args.case, CLAUDE_CONFIG_DIR=str(root / "home/.claude"), CODEX_HOME=str(root / "home/.codex"))
            remaining = tunnel.deadline - time.monotonic()
            if remaining <= 0 or tunnel.forwarded != 0 or any(sentinel in value for value in environment.values()):
                raise ValueError("输入前隔离边界失败")
            version = subprocess.run([str(wrapper), "--version"], env=environment, cwd=root / "project",
                capture_output=True, text=True, timeout=min(10, remaining), check=True)
            if version.stdout.strip() != shared.VERSION or tunnel.forwarded != 0:
                raise ValueError("CLI 版本或输入前网络预算不匹配")
            command = [str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
            with os.fdopen(os.open(root / "private-test-output.txt", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600), "wb") as output:
                process = subprocess.Popen(command, cwd=Path(__file__).resolve().parents[2], env=environment, stdout=output, stderr=subprocess.STDOUT)
                try:
                    process.wait(timeout=max(0.001, tunnel.deadline - time.monotonic()))
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
            metadata.update(observation(process.returncode, events, args.case, expected_sha256))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            metadata["case_passed"] = False
            metadata["runner_error_type"] = type(error).__name__
        finally:
            try:
                metadata["tunnels_stopped"] = tunnel.close()
            except Exception as error:
                # 关闭错误不得跳过认证清理；仅公开异常类型，整次验收继续保持失败。
                metadata["tunnels_stopped"] = False
                metadata["tunnel_cleanup_error_type"] = type(error).__name__
            try:
                auth = root / "home/.grok/auth.json"
                auth.unlink(missing_ok=True)
                metadata["private_auth_copy_removed"] = not auth.exists()
                metadata["original_auth_stat_unchanged"] = lease.auth_identity(args.official_grok_home) == before_auth
                metadata["profile_snapshot_unchanged"] = profile.read_bytes() == profile_bytes
                metadata["project_snapshot_unchanged"] = project_unchanged(root, skill)
                if settings is not None and before_settings is not None:
                    metadata["private_settings_audit"] = official.audit_private_settings(before_settings, settings.read_bytes())
                launches = isolation.private_events(root / "wrapper-audit.ndjson")
            except (OSError, ValueError) as error:
                metadata["cleanup_error_type"] = type(error).__name__
            metadata["boundary_passed"] = boundary_passed(metadata, launches, tunnel)
            metadata["case_passed"] &= metadata["boundary_passed"]
            metadata["native_skill_expansion_verified"] &= metadata["case_passed"]
            metadata["hidden_control_verified"] &= metadata["case_passed"]
            metadata["native_catalog_selected_path_verified"] &= metadata["case_passed"]
            metadata["native_catalog_selected_path_seen_before_input"] &= metadata["case_passed"]
            safe = events if all(validate_event(event) for event in events) else []
            args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in safe), encoding="utf-8")
            network = args.output.with_suffix(".network.json")
            network.write_text(json.dumps({"events": tunnel.events, "tls_connections_attempted": tunnel.forwarded,
                "tls_bytes": tunnel.bytes, "http_model_calls_observable": False}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            metadata.update(public_evidence_sha256=shared.digest(args.output), public_network_sha256=shared.digest(network))
            args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("原生技能黑盒验收" + ("通过" if metadata["case_passed"] else "未通过"))
    print(f"脱敏证据：{args.output}")
    return 0 if metadata["case_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("test-binary", "grok", "supervisor", "official-grok-home", "output"):
        parser.add_argument("--" + option, type=Path, required=True)
    parser.add_argument("--case", choices=("visible", "hidden"), default="visible")
    parser.add_argument("--max-native-inputs", type=int, default=1)
    parser.add_argument("--timeout", type=int, default=MAX_DEADLINE)
    args = parser.parse_args()
    try:
        isolation.validate_paths(args)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"技能运行器启动失败：{type(error).__name__}\n")


if __name__ == "__main__":
    raise SystemExit(main())
