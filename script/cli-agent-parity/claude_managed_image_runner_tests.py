"""生产PNG验收证据的离线回归；不启动CLI、模型或读取认证文件。"""

import copy
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_claude_managed_image_live as runner


SUCCESS = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n"
NATIVE = "00000000-0000-4000-8000-000000000010"
GENERATIONS = ["00000000-0000-4000-8000-000000000011", "00000000-0000-4000-8000-000000000012"]
INPUTS = ["00000000-0000-4000-8000-000000000021", "00000000-0000-4000-8000-000000000022", "00000000-0000-4000-8000-000000000023"]
IMAGE_SHA = "a" * 64
ARRAY_SHA = "b" * 64
PROMPT_SHA = "356737d1060e0761b76a4b7cb20ffb253b4b3d0209229def05c63e0d150f0c06"
COLOR_SHA = hashlib.sha256(b"RED GREEN BLUE YELLOW").hexdigest()
TEXT_SHA = hashlib.sha256(b"CLAUDE_PNG_TEXT").hexdigest()


def native(kind, generation, **fields):
    row = {key: None for key in runner.TRACE_KEYS}
    row.update({"type": kind, "runtime_generation": generation, "tools": []})
    row.update(fields)
    return row


def complete_proof():
    events = [{"event": "acceptance_started", "scope": runner.SCOPE, "max_native_inputs": 3,
        "production_adapter": True, "credential_files_read_by_probe": False, "real_gui_verified": False,
        "sqlite_verified": False, "parent_permission_ceiling_verified": False, "http_request_count_verified": False},
        {"event": "attachment_prepared", "image_sha256": IMAGE_SHA, "image_bytes": 12420,
         "prompt_sha256": PROMPT_SHA, "prompt_bytes": 310, "block_types": ["text", "image"],
         "replay_array_sha256": ARRAY_SHA,
         "media_type": "image/png", "attachment_hash_name": IMAGE_SHA + ".png", "reference_persisted": True,
         "expected_reply_sha256": COLOR_SHA}]
    protocol = []
    for index, (phase, message_id, output_sha) in enumerate(zip(runner.PHASES, INPUTS, [COLOR_SHA, TEXT_SHA, COLOR_SHA])):
        generation = GENERATIONS[0 if index < 2 else 1]
        if index in (0, 2):
            events.append({"event": "managed_connection_ready", "phase": "new" if index == 0 else "resume",
                           "generation": generation, "native_session_id": None if index == 0 else NATIVE,
                           "requested_native_session_id": None if index == 0 else NATIVE,
                           "native_session_association_confirmed": index != 0, "inputs_replayed": 0})
            protocol.append(native("control_response", generation, response_request_id=f"infinishell-{generation}-1",
                                   response_subtype="success"))
        events.extend([
            {"event": "input_submitted", "phase": phase, "generation": generation, "message_id": message_id,
             "typed_input": True, "controller_send_count": 2 if index == 0 else 1,
             "identical_message_retransmitted": index == 0, "expected_reply_sha256": output_sha},
            {"event": "message_accepted", "phase": phase, "generation": generation, "message_id": message_id,
             "turn_id": message_id, "native_session_id": NATIVE, "receipt_source": "NativeProtocol"},
            {"event": "turn_started", "phase": phase, "generation": generation, "turn_id": message_id, "native_session_id": NATIVE},
            {"event": "turn_finished", "phase": phase, "generation": generation, "turn_id": message_id,
             "native_session_id": NATIVE, "outcome": "Completed", "full_output_sha256": output_sha,
             "trimmed_output_sha256": output_sha, "output_bytes": 15 if index == 1 else 21}])
        projection = {"array_sha256": ARRAY_SHA, "text_sha256": PROMPT_SHA, "text_bytes": 310,
                      "image_sha256": IMAGE_SHA, "image_bytes": 12420, "block_types": ["text", "image"],
                      "media_type": "image/png"} if index == 0 else None
        protocol.extend([
            native("command_lifecycle", generation, session_id=NATIVE, command_uuid=message_id, state="queued"),
            native("command_lifecycle", generation, session_id=NATIVE, command_uuid=message_id, state="started"),
            native("user", generation, session_id=NATIVE, uuid=message_id, image_content_projection=projection),
            native("assistant", generation, session_id=NATIVE, uuid=f"00000000-0000-4000-8000-00000000004{index}",
                   user_message_uuid=message_id, user_message_uuids=[message_id]),
            native("result", generation, session_id=NATIVE, uuid=f"00000000-0000-4000-8000-00000000005{index}",
                   user_message_uuid=message_id, user_message_uuids=[message_id], subtype="success", is_error=False,
                   terminal_reason="completed", result_text_sha256=output_sha)])
        if index in (1, 2):
            events.extend([
                {"event": "connection_shutdown", "native_session_id": NATIVE},
                {"event": "cleanup_checked", "generation": generation, "native_session_id": NATIVE,
                 "transport_closed": True, "cleanup_receipt_read": True, "cleanup_confirmed": True,
                 "normal_exit": True, "receipt": {"version": 1, "generation": generation,
                    "cleanup_confirmed": True, "containment": "macos_resource_coalition",
                    "exit_reason": "stdio_closed", "exit_code": 0}}])
            if index == 1:
                events.append({"event": "attachment_restore_verified", "image_sha256": IMAGE_SHA,
                               "image_bytes": 12420, "typed_reference_matches": True, "image_replayed_to_native": False})
    events.append({"event": "acceptance_passed", "scope": runner.SCOPE, "native_session_id": NATIVE,
        "native_inputs": 3, "runtime_generations": 2, "same_id_deduplication_verified": True,
        "durable_attachment_restore_verified": True, "production_adapter_verified": True, "native_framing_only": False,
        "app_restart_and_ui_verified": False, "sqlite_verified": False, "parent_permission_ceiling_verified": False,
        "http_request_count_verified": False})
    output = "".join(runner.MARKER + json.dumps(row) + "\n" for row in protocol) + SUCCESS
    return events, output


def fixture():
    events, output = complete_proof()
    return events + runner.project_protocol_output(output)


def one(events, kind, **filters):
    return next(e for e in events if e["event"] == kind and all(e.get(k) == v for k, v in filters.items()))


class ManagedImageAuditTests(unittest.TestCase):
    def test_complete_production_proof_passes_and_summary_alone_fails(self):
        events, output = complete_proof()
        self.assertTrue(runner.verified_acceptance(0, output, events))
        self.assertFalse(runner.verified_acceptance(0, SUCCESS, events))
        self.assertFalse(runner.verified_acceptance(1, output, events))
        self.assertFalse(runner.verified_acceptance(False, output, events))
        self.assertEqual(runner.audit_events(fixture())["input_ids"], INPUTS)

    def test_every_runtime_and_storage_proof_is_required(self):
        for kind in ("attachment_prepared", "attachment_restore_verified", "managed_connection_ready",
                     "input_submitted", "message_accepted", "turn_started", "turn_finished", "cleanup_checked"):
            events = fixture()
            events.remove(one(events, kind))
            with self.subTest(kind=kind), self.assertRaises((ValueError, KeyError)):
                runner.audit_events(events)

    def test_native_sha_and_image_shape_must_match_stored_attachment(self):
        for key, value in (("image_sha256", "c" * 64), ("text_sha256", "d" * 64), ("text_bytes", 309),
                           ("image_bytes", 12419), ("block_types", ["image", "text"]),
                           ("media_type", "image/jpeg"), ("array_sha256", "wrong"), ("array_sha256", "e" * 64)):
            events = fixture()
            row = next(e["native"] for e in events if e["event"] == "native_protocol_ids"
                       and e["native"]["image_content_projection"] is not None)
            row["image_content_projection"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                runner.audit_events(events)

    def test_native_full_result_hash_and_exact_ids_are_required(self):
        for key, value in (("result_text_sha256", "f" * 64), ("session_id", INPUTS[0]),
                           ("user_message_uuid", INPUTS[1]), ("user_message_uuids", INPUTS[:2]),
                           ("is_error", True), ("terminal_reason", "cancelled")):
            events = fixture()
            row = next(e["native"] for e in events if e["event"] == "native_protocol_ids" and e["native"]["type"] == "result")
            row[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                runner.audit_events(events)

    def test_extra_or_old_generation_native_callbacks_fail(self):
        for kind in ("user", "result", "command_lifecycle", "assistant"):
            events = fixture()
            event = next(e for e in events if e["event"] == "native_protocol_ids" and e["native"]["type"] == kind)
            if kind in ("user", "result", "command_lifecycle"):
                extra = copy.deepcopy(event)
                extra["sequence"] = len([e for e in events if e["event"] == "native_protocol_ids"]) + 1
                events.append(extra)
            else:
                event["generation"] = GENERATIONS[1]
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                runner.audit_events(events)

    def test_resume_does_not_replay_and_uses_new_generation_with_same_session(self):
        for key, value in (("inputs_replayed", 1), ("native_session_id", INPUTS[0]), ("generation", GENERATIONS[0])):
            events = fixture()
            one(events, "managed_connection_ready", phase="resume")[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                runner.audit_events(events)

    def test_resume_startup_without_native_id_still_requires_real_same_session_receipts(self):
        events = fixture()
        ready = one(events, "managed_connection_ready", phase="resume")
        ready["native_session_id"] = None
        ready["native_session_association_confirmed"] = False
        self.assertEqual(runner.audit_events(events)["input_ids"], INPUTS)
        for key, value in (("native_session_association_confirmed", True),
                           ("requested_native_session_id", INPUTS[0])):
            changed = copy.deepcopy(events)
            one(changed, "managed_connection_ready", phase="resume")[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                runner.audit_events(changed)
        for kind in ("message_accepted", "turn_started", "turn_finished"):
            changed = copy.deepcopy(events)
            one(changed, kind, phase="recall")["native_session_id"] = INPUTS[0]
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                runner.audit_events(changed)

    def test_both_generation_cleanups_require_real_stdio_zero_and_confirmation(self):
        for generation in GENERATIONS:
            for key, value in (("exit_code", 1), ("exit_code", None), ("exit_code", False),
                               ("exit_reason", "host_disconnected"), ("exit_reason", "stop_requested"),
                               ("cleanup_confirmed", False), ("generation", INPUTS[0])):
                events = fixture()
                one(events, "cleanup_checked", generation=generation)["receipt"][key] = value
                with self.subTest(generation=generation, key=key, value=value), self.assertRaises(ValueError):
                    runner.audit_events(events)

    def test_transport_or_durable_attachment_failure_cannot_be_hidden(self):
        for kind, key in (("cleanup_checked", "transport_closed"), ("cleanup_checked", "normal_exit"),
                          ("attachment_restore_verified", "typed_reference_matches")):
            events = fixture()
            one(events, kind)[key] = False
            with self.subTest(kind=kind, key=key), self.assertRaises(ValueError):
                runner.audit_events(events)

    def test_trace_parser_rejects_raw_body_tool_permission_and_unknown_fields(self):
        events, output = complete_proof()
        self.assertTrue(runner.verified_acceptance(0, "test case prefix " + output, events))
        row = native("assistant", GENERATIONS[0], session_id=NATIVE, user_message_uuid=INPUTS[0])
        for key, value in (("content", "PRIVATE_CANARY"), ("environment", {"secret": "PRIVATE_CANARY"}),
                           ("tools", [{"id": "toolu_canary", "name": "Read"}]), ("type", "control_request")):
            changed = copy.deepcopy(row)
            changed[key] = value
            self.assertFalse(runner.verified_acceptance(0, output + runner.MARKER + json.dumps(changed), events))
        self.assertFalse(runner.verified_acceptance(0, output + runner.MARKER + "not JSON", events))

    def test_runtime_records_never_accept_body_fields_or_local_ack(self):
        events = fixture()
        one(events, "turn_finished")["output"] = "PRIVATE_CANARY"
        with self.assertRaises(ValueError):
            runner.audit_events(events)
        events = fixture()
        one(events, "message_accepted")["receipt_source"] = "StateQueued"
        with self.assertRaises(ValueError):
            runner.audit_events(events)

    def test_one_exact_cached_image_ack_is_allowed_before_or_during_multiline(self):
        for phase in ("image", "multiline"):
            events = fixture()
            replay = {"event": "cached_native_ack_replayed", "phase": "image", "observed_phase": phase,
                      "generation": GENERATIONS[0], "message_id": INPUTS[0], "turn_id": INPUTS[0],
                      "native_session_id": NATIVE, "receipt_source": "CachedNativeProtocol", "native_input_added": False}
            events.insert(events.index(one(events, "turn_started", phase=phase)), replay)
            with self.subTest(phase=phase):
                self.assertEqual(runner.audit_events(events)["cached_native_ack_replay_count"], 1)

    def test_cached_ack_cannot_add_native_input_or_change_scope_identity_or_count(self):
        for key, value in (("phase", "multiline"), ("observed_phase", "recall"),
                           ("generation", GENERATIONS[1]), ("message_id", INPUTS[1]), ("turn_id", INPUTS[1]),
                           ("native_session_id", INPUTS[0]), ("receipt_source", "NativeProtocol"),
                           ("native_input_added", True)):
            events = fixture()
            replay = {"event": "cached_native_ack_replayed", "phase": "image", "observed_phase": "image",
                      "generation": GENERATIONS[0], "message_id": INPUTS[0], "turn_id": INPUTS[0],
                      "native_session_id": NATIVE, "receipt_source": "CachedNativeProtocol", "native_input_added": False}
            replay[key] = value
            events.insert(events.index(one(events, "turn_started", phase="image")), replay)
            with self.subTest(key=key), self.assertRaises(ValueError):
                runner.audit_events(events)
        events = fixture()
        accepted = one(events, "message_accepted", phase="image")
        replay = dict(accepted, event="cached_native_ack_replayed", observed_phase="image",
                      receipt_source="CachedNativeProtocol", native_input_added=False)
        insertion = events.index(one(events, "turn_started", phase="image"))
        events[insertion:insertion] = [replay, copy.deepcopy(replay)]
        with self.assertRaises(ValueError):
            runner.audit_events(events)

    def test_cached_ack_does_not_excuse_second_actual_native_image_frame(self):
        events = fixture()
        accepted = one(events, "message_accepted", phase="image")
        replay = dict(accepted, event="cached_native_ack_replayed", observed_phase="image",
                      receipt_source="CachedNativeProtocol", native_input_added=False)
        events.insert(events.index(one(events, "turn_started", phase="image")), replay)
        native_image = next(e for e in events if e["event"] == "native_protocol_ids" and e["native"]["type"] == "user")
        extra = copy.deepcopy(native_image)
        extra["sequence"] = len([e for e in events if e["event"] == "native_protocol_ids"]) + 1
        events.append(extra)
        with self.assertRaises(ValueError):
            runner.audit_events(events)

    def test_each_native_input_requires_its_own_assistant_before_result(self):
        events = fixture()
        position = next(i for i, e in enumerate(events) if e["event"] == "native_protocol_ids"
                        and e["native"]["type"] == "assistant")
        events.pop(position)
        for sequence, event in enumerate([e for e in events if e["event"] == "native_protocol_ids"], 1):
            event["sequence"] = sequence
        with self.assertRaises(ValueError):
            runner.audit_events(events)

    def test_runner_reuses_auth_chain_restores_hooks_and_keeps_stdout_hash_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(max_native_inputs=3, timeout_seconds=900, output=Path(tmp) / "probe.ndjson")
            original = runner.adapter.authenticated_environment
            def isolated_stub(received):
                events, output = complete_proof()
                received.output.write_text("".join(json.dumps(e) + "\n" for e in events))
                passed = runner.image_probe.verified_acceptance(0, output, events)
                received.output.with_suffix(".metadata.json").write_text(json.dumps({
                    "acceptance_passed": passed, "production_image_gate_open": False,
                    "native_image_input_verified": False, "persistence_verified": False,
                }))
                received.output.with_suffix(".test-output.txt").write_text(output + "PRIVATE_STDOUT_CANARY")
                return 0 if passed else 1
            with patch.object(runner.image_probe, "run", side_effect=isolated_stub):
                self.assertEqual(runner.run(args), 0)
            self.assertIs(runner.adapter.authenticated_environment, original)
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            self.assertTrue(metadata["managed_image_adapter_verified"])
            for inherited in ("production_image_gate_open", "native_image_input_verified", "persistence_verified"):
                self.assertNotIn(inherited, metadata)
            transcript = args.output.with_suffix(".test-output.txt").read_text()
            self.assertNotIn("PRIVATE_STDOUT_CANARY", transcript)
            self.assertNotIn(runner.MARKER, transcript)
            self.assertFalse(json.loads(transcript)["raw_stdout_archived"])

    def test_invalid_budgets_and_delegate_failure_restore_all_hooks(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(max_native_inputs=4, timeout_seconds=900, output=Path(tmp) / "probe.ndjson")
            with self.assertRaises(ValueError):
                runner.run(args)
            args.max_native_inputs = 3
            original = runner.adapter.authenticated_environment
            with patch.object(runner.image_probe, "run", side_effect=ValueError("offline failure")):
                with self.assertRaises(ValueError):
                    runner.run(args)
            self.assertIs(runner.adapter.authenticated_environment, original)

    def test_failed_acceptance_keeps_partial_safe_native_proof_without_claiming_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(max_native_inputs=3, timeout_seconds=900, output=Path(tmp) / "probe.ndjson")
            def failed_native(received):
                complete, output = complete_proof()
                partial = complete[:4]
                trace = output.splitlines()[0] + "\nPRIVATE_FAILURE_CANARY\n"
                received.output.write_text("".join(json.dumps(e) + "\n" for e in partial))
                passed = runner.image_probe.verified_acceptance(101, trace, partial)
                self.assertFalse(passed)
                received.output.with_suffix(".metadata.json").write_text(json.dumps({"acceptance_passed": passed}))
                received.output.with_suffix(".test-output.txt").write_text(trace)
                return 1
            with patch.object(runner.image_probe, "run", side_effect=failed_native):
                self.assertEqual(runner.run(args), 1)
            saved = [json.loads(line) for line in args.output.read_text().splitlines()]
            native_events = [e for e in saved if e["event"] == "native_protocol_ids"]
            self.assertEqual(len(native_events), 1)
            self.assertEqual(native_events[0]["generation"], GENERATIONS[0])
            self.assertEqual(native_events[0]["native"]["type"], "control_response")
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            self.assertFalse(metadata["acceptance_passed"])
            self.assertFalse(metadata["managed_image_adapter_verified"])
            self.assertNotIn("PRIVATE_FAILURE_CANARY", args.output.with_suffix(".test-output.txt").read_text())

    def test_failed_native_trace_with_unknown_body_field_is_never_saved(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(max_native_inputs=3, timeout_seconds=900, output=Path(tmp) / "probe.ndjson")
            def invalid_native(received):
                complete, output = complete_proof()
                partial = complete[:4]
                row = json.loads(output.splitlines()[0][len(runner.MARKER):])
                row["image_body"] = "PRIVATE_IMAGE_BODY_CANARY"
                trace = runner.MARKER + json.dumps(row) + "\n"
                received.output.write_text("".join(json.dumps(e) + "\n" for e in partial))
                passed = runner.image_probe.verified_acceptance(101, trace, partial)
                self.assertFalse(passed)
                received.output.with_suffix(".metadata.json").write_text(json.dumps({"acceptance_passed": passed}))
                received.output.with_suffix(".test-output.txt").write_text(trace)
                return 1
            with patch.object(runner.image_probe, "run", side_effect=invalid_native):
                self.assertEqual(runner.run(args), 1)
            self.assertNotIn("PRIVATE_IMAGE_BODY_CANARY", args.output.read_text())
            self.assertNotIn("native_protocol_ids", args.output.read_text())
            self.assertNotIn("PRIVATE_IMAGE_BODY_CANARY", args.output.with_suffix(".test-output.txt").read_text())

    def test_failed_delegate_stdout_is_compacted_and_existing_output_is_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(max_native_inputs=3, timeout_seconds=900, output=Path(tmp) / "probe.ndjson")
            transcript = args.output.with_suffix(".test-output.txt")
            def failed_delegate(received):
                transcript.write_text("PRIVATE_FAILURE_CANARY")
                raise ValueError("offline failure")
            with patch.object(runner.image_probe, "run", side_effect=failed_delegate):
                with self.assertRaises(ValueError):
                    runner.run(args)
            self.assertNotIn("PRIVATE_FAILURE_CANARY", transcript.read_text())
            transcript.write_text("EXISTING_ARTIFACT")
            with patch.object(runner.image_probe, "run", side_effect=ValueError("existing output rejected")):
                with self.assertRaises(ValueError):
                    runner.run(args)
            self.assertEqual(transcript.read_text(), "EXISTING_ARTIFACT")


if __name__ == "__main__":
    unittest.main()
