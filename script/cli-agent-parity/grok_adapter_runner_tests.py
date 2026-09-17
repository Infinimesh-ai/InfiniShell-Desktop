#!/usr/bin/env python3
"""验证 Grok 运行器的凭据、预算与证据边界；不调用真实 CLI 或模型。"""

import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from run_grok_adapter_live import (
    MessagesProxy, MODEL, RECEIPT_SOURCE, SCOPE, load_credentials, native_environment,
    prepare_native, redacted_response_chunks, sanitizer, verified_acceptance,
)


def final_response_fields(event, response, sequence):
    body = event["output"].encode("utf-8")
    reply = response.encode("utf-8")
    event.update({"final_response": response, "receipt_source": RECEIPT_SOURCE,
        "history_verified": True, "history_native_session_id": event["native_session_id"],
        "history_native_turn_id": event["turn_id"],
        "history_completion_watermark": event["native_session_id"] + f"-{sequence}",
        "final_response_stream_start_ms": 1000 + sequence if response else None,
        "final_response_bytes": len(reply), "final_response_sha256": hashlib.sha256(reply).hexdigest(),
        "full_output_bytes": len(body), "full_output_sha256": hashlib.sha256(body).hexdigest(),
        "output_truncated": False, "product_full_output_preserved": True})
    return event


def acceptance_fixture():
    events = [{"event": "acceptance_passed", "scope": SCOPE, "native_session_id": "native-session",
        "queued_input_verified": True, "same_turn_steering_supported": False,
        "public_product_gate_open": False, "app_restart_and_ui_verified": False,
        "parent_permission_ceiling_verified": False, "official_grok_model_tested": False}]
    turns = [("first_turn", "Completed", "PARITY_ONE"), ("second_turn", "Completed", "PARITY_TWO"),
        ("approval_allow", "Completed", "APPROVED"), ("approval_deny", "Cancelled", ""),
        ("queued_input", "Completed", "QUEUE_PARENT_DONE"), ("queued_input", "Completed", "random-marker"),
        ("cancel", "Cancelled", "READY"), ("resume_result", "Completed", "random-marker")]
    for index, (phase, outcome, output) in enumerate(turns):
        turn = f"turn-{index}"
        events.extend([final_response_fields({"event": "turn_finished", "phase": phase, "turn_id": turn,
            "outcome": outcome, "output": output, "native_session_id": "native-session"},output,index+1),
            {"event": "turn_started", "turn_id": turn},
            {"event": "message_accepted", "message_id": f"local-{index}", "turn_id": turn, "native_receipt": True}])
    for phase, decision, allowed in (("approval_allow", "AllowOnce", True), ("approval_deny", "DenyOnce", False)):
        events.extend([{"event": "approval_requested", "phase": phase, "decision": decision, "exact_write_fixture": True},
            {"event": "file_effect_verified", "phase": phase, "allowed": allowed}])
    events.extend([{"event": "queued_input_result_verified", "marker": "random-marker", "native_acknowledgement_verified": True},
        {"event": "queued_input_submitted", "submitted_after_real_text": True},
        {"event": "cancel_submitted", "submitted_after_real_text": True},
        {"event": "connection_shutdown", "cleanup_confirmed": True, "native_session_id": "native-session", "queued_submissions_observed_inside_adapter": 1},
        {"event": "connection_shutdown", "cleanup_confirmed": True, "native_session_id": "native-session", "queued_submissions_observed_inside_adapter": 0}])
    return events


class RunnerBoundaryTests(unittest.TestCase):
    def test_old_internal_command_scope_cannot_claim_production_command_verification(self):
        events = acceptance_fixture()
        events[0]["scope"] = "production_process_transport_with_test_only_acp_commands"
        self.assertFalse(verified_acceptance(0, "test result: ok. 1 passed; 0 failed; 0 ignored;", events))

    def test_approval_receipt_uses_verified_last_response_and_keeps_product_prelude(self):
        events = acceptance_fixture()
        result = next(event for event in events if event.get("event") == "turn_finished" and event.get("phase") == "approval_allow")
        result["output"] = "请批准这次写入。\nAPPROVED"
        final_response_fields(result,"APPROVED",3)
        self.assertTrue(verified_acceptance(0,"test result: ok. 1 passed; 0 failed; 0 ignored;",events))
        self.assertEqual(result["output"],"请批准这次写入。\nAPPROVED")

    def test_suffix_or_substring_cannot_replace_final_response_receipt(self):
        for response in ("请批准这次写入。\nAPPROVED", "NOT_APPROVED", "APPROVED is only mentioned"):
            with self.subTest(response=response):
                events = acceptance_fixture()
                result = next(event for event in events if event.get("event") == "turn_finished" and event.get("phase") == "approval_allow")
                result["output"] = "APPROVED from earlier stream\n" + response
                final_response_fields(result,response,3)
                self.assertFalse(verified_acceptance(0,"test result: ok. 1 passed; 0 failed; 0 ignored;",events))

    def test_final_response_requires_native_tuple_watermark_and_full_output_preservation(self):
        changes = [("history_native_session_id","other-session"),("history_native_turn_id","old-turn"),
            ("history_completion_watermark",None),("history_completion_watermark","native-session-03"),
            ("history_completion_watermark","native-session-18446744073709551616"),
            ("history_verified",False),("receipt_source","text_suffix"),("product_full_output_preserved",False),
            ("final_response_stream_start_ms",None),("final_response_stream_start_ms",True),
            ("final_response_sha256","0" * 64),("final_response_bytes",True),("full_output_sha256","0" * 64)]
        for key,value in changes:
            with self.subTest(key=key,value=value):
                events = acceptance_fixture()
                result = next(event for event in events if event.get("event") == "turn_finished" and event.get("phase") == "approval_allow")
                result[key] = value
                self.assertFalse(verified_acceptance(0,"test result: ok. 1 passed; 0 failed; 0 ignored;",events))

    def test_truncated_evidence_cannot_claim_the_product_output_was_replaced(self):
        events = acceptance_fixture()
        result = next(event for event in events if event.get("event") == "turn_finished" and event.get("phase") == "approval_allow")
        result["output"] = "x" * 5000 + "APPROVED"
        final_response_fields(result,"APPROVED",3)
        result["output"] = result["output"][:4096]
        result["output_truncated"] = True
        self.assertTrue(verified_acceptance(0,"test result: ok. 1 passed; 0 failed; 0 ignored;",events))
        result["output"] = "APPROVED"
        self.assertFalse(verified_acceptance(0,"test result: ok. 1 passed; 0 failed; 0 ignored;",events))

    def test_official_projection_keeps_receipt_proof_and_omits_unfixed_body(self):
        from run_grok_official_adapter_live import public_events
        event = next(event for event in acceptance_fixture() if event.get("phase") == "approval_allow")
        event["native_session_id"] = "00000000-0000-0000-0000-000000000001"
        event["output"] = "private pre-tool narrative\nAPPROVED"
        final_response_fields(event,"APPROVED",3)
        projected = public_events([event])[0]
        self.assertEqual(projected["output"],"<省略非固定文本>")
        self.assertEqual(projected["final_response"],"APPROVED")
        for key in ("receipt_source","history_completion_watermark","full_output_sha256","final_response_sha256"):
            self.assertEqual(projected[key],event[key])

    def test_no_pass_without_one_executed_test_and_full_native_evidence(self):
        output = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        self.assertTrue(verified_acceptance(0, output, acceptance_fixture()))
        self.assertFalse(verified_acceptance(0, "test result: ok. 0 passed; 0 failed; 0 ignored;", acceptance_fixture()))
        self.assertFalse(verified_acceptance(0, output, []))
        events = acceptance_fixture()
        events[-2]["queued_submissions_observed_inside_adapter"] = 0
        self.assertFalse(verified_acceptance(0, output, events))
        events = acceptance_fixture()
        events[0]["public_product_gate_open"] = True
        self.assertFalse(verified_acceptance(0, output, events))

    def test_local_dispatch_cannot_replace_native_acceptance_or_terminal(self):
        output = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        events = acceptance_fixture()
        for event in events:
            if event.get("event") == "message_accepted":
                event["event"] = "command_dispatched"
        self.assertFalse(verified_acceptance(0, output, events))
        events = [event for event in acceptance_fixture() if not (event.get("event") == "turn_finished" and event.get("phase") == "cancel")]
        self.assertFalse(verified_acceptance(0, output, events))

    def test_real_service_credentials_never_enter_child_environment(self):
        with patch.dict(os.environ, {"XAI_API_KEY": "not-a-real-key", "ANTHROPIC_API_KEY": "not-a-real-key", "ANTHROPIC_BASE_URL": "https://invalid.example"}):
            environment = native_environment(Path("/tmp/private-probe"), "fake-capability")
        self.assertFalse({"XAI_API_KEY", "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL"} & environment.keys())
        self.assertEqual(environment["INFINISHELL_GROK_BYOK_KEY"], "fake-capability")

    def test_proxy_rejects_foreign_models_missing_marker_and_unbounded_output(self):
        body = {"model": MODEL, "max_tokens": 2048, "messages": [{"content": "INFINISHELL_GROK_ADAPTER"}]}
        self.assertTrue(MessagesProxy.allowed_body(body, json.dumps(body).encode()))
        for field, value in (("model", "other-model"), ("max_tokens", 2049), ("max_tokens", True)):
            invalid = copy.deepcopy(body); invalid[field] = value
            self.assertFalse(MessagesProxy.allowed_body(invalid, json.dumps(invalid).encode()))
        self.assertFalse(MessagesProxy.allowed_body(body, b"unrelated request"))

    def test_credentials_require_explicit_https_origin_and_are_redacted(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fake.json"
            credentials = {"ANTHROPIC_API_KEY": 'dummy-quote-"-key', "ANTHROPIC_BASE_URL": "https://invalid.example/v1"}
            path.write_text(json.dumps(credentials))
            loaded, target = load_credentials(path)
            self.assertEqual(target.hostname, "invalid.example")
            clean = sanitizer(loaded, Path(directory), "fake-capability")
            encoded = clean(json.dumps({"error": credentials["ANTHROPIC_API_KEY"] + " invalid.example fake-capability"}))
            self.assertNotIn("invalid.example", encoded)
            self.assertNotIn("dummy-quote", encoded)
            self.assertNotIn("fake-capability", encoded)
            json.loads(encoded)
            credentials["ANTHROPIC_BASE_URL"] = "https://invalid.example/another/path"
            path.write_text(json.dumps(credentials))
            with self.assertRaises(ValueError):
                load_credentials(path)

    def test_upstream_secret_echo_is_redacted_across_network_chunks(self):
        class Response:
            def __init__(self, chunks):
                self.chunks = iter(chunks)
            def read1(self, size):
                return next(self.chunks, b"")
        response = Response([b"prefix dummy-", b"private-key suffix"])
        encoded = b"".join(redacted_response_chunks(response, ["dummy-private-key"]))
        self.assertEqual(encoded, b"prefix <redacted> suffix")

    def test_generated_wrapper_is_valid_python_and_keeps_leader_arguments(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / "home/.grok").mkdir(parents=True)
            wrapper, settings = prepare_native(root, Path("/tmp/fixed-grok"), Path("/tmp/private-credential-dir"), 12345)
            source = wrapper.read_text()
            compile(source, str(wrapper), "exec")
            self.assertIn("'agent','stdio','--leader-socket'", source)
            self.assertNotIn("--no-leader", source)
            self.assertIn("use_leader = true", settings.read_text())
            self.assertNotIn("ANTHROPIC_API_KEY", source)


if __name__ == "__main__":
    unittest.main()
