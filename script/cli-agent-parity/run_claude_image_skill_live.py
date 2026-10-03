#!/usr/bin/env python3
"""固定 Claude 图片加单技能的真实生产适配器与逐次审批验收。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile

from prepare_claude_cli import current_platform, verify_binary, verify_version
from run_claude_adapter_live import authorized_default_account_environment, prepare_project, sanitize


TEST = "ai::cli_agent_runtime::claude::live_tests::image_skill_live_tests::real_claude_managed_image_skill_resume"
MULTI_TEST = "ai::cli_agent_runtime::claude::live_tests::image_skill_live_tests::real_claude_managed_image_multi_skill_resume"
COMMAND = "infinishell-local-skills:inspect-managed-image"
MULTI_COMMANDS = ["infinishell-local-skills:image-alpha", "infinishell-local-skills:image-beta"]
REPOSITORY = Path(__file__).resolve().parents[2]


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def source_binding():
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=REPOSITORY, text=True, encoding="utf-8")
    paths = ["app/src/ai/cli_agent_runtime/claude.rs",
             "app/src/ai/cli_agent_runtime/claude_live_tests.rs",
             "app/src/ai/cli_agent_runtime/claude_image_skill_live_tests.rs",
             "app/src/ai/cli_agent_runtime/managed_input.rs",
             "app/src/ai/cli_agent_runtime/local_skills.rs"]
    return {"source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPOSITORY, text=True, encoding="utf-8").strip(),
            "source_worktree_dirty": bool(status.strip()), "source_commit_is_baseline_only": bool(status.strip()),
            "source_status_porcelain": status, "runner_sha256": digest(Path(__file__)),
            "critical_source_sha256": {name: digest(REPOSITORY / name) for name in paths}}


def verify_native_receipts(events, traces, commands=None):
    commands = [COMMAND] if commands is None else commands
    ready = [event for event in events if event.get("event") == "ready"]
    prepared = [event for event in events if event.get("event") == "attachment_prepared"]
    approvals = [event for event in events if event.get("event") == "skill_approval"]
    resolutions = [event for event in events if event.get("event") == "skill_approval_resolved"]
    dispatches = [event for event in events if event.get("event") == "skill_response_dispatched"]
    results = [event for event in events if event.get("event") == "finished"]
    cleanup = [event for event in events if event.get("event") == "cleanup"]
    passed = (len(ready) == len(prepared) == len(results) == len(cleanup) == 2
              and len(approvals) == len(resolutions) == len(dispatches) == 2 * len(commands))
    checks = []
    for index, (result, image) in enumerate(zip(results, prepared)):
        native = [trace for trace in traces if trace.get("runtime_generation") == result["generation"]]
        replay = [trace for trace in native if trace.get("type") == "user"
                  and trace.get("uuid") == result["turn_id"]
                  and trace.get("session_id") == result["native_session_id"]
                  and trace.get("rich_image_content_projection") is not None]
        matching = [event for event in approvals if event.get("generation") == result["generation"]
                    and event.get("native_session_id") == result["native_session_id"]
                    and event.get("turn_id") == result["turn_id"]]
        ordered_approvals = len(matching) == len(commands)
        for position, approval in enumerate(matching):
            resolved = [event for event in resolutions if event.get("generation") == result["generation"]
                        and event.get("native_session_id") == result["native_session_id"]
                        and event.get("turn_id") == result["turn_id"]
                        and event.get("approval_id") == approval.get("approval_id")
                        and event.get("decision") == "AllowOnce"]
            dispatched = [event for event in dispatches if event.get("generation") == result["generation"]
                          and event.get("native_session_id") == result["native_session_id"]
                          and event.get("turn_id") == result["turn_id"]
                          and event.get("control_message_id") == approval.get("control_message_id")]
            ordered_approvals = ordered_approvals and len(resolved) == len(dispatched) == 1
            if ordered_approvals:
                ordered_approvals = (events.index(approval) < events.index(resolved[0])
                                     and events.index(approval) < events.index(dispatched[0]))
                if position + 1 < len(matching):
                    ordered_approvals = (ordered_approvals
                                         and events.index(resolved[0]) < events.index(matching[position + 1])
                                         and events.index(dispatched[0]) < events.index(matching[position + 1]))
        tools = [(trace, tool) for trace in native
                 for tool in trace.get("image_skill_projection", {}).get("tools", [])]
        registered = any(
            projection.get("selected_command_registered") is True if commands == [COMMAND]
            else (isinstance(projection.get("selected_commands_registered"), list)
                  and len(projection["selected_commands_registered"]) == len(commands)
                  and set(projection["selected_commands_registered"]) == set(commands))
            for trace in native
            for projection in [trace.get("image_skill_projection", {})]
        )
        model_verified = any(trace.get("image_skill_projection", {}).get("native_model_matches_fixture") is True
                             for trace in native)
        image_match = len(replay) == 1 and replay[0]["rich_image_content_projection"] == {
            "array_sha256": image["native_array_sha256"],
            "array_bytes": replay[0]["rich_image_content_projection"]["array_bytes"],
            "images": [{"image_sha256": image["image_sha256"], "image_bytes": image["image_bytes"],
                        "media_type": image["media_type"]}]}
        skill_match = (len(matching) == len(tools) == len(commands)
                       and all(
                           approval["exact_registered_command"] is True
                           and approval["decision"] == "AllowOnce"
                           and tool_trace.get("session_id") == result["native_session_id"]
                           and tool.get("selected_skill_command") == command
                           and tool["tool_use_id"] == approval["tool_use_id"]
                           and tool["input_sha256"] == approval["input_sha256"]
                           for approval, (tool_trace, tool), command in zip(matching, tools, commands)))
        successful_result = (result["outcome"] == "Completed" and result["result"].strip() == result["expected"]
                             and result["skill_approval_resolved"] and result["skill_response_dispatched"]
                             and result.get("approved_commands") == commands)
        prepared_match = (image.get("skill_commands") == commands
                          and image.get("resumed") is (index == 1))
        ready_match = (any(event.get("generation") == result["generation"]
                           and (event.get("selected_skill_command") == COMMAND if commands == [COMMAND]
                                else event.get("selected_skill_commands") == commands)
                           for event in ready))
        passed = (passed and registered and model_verified and image_match and skill_match
                  and ordered_approvals
                  and successful_result and prepared_match and ready_match)
        checks.append({"generation": result["generation"], "native_session_id": result["native_session_id"],
                       "turn_id": result["turn_id"], "registered": registered, "native_model_verified": model_verified,
                       "image_bytes_and_array_match": image_match, "approved_tool_identity_match": skill_match,
                       "approval_resolved_and_dispatched_before_next": ordered_approvals,
                       "correct_image_and_skill_result": successful_result,
                       "prepared_skills_match": prepared_match, "ready_skills_match": ready_match})
    passed = (passed and events and events[-1].get("event") == "acceptance_passed"
              and events[-1].get("native_session_id") == results[0]["native_session_id"]
              and results[0]["native_session_id"] == results[1]["native_session_id"]
              and results[0]["generation"] != results[1]["generation"]
              and prepared[0]["image_sha256"] != prepared[1]["image_sha256"]
              and all(event["natural_exit"] and event["cleanup_confirmed"]
                      and event["exit_code"] == 0
                      and event.get("generation") in [result["generation"] for result in results]
                      and event.get("native_session_id") == results[0]["native_session_id"]
                      for event in cleanup)
              and len({event["generation"] for event in cleanup}) == 2
              and (commands == [COMMAND] or all(event.get("skills_per_input") == 2
                                                for event in events if event.get("event") == "acceptance_passed")))
    return bool(passed), checks


def private_macos_root(path):
    base = Path("/Users/zhishi/InfiniShell-Tests")
    if path is None or path.parent != base or not path.name.startswith("r-"):
        raise ValueError("macOS 图片技能在线验收须提供本机固定根下的新轮次目录")
    users = Path("/Users")
    device = users.stat().st_dev
    for directory in (Path("/"), users, Path("/Users/zhishi"), base, path):
        metadata = directory.lstat()
        if (not stat.S_ISDIR(metadata.st_mode) or directory.is_symlink()
                or metadata.st_mode & 0o022
                or directory in (base, path) and metadata.st_dev != device):
            raise ValueError("验收目录或祖先身份、权限、设备不符合本机测试约定")
    base_info, run_info = base.stat(), path.stat()
    if (base_info.st_uid != os.getuid() or run_info.st_uid != os.getuid()
            or stat.S_IMODE(base_info.st_mode) != 0o700
            or stat.S_IMODE(run_info.st_mode) != 0o700):
        raise ValueError("验收根与轮次目录须归当前用户所有且权限为 0700")
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--multi", action="store_true", help="验收图片加两个技能的新建与冷恢复")
    parser.add_argument("--private-root", type=Path, help="本机 macOS 图片单/双技能验收的已登记 r- 轮次目录")
    args = parser.parse_args()
    executable = args.executable.resolve(strict=True)
    native = verify_binary(executable, current_platform(), "2.1.280")
    args.output.mkdir(parents=True, exist_ok=False)
    if current_platform().startswith("darwin-"):
        root = private_macos_root(args.private_root)
    else:
        root = Path(tempfile.mkdtemp(prefix="infinishell-claude-image-skill-",
                                     dir="/private/tmp" if Path("/private/tmp").is_dir() else None))
    verify_version(executable, root, "2.1.280")
    settings = prepare_project(root)
    # 只修改本次私有项目；原生仍须逐次询问 Skill，其他工具和自动 hooks 不属于校准范围。
    settings.write_text(json.dumps({"disableAllHooks": True, "enableAllProjectMcpServers": False,
        "permissions": {"defaultMode": "default", "ask": ["Skill"],
                        "deny": ["Bash", "Read", "Edit", "Write", "Grep", "Glob", "WebFetch", "WebSearch",
                                 "Agent", "Task", "NotebookEdit", "TodoWrite", "mcp__*"]}}) + "\n")
    settings_hash = digest(settings)
    marker = (".infinishell-claude-image-multi-skill-probe" if args.multi
              else ".infinishell-claude-image-skill-probe")
    (root / marker).write_text(
        "isolated production Claude image and two selected skills verification\n" if args.multi
        else "isolated production Claude image and selected skill verification\n")
    environment = authorized_default_account_environment(root)
    if current_platform().startswith("darwin-"):
        environment.update({"TMPDIR": str(root), "TMP": str(root), "TEMP": str(root)})
    environment.update({"INFINISHELL_CLAUDE_LIVE_ROOT": str(root),
        "INFINISHELL_CLAUDE_LIVE_AUTH_MODE": "authorized_default_account",
        "INFINISHELL_CLAUDE_LIVE_EXECUTABLE": str(executable),
        "INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION": "2.1.280",
        "INFINISHELL_CLAUDE_LIVE_ARTIFACT": str(args.output.resolve() / "events.ndjson"),
        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor.resolve()),
        "INFINISHELL_CLAUDE_LIVE_MODEL": "claude-opus-5-5",
        "INFINISHELL_CLAUDE_IMAGE_SKILL_TRACE": "1",
        "INFINISHELL_CLAUDE_RICH_IMAGE_CASE": "image-skill"})
    receipt = {"scope": "production_adapter_image_and_two_registered_skills_new_and_cold_resume" if args.multi
               else "production_adapter_image_and_registered_skill_new_and_cold_resume",
               "cli": native, "model": "claude-opus-5-5", "workspace": str(root),
               "test_binary_sha256": digest(args.test_binary), "supervisor_sha256": digest(args.supervisor),
               "credential_material_read": False, "app_restart_or_gui_verified": False, "passed": False}
    receipt.update(source_binding())
    try:
        process = subprocess.run([str(args.test_binary.resolve()), MULTI_TEST if args.multi else TEST,
                                  "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                 cwd=REPOSITORY, env=environment, capture_output=True, text=True, encoding="utf-8", timeout=510)
    except subprocess.TimeoutExpired as error:
        receipt["timed_out"] = True
        process = subprocess.CompletedProcess([], -1,
            (error.stdout or b"").decode("utf-8", "replace"), (error.stderr or b"").decode("utf-8", "replace"))
    output = sanitize(process.stdout + process.stderr, root, None, None)
    (args.output / "test-output.txt").write_text(output, encoding="utf-8", newline="\n")
    evidence = args.output / "events.ndjson"
    events = [json.loads(line) for line in evidence.read_text(encoding="utf-8").splitlines()] if evidence.exists() else []
    traces = [json.loads(line.split("CLAUDE_NATIVE_PROTOCOL_IDS ", 1)[1])
              for line in output.splitlines() if "CLAUDE_NATIVE_PROTOCOL_IDS " in line]
    (args.output / "native-projections.json").write_text(json.dumps(traces, indent=2) + "\n", encoding="utf-8", newline="\n")
    verified, checks = verify_native_receipts(events, traces, MULTI_COMMANDS if args.multi else None)
    receipt.update(exit_code=process.returncode, native_receipt_checks=checks,
                   native_receipts_passed=verified, private_settings_unchanged=digest(settings) == settings_hash,
                   critical_sources_unchanged=receipt["critical_source_sha256"] == source_binding()["critical_source_sha256"],
                   native_binary_unchanged=verify_binary(executable, current_platform(), "2.1.280") == native)
    receipt["passed"] = (verified and process.returncode == 0 and "1 passed" in output
                         and receipt["private_settings_unchanged"] and receipt["critical_sources_unchanged"]
                         and receipt["native_binary_unchanged"])
    (args.output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"output": str(args.output), "passed": receipt["passed"]}))
    if not receipt["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
