#!/usr/bin/env python3
"""默认生产入口的单次 typed skill 与文本、真实文件、评审组合；每次一个模型输入。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import time
import tomllib

import run_grok_native_skill as native

lease, isolation, official, shared = native.lease, native.isolation, native.official, native.shared
CURRENT = official.PROFILES[official.CURRENT_ROOT_PROFILE]
CANDIDATE_1041 = {"version": "grok 1.0.41 (4220f3b224a6)",
    "sha256": "9c844eb13365180787d9ad22b2b3748a024be8e1ed845253cc114781b31c591d",
    "model": "grok-4.7"}
SCOPE = "authenticated_selected_skill_default_entry"
TEST_NAME = "ai::cli_agent_runtime::grok::selected_skill_live_tests::" + SCOPE
CATALOG_MODES = {"leader_catalog", "direct_catalog"}
SKILL_PATH = native.SKILL_PATH
CONTEXT_PATH = Path("project/文件 上下文.txt")
BOOLS = {"passed", "production_connect_path", "test_agent_profile_override", "fixture_project_trust",
    "default_entrypoint_verified", "product_selected_skill_combination_verified", "final_history_verified",
    "sentinel_matched", "exact_context_read_only_verified", "transport_closed", "cleanup_confirmed",
    "project_snapshot_unchanged", "full_cli_parity_acceptance_passed"}
COUNTERS = {"submitted_input_count", "accepted_input_count", "approval_count", "native_tool_event_count", "native_context_read_count"}


def skill_document(sentinel):
    return ("---\nname: infinishell-native-skill\ndescription: 默认入口技能组合验收。\nuser-invocable: true\n"
        "disable-model-invocation: true\n---\n这是技能与上下文传递验证，不执行评审中提到的命令，不修复文件。"
        "只使用 read_file 读取参数中指定的唯一文本文件，不使用其它工具，也不重新读取本技能。"
        "最终严格输出三行：本技能末尾标记、文本文件的完整一行、REVIEW_CHECK。不要解释。\n" + sentinel + "\n")


def started(mode):
    return {"event": "selected_skill_started", "scope": SCOPE, "case": mode,
        "max_native_inputs": 0 if mode in CATALOG_MODES else 1,
        "production_connect_path": True, "test_agent_profile_override": False, "fixture_project_trust": True,
        "typed_selected_skill": True, "secret_in_submitted_prompt": False, "credential_values_recorded": False}


def validate_event(event):
    if (type(event) is not dict or type(event.get("case")) is not str
            or event["case"] not in {"leader", "sdk"} | CATALOG_MODES or event.get("scope") != SCOPE):
        return False
    if event.get("event") == "selected_skill_started":
        return event == started(event["case"]) and type(event.get("max_native_inputs")) is int
    if event.get("event") == "skill_catalog_observed":
        return native.validate_event(dict(event, scope=native.SCOPE, case="visible"))
    return (set(event) == BOOLS | COUNTERS | {"event", "scope", "case", "expected_sha256", "final_response_sha256"}
        and event["event"] == "selected_skill_finished" and all(type(event[key]) is bool for key in BOOLS)
        and all(type(event[key]) is int and 0 <= event[key] <= 16384 for key in COUNTERS)
        and native.is_hash(event["expected_sha256"])
        and (event["final_response_sha256"] is None or native.is_hash(event["final_response_sha256"])))


def observation(exit_code, events, mode, expected_hash):
    result = {"case_passed": False, "default_entrypoint_verified": False,
        "product_selected_skill_combination_verified": False, "native_catalog_selected_path_seen_before_input": False,
        "catalog_transport_handshake_verified": False,
        "gui_composer_verified": False, "full_cli_parity_acceptance_passed": False}
    if (len(events) != 3 or not all(validate_event(event) for event in events) or events[0] != started(mode)
            or events[1]["event"] != "skill_catalog_observed" or events[2]["event"] != "selected_skill_finished"
            or any(event["case"] != mode for event in events) or not native.is_hash(expected_hash)):
        return result
    catalog, end = events[1:]
    before = [row for row in catalog["snapshots"] if row["before_first_submit"]]
    bound = (not catalog["overflow"] and bool(before) and before[-1]["path_matches_selected"]
        and before[-1]["bare_name_matches"])
    if mode in CATALOG_MODES:
        diagnostic = (type(exit_code) is int and exit_code == 0 and bound
            and end["passed"] is False and end["transport_closed"] is True
            and end["cleanup_confirmed"] is True and end["project_snapshot_unchanged"] is True
            and end["test_agent_profile_override"] is False
            and end["default_entrypoint_verified"] is False
            and end["product_selected_skill_combination_verified"] is False
            and end["full_cli_parity_acceptance_passed"] is False
            and all(end[key] == 0 for key in COUNTERS)
            and end["final_response_sha256"] is None)
        result.update(catalog_transport_handshake_verified=diagnostic,
            native_catalog_selected_path_seen_before_input=diagnostic)
        return result
    passed = (type(exit_code) is int and exit_code == 0 and bound
        and all(end[key] is True for key in BOOLS - {"test_agent_profile_override", "full_cli_parity_acceptance_passed"})
        and end["test_agent_profile_override"] is False and end["full_cli_parity_acceptance_passed"] is False
        and end["submitted_input_count"] == end["accepted_input_count"] == end["native_context_read_count"] == 1
        and end["approval_count"] in (0, 1) and end["native_tool_event_count"] >= 1
        and end["expected_sha256"] == end["final_response_sha256"] == expected_hash)
    result.update(case_passed=passed, default_entrypoint_verified=passed,
        product_selected_skill_combination_verified=passed, native_catalog_selected_path_seen_before_input=passed)
    return result


def prepare_native(root, executable, source_home, port, mode, profile=CURRENT):
    if mode == "sdk":
        wrapper, settings = isolation.prepare_probe_native(root, executable, source_home, port)
    elif mode in {"leader", "leader_catalog", "direct_catalog"}:
        wrapper, settings = official.prepare_native(root, executable, source_home, port,
            binary_sha256=profile["sha256"], model=profile["model"])
        code = wrapper.read_text(encoding="utf-8")
        if mode == "direct_catalog":
            leader = """elif len(args)==4 and args[:3]==['agent','stdio','--leader-socket']:
 endpoint=Path(args[3]);resolved=endpoint.resolve()
 if not endpoint.is_absolute() or endpoint!=resolved or not resolved.is_relative_to(socket_root) or resolved.name!='leader.sock' or resolved.exists():raise SystemExit(92)
 parent=resolved.parent
 if parent.stat().st_uid!=os.getuid() or parent.stat().st_mode & 0o077:raise SystemExit(93)
 quoted=json.dumps(str(resolved))
 profile+='(allow file-write* (subpath '+json.dumps(str(socket_root))+'))(allow network-bind network-inbound (literal '+quoted+'))(allow network-outbound (remote unix-socket (path-literal '+quoted+')))'
 kind='private_leader'
"""
            if code.count(leader) != 1:
                raise ValueError("原生直连入口改变")
            code = code.replace(leader, "elif args==['agent','--no-leader','stdio']:kind='direct_agent';endpoint=None\n", 1)
        if code.count("if args==['--version']:") != 1:
            raise ValueError("默认入口包装器边界改变")
        wrapper.write_text(code.replace("if args==['--version']:", f"profile+={isolation.project_deny(root)!r}\nif args==['--version']:", 1), encoding="utf-8")
    else:
        raise ValueError("入口类型无效")
    code = wrapper.read_text(encoding="utf-8")
    audit = "with (root/'wrapper-audit.ndjson').open('a') as evidence:"
    guard = ("if kind!='version':\n"
        " if root.resolve()!=root or any((root/part).resolve(strict=True)!=root/part for part in ('home','home/.grok','project')):raise SystemExit(96)\n"
        " if Path.cwd()!=root/'project' or Path(os.environ.get('HOME',''))!=root/'home' or Path(os.environ.get('GROK_HOME',''))!=root/'home/.grok':raise SystemExit(96)\n")
    if code.count(audit) != 1 or code.count("'arguments_unchanged':True,") != 1 or code.count("str(native),*args]") != 1:
        raise ValueError("默认入口包装器审计改变")
    code = code.replace(audit, guard + audit, 1).replace("'arguments_unchanged':True,",
        "'arguments_unchanged':kind=='version','synthetic_project_trust_requested':kind!='version',", 1)
    code = code.replace("str(native),*args]", "str(native),*(['--trust',*args] if kind!='version' else args)]", 1)
    compile(code, str(wrapper), "exec")
    wrapper.write_text(code, encoding="utf-8")
    return wrapper, settings


def project_unchanged(root, snapshot):
    entries = list((root / "project").rglob("*"))
    expected = {".grok", ".grok/skills", ".grok/skills/infinishell-native-skill",
        ".grok/skills/infinishell-native-skill/SKILL.md", "文件 上下文.txt"}
    return ({path.relative_to(root / "project").as_posix() for path in entries} == expected
        and all(not path.is_symlink() for path in entries)
        and all((root / relative).read_bytes() == body for relative, body in snapshot.items()))


def stop_tunnel(tunnel, metadata):
    # 关闭代理失败不能跳过凭据清理，原始失败仍会阻止验收通过。
    try:
        metadata["tunnels_stopped"] = tunnel.close() is True
    except Exception as error:
        metadata["tunnels_stopped"] = False
        metadata["tunnel_close_error_type"] = type(error).__name__


def audit_current_settings(before, after):
    audit = official.audit_private_settings(before, after)
    purge_only = False
    if audit["toml_parse_succeeded"]:
        initial = tomllib.loads(before.decode("utf-8"))
        current = tomllib.loads(after.decode("utf-8"))
        expected = dict(initial, marketplace={"default_skills_installs_purged": True})
        purge_only = ("marketplace" not in initial
            and json.dumps(current, sort_keys=True) == json.dumps(expected, sort_keys=True))
    audit["native_marketplace_purge_only"] = purge_only
    audit["settings_scope_verified"] = audit["settings_scope_verified"] or purge_only
    return audit


def boundary_passed(metadata, launches, tunnel, mode):
    expected = "private_leader" if mode in {"leader", "leader_catalog"} else "direct_agent"
    return (metadata.get("test_exit_code") == 0 and metadata.get("timed_out") is not True
        and all(metadata.get(key) is True for key in ("tunnels_stopped", "private_auth_copy_removed",
            "original_auth_stat_unchanged", "project_snapshot_unchanged"))
        and metadata.get("private_settings_audit", {}).get("settings_scope_verified") is True
        and sorted(row.get("kind", "") for row in launches) == sorted([expected, "version", "version"])
        and all(row.get("arguments_unchanged") is (row.get("kind") == "version")
            and row.get("synthetic_project_trust_requested") is (row.get("kind") != "version") for row in launches)
        and tunnel.forwarded <= lease.MAX_TLS_CONNECTIONS and tunnel.bytes <= lease.MAX_TLS_BYTES
        and any(row == {"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"} for row in tunnel.events)
        and not any(row.get("event") in {"official_connect_budget_rejected", "tunnel_byte_budget_exhausted"} for row in tunnel.events))


def run(args):
    catalog_only = args.mode in CATALOG_MODES
    test_candidate_1041_catalog = getattr(args, "test_candidate_1041_catalog", False)
    test_candidate_1041_turn = getattr(args, "test_candidate_1041_turn", False)
    if test_candidate_1041_catalog and (args.mode != "leader_catalog" or args.max_native_inputs != 0):
        raise ValueError("1.0.41 候选仅允许零输入默认 leader 目录")
    if test_candidate_1041_turn and (test_candidate_1041_catalog or args.mode != "leader"
            or args.max_native_inputs != 1):
        raise ValueError("1.0.41 回合候选仅允许一次输入默认 leader")
    test_candidate_1041 = test_candidate_1041_catalog or test_candidate_1041_turn
    profile = CANDIDATE_1041 if test_candidate_1041 else CURRENT
    args.output.parent.mkdir(parents=True, exist_ok=True)
    isolation.reserve_artifacts(args.output)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-selected-skill-", dir="/private/tmp")).resolve()
    root.chmod(0o700)
    for relative in ("home/.grok", "home/.claude", "home/.codex", str(SKILL_PATH.parent), "tmp", "state"):
        (root / relative).mkdir(parents=True, mode=0o700, exist_ok=True)
    (root / ".infinishell-grok-live-probe").write_text(shared.MARKER, encoding="utf-8")
    sentinel = "INFINISHELL_SKILL_" + secrets.token_hex(32)
    context = "INFINISHELL_CONTEXT_" + secrets.token_hex(32)
    expected_hash = hashlib.sha256(f"{sentinel}\n{context}\nREVIEW_CHECK".encode()).hexdigest()
    snapshot = {SKILL_PATH: skill_document(sentinel).encode(), CONTEXT_PATH: (context + "\n").encode()}
    for relative, body in snapshot.items():
        (root / relative).write_bytes(body)
        (root / relative).chmod(0o400)
    raw = root / "private-evidence.ndjson"
    raw.touch(mode=0o600)
    (root / "wrapper-audit.ndjson").touch(mode=0o600)
    before_auth = lease.auth_identity(args.official_grok_home)
    metadata = dict(observation(None, [], args.mode, expected_hash), scope=SCOPE, case=args.mode,
        private_workspace=str(root), test_name=TEST_NAME, max_native_inputs=0 if catalog_only else 1, expected_sha256=expected_hash,
        max_tls_connections=lease.MAX_TLS_CONNECTIONS, max_tls_bytes=lease.MAX_TLS_BYTES, deadline_seconds=args.timeout,
        test_agent_profile_override=False, fixture_project_trust=True, project_trust_scope="synthetic_project_only",
        trust_store_scope="private_home_only", native_profile_override=False, runtime_permission_policy="Inherit",
        auth_copy_method="opaque_auth_json_only", allowed_https_hosts=sorted(official.OFFICIAL_HOSTS),
        http_model_calls_observable=False, http_model_call_budget_enforced=False, tls_decrypted=False,
        same_commit_verified_by_runner=False, grok_sha256=shared.digest(args.grok),
        verified_cli_version=profile["version"], requested_model=profile["model"],
        test_only_1041_profile=test_candidate_1041,
        test_binary_sha256=shared.digest(args.test_binary), supervisor_sha256=shared.digest(args.supervisor))
    events, launches, settings, before_settings = [], [], None, None
    with isolation.bounded_tunnel(args.timeout) as tunnel:
        try:
            port = tunnel.start()
            official.copy_private_auth(args.official_grok_home, root / "home/.grok")
            metadata["sandbox_canary"] = shared.network_canary(root, args.official_grok_home / "auth.json", port)
            metadata["project_write_canary"] = isolation.project_write_canary(root, args.official_grok_home, port, tunnel.deadline - time.monotonic())
            wrapper, settings = prepare_native(root, args.grok, args.official_grok_home, port,
                args.mode, profile)
            before_settings = settings.read_bytes()
            environment = official.official_environment(root, port)
            environment.update(INFINISHELL_GROK_LIVE_ROOT=str(root), INFINISHELL_GROK_LIVE_EXECUTABLE=str(wrapper),
                INFINISHELL_GROK_LIVE_ARTIFACT=str(raw), INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(args.supervisor),
                INFINISHELL_GROK_SELECTED_SKILL_MODE=args.mode,
                CLAUDE_CONFIG_DIR=str(root / "home/.claude"), CODEX_HOME=str(root / "home/.codex"))
            if test_candidate_1041:
                environment["INFINISHELL_GROK_TEST_CANDIDATE_1041"] = "1"
                environment["INFINISHELL_GROK_TEST_CANDIDATE_NATIVE"] = str(args.grok)
            remaining = tunnel.deadline - time.monotonic()
            if remaining <= 0 or tunnel.forwarded != 0 or any(sentinel in value or context in value for value in environment.values()):
                raise ValueError("输入前隔离边界失败")
            version = subprocess.run([str(wrapper), "--version"], env=environment, cwd=root / "project",
                capture_output=True, text=True, timeout=min(10, remaining), check=True)
            if version.stdout.strip() != profile["version"] or tunnel.forwarded != 0:
                raise ValueError("固定 CLI 版本或网络预算不匹配")
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
            events = native.read_events(raw)
            metadata.update(observation(process.returncode, events, args.mode, expected_hash))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            metadata["case_passed"] = False
            metadata["runner_error_type"] = type(error).__name__
        finally:
            stop_tunnel(tunnel, metadata)
            try:
                auth = root / "home/.grok/auth.json"
                auth.unlink(missing_ok=True)
                metadata["private_auth_copy_removed"] = not auth.exists()
                metadata["original_auth_stat_unchanged"] = lease.auth_identity(args.official_grok_home) == before_auth
                metadata["project_snapshot_unchanged"] = project_unchanged(root, snapshot)
                if settings is not None and before_settings is not None:
                    metadata["private_settings_audit"] = audit_current_settings(before_settings, settings.read_bytes())
                launches = isolation.private_events(root / "wrapper-audit.ndjson")
            except (OSError, ValueError) as error:
                metadata["cleanup_error_type"] = type(error).__name__
            metadata["boundary_passed"] = boundary_passed(metadata, launches, tunnel, args.mode)
            metadata["case_passed"] &= metadata["boundary_passed"]
            for key in ("default_entrypoint_verified", "product_selected_skill_combination_verified"):
                metadata[key] &= metadata["case_passed"]
            for key in ("native_catalog_selected_path_seen_before_input", "catalog_transport_handshake_verified"):
                metadata[key] &= metadata["boundary_passed"]
            safe = events if all(validate_event(event) for event in events) else []
            args.output.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in safe), encoding="utf-8")
            network = args.output.with_suffix(".network.json")
            network.write_text(json.dumps({"events": tunnel.events, "tls_connections_attempted": tunnel.forwarded,
                "tls_bytes": tunnel.bytes, "http_model_calls_observable": False}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            metadata.update(public_evidence_sha256=shared.digest(args.output), public_network_sha256=shared.digest(network))
            args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if catalog_only:
        print("Grok 零输入目录传输对照" + ("通过" if metadata["catalog_transport_handshake_verified"] else "未通过"))
    else:
        print("默认入口技能组合验收" + ("通过" if metadata["case_passed"] else "未通过"))
    print(f"脱敏证据：{args.output}")
    return 0 if metadata["case_passed"] or (catalog_only and metadata["catalog_transport_handshake_verified"]) else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("test-binary", "grok", "supervisor", "official-grok-home", "output"):
        parser.add_argument("--" + option, type=Path, required=True)
    parser.add_argument("--mode", choices=("leader", "leader_catalog", "direct_catalog"), required=True)
    parser.add_argument("--max-native-inputs", type=int, default=1)
    parser.add_argument("--test-candidate-1041-catalog", action="store_true")
    parser.add_argument("--test-candidate-1041-turn", action="store_true")
    parser.add_argument("--timeout", type=int, default=lease.MAX_DEADLINE)
    args = parser.parse_args()
    try:
        if args.max_native_inputs != (0 if args.mode in CATALOG_MODES else 1):
            raise ValueError("模型输入预算与入口类型不匹配")
        if args.test_candidate_1041_catalog and args.mode != "leader_catalog":
            raise ValueError("1.0.41 候选仅允许零输入默认 leader 目录")
        if args.test_candidate_1041_turn and (args.test_candidate_1041_catalog or args.mode != "leader"):
            raise ValueError("1.0.41 回合候选仅允许一次输入默认 leader")
        profile = CANDIDATE_1041 if args.test_candidate_1041_catalog or args.test_candidate_1041_turn else CURRENT
        isolation.validate_paths(args, profile["sha256"],
            expected_inputs=0 if args.mode in CATALOG_MODES else 1)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"默认技能运行器启动失败：{type(error).__name__}\n")


if __name__ == "__main__":
    raise SystemExit(main())
