#!/usr/bin/env python3
"""隔离校准 Claude 等待 Edit 审批时的取消；不放宽生产判据或保存 stdout 正文。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import uuid

import run_claude_adapter_live as adapter
from run_claude_batch_cancel_live import NATIVE_KEYS, validate_cancel_diagnostics, validate_cancel_terminal
from run_claude_managed_image_live import compact_stdout


TEST_NAME = ("ai::cli_agent_runtime::claude::live_tests::batch_cancel_live_tests::"
             "approval_cancel_live_tests::real_claude_pending_edit_cancel")
SCOPE = "rust_adapter_pending_edit_cancel"
PROJECT_SETTINGS = {"permissions": {"ask": ["Edit"]}}
BEFORE = b"APPROVAL_CANCEL_BEFORE\n"
AFTER = b"APPROVAL_CANCEL_AFTER\n"
FILE_REF = "edit-cancel.txt"
PREPARE_BASE_PROJECT = adapter.prepare_project
PHASES = {"pending_edit", "continue"}
RECEIPT_KEYS = {"version", "generation", "containment", "cleanup_confirmed", "exit_code", "exit_reason"}
EVENT_KEYS = {
    "acceptance_started": {"event", "scope", "max_native_inputs", "phase_deadline_seconds", "permission_policy",
                           "credential_files_read_by_probe", "real_gui_verified", "persistence_verified"},
    "profile_verified": {"event", "permission_mode", "fixed_profile_verified", "profile_sha256",
                         "filesystem_sandbox_verified", "parent_permission_ceiling_verified"},
    "file_unchanged": {"event", "phase", "file_ref", "full_bytes_verified", "file_bytes", "file_sha256"},
    "input_submitted": {"event", "phase", "message_id", "submitted_input_count"},
    "message_accepted": {"event", "message_id", "turn_id", "native_session_id", "native_receipt"},
    "turn_started": {"event", "turn_id", "native_session_id"},
    "edit_approval_requested": {"event", "approval_id", "turn_id", "native_session_id", "tool_use_id",
                                "exact_edit_verified", "edit_allowed", "file_ref", "old_string_sha256",
                                "new_string_sha256", "input_keys", "replace_all"},
    "interrupt_submitted": {"event", "message_id", "execution_turn_id", "approval_id",
                            "approval_still_pending", "edit_allowed"},
    "interrupt_accepted": {"event", "message_id", "turn_id", "native_session_id", "native_receipt"},
    "edit_approval_cancelled": {"event", "approval_id", "native_session_id", "native_execution_cancelled_verified"},
    "turn_finished": {"event", "phase", "turn_id", "native_session_id", "outcome", "output_bytes",
                      "full_output_sha256", "trimmed_output_sha256", "output", "same_native_session_verified"},
    "connection_shutdown": {"event", "native_session_id"},
    "cleanup_checked": {"event", "generation", "cleanup_confirmed", "normal_exit", "transport_closed",
                        "cleanup_receipt_read", "receipt"},
    "native_protocol_ids": {"event", "generation", "sequence", "native"},
    "native_pending_edit_cancel_verified": {"event", "native_session_id", "runtime_generation",
        "execution_input_id", "continuation_input_id", "approval_id", "tool_use_id", "interrupt_request_id",
        "native_result_uuid", "continue_result_uuid", "terminal_reason", "is_error", "user_message_uuids",
        "edit_allowed", "read_call_count", "all_four_evidence_verified"},
    "acceptance_passed": {"event", "scope", "native_session_id", "native_inputs", "edit_allowed",
        "all_four_evidence_verified", "full_file_unchanged_verified", "same_native_session_verified",
        "normal_exit", "cleanup_confirmed", "transport_closed", "real_gui_verified", "persistence_verified",
        "http_request_count_verified"},
    "acceptance_failed": {"event", "scope", "reason_sha256"},
    "public_projection_rejected": {"event", "dropped_records"},
    "cancel_terminal_observed": {"event", "phase", "turn_id", "native_session_id", "outcome", "expected_turn_id",
        "turn_id_matches", "interrupt_acknowledged", "approval_cancelled", "turn_started", "error_bytes", "error_sha256"},
}
NATIVE_TYPES = {"assistant", "user", "result", "system", "control_request", "control_response",
                "command_lifecycle", "stream_event", "control_cancel_request", "keep_alive",
                "rate_limit_event", "tool_progress", "tool_use_summary", "auth_status", "unknown"}
NATIVE_SUBTYPES = {"success", "error_during_execution", "error_max_turns", "error_max_budget_usd",
    "error_max_structured_output_retries", "init", "status", "hook_started", "hook_progress", "hook_response",
    "task_started", "task_progress", "task_notification", "compact_boundary", "elicitation_complete",
    "permission_denied", "permission_mode_changed", "unknown"}
REQUEST_SUBTYPES = {"initialize", "interrupt", "can_use_tool", "mcp_message", "get_settings",
    "list_permission_rules", "get_hooks", "list_hooks", "mcp_status", "set_permission_mode", "set_model",
    "set_max_thinking_tokens", "rewind_files", "reload_plugins", "unknown"}
TOOL_NAMES = {"Read", "Edit", "Write", "Bash", "Grep", "Glob", "LS", "Task", "Agent", "TaskCreate",
    "TaskGet", "TaskUpdate", "TaskList", "TodoWrite", "WebFetch", "WebSearch", "NotebookEdit", "AskUserQuestion",
    "EnterPlanMode", "ExitPlanMode", "mcp__infinishell-local-tasks__inspect_local_tasks",
    "mcp__infinishell-local-tasks__run_agents", "mcp__infinishell-local-tasks__send_message_to_agent", "unknown"}
TERMINAL_REASONS = {"aborted_streaming", "aborted_tools", "interrupted", "cancelled", "api_error", "completed", "end_turn", "unknown"}
BOOL_KEYS = {"credential_files_read_by_probe", "real_gui_verified", "persistence_verified", "fixed_profile_verified",
    "filesystem_sandbox_verified", "parent_permission_ceiling_verified", "full_bytes_verified", "native_receipt",
    "exact_edit_verified", "edit_allowed", "approval_still_pending", "native_execution_cancelled_verified",
    "same_native_session_verified", "cleanup_confirmed", "normal_exit", "transport_closed", "cleanup_receipt_read",
    "all_four_evidence_verified", "full_file_unchanged_verified", "http_request_count_verified"}
ID_KEYS = {"message_id", "turn_id", "native_session_id", "approval_id", "tool_use_id", "generation",
    "runtime_generation", "execution_turn_id", "execution_input_id", "continuation_input_id", "interrupt_request_id",
    "native_result_uuid", "continue_result_uuid"}
INT_KEYS = {"max_native_inputs", "phase_deadline_seconds", "file_bytes", "submitted_input_count", "output_bytes",
            "sequence", "native_inputs", "read_call_count", "dropped_records"}


def _require(condition):
    if not condition:
        raise ValueError("待审批取消证据不完整或关联错误")


def _hash(value):
    _require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None)
    return value


def _uuid(value):
    _require(isinstance(value, str) and str(uuid.UUID(value)) == value)
    return value


def _safe_id(value, nullable=False):
    if nullable and value is None:
        return value
    _require(isinstance(value, str) and len(value) <= 160 and re.fullmatch(
        r"(?:[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}|(?:msg_|toolu_)[A-Za-z0-9_-]+|"
        r"infinishell-[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}-[0-9]+|sha256:[0-9a-f]{64})", value))
    return value


def _native(row):
    _require(isinstance(row, dict) and set(row).issubset(NATIVE_KEYS)
             and row.get("direction") in {"stdin", "stdout"})
    for key, value in row.items():
        if key == "direction":
            continue
        if key in {"type", "subtype", "request_subtype", "state", "terminal_reason", "response_subtype"}:
            allowed = {"type": NATIVE_TYPES, "subtype": NATIVE_SUBTYPES, "request_subtype": REQUEST_SUBTYPES,
                       "state": {"queued", "started", "completed", "cancelled", "unknown"},
                       "terminal_reason": TERMINAL_REASONS, "response_subtype": {"success", "error", "unknown"}}[key]
            _require(value is None or value in allowed)
        elif key == "is_error":
            _require(value is None or type(value) is bool)
        elif key == "user_message_uuids":
            _require(value is None or isinstance(value, list) and len(value) <= 32)
            for identity in value or []:
                _safe_id(identity, nullable=True)
        elif key == "tools":
            _require(isinstance(value, list) and len(value) <= 32)
            for tool in value:
                _require(isinstance(tool, dict) and set(tool) == {"id", "name"} and tool["name"] in TOOL_NAMES)
                _safe_id(tool["id"], nullable=True)
        elif key == "cancel_diagnostics":
            _require(row.get("type") == "result")
            validate_cancel_diagnostics(value)
        else:
            _safe_id(value, nullable=True)
    return row


def _event(event):
    _require(isinstance(event, dict) and event.get("event") in EVENT_KEYS
             and set(event).issubset(EVENT_KEYS[event["event"]]))
    if event["event"] == "cancel_terminal_observed":
        return validate_cancel_terminal(event, PHASES, approval=True)
    for key, value in event.items():
        if key == "event":
            continue
        if key in BOOL_KEYS:
            _require(type(value) is bool)
        elif key in ID_KEYS:
            _safe_id(value, nullable=key == "native_session_id")
        elif key in INT_KEYS:
            _require(type(value) is int and 0 <= value <= 1024 * 1024)
        elif key.endswith("sha256"):
            _hash(value)
        elif key == "native":
            _native(value)
        elif key == "receipt":
            if value is not None:
                _require(isinstance(value, dict) and set(value) == RECEIPT_KEYS)
                _uuid(value["generation"])
                _require(type(value["version"]) is int and type(value["cleanup_confirmed"]) is bool
                         and (value["exit_code"] is None or type(value["exit_code"]) is int)
                         and value["containment"] in {"macos_resource_coalition", "linux_subtree", "windows_job", "unix_process_group"}
                         and value["exit_reason"] in {"native_exit", "stop_requested", "host_disconnected", "stdio_closed"})
        elif key == "input_keys":
            _require(isinstance(value, list) and value == sorted(set(value))
                     and set(value).issubset({"file_path", "old_string", "new_string", "replace_all"}))
        elif key == "replace_all":
            _require(value is None or type(value) is bool)
        elif key == "user_message_uuids":
            _require(isinstance(value, list) and len(value) <= 2)
            for identity in value:
                _uuid(identity)
        elif key == "output":
            _require(isinstance(value, str) and re.fullmatch(r"APPROVAL_CANCEL_CONTINUED_[0-9a-f]{32}", value))
        elif key == "scope":
            _require(value == SCOPE)
        elif key == "permission_policy":
            _require(value == "ClaudeRestrictedFilesV1")
        elif key == "permission_mode":
            _require(value == "plan")
        elif key == "file_ref":
            _require(value == FILE_REF)
        elif key == "phase":
            _require(value in PHASES | {"before", "after_cancel", "after_continue", "after_shutdown"})
        elif key == "outcome":
            _require(value in {"Completed", "Cancelled"})
        elif key == "terminal_reason":
            _require(value in TERMINAL_REASONS)
        elif key == "is_error":
            _require(type(value) is bool)
        else:
            raise ValueError("非白名单公开字段")
    return event


def project_events(events):
    """失败仍保留安全协议 ID；未知字段或任意正文不得借诊断流出。"""
    projected, dropped = [], 0
    for event in events:
        try:
            projected.append(_event(event))
        except (ValueError, TypeError, KeyError, AttributeError):
            dropped += 1
    if dropped:
        projected.append({"event": "public_projection_rejected", "dropped_records": dropped})
    return projected, dropped == 0


def _one(events, kind, **fields):
    rows = [event for event in events if event.get("event") == kind
            and all(event.get(key) == value for key, value in fields.items())]
    _require(len(rows) == 1)
    return rows[0]


def audit_events(events):
    """重算原生请求、ACK、执行生命周期和完整结果，拒绝本地成功旗替代证据。"""
    _require(isinstance(events, list) and events and all(_event(event) is event for event in events)
             and not any(event["event"] in {"acceptance_failed", "public_projection_rejected"} for event in events))
    beginning, ending = _one(events, "acceptance_started"), _one(events, "acceptance_passed")
    _require(events[0] is beginning and events[-1] is ending and beginning["scope"] == ending["scope"] == SCOPE
             and beginning["max_native_inputs"] == 2 and beginning["phase_deadline_seconds"] == 180
             and beginning["permission_policy"] == "ClaudeRestrictedFilesV1")
    native_id = _uuid(ending["native_session_id"])
    observed = [event for event in events if event["event"] == "cancel_terminal_observed"]
    _require(len({event["turn_id"] for event in observed}) == len(observed))
    for event in observed:
        matching = [row for row in events if row["event"] == "turn_finished" and row.get("turn_id") == event["turn_id"]]
        _require(len(matching) == 1 and event["native_session_id"] == native_id
                 and event["phase"] == matching[0].get("phase") and event["outcome"] == matching[0].get("outcome")
                 and event["outcome"] != "Failed" and event["turn_id_matches"] is True
                 and event["interrupt_acknowledged"] is True and event["turn_started"] is True
                 and event["approval_cancelled"] is True)
    _require(ending["native_inputs"] == 2 and ending["edit_allowed"] is False)
    for event, true, false in ((beginning, (), ("credential_files_read_by_probe", "real_gui_verified", "persistence_verified")),
        (ending, ("all_four_evidence_verified", "full_file_unchanged_verified", "same_native_session_verified",
                  "normal_exit", "cleanup_confirmed", "transport_closed"),
                 ("real_gui_verified", "persistence_verified", "http_request_count_verified"))):
        _require(all(event[key] is True for key in true) and all(event[key] is False for key in false))
    profiles = [event for event in events if event["event"] == "profile_verified"]
    _require(profiles and all(event["profile_sha256"] == profiles[0]["profile_sha256"]
        and event["permission_mode"] == "plan" and event["fixed_profile_verified"] is True
        and event["filesystem_sandbox_verified"] is event["parent_permission_ceiling_verified"] is False for event in profiles))
    submits = [event for event in events if event["event"] == "input_submitted"]
    _require(len(submits) == 2 and [event["phase"] for event in submits] == ["pending_edit", "continue"]
             and [event["submitted_input_count"] for event in submits] == [1, 2])
    running, continued = [_uuid(event["message_id"]) for event in submits]
    _require(running != continued)
    accepts, starts, finishes = [], [], []
    for identity, phase in ((running, "pending_edit"), (continued, "continue")):
        accepted = _one(events, "message_accepted", message_id=identity)
        started = _one(events, "turn_started", turn_id=identity)
        finished = _one(events, "turn_finished", phase=phase)
        _require(accepted["native_receipt"] is True and accepted["turn_id"] == started["turn_id"] == finished["turn_id"] == identity
                 and all(event["native_session_id"] == native_id for event in (accepted, started, finished)))
        accepts.append(accepted); starts.append(started); finishes.append(finished)
    _require(all(len([event for event in events if event["event"] == kind]) == 2
                 for kind in ("message_accepted", "turn_started", "turn_finished")))
    _require(finishes[0]["outcome"] == "Cancelled" and "output" not in finishes[0]
             and finishes[1]["outcome"] == "Completed" and finishes[1]["same_native_session_verified"] is True
             and hashlib.sha256(finishes[1]["output"].encode()).hexdigest() == finishes[1]["trimmed_output_sha256"])
    files = [_one(events, "file_unchanged", phase=phase) for phase in ("before", "after_cancel", "after_continue", "after_shutdown")]
    _require(len([event for event in events if event["event"] == "file_unchanged"]) == 4
             and all(event["full_bytes_verified"] is True and event["file_bytes"] == len(BEFORE)
                     and event["file_sha256"] == hashlib.sha256(BEFORE).hexdigest() for event in files))
    approval = _one(events, "edit_approval_requested")
    approval_id, tool_id = _safe_id(approval["approval_id"]), _safe_id(approval["tool_use_id"])
    _require(approval["turn_id"] == running and approval["native_session_id"] == native_id
             and tool_id.startswith("toolu_") and approval["exact_edit_verified"] is True and approval["edit_allowed"] is False
             and approval["old_string_sha256"] == hashlib.sha256(BEFORE).hexdigest()
             and approval["new_string_sha256"] == hashlib.sha256(AFTER).hexdigest()
             and set(approval["input_keys"]) in ({"file_path", "old_string", "new_string"},
                                               {"file_path", "old_string", "new_string", "replace_all"})
             and (approval["replace_all"] is False or approval["replace_all"] is None
                  and "replace_all" not in approval["input_keys"]))
    interrupt = _one(events, "interrupt_submitted")
    interrupt_id = _uuid(interrupt["message_id"])
    ack = _one(events, "interrupt_accepted")
    withdrawn = _one(events, "edit_approval_cancelled")
    _require(interrupt_id not in {running, continued} and interrupt["execution_turn_id"] == running
             and interrupt["approval_id"] == approval_id and interrupt["approval_still_pending"] is True
             and interrupt["edit_allowed"] is False and ack["message_id"] == interrupt_id and ack["turn_id"] == running
             and ack["native_session_id"] == withdrawn["native_session_id"] == native_id
             and ack["native_receipt"] is True and withdrawn["approval_id"] == approval_id
             and withdrawn["native_execution_cancelled_verified"] is False)
    shutdown, cleanup = _one(events, "connection_shutdown"), _one(events, "cleanup_checked")
    _require(shutdown["native_session_id"] == native_id and all(cleanup[key] is True for key in
        ("cleanup_confirmed", "normal_exit", "transport_closed", "cleanup_receipt_read")))
    generation = _uuid(cleanup["generation"])
    receipt = cleanup["receipt"]
    _require(receipt is not None and receipt["version"] == 1 and receipt["generation"] == generation
             and receipt["cleanup_confirmed"] is True and receipt["exit_code"] == 0 and receipt["exit_reason"] == "stdio_closed")
    position = lambda event: next(index for index, candidate in enumerate(events) if candidate is event)
    ordered = [files[0], profiles[0], submits[0], accepts[0], starts[0], approval, interrupt,
               finishes[0], files[1], submits[1], accepts[1], starts[1], finishes[1], files[2], shutdown, cleanup]
    _require(all(position(left) < position(right) for left, right in zip(ordered, ordered[1:]))
             and position(interrupt) < position(ack) < position(finishes[0])
             and position(interrupt) < position(withdrawn) < position(finishes[0]))
    projections = [event for event in events if event["event"] == "native_protocol_ids"]
    _require(9 <= len(projections) <= 512 and [event["sequence"] for event in projections] == list(range(1, len(projections) + 1))
             and all(event["generation"] == generation for event in projections))
    rows = [event["native"] for event in projections]
    def select(direction, kind, **fields):
        return [(index, row) for index, row in enumerate(rows) if row.get("direction") == direction and row.get("type") == kind
                and all(row.get(key) == value for key, value in fields.items())]
    def unique(direction, kind, **fields):
        selected = select(direction, kind, **fields)
        _require(len(selected) == 1)
        return selected[0]
    users = select("stdin", "user")
    _require(len(users) == 2 and [row.get("uuid") for _, row in users] == [running, continued]
             and users[0][1].get("session_id") in {None, native_id, "sha256:" + hashlib.sha256(b"").hexdigest()}
             and users[1][1].get("session_id") == native_id
             and all(row.get("session_id") in {None, native_id} for row in rows if row.get("direction") == "stdout"))
    native_starts = []
    for identity, (write_index, _) in zip((running, continued), users):
        start_index, _ = unique("stdout", "command_lifecycle", command_uuid=identity, state="started", session_id=native_id)
        native_starts.append(start_index)
        _require(write_index < start_index)
    permissions = select("stdout", "control_request", request_subtype="can_use_tool")
    _require(len(permissions) == 1)
    permission_index, pending = permissions[0]
    _require(pending.get("request_id") == approval_id and pending.get("tool_use_id") == tool_id
             and not select("stdin", "control_response", response_request_id=approval_id))
    seen_tools, read_tools = [], set()
    for index, row in enumerate(rows):
        for tool in row.get("tools", []):
            _require(row.get("direction") == "stdout" and row.get("type") == "assistant"
                     and row.get("session_id") == native_id and native_starts[0] < index < permission_index)
            if tool["name"] == "Read":
                _require(isinstance(tool["id"], str) and tool["id"].startswith("toolu_"))
                read_tools.add(tool["id"])
            else:
                _require(tool == {"id": tool_id, "name": "Edit"})
                seen_tools.append(tool_id)
    _require(seen_tools and set(seen_tools) == {tool_id} and len(read_tools) <= 1 and tool_id not in read_tools)
    controls = select("stdin", "control_request", request_subtype="interrupt")
    _require(len(controls) == 1)
    control_index, control = controls[0]
    request_id = control.get("request_id")
    _require(isinstance(request_id, str) and re.fullmatch(r"infinishell-" + re.escape(generation) + r"-[1-9][0-9]*", request_id))
    ack_index, native_ack = unique("stdout", "control_response", response_request_id=request_id)
    cancelled_index, _ = unique("stdout", "command_lifecycle", command_uuid=running, state="cancelled", session_id=native_id)
    results = select("stdout", "result")
    _require(len(results) == 2)
    (result_index, result), (continue_index, native_continue) = results
    shape = ((result.get("terminal_reason") in {"aborted_streaming", "aborted_tools"} and result.get("subtype") == "error_during_execution"
              and result.get("is_error") is True)
             or (result.get("terminal_reason") in {"interrupted", "cancelled"} and result.get("subtype") == "success"
                 and result.get("is_error") is False))
    _require(native_ack.get("response_subtype") == "success" and shape
             and result.get("session_id") == native_id and result.get("user_message_uuid") == running
             and result.get("user_message_uuids") == [running]
             and native_continue.get("session_id") == native_id and native_continue.get("user_message_uuid") == continued
             and native_continue.get("user_message_uuids") == [continued] and native_continue.get("subtype") == "success"
             and native_continue.get("is_error") is False and native_continue.get("terminal_reason") in {None, "completed", "end_turn"}
             and native_starts[0] < permission_index < control_index < min(ack_index, cancelled_index, result_index)
             and max(ack_index, cancelled_index, result_index) < users[1][0] < native_starts[1] < continue_index)
    result_id, continue_id = _uuid(result.get("uuid")), _uuid(native_continue.get("uuid"))
    _require(result_id != continue_id)
    for _, row in select("stdout", "control_cancel_request"):
        _require(row.get("request_id") == approval_id)
    for _, row in select("stdout", "command_lifecycle"):
        _require(row.get("command_uuid") in {running, continued}
                 and row.get("state") in {"queued", "started", "completed", "cancelled"})
    proof = _one(events, "native_pending_edit_cancel_verified")
    expected = {"native_session_id": native_id, "runtime_generation": generation, "execution_input_id": running,
        "continuation_input_id": continued, "approval_id": approval_id, "tool_use_id": tool_id,
        "interrupt_request_id": request_id, "native_result_uuid": result_id, "continue_result_uuid": continue_id,
        "terminal_reason": result["terminal_reason"], "is_error": result["is_error"], "user_message_uuids": [running],
        "edit_allowed": False, "read_call_count": len(read_tools), "all_four_evidence_verified": True}
    _require(all(proof.get(key) == value for key, value in expected.items())
             and position(cleanup) < position(projections[0]) <= position(projections[-1])
             < position(files[3]) < position(proof) < position(ending))
    return expected | {"profile_sha256": profiles[0]["profile_sha256"], "native_protocol_records": len(rows)}


def verified_acceptance(exit_code, output, events):
    if exit_code != 0 or len(re.findall(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output)) != 1:
        return False
    try:
        audit_events(events)
    except (ValueError, TypeError, KeyError, AttributeError):
        return False
    return True


def prepare_project(root):
    settings = PREPARE_BASE_PROJECT(root)
    with (root / "project" / FILE_REF).open("xb") as output:
        output.write(BEFORE)
    return settings


def run(args):
    if args.api_environment_file is None or not isinstance(args.model, str) or not args.model \
            or any(character in args.model for character in "\x00\r\n"):
        raise ValueError("必须显式指定私有API环境文件和固定模型")
    if args.max_native_inputs != 2 or args.timeout_seconds != 900:
        raise ValueError("本夹具固定最多两条原生输入和900秒外层时限")
    private = Path(tempfile.mkdtemp(prefix="infinishell-claude-approval-cancel-auth-")).resolve()
    args.config_dir, args.auth_home = private / "claude", private / "home"
    args.config_dir.mkdir(mode=0o700)
    args.auth_home.mkdir(mode=0o700)
    adapter.validate_paths(args)
    if os.name != "nt" and args.api_environment_file.stat().st_mode & 0o077:
        raise ValueError("显式API环境文件必须仅当前用户可读写")
    original_environment = adapter.authenticated_environment
    def environment(*values):
        result = original_environment(*values)
        result["INFINISHELL_CLAUDE_APPROVAL_CANCEL_MAX_NATIVE_INPUTS"] = "2"
        return result
    replacements = {"TEST_NAME": TEST_NAME, "PROJECT_SETTINGS": PROJECT_SETTINGS,
                    "prepare_project": prepare_project, "verified_acceptance": verified_acceptance,
                    "authenticated_environment": environment}
    originals = {key: getattr(adapter, key) for key in replacements}
    transcript = args.output.with_suffix(".test-output.txt")
    stdout_proof = None
    try:
        for key, value in replacements.items():
            setattr(adapter, key, value)
        result = adapter.run(args)
    finally:
        for key, value in originals.items():
            setattr(adapter, key, value)
        # 既有路径校验已拒绝预存产物；异常也将本轮 stdout 改为摘要，绝不保留正文。
        stdout_proof = compact_stdout(transcript)
    metadata_path = args.output.with_suffix(".metadata.json")
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata.update({"scope": SCOPE, "permission_policy": "ClaudeRestrictedFilesV1", "max_native_inputs": 2,
        "phase_deadline_seconds": 180, "timeout_seconds": 900, "fresh_private_auth_home": True,
        "private_auth_workspace": str(private), "waiting_edit_cancel_verified": False,
        "app_restart_and_ui_verified": False, "persistence_verified": False, "http_request_count_verified": False,
        "parent_permission_ceiling_verified": False, "filesystem_sandbox_verified": False})
    try:
        events = [json.loads(line) for line in args.output.read_text(encoding="utf-8").splitlines()]
        projected, safe = project_events(events)
    except (OSError, ValueError, TypeError):
        projected, safe = [{"event": "public_projection_rejected", "dropped_records": 1}], False
    args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in projected), encoding="utf-8")
    metadata["public_projection_verified"] = safe
    metadata["stdout_proof"] = stdout_proof
    if "runner_error" in metadata:
        metadata["runner_error"] = "基础运行器未完成验收，请检查受限证据和退出状态"
    if metadata.get("acceptance_passed") is True:
        try:
            _require(type(metadata.get("test_exit_code")) is int and metadata["test_exit_code"] == 0
                     and metadata.get("project_settings_unchanged") is True and not metadata.get("timed_out", False)
                     and safe and stdout_proof is not None and not any(stdout_proof["credential_shape_counts"].values())
                     and stdout_proof["test_success_summary_count"] == 1)
            metadata["native_pending_edit_cancel_proof"] = audit_events(projected)
            metadata["waiting_edit_cancel_verified"] = True
        except (ValueError, TypeError, KeyError, AttributeError):
            metadata["acceptance_passed"] = False
            metadata["runner_error"] = "公开投影、stdout扫描或待审批取消四证据复核失败"
            result = 1
    if not safe or stdout_proof is None or any(stdout_proof["credential_shape_counts"].values()):
        metadata["acceptance_passed"] = False
        metadata["waiting_edit_cancel_verified"] = False
        result = 1
    metadata_path.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("test-binary", "claude", "supervisor", "api-environment-file", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--max-native-inputs", type=int, choices=[2], default=2)
    parser.add_argument("--timeout-seconds", type=int, choices=[900], default=900)
    args = parser.parse_args()
    try:
        return run(args)
    except (OSError, ValueError, TypeError, KeyError, adapter.subprocess.SubprocessError) as error:
        parser.exit(2, f"待审批取消运行器未完成：{type(error).__name__}；检查显式私有参数。\n")


if __name__ == "__main__":
    raise SystemExit(main())
