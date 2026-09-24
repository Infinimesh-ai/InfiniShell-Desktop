"""固定策略验收投影的离线回归；不会启动 CLI、访问网络或读取认证。"""

import copy
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_grok_fixed_policy as runner


def evidence():
    phases = []
    for phase, allow, value in (("new_allow", True, "allowed"), ("load_deny", False, "")):
        event = {key: True for key in runner.PHASE_BOOLS}
        event.update({key: 1 for key in runner.COUNTERS})
        event.update(event="fixed_policy_phase", scope=runner.SCOPE, phase=phase, allow=allow,
            final_sha256=runner.sha(value.encode()), native_session_sha256=runner.sha(b"native-id"),
            native_outcome="Completed" if allow else "Cancelled", failure_stage=None, denied_read_not_executed=not allow)
        phases.append(event)
    end = {key: False for key in runner.END_BOOLS}
    end.update(event="fixed_policy_finished", scope=runner.SCOPE, passed=True, system_managed_policies_apply=True)
    return [dict(runner.START), *phases, end]


def passed(events, code=0):
    return runner.observation(code, events, runner.sha(b"allowed"))["fixed_policy_passed"]


class FixedPolicyEvidenceTests(unittest.TestCase):
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
                stack.enter_context(patch.object(runner, "setup_project_sentinel", side_effect=lambda path: (path / "project").mkdir() or {}))
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
            self.assertFalse(metadata["fixed_policy_passed"])
            self.assertNotIn("OFFLINE_PRIVATE", text)

    def test_both_native_phases_pass_without_claiming_full_parity_or_sandbox(self):
        result = runner.observation(0, evidence(), runner.sha(b"allowed"))
        self.assertTrue(result["fixed_policy_passed"])
        self.assertTrue(result["cold_native_restore_verified"])
        for key in ("filesystem_sandbox_verified", "app_restart_verified", "coordinator_verified",
                "spawn_verified", "native_effective_policy_verified", "full_cli_parity_acceptance_passed"):
            self.assertFalse(result[key])

    def test_exit_success_does_not_replace_missing_native_phase(self):
        self.assertFalse(passed(evidence()[:-1]))
        self.assertFalse(passed([evidence()[0], evidence()[2], evidence()[3]]))
        self.assertFalse(passed(evidence(), 101))
        self.assertFalse(passed(evidence(), False))

    def test_tool_terminal_and_actual_approval_reply_are_required(self):
        for key, value in (("native_tool_terminal", False), ("approval_resolved", 0), ("approvals", 2),
                ("accepted", 0), ("submitted", 2), ("final_history_verified", False)):
            with self.subTest(key=key):
                events = evidence(); events[2][key] = value
                self.assertFalse(passed(events))

    def test_denial_requires_native_cancellation_and_no_read_or_post_tool_text(self):
        for key, value in (("native_outcome", "Completed"), ("native_outcome", "Failed"),
                ("denied_read_not_executed", False), ("failure_stage", "turn_finished"),
                ("final_sha256", None)):
            with self.subTest(key=key, value=value):
                events = evidence(); events[2][key] = value
                self.assertFalse(passed(events))
        events = evidence(); events[1]["native_outcome"] = "Cancelled"
        self.assertFalse(passed(events))
        events = evidence(); events[2]["final_sha256"] = runner.sha("工具调用前的说明".encode())
        self.assertTrue(passed(events))
        events[2]["denied_read_not_executed"] = False
        self.assertFalse(passed(events))

    def test_outcome_and_failure_stage_reject_private_or_malformed_values(self):
        for key in ("native_outcome", "failure_stage"):
            for value in (True, [], {}, "PRIVATE_CANARY"):
                with self.subTest(key=key, value=value):
                    events = evidence(); events[2][key] = value
                    self.assertFalse(runner.validate_event(events[2]))
                    self.assertFalse(passed(events))

    def test_cold_restore_cannot_change_session_profile_or_replay(self):
        for key, value in (("same_saved_profile", False), ("same_native_session", False),
                ("no_replay_before_input", False), ("native_session_sha256", runner.sha(b"other-id"))):
            with self.subTest(key=key):
                events = evidence(); events[2][key] = value
                self.assertFalse(passed(events))

    def test_project_hook_or_cleanup_failure_cannot_be_ignored(self):
        for key in ("hook_absent_at_ready", "hook_absent_after_shutdown", "managed_auth_removed",
                "cleanup_confirmed", "transport_closed"):
            with self.subTest(key=key):
                events = evidence(); events[1][key] = False
                self.assertFalse(passed(events))

    def test_unexpected_private_text_and_duplicate_phase_fail_closed(self):
        events = evidence(); events[1]["raw_content"] = "PRIVATE_CANARY"
        self.assertFalse(runner.validate_event(events[1]))
        self.assertFalse(passed(events))
        events = evidence(); events.insert(2, copy.deepcopy(events[1]))
        self.assertFalse(passed(events))

    def test_counter_types_and_digest_are_strict(self):
        for value in (True, 1.0, -1):
            events = evidence(); events[1]["approvals"] = value
            self.assertFalse(passed(events))
        events = evidence(); events[2]["final_sha256"] = None
        self.assertFalse(passed(events))
        events = evidence(); events[-1]["filesystem_sandbox_verified"] = True
        self.assertFalse(passed(events))

    def test_real_files_keep_hook_sentinel_visible_and_do_not_hide_changes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "home").mkdir(); (root / "tmp").mkdir()
            with patch.object(runner.subprocess, "run") as command:
                snapshot = runner.setup_project_sentinel(root)
                command.assert_called_once()
            hook = json.loads((root / "project/.grok/hooks/managed-policy-canary.json").read_text())
            self.assertEqual(set(hook["hooks"]), {"SessionStart", "UserPromptSubmit", "SessionEnd"})
            self.assertEqual(json.loads((root / "project/.claude/settings.json").read_text()), hook)
            self.assertTrue(runner.project_snapshot_matches(root, snapshot))
            sentinel = root / "project/.project-hook-ran"; sentinel.write_text("ran")
            self.assertFalse(runner.project_snapshot_matches(root, snapshot))
            sentinel.unlink()
            (root / "project/.grok/hooks/managed-policy-canary.json").write_text("{}")
            self.assertFalse(runner.project_snapshot_matches(root, snapshot))

    def test_cleanup_removes_only_private_auth_and_preserves_history(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for directory in (root / "home/.grok", root / "state/grok-managed/test-id/grok"):
                directory.mkdir(parents=True)
                (directory / "auth.json").write_text("offline cache")
                (directory / "history").write_text("keep")
            self.assertTrue(runner.cleanup_auth(root))
            self.assertEqual((root / "state/grok-managed/test-id/grok/history").read_text(), "keep")

    def test_cleanup_finds_coordinator_platform_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "private"
            data = root / "home/Library/Application Support/InfiniShell/grok-managed/session/grok"
            data.mkdir(parents=True)
            (data / "auth.json").write_text("private cache")
            (data / "history").write_text("keep history")
            self.assertTrue(runner.cleanup_auth(root))
            self.assertFalse((data / "auth.json").exists())
            self.assertEqual((data / "history").read_text(), "keep history")

    @unittest.skipUnless(os.name == "posix", "私有符号链接检查使用 POSIX 前提；不要求 Windows 创建链接权限")
    def test_cleanup_rejects_external_posix_symlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "private"
            (root / "home").mkdir(parents=True)
            source = Path(temporary) / "source"
            source.mkdir()
            (source / "auth.json").write_text("source cache")
            (root / "home/.grok").symlink_to(source, target_is_directory=True)
            with self.assertRaises(ValueError):
                runner.cleanup_auth(root)
            self.assertEqual((source / "auth.json").read_text(), "source cache")

    def test_four_phase_json_budget_is_checked_on_every_platform(self):
        payload = "".join(json.dumps(event) + "\n" for event in evidence()).encode()
        with patch.object(runner.isolation, "private_bytes", return_value=payload):
            self.assertEqual(runner.read_events(Path("offline-events.ndjson")), evidence())
        with patch.object(runner.isolation, "private_bytes", return_value=payload + json.dumps(evidence()[-1]).encode()):
            with self.assertRaises(ValueError):
                runner.read_events(Path("offline-events.ndjson"))

    @unittest.skipUnless(os.name == "posix", "在线运行器私有文件读取限定 POSIX；不代表 Windows 原生验收")
    def test_four_phase_events_are_read_without_the_single_input_lease_limit(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "events.ndjson"
            path.write_text("".join(json.dumps(event) + "\n" for event in evidence()))
            path.chmod(0o600)
            self.assertEqual(runner.read_events(path), evidence())
            with path.open("a") as output:
                output.write(json.dumps(evidence()[-1]) + "\n")
            with self.assertRaises(ValueError):
                runner.read_events(path)

    def test_duplicate_keys_and_non_finite_json_are_rejected(self):
        # 只替代 POSIX 私有文件读取；两平台仍运行真实 JSON 拒绝逻辑。
        for body in ('{"event":"started","event":"ended"}', '{"number":NaN}', '[]'):
            with self.subTest(body=body), patch.object(runner.isolation, "private_bytes", return_value=(body + "\n").encode()):
                with self.assertRaises(ValueError):
                    runner.read_events(Path("offline-events.ndjson"))

    def test_shared_path_validator_preserves_explicit_callers_budget(self):
        args = SimpleNamespace(max_native_inputs=6, timeout=800, test_binary=Path("test"),
            grok=Path("grok"), supervisor=Path("supervisor"), official_grok_home=Path("home"),
            output=Path("evidence.ndjson"))
        with patch.object(runner.isolation, "validate_paths") as validate:
            runner.validate_paths(args, max_native_inputs=6, max_deadline=900)
            validated = validate.call_args.args[0]
            self.assertEqual(validated.max_native_inputs, 1)
            self.assertEqual(validated.timeout, 450)
            self.assertEqual(args.max_native_inputs, 6)
            self.assertEqual(args.timeout, 800)
        for value in (True, 3, 6.0):
            args.max_native_inputs = value
            with self.assertRaises(ValueError):
                runner.validate_paths(args, max_native_inputs=6, max_deadline=900)

    def test_proxy_observations_cannot_claim_native_os_or_direct_network_limits(self):
        metadata = dict(runner.SANDBOX_SCOPE_FIELDS, test_exit_code=0, tunnels_stopped=True,
            private_auth_copy_removed=True, original_auth_stat_unchanged=True,
            project_snapshot_unchanged=True, binary_unchanged=True)
        tunnel = SimpleNamespace(forwarded=2, bytes=1024,
            events=[{"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"}])
        self.assertTrue(runner.boundary_passed(metadata, tunnel))
        for key in ("runtime_os_sandbox", "test_process_sandbox", "launchd_native_sandbox_verified",
                "native_direct_network_blocked", "native_network_budget_enforced"):
            with self.subTest(key=key):
                self.assertFalse(runner.boundary_passed(dict(metadata, **{key: True}), tunnel))
                self.assertFalse(runner.boundary_passed(dict(metadata, **{key: 0}), tunnel))
        self.assertFalse(runner.boundary_passed(dict(metadata, tls_budget_scope="all_native_traffic"), tunnel))

    def test_network_budget_and_binary_change_prevent_success(self):
        metadata = {"test_exit_code": 0}
        for key in ("tunnels_stopped", "private_auth_copy_removed", "original_auth_stat_unchanged",
                "project_snapshot_unchanged", "binary_unchanged"):
            metadata[key] = True
        metadata.update(runner.SANDBOX_SCOPE_FIELDS)
        tunnel = SimpleNamespace(forwarded=2, bytes=1024,
            events=[{"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"}])
        self.assertTrue(runner.boundary_passed(metadata, tunnel))
        tunnel.forwarded = 33
        self.assertFalse(runner.boundary_passed(metadata, tunnel))
        self.assertTrue(runner.boundary_passed(metadata, tunnel, connection_budget=64))
        tunnel.forwarded = 65
        self.assertFalse(runner.boundary_passed(metadata, tunnel, connection_budget=64))
        tunnel.forwarded = 2
        for budget in (True, 0, 65, 64.0):
            self.assertFalse(runner.boundary_passed(metadata, tunnel, connection_budget=budget))
        metadata["binary_unchanged"] = False
        self.assertFalse(runner.boundary_passed(metadata, tunnel))
        metadata["binary_unchanged"] = True; tunnel.bytes = runner.lease.MAX_TLS_BYTES + 1
        self.assertFalse(runner.boundary_passed(metadata, tunnel))


if __name__ == "__main__":
    unittest.main()
