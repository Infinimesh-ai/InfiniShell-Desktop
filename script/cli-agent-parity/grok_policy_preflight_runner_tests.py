#!/usr/bin/env python3
"""Grok 无模型接口调查运行器的离线回归；不启动原生进程或访问认证。"""

from contextlib import contextmanager, redirect_stderr
import copy
import io
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import run_grok_policy_preflight as runner


FIRST = "11111111-1111-4111-8111-111111111111"
SECOND = "22222222-2222-4222-8222-222222222222"
FOREIGN = "33333333-3333-4333-8333-333333333333"
PROFILE = runner.sha(b"offline-profile")
CONFIG = runner.sha(b"offline-config")
SUMMARY = b"test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;\n"


def fixture():
    # 合成记录只用于审计负例，不能作为原生协议、模式或权限证明。
    rows = [{"event": "probe_started", "scope": runner.SCOPE, "max_native_inputs": 0,
        "request_budget": 12, "allowed_methods_sha256": runner.sha(runner.canonical(list(runner.ALLOWED_METHODS))),
        "guard_before_native_write": True, "production_supervision": True,
        "candidate_source_is_exact_binary": False}]
    sequence = 0
    for generation, phase, opening in ((FIRST, "new", "session/new"), (SECOND, "resume", "session/load")):
        rows.append({"event": "launch_snapshot", "generation": generation, "phase": phase,
            "cli_version": runner.VERSION, "cli_sha256": runner.BINARY_SHA256,
            "profile_sha256": PROFILE, "config_sha256": CONFIG,
            "always_approve_requested": False, "auto_mode_requested": False})
        for method in ("initialize", opening, *runner.DIAGNOSTIC_METHODS):
            sequence += 1
            rows.append({"event": "rpc_sent", "generation": generation, "sequence": sequence,
                "method": method, "rpc_id": sequence, "request_bytes": 80,
                "request_sha256": runner.sha(method.encode()), "guard_checked_before_write": True})
            rows.append({"event": "rpc_response", "generation": generation, "sequence": sequence,
                "rpc_id": sequence, "status": "ok", "response_bytes": 50,
                "response_sha256": runner.sha(b"offline-response")})
            if method in runner.DIAGNOSTIC_METHODS:
                rows.append({"event": "diagnostic_observed", "generation": generation, "method": method,
                    "status": "ok", "top_level_key_sha256s": [runner.sha(b"offline-key")]})
        rows.extend([
            {"event": "mode_observation", "generation": generation, "state": "unknown", "value_sha256": None},
            {"event": "catalog_observation", "generation": generation, "state": "unknown",
                "coverage": "mcp_only", "tool_count": 0, "catalog_sha256": runner.sha(b"[]"),
                "fixed_tool_closure_verified": False},
            {"event": "source_observation", "generation": generation, "state": "unknown",
                "extra_source_count": 0, "all_configuration_sources_verified": False},
        ])
        if phase == "resume":
            rows.append({"event": "resume_checked", "generation": generation,
                "native_session_id_sha256": runner.sha(b"offline-session"),
                "original_profile_sha256": PROFILE, "current_profile_sha256": PROFILE,
                "same_profile": True, "inputs_replayed": 0})
        rows.append({"event": "process_cleanup", "generation": generation, "exit_code": 0,
            "exit_reason": "stdio_closed", "cleanup_confirmed": True,
            "receipt_sha256": runner.sha(b"offline-cleanup")})
    rows.append({"event": "probe_finished", "scope": runner.SCOPE, "native_inputs": 0,
        "tool_exec_count": 0, "unexpected_native_activity": 0, "protocol_request_count": sequence,
        "guard_before_native_write": True, "transport_closed": True,
        "parent_permission_ceiling_verified": False, "filesystem_sandbox_verified": False,
        "profile_loading": "unknown", "effective_mode": "unknown", "configuration_sources": "unknown",
        "builtin_catalog": "unknown", "wrapper_closure": "unknown"})
    return rows


def event(rows, name, generation=None):
    return next(row for row in rows if row["event"] == name
        and (generation is None or row.get("generation") == generation))


class PolicyPreflightRunnerTests(unittest.TestCase):
    def audit(self, rows=None, stdout=SUMMARY, exit_code=0):
        return runner.audit_events(exit_code, stdout, fixture() if rows is None else rows)

    def rejected(self, rows, code=None):
        result = self.audit(rows)
        self.assertFalse(result["execution_boundary_passed"])
        self.assertFalse(result["interface_investigation_completed"])
        self.assertEqual(result["policy"], runner.unknown_policy())
        if code is not None:
            self.assertEqual(result["failure_code"], code)
        return result

    def resource_exit_failure(self, failure):
        prepared = runner.RUST_ENTRYPOINT_PREPARED
        # 只模拟两个资源域的退出错误；输入、输出和工作区都是无认证的合成材料。
        with TemporaryDirectory() as temporary:
            test_root = Path(temporary).resolve()
            workspace = TemporaryDirectory()
            try:
                args = SimpleNamespace(test_binary=test_root / "test-binary", supervisor=test_root / "supervisor",
                    grok=test_root / "grok", profile=test_root / "profile.md", config=test_root / "config.toml",
                    output=test_root / "result.ndjson", official_grok_home=None, timeout=30,
                    max_tls_connections=1, max_tls_bytes=1024, max_native_inputs=0)
                args.profile.write_bytes(b"offline-profile")
                args.config.write_bytes(b"offline-config")
                args.test_binary.write_bytes(b"offline-test-binary")
                args.supervisor.write_bytes(b"offline-supervisor")
                args.grok.write_bytes(b"offline-grok")
                tunnel = SimpleNamespace(start=lambda: 44321, closing=False, forwarded=0, bytes=0,
                    thread=SimpleNamespace(is_alive=lambda: False))

                def close_tunnel():
                    tunnel.closing = True
                    if failure == "tunnel":
                        raise OSError("PRIVATE_TUNNEL_EXIT_CANARY")
                    return True

                tunnel.close = close_tunnel

                @contextmanager
                def temporary_workspace(**options):
                    try:
                        yield workspace.name
                    finally:
                        if failure == "workspace":
                            raise OSError("PRIVATE_WORKSPACE_EXIT_CANARY")
                        workspace.cleanup()

                def prepare_native(root, executable, source_home, port):
                    wrapper = root / "grok-sandbox"
                    settings = root / "home/.grok/config.toml"
                    wrapper.write_text("offline-wrapper", encoding="utf-8")
                    settings.write_bytes(b"offline-config")
                    return wrapper, settings

                def capture(command, environment, directory, timeout, byte_budget):
                    if "--list" in command:
                        return 0, (runner.TEST_NAME + ": test\n").encode()
                    artifact = Path(environment["INFINISHELL_GROK_POLICY_PREFLIGHT_ARTIFACT"])
                    artifact.write_text("".join(json.dumps(row) + "\n" for row in fixture()), encoding="utf-8")
                    return 0, SUMMARY + b"PRIVATE_STDOUT_CANARY\n"

                official = SimpleNamespace(MAX_TUNNELS=32, MAX_BYTES=2048,
                    OfficialTunnel=lambda timeout: tunnel, prepare_native=prepare_native,
                    official_environment=lambda root, port: {}, copy_private_auth=Mock(),
                    artifacts=lambda output: [output, output.with_suffix(".metadata.json"), output.with_suffix(".network.json")])
                fixed = SimpleNamespace(digest=lambda path: runner.sha(path.read_bytes()))
                with patch.dict(sys.modules, {"run_grok_official_adapter_live": official, "prepare_grok_cli": fixed}), \
                        patch.object(runner, "validate_paths") as validate, \
                        patch.object(runner.tempfile, "TemporaryDirectory", temporary_workspace), \
                        patch.object(runner, "rewrite_wrapper", side_effect=lambda code, path, checksum: code), \
                        patch.object(runner, "capture_process", side_effect=capture) as captured, \
                        patch.object(runner.subprocess, "Popen") as spawn:
                    code = runner.run(args)
                    validate.assert_called_once_with(args)
                    self.assertEqual(captured.call_count, 2)
                    self.assertIn("--list", captured.call_args_list[0].args[0])
                    official.copy_private_auth.assert_not_called()
                    spawn.assert_not_called()
                rows = [json.loads(line) for line in args.output.read_text(encoding="utf-8").splitlines()]
                metadata = json.loads(args.output.with_suffix(".metadata.json").read_text(encoding="utf-8"))
                network = json.loads(args.output.with_suffix(".network.json").read_text(encoding="utf-8"))
                self.assertEqual(code, 1)
                self.assertEqual(rows[:-1], fixture())
                self.assertEqual(rows[-1], {"event": "probe_failed", "reason_bytes": len(b"outer_resource_failed"),
                    "reason_sha256": runner.sha(b"outer_resource_failed")})
                self.assertEqual(runner.project_events(rows), (rows, []))
                self.assertFalse(metadata["execution_boundary_passed"])
                self.assertFalse(metadata["interface_investigation_completed"])
                self.assertFalse(metadata["runner_cleanup_confirmed"])
                self.assertEqual(metadata["outer_resource_error_type"], "OSError")
                self.assertEqual(metadata["runner_error_type"], "OSError")
                self.assertTrue(metadata["opaque_auth_copy_removed"])
                self.assertEqual(metadata["policy"], runner.unknown_policy())
                self.assertEqual((official.MAX_TUNNELS, official.MAX_BYTES), (32, 2048))
                self.assertEqual(network["tls_bytes"], 0)
                self.assertNotIn("PRIVATE_", json.dumps([rows, metadata, network]))
                self.assertIs(runner.RUST_ENTRYPOINT_PREPARED, prepared)
                return metadata
            finally:
                workspace.cleanup()

    def test_temporary_directory_exit_error_preserves_strict_evidence_and_failure(self):
        metadata = self.resource_exit_failure("workspace")
        self.assertTrue(metadata["tunnels_stopped"])
        self.assertFalse(metadata["private_workspace_removed"])

    def test_tunnel_exit_error_preserves_strict_evidence_and_failure(self):
        metadata = self.resource_exit_failure("tunnel")
        self.assertFalse(metadata["tunnels_stopped"])
        self.assertTrue(metadata["private_workspace_removed"])

    def test_unprepared_guard_rejects_before_run_or_io(self):
        with patch.object(runner, "RUST_ENTRYPOINT_PREPARED", False), patch.object(runner, "run") as run, patch.object(runner.subprocess, "Popen") as spawn, \
                patch.object(Path, "read_bytes") as read, redirect_stderr(io.StringIO()) as output:
            self.assertEqual(runner.main([]), 2)
            run.assert_not_called()
            spawn.assert_not_called()
            read.assert_not_called()
        self.assertNotIn("Traceback", output.getvalue())

    def test_explicit_auth_path_cannot_bypass_unprepared_guard(self):
        path = "/nonexistent/offline-auth-do-not-read"
        with patch.object(runner, "RUST_ENTRYPOINT_PREPARED", False), patch.object(runner, "run") as run, patch.object(runner.os, "open") as opening, \
                redirect_stderr(io.StringIO()) as output:
            self.assertEqual(runner.main(["--official-grok-home", path, "--max-native-inputs", "1"]), 2)
            run.assert_not_called()
            opening.assert_not_called()
        self.assertNotIn(path, output.getvalue())

    def test_protocol_budget_zero_and_invalid_values_reject(self):
        for budget in (0, -1, 13, False):
            with self.subTest(budget=budget), self.assertRaisesRegex(runner.PreflightRejected, "request_budget_invalid"):
                runner.make_plan(PROFILE, CONFIG, budget)
        plan = runner.make_plan(PROFILE, CONFIG)
        self.assertEqual(plan["max_native_inputs"], 0)
        self.assertNotIn("session/prompt", plan["allowed_methods"])
        self.assertNotIn("authenticate", plan["allowed_methods"])

    def test_stdout_budget_zero_prevents_process_spawn(self):
        with patch.object(runner.subprocess, "Popen") as spawn:
            with self.assertRaisesRegex(runner.PreflightRejected, "stdout_budget_invalid"):
                runner.capture_process([], {}, Path("/nonexistent"), 30, 0)
            spawn.assert_not_called()

    def test_network_zero_budget_disables_connections_and_restores_limits(self):
        observed = []
        official = SimpleNamespace(MAX_TUNNELS=32, MAX_BYTES=1024)
        tunnel = SimpleNamespace(close=lambda: True)
        def construct(timeout):
            observed.append((official.MAX_TUNNELS, official.MAX_BYTES, timeout))
            return tunnel
        official.OfficialTunnel = construct
        with runner.bounded_tunnel(official, 30, 16, 0):
            self.assertFalse(tunnel.preflight_cleanup_confirmed)
        self.assertEqual(observed, [(0, 0, 30)])
        self.assertEqual((official.MAX_TUNNELS, official.MAX_BYTES), (32, 1024))
        self.assertTrue(tunnel.preflight_cleanup_confirmed)

    def test_tunnel_close_false_is_not_cleanup_success(self):
        tunnel = SimpleNamespace(close=lambda: False)
        official = SimpleNamespace(MAX_TUNNELS=32, MAX_BYTES=1024, OfficialTunnel=lambda timeout: tunnel)
        with runner.bounded_tunnel(official, 30, 1, 100):
            pass
        self.assertFalse(tunnel.preflight_cleanup_confirmed)
        self.assertEqual((official.MAX_TUNNELS, official.MAX_BYTES), (32, 1024))

    def test_tunnel_constructor_failure_restores_limits(self):
        def fail(timeout):
            raise RuntimeError("offline-failure")
        official = SimpleNamespace(MAX_TUNNELS=32, MAX_BYTES=1024, OfficialTunnel=fail)
        with self.assertRaises(RuntimeError):
            with runner.bounded_tunnel(official, 30, 1, 100):
                self.fail("constructor must reject")
        self.assertEqual((official.MAX_TUNNELS, official.MAX_BYTES), (32, 1024))

    def test_complete_synthetic_ledger_preserves_unknown_policy(self):
        result = self.audit()
        self.assertTrue(result["execution_boundary_passed"])
        self.assertTrue(result["interface_investigation_completed"])
        self.assertEqual(result["policy"], runner.unknown_policy())
        self.assertFalse(result["policy"]["ready_for_policy_implementation"])

    def test_missing_native_method_preserves_failure_without_catalog_success(self):
        rows = fixture()
        response = next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == 5)
        response["status"] = "method_not_found"
        event(rows, "diagnostic_observed")["status"] = "method_not_found"
        result = self.rejected(rows, "interface_unavailable")
        self.assertIn(response, result["events"])

    def test_unknown_and_requested_ask_modes_remain_unknown(self):
        for mode in (None, "ask", "yolo", "offline-private-mode-canary"):
            with self.subTest(mode=mode):
                observation = runner.observe_mode(mode)
                self.assertEqual(observation["state"], "unknown")
                self.assertNotIn("offline-private-mode-canary", json.dumps(observation))
        rows = fixture()
        event(rows, "mode_observation")["state"] = "ask"
        self.rejected(rows, "test_or_projection_failed")

    def test_unknown_tool_fallback_and_wrappers_never_prove_catalog(self):
        result = runner.catalog_observation(["offline-missing-tool"], ["ReadFile", "SearchTool", "UseTool", "Agent"])
        self.assertEqual(result["reason"], "unknown_name_or_native_fallback")
        self.assertEqual(result["missing_name_count"], 1)
        self.assertEqual(result["extra_tool_count"], 4)
        self.assertFalse(result["fixed_tool_closure_verified"])
        self.assertNotIn("offline-missing-tool", json.dumps(result))
        same = runner.catalog_observation(["ReadFile"], ["ReadFile"])
        self.assertEqual(same["state"], "unknown")
        self.assertFalse(same["fixed_tool_closure_verified"])

    def test_extra_configuration_source_rejects_and_is_not_disclosed(self):
        extra = runner.sha(b"offline-extra-source")
        observation = runner.source_observation([CONFIG], [CONFIG, extra])
        self.assertEqual(observation["extra_source_count"], 1)
        self.assertFalse(observation["all_configuration_sources_verified"])
        rows = fixture()
        event(rows, "source_observation")["extra_source_count"] = 1
        self.rejected(rows, "extra_source_observed")

    def test_different_resume_profile_rejects_before_claiming_recovery(self):
        changed = runner.sha(b"offline-changed-profile")
        self.assertFalse(runner.resume_profile_matches(PROFILE, changed))
        rows = fixture()
        event(rows, "launch_snapshot", SECOND)["profile_sha256"] = changed
        self.rejected(rows, "resume_snapshot_changed")
        rows = fixture()
        event(rows, "resume_checked")["current_profile_sha256"] = changed
        self.rejected(rows, "resume_profile_changed")

    def test_tool_and_prompt_methods_are_rejected_with_hash_only(self):
        for method in ("session/prompt", "x.ai/tools/list", "terminal/create", "tools/call"):
            with self.subTest(method=method):
                rows = fixture()
                event(rows, "rpc_sent")["method"] = method
                result = self.rejected(rows, "test_or_projection_failed")
                self.assertNotIn(method, json.dumps(result))
                self.assertTrue(any(row["event"] == "projection_rejected" for row in result["events"]))

    def test_extra_native_fields_and_unhashable_event_do_not_leak(self):
        canary = "offline-raw-response-body-canary"
        rows = fixture()
        event(rows, "rpc_response")["raw_response"] = canary
        rows.insert(1, {"event": [], "raw_body": canary})
        result = self.rejected(rows, "test_or_projection_failed")
        self.assertNotIn(canary, json.dumps(result))
        self.assertEqual(sum(row["event"] == "projection_rejected" for row in result["events"]), 2)

    def test_duplicate_and_old_generation_responses_reject(self):
        rows = fixture()
        rows.insert(-1, copy.deepcopy(event(rows, "rpc_response")))
        self.rejected(rows, "response_missing")
        rows = fixture()
        event(rows, "rpc_response")["generation"] = FOREIGN
        self.rejected(rows, "event_generation_invalid")
        rows = fixture()
        event(rows, "rpc_response")["rpc_id"] = 99
        self.rejected(rows, "response_uncorrelated")

    def test_response_before_request_and_after_cleanup_reject(self):
        rows = fixture()
        response = event(rows, "rpc_response")
        rows.remove(response)
        rows.insert(2, response)
        self.rejected(rows, "response_before_request")
        rows = fixture()
        observation = event(rows, "mode_observation", FIRST)
        rows.remove(observation)
        rows.insert(-1, observation)
        self.rejected(rows, "event_outside_process")

    def test_cleanup_zero_without_diagnostics_never_completes_probe(self):
        rows = [row for row in fixture() if row["event"] != "diagnostic_observed"]
        self.rejected(rows, "diagnostic_missing")
        rows = fixture()
        event(rows, "process_cleanup")["cleanup_confirmed"] = False
        self.rejected(rows, "cleanup_unconfirmed")

    def test_native_input_nonzero_boolean_and_replay_reject(self):
        for value in (1, False):
            with self.subTest(value=value):
                rows = fixture()
                event(rows, "probe_finished")["native_inputs"] = value
                self.rejected(rows, "test_or_projection_failed")
        rows = fixture()
        event(rows, "resume_checked")["inputs_replayed"] = 1
        self.rejected(rows, "test_or_projection_failed")

    def test_policy_catalog_source_and_ceiling_overclaims_reject(self):
        changes = (("catalog_observation", "fixed_tool_closure_verified", True, "catalog_overclaimed"),
            ("source_observation", "all_configuration_sources_verified", True, "extra_source_observed"),
            ("probe_finished", "parent_permission_ceiling_verified", True, "finish_overclaimed"),
            ("launch_snapshot", "always_approve_requested", True, "requested_mode_invalid"))
        for name, field, value, code in changes:
            with self.subTest(field=field):
                rows = fixture()
                event(rows, name)[field] = value
                self.rejected(rows, code)

    def test_stdout_body_and_credential_shapes_only_export_hashes_counts(self):
        body = b"offline-stdout-body-canary " + b"sk-" + b"a" * 32 + b" account@example.invalid\n"
        summary = runner.stdout_summary(body)
        public = json.dumps(summary)
        self.assertNotIn("offline-stdout-body-canary", public)
        self.assertNotIn("account@example.invalid", public)
        self.assertEqual(summary["stdout_sha256"], runner.sha(body))
        self.assertEqual(summary["credential_shape_counts"]["api_key"], 1)
        self.assertEqual(summary["credential_shape_counts"]["email"], 1)
        result = self.audit(stdout=SUMMARY + body)
        self.assertFalse(result["execution_boundary_passed"])
        self.assertNotIn("offline-stdout-body-canary", json.dumps(result))

    def test_missing_summary_crash_or_failure_event_never_succeed(self):
        self.assertFalse(self.audit(stdout=b"native exit 0\n")["execution_boundary_passed"])
        self.assertFalse(self.audit(exit_code=101)["execution_boundary_passed"])
        rows = fixture()
        rows.insert(-1, {"event": "probe_failed", "reason_sha256": runner.sha(b"offline-error"), "reason_bytes": 13})
        result = self.rejected(rows, "probe_incomplete")
        self.assertTrue(any(row["event"] == "probe_failed" for row in result["events"]))


if __name__ == "__main__":
    unittest.main()
