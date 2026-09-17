"""合并批次取消运行器的离线回归；不读取真实认证或启动 CLI。"""

import copy
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import uuid

import run_claude_batch_cancel_live as runner


def fixture_id(number):
    return str(uuid.UUID(int=number))


def complete_events():
    a, j, c, interrupt = [fixture_id(number) for number in (1, 2, 3, 4)]
    native, generation, approval = [fixture_id(number) for number in (10, 11, 12)]
    request_id = f"infinishell-{generation}-7"
    result_id, continue_id = fixture_id(20), fixture_id(21)
    profile_hash, output_hash = "a" * 64, hashlib.sha256(b"").hexdigest()
    tool_id = "toolu_batch_probe"
    marker = "BATCH_CONTINUED_" + "f" * 32
    def lifecycle(input_id, state):
        return {"direction": "stdout", "type": "command_lifecycle", "session_id": native,
                "command_uuid": input_id, "state": state}
    rows = [
        {"direction": "stdin", "type": "user", "uuid": a, "session_id": "sha256:" + output_hash},
        lifecycle(a, "queued"), lifecycle(a, "started"),
        {"direction": "stdout", "type": "assistant", "session_id": native,
         "tools": [{"id": tool_id, "name": runner.INSPECT_TOOL}]},
        {"direction": "stdout", "type": "control_request", "request_subtype": "can_use_tool",
         "request_id": approval, "tool_use_id": tool_id},
        {"direction": "stdin", "type": "user", "uuid": j, "session_id": native},
        lifecycle(j, "queued"),
        {"direction": "stdin", "type": "control_response", "response_request_id": approval,
         "response_subtype": "success"},
        lifecycle(j, "started"),
        {"direction": "stdin", "type": "control_request", "request_subtype": "interrupt", "request_id": request_id},
        {"direction": "stdout", "type": "control_response", "response_request_id": request_id,
         "response_subtype": "success", "request_id": None},
        {"direction": "stdout", "type": "result", "uuid": result_id, "session_id": native,
         "subtype": "error_during_execution", "is_error": True, "terminal_reason": "aborted_streaming",
         "user_message_uuid": j, "user_message_uuids": [a, j]},
        lifecycle(a, "cancelled"),
        {"direction": "stdin", "type": "user", "uuid": c, "session_id": native},
        lifecycle(c, "queued"), lifecycle(c, "started"),
        {"direction": "stdout", "type": "result", "uuid": continue_id, "session_id": native,
         "subtype": "success", "is_error": False, "user_message_uuid": c, "user_message_uuids": [c]},
    ]
    events = [
        {"event": "acceptance_started", "scope": runner.SCOPE, "max_native_inputs": 3, "max_inspect_calls": 1,
         "permission_policy": "ClaudeRestrictedFilesV1", "credential_files_read_by_probe": False,
         "real_gui_verified": False, "persistence_verified": False},
        {"event": "profile_verified", "permission_mode": "plan", "fixed_profile_verified": True,
         "profile_sha256": profile_hash, "filesystem_sandbox_verified": False, "parent_permission_ceiling_verified": False},
        {"event": "input_submitted", "phase": "batch_cancel", "message_id": a, "submitted_input_count": 1},
        {"event": "message_accepted", "message_id": a, "turn_id": a, "native_session_id": native, "native_receipt": True},
        {"event": "turn_started", "turn_id": a, "native_session_id": native},
        {"event": "inspect_approval_requested", "approval_id": approval, "turn_id": a, "exact_inspect_fixture": True},
        {"event": "input_submitted", "phase": "batch_cancel", "message_id": j, "active_turn_id": a,
         "submitted_input_count": 2, "submitted_while_running": True},
        {"event": "message_accepted", "message_id": j, "turn_id": j, "native_session_id": native, "native_receipt": True},
        {"event": "inspect_approval_allowed", "approval_id": approval, "decision": "AllowOnce", "native_receipt": False},
        {"event": "inspect_literal_returned", "call_id": "inspect-fixture-call", "turn_id": a,
         "inspect_call_count": 1, "no_project_read_or_write": True, "native_receipt": False, "transport_write_confirmed": True},
        {"event": "input_joined", "message_id": j, "turn_id": a, "native_session_id": native},
        {"event": "interrupt_submitted", "message_id": interrupt, "execution_turn_id": a, "joined_input_id": j,
         "both_inputs_native_acknowledged": True, "input_joined_verified": True},
        {"event": "interrupt_accepted", "message_id": interrupt, "turn_id": a,
         "native_session_id": native, "native_receipt": True},
        {"event": "turn_finished", "phase": "batch_cancel", "turn_id": j, "native_session_id": native,
         "outcome": "Cancelled", "output_bytes": 0, "output_sha256": output_hash, "batch_terminal_index": 1},
        {"event": "turn_finished", "phase": "batch_cancel", "turn_id": a, "native_session_id": native,
         "outcome": "Cancelled", "output_bytes": 0, "output_sha256": output_hash, "batch_terminal_index": 2},
        {"event": "input_submitted", "phase": "continue", "message_id": c, "submitted_input_count": 3},
        {"event": "message_accepted", "message_id": c, "turn_id": c, "native_session_id": native, "native_receipt": True},
        {"event": "turn_started", "turn_id": c, "native_session_id": native},
        {"event": "turn_finished", "phase": "continue", "turn_id": c, "native_session_id": native,
         "outcome": "Completed", "output": marker, "same_native_session_verified": True},
        {"event": "connection_shutdown", "native_session_id": native},
        {"event": "cleanup_checked", "generation": generation, "cleanup_confirmed": True,
         "transport_closed": True, "cleanup_receipt_read": True,
         "receipt": {"generation": generation, "containment": "macos_resource_coalition",
                     "cleanup_confirmed": True, "exit_code": 0, "exit_reason": "stdio_closed"}},
    ]
    events.extend({"event": "native_protocol_ids", "sequence": index, "native": row} for index, row in enumerate(rows, 1))
    events.extend([
        {"event": "native_batch_cancel_verified", "native_session_id": native, "execution_turn_id": a,
         "joined_input_id": j, "interrupt_request_id": request_id, "native_result_uuid": result_id,
         "terminal_reason": "aborted_streaming", "is_error": True, "user_message_uuids": [a, j],
         "native_input_ack_count": 2, "native_interrupt_ack_verified": True,
         "native_execution_cancelled_verified": True, "native_complete_batch_result_verified": True,
         "all_three_signals_verified": True},
        {"event": "acceptance_passed", "scope": runner.SCOPE, "native_session_id": native,
         "native_inputs": 3, "native_executions": 2, "joined_inputs": 1, "cancelled_inputs": 2,
         "continued_inputs": 1, "inspect_call_count": 1, "all_three_signals_verified": True,
         "same_native_session_verified": True, "cleanup_confirmed": True, "transport_closed": True,
         "credential_files_read_by_probe": False, "persistence_verified": False,
         "app_restart_and_ui_verified": False, "parent_permission_ceiling_verified": False},
    ])
    return events


def event_of(events, kind):
    return next(event for event in events if event["event"] == kind)


def native_of(events, **filters):
    return next(event["native"] for event in events if event["event"] == "native_protocol_ids"
                and all(event["native"].get(key) == value for key, value in filters.items()))


class BatchCancelRunnerTests(unittest.TestCase):
    output = "test result: ok. 1 passed; 0 failed; 0 ignored; 7400 filtered out"

    def accepts(self, events):
        return runner.verified_acceptance(0, self.output, events)

    def test_complete_native_proof_passes_and_summary_only_does_not(self):
        events = complete_events()
        self.assertTrue(self.accepts(events))
        self.assertFalse(self.accepts([events[-1]]))
        self.assertFalse(runner.verified_acceptance(1, self.output, events))
        self.assertFalse(runner.verified_acceptance(0, "test result: ok. 0 passed; 0 failed; 1 ignored;", events))
        self.assertFalse(self.accepts(events + events))
        self.assertFalse(self.accepts(events + [{"event": "acceptance_failed"}]))

    def test_every_required_record_is_necessary(self):
        for index in range(len(complete_events())):
            events = complete_events()
            events.pop(index)
            self.assertFalse(self.accepts(events), index)

    def test_each_cancel_signal_is_necessary_after_sequence_is_repaired(self):
        for filters in ({"direction": "stdin", "request_subtype": "interrupt"},
                        {"direction": "stdout", "response_subtype": "success"},
                        {"direction": "stdout", "state": "cancelled"},
                        {"direction": "stdout", "type": "result", "subtype": "error_during_execution"}):
            events = complete_events()
            native = native_of(events, **filters)
            events = [event for event in events if event.get("native") is not native]
            for number, event in enumerate((event for event in events if event["event"] == "native_protocol_ids"), 1):
                event["sequence"] = number
            self.assertFalse(self.accepts(events), filters)

    def test_started_is_a_native_ack_without_an_optional_queued_frame(self):
        events = [event for event in complete_events() if not (event["event"] == "native_protocol_ids"
                  and event["native"].get("state") == "queued")]
        for index, event in enumerate((event for event in events if event["event"] == "native_protocol_ids"), 1):
            event["sequence"] = index
        self.assertTrue(self.accepts(events))

    def test_runtime_ids_receipts_and_cancelled_order_cannot_be_replaced(self):
        cases = (("message_accepted", "native_receipt", False), ("message_accepted", "native_session_id", fixture_id(99)),
                 ("input_joined", "turn_id", fixture_id(99)), ("input_joined", "message_id", fixture_id(99)),
                 ("interrupt_accepted", "native_receipt", False), ("interrupt_accepted", "message_id", fixture_id(99)),
                 ("turn_finished", "outcome", "Failed"), ("turn_finished", "batch_terminal_index", 2),
                 ("inspect_literal_returned", "transport_write_confirmed", False),
                 ("inspect_literal_returned", "native_receipt", True), ("inspect_approval_allowed", "native_receipt", True),
                 ("profile_verified", "permission_mode", "bypassPermissions"),
                 ("acceptance_passed", "persistence_verified", True), ("acceptance_passed", "native_inputs", 4))
        for kind, key, value in cases:
            events = complete_events(); event_of(events, kind)[key] = value
            self.assertFalse(self.accepts(events), (kind, key))
        events = complete_events(); finishes = [event for event in events if event["event"] == "turn_finished"]
        finishes[0]["turn_id"], finishes[1]["turn_id"] = finishes[1]["turn_id"], finishes[0]["turn_id"]
        self.assertFalse(self.accepts(events))

    def test_native_nested_ack_not_top_level_request_id_is_required(self):
        for key, value in (("response_request_id", None), ("response_request_id", fixture_id(99)),
                           ("response_subtype", "error"), ("response_subtype", None)):
            events = complete_events()
            ack = native_of(events, direction="stdout", type="control_response")
            ack["request_id"] = ack["response_request_id"]; ack[key] = value
            self.assertFalse(self.accepts(events), key)

    def test_complete_batch_uuid_list_and_correlated_result_are_required(self):
        for key, value in (("user_message_uuids", [fixture_id(2)]),
                           ("user_message_uuids", [fixture_id(1), fixture_id(1)]),
                           ("user_message_uuids", [fixture_id(1), fixture_id(99)]),
                           ("user_message_uuid", fixture_id(99)), ("session_id", fixture_id(99)),
                           ("terminal_reason", "api_error"), ("subtype", "success"), ("is_error", None)):
            events = complete_events(); native_of(events, type="result", subtype="error_during_execution")[key] = value
            self.assertFalse(self.accepts(events), key)
        events = complete_events(); native_of(events, type="result", subtype="error_during_execution")["user_message_uuids"].reverse()
        self.assertTrue(self.accepts(events))

    def test_only_explicit_success_interrupted_or_cancelled_variant_is_allowed(self):
        for reason in ("interrupted", "cancelled"):
            events = complete_events(); result = native_of(events, type="result", subtype="error_during_execution")
            result.update(terminal_reason=reason, subtype="success", is_error=False)
            proof = event_of(events, "native_batch_cancel_verified"); proof.update(terminal_reason=reason, is_error=False)
            self.assertTrue(self.accepts(events))
            for key, invalid in (("subtype", "unknown"), ("is_error", None), ("is_error", True)):
                broken = copy.deepcopy(events); native_of(broken, type="result", terminal_reason=reason)[key] = invalid
                self.assertFalse(self.accepts(broken), (reason, key))

    def test_duplicate_submission_extra_tool_or_unsafe_native_payload_is_rejected(self):
        for mutation in ("duplicate_input", "extra_tool", "private_body"):
            events = complete_events()
            if mutation == "duplicate_input": native_of(events, direction="stdin", uuid=fixture_id(3))["uuid"] = fixture_id(2)
            elif mutation == "extra_tool": native_of(events, type="assistant")["tools"].append({"id": "toolu_extra", "name": "Write"})
            else: native_of(events, type="assistant")["errors"] = ["任意私人正文"]
            self.assertFalse(self.accepts(events), mutation)

    def test_continue_is_exact_completed_same_session_with_full_single_input_result(self):
        for key, value in (("session_id", fixture_id(99)), ("user_message_uuids", []),
                           ("user_message_uuids", [fixture_id(3), fixture_id(2)]),
                           ("user_message_uuid", fixture_id(2)), ("subtype", "unknown"), ("is_error", True),
                           ("terminal_reason", "api_error"), ("terminal_reason", "unknown")):
            events = complete_events(); native_of(events, type="result", subtype="success")[key] = value
            self.assertFalse(self.accepts(events), key)
        events = complete_events(); next(event for event in events if event["event"] == "turn_finished" and event["phase"] == "continue")["output"] += "\n额外正文"
        self.assertFalse(self.accepts(events))

    def test_interrupt_before_join_or_before_native_started_is_not_a_batch_cancel(self):
        events = complete_events(); join, interrupt = event_of(events, "input_joined"), event_of(events, "interrupt_submitted")
        events.remove(join); events.insert(events.index(interrupt) + 1, join)
        self.assertFalse(self.accepts(events))
        events = complete_events(); native_of(events, request_subtype="interrupt")["request_subtype"] = "set_permission_mode"
        self.assertFalse(self.accepts(events))

    def test_native_ledger_sequence_old_generation_or_cleanup_without_receipt_is_rejected(self):
        for kind, key, value in (("native_protocol_ids", "sequence", 0), ("cleanup_checked", "generation", fixture_id(99)),
                                  ("cleanup_checked", "receipt", None), ("cleanup_checked", "cleanup_confirmed", False),
                                  ("cleanup_checked", "transport_closed", False), ("cleanup_checked", "cleanup_receipt_read", False)):
            events = complete_events(); event_of(events, kind)[key] = value
            self.assertFalse(self.accepts(events), key)
        for key, value in (("containment", "not_started"), ("cleanup_confirmed", False), ("exit_code", 1)):
            events = complete_events(); event_of(events, "cleanup_checked")["receipt"][key] = value
            self.assertFalse(self.accepts(events), key)

    def test_same_profile_repeated_ready_is_allowed_but_drift_is_not(self):
        events = complete_events(); profile = copy.deepcopy(event_of(events, "profile_verified")); events.insert(2, profile)
        self.assertTrue(self.accepts(events))
        profile["profile_sha256"] = "b" * 64; self.assertFalse(self.accepts(events))

    def test_old_generation_control_id_cannot_be_repaired_by_an_old_ack_and_summary(self):
        events = complete_events(); old_id = f"infinishell-{fixture_id(99)}-7"
        control = native_of(events, request_subtype="interrupt"); current_id = control["request_id"]
        control["request_id"] = old_id
        native_of(events, response_request_id=current_id)["response_request_id"] = old_id
        event_of(events, "native_batch_cancel_verified")["interrupt_request_id"] = old_id
        self.assertFalse(self.accepts(events))

    def test_boolean_values_cannot_substitute_for_numeric_counts_or_zero_exit_status(self):
        for kind, key, value in (("acceptance_started", "max_inspect_calls", True),
                                  ("input_submitted", "submitted_input_count", True),
                                  ("inspect_literal_returned", "inspect_call_count", True),
                                  ("turn_finished", "batch_terminal_index", True),
                                  ("acceptance_passed", "joined_inputs", True)):
            events = complete_events(); event_of(events, kind)[key] = value
            self.assertFalse(self.accepts(events), (kind, key))
        events = complete_events(); event_of(events, "cleanup_checked")["receipt"]["exit_code"] = False
        self.assertFalse(self.accepts(events))

    def test_prepare_project_only_creates_existing_base_isolation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(runner.adapter, "PROJECT_SETTINGS", runner.PROJECT_SETTINGS):
                settings = runner.prepare_project(root)
            self.assertEqual(json.loads(settings.read_text()), {"permissions": {"ask": ["Edit"]}})
            self.assertEqual((root / ".infinishell-claude-live-probe").read_text(), runner.adapter.MARKER)
            self.assertEqual(sorted(path.name for path in (root / "project").iterdir()), [".claude"])

    def test_api_and_model_are_explicit_no_existing_login_fallback(self):
        for args in (SimpleNamespace(api_environment_file=None), SimpleNamespace(api_environment_file=Path("unused"), model=None)):
            with self.assertRaises(ValueError): runner.run(args)

    def test_existing_runner_global_hooks_restore_on_success_failure_or_exception(self):
        for state in ("success", "failure", "corrupt_success", "exception"):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary); private = root / "private"; private.mkdir()
                api = root / "explicit-api.json"; api.write_text("{}"); api.chmod(0o600)
                args = SimpleNamespace(api_environment_file=api, model="offline-fixture-model", output=root / "proof.ndjson")
                original = {key: getattr(runner.adapter, key) for key in ("TEST_NAME", "PROJECT_SETTINGS", "prepare_project", "verified_acceptance")}
                def fake_run(value):
                    self.assertIs(value, args); self.assertEqual(runner.adapter.TEST_NAME, runner.TEST_NAME)
                    self.assertIs(runner.adapter.verified_acceptance, runner.verified_acceptance)
                    self.assertEqual(list(args.config_dir.iterdir()), []); self.assertEqual(list(args.auth_home.iterdir()), [])
                    if state == "exception": raise ValueError("离线固定失败")
                    metadata = {"acceptance_passed": state in ("success", "corrupt_success")}
                    args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata))
                    if state == "success": args.output.write_text("".join(json.dumps(event) + "\n" for event in complete_events()))
                    if state == "corrupt_success": args.output.write_text(json.dumps(complete_events()[-1]) + "\n")
                    return 0 if state in ("success", "corrupt_success") else 1
                with patch.object(runner.tempfile, "mkdtemp", return_value=str(private)), patch.object(runner.adapter, "validate_paths"), patch.object(runner.adapter, "run", side_effect=fake_run):
                    if state == "exception":
                        with self.assertRaises(ValueError): runner.run(args)
                    else: self.assertEqual(runner.run(args), 0 if state == "success" else 1)
                for key, value in original.items(): self.assertIs(getattr(runner.adapter, key), value)
                if state != "exception":
                    metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
                    self.assertEqual(metadata["scope"], runner.SCOPE)
                    self.assertEqual(metadata["acceptance_passed"], state == "success")
                    self.assertEqual(metadata["joined_batch_cancel_verified"], state == "success")
                    self.assertFalse(metadata["app_restart_and_ui_verified"])
                    self.assertFalse(metadata["http_request_count_verified"])


if __name__ == "__main__":
    unittest.main()
