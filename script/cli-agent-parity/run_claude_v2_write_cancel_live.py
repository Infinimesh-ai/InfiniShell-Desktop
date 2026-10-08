#!/usr/bin/env python3
"""固定 Claude V2 父子待审批 Write 取消的隔离真实验收。"""

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

import run_claude_adapter_live as base
from prepare_claude_cli import current_platform, verify_binary, verify_version


TEST_NAME = ("ai::cli_agent_runtime::coordinator::claude_live_tests::"
             "v2_write_cancel_live_tests::real_claude_v2_parent_child_pending_write_cancel")
SCOPE = "real_claude_production_v2_write_cancel"
VERSION = "2.1.280"
EXPECTED_EVENTS = ("parent_started", "parent_spawn_approved", "parent_spawn_called",
                   "child_turn_started", "child_write_pending", "child_interrupt_submitted", "child_interrupt_ack",
                   "child_write_approval_cancelled", "late_allow_submitted", "late_allow_rejected",
                   "child_turn_cancelled", "saved_cancel_chain_verified")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha256(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def once(events, kind):
    found = [event for event in events if event.get("event") == kind]
    require(len(found) == 1, f"{kind} 收据缺失或重复")
    return found[0]


def private_path(path):
    require(not path.is_symlink(), "输入不能是符号链接")
    path = path.resolve(strict=True)
    info = path.stat()
    require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1, "输入必须是独立普通文件")
    return path


def verify_workdir(path):
    require(path.is_absolute() and not path.is_symlink(), "测试轮目录不能是相对路径或符号链接")
    require(path.resolve(strict=True) == path, "测试轮目录必须使用真实路径")
    expected_root = Path("/Users/zhishi/InfiniShell-Tests").resolve(strict=True)
    users = Path("/Users").stat()
    require(path.parent == expected_root and re.fullmatch(r"r-[a-z0-9]{4,16}", path.name),
            "测试轮目录必须是固定根下的短 r- 名称")
    for ancestor in (Path("/Users"), Path("/Users/zhishi"), expected_root, path):
        require(not ancestor.is_symlink(), "测试路径祖先不能是符号链接")
        info = ancestor.stat()
        require(stat.S_ISDIR(info.st_mode) and info.st_dev == users.st_dev,
                "测试路径不在 /Users 所在设备")
        require(info.st_mode & 0o022 == 0, "测试路径祖先可由组或其他用户写入")
    info = path.stat()
    require(info.st_uid == os.getuid() and info.st_mode & 0o077 == 0,
            "测试轮目录所有权或私有权限不符")
    return path


def project_public(events):
    fields = {
        "parent_started": ("parent_task_id", "policy", "target_absent", "target_ref", "content_sha256"),
        "parent_spawn_approved": ("approval_id", "turn_id", "decision"),
        "parent_spawn_called": ("call_id", "turn_id", "tool"),
        "child_turn_started": ("task_id", "turn_id", "native_session_id"),
        "child_write_pending": ("task_id", "generation", "approval_id", "turn_id", "tool_use_id",
                                "exact_write_verified", "target_absent", "write_allowed"),
        "child_interrupt_submitted": ("task_id", "generation", "turn_id", "message_id",
                                      "approval_id", "write_allowed"),
        "child_interrupt_ack": ("message_id", "turn_id", "native_session_id"),
        "child_write_approval_cancelled": ("approval_id", "turn_id", "native_session_id",
                                           "target_absent"),
        "late_allow_submitted": ("approval_id", "message_id", "decision", "same_cancelled_request_id"),
        "late_allow_rejected": ("approval_id", "message_id", "reason_sha256",
                                "native_request_failed", "target_absent"),
        "child_turn_cancelled": ("task_id", "turn_id", "native_session_id", "outcome",
                                 "target_absent"),
        "saved_cancel_chain_verified": ("parent_task_id", "child_task_id", "parent_generation",
                                        "child_generation", "parent_native_session_id",
                                        "child_native_session_id", "same_fixed_profile",
                                        "parent_ceiling_saved", "child_state", "late_allow_rejected",
                                        "target_absent"),
    }
    return [{"event": kind, **{key: row[key] for key in fields[kind]}}
            for kind in EXPECTED_EVENTS for row in [once(events, kind)]]


def audit(events, test_exit, test_output, target):
    require(test_exit == 0 and re.search(
        r"test result: ok\. 1 passed; 0 failed; 0 ignored;", test_output),
        "真实 Rust libtest 没有成功")
    require(events and events[0].get("event") == "acceptance_started"
            and events[0].get("scope") == SCOPE
            and events[-1].get("event") == "acceptance_passed"
            and events[-1].get("scope") == SCOPE
            and not any(row.get("event") in {"acceptance_failed", "cleanup_failed"} for row in events),
            "真实验收首尾或失败状态不符")
    for key in ("waiting_write_cancel_verified", "late_allow_rejected", "target_file_unchanged",
                "parent_child_ceiling_verified", "all_runtime_hosts_cleaned"):
        require(events[-1].get(key) is True, f"验收缺少 {key}")
    receipt = {kind: once(events, kind) for kind in EXPECTED_EVENTS}
    positions = {kind: events.index(row) for kind, row in receipt.items()}
    require(positions["parent_started"] < positions["parent_spawn_approved"]
            < positions["parent_spawn_called"] < positions["child_turn_started"]
            < positions["child_write_pending"]
            < positions["child_interrupt_submitted"] < positions["child_write_approval_cancelled"]
            < positions["late_allow_submitted"] < positions["late_allow_rejected"]
            < positions["saved_cancel_chain_verified"]
            and positions["child_interrupt_submitted"] < positions["child_interrupt_ack"]
            < positions["saved_cancel_chain_verified"]
            and positions["child_write_approval_cancelled"] < positions["child_turn_cancelled"]
            < positions["saved_cancel_chain_verified"],
            "父子审批和取消收据时序不符")
    pending = receipt["child_write_pending"]
    interrupted = receipt["child_interrupt_submitted"]
    cancelled = receipt["child_write_approval_cancelled"]
    late = receipt["late_allow_submitted"]
    rejected = receipt["late_allow_rejected"]
    ended = receipt["child_turn_cancelled"]
    saved = receipt["saved_cancel_chain_verified"]
    require(receipt["child_turn_started"]["turn_id"] == pending["turn_id"]
            and receipt["child_turn_started"]["task_id"] == pending["task_id"]
            and pending["exact_write_verified"] is True and pending["write_allowed"] is False
            and interrupted["write_allowed"] is False and pending["target_absent"] is True
            and pending["approval_id"] == interrupted["approval_id"] == cancelled["approval_id"]
            == late["approval_id"] == rejected["approval_id"]
            and pending["turn_id"] == interrupted["turn_id"] == cancelled["turn_id"]
            == ended["turn_id"] and interrupted["message_id"] == receipt["child_interrupt_ack"]["message_id"]
            and late["message_id"] == rejected["message_id"]
            and late["same_cancelled_request_id"] is True
            and rejected["native_request_failed"] is True
            and all(row["target_absent"] is True for row in (cancelled, rejected, ended, saved))
            and ended["outcome"] == "Cancelled" and saved["child_state"] == "cancelled"
            and saved["same_fixed_profile"] is True and saved["parent_ceiling_saved"] is True
            and saved["parent_native_session_id"] != saved["child_native_session_id"]
            and not target.exists() and not target.is_symlink(),
            "审批、取消、迟到允许拒绝或文件终态关联不符")
    runtime = [row for row in events if row.get("event") == "runtime"]
    child = saved["child_task_id"]
    require(sum(row["task"]["task_id"] == child and "ApprovalRequested" in row["runtime"]["kind"]
                for row in runtime) == 1
            and sum(row["task"]["task_id"] == child and "ApprovalCancelled" in row["runtime"]["kind"]
                    for row in runtime) == 1
            and sum(row["task"]["task_id"] == child and "ApprovalResolved" in row["runtime"]["kind"]
                    for row in runtime) == 0
            and sum(row["task"]["task_id"] == child
                    and row["runtime"]["kind"].get("RequestFailed", {}).get("message_id")
                    == rejected["message_id"] for row in runtime) == 1
            and sum(row["task"]["task_id"] == child
                    and row["runtime"]["kind"].get("TurnFinished", {}).get("turn_id")
                    == ended["turn_id"] and row["runtime"]["kind"]["TurnFinished"].get("outcome")
                    == "Cancelled" for row in runtime) == 1,
            "原生子审批、迟到拒绝或取消终态数量不符")
    cleanup = [row for row in events if row.get("event") == "cleanup_confirmed"]
    require(len(cleanup) == 2 and {row["task_id"] for row in cleanup}
            == {saved["parent_task_id"], child}, "父子宿主清理收据数量不符")
    for row in cleanup:
        host = row["receipt"]
        require(host["native_process"] == "exited" and host["adapter_task_terminated"] is True
                and host["event_journal_completed"] is True and host["native_cleanup_sha256"],
                "真实原生退出、适配器或日志收据不完整")
    return receipt, cleanup


def run(args):
    require(sys.platform == "darwin", "固定 2.1.280 取消验收只在 macOS 运行")
    require(args.model == "claude-opus-5-5", "V2 取消验收只接受已校准的固定模型")
    workdir = verify_workdir(args.workdir)
    repository = Path(__file__).resolve().parents[2]
    require(args.output.suffix == ".json" and not args.output.exists()
            and args.output.parent.is_dir() and not args.output.parent.is_symlink()
            and not args.output.parent.resolve().is_relative_to(repository),
            "安全收据必须写入新建的仓外 JSON 文件")
    test_binary, claude, supervisor = [private_path(path) for path in
                                       (args.test_binary, args.claude, args.supervisor)]
    require(test_binary != supervisor and claude != supervisor, "测试和监督入口不能重用")
    binary = verify_binary(claude, current_platform(), VERSION)
    version = verify_version(claude, workdir, VERSION)
    test_binary_digest = sha256(test_binary)
    supervisor_digest = sha256(supervisor)
    signature = subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(supervisor)],
                               capture_output=True, timeout=10)
    require(signature.returncode == 0, "生产 supervisor 签名核对失败")
    root = workdir / "g10-v2-cancel"
    root.mkdir(mode=0o700)
    settings = base.prepare_project(root)
    settings.write_text(json.dumps({"permissions": {"ask": ["Write"]}}) + "\n", encoding="utf-8")
    settings_digest = sha256(settings)
    (root / ".infinishell-claude-v2-cancel-probe").write_text(SCOPE, encoding="utf-8")
    raw = root / "v2-cancel.raw.ndjson"
    raw.touch(mode=0o600, exist_ok=False)
    environment = base.authorized_default_account_environment(root)
    base.probe_authorized_default_account(claude, environment.copy(), root / "project")
    profile = "claude-g10-cancel-" + uuid.uuid4().hex
    environment.update({
        "WARP_DATA_PROFILE": profile,
        "INFINISHELL_CLAUDE_LIVE_ROOT": str(root),
        "INFINISHELL_CLAUDE_LIVE_AUTH_MODE": "authorized_default_account",
        "INFINISHELL_CLAUDE_LIVE_EXECUTABLE": str(claude),
        "INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION": VERSION,
        "INFINISHELL_CLAUDE_LIVE_STATE_PROFILE": profile,
        "INFINISHELL_CLAUDE_LIVE_ARTIFACT": str(raw),
        "INFINISHELL_CLAUDE_LIVE_MODEL": args.model,
        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(supervisor),
    })
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repository,
                                     text=True).strip()
    process = subprocess.Popen(
        [str(test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
        cwd=repository, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, encoding="utf-8", errors="replace")
    timed_out = False
    try:
        output, _ = process.communicate(timeout=480)
    except subprocess.TimeoutExpired:
        timed_out = True
        process.kill()
        output, _ = process.communicate(timeout=20)
    redacted = base.sanitize(output, root, None, None)
    events = [json.loads(line) for line in raw.read_text(encoding="utf-8").splitlines() if line.strip()]
    passed = False
    error = None
    try:
        require(not timed_out, "真实 libtest 超时")
        receipt, cleanup = audit(events, process.returncode, redacted,
                                 root / "project/cancelled-write.txt")
        require(sha256(test_binary) == test_binary_digest
                and sha256(supervisor) == supervisor_digest
                and sha256(claude) == binary["sha256"],
                "真实验收期间输入二进制发生变化")
        require(sha256(settings) == settings_digest, "隔离项目的审批设置在验收中改变")
        passed = True
    except (ValueError, KeyError, TypeError) as failure:
        error = str(failure)
        receipt, cleanup = {}, []
    summary = {
        "scope": SCOPE, "passed": passed, "error": error,
        "source_commit": commit, "source_dirty": bool(subprocess.check_output(
            ["git", "status", "--porcelain"], cwd=repository)),
        "platform": sys.platform, "cli_version": version, "cli_binary": binary,
        "model": args.model, "authorized_default_account_verified": True,
        "test_binary_sha256": test_binary_digest, "supervisor_sha256": supervisor_digest,
        "project_settings_sha256": settings_digest,
        "project_settings_unchanged": sha256(settings) == settings_digest,
        "raw_event_sha256": sha256(raw),
        "raw_event_count": len(events), "test_exit_code": process.returncode,
        "test_output_sha256": hashlib.sha256(redacted.encode()).hexdigest(),
        "timed_out": timed_out, "profile": profile,
        "safe_events": project_public(events) if passed else [],
        "cleanup": [{"task_id": row["task_id"], "receipt": row["receipt"]} for row in cleanup],
        "target_absent_after_test": not (root / "project/cancelled-write.txt").exists(),
        "private_tree_preserved": True, "credential_files_read_by_runner": False,
        "real_gui_verified": False, "http_request_count_verified": False,
    }
    serialized = json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    require(base.sanitize(serialized, root, None, None) == serialized,
            "公开收据包含私有目录或身份值")
    args.output.write_text(serialized, encoding="utf-8")
    args.output.chmod(0o600)
    print("Claude V2 父子待审批 Write 取消" + ("通过" if passed else "未通过"))
    print(f"安全收据：{args.output}")
    print(f"测试轮目录保留：{workdir}")
    return 0 if passed else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--workdir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"验收未启动或收据写入失败：{type(error).__name__}: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
