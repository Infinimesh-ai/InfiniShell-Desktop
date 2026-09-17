"""等待 Edit 审批取消的离线证据回归；不读取真实认证或启动 CLI。"""

import copy
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import uuid

import run_claude_approval_cancel_live as runner


def identity(number):
    return str(uuid.UUID(int=number))


def complete_events():
    running, continued, interrupt, native, generation, approval, cancelled_result, continue_result = [
        identity(number) for number in range(1, 9)]
    tool_id = "toolu_approval_cancel_fixture"
    request_id = f"infinishell-{generation}-7"
    marker = "APPROVAL_CANCEL_CONTINUED_" + "f" * 32
    empty = hashlib.sha256(b"").hexdigest()
    def lifecycle(input_id, state):
        return {"direction": "stdout", "type": "command_lifecycle", "session_id": native,
                "command_uuid": input_id, "state": state}
    rows = [
        {"direction": "stdin", "type": "user", "uuid": running, "session_id": "sha256:" + empty},
        lifecycle(running, "queued"), lifecycle(running, "started"),
        {"direction": "stdout", "type": "assistant", "session_id": native,
         "tools": [{"id": tool_id, "name": "Edit"}]},
        {"direction": "stdout", "type": "control_request", "request_subtype": "can_use_tool",
         "request_id": approval, "tool_use_id": tool_id},
        {"direction": "stdin", "type": "control_request", "request_subtype": "interrupt", "request_id": request_id},
        {"direction": "stdout", "type": "control_cancel_request", "request_id": approval},
        {"direction": "stdout", "type": "control_response", "response_request_id": request_id,
         "response_subtype": "success"},
        {"direction": "stdout", "type": "result", "uuid": cancelled_result, "session_id": native,
         "subtype": "error_during_execution", "is_error": True, "terminal_reason": "aborted_streaming",
         "user_message_uuid": running, "user_message_uuids": [running]},
        lifecycle(running, "cancelled"),
        {"direction": "stdin", "type": "user", "uuid": continued, "session_id": native},
        lifecycle(continued, "queued"), lifecycle(continued, "started"),
        {"direction": "stdout", "type": "result", "uuid": continue_result, "session_id": native,
         "subtype": "success", "is_error": False, "terminal_reason": "completed",
         "user_message_uuid": continued, "user_message_uuids": [continued]},
    ]
    def file_event(phase):
        return {"event": "file_unchanged", "phase": phase, "file_ref": runner.FILE_REF,
                "full_bytes_verified": True, "file_bytes": len(runner.BEFORE),
                "file_sha256": hashlib.sha256(runner.BEFORE).hexdigest()}
    events = [
        {"event": "acceptance_started", "scope": runner.SCOPE, "max_native_inputs": 2,
         "phase_deadline_seconds": 180, "permission_policy": "ClaudeRestrictedFilesV1",
         "credential_files_read_by_probe": False, "real_gui_verified": False, "persistence_verified": False},
        file_event("before"),
        {"event": "profile_verified", "permission_mode": "plan", "fixed_profile_verified": True,
         "profile_sha256": "a" * 64, "filesystem_sandbox_verified": False, "parent_permission_ceiling_verified": False},
        {"event": "input_submitted", "phase": "pending_edit", "message_id": running, "submitted_input_count": 1},
        {"event": "message_accepted", "message_id": running, "turn_id": running,
         "native_session_id": native, "native_receipt": True},
        {"event": "turn_started", "turn_id": running, "native_session_id": native},
        {"event": "edit_approval_requested", "approval_id": approval, "turn_id": running,
         "native_session_id": native, "tool_use_id": tool_id, "exact_edit_verified": True, "edit_allowed": False,
         "file_ref": runner.FILE_REF, "old_string_sha256": hashlib.sha256(runner.BEFORE).hexdigest(),
         "new_string_sha256": hashlib.sha256(runner.AFTER).hexdigest(),
         "input_keys": ["file_path", "new_string", "old_string", "replace_all"], "replace_all": False},
        {"event": "interrupt_submitted", "message_id": interrupt, "execution_turn_id": running,
         "approval_id": approval, "approval_still_pending": True, "edit_allowed": False},
        {"event": "edit_approval_cancelled", "approval_id": approval, "native_session_id": native,
         "native_execution_cancelled_verified": False},
        {"event": "interrupt_accepted", "message_id": interrupt, "turn_id": running,
         "native_session_id": native, "native_receipt": True},
        {"event": "turn_finished", "phase": "pending_edit", "turn_id": running, "native_session_id": native,
         "outcome": "Cancelled", "output_bytes": 0, "full_output_sha256": empty},
        file_event("after_cancel"),
        {"event": "input_submitted", "phase": "continue", "message_id": continued, "submitted_input_count": 2},
        {"event": "message_accepted", "message_id": continued, "turn_id": continued,
         "native_session_id": native, "native_receipt": True},
        {"event": "turn_started", "turn_id": continued, "native_session_id": native},
        {"event": "turn_finished", "phase": "continue", "turn_id": continued, "native_session_id": native,
         "outcome": "Completed", "output": marker, "output_bytes": len(marker),
         "full_output_sha256": hashlib.sha256(marker.encode()).hexdigest(),
         "trimmed_output_sha256": hashlib.sha256(marker.encode()).hexdigest(), "same_native_session_verified": True},
        file_event("after_continue"),
        {"event": "connection_shutdown", "native_session_id": native},
        {"event": "cleanup_checked", "generation": generation, "cleanup_confirmed": True, "normal_exit": True,
         "transport_closed": True, "cleanup_receipt_read": True,
         "receipt": {"version": 1, "generation": generation, "cleanup_confirmed": True,
                     "containment": "macos_resource_coalition", "exit_code": 0, "exit_reason": "stdio_closed"}},
    ]
    events.extend({"event": "native_protocol_ids", "generation": generation, "sequence": index, "native": row}
                  for index, row in enumerate(rows, 1))
    events.extend([file_event("after_shutdown"),
        {"event": "native_pending_edit_cancel_verified", "native_session_id": native, "runtime_generation": generation,
         "execution_input_id": running, "continuation_input_id": continued, "approval_id": approval, "tool_use_id": tool_id,
         "interrupt_request_id": request_id, "native_result_uuid": cancelled_result, "continue_result_uuid": continue_result,
         "terminal_reason": "aborted_streaming", "is_error": True, "user_message_uuids": [running],
         "edit_allowed": False, "read_call_count": 0, "all_four_evidence_verified": True},
        {"event": "acceptance_passed", "scope": runner.SCOPE, "native_session_id": native, "native_inputs": 2,
         "edit_allowed": False, "all_four_evidence_verified": True, "full_file_unchanged_verified": True,
         "same_native_session_verified": True, "normal_exit": True, "cleanup_confirmed": True, "transport_closed": True,
         "real_gui_verified": False, "persistence_verified": False, "http_request_count_verified": False},
    ])
    return events


def event_of(events, kind, **fields):
    return next(event for event in events if event["event"] == kind
                and all(event.get(key) == value for key, value in fields.items()))


def native_of(events, **fields):
    return next(event["native"] for event in events if event["event"] == "native_protocol_ids"
                and all(event["native"].get(key) == value for key, value in fields.items()))


def resequence(events):
    number = 0
    for event in events:
        if event["event"] == "native_protocol_ids":
            number += 1
            event["sequence"] = number


class ApprovalCancelAuditTests(unittest.TestCase):
    output = "test result: ok. 1 passed; 0 failed; 0 ignored; 7400 filtered out"

    def assert_rejected(self, events):
        self.assertFalse(runner.verified_acceptance(0, self.output, events))

    def test_complete_native_evidence_is_accepted_without_changing_production_rules(self):
        events = complete_events()
        self.assertTrue(runner.verified_acceptance(0, self.output, events))
        self.assertEqual(runner.audit_events(events)["native_protocol_records"], 14)

    def test_each_of_four_native_signals_is_required(self):
        for fields in ({"direction": "stdin", "request_subtype": "interrupt"},
                       {"direction": "stdout", "response_subtype": "success"},
                       {"type": "command_lifecycle", "state": "cancelled"},
                       {"type": "result", "terminal_reason": "aborted_streaming"}):
            with self.subTest(fields=fields):
                events = complete_events()
                row = native_of(events, **fields)
                events[:] = [event for event in events if event.get("native") is not row]
                resequence(events)
                self.assert_rejected(events)

    def test_ack_only_and_approval_retraction_only_are_not_execution_cancellation(self):
        for keep in ("control_response", "control_cancel_request"):
            with self.subTest(keep=keep):
                events = complete_events()
                events[:] = [event for event in events if event["event"] != "native_protocol_ids"
                             or event["native"].get("type") == keep]
                resequence(events)
                self.assert_rejected(events)

    def test_old_control_request_or_ack_cannot_finish_current_execution(self):
        for fields, key in (({"request_subtype": "interrupt"}, "request_id"),
                            ({"response_subtype": "success"}, "response_request_id")):
            with self.subTest(key=key):
                events = complete_events()
                native_of(events, **fields)[key] = f"infinishell-{identity(100)}-7"
                self.assert_rejected(events)

    def test_foreign_session_uuid_and_generation_are_rejected(self):
        for kind, key in (("result", "session_id"), ("result", "user_message_uuid"),
                          ("command_lifecycle", "command_uuid")):
            with self.subTest(key=key):
                events = complete_events()
                native_of(events, type=kind)[key] = identity(100)
                self.assert_rejected(events)
        events = complete_events()
        event_of(events, "native_protocol_ids")["generation"] = identity(100)
        self.assert_rejected(events)

    def test_complete_single_input_uuid_set_cannot_be_missing_mixed_or_duplicated(self):
        for value in (None, [], [identity(1), identity(2)], [identity(1), identity(1)]):
            with self.subTest(value=value):
                events = complete_events()
                native_of(events, type="result")["user_message_uuids"] = value
                self.assert_rejected(events)

    def test_regular_api_error_cannot_be_upgraded_after_interrupt_ack(self):
        events = complete_events()
        native_of(events, type="result")["terminal_reason"] = "api_error"
        self.assert_rejected(events)

    def test_terminal_success_cancellation_shapes_remain_strict(self):
        for reason in ("interrupted", "cancelled"):
            with self.subTest(reason=reason):
                events = complete_events()
                row = native_of(events, type="result")
                row.update(terminal_reason=reason, subtype="success", is_error=False)
                event_of(events, "native_pending_edit_cancel_verified").update(terminal_reason=reason, is_error=False)
                self.assertTrue(runner.verified_acceptance(0, self.output, events))

    def test_native_ack_result_and_lifecycle_can_arrive_in_any_order(self):
        import itertools
        for order in itertools.permutations((7, 8, 9)):
            with self.subTest(order=order):
                events = complete_events()
                projections = [event for event in events if event["event"] == "native_protocol_ids"]
                original = copy.deepcopy([event["native"] for event in projections])
                for index, source in zip((7, 8, 9), order):
                    projections[index]["native"] = original[source]
                self.assertTrue(runner.verified_acceptance(0, self.output, events))

    def test_duplicate_native_proofs_and_old_callbacks_are_rejected(self):
        for kind in ("control_response", "result"):
            with self.subTest(kind=kind):
                events = complete_events()
                event = event_of(events, "native_protocol_ids")
                duplicate = copy.deepcopy(event)
                duplicate["native"] = copy.deepcopy(native_of(events, type=kind))
                events.insert(events.index(event) + 1, duplicate)
                resequence(events)
                self.assert_rejected(events)

    def test_permission_answer_or_other_tool_is_rejected(self):
        events = complete_events()
        first = event_of(events, "native_protocol_ids")
        answer = copy.deepcopy(first)
        answer["native"] = {"direction": "stdin", "type": "control_response",
                            "response_request_id": identity(6), "response_subtype": "success"}
        events.insert(events.index(first) + 1, answer)
        resequence(events)
        self.assert_rejected(events)
        events = complete_events()
        native_of(events, type="assistant")["tools"][0]["name"] = "Bash"
        self.assert_rejected(events)

    def test_optional_single_read_before_edit_is_bounded_and_not_cancel_evidence(self):
        events = complete_events()
        assistant = native_of(events, type="assistant")
        assistant["tools"].insert(0, {"id": "toolu_read_fixture", "name": "Read"})
        event_of(events, "native_pending_edit_cancel_verified")["read_call_count"] = 1
        self.assertTrue(runner.verified_acceptance(0, self.output, events))
        assistant["tools"].insert(0, {"id": "toolu_read_second", "name": "Read"})
        self.assert_rejected(events)

    def test_edit_parameters_cannot_expand_or_change(self):
        for key, value in (("replace_all", True), ("input_keys", ["file_path", "new_string", "old_string", "unexpected"]),
                           ("file_ref", "other.txt"), ("old_string_sha256", "b" * 64), ("edit_allowed", True)):
            with self.subTest(key=key):
                events = complete_events()
                event_of(events, "edit_approval_requested")[key] = value
                self.assert_rejected(events)

    def test_file_must_be_full_bytes_unchanged_at_every_checkpoint(self):
        for phase in ("before", "after_cancel", "after_continue", "after_shutdown"):
            with self.subTest(phase=phase):
                events = complete_events()
                event_of(events, "file_unchanged", phase=phase)["file_sha256"] = "b" * 64
                self.assert_rejected(events)

    def test_cleanup_requires_stdio_closed_zero_exit_and_true_receipt(self):
        for key, value in (("exit_reason", "stop_requested"), ("exit_code", 1), ("cleanup_confirmed", False),
                           ("generation", identity(100)), ("version", 2)):
            with self.subTest(key=key):
                events = complete_events()
                event_of(events, "cleanup_checked")["receipt"][key] = value
                self.assert_rejected(events)

    def test_continue_cannot_use_old_result_or_wrong_marker(self):
        for key, value in (("user_message_uuid", identity(1)), ("user_message_uuids", [identity(1)]),
                           ("uuid", identity(7)), ("session_id", identity(100))):
            with self.subTest(key=key):
                events = complete_events()
                native_of(events, type="result", subtype="success")[key] = value
                self.assert_rejected(events)
        events = complete_events()
        event_of(events, "turn_finished", phase="continue")["trimmed_output_sha256"] = "b" * 64
        self.assert_rejected(events)

    def test_extra_inputs_or_native_calls_exceed_fixed_budget(self):
        events = complete_events()
        first = event_of(events, "native_protocol_ids")
        extra = copy.deepcopy(first)
        extra["native"]["uuid"] = identity(100)
        events.insert(events.index(first) + 1, extra)
        resequence(events)
        self.assert_rejected(events)

    def test_boolean_counts_and_failed_libtest_never_count_as_success(self):
        events = complete_events()
        event_of(events, "acceptance_passed")["native_inputs"] = True
        self.assert_rejected(events)
        self.assertFalse(runner.verified_acceptance(101, self.output, complete_events()))
        self.assertFalse(runner.verified_acceptance(0, "test result: ok. 0 passed;", complete_events()))
        self.assertFalse(runner.verified_acceptance(0, self.output + self.output, complete_events()))

    def test_failure_keeps_safe_native_ids_but_drops_unknown_bodies(self):
        events = complete_events()
        events.append({"event": "acceptance_failed", "scope": runner.SCOPE, "reason_sha256": "b" * 64})
        events.append({"event": "native_protocol_ids", "generation": identity(5), "sequence": 15,
                       "native": {"direction": "stdout", "type": "assistant", "body": "非公开正文"}})
        projected, safe = runner.project_events(events)
        self.assertFalse(safe)
        self.assertEqual(len([event for event in projected if event["event"] == "native_protocol_ids"]), 14)
        self.assertNotIn("非公开正文", json.dumps(projected, ensure_ascii=False))
        self.assert_rejected(projected)

    def test_arbitrary_strings_cannot_escape_via_native_id_or_marker(self):
        for key, value in (("request_id", "任意私人正文"), ("type", "任意私人正文")):
            with self.subTest(key=key):
                events = complete_events()
                native_of(events, request_subtype="interrupt")[key] = value
                projected, safe = runner.project_events(events)
                self.assertFalse(safe)
                self.assertNotIn(value, json.dumps(projected, ensure_ascii=False))


class ApprovalCancelWrapperTests(unittest.TestCase):
    def mocked_run(self, passed, exit_code, output, events):
        with tempfile.TemporaryDirectory(prefix="approval-cancel-wrapper-test-") as temporary:
            root = Path(temporary)
            api = root / "api.json"
            api.write_text("{}")
            api.chmod(0o600)
            args = SimpleNamespace(api_environment_file=api, model="offline-fixture", max_native_inputs=2,
                                   timeout_seconds=900, output=root / "evidence.ndjson")
            original = {key: getattr(runner.adapter, key) for key in
                        ("TEST_NAME", "PROJECT_SETTINGS", "prepare_project", "verified_acceptance", "authenticated_environment")}
            observed_private = []
            def run(received):
                observed_private.append(received.auth_home.parent)
                self.assertEqual(runner.adapter.TEST_NAME, runner.TEST_NAME)
                self.assertEqual(runner.adapter.PROJECT_SETTINGS, {"permissions": {"ask": ["Edit"]}})
                received.output.write_text("".join(json.dumps(event) + "\n" for event in events))
                received.output.with_suffix(".metadata.json").write_text(json.dumps({
                    "acceptance_passed": passed, "test_exit_code": exit_code, "project_settings_unchanged": True}))
                received.output.with_suffix(".test-output.txt").write_text(output)
                return 0 if passed else 1
            try:
                with patch.object(runner.adapter, "validate_paths"), patch.object(runner.adapter, "run", side_effect=run):
                    result = runner.run(args)
                metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
                transcript = json.loads(args.output.with_suffix(".test-output.txt").read_text())
                public = [json.loads(line) for line in args.output.read_text().splitlines()]
                self.assertEqual({key: getattr(runner.adapter, key) for key in original}, original)
                return result, metadata, transcript, public
            finally:
                # 只回收本离线模拟创建的空目录，不接触真实认证或其它工作区。
                for private in observed_private:
                    (private / "claude").rmdir(); (private / "home").rmdir(); private.rmdir()

    def test_success_rechecks_public_proof_and_archives_stdout_hash_only(self):
        output = ApprovalCancelAuditTests.output
        result, metadata, transcript, public = self.mocked_run(True, 0, output, complete_events())
        self.assertEqual(result, 0)
        self.assertTrue(metadata["waiting_edit_cancel_verified"])
        self.assertFalse(transcript["raw_stdout_archived"])
        self.assertEqual(transcript["sanitized_stdout_sha256"], hashlib.sha256(output.encode()).hexdigest())
        self.assertEqual(transcript["test_success_summary_count"], 1)
        self.assertNotIn("test result:", json.dumps(transcript))
        self.assertEqual(len([event for event in public if event["event"] == "native_protocol_ids"]), 14)

    def test_failed_test_keeps_partial_native_evidence_without_promoting_success(self):
        events = complete_events()[:-2]
        events.append({"event": "acceptance_failed", "scope": runner.SCOPE, "reason_sha256": "b" * 64})
        result, metadata, transcript, public = self.mocked_run(False, 101, "test result: FAILED", events)
        self.assertEqual(result, 1)
        self.assertFalse(metadata["acceptance_passed"])
        self.assertFalse(metadata["waiting_edit_cancel_verified"])
        self.assertEqual(metadata["test_exit_code"], 101)
        self.assertFalse(transcript["raw_stdout_archived"])
        self.assertEqual(len([event for event in public if event["event"] == "native_protocol_ids"]), 14)

    def test_unknown_projection_cannot_keep_old_success_flag(self):
        events = complete_events()
        events.append({"event": "unknown_event", "private_body": "非公开正文"})
        result, metadata, _, public = self.mocked_run(True, 0, ApprovalCancelAuditTests.output, events)
        self.assertEqual(result, 1)
        self.assertFalse(metadata["acceptance_passed"])
        self.assertFalse(metadata["public_projection_verified"])
        self.assertNotIn("非公开正文", json.dumps(public, ensure_ascii=False))

    def test_base_success_flag_cannot_override_real_nonzero_exit(self):
        result, metadata, _, _ = self.mocked_run(True, 101, ApprovalCancelAuditTests.output, complete_events())
        self.assertEqual(result, 1)
        self.assertFalse(metadata["acceptance_passed"])

    def test_nonzero_credential_shape_count_revokes_success_without_archiving_value(self):
        canary = "Bearer " + "A" * 32
        result, metadata, transcript, _ = self.mocked_run(True, 0, ApprovalCancelAuditTests.output + "\n" + canary,
                                                       complete_events())
        self.assertEqual(result, 1)
        self.assertFalse(metadata["waiting_edit_cancel_verified"])
        self.assertEqual(transcript["credential_shape_counts"]["bearer"], 1)
        self.assertNotIn(canary, json.dumps(transcript))


if __name__ == "__main__":
    unittest.main()
