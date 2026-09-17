#!/usr/bin/env python3
"""Claude 真实验收运行器的离线边界测试，不调用 CLI 或网络。"""

import argparse
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from run_claude_adapter_live import (
    MARKER, PROJECT_SETTINGS, authenticated_environment, load_api_environment, prepare_project,
    sanitize, sanitize_event, validate_paths, verified_acceptance,
)


SUMMARY = "test result: ok. 1 passed; 0 failed; 0 ignored;"
NATIVE_ID = "00000000-0000-4000-8000-000000000001"


def complete_events():
    events = []
    for index, (phase, output, outcome) in enumerate([
        ("first_turn", "PARITY_ONE", "Completed"), ("second_turn", "PARITY_TWO", "Completed"),
        ("approval_allow", "APPROVED", "Completed"), ("approval_deny", "DENIED", "Completed"),
        ("queued_input", "QUEUE_PARENT_DONE", "Completed"), ("queued_input", "QUEUED_APPLIED_fixture", "Completed"),
        ("cancel", "", "Cancelled"), ("resume_result", "QUEUED_APPLIED_fixture", "Completed"),
    ]):
        turn_id = f"turn-{index}"
        events.extend([
            {"event": "message_accepted", "phase": phase, "turn_id": turn_id, "message_id": turn_id},
            {"event": "turn_started", "phase": phase, "turn_id": turn_id},
            {"event": "turn_finished", "phase": phase, "turn_id": turn_id,
             "outcome": outcome, "output": output, "native_session_id": NATIVE_ID},
        ])
    for phase, allowed, decision in (("approval_allow", True, "AllowOnce"), ("approval_deny", False, "DenyOnce")):
        events.extend([
            {"event": "approval_requested", "phase": phase, "exact_write_fixture": True, "decision": decision},
            {"event": "file_effect_verified", "phase": phase, "allowed": allowed},
        ])
    events.extend([
        {"event": "queued_input_submitted", "submitted_while_running": True, "same_turn_steering_verified": False},
        {"event": "queued_input_result_verified", "marker": "QUEUED_APPLIED_fixture",
         "native_acknowledgement_verified": True, "same_turn_steering_supported": False},
        {"event": "connection_shutdown", "native_session_id": NATIVE_ID},
        {"event": "connection_shutdown", "native_session_id": NATIVE_ID},
        {"event": "acceptance_passed", "scope": "rust_adapter_process_restart", "native_session_id": NATIVE_ID,
         "queued_input_verified": True, "same_turn_steering_supported": False,
         "app_restart_and_ui_verified": False, "parent_permission_ceiling_verified": False},
    ])
    return events


class AcceptanceTests(unittest.TestCase):
    def joined_events(self):
        events = complete_events()
        parent = next(event for event in events if event.get("event") == "turn_finished" and event.get("turn_id") == "turn-4")
        joined = next(event for event in events if event.get("event") == "turn_finished" and event.get("turn_id") == "turn-5")
        parent["output"] = joined["output"]
        events.remove(parent)
        events.insert(events.index(joined) + 1, parent)
        start = next(event for event in events if event.get("event") == "turn_started" and event.get("turn_id") == "turn-5")
        start.update({"event":"input_joined", "message_id":"turn-5", "turn_id":"turn-4", "native_session_id":NATIVE_ID})
        return events

    def test_joined_input_requires_the_real_execution_and_ordered_batch_result(self):
        events = self.joined_events()
        self.assertTrue(verified_acceptance(0, SUMMARY, events))
        for key, value in (("turn_id", "turn-3"), ("native_session_id", "other"), ("message_id", "unknown")):
            changed = copy.deepcopy(events)
            next(event for event in changed if event.get("event") == "input_joined")[key] = value
            with self.subTest(key=key):
                self.assertFalse(verified_acceptance(0, SUMMARY, changed))
        changed = self.joined_events()
        parent = next(event for event in changed if event.get("event") == "turn_finished" and event.get("turn_id") == "turn-4")
        parent["output"] = "different-result"
        self.assertFalse(verified_acceptance(0, SUMMARY, changed))
        changed = self.joined_events()
        joined = next(event for event in changed if event.get("event") == "input_joined")
        changed.remove(joined)
        changed.append(joined)
        self.assertFalse(verified_acceptance(0, SUMMARY, changed))

    def test_duplicate_or_unknown_join_cannot_replace_a_native_start(self):
        events = self.joined_events()
        joined = next(event for event in events if event.get("event") == "input_joined")
        self.assertFalse(verified_acceptance(0, SUMMARY, events + [joined.copy()]))
        self.assertFalse(verified_acceptance(0, SUMMARY, complete_events() + [joined.copy()]))

    def test_requires_one_matching_test_and_full_native_evidence(self):
        events = complete_events()
        self.assertTrue(verified_acceptance(0, SUMMARY, events))
        self.assertFalse(verified_acceptance(1, SUMMARY, events))
        self.assertFalse(verified_acceptance(0, "test result: ok. 0 passed; 0 failed; 0 ignored;", events))
        self.assertFalse(verified_acceptance(0, SUMMARY, []))
        self.assertFalse(verified_acceptance(0, SUMMARY, [events[-1]]))
        self.assertFalse(verified_acceptance(0, SUMMARY, events + [{"event": "acceptance_failed"}]))

    def test_file_effects_and_real_cancel_cannot_be_replaced_by_local_ack(self):
        events = complete_events()
        for name in ("file_effect_verified", "message_accepted", "turn_started", "queued_input_submitted", "connection_shutdown"):
            with self.subTest(name=name):
                self.assertFalse(verified_acceptance(0, SUMMARY, [event for event in events if event["event"] != name]))
        changed = copy.deepcopy(events)
        next(event for event in changed if event.get("phase") == "cancel" and event["event"] == "turn_finished")["outcome"] = "Completed"
        self.assertFalse(verified_acceptance(0, SUMMARY, changed))

    def test_queue_and_resume_cannot_claim_same_turn_steering_or_new_identity(self):
        for field in ("same_turn_steering_supported", "app_restart_and_ui_verified", "parent_permission_ceiling_verified"):
            events = complete_events()
            events[-1][field] = True
            with self.subTest(field=field):
                self.assertFalse(verified_acceptance(0, SUMMARY, events))
        events = complete_events()
        next(event for event in events if event.get("phase") == "resume_result" and event["event"] == "turn_finished")["native_session_id"] = "different-session"
        self.assertFalse(verified_acceptance(0, SUMMARY, events))
        events = complete_events()
        next(event for event in events if event.get("phase") == "resume_result" and event["event"] == "turn_finished")["output"] = "PARITY_ONE"
        self.assertFalse(verified_acceptance(0, SUMMARY, events))

    def test_duplicate_completion_and_nonfixture_approval_fail(self):
        events = complete_events()
        self.assertFalse(verified_acceptance(0, SUMMARY, events + [events[2]]))
        events = complete_events()
        next(event for event in events if event["event"] == "approval_requested")["exact_write_fixture"] = False
        self.assertFalse(verified_acceptance(0, SUMMARY, events))


class EnvironmentTests(unittest.TestCase):
    def test_reuses_auth_paths_without_touching_settings_or_reading_native_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            config = root / "existing-config"
            home = root / "existing-home"
            config.mkdir()
            home.mkdir()
            settings = config / "settings.json"
            settings.write_text('{"existing":"preserve"}\n', encoding="utf-8")
            inherited = {"PATH": "test-path", "SystemRoot": "test-system", "ANTHROPIC_API_KEY": "inherited-secret",
                         "ANTHROPIC_AUTH_TOKEN": "inherited-token", "ANTHROPIC_BASE_URL": "https://inherited.invalid",
                         "HTTPS_PROXY": "inherited-proxy", "DYLD_INSERT_LIBRARIES": "inherited-library"}
            with mock.patch.dict(os.environ, inherited, clear=True), \
                    mock.patch.object(Path, "read_bytes", side_effect=AssertionError("不能读取认证文件")), \
                    mock.patch.object(Path, "read_text", side_effect=AssertionError("不能读取认证文件")):
                environment = authenticated_environment(root / "probe", config, home)
            self.assertEqual(environment["CLAUDE_CONFIG_DIR"], str(config))
            self.assertEqual(environment["HOME"], str(home))
            self.assertEqual(environment["USERPROFILE"], str(home))
            self.assertEqual(environment["PATH"], "test-path")
            for key in ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "HTTPS_PROXY", "DYLD_INSERT_LIBRARIES"):
                self.assertNotIn(key, environment)
            self.assertEqual(settings.read_text(encoding="utf-8"), '{"existing":"preserve"}\n')

    def test_project_only_requests_write_approval_and_refuses_reinitialization(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            settings = prepare_project(root)
            self.assertEqual(json.loads(settings.read_text(encoding="utf-8")), PROJECT_SETTINGS)
            self.assertEqual(PROJECT_SETTINGS, {"permissions": {"defaultMode": "default", "ask": ["Write"]}})
            self.assertEqual((root / ".infinishell-claude-live-probe").read_text(encoding="utf-8"), MARKER)
            with self.assertRaises(FileExistsError):
                prepare_project(root)

    def test_api_file_only_loads_explicit_whitelist_and_one_authentication_method(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "api.json"
            safe = {"ANTHROPIC_API_KEY": "fake-unit-test-key", "ANTHROPIC_BASE_URL": "https://example.invalid",
                    "ANTHROPIC_MODEL": "fixture-model"}
            path.write_text(json.dumps(safe), encoding="utf-8")
            self.assertEqual(load_api_environment(path), safe)
            self.assertEqual(load_api_environment(None), {})
            for value in (safe | {"DYLD_INSERT_LIBRARIES": "extra"}, safe | {"ANTHROPIC_AUTH_TOKEN": "other"},
                          {"ANTHROPIC_MODEL": "only-model"}, safe | {"ANTHROPIC_API_KEY": "line\nbreak"},
                          safe | {"ANTHROPIC_API_KEY": False}):
                with self.subTest(value=list(value)):
                    path.write_text(json.dumps(value), encoding="utf-8")
                    with self.assertRaises(ValueError):
                        load_api_environment(path)

    def test_api_and_path_redaction_preserves_json_and_never_hashes_secrets(self):
        secret = 'fake-secret-"quote"-\\slash'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            redact = lambda text: sanitize(text, root, root / "claude", root / "home",
                {"ANTHROPIC_API_KEY": secret, "ANTHROPIC_BASE_URL": "https://private.invalid"})
            event = {"message": f"credential={secret}", "items": ["https://private.invalid", str(root / "claude")], "passed": True}
            cleaned = sanitize_event(event, redact)
            encoded = json.dumps(cleaned)
            self.assertEqual(json.loads(encoded), cleaned)
            self.assertEqual(cleaned["message"], "credential=<redacted>")
            self.assertEqual(cleaned["items"], ["<redacted>", "<private-claude-config>"])
            self.assertTrue(cleaned["passed"])
            self.assertNotIn(secret, redact(json.dumps(event)))
            self.assertNotIn("private.invalid", encoded)

    def test_output_rejects_credentials_executables_and_existing_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for name in ("claude", "libtest", "supervisor", "api.json"):
                (root / name).write_text("fixture", encoding="utf-8")
            (root / "config").mkdir()
            (root / "home").mkdir()
            args = argparse.Namespace(test_binary=root / "libtest", claude=root / "claude", supervisor=root / "supervisor",
                config_dir=root / "config", auth_home=root / "home", api_environment_file=root / "api.json", output=root / "new.ndjson")
            validate_paths(args)
            for output in (root / "api.json", root / "libtest", root / "home/new.ndjson", root / "config/new.ndjson", root / "new.metadata.json"):
                with self.subTest(output=output.name), self.assertRaises(ValueError):
                    validate_paths(argparse.Namespace(**(vars(args) | {"output": output})))
            args.output.write_text("old evidence", encoding="utf-8")
            with self.assertRaises(ValueError):
                validate_paths(args)


if __name__ == "__main__":
    unittest.main()
