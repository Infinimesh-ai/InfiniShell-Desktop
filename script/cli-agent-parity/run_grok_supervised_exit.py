#!/usr/bin/env python3
"""以同次构建产物验证固定 Grok 的生产监督清理；不提供认证或模型输入。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import uuid

sys.dont_write_bytecode = True
import prepare_grok_cli as fixed

VERSION = "1.0.41"
TEST_NAME = "ai::cli_agent_runtime::grok::supervised_exit_live_tests::real_grok_1041_supervised_exit_without_credentials"
SCOPE = "grok_1041_production_supervised_exit_zero_model"
MARKER = ".infinishell-grok-supervised-exit"
MARKER_BYTES = b"isolated Grok supervised exit verification v1\n"
SCENARIOS = ("private_leader_finish", "private_leader_stdin_eof", "no_leader_finish", "no_leader_stdin_eof")
CASE_STAGES = ("startup", "acp", "native_identity", "finish", "receipt")
SETUP_PHASES = ("auth", "resolve_workspace", "folder_trust", "plugin_registry", "mcp_merge",
                "persistence_init", "spawn_session_actor")
REPO = Path(__file__).resolve().parents[2]
SOURCES = (
    "app/src/ai/cli_agent_runtime/grok.rs",
    "app/src/ai/cli_agent_runtime/grok_supervised_exit_live_tests.rs",
    "app/src/ai/cli_agent_runtime/managed_process.rs",
    "app/src/ai/cli_agent_runtime/managed_process_macos.rs",
    "app/src/ai/cli_agent_runtime/codex_idle_crash_identity_tests.rs",
    "crates/command/src/managed.rs",
    "crates/command/src/managed_macos.rs",
    "script/cli-agent-parity/prepare_grok_cli.py",
    "script/cli-agent-parity/run_grok_supervised_exit.py",
    "script/cli-agent-parity/run_grok_supervised_exit_tests.py",
)
CONTAINMENT = {"darwin-arm64": "macos_resource_coalition", "linux-x64": "linux_subtree", "win32-x64": "windows_job"}
IDENTITY = {"darwin-arm64": "darwin_audit_token_pidversion", "linux-x64": "linux_pidfd_send_signal", "win32-x64": "windows_owned_process_handle"}
MAX_LOG_BYTES = 8 * 1024 * 1024
MAX_EVENTS_BYTES = 256 * 1024


def require(value, reason):
    if not value:
        raise ValueError(reason)


def digest(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def regular(path):
    require(path.is_absolute(), "input_not_absolute")
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and not path.is_symlink()
            and not getattr(info, "st_file_attributes", 0) & 0x400, "input_not_regular")
    return path.resolve(strict=True)


def binding(path):
    regular(path)
    return {"path": str(path), "sha256": digest(path), "bytes": path.stat().st_size}


def write_private(path, value):
    data = json.dumps(value, ensure_ascii=False, indent=2).encode() + b"\n"
    with path.open("xb") as handle:
        handle.write(data)
    path.chmod(0o600)


def source_identity():
    def git(*args):
        return subprocess.check_output(["git", "-C", str(REPO), *args], stderr=subprocess.DEVNULL).decode().strip()
    commit = git("rev-parse", "HEAD")
    changed = {path for path in git("diff", "--name-only", "HEAD").splitlines()
               if not path.startswith("specs/cli-agent-parity/")}
    # 预提交只读取明确的测试源码及已跟踪变更；不扫描或收录任意私有产物。
    tracked = set(git("ls-files", "--", *SOURCES).splitlines())
    changed.update(set(SOURCES) - tracked)
    source_dirty = bool(changed)
    if os.environ.get("GITHUB_SHA"):
        require(os.environ["GITHUB_SHA"] == commit and not source_dirty, "workflow_source_mismatch")
    files = []
    for relative in sorted(set(SOURCES) | changed):
        path = REPO / relative
        result = subprocess.run(["git", "-C", str(REPO), "rev-parse", f"{commit}:{relative}"],
                                capture_output=True, text=True, check=False)
        files.append({"path": relative, "base_git_object": result.stdout.strip() if result.returncode == 0 else None,
                      "sha256": digest(path) if path.is_file() else None})
    return {"commit": commit, "tree": git("rev-parse", "HEAD^{tree}"), "files": files,
            "source_dirty": source_dirty, "candidate_changed_paths": sorted(changed),
            "execution_source_kind": "precommit_candidate" if source_dirty else "clean_commit",
            "compiled_source_identity_independently_verified": False,
            "build_provenance_requirement": "测试二进制和 worker 的同提交或同候选构建须结合本次构建日志；本运行器绑定源码和输入摘要，不从文件路径推断编译来源。"}


def is_hash(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def is_uuid(value):
    try:
        return isinstance(value, str) and str(uuid.UUID(value)) == value
    except (ValueError, AttributeError):
        return False


def project_case(event, target, manifest_digest):
    keys = {"event", "scenario", "cli_version", "generation", "inputs_manifest_sha256", "manifest_sha256",
            "exit_receipt_sha256", "exit_receipt", "observed_native_processes", "observed_native_processes_exited",
            "native_identity_mechanism", "stdout_eof_confirmed", "drained_tail_bytes", "old_generation_rejected",
            "isolated_home", "credentials_provided", "model_commands_sent", "authentication_commands_sent",
            "acp_messages_sent", "setup_phases", "missing_cancel_ack_claimed", "full_adapter_lifecycle_claimed", "fixed_policy_permissions_claimed"}
    require(type(event) is dict and set(event) == keys, "case_fields_changed")
    require(event["event"] == "case_passed" and event["scenario"] in SCENARIOS
            and event["cli_version"] == VERSION and is_uuid(event["generation"]), "case_identity_invalid")
    require(event["inputs_manifest_sha256"] == manifest_digest and is_hash(manifest_digest)
            and all(is_hash(event[key]) for key in ("manifest_sha256", "exit_receipt_sha256")), "case_digest_invalid")
    receipt = event["exit_receipt"]
    require(type(receipt) is dict and set(receipt) == {"version", "generation", "cleanup_confirmed", "containment", "exit_reason", "exit_code", "manifest_sha256"}, "receipt_fields_changed")
    require(type(receipt["version"]) is int and receipt["version"] == 1
            and receipt["generation"] == event["generation"] and receipt["cleanup_confirmed"] is True
            and receipt["containment"] == CONTAINMENT[target] and receipt["manifest_sha256"] == event["manifest_sha256"]
            and receipt["exit_reason"] in ("native_exit", "stop_requested", "host_disconnected", "stdio_closed")
            and (receipt["exit_code"] is None or type(receipt["exit_code"]) is int), "receipt_not_current_native_cleanup")
    require(event["native_identity_mechanism"] == IDENTITY[target], "native_identity_invalid")
    expected_setup = SETUP_PHASES[:5] if event["scenario"].startswith("private_leader_") else SETUP_PHASES
    require(event["setup_phases"] == list(expected_setup), "setup_sequence_changed")
    minimum = 2 if event["scenario"].startswith("private_leader_") else 1
    require(type(event["observed_native_processes"]) is int and minimum <= event["observed_native_processes"] <= 64
            and type(event["drained_tail_bytes"]) is int and 0 <= event["drained_tail_bytes"] <= 1024 * 1024, "native_observation_invalid")
    for key in ("observed_native_processes_exited", "stdout_eof_confirmed", "old_generation_rejected", "isolated_home"):
        require(event[key] is True, "required_boundary_not_verified")
    for key in ("credentials_provided", "missing_cancel_ack_claimed", "full_adapter_lifecycle_claimed", "fixed_policy_permissions_claimed"):
        require(event[key] is False, "unsupported_claim")
    for key, expected in (("model_commands_sent", 0), ("authentication_commands_sent", 0), ("acp_messages_sent", 5)):
        require(type(event[key]) is int and event[key] == expected, "input_budget_changed")
    # 只有通过类型、值和字段白名单的内容才进入公开 JSON。
    return dict(event)


def project_events(events, target, manifest_digest):
    cases = []
    started = []
    phases = []
    for event in events:
        require(type(event) is dict, "event_not_object")
        if event.get("event") == "case_started":
            require(set(event) == {"event", "scenario"} and len(started) < 4
                    and event["scenario"] == SCENARIOS[len(started)] and len(started) == len(cases), "case_order_invalid")
            started.append(event["scenario"])
            phases = []
        elif event.get("event") == "case_stage":
            require(set(event) == {"event", "scenario", "stage"} and len(started) == len(cases) + 1
                    and event["scenario"] == started[-1] and len(phases) < len(CASE_STAGES)
                    and event["stage"] == CASE_STAGES[len(phases)], "case_stage_invalid")
            phases.append(event["stage"])
        else:
            case = project_case(event, target, manifest_digest)
            require(len(started) == len(cases) + 1 and case["scenario"] == started[-1]
                    and case["generation"] not in {row["generation"] for row in cases}
                    and phases == list(CASE_STAGES), "case_generation_or_order_invalid")
            cases.append(case)
    return cases, started


def project_partial_events(events, target, manifest_digest):
    accepted = ([], [])
    for end in range(1, len(events) + 1):
        try:
            accepted = project_events(events[:end], target, manifest_digest)
        except (ValueError, KeyError, TypeError):
            break
    return accepted


def verify_case_files(root, case, inputs):
    directory = root / case["scenario"] / "state/cli-agent-processes" / case["generation"]
    receipt_path, manifest_path = directory / "exit.json", directory / "manifest.json"
    require(digest(receipt_path) == case["exit_receipt_sha256"] and digest(manifest_path) == case["manifest_sha256"], "persisted_receipt_digest_changed")
    require(json.loads(receipt_path.read_bytes()) == case["exit_receipt"], "persisted_receipt_changed")
    manifest = json.loads(manifest_path.read_bytes())
    # Windows 普通路径与扩展路径可能同指一个对象；目录和二进制必须实际存在且身份相同。
    try:
        executable_matches = Path(manifest["executable"]).samefile(inputs["grok"]["path"])
        home_matches = Path(manifest["isolated_home"]).samefile(
            root / case["scenario"] / "state/grok-managed" / case["generation"])
    except OSError as error:
        raise ValueError("persisted_manifest_binding_changed") from error
    require(manifest["generation"] == case["generation"] and manifest["launch_allowed"] is True
            and executable_matches and home_matches, "persisted_manifest_binding_changed")


def read_events(path, allow_partial=False):
    if not path.exists():
        return []
    require(path.stat().st_size <= MAX_EVENTS_BYTES, "event_budget_exceeded")
    lines = path.read_bytes().splitlines(keepends=True)
    events = []
    for index, line in enumerate(lines):
        if not line.strip():
            continue
        try:
            events.append(json.loads(line))
        except ValueError:
            # 超时恰好中断末次写入时，只保留此前完整记录；损坏的完整行仍拒绝。
            if allow_partial and index == len(lines) - 1 and not line.endswith(b"\n"):
                break
            raise
    return events


def safe_failure_code(error):
    codes = {"workflow_source_mismatch", "input_not_absolute", "input_not_regular", "case_fields_changed",
             "case_identity_invalid", "case_digest_invalid", "receipt_fields_changed", "receipt_not_current_native_cleanup",
             "native_identity_invalid", "native_observation_invalid", "setup_sequence_changed", "required_boundary_not_verified", "unsupported_claim",
             "input_budget_changed", "event_not_object", "case_order_invalid", "case_stage_invalid", "case_generation_or_order_invalid",
             "persisted_receipt_digest_changed", "persisted_receipt_changed", "persisted_manifest_binding_changed",
             "event_budget_exceeded", "output_not_absolute_json", "invalid_timeout", "output_already_exists",
             "binary_roles_not_distinct", "test_not_listed", "log_budget_exceeded", "test_did_not_complete_four_cases",
             "inputs_or_source_changed"}
    if isinstance(error, ValueError) and len(error.args) == 1 and error.args[0] in codes:
        return error.args[0]
    return "unclassified_safe_error"


def panic_locations(output):
    result = []
    for path, line, column in re.findall(r"panicked at ((?:app|crates)/[A-Za-z0-9_./-]+\.rs):([0-9]+):([0-9]+):", output.replace("\\", "/")):
        if path in SOURCES:
            item = {"path": path, "line": int(line), "column": int(column)}
            if item not in result:
                result.append(item)
    return result[:4]


def pidfd_open_os_errors(output):
    # 仅导出已知身份夹具的数值错误码，不发布原始错误文本、路径或进程信息。
    if not any(item["path"] == "app/src/ai/cli_agent_runtime/codex_idle_crash_identity_tests.rs"
               for item in panic_locations(output)):
        return []
    codes = re.findall(r"(?m)^pidfd_open 失败：[^\r\n]{0,256}\(os error ([0-9]{1,5})\)\r?$", output)
    return sorted({int(code) for code in codes if 0 < int(code) <= 4095})[:4]


def safe_progress(events, target, manifest_digest):
    result = []
    for end in range(1, len(events) + 1):
        try:
            project_events(events[:end], target, manifest_digest)
        except (ValueError, KeyError, TypeError):
            break
        event = events[end - 1]
        if event.get("event") == "case_stage":
            result.append({"scenario": event["scenario"], "stage": event["stage"]})
    return result


def run(args):
    require(args.output.is_absolute() and args.output.suffix == ".json", "output_not_absolute_json")
    require(30 <= args.timeout <= 600, "invalid_timeout")
    private = args.output.with_suffix(".private")
    require(not args.output.exists() and not args.output.is_symlink()
            and not private.exists() and not private.is_symlink(), "output_already_exists")
    paths = {key: regular(getattr(args, key)) for key in ("test_binary", "grok", "supervisor")}
    require(len(set(paths.values())) == 3, "binary_roles_not_distinct")
    target = fixed.current_platform(VERSION)
    native = fixed.verify_binary(paths["grok"], target, VERSION)
    source = source_identity()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    private.mkdir(mode=0o700)
    (private / MARKER).write_bytes(MARKER_BYTES)
    (private / MARKER).chmod(0o600)
    environment = fixed.isolated_environment(private / "runner")
    inputs = {"schema": 1, "root": str(private.resolve()), "source_commit": source["commit"], "source_tree": source["tree"],
              "cli_version": VERSION, **{key: binding(path) for key, path in paths.items()}}
    manifest = private / "inputs.private.json"
    write_private(manifest, inputs)
    manifest_digest = digest(manifest)
    environment.update(INFINISHELL_GROK_SUPERVISED_EXIT_MANIFEST=str(manifest),
                       INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(paths["supervisor"]))
    report = {"schema": 1, "scope": SCOPE, "source": source, "cli_version": VERSION,
              "platform": target, "native_binary": native,
              "inputs": {key: {field: value[field] for field in ("sha256", "bytes")} for key, value in inputs.items() if key in paths},
              "inputs_manifest_sha256": manifest_digest, "credentials_provided": False,
              "model_commands_sent": 0, "authentication_commands_sent": 0,
              "model_http_count_measured": False, "test_name": TEST_NAME,
              "timeout_seconds": args.timeout, "cases": [], "started_scenarios": [], "passed": False,
              "limitations": ["只证明真实 Grok ACP 进程经生产监督器的显式结束及 EOF 清理；不宣称完整 GrokAdapter 生命周期或用户 Stop 操作。",
                              "--no-leader 只覆盖进程/stdio 拓扑，不代表固定权限 profile 的完整验收。",
                              "session/cancel 指向不存在的会话且无响应确认；不宣称取消了运行中回合。",
                              "零模型结论来自发送消息集合和无凭据配置，不是 HTTP 流量计量。"]}
    stage = "version"
    log_path = private / "libtest.private.log"
    exit_code = None
    timed_out = False
    try:
        (private / "version").mkdir(mode=0o700)
        fixed.verify_version(paths["grok"], private / "version", VERSION)
        stage = "list_test"
        listing = subprocess.run([str(paths["test_binary"]), TEST_NAME, "--exact", "--ignored", "--list"],
                                 cwd=private, env=environment, capture_output=True, timeout=30, check=False)
        require(listing.returncode == 0 and len(listing.stdout) <= 65536
                and listing.stdout.decode(errors="replace").splitlines().count(TEST_NAME + ": test") == 1, "test_not_listed")
        stage = "run_test"
        with log_path.open("xb") as log:
            process = subprocess.Popen([str(paths["test_binary"]), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                       cwd=private, env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
            try:
                exit_code = process.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                # 仅终止自己持有的测试宿主；控制管道关闭后由生产监督器负责原生进程树。
                process.kill()
                exit_code = process.wait(timeout=10)
        stage = "verify_receipts"
        cases, started = project_events(read_events(private / "events.private.ndjson"), target, manifest_digest)
        report.update(cases=cases, started_scenarios=started)
        for case in cases:
            verify_case_files(private, case, inputs)
        require(log_path.stat().st_size <= MAX_LOG_BYTES, "log_budget_exceeded")
        text = log_path.read_text(encoding="utf-8", errors="replace")
        require(not timed_out and exit_code == 0 and len(cases) == 4
                and re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", text) is not None, "test_did_not_complete_four_cases")
        require(digest(manifest) == manifest_digest and all(binding(path) == inputs[key] for key, path in paths.items())
                and source_identity() == source, "inputs_or_source_changed")
        report.update(passed=True, inputs_unchanged=True, source_unchanged=True,
                      cleanup_confirmed=True, all_observed_native_processes_exited=True)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        report.update(failure_stage=stage, failure_type=type(error).__name__, failure_code=safe_failure_code(error))
        # 即使末场景失败，也保留前面已经完成且字段安全的真实回执。
        try:
            cases, started = project_partial_events(read_events(private / "events.private.ndjson", allow_partial=True), target, manifest_digest)
            for case in cases:
                verify_case_files(private, case, inputs)
            report.update(cases=cases, started_scenarios=started)
        except (OSError, ValueError, KeyError, TypeError):
            report["partial_receipts_unavailable"] = True
    report.update(test_exit_code=exit_code, timed_out=timed_out)
    if log_path.exists():
        if log_path.stat().st_size <= MAX_LOG_BYTES:
            output = log_path.read_text(encoding="utf-8", errors="replace")
            report["panic_locations"] = panic_locations(output)
            report["pidfd_open_os_errors"] = pidfd_open_os_errors(output)
        log_path.chmod(0o600)
        report["raw_log"] = {"sha256": digest(log_path), "bytes": log_path.stat().st_size, "content_published": False}
    events = private / "events.private.ndjson"
    if events.exists():
        try:
            report["case_progress"] = safe_progress(read_events(events, allow_partial=True), target, manifest_digest)
        except (OSError, ValueError, KeyError, TypeError):
            report["case_progress_unavailable"] = True
        events.chmod(0o600)
        report["raw_events"] = {"sha256": digest(events), "bytes": events.stat().st_size, "content_published": False}
    write_private(args.output, report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("test-binary", "grok", "supervisor", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args()
    try:
        report = run(args)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(json.dumps({"passed": False, "failure_type": type(error).__name__, "failure_code": safe_failure_code(error)}))
        return 1
    print(json.dumps({"passed": report["passed"], "cases": len(report["cases"]), "output": str(args.output)}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
