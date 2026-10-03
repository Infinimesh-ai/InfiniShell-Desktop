#!/usr/bin/env python3
"""在两个独立应用进程间验收固定 Claude 父子 Skill 待审批冷恢复。"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import uuid

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_claude_adapter_live as base
from prepare_claude_cli import current_platform, verify_binary, verify_version
from run_claude_v2_write_cancel_live import verify_workdir

TEST_BASE = "ai::cli_agent_runtime::coordinator::g10_cold_recovery_live_tests::"
PREPARE = TEST_BASE + "real_claude_g10_cold_prepare_pending_skill"
RECOVER = TEST_BASE + "real_claude_g10_cold_recover_pending_skill"
CLEANUP = TEST_BASE + "real_claude_g10_cold_cleanup_after_failure"
SCOPE = "real_claude_g10_skill_parent_child_cold_recovery"
VERSION = "2.1.280"
MODEL = "claude-opus-5-5"


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def save_record(path, value):
    temporary = path.with_suffix(".json.tmp")
    with temporary.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False)
        stream.write("\n")
    temporary.chmod(0o600)
    os.replace(temporary, path)


def start_record(workdir):
    records = workdir.parent / "records"
    require(not records.is_symlink() and records.is_dir(), "缺少固定根下的逐轮记录目录")
    info = records.stat()
    require(info.st_uid == os.getuid() and info.st_dev == workdir.stat().st_dev
            and info.st_mode & 0o077 == 0, "逐轮记录目录身份或权限不符")
    path = records / f"{workdir.name}.json"
    require(not path.exists() and not path.is_symlink(), "本轮记录已存在，禁止复用测试目录")
    directory = workdir.stat()
    value = {
        "run_id": workdir.name,
        "started_at": datetime.now(timezone.utc).isoformat(),
        "finished_at": None,
        "status": "running",
        "command": [PREPARE, RECOVER],
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO,
                                                 text=True).strip(),
        "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=REPO)),
        "temporary_directory": str(workdir),
        "temporary_realpath": str(workdir.resolve(strict=True)),
        "directory_device": directory.st_dev,
        "directory_inode": directory.st_ino,
        "directory_owner": directory.st_uid,
        "directory_mode": directory.st_mode & 0o777,
        "main_pid": os.getpid(),
        "process_started_at": subprocess.check_output(
            ["/bin/ps", "-p", str(os.getpid()), "-o", "lstart="], text=True).strip(),
        "test_processes": [],
        "launchd_labels": [],
        "archive_log": None,
        "archive_sha256": None,
        "cleanup_ready": False,
    }
    save_record(path, value)
    return path, value


def require(condition, message):
    if not condition:
        raise ValueError(message)


def one(events, kind):
    rows = [row for row in events if row.get("event") == kind]
    require(len(rows) == 1, f"{kind} 收据数量不符")
    return rows[0]


def run_test(binary, name, environment, output, timeout):
    with output.open("xb") as stream:
        process = subprocess.Popen(
            [str(binary), name, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
            cwd=REPO, env=environment, stdout=stream, stderr=subprocess.STDOUT,
            start_new_session=True)
        try:
            started = subprocess.check_output(
                ["/bin/ps", "-p", str(process.pid), "-o", "lstart="], text=True).strip()
        except subprocess.CalledProcessError:
            started = None
        try:
            exit_code = process.wait(timeout=timeout)
            timed_out = False
        except subprocess.TimeoutExpired:
            timed_out = True
            process.kill()
            exit_code = process.wait(timeout=20)
    return {"pid": process.pid, "process_started_at": started,
            "exit_code": exit_code, "timed_out": timed_out,
            "log_sha256": sha(output)}


def audit(root, prepare_result, recover_result):
    prepared = [json.loads(line) for line in (root / "prepare.raw.ndjson").read_text().splitlines()
                if line.strip()]
    recovered = [json.loads(line) for line in (root / "recover.raw.ndjson").read_text().splitlines()
                 if line.strip()]
    for phase, result, events in (("prepare", prepare_result, prepared),
                                   ("recover", recover_result, recovered)):
        require(result is not None and result["exit_code"] == 0 and not result["timed_out"],
                f"{phase} 测试进程未成功")
        require(re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;",
                          (root / f"{phase}.raw.log").read_text(errors="replace")),
                f"{phase} libtest 未通过")
        started = one(events, "acceptance_started")
        require(started["scope"] == SCOPE and started["phase"] == phase,
                f"{phase} 范围不符")
        require(not any(row.get("event", "").endswith("failed") for row in events),
                f"{phase} 存在失败事件")
    require(one(prepared, "prepare_passed")["native_hosts_intentionally_live"] is True,
            "第一进程未保留宿主")
    pending = one(prepared, "child_skill_pending")
    require(pending["parent_ceiling_saved"] is True
            and pending["child_skill_unresolved"] is True,
            "第一进程没有停在真实受限技能待审批点")
    attached = one(recovered, "cold_reattach_verified")
    require(attached["parent_task_id"] == pending["parent_task_id"]
            and attached["child_task_id"] == pending["child_task_id"]
            and all(attached[key] is True for key in
                    ("same_native_sessions", "same_pending_approval",
                     "parent_ceiling_saved", "user_inputs_not_replayed",
                     "app_process_restart_verified")),
            "冷恢复身份、父上限或重投检查不符")
    approved = one(recovered, "recovered_skill_approved")
    finished = one(recovered, "recovered_skill_result_verified")
    require(approved["approval_sha256"] == pending["approval_sha256"]
            and finished["child_task_id"] == pending["child_task_id"]
            and finished["native_skill_resolved"] is True
            and finished["saved_result_contains_marker"] is True
            and finished["new_child_count"] == 0,
            "恢复后审批或技能结果不符")
    cleanup = [row for row in recovered if row.get("event") == "cleanup_confirmed"]
    require(len(cleanup) == 2
            and {row["task_id"] for row in cleanup}
            == {pending["parent_task_id"], pending["child_task_id"]}
            and all(row["native_cleanup_sha256"] for row in cleanup),
            "父子监督退出收据不完整")
    passed = one(recovered, "acceptance_passed")
    require(all(passed[key] is True for key in
                ("app_process_restart_verified", "same_native_sessions",
                 "same_pending_approval", "parent_ceiling_saved",
                 "saved_skill_result_verified", "all_runtime_hosts_cleaned")),
            "冷恢复正例未完整通过")
    checkpoint = json.loads((root / "pending-checkpoint.raw.json").read_text())
    require(checkpoint["scope"] == SCOPE
            and checkpoint["parent_task_id"] == pending["parent_task_id"]
            and checkpoint["child_task_id"] == pending["child_task_id"],
            "阶段间私有检查点不符")
    return len(prepared), len(recovered)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--workdir", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    require(sys.platform == "darwin", "仅支持本机 macOS 验收")
    workdir = verify_workdir(args.workdir)
    output = args.output or workdir / "g10-cold.safe.json"
    require(output.parent == workdir and not output.exists(), "安全收据必须位于本轮新目录")
    sources = (args.test_binary, args.claude, args.supervisor)
    require(all(not path.is_symlink() and path.is_file() for path in sources),
            "二进制来源不允许符号链接")
    source_app = args.supervisor.parents[2]
    require(source_app.suffix == ".app"
            and args.supervisor == source_app / "Contents/MacOS/infinishell",
            "宿主 worker 必须来自已签名应用包")
    inputs = workdir / "inputs"
    inputs.mkdir(mode=0o700)
    test, claude = (inputs / name for name in ("warp-libtest", "claude"))
    for source, target in zip(sources[:2], (test, claude)):
        shutil.copy2(source, target)
        target.chmod(0o700)
    private_app = inputs / "InfiniShellParity.app"
    shutil.copytree(source_app, private_app)
    supervisor = private_app / "Contents/MacOS/infinishell"
    subprocess.run(["/usr/bin/codesign", "-s", "-", "--force", "--timestamp=none", str(test)],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for binary in (test, claude):
        subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(binary)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(private_app)],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    require(sha(claude) == sha(args.claude), "固定 Claude 副本摘要不符")
    native = verify_binary(claude, current_platform(), VERSION)
    version = verify_version(claude, workdir, VERSION)
    root = workdir / "g10-cold"
    root.mkdir(mode=0o700)
    (root / "project" / ".claude").mkdir(parents=True, mode=0o700)
    (root / ".infinishell-claude-g10-cold-probe").write_text(SCOPE)
    for phase in ("prepare", "recover", "cleanup"):
        (root / f"{phase}.raw.ndjson").touch(mode=0o600, exist_ok=False)
    environment = base.authorized_default_account_environment(root)
    environment.update({"TMPDIR": str(workdir), "TMP": str(workdir), "TEMP": str(workdir)})
    auth = base.probe_authorized_default_account(claude, environment.copy(), root / "project")
    profile = "claude-g10-cold-" + uuid.uuid4().hex
    environment.update({
        "WARP_DATA_PROFILE": profile,
        "INFINISHELL_CLAUDE_LIVE_ROOT": str(root),
        "INFINISHELL_CLAUDE_LIVE_AUTH_MODE": "authorized_default_account",
        "INFINISHELL_CLAUDE_LIVE_EXECUTABLE": str(claude),
        "INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION": VERSION,
        "INFINISHELL_CLAUDE_LIVE_STATE_PROFILE": profile,
        "INFINISHELL_CLAUDE_LIVE_MODEL": MODEL,
        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(supervisor),
    })
    record_path, run_record = start_record(workdir)
    prepared = None
    recovered = None
    cleanup = None
    cleanup_verified = False
    failure = None
    counts = None
    try:
        environment["INFINISHELL_CLAUDE_LIVE_ARTIFACT"] = str(root / "prepare.raw.ndjson")
        prepared = run_test(test, PREPARE, environment, root / "prepare.raw.log", 600)
        require(prepared["exit_code"] == 0 and not prepared["timed_out"],
                "准备阶段未成功，保留原生宿主供精确审计")
        environment["INFINISHELL_CLAUDE_LIVE_ARTIFACT"] = str(root / "recover.raw.ndjson")
        recovered = run_test(test, RECOVER, environment, root / "recover.raw.log", 360)
        counts = audit(root, prepared, recovered)
    except (ValueError, KeyError, TypeError, OSError, subprocess.TimeoutExpired) as error:
        failure = str(error)
    if failure is not None:
        environment["INFINISHELL_CLAUDE_LIVE_ARTIFACT"] = str(root / "cleanup.raw.ndjson")
        try:
            cleanup = run_test(test, CLEANUP, environment, root / "cleanup.raw.log", 240)
            cleanup_events = [json.loads(line) for line in
                              (root / "cleanup.raw.ndjson").read_text().splitlines() if line.strip()]
            require(cleanup["exit_code"] == 0 and not cleanup["timed_out"]
                    and one(cleanup_events, "failure_cleanup_passed")
                    ["all_recovered_hosts_exited"] is True
                    and not any(row.get("event") == "failure_cleanup_unconfirmed"
                                for row in cleanup_events),
                    "失败轮宿主清理未确认，必须保留现场并按精确身份处理")
            cleanup_verified = True
        except (ValueError, KeyError, TypeError, OSError, subprocess.TimeoutExpired) as error:
            failure += "; failure_cleanup_unconfirmed: " + str(error)
    summary = {
        "scope": SCOPE, "passed": failure is None, "failure": failure,
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO,
                                                 text=True).strip(),
        "source_dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=REPO)),
        "state_profile": profile, "cli_version": version, "cli_binary_sha256": native["sha256"],
        "model": MODEL, "authorized_account_logged_in": auth["loggedIn"],
        "test_binary_sha256": sha(test), "supervisor_sha256": sha(supervisor),
        "prepare": prepared, "recover": recovered, "failure_cleanup": cleanup,
        "prepare_event_sha256": sha(root / "prepare.raw.ndjson"),
        "recover_event_sha256": sha(root / "recover.raw.ndjson"),
        "checkpoint_sha256": sha(root / "pending-checkpoint.raw.json")
        if (root / "pending-checkpoint.raw.json").exists() else None,
        "event_counts": counts,
        "app_process_restart_verified": failure is None,
        "native_hosts_exited_after_failure": cleanup_verified,
        "real_gui_verified": False,
    }
    output.write_text(json.dumps(summary, indent=2, ensure_ascii=False) + "\n")
    output.chmod(0o600)
    run_record.update({
        "finished_at": datetime.now(timezone.utc).isoformat(),
        "status": "finished_pending_archive" if failure is None else "failed_pending_archive",
        "exit_code": 0 if failure is None else 1,
        "test_processes": [item for item in (prepared, recovered, cleanup) if item is not None],
        "exit_proof": {"normal_chain": failure is None,
                       "failure_cleanup_verified": cleanup_verified,
                       "safe_receipt_sha256": sha(output)},
        "cleanup_ready": False,
    })
    save_record(record_path, run_record)
    print("G10 固定技能父子冷恢复：" + ("通过" if failure is None else "未通过"))
    print("安全收据：" + str(output))
    return 0 if failure is None else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print("G10 冷恢复运行器前置或执行失败：" + type(error).__name__)
        sys.exit(2)
