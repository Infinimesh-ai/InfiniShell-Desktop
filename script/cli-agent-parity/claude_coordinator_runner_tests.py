#!/usr/bin/env python3
"""离线校验父子协调器验收谓词与私有环境边界；不启动 CLI，不请求模型。"""

import argparse
import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_claude_coordinator_live as runner


OUTPUT = "test result: ok. 1 passed; 0 failed; 0 ignored; 99 filtered out;\n"
PARENT = "00000000-0000-4000-8000-000000000001"
CHILD = "00000000-0000-4000-8000-000000000002"


def result_id(generation):
    return str(runner.uuid.UUID(bytes=runner.hashlib.sha256(
        f"infinishell-result:{CHILD}:{generation}".encode()).digest()[:16]))


def result_body(task):
    return json.dumps({"task_id": CHILD, "generation": task["generation"], "state": "completed",
        "result": task["result"], "result_bytes": len(task["result"].encode()), "truncated": False,
        "terminal_evidence": task["terminal_evidence"], "evidence_truncated": False})


def fixture(version=runner.VERSION):
    profile = {
        "version": 1, "workingDirectory": "/probe/project",
        "canonicalWorkingDirectory": "/probe/project", "executableSha256": "a" * 64,
        "denyRules": ["Read(//probe/project/blocked.txt)"],
        "sourceRules": [{"source": "localSettings", "behavior": "deny", "rule": "Read(//probe/project/blocked.txt)"}],
        "localTools": {"allow_spawn": True, "allow_message": True},
    }
    markers = {"initial": "CHILD_INITIAL_1", "final_result": "CHILD_FINAL_1",
               "collected": "PARENT_COLLECTED_1", "parent_progress": "PARENT_PROGRESS_1",
               "automatic_result": "PARENT_AUTOMATIC_1",
               "followup": "followup-1", "progress": "progress-1"}
    parent_config = {"model": "claude-sonnet-4-6", "cli_version": version,
                     "permission_policy": "ClaudeRestrictedFilesV1",
                     "claude_profile": profile, "runtime_generation": "parent-process-1",
                     "effective_permissions": {"claudeRestrictedFilesV1": profile,
                                               "fixedProfileVerified": True, "permissionMode": "default"}}
    ceiling = {"parent_task_id": PARENT, "parent_generation": 1, "parent_native_session_id": "parent-native-1",
               "working_directory": "/probe/project", "permissions": {"claudeRestrictedFilesV1": profile}}
    child_config = copy.deepcopy(parent_config)
    child_config.update({"permission_ceiling": ceiling, "runtime_generation": "child-process-1"})

    def task(task_id, generation, result, turn_id=None):
        native_id = "parent-native-1" if task_id == PARENT else "child-native-1"
        turn_id = turn_id or f"{task_id}-{generation}"
        turn = {"turn_id": turn_id, "outcome": "Completed", "output": result}
        config = copy.deepcopy(parent_config if task_id == PARENT else child_config)
        config["claude_current_input"] = {"turn_id": turn_id, "submission_generation": 1}
        return {"task_id": task_id, "generation": generation, "state": "completed", "result": result,
                "harness": "claude", "working_directory": "/probe/project", "native_session_id": native_id,
                "parent_task_id": PARENT if task_id == CHILD else None,
                "parent_generation": 1 if task_id == CHILD else None,
                "config_json": json.dumps(config),
                "terminal_evidence": json.dumps({"native_session_id": native_id, "event": {"TurnFinished": turn}})}

    parent_one = task(PARENT, 1, markers["collected"])
    parent_two = task(PARENT, 2, markers["parent_progress"])
    parent_three = task(PARENT, 3, markers["automatic_result"], result_id(1))
    parent_four = task(PARENT, 4, markers["automatic_result"], result_id(2))
    child_one = task(CHILD, 1, markers["initial"])
    child_two = task(CHILD, 2, markers["final_result"])

    def message(message_id, sender, recipient, sender_generation, subject, body, state="acknowledged", receipt="native_protocol"):
        return {"message_id": message_id, "sender_task_id": sender, "recipient_task_id": recipient,
                "sender_generation": sender_generation, "recipient_generation": 1, "subject": subject,
                "body": body, "state": state, "receipt_kind": receipt}

    followup = message(f"{CHILD}-2", PARENT, CHILD, 1, markers["followup"], "followup prompt")
    progress = message(f"{PARENT}-2", CHILD, PARENT, 2, markers["progress"], "progress prompt")
    result_messages = [message(result_id(saved["generation"]), CHILD, PARENT, saved["generation"],
                               "local_task_result", result_body(saved)) for saved in (child_one, child_two)]
    messages = [followup, progress, *result_messages]
    calls = [
        (PARENT, 1, "run_agents", {"harness": "claude", "model_id": "claude-sonnet-4-6", "skills": [],
                                   "agent_run_configs": [{"name": "child", "prompt": "authorized child"}]}),
        (PARENT, 1, "send_message_to_agent", {"addresses": [CHILD], "subject": markers["followup"], "message": "followup prompt"}),
        (PARENT, 1, "inspect_local_tasks", {"task_ids": [CHILD]}),
        (CHILD, 2, "send_message_to_agent", {"addresses": [PARENT], "subject": markers["progress"], "message": "progress prompt"}),
    ]
    inspected = {key: value for key, value in child_two.items() if key != "terminal_evidence"}
    results = [
        {"status": "queued", "children": [{"task_id": CHILD, "status": "queued"}]},
        {"status": "acknowledged", "message_ids": [followup["message_id"]],
         "receipts": [{"message_id": followup["message_id"], "receipt_kind": "native_protocol"}]},
        {"tasks": [inspected]},
        {"status": "acknowledged", "message_ids": [progress["message_id"]],
         "receipts": [{"message_id": progress["message_id"], "receipt_kind": "native_protocol"}]},
    ]
    approvals = []
    for index, ((sender, generation, tool, arguments), result) in enumerate(zip(calls, results)):
        row = message(f"call-{index}", sender, sender, generation, "native_tool_call",
                      json.dumps({"tool": tool, "arguments": arguments}), "sent", None)
        row["recipient_generation"] = generation
        messages.append(row)
        row = message(f"tool-result-{index}", sender, sender, generation, "native_tool_result", json.dumps({"Ok": result}), "sent", None)
        row["recipient_generation"] = generation
        messages.append(row)
        approvals.append({"event": "approval_allowed", "task_id": sender, "generation": generation,
                          "decision": "AllowOnce", "approval": {"approval_id": f"approval-{index}",
                          "details": {"tool_name": "mcp__infinishell-local-tasks__" + tool, "input": arguments}}})
    approvals.append({"event": "approval_allowed", "task_id": CHILD, "generation": 1, "decision": "AllowOnce",
                      "approval": {"approval_id": "approval-edit", "details": {"tool_name": "Edit", "input": {
                          "file_path": "/probe/project/child-approved.txt", "old_string": "CHILD_BEFORE",
                          "new_string": markers["initial"], "replace_all": False}}}})
    runtime = []
    for saved in (parent_one, parent_two, parent_three, parent_four, child_one, child_two):
        task_id, generation = saved["task_id"], saved["generation"]
        proof = json.loads(saved["terminal_evidence"])
        process = json.loads(saved["config_json"])["runtime_generation"]
        turn_id = proof["event"]["TurnFinished"]["turn_id"]
        for kind in ({"TurnStarted": {"turn_id": turn_id}},
                     {"Progress": {"turn_id": turn_id, "message": json.dumps({
                         "kind": "native_result_correlated_v1",
                         "result_id": f"result-{task_id}-{generation}", "subtype": "success",
                         "primary_input_id": turn_id, "input_ids": [turn_id]})}},
                     proof["event"]):
            runtime.append({"event": "runtime", "task": saved,
                            "runtime": {"native_session_id": saved["native_session_id"], "generation": process, "kind": kind}})
    cleanup = [{"event": "cleanup_confirmed", "task_id": task_id,
                "receipt": {"version": 2, "runtime_generation": process,
                            "host_instance_id": host_instance_id, "native_process": "exited",
                            "adapter_succeeded": True, "adapter_task_terminated": True,
                            "event_journal_completed": True, "last_event_sequence": 12,
                            "acknowledged_sequence": 11, "manifest_sha256": "b" * 64,
                            "journal_sha256": "c" * 64, "native_cleanup_sha256": "d" * 64}}
               for task_id, process, host_instance_id in (
                   (PARENT, "parent-process-1", "00000000-0000-4000-8000-000000000011"),
                   (CHILD, "child-process-1", "00000000-0000-4000-8000-000000000012"))]
    return [
        {"event": "acceptance_started", "scope": runner.SCOPE},
        {"event": "native_ack_before_child_edit_allow", "child_id": CHILD, "child_generation": 1,
         "child_edit_waiting": True, "matching_messages": [copy.deepcopy(followup)]},
        *runtime, *approvals,
        {"event": "saved_chain_verified", "parent": parent_four, "child": child_two,
         "parent_generations": [parent_one, parent_two, parent_three, parent_four], "child_generations": [child_one, child_two],
         "messages": messages, "markers": markers},
        {"event": "coordinator_chain_finished", "native_tool_calls": 4, "native_inputs": 6,
         "native_executions": 6, "joined_inputs": 0,
         "approvals_allowed": 5, "child_edit_effect_verified": True, "expected_child_file_content": markers["initial"]},
        *cleanup,
        {"event": "acceptance_passed", "scope": runner.SCOPE, "production_spawn_verified": True,
         "saved_profile_equal": True, "native_message_ack_both_directions": True, "final_result_via_inspect": True,
         "real_gui_verified": False, "automatic_result_delivery_ack_verified": True},
    ]


def batch_fixture(parent_joined=True, child_joined=True):
    """按生产关联形状生成合并及自动结果输入，不隐去中间代的真实结果。"""
    events = fixture()
    chain = next(row for row in events if row["event"] == "saved_chain_verified")
    if child_joined:
        source = chain["child_generations"][0]
        source["result"] = chain["markers"]["final_result"]
        proof = json.loads(source["terminal_evidence"])
        proof["event"]["TurnFinished"]["output"] = source["result"]
        source["terminal_evidence"] = json.dumps(proof)
        joined = {"message_id": f"{CHILD}-2", "turn_id": f"{CHILD}-1",
                  "submission_generation": 1, "outcome": "Completed"}
        config = json.loads(source["config_json"])
        config["claude_joined_inputs"] = [joined]
        source["config_json"] = json.dumps(config)
        chain["child"] = source
        chain["child_generations"] = [source]
        chain["messages"] = [row for row in chain["messages"] if row["message_id"] != result_id(2)]
        chain["parent_generations"] = chain["parent_generations"][:3]
        chain["parent"] = chain["parent_generations"][-1]
        events = [row for row in events if not (row.get("event") == "runtime" and (
            (row["task"]["task_id"] == CHILD and row["task"]["generation"] == 2)
            or (row["task"]["task_id"] == PARENT and row["task"]["generation"] == 4)))]
        for row in chain["messages"]:
            if row["sender_task_id"] == CHILD and row["sender_generation"] == 2:
                row["sender_generation"] = 1
                if row["subject"] in ("native_tool_call", "native_tool_result"):
                    row["recipient_generation"] = 1
        for row in events:
            if row.get("event") == "approval_allowed" and row["task_id"] == CHILD:
                row["generation"] = 1
        end_index = next(index for index, row in enumerate(events) if row.get("event") == "runtime"
                         and row["task"]["task_id"] == CHILD and "TurnFinished" in row["runtime"]["kind"])
        events[end_index]["runtime"]["kind"] = proof["event"]
        runtime = {"native_session_id": source["native_session_id"], "generation": config["runtime_generation"]}
        events[end_index:end_index] = [
            {"event": "runtime", "task": source, "runtime": {**runtime, "kind": {"InputJoined": {
                "message_id": joined["message_id"], "turn_id": joined["turn_id"]}}}},
            {"event": "runtime", "task": source, "runtime": {**runtime, "kind": {"TurnFinished": {
                "turn_id": joined["message_id"], "outcome": "Completed", "output": source["result"]}}}},
        ]
    for message in chain["messages"]:
        if message["subject"] == "local_task_result":
            saved = next(task for task in chain["child_generations"] if task["generation"] == message["sender_generation"])
            message["body"] = result_body(saved)
    if parent_joined:
        source = chain["parent_generations"][0]
        source["result"] = "\n".join(chain["markers"][key] for key in ("collected", "parent_progress", "automatic_result"))
        proof = json.loads(source["terminal_evidence"])
        proof["event"]["TurnFinished"]["output"] = source["result"]
        source["terminal_evidence"] = json.dumps(proof)
        ids = [f"{PARENT}-2", *[row["message_id"] for row in chain["messages"] if row["subject"] == "local_task_result"]]
        joined = [{"message_id": value, "turn_id": f"{PARENT}-1", "submission_generation": 1, "outcome": "Completed"} for value in ids]
        config = json.loads(source["config_json"])
        config["claude_joined_inputs"] = joined
        source["config_json"] = json.dumps(config)
        chain["parent"] = source
        chain["parent_generations"] = [source]
        events = [row for row in events if not (row.get("event") == "runtime"
                  and row["task"]["task_id"] == PARENT and row["task"]["generation"] > 1)]
        end_index = next(index for index, row in enumerate(events) if row.get("event") == "runtime"
                         and row["task"]["task_id"] == PARENT and "TurnFinished" in row["runtime"]["kind"])
        events[end_index]["runtime"]["kind"] = proof["event"]
        runtime = {"native_session_id": source["native_session_id"], "generation": config["runtime_generation"]}
        additions = []
        for record in joined:
            additions.append({"event": "runtime", "task": source, "runtime": {**runtime, "kind": {"InputJoined": {
                "message_id": record["message_id"], "turn_id": record["turn_id"]}}}})
        for record in joined:
            additions.append({"event": "runtime", "task": source, "runtime": {**runtime, "kind": {"TurnFinished": {
                "turn_id": record["message_id"], "outcome": "Completed", "output": source["result"]}}}})
        events[end_index:end_index] = additions
    result = next(row for row in chain["messages"] if row["subject"] == "native_tool_result"
                  and "tasks" in json.loads(row["body"])["Ok"])
    result["body"] = json.dumps({"Ok": {"tasks": [{key: value for key, value in chain["child"].items()
                                                 if key != "terminal_evidence"}]}})
    finished = next(row for row in events if row["event"] == "coordinator_chain_finished")
    finished["native_inputs"] = 4 + len(chain["child_generations"])
    finished["joined_inputs"] = sum(1 for row in events if row.get("event") == "runtime" and "InputJoined" in row["runtime"]["kind"])
    finished["native_executions"] = finished["native_inputs"] - finished["joined_inputs"]
    for row in events:
        progress = row.get("runtime", {}).get("kind", {}).get("Progress")
        if not isinstance(progress, dict):
            continue
        correlation = json.loads(progress["message"])
        if correlation.get("kind") != "native_result_correlated_v1":
            continue
        config = json.loads(row["task"]["config_json"])
        turn_id = json.loads(row["task"]["terminal_evidence"])["event"]["TurnFinished"]["turn_id"]
        joined = [record["message_id"] for record in config.get("claude_joined_inputs", [])
                  if record["turn_id"] == turn_id]
        correlation["input_ids"] = [turn_id, *joined]
        correlation["primary_input_id"] = correlation["input_ids"][-1]
        progress["message"] = json.dumps(correlation)
    return events


def native_output(_events):
    return OUTPUT


OUTPUT = native_output(fixture())

class AcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.events = fixture()
        self.chain = next(row for row in self.events if row["event"] == "saved_chain_verified")

    def verify(self):
        return runner.verified_acceptance(0, OUTPUT, self.events)

    def test_complete_structured_evidence_is_accepted_without_gui_claim(self):
        self.assertTrue(self.verify())

    def test_boolean_ending_alone_does_not_establish_native_work(self):
        self.assertFalse(runner.verified_acceptance(0, OUTPUT, [self.events[-1]]))

    def test_zero_tests_does_not_pass(self):
        self.assertFalse(runner.verified_acceptance(0, "test result: ok. 0 passed;", self.events))

    def test_wrong_scope_does_not_pass(self):
        self.events[-1]["scope"] = "gui"
        self.assertFalse(self.verify())

    def test_application_history_cannot_replace_native_message_ack(self):
        self.chain["messages"][1]["receipt_kind"] = "application_history"
        self.assertFalse(self.verify())

    def test_sent_cannot_replace_acknowledged(self):
        self.chain["messages"][0]["state"] = "sent"
        self.assertFalse(self.verify())

    def test_duplicate_message_does_not_pass(self):
        self.chain["messages"].append(copy.deepcopy(self.chain["messages"][0]))
        self.assertFalse(self.verify())

    def test_ack_after_approval_cannot_replace_waiting_approval_gate(self):
        gate = next(row for row in self.events if row["event"] == "native_ack_before_child_edit_allow")
        gate["child_edit_waiting"] = False
        self.assertFalse(self.verify())

    def test_child_cannot_claim_another_parent_generation(self):
        self.chain["child"]["parent_generation"] = 2
        self.assertFalse(self.verify())

    def test_same_native_session_cannot_be_both_parent_and_child(self):
        self.chain["child"]["native_session_id"] = self.chain["parent"]["native_session_id"]
        self.assertFalse(self.verify())

    def test_completed_without_native_terminal_evidence_does_not_pass(self):
        self.chain["child"]["terminal_evidence"] = None
        self.assertFalse(self.verify())

    def test_disconnected_without_completion_does_not_pass(self):
        self.chain["child"]["state"] = "disconnected"
        self.assertFalse(self.verify())

    def test_changed_profile_on_parent_followup_does_not_pass(self):
        config = json.loads(self.chain["parent"]["config_json"])
        config["claude_profile"]["canonicalWorkingDirectory"] = "/changed"
        self.chain["parent"]["config_json"] = json.dumps(config)
        self.assertFalse(self.verify())

    def test_permission_ceiling_cannot_be_expanded(self):
        config = json.loads(self.chain["child"]["config_json"])
        config["permission_ceiling"]["permissions"]["claudeRestrictedFilesV1"]["denyRules"] = []
        self.chain["child"]["config_json"] = json.dumps(config)
        self.assertFalse(self.verify())

    def test_native_inspect_result_must_contain_actual_final_result(self):
        row = next(row for row in self.chain["messages"] if row["subject"] == "native_tool_result" and "tasks" in json.loads(row["body"])["Ok"])
        result = json.loads(row["body"])
        result["Ok"]["tasks"][0]["result"] = "not collected"
        row["body"] = json.dumps(result)
        self.assertFalse(self.verify())

    def test_spawn_result_must_identify_actual_child(self):
        row = next(row for row in self.chain["messages"] if row["subject"] == "native_tool_result" and json.loads(row["body"])["Ok"].get("status") == "queued")
        result = json.loads(row["body"])
        result["Ok"]["children"][0]["task_id"] = "another-child"
        row["body"] = json.dumps(result)
        self.assertFalse(self.verify())

    def test_failed_native_tool_result_does_not_pass(self):
        row = next(row for row in self.chain["messages"] if row["subject"] == "native_tool_result")
        row["body"] = json.dumps({"Err": "native tool failed"})
        self.assertFalse(self.verify())

    def test_out_of_scope_edit_approval_does_not_pass(self):
        row = next(row for row in self.events if row["event"] == "approval_allowed" and row["approval"]["details"]["tool_name"] == "Edit")
        row["approval"]["details"]["input"]["file_path"] = "/outside.txt"
        self.assertFalse(self.verify())

    def test_native_failed_turn_cannot_be_relabelled_completed(self):
        row = next(row for row in self.events if row["event"] == "runtime" and "TurnFinished" in row["runtime"]["kind"])
        row["runtime"]["kind"]["TurnFinished"]["outcome"] = {"Failed": {"message": "failure"}}
        self.assertFalse(self.verify())

    def test_old_native_generation_does_not_pass(self):
        row = next(row for row in self.events if row["event"] == "runtime")
        row["runtime"]["generation"] = "old-process"
        self.assertFalse(self.verify())

    def test_cleanup_receipt_must_match_actual_generation(self):
        row = next(row for row in self.events if row["event"] == "cleanup_confirmed")
        row["receipt"]["runtime_generation"] = "old-process"
        self.assertFalse(self.verify())

    def test_runtime_host_cleanup_requires_confirmed_native_exit(self):
        row = next(row for row in self.events if row["event"] == "cleanup_confirmed")
        row["receipt"]["native_process"] = "unconfirmed"
        self.assertFalse(self.verify())

    def test_runtime_host_cleanup_allows_a_sealed_final_event_without_a_late_ack(self):
        row = next(row for row in self.events if row["event"] == "cleanup_confirmed")
        row["receipt"]["last_event_sequence"] = 42
        row["receipt"]["acknowledged_sequence"] = 41
        self.assertTrue(self.verify())

    def test_runtime_host_cleanup_rejects_an_ack_past_the_journal_end(self):
        row = next(row for row in self.events if row["event"] == "cleanup_confirmed")
        row["receipt"]["acknowledged_sequence"] = row["receipt"]["last_event_sequence"] + 1
        self.assertFalse(self.verify())

    def test_malformed_evidence_does_not_raise_or_pass(self):
        self.assertFalse(runner.verified_acceptance(0, OUTPUT, [None]))
        self.chain["child"]["config_json"] = "not json"
        self.assertFalse(self.verify())


class BatchAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.events = batch_fixture()
        self.chain = next(row for row in self.events if row["event"] == "saved_chain_verified")
        self.output = native_output(self.events)

    def verify(self):
        return runner.verified_acceptance(0, self.output, self.events)

    def test_both_joined_inputs_are_two_executions_not_four(self):
        self.assertTrue(self.verify())
        self.assertTrue(runner.verified_acceptance(0, self.output,
            runner.project_public_events(self.events, lambda value: value)))

    def test_mixed_joined_and_separate_executions_are_distinguished(self):
        for parent_joined, child_joined in ((True, False), (False, True), (False, False)):
            events = batch_fixture(parent_joined, child_joined)
            self.assertTrue(runner.verified_acceptance(0, native_output(events), events))

    def test_joined_input_without_real_input_joined_event_fails(self):
        self.events = [row for row in self.events if not (row.get("event") == "runtime"
                       and "InputJoined" in row["runtime"]["kind"])]
        self.assertFalse(self.verify())

    def test_joined_input_without_native_result_outcome_fails(self):
        config = json.loads(self.chain["child"]["config_json"])
        config["claude_joined_inputs"][0].pop("outcome")
        self.chain["child"]["config_json"] = json.dumps(config)
        self.assertFalse(self.verify())

    def test_last_uuid_alone_cannot_complete_joined_batch(self):
        rows = runner._native_result_correlations(self.events)
        row = next(row for row in self.events if row.get("event") == "runtime"
                   and row["task"]["task_id"] == CHILD
                   and "Progress" in row["runtime"]["kind"])
        progress = row["runtime"]["kind"]["Progress"]
        correlation = json.loads(progress["message"])
        correlation["input_ids"] = [correlation["primary_input_id"]]
        progress["message"] = json.dumps(correlation)
        self.assertEqual(len(rows), 2)
        self.assertFalse(self.verify())

    def test_joined_input_completion_must_precede_active_execution_completion(self):
        indexes = [index for index, row in enumerate(self.events) if row.get("event") == "runtime"
                   and row["task"]["task_id"] == CHILD and "TurnFinished" in row["runtime"]["kind"]]
        self.events[indexes[0]], self.events[indexes[1]] = self.events[indexes[1]], self.events[indexes[0]]
        self.assertFalse(self.verify())

    def test_joined_batch_cannot_count_inputs_as_independent_executions(self):
        finished = next(row for row in self.events if row["event"] == "coordinator_chain_finished")
        finished["native_executions"] = 4
        self.assertFalse(self.verify())

    def test_failed_joined_outcome_cannot_be_relabelled_success(self):
        config = json.loads(self.chain["child"]["config_json"])
        config["claude_joined_inputs"][0]["outcome"] = {"Failed": {"message": "native_failure"}}
        self.chain["child"]["config_json"] = json.dumps(config)
        self.assertFalse(self.verify())

    def test_missing_native_result_trace_cannot_prove_completion(self):
        self.events = [row for row in self.events if not (row.get("event") == "runtime"
                       and "Progress" in row["runtime"]["kind"])]
        self.assertFalse(self.verify())

    def test_five_inputs_are_valid_under_six_input_upper_bound(self):
        finished = next(row for row in self.events if row["event"] == "coordinator_chain_finished")
        self.assertEqual(finished["native_inputs"], 5)
        self.assertTrue(self.verify())
        finished["native_inputs"] = 6
        self.assertFalse(self.verify())

    def test_automatic_result_needs_native_protocol_ack(self):
        message = next(row for row in self.chain["messages"] if row["subject"] == "local_task_result")
        message["receipt_kind"] = "application_history"
        self.assertFalse(self.verify())

    def test_automatic_result_cannot_be_replaced_by_previous_generation_body(self):
        message = next(row for row in self.chain["messages"] if row["subject"] == "local_task_result")
        body = json.loads(message["body"])
        body["result"] = "previous-generation-result"
        message["body"] = json.dumps(body)
        self.assertFalse(self.verify())

    def test_automatic_result_must_have_actual_native_input_completion(self):
        message = next(row for row in self.chain["messages"] if row["subject"] == "local_task_result")
        self.events = [row for row in self.events if not (row.get("event") == "runtime"
                       and row["runtime"]["kind"].get("TurnFinished", {}).get("turn_id") == message["message_id"])]
        self.assertFalse(self.verify())

    def test_independent_child_generations_require_both_automatic_results(self):
        events = batch_fixture(True, False)
        output = native_output(events)
        chain = next(row for row in events if row["event"] == "saved_chain_verified")
        self.assertEqual(len([row for row in chain["messages"] if row["subject"] == "local_task_result"]), 2)
        self.assertTrue(runner.verified_acceptance(0, output, events))
        chain["messages"] = [row for row in chain["messages"] if row["message_id"] != result_id(1)]
        self.assertFalse(runner.verified_acceptance(0, output, events))

    def test_automatic_result_identity_is_derived_from_actual_child_generation(self):
        message = next(row for row in self.chain["messages"] if row["subject"] == "local_task_result")
        message["message_id"] = "another-generation-result-id"
        self.assertFalse(self.verify())

    def test_cleanup_failure_cannot_be_hidden_by_successful_completion(self):
        self.events.append({"event": "cleanup_failed", "reason": "missing_exit_receipt"})
        self.assertFalse(self.verify())


class IsolationTests(unittest.TestCase):
    def test_public_projection_preserves_structured_acceptance(self):
        projected = runner.project_public_events(fixture(), lambda value: value)
        self.assertTrue(runner.verified_acceptance(0, OUTPUT, projected))
        for row in projected:
            self.assertRegex(row["private_record_sha256"], r"^[0-9a-f]{64}$")
        runtime = next(row for row in projected if row["event"] == "runtime")
        self.assertNotIn("config_json", runtime["task"])
        self.assertRegex(runtime["task"]["policy_summary"]["private_config_sha256"], r"^[0-9a-f]{64}$")

    def test_permission_observation_full_tree_remains_private(self):
        observation = {"settings": {"huge_private_tree": "private-observation" * 10000},
                       "atomicPermissionCeilingProven": False, "connectionGeneration": "process-1",
                       "dispatchAuthorized": True, "modeAfter": "plan", "modeBefore": "plan",
                       "rules": ["deny"], "rejections": []}
        event = {"event": "runtime", "runtime": {"kind": {"SessionReady": {
            "effective_permissions": {"permissionObservation": observation, "fixedProfileVerified": True,
                                      "permissionMode": "default", "unapprovedSettings": "private-extra"}}}}}
        projected = runner.project_public_events([event], lambda value: value)[0]
        permissions = projected["runtime"]["kind"]["SessionReady"]["effective_permissions"]
        self.assertNotIn("permissionObservation", permissions)
        self.assertNotIn("unapprovedSettings", permissions)
        self.assertNotIn("private-observation", json.dumps(projected))
        self.assertLess(len(json.dumps(projected)), 1500)
        summary = permissions["permissionObservationSummary"]
        self.assertEqual(summary["private_observation_sha256"], runner._value_sha256(observation))
        self.assertFalse(summary["atomicPermissionCeilingProven"])
        self.assertEqual(summary["rule_count"], 1)

    def test_nested_inspect_configuration_does_not_reintroduce_private_tree(self):
        events = fixture()
        chain = next(row for row in events if row["event"] == "saved_chain_verified")
        result = next(row for row in chain["messages"] if row["subject"] == "native_tool_result"
                      and "tasks" in json.loads(row["body"])["Ok"])
        body = json.loads(result["body"])
        config = json.loads(body["Ok"]["tasks"][0]["config_json"])
        config["effective_permissions"]["permissionObservation"] = {"settings": "private-inspect-tree" * 10000}
        config["unapprovedTaskConfiguration"] = "private-configuration"
        body["Ok"]["tasks"][0]["config_json"] = json.dumps(config)
        result["body"] = json.dumps(body)
        projected = runner.project_public_events(events, lambda value: value)
        encoded = json.dumps(projected)
        self.assertNotIn("private-inspect-tree", encoded)
        self.assertNotIn("private-configuration", encoded)
        self.assertTrue(runner.verified_acceptance(0, OUTPUT, projected))

    def test_projection_cannot_make_an_expanded_ceiling_pass(self):
        events = fixture()
        chain = next(row for row in events if row["event"] == "saved_chain_verified")
        config = json.loads(chain["child"]["config_json"])
        config["permission_ceiling"]["permissions"]["claudeRestrictedFilesV1"]["denyRules"] = []
        chain["child"]["config_json"] = json.dumps(config)
        self.assertFalse(runner.verified_acceptance(0, OUTPUT, runner.project_public_events(events, lambda value: value)))

    def test_projection_cannot_hide_failed_acceptance(self):
        events = fixture() + [{"event": "acceptance_failed", "reason": "native_command_overlap"}]
        self.assertFalse(runner.verified_acceptance(0, OUTPUT, runner.project_public_events(events, lambda value: value)))

    def test_malformed_configuration_is_not_copied_to_public_artifact(self):
        with self.assertRaises(ValueError):
            runner.project_public_events([{"event": "runtime", "task": {"config_json": "private-invalid-json"}}], lambda value: value)

    def test_ambient_provider_keys_are_not_forwarded(self):
        with patch.dict(runner.base.os.environ, {"ANTHROPIC_API_KEY": "ambient-secret",
                                                "XAI_API_KEY": "other-secret", "PATH": "/safe/bin"}, clear=True):
            environment = runner.base.authenticated_environment(Path("/probe"), Path("/fresh/claude"), Path("/fresh/home"))
        self.assertNotIn("ANTHROPIC_API_KEY", environment)
        self.assertNotIn("XAI_API_KEY", environment)
        self.assertEqual(environment["HOME"], str(Path("/fresh/home")))
        self.assertEqual(environment["CLAUDE_CONFIG_DIR"], str(Path("/fresh/claude")))

    def test_api_secrets_are_redacted_in_values_and_json_strings(self):
        environment = {"ANTHROPIC_API_KEY": 'secret-with-"-quote', "ANTHROPIC_BASE_URL": "https://private-api.test"}
        text = json.dumps(environment)
        redacted = runner.base.sanitize(text, Path("/probe"), Path("/fresh/claude"), Path("/fresh/home"), environment)
        self.assertNotIn("private-api.test", redacted)
        self.assertNotIn("secret-with", redacted)

    def test_runner_does_not_accept_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = [root / name for name in ("test", "claude", "supervisor", "api.json")]
            for path in paths:
                path.write_text("placeholder")
                path.chmod(0o600)
            args = argparse.Namespace(test_binary=paths[0], claude=paths[1], supervisor=paths[2], api_environment_file=paths[3], output=root / "evidence.ndjson")
            args.output.write_text("old success")
            with self.assertRaises(ValueError):
                runner.validate_inputs(args)

    def test_api_file_with_shared_read_permission_is_rejected(self):
        if runner.sys.platform == "win32":
            self.skipTest("Windows 使用 ACL，不按 Unix mode 判定")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = [root / name for name in ("test", "claude", "supervisor", "api.json")]
            for path in paths:
                path.write_text("placeholder")
                path.chmod(0o600)
            paths[3].chmod(0o644)
            args = argparse.Namespace(test_binary=paths[0], claude=paths[1], supervisor=paths[2], api_environment_file=paths[3], output=root / "evidence.ndjson")
            with self.assertRaises(ValueError):
                runner.validate_inputs(args)

    def test_default_account_requires_opt_in_and_excludes_api_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = [root / name for name in ("test", "claude", "supervisor", "api.json")]
            for path in paths:
                path.write_text("placeholder")
                path.chmod(0o600)
            common = dict(test_binary=paths[0], claude=paths[1], supervisor=paths[2],
                          output=root / "evidence.ndjson", model="claude-sonnet-4-6")
            online = argparse.Namespace(**common, api_environment_file=None,
                use_authorized_default_account=True, claude_version="2.1.278")
            runner.validate_inputs(online)
            self.assertTrue(runner.validate_auth_selection(online))
            for changes in (
                {"api_environment_file": paths[3]}, {"claude_version": "2.1.273"},
                {"use_authorized_default_account": False},
            ):
                args = argparse.Namespace(**(vars(online) | changes))
                with self.subTest(changes=changes), self.assertRaises(ValueError):
                    runner.validate_inputs(args)

    def test_sensitive_value_scan_requires_already_redacted_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            receipt = root / "receipt.txt"
            redact = lambda value: runner.base.sanitize(value, root, None, None, {})
            receipt.write_text("email=person@example.invalid token=secret-token", encoding="utf-8")
            self.assertFalse(runner._file_has_no_sensitive_values(receipt, redact))
            receipt.write_text(redact(receipt.read_text(encoding="utf-8")), encoding="utf-8")
            self.assertTrue(runner._file_has_no_sensitive_values(receipt, redact))


class InitialReadyAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.events = batch_fixture()
        self.output = native_output(self.events)
        chain = next(row for row in self.events if row["event"] == "saved_chain_verified")
        self.ready_rows = []
        for name in ("parent_generations", "child_generations"):
            saved = chain[name][0]
            config = json.loads(saved["config_json"])
            permissions = copy.deepcopy(config["effective_permissions"])
            permissions["sessionAssociationConfirmed"] = False
            row = {"event": "runtime", "task": {"task_id": saved["task_id"], "generation": 1,
                    "native_session_id": None}, "runtime": {"native_session_id": None,
                    "generation": config["runtime_generation"], "kind": {"SessionReady": {
                        "effective_permissions": permissions}}}}
            self.ready_rows.append(row)
        self.events[1:1] = self.ready_rows

    def verify(self):
        return runner.verified_acceptance(0, self.output, self.events)

    def test_verified_initial_ready_precedes_native_session_association(self):
        self.assertTrue(self.verify())
        self.assertTrue(runner.verified_acceptance(0, self.output,
            runner.project_public_events(self.events, lambda value: value)))

    def test_old_runtime_token_is_rejected_even_on_initial_ready(self):
        self.ready_rows[0]["runtime"]["generation"] = "old-process"
        self.assertFalse(self.verify())

    def test_initial_ready_cannot_claim_an_unverified_fixed_profile(self):
        self.ready_rows[0]["runtime"]["kind"]["SessionReady"]["effective_permissions"]["fixedProfileVerified"] = False
        self.assertFalse(self.verify())

    def test_missing_profile_cannot_replace_verified_initial_ready(self):
        self.ready_rows[0]["runtime"]["kind"]["SessionReady"]["effective_permissions"].pop("claudeRestrictedFilesV1")
        self.assertFalse(self.verify())

    def test_confirmed_ready_cannot_omit_native_session_id(self):
        self.ready_rows[0]["runtime"]["kind"]["SessionReady"]["effective_permissions"]["sessionAssociationConfirmed"] = True
        self.assertFalse(self.verify())

    def test_null_session_is_not_allowed_on_input_or_execution_events(self):
        for kind in ("TurnStarted", "InputJoined", "MessageAccepted"):
            events = copy.deepcopy(self.events)
            row = next(row for row in events if row.get("event") == "runtime"
                       and ("InputJoined" if kind == "MessageAccepted" else kind) in row["runtime"]["kind"])
            if kind == "MessageAccepted":
                acknowledgement = copy.deepcopy(row)
                joined = row["runtime"]["kind"]["InputJoined"]
                acknowledgement["runtime"]["kind"] = {"MessageAccepted": {
                    "message_id": joined["message_id"], "turn_id": joined["message_id"]}}
                events.insert(events.index(row), acknowledgement)
                row = acknowledgement
            row["runtime"]["native_session_id"] = None
            self.assertFalse(runner.verified_acceptance(0, self.output, events), kind)

    def test_null_initial_ready_after_native_association_is_rejected(self):
        self.events.remove(self.ready_rows[0])
        self.events.append(self.ready_rows[0])
        self.assertFalse(self.verify())


class FixedVersionCoordinatorTests(unittest.TestCase):
    def test_persisted_parent_and_child_versions_match_selected_input(self):
        for version in ("2.1.273", "2.1.278"):
            events = fixture(version)
            self.assertTrue(runner.verified_acceptance(0, OUTPUT, events, version))
            self.assertTrue(runner.verified_acceptance(0, OUTPUT,
                runner.project_public_events(events, lambda value: value), version))
            other = "2.1.278" if version == "2.1.273" else "2.1.273"
            self.assertFalse(runner.verified_acceptance(0, OUTPUT, events, other))
            self.assertFalse(runner.verified_acceptance(0, OUTPUT, events, "latest"))
            for key in ("parent", "child", "parent_generations", "child_generations"):
                changed = copy.deepcopy(events)
                chain = next(row for row in changed if row["event"] == "saved_chain_verified")
                saved = chain[key][0] if isinstance(chain[key], list) else chain[key]
                config = json.loads(saved["config_json"])
                config["cli_version"] = other
                saved["config_json"] = json.dumps(config)
                self.assertFalse(runner.verified_acceptance(0, OUTPUT, changed, version), key)

    def test_unverified_input_stops_before_api_read_or_process(self):
        with tempfile.TemporaryDirectory() as temporary:
            executable = Path(temporary) / "claude"
            executable.write_bytes(b"not an official binary")
            with patch.object(runner.base, "load_api_environment") as load_api, \
                    patch.object(runner, "verify_version") as probe, \
                    patch.object(runner.subprocess, "Popen") as spawn:
                for version in ("2.1.273", "2.1.278", "2.1.280", "latest", "2.1.279"):
                    with self.subTest(version=version), self.assertRaises(ValueError):
                        runner.run(argparse.Namespace(claude=executable, claude_version=version))
                load_api.assert_not_called()
                probe.assert_not_called()
                spawn.assert_not_called()

    def test_candidate_requires_version_flag_and_authorized_account_before_probe(self):
        with patch.object(runner, "verify_binary") as verify, \
                patch.object(runner.base, "load_api_environment") as load_api, \
                patch.object(runner.subprocess, "Popen") as spawn:
            for version, enabled, account in (
                ("2.1.280", False, True), ("2.1.278", True, True),
                ("2.1.280", True, False),
            ):
                with self.subTest(version=version, enabled=enabled, account=account):
                    args = argparse.Namespace(claude_version=version,
                        allow_claude_21280_coordinator_candidate=enabled,
                        use_authorized_default_account=account)
                    with self.assertRaises(ValueError):
                        runner.run(args)
            with self.assertRaises(ValueError):
                runner.run(argparse.Namespace(
                    claude_version="2.1.280", allow_claude_21280_coordinator_candidate=True,
                    use_authorized_default_account=True, config_dir=Path("/private/config")))
            verify.assert_not_called()
            load_api.assert_not_called()
            spawn.assert_not_called()

    def test_candidate_supervisor_probe_rejects_missing_build_marker(self):
        path = Path("/private/tmp/synthetic-supervisor")
        for code, output, error in ((1, "", ""), (0, "wrong\n", ""),
                                    (0, runner.TEST_CANDIDATE_BUILD_MARKER + "\n", "extra")):
            with self.subTest(code=code, output=output, error=error), \
                    patch.object(runner.sys, "platform", "linux"), \
                    patch.object(runner.subprocess, "run", return_value=SimpleNamespace(
                        returncode=code, stdout=output, stderr=error)):
                with self.assertRaises(ValueError):
                    runner.verify_candidate_supervisor(path)

    def test_candidate_280_keeps_parent_child_audit_and_build_marker(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for name in ("claude-fixture", "libtest", "supervisor"):
                (root / name).write_bytes(b"synthetic executable")
            args = argparse.Namespace(
                claude=root / "claude-fixture", claude_version="2.1.280",
                allow_claude_21280_coordinator_candidate=True,
                test_binary=root / "libtest", supervisor=root / "supervisor",
                api_environment_file=None, use_authorized_default_account=True,
                output=root / "events.ndjson", model="claude-sonnet-4-6",
                config_dir=None, auth_home=None,
            )
            status = {"loggedIn": True, "authMethod": "claude.ai", "apiProvider": "firstParty",
                      "subscriptionType": "pro"}

            def command(command, **kwargs):
                if command[0] == "/usr/bin/codesign":
                    return SimpleNamespace(returncode=0, stdout=b"", stderr=b"")
                if command == [str(args.supervisor), runner.TEST_CANDIDATE_BUILD_ARGUMENT]:
                    return SimpleNamespace(returncode=0,
                        stdout=runner.TEST_CANDIDATE_BUILD_MARKER + "\n", stderr="")
                return SimpleNamespace(stdout="")

            def spawn(command, **kwargs):
                environment = kwargs["env"]
                self.assertEqual(environment["INFINISHELL_CLAUDE_COORDINATOR_CANDIDATE_21280"], "1")
                self.assertEqual(environment["INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION"], "2.1.280")
                marker = Path(environment["INFINISHELL_CLAUDE_LIVE_ROOT"]) / runner.base.TEST_CANDIDATE_MARKER
                self.assertEqual(marker.read_text(), runner.base.TEST_CANDIDATE_MARKER_CONTENT)
                self.assertRegex(environment["WARP_DATA_PROFILE"],
                                 r"^claude-coordinator-[0-9a-f]{32}$")
                Path(environment["INFINISHELL_CLAUDE_LIVE_ARTIFACT"]).write_text(
                    "".join(json.dumps(row) + "\n" for row in fixture("2.1.280")),
                    encoding="utf-8")
                return SimpleNamespace(returncode=0, communicate=lambda timeout: (OUTPUT, None))

            with patch.object(runner, "current_platform", return_value="darwin-arm64"), \
                    patch.object(runner, "verify_binary", return_value={"sha256": "synthetic"}) as verify, \
                    patch.object(runner, "verify_version", return_value="2.1.280 (Claude Code)"), \
                    patch.object(runner.tempfile, "mkdtemp", return_value=str(root)), \
                    patch.object(runner.base, "authorized_default_account_environment",
                                 return_value={"HOME": str(Path.home()), "PATH": "/safe/bin"}), \
                    patch.object(runner.base, "probe_authorized_default_account", return_value=status), \
                    patch.object(runner.subprocess, "run", side_effect=command), \
                    patch.object(runner.subprocess, "Popen", side_effect=spawn), \
                    patch("builtins.print"):
                self.assertEqual(runner.run(args), 0)
            self.assertEqual(verify.call_count, 3)
            self.assertTrue(all(call.args[-1] == "2.1.280" for call in verify.call_args_list))
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            self.assertTrue(metadata["test_only_coordinator_candidate_21280"])
            self.assertTrue(metadata["candidate_supervisor_marker_verified"])
            self.assertTrue(metadata["candidate_supervisor_binary_unchanged"])
            self.assertTrue(metadata["acceptance_passed"])

    def test_probe_failure_stops_before_api_read(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(runner, "verify_binary", return_value={"sha256": "synthetic"}), \
                patch.object(runner.tempfile, "mkdtemp", return_value=temporary), \
                patch.object(runner, "verify_version", side_effect=ValueError("版本不匹配")), \
                patch.object(runner.base, "load_api_environment") as load_api, \
                patch.object(runner.subprocess, "Popen") as spawn:
            with self.assertRaises(ValueError):
                runner.run(argparse.Namespace(claude=Path(temporary) / "claude", claude_version="2.1.278"))
            load_api.assert_not_called()
            spawn.assert_not_called()

    def test_direct_default_account_run_rejects_api_before_reading_it(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(runner, "verify_binary", return_value={"sha256": "synthetic"}), \
                patch.object(runner.tempfile, "mkdtemp", return_value=temporary), \
                patch.object(runner, "verify_version", return_value="2.1.278 (Claude Code)"), \
                patch.object(runner.base, "load_api_environment") as load_api, \
                patch.object(runner.base, "probe_authorized_default_account") as probe:
            root = Path(temporary)
            args = argparse.Namespace(
                claude=root / "claude", claude_version="2.1.278",
                use_authorized_default_account=True, api_environment_file=root / "api.json",
            )
            with self.assertRaisesRegex(ValueError, "不能同时提供"):
                runner.run(args)
            load_api.assert_not_called()
            probe.assert_not_called()

    def test_default_and_explicit_version_keep_platform_through_final_binary_verification(self):
        for selected in (None, "2.1.278"):
            expected = selected or "2.1.273"
            with self.subTest(version=expected), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                for name in ("claude-fixture", "libtest", "supervisor"):
                    (root / name).write_bytes(b"synthetic executable")
                api = root / "api.json"
                api.write_text('{"ANTHROPIC_API_KEY":"synthetic-test-only"}', encoding="utf-8")
                args = argparse.Namespace(claude=root / "claude-fixture", test_binary=root / "libtest",
                    supervisor=root / "supervisor", api_environment_file=api, output=root / "events.ndjson",
                    model="claude-sonnet-4-6")
                if selected is not None:
                    args.claude_version = selected

                def verify_native(path, platform, version):
                    self.assertEqual(path, args.claude)
                    self.assertEqual(platform, "darwin-arm64")
                    self.assertEqual(version, expected)
                    return {"sha256": "synthetic"}

                def spawn(command, **kwargs):
                    env = kwargs["env"]
                    self.assertEqual(env["INFINISHELL_CLAUDE_LIVE_EXPECTED_VERSION"], expected)
                    self.assertEqual(command[0], str(args.test_binary))
                    Path(env["INFINISHELL_CLAUDE_LIVE_ARTIFACT"]).write_text(
                        "".join(json.dumps(row) + "\n" for row in fixture(expected)), encoding="utf-8")
                    return SimpleNamespace(returncode=0, communicate=lambda timeout: (OUTPUT, None))

                with patch.object(runner, "current_platform", return_value="darwin-arm64"), \
                        patch.object(runner, "verify_binary", side_effect=verify_native) as verify, \
                        patch.object(runner, "verify_version", return_value=f"{expected} (Claude Code)") as probe, \
                        patch.object(runner.tempfile, "mkdtemp", return_value=str(root)), \
                        patch.object(runner.subprocess, "run", return_value=SimpleNamespace(stdout="")), \
                        patch.object(runner.subprocess, "Popen", side_effect=spawn), \
                        patch("builtins.print"):
                    self.assertEqual(runner.run(args), 0)
                probe.assert_called_once_with(args.claude, root, expected)
                self.assertEqual(verify.call_count, 3)
                self.assertTrue(all(call.args == (args.claude, "darwin-arm64", expected)
                                    for call in verify.call_args_list))
                metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
                self.assertEqual(metadata["requested_cli_version"], expected)
                self.assertEqual(metadata["cli_version"], f"{expected} (Claude Code)")
                self.assertTrue(metadata["acceptance_passed"])
                self.assertTrue(metadata["cli_binary_unchanged"])

    def test_authorized_default_account_runs_coordinator_without_config_override(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for name in ("claude-fixture", "libtest", "supervisor"):
                (root / name).write_bytes(b"synthetic executable")
            args = argparse.Namespace(
                claude=root / "claude-fixture", claude_version="2.1.278",
                test_binary=root / "libtest", supervisor=root / "supervisor",
                api_environment_file=None, use_authorized_default_account=True,
                output=root / "events.ndjson", model="claude-sonnet-4-6",
                config_dir=None, auth_home=None,
            )
            status = {"loggedIn": True, "authMethod": "claude.ai", "apiProvider": "firstParty",
                      "subscriptionType": "pro"}

            def spawn(command, **kwargs):
                environment = kwargs["env"]
                self.assertNotIn("CLAUDE_CONFIG_DIR", environment)
                self.assertNotIn("INFINISHELL_CLAUDE_LIVE_CONFIG_DIR", environment)
                self.assertEqual(environment["INFINISHELL_CLAUDE_LIVE_AUTH_MODE"],
                                 "authorized_default_account")
                self.assertRegex(environment["WARP_DATA_PROFILE"],
                                 r"^claude-coordinator-[0-9a-f]{32}$")
                Path(environment["INFINISHELL_CLAUDE_LIVE_ARTIFACT"]).write_text(
                    "".join(json.dumps(row) + "\n" for row in fixture("2.1.278")),
                    encoding="utf-8")
                return SimpleNamespace(returncode=0, communicate=lambda timeout: (OUTPUT, None))

            account_environment = {"HOME": str(Path.home()), "PATH": "/safe/bin"}
            with patch.object(runner, "current_platform", return_value="darwin-arm64"), \
                    patch.object(runner, "verify_binary", return_value={"sha256": "synthetic"}), \
                    patch.object(runner, "verify_version", return_value="2.1.278 (Claude Code)"), \
                    patch.object(runner.tempfile, "mkdtemp", return_value=str(root)), \
                    patch.object(runner.base, "authorized_default_account_environment",
                                 return_value=account_environment.copy()), \
                    patch.object(runner.base, "probe_authorized_default_account",
                                 return_value=status) as probe, \
                    patch.object(runner.subprocess, "run", return_value=SimpleNamespace(stdout="")), \
                    patch.object(runner.subprocess, "Popen", side_effect=spawn), \
                    patch("builtins.print"):
                self.assertEqual(runner.run(args), 0)
            probe.assert_called_once_with(args.claude, account_environment, root / "project")
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            self.assertEqual(metadata["authentication_source"], "authorized_default_account")
            self.assertEqual(metadata["authorized_default_account"], status)
            self.assertTrue(metadata["sensitive_value_scan_passed"])
            self.assertTrue(metadata["acceptance_passed"])


if __name__ == "__main__":
    unittest.main()
