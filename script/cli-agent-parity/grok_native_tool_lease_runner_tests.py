"""生产租约验收的离线证据边界；不启动 CLI、模型或读取用户认证。"""

import copy
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

import run_grok_native_tool_lease as runner


def evidence():
    audit = {"owned_process_confirmed": True, "capability_confirmed": True, "retired": False,
        "full_native_sdk_origin_fields_observed": False, "registration_requests": 2,
        "native_tool_frames": 3, "native_initial_inputs": 1, "native_complete_inputs": 1,
        "permission_writes_allow": 1, "permission_writes_deny": 0, "business_dispatches": 1,
        "reply_writes": 1, "native_completions": 1, "protocol_errors": 0, "sdk_origin_observations": 1}
    finish = {"event": "lease_finished", "scope": runner.SCOPE, "passed": True,
        "audit_before_shutdown": audit, "failure_source": None,
        "final_response_sha256": hashlib.sha256(b"INFINISHELL_NATIVE_LEASE_OK").hexdigest(),
        "production_connect_path": True, "production_sdk_registration": True, "test_only_sdk_hook": False,
        "final_history_verified": True, "authenticated_native_tool_lease_verified": True,
        "full_native_origin_fields_observed": False, "native_origin_verified": False,
        "parent_permission_ceiling_verified": False, "spawn_verified": False,
        "coordinator_dispatch_verified": False, "full_cli_parity_acceptance_passed": False,
        "cleanup_confirmed": True, "cleanup_receipt_read": True, "transport_closed": True,
        "no_project_files": True, "submitted_input_count": 1, "accepted_input_count": 1,
        "inspect_call_count": 1, "inspect_approval_count": 1, "search_approval_count": 1,
        "unexpected_tool_count": 0}
    return [dict(runner.START), finish]


class NativeToolLeaseEvidenceTests(unittest.TestCase):
    def test_complete_lease_never_claims_native_fields_or_spawn(self):
        result = runner.observation(0, evidence())
        self.assertTrue(result["lease_passed"])
        self.assertTrue(result["authenticated_native_tool_lease_verified"])
        self.assertFalse(result["native_origin_verified"])
        self.assertFalse(result["parent_permission_ceiling_verified"])
        self.assertFalse(result["spawn_verified"])
        self.assertFalse(result["full_cli_parity_acceptance_passed"])

    def test_success_exit_without_observed_fixture_cannot_pass(self):
        self.assertFalse(runner.observation(0, [dict(runner.START)])["lease_passed"])
        self.assertFalse(runner.observation(0, [])["lease_passed"])

    def test_runtime_failure_cannot_be_overridden_by_positive_audit(self):
        self.assertFalse(runner.observation(101, evidence())["lease_passed"])
        self.assertFalse(runner.observation(-9, evidence())["lease_passed"])
        self.assertFalse(runner.observation(False, evidence())["lease_passed"])

    def test_absent_second_native_input_phase_is_not_full_lease(self):
        events = evidence()
        events[-1]["audit_before_shutdown"]["native_complete_inputs"] = 0
        events[-1]["audit_before_shutdown"]["native_tool_frames"] = 20
        self.assertFalse(runner.observation(0, events)["lease_passed"])

    def test_planned_reply_is_not_written_reply(self):
        events = evidence()
        events[-1]["audit_before_shutdown"]["reply_writes"] = 0
        self.assertFalse(runner.observation(0, events)["lease_passed"])

    def test_finished_model_does_not_replace_native_tool_completion(self):
        events = evidence()
        events[-1]["audit_before_shutdown"]["native_completions"] = 0
        self.assertFalse(runner.observation(0, events)["lease_passed"])

    def test_unknown_origin_observation_preserves_null_and_fails(self):
        events = evidence()
        events[-1]["full_native_origin_fields_observed"] = None
        events[-1]["audit_before_shutdown"]["full_native_sdk_origin_fields_observed"] = None
        events[-1]["audit_before_shutdown"]["sdk_origin_observations"] = 0
        events[-1]["passed"] = False
        events[-1]["failure_source"] = "acceptance"
        self.assertTrue(runner.validate_event(events[-1]))
        self.assertFalse(runner.observation(0, events)["lease_passed"])

    def test_false_claims_of_origin_or_parent_permission_fail(self):
        for key in ("full_native_origin_fields_observed", "native_origin_verified",
                "parent_permission_ceiling_verified", "spawn_verified", "coordinator_dispatch_verified"):
            with self.subTest(key=key):
                events = evidence()
                events[-1][key] = True
                self.assertFalse(runner.observation(0, events)["lease_passed"])

    def test_bool_counter_or_multiple_inputs_fail(self):
        for value in (True, 2, -1, 1.0):
            with self.subTest(value=value):
                events = evidence()
                events[-1]["submitted_input_count"] = value
                self.assertFalse(runner.observation(0, events)["lease_passed"])

    def test_duplicate_finish_or_unknown_public_text_fail_closed(self):
        events = evidence()
        self.assertFalse(runner.observation(0, events + [copy.deepcopy(events[-1])])["lease_passed"])
        events[-1]["raw_error"] = "OFFLINE_PRIVATE_TEXT_CANARY"
        self.assertFalse(runner.validate_event(events[-1]))
        self.assertFalse(runner.observation(0, events)["lease_passed"])

    def test_duplicate_json_keys_and_nonfinite_numbers_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "events.ndjson"
            path.touch(mode=0o600)
            for body in ('{"event":"lease_started","event":"lease_finished"}\n', '{"counter":NaN}\n'):
                path.write_text(body)
                with self.assertRaises(ValueError):
                    runner.read_events(path)

    def test_network_or_cleanup_failure_prevents_pass(self):
        metadata = {"test_exit_code": 0, "tunnels_stopped": True, "private_auth_copy_removed": True,
            "original_auth_stat_unchanged": True, "private_settings_audit": {"settings_scope_verified": True},
            "project_entry_count": 0}
        launches = [{"kind": kind, "arguments_unchanged": True} for kind in ("version", "version", "direct_agent")]
        tunnel = SimpleNamespace(forwarded=4, bytes=1024,
            events=[{"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"}])
        self.assertTrue(runner.boundary_passed(metadata, launches, tunnel))
        tunnel.bytes = runner.MAX_TLS_BYTES + 1
        self.assertFalse(runner.boundary_passed(metadata, launches, tunnel))
        tunnel.bytes = 1024
        metadata["original_auth_stat_unchanged"] = False
        self.assertFalse(runner.boundary_passed(metadata, launches, tunnel))
        metadata["original_auth_stat_unchanged"] = True
        self.assertFalse(runner.boundary_passed(metadata, launches[:-1], tunnel))

    def test_auth_identity_uses_only_private_regular_file_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            auth = home / "auth.json"
            auth.write_text("not parsed JSON")
            auth.chmod(0o600)
            self.assertEqual(runner.auth_identity(home), runner.auth_identity(home))
            auth.chmod(0o644)
            with self.assertRaises(ValueError):
                runner.auth_identity(home)
            auth.unlink()
            target = home / "target"
            target.touch(mode=0o600)
            auth.symlink_to(target)
            with self.assertRaises(ValueError):
                runner.auth_identity(home)


if __name__ == "__main__":
    unittest.main()
