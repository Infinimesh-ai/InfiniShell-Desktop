#!/usr/bin/env python3
"""离线攻击 V2 生产链收据谓词；不启动 CLI，不作为在线验收正例。"""

import copy
import json
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import claude_coordinator_runner_tests as legacy
import run_claude_coordinator_live as runner


VERSION = "2.1.280"


def _convert_profile(value):
    if isinstance(value, list):
        return [_convert_profile(item) for item in value]
    if isinstance(value, str):
        if value.lstrip().startswith(("{", "[")):
            return json.dumps(_convert_profile(json.loads(value)))
        return "ClaudeRestrictedFilesV2" if value == "ClaudeRestrictedFilesV1" else value
    if not isinstance(value, dict):
        return value
    if value.get("version") == 1 and "workingDirectory" in value:
        return {"version": 2, "cliVersion": VERSION, "base": copy.deepcopy(value)}
    return {"claudeRestrictedFilesV2" if key == "claudeRestrictedFilesV1" else key:
            _convert_profile(child) for key, child in value.items()}


def fixture(file_policy):
    events = legacy.fixture(VERSION)
    if file_policy == "v2":
        events = _convert_profile(events)
    for event in events:
        if event["event"] in ("acceptance_started", "acceptance_passed"):
            event["scope"] = runner.FILE_POLICY_SCOPES[file_policy]
    runner._one(events, "acceptance_started")["production_version_gate_verified"] = True
    runner._one(events, "acceptance_passed").update({
        "waiting_write_cancel_verified": False, "live_host_reattach_verified": False,
        "cold_history_resume_verified": False})
    chain = runner._one(events, "saved_chain_verified")
    child = chain["child_generations"][0]
    config = json.loads(child["config_json"])
    if file_policy == "v1-ceiling":
        parent = chain["parent_generations"][0]
        events.insert(-1, {"event": "v1_parent_v2_rejected", "parent_task_id": parent["task_id"],
            "parent_generation": 1, "parent_native_session_id": parent["native_session_id"],
            "ceiling": config["permission_ceiling"], "requested_profile": {
                "version": 2, "cliVersion": VERSION, "base": config["claude_profile"]},
            "error": {"reason": "claude_profile_parent_mismatch"}, "native_connection_created": False,
            "native_input_sent": False, "parent_record_unchanged": True})
        return events
    gate = runner._one(events, "native_ack_before_child_edit_allow")
    gate.update({"event": "native_ack_before_child_write_allow", "child_edit_waiting": False,
                 "child_write_waiting": True})
    runner._one(events, "coordinator_chain_finished").update({
        "approvals_denied": 1, "child_edit_effect_verified": False,
        "child_write_effect_verified": True, "denied_write_unchanged": True})
    turn_id = json.loads(child["terminal_evidence"])["event"]["TurnFinished"]["turn_id"]
    allowed = next(row for row in events if row.get("event") == "approval_allowed"
                   and row["approval"]["details"]["tool_name"] == "Edit")
    events.remove(allowed)
    allowed["approval"] = {"approval_id": "allow-write", "turn_id": turn_id, "method": "can_use_tool",
                           "details": {"tool_name": "Write", "input": {
                               "file_path": "/probe/project/child-approved.txt", "content": chain["markers"]["initial"]}}}
    denied = copy.deepcopy(allowed)
    denied.update({"event": "approval_denied", "decision": "DenyOnce"})
    denied["approval"]["approval_id"] = "deny-write"
    denied["approval"]["details"]["input"] = {
        "file_path": "/probe/project/child-denied.txt", "content": runner.DENIED_WRITE}
    native = {"native_session_id": child["native_session_id"], "generation": config["runtime_generation"]}
    additions = []
    for chosen in (denied, allowed):
        request = chosen["approval"]
        additions.extend([
            {"event": "runtime", "task": child, "runtime": {
                **native, "kind": {"ApprovalRequested": copy.deepcopy(request)}}},
            chosen,
            {"event": "runtime", "task": child, "runtime": {**native, "kind": {"ApprovalResolved": {
                "approval_id": request["approval_id"], "decision": chosen["decision"]}}}},
        ])
    index = next(index for index, row in enumerate(events) if row.get("event") == "runtime"
                 and row["task"]["task_id"] == child["task_id"] and row["task"]["generation"] == 1
                 and "TurnFinished" in row["runtime"]["kind"])
    events[index:index] = additions
    events.insert(1, {"event": "write_fixtures_verified", "allowed_path": "/probe/project/child-approved.txt",
                     "allowed_absent": True, "denied_path": "/probe/project/child-denied.txt", "denied_before": "CHILD_BEFORE"})
    events.insert(-1, {"event": "write_effects_verified", "allowed_path": "/probe/project/child-approved.txt",
                      "allowed_content": chain["markers"]["initial"], "denied_path": "/probe/project/child-denied.txt",
                      "denied_content": "CHILD_BEFORE"})
    return events


class V2EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.events = fixture("v2")

    def verify(self):
        return runner.verified_acceptance(0, legacy.OUTPUT, self.events, VERSION, "v2")

    def test_private_and_public_evidence_are_both_required_and_valid(self):
        self.assertTrue(self.verify())
        projected = runner.project_public_events(self.events, lambda value: value)
        self.assertTrue(runner.verified_acceptance(0, legacy.OUTPUT, projected, VERSION, "v2"))

    def test_original_v1_predicate_cannot_accept_v2(self):
        self.assertFalse(runner.verified_acceptance(0, legacy.OUTPUT, self.events, VERSION))

    def test_initial_ready_binds_exact_v2_profile_before_native_session(self):
        chain = runner._one(self.events, "saved_chain_verified")
        first = copy.deepcopy(chain["child_generations"][0])
        config = json.loads(first["config_json"])
        first["native_session_id"] = None
        ready = {"event": "runtime", "task": first, "runtime": {
            "native_session_id": None, "generation": config["runtime_generation"], "kind": {"SessionReady": {
                "effective_permissions": {**config["effective_permissions"], "sessionAssociationConfirmed": False}}}}}
        self.events.insert(1, ready)
        self.assertTrue(self.verify())
        permissions = ready["runtime"]["kind"]["SessionReady"]["effective_permissions"]
        permissions["claudeRestrictedFilesV1"] = permissions.pop("claudeRestrictedFilesV2")["base"]
        self.assertFalse(self.verify())

    def test_other_version_cannot_accept_v2(self):
        self.assertFalse(runner.verified_acceptance(0, legacy.OUTPUT, self.events, "2.1.278", "v2"))

    def test_missing_denial_does_not_pass(self):
        self.events.remove(runner._one(self.events, "approval_denied"))
        self.assertFalse(self.verify())

    def test_changed_denied_file_does_not_pass(self):
        runner._one(self.events, "write_effects_verified")["denied_content"] = runner.DENIED_WRITE
        self.assertFalse(self.verify())

    def test_existing_allowed_file_does_not_establish_write_creation(self):
        runner._one(self.events, "write_fixtures_verified")["allowed_absent"] = False
        self.assertFalse(self.verify())

    def test_allowing_another_input_does_not_pass(self):
        chosen = next(row for row in self.events if row.get("event") == "approval_allowed"
                      and row["approval"]["details"]["tool_name"] == "Write")
        chosen["approval"]["details"]["input"]["content"] += "\n"
        self.assertFalse(self.verify())

    def test_denied_request_cannot_target_another_path(self):
        runner._one(self.events, "approval_denied")["approval"]["details"]["input"]["file_path"] = "/outside/file"
        self.assertFalse(self.verify())

    def test_missing_native_resolution_does_not_pass(self):
        self.events = [row for row in self.events if row.get("runtime", {}).get("kind", {}).get(
            "ApprovalResolved", {}).get("decision") != "DenyOnce"]
        self.assertFalse(self.verify())

    def test_approval_native_session_or_generation_mismatch_does_not_pass(self):
        for field, replacement in (("native_session_id", "other"), ("generation", "old-process")):
            with self.subTest(field=field):
                self.events = fixture("v2")
                row = next(row for row in self.events if "ApprovalResolved" in row.get("runtime", {}).get("kind", {}))
                row["runtime"][field] = replacement
                self.assertFalse(self.verify())

    def test_resolution_cannot_change_host_decision(self):
        row = next(row for row in self.events if "ApprovalResolved" in row.get("runtime", {}).get("kind", {}))
        row["runtime"]["kind"]["ApprovalResolved"]["decision"] = "AllowOnce"
        self.assertFalse(self.verify())

    def test_duplicate_native_approval_does_not_pass(self):
        row = next(row for row in self.events if "ApprovalRequested" in row.get("runtime", {}).get("kind", {}))
        self.events.insert(self.events.index(row), copy.deepcopy(row))
        self.assertFalse(self.verify())

    def test_resolution_before_host_decision_does_not_pass(self):
        row = next(row for row in self.events if "ApprovalResolved" in row.get("runtime", {}).get("kind", {}))
        self.events.remove(row)
        self.events.insert(0, row)
        self.assertFalse(self.verify())

    def test_process_cleanup_remains_required(self):
        next(row for row in self.events if row["event"] == "cleanup_confirmed")["receipt"]["native_process"] = "running"
        self.assertFalse(self.verify())

    def test_unperformed_cancel_or_recovery_cannot_be_claimed(self):
        for key in ("waiting_write_cancel_verified", "live_host_reattach_verified", "cold_history_resume_verified"):
            with self.subTest(key=key):
                self.events = fixture("v2")
                runner._one(self.events, "acceptance_passed")[key] = True
                self.assertFalse(self.verify())

    def test_parent_profile_cannot_be_downgraded_or_replaced(self):
        chain = runner._one(self.events, "saved_chain_verified")
        config = json.loads(chain["child"]["config_json"])
        config["permission_ceiling"]["permissions"]["claudeRestrictedFilesV2"]["base"]["localTools"]["allow_spawn"] = False
        chain["child"]["config_json"] = json.dumps(config)
        self.assertFalse(self.verify())


class V1CeilingEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.events = fixture("v1-ceiling")

    def verify(self):
        return runner.verified_acceptance(0, legacy.OUTPUT, self.events, VERSION, "v1-ceiling")

    def test_actual_v1_chain_and_synchronous_v2_rejection_are_required(self):
        self.assertTrue(self.verify())
        projected = runner.project_public_events(self.events, lambda value: value)
        self.assertTrue(runner.verified_acceptance(0, legacy.OUTPUT, projected, VERSION, "v1-ceiling"))

    def test_missing_rejection_does_not_pass(self):
        self.events.remove(runner._one(self.events, "v1_parent_v2_rejected"))
        self.assertFalse(self.verify())

    def test_wrong_parent_identity_reason_or_native_launch_does_not_pass(self):
        for field, value in (("parent_task_id", "old-parent"), ("parent_generation", 2),
                             ("parent_native_session_id", "old-session"), ("parent_record_unchanged", False),
                             ("native_connection_created", True), ("native_input_sent", True),
                             ("error", {"reason": "unrelated_error"})):
            with self.subTest(field=field):
                self.events = fixture("v1-ceiling")
                runner._one(self.events, "v1_parent_v2_rejected")[field] = value
                self.assertFalse(self.verify())

    def test_v1_requested_instead_of_v2_does_not_pass(self):
        row = runner._one(self.events, "v1_parent_v2_rejected")
        row["requested_profile"] = row["requested_profile"]["base"]
        self.assertFalse(self.verify())


class V2AuthSelectionTests(unittest.TestCase):
    def test_only_formal_fixed_account_path_is_accepted(self):
        for policy in ("v2", "v1-ceiling"):
            args = SimpleNamespace(file_policy=policy, claude_version=VERSION,
                                   use_authorized_default_account=True)
            with patch.object(runner.sys, "platform", "darwin"):
                self.assertTrue(runner.validate_auth_selection(args))
                for field, value in (("claude_version", "2.1.278"), ("use_authorized_default_account", False),
                                     ("allow_claude_21280_coordinator_candidate", True), ("api_environment_file", "/private/api")):
                    changed = copy.copy(args)
                    setattr(changed, field, value)
                    with self.subTest(policy=policy, field=field), self.assertRaises(ValueError):
                        runner.validate_auth_selection(changed)
            with patch.object(runner.sys, "platform", "linux"), self.assertRaises(ValueError):
                runner.validate_auth_selection(args)


class TruncatedInspectionProjectionTests(unittest.TestCase):
    def test_declared_truncated_json_body_is_hashed_without_public_partial_tree(self):
        original = {"body": '{"config_json":"unfinished', "body_truncated": True}
        projected = runner.project_public_value(original)
        self.assertEqual(original["body"], '{"config_json":"unfinished')
        self.assertTrue(projected["body_truncated"])
        self.assertNotIn("config_json", projected["body"])
        self.assertEqual(projected["private_body_sha256"],
                         runner.hashlib.sha256(original["body"].encode()).hexdigest())

    def test_unmarked_or_non_boolean_truncation_cannot_hide_bad_json(self):
        for flag in (None, False, "true", 1):
            with self.subTest(flag=flag), self.assertRaises(ValueError):
                runner.project_public_value({"body": '{"broken":', "body_truncated": flag})

    def test_truncated_inspection_copy_does_not_replace_full_saved_proof(self):
        events = fixture("v2")
        messages = runner._one(events, "saved_chain_verified")["messages"]
        inspection = next(message for message in messages if message["subject"] == "native_tool_result"
                          and "tasks" in json.loads(message["body"])["Ok"])
        body = json.loads(inspection["body"])
        body["Ok"]["tasks"][0]["messages"] = [{
            "body": '{"Submit":{"input":["truncated', "body_truncated": True}]
        inspection["body"] = json.dumps(body)
        self.assertTrue(runner.verified_acceptance(0, legacy.OUTPUT, events, VERSION, "v2"))
        projected = runner.project_public_events(events, lambda value: value)
        self.assertTrue(runner.verified_acceptance(0, legacy.OUTPUT, projected, VERSION, "v2"))
        runner._one(events, "saved_chain_verified")["child_generations"][0]["terminal_evidence"] = '{"broken":'
        self.assertFalse(runner.verified_acceptance(0, legacy.OUTPUT, events, VERSION, "v2"))


if __name__ == "__main__":
    unittest.main()
