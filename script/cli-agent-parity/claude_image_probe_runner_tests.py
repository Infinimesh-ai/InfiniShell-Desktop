"""图片校准运行器的离线回归；合成证据不计为原生图片验证。"""

import copy
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import uuid

import run_claude_image_probe as runner


def identity(number):
    return str(uuid.UUID(int=number))


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


def native(**fields):
    row = {key: None for key in runner.NATIVE_KEYS}
    row.update({"tools": [], "parent_tool_use_present": False, "assistant_error_present": False, "content": {"shape": "missing"}})
    row.update(fields)
    return row


def complete_events():
    generation, session, user, assistant, result = [identity(number) for number in range(1, 6)]
    request = f"infinishell-{generation}-1"
    prompt_sha, image_sha, answer_sha = digest("synthetic prompt"), digest("synthetic PNG"), digest("RED GREEN BLUE YELLOW")
    content = {"shape": "array", "array_sha256": digest("synthetic array"), "blocks": [
        {"type": "text", "text_sha256": prompt_sha, "text_bytes": 16},
        {"type": "image", "source_type": "base64", "media_type": "image/png", "image_sha256": image_sha, "image_bytes": 12420},
    ]}
    rows = [
        native(direction="stdin", type="control_request", request_id=request, request_subtype="initialize"),
        native(direction="stdout", type="control_response", response_request_id=request, response_subtype="success", initialize_session_state="idle", permission_mode="default"),
        native(direction="stdin", type="user", uuid=user, message_role="user", session_id="sha256:" + runner.EMPTY_SHA, content=copy.deepcopy(content)),
        native(direction="stdout", type="command_lifecycle", command_uuid=user, state="queued", session_id=session),
        native(direction="stdout", type="command_lifecycle", command_uuid=user, state="started", session_id=session),
        native(direction="stdout", type="system", subtype="init", session_id=session, native_version="2.1.273", system_tool_count=0, system_mcp_count=0, permission_mode="default"),
        native(direction="stdout", type="user", uuid=user, message_role="user", session_id=session, is_replay=True, content=copy.deepcopy(content)),
        native(direction="stdout", type="assistant", uuid=assistant, message_role="assistant", user_message_uuid=user, session_id=session,
               content={"shape": "array", "array_sha256": None, "blocks": [
                   {"type": "text", "text_sha256": answer_sha, "text_bytes": 21}]},
               assistant_text_sha256=answer_sha, assistant_trimmed_sha256=answer_sha),
        native(direction="stdout", type="result", uuid=result, user_message_uuid=user, user_message_uuids=[user], session_id=session,
               subtype="success", is_error=False, result_text_sha256=answer_sha, result_trimmed_sha256=answer_sha),
    ]
    projections = [{"event": "native_protocol_ids", "sequence": number, "native": row} for number, row in enumerate(rows, 1)]
    return [
        {"event": "acceptance_started", "scope": runner.SCOPE, "generation": generation, "max_native_inputs": 1,
         "native_framing_only": True, "production_image_gate_open": False, "credential_files_read_by_probe": False,
         "real_gui_verified": False, "persistence_verified": False, "http_request_count_verified": False},
        projections[0],
        {"event": "png_generated", "image_sha256": image_sha, "image_bytes": 12420, "width": 64, "height": 64, "quadrants": 4,
         "expected_reply_sha256": answer_sha, "prompt_sha256": prompt_sha},
        projections[1], projections[2],
        {"event": "input_submitted", "message_id": user, "submitted_input_count": 1, "array_content": True, "transport_write_confirmed": True},
        *projections[3:],
        {"event": "shutdown_output_drained", "bytes": 0, "sha256": runner.EMPTY_SHA,
         "frames": 0, "native": [], "verified": True},
        {"event": "cleanup_checked", "generation": generation, "cleanup_confirmed": True, "transport_closed": True,
         "cleanup_receipt_read": True, "receipt": {"generation": generation, "containment": "macos_resource_coalition",
             "cleanup_confirmed": True, "exit_code": 0, "exit_reason": "stdio_closed"}},
        {"event": "native_image_proof", "native_session_id": session, "message_id": user, "native_result_uuid": result,
         "user_message_uuids": [user], "native_inputs": 1, "replay_array_verified": True, "native_ack_verified": True,
         "native_started_verified": True, "native_no_tools_verified": True, "assistant_origin_verified": True,
         "assistant_frames": 1, "assistant_output_sha256": answer_sha, "assistant_trimmed_sha256": answer_sha,
         "result_trimmed_sha256": answer_sha, "image_pixels_verified": True},
        {"event": "acceptance_passed", "scope": runner.SCOPE, "native_session_id": session, "message_id": user, "native_inputs": 1,
         "native_framing_only": True, "production_image_gate_open": False, "cleanup_confirmed": True, "transport_closed": True,
         "credential_files_read_by_probe": False, "persistence_verified": False, "app_restart_and_ui_verified": False,
         "parent_permission_ceiling_verified": False, "http_request_count_verified": False},
    ]


def event_of(events, kind):
    return next(event for event in events if event["event"] == kind)


def row_of(events, **filters):
    return next(event["native"] for event in events if event["event"] == "native_protocol_ids"
                and all(event["native"].get(key) == value for key, value in filters.items()))


def reindex(events):
    for number, event in enumerate((event for event in events if event["event"] == "native_protocol_ids"), 1):
        event["sequence"] = number


class ImageProbeRunnerTests(unittest.TestCase):
    output = "test result: ok. 1 passed; 0 failed; 0 ignored; 7400 filtered out"

    def accepts(self, events):
        return runner.verified_acceptance(0, self.output, events)

    def test_complete_native_ledger_is_required(self):
        events = complete_events()
        self.assertTrue(self.accepts(events))
        self.assertFalse(self.accepts([events[-1]]))
        self.assertFalse(runner.verified_acceptance(1, self.output, events))
        self.assertFalse(runner.verified_acceptance(0, "test result: ok. 0 passed; 0 failed; 1 ignored;", events))
        self.assertFalse(self.accepts(events + events))

    def test_every_required_record_is_necessary(self):
        for kind in runner.EVENT_KEYS:
            if kind != "native_protocol_ids":
                events = [event for event in complete_events() if event["event"] != kind]
                self.assertFalse(self.accepts(events), kind)

    def test_each_native_proof_is_necessary(self):
        for kind in ("control_request", "control_response", "user", "system", "assistant", "result"):
            events = [event for event in complete_events() if not (event["event"] == "native_protocol_ids" and event["native"]["type"] == kind)]
            reindex(events)
            self.assertFalse(self.accepts(events), kind)

    def test_started_is_native_ack_without_optional_queued_frame(self):
        events = [event for event in complete_events() if not (event["event"] == "native_protocol_ids" and event["native"]["state"] == "queued")]
        reindex(events)
        self.assertTrue(self.accepts(events))
        events = [event for event in events if not (event["event"] == "native_protocol_ids" and event["native"]["state"] == "started")]
        reindex(events)
        self.assertFalse(self.accepts(events))

    def test_shutdown_output_can_only_finish_the_same_completed_input(self):
        events = complete_events()
        shutdown = event_of(events, "shutdown_output_drained")
        row = native(direction="stdout", type="command_lifecycle", state="completed",
                     session_id=identity(2), command_uuid=identity(3))
        shutdown.update(bytes=200, sha256=digest("synthetic final lifecycle"), frames=1, native=[row])
        self.assertTrue(self.accepts(events))
        for key, value in (("session_id", identity(99)), ("command_uuid", identity(99)),
                           ("state", "cancelled"), ("type", "result"), ("tools", ["Write"]),
                           ("parent_tool_use_present", True), ("assistant_error_present", True),
                           ("content", {"shape": "string", "text_sha256": "a" * 64, "text_bytes": 4})):
            mutated = copy.deepcopy(events)
            event_of(mutated, "shutdown_output_drained")["native"][0][key] = value
            self.assertFalse(self.accepts(mutated), key)

    def test_shutdown_output_requires_bounded_consistent_evidence(self):
        for key, value in (("bytes", True), ("bytes", -1), ("bytes", 8 * 1024 * 1024 + 1),
                           ("frames", 2), ("frames", 1), ("native", [native()]),
                           ("verified", False), ("sha256", digest("hidden extra output"))):
            events = complete_events()
            event_of(events, "shutdown_output_drained")[key] = value
            self.assertFalse(self.accepts(events), key)
        events = complete_events()
        shutdown = event_of(events, "shutdown_output_drained")
        events.remove(shutdown)
        events.insert(events.index(event_of(events, "cleanup_checked")) + 1, shutdown)
        self.assertFalse(self.accepts(events))

    def test_array_replay_cannot_be_substituted_by_string_summary(self):
        events = complete_events()
        row_of(events, direction="stdout", type="user")["content"] = {"shape": "string", "text_sha256": "a" * 64, "text_bytes": 10}
        self.assertFalse(self.accepts(events))

    def test_actual_array_image_and_text_identity_are_checked(self):
        mutations = (("array_sha256", "b" * 64), ("text_sha256", "c" * 64), ("image_sha256", "d" * 64),
                     ("image_bytes", 2), ("media_type", "image/jpeg"), ("source_type", "url"))
        for key, value in mutations:
            events = complete_events()
            content = row_of(events, direction="stdout", type="user")["content"]
            target = content if key == "array_sha256" else content["blocks"][0 if key == "text_sha256" else 1]
            target[key] = value
            self.assertFalse(self.accepts(events), key)

    def test_generated_input_hashes_must_match_even_if_replay_matches(self):
        for key in ("image_sha256", "image_bytes", "prompt_sha256", "expected_reply_sha256"):
            events = complete_events()
            event_of(events, "png_generated")[key] = 3 if key == "image_bytes" else "d" * 64
            self.assertFalse(self.accepts(events), key)

    def test_native_uuid_session_and_single_result_batch_are_checked(self):
        mutations = (("assistant", "user_message_uuid", identity(99)), ("result", "user_message_uuids", [identity(3), identity(99)]),
                     ("result", "user_message_uuids", None), ("result", "user_message_uuid", identity(99)),
                     ("result", "session_id", identity(99)), ("assistant", "session_id", None),
                     ("assistant", "message_role", "user"), ("user", "message_role", None),
                     ("user", "is_replay", False), ("result", "uuid", identity(0)))
        for kind, key, value in mutations:
            events = complete_events()
            row_of(events, direction="stdout", type=kind)[key] = value
            self.assertFalse(self.accepts(events), (kind, key))

    def test_no_tool_or_permission_request_is_allowed(self):
        for mutation in ("assistant_tool", "system_tool", "system_mcp", "native_permission"):
            events = complete_events()
            if mutation == "assistant_tool":
                row_of(events, type="assistant")["tools"] = [{"name": "Read", "id": "toolu_synthetic"}]
            elif mutation == "system_tool":
                row_of(events, type="system")["system_tool_count"] = 1
            elif mutation == "system_mcp":
                row_of(events, type="system")["system_mcp_count"] = 1
            else:
                row_of(events, type="control_request")["direction"] = "stdout"
                row_of(events, type="control_request")["request_subtype"] = "can_use_tool"
            self.assertFalse(self.accepts(events), mutation)
        self.assertFalse(self.accepts(complete_events() + [{"event": "tool_request_rejected"}]))

    def test_failed_cancelled_or_wrong_color_response_never_passes(self):
        for key, value in (("subtype", "error_during_execution"), ("is_error", True), ("terminal_reason", "cancelled"),
                           ("result_trimmed_sha256", digest("YELLOW BLUE GREEN RED"))):
            events = complete_events()
            row_of(events, type="result")[key] = value
            self.assertFalse(self.accepts(events), key)

    def test_raw_payload_and_thinking_text_are_never_allowed_in_public_projection(self):
        for target in ("native", "image", "thinking", "outer"):
            events = complete_events()
            if target == "native":
                row_of(events, type="result")["result"] = "arbitrary body"
            elif target == "image":
                row_of(events, direction="stdin", type="user")["content"]["blocks"][1]["data"] = "arbitrary base64"
            elif target == "thinking":
                row_of(events, type="assistant")["content"]["blocks"].append({"type": "thinking", "text": "private reasoning"})
            else:
                event_of(events, "acceptance_passed")["api_environment"] = "arbitrary value"
            self.assertFalse(self.accepts(events), target)

    def test_thinking_block_type_only_is_permitted(self):
        events = complete_events()
        row_of(events, type="assistant")["content"]["blocks"].insert(0, {"type": "thinking"})
        self.assertTrue(self.accepts(events))

    def test_duplicate_native_input_assistant_result_and_old_initialization_fail(self):
        for kind in ("user", "assistant", "result", "control_response"):
            events = complete_events()
            index = next(index for index, event in enumerate(events) if event["event"] == "native_protocol_ids" and event["native"]["type"] == kind)
            events.insert(index + 1, copy.deepcopy(events[index]))
            reindex(events)
            self.assertFalse(self.accepts(events), kind)
        events = complete_events()
        row_of(events, type="control_response")["response_request_id"] = f"infinishell-{identity(99)}-1"
        self.assertFalse(self.accepts(events))

    def test_result_before_replay_or_started_fails(self):
        for kind in ("user", "command_lifecycle"):
            events = complete_events()
            result_index = next(index for index, event in enumerate(events) if event["event"] == "native_protocol_ids" and event["native"]["type"] == "result")
            index = next(index for index, event in enumerate(events) if event["event"] == "native_protocol_ids" and event["native"]["direction"] == "stdout" and event["native"]["type"] == kind)
            events[index], events[result_index] = events[result_index], events[index]
            reindex(events)
            self.assertFalse(self.accepts(events), kind)

    def test_cleanup_is_real_current_generation_and_never_summary_only(self):
        for key, value in (("cleanup_confirmed", False), ("transport_closed", False), ("cleanup_receipt_read", False),
                           ("generation", identity(99)), ("receipt", None)):
            events = complete_events()
            event_of(events, "cleanup_checked")[key] = value
            self.assertFalse(self.accepts(events), key)
        for containment in ("linux_subtree", "windows_job", "unix_process_group"):
            events = complete_events()
            event_of(events, "cleanup_checked")["receipt"]["containment"] = containment
            self.assertTrue(self.accepts(events))

    def test_correct_image_protocol_requires_zero_stdio_closed_exit_receipt(self):
        self.assertTrue(self.accepts(complete_events()))
        for key, value in (("exit_code", 1), ("exit_code", -1), ("exit_code", None), ("exit_code", False),
                           ("exit_reason", "host_disconnected"), ("exit_reason", "native_exit"),
                           ("exit_reason", "stop_requested"), ("cleanup_confirmed", False)):
            events = complete_events()
            event_of(events, "cleanup_checked")["receipt"][key] = value
            # 图片协议结果和外层清理摘要保持正确；异常真实退出回执仍不得通过。
            self.assertTrue(event_of(events, "native_image_proof")["image_pixels_verified"])
            self.assertTrue(event_of(events, "acceptance_passed")["cleanup_confirmed"])
            self.assertFalse(self.accepts(events), (key, value))

    def test_probe_cannot_claim_product_gui_or_persistence_support(self):
        for kind, key in (("acceptance_started", "production_image_gate_open"), ("acceptance_passed", "production_image_gate_open"),
                          ("acceptance_passed", "persistence_verified"), ("acceptance_passed", "app_restart_and_ui_verified"),
                          ("acceptance_started", "http_request_count_verified")):
            events = complete_events()
            event_of(events, kind)[key] = True
            self.assertFalse(self.accepts(events), key)

    def test_native_projection_sequence_and_version_are_checked(self):
        events = complete_events()
        event_of(events, "native_protocol_ids")["sequence"] = True
        self.assertFalse(self.accepts(events))
        events = complete_events()
        row_of(events, type="system")["native_version"] = "unknown"
        self.assertFalse(self.accepts(events))

    def test_budget_counts_are_integers_and_no_extra_stdin_is_accepted(self):
        for kind, key in (("acceptance_started", "max_native_inputs"), ("input_submitted", "submitted_input_count"),
                          ("native_image_proof", "native_inputs"), ("acceptance_passed", "native_inputs")):
            events = complete_events()
            event_of(events, kind)[key] = True
            self.assertFalse(self.accepts(events), (kind, key))
        events = complete_events()
        projection = {"event": "native_protocol_ids", "sequence": 0, "native": native(direction="stdin", type="keep_alive")}
        events.insert(2, projection)
        reindex(events)
        self.assertFalse(self.accepts(events))

    def test_default_permissions_and_assistant_error_flag_are_native_evidence(self):
        for kind, key, value in (("control_response", "permission_mode", "unknown"), ("system", "permission_mode", None),
                                 ("assistant", "assistant_error_present", True)):
            events = complete_events()
            row_of(events, type=kind)[key] = value
            self.assertFalse(self.accepts(events), (kind, key))

    def test_project_is_private_denied_and_no_image_is_written_to_project(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = runner.adapter.PROJECT_SETTINGS
            try:
                runner.adapter.PROJECT_SETTINGS = runner.PROJECT_SETTINGS
                settings = runner.adapter.prepare_project(root)
            finally:
                runner.adapter.PROJECT_SETTINGS = original
            self.assertEqual(json.loads(settings.read_text()), {"permissions": {"deny": ["*"]}})
            self.assertEqual((root / ".infinishell-claude-live-probe").read_text(), runner.adapter.MARKER)
            self.assertEqual([path.relative_to(root / "project").as_posix() for path in (root / "project").rglob("*") if path.is_file()],
                             [".claude/settings.local.json"])

    def test_missing_api_or_model_rejected_without_native_launch(self):
        for api, model in ((None, "fixed-model"), (Path("unused.json"), ""), (Path("unused.json"), "bad\nmodel")):
            with patch.object(runner.adapter, "run") as native_run:
                with self.assertRaises(ValueError):
                    runner.run(SimpleNamespace(api_environment_file=api, model=model))
                native_run.assert_not_called()

    def test_wrapper_isolates_auth_restores_hooks_and_preserves_product_gate(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            api = root / "explicit.json"
            api.write_text("{}")
            api.chmod(0o600)
            args = SimpleNamespace(api_environment_file=api, model="fixed-model", output=root / "proof.ndjson")
            old_test, old_settings, old_verifier = runner.adapter.TEST_NAME, runner.adapter.PROJECT_SETTINGS, runner.adapter.verified_acceptance
            observed = {}
            def simulated_run(value):
                observed.update({"test": runner.adapter.TEST_NAME, "settings": runner.adapter.PROJECT_SETTINGS,
                                 "config": value.config_dir, "home": value.auth_home})
                value.output.write_text("".join(json.dumps(event) + "\n" for event in complete_events()))
                value.output.with_suffix(".metadata.json").write_text(json.dumps({"acceptance_passed": True}))
                return 0
            # mock 阻断路径校验与原生入口；合成 API 文件从未被读取为认证资料。
            with patch.object(runner.adapter, "validate_paths"), patch.object(runner.adapter, "run", side_effect=simulated_run), \
                    patch.object(runner.tempfile, "mkdtemp", return_value=str(root / "private")):
                (root / "private").mkdir()
                self.assertEqual(runner.run(args), 0)
            self.assertEqual(observed["test"], runner.TEST_NAME)
            self.assertEqual(observed["settings"], runner.PROJECT_SETTINGS)
            self.assertNotEqual(observed["config"], observed["home"])
            self.assertEqual((runner.adapter.TEST_NAME, runner.adapter.PROJECT_SETTINGS, runner.adapter.verified_acceptance),
                             (old_test, old_settings, old_verifier))
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            self.assertTrue(metadata["native_image_input_verified"])
            self.assertFalse(metadata["production_image_gate_open"])


if __name__ == "__main__":
    unittest.main()
