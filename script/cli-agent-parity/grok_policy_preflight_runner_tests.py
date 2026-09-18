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
        "request_budget": 14, "allowed_methods_sha256": runner.sha(runner.canonical(list(runner.ALLOWED_METHODS))),
        "guard_before_native_write": True, "production_supervision": True,
        "candidate_source_is_exact_binary": False}]
    sequence = 0
    for generation, phase, opening in ((FIRST, "new", "session/new"), (SECOND, "resume", "session/load")):
        rows.append({"event": "launch_snapshot", "generation": generation, "phase": phase,
            "cli_version": runner.VERSION, "cli_sha256": runner.BINARY_SHA256,
            "profile_sha256": PROFILE, "config_sha256": CONFIG,
            "always_approve_requested": False, "auto_mode_requested": False})
        for method in ("initialize", "authenticate", opening, *runner.DIAGNOSTIC_METHODS):
            sequence += 1
            rows.append({"event": "rpc_sent", "generation": generation, "sequence": sequence,
                "method": method, "rpc_id": sequence, "request_bytes": 80,
                "request_sha256": runner.sha(method.encode()), "guard_checked_before_write": True})
            rows.append({"event": "rpc_response", "generation": generation, "sequence": sequence,
                "rpc_id": sequence, "status": "ok", "response_bytes": 50,
                "response_sha256": runner.sha(b"offline-response")})
            if method == "initialize":
                rows.append({"event": "auth_method_selected", "generation": generation,
                    "method_id": "cached_token", "advertised": True, "headless": True})
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
        for budget in (0, -1, 15, False):
            with self.subTest(budget=budget), self.assertRaisesRegex(runner.PreflightRejected, "request_budget_invalid"):
                runner.make_plan(PROFILE, CONFIG, budget)
        plan = runner.make_plan(PROFILE, CONFIG)
        self.assertEqual(plan["max_native_inputs"], 0)
        self.assertNotIn("session/prompt", plan["allowed_methods"])
        self.assertIn("authenticate", plan["allowed_methods"])

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


class NativeFailureDiagnosticTests(unittest.TestCase):
    def summary(self, value="OFFLINE_NOTIFICATION_BODY"):
        raw = runner.canonical(value)
        return {"type": "string", "bytes": len(raw), "sha256": runner.sha(raw)}

    def notification(self):
        return {"event": "native_notification_diagnostic", "generation": FIRST, "method": "session/update",
            "method_summary": self.summary(), "frame_type": "object", "params_type": "object", "id_present": False,
            "result_present": False, "error_present": False, "session_id": self.summary("OFFLINE_SESSION")}

    def insert(self, row):
        rows = fixture()
        rows.insert(rows.index(event(rows, "process_cleanup", FIRST)), row)
        return rows

    def global_catalog(self):
        return self.notification() | {"method": runner.GLOBAL_CATALOG_METHOD,
            "method_summary": {"type": "string", "bytes": 25,
                "sha256": runner.sha(runner.GLOBAL_CATALOG_METHOD.encode())},
            "session_id": {"type": "absent"}, "global_catalog_closed_empty": True}

    def test_four_notification_methods_and_typed_summaries_have_closed_public_projection(self):
        for method in runner.NOTIFICATION_METHODS - {runner.GLOBAL_CATALOG_METHOD, runner.SESSION_NOTIFICATION_METHOD}:
            row = self.notification() | {"method": method}
            public, faults = runner.project_events([row])
            self.assertEqual(public, [row])
            self.assertEqual(faults, [])
            self.assertNotIn("OFFLINE_NOTIFICATION_BODY", json.dumps(public))
            result = runner.audit_events(0, SUMMARY, self.insert(row))
            self.assertTrue(result["execution_boundary_passed"])
            self.assertEqual(result["policy"], runner.unknown_policy())

    def session_metadata(self):
        return self.notification() | {"method": runner.SESSION_NOTIFICATION_METHOD,
            "method_summary": {"type": "string", "bytes": 26,
                "sha256": runner.sha(runner.SESSION_NOTIFICATION_METHOD.encode())},
            "session_notification_metadata_only": True}

    def test_session_metadata_duplicates_do_not_replace_actual_rpc_or_prove_policy(self):
        rows = self.insert(self.session_metadata())
        rows.insert(rows.index(event(rows, "process_cleanup", FIRST)), self.session_metadata())
        result = runner.audit_events(0, SUMMARY, rows)
        self.assertTrue(result["execution_boundary_passed"])
        self.assertEqual(result["policy"], runner.unknown_policy())
        rows.remove(event(rows, "rpc_response", FIRST))
        self.assertFalse(runner.audit_events(0, SUMMARY, rows)["execution_boundary_passed"])

    def test_session_notification_requires_closed_metadata_proof_and_exact_method(self):
        for changes in ({"session_notification_metadata_only": False}, {"id_present": True},
                {"method_summary": self.summary("FOREIGN_METHOD")}, {"generation": FOREIGN}):
            self.assertFalse(runner.audit_events(0, SUMMARY, self.insert(self.session_metadata() | changes))["execution_boundary_passed"])
        for changes in ({"rawInput": {"tool": "DO_NOT_EXPORT"}}, {"session_notification_metadata_only": 1},
                {"method": "_x.ai/session_notification_unknown"}):
            public, faults = runner.project_events([self.session_metadata() | changes])
            self.assertEqual(public, [])
            self.assertEqual(len(faults), 1)

    def test_closed_empty_global_catalog_preserves_unknown_policy_without_native_session(self):
        row = self.global_catalog()
        self.assertEqual(runner.project_events([row]), ([row], []))
        result = runner.audit_events(0, SUMMARY, self.insert(row))
        self.assertTrue(result["execution_boundary_passed"])
        self.assertEqual(result["policy"], runner.unknown_policy())
        self.assertEqual(result["failure_diagnostics"]["notifications"][0]["session_id"], {"type": "absent"})
        self.assertNotIn("mcpServers", json.dumps(result))

    def test_global_catalog_repeats_and_rpc_interleaving_preserve_the_owned_response_ledger(self):
        rows = fixture()
        owned = next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == 2)
        before = copy.deepcopy([row for row in rows if row["event"] in {"rpc_sent", "rpc_response"}])
        rows.insert(rows.index(owned), self.global_catalog())
        rows.insert(rows.index(owned) + 1, self.global_catalog())
        result = runner.audit_events(0, SUMMARY, rows)
        self.assertTrue(result["execution_boundary_passed"])
        self.assertEqual([row for row in result["events"] if row["event"] in {"rpc_sent", "rpc_response"}], before)
        self.assertEqual(len(result["failure_diagnostics"]["notifications"]), 2)
        self.assertEqual(result["policy"], runner.unknown_policy())

    def test_global_catalog_cannot_replace_a_missing_pending_response_or_supply_an_id(self):
        rows = self.insert(self.global_catalog())
        rows.remove(next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == 2))
        result = runner.audit_events(0, SUMMARY, rows)
        self.assertFalse(result["execution_boundary_passed"])
        self.assertEqual(result["failure_code"], "response_missing")
        for changes in ({"rpc_id": 2}, {"rpc_id": "skills-reload"}, {"id_present": True}):
            result = runner.audit_events(0, SUMMARY, self.insert(self.global_catalog() | changes))
            self.assertFalse(result["execution_boundary_passed"])
            self.assertEqual(result["policy"], runner.unknown_policy())

    def test_unknown_notification_cannot_borrow_the_global_catalog_proof_field(self):
        for method in ("OFFLINE_UNKNOWN_METHOD", "_x.ai/mcp/servers_updated/extra", self.summary("OFFLINE_UNKNOWN_METHOD")):
            public, faults = runner.project_events([self.global_catalog() | {"method": method}])
            self.assertEqual(public, [])
            self.assertEqual(len(faults), 1)
            self.assertNotIn("OFFLINE_UNKNOWN_METHOD", json.dumps(faults))

    def test_global_catalog_rejects_claimed_session_and_explicit_null_session(self):
        for session in (self.summary("OFFLINE_NATIVE_SESSION"), {"type": "null", "bytes": 4, "sha256": runner.sha(b"null")}):
            row = self.global_catalog() | {"session_id": session}
            self.assertEqual(runner.project_events([row]), ([row], []))
            result = runner.audit_events(0, SUMMARY, self.insert(row))
            self.assertFalse(result["execution_boundary_passed"])
            self.assertEqual(result["failure_code"], "notification_unproved")

    def test_global_catalog_requires_a_true_boolean_closed_empty_shape_proof(self):
        row = self.global_catalog() | {"global_catalog_closed_empty": False}
        self.assertEqual(runner.project_events([row]), ([row], []))
        result = runner.audit_events(0, SUMMARY, self.insert(row))
        self.assertFalse(result["execution_boundary_passed"])
        self.assertEqual(result["failure_code"], "notification_unproved")
        for proof in (1, None, "true", {}):
            public, faults = runner.project_events([self.global_catalog() | {"global_catalog_closed_empty": proof}])
            self.assertEqual(public, [])
            self.assertEqual(len(faults), 1)

    def test_global_catalog_raw_nonempty_catalog_and_extra_metadata_only_export_hashes(self):
        for changes in ({"mcpServers": [{"name": "OFFLINE_PRIVATE_SERVER"}]},
                {"params": {"mcpServers": [], "sessionId": "OFFLINE_PRIVATE_SESSION"}},
                {"metadata": "OFFLINE_PRIVATE_METADATA"}):
            public, faults = runner.project_events([self.global_catalog() | changes])
            self.assertEqual(public, [])
            self.assertEqual(len(faults), 1)
            self.assertNotIn("OFFLINE_PRIVATE_", json.dumps(faults))

    def test_global_catalog_method_hash_size_and_type_must_match_the_fixed_utf8_name(self):
        summary = self.global_catalog()["method_summary"]
        for changes in ({"bytes": 26}, {"bytes": True}, {"bytes": runner.MAX_EVIDENCE_BYTES + 1},
                {"sha256": "a" * 64}, {"sha256": "INVALID_HASH"}, {"type": "object"}, {"type": []}):
            row = self.global_catalog() | {"method_summary": summary | changes}
            result = runner.audit_events(0, SUMMARY, self.insert(row))
            self.assertFalse(result["execution_boundary_passed"])
            self.assertEqual(result["policy"], runner.unknown_policy())

    def test_global_catalog_stale_generation_or_foreign_phase_cannot_pass(self):
        row = self.global_catalog() | {"generation": FOREIGN}
        result = runner.audit_events(0, SUMMARY, self.insert(row))
        self.assertFalse(result["execution_boundary_passed"])
        self.assertEqual(result["failure_code"], "event_generation_invalid")
        rows = fixture()
        rows.insert(rows.index(event(rows, "process_cleanup", FIRST)) + 1, self.global_catalog())
        result = runner.audit_events(0, SUMMARY, rows)
        self.assertFalse(result["execution_boundary_passed"])
        self.assertEqual(result["failure_code"], "event_outside_process")

    def test_global_catalog_proof_does_not_relax_other_notification_session_binding(self):
        row = self.notification() | {"session_id": {"type": "absent"}}
        result = runner.audit_events(0, SUMMARY, self.insert(row))
        self.assertFalse(result["execution_boundary_passed"])
        self.assertEqual(result["failure_code"], "notification_unproved")
        self.assertEqual(runner.project_events([self.notification() | {"global_catalog_closed_empty": True}])[0], [])

    def test_unknown_method_summary_survives_failure_without_becoming_interface_success(self):
        row = self.notification() | {"method": self.summary("OFFLINE_UNKNOWN_METHOD")}
        result = runner.audit_events(0, SUMMARY, self.insert(row))
        self.assertFalse(result["execution_boundary_passed"])
        self.assertEqual(result["failure_code"], "notification_unproved")
        self.assertEqual(result["failure_diagnostics"]["notifications"], [row])
        self.assertNotIn("OFFLINE_UNKNOWN_METHOD", json.dumps(result))

    def test_extra_fields_raw_session_body_and_invalid_summary_are_rejected_without_text(self):
        for patching in ({"body": "OFFLINE_BODY"}, {"session_id": "OFFLINE_BODY"},
                {"method_summary": self.summary() | {"body": "OFFLINE_BODY"}}, {"id_present": 1},
                {"method_summary": {"type": {}, "bytes": 1, "sha256": "a" * 64}}):
            public, faults = runner.project_events([self.notification() | patching])
            self.assertEqual(public, [])
            self.assertEqual(len(faults), 1)
            self.assertNotIn("OFFLINE_BODY", json.dumps(faults))

    def test_phase_failure_each_stage_cannot_be_overwritten_by_a_forged_success_finish(self):
        for stage in ("primary", "cleanup", "drain"):
            row = {"event": "phase_failure", "generation": FIRST, "stage": stage,
                "reason_bytes": 13, "reason_sha256": runner.sha(b"OFFLINE_CAUSE")}
            result = runner.audit_events(0, SUMMARY, self.insert(row))
            self.assertFalse(result["interface_investigation_completed"])
            self.assertEqual(result["failure_code"], "probe_incomplete")
            self.assertEqual(result["failure_diagnostics"]["phase_failures"], [row])
            self.assertNotIn("OFFLINE_CAUSE", json.dumps(result))

    def test_unknown_phase_stage_or_generation_is_strictly_rejected(self):
        row = {"event": "phase_failure", "generation": FIRST, "stage": "primary", "reason_bytes": 1,
            "reason_sha256": "a" * 64}
        for patching in ({"stage": "OFFLINE_STAGE"}, {"generation": "OFFLINE_GENERATION"}, {"reason_bytes": True}):
            public, faults = runner.project_events([row | patching])
            self.assertEqual(public, [])
            self.assertEqual(len(faults), 1)

    def test_drain_duplicate_is_correlated_and_unknown_old_or_new_response_cannot_pass(self):
        row = {"event": "drain_response_observed", "generation": FIRST, "rpc_id": 1, "duplicate": True,
            "response_bytes": 50, "response_sha256": runner.sha(b"OFFLINE_RESPONSE")}
        result = runner.audit_events(0, SUMMARY, self.insert(row))
        self.assertTrue(result["execution_boundary_passed"])
        self.assertEqual(result["failure_diagnostics"]["drain_responses"], [row])
        for patching in ({"rpc_id": 99}, {"generation": FOREIGN}, {"duplicate": False}):
            result = runner.audit_events(0, SUMMARY, self.insert(row | patching))
            self.assertFalse(result["execution_boundary_passed"])
            self.assertEqual(result["policy"], runner.unknown_policy())

    def test_cancelled_cleanup_null_is_preserved_and_never_normal_success(self):
        rows = fixture()
        cleanup = event(rows, "process_cleanup", FIRST)
        cleanup.update(exit_code=None, exit_reason="stop_requested")
        self.assertEqual(runner.project_events([cleanup]), ([cleanup], []))
        result = runner.audit_events(0, SUMMARY, rows)
        self.assertFalse(result["execution_boundary_passed"])
        self.assertFalse(result["interface_investigation_completed"])
        self.assertEqual(result["failure_code"], "cleanup_unconfirmed")
        self.assertEqual(event(result["events"], "process_cleanup", FIRST), cleanup)
        result = runner.audit_events(101, b"", rows)
        self.assertFalse(result["execution_boundary_passed"])
        self.assertEqual(result["failure_code"], "test_or_projection_failed")
        self.assertIsNone(event(result["events"], "process_cleanup", FIRST)["exit_code"])

    def test_cleanup_null_does_not_relax_numeric_or_boolean_projection(self):
        cleanup = event(fixture(), "process_cleanup", FIRST)
        for value in (True, False, "0", 0.0, -256, 256):
            public, faults = runner.project_events([cleanup | {"exit_code": value}])
            self.assertEqual(public, [])
            self.assertEqual(len(faults), 1)

    def test_failed_test_retains_notification_phase_and_drain_summaries_in_metadata(self):
        rows = self.insert(self.notification())
        rows.insert(-1, {"event": "phase_failure", "generation": FIRST, "stage": "drain",
            "reason_bytes": 13, "reason_sha256": runner.sha(b"OFFLINE_CAUSE")})
        result = runner.audit_events(101, b"", rows)
        self.assertFalse(result["execution_boundary_passed"])
        self.assertFalse(result["interface_investigation_completed"])
        self.assertEqual(len(result["failure_diagnostics"]["notifications"]), 1)
        self.assertEqual(len(result["failure_diagnostics"]["phase_failures"]), 1)
        self.assertEqual(runner.failure_diagnostics(result["events"]), result["failure_diagnostics"])


class OwnedRpcErrorDiagnosticTests(unittest.TestCase):
    def rows(self):
        rows = fixture()
        response = next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == 3)
        response["status"] = "rpc_error"
        raw = runner.canonical("OFFLINE_PRIVATE_MESSAGE")
        diagnostic = {"event": "rpc_error_diagnostic", "generation": FIRST, "sequence": 3, "rpc_id": 3,
            "method": "session/new", "owned_current_request": True, "error_code": -32602,
            "message": {"type": "string", "bytes": len(raw), "sha256": runner.sha(raw)},
            "data": {"type": "absent"}, "response_bytes": response["response_bytes"],
            "response_sha256": response["response_sha256"]}
        rows.insert(rows.index(response) + 1, diagnostic)
        return rows, diagnostic

    def test_owned_error_survives_failed_runner_without_policy_or_body(self):
        rows, diagnostic = self.rows()
        result = runner.audit_events(101, b"", rows)
        self.assertEqual(result["failure_diagnostics"]["rpc_errors"], [{"diagnostic": diagnostic, "ledger_correlated": True}])
        self.assertFalse(result["execution_boundary_passed"])
        self.assertEqual(result["policy"], runner.unknown_policy())
        self.assertNotIn("OFFLINE_PRIVATE_MESSAGE", json.dumps(result))
        self.assertEqual(runner.audit_events(0, SUMMARY, rows)["failure_code"], "interface_unavailable")
        for patching in ({"error_code": True}, {"error_code": 2 ** 31}, {"error_code": "-32602"},
                {"message": "OFFLINE_PRIVATE_MESSAGE"}, {"raw_error": "OFFLINE_PRIVATE_MESSAGE"}):
            self.assertEqual(runner.project_events([diagnostic | patching])[0], [])
        self.assertEqual(runner.project_events([diagnostic | {"error_code": None}])[0], [diagnostic | {"error_code": None}])

    def test_duplicate_old_outoforder_or_wrong_method_error_has_no_ledger_proof(self):
        for change in ("duplicate", "old_generation", "late", "wrong_method", "hash"):
            rows, diagnostic = self.rows()
            if change == "duplicate": rows.insert(rows.index(diagnostic), copy.deepcopy(diagnostic))
            elif change == "old_generation": diagnostic["generation"] = SECOND
            elif change == "late": rows.remove(diagnostic); rows.insert(-1, diagnostic)
            elif change == "wrong_method": diagnostic["method"] = "authenticate"
            else: diagnostic["response_sha256"] = "a" * 64
            result = runner.audit_events(101, b"", rows)
            self.assertTrue(all(not row["ledger_correlated"] for row in result["failure_diagnostics"]["rpc_errors"]))
            self.assertFalse(result["execution_boundary_passed"])


class CachedTokenHandshakeTests(unittest.TestCase):
    def rejected(self, rows, code):
        result = runner.audit_events(0, SUMMARY, rows)
        self.assertFalse(result["execution_boundary_passed"])
        self.assertFalse(result["interface_investigation_completed"])
        self.assertEqual(result["policy"], runner.unknown_policy())
        self.assertEqual(result["failure_code"], code)
        return result

    def test_two_phases_require_fourteen_owned_requests_and_cached_auth_selection(self):
        rows = fixture()
        result = runner.audit_events(0, SUMMARY, rows)
        self.assertTrue(result["execution_boundary_passed"])
        sent = [row for row in rows if row["event"] == "rpc_sent"]
        self.assertEqual(len(sent), runner.MAX_REQUESTS)
        self.assertEqual(runner.MAX_REQUESTS, 14)
        for generation, opening in ((FIRST, "session/new"), (SECOND, "session/load")):
            self.assertEqual([row["method"] for row in sent if row["generation"] == generation],
                ["initialize", "authenticate", opening, *runner.DIAGNOSTIC_METHODS])
            selection = event(rows, "auth_method_selected", generation)
            self.assertEqual(selection, {"event": "auth_method_selected", "generation": generation,
                "method_id": "cached_token", "advertised": True, "headless": True})
        self.assertEqual(result["policy"], runner.unknown_policy())

    def test_authentication_method_projection_is_closed_without_credential_arguments(self):
        selection = event(fixture(), "auth_method_selected")
        self.assertEqual(runner.project_events([selection]), ([selection], []))
        for patching in ({"method_id": "xai.api_key"}, {"method_id": "interactive"},
                {"params": {"token": "OFFLINE_PRIVATE_TOKEN"}}, {"sessionId": "OFFLINE_SESSION"}):
            public, faults = runner.project_events([selection | patching])
            self.assertEqual(public, [])
            self.assertEqual(len(faults), 1)
            self.assertNotIn("OFFLINE_", json.dumps(faults))

    def test_missing_duplicate_or_unadvertised_selection_cannot_complete_probe(self):
        rows = fixture()
        rows.remove(event(rows, "auth_method_selected", FIRST))
        self.rejected(rows, "auth_method_unproved")
        rows = fixture()
        selection = event(rows, "auth_method_selected", FIRST)
        rows.insert(rows.index(selection), copy.deepcopy(selection))
        self.rejected(rows, "auth_method_unproved")
        for field in ("advertised", "headless"):
            rows = fixture()
            event(rows, "auth_method_selected")[field] = False
            self.rejected(rows, "auth_method_unproved")

    def test_selection_must_follow_initialize_response_and_precede_auth_write(self):
        for destination in ("before_initialize", "after_auth_response"):
            rows = fixture()
            selection = event(rows, "auth_method_selected")
            rows.remove(selection)
            if destination == "before_initialize":
                rows.insert(rows.index(event(rows, "rpc_response")), selection)
            else:
                response = next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == 2)
                rows.insert(rows.index(response) + 1, selection)
            self.rejected(rows, "auth_sequence_invalid")

    def test_authentication_response_must_precede_new_and_load_write(self):
        for generation, rpc_id in ((FIRST, 2), (SECOND, 9)):
            rows = fixture()
            response = next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == rpc_id)
            rows.remove(response)
            opening = next(row for row in rows if row["event"] == "rpc_sent"
                and row["generation"] == generation and row["method"] in ("session/new", "session/load"))
            rows.insert(rows.index(opening) + 1, response)
            self.rejected(rows, "auth_sequence_invalid")

    def test_missing_auth_rpc_does_not_turn_matching_new_response_into_authentication(self):
        rows = fixture()
        rows = [row for row in rows if not (row["event"] in ("rpc_sent", "rpc_response") and row["rpc_id"] == 2)]
        for row in rows:
            if row["event"] in ("rpc_sent", "rpc_response") and row["rpc_id"] > 2:
                row["rpc_id"] -= 1
                row["sequence"] -= 1
        event(rows, "probe_finished")["protocol_request_count"] = 13
        self.rejected(rows, "method_sequence_invalid")

    def test_auth_error_response_is_retained_as_failure_with_unknown_policy(self):
        for status in ("rpc_error", "auth_required", "method_not_found", "timeout"):
            rows = fixture()
            response = next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == 2)
            response["status"] = status
            result = self.rejected(rows, "interface_unavailable")
            self.assertIn(response, result["events"])

    def test_duplicate_stale_or_reordered_auth_response_cannot_claim_new_pending(self):
        rows = fixture()
        response = next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == 2)
        rows.insert(rows.index(response), copy.deepcopy(response))
        self.rejected(rows, "response_missing")
        rows = fixture()
        response = next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == 9)
        response["generation"] = FIRST
        self.rejected(rows, "response_uncorrelated")
        rows = fixture()
        response = next(row for row in rows if row["event"] == "rpc_response" and row["rpc_id"] == 2)
        rows.remove(response)
        rows.insert(rows.index(event(rows, "rpc_sent")), response)
        self.rejected(rows, "response_uncorrelated")

    def test_old_generation_auth_selection_and_relaxed_booleans_are_rejected(self):
        rows = fixture()
        event(rows, "auth_method_selected")["generation"] = FOREIGN
        self.rejected(rows, "event_generation_invalid")
        for patching in ({"advertised": 1}, {"headless": "true"}, {"generation": "OFFLINE_OLD_GENERATION"}):
            selection = event(fixture(), "auth_method_selected") | patching
            self.assertEqual(runner.project_events([selection])[0], [])

    def test_fourteen_rpc_budget_rejects_stale_twelve_claim_and_fifteenth(self):
        rows = fixture()
        event(rows, "probe_started")["request_budget"] = 12
        self.rejected(rows, "request_ledger_invalid")
        self.assertEqual(runner.make_plan(PROFILE, CONFIG, 14)["max_protocol_requests"], 14)
        with self.assertRaisesRegex(runner.PreflightRejected, "request_budget_invalid"):
            runner.make_plan(PROFILE, CONFIG, 15)
        self.assertNotIn("session/prompt", runner.make_plan(PROFILE, CONFIG)["allowed_methods"])


if __name__ == "__main__":
    unittest.main()
