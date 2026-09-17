"""固定 Claude 策略验收的离线证据门禁；不发送模型请求。"""

import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_claude_profile_live as runner


def complete_events():
    native_id, profile_hash = "123e4567-e89b-12d3-a456-426614174000", "a" * 64
    events = [{"event": "profile_verified", "native_session_id": None, "profile_sha256": profile_hash,
               "permission_mode": "plan", "fixed_profile_verified": True}]
    for phase, output in (("first_turn", "PROFILE_ONE"), ("second_turn", "PROFILE_TWO"),
                          ("edit_allow", "EDIT_ALLOWED"), ("edit_deny", "EDIT_DENIED"),
                          ("resume_result", "PROFILE_RESTORED_fixture")):
        events.extend([
            {"event": "message_accepted", "phase": phase, "message_id": phase,
             "turn_id": phase, "native_session_id": native_id},
            {"event": "turn_started", "phase": phase, "turn_id": phase, "native_session_id": native_id},
            {"event": "turn_finished", "phase": phase, "turn_id": phase, "native_session_id": native_id,
             "outcome": "Completed", "output": output},
        ])
    for phase, allowed, decision, content in (("edit_allow", True, "AllowOnce", "PROFILE_AFTER"),
                                             ("edit_deny", False, "DenyOnce", "PROFILE_BEFORE")):
        events.extend([
            {"event": "approval_requested", "phase": phase, "approval_id": phase,
             "exact_edit_fixture": True, "decision": decision},
            {"event": "approval_resolved", "phase": phase, "approval_id": phase,
             "decision": decision, "native_receipt": False},
            {"event": "file_effect_verified", "phase": phase, "allowed": allowed, "expected_content": content},
        ])
    events.extend([
        {"event": "profile_verified", "native_session_id": native_id, "profile_sha256": profile_hash,
         "permission_mode": "plan", "fixed_profile_verified": True},
        {"event": "same_profile_resume_verified", "native_session_id": native_id, "profile_sha256": profile_hash,
         "saved_profile_reloaded": True, "marker": "PROFILE_RESTORED_fixture"},
    ])
    for phase, reason in (("changed_directory", "claude_profile_identity_invalid"),
                          ("changed_policy", "claude_profile_wrong_policy"),
                          ("expanded_source_rules", "claude_profile_source_changed")):
        events.append({"event": "resume_rejected", "phase": phase, "rejected": True,
                       "user_input_sent": False, "expected_reason": reason, "details": {"reason": reason}})
    events.extend([{"event": "connection_shutdown", "native_session_id": native_id} for _ in range(2)])
    events.append({"event": "acceptance_passed", "scope": runner.SCOPE, "native_session_id": native_id,
                   "profile_sha256": profile_hash, "same_profile_restored": True, "edit_allow_and_deny_verified": True,
                   "resume_escalation_rejections": 3, "filesystem_sandbox_verified": False,
                   "app_restart_and_ui_verified": False, "native_mid_turn_mode_change_verified": False,
                   "outside_directory_tool_execution_verified": False})
    return events


class ProfileRunnerTests(unittest.TestCase):
    output = "test result: ok. 1 passed; 0 failed; 0 ignored; 7310 filtered out"

    def test_complete_proof_accepts_but_zero_test_stale_or_failed_records_do_not(self):
        events = complete_events()
        self.assertTrue(runner.verified_acceptance(0, self.output, events))
        self.assertFalse(runner.verified_acceptance(1, self.output, events))
        self.assertFalse(runner.verified_acceptance(0, "test result: ok. 0 passed; 0 failed; 1 ignored;", events))
        self.assertFalse(runner.verified_acceptance(0, self.output, events + events))
        self.assertFalse(runner.verified_acceptance(0, self.output, events + [{"event": "acceptance_failed"}]))

    def test_every_required_event_is_necessary(self):
        for index in range(len(complete_events())):
            events = complete_events()
            events.pop(index)
            self.assertFalse(runner.verified_acceptance(0, self.output, events), index)

    def test_profile_drift_or_native_session_replacement_does_not_count_as_recovery(self):
        for kind, key, value in (("profile_verified", "profile_sha256", "b" * 64),
                                  ("profile_verified", "permission_mode", "acceptEdits"),
                                  ("same_profile_resume_verified", "saved_profile_reloaded", False),
                                  ("same_profile_resume_verified", "native_session_id", "different-session"),
                                  ("message_accepted", "native_session_id", "old-session"),
                                  ("turn_started", "native_session_id", "old-session")):
            events = complete_events()
            next(event for event in events if event["event"] == kind)[key] = value
            self.assertFalse(runner.verified_acceptance(0, self.output, events), key)

    def test_edit_and_rejection_proofs_cannot_be_replaced_by_generic_success(self):
        for kind, key, value in (("approval_requested", "exact_edit_fixture", False),
                                  ("approval_resolved", "native_receipt", True),
                                  ("file_effect_verified", "expected_content", "unchanged"),
                                  ("resume_rejected", "user_input_sent", True),
                                  ("resume_rejected", "details", {"reason": "unrelated failure"}),
                                  ("resume_rejected", "rejected", False),
                                  ("acceptance_passed", "app_restart_and_ui_verified", True),
                                  ("acceptance_passed", "native_mid_turn_mode_change_verified", True)):
            events = complete_events()
            next(event for event in events if event["event"] == kind)[key] = value
            self.assertFalse(runner.verified_acceptance(0, self.output, events), key)

    def test_project_uses_only_edit_approval_and_existing_file_fixtures(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(runner.adapter, "PROJECT_SETTINGS", runner.PROJECT_SETTINGS):
                settings = runner.prepare_project(root)
            self.assertEqual(json.loads(settings.read_text()), {"permissions": {"ask": ["Edit"]}})
            for name in ("edit-allow.txt", "edit-deny.txt"):
                self.assertEqual((root / "project" / name).read_text(), "PROFILE_BEFORE")
            self.assertEqual((root / ".infinishell-claude-profile-probe").read_text(), runner.MARKER)
            self.assertTrue((root / "outside").is_dir())

    def test_api_file_is_mandatory_no_fallback_to_existing_login(self):
        with self.assertRaisesRegex(ValueError, "API"):
            runner.run(SimpleNamespace(api_environment_file=None))

    def test_existing_runner_is_reused_and_global_configuration_is_restored(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            private = root / "private"
            private.mkdir()
            args = SimpleNamespace(api_environment_file=root / "explicit-api.json", output=root / "proof.ndjson")
            original = {key: getattr(runner.adapter, key) for key in (
                "TEST_NAME", "PROJECT_SETTINGS", "prepare_project", "verified_acceptance")}
            def fake_run(value):
                self.assertIs(value, args)
                self.assertEqual(runner.adapter.TEST_NAME, runner.TEST_NAME)
                self.assertIs(runner.adapter.verified_acceptance, runner.verified_acceptance)
                self.assertEqual(list(args.config_dir.iterdir()), [])
                self.assertEqual(list(args.auth_home.iterdir()), [])
                args.output.with_suffix(".metadata.json").write_text(json.dumps({"acceptance_passed": False}))
                return 1
            with patch.object(runner.tempfile, "mkdtemp", return_value=str(private)), \
                    patch.object(runner.adapter, "validate_paths"), patch.object(runner.adapter, "run", side_effect=fake_run):
                self.assertEqual(runner.run(args), 1)
            for key, value in original.items():
                self.assertIs(getattr(runner.adapter, key), value)
            metadata = json.loads(args.output.with_suffix(".metadata.json").read_text())
            self.assertFalse(metadata["acceptance_passed"])
            self.assertEqual(metadata["scope"], runner.SCOPE)
            self.assertEqual(metadata["authentication_source"], "explicit_api_environment")


if __name__ == "__main__":
    unittest.main()
