#!/usr/bin/env python3
"""显式运行真实 Claude 父子生产协调器验收；认证与证据均有独立边界，不执行 GUI 验收。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import uuid

import run_claude_adapter_live as base
from prepare_claude_cli import RELEASE_CATALOG, VERSION, current_platform, verify_binary, verify_version


TEST_NAME = "ai::cli_agent_runtime::coordinator::claude_live_tests::real_claude_fixed_profile_parent_child"
SCOPE = "real_claude_production_coordinator"
TEST_CANDIDATE_VERSION = "2.1.280"
TEST_CANDIDATE_BUILD_ARGUMENT = "--infinishell-claude-21280-test-build"
TEST_CANDIDATE_BUILD_MARKER = "infinishell-claude-21280-test-build-v1"
TEST_CANDIDATE_STATE_MARKER_CONTENT = "isolated Claude Code 2.1.280 coordinator candidate\n"


def coordinator_candidate(args):
    version = getattr(args, "claude_version", VERSION)
    enabled = getattr(args, "allow_claude_21280_coordinator_candidate", False)
    if enabled and version != TEST_CANDIDATE_VERSION:
        raise ValueError("Claude 父子候选标记仅允许精确 2.1.280")
    if version == TEST_CANDIDATE_VERSION and sys.platform != "darwin":
        raise ValueError("Claude 2.1.280 父子验收仅开放 macOS")
    if version == TEST_CANDIDATE_VERSION and not getattr(args, "use_authorized_default_account", False):
        raise ValueError("Claude 2.1.280 父子验收仅使用显式授权的默认账户")
    if version == TEST_CANDIDATE_VERSION and any(getattr(args, name, None) is not None for name in
                       ("config_dir", "auth_home", "api_environment_file")):
        raise ValueError("Claude 2.1.280 父子验收不能指定私有认证路径或 API 环境")
    return enabled


def verify_candidate_supervisor(path):
    environment = {"PATH": os.environ.get("PATH", "/usr/bin:/bin")}
    if sys.platform == "darwin":
        signature = subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(path)],
                                   env=environment, capture_output=True, timeout=10)
        if signature.returncode != 0:
            raise ValueError("Claude 专用测试 supervisor 签名核对失败")
    probe = subprocess.run([str(path), TEST_CANDIDATE_BUILD_ARGUMENT], env=environment,
                           capture_output=True, text=True, timeout=5)
    if (probe.returncode != 0 or probe.stdout != TEST_CANDIDATE_BUILD_MARKER + "\n"
            or probe.stderr):
        raise ValueError("supervisor 未声明 Claude 2.1.280 专用测试构建")


def _value_sha256(value):
    encoded = json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def _permission_summary(value):
    """仅公开固定策略和原生模式证明；完整 settings 观察留在私有原始证据。"""
    if not isinstance(value, dict):
        raise ValueError("原生权限记录必须为对象")
    summary = {key: project_public_value(value[key]) for key in (
        "claudeRestrictedFilesV1", "fixedProfileSha256", "fixedProfileVerified",
        "permissionMode", "sessionAssociationConfirmed") if key in value}
    summary["private_permissions_sha256"] = _value_sha256(value)
    observation = value.get("permissionObservation")
    if isinstance(observation, dict):
        summary["permissionObservationSummary"] = {
            key: observation[key] for key in (
                "atomicPermissionCeilingProven", "connectionGeneration", "dispatchAuthorized",
                "modeAfter", "modeBefore") if key in observation}
        summary["permissionObservationSummary"].update({
            "private_observation_sha256": _value_sha256(observation),
            "rule_count": len(observation.get("rules", [])),
            "rejection_count": len(observation.get("rejections", [])),
        })
    return summary


def _config_summary(value):
    if not isinstance(value, dict):
        raise ValueError("任务配置必须为对象")
    summary = {key: project_public_value(value[key]) for key in (
        "claude_profile", "cli_version", "local_tools", "model", "permission_ceiling",
        "permission_policy", "runtime_generation", "selected_skills", "claude_current_input",
        "claude_pending_input", "claude_joined_inputs") if key in value}
    if "effective_permissions" in value:
        summary["effective_permissions"] = _permission_summary(value["effective_permissions"])
    summary["private_config_sha256"] = _value_sha256(value)
    return summary


def project_public_value(value):
    """解析嵌套 JSON 字符串后投影，避免 inspect 的正文重新泄露完整权限树。"""
    if isinstance(value, list):
        return [project_public_value(item) for item in value]
    if not isinstance(value, dict):
        return value
    result = {}
    for key, child in value.items():
        if key == "effective_permissions":
            result[key] = _permission_summary(child)
        elif key == "permissionObservation":
            result["private_observation_sha256"] = _value_sha256(child)
        elif key == "config_json":
            result[key] = json.dumps(_config_summary(json.loads(child)), ensure_ascii=False)
        elif key in ("body", "terminal_evidence") and isinstance(child, str) and child.lstrip().startswith(("{", "[")):
            result[key] = json.dumps(project_public_value(json.loads(child)), ensure_ascii=False)
        else:
            result[key] = project_public_value(child)
    return result


def project_public_events(events, redact):
    result = []
    for event in events:
        if not isinstance(event, dict):
            raise ValueError("私有证据事件必须为对象")
        projected = project_public_value(event)
        # 回合事件只需任务身份；完整配置在最终 SQLite 证据中公开一次摘要。
        if projected.get("event") == "runtime" and isinstance(projected.get("task"), dict):
            config = projected["task"].pop("config_json", None)
            if config is not None:
                parsed = json.loads(config)
                projected["task"]["policy_summary"] = {key: parsed[key] for key in (
                    "private_config_sha256", "permission_policy", "cli_version", "runtime_generation") if key in parsed}
        projected["private_record_sha256"] = _value_sha256(event)
        result.append(base.sanitize_event(projected, redact))
    return result


def validate_auth_selection(args):
    coordinator_candidate(args)
    default_account = getattr(args, "use_authorized_default_account", False)
    api_file = getattr(args, "api_environment_file", None)
    if default_account:
        if api_file is not None:
            raise ValueError("默认在线账户模式不能同时提供 API 环境")
        if getattr(args, "claude_version", VERSION) not in (base.DEFAULT_ACCOUNT_VERSION, TEST_CANDIDATE_VERSION):
            raise ValueError("默认在线账户验收只接受固定官方 Claude 2.1.278 或 macOS 2.1.280")
    elif api_file is None:
        raise ValueError("私有 API 模式必须提供 api-environment-file")
    return default_account


def validate_inputs(args):
    default_account = validate_auth_selection(args)
    for name in ("test_binary", "claude", "supervisor", "api_environment_file"):
        path = getattr(args, name, None)
        if path is None:
            continue
        if path.is_symlink() or getattr(path.lstat(), "st_file_attributes", 0) & 0x400:
            raise ValueError("输入不能使用符号链接或重解析点")
        path = path.resolve(strict=True)
        if not path.is_file():
            raise ValueError("输入必须是文件")
        if name == "api_environment_file" and sys.platform != "win32" and path.stat().st_mode & 0o077:
            raise ValueError("API 环境文件必须仅当前用户可读写")
        setattr(args, name, path)
    if args.test_binary == args.supervisor:
        raise ValueError("生产监督入口不能使用 libtest")
    args.output = args.output.resolve()
    if args.output.suffix != ".ndjson":
        raise ValueError("证据输出必须使用 .ndjson 扩展名")
    inputs = {args.test_binary, args.claude, args.supervisor,
              getattr(args, "api_environment_file", None)}
    for output in (args.output, args.output.with_suffix(".metadata.json"), args.output.with_suffix(".test-output.txt")):
        default_config = (Path.home() / ".claude").resolve()
        if (output in inputs or output.exists() or output.is_symlink()
                or default_account and output.is_relative_to(default_config)):
            raise ValueError("输出不得覆盖输入或既有证据")


def _file_has_no_sensitive_values(path, redact):
    if not path.exists() or not path.is_file():
        return False
    text = path.read_text(encoding="utf-8", errors="replace")
    return redact(text) == text


def _one(events, kind):
    rows = [row for row in events if row.get("event") == kind]
    if len(rows) != 1:
        raise ValueError("验收证据缺失或重复")
    return rows[0]


def _completed(task, task_id, generation, marker, terminal_evidence=True):
    correct = (task.get("task_id") == task_id and task.get("generation") == generation
            and task.get("state") == "completed"
            and isinstance(task.get("result"), str) and marker in task["result"]
            and (not terminal_evidence or bool(task.get("terminal_evidence"))))
    if not correct or not terminal_evidence:
        return correct
    proof = json.loads(task["terminal_evidence"])
    result = proof["event"]["TurnFinished"]
    return (proof.get("native_session_id") == task.get("native_session_id")
            and bool(proof.get("native_session_id")) and bool(result.get("turn_id"))
            and result.get("outcome") == "Completed" and result.get("output") == task["result"])


def _native_ack(message, source, target, sender_generation, subject):
    return (message.get("sender_task_id") == source
            and message.get("recipient_task_id") == target
            and message.get("sender_generation") == sender_generation
            and message.get("recipient_generation") == 1
            and message.get("subject") == subject and bool(message.get("message_id"))
            and message.get("state") == "acknowledged"
            and message.get("receipt_kind") == "native_protocol")


def _native_result_correlations(events):
    records = []
    for row in events:
        if not isinstance(row, dict) or row.get("event") != "runtime":
            continue
        progress = row.get("runtime", {}).get("kind", {}).get("Progress")
        try:
            value = json.loads(progress["message"]) if isinstance(progress, dict) else None
        except (KeyError, TypeError, ValueError):
            value = None
        if isinstance(value, dict) and value.get("kind") == "native_result_correlated_v1":
            records.append({"result_id": value.get("result_id"),
                "session_id": row["runtime"].get("native_session_id"),
                "subtype": value.get("subtype"), "primary_input_id": value.get("primary_input_id"),
                "input_ids": value.get("input_ids")})
    return records


def _audit_inputs(runtime, histories, expected, metrics):
    """按输入到执行的真实关联审核，允许一份原生结果覆盖多个已合并输入。"""
    starts = [(index, row) for index, row in enumerate(runtime) if "TurnStarted" in row["runtime"]["kind"]]
    joins = [(index, row) for index, row in enumerate(runtime) if "InputJoined" in row["runtime"]["kind"]]
    ends = [(index, row) for index, row in enumerate(runtime) if "TurnFinished" in row["runtime"]["kind"]]
    if (metrics.get("native_inputs") != len(expected) or metrics.get("native_executions") != len(starts)
            or metrics.get("joined_inputs") != len(joins) or len(starts) + len(joins) != len(expected)
            or len(ends) != len(expected) or len(starts) != len(histories)):
        return False
    executions, inputs, persisted_joins = {}, {}, {}
    for index, row in starts:
        task_id, generation = row["task"]["task_id"], row["task"]["generation"]
        key = (task_id, generation)
        turn_id = row["runtime"]["kind"]["TurnStarted"]["turn_id"]
        saved = histories[key]
        proof = json.loads(saved["terminal_evidence"])["event"]["TurnFinished"]
        input_key = (task_id, turn_id)
        if key in executions or input_key in inputs or input_key not in expected or proof["turn_id"] != turn_id:
            return False
        config = json.loads(saved["config_json"])
        if config.get("claude_current_input") != {"turn_id": turn_id,
                "submission_generation": expected[input_key]["submission_generation"]}:
            return False
        executions[key] = {"turn_id": turn_id, "start_index": index, "ids": {turn_id}}
        inputs[input_key] = {"execution": key, "index": index, "joined": False}
    if set(executions) != set(histories):
        return False
    for index, row in joins:
        task_id, generation = row["task"]["task_id"], row["task"]["generation"]
        key = (task_id, generation)
        joined = row["runtime"]["kind"]["InputJoined"]
        input_key = (task_id, joined["message_id"])
        if (key not in executions or input_key in inputs or input_key not in expected
                or joined["turn_id"] != executions[key]["turn_id"] or executions[key]["start_index"] >= index):
            return False
        record = {"message_id": joined["message_id"], "turn_id": joined["turn_id"],
                  "submission_generation": expected[input_key]["submission_generation"], "outcome": "Completed"}
        persisted_joins.setdefault(key, []).append(record)
        inputs[input_key] = {"execution": key, "index": index, "joined": True}
        executions[key]["ids"].add(joined["message_id"])
    if set(inputs) != set(expected):
        return False
    for key, saved in histories.items():
        records = [record for prior, values in persisted_joins.items()
                   if prior[0] == key[0] and prior[1] <= key[1] for record in values]
        actual = json.loads(saved["config_json"]).get("claude_joined_inputs", [])
        if actual != records:
            return False
    finished = {}
    for index, row in ends:
        task_id, generation = row["task"]["task_id"], row["task"]["generation"]
        key = (task_id, generation)
        result = row["runtime"]["kind"]["TurnFinished"]
        input_key = (task_id, result["turn_id"])
        if (input_key not in inputs or input_key in finished or inputs[input_key]["execution"] != key
                or index <= inputs[input_key]["index"] or result["outcome"] != "Completed"
                or result["output"] != histories[key]["result"]
                or expected[input_key]["marker"] not in result["output"]):
            return False
        if not inputs[input_key]["joined"] and json.loads(histories[key]["terminal_evidence"])["event"]["TurnFinished"] != result:
            return False
        finished[input_key] = index
    if set(finished) != set(expected):
        return False
    for input_key, input_record in inputs.items():
        if input_record["joined"]:
            task_id, generation = input_record["execution"]
            if finished[input_key] >= finished[(task_id, executions[(task_id, generation)]["turn_id"])]:
                return False
    session_bound = set()
    for row in runtime:
        key = (row["task"]["task_id"], row["task"]["generation"])
        if key not in histories:
            return False
        saved = histories[key]
        config = json.loads(saved["config_json"])
        if row["runtime"].get("generation") != config["runtime_generation"]:
            return False
        native_id = row["runtime"].get("native_session_id")
        if native_id == saved["native_session_id"]:
            session_bound.add(key[0])
            continue
        # 初始化先验证固定权限，再由首条原生输入关联会话；这类 Ready 不是执行证据。
        kind = row["runtime"]["kind"]
        ready = kind.get("SessionReady", {}).get("effective_permissions", {})
        if (native_id is not None or row["task"].get("native_session_id") is not None
                or key[1] != 1 or key[0] in session_bound or set(kind) != {"SessionReady"}
                or ready.get("fixedProfileVerified") is not True or ready.get("permissionMode") != "default"
                or ready.get("sessionAssociationConfirmed") is not False
                or ready.get("claudeRestrictedFilesV1") != config["claude_profile"]
                or ready.get("fixedProfileSha256") != config["effective_permissions"].get("fixedProfileSha256")):
            return False
    native_results = _native_result_correlations(runtime)
    if (len(native_results) != len(executions)
            or len({row.get("result_id") for row in native_results}) != len(native_results)):
        return False
    observed = set()
    for result in native_results:
        ids = result.get("input_ids")
        if (not isinstance(ids, list) or not ids or len(ids) != len(set(ids))
                or result.get("primary_input_id") not in ids or not result.get("result_id")
                or result.get("subtype") != "success"):
            return False
        matches = [key for key, execution in executions.items()
                   if histories[key]["native_session_id"] == result.get("session_id") and execution["ids"] == set(ids)]
        if len(matches) != 1 or matches[0] in observed:
            return False
        observed.add(matches[0])
    return observed == set(executions)


def _runtime_host_cleanup_receipt(receipt, runtime_generation):
    if not isinstance(receipt, dict):
        return False
    try:
        uuid.UUID(receipt["host_instance_id"])
    except (KeyError, TypeError, ValueError, AttributeError):
        return False
    last = receipt.get("last_event_sequence")
    acknowledged = receipt.get("acknowledged_sequence")
    return (receipt.get("version") == 2
            and receipt.get("runtime_generation") == runtime_generation
            and receipt.get("native_process") == "exited"
            and receipt.get("adapter_succeeded") is True
            and receipt.get("adapter_task_terminated") is True
            and receipt.get("event_journal_completed") is True
            and all(re.fullmatch(r"[0-9a-f]{64}", receipt.get(key, "")) for key in (
                "manifest_sha256", "journal_sha256", "native_cleanup_sha256"))
            and isinstance(last, int) and not isinstance(last, bool) and last >= 0
            and isinstance(acknowledged, int) and not isinstance(acknowledged, bool)
            and 0 <= acknowledged <= last)


def verified_acceptance(exit_code, output, events, expected_version=VERSION):
    """校验实际持久记录及原生工具结果；结束行的布尔声明不能单独证明通过。"""
    if (not isinstance(expected_version, str) or expected_version not in RELEASE_CATALOG
            or exit_code != 0 or not re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", output)
            or not isinstance(events, list) or any(not isinstance(row, dict) for row in events)
            or any(row.get("event") in ("acceptance_failed", "cleanup_failed") for row in events)):
        return False
    try:
        started = _one(events, "acceptance_started")
        ending = _one(events, "acceptance_passed")
        chain = _one(events, "saved_chain_verified")
        gate = _one(events, "native_ack_before_child_edit_allow")
        finished = _one(events, "coordinator_chain_finished")
        if (started.get("scope") != SCOPE or ending.get("scope") != SCOPE
                or ending.get("real_gui_verified") is not False
                or ending.get("automatic_result_delivery_ack_verified") is not True
                or any(ending.get(key) is not True for key in (
                    "production_spawn_verified", "saved_profile_equal",
                    "native_message_ack_both_directions", "final_result_via_inspect"))
                or finished.get("native_tool_calls") != 4 or finished.get("native_inputs") not in (5, 6)
                or finished.get("approvals_allowed") != 5
                or finished.get("child_edit_effect_verified") is not True):
            return False
        parent, child = chain["parent"], chain["child"]
        parent_id, child_id = parent["task_id"], child["task_id"]
        parent_generation, child_generation = parent["generation"], child["generation"]
        markers = chain["markers"]
        if (not parent_id or not child_id or parent_id == child_id
                or any(not isinstance(markers.get(key), str) or not markers[key] for key in (
                    "initial", "final_result", "collected", "parent_progress", "automatic_result", "followup", "progress"))
                or child.get("parent_task_id") != parent_id or child.get("parent_generation") != 1
                or parent.get("harness") != "claude" or child.get("harness") != "claude"
                or parent.get("working_directory") != child.get("working_directory")
                or not parent.get("native_session_id") or not child.get("native_session_id")
                or parent["native_session_id"] == child["native_session_id"]
                or parent_generation not in (1, 2, 3, 4) or child_generation not in (1, 2)
                or not _completed(parent, parent_id, parent_generation, markers["automatic_result"])
                or not _completed(child, child_id, child_generation, markers["final_result"])
                or finished.get("expected_child_file_content") != markers["initial"]):
            return False
        histories = {}
        for name, task_id, generation, first, second in (
                ("parent_generations", parent_id, parent_generation, markers["collected"], markers["parent_progress"]),
                ("child_generations", child_id, child_generation, markers["initial"], markers["final_result"])):
            rows = chain[name]
            if len(rows) != generation or {row["generation"] for row in rows} != set(range(1, generation + 1)):
                return False
            histories[name] = {row["generation"]: row for row in rows}
            # 子输入折入同一执行时，文件效果单独核验；不伪造独立的首轮输出。
            first_marker = second if task_id == child_id and generation == 1 else first
            if (not _completed(histories[name][1], task_id, 1, first_marker)
                    or not any(_completed(task, task_id, task["generation"], second) for task in rows)):
                return False
        source = histories["parent_generations"][1]
        parent_config = json.loads(source["config_json"])
        child_config = json.loads(child["config_json"])
        profile = parent_config["claude_profile"]
        ceiling = {
            "parent_task_id": parent_id, "parent_generation": 1,
            "parent_native_session_id": source["native_session_id"],
            "working_directory": source["working_directory"],
            "permissions": {"claudeRestrictedFilesV1": profile},
        }
        if (profile != child_config["claude_profile"]
                or child_config["effective_permissions"]["claudeRestrictedFilesV1"] != profile
                or child_config["effective_permissions"]["fixedProfileVerified"] is not True
                or child_config["effective_permissions"]["permissionMode"] != "default"
                or profile.get("localTools") != {"allow_spawn": True, "allow_message": True}
                or child_config["permission_ceiling"] != ceiling):
            return False
        for task in [parent, child, *chain["parent_generations"], *chain["child_generations"]]:
            config = json.loads(task["config_json"])
            if (config.get("cli_version") != expected_version
                    or config.get("claude_profile") != profile
                    or config.get("permission_policy") != "ClaudeRestrictedFilesV1"
                    or config.get("effective_permissions", {}).get("claudeRestrictedFilesV1") != profile
                    or config.get("effective_permissions", {}).get("fixedProfileVerified") is not True
                    or config.get("effective_permissions", {}).get("permissionMode") != "default"):
                return False
        messages = chain["messages"]
        ids = [row["message_id"] for row in messages]
        if len(ids) != len(set(ids)):
            return False
        followups = [row for row in messages if row.get("subject") == markers["followup"]]
        progresses = [row for row in messages if row.get("subject") == markers["progress"]]
        if (len(followups) != 1 or len(progresses) != 1
                or not _native_ack(followups[0], parent_id, child_id, 1, markers["followup"])
                or not _native_ack(progresses[0], child_id, parent_id, child_generation, markers["progress"])
                or gate.get("child_id") != child_id or gate.get("child_generation") != 1
                or gate.get("child_edit_waiting") is not True
                or gate.get("matching_messages") != followups):
            return False
        result_messages = [row for row in messages if row.get("subject") == "local_task_result"
                           and row.get("sender_task_id") == child_id
                           and row.get("recipient_task_id") == parent_id
                           and row.get("recipient_generation") == 1]
        if (len(result_messages) != child_generation
                or {row["sender_generation"] for row in result_messages} != set(range(1, child_generation + 1))):
            return False
        for message in result_messages:
            generation = message["sender_generation"]
            task = histories["child_generations"][generation]
            expected_id = str(uuid.UUID(bytes=hashlib.sha256(
                f"infinishell-result:{child_id}:{generation}".encode()).digest()[:16]))
            body = json.loads(message["body"])
            if (message["message_id"] != expected_id
                    or not _native_ack(message, child_id, parent_id, generation, "local_task_result")
                    or body != {"task_id": child_id, "generation": generation, "state": "completed",
                        "result": task["result"], "result_bytes": len(task["result"].encode()), "truncated": False,
                        "terminal_evidence": task["terminal_evidence"], "evidence_truncated": False}):
                return False
        calls = [row for row in messages if row.get("subject") == "native_tool_call"]
        operations = []
        arguments = {}
        for row in calls:
            request = json.loads(row["body"])
            operations.append((row["sender_task_id"], row["sender_generation"], request["tool"]))
            args = request["arguments"]
            arguments[(row["sender_task_id"], row["sender_generation"], request["tool"])] = args
            if request["tool"] == "send_message_to_agent":
                expected = (child_id, markers["followup"]) if row["sender_task_id"] == parent_id else (parent_id, markers["progress"])
                if args.get("addresses") != [expected[0]] or args.get("subject") != expected[1]:
                    return False
            elif request["tool"] == "inspect_local_tasks" and args.get("task_ids") != [child_id]:
                return False
            elif request["tool"] == "run_agents" and (
                    args.get("harness") != "claude" or args.get("model_id") != parent_config.get("model")
                    or args.get("skills") != [] or len(args.get("agent_run_configs", [])) != 1):
                return False
        if set(operations) != {
                (parent_id, 1, "run_agents"), (parent_id, 1, "send_message_to_agent"),
                (parent_id, 1, "inspect_local_tasks"), (child_id, child_generation, "send_message_to_agent")} or len(operations) != 4:
            return False
        results = []
        for row in messages:
            if row.get("subject") == "native_tool_result":
                result = json.loads(row["body"])
                if set(result) != {"Ok"}:
                    return False
                results.append((row["sender_task_id"], row["sender_generation"], result["Ok"]))
        if len(results) != 4:
            return False
        spawned = [result for source_id, generation, result in results
                   if source_id == parent_id and generation == 1 and result.get("status") == "queued"]
        inspections = [result for source_id, generation, result in results
                       if source_id == parent_id and generation == 1 and "tasks" in result]
        if (len(spawned) != 1 or len(spawned[0].get("children", [])) != 1
                or spawned[0]["children"][0].get("task_id") != child_id
                or len(inspections) != 1 or len(inspections[0]["tasks"]) != 1
                or not _completed(inspections[0]["tasks"][0], child_id, child_generation, markers["final_result"], False)
                or inspections[0]["tasks"][0].get("native_session_id") != child["native_session_id"]):
            return False
        for sender_id, sender_generation, message in ((parent_id, 1, followups[0]), (child_id, child_generation, progresses[0])):
            sent = [result for source_id, generation, result in results
                    if source_id == sender_id and generation == sender_generation
                    and result.get("status") == "acknowledged"]
            if (len(sent) != 1 or sent[0].get("message_ids") != [message["message_id"]]
                    or sent[0].get("receipts") != [{"message_id": message["message_id"], "receipt_kind": "native_protocol"}]):
                return False
        approvals = [row for row in events if row.get("event") == "approval_allowed"]
        if (len(approvals) != 5 or len({row["approval"]["approval_id"] for row in approvals}) != 5
                or any(row.get("decision") != "AllowOnce" for row in approvals)):
            return False
        approved_operations = set()
        for row in approvals:
            details = row["approval"]["details"]
            tool = details["tool_name"]
            operation = (row["task_id"], row["generation"], tool.removeprefix("mcp__infinishell-local-tasks__"))
            if operation in approved_operations:
                return False
            approved_operations.add(operation)
            if tool == "Edit":
                edit = dict(details["input"])
                replace_all = edit.pop("replace_all", False)
                if (operation != (child_id, 1, "Edit")
                        or replace_all is not False
                        or edit != {"file_path": child["working_directory"] + "/child-approved.txt",
                            "old_string": "CHILD_BEFORE", "new_string": markers["initial"]}):
                    return False
            elif (not tool.startswith("mcp__infinishell-local-tasks__")
                    or operation not in arguments or details["input"] != arguments[operation]):
                return False
        runtime = [row for row in events if row.get("event") == "runtime"]
        all_histories = {(task["task_id"], task["generation"]): task
                         for name in ("parent_generations", "child_generations") for task in chain[name]}
        expected_inputs = {}
        for task_id, first, marker in ((parent_id, source, markers["collected"]),
                (child_id, histories["child_generations"][1],
                 markers["initial"] if child_generation == 2 else markers["final_result"])):
            turn_id = json.loads(first["terminal_evidence"])["event"]["TurnFinished"]["turn_id"]
            expected_inputs[(task_id, turn_id)] = {"marker": marker, "submission_generation": 1}
        for message, marker in ((followups[0], markers["final_result"]),
                (progresses[0], markers["parent_progress"]),
                *[(message, markers["automatic_result"]) for message in result_messages]):
            expected_inputs[(message["recipient_task_id"], message["message_id"])] = {
                "marker": marker, "submission_generation": message["recipient_generation"]}
        if len(expected_inputs) != 4 + child_generation or not _audit_inputs(
                runtime, all_histories, expected_inputs, finished):
            return False
        cleanup = [row for row in events if row.get("event") == "cleanup_confirmed"]
        return (len(cleanup) == 2 and {row.get("task_id") for row in cleanup} == {parent_id, child_id}
                and all(_runtime_host_cleanup_receipt(row.get("receipt"), json.loads(
                    parent["config_json"] if row["task_id"] == parent_id else child["config_json"])["runtime_generation"])
                    for row in cleanup))
    except (KeyError, TypeError, ValueError, AttributeError):
        return False


def run(args):
    # 先按固定发行摘要校验；不向任意 PATH 命中的程序传入 API 环境执行 --version。
    candidate = coordinator_candidate(args)
    selected_version = getattr(args, "claude_version", VERSION)
    formal_current = selected_version == TEST_CANDIDATE_VERSION and not candidate
    if selected_version == TEST_CANDIDATE_VERSION:
        # 父子运行器独立核对路径；正式版本不能借用基础运行器的候选标记。
        validate_inputs(args)
    target = current_platform()
    verified_cli = verify_binary(args.claude, target, selected_version)
    root = Path(tempfile.mkdtemp(prefix="infinishell-claude-coordinator-")).resolve()
    detected = verify_version(args.claude, root, selected_version)
    if verify_binary(args.claude, target, selected_version) != verified_cli:
        raise ValueError("原生 Claude 在版本探测期间变化")
    if candidate:
        verify_candidate_supervisor(args.supervisor)
    elif formal_current:
        signature = subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(args.supervisor)],
            env={"PATH": "/usr/bin:/bin"}, capture_output=True, timeout=10)
        if signature.returncode != 0:
            raise ValueError("Claude 正式 supervisor 签名核对失败")
    default_account = validate_auth_selection(args)
    api_environment = ({} if default_account else
                       base.load_api_environment(args.api_environment_file))
    if default_account:
        args.auth_home, args.config_dir = None, None
    else:
        # 私有 API 模式每次创建全新 HOME／配置，不接受既有原生登录域，也不复制凭据。
        args.auth_home, args.config_dir = root / "home", root / "claude"
        args.auth_home.mkdir(mode=0o700)
        args.config_dir.mkdir(mode=0o700)
        base.validate_paths(args)
    settings = base.prepare_project(root)
    (root / ".infinishell-claude-coordinator-probe").write_text(SCOPE, encoding="utf-8")
    if candidate:
        with (root / base.TEST_CANDIDATE_MARKER).open("x", encoding="utf-8", newline="\n") as target_file:
            target_file.write(base.TEST_CANDIDATE_MARKER_CONTENT)
    blocked = root / "project/blocked.txt"
    blocked.write_text("This file is outside the allowed read policy.\n", encoding="utf-8")
    child_file = root / "project/child-approved.txt"
    child_file.write_text("CHILD_BEFORE", encoding="utf-8")
    blocked_digest = base.digest(blocked)
    settings.write_text(json.dumps({"permissions": {
        "ask": ["Edit"], "deny": [f"Read(/{blocked.as_posix()})"]
    }}) + "\n", encoding="utf-8")
    account_status = None
    data_profile = None
    if default_account:
        environment = base.authorized_default_account_environment(root)
        account_status = base.probe_authorized_default_account(
            args.claude, environment.copy(), root / "project")
        data_profile = f"claude-coordinator-{uuid.uuid4().hex}"
        environment["WARP_DATA_PROFILE"] = data_profile
        auth_mode = "authorized_default_account"
    else:
        environment = base.authenticated_environment(root, args.config_dir, args.auth_home)
        environment.update(api_environment)
        auth_mode = "private_api"
    raw_artifact = root / "coordinator.raw.ndjson"
    with raw_artifact.open("x", encoding="utf-8"):
        pass
    raw_artifact.chmod(0o600)
    environment.update({
        "INFINISHELL_CLAUDE_LIVE_ROOT": str(root),
        "INFINISHELL_CLAUDE_LIVE_AUTH_MODE": auth_mode,
        "INFINISHELL_CLAUDE_LIVE_EXECUTABLE": str(args.claude),
        "INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION": selected_version,
        "INFINISHELL_CLAUDE_LIVE_ARTIFACT": str(raw_artifact),
        "INFINISHELL_CLAUDE_LIVE_MODEL": args.model,
        "INFINISHELL_CLI_SUPERVISOR_EXECUTABLE": str(args.supervisor),
    })
    if candidate:
        environment["INFINISHELL_CLAUDE_COORDINATOR_CANDIDATE_21280"] = "1"
    if args.config_dir is not None:
        environment["INFINISHELL_CLAUDE_LIVE_CONFIG_DIR"] = str(args.config_dir)
    if data_profile is not None:
        environment["INFINISHELL_CLAUDE_LIVE_STATE_PROFILE"] = data_profile
    redact = lambda value: base.sanitize(value, root, args.config_dir, args.auth_home, api_environment)
    repository = Path(__file__).resolve().parents[2]
    metadata = {
        "test": TEST_NAME, "scope": SCOPE, "platform": sys.platform,
        "cli": verified_cli, "cli_version": detected, "requested_cli_version": selected_version,
        "model": args.model,
        "test_binary_sha256": base.digest(args.test_binary), "supervisor_binary_sha256": base.digest(args.supervisor),
        "private_workspace": str(root), "project_settings_sha256": base.digest(settings),
        "private_workspace_preserved": True, "runner_modifies_auth_configuration": False,
        "fresh_private_auth_home": not default_account,
        "authentication_source": ("authorized_default_account" if default_account else
                                  "explicit_api_environment"),
        "native_credential_files_read_or_copied_by_runner": False,
        "api_environment_values_recorded": False,
        "native_cli_may_refresh_credentials": True, "real_gui_verified": False,
        "automatic_result_delivery_ack_verified": False, "model_requests_expected": True,
        "max_native_tools": 4, "max_native_inputs": 6, "max_native_executions": 6, "max_test_seconds": 450,
        "http_request_count_verified": False, "runner_timeout_seconds": 600, "acceptance_passed": False,
        "public_evidence_projection_version": 1, "private_raw_evidence_preserved": True,
        "private_raw_evidence_filename": raw_artifact.name,
        "test_only_coordinator_candidate_21280": candidate,
        "candidate_supervisor_marker_verified": candidate,
        "production_version_gate_expected": formal_current,
    }
    if account_status is not None:
        metadata["authorized_default_account"] = account_status
        metadata["native_cli_credential_access_expected"] = True
        metadata["credential_access_boundary"] = "native_cli_only"
        metadata["isolated_data_profile"] = data_profile
    else:
        metadata["native_credential_files_read_or_copied"] = False
    for command, key in ((["git", "rev-parse", "HEAD"], "commit"), (["git", "status", "--porcelain"], "worktree_dirty")):
        result = subprocess.run(command, cwd=repository, text=True, capture_output=True, check=True)
        metadata[key] = bool(result.stdout.strip()) if key == "worktree_dirty" else result.stdout.strip()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    output = ""
    events = []
    owns_output = False
    private_output = root / "coordinator.raw.test-output.txt"
    try:
        with args.output.open("x", encoding="utf-8"):
            pass
        args.output.chmod(0o600)
        owns_output = True
        process = subprocess.Popen(
            [str(args.test_binary), TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
            cwd=repository, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace")
        try:
            output, _ = process.communicate(timeout=600)
        except subprocess.TimeoutExpired:
            metadata["timed_out"] = True
            process.kill()
            output, _ = process.communicate(timeout=15)
        except BaseException:
            process.kill()
            process.wait(timeout=15)
            raise
        metadata["test_exit_code"] = process.returncode
        if candidate or formal_current:
            metadata["supervisor_binary_unchanged"] = (
                base.digest(args.supervisor) == metadata["supervisor_binary_sha256"])
        if candidate:
            metadata["candidate_supervisor_binary_unchanged"] = (
                metadata["supervisor_binary_unchanged"])
        raw_text = redact(raw_artifact.read_text(encoding="utf-8"))
        raw_artifact.write_text(raw_text, encoding="utf-8")
        raw_events = [json.loads(line) for line in raw_text.splitlines() if line.strip()]
        metadata["production_version_gate_verified"] = (formal_current
            and _one(raw_events, "acceptance_started").get("production_version_gate_verified") is True)
        metadata["native_result_correlations"] = base.sanitize_event(
            _native_result_correlations(raw_events), redact)
        # 验收先审核私有全树，再审核公开摘要；删除字段不能把失败记录变成通过。
        private_verified = verified_acceptance(process.returncode, output,
            [base.sanitize_event(row, redact) for row in raw_events], selected_version)
        events = project_public_events(raw_events, redact)
        metadata["private_raw_evidence_sha256"] = base.digest(raw_artifact)
        metadata["private_event_count"] = len(raw_events)
        metadata["public_event_count"] = len(events)
        metadata["project_settings_unchanged"] = base.digest(settings) == metadata["project_settings_sha256"]
        metadata["denied_read_fixture_unchanged"] = base.digest(blocked) == blocked_digest
        metadata["cli_binary_unchanged"] = verify_binary(args.claude, target, selected_version) == verified_cli
        metadata["acceptance_passed"] = (not metadata.get("timed_out", False)
            and (not (candidate or formal_current) or metadata["supervisor_binary_unchanged"])
            and (not formal_current or metadata["production_version_gate_verified"])
            and metadata["project_settings_unchanged"] and metadata["denied_read_fixture_unchanged"]
            and metadata["cli_binary_unchanged"]
            and private_verified and verified_acceptance(process.returncode, output, events, selected_version))
        metadata["automatic_result_delivery_ack_verified"] = metadata["acceptance_passed"]
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        metadata["runner_error"] = redact(f"{type(error).__name__}: {error}")
    finally:
        raw_text = redact(raw_artifact.read_text(encoding="utf-8", errors="replace"))
        raw_artifact.write_text(raw_text, encoding="utf-8")
        if not private_output.exists():
            private_output.write_text(redact(output), encoding="utf-8")
            private_output.chmod(0o600)
        else:
            private_output.write_text(redact(private_output.read_text(
                encoding="utf-8", errors="replace")), encoding="utf-8")
        metadata["private_test_output_filename"] = private_output.name
        metadata["private_test_output_sha256"] = base.digest(private_output)
        if owns_output and args.output.exists():
            args.output.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in events), encoding="utf-8")
            metadata["public_evidence_sha256"] = base.digest(args.output)
        metadata["private_raw_evidence_sha256"] = base.digest(raw_artifact)
        public_output = args.output.with_suffix(".test-output.txt")
        public_output.write_text(redact(output), encoding="utf-8")
        scan = {
            args.output.name: _file_has_no_sensitive_values(args.output, redact),
            raw_artifact.name: _file_has_no_sensitive_values(raw_artifact, redact),
            private_output.name: _file_has_no_sensitive_values(private_output, redact),
            public_output.name: _file_has_no_sensitive_values(public_output, redact),
        }
        metadata["sensitive_value_scan"] = scan
        metadata["sensitive_value_scan_passed"] = all(scan.values())
        metadata["acceptance_passed"] = (metadata["acceptance_passed"]
                                         and metadata["sensitive_value_scan_passed"])
        metadata = base.sanitize_event(metadata, redact)
        metadata_output = args.output.with_suffix(".metadata.json")
        metadata_text = json.dumps(metadata, ensure_ascii=False, indent=2) + "\n"
        if redact(metadata_text) != metadata_text:
            metadata["sensitive_value_scan_passed"] = False
            metadata["acceptance_passed"] = False
            metadata_text = json.dumps(base.sanitize_event(metadata, redact),
                                       ensure_ascii=False, indent=2) + "\n"
        metadata_output.write_text(metadata_text, encoding="utf-8")
        for receipt in (args.output, public_output, metadata_output, raw_artifact, private_output):
            if receipt.exists():
                receipt.chmod(0o600)
    print("真实 Claude 生产协调器父子验收" + ("通过" if metadata["acceptance_passed"] else "未通过"))
    print(f"证据：{args.output}")
    print(f"私有工作目录已保留：{root}")
    return 0 if metadata["acceptance_passed"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--claude", type=Path, required=True)
    parser.add_argument("--claude-version", choices=tuple(RELEASE_CATALOG), default=VERSION,
                        help="精确官方版本；私有 API 模式缺省保留 2.1.273")
    parser.add_argument("--allow-claude-21280-coordinator-candidate", action="store_true",
                        help="仅专用测试构建；显式启用 Claude 2.1.280 父子候选")
    parser.add_argument("--supervisor", type=Path, required=True)
    parser.add_argument("--api-environment-file", type=Path)
    parser.add_argument("--use-authorized-default-account", action="store_true",
                        help="显式使用用户已授权的默认 Claude 在线订阅；与 API 环境互斥")
    parser.add_argument("--model", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        args.config_dir, args.auth_home = None, None
        # 对路径与独立输出先做校验；新私有认证目录在 run 内创建。
        validate_inputs(args)
        return run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"验收未启动：{type(error).__name__}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
