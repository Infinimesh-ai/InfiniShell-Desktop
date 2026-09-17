#!/usr/bin/env python3
"""以官方隔离链验证 Grok 根任务生产协调器；需要下一快照的已验证历史安全事件。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import tempfile
import time

import run_grok_official_adapter_live as official

TEST_NAME = "ai::cli_agent_runtime::coordinator::grok_live_tests::real_grok_root_coordinator"
SCOPE = "real_grok_root_production_coordinator"
MAX_NATIVE_INPUTS = 8
CONTENT = "GROK_COORDINATOR_APPROVAL"
HISTORY_TRACE = "GROK_NATIVE_FINAL_HISTORY_VERIFIED "
PHASES = ("first_turn", "second_turn", "approval_allow", "approval_deny",
    "queue_primary", "queue_instruction", "cancel", "resume_result")
GENERATIONS = (1, 2, 3, 4, 5, 5, 7, 8)
OUTCOMES = ("Completed", "Completed", "Completed", "Cancelled",
    "Completed", "Completed", "Cancelled", "Completed")


def sha(value):
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def require(condition):
    if not condition:
        raise ValueError("独立协调器审计缺少真实关联证据")


def one(events, name, **fields):
    found = [event for event in events if event.get("event") == name
        and all(event.get(key) == value for key, value in fields.items())]
    require(len(found) == 1)
    return found[0]


def valid_uuid(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", value)


def input_link_projection(value):
    require(isinstance(value, dict) and set(value) == {"message_id", "submission_generation",
        "runtime_generation", "native_turn_id"})
    require(valid_uuid(value["message_id"]) and valid_uuid(value["runtime_generation"]))
    require(type(value["submission_generation"]) is int and 0 < value["submission_generation"] <= 2**63 - 1)
    require(value["native_turn_id"] is None or valid_uuid(value["native_turn_id"]))
    return {key: value[key] for key in ("message_id", "submission_generation",
        "runtime_generation", "native_turn_id")}


def pending_inputs_projection(value):
    if value is None:
        return None
    require(isinstance(value, list) and len(value) <= MAX_NATIVE_INPUTS)
    return [input_link_projection(entry) for entry in value]


def approval_identity_projection(value):
    require(isinstance(value, str))
    if valid_uuid(value):
        return value
    if value.startswith("grok:"):
        try:
            identity = json.loads(value[5:])
        except ValueError:
            identity = None
        if ((type(identity) is int and 0 <= identity <= 2**64 - 1) or valid_uuid(identity)):
            if "grok:" + json.dumps(identity, separators=(",", ":")) == value:
                return value
    return {"identity_sha256": sha(value)}


def project_file_projection(project):
    # 文件名可能来自模型；只公开两个固定目标，其他相对名称仅记录数量与摘要。
    known, unexpected = [], []
    for path in project.rglob("*"):
        if not path.is_file():
            continue
        name = str(path.relative_to(project))
        if name in ("approval-allow.txt", "approval-deny.txt"):
            known.append(name)
        else:
            unexpected.append(sha(name))
    return {"project_files": sorted(known), "project_file_count": len(known) + len(unexpected),
        "unexpected_project_file_count": len(unexpected),
        "unexpected_project_file_name_sha256": sorted(unexpected)}


def native_histories(output):
    result = []
    for line in output.splitlines():
        if HISTORY_TRACE not in line:
            continue
        value = json.loads(line.split(HISTORY_TRACE, 1)[1])
        require(set(value) == {"runtime_generation", "session_id", "turn_id",
            "completion_watermark", "outcome", "full_output_bytes", "full_output_sha256"})
        for key in ("runtime_generation", "session_id", "turn_id"):
            require(valid_uuid(value[key]))
        require(value["outcome"] in ("Completed", "Cancelled"))
        require(type(value["full_output_bytes"]) is int and value["full_output_bytes"] >= 0)
        require(isinstance(value["full_output_sha256"], str)
            and re.fullmatch(r"[0-9a-f]{64}", value["full_output_sha256"]))
        require(isinstance(value["completion_watermark"], str)
            and re.fullmatch(re.escape(value["session_id"]) + r"-(?:0|[1-9][0-9]{0,19})", value["completion_watermark"]))
        result.append(value)
    return result


def task_projection(task):
    config = json.loads(task["config_json"])
    require(task["harness"] == "grok" and task.get("parent_task_id") is None
        and task.get("parent_generation") is None and config.get("permission_policy") == "Inherit")
    require(all(config.get(key) is None for key in ("permission_ceiling", "claude_profile", "local_tools", "model")))
    require(config.get("selected_skills") == [])
    proof = json.loads(task["terminal_evidence"]) if task.get("terminal_evidence") else {}
    terminal = proof.get("event", {}).get("TurnFinished", {})
    result = task.get("result")
    if terminal:
        require(terminal.get("output") == result)
    current = config.get("grok_current_input")
    current = input_link_projection(current) if current is not None else None
    pending = pending_inputs_projection(config.get("grok_pending_inputs"))
    outcome = terminal.get("outcome")
    if isinstance(outcome, dict) and set(outcome) == {"Failed"}:
        outcome = "Failed"
    else:
        outcome = outcome if isinstance(outcome, str) and outcome in ("Completed", "Cancelled", "Failed") else None
    return {"task_id": task["task_id"], "harness": "grok", "parent_task_id": None,
        "parent_generation": None, "generation": task["generation"], "revision": task["revision"],
        "state": task["state"], "native_session_id": task.get("native_session_id"),
        "result_bytes": len(result.encode("utf-8")) if result is not None else None,
        "result_sha256": sha(result) if result is not None else None,
        "terminal_evidence_present": task.get("terminal_evidence") is not None,
        "terminal_native_session_id": proof.get("native_session_id"),
        "terminal_turn_id": terminal.get("turn_id"), "terminal_outcome": outcome,
        "runtime_generation": config.get("runtime_generation"), "permission_policy": "Inherit",
        "permission_ceiling": None, "claude_profile": None, "local_tools": None,
        "model": None, "selected_skills": [], "grok_pending_inputs": pending,
        "grok_current_input": current}


def message_projection(message):
    action = json.loads(message["body"])
    require(isinstance(action, dict) and set(action) == {"Submit"})
    submitted = action["Submit"]
    require(isinstance(submitted, dict) and set(submitted) == {"input"})
    content = submitted["input"]
    require(isinstance(content, list) and len(content) == 1 and isinstance(content[0], dict)
        and set(content[0]) == {"Text"} and isinstance(content[0]["Text"], str))
    result = {key: message.get(key) for key in ("message_id", "sender_task_id", "recipient_task_id",
        "sender_generation", "recipient_generation", "subject", "state", "receipt_kind")}
    result.update(body_bytes=len(message["body"].encode("utf-8")), body_sha256=sha(message["body"]))
    return result


def sqlite_projection(path):
    # 只读取本次隔离任务数据库，不将完整配置、正文或消息体写入公开报告。
    require(path.is_file() and not path.is_symlink())
    with sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True) as connection:
        tasks = [json.loads(row[0]) for row in connection.execute("SELECT data FROM local_cli_tasks")]
        history = [json.loads(row[0]) for row in connection.execute(
            "SELECT data FROM local_cli_task_generations ORDER BY generation")]
        messages = [json.loads(row[0]) for row in connection.execute(
            "SELECT data FROM local_cli_messages ORDER BY sequence")]
    require(len(tasks) == 1)
    return {"task": task_projection(tasks[0]), "history": [task_projection(task) for task in history],
        "messages": [message_projection(message) for message in messages]}


def audit_acceptance(exit_code, output, events, database):
    require(exit_code == 0 and re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output))
    start = one(events, "acceptance_started")
    require(start["scope"] == SCOPE and start["max_native_inputs"] == 8
        and start["production_runtime_commands"] is True and start["test_only_internal_command_switch"] is False)
    require(start["permission_policy"] == "Inherit" and start["local_tools"] is None)
    for key in ("real_gui_verified", "app_restart_verified", "sdk_verified", "parent_child_verified",
            "parent_permission_ceiling_verified", "credential_files_read_by_probe"):
        require(start[key] is False)
    require(not any(event.get("event") in ("acceptance_failed", "cleanup_failed") for event in events))
    passed = one(events, "acceptance_passed")
    require(passed["scope"] == SCOPE)
    for key, value in {"native_inputs": 8, "native_executions": 8, "native_acknowledged_inputs": 8,
            "completed_inputs": 6, "cancelled_inputs": 2, "approvals_verified": 2}.items():
        require(type(passed[key]) is int and passed[key] == value)
    for key in ("sqlite_verified", "same_native_session_verified", "cleanup_confirmed"):
        require(passed[key] is True)
    for key in ("real_gui_verified", "app_restart_verified", "sdk_verified", "parent_child_verified",
            "parent_permission_ceiling_verified"):
        require(passed[key] is False)
    submitted = [event for event in events if event.get("event") == "input_submitted"]
    require(len(submitted) == 8 and [event["phase"] for event in submitted] == list(PHASES))
    require([event["submission_generation"] for event in submitted] == list(GENERATIONS))
    require([event["expected_outcome"] for event in submitted] == list(OUTCOMES))
    require([event["submitted_input_count"] for event in submitted] == list(range(1, 9)))
    require(len({event["message_id"] for event in submitted}) == 8)
    runtime = [event for event in events if event.get("event") == "runtime"]
    require(not any(event["kind"] in ("InputJoined", "RequestFailed", "LocalToolRequested") for event in runtime))
    require(len([event for event in runtime if event["kind"] == "MessageAccepted"]) == 8)
    require(len([event for event in runtime if event["kind"] == "TurnStarted"]) == 8)
    finished = [event for event in runtime if event["kind"] == "TurnFinished"]
    require(len(finished) == 8)
    histories = native_histories(output)
    require(len(histories) == 8 and len({event["turn_id"] for event in histories}) == 8)
    native_ids = {event["native_session_id"] for event in finished}
    task_ids = {event["task_id"] for event in finished}
    require(len(native_ids) == 1 and len(task_ids) == 1)
    native_id = native_ids.pop()
    task_id = task_ids.pop()
    require(valid_uuid(native_id) and valid_uuid(task_id))
    final = one(events, "sqlite_snapshot", label="final")
    require(database == {key: final[key] for key in ("task", "history", "messages")})
    require(len(database["history"]) == 8 and len(database["messages"]) == 8)
    require(database["task"] == database["history"][-1])
    require([task["generation"] for task in database["history"]] == list(range(1, 9)))
    turns = []
    for index, item in enumerate(submitted):
        message_id = item["message_id"]
        require(valid_uuid(message_id) and valid_uuid(item["runtime_generation"]))
        ack = one(events, "runtime", kind="MessageAccepted", message_id=message_id)
        turn = ack["turn_id"]
        require(valid_uuid(turn) and ack["native_receipt"] is True)
        require(ack["runtime_generation"] == item["runtime_generation"])
        started = one(events, "runtime", kind="TurnStarted", turn_id=turn)
        ended = one(events, "runtime", kind="TurnFinished", turn_id=turn)
        require(events.index(item) < events.index(ack) < events.index(started) < events.index(ended))
        for event in (ack, started, ended):
            require(event["task_id"] == task_id and event["native_session_id"] == native_id
                and event["runtime_generation"] == item["runtime_generation"])
        require(ended["message_id"] == message_id and ended["phase"] == item["phase"]
            and ended["outcome"] == OUTCOMES[index] and ended["task_generation"] == index + 1
            and started["task_generation"] == index + 1
            and ended["submission_generation"] == GENERATIONS[index])
        proof = [event for event in histories if event["turn_id"] == turn]
        require(len(proof) == 1)
        proof = proof[0]
        require(proof["session_id"] == native_id and proof["runtime_generation"] == item["runtime_generation"]
            and proof["outcome"] == ended["outcome"] and proof["full_output_bytes"] == ended["output_bytes"]
            and proof["full_output_sha256"] == ended["output_sha256"])
        task = database["history"][index]
        require(task["harness"] == "grok" and task["parent_task_id"] is None
            and task["parent_generation"] is None and task["permission_policy"] == "Inherit"
            and task["selected_skills"] == [] and all(task[key] is None
                for key in ("permission_ceiling", "claude_profile", "local_tools", "model")))
        require(task["task_id"] == task_id and task["native_session_id"] == native_id
            and task["state"] == OUTCOMES[index].lower() and task["terminal_evidence_present"] is True
            and task["terminal_native_session_id"] == native_id
            and task["terminal_turn_id"] == turn and task["terminal_outcome"] == OUTCOMES[index]
            and task["result_bytes"] == ended["output_bytes"] and task["result_sha256"] == ended["output_sha256"]
            and task["runtime_generation"] == item["runtime_generation"])
        link = task["grok_current_input"]
        require(input_link_projection(link) == link
            and pending_inputs_projection(task["grok_pending_inputs"]) == task["grok_pending_inputs"])
        require(isinstance(link, dict) and link["message_id"] == message_id
            and link["native_turn_id"] == turn and link["submission_generation"] == GENERATIONS[index]
            and link["runtime_generation"] == item["runtime_generation"])
        message = database["messages"][index]
        require(type(item["body_bytes"]) is int and item["body_bytes"] >= 0
            and isinstance(item["body_sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", item["body_sha256"])
            and item["body_bytes"] == message["body_bytes"] and item["body_sha256"] == message["body_sha256"])
        require(message["message_id"] == message_id and message["subject"] == "user_input"
            and message["state"] == "acknowledged" and message["receipt_kind"] == "native_protocol"
            and message["sender_task_id"] == message["recipient_task_id"] == task_id
            and message["sender_generation"] == message["recipient_generation"] == GENERATIONS[index])
        if item["expected_marker"] is not None:
            marker = item["expected_marker"]
            require(ended["expected_marker_matched"] is True
                and ended["output_bytes"] == len(marker.encode("utf-8")) and ended["output_sha256"] == sha(marker))
        turns.append(turn)
    require(submitted[0]["expected_marker"] == "GROK_COORD_ONE"
        and submitted[1]["expected_marker"] == "GROK_COORD_TWO")
    queued_marker = submitted[5]["expected_marker"]
    require(isinstance(queued_marker, str) and re.fullmatch(r"GROK_COORD_QUEUED_[0-9a-f]{32}", queued_marker)
        and submitted[7]["expected_marker"] == queued_marker)
    require(submitted[5]["submitted_while_running"] is True and submitted[5]["active_turn_id"] == turns[4])
    queue_text = one(events, "runtime", kind="FirstText", turn_id=turns[4])
    queue_finish = one(events, "runtime", kind="TurnFinished", turn_id=turns[4])
    require(events.index(queue_text) < events.index(submitted[5]) < events.index(queue_finish))
    cancel = one(events, "cancel_submitted")
    cancel_text = one(events, "runtime", kind="FirstText", turn_id=turns[6])
    require(cancel["execution_turn_id"] == turns[6] and cancel["native_input_acknowledged"] is True
        and cancel["real_text_started"] is True and events.index(cancel_text) < events.index(cancel))
    dispatched = one(events, "runtime", kind="CommandDispatched", message_id=cancel["message_id"])
    cancelled = one(events, "runtime", kind="TurnFinished", turn_id=turns[6])
    require(dispatched["turn_id"] == turns[6] and dispatched["native_receipt"] is False
        and dispatched["runtime_generation"] == submitted[6]["runtime_generation"]
        and events.index(cancel) < events.index(dispatched) < events.index(cancelled))
    require(len([event for event in runtime if event["kind"] == "ApprovalRequested"]) == 2)
    require(len([event for event in runtime if event["kind"] == "ApprovalResolved"]) == 2)
    for index, decision in ((2, "AllowOnce"), (3, "DenyOnce")):
        approval = one(events, "runtime", kind="ApprovalRequested", turn_id=turns[index])
        require(approval["exact_write_verified"] is True)
        selected = one(events, "approval_decision_submitted", approval_id=approval["approval_id"])
        resolved = one(events, "runtime", kind="ApprovalResolved", approval_id=approval["approval_id"])
        require(selected["decision"] == resolved["decision"] == decision
            and selected["native_receipt"] is False and resolved["native_receipt"] is False)
        dispatched = one(events, "runtime", kind="CommandDispatched", message_id=selected["message_id"])
        require(dispatched["turn_id"] == turns[index] and dispatched["native_receipt"] is False
            and dispatched["runtime_generation"] == submitted[index]["runtime_generation"]
            and events.index(approval) < events.index(selected) < events.index(dispatched) < events.index(resolved))
        effect = one(events, "file_effect_verified", phase=PHASES[index])
        require(effect["allowed"] is (index == 2) and effect["exact_file_state_verified"] is True)
    before = one(events, "sqlite_snapshot", label="before_history_reload")
    ready = one(events, "sqlite_snapshot", label="resume_ready_no_replay")
    require(len(before["messages"]) == 7 and before["messages"] == ready["messages"] == database["messages"][:7])
    require(ready["task"]["generation"] == 8 and ready["task"]["state"] == "queued"
        and ready["task"]["native_session_id"] == native_id and ready["task"]["result_sha256"] is None
        and not ready["task"]["grok_pending_inputs"] and ready["task"]["grok_current_input"] is None)
    require(events.index(ready) < events.index(submitted[7]))
    tokens = {event["runtime_generation"] for event in submitted}
    require(len(tokens) == 2 and submitted[0]["runtime_generation"] != submitted[7]["runtime_generation"]
        and all(event["runtime_generation"] == submitted[0]["runtime_generation"] for event in submitted[:7]))
    cleanup = [event for event in events if event.get("event") == "cleanup_confirmed"]
    require(len(cleanup) == 2 and {event["runtime_generation"] for event in cleanup} == tokens)
    for event in cleanup:
        receipt = event["receipt"]
        require(receipt["generation"] == event["runtime_generation"] and receipt["cleanup_confirmed"] is True
            and receipt["exit_code"] == 0 and receipt["exit_reason"] == "stdio_closed"
            and receipt["containment"] == "macos_resource_coalition")
    require(events.index(cleanup[0]) < events.index(ready) and events.index(cleanup[1]) < events.index(passed))
    return {"native_session_id": native_id, "task_id": task_id, "native_inputs": 8,
        "native_executions": 8, "native_histories": histories, "completed_inputs": 6,
        "cancelled_inputs": 2, "sqlite_final_projection": database, "acceptance_passed": True}


def verified_acceptance(exit_code, output, events, database):
    try:
        audit_acceptance(exit_code, output, events, database)
        return True
    except (KeyError, IndexError, TypeError, ValueError):
        return False


def public_events(events):
    # 每个事件和嵌套对象使用显式字段契约；未知字段即使形似身份或摘要也不公开。
    def choices(*values):
        return lambda value: isinstance(value, str) and value in values

    def nullable(rule):
        return lambda value: value is None or rule(value)

    integer = lambda value: type(value) is int and 0 <= value <= 2**63 - 1
    signed_integer = lambda value: type(value) is int and -(2**63) <= value <= 2**63 - 1
    boolean = lambda value: type(value) is bool
    digest = lambda value: isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value)
    empty = lambda value: isinstance(value, list) and not value
    null = lambda value: value is None
    outcome = choices("Completed", "Cancelled", "Failed")
    marker = lambda value: isinstance(value, str) and (value in ("GROK_COORD_ONE", "GROK_COORD_TWO")
        or re.fullmatch(r"GROK_COORD_QUEUED_[0-9a-f]{32}", value))
    watermark = lambda value: isinstance(value, str) and re.fullmatch(
        r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}-(?:0|[1-9][0-9]{0,19})", value)

    def project(value, schema):
        if not isinstance(value, dict):
            return "<省略不符合契约的字段>"
        result = {}
        for key, rule in schema.items():
            if key not in value:
                continue
            item = value[key]
            try:
                if isinstance(rule, dict):
                    result[key] = project(item, rule)
                elif isinstance(rule, tuple):
                    result[key] = ([project(entry, rule[0]) for entry in item]
                        if isinstance(item, list) and len(item) <= MAX_NATIVE_INPUTS else "<省略不符合契约的字段>")
                elif rule in (input_link_projection, pending_inputs_projection, approval_identity_projection):
                    result[key] = rule(item) if item is not None else None
                else:
                    result[key] = item if rule(item) else "<省略不符合契约的字段>"
            except (KeyError, TypeError, ValueError):
                result[key] = "<省略不符合契约的字段>"
        return result

    task = {"task_id": valid_uuid, "harness": choices("grok"), "parent_task_id": null,
        "parent_generation": null, "generation": integer, "revision": integer,
        "state": choices("queued", "running", "waiting_for_user", "completed", "cancelled",
            "failed", "disconnected", "unconfirmed", "unknown"),
        "native_session_id": nullable(valid_uuid), "result_bytes": nullable(integer),
        "result_sha256": nullable(digest), "terminal_evidence_present": boolean,
        "terminal_native_session_id": nullable(valid_uuid), "terminal_turn_id": nullable(valid_uuid),
        "terminal_outcome": nullable(outcome), "runtime_generation": nullable(valid_uuid),
        "permission_policy": choices("Inherit"), "permission_ceiling": null, "claude_profile": null,
        "local_tools": null, "model": null, "selected_skills": empty,
        "grok_pending_inputs": pending_inputs_projection, "grok_current_input": input_link_projection}
    message = {"message_id": valid_uuid, "sender_task_id": valid_uuid, "recipient_task_id": valid_uuid,
        "sender_generation": integer, "recipient_generation": integer, "subject": choices("user_input"),
        "state": choices("queued", "sent", "acknowledged", "failed", "cancelled"),
        "receipt_kind": nullable(choices("native_protocol")), "body_bytes": integer, "body_sha256": digest}
    receipt = {"version": integer, "generation": valid_uuid, "cleanup_confirmed": boolean,
        "exit_code": nullable(signed_integer), "exit_reason": choices("stdio_closed", "native_exit",
            "host_disconnected", "stop_requested"),
        "containment": choices("macos_resource_coalition"), "manifest_sha256": digest}
    runtime = {"task_id": valid_uuid, "task_generation": integer,
        "runtime_generation": valid_uuid, "native_session_id": nullable(valid_uuid)}
    kinds = {"SessionReady": {}, "MessageAccepted": {"message_id": valid_uuid, "turn_id": valid_uuid,
            "native_receipt": boolean}, "TurnStarted": {"turn_id": valid_uuid},
        "FirstText": {"turn_id": valid_uuid, "text_bytes": integer},
        "TurnFinished": {"turn_id": valid_uuid, "message_id": valid_uuid, "phase": choices(*PHASES),
            "outcome": outcome, "output_bytes": integer, "output_sha256": digest,
            "expected_marker_matched": nullable(boolean), "submission_generation": integer},
        "ApprovalRequested": {"approval_id": approval_identity_projection, "turn_id": valid_uuid,
            "method": choices("session/request_permission"), "exact_write_verified": boolean},
        "ApprovalResolved": {"approval_id": approval_identity_projection, "decision": choices("AllowOnce", "DenyOnce"),
            "native_receipt": boolean},
        "CommandDispatched": {"message_id": valid_uuid, "turn_id": nullable(valid_uuid), "native_receipt": boolean},
        "Disconnected": {}}
    boundaries = {key: boolean for key in ("real_gui_verified", "app_restart_verified", "sdk_verified",
        "parent_child_verified", "parent_permission_ceiling_verified")}
    database = {"task": task, "history": (task,), "messages": (message,)}
    schemas = {
        "acceptance_started": {"scope": choices(SCOPE), "max_native_inputs": integer,
            "production_runtime_commands": boolean, "test_only_internal_command_switch": boolean,
            "permission_policy": choices("Inherit"), "local_tools": null,
            "credential_files_read_by_probe": boolean, **boundaries},
        "input_submitted": {"phase": choices(*PHASES), "message_id": valid_uuid,
            "submission_generation": integer, "runtime_generation": valid_uuid,
            "submitted_while_running": boolean, "active_turn_id": nullable(valid_uuid),
            "expected_marker": nullable(marker), "expected_outcome": outcome, "submitted_input_count": integer,
            "body_bytes": integer, "body_sha256": digest},
        "unexpected_turn_finished": {"turn_id": valid_uuid, "task_generation": integer,
            "runtime_generation": valid_uuid, "native_session_id": nullable(valid_uuid),
            "outcome": outcome, "output_bytes": integer, "output_sha256": digest},
        "approval_decision_submitted": {"approval_id": approval_identity_projection, "message_id": valid_uuid,
            "decision": choices("AllowOnce", "DenyOnce"), "native_receipt": boolean},
        "cancel_submitted": {"message_id": valid_uuid, "execution_turn_id": valid_uuid,
            "native_input_acknowledged": boolean, "real_text_started": boolean},
        "file_effect_verified": {"phase": choices("approval_allow", "approval_deny"), "allowed": boolean,
            "exact_file_state_verified": boolean},
        "sqlite_snapshot": {"label": choices("queued_original_submission", "phase_completed",
            "before_history_reload", "resume_ready_no_replay", "final"), **database},
        "independent_sqlite_audit": database,
        "cleanup_confirmed": {"runtime_generation": valid_uuid, "receipt": receipt},
        "shutdown_tail": {"task_id": valid_uuid, "runtime_generation": valid_uuid},
        "exercise_failed": {"reason_bytes": integer, "reason_sha256": digest},
        "cleanup_failed": {"reason_bytes": integer, "reason_sha256": digest},
        "acceptance_passed": {"scope": choices(SCOPE), **boundaries,
            **{key: integer for key in ("native_inputs", "native_executions", "native_acknowledged_inputs",
                "completed_inputs", "cancelled_inputs", "approvals_verified")},
            "sqlite_verified": boolean, "same_native_session_verified": boolean, "cleanup_confirmed": boolean},
        "acceptance_failed": {"scope": choices(SCOPE)},
        "verified_native_history": {"runtime_generation": valid_uuid, "session_id": valid_uuid,
            "turn_id": valid_uuid, "completion_watermark": watermark,
            "outcome": choices("Completed", "Cancelled"), "full_output_bytes": integer,
            "full_output_sha256": digest}}
    result = []
    for event in events:
        name = event.get("event") if isinstance(event, dict) else None
        if name == "runtime":
            kind = event.get("kind")
            if isinstance(kind, str) and kind in kinds:
                result.append({"event": "runtime", "kind": kind, **project(event, {**runtime, **kinds[kind]})})
            else:
                result.append({"event": "runtime", "kind": "unknown_kind"})
        elif isinstance(name, str) and name in schemas:
            result.append({"event": name, **project(event, schemas[name])})
        else:
            result.append({"event": "unknown_event"})
    return result


def run(args):
    # 复用官方沙箱、opaque auth 副本与严格 TLS origin 白名单，不使用内部命令旁路。
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for path in official.artifacts(args.output):
        descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        os.close(descriptor)
    root = Path(tempfile.mkdtemp(prefix="infinishell-grok-coordinator-", dir="/private/tmp")).resolve()
    for relative in ("home", "home/.grok", "project", "tmp", "state"):
        (root / relative).mkdir(parents=True, exist_ok=True, mode=0o700)
    (root / ".infinishell-grok-coordinator-probe").write_text(SCOPE, encoding="utf-8")
    raw_path = root / "private-evidence.ndjson"
    raw_path.touch(mode=0o600)
    metadata = {"scope": SCOPE, "test_name": TEST_NAME, "platform": "darwin",
        "acceptance_passed": False, "official_grok_model_tested": False,
        "production_runtime_commands": True, "test_only_internal_command_switch": False,
        "minimum_fixture_source": "下一快照包含生产已验证历史安全事件；source11 不满足",
        "app_test_is_gui": False, "app_restart_verified": False, "parent_child_verified": False,
        "parent_permission_ceiling_verified": False, "sdk_verified": False,
        "same_commit_verified_by_runner": False, "cross_platform_verified": False,
        "auth_copy_method": "opaque_auth_json_only", "credential_files_read_by_probe": False,
        "private_workspace": str(root), "max_native_inputs": 8,
        "http_model_call_budget_enforced": False, "cost_budget_enforced": False,
        "requested_model": official.MODEL, "tls_decrypted": False,
        "grok_sha256": official.shared.digest(args.grok),
        "test_binary_sha256": official.shared.digest(args.test_binary),
        "supervisor_sha256": official.shared.digest(args.supervisor)}
    tunnel = official.OfficialTunnel(args.timeout)
    events = []
    output = ""
    settings = None
    before = None
    try:
        port = tunnel.start()
        official.copy_private_auth(args.official_grok_home, root / "home/.grok")
        metadata["sandbox_canary"] = official.shared.network_canary(root, args.official_grok_home / "auth.json", port)
        wrapper, settings = official.prepare_native(root, args.grok, args.official_grok_home, port)
        before = settings.read_bytes()
        environment = official.official_environment(root, port)
        environment.update(INFINISHELL_GROK_LIVE_ROOT=str(root), INFINISHELL_GROK_LIVE_EXECUTABLE=str(wrapper),
            INFINISHELL_GROK_LIVE_ARTIFACT=str(raw_path), INFINISHELL_CLI_SUPERVISOR_EXECUTABLE=str(args.supervisor))
        version = subprocess.run([str(wrapper), "--version"], cwd=root / "project", env=environment,
            capture_output=True, text=True, timeout=10, check=True)
        require(version.stdout.strip() == official.shared.VERSION)
        metadata["grok_version"] = official.shared.VERSION
        command = [str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
        process = subprocess.Popen(command, cwd=Path(__file__).resolve().parents[2], env=environment,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
        try:
            output, _ = process.communicate(timeout=max(1, tunnel.deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            metadata["timed_out"] = True
            process.kill()
            output, _ = process.communicate(timeout=20)
        except BaseException:
            process.kill()
            process.wait(timeout=20)
            raise
        metadata["test_exit_code"] = process.returncode
        events = [json.loads(line) for line in raw_path.read_text().splitlines() if line.strip()]
        database = sqlite_projection(root / "coordinator.sqlite")
        audit = audit_acceptance(process.returncode, output, events, database)
        events.extend({"event": "verified_native_history", **value} for value in audit["native_histories"])
        events.append({"event": "independent_sqlite_audit", "task": database["task"],
            "history": database["history"], "messages": database["messages"]})
        metadata["private_settings_audit"] = official.audit_private_settings(before, settings.read_bytes())
        launches = [json.loads(line) for line in (root / "wrapper-audit.ndjson").read_text().splitlines()]
        leaders = [event for event in launches if event.get("kind") == "private_leader"]
        metadata["native_launches"] = launches
        files = project_file_projection(root / "project")
        metadata.update(files)
        require(files["project_files"] == ["approval-allow.txt"] and files["project_file_count"] == 1
            and files["unexpected_project_file_count"] == 0
            and (root / "project/approval-allow.txt").read_text() == CONTENT)
        model_tunnel = any(event == {"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"} for event in tunnel.events)
        metadata["acceptance_passed"] = (model_tunnel and not metadata.get("timed_out", False)
            and metadata["private_settings_audit"]["settings_scope_verified"]
            and len(leaders) == 2 and len({event["private_socket"] for event in leaders}) == 2
            and all(event["arguments_unchanged"] for event in leaders))
    except (OSError, ValueError, TypeError, KeyError, sqlite3.Error, subprocess.SubprocessError) as error:
        metadata["runner_error_type"] = type(error).__name__
    finally:
        metadata["tunnels_stopped"] = tunnel.close()
        metadata["acceptance_passed"] &= metadata["tunnels_stopped"]
        auth = root / "home/.grok/auth.json"
        try:
            auth.unlink(missing_ok=True)
            metadata["private_auth_copy_removed"] = not auth.exists()
            metadata["acceptance_passed"] &= metadata["private_auth_copy_removed"]
        except OSError as error:
            metadata["private_auth_copy_removed"] = False
            metadata["auth_cleanup_error_type"] = type(error).__name__
            metadata["acceptance_passed"] = False
        metadata["official_grok_model_tested"] = metadata["acceptance_passed"]
        descriptor = os.open(root / "private-test-output.txt", os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as file:
            file.write(output)
        if not events:
            try:
                events = [json.loads(line) for line in raw_path.read_text().splitlines() if line.strip()]
            except (OSError, ValueError):
                metadata["private_evidence_parse_failed"] = True
        # 失败运行也保留可验证的历史投影、SQLite 快照及来源摘要，原验收结论不改。
        metadata["raw_evidence_sha256"] = official.shared.digest(raw_path)
        metadata["private_test_output_sha256"] = sha(output)
        try:
            proofs = native_histories(output)
            metadata["validated_native_history_trace_records"] = len(proofs)
            if not any(event.get("event") == "verified_native_history" for event in events):
                events.extend({"event": "verified_native_history", **proof} for proof in proofs)
        except (ValueError, TypeError, KeyError) as error:
            metadata["history_projection_error_type"] = type(error).__name__
            metadata["acceptance_passed"] = False
        try:
            database = sqlite_projection(root / "coordinator.sqlite")
            metadata["independent_sqlite_projection_available"] = True
            if not any(event.get("event") == "independent_sqlite_audit" for event in events):
                events.append({"event": "independent_sqlite_audit", **database})
        except (OSError, ValueError, TypeError, KeyError, sqlite3.Error) as error:
            metadata["independent_sqlite_projection_available"] = False
            metadata["sqlite_projection_error_type"] = type(error).__name__
            metadata["acceptance_passed"] = False
        try:
            launches = root / "wrapper-audit.ndjson"
            metadata["native_launches"] = ([json.loads(line) for line in launches.read_text().splitlines()]
                if launches.is_file() else [])
            metadata.update(project_file_projection(root / "project"))
            if settings is not None and before is not None:
                metadata["private_settings_audit"] = official.audit_private_settings(before, settings.read_bytes())
        except (OSError, ValueError, TypeError) as error:
            metadata["final_evidence_error_type"] = type(error).__name__
            metadata["acceptance_passed"] = False
        metadata["official_grok_model_tested"] = metadata["acceptance_passed"]
        args.output.write_text("".join(json.dumps(event, ensure_ascii=False) + "\n" for event in public_events(events)), encoding="utf-8")
        args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        args.output.with_suffix(".network.json").write_text(json.dumps({"events": tunnel.events,
            "tls_connections_attempted": tunnel.forwarded, "tls_bytes": tunnel.bytes}, indent=2) + "\n", encoding="utf-8")
    print("官方 Grok 根任务协调器验收" + ("通过" if metadata["acceptance_passed"] else "未通过"))
    print(f"安全证据：{args.output}")
    print(f"私有诊断目录：{root}")
    return 0 if metadata["acceptance_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--grok", type=Path, required=True)
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--official-grok-home", type=Path, required=True)
    parser.add_argument("--max-acp-inputs", type=int, default=8)
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        official.validate_paths(args)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"运行器启动失败：{type(error).__name__}；请核对专用登录与固定输入。\n")


if __name__ == "__main__":
    raise SystemExit(main())
