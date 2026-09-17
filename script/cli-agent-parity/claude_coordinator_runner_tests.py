#!/usr/bin/env python3
"""离线校验父子协调器验收谓词与私有环境边界；不启动 CLI，不请求模型。"""

import argparse
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run_claude_coordinator_live as runner


OUTPUT = "test result: ok. 1 passed; 0 failed; 0 ignored; 99 filtered out;\n" + "\n".join(
    "CLAUDE_NATIVE_PROTOCOL_IDS " + json.dumps({"type": "result", "subtype": "success",
        "uuid": f"result-{task_id}-{generation}", "session_id": native_id,
        "user_message_uuid": f"{task_id}-{generation}", "user_message_uuids": [f"{task_id}-{generation}"]})
    for task_id, native_id in (("00000000-0000-4000-8000-000000000001", "parent-native-1"),
                               ("00000000-0000-4000-8000-000000000002", "child-native-1"))
    for generation in (1, 2))
PARENT = "00000000-0000-4000-8000-000000000001"
CHILD = "00000000-0000-4000-8000-000000000002"


def result_id(generation):
    return str(runner.uuid.UUID(bytes=runner.hashlib.sha256(
        f"infinishell-result:{CHILD}:{generation}".encode()).digest()[:16]))


def result_body(task):
    return json.dumps({"task_id": CHILD, "generation": task["generation"], "state": "completed",
        "result": task["result"], "result_bytes": len(task["result"].encode()), "truncated": False,
        "terminal_evidence": task["terminal_evidence"], "evidence_truncated": False})


def fixture():
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
    parent_config = {"model": "claude-sonnet-4-6", "permission_policy": "ClaudeRestrictedFilesV1",
                     "claude_profile": profile, "runtime_generation": "parent-process-1",
                     "effective_permissions": {"claudeRestrictedFilesV1": profile,
                                               "fixedProfileVerified": True, "permissionMode": "plan"}}
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
        proof = json.loads(saved["terminal_evidence"])
        process = json.loads(saved["config_json"])["runtime_generation"]
        for kind in ({"TurnStarted": {"turn_id": proof["event"]["TurnFinished"]["turn_id"]}}, proof["event"]):
            runtime.append({"event": "runtime", "task": saved,
                            "runtime": {"native_session_id": saved["native_session_id"], "generation": process, "kind": kind}})
    cleanup = [{"event": "cleanup_confirmed", "task_id": task_id,
                "receipt": {"cleanup_confirmed": True, "generation": process, "containment": "macos_coalition",
                            "manifest_sha256": "b" * 64}}
               for task_id, process in ((PARENT, "parent-process-1"), (CHILD, "child-process-1"))]
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
    return events


def native_output(events):
    chain = next(row for row in events if row["event"] == "saved_chain_verified")
    records = []
    for row in events:
        if row.get("event") != "runtime" or "TurnStarted" not in row["runtime"]["kind"]:
            continue
        turn_id = row["runtime"]["kind"]["TurnStarted"]["turn_id"]
        task_id, generation = row["task"]["task_id"], row["task"]["generation"]
        task = next(task for task in chain["parent_generations" if task_id == PARENT else "child_generations"]
                    if task["generation"] == generation)
        joined = json.loads(task["config_json"]).get("claude_joined_inputs", [])
        ids = [turn_id, *[record["message_id"] for record in joined if record["turn_id"] == turn_id]]
        records.append("CLAUDE_NATIVE_PROTOCOL_IDS " + json.dumps({"type": "result", "subtype": "success",
            "uuid": "native-result-" + turn_id, "session_id": task["native_session_id"],
            "user_message_uuid": ids[-1], "user_message_uuids": ids}))
    return "test result: ok. 1 passed; 0 failed; 0 ignored; 99 filtered out;\n" + "\n".join(records)


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
        row["receipt"]["generation"] = "old-process"
        self.assertFalse(self.verify())

    def test_not_started_cleanup_cannot_prove_native_process_cleanup(self):
        row = next(row for row in self.events if row["event"] == "cleanup_confirmed")
        row["receipt"]["containment"] = "not_started"
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
        rows = runner._native_results(self.output)
        self.output = self.output.replace(json.dumps([f"{CHILD}-1", f"{CHILD}-2"]), json.dumps([f"{CHILD}-2"]))
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
        self.output = "test result: ok. 1 passed; 0 failed; 0 ignored;"
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
                                      "permissionMode": "plan", "unapprovedSettings": "private-extra"}}}}}
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


if __name__ == "__main__":
    unittest.main()
