#!/usr/bin/env python3
"""真实验收运行器必须同时取得测试匹配、退出成功及对应协议终态。"""

import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from run_codex_adapter_live import (
    VERSION_PROBE_TIMEOUT_SECONDS,
    audit_and_cleanup_fixture,
    idle_crash_environment,
    missing_session_environment,
    owned_fixture_pids,
    owned_native_root_pids,
    owned_supervisor_pids,
    probe_version,
    verified_acceptance,
)


class AcceptanceEvidenceTests(unittest.TestCase):
    def test_fixed_tool_cleanup_targets_only_private_fixture_and_is_not_product_proof(self):
        root = Path("/private/tmp/codex-fixture-unique")
        scripts = {41: f"python {root / 'project/cancel-parent.py'}",
                   42: f"python {root / 'project/cancel-leaf.py'}",
                   43: "python /another/project/cancel-parent.py"}
        self.assertEqual(owned_fixture_pids(root, scripts), {41, 42})
        with mock.patch("run_codex_adapter_live.macos_native_root_pids", return_value=set()), \
                mock.patch("run_codex_adapter_live.process_commands", side_effect=[scripts, scripts, scripts, {}, {}]), \
                mock.patch("run_codex_adapter_live.time.monotonic", side_effect=[0, 21, 22]), \
                mock.patch("run_codex_adapter_live.os.kill") as kill, \
                mock.patch("run_codex_adapter_live.subprocess.run") as run, \
                mock.patch("run_codex_adapter_live.time.sleep"):
            result = audit_and_cleanup_fixture(root, Path("/usr/bin/codex"), Path("/opt/infinishell"))
        self.assertTrue(result["fallback_attempted"])
        self.assertTrue(result["zero_residual"])
        self.assertFalse(result["audit_error"])
        if os.name == "nt":
            self.assertEqual({int(call.args[0][2]) for call in run.call_args_list}, {41, 42})
            kill.assert_not_called()
        else:
            self.assertEqual({call.args[0] for call in kill.call_args_list}, {41, 42})
            run.assert_not_called()

    def test_native_root_fallback_requires_private_cwd_and_exact_cli_path(self):
        root = Path("/private/tmp/codex-fixture-unique")
        codex = Path("/opt/codex/codex")
        commands = {41: f"{codex} app-server", 42: "/another/codex app-server"}
        inspected = mock.Mock(stdout=f"p41\nfcwd\nn{root / 'project'}\n")
        with mock.patch("run_codex_adapter_live.process_commands", return_value=commands), \
                mock.patch("run_codex_adapter_live.subprocess.run", return_value=inspected):
            self.assertEqual(owned_native_root_pids(root, codex, {41, 42}), {41})
        inspected.stdout = "p41\nfcwd\nn/another/project\n"
        with mock.patch("run_codex_adapter_live.process_commands", return_value=commands), \
                mock.patch("run_codex_adapter_live.subprocess.run", return_value=inspected):
            self.assertEqual(owned_native_root_pids(root, codex, {41}), set())

    def test_supervisor_cleanup_requires_private_manifest_and_exact_executable(self):
        root = Path("/private/tmp/codex-fixture-unique")
        supervisor = Path("/opt/infinishell")
        commands = {41: f"{supervisor} cli-agent-supervisor {root / 'cli-agent-processes/one/manifest.json'}",
                    42: f"{supervisor} cli-agent-supervisor /another/root/manifest.json",
                    43: f"/another/infinishell cli-agent-supervisor {root / 'cli-agent-processes/one/manifest.json'}"}
        self.assertEqual(owned_supervisor_pids(root, supervisor, commands), {41})

    def test_version_probe_preserves_special_path_and_allows_cold_scan_budget(self):
        executable = Path("C:/runner temp/codex shim 中文 & path/codex.cmd")
        environment = {"PATH": "isolated"}
        completed = mock.Mock(stdout="codex-cli 0.147.0\n")
        with mock.patch("run_codex_adapter_live.subprocess.run", return_value=completed) as run:
            self.assertIs(probe_version(executable, environment), completed)
        self.assertEqual(VERSION_PROBE_TIMEOUT_SECONDS, 30)
        run.assert_called_once_with(
            [str(executable), "--version"],
            env=environment,
            text=True,
            capture_output=True,
            timeout=30,
            check=True,
        )

    def test_each_case_requires_current_success_and_matching_test(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        zero = "test result: ok. 0 passed; 0 failed; 0 ignored;"
        cases = {
            "lifecycle": {"event": "acceptance_passed"},
            "running-tool-cancel": {
                "event": "running_tool_cancel_finished", "passed": True,
                "native_item_started_before_interrupt": True, "interrupt_native_ack": True,
                "same_generation_receipt": True, "cleanup_confirmed": True,
                "tool_tree_zero_residual": True, "terminal_before_disconnected": True,
                "event_order": ["native_item_started", "interrupt_sent", "interrupt_accepted",
                                "turn_finished", "disconnected"],
                "containment": "macos_resource_coalition",
            },
            "local-tools-restore": {"event": "tool_restore_probe_finished", "passed": True},
            "image-input": {"event": "image_probe_finished", "passed": True},
            "missing-session": {"event": "missing_session_probe_finished", "passed": True},
            "idle-crash": {"event": "idle_crash_probe_finished", "passed": True,
                           "phase": "after_session_ready", "native_root_exit_observed": True,
                           "credentials_provided": False, "model_commands_sent": 0},
        }
        for case, event in cases.items():
            with self.subTest(case=case):
                self.assertTrue(verified_acceptance(case, 0, summary, [event]))
                self.assertFalse(verified_acceptance(case, 1, summary, [event]))
                self.assertFalse(verified_acceptance(case, 0, zero, [event]))
                self.assertFalse(verified_acceptance(case, 0, summary, []))

    def test_running_tool_cancel_rejects_missing_cleanup_or_wrong_order(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        valid = {
            "event": "running_tool_cancel_finished", "passed": True,
            "native_item_started_before_interrupt": True, "interrupt_native_ack": True,
            "same_generation_receipt": True, "cleanup_confirmed": True,
            "tool_tree_zero_residual": True, "terminal_before_disconnected": True,
            "event_order": ["native_item_started", "interrupt_sent", "interrupt_accepted",
                            "turn_finished", "disconnected"],
            "containment": "macos_resource_coalition",
        }
        self.assertTrue(verified_acceptance("running-tool-cancel", 0, summary, [valid]))
        for key in ("native_item_started_before_interrupt", "interrupt_native_ack",
                    "same_generation_receipt", "cleanup_confirmed", "tool_tree_zero_residual",
                    "terminal_before_disconnected"):
            with self.subTest(key=key):
                self.assertFalse(verified_acceptance("running-tool-cancel", 0, summary,
                                                     [valid | {key: False}]))
        self.assertFalse(verified_acceptance("running-tool-cancel", 0, summary,
                                             [valid | {"event_order": list(reversed(valid["event_order"]))}]))
        self.assertFalse(verified_acceptance("running-tool-cancel", 0, summary,
                                             [valid | {"containment": "unix_process_group"}]))

    def test_image_check_cannot_reuse_another_probe_or_failed_result(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        for event in [
            {"event": "tool_restore_probe_finished", "passed": True},
            {"event": "image_probe_finished", "passed": False},
        ]:
            with self.subTest(event=event):
                self.assertFalse(verified_acceptance("image-input", 0, summary, [event]))

    def test_missing_session_cannot_reuse_another_probe_or_failed_result(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        for event in [
            {"event": "acceptance_passed"},
            {"event": "tool_restore_probe_finished", "passed": True},
            {"event": "missing_session_probe_finished", "passed": False},
        ]:
            with self.subTest(event=event):
                self.assertFalse(verified_acceptance("missing-session", 0, summary, [event]))

    def test_missing_session_environment_has_no_user_credentials_or_configuration(self):
        inherited = {"PATH": "fixture-bin", "SystemRoot": "fixture-system", "OPENAI_API_KEY": "test-only",
                     "HOME": "user-home", "CODEX_HOME": "user-codex", "CODEX_CONFIG": "user-config",
                     "HTTPS_PROXY": "user-proxy", "DYLD_INSERT_LIBRARIES": "user-library"}
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ, inherited, clear=True):
            root = Path(temporary).resolve()
            environment = missing_session_environment(root)
            self.assertEqual(environment["PATH"], "fixture-bin")
            # Windows 会正规化环境变量键；核对系统目录值，不依赖键名大小写。
            self.assertEqual({key.upper(): value for key, value in environment.items() if key.upper() == "SYSTEMROOT"},
                             {"SYSTEMROOT": "fixture-system"})
            for key in ("OPENAI_API_KEY", "CODEX_CONFIG", "HTTPS_PROXY", "DYLD_INSERT_LIBRARIES"):
                self.assertNotIn(key, environment)
            for key in ("HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "CODEX_HOME", "TMP", "TEMP", "TMPDIR"):
                self.assertTrue(Path(environment[key]).is_relative_to(root))
                self.assertTrue(Path(environment[key]).is_dir())
            self.assertEqual((root / "codex/config.toml").read_bytes(), b'cli_auth_credentials_store = "file"\n')
            self.assertEqual((root / ".infinishell-missing-session-probe").read_bytes(),
                             b"isolated unauthenticated missing-session verification\n")
            self.assertFalse((root / "codex/auth.json").exists())
            self.assertEqual(environment["INFINISHELL_CODEX_MISSING_ROOT"], str(root))

    def test_idle_crash_requires_real_ready_root_exit_and_no_model(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        valid = {"event": "idle_crash_probe_finished", "passed": True,
                 "phase": "after_session_ready", "native_root_exit_observed": True,
                 "credentials_provided": False, "model_commands_sent": 0}
        for key, value in {"event": "missing_session_probe_finished", "passed": False,
                           "phase": "before_session_ready", "native_root_exit_observed": False,
                           "credentials_provided": True, "model_commands_sent": 1}.items():
            with self.subTest(key=key):
                self.assertFalse(verified_acceptance("idle-crash", 0, summary, [valid | {key: value}]))

    def test_idle_crash_environment_reuses_credential_free_boundary_with_own_marker(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(os.environ,
                {"OPENAI_API_KEY": "test-only", "CODEX_HOME": "user-home"}, clear=True):
            root = Path(temporary).resolve()
            environment = idle_crash_environment(root)
            self.assertNotIn("OPENAI_API_KEY", environment)
            self.assertNotIn("INFINISHELL_CODEX_MISSING_ROOT", environment)
            self.assertFalse((root / ".infinishell-missing-session-probe").exists())
            self.assertEqual(environment["INFINISHELL_CODEX_IDLE_CRASH_ROOT"], str(root))
            self.assertEqual((root / ".infinishell-idle-crash-probe").read_bytes(),
                             b"isolated unauthenticated idle-crash verification\n")
            self.assertEqual((root / "codex/config.toml").read_bytes(), b'cli_auth_credentials_store = "file"\n')
            self.assertFalse((root / "codex/auth.json").exists())

    def test_new_macos_idle_cleanup_requires_coalition_binding_and_completed_proof(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        valid = {"event": "idle_crash_probe_finished", "passed": True,
                 "phase": "after_session_ready", "native_root_exit_observed": True,
                 "credentials_provided": False, "model_commands_sent": 0,
                 "exit_receipt": {"containment": "macos_resource_coalition", "cleanup_confirmed": True},
                 "idle_process_cleanup_confirmed": True, "unsafe_recovery_prevented": False,
                 "macos_coalition_ownership_verified": True, "macos_cleanup_proof_verified": True,
                 "running_tool_tree_cleanup_verified": False}
        self.assertTrue(verified_acceptance("idle-crash", 0, summary, [valid]))
        for key in ("macos_coalition_ownership_verified", "macos_cleanup_proof_verified",
                    "idle_process_cleanup_confirmed", "running_tool_tree_cleanup_verified"):
            with self.subTest(key=key):
                missing = dict(valid)
                missing.pop(key)
                self.assertFalse(verified_acceptance("idle-crash", 0, summary, [missing]))
                self.assertFalse(verified_acceptance("idle-crash", 0, summary,
                                                     [valid | {key: not valid[key]}]))
        self.assertFalse(verified_acceptance("idle-crash", 0, summary, [valid | {
            "exit_receipt": {"containment": "macos_resource_coalition", "cleanup_confirmed": False}}]))

    def test_legacy_macos_process_group_crash_remains_a_negative_cleanup_result(self):
        summary = "test result: ok. 1 passed; 0 failed; 0 ignored;"
        legacy = {"event": "idle_crash_probe_finished", "passed": True,
                  "phase": "after_session_ready", "native_root_exit_observed": True,
                  "credentials_provided": False, "model_commands_sent": 0,
                  "exit_receipt": {"containment": "unix_process_group", "cleanup_confirmed": False},
                  "idle_process_cleanup_confirmed": False, "unsafe_recovery_prevented": True,
                  "running_tool_tree_cleanup_verified": False}
        self.assertTrue(verified_acceptance("idle-crash", 0, summary, [legacy]))
        for patch in ({"exit_receipt": {"containment": "unix_process_group", "cleanup_confirmed": True}},
                      {"idle_process_cleanup_confirmed": True}, {"unsafe_recovery_prevented": False},
                      {"running_tool_tree_cleanup_verified": True}):
            with self.subTest(patch=patch):
                self.assertFalse(verified_acceptance("idle-crash", 0, summary, [legacy | patch]))


if __name__ == "__main__":
    unittest.main()
