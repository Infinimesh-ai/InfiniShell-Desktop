#!/usr/bin/env python3
"""复用私有 API 隔离运行器，核验 Claude 合并批次的真实取消及同会话后续执行。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import uuid

import run_claude_adapter_live as adapter
from prepare_claude_cli import RELEASE_CATALOG, VERSION, release_contract


TEST_NAME = ("ai::cli_agent_runtime::claude::live_tests::batch_cancel_live_tests::"
             "real_claude_joined_batch_cancel")
SCOPE = "rust_adapter_joined_batch_cancel"
PROJECT_SETTINGS = {"permissions": {"ask": ["Edit"]}}
PREPARE_BASE_PROJECT = adapter.prepare_project
INSPECT_TOOL = "mcp__infinishell-local-tasks__inspect_local_tasks"
NATIVE_KEYS = {
    "direction", "type", "subtype", "uuid", "session_id", "message_id",
    "user_message_uuid", "user_message_uuids", "command_uuid", "state", "request_id",
    "request_subtype", "tool_use_id", "tools", "terminal_reason", "is_error",
    "response_request_id", "response_subtype", "cancel_diagnostics",
}
EVENT_KINDS = {
    "acceptance_started", "profile_verified", "input_submitted", "message_accepted",
    "turn_started", "input_joined", "inspect_approval_requested", "inspect_approval_allowed",
    "inspect_literal_returned", "interrupt_submitted", "interrupt_accepted", "turn_finished",
    "connection_shutdown", "cleanup_checked", "native_protocol_ids",
    "native_batch_cancel_verified", "acceptance_passed", "cancel_terminal_observed",
}


def prepare_project(root):
    # 只创建既有隔离项目与默认探针标记，不增加项目读写或用户全局允许规则。
    return PREPARE_BASE_PROJECT(root)


def _require(condition):
    if not condition:
        raise ValueError("真实合并取消证据不完整或关联错误")


def _one(events, kind):
    rows = [event for event in events if event.get("event") == kind]
    _require(len(rows) == 1)
    return rows[0]


def _uuid(value):
    _require(isinstance(value, str) and str(uuid.UUID(value)) == value)
    return value


def _hash(value):
    _require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None)
    return value


def _id(value):
    _require(isinstance(value, str) and 0 < len(value) <= 160
             and not any(character.isspace() or ord(character) < 32 for character in value))
    return value


def validate_cancel_diagnostics(value):
    """可选取消摘要只接受固定形状；旧账本不要求新增字段。"""
    _require(isinstance(value, dict) and set(value) == {"terminal_reason", "errors"})
    for key, shape in value.items():
        keys = {"type", "bytes", "sha256"} | ({"count"} if key == "errors" else set())
        _require(isinstance(shape, dict) and set(shape) == keys
                 and shape["type"] in {"missing", "null", "boolean", "number", "string", "array", "object"}
                 and type(shape["bytes"]) is int and 0 <= shape["bytes"] <= 8 * 1024 * 1024)
        if shape["type"] == "missing":
            _require(shape["bytes"] == 0 and shape["sha256"] is None)
        else:
            _hash(shape["sha256"])
        if key == "errors":
            _require(type(shape["count"]) is int and 0 <= shape["count"] <= 8 * 1024 * 1024
                     and (shape["type"] == "array" or shape["count"] == 0))
    return value


def validate_cancel_terminal(event, phases, approval=False):
    keys = {"event", "phase", "turn_id", "native_session_id", "outcome", "expected_turn_id",
            "turn_id_matches", "interrupt_acknowledged", "turn_started", "error_bytes", "error_sha256"}
    if approval:
        keys.add("approval_cancelled")
    _require(isinstance(event, dict) and set(event) == keys and event["event"] == "cancel_terminal_observed"
             and event["phase"] in phases and event["outcome"] in {"Completed", "Cancelled", "Failed"})
    _uuid(event["turn_id"])
    _uuid(event["expected_turn_id"])
    if event["native_session_id"] is not None:
        _uuid(event["native_session_id"])
    for key in {"turn_id_matches", "interrupt_acknowledged", "turn_started"} | ({"approval_cancelled"} if approval else set()):
        _require(type(event[key]) is bool)
    _require(event["turn_id_matches"] == (event["turn_id"] == event["expected_turn_id"])
             and type(event["error_bytes"]) is int and 0 <= event["error_bytes"] <= 8 * 1024 * 1024)
    if event["outcome"] == "Failed":
        _hash(event["error_sha256"])
    else:
        _require(event["error_bytes"] == 0 and event["error_sha256"] is None)
    return event


def _flags(event, true=(), false=()):
    _require(all(event.get(key) is True for key in true)
             and all(event.get(key) is False for key in false))


def audit_events(events):
    """独立重算 ID、真实协议账本和三类取消信号，不以生产成功旗替代执行证据。"""
    _require(isinstance(events, list) and events
             and all(isinstance(event, dict) and event.get("event") in EVENT_KINDS for event in events))
    for event in events:
        if event["event"] == "cancel_terminal_observed":
            validate_cancel_terminal(event, {"batch_cancel", "continue"})
    ending = _one(events, "acceptance_passed")
    native_id = _uuid(ending.get("native_session_id"))
    observed = [event for event in events if event["event"] == "cancel_terminal_observed"]
    _require(len({event["turn_id"] for event in observed}) == len(observed))
    for event in observed:
        matching = [row for row in events if row["event"] == "turn_finished" and row.get("turn_id") == event["turn_id"]]
        _require(len(matching) == 1 and event["native_session_id"] == native_id
                 and event["phase"] == matching[0].get("phase") and event["outcome"] == matching[0].get("outcome")
                 and event["outcome"] != "Failed" and event["turn_id_matches"] is True
                 and event["interrupt_acknowledged"] is True
                 and type(event.get("turn_started")) is bool)
    _require(ending.get("scope") == SCOPE)
    for key, expected in (("native_inputs", 3), ("native_executions", 2), ("joined_inputs", 1),
                          ("cancelled_inputs", 2), ("continued_inputs", 1), ("inspect_call_count", 1)):
        _require(type(ending.get(key)) is int and ending[key] == expected)
    _flags(ending, ("all_three_signals_verified", "same_native_session_verified",
                    "cleanup_confirmed", "transport_closed"),
           ("credential_files_read_by_probe", "persistence_verified", "app_restart_and_ui_verified",
            "parent_permission_ceiling_verified"))
    beginning = _one(events, "acceptance_started")
    _require(events[0] is beginning and events[-1] is ending
             and beginning.get("scope") == SCOPE and beginning.get("max_native_inputs") == 3
             and type(beginning.get("max_inspect_calls")) is int and beginning["max_inspect_calls"] == 1
             and beginning.get("permission_policy") == "ClaudeRestrictedFilesV1")
    _flags(beginning, false=("credential_files_read_by_probe", "real_gui_verified", "persistence_verified"))
    profiles = [event for event in events if event["event"] == "profile_verified"]
    _require(profiles)
    profile_hash = _hash(profiles[0].get("profile_sha256"))
    for profile in profiles:
        _require(profile.get("permission_mode") == "plan" and profile.get("profile_sha256") == profile_hash)
        _flags(profile, ("fixed_profile_verified",),
               ("filesystem_sandbox_verified", "parent_permission_ceiling_verified"))
    submissions = [event for event in events if event["event"] == "input_submitted"]
    _require(len(submissions) == 3)
    running, joined, continuation = [_uuid(event.get("message_id")) for event in submissions]
    _require(len({running, joined, continuation}) == 3
             and all(type(event.get("submitted_input_count")) is int for event in submissions)
             and [event.get("phase") for event in submissions] == ["batch_cancel", "batch_cancel", "continue"]
             and [event.get("submitted_input_count") for event in submissions] == [1, 2, 3]
             and submissions[1].get("active_turn_id") == running)
    if observed:
        observed_by_id = {event["turn_id"]: event for event in observed}
        _require(len(observed) == 3 and set(observed_by_id) == {running, joined, continuation}
                 and observed_by_id[running]["turn_started"] is True
                 and observed_by_id[joined]["turn_started"] is False
                 and observed_by_id[continuation]["turn_started"] is True)
    _flags(submissions[1], ("submitted_while_running",))
    accepts = [event for event in events if event["event"] == "message_accepted"]
    _require(len(accepts) == 3 and {event.get("message_id") for event in accepts} == {running, joined, continuation})
    for event in accepts:
        _require(event.get("turn_id") == event["message_id"] and event.get("native_session_id") == native_id)
        _flags(event, ("native_receipt",))
    accepts_by_id = {event["message_id"]: event for event in accepts}
    starts = [event for event in events if event["event"] == "turn_started"]
    _require(len(starts) == 2 and {event.get("turn_id") for event in starts} == {running, continuation})
    for event in starts:
        _require(event.get("native_session_id") == native_id)
    starts_by_id = {event["turn_id"]: event for event in starts}
    join = _one(events, "input_joined")
    _require(join.get("message_id") == joined and join.get("turn_id") == running
             and join.get("native_session_id") == native_id)
    approval = _one(events, "inspect_approval_requested")
    _id(approval.get("approval_id"))
    _require(approval.get("turn_id") == running)
    _flags(approval, ("exact_inspect_fixture",))
    allowed = _one(events, "inspect_approval_allowed")
    _require(allowed.get("approval_id") == approval["approval_id"] and allowed.get("decision") == "AllowOnce")
    _flags(allowed, false=("native_receipt",))
    literal = _one(events, "inspect_literal_returned")
    _id(literal.get("call_id"))
    _require(literal.get("turn_id") == running and type(literal.get("inspect_call_count")) is int
             and literal["inspect_call_count"] == 1)
    _flags(literal, ("no_project_read_or_write", "transport_write_confirmed"), ("native_receipt",))
    interrupt = _one(events, "interrupt_submitted")
    interrupt_id = _uuid(interrupt.get("message_id"))
    _require(interrupt_id not in {running, joined, continuation}
             and interrupt.get("execution_turn_id") == running and interrupt.get("joined_input_id") == joined)
    _flags(interrupt, ("both_inputs_native_acknowledged", "input_joined_verified"))
    interrupt_ack = _one(events, "interrupt_accepted")
    _require(interrupt_ack.get("message_id") == interrupt_id and interrupt_ack.get("turn_id") == running
             and interrupt_ack.get("native_session_id") == native_id)
    _flags(interrupt_ack, ("native_receipt",))
    finishes = [event for event in events if event["event"] == "turn_finished"]
    _require(len(finishes) == 3 and [event.get("turn_id") for event in finishes] == [joined, running, continuation])
    for index, event in enumerate(finishes[:2], 1):
        _require(event.get("phase") == "batch_cancel" and event.get("outcome") == "Cancelled"
                 and event.get("native_session_id") == native_id and type(event.get("batch_terminal_index")) is int
                 and event["batch_terminal_index"] == index
                 and type(event.get("output_bytes")) is int and event["output_bytes"] >= 0 and "output" not in event)
        _hash(event.get("output_sha256"))
    _require(finishes[0]["output_bytes"] == finishes[1]["output_bytes"]
             and finishes[0]["output_sha256"] == finishes[1]["output_sha256"])
    completed = finishes[2]
    marker = completed.get("output")
    _require(completed.get("phase") == "continue" and completed.get("outcome") == "Completed"
             and completed.get("native_session_id") == native_id and isinstance(marker, str)
             and re.fullmatch(r"BATCH_CONTINUED_[0-9a-f]{32}", marker) is not None)
    _flags(completed, ("same_native_session_verified",))
    shutdown = _one(events, "connection_shutdown")
    _require(shutdown.get("native_session_id") == native_id)
    cleanup = _one(events, "cleanup_checked")
    _flags(cleanup, ("cleanup_confirmed", "transport_closed", "cleanup_receipt_read"))
    generation = _uuid(cleanup.get("generation"))
    receipt = cleanup.get("receipt")
    _require(isinstance(receipt, dict) and receipt.get("generation") == generation
             and receipt.get("cleanup_confirmed") is True and type(receipt.get("exit_code")) is int
             and receipt["exit_code"] == 0
             and receipt.get("containment") in {"macos_resource_coalition", "linux_subtree", "windows_job", "unix_process_group"}
             and receipt.get("exit_reason") in {"native_exit", "stop_requested", "host_disconnected", "stdio_closed"})
    position = lambda event: next(index for index, candidate in enumerate(events) if candidate is event)
    ordered = [submissions[0], accepts_by_id[running], starts_by_id[running], approval, submissions[1],
               accepts_by_id[joined], allowed, literal, interrupt, interrupt_ack, finishes[0], finishes[1],
               submissions[2], accepts_by_id[continuation], starts_by_id[continuation], completed, shutdown, cleanup]
    _require(all(position(left) < position(right) for left, right in zip(ordered, ordered[1:])))
    _require(position(profiles[0]) < position(submissions[0])
             and position(accepts_by_id[joined]) < position(join) < position(interrupt)
             and position(starts_by_id[running]) < position(join))

    projections = [event for event in events if event["event"] == "native_protocol_ids"]
    _require(projections and [event.get("sequence") for event in projections] == list(range(1, len(projections) + 1)))
    rows = [event.get("native") for event in projections]
    _require(all(isinstance(row, dict) and set(row).issubset(NATIVE_KEYS)
                 and row.get("direction") in {"stdin", "stdout"} for row in rows))
    for row in rows:
        if "cancel_diagnostics" in row:
            _require(row.get("type") == "result")
            validate_cancel_diagnostics(row["cancel_diagnostics"])
    users = [(index, row) for index, row in enumerate(rows) if row.get("direction") == "stdin" and row.get("type") == "user"]
    _require(len(users) == 3 and [row.get("uuid") for _, row in users] == [running, joined, continuation])
    empty_id_hash = "sha256:" + hashlib.sha256(b"").hexdigest()
    _require(users[0][1].get("session_id") in {None, native_id, empty_id_hash}
             and all(row.get("session_id") == native_id for _, row in users[1:]))
    _require(all(row.get("session_id") in {None, native_id} for row in rows if row.get("direction") == "stdout"))
    native_tools = {}
    for row in rows:
        tools = row.get("tools", [])
        _require(isinstance(tools, list))
        for tool in tools:
            _require(isinstance(tool, dict) and set(tool) == {"id", "name"} and tool["name"] == INSPECT_TOOL
                     and row.get("direction") == "stdout" and row.get("type") == "assistant")
            native_tools[_id(tool["id"])] = tool["name"]
    _require(len(native_tools) == 1)
    permission_requests = [row for row in rows if row.get("direction") == "stdout"
                           and row.get("type") == "control_request" and row.get("request_subtype") == "can_use_tool"]
    _require(len(permission_requests) == 1 and permission_requests[0].get("request_id") == approval["approval_id"]
             and permission_requests[0].get("tool_use_id") in native_tools)
    permission_writes = [row for row in rows if row.get("direction") == "stdin"
                         and row.get("type") == "control_response" and row.get("response_request_id") == approval["approval_id"]]
    _require(len(permission_writes) == 1 and permission_writes[0].get("response_subtype") == "success")
    # 审批与工具回复的本地写入不是原生 ACK；取消 ACK 则必须是原生返回的嵌套成功响应。
    def lifecycle(input_id, state):
        matching = [(index, row) for index, row in enumerate(rows) if row.get("direction") == "stdout"
                    and row.get("type") == "command_lifecycle" and row.get("command_uuid") == input_id
                    and row.get("session_id") == native_id and row.get("state") == state]
        _require(len(matching) == 1)
        return matching[0][0]
    starts_native = {input_id: lifecycle(input_id, "started") for input_id in (running, joined, continuation)}
    for (write_index, _), input_id in zip(users, (running, joined, continuation)):
        acks = [index for index, row in enumerate(rows) if row.get("direction") == "stdout"
                and row.get("type") == "command_lifecycle" and row.get("command_uuid") == input_id
                and row.get("session_id") == native_id and row.get("state") in {"queued", "started"}]
        _require(acks and write_index < min(acks) <= starts_native[input_id])
    controls = [(index, row) for index, row in enumerate(rows) if row.get("direction") == "stdin"
                and row.get("type") == "control_request" and row.get("request_subtype") == "interrupt"]
    _require(len(controls) == 1)
    cancel_index, native_interrupt = controls[0]
    request_id = _id(native_interrupt.get("request_id"))
    # 原生取消控制 ID 必须属于实际监督运行代，旧运行 ACK 不能借同会话 ID 迁入。
    _require(re.fullmatch(r"infinishell-" + re.escape(generation) + r"-[1-9][0-9]*", request_id) is not None)
    native_acks = [(index, row) for index, row in enumerate(rows) if row.get("direction") == "stdout"
                   and row.get("type") == "control_response" and row.get("response_request_id") == request_id]
    _require(len(native_acks) == 1 and native_acks[0][1].get("response_subtype") == "success")
    cancelled_index = lifecycle(running, "cancelled")
    result_rows = [(index, row) for index, row in enumerate(rows) if row.get("direction") == "stdout" and row.get("type") == "result"]
    _require(len(result_rows) == 2)
    batch_index, native_batch = result_rows[0]
    continue_index, native_continue = result_rows[1]
    batch_ids = native_batch.get("user_message_uuids")
    _require(isinstance(batch_ids, list) and len(batch_ids) == 2 and set(batch_ids) == {running, joined}
             and native_batch.get("user_message_uuid") in {running, joined} and native_batch.get("session_id") == native_id)
    batch_result_uuid = _uuid(native_batch.get("uuid"))
    reason = native_batch.get("terminal_reason")
    subtype = native_batch.get("subtype")
    _require((reason in {"aborted_streaming", "aborted_tools"} and subtype == "error_during_execution" and native_batch.get("is_error") is True)
             or (reason in {"interrupted", "cancelled"} and native_batch.get("is_error") is False
                 and subtype == "success"))
    _require(native_continue.get("session_id") == native_id and native_continue.get("subtype") == "success"
             and native_continue.get("is_error") is False and native_continue.get("user_message_uuid") == continuation
             and native_continue.get("user_message_uuids") == [continuation]
             and native_continue.get("terminal_reason") in {None, "completed", "end_turn"})
    continue_result_uuid = _uuid(native_continue.get("uuid"))
    _require(continue_result_uuid != batch_result_uuid and starts_native[running] < starts_native[joined] < cancel_index
             and cancel_index < native_acks[0][0] and cancel_index < cancelled_index and cancel_index < batch_index
             and max(native_acks[0][0], cancelled_index, batch_index) < users[2][0]
             and users[2][0] < starts_native[continuation] < continue_index)
    proof = _one(events, "native_batch_cancel_verified")
    _require(proof.get("native_session_id") == native_id and proof.get("execution_turn_id") == running
             and proof.get("joined_input_id") == joined and proof.get("interrupt_request_id") == request_id
             and proof.get("native_result_uuid") == batch_result_uuid and proof.get("terminal_reason") == reason
             and proof.get("is_error") is native_batch.get("is_error") and proof.get("native_input_ack_count") == 2
             and isinstance(proof.get("user_message_uuids"), list)
             and len(proof["user_message_uuids"]) == 2 and set(proof["user_message_uuids"]) == {running, joined})
    _flags(proof, ("native_interrupt_ack_verified", "native_execution_cancelled_verified",
                   "native_complete_batch_result_verified", "all_three_signals_verified"))
    _require(position(cleanup) < position(projections[0])
             and position(projections[-1]) < position(proof) < position(ending))
    return {"native_session_id": native_id, "runtime_generation": generation,
            "execution_input_id": running, "joined_input_id": joined, "continuation_input_id": continuation,
            "interrupt_message_id": interrupt_id, "interrupt_request_id": request_id,
            "batch_result_uuid": batch_result_uuid, "continue_result_uuid": continue_result_uuid,
            "native_protocol_records": len(rows), "profile_sha256": profile_hash}


def verified_acceptance(exit_code, output, events):
    if exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output):
        return False
    try:
        audit_events(events)
    except (ValueError, TypeError, KeyError, AttributeError):
        return False
    return True


def run(args):
    # 兼容旧 Namespace；未知版本在配置准备、API 读取和原生执行之前拒绝。
    release_contract(getattr(args, "claude_version", VERSION))
    if args.api_environment_file is None:
        raise ValueError("合并取消固定策略必须显式提供 API 环境文件")
    if not isinstance(args.model, str) or not args.model or any(character in args.model for character in "\x00\r\n"):
        raise ValueError("合并取消验收必须显式固定模型")
    private = Path(tempfile.mkdtemp(prefix="infinishell-claude-batch-cancel-auth-")).resolve()
    args.config_dir, args.auth_home = private / "claude", private / "home"
    args.config_dir.mkdir(mode=0o700)
    args.auth_home.mkdir(mode=0o700)
    adapter.validate_paths(args)
    if os.name != "nt" and args.api_environment_file.stat().st_mode & 0o077:
        raise ValueError("API 环境文件必须仅当前用户可读写")
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
                     "private_auth_workspace": str(private), "max_native_inputs": 3,
                     "native_executions_expected": 2, "max_inspect_calls": 1,
                     "same_turn_steering_supported": False, "joined_batch_cancel_verified": False,
                     "http_request_count_verified": False, "app_restart_and_ui_verified": False,
                     "persistence_verified": False, "parent_permission_ceiling_verified": False})
    if metadata.get("acceptance_passed") is True:
        try:
            events = [json.loads(line) for line in args.output.read_text(encoding="utf-8").splitlines()]
            metadata["native_batch_cancel_proof"] = audit_events(events)
            metadata["joined_batch_cancel_verified"] = True
        except (ValueError, TypeError, KeyError, AttributeError):
            # 公共证据发生漂移或被替换时，不能保留先前的成功旗；错误不附带任意正文。
            metadata["acceptance_passed"] = False
            metadata["runner_error"] = "公开证据的独立合并取消审核未通过"
            result = 1
    path.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--claude-version", choices=tuple(RELEASE_CATALOG), default=VERSION,
                        help="精确官方版本；缺省保留 2.1.273")
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--claude", type=Path, required=True, help="所选固定官方版本的原生可执行文件")
    parser.add_argument("--supervisor", type=Path, required=True, help="同提交主程序或 TUI 监督入口")
    parser.add_argument("--api-environment-file", type=Path, required=True, help="显式私有 Anthropic API 环境 JSON")
    parser.add_argument("--model", required=True, help="必须固定真实原生模型")
    parser.add_argument("--output", type=Path, required=True, help="全新 .ndjson 证据路径")
    args = parser.parse_args()
    try:
        return run(args)
    except (OSError, ValueError, adapter.subprocess.SubprocessError) as error:
        parser.exit(2, f"合并取消运行器启动失败：{type(error).__name__}；请检查显式文件参数。\n")


if __name__ == "__main__":
    raise SystemExit(main())
