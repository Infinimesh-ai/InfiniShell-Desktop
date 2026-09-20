#!/usr/bin/env python3
"""复用 Claude 适配器运行器，显式用 API 验收固定文件策略；不读取原生登录资料。"""

import argparse
import json
from pathlib import Path
import re
import tempfile

import run_claude_adapter_live as adapter
from prepare_claude_cli import RELEASE_CATALOG, VERSION, release_contract


TEST_NAME = ("ai::cli_agent_runtime::claude::live_tests::profile_live_tests::"
             "real_claude_restricted_files_lifecycle")
MARKER = "isolated Claude restricted files profile verification\n"
SCOPE = "claude_restricted_files_process_restart"
PROJECT_SETTINGS = {"permissions": {"ask": ["Edit"]}}
PREPARE_BASE_PROJECT = adapter.prepare_project


def prepare_project(root):
    settings = PREPARE_BASE_PROJECT(root)
    (root / ".infinishell-claude-profile-probe").write_text(MARKER, encoding="utf-8", newline="\n")
    (root / "outside").mkdir(mode=0o700)
    for name in ("edit-allow.txt", "edit-deny.txt"):
        (root / "project" / name).write_text("PROFILE_BEFORE", encoding="utf-8", newline="\n")
    return settings


def verified_acceptance(exit_code, output, events):
    if exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return False
    if any(not isinstance(event, dict) or event.get("event") == "acceptance_failed" for event in events):
        return False
    endings = [event for event in events if event.get("event") == "acceptance_passed"]
    if len(endings) != 1:
        return False
    ending = endings[0]
    native_id, profile_hash = ending.get("native_session_id"), ending.get("profile_sha256")
    if (ending.get("scope") != SCOPE or not isinstance(native_id, str) or not native_id
            or not isinstance(profile_hash, str) or not re.fullmatch(r"[0-9a-f]{64}", profile_hash)
            or ending.get("same_profile_restored") is not True
            or ending.get("edit_allow_and_deny_verified") is not True
            or ending.get("resume_escalation_rejections") != 3
            or any(ending.get(key) is not False for key in (
                "filesystem_sandbox_verified", "app_restart_and_ui_verified",
                "native_mid_turn_mode_change_verified", "outside_directory_tool_execution_verified"))):
        return False
    profiles = [event for event in events if event.get("event") == "profile_verified"]
    if len(profiles) < 2 or any(event.get("profile_sha256") != profile_hash
            or event.get("permission_mode") != "plan" or event.get("fixed_profile_verified") is not True
            or event.get("native_session_id") not in (None, native_id) for event in profiles):
        return False
    restored = [event for event in events if event.get("event") == "same_profile_resume_verified"]
    if (len(restored) != 1 or restored[0].get("profile_sha256") != profile_hash
            or restored[0].get("native_session_id") != native_id
            or restored[0].get("saved_profile_reloaded") is not True or not restored[0].get("marker")):
        return False
    expected = {"first_turn": "PROFILE_ONE", "second_turn": "PROFILE_TWO", "edit_allow": "EDIT_ALLOWED",
                "edit_deny": "EDIT_DENIED", "resume_result": restored[0]["marker"]}
    results = [event for event in events if event.get("event") == "turn_finished"]
    if len(results) != 5 or {event.get("phase") for event in results} != set(expected):
        return False
    ids = [event.get("turn_id") for event in results]
    if any(not isinstance(value, str) or not value for value in ids) or len(set(ids)) != 5:
        return False
    for result in results:
        phase, turn_id = result["phase"], result["turn_id"]
        if (result.get("outcome") != "Completed" or result.get("output", "").strip() != expected[phase]
                or result.get("native_session_id") != native_id):
            return False
        if not any(event.get("event") == "message_accepted" and event.get("phase") == phase
                   and event.get("message_id") == turn_id and event.get("turn_id") == turn_id
                   and event.get("native_session_id") == native_id for event in events):
            return False
        if sum(event.get("event") == "turn_started" and event.get("phase") == phase
               and event.get("turn_id") == turn_id and event.get("native_session_id") == native_id for event in events) != 1:
            return False
    if any(sum(event.get("event") == kind for event in events) != 2
           for kind in ("approval_requested", "approval_resolved", "file_effect_verified")):
        return False
    for phase, allowed, decision, content in (("edit_allow", True, "AllowOnce", "PROFILE_AFTER"),
                                             ("edit_deny", False, "DenyOnce", "PROFILE_BEFORE")):
        requests = [event for event in events if event.get("event") == "approval_requested" and event.get("phase") == phase]
        if len(requests) != 1 or requests[0].get("exact_edit_fixture") is not True or requests[0].get("decision") != decision:
            return False
        resolutions = [event for event in events if event.get("event") == "approval_resolved" and event.get("phase") == phase]
        if (len(resolutions) != 1 or resolutions[0].get("approval_id") != requests[0].get("approval_id")
                or resolutions[0].get("decision") != decision or resolutions[0].get("native_receipt") is not False):
            return False
        effects = [event for event in events if event.get("event") == "file_effect_verified" and event.get("phase") == phase]
        if len(effects) != 1 or effects[0].get("allowed") is not allowed or effects[0].get("expected_content") != content:
            return False
    rejections = [event for event in events if event.get("event") == "resume_rejected"]
    reasons = {"changed_directory": "claude_profile_identity_invalid", "changed_policy": "claude_profile_wrong_policy",
               "expanded_source_rules": "claude_profile_source_changed"}
    if len(rejections) != 3 or {event.get("phase") for event in rejections} != set(reasons):
        return False
    for event in rejections:
        reason = reasons[event["phase"]]
        if (event.get("rejected") is not True or event.get("user_input_sent") is not False
                or event.get("expected_reason") != reason or not isinstance(event.get("details"), dict)
                or event["details"].get("reason") != reason):
            return False
    shutdowns = [event for event in events if event.get("event") == "connection_shutdown"]
    return len(shutdowns) == 2 and all(event.get("native_session_id") == native_id for event in shutdowns)


def run(args):
    # 兼容旧 Namespace；未知版本在配置准备、API 读取和原生执行之前拒绝。
    release_contract(getattr(args, "claude_version", VERSION))
    if args.api_environment_file is None:
        raise ValueError("固定策略使用 --bare，必须显式提供 API 环境文件")
    # 新配置域与实际登录域完全分开；会话历史保留在该私有目录供失败诊断。
    private = Path(tempfile.mkdtemp(prefix="infinishell-claude-profile-auth-")).resolve()
    args.config_dir, args.auth_home = private / "claude", private / "home"
    args.config_dir.mkdir(mode=0o700)
    args.auth_home.mkdir(mode=0o700)
    adapter.validate_paths(args)
    # 复用既有运行链，仅替换测试选择、项目夹具和验收谓词；返回后恢复，避免污染其他调用。
    replacements = {"TEST_NAME": TEST_NAME, "PROJECT_SETTINGS": PROJECT_SETTINGS,
                    "prepare_project": prepare_project, "verified_acceptance": verified_acceptance}
    original = {key: getattr(adapter, key) for key in replacements}
    try:
        for key, value in replacements.items():
            setattr(adapter, key, value)
        result = adapter.run(args)
    finally:
        for key, value in original.items():
            setattr(adapter, key, value)
    path = args.output.with_suffix(".metadata.json")
    metadata = json.loads(path.read_text(encoding="utf-8"))
    metadata.update({"scope": SCOPE, "permission_policy": "ClaudeRestrictedFilesV1",
                     "fresh_private_auth_home": True, "authentication_source": "explicit_api_environment",
                     "private_auth_workspace": str(private), "same_turn_steering_supported": False,
                     "parent_permission_ceiling_verified": False,
                     "native_mid_turn_mode_change_verified": False,
                     "outside_directory_tool_execution_verified": False})
    path.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claude-version", choices=tuple(RELEASE_CATALOG), default=VERSION,
                        help="精确官方版本；缺省保留 2.1.273")
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--claude", type=Path, required=True, help="所选固定官方版本的原生可执行文件")
    parser.add_argument("--supervisor", type=Path, required=True, help="同提交主程序或 TUI 监督入口")
    parser.add_argument("--api-environment-file", type=Path, required=True, help="显式私有 JSON，沿用 Anthropic API 环境白名单")
    parser.add_argument("--model", help="可选原生模型，例如 claude-sonnet-4-6")
    parser.add_argument("--output", type=Path, required=True, help="全新 .ndjson 证据路径")
    args = parser.parse_args()
    try:
        return run(args)
    except (OSError, ValueError, adapter.subprocess.SubprocessError) as error:
        parser.exit(2, f"固定策略运行器启动失败：{type(error).__name__}；请检查显式文件参数。\n")


if __name__ == "__main__":
    raise SystemExit(main())
