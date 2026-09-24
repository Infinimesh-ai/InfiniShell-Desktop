#!/usr/bin/env python3
"""升级驱动离线边界；只用合成文件和 Python/mock，不执行原生 CLI。"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch

import verify_cli_autoupdate as runner


class DigestTests(unittest.TestCase):
    def test_debug_binary_size_boundary_keeps_streaming_reads(self):
        path = MagicMock()
        path.is_file.return_value = True
        path.stat.return_value = SimpleNamespace(st_size=8 * 1024 ** 3)
        stream = path.open.return_value.__enter__.return_value
        body = b"synthetic-debug-binary"
        stream.read.side_effect = [body, b""]
        self.assertEqual(runner.digest(path), hashlib.sha256(body).hexdigest())
        self.assertTrue(all(call.args == (1024 * 1024,) for call in stream.read.call_args_list))
        path.reset_mock()
        path.stat.return_value = SimpleNamespace(st_size=8 * 1024 ** 3 + 1)
        with self.assertRaisesRegex(ValueError, "^binary_too_large$"):
            runner.digest(path)
        path.open.assert_not_called()

    def test_non_file_and_growth_beyond_limit_are_distinct(self):
        path = MagicMock()
        path.is_file.return_value = False
        with self.assertRaisesRegex(ValueError, "^binary_not_regular$"):
            runner.digest(path)
        path.open.assert_not_called()
        path.is_file.return_value = True
        path.stat.return_value = SimpleNamespace(st_size=1)
        path.open.return_value.__enter__.return_value.read.return_value = b"1234"
        with patch.object(runner, "MAX_BINARY_BYTES", 3):
            with self.assertRaisesRegex(ValueError, "^binary_too_large$"):
                runner.digest(path)


class DriverTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="infinishell-cli-autoupdate-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        self.root.chmod(0o700)
        self.value = {"schema": 1, "scope": runner.SCOPE, "case_id": "claude-latest-one",
            "root": str(self.root), "agent": "claude", "channel": "latest", "expected": "updated",
            "old_version": "2.1.273", "target_version": "2.1.278", "timeout_seconds": 480,
            "entry": str(self.root / "home/.local/bin/claude")}
        for name in runner.BINDINGS:
            path = self.root / name
            path.write_bytes(("synthetic-" + name).encode())
            self.value[name] = {"path": str(path), "sha256": runner.digest(path)}
        repository = Path(runner.__file__).resolve().parents[2]
        self.write_binding("source_manifest", {"files": [{"path": relative,
            "bytes": (repository / relative).stat().st_size,
            "sha256": runner.digest(repository / relative)} for relative in runner.REQUIRED_SOURCE_FILES]})
        self.write_binding("gates_report", {"source_manifest_sha256": self.value["source_manifest"]["sha256"],
            "all_passed": True, "test_binary": self.value["worker"]})
        self.write_binding("bundle_report", {"source_manifest_sha256": self.value["source_manifest"]["sha256"],
            "exit_code": 0, "strict_signature_exit_code": 0, "worker": self.value["supervisor"]})
        self.strict_signature_verified = runner.strict_signature_verified
        signature = patch.object(runner, "strict_signature_verified", return_value=True)
        signature.start()
        self.addCleanup(signature.stop)
        source_binding = patch.object(runner, "supervisor_source_binding_verified", return_value=True)
        source_binding.start()
        self.addCleanup(source_binding.stop)

    def write_binding(self, name, data):
        path = Path(self.value[name]["path"])
        path.write_bytes(runner.encoded(data))
        self.value[name]["sha256"] = runner.digest(path)

    def event(self):
        event = {key: True for key in runner.BOOLS}
        event.update({key: False for key in ("credentials_provided", "same_commit_verified_by_runner")})
        event.update({key: self.value.get(key, False) for key in ("fixed_release_input", "test_only_target_candidate")})
        event.update({key: self.value[key] for key in ("case_id", "agent", "channel", "expected", "old_version", "target_version")})
        event.update(schema=1, scope=runner.SCOPE, stage="finished", error=None, failure_code=None,
            manifest_sha256="a" * 64, worker_sha256=self.value["worker"]["sha256"],
            supervisor_sha256=self.value["supervisor"]["sha256"],
            source_manifest_sha256=self.value["source_manifest"]["sha256"], old_sha256=self.value["old_binary"]["sha256"],
            target_sha256=self.value["target_binary"]["sha256"], product_inspect_calls=2,
            product_execute_calls=1, model_inputs_sent=0, config_files_checked=13,
            config_transition=self.value.get("config_transition"),
            plan_requires_native_update=self.value["expected"] != "channel_only")
        return event

    def test_fixed_candidate_requires_explicit_release_exact_hash_and_macos(self):
        value = dict(self.value, fixed_release_input=True, test_only_target_candidate=True,
                     target_version="2.1.280", target_binary=dict(self.value["target_binary"],
                     sha256=runner.FIXED_CANDIDATES["claude"][1]))
        with patch.object(runner.sys, "platform", "darwin"):
            runner.validate_manifest(value)
            for bad in (dict(value, fixed_release_input=False), dict(value, fixed_release_input=1),
                        dict(value, target_version="2.1.281"),
                        dict(value, target_binary=dict(value["target_binary"], sha256="b" * 64))):
                with self.subTest(value=bad), self.assertRaises(ValueError):
                    runner.validate_manifest(bad)
        with patch.object(runner.sys, "platform", "linux"), self.assertRaises(ValueError):
            runner.validate_manifest(value)

    def test_linux_candidate_binds_platform_architecture_version_and_digest(self):
        for agent, (version, checksum) in runner.LINUX_FIXED_CANDIDATES.items():
            value = dict(self.value, agent=agent, channel="follow_installation", fixed_release_input=True,
                         test_only_target_candidate=True, target_version=version,
                         target_binary=dict(self.value["target_binary"], sha256=checksum))
            with patch.object(runner.sys, "platform", "linux"), patch.object(runner.platform, "machine", return_value="x86_64"):
                runner.validate_manifest(value)
                with self.assertRaises(ValueError):
                    runner.validate_manifest(dict(value, fixed_release_input=False))
            for system, machine in (("darwin", "x86_64"), ("win32", "amd64"), ("linux", "aarch64")):
                with self.subTest(agent=agent, platform=system, machine=machine), patch.object(runner.sys, "platform", system), patch.object(runner.platform, "machine", return_value=machine):
                    with self.assertRaises(ValueError):
                        runner.validate_manifest(value)

    def test_fixed_release_receipt_cannot_be_claimed_as_live_discovery_or_production_gate(self):
        event = self.event()
        text = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        for key in ("fixed_release_input", "test_only_target_candidate"):
            with self.subTest(key=key):
                self.assertFalse(runner.acceptance(0, text, dict(event, **{key: True}), self.value, "a" * 64))

    def test_manifest_rejects_wrong_channels_boolean_deadlines_and_extra_commands(self):
        runner.validate_manifest(self.value)
        for key, bad in (("channel", "alpha"), ("timeout_seconds", True), ("timeout_seconds", 601),
                         ("expected", "run_command"), ("schema", True), ("old_version", "2.1.273;echo secret")):
            with self.subTest(key=key, bad=bad):
                value = dict(self.value, **{key: bad})
                with self.assertRaises(ValueError):
                    runner.validate_manifest(value)
        with self.assertRaises(ValueError):
            runner.validate_manifest(dict(self.value, argv=["arbitrary-command"]))
        with self.assertRaises(ValueError):
            runner.validate_manifest(dict(self.value, target_binary=self.value["old_binary"], expected="source_changed_rejected"))

    def test_channel_transition_rejects_extra_fields_wrong_agent_and_implicit_default(self):
        change = {"kind": "channel", "before": "stable", "after": "latest"}
        runner.validate_manifest(dict(self.value, config_transition=change))
        for bad in (dict(change, permissions="bypass"), dict(change, before=True), dict(change, after="stable"),
                    dict(change, before=None), {"kind": "other"}, [], "unsafe text"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                runner.validate_manifest(dict(self.value, config_transition=bad))
        with self.assertRaises(ValueError):
            runner.validate_manifest(dict(self.value, expected="source_changed_rejected", config_transition=change))
        with self.assertRaises(ValueError):
            runner.validate_manifest(dict(self.value, agent="codex", config_transition=change))

    def test_channel_only_requires_same_binary_and_explicit_nondefault_channel_change(self):
        value = dict(self.value, expected="channel_only", target_version=self.value["old_version"],
            target_binary=self.value["old_binary"], config_transition={"kind": "channel", "before": "stable", "after": "latest"})
        runner.validate_manifest(value)
        for fields in ({"config_transition": None}, {"target_version": "2.1.278"},
                       {"target_binary": self.value["target_binary"]}, {"expected": "updated"}):
            with self.subTest(fields=fields), self.assertRaises(ValueError):
                runner.validate_manifest(dict(value, **fields))

    def test_codex_marker_contract_has_hashes_only_and_cannot_mutate_other_agents(self):
        change = {"kind": "codex_marker", "before_sha256": "a" * 64, "after_sha256": None}
        value = dict(self.value, agent="codex", channel="alpha", target_version="0.150.0-alpha.4", config_transition=change)
        runner.validate_manifest(value)
        for bad in (dict(change, before_sha256="old user text"), dict(change, after_sha256=True),
                    dict(change, after="arbitrary content")):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                runner.validate_manifest(dict(value, config_transition=bad))
        with self.assertRaises(ValueError):
            runner.validate_manifest(dict(self.value, config_transition=change))

    def test_environment_never_inherits_auth_proxy_or_process_injection(self):
        with patch.dict(os.environ, {"ANTHROPIC_API_KEY": "not-a-real-key", "HTTPS_PROXY": "unexpected-proxy",
                                     "DYLD_INSERT_LIBRARIES": "unexpected-library", "CODEX_MANAGED_BY_NPM": "1"}):
            environment = runner.isolated_environment(self.root, self.value)
        self.assertNotIn("ANTHROPIC_API_KEY", environment)
        self.assertNotIn("HTTPS_PROXY", environment)
        self.assertNotIn("DYLD_INSERT_LIBRARIES", environment)
        self.assertNotIn("CODEX_MANAGED_BY_NPM", environment)
        self.assertEqual(environment["INFINISHELL_CLI_SUPERVISOR_EXECUTABLE"], self.value["supervisor"]["path"])
        self.assertEqual(environment["HOME"], str(self.root / "home"))
        self.assertEqual(environment["INFINISHELL_CLI_AUTOUPDATE_CASE"], "claude-latest-one")

    def test_receipt_requires_exact_test_count_source_binding_and_all_safety_flags(self):
        event = self.event()
        text = "test result: ok. 1 passed; 0 failed; 0 ignored; 100 filtered out;"
        self.assertTrue(runner.acceptance(0, text, event, self.value, "a" * 64))
        for key, bad in (("worker_sha256", "b" * 64), ("source_manifest_sha256", "b" * 64),
                         ("product_execute_calls", 2), ("product_execute_calls", True), ("model_inputs_sent", 1),
                         ("config_permissions_unchanged", False), ("journal_absent", False),
                         ("failure_intent_persisted", False), ("failure_code", "execute_failed"),
                         ("case_id", "other-case")):
            with self.subTest(key=key):
                self.assertFalse(runner.acceptance(0, text, dict(event, **{key: bad}), self.value, "a" * 64))
        self.assertFalse(runner.acceptance(0, "test result: ok. 0 passed;", event, self.value, "a" * 64))
        self.assertFalse(runner.acceptance(1, text, event, self.value, "a" * 64))

    def test_declared_channel_change_preserves_literal_change_flags_and_requires_exact_proof(self):
        change = {"kind": "channel", "before": "stable", "after": "latest"}
        value = dict(self.value, config_transition=change)
        event = dict(self.event(), config_transition=change, config_bytes_unchanged=False,
            config_semantics_unchanged=False)
        text = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        self.assertTrue(runner.acceptance(0, text, event, value, "a" * 64))
        for key in ("config_transition_verified", "unrelated_config_bytes_unchanged", "config_permissions_preserved"):
            with self.subTest(key=key):
                self.assertFalse(runner.acceptance(0, text, dict(event, **{key: False}), value, "a" * 64))
        self.assertFalse(runner.acceptance(0, text, event, self.value, "a" * 64))
        self.assertFalse(runner.acceptance(0, text, dict(event, config_transition=None), value, "a" * 64))

    def test_channel_only_needs_observed_plan_and_unchanged_entry_and_generation_set(self):
        self.value.update(expected="channel_only", target_version=self.value["old_version"],
            target_binary=self.value["old_binary"], config_transition={"kind": "channel", "before": "stable", "after": "latest"})
        event = dict(self.event(), config_bytes_unchanged=False, config_semantics_unchanged=False)
        text = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        self.assertTrue(runner.acceptance(0, text, event, self.value, "a" * 64))
        for key, bad in (("plan_requires_native_update", True), ("plan_requires_native_update", None),
                         ("plan_requires_native_update", 0), ("entry_unchanged", False),
                         ("supervisor_generations_unchanged", False), ("product_execute_calls", 0)):
            with self.subTest(key=key, bad=bad):
                self.assertFalse(runner.acceptance(0, text, dict(event, **{key: bad}), self.value, "a" * 64))

    def test_safe_event_rejects_unknown_config_content_and_unobserved_plan_cannot_pass(self):
        event = self.event()
        self.assertFalse(runner.valid_event(dict(event, config_transition={"kind": "channel", "before": "private text", "after": "latest"})))
        self.assertFalse(runner.valid_event(dict(event, config_transition={"kind": "codex_marker", "before_sha256": "private text", "after_sha256": None})))
        self.assertFalse(runner.acceptance(0, "test result: ok. 1 passed; 0 failed; 0 ignored;",
            dict(event, plan_requires_native_update=None), self.value, "a" * 64))

    def test_unknown_event_text_cannot_be_archived_as_safe_evidence(self):
        event = self.event()
        self.assertFalse(runner.valid_event(dict(event, stderr="not-for-public-output")))
        self.assertFalse(runner.valid_event(dict(event, error="arbitrary-native-output")))
        self.assertFalse(runner.valid_event(dict(event, failure_code="arbitrary-native-output")))
        for key in ("agent", "channel", "expected", "stage", "error", "failure_code"):
            for bad in ([], {}, 7):
                with self.subTest(key=key, bad=bad):
                    self.assertFalse(runner.valid_event(dict(event, **{key: bad})))

    def test_source_changed_case_requires_true_rejection_and_original_version_contract(self):
        value = dict(self.value, expected="source_changed_rejected")
        event = dict(self.event(), expected="source_changed_rejected", error="SourceChanged")
        self.assertTrue(runner.acceptance(0, "test result: ok. 1 passed; 0 failed; 0 ignored;", event, value, "a" * 64))
        self.assertFalse(runner.acceptance(0, "test result: ok. 1 passed; 0 failed; 0 ignored;",
                                           dict(event, error=None), value, "a" * 64))

    def test_failure_and_interruption_require_old_entry_real_generation_and_exact_error(self):
        text = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        for expected, error in (("command_failed_rolled_back", "CommandFailed"),
                                ("interrupted_recovered", None)):
            with self.subTest(expected=expected):
                value = dict(self.value, expected=expected)
                runner.validate_manifest(value)
                event = dict(self.event(), expected=expected, error=error,
                    entry_unchanged=True, supervisor_generations_unchanged=False)
                self.assertTrue(runner.acceptance(0, text, event, value, "a" * 64))
                self.assertFalse(runner.acceptance(0, text,
                    dict(event, entry_unchanged=False), value, "a" * 64))
                self.assertFalse(runner.acceptance(0, text,
                    dict(event, supervisor_generations_unchanged=True), value, "a" * 64))
                self.assertFalse(runner.acceptance(0, text,
                    dict(event, error=None if error else "CommandFailed"), value, "a" * 64))
                with self.assertRaises(ValueError):
                    runner.validate_manifest(dict(value, config_transition={
                        "kind": "channel", "before": "stable", "after": "latest"}))

    def test_reservation_is_exclusive_and_cannot_silently_retry(self):
        path = self.root / "invocation.safe.json"
        runner.exclusive_bytes(path, b"original")
        with self.assertRaises(FileExistsError):
            runner.exclusive_bytes(path, b"replacement")
        self.assertEqual(path.read_bytes(), b"original")

    def test_supervisor_and_test_worker_must_bind_to_the_same_successful_source(self):
        runner.verify_build_binding(self.value)
        bundle = json.loads(Path(self.value["bundle_report"]["path"]).read_bytes())
        for changed in (dict(bundle, source_manifest_sha256="c" * 64),
                        dict(bundle, worker=self.value["worker"])):
            with self.subTest(changed=changed):
                self.write_binding("bundle_report", changed)
                with self.assertRaises(ValueError):
                    runner.verify_build_binding(self.value)
        self.write_binding("bundle_report", bundle)
        self.write_binding("source_manifest", {"files": []})
        with self.assertRaises(ValueError):
            runner.verify_build_binding(self.value)

    def test_source_manifest_and_signature_are_verified_instead_of_trusting_reports(self):
        source = json.loads(Path(self.value["source_manifest"]["path"]).read_bytes())
        altered = json.loads(json.dumps(source))
        altered["files"][0]["sha256"] = "c" * 64
        self.write_binding("source_manifest", altered)
        gates = json.loads(Path(self.value["gates_report"]["path"]).read_bytes())
        gates["source_manifest_sha256"] = self.value["source_manifest"]["sha256"]
        self.write_binding("gates_report", gates)
        bundle = json.loads(Path(self.value["bundle_report"]["path"]).read_bytes())
        bundle["source_manifest_sha256"] = self.value["source_manifest"]["sha256"]
        self.write_binding("bundle_report", bundle)
        with self.assertRaisesRegex(ValueError, "source_file_manifest_mismatch"):
            runner.verify_build_binding(self.value)

        self.write_binding("source_manifest", source)
        gates["source_manifest_sha256"] = self.value["source_manifest"]["sha256"]
        self.write_binding("gates_report", gates)
        bundle["source_manifest_sha256"] = self.value["source_manifest"]["sha256"]
        self.write_binding("bundle_report", bundle)
        with patch.object(runner, "supervisor_source_binding_verified", return_value=False), \
             self.assertRaisesRegex(ValueError, "source_build_mismatch"):
            runner.verify_build_binding(self.value)
        with patch.object(runner, "strict_signature_verified", return_value=False), \
             self.assertRaisesRegex(ValueError, "supervisor_signature_not_verified"):
            runner.verify_build_binding(self.value)

    def test_macos_signature_check_executes_codesign_instead_of_reading_report_status(self):
        succeeded = type("Result", (), {"returncode": 0})()
        failed = type("Result", (), {"returncode": 1})()
        with patch.object(runner.sys, "platform", "darwin"), \
             patch.object(runner.subprocess, "run", side_effect=[succeeded, failed]) as called:
            self.assertTrue(self.strict_signature_verified(Path(self.value["supervisor"]["path"])))
            self.assertFalse(self.strict_signature_verified(Path(self.value["supervisor"]["path"])))
        self.assertEqual(called.call_args_list[0].args[0][:3],
            ["/usr/bin/codesign", "--verify", "--strict"])

    def test_binary_source_binding_matches_across_read_boundaries(self):
        path = self.root / "source-bound-supervisor"
        path.write_bytes(b"x" * (1024 * 1024 - 2) + b"source-one-middle-source-two")
        self.assertTrue(runner.binary_contains_all(path, [b"source-one", b"source-two"]))
        self.assertFalse(runner.binary_contains_all(path, [b"source-one", b"missing"]))

    def test_source_binding_covers_adapter_versions_and_plugin_compatibility(self):
        required = {
            "app/src/ai/cli_agent_runtime/codex.rs",
            "app/src/ai/cli_agent_runtime/claude.rs",
            "app/src/ai/cli_agent_runtime/grok.rs",
            "app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs",
            "app/src/terminal/cli_agent_sessions/plugin_manager/codex.rs",
            "app/src/terminal/cli_agent_sessions/plugin_manager/claude.rs",
            "app/src/terminal/cli_agent_sessions/plugin_manager/grok.rs",
            "app/src/terminal/cli_agent_sessions/plugin_manager/codex_source.rs",
            "app/src/terminal/cli_agent_sessions/plugin_manager/codex_hook_trust.rs",
            "app/src/terminal/cli_agent_sessions/plugin_manager/notification_patch.rs",
        }
        self.assertTrue(required.issubset(runner.REQUIRED_SOURCE_FILES))
        self.assertTrue(required.issubset(runner.SUPERVISOR_SOURCE_FILES))

        source = (Path(__file__).resolve().parents[2]
                  / "app/src/terminal/cli_agent_updates/sources.rs").read_text(encoding="utf-8")
        for path in required:
            relative = path.removeprefix("app/src/terminal/")
            if relative == path:
                relative = "../../" + path.removeprefix("app/src/")
            else:
                relative = "../" + relative
            self.assertIn(f'include_bytes!("{relative}")', source)

    def test_missing_or_modified_gate_artifact_cannot_reuse_success_metadata(self):
        path = Path(self.value["gates_report"]["path"])
        body = path.read_bytes()
        path.write_bytes(body + b" ")
        with self.assertRaises(ValueError):
            runner.verify_build_binding(self.value)
        path.unlink()
        with self.assertRaises(OSError):
            runner.verify_build_binding(self.value)

    @unittest.skipUnless(os.name == "posix", "真实原生夹具的链接和 0600 权限限定 POSIX")
    def test_fixture_rejects_tampering_symlink_escape_and_private_permission_changes(self):
        entry = Path(self.value["entry"])
        entry.parent.mkdir(parents=True)
        entry.symlink_to(self.root / "old_binary")
        runner.exclusive_bytes(self.root / ".infinishell-cli-autoupdate", runner.MARKER)
        self.assertEqual(runner.verify_fixture(self.value), self.root)
        (self.root / "target_binary").write_bytes(b"modified")
        with self.assertRaises(ValueError):
            runner.verify_fixture(self.value)
        (self.root / "target_binary").write_bytes(b"synthetic-target_binary")
        (self.root / ".infinishell-cli-autoupdate").chmod(0o644)
        with self.assertRaises(ValueError):
            runner.verify_fixture(self.value)
        (self.root / ".infinishell-cli-autoupdate").chmod(0o600)
        moved = self.root / "moved-bin"
        entry.parent.rename(moved)
        entry.parent.symlink_to(moved, target_is_directory=True)
        with self.assertRaises(ValueError):
            runner.verify_fixture(self.value)

    def mocked_run(self, *, timeout=False, event_override=None):
        manifest = self.root / "manifest.private.json"
        runner.exclusive_bytes(manifest, runner.encoded(self.value))
        args = type("Args", (), {"manifest": manifest, "case_id": self.value["case_id"], "allow_native_update": True})()
        event = self.event()
        event["manifest_sha256"] = runner.digest(manifest)
        if event_override:
            event.update(event_override)
        captured = {}

        class Process:
            pid = 12345
            returncode = None if timeout else 0

            def wait(self, timeout=None):
                if self.returncode is None:
                    raise subprocess.TimeoutExpired("synthetic-python-only", timeout)
                return self.returncode

        def spawn(argv, **kwargs):
            captured.update(argv=argv, kwargs=kwargs)
            kwargs["stdout"].write(b"test result: ok. 1 passed; 0 failed; 0 ignored;\n")
            kwargs["stdout"].flush()
            runner.exclusive_bytes(self.root / "events.safe.json", runner.encoded(event))
            return Process()

        with patch.object(runner, "native_platform_supported", return_value=True), patch.object(runner, "private_bytes", side_effect=lambda path: path.read_bytes()), \
             patch.object(runner, "verify_fixture", return_value=self.root), patch.object(runner, "isolated_environment", return_value={}), \
             patch.object(runner.subprocess, "Popen", side_effect=spawn), patch.object(runner, "group_exists", return_value=timeout), \
             patch.object(runner, "stop_group", return_value=True) as cleanup:
            result = runner.run(args)
            cleanup.assert_called_once()
        return result, captured

    def test_run_uses_only_exact_ignored_test_and_records_real_success_contract(self):
        result, captured = self.mocked_run()
        self.assertTrue(result["passed"])
        self.assertEqual(captured["argv"][1:], [runner.TEST_NAME, "--exact", "--ignored", "--nocapture", "--test-threads=1"])
        self.assertTrue(captured["kwargs"]["start_new_session"])
        self.assertNotIn("detached_descendants_verified", result)

    def test_windows_reparse_attribute_is_never_plain(self):
        self.assertTrue(runner.windows_file_attributes_are_plain(0))
        self.assertTrue(runner.windows_file_attributes_are_plain(0x20))
        self.assertFalse(runner.windows_file_attributes_are_plain(0x400))
        self.assertFalse(runner.windows_file_attributes_are_plain(0x420))

    def test_channel_only_run_reserves_single_execute_without_claiming_native_update(self):
        self.value.update(expected="channel_only", target_version=self.value["old_version"],
            target_binary=self.value["old_binary"], config_transition={"kind": "channel", "before": "stable", "after": "latest"})
        result, _ = self.mocked_run(event_override={"config_bytes_unchanged": False, "config_semantics_unchanged": False})
        self.assertTrue(result["passed"])
        self.assertFalse(result["native_update_expected"])
        self.assertEqual(result["max_product_execute_calls"], 1)
        self.assertNotIn("native_update_exit_code", result)
        self.assertFalse(result["event"]["plan_requires_native_update"])

    def test_prepare_formal_cli_accepts_explicit_transition_and_channel_only(self):
        change = {"kind": "channel", "before": "stable", "after": "latest"}
        captured = {}
        def prepared(args):
            captured.update(vars(args))
            return {"scope": runner.SCOPE, "case_id": "same-version"}
        argv = ["prepare", "--root", str(self.root), "--entry", self.value["entry"],
            "--case-id", "same-version", "--agent", "claude", "--channel", "latest",
            "--old-version", "2.1.278", "--target-version", "2.1.278", "--expected", "channel_only",
            "--config-transition", json.dumps(change)]
        for name in runner.BINDINGS:
            argv.extend(["--" + name.replace("_", "-"), self.value[name]["path"],
                         "--" + name.replace("_", "-") + "-sha256", self.value[name]["sha256"]])
        with patch.object(runner, "prepare", side_effect=prepared), patch("builtins.print"):
            self.assertEqual(runner.main(argv), 0)
        self.assertEqual(json.loads(captured["config_transition"]), change)
        self.assertEqual(captured["expected"], "channel_only")

    def test_timeout_cleans_group_but_never_becomes_success(self):
        result, _ = self.mocked_run(timeout=True)
        self.assertTrue(result["worker_group_stopped"])
        self.assertTrue(result["timed_out"])
        self.assertFalse(result["passed"])
        self.assertIsNone(result["test_exit_code"])

    def test_native_error_body_is_not_copied_to_public_receipt(self):
        result, _ = self.mocked_run(event_override={"error": "unsafe-native-text"})
        self.assertFalse(result["passed"])
        self.assertNotIn("event", result)
        self.assertNotIn("unsafe-native-text", (self.root / "receipt.safe.json").read_text(encoding="utf-8"))

    @unittest.skipUnless(os.name == "posix", "真实进程组退出由 POSIX setsid/killpg 提供")
    def test_cleanup_stops_an_actual_python_process_group_without_native_cli(self):
        process = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(30)"], start_new_session=True,
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            self.assertTrue(runner.stop_group(process))
            self.assertIsNotNone(process.poll())
            self.assertFalse(runner.group_exists(process.pid))
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=3)

    def test_main_propagates_failure_to_real_python_process_exit_status(self):
        code = ("import sys;sys.path.insert(0,sys.argv[1]);import verify_cli_autoupdate as r;"
                "r.run=lambda args:{'scope':r.SCOPE,'case_id':'mock','passed':False};"
                "raise SystemExit(r.main(['run','--manifest','unused','--case-id','mock','--allow-native-update']))")
        result = subprocess.run([sys.executable, "-c", code, str(Path(runner.__file__).parent)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)


if __name__ == "__main__":
    unittest.main()
