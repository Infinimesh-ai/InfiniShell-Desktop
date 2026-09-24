"""SDK 来源探针的离线安全边界回归；不请求真实模型或执行真实 CLI。"""

import argparse
from contextlib import ExitStack
import io
import json
import os
from pathlib import Path
import runpy
import socket
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run_grok_sdk_origin_probe as runner


def evidence(state="verified_native_fields"):
    return [
        {"event": "probe_started", "scope": runner.SCOPE, "max_native_inputs": 1,
            "origin_verification": "unknown", "credential_files_read_by_probe": False,
            "public_product_gate_open": False},
        {"event": "registration_received", "framework": "sdk_mcp", "protocol_version": "2025-11-25",
            "initialization_mode": "legacy_initialize"},
        {"event": "sdk_request_received", "sequence": 1, "outer_id": 1, "inner_id": 0,
            "mcp_method": "tools/call", "outer_keys": ["jsonrpc", "id", "method", "params"],
            "params_keys": ["serverId", "message"], "inner_keys": ["id", "method", "params"],
            "tool_params_keys": ["name", "arguments"],
            "metadata_keys": {"outer": [], "sdk_params": [], "mcp_envelope": [], "mcp_params": []}},
        {"event": "probe_finished", "scope": runner.SCOPE, "passed": True, "native_inputs": 1,
            "sdk_request_count": 1, "inspect_call_count": 1, "origin_verification": state,
            "full_native_origin_fields_observed": state == "verified_native_fields",
            "native_origin_ledger_relation_verified": state == "verified_native_fields",
            "candidate_mapping_only": state == "candidate_mapping_only", "no_side_effects": True,
            "approval_all_denied": True, "permission_request_count": 0, "unexpected_tool_count": 0,
            "approval_allow_count": 0, "approval_deny_count": 0, "approval_duplicate_count": 0,
            "native_contract_verified": True, "discovery_native_contract_verified": False,
            "search_allow_count": 0, "inspect_allow_count": 0,
            "cleanup_confirmed": True, "public_product_gate_open": False, "submitted_input_count": 1,
            "initialization_count": 1, "tools_list_count": 1, "cleanup_receipt_read": True,
            "discovery_count": 0, "negotiatedProtocolVersion": "2025-11-25", "servedToolNames": ["inspect"],
            "transport_closed": True, "no_project_files": True, "parent_permission_ceiling_verified": False},
    ]


def modern_evidence(state="verified_native_fields"):
    events = evidence(state)
    registration = dict(events[1], protocol_version="2026-07-28", initialization_mode="modern_discover")
    requests = [dict(events[2], sequence=number + 1, outer_id=number, inner_id=number,
        mcp_method=method, wire_method="_x.ai/mcp/sdk_call",
        requested_protocol_version="2026-07-28", protocol_version_carrier="params._meta",
        metadata_schema_valid=True, tool_params_keys=(["_meta", "name", "arguments"]
            if method == "tools/call" else ["_meta"]),
        metadata_keys={"outer": [], "sdk_params": [], "mcp_envelope": [], "mcp_params": [
            "io.modelcontextprotocol/protocolVersion", "io.modelcontextprotocol/clientInfo",
            "io.modelcontextprotocol/clientCapabilities"]})
        for number, method in enumerate(("server/discover", "tools/list", "tools/call"))]
    finish = dict(events[-1], sdk_request_count=3, initialization_count=0, discovery_count=1,
        negotiatedProtocolVersion="2026-07-28")
    return [events[0], requests[0], registration, requests[1], requests[2], finish]



def calibrated_three_option_request():
    return {"jsonrpc": "2.0", "id": 71, "method": "session/request_permission", "params": {
        "sessionId": "01a0af68-fdc6-7b92-9918-4d98e45b1ee0",
        "toolCall": {"toolCallId": "native-inspect-1", "kind": "other", "rawInput": {
            "tool_name": "infinishell-sdk-origin-probe__inspect", "tool_input": {}, "variant": "UseTool"}},
        "options": [{"optionId": "always-allow", "kind": "allow_always", "name": "OFFLINE_PERMANENT_LABEL_CANARY"},
            {"optionId": "allow-once", "kind": "allow_once", "name": "OFFLINE_ONCE_LABEL_CANARY"},
            {"optionId": "reject-once", "kind": "reject_once", "name": "OFFLINE_REJECT_LABEL_CANARY"}]}}


def request_fingerprint(request):
    return runner.sha(json.dumps(request, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8"))

def evidence_with_exact_approval(state="verified_native_fields", retries=0):
    events = evidence(state)
    session = "01a0af68-fdc6-7b92-9918-4d98e45b1ee0"
    prompt = "9d1a087e-2fd5-430f-a453-ecc8fce96c42"
    tool = "native-inspect-1"
    events.insert(-1, {"event": "native_tool_update_received", "session_id": session,
        "prompt_id": prompt, "tool_call_id": tool, "probe_tool": True,
        "discovery_tool": False, "calibrated_tool_kind": "inspect", "raw_input_present": True, "raw_input_sha256": "a" * 64})
    approval = {"event": "probe_approval_observed", "request_id": 71, "session_id": session,
        "prompt_id": prompt, "tool_call_id": tool, "decision": "allow_once", "duplicate": False,
        "native_contract_verified": True, "discovery_native_contract_verified": False,
        "approval_contract_verified": True, "calibrated_tool_kind": "inspect",
        "session_id_matches": True, "prompt_id_matches": True,
        "tool_call_id_in_native_ledger": True, "raw_input_sha256": "a" * 64,
        "selected_option_id": "allow-once",
        "request_payload_sha256": request_fingerprint(calibrated_three_option_request())}
    events.insert(-1, approval)
    for _ in range(retries):
        events.insert(-1, dict(approval, duplicate=True))
    events[-1].update(approval_allow_count=1, search_allow_count=0, inspect_allow_count=1,
        approval_deny_count=0, approval_duplicate_count=retries,
        permission_request_count=1 + retries, approval_all_denied=False,
        native_session_id=session, native_prompt_id=prompt)
    return events


def calibrated_search_request():
    request = calibrated_three_option_request()
    request["id"] = 81
    request["params"]["toolCall"]["toolCallId"] = "native-search-1"
    request["params"]["toolCall"]["rawInput"] = {
        "limit": 5, "query": "infinishell-sdk-origin-probe inspect", "variant": "SearchTool"}
    return request


def evidence_with_search_and_inspect(state="verified_native_fields", search_retries=0, inspect_retries=0):
    events = evidence_with_exact_approval(state, retries=inspect_retries)
    request = calibrated_search_request()
    search = dict(events[4], request_id=81, tool_call_id="native-search-1", calibrated_tool_kind="search_discovery",
        native_contract_verified=False, discovery_native_contract_verified=True,
        raw_input_sha256=request_fingerprint(request["params"]["toolCall"]["rawInput"]),
        request_payload_sha256=request_fingerprint(request))
    ledger = dict(events[3], tool_call_id="native-search-1", calibrated_tool_kind="search_discovery",
        probe_tool=False, discovery_tool=True, raw_input_sha256=search["raw_input_sha256"])
    events[3:3] = [ledger, search] + [dict(search, duplicate=True) for _ in range(search_retries)]
    events[-1].update(approval_allow_count=2,search_allow_count=1,inspect_allow_count=1,
        approval_duplicate_count=search_retries+inspect_retries,
        permission_request_count=2+search_retries+inspect_retries,discovery_native_contract_verified=True)
    return events


class ProbeRunnerTests(unittest.TestCase):
    def test_native_tool_name_diagnostics_preserve_only_type_digest_and_closed_kind(self):
        for kind in runner.NATIVE_TOOL_KINDS:
            with self.subTest(kind=kind):
                event = {"event": "native_tool_update_received", "native_tool_name_type": "string",
                    "native_tool_name_sha256": "a" * 64, "native_tool_kind": kind, "probe_tool": False}
                self.assertEqual(runner.public_events([event]), [event])

    def test_native_tool_name_diagnostics_hash_arbitrary_credential_shapes(self):
        canary = "Bearer OFFLINE_NATIVE_NAME_SECRET_CANARY"
        for key in ("native_tool_name_type", "native_tool_name_sha256", "native_tool_kind"):
            for value in (canary, {"name": canary}, [canary]):
                with self.subTest(key=key, shape=type(value).__name__):
                    projected = runner.projection({key: value})
                    self.assertNotIn(canary, json.dumps(projected))
        for key in ("native_tool_name_type", "native_tool_name_sha256", "native_tool_kind"):
            projected = runner.projection({key: canary})
            self.assertIn("sha256", projected[key])
        for value in ("ready", "initialize", "probe_inspect/PRIVATE_CANARY", True, 1, None):
            projected = runner.projection({"native_tool_kind": value})
            self.assertIsInstance(projected["native_tool_kind"], dict)

    def test_native_tool_name_kind_cannot_replace_registration_or_origin(self):
        events = evidence()
        events.insert(2, {"event": "native_tool_update_received", "native_tool_name_type": "string",
            "native_tool_name_sha256": "a" * 64, "native_tool_kind": "ToolSearch", "probe_tool": False})
        events[-1].update(passed=False, sdk_request_count=0, initialization_count=0,
            tools_list_count=0, inspect_call_count=0, unexpected_tool_count=1, origin_verification="unknown")
        observed = self.observe(events)
        self.assertFalse(observed["probe_passed"])
        self.assertFalse(observed["native_origin_verified"])
        self.assertEqual(observed["origin_verification"], "unknown")

    def test_requested_sdk_version_preserves_only_supported_protocol_versions(self):
        self.assertEqual(runner.projection({"requested_protocol_version": "2026-07-28"}),
                         {"requested_protocol_version": "2026-07-28"})
        self.assertIsInstance(runner.projection({"requested_protocol_version": "2024-11-05"})["requested_protocol_version"], dict)
        projected = runner.projection({"requested_protocol_version": "dummy-private-version-value"})
        self.assertNotIn("dummy-private-version-value", json.dumps(projected))
        self.assertEqual(projected["requested_protocol_version"]["type"], "string")

    def test_initialize_capability_observation_keeps_only_safe_fields(self):
        observed = {"event": "probe_initialize_observed", "sdk_capability_present": True,
            "sdk_capability_type": "boolean", "sdk_capability_enabled": True}
        self.assertEqual(runner.public_events([observed]), [observed])
        projected = runner.projection({"sdk_capability_type": "PRIVATE_CAPABILITY_CANARY",
            "sdk_capability_present": "PRIVATE_PRESENT_CANARY",
            "sdk_capability_enabled": "PRIVATE_ENABLED_CANARY"})
        self.assertNotIn("PRIVATE_", json.dumps(projected))
        self.assertEqual(projected["sdk_capability_type"]["type"], "string")

    def test_sdk_wire_methods_preserve_only_two_exact_protocol_markers(self):
        for method in ("x.ai/mcp/sdk_call", "_x.ai/mcp/sdk_call"):
            with self.subTest(method=method):
                self.assertEqual(runner.projection({"wire_method": method}), {"wire_method": method})
        for method in ("Bearer OFFLINE_WIRE_SECRET_CANARY", "initialize", "_x.ai/mcp/sdk_call/PRIVATE_CANARY"):
            with self.subTest(method=method):
                projected = runner.projection({"wire_method": method})
                self.assertEqual(projected["wire_method"]["type"], "string")
                self.assertNotIn(method, json.dumps(projected))

    def test_mcp_status_observation_preserves_only_fixed_diagnostic_fields(self):
        observed = {"event": "native_mcp_status_observed", "notification_method": "_x.ai/mcp/server_status",
            "session_known": True, "session_id_present": True, "session_id_type": "string",
            "session_id_matches": True, "probe_server_name_matches": True, "name_type": "string",
            "params_type": "object", "mcp_status": "ready", "mcp_reason": "initialized",
            "detail_present": True, "detail_type": "string", "detail_sha256": "a" * 64}
        self.assertEqual(runner.public_events([observed]), [observed])
        for method in runner.MCP_STATUS_METHODS:
            self.assertEqual(runner.projection({"notification_method": method}), {"notification_method": method})
        for marker in ("Bearer OFFLINE_STATUS_SECRET_CANARY", "initialize"):
            projected = runner.projection({"notification_method": marker, "mcp_status": marker,
                "mcp_reason": marker, "name_type": marker, "detail_sha256": marker})
            self.assertNotIn(marker, json.dumps(projected))
            self.assertTrue(all(value["type"] == "string" for value in projected.values()))

    def test_mcp_counter_projection_rejects_bool_negative_and_unbounded_values(self):
        for key in runner.COUNTER_KEYS:
            for value in (True, -1, 1.5, 2 ** 63, "OFFLINE_COUNTER_SECRET_CANARY"):
                with self.subTest(key=key, value=value):
                    self.assertEqual(runner.projection({key: value}), {key: None})
            self.assertEqual(runner.projection({key: 2 ** 63 - 1}), {key: 2 ** 63 - 1})
        self.assertEqual(runner.projection({"detail_type": "absent_or_null"}), {"detail_type": "absent_or_null"})

    def test_unknown_mcp_status_digest_is_preserved_without_detail_text(self):
        observed = {"event": "native_mcp_status_observed", "mcp_status": {"type": "string", "sha256": "b" * 64},
            "mcp_reason": {"type": "array", "sha256": "c" * 64}, "detail_present": True,
            "detail_type": "object", "detail_sha256": "d" * 64}
        self.assertEqual(runner.public_events([observed]), [observed])
        projected = runner.projection(dict(observed, detail="Bearer OFFLINE_DETAIL_SECRET_CANARY"))
        self.assertNotIn("OFFLINE_DETAIL_SECRET_CANARY", json.dumps(projected))

    def test_server_discover_classification_stays_failed_without_registration(self):
        events = evidence()
        events[2]["mcp_method"] = "server/discover"
        events[-1].update(passed=False, initialization_count=0, tools_list_count=0,
            inspect_call_count=0, origin_verification="unknown", unexpected_tool_count=1)
        projected = runner.public_events(events)
        self.assertEqual(projected[2]["mcp_method"], "server/discover")
        observed = self.observe(events)
        self.assertFalse(observed["probe_passed"])
        self.assertFalse(observed["native_origin_verified"])
        self.assertEqual(observed["origin_verification"], "unknown")

    def test_ready_mcp_status_cannot_replace_registration_or_sdk_origin(self):
        events = evidence("missing_native_origin")
        events.insert(2, {"event": "native_mcp_status_observed", "mcp_status": "ready",
            "probe_server_name_matches": True, "session_known": True, "session_id_matches": True})
        self.assertFalse(self.observe(events)["probe_passed"])
        self.assertFalse(self.observe(events)["native_origin_verified"])
        events[-1].update(passed=False, initialization_count=0, inspect_call_count=0,
            origin_verification="unknown")
        self.assertFalse(self.observe(events)["probe_passed"])

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="sdk-origin-runner-tests-")
        self.root = Path(self.temporary.name).resolve()
        self.root.chmod(0o700)
        self.home = self.root / "original-auth"
        self.home.mkdir(mode=0o700)
        self.native = self.root / "native"
        self.binary = self.root / "tests"
        self.supervisor = self.root / "supervisor"
        for path in (self.native, self.binary, self.supervisor):
            path.write_bytes(b"offline fixture")
        self.args = argparse.Namespace(test_binary=self.binary, grok=self.native,
            supervisor=self.supervisor, official_grok_home=self.home, max_native_inputs=1,
            timeout=450, output=self.root / "probe.ndjson")

    def tearDown(self):
        self.temporary.cleanup()

    def validate(self):
        with patch.object(runner.sys, "platform", "darwin"), patch.object(runner.shared, "digest", return_value=runner.shared.BINARY_SHA256):
            runner.validate_paths(self.args)

    def observe(self, events=None, code=0, output="test result: ok. 1 passed; 0 failed"):
        return runner.probe_observation(code, output, evidence() if events is None else events)

    def test_fixed_native_input_and_deadline_limits(self):
        self.validate()
        for budget in (0, 2):
            self.args.max_native_inputs = budget
            with self.assertRaises(ValueError):
                self.validate()
        self.args.max_native_inputs = 0
        with patch.object(runner.sys, "platform", "darwin"), patch.object(runner.shared, "digest", return_value=runner.shared.BINARY_SHA256):
            runner.validate_paths(self.args, expected_inputs=0)
        self.args.max_native_inputs = 1
        for deadline in (29, 451, 900):
            self.args.timeout = deadline
            with self.assertRaises(ValueError):
                self.validate()

    def test_fixed_binary_hash_is_required(self):
        with patch.object(runner.sys, "platform", "darwin"), patch.object(runner.shared, "digest", return_value="0" * 64):
            with self.assertRaises(ValueError):
                runner.validate_paths(self.args)

    def test_source_auth_directory_must_be_private(self):
        self.home.chmod(0o755)
        with self.assertRaises(ValueError):
            self.validate()

    def test_symlink_native_is_rejected(self):
        alias = self.root / "native-alias"
        alias.symlink_to(self.native)
        self.args.grok = alias
        with self.assertRaises(ValueError):
            self.validate()

    def test_source_auth_directory_cannot_be_output(self):
        self.args.output = self.home / "probe.ndjson"
        with self.assertRaises(ValueError):
            self.validate()

    def test_fresh_artifact_reservation_rolls_back_without_overwriting(self):
        existing = self.args.output.with_suffix(".metadata.json")
        existing.write_text("保留", encoding="utf-8")
        with self.assertRaises(FileExistsError):
            runner.reserve_artifacts(self.args.output)
        self.assertFalse(self.args.output.exists())
        self.assertEqual(existing.read_text(encoding="utf-8"), "保留")

    def test_all_public_artifacts_are_private(self):
        runner.reserve_artifacts(self.args.output)
        for path in runner.artifacts(self.args.output):
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    def test_private_raw_rejects_symlink_and_public_mode(self):
        raw = self.root / "raw.ndjson"
        raw.write_text('{}\n', encoding="utf-8")
        raw.chmod(0o644)
        with self.assertRaises(ValueError):
            runner.private_events(raw)
        raw.chmod(0o600)
        alias = self.root / "raw-alias"
        alias.symlink_to(raw)
        with self.assertRaises(OSError):
            runner.private_events(alias)

    def test_private_raw_rejects_arrays_and_size_overrun(self):
        raw = self.root / "raw.ndjson"
        raw.touch(mode=0o600)
        raw.write_text('[]\n', encoding="utf-8")
        with self.assertRaises(ValueError):
            runner.private_events(raw)
        with patch.object(runner, "MAX_EVIDENCE_BYTES", 1):
            with self.assertRaises(ValueError):
                runner.private_events(raw)

    def test_unknown_origin_is_not_preset_to_missing(self):
        self.assertEqual(self.observe([])["origin_verification"], "unknown")
        self.assertFalse(self.observe([])["native_origin_verified"])

    def test_missing_origin_fails_whole_probe_without_verified_relation(self):
        observed = self.observe(evidence("missing_native_origin"))
        self.assertFalse(observed["probe_passed"])
        self.assertEqual(observed["origin_verification"], "missing_native_origin")
        self.assertFalse(observed["native_origin_verified"])

    def test_candidate_relation_does_not_open_verified_origin(self):
        observed = self.observe(evidence("candidate_mapping_only"))
        self.assertFalse(observed["probe_passed"])
        self.assertFalse(observed["native_origin_verified"])

    def test_verified_origin_requires_native_ledger_and_complete_fields(self):
        events = evidence("verified_native_fields")
        self.assertTrue(self.observe(events)["native_origin_verified"])
        for field in ("full_native_origin_fields_observed", "native_origin_ledger_relation_verified"):
            changed = json.loads(json.dumps(events))
            changed[-1][field] = False
            self.assertFalse(self.observe(changed)["native_origin_verified"])
        events[-1]["candidate_mapping_only"] = True
        self.assertFalse(self.observe(events)["native_origin_verified"])

    def test_bool_cannot_satisfy_integer_counter(self):
        events = evidence()
        for key in ("native_inputs", "sdk_request_count", "inspect_call_count", "permission_request_count"):
            changed = json.loads(json.dumps(events))
            changed[-1][key] = True
            self.assertFalse(self.observe(changed)["probe_passed"])

    def test_any_approval_or_unexpected_tool_fails_probe(self):
        for key in ("permission_request_count", "unexpected_tool_count"):
            events = evidence()
            events[-1][key] = 1
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_duplicate_finish_and_start_are_rejected(self):
        events = evidence()
        for event in (events[0], events[-1]):
            self.assertFalse(self.observe(events + [event])["probe_passed"])

    def test_counter_mismatch_and_registration_missing_fail(self):
        events = evidence()
        events[-1]["sdk_request_count"] = 2
        self.assertFalse(self.observe(events)["probe_passed"])
        self.assertFalse(self.observe([event for event in evidence() if event["event"] != "registration_received"])["probe_passed"])

    def test_test_failure_cleanup_failure_and_scope_mismatch_are_preserved(self):
        self.assertFalse(self.observe(code=101)["probe_passed"])
        self.assertFalse(self.observe(output="test result: FAILED. 1 passed; 0 failed")["probe_passed"])
        for key, value in (("cleanup_confirmed", False), ("scope", "other"),
                ("public_product_gate_open", True), ("origin_verification", "unrecognized"), ("passed", False)):
            events = evidence()
            events[-1][key] = value
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_public_projection_never_records_arbitrary_ids_values_or_keys(self):
        secret = "PRIVATE_SECRET_CANARY_92"
        events = evidence()
        events[2]["outer_id"] = secret
        events[2]["outer_keys"].append(secret)
        events[2]["params_keys"].append("sessionId")
        events[2][secret] = {"arguments": {"name": secret, "id": 111222333}}
        events[-1]["failure_reason"] = secret
        public = runner.public_events(events)
        encoded = json.dumps(public)
        self.assertNotIn(secret, encoded)
        self.assertNotIn("111222333", encoded)
        self.assertIn("sessionId", encoded)
        self.assertIn(runner.sha(secret.encode()), encoded)

    def test_projected_native_id_hash_and_origin_types_are_preserved(self):
        value = {"type": "string", "sha256": "d" * 64}
        event = {"event": "sdk_request_received", "outer_id": value,
            "origin_fields": [{"carrier": "sdk_params_meta",
                "fields": {"sessionId": "absent_or_null", "promptId": "string", "toolCallId": "string"}}]}
        public = runner.public_events([event])[0]
        self.assertEqual(public["outer_id"], value)
        self.assertEqual(public["origin_fields"], event["origin_fields"])

    def test_unknown_events_are_hash_only(self):
        public = runner.public_events([{"event": "secret_event", "raw_input": {"secret": "secret_body"}}])
        self.assertEqual(public[0]["event"], "unrecognized_private_event")
        self.assertNotIn("secret", json.dumps(public))

    def test_malformed_event_and_origin_states_preserve_failure(self):
        events = evidence()
        events[-1]["origin_verification"] = {"untrusted": "private"}
        observed = self.observe(events)
        self.assertFalse(observed["probe_passed"])
        self.assertEqual(observed["origin_verification"], "unknown")
        self.assertEqual(runner.public_events([{"event": {"private": "value"}}])[0]["event"], "unrecognized_private_event")

    def test_official_environment_drops_api_credentials_and_custom_endpoints(self):
        with patch.dict(os.environ, {"ANTHROPIC_API_KEY": "secret", "OPENAI_API_KEY": "secret",
                "GROK_API_KEY": "secret", "GROK_BASE_URL": "https://private.invalid", "HTTPS_PROXY": "bad"}):
            environment = runner.official.official_environment(self.root, 1234)
        self.assertNotIn("secret", json.dumps(environment))
        self.assertNotIn("private.invalid", json.dumps(environment))
        self.assertNotIn("INFINISHELL_GROK_BYOK_KEY", environment)
        self.assertEqual(environment["HTTPS_PROXY"], "http://127.0.0.1:1234")
        self.assertEqual(environment["GROK_CLAUDE_HOOKS_ENABLED"], "0")
        self.assertEqual(environment["GROK_CLAUDE_MCPS_ENABLED"], "0")

    def test_wrapper_retains_native_arguments_hash_and_adds_project_deny(self):
        (self.root / "home/.grok").mkdir(parents=True, mode=0o700)
        wrapper, settings = runner.prepare_probe_native(self.root, self.native, self.home, 1234)
        code = wrapper.read_text(encoding="utf-8")
        compile(code, str(wrapper), "exec")
        self.assertIn(runner.project_deny(self.root).__repr__(), code)
        self.assertIn(runner.shared.BINARY_SHA256, code)
        self.assertIn("str(native),*args", code)
        self.assertIn("args==['agent','--no-leader','stdio']", code)
        self.assertNotIn("--leader-socket", code)
        self.assertNotIn("network-bind network-inbound", code)
        self.assertEqual(stat.S_IMODE(wrapper.stat().st_mode), 0o700)
        self.assertIn('action = "ask"', settings.read_text(encoding="utf-8"))
        self.assertNotIn('action = "allow"', settings.read_text(encoding="utf-8"))

    def test_wrapper_boundary_change_is_fail_closed(self):
        wrapper = self.root / "wrapper"
        wrapper.write_text("changed protocol\n", encoding="utf-8")
        with patch.object(runner.official, "prepare_native", return_value=(wrapper, self.root / "settings")):
            with self.assertRaises(ValueError):
                runner.prepare_probe_native(self.root, self.native, self.home, 1234)

    def test_wrapper_executes_only_exact_direct_arguments(self):
        (self.root / "home/.grok").mkdir(parents=True, mode=0o700)
        with patch.object(runner.shared, "BINARY_SHA256", runner.shared.digest(self.native)):
            wrapper, _settings = runner.prepare_probe_native(self.root, self.native, self.home, 1234)
        # 假原生文件只核对 argv；execv 被拦截，不执行 CLI、沙箱或网络请求。
        with patch.object(runner.os, "execv") as invocation:
            with patch.object(runner.sys, "argv", [str(wrapper), "agent", "--no-leader", "stdio"]):
                runpy.run_path(str(wrapper), run_name="__main__")
        self.assertEqual(invocation.call_count, 1)
        self.assertEqual(invocation.call_args.args[0], "/usr/bin/sandbox-exec")
        self.assertEqual(invocation.call_args.args[1][-4:], [str(self.native), "agent", "--no-leader", "stdio"])
        launch = json.loads((self.root / "wrapper-audit.ndjson").read_text(encoding="utf-8"))
        self.assertEqual(launch["kind"], "direct_agent")
        self.assertTrue(launch["arguments_unchanged"])
        self.assertIsNone(launch["private_socket"])

    def test_wrapper_rejects_leader_and_nonexact_direct_arguments(self):
        (self.root / "home/.grok").mkdir(parents=True, mode=0o700)
        with patch.object(runner.shared, "BINARY_SHA256", runner.shared.digest(self.native)):
            wrapper, _settings = runner.prepare_probe_native(self.root, self.native, self.home, 1234)
        for arguments in (["agent", "stdio", "--leader-socket", str(self.root / "tmp/leader.sock")],
                ["agent", "stdio"], ["agent", "stdio", "--no-leader"],
                ["agent", "--no-leader", "stdio", "--always-approve"]):
            with self.subTest(arguments=arguments):
                with patch.object(runner.os, "execv") as invocation:
                    with patch.object(runner.sys, "argv", [str(wrapper), *arguments]):
                        with self.assertRaises(SystemExit) as failed:
                            runpy.run_path(str(wrapper), run_name="__main__")
                self.assertEqual(failed.exception.code, 94)
                self.assertEqual(invocation.call_count, 0)
                self.assertFalse((self.root / "wrapper-audit.ndjson").exists())

    def test_wrapper_duplicate_leader_boundary_is_fail_closed(self):
        (self.root / "home/.grok").mkdir(parents=True, mode=0o700)
        wrapper, settings = runner.official.prepare_native(self.root, self.native, self.home, 1234)
        code = wrapper.read_text(encoding="utf-8")
        start = code.index("elif len(args)==4")
        end = code.index("else:raise SystemExit(94)", start)
        wrapper.write_text(code + code[start:end], encoding="utf-8")
        with patch.object(runner.official, "prepare_native", return_value=(wrapper, settings)):
            with self.assertRaises(ValueError):
                runner.prepare_probe_native(self.root, self.native, self.home, 1234)

    def test_canary_accepts_only_permission_denied_not_bool_true(self):
        for result in (subprocess.CompletedProcess([], 0, '{"project_write": true}'),
                subprocess.CompletedProcess([], 1, '{"project_write": 1}'),
                subprocess.CompletedProcess([], 0, '{"project_write": 13}')):
            with patch.object(runner.subprocess, "run", return_value=result):
                with self.assertRaises(ValueError):
                    runner.project_write_canary(self.root, self.home, 1234, 10)
        with patch.object(runner.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, '{"project_write": 1}')) as invocation:
            self.assertEqual(runner.project_write_canary(self.root, self.home, 1234, 10), {"project_write": 1})
        self.assertIn(runner.project_deny(self.root), invocation.call_args.args[0][2])

    def test_opaque_auth_copy_does_not_parse_credentials(self):
        source = self.home / "auth.json"
        source.touch(mode=0o600)
        source.write_bytes(b"opaque-not-json-offline")
        target = self.root / "copied"
        target.mkdir(mode=0o700)
        copied = runner.official.copy_private_auth(self.home, target)
        self.assertEqual(copied.read_bytes(), source.read_bytes())
        self.assertEqual(stat.S_IMODE(copied.stat().st_mode), 0o600)

    def test_tunnel_budget_restores_even_when_body_fails(self):
        previous = runner.official.MAX_TUNNELS
        with self.assertRaises(ValueError):
            with runner.bounded_tunnel(30):
                self.assertEqual(runner.official.MAX_TUNNELS, 32)
                raise ValueError("离线中断")
        self.assertEqual(runner.official.MAX_TUNNELS, previous)

    def test_thirty_third_tls_connect_is_denied_before_upstream_connection(self):
        self.assert_tls_connection_budget(None, 32)

    def test_explicit_six_process_budget_denies_sixty_fifth_connect(self):
        self.assert_tls_connection_budget(64, 64)

    def assert_tls_connection_budget(self, budget, expected):
        upstream_peers = []
        relays = []

        def connect(_host):
            upstream, peer = socket.socketpair()
            upstream_peers.append(peer)
            relays.append(upstream)
            return upstream

        with patch.object(runner.official.OfficialTunnel, "connect", side_effect=connect) as invocation:
            options = {} if budget is None else {"connection_budget": budget}
            with runner.bounded_tunnel(30, **options) as tunnel:
                port = tunnel.start()
                try:
                    for index in range(expected + 1):
                        client = socket.create_connection(("127.0.0.1", port), timeout=3)
                        try:
                            client.sendall(b"CONNECT cli-chat-proxy.grok.com:443 HTTP/1.1\r\nHost: cli-chat-proxy.grok.com\r\n\r\n")
                            response = client.recv(4096)
                            self.assertIn(b" 200 " if index < expected else b" 403 ", response)
                        finally:
                            client.close()
                    self.assertEqual(invocation.call_count, expected)
                    self.assertEqual(tunnel.forwarded, expected)
                    self.assertIn({"event": "official_connect_budget_rejected", "host": "cli-chat-proxy.grok.com"}, tunnel.events)
                finally:
                    for peer in upstream_peers:
                        peer.close()
        self.assertTrue(all(connection.fileno() == -1 for connection in relays))

    def mocked_run(self, *, cleanup=True, budget_rejected=False, canary_error=False, bad_version=False,
                   version_launches=2, agent_kind="direct_agent", extra_launch=False):
        source = self.home / "auth.json"
        source.touch(mode=0o600)
        source.write_bytes(b"offline-auth-copy-canary")
        private = self.root / "private-workspace"
        private.mkdir(mode=0o700)
        tunnel = SimpleNamespace(deadline=runner.time.monotonic() + 450, forwarded=0, bytes=0,
            events=[], closing=False, thread=SimpleNamespace(is_alive=lambda: True), start=lambda: 1234)

        def close():
            tunnel.closing = True
            return cleanup

        tunnel.close = close

        def prepare(root, *_args):
            wrapper = root / "grok-sandbox"
            wrapper.touch(mode=0o700)
            settings = root / "home/.grok/config.toml"
            settings.touch(mode=0o600)
            settings.write_text('[[permission.rules]]\naction = "ask"\ntool = "any"\n', encoding="utf-8")
            return wrapper, settings

        def launch(command, **_kwargs):
            self.assertEqual(command[1], runner.TEST_NAME)
            rows = evidence()
            # 新运行器mock补齐当前Rust诊断账本；保留旧合成fixture本身的读取兼容。
            rows[-1].update(private_response_envelope_status="not_observed", private_response_envelope_bytes=0,
                private_response_envelope_sha256=None)
            (private / "private-evidence.ndjson").write_text("".join(json.dumps(event) + "\n" for event in rows), encoding="utf-8")
            (private / "private-native-error-response.ndjson").touch(mode=0o600)
            (private / "private-native-response-envelope.ndjson").touch(mode=0o600)
            (private / "wrapper-audit.ndjson").write_text("".join(json.dumps({"event": "native_launch", "kind": kind,
                "arguments_unchanged": True}) + "\n" for kind in
                ["version"] * version_launches + [agent_kind] + (["unexpected"] if extra_launch else [])), encoding="utf-8")
            tunnel.forwarded = runner.MAX_TLS_CONNECTIONS if budget_rejected else 2
            tunnel.events.append({"event": "official_tunnel_opened", "host": "cli-chat-proxy.grok.com"})
            if budget_rejected:
                tunnel.events.append({"event": "official_connect_budget_rejected", "host": "cli-chat-proxy.grok.com"})
            return SimpleNamespace(returncode=0,
                communicate=lambda **_kw: ("PRIVATE_BODY_CANARY\ntest result: ok. 1 passed; 0 failed", None))

        with ExitStack() as stack:
            stack.enter_context(patch.object(runner.tempfile, "mkdtemp", return_value=str(private)))
            stack.enter_context(patch.object(runner.official, "OfficialTunnel", return_value=tunnel))
            stack.enter_context(patch.object(runner.shared, "network_canary", return_value={"credential_read": 1}))
            stack.enter_context(patch.object(runner, "project_write_canary", side_effect=ValueError("拒绝未证明")
                if canary_error else None, return_value={"project_write": 1}))
            stack.enter_context(patch.object(runner, "prepare_probe_native", side_effect=prepare))
            version = stack.enter_context(patch.object(runner.subprocess, "run",
                return_value=subprocess.CompletedProcess([], 0, "wrong version" if bad_version else runner.shared.VERSION)))
            native = stack.enter_context(patch.object(runner.subprocess, "Popen", side_effect=launch))
            stack.enter_context(patch("sys.stdout", new=io.StringIO()))
            result = runner.run(self.args)
        metadata = json.loads(self.args.output.with_suffix(".metadata.json").read_text(encoding="utf-8"))
        return result, metadata, private, native.call_count, version.call_count

    def test_complete_runner_keeps_raw_private_and_model_http_count_unknown(self):
        result, metadata, private, count, _ = self.mocked_run()
        self.assertEqual(result, 0)
        self.assertEqual(count, 1)
        self.assertTrue(metadata["probe_passed"])
        # 合成报告显式提供完整来源；这里不计为真实 CLI 来源证据。
        self.assertTrue(metadata["native_origin_verified"])
        self.assertFalse(metadata["public_product_gate_open"])
        self.assertFalse(metadata["http_model_call_budget_enforced"])
        self.assertFalse(metadata["http_model_calls_observable"])
        self.assertEqual(metadata["max_tls_connections"], 32)
        self.assertEqual(metadata["native_launch_counters"], {"version": 2, "direct_agent": 1})
        self.assertEqual(metadata["max_native_inputs"], 1)
        self.assertEqual(metadata["max_tls_bytes"], 32 * 1024 * 1024)
        self.assertEqual(metadata["deadline_seconds"], 450)
        self.assertIn("最多 32 次 opaque TLS CONNECT", metadata["budget_boundary"])
        self.assertEqual(metadata["requested_model_request_budget"], 2)
        self.assertFalse((private / "home/.grok/auth.json").exists())
        self.assertTrue((self.home / "auth.json").exists())
        for path in (private / "private-evidence.ndjson", private / "private-test-output.txt"):
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
        self.assertEqual(metadata["private_evidence_sha256"], runner.shared.digest(private / "private-evidence.ndjson"))
        self.assertEqual(metadata["private_native_error_response_sha256"],
            runner.shared.digest(private / "private-native-error-response.ndjson"))
        self.assertEqual(metadata["private_native_error_response_count"], 0)
        self.assertTrue(metadata["private_native_error_response_ledger_matches"])
        for path in runner.artifacts(self.args.output):
            public = path.read_text(encoding="utf-8")
            self.assertNotIn("PRIVATE_BODY_CANARY", public)
            self.assertNotIn("offline-auth-copy-canary", public)

    def test_network_budget_failure_does_not_mean_missing_origin(self):
        result, metadata, _private, count, _ = self.mocked_run(budget_rejected=True)
        self.assertEqual(result, 1)
        self.assertEqual(count, 1)
        self.assertFalse(metadata["probe_passed"])
        self.assertTrue(metadata["network_budget_exhausted"])
        self.assertEqual(metadata["origin_verification_limit"], "network_budget_insufficient")
        self.assertFalse(metadata["native_origin_verified"])

    def test_missing_runtime_version_check_rejects_observation(self):
        result, metadata, _private, count, _ = self.mocked_run(version_launches=1)
        self.assertEqual(result, 1)
        self.assertEqual(count, 1)
        self.assertEqual(metadata["native_launch_counters"], {"version": 1, "direct_agent": 1})
        self.assertFalse(metadata["probe_passed"])

    def test_extra_version_check_rejects_observation(self):
        result, metadata, _private, count, _ = self.mocked_run(version_launches=3)
        self.assertEqual(result, 1)
        self.assertEqual(count, 1)
        self.assertEqual(metadata["native_launch_counters"], {"version": 3, "direct_agent": 1})
        self.assertFalse(metadata["probe_passed"])

    def test_legacy_leader_audit_cannot_pass_direct_probe(self):
        result, metadata, _private, count, _ = self.mocked_run(agent_kind="private_leader")
        self.assertEqual(result, 1)
        self.assertEqual(count, 1)
        self.assertEqual(metadata["native_launch_counters"], {"version": 2, "direct_agent": 0})
        self.assertFalse(metadata["probe_passed"])

    def test_extra_unknown_launch_cannot_pass_exact_direct_count(self):
        result, metadata, _private, count, _ = self.mocked_run(extra_launch=True)
        self.assertEqual(result, 1)
        self.assertEqual(count, 1)
        self.assertEqual(metadata["native_launch_counters"], {"version": 2, "direct_agent": 1})
        self.assertFalse(metadata["probe_passed"])

    def test_cleanup_failure_preserves_probe_failure(self):
        result, metadata, _private, _, _ = self.mocked_run(cleanup=False)
        self.assertEqual(result, 1)
        self.assertFalse(metadata["tunnels_stopped"])
        self.assertFalse(metadata["probe_passed"])
        self.assertTrue(metadata["private_auth_copy_removed"])

    def test_canary_failure_prevents_version_and_native_request(self):
        result, metadata, _private, native, version = self.mocked_run(canary_error=True)
        self.assertEqual(result, 1)
        self.assertEqual(native, 0)
        self.assertEqual(version, 0)
        self.assertEqual(metadata["origin_verification"], "unknown")
        self.assertTrue(metadata["private_auth_copy_removed"])

    def test_version_failure_prevents_native_request(self):
        result, metadata, _private, native, version = self.mocked_run(bad_version=True)
        self.assertEqual(result, 1)
        self.assertEqual(native, 0)
        self.assertEqual(version, 1)
        self.assertEqual(metadata["origin_verification"], "unknown")



class ExactNativeApprovalTests(unittest.TestCase):
    def observe(self, events):
        return runner.probe_observation(0, "test result: ok. 1 passed; 0 failed", events)

    def test_exact_single_allow_once_without_complete_native_origin_fails_whole_probe(self):
        events = evidence_with_exact_approval("missing_native_origin")
        result = self.observe(events)
        self.assertFalse(result["probe_passed"])
        self.assertFalse(result["native_origin_verified"])
        self.assertEqual(result["origin_verification"], "missing_native_origin")

    def test_identical_approval_retries_preserve_one_allowance(self):
        events = evidence_with_exact_approval(retries=3)
        self.assertTrue(self.observe(events)["probe_passed"])
        self.assertEqual(self.observe(events)["native_counters"]["approval_allow_count"], 1)

    def test_approval_count_bool_negative_missing_and_multiple_allowances_fail(self):
        for key in ("approval_allow_count", "approval_deny_count", "approval_duplicate_count", "permission_request_count"):
            for value in (True, -1, None, "1", {}, 2):
                with self.subTest(key=key, value=value):
                    events = evidence_with_exact_approval()
                    events[-1][key] = value
                    self.assertFalse(self.observe(events)["probe_passed"])

    def test_denials_and_unsafe_native_contract_fail(self):
        for key, value in (("approval_deny_count", 1), ("native_contract_verified", False),
                ("approval_all_denied", True), ("unexpected_tool_count", 1)):
            events = evidence_with_exact_approval()
            events[-1][key] = value
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_approval_identity_hash_decision_or_confirmation_mismatch_fails(self):
        for key, value in (("request_id", None), ("session_id", "other-session"), ("prompt_id", "other-prompt"),
                ("tool_call_id", "other-tool"), ("decision", "allow_always"), ("decision", "denied"),
                ("duplicate", True), ("native_contract_verified", False), ("session_id_matches", False),
                ("prompt_id_matches", False), ("tool_call_id_in_native_ledger", False),
                ("raw_input_sha256", "invalid"), ("raw_input_sha256", "b" * 64),
                ("selected_option_id", "allow-always"), ("request_payload_sha256", "invalid")):
            with self.subTest(key=key, value=value):
                events = evidence_with_exact_approval()
                events[-2][key] = value
                self.assertFalse(self.observe(events)["probe_passed"])

    def test_missing_approval_or_native_ledger_cannot_be_replaced_by_finish_counter(self):
        for kind in ("probe_approval_observed", "native_tool_update_received"):
            events = [event for event in evidence_with_exact_approval() if event["event"] != kind]
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_native_ledger_must_have_same_tool_session_prompt_and_full_input_hash(self):
        for key, value in (("probe_tool", False), ("session_id", "other-session"),
                ("prompt_id", "other-prompt"), ("tool_call_id", "other-tool"),
                ("raw_input_present", False), ("raw_input_sha256", "b" * 64)):
            events = evidence_with_exact_approval()
            events[-3][key] = value
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_second_approval_request_cannot_change_id_tool_hash_or_claim_second_new_allow(self):
        for key, value in (("request_id", 72), ("tool_call_id", "other-tool"),
                ("raw_input_sha256", "b" * 64), ("duplicate", False),
                ("request_payload_sha256", "c" * 64), ("selected_option_id", "other-once")):
            events = evidence_with_exact_approval(retries=1)
            events[-2][key] = value
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_closed_native_source_gate_requires_complete_origin_even_after_approval(self):
        events = evidence_with_exact_approval("candidate_mapping_only")
        self.assertFalse(self.observe(events)["probe_passed"])
        self.assertFalse(self.observe(events)["native_origin_verified"])
        events[-1]["origin_verification"] = "verified_native_fields"
        self.assertFalse(self.observe(events)["native_origin_verified"])
        events[-1].update(full_native_origin_fields_observed=True,
            native_origin_ledger_relation_verified=True, candidate_mapping_only=False)
        self.assertTrue(self.observe(events)["native_origin_verified"])
        self.assertFalse(events[-1]["public_product_gate_open"])

    def test_public_approval_projection_retains_only_closed_decision_ids_counts_and_hashes(self):
        for decision in ("allow_once", "denied"):
            event = dict(evidence_with_exact_approval()[-2], decision=decision)
            projected = runner.public_events([event])[0]
            self.assertEqual(projected["decision"], decision)
            self.assertEqual(projected["native_contract_verified"], True)
            self.assertEqual(projected["request_id"], 71)
            self.assertEqual(projected["raw_input_sha256"], "a" * 64)

    def test_public_approval_projection_hashes_arbitrary_sensitive_decision_and_fields(self):
        canary = "OFFLINE_SDK_APPROVAL_SECRET_CANARY"
        for value in (canary, {"credential": canary}, [canary], True, None):
            event = dict(evidence_with_exact_approval()[-2], decision=value,
                request_id=canary, raw_input_sha256=canary, request_payload_sha256=canary,
                selected_option_id=value, unknown_payload={"token": canary})
            public = runner.public_events([event])[0]
            self.assertNotIn(canary, json.dumps(public))
            self.assertIsInstance(public["decision"], dict)
            self.assertIsInstance(public["request_id"], dict)

    def test_calibrated_three_options_include_permanent_choice_but_observation_selects_only_once(self):
        request = calibrated_three_option_request()
        self.assertEqual([(option["optionId"], option["kind"]) for option in request["params"]["options"]],
            [("always-allow", "allow_always"), ("allow-once", "allow_once"), ("reject-once", "reject_once")])
        events = evidence_with_exact_approval(retries=1)
        self.assertEqual(events[-2]["request_payload_sha256"], request_fingerprint(request))
        self.assertTrue(self.observe(events)["probe_passed"])
        self.assertTrue(all(event["selected_option_id"] == "allow-once" for event in events
            if event["event"] == "probe_approval_observed"))
        self.assertEqual(self.observe(events)["native_counters"]["approval_allow_count"], 1)

    def test_permanent_option_cannot_be_selected_or_published_as_selected_even_if_offered(self):
        for key, value in (("selected_option_id", "always-allow"), ("decision", "allow_always")):
            events = evidence_with_exact_approval()
            events[-2][key] = value
            self.assertFalse(self.observe(events)["probe_passed"])
            projected = runner.public_events([events[-2]])[0]
            self.assertIsInstance(projected[key], dict)
            self.assertNotIn(value, json.dumps(projected))

    def test_changed_permanent_option_display_label_is_a_conflicting_full_request_retry(self):
        events = evidence_with_exact_approval(retries=1)
        request = calibrated_three_option_request()
        request["params"]["options"][0]["name"] = "OFFLINE_CHANGED_PERMANENT_LABEL_CANARY"
        events[-2]["request_payload_sha256"] = request_fingerprint(request)
        self.assertFalse(self.observe(events)["probe_passed"])
        public = runner.public_events(events)
        self.assertNotIn("PERMANENT_LABEL_CANARY", json.dumps(public))

    def test_zero_approval_requires_consistent_zero_counts_and_verified_native_contract(self):
        self.assertTrue(self.observe(evidence())["probe_passed"])
        for key, value in (("approval_duplicate_count", 1), ("approval_all_denied", False),
                ("native_contract_verified", False), ("approval_allow_count", 1)):
            events = evidence()
            events[-1][key] = value
            self.assertFalse(self.observe(events)["probe_passed"])


class CalibratedDiscoveryTests(unittest.TestCase):
    def observe(self, events):
        return runner.probe_observation(0, "test result: ok. 1 passed; 0 failed", events)

    def search_approval(self, events):
        return next(event for event in events if event.get("event") == "probe_approval_observed"
            and event.get("calibrated_tool_kind") == "search_discovery")

    def test_exact_readonly_search_and_inspect_each_allow_once_with_independent_origin(self):
        events = evidence_with_search_and_inspect()
        result = self.observe(events)
        self.assertTrue(result["probe_passed"])
        self.assertTrue(result["native_origin_verified"])
        self.assertEqual(result["native_counters"]["search_allow_count"],1)
        self.assertEqual(result["native_counters"]["inspect_allow_count"],1)
        search = self.search_approval(events)
        self.assertFalse(search["native_contract_verified"])
        self.assertTrue(search["discovery_native_contract_verified"])
        self.assertEqual(search["selected_option_id"],"allow-once")
        self.assertFalse(events[-1]["public_product_gate_open"])

    def test_search_and_inspect_retries_do_not_expand_per_tool_allowances(self):
        events = evidence_with_search_and_inspect(search_retries=2,inspect_retries=3)
        self.assertTrue(self.observe(events)["probe_passed"])
        self.assertEqual(self.observe(events)["native_counters"]["approval_allow_count"],2)
        self.assertEqual(self.observe(events)["native_counters"]["approval_duplicate_count"],5)

    def test_search_cannot_replace_missing_sdk_source_or_tool_registration(self):
        for state in ("unknown","missing_native_origin","candidate_mapping_only"):
            events = evidence_with_search_and_inspect(state)
            self.assertFalse(self.observe(events)["probe_passed"])
            self.assertFalse(self.observe(events)["native_origin_verified"])
        events = evidence_with_search_and_inspect()
        events[-1].update(initialization_count=0,tools_list_count=0,inspect_call_count=0,
            sdk_request_count=0,native_contract_verified=False,origin_verification="unknown",
            full_native_origin_fields_observed=False,native_origin_ledger_relation_verified=False,passed=False)
        self.assertFalse(self.observe(events)["probe_passed"])

    def test_second_discovery_and_bool_or_over_limit_per_kind_counts_fail(self):
        for key,value in (("search_allow_count",2),("inspect_allow_count",2),("search_allow_count",True),
                ("inspect_allow_count",True),("approval_allow_count",1),("approval_allow_count",3)):
            events = evidence_with_search_and_inspect()
            events[-1][key] = value
            self.assertFalse(self.observe(events)["probe_passed"])
        events = evidence_with_search_and_inspect(search_retries=1)
        duplicates = [event for event in events if event.get("calibrated_tool_kind") == "search_discovery"
            and event.get("event") == "probe_approval_observed"]
        duplicates[-1]["duplicate"] = False
        self.assertFalse(self.observe(events)["probe_passed"])

    def test_changed_search_query_limit_variant_or_extra_arguments_cannot_match_native_input_hash(self):
        for key,value in (("query","other-server inspect"),("limit",6),("variant","UseTool"),("extra",{})):
            events = evidence_with_search_and_inspect()
            request = calibrated_search_request()
            request["params"]["toolCall"]["rawInput"][key] = value
            search = self.search_approval(events)
            search["raw_input_sha256"] = request_fingerprint(request["params"]["toolCall"]["rawInput"])
            search["request_payload_sha256"] = request_fingerprint(request)
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_discovery_approval_requires_same_session_prompt_tool_and_separate_contract_flags(self):
        for key,value in (("session_id","other-session"),("prompt_id","other-prompt"),
                ("tool_call_id","other-tool"),("native_contract_verified",True),
                ("discovery_native_contract_verified",False),("approval_contract_verified",False),
                ("calibrated_tool_kind","inspect"),("calibrated_tool_kind",{"secret":"OFFLINE_DISCOVERY_CANARY"})):
            events = evidence_with_search_and_inspect()
            self.search_approval(events)[key] = value
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_discovery_ledger_is_distinct_from_inspect_ledger_and_never_marked_probe_tool(self):
        for key,value in (("probe_tool",True),("discovery_tool",False),("calibrated_tool_kind","inspect"),
                ("tool_call_id","native-inspect-1"),("session_id","other-session"),("prompt_id","other-prompt")):
            events = evidence_with_search_and_inspect()
            ledger = next(event for event in events if event.get("event") == "native_tool_update_received"
                and event.get("calibrated_tool_kind") == "search_discovery")
            ledger[key] = value
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_changed_payload_retry_or_shared_request_or_tool_id_between_discovery_and_inspect_fails(self):
        events = evidence_with_search_and_inspect(search_retries=1)
        second = [event for event in events if event.get("event") == "probe_approval_observed"
            and event.get("calibrated_tool_kind") == "search_discovery"][-1]
        second["request_payload_sha256"] = "c"*64
        self.assertFalse(self.observe(events)["probe_passed"])
        for key,value in (("request_id",71),("tool_call_id","native-inspect-1")):
            events = evidence_with_search_and_inspect()
            self.search_approval(events)[key] = value
            self.assertFalse(self.observe(events)["probe_passed"])

    def test_discovery_contract_flag_must_match_observed_discovery_ledger(self):
        events = evidence_with_search_and_inspect()
        events[-1]["discovery_native_contract_verified"] = False
        self.assertFalse(self.observe(events)["probe_passed"])
        events = evidence_with_exact_approval()
        events[-1]["discovery_native_contract_verified"] = True
        self.assertFalse(self.observe(events)["probe_passed"])

    def test_public_discovery_projection_uses_closed_kind_and_omits_query_and_tool_title(self):
        event = self.search_approval(evidence_with_search_and_inspect())
        public = runner.public_events([event])[0]
        self.assertEqual(public["calibrated_tool_kind"],"search_discovery")
        self.assertEqual(public["discovery_native_contract_verified"],True)
        self.assertEqual(public["native_contract_verified"],False)
        canary = "OFFLINE_DISCOVERY_PRIVATE_CANARY"
        for kind in (canary,{"token":canary},[canary],True,None):
            changed = dict(event,calibrated_tool_kind=kind,query=canary,title=canary)
            public = runner.public_events([changed])[0]
            self.assertIsInstance(public["calibrated_tool_kind"],dict)
            self.assertNotIn(canary,json.dumps(public))

    def test_modern_discovery_is_counted_without_fabricating_initialize(self):
        observed = self.observe(modern_evidence())
        self.assertTrue(observed["probe_passed"])
        self.assertEqual(observed["native_counters"]["initialization_count"], 0)
        self.assertEqual(observed["native_counters"]["discovery_count"], 1)

    def test_modern_discovery_does_not_replace_missing_native_origin(self):
        observed = self.observe(modern_evidence("missing_native_origin"))
        self.assertFalse(observed["probe_passed"])
        self.assertFalse(observed["native_origin_verified"])
        self.assertEqual(observed["origin_verification"], "missing_native_origin")

    def test_modern_initialization_mode_and_version_must_agree(self):
        for field, value in (("initialization_count", 1), ("discovery_count", 0),
                ("discovery_count", True), ("negotiatedProtocolVersion", "2025-11-25"),
                ("servedToolNames", ["run_agents"]), ("tools_list_count", 2)):
            with self.subTest(field=field, value=value):
                events = modern_evidence()
                events[-1][field] = value
                self.assertFalse(self.observe(events)["probe_passed"])
        events = modern_evidence()
        events[2]["initialization_mode"] = "legacy_initialize"
        self.assertFalse(self.observe(events)["probe_passed"])

    def test_modern_request_needs_actual_version_carrier_and_metadata_validation(self):
        for field, value in (("requested_protocol_version", "2030-01-01"),
                ("requested_protocol_version", None), ("protocol_version_carrier", "params.protocolVersion"),
                ("metadata_schema_valid", False), ("metadata_schema_valid", 1), ("mcp_method", "ping")):
            with self.subTest(field=field, value=value):
                events = modern_evidence()
                events[3][field] = value
                self.assertFalse(self.observe(events)["probe_passed"])

    def test_modern_discovery_list_and_call_are_all_required(self):
        for position in (1, 3, 4):
            with self.subTest(position=position):
                events = modern_evidence()
                del events[position]
                events[-1]["sdk_request_count"] = 2
                self.assertFalse(self.observe(events)["probe_passed"])

    def test_modern_duplicate_discovery_does_not_count_as_another_initialize(self):
        events = modern_evidence()
        events.insert(2, dict(events[1], outer_id=9))
        events[-1]["sdk_request_count"] = 4
        self.assertTrue(self.observe(events)["probe_passed"])
        events[2]["inner_id"] = 99
        self.assertFalse(self.observe(events)["probe_passed"])

    def test_modern_diagnostics_preserve_only_exact_fixed_enums_and_names(self):
        diagnostics = {"negotiatedProtocolVersion": "2026-07-28", "initialization_mode": "modern_discover",
            "protocol_version_carrier": "params._meta", "discovery_count": 1, "servedToolNames": ["inspect"],
            "requested_protocol_version_sha256": "a" * 64, "metadata_schema_valid": True}
        self.assertEqual(runner.projection(diagnostics), diagnostics)
        canary = "Bearer OFFLINE_MODERN_METADATA_CANARY"
        for field in ("negotiatedProtocolVersion", "initialization_mode", "protocol_version_carrier", "servedToolNames"):
            for value in (canary, {"name": canary}, [canary], True):
                with self.subTest(field=field, value=value):
                    projected = runner.projection({field: value})
                    self.assertIsInstance(projected[field], dict)
                    self.assertNotIn(canary, json.dumps(projected))

    def test_modern_metadata_key_projection_does_not_publish_unknown_bodies(self):
        keys = ["io.modelcontextprotocol/protocolVersion", "io.modelcontextprotocol/clientInfo",
            "io.modelcontextprotocol/clientCapabilities"]
        event = {"event": "sdk_request_received", "metadata_keys": {"mcp_params": keys}}
        self.assertEqual(runner.public_events([event])[0]["metadata_keys"]["mcp_params"], sorted(keys))
        canary = "Bearer OFFLINE_MODERN_BODY_CANARY"
        event["metadata_keys"]["unknown"] = {"body": canary}
        event["private_body"] = {"body": canary}
        self.assertNotIn(canary, json.dumps(runner.public_events([event])))

class SafeTerminationDiagnosticTests(unittest.TestCase):
    def test_production_error_survives_the_closed_event_stream_without_proving_origin(self):
        events = modern_evidence()
        diagnostic = {"runtime_error_kind": "protocol", "protocol_failure_kind": "invalid_response_id",
            "runtime_error_message": {"type": "string", "bytes": 42, "sha256": "a" * 64}}
        events[-1].update(transport_task_status="runtime_error", transport_error=diagnostic,
            transport_join_error=None, failure_source="production_runtime")
        result = runner.probe_observation(0, "1 passed; 0 failed", events)
        self.assertFalse(result["probe_passed"])
        self.assertFalse(result["native_origin_verified"])
        self.assertEqual(result["failure_diagnostics"]["transport_error"], diagnostic)
        self.assertEqual(result["failure_diagnostics"]["failure_source"], "production_runtime")

    def test_error_summaries_never_publish_raw_bodies_even_when_they_equal_protocol_values(self):
        canary = "OFFLINE_ERROR_DIAGNOSTIC_PRIVATE_CANARY"
        event = {"event": "probe_finished", "transport_error": {"runtime_error_kind": "protocol",
            "protocol_failure_kind": "unknown", "runtime_error_message": "initialize", "message": canary},
            "last_native_response_diagnostic": {"jsonrpc": {"type": "string", "bytes": 3, "sha256": "a" * 64},
                "jsonrpc_is_2_0": True, "error_present": True, "error_type": "object", "error_code": -32603,
                "error_message": canary, "error_data_message": {"type": "string", "bytes": 1,
                    "sha256": "b" * 64, "message": "tools/list"}, "result_stop_reason": [canary]}}
        public = runner.public_events([event])[0]
        text = json.dumps(public)
        self.assertNotIn(canary, text)
        self.assertNotIn("initialize", text)
        self.assertNotIn("tools/list", text)
        self.assertEqual(public["last_native_response_diagnostic"]["error_code"], -32603)
        self.assertTrue(public["last_native_response_diagnostic"]["jsonrpc_is_2_0"])

    def test_malformed_diagnostic_enums_and_numbers_cannot_become_trusted_values(self):
        for value in (True, -1, 2 ** 63, "OFFLINE_PRIVATE_NUMBER", {"private": "OFFLINE_PRIVATE_NUMBER"}):
            with self.subTest(value=value):
                self.assertIsNone(runner.projection(value, "bytes"))
        for value in (True, 0.5, 2 ** 63, "OFFLINE_PRIVATE_CODE"):
            with self.subTest(value=value):
                self.assertIsNone(runner.projection(value, "error_code"))
        for key in ("transport_task_status", "failure_source", "runtime_error_kind", "protocol_failure_kind"):
            with self.subTest(key=key):
                public = runner.projection("OFFLINE_PRIVATE_ENUM", key)
                self.assertNotIn("OFFLINE_PRIVATE_ENUM", json.dumps(public))
        self.assertIsNone(runner.projection(True, "native_error_http_status"))
        self.assertIsNone(runner.projection("429", "native_error_http_status"))
        self.assertEqual(runner.projection(429, "native_error_http_status"), 429)
        self.assertEqual(runner.projection("http_429", "native_error_category"), "http_429")

    def test_join_failure_or_unknown_transport_status_cannot_pass_a_forged_success(self):
        for status in ("join_error", "join_timeout", "OFFLINE_PRIVATE_STATUS"):
            with self.subTest(status=status):
                events = evidence()
                events[-1]["transport_task_status"] = status
                observed = runner.probe_observation(0, "1 passed; 0 failed", events)
                self.assertFalse(observed["probe_passed"])
                self.assertNotIn("OFFLINE_PRIVATE_STATUS", json.dumps(observed))

    def test_private_error_audit_publishes_only_count_length_and_file_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "error.ndjson"
            path.touch(mode=0o600)
            empty = runner.private_error_audit(path)
            self.assertEqual(empty["private_native_error_response_count"], 0)
            path.write_text(json.dumps({"jsonrpc": "2.0", "id": "offline-unknown-id", "error": {
                "code": -32603, "message": "OFFLINE_RAW_ERROR_CANARY",
                "data": {"http_status": 429, "message": "OFFLINE_RAW_NESTED_CANARY"}}}) + "\n")
            audit = runner.private_error_audit(path)
            self.assertEqual(audit["private_native_error_response_count"], 1)
            self.assertEqual(audit["private_native_error_response_sha256"], runner.shared.digest(path))
            self.assertTrue(audit["private_native_error_response_scope_verified"])
            self.assertNotIn("CANARY", json.dumps(audit))

    def test_private_error_audit_rejects_model_fields_multiple_records_and_oversize(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "error.ndjson"
            path.touch(mode=0o600)
            for response in ({"error": {}, "result": {}}, {"error": {"data": {"body": "OFFLINE_BODY"}}},
                    {"error": {"message": {"prompt": "OFFLINE_PROMPT"}}}):
                with self.subTest(response=response):
                    path.write_text(json.dumps(response))
                    with self.assertRaises(ValueError):
                        runner.private_error_audit(path)
            path.write_text('{"error":{}}\n{"error":{}}\n')
            with self.assertRaises(ValueError):
                runner.private_error_audit(path)
            path.write_bytes(b"x" * (runner.MAX_PRIVATE_ERROR_BYTES + 1))
            with self.assertRaises(ValueError):
                runner.private_error_audit(path)

    def test_private_error_audit_refuses_public_files_symlinks_and_hardlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "error.ndjson"
            path.touch(mode=0o600)
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                runner.private_error_audit(path)
            path.chmod(0o600)
            link = Path(directory) / "link.ndjson"
            link.symlink_to(path)
            with self.assertRaises(OSError):
                runner.private_error_audit(link)
            hardlink = Path(directory) / "hardlink.ndjson"
            os.link(path, hardlink)
            with self.assertRaises(ValueError):
                runner.private_error_audit(path)


class TransactionEnvelopeDiagnosticTests(unittest.TestCase):
    def summary(self, value="OFFLINE_ID"):
        raw = json.dumps(value).encode()
        return {"type": "string" if isinstance(value, str) else "number", "bytes": len(raw), "sha256": runner.sha(raw)}

    def context(self):
        return {"generation": "11111111-1111-4111-8111-111111111111", "next_request_id": 2,
            "pending_id": 1, "pending_kind": "initialize"}

    def outbound(self):
        return {"event": "outbound_transaction_observed", "sequence": 1, "transaction_context": self.context(),
            "method": "initialize", "id": self.summary(1), "id_number": 1, "inner_response_id": {"type": "absent"},
            "method_present": True, "result_present": False, "error_present": False}

    def envelope(self):
        return {"jsonrpc": "2.0", "id": "OFFLINE_EXACT_PRIVATE_ID", "result_type": "object", "result_field_count": 1,
            "result_fields": [{"key": self.summary("OFFLINE_PRIVATE_KEY"), "value_type": "string"}], "error_present": False,
            "skills_reload_closed_success_shape": False}

    def maintenance_envelope(self):
        return self.envelope() | {"id": "skills-reload", "skills_reload_closed_success_shape": True,
            "result_fields": [{"key": {"type": "string", "bytes": 6, "sha256": runner.sha(b"result")},
                "value_type": "object"}]}

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_maintenance_shape_audit_exports_only_the_strict_boolean_and_file_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            path.write_text(json.dumps(self.maintenance_envelope()))
            audit = runner.private_response_envelope_audit(path)
            self.assertIs(audit["skills_reload_closed_success_shape"], True)
            public = json.dumps(audit)
            for forbidden in ("skills-reload", "reloaded", "result_fields", "OFFLINE"):
                self.assertNotIn(forbidden, public)
            path.write_text(json.dumps(self.envelope()))
            self.assertIs(runner.private_response_envelope_audit(path)["skills_reload_closed_success_shape"], False)

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_maintenance_shape_audit_rejects_non_boolean_flags_and_legacy_schema(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            for flag in (0, 1, -1, 1.0, "true", None, {}, [], {"body": "OFFLINE_DEEP_BODY"}):
                path.write_text(json.dumps(self.maintenance_envelope() | {"skills_reload_closed_success_shape": flag}))
                with self.assertRaises(ValueError):
                    runner.private_response_envelope_audit(path)
            legacy = self.envelope()
            legacy.pop("skills_reload_closed_success_shape")
            path.write_text(json.dumps(legacy))
            with self.assertRaises(ValueError):
                runner.private_response_envelope_audit(path)

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_true_maintenance_shape_audit_rejects_conflicting_outer_summaries(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            for fields in ({"id": "OFFLINE_OTHER_ID"}, {"id": 1}, {"jsonrpc": "1.0"},
                    {"error_present": True}, {"result_type": "string", "result_field_count": 0, "result_fields": None},
                    {"result_field_count": 0, "result_fields": []},
                    {"result_fields": [{"key": self.summary("OFFLINE_OTHER_KEY"), "value_type": "object"}]},
                    {"result_fields": [{"key": {"type": "string", "bytes": 6, "sha256": runner.sha(b"result")},
                        "value_type": "number"}]},
                    {"result_fields": [{"key": {"type": "string", "bytes": 5, "sha256": runner.sha(b"result")},
                        "value_type": "object"}]}):
                path.write_text(json.dumps(self.maintenance_envelope() | fields))
                with self.assertRaises(ValueError):
                    runner.private_response_envelope_audit(path)

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_maintenance_shape_audit_rejects_control_fields_and_deep_values(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            for fields in ({"method": "_x.ai/internal/reload_skills"}, {"error": None},
                    {"sessionId": "OFFLINE_OTHER_SESSION"}, {"generation": "OFFLINE_OTHER_GENERATION"},
                    {"result": {"result": {"reloaded": {"body": ["OFFLINE_DEEP_BODY"]}}}},
                    {"reloaded": 18446744073709551615}, {"extra": "OFFLINE_BODY"}):
                path.write_text(json.dumps(self.maintenance_envelope() | fields))
                with self.assertRaises(ValueError):
                    runner.private_response_envelope_audit(path)

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_empty_capture_has_no_maintenance_shape_claim(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            self.assertNotIn("skills_reload_closed_success_shape", runner.private_response_envelope_audit(path))

    def test_maintenance_shape_public_projection_never_promotes_non_boolean_values(self):
        for value in (True, False):
            self.assertIs(runner.projection({"skills_reload_closed_success_shape": value})[
                "skills_reload_closed_success_shape"], value)
        for value in (0, 1, -1, 1.0, "OFFLINE_BODY", None, {"body": ["OFFLINE_DEEP_BODY"]}):
            projected = runner.projection({"skills_reload_closed_success_shape": value})
            self.assertIsNone(projected["skills_reload_closed_success_shape"])
            self.assertNotIn("OFFLINE", json.dumps(projected))

    def test_maintenance_shape_boolean_cannot_verify_sdk_origin_or_complete_the_probe(self):
        events = evidence()
        events[-1].update(passed=False, skills_reload_closed_success_shape=True)
        observation = runner.probe_observation(101, "", events)
        self.assertFalse(observation["probe_passed"])
        self.assertFalse(observation["native_origin_verified"])

    def test_outbound_closed_context_and_summaries_are_retained_without_id_body(self):
        row = self.outbound()
        self.assertEqual(runner.public_events([row]), [row])
        self.assertNotIn("OFFLINE_ID", json.dumps(runner.public_events([row])))
        for kind in runner.PENDING_KINDS:
            context = self.context() | {"pending_kind": kind}
            self.assertTrue(runner.transaction_context_valid(context))
        self.assertTrue(runner.transaction_context_valid(self.context() | {"pending_id": None, "pending_kind": None}))

    def test_unknown_outbound_fields_raw_ids_and_boolean_counters_are_rejected(self):
        for patching in ({"prompt": "OFFLINE_BODY"}, {"id": "initialize"}, {"sequence": True},
                {"method": "OFFLINE_METHOD"}, {"id_number": True}):
            row = self.outbound() | patching
            self.assertFalse(runner.outbound_transaction_valid(row))
            public = runner.public_events([row])
            self.assertEqual(public[0]["event"], "unrecognized_private_event")
            self.assertNotIn("OFFLINE", json.dumps(public))

    def test_bad_transaction_context_is_hashed_and_cannot_pass_forged_success(self):
        for patching in ({"pending_kind": "OFFLINE_BODY"}, {"pending_id": True}, {"generation": "OFFLINE_BODY"},
                {"pending_id": None}, {"extra": "OFFLINE_BODY"}):
            context = self.context() | patching
            self.assertFalse(runner.transaction_context_valid(context))
            self.assertNotIn("OFFLINE_BODY", json.dumps(runner.projection(context, "transaction_context")))
        events = evidence()
        events.insert(-1, self.outbound() | {"transaction_context": self.context() | {"next_request_id": True}})
        self.assertFalse(runner.probe_observation(0, "1 passed; 0 failed", events)["probe_passed"])

    def test_response_transaction_context_and_capture_status_survive_failure_metadata(self):
        events = evidence()
        events[-1].update(passed=False, last_native_response_diagnostic={"transaction_context": self.context()},
            private_response_envelope_status="captured", private_response_envelope_bytes=20,
            private_response_envelope_sha256="a" * 64)
        result = runner.probe_observation(101, "", events)
        self.assertEqual(result["failure_diagnostics"]["last_native_response_diagnostic"]["transaction_context"], self.context())
        self.assertEqual(result["failure_diagnostics"]["private_response_envelope_status"], "captured")
        self.assertFalse(result["probe_passed"])
        self.assertIsNone(runner.projection("OFFLINE_BODY", "private_response_envelope_sha256"))

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_private_envelope_exports_only_file_digest_and_matches_captured_ledger(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            path.write_text(json.dumps(self.envelope()) + "\n")
            audit = runner.private_response_envelope_audit(path)
            self.assertEqual(audit["private_native_response_envelope_count"], 1)
            self.assertNotIn("OFFLINE", json.dumps(audit))
            events = [{"event": "probe_finished", "private_response_envelope_status": "captured",
                "private_response_envelope_bytes": path.stat().st_size, "private_response_envelope_sha256": runner.shared.digest(path)}]
            self.assertTrue(runner.response_envelope_ledger_matches(audit, events))
            for key, value in (("private_response_envelope_bytes", True), ("private_response_envelope_bytes", 0),
                    ("private_response_envelope_sha256", "b" * 64), ("private_response_envelope_status", "not_observed")):
                self.assertFalse(runner.response_envelope_ledger_matches(audit, [events[0] | {key: value}]))

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_empty_envelope_is_not_an_API_error_or_a_failed_capture_success(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            audit = runner.private_response_envelope_audit(path)
            row = {"event": "probe_finished", "private_response_envelope_status": "not_observed",
                "private_response_envelope_bytes": 0, "private_response_envelope_sha256": None}
            self.assertTrue(runner.response_envelope_ledger_matches(audit, [row]))
            for status in runner.PRIVATE_ENVELOPE_STATES - {"not_observed"}:
                self.assertFalse(runner.response_envelope_ledger_matches(audit, [row | {"private_response_envelope_status": status}]))
            self.assertFalse(runner.response_envelope_ledger_matches(audit, [row, row]))

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_private_envelope_refuses_extra_body_duplicate_fields_multiple_and_oversized_records(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            for row in (self.envelope() | {"result": {"prompt": "OFFLINE_BODY"}}, self.envelope() | {"id": True},
                    self.envelope() | {"result_fields": [{"key": "OFFLINE_BODY", "value_type": "string"}]},
                    self.envelope() | {"result_field_count": True}):
                path.write_text(json.dumps(row))
                with self.assertRaises(ValueError): runner.private_response_envelope_audit(path)
            for body in ('{"id":1,"id":2}', json.dumps(self.envelope()) + "\n" + json.dumps(self.envelope()),
                    json.dumps(self.envelope()).replace('"OFFLINE_EXACT_PRIVATE_ID"', '1e999'),
                    "x" * (runner.MAX_PRIVATE_ERROR_BYTES + 1)):
                path.write_text(body)
                with self.assertRaises(ValueError): runner.private_response_envelope_audit(path)

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_private_envelope_refuses_public_permissions_symlinks_hardlinks_and_public_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            path.chmod(0o644)
            with self.assertRaises(ValueError): runner.private_response_envelope_audit(path)
            path.chmod(0o600)
            alias = Path(directory) / "alias"
            alias.symlink_to(path)
            with self.assertRaises(OSError): runner.private_response_envelope_audit(alias)
            alias.unlink()
            os.link(path, alias)
            with self.assertRaises(ValueError): runner.private_response_envelope_audit(path)
            alias.unlink()
            Path(directory).chmod(0o755)
            with self.assertRaises(ValueError): runner.private_response_envelope_audit(path)
            Path(directory).chmod(0o700)

    @unittest.skipUnless(os.name == "posix", "私有信封权限只验证Unix")
    def test_envelope_field_fingerprints_are_bounded_at_128_without_values(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "envelope.ndjson"
            path.touch(mode=0o600)
            row = self.envelope() | {"result_field_count": 129,
                "result_fields": [{"key": self.summary(str(index)), "value_type": "object"} for index in range(128)]}
            path.write_text(json.dumps(row))
            self.assertEqual(runner.private_response_envelope_audit(path)["private_native_response_envelope_count"], 1)
            row["result_fields"].append({"key": self.summary("OFFLINE_EXTRA"), "value_type": "object"})
            path.write_text(json.dumps(row))
            with self.assertRaises(ValueError): runner.private_response_envelope_audit(path)


if __name__ == "__main__":
    unittest.main()
