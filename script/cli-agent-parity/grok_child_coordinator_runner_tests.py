"""父子运行器只从完整、真实边界内的脱敏证据确认有限能力。"""

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import run_grok_child_coordinator as runner


def evidence():
    chain = dict(runner.CHAIN, parent_native_sha256="1" * 64, child_native_sha256="2" * 64,
        parent_result_sha256="3" * 64, child_result_sha256="4" * 64)
    def cleanup_pair(token):
        runtime = f"00000000-0000-4000-8000-{token:012d}"
        receipt = {"event": "runtime_host_cleanup_receipt", "scope": runner.SCOPE,
            "runtime_generation": runtime, "receipt": {"version": 2,
                "runtime_generation": runtime, "host_instance_id": runtime,
                "manifest_sha256": "5" * 64, "journal_sha256": "6" * 64,
                "native_cleanup_sha256": "7" * 64, "last_event_sequence": 20,
                "acknowledged_sequence": 20, "native_process": "exited",
                "adapter_task_terminated": True, "adapter_succeeded": True,
                "event_journal_completed": True}}
        cleanup = {"event": "cleanup_verified", "scope": runner.SCOPE,
            "runtime_sha256": hashlib.sha256(runtime.encode()).hexdigest(),
            "cleanup_confirmed": True}
        return [receipt, cleanup]
    return [dict(runner.START), dict(runner.GATE), chain, *cleanup_pair(1),
        *cleanup_pair(2), dict(runner.RESUME), *cleanup_pair(3), dict(runner.END)]


class EvidenceTests(unittest.TestCase):
    def test_tunnel_close_error_still_removes_auth_and_publishes_only_error_type(self):
        from contextlib import ExitStack, nullcontext, redirect_stdout
        import io
        from types import SimpleNamespace
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary); root = base / "workspace"; root.mkdir()
            binary = base / "binary"; binary.write_bytes(b"offline-binary")
            args = SimpleNamespace(output=base / "safe.ndjson", grok=binary, test_binary=binary,
                supervisor=binary, official_grok_home=base / "source", timeout=30, case="visible")
            tunnel = SimpleNamespace(start=lambda: 1234, forwarded=0, bytes=0, events=[])
            def close():
                raise OSError("OFFLINE_PRIVATE_CLOSE_CANARY")
            tunnel.close = close
            def copy_auth(source, target):
                (target / "auth.json").write_text("OFFLINE_PRIVATE_AUTH_CANARY", encoding="utf-8")
            with ExitStack() as stack:
                stack.enter_context(patch.object(runner.tempfile, "mkdtemp", return_value=str(root)))
                stack.enter_context(patch.object(runner.isolation, "reserve_artifacts"))
                stack.enter_context(patch.object(runner.lease, "auth_identity", return_value=(1, 2, 3)))
                stack.enter_context(patch.object(runner.isolation, "bounded_tunnel", return_value=nullcontext(tunnel)))
                stack.enter_context(patch.object(runner.official, "copy_private_auth", side_effect=copy_auth))
                stack.enter_context(patch.object(runner.subprocess, "Popen", side_effect=AssertionError("不应启动进程")))
                stack.enter_context(patch.object(runner.fixed, "setup_project_sentinel", side_effect=lambda path: (path / "project").mkdir() or {}))
                stack.enter_context(patch.object(runner.official, "official_environment", side_effect=OSError("OFFLINE_PRIVATE_STAGE_CANARY")))
                stack.enter_context(redirect_stdout(io.StringIO()))
                self.assertEqual(runner.run(args), 1)
            self.assertFalse((root / "home/.grok/auth.json").exists())
            text = args.output.with_suffix(".metadata.json").read_text(encoding="utf-8")
            metadata = json.loads(text)
            self.assertEqual(metadata["tunnel_cleanup_error_type"], "OSError")
            self.assertFalse(metadata["tunnels_stopped"])
            self.assertTrue(metadata["private_auth_copy_removed"])
            self.assertFalse(metadata["boundary_passed"])
            self.assertFalse(metadata["parent_child_passed"])
            self.assertNotIn("OFFLINE_PRIVATE", text)

    def test_complete_chain_claims_only_observed_scope(self):
        result = runner.observation(0, evidence())
        self.assertTrue(result["parent_child_passed"])
        self.assertTrue(result["cold_resume_ready_verified"])
        self.assertTrue(result["native_ack_both_directions"])
        self.assertTrue(result["automatic_result_ack_verified"])
        self.assertTrue(result["final_result_via_inspect_verified"])
        for key in ("app_restart_verified", "real_gui_verified", "native_effective_policy_verified",
                "filesystem_sandbox_verified", "full_cli_parity_acceptance_passed"):
            self.assertFalse(result[key])

    def test_nonzero_or_boolean_exit_never_passes(self):
        for code in (1, -9, False, None, "0"):
            with self.subTest(code=code):
                self.assertFalse(runner.observation(code, evidence())["parent_child_passed"])

    def test_missing_duplicate_or_reordered_events_fail(self):
        events = evidence()
        changed = copy.deepcopy(events)
        changed[3], changed[5] = changed[5], changed[3]
        for value in (events[:-1], events + [events[-1]], changed, []):
            self.assertFalse(runner.observation(0, value)["parent_child_passed"])

    def test_sent_cannot_be_claimed_as_early_native_ack(self):
        events = evidence()
        events[1]["native_ack"] = True
        self.assertFalse(runner.observation(0, events)["parent_child_passed"])

    def test_cleanup_requires_three_distinct_runtime_receipts(self):
        events = evidence()
        events[8:10] = copy.deepcopy(events[3:5])
        self.assertFalse(runner.observation(0, events)["parent_child_passed"])

    def test_exit_receipt_must_prove_native_cleanup_for_the_matching_generation(self):
        for key, value in (("native_process", "unknown"), ("adapter_task_terminated", False),
                ("adapter_succeeded", False), ("event_journal_completed", False),
                ("runtime_generation", "00000000-0000-4000-8000-000000000009"),
                ("acknowledged_sequence", 21), ("version", True), ("manifest_sha256", "private")):
            with self.subTest(key=key):
                events = evidence()
                events[3]["receipt"][key] = value
                self.assertFalse(runner.observation(0, events)["parent_child_passed"])
        events = evidence()
        events[3]["receipt"]["private_transcript"] = "DO_NOT_PUBLISH"
        self.assertFalse(runner.validate_event(events[3]))

    def test_fixed_cli_version_cannot_reuse_an_older_chain_receipt(self):
        events = evidence()
        events[0]["cli_version"] = "1.0.30"
        self.assertFalse(runner.observation(0, events)["parent_child_passed"])

    def test_failed_adapter_receipt_remains_publishable_without_passing_acceptance(self):
        events = evidence()
        events[3]["receipt"]["adapter_succeeded"] = False
        self.assertTrue(all(runner.validate_event(event) for event in events))
        self.assertFalse(runner.observation(0, events)["parent_child_passed"])
        events[3]["receipt"]["adapter_succeeded"] = "false"
        self.assertFalse(runner.validate_event(events[3]))

    def test_parent_child_must_have_distinct_native_sessions(self):
        events = evidence()
        events[2]["child_native_sha256"] = events[2]["parent_native_sha256"]
        self.assertFalse(runner.observation(0, events)["parent_child_passed"])

    def test_native_input_and_result_counts_are_exact(self):
        for key in ("tasks", "parent_generations", "child_generations", "native_inputs", "sdk_tool_calls"):
            events = evidence()
            events[2][key] += 1
            self.assertFalse(runner.observation(0, events)["parent_child_passed"])

    def test_boolean_and_integer_are_not_interchangeable(self):
        for index, key, value in ((0, "max_native_inputs", True), (1, "sent", 1),
                (2, "native_ack_both_directions", 1), (6, "cleanup_confirmed", 1)):
            events = evidence()
            events[index][key] = value
            self.assertFalse(runner.observation(0, events)["parent_child_passed"])

    def test_unknown_field_is_not_exportable(self):
        for event in evidence():
            event["auth"] = "DO_NOT_PUBLISH"
            self.assertFalse(runner.validate_event(event))

    def test_only_hashes_are_accepted_for_identity_and_result(self):
        for key in runner.CHAIN_HASHES:
            events = evidence()
            events[2][key] = "PRIVATE_BODY_CANARY"
            self.assertFalse(runner.validate_event(events[2]))

    def test_full_goal_or_restart_cannot_be_inferred(self):
        for key in ("app_restart_verified", "full_cli_parity_acceptance_passed", "native_effective_policy_verified"):
            events = evidence()
            events[-1][key] = True
            self.assertFalse(runner.observation(0, events)["parent_child_passed"])

    def test_failure_is_exportable_but_never_passes(self):
        event = {"event": "acceptance_failed", "scope": runner.SCOPE,
            "reason_bytes": 16, "reason_sha256": "8" * 64}
        self.assertTrue(runner.validate_event(event))
        self.assertFalse(runner.observation(0, evidence()[:-1] + [event])["parent_child_passed"])

    def test_eleven_event_json_budget_is_checked_on_every_platform(self):
        payload = "".join(json.dumps(event) + "\n" for event in evidence()).encode()
        with mock.patch.object(runner.isolation, "private_bytes", return_value=payload):
            self.assertEqual(runner.read_events(Path("offline-events.ndjson")), evidence())
        with mock.patch.object(runner.isolation, "private_bytes", return_value=payload + json.dumps(runner.END).encode()):
            with self.assertRaises(ValueError):
                runner.read_events(Path("offline-events.ndjson"))

    @unittest.skipUnless(os.name == "posix", "在线运行器私有文件读取限定 POSIX；不代表 Windows 原生验收")
    def test_reader_accepts_eleven_events_and_rejects_twelfth(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "events.ndjson"
            path.write_text("".join(json.dumps(item) + "\n" for item in evidence()))
            path.chmod(0o600)
            self.assertEqual(runner.read_events(path), evidence())
            with path.open("a") as file:
                file.write(json.dumps(runner.END) + "\n")
            with self.assertRaises(ValueError):
                runner.read_events(path)

    def test_reader_rejects_duplicate_keys_or_nonfinite_numbers(self):
        # 只替代 POSIX 私有文件读取；两平台仍运行真实 JSON 拒绝逻辑。
        for source in ('{"event":1,"event":2}', '{"event":NaN}', '[]'):
            with self.subTest(source=source), mock.patch.object(runner.isolation, "private_bytes", return_value=source.encode()):
                with self.assertRaises(ValueError):
                    runner.read_events(Path("offline-events.ndjson"))

    def test_private_state_and_observed_proxy_boundary_remains_required(self):
        tunnel = mock.Mock(forwarded=1, bytes=1,
            events=[{"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"}])
        metadata = {"test_exit_code": 0, "timed_out": False, "tunnels_stopped": True,
            "private_auth_copy_removed": True, "original_auth_stat_unchanged": True,
            "project_snapshot_unchanged": True, "binary_unchanged": True}
        metadata.update(runner.fixed.SANDBOX_SCOPE_FIELDS)
        self.assertTrue(runner.fixed.boundary_passed(metadata, tunnel))
        for key in ("private_auth_copy_removed", "binary_unchanged", "original_auth_stat_unchanged", "tunnels_stopped"):
            changed = dict(metadata, **{key: False})
            self.assertFalse(runner.fixed.boundary_passed(changed, tunnel))

    def test_child_runner_cannot_infer_a_sandbox_from_private_state(self):
        metadata = dict(runner.fixed.SANDBOX_SCOPE_FIELDS, test_exit_code=0, tunnels_stopped=True,
            private_auth_copy_removed=True, original_auth_stat_unchanged=True,
            project_snapshot_unchanged=True, binary_unchanged=True)
        tunnel = mock.Mock(forwarded=1, bytes=1,
            events=[{"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"}])
        self.assertTrue(runner.fixed.boundary_passed(metadata, tunnel))
        metadata["runtime_os_sandbox"] = True
        self.assertFalse(runner.fixed.boundary_passed(metadata, tunnel))
        metadata["runtime_os_sandbox"] = False
        metadata.pop("tls_budget_scope")
        self.assertFalse(runner.fixed.boundary_passed(metadata, tunnel))

    def test_validation_preserves_six_input_budget(self):
        args = argparse.Namespace(max_native_inputs=6, timeout=600, test_binary=Path("test"), grok=Path("grok"), supervisor=Path("supervisor"), official_grok_home=Path("home"), output=Path("output"))
        with mock.patch.object(runner.fixed.isolation, "validate_paths") as validate:
            runner.fixed.validate_paths(args, max_native_inputs=6, max_deadline=600)
        validate.assert_called_once()
        self.assertEqual(args.max_native_inputs, 6)
        self.assertEqual(args.timeout, 600)


if __name__ == "__main__":
    unittest.main()
